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
    pub progress_error: bool,
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
            progress_error: s.progress_error,
            icon: s.icon.map(|p| p.to_string_lossy().into_owned()),
            indent: s.indent as u32,
            tree: s.tree,
            deadline: s.deadline.map(Deadline::from),
        }
    }
}

pub struct OpenProfile {
    pub lib_ms: u64,
    pub candidates_classified: u64,
    pub manifest_reads: u64,
    pub decks_loaded: u64,
    pub prerequisite_loads: u64,
    pub id_scans: u64,
    pub diagram_geometry_reads: u64,
    pub store_documents_read: u64,
    pub augment_documents_read: u64,
    pub canonicalize_calls: u64,
    pub sidecar_reads: u64,
}

impl OpenProfile {
    fn new(lib_ms: u64, counts: alix::profile::Counts) -> Self {
        OpenProfile {
            lib_ms,
            candidates_classified: counts.candidates_classified,
            manifest_reads: counts.manifest_reads,
            decks_loaded: counts.decks_loaded,
            prerequisite_loads: counts.prerequisite_loads,
            id_scans: counts.id_scans,
            diagram_geometry_reads: counts.diagram_geometry_reads,
            store_documents_read: counts.store_documents_read,
            augment_documents_read: counts.augment_documents_read,
            canonicalize_calls: counts.canonicalize_calls,
            sidecar_reads: counts.sidecar_reads,
        }
    }
}

pub struct RootScreen {
    pub entries: Vec<DeckEntry>,
    pub profile: Option<OpenProfile>,
}

pub struct MembersScreen {
    pub entries: Vec<DeckEntry>,
    pub deadline: Option<Deadline>,
    pub profile: Option<OpenProfile>,
}

fn profiled<T>(profile: bool, f: impl FnOnce() -> T) -> (T, Option<OpenProfile>) {
    if !profile {
        return (f(), None);
    }
    let started = std::time::Instant::now();
    let (value, counts) = alix::profile::collect(f);
    let lib_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    (value, Some(OpenProfile::new(lib_ms, counts)))
}

#[flutter_rust_bridge::frb(sync)]
pub fn set_workspace_deadline(dir: String, date: Option<String>) -> Result<()> {
    alix::workspace::set_deadline_str(Path::new(&dir), date.as_deref())
}

#[flutter_rust_bridge::frb(sync)]
pub fn list_root(root: String, now_ms: Option<u64>, profile: bool) -> RootScreen {
    let now = now_ms.unwrap_or_else(alix::time::now_ms);
    let (entries, profile) = profiled(profile, || {
        alix::listing::list_root(
            Path::new(&root),
            &alix::config::ReviewConfig::default(),
            now,
        )
        .into_iter()
        .map(DeckEntry::from)
        .collect()
    });
    RootScreen { entries, profile }
}

