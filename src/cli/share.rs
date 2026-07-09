//! `alix share`/`receive`: send or receive a deck, folder, or workspace over
//! magic-wormhole (or a `.zip` fallback), staging out personal state so only
//! deck content travels.

use std::path::{Path, PathBuf};

use alix::{config::Config, workspace};
use anyhow::{Context, Result, bail};

use crate::{ReceiveArgs, ShareArgs, common::deck_out_dir};

/// `alix share`: stage a personal-state-free copy and hand it to wormhole.
/// The wormhole binary prints the code mnemonic and the progress itself.
pub(crate) fn share_cmd(args: ShareArgs) -> Result<()> {
    let path = &args.path;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shared-decks")
        .to_string();

    // A single deck has no personal state and travels as-is (its augmentations
    // live in a shared per-store cache and stay home). A folder is staged
    // first, so progress and personal config never leave.
    let tmp = tempfile::tempdir().context("cannot create a staging directory")?;
    let (to_send, staged) = if path.is_file() {
        (path.clone(), 1)
    } else {
        if !path.is_dir() {
            bail!("`{}` is neither a deck file nor a folder", path.display());
        }
        if !workspace::has_decks(path) {
            bail!("no decks in `{}` — nothing to share", path.display());
        }
        let stage = tmp.path().join(&name);
        let staged = alix::share::stage_dir(path, &stage)?;
        (stage, staged)
    };

    // `--zip`: the offline fallback — write an archive instead of sending.
    if args.zip {
        let stem = name.strip_suffix(".txt").unwrap_or(&name);
        let out = match &args.output {
            Some(p) if p.is_dir() => p.join(format!("{stem}.zip")),
            Some(p) => p.clone(),
            None => PathBuf::from(format!("{stem}.zip")),
        };
        let entries = alix::share::zip_to(&to_send, &out)?;
        println!(
            "Wrote {} ({entries} files — progress and personal config stay home).",
            out.display()
        );
        return Ok(());
    }

    println!(
        "Sharing {name} ({staged} files — progress and personal config stay home). \
         Tell the receiver the code below."
    );
    alix::share::wormhole(&["send", &to_send.to_string_lossy()], None)
}

/// `alix receive`: run wormhole in a scratch dir, strip any leaked personal
/// files, and move the result where it belongs.
pub(crate) fn receive_cmd(args: ReceiveArgs) -> Result<()> {
    let config = Config::load(None)?;
    let tmp = tempfile::tempdir().context("cannot create a receiving directory")?;
    // A `.zip` path skips the wormhole entirely — same staging, same landing.
    let zip_path = Path::new(&args.code);
    if args.code.ends_with(".zip") && zip_path.is_file() {
        alix::share::unzip_to(zip_path, tmp.path())?;
    } else {
        alix::share::wormhole(&["receive", "--accept-file", &args.code], Some(tmp.path()))?;
    }

    // Whatever arrived is the single new entry in the scratch dir.
    let mut entries: Vec<PathBuf> = std::fs::read_dir(tmp.path())?
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

    if got.is_dir() {
        if args.workspace.is_some() {
            bail!(
                "--workspace places a received deck; a folder lands under the decks dir as `{name}`"
            );
        }
        let removed = alix::share::sanitize_received(&got)?;
        for r in &removed {
            println!("stripped a leaked personal file: {r}");
        }
        let dest = config
            .decks_dir()
            .context("cannot determine the decks directory")?
            .join(&name);
        if dest.exists() {
            bail!(
                "{} already exists — move it aside first (folders are never overwritten)",
                dest.display()
            );
        }
        alix::share::move_into(&got, &dest)?;
        println!(
            "Received {} — open it:  alix {}",
            dest.display(),
            dest.display()
        );
    } else {
        let dest_dir = deck_out_dir(args.workspace.as_deref(), &config)?;
        std::fs::create_dir_all(&dest_dir)
            .with_context(|| format!("cannot create {}", dest_dir.display()))?;
        let dest = dest_dir.join(&name);
        if dest.exists() && !args.force {
            bail!(
                "{} already exists; pass --force to overwrite",
                dest.display()
            );
        }
        alix::share::move_into(&got, &dest)?;
        println!(
            "Received {} — it shows up in the picker (`alix`).",
            dest.display()
        );
    }
    Ok(())
}
