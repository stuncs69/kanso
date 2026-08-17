use crate::buffer::{Buffer, IndentStyle, Motion, Position};
use crate::config::Settings;
use crate::syntax::{Highlighter, Span};
use crate::ui::viewport::Viewport;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::command::Command;
use super::completion::{self, Completion};
use super::explorer::Explorer;
use super::finder::Finder;
use super::search::{self, Field, Match, Search};
use crate::plugins::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Error,
}

#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub kind: MessageKind,
    pub expires_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct HoverInfo {
    pub anchor: Position,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Menu {
    pub title: String,
    pub lines: Vec<String>,
    pub footer: String,
    pub scroll: usize,
}

impl Menu {
    pub fn new(title: impl Into<String>, lines: Vec<String>, footer: impl Into<String>) -> Self {
        Menu {
            title: title.into(),
            lines,
            footer: footer.into(),
            scroll: 0,
        }
    }
}

pub struct Editor {
    pub buffer: Buffer,
    pub viewport: Viewport,
    pub settings: Settings,
    pub status: Option<StatusMessage>,
    pub line_highlights: Vec<Vec<Span>>,
    pub completion: Option<Completion>,
    pub hover: Option<HoverInfo>,
    pub menu: Option<Menu>,
    pub explorer: Option<Explorer>,
    pub finder: Option<Finder>,
    pub search: Option<Search>,
    pub lsp_indicator: Option<String>,
    background: Vec<Buffer>,
    highlighter: Highlighter,
    clipboard: String,
    search_memo: Option<(String, bool)>,
    lsp_completion_wanted: Option<usize>,
    lsp_hover_wanted: Option<Position>,
    menu_wanted: bool,
    help_wanted: bool,
    plugin_calls: Vec<usize>,
    events: Vec<Event>,
    quitting: bool,
    quit_confirmed: bool,
}

impl Editor {
    pub fn new(buffer: Buffer, settings: Settings) -> Self {
        let highlighter = Highlighter::new(buffer.path());
        let mut editor = Editor {
            buffer,
            viewport: Viewport::default(),
            settings,
            status: None,
            line_highlights: Vec::new(),
            completion: None,
            hover: None,
            menu: None,
            explorer: None,
            finder: None,
            search: None,
            lsp_indicator: None,
            background: Vec::new(),
            highlighter,
            clipboard: String::new(),
            search_memo: None,
            lsp_completion_wanted: None,
            lsp_hover_wanted: None,
            menu_wanted: false,
            help_wanted: false,
            plugin_calls: Vec::new(),
            events: Vec::new(),
            quitting: false,
            quit_confirmed: false,
        };
        editor.apply_indent_settings();
        editor.emit(Event::buffer_opened);
        editor
    }

    fn apply_indent_settings(&mut self) {
        let fallback = IndentStyle {
            use_spaces: self.settings.insert_spaces,
            width: self.settings.tab_width,
        };
        if self.settings.detect_indentation {
            self.buffer.detect_indent(fallback);
        } else {
            self.buffer.set_indent(fallback);
        }
    }

    pub fn should_quit(&self) -> bool {
        self.quitting
    }

    pub fn add_background(&mut self, buffer: Buffer) {
        self.events.push(Event::buffer_opened(&buffer));
        self.background.push(buffer);
    }

    fn emit(&mut self, event: fn(&Buffer) -> Event) {
        let event = event(&self.buffer);
        self.events.push(event);
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn take_plugin_calls(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.plugin_calls)
    }

    pub fn buffer_count(&self) -> usize {
        1 + self.background.len()
    }

    pub fn open_file(&mut self, path: &Path) {
        let target = canonical(path);
        if self.buffer.path().map(canonical) == Some(target.clone()) {
            return;
        }
        if let Some(i) = self
            .background
            .iter()
            .position(|b| b.path().map(canonical) == Some(target.clone()))
        {
            let next = self.background.remove(i);
            let prev = std::mem::replace(&mut self.buffer, next);
            self.background.push(prev);
            self.after_buffer_switch();
            return;
        }
        match Buffer::from_path(&target) {
            Ok(next) => {
                let prev = std::mem::replace(&mut self.buffer, next);
                self.background.push(prev);
                self.after_buffer_switch();
                self.emit(Event::buffer_opened);
            }
            Err(e) => self.set_error(format!("cannot open {}: {e}", path.display())),
        }
    }

    fn next_buffer(&mut self) {
        if self.background.is_empty() {
            return;
        }
        let next = self.background.remove(0);
        let prev = std::mem::replace(&mut self.buffer, next);
        self.background.push(prev);
        self.after_buffer_switch();
    }

    fn prev_buffer(&mut self) {
        let Some(next) = self.background.pop() else {
            return;
        };
        let prev = std::mem::replace(&mut self.buffer, next);
        self.background.insert(0, prev);
        self.after_buffer_switch();
    }

    fn close_buffer(&mut self) {
        if self.buffer.is_dirty() && !self.quit_confirmed {
            self.quit_confirmed = true;
            self.set_error("Unsaved changes. Ctrl+S to save, or press again to discard");
            return;
        }
        if self.background.is_empty() {
            self.quitting = true;
            return;
        }
        self.buffer = self.background.remove(0);
        self.after_buffer_switch();
    }

    fn after_buffer_switch(&mut self) {
        self.highlighter = Highlighter::new(self.buffer.path());
        self.apply_indent_settings();
        self.buffer.take_dirty_from();
        self.completion = None;
        self.hover = None;
        self.search = None;
        self.quit_confirmed = false;
        self.viewport.top_line = 0;
        self.viewport.left_col = 0;
        self.sync_viewport();
    }

    pub fn set_info(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage {
            text: text.into(),
            kind: MessageKind::Info,
            expires_at: None,
        });
    }

    pub fn set_error(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage {
            text: text.into(),
            kind: MessageKind::Error,
            expires_at: None,
        });
    }

    pub fn set_info_for(&mut self, text: impl Into<String>, duration: Duration) {
        self.set_info(text);
        if let Some(status) = self.status.as_mut() {
            status.expires_at = Some(Instant::now() + duration);
        }
    }

    pub fn expire_status(&mut self) {
        if let Some(at) = self.status.as_ref().and_then(|s| s.expires_at) {
            if Instant::now() >= at {
                self.status = None;
            }
        }
    }

    pub fn gutter_width(&self) -> u16 {
        if !self.settings.line_numbers {
            return 0;
        }
        let digits = self.buffer.len_lines().max(1).ilog10() as u16 + 1;
        digits + 2
    }

    fn page_rows(&self) -> usize {
        self.viewport.text_height().max(1)
    }

    pub fn sync_viewport(&mut self) {
        let gutter = self.gutter_width();
        let display_col = self
            .buffer
            .display_col(self.buffer.cursor.pos, self.settings.tab_width);
        self.viewport
            .ensure_visible(self.buffer.cursor.pos.line, display_col, gutter);
        self.update_highlights();
    }

    fn update_highlights(&mut self) {
        if !self.settings.syntax_highlighting {
            self.line_highlights.clear();
            return;
        }
        if let Some(line) = self.buffer.take_dirty_from() {
            self.highlighter.invalidate_from(line);
        }
        let top = self.viewport.top_line;
        let rows = self.viewport.text_height();
        let mut highlights = Vec::with_capacity(rows);
        for row in 0..rows {
            let line = top + row;
            let spans = if line < self.buffer.len_lines() {
                self.highlighter.line_spans(&self.buffer, line)
            } else {
                Vec::new()
            };
            highlights.push(spans);
        }
        self.line_highlights = highlights;
    }

    pub fn paste_text(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.buffer.insert_text(&text);
        self.sync_viewport();
    }

    pub fn mouse_down(&mut self, x: u16, y: u16, extend: bool) {
        let Some(pos) = self.position_at_screen(x, y) else {
            return;
        };
        self.quit_confirmed = false;
        self.status = None;
        self.completion = None;
        self.hover = None;
        self.menu = None;
        self.explorer = None;
        self.finder = None;
        self.close_search();
        self.buffer.set_cursor_pos(pos, extend);
        self.sync_viewport();
    }

    pub fn mouse_drag(&mut self, x: u16, y: u16) {
        let Some(pos) = self.position_at_screen(x, y) else {
            return;
        };
        self.buffer.set_cursor_pos(pos, true);
        self.sync_viewport();
    }

    pub fn scroll_viewport(&mut self, delta: isize) {
        let max_top = self.buffer.len_lines().saturating_sub(1) as isize;
        let top = self.viewport.top_line as isize + delta;
        self.viewport.top_line = top.clamp(0, max_top) as usize;
        self.completion = None;
        self.hover = None;
        self.menu = None;
        self.finder = None;
        self.update_highlights();
    }

    pub fn text_position_at(&self, x: u16, y: u16) -> Option<Position> {
        self.position_at_screen(x, y)
    }

    fn position_at_screen(&self, x: u16, y: u16) -> Option<Position> {
        let row = y as usize;
        if row >= self.viewport.text_height() {
            return None;
        }
        let line = (self.viewport.top_line + row).min(self.buffer.len_lines().saturating_sub(1));
        let gutter = self.gutter_width();
        let target = self.viewport.left_col + x.saturating_sub(gutter) as usize;
        let col = self
            .buffer
            .col_at_display_col(line, target, self.settings.tab_width);
        Some(Position::new(line, col))
    }

    pub fn execute(&mut self, command: Command) {
        if !matches!(command, Command::Quit | Command::CloseBuffer) {
            self.quit_confirmed = false;
        }
        if self.menu.is_some() {
            self.menu_command(command);
            return;
        }
        if self.explorer.is_some() {
            self.explorer_command(command);
            return;
        }
        if self.finder.is_some() {
            self.finder_command(command);
            return;
        }
        if self.search.is_some() {
            self.search_command(command);
            return;
        }
        self.hover = None;
        if self.handle_completion_command(command) {
            self.sync_viewport();
            return;
        }
        self.status = None;
        match command {
            Command::InsertChar(c) => self.type_char(c),
            Command::InsertNewline => self.smart_newline(),
            Command::InsertTab => self.smart_tab(),
            Command::Outdent => self.smart_outdent(),
            Command::Backspace => self.smart_backspace(),
            Command::Delete => self.buffer.delete_forward(),
            Command::Move(motion) => self.buffer.move_cursor(motion, false, self.page_rows()),
            Command::Select(motion) => self.buffer.move_cursor(motion, true, self.page_rows()),
            Command::SelectAll => self.buffer.select_all(),
            Command::Cancel => self.buffer.clear_selection(),
            Command::Undo => {
                if !self.buffer.undo() {
                    self.set_info("Nothing to undo");
                }
            }
            Command::Redo => {
                if !self.buffer.redo() {
                    self.set_info("Nothing to redo");
                }
            }
            Command::Copy => self.copy(false),
            Command::Cut => self.copy(true),
            Command::Paste => self.paste_clipboard(),
            Command::DuplicateLine => self.buffer.duplicate_line(),
            Command::MoveLineUp => self.buffer.move_line_up(),
            Command::MoveLineDown => self.buffer.move_line_down(),
            Command::TriggerCompletion => {}
            Command::Hover => self.lsp_hover_wanted = Some(self.buffer.cursor.pos),
            Command::LspMenu => self.menu_wanted = true,
            Command::Help => self.help_wanted = true,
            Command::Explorer => self.open_explorer(),
            Command::FindFile => self.open_finder(),
            Command::Find => self.open_search(false),
            Command::Replace | Command::ReplaceAll => self.open_search(true),
            Command::FindNext => self.repeat_search(1),
            Command::FindPrevious => self.repeat_search(-1),
            Command::ToggleSearchCase => self.toggle_remembered_case(),
            Command::NextBuffer => self.next_buffer(),
            Command::PrevBuffer => self.prev_buffer(),
            Command::CloseBuffer => self.close_buffer(),
            Command::Plugin(handle) => self.plugin_calls.push(handle),
            Command::Save => self.save(),
            Command::Quit => self.request_quit(),
        }
        self.update_completion_after(command);
        self.sync_viewport();
    }

    fn handle_completion_command(&mut self, command: Command) -> bool {
        if self.completion.is_none() {
            return false;
        }
        match command {
            Command::Move(Motion::Up) => {
                self.cycle_completion(-1);
                true
            }
            Command::Move(Motion::Down) => {
                self.cycle_completion(1);
                true
            }
            Command::InsertTab | Command::InsertNewline => {
                self.accept_completion();
                true
            }
            Command::Cancel => {
                self.completion = None;
                true
            }
            _ => false,
        }
    }

    fn cycle_completion(&mut self, delta: isize) {
        if let Some(state) = self.completion.as_mut() {
            let len = state.items.len() as isize;
            state.selected = (state.selected as isize + delta).rem_euclid(len) as usize;
        }
    }

    fn update_completion_after(&mut self, command: Command) {
        match command {
            Command::InsertChar(c) if completion::is_word_char(c) => {
                self.refresh_completion(false);
                self.queue_lsp_completion();
            }
            Command::InsertChar('.') | Command::InsertChar(':') => {
                self.completion = None;
                self.queue_lsp_completion();
            }
            Command::Backspace if self.completion.is_some() => {
                self.refresh_completion(false);
                self.queue_lsp_completion();
            }
            Command::TriggerCompletion => {
                self.refresh_completion(true);
                self.queue_lsp_completion();
                if self.completion.is_none() && self.lsp_indicator.is_none() {
                    self.set_info("No completions");
                }
            }
            Command::Hover => {}
            _ => {
                self.completion = None;
                self.lsp_completion_wanted = None;
            }
        }
    }

    fn queue_lsp_completion(&mut self) {
        let (start, _) = completion::prefix_at_cursor(&self.buffer);
        self.lsp_completion_wanted = Some(start);
    }

    pub fn take_completion_request(&mut self) -> Option<usize> {
        self.lsp_completion_wanted.take()
    }

    pub fn take_hover_request(&mut self) -> Option<Position> {
        self.lsp_hover_wanted.take()
    }

    pub fn take_menu_request(&mut self) -> bool {
        std::mem::take(&mut self.menu_wanted)
    }

    pub fn take_help_request(&mut self) -> bool {
        std::mem::take(&mut self.help_wanted)
    }

    pub fn menu_visible_rows(&self) -> usize {
        self.viewport.text_height().saturating_sub(3).max(1)
    }

    fn menu_command(&mut self, command: Command) {
        let visible = self.menu_visible_rows();
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let max_scroll = menu.lines.len().saturating_sub(visible);
        let delta = match command {
            Command::Move(Motion::Up) | Command::Select(Motion::Up) => -1,
            Command::Move(Motion::Down) | Command::Select(Motion::Down) => 1,
            Command::Move(Motion::PageUp) => -(visible as isize),
            Command::Move(Motion::PageDown) => visible as isize,
            Command::Move(Motion::BufferStart) => -(menu.lines.len() as isize),
            Command::Move(Motion::BufferEnd) => menu.lines.len() as isize,
            Command::Cancel | Command::Help | Command::LspMenu | Command::Quit => {
                self.menu = None;
                return;
            }
            _ => return,
        };
        menu.scroll = (menu.scroll as isize + delta).clamp(0, max_scroll as isize) as usize;
    }

    pub fn apply_lsp_completions(&mut self, line: usize, prefix_start: usize, items: Vec<String>) {
        if self.buffer.cursor.pos.line != line {
            return;
        }
        let (start, prefix) = completion::prefix_at_cursor(&self.buffer);
        if start != prefix_start {
            return;
        }
        let mut filtered: Vec<String> = Vec::new();
        for item in items {
            let insertable = item
                .chars()
                .all(|c| completion::is_word_char(c) || c == '$');
            if insertable
                && item.starts_with(&prefix)
                && item != prefix
                && !filtered.contains(&item)
            {
                filtered.push(item);
            }
            if filtered.len() >= 50 {
                break;
            }
        }
        if filtered.is_empty() {
            return;
        }
        self.completion = Some(Completion {
            items: filtered,
            selected: 0,
            prefix_start,
        });
    }

    pub fn show_hover(&mut self, anchor: Position, text: &str) {
        let mut lines: Vec<String> = Vec::new();
        for line in text.lines() {
            let line = line.trim_end();
            if line.trim_start().starts_with("```") {
                continue;
            }
            if line.is_empty() && lines.last().is_some_and(|l| l.is_empty()) {
                continue;
            }
            lines.push(line.to_string());
        }
        while lines.first().is_some_and(|l| l.is_empty()) {
            lines.remove(0);
        }
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        if lines.is_empty() {
            return;
        }
        lines.truncate(12);
        self.hover = Some(HoverInfo { anchor, lines });
    }

    fn refresh_completion(&mut self, manual: bool) {
        let (start, prefix) = completion::prefix_at_cursor(&self.buffer);
        if !manual && prefix.chars().count() < 2 {
            self.completion = None;
            return;
        }
        let spec = crate::syntax::detect(self.buffer.path());
        let items = completion::collect_candidates(&self.buffer, spec, &prefix);
        self.completion = if items.is_empty() {
            None
        } else {
            Some(Completion {
                items,
                selected: 0,
                prefix_start: start,
            })
        };
    }

    fn accept_completion(&mut self) {
        let Some(state) = self.completion.take() else {
            return;
        };
        let item = &state.items[state.selected];
        let typed = self.buffer.cursor.pos.col - state.prefix_start;
        let suffix: String = item.chars().skip(typed).collect();
        if !suffix.is_empty() {
            self.buffer.insert_plain(&suffix);
        }
    }

    fn smart_tab(&mut self) {
        if let Some(sel) = self.buffer.selection() {
            if sel.start.line != sel.end.line {
                let unit = self.buffer.indent().unit();
                self.buffer
                    .indent_lines(sel.start.line, sel.end.line, &unit);
                return;
            }
        }
        self.buffer.insert_tab(self.settings.tab_width);
    }

    fn smart_outdent(&mut self) {
        let (first, last) = match self.buffer.selection() {
            Some(sel) => (sel.start.line, sel.end.line),
            None => {
                let line = self.buffer.cursor.pos.line;
                (line, line)
            }
        };
        let width = self.buffer.indent().width;
        self.buffer.dedent_lines(first, last, width);
    }

    fn smart_newline(&mut self) {
        if !self.settings.auto_indent {
            self.buffer.insert_newline();
            return;
        }
        let (start, end) = match self.buffer.selection() {
            Some(sel) => (sel.start, sel.end),
            None => (self.buffer.cursor.pos, self.buffer.cursor.pos),
        };
        let before: String = self.buffer.line_chars(start.line).take(start.col).collect();
        let after: String = self.buffer.line_chars(end.line).skip(end.col).collect();
        let leading: String = before
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let trimmed = before.trim_end();
        let colon = crate::syntax::detect(self.buffer.path()).is_some_and(|s| s.colon_indent);
        let opens = trimmed.ends_with(['{', '[', '(']) || (colon && trimmed.ends_with(':'));
        if !opens {
            self.buffer.insert_newline_indented(&leading, None);
            return;
        }
        let inner = format!("{leading}{}", self.buffer.indent().unit());
        if after.trim_start().starts_with(['}', ']', ')']) {
            self.buffer.insert_newline_indented(&inner, Some(&leading));
        } else {
            self.buffer.insert_newline_indented(&inner, None);
        }
    }

    fn outdent_before_closer(&mut self, c: char) {
        if !self.settings.auto_indent || !matches!(c, '}' | ']' | ')') {
            return;
        }
        if self.buffer.selection().is_some() {
            return;
        }
        let pos = self.buffer.cursor.pos;
        if pos.col == 0 || pos.col != self.buffer.indent_width_of(pos.line) {
            return;
        }
        let width = self.buffer.indent().width;
        self.buffer.dedent_lines(pos.line, pos.line, width);
    }

    fn type_char(&mut self, c: char) {
        if !self.settings.auto_pairs {
            self.outdent_before_closer(c);
            self.buffer.insert_char(c);
            return;
        }
        match c {
            '(' | '[' | '{' => {
                let close = closing_bracket(c);
                if !self.buffer.surround_selection(c, close) {
                    self.buffer.insert_pair(c, close);
                }
            }
            ')' | ']' | '}' => {
                if self.buffer.selection().is_none() && self.buffer.char_after_cursor() == Some(c) {
                    self.buffer.move_cursor(Motion::Right, false, 1);
                } else {
                    self.outdent_before_closer(c);
                    self.buffer.insert_char(c);
                }
            }
            '"' => {
                if self.buffer.surround_selection('"', '"') {
                    return;
                }
                let next = self.buffer.char_after_cursor();
                if next == Some('"') {
                    self.buffer.move_cursor(Motion::Right, false, 1);
                } else if next.is_none_or(|ch| ch.is_whitespace() || ")]},;".contains(ch)) {
                    self.buffer.insert_pair('"', '"');
                } else {
                    self.buffer.insert_char('"');
                }
            }
            _ => self.buffer.insert_char(c),
        }
    }

    fn smart_backspace(&mut self) {
        if self.buffer.selection().is_none() {
            if self.settings.auto_pairs {
                let pair = (
                    self.buffer.char_before_cursor(),
                    self.buffer.char_after_cursor(),
                );
                if let (Some(prev), Some(next)) = pair {
                    if matches!(
                        (prev, next),
                        ('(', ')') | ('[', ']') | ('{', '}') | ('"', '"')
                    ) {
                        self.buffer.delete_pair_around_cursor();
                        return;
                    }
                }
            }
            if let Some(count) = self.indent_backspace_count() {
                self.buffer.delete_before_cursor(count);
                return;
            }
        }
        self.buffer.backspace();
    }

    fn indent_backspace_count(&self) -> Option<usize> {
        let style = self.buffer.indent();
        if !style.use_spaces || style.width == 0 {
            return None;
        }
        let pos = self.buffer.cursor.pos;
        if pos.col == 0 || pos.col > self.buffer.indent_width_of(pos.line) {
            return None;
        }
        if self.buffer.leading_whitespace(pos.line).contains('\t') {
            return None;
        }
        let target = ((pos.col - 1) / style.width) * style.width;
        Some(pos.col - target)
    }

    fn copy(&mut self, cut: bool) {
        self.buffer.commit_undo_group();
        let (text, what) = match self.buffer.selected_text() {
            Some(text) => {
                if cut {
                    self.buffer.delete_selection();
                }
                (text, "selection")
            }
            None => {
                let line = self.buffer.cursor.pos.line;
                let text = self.buffer.line_with_newline(line);
                if cut {
                    self.buffer.delete_line(line);
                }
                (text, "line")
            }
        };
        self.clipboard = text;
        self.set_info(format!("{} {what}", if cut { "Cut" } else { "Copied" }));
    }

    fn paste_clipboard(&mut self) {
        if self.clipboard.is_empty() {
            self.set_info("Clipboard is empty");
        } else {
            let text = self.clipboard.clone();
            self.buffer.insert_text(&text);
        }
    }

    fn save(&mut self) {
        if self.buffer.path().is_none() {
            self.set_error("No file name. Start kanso with a file path to save");
            return;
        }
        match self.buffer.save() {
            Ok(()) => {
                let lines = self.buffer.len_lines();
                self.set_info(format!("Saved {} ({lines} lines)", self.buffer.file_name()));
                self.emit(Event::buffer_saved);
            }
            Err(e) => self.set_error(format!("Save failed: {e}")),
        }
    }

    fn request_quit(&mut self) {
        let any_dirty = self.buffer.is_dirty() || self.background.iter().any(Buffer::is_dirty);
        if any_dirty && !self.quit_confirmed {
            self.quit_confirmed = true;
            self.set_error("Unsaved changes. Ctrl+S to save, or press again to discard and quit");
        } else {
            self.quitting = true;
        }
    }

    fn open_explorer(&mut self) {
        let dir = self
            .buffer
            .path()
            .and_then(Path::parent)
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        match Explorer::open(&dir) {
            Ok(mut explorer) => {
                if let Some(name) = self.buffer.path().and_then(Path::file_name) {
                    explorer.select_name(&name.to_string_lossy());
                }
                self.completion = None;
                self.hover = None;
                self.menu = None;
                self.explorer = Some(explorer);
            }
            Err(e) => self.set_error(format!("explorer: {e}")),
        }
    }

    fn explorer_command(&mut self, command: Command) {
        match command {
            Command::Move(Motion::Up) => self.explorer_move(-1),
            Command::Move(Motion::Down) => self.explorer_move(1),
            Command::Move(Motion::PageUp) => self.explorer_move(-10),
            Command::Move(Motion::PageDown) => self.explorer_move(10),
            Command::InsertNewline | Command::Move(Motion::Right) => self.explorer_activate(),
            Command::Backspace | Command::Move(Motion::Left) => self.explorer_ascend(),
            Command::Cancel | Command::Explorer | Command::Quit | Command::CloseBuffer => {
                self.explorer = None
            }
            Command::InsertChar(c) => {
                if let Some(explorer) = self.explorer.as_mut() {
                    explorer.jump_to_prefix(c);
                }
            }
            _ => {}
        }
    }

    fn explorer_move(&mut self, delta: isize) {
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.move_selection(delta);
        }
    }

    fn explorer_activate(&mut self) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        let Some(entry) = explorer.selected_entry() else {
            return;
        };
        if entry.name == ".." {
            self.explorer_ascend();
            return;
        }
        let target = explorer.dir.join(&entry.name);
        if entry.is_dir {
            match Explorer::open(&target) {
                Ok(next) => self.explorer = Some(next),
                Err(e) => self.set_error(format!("explorer: {e}")),
            }
        } else {
            self.explorer = None;
            self.open_file(&target);
        }
    }

    fn open_search(&mut self, replacing: bool) {
        if let Some(search) = self.search.as_mut() {
            search.replacing |= replacing;
            search.field = if replacing {
                Field::Replacement
            } else {
                Field::Query
            };
            return;
        }
        let seed = self
            .buffer
            .selected_text()
            .filter(|text| !text.is_empty() && !text.contains('\n'))
            .or_else(|| self.search_memo.as_ref().map(|(query, _)| query.clone()))
            .unwrap_or_default();
        let case_sensitive = self.search_memo.as_ref().is_some_and(|(_, case)| *case);
        let origin = self
            .buffer
            .selection()
            .map(|sel| sel.start)
            .unwrap_or(self.buffer.cursor.pos);
        self.completion = None;
        self.hover = None;
        let mut search = Search::new(&seed, case_sensitive, replacing, origin);
        search.refresh(&self.buffer);
        if replacing && !seed.is_empty() {
            search.field = Field::Replacement;
        }
        self.search = Some(search);
        self.select_current_match();
    }

    fn close_search(&mut self) {
        if let Some(search) = self.search.take() {
            let query = search.query.value();
            if !query.is_empty() {
                self.search_memo = Some((query, search.case_sensitive));
            }
        }
    }

    fn select_current_match(&mut self) {
        let Some(hit) = self.search.as_ref().and_then(Search::current_match) else {
            return;
        };
        self.select_match(hit);
    }

    fn select_match(&mut self, hit: Match) {
        self.buffer.set_cursor_pos(hit.start_position(), false);
        self.buffer.set_cursor_pos(hit.end_position(), true);
        self.sync_viewport();
    }

    fn search_command(&mut self, command: Command) {
        let mut requery = false;
        let mut step = 0isize;
        let mut close = false;
        let mut paste = false;
        let mut replace_current = false;
        let mut replace_all = false;
        let mut forward = None;
        {
            let Some(search) = self.search.as_mut() else {
                return;
            };
            let editing_query = search.field == Field::Query;
            match command {
                Command::InsertChar(c) => {
                    search.active_prompt().insert(c);
                    requery = editing_query;
                }
                Command::Backspace => {
                    search.active_prompt().backspace();
                    requery = editing_query;
                }
                Command::Delete => {
                    search.active_prompt().delete();
                    requery = editing_query;
                }
                Command::Paste => paste = true,
                Command::SelectAll => {
                    search.active_prompt().clear();
                    requery = editing_query;
                }
                Command::Move(Motion::Up) | Command::Select(Motion::Up) => step = -1,
                Command::Move(Motion::Down) | Command::Select(Motion::Down) => step = 1,
                Command::Move(motion) | Command::Select(motion) => {
                    search.active_prompt().move_cursor(motion);
                }
                Command::InsertNewline => {
                    if editing_query {
                        step = 1;
                    } else {
                        replace_current = true;
                    }
                }
                Command::InsertTab | Command::Outdent => {
                    search.replacing = true;
                    search.field = if editing_query {
                        Field::Replacement
                    } else {
                        Field::Query
                    };
                }
                Command::Find => search.field = Field::Query,
                Command::Replace => {
                    search.replacing = true;
                    search.field = Field::Replacement;
                }
                Command::FindNext => step = 1,
                Command::FindPrevious => step = -1,
                Command::ReplaceAll => replace_all = true,
                Command::ToggleSearchCase => {
                    search.case_sensitive = !search.case_sensitive;
                    requery = true;
                }
                Command::Cancel | Command::Quit | Command::CloseBuffer => close = true,
                Command::Save
                | Command::Help
                | Command::LspMenu
                | Command::Explorer
                | Command::FindFile => {
                    close = true;
                    forward = Some(command);
                }
                _ => {}
            }
        }

        if paste {
            let text = self.clipboard.clone();
            if let Some(search) = self.search.as_mut() {
                let editing_query = search.field == Field::Query;
                search.active_prompt().insert_text(&text);
                requery = editing_query;
            }
        }
        if requery {
            if let Some(search) = self.search.as_mut() {
                search.refresh(&self.buffer);
            }
            self.select_current_match();
        }
        if step != 0 {
            if let Some(search) = self.search.as_mut() {
                search.step(step);
            }
            self.select_current_match();
            self.report_search_position();
        }
        if replace_current {
            self.replace_current_match();
        }
        if replace_all {
            self.replace_every_match();
        }
        if close {
            self.close_search();
        }
        if let Some(command) = forward {
            self.execute(command);
        }
    }

    fn report_search_position(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        if search.matches.is_empty() && !search.query.is_empty() {
            let query = search.query.value();
            self.set_info(format!("No results for {query}"));
        }
    }

    fn replace_current_match(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let Some(hit) = search.current_match() else {
            self.set_info("No results");
            return;
        };
        let replacement = search.replacement.value();
        self.buffer
            .replace_ranges(&[(hit.start_position(), hit.end_position())], &replacement);
        let resume = self.buffer.cursor.pos;
        if let Some(search) = self.search.as_mut() {
            search.origin = resume;
            search.refresh(&self.buffer);
        }
        self.select_current_match();
        self.sync_viewport();
    }

    fn replace_every_match(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        if search.matches.is_empty() {
            self.set_info("No results");
            return;
        }
        let ranges = search.ranges();
        let replacement = search.replacement.value();
        let count = self.buffer.replace_ranges(&ranges, &replacement);
        let resume = self.buffer.cursor.pos;
        if let Some(search) = self.search.as_mut() {
            search.origin = resume;
            search.refresh(&self.buffer);
        }
        self.sync_viewport();
        let plural = if count == 1 {
            "occurrence"
        } else {
            "occurrences"
        };
        self.set_info(format!("Replaced {count} {plural}"));
    }

    fn repeat_search(&mut self, delta: isize) {
        let Some((query, case_sensitive)) = self.search_memo.clone() else {
            self.open_search(false);
            return;
        };
        let matches = search::find_matches(&self.buffer, &query, case_sensitive);
        if matches.is_empty() {
            self.set_info(format!("No results for {query}"));
            return;
        }
        let selection = self.buffer.selection();
        let index = if delta >= 0 {
            let from = selection
                .map(|sel| sel.end)
                .unwrap_or(self.buffer.cursor.pos);
            matches
                .iter()
                .position(|hit| hit.start_position() >= from)
                .unwrap_or(0)
        } else {
            let from = selection
                .map(|sel| sel.start)
                .unwrap_or(self.buffer.cursor.pos);
            matches
                .iter()
                .rposition(|hit| hit.start_position() < from)
                .unwrap_or(matches.len() - 1)
        };
        self.select_match(matches[index]);
        self.set_info(format!("{} of {}", index + 1, matches.len()));
    }

    fn toggle_remembered_case(&mut self) {
        let Some((_, case_sensitive)) = self.search_memo.as_mut() else {
            self.set_info("No search to configure");
            return;
        };
        *case_sensitive = !*case_sensitive;
        let state = if *case_sensitive {
            "case sensitive"
        } else {
            "case insensitive"
        };
        self.set_info(format!("Search is {state}"));
    }

    pub fn finder_visible_rows(&self) -> usize {
        self.viewport.text_height().saturating_sub(4).clamp(1, 14)
    }

    fn open_finder(&mut self) {
        let start = self
            .buffer
            .path()
            .and_then(Path::parent)
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        match Finder::open(&start) {
            Ok(finder) => {
                self.completion = None;
                self.hover = None;
                self.menu = None;
                self.explorer = None;
                self.close_search();
                self.finder = Some(finder);
            }
            Err(e) => self.set_error(format!("finder: {e}")),
        }
    }

    fn finder_command(&mut self, command: Command) {
        let rows = self.finder_visible_rows() as isize;
        let mut open_selected = false;
        let mut paste = false;
        let mut close = false;
        {
            let Some(finder) = self.finder.as_mut() else {
                return;
            };
            match command {
                Command::InsertChar(c) => {
                    finder.query.insert(c);
                    finder.refresh();
                }
                Command::Backspace => {
                    if finder.query.backspace() {
                        finder.refresh();
                    }
                }
                Command::Delete => {
                    if finder.query.delete() {
                        finder.refresh();
                    }
                }
                Command::Paste => paste = true,
                Command::SelectAll => {
                    finder.query.clear();
                    finder.refresh();
                }
                Command::Move(Motion::Up) | Command::Select(Motion::Up) => {
                    finder.move_selection(-1)
                }
                Command::Move(Motion::Down) | Command::Select(Motion::Down) => {
                    finder.move_selection(1)
                }
                Command::Move(Motion::PageUp) => finder.move_selection(-rows),
                Command::Move(Motion::PageDown) => finder.move_selection(rows),
                Command::Move(motion) | Command::Select(motion) => {
                    finder.query.move_cursor(motion);
                }
                Command::InsertNewline => open_selected = true,
                Command::Cancel | Command::Quit | Command::CloseBuffer | Command::FindFile => {
                    close = true
                }
                _ => {}
            }
        }
        if paste {
            let text = self.clipboard.clone();
            if let Some(finder) = self.finder.as_mut() {
                finder.query.insert_text(&text);
                finder.refresh();
            }
        }
        if open_selected {
            let target = self.finder.as_ref().and_then(Finder::selected_path);
            match target {
                Some(path) => {
                    self.finder = None;
                    self.open_file(&path);
                }
                None => self.set_info("No matching files"),
            }
        }
        if close {
            self.finder = None;
        }
    }

    fn explorer_ascend(&mut self) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        let Some(parent) = explorer.dir.parent() else {
            return;
        };
        let came_from = explorer
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        match Explorer::open(parent) {
            Ok(mut next) => {
                if let Some(name) = came_from {
                    next.select_name(&name);
                }
                self.explorer = Some(next);
            }
            Err(e) => self.set_error(format!("explorer: {e}")),
        }
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn closing_bracket(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => open,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Motion, Position};

    fn editor_with(text: &str) -> Editor {
        let mut buffer = Buffer::scratch();
        buffer.insert_text(text);
        buffer.commit_undo_group();
        buffer.cursor.pos = Position::new(0, 0);
        Editor::new(buffer, Settings::default())
    }

    #[test]
    fn copy_and_paste_selection() {
        let mut ed = editor_with("hello");
        ed.execute(Command::Select(Motion::WordRight));
        ed.execute(Command::Copy);
        ed.execute(Command::Move(Motion::LineEnd));
        ed.execute(Command::Paste);
        assert_eq!(ed.buffer.text(), "hellohello");
    }

    #[test]
    fn cut_without_selection_cuts_line() {
        let mut ed = editor_with("one\ntwo");
        ed.execute(Command::Cut);
        assert_eq!(ed.buffer.text(), "two");
        ed.execute(Command::Paste);
        assert_eq!(ed.buffer.text(), "one\ntwo");
    }

    #[test]
    fn quit_warns_when_dirty_then_quits() {
        let mut ed = editor_with("x");
        ed.execute(Command::InsertChar('y'));
        ed.execute(Command::Quit);
        assert!(!ed.should_quit());
        assert!(matches!(
            ed.status.as_ref().map(|s| s.kind),
            Some(MessageKind::Error)
        ));
        ed.execute(Command::Quit);
        assert!(ed.should_quit());
    }

    #[test]
    fn quit_confirmation_resets_on_other_commands() {
        let mut ed = editor_with("x");
        ed.execute(Command::InsertChar('y'));
        ed.execute(Command::Quit);
        ed.execute(Command::Move(Motion::Left));
        ed.execute(Command::Quit);
        assert!(!ed.should_quit());
    }

    #[test]
    fn quit_immediately_when_clean() {
        let mut ed = editor_with("");
        ed.execute(Command::Quit);
        assert!(ed.should_quit());
    }

    #[test]
    fn open_bracket_inserts_pair_and_close_skips_over() {
        let mut ed = editor_with("");
        ed.execute(Command::InsertChar('('));
        assert_eq!(ed.buffer.text(), "()");
        assert_eq!(ed.buffer.cursor.pos, Position::new(0, 1));
        ed.execute(Command::InsertChar(')'));
        assert_eq!(ed.buffer.text(), "()");
        assert_eq!(ed.buffer.cursor.pos, Position::new(0, 2));
    }

    #[test]
    fn backspace_inside_empty_pair_deletes_both() {
        let mut ed = editor_with("");
        ed.execute(Command::InsertChar('{'));
        ed.execute(Command::Backspace);
        assert_eq!(ed.buffer.text(), "");
    }

    #[test]
    fn typing_pair_is_undoable_with_following_text() {
        let mut ed = editor_with("");
        ed.execute(Command::InsertChar('f'));
        ed.execute(Command::InsertChar('('));
        ed.execute(Command::InsertChar('x'));
        assert_eq!(ed.buffer.text(), "f(x)");
        ed.execute(Command::Undo);
        assert_eq!(ed.buffer.text(), "f()");
        ed.execute(Command::Undo);
        assert_eq!(ed.buffer.text(), "");
    }

    #[test]
    fn open_bracket_surrounds_selection() {
        let mut ed = editor_with("hello");
        ed.execute(Command::SelectAll);
        ed.execute(Command::InsertChar('['));
        assert_eq!(ed.buffer.text(), "[hello]");
        assert_eq!(ed.buffer.selected_text().as_deref(), Some("hello"));
        ed.execute(Command::Undo);
        assert_eq!(ed.buffer.text(), "hello");
    }

    #[test]
    fn quote_pairs_at_line_end_but_not_before_word() {
        let mut ed = editor_with("");
        ed.execute(Command::InsertChar('"'));
        assert_eq!(ed.buffer.text(), "\"\"");
        assert_eq!(ed.buffer.cursor.pos, Position::new(0, 1));
        ed.execute(Command::InsertChar('"'));
        assert_eq!(ed.buffer.text(), "\"\"");

        let mut ed = editor_with("word");
        ed.execute(Command::InsertChar('"'));
        assert_eq!(ed.buffer.text(), "\"word");
    }

    #[test]
    fn auto_pairs_can_be_disabled() {
        let mut buffer = Buffer::scratch();
        buffer.insert_text("");
        let settings = Settings {
            auto_pairs: false,
            ..Settings::default()
        };
        let mut ed = Editor::new(buffer, settings);
        ed.execute(Command::InsertChar('('));
        assert_eq!(ed.buffer.text(), "(");
    }

    #[test]
    fn close_bracket_inserted_when_not_matching() {
        let mut ed = editor_with("");
        ed.execute(Command::InsertChar(')'));
        assert_eq!(ed.buffer.text(), ")");
    }

    fn editor_with_screen(text: &str, width: u16, height: u16) -> Editor {
        let mut ed = editor_with(text);
        ed.viewport.resize(width, height);
        ed
    }

    #[test]
    fn click_places_cursor_at_screen_position() {
        let mut ed = editor_with_screen("hello\nworld", 40, 10);
        let gutter = ed.gutter_width();
        ed.mouse_down(gutter + 2, 1, false);
        assert_eq!(ed.buffer.cursor.pos, Position::new(1, 2));
        assert!(ed.buffer.selection().is_none());
    }

    #[test]
    fn click_in_gutter_goes_to_line_start() {
        let mut ed = editor_with_screen("hello\nworld", 40, 10);
        ed.mouse_down(0, 1, false);
        assert_eq!(ed.buffer.cursor.pos, Position::new(1, 0));
    }

    #[test]
    fn click_past_line_end_clamps_to_line_end() {
        let mut ed = editor_with_screen("hi\nlonger line", 40, 10);
        ed.mouse_down(30, 0, false);
        assert_eq!(ed.buffer.cursor.pos, Position::new(0, 2));
    }

    #[test]
    fn click_below_text_goes_to_last_line() {
        let mut ed = editor_with_screen("one\ntwo", 40, 10);
        ed.mouse_down(ed.gutter_width(), 7, false);
        assert_eq!(ed.buffer.cursor.pos.line, 1);
    }

    #[test]
    fn click_on_statusline_is_ignored() {
        let mut ed = editor_with_screen("one\ntwo", 40, 10);
        ed.mouse_down(ed.gutter_width() + 1, 1, false);
        let before = ed.buffer.cursor.pos;
        ed.mouse_down(0, 9, false);
        assert_eq!(ed.buffer.cursor.pos, before);
    }

    #[test]
    fn drag_extends_selection_from_press_point() {
        let mut ed = editor_with_screen("hello\nworld", 40, 10);
        let gutter = ed.gutter_width();
        ed.mouse_down(gutter, 0, false);
        ed.mouse_drag(gutter + 3, 1);
        assert_eq!(ed.buffer.selected_text().as_deref(), Some("hello\nwor"));
    }

    #[test]
    fn shift_click_extends_selection() {
        let mut ed = editor_with_screen("hello", 40, 10);
        let gutter = ed.gutter_width();
        ed.mouse_down(gutter + 1, 0, false);
        ed.mouse_down(gutter + 4, 0, true);
        assert_eq!(ed.buffer.selected_text().as_deref(), Some("ell"));
    }

    #[test]
    fn wheel_scrolls_viewport_without_moving_cursor() {
        let text = (0..50).map(|i| format!("line {i}\n")).collect::<String>();
        let mut ed = editor_with_screen(&text, 40, 10);
        ed.scroll_viewport(3);
        assert_eq!(ed.viewport.top_line, 3);
        assert_eq!(ed.buffer.cursor.pos, Position::new(0, 0));
        ed.scroll_viewport(1000);
        assert_eq!(ed.viewport.top_line, 50);
        ed.scroll_viewport(-1000);
        assert_eq!(ed.viewport.top_line, 0);
    }

    fn type_text(ed: &mut Editor, text: &str) {
        for c in text.chars() {
            ed.execute(Command::InsertChar(c));
        }
    }

    #[test]
    fn typing_opens_completion_and_tab_accepts() {
        let mut ed = editor_with("hello helper\n");
        ed.execute(Command::Move(Motion::BufferEnd));
        type_text(&mut ed, "hel");
        let items = ed.completion.as_ref().map(|c| c.items.clone()).unwrap();
        assert_eq!(items, vec!["hello".to_string(), "helper".to_string()]);
        ed.execute(Command::Move(Motion::Down));
        ed.execute(Command::InsertTab);
        assert_eq!(ed.buffer.text(), "hello helper\nhelper");
        assert!(ed.completion.is_none());
        ed.execute(Command::Undo);
        assert_eq!(ed.buffer.text(), "hello helper\n");
    }

    #[test]
    fn enter_accepts_completion_without_newline() {
        let mut ed = editor_with("window\n");
        ed.execute(Command::Move(Motion::BufferEnd));
        type_text(&mut ed, "wi");
        assert!(ed.completion.is_some());
        ed.execute(Command::InsertNewline);
        assert_eq!(ed.buffer.text(), "window\nwindow");
    }

    #[test]
    fn escape_dismisses_completion_and_keeps_text() {
        let mut ed = editor_with("hello\n");
        ed.execute(Command::Move(Motion::BufferEnd));
        type_text(&mut ed, "he");
        assert!(ed.completion.is_some());
        ed.execute(Command::Cancel);
        assert!(ed.completion.is_none());
        assert_eq!(ed.buffer.text(), "hello\nhe");
        ed.execute(Command::InsertNewline);
        assert_eq!(ed.buffer.text(), "hello\nhe\n");
    }

    #[test]
    fn cursor_movement_dismisses_completion() {
        let mut ed = editor_with("hello\n");
        ed.execute(Command::Move(Motion::BufferEnd));
        type_text(&mut ed, "he");
        ed.execute(Command::Move(Motion::Left));
        assert!(ed.completion.is_none());
    }

    #[test]
    fn single_char_prefix_does_not_auto_open() {
        let mut ed = editor_with("hello\n");
        ed.execute(Command::Move(Motion::BufferEnd));
        type_text(&mut ed, "h");
        assert!(ed.completion.is_none());
    }

    #[test]
    fn manual_trigger_works_with_empty_prefix() {
        let mut ed = editor_with("alpha beta\n");
        ed.execute(Command::Move(Motion::BufferEnd));
        ed.execute(Command::TriggerCompletion);
        let items = ed.completion.as_ref().map(|c| c.items.clone()).unwrap();
        assert_eq!(items, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn typing_and_backspace_refilter_completion() {
        let mut ed = editor_with("helm help hero\n");
        ed.execute(Command::Move(Motion::BufferEnd));
        type_text(&mut ed, "he");
        let items = ed.completion.as_ref().map(|c| c.items.clone()).unwrap();
        assert_eq!(items.len(), 3);
        type_text(&mut ed, "l");
        let items = ed.completion.as_ref().map(|c| c.items.clone()).unwrap();
        assert_eq!(items, vec!["helm".to_string(), "help".to_string()]);
        ed.execute(Command::Backspace);
        let items = ed.completion.as_ref().map(|c| c.items.clone()).unwrap();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn hover_command_queues_lsp_request() {
        let mut ed = editor_with("hello");
        ed.execute(Command::Move(Motion::Right));
        ed.execute(Command::Hover);
        assert_eq!(ed.take_hover_request(), Some(Position::new(0, 1)));
        assert_eq!(ed.take_hover_request(), None);
    }

    #[test]
    fn show_hover_strips_code_fences_and_blank_runs() {
        let mut ed = editor_with("x");
        ed.show_hover(Position::new(0, 0), "```rust\nfn x()\n```\n\n\nDocs\n");
        let hover = ed.hover.as_ref().unwrap();
        assert_eq!(hover.lines, vec!["fn x()", "", "Docs"]);
        ed.show_hover(Position::new(0, 0), "```\n```\n \n");
        assert_eq!(ed.hover.as_ref().unwrap().lines, vec!["fn x()", "", "Docs"]);
    }

    #[test]
    fn dot_queues_lsp_completion_request() {
        let mut ed = editor_with("");
        ed.execute(Command::InsertChar('a'));
        ed.take_completion_request();
        ed.execute(Command::InsertChar('.'));
        assert_eq!(ed.take_completion_request(), Some(2));
        assert!(ed.completion.is_none());
    }

    #[test]
    fn lsp_completions_filter_by_current_prefix() {
        let mut ed = editor_with("");
        type_text(&mut ed, "mo");
        assert!(ed.completion.is_none());
        ed.apply_lsp_completions(0, 0, vec!["mock_a".into(), "other".into(), "mo".into()]);
        let items = ed.completion.as_ref().map(|c| c.items.clone()).unwrap();
        assert_eq!(items, vec!["mock_a".to_string()]);
    }

    #[test]
    fn lsp_completions_keep_server_order_and_drop_uninsertable() {
        let mut ed = editor_with("zzz.");
        ed.execute(Command::Move(Motion::LineEnd));
        ed.apply_lsp_completions(
            0,
            4,
            vec![
                "length".into(),
                "[Symbol.iterator]".into(),
                "at".into(),
                "length".into(),
            ],
        );
        let items = ed.completion.as_ref().map(|c| c.items.clone()).unwrap();
        assert_eq!(items, vec!["length".to_string(), "at".to_string()]);
    }

    #[test]
    fn stale_lsp_completions_are_discarded() {
        let mut ed = editor_with("zzz\n");
        ed.execute(Command::Move(Motion::BufferEnd));
        type_text(&mut ed, "mo");
        assert!(ed.completion.is_none());
        ed.apply_lsp_completions(0, 0, vec!["mock_a".into()]);
        assert!(ed.completion.is_none());
        ed.apply_lsp_completions(1, 1, vec!["mock_a".into()]);
        assert!(ed.completion.is_none());
        ed.apply_lsp_completions(1, 0, vec!["mock_a".into()]);
        assert!(ed.completion.is_some());
    }

    #[test]
    fn empty_lsp_result_keeps_word_completions() {
        let mut ed = editor_with("hello\n");
        ed.execute(Command::Move(Motion::BufferEnd));
        type_text(&mut ed, "he");
        assert!(ed.completion.is_some());
        ed.apply_lsp_completions(1, 0, vec![]);
        let items = ed.completion.as_ref().map(|c| c.items.clone()).unwrap();
        assert_eq!(items, vec!["hello".to_string()]);
    }

    #[test]
    fn lsp_menu_requests_and_closes_on_escape() {
        let mut ed = editor_with("x");
        ed.execute(Command::LspMenu);
        assert!(ed.take_menu_request());
        assert!(!ed.take_menu_request());
        ed.menu = Some(Menu::new("Language Servers", vec!["Rust".to_string()], ""));
        ed.execute(Command::InsertChar('a'));
        assert!(ed.menu.is_some());
        assert_eq!(ed.buffer.text(), "x");
        ed.execute(Command::Cancel);
        assert!(ed.menu.is_none());
    }

    #[test]
    fn help_menu_scrolls_and_closes() {
        let mut ed = editor_with("x");
        ed.viewport.resize(80, 10);
        ed.execute(Command::Help);
        assert!(ed.take_help_request());
        assert!(!ed.take_help_request());

        let lines: Vec<String> = (0..40).map(|i| format!("row {i}")).collect();
        ed.menu = Some(Menu::new("Keybindings", lines, "footer"));
        let visible = ed.menu_visible_rows();
        assert_eq!(visible, 6);

        ed.execute(Command::Move(Motion::Down));
        assert_eq!(ed.menu.as_ref().unwrap().scroll, 1);
        ed.execute(Command::Move(Motion::PageDown));
        assert_eq!(ed.menu.as_ref().unwrap().scroll, 7);
        ed.execute(Command::Move(Motion::Up));
        assert_eq!(ed.menu.as_ref().unwrap().scroll, 6);
        ed.execute(Command::Move(Motion::BufferEnd));
        assert_eq!(ed.menu.as_ref().unwrap().scroll, 40 - visible);
        ed.execute(Command::Move(Motion::BufferStart));
        assert_eq!(ed.menu.as_ref().unwrap().scroll, 0);

        ed.execute(Command::InsertChar('q'));
        assert!(ed.menu.is_some());
        assert_eq!(ed.buffer.text(), "x");
        ed.execute(Command::Help);
        assert!(ed.menu.is_none());
        ed.menu = Some(Menu::new("Keybindings", vec!["row".to_string()], "f"));
        ed.execute(Command::Cancel);
        assert!(ed.menu.is_none());
    }

    #[test]
    fn short_menu_does_not_scroll() {
        let mut ed = editor_with("x");
        ed.viewport.resize(80, 24);
        ed.menu = Some(Menu::new("M", vec!["a".to_string(), "b".to_string()], "f"));
        ed.execute(Command::Move(Motion::Down));
        assert_eq!(ed.menu.as_ref().unwrap().scroll, 0);
    }

    fn temp_file(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kanso-editor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn open_file_switches_and_cycles_buffers() {
        let a = temp_file("a.txt", "aaa");
        let b = temp_file("b.txt", "bbb");
        let mut ed = Editor::new(Buffer::from_path(&a).unwrap(), Settings::default());
        ed.open_file(&b);
        assert_eq!(ed.buffer.file_name(), "b.txt");
        assert_eq!(ed.buffer_count(), 2);
        ed.execute(Command::NextBuffer);
        assert_eq!(ed.buffer.file_name(), "a.txt");
        ed.execute(Command::PrevBuffer);
        assert_eq!(ed.buffer.file_name(), "b.txt");
        ed.open_file(&a);
        assert_eq!(ed.buffer.file_name(), "a.txt");
        assert_eq!(ed.buffer_count(), 2);
    }

    #[test]
    fn close_buffer_activates_next_and_last_close_quits() {
        let a = temp_file("c.txt", "ccc");
        let b = temp_file("d.txt", "ddd");
        let mut ed = Editor::new(Buffer::from_path(&a).unwrap(), Settings::default());
        ed.open_file(&b);
        ed.execute(Command::CloseBuffer);
        assert!(!ed.should_quit());
        assert_eq!(ed.buffer.file_name(), "c.txt");
        ed.execute(Command::CloseBuffer);
        assert!(ed.should_quit());
    }

    #[test]
    fn close_dirty_buffer_warns_first() {
        let a = temp_file("e.txt", "eee");
        let mut ed = Editor::new(Buffer::from_path(&a).unwrap(), Settings::default());
        ed.execute(Command::InsertChar('x'));
        ed.execute(Command::CloseBuffer);
        assert!(!ed.should_quit());
        ed.execute(Command::CloseBuffer);
        assert!(ed.should_quit());
    }

    #[test]
    fn quit_warns_about_dirty_background_buffer() {
        let a = temp_file("f.txt", "fff");
        let b = temp_file("g.txt", "ggg");
        let mut ed = Editor::new(Buffer::from_path(&a).unwrap(), Settings::default());
        ed.execute(Command::InsertChar('x'));
        ed.open_file(&b);
        ed.execute(Command::Quit);
        assert!(!ed.should_quit());
        ed.execute(Command::Quit);
        assert!(ed.should_quit());
    }

    #[test]
    fn explorer_opens_at_file_dir_and_opens_files() {
        let a = temp_file("h.txt", "hhh");
        let i = temp_file("i.txt", "iii");
        let mut ed = Editor::new(Buffer::from_path(&a).unwrap(), Settings::default());
        ed.execute(Command::Explorer);
        let explorer = ed.explorer.as_ref().unwrap();
        assert_eq!(explorer.dir, a.parent().unwrap().canonicalize().unwrap());
        assert_eq!(explorer.selected_entry().unwrap().name, "h.txt");
        ed.execute(Command::InsertChar('i'));
        assert_eq!(
            ed.explorer.as_ref().unwrap().selected_entry().unwrap().name,
            "i.txt"
        );
        ed.execute(Command::InsertNewline);
        assert!(ed.explorer.is_none());
        assert_eq!(ed.buffer.file_name(), "i.txt");
        assert_eq!(ed.buffer.text(), std::fs::read_to_string(&i).unwrap());
        assert_eq!(ed.buffer_count(), 2);
    }

    #[test]
    fn explorer_ascends_and_escape_closes() {
        let a = temp_file("j.txt", "jjj");
        let mut ed = Editor::new(Buffer::from_path(&a).unwrap(), Settings::default());
        ed.execute(Command::Explorer);
        let before = ed.explorer.as_ref().unwrap().dir.clone();
        ed.execute(Command::Backspace);
        let explorer = ed.explorer.as_ref().unwrap();
        assert_eq!(explorer.dir, before.parent().unwrap());
        assert_eq!(
            explorer.selected_entry().unwrap().name,
            before.file_name().unwrap().to_string_lossy()
        );
        ed.execute(Command::Cancel);
        assert!(ed.explorer.is_none());
    }

    fn rust_editor(name: &str, text: &str) -> Editor {
        let path = temp_file(&format!("{name}.rs"), text);
        Editor::new(Buffer::from_path(&path).unwrap(), Settings::default())
    }

    #[test]
    fn detects_indent_style_from_content() {
        let ed = rust_editor("detect_spaces", "fn a() {\n  b();\n}\n");
        assert_eq!(ed.buffer.indent().width, 2);
        assert!(ed.buffer.indent().use_spaces);

        let ed = rust_editor("detect_tabs", "fn a() {\n\tb();\n}\n");
        assert!(!ed.buffer.indent().use_spaces);
    }

    #[test]
    fn newline_keeps_current_indentation() {
        let mut ed = rust_editor("keep_indent", "fn a() {\n    let x = 1;\n}\n");
        ed.buffer.cursor.pos = Position::new(1, 14);
        ed.execute(Command::InsertNewline);
        assert_eq!(ed.buffer.text(), "fn a() {\n    let x = 1;\n    \n}\n");
        assert_eq!(ed.buffer.cursor.pos, Position::new(2, 4));
    }

    #[test]
    fn newline_after_brace_adds_one_level() {
        let mut ed = rust_editor("after_brace", "fn a() {\n    if x {\n    }\n}\n");
        ed.buffer.cursor.pos = Position::new(1, 11);
        ed.execute(Command::InsertNewline);
        assert_eq!(
            ed.buffer.text(),
            "fn a() {\n    if x {\n        \n    }\n}\n"
        );
        assert_eq!(ed.buffer.cursor.pos, Position::new(2, 8));
    }

    #[test]
    fn newline_between_braces_splits_them() {
        let mut ed = rust_editor("between_braces", "fn a() {\n    if x {}\n}\n");
        ed.buffer.cursor.pos = Position::new(1, 10);
        ed.execute(Command::InsertNewline);
        assert_eq!(
            ed.buffer.text(),
            "fn a() {\n    if x {\n        \n    }\n}\n"
        );
        assert_eq!(ed.buffer.cursor.pos, Position::new(2, 8));
        ed.execute(Command::Undo);
        assert_eq!(ed.buffer.text(), "fn a() {\n    if x {}\n}\n");
    }

    #[test]
    fn typing_brace_pair_then_newline_expands_block() {
        let mut ed = rust_editor("brace_pair", "fn a() {\n    x\n}\n");
        ed.buffer.cursor.pos = Position::new(1, 5);
        ed.execute(Command::InsertChar('{'));
        assert_eq!(ed.buffer.text(), "fn a() {\n    x{}\n}\n");
        ed.execute(Command::InsertNewline);
        assert_eq!(ed.buffer.text(), "fn a() {\n    x{\n        \n    }\n}\n");
    }

    #[test]
    fn python_colon_indents_next_line() {
        let path = temp_file("indent.py", "def a():\n    pass\n");
        let mut ed = Editor::new(Buffer::from_path(&path).unwrap(), Settings::default());
        ed.buffer.cursor.pos = Position::new(0, 8);
        ed.execute(Command::InsertNewline);
        assert_eq!(ed.buffer.text(), "def a():\n    \n    pass\n");
    }

    #[test]
    fn tab_indents_multiline_selection_and_shift_tab_outdents() {
        let mut ed = rust_editor("block_indent", "fn a() {\n    x();\n}\nb();\nc();\n");
        ed.buffer.set_cursor_pos(Position::new(3, 0), false);
        ed.execute(Command::Select(Motion::Down));
        ed.execute(Command::InsertTab);
        assert_eq!(
            ed.buffer.text(),
            "fn a() {\n    x();\n}\n    b();\n    c();\n"
        );
        ed.execute(Command::Outdent);
        assert_eq!(ed.buffer.text(), "fn a() {\n    x();\n}\nb();\nc();\n");
    }

    #[test]
    fn shift_tab_outdents_current_line_without_selection() {
        let mut ed = rust_editor(
            "outdent_line",
            "fn a() {\n    if x {\n        deep();\n    }\n}\n",
        );
        ed.buffer.cursor.pos = Position::new(2, 10);
        ed.execute(Command::Outdent);
        assert_eq!(
            ed.buffer.text(),
            "fn a() {\n    if x {\n    deep();\n    }\n}\n"
        );
        assert_eq!(ed.buffer.cursor.pos, Position::new(2, 6));
    }

    #[test]
    fn backspace_removes_whole_indent_level() {
        let mut ed = rust_editor(
            "backspace_indent",
            "fn a() {\n    if x {\n        y();\n    }\n}\n",
        );
        ed.buffer.cursor.pos = Position::new(2, 8);
        ed.execute(Command::Backspace);
        assert_eq!(
            ed.buffer.text(),
            "fn a() {\n    if x {\n    y();\n    }\n}\n"
        );
        ed.execute(Command::Backspace);
        assert_eq!(ed.buffer.text(), "fn a() {\n    if x {\ny();\n    }\n}\n");
    }

    #[test]
    fn backspace_in_text_stays_single_char() {
        let mut ed = rust_editor("backspace_text", "fn a() {\n    abc\n}\n");
        ed.buffer.cursor.pos = Position::new(1, 7);
        ed.execute(Command::Backspace);
        assert_eq!(ed.buffer.text(), "fn a() {\n    ab\n}\n");
    }

    #[test]
    fn closing_brace_outdents_its_line() {
        let mut ed = rust_editor(
            "close_outdent",
            "fn a() {\n    if x {\n        y();\n        \n}\n",
        );
        ed.buffer.cursor.pos = Position::new(3, 8);
        ed.execute(Command::InsertChar('}'));
        assert_eq!(
            ed.buffer.text(),
            "fn a() {\n    if x {\n        y();\n    }\n}\n"
        );
    }

    #[test]
    fn find_highlights_matches_and_enter_walks_them() {
        let mut ed = editor_with("foo\nbar foo\nfoo");
        ed.execute(Command::Find);
        type_text(&mut ed, "foo");
        let search = ed.search.as_ref().unwrap();
        assert_eq!(search.matches.len(), 3);
        assert_eq!(search.counter(), "1/3");
        assert_eq!(ed.buffer.selected_text().as_deref(), Some("foo"));
        assert_eq!(ed.buffer.cursor.pos, Position::new(0, 3));

        ed.execute(Command::InsertNewline);
        assert_eq!(ed.buffer.cursor.pos, Position::new(1, 7));
        assert_eq!(ed.search.as_ref().unwrap().counter(), "2/3");
        ed.execute(Command::InsertNewline);
        ed.execute(Command::InsertNewline);
        assert_eq!(ed.search.as_ref().unwrap().counter(), "1/3");
        assert_eq!(ed.buffer.text(), "foo\nbar foo\nfoo");
    }

    #[test]
    fn find_seeds_from_a_single_line_selection() {
        let mut ed = editor_with("alpha beta\nalpha");
        ed.execute(Command::Select(Motion::WordRight));
        ed.execute(Command::Find);
        let search = ed.search.as_ref().unwrap();
        assert_eq!(search.query.value(), "alpha");
        assert_eq!(search.matches.len(), 2);
    }

    #[test]
    fn escape_closes_search_and_find_next_repeats_it() {
        let mut ed = editor_with("foo\nbar foo\nfoo");
        ed.execute(Command::Find);
        type_text(&mut ed, "foo");
        ed.execute(Command::Cancel);
        assert!(ed.search.is_none());

        ed.execute(Command::FindNext);
        assert_eq!(ed.buffer.cursor.pos, Position::new(1, 7));
        ed.execute(Command::FindNext);
        assert_eq!(ed.buffer.cursor.pos, Position::new(2, 3));
        ed.execute(Command::FindNext);
        assert_eq!(ed.buffer.cursor.pos, Position::new(0, 3));
        ed.execute(Command::FindPrevious);
        assert_eq!(ed.buffer.cursor.pos, Position::new(2, 3));
    }

    #[test]
    fn typing_in_the_search_field_never_edits_the_buffer() {
        let mut ed = editor_with("hello");
        ed.execute(Command::Find);
        type_text(&mut ed, "zzz");
        ed.execute(Command::Backspace);
        ed.execute(Command::InsertTab);
        assert_eq!(ed.buffer.text(), "hello");
        assert_eq!(ed.search.as_ref().unwrap().query.value(), "zz");
    }

    #[test]
    fn toggling_case_sensitivity_narrows_matches() {
        let mut ed = editor_with("Foo foo");
        ed.execute(Command::Find);
        type_text(&mut ed, "foo");
        assert_eq!(ed.search.as_ref().unwrap().matches.len(), 2);
        ed.execute(Command::ToggleSearchCase);
        let search = ed.search.as_ref().unwrap();
        assert_eq!(search.matches.len(), 1);
        assert_eq!(search.matches[0].start, 4);
    }

    #[test]
    fn replace_rewrites_the_current_match_and_advances() {
        let mut ed = editor_with("foo foo");
        ed.execute(Command::Find);
        type_text(&mut ed, "foo");
        ed.execute(Command::Replace);
        type_text(&mut ed, "x");
        ed.execute(Command::InsertNewline);
        assert_eq!(ed.buffer.text(), "x foo");
        assert_eq!(ed.search.as_ref().unwrap().counter(), "1/1");
        ed.execute(Command::InsertNewline);
        assert_eq!(ed.buffer.text(), "x x");
        ed.execute(Command::Cancel);
        ed.execute(Command::Undo);
        assert_eq!(ed.buffer.text(), "x foo");
    }

    #[test]
    fn replace_all_rewrites_every_match_in_one_undo_step() {
        let mut ed = editor_with("foo bar foo\nfoo");
        ed.execute(Command::Replace);
        assert_eq!(ed.search.as_ref().unwrap().field, Field::Query);
        type_text(&mut ed, "foo");
        ed.execute(Command::InsertTab);
        assert_eq!(ed.search.as_ref().unwrap().field, Field::Replacement);
        type_text(&mut ed, "baz");
        ed.execute(Command::ReplaceAll);
        assert_eq!(ed.buffer.text(), "baz bar baz\nbaz");
        ed.execute(Command::Cancel);
        ed.execute(Command::Undo);
        assert_eq!(ed.buffer.text(), "foo bar foo\nfoo");
    }

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kanso-project-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "notes\n").unwrap();
        dir
    }

    #[test]
    fn finder_opens_a_file_by_fuzzy_query() {
        let dir = temp_project("open");
        let start = dir.join("notes.txt");
        let mut ed = Editor::new(Buffer::from_path(&start).unwrap(), Settings::default());
        ed.execute(Command::FindFile);
        assert_eq!(ed.finder.as_ref().unwrap().files.len(), 2);
        type_text(&mut ed, "mn");
        let finder = ed.finder.as_ref().unwrap();
        assert_eq!(finder.hits.len(), 1);
        assert_eq!(finder.hit_path(&finder.hits[0]), "src/main.rs");
        ed.execute(Command::InsertNewline);
        assert!(ed.finder.is_none());
        assert_eq!(ed.buffer.file_name(), "main.rs");
        assert_eq!(ed.buffer_count(), 2);
    }

    #[test]
    fn finder_typing_leaves_the_buffer_untouched_and_escape_closes() {
        let dir = temp_project("escape");
        let start = dir.join("notes.txt");
        let mut ed = Editor::new(Buffer::from_path(&start).unwrap(), Settings::default());
        ed.execute(Command::FindFile);
        type_text(&mut ed, "zzz");
        assert!(ed.finder.as_ref().unwrap().hits.is_empty());
        ed.execute(Command::InsertNewline);
        assert!(ed.finder.is_some());
        assert_eq!(ed.buffer.text(), "notes\n");
        ed.execute(Command::Cancel);
        assert!(ed.finder.is_none());
        assert_eq!(ed.buffer_count(), 1);
    }

    #[test]
    fn gutter_width_grows_with_line_count() {
        let ed = editor_with("a\nb\nc");
        assert_eq!(ed.gutter_width(), 3);
        let many = "x\n".repeat(120);
        let ed = editor_with(&many);
        assert_eq!(ed.gutter_width(), 5);
    }
}
