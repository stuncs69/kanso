use std::io;

use crate::buffer::{char_display_width, Position};
use crate::editor::Editor;

use super::frame::{str_width, Frame, Style, WIDE_CONTINUATION};
use super::statusline;
use super::terminal::Terminal;
use super::theme::Theme;

pub struct Renderer {
    frame: Frame,
    prev: Frame,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer {
            frame: Frame::new(0, 0),
            prev: Frame::new(0, 0),
        }
    }

    pub fn draw(
        &mut self,
        terminal: &mut Terminal,
        editor: &Editor,
        theme: &Theme,
    ) -> io::Result<()> {
        let (width, height) = (editor.viewport.width, editor.viewport.height);
        if width == 0 || height == 0 {
            return Ok(());
        }
        self.frame.reset(width, height, theme.text());
        self.render_text(editor, theme);
        statusline::render(&mut self.frame, editor, theme, height - 1);
        self.render_completion(editor, theme);
        self.render_hover(editor, theme);
        self.render_menu(editor, theme);
        self.render_explorer(editor, theme);

        let changed = self.frame.changed_rows(&self.prev);
        terminal.draw(&self.frame, &changed, cursor_screen_pos(editor))?;
        std::mem::swap(&mut self.frame, &mut self.prev);
        Ok(())
    }

    fn render_text(&mut self, editor: &Editor, theme: &Theme) {
        let vp = &editor.viewport;
        let buffer = &editor.buffer;
        let gutter = editor.gutter_width();
        let text_width = vp.width.saturating_sub(gutter) as usize;
        let text_style = theme.text();
        let selection_style = theme.selection();
        let selection = buffer.selection();
        let tab_width = editor.settings.tab_width;

        for row in 0..vp.text_height() {
            let line = vp.top_line + row;
            let y = row as u16;
            if line >= buffer.len_lines() {
                continue;
            }

            if gutter > 0 {
                let number = format!("{:>width$} ", line + 1, width = gutter as usize - 1);
                let style = theme.line_number(line == buffer.cursor.pos.line);
                self.frame.put_str(0, y, &number, style, gutter);
            }

            let selected_cols = selection.as_ref().and_then(|sel| sel.line_cols(line));
            let spans = editor
                .line_highlights
                .get(row)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut span_idx = 0usize;
            let mut display_col = 0usize;
            for (char_col, ch) in buffer.line_chars(line).enumerate() {
                if display_col >= vp.left_col + text_width {
                    break;
                }
                let w = char_display_width(ch, display_col, tab_width);
                if w == 0 {
                    continue;
                }
                let selected = selected_cols.is_some_and(|(start, end)| {
                    char_col >= start && char_col < end.unwrap_or(usize::MAX)
                });
                while span_idx < spans.len() && spans[span_idx].end <= char_col {
                    span_idx += 1;
                }
                let mut style = match spans.get(span_idx).filter(|s| s.start <= char_col) {
                    Some(span) => theme.syntax(span.kind),
                    None => text_style,
                };
                if selected {
                    style.bg = theme.editor_selection;
                }

                let mut head_drawn = false;
                for k in 0..w {
                    let col = display_col + k;
                    if col < vp.left_col {
                        continue;
                    }
                    let screen_col = col - vp.left_col;
                    if screen_col >= text_width {
                        break;
                    }
                    let x = gutter + screen_col as u16;
                    let cell = if k == 0 {
                        if ch != '\t' && w == 2 && screen_col + 1 >= text_width {
                            ' '
                        } else {
                            head_drawn = ch != '\t';
                            if ch == '\t' {
                                ' '
                            } else {
                                ch
                            }
                        }
                    } else if ch == '\t' {
                        ' '
                    } else if head_drawn {
                        WIDE_CONTINUATION
                    } else {
                        ' '
                    };
                    self.frame.set(x, y, cell, style);
                }
                display_col += w;
            }

            if let Some((_, None)) = selected_cols {
                if display_col >= vp.left_col && display_col < vp.left_col + text_width {
                    let x = gutter + (display_col - vp.left_col) as u16;
                    self.frame.set(x, y, ' ', selection_style);
                }
            }
        }
    }

    fn render_completion(&mut self, editor: &Editor, theme: &Theme) {
        let Some(state) = &editor.completion else {
            return;
        };
        let vp = &editor.viewport;
        let pos = editor.buffer.cursor.pos;
        if pos.line < vp.top_line {
            return;
        }
        let row = pos.line - vp.top_line;
        if row >= vp.text_height() {
            return;
        }

        let max_height = state.items.len().min(8);
        let below = vp.text_height() - row - 1;
        let (top_row, height) = if below > 0 {
            (row + 1, max_height.min(below))
        } else if row > 0 {
            let height = max_height.min(row);
            (row - height, height)
        } else {
            return;
        };

        let gutter = editor.gutter_width();
        let anchor = editor.buffer.display_col(
            Position::new(pos.line, state.prefix_start),
            editor.settings.tab_width,
        );
        let content_width = state
            .items
            .iter()
            .map(|item| str_width(item))
            .max()
            .unwrap_or(0)
            .min(34);
        let width = (content_width as u16 + 2).min(vp.width);
        let mut x = gutter + anchor.saturating_sub(vp.left_col) as u16;
        x = x.min(vp.width.saturating_sub(width));

        let offset = state.selected.saturating_sub(height - 1);
        for i in 0..height {
            let index = offset + i;
            let y = (top_row + i) as u16;
            let style = theme.popup(index == state.selected);
            for cell_x in x..x + width {
                self.frame.set(cell_x, y, ' ', style);
            }
            if let Some(item) = state.items.get(index) {
                self.frame.put_str(x + 1, y, item, style, x + width - 1);
            }
        }
    }

    fn render_hover(&mut self, editor: &Editor, theme: &Theme) {
        let Some(hover) = &editor.hover else {
            return;
        };
        let vp = &editor.viewport;
        if hover.anchor.line < vp.top_line {
            return;
        }
        let row = hover.anchor.line - vp.top_line;
        if row >= vp.text_height() {
            return;
        }

        let wanted = hover.lines.len().min(12);
        let (top_row, height) = if row >= wanted {
            (row - wanted, wanted)
        } else {
            let below = vp.text_height() - row - 1;
            if below == 0 {
                return;
            }
            (row + 1, wanted.min(below))
        };

        let gutter = editor.gutter_width();
        let anchor_col = editor
            .buffer
            .display_col(hover.anchor, editor.settings.tab_width);
        let content_width = hover
            .lines
            .iter()
            .map(|line| str_width(line))
            .max()
            .unwrap_or(0)
            .min(70);
        let width = (content_width as u16 + 2).min(vp.width);
        let mut x = gutter + anchor_col.saturating_sub(vp.left_col) as u16;
        x = x.min(vp.width.saturating_sub(width));

        for i in 0..height {
            let y = (top_row + i) as u16;
            let style = theme.popup(false);
            for cell_x in x..x + width {
                self.frame.set(cell_x, y, ' ', style);
            }
            if let Some(line) = hover.lines.get(i) {
                self.frame.put_str(x + 1, y, line, style, x + width - 1);
            }
        }
    }

    fn render_explorer(&mut self, editor: &Editor, theme: &Theme) {
        let Some(explorer) = &editor.explorer else {
            return;
        };
        let vp = &editor.viewport;
        let text_height = vp.text_height();
        if text_height < 4 || vp.width < 20 {
            return;
        }
        let title = crate::util::truncate_left(
            &crate::util::contract_home(&explorer.dir.display().to_string()),
            40,
        );
        let content_width = explorer
            .entries
            .iter()
            .map(|e| str_width(&e.name) + if e.is_dir { 1 } else { 0 })
            .max()
            .unwrap_or(0)
            .max(str_width(&title))
            .clamp(20, 44);
        let width = ((content_width + 4) as u16).min(vp.width);
        let rows = explorer.entries.len().min(text_height.saturating_sub(3));
        let height = rows + 2;
        let x = (vp.width - width) / 2;
        let top = (text_height - height) / 2;

        let base = theme.popup(false);
        let title_style = Style::fg_bg(theme.ui_accent, theme.popup_background).bold();
        let dir_style = Style::fg_bg(theme.ui_accent, theme.popup_background);
        let offset = explorer.selected.saturating_sub(rows.saturating_sub(1));
        for i in 0..height {
            let y = (top + i) as u16;
            for cell_x in x..x + width {
                self.frame.set(cell_x, y, ' ', base);
            }
            if i == 0 {
                self.frame
                    .put_str(x + 2, y, &title, title_style, x + width - 2);
                continue;
            }
            if i == 1 {
                continue;
            }
            let index = offset + i - 2;
            let Some(entry) = explorer.entries.get(index) else {
                continue;
            };
            let selected = index == explorer.selected;
            let mut style = if entry.is_dir {
                dir_style
            } else {
                theme.popup(false)
            };
            if selected {
                style.bg = theme.popup_selected;
            }
            if selected {
                for cell_x in x..x + width {
                    self.frame.set(cell_x, y, ' ', style);
                }
            }
            let name = if entry.is_dir {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            };
            self.frame.put_str(x + 2, y, &name, style, x + width - 2);
        }
    }

    fn render_menu(&mut self, editor: &Editor, theme: &Theme) {
        let Some(menu) = &editor.menu else {
            return;
        };
        let vp = &editor.viewport;
        let text_height = vp.text_height();
        if text_height < 4 || vp.width < 24 {
            return;
        }
        let content_width = menu
            .lines
            .iter()
            .chain(std::iter::once(&menu.footer))
            .map(|line| str_width(line))
            .max()
            .unwrap_or(0)
            .max(str_width(&menu.title));
        let width = ((content_width + 4) as u16).min(vp.width);
        let visible = editor.menu_visible_rows().min(menu.lines.len()).max(1);
        let height = visible + 3;
        let x = (vp.width - width) / 2;
        let top = (text_height - height) / 2;

        let base = theme.popup(false);
        let title_style = Style::fg_bg(theme.ui_accent, theme.popup_background).bold();
        let footer_style = Style::fg_bg(theme.line_number_normal, theme.popup_background);
        for i in 0..height {
            let y = (top + i) as u16;
            for cell_x in x..x + width {
                self.frame.set(cell_x, y, ' ', base);
            }
            if i == 0 {
                self.frame
                    .put_str(x + 2, y, &menu.title, title_style, x + width - 2);
                if menu.lines.len() > visible {
                    let shown = (menu.scroll + visible).min(menu.lines.len());
                    let position = format!("{shown}/{}", menu.lines.len());
                    let px = x + width - 2 - str_width(&position) as u16;
                    self.frame
                        .put_str(px, y, &position, footer_style, x + width - 2);
                }
            } else if i == 1 {
                continue;
            } else if i == height - 1 {
                self.frame
                    .put_str(x + 2, y, &menu.footer, footer_style, x + width - 2);
            } else if let Some(line) = menu.lines.get(menu.scroll + i - 2) {
                self.frame.put_str(x + 2, y, line, base, x + width - 2);
            }
        }
    }
}

fn cursor_screen_pos(editor: &Editor) -> Option<(u16, u16)> {
    let vp = &editor.viewport;
    let pos = editor.buffer.cursor.pos;
    let gutter = editor.gutter_width();
    let display_col = editor.buffer.display_col(pos, editor.settings.tab_width);

    if pos.line < vp.top_line || pos.line >= vp.top_line + vp.text_height() {
        return None;
    }
    let text_width = vp.width.saturating_sub(gutter) as usize;
    if display_col < vp.left_col || display_col >= vp.left_col + text_width {
        return None;
    }
    Some((
        gutter + (display_col - vp.left_col) as u16,
        (pos.line - vp.top_line) as u16,
    ))
}
