//! The picker's deck/workspace catalog — the DTO builders behind `/api/decks`
//! and `/api/browse` — plus [`resolve_row`], the single name-resolution path
//! every name-taking endpoint (`select`, `browse`, `reset`, `augment`, `share`,
//! …) shares so no client-supplied name is ever turned into a filesystem path
//! except through this lookup.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::Hasher,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use tiny_http::Request;
use twox_hash::XxHash64;

use super::{SelectOptions, dto::*};
use crate::{
    assemble,
    augment::{self, AugmentCache},
    card::Card,
    config::{Config, ReviewConfig},
    deck::{self, Deck},
    depth::{Depth, depth_name},
    picker,
    recent::RecentDecks,
    store::Store,
};

/// Per-deck data the server needs to apply a removal: the file path, plus the
/// file's original text so removals can be re-derived from a fixed snapshot
/// (see [`deck::rewrite_without_cards`]).
pub(super) struct DeckFiles {
    /// Subject → file path.
    pub(super) paths: HashMap<String, PathBuf>,
    /// Subject → original file text (decks whose text could not be read are
    /// absent, and simply cannot have cards removed).
    snapshots: HashMap<String, String>,
    /// Subject → the 1-based front lines removed so far this run.
    removed: HashMap<String, BTreeSet<usize>>,
}

impl DeckFiles {
    pub(super) fn new(paths: HashMap<String, PathBuf>) -> Self {
        let snapshots = paths
            .iter()
            .filter_map(|(subject, path)| {
                std::fs::read_to_string(path)
                    .ok()
                    .map(|text| (subject.clone(), text))
            })
            .collect();
        Self {
            paths,
            snapshots,
            removed: HashMap::new(),
        }
    }

    /// Appends condensed note lines to the card block at `line` of `subject`,
    /// then refreshes the snapshot so a later card removal keeps the new note
    /// (removals rewrite from the snapshot). Returns a message on failure.
    pub(super) fn append_note(
        &mut self,
        subject: &str,
        line: usize,
        notes: &[String],
    ) -> Result<(), String> {
        let path = self
            .paths
            .get(subject)
            .ok_or_else(|| format!("no deck file known for {subject}"))?;
        deck::append_note(path, line, notes).map_err(|e| e.to_string())?;
        if let Ok(text) = std::fs::read_to_string(path) {
            self.snapshots.insert(subject.to_string(), text);
        }
        Ok(())
    }

    /// Records that the card block at `line` of `subject` was removed and
    /// rewrites the deck file from its snapshot. Best-effort.
    pub(super) fn remove_block(&mut self, subject: &str, line: usize) {
        let lines = self.removed.entry(subject.to_string()).or_default();
        lines.insert(line);
        if let (Some(path), Some(original)) = (self.paths.get(subject), self.snapshots.get(subject))
        {
            let lines: Vec<usize> = lines.iter().copied().collect();
            if let Err(e) = deck::rewrite_without_cards(path, original, &lines) {
                eprintln!("warning: could not update {}: {e}", path.display());
            }
        }
    }
}

