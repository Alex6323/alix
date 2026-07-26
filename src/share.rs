use std::{
    io::BufRead,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc, mpsc::Receiver},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Named personal entries excluded from staging and stripped on receive.
pub const PERSONAL: [&str; 3] = ["progress", "recent.json", "alix.local.toml"];
const DECK_BUNDLE_MARKER: &str = ".alix-deck-share.json";
const DECK_BUNDLE_VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
struct DeckBundle {
    version: u32,
    deck: String,
}

fn stays_home(name: &str) -> bool {
    PERSONAL.contains(&name)
        || name.starts_with('.')
        || crate::workspace::is_conflict_name(name)
        || name.ends_with("-bak")
        || name.ends_with(".json.tmp")
}

pub fn stage_path(path: &Path, state_root: &Path, stage_root: &Path) -> Result<(PathBuf, usize)> {
    if path.is_dir() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("shared-decks");
        let stage = stage_root.join(name);
        let staged = stage_dir(path, &stage)?;
        return Ok((stage, staged));
    }
    if !path.is_file() {
        bail!("`{}` is neither a deck file nor a folder", path.display());
    }
    let deck = crate::deck::Deck::load(path)?;
    let Some(deck_id) = deck.deck_token.as_deref() else {
        return Ok((path.to_path_buf(), 1));
    };
    let augmentation = crate::state::Layout::new(state_root).augment_for(deck_id);
    if !augmentation.is_file() {
        return Ok((path.to_path_buf(), 1));
    }
    crate::augment::read_deck_data(&augmentation, deck_id)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("deck.md");
    let stem = file_name.strip_suffix(".md").unwrap_or(file_name);
    let stage = stage_root.join(format!("{stem}.alix-deck"));
    std::fs::create_dir_all(stage.join("augment"))
        .with_context(|| format!("cannot create {}", stage.display()))?;
    std::fs::copy(path, stage.join(file_name))
        .with_context(|| format!("cannot copy {}", path.display()))?;
    std::fs::copy(
        &augmentation,
        stage.join("augment").join(format!("{deck_id}.json")),
    )
    .with_context(|| format!("cannot copy {}", augmentation.display()))?;
    let marker = serde_json::to_string_pretty(&DeckBundle {
        version: DECK_BUNDLE_VERSION,
        deck: file_name.to_string(),
    })?;
    std::fs::write(stage.join(DECK_BUNDLE_MARKER), marker)
        .with_context(|| format!("cannot write {}", stage.display()))?;
    Ok((stage, 3))
}

pub fn stage_dir(dir: &Path, stage: &Path) -> Result<usize> {
    std::fs::create_dir_all(stage).with_context(|| format!("cannot create {}", stage.display()))?;
    let deck_ids: std::collections::HashSet<String> = crate::workspace::deck_files(dir)
        .into_iter()
        .filter_map(|path| crate::deck::Deck::load(path).ok()?.deck_token)
        .collect();
    let mut staged = 0;
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let from = entry.path();
        if stays_home(&name) {
            continue;
        }
        let to = stage.join(&name);
        if name == "augment" && from.is_dir() {
            staged += stage_augmentation(&from, &to, &deck_ids)?;
        } else if from.is_dir() {
            staged += stage_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).with_context(|| format!("cannot copy {}", from.display()))?;
            staged += 1;
        }
    }
    Ok(staged)
}

fn stage_augmentation(
    dir: &Path,
    stage: &Path,
    deck_ids: &std::collections::HashSet<String>,
) -> Result<usize> {
    let mut staged = 0;
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let from = entry.path();
        let Some(deck_id) = crate::state::deck_id_from_document(&from) else {
            continue;
        };
        if !from.is_file()
            || !deck_ids.contains(deck_id)
            || from
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(crate::workspace::is_conflict_name)
        {
            continue;
        }
        std::fs::create_dir_all(stage)
            .with_context(|| format!("cannot create {}", stage.display()))?;
        std::fs::copy(&from, stage.join(entry.file_name()))
            .with_context(|| format!("cannot copy {}", from.display()))?;
        staged += 1;
    }
    Ok(staged)
}

