// SPDX-License-Identifier: Apache-2.0

use clap::{Arg, Command as ClapCommand};
use crossterm::{
    event::{self, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph, Tabs},
    Terminal,
};
use regex::Regex;
use std::sync::mpsc::channel;
use std::{env, fs, io, process::Command, time::Duration};

struct App {
    code: String,
    unopt_ir: String,
    opt_ir: String,
    delay_info: String,
    error_message: Option<String>,
    selected_tab: usize, // 0: unopt IR, 1: opt IR, 2: delay info
    dslx_stdlib_path: Option<String>,
    tests_passed: Option<bool>,
    test_output: Option<String>,
    entry_points: Vec<String>,
    selected_entry: usize,
    file_path: Option<String>,
    last_update: Option<String>,
}

impl App {
    fn new() -> Self {
        Self {
            code: String::new(),
            unopt_ir: String::new(),
            opt_ir: String::new(),
            delay_info: String::new(),
            error_message: None,
            selected_tab: 0,
            dslx_stdlib_path: None,
            tests_passed: None,
            test_output: None,
            entry_points: Vec::new(),
            selected_entry: 0,
            file_path: None,
            last_update: None,
        }
    }

    fn update_entry_points(&mut self) {
        // Use regex to extract function names from unopt_ir
        let re = Regex::new(r"(?m)^fn (\w+)").unwrap();
        let mut matches = Vec::new();
        for cap in re.captures_iter(&self.unopt_ir) {
            matches.push(cap[1].to_string());
        }
        if matches.is_empty() {
            matches.push("main".into());
        }
        self.entry_points = matches;
        if self.selected_entry >= self.entry_points.len() {
            self.selected_entry = 0;
        }
    }

    fn run_conversion(&mut self) {
        self.tests_passed = Some(false);
        let file_path = self.file_path.clone().expect("file_path not set");

        let tools = env::var("XLSYNTH_TOOLS").expect("XLSYNTH_TOOLS not set");
        let ir_converter_path = format!("{}/ir_converter_main", tools);
        let mut ir_conv_cmd = Command::new(&ir_converter_path);
        ir_conv_cmd.arg(file_path.clone());
        if let Some(ref stdlib) = self.dslx_stdlib_path {
            ir_conv_cmd.arg("--dslx_stdlib_path").arg(stdlib);
        }
        let ir_conv_output = ir_conv_cmd
            .output()
            .expect("Failed to run ir_converter_main");
        if !ir_conv_output.status.success() {
            self.error_message = Some(format!(
                "ir_converter_main: {}",
                String::from_utf8_lossy(&ir_conv_output.stderr)
            ));
            self.tests_passed = Some(false);
            return;
        }
        self.error_message = None;
        let unopt_ir = String::from_utf8_lossy(&ir_conv_output.stdout).to_string();
        self.unopt_ir = unopt_ir.clone();
        self.update_entry_points();

        let opt_file = format!("{}.unopt.ir", file_path.clone());
        fs::write(&opt_file, &unopt_ir).expect("Failed to write unoptimized IR file");
        let opt_main_path = format!("{}/opt_main", tools);
        let entry_name = &self.entry_points[self.selected_entry];
        let top_arg = entry_name.to_string();
        let opt_output = Command::new(&opt_main_path)
            .arg(&opt_file)
            .arg("--top")
            .arg(top_arg)
            .output()
            .expect("Failed to run opt_main");
        if !opt_output.status.success() {
            self.error_message = Some(format!(
                "opt_main: {}",
                String::from_utf8_lossy(&opt_output.stderr)
            ));
            self.tests_passed = Some(false);
            return;
        }
        self.error_message = None;
        let opt_ir = String::from_utf8_lossy(&opt_output.stdout).to_string();
        self.opt_ir = opt_ir.clone();

        let opt_file = format!("{}.opt.ir", file_path.clone());
        fs::write(&opt_file, &opt_ir).expect("Failed to write optimized IR file");

        let delay_main_path = format!("{}/delay_info_main", tools);
        let delay_output = Command::new(&delay_main_path)
            .arg(&opt_file)
            .arg("--delay_model")
            .arg("asap7")
            .output()
            .expect("Failed to run delay_info_main");
        if !delay_output.status.success() {
            self.error_message = Some(format!(
                "delay_info_main: {}",
                String::from_utf8_lossy(&delay_output.stderr)
            ));
            self.tests_passed = Some(false);
            return;
        }
        self.error_message = None;
        let delay_info = String::from_utf8_lossy(&delay_output.stdout).to_string();
        self.delay_info = delay_info;

        let interpreter_path = format!("{}/dslx_interpreter_main", tools);
        if std::path::Path::new(&interpreter_path).exists() {
            let mut interpreter_cmd = Command::new(&interpreter_path);
            interpreter_cmd.arg(file_path.clone());
            if let Some(ref stdlib) = self.dslx_stdlib_path {
                interpreter_cmd.arg("--dslx_stdlib_path").arg(stdlib);
            }
            interpreter_cmd.arg("--compare=jit");
            let interpreter_output = interpreter_cmd
                .output()
                .expect("Failed to run dslx_interpreter_main");
            if interpreter_output.status.success() {
                self.tests_passed = Some(true);
                let output = if interpreter_output.stdout.is_empty() {
                    interpreter_output.stderr
                } else {
                    interpreter_output.stdout
                };
                self.test_output = Some(String::from_utf8_lossy(&output).to_string());
            } else {
                self.error_message = Some(format!(
                    "dslx_interpreter_main: {}",
                    String::from_utf8_lossy(&interpreter_output.stderr)
                ));
                self.tests_passed = Some(false);
                return;
            }
        }
    }

