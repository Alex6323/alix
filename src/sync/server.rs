use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use super::{
    SYNC_PULL_MANIFEST_VERSION, SyncDeckDto, SyncFileDto, SyncPullManifest, digest_reader,
    entry_digest,
};
#[cfg(feature = "full")]
use crate::share::StagedFile;

pub const ROOT_ID_PREFIX: &str = "root-";

fn root_id_path(served_dir: &Path) -> std::path::PathBuf {
    served_dir.join(".alix/sync.toml")
}

fn checked_root_id(path: &Path, value: &toml::Value) -> Result<String> {
    let Some(id) = value.as_str() else {
        bail!("{}: root_id must be a string", path.display());
    };
    if !id
        .strip_prefix(ROOT_ID_PREFIX)
        .is_some_and(crate::token::is_canonical)
    {
        bail!(
            "{}: root_id must be `root-` plus 26 lowercase Crockford base32 characters",
            path.display()
        );
    }
    Ok(id.to_string())
}

fn read_table(path: &Path) -> Result<Option<toml::Table>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    toml::from_str(&text)
        .with_context(|| format!("cannot parse {}", path.display()))
        .map(Some)
}

pub fn root_id(served_dir: &Path) -> Result<String> {
    let path = root_id_path(served_dir);
    let mut table = read_table(&path)?.unwrap_or_default();
    if let Some(value) = table.get("root_id") {
        return checked_root_id(&path, value);
    }
    let token = crate::token::mint()
        .map_err(|error| anyhow::anyhow!("cannot mint root identity: {error}"))?;
    let id = format!("{ROOT_ID_PREFIX}{token}");
    table.insert("root_id".to_string(), toml::Value::String(id.clone()));
    let text = toml::to_string_pretty(&table)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    crate::fsio::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    crate::fsio::replace_file(&parent.join(".sync.toml.tmp"), &path, text.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(id)
}

pub fn read_root_id(served_dir: &Path) -> Result<Option<String>> {
    let path = root_id_path(served_dir);
    let Some(table) = read_table(&path)? else {
        return Ok(None);
    };
    table
        .get("root_id")
        .map(|value| checked_root_id(&path, value))
        .transpose()
}

#[cfg(feature = "full")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncEntry {
    pub name: String,
    pub kind: String,
    pub members: u64,
    pub unpacked_bytes: u64,
    pub digest: String,
    pub left_out: Vec<String>,
    path: PathBuf,
    store_root: PathBuf,
    decks: Vec<EntryDeck>,
    excluded_decks: HashSet<PathBuf>,
    excluded_deck_ids: HashSet<String>,
}

#[cfg(feature = "full")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct EntryDeck {
    path: PathBuf,
    relative_path: PathBuf,
    deck_id: String,
}

#[cfg(feature = "full")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckTarget {
    pub path: PathBuf,
    pub store_root: PathBuf,
}

