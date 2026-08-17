use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use mlua::{
    Error as LuaError, Function, Lua, Result as LuaResult, Table, UserData, UserDataMethods,
};

use crate::input::KeyPress;

use super::events::Event;

pub(super) const COMMANDS_KEY: &str = "kanso.commands";
pub(super) const HANDLERS_KEY: &str = "kanso.event_handlers";

#[derive(Debug, Default, Clone)]
pub(super) struct BufferInfo {
    pub id: u64,
    pub revision: u64,
    pub name: String,
    pub path: Option<String>,
    pub text: String,
    pub modified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Action {
    Notify(String),
    Status(String, Duration),
}

#[derive(Default)]
pub(super) struct Shared {
    pub buffer: BufferInfo,
    pub actions: Vec<Action>,
    pub commands: Vec<String>,
    pub known_commands: HashSet<String>,
    pub bindings: Vec<(String, String)>,
}

struct LuaBuffer(Rc<RefCell<Shared>>);

impl UserData for LuaBuffer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("id", |_, this, ()| Ok(this.0.borrow().buffer.id));
        methods.add_method("name", |_, this, ()| {
            Ok(this.0.borrow().buffer.name.clone())
        });
        methods.add_method("path", |_, this, ()| {
            Ok(this.0.borrow().buffer.path.clone())
        });
        methods.add_method("text", |_, this, ()| {
            Ok(this.0.borrow().buffer.text.clone())
        });
        methods.add_method("is_modified", |_, this, ()| {
            Ok(this.0.borrow().buffer.modified)
        });
    }
}

pub(super) fn install(lua: &Lua, shared: &Rc<RefCell<Shared>>) -> LuaResult<()> {
    lua.set_named_registry_value(COMMANDS_KEY, lua.create_table()?)?;
    lua.set_named_registry_value(HANDLERS_KEY, lua.create_table()?)?;

    let kanso = lua.create_table()?;
    kanso.set("editor", editor_table(lua, shared)?)?;
    kanso.set("commands", commands_table(lua, shared)?)?;
    kanso.set("keymap", keymap_table(lua, shared)?)?;
    kanso.set("events", events_table(lua)?)?;
    kanso.set("ui", ui_table(lua, shared)?)?;
    lua.globals().set("kanso", kanso)?;
    Ok(())
}

fn editor_table(lua: &Lua, shared: &Rc<RefCell<Shared>>) -> LuaResult<Table> {
    let table = lua.create_table()?;
    let shared = Rc::clone(shared);
    table.set(
        "current_buffer",
        lua.create_function(move |_, ()| Ok(LuaBuffer(Rc::clone(&shared))))?,
    )?;
    Ok(table)
}

fn commands_table(lua: &Lua, shared: &Rc<RefCell<Shared>>) -> LuaResult<Table> {
    let table = lua.create_table()?;
    let shared = Rc::clone(shared);
    table.set(
        "register",
        lua.create_function(move |lua, (name, callback): (String, Function)| {
            let mut state = shared.borrow_mut();
            if name.trim().is_empty() {
                return Err(LuaError::runtime("command name must not be empty"));
            }
            if !state.known_commands.insert(name.clone()) {
                return Err(LuaError::runtime(format!(
                    "command `{name}` is already registered"
                )));
            }
            let callbacks: Table = lua.named_registry_value(COMMANDS_KEY)?;
            state.commands.push(name);
            callbacks.set(state.commands.len(), callback)?;
            Ok(())
        })?,
    )?;
    Ok(table)
}

fn keymap_table(lua: &Lua, shared: &Rc<RefCell<Shared>>) -> LuaResult<Table> {
    let table = lua.create_table()?;
    let shared = Rc::clone(shared);
    table.set(
        "bind",
        lua.create_function(move |_, (keys, command): (String, String)| {
            KeyPress::parse_sequence(&keys).map_err(|e| LuaError::runtime(e.to_string()))?;
            shared.borrow_mut().bindings.push((keys, command));
            Ok(())
        })?,
    )?;
    Ok(table)
}

fn events_table(lua: &Lua) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set(
        "subscribe",
        lua.create_function(|lua, (event, callback): (String, Function)| {
            let Some(name) = Event::NAMES.iter().find(|n| **n == event) else {
                return Err(LuaError::runtime(format!(
                    "unknown event `{event}` (known: {})",
                    Event::NAMES.join(", ")
                )));
            };
            let handlers: Table = lua.named_registry_value(HANDLERS_KEY)?;
            let list: Table = match handlers.get(*name)? {
                Some(list) => list,
                None => {
                    let list = lua.create_table()?;
                    handlers.set(*name, &list)?;
                    list
                }
            };
            list.set(list.raw_len() + 1, callback)?;
            Ok(())
        })?,
    )?;
    Ok(table)
}

fn ui_table(lua: &Lua, shared: &Rc<RefCell<Shared>>) -> LuaResult<Table> {
    let table = lua.create_table()?;

    let notify_shared = Rc::clone(shared);
    table.set(
        "notify",
        lua.create_function(move |_, message: String| {
            notify_shared
                .borrow_mut()
                .actions
                .push(Action::Notify(message));
            Ok(())
        })?,
    )?;

    let status_shared = Rc::clone(shared);
    table.set(
        "set_status_message",
        lua.create_function(move |_, (message, ms): (String, Option<u64>)| {
            let duration = Duration::from_millis(ms.unwrap_or(2000));
            status_shared
                .borrow_mut()
                .actions
                .push(Action::Status(message, duration));
            Ok(())
        })?,
    )?;

    Ok(table)
}
