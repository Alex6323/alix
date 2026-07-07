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
//! recorded as acquired, then left to settle ~1 min before its first quiz.

use std::{collections::VecDeque, path::PathBuf};

use rs_fsrs::Parameters;

use crate::{
    augment::TopologyOrder,
    card::Card,
    level::Level,
    scheduler::{Grade, Scheduler},
    store::{Store, VirtualCard},
    time,
    trace::SourceBase,
};

/// Per-deck information the TUI needs, keyed by subject.
pub struct DeckInfo {
    /// The deck file, for saving notes from the ask view.
    pub path: PathBuf,
    /// Reference links (`% link:` lines) offered to Claude as background.
    pub links: Vec<String>,
    /// The deck's `% source:` project root, for the grounded ask-tutor
    /// (`[ask] source_access`); `None` when there's no local source.
    pub source_root: Option<PathBuf>,
    /// Whether the grounded tutor may read this deck's source — the *effective*
    /// value (the deck's workspace `source_access` override, else the global
    /// `[ask] source_access`).
    pub source_access: bool,
    /// The deck's source base, for resolving a card's `% at:` citation excerpt
    /// on reveal (fact-card citations).
    pub source_base: SourceBase,
}

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
    /// The learner's depth-ladder target — the rung a graduated card climbs
    /// toward on a spaced pass (see [`Session::grade`]). From the resolved
    /// `[review] depth` (per-workspace overridable).
    pub target: Level,
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
            target: Level::default(),
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

