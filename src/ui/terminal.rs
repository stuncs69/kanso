use std::io::{self, Write};

use crossterm::{
    cursor, event, execute, queue,
    style::{self, Attribute, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::config::CursorStyle;

use super::frame::{Color, Frame, Style, WIDE_CONTINUATION};

pub struct Terminal {
    out: io::Stdout,
}

impl Terminal {
    pub fn new(cursor_style: CursorStyle, mouse: bool) -> io::Result<Self> {
        let mut out = io::stdout();
        terminal::enable_raw_mode()?;
        execute!(
            out,
            EnterAlternateScreen,
            event::EnableBracketedPaste,
            match cursor_style {
                CursorStyle::Block => cursor::SetCursorStyle::SteadyBlock,
                CursorStyle::Bar => cursor::SetCursorStyle::SteadyBar,
                CursorStyle::Underline => cursor::SetCursorStyle::SteadyUnderScore,
            },
        )?;
        if mouse {
            execute!(out, event::EnableMouseCapture)?;
        }
        Ok(Terminal { out })
    }

    pub fn size() -> io::Result<(u16, u16)> {
        terminal::size()
    }

    pub fn draw(
        &mut self,
        frame: &Frame,
        rows: &[u16],
        cursor_pos: Option<(u16, u16)>,
    ) -> io::Result<()> {
        queue!(self.out, cursor::Hide)?;
        for &y in rows {
            queue!(self.out, cursor::MoveTo(0, y))?;
            let mut current: Option<Style> = None;
            for cell in frame.row(y) {
                if cell.ch == WIDE_CONTINUATION {
                    continue;
                }
                if current != Some(cell.style) {
                    apply_style(&mut self.out, cell.style)?;
                    current = Some(cell.style);
                }
                queue!(self.out, style::Print(cell.ch))?;
            }
        }
        if let Some((x, y)) = cursor_pos {
            queue!(self.out, cursor::MoveTo(x, y), cursor::Show)?;
        }
        self.out.flush()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

pub fn restore() {
    let mut out = io::stdout();
    let _ = execute!(
        out,
        style::ResetColor,
        SetAttribute(Attribute::Reset),
        cursor::SetCursorStyle::DefaultUserShape,
        cursor::Show,
        event::DisableMouseCapture,
        event::DisableBracketedPaste,
        LeaveAlternateScreen,
    );
    let _ = terminal::disable_raw_mode();
}

fn apply_style(out: &mut impl Write, style: Style) -> io::Result<()> {
    queue!(
        out,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(convert(style.fg)),
        SetBackgroundColor(convert(style.bg)),
    )?;
    if style.bold {
        queue!(out, SetAttribute(Attribute::Bold))?;
    }
    if style.italic {
        queue!(out, SetAttribute(Attribute::Italic))?;
    }
    if style.underline {
        queue!(out, SetAttribute(Attribute::Underlined))?;
    }
    Ok(())
}

fn convert(color: Color) -> style::Color {
    match color {
        Color::Reset => style::Color::Reset,
        Color::Ansi(index) => style::Color::AnsiValue(index),
        Color::Rgb(r, g, b) => style::Color::Rgb { r, g, b },
    }
}
