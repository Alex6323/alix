//! A small checkbox TUI for picking one or more items: decks to review (the
//! startup picker, used when `alix` is launched without deck arguments), a
//! deck's prerequisites (the `deps` editor), or cards to reset.
//!
//! Type to filter, Space to (de)select, Enter to confirm, Esc to cancel. The
//! widget is generic over the item key (`PathBuf` for decks, `u64` card id for
//! cards); the deck-specific candidate building lives below.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    config::{self, KeyPattern, PickerKeys, ReviewConfig},
    deck::{self, Deck, DeckState},
    parser,
    recent::RecentDecks,
    session,
    store::{Store, default_store_path},
    title, workspace,
};

/// Turns a key event into a [`KeyPattern`] for matching against [`PickerKeys`].
/// Keys without a `config::Key` equivalent (arrows, etc.) yield `None`.
fn key_pattern(code: KeyCode, ctrl: bool) -> Option<KeyPattern> {
    let key = match code {
        KeyCode::Char(c) => config::Key::Char(c),
        KeyCode::Enter => config::Key::Enter,
        KeyCode::Tab => config::Key::Tab,
        KeyCode::Esc => config::Key::Esc,
        KeyCode::Backspace => config::Key::Backspace,
        _ => return None,
    };
    Some(KeyPattern { key, ctrl })
}

const HEADER_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Cyan);

/// A group an item belongs to in the startup picker, drawn with a header above
/// its first row. `None` is the default — no header — used by the flat reset /
/// dependency / card pickers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Workspaces,
    Recent,
    Folders,
}

impl Section {
    /// The header shown above the section, or `None` for an unsectioned list.
    fn header(self) -> Option<&'static str> {
        match self {
            Section::None => None,
            Section::Workspaces => Some("Workspaces"),
            Section::Recent => Some("Recent"),
            Section::Folders => Some("Folders"),
        }
    }
}

/// A row in the rendered list: a blank spacer, a section header, an item at a
/// given position within `Picker::filtered`, or a dim subtitle line below an
/// item (carrying that item's index into `Picker::all`). A subtitle isn't
/// selectable — the cursor skips over it.
enum DisplayRow {
    Blank,
    Header(&'static str),
    Item(usize),
    Subtitle(usize),
}

/// One selectable row: identified by `key`, matched/displayed by `label`, with
/// an optional dim `meta` suffix (a deck's last-used age, a card's stage/id).
#[derive(Clone)]
struct Item<K> {
    key: K,
    label: String,
    meta: Option<String>,
    /// A dim one-line subtitle drawn below the row — a workspace's `description`.
    /// `None` keeps the row a single line.
    subtitle: Option<String>,
    /// Deck rows: locked because a `% requires:` prerequisite isn't finished.
    /// Shown dimmed with a lock glyph, but still selectable (advisory).
    locked: bool,
    /// Deck rows: completion state, used to tint the meta (finished → green).
    /// `None` for non-deck pickers (cards, dependency editor).
    state: Option<DeckState>,
    /// A folder/workspace row: it opens (drills in) on Enter rather than being
    /// ticked. A folder with an `alix.toml` is a workspace; without one it's a
    /// plain folder — both drill in.
    is_workspace: bool,
    /// A trace deck (`% trace:`): launched as a predict-verify walk, never ticked
    /// into a merged card review.
    is_trace: bool,
    /// Whether launching this row right now would have anything to do — a card
    /// due or new (or, for a trace, a checkpoint; for an exam-due deck, its exam).
    /// A deck with nothing due (fully drilled, or all on cooldown) is `false`: the
    /// review launcher won't start it, so Enter is a no-op instead of bouncing out
    /// to a "nothing to review" message. `true` for non-review pickers.
    reviewable: bool,
    /// Dim location hint (parent dir) for entries outside the decks dir; `None`
    /// keeps the row clean.
    hint: Option<String>,
    /// The section this row groups under (startup picker), drawn with a header.
    section: Section,
    /// Whether the row shows with no filter. `false` for non-recent loose decks,
    /// which appear only once the filter matches them (so a huge decks folder
    /// doesn't drown the recent ones).
    default_shown: bool,
    /// Tree-branch prefix (`├─ `, `│  └─ `, …) drawn before the row when the
    /// list is a dependency tree (the workspace drill-in). Empty for a flat list;
    /// suppressed while filtering, when the tree structure no longer holds.
    tree_prefix: String,
}

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
    /// deck, which the review launcher won't start (shown with a 🕒).
    pub reviewable: bool,
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
        DeckState::Started => format!("{retired}/{total}"),
    };
    let actually_locked = deck::is_locked(deck, decks_dir, store);
    let locked = enforce_locks && actually_locked;
    // A deck has an exam when it has a source (a fact deck) or is a trace (its
    // exam is the graded compression); it can be SAT when, additionally, it isn't
    // locked — drilled or not (test out early). The failed-exam re-sit cooldown
    // (trace exams) is enforced at the launch sites, which have the config.
    let has_exam = deck.has_exam();
    let examable = has_exam && !actually_locked;
    // Is there anything to launch right now? A trace always walks; an exam-due
    // deck launches its exam (only when its exam isn't locked); otherwise there
    // must be a card due or new. Drilling is never gated by the lock — a
    // prerequisite-locked deck with due cards is still reviewable.
    let scheduler = crate::scheduler::Fsrs::new(review.retention);
    let reviewable = deck.is_trace()
        || (matches!(state, DeckState::ExamDue) && examable)
        || session::has_reviewable(
            &deck.cards,
            store,
            &scheduler,
            session::now_ms(),
            review.retire_after_days,
        );
    DeckStatus {
        state,
        badge,
        locked,
        reviewable,
        mastered,
        is_trace: deck.is_trace(),
        examable,
        has_exam,
    }
}

