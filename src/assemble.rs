//! Session assembly: turn deck paths into something reviewable.
//!
//! The one place that knows how a selection becomes a session, a walk, or a
//! browse — workspace expansion, augment overlays, topology and region focus,
//! virtual cards, pacing, depth. The server and the CLI both consume it; no
//! policy that changes an `/api/*` response may live outside this module,
//! except the two spec-sanctioned exceptions: recent-recording (the serve
//! arms, conditioned on lib state) and group-row `reviewable*` aggregation
//! (the catalog, folded from member values).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};

use crate::{
    augment::{self, AugmentCache, Topology, TopologyOrder},
    card::Card,
    config::{AskConfig, ReviewConfig},
    deck::{Deck, DeckSettings},
    depth::Depth,
    parser,
    scheduler::Fsrs,
    session::{self, DeckInfo, Order, Session, SessionOptions},
    store::{Store, VirtualCard, default_store_path},
    time::now_ms,
    trace::{SourceBase, Trace, Walk},
    workspace,
};

/// Opens the progress store (creating an empty one on first use).
pub fn open_store(path: Option<PathBuf>) -> Result<Store> {
    let path = match path {
        Some(path) => path,
        None => default_store_path().context("cannot determine the data directory")?,
    };
    Store::open(&path).context("cannot open the progress store")
}

/// Which progress store a set of decks should use: the `--store` override, else
/// the single workspace they all share (a deck is "in" a workspace when its
/// parent folder has an `alix.toml`), else the global default (`None`). Loose
/// decks, a plain folder, or decks spanning different workspaces all fall back
/// to the global store — so a workspace's progress lives with the workspace,
/// while everything else shares the one global store.
pub fn store_path_for(decks: &[PathBuf], cli_override: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = cli_override {
        return Some(path.to_path_buf());
    }
    let mut stores = decks.iter().map(|deck| {
        deck.parent()
            .filter(|p| workspace::is_workspace(p))
            .map(workspace::store_path)
    });
    match stores.next() {
        Some(Some(first)) if stores.all(|s| s.as_ref() == Some(&first)) => Some(first),
        _ => None,
    }
}

/// Opens the store for `paths`: their shared workspace store when they have
/// one, else `instance`'s store (a served folder's own file), else the global
/// default. The fallback a served instance (`alix <dir>` or bare `alix`)
/// applies once no workspace claims the selection.
pub fn store_for(paths: &[PathBuf], instance: Option<&Path>) -> Result<Store> {
    open_store(store_path_for(paths, None).or_else(|| instance.map(Path::to_path_buf)))
}

/// The per-session pacing an instance applies to every session it builds:
/// CLI flag > `[review]` config key > built-in default.
#[derive(Clone, Copy)]
pub struct Pacing {
    pub max_new: usize,
    pub limit: Option<usize>,
}

/// Everything [`select`] needs beyond the picked deck paths and the picker's
/// per-launch choices ([`SelectOptions`]): the instance's review/ask config,
/// whether a trace walk auto-grades, its session pacing, and its instance
/// store path (the served folder's own file, when this deck selection turns
/// out to belong to no workspace).
pub struct AssembleConfig {
    pub review: ReviewConfig,
    pub ask: AskConfig,
    pub trace_auto_grade: bool,
    pub pacing: Pacing,
    pub instance_store: Option<PathBuf>,
}

/// The per-launch choices a selection carries beyond which deck: the picker's
/// depth pick, focus-drawer topology/region scope, the cram tick-box, and
/// optional pacing overrides (absent → the instance's CLI/config values).
#[derive(Default)]
pub struct SelectOptions {
    pub topology: Option<String>,
    pub region: Option<String>,
    pub depth: Option<Depth>,
    pub cram: bool,
    pub max_new: Option<usize>,
    pub limit: Option<usize>,
    /// The session clock (Unix ms); `None` means the wall clock. Select was
    /// the one core path that hardcoded `now_ms()` (everything else threads
    /// time as a parameter), so embedders (the frb bridge, tests) inject here.
    pub now_ms: Option<u64>,
}

/// A review session ready to serve: the session, its header label, the
/// subject → deck file path map used for card removal, and the subject → deck
/// reference links (`% link:`) offered to ask-Claude. Produced by [`select`]
/// when decks are chosen (on the CLI or in the browser picker).
pub struct SessionBuild {
    pub session: Session,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
    pub links: HashMap<String, Vec<String>>,
    /// Subject → its deck's `% source:` project root, for the grounded ask-tutor
    /// (`[ask] source_access`). Only decks with a local source appear.
    pub source_roots: HashMap<String, PathBuf>,
    /// Subject → its deck's source base, for resolving a card's `% at:` citation
    /// excerpt on reveal.
    pub source_bases: HashMap<String, SourceBase>,
    /// The resolved topology name when this session is topology-ordered, so the
    /// server can show the connective cue from that topology. `None` otherwise.
    pub topology_name: Option<String>,
    /// The decks' augment sidecar (distractors, keypoints, notes), already
    /// opened by [`select`] for its format/note overlays and handed on so a
    /// consumer (the frb bridge) can build choice questions from the same
    /// cache instead of re-opening it.
    pub augment: AugmentCache,
}

