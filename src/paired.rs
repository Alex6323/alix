use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    state::UserFiles,
    store::{self, Writer},
    sync::{SYNC_PULL_MANIFEST_VERSION, SyncDeckDto, SyncPullManifest, digest},
    token,
};

pub const PAIRED_DIR: &str = "paired";
pub const KIND_WORKSPACE: &str = "workspace";
pub const KIND_DECK: &str = "deck";
pub const MANIFEST_IN_ZIP: &str = ".alix/pull.json";
const PULL_DIR: &str = "pull";
const STAGING_DIR: &str = "staging";
const PUSHED_FILE: &str = "pushed.json";
const CONFLICTS_FILE: &str = "conflicts.json";
const PULLED_SUFFIX: &str = ".pulled";
const OLD_SUFFIX: &str = ".old";
const STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairedRoot {
    dir: PathBuf,
}

impl PairedRoot {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn root_id(&self) -> String {
        self.dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn private(&self) -> PathBuf {
        UserFiles::new(&self.dir).private_root()
    }

    pub fn manifest_path(&self, entry: &str) -> PathBuf {
        self.private().join(PULL_DIR).join(format!("{entry}.json"))
    }

    pub fn staging(&self) -> PathBuf {
        self.private().join(STAGING_DIR)
    }

    fn pushed_path(&self) -> PathBuf {
        self.private().join(PUSHED_FILE)
    }

    fn conflicts_path(&self) -> PathBuf {
        self.private().join(CONFLICTS_FILE)
    }

    pub fn entry_root(&self, kind: &str, entry: &str) -> PathBuf {
        if kind == KIND_WORKSPACE {
            self.dir.join(entry)
        } else {
            self.dir.clone()
        }
    }

