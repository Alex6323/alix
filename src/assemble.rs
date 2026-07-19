use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};

use crate::{
    augment::{self, AugmentCache, Topology, TopologyOrder},
    card::Card,
    config::{AskConfig, ReviewConfig},
    deck::{Deck, DeckSettings},
    depth::{Depth, default_depth},
    l1,
    scheduler::Fsrs,
    session::{self, DeckInfo, Order, Session, SessionOptions},
    stamp,
    store::{Store, VirtualCard, default_store_path},
    time::now_ms,
    trace::{SourceBase, Trace, Walk},
    workspace,
};

pub fn open_store(path: Option<PathBuf>) -> Result<Store> {
    let path = match path {
        Some(path) => path,
        None => default_store_path().context("cannot determine the data directory")?,
    };
    let mut store = Store::open(&path).context("cannot open the progress store")?;
    store.device = crate::store::device_label();
    Ok(store)
}

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

pub fn store_for(paths: &[PathBuf], instance: Option<&Path>) -> Result<Store> {
    open_store(store_path_for(paths, None).or_else(|| instance.map(Path::to_path_buf)))
}

#[derive(Clone, Copy)]
pub struct Pacing {
    pub max_new: usize,
    pub limit: Option<usize>,
}

pub struct AssembleConfig {
    pub review: ReviewConfig,
    pub ask: AskConfig,
    pub trace_auto_grade: bool,
    pub pacing: Pacing,
    pub instance_store: Option<PathBuf>,
}

#[derive(Default)]
pub struct SelectOptions {
    pub topology: Option<String>,
    pub region: Option<String>,
    pub depth: Option<Depth>,
    pub cram: bool,
    pub max_new: Option<usize>,
    pub limit: Option<usize>,
    pub now_ms: Option<u64>,
}

pub struct SessionBuild {
    pub session: Session,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
    pub links: HashMap<String, Vec<String>>,
    pub source_roots: HashMap<String, PathBuf>,
    pub source_bases: HashMap<String, SourceBase>,
    pub topology_name: Option<String>,
    pub augment: AugmentCache,
}

pub struct WalkBuild {
    pub walk: Walk,
    pub grade: Option<AskConfig>,
}

pub enum Selected {
    Review(SessionBuild),
    Walk(WalkBuild),
}

#[derive(Debug)]
pub struct CardsBuild {
    pub cards: Vec<Card>,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
}

pub struct Expanded {
    pub decks: Vec<PathBuf>,
    pub defaults: HashMap<String, DeckSettings>,
}

pub fn expand_workspaces(deck_paths: &[PathBuf]) -> Result<Expanded> {
    let mut decks = Vec::new();
    let mut defaults: HashMap<String, DeckSettings> = HashMap::new();
    for path in deck_paths {
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

/// Far past any real deck's line count, so a virtual card's `line` never
/// collides with a real card's.
pub const VIRTUAL_LINE_BASE: usize = 1_000_000;

/// `subject` must equal `vc.parent`, or the id won't reproduce (`Card::id`
/// hashes the subject).
pub fn synthesize_virtual(vc: &VirtualCard, subject: &Arc<str>, line: usize) -> Option<Card> {
    let mut card = l1::parse_str(subject, &vc.text)
        .ok()?
        .into_iter()
        .find(|c| c.id().as_deref() == Some(vc.id.as_str()))?;
    card.line = line;
    Some(card)
}

pub type LoadedDecks = (
    Vec<Card>,
    String,
    HashMap<String, DeckInfo>,
    Vec<DeckSettings>,
);

pub fn load_decks(
    paths: &[PathBuf],
    defaults: &HashMap<String, DeckSettings>,
) -> Result<LoadedDecks> {
    let mut cards = Vec::new();
    let mut names = Vec::new();
    let mut decks = HashMap::new();
    let mut settings = Vec::new();
    for path in paths {
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
                deck_token: deck.deck_token.clone(),
                links: deck.reference_links(),
                source_root: deck.source_root(),
                source_access: false,
                source_base: SourceBase::for_deck(&deck),
            },
        );
        settings.push(deck.settings);
        cards.extend(deck.cards);
    }
    Ok((cards, names.join(", "), decks, settings))
}

fn resolve_topology<'a>(
    name: Option<&str>,
    augment: &'a AugmentCache,
    deck_tokens: &std::collections::HashSet<String>,
) -> Result<Option<&'a Topology>> {
    // Only this deck's topologies: a shared cache (decks sharing a store) may
    // hold others', which must not be auto-applied or named here.
    let mine = augment.topologies_for(deck_tokens);
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

fn single_trace_to_walk(deck_paths: &[PathBuf]) -> Option<Deck> {
    match deck_paths {
        [path] => Deck::load(path).ok().filter(|deck| deck.is_trace()),
        _ => None,
    }
}

fn subject_paths(decks: HashMap<String, DeckInfo>) -> HashMap<String, PathBuf> {
    decks
        .into_iter()
        .map(|(subject, info)| (subject, info.path))
        .collect()
}

/// A deck file that fails to parse is still selectable: that's a load
/// failure, not a structural rejection.
pub fn selectable(path: &Path) -> bool {
    !workspace::has_decks(path)
}