/// Turns a deck candidate into a picker item, deriving its completion-state
/// meta (`new` / `m/total` at the top stage / `done ✓`) and lock status (a
/// `% requires:` prerequisite not yet finished) from the progress store. A deck
/// that fails to load shows a plain row. `enforce_locks` is false for the
/// browse picker — locking gates *review* progression only, so any deck is
/// browsable.
fn deck_item(
    c: Candidate,
    store: &Store,
    decks_dir: &Path,
    enforce_locks: bool,
    show_kind: bool,
    review: ReviewConfig,
) -> Item<PathBuf> {
    let hint = location_hint(&c.path, decks_dir);
    // A folder row: its title (or folder name) and a deck count; always
    // selectable, no lock/state of its own. Its section header already says
    // whether it's a workspace or a plain folder, so the row doesn't repeat it;
    // a real workspace instead shows when it last made progress.
    if c.is_workspace {
        let (label, meta, subtitle) = match workspace::Workspace::load(&c.path) {
            Ok(ws) => {
                let mut parts = format!("· {} decks", ws.members.len());
                if workspace::is_workspace(&c.path)
                    && let Some(when) = workspace_last_progress(&c.path)
                {
                    parts.push_str(&format!(" · {when}"));
                }
                (ws.display_name(), Some(parts), ws.description)
            }
            Err(_) => (c.name, None, None),
        };
        return Item {
            key: c.path,
            label,
            meta,
            subtitle,
            locked: false,
            state: None,
            is_workspace: true,
            is_trace: false,
            reviewable: true, // a workspace/folder opens (drills in) on Enter
            hint,
            section: Section::None,
            default_shown: true,
            tree_prefix: String::new(),
        };
    }
    let (label, meta, locked, state, is_trace, reviewable) = match Deck::load(&c.path) {
        Ok(deck) => {
            let status = deck_status(&deck, store, Some(decks_dir), enforce_locks, review);
            // In a workspace drill-in, badge whether a member is a trace (walked)
            // or a facts deck (reviewed). In Recent, every row is a loose file, so
            // the kind is just noise — drop it.
            let meta = if show_kind {
                let kind = if status.is_trace { "trace" } else { "deck" };
                format!("· {kind} · {}", status.badge)
            } else {
                format!("· {}", status.badge)
            };
            // Use the explicit `% title:`, else — for a trace — a condensed form
            // of its `% trace:` path-question (a label, not the whole sentence),
            // else the file name (without `.txt`).
            let label = deck
                .title
                .clone()
                .or_else(|| deck.trace.as_deref().map(title::condense))
                .unwrap_or_else(|| stem(&c.name));
            (
                label,
                Some(meta),
                status.locked,
                Some(status.state),
                status.is_trace,
                status.reviewable,
            )
        }
        // A deck that fails to load stays launchable so the load error surfaces.
        Err(_) => (c.name, None, false, None, false, true),
    };
    Item {
        key: c.path,
        label,
        meta,
        subtitle: None,
        locked,
        state,
        is_workspace: false,
        is_trace,
        reviewable,
        hint,
        section: Section::None,
        default_shown: true,
        tree_prefix: String::new(),
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

/// Gives rows that share a display label a location hint so they can be told
/// apart — two workspaces with the same `% title:`, say, need their paths shown.
/// Rows with a unique label keep whatever hint they already had.
fn disambiguate(items: &mut [Item<PathBuf>]) {
    let dups: HashSet<String> = {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for item in items.iter() {
            *seen.entry(item.label.as_str()).or_default() += 1;
        }
        seen.into_iter()
            .filter(|(_, n)| *n > 1)
            .map(|(label, _)| label.to_string())
            .collect()
    };
    for item in items.iter_mut() {
        if dups.contains(&item.label) {
            item.hint = Some(abbreviate_home(&item.key));
        }
    }
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

/// The outcome of the startup picker: the chosen `decks` (empty if cancelled),
/// and `workspace` — the folder they were drilled into, if any, so the caller can
/// return there after the launched activity (review / walk / exam) finishes.
pub struct Picked {
    pub decks: Vec<PathBuf>,
    pub workspace: Option<PathBuf>,
}

/// Runs the startup picker. `enforce_locks` gates launching a deck whose
/// `% requires:` prerequisites aren't finished — true for `review`, false for
/// `browse` (any deck is browsable). `gate_reviewable` refuses to launch a deck
/// with nothing to review right now (true for `review`, off for `browse` and
/// under `--cram`). `start_in` opens straight into a workspace's drill-in
/// (returning there after an activity); `Esc` from it falls back to the top list.
/// `focus` is the deck just launched — the picker re-opens with the cursor on it
/// (so the selection doesn't jump while the user was away), if it's still shown.
#[expect(clippy::too_many_arguments)] // each is a distinct, named picker setting
pub fn pick(
    terminal: &mut ratatui::DefaultTerminal,
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
    enforce_locks: bool,
    gate_reviewable: bool,
    start_in: Option<&Path>,
    focus: Option<&Path>,
    keys: &PickerKeys,
    review: ReviewConfig,
) -> Result<Picked> {
    // Runs on the caller's terminal: opening a project, stepping back, *and*
    // returning from a launched review all stay on one live screen — the TUI is
    // never torn down and reopened between them.
    let mut top = top_picker(
        decks_dir,
        recent,
        store,
        enforce_locks,
        gate_reviewable,
        keys,
        review,
    );
    // Land on the just-launched loose deck (a workspace member is focused in its
    // own drill-in, inside `navigate`).
    if start_in.is_none()
        && let Some(f) = focus
    {
        top.focus_key(f);
    }
    navigate(
        terminal,
        &mut top,
        decks_dir,
        enforce_locks,
        gate_reviewable,
        start_in,
        focus,
        review,
    )
}

/// Builds the top-level picker — Workspaces · Recent (loose decks, recent-first;
/// the rest appear only when filtering) · Folders (plain decks folders). A deck
/// inside a workspace stays out of Recent: you reach it by opening its workspace.
fn top_picker(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
    enforce_locks: bool,
    gate_reviewable: bool,
    keys: &PickerKeys,
    review: ReviewConfig,
) -> Picker<PathBuf> {
    let (mut workspaces, mut loose, mut folders) = (Vec::new(), Vec::new(), Vec::new());
    for c in build_candidates(decks_dir, recent) {
        // A deck that lives inside a workspace belongs to it — you reach it by
        // opening the workspace, so it doesn't clutter Recent.
        if !c.is_workspace && c.path.parent().is_some_and(workspace::is_workspace) {
            continue;
        }
        let section = if c.is_workspace {
            if workspace::is_workspace(&c.path) {
                Section::Workspaces
            } else {
                Section::Folders
            }
        } else {
            Section::Recent
        };
        let is_recent = c.last_used_ms.is_some();
        // Recent rows drop the trace/deck kind label (it's just noise there).
        let mut item = deck_item(c, store, decks_dir, enforce_locks, false, review);
        item.section = section;
        // Recent shows recent loose decks you can actually start now — finished
        // (mastered / done) ones are hidden by default so the list stays a quick
        // launchpad; they're still reachable by filtering. A prerequisite-locked
        // deck is still drillable, so it stays in Recent.
        item.default_shown = section != Section::Recent
            || (is_recent && !matches!(item.state, Some(DeckState::Finished)));
        match section {
            Section::Workspaces => workspaces.push(item),
            Section::Folders => folders.push(item),
            _ => loose.push(item),
        }
    }
    disambiguate(&mut workspaces);
    disambiguate(&mut loose);
    disambiguate(&mut folders);
    let items = workspaces
        .into_iter()
        .chain(loose)
        .chain(folders)
        .collect::<Vec<_>>();
    let mut picker = Picker::new(
        items,
        HashSet::new(),
        // No title — the top picker's header is just "alix".
        String::new(),
        // Footer is computed per launcher state in `draw`.
        String::new(),
        false,
        no_decks_message(decks_dir),
    );
    picker.launcher = true;
    // Single-launch for now: Enter opens the focused row, no checkboxes. The
    // multi-select machinery (Space/Tab/confirm) stays in place but unused — we
    // may bring a deliberate multi-deck flow back later.
    picker.multi_select = false;
    picker.gate_reviewable = gate_reviewable;
    picker.keys = keys.clone();
    picker
}

/// Drives the top picker and its drill-ins on one live `terminal`: opening a
/// project runs its drill-in sub-picker, and stepping back re-runs the top picker
/// exactly where it was left — no TUI teardown between views. With `start_in`, it
/// opens straight into that workspace's drill-in first (used to return there after
/// an activity); `Esc` from it drops to the top list.
#[expect(clippy::too_many_arguments)] // each is a distinct, named picker/navigation setting
fn navigate(
    terminal: &mut ratatui::DefaultTerminal,
    top: &mut Picker<PathBuf>,
    decks_dir: &Path,
    enforce_locks: bool,
    gate_reviewable: bool,
    start_in: Option<&Path>,
    focus: Option<&Path>,
    review: ReviewConfig,
) -> Result<Picked> {
    // Resuming straight into a workspace (returning after an activity): land on the
    // member just launched, so the selection doesn't jump (the user can then step
    // down to its dependent).
    if let Some(ws) = start_in {
        let mut sub =
            workspace_picker(ws, decks_dir, enforce_locks, gate_reviewable, &top.keys, review)?;
        if let Some(f) = focus {
            sub.focus_key(f);
        }
        if let Some(decks) = sub.run(terminal)? {
            return Ok(Picked {
                decks,
                workspace: Some(ws.to_path_buf()),
            });
        }
        // Backed out of the workspace → fall through to the top list.
    }
    loop {
        top.rearm(); // re-runnable, keeping its cursor / filter / selection
        let chosen = top.run(terminal)?;
        // `m` opens the Mastered window — the completed decks tucked out of Recent.
        if std::mem::take(&mut top.request_mastered) {
            if let Some(decks) = run_mastered_window(terminal, top)?
                && !decks.is_empty()
            {
                return Ok(Picked {
                    decks,
                    workspace: None,
                });
            }
            continue; // closed the window → back to the top list
        }
        let Some(chosen) = chosen else {
            return Ok(Picked {
                decks: Vec::new(),
                workspace: None,
            }); // cancelled at the top level
        };
        // Opening a single folder/workspace drills into a sub-picker of its decks.
        if let [path] = chosen.as_slice()
            && workspace::has_decks(path)
        {
            let mut sub = workspace_picker(
                path,
                decks_dir,
                enforce_locks,
                gate_reviewable,
                &top.keys,
                review,
            )?;
            match sub.run(terminal)? {
                Some(decks) => {
                    return Ok(Picked {
                        decks,
                        workspace: Some(path.clone()),
                    });
                }
                None => continue, // back: re-run the top list where we left off
            }
        }
        return Ok(Picked {
            decks: chosen,
            workspace: None,
        });
    }
}

/// Opens the **Mastered** window: the completed (exam-passed) decks tucked out of
/// Recent, gathered from the top picker's own rows. Returns the chosen deck (to
/// reopen it), or `None` on `Esc`/`h`.
fn run_mastered_window(
    terminal: &mut ratatui::DefaultTerminal,
    top: &Picker<PathBuf>,
) -> Result<Option<Vec<PathBuf>>> {
    let items: Vec<Item<PathBuf>> = top
        .all
        .iter()
        .filter(|item| item.meta.as_deref().is_some_and(|m| m.contains("mastered")))
        .cloned()
        .map(|mut item| {
            item.default_shown = true; // every mastered deck shows in this view
            item.section = Section::None;
            item
        })
        .collect();
    let mut picker = Picker::new(
        items,
        HashSet::new(),
        "mastered 🎉".to_string(),
        String::new(),
        false,
        vec![Line::from(
            "  No mastered decks yet — pass an exam to earn one. 🎉",
        )],
    );
    picker.launcher = true;
    picker.multi_select = false;
    picker.keys = top.keys.clone();
    // Not gated: you can reopen a mastered deck (e.g. to cram or re-examine it).
    picker.run(terminal)
}

/// Opens a workspace directly into its member sub-picker (for `alix workspace
/// <dir>`): the same drill-in list, with the folder itself as the lookup root so
/// sibling `% requires:` and locks resolve within it. `None` if cancelled.
pub fn pick_workspace(
    folder: &Path,
    enforce_locks: bool,
    review: ReviewConfig,
) -> Result<Option<Vec<PathBuf>>> {
    // `alix workspace` is a review context (no `--cram`), so gate unreviewable
    // members just like the review launcher.
    launch(workspace_picker(
        folder,
        folder,
        enforce_locks,
        enforce_locks,
        &PickerKeys::default(),
        review,
    )?)
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

/// The drill-in sub-picker for a folder/workspace: its members drawn as an
/// **unlock dependency tree** — a deck nests under the `% requires:` prerequisite
/// that gates it, foundations at the roots, siblings startable-first. Each row is
/// badged `· trace ·` / `· deck ·`; nothing starts ticked — `Enter` launches the
/// focused row (a trace **walks**, a deck **reviews**), `Space` ticks fact decks
/// (a trace can't join a card review) and `Tab` confirms a merged review. `Esc`
/// returns `None` to step back to the top list. (Filtering flattens the tree.)
/// Member badges/locks are drawn from the **right** store — a workspace's own
/// (`workspace::store_path`) or the global store for a plain folder — so they
/// match what the session will write.
fn workspace_picker(
    folder: &Path,
    decks_dir: &Path,
    enforce_locks: bool,
    gate_reviewable: bool,
    keys: &PickerKeys,
    review: ReviewConfig,
) -> Result<Picker<PathBuf>> {
    let ws = workspace::Workspace::load(folder)?;
    let store_path = if workspace::is_workspace(folder) {
        workspace::store_path(folder)
    } else {
        default_store_path().context("cannot determine the data directory")?
    };
    let store = Store::open(&store_path)?;
    let items: Vec<Item<PathBuf>> = ws
        .members
        .iter()
        .map(|m| {
            let c = Candidate {
                name: file_name(m),
                path: m.clone(),
                last_used_ms: None,
                is_workspace: false,
            };
            // Drill-in rows show the trace/deck kind (walk vs review differ).
            let mut item = deck_item(c, &store, decks_dir, enforce_locks, true, review);
            item.hint = None; // inside the workspace, the folder path is redundant
            item
        })
        .collect();

    // Lay the members out as a dependency tree. Siblings come startable-first
    // (the rest — locked, or nothing due — after), then by name, so what you can
    // act on stays near the top within each branch.
    let parent = member_parents(&ws.members, decks_dir);
    let key: Vec<(bool, String)> = items
        .iter()
        .map(|item| {
            // Startable-first: a deck is "blocked" only when there's nothing to
            // launch (nothing due / exam-locked-and-drilled). A prereq-locked
            // deck with due cards is still drillable, so it isn't blocked.
            let blocked = gate_reviewable && !item.reviewable;
            (blocked, item.label.clone())
        })
        .collect();
    let mut slots: Vec<Option<Item<PathBuf>>> = items.into_iter().map(Some).collect();
    let ordered: Vec<Item<PathBuf>> = dependency_forest(&parent, &key)
        .into_iter()
        .map(|(i, prefix)| {
            let mut item = slots[i].take().expect("each member placed once");
            item.tree_prefix = prefix;
            item
        })
        .collect();

    let mut picker = Picker::new(
        ordered,
        HashSet::new(),
        ws.display_name(),
        // Footer is computed per launcher state in `draw`.
        String::new(),
        false,
        vec![Line::from("  This workspace has no decks.")],
    );
    picker.launcher = true;
    picker.multi_select = false; // single-launch: pick one deck/trace, no checkboxes
    picker.gate_reviewable = gate_reviewable;
    picker.keys = keys.clone();
    Ok(picker)
}

/// Runs the deck picker for `reset`: the same checkbox UI, but `exact` (an
/// empty tick set means "nothing", never the card under the cursor) and reset
/// wording.
pub fn pick_to_reset(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
    review: ReviewConfig,
) -> Result<Vec<PathBuf>> {
    let items = build_candidates(decks_dir, recent)
        .into_iter()
        .map(|c| deck_item(c, store, decks_dir, false, false, review))
        .collect();
    let picker = Picker::new(
        items,
        HashSet::new(),
        "select decks to reset".to_string(),
        " SPACE select │ ENTER reset │ ↑↓ move │ type to filter │ ESC cancel".to_string(),
        true,
        no_decks_message(decks_dir),
    );
    Ok(launch(picker)?.unwrap_or_default())
}

/// Picks cards to act on from a pre-built `(id, label, meta)` list. Returns the
/// chosen ids (empty if cancelled or nothing ticked).
pub fn pick_cards(items: Vec<(u64, String, Option<String>)>, title: &str) -> Result<Vec<u64>> {
    let items = items
        .into_iter()
        .map(|(key, label, meta)| Item {
            key,
            label,
            meta,
            subtitle: None,
            locked: false,
            state: None,
            is_workspace: false,
            is_trace: false,
            reviewable: true,
            hint: None,
            section: Section::None,
            default_shown: true,
            tree_prefix: String::new(),
        })
        .collect();
    let picker = Picker::new(
        items,
        HashSet::new(),
        title.to_string(),
        " SPACE select │ ENTER reset │ ↑↓ move │ type to filter │ ESC cancel".to_string(),
        true,
        vec![Line::from("  No cards.")],
    );
    Ok(launch(picker)?.unwrap_or_default())
}

/// Runs the dependency editor for `target`: the same checkbox UI over the
/// decks in `decks_dir`, pre-ticked to the deck's current prerequisites
/// (`requires`, matched by name). Returns the chosen prerequisite paths
/// (possibly empty, meaning "no dependencies"), or `None` if cancelled.
/// `target` is excluded — a deck can't require itself.
pub fn edit_dependencies(
    decks_dir: &Path,
    target: &Path,
    requires: &[String],
) -> Result<Option<Vec<PathBuf>>> {
    let target_name = file_name(target);
    let mut candidates = dir_candidates(decks_dir);
    // Prerequisites are decks, not workspaces.
    candidates.retain(|c| !c.is_workspace && c.name != target_name);

    // Keep any current prerequisite that isn't a deck in the decks dir visible
    // and pre-checked, so saving doesn't silently drop it.
    let listed: HashSet<String> = candidates.iter().map(|c| stem(&c.name)).collect();
    for req in requires {
        if !listed.contains(&stem(req)) {
            candidates.push(Candidate {
                name: req.clone(),
                path: PathBuf::from(req),
                last_used_ms: None,
                is_workspace: false,
            });
        }
    }

    let want: HashSet<String> = requires.iter().map(|r| stem(r)).collect();
    let preselected: HashSet<PathBuf> = candidates
        .iter()
        .filter(|c| want.contains(&stem(&c.name)))
        .map(|c| c.path.clone())
        .collect();

    let items = candidates
        .into_iter()
        .map(|c| Item {
            key: c.path,
            label: c.name,
            meta: None,
            subtitle: None,
            locked: false,
            state: None,
            is_workspace: false,
            is_trace: false,
            reviewable: true,
            hint: None,
            section: Section::None,
            default_shown: true,
            tree_prefix: String::new(),
        })
        .collect();
    let picker = Picker::new(
        items,
        preselected,
        format!("prerequisites for {target_name}"),
        " SPACE toggle │ ENTER save │ ↑↓ move │ type to filter │ ESC cancel".to_string(),
        true,
        vec![
            Line::default(),
            Line::from(format!("  No other decks in {}.", decks_dir.display())),
        ],
    );
    launch(picker)
}

/// The empty-state shown when there are no decks to list.
fn no_decks_message(decks_dir: &Path) -> Vec<Line<'static>> {
    vec![
        Line::default(),
        Line::from(format!("  No decks found in {}.", decks_dir.display())),
        Line::default(),
        Line::from("  Put .txt decks there, or pass deck files on the command line.".dim()),
    ]
}

/// Sets up the terminal, runs the picker, and restores the terminal.
fn launch<K: Clone + Eq + Hash>(mut picker: Picker<K>) -> Result<Option<Vec<K>>> {
    let mut terminal = ratatui::init();
    let result = picker.run(&mut terminal);
    ratatui::restore();
    result
}

// ---- the widget ---------------------------------------------------------

struct Picker<K> {
    all: Vec<Item<K>>,
    /// Configurable navigation keys (launcher only); defaults are Vim-style.
    keys: PickerKeys,
    filter: String,
    /// Indices into `all` matching the filter.
    filtered: Vec<usize>,
    /// Cursor position within `filtered`.
    cursor: usize,
    /// Scroll offset within `filtered`.
    offset: usize,
    selected: HashSet<K>,
    /// Header label (after "alix — ").
    title: String,
    /// Footer key hints.
    footer: String,
    /// When true, Enter returns exactly the ticked set (possibly empty); when
    /// false (startup picker), an empty tick set falls back to the item under
    /// the cursor.
    exact: bool,
    /// Lines shown when there are no items.
    empty: Vec<Line<'static>>,
    /// Two-phase deck-launcher mode (the startup picker): Enter launches the
    /// focused deck; `Space` ticks decks; `Tab` confirms a multi-deck
    /// selection. Off for the reset / dependency pickers (plain tick +
    /// Enter).
    launcher: bool,
    /// Launcher only: whether we're in filter mode (typing narrows the list).
    /// Off by default so the focus is on the list — `j`/`k` and friends navigate,
    /// `/` or `Ctrl-F` starts filtering, `Esc` leaves it. The classic pickers
    /// (reset / deps / cards) ignore this and filter on every keystroke.
    filtering: bool,
    /// Launcher only: set when the user pressed `m` to open the Mastered window;
    /// the caller picks this up after `run` returns and opens that view.
    request_mastered: bool,
    /// Whether rows can be ticked into a multi-selection: shows the `[ ]`/`[x]`
    /// column and enables `Space`/`Tab`. Off for the workspace drill-in, which is
    /// a single-launch list (Enter a deck to review, a trace to walk) — no
    /// checkboxes.
    multi_select: bool,
    /// Review launcher only: refuse to start a deck with nothing to review right
    /// now (`Item::reviewable == false`), so Enter is a no-op instead of bouncing
    /// out to a "nothing to review" message. Off for browse (any deck is
    /// browsable) and under `--cram` (cooldowns are ignored).
    gate_reviewable: bool,
    done: bool,
    cancelled: bool,
}

impl<K: Clone + Eq + Hash> Picker<K> {
    fn new(
        all: Vec<Item<K>>,
        selected: HashSet<K>,
        title: String,
        footer: String,
        exact: bool,
        empty: Vec<Line<'static>>,
    ) -> Self {
        // With no filter yet, show only the default rows (non-recent loose
        // decks stay hidden until the filter matches them — see `refilter`).
        let filtered = (0..all.len()).filter(|&i| all[i].default_shown).collect();
        Self {
            all,
            keys: PickerKeys::default(),
            filter: String::new(),
            filtered,
            cursor: 0,
            offset: 0,
            selected,
            title,
            footer,
            exact,
            empty,
            launcher: false,
            filtering: false,
            request_mastered: false,
            multi_select: true,
            gate_reviewable: false,
            done: false,
            cancelled: false,
        }
    }

    /// Returns `None` if cancelled, else the chosen keys.
    fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<Option<Vec<K>>> {
        while !self.done {
            terminal.draw(|frame| self.draw(frame))?;
            match event::read()? {
                // Resize with the event's own dimensions, not a (possibly stale)
                // ioctl query, so the next draw reflows immediately.
                Event::Resize(w, h) => terminal.resize(Rect::new(0, 0, w, h))?,
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    // In the launcher, keys navigate by default; `/` or `Ctrl-F`
                    // starts filter mode, where letters narrow the list instead. The
                    // classic reset / deps / card pickers always filter on a keypress.
                    let nav = self.launcher && !self.filtering;
                    let take_text = self.filtering || !self.launcher;
                    // In nav mode, letters are commands, matched against the
                    // (configurable) navigation keys.
                    let pattern = key_pattern(key.code, ctrl);
                    let hit = |list: &[KeyPattern]| pattern.is_some_and(|p| list.contains(&p));
                    match key.code {
                        KeyCode::Char('c') if ctrl => self.cancel(),
                        // Esc in the filter box keeps the filter and drops back to the
                        // list, focused on the first match.
                        KeyCode::Esc if self.filtering => {
                            self.filtering = false;
                            self.cursor = 0;
                        }
                        // Esc in nav with a filter applied clears it; otherwise cancel.
                        KeyCode::Esc if self.launcher && !self.filter.is_empty() => {
                            self.stop_filtering()
                        }
                        KeyCode::Esc => self.cancel(),
                        // Launcher: Enter opens the focused row; otherwise (reset / deps
                        // pickers) Enter accepts the selection.
                        KeyCode::Enter if self.launcher => self.launch_focused(),
                        KeyCode::Enter => self.done = true,
                        // Arrows + Ctrl-n/p always move; the rest of nav is the
                        // configurable key set (Vim-style by default).
                        KeyCode::Up => self.move_cursor(-1),
                        KeyCode::Down => self.move_cursor(1),
                        KeyCode::Char('p') if ctrl => self.move_cursor(-1),
                        KeyCode::Char('n') if ctrl => self.move_cursor(1),
                        _ if nav && hit(&self.keys.down) => self.move_cursor(1),
                        _ if nav && hit(&self.keys.up) => self.move_cursor(-1),
                        _ if nav && hit(&self.keys.open) => self.launch_focused(),
                        _ if nav && hit(&self.keys.back) => self.cancel(),
                        // Jump to first/last is fixed at g/G/Home/End, matched
                        // case-sensitively here: the config key layer lowercases
                        // letters, so `g` and `G` can't be told apart there (the same
                        // reason the browse pager hardcodes them). Letters act only in
                        // nav mode — while filtering they're text.
                        KeyCode::Home => self.cursor = 0,
                        KeyCode::End => self.cursor = self.filtered.len().saturating_sub(1),
                        KeyCode::Char('g') if nav => self.cursor = 0,
                        KeyCode::Char('G') if nav => {
                            self.cursor = self.filtered.len().saturating_sub(1);
                        }
                        _ if nav && hit(&self.keys.filter) => self.filtering = true,
                        // `m` opens the Mastered window (handled by the caller).
                        _ if nav && hit(&self.keys.mastered) => {
                            self.request_mastered = true;
                            self.done = true;
                        }
                        // Space ticks in the multi-select pickers; elsewhere it's just
                        // a filter character (handled below) or ignored in nav mode.
                        KeyCode::Char(' ') if self.multi_select => self.toggle(),
                        // Backspace: edit the filter, or step back when there's nothing
                        // to delete (leave filter mode, return from a drill-in, cancel).
                        KeyCode::Backspace if nav => self.cancel(),
                        KeyCode::Backspace if self.filter.is_empty() => {
                            if self.filtering {
                                self.stop_filtering();
                            } else {
                                self.cancel();
                            }
                        }
                        KeyCode::Backspace => {
                            self.filter.pop();
                            self.refilter();
                        }
                        KeyCode::Char(c) if !ctrl && take_text => {
                            self.filter.push(c);
                            self.refilter();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if self.cancelled {
            return Ok(None);
        }
        Ok(Some(self.result()))
    }

    /// The item under the cursor, if any.
    fn focused(&self) -> Option<&Item<K>> {
        self.filtered.get(self.cursor).map(|&i| &self.all[i])
    }

    /// The keys of the ticked items, in list order.
    fn ticked(&self) -> Vec<K> {
        self.all
            .iter()
            .filter(|item| self.selected.contains(&item.key))
            .map(|item| item.key.clone())
            .collect()
    }

    /// The chosen keys once the picker is done (not cancelled).
    fn result(&self) -> Vec<K> {
        if self.launcher {
            // The single focused deck that Enter launched (one deck per session).
            return self
                .focused()
                .map(|item| vec![item.key.clone()])
                .unwrap_or_default();
        }
        let chosen = self.ticked();
        if self.exact || !chosen.is_empty() {
            return chosen;
        }
        // Startup picker with nothing ticked: use the item under the cursor.
        self.focused()
            .map(|item| vec![item.key.clone()])
            .unwrap_or_default()
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.done = true;
    }

    /// Clears the run flags so the picker can be run again on the same terminal
    /// (after stepping back from a drill-in), keeping its cursor, filter and
    /// selection so the user lands where they left off.
    fn rearm(&mut self) {
        self.done = false;
        self.cancelled = false;
    }

    /// Leaves filter mode and clears the filter (back to the full list).
    fn stop_filtering(&mut self) {
        self.filtering = false;
        self.filter.clear();
        self.refilter();
    }

    /// Launcher: open the focused row (Enter / `l`), unless it's locked or — when
    /// gating — has nothing to review (then it's a no-op, and a workspace/folder
    /// drills in via the caller).
    fn launch_focused(&mut self) {
        let gate = self.gate_reviewable;
        // Drilling is never gated by the prerequisite lock — only the exam is
        // (and an exam-locked deck isn't `reviewable`), so launch on `reviewable`
        // alone.
        if self.focused().is_some_and(|item| item.reviewable || !gate) {
            self.done = true;
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let last = self.filtered.len() - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last as isize) as usize;
    }

    /// Moves the cursor onto the row whose key is `key`, if it's currently shown
    /// (in `filtered`). Used to re-land on the just-launched deck when the picker
    /// re-opens after a review/browse, so the selection doesn't jump under the
    /// user; a no-op if the row is filtered/hidden (then the cursor stays put).
    /// Generic over `Borrow` so a `&Path` matches a `PathBuf` key.
    fn focus_key<Q>(&mut self, key: &Q)
    where
        K: std::borrow::Borrow<Q>,
        Q: Eq + ?Sized,
    {
        if let Some(pos) = self
            .filtered
            .iter()
            .position(|&i| self.all[i].key.borrow() == key)
        {
            self.cursor = pos;
        }
    }

    fn toggle(&mut self) {
        if !self.multi_select {
            return; // single-launch lists have nothing to tick
        }
        if let Some(&i) = self.filtered.get(self.cursor) {
            // An exam-due deck has no reviewable cards — it only launches its own
            // exam, so it can't join a merged review. A workspace opens (Enter)
            // rather than being ticked. A trace is walked, not card-reviewed, so
            // it can't join a merged review. A deck with nothing due can't either
            // (when the review launcher is gating). A prerequisite-locked deck is
            // still drillable, so it CAN be ticked. Non-deck pickers set none of
            // these.
            if self.all[i].state == Some(DeckState::ExamDue)
                || self.all[i].is_workspace
                || self.all[i].is_trace
                || (self.gate_reviewable && !self.all[i].reviewable)
            {
                return;
            }
            let key = &self.all[i].key;
            if !self.selected.remove(key) {
                self.selected.insert(key.clone());
            }
        }
    }

    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        let filtering = !needle.is_empty();
        // Recent-default rule: with no filter, hide non-default rows (non-recent
        // loose decks, and mastered/done/locked ones); once filtering, search
        // every row so the filter can still reach them.
        self.filtered = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                (filtering || item.default_shown) && item.label.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
        self.cursor = self.cursor.min(self.filtered.len().saturating_sub(1));
    }

    // ---- rendering -----------------------------------------------------

    fn draw(&mut self, frame: &mut Frame) {
        let [header, filter, list, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let title = self.title.as_str();
        // Just "alix" when there's no title (the top picker); else "alix —
        // <title>" (a drilled-into workspace). No item count.
        let left = if title.is_empty() {
            " alix".to_string()
        } else {
            format!(" alix — {title}")
        };
        // The selected-count only means something where rows can be ticked.
        let right = if self.multi_select {
            format!("{} selected ", self.selected.len())
        } else {
            String::new()
        };
        frame.render_widget(bar(&left, &right, header.width), header);

        // Filter line — shown as an input (with a cursor) while editing, or as the
        // still-applied filter once you leave the box; with nothing to show it's a
        // dim hint (focus is on the list).
        let editing = self.filtering || !self.launcher;
        if editing || !self.filter.is_empty() {
            frame.render_widget(Paragraph::new(format!(" filter: {}", self.filter)), filter);
            if editing {
                frame.set_cursor_position(Position::new(
                    filter.x + 9 + self.filter.chars().count() as u16,
                    filter.y,
                ));
            }
        } else {
            frame.render_widget(
                Paragraph::new(" / or Ctrl-F to filter").style(Style::new().fg(Color::DarkGray)),
                filter,
            );
        }

        self.draw_list(frame, list);

        let footer_text = self.footer_text();
        // When the focused deck's exam is locked, spell it out on the right — the
        // row no longer carries a (confusing) lock glyph.
        let footer_right = if self.focused().is_some_and(|it| it.locked) {
            "🔒 Exam locked "
        } else {
            ""
        };
        frame.render_widget(bar(&footer_text, footer_right, footer.width), footer);
    }

    /// The footer hints for the current state. The launcher computes them per
    /// state; other pickers use their fixed `footer`.
    fn footer_text(&self) -> String {
        if !self.launcher {
            return self.footer.clone();
        }
        // Filter mode: typing narrows; arrows still move; Esc leaves it.
        if self.filtering {
            return " ENTER open │ ↑↓ move │ ESC clear filter".to_string();
        }
        // The launcher is single-launch — one deck per session.
        " ENTER/l open │ j/k move │ / filter │ m mastered │ h/ESC back".to_string()
    }

    /// The display sequence: a section header above the first row of each
    /// section, interleaved with the filtered item rows. A [`DisplayRow::Item`]
    /// carries a position into `self.filtered`.
    fn display_rows(&self) -> Vec<DisplayRow> {
        let mut rows = Vec::new();
        let mut prev: Option<Section> = None;
        for (pos, &i) in self.filtered.iter().enumerate() {
            let section = self.all[i].section;
            if Some(section) != prev
                && let Some(header) = section.header()
            {
                // A blank line above every header — including the first, to set
                // the sections off from the filter line — so they breathe instead
                // of running together.
                rows.push(DisplayRow::Blank);
                rows.push(DisplayRow::Header(header));
            }
            prev = Some(section);
            rows.push(DisplayRow::Item(pos));
            // A workspace's description trails it as a dim, non-selectable line.
            if self.all[i].subtitle.is_some() {
                rows.push(DisplayRow::Subtitle(i));
            }
        }
        rows
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect) {
        if self.all.is_empty() {
            frame.render_widget(Paragraph::new(self.empty.clone()), area);
            return;
        }

        let display = self.display_rows();
        // The display line holding the cursor's item, then a window around it.
        let cursor_line = display
            .iter()
            .position(|row| matches!(row, DisplayRow::Item(pos) if *pos == self.cursor))
            .unwrap_or(0);
        let height = area.height as usize;
        if cursor_line < self.offset {
            self.offset = cursor_line;
        } else if cursor_line >= self.offset + height {
            self.offset = cursor_line + 1 - height;
        }

        let width = area.width as usize;
        let lines: Vec<Line> = display
            .iter()
            .skip(self.offset)
            .take(height)
            .map(|row| match row {
                DisplayRow::Blank => Line::default(),
                DisplayRow::Header(header) => Line::from(Span::styled(
                    format!(" {header}"),
                    Style::new()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )),
                DisplayRow::Item(pos) => {
                    self.item_line(self.filtered[*pos], *pos == self.cursor, width)
                }
                // The workspace description: dim, indented under its row, capped.
                DisplayRow::Subtitle(i) => {
                    let text = self.all[*i].subtitle.as_deref().unwrap_or_default();
                    let cap = width.saturating_sub(6).clamp(12, 72);
                    Line::from(Span::styled(
                        format!("   {}", truncate(text, cap)),
                        Style::new().fg(Color::DarkGray),
                    ))
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// Renders one deck/card row: `i` indexes `self.all`, `on_cursor` highlights
    /// it, `width` is the list width for truncation. Draws the checkbox/lock,
    /// label, optional location hint, and the tinted state suffix.
    fn item_line(&self, i: usize, on_cursor: bool, width: usize) -> Line<'static> {
        let item = &self.all[i];
        let checked = self.selected.contains(&item.key);

        // A deck the review launcher won't start right now: nothing is due (or
        // it's fully drilled, exam-locked). Dimmed and disabled, with a 🕒 clock.
        // Only when gating (review, not browse / cram).
        let unreviewable = self.gate_reviewable && !item.reviewable;
        let marker = if on_cursor { "›" } else { " " };
        // 🕒 nothing due right now (on cooldown). A finished deck shows its 🎉 in
        // the badge, not here. An exam-locked deck carries no row glyph — the
        // footer spells out "🔒 Exam locked" when it's focused, which is clearer
        // than a bare lock on the row (that read as "the deck is locked").
        let glyph = if unreviewable && !matches!(item.state, Some(DeckState::Finished)) {
            "🕒 "
        } else {
            ""
        };
        // The dependency-tree branch prefix (drill-in only), dropped while
        // filtering — a filtered subset is no longer a tree.
        let tree = if self.filter.is_empty() {
            item.tree_prefix.as_str()
        } else {
            ""
        };
        let show_check = self.multi_select;

        // Truncate a long label (a trace's `% trace:` sentence can be a
        // paragraph) so the meta badge — `· trace · new` — stays on screen
        // instead of being clipped off the right edge.
        let fixed = 2 // marker + space
            + if show_check { 4 } else { 0 } // "[ ] "
            + tree.chars().count()
            + glyph.chars().count()
            + item.meta.as_ref().map_or(0, |m| 2 + m.chars().count())
            + item.hint.as_ref().map_or(0, |h| 2 + h.chars().count());
        // Fit the width (+2 margin for wide glyphs), but also hard-cap the label
        // so a long `% title:` / `% trace:` sentence stays a short list entry.
        const LABEL_CAP: usize = 48;
        let budget = width.saturating_sub(fixed + 2).clamp(12, LABEL_CAP);
        let label = truncate(&item.label, budget);

        // The checkbox column, shown only by multi-select pickers (reset / deps /
        // card selection). The deck launcher is single-launch — no boxes — so its
        // rows (decks and workspaces alike) sit flush after the cursor marker.
        let main = if show_check {
            let check = if checked { "[x]" } else { "[ ]" };
            format!("{marker} {check} {tree}{glyph}{label}")
        } else {
            format!("{marker} {tree}{glyph}{label}")
        };

        let mut style = Style::new();
        if on_cursor {
            style = style.fg(Color::Black).bg(Color::Cyan);
        } else if checked {
            style = style.fg(Color::Cyan);
        } else if unreviewable {
            // Advisory: a deck with nothing to launch right now (nothing due, or
            // fully drilled with its exam locked) is dimmed. A drillable locked
            // deck stays bright — it's startable.
            style = style.fg(Color::DarkGray);
        }

        let mut spans = vec![Span::styled(main, style)];
        // A dim location hint for out-of-decks-dir entries (disambiguates
        // same-named decks/workspaces). Follows the row style on the cursor.
        if let Some(hint) = &item.hint {
            let hint_style = if on_cursor {
                style
            } else {
                Style::new().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!("  {hint}"), hint_style));
        }
        if let Some(meta) = &item.meta {
            // Tint the state suffix (finished → green, exam due → yellow),
            // but keep the cursor and dimmed (nothing-due) styling dominant
            // where they apply.
            let meta_style = if on_cursor || unreviewable {
                style
            } else {
                match item.state {
                    Some(DeckState::Finished) => Style::new().fg(Color::Green),
                    Some(DeckState::ExamDue) => Style::new().fg(Color::Yellow),
                    _ => style,
                }
            };
            spans.push(Span::styled(format!("  {meta}"), meta_style));
        }
        Line::from(spans)
    }
}

/// Shortens `s` to at most `max` characters, ending with `…` when it cut
/// anything (so a long trace description doesn't crowd out the row's meta badge).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Renders a full-width colored bar with left/right text.
fn bar(left: &str, right: &str, width: u16) -> Paragraph<'static> {
    let pad = (width as usize)
        .saturating_sub(left.chars().count())
        .saturating_sub(right.chars().count());
    Paragraph::new(format!("{left}{}{right}", " ".repeat(pad))).style(HEADER_STYLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An item with a given display `label` and `key` path (hint unset).
    fn labeled(label: &str, key: &str) -> Item<PathBuf> {
        Item {
            key: PathBuf::from(key),
            label: label.to_string(),
            meta: None,
            subtitle: None,
            locked: false,
            state: None,
            is_workspace: false,
            is_trace: false,
            reviewable: true,
            hint: None,
            section: Section::None,
            default_shown: true,
            tree_prefix: String::new(),
        }
    }

    #[test]
    fn disambiguate_adds_a_path_hint_to_same_titled_rows() {
        let mut items = vec![
            labeled("Scheduling", "/srv/decks/sched-a"),
            labeled("Scheduling", "/srv/decks/sched-b"),
            labeled("Unique", "/srv/decks/unique"),
        ];
        disambiguate(&mut items);
        // The two same-titled rows get distinct path hints; the unique one none.
        assert!(items[0].hint.is_some());
        assert!(items[1].hint.is_some());
        assert_ne!(items[0].hint, items[1].hint);
        assert!(items[2].hint.is_none());
    }

    fn picker_with(labels: &[&str]) -> Picker<PathBuf> {
        let all = labels
            .iter()
            .map(|n| Item {
                key: PathBuf::from(n),
                label: n.to_string(),
                meta: None,
                subtitle: None,
                locked: false,
                state: None,
                is_workspace: false,
                is_trace: false,
                reviewable: true,
                hint: None,
                section: Section::None,
                default_shown: true,
                tree_prefix: String::new(),
            })
            .collect();
        Picker::new(
            all,
            HashSet::new(),
            "t".to_string(),
            "f".to_string(),
            false,
            Vec::new(),
        )
    }

    fn launcher_with(items: &[(&str, bool)]) -> Picker<PathBuf> {
        let all = items
            .iter()
            .map(|(n, locked)| Item {
                key: PathBuf::from(n),
                label: n.to_string(),
                meta: None,
                subtitle: None,
                locked: *locked,
                state: None,
                is_workspace: false,
                is_trace: false,
                reviewable: true,
                hint: None,
                section: Section::None,
                default_shown: true,
                tree_prefix: String::new(),
            })
            .collect();
        let mut p = Picker::new(
            all,
            HashSet::new(),
            "t".to_string(),
            String::new(),
            false,
            Vec::new(),
        );
        p.launcher = true;
        p.multi_select = false; // the real launcher is single-launch (no ticking)
        p
    }

    /// A picker over `(label, section, default_shown)` rows.
    fn sectioned(rows: &[(&str, Section, bool)]) -> Picker<PathBuf> {
        let all = rows
            .iter()
            .map(|(n, section, default_shown)| Item {
                key: PathBuf::from(n),
                label: n.to_string(),
                meta: None,
                subtitle: None,
                locked: false,
                state: None,
                is_workspace: false,
                is_trace: false,
                reviewable: true,
                hint: None,
                section: *section,
                default_shown: *default_shown,
                tree_prefix: String::new(),
            })
            .collect();
        Picker::new(
            all,
            HashSet::new(),
            "t".to_string(),
            "f".to_string(),
            false,
            Vec::new(),
        )
    }

    /// The section headers, in order, in the picker's current display.
    fn headers(p: &Picker<PathBuf>) -> Vec<&'static str> {
        p.display_rows()
            .into_iter()
            .filter_map(|row| match row {
                DisplayRow::Header(h) => Some(h),
                DisplayRow::Blank | DisplayRow::Item(_) | DisplayRow::Subtitle(_) => None,
            })
            .collect()
    }

    #[test]
    fn display_rows_insert_a_header_above_each_section() {
        let p = sectioned(&[
            ("proj", Section::Workspaces, true),
            ("rec", Section::Recent, true),
            ("fold", Section::Folders, true),
        ]);
        assert_eq!(vec!["Workspaces", "Recent", "Folders"], headers(&p));
        // 3 headers + 3 items + a blank spacer above each of the 3 headers.
        let rows = p.display_rows();
        assert_eq!(9, rows.len());
        let blanks = rows
            .iter()
            .filter(|r| matches!(r, DisplayRow::Blank))
            .count();
        assert_eq!(3, blanks);
    }

    #[test]
    fn display_rows_group_consecutive_items_under_one_header() {
        let p = sectioned(&[
            ("a", Section::Recent, true),
            ("b", Section::Recent, true),
            ("c", Section::Folders, true),
        ]);
        // Two Recent items share a single header.
        assert_eq!(vec!["Recent", "Folders"], headers(&p));
    }

    #[test]
    fn non_default_rows_appear_only_when_filtering() {
        let mut p = sectioned(&[
            ("recent-deck", Section::Recent, true),
            ("old-deck", Section::Recent, false),
        ]);
        // No filter: the non-default (non-recent) row is hidden.
        assert_eq!(1, p.filtered.len());
        // A filter that matches it reveals it.
        p.filter = "old".to_string();
        p.refilter();
        assert_eq!(1, p.filtered.len());
        assert_eq!(PathBuf::from("old-deck"), p.all[p.filtered[0]].key);
        // Clearing the filter hides it again.
        p.filter.clear();
        p.refilter();
        assert_eq!(1, p.filtered.len());
        assert_eq!(PathBuf::from("recent-deck"), p.all[p.filtered[0]].key);
    }

    #[test]
    fn cursor_position_maps_to_its_display_line_skipping_headers() {
        // Three sections, one item each. Rows: Blank, Header, Item, Blank, Header,
        // Item, … so the cursor on the 2nd item sits on display line 5.
        let mut p = sectioned(&[
            ("proj", Section::Workspaces, true),
            ("rec", Section::Recent, true),
            ("fold", Section::Folders, true),
        ]);
        p.cursor = 1; // the Recent item
        let line = p
            .display_rows()
            .iter()
            .position(|row| matches!(row, DisplayRow::Item(pos) if *pos == p.cursor));
        assert_eq!(Some(5), line);
    }

    #[test]
    fn launcher_enter_returns_focused_deck() {
        let mut p = launcher_with(&[("a.txt", false), ("b.txt", false)]);
        p.cursor = 1;
        assert_eq!(vec![PathBuf::from("b.txt")], p.result());
    }

    #[test]
    fn launcher_launches_a_drillable_locked_deck() {
        // A prerequisite-locked deck is still drillable (only its exam is gated),
        // so it launches on Enter.
        let mut p = launcher_with(&[("locked.txt", true)]);
        p.cursor = 0;
        p.launch_focused();
        assert!(p.done);
    }

    #[test]
    fn launcher_does_not_tick_traces() {
        // A trace is walked, not card-reviewed, so it can't join a merged review.
        let mut p = launcher_with(&[("walk.txt", false)]);
        p.all[0].is_trace = true;
        p.cursor = 0;
        p.toggle();
        assert!(p.selected.is_empty());
    }

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
    fn truncate_caps_long_labels_with_ellipsis() {
        assert_eq!("short", truncate("short", 10)); // shorter than max: unchanged
        assert_eq!("hello", truncate("hello", 5)); // exactly max: unchanged
        assert_eq!("abcd…", truncate("abcdefghij", 5)); // capped, ends with …
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
    fn single_launch_list_has_nothing_to_tick() {
        // The workspace drill-in (multi_select off) can't tick rows at all.
        let mut p = launcher_with(&[("a.txt", false), ("b.txt", false)]);
        p.multi_select = false;
        p.cursor = 0;
        p.toggle();
        assert!(p.selected.is_empty());
    }

    #[test]
    fn gating_disables_launching_an_unreviewable_deck() {
        // Nothing due + the review launcher is gating: Enter is a no-op, just like
        // a locked deck — you can't start a deck with nothing to review.
        let mut p = launcher_with(&[("done.txt", false)]);
        p.all[0].reviewable = false;
        p.gate_reviewable = true;
        p.cursor = 0;
        p.launch_focused();
        assert!(!p.done);

        // Without gating (browse, or `--cram`), the same deck launches.
        p.gate_reviewable = false;
        p.launch_focused();
        assert!(p.done);
    }

    #[test]
    fn filter_narrows_the_list() {
        let mut p = picker_with(&["rust.txt", "ruby.txt", "go.txt"]);
        assert_eq!(3, p.filtered.len());
        p.filter = "ru".to_string();
        p.refilter();
        assert_eq!(2, p.filtered.len());
        p.filter = "rust".to_string();
        p.refilter();
        assert_eq!(1, p.filtered.len());
    }

    #[test]
    fn filter_is_case_insensitive() {
        let mut p = picker_with(&["Rust.txt", "go.txt"]);
        p.filter = "RUST".to_string();
        p.refilter();
        assert_eq!(1, p.filtered.len());
    }

    #[test]
    fn cursor_clamps_after_filtering() {
        let mut p = picker_with(&["a.txt", "b.txt", "c.txt"]);
        p.cursor = 2;
        p.filter = "a".to_string();
        p.refilter();
        assert_eq!(0, p.cursor);
    }

    #[test]
    fn toggle_selects_under_cursor_returns_selection() {
        let mut p = picker_with(&["a.txt", "b.txt", "c.txt"]);
        p.toggle(); // a
        p.move_cursor(2);
        p.toggle(); // c
        let chosen: Vec<PathBuf> = p
            .all
            .iter()
            .filter(|item| p.selected.contains(&item.key))
            .map(|item| item.key.clone())
            .collect();
        assert_eq!(vec![PathBuf::from("a.txt"), PathBuf::from("c.txt")], chosen);
    }

    #[test]
    fn toggle_is_idempotent_pair() {
        let mut p = picker_with(&["a.txt"]);
        p.toggle();
        assert!(p.selected.contains(&PathBuf::from("a.txt")));
        p.toggle();
        assert!(p.selected.is_empty());
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
}
