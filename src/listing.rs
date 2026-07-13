//! A minimal deck lister for folder-picking clients (the frb mobile app):
//! what is in a decks folder, with a title and a due-now signal per entry.
//! The core sibling of the gated picker's catalog, deliberately without its
//! recency ordering, badges, and dependency locks; scan rules and the due
//! semantics mirror `picker::dir_candidates` / `picker::deck_status` so both
//! surfaces agree on what a folder contains and what is launchable.
use std::path::{Path, PathBuf};

use crate::{
    config::ReviewConfig, deck::Deck, depth::Depth, scheduler::Fsrs, session, store::Store,
    workspace,
};

/// One row of a folder listing: a loose deck, or a drillable folder of decks
/// (a workspace, or a plain folder holding `*.txt`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckSummary {
    /// The deck's `% title:` / the workspace manifest title, else the file
    /// stem or folder name.
    pub title: String,
    pub path: PathBuf,
    /// Drillable: list its members with [`list_members`].
    pub is_workspace: bool,
    /// Anything to do right now at any depth (new, due, or a due virtual
    /// card), against the store this entry actually reviews into. Matches the
    /// web picker's launchable signal.
    pub due: bool,
}

/// Lists a decks root: workspaces and plain deck folders as drillable
/// entries, loose `*.txt` files as decks, name-sorted, dot-names skipped.
/// A root that is itself a workspace collapses to that one entry. Unreadable
/// entries degrade (stem title, `due: false`); they never error.
pub fn list_root(root: &Path, review: &ReviewConfig, now_ms: u64) -> Vec<DeckSummary> {
    if workspace::is_workspace(root) {
        return vec![folder_summary(root, root, review, now_ms)];
    }
    let root_store = Store::open(workspace::root_store_path(root)).ok();
    let mut names: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    names.sort();
    let mut out = Vec::new();
    for path in names {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() && workspace::has_decks(&path) {
            out.push(folder_summary(root, &path, review, now_ms));
        } else if path.is_file() && path.extension().is_some_and(|e| e == "txt") {
            out.push(deck_summary(&path, root_store.as_ref(), review, now_ms));
        }
    }
    out
}

/// Lists the decks inside one drillable folder of `root`. Members review
/// into the folder's own store when it is a workspace (`alix.toml`), else
/// into the root's shared store — the same routing `assemble::store_for`
/// applies when one of them is opened.
pub fn list_members(
    root: &Path,
    dir: &Path,
    review: &ReviewConfig,
    now_ms: u64,
) -> Vec<DeckSummary> {
    let store = member_store(root, dir);
    workspace::Workspace::load(dir)
        .map(|ws| {
            ws.members
                .iter()
                .map(|m| deck_summary(m, store.as_ref(), review, now_ms))
                .collect()
        })
        .unwrap_or_default()
}

/// The store a folder's members review into: the workspace's own when the
/// folder has a manifest, else the root's shared store.
fn member_store(root: &Path, dir: &Path) -> Option<Store> {
    let path = if workspace::is_workspace(dir) {
        workspace::store_path(dir)
    } else {
        workspace::root_store_path(root)
    };
    Store::open(path).ok()
}

