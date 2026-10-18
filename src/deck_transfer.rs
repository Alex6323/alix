use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    cite::{self, DeckPlace, ExamPlace},
    deck::{self, Deck},
    share,
    state::UserFiles,
    workspace::{self, WorkspaceFiles},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferMode {
    Copy,
    Move,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TransferReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub assets: usize,
    pub augmentation: bool,
    pub progress: bool,
    pub leftovers: Vec<PathBuf>,
}

struct Transfer {
    source: PathBuf,
    destination_deck: PathBuf,
    deck: Deck,
    deck_id: String,
    source_files: WorkspaceFiles,
    destination_files: WorkspaceFiles,
    source_progress: PathBuf,
    destination_progress: PathBuf,
    assets: usize,
    augmentation: bool,
    progress: bool,
    baseline: SourceBaseline,
}

struct SourceBaseline {
    deck: Vec<u8>,
    augmentation: Option<Vec<u8>>,
    progress: Option<Vec<u8>>,
    assets: BTreeMap<PathBuf, Vec<u8>>,
}

pub fn transfer(
    source: &Path,
    destination_workspace: &Path,
    mode: TransferMode,
) -> Result<TransferReport> {
    transfer_with_remove(source, destination_workspace, mode, |path| {
        std::fs::remove_file(path)
    })
}

