use super::cursor::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: Position,
    pub end: Position,
}

impl Selection {
    pub fn from_points(a: Position, b: Position) -> Self {
        Selection {
            start: a.min(b),
            end: a.max(b),
        }
    }

    pub fn line_cols(&self, line: usize) -> Option<(usize, Option<usize>)> {
        if line < self.start.line || line > self.end.line {
            return None;
        }
        let start = if line == self.start.line {
            self.start.col
        } else {
            0
        };
        let end = if line == self.end.line {
            Some(self.end.col)
        } else {
            None
        };
        Some((start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_points_normalizes() {
        let a = Position::new(3, 2);
        let b = Position::new(1, 5);
        let sel = Selection::from_points(a, b);
        assert_eq!(sel.start, b);
        assert_eq!(sel.end, a);
    }

    #[test]
    fn line_cols_spanning_lines() {
        let sel = Selection::from_points(Position::new(1, 2), Position::new(3, 4));
        assert_eq!(sel.line_cols(0), None);
        assert_eq!(sel.line_cols(1), Some((2, None)));
        assert_eq!(sel.line_cols(2), Some((0, None)));
        assert_eq!(sel.line_cols(3), Some((0, Some(4))));
        assert_eq!(sel.line_cols(4), None);
    }

    #[test]
    fn line_cols_single_line() {
        let sel = Selection::from_points(Position::new(0, 1), Position::new(0, 4));
        assert_eq!(sel.line_cols(0), Some((1, Some(4))));
    }
}
