use crate::buffer::Motion;

#[derive(Debug, Default, Clone)]
pub struct Prompt {
    chars: Vec<char>,
    cursor: usize,
}

impl Prompt {
    pub fn from_text(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let cursor = chars.len();
        Prompt { chars, cursor }
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn insert(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    pub fn insert_text(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            self.insert(c);
        }
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.chars.remove(self.cursor);
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.chars.len() {
            return false;
        }
        self.chars.remove(self.cursor);
        true
    }

    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
    }

    pub fn move_cursor(&mut self, motion: Motion) -> bool {
        let target = match motion {
            Motion::Left => self.cursor.saturating_sub(1),
            Motion::Right => (self.cursor + 1).min(self.chars.len()),
            Motion::WordLeft => self.word_left(),
            Motion::WordRight => self.word_right(),
            Motion::LineStart | Motion::BufferStart => 0,
            Motion::LineEnd | Motion::BufferEnd => self.chars.len(),
            _ => return false,
        };
        self.cursor = target;
        true
    }

    fn word_left(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let mut i = self.cursor;
        while i < self.chars.len() && self.chars[i].is_whitespace() {
            i += 1;
        }
        while i < self.chars.len() && !self.chars[i].is_whitespace() {
            i += 1;
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_deleting_track_the_cursor() {
        let mut prompt = Prompt::default();
        prompt.insert_text("abc");
        assert_eq!(prompt.value(), "abc");
        assert_eq!(prompt.cursor(), 3);
        prompt.move_cursor(Motion::Left);
        prompt.insert('X');
        assert_eq!(prompt.value(), "abXc");
        assert!(prompt.backspace());
        assert_eq!(prompt.value(), "abc");
        assert!(prompt.delete());
        assert_eq!(prompt.value(), "ab");
        assert!(!prompt.delete());
    }

    #[test]
    fn motions_clamp_and_skip_words() {
        let mut prompt = Prompt::from_text("one two");
        assert!(prompt.move_cursor(Motion::WordLeft));
        assert_eq!(prompt.cursor(), 4);
        prompt.move_cursor(Motion::WordLeft);
        assert_eq!(prompt.cursor(), 0);
        prompt.move_cursor(Motion::Left);
        assert_eq!(prompt.cursor(), 0);
        prompt.move_cursor(Motion::WordRight);
        assert_eq!(prompt.cursor(), 3);
        prompt.move_cursor(Motion::LineEnd);
        assert_eq!(prompt.cursor(), 7);
        prompt.move_cursor(Motion::Right);
        assert_eq!(prompt.cursor(), 7);
        assert!(!prompt.move_cursor(Motion::Up));
    }

    #[test]
    fn control_characters_are_not_inserted() {
        let mut prompt = Prompt::default();
        prompt.insert_text("a\nb\tc");
        assert_eq!(prompt.value(), "abc");
    }
}
