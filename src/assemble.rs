use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};

#[cfg(test)]
use crate::augment;
use crate::{
    augment::{AugmentCache, Topology, TopologyOrder},
    card::Card,
    config::{AskConfig, ReviewConfig},
    deck::{Deck, DeckSettings, SourceLayers},
    depth::{Depth, default_depth},
    scheduler::Fsrs,
    session::{DeckInfo, Order, Session, SessionOptions},
    source::SourceBase,
    stamp, state,
    store::{Store, default_store_path},
    time::now_ms,
    trace::{Trace, Walk},
    workspace,
};

pub fn open_store(path: Option<PathBuf>) -> Result<Store> {
    let path = match path {
        Some(path) => path,
        None => default_store_path().context("cannot determine the data directory")?,
    };
    state::open_aggregate_store(&path).context("cannot open the progress store")
}

/// The server-boot open: an unreadable document must not keep the picker
/// from serving (its deck reds out per document instead), while a command
/// that operates on the whole store (`reset --all`) keeps the strict
/// [`open_store`] and fails loud on the same damage.
pub fn open_store_tolerant(path: Option<PathBuf>) -> Result<Store> {
    let path = match path {
        Some(path) => path,
        None => default_store_path().context("cannot determine the data directory")?,
    };
    state::open_aggregate_store_tolerant(&path).context("cannot open the progress store")
}

pub fn store_path_for(decks: &[PathBuf], cli_override: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = cli_override {
        return Some(path.to_path_buf());
    }
    let mut stores = decks
        .iter()
        .map(|deck| workspace::root_for_deck(deck).map(|root| workspace::store_path(&root)));
    match stores.next() {
        Some(Some(first)) if stores.all(|s| s.as_ref() == Some(&first)) => Some(first),
        _ => None,
    }
}

pub fn store_for(paths: &[PathBuf], instance: Option<&Path>) -> Result<Store> {
    store_for_with_default(paths, instance, default_store_path())
}

fn store_for_with_default(
    paths: &[PathBuf],
    instance: Option<&Path>,
    default: Option<PathBuf>,
) -> Result<Store> {
    let user_root = store_path_for(paths, None).or_else(|| instance.map(Path::to_path_buf));
    let user_root = match user_root {
        Some(path) => path,
        None => default.context("cannot determine the data directory")?,
    };
    state::open_stores(paths, &user_root).context("cannot open deck progress")
}

#[derive(Clone, Copy)]
pub struct Pacing {
    pub max_session: usize,
    pub new_cards_percent: u8,
}

pub struct AssembleConfig {
    pub review: ReviewConfig,
    pub ask: AskConfig,
    pub pacing: Pacing,
    pub instance_store: Option<PathBuf>,
}

#[derive(Default)]
pub struct SelectOptions {
    pub topology: Option<String>,
    pub region: Option<String>,
    pub depth: Option<Depth>,
    pub cram: bool,
    /// A per-launch override of `max_session` for this sitting.
    pub session: Option<usize>,
    pub now_ms: Option<u64>,
}

pub struct SessionBuild {
    pub session: Session,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
    /// Deck-load diagnostics for the whole selection, surfaced by the
    /// session so a silent fallback cannot pass for a successful freeze.
    pub load_warnings: Vec<String>,
    pub links: HashMap<String, Vec<String>>,
    pub source_layers: HashMap<String, SourceLayers>,
    pub base_roots: HashMap<String, PathBuf>,
    pub source_bases: HashMap<String, SourceBase>,
    pub topology_name: Option<String>,
    pub region_name: Option<String>,
    pub augment: AugmentCache,
}

pub struct WalkBuild {
    pub walk: Walk,
}

#[allow(
    clippy::large_enum_variant,
    reason = "boxing the common review build adds indirection to a short-lived selection result"
)]
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
        if let Some(root) = workspace::root_for_deck(path)
            && let Ok(ws) = workspace::Workspace::load(root)
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

/// Far past any real deck's line count, so a personal card's `line` never
/// collides with a deck card's.
pub const PERSONAL_LINE_BASE: usize = 1_000_000;