/// A single loose deck as a selection row, its badge/lock/gating from the
/// shared [`picker::deck_status`].
pub(super) fn deck_item_dto(
    e: &picker::DeckEntry,
    store: &Store,
    decks_dir: &Path,
    with_lock: bool,
    augment: &AugmentCache,
    review: ReviewConfig,
) -> DeckItemDto {
    let recent = e.last_used_ms.is_some();
    match Deck::load(&e.path) {
        Ok(deck) => {
            let s = picker::deck_status(&deck, store, Some(decks_dir), with_lock, review);
            let deck_ids: HashSet<u64> = deck.cards.iter().map(|c| c.id()).collect();
            let last_depth = depth_name(
                store
                    .last_depth(&deck.subject)
                    .unwrap_or_else(|| crate::depth::default_depth(&deck.cards, augment)),
            );
            DeckItemDto {
                name: e.name.clone(),
                selectable: assemble::selectable(&e.path),
                label: e.label.clone(),
                meta: Some(s.badge),
                state: state_name(s.state),
                locked: s.locked,
                reviewable: s.reviewable,
                reviewable_recognize: s.reviewable_recognize,
                reviewable_recall: s.reviewable_recall,
                reviewable_reconstruct: s.reviewable_reconstruct,
                mastered: s.mastered,
                is_trace: s.is_trace,
                examable: s.examable,
                has_exam: s.has_exam,
                recent,
                is_workspace: false,
                description: None,
                members: Vec::new(),
                path: e.path_hint.clone(),
                icon: None,
                icon_svg: false,
                has_topology: augment.has_topology_for(&deck_ids),
                badge_depth: s.badge_depth.map(depth_name),
                badge_dotted: s.badge_dotted,
                new_cards: s.new_cards,
                last_depth,
                deadline: None, // a deadline is a workspace-level setting, not a deck's
            }
        }
        // A deck that fails to load stays launchable so the error surfaces —
        // structurally it's still a deck file (`selectable`), but there's
        // nothing honest to report as due (`reviewable*` all false).
        Err(_) => DeckItemDto {
            name: e.name.clone(),
            selectable: true,
            label: e.label.clone(),
            meta: None,
            state: "new",
            locked: false,
            reviewable: false,
            reviewable_recognize: false,
            reviewable_recall: false,
            reviewable_reconstruct: false,
            mastered: false,
            is_trace: false,
            examable: false,
            has_exam: false,
            recent,
            is_workspace: false,
            description: None,
            members: Vec::new(),
            path: e.path_hint.clone(),
            icon: None,
            icon_svg: false,
            has_topology: false,
            badge_depth: None,
            badge_dotted: false,
            new_cards: false,
            last_depth: depth_name(Depth::default()),
            deadline: None,
        },
    }
}

