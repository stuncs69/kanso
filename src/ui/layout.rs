#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenRow {
    Text(usize),
    Underline(usize),
    Message { line: usize, index: usize },
}

impl ScreenRow {
    pub fn line(self) -> usize {
        match self {
            ScreenRow::Text(line) | ScreenRow::Underline(line) => line,
            ScreenRow::Message { line, .. } => line,
        }
    }
}

pub fn text_row(rows: &[ScreenRow], line: usize) -> Option<u16> {
    rows.iter()
        .position(|row| *row == ScreenRow::Text(line))
        .map(|row| row as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_row_skips_virtual_rows() {
        let rows = [
            ScreenRow::Text(4),
            ScreenRow::Underline(4),
            ScreenRow::Message { line: 4, index: 0 },
            ScreenRow::Text(5),
        ];
        assert_eq!(text_row(&rows, 4), Some(0));
        assert_eq!(text_row(&rows, 5), Some(3));
        assert_eq!(text_row(&rows, 6), None);
        assert_eq!(rows[2].line(), 4);
    }
}
