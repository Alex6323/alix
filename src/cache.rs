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

#[derive(Clone, Default)]
pub struct DeckCache {
    entries: HashMap<PathBuf, Entry>,
}

#[derive(Clone)]
struct Entry {
    mtime: SystemTime,
    size: u64,
    is_deck: Option<bool>,
    label: Option<Option<String>>,
    deck: Option<CachedDeck>,
    manifest: Option<ManifestMeta>,
}

/// A member's exam state and its card list both depend on its workspace
/// manifest, a second file the deck's own (mtime, size) cannot see.
#[derive(Clone)]
struct CachedDeck {
    workspace_has_sources: bool,
    settings: DeckSettings,
    deck: Result<Arc<Deck>, Arc<DeckError>>,
}

impl CachedDeck {
    fn load(path: &Path, settings: DeckSettings, workspace_has_sources: bool) -> Self {
        let deck = Deck::load_in_workspace(path, &settings, workspace_has_sources)
            .map(Arc::new)
            .map_err(Arc::new);
        CachedDeck {
            workspace_has_sources,
            settings,
            deck,
        }
    }
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
    source: Vec<String>,
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
        let workspace_has_sources = self.workspace_has_sources(path);
        let settings = self.workspace_settings(path);
        match self.slot(path) {
            Some(entry) => {
                if entry.deck.as_ref().is_some_and(|cached| {
                    cached.workspace_has_sources != workspace_has_sources
                        || cached.settings != settings
                }) {
                    entry.deck = None;
                }
                entry
                    .deck
                    .get_or_insert_with(|| CachedDeck::load(path, settings, workspace_has_sources))
                    .deck
                    .clone()
            }
            None => CachedDeck::load(path, settings, workspace_has_sources).deck,
        }
    }

    fn workspace_settings(&mut self, deck: &Path) -> DeckSettings {
        let manifest = workspace::content_root(deck).join(workspace::MANIFEST);
        match self.slot(&manifest) {
            Some(entry) => entry
                .manifest
                .get_or_insert_with(|| read_manifest_meta(&manifest))
                .settings
                .clone(),
            None => DeckSettings::default(),
        }
    }

    fn workspace_has_sources(&mut self, deck: &Path) -> bool {
        let manifest = workspace::content_root(deck).join(workspace::MANIFEST);
        match self.slot(&manifest) {
            Some(entry) => !entry
                .manifest
                .get_or_insert_with(|| read_manifest_meta(&manifest))
                .source
                .is_empty(),
            None => false,
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
        workspace::has_manifest(path)
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
            source: meta.source,
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
    let (title, description, settings, icon, source) = workspace::read_manifest(path);
    ManifestMeta {
        title,
        description,
        settings,
        source,
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
        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d\"\ntitle: Old Title\n---\n## q\na\n",
        );
        let mut cache = DeckCache::default();
        assert!(cache.is_deck(&path));
        assert_eq!(Some("Old Title".to_string()), cache.label(&path));
        let first = cache.load(&path).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let (mtime, size) = (meta.modified().unwrap(), meta.len());

        std::fs::write(&path, "z".repeat(size as usize)).unwrap();
        set_mtime(&path, mtime);

        assert!(cache.is_deck(&path));
        assert_eq!(Some("Old Title".to_string()), cache.label(&path));
        assert!(Arc::ptr_eq(&first, &cache.load(&path).unwrap()));
    }

    #[test]
    fn a_size_change_invalidates_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d\"\ntitle: Old Title\n---\n## q\na\n",
        );
        let mut cache = DeckCache::default();
        assert_eq!(Some("Old Title".to_string()), cache.label(&path));

        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d\"\ntitle: A New Title Grown Longer\n---\n## q\na\n",
        );

        assert_eq!(
            Some("A New Title Grown Longer".to_string()),
            cache.label(&path)
        );
    }

    #[test]
    fn an_mtime_change_with_equal_size_invalidates_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d\"\ntitle: Title A\n---\n## q\na\n",
        );
        let mut cache = DeckCache::default();
        assert_eq!(Some("Title A".to_string()), cache.label(&path));
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();

        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d\"\ntitle: Title B\n---\n## q\na\n",
        );
        set_mtime(&path, mtime + Duration::from_secs(1));

        assert_eq!(Some("Title B".to_string()), cache.label(&path));
    }

    #[test]
    fn members_sees_a_new_file_immediately_and_still_caches_content_checks() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("a.md"),
            "---\nformat-version: 1\nid: \"deck-a\"\n---\n## q\na\n",
        );
        let mut cache = DeckCache::default();
        assert_eq!(vec![dir.path().join("a.md")], cache.members(dir.path()));

        write(
            &dir.path().join("b.md"),
            "---\nformat-version: 1\nid: \"deck-b\"\n---\n## q2\nb\n",
        );

        assert_eq!(
            vec![dir.path().join("a.md"), dir.path().join("b.md")],
            cache.members(dir.path())
        );
    }

    #[test]
    fn a_vanished_file_is_answered_uncached() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.md");
        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-gone\"\n---\n## q\na\n",
        );
        let mut cache = DeckCache::default();
        assert!(cache.is_deck(&path));

        std::fs::remove_file(&path).unwrap();

        assert!(cache.load(&path).is_err());
        assert_eq!(None, cache.label(&path));
    }

    #[test]
    fn has_decks_requires_both_a_directory_and_a_member() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plain.txt");
        write(&file, "not a directory");
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        let populated = dir.path().join("populated");
        std::fs::create_dir(&populated).unwrap();
        let deck = populated.join("deck.md");
        write(&deck, "## front\nback\n");
        crate::stamp::stamp_deck(&deck).unwrap();
        let mut cache = DeckCache::default();

        assert!(!cache.has_decks(&file));
        assert!(!cache.has_decks(&empty));
        assert!(cache.has_decks(&populated));
    }

    #[test]
    fn a_manifest_source_edit_refreshes_a_cached_decks_exam_state() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let decks = workspace.join("decks");
        std::fs::create_dir_all(&decks).unwrap();
        write(&workspace.join("alix.toml"), "");
        let path = decks.join("d.md");
        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d\"\n---\n## q\na\n",
        );
        let mut cache = DeckCache::default();
        let first = cache.load(&path).unwrap();
        assert!(!first.has_exam(), "an unsourced workspace has no exam");

        write(&workspace.join("alix.toml"), "source = \"notes.md\"\n");
        assert_eq!(
            vec!["notes.md".to_string()],
            cache.workspace(&workspace).source,
            "the same server cache sees the edited manifest"
        );
        assert!(
            cache.load(&path).unwrap().has_exam(),
            "adding a workspace source must make its cached member exam-capable"
        );
    }

    #[test]
    fn removing_the_workspace_source_takes_the_exam_from_its_cached_member() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let decks = workspace.join("decks");
        std::fs::create_dir_all(&decks).unwrap();
        write(&workspace.join("alix.toml"), "source = \"notes.md\"\n");
        let path = decks.join("d.md");
        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d\"\n---\n## q\na\n",
        );
        let mut cache = DeckCache::default();
        assert!(cache.load(&path).unwrap().has_exam(), "sourced: exam");

        write(&workspace.join("alix.toml"), "");

        assert!(
            !cache.load(&path).unwrap().has_exam(),
            "removing the workspace source must take the exam from its cached member"
        );
    }

    #[test]
    fn a_manifest_edit_that_keeps_the_source_leaves_the_member_cached() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let decks = workspace.join("decks");
        std::fs::create_dir_all(&decks).unwrap();
        write(&workspace.join("alix.toml"), "source = \"notes.md\"\n");
        let path = decks.join("d.md");
        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d\"\n---\n## q\na\n",
        );
        let mut cache = DeckCache::default();
        let first = cache.load(&path).unwrap();

        write(
            &workspace.join("alix.toml"),
            "title = \"Renamed\"\nsource = \"notes.md\"\n",
        );
        assert_eq!(
            Some("Renamed".to_string()),
            cache.workspace(&workspace).title,
            "the same server cache sees the edited manifest"
        );
        let second = cache.load(&path).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "a manifest edit that keeps the source must not re-parse the member"
        );
        assert!(second.has_exam(), "the source is still there");
    }

    #[test]
    fn a_manifest_defaults_edit_refreshes_a_cached_decks_card_list() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let decks = workspace.join("decks");
        std::fs::create_dir_all(&decks).unwrap();
        write(&workspace.join("alix.toml"), "");
        let path = decks.join("d.md");
        write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d\"\n---\n## q\na\n",
        );
        let mut cache = DeckCache::default();
        assert_eq!(
            1,
            cache.load(&path).unwrap().cards.len(),
            "one authored card, no workspace default"
        );

        write(
            &workspace.join("alix.toml"),
            "[defaults]\ndirection = \"both\"\n",
        );

        assert_eq!(
            2,
            cache.load(&path).unwrap().cards.len(),
            "a workspace `direction: both` must mint the reversed twin of a cached member"
        );
    }
}
