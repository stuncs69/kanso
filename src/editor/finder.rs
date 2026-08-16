use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use super::prompt::Prompt;
use super::search::fold;

const MAX_FILES: usize = 20_000;
const MAX_DEPTH: usize = 24;
const MAX_HITS: usize = 200;

const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "vendor",
    "venv",
    ".venv",
    "__pycache__",
];

#[derive(Debug, Clone)]
pub struct Hit {
    pub index: usize,
    pub score: i32,
    pub positions: Vec<usize>,
}

pub struct Finder {
    pub root: PathBuf,
    pub query: Prompt,
    pub files: Vec<String>,
    pub hits: Vec<Hit>,
    pub selected: usize,
    pub truncated: bool,
}

impl Finder {
    pub fn open(start: &Path) -> io::Result<Finder> {
        let root = project_root(start);
        let ignore = Ignore::for_root(&root);
        let (files, truncated) = walk(&root, &ignore);
        let mut finder = Finder {
            root,
            query: Prompt::default(),
            files,
            hits: Vec::new(),
            selected: 0,
            truncated,
        };
        finder.refresh();
        Ok(finder)
    }

    pub fn refresh(&mut self) {
        let query: Vec<char> = self.query.chars().iter().copied().map(fold).collect();
        let mut hits: Vec<Hit> = Vec::new();
        for (index, path) in self.files.iter().enumerate() {
            let candidate: Vec<char> = path.chars().collect();
            let Some((score, positions)) = fuzzy_match(&candidate, &query) else {
                continue;
            };
            hits.push(Hit {
                index,
                score,
                positions,
            });
        }
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| self.files[a.index].len().cmp(&self.files[b.index].len()))
                .then_with(|| self.files[a.index].cmp(&self.files[b.index]))
        });
        hits.truncate(MAX_HITS);
        self.hits = hits;
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.hits.is_empty() {
            self.selected = 0;
            return;
        }
        let last = self.hits.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    pub fn hit_path(&self, hit: &Hit) -> &str {
        &self.files[hit.index]
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        let hit = self.hits.get(self.selected)?;
        Some(self.root.join(self.hit_path(hit)))
    }

    pub fn counter(&self) -> String {
        let total = self.files.len();
        let suffix = if self.truncated { "+" } else { "" };
        format!("{}/{total}{suffix}", self.hits.len())
    }
}

pub fn project_root(start: &Path) -> PathBuf {
    let base = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let base = base
        .canonicalize()
        .or_else(|_| std::env::current_dir())
        .unwrap_or_else(|_| PathBuf::from("."));
    let mut dir = base.as_path();
    loop {
        if dir.join(".git").exists() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return base,
        }
    }
}

struct Ignore {
    names: HashSet<String>,
    extensions: HashSet<String>,
}

impl Ignore {
    fn for_root(root: &Path) -> Ignore {
        let mut ignore = Ignore {
            names: SKIP_DIRS.iter().map(|s| s.to_string()).collect(),
            extensions: HashSet::new(),
        };
        if let Ok(text) = std::fs::read_to_string(root.join(".gitignore")) {
            ignore.extend_from_gitignore(&text);
        }
        ignore
    }

    fn extend_from_gitignore(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                continue;
            }
            let pattern = line.trim_start_matches('/').trim_end_matches('/');
            if let Some(ext) = pattern.strip_prefix("*.") {
                if !ext.contains(['*', '/', '?']) {
                    self.extensions.insert(ext.to_string());
                }
            } else if !pattern.contains(['*', '/', '?', '[']) && !pattern.is_empty() {
                self.names.insert(pattern.to_string());
            }
        }
    }

    fn skips(&self, name: &str) -> bool {
        if name.starts_with('.') || self.names.contains(name) {
            return true;
        }
        match name.rsplit_once('.') {
            Some((_, ext)) => self.extensions.contains(ext),
            None => false,
        }
    }
}

fn walk(root: &Path, ignore: &Ignore) -> (Vec<String>, bool) {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new(), 0usize)];
    let mut truncated = false;
    while let Some((dir, prefix, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if ignore.skips(&name) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((entry.path(), relative, depth + 1));
                }
            } else if kind.is_file() {
                if files.len() >= MAX_FILES {
                    truncated = true;
                    stack.clear();
                    break;
                }
                files.push(relative);
            }
        }
    }
    files.sort();
    (files, truncated)
}

pub fn fuzzy_match(candidate: &[char], folded_query: &[char]) -> Option<(i32, Vec<usize>)> {
    let query = folded_query;
    if query.is_empty() {
        return Some((-(candidate.len() as i32), Vec::new()));
    }
    let mut qi = 0;
    let mut end = None;
    for (i, c) in candidate.iter().enumerate() {
        if fold(*c) == query[qi] {
            qi += 1;
            if qi == query.len() {
                end = Some(i);
                break;
            }
        }
    }
    let end = end?;

    let mut positions = vec![0usize; query.len()];
    let mut qi = query.len();
    let mut i = end + 1;
    while i > 0 && qi > 0 {
        i -= 1;
        if fold(candidate[i]) == query[qi - 1] {
            qi -= 1;
            positions[qi] = i;
        }
    }
    Some((score(candidate, &positions), positions))
}

