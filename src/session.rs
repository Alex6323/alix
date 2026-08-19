use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
};

use rs_fsrs::Parameters;

use crate::{
    augment::TopologyOrder,
    card::Card,
    depth::Depth,
    scheduler::{Grade, Scheduler},
    source::SourceBase,
    store::Store,
    time,
};

pub struct DeckInfo {
    pub path: PathBuf,
    pub deck_token: Option<String>,
    pub links: Vec<String>,
    pub source_layers: crate::deck::SourceLayers,
    pub base_root: Option<PathBuf>,
    pub source_access: bool,
    pub source_base: SourceBase,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
pub enum Order {
    #[default]
    Scheduled,
    Sequential,
}

impl Order {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "scheduled" => Some(Self::Scheduled),
            "sequential" => Some(Self::Sequential),
            _ => None,
        }
    }
}

/// Cards served in a single sitting, before the two pools are backfilled into
/// each other.
pub const DEFAULT_MAX_SESSION: usize = 10;

/// New-card share of `max_session`, as a percentage; the rest go to due cards.
pub const DEFAULT_NEW_CARDS_PERCENT: u8 = 30;

#[derive(Clone, Debug)]
pub struct SessionOptions {
    /// The number of queue entries a single sitting serves.
    pub max_session: usize,
    /// The new-card share of `max_session` (0-100); the remainder is the due
    /// share. Either pool backfills the other when it runs short.
    pub new_cards_percent: u8,
    pub cram: bool,
    pub order: Order,
    pub topology: Option<TopologyOrder>,
    pub retire_after_days: Option<u32>,
    pub depth: Depth,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            max_session: DEFAULT_MAX_SESSION,
            new_cards_percent: DEFAULT_NEW_CARDS_PERCENT,
            cram: false,
            order: Order::Scheduled,
            topology: None,
            retire_after_days: Some(DEFAULT_RETIRE_AFTER_DAYS),
            depth: Depth::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SessionStats {
    pub reviews: usize,
    pub passed: usize,
    pub failed: usize,
    // Partial passes at any depth; the retired per-depth Recognize tallies
    // collapsed into this one generic counter (ADR 0033).
    pub partial: usize,
    pub introduced: usize,
}

pub struct Session {
    cards: Vec<Card>,
    roster: Vec<usize>,
    current_idx: Option<usize>,
    remaining_now: usize,
    floors: HashMap<String, u64>,
    // Cards completed this sitting (passed, crammed, recognized, or removed),
    // so the done-summary backlog counts don't re-count what was just drilled.
    // Accumulates across chained restarts within one session.
    served: HashSet<String>,
    appearances: Vec<u32>,
    choice_seed: u64,
    scheduler: Box<dyn Scheduler>,
    options: SessionOptions,
    // Cards the depth filter kept out of this sitting entirely (Recognize
    // schedules only pick-capable cards), so the done summary can say what
    // still waits beyond the depth instead of "nothing".
    depth_excluded: Vec<Card>,
    pub initial_size: usize,
    pub stats: SessionStats,
}

struct SelectionDecision {
    index: usize,
    id: String,
    tier: CardTier,
    fresh: bool,
    due: u64,
    floor: u64,
}

/// What an exhausted Recognize sitting hides: cards workable at Recall right
/// now, and cards no pick can be built for at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecognizeGap {
    pub recall: u32,
    pub unaugmented: u32,
}

impl Session {
    pub fn new(
        cards: Vec<Card>,
        store: &mut Store,
        scheduler: Box<dyn Scheduler>,
        options: SessionOptions,
        now_ms: u64,
    ) -> Self {
        let floors = HashMap::new();
        let roster: Vec<usize> =
            build_queue(&cards, store, &*scheduler, &options, &floors, now_ms).into();
        let initial_size = roster.len();
        let appearances = vec![0; cards.len()];

        let mut session = Self {
            cards,
            roster,
            current_idx: None,
            remaining_now: 0,
            floors,
            served: HashSet::new(),
            appearances,
            choice_seed: now_ms,
            scheduler,
            options,
            depth_excluded: Vec::new(),
            initial_size,
            stats: SessionStats::default(),
        };
        session.advance(store, now_ms);
        session
    }

    pub fn set_depth_excluded(&mut self, cards: Vec<Card>) {
        self.depth_excluded = cards;
    }

    /// `None` unless this is a Recognize sitting that actually excluded cards.
    pub fn recognize_gap(&self, store: &Store, now_ms: u64) -> Option<RecognizeGap> {
        if self.options.depth != Depth::Recognize || self.depth_excluded.is_empty() {
            return None;
        }
        let recall = self
            .depth_excluded
            .iter()
            .chain(self.cards.iter())
            .filter_map(|c| c.id())
            .filter(|id| match store.progress(id) {
                None => true,
                Some(state) => self.scheduler.is_due(state, Depth::Recall, now_ms),
            })
            .count();
        Some(RecognizeGap {
            recall: recall as u32,
            unaugmented: self.depth_excluded.len() as u32,
        })
    }

    pub fn restart(&mut self, store: &mut Store, now_ms: u64) -> bool {
        // Floors survive: a chained sitting keeps the intra-sitting spacing, and
        // selection below skips a card still cooling instead of re-facing it.
        // `served` survives too, so the backlog count keeps shrinking across the
        // chain rather than reading the same figure every restart.
        let roster: Vec<usize> = build_queue(
            &self.cards,
            store,
            &*self.scheduler,
            &self.options,
            &self.floors,
            now_ms,
        )
        .into();
        if roster.is_empty() {
            return false;
        }
        self.initial_size = roster.len();
        self.roster = roster;
        self.stats = SessionStats::default();
        self.choice_seed = now_ms;
        self.advance(store, now_ms);
        true
    }

    pub fn has_due_now(&self, store: &Store, now_ms: u64) -> bool {
        !build_queue(
            &self.cards,
            store,
            &*self.scheduler,
            &self.options,
            &self.floors,
            now_ms,
        )
        .is_empty()
    }

    /// Deck-wide, not this sitting: `(met, total)` over every card the deck
    /// holds, where met means the store already carries progress for it.
    pub fn deck_progress(&self, store: &Store) -> (usize, usize) {
        let met = self
            .cards
            .iter()
            .filter(|card| card.id().is_some_and(|id| store.progress(&id).is_some()))
            .count();
        (met, self.cards.len())
    }

    /// The uncapped backlog split `(due_left, new_left)` at `now_ms`: how many
    /// due (or, for Recognize, met-but-unrecognized) and never-met cards remain
    /// beyond what this sitting already drilled. Feeds the done-summary so a
    /// heavy day knows to chain another sitting.
    pub fn remaining_split(&self, store: &Store, now_ms: u64) -> (usize, usize) {
        let depth = self.options.depth;
        let mut due_left = 0;
        let mut new_left = 0;
        for card in &self.cards {
            let Some(id) = card.id() else {
                continue;
            };
            if is_retired(card, store, self.options.retire_after_days) {
                continue;
            }
            if self.served.contains(&id) {
                continue;
            }
            match store.progress(&id) {
                Some(state) => {
                    if self.options.cram || self.scheduler.is_due(state, depth, now_ms) {
                        due_left += 1;
                    }
                }
                None => new_left += 1,
            }
        }
        (due_left, new_left)
    }

    /// The soonest instant an unserved roster card becomes servable, floors
    /// included — `next_due_at` is schedule-wide and floor-blind, so it cannot
    /// explain a done sitting whose cards are merely cooling.
    pub fn next_servable_at(&self, store: &Store, now_ms: u64) -> Option<u64> {
        let cooldown = self.scheduler.introduction_cooldown_ms();
        self.roster
            .iter()
            .filter_map(|&i| {
                let card = &self.cards[i];
                if is_retired(card, store, self.options.retire_after_days) {
                    return None;
                }
                let id = card.id()?;
                let due_at = match store.progress(&id) {
                    None => now_ms,
                    Some(_) if self.options.cram => now_ms,
                    Some(state) => self.scheduler.due_at(state, self.options.depth),
                };
                let floor_open = self
                    .floors
                    .get(id.as_str())
                    .map(|t| t.saturating_add(cooldown))
                    .unwrap_or(now_ms);
                Some(due_at.max(floor_open))
            })
            .min()
    }

    pub fn next_due_at(&self, store: &Store) -> Option<u64> {
        self.cards
            .iter()
            .filter_map(|c| c.id())
            .filter_map(|id| store.progress(&id))
            .map(|state| self.scheduler.due_at(state, self.options.depth))
            .min()
    }

    pub fn depth(&self) -> Depth {
        self.options.depth
    }

    pub fn retire_after_days(&self) -> Option<u32> {
        self.options.retire_after_days
    }

    pub fn current(&self) -> Option<&Card> {
        self.current_idx.map(|i| &self.cards[i])
    }

    pub fn current_mut(&mut self) -> Option<&mut Card> {
        let i = self.current_idx?;
        Some(&mut self.cards[i])
    }

    pub fn current_id(&self) -> Option<String> {
        self.current_idx.and_then(|i| self.cards[i].id())
    }

    pub fn current_fresh(&self, store: &Store) -> bool {
        self.current()
            .and_then(Card::id)
            .is_some_and(|id| store.progress(&id).is_none())
    }

    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    pub fn remaining(&self) -> usize {
        self.remaining_now
    }

    pub fn is_finished(&self) -> bool {
        self.current_idx.is_none()
    }

    pub fn appearance(&self, id: &str) -> u32 {
        self.cards
            .iter()
            .position(|c| c.id().as_deref() == Some(id))
            .map(|i| self.appearances[i])
            .unwrap_or(0)
    }

    pub fn choice_seed(&self) -> u64 {
        self.choice_seed
    }

    pub fn grade(&mut self, store: &mut Store, grade: Grade, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        let Some(id) = self.cards[index].id() else {
            self.advance(store, now_ms);
            return;
        };
        let depth = self.options.depth;

        let state = store.get_or_insert(&id);
        let was_due = self.scheduler.is_due(state, depth, now_ms);
        // A cram pass on a card not yet due only re-anchors an existing
        // schedule. With no schedule at all (a just-introduced card), re-anchor is
        // a no-op that leaves the card pinned to introduced+cooldown, so it wins
        // every rebuild and blocks the rest; apply a genuine first learning
        // event instead.
        if self.options.cram && grade.passed() && !was_due && state.schedule(depth).is_some() {
            self.scheduler.reanchor(state, depth, now_ms);
        } else {
            self.scheduler.apply(state, depth, grade, now_ms, false);
        }

        // A pass propagates to EVERY shallower depth (ADR 0033 clause 4),
        // under the same five guards per source-target pair; a missing target
        // schedule is never created.
        if grade == Grade::Pass && (!self.options.cram || was_due) {
            for target in depth.shallower() {
                if state.schedule(target).is_none() {
                    continue;
                }
                if self.scheduler.is_due(state, target, now_ms) {
                    self.scheduler
                        .apply(state, target, Grade::Pass, now_ms, true);
                } else {
                    self.scheduler.reanchor(state, target, now_ms);
                }
            }
        }

        self.stats.reviews += 1;
        let passed = grade.passed();
        if passed {
            self.stats.passed += 1;
        } else {
            self.stats.failed += 1;
        }
        if grade == Grade::Partial {
            self.stats.partial += 1;
        }
        if passed || self.options.cram {
            self.roster.retain(|&i| i != index);
            self.served.insert(id.clone());
        }
        self.floor(&id, now_ms);
        self.advance(store, now_ms);
    }

    pub fn introduce_current(&mut self, store: &mut Store, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        let Some(id) = self.cards[index].id() else {
            self.advance(store, now_ms);
            return;
        };
        let state = store.get_or_insert(&id);
        if state.introduced_ms.is_none() {
            state.introduced_ms = Some(now_ms);
        }
        self.stats.introduced += 1;
        self.floor(&id, now_ms);
        self.advance(store, now_ms);
    }

    pub fn skip(&mut self, store: &mut Store, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        self.roster.retain(|&i| i != index);
        self.roster.push(index);
        self.advance(store, now_ms);
    }

    pub fn remove_current(&mut self, store: &mut Store, now_ms: u64) -> Vec<Card> {
        let Some(index) = self.current_idx else {
            return Vec::new();
        };
        let (group_deck, group_line) = {
            let (deck, line) = sibling_group(&self.cards[index]);
            (deck.to_string(), line)
        };
        let in_group = |card: &Card| card.deck_id.as_ref() == group_deck && card.line == group_line;
        // A region card removes only itself: its file address is a directive
        // line inside the parent block, never the block. Removing the parent
        // sweeps every card of its block from ALL cards, not the capped
        // roster: a sibling the session cap excluded still loses its source
        // when the block goes.
        let region_only = self.cards[index].region.is_some();
        let doomed: Vec<usize> = self
            .cards
            .iter()
            .enumerate()
            .filter(|(i, card)| *i == index || (!region_only && in_group(card)))
            .map(|(i, _)| i)
            .collect();
        let mut removed: Vec<Card> = doomed.iter().map(|&i| self.cards[i].clone()).collect();
        // Depth-excluded cards of the same removal lose their source too and
        // must join the RETURNED set: the caller clears store progress only
        // for what comes back, and a discarded exclusion would leave an
        // orphan schedule behind a successful removal.
        let (gone, kept): (Vec<Card>, Vec<Card>) = std::mem::take(&mut self.depth_excluded)
            .into_iter()
            .partition(|card| {
                if region_only {
                    let removed_id = removed[0].id();
                    removed_id.is_some() && card.id() == removed_id
                } else {
                    in_group(card)
                }
            });
        self.depth_excluded = kept;
        removed.extend(gone);
        for card in &removed {
            if let Some(id) = card.id() {
                self.served.insert(id);
            }
        }
        // Physically drop them, remapping every index, so restart and
        // has_due_now are right by construction and cannot resurrect a card
        // whose source is gone from the file.
        let doomed_set: std::collections::HashSet<usize> = doomed.iter().copied().collect();
        let mut new_index = vec![usize::MAX; self.cards.len()];
        let mut kept_count = 0;
        for (i, slot) in new_index.iter_mut().enumerate() {
            if !doomed_set.contains(&i) {
                *slot = kept_count;
                kept_count += 1;
            }
        }
        let mut card_i = 0;
        self.cards.retain(|_| {
            let keep = !doomed_set.contains(&card_i);
            card_i += 1;
            keep
        });
        let mut app_i = 0;
        self.appearances.retain(|_| {
            let keep = !doomed_set.contains(&app_i);
            app_i += 1;
            keep
        });
        self.roster = self
            .roster
            .iter()
            .filter(|&&i| !doomed_set.contains(&i))
            .map(|&i| new_index[i])
            .collect();
        self.current_idx = None;
        self.advance(store, now_ms);
        removed
    }

