//! Every name-taking endpoint resolves through the Catalog owner's cached
//! [`ResolutionMaps`], so no client-supplied name is ever turned into a
//! filesystem path except here.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::Hasher,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Deserialize;
use tiny_http::Request;
use twox_hash::XxHash64;

use super::{SelectOptions, dto::*};
use crate::{
    assemble,
    augment::AugmentCache,
    cache::DeckCache,
    card::Card,
    config::{Config, ReviewConfig},
    deck,
    depth::{Depth, depth_name},
    picker,
    recent::RecentDecks,
    store::Store,
};

/// The generic red-row line: the real failure detail belongs to `alix
/// doctor`, not a picker row.
const PROGRESS_ERROR_META: &str = "progress for this deck cannot be read; run alix doctor";

/// Per-deck IO routing, keyed by each deck's stable id, not its filename.
pub(super) struct DeckFiles {
    pub(super) paths: HashMap<String, PathBuf>,
    /// Absent for a deck whose text couldn't be read (it can't have cards
    /// removed then).
    snapshots: HashMap<String, String>,
    removed: HashMap<String, BTreeSet<usize>>,
    removed_region_lines: HashMap<String, BTreeSet<usize>>,
}

impl DeckFiles {
    pub(super) fn new(paths: HashMap<String, PathBuf>) -> Self {
        let snapshots = paths
            .iter()
            .filter_map(|(deck_id, path)| {
                std::fs::read_to_string(path)
                    .ok()
                    .map(|text| (deck_id.clone(), text))
            })
            .collect();
        Self {
            paths,
            snapshots,
            removed: HashMap::new(),
            removed_region_lines: HashMap::new(),
        }
    }

    pub(super) fn append_note(
        &mut self,
        deck_id: &str,
        card_id: &str,
        notes: &[String],
    ) -> Result<(), String> {
        let path = self
            .paths
            .get(deck_id)
            .ok_or_else(|| format!("no deck file known for {deck_id}"))?;
        crate::personal::append_note(path, deck_id, card_id, notes).map_err(|e| e.to_string())
    }

    /// Best-effort: a rewrite failure only warns, never propagates.
    pub(super) fn remove_block(&mut self, deck_id: &str, line: usize) {
        self.removed
            .entry(deck_id.to_string())
            .or_default()
            .insert(line);
        self.rewrite(deck_id);
    }

    /// A region card's removal address: exact directive lines inside a
    /// surviving block, never a block boundary.
    pub(super) fn remove_region_lines(&mut self, deck_id: &str, lines: &[usize]) {
        self.removed_region_lines
            .entry(deck_id.to_string())
            .or_default()
            .extend(lines.iter().copied());
        self.rewrite(deck_id);
    }