pub fn stamp_for_session(path: &Path) {
    stamp_for_session_reclaiming(path, &HashMap::new());
}

pub fn stamp_for_session_reclaiming(path: &Path, reclaim: &HashMap<u64, String>) {
    if let Err(e) = stamp::stamp_deck_reclaiming(path, reclaim) {
        eprintln!(
            "warning: cannot stamp {}: {e}; its unstamped cards are excluded from this session",
            path.display()
        );
    }
}

pub fn reclaim_map(store: &Store, path: &Path) -> HashMap<u64, String> {
    let Ok(deck) = Deck::load(path) else {
        return HashMap::new();
    };
    let wanted: HashSet<u64> = deck
        .cards
        .iter()
        .filter(|card| card.token.is_none())
        .map(|card| card.content_fingerprint)
        .collect();
    if wanted.is_empty() {
        return HashMap::new();
    }
    let mut live: HashSet<String> = HashSet::new();
    if let Some(dir) = path.parent() {
        for file in workspace::deck_files(dir) {
            if let Ok(other) = Deck::load(&file) {
                for card in &other.cards {
                    if let Some(token) = card.token.as_deref() {
                        live.insert(token.to_string());
                    }
                }
            }
        }
    }
    store.reclaim_candidates(&live, &wanted)
}

pub fn resolve_duplicates_at_open(path: &Path) {
    let Some(dir) = path.parent() else {
        return;
    };
    for dupe in crate::dedup::scan_dir(dir).card_dupes {
        if dupe.losers.iter().any(|(p, _)| p == path)
            && let Err(e) = stamp::replace_card_token(path, &dupe.token)
        {
            eprintln!(
                "warning: cannot resolve the duplicate token `{}` in {}: {e}",
                dupe.token,
                path.display()
            );
        }
    }
}

fn realign_and_record(store: &mut Store, augment: &mut AugmentCache, cards: &[Card]) -> bool {
    let mut cascaded = false;
    let mut seen: HashSet<&str> = HashSet::new();
    for card in cards {
        let Some(token) = card.token.as_deref() else {
            continue;
        };
        if !seen.insert(token) {
            continue;
        }
        if card.block_holes.is_empty() {
            store.ensure_records(card);
        } else if let Some(outcome) =
            store.realign_card_holes(token, &card.block_holes, card.content_fingerprint)
        {
            augment.remap_holes(token, &outcome);
            cascaded = true;
        }
    }
    cascaded
}

pub fn stamp_and_load_deck(path: &Path) -> Result<Deck> {
    stamp_for_session(path);
    let mut deck = Deck::load(path)?;
    let cards = std::mem::take(&mut deck.cards);
    deck.cards = exclude_unstamped(cards, &deck.subject);
    Ok(deck)
}

pub fn stamp_and_load_cards(files: &[PathBuf]) -> Result<Vec<Card>> {
    let mut cards = Vec::new();
    for path in files {
        cards.extend(stamp_and_load_deck(path)?.cards);
    }
    Ok(cards)
}

pub fn exclude_unstamped(cards: Vec<Card>, label: &str) -> Vec<Card> {
    let before = cards.len();
    let kept: Vec<Card> = cards
        .into_iter()
        .filter(|card| card.id().is_some())
        .collect();
    let dropped = before - kept.len();
    if dropped > 0 {
        eprintln!(
            "warning: {dropped} unstamped card(s) in {label} are excluded from this session \
             (the deck could not be stamped)"
        );
    }
    kept
}