fn transfer_with_remove(
    source: &Path,
    destination_workspace: &Path,
    mode: TransferMode,
    remove_source_deck: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<TransferReport> {
    let transfer = Transfer::prepare(source, destination_workspace, mode)?;
    let scratch = tempfile::tempdir().context("cannot create deck transfer staging directory")?;
    let (bundle, _) = share::stage_deck_bundle(&transfer.source, scratch.path())?;
    transfer.verify_staged_identity(&bundle)?;

    if let Err(error) = share::land_deck_bundle(&bundle, &transfer.destination_files.decks()) {
        let leftovers = transfer.rollback_destination(false);
        return Err(with_rollback(error, &leftovers));
    }

    let copied_progress = mode == TransferMode::Move
        && transfer.progress
        && transfer.source_progress != transfer.destination_progress;
    if copied_progress && let Err(error) = transfer.write_progress() {
        let leftovers = transfer.rollback_destination(true);
        return Err(with_rollback(error, &leftovers));
    }

    if mode == TransferMode::Copy {
        return Ok(transfer.report(false, Vec::new()));
    }

    if let Err(error) = transfer.verify_source_unchanged() {
        let leftovers = transfer.rollback_destination(copied_progress);
        return Err(with_rollback(error, &leftovers));
    }
    if let Err(error) = remove_source_deck(&transfer.source) {
        let leftovers = transfer.rollback_destination(copied_progress);
        return Err(with_rollback(
            anyhow::Error::new(error)
                .context(format!("cannot remove {}", transfer.source.display())),
            &leftovers,
        ));
    }

    let leftovers = transfer.remove_source_sidecars(copied_progress);
    Ok(transfer.report(copied_progress, leftovers))
}

impl Transfer {
    fn prepare(source: &Path, destination_workspace: &Path, mode: TransferMode) -> Result<Self> {
        let source = source
            .canonicalize()
            .with_context(|| format!("cannot resolve {}", source.display()))?;
        let source_root = workspace::root_for_deck(&source)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} is not a direct member of an Alix workspace",
                    source.display()
                )
            })?
            .to_path_buf();
        let destination_root = destination_workspace.canonicalize().with_context(|| {
            format!(
                "cannot resolve destination workspace {}",
                destination_workspace.display()
            )
        })?;
        if !workspace::is_workspace(&destination_root) {
            bail!(
                "{} is not an Alix workspace",
                destination_workspace.display()
            );
        }
        if source_root == destination_root {
            bail!("source and destination are the same workspace");
        }

        let source_workspace = workspace::Workspace::load(&source_root)
            .with_context(|| format!("cannot load {}", source_root.display()))?;
        let deck = Deck::load_with_defaults(&source, &source_workspace.settings)?;
        let deck_id = deck
            .deck_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("{} is not initialized", source.display()))?;
        let source_files = WorkspaceFiles::new(&source_root);
        let destination_files = WorkspaceFiles::new(&destination_root);
        let file_name = source
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("{} has no file name", source.display()))?;
        let destination_deck = destination_files.decks().join(file_name);
        let source_user = UserFiles::new(resolved_user_root(&source_root));
        let destination_user = UserFiles::new(resolved_user_root(&destination_root));
        let source_progress = source_user.progress_for(&deck_id);
        let destination_progress = destination_user.progress_for(&deck_id);

        ensure_destination_available(
            &destination_files,
            &destination_deck,
            &destination_progress,
            &source_progress,
            &deck_id,
        )?;
        ensure_requirements_resolve(&deck, &destination_files.decks())?;
        ensure_citations_and_exam_resolve_alike(&deck, &source_root, &destination_root)?;
        if mode == TransferMode::Move {
            let dependents = deck::dependents(&source);
            if !dependents.is_empty() {
                bail!(
                    "cannot move {} because it is required by {}",
                    source.display(),
                    dependents.join(", ")
                );
            }
        }

        let source_augmentation = source_files.augment_for(&deck_id);
        if source_augmentation.is_file() {
            crate::augment::read_deck_data(&source_augmentation, &deck_id)?;
        }
        if source_progress.is_file() {
            crate::store::Store::open_deck(&source_progress, &deck_id, deck.subject.clone())?;
        }
        let source_assets = source_files.assets_for(&deck_id);
        let baseline = SourceBaseline {
            deck: std::fs::read(&source)
                .with_context(|| format!("cannot read {}", source.display()))?,
            augmentation: read_optional(&source_augmentation)?,
            progress: read_optional(&source_progress)?,
            assets: read_tree(&source_assets)?,
        };
        let assets = baseline.assets.len();
        let augmentation = baseline.augmentation.is_some();
        let progress = baseline.progress.is_some();

        Ok(Self {
            source,
            destination_deck,
            deck,
            deck_id,
            source_files,
            destination_files,
            source_progress,
            destination_progress,
            assets,
            augmentation,
            progress,
            baseline,
        })
    }

    fn verify_staged_identity(&self, bundle: &Path) -> Result<()> {
        let staged = Deck::load(self.staged_deck(bundle)?)?;
        if staged.deck_token != self.deck.deck_token {
            bail!("staging changed the deck ID");
        }
        let original_ids = self
            .deck
            .cards
            .iter()
            .map(|card| card.id())
            .collect::<Vec<_>>();
        let staged_ids = staged
            .cards
            .iter()
            .map(|card| card.id())
            .collect::<Vec<_>>();
        if staged_ids != original_ids {
            bail!("staging changed one or more card IDs");
        }
        Ok(())
    }

    fn staged_deck(&self, bundle: &Path) -> Result<PathBuf> {
        let name = self
            .source
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("{} has no file name", self.source.display()))?;
        Ok(bundle.join(name))
    }

    fn write_progress(&self) -> Result<()> {
        let bytes = self
            .baseline
            .progress
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("source progress disappeared before transfer"))?;
        let parent = self.destination_progress.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "{} has no parent directory",
                self.destination_progress.display()
            )
        })?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
        let tmp = self.destination_progress.with_extension("json.tmp");
        crate::fsio::replace_file(&tmp, &self.destination_progress, bytes)
            .with_context(|| format!("cannot write {}", self.destination_progress.display()))
    }

    fn verify_source_unchanged(&self) -> Result<()> {
        if std::fs::read(&self.source).ok().as_deref() != Some(self.baseline.deck.as_slice()) {
            bail!("{} changed during the move", self.source.display());
        }
        ensure_optional_unchanged(
            &self.source_files.augment_for(&self.deck_id),
            self.baseline.augmentation.as_deref(),
        )?;
        ensure_optional_unchanged(&self.source_progress, self.baseline.progress.as_deref())?;
        let assets = read_tree(&self.source_files.assets_for(&self.deck_id))?;
        if assets != self.baseline.assets {
            bail!(
                "{} changed during the move",
                self.source_files.assets_for(&self.deck_id).display()
            );
        }
        Ok(())
    }

    fn remove_source_sidecars(&self, moved_progress: bool) -> Vec<PathBuf> {
        let mut leftovers = Vec::new();
        remove_dir_if_present(&self.source_files.assets_for(&self.deck_id), &mut leftovers);
        remove_file_if_present(
            &self.source_files.augment_for(&self.deck_id),
            &mut leftovers,
        );
        if moved_progress {
            remove_file_if_present(&self.source_progress, &mut leftovers);
        }
        leftovers
    }

    fn rollback_destination(&self, remove_progress: bool) -> Vec<PathBuf> {
        let mut leftovers = Vec::new();
        remove_file_if_present(&self.destination_deck, &mut leftovers);
        remove_dir_if_present(
            &self.destination_files.assets_for(&self.deck_id),
            &mut leftovers,
        );
        remove_file_if_present(
            &self.destination_files.augment_for(&self.deck_id),
            &mut leftovers,
        );
        if remove_progress {
            remove_file_if_present(&self.destination_progress, &mut leftovers);
        }
        leftovers
    }

    fn report(&self, moved_progress: bool, leftovers: Vec<PathBuf>) -> TransferReport {
        TransferReport {
            source: self.source.clone(),
            destination: self.destination_deck.clone(),
            assets: self.assets,
            augmentation: self.augmentation,
            progress: moved_progress,
            leftovers,
        }
    }
}