pub fn sanitize_received(dir: &Path) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let private = PERSONAL.contains(&name.as_str())
            || crate::workspace::is_conflict_name(&name)
            || name.ends_with("-bak")
            || name.ends_with(".json.tmp")
            || (name.starts_with('.') && name != DECK_BUNDLE_MARKER);
        if private {
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
                removed.push(name);
            } else if path.is_file() {
                std::fs::remove_file(&path)?;
                removed.push(name);
            }
        } else if path.is_dir() {
            for inner in sanitize_received(&path)? {
                removed.push(format!("{name}/{inner}"));
            }
        }
    }
    Ok(removed)
}

pub fn move_into(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        copy_tree(from, to)?;
        std::fs::remove_dir_all(from).ok();
    } else {
        std::fs::copy(from, to).with_context(|| format!("cannot copy to {}", to.display()))?;
        std::fs::remove_file(from).ok();
    }
    Ok(())
}

/// Recursive copy with no filtering (the staging already filtered).
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

pub fn zip_to(path: &Path, out: &Path) -> Result<usize> {
    use std::io::Write;
    let file =
        std::fs::File::create(out).with_context(|| format!("cannot create {}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::SimpleFileOptions = Default::default();
    let mut entries = 0;
    if path.is_file() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "deck.md".to_string());
        zip.start_file(name, options)?;
        zip.write_all(&std::fs::read(path)?)?;
        entries = 1;
    } else {
        let root = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "decks".to_string());
        zip_walk(&mut zip, path, &root, options, &mut entries)?;
    }
    zip.finish()?;
    Ok(entries)
}

fn zip_walk(
    zip: &mut zip::ZipWriter<std::fs::File>,
    dir: &Path,
    prefix: &str,
    options: zip::write::SimpleFileOptions,
    entries: &mut usize,
) -> Result<()> {
    use std::io::Write;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
        if entry.path().is_dir() {
            zip_walk(zip, &entry.path(), &name, options, entries)?;
        } else {
            zip.start_file(name, options)?;
            zip.write_all(&std::fs::read(entry.path())?)?;
            *entries += 1;
        }
    }
    Ok(())
}

