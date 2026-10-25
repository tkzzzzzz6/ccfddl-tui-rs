//! Favorites persistence. Uses the same file and format as the Python version
//! (`~/.ccfddl/favorites.json` with `{"favorites": ["Title_Year", ...]}`),
//! so both implementations share the same favorites.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde_json::{Value, json};

pub fn favorite_id(title: &str, year: i32) -> String {
    format!("{title}_{year}")
}

/// Directory holding favorites and the data cache (`~/.ccfddl`).
pub fn app_dir() -> PathBuf {
    BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".ccfddl"))
        .unwrap_or_else(|| PathBuf::from(".ccfddl"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleOutcome {
    pub is_favorite: bool,
    /// Set when the in-memory change could not be written to disk.
    pub save_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Favorites {
    path: PathBuf,
    ids: BTreeSet<String>,
}

impl Favorites {
    pub fn default_path() -> PathBuf {
        app_dir().join("favorites.json")
    }

    /// Empty favorites that will be saved to `path`; nothing is read from disk.
    pub fn empty_at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            ids: BTreeSet::new(),
        }
    }

    /// Load from `path`. A missing, corrupted or malformed file yields no favorites.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let ids = read_ids(&path);
        Self { path, ids }
    }

    pub fn load_default() -> Self {
        Self::load(Self::default_path())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, title: &str, year: i32) -> bool {
        self.ids.contains(&favorite_id(title, year))
    }

    pub fn toggle(&mut self, title: &str, year: i32) -> ToggleOutcome {
        let id = favorite_id(title, year);
        let is_favorite = if self.ids.remove(&id) {
            false
        } else {
            self.ids.insert(id);
            true
        };
        let save_error = self.save().err().map(|e| {
            log::warn!("Failed to save favorites: {e}");
            e.to_string()
        });
        ToggleOutcome {
            is_favorite,
            save_error,
        }
    }

    pub fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let ids: Vec<&String> = self.ids.iter().collect();
        let body = serde_json::to_string(&json!({ "favorites": ids }))?;

        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, body)?;
        fs::rename(&tmp, &self.path)
    }
}

fn read_ids(path: &Path) -> BTreeSet<String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return BTreeSet::new(),
        Err(e) => {
            log::warn!("Failed to read favorites file: {e}");
            return BTreeSet::new();
        }
    };
    let data: Value = match serde_json::from_str(&content) {
        Ok(data) => data,
        Err(e) => {
            log::warn!("Favorites file corrupted: {e}");
            return BTreeSet::new();
        }
    };
    match data.get("favorites").and_then(Value::as_array) {
        Some(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        None => {
            log::warn!("Favorites data is not a list");
            BTreeSet::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn toggle_adds_then_removes() {
        let dir = tempdir().unwrap();
        let mut fav = Favorites::empty_at(dir.path().join("favorites.json"));

        let added = fav.toggle("CVPR", 2025);
        assert!(added.is_favorite);
        assert_eq!(added.save_error, None);
        assert!(fav.contains("CVPR", 2025));
        assert!(!fav.contains("CVPR", 2024));

        let removed = fav.toggle("CVPR", 2025);
        assert!(!removed.is_favorite);
        assert!(!fav.contains("CVPR", 2025));
    }

    #[test]
    fn persists_in_python_compatible_format() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("favorites.json");
        let mut fav = Favorites::empty_at(&path);
        fav.toggle("ICML", 2025);

        let data: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data["favorites"], json!(["ICML_2025"]));
        assert!(!path.with_extension("json.tmp").exists());

        let reloaded = Favorites::load(&path);
        assert!(reloaded.contains("ICML", 2025));
    }

    #[test]
    fn loads_python_written_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("favorites.json");
        fs::write(
            &path,
            r#"{"favorites": ["NeurIPS_2025", "ICML_2025", 42, null]}"#,
        )
        .unwrap();

        let fav = Favorites::load(&path);
        assert_eq!(fav.len(), 2);
        assert!(fav.contains("NeurIPS", 2025));
        assert!(fav.contains("ICML", 2025));
    }

    #[test]
    fn unicode_ids_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("favorites.json");
        let mut fav = Favorites::empty_at(&path);
        fav.toggle("会议", 2026);
        assert!(Favorites::load(&path).contains("会议", 2026));
    }

    #[test]
    fn bad_files_yield_empty_favorites() {
        let dir = tempdir().unwrap();
        assert!(Favorites::load(dir.path().join("missing.json")).is_empty());

        for (name, body) in [
            ("corrupt.json", "{not json"),
            ("wrong_type.json", r#"{"favorites": "oops"}"#),
            ("no_key.json", r#"{"other": []}"#),
            ("not_object.json", "[1, 2]"),
            ("empty.json", ""),
        ] {
            let path = dir.path().join(name);
            fs::write(&path, body).unwrap();
            assert!(Favorites::load(&path).is_empty(), "{name}");
        }
    }

    #[test]
    fn save_failure_is_reported_but_state_kept() {
        let dir = tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "a file, not a directory").unwrap();

        let mut fav = Favorites::empty_at(blocker.join("favorites.json"));
        let outcome = fav.toggle("CVPR", 2025);
        assert!(outcome.is_favorite);
        assert!(outcome.save_error.is_some());
        assert!(fav.contains("CVPR", 2025));
    }
}
