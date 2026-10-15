use std::io;
use std::path::{Path, PathBuf};

pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    path.canonicalize().map(strip_verbatim)
}

pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        if let Some(text) = path.to_str() {
            if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{rest}"));
            }
            if let Some(rest) = text.strip_prefix(r"\\?\") {
                if rest.as_bytes().get(1) == Some(&b':') {
                    return PathBuf::from(rest);
                }
            }
        }
    }
    path
}

pub fn contract_home(text: &str) -> String {
    if let Some(dirs) = directories::BaseDirs::new() {
        let home = dirs.home_dir().display().to_string();
        if let Some(rest) = text.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    text.to_string()
}

pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn truncate_left(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max.saturating_sub(1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_variants() {
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("hello", 3), "he…");
        assert_eq!(truncate_left("/a/b/c/d", 5), "…/c/d");
        assert_eq!(truncate_left("abc", 5), "abc");
    }

    #[test]
    fn home_contracts_to_tilde() {
        let home = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .display()
            .to_string();
        assert_eq!(contract_home(&format!("{home}/x")), "~/x");
        assert_eq!(contract_home("/usr/bin"), "/usr/bin");
    }

    #[test]
    fn canonical_paths_have_no_verbatim_prefix() {
        let dir = canonicalize(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        assert!(!dir.to_string_lossy().starts_with(r"\\?\"));
        assert!(dir.join("Cargo.toml").exists());
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_prefixes_are_stripped() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\D:\dev")),
            PathBuf::from(r"D:\dev")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\Volume{abc}\x")),
            PathBuf::from(r"\\?\Volume{abc}\x")
        );
    }
}
