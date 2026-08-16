mod cursor;
mod indent;
mod selection;

#[allow(clippy::module_inception)]
mod buffer;

pub use buffer::Buffer;
pub use cursor::{Motion, Position};
pub use indent::IndentStyle;

use unicode_width::UnicodeWidthChar;

pub fn char_display_width(ch: char, at_col: usize, tab_width: usize) -> usize {
    debug_assert!(tab_width > 0);
    if ch == '\t' {
        tab_width - (at_col % tab_width)
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0)
    }
}