pub fn select(
    paths: Vec<PathBuf>,
    store: &mut Store,
    cfg: &AssembleConfig,
    opts: &SelectOptions,
) -> Result<Selected> {
    if let [path] = paths.as_slice()
        && path.is_file()
    {
        let reclaim = reclaim_map(store, path);
        stamp_for_session_reclaiming(path, &reclaim);
        resolve_duplicates_at_open(path);
    }

    if let Some(mut deck) = single_trace_to_walk(&paths) {
        deck.cards = exclude_unstamped(deck.cards, &deck.subject);
        let trace = Trace::from_deck(&deck)?;
        return Ok(Selected::Walk(WalkBuild {
            walk: Walk::new(trace),
            grade: cfg.trace_auto_grade.then(|| cfg.ask.clone()),
        }));
    }

    let deck_paths = paths;
    let topology_sel = opts.topology.as_deref();
    let region_sel = opts.region.as_deref();
    let depth_sel = opts.depth;
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
    let expanded = expand_workspaces(&deck_paths)?;
    let (cards, deck_label, mut decks, settings) = load_decks(&expanded.decks, &expanded.defaults)?;
    let mut cards = exclude_unstamped(cards, &deck_label);
    for info in decks.values_mut() {
        let workspace_override = info
            .path
            .parent()
            .filter(|p| workspace::is_workspace(p))
            .and_then(workspace::manifest_source_access);
        info.source_access = workspace_override.unwrap_or(cfg.ask.source_access);
    }
    let label = deck_label;

    let deck_tokens: std::collections::HashSet<String> = decks
        .values()
        .filter_map(|d| d.deck_token.clone())
        .collect();
    // Computed before virtual injection adds to `cards`, so it only holds
    // authored ids.
    let deck_card_ids: std::collections::HashSet<String> =
        cards.iter().filter_map(Card::id).collect();

    let mut augment = AugmentCache::open(augment::augment_path_for(store.path()));
    // Records must land before the session build reaches any `get_or_insert`.
    if realign_and_record(store, &mut augment, &cards) {
        if let Err(e) = augment.save() {
            eprintln!("warning: could not save the augment cache: {e}");
        }
        if let Err(e) = store.save() {
            eprintln!("warning: could not save progress: {e}");
        }
    }
    for card in &mut cards {
        augment.apply_format(card);
        if let Some(note) = card
            .id()
            .and_then(|id| augment.note(&id))
            .map(str::to_string)
        {
            card.append_note(&[note]);
        }
    }

    let topology = resolve_topology(topology_sel, &augment, &deck_tokens)?;
    let topology_name = topology.map(|t| t.name.clone());
    let topology_order = topology.map(|t| TopologyOrder::from_walk(&t.walk));

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
        let ids: std::collections::HashSet<String> = region_ids.iter().cloned().collect();
        cards.retain(|c| c.id().is_some_and(|id| ids.contains(&id)));
    }

    let review = cfg
        .review
        .for_workspace(deck.parent().unwrap_or_else(|| Path::new("")));

    let subject: Arc<str> = decks
        .keys()
        .next()
        .map(|s| Arc::from(s.as_str()))
        .unwrap_or_else(|| Arc::from(label.as_str()));
    // Quirk: a `--region` focus always excludes virtual cards (they belong to
    // no topology).
    if region_sel.is_none() {
        for (k, vc) in store
            .virtual_cards_for(subject.as_ref())
            .into_iter()
            .filter(|v| !session::is_retired_id(&v.id, store, review.retire_after_days))
            .filter(|v| !deck_card_ids.contains(&v.id)) // collision belt-and-suspenders
            .enumerate()
        {
            if let Some(mut card) = synthesize_virtual(vc, &subject, VIRTUAL_LINE_BASE + k) {
                // Repeats the deck-card reshape/note steps: this loop runs
                // after virtual cards are added, so it can't merge into the
                // earlier one.
                augment.apply_format(&mut card);
                if let Some(note) = card
                    .id()
                    .and_then(|id| augment.note(&id))
                    .map(str::to_string)
                {
                    card.append_note(&[note]);
                }
                cards.push(card);
            }
        }
    }

    // Order comes from the deck's own setting, not a CLI flag: ordering is
    // authored, not launched.
    let target_settings: Vec<&DeckSettings> = settings.iter().collect();
    let order = resolve(
        "order",
        None,
        target_settings.iter().map(|s| s.order),
        Order::default(),
    );

    let depth = depth_sel
        .or_else(|| store.last_depth(subject.as_ref()))
        .unwrap_or_else(|| default_depth(&cards, &augment));
    let options = SessionOptions {
        max_new: opts.max_new.unwrap_or(cfg.pacing.max_new),
        limit: opts.limit.or(cfg.pacing.limit),
        cram: opts.cram,
        order,
        topology: topology_order,
        retire_after_days: review.retire_after_days,
        depth,
    };
    let now = opts.now_ms.unwrap_or_else(now_ms);
    let tuning = review.deadline.and_then(|date| {
        crate::scheduler::deadline_tuning(
            date,
            review.deadline_ramp_days,
            review.retention,
            crate::time::local_date(now),
            crate::time::end_of_local_day_ms(date),
        )
    });
    // Recognize schedules only cards with cached distractors, so it never
    // degrades to a plain flip; un-augmented cards stay reviewable at other
    // depths.
    let cards = if depth == Depth::Recognize {
        cards
            .into_iter()
            .filter(|c| crate::depth::card_recognizable(c, &augment))
            .collect()
    } else {
        cards
    };
    let session = Session::new(
        cards,
        store,
        Box::new(Fsrs::tuned(
            review.retention,
            review.acquire_cooldown_ms,
            tuning,
        )),
        options,
        now,
    );

    // Quirk: this write always fires even when the built session has nothing
    // due, so a restart still reopens at the last-chosen depth.
    let resolved_depth = session.depth();
    store.set_last_depth(subject.as_ref(), resolved_depth);
    if let Err(e) = store.save() {
        eprintln!("warning: could not save progress: {e}");
    }

    let links = decks
        .iter()
        .map(|(subject, info)| (subject.clone(), info.links.clone()))
        .collect();
    let source_roots = decks
        .iter()
        .filter(|(_, info)| info.source_access)
        .filter_map(|(subject, info)| info.source_root.clone().map(|root| (subject.clone(), root)))
        .collect();
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