    pub fn document_path(&self, kind: &str, entry: &str, deck_id: &str) -> PathBuf {
        UserFiles::new(self.entry_root(kind, entry)).progress_for(deck_id)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pushed {
    pub desktop: Option<u64>,
    pub phone: u64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PushedFile {
    version: u32,
    decks: BTreeMap<String, Pushed>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub enum Conflict {
    Push {
        desktop_revision: Option<u64>,
        pulled_revision: Option<u64>,
        desktop_writer: Option<Writer>,
    },
    Pull {
        pulled_revision: Option<u64>,
        pulled_writer: Option<Writer>,
    },
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConflictsFile {
    version: u32,
    decks: BTreeMap<String, Conflict>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushItem {
    pub deck_id: String,
    pub entry: String,
    pub document: PathBuf,
    pub base: Option<u64>,
    pub phone_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushOutcome {
    Accepted {
        revision: u64,
    },
    Conflict {
        desktop_revision: Option<u64>,
        desktop_writer: Option<Writer>,
    },
    NotServed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    KeepPhone,
    TakeDesktop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Done,
    Push(PushItem),
    Pull { entry: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullReport {
    pub entry: String,
    pub kind: String,
    pub landed: Vec<String>,
    pub kept: Vec<String>,
    pub conflicts: Vec<String>,
    pub phone_only: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckState {
    pub deck_id: String,
    pub path: String,
    pub unpushed: bool,
    pub conflict: Option<Conflict>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PulledEntry {
    pub entry: String,
    pub kind: String,
    pub decks: Vec<DeckState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocumentPlan {
    Land,
    KeepUnpushed,
    Conflict,
}

struct DocumentHead {
    revision: u64,
    writer: Option<Writer>,
}

pub fn manifests(root: &PairedRoot) -> Result<Vec<SyncPullManifest>> {
    let dir = root.private().join(PULL_DIR);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    for entry in entries {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            out.push(read_manifest(&path)?);
        }
    }
    out.sort_by(|left, right| left.entry.cmp(&right.entry));
    Ok(out)
}

fn read_manifest(path: &Path) -> Result<SyncPullManifest> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let manifest: SyncPullManifest = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a pull manifest", path.display()))?;
    if manifest.version != SYNC_PULL_MANIFEST_VERSION {
        bail!(
            "{} is manifest version {}, this build reads {}",
            path.display(),
            manifest.version,
            SYNC_PULL_MANIFEST_VERSION
        );
    }
    Ok(manifest)
}

fn check_manifest(root: &PairedRoot, manifest: &SyncPullManifest) -> Result<()> {
    if manifest.root_id != root.root_id() {
        bail!(
            "the pull names root `{}` but this pairing is root `{}`",
            manifest.root_id,
            root.root_id()
        );
    }
    check_entry_name(&manifest.entry)?;
    if manifest.kind != KIND_WORKSPACE && manifest.kind != KIND_DECK {
        bail!("`{}` is not an entry kind", manifest.kind);
    }
    for file in &manifest.files {
        owned_path(manifest, &file.path)?;
    }
    for deck in &manifest.decks {
        if !matches!(
            token::parse_id(&deck.deck_id),
            Some((token::Kind::Deck, ..))
        ) {
            bail!("`{}` is not a deck id", deck.deck_id);
        }
        owned_path(manifest, &deck.path)?;
        let listed = manifest
            .files
            .iter()
            .any(|f| f.path == document_rel(&deck.deck_id));
        if deck.revision.is_some() != listed {
            bail!(
                "deck `{}` has revision {:?} but its document is {}listed",
                deck.deck_id,
                deck.revision,
                if listed { "" } else { "not " }
            );
        }
    }
    Ok(())
}

fn check_entry_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('.')
        || name.contains('/')
        || name.contains('\\')
        || name.ends_with(OLD_SUFFIX)
    {
        bail!("`{name}` is not an entry name");
    }
    Ok(())
}

fn document_rel(deck_id: &str) -> String {
    format!(".alix/progress/{deck_id}.json")
}

fn member_files(manifest: &SyncPullManifest, deck: &SyncDeckDto) -> Vec<String> {
    let sidecar = sidecar_rel(&deck.path);
    let augment = format!("augment/{}.json", deck.deck_id);
    let assets = format!("assets/{}/", deck.deck_id);
    let document = document_rel(&deck.deck_id);
    manifest
        .files
        .iter()
        .map(|file| file.path.clone())
        .filter(|path| {
            manifest.kind == KIND_DECK
                || *path == deck.path
                || *path == sidecar
                || *path == augment
                || *path == document
                || path.starts_with(&assets)
        })
        .collect()
}

fn sidecar_rel(rel: &str) -> String {
    let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    let twin = crate::personal::sidecar_path(Path::new(name))
        .to_string_lossy()
        .into_owned();
    if dir.is_empty() {
        twin
    } else {
        format!("{dir}/{twin}")
    }
}

fn owned_path(manifest: &SyncPullManifest, path: &str) -> Result<Vec<String>> {
    let parts: Vec<String> = path.split('/').map(str::to_string).collect();
    let bad = parts.is_empty()
        || parts.iter().any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.contains('\\')
                || part.ends_with(OLD_SUFFIX)
        });
    if bad {
        bail!("`{path}` is not an entry-relative path");
    }
    if parts[0] == ".alix" {
        let document = parts.len() == 3 && parts[1] == "progress" && parts[2].ends_with(".json");
        if !document && path != MANIFEST_IN_ZIP {
            bail!("`{path}` is private phone state, never desktop-owned");
        }
    }
    if manifest.kind == KIND_DECK {
        let sidecar = crate::personal::sidecar_path(Path::new(&manifest.entry))
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let single = parts.len() == 1;
        let allowed = (single && parts[0] == manifest.entry)
            || (single && parts[0] == sidecar)
            || (single && parts[0] == crate::config::LOCAL_MANIFEST)
            || parts[0] == "augment"
            || parts[0] == "assets"
            || parts[0] == ".alix";
        if !allowed {
            bail!(
                "`{path}` is outside what deck entry `{}` owns",
                manifest.entry
            );
        }
    }
    Ok(parts)
}

fn rel_path(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in rel.split('/') {
        out.push(part);
    }
    out
}

fn walk_files(dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    walk_into(dir, "", &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_into(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = std::fs::symlink_metadata(&path)?.file_type();
        if kind.is_symlink() {
            bail!("`{rel}` is a link, and a pull lands files only");
        }
        if kind.is_dir() {
            walk_into(&path, &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

fn verify_files(unpacked: &Path, manifest: &SyncPullManifest, present: &[String]) -> Result<()> {
    let listed: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    for rel in present {
        if rel != MANIFEST_IN_ZIP && !listed.contains(rel.as_str()) {
            bail!("the pull carries `{rel}`, which its manifest does not list");
        }
    }
    for file in &manifest.files {
        if file.path == MANIFEST_IN_ZIP {
            continue;
        }
        let path = rel_path(unpacked, &file.path);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("the manifest lists `{}` but the pull lacks it", file.path))?;
        if bytes.len() as u64 != file.bytes {
            bail!(
                "`{}` is {} bytes, the manifest says {}",
                file.path,
                bytes.len(),
                file.bytes
            );
        }
        let actual = digest(&bytes);
        if actual != file.digest {
            bail!(
                "`{}` digests to {actual}, the manifest says {}",
                file.path,
                file.digest
            );
        }
    }
    Ok(())
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

fn read_state<T: Default + for<'de> Deserialize<'de>>(
    path: &Path,
    version_of: fn(&T) -> u32,
) -> Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let value: T =
        serde_json::from_str(&text).with_context(|| format!("{} is unreadable", path.display()))?;
    if version_of(&value) != STATE_VERSION {
        bail!(
            "{} is version {}, this build reads {STATE_VERSION}",
            path.display(),
            version_of(&value)
        );
    }
    Ok(value)
}

fn read_pushed(root: &PairedRoot) -> Result<BTreeMap<String, Pushed>> {
    Ok(read_state::<PushedFile>(&root.pushed_path(), |f| f.version)?.decks)
}

fn write_pushed(root: &PairedRoot, decks: &BTreeMap<String, Pushed>) -> Result<()> {
    write_json_atomic(
        &root.pushed_path(),
        &PushedFile {
            version: STATE_VERSION,
            decks: decks.clone(),
        },
    )
}

pub fn conflicts(root: &PairedRoot) -> Result<BTreeMap<String, Conflict>> {
    Ok(read_state::<ConflictsFile>(&root.conflicts_path(), |f| f.version)?.decks)
}

fn write_conflicts(root: &PairedRoot, decks: &BTreeMap<String, Conflict>) -> Result<()> {
    write_json_atomic(
        &root.conflicts_path(),
        &ConflictsFile {
            version: STATE_VERSION,
            decks: decks.clone(),
        },
    )
}

fn document_head(path: &Path, deck_id: &str) -> Result<Option<DocumentHead>> {
    if !path.is_file() {
        return Ok(None);
    }
    let (revision, _, data) = store::read_deck_data(path, deck_id, None)?;
    Ok(Some(DocumentHead {
        revision,
        writer: data.writer,
    }))
}

fn document_plan(
    head: Option<&DocumentHead>,
    state: Option<&Pushed>,
    desktop: Option<u64>,
) -> DocumentPlan {
    let Some(head) = head else {
        return DocumentPlan::Land;
    };
    let unpushed = state.is_none_or(|s| s.phone != head.revision);
    if !unpushed {
        return DocumentPlan::Land;
    }
    let desktop_moved = state.map_or(desktop.is_some(), |s| s.desktop != desktop);
    if desktop_moved {
        DocumentPlan::Conflict
    } else {
        DocumentPlan::KeepUnpushed
    }
}

pub fn plan_pushes(root: &PairedRoot) -> Result<Vec<PushItem>> {
    let pushed = read_pushed(root)?;
    let conflicts = conflicts(root)?;
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for manifest in manifests(root)? {
        for deck in &manifest.decks {
            if conflicts.contains_key(&deck.deck_id) || !seen.insert(deck.deck_id.clone()) {
                continue;
            }
            let document = root.document_path(&manifest.kind, &manifest.entry, &deck.deck_id);
            let Some(head) = document_head(&document, &deck.deck_id)? else {
                continue;
            };
            let state = pushed.get(&deck.deck_id);
            if state.is_some_and(|s| s.phone == head.revision) {
                continue;
            }
            items.push(PushItem {
                deck_id: deck.deck_id.clone(),
                entry: manifest.entry.clone(),
                document,
                base: state.and_then(|s| s.desktop),
                phone_revision: head.revision,
            });
        }
    }
    Ok(items)
}

pub fn record_push(root: &PairedRoot, item: &PushItem, outcome: PushOutcome) -> Result<()> {
    let mut pushed = read_pushed(root)?;
    let mut marks = conflicts(root)?;
    match outcome {
        PushOutcome::Accepted { revision } => {
            pushed.insert(
                item.deck_id.clone(),
                Pushed {
                    desktop: Some(revision),
                    phone: item.phone_revision,
                },
            );
            marks.remove(&item.deck_id);
        }
        PushOutcome::Conflict {
            desktop_revision,
            desktop_writer,
        } => {
            marks.insert(
                item.deck_id.clone(),
                Conflict::Push {
                    desktop_revision,
                    pulled_revision: item.base,
                    desktop_writer,
                },
            );
        }
        PushOutcome::NotServed => return Ok(()),
    }
    write_pushed(root, &pushed)?;
    write_conflicts(root, &marks)
}

pub fn pulled_entries(root: &PairedRoot) -> Result<Vec<PulledEntry>> {
    let pushed = read_pushed(root)?;
    let marks = conflicts(root)?;
    let mut out = Vec::new();
    for manifest in manifests(root)? {
        let mut decks = Vec::new();
        for deck in &manifest.decks {
            let document = root.document_path(&manifest.kind, &manifest.entry, &deck.deck_id);
            let head = document_head(&document, &deck.deck_id)?;
            let unpushed = head.as_ref().is_some_and(|h| {
                pushed
                    .get(&deck.deck_id)
                    .is_none_or(|s| s.phone != h.revision)
            });
            decks.push(DeckState {
                deck_id: deck.deck_id.clone(),
                path: deck.path.clone(),
                unpushed,
                conflict: marks.get(&deck.deck_id).cloned(),
            });
        }
        out.push(PulledEntry {
            entry: manifest.entry,
            kind: manifest.kind,
            decks,
        });
    }
    Ok(out)
}

pub fn apply_unpacked(
    root: &PairedRoot,
    unpacked: &Path,
    hook: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<PullReport> {
    if !unpacked.starts_with(root.staging()) {
        bail!(
            "an unpacked pull must sit under {}",
            root.staging().display()
        );
    }
    let manifest = read_manifest(&unpacked.join(MANIFEST_IN_ZIP))?;
    check_manifest(root, &manifest)?;
    let present = walk_files(unpacked)?;
    verify_files(unpacked, &manifest, &present)?;
    let previous_path = root.manifest_path(&manifest.entry);
    let previous = previous_path
        .exists()
        .then(|| read_manifest(&previous_path))
        .transpose()?;
    if let Some(previous) = &previous {
        check_manifest(root, previous)?;
        if previous.kind != manifest.kind {
            bail!(
                "entry `{}` was pulled as a {} and is now a {}",
                manifest.entry,
                previous.kind,
                manifest.kind
            );
        }
    }
    let owned_before: BTreeSet<String> = previous
        .iter()
        .flat_map(|p| p.files.iter().map(|f| f.path.clone()))
        .collect();
    let new_files: BTreeSet<String> = manifest.files.iter().map(|f| f.path.clone()).collect();
    let mut pushed = read_pushed(root)?;
    let mut marks = conflicts(root)?;
    let entry_root = root.entry_root(&manifest.kind, &manifest.entry);
    let mut report = PullReport {
        entry: manifest.entry.clone(),
        kind: manifest.kind.clone(),
        ..Default::default()
    };
    let mut plans: Vec<(SyncDeckDto, DocumentPlan)> = Vec::new();
    let mut keep_docs = BTreeSet::new();
    for deck in &manifest.decks {
        let rel = document_rel(&deck.deck_id);
        let head = document_head(&rel_path(&entry_root, &rel), &deck.deck_id)?;
        let plan = document_plan(head.as_ref(), pushed.get(&deck.deck_id), deck.revision);
        match plan {
            DocumentPlan::Land => report.landed.push(deck.deck_id.clone()),
            DocumentPlan::KeepUnpushed => {
                report.kept.push(deck.deck_id.clone());
                keep_docs.insert(rel);
            }
            DocumentPlan::Conflict => {
                report.conflicts.push(deck.deck_id.clone());
                keep_docs.insert(rel);
            }
        }
        plans.push((deck.clone(), plan));
    }
    let new_deck_ids: BTreeSet<&str> = manifest
        .decks
        .iter()
        .map(|deck| deck.deck_id.as_str())
        .collect();
    let mut retained: BTreeSet<String> = BTreeSet::new();
    if let Some(previous) = &previous {
        for deck in &previous.decks {
            if new_deck_ids.contains(deck.deck_id.as_str()) {
                continue;
            }
            let live = rel_path(&entry_root, &document_rel(&deck.deck_id));
            let head = document_head(&live, &deck.deck_id)?;
            let unpushed = head.as_ref().is_some_and(|h| {
                pushed
                    .get(&deck.deck_id)
                    .is_none_or(|s| s.phone != h.revision)
            });
            if !unpushed && !marks.contains_key(&deck.deck_id) {
                continue;
            }
            report.kept.push(deck.deck_id.clone());
            retained.extend(member_files(previous, deck));
        }
    }
    std::fs::remove_file(unpacked.join(MANIFEST_IN_ZIP))?;
    for (deck, plan) in &plans {
        let rel = document_rel(&deck.deck_id);
        let staged = rel_path(unpacked, &rel);
        let live = rel_path(&entry_root, &rel);
        match plan {
            DocumentPlan::Land => {}
            DocumentPlan::KeepUnpushed => {
                if staged.exists() {
                    std::fs::remove_file(&staged)?;
                }
                if manifest.kind == KIND_WORKSPACE {
                    std::fs::create_dir_all(staged.parent().unwrap_or(unpacked))?;
                    std::fs::copy(&live, &staged)?;
                }
            }
            DocumentPlan::Conflict => {
                if staged.exists() {
                    std::fs::rename(&staged, pulled_path(&staged))?;
                }
                if manifest.kind == KIND_WORKSPACE {
                    std::fs::create_dir_all(staged.parent().unwrap_or(unpacked))?;
                    std::fs::copy(&live, &staged)?;
                }
            }
        }
    }
    if manifest.kind == KIND_WORKSPACE {
        if entry_root.is_dir() {
            for rel in walk_files(&entry_root)? {
                let carried = retained.contains(&rel);
                if (owned_before.contains(&rel) && !carried) || rel == MANIFEST_IN_ZIP {
                    continue;
                }
                let dest = rel_path(unpacked, &rel);
                if dest.exists() {
                    continue;
                }
                std::fs::create_dir_all(dest.parent().unwrap_or(unpacked))?;
                std::fs::copy(rel_path(&entry_root, &rel), &dest)?;
                if !carried {
                    report.phone_only.push(rel);
                }
            }
        }
        for rel in &owned_before {
            if !new_files.contains(rel)
                && !keep_docs.contains(rel)
                && !retained.contains(rel)
                && rel_path(&entry_root, rel).exists()
            {
                report.removed.push(rel.clone());
            }
        }
        let old = old_path(&entry_root);
        hook("swap-out")?;
        if entry_root.exists() {
            std::fs::rename(&entry_root, &old)?;
        }
        hook("swap-in")?;
        std::fs::rename(unpacked, &entry_root)?;
        hook("swap-done")?;
        if old.exists() {
            std::fs::remove_dir_all(&old)?;
        }
    } else {
        for rel in &owned_before {
            if new_files.contains(rel) || keep_docs.contains(rel) || retained.contains(rel) {
                continue;
            }
            let live = rel_path(&root.dir, rel);
            if live.exists() {
                hook(&format!("remove:{rel}"))?;
                std::fs::remove_file(&live)?;
                report.removed.push(rel.clone());
            }
        }
        for (rel, is_dir) in landing_units(unpacked, &manifest.entry)? {
            hook(&format!("land:{rel}"))?;
            let src = rel_path(unpacked, &rel);
            let dest = rel_path(&root.dir, &rel);
            std::fs::create_dir_all(dest.parent().unwrap_or(&root.dir))?;
            if is_dir {
                let old = old_path(&dest);
                if dest.exists() {
                    std::fs::rename(&dest, &old)?;
                }
                std::fs::rename(&src, &dest)?;
                if old.exists() {
                    std::fs::remove_dir_all(&old)?;
                }
            } else {
                std::fs::rename(&src, &dest)?;
            }
        }
        std::fs::remove_dir_all(unpacked)?;
    }
    hook("manifest")?;
    write_json_atomic(&previous_path, &manifest)?;
    hook("state")?;
    for (deck, plan) in &plans {
        let live = rel_path(&entry_root, &document_rel(&deck.deck_id));
        match plan {
            DocumentPlan::Land => {
                match (deck.revision, document_head(&live, &deck.deck_id)?) {
                    (Some(desktop), Some(head)) => {
                        pushed.insert(
                            deck.deck_id.clone(),
                            Pushed {
                                desktop: Some(desktop),
                                phone: head.revision,
                            },
                        );
                    }
                    _ => {
                        pushed.remove(&deck.deck_id);
                    }
                }
                marks.remove(&deck.deck_id);
            }
            DocumentPlan::KeepUnpushed => {}
            DocumentPlan::Conflict => {
                let pulled = document_head(&pulled_path(&live), &deck.deck_id)?;
                marks.insert(
                    deck.deck_id.clone(),
                    Conflict::Pull {
                        pulled_revision: deck.revision,
                        pulled_writer: pulled.and_then(|h| h.writer),
                    },
                );
            }
        }
    }
    write_pushed(root, &pushed)?;
    write_conflicts(root, &marks)?;
    Ok(report)
}

fn pulled_path(document: &Path) -> PathBuf {
    let mut name = document.file_name().unwrap_or_default().to_os_string();
    name.push(PULLED_SUFFIX);
    document.with_file_name(name)
}

fn old_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(OLD_SUFFIX);
    path.with_file_name(name)
}

/// Assets land per deck directory (one rename), everything else per file,
/// the deck file last so a crash never shows a new deck beside old parts.
fn landing_units(unpacked: &Path, deck_file: &str) -> Result<Vec<(String, bool)>> {
    let mut units: Vec<(String, bool)> = Vec::new();
    let mut seen = BTreeSet::new();
    for rel in walk_files(unpacked)? {
        let parts: Vec<&str> = rel.split('/').collect();
        if parts.len() >= 3 && parts[0] == "assets" {
            let dir = format!("{}/{}", parts[0], parts[1]);
            if seen.insert(dir.clone()) {
                units.push((dir, true));
            }
        } else if rel != deck_file {
            units.push((rel, false));
        }
    }
    if unpacked.join(deck_file).is_file() {
        units.push((deck_file.to_string(), false));
    }
    Ok(units)
}

pub fn recover(root: &PairedRoot) -> Result<Vec<String>> {
    let mut actions = Vec::new();
    for dir in [root.dir.clone(), root.dir.join(crate::assets::ROOT)] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry?.path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let Some(live_name) = name.strip_suffix(OLD_SUFFIX) else {
                continue;
            };
            if !path.is_dir() {
                continue;
            }
            let live = dir.join(live_name);
            if live.exists() {
                std::fs::remove_dir_all(&path)?;
                actions.push(format!("removed {name}"));
            } else {
                std::fs::rename(&path, &live)?;
                actions.push(format!("restored {live_name}"));
            }
        }
    }
    if root.staging().exists() {
        std::fs::remove_dir_all(root.staging())?;
        actions.push("cleared staging".to_string());
    }
    Ok(actions)
}

pub fn resolve_conflict(root: &PairedRoot, deck_id: &str, choice: Choice) -> Result<Resolution> {
    let mut marks = conflicts(root)?;
    let Some(mark) = marks.remove(deck_id) else {
        bail!("no conflict is recorded for `{deck_id}`");
    };
    let manifest = manifests(root)?
        .into_iter()
        .find(|m| m.decks.iter().any(|d| d.deck_id == deck_id))
        .with_context(|| format!("no pulled entry carries `{deck_id}`"))?;
    let document = root.document_path(&manifest.kind, &manifest.entry, deck_id);
    let mut pushed = read_pushed(root)?;
    let resolution = match (mark, choice) {
        (
            Conflict::Pull {
                pulled_revision, ..
            },
            Choice::TakeDesktop,
        ) => {
            let pulled = pulled_path(&document);
            if pulled.exists() {
                std::fs::rename(&pulled, &document)?;
            } else if document.exists() {
                std::fs::remove_file(&document)?;
            }
            match (pulled_revision, document_head(&document, deck_id)?) {
                (Some(desktop), Some(head)) => {
                    pushed.insert(
                        deck_id.to_string(),
                        Pushed {
                            desktop: Some(desktop),
                            phone: head.revision,
                        },
                    );
                }
                _ => {
                    pushed.remove(deck_id);
                }
            }
            Resolution::Done
        }
        (
            Conflict::Pull {
                pulled_revision, ..
            },
            Choice::KeepPhone,
        ) => {
            let pulled = pulled_path(&document);
            if pulled.exists() {
                std::fs::remove_file(&pulled)?;
            }
            let head = document_head(&document, deck_id)?
                .with_context(|| format!("the phone holds no document for `{deck_id}`"))?;
            pushed.entry(deck_id.to_string()).or_default().desktop = pulled_revision;
            Resolution::Push(PushItem {
                deck_id: deck_id.to_string(),
                entry: manifest.entry,
                document,
                base: pulled_revision,
                phone_revision: head.revision,
            })
        }
        (
            Conflict::Push {
                desktop_revision, ..
            },
            Choice::KeepPhone,
        ) => {
            let head = document_head(&document, deck_id)?
                .with_context(|| format!("the phone holds no document for `{deck_id}`"))?;
            pushed.entry(deck_id.to_string()).or_default().desktop = desktop_revision;
            Resolution::Push(PushItem {
                deck_id: deck_id.to_string(),
                entry: manifest.entry,
                document,
                base: desktop_revision,
                phone_revision: head.revision,
            })
        }
        (Conflict::Push { .. }, Choice::TakeDesktop) => {
            if let Some(head) = document_head(&document, deck_id)? {
                pushed.entry(deck_id.to_string()).or_default().phone = head.revision;
            }
            Resolution::Pull {
                entry: manifest.entry,
            }
        }
    };
    write_pushed(root, &pushed)?;
    write_conflicts(root, &marks)?;
    Ok(resolution)
}

pub fn remove_entry(root: &PairedRoot, entry: &str) -> Result<()> {
    let all = manifests(root)?;
    let manifest = all
        .iter()
        .find(|m| m.entry == entry)
        .with_context(|| format!("`{entry}` is not a pulled entry"))?;
    let others: Vec<&SyncPullManifest> = all.iter().filter(|m| m.entry != entry).collect();
    let entry_root = root.entry_root(&manifest.kind, entry);
    if manifest.kind == KIND_WORKSPACE {
        if entry_root.exists() {
            std::fs::remove_dir_all(&entry_root)?;
        }
    } else {
        let mut seen = BTreeSet::new();
        for file in &manifest.files {
            let parts: Vec<&str> = file.path.split('/').collect();
            let unit = if parts.len() >= 3 && parts[0] == "assets" {
                format!("{}/{}", parts[0], parts[1])
            } else {
                file.path.clone()
            };
            if !seen.insert(unit.clone()) {
                continue;
            }
            let shared = others.iter().any(|m| {
                m.files
                    .iter()
                    .any(|f| f.path == unit || f.path.starts_with(&format!("{unit}/")))
            });
            if shared {
                continue;
            }
            let live = rel_path(&root.dir, &unit);
            if live.is_dir() {
                std::fs::remove_dir_all(&live)?;
            } else if live.exists() {
                std::fs::remove_file(&live)?;
            }
        }
    }
    let mut pushed = read_pushed(root)?;
    let mut marks = conflicts(root)?;
    for deck in &manifest.decks {
        let elsewhere = others
            .iter()
            .any(|m| m.decks.iter().any(|d| d.deck_id == deck.deck_id));
        if !elsewhere {
            pushed.remove(&deck.deck_id);
            marks.remove(&deck.deck_id);
        }
    }
    std::fs::remove_file(root.manifest_path(entry))?;
    write_pushed(root, &pushed)?;
    write_conflicts(root, &marks)
}

pub fn tidy_renamed(root: &PairedRoot, listed: &[String]) -> Result<Vec<(String, String)>> {
    let all = manifests(root)?;
    let states = pulled_entries(root)?;
    let mut renamed = Vec::new();
    for manifest in &all {
        if listed.contains(&manifest.entry) || manifest.decks.is_empty() {
            continue;
        }
        let dirty = states.iter().any(|s| {
            s.entry == manifest.entry && s.decks.iter().any(|d| d.unpushed || d.conflict.is_some())
        });
        if dirty || has_phone_only(root, manifest)? {
            continue;
        }
        let target = all.iter().find(|other| {
            other.entry != manifest.entry
                && listed.contains(&other.entry)
                && manifest
                    .decks
                    .iter()
                    .all(|d| other.decks.iter().any(|o| o.deck_id == d.deck_id))
        });
        if let Some(target) = target {
            renamed.push((manifest.entry.clone(), target.entry.clone()));
        }
    }
    for (old, _) in &renamed {
        remove_entry(root, old)?;
    }
    Ok(renamed)
}

fn has_phone_only(root: &PairedRoot, manifest: &SyncPullManifest) -> Result<bool> {
    if manifest.kind != KIND_WORKSPACE {
        return Ok(false);
    }
    let entry_root = root.entry_root(&manifest.kind, &manifest.entry);
    if !entry_root.is_dir() {
        return Ok(false);
    }
    let owned: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    Ok(walk_files(&entry_root)?
        .iter()
        .any(|rel| !owned.contains(rel.as_str())))
}

pub fn needs_space(compressed: u64, unpacked: u64) -> u64 {
    compressed.saturating_add(unpacked)
}

#[cfg(all(unix, feature = "sync-client"))]
pub fn free_space(path: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .with_context(|| format!("{} holds a NUL byte", path.display()))?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c_path is a valid NUL-terminated string and stat is a zeroed
    // struct the call fills in.
    let code = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if code != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("cannot stat the filesystem of {}", path.display()));
    }
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

#[cfg(feature = "sync-client")]
pub fn unpack(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("cannot open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("{} is not a readable zip archive", zip_path.display()))?;
    archive
        .extract(dest)
        .with_context(|| format!("cannot extract {}", zip_path.display()))?;
    Ok(())
}

#[cfg(feature = "sync-client")]
pub fn apply_pull(
    root: &PairedRoot,
    entry: &str,
    zip_path: &Path,
    hook: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<PullReport> {
    check_entry_name(entry)?;
    let dest = root.staging().join(entry);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::create_dir_all(&dest)?;
    let result = unpack(zip_path, &dest).and_then(|()| apply_unpacked(root, &dest, hook));
    if result.is_err() && dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        store::Store,
        sync::{SyncFileDto, SyncPullManifest},
    };

    const ROOT_ID: &str = "root-9w2c7x4k1m8q3z5t0v6b2n4d8f";
    const DECK_A: &str = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f";
    const DECK_B: &str = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8g";
    const DECK_C: &str = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8h";

    fn fresh_root() -> (tempfile::TempDir, PairedRoot) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(PAIRED_DIR).join(ROOT_ID);
        std::fs::create_dir_all(&dir).unwrap();
        (tmp, PairedRoot::new(dir))
    }

    /// A progress document with `saves` saves, so `revision == saves`.
    fn document_bytes(deck_id: &str, saves: u64) -> Vec<u8> {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(format!("{deck_id}.json"));
        for _ in 0..saves {
            let store = Store::open_deck(&path, deck_id, "subject").unwrap();
            store.save().unwrap();
        }
        std::fs::read(&path).unwrap()
    }

    fn bump(path: &Path, deck_id: &str) {
        Store::open_deck(path, deck_id, "subject")
            .unwrap()
            .save()
            .unwrap();
    }

    fn revision(path: &Path, deck_id: &str) -> Option<u64> {
        document_head(path, deck_id).unwrap().map(|h| h.revision)
    }

    struct Bundle<'a> {
        entry: &'a str,
        kind: &'a str,
        files: Vec<(String, Vec<u8>)>,
        decks: Vec<(String, String, Option<u64>)>,
    }

    impl<'a> Bundle<'a> {
        fn new(entry: &'a str, kind: &'a str) -> Self {
            Self {
                entry,
                kind,
                files: Vec::new(),
                decks: Vec::new(),
            }
        }

        fn file(mut self, rel: &str, bytes: &[u8]) -> Self {
            self.files.push((rel.to_string(), bytes.to_vec()));
            self
        }

        fn deck(mut self, rel: &str, deck_id: &str, saves: Option<u64>) -> Self {
            if let Some(saves) = saves {
                self.files
                    .push((document_rel(deck_id), document_bytes(deck_id, saves)));
            }
            self.decks
                .push((rel.to_string(), deck_id.to_string(), saves));
            self
        }

        fn manifest(&self, root_id: &str) -> SyncPullManifest {
            SyncPullManifest {
                version: SYNC_PULL_MANIFEST_VERSION,
                root_id: root_id.to_string(),
                entry: self.entry.to_string(),
                kind: self.kind.to_string(),
                files: self
                    .files
                    .iter()
                    .map(|(path, bytes)| SyncFileDto {
                        path: path.clone(),
                        bytes: bytes.len() as u64,
                        digest: digest(bytes),
                    })
                    .collect(),
                decks: self
                    .decks
                    .iter()
                    .map(|(path, deck_id, revision)| SyncDeckDto {
                        path: path.clone(),
                        deck_id: deck_id.clone(),
                        revision: *revision,
                    })
                    .collect(),
            }
        }

        /// Writes the unpacked pull under the root's staging directory.
        fn unpack(&self, root: &PairedRoot) -> PathBuf {
            self.unpack_with(root, self.manifest(&root.root_id()))
        }

        fn unpack_with(&self, root: &PairedRoot, manifest: SyncPullManifest) -> PathBuf {
            let dir = root.staging().join(self.entry);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).unwrap();
            }
            for (rel, bytes) in &self.files {
                let path = rel_path(&dir, rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
            let manifest_path = dir.join(MANIFEST_IN_ZIP);
            std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
            std::fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            dir
        }
    }

    fn apply(root: &PairedRoot, bundle: &Bundle) -> PullReport {
        let dir = bundle.unpack(root);
        apply_unpacked(root, &dir, &mut |_| Ok(())).unwrap()
    }

    fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        if !dir.is_dir() {
            return BTreeMap::new();
        }
        walk_files(dir)
            .unwrap()
            .into_iter()
            .map(|rel| {
                let bytes = std::fs::read(rel_path(dir, &rel)).unwrap();
                (rel, bytes)
            })
            .collect()
    }

    fn desktop_owned(root: &PairedRoot, bundle: &Bundle) -> BTreeMap<String, Vec<u8>> {
        let entry_root = root.entry_root(bundle.kind, bundle.entry);
        bundle
            .files
            .iter()
            .map(|(rel, _)| {
                (
                    rel.clone(),
                    std::fs::read(rel_path(&entry_root, rel)).unwrap(),
                )
            })
            .collect()
    }

    fn workspace_bundle<'a>() -> Bundle<'a> {
        Bundle::new("Biology", KIND_WORKSPACE)
            .file("alix.toml", b"title = \"Biology\"\n")
            .file("alix.local.toml", b"[review]\n")
            .file("decks/cells.md", b"## q\na\n")
            .file("decks/cells.local.md", b"note\n")
            .file("assets/icon.svg", b"<svg/>")
            .file(&format!("augment/{DECK_A}.json"), b"{}")
            .deck("decks/cells.md", DECK_A, Some(3))
            .deck("decks/organs.md", DECK_B, None)
            .file("decks/organs.md", b"## q2\na2\n")
    }

    fn deck_bundle<'a>() -> Bundle<'a> {
        Bundle::new("physics.md", KIND_DECK)
            .file("physics.md", b"## q\na\n")
            .file("physics.local.md", b"note\n")
            .file("alix.local.toml", b"[review]\n")
            .file(&format!("augment/{DECK_C}.json"), b"{}")
            .file(&format!("assets/{DECK_C}/one.png"), b"png")
            .deck("physics.md", DECK_C, Some(2))
    }

    #[test]
    fn a_first_pull_lands_every_entry_shape_byte_for_byte() {
        for bundle in [workspace_bundle(), deck_bundle()] {
            let (_tmp, root) = fresh_root();
            let report = apply(&root, &bundle);
            let expected: BTreeMap<String, Vec<u8>> = bundle.files.iter().cloned().collect();
            assert_eq!(
                desktop_owned(&root, &bundle),
                expected,
                "{}: tree",
                bundle.entry
            );
            assert!(
                root.manifest_path(bundle.entry).is_file(),
                "{}: manifest",
                bundle.entry
            );
            assert!(
                !root.staging().join(bundle.entry).exists(),
                "{}: staging",
                bundle.entry
            );
            assert_eq!(report.kept, Vec::<String>::new(), "{}: kept", bundle.entry);
            assert_eq!(
                report.conflicts,
                Vec::<String>::new(),
                "{}: conflicts",
                bundle.entry
            );
            let pushed = read_pushed(&root).unwrap();
            for (_, deck_id, saves) in &bundle.decks {
                match saves {
                    Some(saves) => assert_eq!(
                        pushed.get(deck_id),
                        Some(&Pushed {
                            desktop: Some(*saves),
                            phone: *saves
                        }),
                        "{}: pushed state of {deck_id}",
                        bundle.entry
                    ),
                    None => assert!(!pushed.contains_key(deck_id), "{}: no state", bundle.entry),
                }
            }
        }
    }

    #[test]
    fn a_second_pull_replaces_removes_and_carries_by_ownership() {
        let (_tmp, root) = fresh_root();
        apply(&root, &workspace_bundle());
        let entry_root = root.entry_root(KIND_WORKSPACE, "Biology");
        let mine = entry_root.join("decks/mine.md");
        std::fs::write(&mine, b"## phone\nborn\n").unwrap();
        let organs_doc = entry_root.join(document_rel(DECK_B));
        bump(&organs_doc, DECK_B);
        let second = Bundle::new("Biology", KIND_WORKSPACE)
            .file("alix.toml", b"title = \"Biology 2\"\n")
            .file("alix.local.toml", b"[review]\n")
            .file("decks/cell-biology.md", b"## q\na\n")
            .file("decks/cells.local.md", b"note\n")
            .file(&format!("augment/{DECK_A}.json"), b"{}")
            .deck("decks/cell-biology.md", DECK_A, Some(4))
            .deck("decks/organs.md", DECK_B, None)
            .file("decks/organs.md", b"## q2\na2\n");
        let report = apply(&root, &second);
        let expected: BTreeMap<String, Vec<u8>> = second.files.iter().cloned().collect();
        assert_eq!(
            desktop_owned(&root, &second),
            expected,
            "desktop-owned tree"
        );
        assert!(
            !entry_root.join("decks/cells.md").exists(),
            "renamed member's old file"
        );
        assert!(
            !entry_root.join("assets/icon.svg").exists(),
            "removed entry-level file"
        );
        assert_eq!(
            std::fs::read(&mine).unwrap(),
            b"## phone\nborn\n",
            "phone-only deck"
        );
        assert_eq!(
            revision(&organs_doc, DECK_B),
            Some(1),
            "phone-only document"
        );
        assert_eq!(report.phone_only, vec!["decks/mine.md".to_string()]);
        assert_eq!(
            report.removed,
            vec!["assets/icon.svg".to_string(), "decks/cells.md".to_string()]
        );
        assert_eq!(report.landed, vec![DECK_A.to_string()]);
        assert_eq!(
            report.kept,
            vec![DECK_B.to_string()],
            "a phone-born document the desktop still lacks"
        );
    }

    #[test]
    fn a_deleted_member_with_unpushed_progress_keeps_its_files_and_document() {
        let (_tmp, root) = fresh_root();
        apply(&root, &workspace_bundle());
        let entry_root = root.entry_root(KIND_WORKSPACE, "Biology");
        let cells_doc = entry_root.join(document_rel(DECK_A));
        bump(&cells_doc, DECK_A);
        let second = Bundle::new("Biology", KIND_WORKSPACE)
            .file("alix.toml", b"title = \"Biology\"\n")
            .file("decks/organs.md", b"## q2\na2\n")
            .deck("decks/organs.md", DECK_B, None);
        let report = apply(&root, &second);
        for rel in [
            "decks/cells.md".to_string(),
            "decks/cells.local.md".to_string(),
            format!("augment/{DECK_A}.json"),
            document_rel(DECK_A),
        ] {
            assert!(
                entry_root.join(&rel).is_file(),
                "retained member file {rel}"
            );
        }
        assert_eq!(
            revision(&cells_doc, DECK_A),
            Some(4),
            "the unpushed document keeps its revision"
        );
        assert_eq!(
            report.kept,
            vec![DECK_A.to_string()],
            "kept names the deleted member"
        );
        assert_eq!(
            report.removed,
            vec!["alix.local.toml".to_string(), "assets/icon.svg".to_string()],
            "removed excludes the retained member"
        );
        assert!(
            !report
                .phone_only
                .iter()
                .any(|p| p.starts_with("decks/cells")),
            "retained files are not phone-only rows: {:?}",
            report.phone_only
        );
    }

    #[test]
    fn a_pull_keeps_or_conflicts_an_unpushed_document_by_desktop_movement() {
        let (_tmp, root) = fresh_root();
        apply(&root, &deck_bundle());
        let doc = root.document_path(KIND_DECK, "physics.md", DECK_C);
        bump(&doc, DECK_C);
        let same_desktop = deck_bundle();
        let report = apply(&root, &same_desktop);
        assert_eq!(
            report.kept,
            vec![DECK_C.to_string()],
            "desktop unchanged: kept"
        );
        assert_eq!(revision(&doc, DECK_C), Some(3), "phone document untouched");
        assert!(
            !pulled_path(&doc).exists(),
            "no pulled copy when the desktop did not move"
        );
        assert!(conflicts(&root).unwrap().is_empty(), "no mark when kept");

        let moved = Bundle::new("physics.md", KIND_DECK)
            .file("physics.md", b"## q\na\n")
            .file("physics.local.md", b"note\n")
            .file("alix.local.toml", b"[review]\n")
            .file(&format!("augment/{DECK_C}.json"), b"{}")
            .file(&format!("assets/{DECK_C}/one.png"), b"png")
            .deck("physics.md", DECK_C, Some(5));
        let report = apply(&root, &moved);
        assert_eq!(
            report.conflicts,
            vec![DECK_C.to_string()],
            "desktop moved: conflict"
        );
        assert_eq!(revision(&doc, DECK_C), Some(3), "phone document still live");
        assert_eq!(
            revision(&pulled_path(&doc), DECK_C),
            Some(5),
            "desktop copy kept aside"
        );
        assert_eq!(
            conflicts(&root).unwrap().get(DECK_C),
            Some(&Conflict::Pull {
                pulled_revision: Some(5),
                pulled_writer: None
            })
        );
        assert_eq!(
            plan_pushes(&root).unwrap(),
            Vec::new(),
            "a conflicted deck is not pushed"
        );

        assert_eq!(
            resolve_conflict(&root, DECK_C, Choice::TakeDesktop).unwrap(),
            Resolution::Done
        );
        assert_eq!(
            revision(&doc, DECK_C),
            Some(5),
            "take-desktop swaps the pulled copy in"
        );
        assert!(!pulled_path(&doc).exists());
        assert_eq!(
            read_pushed(&root).unwrap().get(DECK_C),
            Some(&Pushed {
                desktop: Some(5),
                phone: 5
            })
        );
        assert!(conflicts(&root).unwrap().is_empty());
    }

    #[test]
    fn keep_phone_after_a_pull_conflict_pushes_over_the_desktop_revision() {
        let (_tmp, root) = fresh_root();
        apply(&root, &deck_bundle());
        let doc = root.document_path(KIND_DECK, "physics.md", DECK_C);
        bump(&doc, DECK_C);
        let mut moved = deck_bundle();
        moved.decks.clear();
        moved.files.retain(|(rel, _)| !rel.starts_with(".alix/"));
        let moved = moved.deck("physics.md", DECK_C, Some(7));
        apply(&root, &moved);
        let resolution = resolve_conflict(&root, DECK_C, Choice::KeepPhone).unwrap();
        assert_eq!(
            resolution,
            Resolution::Push(PushItem {
                deck_id: DECK_C.to_string(),
                entry: "physics.md".to_string(),
                document: doc.clone(),
                base: Some(7),
                phone_revision: 3,
            })
        );
        assert!(
            !pulled_path(&doc).exists(),
            "keep-phone discards the pulled copy"
        );
        assert_eq!(
            plan_pushes(&root).unwrap().len(),
            1,
            "the deck is pushable again"
        );
    }

    #[test]
    fn push_plan_rows() {
        let (_tmp, root) = fresh_root();
        apply(&root, &workspace_bundle());
        let cells = root.document_path(KIND_WORKSPACE, "Biology", DECK_A);
        let organs = root.document_path(KIND_WORKSPACE, "Biology", DECK_B);
        assert_eq!(
            plan_pushes(&root).unwrap(),
            Vec::new(),
            "pulled, unchanged: nothing"
        );

        bump(&cells, DECK_A);
        bump(&organs, DECK_B);
        let items = plan_pushes(&root).unwrap();
        assert_eq!(
            items,
            vec![
                PushItem {
                    deck_id: DECK_A.to_string(),
                    entry: "Biology".to_string(),
                    document: cells.clone(),
                    base: Some(3),
                    phone_revision: 4,
                },
                PushItem {
                    deck_id: DECK_B.to_string(),
                    entry: "Biology".to_string(),
                    document: organs.clone(),
                    base: None,
                    phone_revision: 1,
                },
            ],
            "reviewed after the pull, and phone-born with no desktop document"
        );

        record_push(&root, &items[0], PushOutcome::Accepted { revision: 4 }).unwrap();
        record_push(
            &root,
            &items[1],
            PushOutcome::Conflict {
                desktop_revision: Some(2),
                desktop_writer: Some(Writer {
                    device: "desk".to_string(),
                    at_ms: 5,
                }),
            },
        )
        .unwrap();
        assert_eq!(
            plan_pushes(&root).unwrap(),
            Vec::new(),
            "accepted and conflicted: nothing"
        );
        assert_eq!(
            read_pushed(&root).unwrap().get(DECK_A),
            Some(&Pushed {
                desktop: Some(4),
                phone: 4
            })
        );
        assert_eq!(
            conflicts(&root).unwrap().get(DECK_B),
            Some(&Conflict::Push {
                desktop_revision: Some(2),
                pulled_revision: None,
                desktop_writer: Some(Writer {
                    device: "desk".to_string(),
                    at_ms: 5
                })
            })
        );

        bump(&cells, DECK_A);
        assert_eq!(
            plan_pushes(&root).unwrap()[0].base,
            Some(4),
            "the next push descends from the accepted desktop revision"
        );

        let keep = resolve_conflict(&root, DECK_B, Choice::KeepPhone).unwrap();
        assert!(
            matches!(&keep, Resolution::Push(item) if item.base == Some(2) && item.phone_revision == 1),
            "keep-phone repushes with the 409's desktop revision: {keep:?}"
        );
        record_push(
            &root,
            &plan_pushes(&root).unwrap()[1],
            PushOutcome::NotServed,
        )
        .unwrap();
        assert!(organs.exists(), "an unserved deck's document stays");
    }

    #[test]
    fn take_desktop_after_a_push_conflict_lets_the_next_pull_land() {
        let (_tmp, root) = fresh_root();
        apply(&root, &deck_bundle());
        let doc = root.document_path(KIND_DECK, "physics.md", DECK_C);
        bump(&doc, DECK_C);
        let item = plan_pushes(&root).unwrap().remove(0);
        record_push(
            &root,
            &item,
            PushOutcome::Conflict {
                desktop_revision: Some(9),
                desktop_writer: None,
            },
        )
        .unwrap();
        assert_eq!(
            resolve_conflict(&root, DECK_C, Choice::TakeDesktop).unwrap(),
            Resolution::Pull {
                entry: "physics.md".to_string()
            }
        );
        let mut fresh = deck_bundle();
        fresh.decks.clear();
        fresh.files.retain(|(rel, _)| !rel.starts_with(".alix/"));
        let report = apply(&root, &fresh.deck("physics.md", DECK_C, Some(9)));
        assert_eq!(report.landed, vec![DECK_C.to_string()]);
        assert_eq!(revision(&doc, DECK_C), Some(9));
    }

    #[test]
    fn document_plan_rows() {
        let head = |revision| DocumentHead {
            revision,
            writer: None,
        };
        let state = |desktop, phone| Pushed { desktop, phone };
        let rows = [
            ("no phone document", None, None, None, DocumentPlan::Land),
            (
                "unchanged since the pull",
                Some(head(3)),
                Some(state(Some(3), 3)),
                Some(3),
                DocumentPlan::Land,
            ),
            (
                "unchanged, desktop moved",
                Some(head(3)),
                Some(state(Some(3), 3)),
                Some(4),
                DocumentPlan::Land,
            ),
            (
                "unpushed, desktop same",
                Some(head(4)),
                Some(state(Some(3), 3)),
                Some(3),
                DocumentPlan::KeepUnpushed,
            ),
            (
                "unpushed, desktop moved",
                Some(head(4)),
                Some(state(Some(3), 3)),
                Some(4),
                DocumentPlan::Conflict,
            ),
            (
                "unpushed, desktop reset",
                Some(head(4)),
                Some(state(Some(3), 3)),
                None,
                DocumentPlan::Conflict,
            ),
            (
                "phone-born, desktop none",
                Some(head(1)),
                None,
                None,
                DocumentPlan::KeepUnpushed,
            ),
            (
                "phone-born, desktop now has one",
                Some(head(1)),
                None,
                Some(1),
                DocumentPlan::Conflict,
            ),
            (
                "pushed then reviewed, desktop at the push",
                Some(head(5)),
                Some(state(Some(4), 4)),
                Some(4),
                DocumentPlan::KeepUnpushed,
            ),
        ];
        for (name, head, state, desktop, expected) in rows {
            assert_eq!(
                document_plan(head.as_ref(), state.as_ref(), desktop),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn a_crash_at_any_step_leaves_the_old_or_the_new_entry() {
        let shapes: Vec<(Bundle, Bundle)> = vec![
            (workspace_bundle(), {
                Bundle::new("Biology", KIND_WORKSPACE)
                    .file("alix.toml", b"title = \"Biology 2\"\n")
                    .file("decks/cells.md", b"## q\nchanged\n")
                    .file(&format!("augment/{DECK_A}.json"), b"{\"x\":1}")
                    .deck("decks/cells.md", DECK_A, Some(4))
            }),
            (deck_bundle(), {
                Bundle::new("physics.md", KIND_DECK)
                    .file("physics.md", b"## q\nchanged\n")
                    .file(&format!("augment/{DECK_C}.json"), b"{\"x\":1}")
                    .file(&format!("assets/{DECK_C}/two.png"), b"png2")
                    .deck("physics.md", DECK_C, Some(3))
            }),
        ];
        for (first, second) in shapes {
            let (_tmp, root) = fresh_root();
            apply(&root, &first);
            let entry_root = root.entry_root(first.kind, first.entry);
            let old_tree = tree(&entry_root);
            let steps = {
                let dir = second.unpack(&root);
                let mut count = 0;
                apply_unpacked(&root, &dir, &mut |_| {
                    count += 1;
                    Ok(())
                })
                .unwrap();
                count
            };
            let new_tree = tree(&entry_root);
            assert!(steps >= 3, "{}: {steps} steps", first.entry);
            assert_ne!(
                old_tree, new_tree,
                "{}: the second pull changes the tree",
                first.entry
            );
            for fail_at in 1..=steps {
                let (_tmp, root) = fresh_root();
                apply(&root, &first);
                let entry_root = root.entry_root(first.kind, first.entry);
                let dir = second.unpack(&root);
                let mut count = 0;
                let result = apply_unpacked(&root, &dir, &mut |step| {
                    count += 1;
                    if count == fail_at {
                        bail!("crash at {step}");
                    }
                    Ok(())
                });
                assert!(result.is_err(), "{}: step {fail_at} must fail", first.entry);
                recover(&root).unwrap();
                let live = tree(&entry_root);
                if first.kind == KIND_WORKSPACE {
                    assert!(
                        live == old_tree || live == new_tree,
                        "{}: step {fail_at} left a mixed tree: {:?}",
                        first.entry,
                        live.keys().collect::<Vec<_>>()
                    );
                } else {
                    let deck_file = first.entry.to_string();
                    let deck_is_new = live.get(&deck_file) == new_tree.get(&deck_file);
                    for (rel, bytes) in &live {
                        if rel.starts_with(".alix/pull/")
                            || rel.starts_with(".alix/pushed")
                            || rel.starts_with(".alix/conflicts")
                        {
                            continue;
                        }
                        let is_old = old_tree.get(rel) == Some(bytes);
                        let is_new = new_tree.get(rel) == Some(bytes);
                        assert!(
                            is_old || is_new,
                            "{}: step {fail_at} left `{rel}` neither old nor new",
                            first.entry
                        );
                        if deck_is_new && new_tree.contains_key(rel) {
                            assert!(
                                is_new,
                                "{}: step {fail_at}: new deck file beside old `{rel}`",
                                first.entry
                            );
                        }
                    }
                }
                assert!(
                    !root.staging().exists(),
                    "{}: step {fail_at}: staging cleared",
                    first.entry
                );
            }
        }
    }

    #[test]
    fn recover_rows() {
        let (_tmp, root) = fresh_root();
        let entry = root.dir().join("Biology");
        let old = old_path(&entry);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("alix.toml"), b"old").unwrap();
        assert_eq!(
            recover(&root).unwrap(),
            vec!["restored Biology".to_string()]
        );
        assert_eq!(
            std::fs::read(entry.join("alix.toml")).unwrap(),
            b"old",
            "sibling absent: restored"
        );

        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(root.staging()).unwrap();
        assert_eq!(
            recover(&root).unwrap(),
            vec![
                "removed Biology.old".to_string(),
                "cleared staging".to_string()
            ]
        );
        assert!(
            !old.exists() && !root.staging().exists(),
            "both present: old removed"
        );

        let assets_old = root.dir().join("assets").join(format!("{DECK_C}.old"));
        std::fs::create_dir_all(&assets_old).unwrap();
        assert_eq!(recover(&root).unwrap(), vec![format!("restored {DECK_C}")]);
        assert!(root.dir().join("assets").join(DECK_C).is_dir());
    }

    #[test]
    fn orphan_rows() {
        let (_tmp, root) = fresh_root();
        apply(&root, &workspace_bundle());
        apply(&root, &deck_bundle());
        let listed = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();

        assert_eq!(
            tidy_renamed(&root, &listed(&["Biology", "physics.md"])).unwrap(),
            Vec::new(),
            "listed entries are untouched"
        );
        assert_eq!(
            tidy_renamed(&root, &listed(&["physics.md"])).unwrap(),
            Vec::new(),
            "a deleted entry stays"
        );
        assert!(root.dir().join("Biology").is_dir());

        let renamed = Bundle::new("Life", KIND_WORKSPACE)
            .file("alix.toml", b"title = \"Life\"\n")
            .file("decks/cells.md", b"## q\na\n")
            .deck("decks/cells.md", DECK_A, Some(3))
            .deck("decks/organs.md", DECK_B, None)
            .file("decks/organs.md", b"## q2\na2\n");
        apply(&root, &renamed);
        let cells = root.document_path(KIND_WORKSPACE, "Biology", DECK_A);
        bump(&cells, DECK_A);
        assert_eq!(
            tidy_renamed(&root, &listed(&["Life", "physics.md"])).unwrap(),
            Vec::new(),
            "an orphan with unpushed progress stays"
        );
        record_push(
            &root,
            &plan_pushes(&root).unwrap().remove(0),
            PushOutcome::Accepted { revision: 4 },
        )
        .unwrap();
        std::fs::write(root.dir().join("Biology/decks/mine.md"), b"## phone\n").unwrap();
        assert_eq!(
            tidy_renamed(&root, &listed(&["Life", "physics.md"])).unwrap(),
            Vec::new(),
            "an orphan holding a phone-born member stays"
        );
        std::fs::remove_file(root.dir().join("Biology/decks/mine.md")).unwrap();
        assert_eq!(
            tidy_renamed(&root, &listed(&["Life", "physics.md"])).unwrap(),
            vec![("Biology".to_string(), "Life".to_string())],
            "a clean renamed orphan is tidied"
        );
        assert!(!root.dir().join("Biology").exists());
        assert!(!root.manifest_path("Biology").exists());
        assert_eq!(
            read_pushed(&root).unwrap().get(DECK_A),
            Some(&Pushed {
                desktop: Some(4),
                phone: 4
            }),
            "state of a deck the new entry still carries survives"
        );

        let physics_renamed = Bundle::new("mechanics.md", KIND_DECK)
            .file("mechanics.md", b"## q\na\n")
            .file("alix.local.toml", b"[review]\n")
            .file(&format!("augment/{DECK_C}.json"), b"{}")
            .file(&format!("assets/{DECK_C}/one.png"), b"png")
            .deck("mechanics.md", DECK_C, Some(2));
        apply(&root, &physics_renamed);
        assert_eq!(
            tidy_renamed(&root, &listed(&["Life", "mechanics.md"])).unwrap(),
            vec![("physics.md".to_string(), "mechanics.md".to_string())]
        );
        assert!(
            !root.dir().join("physics.md").exists(),
            "old deck file removed"
        );
        assert!(
            !root.dir().join("physics.local.md").exists(),
            "old sidecar removed"
        );
        assert!(root.dir().join("mechanics.md").exists());
        assert!(
            root.dir().join(format!("augment/{DECK_C}.json")).exists(),
            "shared augment kept"
        );
        assert!(
            root.dir().join(format!("assets/{DECK_C}/one.png")).exists(),
            "shared assets kept"
        );
        assert!(
            root.document_path(KIND_DECK, "mechanics.md", DECK_C)
                .exists(),
            "shared document kept"
        );
        assert!(
            root.dir().join("alix.local.toml").exists(),
            "the root's own local manifest kept"
        );
    }

    #[test]
    fn a_refused_pull_moves_nothing() {
        let (_tmp, root) = fresh_root();
        apply(&root, &deck_bundle());
        let live = |root: &PairedRoot| {
            let mut all = tree(root.dir());
            all.retain(|rel, _| !rel.starts_with(".alix/staging/"));
            all
        };
        let before = live(&root);
        type ManifestEdit = fn(&mut SyncPullManifest);
        type DirEdit = fn(&Path);
        let none_m: Option<ManifestEdit> = None;
        let none_d: Option<DirEdit> = None;
        let cases: Vec<(&str, Option<ManifestEdit>, Option<DirEdit>)> = vec![
            (
                "another root",
                Some(|m| m.root_id = "root-other".to_string()),
                none_d,
            ),
            (
                "a parent path",
                Some(|m| m.files[0].path = "../x".to_string()),
                none_d,
            ),
            (
                "phone state",
                Some(|m| m.files[0].path = ".alix/pushed.json".to_string()),
                none_d,
            ),
            (
                "another entry's file",
                Some(|m| m.files[0].path = "Life/decks/x.md".to_string()),
                none_d,
            ),
            (
                "an unknown kind",
                Some(|m| m.kind = "folder".to_string()),
                none_d,
            ),
            (
                "a nested entry name",
                Some(|m| m.entry = "a/b".to_string()),
                none_d,
            ),
            ("a wrong version", Some(|m| m.version = 2), none_d),
            (
                "a bad deck id",
                Some(|m| m.decks[0].deck_id = "deck-x".to_string()),
                none_d,
            ),
            (
                "a revision without a document",
                Some(|m| m.files.retain(|f| !f.path.starts_with(".alix/"))),
                none_d,
            ),
            (
                "a wrong digest",
                Some(|m| m.files[0].digest = "xxh64-0000000000000000".to_string()),
                none_d,
            ),
            (
                "a wrong byte count",
                Some(|m| m.files[0].bytes += 1),
                none_d,
            ),
            (
                "an unlisted file",
                none_m,
                Some(|dir| std::fs::write(dir.join("extra.md"), b"x").unwrap()),
            ),
            (
                "a missing file",
                none_m,
                Some(|dir| std::fs::remove_file(dir.join("physics.local.md")).unwrap()),
            ),
            (
                "a link",
                none_m,
                Some(|dir| {
                    std::fs::remove_file(dir.join("physics.local.md")).unwrap();
                    std::os::unix::fs::symlink(
                        dir.join("physics.md"),
                        dir.join("physics.local.md"),
                    )
                    .unwrap();
                }),
            ),
        ];
        for (name, edit_manifest, edit_dir) in cases {
            let bundle = deck_bundle();
            let mut manifest = bundle.manifest(&root.root_id());
            if let Some(edit) = edit_manifest {
                edit(&mut manifest);
            }
            let dir = bundle.unpack_with(&root, manifest);
            if let Some(edit) = edit_dir {
                edit(&dir);
            }
            let result = apply_unpacked(&root, &dir, &mut |_| Ok(()));
            assert!(result.is_err(), "{name}: must be refused");
            std::fs::remove_dir_all(&dir).unwrap();
            assert_eq!(live(&root), before, "{name}: nothing moved");
        }
    }

    #[test]
    fn state_files_fail_loud_on_unknown_fields_and_versions() {
        let (_tmp, root) = fresh_root();
        std::fs::create_dir_all(root.pushed_path().parent().unwrap()).unwrap();
        std::fs::write(root.pushed_path(), br#"{"version":1,"decks":{},"extra":1}"#).unwrap();
        assert!(read_pushed(&root).is_err(), "unknown field");
        std::fs::write(root.pushed_path(), br#"{"version":2,"decks":{}}"#).unwrap();
        assert!(read_pushed(&root).is_err(), "version");
        std::fs::write(
            root.conflicts_path(),
            br#"{"version":1,"decks":{"deck-x":{"kind":"lost"}}}"#,
        )
        .unwrap();
        assert!(conflicts(&root).is_err(), "unknown conflict kind");
    }

    #[test]
    fn needs_space_saturates() {
        assert_eq!(needs_space(3, 4), 7);
        assert_eq!(needs_space(u64::MAX, 1), u64::MAX);
    }

    #[cfg(all(unix, feature = "sync-client"))]
    #[test]
    fn free_space_reads_the_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(free_space(tmp.path()).unwrap() > 0);
        assert!(free_space(&tmp.path().join("missing")).is_err());
    }

    #[cfg(feature = "sync-client")]
    #[test]
    fn a_zip_pull_unpacks_into_staging_and_applies() {
        use std::io::Write;
        let (_tmp, root) = fresh_root();
        let bundle = deck_bundle();
        let manifest = bundle.manifest(&root.root_id());
        let zip_path = root.dir().parent().unwrap().join("pull.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let options: zip::write::SimpleFileOptions = Default::default();
        for (rel, bytes) in &bundle.files {
            zip.start_file(rel.clone(), options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.start_file(MANIFEST_IN_ZIP, options).unwrap();
        zip.write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        zip.finish().unwrap();
        let report = apply_pull(&root, "physics.md", &zip_path, &mut |_| Ok(())).unwrap();
        assert_eq!(report.landed, vec![DECK_C.to_string()]);
        assert_eq!(
            std::fs::read(root.dir().join("physics.md")).unwrap(),
            b"## q\na\n"
        );
        assert!(!root.staging().join("physics.md").exists());
    }
}
