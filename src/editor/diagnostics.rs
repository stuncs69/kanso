use crate::buffer::{Buffer, Position};
use crate::lsp::Severity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub start: Position,
    pub end: Position,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
}

impl Diagnostic {
    pub fn summary(&self) -> String {
        let first = self
            .message
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("");
        match &self.source {
            Some(source) => format!("{first} [{source}]"),
            None => first.to_string(),
        }
    }

    pub fn covers(&self, pos: Position) -> bool {
        pos >= self.start && pos < self.end
    }
}

pub fn resolve(buffer: &Buffer, raw: &[crate::lsp::Diagnostic]) -> Vec<Diagnostic> {
    let last_line = buffer.len_lines().saturating_sub(1);
    let mut out: Vec<Diagnostic> = raw
        .iter()
        .map(|item| {
            let start_line = item.start_line.min(last_line);
            let end_line = item.end_line.min(last_line);
            let start = Position::new(
                start_line,
                buffer.col_from_utf16(start_line, item.start_utf16),
            );
            let mut end = Position::new(end_line, buffer.col_from_utf16(end_line, item.end_utf16));
            if end <= start {
                end = Position::new(start.line, start.col + 1);
            }
            Diagnostic {
                start,
                end,
                severity: item.severity,
                message: item.message.clone(),
                source: item.source.clone(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.severity.cmp(&b.severity)));
    out
}

pub fn counts(items: &[Diagnostic]) -> (usize, usize) {
    let errors = items
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = items
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count();
    (errors, warnings)
}

pub fn on_line(items: &[Diagnostic], line: usize) -> impl Iterator<Item = &Diagnostic> {
    items.iter().filter(move |d| d.start.line == line)
}

pub fn line_severity(items: &[Diagnostic], line: usize) -> Option<Severity> {
    on_line(items, line).map(|d| d.severity).min()
}

pub fn at_position(items: &[Diagnostic], pos: Position) -> Option<&Diagnostic> {
    items
        .iter()
        .filter(|d| d.covers(pos))
        .min_by_key(|d| d.severity)
        .or_else(|| on_line(items, pos.line).min_by_key(|d| d.severity))
}

pub fn step(items: &[Diagnostic], from: Position, forward: bool) -> Option<&Diagnostic> {
    if items.is_empty() {
        return None;
    }
    let found = if forward {
        items.iter().find(|d| d.start > from)
    } else {
        items.iter().rev().find(|d| d.start < from)
    };
    found.or_else(|| if forward { items.first() } else { items.last() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(line: usize, character: usize, severity: Severity) -> crate::lsp::Diagnostic {
        crate::lsp::Diagnostic {
            start_line: line,
            start_utf16: character,
            end_line: line,
            end_utf16: character + 3,
            severity,
            message: format!("problem at {line}"),
            source: None,
        }
    }

    fn buffer_with(text: &str) -> Buffer {
        let mut buffer = Buffer::scratch();
        buffer.insert_text(text);
        buffer
    }

    #[test]
    fn resolve_clamps_to_buffer_and_maps_utf16() {
        let buffer = buffer_with("a𝕏bcd\nshort\n");
        let items = resolve(
            &buffer,
            &[raw(0, 3, Severity::Error), raw(900, 0, Severity::Warning)],
        );
        assert_eq!(items[0].start, Position::new(0, 2));
        assert_eq!(items[0].end, Position::new(0, 5));
        assert_eq!(items[1].start.line, buffer.len_lines() - 1);
    }

    #[test]
    fn empty_ranges_still_cover_one_column() {
        let buffer = buffer_with("abc\n");
        let mut item = raw(0, 1, Severity::Error);
        item.end_utf16 = 1;
        let items = resolve(&buffer, &[item]);
        assert_eq!(items[0].start, Position::new(0, 1));
        assert_eq!(items[0].end, Position::new(0, 2));
        assert!(items[0].covers(Position::new(0, 1)));
        assert!(!items[0].covers(Position::new(0, 2)));
    }

    #[test]
    fn counts_and_line_severity_prefer_errors() {
        let buffer = buffer_with("one\ntwo\nthree\n");
        let items = resolve(
            &buffer,
            &[
                raw(1, 0, Severity::Warning),
                raw(1, 1, Severity::Error),
                raw(2, 0, Severity::Hint),
            ],
        );
        assert_eq!(counts(&items), (1, 1));
        assert_eq!(line_severity(&items, 1), Some(Severity::Error));
        assert_eq!(line_severity(&items, 2), Some(Severity::Hint));
        assert_eq!(line_severity(&items, 0), None);
    }

    #[test]
    fn step_moves_forward_and_wraps() {
        let buffer = buffer_with("one\ntwo\nthree\n");
        let items = resolve(
            &buffer,
            &[raw(0, 0, Severity::Error), raw(2, 0, Severity::Error)],
        );
        let next = step(&items, Position::new(0, 0), true).unwrap();
        assert_eq!(next.start.line, 2);
        let wrapped = step(&items, Position::new(2, 0), true).unwrap();
        assert_eq!(wrapped.start.line, 0);
        let previous = step(&items, Position::new(2, 0), false).unwrap();
        assert_eq!(previous.start.line, 0);
        let wrapped_back = step(&items, Position::new(0, 0), false).unwrap();
        assert_eq!(wrapped_back.start.line, 2);
        assert!(step(&[], Position::new(0, 0), true).is_none());
    }

    #[test]
    fn summary_takes_first_line_and_source() {
        let diagnostic = Diagnostic {
            start: Position::new(0, 0),
            end: Position::new(0, 1),
            severity: Severity::Error,
            message: "\nbad thing\nmore detail".to_string(),
            source: Some("clangd".to_string()),
        };
        assert_eq!(diagnostic.summary(), "bad thing [clangd]");
    }
}
