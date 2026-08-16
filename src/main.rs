mod app;
mod buffer;
mod config;
mod editor;
mod input;
mod lsp;
mod syntax;
mod ui;
mod util;

use std::path::Path;
use std::process::ExitCode;

use app::App;
use buffer::Buffer;
use ui::theme::Theme;

const USAGE: &str = "\
kanso, a fast and riceable terminal text editor

usage: kanso [FILE...]

options:
  -h, --help       print this help
  -V, --version    print the version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("kanso {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    match run(&args[1..]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kanso: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(files: &[String]) -> std::io::Result<()> {
    let config = config::load();
    let mut warnings = config.warnings;

    let theme = match &config.settings.theme {
        Some(name) => load_theme(name, &mut warnings),
        None => Theme::default(),
    };

    let mut buffers = Vec::new();
    for path in files {
        let buffer = Buffer::from_path(Path::new(path))
            .map_err(|e| std::io::Error::new(e.kind(), format!("cannot open {path}: {e}")))?;
        buffers.push(buffer);
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ui::terminal::restore();
        default_hook(info);
    }));

    App::new(
        buffers,
        config.settings,
        theme,
        &config.keybindings,
        warnings,
    )?
    .run()
}

fn load_theme(name: &str, warnings: &mut Vec<String>) -> Theme {
    let Some(dir) = config::config_dir() else {
        warnings.push("theme: no config directory found".to_string());
        return Theme::default();
    };
    let file = if name.ends_with(".toml") {
        name.to_string()
    } else {
        format!("{name}.toml")
    };
    match Theme::load(&dir.join("themes").join(file)) {
        Ok(theme) => theme,
        Err(e) => {
            warnings.push(format!("theme `{name}`: {e}"));
            Theme::default()
        }
    }
}
