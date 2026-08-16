# kanso

A fast, riceable terminal text editor.

Kanso is non-modal and keyboard-first. The default keybindings follow VS
Code. Without any configuration you get:

- Syntax highlighting
- LSP completions and hover docs
- Autocomplete as you type
- File explorer and multiple buffers
- Indentation detected per file, with auto-indent and block indent
- Auto-closing brackets and quotes, surround-selection
- Mouse support: click, drag-select, shift+click, wheel scroll
- Undo/redo that groups typing into whole words

## Usage

```bash
kanso file.rs            # or several: kanso a.rs b.ts
```

Missing files are created on first save. `Ctrl+W` closes the buffer, and
closing the last one quits. Unsaved changes always warn first.

`Ctrl+E` opens the file explorer in the current file's directory. Use
`Up`/`Down` or type a letter to select, `Enter` to open a file or enter a
directory, `Backspace` to go up, `Esc` to close. Files you open become
buffers; cycle them with `Ctrl+PgDn` and `Ctrl+PgUp`.

## Keys

`F1` lists every active binding with its command id, including anything you
rebound. Arrows and page keys scroll the list, `Esc` closes it.

| Keys | Action |
| --- | --- |
| `F1` | help, lists all keybindings |
| `Ctrl+S` / `Ctrl+W` | save / close buffer |
| `Ctrl+E` or `Ctrl+O` | file explorer |
| `Ctrl+PgDn` / `Ctrl+PgUp` | next / previous buffer |
| `Ctrl+Z` / `Ctrl+Y` | undo / redo |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | copy / cut / paste (line if no selection) |
| `Ctrl+A` | select all |
| `Tab` / `Shift+Tab` | indent / outdent line or selection |
| `Alt+Up/Down` | move line, add `Shift` to duplicate |
| `Ctrl+Left/Right` | word movement, add `Shift` to select |
| `Ctrl+Space` | completion, `Tab` or `Enter` accepts, `Esc` dismisses |
| `Alt+H` | hover docs, or rest the mouse on a symbol |
| `Alt+L` | language server overview |

Every action is a command with a stable id (`file.save`, `cursor.word_left`
and so on), so any key can be bound to any command.

## Configuration

Everything lives in `~/.config/kanso/`. All keys are optional.

```toml
# config.toml
tab_width = 4            # fallback width, and how a tab character is drawn
insert_spaces = true     # fallback when a file has no indentation to detect
detect_indentation = true
auto_indent = true
line_numbers = true
syntax_highlighting = true
auto_pairs = true
mouse = true
lsp = true
cursor_style = "block"   # block, bar, underline
theme = "my-theme"       # themes/my-theme.toml

[language_servers]       # override or add servers, any stdio server works
rust = "rust-analyzer"
```

```toml
# keybindings.toml, chords are space-separated
[keybindings]
"ctrl+shift+d" = "editor.duplicate_line"
"ctrl+k ctrl+w" = "app.quit"
```

```toml
# themes/my-theme.toml
[colors]
"editor.background" = "#171a21"
"syntax.keyword" = "#81a1c1"
```

Theme roles: `editor.background/foreground/selection`,
`line_number.normal/active`, `statusline.background/foreground`,
`ui.accent/warning/error`, `syntax.keyword/string/comment/function/type/number`,
`popup.background/foreground/selected`.

## Indentation

Kanso reads each file when you open it and matches whatever it already
uses, tabs or spaces and how wide, so you do not reindent someone else's
code by accident. The statusline shows the result, for example
`Spaces: 2`. Files with no indentation fall back to `insert_spaces` and
`tab_width`, and setting `detect_indentation = false` always uses those.

New lines keep the indentation of the line above and add a level after an
opening bracket. Pressing `Enter` between a pair of brackets puts the
closing one on its own line. Typing a closing bracket on a blank line
lines it up with its opener. `Tab` and `Shift+Tab` indent and outdent the
selected lines, or the current one when nothing is selected, and
`Backspace` in leading whitespace removes a whole level at a time.

## Language servers

Kanso finds servers on its own: your config first, then `PATH` and the
common install directories (`~/.cargo/bin`, `~/go/bin`, `~/.bun`). Defaults
exist for `rust-analyzer`, `gopls`, `clangd`, `pylsp`, and
TypeScript/JavaScript. For TypeScript the native LSP built into TypeScript
7 is preferred, so `bun install -g typescript` works without node. Press
`Alt+L` to see what was detected.

## Building

```bash
cargo build --release
./target/release/kanso file.rs
```

## License

MIT