/// The zip crate's extract handles hostile paths (zip-slip) itself.
pub fn unzip_to(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("cannot open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("{} is not a readable zip archive", zip_path.display()))?;
    archive
        .extract(dest)
        .with_context(|| format!("cannot extract {}", zip_path.display()))?;
    Ok(())
}

pub fn wormhole(args: &[&str], cwd: Option<&Path>) -> Result<()> {
    wormhole_with("wormhole", args, cwd)
}

fn wormhole_with(cmd: &str, args: &[&str], cwd: Option<&Path>) -> Result<()> {
    let mut command = Command::new(cmd);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let status = command.status().with_context(|| {
        format!(
            "cannot run `{cmd}` — is magic-wormhole installed? \
             (e.g. `pipx install magic-wormhole`, or your package manager)"
        )
    })?;
    if !status.success() {
        bail!("`{cmd} {}` failed", args.join(" "));
    }
    Ok(())
}

#[derive(Debug)]
pub enum ShareEvent {
    Code(String),
    Done,
    Error(String),
}

/// A sender waits indefinitely for its receiver, so `cancel` must be able to
/// kill the child.
pub struct ShareJob {
    pub events: Receiver<ShareEvent>,
    child: Arc<Mutex<Child>>,
}

impl ShareJob {
    pub fn cancel(&self) {
        if let Ok(mut c) = self.child.lock() {
            c.kill().ok();
        }
    }
}

impl Drop for ShareJob {
    /// An abandoned job must not leave a wormhole process running.
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn send_spawn(path: &Path) -> Result<ShareJob> {
    spawn_job("wormhole", &["send", &path.to_string_lossy()], None)
}

pub fn receive_spawn(code: &str, dest: &Path) -> Result<ShareJob> {
    spawn_job("wormhole", &["receive", "--accept-file", code], Some(dest))
}

fn spawn_job(cmd: &str, args: &[&str], cwd: Option<&Path>) -> Result<ShareJob> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = command.spawn().with_context(|| {
        format!(
            "cannot run `{cmd}` — is magic-wormhole installed? \
             (e.g. `pipx install magic-wormhole`, or your package manager)"
        )
    })?;
    let (tx, rx) = mpsc::channel();
    let mut readers = Vec::new();
    // magic-wormhole prints the code line to stderr; scan both pipes to be safe.
    for pipe in [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in std::io::BufReader::new(pipe).lines().map_while(|l| l.ok()) {
                if let Some(code) = line.trim().strip_prefix("Wormhole code is:") {
                    let _ = tx.send(ShareEvent::Code(code.trim().to_string()));
                }
            }
        }));
    }
    let child = Arc::new(Mutex::new(child));
    let waiter = Arc::clone(&child);
    std::thread::spawn(move || {
        // Poll-wait so `cancel` never contends with a blocking `wait`.
        loop {
            let status = waiter
                .lock()
                .ok()
                .and_then(|mut c| c.try_wait().ok().flatten());
            if let Some(status) = status {
                // Drains before the terminal event: a fast-exiting process
                // could otherwise have Done overtake its own Code line.
                // Bounded by EOF, since exit closes the pipes.
                for reader in readers.drain(..) {
                    let _ = reader.join();
                }
                let _ = tx.send(if status.success() {
                    ShareEvent::Done
                } else {
                    ShareEvent::Error("the wormhole transfer failed or was cancelled".to_string())
                });
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    });
    Ok(ShareJob { events: rx, child })
}

pub fn land_received(tmp: &Path, dest_dir: &Path) -> Result<(String, Vec<String>)> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(tmp)?
        .flatten()
        .map(|e| e.path())
        .collect();
    let Some(got) = entries.pop().filter(|_| entries.is_empty()) else {
        bail!("expected exactly one received file or folder");
    };
    let name = got
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("received")
        .to_string();
    if is_deck_bundle(&got) {
        return land_deck_bundle(&got, dest_dir);
    }
    let stripped = if got.is_dir() {
        sanitize_received(&got)?
    } else {
        Vec::new()
    };
    let dest = dest_dir.join(&name);
    if dest.exists() {
        bail!("{} already exists — move it aside first", dest.display());
    }
    move_into(&got, &dest)?;
    Ok((name, stripped))
}

pub fn is_deck_bundle(path: &Path) -> bool {
    path.is_dir() && path.join(DECK_BUNDLE_MARKER).is_file()
}

pub fn land_deck_bundle(bundle: &Path, dest_dir: &Path) -> Result<(String, Vec<String>)> {
    land_deck_bundle_with_force(bundle, dest_dir, false)
}

