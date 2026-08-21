use std::path::{Path, PathBuf};

use crate::{
    augment::AugmentCache,
    card::Card,
    config::ReviewConfig,
    deck::{self, Deck, DeckState},
    depth::{self, Depth},
    scheduler::Fsrs,
    session,
    store::{self, Store},
    workspace,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckSummary {
    pub title: String,
    pub path: PathBuf,
    pub is_workspace: bool,
    pub due: bool,
    pub can_recognize: bool,
    pub ready: bool,
    pub deadline: Option<DeckDeadline>,
    pub is_trace: bool,
    pub last_depth: Depth,
    pub mastered: bool,
    pub exam_due: bool,
    pub has_exam: bool,
    pub locked: bool,
    /// The deck's OWN progress document failed to load, so every
    /// progress-derived claim here is suppressed and review is refused;
    /// siblings are unaffected (per-document read granularity).
    pub progress_error: bool,
    pub icon: Option<PathBuf>,
    pub indent: usize,
    pub tree: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckDeadline {
    pub date: chrono::NaiveDate,
    pub days_left: i64,
    pub ready: usize,
    pub total: usize,
}

pub fn list_root(root: &Path, review: &ReviewConfig, now_ms: u64) -> Vec<DeckSummary> {
    if workspace::is_workspace(root) {
        return vec![folder_summary(root, root, review, now_ms)];
    }
    let (root_store, health) = open_listing_store(&workspace::root_store_path(root));
    let augment = AugmentCache::open_for_workspace(root).ok();
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
        if path.is_dir() && (workspace::is_workspace(&path) || workspace::has_decks(&path)) {
            out.push(folder_summary(root, &path, review, now_ms));
        } else if path.is_file()
            && path.extension().is_some_and(|e| e == "md")
            && !workspace::is_conventional_non_deck(name)
            && !workspace::is_conflict_name(name)
            && !workspace::is_sidecar_name(name)
            && workspace::file_is_deck(&path)
        {
            out.push(
                deck_summary(
                    &path,
                    root_store.as_ref(),
                    &health,
                    augment.as_ref(),
                    root,
                    review,
                    now_ms,
                )
                .0,
            );
        }
    }
    out
}

pub fn list_members(
    root: &Path,
    dir: &Path,
    review: &ReviewConfig,
    now_ms: u64,
) -> Vec<DeckSummary> {
    let (paths, rows) = member_rows(root, dir, review, now_ms);
    let parent = member_parents(&paths, root);
    let key: Vec<(bool, String)> = rows
        .iter()
        .map(|(row, loaded)| {
            let blocked = *loaded
                && (row.locked
                    || !(row.is_trace || (row.exam_due && row.has_exam && !row.locked) || row.due));
            (blocked, row.title.clone())
        })
        .collect();
    dependency_forest(&parent, &key)
        .into_iter()
        .map(|(i, prefix)| {
            let mut row = rows[i].0.clone();
            row.indent = prefix.chars().count() / 3;
            row.tree = prefix;
            row
        })
        .collect()
}

fn member_rows(
    root: &Path,
    dir: &Path,
    review: &ReviewConfig,
    now_ms: u64,
) -> (Vec<PathBuf>, Vec<(DeckSummary, bool)>) {
    let (store, health) = member_store(root, dir);
    let augment = AugmentCache::open_for_workspace(dir).ok();
    let paths: Vec<PathBuf> = match workspace::Workspace::load(dir) {
        Ok(ws) => ws.members,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    let rows: Vec<(DeckSummary, bool)> = paths
        .iter()
        .map(|m| {
            deck_summary(
                m,
                store.as_ref(),
                &health,
                augment.as_ref(),
                root,
                review,
                now_ms,
            )
        })
        .collect();
    (paths, rows)
}

pub fn workspace_deadline(
    root: &Path,
    dir: &Path,
    review: &ReviewConfig,
    now_ms: u64,
) -> Option<DeckDeadline> {
    let rows = member_rows(root, dir, review, now_ms).1;
    deadline_for(dir, &rows, review, now_ms)
}

fn deadline_for(
    dir: &Path,
    rows: &[(DeckSummary, bool)],
    review: &ReviewConfig,
    now_ms: u64,
) -> Option<DeckDeadline> {
    if !workspace::is_workspace(dir) {
        return None;
    }
    let date = (*review).for_workspace(dir).deadline?;
    let loadable = rows.iter().filter(|(_, loaded)| *loaded);
    Some(DeckDeadline {
        date,
        days_left: (date - crate::time::local_date(now_ms)).num_days(),
        ready: loadable.clone().filter(|(m, _)| m.ready).count(),
        total: loadable.count(),
    })
}

pub fn sync_conflicts_under(root: &Path) -> Vec<PathBuf> {
    let mut out = store::sync_conflicts(&workspace::root_store_path(root));
    out.extend(crate::augment::sync_conflicts(root));
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() && workspace::is_workspace(&path) {
            out.extend(store::sync_conflicts(&workspace::store_path(&path)));
            out.extend(crate::augment::sync_conflicts(&path));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Which decks' progress documents failed to read: per-deck ids from the
/// tolerant open, or everything when the store itself cannot be opened
/// (directory damage, a cross-document ownership conflict). A missing
/// progress directory is a FRESH store, never damage.
enum ProgressHealth {
    PerDeck(std::collections::HashSet<String>),
    AllFailed,
}

impl ProgressHealth {
    fn error_for(&self, deck: Option<&Deck>) -> bool {
        match self {
            ProgressHealth::AllFailed => true,
            ProgressHealth::PerDeck(failed) => deck
                .and_then(|d| d.deck_token.as_deref())
                .is_some_and(|token| failed.contains(token)),
        }
    }
}

fn open_listing_store(path: &Path) -> (Option<Store>, ProgressHealth) {
    match crate::state::open_aggregate_store_tolerant(path) {
        Ok(store) => {
            let failed = store.failed_decks().keys().cloned().collect();
            (Some(store), ProgressHealth::PerDeck(failed))
        }
        Err(_) => (None, ProgressHealth::AllFailed),
    }
}

fn member_store(root: &Path, dir: &Path) -> (Option<Store>, ProgressHealth) {
    let path = if workspace::is_workspace(dir) {
        workspace::store_path(dir)
    } else {
        workspace::root_store_path(root)
    };
    open_listing_store(&path)
}

fn folder_summary(root: &Path, dir: &Path, review: &ReviewConfig, now_ms: u64) -> DeckSummary {
    let ws = workspace::Workspace::load(dir).ok();
    let title = ws.as_ref().map(|ws| ws.display_name()).unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let icon = ws.and_then(|ws| ws.icon);
    let rows = member_rows(root, dir, review, now_ms).1;
    let due = rows.iter().any(|(m, _)| m.due);
    let can_recognize = rows.iter().any(|(m, _)| m.can_recognize);
    let deadline = deadline_for(dir, &rows, review, now_ms);
    DeckSummary {
        title,
        path: dir.to_path_buf(),
        is_workspace: true,
        due,
        can_recognize,
        ready: false,
        deadline,
        is_trace: false,
        last_depth: Depth::default(),
        mastered: false,
        exam_due: false,
        has_exam: false,
        locked: false,
        progress_error: false,
        icon,
        indent: 0,
        tree: String::new(),
    }
}

fn deck_summary(
    path: &Path,
    store: Option<&Store>,
    health: &ProgressHealth,
    augment: Option<&AugmentCache>,
    decks_dir: &Path,
    review: &ReviewConfig,
    now_ms: u64,
) -> (DeckSummary, bool) {
    let stem = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let deck = Deck::load(path).ok();
    let loaded = deck.is_some();
    let title = deck.as_ref().map(|d| d.display_name()).unwrap_or_default();
    let is_trace = deck.as_ref().is_some_and(|d| d.is_trace());
    let has_exam = deck.as_ref().is_some_and(|d| d.has_exam());
    // `augment` is `Some` exactly when `store` is, so gating on all three
    // preserves that invariant for `deck_due`.
    let due = match (&deck, store, augment) {
        (Some(d), Some(s), Some(a)) => deck_due(d, s, a, review, now_ms),
        _ => false,
    };
    let can_recognize = match (&deck, augment) {
        (Some(d), Some(a)) => depth::deck_recognizable(&d.cards, a),
        _ => false,
    };
    let (mastered, exam_due, ready, locked) = match (&deck, store) {
        (Some(d), Some(s)) => {
            let mastered = s.deck_mastered(d.deck_token.as_deref().unwrap_or_default());
            let state = d.state(s);
            (
                mastered,
                state == DeckState::ExamDue,
                deadline_ready(mastered, state == DeckState::Finished, has_exam),
                deck::is_locked(d, Some(decks_dir), s),
            )
        }
        _ => (false, false, false, false),
    };
    let last_depth = match (&deck, store, augment) {
        (Some(d), Some(s), Some(a)) => s
            .last_depth(d.deck_token.as_deref().unwrap_or_default())
            .unwrap_or_else(|| depth::default_depth(&d.cards, a)),
        _ => Depth::default(),
    };
    let progress_error = health.error_for(deck.as_ref());
    // An error row makes NO progress-derived claims: even "due" would be a
    // fabrication (an empty view of an unreadable document reads as fresh).
    let row = DeckSummary {
        title: if title.is_empty() { stem } else { title },
        path: path.to_path_buf(),
        is_workspace: false,
        due: due && !progress_error,
        can_recognize,
        ready: ready && !progress_error,
        deadline: None,
        is_trace,
        last_depth,
        mastered: mastered && !progress_error,
        exam_due: exam_due && !progress_error,
        has_exam,
        locked,
        progress_error,
        icon: None,
        indent: 0,
        tree: String::new(),
    };
    (row, loaded)
}

fn deck_due(
    deck: &Deck,
    store: &Store,
    augment: &AugmentCache,
    review: &ReviewConfig,
    now_ms: u64,
) -> bool {
    let scheduler = Fsrs::new(review.retention, review.introduction_cooldown_ms);
    let retire = review.retire_after_days;
    // Recognize needs both due AND recognizable, or an un-augmented deck
    // over-reports as due.
    let recognize_due = deck.cards.iter().any(|c| {
        session::is_reviewable(c, store, &scheduler, Depth::Recognize, now_ms, retire)
            && depth::card_recognizable(c, augment, &deck.cards)
    });
    recognize_due
        || session::has_reviewable(
            &deck.cards,
            store,
            &scheduler,
            Depth::Recall,
            now_ms,
            retire,
        )
        || session::has_reviewable(
            &deck.cards,
            store,
            &scheduler,
            Depth::Reconstruct,
            now_ms,
            retire,
        )
        || session::has_reviewable(
            &crate::personal::read(&deck.path, &deck.subject).cards,
            store,
            &scheduler,
            Depth::Recall,
            now_ms,
            retire,
        )
}

pub fn deadline_ready(mastered: bool, finished: bool, has_exam: bool) -> bool {
    mastered || (finished && !has_exam)
}

pub struct WorkspaceReadiness {
    pub ready: usize,
    pub total: usize,
}

pub fn workspace_readiness(statuses: &[DeckStatus]) -> WorkspaceReadiness {
    let ready = statuses
        .iter()
        .filter(|s| deadline_ready(s.mastered, s.state == DeckState::Finished, s.has_exam))
        .count();
    WorkspaceReadiness {
        ready,
        total: statuses.len(),
    }
}

#[derive(Clone)]
pub struct DeckStatus {
    pub state: DeckState,
    pub badge: String,
    pub locked: bool,
    pub reviewable: bool,
    pub reviewable_recognize: bool,
    pub can_recognize: bool,
    pub reviewable_recall: bool,
    pub reviewable_reconstruct: bool,
    pub mastered: bool,
    pub is_trace: bool,
    pub examable: bool,
    pub has_exam: bool,
    pub badge_depth: Option<Depth>,
    pub badge_dotted: bool,
    pub new_cards: bool,
    pub progress_error: bool,
}

fn badge_depth_for(deck_id: &str, cards: &[Card], store: &Store) -> (Option<Depth>, bool) {
    const DEPTHS: [Depth; 3] = [Depth::Reconstruct, Depth::Recall, Depth::Recognize];
    if let Some(depth) = DEPTHS
        .into_iter()
        .find(|&depth| store::badge_solid(cards, store, depth))
    {
        return (Some(depth), false);
    }
    let earned = DEPTHS
        .into_iter()
        .find(|&depth| store.badge_earned(deck_id, depth).is_some());
    let dotted = earned.is_some();
    (earned, dotted)
}

pub fn deck_status(
    deck: &Deck,
    store: &Store,
    augment: &AugmentCache,
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
    // "mastered" is reserved for passing the exam; a deck without exam
    // grounding that's merely fully drilled stays "done".
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    let mastered = matches!(state, DeckState::Finished) && store.deck_mastered(deck_id);
    let badge = match state {
        DeckState::Finished if mastered => {
            let mut s = "mastered 🎉".to_string();
            if let Some(at) = store.deck_mastered_at(deck_id) {
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
        DeckState::NotStarted | DeckState::Started => String::new(),
    };
    let actually_locked = deck::is_locked(deck, decks_dir, store);
    let locked = enforce_locks && actually_locked;
    // examable doesn't require drilling first: you can test out early. The
    // exam re-sit cooldown is enforced at the launch sites, not here.
    let has_exam = deck.has_exam();
    let examable = has_exam && !actually_locked;
    let scheduler = crate::scheduler::Fsrs::new(review.retention, review.introduction_cooldown_ms);
    let now = session::now_ms();
    // A card counts only if it's both unrecognized AND recognizable; an
    // un-augmented card must never count as due.
    let reviewable_recognize = deck.cards.iter().any(|card| {
        session::is_reviewable(
            card,
            store,
            &scheduler,
            Depth::Recognize,
            now,
            review.retire_after_days,
        ) && depth::card_recognizable(card, augment, &deck.cards)
    });
    let can_recognize = depth::deck_recognizable(&deck.cards, augment);
    let reviewable_recall = session::has_reviewable(
        &deck.cards,
        store,
        &scheduler,
        Depth::Recall,
        now,
        review.retire_after_days,
    ) || session::has_reviewable(
        &crate::personal::read(&deck.path, &deck.subject).cards,
        store,
        &scheduler,
        Depth::Recall,
        now,
        review.retire_after_days,
    );
    // A deck fully settled at Recall reads due again immediately at
    // Reconstruct (the scheduler's cross-depth immediacy rule), not a bug.
    let reviewable_reconstruct = session::has_reviewable(
        &deck.cards,
        store,
        &scheduler,
        Depth::Reconstruct,
        now,
        review.retire_after_days,
    );
    // Locked never blocks drilling: a prerequisite-locked deck with due
    // cards is still reviewable.
    let reviewable = deck.is_trace()
        || (matches!(state, DeckState::ExamDue) && examable)
        || reviewable_recognize
        || reviewable_recall
        || reviewable_reconstruct;
    let (badge_depth, badge_dotted) = badge_depth_for(deck_id, &deck.cards, store);
    let new_cards = deck
        .cards
        .iter()
        .any(|card| card.id().and_then(|id| store.progress(&id)).is_none());
    // An unreadable progress document voids every progress-derived claim:
    // the empty view would otherwise read as fresh-and-due, and any session
    // or exam it invites would mint fresh progress over the damaged file.
    let progress_error = store.progress_error(deck_id);
    DeckStatus {
        state,
        badge: if progress_error { String::new() } else { badge },
        locked,
        reviewable: reviewable && !progress_error,
        reviewable_recognize: reviewable_recognize && !progress_error,
        can_recognize,
        reviewable_recall: reviewable_recall && !progress_error,
        reviewable_reconstruct: reviewable_reconstruct && !progress_error,
        mastered: mastered && !progress_error,
        is_trace: deck.is_trace(),
        examable: examable && !progress_error,
        has_exam,
        badge_depth: badge_depth.filter(|_| !progress_error),
        badge_dotted: badge_dotted && !progress_error,
        new_cards: new_cards && !progress_error,
        progress_error,
    }
}

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

/// Each branch segment is exactly 3 chars wide, so a caller recovers nesting
/// depth as `prefix.chars().count() / 3`.
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
    use crate::scheduler::{Grade, Scheduler};

    const T0: u64 = 1_000_000;
    const MUCH_LATER: u64 = T0 + 30 * 86_400_000;

    fn write(path: &Path, text: &str) {
        let text = if path.extension().is_some_and(|extension| extension == "md")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(workspace::is_conventional_non_deck)
            && crate::parser::is_deck_content(text)
            && crate::parser::deck_identity(text).ok().flatten().is_none()
        {
            let id: String = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("deck")
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect();
            if let Some(rest) = text.strip_prefix("---\n") {
                format!("---\nformat-version: 1\nid: \"deck-{id}\"\n{rest}")
            } else {
                format!("---\nformat-version: 1\nid: \"deck-{id}\"\n---\n{text}")
            }
        } else {
            text.to_string()
        };
        std::fs::write(path, text).unwrap();
    }

    fn no_augment() -> AugmentCache {
        AugmentCache::open(Path::new("unused-generated.json"))
    }

    fn arm(augment: &mut AugmentCache, cards: &[Card]) {
        for card in cards {
            augment.set_distractors(
                &card.id().unwrap(),
                vec!["w1".into(), "w2".into(), "w3".into()],
                card.content_fingerprint,
            );
        }
    }

    #[test]
    fn a_trace_deck_lists_flagged_so_a_client_never_opens_a_doomed_review() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("walk.md"),
            "---\ntrace: How it flows\n---\n## hop? <!-- id: card-qhop -->\nstep\n",
        );
        write(
            &dir.path().join("facts.md"),
            "## q? <!-- id: card-qfacts -->\na\n",
        );
        let rows = list_root(dir.path(), &ReviewConfig::default(), T0);
        let flags: Vec<(&str, bool)> = rows
            .iter()
            .map(|r| (r.title.as_str(), r.is_trace))
            .collect();
        assert_eq!(vec![("facts", false), ("How it flows", true)], flags);
    }

    fn settle(user_root: &Path, deck_path: &Path) {
        let mut store = crate::state::open_store(deck_path, user_root).unwrap();
        let deck = Deck::load(deck_path).unwrap();
        let scheduler = Fsrs::default();
        for card in &deck.cards {
            let state = store.get_or_insert(&card.id().unwrap());
            scheduler.apply(state, Depth::Recognize, Grade::Pass, T0, false);
            scheduler.apply(state, Depth::Recall, Grade::Pass, T0, false);
            scheduler.apply(state, Depth::Reconstruct, Grade::Pass, T0, false);
        }
        store.save().unwrap();
    }

    #[test]
    fn lists_titles_and_kinds_and_skips_dot_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("b-loose.md"),
            "# Loose Deck\n## q <!-- id: card-qloose -->\na\n",
        );
        std::fs::create_dir_all(root.join("a-ws/decks")).unwrap();
        write(&root.join("a-ws/alix.toml"), "title = \"My Workspace\"\n");
        write(
            &root.join("a-ws/decks/m.md"),
            "## q <!-- id: card-qm -->\na\n",
        );
        std::fs::create_dir(root.join("c-plain")).unwrap();
        write(&root.join("c-plain/d.md"), "## q <!-- id: card-qd -->\na\n");
        std::fs::create_dir(root.join(".hidden")).unwrap();
        write(&root.join(".hidden/x.md"), "## q <!-- id: card-qx -->\na\n");
        write(&root.join("README.md"), "# A Readme\nprose\n");

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
    fn a_members_own_unreadable_document_reds_only_that_member() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
        write(&ws.join("decks").join("a.md"), "## qa\nans-a\n");
        write(&ws.join("decks").join("b.md"), "## qb\nans-b\n");
        let progress = workspace::store_path(&ws).join("progress");
        std::fs::create_dir_all(&progress).unwrap();
        std::fs::write(progress.join("deck-b.json"), "{ not json").unwrap();

        let rows = list_members(root, &ws, &ReviewConfig::default(), T0);
        let a = rows.iter().find(|r| r.title == "a").unwrap();
        let b = rows.iter().find(|r| r.title == "b").unwrap();
        assert!(
            !a.progress_error,
            "a sibling with a healthy document lists normally"
        );
        assert!(
            b.progress_error,
            "the member whose OWN document is unreadable reds out"
        );
        assert!(
            !b.due && !b.mastered && !b.exam_due,
            "an error row makes no progress claims"
        );
    }

    #[test]
    fn a_healthy_siblings_loaded_progress_survives_an_unreadable_document() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
        write(&ws.join("decks").join("a.md"), "## qa\nans-a\n");
        write(&ws.join("decks").join("b.md"), "## qb\nans-b\n");
        let progress = workspace::store_path(&ws).join("progress");
        std::fs::create_dir_all(&progress).unwrap();
        std::fs::write(
            progress.join("deck-a.json"),
            r#"{"version":1,"deck_id":"deck-a","subject":"a.md","revision":1,"cards":{},"deck":{"mastered_at_ms":1}}"#,
        )
        .unwrap();
        std::fs::write(progress.join("deck-b.json"), "{ not json").unwrap();

        let rows = list_members(root, &ws, &ReviewConfig::default(), T0);
        let a = rows.iter().find(|r| r.title == "a").unwrap();
        let b = rows.iter().find(|r| r.title == "b").unwrap();
        assert!(!a.progress_error);
        assert!(
            a.mastered,
            "the bad later document must not erase the healthy sibling's loaded progress"
        );
        assert!(b.progress_error);
    }

    #[test]
    fn a_deck_status_over_an_unreadable_document_voids_every_progress_claim() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("a.md"), "## qa <!-- id: card-qa1 -->\nans-a\n");
        let progress = root.join("progress");
        std::fs::create_dir_all(&progress).unwrap();
        std::fs::write(progress.join("deck-a.json"), "{ not json").unwrap();

        let store = crate::state::open_aggregate_store_tolerant(root).unwrap();
        let deck = Deck::load(root.join("a.md")).unwrap();
        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            true,
            ReviewConfig::default(),
        );

        assert!(status.progress_error);
        assert!(!status.reviewable, "an error status makes no due claim");
        assert!(!status.reviewable_recognize);
        assert!(!status.reviewable_recall);
        assert!(!status.reviewable_reconstruct);
        assert!(
            !status.new_cards,
            "a fabricated 'new' reading is still a progress claim"
        );
        assert!(status.badge.is_empty());
        assert!(status.badge_depth.is_none());
        assert!(!status.badge_dotted);
        assert!(!status.mastered);
        assert!(!status.examable);
    }

    #[test]
    fn an_existing_non_directory_progress_root_reds_every_member() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
        write(&ws.join("decks").join("a.md"), "## qa\nans-a\n");
        write(&ws.join("decks").join("b.md"), "## qb\nans-b\n");
        let store_root = workspace::store_path(&ws);
        std::fs::create_dir_all(&store_root).unwrap();
        std::fs::write(store_root.join("progress"), "not a directory").unwrap();

        let rows = list_members(root, &ws, &ReviewConfig::default(), T0);
        assert_eq!(2, rows.len());
        assert!(
            rows.iter().all(|row| row.progress_error),
            "an existing invalid store root is damage, not a fresh missing store: {rows:#?}"
        );
    }

    #[test]
    fn a_root_that_is_a_workspace_collapses_to_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("alix.toml"), "title = \"The Root\"\n");
        std::fs::create_dir(root.join("decks")).unwrap();
        write(&root.join("decks/a.md"), "## q <!-- id: card-qa -->\na\n");
        let rows = list_root(root, &ReviewConfig::default(), T0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "The Root");
        assert!(rows[0].is_workspace);
        assert!(rows[0].due, "a fresh deck has new cards");
        let members = list_members(root, root, &ReviewConfig::default(), T0);
        assert_eq!(members.len(), 1);
    }

    #[test]
    fn an_empty_initialized_workspace_is_still_a_workspace_row() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("alix.toml"), "title = \"The Root\"\n");

        let rows = list_root(root, &ReviewConfig::default(), T0);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "The Root");
        assert!(rows[0].is_workspace);
        assert!(!rows[0].due);
        assert!(list_members(root, root, &ReviewConfig::default(), T0).is_empty());
    }

    #[test]
    fn due_reads_each_entrys_own_store_and_the_injected_clock() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("loose.md"), "## q <!-- id: card-qloose -->\na\n");
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(&root.join("ws/alix.toml"), "");
        write(
            &root.join("ws/decks/m.md"),
            "## q <!-- id: card-qm -->\na\n",
        );

        settle(
            &workspace::store_path(&root.join("ws")),
            &root.join("ws/decks/m.md"),
        );
        settle(&workspace::root_store_path(root), &root.join("loose.md"));

        let just_after = list_root(root, &ReviewConfig::default(), T0 + 1_000);
        assert!(
            just_after.iter().all(|r| !r.due),
            "everything settled: {just_after:?}"
        );
        let later = list_root(root, &ReviewConfig::default(), MUCH_LATER);
        assert!(later.iter().all(|r| r.due), "schedules elapsed: {later:?}");
    }

    #[test]
    fn sync_conflicts_under_covers_the_root_and_workspace_stores() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("loose.md"), "## q <!-- id: card-qloose -->\na\n");
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(&root.join("ws/alix.toml"), "");
        write(
            &root.join("ws/decks/m.md"),
            "## q <!-- id: card-qm -->\na\n",
        );

        std::fs::create_dir(root.join("progress")).unwrap();
        std::fs::create_dir(root.join("augment")).unwrap();
        std::fs::create_dir(root.join("ws/progress")).unwrap();
        std::fs::create_dir(root.join("ws/augment")).unwrap();
        let root_conflict = root.join("progress/loose.sync-conflict-20260714-101112-AAAAAAA.json");
        let root_augment = root.join("augment/loose.sync-conflict-20260714-101112-DDDDDDD.json");
        let ws_conflict =
            root.join("ws/progress/member.sync-conflict-20260715-101112-BBBBBBB.json");
        let ws_augment = root.join("ws/augment/member.sync-conflict-20260715-101112-EEEEEEE.json");
        write(&root_conflict, "{}");
        write(&root_augment, "{}");
        write(&ws_conflict, "{}");
        write(&ws_augment, "{}");
        write(
            &root.join("ws/notes.sync-conflict-20260715-101112-CCCCCCC.json"),
            "{}",
        );

        let mut expected = vec![root_conflict, root_augment, ws_conflict, ws_augment];
        expected.sort();
        assert_eq!(sync_conflicts_under(root), expected);
        assert_eq!(
            sync_conflicts_under(&root.join("nowhere")),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn unreadable_entries_degrade_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("broken.md"), "## q with no answer\n");
        write(&root.join("ok.md"), "## q <!-- id: card-qok -->\na\n");
        let rows = list_root(root, &ReviewConfig::default(), T0);
        let broken = rows.iter().find(|r| r.title == "broken").expect("listed");
        assert!(!broken.due);
        assert!(rows.iter().any(|r| r.title == "ok" && r.due));
        assert_eq!(
            list_root(&root.join("nowhere"), &ReviewConfig::default(), T0),
            []
        );
    }

    #[test]
    fn dependency_forest_nests_dependents_under_prerequisites() {
        let names: Vec<String> = [
            "data-model",
            "lapses",
            "grading",
            "stability",
            "queue-building",
        ]
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
        let names = vec!["a".to_string(), "b".to_string()];
        let parent = vec![Some(1), Some(0)];
        let order = dependency_forest(&parent, &names);
        let mut indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        indices.sort();
        assert_eq!(vec![0, 1], indices);
    }

    #[test]
    fn listing_status_fields_match_deck_status_for_the_same_deck_and_store() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        write(&ws.join("alix.toml"), "title = \"WS\"\n");
        write(
            &ws.join("decks/base.md"),
            "---\nformat-version: 1\nid: \"deck-base1\"\nsource: https://x\n---\n## q <!-- id: card-qbase -->\na\n",
        );
        write(
            &ws.join("decks/advanced.md"),
            "---\nformat-version: 1\nid: \"deck-adv1\"\nrequires: base\n---\n## q2 <!-- id: card-qadv -->\nb\n",
        );
        write(
            &ws.join("decks/walk.md"),
            "---\nformat-version: 1\nid: \"deck-walk1\"\ntrace: How it flows\n---\n## hop? <!-- id: card-qhop -->\nstep\n",
        );

        let store_path = workspace::store_path(&ws);
        let base_id = Deck::load(ws.join("decks/base.md")).unwrap().cards[0]
            .id()
            .unwrap();
        let paths = workspace::deck_files(&ws);
        let mut store = crate::state::open_stores(&paths, &store_path).unwrap();
        store.get_or_insert(&base_id).recall = Some(graduated_not_due(T0));
        store.save().unwrap();
        let review = ReviewConfig::default();

        let assert_parity = |rows: &[DeckSummary], store: &Store| {
            assert_eq!(3, rows.len());
            for row in rows {
                let deck = Deck::load(&row.path).unwrap();
                let status = deck_status(&deck, store, &no_augment(), Some(root), true, review);
                assert_eq!(row.mastered, status.mastered, "mastered: {}", row.title);
                assert_eq!(
                    row.exam_due,
                    status.state == DeckState::ExamDue,
                    "exam_due: {}",
                    row.title
                );
                assert_eq!(row.has_exam, status.has_exam, "has_exam: {}", row.title);
                assert_eq!(row.locked, status.locked, "locked: {}", row.title);
                assert_eq!(row.is_trace, status.is_trace, "is_trace: {}", row.title);
                assert_eq!(
                    row.can_recognize, status.can_recognize,
                    "can_recognize: {}",
                    row.title
                );
            }
        };

        let rows = list_members(root, &ws, &review, T0 + 1_000);
        let store = crate::state::open_stores(&paths, &store_path).unwrap();
        assert_parity(&rows, &store);
        let base = rows.iter().find(|r| r.title == "base").unwrap();
        assert!(base.exam_due);
        assert!(!base.mastered);
        let advanced = rows.iter().find(|r| r.title == "advanced").unwrap();
        assert!(advanced.locked);
        let walk = rows.iter().find(|r| r.title == "How it flows").unwrap();
        assert!(walk.is_trace);
        assert!(walk.has_exam);

        let mut store = crate::state::open_stores(&paths, &store_path).unwrap();
        let base_deck = Deck::load(ws.join("decks/base.md")).unwrap();
        store.set_deck_mastered(base_deck.deck_token.as_deref().unwrap(), T0 + 1_000);
        store.save().unwrap();

        let rows = list_members(root, &ws, &review, T0 + 1_000);
        let store = crate::state::open_stores(&paths, &store_path).unwrap();
        assert_parity(&rows, &store);
        let base = rows.iter().find(|r| r.title == "base").unwrap();
        assert!(base.mastered);
        assert!(!base.exam_due);
        let advanced = rows.iter().find(|r| r.title == "advanced").unwrap();
        assert!(!advanced.locked, "base mastered: advanced should unlock");
    }

    #[test]
    fn last_depth_falls_back_to_default_then_remembers_after_being_set() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.md"), "## q <!-- id: card-qd -->\na\n");

        let rows = list_root(root, &ReviewConfig::default(), T0);
        let row = rows.iter().find(|r| r.title == "d").expect("listed");
        assert_eq!(Depth::default(), row.last_depth);

        let store_path = workspace::root_store_path(root);
        let mut store = crate::state::open_store(&root.join("d.md"), &store_path).unwrap();
        store.set_last_depth("deck-d", Depth::Reconstruct);
        store.save().unwrap();

        let rows = list_root(root, &ReviewConfig::default(), T0);
        let row = rows.iter().find(|r| r.title == "d").expect("listed");
        assert_eq!(Depth::Reconstruct, row.last_depth);
    }

    #[test]
    fn list_members_orders_and_indents_a_requires_chain_like_the_dependency_forest() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        write(&ws.join("alix.toml"), "");
        write(
            &ws.join("decks/base.md"),
            "## q <!-- id: card-qbase -->\na\n",
        );
        write(
            &ws.join("decks/mid.md"),
            "---\nrequires: base\n---\n## q <!-- id: card-qmid -->\na\n",
        );
        write(
            &ws.join("decks/tip.md"),
            "---\nrequires: mid\n---\n## q <!-- id: card-qtip -->\na\n",
        );
        write(
            &ws.join("decks/other.md"),
            "## q <!-- id: card-qother -->\na\n",
        );

        let rows = list_members(root, &ws, &ReviewConfig::default(), T0);
        let shape: Vec<(&str, usize, &str)> = rows
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
    fn list_members_sorts_an_exam_due_sibling_before_a_merely_locked_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("gate.md"),
            "---\nsource: https://x\n---\n## q <!-- id: card-qgate -->\na\n",
        );
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        write(&ws.join("alix.toml"), "");
        write(
            &ws.join("decks/aaa-locked.md"),
            "---\nrequires: gate\n---\n## q2 <!-- id: card-qlocked -->\nb\n",
        );
        write(
            &ws.join("decks/zzz-examdue.md"),
            "---\nsource: https://y\n---\n## q <!-- id: card-qexamdue -->\na\n",
        );

        let store_path = workspace::store_path(&ws);
        let examdue_id = Deck::load(ws.join("decks/zzz-examdue.md")).unwrap().cards[0]
            .id()
            .unwrap();
        let mut store =
            crate::state::open_store(&ws.join("decks/zzz-examdue.md"), &store_path).unwrap();
        let entry = store.get_or_insert(&examdue_id);
        entry.recognize = Some(graduated_not_due(T0));
        entry.recall = Some(graduated_not_due(T0));
        entry.reconstruct = Some(graduated_not_due(T0));
        store.save().unwrap();

        let rows = list_members(root, &ws, &ReviewConfig::default(), T0 + 1_000);
        assert_eq!(2, rows.len());
        let examdue = rows.iter().find(|r| r.title == "zzz-examdue").unwrap();
        assert!(examdue.exam_due, "should have graduated into ExamDue");
        assert!(!examdue.due, "settled at every depth: nothing due");
        assert!(!examdue.locked);
        let locked = rows.iter().find(|r| r.title == "aaa-locked").unwrap();
        assert!(locked.locked, "gated by the unmastered gate.md");

        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(
            vec!["zzz-examdue", "aaa-locked"],
            titles,
            "the startable exam-due deck must sort before the locked sibling"
        );
    }

    #[test]
    fn list_members_sorts_a_load_failed_member_as_not_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("gate.md"),
            "---\nsource: https://x\n---\n## q <!-- id: card-qgate -->\na\n",
        );
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        write(&ws.join("alix.toml"), "");
        write(&ws.join("decks/zzz-broken.md"), "## q with no answer\n");
        write(
            &ws.join("decks/aaa-locked.md"),
            "---\nrequires: gate\n---\n## q2 <!-- id: card-qlocked -->\nb\n",
        );

        let rows = list_members(root, &ws, &ReviewConfig::default(), T0);
        assert_eq!(2, rows.len());
        let locked = rows.iter().find(|r| r.title == "aaa-locked").unwrap();
        assert!(locked.locked, "gated by the unmastered gate.md");

        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(
            vec!["zzz-broken", "aaa-locked"],
            titles,
            "a load-failed member must sort as not-blocked, before a locked sibling"
        );
    }

    #[test]
    fn workspace_icon_resolves_only_for_workspace_rows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(&root.join("ws/alix.toml"), "");
        std::fs::create_dir_all(root.join("ws/assets")).unwrap();
        write(&root.join("ws/assets/icon.svg"), "<svg/>");
        write(
            &root.join("ws/decks/m.md"),
            "## q <!-- id: card-qm -->\na\n",
        );
        write(&root.join("loose.md"), "## q <!-- id: card-qloose -->\na\n");

        let rows = list_root(root, &ReviewConfig::default(), T0);
        let ws_row = rows.iter().find(|r| r.is_workspace).expect("listed");
        assert_eq!(Some(root.join("ws/assets/icon.svg")), ws_row.icon);
        let deck_row = rows.iter().find(|r| !r.is_workspace).expect("listed");
        assert_eq!(None, deck_row.icon);
    }

    fn graduated_not_due(now: u64) -> crate::store::FsrsState {
        crate::store::FsrsState {
            state: 2, // Review (graduated)
            scheduled_days: 30,
            due_ms: now + 30 * 86_400_000,
            ..Default::default()
        }
    }

    fn write_due_personal_card(store: &mut Store, deck: &Path, deck_id: &str) {
        let block = "## personal front <!-- id: card-vq1 -->\npersonal back\n";
        let id = crate::parser::parse_str(deck_id, block).unwrap()[0]
            .id()
            .unwrap();
        crate::personal::append_cards(deck, deck_id, block).unwrap();
        store.get_or_insert(&id).introduced_ms = Some(0);
    }

    #[test]
    fn deck_status_reviewable_true_when_only_a_personal_card_is_due() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "---\nformat-version: 1\nid: \"deck-rust\"\n---\n## q1 <!-- id: card-q1 -->\na1\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let now = session::now_ms();
        let card_id = deck.cards[0].id().unwrap();
        let entry = store.get_or_insert(&card_id);
        entry.recognize = Some(graduated_not_due(now));
        entry.recall = Some(graduated_not_due(now));
        entry.reconstruct = Some(graduated_not_due(now));

        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert_eq!("done ✓", status.badge);
        assert!(!status.reviewable);

        write_due_personal_card(&mut store, &deck_path, deck.deck_token.as_deref().unwrap());
        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert!(status.reviewable);
        assert_eq!("done ✓", status.badge);
    }

    #[test]
    fn an_initialized_deck_with_unstamped_cards_is_reviewable() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "---\nformat-version: 1\nid: \"deck-rust\"\n---\n## q1\na1\n## q2\na2\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        assert!(
            deck.cards.iter().all(|c| c.id().is_none()),
            "fixture must be unstamped"
        );

        let store = Store::open(dir.path().join("deck1.json")).unwrap();
        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert!(
            status.reviewable,
            "new cards in an initialized deck must be drillable"
        );
    }

    #[test]
    fn a_recall_settled_deck_is_still_reviewable_at_reconstruct() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(&deck_path, "## q1 <!-- id: card-q1 -->\na1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let now = session::now_ms();
        let card_id = deck.cards[0].id().unwrap();
        store.get_or_insert(&card_id).recall = Some(mature(now, 25.0));

        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert!(!status.reviewable_recall, "nothing due at recall");
        assert!(
            status.reviewable_reconstruct,
            "due now at reconstruct via the immediacy rule"
        );
        assert!(status.reviewable, "reviewable overall via reconstruct");
    }

    #[test]
    fn recognize_reviewability_tracks_unrecognized_recognizable_cards() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "## q1 <!-- id: card-q1 -->\na1\n## q2 <!-- id: card-q2 -->\na2\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let mut augment = AugmentCache::open(dir.path().join("deck1-generated.json"));
        arm(&mut augment, &deck.cards);
        let now = session::now_ms();
        store.get_or_insert(&deck.cards[0].id().unwrap()).recognize = Some(graduated_not_due(now));

        let status = deck_status(
            &deck,
            &store,
            &augment,
            None,
            false,
            ReviewConfig::default(),
        );
        assert!(status.reviewable_recognize);
        assert!(status.can_recognize);

        store.get_or_insert(&deck.cards[1].id().unwrap()).recognize = Some(graduated_not_due(now));
        let status = deck_status(
            &deck,
            &store,
            &augment,
            None,
            false,
            ReviewConfig::default(),
        );
        assert!(
            !status.reviewable_recognize,
            "every recognizable card is recognized"
        );
        assert!(status.can_recognize, "the deck can still be crammed");
    }

    #[test]
    fn an_unaugmented_deck_is_not_recognizable() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "## q1 <!-- id: card-q1 -->\na1\n## q2 <!-- id: card-q2 -->\na2\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let store = Store::open(dir.path().join("deck1.json")).unwrap();

        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert!(!status.reviewable_recognize);
        assert!(!status.can_recognize);
    }

    #[test]
    fn a_workspace_deadline_lists_with_live_readiness_and_a_plain_folder_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        write(&ws.join("alix.toml"), "title = \"WS\"\n");
        write(
            &ws.join("decks/done.md"),
            "## q <!-- id: card-qdone -->\na\n",
        );
        write(
            &ws.join("decks/fresh.md"),
            "## q2 <!-- id: card-qfresh -->\nb\n",
        );
        let date = crate::time::local_date(T0) + chrono::Days::new(5);
        write(
            &ws.join("alix.local.toml"),
            &format!("[review]\ndeadline = \"{}\"\n", date.format("%Y-%m-%d")),
        );
        let done_id = Deck::load(ws.join("decks/done.md")).unwrap().cards[0]
            .id()
            .unwrap();
        let mut store =
            crate::state::open_store(&ws.join("decks/done.md"), &workspace::store_path(&ws))
                .unwrap();
        store.get_or_insert(&done_id).recall = Some(graduated_not_due(T0));
        store.save().unwrap();

        let review = ReviewConfig::default();
        let rows = list_root(root, &review, T0);
        let ws_row = rows.iter().find(|r| r.is_workspace).expect("listed");
        let deadline = ws_row.deadline.as_ref().expect("a set deadline lists");
        assert_eq!(date, deadline.date);
        assert_eq!(5, deadline.days_left);
        assert_eq!((1, 2), (deadline.ready, deadline.total));

        let fetched = workspace_deadline(root, &ws, &review, T0).expect("fetchable");
        assert_eq!(Some(fetched), ws_row.deadline);

        let plain = root.join("plain");
        std::fs::create_dir(&plain).unwrap();
        write(&plain.join("d.md"), "## q <!-- id: card-qd -->\na\n");
        write(
            &plain.join("alix.local.toml"),
            "[review]\ndeadline = \"2027-01-01\"\n",
        );
        let rows = list_root(root, &review, T0);
        let plain_row = rows
            .iter()
            .find(|r| r.path.file_name().is_some_and(|n| n == "plain"))
            .expect("listed");
        assert_eq!(None, plain_row.deadline);
    }

    #[test]
    fn deck_summary_can_recognize_tracks_augmentation() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("d.md");
        std::fs::write(&deck_path, "## q1 <!-- id: card-q1 -->\na1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let store = Store::open(dir.path().join("deck1.json")).unwrap();
        let review = ReviewConfig::default();
        let now = session::now_ms();

        let healthy = ProgressHealth::PerDeck(std::collections::HashSet::new());
        let (bare, _) = deck_summary(
            &deck_path,
            Some(&store),
            &healthy,
            Some(&no_augment()),
            dir.path(),
            &review,
            now,
        );
        assert!(!bare.can_recognize, "un-augmented deck is not recognizable");

        let mut augment = AugmentCache::open(dir.path().join("deck1-generated.json"));
        arm(&mut augment, &deck.cards);
        let (armed, _) = deck_summary(
            &deck_path,
            Some(&store),
            &healthy,
            Some(&augment),
            dir.path(),
            &review,
            now,
        );
        assert!(
            armed.can_recognize,
            "cached distractors make it recognizable"
        );
    }

    #[test]
    fn deck_due_does_not_over_report_recognize_on_an_unaugmented_deck() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(&deck_path, "## q1 <!-- id: card-q1 -->\na1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let now = session::now_ms();
        let entry = store.get_or_insert(&deck.cards[0].id().unwrap());
        entry.recall = Some(graduated_not_due(now));
        entry.reconstruct = Some(graduated_not_due(now));

        assert!(
            !deck_due(&deck, &store, &no_augment(), &ReviewConfig::default(), now),
            "un-augmented settled deck is not due"
        );

        let mut augment = AugmentCache::open(dir.path().join("deck1-generated.json"));
        arm(&mut augment, &deck.cards);
        assert!(
            deck_due(&deck, &store, &augment, &ReviewConfig::default(), now),
            "an augmented unrecognized card is due at Recognize"
        );
    }

    /// A personal file's own cards are the reader's, and a deck whose authored
    /// cards have all settled is still due while one of them is unseen.
    #[test]
    fn deck_due_counts_a_card_that_lives_only_in_the_personal_file() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "---\nformat-version: 1\nid: \"deck-r1\"\n---\n## q1 <!-- id: card-q1 -->\na1\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let now = session::now_ms();
        let entry = store.get_or_insert(&deck.cards[0].id().unwrap());
        entry.recall = Some(graduated_not_due(now));
        entry.reconstruct = Some(graduated_not_due(now));

        assert!(
            !deck_due(&deck, &store, &no_augment(), &ReviewConfig::default(), now),
            "the authored card has settled"
        );

        crate::personal::append_cards(
            &deck_path,
            "deck-r1",
            "## mine <!-- id: card-mine1 -->\nmy answer\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        assert!(
            deck_due(&deck, &store, &no_augment(), &ReviewConfig::default(), now),
            "a card of the reader's own is still waiting"
        );
    }

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
    fn the_highest_currently_solid_depth_wins_the_badge() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(&deck_path, "## q1 <!-- id: card-q1 -->\na1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let now = session::now_ms();
        let card_id = deck.cards[0].id().unwrap();
        let entry = store.get_or_insert(&card_id);
        entry.recall = Some(mature(now, 25.0));
        entry.reconstruct = Some(mature(now, 30.0));

        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert_eq!(Some(Depth::Reconstruct), status.badge_depth);
        assert!(!status.badge_dotted);
    }

    #[test]
    fn an_earned_but_lapsed_badge_shows_dotted() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "---\nformat-version: 1\nid: \"deck-rust\"\n---\n## q1 <!-- id: card-q1 -->\na1\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let now = session::now_ms();
        let card_id = deck.cards[0].id().unwrap();
        store.get_or_insert(&card_id).recall = Some(mature(now, 25.0));
        crate::store::record_badges(
            &mut store,
            deck.deck_token.as_deref().unwrap(),
            &deck.cards,
            now,
        );

        store.get_or_insert(&card_id).recall = Some(mature(now, 5.0));

        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert_eq!(Some(Depth::Recall), status.badge_depth);
        assert!(status.badge_dotted);
    }

    #[test]
    fn a_not_started_deck_has_an_empty_badge() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "## q1 <!-- id: card-q1 -->\na1\n## q2 <!-- id: card-q2 -->\na2\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let store = Store::open(dir.path().join("deck1.json")).unwrap();

        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert_eq!(DeckState::NotStarted, status.state);
        assert_eq!(
            "", status.badge,
            "the NEW chip is the single new-signal; the badge slot stays empty"
        );
        assert!(status.new_cards);
    }

    #[test]
    fn new_cards_flag_fires_without_touching_badges() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "## q1 <!-- id: card-q1 -->\na1\n## q2 <!-- id: card-q2 -->\na2\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let now = session::now_ms();
        store.get_or_insert(&deck.cards[0].id().unwrap()).recall = Some(graduated_not_due(now));

        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert!(status.new_cards);
        assert_eq!(
            "", status.badge,
            "a started deck's row no longer carries a k/N counter"
        );
        assert_eq!(None, status.badge_depth);
        assert!(!status.badge_dotted);
    }
    #[test]
    fn a_listing_scan_ignores_deck_like_prose_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.md");
        let body = "# Notes\n\n## Design\nordinary prose\n";
        std::fs::write(&notes, body).unwrap();
        let summaries = list_root(dir.path(), &ReviewConfig::default(), T0);
        assert!(summaries.is_empty());
        assert_eq!(body, std::fs::read_to_string(&notes).unwrap());
    }
}
