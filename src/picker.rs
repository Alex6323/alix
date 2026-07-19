use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[cfg(test)]
use crate::deck::DeckState;
pub use crate::listing::{DeckStatus, deck_status, dependency_forest, member_parents};
use crate::{recent::RecentDecks, store::Store, title, workspace};

struct Candidate {
    path: PathBuf,
    name: String,
    last_used_ms: Option<u64>,
    is_workspace: bool,
}

fn dir_candidates(decks_dir: &Path) -> Vec<Candidate> {
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
            // Dot-prefixed entries are hidden: `alix generate`'s workspace
            // staging dir uses one and must not surface as a bogus workspace.
            .filter(|path| !file_name(path).starts_with('.'))
            .filter_map(|path| {
                let name = file_name(&path);
                let is_deck = path.is_file()
                    && path.extension().is_some_and(|e| e == "md")
                    && !workspace::is_conventional_non_deck(&name)
                    && !workspace::is_conflict_name(&name)
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

pub use crate::listing::{WorkspaceReadiness, workspace_readiness};

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

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

fn stem(name: &str) -> String {
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

fn location_hint(path: &Path, decks_dir: &Path) -> Option<String> {
    let parent = path.parent()?;
    if parent == decks_dir {
        return None;
    }
    Some(abbreviate_home(parent))
}

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

pub struct DeckEntry {
    pub name: String,
    pub label: String,
    pub path: PathBuf,
    pub last_used_ms: Option<u64>,
    pub is_workspace: bool,
    pub description: Option<String>,
    pub members: Vec<DeckEntry>,
    pub path_hint: Option<String>,
    pub icon: Option<PathBuf>,
}

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
                                    // Qualified so a member never collides with a
                                    // top-level deck in the resolution map.
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
        assert_eq!(vec!["mid.md", "alpha.md", "zeta.md"], names);
        assert!(cands[0].last_used_ms.is_some());
        assert!(cands[1].last_used_ms.is_none());
    }

    #[test]
    fn a_workspace_root_lists_as_that_single_workspace() {
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
        assert_eq!(vec!["zeta.md", "alpha.md"], names);
        assert_eq!(dir.path().join("zeta.md"), entries[0].path);
        assert!(entries[0].last_used_ms.is_some());
    }

    #[test]
    fn deck_label_condenses_a_trace_path_question_instead_of_the_slug() {
        let dir = tempfile::tempdir().unwrap();
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

        let titled = dir.path().join("01-the-domain-model.md");
        std::fs::write(&titled, "# The Domain Model\n## f\nb\n").unwrap();
        assert_eq!(Some("The Domain Model".to_string()), deck_label(&titled));

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
        assert_eq!(None, location_hint(&decks.join("foo.md"), &decks));
        assert_eq!(None, location_hint(&decks.join("english"), &decks));
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
        assert_eq!("english", w.name);
        assert_eq!("English", w.label);
        let members: Vec<&str> = w.members.iter().map(|m| m.name.as_str()).collect();
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
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stub.md"), "---\ntrace: a walk\n---\n").unwrap();
        let names: Vec<String> = dir_candidates(dir.path())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(vec!["stub.md".to_string()], names);
    }
}