pub fn browse(paths: Vec<PathBuf>) -> Result<CardsBuild> {
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

    // Quirk: no instance-store fallback here (browse only resolves a
    // workspace's own store, else the global default).
    let store = store_for(&expanded.decks, None)?;
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for card in &mut cards {
        augment.apply_format(card);
        if let Some(note) = card
            .id()
            .and_then(|id| augment.note(&id))
            .map(str::to_string)
        {
            card.append_note(&[note]);
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
    use crate::{answer::Mode, scheduler::DEFAULT_ACQUIRE_COOLDOWN_MS, store::VirtualKind};

    #[test]
    fn selectable_is_false_only_for_a_folder_that_contains_decks() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("d.md");
        std::fs::write(&file, "## q <!-- id: q1 -->\na\n").unwrap();
        let ws = dir.path().join("box");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("m.md"), "## q <!-- id: qm -->\na\n").unwrap();
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();

        assert!(selectable(&file), "a deck file is selectable");
        assert!(!selectable(&ws), "a folder of decks is not selectable");
        assert!(selectable(&empty), "an empty folder has no decks to reject");
    }

    #[test]
    fn augment_open_stamps_an_unstamped_deck() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## q1\na1\n## q2\na2\n").unwrap();

        let cards = stamp_and_load_cards(std::slice::from_ref(&path)).unwrap();

        assert_eq!(2, cards.len(), "both cards survive with real ids");
        assert!(
            cards.iter().all(|c| c.id().is_some()),
            "every returned card carries a minted token"
        );

        let stamped = std::fs::read_to_string(&path).unwrap();
        assert!(stamped.contains("id: \""), "{stamped:?}");
        assert_eq!(2, stamped.matches("<!-- id: ").count(), "{stamped:?}");
    }

    #[test]
    fn store_for_prefers_workspace_then_instance_then_global() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("box");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Box\"\n").unwrap();
        let member = ws.join("a.md");
        std::fs::write(&member, "## q <!-- id: q1 -->\na\n").unwrap();
        let loose = dir.path().join("loose.md");
        std::fs::write(&loose, "## q <!-- id: q2 -->\na\n").unwrap();
        let instance = dir.path().join("instance-progress.json");

        let p = store_path_for(std::slice::from_ref(&member), None).expect("workspace store");
        assert_eq!(p, ws.join("progress.json"));
        let s = store_for(std::slice::from_ref(&member), Some(&instance)).unwrap();
        assert_eq!(s.path(), ws.join("progress.json").as_path());
        let s = store_for(std::slice::from_ref(&loose), Some(&instance)).unwrap();
        assert_eq!(s.path(), instance.as_path());
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
            std::fs::write(ws.join("a.md"), "## a <!-- id: qa -->\n1\n").unwrap();
            std::fs::write(ws.join("b.md"), "## b <!-- id: qb -->\n1\n").unwrap();
            ws
        };
        let ws = mk_ws("ws");
        let ws2 = mk_ws("ws2");
        let ws_store = ws.join("progress.json");
        let loose = dir.path().join("loose.md");
        std::fs::write(&loose, "## c <!-- id: qc -->\n1\n").unwrap();

        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("a.md")], None)
        );
        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("a.md"), ws.join("b.md")], None)
        );
        assert_eq!(None, store_path_for(std::slice::from_ref(&loose), None));
        assert_eq!(
            None,
            store_path_for(&[ws.join("a.md"), loose.clone()], None)
        );
        assert_eq!(
            None,
            store_path_for(&[ws.join("a.md"), ws2.join("a.md")], None)
        );
        assert_eq!(None, store_path_for(&[], None));
        let over = dir.path().join("x.json");
        assert_eq!(
            Some(over.clone()),
            store_path_for(&[ws.join("a.md")], Some(&over))
        );
    }

    const TRACE_DECK: &str = "---\ntrace: how it works\nsource: source.txt\n---\n\
## Predict the first hop <!-- id: qhop1 -->\n\
it reads the first line\n\
<!-- at: 1 -->\n\
## Predict the second hop <!-- id: qhop2 -->\n\
it reads line two\n\
<!-- at: 2 -->\n";

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
    fn review_open_records_every_deck_card_including_cloze_holes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nid: \"deck1\"\n---\n## Fill <!-- id: fillcard -->\n\
             the \\cloze{alpha} and \\cloze{beta}\n## Plain <!-- id: plaincard -->\nanswer\n",
        )
        .unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();

        select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap();

        let plain = store.records("plaincard").expect("plain card records");
        assert!(plain.holes.is_empty());
        let cloze = store.records("fillcard").expect("cloze card records");
        assert_eq!(2, cloze.holes.len(), "one fingerprint per hole");
    }

    #[test]
    fn reordering_cloze_holes_in_the_file_moves_schedules_through_review_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nid: \"deck1\"\n---\n## Fill <!-- id: fillcard -->\n\\cloze{alpha} then \\cloze{beta}\n",
        )
        .unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();

        select(
            vec![path.clone()],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap();
        store.get_or_insert("fillcard-0", 0).total_reviews = 1;
        store.get_or_insert("fillcard-1", 0).total_reviews = 2;

        std::fs::write(
            &path,
            "---\nid: \"deck1\"\n---\n## Fill <!-- id: fillcard -->\n\\cloze{beta} then \\cloze{alpha}\n",
        )
        .unwrap();
        select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap();

        assert_eq!(1, store.get("fillcard-1").unwrap().total_reviews, "alpha");
        assert_eq!(2, store.get("fillcard-0").unwrap().total_reviews, "beta");
        assert!(
            store.hole_orphans().is_empty(),
            "a pure swap orphans nothing"
        );
    }

    #[test]
    fn an_unstamped_card_matching_an_orphans_content_fingerprint_readopts_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("geo.md");
        std::fs::write(
            &path,
            "---\nid: \"deck1\"\n---\n## Capital of France?\nParis\n",
        )
        .unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();

        let content_fp =
            crate::l1::content_fingerprint("Capital of France?", &["Paris".to_string()]);
        let orphan = "orphantoken0000000000000000";
        store.ensure_records_raw(orphan, content_fp, &[]);
        store.get_or_insert(orphan, 0).total_reviews = 5;

        select(
            vec![path.clone()],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap();

        let stamped = std::fs::read_to_string(&path).unwrap();
        assert!(
            stamped.contains(&format!("<!-- id: {orphan} -->")),
            "expected the reclaimed token in the file: {stamped}"
        );
        assert_eq!(5, store.get(orphan).unwrap().total_reviews);
    }

    #[test]
    fn a_reverse_only_card_reclaims_its_orphaned_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vocab.md");
        let token = "revtok00000000000000000000";
        std::fs::write(
            &path,
            format!("---\nid: \"deck1\"\ndirection: reverse\n---\n## Word <!-- id: {token} -->\nAntwort\n"),
        )
        .unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();

        select(
            vec![path.clone()],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap();
        store.get_or_insert(&format!("{token}-r"), 0).total_reviews = 5;

        std::fs::write(
            &path,
            "---\nid: \"deck1\"\ndirection: reverse\n---\n## Word\nAntwort\n",
        )
        .unwrap();

        select(
            vec![path.clone()],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap();

        let stamped = std::fs::read_to_string(&path).unwrap();
        assert!(
            stamped.contains(&format!("<!-- id: {token} -->")),
            "the reverse-only card should re-adopt its orphaned token: {stamped}"
        );
        assert_eq!(5, store.get(&format!("{token}-r")).unwrap().total_reviews);
    }

    #[test]
    fn a_both_direction_card_reclaims_its_orphaned_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vocab.md");
        let token = "bothtok0000000000000000000";
        std::fs::write(
            &path,
            format!(
                "---\nid: \"deck1\"\ndirection: both\n---\n## Word <!-- id: {token} -->\nAntwort\n"
            ),
        )
        .unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();

        select(
            vec![path.clone()],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap();
        store.get_or_insert(token, 0).total_reviews = 4;

        std::fs::write(
            &path,
            "---\nid: \"deck1\"\ndirection: both\n---\n## Word\nAntwort\n",
        )
        .unwrap();

        select(
            vec![path.clone()],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap();

        let stamped = std::fs::read_to_string(&path).unwrap();
        assert!(
            stamped.contains(&format!("<!-- id: {token} -->")),
            "the both-direction card should re-adopt its orphaned token: {stamped}"
        );
        assert_eq!(4, store.get(token).unwrap().total_reviews);
    }

    #[test]
    fn read_only_scans_never_write_records() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("d.md"),
            "---\nid: \"deck1\"\n---\n## q <!-- id: qcard -->\na\n",
        )
        .unwrap();
        let store_path = workspace::root_store_path(dir.path());
        let mut store = Store::open(&store_path).unwrap();
        store.get_or_insert("qcard", 0);
        store.save().unwrap();
        let before = std::fs::read(&store_path).unwrap();

        crate::listing::list_root(dir.path(), &ReviewConfig::default(), 1000);

        let after = std::fs::read(&store_path).unwrap();
        assert_eq!(
            before, after,
            "a read-only listing must not write the store"
        );
    }

    #[test]
    fn a_lone_trace_deck_selects_as_a_walk_and_a_fact_deck_as_a_review() {
        let dir = tempfile::tempdir().unwrap();
        let trace = dir.path().join("t.md");
        std::fs::write(&trace, TRACE_DECK).unwrap();
        std::fs::write(dir.path().join("source.txt"), "first\nsecond\nthird\n").unwrap();
        let fact = dir.path().join("f.md");
        std::fs::write(&fact, "## q <!-- id: qf -->\na\n").unwrap();
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
        let trace = dir.path().join("t.md");
        std::fs::write(
            &trace,
            "---\ntrace: how it works\nsource: .\n---\n\n## q <!-- id: qq -->\npoint\n<!-- at: 1 -->\n",
        )
        .unwrap();
        let fact = dir.path().join("f.md");
        std::fs::write(&fact, "## q <!-- id: qf -->\na\n").unwrap();

        assert!(single_trace_to_walk(std::slice::from_ref(&trace)).is_some());
        assert!(single_trace_to_walk(std::slice::from_ref(&fact)).is_none());
        assert!(single_trace_to_walk(&[trace, fact]).is_none());
    }

    #[test]
    fn expand_workspaces_member_file_inherits_workspace_settings() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.md"), "## a <!-- id: qa -->\nb\n").unwrap();
        std::fs::write(ws.join("alix.toml"), "[defaults]\ndirection = \"both\"\n").unwrap();

        let exp = expand_workspaces(&[ws.join("a.md")]).unwrap();
        assert_eq!(1, exp.decks.len());
        assert_eq!(
            Some(crate::card::Direction::Both),
            exp.defaults.get("a.md").unwrap().direction
        );
    }

    fn insert_virtual_card(store: &mut Store, subject: &str) {
        let text = "## virtual front <!-- id: vq1 -->\nvirtual back\n".to_string();
        let id = crate::l1::parse_str(subject, &text).unwrap()[0]
            .id()
            .unwrap();
        store.insert_virtual(VirtualCard {
            id: id.clone(),
            kind: VirtualKind::Remediation,
            parent: subject.to_string(),
            text,
            created_ms: 0,
        });
        store.get_or_insert(&id, 0);
    }

    #[test]
    fn select_rejects_a_folder_of_decks() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("animals");
        std::fs::create_dir(&ws).unwrap();
        let member = ws.join("m.md");
        std::fs::write(&member, "## q <!-- id: qm -->\na\n").unwrap();
        // Pin the store explicitly: a bare `None` would fall through to the
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
        let path = dir.path().join("rust.md");
        std::fs::write(&path, "## q1 <!-- id: q1 -->\na1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override: a
        // bare `None` here would fall through to the real global data dir.
        let mut store = store_for(
            std::slice::from_ref(&path),
            Some(&dir.path().join("store.json")),
        )
        .unwrap();
        insert_virtual_card(&mut store, "rust.md");

        let Selected::Review(build) = select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap() else {
            panic!("a fact deck must review");
        };
        assert_eq!(2, build.session.initial_size);
    }

    #[test]
    fn region_focus_excludes_virtual_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.md");
        // A frontmatter `id:` gives the deck a stable token, which topology
        // matching is bound to (not card overlap).
        std::fs::write(&path, "---\nid: dtok1\n---\n## q1 <!-- id: q1 -->\na1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override: a
        // bare `None` here would fall through to the real global data dir.
        let mut store = store_for(
            std::slice::from_ref(&path),
            Some(&dir.path().join("store.json")),
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        let card_id = deck.cards[0].id().unwrap();
        let deck_token = deck.deck_token.clone().unwrap();

        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.add_topology(Topology {
            name: "auto".to_string(),
            principle: "test".to_string(),
            edges: vec![],
            walk: vec![card_id.clone()],
            regions: vec![augment::TopologyRegion {
                name: "r1".to_string(),
                cards: vec![card_id],
            }],
            deck_token,
        });
        cache.save().unwrap();

        insert_virtual_card(&mut store, "rust.md");

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
        assert_eq!(1, build.session.initial_size);
    }

    #[test]
    fn a_format_cache_entry_applies_to_a_synthesized_virtual_card() {
        let subject: Arc<str> = Arc::from("rust.md");
        let text = "## List the parts <!-- id: vlist -->\nA, B, C\n".to_string();
        let id = crate::l1::parse_str(&subject, &text).unwrap()[0]
            .id()
            .unwrap();
        let vc = VirtualCard {
            id: id.clone(),
            kind: VirtualKind::Remediation,
            parent: subject.to_string(),
            text,
            created_ms: 0,
        };
        let mut synth = synthesize_virtual(&vc, &subject, VIRTUAL_LINE_BASE).unwrap();

        let mut cache =
            AugmentCache::open(std::env::temp_dir().join("nonexistent-augment-virtual.json"));
        cache.set_format(
            &id,
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
        assert_eq!(Some(id), synth.id(), "reshaping must not change identity");
    }

    #[test]
    fn select_applies_a_cached_format_to_an_injected_virtual_card() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.md");
        std::fs::write(&path, "## q1 <!-- id: q1 -->\na1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override: a
        // bare `None` here would fall through to the real global data dir.
        let mut store = store_for(
            std::slice::from_ref(&path),
            Some(&dir.path().join("store.json")),
        )
        .unwrap();
        insert_virtual_card(&mut store, "rust.md");
        let virtual_id = crate::l1::parse_str(
            "rust.md",
            "## virtual front <!-- id: vq1 -->\nvirtual back\n",
        )
        .unwrap()[0]
            .id()
            .unwrap();

        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.set_format(
            &virtual_id,
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
            .find(|c| c.id().as_deref() == Some(virtual_id.as_str()))
            .expect("the injected virtual card should be in the session");
        assert_eq!("Reshaped virtual front", synth.front);
        assert_eq!(["Reshaped virtual back"], *synth.back_for_display());
    }

    #[test]
    fn select_falls_back_to_the_stored_last_depth_before_the_default() {
        use crate::depth::Depth;

        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("d.md");
        std::fs::write(&deck, "## q <!-- id: q1 -->\na\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let cfg = test_config();

        let explicit = SelectOptions {
            depth: Some(Depth::Recognize),
            ..Default::default()
        };
        select(vec![deck.clone()], &mut store, &cfg, &explicit).unwrap();
        assert_eq!(Some(Depth::Recognize), store.last_depth("d.md"));

        let Selected::Review(build) =
            select(vec![deck], &mut store, &cfg, &SelectOptions::default()).unwrap()
        else {
            panic!("a fact deck must review");
        };
        assert_eq!(Depth::Recognize, build.session.depth());
    }

    #[test]
    fn select_defaults_a_never_drilled_deck_to_recognize_when_choices_are_cached() {
        use crate::depth::Depth;

        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("d.md");
        std::fs::write(&deck_path, "## q <!-- id: q1 -->\na\n").unwrap();
        let store_path = dir.path().join("p.json");
        let mut store = open_store(Some(store_path.clone())).unwrap();
        let cfg = test_config();

        // Distractor set keyed by the real id, read from the loaded deck
        // (never hand-computed).
        let card_id = Deck::load(&deck_path).unwrap().cards[0].id().unwrap();
        let mut cache = AugmentCache::open(augment::augment_path_for(&store_path));
        cache.set_distractors(&card_id, vec!["w1".into(), "w2".into(), "w3".into()]);
        cache.save().unwrap();

        let Selected::Review(build) =
            select(vec![deck_path], &mut store, &cfg, &SelectOptions::default()).unwrap()
        else {
            panic!("a fact deck must review");
        };
        assert_eq!(Depth::Recognize, build.session.depth());
    }

    #[test]
    fn select_keeps_recall_for_a_never_drilled_unaugmented_deck() {
        use crate::depth::Depth;

        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("d.md");
        std::fs::write(&deck_path, "## q <!-- id: q1 -->\na\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let cfg = test_config();

        let Selected::Review(build) =
            select(vec![deck_path], &mut store, &cfg, &SelectOptions::default()).unwrap()
        else {
            panic!("a fact deck must review");
        };
        assert_eq!(Depth::Recall, build.session.depth());
    }

    #[test]
    fn browse_of_a_folder_bails_with_the_workspace_hint() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "## q <!-- id: qa -->\na\n").unwrap();
        let err = browse(vec![dir.path().to_path_buf()]).unwrap_err();
        assert!(
            err.to_string().contains("browse a deck inside it"),
            "got: {err}"
        );
    }

    #[test]
    fn browse_loads_from_explicit_paths_including_image_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nimg-dir: /imgs\n---\n## plain\nanswer\n## pic <!-- img: a.png -->\nphoto\n",
        )
        .unwrap();

        let build = browse(vec![path]).unwrap();
        assert_eq!(2, build.cards.len());
    }

    #[test]
    fn browse_applies_a_cached_format_reshape() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Eng\"\n").unwrap();
        let path = ws.join("d.md");
        std::fs::write(&path, "## List the parts <!-- id: qlist -->\nA, B, C\n").unwrap();

        let raw = browse(vec![path.clone()]).unwrap();
        let id = raw.cards[0].id().unwrap();
        assert_eq!(raw.cards[0].back_for_display(), ["A, B, C"]);

        let store = store_for(std::slice::from_ref(&path), None).unwrap();
        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.set_format(
            &id,
            augment::Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: None,
            },
        );
        cache.set_note(&id, "the parts are well known".to_string());
        cache.save().unwrap();

        let merged = browse(vec![path]).unwrap();
        assert_eq!(merged.cards[0].front, "Name the parts");
        assert_eq!(merged.cards[0].back_for_display(), ["A", "B", "C"]);
        let note = merged.cards[0].note.clone().unwrap_or_default();
        assert!(note.contains("the parts are well known"), "{note}");
    }

    #[test]
    fn browse_rejects_multiple_decks() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "## q <!-- id: qa -->\na\n").unwrap();
        std::fs::write(&b, "## q <!-- id: qb -->\nb\n").unwrap();
        let err = browse(vec![a, b]).err().unwrap();
        assert!(format!("{err}").contains("one deck"), "{err}");
    }

    #[test]
    fn select_returns_the_decks_augment_cache() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("f.md");
        std::fs::write(&deck, "## q <!-- id: q1 -->\na\n").unwrap();
        let store_path = dir.path().join("p.json");
        let mut store = open_store(Some(store_path.clone())).unwrap();
        let id = crate::deck::Deck::load(&deck).unwrap().cards[0]
            .id()
            .unwrap();
        let mut cache = AugmentCache::open(augment::augment_path_for(&store_path));
        cache.set_note(&id, "seeded".to_string());
        cache.save().unwrap();

        match select(
            vec![deck],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap()
        {
            Selected::Review(build) => assert_eq!(build.augment.note(&id), Some("seeded")),
            Selected::Walk(_) => panic!("a fact deck must review"),
        }
    }

    #[test]
    fn a_configured_acquire_cooldown_reaches_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("f.md");
        std::fs::write(&deck, "## q <!-- id: q1 -->\na\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let id = crate::deck::Deck::load(&deck).unwrap().cards[0]
            .id()
            .unwrap();
        let t0 = 1_000_000;
        store.get_or_insert(&id, t0);

        let mut config = test_config();
        config.review.acquire_cooldown_ms = 1_000;
        let opts = SelectOptions {
            now_ms: Some(t0 + 2_000),
            ..Default::default()
        };
        match select(vec![deck], &mut store, &config, &opts).unwrap() {
            Selected::Review(build) => assert!(
                !build.session.is_finished(),
                "served once the short cooldown passed"
            ),
            Selected::Walk(_) => panic!("a fact deck must review"),
        }
    }

    #[test]
    fn select_serves_by_the_injected_clock() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("f.md");
        std::fs::write(&deck, "## q <!-- id: q1 -->\na\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let id = crate::deck::Deck::load(&deck).unwrap().cards[0]
            .id()
            .unwrap();
        let t0 = 1_000_000;
        store.get_or_insert(&id, t0);

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
            now_ms: Some(t0 + DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000),
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

    #[test]
    fn a_workspace_deadline_ceilings_what_a_session_schedules() {
        let dir = tempfile::tempdir().unwrap();
        // The deadline overlay only fires inside a real workspace (manifest present).
        std::fs::write(dir.path().join("alix.toml"), "title = \"W\"\n").unwrap();
        let deck = dir.path().join("m.md");
        std::fs::write(&deck, "## q <!-- id: q1 -->\na\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let id = crate::deck::Deck::load(&deck).unwrap().cards[0]
            .id()
            .unwrap();

        let now = crate::time::now_ms();
        store.get_or_insert(&id, now).recall = Some(crate::store::FsrsState {
            stability: 200.0,
            difficulty: 5.0,
            state: 2,
            reps: 10,
            scheduled_days: 90,
            last_review_ms: now.saturating_sub(90 * 86_400_000),
            due_ms: now.saturating_sub(1_000), // due now
            ..Default::default()
        });

        let deadline = crate::time::local_date(now) + chrono::Days::new(3);
        std::fs::write(
            dir.path().join("alix.local.toml"),
            format!("[review]\ndeadline = \"{}\"\n", deadline.format("%Y-%m-%d")),
        )
        .unwrap();

        let opts = SelectOptions {
            now_ms: Some(now),
            ..Default::default()
        };
        let Selected::Review(mut build) =
            select(vec![deck], &mut store, &test_config(), &opts).unwrap()
        else {
            panic!("a fact deck must review");
        };
        build
            .session
            .grade(&mut store, crate::scheduler::Grade::Pass, now);

        let ceiling = crate::time::end_of_local_day_ms(deadline);
        let due = store.get(&id).unwrap().recall.unwrap().due_ms;
        assert!(
            due <= ceiling,
            "due {due} must respect the deadline ceiling {ceiling}"
        );
    }
    #[test]
    fn review_open_stamps_the_deck_and_serves_it() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("fresh.md");
        std::fs::write(&deck, "## q1\na\n\n## q2\nb\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let Selected::Review(build) = select(
            vec![deck.clone()],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap() else {
            panic!("expected a review");
        };
        let text = std::fs::read_to_string(&deck).unwrap();
        assert_eq!(2, text.matches("<!-- id: ").count(), "{text}");
        assert!(!build.session.cards().is_empty());
        assert!(build.session.cards().iter().all(|c| c.id().is_some()));
    }

    #[test]
    fn duplicate_card_tokens_are_detected_read_only_and_resolved_at_review_open() {
        let dir = tempfile::tempdir().unwrap();
        // The dedup tie-break picks the undecorated name as keeper, so
        // `notes.md` keeps `cshared` and `notes copy.md` loses it.
        let keeper = dir.path().join("notes.md");
        std::fs::write(
            &keeper,
            "---\nid: \"dtoka\"\n---\n## q <!-- id: cshared -->\na\n",
        )
        .unwrap();
        let loser = dir.path().join("notes copy.md");
        std::fs::write(
            &loser,
            "---\nid: \"dtokb\"\n---\n## q <!-- id: cshared -->\nb\n",
        )
        .unwrap();

        let before = std::fs::read_to_string(&loser).unwrap();
        let map = crate::dedup::scan_dir(dir.path());
        assert_eq!(before, std::fs::read_to_string(&loser).unwrap());
        assert_eq!(1, map.card_dupes.len());
        assert_eq!("cshared", map.card_dupes[0].token);
        assert_eq!(keeper.clone(), map.card_dupes[0].keeper.0);

        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        store.get_or_insert("cshared", 1_000);
        store.save().unwrap();

        let Selected::Review(build) = select(
            vec![loser.clone()],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap() else {
            panic!("a fact deck must review");
        };

        assert!(
            !std::fs::read_to_string(&loser).unwrap().contains("cshared"),
            "the loser deck's token must be re-minted"
        );
        assert!(
            std::fs::read_to_string(&keeper)
                .unwrap()
                .contains("cshared")
        );
        assert!(store.get("cshared").is_some());
        let served = build.session.cards()[0].id().unwrap();
        assert_ne!(
            "cshared", served,
            "the loser's card forked to a fresh token"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_stamp_failure_excludes_unstamped_cards_loudly() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join("decks");
        std::fs::create_dir(&decks).unwrap();
        let deck = decks.join("half.md");
        std::fs::write(&deck, "## a <!-- id: q1 -->\n1\n\n## b\n2\n").unwrap();
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        std::fs::set_permissions(&decks, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = select(
            vec![deck.clone()],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        );
        std::fs::set_permissions(&decks, std::fs::Permissions::from_mode(0o755)).unwrap();
        let Selected::Review(build) = result.unwrap() else {
            panic!("expected a review");
        };
        let cards = build.session.cards();
        assert_eq!(1, cards.len(), "the tokenless card must be excluded");
        assert_eq!(Some("q1".to_string()), cards[0].id());
    }
}