#[cfg(feature = "full")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeckLookup<'a> {
    One(&'a DeckTarget),
    Ambiguous,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryLookup<'a> {
    One(&'a SyncEntry),
    Ambiguous,
    Missing,
}

#[cfg(feature = "full")]
#[derive(Clone, Debug)]
enum IndexedDeck {
    One(DeckTarget),
    Ambiguous,
}

#[cfg(feature = "full")]
#[derive(Clone, Debug)]
pub struct SyncCatalog {
    root: PathBuf,
    entries: Vec<SyncEntry>,
    decks: HashMap<String, IndexedDeck>,
}

#[cfg(feature = "full")]
pub struct StagedPull {
    pub root: PathBuf,
    pub manifest: SyncPullManifest,
}

#[cfg(feature = "full")]
impl SyncCatalog {
    pub fn load(
        root: &Path,
        recent: &crate::recent::RecentDecks,
        cache: &mut crate::cache::DeckCache,
    ) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("cannot resolve served folder {}", root.display()))?;
        let rows = crate::picker::catalog(&root, recent, cache)
            .with_context(|| format!("cannot list {}", root.display()))?;
        let contained: Vec<_> = rows
            .into_iter()
            .filter_map(|row| {
                let path = row.path.canonicalize().ok()?;
                path.starts_with(&root).then_some((row, path))
            })
            .collect();
        let workspace_members: HashSet<PathBuf> = contained
            .iter()
            .filter(|(row, _)| row.is_workspace)
            .flat_map(|(row, _)| row.members.iter())
            .filter_map(|member| member.path.canonicalize().ok())
            .collect();
        let mut entries = Vec::new();
        let mut decks = HashMap::new();
        for (row, path) in contained {
            if !row.is_workspace && workspace_members.contains(&path) {
                continue;
            }
            let store_root = if row.is_workspace {
                crate::workspace::root_store_path(&path)
            } else {
                let parent = path.parent().ok_or_else(|| {
                    anyhow::anyhow!("{} has no containing folder", path.display())
                })?;
                crate::workspace::root_store_path(parent)
            };
            let source_decks: Vec<PathBuf> = if row.is_workspace {
                row.members.into_iter().map(|member| member.path).collect()
            } else {
                vec![path.clone()]
            };
            let mut entry_decks = Vec::new();
            let mut left_out = Vec::new();
            let mut excluded_decks = HashSet::new();
            let mut excluded_deck_ids = HashSet::new();
            for deck_path in source_decks {
                let relative_path = if row.is_workspace {
                    deck_path
                        .strip_prefix(&path)
                        .with_context(|| {
                            format!(
                                "workspace member {} is outside {}",
                                deck_path.display(),
                                path.display()
                            )
                        })?
                        .to_path_buf()
                } else {
                    PathBuf::from(deck_path.file_name().ok_or_else(|| {
                        anyhow::anyhow!("{} has no file name", deck_path.display())
                    })?)
                };
                let wire_relative_path = wire_path(&relative_path)?;
                let deck_path = match deck_path.canonicalize() {
                    Ok(deck_path) if !row.is_workspace || deck_path.starts_with(&path) => deck_path,
                    Ok(_) | Err(_) => {
                        excluded_decks.insert(deck_path);
                        left_out.push(wire_relative_path);
                        continue;
                    }
                };
                let deck = match crate::deck::Deck::load(&deck_path) {
                    Ok(deck) => deck,
                    Err(_) => {
                        if let Some(deck_id) = lightweight_deck_id(&deck_path) {
                            excluded_deck_ids.insert(deck_id);
                        }
                        excluded_decks.insert(deck_path);
                        left_out.push(wire_relative_path);
                        continue;
                    }
                };
                let Some(deck_id) = deck.deck_token else {
                    excluded_decks.insert(deck_path);
                    left_out.push(wire_relative_path);
                    continue;
                };
                let target = DeckTarget {
                    path: deck_path.clone(),
                    store_root: store_root.clone(),
                };
                match decks.entry(deck_id.clone()) {
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert(IndexedDeck::One(target));
                    }
                    std::collections::hash_map::Entry::Occupied(mut slot) => {
                        if !matches!(slot.get(), IndexedDeck::One(existing) if existing == &target)
                        {
                            slot.insert(IndexedDeck::Ambiguous);
                        }
                    }
                }
                entry_decks.push(EntryDeck {
                    path: deck_path,
                    relative_path,
                    deck_id,
                });
            }
            entry_decks.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
            left_out.sort();
            let kind = if row.is_workspace {
                "workspace"
            } else {
                "deck"
            };
            let mut staged = if row.is_workspace {
                crate::share::staged_workspace_files_excluding(
                    &path,
                    &excluded_decks,
                    &excluded_deck_ids,
                )?
            } else if entry_decks.is_empty() {
                Vec::new()
            } else {
                crate::share::staged_deck_contents_files(&path)?
            };
            staged.extend(private_files(&store_root, &entry_decks)?);
            let files = manifest_rows(&staged)?;
            let unpacked_bytes = files
                .iter()
                .try_fold(0u64, |total, file| total.checked_add(file.bytes))
                .ok_or_else(|| {
                    anyhow::anyhow!("sync entry {} is too large to count", path.display())
                })?;
            entries.push(SyncEntry {
                name: row.name,
                kind: kind.to_string(),
                members: entry_decks.len() as u64,
                unpacked_bytes,
                digest: entry_digest(&files),
                left_out,
                path,
                store_root,
                decks: entry_decks,
                excluded_decks,
                excluded_deck_ids,
            });
        }
        Ok(Self {
            root,
            entries,
            decks,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn entries(&self) -> &[SyncEntry] {
        &self.entries
    }

    pub fn deck(&self, deck_id: &str) -> DeckLookup<'_> {
        match self.decks.get(deck_id) {
            Some(IndexedDeck::One(target)) => DeckLookup::One(target),
            Some(IndexedDeck::Ambiguous) => DeckLookup::Ambiguous,
            None => DeckLookup::Missing,
        }
    }

    pub fn entry(&self, name: &str) -> EntryLookup<'_> {
        let mut matches = self.entries.iter().filter(|entry| entry.name == name);
        match (matches.next(), matches.next()) {
            (Some(entry), None) => EntryLookup::One(entry),
            (Some(_), Some(_)) => EntryLookup::Ambiguous,
            (None, _) => EntryLookup::Missing,
        }
    }

    pub fn stage_pull(&self, name: &str, root_id: &str, stage: &Path) -> Result<StagedPull> {
        let entry = match self.entry(name) {
            EntryLookup::One(entry) => entry,
            EntryLookup::Ambiguous => bail!("ambiguous sync entry {name:?}"),
            EntryLookup::Missing => bail!("unknown sync entry {name:?}"),
        };
        stage_entry(entry, root_id, stage)
    }
}

