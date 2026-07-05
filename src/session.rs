//! Review session logic, independent of any UI.
//!
//! A session takes the cards of one or more decks, asks the store which are due,
//! and serves them **by FSRS due**: each card is graded at most once per
//! appearance. A miss is *not* re-drilled immediately — the card keeps the short
//! due FSRS gave it and re-appears only once that step has elapsed, interleaved
//! behind other due cards (so every scored review is genuinely time-separated). A
//! pass (or acquire) leaves the session. When nothing is due right now the
//! session is finished-for-now; [`Session::poll`] lets a frontend re-enter it
//! when a cooling card comes back. A never-seen card is *acquired* first — shown,
//! recorded at stage 1, then left to settle ~1 min before its first quiz.

use std::collections::VecDeque;

use crate::{
    augment::TopologyOrder,
    card::Card,
    scheduler::{Grade, Scheduler},
    store::{CardState, MAX_STAGE, Store, VirtualCard},
    time,
};

/// The order in which the due/new cards of a session are presented.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Order {
    /// The scheduler decides the order (FSRS: earliest due first), then up to
    /// `max_new` new cards.
    #[default]
    Scheduled,
    /// Present the cards in deck/file order, top to bottom — useful for
    /// memorizing something with an inherent sequence, like song lyrics.
    Sequential,
}

/// Options controlling which cards enter a session and in what order.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    /// Maximum number of never-seen cards to introduce.
    pub max_new: usize,
    /// Maximum number of cards in the queue (due cards take priority).
    pub limit: Option<usize>,
    /// Ignore due times and review everything (new cards still capped by
    /// `max_new`).
    pub cram: bool,
    /// How the queued cards are ordered.
    pub order: Order,
    /// Reorder the due/new set by this AI topology walk. `None` keeps the
    /// scheduler's order — only the *sort* changes, never which cards are due.
    pub topology: Option<TopologyOrder>,
    /// A card retires once its FSRS interval reaches this many days; `None`
    /// disables retirement. From `[review] retire_after` (per-workspace overridable).
    pub retire_after_days: Option<u32>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            max_new: 10,
            limit: None,
            cram: false,
            order: Order::Scheduled,
            topology: None,
            retire_after_days: Some(DEFAULT_RETIRE_AFTER_DAYS),
        }
    }
}

/// Counters for a running session.
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionStats {
    /// Number of grades given (a card re-served after its step counts again).
    pub reviews: usize,
    /// Number of passed reviews.
    pub passed: usize,
    /// Number of failed reviews.
    pub failed: usize,
    /// Number of never-seen cards introduced (acquired) this session.
    pub acquired: usize,
}

/// Per-stage card counts for the loaded decks; index 0 holds unseen cards.
pub type StageHistogram = [usize; 6];

/// A review session over a fixed set of cards.
pub struct Session {
    cards: Vec<Card>,
    /// Aligned 1:1 with `cards`. `Some(v_id)` marks a virtual card whose
    /// schedule lives in the store's `v:`-namespaced entry (`v_id`), not in
    /// `store.cards` keyed by its own `u64` id. See
    /// [`state_of`](Self::state_of).
    virtual_ids: Vec<Option<String>>,
    /// In-play card indices in session order (due cards, then new). A card
    /// leaves on pass/acquire/remove; a miss keeps it — its short FSRS due gates
    /// when it becomes servable again.
    roster: Vec<usize>,
    /// The card currently up for review — pinned once served, so a card that
    /// cools back into due-ness mid-answer can't be graded by mistake. `None`
    /// means nothing is servable right now (the session is finished-for-now).
    current_idx: Option<usize>,
    /// Roster cards servable at the last `advance` — the "remaining now" count.
    remaining_now: usize,
    scheduler: Box<dyn Scheduler>,
    options: SessionOptions,
    /// Total distinct cards that entered the roster initially.
    pub initial_size: usize,
    /// Session counters.
    pub stats: SessionStats,
}

impl Session {
    /// Builds a session at time `now_ms`.
    ///
    /// The roster holds, in order: all due cards (earliest FSRS due first), then
    /// up to `max_new` unseen cards in deck order. Sub-cards of the same cloze
    /// card are kept apart whenever other cards are available.
    pub fn new(
        cards: Vec<Card>,
        store: &Store,
        scheduler: Box<dyn Scheduler>,
        options: SessionOptions,
        now_ms: u64,
    ) -> Self {
        let virtual_ids = vec![None; cards.len()];
        Self::build(cards, virtual_ids, store, scheduler, options, now_ms)
    }

    /// Builds a session whose roster may include virtual cards alongside a
    /// deck's authored ones: `virtual_ids[i]` marks card `i`'s schedule as
    /// living in the store's `v:`-namespaced entry rather than its own `u64`
    /// id (see [`state_of`](Self::state_of)). `build_review` uses this to
    /// inject a deck's due (remediation) virtual cards into the same queue.
    pub fn new_with_virtual(
        cards: Vec<Card>,
        virtual_ids: Vec<Option<String>>,
        store: &Store,
        scheduler: Box<dyn Scheduler>,
        options: SessionOptions,
        now_ms: u64,
    ) -> Self {
        Self::build(cards, virtual_ids, store, scheduler, options, now_ms)
    }

    /// The shared constructor body: builds the roster (from both keyspaces)
    /// and points the cursor at the first servable card.
    fn build(
        cards: Vec<Card>,
        virtual_ids: Vec<Option<String>>,
        store: &Store,
        scheduler: Box<dyn Scheduler>,
        options: SessionOptions,
        now_ms: u64,
    ) -> Self {
        let roster: Vec<usize> =
            build_queue(&cards, &virtual_ids, store, &*scheduler, &options, now_ms).into();
        let initial_size = roster.len();

        let mut session = Self {
            cards,
            virtual_ids,
            roster,
            current_idx: None,
            remaining_now: 0,
            scheduler,
            options,
            initial_size,
            stats: SessionStats::default(),
        };
        session.advance(store, now_ms);
        session
    }

    /// Starts a fresh session over the same decks with the same settings,
    /// picking up whatever is due (or new) at `now_ms`.
    ///
    /// Returns `false` — leaving the roster and stats untouched — if nothing is
    /// due, so a summary screen can keep showing the finished session.
    pub fn restart(&mut self, store: &Store, now_ms: u64) -> bool {
        let roster: Vec<usize> = build_queue(
            &self.cards,
            &self.virtual_ids,
            store,
            &*self.scheduler,
            &self.options,
            now_ms,
        )
        .into();
        if roster.is_empty() {
            return false;
        }
        self.initial_size = roster.len();
        self.roster = roster;
        self.stats = SessionStats::default();
        self.advance(store, now_ms);
        true
    }

    /// Whether a [`restart`](Self::restart) right now would find any cards —
    /// i.e. anything is due (or a new card can be introduced) at `now_ms`.
    /// Non-mutating; runs the same queue build `restart` would.
    pub fn has_due_now(&self, store: &Store, now_ms: u64) -> bool {
        !build_queue(
            &self.cards,
            &self.virtual_ids,
            store,
            &*self.scheduler,
            &self.options,
            now_ms,
        )
        .is_empty()
    }

    /// The earliest upcoming due time over all seen cards of this session's
    /// decks (deck cards and virtual cards alike), if any.
    pub fn next_due_at(&self, store: &Store) -> Option<u64> {
        (0..self.cards.len())
            .filter_map(|i| self.state_of(store, i))
            .map(|state| self.scheduler.due_at(state))
            .min()
    }

    /// The card currently up for review — the pinned cursor set by [`advance`].
    pub fn current(&self) -> Option<&Card> {
        self.current_idx.map(|i| &self.cards[i])
    }

    /// The current card, mutable — e.g. to attach a note just saved from the ask
    /// tutor so the card shows it without re-reading the deck file.
    pub fn current_mut(&mut self) -> Option<&mut Card> {
        let i = self.current_idx?;
        Some(&mut self.cards[i])
    }

