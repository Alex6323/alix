//! A minimal deck lister for folder-picking clients (the frb mobile app):
//! what is in a decks folder, with a title and a due-now signal per entry.
//! The core sibling of the gated picker's catalog, deliberately without its
//! recency ordering and badges; scan rules mirror `picker::dir_candidates`
//! so both surfaces agree on what a folder contains. The store-derived deck
//! status ([`deck_status`]) and workspace dependency-layout helpers
//! ([`member_parents`], [`dependency_forest`]) live here too, moved from
//! `picker` (which re-exports them for the web picker) so the lean mobile
//! build can use them — and `DeckSummary`'s own `mastered`/`exam_due`/
//! `has_exam`/`locked`/`is_trace` are now built from those same helpers, one
//! reconciled truth (pinned by a parity test against `deck_status`). `due`
//! remains this module's own simpler signal via [`deck_due`], not fully
//! reconciled with `deck_status.reviewable` (it drops the trace/exam special
//! cases the mobile client doesn't open yet), but its Recognize share now
//! matches `reviewable_recognize`'s pick-only rule.
use std::path::{Path, PathBuf};

use crate::{
    augment::{self, AugmentCache},
    card::Card,
    config::ReviewConfig,
    deck::{self, Deck, DeckState},
    depth::{self, Depth},
    scheduler::Fsrs,
    session,
    store::{self, Store},
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
    /// The deck has at least one recognizable card ([`depth::deck_recognizable`])
    /// — cached distractors that build a pick. The client gates the Recognize
    /// depth on this: Recognize is pick-only, so an un-augmented deck greys it
    /// out. Deck rows only; `false` for workspace/folder rows or a missing
    /// store/augment. Mirrors [`DeckStatus::can_recognize`].
    pub can_recognize: bool,
    /// The deck counts as ready toward its workspace's deadline
    /// ([`deadline_ready`]): mastered, or finished with no exam to pass. Deck
    /// rows only; `false` for workspace/folder rows or a missing store.
    pub ready: bool,
    /// The workspace's "ready by" target with live readiness ({#deadlines}),
    /// mirroring the web catalog's `DeadlineDto`. Real-workspace rows only
    /// (`alix.local.toml` present with a `deadline`); `None` for deck rows,
    /// plain folders, and workspaces without one.
    pub deadline: Option<DeckDeadline>,
    /// A trace deck (`% trace:`): a predict-and-verify walk, not a card
    /// review. A client dispatches on this flag: the phone opens a trace row
    /// as a walk (`WalkSession`), never as a review session.
    pub is_trace: bool,
    /// The session depth remembered for this deck (`Store::last_depth`), else
    /// the deck's fresh-session default (`depth::default_depth`). Deck rows
    /// only; `Depth::default()` for workspace/folder rows, an unreadable
    /// deck, or a missing store.
    pub last_depth: Depth,
    /// Finished *and* exam-passed (`Store::deck_mastered`). Deck rows only;
    /// `false` for workspace/folder rows, an unreadable deck, or a missing
    /// store.
    pub mastered: bool,
    /// Drilled and awaiting its AI exam (`DeckState::ExamDue`). Deck rows
    /// only; `false` for workspace/folder rows, an unreadable deck, or a
    /// missing store.
    pub exam_due: bool,
    /// The deck has an AI exam at all (`Deck::has_exam`) — a sourced fact
    /// deck, or a trace. Deck rows only; `false` for workspace/folder rows
    /// or an unreadable deck (needs no store).
    pub has_exam: bool,
    /// A `% requires:` prerequisite isn't finished (`deck::is_locked`) — the
    /// fact of the lock, unconditional; display gating is the client's
    /// choice. Deck rows only; `false` for workspace/folder rows, an
    /// unreadable deck, or a missing store.
    pub locked: bool,
    /// The workspace's resolved picker icon file (manifest `icon`, else a
    /// conventional `assets/icon.*`). Workspace/folder rows only; `None` for
    /// deck rows and when nothing resolves.
    pub icon: Option<PathBuf>,
    /// Nesting depth in the workspace's dependency tree. Member rows only;
    /// `0` for loose-deck and workspace/folder rows.
    pub indent: usize,
    /// The branch-line prefix (`├─`/`└─`/`│`) for this row in the
    /// workspace's dependency tree. Member rows only; empty for loose-deck
    /// and workspace/folder rows.
    pub tree: String,
}

