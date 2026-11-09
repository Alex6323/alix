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
    deck::{Deck, DeckError, DeckSettings, DeclaredSources},
    listing,
    profile::{self, Counter},
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

/// A member's card list depends on its workspace settings and its exam state
/// on its sources manifest, files the deck's own (mtime, size) cannot see.
#[derive(Clone)]
struct CachedDeck {
    settings: DeckSettings,
    deck: Result<Arc<Deck>, Arc<DeckError>>,
}

impl CachedDeck {
    fn load(path: &Path, settings: DeckSettings) -> Self {
        profile::hit(Counter::DecksLoaded);
        let deck = Deck::load_with_defaults(path, &settings)
            .map(Arc::new)
            .map_err(Arc::new);
        CachedDeck { settings, deck }
    }

    fn sources_changed(&self) -> bool {
        let Ok(deck) = &self.deck else {
            return false;
        };
        let Some(manifest) = DeclaredSources::manifest_path(&deck.path, deck.deck_token.as_deref())
        else {
            return false;
        };
        DeclaredSources::stamp_of(&manifest) != deck.declared().stamp
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
    exam: Vec<String>,
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
                .get_or_insert_with(|| listing::deck_label(path))
                .clone(),
            None => listing::deck_label(path),
        }
    }

    pub fn load(&mut self, path: &Path) -> Result<Arc<Deck>, Arc<DeckError>> {
        let settings = self.workspace_settings(path);
        self.load_with(path, &settings)
    }

    pub fn load_with(
        &mut self,
        path: &Path,
        settings: &DeckSettings,
    ) -> Result<Arc<Deck>, Arc<DeckError>> {
        match self.slot(path) {
            Some(entry) => {
                if entry
                    .deck
                    .as_ref()
                    .is_some_and(|cached| cached.settings != *settings || cached.sources_changed())
                {
                    entry.deck = None;
                }
                entry
                    .deck
                    .get_or_insert_with(|| CachedDeck::load(path, settings.clone()))
                    .deck
                    .clone()
            }
            None => CachedDeck::load(path, settings.clone()).deck,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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
        let (members, identities): (Vec<PathBuf>, HashMap<PathBuf, PathBuf>) =
            workspace::members_where_with_identity(dir, |p| self.is_deck(p))
                .unwrap_or_default()
                .into_iter()
                .map(|(path, identity)| (path.clone(), (path, identity)))
                .unzip();
        let meta = self.manifest_meta(&dir.join(workspace::MANIFEST));
        let icon = workspace::resolve_icon(dir, meta.icon.as_deref());
        Workspace {
            path: dir.to_path_buf(),
            title: meta.title,
            description: meta.description,
            settings: meta.settings,
            exam: meta.exam,
            members,
            identities,
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
    let (title, description, settings, icon, exam) = workspace::read_manifest(path);
    ManifestMeta {
        title,
        description,
        settings,
        exam,
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
    fn the_cache_counts_one_entry_per_path_however_often_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = DeckCache::default();
        assert!(cache.is_empty(), "a fresh cache holds nothing");
        assert_eq!(0, cache.len(), "a fresh cache holds nothing");

        let paths = ["a.md", "b.md", "c.md"].map(|name| dir.path().join(name));
        for path in &paths {
            write(
                path,
                "---\nformat-version: 1\nid: \"deck-d\"\n---\n## q\na\n",
            );
            cache.load(path).unwrap();
            cache.label(path);
        }

        assert!(!cache.is_empty(), "three read decks: not empty");
        assert_eq!(3, cache.len(), "three paths, each read twice");
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
    fn a_sources_manifest_edit_refreshes_a_cached_decks_exam_state_and_nothing_else_does() {
        let ws = crate::sources::testkit::Workspace::new();
        let mut cache = DeckCache::default();
        let first = cache.load(&ws.deck()).unwrap();
        assert!(!first.declares_sources(), "step 1: no sources, no exam");

        ws.write_live("notes.md", b"notes\n");
        ws.add(&["notes.md"], &["topic-alpha"]).unwrap();
        let sourced = cache.load(&ws.deck()).unwrap();
        assert!(
            sourced.declares_sources(),
            "step 2: adding a source must make the cached deck exam-capable"
        );

        write(&ws.root().join("alix.toml"), "title = \"Renamed\"\n");
        assert_eq!(
            Some("Renamed".to_string()),
            cache.workspace(ws.root()).title,
            "step 3: the same server cache sees the edited workspace manifest"
        );
        let kept = cache.load(&ws.deck()).unwrap();
        assert!(
            Arc::ptr_eq(&sourced, &kept),
            "step 3: an edit that leaves the sources alone must not re-parse the member"
        );

        let id = crate::sources::source_id(b"notes\n");
        crate::sources::commands::remove(&ws.deck(), &[id]).unwrap();
        assert!(
            !cache.load(&ws.deck()).unwrap().declares_sources(),
            "step 4: removing the last source takes the exam from the cached deck"
        );
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