    /// The store's `v:`-namespaced id of the current card, if it is a virtual
    /// one (`None` for an authored deck card, or when nothing is current). Lets
    /// a frontend offer to promote the card being reviewed right now.
    pub fn current_virtual_id(&self) -> Option<&str> {
        let i = self.current_idx?;
        self.virtual_ids[i].as_deref()
    }

    /// Whether the current card has never been seen (no stored progress). Such a
    /// card is *acquired* — shown via [`acquire_current`](Self::acquire_current) —
    /// rather than quizzed cold. A virtual card's entry always exists (it is
    /// created already-scheduled), so this is never true for one — it skips
    /// the attempt-first acquire screen entirely.
    pub fn current_unseen(&self, store: &Store) -> bool {
        self.current_idx
            .is_some_and(|i| self.state_of(store, i).is_none())
    }

    /// All cards of this session's decks (e.g. as the distractor pool for
    /// multiple-choice questions).
    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    /// Cards servable right now (the current one included) — reaches 0 exactly
    /// when the session is finished-for-now. A missed card cooling back in nudges
    /// this up by one when it returns.
    pub fn remaining(&self) -> usize {
        self.remaining_now
    }

    /// `true` when nothing is due right now — the session is finished-for-now.
    /// Cards still cooling (missed, not yet re-due) are picked up next session,
    /// or re-enter this one via [`poll`](Self::poll) once their step elapses.
    pub fn is_finished(&self) -> bool {
        self.current_idx.is_none()
    }

