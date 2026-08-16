use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

use crate::syntax::TokenKind;

use super::frame::{Color, Style};

#[derive(Debug, Clone)]
pub struct Theme {
    pub editor_background: Color,
    pub editor_foreground: Color,
    pub editor_selection: Color,
    pub line_number_normal: Color,
    pub line_number_active: Color,
    pub statusline_background: Color,
    pub statusline_foreground: Color,
    pub ui_accent: Color,
    pub ui_warning: Color,
    pub ui_error: Color,
    pub syntax_keyword: Color,
    pub syntax_string: Color,
    pub syntax_comment: Color,
    pub syntax_function: Color,
    pub syntax_type: Color,
    pub syntax_number: Color,
    pub popup_background: Color,
    pub popup_foreground: Color,
    pub popup_selected: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            editor_background: Color::Rgb(0x17, 0x1a, 0x21),
            editor_foreground: Color::Rgb(0xc8, 0xcc, 0xd4),
            editor_selection: Color::Rgb(0x3b, 0x42, 0x52),
            line_number_normal: Color::Rgb(0x4c, 0x56, 0x6a),
            line_number_active: Color::Rgb(0xc8, 0xcc, 0xd4),
            statusline_background: Color::Rgb(0x23, 0x28, 0x33),
            statusline_foreground: Color::Rgb(0xaa, 0xb2, 0xc0),
            ui_accent: Color::Rgb(0x88, 0xc0, 0xd0),
            ui_warning: Color::Rgb(0xeb, 0xcb, 0x8b),
            ui_error: Color::Rgb(0xbf, 0x61, 0x6a),
            syntax_keyword: Color::Rgb(0x81, 0xa1, 0xc1),
            syntax_string: Color::Rgb(0xa3, 0xbe, 0x8c),
            syntax_comment: Color::Rgb(0x61, 0x6e, 0x88),
            syntax_function: Color::Rgb(0x88, 0xc0, 0xd0),
            syntax_type: Color::Rgb(0x8f, 0xbc, 0xbb),
            syntax_number: Color::Rgb(0xb4, 0x8e, 0xad),
            popup_background: Color::Rgb(0x25, 0x2b, 0x38),
            popup_foreground: Color::Rgb(0xc8, 0xcc, 0xd4),
            popup_selected: Color::Rgb(0x3e, 0x46, 0x5a),
        }
    }
}

impl Theme {
    pub fn text(&self) -> Style {
        Style::fg_bg(self.editor_foreground, self.editor_background)
    }

    pub fn selection(&self) -> Style {
        Style::fg_bg(self.editor_foreground, self.editor_selection)
    }

    pub fn line_number(&self, active: bool) -> Style {
        let fg = if active {
            self.line_number_active
        } else {
            self.line_number_normal
        };
        Style::fg_bg(fg, self.editor_background)
    }

    pub fn statusline(&self) -> Style {
        Style::fg_bg(self.statusline_foreground, self.statusline_background)
    }

    pub fn syntax(&self, kind: TokenKind) -> Style {
        let fg = match kind {
            TokenKind::Keyword => self.syntax_keyword,
            TokenKind::String => self.syntax_string,
            TokenKind::Comment => self.syntax_comment,
            TokenKind::Function => self.syntax_function,
            TokenKind::Type => self.syntax_type,
            TokenKind::Number => self.syntax_number,
        };
        let mut style = Style::fg_bg(fg, self.editor_background);
        style.italic = kind == TokenKind::Comment;
        style
    }

    pub fn popup(&self, selected: bool) -> Style {
        let bg = if selected {
            self.popup_selected
        } else {
            self.popup_background
        };
        Style::fg_bg(self.popup_foreground, bg)
    }

