use std::fs::File;
use std::io::{self, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ropey::Rope;

use crate::editor::history::{EditOp, GroupKind, History, Merge};

use super::char_display_width;
use super::cursor::{Cursor, Motion, Position};
use super::indent::{self, Indent, IndentStyle};
use super::selection::Selection;

const DETECT_LINES: usize = 400;

pub struct Buffer {
    id: u64,
    rope: Rope,
    path: Option<PathBuf>,
    dirty: bool,
    dirty_from: Option<usize>,
    revision: u64,
    indent: IndentStyle,
    pub cursor: Cursor,
    anchor: Option<Position>,
    history: History,
}

impl Buffer {
    pub fn scratch() -> Self {
        Self::from_rope(Rope::new(), None)
    }

    pub fn from_path(path: &Path) -> io::Result<Self> {
        let rope = if path.exists() {
            Rope::from_str(&std::fs::read_to_string(path)?)
        } else {
            Rope::new()
        };
        Ok(Self::from_rope(rope, Some(path.to_path_buf())))
    }

    fn from_rope(rope: Rope, path: Option<PathBuf>) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Buffer {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            rope,
            path,
            dirty: false,
            dirty_from: None,
            revision: 0,
            indent: IndentStyle {
                use_spaces: true,
                width: 4,
            },
            cursor: Cursor::default(),
            anchor: None,
            history: History::default(),
        }
    }

    fn touch(&mut self, line: usize) {
        self.dirty = true;
        self.revision += 1;
        self.dirty_from = Some(self.dirty_from.map_or(line, |l| l.min(line)));
    }

    pub fn take_dirty_from(&mut self) -> Option<usize> {
        self.dirty_from.take()
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn indent(&self) -> IndentStyle {
        self.indent
    }

    pub fn set_indent(&mut self, style: IndentStyle) {
        self.indent = style;
    }

    pub fn detect_indent(&mut self, fallback: IndentStyle) {
        let indents: Vec<Indent> = (0..self.len_lines().min(DETECT_LINES))
            .filter_map(|line| indent::measure(&self.line_text(line)))
            .collect();
        self.indent = indent::detect(&indents, fallback);
    }

    pub fn leading_whitespace(&self, line: usize) -> String {
        self.line_chars(line)
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    pub fn indent_width_of(&self, line: usize) -> usize {
        self.leading_whitespace(line).chars().count()
    }

    pub fn utf16_col(&self, line: usize, char_col: usize) -> usize {
        self.line_chars(line)
            .take(char_col)
            .map(char::len_utf16)
            .sum()
    }

    pub fn col_from_utf16(&self, line: usize, utf16_col: usize) -> usize {
        if line >= self.len_lines() {
            return 0;
        }
        let mut remaining = utf16_col;
        let mut col = 0;
        for ch in self.line_chars(line) {
            let width = ch.len_utf16();
            if remaining < width {
                break;
            }
            remaining -= width;
            col += 1;
        }
        col
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn file_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "[scratch]".to_string())
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_len_chars(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
        }
        if len > 0 && slice.char(len - 1) == '\r' {
            len -= 1;
        }
        len
    }

    pub fn line_chars(&self, line: usize) -> impl Iterator<Item = char> + '_ {
        let len = self.line_len_chars(line);
        self.rope.line(line).chars().take(len)
    }

    pub fn line_text(&self, line: usize) -> String {
        self.line_chars(line).collect()
    }

    pub fn line_with_newline(&self, line: usize) -> String {
        let mut text = self.line_text(line);
        text.push('\n');
        text
    }

    pub fn clamp(&self, pos: Position) -> Position {
        let line = pos.line.min(self.len_lines().saturating_sub(1));
        Position::new(line, pos.col.min(self.line_len_chars(line)))
    }

    pub fn char_idx(&self, pos: Position) -> usize {
        let pos = self.clamp(pos);
        self.rope.line_to_char(pos.line) + pos.col
    }

    pub fn pos_of(&self, idx: usize) -> Position {
        let idx = idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        Position::new(line, idx - self.rope.line_to_char(line))
    }

    pub fn end_position(&self) -> Position {
        self.pos_of(self.rope.len_chars())
    }

    pub fn display_col(&self, pos: Position, tab_width: usize) -> usize {
        let mut d = 0;
        for (i, ch) in self.line_chars(pos.line).enumerate() {
            if i >= pos.col {
                break;
            }
            d += char_display_width(ch, d, tab_width);
        }
        d
    }

    pub fn col_at_display_col(&self, line: usize, target: usize, tab_width: usize) -> usize {
        let mut d = 0;
        for (i, ch) in self.line_chars(line).enumerate() {
            let w = char_display_width(ch, d, tab_width);
            if w > 0 && target < d + w {
                return i;
            }
            d += w;
        }
        self.line_len_chars(line)
    }

    pub fn set_cursor_pos(&mut self, pos: Position, extend: bool) {
        self.history.commit();
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor.pos);
            }
        } else {
            self.anchor = None;
        }
        self.set_cursor(pos, true);
    }

    fn set_cursor(&mut self, pos: Position, update_goal: bool) {
        let pos = self.clamp(pos);
        self.cursor.pos = pos;
        if update_goal {
            self.cursor.goal_col = pos.col;
        }
    }

    pub fn selection(&self) -> Option<Selection> {
        let anchor = self.anchor?;
        if anchor == self.cursor.pos {
            None
        } else {
            Some(Selection::from_points(anchor, self.cursor.pos))
        }
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(Position::new(0, 0));
        let end = self.end_position();
        self.set_cursor(end, true);
    }

    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection()?;
        let (si, ei) = (self.char_idx(sel.start), self.char_idx(sel.end));
        Some(self.rope.slice(si..ei).to_string())
    }

    pub fn move_cursor(&mut self, motion: Motion, extend: bool, page_rows: usize) {
        self.history.commit();
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor.pos);
            }
        } else if let Some(sel) = self.selection() {
            self.anchor = None;
            match motion {
                Motion::Left => {
                    self.set_cursor(sel.start, true);
                    return;
                }
                Motion::Right => {
                    self.set_cursor(sel.end, true);
                    return;
                }
                _ => {}
            }
        } else {
            self.anchor = None;
        }

        let pos = self.cursor.pos;
        let goal = self.cursor.goal_col;
        let last_line = self.len_lines() - 1;
        let (new_pos, update_goal) = match motion {
            Motion::Left => {
                if pos.col > 0 {
                    (Position::new(pos.line, pos.col - 1), true)
                } else if pos.line > 0 {
                    let line = pos.line - 1;
                    (Position::new(line, self.line_len_chars(line)), true)
                } else {
                    (pos, true)
                }
            }
            Motion::Right => {
                if pos.col < self.line_len_chars(pos.line) {
                    (Position::new(pos.line, pos.col + 1), true)
                } else if pos.line < last_line {
                    (Position::new(pos.line + 1, 0), true)
                } else {
                    (pos, true)
                }
            }
            Motion::Up => {
                if pos.line > 0 {
                    (Position::new(pos.line - 1, goal), false)
                } else {
                    (Position::new(0, 0), true)
                }
            }
            Motion::Down => {
                if pos.line < last_line {
                    (Position::new(pos.line + 1, goal), false)
                } else {
                    (Position::new(pos.line, self.line_len_chars(pos.line)), true)
                }
            }
            Motion::WordLeft => {
                let idx = self.word_left_idx(self.char_idx(pos));
                (self.pos_of(idx), true)
            }
            Motion::WordRight => {
                let idx = self.word_right_idx(self.char_idx(pos));
                (self.pos_of(idx), true)
            }
            Motion::LineStart => (Position::new(pos.line, 0), true),
            Motion::LineEnd => (Position::new(pos.line, self.line_len_chars(pos.line)), true),
            Motion::PageUp => (
                Position::new(pos.line.saturating_sub(page_rows), goal),
                false,
            ),
            Motion::PageDown => (
                Position::new((pos.line + page_rows).min(last_line), goal),
                false,
            ),
            Motion::BufferStart => (Position::new(0, 0), true),
            Motion::BufferEnd => (self.end_position(), true),
        };
        self.set_cursor(new_pos, update_goal);
    }

    fn word_right_idx(&self, mut idx: usize) -> usize {
        let len = self.rope.len_chars();
        while idx < len && char_class(self.rope.char(idx)) == CharClass::Whitespace {
            idx += 1;
        }
        if idx >= len {
            return len;
        }
        let class = char_class(self.rope.char(idx));
        while idx < len && char_class(self.rope.char(idx)) == class {
            idx += 1;
        }
        idx
    }

    fn word_left_idx(&self, mut idx: usize) -> usize {
        while idx > 0 && char_class(self.rope.char(idx - 1)) == CharClass::Whitespace {
            idx -= 1;
        }
        if idx == 0 {
            return 0;
        }
        let class = char_class(self.rope.char(idx - 1));
        while idx > 0 && char_class(self.rope.char(idx - 1)) == class {
            idx -= 1;
        }
        idx
    }

    pub fn insert_plain(&mut self, text: &str) {
        debug_assert!(!text.contains('\n'));
        let had_selection = self.delete_selection_with(Merge::Start);
        self.anchor = None;
        let before = self.cursor.pos;
        let at = self.char_idx(before);
        self.rope.insert(at, text);
        self.touch(before.line);
        self.set_cursor(
            Position::new(before.line, before.col + text.chars().count()),
            true,
        );
        let merge = if had_selection {
            Merge::Always
        } else {
            Merge::Auto(GroupKind::Insert)
        };
        self.history.record(
            EditOp::Insert {
                at,
                text: text.to_string(),
            },
            merge,
            before,
            self.cursor.pos,
        );
    }

    pub fn insert_char(&mut self, c: char) {
        let mut utf8 = [0u8; 4];
        self.insert_plain(c.encode_utf8(&mut utf8));
    }

    pub fn insert_tab(&mut self, display_tab_width: usize) {
        let style = self.indent;
        if style.use_spaces {
            let col = self.display_col(self.cursor.pos, display_tab_width);
            let n = style.width - (col % style.width);
            self.insert_plain(&" ".repeat(n));
        } else {
            self.insert_plain("\t");
        }
    }

    pub fn insert_newline(&mut self) {
        let had_selection = self.delete_selection_with(Merge::Start);
        self.anchor = None;
        let before = self.cursor.pos;
        let at = self.char_idx(before);
        self.rope.insert(at, "\n");
        self.touch(before.line);
        self.set_cursor(Position::new(before.line + 1, 0), true);
        let merge = if had_selection {
            Merge::Always
        } else {
            Merge::Start
        };
        self.history.record(
            EditOp::Insert {
                at,
                text: "\n".to_string(),
            },
            merge,
            before,
            self.cursor.pos,
        );
    }

    pub fn insert_newline_indented(&mut self, indent_text: &str, closing: Option<&str>) {
        let had_selection = self.delete_selection_with(Merge::Start);
        self.anchor = None;
        let before = self.cursor.pos;
        let at = self.char_idx(before);
        let mut text = String::with_capacity(indent_text.len() + 2);
        text.push('\n');
        text.push_str(indent_text);
        let cursor_at = at + text.chars().count();
        if let Some(closing) = closing {
            text.push('\n');
            text.push_str(closing);
        }
        self.rope.insert(at, &text);
        self.touch(before.line);
        let after = self.pos_of(cursor_at);
        self.set_cursor(after, true);
        let merge = if had_selection {
            Merge::Always
        } else {
            Merge::Start
        };
        self.history
            .record(EditOp::Insert { at, text }, merge, before, self.cursor.pos);
    }

    pub fn delete_before_cursor(&mut self, count: usize) {
        self.anchor = None;
        let before = self.cursor.pos;
        let idx = self.char_idx(before);
        let count = count.min(idx);
        if count == 0 {
            return;
        }
        let text = self.rope.slice(idx - count..idx).to_string();
        let new_pos = self.pos_of(idx - count);
        self.rope.remove(idx - count..idx);
        self.touch(new_pos.line);
        self.set_cursor(new_pos, true);
        self.history.record(
            EditOp::Delete {
                at: idx - count,
                text,
            },
            Merge::Auto(GroupKind::Delete),
            before,
            self.cursor.pos,
        );
    }

    pub fn indent_lines(&mut self, first: usize, last: usize, unit: &str) {
        let last = last.min(self.len_lines().saturating_sub(1));
        let width = unit.chars().count();
        if width == 0 || first > last {
            return;
        }
        let shifts: Vec<bool> = (first..=last)
            .map(|line| self.line_len_chars(line) > 0)
            .collect();
        let shift_of = |pos: Position| -> usize {
            if pos.line < first || pos.line > last {
                return 0;
            }
            if shifts[pos.line - first] {
                width
            } else {
                0
            }
        };
        let before = self.cursor.pos;
        let after = Position::new(before.line, before.col + shift_of(before));
        let new_anchor = self
            .anchor
            .map(|a| Position::new(a.line, a.col + shift_of(a)));

        let mut started = false;
        for line in first..=last {
            if !shifts[line - first] {
                continue;
            }
            let at = self.rope.line_to_char(line);
            self.rope.insert(at, unit);
            let merge = if started { Merge::Always } else { Merge::Start };
            started = true;
            self.history.record(
                EditOp::Insert {
                    at,
                    text: unit.to_string(),
                },
                merge,
                before,
                after,
            );
        }
        if !started {
            return;
        }
        self.touch(first);
        self.anchor = new_anchor;
        self.set_cursor(after, true);
    }

    pub fn dedent_lines(&mut self, first: usize, last: usize, width: usize) {
        let last = last.min(self.len_lines().saturating_sub(1));
        if first > last {
            return;
        }
        let removals: Vec<usize> = (first..=last)
            .map(|line| {
                let ws = self.leading_whitespace(line);
                match ws.chars().next() {
                    Some('\t') => 1,
                    Some(' ') => ws.chars().take_while(|c| *c == ' ').count().min(width),
                    _ => 0,
                }
            })
            .collect();
        if removals.iter().all(|r| *r == 0) {
            return;
        }
        let shift_of = |pos: Position| -> usize {
            if pos.line < first || pos.line > last {
                return 0;
            }
            removals[pos.line - first].min(pos.col)
        };
        let before = self.cursor.pos;
        let after = Position::new(before.line, before.col - shift_of(before));
        let new_anchor = self
            .anchor
            .map(|a| Position::new(a.line, a.col - shift_of(a)));

        let mut started = false;
        for line in first..=last {
            let remove = removals[line - first];
            if remove == 0 {
                continue;
            }
            let at = self.rope.line_to_char(line);
            let text = self.rope.slice(at..at + remove).to_string();
            self.rope.remove(at..at + remove);
            let merge = if started { Merge::Always } else { Merge::Start };
            started = true;
            self.history
                .record(EditOp::Delete { at, text }, merge, before, after);
        }
        self.touch(first);
        self.anchor = new_anchor;
        self.set_cursor(after, true);
    }

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let had_selection = self.delete_selection_with(Merge::Start);
        self.anchor = None;
        let before = self.cursor.pos;
        let at = self.char_idx(before);
        self.rope.insert(at, text);
        self.touch(before.line);
        let after = self.pos_of(at + text.chars().count());
        self.set_cursor(after, true);
        let merge = if had_selection {
            Merge::Always
        } else {
            Merge::Start
        };
        self.history.record(
            EditOp::Insert {
                at,
                text: text.to_string(),
            },
            merge,
            before,
            self.cursor.pos,
        );
    }

    pub fn replace_ranges(&mut self, ranges: &[(Position, Position)], text: &str) -> usize {
        self.history.commit();
        let mut spans: Vec<(usize, usize)> = ranges
            .iter()
            .map(|(start, end)| (self.char_idx(*start), self.char_idx(*end)))
            .filter(|(start, end)| end > start)
            .collect();
        spans.sort_unstable();
        spans.dedup();
        let Some((first_start, _)) = spans.first().copied() else {
            return 0;
        };
        let (last_start, _) = spans.last().copied().expect("checked above");

        let inserted = text.chars().count();
        let shift: isize = spans[..spans.len() - 1]
            .iter()
            .map(|(start, end)| inserted as isize - (end - start) as isize)
            .sum();
        let cursor_idx = (last_start as isize + shift + inserted as isize).max(0) as usize;

        let before = self.cursor.pos;
        self.anchor = None;
        let mut ops = Vec::with_capacity(spans.len() * 2);
        for (start, end) in spans.iter().rev() {
            let removed = self.rope.slice(*start..*end).to_string();
            self.rope.remove(*start..*end);
            ops.push(EditOp::Delete {
                at: *start,
                text: removed,
            });
            if !text.is_empty() {
                self.rope.insert(*start, text);
                ops.push(EditOp::Insert {
                    at: *start,
                    text: text.to_string(),
                });
            }
        }
        self.touch(self.pos_of(first_start).line);
        let after = self.pos_of(cursor_idx.min(self.rope.len_chars()));
        self.set_cursor(after, true);
        for (i, op) in ops.into_iter().enumerate() {
            let merge = if i == 0 { Merge::Start } else { Merge::Always };
            self.history.record(op, merge, before, after);
        }
        self.history.commit();
        spans.len()
    }

    pub fn backspace(&mut self) {
        if self.delete_selection_with(Merge::Start) {
            return;
        }
        self.anchor = None;
        let before = self.cursor.pos;
        let idx = self.char_idx(before);
        if idx == 0 {
            return;
        }
        let ch = self.rope.char(idx - 1);
        let new_pos = self.pos_of(idx - 1);
        self.rope.remove(idx - 1..idx);
        self.touch(new_pos.line);
        self.set_cursor(new_pos, true);
        self.history.record(
            EditOp::Delete {
                at: idx - 1,
                text: ch.to_string(),
            },
            Merge::Auto(GroupKind::Delete),
            before,
            self.cursor.pos,
        );
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection_with(Merge::Start) {
            return;
        }
        self.anchor = None;
        let before = self.cursor.pos;
        let idx = self.char_idx(before);
        if idx >= self.rope.len_chars() {
            return;
        }
        let ch = self.rope.char(idx);
        self.rope.remove(idx..idx + 1);
        self.touch(before.line);
        self.history.record(
            EditOp::Delete {
                at: idx,
                text: ch.to_string(),
            },
            Merge::Auto(GroupKind::Delete),
            before,
            self.cursor.pos,
        );
    }

    pub fn delete_selection(&mut self) -> bool {
        self.delete_selection_with(Merge::Start)
    }

    fn delete_selection_with(&mut self, merge: Merge) -> bool {
        let Some(sel) = self.selection() else {
            return false;
        };
        let before = self.cursor.pos;
        let (si, ei) = (self.char_idx(sel.start), self.char_idx(sel.end));
        let text = self.rope.slice(si..ei).to_string();
        self.rope.remove(si..ei);
        self.touch(sel.start.line);
        self.anchor = None;
        self.set_cursor(sel.start, true);
        self.history.record(
            EditOp::Delete { at: si, text },
            merge,
            before,
            self.cursor.pos,
        );
        true
    }

    pub fn delete_line(&mut self, line: usize) {
        self.anchor = None;
        let before = self.cursor.pos;
        let start = self.rope.line_to_char(line);
        let end = start + self.rope.line(line).len_chars();
        let at = if end == self.rope.len_chars() && start > 0 {
            start - 1
        } else {
            start
        };
        if at == end {
            return;
        }
        let text = self.rope.slice(at..end).to_string();
        self.rope.remove(at..end);
        self.touch(line.saturating_sub(1));
        self.set_cursor(Position::new(line, before.col), true);
        self.history.record(
            EditOp::Delete { at, text },
            Merge::Start,
            before,
            self.cursor.pos,
        );
    }

    pub fn duplicate_line(&mut self) {
        self.anchor = None;
        let before = self.cursor.pos;
        let line = before.line;
        let at = self.rope.line_to_char(line);
        let text = self.line_with_newline(line);
        self.rope.insert(at, &text);
        self.touch(line);
        self.set_cursor(Position::new(line + 1, before.col), true);
        self.history.record(
            EditOp::Insert { at, text },
            Merge::Start,
            before,
            self.cursor.pos,
        );
    }

    pub fn move_line_up(&mut self) {
        let line = self.cursor.pos.line;
        if line > 0 {
            self.swap_lines(line - 1, line - 1);
        }
    }

    pub fn move_line_down(&mut self) {
        let line = self.cursor.pos.line;
        if line + 1 < self.len_lines() {
            self.swap_lines(line, line + 1);
        }
    }

    fn swap_lines(&mut self, a: usize, cursor_line: usize) {
        self.anchor = None;
        let before = self.cursor.pos;
        let b = a + 1;
        let start = self.rope.line_to_char(a);
        let end = self.rope.line_to_char(b) + self.rope.line(b).len_chars();
        let old = self.rope.slice(start..end).to_string();
        let (line_a, line_b) = (self.line_text(a), self.line_text(b));
        let mut new = String::with_capacity(old.len());
        new.push_str(&line_b);
        new.push('\n');
        new.push_str(&line_a);
        if old.ends_with('\n') {
            new.push('\n');
        }
        self.rope.remove(start..start + old.chars().count());
        self.rope.insert(start, &new);
        self.touch(a);
        self.set_cursor(Position::new(cursor_line, before.col), true);
        let after = self.cursor.pos;
        self.history.record(
            EditOp::Delete {
                at: start,
                text: old,
            },
            Merge::Start,
            before,
            after,
        );
        self.history.record(
            EditOp::Insert {
                at: start,
                text: new,
            },
            Merge::Always,
            before,
            after,
        );
    }

    pub fn commit_undo_group(&mut self) {
        self.history.commit();
    }

    pub fn char_before_cursor(&self) -> Option<char> {
        let idx = self.char_idx(self.cursor.pos);
        if idx == 0 {
            None
        } else {
            Some(self.rope.char(idx - 1))
        }
    }

    pub fn char_after_cursor(&self) -> Option<char> {
        let idx = self.char_idx(self.cursor.pos);
        if idx >= self.rope.len_chars() {
            None
        } else {
            Some(self.rope.char(idx))
        }
    }

    pub fn insert_pair(&mut self, open: char, close: char) {
        self.anchor = None;
        let before = self.cursor.pos;
        let at = self.char_idx(before);
        let mut text = String::with_capacity(8);
        text.push(open);
        text.push(close);
        self.rope.insert(at, &text);
        self.touch(before.line);
        self.set_cursor(Position::new(before.line, before.col + 1), true);
        self.history.record(
            EditOp::Insert { at, text },
            Merge::Auto(GroupKind::Insert),
            before,
            self.cursor.pos,
        );
    }

    pub fn surround_selection(&mut self, open: char, close: char) -> bool {
        let Some(sel) = self.selection() else {
            return false;
        };
        let before = self.cursor.pos;
        let (si, ei) = (self.char_idx(sel.start), self.char_idx(sel.end));
        self.rope.insert_char(ei, close);
        self.rope.insert_char(si, open);
        self.touch(sel.start.line);
        self.anchor = Some(self.pos_of(si + 1));
        let after = self.pos_of(ei + 1);
        self.set_cursor(after, true);
        self.history.record(
            EditOp::Insert {
                at: ei,
                text: close.to_string(),
            },
            Merge::Start,
            before,
            self.cursor.pos,
        );
        self.history.record(
            EditOp::Insert {
                at: si,
                text: open.to_string(),
            },
            Merge::Always,
            before,
            self.cursor.pos,
        );
        true
    }

    pub fn delete_pair_around_cursor(&mut self) {
        self.anchor = None;
        let before = self.cursor.pos;
        let idx = self.char_idx(before);
        if idx == 0 || idx >= self.rope.len_chars() {
            return;
        }
        let text = self.rope.slice(idx - 1..idx + 1).to_string();
        let new_pos = self.pos_of(idx - 1);
        self.rope.remove(idx - 1..idx + 1);
        self.touch(new_pos.line);
        self.set_cursor(new_pos, true);
        self.history.record(
            EditOp::Delete { at: idx - 1, text },
            Merge::Auto(GroupKind::Delete),
            before,
            self.cursor.pos,
        );
    }

    pub fn undo(&mut self) -> bool {
        self.history.commit();
        let Some(tx) = self.history.pop_undo() else {
            return false;
        };
        for op in tx.ops.iter().rev() {
            match op {
                EditOp::Insert { at, text } => {
                    self.rope.remove(*at..at + text.chars().count());
                }
                EditOp::Delete { at, text } => {
                    self.rope.insert(*at, text);
                }
            }
        }
        self.anchor = None;
        self.touch(tx.before.line.min(tx.after.line));
        let pos = self.clamp(tx.before);
        self.set_cursor(pos, true);
        self.history.push_redo(tx);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(tx) = self.history.pop_redo() else {
            return false;
        };
        for op in &tx.ops {
            match op {
                EditOp::Insert { at, text } => {
                    self.rope.insert(*at, text);
                }
                EditOp::Delete { at, text } => {
                    self.rope.remove(*at..at + text.chars().count());
                }
            }
        }
        self.anchor = None;
        self.touch(tx.before.line.min(tx.after.line));
        let pos = self.clamp(tx.after);
        self.set_cursor(pos, true);
        self.history.push_undo(tx);
        true
    }

    pub fn save(&mut self) -> io::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer has no file path",
            ));
        };
        let mut writer = BufWriter::new(File::create(path)?);
        self.rope.write_to(&mut writer)?;
        writer.flush()?;
        self.dirty = false;
        self.history.commit();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Word,
    Punctuation,
}

fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_with(text: &str) -> Buffer {
        Buffer::from_rope(Rope::from_str(text), None)
    }

    fn type_str(buf: &mut Buffer, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                buf.insert_newline();
            } else {
                buf.insert_char(c);
            }
        }
    }

    #[test]
    fn typing_a_word_is_one_undo_step() {
        let mut buf = Buffer::scratch();
        type_str(&mut buf, "hello");
        assert_eq!(buf.text(), "hello");
        assert!(buf.undo());
        assert_eq!(buf.text(), "");
        assert!(buf.redo());
        assert_eq!(buf.text(), "hello");
        assert_eq!(buf.cursor.pos, Position::new(0, 5));
    }

    #[test]
    fn newline_breaks_undo_groups() {
        let mut buf = Buffer::scratch();
        type_str(&mut buf, "ab\ncd");
        assert!(buf.undo());
        assert_eq!(buf.text(), "ab\n");
        assert!(buf.undo());
        assert_eq!(buf.text(), "ab");
        assert!(buf.undo());
        assert_eq!(buf.text(), "");
        assert!(!buf.undo());
    }

    #[test]
    fn movement_breaks_undo_groups() {
        let mut buf = Buffer::scratch();
        type_str(&mut buf, "ab");
        buf.move_cursor(Motion::Left, false, 1);
        type_str(&mut buf, "x");
        assert_eq!(buf.text(), "axb");
        assert!(buf.undo());
        assert_eq!(buf.text(), "ab");
        assert!(buf.undo());
        assert_eq!(buf.text(), "");
    }

    #[test]
    fn consecutive_backspaces_are_one_undo_step() {
        let mut buf = buffer_with("abc");
        buf.move_cursor(Motion::LineEnd, false, 1);
        buf.backspace();
        buf.backspace();
        buf.backspace();
        assert_eq!(buf.text(), "");
        assert!(buf.undo());
        assert_eq!(buf.text(), "abc");
    }

    #[test]
    fn backspace_joins_lines() {
        let mut buf = buffer_with("ab\ncd");
        buf.cursor.pos = Position::new(1, 0);
        buf.backspace();
        assert_eq!(buf.text(), "abcd");
        assert_eq!(buf.cursor.pos, Position::new(0, 2));
    }

    #[test]
    fn delete_forward_at_line_end_joins_lines() {
        let mut buf = buffer_with("ab\ncd");
        buf.move_cursor(Motion::LineEnd, false, 1);
        buf.delete_forward();
        assert_eq!(buf.text(), "abcd");
    }

    #[test]
    fn typing_over_selection_is_one_undo_step() {
        let mut buf = buffer_with("hello");
        buf.select_all();
        buf.insert_char('x');
        assert_eq!(buf.text(), "x");
        assert!(buf.undo());
        assert_eq!(buf.text(), "hello");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut buf = Buffer::scratch();
        type_str(&mut buf, "abc");
        buf.undo();
        type_str(&mut buf, "x");
        assert!(!buf.redo());
        assert_eq!(buf.text(), "x");
    }

    #[test]
    fn vertical_movement_keeps_goal_column() {
        let mut buf = buffer_with("long line\nab\nanother line");
        buf.cursor.pos = Position::new(0, 7);
        buf.cursor.goal_col = 7;
        buf.move_cursor(Motion::Down, false, 1);
        assert_eq!(buf.cursor.pos, Position::new(1, 2));
        buf.move_cursor(Motion::Down, false, 1);
        assert_eq!(buf.cursor.pos, Position::new(2, 7));
    }

    #[test]
    fn word_movement() {
        let mut buf = buffer_with("foo bar_baz  qux");
        buf.move_cursor(Motion::WordRight, false, 1);
        assert_eq!(buf.cursor.pos.col, 3);
        buf.move_cursor(Motion::WordRight, false, 1);
        assert_eq!(buf.cursor.pos.col, 11);
        buf.move_cursor(Motion::WordRight, false, 1);
        assert_eq!(buf.cursor.pos.col, 16);
        buf.move_cursor(Motion::WordLeft, false, 1);
        assert_eq!(buf.cursor.pos.col, 13);
        buf.move_cursor(Motion::WordLeft, false, 1);
        assert_eq!(buf.cursor.pos.col, 4);
    }

    #[test]
    fn word_movement_crosses_lines() {
        let mut buf = buffer_with("foo\nbar");
        buf.move_cursor(Motion::LineEnd, false, 1);
        buf.move_cursor(Motion::WordRight, false, 1);
        assert_eq!(buf.cursor.pos, Position::new(1, 3));
    }

    #[test]
    fn selection_via_extend_and_delete() {
        let mut buf = buffer_with("hello world");
        for _ in 0..5 {
            buf.move_cursor(Motion::Right, true, 1);
        }
        assert_eq!(buf.selected_text().as_deref(), Some("hello"));
        buf.delete_selection();
        assert_eq!(buf.text(), " world");
        assert!(buf.undo());
        assert_eq!(buf.text(), "hello world");
    }

    #[test]
    fn plain_left_collapses_selection_to_start() {
        let mut buf = buffer_with("hello");
        buf.move_cursor(Motion::Right, true, 1);
        buf.move_cursor(Motion::Right, true, 1);
        buf.move_cursor(Motion::Left, false, 1);
        assert!(buf.selection().is_none());
        assert_eq!(buf.cursor.pos, Position::new(0, 0));
    }

    #[test]
    fn move_line_down_and_up_round_trip() {
        let mut buf = buffer_with("one\ntwo\nthree");
        buf.cursor.pos = Position::new(0, 2);
        buf.move_line_down();
        assert_eq!(buf.text(), "two\none\nthree");
        assert_eq!(buf.cursor.pos, Position::new(1, 2));
        buf.move_line_up();
        assert_eq!(buf.text(), "one\ntwo\nthree");
        assert_eq!(buf.cursor.pos, Position::new(0, 2));
    }

    #[test]
    fn move_last_line_without_trailing_newline() {
        let mut buf = buffer_with("one\ntwo");
        buf.cursor.pos = Position::new(1, 0);
        buf.move_line_up();
        assert_eq!(buf.text(), "two\none");
        assert!(buf.undo());
        assert_eq!(buf.text(), "one\ntwo");
    }

    #[test]
    fn duplicate_line_moves_cursor_down() {
        let mut buf = buffer_with("one\ntwo");
        buf.cursor.pos = Position::new(0, 1);
        buf.duplicate_line();
        assert_eq!(buf.text(), "one\none\ntwo");
        assert_eq!(buf.cursor.pos, Position::new(1, 1));
        assert!(buf.undo());
        assert_eq!(buf.text(), "one\ntwo");
    }

    #[test]
    fn delete_last_line_removes_preceding_newline() {
        let mut buf = buffer_with("one\ntwo");
        buf.cursor.pos = Position::new(1, 0);
        buf.delete_line(1);
        assert_eq!(buf.text(), "one");
        assert!(buf.undo());
        assert_eq!(buf.text(), "one\ntwo");
    }

    #[test]
    fn line_len_ignores_line_endings() {
        let buf = buffer_with("ab\r\ncd\n");
        assert_eq!(buf.line_len_chars(0), 2);
        assert_eq!(buf.line_len_chars(1), 2);
        assert_eq!(buf.line_len_chars(2), 0);
    }

    #[test]
    fn insert_text_multiline_places_cursor_after() {
        let mut buf = buffer_with("ab");
        buf.move_cursor(Motion::Right, false, 1);
        buf.insert_text("x\ny");
        assert_eq!(buf.text(), "ax\nyb");
        assert_eq!(buf.cursor.pos, Position::new(1, 1));
        assert!(buf.undo());
        assert_eq!(buf.text(), "ab");
    }

    #[test]
    fn display_col_handles_tabs_and_wide_chars() {
        let buf = buffer_with("\ta你");
        assert_eq!(buf.display_col(Position::new(0, 1), 4), 4);
        assert_eq!(buf.display_col(Position::new(0, 2), 4), 5);
        assert_eq!(buf.display_col(Position::new(0, 3), 4), 7);
    }

    #[test]
    fn utf16_columns_count_surrogate_pairs() {
        let buf = buffer_with("a𝕏b");
        assert_eq!(buf.utf16_col(0, 0), 0);
        assert_eq!(buf.utf16_col(0, 1), 1);
        assert_eq!(buf.utf16_col(0, 2), 3);
        assert_eq!(buf.utf16_col(0, 3), 4);
    }

    #[test]
    fn revision_increments_on_edits() {
        let mut buf = buffer_with("ab");
        let before = buf.revision();
        buf.insert_char('x');
        assert!(buf.revision() > before);
    }

    #[test]
    fn col_at_display_col_handles_tabs_and_wide_chars() {
        let buf = buffer_with("\ta你x");
        assert_eq!(buf.col_at_display_col(0, 0, 4), 0);
        assert_eq!(buf.col_at_display_col(0, 3, 4), 0);
        assert_eq!(buf.col_at_display_col(0, 4, 4), 1);
        assert_eq!(buf.col_at_display_col(0, 5, 4), 2);
        assert_eq!(buf.col_at_display_col(0, 6, 4), 2);
        assert_eq!(buf.col_at_display_col(0, 7, 4), 3);
        assert_eq!(buf.col_at_display_col(0, 99, 4), 4);
    }

    #[test]
    fn set_cursor_pos_places_and_extends() {
        let mut buf = buffer_with("hello\nworld");
        buf.set_cursor_pos(Position::new(1, 3), false);
        assert_eq!(buf.cursor.pos, Position::new(1, 3));
        assert!(buf.selection().is_none());
        buf.set_cursor_pos(Position::new(0, 1), true);
        assert_eq!(buf.selected_text().as_deref(), Some("ello\nwor"));
        buf.set_cursor_pos(Position::new(9, 99), false);
        assert_eq!(buf.cursor.pos, Position::new(1, 5));
        assert!(buf.selection().is_none());
    }

    #[test]
    fn indent_lines_shifts_text_and_selection() {
        let mut buf = buffer_with("one\ntwo\n\nthree");
        buf.cursor.pos = Position::new(0, 1);
        buf.cursor.goal_col = 1;
        buf.move_cursor(Motion::Down, true, 1);
        buf.indent_lines(0, 3, "  ");
        assert_eq!(buf.text(), "  one\n  two\n\n  three");
        assert_eq!(buf.cursor.pos, Position::new(1, 3));
        assert_eq!(buf.selection().unwrap().start, Position::new(0, 3));
        assert!(buf.undo());
        assert_eq!(buf.text(), "one\ntwo\n\nthree");
    }

    #[test]
    fn dedent_lines_removes_one_level() {
        let mut buf = buffer_with("    one\n  two\nthree\n\tfour");
        buf.cursor.pos = Position::new(0, 6);
        buf.dedent_lines(0, 3, 4);
        assert_eq!(buf.text(), "one\ntwo\nthree\nfour");
        assert_eq!(buf.cursor.pos, Position::new(0, 2));
        assert!(buf.undo());
        assert_eq!(buf.text(), "    one\n  two\nthree\n\tfour");
    }

    #[test]
    fn dedent_without_indentation_does_nothing() {
        let mut buf = buffer_with("one\ntwo");
        buf.dedent_lines(0, 1, 4);
        assert_eq!(buf.text(), "one\ntwo");
        assert!(!buf.undo());
    }

    #[test]
    fn newline_indented_with_and_without_closing() {
        let mut buf = buffer_with("    fn x() {");
        buf.move_cursor(Motion::LineEnd, false, 1);
        buf.insert_newline_indented("        ", Some("    "));
        assert_eq!(buf.text(), "    fn x() {\n        \n    ");
        assert_eq!(buf.cursor.pos, Position::new(1, 8));
        assert!(buf.undo());
        assert_eq!(buf.text(), "    fn x() {");

        buf.insert_newline_indented("  ", None);
        assert_eq!(buf.text(), "    fn x() {\n  ");
        assert_eq!(buf.cursor.pos, Position::new(1, 2));
    }

    #[test]
    fn delete_before_cursor_removes_run() {
        let mut buf = buffer_with("        x");
        buf.cursor.pos = Position::new(0, 8);
        buf.delete_before_cursor(4);
        assert_eq!(buf.text(), "    x");
        assert_eq!(buf.cursor.pos, Position::new(0, 4));
        assert!(buf.undo());
        assert_eq!(buf.text(), "        x");
    }

    #[test]
    fn detect_indent_sets_style() {
        let mut buf = buffer_with("a\n  b\n    c\n");
        buf.detect_indent(IndentStyle {
            use_spaces: true,
            width: 4,
        });
        assert_eq!(buf.indent().width, 2);
        assert!(buf.indent().use_spaces);
    }

    #[test]
    fn leading_whitespace_of_line() {
        let buf = buffer_with("  \tx\ny");
        assert_eq!(buf.leading_whitespace(0), "  \t");
        assert_eq!(buf.indent_width_of(0), 3);
        assert_eq!(buf.leading_whitespace(1), "");
    }

    #[test]
    fn replace_ranges_rewrites_every_span_in_one_undo_step() {
        let mut buf = buffer_with("foo bar\nfoo baz\nfoo");
        let ranges = [
            (Position::new(0, 0), Position::new(0, 3)),
            (Position::new(1, 0), Position::new(1, 3)),
            (Position::new(2, 0), Position::new(2, 3)),
        ];
        assert_eq!(buf.replace_ranges(&ranges, "quux"), 3);
        assert_eq!(buf.text(), "quux bar\nquux baz\nquux");
        assert_eq!(buf.cursor.pos, Position::new(2, 4));
        assert!(buf.undo());
        assert_eq!(buf.text(), "foo bar\nfoo baz\nfoo");
        assert!(buf.redo());
        assert_eq!(buf.text(), "quux bar\nquux baz\nquux");
    }

    #[test]
    fn replace_ranges_handles_deletion_and_empty_input() {
        let mut buf = buffer_with("aXbXc");
        let ranges = [
            (Position::new(0, 1), Position::new(0, 2)),
            (Position::new(0, 3), Position::new(0, 4)),
        ];
        assert_eq!(buf.replace_ranges(&ranges, ""), 2);
        assert_eq!(buf.text(), "abc");
        assert_eq!(buf.cursor.pos, Position::new(0, 2));
        assert!(buf.undo());
        assert_eq!(buf.text(), "aXbXc");
        assert_eq!(buf.replace_ranges(&[], "y"), 0);
        assert_eq!(buf.text(), "aXbXc");
    }

    #[test]
    fn replace_ranges_is_not_merged_with_neighbouring_edits() {
        let mut buf = buffer_with("foo");
        type_str(&mut buf, "z");
        buf.replace_ranges(&[(Position::new(0, 1), Position::new(0, 4))], "bar");
        assert_eq!(buf.text(), "zbar");
        assert!(buf.undo());
        assert_eq!(buf.text(), "zfoo");
        assert!(buf.undo());
        assert_eq!(buf.text(), "foo");
    }

    #[test]
    fn insert_tab_with_spaces_advances_to_tab_stop() {
        let mut buf = buffer_with("ab");
        buf.move_cursor(Motion::LineEnd, false, 1);
        buf.insert_tab(4);
        assert_eq!(buf.text(), "ab  ");
        let mut buf = Buffer::scratch();
        buf.set_indent(IndentStyle {
            use_spaces: false,
            width: 4,
        });
        buf.insert_tab(4);
        assert_eq!(buf.text(), "\t");
    }
}
