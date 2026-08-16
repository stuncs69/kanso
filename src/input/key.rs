use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Insert,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyPress {
    pub key: Key,
    pub mods: Mods,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid key spec `{0}`")]
pub struct KeyParseError(pub String);

impl KeyPress {
    pub fn from_event(event: &KeyEvent) -> Option<Self> {
        let mut mods = Mods {
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
            shift: event.modifiers.contains(KeyModifiers::SHIFT),
        };
        let key = match event.code {
            KeyCode::Char(c) => {
                if c.is_ascii_uppercase() {
                    mods.shift = true;
                    Key::Char(c.to_ascii_lowercase())
                } else {
                    Key::Char(c)
                }
            }
            KeyCode::Enter => Key::Enter,
            KeyCode::Tab => Key::Tab,
            KeyCode::BackTab => {
                mods.shift = true;
                Key::Tab
            }
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Delete => Key::Delete,
            KeyCode::Insert => Key::Insert,
            KeyCode::Esc => Key::Esc,
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Left => Key::Left,
            KeyCode::Right => Key::Right,
            KeyCode::Home => Key::Home,
            KeyCode::End => Key::End,
            KeyCode::PageUp => Key::PageUp,
            KeyCode::PageDown => Key::PageDown,
            KeyCode::F(n) => Key::F(n),
            _ => return None,
        };
        Some(KeyPress { key, mods })
    }

    pub fn to_text(self) -> Option<char> {
        if self.mods.ctrl || self.mods.alt {
            return None;
        }
        match self.key {
            Key::Char(c) => Some(if self.mods.shift {
                c.to_ascii_uppercase()
            } else {
                c
            }),
            _ => None,
        }
    }

    pub fn parse(spec: &str) -> Result<Self, KeyParseError> {
        let err = || KeyParseError(spec.to_string());
        let parts: Vec<&str> = spec.split('+').collect();
        let (&key_part, mod_parts) = parts.split_last().ok_or_else(err)?;
        let mut mods = Mods::default();
        for part in mod_parts {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "meta" | "option" => mods.alt = true,
                "shift" => mods.shift = true,
                _ => return Err(err()),
            }
        }
        let lowered = key_part.to_ascii_lowercase();
        let key = match lowered.as_str() {
            "enter" | "return" => Key::Enter,
            "tab" => Key::Tab,
            "backspace" => Key::Backspace,
            "delete" | "del" => Key::Delete,
            "insert" => Key::Insert,
            "esc" | "escape" => Key::Esc,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" | "pgup" => Key::PageUp,
            "pagedown" | "pgdn" => Key::PageDown,
            "space" => Key::Char(' '),
            _ => {
                let mut chars = lowered.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Key::Char(c),
                    (Some('f'), Some(_)) => {
                        let n: u8 = lowered[1..].parse().map_err(|_| err())?;
                        if (1..=24).contains(&n) {
                            Key::F(n)
                        } else {
                            return Err(err());
                        }
                    }
                    _ => return Err(err()),
                }
            }
        };
        Ok(KeyPress { key, mods })
    }

    pub fn display(&self) -> String {
        let mut out = String::new();
        if self.mods.ctrl {
            out.push_str("ctrl+");
        }
        if self.mods.shift {
            out.push_str("shift+");
        }
        if self.mods.alt {
            out.push_str("alt+");
        }
        out.push_str(&key_name(self.key));
        out
    }

    pub fn parse_sequence(spec: &str) -> Result<Vec<Self>, KeyParseError> {
        let keys: Vec<KeyPress> = spec
            .split_whitespace()
            .map(KeyPress::parse)
            .collect::<Result<_, _>>()?;
        if keys.is_empty() {
            return Err(KeyParseError(spec.to_string()));
        }
        Ok(keys)
    }
}

fn key_name(key: Key) -> String {
    match key {
        Key::Char(' ') => "space".to_string(),
        Key::Char(c) => c.to_string(),
        Key::Enter => "enter".to_string(),
        Key::Tab => "tab".to_string(),
        Key::Backspace => "backspace".to_string(),
        Key::Delete => "delete".to_string(),
        Key::Insert => "insert".to_string(),
        Key::Esc => "esc".to_string(),
        Key::Up => "up".to_string(),
        Key::Down => "down".to_string(),
        Key::Left => "left".to_string(),
        Key::Right => "right".to_string(),
        Key::Home => "home".to_string(),
        Key::End => "end".to_string(),
        Key::PageUp => "pageup".to_string(),
        Key::PageDown => "pagedown".to_string(),
        Key::F(n) => format!("f{n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        let mut event = KeyEvent::new(code, mods);
        event.kind = KeyEventKind::Press;
        event
    }

    #[test]
    fn parse_modified_keys() {
        let kp = KeyPress::parse("ctrl+shift+p").unwrap();
        assert_eq!(kp.key, Key::Char('p'));
        assert!(kp.mods.ctrl && kp.mods.shift && !kp.mods.alt);

        let kp = KeyPress::parse("alt+up").unwrap();
        assert_eq!(kp.key, Key::Up);
        assert!(kp.mods.alt && !kp.mods.ctrl);

        assert!(KeyPress::parse("hyper+x").is_err());
        assert!(KeyPress::parse("ctrl+nosuchkey").is_err());
    }

    #[test]
    fn parse_function_keys() {
        assert_eq!(KeyPress::parse("f5").unwrap().key, Key::F(5));
        assert!(KeyPress::parse("f99").is_err());
    }

    #[test]
    fn parse_chord_sequence() {
        let seq = KeyPress::parse_sequence("ctrl+k ctrl+c").unwrap();
        assert_eq!(seq.len(), 2);
        assert_eq!(seq[0].key, Key::Char('k'));
        assert_eq!(seq[1].key, Key::Char('c'));
    }

    #[test]
    fn uppercase_events_normalize_to_shift() {
        let kp = KeyPress::from_event(&press(KeyCode::Char('P'), KeyModifiers::SHIFT)).unwrap();
        assert_eq!(kp, KeyPress::parse("shift+p").unwrap());
        assert_eq!(kp.to_text(), Some('P'));
    }

    #[test]
    fn display_round_trips_specs() {
        for spec in [
            "ctrl+shift+p",
            "alt+up",
            "ctrl+space",
            "f5",
            "pagedown",
            "x",
        ] {
            assert_eq!(KeyPress::parse(spec).unwrap().display(), spec);
        }
    }

    #[test]
    fn ctrl_keys_do_not_insert_text() {
        let kp = KeyPress::from_event(&press(KeyCode::Char('s'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(kp.to_text(), None);
        assert_eq!(kp, KeyPress::parse("ctrl+s").unwrap());
    }
}
