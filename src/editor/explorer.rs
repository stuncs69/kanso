use std::io;
use std::path::{Path, PathBuf};

pub struct Explorer {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

impl Explorer {
    pub fn open(dir: &Path) -> io::Result<Explorer> {
        let dir = dir.canonicalize()?;
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let Ok(entry) = entry else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                dirs.push(Entry { name, is_dir: true });
            } else {
                files.push(Entry {
                    name,
                    is_dir: false,
                });
            }
        }
        dirs.sort_by_key(|e| e.name.to_lowercase());
        files.sort_by_key(|e| e.name.to_lowercase());
        let mut entries = Vec::with_capacity(dirs.len() + files.len() + 1);
        if dir.parent().is_some() {
            entries.push(Entry {
                name: "..".to_string(),
                is_dir: true,
            });
        }
        entries.extend(dirs);
        entries.extend(files);
        Ok(Explorer {
            dir,
            entries,
            selected: 0,
        })
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    pub fn select_name(&mut self, name: &str) {
        if let Some(i) = self.entries.iter().position(|e| e.name == name) {
            self.selected = i;
        }
    }

    pub fn jump_to_prefix(&mut self, c: char) {
        if self.entries.is_empty() {
            return;
        }
        let target = c.to_lowercase().next().unwrap_or(c);
        let len = self.entries.len();
        for step in 1..=len {
            let i = (self.selected + step) % len;
            let starts = self.entries[i]
                .name
                .chars()
                .next()
                .and_then(|f| f.to_lowercase().next())
                == Some(target);
            if starts {
                self.selected = i;
                return;
            }
        }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tree(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kanso-explorer-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("beta.txt"), "b").unwrap();
        std::fs::write(dir.join("alpha.txt"), "a").unwrap();
        dir
    }

    #[test]
    fn lists_dirs_first_then_files_sorted() {
        let dir = temp_tree("list");
        let explorer = Explorer::open(&dir).unwrap();
        let names: Vec<&str> = explorer.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["..", "sub", "alpha.txt", "beta.txt"]);
        assert!(explorer.entries[1].is_dir);
        assert!(!explorer.entries[2].is_dir);
    }

    #[test]
    fn selection_moves_and_clamps() {
        let dir = temp_tree("select");
        let mut explorer = Explorer::open(&dir).unwrap();
        explorer.move_selection(100);
        assert_eq!(explorer.selected, 3);
        explorer.move_selection(-1);
        assert_eq!(explorer.selected, 2);
        explorer.move_selection(-100);
        assert_eq!(explorer.selected, 0);
    }

    #[test]
    fn prefix_jump_wraps() {
        let dir = temp_tree("jump");
        let mut explorer = Explorer::open(&dir).unwrap();
        explorer.jump_to_prefix('b');
        assert_eq!(explorer.selected_entry().unwrap().name, "beta.txt");
        explorer.jump_to_prefix('a');
        assert_eq!(explorer.selected_entry().unwrap().name, "alpha.txt");
    }

    #[test]
    fn select_name_finds_entry() {
        let dir = temp_tree("name");
        let mut explorer = Explorer::open(&dir).unwrap();
        explorer.select_name("sub");
        assert_eq!(explorer.selected, 1);
    }
}