    /// Grades the current card, updates the store, and advances the cursor.
    /// A pass (or acquire, or any cram grade) leaves the session; a normal miss
    /// keeps the card in the roster, and it re-appears only once its short FSRS
    /// due has elapsed — never re-drilled immediately in the same sitting.
    pub fn grade(&mut self, store: &mut Store, grade: Grade, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        match self.virtual_ids[index].as_deref() {
            Some(vid) => {
                let Some(vc) = store.get_virtual_mut(vid) else {
                    // A dangling reference: nothing to grade. Drop it and move
                    // on rather than fabricate a `u64` entry for it.
                    self.roster.retain(|&i| i != index);
                    self.advance(store, now_ms);
                    return;
                };
                // Same cram-refresh-vs-real-review split as a deck card (see
                // below), applied to the virtual entry's own state. No stage
                // clamp: a virtual card's `stage` is a frozen legacy marker
                // seeded once by `CardState::new`, not live scheduling state,
                // so there is nothing for the scheduler to push past
                // `MAX_STAGE`.
                if self.options.cram && grade.passed() {
                    self.scheduler.reanchor(&mut vc.state, now_ms);
                } else {
                    self.scheduler.apply(&mut vc.state, grade, now_ms);
                }
                // Archive once this review pushes the interval to/past the
                // cap. Retire at the cap only — never elsewhere (in
                // particular, never on an exam re-pass; that path must not
                // touch `retired`).
                if virtual_retired(vc, self.options.retire_after_days) {
                    vc.retired = true;
                }
            }
            None => {
                let card = &self.cards[index];
                let state = store.get_or_insert(card.id(), now_ms);
                // Cram refresh: a correct answer keeps the card fresh without rewarding it
                // (re-anchor its due — no FSRS update, no recorded review). A cram miss is a
                // genuine lapse, and every normal grade runs the real scheduler.
                if self.options.cram && grade.passed() {
                    self.scheduler.reanchor(state, now_ms);
                } else {
                    self.scheduler.apply(state, grade, now_ms);
                }
                // Safety net: keep the stage within the top (reaching `MAX_STAGE`
                // retires the card). The scheduler already caps a pass at `MAX_STAGE`.
                if state.stage > MAX_STAGE {
                    state.stage = MAX_STAGE;
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
        // A pass leaves the session; a cram card is single-pass so it also leaves;
        // a normal miss stays in the roster and re-appears only once its FSRS due
        // (its short learning/relearning step) has elapsed.
        if passed || self.options.cram {
            self.roster.retain(|&i| i != index);
        }
        self.advance(store, now_ms);
    }

    /// Introduces the current never-seen card: records it at stage 1 and moves on.
    /// It is *not* graded and gets *no* history entry — acquiring is a first
    /// exposure, not a review. The card is **kept** in the session, cooling on its
    /// ~1 min stage-1 gap, so its first real quiz surfaces again later in *this
    /// same session* once that gap passes. Does nothing when nothing is up.
    pub fn acquire_current(&mut self, store: &mut Store, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        // Defensive: a virtual card's entry always exists, so `current_unseen`
        // is never true for one and this branch is unreachable in practice —
        // but skip cleanly (no `u64` ghost entry) rather than assume a deck
        // card is at this index.
        if self.virtual_ids[index].is_none() {
            // `get_or_insert` creates the state at stage 1, due ~1 min out via the
            // stage-1 cooldown — no `scheduler.apply`, no recorded review. The card
            // stays in the roster so the due-driven serving surfaces it again for its
            // first real quiz once the gap elapses, in this same session.
            store.get_or_insert(self.cards[index].id(), now_ms);
            self.stats.acquired += 1;
        }
        self.advance(store, now_ms);
    }

    /// Defers the current card without grading it: another servable card is
    /// offered first, and the skipped card returns after the rest.
    pub fn skip(&mut self, store: &Store, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        self.roster.retain(|&i| i != index);
        self.roster.push(index);
        self.advance(store, now_ms);
    }

    /// Drops the current card from the queue without grading it, along with any
    /// remaining cards in the same sibling group (cloze sub-cards of one source
    /// card) so a card marked for removal is not asked again in any form.
    /// Returns clones of every dropped card (the current one first), or an
    /// empty vec if the queue was empty. The store is left untouched;
    /// pruning the cards' progress is the caller's job once the deck file
    /// is rewritten.
    pub fn remove_current(&mut self, store: &Store, now_ms: u64) -> Vec<Card> {
        let Some(index) = self.current_idx else {
            return Vec::new();
        };
        let group = sibling_group(&self.cards[index]);
        let mut removed = vec![self.cards[index].clone()];
        let mut kept: Vec<usize> = Vec::with_capacity(self.roster.len());
        for &i in &self.roster {
            if i == index {
                continue; // the current card is already in `removed`, and dropped
            }
            if sibling_group(&self.cards[i]) == group {
                removed.push(self.cards[i].clone());
            } else {
                kept.push(i);
            }
        }
        self.roster = kept;
        self.advance(store, now_ms);
        removed
    }

    /// Re-checks due times without rebuilding the roster or resetting stats: a
    /// missed card cooling back into due-ness becomes current again. The
    /// frontends call this while idle at the summary so a review re-enters on its
    /// own (unlike [`restart`](Self::restart), which starts a fresh sitting).
    /// Returns whether a card is now up.
    pub fn poll(&mut self, store: &Store, now_ms: u64) -> bool {
        self.advance(store, now_ms);
        self.current_idx.is_some()
    }

    /// The stored schedule/history state for roster card `i`: the virtual
    /// entry's state when `i` is virtual, else the deck-card state keyed by
    /// its `u64` id. `None` for an unseen deck card (a virtual card always
    /// has state — see [`current_unseen`](Self::current_unseen)). This is
    /// the one place a roster index is turned into stored state; every
    /// reader routes through it (or [`slot_retired`](Self::slot_retired))
    /// rather than calling `store.get(card.id())` directly.
    fn state_of<'s>(&self, store: &'s Store, i: usize) -> Option<&'s CardState> {
        match self.virtual_ids[i].as_deref() {
            Some(vid) => store.get_virtual(vid).map(|v| &v.state),
            None => store.get(self.cards[i].id()),
        }
    }

    /// Whether roster card `i` is retired/archived (excluded from scheduling):
    /// for a virtual card, its stored archive flag or its interval past the
    /// cap ([`virtual_retired`]); for a deck card, the derived interval-cap
    /// rule ([`is_retired`]).
    fn slot_retired(&self, store: &Store, i: usize) -> bool {
        let cap = self.options.retire_after_days;
        match self.virtual_ids[i].as_deref() {
            Some(vid) => store.get_virtual(vid).is_some_and(|v| virtual_retired(v, cap)),
            None => is_retired(&self.cards[i], store, cap),
        }
    }

    /// Whether roster card `i` can be served right now: a retired/archived
    /// slot never is (cram included — only `alix reset`, or reviving a
    /// virtual card, brings it back); otherwise under cram, always; otherwise
    /// unseen (a deck card) or FSRS-due. A virtual card always has state, so
    /// a missing entry (a dangling reference) is never servable.
    fn servable(&self, i: usize, store: &Store, now_ms: u64) -> bool {
        if self.slot_retired(store, i) {
            return false;
        }
        match self.state_of(store, i) {
            Some(state) => self.options.cram || self.scheduler.is_due(state, now_ms),
            None => self.virtual_ids[i].is_none(),
        }
    }

    /// Re-points the cursor to the first servable roster card (session order) and
    /// refreshes the servable-now count. Called after every transition.
    fn advance(&mut self, store: &Store, now_ms: u64) {
        self.current_idx = self
            .roster
            .iter()
            .copied()
            .find(|&i| self.servable(i, store, now_ms));
        self.remaining_now = self
            .roster
            .iter()
            .copied()
            .filter(|&i| self.servable(i, store, now_ms))
            .count();
    }

    /// Per-stage counts over all cards of this session's decks (stage 0 =
    /// never seen). Deck composition only — a virtual card has no deck stage
    /// and would only inflate the "new" bucket, so it is excluded here.
    pub fn stage_histogram(&self, store: &Store) -> StageHistogram {
        let deck_cards: Vec<Card> = self
            .cards
            .iter()
            .zip(&self.virtual_ids)
            .filter(|(_, vid)| vid.is_none())
            .map(|(card, _)| card.clone())
            .collect();
        histogram(&deck_cards, store)
    }

    /// The session's top Leitner stage — always [`MAX_STAGE`] now that decks no
    /// longer cap below it. Kept as the single source for the stage bar's height
    /// (terminal and the web DTO).
    pub fn top_stage(&self) -> u8 {
        MAX_STAGE
    }
}

/// Builds the review queue: due cards in scheduler order, then up to
/// `max_new` unseen cards, capped by `limit`, with cloze siblings separated.
/// `virtual_ids[i]` routes card `i`'s state to the store's `v:`-namespaced
/// entry instead of its own `u64` id — a free-fn mirror of
/// [`Session::state_of`]/[`Session::slot_retired`], needed because this runs
/// before a `Session` exists to call those methods on.
fn build_queue(
    cards: &[Card],
    virtual_ids: &[Option<String>],
    store: &Store,
    scheduler: &dyn Scheduler,
    options: &SessionOptions,
    now_ms: u64,
) -> VecDeque<usize> {
    let mut due: Vec<usize> = Vec::new();
    let mut fresh: Vec<usize> = Vec::new();

    for (i, card) in cards.iter().enumerate() {
        match virtual_ids[i].as_deref() {
            Some(vid) => {
                // A virtual card always has state (no "unseen" acquire path)
                // and, once archived, rests until revived — never scheduled,
                // not even under cram. A dangling reference is silently
                // skipped (nothing to schedule).
                if let Some(vc) = store.get_virtual(vid)
                    && !virtual_retired(vc, options.retire_after_days)
                    && (options.cram || scheduler.is_due(&vc.state, now_ms))
                {
                    due.push(i);
                }
            }
            None => match store.get(card.id()) {
                // A retired card rests until `alix reset` — never scheduled, not
                // even under cram.
                Some(_) if is_retired(card, store, options.retire_after_days) => {}
                Some(state) => {
                    if options.cram || scheduler.is_due(state, now_ms) {
                        due.push(i);
                    }
                }
                None => fresh.push(i),
            },
        }
    }

    // Order due cards by their FSRS due time, earliest first.
    due.sort_by_key(|&i| {
        let state = match virtual_ids[i].as_deref() {
            Some(vid) => store.get_virtual(vid).map(|v| &v.state),
            None => store.get(cards[i].id()),
        };
        state.map_or(u64::MAX, |s| scheduler.due_at(s))
    });

    let mut fresh: Vec<usize> = fresh.into_iter().take(options.max_new).collect();

    // A topology reorders the already-selected cards by the AI walk; the
    // scheduler still chose *which* cards are here. Sorting the due and new sets
    // separately keeps due cards ahead of new ones, so a session `limit` still
    // favors what's due. The stable sort leaves cards absent from the walk (rank
    // `None` → `usize::MAX`) in their existing scheduler order within each group.
    if let Some(topo) = &options.topology {
        let rank = |&i: &usize| topo.rank_of(cards[i].id()).unwrap_or(usize::MAX);
        due.sort_by_key(rank);
        fresh.sort_by_key(rank);
    }

    let mut order: Vec<usize> = due;
    order.extend(fresh);

    if options.order == Order::Sequential {
        // Card indices follow deck/file order, so sorting restores it while
        // keeping the due/new selection above. An explicit Sequential override
        // runs last, so it wins over a topology if both are somehow set.
        order.sort_unstable();
    }
    if let Some(limit) = options.limit {
        order.truncate(limit);
    }

    // A topology is a deliberate ordering, so don't let sibling-separation
    // reshuffle it: to break two adjacent cloze holes apart it would pull a card
    // from another region between them, and that shows up as the orientation
    // breadcrumb jumping out of a region and back. Cloze holes that land
    // adjacent under a topology are the deferred cloze-as-one-node case.
    if options.topology.is_some() {
        order.into()
    } else {
        separate_siblings(order, cards)
    }
}

/// The sibling group of a card. Sub-cards of one cloze card share their
/// deck file and front line number; plain cards have unique lines.
fn sibling_group(card: &Card) -> (&str, usize) {
    (card.subject.as_ref(), card.line)
}

/// Reorders the queue so cards of the same sibling group (cloze sub-cards of
/// one source card) are not adjacent whenever other cards are available.
/// Apart from that the given order is preserved: each slot takes the first
/// remaining card that doesn't repeat the previous group.
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
            // Only siblings of the previous card are left; adjacency is
            // unavoidable.
            .unwrap_or(0);
        let index = remaining.remove(pos).unwrap();
        last = Some(index);
        queue.push_back(index);
    }
    queue
}

/// Retirement cap: an FSRS card retires once its scheduled interval reaches this many
/// days (a very stable card rests until `alix reset`). Default for now — per-user
/// `retire_after` config is a follow-up.
/// The default retirement cap (1 year), used when a [`SessionOptions`] or config is
/// built without an explicit one. The effective cap comes from `[review]
/// retire_after` (see [`crate::config::ReviewConfig`]).
pub const DEFAULT_RETIRE_AFTER_DAYS: u32 = 365;

/// Whether a card is *retired* (resting), so it is no longer scheduled until
/// `alix reset`: its FSRS interval has reached `retire_after_days`. `None` disables
/// retirement (drill forever). A card with no FSRS state yet — unseen, or a legacy
/// card not yet reviewed under FSRS — is never retired; its first FSRS review is what
/// can push the interval past the cap.
pub fn is_retired(card: &Card, store: &Store, retire_after_days: Option<u32>) -> bool {
    is_retired_id(card.id(), store, retire_after_days)
}

/// Id-only variant of [`is_retired`], so callers that hold an id but not the
/// [`Card`] (e.g. a trace checkpoint) share the one retirement rule.
pub fn is_retired_id(card_id: u64, store: &Store, retire_after_days: Option<u32>) -> bool {
    let Some(cap) = retire_after_days else {
        return false;
    };
    store
        .get(card_id)
        .and_then(|s| s.fsrs.as_ref())
        .is_some_and(|f| f.scheduled_days >= cap)
}

