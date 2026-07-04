//! Review session logic, independent of any UI.
//!
//! A session takes the cards of one or more decks, asks the store which are
//! due, builds a review queue and applies grades. Failed cards are re-queued at
//! the end of the session until they pass: the queue is served *by position*
//! (front of the queue), so a re-queued card comes up again in the same run
//! regardless of its cooldown. A never-seen card is *acquired* first — shown,
//! recorded at stage 1, then left for a later session to quiz.

use std::collections::VecDeque;

use crate::{
    augment::TopologyOrder,
    card::Card,
    scheduler::{Grade, Scheduler},
    store::{MAX_STAGE, Store},
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
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            max_new: 10,
            limit: None,
            cram: false,
            order: Order::Scheduled,
            topology: None,
        }
    }
}

/// Counters for a running session.
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionStats {
    /// Number of grades given (re-reviews of failed cards count again).
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
    /// Indices into `cards`, front = current card.
    queue: VecDeque<usize>,
    scheduler: Box<dyn Scheduler>,
    options: SessionOptions,
    /// Total distinct cards that entered the queue initially.
    pub initial_size: usize,
    /// Session counters.
    pub stats: SessionStats,
}

impl Session {
    /// Builds a session at time `now_ms`.
    ///
    /// The queue holds, in order: all due cards (earliest FSRS due first), then
    /// up to `max_new` unseen cards in deck order. Sub-cards of the same cloze
    /// card are kept apart whenever other cards are available.
    pub fn new(
        cards: Vec<Card>,
        store: &Store,
        scheduler: Box<dyn Scheduler>,
        options: SessionOptions,
        now_ms: u64,
    ) -> Self {
        let queue = build_queue(&cards, store, &*scheduler, &options, now_ms);
        let initial_size = queue.len();

        Self {
            cards,
            queue,
            scheduler,
            options,
            initial_size,
            stats: SessionStats::default(),
        }
    }

    /// Starts a fresh session over the same decks with the same settings,
    /// picking up whatever is due (or new) at `now_ms`.
    ///
    /// Returns `false` — leaving queue and stats untouched — if nothing is
    /// due, so a summary screen can keep showing the finished session.
    pub fn restart(&mut self, store: &Store, now_ms: u64) -> bool {
        let queue = build_queue(&self.cards, store, &*self.scheduler, &self.options, now_ms);
        if queue.is_empty() {
            return false;
        }
        self.initial_size = queue.len();
        self.queue = queue;
        self.stats = SessionStats::default();
        true
    }

    /// Whether a [`restart`](Self::restart) right now would find any cards —
    /// i.e. anything is due (or a new card can be introduced) at `now_ms`.
    /// Non-mutating; runs the same queue build `restart` would.
    pub fn has_due_now(&self, store: &Store, now_ms: u64) -> bool {
        !build_queue(&self.cards, store, &*self.scheduler, &self.options, now_ms).is_empty()
    }

    /// The earliest upcoming due time over all seen cards of this session's
    /// decks, if any.
    pub fn next_due_at(&self, store: &Store) -> Option<u64> {
        self.cards
            .iter()
            .filter_map(|card| store.get(card.id()))
            .map(|state| self.scheduler.due_at(state))
            .min()
    }

    /// The card currently up for review.
    pub fn current(&self) -> Option<&Card> {
        self.queue.front().map(|&i| &self.cards[i])
    }

    /// The current card, mutable — e.g. to attach a note just saved from the ask
    /// tutor so the card shows it without re-reading the deck file.
    pub fn current_mut(&mut self) -> Option<&mut Card> {
        let i = *self.queue.front()?;
        Some(&mut self.cards[i])
    }

    /// Whether the current card has never been seen (no stored progress). Such a
    /// card is *acquired* — shown via [`acquire_current`](Self::acquire_current) —
    /// rather than quizzed cold.
    pub fn current_unseen(&self, store: &Store) -> bool {
        self.current()
            .is_some_and(|card| store.get(card.id()).is_none())
    }

