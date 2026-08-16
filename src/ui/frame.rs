use unicode_width::UnicodeWidthChar;

pub const WIDE_CONTINUATION: char = '\0';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Reset,
    Ansi(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fg: Color::Reset,
            bg: Color::Reset,
            bold: false,
            italic: false,
            underline: false,
        }
    }
}

impl Style {
    pub fn fg_bg(fg: Color, bg: Color) -> Self {
        Style {
            fg,
            bg,
            ..Style::default()
        }
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

impl Cell {
    fn blank(style: Style) -> Self {
        Cell { ch: ' ', style }
    }
}

pub struct Frame {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Frame {
    pub fn new(width: u16, height: u16) -> Self {
        Frame {
            width,
            height,
            cells: vec![Cell::blank(Style::default()); width as usize * height as usize],
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn reset(&mut self, width: u16, height: u16, style: Style) {
        let blank = Cell::blank(style);
        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            self.cells.clear();
            self.cells.resize(width as usize * height as usize, blank);
        } else {
            self.cells.fill(blank);
        }
    }

    pub fn set(&mut self, x: u16, y: u16, ch: char, style: Style) {
        if x < self.width && y < self.height {
            self.cells[y as usize * self.width as usize + x as usize] = Cell { ch, style };
        }
    }

    pub fn row(&self, y: u16) -> &[Cell] {
        let w = self.width as usize;
        let start = y as usize * w;
        &self.cells[start..start + w]
    }

    pub fn put_str(&mut self, x: u16, y: u16, text: &str, style: Style, max_x: u16) -> u16 {
        let mut x = x;
        let max_x = max_x.min(self.width);
        for ch in text.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
            if w == 0 {
                continue;
            }
            if x + w > max_x {
                break;
            }
            self.set(x, y, ch, style);
            if w == 2 {
                self.set(x + 1, y, WIDE_CONTINUATION, style);
            }
            x += w;
        }
        x
    }

    pub fn changed_rows(&self, prev: &Frame) -> Vec<u16> {
        if self.width != prev.width || self.height != prev.height {
            return (0..self.height).collect();
        }
        (0..self.height)
            .filter(|&y| self.row(y) != prev.row(y))
            .collect()
    }
}

pub fn str_width(text: &str) -> usize {
    text.chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_str_handles_wide_chars() {
        let mut frame = Frame::new(10, 1);
        let end = frame.put_str(0, 0, "a你b", Style::default(), 10);
        assert_eq!(end, 4);
        assert_eq!(frame.row(0)[0].ch, 'a');
        assert_eq!(frame.row(0)[1].ch, '你');
        assert_eq!(frame.row(0)[2].ch, WIDE_CONTINUATION);
        assert_eq!(frame.row(0)[3].ch, 'b');
    }

    #[test]
    fn put_str_never_splits_a_wide_char() {
        let mut frame = Frame::new(10, 1);
        let end = frame.put_str(0, 0, "ab你", Style::default(), 3);
        assert_eq!(end, 2);
        assert_eq!(frame.row(0)[2].ch, ' ');
    }

    #[test]
    fn changed_rows_only_reports_differences() {
        let mut a = Frame::new(4, 3);
        let b = Frame::new(4, 3);
        assert!(a.changed_rows(&b).is_empty());
        a.set(1, 2, 'x', Style::default());
        assert_eq!(a.changed_rows(&b), vec![2]);
    }

    #[test]
    fn resize_forces_full_repaint() {
        let a = Frame::new(4, 3);
        let b = Frame::new(5, 3);
        assert_eq!(a.changed_rows(&b), vec![0, 1, 2]);
    }
}
