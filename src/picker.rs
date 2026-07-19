//! Frontend-agnostic deck catalog for the web deck picker: the list of decks
//! and workspaces to offer ([`catalog`]), recency ordering, and workspace
//! readiness ([`workspace_readiness`]). A deck's store-derived
//! badge/lock/gating ([`deck_status`]) and the workspace dependency-forest
//! layout ([`member_parents`] / [`dependency_forest`]) now live in
//! [`crate::listing`] (so the lean mobile build can use them too) and are
//! re-exported here for existing callers.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

// Only the readiness test uses `DeckState` since `workspace_readiness` moved
// to `listing`; a top-level import would be an unused-import warning outside
// tests.
#[cfg(test)]
use crate::deck::DeckState;
pub use crate::listing::{DeckStatus, deck_status, dependency_forest, member_parents};
use crate::{recent::RecentDecks, store::Store, title, workspace};

// ---- deck candidates ----------------------------------------------------

/// A selectable deck or workspace, before it becomes a picker `Item`.
struct Candidate {
    path: PathBuf,
    /// File name (deck) or folder name (workspace) — the stable selection key.
    name: String,
    /// When last reviewed, if it is a recent entry.
    last_used_ms: Option<u64>,
    /// `true` for a drillable folder (a workspace if it has an `alix.toml`, else
    /// a plain folder), `false` for a single deck file.
    is_workspace: bool,
}

