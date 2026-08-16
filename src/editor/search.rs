use crate::buffer::{Buffer, Position};

use super::prompt::Prompt;

const MAX_MATCHES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

impl Match {
    pub fn start_position(&self) -> Position {
        Position::new(self.line, self.start)
    }

    pub fn end_position(&self) -> Position {
        Position::new(self.line, self.end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Query,
    Replacement,
}

pub struct Search {
    pub query: Prompt,
    pub replacement: Prompt,
    pub field: Field,
    pub replacing: bool,
    pub case_sensitive: bool,
    pub matches: Vec<Match>,
    pub current: Option<usize>,
    pub truncated: bool,
    pub origin: Position,
}

impl Search {
    pub fn new(seed: &str, case_sensitive: bool, replacing: bool, origin: Position) -> Self {
        Search {
            query: Prompt::from_text(seed),
            replacement: Prompt::default(),
            field: Field::Query,
            replacing,
            case_sensitive,
            matches: Vec::new(),
            current: None,
            truncated: false,
            origin,
        }
    }

    pub fn active_prompt(&mut self) -> &mut Prompt {
        match self.field {
            Field::Query => &mut self.query,
            Field::Replacement => &mut self.replacement,
        }
    }

    pub fn refresh(&mut self, buffer: &Buffer) {
        let query = self.query.value();
        let found = find_matches(buffer, &query, self.case_sensitive);
        self.truncated = found.len() >= MAX_MATCHES;
        self.matches = found;
        self.current = self.index_at_or_after(self.origin);
    }

    fn index_at_or_after(&self, pos: Position) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let at = self
            .matches
            .iter()
            .position(|m| m.start_position() >= pos)
            .unwrap_or(0);
        Some(at)
    }

    pub fn step(&mut self, delta: isize) {
        if self.matches.is_empty() {
            self.current = None;
            return;
        }
        let len = self.matches.len() as isize;
        let base = self.current.map(|c| c as isize).unwrap_or(0);
        let next = (base + delta).rem_euclid(len) as usize;
        self.current = Some(next);
        self.origin = self.matches[next].start_position();
    }

    pub fn current_match(&self) -> Option<Match> {
        self.current.and_then(|i| self.matches.get(i).copied())
    }

    pub fn counter(&self) -> String {
        if self.query.is_empty() {
            return String::new();
        }
        if self.matches.is_empty() {
            return "no results".to_string();
        }
        let position = self.current.map(|i| i + 1).unwrap_or(0);
        let total = self.matches.len();
        if self.truncated {
            format!("{position}/{total}+")
        } else {
            format!("{position}/{total}")
        }
    }

    pub fn ranges(&self) -> Vec<(Position, Position)> {
        self.matches
            .iter()
            .map(|m| (m.start_position(), m.end_position()))
            .collect()
    }
}

pub fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

pub fn find_matches(buffer: &Buffer, query: &str, case_sensitive: bool) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle: Vec<char> = if case_sensitive {
        query.chars().collect()
    } else {
        query.chars().map(fold).collect()
    };
    let mut matches = Vec::new();
    for line in 0..buffer.len_lines() {
        let hay: Vec<char> = if case_sensitive {
            buffer.line_chars(line).collect()
        } else {
            buffer.line_chars(line).map(fold).collect()
        };
        if hay.len() < needle.len() {
            continue;
        }
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            if hay[i..i + needle.len()] == needle[..] {
                matches.push(Match {
                    line,
                    start: i,
                    end: i + needle.len(),
                });
                i += needle.len();
            } else {
                i += 1;
            }
        }
        if matches.len() >= MAX_MATCHES {
            matches.truncate(MAX_MATCHES);
            break;
        }
    }
    matches
}

pub fn matches_on_line(matches: &[Match], line: usize) -> &[Match] {
    let start = matches.partition_point(|m| m.line < line);
    let end = matches.partition_point(|m| m.line <= line);
    &matches[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_with(text: &str) -> Buffer {
        let mut buffer = Buffer::scratch();
        buffer.insert_text(text);
        buffer
    }

    #[test]
    fn finds_case_insensitive_matches_in_order() {
        let buffer = buffer_with("Foo foo\nbar\nFOO");
        let matches = find_matches(&buffer, "foo", false);
        assert_eq!(matches.len(), 3);
        assert_eq!(
            matches[0],
            Match {
                line: 0,
                start: 0,
                end: 3
            }
        );
        assert_eq!(matches[2].line, 2);
    }

    #[test]
    fn case_sensitive_search_narrows_results() {
        let buffer = buffer_with("Foo foo\nFOO");
        let matches = find_matches(&buffer, "foo", true);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].start, 4);
    }

    #[test]
    fn overlapping_matches_are_not_reported_twice() {
        let buffer = buffer_with("aaaa");
        let matches = find_matches(&buffer, "aa", false);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[1].start, 2);
    }

    #[test]
    fn empty_query_finds_nothing() {
        let buffer = buffer_with("anything");
        assert!(find_matches(&buffer, "", false).is_empty());
    }

    #[test]
    fn refresh_selects_first_match_at_or_after_the_origin() {
        let buffer = buffer_with("x\nfoo\nfoo");
        let mut search = Search::new("foo", false, false, Position::new(2, 0));
        search.refresh(&buffer);
        assert_eq!(search.current, Some(1));
        search.origin = Position::new(0, 0);
        search.refresh(&buffer);
        assert_eq!(search.current, Some(0));
        search.origin = Position::new(9, 0);
        search.refresh(&buffer);
        assert_eq!(search.current, Some(0));
    }

    #[test]
    fn stepping_wraps_and_carries_the_origin() {
        let buffer = buffer_with("foo foo foo");
        let mut search = Search::new("foo", false, false, Position::new(0, 0));
        search.refresh(&buffer);
        assert_eq!(search.counter(), "1/3");
        search.step(1);
        search.step(1);
        assert_eq!(search.current, Some(2));
        assert_eq!(search.origin, Position::new(0, 8));
        search.step(1);
        assert_eq!(search.current, Some(0));
        search.step(-1);
        assert_eq!(search.current, Some(2));
        search.refresh(&buffer);
        assert_eq!(search.current, Some(2));
    }

    #[test]
    fn counter_reports_missing_results() {
        let buffer = buffer_with("abc");
        let mut search = Search::new("zzz", false, false, Position::new(0, 0));
        search.refresh(&buffer);
        assert_eq!(search.counter(), "no results");
        assert!(search.current_match().is_none());
    }

    #[test]
    fn line_slice_selects_only_that_line() {
        let buffer = buffer_with("foo\nfoo foo\nbar");
        let matches = find_matches(&buffer, "foo", false);
        assert_eq!(matches_on_line(&matches, 0).len(), 1);
        assert_eq!(matches_on_line(&matches, 1).len(), 2);
        assert!(matches_on_line(&matches, 2).is_empty());
    }
}
