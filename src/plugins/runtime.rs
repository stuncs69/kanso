use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDir {
    pub name: String,
    pub entry: PathBuf,
}

pub fn discover(dir: &Path) -> (Vec<PluginDir>, Vec<String>) {
    let mut found = Vec::new();
    let mut warnings = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (found, warnings),
        Err(e) => {
            warnings.push(format!("plugins: {}: {e}", dir.display()));
            return (found, warnings);
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        let init = path.join("init.lua");
        if init.is_file() {
            found.push(PluginDir { name, entry: init });
        } else {
            warnings.push(format!("plugin `{name}`: no init.lua"));
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    (found, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("kanso-plugins-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn plugin(&self, name: &str, source: &str) {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("init.lua"), source).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        let (found, warnings) = discover(Path::new("/nonexistent/kanso-plugins"));
        assert!(found.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn finds_plugins_sorted_and_warns_about_incomplete_ones() {
        let temp = TempDir::new("discover");
        temp.plugin("zeta", "");
        temp.plugin("alpha", "");
        std::fs::create_dir_all(temp.0.join("empty")).unwrap();
        std::fs::write(temp.0.join("loose.lua"), "").unwrap();

        let (found, warnings) = discover(&temp.0);
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta"]);
        assert!(found[0].entry.ends_with("alpha/init.lua"));
        assert_eq!(warnings, ["plugin `empty`: no init.lua"]);
    }
}
