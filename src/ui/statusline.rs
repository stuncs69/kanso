use std::path::Path;

use crate::editor::{Editor, MessageKind};

use super::frame::{str_width, Frame, Style};
use super::theme::Theme;

pub fn render(frame: &mut Frame, editor: &Editor, theme: &Theme, y: u16) {
    let width = frame.width();
    let base = theme.statusline();
    for x in 0..width {
        frame.set(x, y, ' ', base);
    }

    let cursor = editor.buffer.cursor.pos;
    let lsp = editor
        .lsp_indicator
        .as_ref()
        .map(|name| format!("{name}   "))
        .unwrap_or_default();
    let buffers = match editor.buffer_count() {
        1 => String::new(),
        n => format!("{n} buffers   "),
    };
    let tail = format!(
        "{buffers}{lsp}{}   {}   Ln {}, Col {}   UTF-8 ",
        language_name(editor.buffer.path()),
        editor.buffer.indent().label(),
        cursor.line + 1,
        cursor.col + 1,
    );
    let mut right: Vec<(String, Style)> = diagnostic_segments(editor, theme);
    right.push((tail, base));
    let total: usize = right.iter().map(|(text, _)| str_width(text)).sum();
    let right_x = width.saturating_sub(total as u16);
    let mut x = right_x;
    for (text, style) in &right {
        x = frame.put_str(x, y, text, *style, width);
    }

    let left_max = right_x.saturating_sub(1);
    let mut x = frame.put_str(1, y, &editor.buffer.file_name(), base.bold(), left_max);
    if editor.buffer.is_dirty() {
        let dirty = Style::fg_bg(theme.ui_warning, theme.statusline_background);
        x = frame.put_str(x, y, " ●", dirty, left_max);
    }
    if let Some(message) = &editor.status {
        let fg = match message.kind {
            MessageKind::Info => theme.ui_accent,
            MessageKind::Error => theme.ui_error,
        };
        let style = Style::fg_bg(fg, theme.statusline_background);
        frame.put_str(x + 2, y, &message.text, style, left_max);
    }
}

fn diagnostic_segments(editor: &Editor, theme: &Theme) -> Vec<(String, Style)> {
    let (errors, warnings) = editor.diagnostic_counts();
    let mut segments = Vec::new();
    if errors > 0 {
        let style = Style::fg_bg(theme.diagnostic_error, theme.statusline_background);
        segments.push((format!("E {errors}  "), style));
    }
    if warnings > 0 {
        let style = Style::fg_bg(theme.diagnostic_warning, theme.statusline_background);
        segments.push((format!("W {warnings}  "), style));
    }
    if !segments.is_empty() {
        segments.push(("   ".to_string(), theme.statusline()));
    }
    segments
}

fn language_name(path: Option<&Path>) -> &'static str {
    if let Some(spec) = crate::syntax::detect(path) {
        return spec.name;
    }
    let Some(ext) = path.and_then(Path::extension).and_then(|e| e.to_str()) else {
        return "Plain Text";
    };
    match ext.to_ascii_lowercase().as_str() {
        "md" | "markdown" => "Markdown",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "lua" => "Lua",
        "vx" => "Vexel",
        _ => "Plain Text",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages_by_extension() {
        assert_eq!(language_name(Some(Path::new("main.rs"))), "Rust");
        assert_eq!(language_name(Some(Path::new("Cargo.toml"))), "TOML");
        assert_eq!(language_name(Some(Path::new("notes"))), "Plain Text");
        assert_eq!(language_name(None), "Plain Text");
    }
}