/// Every `*.md` deck and every workspace folder directly in `decks_dir`,
/// sorted by name. Conventional non-deck names (`README.*`, `LICENSE.*`,
/// any-case) and prose `.md` files (no card, no frontmatter) are excluded
/// ([`workspace::file_is_deck`]).
fn dir_candidates(decks_dir: &Path) -> Vec<Candidate> {
    // A served root that is itself a workspace lists as that one workspace, so
    // `alix <workspace-dir>` opens the picker drilled into it — members keep
    // their dependency tree instead of flattening into loose decks.
    if workspace::is_workspace(decks_dir) {
        return vec![Candidate {
            name: file_name(decks_dir),
            path: decks_dir.to_path_buf(),
            last_used_ms: None,
            is_workspace: true,
        }];
    }
    let mut cands: Vec<Candidate> = match std::fs::read_dir(decks_dir) {
        Ok(read_dir) => read_dir
            .filter_map(|r| r.ok().map(|d| d.path()))
            // Hidden by convention, same rule `share::stays_home` applies: in
            // particular, `alix generate`'s staging dir for a workspace build
            // (`.<name>.building`) is deliberately kept around on a merge
            // conflict, holding real `.md` decks — without this filter it
            // would show up here as a bogus workspace. An explicitly named
            // dot-dir (`alix share .foo`, `alix stats .foo`) still works —
            // this only filters the directory *scan*.
            .filter(|path| !file_name(path).starts_with('.'))
            .filter_map(|path| {
                let is_deck = path.is_file()
                    && path.extension().is_some_and(|e| e == "md")
                    && !workspace::is_conventional_non_deck(&file_name(&path))
                    && workspace::file_is_deck(&path);
                if is_deck {
                    Some((path, false))
                } else if workspace::has_decks(&path) {
                    Some((path, true))
                } else {
                    None
                }
            })
            .map(|(path, is_workspace)| Candidate {
                name: file_name(&path),
                path,
                last_used_ms: None,
                is_workspace,
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    cands.sort_by(|a, b| a.name.cmp(&b.name));
    cands
}

/// Builds the candidate list: existing recent entries first (recency order),
/// then every other deck/workspace in `decks_dir`, sorted by name.
fn build_candidates(decks_dir: &Path, recent: &RecentDecks) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for entry in recent.entries() {
        let is_workspace = workspace::has_decks(&entry.path);
        if entry.path.is_file() || is_workspace {
            out.push(Candidate {
                name: file_name(&entry.path),
                path: entry.path.clone(),
                last_used_ms: Some(entry.last_used_ms),
                is_workspace,
            });
            seen.insert(entry.path.clone());
        }
    }

    for candidate in dir_candidates(decks_dir) {
        if !seen.contains(&candidate.path) {
            out.push(candidate);
        }
    }
    out
}

// The deadline-readiness rule moved to `listing` (the deck_status precedent:
// the lean mobile build needs it too); re-exported here for the web picker.
pub use crate::listing::{WorkspaceReadiness, workspace_readiness};

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A "2h ago"-style label for the last time progress was made in `folder`'s own
/// workspace store (an actual review, not merely opening it), or `None` if it has
/// none yet. Shared with the web picker, which shows the same time on workspace
/// rows.
pub fn workspace_last_progress(folder: &Path) -> Option<String> {
    let ts = Store::open(workspace::store_path(folder))
        .ok()?
        .last_review_ms()?;
    let now = crate::time::now_ms();
    Some(if now > ts {
        format!("{} ago", crate::time::humanize_ms(now - ts))
    } else {
        "just now".to_string()
    })
}

/// A deck name without its `.md` extension, for matching.
fn stem(name: &str) -> String {
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

/// A dim location hint for entries that don't live directly in the decks dir
/// (a recent deck/workspace from elsewhere, or a member nested in a workspace):
/// the parent directory, abbreviated with `~`. `None` for entries in the decks
/// dir root, so the common listing stays clean and only the odd ones out —
/// which is where two same-named entries get told apart — show a path.
fn location_hint(path: &Path, decks_dir: &Path) -> Option<String> {
    let parent = path.parent()?;
    if parent == decks_dir {
        return None;
    }
    Some(abbreviate_home(parent))
}

/// `path` with the home directory replaced by `~`, else as-is.
fn abbreviate_home(path: &Path) -> String {
    directories::BaseDirs::new()
        .and_then(|dirs| {
            path.strip_prefix(dirs.home_dir()).ok().map(|rest| {
                if rest.as_os_str().is_empty() {
                    "~".to_string()
                } else {
                    format!("~/{}", rest.display())
                }
            })
        })
        .unwrap_or_else(|| path.display().to_string())
}

// ---- public entry points ------------------------------------------------

/// One entry offered by [`catalog`]: a deck or a workspace. `name` is the
/// stable selection key (file/folder name, or `<workspace>/<file>` for a
/// member); `label` is the display title (`% title:`, else the name without
/// `.txt`, else the workspace's folder name). A workspace entry carries its
/// member decks in `members` (each a deck entry with a qualified `name`); decks
/// have none.
pub struct DeckEntry {
    pub name: String,
    pub label: String,
    pub path: PathBuf,
    pub last_used_ms: Option<u64>,
    pub is_workspace: bool,
    /// A workspace's one-line `description` (its learning goal), shown dim under
    /// the row. `None` for decks and folders.
    pub description: Option<String>,
    pub members: Vec<DeckEntry>,
    /// Dim location hint (parent dir, `~`-abbreviated) when not in the decks
    /// dir.
    pub path_hint: Option<String>,
    /// A workspace's resolved picker icon file, or `None`. Members and decks
    /// never carry one.
    pub icon: Option<PathBuf>,
}

/// The catalog the picker shows, as plain data: recent entries first (recency
/// order), then every other deck and workspace in `decks_dir`.
/// Frontend-agnostic, so the web deck-selection screen (and any thin client
/// over the same JSON API) can present the same list from the same data.
pub fn catalog(decks_dir: &Path, recent: &RecentDecks) -> Vec<DeckEntry> {
    build_candidates(decks_dir, recent)
        .into_iter()
        .map(|c| {
            if c.is_workspace {
                let (label, description, members, icon) = match workspace::Workspace::load(&c.path)
                {
                    Ok(ws) => {
                        let members = ws
                            .members
                            .iter()
                            .map(|m| {
                                let file = file_name(m);
                                DeckEntry {
                                    // Qualified key so members never collide with
                                    // top-level decks in the resolution map.
                                    name: format!("{}/{}", c.name, file),
                                    label: deck_label(m).unwrap_or_else(|| stem(&file)),
                                    path: m.clone(),
                                    last_used_ms: None,
                                    is_workspace: false,
                                    description: None,
                                    members: Vec::new(),
                                    path_hint: None, // shown only in the drill-in
                                    icon: None,
                                }
                            })
                            .collect();
                        (ws.display_name(), ws.description, members, ws.icon)
                    }
                    Err(_) => (c.name.clone(), None, Vec::new(), None),
                };
                DeckEntry {
                    path_hint: location_hint(&c.path, decks_dir),
                    name: c.name,
                    label,
                    path: c.path,
                    last_used_ms: c.last_used_ms,
                    is_workspace: true,
                    description,
                    members,
                    icon,
                }
            } else {
                DeckEntry {
                    label: deck_label(&c.path).unwrap_or_else(|| stem(&c.name)),
                    path_hint: location_hint(&c.path, decks_dir),
                    name: c.name,
                    path: c.path,
                    last_used_ms: c.last_used_ms,
                    is_workspace: false,
                    description: None,
                    members: Vec::new(),
                    icon: None,
                }
            }
        })
        .collect()
}

/// A deck's display label: its `# H1` title, else — for a trace — a condensed
/// form of its `trace:` path-question (an `explore` trace's is already short,
/// a `--build`/hand-written one gets cut to a label-sized head). `None` when
/// it declares neither (or does not parse), so the caller falls back to the
/// file stem. Read-only: listing never stamps.
fn deck_label(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let deck = crate::l1::parse_l1("deck.md", &text).ok()?;
    deck.title
        .or_else(|| deck.frontmatter.trace.map(|t| title::condense(&t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_candidates_orders_recent_first_then_alpha() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["zeta.md", "alpha.md", "mid.md"] {
            std::fs::write(dir.path().join(n), "## f\nb\n").unwrap();
        }
        let recent_path = dir.path().join("recent.json");
        let mut recent = RecentDecks::load(&recent_path);
        recent.record(&[dir.path().join("mid.md")], 1000);

        let cands = build_candidates(dir.path(), &recent);
        let names: Vec<&str> = cands.iter().map(|c| c.name.as_str()).collect();
        // Recent (mid) first, then the rest alphabetically.
        assert_eq!(vec!["mid.md", "alpha.md", "zeta.md"], names);
        assert!(cands[0].last_used_ms.is_some());
        assert!(cands[1].last_used_ms.is_none());
    }

    #[test]
    fn a_workspace_root_lists_as_that_single_workspace() {
        // Serving a workspace dir directly (`alix <workspace>`) must present
        // the workspace itself — drill-in intact — not its members flattened
        // into loose decks.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "title = \"T\"\n").unwrap();
        std::fs::write(dir.path().join("m.md"), "## f\nb\n").unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));
        let entries = catalog(dir.path(), &recent);
        assert_eq!(1, entries.len());
        assert!(entries[0].is_workspace);
        assert_eq!(1, entries[0].members.len());
    }

    #[test]
    fn catalog_mirrors_candidate_order_and_paths() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["zeta.md", "alpha.md"] {
            std::fs::write(dir.path().join(n), "## f\nb\n").unwrap();
        }
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[dir.path().join("zeta.md")], 1000);

        let entries = catalog(dir.path(), &recent);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(vec!["zeta.md", "alpha.md"], names); // recent first
        assert_eq!(dir.path().join("zeta.md"), entries[0].path);
        assert!(entries[0].last_used_ms.is_some());
    }

    #[test]
    fn deck_label_condenses_a_trace_path_question_instead_of_the_slug() {
        let dir = tempfile::tempdir().unwrap();
        // A trace declares its name in `% trace:`, not `% title:` — the label
        // comes from a condensed form of it, never the file stem.
        let trace = dir.path().join("06-how-a-digest-becomes-verified.md");
        std::fs::write(
            &trace,
            "---\ntrace: \"how a transaction digest becomes verified effects and events: \
             fetch the checkpoint, derive the committee, then verify\"\n---\n",
        )
        .unwrap();
        assert_eq!(
            Some("How a Transaction Digest Becomes Verified Effects and Events".to_string()),
            deck_label(&trace),
        );

        // An explicit `% title:` still wins outright.
        let titled = dir.path().join("01-the-domain-model.md");
        std::fs::write(&titled, "# The Domain Model\n## f\nb\n").unwrap();
        assert_eq!(Some("The Domain Model".to_string()), deck_label(&titled));

        // A plain deck with neither yields None (the caller falls back to stem).
        let plain = dir.path().join("plain.md");
        std::fs::write(&plain, "## f\nb\n").unwrap();
        assert_eq!(None, deck_label(&plain));
    }

    #[test]
    fn location_hint_only_for_entries_outside_the_decks_dir() {
        let home = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf();
        let decks = home.join("decks");
        // In the decks dir root → no hint (keeps the common listing clean).
        assert_eq!(None, location_hint(&decks.join("foo.md"), &decks));
        assert_eq!(None, location_hint(&decks.join("english"), &decks));
        // Elsewhere → the parent dir, home abbreviated to `~`.
        assert_eq!(
            Some("~/other".to_string()),
            location_hint(&home.join("other").join("x.md"), &decks)
        );
        assert_eq!(
            Some("/tmp".to_string()),
            location_hint(Path::new("/tmp/x.md"), &decks)
        );
    }

    #[test]
    fn catalog_surfaces_workspace_with_qualified_members() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("english");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.md"), "## a\nb\n").unwrap();
        std::fs::write(ws.join("b.md"), "## c\nd\n").unwrap();
        std::fs::write(ws.join(workspace::MANIFEST), "title = \"English\"\n").unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));

        let entries = catalog(dir.path(), &recent);
        let w = entries
            .iter()
            .find(|e| e.is_workspace)
            .expect("workspace entry");
        assert_eq!("english", w.name); // folder name is the selection key
        assert_eq!("English", w.label); // manifest title is the display name
        let members: Vec<&str> = w.members.iter().map(|m| m.name.as_str()).collect();
        // Members carry qualified keys so they never collide with top-level decks.
        assert_eq!(vec!["english/a.md", "english/b.md"], members);
    }

    #[test]
    fn build_candidates_skips_missing_recent_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.md"), "## f\nb\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[dir.path().join("deleted.md")], 1000);

        let cands = build_candidates(dir.path(), &recent);
        let names: Vec<&str> = cands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(vec!["real.md"], names);
    }

    #[test]
    fn a_dot_prefixed_folder_is_invisible_to_the_scan() {
        // `alix generate`'s staging dir (`.<name>.building`) is deliberately
        // kept around on a merge conflict, holding real `.txt` decks — it must
        // never surface in the picker as a bogus workspace.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.md"), "## f\nb\n").unwrap();
        let leftover = dir.path().join(".leftover.building");
        std::fs::create_dir(&leftover).unwrap();
        std::fs::write(leftover.join("x.md"), "## q\na\n").unwrap();

        let names: Vec<String> = dir_candidates(dir.path())
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(vec!["real.md".to_string()], names);

        let recent = RecentDecks::load(dir.path().join("recent.json"));
        let entries = catalog(dir.path(), &recent);
        assert!(entries.iter().all(|e| !e.name.starts_with('.')));
        assert_eq!(1, entries.len());
    }

    /// A minimal `DeckStatus` for readiness tests: only `state`/`mastered`/
    /// `has_exam` vary (the readiness rule reads none of the rest).
    fn status_for_readiness(state: DeckState, mastered: bool, has_exam: bool) -> DeckStatus {
        DeckStatus {
            state,
            badge: String::new(),
            locked: false,
            reviewable: false,
            reviewable_recognize: false,
            can_recognize: false,
            reviewable_recall: false,
            reviewable_reconstruct: false,
            mastered,
            is_trace: false,
            examable: false,
            has_exam,
            badge_depth: None,
            badge_dotted: false,
            new_cards: false,
        }
    }

    /// 4 members: mastered+sourced (ready), finished sourceless (ready),
    /// finished sourced but exam not passed (NOT ready), started (NOT
    /// ready).
    fn readiness_fixture() -> Vec<DeckStatus> {
        vec![
            status_for_readiness(DeckState::Finished, true, true),
            status_for_readiness(DeckState::Finished, false, false),
            status_for_readiness(DeckState::Finished, false, true),
            status_for_readiness(DeckState::Started, false, false),
        ]
    }

    #[test]
    fn workspace_readiness_counts_mastered_and_done_sourceless_members() {
        let statuses = readiness_fixture();
        let r = workspace_readiness(&statuses);
        assert_eq!((2, 4), (r.ready, r.total));
    }
    #[test]
    fn readme_and_license_are_not_decks() {
        // The picker scan applies the same conventional-non-deck exclusion as
        // the workspace member scan.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.md"), "## q\na\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "about\n").unwrap();
        std::fs::write(dir.path().join("LICENSE.md"), "MIT\n").unwrap();
        let names: Vec<String> = dir_candidates(dir.path())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(vec!["real.md".to_string()], names);
    }

    #[test]
    fn a_prose_md_file_never_lists_as_a_deck() {
        // A `.md` with neither a `## ` card nor frontmatter is prose, not a
        // deck: it must not list (and so is never selected and stamped).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.md"), "## q\na\n").unwrap();
        std::fs::write(
            dir.path().join("notes.md"),
            "# My notes\n\njust prose, no cards\n",
        )
        .unwrap();
        let names: Vec<String> = dir_candidates(dir.path())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(vec!["real.md".to_string()], names);
    }

    #[test]
    fn a_header_only_stub_still_lists() {
        // A trace stub (frontmatter, zero cards) has no `## ` card yet, but must
        // still list so the user can select and build it. The frontmatter arm
        // of the deck-ness predicate carries it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stub.md"), "---\ntrace: a walk\n---\n").unwrap();
        let names: Vec<String> = dir_candidates(dir.path())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(vec!["stub.md".to_string()], names);
    }
}