pub fn land_deck_bundle_with_force(
    bundle: &Path,
    dest_dir: &Path,
    force: bool,
) -> Result<(String, Vec<String>)> {
    let stripped = sanitize_received(bundle)?;
    let marker_path = bundle.join(DECK_BUNDLE_MARKER);
    let marker: DeckBundle = serde_json::from_str(
        &std::fs::read_to_string(&marker_path)
            .with_context(|| format!("cannot read {}", marker_path.display()))?,
    )
    .with_context(|| format!("cannot read {}", marker_path.display()))?;
    if marker.version != DECK_BUNDLE_VERSION
        || Path::new(&marker.deck)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(marker.deck.as_str())
        || !marker.deck.ends_with(".md")
    {
        bail!("{} is not a supported deck share", bundle.display());
    }
    let source_deck = bundle.join(&marker.deck);
    let deck = crate::deck::Deck::load(&source_deck)?;
    let deck_id = deck
        .deck_token
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("{} is not initialized", source_deck.display()))?;
    let source_augmentation = bundle.join("augment").join(format!("{deck_id}.json"));
    let (source_revision, source_data) =
        crate::augment::read_deck_data(&source_augmentation, deck_id)?;

    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("cannot create {}", dest_dir.display()))?;
    let destination = dest_dir.join(&marker.deck);
    if destination.exists() && !force {
        bail!(
            "{} already exists; move it aside first",
            destination.display()
        );
    }
    let staged_deck = dest_dir.join(format!(".{}.receive.md", marker.deck));
    if staged_deck.exists() {
        bail!(
            "{} already exists; move it aside first",
            staged_deck.display()
        );
    }
    std::fs::copy(&source_deck, &staged_deck)
        .with_context(|| format!("cannot stage {}", destination.display()))?;

    let state_root = crate::workspace::root_store_path(dest_dir);
    let (layout, _) = match crate::state::prepare(&staged_deck, &state_root) {
        Ok(prepared) => prepared,
        Err(error) => {
            let _ = std::fs::remove_file(&staged_deck);
            return Err(error.into());
        }
    };
    let destination_augmentation = layout.augment_for(deck_id);
    let destination_revision = if destination_augmentation.is_file() {
        let (revision, data) = crate::augment::read_deck_data(&destination_augmentation, deck_id)?;
        if !force && data != source_data && (!data.cards.is_empty() || !data.topologies.is_empty())
        {
            let _ = std::fs::remove_file(&staged_deck);
            bail!(
                "{} already has different augmentation for deck `{deck_id}`",
                destination_augmentation.display()
            );
        }
        revision
    } else {
        0
    };
    crate::augment::write_deck_data(
        &destination_augmentation,
        deck_id,
        source_revision.max(destination_revision.saturating_add(1)),
        &source_data,
    )?;
    move_into(&staged_deck, &destination)
        .with_context(|| format!("cannot write {}", destination.display()))?;
    Ok((marker.deck, stripped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), "x").unwrap();
    }

    #[test]
    fn staging_excludes_personal_state_and_keeps_content() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("ws");
        std::fs::create_dir_all(src.join("assets")).unwrap();
        std::fs::create_dir_all(src.join("decks")).unwrap();
        std::fs::write(
            src.join("decks/deck.md"),
            "---\nalix-id: deck1\n---\n## q <!-- id: card1 -->\na\n",
        )
        .unwrap();
        touch(&src, "alix.toml");
        touch(&src, "recent.json");
        touch(&src, "alix.local.toml");
        std::fs::create_dir(src.join("progress")).unwrap();
        touch(&src.join("progress"), "deck1.json");
        std::fs::create_dir(src.join("augment")).unwrap();
        touch(&src.join("augment"), "deck1.json");
        touch(&src.join("augment"), "orphan.json");
        touch(&src.join("assets"), "icon.svg");

        let stage = dir.path().join("stage");
        let n = stage_dir(&src, &stage).unwrap();

        assert_eq!(
            4, n,
            "decks/deck.md, alix.toml, augment/deck1.json, assets/icon.svg"
        );
        assert!(stage.join("decks/deck.md").exists());
        assert!(stage.join("alix.toml").exists());
        assert!(stage.join("augment/deck1.json").exists());
        assert!(!stage.join("augment/orphan.json").exists());
        assert!(stage.join("assets/icon.svg").exists());
        assert!(!stage.join("progress").exists());
        assert!(!stage.join("recent.json").exists());
        assert!(!stage.join("alix.local.toml").exists());
    }

    #[test]
    fn a_single_deck_round_trip_carries_augmentation_but_not_progress() {
        let dir = tempfile::tempdir().unwrap();
        let sender = dir.path().join("sender");
        std::fs::create_dir(&sender).unwrap();
        let deck_path = sender.join("math.md");
        std::fs::write(
            &deck_path,
            "---\nalix-id: deck1\n---\n## q <!-- id: card1 -->\na\n",
        )
        .unwrap();
        let mut progress = crate::state::open_store(&deck_path, &sender).unwrap();
        progress.get_or_insert("card1", 1);
        progress.save().unwrap();
        let mut augmentation = crate::state::open_augment(&deck_path, &sender).unwrap();
        augmentation.set_note("card1", "shared note".to_string(), 7);
        augmentation.save().unwrap();

        let transfer = dir.path().join("transfer");
        std::fs::create_dir(&transfer).unwrap();
        let (bundle, count) = stage_path(&deck_path, &sender, &transfer).unwrap();

        assert_eq!(3, count);
        assert!(is_deck_bundle(&bundle));
        assert!(bundle.join("math.md").is_file());
        assert!(bundle.join("augment/deck1.json").is_file());
        assert!(!bundle.join("progress").exists());

        let receiver = dir.path().join("receiver");
        let (landed, stripped) = land_received(&transfer, &receiver).unwrap();

        assert_eq!("math.md", landed);
        assert!(stripped.is_empty());
        let received_deck = receiver.join("math.md");
        let received_progress = crate::state::open_store(&received_deck, &receiver).unwrap();
        assert!(received_progress.get("card1").is_none());
        let received_augmentation = crate::state::open_augment(&received_deck, &receiver).unwrap();
        assert_eq!(Some("shared note"), received_augmentation.note("card1", 7));

        let mut changed_augmentation =
            crate::state::open_augment(&received_deck, &receiver).unwrap();
        changed_augmentation.set_note("card1", "local note".to_string(), 7);
        changed_augmentation.save().unwrap();
        std::fs::write(
            &received_deck,
            "---\nalix-id: deck1\n---\n## changed <!-- id: card1 -->\nlocally\n",
        )
        .unwrap();

        land_deck_bundle_with_force(&bundle, &receiver, true).unwrap();

        assert_eq!(
            std::fs::read_to_string(&deck_path).unwrap(),
            std::fs::read_to_string(&received_deck).unwrap()
        );
        let received_augmentation = crate::state::open_augment(&received_deck, &receiver).unwrap();
        assert_eq!(Some("shared note"), received_augmentation.note("card1", 7));
    }

    #[test]
    fn sanitize_strips_leaked_personal_files_at_any_depth() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("got");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        touch(&root, "a.txt");
        touch(&root, ".private");
        touch(&root.join("nested"), "alix.local.toml");
        std::fs::create_dir(root.join("nested/progress")).unwrap();
        touch(&root.join("nested/progress"), "deck1.json");
        std::fs::create_dir(root.join("nested/augment")).unwrap();
        touch(
            &root.join("nested/augment"),
            "deck1.sync-conflict-20260725-phone.json",
        );

        let removed = sanitize_received(&root).unwrap();

        assert!(root.join("a.txt").exists());
        assert!(!root.join(".private").exists());
        assert!(!root.join("nested/alix.local.toml").exists());
        assert!(!root.join("nested/progress").exists());
        assert!(
            !root
                .join("nested/augment/deck1.sync-conflict-20260725-phone.json")
                .exists()
        );
        assert_eq!(4, removed.len(), "{removed:?}");
    }

    #[test]
    fn zipping_a_staged_folder_writes_every_entry_under_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("ws");
        std::fs::create_dir_all(src.join("assets")).unwrap();
        touch(&src, "a.txt");
        touch(&src.join("assets"), "icon.svg");

        let out = dir.path().join("ws.zip");
        let n = zip_to(&src, &out).unwrap();

        assert_eq!(2, n);
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.contains(&"ws/a.txt".to_string()), "{names:?}");
        assert!(
            names.contains(&"ws/assets/icon.svg".to_string()),
            "{names:?}"
        );
    }

    #[test]
    fn a_zip_round_trip_restores_the_staged_tree() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("ws");
        std::fs::create_dir_all(src.join("assets")).unwrap();
        touch(&src, "a.txt");
        touch(&src.join("assets"), "icon.svg");
        let archive = dir.path().join("ws.zip");
        zip_to(&src, &archive).unwrap();

        let out = dir.path().join("landed");
        unzip_to(&archive, &out).unwrap();

        assert!(out.join("ws/a.txt").exists());
        assert!(out.join("ws/assets/icon.svg").exists());
    }

    #[test]
    fn a_missing_wormhole_binary_errors_with_the_install_hint() {
        let err = wormhole_with("definitely-not-wormhole-xyz", &["send"], None).unwrap_err();
        assert!(
            format!("{err:#}").contains("magic-wormhole installed"),
            "{err:#}"
        );
    }

    #[test]
    fn a_send_job_reports_the_code_then_done() {
        let _lock = crate::testutil::exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let fake =
            crate::testutil::fake_cli(dir.path(), "echo 'Wormhole code is: 7-alpha-bravo'\nexit 0");
        let job = spawn_job(&fake.to_string_lossy(), &["send", "x"], None).unwrap();
        let mut got = Vec::new();
        while let Ok(ev) = job.events.recv_timeout(std::time::Duration::from_secs(10)) {
            got.push(ev);
        }
        assert!(
            matches!(got.first(), Some(ShareEvent::Code(c)) if c == "7-alpha-bravo"),
            "{got:?}"
        );
        assert!(matches!(got.last(), Some(ShareEvent::Done)), "{got:?}");
    }

    #[test]
    fn a_failing_send_job_reports_an_error() {
        let _lock = crate::testutil::exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let fake = crate::testutil::fake_cli(dir.path(), "exit 1");
        let job = spawn_job(&fake.to_string_lossy(), &["send", "x"], None).unwrap();
        let last = std::iter::from_fn(|| {
            job.events
                .recv_timeout(std::time::Duration::from_secs(10))
                .ok()
        })
        .last();
        assert!(matches!(last, Some(ShareEvent::Error(_))), "{last:?}");
    }

    #[test]
    fn cancelling_a_running_job_reports_an_error_event_promptly() {
        let _lock = crate::testutil::exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let fake = crate::testutil::fake_cli(dir.path(), "sleep 30");
        let job = spawn_job(&fake.to_string_lossy(), &["send", "x"], None).unwrap();
        job.cancel();
        let ev = job
            .events
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(ev, ShareEvent::Error(_)), "{ev:?}");
    }

    #[test]
    fn landing_a_received_folder_sanitizes_and_moves_it() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("scratch");
        std::fs::create_dir_all(tmp.join("ws")).unwrap();
        std::fs::write(tmp.join("ws/a.txt"), "x").unwrap();
        std::fs::create_dir(tmp.join("ws/progress")).unwrap();
        std::fs::write(tmp.join("ws/progress/deck1.json"), "x").unwrap();
        let dest = dir.path().join("decks");
        std::fs::create_dir_all(&dest).unwrap();
        let (landed, stripped) = land_received(&tmp, &dest).unwrap();
        assert_eq!("ws", landed);
        assert_eq!(vec!["progress".to_string()], stripped);
        assert!(dest.join("ws/a.txt").exists());
        assert!(!dest.join("ws/progress").exists());
    }

    #[test]
    fn landing_onto_an_existing_name_errors_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("scratch");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.txt"), "new").unwrap();
        let dest = dir.path().join("decks");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("a.txt"), "old").unwrap();
        assert!(land_received(&tmp, &dest).is_err());
        assert_eq!("old", std::fs::read_to_string(dest.join("a.txt")).unwrap());
    }
}
