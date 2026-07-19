use std::path::Path;

use anyhow::Result;

pub struct DeckEntry {
    pub title: String,
    pub path: String,
    pub is_workspace: bool,
    pub due: bool,
    pub can_recognize: bool,
    pub is_trace: bool,
    pub last_depth: alix::depth::Depth,
    pub mastered: bool,
    pub exam_due: bool,
    pub has_exam: bool,
    pub locked: bool,
    pub icon: Option<String>,
    pub indent: u32,
    pub tree: String,
    pub deadline: Option<Deadline>,
}

pub struct Deadline {
    pub date: String,
    pub days_left: i64,
    pub ready: u32,
    pub total: u32,
}

impl From<alix::listing::DeckDeadline> for Deadline {
    fn from(d: alix::listing::DeckDeadline) -> Self {
        Deadline {
            date: d.date.format("%Y-%m-%d").to_string(),
            days_left: d.days_left,
            ready: d.ready as u32,
            total: d.total as u32,
        }
    }
}

impl From<alix::listing::DeckSummary> for DeckEntry {
    fn from(s: alix::listing::DeckSummary) -> Self {
        DeckEntry {
            title: s.title,
            path: s.path.to_string_lossy().into_owned(),
            is_workspace: s.is_workspace,
            due: s.due,
            can_recognize: s.can_recognize,
            is_trace: s.is_trace,
            last_depth: s.last_depth,
            mastered: s.mastered,
            exam_due: s.exam_due,
            has_exam: s.has_exam,
            locked: s.locked,
            icon: s.icon.map(|p| p.to_string_lossy().into_owned()),
            indent: s.indent as u32,
            tree: s.tree,
            deadline: s.deadline.map(Deadline::from),
        }
    }
}

#[flutter_rust_bridge::frb(sync)]
pub fn workspace_deadline(root: String, dir: String, now_ms: Option<u64>) -> Option<Deadline> {
    let now = now_ms.unwrap_or_else(alix::time::now_ms);
    alix::listing::workspace_deadline(
        Path::new(&root),
        Path::new(&dir),
        &alix::config::ReviewConfig::default(),
        now,
    )
    .map(Deadline::from)
}

#[flutter_rust_bridge::frb(sync)]
pub fn set_workspace_deadline(dir: String, date: Option<String>) -> Result<()> {
    alix::workspace::set_deadline_str(Path::new(&dir), date.as_deref())
}

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

#[flutter_rust_bridge::frb(sync)]
pub fn sync_conflicts(root: String) -> Vec<String> {
    alix::listing::sync_conflicts_under(Path::new(&root))
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

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
        std::fs::write(
            root.join("loose.md"),
            "# Loose\n\n## q <!-- id: q1 -->\na\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("ws")).unwrap();
        std::fs::write(root.join("ws/alix.toml"), "title = \"Ws\"\n").unwrap();
        std::fs::write(root.join("ws/m.md"), "## q <!-- id: q1 -->\na\n").unwrap();

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
        std::fs::write(root.join("loose.md"), "## q\na\n").unwrap();
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

    fn graduated_not_due(now: u64) -> alix::store::FsrsState {
        alix::store::FsrsState {
            state: 2,
            scheduled_days: 30,
            due_ms: now + 30 * 86_400_000,
            ..Default::default()
        }
    }

    #[test]
    fn mastered_and_exam_due_cross_the_boundary_for_a_sourced_deck() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("base.md"),
            "---\nsource: \"https://x\"\n---\n## q <!-- id: q1 -->\na\n",
        );
        write(&root.join("fresh.md"), "## q\na\n");

        let base_subject = alix::deck::Deck::load(root.join("base.md")).unwrap().subject;
        let base_id = alix::deck::Deck::load(root.join("base.md")).unwrap().cards[0]
            .id()
            .expect("the fixture stamps its own id");
        let store_path = alix::workspace::root_store_path(root);
        let mut store = alix::store::Store::open(&store_path).unwrap();
        store.get_or_insert(&base_id, T0).recall = Some(graduated_not_due(T0));
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
        write(&root.join("gate.md"), "---\nsource: \"https://x\"\n---\n## q\na\n");
        let ws = root.join("ws");
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "");
        write(&ws.join("child.md"), "---\nrequires: gate\n---\n## q2\nb\n");
        write(&ws.join("other.md"), "## q\na\n");

        let rows = list_members(
            root.to_string_lossy().into_owned(),
            ws.to_string_lossy().into_owned(),
            Some(T0),
        );
        let child = rows.iter().find(|r| r.title == "child").unwrap();
        assert!(child.locked, "gated by the unmastered gate.md");
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
        write(&root.join("ws/m.md"), "## q\na\n");
        write(&root.join("loose.md"), "## q\na\n");

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
        write(&ws.join("base.md"), "## q\na\n");
        write(&ws.join("mid.md"), "---\nrequires: base\n---\n## q\na\n");
        write(&ws.join("tip.md"), "---\nrequires: mid\n---\n## q\na\n");
        write(&ws.join("other.md"), "## q\na\n");

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
    fn workspace_deadline_sets_moves_clears_and_lists_across_the_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "title = \"Ws\"\n");
        write(&ws.join("m.md"), "## q\na\n");
        let ws_s = ws.to_string_lossy().into_owned();
        let root_s = root.to_string_lossy().into_owned();

        assert!(workspace_deadline(root_s.clone(), ws_s.clone(), Some(T0)).is_none());

        let date = alix::time::local_date(T0) + chrono::Days::new(5);
        let date_s = date.format("%Y-%m-%d").to_string();
        set_workspace_deadline(ws_s.clone(), Some(date_s.clone())).unwrap();
        let text = std::fs::read_to_string(ws.join("alix.local.toml")).unwrap();
        assert!(text.contains(&format!("deadline = \"{date_s}\"")));
        let fetched = workspace_deadline(root_s.clone(), ws_s.clone(), Some(T0)).unwrap();
        assert_eq!((date_s.as_str(), 5, 0, 1), (
            fetched.date.as_str(),
            fetched.days_left,
            fetched.ready,
            fetched.total,
        ));
        let rows = list_root(root_s.clone(), Some(T0));
        let row = rows.iter().find(|r| r.is_workspace).unwrap();
        assert_eq!(Some(date_s.as_str()), row.deadline.as_ref().map(|d| d.date.as_str()));

        assert!(set_workspace_deadline(ws_s.clone(), Some("someday".into())).is_err());

        set_workspace_deadline(ws_s.clone(), None).unwrap();
        let text = std::fs::read_to_string(ws.join("alix.local.toml")).unwrap();
        assert!(!text.contains("deadline"));
        assert!(workspace_deadline(root_s.clone(), ws_s, Some(T0)).is_none());
        let rows = list_root(root_s, Some(T0));
        assert!(rows.iter().find(|r| r.is_workspace).unwrap().deadline.is_none());
    }

    #[test]
    fn last_depth_crosses_the_boundary_remembered_then_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.md"), "## q\na\n");

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0));
        let row = rows.iter().find(|r| r.title == "d").expect("listed");
        assert_eq!(alix::depth::Depth::default(), row.last_depth);

        let store_path = alix::workspace::root_store_path(root);
        let mut store = alix::store::Store::open(&store_path).unwrap();
        store.set_last_depth("d.md", alix::depth::Depth::Reconstruct);
        store.save().unwrap();

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0));
        let row = rows.iter().find(|r| r.title == "d").expect("listed");
        assert_eq!(alix::depth::Depth::Reconstruct, row.last_depth);
    }
}