    fn rewrite(&mut self, deck_id: &str) {
        if let (Some(path), Some(original)) = (self.paths.get(deck_id), self.snapshots.get(deck_id))
        {
            let blocks: Vec<usize> = self
                .removed
                .get(deck_id)
                .map(|set| set.iter().copied().collect())
                .unwrap_or_default();
            let exact: Vec<usize> = self
                .removed_region_lines
                .get(deck_id)
                .map(|set| set.iter().copied().collect())
                .unwrap_or_default();
            if let Err(e) = deck::rewrite_without(path, original, &blocks, &exact) {
                eprintln!("warning: could not update {}: {e}", path.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_files_routes_by_deck_id_never_by_the_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("whatever-its-called.md");
        std::fs::write(&path, "## q\na\n").unwrap();
        let mut paths = HashMap::new();
        paths.insert("stable-deck-id".to_string(), path.clone());
        let mut files = DeckFiles::new(paths);

        assert_eq!(Some(&path), files.paths.get("stable-deck-id"));
        assert_eq!(None, files.paths.get("whatever-its-called.md"));

        files
            .append_note("stable-deck-id", "card-q1", &["a note".to_string()])
            .expect("routes to the file through its deck_id, not its filename");
        let text = std::fs::read_to_string(crate::personal::sidecar_path(&path)).unwrap();
        assert!(text.contains("a note"), "sidecar:\n{text}");
        assert_eq!(
            "## q\na\n",
            std::fs::read_to_string(&path).unwrap(),
            "the authored deck is never rewritten"
        );
    }
}

pub(super) fn deck_item_dto(
    e: &picker::DeckEntry,
    store: &Store,
    decks_dir: &Path,
    with_lock: bool,
    augment: &AugmentCache,
    review: ReviewConfig,
    cache: &mut DeckCache,
) -> DeckItemDto {
    let recent = e.last_used_ms.is_some();
    match cache.load(&e.path) {
        Ok(deck) => {
            let s = picker::deck_status(&deck, store, augment, Some(decks_dir), with_lock, review);
            let deck_tokens: HashSet<String> = deck.deck_token.iter().cloned().collect();
            let last_depth = depth_name(
                store
                    .last_depth(deck.deck_token.as_deref().unwrap_or_default())
                    .unwrap_or_else(|| crate::depth::default_depth(&deck.cards, augment)),
            );
            DeckItemDto {
                name: e.name.clone(),
                selectable: assemble::selectable(&e.path),
                label: e.label.clone(),
                meta: if s.progress_error {
                    Some(PROGRESS_ERROR_META.to_string())
                } else {
                    (!s.badge.is_empty()).then_some(s.badge)
                },
                state: if s.progress_error {
                    "error"
                } else {
                    state_name(s.state)
                },
                locked: s.locked,
                reviewable: s.reviewable,
                reviewable_recognize: s.reviewable_recognize,
                can_recognize: s.can_recognize,
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
                has_topology: augment.has_topology_for(&deck_tokens),
                badge_depth: s.badge_depth.map(depth_name),
                badge_dotted: s.badge_dotted,
                new_cards: s.new_cards,
                last_depth,
                deadline: None, // a deadline is a workspace-level setting, not a deck's
            }
        }
        // A deck that fails to load stays selectable (so opening it surfaces
        // the real error), but nothing is honestly reviewable, so those
        // fields are false.
        Err(_) => DeckItemDto {
            name: e.name.clone(),
            selectable: true,
            label: e.label.clone(),
            meta: None,
            state: "new",
            locked: false,
            reviewable: false,
            reviewable_recognize: false,
            can_recognize: false,
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

/// Each member nests under the `requires:` that gates it; badges come from
/// the workspace's own store (or the served root store for a plain folder).
pub(super) fn workspace_members(
    e: &picker::DeckEntry,
    decks_dir: &Path,
    with_lock: bool,
    review: ReviewConfig,
    instance_store: &Store,
    retained: &HashMap<PathBuf, Arc<Store>>,
    cache: &mut DeckCache,
) -> (Vec<MemberDto>, picker::WorkspaceReadiness) {
    let review = review.for_workspace(&e.path);
    let is_ws = cache.is_workspace(&e.path);
    // The owner's projection is authoritative for every document it has
    // ATTEMPTED, active or retained: opening such a store from disk here
    // could resurrect members as new while the owner holds unflushed truth
    // or the document is briefly unavailable (an editor or sync tool
    // mid-rename). A member session's single-document store attempted only
    // its own document, so it must not stand in for the whole workspace (a
    // damaged sibling would resurrect as startable): the workspace view is
    // the retained or freshly opened workspace store with the owner's held
    // document OVERLAID, so sibling truth comes from disk while the owner's
    // document, and any dependency gate reading it, keeps the owner's
    // authoritative verdicts.
    let ws_store_root = crate::workspace::store_path(&e.path);
    let active_in_root = instance_store.path().starts_with(&ws_store_root);
    let projection_covers = is_ws && active_in_root && instance_store.is_aggregate();
    let retained_store = (is_ws && !projection_covers)
        .then(|| {
            retained
                .iter()
                .find(|(path, store)| path.starts_with(&ws_store_root) && store.is_aggregate())
                .map(|(_, store)| Arc::clone(store))
        })
        .flatten();
    // A whole-root failure (the progress root exists but is not a usable
    // directory) is the AllFailed case, matching the listing path: no store,
    // every member red. Flattening it to None would fabricate fresh rows.
    let own_workspace_store = (is_ws && !projection_covers && retained_store.is_none())
        .then(|| crate::state::open_aggregate_store_tolerant(&ws_store_root));
    let root_failed = matches!(own_workspace_store, Some(Err(_)));
    let own_workspace_store = own_workspace_store.and_then(Result::ok);
    let fallback = retained_store.as_deref().or(own_workspace_store.as_ref());
    // Every owner-held document store overlays the base: the retained ones
    // (their document may be mid-rename on disk, or awaiting a save retry)
    // and the active session's last, so the newest verdicts win.
    let overlaid = (is_ws && !projection_covers)
        .then(|| {
            fallback.map(|base| {
                let mut view = base.clone();
                for (path, held) in retained {
                    if path.starts_with(&ws_store_root) {
                        view.overlay_owner(held);
                    }
                }
                if active_in_root {
                    view.overlay_owner(instance_store);
                }
                view
            })
        })
        .flatten();
    let store: Option<&Store> = if !is_ws || projection_covers {
        Some(instance_store)
    } else {
        overlaid.as_ref().or(fallback)
    };
    let paths: Vec<PathBuf> = e.members.iter().map(|m| m.path.clone()).collect();
    let augment = AugmentCache::open_for_workspace(&e.path).ok();
    // Load each member deck once, deriving its status, whether it has a
    // topology, and its last-used session depth from the same parse.
    let loaded: Vec<(Option<picker::DeckStatus>, bool, &'static str)> = paths
        .iter()
        .map(|p| {
            let deck = cache.load(p).ok();
            let status = match (store, augment.as_ref(), deck.as_ref()) {
                (Some(st), Some(a), Some(d)) => Some(picker::deck_status(
                    d,
                    st,
                    a,
                    Some(decks_dir),
                    with_lock,
                    review,
                )),
                _ => None,
            };
            let has_topology = match (augment.as_ref(), deck.as_ref()) {
                (Some(a), Some(d)) => {
                    let tokens: HashSet<String> = d.deck_token.iter().cloned().collect();
                    a.has_topology_for(&tokens)
                }
                _ => false,
            };
            let last_depth = match (store, augment.as_ref(), deck.as_ref()) {
                (Some(st), Some(ag), Some(d)) => st
                    .last_depth(d.deck_token.as_deref().unwrap_or_default())
                    .unwrap_or_else(|| crate::depth::default_depth(&d.cards, ag)),
                _ => Depth::default(),
            };
            (status, has_topology, depth_name(last_depth))
        })
        .collect();
    // A member whose deck failed to load counts toward neither `ready` nor
    // `total` (the rule itself lives in `picker::workspace_readiness`).
    let member_statuses: Vec<picker::DeckStatus> = loaded
        .iter()
        .filter_map(|(status, _, _)| status.clone())
        .collect();
    let readiness = picker::workspace_readiness(&member_statuses);
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
                    meta: if s.progress_error {
                        Some(PROGRESS_ERROR_META.to_string())
                    } else {
                        (!s.badge.is_empty()).then(|| s.badge.clone())
                    },
                    state: if s.progress_error {
                        "error"
                    } else {
                        state_name(s.state)
                    },
                    locked: s.locked,
                    reviewable: s.reviewable,
                    reviewable_recognize: s.reviewable_recognize,
                    can_recognize: s.can_recognize,
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
                // Mirrors deck_item_dto's failed-load fallback: still
                // selectable, nothing reviewable. Whole-root store damage
                // reds the member instead of faking a fresh one.
                None => MemberDto {
                    name: m.name.clone(),
                    selectable: true,
                    label: m.label.clone(),
                    meta: root_failed.then(|| PROGRESS_ERROR_META.to_string()),
                    state: if root_failed { "error" } else { "new" },
                    locked: false,
                    reviewable: false,
                    reviewable_recognize: false,
                    can_recognize: false,
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

/// A config that fails to parse keeps the current dir (the picker must never
/// go down over a typo).
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

/// `with_lock` is false for the browse screen: locking gates review only, so
/// any deck stays browsable.
#[expect(
    clippy::too_many_arguments,
    reason = "the catalog entry point takes the whole build input set"
)]
pub(super) fn deck_catalog(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
    retained: &HashMap<PathBuf, Arc<Store>>,
    with_lock: bool,
    icons: &mut HashMap<String, PathBuf>,
    review: ReviewConfig,
    cache: &mut DeckCache,
) -> Result<DeckListDto, std::io::Error> {
    let mut workspaces = Vec::new();
    let mut recent_decks = Vec::new();
    let mut folders = Vec::new();
    let augment = AugmentCache::open_for_workspace(decks_dir)
        .unwrap_or_else(|_| AugmentCache::open(Path::new("")));
    for e in picker::catalog(decks_dir, recent, cache)? {
        if e.is_workspace {
            let is_ws = cache.is_workspace(&e.path);
            let (members, readiness) =
                workspace_members(&e, decks_dir, with_lock, review, store, retained, cache);
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
            // A group row's `reviewable` is the aggregate of its members (it
            // stays unselectable itself; `selectable: false` below owns that).
            let reviewable = members.iter().any(|m| m.reviewable);
            let reviewable_recognize = members.iter().any(|m| m.reviewable_recognize);
            let can_recognize = members.iter().any(|m| m.can_recognize);
            let reviewable_recall = members.iter().any(|m| m.reviewable_recall);
            let reviewable_reconstruct = members.iter().any(|m| m.reviewable_reconstruct);
            let dto = DeckItemDto {
                meta: Some(meta),
                state: if is_ws { "workspace" } else { "folder" },
                locked: false,
                selectable: false,
                reviewable,
                reviewable_recognize,
                can_recognize,
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
        // A loose deck inside a workspace belongs to it (reached by opening
        // the workspace), so it's excluded from Recent.
        if crate::workspace::root_for_deck(&e.path).is_some() {
            continue;
        }
        recent_decks.push(deck_item_dto(
            &e, store, decks_dir, with_lock, &augment, review, cache,
        ));
    }
    Ok(DeckListDto {
        workspaces,
        recent: recent_decks,
        folders,
    })
}

pub(super) struct Selection {
    pub(super) deck: PathBuf,
    pub(super) opts: SelectOptions,
}

/// Parses a selection body without resolving its name: resolution is the
/// Catalog owner's job, so no filesystem path is ever built from request
/// input outside it.
pub(super) fn parse_selection(request: &mut Request) -> Option<(String, SelectOptions)> {
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
        session: Option<usize>,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    if body.deck.is_empty() {
        return None;
    }
    Some((
        body.deck,
        SelectOptions {
            topology: body.topology,
            region: body.region,
            depth: body.depth,
            cram: body.cram,
            session: body.session,
            // The web serves on the wall clock; only embedders inject time.
            now_ms: None,
        },
    ))
}

/// A name matching more than one container/member resolves to `Ambiguous`
/// (silently picking one was dangerous behind `/api/reset`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Resolved {
    One(PathBuf),
    /// A container row: its directory and member files, so a caller never
    /// reconstructs one from the other.
    Many {
        dir: PathBuf,
        files: Vec<PathBuf>,
    },
    Ambiguous,
    Unknown,
}

/// Resolution treats an unreadable root as "no names known" (requests then
/// 400 as before); only the listing endpoint surfaces the root error itself.
/// The complete name map derived from one discovery pass: row and member
/// names to validated targets, plus directory rows by bare name for
/// destination resolution. A name seen more than once resolves to
/// `Ambiguous`, never silently picking one of several same-named entries.
pub(super) struct ResolutionMaps {
    pub(super) map: HashMap<String, Resolved>,
    pub(super) dirs: HashMap<String, Vec<PathBuf>>,
}

pub(super) fn resolution_maps(
    decks_dir: &Path,
    recent: &RecentDecks,
    cache: &mut DeckCache,
) -> Result<ResolutionMaps, std::io::Error> {
    let mut map: HashMap<String, Resolved> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut dirs: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for e in picker::catalog(decks_dir, recent, cache)? {
        if e.path.is_dir() {
            dirs.entry(e.name.clone()).or_default().push(e.path.clone());
        }
        for m in &e.members {
            if seen.insert(m.name.clone()) {
                map.insert(m.name.clone(), Resolved::One(m.path.clone()));
            } else {
                map.insert(m.name.clone(), Resolved::Ambiguous);
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
            map.insert(e.name, row);
        } else {
            map.insert(e.name, Resolved::Ambiguous);
        }
    }
    Ok(ResolutionMaps { map, dirs })
}

/// Resolution treats an unreadable root as "no names known" (requests then
/// 400 as before); only the listing endpoint surfaces the root error itself.
/// Production resolution goes through the Catalog owner's cached maps; these
/// one-shot wrappers serve the unit tests' fixtures.
#[cfg(test)]
pub(super) fn resolve_row(
    name: &str,
    decks_dir: &Path,
    recent: &RecentDecks,
    cache: &mut DeckCache,
) -> Resolved {
    resolution_maps(decks_dir, recent, cache)
        .ok()
        .and_then(|maps| maps.map.get(name).cloned())
        .unwrap_or(Resolved::Unknown)
}

/// A workspace/folder row collapses to its directory; `/api/reset` instead
/// matches on `Resolved` directly since it wants the member list.
pub(super) fn resolved_path(resolved: Resolved) -> Option<PathBuf> {
    match resolved {
        Resolved::One(p) => Some(p),
        Resolved::Many { dir, .. } => Some(dir),
        Resolved::Ambiguous | Resolved::Unknown => None,
    }
}

/// `None` for an unknown name or one duplicated across containers (the
/// caller then rejects with 400); never a client-crafted path.
#[cfg(test)]
pub(super) fn resolve_dest(
    dest: Option<&str>,
    decks_dir: &Path,
    recent: &RecentDecks,
    cache: &mut DeckCache,
) -> Option<PathBuf> {
    let Some(name) = dest.filter(|d| !d.is_empty()) else {
        return Some(crate::workspace::member_dir(decks_dir));
    };
    let maps = resolution_maps(decks_dir, recent, cache).ok()?;
    match maps.dirs.get(name)?.as_slice() {
        [only] => Some(crate::workspace::member_dir(only)),
        _ => None, // ambiguous: more than one dir row shares this name
    }
}

/// The hex `XxHash64` of the path. Keeps `/img/` safe from traversal, since no
/// user input is ever joined to a path.
pub(super) fn img_key(path: &Path) -> String {
    let mut hasher = XxHash64::default();
    hasher.write(path.to_string_lossy().as_bytes());
    format!("{:016x}", hasher.finish())
}

pub(super) fn collect_images(cards: &[Card]) -> HashMap<String, PathBuf> {
    let mut images = HashMap::new();
    for card in cards {
        for image in card.images.iter().chain(&card.images_back) {
            images.insert(img_key(&image.src), image.src.clone());
        }
        // Frozen diagram rasters ride the same allowlist, or their
        // /img/<key> URLs would 404: nothing else registers them.
        for diagram in &card.resolved_diagrams {
            images.insert(img_key(&diagram.png), diagram.png.clone());
        }
    }
    images
}