fn folder_summary(root: &Path, dir: &Path, review: &ReviewConfig, now_ms: u64) -> DeckSummary {
    let title = workspace::Workspace::load(dir)
        .map(|ws| ws.display_name())
        .unwrap_or_else(|_| {
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let due = list_members(root, dir, review, now_ms)
        .iter()
        .any(|m| m.due);
    DeckSummary {
        title,
        path: dir.to_path_buf(),
        is_workspace: true,
        due,
    }
}

fn deck_summary(
    path: &Path,
    store: Option<&Store>,
    review: &ReviewConfig,
    now_ms: u64,
) -> DeckSummary {
    let stem = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (title, due) = match (Deck::load(path), store) {
        (Ok(deck), Some(store)) => {
            let due = deck_due(&deck, store, review, now_ms);
            (deck.display_name(), due)
        }
        (Ok(deck), None) => (deck.display_name(), false),
        (Err(_), _) => (stem.clone(), false),
    };
    DeckSummary {
        title: if title.is_empty() { stem } else { title },
        path: path.to_path_buf(),
        is_workspace: false,
        due,
    }
}

/// Anything to launch right now at any depth: the web picker's aggregate
/// (`picker::deck_status`) minus its trace and exam special cases, which the
/// mobile client does not open yet.
fn deck_due(deck: &Deck, store: &Store, review: &ReviewConfig, now_ms: u64) -> bool {
    let scheduler = Fsrs::new(review.retention);
    let retire = review.retire_after_days;
    [Depth::Recognize, Depth::Recall, Depth::Reconstruct]
        .into_iter()
        .any(|depth| session::has_reviewable(&deck.cards, store, &scheduler, depth, now_ms, retire))
        || session::has_reviewable_virtual(store, &deck.subject, &scheduler, now_ms, retire)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::{Grade, Scheduler};

    const T0: u64 = 1_000_000;
    /// Far past any first-Good learning interval, so everything is due again.
    const MUCH_LATER: u64 = T0 + 30 * 86_400_000;

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    /// Marks the deck's single card fully settled at T0: recognized, and
    /// Pass-graded at both scheduled depths, so nothing is due until the
    /// schedules elapse.
    fn settle(store_path: &Path, deck_path: &Path) {
        let mut store = Store::open(store_path).unwrap();
        let deck = Deck::load(deck_path).unwrap();
        let scheduler = Fsrs::default();
        for card in &deck.cards {
            let state = store.get_or_insert(card.id(), T0);
            state.recognized_ms = Some(T0);
            scheduler.apply(state, Depth::Recall, Grade::Pass, T0, false);
            scheduler.apply(state, Depth::Reconstruct, Grade::Pass, T0, false);
        }
        store.save().unwrap();
    }

    #[test]
    fn lists_titles_and_kinds_and_skips_dot_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("b-loose.txt"), "% title: Loose Deck\n# q\n\ta\n");
        std::fs::create_dir(root.join("a-ws")).unwrap();
        write(&root.join("a-ws/alix.toml"), "title = \"My Workspace\"\n");
        write(&root.join("a-ws/m.txt"), "# q\n\ta\n");
        std::fs::create_dir(root.join("c-plain")).unwrap();
        write(&root.join("c-plain/d.txt"), "# q\n\ta\n");
        std::fs::create_dir(root.join(".hidden")).unwrap();
        write(&root.join(".hidden/x.txt"), "# q\n\ta\n");
        write(&root.join("notes.md"), "not a deck");

        let rows = list_root(root, &ReviewConfig::default(), T0);
        let names: Vec<(&str, bool)> = rows
            .iter()
            .map(|r| (r.title.as_str(), r.is_workspace))
            .collect();
        assert_eq!(
            names,
            [
                ("My Workspace", true),
                ("Loose Deck", false),
                ("c-plain", true),
            ]
        );
    }

    #[test]
    fn a_root_that_is_a_workspace_collapses_to_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("alix.toml"), "title = \"The Root\"\n");
        write(&root.join("a.txt"), "# q\n\ta\n");
        let rows = list_root(root, &ReviewConfig::default(), T0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "The Root");
        assert!(rows[0].is_workspace);
        assert!(rows[0].due, "a fresh deck has new cards");
        let members = list_members(root, root, &ReviewConfig::default(), T0);
        assert_eq!(members.len(), 1);
    }

    #[test]
    fn due_reads_each_entrys_own_store_and_the_injected_clock() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("loose.txt"), "# q\n\ta\n");
        std::fs::create_dir(root.join("ws")).unwrap();
        write(&root.join("ws/alix.toml"), "");
        write(&root.join("ws/m.txt"), "# q\n\ta\n");

        // Settle the workspace member in the WORKSPACE store and the loose
        // deck in the ROOT store; each entry must read its own.
        settle(
            &workspace::store_path(&root.join("ws")),
            &root.join("ws/m.txt"),
        );
        settle(&workspace::root_store_path(root), &root.join("loose.txt"));

        let just_after = list_root(root, &ReviewConfig::default(), T0 + 1_000);
        assert!(
            just_after.iter().all(|r| !r.due),
            "everything settled: {just_after:?}"
        );
        let later = list_root(root, &ReviewConfig::default(), MUCH_LATER);
        assert!(later.iter().all(|r| r.due), "schedules elapsed: {later:?}");
    }

    #[test]
    fn unreadable_entries_degrade_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A deck file whose content fails to parse (a cloze with no holes):
        // listed with its stem, just never due.
        write(
            &root.join("broken.txt"),
            "# q\n\t% reveal: cloze\n\tno holes here\n",
        );
        write(&root.join("ok.txt"), "# q\n\ta\n");
        let rows = list_root(root, &ReviewConfig::default(), T0);
        let broken = rows.iter().find(|r| r.title == "broken").expect("listed");
        assert!(!broken.due);
        assert!(rows.iter().any(|r| r.title == "ok" && r.due));
        assert_eq!(
            list_root(&root.join("nowhere"), &ReviewConfig::default(), T0),
            []
        );
    }
}
