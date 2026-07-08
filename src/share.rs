//! Sharing decks and workspaces over magic-wormhole (`alix share` /
//! `alix receive`): stage a copy free of personal state, shell out to the
//! `wormhole` binary for the transfer, and integrate what arrives. The
//! transfer, the code mnemonic, and the progress output are wormhole's job —
//! alix only decides what travels and where it lands.

use std::{path::Path, process::Command};

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
    fn a_missing_wormhole_binary_errors_with_the_install_hint() {
        let err = wormhole_with("definitely-not-wormhole-xyz", &["send"], None).unwrap_err();
        assert!(
            format!("{err:#}").contains("magic-wormhole installed"),
            "{err:#}"
        );
    }
}
