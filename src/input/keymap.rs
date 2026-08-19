use std::collections::HashMap;

use super::key::{KeyParseError, KeyPress};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyLookup {
    Command(String),
    Pending,
    Unmapped(KeyPress),
    NoMatch,
}

pub struct Keymap {
    bindings: HashMap<Vec<KeyPress>, String>,
    pending: Vec<KeyPress>,
}

impl Keymap {
    pub fn empty() -> Self {
        Keymap {
            bindings: HashMap::new(),
            pending: Vec::new(),
        }
    }

    pub fn with_defaults() -> Self {
        let mut keymap = Keymap::empty();
        for (keys, command) in DEFAULT_BINDINGS {
            keymap
                .bind_str(keys, command)
                .expect("default keybindings must parse");
        }
        keymap
    }

    pub fn bind(&mut self, sequence: Vec<KeyPress>, command: String) {
        self.bindings.insert(sequence, command);
    }

    pub fn bind_str(&mut self, keys: &str, command: &str) -> Result<(), KeyParseError> {
        self.bind(KeyPress::parse_sequence(keys)?, command.to_string());
        Ok(())
    }

    pub fn bindings(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = self
            .bindings
            .iter()
            .map(|(sequence, command)| {
                let keys = sequence
                    .iter()
                    .map(KeyPress::display)
                    .collect::<Vec<_>>()
                    .join(" ");
                (keys, command.clone())
            })
            .collect();
        rows.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        rows
    }

    pub fn press(&mut self, key: KeyPress) -> KeyLookup {
        let mut candidate = std::mem::take(&mut self.pending);
        candidate.push(key);
        if let Some(command) = self.bindings.get(&candidate) {
            return KeyLookup::Command(command.clone());
        }
        let is_prefix = self
            .bindings
            .keys()
            .any(|seq| seq.len() > candidate.len() && seq.starts_with(&candidate));
        if is_prefix {
            self.pending = candidate;
            return KeyLookup::Pending;
        }
        if candidate.len() > 1 {
            KeyLookup::NoMatch
        } else {
            KeyLookup::Unmapped(key)
        }
    }
}