/// A trace walk ready to serve, built when a single trace deck is picked from the
/// review server's deck-selection screen. The walk is self-graded (no live
/// `--grade`), matching the terminal picker's trace → walk.
pub struct WalkBuild {
    pub walk: Walk,
    /// AI-grades each prediction when set (`[trace] auto_grade` + the ask
    /// config); `None` = self-graded.
    pub grade: Option<AskConfig>,
}

/// What a deck selection resolves to: most selections review; a lone trace
/// deck walks (predict → verify) instead of flattening into a card review.
pub enum Selected {
    Review(SessionBuild),
    Walk(WalkBuild),
}

/// A browse card list ready to serve, with its label and deck paths.
#[derive(Debug)]
pub struct CardsBuild {
    pub cards: Vec<Card>,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
}

/// The result of [`expand_workspaces`]: the deck file(s) to load and the per-deck
/// workspace directive defaults (keyed by file name).
pub struct Expanded {
    pub decks: Vec<PathBuf>,
    pub defaults: HashMap<String, DeckSettings>,
}

/// Resolves each deck file's workspace context: a member file whose parent folder
/// is a workspace inherits that workspace's shared directive defaults (keyed by
/// file name); plain files pass through untagged. A review/browse target is a
/// single deck *file* (whole-workspace review was removed), so this no longer
/// expands a folder — it just tags the file with its workspace's directives.
pub fn expand_workspaces(deck_paths: &[PathBuf]) -> Result<Expanded> {
    let mut decks = Vec::new();
    let mut defaults: HashMap<String, DeckSettings> = HashMap::new();
    for path in deck_paths {
        // A deck file inside a workspace folder inherits its shared directives.
        if let Some(parent) = path.parent()
            && parent.join(workspace::MANIFEST).is_file()
            && let Ok(ws) = workspace::Workspace::load(parent)
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            defaults.insert(name.to_string(), ws.settings);
        }
        decks.push(path.clone());
    }
    Ok(Expanded { decks, defaults })
}

/// Resolves a per-run setting from three sources, most specific first: an
/// explicit CLI flag, then a value declared by the loaded decks (used when
/// they agree), then the built-in default. Decks that disagree fall back to
/// the default with a warning.
fn resolve<T: Copy + PartialEq>(
    name: &str,
    cli: Option<T>,
    declared: impl Iterator<Item = Option<T>>,
    default: T,
) -> T {
    if let Some(value) = cli {
        return value;
    }
    let mut distinct: Vec<T> = Vec::new();
    for value in declared.flatten() {
        if !distinct.contains(&value) {
            distinct.push(value);
        }
    }
    match distinct.as_slice() {
        [] => default,
        [only] => *only,
        _ => {
            eprintln!("warning: decks disagree on `{name}`; using the default");
            default
        }
    }
}

/// Base line number for a synthesized virtual card ([`synthesize_virtual`]) —
/// far past any real deck's line count, so a virtual card's `line` never
/// collides with (and so never shares a sibling group with) a real card's
/// front line.
pub const VIRTUAL_LINE_BASE: usize = 1_000_000;

/// Synthesizes a virtual card's stored deck-format `text` into the real `Card`
/// it stands for — the one in `parse(vc.parent, vc.text)` whose `Card::id`
/// matches `vc.id` (a cloze block yields several sub-cards; the id picks the
/// right hole). `subject` MUST equal `vc.parent`, or the id won't reproduce
/// (`Card::id` hashes the subject). `line` places it far past any real deck
/// line so it never shares a sibling group with a deck card — id-neutral, since
/// `Card::id` ignores `line`. Returns `None` if the text can't be parsed or no
/// card matches (defensive — impossible in practice, but no `unwrap` here).
pub fn synthesize_virtual(vc: &VirtualCard, subject: &Arc<str>, line: usize) -> Option<Card> {
    let mut card = parser::parse_str(subject, &vc.text)
        .ok()?
        .into_iter()
        .find(|c| c.id() == vc.id)?;
    card.line = line;
    Some(card)
}

/// The cards of all loaded decks, a header label, the per-subject deck info
/// for the web session, and the per-deck `% key: value` settings.
pub type LoadedDecks = (
    Vec<Card>,
    String,
    HashMap<String, DeckInfo>,
    Vec<DeckSettings>,
);

/// Loads all decks and returns their cards, a label for the header, the
/// per-subject deck info (file path and reference links) for the web session,
/// and the per-deck `% key: value` settings.
pub fn load_decks(
    paths: &[PathBuf],
    defaults: &HashMap<String, DeckSettings>,
) -> Result<LoadedDecks> {
    let mut cards = Vec::new();
    let mut names = Vec::new();
    let mut decks = HashMap::new();
    let mut settings = Vec::new();
    for path in paths {
        // A deck that belongs to a workspace inherits the workspace's shared
        // directives (keyed by file name); others load with no defaults.
        let deck = match path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| defaults.get(n))
        {
            Some(ws) => Deck::load_with_defaults(path, ws)?,
            None => Deck::load(path)?,
        };
        names.push(deck.display_name());
        decks.insert(
            deck.subject.clone(),
            DeckInfo {
                path: deck.path.clone(),
                // Ask-Claude references include the deck's `% link:`s and any
                // URL `% source:` (a source doubles as a reference).
                links: deck.reference_links(),
                // Where the grounded tutor reads this deck's source (opt-in).
                source_root: deck.source_root(),
                // Resolved against the global config in `select`.
                source_access: false,
                // For resolving a card's `% at:` citation excerpt on reveal.
                source_base: SourceBase::for_deck(&deck),
            },
        );
        settings.push(deck.settings);
        cards.extend(deck.cards);
    }
    Ok((cards, names.join(", "), decks, settings))
}

