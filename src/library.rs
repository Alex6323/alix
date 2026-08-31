use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use thiserror::Error;

use crate::{
    assets,
    augment::AugmentCache,
    deck::Deck,
    parser,
    store::Store,
    workspace::{self, WorkspaceFiles},
};

#[derive(Debug)]
pub struct Placed {
    pub path: PathBuf,
    pub cards: usize,
    pub parse_error: Option<String>,
}

pub fn place_deck(dir: &Path, name: &str, text: &str) -> Result<Placed> {
    let text = &parser::normalize(text);
    let stem = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deck");
    let stem = stem.strip_suffix(".md").unwrap_or(stem);
    let file = format!("{stem}.md");
    let path = dir.join(&file);
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    let parsed = parser::parse_str(&file, text);
    write_body(&path, text)?;
    match parsed {
        Ok(cards) => {
            if let Err(error) = assets::initialize(&path) {
                let _ = std::fs::remove_file(&path);
                return Err(error.into());
            }
            Ok(Placed {
                path,
                cards: cards.len(),
                parse_error: None,
            })
        }
        Err(e) => Ok(Placed {
            path,
            cards: 0,
            parse_error: Some(format!("{e:#}")),
        }),
    }
}

fn write_body(path: &Path, text: &str) -> Result<()> {
    let body = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    };
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("deck");
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    let tmp = parent.join(format!(".{name}.tmp"));
    crate::fsio::replace_file(&tmp, path, body.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

#[derive(Debug)]
pub struct ReplaceReport {
    pub minted: usize,
    pub wiped_cards: usize,
}

pub fn replace_deck(
    dir: &Path,
    name: &str,
    text: &str,
    store: &mut Store,
) -> Result<ReplaceReport> {
    let text = &parser::normalize(text);
    let stem = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deck");
    let stem = stem.strip_suffix(".md").unwrap_or(stem);
    let file = format!("{stem}.md");
    let path = dir.join(&file);

    if let Err(e) = parser::parse(&file, text) {
        let rej = dir.join(format!("{stem}.rej"));
        write_body(&rej, text)?;
        bail!(
            "the replacement for {} does not parse; wrote it aside to {} and left the existing file untouched: {e:#}",
            path.display(),
            rej.display()
        );
    }

    // Lenient on the OLD file: a corrupt old deck under-wipes rather than
    // blocking its replacement.
    let mut old_card_tokens: HashSet<String> = HashSet::new();
    let mut old_deck_tokens: HashSet<String> = HashSet::new();
    let old_text = std::fs::read_to_string(&path).unwrap_or_default();
    if let Ok(old) = parser::parse(&file, &old_text) {
        for card in &old.cards {
            if let Some(token) = card.token.as_deref() {
                old_card_tokens.insert(token.to_string());
            }
        }
        if let Some(token) = old.deck_token {
            old_deck_tokens.insert(token);
        }
    }

    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let workspace_root = workspace::root_for_member_dir(dir).unwrap_or_else(|| dir.to_path_buf());
    let workspace_files = WorkspaceFiles::new(&workspace_root);
    let mut trio = TrioBackup::default();
    trio.back_up(&path)
        .with_context(|| format!("cannot keep {} as its backup", path.display()))?;
    if old_deck_tokens.len() == 1
        && let Some(deck_id) = old_deck_tokens.iter().next()
    {
        let progress = match crate::state::progress_document_for(store.path(), deck_id)
            .context("locating the replaced deck's progress document")
        {
            Ok(progress) => progress,
            Err(error) => {
                trio.restore(&path)?;
                return Err(error);
            }
        };
        // Copied, not renamed: the store's save re-reads the on-disk
        // revision, and a renamed-away document reads as a stale 0.
        if let Err(error) = trio.back_up_copy(&progress) {
            trio.restore(&path)?;
            return Err(error);
        }
        if let Err(error) = trio.back_up(&workspace_files.augment_for(deck_id)) {
            trio.restore(&path)?;
            return Err(error);
        }
    }
    if let Err(error) = write_body(&path, text) {
        trio.restore(&path)?;
        return Err(error);
    }

    let minted = match assets::initialize(&path) {
        Ok(outcome) => outcome.stamp.minted_cards.len(),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            trio.restore(&path)?;
            return Err(error.into());
        }
    };

    // The store saves first: a failed augment save then strands only
    // unreachable cache entries, never store orphans.
    let old_deck_id = old_deck_tokens
        .iter()
        .next()
        .map(String::as_str)
        .unwrap_or_default();
    let wiped_cards = store.wipe_deck(&old_card_tokens, old_deck_id);
    if let Err(error) = store
        .save()
        .context("saving the store after replacing a deck")
    {
        trio.restore(&path)?;
        return Err(error);
    }
    let cache_path = workspace_files.augment();
    if cache_path.exists() {
        let mut cache = match AugmentCache::open_for_workspace(&workspace_root) {
            Ok(cache) => cache,
            Err(error) => {
                trio.restore(&path)?;
                return Err(error.into());
            }
        };
        if cache.wipe_tokens(&old_card_tokens, &old_deck_tokens)
            && let Err(error) = cache
                .save()
                .with_context(|| format!("cannot save {}", cache_path.display()))
        {
            trio.restore(&path)?;
            return Err(error);
        }
    }
    if old_deck_tokens.len() == 1
        && let Some(deck_id) = old_deck_tokens.iter().next()
    {
        if let Err(error) = crate::state::retire_replaced_progress(store.path(), deck_id)
            .context("retiring the replaced deck's progress")
        {
            trio.restore(&path)?;
            return Err(error);
        }
        let replacement = match Deck::load(&path).context("loading the stamped replacement deck") {
            Ok(replacement) => replacement,
            Err(error) => {
                trio.restore(&path)?;
                return Err(error);
            }
        };
        if let Err(error) = store
            .rebind_replaced_deck(deck_id, &replacement)
            .context("binding state to the replacement deck identity")
        {
            trio.restore(&path)?;
            return Err(error);
        }
    }

    Ok(ReplaceReport {
        minted,
        wiped_cards,
    })
}

#[derive(Debug, Default)]
pub struct RemovalPreview {
    pub files: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
    pub cards_with_progress: usize,
    pub earliest_review_ms: Option<u64>,
    pub dependents: Vec<String>,
}

#[derive(Debug)]
pub struct RemovalReport {
    pub removed: Vec<PathBuf>,
    pub dependents: Vec<String>,
}

#[derive(Debug)]
pub struct WorkspaceRemovalPreview {
    pub files: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
    pub decks: usize,
    pub cards_with_progress: usize,
    pub earliest_review_ms: Option<u64>,
    pub dependents: Vec<String>,
}

#[derive(Debug)]
pub struct WorkspaceRemovalReport {
    pub removed: Vec<PathBuf>,
    pub decks_removed: usize,
    pub root_removed: bool,
    pub dependents: Vec<String>,
}

#[derive(Debug, Error)]
#[error("cannot remove {failed}: {source}")]
pub struct RemovalFailure {
    pub removed: Vec<PathBuf>,
    pub failed: PathBuf,
    #[source]
    source: io::Error,
}

#[derive(Debug)]
enum RemovalItem {
    File(PathBuf),
    Directory(PathBuf),
}

impl RemovalItem {
    fn path(&self) -> &Path {
        match self {
            Self::File(path) | Self::Directory(path) => path,
        }
    }
}

#[derive(Debug, Default)]
struct RemovalPlan {
    items: Vec<RemovalItem>,
}

impl RemovalPlan {
    fn extend(&mut self, files: Vec<PathBuf>, directories: Vec<PathBuf>) {
        self.items.extend(files.into_iter().map(RemovalItem::File));
        self.items
            .extend(directories.into_iter().map(RemovalItem::Directory));
    }

    fn file_if_present(&mut self, path: PathBuf) {
        if path.is_file() {
            self.items.push(RemovalItem::File(path));
        }
    }

    fn directory_if_present(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.items.push(RemovalItem::Directory(path));
        }
    }
}

