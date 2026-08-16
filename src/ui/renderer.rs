use std::io;

use crate::buffer::{char_display_width, Position};
use crate::editor::search::{self, Field};
use crate::editor::Editor;

use super::frame::{str_width, Frame, Style, WIDE_CONTINUATION};
use super::statusline;
use super::terminal::Terminal;
use super::theme::Theme;

const FIELD_X: u16 = 11;
const MIN_FIELD_WIDTH: u16 = 20;

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
        if editor.viewport.width == 0 || editor.viewport.height == 0 {
            return Ok(());
        }
        let cursor = self.compose(editor, theme);
        let changed = self.frame.changed_rows(&self.prev);
        terminal.draw(&self.frame, &changed, cursor)?;
        std::mem::swap(&mut self.frame, &mut self.prev);
        Ok(())
    }

    fn compose(&mut self, editor: &Editor, theme: &Theme) -> Option<(u16, u16)> {
        let (width, height) = (editor.viewport.width, editor.viewport.height);
        self.frame.reset(width, height, theme.text());
        self.render_text(editor, theme);
        statusline::render(&mut self.frame, editor, theme, height - 1);
        self.render_completion(editor, theme);
        self.render_hover(editor, theme);
        let mut cursor = cursor_screen_pos(editor);
        if let Some(at) = self.render_search(editor, theme) {
            cursor = Some(at);
        }
        self.render_menu(editor, theme);
        self.render_explorer(editor, theme);
        if let Some(at) = self.render_finder(editor, theme) {
            cursor = Some(at);
        }
        cursor
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
        let all_matches = editor
            .search
            .as_ref()
            .map(|s| s.matches.as_slice())
            .unwrap_or(&[]);
        let current_match = editor
            .search
            .as_ref()
            .and_then(crate::editor::search::Search::current_match);

        for row in 0..vp.text_height() {
            let line = vp.top_line + row;
            let y = row as u16;
            if line >= buffer.len_lines() {
                continue;
            }
            let line_matches = search::matches_on_line(all_matches, line);

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
                if let Some(hit) = line_matches
                    .iter()
                    .find(|hit| char_col >= hit.start && char_col < hit.end)
                {
                    style.bg = if current_match.as_ref() == Some(hit) {
                        theme.search_current
                    } else {
                        theme.search_match
                    };
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

    fn render_search(&mut self, editor: &Editor, theme: &Theme) -> Option<(u16, u16)> {
        let search = editor.search.as_ref()?;
        let vp = &editor.viewport;
        let text_height = vp.text_height();
        let height = if search.replacing { 2usize } else { 1 };
        if text_height <= height || vp.width < 28 {
            return None;
        }
        let top = (text_height - height) as u16;
        let base = theme.popup(false);
        let label_style = Style::fg_bg(theme.ui_accent, theme.popup_background).bold();
        let hint_style = Style::fg_bg(theme.line_number_normal, theme.popup_background);

        let case = if search.case_sensitive { "Aa   " } else { "" };
        let counter = format!("{case}{}", search.counter());
        let hint = if search.replacing {
            "esc close · tab switch · enter replace · alt+r all"
        } else {
            "esc close · enter next · ctrl+h replace"
        };

        let field_x = FIELD_X;
        let mut cursor = None;
        for row in 0..height {
            let y = top + row as u16;
            for x in 0..vp.width {
                self.frame.set(x, y, ' ', base);
            }
            let query_row = row == 0;
            let label = if query_row { "Find" } else { "Replace" };
            let field = if query_row {
                Field::Query
            } else {
                Field::Replacement
            };
            let active = search.field == field;
            let style = if active { label_style } else { hint_style };
            self.frame.put_str(2, y, label, style, field_x);

            let candidates: Vec<(String, Style)> = if !query_row {
                vec![(hint.to_string(), hint_style)]
            } else if search.replacing {
                vec![(counter.clone(), label_style)]
            } else {
                vec![
                    (format!("{hint}   {counter}"), label_style),
                    (counter.clone(), label_style),
                ]
            };
            let field_max = candidates
                .iter()
                .find_map(|(text, style)| self.fit_right(text, y, field_x, vp.width, *style))
                .unwrap_or(vp.width);

            let prompt = if query_row {
                &search.query
            } else {
                &search.replacement
            };
            let at =
                self.render_field(prompt.chars(), prompt.cursor(), field_x, y, base, field_max);
            if active {
                cursor = Some(at);
            }
        }
        cursor
    }

    fn fit_right(
        &mut self,
        text: &str,
        y: u16,
        field_x: u16,
        width: u16,
        style: Style,
    ) -> Option<u16> {
        let text_width = str_width(text) as u16;
        if text_width + field_x + MIN_FIELD_WIDTH > width {
            return None;
        }
        let x = width - text_width - 1;
        self.frame.put_str(x, y, text, style, width);
        Some(x.saturating_sub(2))
    }

    fn render_field(
        &mut self,
        chars: &[char],
        cursor: usize,
        x: u16,
        y: u16,
        style: Style,
        max_x: u16,
    ) -> (u16, u16) {
        let field_width = max_x.saturating_sub(x) as usize;
        if field_width == 0 {
            return (x, y);
        }
        let offset = cursor.saturating_sub(field_width.saturating_sub(1));
        let shown: String = chars.iter().skip(offset).take(field_width).collect();
        self.frame.put_str(x, y, &shown, style, max_x);
        let prefix: String = chars[offset..cursor].iter().collect();
        let cursor_x = (x + str_width(&prefix) as u16).min(max_x.saturating_sub(1));
        (cursor_x, y)
    }

    fn render_finder(&mut self, editor: &Editor, theme: &Theme) -> Option<(u16, u16)> {
        let finder = editor.finder.as_ref()?;
        let vp = &editor.viewport;
        let text_height = vp.text_height();
        if text_height < 7 || vp.width < 34 {
            return None;
        }
        let rows = editor.finder_visible_rows().min(finder.hits.len().max(1));
        let height = rows + 3;
        let width = (vp.width.saturating_sub(4)).min(76);
        let x = (vp.width - width) / 2;
        let top = ((text_height - height) / 3) as u16;

        let base = theme.popup(false);
        let selected_base = theme.popup(true);
        let title_style = Style::fg_bg(theme.ui_accent, theme.popup_background).bold();
        let hint_style = Style::fg_bg(theme.line_number_normal, theme.popup_background);
        let inner_max = x + width - 2;

        for row in 0..height {
            let y = top + row as u16;
            for cell_x in x..x + width {
                self.frame.set(cell_x, y, ' ', base);
            }
        }
        self.frame
            .put_str(x + 2, top, "Open File", title_style, inner_max);
        let counter = finder.counter();
        let counter_x = inner_max.saturating_sub(str_width(&counter) as u16);
        self.frame
            .put_str(counter_x, top, &counter, hint_style, inner_max);

        let query_y = top + 1;
        self.frame
            .put_str(x + 2, query_y, "›", title_style, inner_max);
        let cursor = self.render_field(
            finder.query.chars(),
            finder.query.cursor(),
            x + 4,
            query_y,
            base,
            inner_max,
        );

        let offset = finder.selected.saturating_sub(rows.saturating_sub(1));
        for row in 0..rows {
            let y = top + 3 + row as u16;
            let index = offset + row;
            let Some(hit) = finder.hits.get(index) else {
                if index == 0 {
                    self.frame
                        .put_str(x + 2, y, "no matching files", hint_style, inner_max);
                }
                continue;
            };
            let selected = index == finder.selected;
            let row_base = if selected { selected_base } else { base };
            if selected {
                for cell_x in x..x + width {
                    self.frame.set(cell_x, y, ' ', row_base);
                }
            }
            let path = finder.hit_path(hit);
            let mut dir_style = hint_style;
            let mut match_style = Style::fg_bg(theme.ui_accent, theme.popup_background).bold();
            if selected {
                dir_style.bg = theme.popup_selected;
                match_style.bg = theme.popup_selected;
            }
            self.put_path(
                x + 2,
                y,
                path,
                &hit.positions,
                row_base,
                dir_style,
                match_style,
                inner_max,
            );
        }
        Some(cursor)
    }

    #[allow(clippy::too_many_arguments)]
    fn put_path(
        &mut self,
        x: u16,
        y: u16,
        path: &str,
        positions: &[usize],
        base: Style,
        dir: Style,
        highlight: Style,
        max_x: u16,
    ) {
        let name_start = path
            .rfind('/')
            .map(|i| path[..i].chars().count() + 1)
            .unwrap_or(0);
        let available = max_x.saturating_sub(x) as usize;
        let count = path.chars().count();
        let mut cursor_x = x;
        let skip = if count > available {
            count - available + 1
        } else {
            0
        };
        if skip > 0 {
            cursor_x = self.frame.put_str(cursor_x, y, "…", dir, max_x);
        }
        let mut encoded = [0u8; 4];
        for (i, ch) in path.chars().enumerate().skip(skip) {
            let style = if positions.contains(&i) {
                highlight
            } else if i < name_start {
                dir
            } else {
                base
            };
            cursor_x = self
                .frame
                .put_str(cursor_x, y, ch.encode_utf8(&mut encoded), style, max_x);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::config::Settings;
    use crate::editor::command::Command;

    fn editor_with(text: &str, width: u16, height: u16) -> Editor {
        let mut buffer = Buffer::scratch();
        buffer.insert_text(text);
        buffer.commit_undo_group();
        buffer.cursor.pos = Position::new(0, 0);
        let mut editor = Editor::new(buffer, Settings::default());
        editor.viewport.resize(width, height);
        editor.sync_viewport();
        editor
    }

    fn type_text(editor: &mut Editor, text: &str) {
        for c in text.chars() {
            editor.execute(Command::InsertChar(c));
        }
    }

    fn row_text(renderer: &Renderer, y: u16) -> String {
        renderer
            .frame
            .row(y)
            .iter()
            .map(|cell| cell.ch)
            .filter(|c| *c != WIDE_CONTINUATION)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn backgrounds(renderer: &Renderer, y: u16) -> Vec<super::super::frame::Color> {
        renderer.frame.row(y).iter().map(|c| c.style.bg).collect()
    }

    #[test]
    fn search_bar_shows_the_query_counter_and_cursor() {
        let theme = Theme::default();
        let mut renderer = Renderer::new();
        let mut editor = editor_with("foo\nbar foo\n", 60, 10);
        editor.execute(Command::Find);
        type_text(&mut editor, "foo");

        let cursor = renderer.compose(&editor, &theme).unwrap();
        let bar = row_text(&renderer, 8);
        assert!(bar.starts_with("  Find"), "{bar:?}");
        assert!(bar.contains("foo"), "{bar:?}");
        assert!(bar.ends_with("1/2"), "{bar:?}");
        assert_eq!(cursor, (FIELD_X + 3, 8));
    }

    #[test]
    fn search_bar_keeps_the_query_visible_and_drops_hints_when_narrow() {
        let theme = Theme::default();
        let mut renderer = Renderer::new();

        let mut wide = editor_with("foo\nbar foo\n", 120, 10);
        wide.execute(Command::Find);
        type_text(&mut wide, "foo");
        renderer.compose(&wide, &theme);
        let bar = row_text(&renderer, 8);
        assert!(bar.contains("esc close"), "{bar:?}");
        assert!(bar.ends_with("1/2"), "{bar:?}");
        assert_eq!(&bar[FIELD_X as usize..FIELD_X as usize + 3], "foo");

        let mut narrow = editor_with("foo\nbar foo\n", 40, 10);
        narrow.execute(Command::Find);
        type_text(&mut narrow, "foo");
        renderer.compose(&narrow, &theme);
        let bar = row_text(&renderer, 8);
        assert!(!bar.contains("esc close"), "{bar:?}");
        assert!(bar.ends_with("1/2"), "{bar:?}");
        assert_eq!(&bar[FIELD_X as usize..FIELD_X as usize + 3], "foo");
    }

    #[test]
    fn replace_bar_adds_a_second_row_and_moves_the_cursor() {
        let theme = Theme::default();
        let mut renderer = Renderer::new();
        let mut editor = editor_with("foo\n", 60, 10);
        editor.execute(Command::Find);
        type_text(&mut editor, "foo");
        editor.execute(Command::Replace);
        type_text(&mut editor, "ba");

        let cursor = renderer.compose(&editor, &theme).unwrap();
        assert!(row_text(&renderer, 7).starts_with("  Find"));
        let replace_row = row_text(&renderer, 8);
        assert!(replace_row.starts_with("  Replace"), "{replace_row:?}");
        assert!(replace_row.contains("ba"), "{replace_row:?}");
        assert_eq!(cursor, (FIELD_X + 2, 8));
    }

    #[test]
    fn matches_are_painted_and_the_current_one_stands_out() {
        let theme = Theme::default();
        let mut renderer = Renderer::new();
        let mut editor = editor_with("foo foo\n", 60, 10);
        editor.execute(Command::Find);
        type_text(&mut editor, "foo");
        renderer.compose(&editor, &theme);

        let gutter = editor.gutter_width() as usize;
        let row = backgrounds(&renderer, 0);
        assert_eq!(row[gutter], theme.search_current);
        assert_eq!(row[gutter + 2], theme.search_current);
        assert_eq!(row[gutter + 3], theme.editor_background);
        assert_eq!(row[gutter + 4], theme.search_match);
        assert_eq!(row[gutter + 6], theme.search_match);
    }

    #[test]
    fn text_renders_without_search_backgrounds_when_closed() {
        let theme = Theme::default();
        let mut renderer = Renderer::new();
        let mut editor = editor_with("foo foo\n", 60, 10);
        editor.execute(Command::Find);
        type_text(&mut editor, "foo");
        editor.execute(Command::Cancel);
        renderer.compose(&editor, &theme);

        let gutter = editor.gutter_width() as usize;
        let row = backgrounds(&renderer, 0);
        assert_eq!(row[gutter + 4], theme.editor_background);
        assert_eq!(row[gutter], theme.editor_selection);
    }

    #[test]
    fn finder_popup_lists_matches_with_the_query_row() {
        let theme = Theme::default();
        let mut renderer = Renderer::new();
        let dir = std::env::temp_dir().join(format!("kanso-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join("readme.md"), "").unwrap();

        let mut editor = Editor::new(
            Buffer::from_path(&dir.join("readme.md")).unwrap(),
            Settings::default(),
        );
        editor.viewport.resize(60, 16);
        editor.sync_viewport();
        editor.execute(Command::FindFile);
        type_text(&mut editor, "main");

        let cursor = renderer.compose(&editor, &theme).unwrap();
        let rows: Vec<String> = (0..15).map(|y| row_text(&renderer, y)).collect();
        let joined = rows.join("\n");
        assert!(joined.contains("Open File"), "{joined}");
        assert!(joined.contains("› main"), "{joined}");
        assert!(joined.contains("src/main.rs"), "{joined}");
        assert!(!joined.contains("readme.md"), "{joined}");
        assert!(row_text(&renderer, cursor.1).contains("› main"));
        assert_eq!(renderer.frame.row(cursor.1)[cursor.0 as usize].ch, ' ');
    }
}
