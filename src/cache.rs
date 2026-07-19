//! (mtime, size)-validated memoization of what the web listing derives from
//! file content. Only `serve` constructs one; CLI paths stay parse-fresh by
//! never holding a cache.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use crate::{
    deck::{Deck, DeckError, DeckSettings},
    picker,
    workspace::{self, Workspace},
};

#[derive(Default)]
pub struct DeckCache {
    entries: HashMap<PathBuf, Entry>,
}

struct Entry {
    mtime: SystemTime,
    size: u64,
    is_deck: Option<bool>,
    label: Option<Option<String>>,
    deck: Option<Result<Arc<Deck>, Arc<DeckError>>>,
    manifest: Option<ManifestMeta>,
}

impl Entry {
    fn empty(mtime: SystemTime, size: u64) -> Self {
        Entry {
            mtime,
            size,
            is_deck: None,
            label: None,
            deck: None,
            manifest: None,
        }
    }
}

#[derive(Clone)]
struct ManifestMeta {
    title: Option<String>,
    description: Option<String>,
    settings: DeckSettings,
    icon: Option<String>,
}

impl DeckCache {
    fn slot(&mut self, path: &Path) -> Option<&mut Entry> {
        let Ok(meta) = std::fs::metadata(path) else {
            self.entries.remove(path);
            return None;
        };
        let mtime = meta.modified().ok()?;
        let size = meta.len();
        let entry = self
            .entries
            .entry(path.to_path_buf())
            .or_insert_with(|| Entry::empty(mtime, size));
        if (entry.mtime, entry.size) != (mtime, size) {
            *entry = Entry::empty(mtime, size);
        }
        Some(entry)
    }

    pub fn is_deck(&mut self, path: &Path) -> bool {
        match self.slot(path) {
            Some(entry) => *entry
                .is_deck
                .get_or_insert_with(|| workspace::file_is_deck(path)),
            None => workspace::file_is_deck(path),
        }
    }

    pub fn label(&mut self, path: &Path) -> Option<String> {
        match self.slot(path) {
            Some(entry) => entry
                .label
                .get_or_insert_with(|| picker::deck_label(path))
                .clone(),
            None => picker::deck_label(path),
        }
    }

    pub fn load(&mut self, path: &Path) -> Result<Arc<Deck>, Arc<DeckError>> {
        match self.slot(path) {
            Some(entry) => entry
                .deck
                .get_or_insert_with(|| Deck::load(path).map(Arc::new).map_err(Arc::new))
                .clone(),
            None => Deck::load(path).map(Arc::new).map_err(Arc::new),
        }
    }

    /// The readdir itself is deliberately never memoized (only each member's
    /// content check is): a new or deleted file must show up immediately.
    pub fn members(&mut self, dir: &Path) -> Vec<PathBuf> {
        workspace::members_where(dir, |p| self.is_deck(p)).unwrap_or_default()
    }

    pub fn has_decks(&mut self, path: &Path) -> bool {
        path.is_dir() && !self.members(path).is_empty()
    }

    pub fn is_workspace(&mut self, path: &Path) -> bool {
        path.join(workspace::MANIFEST).is_file() && self.has_decks(path)
    }

    pub fn workspace(&mut self, dir: &Path) -> Workspace {
        let members = self.members(dir);
        let meta = self.manifest_meta(&dir.join(workspace::MANIFEST));
        let icon = workspace::resolve_icon(dir, meta.icon.as_deref());
        Workspace {
            path: dir.to_path_buf(),
            title: meta.title,
            description: meta.description,
            settings: meta.settings,
            members,
            icon,
        }
    }

    fn manifest_meta(&mut self, path: &Path) -> ManifestMeta {
        match self.slot(path) {
            Some(entry) => entry
                .manifest
                .get_or_insert_with(|| read_manifest_meta(path))
                .clone(),
            None => read_manifest_meta(path),
        }
    }
}

fn read_manifest_meta(path: &Path) -> ManifestMeta {
    let (title, description, settings, icon) = workspace::read_manifest(path);
    ManifestMeta {
        title,
        description,
        settings,
        icon,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    fn set_mtime(path: &Path, mtime: SystemTime) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(mtime).unwrap();
    }

    #[test]
    fn an_unchanged_mtime_and_size_serves_every_derivation_from_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        write(&path, "# Old Title\n\n## q\na\n");
        let mut cache = DeckCache::default();
        assert!(cache.is_deck(&path));
        assert_eq!(Some("Old Title".to_string()), cache.label(&path));
        let first = cache.load(&path).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let (mtime, size) = (meta.modified().unwrap(), meta.len());

        write(&path, &"z".repeat(size as usize));
        set_mtime(&path, mtime);

        assert!(cache.is_deck(&path));
        assert_eq!(Some("Old Title".to_string()), cache.label(&path));
        assert!(Arc::ptr_eq(&first, &cache.load(&path).unwrap()));
    }

    #[test]
    fn a_size_change_invalidates_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        write(&path, "# Old Title\n\n## q\na\n");
        let mut cache = DeckCache::default();
        assert_eq!(Some("Old Title".to_string()), cache.label(&path));

        write(&path, "# A New Title Grown Longer\n\n## q\na\n");

        assert_eq!(
            Some("A New Title Grown Longer".to_string()),
            cache.label(&path)
        );
    }

    #[test]
    fn an_mtime_change_with_equal_size_invalidates_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        write(&path, "# Title A\n\n## q\na\n");
        let mut cache = DeckCache::default();
        assert_eq!(Some("Title A".to_string()), cache.label(&path));
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();

        write(&path, "# Title B\n\n## q\na\n");
        set_mtime(&path, mtime + Duration::from_secs(1));

        assert_eq!(Some("Title B".to_string()), cache.label(&path));
    }

    #[test]
    fn members_sees_a_new_file_immediately_and_still_caches_content_checks() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a.md"), "## q\na\n");
        let mut cache = DeckCache::default();
        assert_eq!(vec![dir.path().join("a.md")], cache.members(dir.path()));

        write(&dir.path().join("b.md"), "## q2\nb\n");

        assert_eq!(
            vec![dir.path().join("a.md"), dir.path().join("b.md")],
            cache.members(dir.path())
        );
    }

    #[test]
    fn a_vanished_file_is_answered_uncached() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.md");
        write(&path, "## q\na\n");
        let mut cache = DeckCache::default();
        assert!(cache.is_deck(&path));

        std::fs::remove_file(&path).unwrap();

        assert!(cache.load(&path).is_err());
        assert_eq!(None, cache.label(&path));
    }
}