/// What `remove_deck` will destroy, computed for the confirmation prompt:
/// the exact file set plus the stakes (how much earned history goes).
pub fn removal_preview(deck_path: &Path, store: &Store) -> RemovalPreview {
    let (files, directories, deck) = removal_set(deck_path, store);
    let mut preview = RemovalPreview {
        files,
        directories,
        dependents: crate::deck::dependents(deck_path),
        ..RemovalPreview::default()
    };
    if let Some(deck) = deck {
        let mut earliest: Option<u64> = None;
        for card in &deck.cards {
            let Some(state) = card.id().as_deref().and_then(|id| store.get(id)) else {
                continue;
            };
            preview.cards_with_progress += 1;
            if let Some(seen) = state.introduced_ms {
                earliest = Some(earliest.map_or(seen, |e| e.min(seen)));
            }
        }
        preview.earliest_review_ms = earliest;
    }
    preview
}

/// Deletes the deck and every artifact that is its alone: the deck file,
/// its progress document, its frozen assets, its augment sidecar file, and
/// any `.bak` of those. The removal is total: no backup is written and
/// existing backups go too. Deletion is deck-file-first, so a
/// mid-set failure leaves only the orphan class doctor already detects.
pub fn remove_deck(deck_path: &Path, store: &Store) -> Result<RemovalReport> {
    let dependents = crate::deck::dependents(deck_path);
    let (files, directories, _) = removal_set(deck_path, store);
    let mut plan = RemovalPlan::default();
    plan.extend(files, directories);
    let removed = execute_removal_plan(plan)?;
    Ok(RemovalReport {
        removed,
        dependents,
    })
}

pub fn workspace_removal_preview(
    workspace_root: &Path,
    store: &Store,
) -> Result<WorkspaceRemovalPreview> {
    workspace_removal_plan(workspace_root, store).map(|(_, preview)| preview)
}

pub fn remove_workspace(workspace_root: &Path, store: &Store) -> Result<WorkspaceRemovalReport> {
    let (plan, preview) = workspace_removal_plan(workspace_root, store)?;
    let mut removed = execute_removal_plan(plan)?;
    remove_if_empty(&workspace_root.join(workspace::DECKS), &mut removed)?;
    remove_if_empty(workspace_root, &mut removed)?;
    Ok(WorkspaceRemovalReport {
        decks_removed: preview.decks,
        root_removed: !workspace_root.exists(),
        dependents: preview.dependents,
        removed,
    })
}

fn workspace_removal_plan(
    workspace_root: &Path,
    store: &Store,
) -> Result<(RemovalPlan, WorkspaceRemovalPreview)> {
    if !workspace::has_manifest(workspace_root) {
        bail!("{} is not a workspace", workspace_root.display());
    }
    let members = workspace::classify_deck_files(workspace_root)
        .with_context(|| format!("cannot read {}", workspace_root.display()))?
        .initialized;
    let mut plan = RemovalPlan::default();
    let mut preview = WorkspaceRemovalPreview {
        files: Vec::new(),
        directories: Vec::new(),
        decks: members.len(),
        cards_with_progress: 0,
        earliest_review_ms: None,
        dependents: Vec::new(),
    };
    for member in members {
        let member_preview = removal_preview(&member, store);
        preview.cards_with_progress += member_preview.cards_with_progress;
        if let Some(earliest) = member_preview.earliest_review_ms {
            preview.earliest_review_ms = Some(
                preview
                    .earliest_review_ms
                    .map_or(earliest, |current| current.min(earliest)),
            );
        }
        for dependent in member_preview.dependents {
            if !preview.dependents.contains(&dependent) {
                preview.dependents.push(dependent);
            }
        }
        preview.files.extend(member_preview.files.iter().cloned());
        preview
            .directories
            .extend(member_preview.directories.iter().cloned());
        plan.extend(member_preview.files, member_preview.directories);
    }

    let workspace_files = WorkspaceFiles::new(workspace_root);
    for directory in [workspace_files.assets(), workspace_files.augment()] {
        if directory.is_dir() {
            preview.directories.push(directory.clone());
            plan.directory_if_present(directory);
        }
    }
    if let Some(user_root) = user_root_for_store(store.path())
        && path_is_within(&user_root, workspace_root)
    {
        let files = crate::state::UserFiles::new(&user_root);
        if files.progress().is_dir() {
            preview.directories.push(files.progress());
            plan.directory_if_present(files.progress());
        }
        if files.recent().is_file() {
            preview.files.push(files.recent());
            plan.file_if_present(files.recent());
        }
    }
    let local = crate::state::UserFiles::new(workspace_root).local_manifest();
    if local.is_file() {
        preview.files.push(local.clone());
        plan.file_if_present(local);
    }
    let manifest = workspace_files.manifest();
    preview.files.push(manifest.clone());
    plan.file_if_present(manifest);
    Ok((plan, preview))
}

fn execute_removal_plan(plan: RemovalPlan) -> std::result::Result<Vec<PathBuf>, RemovalFailure> {
    let mut removed = Vec::new();
    for item in plan.items {
        #[cfg(test)]
        let result = if injected_removal_failure(item.path()) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected removal failure",
            ))
        } else {
            remove_item(&item)
        };
        #[cfg(not(test))]
        let result = remove_item(&item);
        if let Err(source) = result {
            return Err(RemovalFailure {
                removed,
                failed: item.path().to_path_buf(),
                source,
            });
        }
        removed.push(item.path().to_path_buf());
    }
    Ok(removed)
}

fn remove_item(item: &RemovalItem) -> io::Result<()> {
    match item {
        RemovalItem::File(path) => std::fs::remove_file(path),
        RemovalItem::Directory(path) => std::fs::remove_dir_all(path),
    }
}