#[cfg(feature = "full")]
fn lightweight_deck_id(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    crate::parser::deck_identity(&text).ok().flatten()
}

/// The private files `stage_entry` copies beside the public projection, at
/// the staged-relative paths it gives them.
#[cfg(feature = "full")]
fn private_files(store_root: &Path, decks: &[EntryDeck]) -> Result<Vec<StagedFile>> {
    let files = crate::state::UserFiles::new(store_root);
    let mut out = Vec::new();
    let mut push = |relative: PathBuf, source: PathBuf| -> Result<()> {
        if is_regular_file(&source)? {
            out.push(StagedFile { relative, source });
        }
        Ok(())
    };
    push(
        PathBuf::from(crate::config::LOCAL_MANIFEST),
        files.local_manifest(),
    )?;
    for deck in decks {
        push(
            Path::new(".alix/progress").join(format!("{}.json", deck.deck_id)),
            files.progress_for(&deck.deck_id),
        )?;
        push(
            crate::personal::sidecar_path(&deck.relative_path),
            crate::personal::sidecar_path(&deck.path),
        )?;
    }
    Ok(out)
}

#[cfg(feature = "full")]
fn manifest_rows(staged: &[StagedFile]) -> Result<Vec<SyncFileDto>> {
    let mut files: BTreeMap<String, SyncFileDto> = BTreeMap::new();
    for file in staged {
        let path = wire_path(&file.relative)?;
        if files.contains_key(&path) {
            continue;
        }
        let mut source = std::fs::File::open(&file.source)
            .with_context(|| format!("cannot open {}", file.source.display()))?;
        let (bytes, digest) = digest_reader(&mut source)?;
        files.insert(
            path.clone(),
            SyncFileDto {
                path,
                bytes,
                digest,
            },
        );
    }
    Ok(files.into_values().collect())
}

#[cfg(feature = "full")]
fn is_regular_file(path: &Path) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", path.display()));
        }
    };
    if metadata.file_type().is_symlink() {
        bail!(
            "{} is a link; paired sync does not follow links",
            path.display()
        );
    }
    if !metadata.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    Ok(true)
}

