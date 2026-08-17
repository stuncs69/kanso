mod api;
mod events;
mod runtime;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use mlua::{Function, Lua, Result as LuaResult, Table};

use crate::editor::Editor;

use api::{Action, Shared};
pub use events::Event;

pub struct PluginHost {
    lua: Lua,
    shared: Rc<RefCell<Shared>>,
}

impl PluginHost {
    pub fn new<I, S>(existing_commands: I) -> LuaResult<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let shared = Rc::new(RefCell::new(Shared {
            known_commands: existing_commands.into_iter().map(Into::into).collect(),
            ..Shared::default()
        }));
        let lua = Lua::new();
        api::install(&lua, &shared)?;
        Ok(PluginHost { lua, shared })
    }

    pub fn load_dir(&mut self, dir: &Path) -> Vec<String> {
        let (plugins, mut warnings) = runtime::discover(dir);
        for plugin in plugins {
            let source = match std::fs::read_to_string(&plugin.entry) {
                Ok(source) => source,
                Err(e) => {
                    warnings.push(format!("plugin `{}`: {e}", plugin.name));
                    continue;
                }
            };
            let chunk = self
                .lua
                .load(source)
                .set_name(format!("@{}", plugin.entry.display()));
            if let Err(e) = chunk.exec() {
                warnings.push(format!("plugin `{}`: {}", plugin.name, first_line(&e)));
            }
        }
        warnings
    }

    pub fn command_names(&self) -> Vec<String> {
        self.shared.borrow().commands.clone()
    }

    pub fn take_bindings(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.shared.borrow_mut().bindings)
    }

    pub fn watches(&self, event: &str) -> bool {
        self.handlers(event).is_some_and(|list| list.raw_len() > 0)
    }

    pub fn run_command(&self, handle: usize, editor: &mut Editor) {
        let name = self
            .shared
            .borrow()
            .commands
            .get(handle)
            .cloned()
            .unwrap_or_default();
        self.snapshot(editor);
        let result = self.callback(handle).and_then(|f| f.call::<()>(()));
        if let Err(e) = result {
            editor.set_error(format!("plugin command `{name}`: {}", first_line(&e)));
        }
        self.apply(editor);
    }

    pub fn dispatch(&self, event: &Event, editor: &mut Editor) {
        let Some(handlers) = self.handlers(event.name()) else {
            return;
        };
        self.snapshot(editor);
        let mut errors = Vec::new();
        match event.to_table(&self.lua) {
            Ok(payload) => {
                for handler in handlers.sequence_values::<Function>() {
                    let result = handler.and_then(|f| f.call::<()>(&payload));
                    if let Err(e) = result {
                        errors.push(format!("plugin {}: {}", event.name(), first_line(&e)));
                    }
                }
            }
            Err(e) => errors.push(format!("plugin {}: {}", event.name(), first_line(&e))),
        }
        self.apply(editor);
        if let Some(first) = errors.into_iter().next() {
            editor.set_error(first);
        }
    }

    fn callback(&self, handle: usize) -> LuaResult<Function> {
        let callbacks: Table = self.lua.named_registry_value(api::COMMANDS_KEY)?;
        callbacks.get(handle + 1)
    }

    fn handlers(&self, event: &str) -> Option<Table> {
        let handlers: Table = self.lua.named_registry_value(api::HANDLERS_KEY).ok()?;
        handlers.get(event).ok().flatten()
    }

    fn snapshot(&self, editor: &Editor) {
        let buffer = &editor.buffer;
        let mut shared = self.shared.borrow_mut();
        let info = &mut shared.buffer;
        if info.id != buffer.id() || info.revision != buffer.revision() {
            info.text = buffer.text();
        }
        info.id = buffer.id();
        info.revision = buffer.revision();
        info.name = buffer.file_name();
        info.path = buffer.path().map(|p| p.display().to_string());
        info.modified = buffer.is_dirty();
    }

    fn apply(&self, editor: &mut Editor) {
        let actions = std::mem::take(&mut self.shared.borrow_mut().actions);
        for action in actions {
            match action {
                Action::Notify(message) => editor.set_info(message),
                Action::Status(message, duration) => editor.set_info_for(message, duration),
            }
        }
    }
}

