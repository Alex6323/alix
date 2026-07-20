//! Every name-taking endpoint resolves through [`resolve_row`], so no
//! client-supplied name is ever turned into a filesystem path except here.

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
    cache::DeckCache,
    card::Card,
    config::{Config, ReviewConfig},
    deck,
    depth::{Depth, depth_name},
    picker,
    recent::RecentDecks,
    store::Store,
};

pub(super) struct DeckFiles {
    pub(super) paths: HashMap<String, PathBuf>,
    /// Absent for a deck whose text couldn't be read (it can't have cards
    /// removed then).
    snapshots: HashMap<String, String>,
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

    /// Refreshes the snapshot after appending, so a later removal keeps the
    /// new note.
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

    /// Best-effort: a rewrite failure only warns, never propagates.
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
    cache: &mut DeckCache,
) -> (Vec<MemberDto>, picker::WorkspaceReadiness) {
    let review = review.for_workspace(&e.path);
    let is_ws = cache.is_workspace(&e.path);
    let own_workspace_store = is_ws
        .then(|| Store::open(crate::workspace::store_path(&e.path)).ok())
        .flatten();
    let store: Option<&Store> = if is_ws {
        own_workspace_store.as_ref()
    } else {
        Some(instance_store)
    };
    let paths: Vec<PathBuf> = e.members.iter().map(|m| m.path.clone()).collect();
    let augment = store.map(|s| AugmentCache::open(augment::augment_path_for(s.path())));
    // Load each member deck once, deriving its status, whether it has a
    // topology, and its last-used session depth from the same parse.
    let loaded: Vec<(Option<picker::DeckStatus>, bool, &'static str)> = paths
        .iter()
        .map(|p| {
            let deck = cache.load(p).ok();
            // `augment` is `Some` exactly when `store` is, so gating on all
            // three keeps that pairing intact.
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
                    .last_depth(&d.subject)
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
                    meta: Some(s.badge.clone()),
                    state: state_name(s.state),
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
                // selectable, nothing reviewable.
                None => MemberDto {
                    name: m.name.clone(),
                    selectable: true,
                    label: m.label.clone(),
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
pub(super) fn deck_catalog(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
    with_lock: bool,
    icons: &mut HashMap<String, PathBuf>,
    review: ReviewConfig,
    cache: &mut DeckCache,
) -> DeckListDto {
    let mut workspaces = Vec::new();
    let mut recent_decks = Vec::new();
    let mut folders = Vec::new();
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for e in picker::catalog(decks_dir, recent, cache) {
        if e.is_workspace {
            let is_ws = cache.is_workspace(&e.path);
            let (members, readiness) =
                workspace_members(&e, decks_dir, with_lock, review, store, cache);
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
        if e.path.parent().is_some_and(|p| cache.is_workspace(p)) {
            continue;
        }
        recent_decks.push(deck_item_dto(
            &e, store, decks_dir, with_lock, &augment, review, cache,
        ));
    }
    DeckListDto {
        workspaces,
        recent: recent_decks,
        folders,
    }
}

pub(super) struct Selection {
    pub(super) deck: PathBuf,
    pub(super) opts: SelectOptions,
}

/// `None` (→ 400) for any name not in the catalog: no filesystem path is
/// ever built from request input.
pub(super) fn read_selection(
    request: &mut Request,
    decks_dir: &Path,
    recent: &RecentDecks,
    cache: &mut DeckCache,
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
    let deck = resolved_path(resolve_row(&body.deck, decks_dir, recent, cache))?;
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

fn resolve_catalog(
    decks_dir: &Path,
    recent: &RecentDecks,
    cache: &mut DeckCache,
) -> Vec<picker::DeckEntry> {
    picker::catalog(decks_dir, recent, cache)
}

/// A name seen more than once (bare row or qualified member) resolves to
/// `Ambiguous`, never silently picking one of several same-named entries.
pub(super) fn resolve_row(
    name: &str,
    decks_dir: &Path,
    recent: &RecentDecks,
    cache: &mut DeckCache,
) -> Resolved {
    let mut known: HashMap<String, Resolved> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for e in resolve_catalog(decks_dir, recent, cache) {
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
pub(super) fn resolve_dest(
    dest: Option<&str>,
    decks_dir: &Path,
    recent: &RecentDecks,
    cache: &mut DeckCache,
) -> Option<PathBuf> {
    let Some(name) = dest.filter(|d| !d.is_empty()) else {
        return Some(decks_dir.to_path_buf());
    };
    let mut matches = resolve_catalog(decks_dir, recent, cache)
        .into_iter()
        .filter(|e| e.name == name && e.path.is_dir());
    let first = matches.next()?;
    if matches.next().is_some() {
        return None; // ambiguous: more than one dir row shares this name
    }
    Some(first.path)
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
    }
    images
}
