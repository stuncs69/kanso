use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde_json::Value;

use crate::buffer::{Buffer, Position};
use crate::config::Settings;
use crate::editor::command::{Command, CommandRegistry};
use crate::editor::{Editor, Menu};
use crate::input::{KeyLookup, KeyPress, Keymap};
use crate::lsp::{self, LspClient, Reply};
use crate::plugins::{Event as PluginEvent, PluginHost};
use crate::syntax;
use crate::ui::renderer::Renderer;
use crate::ui::terminal::Terminal;
use crate::ui::theme::Theme;
use crate::util;

const TICK: Duration = Duration::from_millis(150);
const HOVER_DELAY: Duration = Duration::from_millis(400);

enum AppEvent {
    Term(Event),
    Lsp(&'static str, Value),
}

pub struct App {
    editor: Editor,
    keymap: Keymap,
    registry: CommandRegistry,
    renderer: Renderer,
    terminal: Terminal,
    theme: Theme,
    clients: HashMap<&'static str, LspClient>,
    attempted: HashSet<&'static str>,
    synced: HashMap<String, u64>,
    diagnostics: HashMap<String, Vec<lsp::Diagnostic>>,
    diagnostics_version: u64,
    diagnostics_shown: Option<(String, u64)>,
    completion_request: Option<(&'static str, u64, usize, usize)>,
    hover_request: Option<(&'static str, u64, Position)>,
    mouse_at: Option<(u16, u16, Instant)>,
    hover_sent_at: Option<(u16, u16)>,
    plugins: Option<PluginHost>,
    last_change: (u64, u64),
}

impl App {
    pub fn new(
        mut buffers: Vec<Buffer>,
        settings: Settings,
        theme: Theme,
        user_keybindings: &[(String, String)],
        mut startup_warnings: Vec<String>,
    ) -> io::Result<Self> {
        let mut keymap = Keymap::with_defaults();
        let mut registry = CommandRegistry::with_builtins();
        let plugins = load_plugins(&mut registry, &mut keymap, &mut startup_warnings);
        for (keys, command) in user_keybindings {
            if let Err(e) = keymap.bind_str(keys, command) {
                startup_warnings.push(format!("keybindings.toml: {e}"));
            }
        }

        let terminal = Terminal::new(settings.cursor_style, settings.mouse)?;
        let (width, height) = Terminal::size()?;
        let first = if buffers.is_empty() {
            Buffer::scratch()
        } else {
            buffers.remove(0)
        };
        let mut editor = Editor::new(first, settings);
        for buffer in buffers {
            editor.add_background(buffer);
        }
        editor.viewport.resize(width, height);
        editor.sync_viewport();
        match startup_warnings.first() {
            Some(warning) => editor.set_error(warning.clone()),
            None => editor.set_info("F1 for help"),
        }

        let last_change = (editor.buffer.id(), editor.buffer.revision());
        Ok(App {
            editor,
            keymap,
            registry,
            renderer: Renderer::new(),
            terminal,
            theme,
            clients: HashMap::new(),
            attempted: HashSet::new(),
            synced: HashMap::new(),
            diagnostics: HashMap::new(),
            diagnostics_version: 0,
            diagnostics_shown: None,
            completion_request: None,
            hover_request: None,
            mouse_at: None,
            hover_sent_at: None,
            plugins,
            last_change,
        })
    }

    pub fn run(&mut self) -> io::Result<()> {
        let (tx, rx) = mpsc::channel();
        spawn_input_thread(tx.clone());
        loop {
            self.process_lsp_work(&tx);
            self.process_plugin_work();
            self.editor.expire_status();
            self.renderer
                .draw(&mut self.terminal, &self.editor, &self.theme)?;
            match rx.recv_timeout(TICK) {
                Ok(ev) => {
                    self.dispatch(ev);
                    while !self.editor.should_quit() {
                        match rx.try_recv() {
                            Ok(ev) => self.dispatch(ev),
                            Err(_) => break,
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if self.editor.should_quit() {
                break;
            }
        }
        for (_, mut client) in self.clients.drain() {
            client.shutdown();
        }
        Ok(())
    }

    fn dispatch(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Term(ev) => self.handle_event(ev),
            AppEvent::Lsp(lsp_id, msg) => self.handle_lsp(lsp_id, msg),
        }
    }

    fn handle_event(&mut self, ev: Event) {
        match ev {
            Event::Key(key_event)
                if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
            {
                if let Some(key) = KeyPress::from_event(&key_event) {
                    self.handle_key(key);
                }
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse),
            Event::Resize(width, height) => {
                self.editor.viewport.resize(width, height);
                self.editor.sync_viewport();
            }
            Event::Paste(text) => self.editor.paste_text(&text),
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyPress) {
        match self.keymap.press(key) {
            KeyLookup::Command(id) => match self.registry.get(&id) {
                Some(command) => self.editor.execute(command),
                None => self.editor.set_error(format!("Unknown command: {id}")),
            },
            KeyLookup::Pending => self.editor.set_info("Waiting for chord"),
            KeyLookup::Unmapped(key) => {
                if let Some(c) = key.to_text() {
                    self.editor.execute(Command::InsertChar(c));
                }
            }
            KeyLookup::NoMatch => self.editor.set_error("Unbound key sequence"),
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let extend = mouse.modifiers.contains(KeyModifiers::SHIFT);
                self.editor.mouse_down(mouse.column, mouse.row, extend);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.editor.mouse_drag(mouse.column, mouse.row);
            }
            MouseEventKind::Moved => {
                let cell = (mouse.column, mouse.row);
                if self.mouse_at.map(|(x, y, _)| (x, y)) != Some(cell) {
                    self.mouse_at = Some((cell.0, cell.1, Instant::now()));
                    self.hover_sent_at = None;
                    self.editor.hover = None;
                }
            }
            MouseEventKind::ScrollUp => self.editor.scroll_viewport(-3),
            MouseEventKind::ScrollDown => self.editor.scroll_viewport(3),
            _ => {}
        }
    }

    fn active_language(&self) -> Option<(&'static str, PathBuf)> {
        let path = self.editor.buffer.path()?;
        let spec = syntax::detect(Some(path))?;
        if spec.lsp_id.is_empty() {
            return None;
        }
        Some((spec.lsp_id, path.to_path_buf()))
    }

    fn process_lsp_work(&mut self, tx: &Sender<AppEvent>) {
        self.ensure_active_lsp(tx);
        if self.editor.take_save_notification() {
            self.send_did_save();
        }
        self.publish_diagnostics();
        self.request_completion();
        if let Some(anchor) = self.editor.take_hover_request() {
            self.send_hover(anchor);
        }
        if self.editor.take_menu_request() {
            self.show_lsp_menu();
        }
        if self.editor.take_help_request() {
            self.show_help_menu();
        }
        self.poll_mouse_hover();
    }

    fn process_plugin_work(&mut self) {
        let Some(host) = self.plugins.take() else {
            self.editor.take_events();
            self.editor.take_plugin_calls();
            return;
        };
        let change = (self.editor.buffer.id(), self.editor.buffer.revision());
        if change != self.last_change {
            self.last_change = change;
            if host.watches("buffer_changed") {
                let event = PluginEvent::BufferChanged {
                    buffer_id: change.0,
                    revision: change.1,
                };
                host.dispatch(&event, &mut self.editor);
            }
        }
        for event in self.editor.take_events() {
            host.dispatch(&event, &mut self.editor);
        }
        for handle in self.editor.take_plugin_calls() {
            host.run_command(handle, &mut self.editor);
        }
        self.plugins = Some(host);
    }

    fn ensure_active_lsp(&mut self, tx: &Sender<AppEvent>) {
        if !self.editor.settings.lsp {
            return;
        }
        let Some((lsp_id, path)) = self.active_language() else {
            self.editor.lsp_indicator = None;
            return;
        };
        if !self.clients.contains_key(lsp_id) && self.attempted.insert(lsp_id) {
            let command = self
                .editor
                .settings
                .language_servers
                .get(lsp_id)
                .cloned()
                .or_else(|| lsp::default_server(lsp_id, &path));
            match command {
                Some(command) => {
                    let forward = tx.clone();
                    match lsp::spawn(&command, &path, lsp_id, move |msg| {
                        let _ = forward.send(AppEvent::Lsp(lsp_id, msg));
                    }) {
                        Ok(client) => {
                            self.clients.insert(lsp_id, client);
                        }
                        Err(e) => self.editor.set_error(format!("lsp: {command}: {e}")),
                    }
                }
                None if lsp::has_candidates(lsp_id) => {
                    self.editor.set_info(format!(
                        "lsp: no {lsp_id} server installed (alt+l for details)"
                    ));
                }
                None => {}
            }
        }
        let Some(client) = self.clients.get_mut(lsp_id) else {
            self.editor.lsp_indicator = None;
            return;
        };
        self.editor.lsp_indicator = Some(if client.ready {
            client.name.clone()
        } else {
            format!("{} starting", client.name)
        });
        if !client.ready {
            return;
        }
        let uri = lsp::file_uri(&path);
        let revision = self.editor.buffer.revision();
        let synced = if !client.is_open(&uri) {
            let text = self.editor.buffer.text();
            client.did_open(&uri, &text).is_ok()
        } else if self.synced.get(&uri) != Some(&revision) {
            let text = self.editor.buffer.text();
            client.did_change(&uri, &text).is_ok()
        } else {
            return;
        };
        if synced {
            if client.supports_pull() {
                let _ = client.pull_diagnostics(&uri);
            }
            self.synced.insert(uri, revision);
        }
    }

    fn send_did_save(&mut self) {
        let Some((_, uri, client)) = self.active_ready_client() else {
            return;
        };
        let _ = client.did_save(&uri);
        if client.supports_pull() {
            let _ = client.pull_diagnostics(&uri);
        }
    }

    fn handle_lsp(&mut self, lsp_id: &'static str, msg: Value) {
        debug_log(&format!(
            "recv {lsp_id} id={:?} method={:?}",
            msg.get("id"),
            msg.get("method")
        ));
        let Some(client) = self.clients.get_mut(lsp_id) else {
            return;
        };
        match client.handle_message(msg) {
            Ok(Reply::Ready) => {}
            Ok(Reply::Completion { id, items }) => {
                debug_log(&format!(
                    "completion reply {lsp_id} id={id} items={} expecting={:?}",
                    items.len(),
                    self.completion_request
                ));
                if let Some((expected_lang, expected, line, prefix_start)) = self.completion_request
                {
                    if lsp_id == expected_lang && id == expected {
                        self.completion_request = None;
                        self.editor.apply_lsp_completions(line, prefix_start, items);
                    }
                }
            }
            Ok(Reply::Hover { id, text }) => {
                if let Some((expected_lang, expected, anchor)) = self.hover_request {
                    if lsp_id == expected_lang && id == expected {
                        self.hover_request = None;
                        if let Some(text) = text {
                            self.editor.show_hover(anchor, &text);
                        }
                    }
                }
            }
            Ok(Reply::Diagnostics { uri, items }) => {
                self.diagnostics_version += 1;
                if items.is_empty() {
                    self.diagnostics.remove(&uri);
                } else {
                    self.diagnostics.insert(uri, items);
                }
            }
            Ok(Reply::Failed(message)) => {
                self.editor.set_error(format!("lsp: {message}"));
                self.clients.remove(lsp_id);
            }
            Ok(Reply::None) => {}
            Err(_) => {
                self.clients.remove(lsp_id);
            }
        }
    }

    fn publish_diagnostics(&mut self) {
        let uri = match self.active_language() {
            Some((_, path)) if self.editor.settings.lsp => lsp::file_uri(&path),
            _ => String::new(),
        };
        let stamp = (uri, self.diagnostics_version);
        if self.diagnostics_shown.as_ref() == Some(&stamp) {
            return;
        }
        match self.diagnostics.get(&stamp.0) {
            Some(items) => self.editor.set_diagnostics(items),
            None => self.editor.clear_diagnostics(),
        }
        self.diagnostics_shown = Some(stamp);
    }

    fn active_ready_client(&mut self) -> Option<(&'static str, String, &mut LspClient)> {
        let (lsp_id, path) = self.active_language()?;
        let uri = lsp::file_uri(&path);
        let client = self.clients.get_mut(lsp_id)?;
        if !client.ready || !client.is_open(&uri) {
            return None;
        }
        Some((lsp_id, uri, client))
    }

    fn request_completion(&mut self) {
        let Some(prefix_start) = self.editor.take_completion_request() else {
            return;
        };
        let pos = self.editor.buffer.cursor.pos;
        let character = self.editor.buffer.utf16_col(pos.line, pos.col);
        let Some((lsp_id, uri, client)) = self.active_ready_client() else {
            return;
        };
        if let Ok(id) = client.completion(&uri, pos.line, character) {
            self.completion_request = Some((lsp_id, id, pos.line, prefix_start));
        }
    }

    fn send_hover(&mut self, anchor: Position) {
        let character = self.editor.buffer.utf16_col(anchor.line, anchor.col);
        let Some((lsp_id, uri, client)) = self.active_ready_client() else {
            return;
        };
        if let Ok(id) = client.hover(&uri, anchor.line, character) {
            self.hover_request = Some((lsp_id, id, anchor));
        }
    }

    fn poll_mouse_hover(&mut self) {
        let Some((x, y, since)) = self.mouse_at else {
            return;
        };
        if self.hover_sent_at == Some((x, y)) || since.elapsed() < HOVER_DELAY {
            return;
        }
        if self.editor.completion.is_some()
            || self.editor.menu.is_some()
            || self.editor.explorer.is_some()
        {
            return;
        }
        let Some(pos) = self.editor.text_position_at(x, y) else {
            return;
        };
        self.hover_sent_at = Some((x, y));
        self.send_hover(pos);
    }

    fn show_lsp_menu(&mut self) {
        let root = self
            .editor
            .buffer
            .path()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut lines = Vec::new();
        for spec in syntax::LANGUAGES.iter().filter(|s| !s.lsp_id.is_empty()) {
            let configured = self
                .editor
                .settings
                .language_servers
                .get(spec.lsp_id)
                .cloned();
            let resolved = configured
                .clone()
                .or_else(|| lsp::default_server(spec.lsp_id, &root));
            let status = if let Some(client) = self.clients.get(spec.lsp_id) {
                if client.ready {
                    "active"
                } else {
                    "starting"
                }
            } else if resolved.is_some() {
                if configured.is_some() {
                    "configured"
                } else {
                    "available"
                }
            } else if lsp::has_candidates(spec.lsp_id) {
                "not installed"
            } else {
                "no default"
            };
            let shown = resolved
                .as_deref()
                .or_else(|| lsp::suggested_command(spec.lsp_id));
            lines.push(format_lsp_row(spec.name, shown, status));
        }
        self.editor.menu = Some(Menu::new(
            "Language Servers",
            lines,
            "esc closes, configure in config.toml [language_servers]",
        ));
    }

    fn show_help_menu(&mut self) {
        let bindings = self.keymap.bindings();
        let key_width = bindings
            .iter()
            .map(|(keys, _)| keys.chars().count())
            .max()
            .unwrap_or(0)
            .max(4);
        let mut lines = Vec::with_capacity(bindings.len() + 8);
        let mut group: Option<String> = None;
        for (keys, command) in bindings {
            let prefix = command
                .split_once('.')
                .map(|(p, _)| p.to_string())
                .unwrap_or_else(|| command.clone());
            if group.as_ref() != Some(&prefix) {
                if group.is_some() {
                    lines.push(String::new());
                }
                group = Some(prefix);
            }
            lines.push(format!("{keys:<key_width$}   {command}"));
        }
        self.editor.menu = Some(Menu::new(
            "Keybindings",
            lines,
            "arrows and page keys scroll, esc closes, rebind in keybindings.toml",
        ));
    }
}

fn load_plugins(
    registry: &mut CommandRegistry,
    keymap: &mut Keymap,
    warnings: &mut Vec<String>,
) -> Option<PluginHost> {
    let dir = crate::config::config_dir()?.join("plugins");
    let existing: Vec<String> = registry.ids().map(str::to_string).collect();
    let mut host = match PluginHost::new(existing) {
        Ok(host) => host,
        Err(e) => {
            warnings.push(format!("plugins: {e}"));
            return None;
        }
    };
    warnings.extend(host.load_dir(&dir));
    for (handle, name) in host.command_names().iter().enumerate() {
        registry.register(name, Command::Plugin(handle));
    }
    for (keys, command) in host.take_bindings() {
        if registry.get(&command).is_none() {
            warnings.push(format!("plugins: keymap.bind: unknown command `{command}`"));
            continue;
        }
        if let Err(e) = keymap.bind_str(&keys, &command) {
            warnings.push(format!("plugins: keymap.bind: {e}"));
        }
    }
    Some(host)
}

fn format_lsp_row(name: &str, command: Option<&str>, status: &str) -> String {
    let command = command
        .map(util::contract_home)
        .unwrap_or_else(|| "not set".to_string());
    let command = util::truncate_chars(&command, 44);
    format!("{name:<12} {status:<14} {command}")
}

fn debug_log(message: &str) {
    if let Ok(path) = std::env::var("KANSO_DEBUG_LOG") {
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{message}");
        }
    }
}

fn spawn_input_thread(tx: Sender<AppEvent>) {
    thread::spawn(move || loop {
        match event::read() {
            Ok(ev) => {
                if tx.send(AppEvent::Term(ev)).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_rows_align_and_show_missing_servers() {
        let row = format_lsp_row("Rust", Some("rust-analyzer"), "available");
        assert_eq!(row, "Rust         available      rust-analyzer");
        let row = format_lsp_row("TOML", None, "no default");
        assert!(row.ends_with("not set"));
    }

    #[test]
    fn long_commands_are_truncated() {
        let long = "x".repeat(60);
        let row = format_lsp_row("Go", Some(&long), "available");
        assert!(row.ends_with('…'));
        assert!(row.chars().count() < 80);
    }
}
