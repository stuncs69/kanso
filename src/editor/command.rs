use std::collections::HashMap;

use crate::buffer::Motion;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    InsertChar(char),
    InsertNewline,
    InsertTab,
    Outdent,
    Backspace,
    Delete,
    Move(Motion),
    Select(Motion),
    SelectAll,
    Cancel,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    DuplicateLine,
    MoveLineUp,
    MoveLineDown,
    TriggerCompletion,
    Hover,
    NextDiagnostic,
    PrevDiagnostic,
    LspMenu,
    Help,
    Explorer,
    FindFile,
    Find,
    Replace,
    FindNext,
    FindPrevious,
    ReplaceAll,
    ToggleSearchCase,
    NextBuffer,
    PrevBuffer,
    CloseBuffer,
    Save,
    Quit,
    Plugin(usize),
}

pub struct CommandRegistry {
    commands: HashMap<String, Command>,
}

impl CommandRegistry {
    pub fn with_builtins() -> Self {
        let mut registry = CommandRegistry {
            commands: HashMap::new(),
        };
        for (id, command) in BUILTIN_COMMANDS {
            registry.register(id, *command);
        }
        registry
    }

    pub fn register(&mut self, id: &str, command: Command) {
        self.commands.insert(id.to_string(), command);
    }

    pub fn get(&self, id: &str) -> Option<Command> {
        self.commands.get(id).copied()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.commands.keys().map(String::as_str)
    }
}

const BUILTIN_COMMANDS: &[(&str, Command)] = &[
    ("file.save", Command::Save),
    ("file.open", Command::Explorer),
    ("file.close", Command::CloseBuffer),
    ("view.explorer", Command::Explorer),
    ("view.finder", Command::FindFile),
    ("file.find", Command::FindFile),
    ("search.find", Command::Find),
    ("search.replace", Command::Replace),
    ("search.next", Command::FindNext),
    ("search.previous", Command::FindPrevious),
    ("search.replace_all", Command::ReplaceAll),
    ("search.toggle_case", Command::ToggleSearchCase),
    ("buffer.next", Command::NextBuffer),
    ("buffer.previous", Command::PrevBuffer),
    ("app.quit", Command::Quit),
    ("editor.undo", Command::Undo),
    ("editor.redo", Command::Redo),
    ("editor.cut", Command::Cut),
    ("editor.copy", Command::Copy),
    ("editor.paste", Command::Paste),
    ("editor.select_all", Command::SelectAll),
    ("editor.cancel", Command::Cancel),
    ("editor.insert_newline", Command::InsertNewline),
    ("editor.insert_tab", Command::InsertTab),
    ("editor.indent", Command::InsertTab),
    ("editor.outdent", Command::Outdent),
    ("editor.backspace", Command::Backspace),
    ("editor.delete", Command::Delete),
    ("editor.duplicate_line", Command::DuplicateLine),
    ("editor.move_line_up", Command::MoveLineUp),
    ("editor.move_line_down", Command::MoveLineDown),
    ("editor.trigger_completion", Command::TriggerCompletion),
    ("editor.hover", Command::Hover),
    ("diagnostic.next", Command::NextDiagnostic),
    ("diagnostic.previous", Command::PrevDiagnostic),
    ("view.lsp_status", Command::LspMenu),
    ("view.help", Command::Help),
    ("cursor.left", Command::Move(Motion::Left)),
    ("cursor.right", Command::Move(Motion::Right)),
    ("cursor.up", Command::Move(Motion::Up)),
    ("cursor.down", Command::Move(Motion::Down)),
    ("cursor.word_left", Command::Move(Motion::WordLeft)),
    ("cursor.word_right", Command::Move(Motion::WordRight)),
    ("cursor.line_start", Command::Move(Motion::LineStart)),
    ("cursor.line_end", Command::Move(Motion::LineEnd)),
    ("cursor.page_up", Command::Move(Motion::PageUp)),
    ("cursor.page_down", Command::Move(Motion::PageDown)),
    ("cursor.buffer_start", Command::Move(Motion::BufferStart)),
    ("cursor.buffer_end", Command::Move(Motion::BufferEnd)),
    ("selection.left", Command::Select(Motion::Left)),
    ("selection.right", Command::Select(Motion::Right)),
    ("selection.up", Command::Select(Motion::Up)),
    ("selection.down", Command::Select(Motion::Down)),
    ("selection.word_left", Command::Select(Motion::WordLeft)),
    ("selection.word_right", Command::Select(Motion::WordRight)),
    ("selection.line_start", Command::Select(Motion::LineStart)),
    ("selection.line_end", Command::Select(Motion::LineEnd)),
    ("selection.page_up", Command::Select(Motion::PageUp)),
    ("selection.page_down", Command::Select(Motion::PageDown)),
    (
        "selection.buffer_start",
        Command::Select(Motion::BufferStart),
    ),
    ("selection.buffer_end", Command::Select(Motion::BufferEnd)),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_resolve() {
        let registry = CommandRegistry::with_builtins();
        assert_eq!(registry.get("file.save"), Some(Command::Save));
        assert_eq!(
            registry.get("cursor.left"),
            Some(Command::Move(Motion::Left))
        );
        assert_eq!(registry.get("no.such.command"), None);
    }

    #[test]
    fn registration_overrides() {
        let mut registry = CommandRegistry::with_builtins();
        registry.register("file.save", Command::Quit);
        assert_eq!(registry.get("file.save"), Some(Command::Quit));
    }
}
