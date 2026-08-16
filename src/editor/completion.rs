use std::collections::BTreeSet;

use crate::buffer::Buffer;
use crate::syntax::LanguageSpec;

const MAX_ITEMS: usize = 50;
const MAX_SCAN_CHARS: usize = 200_000;

pub struct Completion {
    pub items: Vec<String>,
    pub selected: usize,
    pub prefix_start: usize,
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn prefix_at_cursor(buffer: &Buffer) -> (usize, String) {
    let pos = buffer.cursor.pos;
    let chars: Vec<char> = buffer.line_chars(pos.line).take(pos.col).collect();
    let mut start = chars.len();
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    (start, chars[start..].iter().collect())
}

pub fn collect_candidates(
    buffer: &Buffer,
    spec: Option<&LanguageSpec>,
    prefix: &str,
) -> Vec<String> {
    let mut set = BTreeSet::new();
    let mut scanned = 0usize;
    for line in 0..buffer.len_lines() {
        let mut word = String::new();
        for ch in buffer.line_chars(line) {
            scanned += 1;
            if is_word_char(ch) {
                word.push(ch);
            } else {
                take_word(&mut word, prefix, &mut set);
            }
        }
        take_word(&mut word, prefix, &mut set);
        if scanned > MAX_SCAN_CHARS {
            break;
        }
    }
    if let Some(spec) = spec {
        for word in spec.keywords.iter().chain(spec.types) {
            if word.starts_with(prefix) {
                set.insert((*word).to_string());
            }
        }
    }
    set.remove(prefix);
    set.into_iter().take(MAX_ITEMS).collect()
}

fn take_word(word: &mut String, prefix: &str, set: &mut BTreeSet<String>) {
    let keep = word.chars().count() > 1
        && !word.starts_with(|c: char| c.is_ascii_digit())
        && word.starts_with(prefix);
    if keep {
        set.insert(std::mem::take(word));
    } else {
        word.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Position;
    use crate::syntax::detect;
    use std::path::Path;

    fn buffer_with(text: &str) -> Buffer {
        let mut buffer = Buffer::scratch();
        buffer.insert_text(text);
        buffer
    }

    #[test]
    fn prefix_scans_back_to_word_start() {
        let mut buffer = buffer_with("foo ba");
        assert_eq!(prefix_at_cursor(&buffer), (4, "ba".to_string()));
        buffer.cursor.pos = Position::new(0, 4);
        assert_eq!(prefix_at_cursor(&buffer), (4, String::new()));
        buffer.cursor.pos = Position::new(0, 2);
        assert_eq!(prefix_at_cursor(&buffer), (0, "fo".to_string()));
    }

    #[test]
    fn candidates_come_from_buffer_words() {
        let buffer = buffer_with("hello helper\nheap hel");
        let items = collect_candidates(&buffer, None, "hel");
        assert_eq!(items, vec!["hello".to_string(), "helper".to_string()]);
    }

    #[test]
    fn candidates_include_language_keywords() {
        let buffer = buffer_with("let whale = 1;");
        let spec = detect(Some(Path::new("x.rs")));
        let items = collect_candidates(&buffer, spec, "wh");
        assert_eq!(
            items,
            vec![
                "whale".to_string(),
                "where".to_string(),
                "while".to_string()
            ]
        );
    }

    #[test]
    fn empty_prefix_lists_all_words() {
        let buffer = buffer_with("alpha beta a 9x");
        let items = collect_candidates(&buffer, None, "");
        assert_eq!(items, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn exact_prefix_word_is_excluded() {
        let buffer = buffer_with("hel hel hel");
        assert!(collect_candidates(&buffer, None, "hel").is_empty());
    }
}
