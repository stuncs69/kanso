mod language;

use std::path::Path;

use crate::buffer::Buffer;

pub use language::{detect, LanguageSpec, LANGUAGES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Type,
    Function,
    String,
    Comment,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineState {
    Normal,
    BlockComment(u32),
    Str(char),
}

pub struct Highlighter {
    spec: Option<&'static LanguageSpec>,
    states: Vec<LineState>,
    valid: usize,
}

impl Highlighter {
    pub fn new(path: Option<&Path>) -> Self {
        Highlighter {
            spec: detect(path),
            states: vec![LineState::Normal],
            valid: 1,
        }
    }

    pub fn invalidate_from(&mut self, line: usize) {
        self.valid = self.valid.min(line + 1);
    }

    pub fn line_spans(&mut self, buffer: &Buffer, line: usize) -> Vec<Span> {
        let Some(spec) = self.spec else {
            return Vec::new();
        };
        if line >= buffer.len_lines() {
            return Vec::new();
        }
        while self.valid <= line {
            let i = self.valid;
            let chars: Vec<char> = buffer.line_chars(i - 1).collect();
            let (_, next) = tokenize(spec, &chars, self.states[i - 1]);
            self.store_state(i, next);
            self.valid += 1;
        }
        let chars: Vec<char> = buffer.line_chars(line).collect();
        let (spans, next) = tokenize(spec, &chars, self.states[line]);
        self.store_state(line + 1, next);
        self.valid = self.valid.max(line + 2);
        spans
    }

    fn store_state(&mut self, index: usize, state: LineState) {
        if self.states.len() <= index {
            self.states.push(state);
        } else {
            self.states[index] = state;
        }
    }
}

fn tokenize(spec: &LanguageSpec, chars: &[char], start_state: LineState) -> (Vec<Span>, LineState) {
    let len = chars.len();
    let mut spans = Vec::new();
    let mut i = 0usize;

    match start_state {
        LineState::BlockComment(depth) => {
            let (end, state) = scan_block_comment(spec, chars, 0, depth);
            push_span(&mut spans, 0, end, TokenKind::Comment);
            if matches!(state, LineState::BlockComment(_)) {
                return (spans, state);
            }
            i = end;
        }
        LineState::Str(delim) => match scan_string(chars, 0, delim) {
            Some(end) => {
                push_span(&mut spans, 0, end, TokenKind::String);
                i = end;
            }
            None => {
                push_span(&mut spans, 0, len, TokenKind::String);
                return (spans, LineState::Str(delim));
            }
        },
        LineState::Normal => {}
    }

    while i < len {
        let c = chars[i];

        if let Some(marker) = spec.line_comment {
            if starts_with(chars, i, marker) {
                push_span(&mut spans, i, len, TokenKind::Comment);
                return (spans, LineState::Normal);
            }
        }

        if let Some((open, _)) = spec.block_comment {
            if starts_with(chars, i, open) {
                let (end, state) = scan_block_comment(spec, chars, i + open.chars().count(), 1);
                push_span(&mut spans, i, end, TokenKind::Comment);
                if matches!(state, LineState::BlockComment(_)) {
                    return (spans, state);
                }
                i = end;
                continue;
            }
        }

        if spec.char_literal && c == '\'' {
            if let Some(end) = scan_char_literal(chars, i) {
                push_span(&mut spans, i, end, TokenKind::String);
                i = end;
            } else {
                i += 1;
                while i < len && is_ident_char(chars[i]) {
                    i += 1;
                }
            }
            continue;
        }

        if spec.string_delims.contains(&c) {
            match scan_string(chars, i + 1, c) {
                Some(end) => {
                    push_span(&mut spans, i, end, TokenKind::String);
                    i = end;
                }
                None => {
                    push_span(&mut spans, i, len, TokenKind::String);
                    if spec.multiline_string_delims.contains(&c) {
                        return (spans, LineState::Str(c));
                    }
                    return (spans, LineState::Normal);
                }
            }
            continue;
        }

        if c.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < len {
                let d = chars[i];
                if d.is_alphanumeric() || d == '_' {
                    i += 1;
                } else if d == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
                    i += 2;
                } else {
                    break;
                }
            }
            push_span(&mut spans, start, i, TokenKind::Number);
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < len && is_ident_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if spec.keywords.contains(&word.as_str()) {
                push_span(&mut spans, start, i, TokenKind::Keyword);
            } else if spec.types.contains(&word.as_str())
                || (spec.uppercase_types && c.is_uppercase())
            {
                push_span(&mut spans, start, i, TokenKind::Type);
            } else if chars.get(i) == Some(&'(') {
                push_span(&mut spans, start, i, TokenKind::Function);
            } else if spec.macro_bang && chars.get(i) == Some(&'!') {
                push_span(&mut spans, start, i + 1, TokenKind::Function);
            }
            continue;
        }

        i += 1;
    }

    (spans, LineState::Normal)
}