/// A workspace/folder's members as an unlock dependency tree (the drill-in
/// list): each member nests under the `% requires:` that gates it, siblings
/// startable-first, carrying an `indent` for the tree nesting. Badges/locks come
/// from the workspace's own store (a real workspace) or the served instance's
/// root store (a plain folder) — the same store its top-level loose-deck
/// badges use — matching what a session will write.
///
/// Also returns the [`picker::WorkspaceReadiness`] computed from the same
/// statuses, for a caller building a deadline readout.
pub(super) fn workspace_members(
    e: &picker::DeckEntry,
    decks_dir: &Path,
    with_lock: bool,
    review: ReviewConfig,
    instance_store: &Store,
) -> (Vec<MemberDto>, picker::WorkspaceReadiness) {
    // Member badges reflect this workspace's personal pacing override, if any.
    let review = review.for_workspace(&e.path);
    let is_ws = crate::workspace::is_workspace(&e.path);
    let own_workspace_store = is_ws
        .then(|| Store::open(crate::workspace::store_path(&e.path)).ok())
        .flatten();
    let store: Option<&Store> = if is_ws {
        own_workspace_store.as_ref()
    } else {
        Some(instance_store)
    };
    let paths: Vec<PathBuf> = e.members.iter().map(|m| m.path.clone()).collect();
    // The workspace's own sidecar tells each member whether it has a focus
    // drawer (topology); opened once, alongside the status pass.
    let augment = store.map(|s| AugmentCache::open(augment::augment_path_for(s.path())));
    // Load each member deck once, deriving its status, whether it has a
    // topology, and its last-used session depth from the same parse.
    let loaded: Vec<(Option<picker::DeckStatus>, bool, &'static str)> = paths
        .iter()
        .map(|p| {
            let deck = Deck::load(p).ok();
            let status = match (store, deck.as_ref()) {
                (Some(st), Some(d)) => Some(picker::deck_status(
                    d,
                    st,
                    Some(decks_dir),
                    with_lock,
                    review,
                )),
                _ => None,
            };
            let has_topology = match (augment.as_ref(), deck.as_ref()) {
                (Some(a), Some(d)) => {
                    let ids: HashSet<u64> = d.cards.iter().map(|c| c.id()).collect();
                    a.has_topology_for(&ids)
                }
                _ => false,
            };
            // Subject-keyed like `deck_item_dto`, from the workspace's own store.
            // `augment` is `Some` exactly when `store` is (both come from the
            // same `store.map(...)` above), so the three-way match never needs
            // to unwrap that pairing.
            let last_depth = match (store, augment.as_ref(), deck.as_ref()) {
                (Some(st), Some(ag), Some(d)) => st
                    .last_depth(&d.subject)
                    .unwrap_or_else(|| crate::depth::default_depth(&d.cards, ag)),
                _ => Depth::default(),
            };
            (status, has_topology, depth_name(last_depth))
        })
        .collect();
    // The readiness RULE lives in `picker::workspace_readiness`; this only
    // gathers the statuses it needs (a member whose deck failed to load
    // contributes to neither `ready` nor `total`).
    let member_statuses: Vec<picker::DeckStatus> = loaded
        .iter()
        .filter_map(|(status, _, _)| status.clone())
        .collect();
    let readiness = picker::workspace_readiness(&member_statuses);
    // Order siblings startable-first (blocked = locked, or — when gating —
    // nothing to review), then by label.
    let parent = picker::member_parents(&paths, decks_dir);
    let key: Vec<(bool, String)> = e
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let blocked = loaded[i]
                .0
                .as_ref()
                .is_some_and(|s| s.locked || (with_lock && !s.reviewable));
            (blocked, m.label.clone())
        })
        .collect();
    let members = picker::dependency_forest(&parent, &key)
        .into_iter()
        .map(|(i, prefix)| {
            let m = &e.members[i];
            // Each tree branch segment is three columns wide (see picker).
            let indent = prefix.chars().count() / 3;
            let has_topology = loaded[i].1;
            let last_depth = loaded[i].2;
            match &loaded[i].0 {
                Some(s) => MemberDto {
                    name: m.name.clone(),
                    selectable: assemble::selectable(&m.path),
                    label: m.label.clone(),
                    meta: Some(s.badge.clone()),
                    state: state_name(s.state),
                    locked: s.locked,
                    reviewable: s.reviewable,
                    reviewable_recognize: s.reviewable_recognize,
                    reviewable_recall: s.reviewable_recall,
                    reviewable_reconstruct: s.reviewable_reconstruct,
                    mastered: s.mastered,
                    is_trace: s.is_trace,
                    examable: s.examable,
                    has_exam: s.has_exam,
                    indent,
                    tree: prefix.clone(),
                    has_topology,
                    badge_depth: s.badge_depth.map(depth_name),
                    badge_dotted: s.badge_dotted,
                    new_cards: s.new_cards,
                    last_depth,
                },
                // A member that failed to load: the same neutral defaults as
                // `deck_item_dto`'s failed-load fallback (structurally still
                // selectable; nothing honest to report as reviewable).
                None => MemberDto {
                    name: m.name.clone(),
                    selectable: true,
                    label: m.label.clone(),
                    meta: None,
                    state: "new",
                    locked: false,
                    reviewable: false,
                    reviewable_recognize: false,
                    reviewable_recall: false,
                    reviewable_reconstruct: false,
                    mastered: false,
                    is_trace: false,
                    examable: false,
                    has_exam: false,
                    indent,
                    tree: prefix.clone(),
                    has_topology,
                    badge_depth: None,
                    badge_dotted: false,
                    new_cards: false,
                    last_depth,
                },
            }
        })
        .collect();
    (members, readiness)
}

