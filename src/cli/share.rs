use std::path::{Path, PathBuf};

use alix::{config::Config, workspace};
use anyhow::{Context, Result, bail};

use crate::{ReceiveArgs, ShareArgs, common::deck_out_dir};

pub(crate) fn share_cmd(args: ShareArgs) -> Result<()> {
    let path = &args.path;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shared-decks")
        .to_string();

    let tmp = tempfile::tempdir().context("cannot create a staging directory")?;
    if path.is_dir() && !workspace::has_decks(path) {
        bail!("no decks in `{}`; nothing to share", path.display());
    }
    let (to_send, staged) = alix::share::stage_path(path, tmp.path())?;

    if args.zip {
        let stem = name.strip_suffix(".md").unwrap_or(&name);
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

pub(crate) fn receive_cmd(args: ReceiveArgs) -> Result<()> {
    let config = Config::load(None)?;
    let tmp = tempfile::tempdir().context("cannot create a receiving directory")?;
    let zip_path = Path::new(&args.code);
    if args.code.ends_with(".zip") && zip_path.is_file() {
        alix::share::unzip_to(zip_path, tmp.path())?;
    } else {
        alix::share::wormhole(&["receive", "--accept-file", &args.code], Some(tmp.path()))?;
    }

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
    alix::share::refuse_received_link(&got)?;

    if got.is_dir() && !alix::share::is_deck_bundle(&got) {
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
        if alix::share::is_deck_bundle(&got) {
            let scratch = tempfile::tempdir().context("cannot stage the received deck")?;
            let staged = scratch.path().join(&name);
            alix::share::move_into(&got, &staged)?;
            let (landed, stripped) =
                alix::share::land_deck_bundle_with_force(&staged, &dest_dir, args.force)?;
            for item in stripped {
                println!("stripped a leaked personal file: {item}");
            }
            println!(
                "Received {}; it shows up in the picker (`alix`).",
                dest_dir.join(landed).display()
            );
            return Ok(());
        }
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
