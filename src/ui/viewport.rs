#[derive(Debug, Default, Clone, Copy)]
pub struct Viewport {
    pub top_line: usize,
    pub left_col: usize,
    pub width: u16,
    pub height: u16,
}

impl Viewport {
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    pub fn text_height(&self) -> usize {
        self.height.saturating_sub(1) as usize
    }

    pub fn ensure_visible(&mut self, line: usize, display_col: usize, gutter: u16) {
        let text_height = self.text_height().max(1);
        if line < self.top_line {
            self.top_line = line;
        } else if line >= self.top_line + text_height {
            self.top_line = line + 1 - text_height;
        }

        let text_width = (self.width.saturating_sub(gutter) as usize).max(1);
        if display_col < self.left_col {
            self.left_col = display_col;
        } else if display_col >= self.left_col + text_width {
            self.left_col = display_col + 1 - text_width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolls_down_and_back_up() {
        let mut vp = Viewport {
            width: 80,
            height: 11,
            ..Viewport::default()
        };
        vp.ensure_visible(25, 0, 4);
        assert_eq!(vp.top_line, 16);
        vp.ensure_visible(3, 0, 4);
        assert_eq!(vp.top_line, 3);
    }

    #[test]
    fn scrolls_horizontally() {
        let mut vp = Viewport {
            width: 20,
            height: 5,
            ..Viewport::default()
        };
        vp.ensure_visible(0, 30, 4);
        assert_eq!(vp.left_col, 15);
        vp.ensure_visible(0, 2, 4);
        assert_eq!(vp.left_col, 2);
    }

    #[test]
    fn no_scroll_when_visible() {
        let mut vp = Viewport {
            width: 80,
            height: 24,
            ..Viewport::default()
        };
        vp.ensure_visible(5, 10, 4);
        assert_eq!(vp.top_line, 0);
        assert_eq!(vp.left_col, 0);
    }
}
