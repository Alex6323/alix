use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[cfg(test)]
use crate::deck::DeckState;
pub use crate::listing::{DeckStatus, deck_status, dependency_forest, member_parents};
use crate::{cache::DeckCache, recent::RecentDecks, title, workspace};

struct Candidate {
    path: PathBuf,
    name: String,
    last_used_ms: Option<u64>,
    is_workspace: bool,
}

fn dir_candidates(
    decks_dir: &Path,
    cache: &mut DeckCache,
) -> Result<Vec<Candidate>, std::io::Error> {
    if cache.is_workspace(decks_dir) {
        return Ok(vec![Candidate {
            name: file_name(decks_dir),
            path: decks_dir.to_path_buf(),
            last_used_ms: None,
            is_workspace: true,
        }]);
    }
    let mut cands: Vec<Candidate> = std::fs::read_dir(decks_dir)?
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
                && !workspace::is_sidecar_name(&name)
                && cache.is_deck(&path);
            if is_deck {
                Some((path, false))
            } else if cache.is_workspace(&path) || cache.has_decks(&path) {
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
        .collect();
    cands.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(cands)
}

fn build_candidates(
    decks_dir: &Path,
    recent: &RecentDecks,
    cache: &mut DeckCache,
) -> Result<Vec<Candidate>, std::io::Error> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for entry in recent.entries() {
        let is_workspace = cache.is_workspace(&entry.path) || cache.has_decks(&entry.path);
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

    for candidate in dir_candidates(decks_dir, cache)? {
        if !seen.contains(&candidate.path) {
            out.push(candidate);
        }
    }
    Ok(out)
}

pub use crate::listing::{WorkspaceReadiness, workspace_readiness};

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn workspace_last_progress(folder: &Path) -> Option<String> {
    let ts = crate::state::open_aggregate_store_tolerant(&workspace::store_path(folder))
        .ok()?
        .last_review_ms()?;
    let now = crate::time::now_ms();
    Some(progress_age(ts, now))
}

fn progress_age(ts: u64, now: u64) -> String {
    if now > ts {
        format!("{} ago", crate::time::humanize_ms(now - ts))
    } else {
        "just now".to_string()
    }
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

pub fn catalog(
    decks_dir: &Path,
    recent: &RecentDecks,
    cache: &mut DeckCache,
) -> Result<Vec<DeckEntry>, std::io::Error> {
    Ok(build_candidates(decks_dir, recent, cache)?
        .into_iter()
        .map(|c| {
            if c.is_workspace {
                let ws = cache.workspace(&c.path);
                let members = ws
                    .members
                    .iter()
                    .map(|m| {
                        let file = file_name(m);
                        DeckEntry {
                            // Qualified so a member never collides with a
                            // top-level deck in the resolution map.
                            name: format!("{}/{}", c.name, file),
                            label: cache.label(m).unwrap_or_else(|| stem(&file)),
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
                DeckEntry {
                    path_hint: location_hint(&c.path, decks_dir),
                    name: c.name,
                    label: ws.display_name(),
                    path: c.path,
                    last_used_ms: c.last_used_ms,
                    is_workspace: true,
                    description: ws.description,
                    members,
                    icon: ws.icon,
                }
            } else {
                DeckEntry {
                    label: cache.label(&c.path).unwrap_or_else(|| stem(&c.name)),
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
        .collect())
}

pub(crate) fn deck_label(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let deck = crate::parser::parse("deck.md", &text).ok()?;
    deck.title
        .or_else(|| deck.frontmatter.trace.map(|t| title::condense(&t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_initialized(path: &Path, text: &str) {
        let id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("deck")
            .replace('-', "");
        let text = if let Some(rest) = text.strip_prefix("---\n") {
            format!("---\nformat-version: 1\nid: \"deck-{id}\"\n{rest}")
        } else {
            format!("---\nformat-version: 1\nid: \"deck-{id}\"\n---\n{text}")
        };
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn workspace_progress_reports_past_equal_and_future_reviews() {
        assert_eq!("1s ago", progress_age(1_000, 2_000));
        assert_eq!("just now", progress_age(2_000, 2_000));
        assert_eq!("just now", progress_age(3_000, 2_000));

        let dir = tempfile::tempdir().unwrap();
        let progress = dir.path().join("progress/deck-test.json");
        let mut store = crate::store::Store::open_deck(&progress, "deck-test", "test.md").unwrap();
        store.get_or_insert("card-test").record_review(
            1,
            crate::scheduler::Grade::Pass,
            crate::depth::Depth::Recall,
            false,
        );
        store.save().unwrap();

        assert!(
            workspace_last_progress(dir.path()).is_some_and(|age| age.ends_with(" ago")),
            "a persisted workspace review must surface a progress age"
        );
    }

    #[test]
    fn stems_drop_only_the_markdown_suffix() {
        assert_eq!("deck", stem("deck.md"));
        assert_eq!("deck.txt", stem("deck.txt"));
    }

    #[test]
    fn a_plain_file_as_decks_root_errors_while_an_empty_dir_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root_file = dir.path().join("root.txt");
        std::fs::write(&root_file, "not a directory").unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));

        assert!(
            catalog(&root_file, &recent, &mut DeckCache::default()).is_err(),
            "a plain-file root must surface the enumeration error"
        );

        let empty = tempfile::tempdir().unwrap();
        let entries = catalog(empty.path(), &recent, &mut DeckCache::default())
            .expect("an empty directory is a successful, empty listing");
        assert!(entries.is_empty());
    }

    #[test]
    fn build_candidates_orders_recent_first_then_alpha() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["zeta.md", "alpha.md", "mid.md"] {
            write_initialized(&dir.path().join(n), "## f\nb\n");
        }
        let recent_path = dir.path().join("recent.json");
        let mut recent = RecentDecks::load(&recent_path);
        recent.record(&[dir.path().join("mid.md")], 1000);

        let cands = build_candidates(dir.path(), &recent, &mut DeckCache::default()).unwrap();
        let names: Vec<&str> = cands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(vec!["mid.md", "alpha.md", "zeta.md"], names);
        assert!(cands[0].last_used_ms.is_some());
        assert!(cands[1].last_used_ms.is_none());
    }

    #[test]
    fn a_workspace_root_lists_as_that_single_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "title = \"T\"\n").unwrap();
        std::fs::create_dir(dir.path().join("decks")).unwrap();
        write_initialized(&dir.path().join("decks/m.md"), "## f\nb\n");
        let recent = RecentDecks::load(dir.path().join("recent.json"));
        let entries = catalog(dir.path(), &recent, &mut DeckCache::default()).unwrap();
        assert_eq!(1, entries.len());
        assert!(entries[0].is_workspace);
        assert_eq!(1, entries[0].members.len());
    }

    #[test]
    fn an_empty_initialized_workspace_still_lists_as_a_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "title = \"T\"\n").unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));

        let entries = catalog(dir.path(), &recent, &mut DeckCache::default()).unwrap();

        assert_eq!(1, entries.len());
        assert!(entries[0].is_workspace);
        assert!(entries[0].members.is_empty());
    }

    #[test]
    fn catalog_mirrors_candidate_order_and_paths() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["zeta.md", "alpha.md"] {
            write_initialized(&dir.path().join(n), "## f\nb\n");
        }
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[dir.path().join("zeta.md")], 1000);

        let entries = catalog(dir.path(), &recent, &mut DeckCache::default()).unwrap();
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
        std::fs::write(&titled, "---\ntitle: The Domain Model\n---\n## f\nb\n").unwrap();
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
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        write_initialized(&ws.join("decks/a.md"), "## a\nb\n");
        write_initialized(&ws.join("decks/b.md"), "## c\nd\n");
        std::fs::write(ws.join(workspace::MANIFEST), "title = \"English\"\n").unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));

        let entries = catalog(dir.path(), &recent, &mut DeckCache::default()).unwrap();
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
        write_initialized(&dir.path().join("real.md"), "## f\nb\n");
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[dir.path().join("deleted.md")], 1000);

        let cands = build_candidates(dir.path(), &recent, &mut DeckCache::default()).unwrap();
        let names: Vec<&str> = cands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(vec!["real.md"], names);
    }

    #[test]
    fn a_dot_prefixed_folder_is_invisible_to_the_scan() {
        let dir = tempfile::tempdir().unwrap();
        write_initialized(&dir.path().join("real.md"), "## f\nb\n");
        let leftover = dir.path().join(".leftover.building");
        std::fs::create_dir(&leftover).unwrap();
        std::fs::write(leftover.join("x.md"), "## q\na\n").unwrap();

        let names: Vec<String> = dir_candidates(dir.path(), &mut DeckCache::default())
            .unwrap()
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(vec!["real.md".to_string()], names);

        let recent = RecentDecks::load(dir.path().join("recent.json"));
        let entries = catalog(dir.path(), &recent, &mut DeckCache::default()).unwrap();
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
            crammable: false,
            progress_error: false,
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
        write_initialized(&dir.path().join("real.md"), "## q\na\n");
        std::fs::write(dir.path().join("README.md"), "about\n").unwrap();
        std::fs::write(dir.path().join("LICENSE.md"), "MIT\n").unwrap();
        let names: Vec<String> = dir_candidates(dir.path(), &mut DeckCache::default())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(vec!["real.md".to_string()], names);
    }

    #[test]
    fn a_prose_md_file_never_lists_as_a_deck() {
        let dir = tempfile::tempdir().unwrap();
        write_initialized(&dir.path().join("real.md"), "## q\na\n");
        std::fs::write(
            dir.path().join("notes.md"),
            "# My notes\n\n## Design\nordinary prose, not a deck\n",
        )
        .unwrap();
        let names: Vec<String> = dir_candidates(dir.path(), &mut DeckCache::default())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(vec!["real.md".to_string()], names);
    }

    #[test]
    fn an_initialized_header_only_stub_still_lists() {
        let dir = tempfile::tempdir().unwrap();
        write_initialized(&dir.path().join("stub.md"), "---\ntrace: a walk\n---\n");
        let names: Vec<String> = dir_candidates(dir.path(), &mut DeckCache::default())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(vec!["stub.md".to_string()], names);
    }
}