/// A workspace's "ready by" target ({#deadlines}) as a client renders it:
/// the date, how far off it is, and how many member decks are ready — the
/// lean sibling of the web catalog's `DeadlineDto`, computed from the same
/// [`deadline_ready`] rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckDeadline {
    /// The target date (`[review] deadline` in the workspace's
    /// `alix.local.toml`).
    pub date: chrono::NaiveDate,
    /// Whole days until the date in local time; negative once it has passed.
    pub days_left: i64,
    /// Member decks currently ready ([`deadline_ready`]).
    pub ready: usize,
    /// Member decks counted (loadable ones, matching the web catalog).
    pub total: usize,
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
    // Opened once for the whole root, same as the web picker's catalog pass.
    let augment = root_store
        .as_ref()
        .map(|s| AugmentCache::open(augment::augment_path_for(s.path())));
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
            out.push(
                deck_summary(
                    &path,
                    root_store.as_ref(),
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

/// Lists the decks inside one drillable folder of `root`. Members review
/// into the folder's own store when it is a workspace (`alix.toml`), else
/// into the root's shared store, the same routing `assemble::store_for`
/// applies when one of them is opened. Ordered as a dependency forest —
/// each member nested under the `% requires:` that gates it, siblings
/// startable-first — mirroring the web picker's `workspace_members`.
pub fn list_members(
    root: &Path,
    dir: &Path,
    review: &ReviewConfig,
    now_ms: u64,
) -> Vec<DeckSummary> {
    let (paths, rows) = member_rows(root, dir, review, now_ms);
    // Sibling key mirrors catalog.rs's `blocked = locked || (with_lock &&
    // !reviewable)` exactly (`with_lock` is unconditionally true here, a
    // locked deck is never startable-first in a drill-in list): `reviewable`
    // expands to `is_trace || (exam_due && has_exam && !locked) || due`,
    // matching `deck_status`'s trace/exam-due special cases plus the
    // per-depth OR that `due` already covers. A deck that failed to load
    // sorts as NOT blocked, mirroring catalog's `Option<DeckStatus>::None`
    // arm (`is_some_and` is false for `None`).
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

/// One folder's member rows in manifest order, each with whether its deck
/// file parsed — the shared first half of [`list_members`], kept separate so
/// [`folder_summary`] and [`workspace_deadline`] can count readiness over
/// loadable members only (a failed parse contributes to neither `ready` nor
/// `total`, mirroring the web catalog's `Option<DeckStatus>::None` arm).
fn member_rows(
    root: &Path,
    dir: &Path,
    review: &ReviewConfig,
    now_ms: u64,
) -> (Vec<PathBuf>, Vec<(DeckSummary, bool)>) {
    let store = member_store(root, dir);
    // Opened once for the whole folder, same as the web picker's catalog pass.
    let augment = store
        .as_ref()
        .map(|s| AugmentCache::open(augment::augment_path_for(s.path())));
    let paths: Vec<PathBuf> = match workspace::Workspace::load(dir) {
        Ok(ws) => ws.members,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    let rows: Vec<(DeckSummary, bool)> = paths
        .iter()
        .map(|m| deck_summary(m, store.as_ref(), augment.as_ref(), root, review, now_ms))
        .collect();
    (paths, rows)
}

/// The workspace's "ready by" target with live readiness ({#deadlines}), for
/// a client's chip or drill-in header. `None` for a plain folder (only a real
/// workspace carries a deadline) or when none is set. Fetchable on its own so
/// a drilled-in client can refresh the readout after reviews change mastery,
/// without re-listing the whole root.
pub fn workspace_deadline(
    root: &Path,
    dir: &Path,
    review: &ReviewConfig,
    now_ms: u64,
) -> Option<DeckDeadline> {
    let rows = member_rows(root, dir, review, now_ms).1;
    deadline_for(dir, &rows, review, now_ms)
}

/// [`workspace_deadline`] over already-listed member rows, so
/// [`folder_summary`] doesn't list the folder twice.
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

/// Syncthing conflict copies next to any store `root` reviews into: the
/// root's shared store plus each workspace member's own. Non-empty means two
/// devices wrote concurrently; clients surface it so the user resolves the
/// fork before reviewing on top of it.
pub fn sync_conflicts_under(root: &Path) -> Vec<PathBuf> {
    let mut out = store::sync_conflicts(&workspace::root_store_path(root));
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
        }
    }
    out
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
    let ws = workspace::Workspace::load(dir).ok();
    let title = ws.as_ref().map(|ws| ws.display_name()).unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let icon = ws.and_then(|ws| ws.icon);
    // A group row aggregates its members, like the web catalog's group DTO.
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
        icon,
        indent: 0,
        tree: String::new(),
    }
}

/// A loose deck's summary row: title/due/trace as before, plus the
/// store-derived status fields, one truth with [`deck_status`] (pinned by a
/// parity test). `decks_dir` roots `% requires:` lock resolution, mirroring
/// what the web picker's catalog passes (the served root for a loose deck,
/// the same root `member_parents` gets for a workspace member). `augment` is
/// opened once per store by the caller, like the web picker's catalog pass.
/// The returned `bool` is whether `Deck::load` succeeded — `list_members`
/// needs it (unlike any `DeckSummary` field) to sort a load-failed member as
/// NOT blocked, mirroring catalog's `Option<DeckStatus>::None` arm.
fn deck_summary(
    path: &Path,
    store: Option<&Store>,
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
    // `augment` is `Some` exactly when `store` is (both from one `*_store`),
    // so gating on all three keeps the same None-ness while giving `deck_due`
    // the cache it needs to judge Recognize.
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
            let mastered = s.deck_mastered(&d.subject);
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
            .last_depth(&d.subject)
            .unwrap_or_else(|| depth::default_depth(&d.cards, a)),
        _ => Depth::default(),
    };
    let row = DeckSummary {
        title: if title.is_empty() { stem } else { title },
        path: path.to_path_buf(),
        is_workspace: false,
        due,
        can_recognize,
        ready,
        deadline: None,
        is_trace,
        last_depth,
        mastered,
        exam_due,
        has_exam,
        locked,
        icon: None,
        indent: 0,
        tree: String::new(),
    };
    (row, loaded)
}

