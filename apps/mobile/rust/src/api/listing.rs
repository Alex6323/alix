//! The mobile deck picker's data: thin field-maps over the core lister
//! (`alix::listing`), which owns the scan rules, titles, and the due-now
//! semantics shared with the web picker.

use std::path::Path;

/// One row of a folder listing, as the picker screen renders it.
pub struct DeckEntry {
    pub title: String,
    pub path: String,
    /// Drillable: list its members with [`list_members`].
    pub is_workspace: bool,
    /// Anything to do right now, against the store this entry reviews into.
    pub due: bool,
    /// A trace deck (`% trace:`): a predict-and-verify walk that lives in
    /// the web app; the phone cannot review it, so the picker must say so
    /// instead of opening a session the core will refuse.
    pub is_trace: bool,
}

impl From<alix::listing::DeckSummary> for DeckEntry {
    fn from(s: alix::listing::DeckSummary) -> Self {
        DeckEntry {
            title: s.title,
            path: s.path.to_string_lossy().into_owned(),
            is_workspace: s.is_workspace,
            due: s.due,
            is_trace: s.is_trace,
        }
    }
}

/// Lists the decks root. `now_ms` injects the clock (tests); `None` is now.
#[flutter_rust_bridge::frb(sync)]
pub fn list_root(root: String, now_ms: Option<u64>) -> Vec<DeckEntry> {
    let now = now_ms.unwrap_or_else(alix::time::now_ms);
    alix::listing::list_root(
        Path::new(&root),
        &alix::config::ReviewConfig::default(),
        now,
    )
    .into_iter()
    .map(DeckEntry::from)
    .collect()
}

/// Syncthing conflict copies next to any store under `root`: non-empty
/// means two devices wrote concurrently and the picker should warn before
/// the user reviews on top of a fork.
#[flutter_rust_bridge::frb(sync)]
pub fn sync_conflicts(root: String) -> Vec<String> {
    alix::listing::sync_conflicts_under(Path::new(&root))
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Lists one drillable folder of the root.
#[flutter_rust_bridge::frb(sync)]
pub fn list_members(root: String, dir: String, now_ms: Option<u64>) -> Vec<DeckEntry> {
    let now = now_ms.unwrap_or_else(alix::time::now_ms);
    alix::listing::list_members(
        Path::new(&root),
        Path::new(&dir),
        &alix::config::ReviewConfig::default(),
        now,
    )
    .into_iter()
    .map(DeckEntry::from)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_a_root_with_a_workspace_and_a_loose_deck() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("loose.txt"), "% title: Loose\n# q\n\ta\n").unwrap();
        std::fs::create_dir(root.join("ws")).unwrap();
        std::fs::write(root.join("ws/alix.toml"), "title = \"Ws\"\n").unwrap();
        std::fs::write(root.join("ws/m.txt"), "# q\n\ta\n").unwrap();

        let rows = list_root(root.to_string_lossy().into_owned(), Some(1_000_000));
        let titles: Vec<(&str, bool, bool)> = rows
            .iter()
            .map(|r| (r.title.as_str(), r.is_workspace, r.due))
            .collect();
        assert_eq!(titles, [("Loose", false, true), ("Ws", true, true)]);

        let members = list_members(
            root.to_string_lossy().into_owned(),
            root.join("ws").to_string_lossy().into_owned(),
            Some(1_000_000),
        );
        assert_eq!(members.len(), 1);
        assert!(!members[0].is_workspace);
    }

    #[test]
    fn sync_conflicts_surfaces_a_conflict_copy_and_is_quiet_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("loose.txt"), "# q\n\ta\n").unwrap();
        assert!(sync_conflicts(root.to_string_lossy().into_owned()).is_empty());

        let conflict = root.join("progress.sync-conflict-20260714-101112-ABCDEF7.json");
        std::fs::write(&conflict, "{}").unwrap();
        assert_eq!(
            sync_conflicts(root.to_string_lossy().into_owned()),
            vec![conflict.to_string_lossy().into_owned()]
        );
    }
}
