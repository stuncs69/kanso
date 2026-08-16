use std::collections::HashMap;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub tab_width: usize,
    pub insert_spaces: bool,
    pub line_numbers: bool,
    pub detect_indentation: bool,
    pub auto_indent: bool,
    pub syntax_highlighting: bool,
    pub auto_pairs: bool,
    pub mouse: bool,
    pub lsp: bool,
    pub cursor_style: CursorStyle,
    pub theme: Option<String>,
    pub language_servers: HashMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            tab_width: 4,
            insert_spaces: true,
            line_numbers: true,
            detect_indentation: true,
            auto_indent: true,
            syntax_highlighting: true,
            auto_pairs: true,
            mouse: true,
            lsp: true,
            cursor_style: CursorStyle::Block,
            theme: None,
            language_servers: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

pub struct Config {
    pub settings: Settings,
    pub keybindings: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

pub fn config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("KANSO_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    ProjectDirs::from("", "", "kanso").map(|dirs| dirs.config_dir().to_path_buf())
}

pub fn load() -> Config {
    load_from(config_dir().as_deref())
}

fn load_from(dir: Option<&Path>) -> Config {
    let mut warnings = Vec::new();
    let mut settings = Settings::default();
    let mut keybindings = Vec::new();

    if let Some(dir) = dir {
        if let Some(text) = read_optional(&dir.join("config.toml"), &mut warnings) {
            match toml::from_str::<Settings>(&text) {
                Ok(parsed) => settings = parsed,
                Err(e) => warnings.push(format!("config.toml: {}", first_line(&e.to_string()))),
            }
        }
        if let Some(text) = read_optional(&dir.join("keybindings.toml"), &mut warnings) {
            match toml::from_str::<KeybindingsFile>(&text) {
                Ok(parsed) => keybindings = parsed.keybindings.into_iter().collect(),
                Err(e) => {
                    warnings.push(format!("keybindings.toml: {}", first_line(&e.to_string())))
                }
            }
        }
    }

    settings.tab_width = settings.tab_width.clamp(1, 16);
    Config {
        settings,
        keybindings,
        warnings,
    }
}

fn read_optional(path: &Path, warnings: &mut Vec<String>) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warnings.push(format!("{}: {e}", path.display()));
            None
        }
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text).trim()
}

#[derive(Debug, Deserialize)]
struct KeybindingsFile {
    #[serde(default)]
    keybindings: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let settings = Settings::default();
        assert_eq!(settings.tab_width, 4);
        assert!(settings.insert_spaces);
        assert!(settings.line_numbers);
        assert_eq!(settings.cursor_style, CursorStyle::Block);
    }

    #[test]
    fn partial_config_keeps_defaults_and_ignores_unknown_keys() {
        let settings: Settings =
            toml::from_str("tab_width = 8\ncursor_style = \"bar\"\nword_wrap = false\n").unwrap();
        assert_eq!(settings.tab_width, 8);
        assert_eq!(settings.cursor_style, CursorStyle::Bar);
        assert!(settings.line_numbers);
        assert!(settings.syntax_highlighting);
        assert!(settings.auto_pairs);
        assert!(settings.detect_indentation);
        assert!(settings.auto_indent);
    }

    #[test]
    fn language_servers_table_parses() {
        let settings: Settings =
            toml::from_str("lsp = true\n[language_servers]\nrust = \"rust-analyzer\"\n").unwrap();
        assert!(settings.lsp);
        assert_eq!(
            settings.language_servers.get("rust").map(String::as_str),
            Some("rust-analyzer")
        );
    }

    #[test]
    fn keybindings_file_parses() {
        let file: KeybindingsFile = toml::from_str(
            "[keybindings]\n\"ctrl+s\" = \"file.save\"\n\"ctrl+k ctrl+c\" = \"editor.comment\"\n",
        )
        .unwrap();
        assert_eq!(file.keybindings.len(), 2);
        assert_eq!(
            file.keybindings.get("ctrl+s").map(String::as_str),
            Some("file.save")
        );
    }

    #[test]
    fn missing_dir_yields_defaults_without_warnings() {
        let config = load_from(Some(Path::new("/nonexistent/kanso-test")));
        assert!(config.warnings.is_empty());
        assert_eq!(config.settings.tab_width, 4);
        assert!(config.keybindings.is_empty());
    }
}