    pub fn apply_role(&mut self, role: &str, color: Color) -> bool {
        let slot = match role {
            "editor.background" => &mut self.editor_background,
            "editor.foreground" => &mut self.editor_foreground,
            "editor.selection" => &mut self.editor_selection,
            "line_number.normal" => &mut self.line_number_normal,
            "line_number.active" => &mut self.line_number_active,
            "statusline.background" => &mut self.statusline_background,
            "statusline.foreground" => &mut self.statusline_foreground,
            "ui.accent" => &mut self.ui_accent,
            "ui.warning" => &mut self.ui_warning,
            "ui.error" => &mut self.ui_error,
            "syntax.keyword" => &mut self.syntax_keyword,
            "syntax.string" => &mut self.syntax_string,
            "syntax.comment" => &mut self.syntax_comment,
            "syntax.function" => &mut self.syntax_function,
            "syntax.type" => &mut self.syntax_type,
            "syntax.number" => &mut self.syntax_number,
            "popup.background" => &mut self.popup_background,
            "popup.foreground" => &mut self.popup_foreground,
            "popup.selected" => &mut self.popup_selected,
            _ => return false,
        };
        *slot = color;
        true
    }

    pub fn load(path: &Path) -> Result<Theme, ThemeError> {
        let text = std::fs::read_to_string(path)?;
        let file: ThemeFile = toml::from_str(&text)?;
        let mut theme = Theme::default();
        for (role, value) in &file.colors {
            let color =
                parse_color(value).ok_or_else(|| ThemeError::Color(role.clone(), value.clone()))?;
            theme.apply_role(role, color);
        }
        Ok(theme)
    }
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    #[serde(default)]
    colors: HashMap<String, String>,
}

#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("cannot read theme: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot parse theme: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid color for `{0}`: `{1}`")]
    Color(String, String),
}

pub fn parse_color(spec: &str) -> Option<Color> {
    let spec = spec.trim().to_ascii_lowercase();
    if let Some(hex) = spec.strip_prefix('#') {
        return match hex.len() {
            3 => {
                let value = u16::from_str_radix(hex, 16).ok()?;
                let (r, g, b) = (
                    (value >> 8) as u8,
                    (value >> 4 & 0xf) as u8,
                    (value & 0xf) as u8,
                );
                Some(Color::Rgb(r * 17, g * 17, b * 17))
            }
            6 => {
                let value = u32::from_str_radix(hex, 16).ok()?;
                Some(Color::Rgb(
                    (value >> 16) as u8,
                    (value >> 8) as u8,
                    value as u8,
                ))
            }
            _ => None,
        };
    }
    if let Ok(index) = spec.parse::<u8>() {
        return Some(Color::Ansi(index));
    }
    let index = match spec.as_str() {
        "default" | "none" | "reset" => return Some(Color::Reset),
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "white" => 7,
        "bright_black" | "gray" | "grey" => 8,
        "bright_red" => 9,
        "bright_green" => 10,
        "bright_yellow" => 11,
        "bright_blue" => 12,
        "bright_magenta" => 13,
        "bright_cyan" => 14,
        "bright_white" => 15,
        _ => return None,
    };
    Some(Color::Ansi(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_color_formats() {
        assert_eq!(parse_color("#ff8000"), Some(Color::Rgb(255, 128, 0)));
        assert_eq!(parse_color("#f80"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(parse_color("red"), Some(Color::Ansi(1)));
        assert_eq!(parse_color("bright_blue"), Some(Color::Ansi(12)));
        assert_eq!(parse_color("42"), Some(Color::Ansi(42)));
        assert_eq!(parse_color("default"), Some(Color::Reset));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("mauve-ish"), None);
    }

    #[test]
    fn apply_role_sets_known_roles_only() {
        let mut theme = Theme::default();
        assert!(theme.apply_role("editor.background", Color::Ansi(0)));
        assert_eq!(theme.editor_background, Color::Ansi(0));
        assert!(theme.apply_role("syntax.keyword", Color::Ansi(1)));
        assert_eq!(theme.syntax_keyword, Color::Ansi(1));
        assert!(!theme.apply_role("syntax.nonsense", Color::Ansi(1)));
    }

    #[test]
    fn comment_style_is_italic() {
        let theme = Theme::default();
        assert!(theme.syntax(TokenKind::Comment).italic);
        assert!(!theme.syntax(TokenKind::Keyword).italic);
    }
}
