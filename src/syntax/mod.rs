mod language;

use std::path::Path;

use crate::buffer::Buffer;

pub use language::{detect, languages, register, LanguageDef, LanguageSpec};

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
    TripleStr(char),
    /// Inside a fenced markdown code block.
    Fence,
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
    if spec.markdown {
        return tokenize_markdown(chars, start_state);
    }
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
        LineState::TripleStr(delim) => match scan_triple_string(chars, 0, delim) {
            Some(end) => {
                push_span(&mut spans, 0, end, TokenKind::String);
                i = end;
            }
            None => {
                push_span(&mut spans, 0, len, TokenKind::String);
                return (spans, LineState::TripleStr(delim));
            }
        },
        LineState::Normal | LineState::Fence => {}
    }

    while i < len {
        let c = chars[i];

        // Block comments first: a line marker can be a prefix of the block
        // opener, as in Lua's `--` and `--[[`.
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

        if let Some(marker) = spec.line_comment {
            if starts_with(chars, i, marker) {
                push_span(&mut spans, i, len, TokenKind::Comment);
                return (spans, LineState::Normal);
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

        if spec.triple_quote_delims.contains(&c)
            && chars.get(i + 1) == Some(&c)
            && chars.get(i + 2) == Some(&c)
        {
            match scan_triple_string(chars, i + 3, c) {
                Some(end) => {
                    push_span(&mut spans, i, end, TokenKind::String);
                    i = end;
                }
                None => {
                    push_span(&mut spans, i, len, TokenKind::String);
                    return (spans, LineState::TripleStr(c));
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
            let word = if spec.case_insensitive_keywords {
                word.to_ascii_lowercase()
            } else {
                word
            };
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

/// Markdown has no keyword model, so it gets its own pass. Reuses the six
/// existing token kinds: heading as Type, code as String, quote as Comment,
/// list marker as Keyword, link target as Function.
// ponytail: emphasis (`*`, `_`) is skipped -- too many false positives in prose.
fn tokenize_markdown(chars: &[char], start_state: LineState) -> (Vec<Span>, LineState) {
    let len = chars.len();
    let mut spans = Vec::new();
    let indent = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(len);
    let in_fence = start_state == LineState::Fence;

    if starts_with(chars, indent, "```") || starts_with(chars, indent, "~~~") {
        push_span(&mut spans, indent, len, TokenKind::String);
        let next = if in_fence {
            LineState::Normal
        } else {
            LineState::Fence
        };
        return (spans, next);
    }
    if in_fence {
        push_span(&mut spans, indent, len, TokenKind::String);
        return (spans, LineState::Fence);
    }
    match chars.get(indent) {
        Some('#') => {
            push_span(&mut spans, indent, len, TokenKind::Type);
            return (spans, LineState::Normal);
        }
        Some('>') => {
            push_span(&mut spans, indent, len, TokenKind::Comment);
            return (spans, LineState::Normal);
        }
        _ => {}
    }

    let mut i = indent;
    if let Some(end) = list_marker(chars, indent) {
        push_span(&mut spans, indent, end, TokenKind::Keyword);
        i = end;
    }
    while i < len {
        if chars[i] == '`' {
            match scan_inline_code(chars, i) {
                Some(end) => {
                    push_span(&mut spans, i, end, TokenKind::String);
                    i = end;
                }
                None => i += 1,
            }
            continue;
        }
        if chars[i] == '(' && i > indent && chars[i - 1] == ']' {
            match chars[i..].iter().position(|c| *c == ')') {
                Some(offset) => {
                    push_span(&mut spans, i, i + offset + 1, TokenKind::Function);
                    i += offset + 1;
                }
                None => i += 1,
            }
            continue;
        }
        i += 1;
    }
    (spans, LineState::Normal)
}

/// End of a `-`/`*`/`+`/`1.` list marker, marker and its space included.
fn list_marker(chars: &[char], at: usize) -> Option<usize> {
    match *chars.get(at)? {
        '-' | '*' | '+' if chars.get(at + 1) == Some(&' ') => Some(at + 1),
        c if c.is_ascii_digit() => {
            let mut i = at;
            while chars.get(i).is_some_and(char::is_ascii_digit) {
                i += 1;
            }
            let terminated = matches!(chars.get(i), Some('.') | Some(')'));
            if terminated && chars.get(i + 1) == Some(&' ') {
                Some(i + 1)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// End of an inline code span, matching the opening backtick run length.
fn scan_inline_code(chars: &[char], at: usize) -> Option<usize> {
    let ticks = backtick_run(chars, at);
    let mut i = at + ticks;
    while i < chars.len() {
        if chars[i] == '`' {
            let run = backtick_run(chars, i);
            if run == ticks {
                return Some(i + run);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

fn backtick_run(chars: &[char], at: usize) -> usize {
    chars[at..].iter().take_while(|c| **c == '`').count()
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

fn scan_triple_string(chars: &[char], from: usize, delim: char) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
        } else if chars[i] == delim
            && chars.get(i + 1) == Some(&delim)
            && chars.get(i + 2) == Some(&delim)
        {
            return Some(i + 3);
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
        languages().into_iter().find(|s| s.name == name).unwrap()
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
    fn detects_language_by_filename() {
        assert_eq!(detect(Some(Path::new("Gemfile"))).unwrap().name, "Ruby");
        assert_eq!(detect(Some(Path::new("a/rakefile"))).unwrap().name, "Ruby");
        assert_eq!(detect(Some(Path::new(".zshrc"))).unwrap().name, "Shell");
        assert_eq!(detect(Some(Path::new("main.rs"))).unwrap().name, "Rust");
        assert!(detect(Some(Path::new("Gemfile.notes"))).is_none());
    }

    #[test]
    fn lua_block_comment_beats_line_comment() {
        let chars: Vec<char> = "local x -- plain".chars().collect();
        let (spans, state) = tokenize(spec("Lua"), &chars, LineState::Normal);
        assert_eq!(state, LineState::Normal);
        assert_eq!(kinds(&spans), vec![TokenKind::Keyword, TokenKind::Comment]);

        let chars: Vec<char> = "local x --[[ open".chars().collect();
        let (spans, state) = tokenize(spec("Lua"), &chars, LineState::Normal);
        assert_eq!(state, LineState::BlockComment(1));
        assert_eq!(spans.last().unwrap().kind, TokenKind::Comment);
        let chars: Vec<char> = "still ]] local".chars().collect();
        let (spans, state) = tokenize(spec("Lua"), &chars, LineState::BlockComment(1));
        assert_eq!(state, LineState::Normal);
        assert_eq!(spans[0].end, 8);
        assert_eq!(spans[1].kind, TokenKind::Keyword);
    }

    #[test]
    fn python_triple_quoted_strings() {
        let spans = spans_of("Python", "x = \"\"\"one line\"\"\"");
        assert_eq!(
            spans,
            vec![Span {
                start: 4,
                end: 18,
                kind: TokenKind::String
            }]
        );

        let chars: Vec<char> = "\"\"\"module docs".chars().collect();
        let (spans, state) = tokenize(spec("Python"), &chars, LineState::Normal);
        assert_eq!(state, LineState::TripleStr('"'));
        assert_eq!(spans[0].kind, TokenKind::String);
        let chars: Vec<char> = "still docs".chars().collect();
        let (_, state) = tokenize(spec("Python"), &chars, LineState::TripleStr('"'));
        assert_eq!(state, LineState::TripleStr('"'));
        let chars: Vec<char> = "\"\"\" def x():".chars().collect();
        let (spans, state) = tokenize(spec("Python"), &chars, LineState::TripleStr('"'));
        assert_eq!(state, LineState::Normal);
        assert_eq!(spans[0].end, 3);
        assert_eq!(
            kinds(&spans[1..]),
            vec![TokenKind::Keyword, TokenKind::Function]
        );
    }

    #[test]
    fn sql_keywords_ignore_case() {
        let upper = spans_of("SQL", "SELECT id FROM t");
        let lower = spans_of("SQL", "select id from t");
        assert_eq!(kinds(&upper), vec![TokenKind::Keyword, TokenKind::Keyword]);
        assert_eq!(kinds(&upper), kinds(&lower));
        assert_eq!(
            kinds(&spans_of("SQL", "x INT")),
            vec![TokenKind::Type],
            "types are matched case-insensitively too"
        );
        assert_eq!(
            kinds(&spans_of("Rust", "SELECT")),
            vec![TokenKind::Type],
            "other languages stay case-sensitive"
        );
    }

    #[test]
    fn markdown_headings_fences_and_inline_code() {
        assert_eq!(
            spans_of("Markdown", "## Heading"),
            vec![Span {
                start: 0,
                end: 10,
                kind: TokenKind::Type
            }]
        );
        assert_eq!(
            kinds(&spans_of("Markdown", "- an `item` here")),
            vec![TokenKind::Keyword, TokenKind::String]
        );
        assert_eq!(
            kinds(&spans_of("Markdown", "see [docs](https://x.dev) now")),
            vec![TokenKind::Function]
        );
        assert_eq!(
            kinds(&spans_of("Markdown", "> quoted")),
            vec![TokenKind::Comment]
        );
        assert!(spans_of("Markdown", "plain prose with * stars").is_empty());

        let chars: Vec<char> = "```rust".chars().collect();
        let (_, state) = tokenize(spec("Markdown"), &chars, LineState::Normal);
        assert_eq!(state, LineState::Fence);
        let chars: Vec<char> = "# not a heading".chars().collect();
        let (spans, state) = tokenize(spec("Markdown"), &chars, LineState::Fence);
        assert_eq!(state, LineState::Fence);
        assert_eq!(kinds(&spans), vec![TokenKind::String]);
        let chars: Vec<char> = "```".chars().collect();
        let (_, state) = tokenize(spec("Markdown"), &chars, LineState::Fence);
        assert_eq!(state, LineState::Normal);
    }

    #[test]
    fn extensions_and_filenames_are_unique_across_languages() {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for spec in languages() {
            for key in spec.extensions.iter().chain(spec.filenames) {
                assert!(
                    !seen.iter().any(|(k, _)| k == key),
                    "`{key}` is claimed by both {:?} and {}",
                    seen.iter().find(|(k, _)| k == key).map(|(_, n)| n),
                    spec.name
                );
                assert_eq!(*key, key.to_ascii_lowercase(), "{} in {}", key, spec.name);
                seen.push((key, spec.name));
            }
        }
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