#[flutter_rust_bridge::frb(sync)]
pub fn list_members(
    root: String,
    dir: String,
    now_ms: Option<u64>,
    profile: bool,
) -> MembersScreen {
    let now = now_ms.unwrap_or_else(alix::time::now_ms);
    let review = alix::config::ReviewConfig::default();
    let ((entries, deadline), profile) = profiled(profile, || {
        let listing =
            alix::listing::list_members(Path::new(&root), Path::new(&dir), &review, now);
        let entries: Vec<DeckEntry> = listing.rows.into_iter().map(DeckEntry::from).collect();
        (entries, listing.deadline.map(Deadline::from))
    });
    MembersScreen {
        entries,
        deadline,
        profile,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_a_root_with_a_workspace_and_a_loose_deck() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_deck(
            root.join("loose.md"),
            "---\ntitle: Loose\n---\n## q\na\n<!-- id: card-q1 -->\n",
        );
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        std::fs::write(root.join("ws/alix.toml"), "title = \"Ws\"\n").unwrap();
        write_deck(root.join("ws/decks/m.md"), "## q\na\n<!-- id: card-q1 -->\n");

        let rows = list_root(root.to_string_lossy().into_owned(), Some(1_000_000), false).entries;
        let titles: Vec<(&str, bool, bool)> = rows
            .iter()
            .map(|r| (r.title.as_str(), r.is_workspace, r.due))
            .collect();
        assert_eq!(titles, [("Loose", false, true), ("Ws", true, true)]);

        let members = list_members(root.to_string_lossy().into_owned(),
            root.join("ws").to_string_lossy().into_owned(),
            Some(1_000_000), false).entries;
        assert_eq!(members.len(), 1);
        assert!(!members[0].is_workspace);
    }

    const T0: u64 = 1_000_000;

    #[test]
    fn law_one_open_is_one_pass_over_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        write(&ws.join("alix.toml"), "title = \"Ws\"\n");
        let members = 6u64;
        for i in 0..members {
            let requires = if i == 0 {
                String::new()
            } else {
                format!("---\nrequires: m{}.md\n---\n", i - 1)
            };
            write_deck(ws.join(format!("decks/m{i}.md")), &format!("{requires}## q{i}\na\n"));
        }
        std::fs::write(ws.join("decks/draft.md"), "## q\na\n").unwrap();
        let candidates = members + 1;
        let root_s = root.to_string_lossy().into_owned();
        let ws_s = ws.to_string_lossy().into_owned();

        let screen = list_members(root_s.clone(), ws_s, Some(T0), true);
        let profile = screen.profile.expect("profiled");
        assert_eq!(members as usize, screen.entries.len(), "every member listed");
        assert_eq!(
            (members, 1, candidates, 0),
            (
                profile.decks_loaded,
                profile.manifest_reads,
                profile.candidates_classified,
                profile.prerequisite_loads,
            ),
            "drill-in: (decks_loaded, manifest_reads, candidates_classified, prerequisite_loads)"
        );

        let screen = list_root(root_s, Some(T0), true);
        let profile = screen.profile.expect("profiled");
        assert_eq!(1, screen.entries.len(), "the root holds one workspace row");
        assert_eq!(
            (members, 1, candidates),
            (
                profile.decks_loaded,
                profile.manifest_reads,
                profile.candidates_classified,
            ),
            "root screen: (decks_loaded, manifest_reads, candidates_classified)"
        );
    }

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    fn write_deck(path: impl AsRef<Path>, text: &str) {
        let path = path.as_ref();
        write(path, text);
        alix::stamp::stamp_deck(path).unwrap();
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
        write_deck(
            root.join("base.md"),
            "---\nsource: \"https://x\"\n---\n## q\na\n<!-- id: card-q1 -->\n",
        );
        write_deck(root.join("fresh.md"), "## q\na\n");

        let base_deck_id = alix::deck::Deck::load(root.join("base.md"))
            .unwrap()
            .deck_token
            .unwrap();
        let base_id = alix::deck::Deck::load(root.join("base.md")).unwrap().cards[0]
            .id()
            .expect("the fixture stamps its own id");
        let store_path = alix::workspace::root_store_path(root);
        let mut store = alix::state::open_store(&root.join("base.md"), &store_path).unwrap();
        store.get_or_insert(&base_id).recall = Some(graduated_not_due(T0));
        store.save().unwrap();

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0 + 1_000), false).entries;
        let base = rows.iter().find(|r| r.title == "base").unwrap();
        assert!(base.exam_due, "graduated but not yet mastered");
        assert!(base.has_exam, "sourced deck has an AI exam");
        assert!(!base.mastered);
        let fresh = rows.iter().find(|r| r.title == "fresh").unwrap();
        assert!(!fresh.mastered);
        assert!(!fresh.has_exam);

        let mut store = alix::state::open_store(&root.join("base.md"), &store_path).unwrap();
        store.set_deck_mastered(&base_deck_id, T0 + 1_000);
        store.save().unwrap();
        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0 + 1_000), false).entries;
        let base = rows.iter().find(|r| r.title == "base").unwrap();
        assert!(base.mastered, "mastered once the exam is recorded passed");
        assert!(!base.exam_due, "no longer awaiting the exam");
    }

    #[test]
    fn locked_and_unlocked_members_cross_the_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_deck(
            root.join("gate.md"),
            "---\nsource: \"https://x\"\n---\n## q\na\n",
        );
        let ws = root.join("ws");
        let members = ws.join("decks");
        std::fs::create_dir_all(&members).unwrap();
        write(&ws.join("alix.toml"), "");
        write_deck(
            members.join("child.md"),
            "---\nrequires: gate\n---\n## q2\nb\n",
        );
        write_deck(members.join("other.md"), "## q\na\n");

        let rows = list_members(root.to_string_lossy().into_owned(),
            ws.to_string_lossy().into_owned(),
            Some(T0), false).entries;
        let child = rows.iter().find(|r| r.title == "child").unwrap();
        assert!(child.locked, "gated by the unmastered gate.md");
        let other = rows.iter().find(|r| r.title == "other").unwrap();
        assert!(!other.locked);
    }

    #[test]
    fn workspace_icon_crosses_to_some_and_a_plain_deck_row_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(&root.join("ws/alix.toml"), "");
        std::fs::create_dir_all(root.join("ws/assets")).unwrap();
        write(&root.join("ws/assets/icon.svg"), "<svg/>");
        write_deck(root.join("ws/decks/m.md"), "## q\na\n");
        write_deck(root.join("loose.md"), "## q\na\n");

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0), false).entries;
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
        let members = ws.join("decks");
        std::fs::create_dir_all(&members).unwrap();
        write(&ws.join("alix.toml"), "");
        write_deck(members.join("base.md"), "## q\na\n");
        write_deck(
            members.join("mid.md"),
            "---\nrequires: base\n---\n## q\na\n",
        );
        write_deck(members.join("tip.md"), "---\nrequires: mid\n---\n## q\na\n");
        write_deck(members.join("other.md"), "## q\na\n");

        let rows = list_members(root.to_string_lossy().into_owned(),
            ws.to_string_lossy().into_owned(),
            Some(T0), false).entries;
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
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        write(&ws.join("alix.toml"), "title = \"Ws\"\n");
        write_deck(ws.join("decks/m.md"), "## q\na\n");
        let ws_s = ws.to_string_lossy().into_owned();
        let root_s = root.to_string_lossy().into_owned();

        assert!(list_members(root_s.clone(), ws_s.clone(), Some(T0), false).deadline.is_none());

        let date = alix::time::local_date(T0) + chrono::Days::new(5);
        let date_s = date.format("%Y-%m-%d").to_string();
        set_workspace_deadline(ws_s.clone(), Some(date_s.clone())).unwrap();
        let text = std::fs::read_to_string(ws.join("alix.local.toml")).unwrap();
        assert!(text.contains(&format!("deadline = \"{date_s}\"")));
        let fetched = list_members(root_s.clone(), ws_s.clone(), Some(T0), false).deadline.unwrap();
        assert_eq!(
            (date_s.as_str(), 5, 0, 1),
            (
                fetched.date.as_str(),
                fetched.days_left,
                fetched.ready,
                fetched.total,
            )
        );
        let rows = list_root(root_s.clone(), Some(T0), false).entries;
        let row = rows.iter().find(|r| r.is_workspace).unwrap();
        assert_eq!(
            Some(date_s.as_str()),
            row.deadline.as_ref().map(|d| d.date.as_str())
        );

        assert!(set_workspace_deadline(ws_s.clone(), Some("someday".into())).is_err());

        set_workspace_deadline(ws_s.clone(), None).unwrap();
        let text = std::fs::read_to_string(ws.join("alix.local.toml")).unwrap();
        assert!(!text.contains("deadline"));
        assert!(list_members(root_s.clone(), ws_s, Some(T0), false).deadline.is_none());
        let rows = list_root(root_s, Some(T0), false).entries;
        assert!(
            rows.iter()
                .find(|r| r.is_workspace)
                .unwrap()
                .deadline
                .is_none()
        );
    }

    #[test]
    fn last_depth_crosses_the_boundary_remembered_then_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_deck(root.join("d.md"), "## q\na\n");

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0), false).entries;
        let row = rows.iter().find(|r| r.title == "d").expect("listed");
        assert_eq!(alix::depth::Depth::default(), row.last_depth);

        let deck_id = alix::deck::Deck::load(root.join("d.md"))
            .unwrap()
            .deck_token
            .unwrap();
        let store_path = alix::workspace::root_store_path(root);
        let mut store = alix::state::open_store(&root.join("d.md"), &store_path).unwrap();
        store.set_last_depth(&deck_id, alix::depth::Depth::Reconstruct);
        store.save().unwrap();

        let rows = list_root(root.to_string_lossy().into_owned(), Some(T0), false).entries;
        let row = rows.iter().find(|r| r.title == "d").expect("listed");
        assert_eq!(alix::depth::Depth::Reconstruct, row.last_depth);
    }
}
