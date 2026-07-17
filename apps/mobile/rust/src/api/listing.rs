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
    /// A trace deck (`% trace:`): opens a walk (`WalkSession`), not a card
    /// review; the flag lets the picker route the row to the walk screen
    /// instead of a review session.
    pub is_trace: bool,
    /// The session depth remembered for this deck, else its fresh-session
    /// default. Deck rows only.
    pub last_depth: alix::depth::Depth,
    /// Finished and exam-passed, mirroring the web picker's mastered badge.
    /// Deck rows only.
    pub mastered: bool,
    /// Drilled and awaiting its AI exam. Deck rows only.
    pub exam_due: bool,
    /// The deck has an AI exam at all (a sourced fact deck, or a trace).
    /// Deck rows only.
    pub has_exam: bool,
    /// A `% requires:` prerequisite isn't finished, mirroring the web
    /// picker's lock icon. Deck rows only.
    pub locked: bool,
    /// The workspace's resolved picker icon file, mirroring the web
    /// picker's emblem. Workspace/folder rows only; `None` otherwise.
    pub icon: Option<String>,
    /// Nesting depth in the workspace's dependency tree. Member rows only;
    /// `0` otherwise.
    pub indent: u32,
    /// The branch-line prefix (`├─`/`└─`/`│`) for this row in the
    /// workspace's dependency tree. Member rows only; empty otherwise.
    pub tree: String,
}

impl From<alix::listing::DeckSummary> for DeckEntry {
    fn from(s: alix::listing::DeckSummary) -> Self {
        DeckEntry {
            title: s.title,
            path: s.path.to_string_lossy().into_owned(),
            is_workspace: s.is_workspace,
            due: s.due,
            is_trace: s.is_trace,
            last_depth: s.last_depth,
            mastered: s.mastered,
            exam_due: s.exam_due,
            has_exam: s.has_exam,
            locked: s.locked,
            icon: s.icon.map(|p| p.to_string_lossy().into_owned()),
            indent: s.indent as u32,
            tree: s.tree,
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

    const T0: u64 = 1_000_000;

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    /// A graduated-and-not-due `FsrsState`: the card reaches FSRS Review
    /// without a real drill session, so `exam_due`/`has_exam` compute
    /// without waiting out a real schedule (mirrors the lean listing fixture).
    fn graduated_not_due(now: u64) -> alix::store::FsrsState {
        alix::store::FsrsState {
            state: 2, // Review
            scheduled_days: 30,
            due_ms: now + 30 * 86_400_000,
            ..Default::default()
        }
    }

    #[test]
    fn mastered_and_exam_due_cross_the_boundary_for_a_sourced_deck() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("base.txt"), "% source: https://x\n# q\n\ta\n");
        write(&root.join("fresh.txt"), "# q\n\ta\n");

        let base_subject = alix::deck::Deck::load(root.join("base.txt")).unwrap().subject;
        let base_id = alix::deck::Deck::load(root.join("base.txt")).unwrap().cards[0].id();
        let store_path = alix::workspace::root_store_path(root);
        let mut store = alix::store::Store::open(&store_path).unwrap();
        store.get_or_insert(base_id, T0).recall = Some(graduated_not_due(T0));
        store.save().unwrap();

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0 + 1_000));
        let base = rows.iter().find(|r| r.title == "base").unwrap();
        assert!(base.exam_due, "graduated but not yet mastered");
        assert!(base.has_exam, "sourced deck has an AI exam");
        assert!(!base.mastered);
        let fresh = rows.iter().find(|r| r.title == "fresh").unwrap();
        assert!(!fresh.mastered);
        assert!(!fresh.has_exam);

        let mut store = alix::store::Store::open(&store_path).unwrap();
        store.set_deck_mastered(&base_subject, T0 + 1_000);
        store.save().unwrap();
        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0 + 1_000));
        let base = rows.iter().find(|r| r.title == "base").unwrap();
        assert!(base.mastered, "mastered once the exam is recorded passed");
        assert!(!base.exam_due, "no longer awaiting the exam");
    }

    #[test]
    fn locked_and_unlocked_members_cross_the_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("gate.txt"), "% source: https://x\n# q\n\ta\n");
        let ws = root.join("ws");
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "");
        write(&ws.join("child.txt"), "% requires: gate.txt\n# q2\n\tb\n");
        write(&ws.join("other.txt"), "# q\n\ta\n");

        let rows = list_members(
            root.to_string_lossy().into_owned(),
            ws.to_string_lossy().into_owned(),
            Some(T0),
        );
        let child = rows.iter().find(|r| r.title == "child").unwrap();
        assert!(child.locked, "gated by the unmastered gate.txt");
        let other = rows.iter().find(|r| r.title == "other").unwrap();
        assert!(!other.locked);
    }

    #[test]
    fn workspace_icon_crosses_to_some_and_a_plain_deck_row_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("ws")).unwrap();
        write(&root.join("ws/alix.toml"), "");
        std::fs::create_dir_all(root.join("ws/assets")).unwrap();
        write(&root.join("ws/assets/icon.svg"), "<svg/>");
        write(&root.join("ws/m.txt"), "# q\n\ta\n");
        write(&root.join("loose.txt"), "# q\n\ta\n");

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0));
        let ws_row = rows.iter().find(|r| r.is_workspace).expect("listed");
        assert_eq!(
            Some(
                root.join("ws/assets/icon.svg")
                    .to_string_lossy()
                    .into_owned()
            ),
            ws_row.icon
        );
        let loose = rows.iter().find(|r| !r.is_workspace).expect("listed");
        assert_eq!(None, loose.icon);
    }

    #[test]
    fn requires_chain_members_carry_tree_and_indent_and_a_loose_row_is_flat() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "");
        write(&ws.join("base.txt"), "# q\n\ta\n");
        write(&ws.join("mid.txt"), "% requires: base\n# q\n\ta\n");
        write(&ws.join("tip.txt"), "% requires: mid\n# q\n\ta\n");
        write(&ws.join("other.txt"), "# q\n\ta\n");

        let rows = list_members(
            root.to_string_lossy().into_owned(),
            ws.to_string_lossy().into_owned(),
            Some(T0),
        );
        let shape: Vec<(&str, u32, &str)> = rows
            .iter()
            .map(|r| (r.title.as_str(), r.indent, r.tree.as_str()))
            .collect();
        assert_eq!(
            vec![
                ("base", 0, ""),
                ("mid", 1, "└─ "),
                ("tip", 2, "   └─ "),
                ("other", 0, ""),
            ],
            shape
        );
    }

    #[test]
    fn last_depth_crosses_the_boundary_remembered_then_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# q\n\ta\n");

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0));
        let row = rows.iter().find(|r| r.title == "d").expect("listed");
        assert_eq!(alix::depth::Depth::default(), row.last_depth);

        let store_path = alix::workspace::root_store_path(root);
        let mut store = alix::store::Store::open(&store_path).unwrap();
        store.set_last_depth("d.txt", alix::depth::Depth::Reconstruct);
        store.save().unwrap();

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0));
        let row = rows.iter().find(|r| r.title == "d").expect("listed");
        assert_eq!(alix::depth::Depth::Reconstruct, row.last_depth);
    }
}
