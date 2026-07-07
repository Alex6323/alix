//! Frontend-agnostic deck catalog and status helpers, shared by the web deck
//! picker: the list of decks and workspaces to offer ([`catalog`]), a deck's
//! store-derived badge/lock/gating ([`deck_status`]), and the workspace
//! dependency-forest layout ([`member_parents`] / [`dependency_forest`]).

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::{
    card::Card,
    config::ReviewConfig,
    deck::{self, Deck, DeckState},
    level::Level,
    parser,
    recent::RecentDecks,
    session,
    store::{self, Store},
    title, workspace,
};

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

/// Every `*.txt` deck and every workspace folder directly in `decks_dir`,
/// sorted by name.
fn dir_candidates(decks_dir: &Path) -> Vec<Candidate> {
    let mut cands: Vec<Candidate> = match std::fs::read_dir(decks_dir) {
        Ok(read_dir) => read_dir
            .filter_map(|r| r.ok().map(|d| d.path()))
            .filter_map(|path| {
                if path.is_file() && path.extension().is_some_and(|e| e == "txt") {
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

/// The store-derived status of a deck, shared by the TUI picker and the web
/// deck-selection screen so both surfaces show the same badge, lock, and gating.
pub struct DeckStatus {
    /// Completion state — drives the meta tint (finished → green, exam due →
    /// yellow) and the frontend's machine-readable state string.
    pub state: DeckState,
    /// The badge after the label: `new` · `m/total` · `done ✓` · `mastered 🎉`
    /// · `exam due`.
    pub badge: String,
    /// A `% requires:` prerequisite isn't finished (only when `enforce_locks`).
    /// Shown dimmed with a 🔒; still advisory (review gating, not browse).
    pub locked: bool,
    /// Launching right now would have something to do — a card due/new, a trace
    /// checkpoint, or a due exam. `false` for a fully-drilled / all-on-cooldown
    /// deck, which the review launcher won't start (shown with a 🕒). The OR of
    /// [`reviewable_recognize`](Self::reviewable_recognize),
    /// [`reviewable_recall`](Self::reviewable_recall),
    /// [`reviewable_reconstruct`](Self::reviewable_reconstruct), plus the
    /// non-level trace/exam-due special cases (untouched by the per-level
    /// split) — "anything to do, at any level".
    pub reviewable: bool,
    /// Any deck card hasn't yet been correctly picked at Recognize
    /// (`recognized_ms` unset) — Recognize is unscheduled, so this is a plain
    /// not-yet-done check, not a due time.
    pub reviewable_recognize: bool,
    /// A card is due (or fresh) at Recall right now, or a virtual
    /// (remediation) card is due — what `reviewable` used to mean, entirely.
    pub reviewable_recall: bool,
    /// A non-retired card is due at Reconstruct right now, via the
    /// level-aware scheduler. The cross-level immediacy rule (`Fsrs::due_at`)
    /// means this is `true` for essentially any Recall-established deck —
    /// that's the point: independent Reconstruct practice is reachable the
    /// moment Recall has settled, not gated behind a separate warm-up.
    pub reviewable_reconstruct: bool,
    /// Finished *and* exam-passed — reads `mastered 🎉` rather than `done ✓`, and
    /// belongs in the Mastered window rather than Recent.
    pub mastered: bool,
    /// A trace deck (`% trace:`): launched as a predict-verify walk, never a
    /// card review.
    pub is_trace: bool,
    /// The AI exam can be sat now: the deck has an exam ([`has_exam`]) and its
    /// `% requires:` are met (not locked) — drilled or not, so you can test out
    /// early. (A failed trace exam's re-sit cooldown is enforced separately at
    /// the launch site, which has the config.)
    ///
    /// [`has_exam`]: DeckStatus::has_exam
    pub examable: bool,
    /// The deck *has* an AI exam at all — a `% source:` fact deck, or a **trace**
    /// (its exam is the graded compression) — whether or not it can be sat right
    /// now. (`examable` is this AND not locked.) Lets a frontend always show a
    /// "Take exam" control, disabled when locked.
    pub has_exam: bool,
    /// The highest level with a badge to show, walking `[Reconstruct, Recall,
    /// Recognize]` high to low: the first level currently solid
    /// ([`store::badge_solid`]) wins (subsumption — a higher badge implies the
    /// lower checks were passed too); else the first level with an earn date
    /// ([`Store::badge_earned`]) wins; else `None`. Additive telemetry —
    /// gates nothing.
    pub badge_level: Option<Level>,
    /// `true` when `badge_level` was won by an earn date rather than current
    /// solidity — the badge lapsed (e.g. stability dropped) and should render
    /// dotted rather than solid.
    pub badge_dotted: bool,
    /// Any deck card has no store entry at all (never reviewed) — fresh
    /// material, distinct from `state`/`badge`.
    pub new_cards: bool,
}

/// The subsumption walk (spec §4.4, `{#check-matrix}`): the highest level
/// that's currently solid wins with `dotted=false`; else the highest with an
/// earn date wins with `dotted=true`; else `(None, false)`.
fn badge_level_for(subject: &str, cards: &[Card], store: &Store) -> (Option<Level>, bool) {
    const LEVELS: [Level; 3] = [Level::Reconstruct, Level::Recall, Level::Recognize];
    if let Some(level) = LEVELS
        .into_iter()
        .find(|&level| store::badge_solid(cards, store, level))
    {
        return (Some(level), false);
    }
    let earned = LEVELS
        .into_iter()
        .find(|&level| store.badge_earned(subject, level).is_some());
    let dotted = earned.is_some();
    (earned, dotted)
}

/// Computes a deck's [`DeckStatus`] from the progress `store`, mirroring what a
/// review session would see. `decks_dir` roots `% requires:` resolution for the
/// lock check; `enforce_locks` is false for browse (any deck is browsable).
pub fn deck_status(
    deck: &Deck,
    store: &Store,
    decks_dir: Option<&Path>,
    enforce_locks: bool,
    review: ReviewConfig,
) -> DeckStatus {
    let state = deck.state(store);
    let total = deck.cards.len();
    let retired = deck
        .cards
        .iter()
        .filter(|card| session::is_retired(card, store, review.retire_after_days))
        .count();
    // Progress toward "finished" is graduation (reaching FSRS review), not
    // retirement — which is a rare, far-later resting point (a year or more out).
    let graduated = deck
        .cards
        .iter()
        .filter(|card| session::has_graduated(card, store))
        .count();
    // "mastered" is reserved for passing the exam; a source-less deck that's
    // merely fully drilled stays "done".
    let mastered = matches!(state, DeckState::Finished) && store.deck_mastered(&deck.subject);
    let badge = match state {
        DeckState::Finished if mastered => {
            // In the Mastered window: when it was mastered, and how many of its
            // cards are still drillable (e.g. you tested out of the exam without
            // drilling them all).
            let mut s = "mastered 🎉".to_string();
            if let Some(at) = store.deck_mastered_at(&deck.subject) {
                let ago = crate::time::humanize_ms(session::now_ms().saturating_sub(at));
                s.push_str(&format!(" · {ago} ago"));
            }
            let to_drill = total - retired;
            if to_drill > 0 {
                s.push_str(&format!(" · {to_drill} to drill"));
            }
            s
        }
        DeckState::Finished => "done ✓".to_string(),
        DeckState::ExamDue => "exam due".to_string(),
        DeckState::NotStarted => "new".to_string(),
        DeckState::Started => format!("{graduated}/{total}"),
    };
    let actually_locked = deck::is_locked(deck, decks_dir, store);
    let locked = enforce_locks && actually_locked;
    // A deck has an exam when it has a source (a fact deck) or is a trace (its
    // exam is the graded compression); it can be SAT when, additionally, it isn't
    // locked — drilled or not (test out early). The failed-exam re-sit cooldown
    // (trace exams) is enforced at the launch sites, which have the config.
    let has_exam = deck.has_exam();
    let examable = has_exam && !actually_locked;
    // Per-level due-ness (spec `{#check-matrix}`): each level's own honest
    // signal, via the level-aware scheduler. Recognize needs any card not yet
    // correctly picked; Recall/Reconstruct each read their own independent
    // schedule — the scheduler's cross-level immediacy rule (`Fsrs::due_at`)
    // is what makes a Recall-settled deck due right now at Reconstruct too.
    let scheduler = crate::scheduler::Fsrs::new(review.retention);
    let now = session::now_ms();
    let reviewable_recognize = session::has_reviewable(
        &deck.cards,
        store,
        &scheduler,
        Level::Recognize,
        now,
        review.retire_after_days,
    );
    let reviewable_recall = session::has_reviewable(
        &deck.cards,
        store,
        &scheduler,
        Level::Recall,
        now,
        review.retire_after_days,
    ) || session::has_reviewable_virtual(
        store,
        &deck.subject,
        &scheduler,
        now,
        review.retire_after_days,
    );
    let reviewable_reconstruct = session::has_reviewable(
        &deck.cards,
        store,
        &scheduler,
        Level::Reconstruct,
        now,
        review.retire_after_days,
    );
    // Is there anything to launch right now, at any level? A trace always
    // walks; an exam-due deck launches its exam (only when its exam isn't
    // locked) — both non-level special cases, unchanged by the per-level
    // split above; otherwise it's whichever level currently has something due
    // (or new, or a due virtual card). Drilling is never gated by the lock —
    // a prerequisite-locked deck with due cards is still reviewable.
    let reviewable = deck.is_trace()
        || (matches!(state, DeckState::ExamDue) && examable)
        || reviewable_recognize
        || reviewable_recall
        || reviewable_reconstruct;
    let (badge_level, badge_dotted) = badge_level_for(&deck.subject, &deck.cards, store);
    let new_cards = deck.cards.iter().any(|card| store.get(card.id()).is_none());
    DeckStatus {
        state,
        badge,
        locked,
        reviewable,
        reviewable_recognize,
        reviewable_recall,
        reviewable_reconstruct,
        mastered,
        is_trace: deck.is_trace(),
        examable,
        has_exam,
        badge_level,
        badge_dotted,
        new_cards,
    }
}

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

/// A deck name without its `.txt` extension, for matching.
fn stem(name: &str) -> String {
    name.strip_suffix(".txt").unwrap_or(name).to_string()
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

/// The catalog the pickers show, as plain data: recent entries first (recency
/// order), then every other deck and workspace in `decks_dir`.
/// Frontend-agnostic, so the web deck-selection screen presents the same list
/// as the TUI picker.
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

/// A deck's display label, read without a full parse: its explicit `% title:`,
/// else — for a trace — a condensed form of its `% trace:` path-question (an
/// `explore` trace's is already short, a `--build`/hand-written one gets cut to
/// a label-sized head). `None` when it declares neither, so the caller falls
/// back to the file stem.
fn deck_label(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    parser::parse_title(&text).or_else(|| parser::parse_trace(&text).map(|t| title::condense(&t)))
}

/// The index of the first prerequisite (`% requires:`) each member declares that
/// is *also a member* of this set — the edge that gates it in the dependency
/// tree. `None` means a root: no prerequisite, or one outside the set. Resolves
/// names with [`deck::resolve_dep`]; an unreadable deck is a root. Shared with
/// the web picker, which lays members out as the same unlock tree.
pub fn member_parents(members: &[PathBuf], decks_dir: &Path) -> Vec<Option<usize>> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let canonical: Vec<PathBuf> = members.iter().map(|m| canon(m)).collect();
    members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let deck = Deck::load(m).ok()?;
            deck.requires.iter().find_map(|req| {
                let dep = canon(&deck::resolve_dep(req, Some(decks_dir), m.parent())?);
                canonical.iter().position(|c| *c == dep).filter(|&j| j != i)
            })
        })
        .collect()
}

/// Orders member indices as a dependency forest for display: roots first, each
/// deck's dependents nested beneath the prerequisite that gates them, siblings by
/// `key` (e.g. startable-first, then name). `parent[i]` is the gating member's
/// index, or `None` for a root. Returns `(index, prefix)` in pre-order, the
/// prefix drawing the `├─`/`└─`/`│` branch lines. A dependency cycle can't strand
/// a node — any left unvisited is appended as its own root. Shared with the web
/// picker: each branch segment is exactly three chars wide, so a row's nesting
/// depth is `prefix.chars().count() / 3`.
pub fn dependency_forest<K: Ord>(parent: &[Option<usize>], key: &[K]) -> Vec<(usize, String)> {
    let n = parent.len();
    let mut children = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (i, p) in parent.iter().enumerate() {
        match *p {
            Some(p) if p < n && p != i => children[p].push(i),
            _ => roots.push(i),
        }
    }
    roots.sort_by(|a, b| key[*a].cmp(&key[*b]));
    for kids in &mut children {
        kids.sort_by(|a, b| key[*a].cmp(&key[*b]));
    }

    let mut out = Vec::new();
    let mut visited = vec![false; n];
    let root_count = roots.len();
    for (k, &r) in roots.iter().enumerate() {
        visit_node(
            r,
            "",
            k + 1 == root_count,
            true,
            &children,
            &mut visited,
            &mut out,
        );
    }
    // A node unreached from any root (a dependency cycle) becomes its own root.
    for i in 0..n {
        if !visited[i] {
            visit_node(i, "", true, true, &children, &mut visited, &mut out);
        }
    }
    out
}

/// Pre-order DFS appending `(index, prefix)`. `ancestor` is the branch prefix
/// inherited from parents, `is_last` whether this node is its parent's last
/// child, `is_root` whether it sits at the top level (no branch glyph).
fn visit_node(
    i: usize,
    ancestor: &str,
    is_last: bool,
    is_root: bool,
    children: &[Vec<usize>],
    visited: &mut [bool],
    out: &mut Vec<(usize, String)>,
) {
    if visited[i] {
        return;
    }
    visited[i] = true;
    let connector = if is_root {
        ""
    } else if is_last {
        "└─ "
    } else {
        "├─ "
    };
    out.push((i, format!("{ancestor}{connector}")));
    let child_ancestor = if is_root {
        String::new()
    } else if is_last {
        format!("{ancestor}   ")
    } else {
        format!("{ancestor}│  ")
    };
    let kids = &children[i];
    for (k, &c) in kids.iter().enumerate() {
        visit_node(
            c,
            &child_ancestor,
            k + 1 == kids.len(),
            false,
            children,
            visited,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_forest_nests_dependents_under_prerequisites() {
        // 0 data-model (root); 1 leitner, 3 sm2, 4 queue-building require 0;
        // 2 grading requires 1. Siblings order by name.
        let names: Vec<String> = ["data-model", "leitner", "grading", "sm2", "queue-building"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parent = vec![None, Some(0), Some(1), Some(0), Some(0)];
        let order = dependency_forest(&parent, &names);
        assert_eq!(
            vec![
                (0, "".to_string()),
                (1, "├─ ".to_string()),
                (2, "│  └─ ".to_string()),
                (4, "├─ ".to_string()),
                (3, "└─ ".to_string()),
            ],
            order
        );
    }

    #[test]
    fn dependency_forest_survives_a_cycle() {
        // 0 and 1 require each other: no roots, but every node still appears once.
        let names = vec!["a".to_string(), "b".to_string()];
        let parent = vec![Some(1), Some(0)];
        let order = dependency_forest(&parent, &names);
        let mut indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        indices.sort();
        assert_eq!(vec![0, 1], indices);
    }

    #[test]
    fn build_candidates_orders_recent_first_then_alpha() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["zeta.txt", "alpha.txt", "mid.txt"] {
            std::fs::write(dir.path().join(n), "# f\n\tb\n").unwrap();
        }
        let recent_path = dir.path().join("recent.json");
        let mut recent = RecentDecks::load(&recent_path);
        recent.record(&[dir.path().join("mid.txt")], 1000);

        let cands = build_candidates(dir.path(), &recent);
        let names: Vec<&str> = cands.iter().map(|c| c.name.as_str()).collect();
        // Recent (mid) first, then the rest alphabetically.
        assert_eq!(vec!["mid.txt", "alpha.txt", "zeta.txt"], names);
        assert!(cands[0].last_used_ms.is_some());
        assert!(cands[1].last_used_ms.is_none());
    }

    #[test]
    fn catalog_mirrors_candidate_order_and_paths() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["zeta.txt", "alpha.txt"] {
            std::fs::write(dir.path().join(n), "# f\n\tb\n").unwrap();
        }
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[dir.path().join("zeta.txt")], 1000);

        let entries = catalog(dir.path(), &recent);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(vec!["zeta.txt", "alpha.txt"], names); // recent first
        assert_eq!(dir.path().join("zeta.txt"), entries[0].path);
        assert!(entries[0].last_used_ms.is_some());
    }

    #[test]
    fn deck_label_condenses_a_trace_path_question_instead_of_the_slug() {
        let dir = tempfile::tempdir().unwrap();
        // A trace declares its name in `% trace:`, not `% title:` — the label
        // comes from a condensed form of it, never the file stem.
        let trace = dir.path().join("06-how-a-digest-becomes-verified.txt");
        std::fs::write(
            &trace,
            "% trace: how a transaction digest becomes verified effects and events: \
             fetch the checkpoint, derive the committee, then verify\n",
        )
        .unwrap();
        assert_eq!(
            Some("How a Transaction Digest Becomes Verified Effects and Events".to_string()),
            deck_label(&trace),
        );

        // An explicit `% title:` still wins outright.
        let titled = dir.path().join("01-the-domain-model.txt");
        std::fs::write(&titled, "% title: The Domain Model\n# f\n\tb\n").unwrap();
        assert_eq!(Some("The Domain Model".to_string()), deck_label(&titled));

        // A plain deck with neither yields None (the caller falls back to stem).
        let plain = dir.path().join("plain.txt");
        std::fs::write(&plain, "# f\n\tb\n").unwrap();
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
        assert_eq!(None, location_hint(&decks.join("foo.txt"), &decks));
        assert_eq!(None, location_hint(&decks.join("english"), &decks));
        // Elsewhere → the parent dir, home abbreviated to `~`.
        assert_eq!(
            Some("~/other".to_string()),
            location_hint(&home.join("other").join("x.txt"), &decks)
        );
        assert_eq!(
            Some("/tmp".to_string()),
            location_hint(Path::new("/tmp/x.txt"), &decks)
        );
    }

    #[test]
    fn catalog_surfaces_workspace_with_qualified_members() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("english");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
        std::fs::write(ws.join("b.txt"), "# c\n\td\n").unwrap();
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
        assert_eq!(vec!["english/a.txt", "english/b.txt"], members);
    }

    #[test]
    fn build_candidates_skips_missing_recent_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.txt"), "# f\n\tb\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[dir.path().join("deleted.txt")], 1000);

        let cands = build_candidates(dir.path(), &recent);
        let names: Vec<&str> = cands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(vec!["real.txt"], names);
    }

    /// A graduated-and-not-due `FsrsState`, so the deck card it's attached to
    /// contributes nothing to `reviewable` (Review state, far-out due, well
    /// under the retirement cap).
    fn graduated_not_due(now: u64) -> crate::store::FsrsState {
        crate::store::FsrsState {
            state: 2, // Review — graduated
            scheduled_days: 30,
            due_ms: now + 30 * 86_400_000,
            ..Default::default()
        }
    }

    /// Inserts a virtual (remediation) card for `subject` into `store`, due
    /// immediately — sidecar content keyed by its `Card::id`, plus a fresh
    /// schedule seeded at t=0 (so any real `now` is well past its acquire
    /// cooldown).
    fn insert_due_virtual_card(store: &mut Store, subject: &str) {
        let text = "# virtual front\n\tvirtual back\n".to_string();
        let id = crate::parser::parse_str(subject, &text).unwrap()[0].id();
        store.insert_virtual(crate::store::VirtualCard {
            id,
            kind: crate::store::VirtualKind::Remediation,
            parent: subject.to_string(),
            text,
            created_ms: 0,
        });
        store.get_or_insert(id, 0);
    }

    #[test]
    fn deck_status_reviewable_true_when_only_a_virtual_card_is_due() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = session::now_ms();
        let card_id = deck.cards[0].id();
        // Fully done at *every* level — recognized, and graduated-not-due at
        // both Recall and Reconstruct (a real Reconstruct schedule, so the
        // cross-level immediacy rule doesn't fire) — so only a virtual card
        // can make this deck reviewable.
        let entry = store.get_or_insert(card_id, now);
        entry.recognized_ms = Some(now);
        entry.recall = Some(graduated_not_due(now));
        entry.reconstruct = Some(graduated_not_due(now));

        // Fully drilled, nothing due: `done ✓` and not reviewable.
        let status = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert_eq!("done ✓", status.badge);
        assert!(!status.reviewable);

        // A due virtual card for this deck makes it reviewable even though
        // every deck card is done at every level.
        insert_due_virtual_card(&mut store, &deck.subject);
        let status = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert!(status.reviewable);
        assert_eq!("done ✓", status.badge); // unaffected by the virtual card
    }

    #[test]
    fn a_recall_settled_deck_is_still_reviewable_at_reconstruct() {
        // The final-review fix (Important #1): a deck that's mature and not
        // due at Recall is due *right now* at Reconstruct, via the
        // scheduler's cross-level immediacy rule — `reviewable_recall` must
        // say no while `reviewable_reconstruct` (and the overall
        // `reviewable`) say yes.
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = session::now_ms();
        let card_id = deck.cards[0].id();
        // Mature at Recall, nothing due there — never touched at Reconstruct.
        store.get_or_insert(card_id, now).recall = Some(mature(now, 25.0));

        let status = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert!(!status.reviewable_recall, "nothing due at recall");
        assert!(
            status.reviewable_reconstruct,
            "due now at reconstruct via the immediacy rule"
        );
        assert!(status.reviewable, "reviewable overall via reconstruct");
    }

    #[test]
    fn recognize_reviewability_tracks_unrecognized_cards() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n# q2\n\ta2\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = session::now_ms();
        store.get_or_insert(deck.cards[0].id(), now).recognized_ms = Some(now);

        // Card 2 has never been correctly picked at Recognize.
        let status = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert!(status.reviewable_recognize);

        store.get_or_insert(deck.cards[1].id(), now).recognized_ms = Some(now);
        let status = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert!(!status.reviewable_recognize);
    }

    #[test]
    fn deck_status_total_ignores_virtual_cards() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n# q2\n\ta2\n# q3\n\ta3\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = session::now_ms();
        // One of the three cards has graduated; the other two are unseen.
        store.get_or_insert(deck.cards[0].id(), now).recall = Some(graduated_not_due(now));

        let before = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert_eq!("1/3", before.badge);

        insert_due_virtual_card(&mut store, &deck.subject);
        let after = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert_eq!(before.badge, after.badge);
    }

    /// A graduated `FsrsState` with the given `stability` (days) — the only
    /// thing `badge_solid` checks. Far-out due, like `graduated_not_due`, so it
    /// never contributes to `reviewable`.
    fn mature(now: u64, stability: f64) -> crate::store::FsrsState {
        crate::store::FsrsState {
            state: 2,
            stability,
            scheduled_days: 30,
            due_ms: now + 30 * 86_400_000,
            ..Default::default()
        }
    }

    #[test]
    fn the_highest_currently_solid_level_wins_the_badge() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = session::now_ms();
        let card_id = deck.cards[0].id();
        // Both Recall and Reconstruct are currently solid — the higher one
        // (Reconstruct) must win, not Recall.
        let entry = store.get_or_insert(card_id, now);
        entry.recall = Some(mature(now, 25.0));
        entry.reconstruct = Some(mature(now, 30.0));

        let status = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert_eq!(Some(Level::Reconstruct), status.badge_level);
        assert!(!status.badge_dotted);
    }

    #[test]
    fn an_earned_but_lapsed_badge_shows_dotted() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = session::now_ms();
        let card_id = deck.cards[0].id();
        store.get_or_insert(card_id, now).recall = Some(mature(now, 25.0));
        crate::store::note_badges(&mut store, &deck.subject, &deck.cards, now);

        // The card's stability lapses back under the mature line — no longer
        // solid, but the earn date persists (high-water mark).
        store.get_or_insert(card_id, now).recall = Some(mature(now, 5.0));

        let status = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert_eq!(Some(Level::Recall), status.badge_level);
        assert!(status.badge_dotted);
    }

    #[test]
    fn new_cards_flag_fires_without_touching_badges() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n# q2\n\ta2\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = session::now_ms();
        // Card 1 has graduated; card 2 has never been touched — no store entry.
        store.get_or_insert(deck.cards[0].id(), now).recall = Some(graduated_not_due(now));

        let status = deck_status(&deck, &store, None, false, ReviewConfig::default());
        assert!(status.new_cards);
        // Unaffected by the flag: same state/badge/badge_level this deck would
        // have had before `new_cards` existed.
        assert_eq!("1/2", status.badge);
        assert_eq!(None, status.badge_level);
        assert!(!status.badge_dotted);
    }
}