    pub fn poll(&mut self, store: &mut Store, now_ms: u64) -> bool {
        self.select(store, now_ms, true);
        self.current_idx.is_some()
    }

    fn selection_decision(
        &self,
        i: usize,
        store: &Store,
        now_ms: u64,
    ) -> Option<SelectionDecision> {
        let card = &self.cards[i];
        let id = card.id()?;
        let tier = card_tier(store, &id, now_ms, self.options.retire_after_days);
        if tier == CardTier::Retired {
            return None;
        }
        let depth = self.options.depth;
        let progress = store.progress(&id);
        let due = if self.options.cram {
            now_ms
        } else {
            progress
                .map(|state| self.scheduler.due_at(state, depth))
                .unwrap_or(now_ms)
        };
        let due_now = self.options.cram || due <= now_ms;
        if !due_now {
            return None;
        }
        let floor = self
            .floors
            .get(id.as_str())
            .map(|at| at.saturating_add(self.scheduler.introduction_cooldown_ms()))
            .unwrap_or(0);
        if now_ms < floor {
            return None;
        }
        Some(SelectionDecision {
            index: i,
            id,
            tier,
            fresh: progress.is_none(),
            due,
            floor,
        })
    }

    fn servable(&self, i: usize, store: &Store, now_ms: u64) -> bool {
        self.selection_decision(i, store, now_ms).is_some()
    }

    fn floor(&mut self, id: &str, now_ms: u64) {
        let cooldown_ms = self.scheduler.introduction_cooldown_ms();
        self.floors
            .retain(|_, &mut t| now_ms < t.saturating_add(cooldown_ms));
        self.floors.insert(id.to_string(), now_ms);
    }

    fn advance(&mut self, store: &mut Store, now_ms: u64) {
        self.select(store, now_ms, false);
    }

    // The single site where a card becomes current.
    fn select(&mut self, store: &mut Store, now_ms: u64, keep_current: bool) {
        let sticky = if keep_current { self.current_idx } else { None }.and_then(|i| {
            self.roster
                .contains(&i)
                .then(|| self.selection_decision(i, store, now_ms))
                .flatten()
        });
        let decision = sticky.or_else(|| {
            self.roster
                .iter()
                .copied()
                .find_map(|i| self.selection_decision(i, store, now_ms))
        });
        let next = decision.as_ref().map(|decision| decision.index);
        let changed = next != self.current_idx;
        if let Some(i) = next
            && changed
        {
            self.appearances[i] = self.appearances[i].saturating_add(1);
        }
        self.current_idx = next;
        self.remaining_now = self
            .roster
            .iter()
            .copied()
            .filter(|&i| self.servable(i, store, now_ms))
            .count();
        if changed
            && crate::log::enabled(crate::log::Target::Select)
            && let Some(decision) = decision
        {
            crate::log::emit(
                crate::log::Target::Select,
                format_args!(
                    "card={} tier={} fresh={} due={} floor={} roster={}",
                    decision.id,
                    decision.tier.wire_name(),
                    u8::from(decision.fresh),
                    decision.due,
                    decision.floor,
                    self.roster.len(),
                ),
            );
        }
    }
}

// A card selected past a floor is off it once one cooldown has elapsed; a card
// with no floor is free to serve.
fn floor_passed(floors: &HashMap<String, u64>, id: &str, cooldown_ms: u64, now_ms: u64) -> bool {
    match floors.get(id) {
        Some(&transition_ms) => now_ms >= transition_ms.saturating_add(cooldown_ms),
        None => true,
    }
}

// Round-robin the given cards by sibling group so every group's first entry
// precedes any group's second: a capped take then spans distinct facts instead
// of being eaten by one many-hole cloze. Group order follows first appearance.
fn round_robin_siblings(order: Vec<usize>, cards: &[Card]) -> Vec<usize> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut index: HashMap<(&str, usize), usize> = HashMap::new();
    for i in order {
        let slot = *index.entry(sibling_group(&cards[i])).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[slot].push(i);
    }
    let rounds = groups.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = Vec::new();
    for round in 0..rounds {
        for group in &groups {
            if let Some(&i) = group.get(round) {
                out.push(i);
            }
        }
    }
    out
}

// Split `max_session` between the two selection-ordered pools. The new share is
// `ceil(cap * pct / 100)`, floored at one whenever new cards exist; the due
// share is the rest. Whichever pool falls short lets the other fill to the cap
// (symmetric backfill), so a fresh deck fills with new and a no-new deck fills
// with due. Returns `(take_due, take_new)`.
fn split_slots(due_len: usize, new_len: usize, cap: usize, percent: u8) -> (usize, usize) {
    let new_slots = if new_len == 0 {
        0
    } else {
        // Floor at one (min-1 rule) but never above the cap; `.max(1).min(cap)`
        // rather than `clamp`, which would panic for a zero cap.
        (cap * usize::from(percent)).div_ceil(100).max(1).min(cap)
    };
    let due_slots = cap.saturating_sub(new_slots);
    let mut take_due = due_slots.min(due_len);
    let mut take_new = new_slots.min(new_len);
    let mut leftover = cap.saturating_sub(take_due + take_new);
    let add_due = leftover.min(due_len - take_due);
    take_due += add_due;
    leftover -= add_due;
    take_new += leftover.min(new_len - take_new);
    (take_due, take_new)
}

fn build_queue(
    cards: &[Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    options: &SessionOptions,
    floors: &HashMap<String, u64>,
    now_ms: u64,
) -> VecDeque<usize> {
    let depth = options.depth;
    let cooldown = scheduler.introduction_cooldown_ms();

    // Partition into the due pool and the never-met pool, dropping retired cards
    // and any still cooling behind a floor so their slots pass to the next
    // servable cards rather than seeding an unservable sitting.
    let mut due: Vec<usize> = Vec::new();
    let mut new_pool: Vec<usize> = Vec::new();
    for (i, card) in cards.iter().enumerate() {
        let Some(id) = card.id() else {
            continue;
        };
        if is_retired(card, store, options.retire_after_days)
            || !floor_passed(floors, &id, cooldown, now_ms)
        {
            continue;
        }
        match store.progress(&id) {
            Some(state) => {
                let eligible = options.cram || scheduler.is_due(state, depth, now_ms);
                if eligible {
                    due.push(i);
                }
            }
            None => new_pool.push(i),
        }
    }

    // Select oldest-due first, so a deep card that is overdue makes the capped
    // set instead of losing its slot to a shallower card the presentation sort
    // would otherwise float above it. Recognize's met pool has no due_at, so it
    // sweeps in review order when one exists, else deck order.
    if depth == Depth::Recognize {
        if let Some(topo) = &options.topology {
            due.sort_by_key(|&i| {
                cards[i]
                    .id()
                    .as_deref()
                    .and_then(|id| topo.rank_of(id))
                    .unwrap_or(usize::MAX)
            });
        }
    } else {
        due.sort_by_key(|&i| {
            cards[i]
                .id()
                .and_then(|id| store.get(&id))
                .map_or(u64::MAX, |s| scheduler.due_at(s, depth))
        });
    }

    // The new pool is selected in review order when one exists, else deck order
    // round-robined across sibling groups for breadth. The due pool keeps its
    // due-time order: due siblings are legitimately due.
    let new_pool = match &options.topology {
        Some(topo) => {
            let mut v = new_pool;
            v.sort_by_key(|&i| {
                cards[i]
                    .id()
                    .as_deref()
                    .and_then(|id| topo.rank_of(id))
                    .unwrap_or(usize::MAX)
            });
            v
        }
        None => round_robin_siblings(new_pool, cards),
    };

    let (take_due, take_new) = split_slots(
        due.len(),
        new_pool.len(),
        options.max_session,
        options.new_cards_percent,
    );
    let mut chosen: Vec<usize> = due[..take_due].to_vec();
    chosen.extend_from_slice(&new_pool[..take_new]);

    // Presentation: order only the already-capped slice. A review order sorts it
    // by rank (siblings ride along in walk order); otherwise due cards lead new,
    // sequential decks fall back to deck order, and siblings are spaced apart.
    if let Some(topo) = &options.topology {
        chosen.sort_by_key(|&i| {
            cards[i]
                .id()
                .as_deref()
                .and_then(|id| topo.rank_of(id))
                .unwrap_or(usize::MAX)
        });
        return chosen.into();
    }
    if options.order == Order::Sequential {
        chosen.sort_unstable();
    }
    separate_siblings(chosen, cards)
}

fn sibling_group(card: &Card) -> (&str, usize) {
    (card.deck_id.as_ref(), card.line)
}

fn separate_siblings(order: Vec<usize>, cards: &[Card]) -> VecDeque<usize> {
    let mut remaining: VecDeque<usize> = order.into();
    let mut queue = VecDeque::with_capacity(remaining.len());
    let mut last: Option<usize> = None;

    while !remaining.is_empty() {
        let pos = remaining
            .iter()
            .position(|&i| {
                last.is_none_or(|l| sibling_group(&cards[i]) != sibling_group(&cards[l]))
            })
            .unwrap_or(0);
        let index = remaining.remove(pos).unwrap();
        last = Some(index);
        queue.push_back(index);
    }
    queue
}

pub const DEFAULT_RETIRE_AFTER_DAYS: u32 = 365;

pub fn is_retired(card: &Card, store: &Store, retire_after_days: Option<u32>) -> bool {
    card.id()
        .is_some_and(|id| is_retired_id(&id, store, retire_after_days))
}

pub fn is_retired_id(card_id: &str, store: &Store, retire_after_days: Option<u32>) -> bool {
    let Some(cap) = retire_after_days else {
        return false;
    };
    store
        .get(card_id)
        .and_then(|s| s.schedule(Depth::Recall))
        .is_some_and(|f| f.scheduled_days >= cap)
}

pub fn count_due_soon(
    cards: &[Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    depth: Depth,
    now_ms: u64,
    window_ms: u64,
    retire_after_days: Option<u32>,
) -> usize {
    cards
        .iter()
        .filter(|card| !is_retired(card, store, retire_after_days))
        .filter(|card| {
            card.id()
                .and_then(|id| store.get(&id).map(|s| scheduler.due_at(s, depth)))
                .is_some_and(|due| due > now_ms && due <= now_ms + window_ms)
        })
        .count()
}

pub fn has_graduated(card: &Card, store: &Store) -> bool {
    card.id()
        .and_then(|id| store.get(&id))
        .and_then(|s| s.schedule(Depth::Recall))
        .is_some_and(|f| f.graduated())
}

// Matches the scheduler's default request retention: at or above it a
// learned card is right on schedule.
pub const LEARNED_STRONG_MIN: f32 = 0.9;
pub const LEARNED_WEAK_BELOW: f32 = 0.7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardTier {
    Unseen,
    Seen,
    Learning,
    LearnedStrong,
    LearnedFading,
    LearnedWeak,
    Retired,
}

impl CardTier {
    pub fn wire_name(self) -> &'static str {
        match self {
            CardTier::Unseen => "unseen",
            CardTier::Seen => "seen",
            CardTier::Learning => "learning",
            CardTier::LearnedStrong => "learned-strong",
            CardTier::LearnedFading => "learned-fading",
            CardTier::LearnedWeak => "learned-weak",
            CardTier::Retired => "retired",
        }
    }
}

impl serde::Serialize for CardTier {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.wire_name())
    }
}