const DEFAULT_BINDINGS: &[(&str, &str)] = &[
    ("ctrl+s", "file.save"),
    ("ctrl+o", "file.open"),
    ("ctrl+e", "view.explorer"),
    ("ctrl+p", "view.finder"),
    ("ctrl+f", "search.find"),
    ("ctrl+h", "search.replace"),
    ("f3", "search.next"),
    ("shift+f3", "search.previous"),
    ("alt+r", "search.replace_all"),
    ("ctrl+alt+enter", "search.replace_all"),
    ("alt+c", "search.toggle_case"),
    ("ctrl+w", "file.close"),
    ("ctrl+q", "app.quit"),
    ("ctrl+tab", "buffer.next"),
    ("ctrl+pagedown", "buffer.next"),
    ("ctrl+pageup", "buffer.previous"),
    ("ctrl+z", "editor.undo"),
    ("ctrl+y", "editor.redo"),
    ("ctrl+shift+z", "editor.redo"),
    ("ctrl+a", "editor.select_all"),
    ("ctrl+c", "editor.copy"),
    ("ctrl+x", "editor.cut"),
    ("ctrl+v", "editor.paste"),
    ("alt+up", "editor.move_line_up"),
    ("alt+down", "editor.move_line_down"),
    ("shift+alt+up", "editor.duplicate_line"),
    ("shift+alt+down", "editor.duplicate_line"),
    ("enter", "editor.insert_newline"),
    ("tab", "editor.insert_tab"),
    ("shift+tab", "editor.outdent"),
    ("backspace", "editor.backspace"),
    ("delete", "editor.delete"),
    ("esc", "editor.cancel"),
    ("ctrl+space", "editor.trigger_completion"),
    ("alt+h", "editor.hover"),
    ("ctrl+k ctrl+i", "editor.hover"),
    ("f8", "diagnostic.next"),
    ("shift+f8", "diagnostic.previous"),
    ("alt+l", "view.lsp_status"),
    ("ctrl+k ctrl+l", "view.lsp_status"),
    ("f1", "view.help"),
    ("ctrl+k ctrl+s", "view.help"),
    ("left", "cursor.left"),
    ("right", "cursor.right"),
    ("up", "cursor.up"),
    ("down", "cursor.down"),
    ("ctrl+left", "cursor.word_left"),
    ("ctrl+right", "cursor.word_right"),
    ("home", "cursor.line_start"),
    ("end", "cursor.line_end"),
    ("ctrl+home", "cursor.buffer_start"),
    ("ctrl+end", "cursor.buffer_end"),
    ("pageup", "cursor.page_up"),
    ("pagedown", "cursor.page_down"),
    ("shift+left", "selection.left"),
    ("shift+right", "selection.right"),
    ("shift+up", "selection.up"),
    ("shift+down", "selection.down"),
    ("ctrl+shift+left", "selection.word_left"),
    ("ctrl+shift+right", "selection.word_right"),
    ("shift+home", "selection.line_start"),
    ("shift+end", "selection.line_end"),
    ("ctrl+shift+home", "selection.buffer_start"),
    ("ctrl+shift+end", "selection.buffer_end"),
    ("shift+pageup", "selection.page_up"),
    ("shift+pagedown", "selection.page_down"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn key(spec: &str) -> KeyPress {
        KeyPress::parse(spec).unwrap()
    }

    #[test]
    fn single_key_binding_matches() {
        let mut keymap = Keymap::with_defaults();
        assert_eq!(
            keymap.press(key("ctrl+s")),
            KeyLookup::Command("file.save".to_string())
        );
    }

    #[test]
    fn unmapped_key_falls_through() {
        let mut keymap = Keymap::with_defaults();
        assert_eq!(keymap.press(key("x")), KeyLookup::Unmapped(key("x")));
    }

    #[test]
    fn chords_go_pending_then_match() {
        let mut keymap = Keymap::with_defaults();
        keymap.bind_str("ctrl+k ctrl+c", "editor.comment").unwrap();
        assert_eq!(keymap.press(key("ctrl+k")), KeyLookup::Pending);
        assert_eq!(
            keymap.press(key("ctrl+c")),
            KeyLookup::Command("editor.comment".to_string())
        );
        assert_eq!(
            keymap.press(key("ctrl+c")),
            KeyLookup::Command("editor.copy".to_string())
        );
    }

    #[test]
    fn broken_chord_reports_no_match() {
        let mut keymap = Keymap::with_defaults();
        keymap.bind_str("ctrl+k ctrl+c", "editor.comment").unwrap();
        assert_eq!(keymap.press(key("ctrl+k")), KeyLookup::Pending);
        assert_eq!(keymap.press(key("z")), KeyLookup::NoMatch);
        assert_eq!(keymap.press(key("z")), KeyLookup::Unmapped(key("z")));
    }

    #[test]
    fn bindings_list_formats_keys_and_chords() {
        let keymap = Keymap::with_defaults();
        let rows = keymap.bindings();
        assert!(rows.contains(&("ctrl+s".to_string(), "file.save".to_string())));
        assert!(rows.contains(&("ctrl+k ctrl+i".to_string(), "editor.hover".to_string())));
        assert!(rows.contains(&("f1".to_string(), "view.help".to_string())));
        let commands: Vec<&String> = rows.iter().map(|(_, c)| c).collect();
        let mut sorted = commands.clone();
        sorted.sort();
        assert_eq!(commands, sorted);
    }

    #[test]
    fn user_bindings_override_defaults() {
        let mut keymap = Keymap::with_defaults();
        keymap.bind_str("ctrl+s", "editor.undo").unwrap();
        assert_eq!(
            keymap.press(key("ctrl+s")),
            KeyLookup::Command("editor.undo".to_string())
        );
    }
}