fn ensure_destination_available(
    destination: &WorkspaceFiles,
    destination_deck: &Path,
    destination_progress: &Path,
    source_progress: &Path,
    deck_id: &str,
) -> Result<()> {
    for path in [
        destination_deck.to_path_buf(),
        destination.assets_for(deck_id),
        destination.augment_for(deck_id),
    ] {
        if path.exists() {
            bail!("{} already exists", path.display());
        }
    }
    if destination_progress != source_progress && destination_progress.exists() {
        bail!("{} already exists", destination_progress.display());
    }
    for member in workspace::deck_files(destination.root()) {
        let candidate = Deck::load(&member)?;
        if candidate.deck_token.as_deref() == Some(deck_id) {
            bail!("{} already uses deck ID `{deck_id}`", member.display());
        }
    }
    Ok(())
}

fn ensure_requirements_resolve(deck: &Deck, destination_decks: &Path) -> Result<()> {
    let missing = deck
        .requires
        .iter()
        .filter(|required| {
            deck::resolve_dep(required, Some(destination_decks), Some(destination_decks)).is_none()
        })
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "destination is missing required deck(s): {}",
            missing.join(", ")
        );
    }
    Ok(())
}

fn ensure_citations_and_exam_resolve_alike(
    deck: &Deck,
    source_root: &Path,
    destination_root: &Path,
) -> Result<()> {
    let source_cite = workspace::manifest_cite(source_root);
    let destination_cite = workspace::manifest_cite(destination_root);
    let used = deck
        .cards
        .iter()
        .flat_map(|card| &card.citations)
        .map(|citation| citation.locator.key.as_str());
    let cite_mismatches = cite::transfer_mismatches(
        &deck.cite,
        used,
        DeckPlace::Member {
            workspace: source_root,
            manifest_cite: &source_cite,
        },
        DeckPlace::Member {
            workspace: destination_root,
            manifest_cite: &destination_cite,
        },
    );
    let source_exam = workspace::manifest_exam(source_root);
    let destination_exam = workspace::manifest_exam(destination_root);
    let exam_mismatches = cite::exam_transfer_mismatches(
        &deck.exam,
        ExamPlace {
            base: source_root,
            manifest_exam: &source_exam,
        },
        ExamPlace {
            base: destination_root,
            manifest_exam: &destination_exam,
        },
    );
    let mismatches: Vec<String> = cite_mismatches
        .iter()
        .map(ToString::to_string)
        .chain(exam_mismatches.iter().map(ToString::to_string))
        .collect();
    if !mismatches.is_empty() {
        bail!(
            "cannot transfer {}: {}",
            deck.path.display(),
            mismatches.join("; ")
        );
    }
    Ok(())
}