pub fn card_tiers(
    card_ids: &[String],
    store: &Store,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> Vec<CardTier> {
    card_ids
        .iter()
        .map(|id| card_tier(store, id, now_ms, retire_after_days))
        .collect()
}

fn card_tier(
    store: &Store,
    card_id: &str,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> CardTier {
    let Some(state) = store.get(card_id) else {
        return CardTier::Unseen;
    };
    if is_retired_id(card_id, store, retire_after_days) {
        return CardTier::Retired;
    }
    if let Some(f) = state
        .schedule(Depth::Recall)
        .filter(|f| f.graduated() && f.stability > 0.0)
    {
        let elapsed_days = now_ms.saturating_sub(f.last_review_ms) as f64 / 86_400_000.0;
        let r = Parameters::forgetting_curve(elapsed_days, f.stability).clamp(0.0, 1.0) as f32;
        return if r >= LEARNED_STRONG_MIN {
            CardTier::LearnedStrong
        } else if r < LEARNED_WEAK_BELOW {
            CardTier::LearnedWeak
        } else {
            CardTier::LearnedFading
        };
    }
    if state.total_passes > 0 {
        CardTier::Learning
    } else {
        CardTier::Seen
    }
}

pub fn is_reviewable(
    card: &Card,
    store: &Store,
    scheduler: &dyn Scheduler,
    depth: Depth,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> bool {
    if is_retired(card, store, retire_after_days) {
        return false;
    }
    // An unstamped card is brand new: it gets its id (and its first review) when
    // a session opens. Treat it as due here, like a stamped card with no
    // progress yet, so a fresh hand-authored deck reads as drillable in the
    // picker instead of being greyed out.
    let Some(id) = card.id() else {
        return true;
    };
    match store.progress(&id) {
        Some(state) => scheduler.is_due(state, depth, now_ms),
        None => true,
    }
}

pub fn has_reviewable(
    cards: &[Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    depth: Depth,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> bool {
    cards
        .iter()
        .any(|card| is_reviewable(card, store, scheduler, depth, now_ms, retire_after_days))
}

pub fn count_reviewable(
    cards: &[&Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    depth: Depth,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> usize {
    cards
        .iter()
        .filter(|card| is_reviewable(card, store, scheduler, depth, now_ms, retire_after_days))
        .count()
}

pub fn now_ms() -> u64 {
    time::now_ms()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        scheduler::DEFAULT_INTRODUCTION_COOLDOWN_MS,
        store::{FsrsState, Store},
    };

    fn card(subject: &str, n: usize) -> Card {
        let mut card = Card::plain(
            Arc::from(subject),
            format!("front {n}"),
            vec![format!("back {n}")],
            None,
            n,
        );
        card.token = Some(Arc::from(format!("tok{n}").as_str()));
        card
    }

    fn cards(n: usize) -> Vec<Card> {
        (0..n).map(|i| card("deck.md", i)).collect()
    }

    fn empty_store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        (store, dir)
    }

    fn sched() -> Box<dyn Scheduler> {
        Box::new(crate::scheduler::Fsrs::default())
    }

    fn personal_card(store: &mut Store, deck_id: &str, back: &str, created_ms: u64) -> Card {
        let slug: String = back
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let text = format!("## personal front <!-- id: card-v{slug} -->\n{back}\n");
        let mut card = crate::parser::parse_str(deck_id, &text).unwrap().remove(0);
        card.line = crate::assemble::PERSONAL_LINE_BASE;
        store.get_or_insert(&card.id().unwrap()).introduced_ms = Some(created_ms);
        card
    }

    #[test]
    fn serve_loop_invariants_hold_under_a_fuzzed_grade_sequence() {
        let (mut store, _dir) = empty_store();
        let n = 12;
        let mut session = Session::new(cards(n), &mut store, sched(), SessionOptions::default(), 0);

        let mut rng: u64 = 0x2545_F491_4F6C_DD1D;
        let mut roll = |bound: u64| -> u64 {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) % bound
        };

        let mut passed = vec![false; n];
        let mut now = 0u64;
        let mut drained = false;

        for _ in 0..2000 {
            let before = session.current_idx;
            session.poll(&mut store, now);

            let servable: Vec<usize> = session
                .roster
                .iter()
                .copied()
                .filter(|&i| session.servable(i, &store, now))
                .collect();
            assert_eq!(
                session.remaining(),
                servable.len(),
                "remaining() must equal the servable roster count"
            );
            assert_eq!(
                session.is_finished(),
                servable.is_empty(),
                "finished iff nothing is servable"
            );

            let Some(idx) = session.current_idx else {
                drained = true;
                break;
            };
            assert!(
                session.servable(idx, &store, now),
                "the served card must be servable"
            );
            assert!(
                session.current_idx == servable.first().copied() || session.current_idx == before,
                "the cursor is the first servable roster card, or the one already being studied"
            );
            assert!(!passed[idx], "a passed card must never be served again");

            if session.current_fresh(&store) {
                session.introduce_current(&mut store, now);
            } else {
                let g = match roll(3) {
                    0 => Grade::Fail,
                    1 => Grade::Partial,
                    _ => Grade::Pass,
                };
                session.grade(&mut store, g, now);
                if g.passed() {
                    passed[idx] = true;
                }
            }

            now = now.saturating_add(roll(2 * 3600 * 1000));
        }

        assert!(
            drained,
            "with time advancing and passes occurring, the fuzzed session drains to finished"
        );
    }

    #[test]
    fn a_fresh_deck_fills_the_cap_with_new_cards() {
        let (mut store, _dir) = empty_store();
        // No due cards, so symmetric backfill lets new fill the whole cap.
        let session = Session::new(
            cards(20),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 5,
                ..Default::default()
            },
            1000,
        );
        assert_eq!(5, session.initial_size);
    }

    #[test]
    fn the_percent_mix_splits_a_full_cap_between_deep_pools() {
        let (mut store, _dir) = empty_store();
        // 8 due (met, overdue) + 8 never-met; cap 10, 30% new → 3 new + 7 due.
        let all = cards(16);
        for c in &all[..8] {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = 2 * 604_800_000;
        let session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 10,
                new_cards_percent: 30,
                ..Default::default()
            },
            now,
        );
        assert_eq!(10, session.initial_size);
        let new_ids: Vec<String> = all[8..].iter().filter_map(|c| c.id()).collect();
        let picked_new = session
            .roster
            .iter()
            .filter(|&&i| new_ids.contains(&all[i].id().unwrap()))
            .count();
        assert_eq!(3, picked_new, "30% of 10 is 3 new");
        assert_eq!(7, session.roster.len() - picked_new, "the rest are due");
    }

    #[test]
    fn deck_progress_counts_every_met_card_not_only_this_sitting() {
        let (mut store, _dir) = empty_store();
        let all = cards(16);
        for c in &all[..3] {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = 2 * 604_800_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);
        assert_eq!(
            (3, 16),
            session.deck_progress(&store),
            "three carried in from earlier sittings"
        );

        while session.current().is_some() {
            if session.current_fresh(&store) {
                session.introduce_current(&mut store, now);
            } else {
                session.grade(&mut store, Grade::Pass, now);
            }
        }

        assert_eq!(
            (10, 16),
            session.deck_progress(&store),
            "the seven new cards this sitting planted join them"
        );
    }

    #[test]
    fn remaining_split_reports_the_backlog_after_a_sitting_is_drilled() {
        let (mut store, _dir) = empty_store();
        // 8 due + 8 never-met, cap 10 (7 due + 3 new). Drill the whole sitting,
        // then the split reports what a chained sitting would still find.
        let all = cards(16);
        for c in &all[..8] {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = 2 * 604_800_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);
        let mut served = 0;
        while session.current().is_some() {
            served += 1;
            assert!(
                served <= 16,
                "the sitting must exhaust, never serve past the deck"
            );
            if session.current_fresh(&store) {
                session.introduce_current(&mut store, now);
            } else {
                session.grade(&mut store, Grade::Pass, now);
            }
        }
        let (due_left, new_left) = session.remaining_split(&store, now);
        // 7 due passed (scheduled out, served); 1 due untouched. 3 never-met
        // were introduced (now have progress); 5 never-met remain.
        assert_eq!((1, 5), (due_left, new_left));
    }

    #[test]
    fn a_non_empty_new_pool_always_wins_at_least_one_slot() {
        let (mut store, _dir) = empty_store();
        // 20 due + 5 never-met, cap 10: ceil(10*10/100)=1, so at least 1 new.
        let all = cards(25);
        for c in &all[..20] {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = 2 * 604_800_000;
        let session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 10,
                new_cards_percent: 10,
                ..Default::default()
            },
            now,
        );
        assert_eq!(10, session.initial_size);
        // The last five cards are the never-met pool; at least one entered.
        let new_ids: Vec<String> = all[20..].iter().filter_map(|c| c.id()).collect();
        let roster_new = session
            .roster
            .iter()
            .filter(|&&i| new_ids.contains(&all[i].id().unwrap()))
            .count();
        assert!(roster_new >= 1, "min-1 new even at a low percent");
    }

    #[test]
    fn a_no_new_deck_fills_the_cap_with_due_cards() {
        let (mut store, _dir) = empty_store();
        let all = cards(20);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = 2 * 604_800_000;
        let session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);
        assert_eq!(10, session.initial_size, "no new pool: due fills the cap");
    }

    #[test]
    fn introduce_current_records_the_card_unscheduled_without_a_review() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), 1000);
        assert!(session.current_fresh(&store));

        session.introduce_current(&mut store, 1000);

        let state = store.get(&id).expect("introduced card is recorded");
        assert!(
            state.recall.is_none(),
            "introducing does not schedule under FSRS"
        );
        assert_eq!(
            Some(1000),
            state.introduced_ms,
            "introduce stamps the introduction time"
        );
        assert!(state.history.is_empty());
        assert_eq!(0, state.total_reviews);
        assert_eq!(1, session.stats.introduced);
        assert_eq!(0, session.stats.reviews);
        assert!(session.is_finished());
    }

    #[test]
    fn introduced_cards_are_not_due_until_the_relearn_cooldown() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(1),
            &mut store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        session.introduce_current(&mut store, 1000);

        assert!(!session.has_due_now(&store, 1000));
        assert!(!session.has_due_now(&store, 1000 + DEFAULT_INTRODUCTION_COOLDOWN_MS - 1));
        assert!(session.has_due_now(&store, 1000 + DEFAULT_INTRODUCTION_COOLDOWN_MS));
    }

    #[test]
    fn an_introduced_card_returns_in_session_after_its_cooldown() {
        {
            let at = "plain introduce";
            let (mut store, _dir) = empty_store();
            let mut session = Session::new(
                cards(1),
                &mut store,
                sched(),
                SessionOptions::default(),
                1000,
            );
            let id = session.current().unwrap().id();
            session.introduce_current(&mut store, 1000);
            assert!(session.is_finished(), "{at}: the sitting empties");
            assert!(
                session.poll(&mut store, 1000 + DEFAULT_INTRODUCTION_COOLDOWN_MS),
                "{at}: the card returns after its cooldown"
            );
            assert_eq!(
                session.current().map(|c| c.id()),
                Some(id),
                "{at}: the same card returns"
            );
            assert!(
                !session.current_fresh(&store),
                "{at}: it returns as a graded review, never as another introduction"
            );
        }
    }

    #[test]
    fn a_missed_card_is_not_re_served_before_its_fsrs_due() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);

        let first = session.current().unwrap().id();
        session.grade(&mut store, Grade::Fail, now);
        assert!(session.current().is_some());
        assert_ne!(first, session.current().unwrap().id());
    }

    #[test]
    fn a_missed_card_reappears_once_its_step_elapses() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let first_id = all[0].id();
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);

        assert_eq!(first_id, session.current().unwrap().id());
        session.grade(&mut store, Grade::Fail, now);
        session.grade(&mut store, Grade::Pass, now + 1000);
        session.poll(&mut store, now + DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000);
        assert_eq!(first_id, session.current().unwrap().id());
    }

    #[test]
    fn a_graded_card_never_immediately_follows_itself_while_another_is_servable() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let a_id = all[0].id().unwrap();
        let b_id = all[1].id().unwrap();
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);
        assert_eq!(Some(a_id.clone()), session.current().unwrap().id());

        session.grade(&mut store, Grade::Fail, now);
        assert_eq!(
            Some(b_id.clone()),
            session.current().unwrap().id(),
            "the other due card takes over right after the miss"
        );

        store.get_or_insert(&a_id).recall.as_mut().unwrap().due_ms = now + 10_000;

        session.poll(&mut store, now + 30_000);
        assert_eq!(
            Some(b_id.clone()),
            session.current().unwrap().id(),
            "the floor keeps A from immediately following itself"
        );

        session.grade(
            &mut store,
            Grade::Fail,
            now + DEFAULT_INTRODUCTION_COOLDOWN_MS,
        );
        assert_eq!(
            Some(a_id.clone()),
            session.current().unwrap().id(),
            "A is eligible again once its floor passes, and takes over when B is graded"
        );
    }

    #[test]
    fn the_only_servable_card_may_repeat_once_the_floor_passes() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        store.get_or_insert(&id).introduced_ms = Some(0);
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);

        session.grade(&mut store, Grade::Fail, now);
        assert!(
            session.is_finished(),
            "cooling on its own retry, nothing else to serve"
        );

        store.get_or_insert(&id).recall.as_mut().unwrap().due_ms = now + 1_000;

        session.poll(&mut store, now + DEFAULT_INTRODUCTION_COOLDOWN_MS - 1);
        assert!(session.is_finished(), "the floor delays the repeat");

        session.poll(&mut store, now + DEFAULT_INTRODUCTION_COOLDOWN_MS);
        assert_eq!(Some(id), session.current().and_then(|c| c.id()));
    }

    #[test]
    fn a_poll_past_an_earlier_cards_cooldown_keeps_the_current_card() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let earlier = all[0].id().unwrap();
        let current = all[1].id().unwrap();
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);

        assert_eq!(
            Some(earlier.clone()),
            session.current().and_then(|c| c.id()),
            "roster order: the earlier card is served first"
        );

        session.grade(&mut store, Grade::Fail, now);
        assert_eq!(
            Some(current.clone()),
            session.current().and_then(|c| c.id()),
            "failing the earlier card moves the learner to the later one"
        );

        session.poll(&mut store, now + DEFAULT_INTRODUCTION_COOLDOWN_MS - 1);
        assert_eq!(
            Some(current.clone()),
            session.current().and_then(|c| c.id()),
            "inside the earlier card's cooldown the learner stays put"
        );

        session.poll(&mut store, now + DEFAULT_INTRODUCTION_COOLDOWN_MS);
        assert_eq!(
            Some(current),
            session.current().and_then(|c| c.id()),
            "the earlier card coming off cooldown must not yank the learner off the card \
             they are on; a poll reports state, it does not reshuffle it"
        );
    }

    #[test]
    fn a_poll_drops_a_card_that_has_stopped_being_servable() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let first = all[0].id().unwrap();
        let second = all[1].id().unwrap();
        let mut s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            0,
        );
        assert_eq!(Some(first.clone()), s.current().and_then(|c| c.id()));

        store.get_or_insert(&first).recognize = Some(mature_fsrs(2_000_000));

        s.poll(&mut store, 1_000);
        assert_eq!(
            Some(second),
            s.current().and_then(|c| c.id()),
            "keeping the current card is conditional on it still being servable"
        );
    }

    #[test]
    fn a_passed_earlier_card_does_not_return_after_its_cooldown() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let current = all[1].id().unwrap();
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);

        session.grade(&mut store, Grade::Pass, now);
        assert_eq!(
            Some(current.clone()),
            session.current().and_then(|c| c.id()),
            "passing the earlier card moves the learner to the later one"
        );

        session.poll(&mut store, now + DEFAULT_INTRODUCTION_COOLDOWN_MS);
        assert_eq!(
            Some(current),
            session.current().and_then(|c| c.id()),
            "a passed card leaves the roster, so no cooldown can bring it back"
        );
    }

    #[test]
    fn the_transition_floor_follows_the_configured_cooldown() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        store.get_or_insert(&id).introduced_ms = Some(0);
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(
            all,
            &mut store,
            Box::new(crate::scheduler::Fsrs::new(0.9, 1_000)),
            SessionOptions::default(),
            now,
        );
        session.grade(&mut store, Grade::Fail, now);
        store.get_or_insert(&id).recall.as_mut().unwrap().due_ms = now + 500;
        session.poll(&mut store, now + 1_000);
        assert_eq!(Some(id), session.current().and_then(|c| c.id()));
    }

    #[test]
    fn a_cards_appearance_count_survives_polls_of_the_same_showing_and_bumps_when_it_returns() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let a_id = all[0].id().unwrap();
        let b_id = all[1].id().unwrap();
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);
        assert_eq!(Some(a_id.clone()), session.current().unwrap().id());
        assert_eq!(
            1,
            session.appearance(&a_id),
            "first showing counts as appearance 1"
        );

        session.poll(&mut store, now + 1_000);
        session.poll(&mut store, now + 2_000);
        assert_eq!(1, session.appearance(&a_id), "still the same appearance");

        session.grade(&mut store, Grade::Fail, now);
        assert_eq!(Some(b_id.clone()), session.current().unwrap().id());
        assert_eq!(
            1,
            session.appearance(&a_id),
            "moving off doesn't bump — only being re-served does"
        );

        store.get_or_insert(&a_id).recall.as_mut().unwrap().due_ms =
            now + DEFAULT_INTRODUCTION_COOLDOWN_MS;
        session.grade(&mut store, Grade::Pass, now + 1_000);
        session.poll(&mut store, now + DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000);
        assert_eq!(
            Some(a_id.clone()),
            session.current().unwrap().id(),
            "A is due again"
        );
        assert_eq!(
            2,
            session.appearance(&a_id),
            "a new appearance bumps the count"
        );
    }

    #[test]
    fn same_session_fail_then_pass_does_not_graduate() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        store.get_or_insert(&id).introduced_ms = Some(0);
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);

        session.grade(&mut store, Grade::Fail, now);
        assert!(session.current().is_none());
        let f = store.get(&id).unwrap().recall.unwrap();
        assert_ne!(
            2, f.state,
            "not graduated to Review off an immediate re-drill"
        );
    }

    #[test]
    fn only_cooling_cards_left_finishes_the_session() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(&all[0].id().unwrap()).introduced_ms = Some(0);
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);
        session.grade(&mut store, Grade::Fail, now);
        assert!(session.is_finished(), "nothing due now → finished");
        assert!(
            session.next_due_at(&store).is_some(),
            "the cooling card still has a future due"
        );
    }

    #[test]
    fn passing_removes_a_card_missing_keeps_it() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);
        session.grade(&mut store, Grade::Fail, now);
        session.grade(&mut store, Grade::Pass, now);
        assert!(session.is_finished());
    }

    #[test]
    fn a_recognize_gap_needs_both_the_recognize_depth_and_exclusions() {
        let (mut store, _dir) = empty_store();
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;

        // A Recall sitting never reports a gap, even with excluded cards.
        let mut recall = Session::new(
            cards(1),
            &mut store,
            sched(),
            SessionOptions::default(),
            now,
        );
        recall.set_depth_excluded(cards(2));
        assert_eq!(
            None,
            recall.recognize_gap(&store, now),
            "the gap is a Recognize concept; Recall must not report one"
        );

        // A Recognize sitting with nothing excluded has no gap to report.
        let recognize = Session::new(
            cards(1),
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            now,
        );
        assert_eq!(
            None,
            recognize.recognize_gap(&store, now),
            "no exclusions means nothing was hidden"
        );
    }

    #[test]
    fn next_servable_preserves_the_future_schedule_outside_cram() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 1_000_000;
        let mut session = Session::new(
            all,
            &mut store,
            Box::new(crate::scheduler::Fsrs::new(0.9, 0)),
            SessionOptions {
                new_cards_percent: 0,
                ..Default::default()
            },
            now,
        );
        assert_eq!(Some(id.clone()), session.current().and_then(Card::id));
        session.grade(&mut store, Grade::Fail, now);
        let due = session
            .scheduler
            .due_at(store.get(&id).unwrap(), Depth::Recall);
        assert!(due > now, "a failed card gets a future relearn step");

        assert_eq!(
            Some(due),
            session.next_servable_at(&store, now),
            "Recall uses the stored schedule unless this is explicitly a cram sitting"
        );
    }

    #[test]
    fn a_card_without_stable_identity_is_never_servable() {
        let (mut store, _dir) = empty_store();
        let mut all = cards(1);
        all[0].token = None;
        let session = Session::new(all, &mut store, sched(), SessionOptions::default(), 1_000);

        assert!(
            !session.servable(0, &store, 1_000),
            "progress and review state cannot be attached without a stable card ID"
        );
    }

    #[test]
    fn the_cap_fixes_the_new_set_at_start() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(5),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 2,
                ..Default::default()
            },
            1000,
        );
        let mut introduced = 0;
        while session.current().is_some() {
            session.introduce_current(&mut store, 1000);
            introduced += 1;
            assert!(
                introduced <= 5,
                "the roster must exhaust, never serve past the deck"
            );
        }
        assert_eq!(2, introduced, "the roster fixes the new set at start");
    }

    #[test]
    fn due_cards_take_the_whole_cap_when_the_new_share_is_zero() {
        let (mut store, _dir) = empty_store();
        let all = cards(10);
        for c in &all[7..] {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 3,
                new_cards_percent: 0,
                cram: false,
                order: Order::Scheduled,
                topology: None,
                retire_after_days: Some(DEFAULT_RETIRE_AFTER_DAYS),
                depth: crate::depth::Depth::default(),
            },
            DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000,
        );
        assert_eq!(3, session.initial_size, "the 3 due cards, no new");
        assert_eq!("front 7", session.current().unwrap().front);
    }

    #[test]
    fn a_deep_overdue_card_makes_the_capped_set() {
        let (mut store, _dir) = empty_store();
        // 12 due cards; card 11 is the most overdue (oldest due_at). Under a cap
        // of 5, selection is by due_at, so the deepest-index overdue card is in
        // the first sitting rather than starved by a shallower one.
        let all = cards(12);
        let now = 10 * 604_800_000;
        for (offset, c) in all.iter().enumerate() {
            // Earlier last_review = older due_at; card 11 the oldest.
            let ts = (11 - offset) as u64;
            store.get_or_insert(&c.id().unwrap()).recall = Some(FsrsState {
                stability: 1.0,
                difficulty: 5.0,
                state: 2,
                scheduled_days: 1,
                last_review_ms: ts,
                due_ms: ts,
                ..Default::default()
            });
        }
        let session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 5,
                new_cards_percent: 0,
                ..Default::default()
            },
            now,
        );
        assert_eq!(5, session.initial_size);
        assert_eq!(
            "front 11",
            session.current().unwrap().front,
            "the oldest-due card leads the capped sitting"
        );
    }

    #[test]
    fn due_cards_are_ordered_by_due_time() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        store.get_or_insert(&all[0].id().unwrap()).introduced_ms = Some(0);
        store.get_or_insert(&all[1].id().unwrap()).introduced_ms = Some(0);

        let now = 2 * 604_800_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);
        assert_eq!("front 0", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, now);
        assert_eq!("front 1", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, now);
        assert_eq!("front 2", session.current().unwrap().front);
    }

    #[test]
    fn sequential_order_follows_deck_order() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        store.get_or_insert(&all[0].id().unwrap()).introduced_ms = Some(0);
        store.get_or_insert(&all[1].id().unwrap()).introduced_ms = Some(0);

        let now = 2 * 604_800_000;
        let mut session = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                order: Order::Sequential,
                ..Default::default()
            },
            now,
        );
        assert_eq!("front 0", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, now);
        assert_eq!("front 1", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, now);
        assert_eq!("front 2", session.current().unwrap().front);
    }

    #[test]
    fn cards_on_cooldown_are_not_due() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let now = 5_000_000;
        store.get_or_insert(&all[0].id().unwrap()).introduced_ms = Some(now);

        // An introduced card is a due-pool card, so no-new intake is irrelevant;
        // it is simply not due one ms into its cooldown.
        let session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions::default(),
            now + 1,
        );
        assert!(session.is_finished());

        let session = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                cram: true,
                ..Default::default()
            },
            now + 1,
        );
        assert_eq!(1, session.initial_size);
    }

    #[test]
    fn stats_count_each_grade_across_the_session() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), now);
        assert_eq!(2, session.remaining());

        session.grade(&mut store, Grade::Fail, now);
        session.grade(&mut store, Grade::Pass, now);

        assert_eq!(2, session.stats.reviews);
        assert_eq!(1, session.stats.passed);
        assert_eq!(1, session.stats.failed);
    }

    #[test]
    fn grading_records_fsrs_state() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), 1000);

        session.grade(&mut store, Grade::Pass, 1000);
        let state = store.get(&id).unwrap();
        assert!(state.recall.is_some());
        assert_eq!(1, state.total_reviews);
    }

    #[test]
    fn skip_rotates_queue() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(2),
            &mut store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        let first = session.current().unwrap().front.clone();
        session.skip(&mut store, 1000);
        assert_ne!(first, session.current().unwrap().front);
        assert_eq!(2, session.remaining());
        session.skip(&mut store, 1000);
        assert_eq!(first, session.current().unwrap().front);
        for card in session.cards() {
            assert!(
                store.progress(&card.id().unwrap()).is_none(),
                "skipping persists nothing"
            );
        }
    }

    #[test]
    fn remove_current_drops_card_without_grading() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(2),
            &mut store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        let removed = session.remove_current(&mut store, 1000);
        assert_eq!(1, removed.len());
        assert_eq!(1, session.remaining());
        assert_ne!(removed[0].front, session.current().unwrap().front);
        for card in session.cards() {
            assert!(
                store.progress(&card.id().unwrap()).is_none(),
                "removal persists nothing"
            );
        }
    }

    #[test]
    fn a_removed_region_card_does_not_return_after_restart() {
        let (mut store, _dir) = empty_store();
        let parent = card("deck.md", 1);
        let mut region = parent.clone();
        region.region = Some(crate::card::RegionSlot::Single {
            stamp: Some(Arc::from("a1b2c3")),
            hidden: Some("lunate".into()),
            line: 3,
        });
        let region_id = region.id().unwrap();
        let mut session = Session::new(
            vec![parent, region],
            &mut store,
            sched(),
            SessionOptions::default(),
            0,
        );

        session.introduce_current(&mut store, 0);
        assert_eq!(Some(region_id.as_str()), session.current_id().as_deref());
        let removed = session.remove_current(&mut store, 0);
        assert_eq!(1, removed.len());
        assert_eq!(Some(region_id.as_str()), removed[0].id().as_deref());

        assert!(
            !session.restart(&mut store, 0),
            "restart must not rebuild a region card whose directive and schedule were removed"
        );
        assert_ne!(Some(region_id), session.current_id());
    }

    #[test]
    fn removing_a_cloze_hole_drops_siblings_outside_the_session_cap() {
        let (mut store, _dir) = empty_store();
        let mut current = card("deck.md", 1);
        current.hole = Some(0);
        let mut sibling = card("deck.md", 1);
        sibling.hole = Some(1);
        let sibling_id = sibling.id().unwrap();
        let mut session = Session::new(
            vec![current, sibling],
            &mut store,
            sched(),
            SessionOptions {
                max_session: 1,
                ..SessionOptions::default()
            },
            0,
        );

        let removed = session.remove_current(&mut store, 0);
        assert!(
            removed
                .iter()
                .any(|card| card.id().as_deref() == Some(sibling_id.as_str())),
            "removing one hole removes its source block, so a sibling the cap kept out of the roster must go too"
        );
    }

    #[test]
    fn removing_a_cloze_hole_returns_depth_excluded_siblings_for_store_cleanup() {
        let (mut store, _dir) = empty_store();
        let mut current = card("deck.md", 1);
        current.hole = Some(0);
        let mut sibling = card("deck.md", 1);
        sibling.hole = Some(1);
        let sibling_id = sibling.id().unwrap();
        store.get_or_insert(&sibling_id).introduced_ms = Some(0);
        let mut session = Session::new(
            vec![current],
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..SessionOptions::default()
            },
            0,
        );
        session.set_depth_excluded(vec![sibling]);

        let removed = session.remove_current(&mut store, 0);
        for card in removed {
            if let Some(id) = card.id() {
                store.remove(&id);
            }
        }

        assert!(
            store.progress(&sibling_id).is_none(),
            "removing one hole removes its source block, so a depth-excluded sibling must come back for the serve layer to clear its progress"
        );
    }

    #[test]
    fn remove_current_also_drops_cloze_siblings() {
        let (mut store, _dir) = empty_store();
        let mut all = vec![card("deck.md", 1), card("deck.md", 1), card("deck.md", 2)];
        all[0].back = vec!["hole a".into()];
        all[0].hole = Some(0);
        all[1].back = vec!["hole b".into()];
        all[1].hole = Some(1);
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), 0);
        assert_eq!(3, session.remaining());
        let removed = session.remove_current(&mut store, 0);
        assert_eq!(2, removed.len());
        assert_eq!(1, session.remaining());
        assert_eq!(2, session.current().unwrap().line);
    }

    #[test]
    fn sibling_grouping_follows_deck_id_not_the_filename() {
        let (mut store, _dir) = empty_store();
        // Same deck_id, different filenames: still grouped as siblings.
        let mut a = card("a.md", 1);
        a.deck_id = Arc::from("shared-deck");
        a.back = vec!["hole a".into()];
        a.hole = Some(0);
        let mut b = card("b.md", 1);
        b.deck_id = Arc::from("shared-deck");
        b.back = vec!["hole b".into()];
        b.hole = Some(1);
        let mut session = Session::new(
            vec![a, b],
            &mut store,
            sched(),
            SessionOptions::default(),
            0,
        );
        assert_eq!(2, session.remaining());
        let removed = session.remove_current(&mut store, 0);
        assert_eq!(
            2,
            removed.len(),
            "cards sharing a deck_id and line group as siblings even under different filenames"
        );
    }

    #[test]
    fn sibling_grouping_does_not_merge_across_deck_ids_sharing_a_filename() {
        let (mut store, _dir) = empty_store();
        // Same filename, different deck_id: must not be treated as siblings.
        let mut a = card("deck.md", 1);
        a.deck_id = Arc::from("deck-one");
        let mut b = card("deck.md", 1);
        b.deck_id = Arc::from("deck-two");
        b.token = Some(Arc::from("tok1b"));
        let mut session = Session::new(
            vec![a, b],
            &mut store,
            sched(),
            SessionOptions::default(),
            0,
        );
        assert_eq!(2, session.remaining());
        let removed = session.remove_current(&mut store, 0);
        assert_eq!(
            1,
            removed.len(),
            "a shared filename alone must not group cards from different decks"
        );
    }

    #[test]
    fn cloze_siblings_are_separated() {
        let (mut store, _dir) = empty_store();
        let mut all = Vec::new();
        for (line, name) in [(1, "A"), (2, "B")] {
            for hole in 1..=2 {
                let mut c = card("deck.md", line);
                c.front = format!("{name}{hole}");
                c.back = vec![format!("{name} answer {hole}")];
                c.hole = Some(hole as u32 - 1);
                all.push(c);
            }
        }
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), 0);

        let mut fronts = Vec::new();
        for _ in 0..session.remaining() {
            fronts.push(session.current().unwrap().front.clone());
            session.skip(&mut store, 0);
        }
        assert_eq!(4, fronts.len());
        for pair in fronts.windows(2) {
            assert_ne!(
                pair[0].chars().next(),
                pair[1].chars().next(),
                "siblings adjacent in queue: {fronts:?}"
            );
        }
    }

    #[test]
    fn lone_sibling_group_still_fully_queued() {
        let (mut store, _dir) = empty_store();
        let mut all = Vec::new();
        for hole in 1..=3 {
            let mut c = card("deck.md", 1);
            c.back = vec![format!("answer {hole}")];
            c.hole = Some(hole as u32 - 1);
            all.push(c);
        }
        let session = Session::new(all, &mut store, sched(), SessionOptions::default(), 0);
        assert_eq!(3, session.initial_size);
    }

    #[test]
    fn restart_picks_up_newly_due_and_new_cards() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(4),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 2,
                ..Default::default()
            },
            1000,
        );
        assert_eq!(2, session.initial_size);
        session.grade(&mut store, Grade::Pass, 1000);
        session.grade(&mut store, Grade::Pass, 1001);
        assert!(session.is_finished());
        assert_eq!(2, session.stats.reviews);

        assert!(session.restart(&mut store, 1002));
        assert_eq!(2, session.initial_size);
        assert_eq!(0, session.stats.reviews);
        assert!(!session.is_finished());
    }

    #[test]
    fn restart_with_nothing_due_returns_false_and_keeps_stats() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(1),
            &mut store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        session.grade(&mut store, Grade::Pass, 1000);
        assert!(session.is_finished());

        assert!(!session.restart(&mut store, 1001));
        assert!(session.is_finished());
        assert_eq!(1, session.stats.reviews);
    }

    #[test]
    fn has_due_now_tracks_what_restart_would_find() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(1),
            &mut store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        assert!(session.has_due_now(&store, 1000));
        session.grade(&mut store, Grade::Pass, 1000);
        assert!(!session.has_due_now(&store, 1001));
        assert!(!session.restart(&mut store, 1001));
        assert!(session.has_due_now(&store, 1000 + 3_600_000));
    }

    #[test]
    fn next_due_at_reports_earliest_due_time() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), 1000);
        assert_eq!(None, session.next_due_at(&store));
        session.grade(&mut store, Grade::Pass, 1000);
        let due = session
            .next_due_at(&store)
            .expect("a seen card has a due time");
        assert!(due > 1000 && due < 1000 + 86_400_000, "due {due}");
    }

    fn retired_fsrs() -> crate::store::FsrsState {
        crate::store::FsrsState {
            scheduled_days: DEFAULT_RETIRE_AFTER_DAYS,
            ..Default::default()
        }
    }

    #[test]
    fn is_retired_once_the_interval_passes_the_cap() {
        let (mut store, _dir) = empty_store();
        let c = card("deck.md", 0);

        assert!(!is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
        store.get_or_insert(&c.id().unwrap()).recall = Some(retired_fsrs());
        assert!(is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
        store.get_or_insert(&c.id().unwrap()).recall = Some(crate::store::FsrsState {
            scheduled_days: DEFAULT_RETIRE_AFTER_DAYS - 1,
            ..Default::default()
        });
        assert!(!is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
        let s = store.get_or_insert(&c.id().unwrap());
        s.recall = None;
        s.streak = 1;
        assert!(!is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
    }

    #[test]
    fn has_reviewable_counts_new_and_due_not_cooldown_or_retired() {
        let (mut store, _dir) = empty_store();
        let sched = sched();
        let now = 10_000_000;

        assert!(has_reviewable(
            &cards(1),
            &store,
            sched.as_ref(),
            Depth::Recall,
            now,
            Some(DEFAULT_RETIRE_AFTER_DAYS)
        ));

        let c = card("deck.md", 0);
        let s = store.get_or_insert(&c.id().unwrap());
        s.streak = 1;
        s.introduced_ms = Some(now);
        let one = std::slice::from_ref(&c);
        let cap = Some(DEFAULT_RETIRE_AFTER_DAYS);
        assert!(!has_reviewable(
            one,
            &store,
            sched.as_ref(),
            Depth::Recall,
            now,
            cap
        ));
        assert!(has_reviewable(
            one,
            &store,
            sched.as_ref(),
            Depth::Recall,
            now + 3_600_000,
            cap
        ));

        store.get_or_insert(&c.id().unwrap()).recall = Some(retired_fsrs());
        assert!(!has_reviewable(
            std::slice::from_ref(&c),
            &store,
            sched.as_ref(),
            Depth::Recall,
            now + 3_600_000,
            cap
        ));
    }

    #[test]
    fn retired_card_excluded_even_under_cram() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(&all[0].id().unwrap()).recall = Some(retired_fsrs());

        let session = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                cram: true,
                ..Default::default()
            },
            1000,
        );
        assert!(session.is_finished());
    }

    #[test]
    fn a_due_cram_pass_grades_like_a_normal_review() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(&all[0].id().unwrap()).recall = Some(mature_fsrs(1000));
        let now = 40 * 86_400_000;

        let mut session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                cram: true,
                ..Default::default()
            },
            now,
        );
        session.grade(&mut store, Grade::Pass, now);

        let after = store.get(&all[0].id().unwrap()).unwrap();
        assert_eq!(1, after.history.len(), "a due cram pass is a real review");
        let f = after.recall.unwrap();
        assert!(f.stability > 30.0, "full credit, not a re-anchor");
        assert!(f.due_ms > now);
    }

    #[test]
    fn an_early_cram_pass_reanchors_without_rewarding() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let now = 10 * 86_400_000;
        store.get_or_insert(&all[0].id().unwrap()).recall = Some(mature_fsrs(40 * 86_400_000));
        let before = store.get(&all[0].id().unwrap()).unwrap().recall.unwrap();

        let mut session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                cram: true,
                ..Default::default()
            },
            now,
        );
        session.grade(&mut store, Grade::Pass, now);

        let after = store.get(&all[0].id().unwrap()).unwrap();
        let f = after.recall.unwrap();
        assert_eq!(before.stability, f.stability);
        assert_eq!(before.scheduled_days, f.scheduled_days);
        assert_eq!(now + 30 * 86_400_000, f.due_ms);
        assert!(after.history.is_empty());
    }

    #[test]
    fn cram_miss_lapses_normally() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(&all[0].id().unwrap()).recall = Some(crate::store::FsrsState {
            stability: 30.0,
            difficulty: 5.0,
            scheduled_days: 30,
            state: 2,
            ..Default::default()
        });

        let mut session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                cram: true,
                ..Default::default()
            },
            10_000,
        );
        session.grade(&mut store, Grade::Fail, 10_000);

        let after = store.get(&all[0].id().unwrap()).unwrap();
        assert_eq!(1, after.history.len());
        assert!(after.recall.unwrap().stability < 30.0);
    }

    #[test]
    fn cram_serves_each_card_once() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let id_a = all[0].id().unwrap();
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).recall = Some(crate::store::FsrsState {
                stability: 30.0,
                difficulty: 5.0,
                scheduled_days: 30,
                state: 2,
                ..Default::default()
            });
        }

        let mut session = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                cram: true,
                ..Default::default()
            },
            10_000,
        );
        session.grade(&mut store, Grade::Fail, 10_000);
        session.grade(&mut store, Grade::Pass, 10_000);
        assert!(
            session.is_finished(),
            "cram is a single pass over the roster"
        );
        assert_eq!(1, store.get(&id_a).unwrap().history.len());
    }

    #[test]
    fn chained_cram_serves_disjoint_batches_off_a_first_learning_event() {
        let (mut store, _dir) = empty_store();
        // Three introduced-but-unscheduled cards. A cram pass on a not-yet-due
        // card with no schedule must be a genuine first learning event, so its
        // due moves out and a chained sitting reaches the next card instead of
        // looping the same one.
        let all = cards(3);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let t1 = 1;
        let mut s = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 1,
                cram: true,
                ..Default::default()
            },
            t1,
        );
        let first = s.current().unwrap().id().unwrap();
        s.grade(&mut store, Grade::Pass, t1);
        assert!(
            store.get(&first).unwrap().recall.is_some(),
            "a first cram pass applies a real schedule, not a no-op re-anchor"
        );
        assert!(s.is_finished(), "cap 1 serves one card per sitting");

        // Restart past the first card's floor: its due has moved out, so it no
        // longer leads, and a fresh card takes the slot.
        assert!(s.restart(&mut store, t1 + DEFAULT_INTRODUCTION_COOLDOWN_MS));
        let second = s.current().unwrap().id().unwrap();
        assert_ne!(first, second, "the chained batch is disjoint");
    }

    #[test]
    fn a_chained_sitting_skips_a_cooling_card_for_the_next_servable() {
        let (mut store, _dir) = empty_store();
        // Card 0 is the oldest-due; cards 1 and 2 follow. Grade card 0 (it
        // floors), pin its due back to the front so, absent the floor, it would
        // lead again, then chain within the cooldown: the surviving floor holds
        // it back and the next servable card takes the sitting (never empty
        // while a servable card remains).
        let all = cards(3);
        let a = all[0].id().unwrap();
        store.get_or_insert(&a).recall = Some(mature_fsrs(5));
        for c in &all[1..] {
            store.get_or_insert(&c.id().unwrap()).recall = Some(mature_fsrs(1_000));
        }
        let t1 = 2_000_000;
        let mut s = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 1,
                new_cards_percent: 0,
                cram: true,
                ..Default::default()
            },
            t1,
        );
        assert_eq!(
            Some(a.clone()),
            s.current().unwrap().id(),
            "oldest-due leads"
        );
        s.grade(&mut store, Grade::Pass, t1);
        store.get_or_insert(&a).recall.as_mut().unwrap().due_ms = 5;
        assert!(s.is_finished(), "cap 1: the sitting ends after the grade");

        let t2 = t1 + 1_000;
        assert!(
            s.restart(&mut store, t2),
            "servable cards remain, so the sitting is not empty"
        );
        assert_ne!(
            a,
            s.current().unwrap().id().unwrap(),
            "the cooling card is skipped for the next servable one"
        );
    }

    #[test]
    fn the_new_pool_round_robins_sibling_groups_into_the_cap() {
        let (mut store, _dir) = empty_store();
        // Two six-hole clozes plus four singles: without round-robin the first
        // ten slots would be eaten by the two cloze groups; the round-robin
        // spreads the cap across many distinct facts.
        let mut all = Vec::new();
        for line in [1usize, 2] {
            for hole in 0..6u32 {
                let mut c = card("deck.md", line);
                c.token = Some(Arc::from(format!("clz{line}").as_str()));
                c.hole = Some(hole);
                c.back = vec![format!("h{line}-{hole}")];
                all.push(c);
            }
        }
        for line in [3usize, 4, 5, 6] {
            all.push(card("deck.md", line));
        }
        let session = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                max_session: 10,
                ..Default::default()
            },
            1_000,
        );
        assert_eq!(10, session.initial_size);
        let groups: std::collections::HashSet<(String, usize)> = session
            .roster
            .iter()
            .map(|&i| (session.cards[i].deck_id.to_string(), session.cards[i].line))
            .collect();
        assert!(
            groups.len() > 2,
            "the capped sitting spans more than the two cloze groups: {} groups",
            groups.len()
        );
    }

    #[test]
    fn cram_remaining_split_subtracts_what_the_sitting_already_drilled() {
        let (mut store, _dir) = empty_store();
        // Five introduced cards, all eligible under cram; cap 2. After drilling the
        // sitting, the backlog counts the three not yet served, not all five.
        let all = cards(5);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).recall = Some(mature_fsrs(10));
        }
        let now = 2_000_000;
        let mut s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                max_session: 2,
                new_cards_percent: 0,
                cram: true,
                ..Default::default()
            },
            now,
        );
        assert_eq!(
            (5, 0),
            s.remaining_split(&store, now),
            "all five ahead at start"
        );
        s.grade(&mut store, Grade::Pass, now);
        s.grade(&mut store, Grade::Pass, now);
        assert!(s.is_finished());
        let (due_left, new_left) = s.remaining_split(&store, now);
        assert_eq!(
            (3, 0),
            (due_left, new_left),
            "the two crammed cards drop out of the eligible count"
        );
    }

    fn topology_order(walk: &[&Card]) -> TopologyOrder {
        let ids: Vec<String> = walk.iter().filter_map(|c| c.id()).collect();
        TopologyOrder::from_walk(&ids)
    }

    #[test]
    fn topology_reorders_the_due_set() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let topo = topology_order(&[&all[2], &all[1], &all[0]]);
        let mut session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                topology: Some(topo),
                ..Default::default()
            },
            1_000_000,
        );
        assert_eq!("front 2", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 1", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 0", session.current().unwrap().front);
    }

    #[test]
    fn topology_only_reorders_does_not_readmit_a_card_that_is_not_due() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let now = 5_000_000;
        store.get_or_insert(&all[0].id().unwrap()).introduced_ms = Some(0);
        store.get_or_insert(&all[1].id().unwrap()).introduced_ms = Some(now);
        let topo = topology_order(&[&all[1], &all[0]]);
        // Both cards are introduced, so the never-met pool is empty; only card 0
        // is due, and the walk cannot re-admit the not-due card 1.
        let session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                topology: Some(topo),
                ..Default::default()
            },
            now + 1,
        );
        assert_eq!(1, session.initial_size);
        assert_eq!("front 0", session.current().unwrap().front);
    }

    #[test]
    fn cards_not_in_walk_append_in_scheduler_order() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let topo = topology_order(&[&all[1]]);
        let mut session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                topology: Some(topo),
                ..Default::default()
            },
            1_000_000,
        );
        assert_eq!("front 1", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 0", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 2", session.current().unwrap().front);
    }

    #[test]
    fn retired_card_excluded_even_with_a_topology() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(&all[0].id().unwrap()).recall = Some(retired_fsrs());
        let topo = topology_order(&[&all[0]]);
        let session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                topology: Some(topo),
                ..Default::default()
            },
            1000,
        );
        assert!(session.is_finished());
    }

    #[test]
    fn topology_keeps_cloze_siblings_in_walk_order_skipping_separation() {
        let (mut store, _dir) = empty_store();
        let mut sib_a = Card::plain(
            Arc::from("d.md"),
            "front a".into(),
            vec!["a".into()],
            None,
            7,
        );
        sib_a.token = Some(Arc::from("sib"));
        sib_a.hole = Some(0);
        let mut sib_b = Card::plain(
            Arc::from("d.md"),
            "front b".into(),
            vec!["b".into()],
            None,
            7,
        );
        sib_b.token = Some(Arc::from("sib"));
        sib_b.hole = Some(1);
        let other = card("d.md", 3);
        let all = vec![sib_a.clone(), sib_b.clone(), other.clone()];
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let topo = topology_order(&[&sib_a, &sib_b, &other]);
        let mut session = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                topology: Some(topo),
                ..Default::default()
            },
            1_000_000,
        );
        assert_eq!("front a", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front b", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 3", session.current().unwrap().front);
    }

    #[test]
    fn a_deep_rank_overdue_card_is_not_starved_by_the_walk_sort() {
        let (mut store, _dir) = empty_store();
        // Four due cards; card 3 is the most overdue but sits last in the walk.
        // Selecting by due_at (not rank) then presenting by rank keeps the deep
        // overdue card in the capped sitting; the old topo-sort-then-truncate
        // would drop it for two shallow-rank cards.
        let all = cards(4);
        let now = 10 * 604_800_000;
        for (i, c) in all.iter().enumerate() {
            let ts = i as u64; // card 3 has the smallest (oldest) due_at
            store.get_or_insert(&c.id().unwrap()).recall = Some(FsrsState {
                stability: 1.0,
                difficulty: 5.0,
                state: 2,
                scheduled_days: 1,
                last_review_ms: 3 - ts,
                due_ms: 3 - ts,
                ..Default::default()
            });
        }
        // Walk order 0,1,2,3 → card 3 is the deepest rank.
        let topo = topology_order(&[&all[0], &all[1], &all[2], &all[3]]);
        let session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 2,
                new_cards_percent: 0,
                topology: Some(topo),
                ..Default::default()
            },
            now,
        );
        assert_eq!(2, session.initial_size);
        let served: Vec<String> = session
            .roster
            .iter()
            .map(|&i| all[i].id().unwrap())
            .collect();
        assert!(
            served.contains(&all[3].id().unwrap()),
            "the deep-rank overdue card made the capped sitting"
        );
    }

    const CAP: Option<u32> = Some(DEFAULT_RETIRE_AFTER_DAYS);

    #[test]
    fn durable_state_changes_only_when_leaving_the_current_card() {
        // ADR 0035's invariant, law-shaped: for every session entry point that
        // takes `&mut Store`, either the current card changes or the
        // serialized store is byte-identical afterwards. Presentation and
        // arrival write nothing; grading and the Seen press write on
        // departure.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.json");
        let mut store = Store::open(&path).unwrap();
        // The saved document's `revision` ticks on every save, so the law
        // compares the card state itself.
        let snapshot = |store: &mut Store| {
            store.save().unwrap();
            let value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            format!("{}{}", value["cards"], value["records"])
        };
        let all = cards(2);
        let before_build = snapshot(&mut store);
        let mut session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions::default(),
            1_000,
        );
        assert_eq!(
            before_build,
            snapshot(&mut store),
            "building a session presents a card and writes nothing"
        );

        type Action = Box<dyn Fn(&mut Session, &mut Store)>;
        let cases: Vec<(&str, Action)> = vec![
            (
                "poll",
                Box::new(|s, st| {
                    s.poll(st, 2_000);
                }),
            ),
            ("skip", Box::new(|s, st| s.skip(st, 2_000))),
            (
                "restart",
                Box::new(|s, st| {
                    s.restart(st, 2_000);
                }),
            ),
        ];
        for (name, act) in cases {
            let current = session.current().and_then(|c| c.id());
            let before = snapshot(&mut store);
            act(&mut session, &mut store);
            let after_current = session.current().and_then(|c| c.id());
            assert!(
                current != after_current || before == snapshot(&mut store),
                "{name}: stayed on the card yet mutated the store"
            );
        }

        let before = snapshot(&mut store);
        let current = session.current().and_then(|c| c.id());
        session.grade(&mut store, Grade::Pass, 2_000);
        assert_ne!(
            current,
            session.current().and_then(|c| c.id()),
            "grading departs"
        );
        assert_ne!(before, snapshot(&mut store), "grading writes on departure");

        let before = snapshot(&mut store);
        let current = session.current().and_then(|c| c.id());
        session.introduce_current(&mut store, 2_000);
        assert_ne!(
            current,
            session.current().and_then(|c| c.id()),
            "the Seen press departs"
        );
        assert_ne!(
            before,
            snapshot(&mut store),
            "the Seen press writes on departure"
        );
    }

    #[test]
    fn a_presented_then_failed_card_is_seen_with_no_introduction_fact() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let mut session = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            1_000,
        );
        session.grade(&mut store, Grade::Fail, 1_000);

        let state = store.get(&id).expect("the failed attempt left an entry");
        assert_eq!(
            None, state.introduced_ms,
            "a grade never introduces; only the Seen press does"
        );
        assert_eq!(0, state.total_passes);
        assert_eq!(
            vec![CardTier::Seen],
            card_tiers(&[id], &store, 1_000, CAP),
            "a failed attempt reads as seen, never learning"
        );
    }

    #[test]
    fn presentation_alone_leaves_the_card_fresh_for_the_next_session() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let session = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions::default(),
            1_000,
        );
        assert!(
            session.current_fresh(&store),
            "still the introduction on-ramp"
        );
        assert_eq!(None, session.next_due_at(&store));
        drop(session);
        assert!(
            store.get(&id).is_none(),
            "presentation alone writes nothing at all"
        );

        let capped = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                max_session: 0,
                ..Default::default()
            },
            2_000,
        );
        assert_eq!(
            0, capped.initial_size,
            "a presented-only card is never-met, and a zero cap admits nothing"
        );
    }

    #[test]
    fn card_tiers_maps_unseen_seen_learning_and_retired() {
        let (mut store, _dir) = empty_store();
        let seen = "seen".to_string();
        store.get_or_insert(&seen).introduced_ms = Some(1_000);
        let learning = "learning".to_string();
        store
            .get_or_insert(&learning)
            .record_review(1_000, Grade::Pass, Depth::Recall, false);
        let retired = "retired".to_string();
        store.get_or_insert(&retired).recall = Some(retired_fsrs());
        let ids = ["unseen".to_string(), seen, learning, retired];
        assert_eq!(
            vec![
                CardTier::Unseen,
                CardTier::Seen,
                CardTier::Learning,
                CardTier::Retired,
            ],
            card_tiers(&ids, &store, 1_000, CAP)
        );
        assert!(card_tiers(&[], &store, 0, CAP).is_empty());
    }

    #[test]
    fn a_first_sight_fail_is_seen_never_learning() {
        // The published ladder's core promise: the tier reflects what the
        // learner DID. A wrong answer is not "correct at least once".
        let (mut store, _dir) = empty_store();
        let id = "missed".to_string();
        store
            .get_or_insert(&id)
            .record_review(1_000, Grade::Fail, Depth::Recall, false);
        assert_eq!(
            vec![CardTier::Seen],
            card_tiers(std::slice::from_ref(&id), &store, 1_000, CAP)
        );
    }

    #[test]
    fn learned_tiers_band_current_retrievability_not_history() {
        let (mut store, _dir) = empty_store();
        let id = "learned".to_string();
        store.get_or_insert(&id).recall = Some(FsrsState {
            stability: 10.0,
            state: 2,
            ..Default::default()
        });
        let day = 86_400_000u64;
        let r_at =
            |days: u64| Parameters::forgetting_curve(days as f64, 10.0).clamp(0.0, 1.0) as f32;

        assert!(r_at(0) >= LEARNED_STRONG_MIN);
        assert_eq!(
            vec![CardTier::LearnedStrong],
            card_tiers(std::slice::from_ref(&id), &store, 0, CAP)
        );

        let mid = r_at(30);
        assert!(
            (LEARNED_WEAK_BELOW..LEARNED_STRONG_MIN).contains(&mid),
            "r(30d) = {mid}"
        );
        assert_eq!(
            vec![CardTier::LearnedFading],
            card_tiers(std::slice::from_ref(&id), &store, 30 * day, CAP),
            "the same card decays into the middle band"
        );

        let low = r_at(100);
        assert!(low < LEARNED_WEAK_BELOW, "r(100d) = {low}");
        assert_eq!(
            vec![CardTier::LearnedWeak],
            card_tiers(std::slice::from_ref(&id), &store, 100 * day, CAP)
        );

        store.get_or_insert(&id).recall = Some(FsrsState {
            stability: 10.0,
            state: 1,
            ..Default::default()
        });
        store
            .get_or_insert(&id)
            .record_review(0, Grade::Pass, Depth::Recall, false);
        assert_eq!(
            vec![CardTier::Learning],
            card_tiers(std::slice::from_ref(&id), &store, 0, CAP),
            "a not-yet-graduated passed card is learning, never banded"
        );
    }

    #[test]
    fn every_engaged_card_carries_its_engagement_not_a_presentation() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        let mut session = Session::new(all, &mut store, sched(), SessionOptions::default(), 1_000);
        session.introduce_current(&mut store, 1_000);
        session.introduce_current(&mut store, 1_000);
        session.introduce_current(&mut store, 1_000);
        let now = 1_000 + DEFAULT_INTRODUCTION_COOLDOWN_MS;
        session.poll(&mut store, now);
        session.grade(&mut store, Grade::Fail, now);
        session.grade(&mut store, Grade::Pass, now);

        let ids: Vec<String> = session.cards().iter().filter_map(|c| c.id()).collect();
        for id in &ids {
            let state = store.get(id).expect("every card was served");
            assert!(
                state.engaged(),
                "an entry exists only because the learner DID something ({id})"
            );
        }
    }

    #[test]
    fn a_personal_card_joins_the_roster_and_is_served() {
        let (mut store, _dir) = empty_store();
        let synth = personal_card(&mut store, "deck.md", "personal back", 0);
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;
        let session = Session::new(
            vec![synth],
            &mut store,
            sched(),
            SessionOptions::default(),
            now,
        );
        assert_eq!(1, session.initial_size);
        assert_eq!("personal front", session.current().unwrap().front);
    }

    #[test]
    fn grading_a_personal_card_updates_store_cards() {
        let (mut store, _dir) = empty_store();
        let synth = personal_card(&mut store, "deck.md", "personal back", 0);
        let id = synth.id().unwrap();
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;
        let mut session = Session::new(
            vec![synth],
            &mut store,
            sched(),
            SessionOptions::default(),
            now,
        );

        session.grade(&mut store, Grade::Pass, now);

        let state = store.get(&id).expect("the personal card's schedule");
        assert!(state.recall.is_some());
        assert_eq!(1, state.total_reviews);
    }

    #[test]
    fn a_personal_card_is_not_treated_as_unseen() {
        let (mut store, _dir) = empty_store();
        let synth = personal_card(&mut store, "deck.md", "personal back", 0);
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;
        let session = Session::new(
            vec![synth],
            &mut store,
            sched(),
            SessionOptions::default(),
            now,
        );
        assert!(!session.current_fresh(&store));
    }

    #[test]
    fn a_missed_personal_card_reappears_on_its_fsrs_due() {
        let (mut store, _dir) = empty_store();
        let synth = personal_card(&mut store, "deck.md", "personal back", 0);
        let synth_id = synth.id();
        let deck_card = card("deck.md", 0);
        store.get_or_insert(&deck_card.id().unwrap()).introduced_ms = Some(0);

        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = Session::new(
            vec![synth, deck_card],
            &mut store,
            sched(),
            SessionOptions::default(),
            now,
        );

        assert_eq!(synth_id, session.current().unwrap().id());
        session.grade(&mut store, Grade::Fail, now);
        session.grade(&mut store, Grade::Pass, now + 1000);
        session.poll(&mut store, now + DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000);
        assert_eq!(synth_id, session.current().unwrap().id());
    }

    #[test]
    fn counting_personal_cards_counts_due_and_excludes_archived() {
        let (mut store, _dir) = empty_store();
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;
        let cap = Some(DEFAULT_RETIRE_AFTER_DAYS);
        let sched = sched();

        let due = personal_card(&mut store, "deck.md", "gap-due", 0);
        let not_due = personal_card(&mut store, "deck.md", "gap-not-due", now);
        let archived = personal_card(&mut store, "deck.md", "gap-archived", 0);
        store.get_or_insert(&archived.id().unwrap()).recall = Some(retired_fsrs());

        let personal = [due, not_due, archived];
        assert_eq!(
            1,
            count_reviewable(
                &personal.iter().collect::<Vec<_>>(),
                &store,
                sched.as_ref(),
                Depth::Recall,
                now,
                cap
            )
        );
        assert!(has_reviewable(
            &personal,
            &store,
            sched.as_ref(),
            Depth::Recall,
            now,
            cap
        ));
    }

    #[test]
    fn next_due_at_includes_personal_cards() {
        let (mut store, _dir) = empty_store();
        let synth = personal_card(&mut store, "deck.md", "personal back", 1000);
        let session = Session::new(
            vec![synth],
            &mut store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        let due = session
            .next_due_at(&store)
            .expect("a personal card's due time is reported");
        assert_eq!(1000 + DEFAULT_INTRODUCTION_COOLDOWN_MS, due);
    }

    #[test]
    fn a_personal_card_is_retired_when_its_interval_reaches_the_cap() {
        let (mut store, _dir) = empty_store();
        let synth = personal_card(&mut store, "deck.md", "personal back", 0);
        let id = synth.id().unwrap();
        let options = SessionOptions {
            retire_after_days: Some(4),
            ..SessionOptions::default()
        };

        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;
        let mut session = Session::new(
            vec![synth.clone()],
            &mut store,
            sched(),
            options.clone(),
            now,
        );
        session.grade(&mut store, Grade::Pass, now);
        assert!(!is_retired_id(&id, &store, options.retire_after_days));

        let now = 86_460_000;
        let mut session = Session::new(
            vec![synth.clone()],
            &mut store,
            sched(),
            options.clone(),
            now,
        );
        session.grade(&mut store, Grade::Pass, now);

        assert!(is_retired_id(&id, &store, options.retire_after_days));
        let state = store.get(&id).expect("schedule kept, not deleted");
        assert_eq!(4, state.recall.as_ref().unwrap().scheduled_days);
        assert_eq!(2, state.total_reviews);

        let session = Session::new(
            vec![synth.clone()],
            &mut store,
            sched(),
            options.clone(),
            now,
        );
        assert!(session.is_finished());
        assert_eq!(
            0,
            count_reviewable(
                &[&synth],
                &store,
                sched().as_ref(),
                Depth::Recall,
                now,
                options.retire_after_days
            )
        );
    }

    #[test]
    fn raising_retire_after_un_retires_a_personal_card() {
        let (mut store, _dir) = empty_store();
        let synth = personal_card(&mut store, "deck.md", "personal back", 0);
        let id = synth.id().unwrap();
        store.get_or_insert(&id).recall = Some(crate::store::FsrsState {
            scheduled_days: 10,
            ..Default::default()
        });
        assert!(is_retired_id(&id, &store, Some(10)));
        assert!(!is_retired_id(&id, &store, Some(20)));

        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;
        let sched = sched();
        assert_eq!(
            0,
            count_reviewable(
                &[&synth],
                &store,
                sched.as_ref(),
                Depth::Recall,
                now,
                Some(10)
            )
        );
        assert_eq!(
            1,
            count_reviewable(
                &[&synth],
                &store,
                sched.as_ref(),
                Depth::Recall,
                now,
                Some(20)
            )
        );
    }

    #[test]
    fn a_retired_personal_card_is_excluded_from_the_queue_and_counts() {
        let (mut store, _dir) = empty_store();
        let synth = personal_card(&mut store, "deck.md", "personal back", 0);
        let id = synth.id().unwrap();
        store.get_or_insert(&id).recall = Some(retired_fsrs());

        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;
        let session = Session::new(
            vec![synth.clone()],
            &mut store,
            sched(),
            SessionOptions::default(),
            now,
        );
        assert!(session.is_finished());

        let cap = Some(DEFAULT_RETIRE_AFTER_DAYS);
        assert_eq!(
            0,
            count_reviewable(&[&synth], &store, sched().as_ref(), Depth::Recall, now, cap)
        );
    }

    #[test]
    fn retire_only_at_cap_not_below() {
        let (mut store, _dir) = empty_store();
        let synth = personal_card(&mut store, "deck.md", "personal back", 0);
        let id = synth.id().unwrap();

        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;
        let mut session = Session::new(
            vec![synth],
            &mut store,
            sched(),
            SessionOptions::default(),
            now,
        );
        session.grade(&mut store, Grade::Pass, now);

        assert!(!is_retired_id(&id, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
    }

    #[test]
    fn a_reconstruct_grade_never_touches_the_recall_schedule() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        store.get_or_insert(&id).recall = Some(FsrsState {
            stability: 30.0,
            state: 2,
            ..Default::default()
        });
        let mut s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Reconstruct,
                ..Default::default()
            },
            0,
        );
        s.grade(&mut store, Grade::Fail, 1_000);
        let st = store.get(&id).unwrap();
        assert_eq!(
            30.0,
            st.recall.unwrap().stability,
            "recall untouched by a reconstruct fail"
        );
        assert!(
            st.reconstruct.is_some(),
            "reconstruct schedule seeded lazily"
        );
    }

    #[test]
    fn a_recall_drilled_deck_is_immediately_due_at_reconstruct() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(&all[0].id().unwrap()).recall = Some(FsrsState {
            stability: 30.0,
            state: 2,
            due_ms: u64::MAX,
            ..Default::default()
        });
        let s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Reconstruct,
                ..Default::default()
            },
            1_000_000,
        );
        assert_eq!(1, s.remaining(), "lazy reconstruct schedule = due now");
    }

    #[test]
    fn recognize_marks_a_correct_pick_and_requeues_a_floored_wrong_one() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let (a, b) = (all[0].id().unwrap(), all[1].id().unwrap());
        let mut s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            0,
        );
        s.grade(&mut store, Grade::Pass, 1_000);
        s.grade(&mut store, Grade::Fail, 2_000);
        assert!(
            store.get(&a).unwrap().recognize.is_some(),
            "a correct pick creates the recognize schedule"
        );
        assert!(
            store.get(&b).unwrap().recognize.is_some(),
            "a failed pick is a review too and seeds its learning schedule"
        );
        assert!(
            store
                .get(&b)
                .unwrap()
                .history
                .iter()
                .all(|r| !r.grade.passed()),
            "the seeded schedule carries the fail, not a pass"
        );
        assert!(
            store.get(&a).unwrap().recall.is_none(),
            "a recognize pass schedules its own depth, never recall"
        );
        assert_eq!(
            0,
            s.remaining(),
            "the wrong pick re-queues, but the floor holds it back"
        );
        s.poll(&mut store, 2_000 + DEFAULT_INTRODUCTION_COOLDOWN_MS);
        assert_eq!(
            1,
            s.remaining(),
            "past the floor, the re-queued card returns"
        );
    }

    #[test]
    fn a_second_wrong_pick_does_not_unfloor_the_first_recognize_card() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        let (a, b, c) = (all[0].id(), all[1].id(), all[2].id());
        let mut s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            0,
        );
        assert_eq!(a, s.current().unwrap().id());

        s.grade(&mut store, Grade::Fail, 1_000);
        assert_eq!(b, s.current().unwrap().id());

        s.grade(&mut store, Grade::Fail, 2_000);
        assert_eq!(
            c,
            s.current().unwrap().id(),
            "A and B are both still floored — C is the only unfloored card left"
        );

        s.grade(
            &mut store,
            Grade::Fail,
            1_000 + DEFAULT_INTRODUCTION_COOLDOWN_MS + 500,
        );
        assert_eq!(
            a,
            s.current().unwrap().id(),
            "A's own floor has passed (B's hasn't): floors are independent per card"
        );
    }

    #[test]
    fn a_recognize_wrong_pick_may_repeat_once_the_floor_passes() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let mut s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            0,
        );

        s.grade(&mut store, Grade::Fail, 1_000);
        assert!(
            s.is_finished(),
            "the only card floors instead of resurfacing instantly"
        );

        s.poll(&mut store, 1_000 + DEFAULT_INTRODUCTION_COOLDOWN_MS - 1);
        assert!(s.is_finished(), "the floor hasn't passed yet");

        s.poll(&mut store, 1_000 + DEFAULT_INTRODUCTION_COOLDOWN_MS);
        assert_eq!(
            Some(id),
            s.current().and_then(|c| c.id()),
            "the floor passed: delayed, not starved"
        );
    }

    #[test]
    fn recognize_queue_holds_only_due_cards() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        store.get_or_insert(&all[0].id().unwrap()).recognize = Some(mature_fsrs(2_000_000));
        let s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            1_000,
        );
        assert_eq!(1, s.remaining());
    }

    #[test]
    fn recognize_selection_is_immediately_available_when_established_elsewhere() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        store.get_or_insert(&id).recall = Some(mature_fsrs(42));
        let now = 1_000_000;
        let session = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            now,
        );

        let decision = session
            .selection_decision(session.current_idx.unwrap(), &store, now)
            .unwrap();
        assert_eq!(
            0, decision.due,
            "established at recall means available since always, not since now"
        );
    }

    #[test]
    fn recognize_caps_never_met_intake_at_the_session_cap() {
        let (mut store, _dir) = empty_store();
        // All 22 are never-met, so with no met pool they fill the whole cap.
        let s = Session::new(
            cards(22),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 10,
                depth: Depth::Recognize,
                ..Default::default()
            },
            1_000,
        );
        assert_eq!(10, s.initial_size);
    }

    #[test]
    fn recognize_caps_the_total_splitting_met_and_never_met() {
        let (mut store, _dir) = empty_store();
        // 15 met-and-due + 5 never-met. Cap 10 at 30%
        // new: 7 met + 3 never-met, and the met sweep finishes across sittings.
        let all = cards(20);
        for c in &all[..15] {
            store.get_or_insert(&c.id().unwrap()).recognize = Some(mature_fsrs(500));
        }
        let now = 1_000;
        let s = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                max_session: 10,
                new_cards_percent: 30,
                depth: Depth::Recognize,
                ..Default::default()
            },
            now,
        );
        assert_eq!(10, s.initial_size, "the total is capped, not uncapped");
        let never_met: Vec<String> = all[15..].iter().filter_map(|c| c.id()).collect();
        let picked_new = s
            .roster
            .iter()
            .filter(|&&i| never_met.contains(&all[i].id().unwrap()))
            .count();
        assert_eq!(3, picked_new, "3 of the 5 never-met, the rest are met");
        // Before any pick is drilled, the whole eligible backlog is still ahead.
        let (due_left, new_left) = s.remaining_split(&store, now);
        assert_eq!((15, 5), (due_left, new_left));
    }

    #[test]
    fn recognize_with_a_topology_serves_in_walk_order() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let topo = topology_order(&[&all[2], &all[1], &all[0]]);
        let mut s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                topology: Some(topo),
                ..Default::default()
            },
            1_000_000,
        );
        assert_eq!("front 2", s.current().unwrap().front);
        s.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 1", s.current().unwrap().front);
        s.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 0", s.current().unwrap().front);
    }

    #[test]
    fn recognize_without_a_topology_keeps_deck_order() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let mut s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            1_000_000,
        );
        assert_eq!("front 0", s.current().unwrap().front);
        s.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 1", s.current().unwrap().front);
        s.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 2", s.current().unwrap().front);
    }

    #[test]
    fn recognize_topology_chooses_the_capped_new_cards_in_walk_order() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        let topo = topology_order(&[&all[2], &all[1], &all[0]]);
        let s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                max_session: 2,
                depth: Depth::Recognize,
                topology: Some(topo),
                ..Default::default()
            },
            1_000,
        );
        assert_eq!(2, s.initial_size);
        // A review order selects the never-met cards in rank order: cards 2 and
        // 1 (the walk's first two), not deck-order 0 and 1.
        assert_eq!("front 2", s.current().unwrap().front);
    }

    #[test]
    fn recognize_limit_keeps_the_topologically_first_cards() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).introduced_ms = Some(0);
        }
        let topo = topology_order(&[&all[2], &all[1], &all[0]]);
        let mut s = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                max_session: 2,
                new_cards_percent: 0,
                depth: Depth::Recognize,
                topology: Some(topo),
                ..Default::default()
            },
            1_000_000,
        );
        assert_eq!(2, s.initial_size);
        assert_eq!("front 2", s.current().unwrap().front);
        s.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 1", s.current().unwrap().front);
    }

    fn mature_fsrs(due_ms: u64) -> FsrsState {
        FsrsState {
            stability: 30.0,
            difficulty: 5.0,
            scheduled_days: 30,
            state: 2,
            due_ms,
            ..Default::default()
        }
    }

    fn reconstruct_session(all: Vec<Card>, store: &mut Store, cram: bool, now: u64) -> Session {
        Session::new(
            all,
            store,
            sched(),
            SessionOptions {
                depth: Depth::Reconstruct,
                cram,
                ..Default::default()
            },
            now,
        )
    }

    #[test]
    fn a_full_reconstruct_pass_on_a_recall_due_card_credits_recall_marked() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        store.get_or_insert(&id).recall = Some(mature_fsrs(500));
        let now = 40 * 86_400_000;

        let mut s = reconstruct_session(all, &mut store, false, now);
        s.grade(&mut store, Grade::Pass, now);

        let st = store.get(&id).unwrap();
        let recall = st.recall.unwrap();
        assert!(recall.due_ms > now, "the due recall schedule advanced");
        assert!(recall.stability > 30.0, "full credit, not just a re-anchor");
        assert_eq!(2, st.history.len());
        assert_eq!(Depth::Reconstruct, st.history[0].depth);
        assert!(!st.history[0].propagated);
        assert_eq!(Depth::Recall, st.history[1].depth);
        assert_eq!(Grade::Pass, st.history[1].grade);
        assert!(st.history[1].propagated);
    }

    fn recognize_session(all: Vec<Card>, store: &mut Store, cram: bool, now: u64) -> Session {
        Session::new(
            all,
            store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                cram,
                ..Default::default()
            },
            now,
        )
    }

    #[test]
    fn a_recognize_pass_creates_a_schedule_and_a_second_pass_extends_it() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 1_000_000;

        let mut s = recognize_session(all.clone(), &mut store, false, now);
        s.grade(&mut store, Grade::Pass, now);
        let first = store.get(&id).unwrap().recognize;
        let first = first.expect("first pass creates the schedule");
        assert!(first.due_ms > now, "scheduled into the future");
        assert_eq!(1, store.get(&id).unwrap().history.len());
        assert_eq!(Depth::Recognize, store.get(&id).unwrap().history[0].depth);

        let later = first.due_ms + 1;
        let mut s2 = recognize_session(all, &mut store, false, later);
        s2.grade(&mut store, Grade::Pass, later);
        let second = store.get(&id).unwrap().recognize.unwrap();
        assert!(
            second.due_ms > first.due_ms,
            "second pass extends: {} then {}",
            first.due_ms,
            second.due_ms
        );
    }

    #[test]
    fn an_empty_recognize_reopen_reports_when_the_scheduled_card_returns() {
        // Codex tenth pass, P2: the flag-era next_due_at guard returned None
        // for Recognize wholesale; a scheduled depth owes its return time.
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 1_000_000;
        let mut s = recognize_session(all.clone(), &mut store, false, now);
        s.grade(&mut store, Grade::Pass, now);
        let due = store.get(&id).unwrap().recognize.unwrap().due_ms;
        assert!(due > now);

        let reopened = recognize_session(all, &mut store, false, now + 1);
        assert!(reopened.is_finished(), "nothing is due yet");
        assert_eq!(
            Some(due),
            reopened.next_due_at(&store),
            "the empty screen must say when the scheduled card returns"
        );
    }

    #[test]
    fn recognize_counts_in_reviews_passed_and_failed() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let now = 1_000_000;

        let mut s = recognize_session(all, &mut store, false, now);
        s.grade(&mut store, Grade::Pass, now);
        s.grade(&mut store, Grade::Fail, now);

        assert_eq!(2, s.stats.reviews, "both recognize grades are reviews");
        assert_eq!(1, s.stats.passed);
        assert_eq!(1, s.stats.failed);
    }

    #[test]
    fn a_partial_at_any_depth_lands_in_the_generic_partial_counter() {
        let now = 1_000_000;
        for depth in [Depth::Recognize, Depth::Recall, Depth::Reconstruct] {
            let (mut store, _dir) = empty_store();
            let all = cards(1);
            let mut s = Session::new(
                all,
                &mut store,
                sched(),
                SessionOptions {
                    depth,
                    ..Default::default()
                },
                now,
            );
            s.grade(&mut store, Grade::Partial, now);
            assert_eq!(
                1, s.stats.partial,
                "a {depth:?} partial increments the shared counter"
            );
        }
    }

    #[test]
    fn a_reconstruct_pass_with_no_recall_schedule_credits_an_existing_recognize_schedule() {
        // ADR 0033 clause 4's discriminator: every SHALLOWER depth is a
        // target, so the missing Recall schedule does not break a chain.
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 40 * 86_400_000;
        let state = store.get_or_insert(&id);
        state.recognize = Some(mature_fsrs(500));

        let mut s = reconstruct_session(all, &mut store, false, now);
        s.grade(&mut store, Grade::Pass, now);

        let st = store.get(&id).unwrap();
        assert!(st.recall.is_none(), "no Recall schedule is created");
        assert!(
            st.recognize.unwrap().stability > 30.0,
            "the due recognize schedule took the propagated credit"
        );
        assert!(
            st.history
                .iter()
                .any(|r| r.depth == Depth::Recognize && r.propagated),
            "the propagated recognize review is recorded"
        );
    }

    #[test]
    fn a_recall_pass_credits_a_due_recognize_and_reanchors_a_not_due_one() {
        let now = 40 * 86_400_000;

        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let state = store.get_or_insert(&id);
        state.recall = Some(mature_fsrs(500));
        state.recognize = Some(mature_fsrs(500));
        let mut s = recall_session(all, &mut store, now);
        s.grade(&mut store, Grade::Pass, now);
        let st = store.get(&id).unwrap();
        assert!(
            st.recognize.unwrap().stability > 30.0,
            "due recognize takes propagated credit from a recall pass"
        );

        let (mut store2, _dir2) = empty_store();
        let all2 = cards(1);
        let id2 = all2[0].id().unwrap();
        let future = 80 * 86_400_000;
        let state2 = store2.get_or_insert(&id2);
        state2.recall = Some(mature_fsrs(500));
        state2.recognize = Some(mature_fsrs(future));
        let mut s2 = recall_session(all2, &mut store2, now);
        s2.grade(&mut store2, Grade::Pass, now);
        let st2 = store2.get(&id2).unwrap();
        let recog = st2.recognize.unwrap();
        assert_eq!(
            30.0, recog.stability,
            "not-due recognize is only re-anchored"
        );
        assert_eq!(
            now + 30 * 86_400_000,
            recog.due_ms,
            "due re-derived from now"
        );
    }

    #[test]
    fn no_propagation_creates_a_missing_recognize_schedule() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 1_000_000;
        store.get_or_insert(&id).recall = Some(mature_fsrs(500));

        let mut s = recall_session(all, &mut store, now);
        s.grade(&mut store, Grade::Pass, now);

        let st = store.get(&id).unwrap();
        assert!(
            st.recognize.is_none(),
            "propagation never creates a schedule at any depth"
        );
    }

    fn recall_session(all: Vec<Card>, store: &mut Store, now: u64) -> Session {
        Session::new(
            all,
            store,
            sched(),
            SessionOptions {
                depth: Depth::Recall,
                ..Default::default()
            },
            now,
        )
    }

    #[test]
    fn a_reconstruct_pass_on_a_not_yet_due_recall_reanchors_without_reward() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 1_000_000;
        store.get_or_insert(&id).recall = Some(mature_fsrs(2_000_000));

        let mut s = reconstruct_session(all, &mut store, false, now);
        s.grade(&mut store, Grade::Pass, now);

        let st = store.get(&id).unwrap();
        let recall = st.recall.unwrap();
        assert_eq!(30.0, recall.stability, "memory untouched — no reward");
        assert_eq!(30, recall.scheduled_days, "interval kept");
        assert_eq!(
            now + 30 * 86_400_000,
            recall.due_ms,
            "due re-derived from now"
        );
        assert!(recall.due_ms > 2_000_000, "strictly later than before");
        assert_eq!(1, st.history.len());
        assert_eq!(Depth::Reconstruct, st.history[0].depth);
    }

    #[test]
    fn no_propagation_without_a_recall_schedule() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 1_000_000;
        store.get_or_insert(&id).reconstruct = Some(mature_fsrs(500));

        let mut s = reconstruct_session(all, &mut store, false, now);
        s.grade(&mut store, Grade::Pass, now);

        let st = store.get(&id).unwrap();
        assert!(st.recall.is_none(), "propagation never creates a schedule");
        assert_eq!(1, st.history.len());
        assert_eq!(Depth::Reconstruct, st.history[0].depth);
    }

    #[test]
    fn partials_and_fails_never_propagate() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let now = 1_000_000;
        for c in &all {
            store.get_or_insert(&c.id().unwrap()).recall = Some(mature_fsrs(500));
        }

        let mut s = reconstruct_session(all.clone(), &mut store, false, now);
        s.grade(&mut store, Grade::Partial, now);
        s.grade(&mut store, Grade::Fail, now);

        for c in &all {
            let st = store.get(&c.id().unwrap()).unwrap();
            assert_eq!(
                mature_fsrs(500),
                st.recall.unwrap(),
                "recall untouched by a partial or a fail"
            );
            assert!(st.recognize.is_none(), "recognize untouched");
            assert!(st.history.iter().all(|r| !r.propagated));
        }
    }

    #[test]
    fn a_due_reconstruct_cram_pass_credits_recall_like_a_normal_review() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 40 * 86_400_000;
        let state = store.get_or_insert(&id);
        state.recall = Some(mature_fsrs(500));
        state.reconstruct = Some(mature_fsrs(500));

        let mut s = reconstruct_session(all, &mut store, true, now);
        s.grade(&mut store, Grade::Pass, now);

        let st = store.get(&id).unwrap();
        assert!(
            st.reconstruct.unwrap().stability > 30.0,
            "the due reconstruct pass took full credit"
        );
        assert!(
            st.recall.unwrap().stability > 30.0,
            "the due recall schedule took the propagated credit"
        );
        assert_eq!(2, st.history.len());
        assert!(st.history[1].propagated);
        assert!(
            st.recognize.is_none(),
            "no recognize schedule existed, so propagation creates none"
        );
    }

    #[test]
    fn an_early_reconstruct_cram_pass_propagates_nothing() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 10 * 86_400_000;
        let future = 40 * 86_400_000;
        let state = store.get_or_insert(&id);
        state.recall = Some(mature_fsrs(future));
        state.reconstruct = Some(mature_fsrs(future));

        let mut s = reconstruct_session(all, &mut store, true, now);
        s.grade(&mut store, Grade::Pass, now);

        let st = store.get(&id).unwrap();
        assert_eq!(
            mature_fsrs(future),
            st.recall.unwrap(),
            "no recall credit, not even a re-anchor"
        );
        let reconstruct = st.reconstruct.unwrap();
        assert_eq!(30.0, reconstruct.stability, "an early pass never rewards");
        assert_eq!(now + 30 * 86_400_000, reconstruct.due_ms, "re-anchored");
        assert!(st.history.is_empty(), "an early cram pass is not a review");
        assert!(
            st.recognize.is_none(),
            "an early cram pass propagates nothing"
        );
    }

    #[test]
    fn recognize_cram_serves_already_recognized_cards() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let now = 1_000_000;
        for card in &all {
            store.get_or_insert(&card.id().unwrap()).recognize = Some(mature_fsrs(2_000_000));
        }

        let normal = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            now,
        );
        assert!(normal.is_finished(), "nothing left to recognize");

        let cram = Session::new(
            all.clone(),
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                cram: true,
                ..Default::default()
            },
            now,
        );
        assert_eq!(2, cram.initial_size, "cram serves every card");
    }

    #[test]
    fn any_full_pass_credits_an_existing_recognize_schedule() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        let now = 40 * 86_400_000;
        for card in &all {
            store.get_or_insert(&card.id().unwrap()).recognize = Some(mature_fsrs(500));
        }

        let mut recall = Session::new(
            vec![all[0].clone()],
            &mut store,
            sched(),
            SessionOptions::default(),
            now,
        );
        recall.grade(&mut store, Grade::Pass, now);
        assert!(
            store
                .get(&all[0].id().unwrap())
                .unwrap()
                .recognize
                .unwrap()
                .stability
                > 30.0,
            "a recall pass credits the due recognize schedule"
        );

        let mut reconstruct = reconstruct_session(vec![all[1].clone()], &mut store, false, now);
        reconstruct.grade(&mut store, Grade::Pass, now);
        assert!(
            store
                .get(&all[1].id().unwrap())
                .unwrap()
                .recognize
                .unwrap()
                .stability
                > 30.0,
            "a reconstruct pass credits the due recognize schedule"
        );

        let mut partial = Session::new(
            vec![all[2].clone()],
            &mut store,
            sched(),
            SessionOptions::default(),
            now,
        );
        partial.grade(&mut store, Grade::Partial, now);
        assert_eq!(
            mature_fsrs(500),
            store.get(&all[2].id().unwrap()).unwrap().recognize.unwrap(),
            "a partial never propagates"
        );
    }

    #[test]
    fn a_recognize_session_counts_in_the_generic_review_tallies() {
        let (mut store, _dir) = empty_store();
        let now = DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;
        let all = cards(3);
        for card in &all {
            store.get_or_insert(&card.id().unwrap()).introduced_ms = Some(0);
        }
        let mut session = Session::new(
            all,
            &mut store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            now,
        );
        session.grade(&mut store, Grade::Pass, now);
        session.grade(&mut store, Grade::Partial, now);
        session.grade(&mut store, Grade::Fail, now);

        assert_eq!(
            3, session.stats.reviews,
            "every recognize grade is a review"
        );
        assert_eq!(2, session.stats.passed, "pass and partial both pass");
        assert_eq!(1, session.stats.failed);
        assert_eq!(
            1, session.stats.partial,
            "the almost lands in the generic counter"
        );
    }

    #[test]
    fn the_session_reports_its_configured_retirement_cap() {
        let (mut store, _dir) = empty_store();
        let session = Session::new(
            cards(1),
            &mut store,
            sched(),
            SessionOptions {
                retire_after_days: Some(7),
                ..Default::default()
            },
            0,
        );
        assert_eq!(Some(7), session.retire_after_days());
    }

    #[test]
    fn leftover_slots_flow_back_to_due_cards_when_new_runs_short() {
        assert_eq!((9, 1), split_slots(20, 1, 10, 30));
    }

    #[test]
    fn due_soon_counts_strictly_future_dues_inside_the_window_only() {
        let (mut store, _dir) = empty_store();
        let sched = sched();
        let now = 1_000;
        let window = 100;
        let scheduled = |due_ms: u64| crate::store::FsrsState {
            state: 2,
            stability: 1.0,
            due_ms,
            ..Default::default()
        };
        let mut personal = Vec::new();
        for (back, due) in [
            ("at now", 1_000),
            ("in window", 1_001),
            ("at edge", 1_100),
            ("past edge", 1_101),
            ("overdue", 999),
        ] {
            let card = personal_card(&mut store, "d.md", back, 0);
            store.get_or_insert(&card.id().unwrap()).recall = Some(scheduled(due));
            personal.push(card);
        }
        let retired = personal_card(&mut store, "d.md", "retired but in window", 0);
        let mut retired_state = retired_fsrs();
        retired_state.state = 2;
        retired_state.stability = 1.0;
        retired_state.due_ms = 1_050;
        store.get_or_insert(&retired.id().unwrap()).recall = Some(retired_state);

        personal.push(retired);
        let count = count_due_soon(
            &personal,
            &store,
            sched.as_ref(),
            Depth::Recall,
            now,
            window,
            Some(DEFAULT_RETIRE_AFTER_DAYS),
        );
        assert_eq!(2, count, "exactly `in window` and `at edge`");
    }

    #[test]
    fn no_cards_means_nothing_is_reviewable() {
        let (store, _dir) = empty_store();
        let sched = sched();
        assert_eq!(
            0,
            count_reviewable(&[], &store, sched.as_ref(), Depth::Recall, 1_000, None)
        );
    }
}

#[cfg(all(test, feature = "full"))]
mod clap_parity {
    use clap::ValueEnum;

    use super::*;

    #[test]
    fn parse_matches_the_clap_value_names() {
        for variant in Order::value_variants() {
            let name = variant.to_possible_value().expect("a value name");
            assert_eq!(Some(*variant), Order::parse(name.get_name()), "{name:?}");
        }
        assert_eq!(None, Order::parse("no-such-value"));
    }
}