/// The picker icon URL for a resolved icon path, registering it in the launcher
/// image map so `/img/<key>` can serve it. Returns the URL and whether it is an
/// SVG (a mask) or a raster (`<img>`).
pub(super) fn icon_field(
    icon: Option<&Path>,
    icons: &mut HashMap<String, PathBuf>,
) -> (Option<String>, bool) {
    match icon {
        Some(path) => {
            let key = img_key(path);
            icons.insert(key.clone(), path.to_path_buf());
            let is_svg = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
            (Some(format!("/img/{key}")), is_svg)
        }
        None => (None, false),
    }
}

/// The decks dir this catalog fetch should serve. A scoped instance
/// (`alix <dir>`) is pinned to its root forever; a config-derived instance
/// follows a live `decks_dir` edit on the next reload (the ⟳ button and the
/// focus-refresh both re-fetch /api/decks). A config that no longer parses
/// keeps the current dir — the picker must never go down over a typo; the
/// doctor sheet is where the parse error surfaces.
pub(super) fn effective_decks_dir(
    scoped: bool,
    config_path: Option<&Path>,
    current: &Path,
) -> PathBuf {
    if scoped {
        return current.to_path_buf();
    }
    Config::load(config_path)
        .ok()
        .and_then(|c| c.decks_dir())
        .unwrap_or_else(|| current.to_path_buf())
}

/// Builds the deck-selection catalog's three sections — workspaces (each with
/// its last-progress time), recent loose decks, and plain folders — each
/// deck's badge/lock from `store`. `with_lock` is false for the browse
/// screen: locking gates *review* only, so any deck is browsable.
pub(super) fn deck_catalog(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
    with_lock: bool,
    icons: &mut HashMap<String, PathBuf>,
    review: ReviewConfig,
) -> DeckListDto {
    let mut workspaces = Vec::new();
    let mut recent_decks = Vec::new();
    let mut folders = Vec::new();
    // Opened once for the whole catalog: the served instance's store's sidecar
    // tells each loose deck whether it has a focus drawer (topology).
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for e in picker::catalog(decks_dir, recent) {
        // A workspace/folder row: its members open on click; it has no state of
        // its own. A folder with an `alix.toml` is a workspace (shown with its
        // last-progress time); without one it's a plain folder.
        if e.is_workspace {
            let is_ws = crate::workspace::is_workspace(&e.path);
            let (members, readiness) = workspace_members(&e, decks_dir, with_lock, review, store);
            // A deadline is a real workspace's own setting (`alix.local.toml`);
            // a plain folder never has one.
            let deadline = is_ws
                .then(|| review.for_workspace(&e.path).deadline)
                .flatten()
                .map(|date| {
                    let today = crate::time::local_date(crate::time::now_ms());
                    DeadlineDto {
                        date: date.format("%Y-%m-%d").to_string(),
                        days_left: (date - today).num_days(),
                        ready: readiness.ready,
                        total: readiness.total,
                    }
                });
            let meta = if is_ws {
                match picker::workspace_last_progress(&e.path) {
                    Some(when) => format!("{} decks · {when}", members.len()),
                    None => format!("{} decks", members.len()),
                }
            } else {
                format!("{} decks", members.len())
            };
            let (icon, icon_svg) = icon_field(e.icon.as_deref(), icons);
            // A group row has no due-ness of its own — it's the aggregate of
            // what its members report, not an invitation to select the group
            // itself (`selectable: false` below owns that).
            let reviewable = members.iter().any(|m| m.reviewable);
            let reviewable_recognize = members.iter().any(|m| m.reviewable_recognize);
            let reviewable_recall = members.iter().any(|m| m.reviewable_recall);
            let reviewable_reconstruct = members.iter().any(|m| m.reviewable_reconstruct);
            let dto = DeckItemDto {
                meta: Some(meta),
                state: if is_ws { "workspace" } else { "folder" },
                locked: false,
                selectable: false,
                reviewable,
                reviewable_recognize,
                reviewable_recall,
                reviewable_reconstruct,
                mastered: false,
                is_trace: false,
                examable: false,
                has_exam: false,
                recent: e.last_used_ms.is_some(),
                is_workspace: true,
                description: e.description,
                members,
                path: e.path_hint,
                name: e.name,
                label: e.label,
                icon,
                icon_svg,
                has_topology: false,
                badge_depth: None,
                badge_dotted: false,
                new_cards: false,
                last_depth: depth_name(Depth::default()),
                deadline,
            };
            if is_ws {
                workspaces.push(dto);
            } else {
                folders.push(dto);
            }
            continue;
        }
        // A loose deck inside a workspace belongs to it — reached by opening the
        // workspace, so it isn't listed loose in Recent.
        if e.path.parent().is_some_and(crate::workspace::is_workspace) {
            continue;
        }
        recent_decks.push(deck_item_dto(
            &e, store, decks_dir, with_lock, &augment, review,
        ));
    }
    DeckListDto {
        workspaces,
        recent: recent_decks,
        folders,
    }
}