/// Whether a virtual card is retired/archived: its stored archive flag, or its
/// FSRS interval has reached the cap. `None` cap disables interval retirement.
/// Mirrors [`is_retired`]'s interval rule; unlike a deck card's purely-derived
/// retirement, a virtual card also carries a persisted flag (set at grade
/// time — see `Session::grade`), because an archived entry must survive to be
/// revived later.
pub fn virtual_retired(vc: &VirtualCard, retire_after_days: Option<u32>) -> bool {
    vc.retired
        || retire_after_days.is_some_and(|cap| {
            vc.state
                .fsrs
                .as_ref()
                .is_some_and(|f| f.scheduled_days >= cap)
        })
}

/// Whether a virtual card would be served now: not archived, and FSRS-due. The
/// virtual-card counterpart of [`is_reviewable`] (a virtual card is never
/// "new" — it always has state, so there is no fresh/unseen branch).
pub fn is_virtual_reviewable(
    vc: &VirtualCard,
    scheduler: &dyn Scheduler,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> bool {
    !virtual_retired(vc, retire_after_days) && scheduler.is_due(&vc.state, now_ms)
}

/// Whether `subject`'s deck has any virtual (remediation) card due right now —
/// the virtual-card counterpart of [`has_reviewable`], added to a deck's own
/// due signal (never to its size/card count). See [`is_virtual_reviewable`].
pub fn has_reviewable_virtual(
    store: &Store,
    subject: &str,
    scheduler: &dyn Scheduler,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> bool {
    store
        .virtual_cards_for(subject)
        .into_iter()
        .any(|vc| is_virtual_reviewable(vc, scheduler, now_ms, retire_after_days))
}

/// How many of `subject`'s virtual (remediation) cards are due right now — the
/// virtual-card counterpart of [`count_reviewable`], added to a deck's own due
/// count (never to its size/card count). See [`is_virtual_reviewable`].
pub fn count_reviewable_virtual(
    store: &Store,
    subject: &str,
    scheduler: &dyn Scheduler,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> usize {
    store
        .virtual_cards_for(subject)
        .into_iter()
        .filter(|vc| is_virtual_reviewable(vc, scheduler, now_ms, retire_after_days))
        .count()
}

/// Whether a card has *graduated* — reached FSRS `Review`, past the initial learning
/// steps. This is the always-on gate for a deck's exam / done state: a card still in
/// `New`/`Learning`, or with no FSRS state yet, has not graduated.
pub fn has_graduated(card: &Card, store: &Store) -> bool {
    store
        .get(card.id())
        .and_then(|s| s.fsrs.as_ref())
        .is_some_and(|f| f.graduated())
}

/// Each card's normalized Leitner stage (`0.0..=1.0`, unseen = 0, top-stage = 1)
/// — the per-card "weak → strong" value for a region's heatmap bar. A region's
/// bar reads all-red when new and all-green once its cards reach the top stage.
/// Deliberately *not* called "mastery", which is the exam's term.
pub fn card_strengths(card_ids: &[u64], store: &Store) -> Vec<f32> {
    card_ids
        .iter()
        .map(|&id| f32::from(store.get(id).map_or(0, |s| s.stage)) / f32::from(MAX_STAGE))
        .collect()
}

/// Whether one card would be served at `now_ms`: never-seen (fresh), or seen and
/// due, but not retired. The per-card decision [`build_queue`] makes (minus cram
/// and the new-card cap), factored out so callers can both test and *count* it.
pub fn is_reviewable(
    card: &Card,
    store: &Store,
    scheduler: &dyn Scheduler,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> bool {
    match store.get(card.id()) {
        Some(_) if is_retired(card, store, retire_after_days) => false,
        Some(state) => scheduler.is_due(state, now_ms),
        None => true,
    }
}

/// Whether these cards would yield anything to review at `now_ms` under
/// `scheduler`, so a caller — e.g. the picker — can tell, *before* building a
/// session, whether a deck has anything to do right now. See [`is_reviewable`].
pub fn has_reviewable(
    cards: &[Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> bool {
    cards
        .iter()
        .any(|card| is_reviewable(card, store, scheduler, now_ms, retire_after_days))
}

/// How many of these cards would be served right now — the due/new count for a
/// region or a whole deck (shown in the focus drawer). See [`is_reviewable`].
pub fn count_reviewable(
    cards: &[&Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> usize {
    cards
        .iter()
        .filter(|card| is_reviewable(card, store, scheduler, now_ms, retire_after_days))
        .count()
}

/// Per-stage counts for a set of cards (stage 0 = never seen).
pub fn histogram(cards: &[Card], store: &Store) -> StageHistogram {
    let mut h = [0usize; 6];
    for card in cards {
        match store.get(card.id()) {
            Some(state) => h[state.stage.clamp(1, 5) as usize] += 1,
            None => h[0] += 1,
        }
    }
    h
}

/// Builds the current timestamp once; convenience for callers.
pub fn now_ms() -> u64 {
    time::now_ms()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::store::Store;

    fn card(subject: &str, n: usize) -> Card {
        Card::plain(
            Arc::from(subject),
            format!("front {n}"),
            vec![format!("back {n}")],
            None,
            n,
        )
    }

    fn cards(n: usize) -> Vec<Card> {
        (0..n).map(|i| card("deck.txt", i)).collect()
    }

    fn empty_store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        (store, dir)
    }

    /// A fresh boxed scheduler for a session under test.
    fn sched() -> Box<dyn Scheduler> {
        Box::new(crate::scheduler::Fsrs::default())
    }

    /// A virtual card's stored entry: freshly created (stage 1, no FSRS yet —
    /// due ~1 min after `now_ms`, mirroring an acquired deck card).
    fn virtual_card(parent: &str, discriminator: &str, now_ms: u64) -> VirtualCard {
        use crate::store::{VirtualContent, VirtualKind, virtual_id};
        VirtualCard {
            id: virtual_id(VirtualKind::Remediation, parent, discriminator),
            kind: VirtualKind::Remediation,
            parent: parent.to_string(),
            content: VirtualContent {
                front: "virtual front".to_string(),
                back: vec!["virtual back".to_string()],
                mode: None,
            },
            state: CardState::new(now_ms),
            created_ms: now_ms,
            retired: false,
        }
    }

    /// The rendered `Card` a virtual card synthesizes to (mirrors what
    /// `main::synthesize_virtual` builds): the virtual content on a distinct,
    /// far-out `line` so it never shares a sibling group with a real card.
    fn virtual_synth_card(subject: &str, line: usize) -> Card {
        Card::plain(
            Arc::from(subject),
            "virtual front".to_string(),
            vec!["virtual back".to_string()],
            None,
            line,
        )
    }

    #[test]
    fn new_cards_enter_up_to_max_new() {
        let (store, _dir) = empty_store();
        let session = Session::new(
            cards(20),
            &store,
            sched(),
            SessionOptions {
                max_new: 5,
                ..Default::default()
            },
            1000,
        );
        assert_eq!(5, session.initial_size);
    }

    #[test]
    fn acquire_current_introduces_a_stage_one_card_without_a_review() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id();
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 1000);
        assert!(session.current_unseen(&store)); // a fresh card is acquired, not quizzed

        session.acquire_current(&mut store, 1000);

        let state = store.get(id).expect("acquired card is recorded");
        assert_eq!(1, state.stage);
        assert!(state.history.is_empty()); // acquiring is not a review
        assert_eq!(0, state.total_reviews);
        assert_eq!(1, session.stats.acquired);
        assert_eq!(0, session.stats.reviews);
        assert!(session.is_finished()); // kept but cooling — nothing servable this instant
    }

    #[test]
    fn acquired_cards_are_not_due_until_the_relearn_cooldown() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(cards(1), &store, sched(), SessionOptions::default(), 1000);
        session.acquire_current(&mut store, 1000);

        // Just acquired: nothing is due the instant it was seen (the ~1-min gap).
        assert!(!session.has_due_now(&store, 1000));
        assert!(!session.has_due_now(&store, 1000 + 60 * 1000 - 1));
        // Once the ~1-min cooldown passes, it is due for its first quiz.
        assert!(session.has_due_now(&store, 1000 + 60 * 1000));
    }

    #[test]
    fn an_acquired_card_returns_in_session_after_its_cooldown() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(cards(1), &store, sched(), SessionOptions::default(), 1000);
        let id = session.current().unwrap().id();
        session.acquire_current(&mut store, 1000);
        // Kept in the roster but cooling: nothing servable the instant it was seen.
        assert!(session.is_finished());
        // Once its ~1 min gap elapses the same session serves it for its first quiz.
        assert!(session.poll(&store, 1000 + 60 * 1000));
        assert_eq!(session.current().map(|c| c.id()), Some(id));
        assert!(!session.current_unseen(&store)); // a real quiz now, not another acquire
    }

    #[test]
    fn a_missed_card_is_not_re_served_before_its_fsrs_due() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        for c in &all {
            store.get_or_insert(c.id(), 0);
        }
        let now = 5 * 60 * 1000; // past the stage-1 cooldown: both due
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);

        let first = session.current().unwrap().id();
        session.grade(&mut store, Grade::Fail, now);
        // The other due card is served; the missed card is cooling, not re-served.
        assert!(session.current().is_some());
        assert_ne!(first, session.current().unwrap().id());
    }

    #[test]
    fn a_missed_card_reappears_once_its_step_elapses() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let first_id = all[0].id();
        for c in &all {
            store.get_or_insert(c.id(), 0);
        }
        let now = 5 * 60 * 1000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);

        assert_eq!(first_id, session.current().unwrap().id());
        session.grade(&mut store, Grade::Fail, now); // card 0 missed → cooling ~1 min
        session.grade(&mut store, Grade::Pass, now + 1000); // clear card 1
        // Well past the ~1 min learning step: the missed card is due again.
        session.poll(&store, now + 5 * 60_000);
        assert_eq!(first_id, session.current().unwrap().id());
    }

    #[test]
    fn same_session_fail_then_pass_does_not_graduate() {
        // The regression: a fail cannot be followed by an immediate scored pass,
        // so a fresh card cannot jump to FSRS Review off a sub-step re-drill.
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id();
        store.get_or_insert(id, 0);
        let now = 5 * 60 * 1000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);

        session.grade(&mut store, Grade::Fail, now);
        // Nothing else is due, so no further grade lands this appearance.
        assert!(session.current().is_none());
        let f = store.get(id).unwrap().fsrs.unwrap();
        assert_ne!(
            2, f.state,
            "not graduated to Review off an immediate re-drill"
        );
    }

    #[test]
    fn only_cooling_cards_left_finishes_the_session() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(all[0].id(), 0);
        let now = 5 * 60 * 1000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);
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
            store.get_or_insert(c.id(), 0);
        }
        let now = 5 * 60 * 1000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);
        session.grade(&mut store, Grade::Fail, now); // card 0 kept (cooling)
        session.grade(&mut store, Grade::Pass, now); // card 1 removed
        // One cooling, one gone: nothing servable right now.
        assert!(session.is_finished());
    }

    #[test]
    fn max_new_is_capped_per_session() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(5),
            &store,
            sched(),
            SessionOptions {
                max_new: 2,
                ..Default::default()
            },
            1000,
        );
        let mut acquired = 0;
        while session.current().is_some() {
            session.acquire_current(&mut store, 1000);
            acquired += 1;
        }
        assert_eq!(2, acquired, "the roster fixes the new set at start");
    }

    #[test]
    fn due_cards_take_priority_over_new_under_limit() {
        let (mut store, _dir) = empty_store();
        let all = cards(10);
        // Cards 7, 8, 9 were seen at t=0 and are due once the 5-min stage-1
        // cooldown has passed.
        for c in &all[7..] {
            store.get_or_insert(c.id(), 0);
        }
        let session = Session::new(
            all.clone(),
            &store,
            sched(),
            SessionOptions {
                max_new: 10,
                limit: Some(3),
                cram: false,
                order: Order::Scheduled,
                topology: None,
                retire_after_days: Some(DEFAULT_RETIRE_AFTER_DAYS),
            },
            5 * 60 * 1000,
        );
        assert_eq!(3, session.initial_size);
        // The queue holds exactly the due cards, not the new ones.
        assert_eq!("front 7", session.current().unwrap().front);
    }

    #[test]
    fn due_cards_are_ordered_by_due_time() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        // Two seen-but-unreviewed cards whose fallback due times differ by their
        // stage cooldown: card 0 (stage 2, ~1h) comes due before card 1 (stage 5,
        // ~1w). Card 2 is new. FSRS orders the due set by due time, earliest first.
        store.get_or_insert(all[0].id(), 0).stage = 2;
        store.get_or_insert(all[1].id(), 0).stage = 5;

        let now = 2 * 604_800_000; // two weeks later, everything is due
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);
        assert_eq!("front 0", session.current().unwrap().front); // due earliest (~1h)
        session.grade(&mut store, Grade::Pass, now);
        assert_eq!("front 1", session.current().unwrap().front); // due later (~1w)
        session.grade(&mut store, Grade::Pass, now);
        assert_eq!("front 2", session.current().unwrap().front); // new
    }

    #[test]
    fn sequential_order_follows_deck_order() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        // By due time card 0 (s2) leads, then card 1 (s5), then the new card 2.
        // Sequential ignores that and follows deck/file order.
        store.get_or_insert(all[0].id(), 0).stage = 2;
        store.get_or_insert(all[1].id(), 0).stage = 5;

        let now = 2 * 604_800_000;
        let mut session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions {
                order: Order::Sequential,
                ..Default::default()
            },
            now,
        );
        // File order, regardless of stage.
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
        // Stage 2 entered "now": cooldown 1h, not due.
        let now = 5_000_000;
        store.get_or_insert(all[0].id(), now).stage = 2;

        let session = Session::new(
            all.clone(),
            &store,
            sched(),
            SessionOptions {
                max_new: 0,
                ..Default::default()
            },
            now + 1,
        );
        assert!(session.is_finished());

        // But cram mode includes them.
        let session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions {
                max_new: 0,
                limit: None,
                cram: true,
                order: Order::Scheduled,
                topology: None,
                retire_after_days: Some(DEFAULT_RETIRE_AFTER_DAYS),
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
            store.get_or_insert(c.id(), 0);
        }
        let now = 5 * 60 * 1000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);
        assert_eq!(2, session.remaining());

        session.grade(&mut store, Grade::Fail, now); // card 0 kept (cooling)
        session.grade(&mut store, Grade::Pass, now); // card 1 passed

        assert_eq!(2, session.stats.reviews);
        assert_eq!(1, session.stats.passed);
        assert_eq!(1, session.stats.failed);
    }

    #[test]
    fn grading_records_fsrs_state() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id();
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 1000);

        session.grade(&mut store, Grade::Pass, 1000);
        // A graded card gains FSRS state and a recorded review (stage is frozen).
        let state = store.get(id).unwrap();
        assert!(state.fsrs.is_some());
        assert_eq!(1, state.total_reviews);
    }

    #[test]
    fn skip_rotates_queue() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(cards(2), &store, sched(), SessionOptions::default(), 1000);
        let first = session.current().unwrap().front.clone();
        session.skip(&store, 1000);
        assert_ne!(first, session.current().unwrap().front);
        assert_eq!(2, session.remaining());
        session.skip(&store, 1000);
        assert_eq!(first, session.current().unwrap().front);
        // Skipping must not touch the store.
        assert!(store.is_empty());
        let _ = &mut store;
    }

    #[test]
    fn remove_current_drops_card_without_grading() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(cards(2), &store, sched(), SessionOptions::default(), 1000);
        let removed = session.remove_current(&store, 1000);
        assert_eq!(1, removed.len());
        assert_eq!(1, session.remaining());
        assert_ne!(removed[0].front, session.current().unwrap().front);
        // The store is untouched by a removal.
        assert!(store.is_empty());
        let _ = &mut store;
    }

    #[test]
    fn remove_current_also_drops_cloze_siblings() {
        let (store, _dir) = empty_store();
        // Two sub-cards of one source card (same line) plus one other card.
        let mut all = vec![
            card("deck.txt", 1),
            card("deck.txt", 1),
            card("deck.txt", 2),
        ];
        all[0].back = vec!["hole a".into()];
        all[1].back = vec!["hole b".into()];
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 0);
        assert_eq!(3, session.remaining());
        // Removing one sub-card removes its sibling too, leaving only card 2.
        let removed = session.remove_current(&store, 0);
        assert_eq!(2, removed.len());
        assert_eq!(1, session.remaining());
        assert_eq!(2, session.current().unwrap().line);
    }

    /// Cards sharing a front line (cloze sub-cards) must not sit next to
    /// each other in the queue when other cards can go in between.
    #[test]
    fn cloze_siblings_are_separated() {
        let (store, _dir) = empty_store();
        // Two cloze groups (lines 1 and 2) with two sub-cards each, in deck
        // order: A1 A2 B1 B2.
        let mut all = Vec::new();
        for (line, name) in [(1, "A"), (2, "B")] {
            for hole in 1..=2 {
                let mut c = card("deck.txt", line);
                c.front = format!("{name}{hole}");
                c.back = vec![format!("{name} answer {hole}")];
                all.push(c);
            }
        }
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 0);

        let mut fronts = Vec::new();
        for _ in 0..session.remaining() {
            fronts.push(session.current().unwrap().front.clone());
            session.skip(&store, 0);
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

    /// With nothing to interleave, siblings are (unavoidably) adjacent.
    #[test]
    fn lone_sibling_group_still_fully_queued() {
        let (store, _dir) = empty_store();
        let mut all = Vec::new();
        for hole in 1..=3 {
            let mut c = card("deck.txt", 1);
            c.back = vec![format!("answer {hole}")];
            all.push(c);
        }
        let session = Session::new(all, &store, sched(), SessionOptions::default(), 0);
        assert_eq!(3, session.initial_size);
    }

    #[test]
    fn restart_picks_up_newly_due_and_new_cards() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(4),
            &store,
            sched(),
            SessionOptions {
                max_new: 2,
                ..Default::default()
            },
            1000,
        );
        assert_eq!(2, session.initial_size);
        session.grade(&mut store, Grade::Pass, 1000);
        session.grade(&mut store, Grade::Pass, 1001);
        assert!(session.is_finished());
        assert_eq!(2, session.stats.reviews);

        // A restart introduces the remaining two new cards and resets stats.
        assert!(session.restart(&store, 1002));
        assert_eq!(2, session.initial_size);
        assert_eq!(0, session.stats.reviews);
        assert!(!session.is_finished());
    }

    #[test]
    fn restart_with_nothing_due_returns_false_and_keeps_stats() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(cards(1), &store, sched(), SessionOptions::default(), 1000);
        session.grade(&mut store, Grade::Pass, 1000);
        assert!(session.is_finished());

        // The only card sits at stage 2 (1h cooldown); nothing is due and
        // the finished session's stats survive for the summary screen.
        assert!(!session.restart(&store, 1001));
        assert!(session.is_finished());
        assert_eq!(1, session.stats.reviews);
    }

    #[test]
    fn has_due_now_tracks_what_restart_would_find() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(cards(1), &store, sched(), SessionOptions::default(), 1000);
        // A new card is available before it is seen.
        assert!(session.has_due_now(&store, 1000));
        session.grade(&mut store, Grade::Pass, 1000);
        // A first Good enters an FSRS learning step (sub-day): nothing due right
        // after, matching restart().
        assert!(!session.has_due_now(&store, 1001));
        assert!(!session.restart(&store, 1001));
        // Once the learning step elapses it is due again (an hour is well past it).
        assert!(session.has_due_now(&store, 1000 + 3_600_000));
    }

    #[test]
    fn next_due_at_reports_earliest_due_time() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 1000);
        assert_eq!(None, session.next_due_at(&store)); // nothing seen yet
        session.grade(&mut store, Grade::Pass, 1000);
        // A first Good enters an FSRS learning step, due some time out (sub-day).
        let due = session
            .next_due_at(&store)
            .expect("a seen card has a due time");
        assert!(due > 1000 && due < 1000 + 86_400_000, "due {due}");
    }

    #[test]
    fn histogram_counts_stages() {
        let (mut store, _dir) = empty_store();
        let all = cards(4);
        store.get_or_insert(all[0].id(), 0).stage = 1;
        store.get_or_insert(all[1].id(), 0).stage = 5;

        let h = histogram(&all, &store);
        assert_eq!([2, 1, 0, 0, 0, 1], h);
    }

    /// An FSRS state whose interval sits at the retirement cap.
    fn retired_fsrs() -> crate::store::FsrsState {
        crate::store::FsrsState {
            scheduled_days: DEFAULT_RETIRE_AFTER_DAYS,
            ..Default::default()
        }
    }

    #[test]
    fn is_retired_once_the_interval_passes_the_cap() {
        let (mut store, _dir) = empty_store();
        let c = card("deck.txt", 0);

        assert!(!is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS))); // unseen — never retired
        // An FSRS interval at/past the cap rests.
        store.get_or_insert(c.id(), 0).fsrs = Some(retired_fsrs());
        assert!(is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
        // Just below the cap: still in rotation.
        store.get_or_insert(c.id(), 0).fsrs = Some(crate::store::FsrsState {
            scheduled_days: DEFAULT_RETIRE_AFTER_DAYS - 1,
            ..Default::default()
        });
        assert!(!is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
        // A legacy card at the top Leitner stage but with no FSRS state is no longer
        // retired — retirement now needs a grown FSRS interval, not a stage.
        let s = store.get_or_insert(c.id(), 0);
        s.fsrs = None;
        s.stage = MAX_STAGE;
        s.streak = 1;
        assert!(!is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
    }

    #[test]
    fn has_reviewable_counts_new_and_due_not_cooldown_or_retired() {
        let (mut store, _dir) = empty_store();
        let sched = sched();
        let now = 10_000_000;

        // A brand-new (unseen) card is reviewable.
        assert!(has_reviewable(
            &cards(1),
            &store,
            sched.as_ref(),
            now,
            Some(DEFAULT_RETIRE_AFTER_DAYS)
        ));

        // A card just passed to stage 2 at `now` is on cooldown (due in 1h):
        // not reviewable now, reviewable once its due time arrives.
        let c = card("deck.txt", 0);
        let s = store.get_or_insert(c.id(), now);
        s.stage = 2;
        s.streak = 1;
        s.stage_entered_ms = now;
        let one = std::slice::from_ref(&c);
        let cap = Some(DEFAULT_RETIRE_AFTER_DAYS);
        assert!(!has_reviewable(one, &store, sched.as_ref(), now, cap));
        assert!(has_reviewable(
            one,
            &store,
            sched.as_ref(),
            now + 3_600_000,
            cap
        ));

        // A retired card (FSRS interval past the cap) never counts, even past due.
        store.get_or_insert(c.id(), now).fsrs = Some(retired_fsrs());
        assert!(!has_reviewable(
            std::slice::from_ref(&c),
            &store,
            sched.as_ref(),
            now + 3_600_000,
            cap
        ));
    }

    #[test]
    fn retired_card_excluded_even_under_cram() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(all[0].id(), 0).fsrs = Some(retired_fsrs()); // retired

        let session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions {
                max_new: 10,
                limit: None,
                cram: true,
                order: Order::Scheduled,
                topology: None,
                retire_after_days: Some(DEFAULT_RETIRE_AFTER_DAYS),
            },
            1000,
        );
        // Resting: not queued, even though cram ignores cooldowns.
        assert!(session.is_finished());
    }

    #[test]
    fn cram_correct_refreshes_without_rewarding() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        // A mature, graduated card with a real 30-day interval.
        store.get_or_insert(all[0].id(), 0).fsrs = Some(crate::store::FsrsState {
            stability: 30.0,
            difficulty: 5.0,
            scheduled_days: 30,
            state: 2,
            due_ms: 1000,
            ..Default::default()
        });
        let before = store.get(all[0].id()).unwrap().fsrs.unwrap();

        let mut session = Session::new(
            all.clone(),
            &store,
            sched(),
            SessionOptions {
                cram: true,
                ..Default::default()
            },
            10_000,
        );
        session.grade(&mut store, Grade::Pass, 10_000);

        let after = store.get(all[0].id()).unwrap();
        let f = after.fsrs.unwrap();
        assert_eq!(before.stability, f.stability); // no reward
        assert_eq!(before.scheduled_days, f.scheduled_days); // interval kept
        assert_eq!(10_000u64 + 30 * 86_400_000, f.due_ms); // re-anchored to now + interval
        assert!(after.history.is_empty()); // a refresh, not a recorded review
    }

    #[test]
    fn cram_miss_lapses_normally() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(all[0].id(), 0).fsrs = Some(crate::store::FsrsState {
            stability: 30.0,
            difficulty: 5.0,
            scheduled_days: 30,
            state: 2,
            ..Default::default()
        });

        let mut session = Session::new(
            all.clone(),
            &store,
            sched(),
            SessionOptions {
                cram: true,
                ..Default::default()
            },
            10_000,
        );
        session.grade(&mut store, Grade::Fail, 10_000);

        // A miss is a real lapse: the scheduler ran (recorded, stability dropped).
        let after = store.get(all[0].id()).unwrap();
        assert_eq!(1, after.history.len());
        assert!(after.fsrs.unwrap().stability < 30.0);
    }

    #[test]
    fn cram_serves_each_card_once() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let id_a = all[0].id();
        for c in &all {
            store.get_or_insert(c.id(), 0).fsrs = Some(crate::store::FsrsState {
                stability: 30.0,
                difficulty: 5.0,
                scheduled_days: 30,
                state: 2,
                ..Default::default()
            });
        }

        let mut session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions {
                cram: true,
                ..Default::default()
            },
            10_000,
        );
        session.grade(&mut store, Grade::Fail, 10_000); // miss → one real lapse, not re-served
        session.grade(&mut store, Grade::Pass, 10_000); // the other card, refreshed
        assert!(
            session.is_finished(),
            "cram is a single pass over the roster"
        );
        // The missed crammed card recorded exactly one review (no in-session re-drill).
        assert_eq!(1, store.get(id_a).unwrap().history.len());
    }

    fn topology_order(walk: &[&Card]) -> TopologyOrder {
        let ids: Vec<u64> = walk.iter().map(|c| c.id()).collect();
        TopologyOrder::from_walk(&ids)
    }

    #[test]
    fn topology_reorders_the_due_set() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        // All three seen at t=0 and due once the 5-min stage-1 cooldown passes;
        // scheduler order is 0,1,2.
        for c in &all {
            store.get_or_insert(c.id(), 0);
        }
        // A topology that reverses that order takes over.
        let topo = topology_order(&[&all[2], &all[1], &all[0]]);
        let mut session = Session::new(
            all.clone(),
            &store,
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
        // Card 0 is due; card 1 is on cooldown (stage 2 entered now, 1h cooldown).
        store.get_or_insert(all[0].id(), 0);
        store.get_or_insert(all[1].id(), now).stage = 2;
        // A topology listing the not-due card first must NOT pull it in.
        let topo = topology_order(&[&all[1], &all[0]]);
        let session = Session::new(
            all.clone(),
            &store,
            sched(),
            SessionOptions {
                max_new: 0,
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
            store.get_or_insert(c.id(), 0);
        }
        // The walk lists only the middle card; the other two keep scheduler order.
        let topo = topology_order(&[&all[1]]);
        let mut session = Session::new(
            all.clone(),
            &store,
            sched(),
            SessionOptions {
                topology: Some(topo),
                ..Default::default()
            },
            1_000_000,
        );
        assert_eq!("front 1", session.current().unwrap().front); // ranked first
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 0", session.current().unwrap().front); // then 0, 2 in order
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 2", session.current().unwrap().front);
    }

    #[test]
    fn retired_card_excluded_even_with_a_topology() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(all[0].id(), 0).fsrs = Some(retired_fsrs()); // retired
        // A topology listing the retired card cannot resurrect it — the filter
        // runs before the topology sort.
        let topo = topology_order(&[&all[0]]);
        let session = Session::new(
            all.clone(),
            &store,
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
        // Two sub-cards of one cloze share (subject, line) → siblings; plus one
        // other card. Without a topology, separate_siblings would slip `other`
        // between the siblings; with a topology, the walk order is kept verbatim.
        let sib_a = Card::plain(
            Arc::from("d.txt"),
            "front a".into(),
            vec!["a".into()],
            None,
            7,
        );
        let sib_b = Card::plain(
            Arc::from("d.txt"),
            "front b".into(),
            vec!["b".into()],
            None,
            7,
        );
        let other = card("d.txt", 3);
        let all = vec![sib_a.clone(), sib_b.clone(), other.clone()];
        for c in &all {
            store.get_or_insert(c.id(), 0);
        }
        let topo = topology_order(&[&sib_a, &sib_b, &other]);
        let mut session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions {
                topology: Some(topo),
                ..Default::default()
            },
            1_000_000,
        );
        // Siblings stay adjacent (walk order), not split by `other`.
        assert_eq!("front a", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front b", session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, 1_000_000);
        assert_eq!("front 3", session.current().unwrap().front);
    }

    #[test]
    fn topology_keeps_due_ahead_of_new_under_a_limit() {
        let (mut store, _dir) = empty_store();
        let all = cards(4);
        // Cards 0,1 are due; 2,3 are new.
        store.get_or_insert(all[0].id(), 0);
        store.get_or_insert(all[1].id(), 0);
        // A walk that ranks a NEW card (3) ahead of the due cards must not let it
        // jump past them: due-priority holds, the topology only orders within a
        // group, so the limit keeps the two due cards (ordered 1 before 0).
        let topo = topology_order(&[&all[3], &all[1], &all[0], &all[2]]);
        let session = Session::new(
            all.clone(),
            &store,
            sched(),
            SessionOptions {
                max_new: 10,
                limit: Some(2),
                topology: Some(topo),
                ..Default::default()
            },
            1_000_000,
        );
        assert_eq!(2, session.initial_size);
        assert_eq!("front 1", session.current().unwrap().front);
    }

    #[test]
    fn card_strengths_normalizes_each_stage() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        store.get_or_insert(all[0].id(), 0).stage = MAX_STAGE; // top → 1.0
        store.get_or_insert(all[1].id(), 0).stage = 1; // 1/5 → 0.2
        // all[2] is unseen → 0.0
        let ids: Vec<u64> = all.iter().map(Card::id).collect();
        let s = card_strengths(&ids, &store);
        assert_eq!(3, s.len());
        assert!((s[0] - 1.0).abs() < 1e-6);
        assert!((s[1] - 0.2).abs() < 1e-6);
        assert_eq!(0.0, s[2]);
        assert!(card_strengths(&[], &store).is_empty());
    }

    #[test]
    fn virtual_card_joins_the_roster_and_is_served() {
        let (mut store, _dir) = empty_store();
        let vc = virtual_card("deck.txt", "gap-1", 0);
        let vid = vc.id.clone();
        store.insert_virtual(vc);

        let synth = virtual_synth_card("deck.txt", 1_000_000);
        let now = 60_000; // past the stage-1 cooldown: due
        let session = Session::new_with_virtual(
            vec![synth],
            vec![Some(vid)],
            &store,
            sched(),
            SessionOptions::default(),
            now,
        );
        assert_eq!(1, session.initial_size);
        assert_eq!("virtual front", session.current().unwrap().front);
    }

    #[test]
    fn grading_a_virtual_card_updates_its_virtual_state_not_store_cards() {
        let (mut store, _dir) = empty_store();
        let vc = virtual_card("deck.txt", "gap-1", 0);
        let vid = vc.id.clone();
        store.insert_virtual(vc);

        let synth = virtual_synth_card("deck.txt", 1_000_000);
        let synth_id = synth.id();
        let now = 60_000;
        let mut session = Session::new_with_virtual(
            vec![synth],
            vec![Some(vid.clone())],
            &store,
            sched(),
            SessionOptions::default(),
            now,
        );

        session.grade(&mut store, Grade::Pass, now);

        let vc_after = store.get_virtual(&vid).expect("virtual entry kept");
        assert!(vc_after.state.fsrs.is_some());
        assert_eq!(1, vc_after.state.total_reviews);
        // No `u64` ghost written to the deck-card keyspace.
        assert!(store.get(synth_id).is_none());
    }

    #[test]
    fn virtual_card_not_treated_as_unseen() {
        let (mut store, _dir) = empty_store();
        let vc = virtual_card("deck.txt", "gap-1", 0);
        let vid = vc.id.clone();
        store.insert_virtual(vc);

        let synth = virtual_synth_card("deck.txt", 1_000_000);
        let now = 60_000;
        let session = Session::new_with_virtual(
            vec![synth],
            vec![Some(vid)],
            &store,
            sched(),
            SessionOptions::default(),
            now,
        );
        assert!(!session.current_unseen(&store));
    }

    #[test]
    fn a_missed_virtual_card_reappears_on_its_fsrs_due() {
        let (mut store, _dir) = empty_store();
        let vc = virtual_card("deck.txt", "gap-1", 0);
        let vid = vc.id.clone();
        store.insert_virtual(vc);
        let deck_card = card("deck.txt", 0);
        store.get_or_insert(deck_card.id(), 0);

        let synth = virtual_synth_card("deck.txt", 1_000_000);
        let synth_id = synth.id();
        let now = 5 * 60 * 1000; // both due (past the stage-1 cooldown)
        let mut session = Session::new_with_virtual(
            vec![synth, deck_card],
            vec![Some(vid), None],
            &store,
            sched(),
            SessionOptions::default(),
            now,
        );

        assert_eq!(synth_id, session.current().unwrap().id());
        session.grade(&mut store, Grade::Fail, now); // virtual missed → cooling
        session.grade(&mut store, Grade::Pass, now + 1000); // clear the deck card
        // Well past the ~1 min learning step: the missed virtual card is due again.
        session.poll(&store, now + 5 * 60_000);
        assert_eq!(synth_id, session.current().unwrap().id());
    }

    #[test]
    fn count_reviewable_virtual_counts_due_excludes_archived() {
        let (mut store, _dir) = empty_store();
        let now = 61_000; // past the stage-1 cooldown for a t=0 card
        let cap = Some(DEFAULT_RETIRE_AFTER_DAYS);
        let sched = sched();

        // Due: created at t=0, past its stage-1 cooldown by `now`.
        store.insert_virtual(virtual_card("deck.txt", "gap-due", 0));
        // Not yet due: created at `now`, still cooling down.
        store.insert_virtual(virtual_card("deck.txt", "gap-not-due", now));
        // Archived: otherwise due, but excluded.
        let mut archived = virtual_card("deck.txt", "gap-archived", 0);
        archived.retired = true;
        store.insert_virtual(archived);

        assert_eq!(
            1,
            count_reviewable_virtual(&store, "deck.txt", sched.as_ref(), now, cap)
        );
        assert!(has_reviewable_virtual(
            &store,
            "deck.txt",
            sched.as_ref(),
            now,
            cap
        ));
    }

    #[test]
    fn next_due_at_includes_virtual_cards() {
        let (mut store, _dir) = empty_store();
        let vc = virtual_card("deck.txt", "gap-1", 1000);
        let vid = vc.id.clone();
        store.insert_virtual(vc);

        let synth = virtual_synth_card("deck.txt", 1_000_000);
        let session = Session::new_with_virtual(
            vec![synth],
            vec![Some(vid)],
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        // The virtual entry already has state (stage 1 @ t=1000): its FSRS
        // fallback due is 1000 + the stage-1 cooldown.
        let due = session
            .next_due_at(&store)
            .expect("a virtual card's due time is reported");
        assert_eq!(1000 + 60_000, due);
    }

    #[test]
    fn virtual_card_retires_at_the_interval_cap() {
        let (mut store, _dir) = empty_store();
        let vc = virtual_card("deck.txt", "gap-1", 0);
        let vid = vc.id.clone();
        store.insert_virtual(vc);
        let options = SessionOptions {
            retire_after_days: Some(4),
            ..SessionOptions::default()
        };

        // First real review: still acquiring (needs two Goods to graduate), so
        // the interval stays at 0 — well under the cap.
        let now = 60_000;
        let mut session = Session::new_with_virtual(
            vec![virtual_synth_card("deck.txt", 1_000_000)],
            vec![Some(vid.clone())],
            &store,
            sched(),
            options.clone(),
            now,
        );
        session.grade(&mut store, Grade::Pass, now);
        assert!(!store.get_virtual(&vid).unwrap().retired);

        // Second review, once due: graduates to `Review` with a 4-day interval —
        // right at the cap.
        let now = 86_460_000;
        let mut session = Session::new_with_virtual(
            vec![virtual_synth_card("deck.txt", 1_000_000)],
            vec![Some(vid.clone())],
            &store,
            sched(),
            options,
            now,
        );
        session.grade(&mut store, Grade::Pass, now);

        let after = store.get_virtual(&vid).expect("entry + history kept, not deleted");
        assert!(after.retired);
        assert_eq!(4, after.state.fsrs.as_ref().unwrap().scheduled_days);
        assert_eq!(2, after.state.total_reviews);
    }

    #[test]
    fn retired_virtual_card_is_excluded_from_queue_and_counts() {
        let (mut store, _dir) = empty_store();
        let mut vc = virtual_card("deck.txt", "gap-1", 0);
        vc.retired = true;
        let vid = vc.id.clone();
        store.insert_virtual(vc);

        // Otherwise past its stage-1 cooldown: would be due if not archived.
        let now = 61_000;
        let session = Session::new_with_virtual(
            vec![virtual_synth_card("deck.txt", 1_000_000)],
            vec![Some(vid.clone())],
            &store,
            sched(),
            SessionOptions::default(),
            now,
        );
        assert!(session.is_finished()); // not served: no roster entry

        let cap = Some(DEFAULT_RETIRE_AFTER_DAYS);
        assert_eq!(
            0,
            count_reviewable_virtual(&store, "deck.txt", sched().as_ref(), now, cap)
        );
        // The entry itself survives — archived, not deleted.
        assert!(store.get_virtual(&vid).is_some());
    }

    #[test]
    fn retire_only_at_cap_not_below() {
        let (mut store, _dir) = empty_store();
        let vc = virtual_card("deck.txt", "gap-1", 0);
        let vid = vc.id.clone();
        store.insert_virtual(vc);

        let now = 60_000; // past the stage-1 cooldown
        let mut session = Session::new_with_virtual(
            vec![virtual_synth_card("deck.txt", 1_000_000)],
            vec![Some(vid.clone())],
            &store,
            sched(),
            SessionOptions::default(), // default cap, far above a 0-day interval
            now,
        );
        session.grade(&mut store, Grade::Pass, now);

        assert!(!store.get_virtual(&vid).unwrap().retired);
    }
}