#[cfg(feature = "full")]
fn stage_entry(entry: &SyncEntry, root_id: &str, stage: &Path) -> Result<StagedPull> {
    std::fs::create_dir_all(stage).with_context(|| format!("cannot create {}", stage.display()))?;
    let root = stage.join(&entry.name);
    if entry.kind == "workspace" {
        let (staged, _) = crate::share::stage_workspace_excluding(
            &entry.path,
            stage,
            &entry.excluded_decks,
            &entry.excluded_deck_ids,
        )?;
        if staged != root {
            bail!(
                "staged entry {} did not keep its picker name {}",
                staged.display(),
                entry.name
            );
        }
    } else if entry.decks.is_empty() {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("cannot create {}", root.display()))?;
    } else {
        let (bundle, _) = crate::share::stage_deck_bundle(&entry.path, stage)?;
        std::fs::remove_file(bundle.join(crate::share::DECK_BUNDLE_MARKER))
            .with_context(|| format!("cannot remove the share marker from {}", bundle.display()))?;
        std::fs::rename(&bundle, &root)
            .with_context(|| format!("cannot name staged deck {}", root.display()))?;
    }

    let files = crate::state::UserFiles::new(&entry.store_root);
    copy_if_file(
        &files.local_manifest(),
        &root.join(crate::config::LOCAL_MANIFEST),
    )?;
    let mut manifest_decks = Vec::new();
    for deck in &entry.decks {
        let progress = files.progress_for(&deck.deck_id);
        let staged_progress = root
            .join(".alix/progress")
            .join(format!("{}.json", deck.deck_id));
        let revision = if copy_if_file(&progress, &staged_progress)? {
            Some(crate::store::read_deck_data(&staged_progress, &deck.deck_id, None)?.0)
        } else {
            None
        };
        let sidecar = crate::personal::sidecar_path(&deck.path);
        let staged_sidecar = root.join(crate::personal::sidecar_path(&deck.relative_path));
        copy_if_file(&sidecar, &staged_sidecar)?;
        manifest_decks.push(SyncDeckDto {
            path: wire_path(&deck.relative_path)?,
            deck_id: deck.deck_id.clone(),
            revision,
        });
    }
    let manifest = SyncPullManifest {
        version: SYNC_PULL_MANIFEST_VERSION,
        root_id: root_id.to_string(),
        entry: entry.name.clone(),
        kind: entry.kind.clone(),
        files: manifest_files(&root)?,
        decks: manifest_decks,
    };
    let manifest_path = root.join(".alix/pull.json");
    let parent = manifest_path.parent().expect("pull manifest has a parent");
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    crate::fsio::replace_file(&parent.join(".pull.json.tmp"), &manifest_path, &bytes)
        .with_context(|| format!("cannot write {}", manifest_path.display()))?;
    Ok(StagedPull { root, manifest })
}

#[cfg(feature = "full")]
fn copy_if_file(source: &Path, destination: &Path) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", source.display()));
        }
    };
    if metadata.file_type().is_symlink() {
        bail!(
            "{} is a link; paired sync does not follow links",
            source.display()
        );
    }
    if !metadata.is_file() {
        bail!("{} is not a regular file", source.display());
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::copy(source, destination)
        .with_context(|| format!("cannot copy {}", source.display()))?;
    Ok(true)
}