/// A review session over a fixed set of cards.
pub struct Session {
    cards: Vec<Card>,
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

// TODO(task 5): `climb_level`/`descend_level` and their call sites in `grade`
// are the old `Rung::climb`/`Rung::descend` (`Level` owns no such methods —
// climbing/descending a card's depth dies with the rung ladder); inlined here
// only to keep this behavior compiling unchanged until Task 5 deletes it.

/// The next level up, saturating at `Reconstruct`.
fn climb_level(level: Level) -> Level {
    match level {
        Level::Recognize => Level::Recall,
        Level::Recall => Level::Reconstruct,
        Level::Reconstruct => Level::Reconstruct,
    }
}

/// One level down, floored at `Recall` (`Recognize` is never a live target).
fn descend_level(level: Level) -> Level {
    match level {
        Level::Reconstruct => Level::Recall,
        Level::Recall | Level::Recognize => Level::Recall,
    }
}

impl Session {
    /// Builds a session at time `now_ms`.
    ///
    /// The roster holds, in order: all due cards (earliest FSRS due first), then
    /// up to `max_new` unseen cards in deck order. Sub-cards of the same cloze
    /// card are kept apart whenever other cards are available. A virtual
    /// (remediation) card is just one more card in `cards` — its schedule is an
    /// ordinary `store.cards` entry keyed by its `Card::id`, so it needs no
    /// special routing; `build_review` synthesizes and injects it before this.
    pub fn new(
        cards: Vec<Card>,
        store: &Store,
        scheduler: Box<dyn Scheduler>,
        options: SessionOptions,
        now_ms: u64,
    ) -> Self {
        let roster: Vec<usize> = build_queue(&cards, store, &*scheduler, &options, now_ms).into();
        let initial_size = roster.len();

        let mut session = Self {
            cards,
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
        let roster: Vec<usize> =
            build_queue(&self.cards, store, &*self.scheduler, &self.options, now_ms).into();
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
        !build_queue(&self.cards, store, &*self.scheduler, &self.options, now_ms).is_empty()
    }

    /// The earliest upcoming due time over all seen cards of this session's
    /// decks (deck cards and virtual cards alike, which share `store.cards`), if
    /// any.
    pub fn next_due_at(&self, store: &Store) -> Option<u64> {
        self.cards
            .iter()
            .filter_map(|c| store.get(c.id()))
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

    /// The `Card::id` of the current card, if one is up (else `None`). Lets a
    /// frontend act on the card being reviewed right now (e.g. promote it).
    pub fn current_id(&self) -> Option<u64> {
        self.current_idx.map(|i| self.cards[i].id())
    }

    /// Whether the current card is a virtual (remediation) one — membership in
    /// the store's content sidecar. `false` for an authored deck card, or when
    /// nothing is current. Lets a frontend offer to promote it.
    pub fn current_is_virtual(&self, store: &Store) -> bool {
        self.current().is_some_and(|c| store.is_virtual(c.id()))
    }

    /// Whether the current card has never been seen (no stored progress). Such a
    /// card is *acquired* — shown via [`acquire_current`](Self::acquire_current) —
    /// rather than quizzed cold. A virtual card's entry always exists (it is
    /// created already-scheduled), so this is never true for one — it skips
    /// the attempt-first acquire screen entirely.
    pub fn current_unseen(&self, store: &Store) -> bool {
        self.current_idx
            .is_some_and(|i| store.get(self.cards[i].id()).is_none())
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
        // Uniform: a virtual card's schedule is an ordinary `store.cards` entry
        // keyed by its `Card::id`, so it grades exactly like a deck card.
        let card = &self.cards[index];
        let state = store.get_or_insert(card.id(), now_ms);
        // Cram refresh: a correct answer keeps the card fresh without rewarding it
        // (re-anchor its due — no FSRS update, no recorded review). A cram miss is a
        // genuine lapse, and every normal grade runs the real scheduler.
        if self.options.cram && grade.passed() {
            // A cram refresh is a documented no-reward path, so the ladder climb
            // must never fire here — only a real review can promote a rung.
            self.scheduler.reanchor(state, now_ms);
        } else {
            // Whether the Recall schedule had *already* reached FSRS `Review`
            // before this review — pinned to Recall (`.recall`, not a rung:
            // lifecycle stays on the Recall schedule regardless of level). The
            // pass that graduates a card must not itself trigger the climb
            // (that would skip the current rung's spaced practice entirely);
            // only a *later* spaced pass counts. Captured before `apply`, which
            // is what advances the card into `Review`.
            let was_graduated_before = state.recall.as_ref().is_some_and(|f| f.graduated());
            self.scheduler.apply(state, grade, now_ms);
            // TODO(task 5): the ladder climb (spec §3.3) used to promote a
            // graduated rung on a spaced pass via `state.rung` /
            // `state.passes_since_graduation` / `state.set_rung` — all deleted
            // from `CardState` in Task 3 (session-owned levels replace
            // card-owned rungs, so there is nowhere left to persist a climb).
            // Stood in with a local, non-persisted default per the brief so
            // `climb_level` keeps compiling unchanged in shape; Task 5 deletes
            // this block and `climb_level`/`descend_level` for good.
            let rung = Level::default();
            let target = self.options.target;
            if was_graduated_before
                && rung < target
                && grade == Grade::Pass
                && state.recall.as_ref().is_some_and(|f| f.scheduled_days >= 3)
            {
                let _ = climb_level(rung); // nowhere left to persist a climb
            }
        }
        // TODO(task 5): the descent-net (spec §3.4 v1) similarly read/wrote the
        // now-deleted `rung`/`passes_since_graduation`/`set_rung` — same
        // stand-in as the climb block above.
        if !grade.passed() {
            let _ = descend_level(Level::default()); // nowhere left to persist a descent
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

    /// Introduces the current never-seen card: records it as acquired and moves on.
    /// It is *not* graded and gets *no* history entry — acquiring is a first
    /// exposure, not a review. The card is **kept** in the session, cooling on its
    /// ~1 min acquire cooldown, so its first real quiz surfaces again later in *this
    /// same session* once that gap passes. Does nothing when nothing is up.
    pub fn acquire_current(&mut self, store: &mut Store, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        // `get_or_insert` creates the state as freshly acquired, due ~1 min out via
        // the acquire cooldown — no `scheduler.apply`, no recorded review. The card
        // stays in the roster so the due-driven serving surfaces it again for its
        // first real quiz once the gap elapses, in this same session. A virtual
        // card always has state (created already-scheduled), so `current_unseen`
        // is never true for one and this is only ever reached by a deck card.
        store.get_or_insert(self.cards[index].id(), now_ms);
        self.stats.acquired += 1;
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

    /// Whether roster card `i` can be served right now: a retired/archived card
    /// never is (cram included — retirement is derived from the FSRS interval, so
    /// a retired card is only un-retired by raising `retire_after` past its
    /// interval, or — for a virtual card — by re-failing its gap, which recreates
    /// it fresh); otherwise under cram, always; otherwise unseen (fresh) or
    /// FSRS-due. Deck and virtual cards share the one `store.cards` rule.
    fn servable(&self, i: usize, store: &Store, now_ms: u64) -> bool {
        let card = &self.cards[i];
        if is_retired(card, store, self.options.retire_after_days) {
            return false;
        }
        match store.get(card.id()) {
            Some(state) => self.options.cram || self.scheduler.is_due(state, now_ms),
            None => true,
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
}

/// Builds the review queue: due cards in scheduler order, then up to
/// `max_new` unseen cards, capped by `limit`, with cloze siblings separated.
/// A virtual (remediation) card is just another `Card` here — its schedule is
/// an ordinary `store.cards` entry keyed by its `Card::id`, so it flows through
/// the same rule as a deck card (it simply never lands in `fresh`, being
/// created already-scheduled).
fn build_queue(
    cards: &[Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    options: &SessionOptions,
    now_ms: u64,
) -> VecDeque<usize> {
    let mut due: Vec<usize> = Vec::new();
    let mut fresh: Vec<usize> = Vec::new();

    for (i, card) in cards.iter().enumerate() {
        match store.get(card.id()) {
            // A retired card rests until its interval drops below the cap —
            // never scheduled, not even under cram.
            Some(_) if is_retired(card, store, options.retire_after_days) => {}
            Some(state) => {
                if options.cram || scheduler.is_due(state, now_ms) {
                    due.push(i);
                }
            }
            None => fresh.push(i),
        }
    }

    // Order due cards by their FSRS due time, earliest first.
    due.sort_by_key(|&i| {
        store
            .get(cards[i].id())
            .map_or(u64::MAX, |s| scheduler.due_at(s))
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
        .and_then(|s| s.schedule(Level::Recall))
        .is_some_and(|f| f.scheduled_days >= cap)
}

/// Whether a virtual card would be served now: not retired, and FSRS-due. The
/// virtual-card counterpart of [`is_reviewable`] (a virtual card is never
/// "new" — it always has state, so there is no fresh/unseen branch). Its
/// schedule and retirement read from `store.cards[vc.id]`, exactly like a deck
/// card — so raising the cap un-retires it symmetrically.
pub fn is_virtual_reviewable(
    vc: &VirtualCard,
    store: &Store,
    scheduler: &dyn Scheduler,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> bool {
    !is_retired_id(vc.id, store, retire_after_days)
        && store
            .get(vc.id)
            .is_some_and(|s| scheduler.is_due(s, now_ms))
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
        .any(|vc| is_virtual_reviewable(vc, store, scheduler, now_ms, retire_after_days))
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
        .filter(|vc| is_virtual_reviewable(vc, store, scheduler, now_ms, retire_after_days))
        .count()
}

/// How many of `subject`'s virtual (remediation) cards become due within
/// `window_ms` of `now_ms` (not already due now) — the virtual-card
/// counterpart of `stats`' `due_24h` bucket for deck cards. Archived
/// (retired) cards never count. See [`count_reviewable_virtual`] for the
/// "due now" count.
pub fn count_due_soon_virtual(
    store: &Store,
    subject: &str,
    scheduler: &dyn Scheduler,
    now_ms: u64,
    window_ms: u64,
    retire_after_days: Option<u32>,
) -> usize {
    store
        .virtual_cards_for(subject)
        .into_iter()
        .filter(|vc| !is_retired_id(vc.id, store, retire_after_days))
        .filter(|vc| {
            store.get(vc.id).is_some_and(|s| {
                let due = scheduler.due_at(s);
                due > now_ms && due <= now_ms + window_ms
            })
        })
        .count()
}

/// Whether a card has *graduated* — reached FSRS `Review`, past the initial learning
/// steps. This is the always-on gate for a deck's exam / done state: a card still in
/// `New`/`Learning`, or with no FSRS state yet, has not graduated. Pinned to the
/// Recall schedule — lifecycle stays on Recall regardless of level (spec §4.5).
pub fn has_graduated(card: &Card, store: &Store) -> bool {
    store
        .get(card.id())
        .and_then(|s| s.schedule(Level::Recall))
        .is_some_and(|f| f.graduated())
}

/// Each card's FSRS retrievability (`0.0..=1.0`, the probability of recall at
/// `now_ms`) — the per-card "weak → strong" value for a region's heatmap bar.
/// A region's bar reads all-red for a card with no FSRS state yet and
/// brightens as retrievability nears 1. Deliberately *not* called "mastery",
/// which is the exam's term. `now_ms` is a parameter (not read internally) so
/// callers stay testable.
pub fn card_strengths(card_ids: &[u64], store: &Store, now_ms: u64) -> Vec<f32> {
    card_ids
        .iter()
        .map(|&id| retrievability(store, id, now_ms))
        .collect()
}

/// One card's FSRS-5 retrievability at `now_ms`, via `rs_fsrs`'s own
/// power-forgetting-curve formula (`Parameters::forgetting_curve`) applied to
/// our stored `FsrsState` (kept as plain `u64` ms, decoupled from
/// `rs_fsrs::Card`'s `DateTime`-based fields — see [`crate::store::FsrsState`]).
/// A card not yet under FSRS, or with non-positive stability, has no
/// meaningful curve — `0.0`.
fn retrievability(store: &Store, card_id: u64, now_ms: u64) -> f32 {
    let Some(f) = store.get(card_id).and_then(|s| s.schedule(Level::Recall)) else {
        return 0.0;
    };
    if f.stability <= 0.0 {
        return 0.0;
    }
    let elapsed_days = now_ms.saturating_sub(f.last_review_ms) as f64 / 86_400_000.0;
    Parameters::forgetting_curve(elapsed_days, f.stability).clamp(0.0, 1.0) as f32
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

/// Builds the current timestamp once; convenience for callers.
pub fn now_ms() -> u64 {
    time::now_ms()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::store::{FsrsState, Store};

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

    /// Inserts a virtual (remediation) card for `parent` into `store` the way
    /// the substrate does — sidecar content keyed by its `Card::id`, plus a
    /// fresh `store.cards` schedule at `created_ms` (freshly acquired, no FSRS
    /// yet, due ~1 min out) — and returns its synthesized `Card` (mirroring
    /// `main::synthesize_virtual`: the parsed card on a far-out `line`). `back`
    /// drives the id, so distinct `back` values give distinct virtual cards.
    fn insert_virtual(store: &mut Store, parent: &str, back: &str, created_ms: u64) -> Card {
        let text = format!("# virtual front\n\t{back}\n");
        let mut card = crate::parser::parse_str(parent, &text).unwrap().remove(0);
        card.line = 1_000_000;
        let id = card.id();
        store.insert_virtual(VirtualCard {
            id,
            kind: crate::store::VirtualKind::Remediation,
            parent: parent.to_string(),
            text,
            created_ms,
        });
        store.get_or_insert(id, created_ms);
        card
    }

    /// Property/invariant guard for the serve loop (added pre-v2 as release
    /// insurance): across a long, deterministically-fuzzed run of acquires,
    /// grades, and time jumps, the cursor and the servable count must never drift
    /// out of sync with the roster — the served card is always servable, the
    /// cursor is the first servable roster card, `remaining()` equals the servable
    /// roster count, `is_finished()` agrees with "nothing servable", and a card
    /// that has passed (left the roster) is never served again.
    #[test]
    fn serve_loop_invariants_hold_under_a_fuzzed_grade_sequence() {
        let (mut store, _dir) = empty_store();
        let n = 12;
        let mut session = Session::new(cards(n), &store, sched(), SessionOptions::default(), 0);

        // Deterministic pseudo-random driver (an LCG) — reproducible, no `rand` dep.
        let mut rng: u64 = 0x2545_F491_4F6C_DD1D;
        let mut roll = |bound: u64| -> u64 {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) % bound
        };

        let mut passed = vec![false; n]; // a card index that left the roster on a pass
        let mut now = 0u64;
        let mut drained = false;

        for _ in 0..2000 {
            session.poll(&store, now);

            // The servable universe is the roster; recompute it independently at
            // this instant and hold the session's bookkeeping to it.
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
            assert_eq!(
                session.current_idx,
                servable.first().copied(),
                "the cursor points at the first servable roster card"
            );
            assert!(!passed[idx], "a passed card must never be served again");

            if session.current_unseen(&store) {
                session.acquire_current(&mut store, now);
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

            // Jump forward up to ~2h so cooled cards re-enter on the next poll.
            now = now.saturating_add(roll(2 * 3600 * 1000));
        }

        assert!(
            drained,
            "with time advancing and passes occurring, the fuzzed session drains to finished"
        );
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
    fn acquire_current_records_the_card_unscheduled_without_a_review() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id();
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 1000);
        assert!(session.current_unseen(&store)); // a fresh card is acquired, not quizzed

        session.acquire_current(&mut store, 1000);

        let state = store.get(id).expect("acquired card is recorded");
        assert!(state.recall.is_none(), "acquiring does not schedule under FSRS");
        assert_eq!(1000, state.acquired_ms, "acquire stamps the acquire time");
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
        let f = store.get(id).unwrap().recall.unwrap();
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
                target: crate::level::Level::default(),
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
        store.get_or_insert(all[0].id(), 0);
        store.get_or_insert(all[1].id(), 0);

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
        store.get_or_insert(all[0].id(), 0);
        store.get_or_insert(all[1].id(), 0);

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
        store.get_or_insert(all[0].id(), now);

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
                target: crate::level::Level::default(),
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
        assert!(state.recall.is_some());
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
        store.get_or_insert(c.id(), 0).recall = Some(retired_fsrs());
        assert!(is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
        // Just below the cap: still in rotation.
        store.get_or_insert(c.id(), 0).recall = Some(crate::store::FsrsState {
            scheduled_days: DEFAULT_RETIRE_AFTER_DAYS - 1,
            ..Default::default()
        });
        assert!(!is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
        // A legacy card at the top Leitner stage but with no FSRS state is no longer
        // retired — retirement now needs a grown FSRS interval, not a stage.
        let s = store.get_or_insert(c.id(), 0);
        s.recall = None;
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
        s.streak = 1;
        s.acquired_ms = now;
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
        store.get_or_insert(c.id(), now).recall = Some(retired_fsrs());
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
        store.get_or_insert(all[0].id(), 0).recall = Some(retired_fsrs()); // retired

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
                target: crate::level::Level::default(),
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
        store.get_or_insert(all[0].id(), 0).recall = Some(crate::store::FsrsState {
            stability: 30.0,
            difficulty: 5.0,
            scheduled_days: 30,
            state: 2,
            due_ms: 1000,
            ..Default::default()
        });
        let before = store.get(all[0].id()).unwrap().recall.unwrap();

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
        let f = after.recall.unwrap();
        assert_eq!(before.stability, f.stability); // no reward
        assert_eq!(before.scheduled_days, f.scheduled_days); // interval kept
        assert_eq!(10_000u64 + 30 * 86_400_000, f.due_ms); // re-anchored to now + interval
        assert!(after.history.is_empty()); // a refresh, not a recorded review
    }

    #[test]
    fn cram_miss_lapses_normally() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(all[0].id(), 0).recall = Some(crate::store::FsrsState {
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
        assert!(after.recall.unwrap().stability < 30.0);
    }

    #[test]
    fn cram_serves_each_card_once() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let id_a = all[0].id();
        for c in &all {
            store.get_or_insert(c.id(), 0).recall = Some(crate::store::FsrsState {
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
        store.get_or_insert(all[1].id(), now);
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
        store.get_or_insert(all[0].id(), 0).recall = Some(retired_fsrs()); // retired
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
    fn card_strength_is_full_right_after_review_and_decays() {
        let (mut store, _dir) = empty_store();
        let id = 42;
        store.get_or_insert(id, 0).recall = Some(FsrsState {
            stability: 10.0,
            state: 2,
            ..Default::default()
        });
        let ten_days_ms = 10 * 86_400_000;
        let fresh = card_strengths(&[id], &store, 0);
        let later = card_strengths(&[id], &store, ten_days_ms);
        assert!(fresh[0] > 0.99, "R≈1 right after review, got {}", fresh[0]);
        assert!(
            later[0] < fresh[0] && later[0] > 0.0,
            "R should decay with elapsed time, got {} then {}",
            fresh[0],
            later[0]
        );
    }

    #[test]
    fn card_with_no_fsrs_has_zero_strength() {
        let (store, _dir) = empty_store();
        assert_eq!(vec![0.0], card_strengths(&[7], &store, 0));
        assert!(card_strengths(&[], &store, 0).is_empty());
    }

    #[test]
    fn virtual_card_joins_the_roster_and_is_served() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.txt", "virtual back", 0);
        let now = 60_000; // past the stage-1 cooldown: due
        let session = Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);
        assert_eq!(1, session.initial_size);
        assert_eq!("virtual front", session.current().unwrap().front);
        assert!(session.current_is_virtual(&store));
    }

    #[test]
    fn grading_a_virtual_card_updates_store_cards() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.txt", "virtual back", 0);
        let id = synth.id();
        let now = 60_000;
        let mut session =
            Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);

        session.grade(&mut store, Grade::Pass, now);

        // A virtual card's schedule now lives in `store.cards`, keyed by its
        // own `Card::id` — the same entry a deck card would use.
        let state = store.get(id).expect("virtual schedule in store.cards");
        assert!(state.recall.is_some());
        assert_eq!(1, state.total_reviews);
        // Still virtual (sidecar membership), not promoted.
        assert!(store.is_virtual(id));
    }

    #[test]
    fn virtual_card_not_treated_as_unseen() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.txt", "virtual back", 0);
        let now = 60_000;
        let session = Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);
        assert!(!session.current_unseen(&store));
    }

    #[test]
    fn a_missed_virtual_card_reappears_on_its_fsrs_due() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.txt", "virtual back", 0);
        let synth_id = synth.id();
        let deck_card = card("deck.txt", 0);
        store.get_or_insert(deck_card.id(), 0);

        let now = 5 * 60 * 1000; // both due (past the stage-1 cooldown)
        let mut session = Session::new(
            vec![synth, deck_card],
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
        insert_virtual(&mut store, "deck.txt", "gap-due", 0);
        // Not yet due: created at `now`, still cooling down.
        insert_virtual(&mut store, "deck.txt", "gap-not-due", now);
        // Archived: its interval already sits at the cap, so it's excluded —
        // derived from `store.cards`, no stored flag.
        let archived = insert_virtual(&mut store, "deck.txt", "gap-archived", 0);
        store.get_or_insert(archived.id(), 0).recall = Some(retired_fsrs());

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
        let synth = insert_virtual(&mut store, "deck.txt", "virtual back", 1000);
        let session = Session::new(
            vec![synth],
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        // The virtual card's schedule (stage 1 @ t=1000) has an FSRS fallback
        // due of 1000 + the stage-1 cooldown.
        let due = session
            .next_due_at(&store)
            .expect("a virtual card's due time is reported");
        assert_eq!(1000 + 60_000, due);
    }

    #[test]
    fn virtual_card_is_retired_when_interval_reaches_cap() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.txt", "virtual back", 0);
        let id = synth.id();
        let options = SessionOptions {
            retire_after_days: Some(4),
            ..SessionOptions::default()
        };

        // First real review: still acquiring (needs two Goods to graduate), so
        // the interval stays at 0 — well under the cap.
        let now = 60_000;
        let mut session = Session::new(vec![synth.clone()], &store, sched(), options.clone(), now);
        session.grade(&mut store, Grade::Pass, now);
        assert!(!is_retired_id(id, &store, options.retire_after_days));

        // Second review, once due: graduates to `Review` with a 4-day interval —
        // right at the cap. No stored flag anywhere — retirement is read fresh
        // from the interval in `store.cards`.
        let now = 86_460_000;
        let mut session = Session::new(vec![synth.clone()], &store, sched(), options.clone(), now);
        session.grade(&mut store, Grade::Pass, now);

        assert!(is_retired_id(id, &store, options.retire_after_days));
        let state = store.get(id).expect("schedule kept, not deleted");
        assert_eq!(4, state.recall.as_ref().unwrap().scheduled_days);
        assert_eq!(2, state.total_reviews);
        assert!(store.is_virtual(id)); // sidecar kept

        // Excluded from the queue and from due counts, same as a deck card.
        let session = Session::new(vec![synth], &store, sched(), options.clone(), now);
        assert!(session.is_finished());
        assert_eq!(
            0,
            count_reviewable_virtual(
                &store,
                "deck.txt",
                sched().as_ref(),
                now,
                options.retire_after_days
            )
        );
    }

    #[test]
    fn raising_retire_after_un_retires_a_virtual_card() {
        // The symmetry the derived model is for: a card archived at cap C is
        // not retired once the cap is raised above its interval — exactly
        // like a deck card. No stickiness from a stored flag.
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.txt", "virtual back", 0);
        let id = synth.id();
        store.get_or_insert(id, 0).recall = Some(crate::store::FsrsState {
            scheduled_days: 10,
            ..Default::default()
        });
        assert!(is_retired_id(id, &store, Some(10)));
        assert!(!is_retired_id(id, &store, Some(20)));

        // Confirmed through the exclusion path too: the same card is excluded
        // from due counts at the lower cap, and counted at the raised one.
        let now = 61_000;
        let sched = sched();
        assert_eq!(
            0,
            count_reviewable_virtual(&store, "deck.txt", sched.as_ref(), now, Some(10))
        );
        assert_eq!(
            1,
            count_reviewable_virtual(&store, "deck.txt", sched.as_ref(), now, Some(20))
        );
    }

    #[test]
    fn retired_virtual_card_is_excluded_from_queue_and_counts() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.txt", "virtual back", 0);
        let id = synth.id();
        store.get_or_insert(id, 0).recall = Some(retired_fsrs());

        // Otherwise past its stage-1 cooldown: would be due if not archived.
        let now = 61_000;
        let session = Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);
        assert!(session.is_finished()); // not served: no roster entry

        let cap = Some(DEFAULT_RETIRE_AFTER_DAYS);
        assert_eq!(
            0,
            count_reviewable_virtual(&store, "deck.txt", sched().as_ref(), now, cap)
        );
        // The sidecar entry itself survives — archived, not deleted.
        assert!(store.is_virtual(id));
    }

    #[test]
    fn retire_only_at_cap_not_below() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.txt", "virtual back", 0);
        let id = synth.id();

        let now = 60_000; // past the stage-1 cooldown
        let mut session =
            Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);
        session.grade(&mut store, Grade::Pass, now);

        assert!(!is_retired_id(id, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
    }

    // TODO(task 5): the climb/descent regression tests that used to live here
    // (`the_graduating_pass_does_not_climb_the_rung`,
    // `a_spaced_pass_after_graduation_climbs_the_rung`,
    // `a_cram_pass_never_climbs_or_resets_the_schedule`,
    // `a_reconstruction_miss_drops_to_recall`,
    // `a_recall_miss_stays_at_recall_without_wiping_schedule`,
    // `a_cram_fail_still_descends_but_a_cram_pass_does_not`,
    // `a_freshly_acquired_card_seeds_at_recall`) all asserted on `CardState::rung`,
    // deleted in Task 3 (session-owned levels replace card-owned rungs — there is
    // nothing left to persist a climb/descent into, so the assertions could no
    // longer express real behavior). Removed rather than left false-positive;
    // Task 5's own plan replaces them with level-routed tests
    // (`a_reconstruct_grade_never_touches_the_recall_schedule`,
    // `a_recall_drilled_deck_is_immediately_due_at_reconstruct`,
    // `recognize_marks_a_correct_pick_and_requeues_a_wrong_one`,
    // `recognize_queue_holds_only_unrecognized_cards`).
}