/// Resolves which stored topology, if any, reorders this session: an explicit
/// `--topology <name>` must name a cached topology (else an error), no flag with
/// exactly one cached topology auto-uses it, and zero-or-several without a name
/// leaves ordering to the scheduler.
fn resolve_topology<'a>(
    name: Option<&str>,
    augment: &'a AugmentCache,
    deck_ids: &std::collections::HashSet<u64>,
) -> Result<Option<&'a Topology>> {
    // Only this deck's topologies — a shared cache (decks sharing a store) holds
    // others', which must not be auto-applied or named here.
    let mine = augment.topologies_for(deck_ids);
    match name {
        Some(name) => match mine.into_iter().find(|t| t.name == name) {
            Some(topology) => Ok(Some(topology)),
            None => bail!(
                "no topology named `{name}` is cached for this deck — run `alix deck augment <deck> --target topology`"
            ),
        },
        None => Ok(match mine.as_slice() {
            [single] => Some(*single),
            _ => None,
        }),
    }
}

/// If a single trace deck was picked, returns its loaded deck — the signal to
/// walk it (predict → verify) rather than flatten it into a card review.
fn single_trace_to_walk(deck_paths: &[PathBuf]) -> Option<Deck> {
    match deck_paths {
        [path] => Deck::load(path).ok().filter(|deck| deck.is_trace()),
        _ => None,
    }
}

/// Subject → deck file path, for the web frontend's card removal.
fn subject_paths(decks: HashMap<String, DeckInfo>) -> HashMap<String, PathBuf> {
    decks
        .into_iter()
        .map(|(subject, info)| (subject, info.path))
        .collect()
}

/// Whether `path` is structurally selectable — the same rule [`select`] bails
/// on for a folder, extracted so the picker catalog can source its
/// `selectable` field from the identical check: `true` for a deck file
/// (including one that fails to parse — that's a load *failure*, not a
/// structural rejection), `false` for a folder that contains decks (a
/// workspace or a plain folder). This is a STRUCTURAL predicate ("is `path`
/// the kind of thing `/api/select` accepts"), not a state one — `reviewable`
/// answers "is there anything due right now", which this never does.
pub fn selectable(path: &Path) -> bool {
    !workspace::has_decks(path)
}