fn resolved_user_root(workspace_root: &Path) -> PathBuf {
    let root = workspace::store_path(workspace_root);
    root.canonicalize().unwrap_or(root)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    path.is_file()
        .then(|| std::fs::read(path).with_context(|| format!("cannot read {}", path.display())))
        .transpose()
}

fn ensure_optional_unchanged(path: &Path, expected: Option<&[u8]>) -> Result<()> {
    if read_optional(path)?.as_deref() != expected {
        bail!("{} changed during the move", path.display());
    }
    Ok(())
}

fn read_tree(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut files = BTreeMap::new();
    if !root.exists() {
        return Ok(files);
    }
    read_tree_into(root, root, &mut files)?;
    Ok(files)
}

fn read_tree_into(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<()> {
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("cannot read {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            read_tree_into(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| anyhow::anyhow!("{} is outside {}", path.display(), root.display()))?;
            files.insert(
                relative.to_path_buf(),
                std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?,
            );
        }
    }
    Ok(())
}

fn remove_file_if_present(path: &Path, leftovers: &mut Vec<PathBuf>) {
    if path.exists() && std::fs::remove_file(path).is_err() {
        leftovers.push(path.to_path_buf());
    }
}

fn remove_dir_if_present(path: &Path, leftovers: &mut Vec<PathBuf>) {
    if path.exists() && std::fs::remove_dir_all(path).is_err() {
        leftovers.push(path.to_path_buf());
    }
}