fn push_span(spans: &mut Vec<Span>, start: usize, end: usize, kind: TokenKind) {
    if start < end {
        spans.push(Span { start, end, kind });
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn starts_with(chars: &[char], at: usize, needle: &str) -> bool {
    (at..)
        .zip(needle.chars())
        .all(|(i, nc)| chars.get(i) == Some(&nc))
}

fn scan_string(chars: &[char], from: usize, delim: char) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
        } else if chars[i] == delim {
            return Some(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

fn scan_block_comment(
    spec: &LanguageSpec,
    chars: &[char],
    from: usize,
    depth: u32,
) -> (usize, LineState) {
    let Some((open, close)) = spec.block_comment else {
        return (chars.len(), LineState::Normal);
    };
    let mut depth = depth;
    let mut i = from;
    while i < chars.len() {
        if spec.nested_block_comments && starts_with(chars, i, open) {
            depth += 1;
            i += open.chars().count();
        } else if starts_with(chars, i, close) {
            depth -= 1;
            i += close.chars().count();
            if depth == 0 {
                return (i, LineState::Normal);
            }
        } else {
            i += 1;
        }
    }
    (chars.len(), LineState::BlockComment(depth))
}

fn scan_char_literal(chars: &[char], at: usize) -> Option<usize> {
    match chars.get(at + 1)? {
        '\\' => {
            let mut i = at + 3;
            while i < chars.len() && i < at + 12 {
                if chars[i] == '\'' {
                    return Some(i + 1);
                }
                i += 1;
            }
            None
        }
        '\'' => None,
        _ => {
            if chars.get(at + 2) == Some(&'\'') {
                Some(at + 3)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> &'static LanguageSpec {
        language::LANGUAGES.iter().find(|s| s.name == name).unwrap()
    }

    fn spans_of(spec_name: &str, line: &str) -> Vec<Span> {
        let chars: Vec<char> = line.chars().collect();
        tokenize(spec(spec_name), &chars, LineState::Normal).0
    }

    fn kinds(spans: &[Span]) -> Vec<TokenKind> {
        spans.iter().map(|s| s.kind).collect()
    }

    #[test]
    fn detects_language_by_extension() {
        assert_eq!(detect(Some(Path::new("main.rs"))).unwrap().name, "Rust");
        assert_eq!(detect(Some(Path::new("a.toml"))).unwrap().name, "TOML");
        assert!(detect(Some(Path::new("notes.txt"))).is_none());
        assert!(detect(None).is_none());
    }

    #[test]
    fn rust_keywords_functions_and_types() {
        let spans = spans_of("Rust", "fn main() -> Option<i32> {");
        assert_eq!(
            spans,
            vec![
                Span {
                    start: 0,
                    end: 2,
                    kind: TokenKind::Keyword
                },
                Span {
                    start: 3,
                    end: 7,
                    kind: TokenKind::Function
                },
                Span {
                    start: 13,
                    end: 19,
                    kind: TokenKind::Type
                },
                Span {
                    start: 20,
                    end: 23,
                    kind: TokenKind::Type
                },
            ]
        );
    }

    #[test]
    fn strings_and_line_comments() {
        let spans = spans_of("Rust", "let s = \"hi \\\" there\"; // done");
        assert_eq!(
            kinds(&spans),
            vec![TokenKind::Keyword, TokenKind::String, TokenKind::Comment]
        );
        assert_eq!(spans[1].start, 8);
        assert_eq!(spans[1].end, 21);
        assert_eq!(spans[2].end, 30);
    }

    #[test]
    fn numbers_and_macros() {
        let spans = spans_of("Rust", "println!(\"{}\", 42usize);");
        assert_eq!(
            kinds(&spans),
            vec![TokenKind::Function, TokenKind::String, TokenKind::Number]
        );
        assert_eq!(spans[0].end, 8);
    }

    #[test]
    fn char_literal_but_not_lifetime() {
        let spans = spans_of("Rust", "'a' 'static \\'\\n'");
        assert_eq!(
            spans.first().map(|s| (s.start, s.end, s.kind)),
            Some((0, 3, TokenKind::String))
        );
        assert!(!spans.iter().any(|s| s.start == 4));
        let spans = spans_of("Rust", "let c = '\\n';");
        assert!(spans
            .iter()
            .any(|s| s.kind == TokenKind::String && s.start == 8 && s.end == 12));
    }

    #[test]
    fn unterminated_string_carries_state() {
        let chars: Vec<char> = "let s = \"open".chars().collect();
        let (spans, state) = tokenize(spec("Rust"), &chars, LineState::Normal);
        assert_eq!(state, LineState::Str('"'));
        assert_eq!(spans.last().unwrap().kind, TokenKind::String);
        let chars: Vec<char> = "still\" fn".chars().collect();
        let (spans, state) = tokenize(spec("Rust"), &chars, LineState::Str('"'));
        assert_eq!(state, LineState::Normal);
        assert_eq!(
            spans[0],
            Span {
                start: 0,
                end: 6,
                kind: TokenKind::String
            }
        );
        assert_eq!(spans[1].kind, TokenKind::Keyword);
    }

    #[test]
    fn nested_block_comments_track_depth() {
        let chars: Vec<char> = "a /* one /* two".chars().collect();
        let (_, state) = tokenize(spec("Rust"), &chars, LineState::Normal);
        assert_eq!(state, LineState::BlockComment(2));
        let chars: Vec<char> = "*/ still */ fn".chars().collect();
        let (spans, state) = tokenize(spec("Rust"), &chars, LineState::BlockComment(2));
        assert_eq!(state, LineState::Normal);
        assert_eq!(
            spans[0],
            Span {
                start: 0,
                end: 11,
                kind: TokenKind::Comment
            }
        );
        assert_eq!(spans[1].kind, TokenKind::Keyword);
    }

    #[test]
    fn python_hash_comments() {
        let spans = spans_of("Python", "def foo():  # note");
        assert_eq!(
            kinds(&spans),
            vec![TokenKind::Keyword, TokenKind::Function, TokenKind::Comment]
        );
    }

    #[test]
    fn highlighter_caches_and_invalidates() {
        let dir = std::env::temp_dir();
        let path = dir.join("kanso-syntax-test.rs");
        let mut buffer = Buffer::from_path(&path).unwrap();
        buffer.insert_text("/* a\nfn x() {}\n*/\nfn y() {}");
        buffer.take_dirty_from();
        let mut hl = Highlighter::new(Some(&path));
        assert_eq!(kinds(&hl.line_spans(&buffer, 1)), vec![TokenKind::Comment]);
        assert_eq!(kinds(&hl.line_spans(&buffer, 3))[0], TokenKind::Keyword);
        buffer.cursor.pos = crate::buffer::Position::new(0, 0);
        buffer.delete_forward();
        buffer.delete_forward();
        hl.invalidate_from(buffer.take_dirty_from().unwrap());
        let spans = hl.line_spans(&buffer, 1);
        assert_eq!(spans[0].kind, TokenKind::Keyword);
    }
}