/// Anything to launch right now at any depth: the web picker's aggregate
/// (`picker::deck_status`) minus its trace and exam special cases, which the
/// mobile client does not open yet. Recognize is pick-only, so its share of the
/// OR mirrors `deck_status`'s `reviewable_recognize` — a card that is both
/// servable at Recognize AND recognizable — never the bare unrecognized check,
/// which would over-report an un-augmented deck as due (empty on tap).
fn deck_due(
    deck: &Deck,
    store: &Store,
    augment: &AugmentCache,
    review: &ReviewConfig,
    now_ms: u64,
) -> bool {
    let scheduler = Fsrs::new(review.retention, review.acquire_cooldown_ms);
    let retire = review.retire_after_days;
    let recognize_due = deck.cards.iter().any(|c| {
        session::is_reviewable(c, store, &scheduler, Depth::Recognize, now_ms, retire)
            && depth::card_recognizable(c, augment)
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
        || session::has_reviewable_virtual(store, &deck.subject, &scheduler, now_ms, retire)
}

// ---- deck status and dependency layout (shared with the web picker) -----

/// One deck's readiness toward its workspace's deadline ({#deadlines} spec
/// decision 2): mastered (exam passed), or, for a source-less deck (no exam to
/// pass), simply finished drilling. The single rule behind
/// [`workspace_readiness`], `DeckSummary::ready`, and the web catalog's
/// deadline readout — keep them one truth.
pub fn deadline_ready(mastered: bool, finished: bool, has_exam: bool) -> bool {
    mastered || (finished && !has_exam)
}

/// A workspace's progress toward its deadline ({#deadlines}): how many member
/// decks count as ready, out of how many total.
pub struct WorkspaceReadiness {
    pub ready: usize,
    pub total: usize,
}

/// Counts a workspace's member decks as ready for its deadline
/// ([`deadline_ready`] per member). Moved from `picker` (which re-exports it)
/// so the lean mobile build shares the rule.
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

/// The store-derived status of a deck, computed once so the web deck-selection
/// screen (and any thin client consuming the same API) shows the same badge,
/// lock, and gating.
#[derive(Clone)]
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
    /// non-depth trace/exam-due special cases (untouched by the per-depth
    /// split) — "anything to do, at any depth".
    pub reviewable: bool,
    /// A card that is BOTH not yet correctly picked at Recognize
    /// (`recognized_ms` unset) AND recognizable (its deck has cached distractors
    /// that build a pick — see [`depth::card_recognizable`]). Recognize is
    /// unscheduled and pick-only, so this is the honest per-card conjunction, not
    /// a due time: a card without a buildable pick is never served at Recognize.
    pub reviewable_recognize: bool,
    /// The deck has at least one recognizable card ([`depth::deck_recognizable`])
    /// — it can be drilled at Recognize at all, cached distractors permitting.
    /// Distinct from [`reviewable_recognize`](Self::reviewable_recognize) (which
    /// also requires a card still unrecognized): the picker uses this to keep the
    /// Recognize depth selectable under **cram** (which re-serves recognized
    /// cards too), and to grey it out entirely on an un-augmented deck.
    pub can_recognize: bool,
    /// A card is due (or fresh) at Recall right now, or a virtual
    /// (remediation) card is due — what `reviewable` used to mean, entirely.
    pub reviewable_recall: bool,
    /// A non-retired card is due at Reconstruct right now, via the
    /// depth-aware scheduler. The cross-depth immediacy rule (`Fsrs::due_at`)
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
    /// The highest depth with a badge to show, walking `[Reconstruct, Recall,
    /// Recognize]` high to low: the first depth currently solid
    /// ([`store::badge_solid`]) wins (subsumption — a higher badge implies the
    /// lower checks were passed too); else the first depth with an earn date
    /// ([`Store::badge_earned`]) wins; else `None`. Additive telemetry —
    /// gates nothing.
    pub badge_depth: Option<Depth>,
    /// `true` when `badge_depth` was won by an earn date rather than current
    /// solidity — the badge lapsed (e.g. stability dropped) and should render
    /// dotted rather than solid.
    pub badge_dotted: bool,
    /// Any deck card has no store entry at all (never reviewed) — fresh
    /// material, distinct from `state`/`badge`.
    pub new_cards: bool,
}

/// The subsumption walk (spec §4.4, `{#check-matrix}`): the highest depth
/// that's currently solid wins with `dotted=false`; else the highest with an
/// earn date wins with `dotted=true`; else `(None, false)`.
fn badge_depth_for(subject: &str, cards: &[Card], store: &Store) -> (Option<Depth>, bool) {
    const DEPTHS: [Depth; 3] = [Depth::Reconstruct, Depth::Recall, Depth::Recognize];
    if let Some(depth) = DEPTHS
        .into_iter()
        .find(|&depth| store::badge_solid(cards, store, depth))
    {
        return (Some(depth), false);
    }
    let earned = DEPTHS
        .into_iter()
        .find(|&depth| store.badge_earned(subject, depth).is_some());
    let dotted = earned.is_some();
    (earned, dotted)
}

/// Computes a deck's [`DeckStatus`] from the progress `store`, mirroring what a
/// review session would see. `decks_dir` roots `% requires:` resolution for the
/// lock check; `enforce_locks` is false for browse (any deck is browsable).
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
    // Per-depth due-ness (spec `{#check-matrix}`): each depth's own honest
    // signal, via the depth-aware scheduler. Recognize needs any card not yet
    // correctly picked; Recall/Reconstruct each read their own independent
    // schedule — the scheduler's cross-depth immediacy rule (`Fsrs::due_at`)
    // is what makes a Recall-settled deck due right now at Reconstruct too.
    let scheduler = crate::scheduler::Fsrs::new(review.retention, review.acquire_cooldown_ms);
    let now = session::now_ms();
    // Recognize is pick-only: a card counts only if it is BOTH still unrecognized
    // AND recognizable (buildable pick). The conjunction lives per card — a deck
    // with one unrecognized-but-unaugmented card and one recognized-and-augmented
    // card has nothing to serve at Recognize, and must read as such.
    let reviewable_recognize = deck.cards.iter().any(|card| {
        session::is_reviewable(
            card,
            store,
            &scheduler,
            Depth::Recognize,
            now,
            review.retire_after_days,
        ) && depth::card_recognizable(card, augment)
    });
    let can_recognize = depth::deck_recognizable(&deck.cards, augment);
    let reviewable_recall = session::has_reviewable(
        &deck.cards,
        store,
        &scheduler,
        Depth::Recall,
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
        Depth::Reconstruct,
        now,
        review.retire_after_days,
    );
    // Is there anything to launch right now, at any depth? A trace always
    // walks; an exam-due deck launches its exam (only when its exam isn't
    // locked) — both non-depth special cases, unchanged by the per-depth
    // split above; otherwise it's whichever depth currently has something due
    // (or new, or a due virtual card). Drilling is never gated by the lock —
    // a prerequisite-locked deck with due cards is still reviewable.
    let reviewable = deck.is_trace()
        || (matches!(state, DeckState::ExamDue) && examable)
        || reviewable_recognize
        || reviewable_recall
        || reviewable_reconstruct;
    let (badge_depth, badge_dotted) = badge_depth_for(&deck.subject, &deck.cards, store);
    let new_cards = deck.cards.iter().any(|card| store.get(card.id()).is_none());
    DeckStatus {
        state,
        badge,
        locked,
        reviewable,
        reviewable_recognize,
        can_recognize,
        reviewable_recall,
        reviewable_reconstruct,
        mastered,
        is_trace: deck.is_trace(),
        examable,
        has_exam,
        badge_depth,
        badge_dotted,
        new_cards,
    }
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
/// indent is `prefix.chars().count() / 3`.
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
/// child, `is_root` whether it sits at the top depth (no branch glyph).
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
    /// Far past any first-Good learning interval, so everything is due again.
    const MUCH_LATER: u64 = T0 + 30 * 86_400_000;

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    /// An empty augment cache, for `deck_status` tests that don't exercise the
    /// Recognize depth (no card is recognizable, so `reviewable_recognize` and
    /// `can_recognize` are both false — which those tests never assert on).
    fn no_augment() -> AugmentCache {
        AugmentCache::open(Path::new("unused-augment.json"))
    }

    /// Caches a full set of choice distractors on every card, so the deck's
    /// cards become recognizable (a Recognize pick can be built).
    fn arm(augment: &mut AugmentCache, cards: &[Card]) {
        for card in cards {
            augment.set_distractors(card.id(), vec!["w1".into(), "w2".into(), "w3".into()]);
        }
    }

    #[test]
    fn a_trace_deck_lists_flagged_so_a_client_never_opens_a_doomed_review() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("walk.txt"),
            "% trace: How it flows\n# hop?\n\tstep\n",
        );
        write(&dir.path().join("facts.txt"), "# q?\n\ta\n");
        let rows = list_root(dir.path(), &ReviewConfig::default(), T0);
        let flags: Vec<(&str, bool)> = rows
            .iter()
            .map(|r| (r.title.as_str(), r.is_trace))
            .collect();
        assert_eq!(vec![("facts", false), ("How it flows", true)], flags);
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
    fn sync_conflicts_under_covers_the_root_and_workspace_stores() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("loose.txt"), "# q\n\ta\n");
        std::fs::create_dir(root.join("ws")).unwrap();
        write(&root.join("ws/alix.toml"), "");
        write(&root.join("ws/m.txt"), "# q\n\ta\n");

        let root_conflict = root.join("progress.sync-conflict-20260714-101112-AAAAAAA.json");
        let ws_conflict = root.join("ws/progress.sync-conflict-20260715-101112-BBBBBBB.json");
        write(&root_conflict, "{}");
        write(&ws_conflict, "{}");
        // Wrong stem: not a store conflict, must not be reported.
        write(
            &root.join("ws/notes.sync-conflict-20260715-101112-CCCCCCC.json"),
            "{}",
        );

        assert_eq!(sync_conflicts_under(root), vec![root_conflict, ws_conflict]);
        assert_eq!(
            sync_conflicts_under(&root.join("nowhere")),
            Vec::<PathBuf>::new()
        );
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

    #[test]
    fn dependency_forest_nests_dependents_under_prerequisites() {
        // 0 data-model (root); 1 lapses, 3 stability, 4 queue-building require 0;
        // 2 grading requires 1. Siblings order by name.
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
        // 0 and 1 require each other: no roots, but every node still appears once.
        let names = vec!["a".to_string(), "b".to_string()];
        let parent = vec![Some(1), Some(0)];
        let order = dependency_forest(&parent, &names);
        let mut indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        indices.sort();
        assert_eq!(vec![0, 1], indices);
    }

    // ---- parity: DeckSummary's status fields pinned to deck_status --------

    #[test]
    fn listing_status_fields_match_deck_status_for_the_same_deck_and_store() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "title = \"WS\"\n");
        // base: sourced (has an exam), no prerequisite — settled to ExamDue.
        write(&ws.join("base.txt"), "% source: https://x\n# q\n\ta\n");
        // advanced: source-less, requires base — locked while base isn't Finished.
        write(&ws.join("advanced.txt"), "% requires: base\n# q2\n\tb\n");
        // walk: a trace deck (its own kind of exam), independent of the chain.
        write(
            &ws.join("walk.txt"),
            "% trace: How it flows\n# hop?\n\tstep\n",
        );

        // Graduate base's one card (Recall reaches FSRS Review) without a
        // real drill session, so `deck.state` reads `ExamDue`.
        let store_path = workspace::store_path(&ws);
        let base_id = Deck::load(ws.join("base.txt")).unwrap().cards[0].id();
        let mut store = Store::open(&store_path).unwrap();
        store.get_or_insert(base_id, T0).recall = Some(graduated_not_due(T0));
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

        // Before base is mastered: base is exam-due, advanced is locked behind it.
        let rows = list_members(root, &ws, &review, T0 + 1_000);
        let store = Store::open(&store_path).unwrap();
        assert_parity(&rows, &store);
        let base = rows.iter().find(|r| r.title == "base").unwrap();
        assert!(base.exam_due);
        assert!(!base.mastered);
        let advanced = rows.iter().find(|r| r.title == "advanced").unwrap();
        assert!(advanced.locked);
        let walk = rows.iter().find(|r| r.title == "How it flows").unwrap();
        assert!(walk.is_trace);
        assert!(walk.has_exam);

        // Master base: it unlocks advanced, and every field must still agree.
        let mut store = Store::open(&store_path).unwrap();
        let base_deck = Deck::load(ws.join("base.txt")).unwrap();
        store.set_deck_mastered(&base_deck.subject, T0 + 1_000);
        store.save().unwrap();

        let rows = list_members(root, &ws, &review, T0 + 1_000);
        let store = Store::open(&store_path).unwrap();
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
        write(&root.join("d.txt"), "# q\n\ta\n");

        let rows = list_root(root, &ReviewConfig::default(), T0);
        let row = rows.iter().find(|r| r.title == "d").expect("listed");
        assert_eq!(Depth::default(), row.last_depth);

        let store_path = workspace::root_store_path(root);
        let mut store = Store::open(&store_path).unwrap();
        store.set_last_depth("d.txt", Depth::Reconstruct);
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
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "");
        write(&ws.join("base.txt"), "# q\n\ta\n");
        write(&ws.join("mid.txt"), "% requires: base\n# q\n\ta\n");
        write(&ws.join("tip.txt"), "% requires: mid\n# q\n\ta\n");
        write(&ws.join("other.txt"), "# q\n\ta\n");

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
        // Sits outside the workspace at `root`, so it gates `aaa-locked.txt`
        // without becoming its dependency-forest parent (it isn't a member
        // of the workspace's own listing) — an unmastered sourced deck, so
        // it has an exam that isn't yet passed.
        write(&root.join("gate.txt"), "% source: https://x\n# q\n\ta\n");
        let ws = root.join("ws");
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "");
        // Named to sort BEFORE the exam-due deck alphabetically, so a plain
        // name sort (ignoring `blocked`) would rank "aaa-locked" first —
        // only the fixed `blocked` key correctly ranks the startable
        // exam-due deck ahead of it.
        write(
            &ws.join("aaa-locked.txt"),
            "% requires: gate.txt\n# q2\n\tb\n",
        );
        write(
            &ws.join("zzz-examdue.txt"),
            "% source: https://y\n# q\n\ta\n",
        );

        // Graduate zzz-examdue's card at Recall and settle it at every depth
        // (no real drill session), so `due` reads false while `state` reads
        // `ExamDue` — the case the web treats as startable but the old
        // `locked || !due` key sorted as blocked.
        let store_path = workspace::store_path(&ws);
        let examdue_id = Deck::load(ws.join("zzz-examdue.txt")).unwrap().cards[0].id();
        let mut store = Store::open(&store_path).unwrap();
        let entry = store.get_or_insert(examdue_id, T0);
        entry.recognized_ms = Some(T0);
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
        assert!(locked.locked, "gated by the unmastered gate.txt");

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
        write(&root.join("gate.txt"), "% source: https://x\n# q\n\ta\n");
        let ws = root.join("ws");
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "");
        // A cloze with no holes fails to parse: `Deck::load` errors, so this
        // member degrades (see `unreadable_entries_degrade_instead_of_failing`).
        // Named to sort AFTER the locked deck alphabetically, so only the
        // fixed `blocked` key (a load failure is not-blocked) puts it first.
        write(
            &ws.join("zzz-broken.txt"),
            "# q\n\t% reveal: cloze\n\tno holes here\n",
        );
        write(
            &ws.join("aaa-locked.txt"),
            "% requires: gate.txt\n# q2\n\tb\n",
        );

        let rows = list_members(root, &ws, &ReviewConfig::default(), T0);
        assert_eq!(2, rows.len());
        let locked = rows.iter().find(|r| r.title == "aaa-locked").unwrap();
        assert!(locked.locked, "gated by the unmastered gate.txt");

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
        std::fs::create_dir(root.join("ws")).unwrap();
        write(&root.join("ws/alix.toml"), "");
        std::fs::create_dir_all(root.join("ws/assets")).unwrap();
        write(&root.join("ws/assets/icon.svg"), "<svg/>");
        write(&root.join("ws/m.txt"), "# q\n\ta\n");
        write(&root.join("loose.txt"), "# q\n\ta\n");

        let rows = list_root(root, &ReviewConfig::default(), T0);
        let ws_row = rows.iter().find(|r| r.is_workspace).expect("listed");
        assert_eq!(Some(root.join("ws/assets/icon.svg")), ws_row.icon);
        let deck_row = rows.iter().find(|r| !r.is_workspace).expect("listed");
        assert_eq!(None, deck_row.icon);
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
        // Fully done at *every* depth — recognized, and graduated-not-due at
        // both Recall and Reconstruct (a real Reconstruct schedule, so the
        // cross-depth immediacy rule doesn't fire) — so only a virtual card
        // can make this deck reviewable.
        let entry = store.get_or_insert(card_id, now);
        entry.recognized_ms = Some(now);
        entry.recall = Some(graduated_not_due(now));
        entry.reconstruct = Some(graduated_not_due(now));

        // Fully drilled, nothing due: `done ✓` and not reviewable.
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

        // A due virtual card for this deck makes it reviewable even though
        // every deck card is done at every depth.
        insert_due_virtual_card(&mut store, &deck.subject);
        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert!(status.reviewable);
        assert_eq!("done ✓", status.badge); // unaffected by the virtual card
    }

    #[test]
    fn a_recall_settled_deck_is_still_reviewable_at_reconstruct() {
        // The final-review fix (Important #1): a deck that's mature and not
        // due at Recall is due *right now* at Reconstruct, via the
        // scheduler's cross-depth immediacy rule — `reviewable_recall` must
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
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n# q2\n\ta2\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let mut augment = AugmentCache::open(dir.path().join("augment.json"));
        arm(&mut augment, &deck.cards);
        let now = session::now_ms();
        store.get_or_insert(deck.cards[0].id(), now).recognized_ms = Some(now);

        // Card 2 has never been correctly picked at Recognize, and both cards
        // are recognizable, so the deck is reviewable at Recognize.
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

        store.get_or_insert(deck.cards[1].id(), now).recognized_ms = Some(now);
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
        // With no cached distractors a deck has no buildable pick, so Recognize
        // is unavailable — reviewable_recognize is false even though its cards
        // are unrecognized, and can_recognize (the cram/greying gate) is false.
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n# q2\n\ta2\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let store = Store::open(dir.path().join("progress.json")).unwrap();

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
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "title = \"WS\"\n");
        // done: source-less and fully drilled — Finished, no exam, so it counts
        // as ready. fresh: untouched, not ready.
        write(&ws.join("done.txt"), "# q\n\ta\n");
        write(&ws.join("fresh.txt"), "# q2\n\tb\n");
        let date = crate::time::local_date(T0) + chrono::Days::new(5);
        write(
            &ws.join("alix.local.toml"),
            &format!("[review]\ndeadline = \"{}\"\n", date.format("%Y-%m-%d")),
        );
        let done_id = Deck::load(ws.join("done.txt")).unwrap().cards[0].id();
        let mut store = Store::open(workspace::store_path(&ws)).unwrap();
        store.get_or_insert(done_id, T0).recall = Some(graduated_not_due(T0));
        store.save().unwrap();

        let review = ReviewConfig::default();
        let rows = list_root(root, &review, T0);
        let ws_row = rows.iter().find(|r| r.is_workspace).expect("listed");
        let deadline = ws_row.deadline.as_ref().expect("a set deadline lists");
        assert_eq!(date, deadline.date);
        assert_eq!(5, deadline.days_left);
        assert_eq!((1, 2), (deadline.ready, deadline.total));

        // The standalone fetch (the drill-in header's refresh path) agrees.
        let fetched = workspace_deadline(root, &ws, &review, T0).expect("fetchable");
        assert_eq!(Some(fetched), ws_row.deadline);

        // A PLAIN folder never carries a deadline, even with a local manifest
        // (only a real workspace does — the web catalog's rule).
        let plain = root.join("plain");
        std::fs::create_dir(&plain).unwrap();
        write(&plain.join("d.txt"), "# q\n\ta\n");
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
        let deck_path = dir.path().join("d.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let store = Store::open(dir.path().join("progress.json")).unwrap();
        let review = ReviewConfig::default();
        let now = session::now_ms();

        let (bare, _) = deck_summary(
            &deck_path,
            Some(&store),
            Some(&no_augment()),
            dir.path(),
            &review,
            now,
        );
        assert!(!bare.can_recognize, "un-augmented deck is not recognizable");

        let mut augment = AugmentCache::open(dir.path().join("augment.json"));
        arm(&mut augment, &deck.cards);
        let (armed, _) = deck_summary(
            &deck_path,
            Some(&store),
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
        // The mobile lister's `due` flag: a seen-but-unrecognized card whose
        // Recall and Reconstruct schedules are settled reads as due ONLY when it
        // is recognizable. Un-augmented, Recognize is pick-only-unavailable, so
        // there is nothing to launch — `deck_due` must be false (before
        // pick-only, the bare `has_reviewable(Recognize)` over-reported it).
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = session::now_ms();
        let entry = store.get_or_insert(deck.cards[0].id(), now);
        entry.recall = Some(graduated_not_due(now));
        entry.reconstruct = Some(graduated_not_due(now));

        assert!(
            !deck_due(&deck, &store, &no_augment(), &ReviewConfig::default(), now),
            "un-augmented settled deck is not due"
        );

        // Armed, the unrecognized card is servable at Recognize, so it is due.
        let mut augment = AugmentCache::open(dir.path().join("augment.json"));
        arm(&mut augment, &deck.cards);
        assert!(
            deck_due(&deck, &store, &augment, &ReviewConfig::default(), now),
            "an augmented unrecognized card is due at Recognize"
        );
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

        let before = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert_eq!("1/3", before.badge);

        insert_due_virtual_card(&mut store, &deck.subject);
        let after = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
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
    fn the_highest_currently_solid_depth_wins_the_badge() {
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
    fn new_cards_flag_fires_without_touching_badges() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n# q2\n\ta2\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = session::now_ms();
        // Card 1 has graduated; card 2 has never been touched — no store entry.
        store.get_or_insert(deck.cards[0].id(), now).recall = Some(graduated_not_due(now));

        let status = deck_status(
            &deck,
            &store,
            &no_augment(),
            None,
            false,
            ReviewConfig::default(),
        );
        assert!(status.new_cards);
        // Unaffected by the flag: same state/badge/badge_depth this deck would
        // have had before `new_cards` existed.
        assert_eq!("1/2", status.badge);
        assert_eq!(None, status.badge_depth);
        assert!(!status.badge_dotted);
    }
}