    /// All cards of this session's decks (e.g. as the distractor pool for
    /// multiple-choice questions).
    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    /// Number of cards still in the queue (including the current one).
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }

    /// `true` once every card in the queue has passed.
    pub fn is_finished(&self) -> bool {
        self.queue.is_empty()
    }

    /// Grades the current card, updates the store, and advances the queue.
    /// A failed card is moved to the back of the queue to be retried in this
    /// session.
    pub fn grade(&mut self, store: &mut Store, grade: Grade, now_ms: u64) {
        let Some(index) = self.queue.pop_front() else {
            return;
        };
        let card = &self.cards[index];
        let state = store.get_or_insert(card.id(), now_ms);
        self.scheduler.apply(state, grade, now_ms);
        // Safety net: keep the stage within the top (reaching `MAX_STAGE`
        // retires the card). The scheduler already caps a pass at `MAX_STAGE`.
        if state.stage > MAX_STAGE {
            state.stage = MAX_STAGE;
        }

        self.stats.reviews += 1;
        if grade.passed() {
            self.stats.passed += 1;
        } else {
            self.stats.failed += 1;
            self.queue.push_back(index);
        }
    }

    /// Introduces the current never-seen card: records it on the Leitner ladder
    /// at stage 1 and drops it from this session's queue. It is *not* graded and
    /// gets *no* history entry — acquiring is a first exposure, not a review. The
    /// card is not re-queued, so its first quiz comes on a later session, once the
    /// stage-1 relearn cooldown (~5 min) has passed. Does nothing on an empty queue.
    pub fn acquire_current(&mut self, store: &mut Store, now_ms: u64) {
        let Some(index) = self.queue.pop_front() else {
            return;
        };
        // `get_or_insert` creates the state at stage 1, due ~5 min out via the
        // stage-1 cooldown — no `scheduler.apply`, no recorded review.
        store.get_or_insert(self.cards[index].id(), now_ms);
        self.stats.acquired += 1;
    }

    /// Moves the current card to the back of the queue without grading it.
    pub fn skip(&mut self) {
        if let Some(index) = self.queue.pop_front() {
            self.queue.push_back(index);
        }
    }

    /// Drops the current card from the queue without grading it, along with any
    /// remaining cards in the same sibling group (cloze sub-cards of one source
    /// card) so a card marked for removal is not asked again in any form.
    /// Returns clones of every dropped card (the current one first), or an
    /// empty vec if the queue was empty. The store is left untouched;
    /// pruning the cards' progress is the caller's job once the deck file
    /// is rewritten.
    pub fn remove_current(&mut self) -> Vec<Card> {
        let Some(index) = self.queue.pop_front() else {
            return Vec::new();
        };
        let group = sibling_group(&self.cards[index]);
        let mut removed = vec![self.cards[index].clone()];
        let mut kept = VecDeque::with_capacity(self.queue.len());
        for &i in &self.queue {
            if sibling_group(&self.cards[i]) == group {
                removed.push(self.cards[i].clone());
            } else {
                kept.push_back(i);
            }
        }
        self.queue = kept;
        removed
    }

    /// Per-stage counts over all cards of this session's decks (stage 0 =
    /// never seen).
    pub fn stage_histogram(&self, store: &Store) -> StageHistogram {
        histogram(&self.cards, store)
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
            // A retired card rests until `alix reset` — never scheduled, not
            // even under cram.
            Some(_) if is_retired(card, store) => {}
            Some(state) => {
                if options.cram || scheduler.is_due(state, now_ms) {
                    due.push(i);
                }
            }
            None => fresh.push(i),
        }
    }

    // Order due cards by their FSRS due time, earliest first.
    due.sort_by_key(|&i| store.get(cards[i].id()).map_or(u64::MAX, |s| scheduler.due_at(s)));

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
const RETIRE_AFTER_DAYS: u32 = 365;

