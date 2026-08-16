# kanso

A fast, riceable terminal text editor.

Kanso is non-modal and keyboard-first. The default keybindings follow VS
Code. Without any configuration you get:

- Syntax highlighting
- LSP completions and hover docs
- Autocomplete as you type
- Find and replace, with every match highlighted
- Fuzzy file finder over the whole project
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

`Ctrl+P` opens the fuzzy file finder. Type any letters that appear in the
path, in order, and the closest matches rise to the top; `Up`/`Down`
choose, `Enter` opens, `Esc` closes. It searches the enclosing git
repository, or the current file's directory when there is no repository,
skipping hidden files, the usual build directories (`target`,
`node_modules`, `dist`, `build`, `vendor`, `venv`, `__pycache__`), and
plain file and `*.ext` entries from the project's top-level `.gitignore`.

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
| `Ctrl+P` | fuzzy file finder |
| `Ctrl+F` / `Ctrl+H` | find / find and replace |
| `F3` / `Shift+F3` | next / previous match |
| `Alt+C` | toggle case sensitivity |
| `Alt+R` | replace every match |
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
`popup.background/foreground/selected`, `search.match/current`.

## Search and replace

`Ctrl+F` opens the search bar above the statusline, seeded with the
selection if there is one. Every match is highlighted as you type and the
current one is picked out in a warmer colour, with a `3/12` counter on the
right. `Enter` or `F3` moves to the next match, `Shift+F3` to the previous
one, and both wrap around. `Esc` closes the bar and leaves the cursor on
the match, so `F3` afterwards keeps walking the same query without
reopening the bar.

`Alt+C` toggles case sensitivity, shown as `Aa` in the bar. `Ctrl+H` adds
a replace row, `Tab` moves between the two fields, `Enter` in the replace
field rewrites the current match and moves on, and `Alt+R` replaces every
match at once as a single undo step.

Searches are literal text, never regular expressions, and matches never
span lines.

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