    fn check_and_run_conversion(&mut self) {
        self.update_entry_points();
        self.run_conversion();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = ClapCommand::new("DSLX Playground")
        .version("1.0")
        .author("Author Name <email@example.com>")
        .about("Watches a DSLX file and renders its IR")
        .arg(
            Arg::new("file")
                .short('f')
                .long("file")
                .value_name("FILE")
                .help("Sets the input file to watch")
                .required(true),
        )
        .arg(
            Arg::new("dslx_stdlib_path")
                .long("dslx_stdlib_path")
                .value_name("PATH")
                .help("Optional path to the DSLX standard library")
                .required(false),
        )
        .get_matches();

    let file_path = matches.get_one::<String>("file").unwrap();
    let dslx_stdlib = matches.get_one::<String>("dslx_stdlib_path").cloned();

    let tools = env::var("XLSYNTH_TOOLS").expect("XLSYNTH_TOOLS environment variable not set");
    let required_binaries = ["ir_converter_main", "opt_main", "delay_info_main"];
    for binary in &required_binaries {
        let binary_path = format!("{}/{}", tools, binary);
        if !std::path::Path::new(&binary_path).exists() {
            panic!(
                "Required binary '{}' not found in XLSYNTH_TOOLS directory",
                binary
            );
        }
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Config::default())?;
    watcher.watch(std::path::Path::new(file_path), RecursiveMode::NonRecursive)?;

    let mut app = App::new();
    app.dslx_stdlib_path = dslx_stdlib;
    app.code = fs::read_to_string(file_path)?;
    app.file_path = Some(file_path.to_string());
    app.check_and_run_conversion();

    loop {
        terminal.draw(|f| {
            let size = f.size();
            let code_line_count = app.code.lines().count() as u16;
            // Compute top height: content lines + 6, but at least 10 and leaving at least 3 lines for error pane
            let top_height = std::cmp::min(
                std::cmp::max(code_line_count + 6, 10),
                size.height.saturating_sub(3),
            );
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(top_height), Constraint::Min(3)].as_ref())
                .split(size);

            let horizontal_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
                .split(chunks[0]);

            let left_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)].as_ref())
                .split(horizontal_chunks[0]);

            let code_with_line_numbers: String = app
                .code
                .lines()
                .enumerate()
                .map(|(i, line)| format!("{:>4} {}", i + 1, line))
                .collect::<Vec<_>>()
                .join("\n");
            let title = if let Some(time) = &app.last_update {
                format!("updated at {}", time)
            } else {
                String::from("File")
            };
            let code_widget = Paragraph::new(code_with_line_numbers)
                .block(Block::default().borders(Borders::ALL).title(title));
            f.render_widget(code_widget, left_chunks[0]);

            if let Some(tests_passed) = app.tests_passed {
                let test_status = if tests_passed {
                    Paragraph::new("Tests passed")
                        .style(Style::default().bg(Color::Green).fg(Color::Black))
                        .block(Block::default().borders(Borders::NONE))
                } else {
                    Paragraph::new("Artifact generation error")
                        .style(Style::default().bg(Color::Red).fg(Color::Black))
                        .block(Block::default().borders(Borders::NONE))
                };
                f.render_widget(test_status, left_chunks[1]);
            }

            let results_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Min(0),
                    ]
                    .as_ref(),
                )
                .split(horizontal_chunks[1]);

            let tabs_titles = vec![
                Spans::from(Span::styled("unopt IR", Style::default().fg(Color::Yellow))),
                Spans::from(Span::styled("opt IR", Style::default().fg(Color::Yellow))),
                Spans::from(Span::styled(
                    "delay info",
                    Style::default().fg(Color::Yellow),
                )),
            ];
            let entry_spans = Spans::from(
                app.entry_points
                    .iter()
                    .enumerate()
                    .map(|(i, ep)| {
                        if i == app.selected_entry {
                            Span::styled(
                                format!("[{}] ", ep),
                                Style::default().fg(Color::LightGreen),
                            )
                        } else {
                            Span::raw(format!("{} ", ep))
                        }
                    })
                    .collect::<Vec<Span>>(),
            );
            let entry_widget = Paragraph::new(entry_spans).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Entry (use ←/→ to change)"),
            );
            f.render_widget(entry_widget, results_chunks[0]);

            let tabs = Tabs::new(tabs_titles)
                .select(app.selected_tab)
                .block(Block::default().borders(Borders::ALL).title("Results"))
                .highlight_style(Style::default().fg(Color::LightGreen));
            f.render_widget(tabs, results_chunks[1]);

            let content = match app.selected_tab {
                0 => app.unopt_ir.as_str(),
                1 => app.opt_ir.as_str(),
                2 => app.delay_info.as_str(),
                _ => "",
            };
            let content_widget =
                Paragraph::new(content).block(Block::default().borders(Borders::ALL));
            f.render_widget(content_widget, results_chunks[2]);

            // Error pane always shown at the bottom
            let error_widget = if let Some(true) = app.tests_passed {
                Paragraph::new(
                    app.test_output
                        .clone()
                        .unwrap_or_else(|| String::from("[ no test output ]")),
                )
                .block(Block::default().borders(Borders::ALL).title("test output"))
            } else if let Some(error) = &app.error_message {
                Paragraph::new(error.clone()).block(Block::default().borders(Borders::ALL).title(
                    Spans::from(Span::styled("Error", Style::default().fg(Color::Red))),
                ))
            } else {
                Paragraph::new("[ none ]")
                    .style(Style::default().fg(Color::Gray))
                    .block(Block::default().borders(Borders::ALL).title("Error"))
            };
            f.render_widget(error_widget, chunks[1]);
        })?;

        // Handle file change events
        if let Ok(event_result) = rx.try_recv() {
            if let Ok(notify::Event {
                kind: EventKind::Modify(_),
                ..
            }) = event_result
            {
                // Reload the file and update the app state
                app.code = fs::read_to_string(file_path)?;
                app.last_update =
                    Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
                app.check_and_run_conversion();
            }
        }

        // Handle keyboard events for tab switching and exit
        if event::poll(Duration::from_millis(50))? {
            if let event::Event::Key(key_event) = event::read()? {
                match key_event.code {
                    KeyCode::Tab => {
                        app.selected_tab = (app.selected_tab + 1) % 3;
                    }
                    KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.selected_tab = 0;
                    }
                    KeyCode::Char('o') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.selected_tab = 1;
                    }
                    KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.selected_tab = 2;
                    }
                    KeyCode::Left => {
                        if app.selected_entry > 0 {
                            app.selected_entry -= 1;
                            app.check_and_run_conversion();
                        }
                    }
                    KeyCode::Right => {
                        if app.selected_entry < app.entry_points.len() - 1 {
                            app.selected_entry += 1;
                            app.check_and_run_conversion();
                        }
                    }
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => break,
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
