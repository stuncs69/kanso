mod client;

use std::path::{Path, PathBuf};

pub use client::{file_uri, spawn, Diagnostic, LspClient, Reply, Severity};

pub fn default_server(lsp_id: &str, file_path: &Path) -> Option<String> {
    if matches!(lsp_id, "javascript" | "typescript") {
        if let Some(native) = native_typescript_lsp(file_path) {
            return Some(native);
        }
    }
    candidates(lsp_id)
        .iter()
        .map(|candidate| expand_home(candidate))
        .find(|candidate| command_available(candidate))
}

pub fn has_candidates(lsp_id: &str) -> bool {
    !candidates(lsp_id).is_empty() || matches!(lsp_id, "javascript" | "typescript")
}

pub fn suggested_command(lsp_id: &str) -> Option<&'static str> {
    candidates(lsp_id).first().copied()
}

fn candidates(lsp_id: &str) -> &'static [&'static str] {
    match lsp_id {
        "rust" => &["rust-analyzer", "~/.cargo/bin/rust-analyzer"],
        "go" => &["gopls", "~/go/bin/gopls"],
        "c" | "cpp" => &["clangd"],
        "python" => &["pylsp"],
        "javascript" | "typescript" => &[
            "typescript-language-server --stdio",
            "bunx typescript-language-server --stdio",
        ],
        _ => &[],
    }
}

fn native_typescript_lsp(file_path: &Path) -> Option<String> {
    let package = native_typescript_package();
    let mut roots: Vec<PathBuf> = Vec::new();
    roots.push(client::project_root(file_path).join("node_modules"));
    if let Some(dirs) = directories::BaseDirs::new() {
        let home = dirs.home_dir();
        roots.push(home.join(".bun/install/global/node_modules"));
        roots.push(home.join(".npm-global/lib/node_modules"));
    }
    roots.push(PathBuf::from("/usr/lib/node_modules"));
    roots.push(PathBuf::from("/usr/local/lib/node_modules"));
    for root in roots {
        let exe = root.join("@typescript").join(&package).join("lib/tsc");
        if is_executable(&exe) {
            return Some(format!("{} --lsp --stdio", exe.display()));
        }
    }
    None
}

fn native_typescript_package() -> String {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("typescript-{platform}-{arch}")
}

fn expand_home(command_line: &str) -> String {
    let Some(rest) = command_line.strip_prefix("~/") else {
        return command_line.to_string();
    };
    match directories::BaseDirs::new() {
        Some(dirs) => format!("{}/{rest}", dirs.home_dir().display()),
        None => command_line.to_string(),
    }
}

fn command_available(command_line: &str) -> bool {
    let Some(program) = command_line.split_whitespace().next() else {
        return false;
    };
    if program.contains('/') {
        return is_executable(Path::new(program));
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(program)))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_and_js_share_bun_aware_candidates() {
        let expected = [
            "typescript-language-server --stdio",
            "bunx typescript-language-server --stdio",
        ];
        assert_eq!(candidates("typescript"), expected);
        assert_eq!(candidates("javascript"), expected);
        assert!(candidates("shellscript").is_empty());
    }

    #[test]
    fn native_package_name_maps_platform() {
        let name = native_typescript_package();
        assert!(name.starts_with("typescript-"));
        assert!(!name.contains("x86_64"));
        assert!(!name.contains("aarch64"));
        assert!(!name.contains("macos"));
    }

    #[test]
    fn command_available_checks_path() {
        assert!(command_available("sh"));
        assert!(command_available("sh -c foo"));
        assert!(command_available("/bin/sh"));
        assert!(!command_available("kanso-no-such-binary-xyz"));
        assert!(!command_available(""));
    }

    #[test]
    fn expand_home_replaces_tilde_prefix() {
        let expanded = expand_home("~/.bun/bin/x --stdio");
        assert!(!expanded.starts_with("~/"));
        assert!(expanded.ends_with("/.bun/bin/x --stdio"));
        assert_eq!(expand_home("plain --stdio"), "plain --stdio");
    }

    #[test]
    fn unknown_language_has_no_default() {
        assert_eq!(default_server("nope", Path::new("/tmp/x.zzz")), None);
    }
}
