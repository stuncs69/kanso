use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndentStyle {
    pub use_spaces: bool,
    pub width: usize,
}

impl IndentStyle {
    pub fn unit(&self) -> String {
        if self.use_spaces {
            " ".repeat(self.width)
        } else {
            "\t".to_string()
        }
    }

    pub fn label(&self) -> String {
        if self.use_spaces {
            format!("Spaces: {}", self.width)
        } else {
            format!("Tab: {}", self.width)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indent {
    pub tabs: usize,
    pub spaces: usize,
}

pub fn measure(line: &str) -> Option<Indent> {
    let mut tabs = 0;
    let mut spaces = 0;
    for c in line.chars() {
        match c {
            '\t' => tabs += 1,
            ' ' => spaces += 1,
            _ => return Some(Indent { tabs, spaces }),
        }
    }
    None
}

pub fn detect(indents: &[Indent], fallback: IndentStyle) -> IndentStyle {
    let mut tab_lines = 0usize;
    let mut space_lines = 0usize;
    let mut deltas: HashMap<usize, usize> = HashMap::new();
    let mut prev: Option<usize> = None;

    for indent in indents {
        if indent.tabs > 0 {
            tab_lines += 1;
            prev = None;
            continue;
        }
        if indent.spaces > 0 {
            space_lines += 1;
        }
        if let Some(previous) = prev {
            let delta = indent.spaces.abs_diff(previous);
            if (2..=8).contains(&delta) {
                *deltas.entry(delta).or_default() += 1;
            }
        }
        prev = Some(indent.spaces);
    }

    if tab_lines > space_lines {
        return IndentStyle {
            use_spaces: false,
            width: fallback.width,
        };
    }
    if space_lines == 0 {
        return fallback;
    }
    let width = deltas
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(width, _)| width)
        .unwrap_or(fallback.width);
    IndentStyle {
        use_spaces: true,
        width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FALLBACK: IndentStyle = IndentStyle {
        use_spaces: true,
        width: 4,
    };

    fn detect_text(text: &str) -> IndentStyle {
        let indents: Vec<Indent> = text.lines().filter_map(measure).collect();
        detect(&indents, FALLBACK)
    }

    #[test]
    fn measures_leading_whitespace() {
        assert_eq!(measure("  x"), Some(Indent { tabs: 0, spaces: 2 }));
        assert_eq!(measure("\t\tx"), Some(Indent { tabs: 2, spaces: 0 }));
        assert_eq!(measure("x"), Some(Indent { tabs: 0, spaces: 0 }));
        assert_eq!(measure("   "), None);
        assert_eq!(measure(""), None);
    }

    #[test]
    fn detects_two_space_indentation() {
        let style = detect_text("fn a() {\n  let x = 1;\n  if x {\n    y();\n  }\n}\n");
        assert_eq!(
            style,
            IndentStyle {
                use_spaces: true,
                width: 2
            }
        );
    }

    #[test]
    fn detects_four_space_indentation() {
        let style = detect_text("fn a() {\n    let x = 1;\n    if x {\n        y();\n    }\n}\n");
        assert_eq!(
            style,
            IndentStyle {
                use_spaces: true,
                width: 4
            }
        );
    }

    #[test]
    fn detects_tabs() {
        let style = detect_text("fn a() {\n\tlet x = 1;\n\tif x {\n\t\ty();\n\t}\n}\n");
        assert!(!style.use_spaces);
        assert_eq!(style.width, FALLBACK.width);
    }

    #[test]
    fn falls_back_without_indentation() {
        assert_eq!(detect_text("one\ntwo\nthree\n"), FALLBACK);
        assert_eq!(detect_text(""), FALLBACK);
    }

    #[test]
    fn blank_lines_do_not_break_detection() {
        let style = detect_text("a\n\n  b\n\n    c\n");
        assert_eq!(style.width, 2);
    }

    #[test]
    fn unit_and_label() {
        let spaces = IndentStyle {
            use_spaces: true,
            width: 3,
        };
        assert_eq!(spaces.unit(), "   ");
        assert_eq!(spaces.label(), "Spaces: 3");
        let tabs = IndentStyle {
            use_spaces: false,
            width: 8,
        };
        assert_eq!(tabs.unit(), "\t");
        assert_eq!(tabs.label(), "Tab: 8");
    }
}