/// Turns a deck selection into something reviewable: most selections resolve
/// to a review session; a lone trace deck resolves to a walk (predict →
/// verify) instead. `% requires:` prerequisites are NOT pulled in — the
/// dependency graph gates exams, not what a review session contains.
///
/// On a review, this also persists the resolved depth (`store.set_last_depth`)
/// so a plain Learn next time reopens at it — even when the built session
/// turns out to have nothing due, matching a restart's expectation.
pub fn select(
    paths: Vec<PathBuf>,
    store: &mut Store,
    cfg: &AssembleConfig,
    opts: &SelectOptions,
) -> Result<Selected> {
    // A single trace picked from the picker walks (predict → verify) rather
    // than flattening to a card review — mirrors the terminal picker's
    // trace → walk.
    if let Some(deck) = single_trace_to_walk(&paths) {
        let trace = Trace::from_deck(&deck)?;
        return Ok(Selected::Walk(WalkBuild {
            walk: Walk::new(trace),
            // Opt-in AI grading of predictions (`[trace] auto_grade`).
            grade: cfg.trace_auto_grade.then(|| cfg.ask.clone()),
        }));
    }

    let deck_paths = paths;
    let topology_sel = opts.topology.as_deref();
    let region_sel = opts.region.as_deref();
    let depth_sel = opts.depth;
    // A session is exactly one deck file's cards — no merging of several loose
    // decks, and no reviewing a whole workspace at once. Workspaces are an
    // organizing layer: review their members one at a time (the picker drills in;
    // `alix workspace <dir>` opens that picker).
    let [deck] = deck_paths.as_slice() else {
        bail!("review one deck at a time (merging decks was removed)");
    };
    if !selectable(deck) {
        bail!(
            "`{}` is a folder — serve it (`alix {}`) and pick a deck inside it",
            deck.display(),
            deck.display()
        );
    }
    // Resolve the deck's workspace context (a member file inherits its workspace's
    // shared directives). `% requires:` prerequisites are NOT pulled in — the
    // dependency graph gates exams, not what a review session contains.
    let expanded = expand_workspaces(&deck_paths)?;
    let (mut cards, deck_label, mut decks, settings) =
        load_decks(&expanded.decks, &expanded.defaults)?;
    // Resolve each deck's effective ask-tutor source access: a deck in a
    // workspace takes that workspace's `source_access` override if it sets one,
    // else the global `[ask] source_access`.
    for info in decks.values_mut() {
        let workspace_override = info
            .path
            .parent()
            .filter(|p| workspace::is_workspace(p))
            .and_then(workspace::manifest_source_access);
        info.source_access = workspace_override.unwrap_or(cfg.ask.source_access);
    }
    // One deck per session, so the label is the deck's own subject.
    let label = deck_label;

    // Every card id in this deck — used to pick out *this* deck's topologies from
    // a cache that may be shared with other decks (one store).
    let deck_ids: std::collections::HashSet<u64> = cards.iter().map(|c| c.id()).collect();

    // Merge in any AI-generated notes from the sidecar cache (`alix deck augment
    // --target notes`) — shown with the card's own deck note on reveal. (Question
    // variants are rotated in per-presentation by the frontends, and distractors
    // are read when a choice question is built.)
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for card in &mut cards {
        // Reshape first (re-renders the deck note, front, answer, mode) …
        augment.apply_format(card);
        // … then stack the notes-target trivia on top of the reshaped note.
        if let Some(note) = augment.note(card.id()) {
            card.append_note(&[note.to_string()]);
        }
    }

    // Resolve the topology that reorders this session (if any) and project it to
    // a session-ready order. The resolved name travels on `SessionBuild` so the
    // web frontend can show the "why this card follows the last" cue from the
    // same topology.
    let topology = resolve_topology(topology_sel, &augment, &deck_ids)?;
    let topology_name = topology.map(|t| t.name.clone());
    let topology_order = topology.map(|t| TopologyOrder::from_walk(&t.walk));

    // `--region` focuses the session on one region of the topology — drill a
    // weak area. SRS still picks what's due *within* that region.
    if let Some(region_name) = region_sel {
        let Some(topology) = topology else {
            bail!("--region needs a topology — pass --topology, or augment one for this deck");
        };
        let Some(region_ids) = topology.region_cards(region_name) else {
            bail!(
                "no region named `{region_name}` in topology `{}`",
                topology.name
            );
        };
        let ids: std::collections::HashSet<u64> = region_ids.iter().copied().collect();
        cards.retain(|c| ids.contains(&c.id()));
    }

    // A workspace member drills under that workspace's `alix.local.toml` pacing
    // override (retention + retirement), else the global `[review]` config.
    let review = cfg
        .review
        .for_workspace(deck.parent().unwrap_or_else(|| Path::new("")));

    // Inject this deck's virtual (remediation) cards alongside its authored
    // ones, so both are drilled by the same FSRS-due queue — but not under a
    // `--region` focus: a region is a deck-topology drill, and virtual cards
    // aren't part of any topology. `decks` has exactly this one deck's entry
    // (one deck per session), keyed by its subject — the same string a
    // virtual card's `parent` is set to.
    let subject: Arc<str> = decks
        .keys()
        .next()
        .map(|s| Arc::from(s.as_str()))
        .unwrap_or_else(|| Arc::from(label.as_str()));
    // Quirk: a `--region` focus always excludes virtual cards — they belong to
    // no topology, so a region drill never injects them.
    if region_sel.is_none() {
        for (k, vc) in store
            .virtual_cards_for(subject.as_ref())
            .into_iter()
            .filter(|v| !session::is_retired_id(v.id, store, review.retire_after_days))
            .filter(|v| !deck_ids.contains(&v.id)) // collision belt-and-suspenders
            .enumerate()
        {
            if let Some(mut card) = synthesize_virtual(vc, &subject, VIRTUAL_LINE_BASE + k) {
                // Reshape/note a synth card exactly as deck cards are above
                // (§8.1) — this loop runs after that one, so it must repeat the
                // same two steps rather than widening the earlier loop's range.
                augment.apply_format(&mut card);
                if let Some(note) = augment.note(card.id()) {
                    card.append_note(&[note.to_string()]);
                }
                cards.push(card);
            }
        }
    }

    // Directives (order) come from the session's decks — the `% order:`
    // directive, else the scheduled default (the CLI override is gone; order
    // is authored, not launched).
    let target_settings: Vec<&DeckSettings> = settings.iter().collect();
    let order = resolve(
        "order",
        None,
        target_settings.iter().map(|s| s.order),
        Order::default(),
    );

    // The session depth: an explicit `--depth` / picker choice, else the deck's
    // last-used depth (keyed by deck subject, like the rest of the deck store),
    // else the default (Recall). The persisted value below lets a plain Learn
    // reopen at it.
    let depth = depth_sel
        .or_else(|| store.last_depth(subject.as_ref()))
        .unwrap_or_default();
    // Pacing: the launch's own overrides win over the instance's flag/config
    // values; cram is purely a per-launch choice (the ▾ menu tick-box).
    let options = SessionOptions {
        max_new: opts.max_new.unwrap_or(cfg.pacing.max_new),
        limit: opts.limit.or(cfg.pacing.limit),
        cram: opts.cram,
        order,
        topology: topology_order,
        retire_after_days: review.retire_after_days,
        depth,
    };
    let session = Session::new(
        cards,
        store,
        Box::new(Fsrs::new(review.retention)),
        options,
        opts.now_ms.unwrap_or_else(now_ms),
    );

    // Remember the resolved depth for this deck so a plain Learn next time
    // reopens at it (keyed by deck subject, like the rest of the deck store).
    // Quirk: this write always fires, even when the session just built above
    // has nothing due — a restart still reopens at the last-chosen depth.
    let resolved_depth = session.depth();
    store.set_last_depth(subject.as_ref(), resolved_depth);
    if let Err(e) = store.save() {
        eprintln!("warning: could not save progress: {e}");
    }

    let links = decks
        .iter()
        .map(|(subject, info)| (subject.clone(), info.links.clone()))
        .collect();
    // Subject → `% source:` project root, but only for decks whose effective
    // source access is on — so the web tutor grounds exactly those.
    let source_roots = decks
        .iter()
        .filter(|(_, info)| info.source_access)
        .filter_map(|(subject, info)| info.source_root.clone().map(|root| (subject.clone(), root)))
        .collect();
    // Subject → source base, so the web can resolve a card's `% at:` citation.
    let source_bases = decks
        .iter()
        .map(|(subject, info)| (subject.clone(), info.source_base.clone()))
        .collect();

    Ok(Selected::Review(SessionBuild {
        session,
        label,
        decks: subject_paths(decks),
        links,
        source_roots,
        source_bases,
        topology_name,
        augment,
    }))
}