/// Whether a card is *retired* (resting), so it is no longer scheduled until
/// `alix reset`: its FSRS interval has grown past [`RETIRE_AFTER_DAYS`]. A card with
/// no FSRS state yet — unseen, or a legacy card not yet reviewed under FSRS — is
/// never retired; its first FSRS review is what can push the interval past the cap.
pub fn is_retired(card: &Card, store: &Store) -> bool {
    is_retired_id(card.id(), store)
}

/// Id-only variant of [`is_retired`], so callers that hold an id but not the
/// [`Card`] (e.g. a trace checkpoint) share the one retirement rule.
pub fn is_retired_id(card_id: u64, store: &Store) -> bool {
    store
        .get(card_id)
        .and_then(|s| s.fsrs.as_ref())
        .is_some_and(|f| f.scheduled_days >= RETIRE_AFTER_DAYS)
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
pub fn is_reviewable(card: &Card, store: &Store, scheduler: &dyn Scheduler, now_ms: u64) -> bool {
    match store.get(card.id()) {
        Some(_) if is_retired(card, store) => false,
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
) -> bool {
    cards
        .iter()
        .any(|card| is_reviewable(card, store, scheduler, now_ms))
}

/// How many of these cards would be served right now — the due/new count for a
/// region or a whole deck (shown in the focus drawer). See [`is_reviewable`].
pub fn count_reviewable(
    cards: &[&Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    now_ms: u64,
) -> usize {
    cards
        .iter()
        .filter(|card| is_reviewable(card, store, scheduler, now_ms))
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
        let mut session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        assert!(session.current_unseen(&store)); // a fresh card is acquired, not quizzed

        session.acquire_current(&mut store, 1000);

        let state = store.get(id).expect("acquired card is recorded");
        assert_eq!(1, state.stage);
        assert!(state.history.is_empty()); // acquiring is not a review
        assert_eq!(0, state.total_reviews);
        assert_eq!(1, session.stats.acquired);
        assert_eq!(0, session.stats.reviews);
        assert!(session.is_finished()); // dropped from the queue, not re-queued
    }

    #[test]
    fn acquired_cards_are_not_due_until_the_relearn_cooldown() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(1),
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        session.acquire_current(&mut store, 1000);

        // Just acquired: a restart right away finds nothing due (the 5-min gap),
        // so it cannot be quizzed the instant it was seen.
        assert!(!session.has_due_now(&store, 1000));
        assert!(!session.has_due_now(&store, 1000 + 5 * 60 * 1000 - 1));
        // Once the relearn cooldown passes, a fresh session quizzes it.
        assert!(session.has_due_now(&store, 1000 + 5 * 60 * 1000));
    }

    #[test]
    fn failed_card_reappears_in_the_same_session_despite_the_cooldown() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id();
        store.get_or_insert(id, 0); // already seen, stage 1
        let now = 5 * 60 * 1000; // past the relearn cooldown, so it is due
        let mut session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions::default(),
            now,
        );
        assert_eq!(1, session.remaining());

        session.grade(&mut store, Grade::Fail, now);

        // Failing pushes the card to the back of the queue; serving is by
        // position, so it returns this same session even though its due time is
        // now ~5 min out.
        assert_eq!(1, session.remaining());
        assert!(!session.is_finished());
        assert_eq!(id, session.current().unwrap().id());
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
            },
            now + 1,
        );
        assert_eq!(1, session.initial_size);
    }

    #[test]
    fn failed_card_is_requeued_until_passed() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(2),
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        assert_eq!(2, session.remaining());

        let first = session.current().unwrap().front.clone();
        session.grade(&mut store, Grade::Fail, 1000);
        assert_eq!(2, session.remaining()); // still two: failed card requeued

        session.grade(&mut store, Grade::Pass, 1001);
        assert_eq!(1, session.remaining());
        // The failed card came back.
        assert_eq!(first, session.current().unwrap().front);
        session.grade(&mut store, Grade::Pass, 1002);
        assert!(session.is_finished());

        assert_eq!(3, session.stats.reviews);
        assert_eq!(2, session.stats.passed);
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
        let mut session = Session::new(
            cards(2),
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        let first = session.current().unwrap().front.clone();
        session.skip();
        assert_ne!(first, session.current().unwrap().front);
        assert_eq!(2, session.remaining());
        session.skip();
        assert_eq!(first, session.current().unwrap().front);
        // Skipping must not touch the store.
        assert!(store.is_empty());
        let _ = &mut store;
    }

    #[test]
    fn remove_current_drops_card_without_grading() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(2),
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        let removed = session.remove_current();
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
        let mut session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions::default(),
            0,
        );
        assert_eq!(3, session.remaining());
        // Removing one sub-card removes its sibling too, leaving only card 2.
        let removed = session.remove_current();
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
        let mut session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions::default(),
            0,
        );

        let mut fronts = Vec::new();
        for _ in 0..session.remaining() {
            fronts.push(session.current().unwrap().front.clone());
            session.skip();
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
        let session = Session::new(
            all,
            &store,
            sched(),
            SessionOptions::default(),
            0,
        );
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
        let mut session = Session::new(
            cards(1),
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
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
        let mut session = Session::new(
            cards(1),
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
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
        let due = session.next_due_at(&store).expect("a seen card has a due time");
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
        crate::store::FsrsState { scheduled_days: RETIRE_AFTER_DAYS, ..Default::default() }
    }

    #[test]
    fn is_retired_once_the_interval_passes_the_cap() {
        let (mut store, _dir) = empty_store();
        let c = card("deck.txt", 0);

        assert!(!is_retired(&c, &store)); // unseen — never retired
        // An FSRS interval at/past the cap rests.
        store.get_or_insert(c.id(), 0).fsrs = Some(retired_fsrs());
        assert!(is_retired(&c, &store));
        // Just below the cap: still in rotation.
        store.get_or_insert(c.id(), 0).fsrs = Some(crate::store::FsrsState {
            scheduled_days: RETIRE_AFTER_DAYS - 1,
            ..Default::default()
        });
        assert!(!is_retired(&c, &store));
        // A legacy card at the top Leitner stage but with no FSRS state is no longer
        // retired — retirement now needs a grown FSRS interval, not a stage.
        let s = store.get_or_insert(c.id(), 0);
        s.fsrs = None;
        s.stage = MAX_STAGE;
        s.streak = 1;
        assert!(!is_retired(&c, &store));
    }

    #[test]
    fn has_reviewable_counts_new_and_due_not_cooldown_or_retired() {
        let (mut store, _dir) = empty_store();
        let sched = sched();
        let now = 10_000_000;

        // A brand-new (unseen) card is reviewable.
        assert!(has_reviewable(&cards(1), &store, sched.as_ref(), now));

        // A card just passed to stage 2 at `now` is on cooldown (due in 1h):
        // not reviewable now, reviewable once its due time arrives.
        let c = card("deck.txt", 0);
        let s = store.get_or_insert(c.id(), now);
        s.stage = 2;
        s.streak = 1;
        s.stage_entered_ms = now;
        let one = std::slice::from_ref(&c);
        assert!(!has_reviewable(one, &store, sched.as_ref(), now));
        assert!(has_reviewable(one, &store, sched.as_ref(), now + 3_600_000));

        // A retired card (FSRS interval past the cap) never counts, even past due.
        store.get_or_insert(c.id(), now).fsrs = Some(retired_fsrs());
        assert!(!has_reviewable(
            std::slice::from_ref(&c),
            &store,
            sched.as_ref(),
            now + 3_600_000
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
            },
            1000,
        );
        // Resting: not queued, even though cram ignores cooldowns.
        assert!(session.is_finished());
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
}
