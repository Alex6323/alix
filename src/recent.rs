use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const CAP: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub last_used_ms: u64,
}

#[derive(Default)]
pub struct RecentDecks {
    path: PathBuf,
    entries: Vec<RecentEntry>,
}

impl RecentDecks {
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<RecentEntry>>(&t).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    pub fn entries(&self) -> &[RecentEntry] {
        &self.entries
    }

    pub fn record(&mut self, paths: &[PathBuf], now_ms: u64) {
        // Insert in reverse so the first given path ends up frontmost.
        for path in paths.iter().rev() {
            self.entries.retain(|e| e.path != *path);
            self.entries.insert(
                0,
                RecentEntry {
                    path: path.clone(),
                    last_used_ms: now_ms,
                },
            );
        }
        self.entries.truncate(CAP);
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(&self.entries).expect("recent entries serialize");
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)
    }
}

pub fn default_recent_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix").map(|dirs| dirs.data_dir().join("recent.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_moves_to_front_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = RecentDecks::load(dir.path().join("recent.json"));
        r.record(&[PathBuf::from("a.txt")], 100);
        r.record(&[PathBuf::from("b.txt")], 200);
        r.record(&[PathBuf::from("a.txt")], 300); // a moves back to front

        let paths: Vec<_> = r.entries().iter().map(|e| e.path.clone()).collect();
        assert_eq!(vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")], paths);
        assert_eq!(300, r.entries()[0].last_used_ms);
    }

    #[test]
    fn record_multi_keeps_given_order_frontmost() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = RecentDecks::load(dir.path().join("recent.json"));
        r.record(&[PathBuf::from("x.txt"), PathBuf::from("y.txt")], 1);
        let paths: Vec<_> = r.entries().iter().map(|e| e.path.clone()).collect();
        assert_eq!(vec![PathBuf::from("x.txt"), PathBuf::from("y.txt")], paths);
    }

    #[test]
    fn list_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = RecentDecks::load(dir.path().join("recent.json"));
        for i in 0..(CAP + 10) {
            r.record(&[PathBuf::from(format!("{i}.txt"))], i as u64);
        }
        assert_eq!(CAP, r.entries().len());
        assert_eq!(
            PathBuf::from(format!("{}.txt", CAP + 9)),
            r.entries()[0].path
        );
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recent.json");
        let mut r = RecentDecks::load(&path);
        r.record(&[PathBuf::from("deck.txt")], 42);
        r.save().unwrap();

        let r2 = RecentDecks::load(&path);
        assert_eq!(1, r2.entries().len());
        assert_eq!(PathBuf::from("deck.txt"), r2.entries()[0].path);
        assert_eq!(42, r2.entries()[0].last_used_ms);
    }

    #[test]
    fn corrupt_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recent.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(RecentDecks::load(&path).entries().is_empty());
    }
}