/// Personal cards are addressed to the deck they sit beside, not to the
/// sidecar, which carries no `id:` of its own.
pub fn bind_personal(cards: &mut [Card], subject: &Arc<str>, deck_id: &Arc<str>) {
    for (k, card) in cards.iter_mut().enumerate() {
        card.line = PERSONAL_LINE_BASE + k;
        card.subject = Arc::clone(subject);
        card.deck_id = Arc::clone(deck_id);
    }
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
            deck.deck_token.clone().unwrap_or_default(),
            DeckInfo {
                path: deck.path.clone(),
                deck_token: deck.deck_token.clone(),
                links: deck.reference_links(),
                source_layers: deck.source_layers(),
                base_root: deck.base_root(),
                source_access: false,
                source_base: SourceBase::for_deck(&deck),
                load_warnings: deck.load_warnings.clone(),
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
                "no review order named `{name}` is cached for this deck; run `alix deck augment <deck> --target order`"
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

fn deck_id_paths(decks: HashMap<String, DeckInfo>) -> HashMap<String, PathBuf> {
    decks
        .into_values()
        .map(|info| (info.deck_token.clone().unwrap_or_default(), info.path))
        .collect()
}

/// A deck file that fails to parse is still selectable: that's a load
/// failure, not a structural rejection.
pub fn selectable(path: &Path) -> bool {
    !workspace::has_decks(path)
}

pub fn stamp_for_session(path: &Path) -> Result<()> {
    match stamp::stamp_initialized_deck(path) {
        Ok(_) => Ok(()),
        Err(e @ stamp::StampError::Uninitialized { .. }) => Err(e.into()),
        Err(e) => {
            eprintln!(
                "warning: cannot stamp {}: {e}; its unstamped cards are excluded from this session",
                path.display()
            );
            Ok(())
        }
    }
}

pub fn resolve_duplicates_at_open(path: &Path) {
    let Some(dir) = path.parent() else {
        return;
    };
    // The line scan over-claims, so it may gate the cost but never authorize
    // the write.
    if !crate::dedup::any_repeated_card_token_fast(dir) {
        return;
    }
    let map = crate::dedup::scan_dir(dir);
    // One authored block holds one `id:`, however many review units it expands
    // to, so every composed collision over that block is one repair.
    let mut blocks: BTreeSet<(&str, usize, &Path)> = BTreeSet::new();
    for dupe in &map.card_dupes {
        for (_, block_line) in dupe.losers.iter().filter(|(p, _)| p == path) {
            blocks.insert((&dupe.base, *block_line, &dupe.keeper.0));
        }
    }
    let repairs: Vec<stamp::TokenRepair<'_>> = blocks
        .iter()
        .map(|(base_token, block_line, keeper)| stamp::TokenRepair {
            base_token,
            block_line: *block_line,
            keeper,
        })
        .collect();
    if repairs.is_empty() {
        return;
    }
    if let Err(e) = stamp::replace_card_tokens(path, &repairs, &map.digests) {
        eprintln!(
            "warning: cannot resolve the duplicate identities in {}: {e}",
            path.display()
        );
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
        } else if let Some(outcome) = store.realign_card_holes(token, &card.block_holes) {
            augment.remap_holes(token, &outcome);
            cascaded = true;
        }
    }
    cascaded
}

pub fn stamp_and_load_deck(path: &Path) -> Result<Deck> {
    stamp_for_session(path)?;
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
        stamp_for_session(path)?;
        resolve_duplicates_at_open(path);
    }

    if let Some(mut deck) = single_trace_to_walk(&paths) {
        deck.cards = exclude_unstamped(deck.cards, &deck.subject);
        let trace = Trace::from_deck(&deck)?;
        return Ok(Selected::Walk(WalkBuild {
            walk: Walk::new(trace),
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
            "`{}` is a folder; serve it (`alix {}`) and pick a deck inside it",
            deck.display(),
            deck.display()
        );
    }
    let expanded = expand_workspaces(&deck_paths)?;
    let (cards, deck_label, mut decks, settings) = load_decks(&expanded.decks, &expanded.defaults)?;
    let mut cards = exclude_unstamped(cards, &deck_label);
    for info in decks.values_mut() {
        let workspace_override = workspace::root_for_deck(&info.path)
            .and_then(|root| workspace::manifest_source_access(&root));
        info.source_access = workspace_override.unwrap_or(cfg.ask.source_access);
    }
    let label = deck_label;

    let deck_tokens: std::collections::HashSet<String> = decks
        .values()
        .filter_map(|d| d.deck_token.clone())
        .collect();
    let mut augment = AugmentCache::open_for_workspace(&workspace::content_root(deck))
        .context("cannot open deck augmentation")?;
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
            .and_then(|id| augment.note(&id, card.content_fingerprint))
            .map(str::to_string)
        {
            card.append_note(&[note]);
        }
    }

    // The gate is a property of the DECK, so its graph is captured while the
    // vector is still complete: both filters below drop cards a parent block's
    // graduation is folded from.
    let lock_graph = crate::session::LockGraph::build(&cards);

    let topology = resolve_topology(topology_sel, &augment, &deck_tokens)?;
    let topology_name = topology.map(|t| t.name.clone());
    let topology_order = topology.map(|t| TopologyOrder::from_walk(&t.walk));

    if let Some(region_name) = region_sel {
        let Some(topology) = topology else {
            bail!("a region needs a review order to sit in; none is selected for this deck");
        };
        let Some(region_ids) = topology.region_cards(region_name) else {
            bail!(
                "no region named `{region_name}` in the review order `{}`",
                topology.name
            );
        };
        let ids: std::collections::HashSet<String> = region_ids.iter().cloned().collect();
        cards.retain(|c| c.id().is_some_and(|id| ids.contains(&id)));
    }

    let review = cfg.review.for_workspace(&workspace::content_root(deck));

    let subject: Arc<str> = Arc::from(label.as_str());
    let deck_id: Arc<str> = decks
        .values()
        .next()
        .and_then(|d| d.deck_token.as_deref())
        .map(Arc::from)
        .unwrap_or_else(|| Arc::from(label.as_str()));
    // Quirk: a `--region` focus always excludes personal cards (they belong to
    // no topology).
    if region_sel.is_none() {
        let personal = crate::personal::read(deck, &label);
        let deck_blocks: Vec<crate::sidecar::DeckCard> = cards
            .iter()
            .map(|card| crate::sidecar::DeckCard {
                id: card.id().unwrap_or_default(),
                notes: Vec::new(),
            })
            .collect();
        let (roster, _orphans) = crate::sidecar::merge(&deck_blocks, &personal.blocks());

        let mut personal_cards = personal.cards;
        bind_personal(&mut personal_cards, &subject, &deck_id);
        for mut card in personal_cards {
            augment.apply_format(&mut card);
            if let Some(note) = card
                .id()
                .and_then(|id| augment.note(&id, card.content_fingerprint))
                .map(str::to_string)
            {
                card.append_note(&[note]);
            }
            cards.push(card);
        }
        for (card, seat) in cards.iter_mut().zip(&roster) {
            card.append_note(&seat.notes);
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
        .or_else(|| store.last_depth(deck_id.as_ref()))
        .unwrap_or_else(|| default_depth(&cards, &augment));
    let options = SessionOptions {
        max_session: opts.session.unwrap_or(cfg.pacing.max_session),
        new_cards_percent: cfg.pacing.new_cards_percent,
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
    // depths. The excluded cards ride along on the session so the done
    // summary can report what waits beyond this depth.
    let mut depth_excluded = Vec::new();
    let cards = if depth == Depth::Recognize {
        // The predicate needs the whole deck (a table card's pool is its
        // sibling rows), so decide before partition consumes the vec.
        let recognizable: Vec<bool> = cards
            .iter()
            .map(|c| crate::depth::card_recognizable(c, &augment, &cards))
            .collect();
        let mut kept = Vec::new();
        for (card, keep) in cards.into_iter().zip(recognizable) {
            if keep {
                kept.push(card);
            } else {
                depth_excluded.push(card);
            }
        }
        kept
    } else {
        cards
    };
    let mut session = Session::from_subset(
        cards,
        store,
        Box::new(Fsrs::tuned(
            review.retention,
            review.recognize_retention,
            review.introduction_cooldown_ms,
            tuning,
        )),
        options,
        now,
        lock_graph,
    );
    session.set_depth_excluded(depth_excluded);

    // Quirk: this write always fires even when the built session has nothing
    // due, so a restart still reopens at the last-chosen depth.
    let resolved_depth = session.depth();
    store.set_last_depth(deck_id.as_ref(), resolved_depth);
    if let Err(e) = store.save() {
        eprintln!("warning: could not save progress: {e}");
    }

    let links = decks
        .values()
        .map(|info| {
            (
                info.deck_token.clone().unwrap_or_default(),
                info.links.clone(),
            )
        })
        .collect();
    let source_layers = decks
        .values()
        .map(|info| {
            (
                info.deck_token.clone().unwrap_or_default(),
                info.source_layers.clone(),
            )
        })
        .collect();
    let base_roots = decks
        .values()
        .filter(|info| info.source_access)
        .filter_map(|info| {
            info.base_root
                .clone()
                .map(|root| (info.deck_token.clone().unwrap_or_default(), root))
        })
        .collect();
    let source_bases = decks
        .values()
        .map(|info| {
            (
                info.deck_token.clone().unwrap_or_default(),
                info.source_base.clone(),
            )
        })
        .collect();

    let mut load_warnings: Vec<String> = decks
        .values()
        .flat_map(|info| info.load_warnings.iter().cloned())
        .collect();
    load_warnings.sort();
    Ok(Selected::Review(SessionBuild {
        session,
        label,
        decks: deck_id_paths(decks),
        load_warnings,
        links,
        source_layers,
        base_roots,
        source_bases,
        topology_name,
        region_name: region_sel.map(str::to_string),
        augment,
    }))
}

pub fn browse(paths: Vec<PathBuf>, _instance: Option<&Path>) -> Result<CardsBuild> {
    let [deck] = paths.as_slice() else {
        bail!("browse one deck at a time (merging decks was removed)");
    };
    if !selectable(deck) {
        bail!(
            "`{}` is a workspace; browse a deck inside it, or open it with `alix workspace`",
            deck.display()
        );
    }
    let expanded = expand_workspaces(&paths)?;
    let (mut cards, deck_label, decks, _) = load_decks(&expanded.decks, &expanded.defaults)?;
    let label = deck_label;

    let augment = AugmentCache::open_for_workspace(&workspace::content_root(deck))
        .context("cannot open deck augmentation")?;
    for card in &mut cards {
        augment.apply_format(card);
        if let Some(note) = card
            .id()
            .and_then(|id| augment.note(&id, card.content_fingerprint))
            .map(str::to_string)
        {
            card.append_note(&[note]);
        }
    }

    Ok(CardsBuild {
        cards,
        label,
        decks: deck_id_paths(decks),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{answer::Mode, scheduler::DEFAULT_INTRODUCTION_COOLDOWN_MS};

    #[test]
    fn plain_and_cloze_copies_do_not_create_duplicate_progress_record_owners() {
        let dir = tempfile::tempdir().unwrap();
        let plain_path = dir.path().join("deck.md");
        let cloze_path = dir.path().join("deck-copy.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        write_initialized(
            &plain_path,
            &format!("## original\nanswer\n<!-- id: {shared} -->\n"),
        );
        write_initialized(
            &cloze_path,
            &format!("## copied and changed\n\\blank{{answer}}\n<!-- id: {shared} -->\n"),
        );

        resolve_duplicates_at_open(&cloze_path);

        let plain = Deck::load(&plain_path).unwrap();
        let cloze = Deck::load(&cloze_path).unwrap();
        let progress = dir.path().join("progress");
        for deck in [&plain, &cloze] {
            let deck_id = deck.deck_token.as_deref().unwrap();
            let mut store = Store::open_deck(
                progress.join(format!("{deck_id}.json")),
                deck_id,
                deck.subject.clone(),
            )
            .unwrap();
            store.ensure_records(&deck.cards[0]);
            store.save().unwrap();
        }

        let reopened = Store::open_for_decks(&progress, &[plain, cloze]);
        assert!(
            reopened.is_ok(),
            "review-open must separate the copied authored base before both deck documents claim its hole-remap record: {:?}",
            reopened.err()
        );
    }

    #[test]
    fn every_composed_duplicate_of_one_base_selects_one_authored_block_as_keeper() {
        let dir = tempfile::tempdir().unwrap();
        let one_hole = dir.path().join("deck.md");
        let two_hole_copy = dir.path().join("deck-copy.md");
        let two_hole_peer = dir.path().join("z.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        write_initialized(
            &one_hole,
            &format!("## one\n\\blank{{alpha}}\n<!-- id: {shared} -->\n"),
        );
        let two_holes =
            format!("## two\n\\blank{{alpha}} and \\blank{{beta}}\n<!-- id: {shared} -->\n");
        write_initialized(&two_hole_copy, &two_holes);
        write_initialized(&two_hole_peer, &two_holes);

        let map = crate::dedup::scan_dir(dir.path());
        let keepers: std::collections::HashSet<_> = map
            .card_dupes
            .iter()
            .filter(|dupe| dupe.base == shared)
            .map(|dupe| dupe.keeper.0.clone())
            .collect();

        assert_eq!(
            1,
            keepers.len(),
            "one id comment cannot preserve different keeper files for its derived review units: {:#?}",
            map.card_dupes
        );
    }

    #[test]
    fn duplicate_resolution_rewrites_the_parsed_id_not_matching_fenced_content() {
        let dir = tempfile::tempdir().unwrap();
        let keeper = dir.path().join("deck.md");
        let loser = dir.path().join("deck-copy.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        write_initialized(
            &keeper,
            &format!("## keeper\nanswer\n<!-- id: {shared} -->\n"),
        );
        write_initialized(
            &loser,
            &format!(
                "## copied syntax card\n```md\n<!-- id: {shared} -->\n```\nanswer\n<!-- id: {shared} -->\n"
            ),
        );
        assert_eq!(
            1,
            crate::dedup::scan_dir(dir.path()).card_dupes.len(),
            "the parser found the one active duplicate and ignored the fenced example"
        );

        resolve_duplicates_at_open(&loser);

        let after = std::fs::read_to_string(&loser).unwrap();
        let live = Deck::load(&loser).unwrap().cards[0].token.clone();
        assert!(
            after.contains(&format!("```md\n<!-- id: {shared} -->\n```"))
                && live.as_deref() != Some(shared),
            "the authored code example must stay byte-identical and the parsed duplicate must be reminted; live token {live:?}, file:\n{after}"
        );
    }

    #[test]
    fn review_open_duplicate_repair_normalizes_the_written_loser() {
        let dir = tempfile::tempdir().unwrap();
        let keeper = dir.path().join("a-keeper.md");
        let loser = dir.path().join("b-loser.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        write_initialized(
            &keeper,
            &format!("## keeper\nanswer\n<!-- id: {shared} -->\n"),
        );
        std::fs::write(
            &loser,
            format!(
                "\u{feff}---\r\nformat-version: 1\r\nid: \"deck-loser\"\r\n---\r\n## copied \t\r\nanswer \r\n<!-- id: {shared} -->\r\n"
            ),
        )
        .unwrap();

        resolve_duplicates_at_open(&loser);

        let written = std::fs::read_to_string(&loser).unwrap();
        assert!(
            !written.contains(shared),
            "precondition: review-open must actually repair the duplicate"
        );
        assert_eq!(
            crate::parser::normalize(&written),
            written,
            "duplicate repair is an active deck-write boundary"
        );
    }

    fn write_initialized(path: &Path, text: &str) {
        let id: String = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("deck")
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        std::fs::write(
            path,
            format!("---\nformat-version: 1\nid: \"deck-{id}\"\n---\n{text}"),
        )
        .unwrap();
    }

    #[test]
    fn review_open_never_reasons_a_token_the_parser_never_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("a.md");
        let good = dir.path().join("b.md");
        // A no-break space on the closing fence: the parser never closes the
        // frontmatter, so it refuses the file and it claims no token at all.
        std::fs::write(
            &broken,
            "---\nformat-version: 1\nid: \"deck-dtok1\"\n---\u{a0}\n## q1\nanswer\n<!-- id: card-shared1 -->\n",
        )
        .unwrap();
        std::fs::write(
            &good,
            "---\nformat-version: 1\nid: \"deck-dtok2\"\n---\n## q2\nanswer\n<!-- id: card-shared1 -->\n",
        )
        .unwrap();

        assert!(
            crate::parser::parse("a.md", &std::fs::read_to_string(&broken).unwrap()).is_err(),
            "the premise: the parser refuses the neighbour"
        );

        resolve_duplicates_at_open(&good);

        assert!(
            std::fs::read_to_string(&good)
                .unwrap()
                .contains("card-shared1"),
            "opening a valid deck re-minted its card token, severing its review \
             history, because an unparseable neighbour claimed the same token"
        );
    }

    #[test]
    fn a_fast_deck_overclaim_never_suppresses_a_real_card_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("a-broken.md");
        let keeper = dir.path().join("b-keeper.md");
        let loser = dir.path().join("c-loser.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";

        std::fs::write(
            &broken,
            "---\nformat-version: 1\nid: \"deck-shared\"\n---\u{a0}\n",
        )
        .unwrap();
        std::fs::write(
            &keeper,
            format!(
                "---\nformat-version: 1\nid: \"deck-shared\"\n---\n## keeper\na\n<!-- id: {shared} -->\n"
            ),
        )
        .unwrap();
        std::fs::write(
            &loser,
            format!(
                "---\nformat-version: 1\nid: \"deck-other\"\n---\n## loser\nb\n<!-- id: {shared} -->\n"
            ),
        )
        .unwrap();

        assert!(Deck::load(&broken).is_err(), "the neighbour is unparseable");
        assert!(
            crate::dedup::scan_dir_fast(dir.path())
                .card_dupes
                .is_empty(),
            "the invalid deck's deck-token claim excludes the real keeper from the fast card map"
        );
        assert_eq!(
            1,
            crate::dedup::scan_dir(dir.path()).card_dupes.len(),
            "the full parser skips the invalid over-claim and sees the real duplicate"
        );

        resolve_duplicates_at_open(&loser);

        assert!(
            !std::fs::read_to_string(&loser).unwrap().contains(shared),
            "the fast gate returned early, so the real duplicate still fuses two cards' progress"
        );
    }

    #[test]
    fn a_same_file_duplicate_remints_the_loser_not_the_keeper() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        std::fs::write(
            &path,
            format!(
                "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## original\na\n<!-- id: {shared} -->\n## pasted\nb\n<!-- id: {shared} -->\n"
            ),
        )
        .unwrap();

        resolve_duplicates_at_open(&path);

        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            Some(shared),
            deck.cards[0].token.as_deref(),
            "the original card is the keeper and must retain its review history"
        );
        assert_ne!(
            Some(shared),
            deck.cards[1].token.as_deref(),
            "the pasted loser must receive the fresh token"
        );
    }

    #[test]
    fn review_open_remints_every_loser_in_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        std::fs::write(
            &path,
            format!(
                "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## original\na\n<!-- id: {shared} -->\n## pasted once\nb\n<!-- id: {shared} -->\n## pasted twice\nc\n<!-- id: {shared} -->\n"
            ),
        )
        .unwrap();
        assert_eq!(
            2,
            crate::dedup::scan_dir(dir.path()).card_dupes[0]
                .losers
                .len(),
            "the premise: one keeper has two independently addressable losers"
        );

        resolve_duplicates_at_open(&path);

        let remaining = crate::dedup::scan_dir(dir.path()).card_dupes;
        assert!(
            remaining.is_empty(),
            "one review-open must resolve every pasted loser before the session can grade them: {remaining:#?}"
        );
    }

    #[test]
    fn a_card_carrying_two_id_directives_is_refused_rather_than_half_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let keeper = dir.path().join("a-keeper.md");
        let loser = dir.path().join("b-loser.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        std::fs::write(
            &keeper,
            format!(
                "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## keeper\na\n<!-- id: {shared} -->\n"
            ),
        )
        .unwrap();
        std::fs::write(
            &loser,
            format!(
                "---\nformat-version: 1\nid: \"deck-6v3c7x4k1m8q3z5t0b2n4d8f9w\"\n---\n## loser\nb\n<!-- id: {shared} -->\n<!-- id: {shared} -->\n"
            ),
        )
        .unwrap();
        // Two id directives used to parse, with the LAST one winning, while a
        // rewrite targets the first: the repair looked applied and the cards
        // still shared an identity.
        let error = Deck::load(&loser).unwrap_err();
        assert!(
            format!("{error}").contains("already carries an `id:`"),
            "a second id directive is refused outright: {error}"
        );

        resolve_duplicates_at_open(&loser);

        assert!(
            std::fs::read_to_string(&loser)
                .unwrap()
                .matches(shared)
                .count()
                == 2,
            "an unparseable loser is left exactly as authored"
        );
    }

    #[test]
    fn a_stale_full_scan_never_remints_a_card_after_the_duplicate_disappears() {
        let dir = tempfile::tempdir().unwrap();
        let keeper = dir.path().join("a-keeper.md");
        let loser = dir.path().join("b-loser.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        let other = "card-6v3c7x4k1m8q3z5t0b2n4d8f9w";
        let keeper_text = format!(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## keeper\na\n<!-- id: {shared} -->\n"
        );
        let loser_text = format!(
            "---\nformat-version: 1\nid: \"deck-6v3c7x4k1m8q3z5t0b2n4d8f9w\"\n---\n## loser\nb\n<!-- id: {shared} -->\n"
        );
        std::fs::write(&keeper, &keeper_text).unwrap();
        std::fs::write(&loser, &loser_text).unwrap();

        let stale = crate::dedup::scan_dir(dir.path());
        let loser_line = stale.card_dupes[0].losers[0].1;

        // An editor or sync save lands after the full directory scan but
        // before its decision is applied. The old token is now unique and
        // must retain its history.
        std::fs::write(&keeper, keeper_text.replace(shared, other)).unwrap();
        assert!(crate::dedup::scan_dir(dir.path()).card_dupes.is_empty());

        let result = stamp::replace_card_tokens(
            &loser,
            &[stamp::TokenRepair {
                base_token: shared,
                block_line: loser_line,
                keeper: &keeper,
            }],
            &stale.digests,
        );

        assert!(
            result.is_err(),
            "a scan result must not authorize changing an identity after its duplicate disappeared"
        );
        assert_eq!(loser_text, std::fs::read_to_string(&loser).unwrap());
    }

    #[test]
    fn review_open_resolves_duplicates_whose_ids_are_composed_from_one_base_token() {
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        let cases = [
            (
                "cloze siblings",
                format!(
                    "## fill both\n\\blank{{alpha}} and \\blank{{beta}}\n<!-- id: {shared} -->\n"
                ),
            ),
            (
                "table siblings",
                format!(
                    "| front | back |\n|---|---|\n| q | a | <!-- r:4k2x9w -->\n| r | b | <!-- r:6v3c7x -->\n<!-- cards -->\n<!-- id: {shared} -->\n"
                ),
            ),
        ];
        let mut unresolved = Vec::new();
        for (shape, body) in cases {
            let dir = tempfile::tempdir().unwrap();
            let keeper = dir.path().join("a-keeper.md");
            let loser = dir.path().join("b-loser.md");
            write_initialized(&keeper, &body);
            write_initialized(&loser, &body);
            let before = crate::dedup::scan_dir(dir.path()).card_dupes;
            assert_eq!(
                1,
                before.len(),
                "one duplicated base token is ONE claim, however many review units its block expands to: {before:#?}"
            );
            assert!(
                Deck::load(&loser).unwrap().cards.len() >= 2,
                "the {shape} fixture must expand that one block into sibling review units"
            );

            resolve_duplicates_at_open(&loser);

            let after = crate::dedup::scan_dir(dir.path()).card_dupes;
            if !after.is_empty() {
                unresolved.push((shape, after));
            }
        }
        assert!(
            unresolved.is_empty(),
            "one base-token rewrite must resolve every composed sibling id before review loads: {unresolved:#?}"
        );
    }

    #[test]
    fn same_file_revalidation_refuses_after_the_keeper_claim_disappears() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        let other = "card-6v3c7x4k1m8q3z5t0b2n4d8f9w";
        let scanned_text = format!(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## keeper\na\n<!-- id: {shared} -->\n## loser\nb\n<!-- id: {shared} -->\n"
        );
        std::fs::write(&path, &scanned_text).unwrap();
        let stale = crate::dedup::scan_dir(dir.path());
        let dupe = &stale.card_dupes[0];
        assert_eq!(dupe.keeper.0, path, "the keeper and loser share one file");
        let loser_line = dupe.losers[0].1;

        let resolved_text = scanned_text.replacen(shared, other, 1);
        std::fs::write(&path, &resolved_text).unwrap();
        assert!(
            crate::dedup::scan_dir(dir.path()).card_dupes.is_empty(),
            "the editor save removed the duplicate before the stale write"
        );

        let result = stamp::replace_card_tokens(
            &path,
            &[stamp::TokenRepair {
                base_token: shared,
                block_line: loser_line,
                keeper: &path,
            }],
            &stale.digests,
        );

        assert_eq!(
            resolved_text,
            std::fs::read_to_string(&path).unwrap(),
            "the now-unique card must retain the identity that owns its progress, got {result:?}"
        );
    }

    #[test]
    fn same_file_revalidation_refuses_when_card_blocks_move_after_the_scan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck.md");
        let shared = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        let head = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n";
        let original = format!("## original\na\n<!-- id: {shared} -->\n");
        let pasted = format!("## pasted\nb\n<!-- id: {shared} -->\n");
        std::fs::write(&path, format!("{head}{original}{pasted}")).unwrap();
        let stale = crate::dedup::scan_dir(dir.path());
        let dupe = &stale.card_dupes[0];
        let loser_line = dupe.losers[0].1;
        assert_eq!(
            "original",
            Deck::load(&path).unwrap().cards[0].front,
            "the original was the scanned keeper"
        );

        let moved_text = format!("{head}{pasted}{original}");
        std::fs::write(&path, &moved_text).unwrap();
        assert_eq!(
            1,
            crate::dedup::scan_dir(dir.path()).card_dupes.len(),
            "the duplicate still exists, but its card addresses moved"
        );

        let result = stamp::replace_card_tokens(
            &path,
            &[stamp::TokenRepair {
                base_token: shared,
                block_line: loser_line,
                keeper: &path,
            }],
            &stale.digests,
        );

        let after = Deck::load(&path).unwrap();
        let original_after = after
            .cards
            .iter()
            .find(|card| card.front == "original")
            .unwrap();
        assert_eq!(
            Some(shared),
            original_after.token.as_deref(),
            "the stale loser line re-minted the moved original and transferred its progress identity to the pasted card, got {result:?}"
        );
        assert_eq!(
            moved_text,
            std::fs::read_to_string(&path).unwrap(),
            "the original card must not lose its progress identity after being moved"
        );
    }

    /// Codex's finding, at the public boundary: a deck reorganized so that
    /// a former card front became a `# ` section leaves its old id below
    /// that section, belonging to no card. That malformed file must not
    /// drag a HEALTHY copy elsewhere into a remint: the healthy card would
    /// get a fresh token and lose its progress row.
    #[test]
    fn a_section_orphan_id_never_remints_a_valid_card_in_another_deck() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("a-broken.md");
        // The reorganization shape: a real card, then a former front turned
        // into a section, with that card's old id stranded below it.
        std::fs::write(
            &broken,
            "## still a card <!-- id: card-7ty2cy2c42qp8vqq5xrfwcqega -->\nanswer\n# was a card\nprose\n<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n",
        )
        .unwrap();
        let valid = dir.path().join("z-valid.md");
        let valid_text = "## a real card <!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\nanswer\n";
        std::fs::write(&valid, valid_text).unwrap();

        let dupes = crate::dedup::scan_dir_fast(dir.path()).card_dupes;
        assert!(
            dupes.is_empty(),
            "precondition: the orphan id below a section claims no card, so \
             there is no duplicate at all: {dupes:?}"
        );

        resolve_duplicates_at_open(&valid);

        assert_eq!(
            valid_text,
            std::fs::read_to_string(&valid).unwrap(),
            "the healthy deck must be byte-identical after opening"
        );
    }

    #[test]
    fn resolve_uses_cli_then_consensus_then_default() {
        assert_eq!(
            3,
            resolve("depth", Some(3), [Some(1), Some(2)].into_iter(), 0)
        );
        assert_eq!(
            2,
            resolve("depth", None, [Some(2), None, Some(2)].into_iter(), 0)
        );
        assert_eq!(0, resolve("depth", None, [None, None].into_iter(), 0));
        assert_eq!(0, resolve("depth", None, [Some(1), Some(2)].into_iter(), 0));
    }

    #[test]
    fn resolve_empty_child() {
        if std::env::var_os("ALIX_RESOLVE_EMPTY_CHILD").is_none() {
            return;
        }
        assert_eq!(0, resolve("depth", None, std::iter::empty(), 0));
    }

    #[test]
    fn an_empty_declaration_set_uses_the_default_without_a_disagreement_warning() {
        #[cfg(all(unix, feature = "full"))]
        let _lock = crate::testutil::exec_lock();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "assemble::tests::resolve_empty_child",
                "--nocapture",
            ])
            .env("ALIX_RESOLVE_EMPTY_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !stderr.contains("decks disagree"),
            "no declarations cannot disagree: {stderr}"
        );
    }

    #[test]
    fn an_explicit_topology_name_selects_only_the_equal_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        for name in ["first", "wanted", "last"] {
            cache.add_topology(Topology {
                name: name.to_string(),
                principle: name.to_string(),
                edges: Vec::new(),
                walk: Vec::new(),
                regions: Vec::new(),
                deck_token: "deck-owner".to_string(),
            });
        }
        let deck_tokens = std::collections::HashSet::from(["deck-owner".to_string()]);

        let selected = resolve_topology(Some("wanted"), &cache, &deck_tokens)
            .unwrap()
            .expect("the named topology exists");

        assert_eq!("wanted", selected.name);
    }

    #[test]
    fn exclude_unstamped_warning_child() {
        if std::env::var_os("ALIX_EXCLUDE_UNSTAMPED_WARNING_CHILD").is_none() {
            return;
        }
        let mut stamped = Card::plain(
            Arc::from("deck"),
            "kept".to_string(),
            vec!["answer".to_string()],
            Vec::new(),
            1,
        );
        stamped.token = Some(Arc::from("card-kept"));
        let unstamped = |front: &str| {
            Card::plain(
                Arc::from("deck"),
                front.to_string(),
                vec!["answer".to_string()],
                Vec::new(),
                2,
            )
        };

        assert_eq!(
            1,
            exclude_unstamped(
                vec![stamped.clone(), unstamped("first"), unstamped("second")],
                "fixture",
            )
            .len()
        );
        assert_eq!(
            1,
            exclude_unstamped(vec![stamped], "all-stamped fixture").len()
        );
    }

    #[test]
    fn exclude_unstamped_reports_the_exact_positive_count_and_stays_quiet_at_zero() {
        #[cfg(all(unix, feature = "full"))]
        let _lock = crate::testutil::exec_lock();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "assemble::tests::exclude_unstamped_warning_child",
                "--nocapture",
            ])
            .env("ALIX_EXCLUDE_UNSTAMPED_WARNING_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(
            stderr.contains("warning: 2 unstamped card(s) in fixture are excluded"),
            "{stderr}"
        );
        assert_eq!(1, stderr.matches("unstamped card(s)").count(), "{stderr}");
    }

    #[test]
    fn selectable_is_false_only_for_a_folder_that_contains_decks() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("d.md");
        write_initialized(&file, "## q <!-- id: card-q1 -->\na\n");
        let ws = dir.path().join("box");
        std::fs::create_dir(&ws).unwrap();
        write_initialized(&ws.join("m.md"), "## q <!-- id: card-qm -->\na\n");
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();

        assert!(selectable(&file), "a deck file is selectable");
        assert!(!selectable(&ws), "a folder of decks is not selectable");
        assert!(selectable(&empty), "an empty folder has no decks to reject");
    }

    #[test]
    fn load_decks_keys_by_deck_id_so_same_basename_decks_survive_together() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let one = a.join("geo.md");
        let two = b.join("geo.md");
        std::fs::write(
            &one,
            "---\nformat-version: 1\nid: \"deck-aaaaaaaaaaaaaaaaaaaaaaaaaa\"\n---\n## q\na\n<!-- id: card-qa -->\n",
        )
        .unwrap();
        std::fs::write(
            &two,
            "---\nformat-version: 1\nid: \"deck-bbbbbbbbbbbbbbbbbbbbbbbbbb\"\n---\n## q\nb\n<!-- id: card-qb -->\n",
        )
        .unwrap();

        let (_, _, decks, _) = load_decks(&[one, two], &HashMap::new()).unwrap();

        assert_eq!(
            2,
            decks.len(),
            "two decks sharing the filename `geo.md` but with distinct ids must both survive; a filename key collapses them to one"
        );
    }

    #[test]
    fn augment_open_refuses_an_uninitialized_deck_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        let original = "## q1\na1\n## q2\na2\n";
        std::fs::write(&path, original).unwrap();

        let error = stamp_and_load_cards(std::slice::from_ref(&path)).unwrap_err();

        assert!(
            error.to_string().contains("alix deck init"),
            "actionable error: {error:#}"
        );
        assert_eq!(original, std::fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn store_for_prefers_workspace_then_instance_then_global() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("box");
        std::fs::create_dir_all(ws.join(workspace::DECKS)).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Box\"\n").unwrap();
        let member = ws.join("decks/a.md");
        write_initialized(&member, "## q\na\n<!-- id: card-q1 -->\n");
        let loose = dir.path().join("loose.md");
        write_initialized(&loose, "## q\na\n<!-- id: card-q2 -->\n");
        let instance = dir.path().join("instance-state");

        let p = store_path_for(std::slice::from_ref(&member), None).expect("workspace store");
        assert_eq!(p, ws);
        let s = store_for(std::slice::from_ref(&member), Some(&instance)).unwrap();
        assert_eq!(s.path(), ws.join("progress/deck-a.json").as_path());
        let s = store_for(std::slice::from_ref(&loose), Some(&instance)).unwrap();
        assert_eq!(
            s.path(),
            dir.path()
                .join("instance-state/progress/deck-loose.json")
                .as_path()
        );
        let global = dir.path().join("global-state");
        let g = store_for_with_default(std::slice::from_ref(&loose), None, Some(global.clone()))
            .unwrap();
        assert_eq!(
            g.path(),
            dir.path().join("global-state/progress/deck-loose.json")
        );
    }

    #[test]
    fn store_path_for_picks_workspace_else_global_else_override() {
        let dir = tempfile::tempdir().unwrap();
        let mk_ws = |name: &str| {
            let ws = dir.path().join(name);
            std::fs::create_dir_all(ws.join(workspace::DECKS)).unwrap();
            std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
            write_initialized(&ws.join("decks/a.md"), "## a <!-- id: card-qa -->\n1\n");
            write_initialized(&ws.join("decks/b.md"), "## b <!-- id: card-qb -->\n1\n");
            ws
        };
        let ws = mk_ws("ws");
        let ws2 = mk_ws("ws2");
        let ws_store = ws.clone();
        let loose = dir.path().join("loose.md");
        write_initialized(&loose, "## c <!-- id: card-qc -->\n1\n");

        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("decks/a.md")], None)
        );
        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("decks/a.md"), ws.join("decks/b.md")], None)
        );
        assert_eq!(None, store_path_for(std::slice::from_ref(&loose), None));
        assert_eq!(
            None,
            store_path_for(&[ws.join("decks/a.md"), loose.clone()], None)
        );
        assert_eq!(
            None,
            store_path_for(&[ws.join("decks/a.md"), ws2.join("decks/a.md")], None)
        );
        assert_eq!(None, store_path_for(&[], None));
        let over = dir.path().join("x.json");
        assert_eq!(
            Some(over.clone()),
            store_path_for(&[ws.join("decks/a.md")], Some(&over))
        );
    }

    const TRACE_DECK: &str = "---\nformat-version: 1\nid: \"deck-trace\"\ntrace: how it works\nsource: source.txt\n---\n\
## Predict the first hop\n\
it reads the first line\n\
<!-- at: 1 -->\n\
<!-- id: card-qhop1 -->\n\
## Predict the second hop\n\
it reads line two\n\
<!-- at: 2 -->\n\
<!-- id: card-qhop2 -->\n";

    fn test_config() -> AssembleConfig {
        AssembleConfig {
            review: ReviewConfig::default(),
            ask: AskConfig::default(),
            pacing: Pacing {
                max_session: 10,
                new_cards_percent: 30,
            },
            instance_store: None,
        }
    }

    #[test]
    fn recognize_selection_keeps_excluded_parent_units_in_the_lock_fold() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("gated.md");
        write_initialized(
            &deck_path,
            "## Parent\n\\blank{one} and \\blank{two}\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n\n### Child\nchild answer\n<!-- id: card-3k5m9q2w7x4c1t8z0v6b2n4d8f -->\n",
        );
        let loaded = Deck::load(&deck_path).unwrap();
        let parents: Vec<&Card> = loaded
            .cards
            .iter()
            .filter(|card| card.parent_block.is_none())
            .collect();
        let child = loaded
            .cards
            .iter()
            .find(|card| card.parent_block.is_some())
            .expect("the child parses");
        assert_eq!(2, parents.len(), "the parent expands to two review units");

        let mut store = state::open_store(&deck_path, dir.path()).unwrap();
        let first_parent_id = parents[0].id().unwrap();
        store.get_or_insert(&first_parent_id).recall = Some(crate::store::FsrsState {
            stability: 10.0,
            state: 2,
            ..Default::default()
        });
        assert!(
            crate::session::LockGraph::build(&loaded.cards)
                .evaluate(&store)
                .is_locked(child),
            "the unseen second parent unit keeps the child locked"
        );

        let child_id = child.id().unwrap();
        let mut cache =
            AugmentCache::open_for_decks(dir.path(), std::slice::from_ref(&loaded)).unwrap();
        for card in [parents[0], child] {
            cache.set_distractors(
                &card.id().unwrap(),
                vec!["wrong one".into(), "wrong two".into(), "wrong three".into()],
                card.content_fingerprint,
            );
        }
        cache.save().unwrap();

        let Selected::Review(build) = select(
            vec![deck_path],
            &mut store,
            &test_config(),
            &SelectOptions {
                depth: Some(Depth::Recognize),
                ..Default::default()
            },
        )
        .unwrap() else {
            panic!("a fact deck selects a review session");
        };
        assert!(
            build
                .session
                .cards()
                .iter()
                .any(|card| card.id().as_deref() == Some(child_id.as_str())),
            "the child itself is recognizable and reaches the session"
        );
        assert_eq!(
            1, build.session.initial_size,
            "only the graduated parent unit may be served; the child still waits on its excluded sibling"
        );
    }

    #[test]
    fn region_selection_keeps_the_graduated_parent_in_the_lock_fold() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("gated.md");
        write_initialized(
            &deck_path,
            "## Parent\nparent answer\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n\n### Child\nchild answer\n<!-- id: card-3k5m9q2w7x4c1t8z0v6b2n4d8f -->\n",
        );
        let loaded = Deck::load(&deck_path).unwrap();
        let parent = loaded
            .cards
            .iter()
            .find(|card| card.parent_block.is_none())
            .expect("the parent parses");
        let child = loaded
            .cards
            .iter()
            .find(|card| card.parent_block.is_some())
            .expect("the child parses");
        let parent_id = parent.id().unwrap();
        let child_id = child.id().unwrap();

        let mut store = state::open_store(&deck_path, dir.path()).unwrap();
        store.get_or_insert(&parent_id).recall = Some(crate::store::FsrsState {
            stability: 10.0,
            state: 2,
            ..Default::default()
        });
        assert!(
            !crate::session::LockGraph::build(&loaded.cards)
                .evaluate(&store)
                .is_locked(child),
            "the graduated parent unlocks its child in the complete deck"
        );

        let mut cache = AugmentCache::open_for_deck(&loaded).unwrap();
        cache.add_topology(Topology {
            name: "child-only".to_string(),
            principle: "review the child".to_string(),
            edges: Vec::new(),
            walk: vec![child_id.clone()],
            regions: vec![augment::TopologyRegion {
                name: "child".to_string(),
                cards: vec![child_id.clone()],
            }],
            deck_token: loaded.deck_token.clone().unwrap(),
        });
        cache.save().unwrap();

        let Selected::Review(build) = select(
            vec![deck_path],
            &mut store,
            &test_config(),
            &SelectOptions {
                depth: Some(Depth::Recall),
                topology: Some("child-only".to_string()),
                region: Some("child".to_string()),
                ..Default::default()
            },
        )
        .unwrap() else {
            panic!("a fact deck selects a review session");
        };
        assert_eq!(
            Some(child_id),
            build.session.current_id(),
            "a child-only region remains reviewable after its parent graduated"
        );
    }

    #[test]
    fn an_explicit_session_cap_is_honored_when_the_depth_defaults_to_recognize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        write_initialized(
            &path,
            "## q1\n- [x] a1\n- [ ] w1\n<!-- choices-single -->\n<!-- id: card-q1 -->\n\
             ## q2\n- [x] a2\n- [ ] w2\n<!-- choices-single -->\n<!-- id: card-q2 -->\n\
             ## q3\n- [x] a3\n- [ ] w3\n<!-- choices-single -->\n<!-- id: card-q3 -->\n\
             ## q4\n- [x] a4\n- [ ] w4\n<!-- choices-single -->\n<!-- id: card-q4 -->\n",
        );
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();

        let selected = select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions {
                session: Some(2),
                ..Default::default()
            },
        )
        .unwrap();

        let Selected::Review(build) = selected else {
            panic!("a facts deck selects a review session");
        };
        assert_eq!(Depth::Recognize, build.session.depth());
        assert_eq!(2, build.session.initial_size);
    }

    #[test]
    fn review_open_records_every_deck_card_including_cloze_holes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: \"deck-deck1\"\n---\n## Fill\n\
             the \\blank{alpha} and \\blank{beta}\n<!-- id: card-fillcard -->\n## Plain\nanswer\n<!-- id: card-plaincard -->\n",
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

        let plain = store.records("card-plaincard").expect("plain card records");
        assert!(plain.holes.is_empty());
        let cloze = store.records("card-fillcard").expect("cloze card records");
        assert_eq!(2, cloze.holes.len(), "one fingerprint per hole");
    }

    #[test]
    fn reordering_cloze_holes_in_the_file_moves_schedules_through_review_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: \"deck-deck1\"\n---\n## Fill\n\\blank{alpha} then \\blank{beta}\n<!-- id: card-fillcard -->\n",
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
        store.get_or_insert("card-fillcard-0").total_reviews = 1;
        store.get_or_insert("card-fillcard-1").total_reviews = 2;

        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: \"deck-deck1\"\n---\n## Fill\n\\blank{beta} then \\blank{alpha}\n<!-- id: card-fillcard -->\n",
        )
        .unwrap();
        select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap();

        assert_eq!(
            1,
            store.get("card-fillcard-1").unwrap().total_reviews,
            "alpha"
        );
        assert_eq!(
            2,
            store.get("card-fillcard-0").unwrap().total_reviews,
            "beta"
        );
    }

    #[test]
    fn read_only_scans_never_write_records() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("d.md");
        std::fs::write(
            &deck_path,
            "---\nformat-version: 1\nid: \"deck-deck1\"\n---\n## q\na\n<!-- id: card-qcard -->\n",
        )
        .unwrap();
        let store_path = workspace::root_store_path(dir.path());
        let mut store = state::open_store(&deck_path, &store_path).unwrap();
        store.get_or_insert("card-qcard").introduced_ms = Some(0);
        store.save().unwrap();
        let before = std::fs::read(store.path()).unwrap();

        crate::listing::list_root(dir.path(), &ReviewConfig::default(), 1000);

        let after = std::fs::read(store.path()).unwrap();
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
        write_initialized(&fact, "## q\na\n<!-- id: card-qf -->\n");
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let cfg = AssembleConfig { ..test_config() };
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
            "---\ntrace: how it works\nsource: .\n---\n\n## q\npoint\n<!-- at: 1 -->\n<!-- id: card-qq -->\n",
        )
        .unwrap();
        let fact = dir.path().join("f.md");
        std::fs::write(&fact, "## q\na\n<!-- id: card-qf -->\n").unwrap();

        assert!(single_trace_to_walk(std::slice::from_ref(&trace)).is_some());
        assert!(single_trace_to_walk(std::slice::from_ref(&fact)).is_none());
        assert!(single_trace_to_walk(&[trace, fact]).is_none());
    }

    #[test]
    fn expand_workspaces_member_file_inherits_workspace_settings() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir_all(ws.join(workspace::DECKS)).unwrap();
        write_initialized(&ws.join("decks/a.md"), "## a <!-- id: card-qa -->\nb\n");
        std::fs::write(ws.join("alix.toml"), "[defaults]\ndirection = \"both\"\n").unwrap();

        let exp = expand_workspaces(&[ws.join("decks/a.md")]).unwrap();
        assert_eq!(1, exp.decks.len());
        assert_eq!(
            Some(crate::card::Direction::Both),
            exp.defaults.get("a.md").unwrap().direction
        );
    }

    fn write_personal_card(store: &mut Store, deck: &Path, deck_id: &str) {
        write_named_personal_card(store, deck, deck_id, "card-vq1", "personal front");
    }

    fn write_named_personal_card(
        store: &mut Store,
        deck: &Path,
        deck_id: &str,
        id: &str,
        front: &str,
    ) {
        let block = format!("## {front}\npersonal back\n<!-- id: {id} -->\n");
        crate::personal::append_cards(deck, deck_id, &block).unwrap();
        store.get_or_insert(id).introduced_ms = Some(0);
    }

    #[test]
    fn select_rejects_a_folder_of_decks() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("animals");
        std::fs::create_dir(&ws).unwrap();
        let member = ws.join("m.md");
        write_initialized(&member, "## q\na\n<!-- id: card-qm -->\n");
        // Pin the store explicitly: a bare `None` would fall through to the
        // real global data dir.
        let mut store = store_for(
            std::slice::from_ref(&member),
            Some(&dir.path().join("state")),
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
    fn select_injects_a_decks_personal_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.md");
        write_initialized(&path, "## q1\na1\n<!-- id: card-q1 -->\n");
        // Not a workspace, so pass an explicit `--store`-style override: a
        // bare `None` here would fall through to the real global data dir.
        let mut store =
            store_for(std::slice::from_ref(&path), Some(&dir.path().join("state"))).unwrap();
        write_personal_card(&mut store, &path, "deck-rust");

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
    fn a_sidecar_note_reaches_the_card_it_addresses_deck_or_personal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.md");
        write_initialized(&path, "## q1\na1\n<!-- id: card-q1 -->\n");
        let mut store =
            store_for(std::slice::from_ref(&path), Some(&dir.path().join("state"))).unwrap();
        write_personal_card(&mut store, &path, "deck-rust");
        crate::personal::append_note(
            &path,
            "deck-rust",
            "card-q1",
            &["mine on the authored card".to_string()],
        )
        .unwrap();
        crate::personal::append_note(
            &path,
            "deck-rust",
            "card-vq1",
            &["mine on my own card".to_string()],
        )
        .unwrap();

        let Selected::Review(build) = select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap() else {
            panic!("a fact deck must review");
        };

        let note_for = |id: &str| {
            build
                .session
                .cards()
                .iter()
                .find(|card| card.id().as_deref() == Some(id))
                .unwrap_or_else(|| panic!("{id} is in the session"))
                .only_note()
                .map(str::to_string)
        };
        assert_eq!(
            Some("mine on the authored card".to_string()),
            note_for("card-q1")
        );
        assert_eq!(
            Some("mine on my own card".to_string()),
            note_for("card-vq1")
        );
    }

    #[test]
    fn a_sidecar_note_for_no_card_leaves_every_card_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.md");
        write_initialized(&path, "## q1\na1\n<!-- id: card-q1 -->\n");
        let mut store =
            store_for(std::slice::from_ref(&path), Some(&dir.path().join("state"))).unwrap();
        crate::personal::append_note(
            &path,
            "deck-rust",
            "card-gonegonegonegonegonegone",
            &["addressed to nothing".to_string()],
        )
        .unwrap();

        let Selected::Review(build) = select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap() else {
            panic!("a fact deck must review");
        };
        assert!(
            build
                .session
                .cards()
                .iter()
                .all(|card| card.only_note().is_none()),
            "an orphan note must not land on some other card"
        );
    }

    #[test]
    fn each_injected_personal_card_gets_a_distinct_reserved_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.md");
        write_initialized(&path, "## q1\na1\n<!-- id: card-q1 -->\n");
        let mut store =
            store_for(std::slice::from_ref(&path), Some(&dir.path().join("state"))).unwrap();
        write_named_personal_card(&mut store, &path, "deck-rust", "card-vq1", "personal one");
        write_named_personal_card(&mut store, &path, "deck-rust", "card-vq2", "personal two");

        let Selected::Review(build) = select(
            vec![path],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap() else {
            panic!("a fact deck must review");
        };
        let mut personal_lines: Vec<usize> = build
            .session
            .cards()
            .iter()
            .filter(|card| card.line >= PERSONAL_LINE_BASE)
            .map(|card| card.line)
            .collect();
        personal_lines.sort_unstable();

        assert_eq!(
            [PERSONAL_LINE_BASE, PERSONAL_LINE_BASE + 1],
            *personal_lines
        );
    }

    #[test]
    fn region_focus_excludes_personal_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.md");
        // A frontmatter `id:` gives the deck a stable token, which topology
        // matching is bound to (not card overlap).
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: \"deck-dtok1\"\n---\n## q1\na1\n<!-- id: card-q1 -->\n",
        )
        .unwrap();
        // Not a workspace, so pass an explicit `--store`-style override: a
        // bare `None` here would fall through to the real global data dir.
        let mut store =
            store_for(std::slice::from_ref(&path), Some(&dir.path().join("state"))).unwrap();

        let deck = Deck::load(&path).unwrap();
        let card_id = deck.cards[0].id().unwrap();
        let deck_token = deck.deck_token.clone().unwrap();

        let mut cache = AugmentCache::open_for_deck(&deck).unwrap();
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

        write_personal_card(&mut store, &path, "deck-dtok1");

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
        assert_eq!(
            Some("r1"),
            build.region_name.as_deref(),
            "a region-scoped sitting must retain its region, or continuing widens it to the whole deck"
        );
        assert_eq!(Some("auto"), build.topology_name.as_deref());
    }

    #[test]
    fn a_format_cache_entry_applies_to_a_bound_personal_card() {
        let subject: Arc<str> = Arc::from("rust.md");
        let text = "## List the parts\nA, B, C\n<!-- id: card-vlist -->\n";
        let mut cards = crate::parser::parse_str(&subject, text).unwrap();
        bind_personal(&mut cards, &subject, &subject);
        let mut synth = cards.remove(0);
        let id = synth.id().unwrap();

        let mut cache =
            AugmentCache::open(std::env::temp_dir().join("nonexistent-augment-personal.json"));
        cache.set_format(
            &id,
            augment::Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: Some(Mode::LineByLine),
            },
            synth.format_fingerprint(),
        );
        cache.apply_format(&mut synth);

        assert_eq!("Name the parts", synth.front);
        assert_eq!(["A", "B", "C"], *synth.back_for_display());
        assert_eq!(Some(id), synth.id(), "reshaping must not change identity");
    }

    #[test]
    fn select_applies_a_cached_format_to_an_injected_personal_card() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.md");
        write_initialized(&path, "## q1\na1\n<!-- id: card-q1 -->\n");
        // Not a workspace, so pass an explicit `--store`-style override: a
        // bare `None` here would fall through to the real global data dir.
        let mut store =
            store_for(std::slice::from_ref(&path), Some(&dir.path().join("state"))).unwrap();
        write_personal_card(&mut store, &path, "deck-rust");
        let personal_card = crate::parser::parse_str(
            "rust.md",
            "## personal front\npersonal back\n<!-- id: card-vq1 -->\n",
        )
        .unwrap()
        .remove(0);
        let personal_id = personal_card.id().unwrap();

        let deck = Deck::load(&path).unwrap();
        let mut cache = AugmentCache::open_for_deck(&deck).unwrap();
        cache.set_format(
            &personal_id,
            augment::Format {
                front: Some("Reshaped personal front".to_string()),
                back: vec!["Reshaped personal back".to_string()],
                note: None,
                mode: None,
            },
            personal_card.format_fingerprint(),
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
            .find(|c| c.id().as_deref() == Some(personal_id.as_str()))
            .expect("the injected personal card should be in the session");
        assert_eq!("Reshaped personal front", synth.front);
        assert_eq!(["Reshaped personal back"], *synth.back_for_display());
    }

    #[test]
    fn select_falls_back_to_the_stored_last_depth_before_the_default() {
        use crate::depth::Depth;

        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("d.md");
        write_initialized(&deck, "## q\na\n<!-- id: card-q1 -->\n");
        let mut store = open_store(Some(dir.path().join("p.json"))).unwrap();
        let cfg = test_config();

        let explicit = SelectOptions {
            depth: Some(Depth::Recognize),
            ..Default::default()
        };
        select(vec![deck.clone()], &mut store, &cfg, &explicit).unwrap();
        assert_eq!(Some(Depth::Recognize), store.last_depth("deck-d"));

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
        write_initialized(&deck_path, "## q\na\n<!-- id: card-q1 -->\n");
        let store_path = dir.path().join("state");
        let mut store = state::open_store(&deck_path, &store_path).unwrap();
        let cfg = test_config();

        // Distractor set keyed by the real id, read from the loaded deck
        // (never hand-computed).
        let loaded = Deck::load(&deck_path).unwrap();
        let card_id = loaded.cards[0].id().unwrap();
        let mut cache =
            AugmentCache::open_for_decks(dir.path(), std::slice::from_ref(&loaded)).unwrap();
        cache.set_distractors(
            &card_id,
            vec!["w1".into(), "w2".into(), "w3".into()],
            loaded.cards[0].content_fingerprint,
        );
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
        write_initialized(&deck_path, "## q\na\n<!-- id: card-q1 -->\n");
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
        write_initialized(&dir.path().join("a.md"), "## q <!-- id: card-qa -->\na\n");
        let err = browse(vec![dir.path().to_path_buf()], None).unwrap_err();
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
            "## plain\nanswer\n## pic\n![](a.png)\n\n---\nphoto\n",
        )
        .unwrap();

        let build = browse(vec![path], Some(&dir.path().join("global-state"))).unwrap();
        assert_eq!(2, build.cards.len());
    }

    #[test]
    fn browse_applies_a_cached_format_reshape() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir_all(ws.join("decks")).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Eng\"\n").unwrap();
        let path = ws.join("decks/d.md");
        write_initialized(
            &path,
            "## List the parts\nA, B, C\n<!-- id: card-qlist -->\n",
        );

        let raw = browse(vec![path.clone()], None).unwrap();
        let id = raw.cards[0].id().unwrap();
        assert_eq!(raw.cards[0].back_for_display(), ["A, B, C"]);

        let loaded = Deck::load(&path).unwrap();
        let mut cache = AugmentCache::open_for_decks(&ws, std::slice::from_ref(&loaded)).unwrap();
        cache.set_format(
            &id,
            augment::Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: None,
            },
            raw.cards[0].format_fingerprint(),
        );
        cache.set_note(
            &id,
            "the parts are well known".to_string(),
            raw.cards[0].content_fingerprint,
        );
        cache.save().unwrap();

        let merged = browse(vec![path], None).unwrap();
        assert_eq!(merged.cards[0].front, "Name the parts");
        assert_eq!(merged.cards[0].back_for_display(), ["A", "B", "C"]);
        let note = merged.cards[0].only_note().unwrap_or_default().to_string();
        assert!(note.contains("the parts are well known"), "{note}");
    }

    #[test]
    fn browse_rejects_multiple_decks() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "## q <!-- id: card-qa -->\na\n").unwrap();
        std::fs::write(&b, "## q <!-- id: card-qb -->\nb\n").unwrap();
        let err = browse(vec![a, b], None).err().unwrap();
        assert!(format!("{err}").contains("one deck"), "{err}");
    }

    #[test]
    fn select_returns_the_decks_augment_cache() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("f.md");
        write_initialized(&deck, "## q\na\n<!-- id: card-q1 -->\n");
        let store_path = dir.path().join("state");
        let mut store = state::open_store(&deck, &store_path).unwrap();
        let loaded = crate::deck::Deck::load(&deck).unwrap();
        let id = loaded.cards[0].id().unwrap();
        let fingerprint = loaded.cards[0].content_fingerprint;
        let mut cache =
            AugmentCache::open_for_decks(dir.path(), std::slice::from_ref(&loaded)).unwrap();
        cache.set_note(&id, "seeded".to_string(), fingerprint);
        cache.save().unwrap();

        match select(
            vec![deck],
            &mut store,
            &test_config(),
            &SelectOptions::default(),
        )
        .unwrap()
        {
            Selected::Review(build) => {
                assert_eq!(build.augment.note(&id, fingerprint), Some("seeded"));
            }
            Selected::Walk(_) => panic!("a fact deck must review"),
        }
    }

    #[test]
    fn a_configured_introduction_cooldown_reaches_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("f.md");
        write_initialized(&deck, "## q\na\n<!-- id: card-q1 -->\n");
        let mut store = state::open_store(&deck, dir.path()).unwrap();
        let id = crate::deck::Deck::load(&deck).unwrap().cards[0]
            .id()
            .unwrap();
        let t0 = 1_000_000;
        store.get_or_insert(&id).introduced_ms = Some(t0);

        let mut config = test_config();
        config.review.introduction_cooldown_ms = 1_000;
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
        write_initialized(&deck, "## q\na\n<!-- id: card-q1 -->\n");
        let mut store = state::open_store(&deck, dir.path()).unwrap();
        let id = crate::deck::Deck::load(&deck).unwrap().cards[0]
            .id()
            .unwrap();
        let t0 = 1_000_000;
        store.get_or_insert(&id).introduced_ms = Some(t0);

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
            now_ms: Some(t0 + DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000),
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
        std::fs::create_dir(dir.path().join("decks")).unwrap();
        let deck = dir.path().join("decks/m.md");
        write_initialized(&deck, "## q\na\n<!-- id: card-q1 -->\n");
        let mut store = crate::state::open_store(&deck, dir.path()).unwrap();
        let id = crate::deck::Deck::load(&deck).unwrap().cards[0]
            .id()
            .unwrap();

        let now = crate::time::now_ms();
        store.get_or_insert(&id).recall = Some(crate::store::FsrsState {
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
    fn review_open_stamps_missing_cards_in_an_initialized_deck() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("fresh.md");
        write_initialized(&deck, "## q1\na\n\n## q2\nb\n");
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
            "---\nformat-version: 1\nid: \"deck-dtoka\"\n---\n## q\na\n<!-- id: card-cshared -->\n",
        )
        .unwrap();
        let loser = dir.path().join("notes copy.md");
        std::fs::write(
            &loser,
            "---\nformat-version: 1\nid: \"deck-dtokb\"\n---\n## q\nb\n<!-- id: card-cshared -->\n",
        )
        .unwrap();

        let before = std::fs::read_to_string(&loser).unwrap();
        let map = crate::dedup::scan_dir(dir.path());
        assert_eq!(before, std::fs::read_to_string(&loser).unwrap());
        assert_eq!(1, map.card_dupes.len());
        assert_eq!("card-cshared", map.card_dupes[0].base);
        assert_eq!(keeper.clone(), map.card_dupes[0].keeper.0);

        let mut store = state::open_store(&loser, dir.path()).unwrap();
        store.get_or_insert("card-cshared").introduced_ms = Some(1_000);
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
        assert!(store.get("card-cshared").is_some());
        let served = build.session.cards()[0].id().unwrap();
        assert_ne!(
            "card-cshared", served,
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
        write_initialized(&deck, "## a\n1\n<!-- id: card-q1 -->\n\n## b\n2\n");
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
        assert_eq!(Some("card-q1".to_string()), cards[0].id());
    }
}
