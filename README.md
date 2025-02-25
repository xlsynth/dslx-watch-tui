# DSLX Watch TUI

A TUI that:

* Watches for update events on a given file.
* Renders output artifacts when changes occur.
* Output artifacts are unopt IR, opt IR, and delay info.
* Multiple entry points can be selected for ease of back-and-forth comparison.
* Any test failures in the file are displayed in the error pane.
* Any failures in rendering output artifacts are displayed in the error pane.

## Hotkeys

* **Tab:** switches between output artifacts
* **Arrow keys:** selects which entry point to use for artifact generation

## Sample Usage

```shell
cargo run -- --file /tmp/my_file.x --dslx_stdlib_path $HOME/opt/xlsynth/latest/xls/dslx/stdlib/
```
