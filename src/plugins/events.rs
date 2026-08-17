use mlua::{Lua, Result as LuaResult, Table};

use crate::buffer::Buffer;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    BufferOpened { buffer_id: u64 },
    BufferSaved { buffer_id: u64 },
    BufferChanged { buffer_id: u64, revision: u64 },
}

impl Event {
    pub const NAMES: &'static [&'static str] = &["buffer_opened", "buffer_saved", "buffer_changed"];

    pub fn buffer_opened(buffer: &Buffer) -> Event {
        Event::BufferOpened {
            buffer_id: buffer.id(),
        }
    }

    pub fn buffer_saved(buffer: &Buffer) -> Event {
        Event::BufferSaved {
            buffer_id: buffer.id(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Event::BufferOpened { .. } => "buffer_opened",
            Event::BufferSaved { .. } => "buffer_saved",
            Event::BufferChanged { .. } => "buffer_changed",
        }
    }

    pub fn to_table(&self, lua: &Lua) -> LuaResult<Table> {
        let table = lua.create_table()?;
        table.set("event", self.name())?;
        match *self {
            Event::BufferOpened { buffer_id } | Event::BufferSaved { buffer_id } => {
                table.set("buffer_id", buffer_id)?;
            }
            Event::BufferChanged {
                buffer_id,
                revision,
            } => {
                table.set("buffer_id", buffer_id)?;
                table.set("revision", revision)?;
            }
        }
        Ok(table)
    }
}