fn with_rollback(error: anyhow::Error, leftovers: &[PathBuf]) -> anyhow::Error {
    if leftovers.is_empty() {
        error
    } else {
        error.context(format!(
            "destination rollback left: {}",
            leftovers
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(root: &Path, defaults: &str) {
        std::fs::create_dir_all(root.join("decks")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("alix.toml"), defaults).unwrap();
    }

    fn deck(root: &Path, name: &str, frontmatter: &str) -> PathBuf {
        let path = root.join("decks").join(name);
        std::fs::write(
            &path,
            format!(
                "---\nformat-version: 1\nid: deck-deck1\n{frontmatter}---\n## q\nanswer\n<!-- id: card-card1 -->\n"
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn copy_uses_the_public_bundle_without_copying_progress() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        workspace(&source, "");
        workspace(&destination, "");
        let asset_name = crate::assets::object_name(b"evidence\n", "txt");
        std::fs::create_dir_all(source.join("assets/deck-deck1")).unwrap();
        std::fs::write(
            source.join("assets/deck-deck1").join(&asset_name),
            "evidence\n",
        )
        .unwrap();
        let deck = deck(&source, "facts.md", "");
        let loaded = Deck::load(&deck).unwrap();
        let mut augmentation = crate::augment::AugmentCache::open_for_deck(&loaded).unwrap();
        augmentation.set_note("card-card1", "note".to_string(), 1);
        augmentation.save().unwrap();
        let mut progress = crate::state::open_store(&deck, &source).unwrap();
        progress.get_or_insert("card-card1");
        progress.save().unwrap();

        let report = transfer(&deck, &destination, TransferMode::Copy).unwrap();

        assert_eq!(1, report.assets);
        assert!(report.augmentation);
        assert!(!report.progress);
        assert!(deck.is_file());
        let copied = destination.join("decks/facts.md");
        assert_eq!(
            std::fs::read_to_string(&deck).unwrap(),
            std::fs::read_to_string(&copied).unwrap()
        );
        assert!(
            destination
                .join("assets/deck-deck1")
                .join(&asset_name)
                .is_file()
        );
        assert!(destination.join("augment/deck-deck1.json").is_file());
        assert!(!destination.join(".alix/progress/deck-deck1.json").exists());
    }

    #[test]
    fn copying_a_source_free_deck_normalizes_the_destination_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        workspace(&source, "");
        workspace(&destination, "");
        let deck = deck(&source, "facts.md", "");
        let dirty = format!(
            "\u{feff}{}",
            std::fs::read_to_string(&deck)
                .unwrap()
                .replace('\n', "\r\n")
                .replace("## q\r\n", "## q \t\r\n")
        );
        std::fs::write(&deck, dirty).unwrap();

        transfer(&deck, &destination, TransferMode::Copy).unwrap();

        let copied = std::fs::read_to_string(destination.join("decks/facts.md")).unwrap();
        assert_eq!(
            crate::parser::normalize(&copied),
            copied,
            "deck transfer is a destination deck-write boundary"
        );
    }

    #[test]
    fn move_relocates_progress_and_removes_the_source_graph() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        workspace(&source, "");
        workspace(&destination, "");
        let deck = deck(&source, "facts.md", "");
        let mut progress = crate::state::open_store(&deck, &source).unwrap();
        progress.get_or_insert("card-card1");
        progress.save().unwrap();
        let progress_bytes = std::fs::read(source.join(".alix/progress/deck-deck1.json")).unwrap();

        let report = transfer(&deck, &destination, TransferMode::Move).unwrap();

        assert!(report.progress);
        assert!(report.leftovers.is_empty());
        assert!(!deck.exists());
        assert!(destination.join("decks/facts.md").is_file());
        assert_eq!(
            progress_bytes,
            std::fs::read(destination.join(".alix/progress/deck-deck1.json")).unwrap()
        );
        assert!(!source.join(".alix/progress/deck-deck1.json").exists());
    }

    #[test]
    fn target_requirements_and_source_dependents_fail_before_writes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        workspace(&source, "");
        workspace(&destination, "");
        let deck = deck(&source, "facts.md", "requires:\n  - foundations\n");

        let error = transfer(&deck, &destination, TransferMode::Copy).unwrap_err();

        assert!(format!("{error:#}").contains("missing required deck"));
        assert!(!destination.join("decks/facts.md").exists());

        std::fs::write(
            source.join("decks/dependent.md"),
            "---\nformat-version: 1\nid: deck-deck2\nrequires:\n  - facts\n---\n## q\nanswer\n<!-- id: card-card2 -->\n",
        )
        .unwrap();
        let destination_requirement = destination.join("decks/foundations.md");
        std::fs::write(
            &destination_requirement,
            "---\nformat-version: 1\nid: deck-base\n---\n## q\nanswer\n<!-- id: card-basecard -->\n",
        )
        .unwrap();

        let error = transfer(&deck, &destination, TransferMode::Move).unwrap_err();

        assert!(format!("{error:#}").contains("required by dependent.md"));
        assert!(deck.is_file());
        assert!(!destination.join("decks/facts.md").exists());
    }

    #[test]
    fn a_cited_key_whose_root_differs_in_the_destination_refuses_before_writes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        let shared = dir.path().join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        let shared_root = format!(
            "shared = {:?}\n",
            shared.canonicalize().unwrap().display().to_string()
        );
        workspace(
            &source,
            &format!("[cite]\ncode = \"crates\"\n{shared_root}"),
        );
        workspace(
            &destination,
            &format!("[cite]\ncode = \"crates\"\n{shared_root}"),
        );
        let path = source.join("decks/facts.md");
        let cited = |key: &str| {
            format!(
                "---\nformat-version: 1\nid: deck-deck1\n---\n## q\nanswer\n<!-- at: {key}:x.txt:1 -->\n<!-- id: card-card1 -->\n"
            )
        };

        std::fs::write(&path, cited("code")).unwrap();
        for mode in [TransferMode::Copy, TransferMode::Move] {
            let error = format!("{:#}", transfer(&path, &destination, mode).unwrap_err());
            assert!(
                error.contains("`code`")
                    && error.contains(
                        &source
                            .canonicalize()
                            .unwrap()
                            .join("crates")
                            .display()
                            .to_string()
                    )
                    && error.contains(
                        &destination
                            .canonicalize()
                            .unwrap()
                            .join("crates")
                            .display()
                            .to_string()
                    ),
                "{mode:?}: the refusal names the key and both roots: {error}"
            );
            assert!(path.is_file(), "{mode:?}");
            assert!(!destination.join("decks/facts.md").exists(), "{mode:?}");
        }

        std::fs::write(&path, cited("shared")).unwrap();
        transfer(&path, &destination, TransferMode::Copy).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            std::fs::read_to_string(destination.join("decks/facts.md")).unwrap(),
            "an allowed transfer never rewrites the deck's citations"
        );
        assert_eq!(
            format!("[cite]\ncode = \"crates\"\n{shared_root}"),
            std::fs::read_to_string(destination.join("alix.toml")).unwrap()
        );
    }

    #[test]
    fn destination_identity_collisions_fail_before_writes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        workspace(&source, "");
        workspace(&destination, "");
        let deck = deck(&source, "facts.md", "");
        std::fs::write(
            destination.join("decks/other.md"),
            "---\nformat-version: 1\nid: deck-deck1\n---\n## q\nanswer\n<!-- id: card-other -->\n",
        )
        .unwrap();

        let error = transfer(&deck, &destination, TransferMode::Copy).unwrap_err();

        assert!(format!("{error:#}").contains("already uses deck ID"));
        assert!(!destination.join("decks/facts.md").exists());
    }

    #[test]
    fn an_exam_entry_that_resolves_differently_in_the_destination_refuses_before_writes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        let shared = dir.path().join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        let shared = shared.canonicalize().unwrap().display().to_string();
        workspace(&source, &format!("exam = {shared:?}\n"));
        workspace(&destination, &format!("exam = {shared:?}\n"));
        std::fs::create_dir(source.join("material")).unwrap();
        std::fs::create_dir(destination.join("material")).unwrap();
        let path = deck(&source, "facts.md", "exam: material\n");

        for mode in [TransferMode::Copy, TransferMode::Move] {
            let error = format!("{:#}", transfer(&path, &destination, mode).unwrap_err());
            let side = |root: &Path| {
                root.canonicalize()
                    .unwrap()
                    .join("material")
                    .display()
                    .to_string()
            };
            assert!(
                error.contains("`exam:` entry `material`")
                    && error.contains(&side(&source))
                    && error.contains(&side(&destination)),
                "{mode:?}: the refusal names the entry and both sides: {error}"
            );
            assert!(path.is_file(), "{mode:?}");
            assert!(!destination.join("decks/facts.md").exists(), "{mode:?}");
        }

        let path = deck(&source, "facts.md", "");
        transfer(&path, &destination, TransferMode::Copy).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            std::fs::read_to_string(destination.join("decks/facts.md")).unwrap(),
            "an allowed transfer never writes `exam:` into the deck"
        );
    }

    #[test]
    fn a_source_deck_delete_failure_rolls_back_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        workspace(&source, "");
        workspace(&destination, "");
        let deck = deck(&source, "facts.md", "");
        let mut progress = crate::state::open_store(&deck, &source).unwrap();
        progress.get_or_insert("card-card1");
        progress.save().unwrap();

        let error = transfer_with_remove(&deck, &destination, TransferMode::Move, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "blocked",
            ))
        })
        .unwrap_err();

        assert!(format!("{error:#}").contains("cannot remove"));
        assert!(deck.is_file());
        assert!(!destination.join("decks/facts.md").exists());
        assert!(!destination.join(".alix/progress/deck-deck1.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_sidecar_cleanup_failure_keeps_the_destination_and_reports_the_orphan() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        workspace(&source, "");
        workspace(&destination, "");
        let deck = deck(&source, "facts.md", "");
        let loaded = Deck::load(&deck).unwrap();
        let mut augmentation = crate::augment::AugmentCache::open_for_deck(&loaded).unwrap();
        augmentation.set_note("card-card1", "note".to_string(), 1);
        augmentation.save().unwrap();
        let source_augmentation = source.join("augment/deck-deck1.json");
        std::fs::set_permissions(
            source.join("augment"),
            std::fs::Permissions::from_mode(0o555),
        )
        .unwrap();

        let report = transfer(&deck, &destination, TransferMode::Move).unwrap();

        std::fs::set_permissions(
            source.join("augment"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        // The report canonicalizes; on macOS the raw tempdir path differs
        // (/var vs /private/var), so compare canonical forms.
        assert_eq!(
            vec![source_augmentation.canonicalize().unwrap()],
            report.leftovers
        );
        assert!(!deck.exists());
        assert!(destination.join("decks/facts.md").is_file());
        assert!(destination.join("augment/deck-deck1.json").is_file());
    }

    fn prepared_transfer() -> (tempfile::TempDir, Transfer) {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        workspace(&source, "");
        workspace(&destination, "");
        let deck_path = deck(&source, "facts.md", "");
        let loaded = Deck::load(&deck_path).unwrap();
        let mut augmentation = crate::augment::AugmentCache::open_for_deck(&loaded).unwrap();
        augmentation.set_note("card-card1", "note".to_string(), 1);
        augmentation.save().unwrap();
        let mut progress = crate::state::open_store(&deck_path, &source).unwrap();
        progress.get_or_insert("card-card1");
        progress.save().unwrap();
        let assets = source.join("assets/deck-deck1");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("evidence.txt"), b"evidence").unwrap();
        let transfer = Transfer::prepare(&deck_path, &destination, TransferMode::Move).unwrap();
        (dir, transfer)
    }

    #[test]
    fn staged_identity_verification_rejects_deck_and_card_id_changes() {
        let (_dir, transfer) = prepared_transfer();
        let bundle = tempfile::tempdir().unwrap();
        let staged = bundle.path().join("facts.md");
        let original = std::fs::read_to_string(&transfer.source).unwrap();

        std::fs::write(&staged, original.replace("deck-deck1", "deck-changed")).unwrap();
        assert!(
            format!(
                "{:#}",
                transfer.verify_staged_identity(bundle.path()).unwrap_err()
            )
            .contains("deck ID")
        );

        std::fs::write(&staged, original.replace("card-card1", "card-changed")).unwrap();
        assert!(
            format!(
                "{:#}",
                transfer.verify_staged_identity(bundle.path()).unwrap_err()
            )
            .contains("card IDs")
        );
    }

    #[test]
    fn source_verification_detects_changes_to_every_snapshotted_part() {
        let (_dir, transfer) = prepared_transfer();
        let augmentation = transfer.source_files.augment_for(&transfer.deck_id);
        let asset = transfer
            .source_files
            .assets_for(&transfer.deck_id)
            .join("evidence.txt");
        let cases = [
            (&transfer.source, transfer.baseline.deck.as_slice()),
            (
                &augmentation,
                transfer.baseline.augmentation.as_deref().unwrap(),
            ),
            (
                &transfer.source_progress,
                transfer.baseline.progress.as_deref().unwrap(),
            ),
            (&asset, b"evidence".as_slice()),
        ];

        for (path, original) in cases {
            std::fs::write(path, b"changed").unwrap();
            assert!(
                transfer.verify_source_unchanged().is_err(),
                "a change to {} must be detected",
                path.display()
            );
            std::fs::write(path, original).unwrap();
            transfer.verify_source_unchanged().unwrap();
        }
    }

    #[test]
    fn optional_file_verification_distinguishes_absent_equal_and_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("optional");
        ensure_optional_unchanged(&path, None).unwrap();
        assert!(ensure_optional_unchanged(&path, Some(b"expected")).is_err());

        std::fs::write(&path, b"expected").unwrap();
        ensure_optional_unchanged(&path, Some(b"expected")).unwrap();
        assert!(ensure_optional_unchanged(&path, None).is_err());
        assert!(ensure_optional_unchanged(&path, Some(b"changed")).is_err());
    }

    #[test]
    fn tree_reads_are_relative_recursive_and_content_exact() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert!(read_tree(&missing).unwrap().is_empty());

        let root = dir.path().join("tree");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("a"), [0]).unwrap();
        std::fs::write(root.join("nested/b"), [1, 2]).unwrap();
        assert_eq!(
            BTreeMap::from([
                (PathBuf::from("a"), vec![0]),
                (PathBuf::from("nested/b"), vec![1, 2]),
            ]),
            read_tree(&root).unwrap()
        );
    }

    #[test]
    fn successful_directory_cleanup_removes_the_tree_without_a_leftover() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remove");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("file"), b"content").unwrap();
        let mut leftovers = Vec::new();

        remove_dir_if_present(&path, &mut leftovers);

        assert!(!path.exists());
        assert!(leftovers.is_empty());
    }
}