#[cfg(feature = "full")]
fn manifest_files(root: &Path) -> Result<Vec<SyncFileDto>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<SyncFileDto>) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("cannot read {}", dir.display()))?
            .collect::<Result<_, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let metadata = entry.file_type()?;
            if metadata.is_symlink() {
                bail!(
                    "{} is a link; paired sync does not follow links",
                    entry.path().display()
                );
            }
            if metadata.is_dir() {
                walk(root, &entry.path(), out)?;
            } else if metadata.is_file() {
                let relative = entry.path().strip_prefix(root)?.to_path_buf();
                let mut file = std::fs::File::open(entry.path())?;
                let (bytes, digest) = digest_reader(&mut file)?;
                out.push(SyncFileDto {
                    path: wire_path(&relative)?,
                    bytes,
                    digest,
                });
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[cfg(feature = "full")]
fn wire_path(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("sync paths must be UTF-8"))?;
                if part.is_empty() || part == "." || part == ".." || part.contains('/') {
                    bail!("unsafe sync path component {part:?}");
                }
                parts.push(part);
            }
            _ => bail!("sync paths must be entry-relative"),
        }
    }
    if parts.is_empty() {
        bail!("sync paths must not be empty");
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_deck(path: &Path, deck_id: &str, card_id: &str) {
        std::fs::write(
            path,
            format!(
                "---\nformat-version: 1\nid: \"{deck_id}\"\n---\n## q\na\n<!-- id: {card_id} -->\n"
            ),
        )
        .unwrap();
    }

    fn write_progress(path: &Path, deck_id: &str, revision: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!(
                "{{\"version\":1,\"deck_id\":\"{deck_id}\",\"subject\":\"deck.md\",\"revision\":{revision},\"cards\":{{}},\"writer\":null}}"
            ),
        )
        .unwrap();
    }

    #[test]
    fn root_identity_mints_once_and_preserves_other_toml_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".alix")).unwrap();
        std::fs::write(
            dir.path().join(".alix/sync.toml"),
            "label = \"kept\"\n[future]\nenabled = true\n",
        )
        .unwrap();

        let first = root_id(dir.path()).unwrap();
        let second = root_id(dir.path()).unwrap();

        assert_eq!(first, second, "a second serve must keep the minted root id");
        assert!(
            first
                .strip_prefix(ROOT_ID_PREFIX)
                .is_some_and(crate::token::is_canonical),
            "the root id must be root- plus one canonical 26-character token: {first}"
        );
        let written = std::fs::read_to_string(dir.path().join(".alix/sync.toml")).unwrap();
        let value: toml::Value = toml::from_str(&written).unwrap();
        assert_eq!(
            Some("kept"),
            value.get("label").and_then(toml::Value::as_str)
        );
        assert_eq!(
            Some(true),
            value
                .get("future")
                .and_then(|v| v.get("enabled"))
                .and_then(toml::Value::as_bool),
            "minting must preserve unrelated tables and values"
        );
    }

    #[test]
    fn malformed_root_identity_names_its_file_and_is_never_repaired() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".alix")).unwrap();
        let path = dir.path().join(".alix/sync.toml");
        std::fs::write(&path, "root_id = \"root-not-canonical\"\n").unwrap();

        let error = root_id(dir.path()).unwrap_err();

        assert!(
            format!("{error:#}").contains(path.to_string_lossy().as_ref()),
            "the malformed-id error must name its file: {error:#}"
        );
        assert_eq!(
            "root_id = \"root-not-canonical\"\n",
            std::fs::read_to_string(path).unwrap(),
            "invalid input must not be repaired or re-minted"
        );
    }

    #[test]
    fn a_missing_root_identity_is_absent_for_doctor_but_minted_for_serve() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(None, read_root_id(dir.path()).unwrap());
        let minted = root_id(dir.path()).unwrap();
        assert_eq!(Some(minted), read_root_id(dir.path()).unwrap());
    }

    #[test]
    fn moving_a_served_folder_keeps_its_root_identity() {
        let parent = tempfile::tempdir().unwrap();
        let before = parent.path().join("before");
        let after = parent.path().join("after");
        std::fs::create_dir(&before).unwrap();
        let root = root_id(&before).unwrap();

        std::fs::rename(&before, &after).unwrap();

        assert_eq!(root, root_id(&after).unwrap());
        assert!(!before.exists());
        assert!(after.join(".alix/sync.toml").is_file());
    }

    #[test]
    fn catalog_excludes_external_recent_rows_and_indexes_contained_decks() {
        let served = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside = served.path().join("inside.md");
        let external = outside.path().join("external.md");
        write_deck(&inside, "deck-inside", "card-inside");
        write_deck(&external, "deck-external", "card-external");
        let mut recent = crate::recent::RecentDecks::load(served.path().join("recent.json"));
        recent.record(&[external], 1);

        let catalog = SyncCatalog::load(
            served.path(),
            &recent,
            &mut crate::cache::DeckCache::default(),
        )
        .unwrap();

        assert_eq!(
            vec!["inside.md"],
            catalog
                .entries()
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>()
        );
        assert!(matches!(catalog.deck("deck-inside"), DeckLookup::One(_)));
        assert!(matches!(catalog.deck("deck-external"), DeckLookup::Missing));
    }

    #[test]
    fn pull_stages_private_files_and_describes_exact_rootless_payload() {
        let served = tempfile::tempdir().unwrap();
        let deck = served.path().join("inside.md");
        write_deck(&deck, "deck-inside", "card-inside");
        std::fs::write(crate::personal::sidecar_path(&deck), "personal bytes\n").unwrap();
        std::fs::write(served.path().join("alix.local.toml"), "[review]\nnew = 3\n").unwrap();
        let progress = crate::state::UserFiles::new(served.path()).progress_for("deck-inside");
        write_progress(&progress, "deck-inside", 7);
        let recent = crate::recent::RecentDecks::load(served.path().join("recent.json"));
        let catalog = SyncCatalog::load(
            served.path(),
            &recent,
            &mut crate::cache::DeckCache::default(),
        )
        .unwrap();
        let stage = tempfile::tempdir().unwrap();

        let pull = catalog
            .stage_pull("inside.md", "root-00000000000000000000000000", stage.path())
            .unwrap();

        assert_eq!("inside.md", pull.root.file_name().unwrap());
        assert_eq!(Some(7), pull.manifest.decks[0].revision);
        let paths: Vec<&str> = pull
            .manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(paths.contains(&"inside.md"));
        assert!(paths.contains(&"inside.local.md"));
        assert!(paths.contains(&"alix.local.toml"));
        assert!(paths.contains(&".alix/progress/deck-inside.json"));
        assert!(!paths.contains(&".alix-deck-share.json"));
        assert!(!paths.contains(&".alix/pull.json"));
        assert_eq!(
            serde_json::from_slice::<SyncPullManifest>(
                &std::fs::read(pull.root.join(".alix/pull.json")).unwrap()
            )
            .unwrap(),
            pull.manifest
        );
        assert_eq!(
            catalog.entries()[0].unpacked_bytes,
            pull.manifest.files.iter().map(|f| f.bytes).sum::<u64>()
        );
        assert_eq!(
            catalog.entries()[0].digest,
            entry_digest(&pull.manifest.files),
            "the listing's entry digest is the pull manifest's, computed without staging"
        );
    }

    #[test]
    fn listing_digest_matches_pull_when_workspace_members_share_a_deck_id() {
        let served = tempfile::tempdir().unwrap();
        let workspace = served.path().join("course");
        std::fs::create_dir_all(workspace.join("decks")).unwrap();
        std::fs::write(workspace.join("alix.toml"), "title = \"Course\"\n").unwrap();
        write_deck(&workspace.join("decks/a.md"), "deck-shared", "card-a");
        write_deck(&workspace.join("decks/b.md"), "deck-shared", "card-b");
        write_progress(
            &crate::state::UserFiles::new(&workspace).progress_for("deck-shared"),
            "deck-shared",
            7,
        );
        let root = root_id(served.path()).unwrap();
        let catalog = SyncCatalog::load(
            served.path(),
            &crate::recent::RecentDecks::load(served.path().join(".alix/recent.json")),
            &mut crate::cache::DeckCache::default(),
        )
        .unwrap();
        let entry = catalog
            .entries()
            .iter()
            .find(|entry| entry.name == "course")
            .unwrap();
        let stage = tempfile::tempdir().unwrap();
        let pull = catalog.stage_pull("course", &root, stage.path()).unwrap();

        assert_eq!(
            entry.digest,
            entry_digest(&pull.manifest.files),
            "the listing digest must describe the pull even when duplicate ids make the push target ambiguous"
        );
    }

    #[test]
    fn workspace_pull_is_share_plus_the_named_private_projection() {
        fn files(root: &Path) -> Vec<String> {
            fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
                for entry in std::fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        walk(root, &path, out);
                    } else {
                        out.push(
                            path.strip_prefix(root)
                                .unwrap()
                                .to_string_lossy()
                                .replace('\\', "/"),
                        );
                    }
                }
            }
            let mut out = Vec::new();
            walk(root, root, &mut out);
            out.sort();
            out
        }

        let served = tempfile::tempdir().unwrap();
        let workspace = served.path().join("course");
        std::fs::create_dir_all(workspace.join("decks")).unwrap();
        std::fs::write(workspace.join("alix.toml"), "title = \"Course\"\n").unwrap();
        write_deck(
            &workspace.join("decks/ready.md"),
            "deck-ready",
            "card-ready",
        );
        write_deck(
            &workspace.join("decks/without-progress.md"),
            "deck-withoutprogress",
            "card-withoutprogress",
        );
        std::fs::write(workspace.join("decks/draft.md"), "## draft\nanswer\n").unwrap();
        std::fs::write(workspace.join("decks/ready.local.md"), "personal notes\n").unwrap();
        std::fs::write(workspace.join("alix.local.toml"), "[review]\nnew = 3\n").unwrap();
        std::fs::create_dir_all(workspace.join("assets")).unwrap();
        std::fs::write(workspace.join("assets/icon.svg"), "<svg/>\n").unwrap();
        crate::assets::write_object(&workspace, "deck-ready", b"asset bytes\n", "txt").unwrap();
        std::fs::create_dir_all(workspace.join("augment")).unwrap();
        std::fs::write(workspace.join("augment/deck-ready.json"), "{}\n").unwrap();
        let progress = crate::state::UserFiles::new(&workspace).progress_for("deck-ready");
        write_progress(&progress, "deck-ready", 7);
        std::fs::write(
            crate::state::UserFiles::new(&workspace).recent(),
            "{\"version\":1,\"entries\":[]}\n",
        )
        .unwrap();

        let share_stage = tempfile::tempdir().unwrap();
        let (shared, _) = crate::share::stage_path(&workspace, share_stage.path()).unwrap();
        let shared_files: HashSet<_> = files(&shared).into_iter().collect();
        std::fs::write(
            workspace.join("decks/broken.md"),
            "---\nformat-version: 1\nid: deck-brokenmember\n---\n## missing answer\n",
        )
        .unwrap();
        std::fs::write(workspace.join("augment/deck-brokenmember.json"), "{}\n").unwrap();
        std::fs::create_dir_all(workspace.join("assets/deck-brokenmember")).unwrap();
        std::fs::write(
            workspace.join("assets/deck-brokenmember/left-out.txt"),
            "left out\n",
        )
        .unwrap();

        let served_root = root_id(served.path()).unwrap();
        assert!(
            !workspace.join(".alix/sync.toml").exists(),
            "serving the parent must not mint an identity under a member workspace"
        );
        std::fs::write(
            workspace.join(".alix/sync.toml"),
            "root_id = \"root-11111111111111111111111111\"\n",
        )
        .unwrap();
        let catalog = SyncCatalog::load(
            served.path(),
            &crate::recent::RecentDecks::load(served.path().join(".alix/recent.json")),
            &mut crate::cache::DeckCache::default(),
        )
        .unwrap();
        let entry = catalog
            .entries()
            .iter()
            .find(|entry| entry.name == "course")
            .unwrap();
        assert_eq!("workspace", entry.kind);
        assert_eq!(2, entry.members, "only initialized members are counted");
        assert_eq!(
            vec!["decks/broken.md"],
            entry.left_out,
            "a parser-rejected initialized member is named but not indexed"
        );

        let pull_stage = tempfile::tempdir().unwrap();
        let pull = catalog
            .stage_pull("course", &served_root, pull_stage.path())
            .unwrap();
        let pull_files: HashSet<_> = files(&pull.root)
            .into_iter()
            .filter(|path| path != ".alix/pull.json")
            .collect();
        let expected: HashSet<_> = shared_files
            .iter()
            .cloned()
            .chain([
                ".alix/progress/deck-ready.json".to_string(),
                "alix.local.toml".to_string(),
                "decks/ready.local.md".to_string(),
            ])
            .collect();
        assert_eq!(
            expected, pull_files,
            "pull re-adds only progress by deck id and the two *.local.* shapes"
        );
        assert!(shared_files.contains("decks/draft.md"));
        assert!(
            !pull_files.contains("decks/broken.md"),
            "a left-out member is absent from the pull manifest projection"
        );
        assert!(!pull_files.contains("augment/deck-brokenmember.json"));
        assert!(!pull_files.contains("assets/deck-brokenmember/left-out.txt"));
        for private in [
            ".alix/progress/deck-ready.json",
            "alix.local.toml",
            "decks/ready.local.md",
        ] {
            assert!(!shared_files.contains(private), "share strips {private}");
        }
        for never_travels in [".alix/recent.json", ".alix/sync.toml"] {
            assert!(
                !pull_files.contains(never_travels),
                "{never_travels} is local to each root"
            );
        }

        assert_eq!(2, pull.manifest.decks.len());
        assert_eq!(
            Some(7),
            pull.manifest
                .decks
                .iter()
                .find(|deck| deck.deck_id == "deck-ready")
                .unwrap()
                .revision
        );
        assert_eq!(
            None,
            pull.manifest
                .decks
                .iter()
                .find(|deck| deck.deck_id == "deck-withoutprogress")
                .unwrap()
                .revision
        );
        assert!(
            pull.manifest
                .decks
                .iter()
                .all(|deck| deck.path != "decks/draft.md" && deck.path != "decks/broken.md")
        );
        for file in &pull.manifest.files {
            let bytes = std::fs::read(pull.root.join(&file.path)).unwrap();
            assert_eq!(bytes.len() as u64, file.bytes, "{} byte count", file.path);
            assert_eq!(
                crate::sync::digest(&bytes),
                file.digest,
                "{} digest",
                file.path
            );
        }
        assert_eq!(
            entry.unpacked_bytes,
            pull.manifest
                .files
                .iter()
                .map(|file| file.bytes)
                .sum::<u64>(),
            "entries size equals the manifest payload sum"
        );
        assert_eq!(
            entry.digest,
            entry_digest(&pull.manifest.files),
            "the listing's entry digest is the pull manifest's, computed without staging"
        );
    }

    #[test]
    #[ignore = "manual 1,000-deck index-cost receipt"]
    fn measure_sync_catalog_index_cost_for_1000_decks() {
        let served = tempfile::tempdir().unwrap();
        for index in 0..1000 {
            write_deck(
                &served.path().join(format!("deck-{index:04}.md")),
                &format!("deck-index{index:04}"),
                &format!("card-index{index:04}"),
            );
        }
        let recent = crate::recent::RecentDecks::load(served.path().join(".alix/recent.json"));
        let mut samples = Vec::new();
        for _ in 0..5 {
            let started = std::time::Instant::now();
            let catalog = SyncCatalog::load(
                served.path(),
                &recent,
                &mut crate::cache::DeckCache::default(),
            )
            .unwrap();
            samples.push(started.elapsed().as_micros());
            assert_eq!(1000, catalog.entries().len());
        }
        samples.sort_unstable();
        eprintln!(
            "sync 1000-deck index: median={}us samples={samples:?}",
            samples[samples.len() / 2]
        );
    }
}