#[cfg(test)]
std::thread_local! {
    static INJECTED_REMOVAL_FAILURE: std::cell::RefCell<Option<PathBuf>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
fn injected_removal_failure(path: &Path) -> bool {
    INJECTED_REMOVAL_FAILURE.with(|failure| failure.borrow().as_deref() == Some(path))
}

#[cfg(test)]
fn with_removal_failure_at<T>(path: &Path, run: impl FnOnce() -> T) -> T {
    INJECTED_REMOVAL_FAILURE.with(|failure| {
        *failure.borrow_mut() = Some(path.to_path_buf());
    });
    let output = run();
    INJECTED_REMOVAL_FAILURE.with(|failure| {
        *failure.borrow_mut() = None;
    });
    output
}

fn remove_if_empty(
    path: &Path,
    removed: &mut Vec<PathBuf>,
) -> std::result::Result<(), RemovalFailure> {
    match std::fs::remove_dir(path) {
        Ok(()) => removed.push(path.to_path_buf()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(source) => {
            return Err(RemovalFailure {
                removed: removed.clone(),
                failed: path.to_path_buf(),
                source,
            });
        }
    }
    Ok(())
}

fn user_root_for_store(store_path: &Path) -> Option<PathBuf> {
    if store_path
        .file_name()
        .is_some_and(|name| name == "progress")
    {
        return store_path.parent().map(Path::to_path_buf);
    }
    if store_path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "progress")
    {
        return store_path.parent()?.parent().map(Path::to_path_buf);
    }
    None
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    match (std::fs::canonicalize(path), std::fs::canonicalize(root)) {
        (Ok(path), Ok(root)) => path.starts_with(root),
        _ => {
            path.starts_with(root)
                && !path
                    .components()
                    .any(|component| component == std::path::Component::ParentDir)
        }
    }
}

/// The deck's own artifact set, in deletion order. Lenient on a corrupt or
/// tokenless deck: it is still removable, just with nothing derivable from
/// a token (progress, assets, augment) in the set.
fn removal_set(deck_path: &Path, store: &Store) -> (Vec<PathBuf>, Vec<PathBuf>, Option<Deck>) {
    let mut files = Vec::new();
    let mut directories = Vec::new();
    if deck_path.exists() {
        files.push(deck_path.to_path_buf());
    }
    let deck = Deck::load(deck_path).ok();
    // Both sides' tokens: the deck's own .bak carries the pre-replace
    // identity, whose progress/augment backups are not derivable from the
    // live token.
    let mut tokens: Vec<String> = Vec::new();
    for side_token in [
        deck.as_ref().and_then(|d| d.deck_token.clone()),
        bak_sibling(deck_path)
            .and_then(|bak| Deck::load(&bak).ok())
            .and_then(|d| d.deck_token),
    ]
    .into_iter()
    .flatten()
    {
        if !tokens.contains(&side_token) {
            tokens.push(side_token);
        }
    }
    let dir = deck_path.parent().unwrap_or_else(|| Path::new("."));
    let workspace_root = workspace::root_for_member_dir(dir).unwrap_or_else(|| dir.to_path_buf());
    let workspace_files = WorkspaceFiles::new(workspace_root);
    for token in &tokens {
        if let Ok(progress) = crate::state::progress_document_for(store.path(), token) {
            files.extend(existing_with_bak(&progress));
        }
        files.extend(existing_with_bak(&workspace_files.augment_for(token)));
        let assets = workspace_files.assets_for(token);
        if assets.is_dir() {
            directories.push(assets);
        }
    }
    if let Some(bak) = bak_sibling(deck_path) {
        files.push(bak);
    }
    (files, directories, deck)
}

fn existing_with_bak(live: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if live.exists() {
        out.push(live.to_path_buf());
    }
    if let Some(bak) = bak_sibling(live) {
        out.push(bak);
    }
    out
}

fn bak_sibling(live: &Path) -> Option<PathBuf> {
    let name = live.file_name()?.to_str()?;
    let bak = live.with_file_name(format!("{name}.bak"));
    bak.exists().then_some(bak)
}

#[derive(Debug, Default, PartialEq)]
pub struct RestoreReport {
    pub deck: bool,
    pub progress: bool,
    pub augment: bool,
}

/// Swaps the deck's live trio with its `.bak` trio (deck file, progress
/// document, per-deck augment file). Self-inverse: running it twice is a
/// byte-identical round trip. Members are paired per side's own deck token,
/// because a replacement mints fresh tokens: the `.bak` deck's documents go
/// live and the live deck's documents become the new `.bak`s.
pub fn restore_deck(deck_path: &Path, store_root: &Path) -> Result<RestoreReport> {
    let bak_deck = deck_path.with_file_name(format!(
        "{}.bak",
        deck_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("deck.md")
    ));
    if !bak_deck.exists() {
        bail!("nothing to restore: {} does not exist", bak_deck.display());
    }
    let dir = deck_path.parent().unwrap_or_else(|| Path::new("."));
    let workspace_root = workspace::root_for_member_dir(dir).unwrap_or_else(|| dir.to_path_buf());
    let workspace_files = WorkspaceFiles::new(workspace_root);
    let progress_dir = store_root.join("progress");

    // Lenient on both sides, like replace is on the old file: an
    // unparseable side contributes no document pairs, never an abort.
    let token_of = |path: &Path| -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        let name = path.file_name()?.to_str()?;
        parser::parse(name, &text).ok()?.deck_token
    };
    let mut tokens: Vec<String> = Vec::new();
    for side in [deck_path, &bak_deck] {
        if let Some(token) = token_of(side)
            && !tokens.contains(&token)
        {
            tokens.push(token);
        }
    }

    swap_with_bak(deck_path)?;
    let mut report = RestoreReport {
        deck: true,
        ..RestoreReport::default()
    };
    for token in &tokens {
        if swap_with_bak(&progress_dir.join(format!("{token}.json")))? {
            report.progress = true;
        }
        if swap_with_bak(&workspace_files.augment_for(token))? {
            report.augment = true;
        }
    }
    Ok(report)
}

/// Exchanges `live` and `live.bak`, tolerating an absent side: both present
/// is a three-rename swap through a temp name, one present is a move.
/// Returns whether anything moved.
fn swap_with_bak(live: &Path) -> Result<bool> {
    let name = live
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("member");
    let bak = live.with_file_name(format!("{name}.bak"));
    let io =
        |from: &Path, to: &Path| format!("cannot swap {} with {}", from.display(), to.display());
    match (live.exists(), bak.exists()) {
        (true, true) => {
            let tmp = live.with_file_name(format!(".{name}.swap.tmp"));
            std::fs::rename(live, &tmp).with_context(|| io(live, &tmp))?;
            std::fs::rename(&bak, live).with_context(|| io(&bak, live))?;
            std::fs::rename(&tmp, &bak).with_context(|| io(&tmp, &bak))?;
            Ok(true)
        }
        (true, false) => {
            std::fs::rename(live, &bak).with_context(|| io(live, &bak))?;
            Ok(true)
        }
        (false, true) => {
            std::fs::rename(&bak, live).with_context(|| io(&bak, live))?;
            Ok(true)
        }
        (false, false) => Ok(false),
    }
}

/// The rename ledger behind replace's backup trio (deck file, progress
/// document, per-deck augment file): every `live -> live.bak` rename is
/// recorded so a failure after any of them can put all of them back.
#[derive(Default)]
struct TrioBackup {
    renamed: Vec<(PathBuf, PathBuf)>,
}

impl TrioBackup {
    fn back_up(&mut self, live: &Path) -> Result<()> {
        let Some(bak) = Self::bak_path(live) else {
            return Ok(());
        };
        std::fs::rename(live, &bak)
            .with_context(|| format!("cannot keep {} as {}", live.display(), bak.display()))?;
        self.renamed.push((live.to_path_buf(), bak));
        Ok(())
    }

    /// The live file stays in place for writers that re-read it (the store's
    /// revision guard); restore still renames the copy back over it.
    fn back_up_copy(&mut self, live: &Path) -> Result<()> {
        let Some(bak) = Self::bak_path(live) else {
            return Ok(());
        };
        std::fs::copy(live, &bak)
            .with_context(|| format!("cannot copy {} to {}", live.display(), bak.display()))?;
        self.renamed.push((live.to_path_buf(), bak));
        Ok(())
    }

    fn bak_path(live: &Path) -> Option<PathBuf> {
        if !live.exists() {
            return None;
        }
        let name = live.file_name().and_then(|n| n.to_str())?;
        Some(live.with_file_name(format!("{name}.bak")))
    }

    fn restore(&mut self, subject: &Path) -> Result<()> {
        while let Some((live, bak)) = self.renamed.pop() {
            std::fs::rename(&bak, &live).with_context(|| {
                format!(
                    "cannot restore {} after replacing {} failed",
                    live.display(),
                    subject.display()
                )
            })?;
        }
        Ok(())
    }
}