fn score(candidate: &[char], positions: &[usize]) -> i32 {
    let name_start = candidate
        .iter()
        .rposition(|c| *c == '/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut total = 0i32;
    let mut previous: Option<usize> = None;
    for position in positions {
        let at = *position;
        if previous == Some(at.saturating_sub(1)) && at > 0 {
            total += 8;
        }
        if at == 0 || is_boundary(candidate, at) {
            total += 6;
        }
        if at >= name_start {
            total += 10;
        }
        if let Some(previous) = previous {
            total -= (at - previous - 1).min(8) as i32;
        }
        previous = Some(at);
    }
    total -= (candidate.len() / 8) as i32;
    total -= (positions[0] / 4) as i32;
    total
}

fn is_boundary(candidate: &[char], at: usize) -> bool {
    if at == 0 {
        return true;
    }
    let previous = candidate[at - 1];
    if matches!(previous, '/' | '_' | '-' | '.' | ' ') {
        return true;
    }
    previous.is_lowercase() && candidate[at].is_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn match_of(candidate: &str, query: &str) -> Option<(i32, Vec<usize>)> {
        let folded: Vec<char> = query.chars().map(fold).collect();
        fuzzy_match(&chars(candidate), &folded)
    }

    fn rank(paths: &[&str], query: &str) -> Vec<String> {
        let mut scored: Vec<(i32, &str)> = paths
            .iter()
            .filter_map(|p| match_of(p, query).map(|(score, _)| (score, *p)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
        scored.into_iter().map(|(_, p)| p.to_string()).collect()
    }

    #[test]
    fn matches_subsequences_case_insensitively() {
        assert!(match_of("src/editor/state.rs", "state").is_some());
        assert!(match_of("src/editor/state.rs", "SRCST").is_some());
        assert!(match_of("src/editor/state.rs", "zzz").is_none());
        assert!(match_of("abc", "abcd").is_none());
    }

    #[test]
    fn reports_positions_of_the_tightest_match() {
        let (_, positions) = match_of("src/state.rs", "state").unwrap();
        assert_eq!(positions, vec![4, 5, 6, 7, 8]);
    }

    #[test]
    fn empty_query_matches_everything_and_prefers_short_paths() {
        let ranked = rank(&["a/b/c/long.rs", "x.rs"], "");
        assert_eq!(ranked[0], "x.rs");
    }

    #[test]
    fn filename_matches_outrank_directory_matches() {
        let ranked = rank(&["state/other/thing.rs", "src/editor/state.rs"], "state");
        assert_eq!(ranked[0], "src/editor/state.rs");
    }

    #[test]
    fn consecutive_matches_outrank_scattered_ones() {
        let ranked = rank(&["s_o_m_e_t_h_i_n_g.rs", "something.rs"], "something");
        assert_eq!(ranked[0], "something.rs");
    }

    #[test]
    fn word_boundaries_are_rewarded() {
        let ranked = rank(&["mybadinput.rs", "my_bad_input.rs"], "bi");
        assert_eq!(ranked[0], "my_bad_input.rs");
    }

    fn temp_tree(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kanso-finder-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "").unwrap();
        std::fs::write(dir.join("target/junk.rs"), "").unwrap();
        std::fs::write(dir.join(".git/config"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        dir
    }

    #[test]
    fn walk_skips_hidden_and_build_directories() {
        let dir = temp_tree("walk");
        let finder = Finder::open(&dir).unwrap();
        assert_eq!(
            finder.files,
            vec![
                "notes.txt".to_string(),
                "src/lib.rs".to_string(),
                "src/main.rs".to_string()
            ]
        );
        assert!(!finder.truncated);
    }

    #[test]
    fn gitignore_names_and_extensions_are_skipped() {
        let dir = temp_tree("gitignore");
        std::fs::write(dir.join(".gitignore"), "notes.txt\n*.rs\n# comment\n").unwrap();
        let finder = Finder::open(&dir).unwrap();
        assert!(finder.files.is_empty());
    }

    #[test]
    fn typing_filters_and_selection_clamps() {
        let dir = temp_tree("filter");
        let mut finder = Finder::open(&dir).unwrap();
        assert_eq!(finder.hits.len(), 3);
        finder.query.insert_text("main");
        finder.refresh();
        assert_eq!(finder.hits.len(), 1);
        assert_eq!(finder.hit_path(&finder.hits[0]), "src/main.rs");
        assert_eq!(
            finder.selected_path(),
            Some(dir.canonicalize().unwrap().join("src/main.rs"))
        );
        finder.move_selection(10);
        assert_eq!(finder.selected, 0);
        finder.query.insert_text("zzz");
        finder.refresh();
        assert!(finder.hits.is_empty());
        assert!(finder.selected_path().is_none());
    }

    #[test]
    fn root_is_the_enclosing_git_repository() {
        let dir = temp_tree("root");
        let root = project_root(&dir.join("src/main.rs"));
        assert_eq!(root, dir.canonicalize().unwrap());
    }
}