/// A deck chosen from the picker, optionally scoped by the focus drawer to one
/// topology and/or region, and at a chosen session `depth` (absent = the deck's
/// last-used depth, defaulting to Recall).
pub(super) struct Selection {
    pub(super) deck: PathBuf,
    pub(super) opts: SelectOptions,
}

/// Parses a `{"decks":[name,…]}` selection and resolves each name to its deck
/// path via the live catalog. Returns `None` (→ 400) for an empty or malformed
/// body, or any name not in the catalog — so no filesystem path is ever built
/// from request input, keeping selection safe under `--lan`.
pub(super) fn read_selection(
    request: &mut Request,
    decks_dir: &Path,
    recent: &RecentDecks,
) -> Option<Selection> {
    #[derive(Deserialize)]
    struct Body {
        deck: String,
        #[serde(default)]
        topology: Option<String>,
        #[serde(default)]
        region: Option<String>,
        #[serde(default)]
        depth: Option<Depth>,
        #[serde(default)]
        cram: bool,
        #[serde(default)]
        max_new: Option<usize>,
        #[serde(default)]
        limit: Option<usize>,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    if body.deck.is_empty() {
        return None;
    }
    // Covers top-level decks/workspaces and every workspace's members (by
    // their qualified `<workspace>/<file>` key), so a member selection from
    // inside a workspace resolves safely too. Unknown, crafted, and ambiguous
    // (duplicated bare) names all resolve to nothing here — `/api/select`
    // and `/api/browse` already answer 400 on `None`; `/api/deck-topology`
    // falls back to an empty DTO.
    let deck = resolved_path(resolve_row(&body.deck, decks_dir, recent))?;
    Some(Selection {
        deck,
        opts: SelectOptions {
            topology: body.topology,
            region: body.region,
            depth: body.depth,
            cram: body.cram,
            max_new: body.max_new,
            limit: body.limit,
            // The web serves on the wall clock; only embedders inject time.
            now_ms: None,
        },
    })
}

/// One resolution map for every name-taking endpoint. Bare names that occur
/// more than once (two containers holding decks with the same file name)
/// resolve to `Ambiguous` — callers answer 400 and the client uses the
/// qualified `<workspace>/<file>` key instead; silently picking one of two
/// same-named decks was wrong everywhere and dangerous behind `/api/reset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Resolved {
    One(PathBuf),
    /// A container row (workspace/folder): its own directory and its member
    /// deck files — so no caller ever has to reconstruct one from the other.
    Many {
        dir: PathBuf,
        files: Vec<PathBuf>,
    },
    Ambiguous,
    Unknown,
}

