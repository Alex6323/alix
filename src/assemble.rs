//! Session assembly: turn deck paths into something reviewable.
//!
//! The one place that knows how a selection becomes a session, a walk, or a
//! browse — workspace expansion, augment overlays, topology and region focus,
//! virtual cards, pacing, depth. The server and the CLI both consume it; no
//! policy that changes an `/api/*` response may live outside this module.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::{
    store::{Store, default_store_path},
    workspace,
};

/// Opens the progress store (creating an empty one on first use).
pub fn open_store(path: Option<PathBuf>) -> Result<Store> {
    let path = match path {
        Some(path) => path,
        None => default_store_path().context("cannot determine the data directory")?,
    };
    Store::open(&path).context("cannot open the progress store")
}

/// Which progress store a set of decks should use: the `--store` override, else
/// the single workspace they all share (a deck is "in" a workspace when its
/// parent folder has an `alix.toml`), else the global default (`None`). Loose
/// decks, a plain folder, or decks spanning different workspaces all fall back
/// to the global store — so a workspace's progress lives with the workspace,
/// while everything else shares the one global store.
pub fn store_path_for(decks: &[PathBuf], cli_override: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = cli_override {
        return Some(path.to_path_buf());
    }
    let mut stores = decks.iter().map(|deck| {
        deck.parent()
            .filter(|p| workspace::is_workspace(p))
            .map(workspace::store_path)
    });
    match stores.next() {
        Some(Some(first)) if stores.all(|s| s.as_ref() == Some(&first)) => Some(first),
        _ => None,
    }
}

/// Opens the store for `paths`: their shared workspace store when they have
/// one, else `instance`'s store (a served folder's own file), else the global
/// default. The fallback a served instance (`alix <dir>` or bare `alix`)
/// applies once no workspace claims the selection.
pub fn store_for(paths: &[PathBuf], instance: Option<&Path>) -> Result<Store> {
    open_store(store_path_for(paths, None).or_else(|| instance.map(Path::to_path_buf)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_for_prefers_workspace_then_instance_then_global() {
        let dir = tempfile::tempdir().unwrap();
        // workspace: a dir with alix.toml + a member deck
        let ws = dir.path().join("box");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Box\"\n").unwrap();
        let member = ws.join("a.txt");
        std::fs::write(&member, "# q\n  a\n").unwrap();
        // a loose deck outside any workspace
        let loose = dir.path().join("loose.txt");
        std::fs::write(&loose, "# q\n  a\n").unwrap();
        let instance = dir.path().join("instance-progress.json");

        // workspace member -> the workspace's store, even with an instance fallback present
        let p = store_path_for(std::slice::from_ref(&member), None).expect("workspace store");
        assert_eq!(p, ws.join("progress.json"));
        // loose deck + instance fallback -> the instance store (via store_for)
        let s = store_for(std::slice::from_ref(&loose), Some(&instance)).unwrap();
        assert_eq!(s.path(), instance.as_path());
        // loose deck, no instance -> the global default (assert it is NOT under our tempdir)
        let g = store_for(std::slice::from_ref(&loose), None).unwrap();
        assert!(!g.path().starts_with(dir.path()));
    }

    #[test]
    fn store_path_for_picks_workspace_else_global_else_override() {
        let dir = tempfile::tempdir().unwrap();
        let mk_ws = |name: &str| {
            let ws = dir.path().join(name);
            std::fs::create_dir(&ws).unwrap();
            std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
            std::fs::write(ws.join("a.txt"), "# a\n\t1\n").unwrap();
            std::fs::write(ws.join("b.txt"), "# b\n\t1\n").unwrap();
            ws
        };
        let ws = mk_ws("ws");
        let ws2 = mk_ws("ws2");
        let ws_store = ws.join("progress.json");
        let loose = dir.path().join("loose.txt");
        std::fs::write(&loose, "# c\n\t1\n").unwrap();

        // a deck (or several) in one workspace → that workspace's store
        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("a.txt")], None)
        );
        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("a.txt"), ws.join("b.txt")], None)
        );
        // loose, mixed loose+workspace, and cross-workspace all → global (None)
        assert_eq!(None, store_path_for(std::slice::from_ref(&loose), None));
        assert_eq!(
            None,
            store_path_for(&[ws.join("a.txt"), loose.clone()], None)
        );
        assert_eq!(
            None,
            store_path_for(&[ws.join("a.txt"), ws2.join("a.txt")], None)
        );
        assert_eq!(None, store_path_for(&[], None));
        // --store wins over everything
        let over = dir.path().join("x.json");
        assert_eq!(
            Some(over.clone()),
            store_path_for(&[ws.join("a.txt")], Some(&over))
        );
    }
}