/// Builds the browse card list from explicit `paths` (no picker). Mirrors
/// [`select`]'s review path for the read-only browse view: loads decks, but
/// builds no scheduler session.
pub fn browse(paths: Vec<PathBuf>) -> Result<CardsBuild> {
    // One deck file per browse — no merging loose decks or whole workspaces.
    let [deck] = paths.as_slice() else {
        bail!("browse one deck at a time (merging decks was removed)");
    };
    if !selectable(deck) {
        bail!(
            "`{}` is a workspace — browse a deck inside it, or open it with `alix workspace`",
            deck.display()
        );
    }
    let expanded = expand_workspaces(&paths)?;
    let (mut cards, deck_label, decks, _) = load_decks(&expanded.decks, &expanded.defaults)?;
    let label = deck_label;

    // Merge in the display augmentations review shows, from the decks' own store
    // (a workspace's when they share one) — so browse renders the same view, not
    // the raw deck. The raw card stays in the deck file; this is display-only.
    // Quirk: no instance-store fallback (`None`) — browse only ever resolves a
    // workspace's own store, else the global default; the store here locates
    // the augment sidecar (`.path()`) only, nothing is read from or written to it.
    let store = store_for(&expanded.decks, None)?;
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for card in &mut cards {
        // Reshape first (re-renders the deck note, front, answer, mode) …
        augment.apply_format(card);
        // … then stack the notes-target trivia on top of the reshaped note.
        if let Some(note) = augment.note(card.id()) {
            card.append_note(&[note.to_string()]);
        }
    }

    Ok(CardsBuild {
        cards,
        label,
        decks: subject_paths(decks),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{answer::Mode, store::VirtualKind};

    #[test]
    fn selectable_is_false_only_for_a_folder_that_contains_decks() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("d.txt");
        std::fs::write(&file, "# q\n\ta\n").unwrap();
        let ws = dir.path().join("box");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("m.txt"), "# q\n\ta\n").unwrap();
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();

        assert!(selectable(&file), "a deck file is selectable");
        assert!(!selectable(&ws), "a folder of decks is not selectable");
        assert!(selectable(&empty), "an empty folder has no decks to reject");
    }

    #[test]
    fn store_for_prefers_workspace_then_instance_then_global() {
        let dir = tempfile::tempdir().unwrap();
        // workspace: a dir with alix.toml + a member deck
        let ws = dir.path().join("box");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Box\"\n").unwrap();
        let member = ws.join("a.txt");
        std::fs::write(&member, "# q\n  a\n").unwrap();
        // a loose deck outside any workspace
        let loose = dir.path().join("loose.txt");
        std::fs::write(&loose, "# q\n  a\n").unwrap();
        let instance = dir.path().join("instance-progress.json");

        // workspace member -> the workspace's store
        let p = store_path_for(std::slice::from_ref(&member), None).expect("workspace store");
        assert_eq!(p, ws.join("progress.json"));
        // workspace member through store_for: the workspace store wins over the instance fallback
        let s = store_for(std::slice::from_ref(&member), Some(&instance)).unwrap();
        assert_eq!(s.path(), ws.join("progress.json").as_path());
        // loose deck + instance fallback -> the instance store (via store_for)
        let s = store_for(std::slice::from_ref(&loose), Some(&instance)).unwrap();
        assert_eq!(s.path(), instance.as_path());
        // loose deck, no instance -> the global default (assert it is NOT under our tempdir)
        let g = store_for(std::slice::from_ref(&loose), None).unwrap();
        assert!(!g.path().starts_with(dir.path()));
    }

    #[test]
    fn store_path_for_picks_workspace_else_global_else_override() {
        let dir = tempfile::tempdir().unwrap();
        let mk_ws = |name: &str| {
            let ws = dir.path().join(name);
            std::fs::create_dir(&ws).unwrap();
            std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
            std::fs::write(ws.join("a.txt"), "# a\n\t1\n").unwrap();
            std::fs::write(ws.join("b.txt"), "# b\n\t1\n").unwrap();
            ws
        };
        let ws = mk_ws("ws");
        let ws2 = mk_ws("ws2");
        let ws_store = ws.join("progress.json");
        let loose = dir.path().join("loose.txt");
        std::fs::write(&loose, "# c\n\t1\n").unwrap();

        // a deck (or several) in one workspace → that workspace's store
        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("a.txt")], None)
        );
        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("a.txt"), ws.join("b.txt")], None)
        );
        // loose, mixed loose+workspace, and cross-workspace all → global (None)
        assert_eq!(None, store_path_for(std::slice::from_ref(&loose), None));
        assert_eq!(
            None,
            store_path_for(&[ws.join("a.txt"), loose.clone()], None)
        );
        assert_eq!(
            None,
            store_path_for(&[ws.join("a.txt"), ws2.join("a.txt")], None)
        );
        assert_eq!(None, store_path_for(&[], None));
        // --store wins over everything
        let over = dir.path().join("x.json");
        assert_eq!(
            Some(over.clone()),
            store_path_for(&[ws.join("a.txt")], Some(&over))
        );
    }

    /// A minimal two-hop trace deck, mirroring `tests/api.rs`'s `TRACE_DECK`
    /// fixture — enough to classify as a trace (`% trace:` + `% source:`),
    /// not enough to need a real source file for classification itself
    /// (`Trace::from_deck` reads the source lazily, past the point `select`
    /// only needs to know it's a trace at all for this test's fact-deck arm;
    /// the trace arm below supplies a real source file).
    const TRACE_DECK: &str = "% trace: how it works\n\
% source: source.txt\n\
# Predict the first hop\n\
\tit reads the first line\n\
\t% at: 1\n\
# Predict the second hop\n\
\tit reads line two\n\
\t% at: 2\n";

    /// The default per-session pacing/config for a `select` test: built-in
    /// `max_new`, no session cap, default review/ask config.
    fn test_config() -> AssembleConfig {
        AssembleConfig {
            review: ReviewConfig::default(),
            ask: AskConfig::default(),
            trace_auto_grade: false,
            pacing: Pacing {
                max_new: 10,
                limit: None,
            },
            instance_store: None,
        }
    }

    #[test]
    fn a_lone_trace_deck_selects_as_a_walk_and_a_fact_deck_as_a_review() {
        let dir = tempfile::tempdir().unwrap();
        let trace = dir.path().join("t.txt");
        std::fs::write(&trace, TRACE_DECK).unwrap();
        std::fs::write(dir.path().join("source.txt"), "first\nsecond\nthird\n").unwrap();
        let fact = dir.path().join("f.txt");
        std::fs::write(&fact, "# q\n  a\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let cfg = AssembleConfig {
            trace_auto_grade: false,
            ..test_config()
        };
        match select(vec![trace], &mut store, &cfg, &SelectOptions::default()).unwrap() {
            Selected::Walk(_) => {}
            Selected::Review(_) => panic!("trace deck must walk"),
        }
        match select(vec![fact], &mut store, &cfg, &SelectOptions::default()).unwrap() {
            Selected::Review(_) => {}
            Selected::Walk(_) => panic!("fact deck must review"),
        }
    }

    #[test]
    fn single_trace_to_walk_only_for_a_lone_trace_deck() {
        let dir = tempfile::tempdir().unwrap();
        let trace = dir.path().join("t.txt");
        std::fs::write(
            &trace,
            "% trace: how it works\n% source: .\n\n# q\n\tpoint\n\t% at: 1\n",
        )
        .unwrap();
        let fact = dir.path().join("f.txt");
        std::fs::write(&fact, "# q\n\ta\n").unwrap();

        // A lone trace → walk it.
        assert!(single_trace_to_walk(std::slice::from_ref(&trace)).is_some());
        // A lone facts deck → review, not walk.
        assert!(single_trace_to_walk(std::slice::from_ref(&fact)).is_none());
        // A trace alongside other decks isn't a lone trace → review/merge.
        assert!(single_trace_to_walk(&[trace, fact]).is_none());
    }

    #[test]
    fn expand_workspaces_member_file_inherits_workspace_settings() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
        std::fs::write(ws.join("alix.toml"), "[defaults]\ndirection = \"both\"\n").unwrap();

        // A member picked as a bare file (a subset selection) still inherits the
        // workspace's directives.
        let exp = expand_workspaces(&[ws.join("a.txt")]).unwrap();
        assert_eq!(1, exp.decks.len());
        assert_eq!(
            Some(crate::card::Direction::Both),
            exp.defaults.get("a.txt").unwrap().direction
        );
    }

    /// Inserts a virtual (remediation) card for deck `subject` into `store` the
    /// way the substrate does — sidecar content keyed by its `Card::id`, plus a
    /// fresh schedule seeded at `t=0` (so it's due, not treated as unseen).
    fn insert_virtual_card(store: &mut Store, subject: &str) {
        let text = "# virtual front\n\tvirtual back\n".to_string();
        let id = parser::parse_str(subject, &text).unwrap()[0].id();
        store.insert_virtual(VirtualCard {
            id,
            kind: VirtualKind::Remediation,
            parent: subject.to_string(),
            text,
            created_ms: 0,
        });
        store.get_or_insert(id, 0);
    }

    #[test]
    fn select_rejects_a_folder_of_decks() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("animals");
        std::fs::create_dir(&ws).unwrap();
        let member = ws.join("m.txt");
        std::fs::write(&member, "# q\n\ta\n").unwrap();
        // Pin the store explicitly — a bare `None` would fall through to the
        // real global data dir.
        let mut store = store_for(
            std::slice::from_ref(&member),
            Some(&dir.path().join("store.json")),
        )
        .unwrap();

        let err = select(
            vec![ws],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .err()
        .expect("a folder of decks is not a reviewable deck");

        assert!(format!("{err}").contains("is a folder"), "{err}");
    }

    #[test]
    fn select_injects_a_decks_virtual_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.txt");
        std::fs::write(&path, "# q1\n\ta1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override — a
        // bare `None` here would fall through to the real global data dir.
        let mut store = store_for(
            std::slice::from_ref(&path),
            Some(&dir.path().join("store.json")),
        )
        .unwrap();
        insert_virtual_card(&mut store, "rust.txt");

        let Selected::Review(build) = select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap() else {
            panic!("a fact deck must review");
        };
        // The deck's one (new) card, plus the injected due virtual card.
        assert_eq!(2, build.session.initial_size);
    }

    #[test]
    fn region_focus_excludes_virtual_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.txt");
        std::fs::write(&path, "# q1\n\ta1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override — a
        // bare `None` here would fall through to the real global data dir.
        let mut store = store_for(
            std::slice::from_ref(&path),
            Some(&dir.path().join("store.json")),
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        let card_id = deck.cards[0].id();

        // Cache a one-region topology covering this deck's one card.
        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.add_topology(Topology {
            name: "auto".to_string(),
            principle: "test".to_string(),
            edges: vec![],
            walk: vec![card_id],
            regions: vec![augment::TopologyRegion {
                name: "r1".to_string(),
                cards: vec![card_id],
            }],
        });
        cache.save().unwrap();

        // A matching virtual card for this deck.
        insert_virtual_card(&mut store, "rust.txt");

        let Selected::Review(build) = select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions {
                region: Some("r1".to_string()),
                ..Default::default()
            },
        )
        .unwrap() else {
            panic!("a fact deck must review");
        };
        // Only the region's one real card — a `--region` focus is a
        // deck-topology drill, and virtual cards aren't part of any topology.
        assert_eq!(1, build.session.initial_size);
    }

    #[test]
    fn a_format_cache_entry_applies_to_a_synthesized_virtual_card() {
        // A synthesized virtual card has a real `Card::id`, so an existing
        // format-cache entry for that id applies with no change to
        // `apply_format` itself — the "free" half of augment-for-virtuals (§8.1).
        let subject: Arc<str> = Arc::from("rust.txt");
        let text = "# List the parts\n\tA, B, C\n".to_string();
        let id = parser::parse_str(&subject, &text).unwrap()[0].id();
        let vc = VirtualCard {
            id,
            kind: VirtualKind::Remediation,
            parent: subject.to_string(),
            text,
            created_ms: 0,
        };
        let mut synth = synthesize_virtual(&vc, &subject, VIRTUAL_LINE_BASE).unwrap();

        let mut cache =
            AugmentCache::open(std::env::temp_dir().join("nonexistent-augment-virtual.json"));
        cache.set_format(
            id,
            augment::Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: Some(Mode::LineByLine),
            },
        );
        cache.apply_format(&mut synth);

        assert_eq!("Name the parts", synth.front);
        assert_eq!(["A", "B", "C"], *synth.back_for_display());
        assert_eq!(id, synth.id(), "reshaping must not change identity");
    }

    #[test]
    fn select_applies_a_cached_format_to_an_injected_virtual_card() {
        // The display half of augment-for-virtuals (§8.1): `select`'s review
        // arm must reshape an injected synth card the same way it reshapes
        // deck cards.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.txt");
        std::fs::write(&path, "# q1\n\ta1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override — a
        // bare `None` here would fall through to the real global data dir.
        let mut store = store_for(
            std::slice::from_ref(&path),
            Some(&dir.path().join("store.json")),
        )
        .unwrap();
        insert_virtual_card(&mut store, "rust.txt");
        let virtual_id =
            parser::parse_str("rust.txt", "# virtual front\n\tvirtual back\n").unwrap()[0].id();

        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.set_format(
            virtual_id,
            augment::Format {
                front: Some("Reshaped virtual front".to_string()),
                back: vec!["Reshaped virtual back".to_string()],
                note: None,
                mode: None,
            },
        );
        cache.save().unwrap();

        let Selected::Review(build) = select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap() else {
            panic!("a fact deck must review");
        };

        let synth = build
            .session
            .cards()
            .iter()
            .find(|c| c.id() == virtual_id)
            .expect("the injected virtual card should be in the session");
        assert_eq!("Reshaped virtual front", synth.front);
        assert_eq!(["Reshaped virtual back"], *synth.back_for_display());
    }

    #[test]
    fn select_falls_back_to_the_stored_last_depth_before_the_default() {
        use crate::depth::Depth;

        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("d.txt");
        std::fs::write(&deck, "# q\n\ta\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let cfg = test_config();

        // An explicit depth resolves the first session AND persists — assert the
        // persisted value directly (not just the session it produced).
        let explicit = SelectOptions {
            depth: Some(Depth::Recognize),
            ..Default::default()
        };
        select(vec![deck.clone()], &mut store, &cfg, &explicit).unwrap();
        assert_eq!(Some(Depth::Recognize), store.last_depth("d.txt"));

        // No explicit depth this time — falls back to the stored last depth
        // (not the built-in default, Recall).
        let Selected::Review(build) =
            select(vec![deck], &mut store, &cfg, &SelectOptions::default()).unwrap()
        else {
            panic!("a fact deck must review");
        };
        assert_eq!(Depth::Recognize, build.session.depth());
    }

    #[test]
    fn browse_of_a_folder_bails_with_the_workspace_hint() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "# q\n  a\n").unwrap();
        let err = browse(vec![dir.path().to_path_buf()]).unwrap_err();
        assert!(
            err.to_string().contains("browse a deck inside it"),
            "got: {err}"
        );
    }

    #[test]
    fn browse_loads_from_explicit_paths_including_image_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        // A normal card and an image card — both render in the web frontend.
        std::fs::write(
            &path,
            "% img-dir: /imgs\n# plain\n\tanswer\n# pic\n% img: a.png\n\tphoto\n",
        )
        .unwrap();

        let build = browse(vec![path]).unwrap();
        assert_eq!(2, build.cards.len());
    }

    #[test]
    fn browse_applies_a_cached_format_reshape() {
        // A deck in a workspace, so `browse` resolves the workspace's own
        // store (a deterministic temp path) rather than the global store.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Eng\"\n").unwrap();
        let path = ws.join("d.txt");
        std::fs::write(&path, "# List the parts\n\tA, B, C\n").unwrap();

        // Without a cached format, browse shows the raw deck answer.
        let raw = browse(vec![path.clone()]).unwrap();
        let id = raw.cards[0].id();
        assert_eq!(raw.cards[0].back_for_display(), ["A, B, C"]);

        // Cache a format reshape (and a notes-target trivia) for that card in the
        // workspace's augment sidecar.
        let store = store_for(std::slice::from_ref(&path), None).unwrap();
        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.set_format(
            id,
            augment::Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: None,
            },
        );
        cache.set_note(id, "the parts are well known".to_string());
        cache.save().unwrap();

        // Browsing now shows the reshaped front/answer and the trivia note.
        let merged = browse(vec![path]).unwrap();
        assert_eq!(merged.cards[0].front, "Name the parts");
        assert_eq!(merged.cards[0].back_for_display(), ["A", "B", "C"]);
        let note = merged.cards[0].note.clone().unwrap_or_default();
        assert!(note.contains("the parts are well known"), "{note}");
    }

    #[test]
    fn browse_rejects_multiple_decks() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "# q\n\ta\n").unwrap();
        std::fs::write(&b, "# q\n\tb\n").unwrap();
        let err = browse(vec![a, b]).err().unwrap();
        assert!(format!("{err}").contains("one deck"), "{err}");
    }

    #[test]
    fn select_returns_the_decks_augment_cache() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("f.txt");
        std::fs::write(&deck, "# q\n\ta\n").unwrap();
        let store_path = dir.path().join("p.json");
        let mut store = open_store(Some(store_path.clone())).unwrap();
        // Seed the sidecar select will open, next to the store.
        let id = crate::deck::Deck::load(&deck).unwrap().cards[0].id();
        let mut cache = AugmentCache::open(augment::augment_path_for(&store_path));
        cache.set_note(id, "seeded".to_string());
        cache.save().unwrap();

        match select(
            vec![deck],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap()
        {
            Selected::Review(build) => assert_eq!(build.augment.note(id), Some("seeded")),
            Selected::Walk(_) => panic!("a fact deck must review"),
        }
    }

    #[test]
    fn select_serves_by_the_injected_clock() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("f.txt");
        std::fs::write(&deck, "# q\n\ta\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        // Acquire the card at t0: it cools until t0 + ACQUIRE_COOLDOWN_MS,
        // so which side of that line the injected clock falls on decides
        // whether select finds anything to serve.
        let id = crate::deck::Deck::load(&deck).unwrap().cards[0].id();
        let t0 = 1_000_000;
        store.get_or_insert(id, t0);

        let early = SelectOptions {
            now_ms: Some(t0 + 30_000),
            ..Default::default()
        };
        match select(vec![deck.clone()], &mut store, &test_config(), &early).unwrap() {
            Selected::Review(build) => {
                assert!(build.session.is_finished(), "nothing is due 30s in")
            }
            Selected::Walk(_) => panic!("a fact deck must review"),
        }
        let late = SelectOptions {
            now_ms: Some(t0 + 61_000),
            ..Default::default()
        };
        match select(vec![deck], &mut store, &test_config(), &late).unwrap() {
            Selected::Review(build) => {
                assert!(
                    !build.session.is_finished(),
                    "due once the cooldown elapsed"
                )
            }
            Selected::Walk(_) => panic!("a fact deck must review"),
        }
    }
}