pub fn reset_decks<'a>(
    store: &mut Store,
    decks: impl IntoIterator<Item = &'a Deck>,
) -> Result<usize> {
    let mut n = 0;
    for deck in decks {
        let deck_id = deck.deck_token.as_deref().unwrap_or_default();
        store.clear_deck_mastered(deck_id);
        // The personal file itself is the user's, like the deck: a reset
        // clears its cards' schedules and leaves the content alone.
        let ids = deck
            .cards
            .iter()
            .filter_map(crate::card::Card::id)
            .chain(crate::personal::card_ids(deck))
            .chain(deck.dormant_base_ids());
        for id in ids {
            if store.get(&id).is_some() {
                store.remove(&id);
                n += 1;
            }
        }
    }
    store.save()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn replacing_a_linked_member_writes_a_file_rather_than_through_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join("decks");
        std::fs::create_dir(&decks).unwrap();
        let outside = dir.path().join("outside.md");
        let original = "---\nid: deck-1xpgnc8f1mypv80cgzyxrn2cqf\n---\n\
                        ## original\nkeep me\n\
                        <!-- id: card-1xpgnc8f1mypv80cgzyxrn2cqf -->\n";
        std::fs::write(&outside, original).unwrap();
        std::os::unix::fs::symlink(&outside, decks.join("x.md")).unwrap();
        let mut store = crate::state::open_store(&decks.join("x.md"), &decks).unwrap();

        replace_deck(&decks, "x.md", "## new\nreplaced\n", &mut store).unwrap();

        assert_eq!(
            original,
            std::fs::read_to_string(&outside).unwrap(),
            "a replacement must not reach a file outside the decks folder"
        );
        assert!(
            !std::fs::symlink_metadata(decks.join("x.md"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "the replacement takes the name as a regular file"
        );
    }

    #[test]
    fn placing_a_valid_deck_writes_it_and_counts_cards() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "rust", "## q\na\n").unwrap();
        assert_eq!(dir.path().join("rust.md"), p.path);
        assert_eq!(1, p.cards);
        assert!(p.parse_error.is_none());
        assert!(p.path.exists());
    }

    #[test]
    fn placed_decks_land_stamped() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "rust", "## q\na\n## r\nb\n").unwrap();
        let deck =
            crate::parser::parse("rust.md", &std::fs::read_to_string(&p.path).unwrap()).unwrap();
        assert!(deck.deck_token.is_some(), "deck id minted");
        assert!(
            deck.cards.iter().all(|c| c.id().is_some()),
            "every card stamped"
        );
    }

    #[test]
    fn failed_workspace_freezing_removes_the_uninitialized_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join(crate::workspace::DECKS);
        std::fs::create_dir(&decks).unwrap();
        std::fs::write(dir.path().join(crate::workspace::MANIFEST), "").unwrap();

        let error =
            place_deck(&decks, "facts", "---\nsource: missing.md\n---\n## q\na\n").unwrap_err();

        assert!(format!("{error:#}").contains("missing.md"));
        assert!(!decks.join("facts.md").exists());
        assert!(!dir.path().join(crate::assets::ROOT).exists());
    }

    #[test]
    fn a_parse_problem_still_writes_the_deck_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "broken.md", "## q with no answer\n").unwrap();
        assert!(p.path.exists());
        assert!(p.parse_error.is_some());
    }

    #[test]
    fn a_name_collision_errors_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rust.md"), "original").unwrap();
        let err = place_deck(dir.path(), "rust", "## q\na\n").unwrap_err();
        assert!(format!("{err:#}").contains("already exists"), "{err:#}");
        assert_eq!(
            "original",
            std::fs::read_to_string(dir.path().join("rust.md")).unwrap()
        );
    }

    #[test]
    fn an_uploaded_name_cannot_traverse_out_of_the_dir() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "../../evil", "## q\na\n").unwrap();
        assert!(p.path.starts_with(dir.path()), "{}", p.path.display());
    }

    #[test]
    fn resetting_a_deck_clears_only_that_decks_progress() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.md"),
            "---\nformat-version: 1\nid: \"deck-da\"\n---\n## qa\nans-a\n<!-- id: card-qa -->\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.md"),
            "---\nformat-version: 1\nid: \"deck-db\"\n---\n## qb\nans-b\n<!-- id: card-qb -->\n",
        )
        .unwrap();
        let deck_a = Deck::load(dir.path().join("a.md")).unwrap();
        let deck_b = Deck::load(dir.path().join("b.md")).unwrap();

        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store
            .get_or_insert(&deck_a.cards[0].id().unwrap())
            .introduced_ms = Some(0);
        store
            .get_or_insert(&deck_b.cards[0].id().unwrap())
            .introduced_ms = Some(0);
        store.set_deck_mastered(deck_a.deck_token.as_deref().unwrap(), 0);

        let n = reset_decks(&mut store, [&deck_a]).unwrap();
        assert_eq!(1, n);
        assert!(
            store.get(&deck_a.cards[0].id().unwrap()).is_none(),
            "a's schedule wiped"
        );
        assert!(
            store.get(&deck_b.cards[0].id().unwrap()).is_some(),
            "b's schedule intact"
        );
        assert!(!store.deck_mastered(deck_a.deck_token.as_deref().unwrap()));
    }

    /// Writes a personal card into the sidecar beside `deck` and files its
    /// schedule, returning its id.
    fn personal_card(store: &mut Store, deck: &Path, deck_id: &str, back: &str) -> String {
        let slug: String = back.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let block = format!(
            "## front\n{back}\n<!-- id: card-v{} -->\n",
            slug.to_ascii_lowercase()
        );
        let id = crate::parser::parse_str(deck_id, &block).unwrap()[0]
            .id()
            .unwrap();
        crate::personal::append_cards(deck, deck_id, &block).unwrap();
        store.get_or_insert(&id).introduced_ms = Some(0);
        id
    }

    fn write_deck(dir: &Path, name: &str, deck_token: &str, card_token: &str) {
        std::fs::write(
            dir.join(name),
            format!(
                "---\nformat-version: 1\nid: \"deck-{deck_token}\"\n---\n## q\nans\n<!-- id: card-{card_token} -->\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn unparseable_regeneration_aborts_before_touching_the_old_file_and_writes_a_rej() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let orig = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("c1").introduced_ms = Some(0);
        store.save().unwrap();

        let err =
            replace_deck(dir.path(), "a", "## broken with no answer\n", &mut store).unwrap_err();

        assert!(format!("{err:#}").contains("does not parse"), "{err:#}");
        assert_eq!(
            orig,
            std::fs::read_to_string(dir.path().join("a.md")).unwrap()
        );
        assert!(!dir.path().join("a.md.bak").exists());
        let rej = std::fs::read_to_string(dir.path().join("a.rej")).unwrap();
        assert!(rej.contains("## broken with no answer"), "{rej}");
        assert!(store.get("c1").is_some());
    }

    #[test]
    fn the_replaced_deck_is_kept_as_a_bak_and_baks_are_not_decks() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let orig = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        let mut store = crate::state::open_store(&dir.path().join("a.md"), dir.path()).unwrap();

        replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut store).unwrap();

        assert_eq!(
            orig,
            std::fs::read_to_string(dir.path().join("a.md.bak")).unwrap()
        );
        let now = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        assert!(now.contains("new q"), "{now}");
        let decks = crate::workspace::deck_files(dir.path());
        assert_eq!(1, decks.len(), "{decks:?}");
        assert!(decks[0].ends_with("a.md"));
    }

    #[test]
    fn a_replace_backs_up_the_full_trio_before_wiping() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let orig_deck = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        let mut store = crate::state::open_store(&dir.path().join("a.md"), dir.path()).unwrap();
        store.get_or_insert("card-c1").introduced_ms = Some(0);
        store.save().unwrap();
        let progress = dir.path().join("progress/deck-da1.json");
        let orig_progress = std::fs::read(&progress).unwrap();
        let deck = Deck::load(dir.path().join("a.md")).unwrap();
        let mut cache = AugmentCache::open_for_decks(dir.path(), &[deck]).unwrap();
        cache.set_distractors("card-c1", vec!["x".into()], 1);
        cache.save().unwrap();
        let augment = WorkspaceFiles::new(dir.path()).augment_for("deck-da1");
        let orig_augment = std::fs::read(&augment).unwrap();

        replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut store).unwrap();

        assert_eq!(
            orig_deck,
            std::fs::read_to_string(dir.path().join("a.md.bak")).unwrap(),
            "the deck backup holds the pre-replace text"
        );
        assert_eq!(
            orig_progress,
            std::fs::read(dir.path().join("progress/deck-da1.json.bak")).unwrap(),
            "the progress backup holds the pre-wipe document"
        );
        assert_eq!(
            orig_augment,
            std::fs::read(dir.path().join("augment/deck-da1.json.bak")).unwrap(),
            "the augment backup holds the pre-wipe sidecar"
        );
        assert!(
            !progress.exists(),
            "the emptied live progress document is retired, not kept beside its backup"
        );
    }

    #[test]
    fn a_replace_that_fails_after_its_renames_restores_the_whole_trio() {
        // A workspace member, because only members freeze their source and a
        // missing `source:` is the portable post-rename failure injection.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let members = ws.join("decks");
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
        write_deck(&members, "a.md", "da1", "c1");
        let orig_deck = std::fs::read_to_string(members.join("a.md")).unwrap();
        let mut store = crate::state::open_store(&members.join("a.md"), dir.path()).unwrap();
        store.get_or_insert("card-c1").introduced_ms = Some(0);
        store.save().unwrap();
        let progress = store.path().to_path_buf();
        let orig_progress = std::fs::read(&progress).unwrap();
        let deck = Deck::load(members.join("a.md")).unwrap();
        let mut cache = AugmentCache::open_for_decks(&ws, &[deck]).unwrap();
        cache.set_distractors("card-c1", vec!["x".into()], 1);
        cache.save().unwrap();
        let augment = WorkspaceFiles::new(&ws).augment_for("deck-da1");
        let orig_augment = std::fs::read(&augment).unwrap();

        let err = replace_deck(
            &members,
            "a",
            "---\nsource: missing.md\n---\n## new q\nnew ans\n",
            &mut store,
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("missing"), "{err:#}");
        assert_eq!(
            orig_deck,
            std::fs::read_to_string(members.join("a.md")).unwrap(),
            "the live deck is restored"
        );
        assert_eq!(
            orig_progress,
            std::fs::read(&progress).unwrap(),
            "the live progress document is restored"
        );
        assert_eq!(
            orig_augment,
            std::fs::read(&augment).unwrap(),
            "the live augment sidecar is restored"
        );
        for leftover in [
            members.join("a.md.bak"),
            progress.with_extension("json.bak"),
            augment.with_extension("json.bak"),
        ] {
            assert!(
                !leftover.exists(),
                "no {} lingers after a restored failure",
                leftover.display()
            );
        }
    }

    fn trio_fixture(dir: &Path) -> (Vec<u8>, Vec<u8>, String) {
        write_deck(dir, "a.md", "da1", "c1");
        let mut store = crate::state::open_store(&dir.join("a.md"), dir).unwrap();
        store.get_or_insert("card-c1").introduced_ms = Some(0);
        store.save().unwrap();
        let deck = Deck::load(dir.join("a.md")).unwrap();
        let mut cache = AugmentCache::open_for_decks(dir, &[deck]).unwrap();
        cache.set_distractors("card-c1", vec!["x".into()], 1);
        cache.save().unwrap();
        let orig_deck = std::fs::read_to_string(dir.join("a.md")).unwrap();
        let orig_progress = std::fs::read(dir.join("progress/deck-da1.json")).unwrap();
        let orig_augment = std::fs::read(dir.join("augment/deck-da1.json")).unwrap();
        replace_deck(dir, "a", "## new q\nnew ans\n", &mut store).unwrap();
        (orig_progress, orig_augment, orig_deck)
    }

    #[test]
    fn restore_swaps_the_replacement_away_and_brings_history_and_augment_back() {
        let dir = tempfile::tempdir().unwrap();
        let (orig_progress, orig_augment, orig_deck) = trio_fixture(dir.path());
        let replaced_deck = std::fs::read_to_string(dir.path().join("a.md")).unwrap();

        let report = restore_deck(&dir.path().join("a.md"), dir.path()).unwrap();

        assert_eq!(
            RestoreReport {
                deck: true,
                progress: true,
                augment: true,
            },
            report
        );
        assert_eq!(
            orig_deck,
            std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
            "the original deck text is live again"
        );
        assert_eq!(
            orig_progress,
            std::fs::read(dir.path().join("progress/deck-da1.json")).unwrap(),
            "the review history is live again"
        );
        assert_eq!(
            orig_augment,
            std::fs::read(dir.path().join("augment/deck-da1.json")).unwrap(),
            "the augmentations are live again"
        );
        assert_eq!(
            replaced_deck,
            std::fs::read_to_string(dir.path().join("a.md.bak")).unwrap(),
            "the restored-away replacement is preserved as the new backup"
        );
    }

    #[test]
    fn restore_twice_is_a_byte_identical_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        trio_fixture(dir.path());
        let snapshot = |root: &Path| -> Vec<(String, Vec<u8>)> {
            let mut all = Vec::new();
            let mut stack = vec![root.to_path_buf()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).unwrap() {
                    let p = e.unwrap().path();
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        let rel = p.strip_prefix(root).unwrap().display().to_string();
                        all.push((rel, std::fs::read(&p).unwrap()));
                    }
                }
            }
            all.sort();
            all
        };
        let before = snapshot(dir.path());

        restore_deck(&dir.path().join("a.md"), dir.path()).unwrap();
        restore_deck(&dir.path().join("a.md"), dir.path()).unwrap();

        assert_eq!(
            before,
            snapshot(dir.path()),
            "a double restore must reproduce every file byte for byte"
        );
    }

    #[test]
    fn a_partial_trio_swaps_the_deck_and_reports_the_absent_members() {
        let dir = tempfile::tempdir().unwrap();
        let (_, _, orig_deck) = trio_fixture(dir.path());
        std::fs::remove_file(dir.path().join("progress/deck-da1.json.bak")).unwrap();
        std::fs::remove_file(dir.path().join("augment/deck-da1.json.bak")).unwrap();

        let report = restore_deck(&dir.path().join("a.md"), dir.path()).unwrap();

        assert_eq!(
            RestoreReport {
                deck: true,
                progress: false,
                augment: false,
            },
            report
        );
        assert_eq!(
            orig_deck,
            std::fs::read_to_string(dir.path().join("a.md")).unwrap()
        );
    }

    #[test]
    fn removing_a_deck_deletes_file_progress_augment_and_every_bak() {
        let dir = tempfile::tempdir().unwrap();
        trio_fixture(dir.path());
        // Post-replace state: live a.md plus the three .baks. Reopen a store
        // on the live deck so progress for its fresh token exists too.
        let mut store = crate::state::open_store(&dir.path().join("a.md"), dir.path()).unwrap();
        let live = Deck::load(dir.path().join("a.md")).unwrap();
        store
            .get_or_insert(&live.cards[0].id().unwrap())
            .introduced_ms = Some(0);
        store.save().unwrap();

        let report = remove_deck(&dir.path().join("a.md"), &store).unwrap();

        assert!(report.removed.len() >= 4, "{:?}", report.removed);
        assert!(!dir.path().join("a.md").exists(), "deck gone");
        assert!(!dir.path().join("a.md.bak").exists(), "deck backup gone");
        let leftovers: Vec<_> = ["progress", "augment"]
            .iter()
            .flat_map(|sub| {
                std::fs::read_dir(dir.path().join(sub))
                    .into_iter()
                    .flatten()
            })
            .map(|e| e.unwrap().path())
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy().to_string();
                n.contains("da1")
                    || n.ends_with(".bak")
                    || n.contains(live.deck_token.as_deref().unwrap())
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn removing_a_workspace_member_deletes_its_frozen_assets_directory() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let members = ws.join("decks");
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
        write_deck(&members, "a.md", "da1", "c1");
        let assets = ws.join("assets/deck-da1");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("img.png"), b"png").unwrap();
        let store = crate::state::open_store(&members.join("a.md"), dir.path()).unwrap();

        let preview = removal_preview(&members.join("a.md"), &store);
        assert!(
            preview.directories.contains(&assets),
            "the preview names the assets dir: {preview:?}"
        );

        remove_deck(&members.join("a.md"), &store).unwrap();

        assert!(!members.join("a.md").exists());
        assert!(!assets.exists(), "the frozen assets directory is gone");
    }

    /// The plan reaches `assets/` and `augment/` by name, and `is_dir` follows
    /// a link, so removal is one of the paths where a link out of the workspace
    /// would take somebody's real folder with it.
    #[cfg(unix)]
    #[test]
    fn removing_a_workspace_never_deletes_through_a_linked_directory() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("elsewhere");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep.png"), b"png").unwrap();
        let ws = dir.path().join("ws");
        let members = ws.join("decks");
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
        write_deck(&members, "a.md", "da1", "ca1");
        std::os::unix::fs::symlink(&outside, ws.join("assets")).unwrap();
        let store = crate::state::open_stores(&[members.join("a.md")], &ws).unwrap();

        remove_workspace(&ws, &store).unwrap();

        assert!(
            outside.join("keep.png").is_file(),
            "a link out of the workspace is not the workspace's to delete"
        );
        assert!(
            ws.join("assets").symlink_metadata().is_err(),
            "the link itself belongs to the workspace and goes with it"
        );
        assert!(!ws.exists(), "the workspace is still removed");
    }

    #[test]
    fn removing_a_workspace_deletes_every_owned_artifact_and_preserves_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let members = ws.join("decks");
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
        std::fs::write(ws.join("alix.local.toml"), "[review]\n").unwrap();
        std::fs::write(ws.join("source.txt"), "keep me\n").unwrap();
        std::fs::write(members.join("notes.md"), "ordinary markdown\n").unwrap();
        write_deck(&members, "a.md", "da1", "ca1");
        write_deck(&members, "b.md", "db1", "cb1");
        std::fs::write(members.join("a.md.bak"), "backup").unwrap();

        let paths = vec![members.join("a.md"), members.join("b.md")];
        let mut store = crate::state::open_stores(&paths, &ws).unwrap();
        store.get_or_insert("card-ca1").introduced_ms = Some(100);
        store.get_or_insert("card-cb1").introduced_ms = Some(200);
        store.save().unwrap();
        std::fs::write(ws.join("recent.json"), "{}\n").unwrap();
        for deck_id in ["deck-da1", "deck-db1"] {
            let augment = WorkspaceFiles::new(&ws).augment_for(deck_id);
            std::fs::create_dir_all(augment.parent().unwrap()).unwrap();
            std::fs::write(&augment, "{}\n").unwrap();
            let assets = WorkspaceFiles::new(&ws).assets_for(deck_id);
            std::fs::create_dir_all(&assets).unwrap();
            std::fs::write(assets.join("image.png"), b"png").unwrap();
        }
        std::fs::write(ws.join("assets/icon.svg"), "<svg/>").unwrap();

        let preview = workspace_removal_preview(&ws, &store).unwrap();
        assert_eq!(2, preview.decks);
        assert_eq!(2, preview.cards_with_progress);
        assert_eq!(Some(100), preview.earliest_review_ms);

        let report = remove_workspace(&ws, &store).unwrap();

        assert_eq!(2, report.decks_removed);
        assert!(!report.root_removed, "the unrelated files keep the folder");
        for removed in [
            ws.join("alix.toml"),
            ws.join("alix.local.toml"),
            ws.join("recent.json"),
            ws.join("assets"),
            ws.join("augment"),
            ws.join("progress"),
            members.join("a.md"),
            members.join("a.md.bak"),
            members.join("b.md"),
        ] {
            assert!(!removed.exists(), "{} must be removed", removed.display());
        }
        assert_eq!(
            "keep me\n",
            std::fs::read_to_string(ws.join("source.txt")).unwrap()
        );
        assert_eq!(
            "ordinary markdown\n",
            std::fs::read_to_string(members.join("notes.md")).unwrap()
        );
    }

    #[test]
    fn removing_a_workspace_does_not_recursively_delete_an_external_store() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let members = ws.join("decks");
        let user = dir.path().join("user");
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(
            ws.join("alix.toml"),
            format!("title = \"W\"\nstore = {:?}\n", user.display().to_string()),
        )
        .unwrap();
        write_deck(&members, "a.md", "da1", "ca1");
        let paths = vec![members.join("a.md")];
        let mut store = crate::state::open_stores(&paths, &user).unwrap();
        store.get_or_insert("card-ca1").introduced_ms = Some(100);
        store.save().unwrap();
        std::fs::write(user.join("progress/unrelated.json"), "keep\n").unwrap();
        std::fs::write(user.join("recent.json"), "keep\n").unwrap();

        let report = remove_workspace(&ws, &store).unwrap();

        assert!(report.root_removed, "an empty workspace root is removed");
        assert!(!ws.exists());
        assert!(!user.join("progress/deck-da1.json").exists());
        assert_eq!(
            "keep\n",
            std::fs::read_to_string(user.join("progress/unrelated.json")).unwrap()
        );
        assert_eq!(
            "keep\n",
            std::fs::read_to_string(user.join("recent.json")).unwrap()
        );
    }

    #[test]
    fn workspace_store_containment_accepts_its_boundary_and_rejects_every_outside_shape() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user");
        let progress = user.join("progress");
        let child = user.join("child");
        std::fs::create_dir_all(&progress).unwrap();
        std::fs::create_dir_all(&child).unwrap();

        assert_eq!(Some(user.clone()), user_root_for_store(&progress));
        assert_eq!(
            Some(user.clone()),
            user_root_for_store(&progress.join("deck-example.json"))
        );
        assert_eq!(
            None,
            user_root_for_store(&user.join("snapshots/deck-example.json"))
        );

        assert!(path_is_within(&user, &user), "the root owns its boundary");
        assert!(path_is_within(&child, &user), "an existing child is owned");
        assert!(
            path_is_within(&child.join(".."), &user),
            "canonical containment accepts a path that resolves to the boundary"
        );
        assert!(
            path_is_within(&user.join("not-created-yet"), &user),
            "a future direct child is owned without requiring it to exist"
        );
        assert!(
            !path_is_within(&dir.path().join("user-sibling"), &user),
            "a lexical prefix lookalike is outside"
        );
        assert!(
            !path_is_within(&user.join("missing/../escape"), &user),
            "a non-canonical path may not escape through a parent component"
        );
    }

    #[test]
    fn workspace_preview_lists_a_shared_dependent_once() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let members = ws.join("decks");
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
        write_deck(&members, "a.md", "da1", "ca1");
        write_deck(&members, "b.md", "db1", "cb1");
        std::fs::write(
            members.join("consumer.md"),
            "---\nformat-version: 1\nid: \"deck-consumer\"\nrequires: [\"a.md\", \"b.md\"]\n---\n## q\na\n<!-- id: card-consumer -->\n",
        )
        .unwrap();
        let paths = vec![
            members.join("a.md"),
            members.join("b.md"),
            members.join("consumer.md"),
        ];
        let store = crate::state::open_stores(&paths, &ws).unwrap();

        let preview = workspace_removal_preview(&ws, &store).unwrap();

        assert_eq!(vec!["consumer.md".to_string()], preview.dependents);
    }

    #[test]
    fn removing_a_non_directory_as_empty_is_a_loud_failure() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, "keep\n").unwrap();
        let mut removed = Vec::new();

        let failure = remove_if_empty(&file, &mut removed).unwrap_err();

        assert_eq!(file, failure.failed);
        assert!(failure.removed.is_empty());
        assert_eq!("keep\n", std::fs::read_to_string(&file).unwrap());
    }

    #[test]
    fn a_workspace_removal_failure_reports_completed_and_failed_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let members = ws.join("decks");
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
        for (name, deck_id, card_id) in [
            ("a.md", "da1", "ca1"),
            ("b.md", "db1", "cb1"),
            ("c.md", "dc1", "cc1"),
        ] {
            write_deck(&members, name, deck_id, card_id);
        }
        let paths = vec![
            members.join("a.md"),
            members.join("b.md"),
            members.join("c.md"),
        ];
        let mut store = crate::state::open_stores(&paths, &ws).unwrap();
        for card_id in ["card-ca1", "card-cb1", "card-cc1"] {
            store.get_or_insert(card_id).introduced_ms = Some(0);
        }
        store.save().unwrap();
        let failed = ws.join("progress/deck-db1.json");

        let error = with_removal_failure_at(&failed, || remove_workspace(&ws, &store)).unwrap_err();
        let failure = error.downcast_ref::<RemovalFailure>().unwrap();

        assert_eq!(&failed, &failure.failed);
        assert!(
            failure.removed.contains(&members.join("a.md")),
            "the completed list names the first removed deck: {failure:?}"
        );
        assert!(
            failure.removed.contains(&members.join("b.md")),
            "deck-first ordering names the second deck too: {failure:?}"
        );
        assert!(
            ws.join("alix.toml").exists(),
            "the workspace marker remains"
        );
        assert!(members.join("c.md").exists(), "later members remain intact");

        let remaining = crate::workspace::deck_files(&ws);
        let mut known_cards = std::collections::HashSet::new();
        let mut known_decks = std::collections::HashSet::new();
        for path in remaining {
            let deck = Deck::load(path).unwrap();
            known_cards.extend(deck.cards.iter().filter_map(|card| card.id()));
            known_decks.extend(deck.deck_token);
        }
        let aggregate = crate::state::open_aggregate_store(&ws).unwrap();
        let orphans = aggregate.orphans(&known_cards, &known_decks);
        assert!(
            orphans.cards.contains(&"card-cb1".to_string()),
            "the same predicate doctor consumes must report the failed member's valid progress: {orphans:?}"
        );
    }

    #[test]
    fn the_removal_preview_names_the_stakes_and_the_dependents() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        std::fs::write(
            dir.path().join("b.md"),
            "---\nformat-version: 1\nid: \"deck-db1\"\nrequires: [\"a.md\"]\n---\n## qb\nb\n<!-- id: card-cb1 -->\n",
        )
        .unwrap();
        let mut store = crate::state::open_store(&dir.path().join("a.md"), dir.path()).unwrap();
        store.get_or_insert("card-c1").introduced_ms = Some(4_200);
        store.save().unwrap();

        let preview = removal_preview(&dir.path().join("a.md"), &store);

        assert_eq!(1, preview.cards_with_progress);
        assert!(
            preview.files.contains(&dir.path().join("a.md")),
            "{preview:?}"
        );
        assert!(
            preview
                .files
                .iter()
                .any(|p| p.ends_with("progress/deck-da1.json")),
            "{preview:?}"
        );
        assert_eq!(vec!["b.md".to_string()], preview.dependents);
    }

    #[test]
    fn a_removal_failure_mid_set_is_loud_and_leaves_a_detectable_state() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let mut store = crate::state::open_store(&dir.path().join("a.md"), dir.path()).unwrap();
        store.get_or_insert("card-c1").introduced_ms = Some(0);
        store.save().unwrap();
        // Make the progress document undeletable as a file by replacing it
        // with a directory: deck-first ordering then fails on member two.
        let progress = dir.path().join("progress/deck-da1.json");
        std::fs::remove_file(&progress).unwrap();
        std::fs::create_dir_all(progress.join("x")).unwrap();

        let err = remove_deck(&dir.path().join("a.md"), &store).unwrap_err();
        let failure = err.downcast_ref::<RemovalFailure>().unwrap();

        assert!(
            format!("{err:#}").contains("deck-da1.json"),
            "the error names the member that failed: {err:#}"
        );
        assert_eq!(&progress, &failure.failed);
        assert_eq!(
            &[dir.path().join("a.md")],
            failure.removed.as_slice(),
            "the report names the completed removal"
        );
        assert!(
            !dir.path().join("a.md").exists(),
            "the deck-first ordering already removed the deck"
        );
        assert!(
            progress.exists(),
            "the failed member is still there for doctor to flag"
        );
    }

    #[test]
    fn restore_without_any_backup_is_a_clean_error_naming_the_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");

        let err = restore_deck(&dir.path().join("a.md"), dir.path()).unwrap_err();

        assert!(
            format!("{err:#}").contains("a.md.bak"),
            "the error names what was looked for: {err:#}"
        );
        assert!(dir.path().join("a.md").exists(), "nothing was touched");
    }

    #[test]
    fn replacing_a_deck_wipes_its_progress_and_augment_entries() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        write_deck(dir.path(), "b.md", "db1", "cb1");
        let paths = [dir.path().join("a.md"), dir.path().join("b.md")];
        let decks = paths
            .iter()
            .map(Deck::load)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut store = crate::state::open_stores(&paths, dir.path()).unwrap();

        // Deck A: a card schedule, deck-family mastery, records.
        store.get_or_insert("card-c1").introduced_ms = Some(0);
        store.set_deck_mastered("deck-da1", 1);
        // Deck B (shares the store): its own schedule + mastery.
        store.get_or_insert("card-cb1").introduced_ms = Some(0);
        store.set_deck_mastered("deck-db1", 1);
        store.save().unwrap();

        let cache_path = WorkspaceFiles::new(dir.path()).augment();
        let mut cache = AugmentCache::open_for_decks(dir.path(), &decks).unwrap();
        cache.set_distractors("card-c1", vec!["x".into()], 1);
        cache.add_topology(crate::augment::Topology {
            name: "auto".into(),
            deck_token: "deck-da1".into(),
            ..Default::default()
        });
        cache.set_distractors("card-cb1", vec!["y".into()], 1);
        cache.add_topology(crate::augment::Topology {
            name: "auto".into(),
            deck_token: "deck-db1".into(),
            ..Default::default()
        });
        cache.save().unwrap();

        let report = replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut store).unwrap();

        assert_eq!(1, report.wiped_cards);
        assert!(store.get("card-c1").is_none());
        assert!(!store.deck_mastered("deck-da1"));
        assert!(store.get("card-cb1").is_some());
        assert!(store.deck_mastered("deck-db1"));

        let cache = AugmentCache::open(&cache_path);
        assert!(cache.distractors("card-c1", 1).is_none());
        assert!(!cache.has_topology_for(&once("deck-da1")));
        assert!(cache.distractors("card-cb1", 1).is_some());
        assert!(cache.has_topology_for(&once("deck-db1")));
    }

    #[test]
    fn a_replaced_deck_leaves_no_orphaned_store_keys() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let mut store = crate::state::open_store(&dir.path().join("a.md"), dir.path()).unwrap();
        store.get_or_insert("card-c1").introduced_ms = Some(0);
        store.set_deck_mastered("deck-da1", 1);
        store.save().unwrap();

        replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut store).unwrap();

        let deck = Deck::load(dir.path().join("a.md")).unwrap();
        let known_ids: HashSet<String> = deck.cards.iter().filter_map(|c| c.id()).collect();
        let known_deck_id = deck.deck_token.clone().unwrap();
        let orphans = store.orphans(&known_ids, &once(&known_deck_id));
        assert!(orphans.is_empty(), "{orphans:?}");
    }

    #[test]
    fn every_replacement_mints_fresh_tokens() {
        let dir = tempfile::tempdir().unwrap();
        place_deck(dir.path(), "a", "## old q\nold ans\n## old r\nold b\n").unwrap();
        let old_text = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        let old = crate::parser::parse("a.md", &old_text).unwrap();
        let old_tokens: Vec<String> = old
            .cards
            .iter()
            .filter_map(|c| c.token.as_deref().map(str::to_string))
            .chain(old.deck_token.clone())
            .collect();
        assert!(old_tokens.len() >= 3, "{old_tokens:?}");

        let mut store = crate::state::open_store(&dir.path().join("a.md"), dir.path()).unwrap();
        replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut store).unwrap();

        let now = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        for tok in &old_tokens {
            assert!(!now.contains(tok.as_str()), "old token {tok} reappeared");
        }
        let bak = std::fs::read_to_string(dir.path().join("a.md.bak")).unwrap();
        assert!(old_tokens.iter().all(|t| bak.contains(t.as_str())));
    }

    #[test]
    fn replacing_one_per_deck_document_retires_its_old_identity_without_touching_its_neighbor() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        write_deck(dir.path(), "b.md", "db1", "c2");
        let paths = [dir.path().join("a.md"), dir.path().join("b.md")];
        let mut aggregate = crate::state::open_stores(&paths, dir.path()).unwrap();
        aggregate.get_or_insert("card-c1").introduced_ms = Some(0);
        aggregate.get_or_insert("card-c2").introduced_ms = Some(0);
        aggregate.save().unwrap();
        let mut augmentation = AugmentCache::open_for_decks(
            dir.path(),
            &paths
                .iter()
                .map(Deck::load)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        )
        .unwrap();
        augmentation.set_note("card-c1", "first".to_string(), 1);
        augmentation.set_note("card-c2", "second".to_string(), 1);
        augmentation.save().unwrap();

        replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut aggregate).unwrap();

        assert!(!dir.path().join("progress/deck-da1.json").exists());
        assert!(!dir.path().join("augment/deck-da1.json").exists());
        assert!(dir.path().join("progress/deck-db1.json").exists());
        assert!(dir.path().join("augment/deck-db1.json").exists());
        let untouched = Store::open_deck(
            dir.path().join("progress/deck-db1.json"),
            "deck-db1",
            "b.md",
        )
        .unwrap();
        assert!(untouched.get("card-c2").is_some());
        let untouched_augmentation =
            AugmentCache::open_deck(dir.path().join("augment/deck-db1.json"), "deck-db1").unwrap();
        assert_eq!(Some("second"), untouched_augmentation.note("card-c2", 1));
    }

    #[test]
    fn a_second_replace_overwrites_the_prior_bak() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let mut store = crate::state::open_store(&dir.path().join("a.md"), dir.path()).unwrap();

        replace_deck(dir.path(), "a", "## first q\nfirst ans\n", &mut store).unwrap();
        let first = std::fs::read_to_string(dir.path().join("a.md")).unwrap();

        replace_deck(dir.path(), "a", "## second q\nsecond ans\n", &mut store).unwrap();

        let baks: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
            .collect();
        assert_eq!(1, baks.len(), "{baks:?}");
        assert_eq!(
            first,
            std::fs::read_to_string(dir.path().join("a.md.bak")).unwrap()
        );
    }

    /// One fixture: frontmatter without `id:`, a divided card (fence + note +
    /// escaped divider + trailing-space front), and a two-hole cloze card.
    const MARKER_FIXTURE: &str = "---\nsource: notes.md\nrequires: basics\n---\n# The Title\nintro prose\n\n## First question \nextra front line\n\n---\nthe answer\n\\--- escaped divider\n> a note\n```\nfenced\n## not a card\n```\ntail prose\n\n## Fill in the blanks\nthe alpha and beta here\n> cloze note\n<!-- blank: span hidden=\"alpha\" b:a1b2c3 -->\n<!-- blank: span hidden=\"beta\" b:d4e5f6 -->\n";

    fn all_tokens(subject: &str, text: &str) -> Vec<String> {
        let deck = crate::parser::parse(subject, text).unwrap();
        let mut toks = Vec::new();
        let mut last_line = None;
        for card in &deck.cards {
            if last_line != Some(card.line) {
                if let Some(t) = card.token.as_deref() {
                    toks.push(t.to_string());
                }
                last_line = Some(card.line);
            }
        }
        toks.extend(deck.deck_token);
        toks
    }

    fn assert_no_duplicate_tokens(subject: &str, text: &str) {
        let toks = all_tokens(subject, text);
        let uniq: HashSet<&String> = toks.iter().collect();
        assert_eq!(
            toks.len(),
            uniq.len(),
            "duplicate token in {subject}: {toks:?}"
        );
    }

    #[test]
    fn every_writer_preserves_tokens_and_text_and_never_duplicates() {
        {
            let dir = tempfile::tempdir().unwrap();
            place_deck(dir.path(), "d", MARKER_FIXTURE).unwrap();
            let text = std::fs::read_to_string(dir.path().join("d.md")).unwrap();
            let deck = crate::parser::parse("d.md", &text).unwrap();
            assert!(deck.cards.iter().all(|c| c.token.is_some()), "all stamped");
            assert!(text.contains("First question"), "front text kept");
            assert_eq!(
                2,
                deck.cards.iter().filter(|c| c.is_blank_card()).count(),
                "both span cards survive the writer"
            );
            assert!(
                text.contains("b:a1b2c3") && text.contains("b:d4e5f6"),
                "authored region stamps preserved: {text:?}"
            );
            assert_no_duplicate_tokens("d.md", &text);
        }
        {
            let dir = tempfile::tempdir().unwrap();
            place_deck(dir.path(), "d", "## x\ny\n").unwrap();
            let mut store = crate::state::open_store(&dir.path().join("d.md"), dir.path()).unwrap();
            replace_deck(dir.path(), "d", MARKER_FIXTURE, &mut store).unwrap();
            let text = std::fs::read_to_string(dir.path().join("d.md")).unwrap();
            let deck = crate::parser::parse("d.md", &text).unwrap();
            assert!(deck.cards.iter().all(|c| c.token.is_some()), "all stamped");
            assert!(text.contains("First question"));
            assert_no_duplicate_tokens("d.md", &text);
        }
        {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("d.md");
            std::fs::write(&path, MARKER_FIXTURE).unwrap();
            crate::stamp::stamp_deck(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let deck = crate::parser::parse("d.md", &text).unwrap();
            assert!(deck.cards.iter().all(|c| c.token.is_some()));
            assert!(text.contains("First question"));
            assert_no_duplicate_tokens("d.md", &text);
        }
        {
            let dir = tempfile::tempdir().unwrap();
            let placed = place_deck(dir.path(), "d", "## base\nb\n").unwrap();
            let added = "card-aaaaaaaaaaaaaaaaaaaaaaaaap";
            crate::deck::append_cards(
                &placed.path,
                &format!("## added\nans\n<!-- id: {added} -->\n"),
            )
            .unwrap();
            let text = std::fs::read_to_string(&placed.path).unwrap();
            assert!(text.contains(added), "appended token preserved");
            assert_no_duplicate_tokens("d.md", &text);
        }
    }

    fn once(s: &str) -> HashSet<String> {
        std::iter::once(s.to_string()).collect()
    }

    #[test]
    fn a_trace_rebuild_routes_through_replace_and_wipes_the_old_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let existing = "---\nformat-version: 1\nid: \"deck-da1\"\ntrace: how x becomes y\nsource: notes.md\n---\n## old cp\nold\n<!-- id: card-c1 -->\n";
        let path = dir.path().join("t.md");
        std::fs::write(&path, existing).unwrap();
        let mut store = crate::state::open_store(&path, dir.path()).unwrap();
        store.get_or_insert("card-c1").introduced_ms = Some(0);
        store.save().unwrap();

        let new_text = crate::deck::trace_checkpoint_text(
            &dir.path().join("t.md"),
            existing,
            "## new cp\nnew\n",
        )
        .unwrap();
        assert!(new_text.contains("trace: how x becomes y"));
        assert!(new_text.contains("source: notes.md"));

        replace_deck(dir.path(), "t", &new_text, &mut store).unwrap();

        assert!(store.get("card-c1").is_none());
        let now = std::fs::read_to_string(dir.path().join("t.md")).unwrap();
        assert!(now.contains("new cp"));
        let rebuilt = crate::parser::parse("t.md", &now).unwrap();
        assert_eq!(1, rebuilt.cards.len());
        assert!(
            rebuilt.cards[0].token.is_some(),
            "the rebuilt checkpoint is stamped"
        );
        assert_ne!(
            Some("card-c1"),
            rebuilt.cards[0].token.as_deref(),
            "old token must not survive as the rebuilt card id"
        );
    }

    #[test]
    fn resetting_a_deck_clears_its_personal_schedules_and_keeps_the_file_and_anothers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.md"),
            "---\nformat-version: 1\nid: \"deck-da\"\n---\n## qa\nans-a\n<!-- id: card-qa -->\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.md"),
            "---\nformat-version: 1\nid: \"deck-db\"\n---\n## qb\nans-b\n<!-- id: card-qb -->\n",
        )
        .unwrap();
        let deck_a = Deck::load(dir.path().join("a.md")).unwrap();

        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let path_a = dir.path().join("a.md");
        let path_b = dir.path().join("b.md");
        let id_a = personal_card(&mut store, &path_a, "deck-da", "vc-a");
        let id_other = personal_card(&mut store, &path_b, "deck-db", "vc-other");

        let n = reset_decks(&mut store, [&deck_a]).unwrap();
        assert_eq!(1, n, "only a's personal card had progress");
        assert!(store.get(&id_a).is_none(), "a's personal schedule dropped");
        assert!(
            crate::personal::sidecar_path(&path_a).exists(),
            "the personal file is the user's: a reset never deletes it"
        );
        assert!(
            store.get(&id_other).is_some(),
            "another deck's personal schedule survives"
        );
    }
}