fn first_line(error: &mlua::Error) -> String {
    let text = error.to_string();
    text.lines().next().unwrap_or(&text).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::config::Settings;
    use crate::editor::MessageKind;

    fn host() -> PluginHost {
        PluginHost::new(["file.save"]).unwrap()
    }

    fn editor() -> Editor {
        let mut editor = Editor::new(Buffer::scratch(), Settings::default());
        editor.buffer.insert_text("hello lua world");
        editor.take_events();
        editor
    }

    fn exec(host: &PluginHost, source: &str) -> LuaResult<()> {
        host.lua.load(source).exec()
    }

    fn status(editor: &Editor) -> String {
        editor
            .status
            .as_ref()
            .map(|s| s.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn command_registration_exposes_a_handle() {
        let host = host();
        exec(
            &host,
            "kanso.commands.register('stats.words', function() end)",
        )
        .unwrap();
        assert_eq!(host.command_names(), ["stats.words"]);
    }

    #[test]
    fn duplicate_command_registration_fails() {
        let host = host();
        exec(&host, "kanso.commands.register('a.b', function() end)").unwrap();
        let err = exec(&host, "kanso.commands.register('a.b', function() end)").unwrap_err();
        assert!(err.to_string().contains("already registered"));
        let err = exec(
            &host,
            "kanso.commands.register('file.save', function() end)",
        )
        .unwrap_err();
        assert!(err.to_string().contains("already registered"));
        assert_eq!(host.command_names(), ["a.b"]);
    }

    #[test]
    fn commands_read_the_buffer_and_notify() {
        let host = host();
        let mut editor = editor();
        exec(
            &host,
            "kanso.commands.register('stats', function()
                 local b = kanso.editor.current_buffer()
                 local words = 0
                 for _ in b:text():gmatch('%S+') do words = words + 1 end
                 kanso.ui.notify(string.format('%s %d %s', b:name(), words, tostring(b:is_modified())))
             end)",
        )
        .unwrap();
        host.run_command(0, &mut editor);
        assert_eq!(status(&editor), "[scratch] 3 true");
    }

    #[test]
    fn command_errors_are_reported_not_fatal() {
        let host = host();
        let mut editor = editor();
        exec(
            &host,
            "kanso.commands.register('boom', function() error('nope') end)",
        )
        .unwrap();
        host.run_command(0, &mut editor);
        assert!(status(&editor).contains("plugin command `boom`"));
        assert_eq!(editor.status.as_ref().unwrap().kind, MessageKind::Error);
    }

    #[test]
    fn events_dispatch_to_every_subscriber_despite_errors() {
        let host = host();
        let mut editor = editor();
        exec(
            &host,
            "kanso.events.subscribe('buffer_saved', function() error('bad plugin') end)
             kanso.events.subscribe('buffer_saved', function(e)
                 kanso.ui.set_status_message('saved ' .. e.buffer_id, 1200)
             end)",
        )
        .unwrap();
        assert!(host.watches("buffer_saved"));
        assert!(!host.watches("buffer_changed"));

        let id = editor.buffer.id();
        host.dispatch(&Event::BufferSaved { buffer_id: id }, &mut editor);
        assert!(status(&editor).contains("bad plugin"));
    }

    #[test]
    fn timed_status_messages_expire() {
        let host = host();
        let mut editor = editor();
        exec(
            &host,
            "kanso.events.subscribe('buffer_saved', function(e)
                 kanso.ui.set_status_message('saved ' .. e.buffer_id, 1200)
             end)",
        )
        .unwrap();
        let id = editor.buffer.id();
        host.dispatch(&Event::BufferSaved { buffer_id: id }, &mut editor);
        assert_eq!(status(&editor), format!("saved {id}"));
        assert!(editor.status.as_ref().unwrap().expires_at.is_some());
        editor.expire_status();
        assert!(editor.status.is_some(), "not expired yet");
    }

    #[test]
    fn unknown_event_and_bad_key_are_errors() {
        let host = host();
        let err = exec(&host, "kanso.events.subscribe('nope', function() end)").unwrap_err();
        assert!(err.to_string().contains("unknown event `nope`"));
        let err = exec(&host, "kanso.keymap.bind('ctrl+nosuchkey', 'a.b')").unwrap_err();
        assert!(err.to_string().contains("invalid key spec"));
    }

    #[test]
    fn keymap_bindings_are_queued_for_the_editor() {
        let mut host = host();
        exec(&host, "kanso.keymap.bind('ctrl+alt+w', 'stats.words')").unwrap();
        assert_eq!(
            host.take_bindings(),
            [("ctrl+alt+w".to_string(), "stats.words".to_string())]
        );
        assert!(host.take_bindings().is_empty());
    }

    #[test]
    fn the_example_plugin_loads_and_runs() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins");
        let mut host = host();
        let warnings = host.load_dir(&dir);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(host.command_names(), ["stats.words"]);
        assert_eq!(
            host.take_bindings(),
            [("alt+w".to_string(), "stats.words".to_string())]
        );

        let mut editor = editor();
        host.run_command(0, &mut editor);
        assert_eq!(status(&editor), "[scratch] — 3 words");

        let id = editor.buffer.id();
        host.dispatch(&Event::BufferSaved { buffer_id: id }, &mut editor);
        assert_eq!(status(&editor), format!("Saved [scratch] (buffer {id})"));
    }

    #[test]
    fn broken_plugin_directory_only_warns() {
        let dir = std::env::temp_dir().join(format!("kanso-badplugin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("broken")).unwrap();
        std::fs::write(dir.join("broken/init.lua"), "this is not lua(((").unwrap();
        std::fs::create_dir_all(dir.join("good")).unwrap();
        std::fs::write(
            dir.join("good/init.lua"),
            "kanso.commands.register('good.cmd', function() end)",
        )
        .unwrap();

        let mut host = host();
        let warnings = host.load_dir(&dir);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].starts_with("plugin `broken`:"));
        assert_eq!(host.command_names(), ["good.cmd"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