/// Resolves a requested name against the live catalog — the one lookup every
/// name-taking endpoint shares. Qualified member keys (`<workspace>/<file>`)
/// and bare top-level row keys never collide with *each other* (a filename
/// can't contain `/`) — but two same-named containers (e.g. one reached via
/// `decks_dir`, the other only via `recent`) can each hold a same-named
/// member, so both key spaces get the same duplicate tombstoning: any name —
/// bare row or qualified member — seen more than once flips to `Ambiguous`
/// for the rest of this call. No name ever silently picks one of several
/// same-named rows *or* same-named members.
pub(super) fn resolve_row(name: &str, decks_dir: &Path, recent: &RecentDecks) -> Resolved {
    let mut known: HashMap<String, Resolved> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for e in picker::catalog(decks_dir, recent) {
        for m in &e.members {
            if seen.insert(m.name.clone()) {
                known.insert(m.name.clone(), Resolved::One(m.path.clone()));
            } else {
                known.insert(m.name.clone(), Resolved::Ambiguous);
            }
        }
        let row = if e.members.is_empty() {
            Resolved::One(e.path)
        } else {
            Resolved::Many {
                dir: e.path.clone(),
                files: e.members.iter().map(|m| m.path.clone()).collect(),
            }
        };
        if seen.insert(e.name.clone()) {
            known.insert(e.name, row);
        } else {
            known.insert(e.name, Resolved::Ambiguous);
        }
    }
    known.get(name).cloned().unwrap_or(Resolved::Unknown)
}

/// Collapses a [`Resolved`] to the single path `read_selection`/augment/share/
/// share-zip need: a plain deck's own file, or — for a workspace/folder row —
/// its directory, matching what these call sites did before `resolve_row`
/// existed (they used the row's own path rather than expanding to members;
/// `/api/reset` is the one caller that wants the member list, so it matches on
/// `Resolved` directly instead of going through this).
pub(super) fn resolved_path(resolved: Resolved) -> Option<PathBuf> {
    match resolved {
        Resolved::One(p) => Some(p),
        Resolved::Many { dir, .. } => Some(dir),
        Resolved::Ambiguous | Resolved::Unknown => None,
    }
}

/// Resolves an add-sheet destination: absent/empty → the served root; a name
/// → a workspace/folder row's directory, looked up through the same catalog
/// `/api/select` uses (never a client-crafted path). `None` = unknown name, or
/// a name duplicated across containers (same rejection `resolve_row` applies
/// to bare names — dest names are top-level-only, so `catalog` can surface the
/// same duplication) — the caller rejects with 400. Tasks 9 and 11 (`generate`,
/// `receive`) reuse this.
pub(super) fn resolve_dest(
    dest: Option<&str>,
    decks_dir: &Path,
    recent: &RecentDecks,
) -> Option<PathBuf> {
    let Some(name) = dest.filter(|d| !d.is_empty()) else {
        return Some(decks_dir.to_path_buf());
    };
    let mut matches = picker::catalog(decks_dir, recent)
        .into_iter()
        .filter(|e| e.name == name && e.path.is_dir());
    let first = matches.next()?;
    if matches.next().is_some() {
        return None; // ambiguous: more than one dir row shares this name
    }
    Some(first.path)
}

/// A stable, opaque URL key for a resolved image path: the hex `XxHash64` of
/// the path. The card DTO and the image registry derive it the same way, so
/// only paths a deck actually references resolve — no user input is joined to a
/// path, which keeps `/img/` safe from traversal even under `--lan`.
pub(super) fn img_key(path: &Path) -> String {
    let mut hasher = XxHash64::default();
    hasher.write(path.to_string_lossy().as_bytes());
    format!("{:016x}", hasher.finish())
}

/// Builds the `key → absolute path` registry the `/img/` route serves from, by
/// scanning every card's resolved image sides.
pub(super) fn collect_images(cards: &[Card]) -> HashMap<String, PathBuf> {
    let mut images = HashMap::new();
    for card in cards {
        for path in [&card.image, &card.image_back].into_iter().flatten() {
            images.insert(img_key(path), path.clone());
        }
    }
    images
}
