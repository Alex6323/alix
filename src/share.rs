//! Sharing decks and workspaces over magic-wormhole (`alix share` /
//! `alix receive`): stage a copy free of personal state, shell out to the
//! `wormhole` binary for the transfer, and integrate what arrives. The
//! transfer, the code mnemonic, and the progress output are wormhole's job —
//! alix only decides what travels and where it lands.

use std::{
    io::BufRead,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc, mpsc::Receiver},
};

use anyhow::{Context, Result, bail};

/// Personal state that must never travel: progress, the recent list, and the
/// private pacing overrides. Excluded from staging AND stripped defensively
/// from anything received (the sender may not have used `alix share`).
pub const PERSONAL: [&str; 3] = ["progress.json", "recent.json", "alix.local.toml"];

/// `true` for entries that stay home when sharing: personal state, hidden
/// files, and backup files from one-off rewrites (`*.bak`, `*-bak`).
fn stays_home(name: &str) -> bool {
    PERSONAL.contains(&name)
        || name.starts_with('.')
        || name.ends_with(".bak")
        || name.ends_with("-bak")
}

/// Copies `dir`'s shareable content into `stage` (created fresh): decks,
/// `alix.toml`, `assets/`, and the precomputed `augment.json` — everything
/// except what [`stays_home`]. Returns how many files were staged.
pub fn stage_dir(dir: &Path, stage: &Path) -> Result<usize> {
    std::fs::create_dir_all(stage).with_context(|| format!("cannot create {}", stage.display()))?;
    let mut staged = 0;
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if stays_home(&name) {
            continue;
        }
        let from = entry.path();
        let to = stage.join(&name);
        if from.is_dir() {
            staged += stage_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).with_context(|| format!("cannot copy {}", from.display()))?;
            staged += 1;
        }
    }
    Ok(staged)
}

/// Removes any personal files that leaked into a received folder, at any
/// depth. Returns the (relative) names removed, for reporting.
pub fn sanitize_received(dir: &Path) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if PERSONAL.contains(&name.as_str()) {
            if path.is_file() {
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

/// Moves `from` to `to`, falling back to copy+delete when a plain rename
/// crosses filesystems (a temp dir usually does).
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

/// Writes `path` (a staged folder or a single deck file) as a ZIP archive at
/// `out` — the offline fallback when wormhole isn't available. Returns the
/// number of entries written.
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
            .unwrap_or_else(|| "deck.txt".to_string());
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

/// Extracts a `.zip` (made by `alix share --zip`, or compatible) into `dest`.
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

/// Runs the wormhole binary with inherited stdio — the code mnemonic and the
/// transfer progress print straight to the terminal — and waits for it.
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

/// Events a UI transfer reports as it progresses. The code arrives long
/// before the transfer finishes — show it and keep polling.
#[derive(Debug)]
pub enum ShareEvent {
    Code(String),
    Done,
    Error(String),
}

/// One UI-driven wormhole transfer: events stream on `events`; `cancel`
/// kills the child (a sender waits indefinitely for its receiver, so the
/// UI must be able to stop it).
pub struct ShareJob {
    pub events: Receiver<ShareEvent>,
    child: Arc<Mutex<Child>>,
}

impl ShareJob {
    /// Kills the transfer; the waiter then reports an error event, which the
    /// caller discards along with the job.
    pub fn cancel(&self) {
        if let Ok(mut c) = self.child.lock() {
            c.kill().ok();
        }
    }
}

impl Drop for ShareJob {
    /// An abandoned job must not leave a wormhole process running — dropping
    /// cancels. Killing an already-exited child is a harmless no-op.
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Spawns `wormhole send <path>` for a UI, scanning its output for the code.
pub fn send_spawn(path: &Path) -> Result<ShareJob> {
    spawn_job("wormhole", &["send", &path.to_string_lossy()], None)
}

/// Spawns `wormhole receive --accept-file <code>` into `dest` for a UI.
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
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(pipe).lines().map_while(|l| l.ok()) {
                if let Some(code) = line.trim().strip_prefix("Wormhole code is:") {
                    let _ = tx.send(ShareEvent::Code(code.trim().to_string()));
                }
            }
        });
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

/// Lands whatever a receive left in `tmp`: expects exactly one entry, strips
/// leaked personal files, and moves it under `dest_dir` — never overwriting.
/// Returns the landed name and the stripped (relative) file names.
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
        touch(&src, "a.txt");
        touch(&src, "alix.toml");
        touch(&src, "augment.json");
        touch(&src, "progress.json");
        touch(&src, "recent.json");
        touch(&src, "alix.local.toml");
        touch(&src, "progress.json.predepth-bak");
        touch(&src.join("assets"), "icon.svg");

        let stage = dir.path().join("stage");
        let n = stage_dir(&src, &stage).unwrap();

        assert_eq!(4, n, "a.txt, alix.toml, augment.json, assets/icon.svg");
        assert!(stage.join("a.txt").exists());
        assert!(stage.join("alix.toml").exists());
        assert!(stage.join("augment.json").exists());
        assert!(stage.join("assets/icon.svg").exists());
        assert!(!stage.join("progress.json").exists());
        assert!(!stage.join("recent.json").exists());
        assert!(!stage.join("alix.local.toml").exists());
        assert!(!stage.join("progress.json.predepth-bak").exists());
    }

    #[test]
    fn sanitize_strips_leaked_personal_files_at_any_depth() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("got");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        touch(&root, "a.txt");
        touch(&root, "progress.json");
        touch(&root.join("nested"), "alix.local.toml");

        let removed = sanitize_received(&root).unwrap();

        assert!(root.join("a.txt").exists());
        assert!(!root.join("progress.json").exists());
        assert!(!root.join("nested/alix.local.toml").exists());
        assert_eq!(2, removed.len(), "{removed:?}");
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
        std::fs::write(tmp.join("ws/progress.json"), "x").unwrap();
        let dest = dir.path().join("decks");
        std::fs::create_dir_all(&dest).unwrap();
        let (landed, stripped) = land_received(&tmp, &dest).unwrap();
        assert_eq!("ws", landed);
        assert_eq!(vec!["progress.json".to_string()], stripped);
        assert!(dest.join("ws/a.txt").exists());
        assert!(!dest.join("ws/progress.json").exists());
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
