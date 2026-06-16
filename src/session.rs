//! Review session logic, independent of any UI.
//!
//! A session takes the cards of one or more decks, asks the store which are
//! due, builds a review queue and applies grades. Failed cards are re-queued
//! at the end of the session until they pass (a failed card drops to stage 1,
//! whose cooldown is 0, so it comes up again in the same run).

use std::collections::VecDeque;

use crate::{
    card::Card,
    scheduler::{Grade, Scheduler, SchedulerKind},
    store::Store,
    time,
};

/// The order in which the due/new cards of a session are presented.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Order {
    /// The scheduler decides the order (Leitner: higher stages first; SM-2:
    /// earliest due first), then up to `max_new` new cards.
    #[default]
    Scheduled,
    /// Present the cards in deck/file order, top to bottom — useful for
    /// memorizing something with an inherent sequence, like song lyrics.
    Sequential,
}

/// Options controlling which cards enter a session and in what order.
#[derive(Clone, Copy, Debug)]
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
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            max_new: 10,
            limit: None,
            cram: false,
            order: Order::Scheduled,
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
}

/// Per-stage card counts for the loaded decks; index 0 holds unseen cards.
pub type StageHistogram = [usize; 6];

/// A review session over a fixed set of cards.
pub struct Session {
    cards: Vec<Card>,
    /// Indices into `cards`, front = current card.
    queue: VecDeque<usize>,
    scheduler: Box<dyn Scheduler>,
    kind: SchedulerKind,
    options: SessionOptions,
    /// Per-card dependency rank (lower = more foundational), parallel to
    /// `cards`. Empty when no deck dependencies are in play. The queue is
    /// stably ordered by it, so prerequisite decks' cards come first.
    dep_ranks: Vec<usize>,
    /// Total distinct cards that entered the queue initially.
    pub initial_size: usize,
    /// Session counters.
    pub stats: SessionStats,
}

impl Session {
    /// Builds a session at time `now_ms`.
    ///
    /// The queue holds, in order: all due cards (for Leitner, higher stages
    /// first; for SM-2, earliest due first), then up to `max_new` unseen cards
    /// in deck order. Sub-cards of the same cloze card are kept apart whenever
    /// other cards are available.
    pub fn new(
        cards: Vec<Card>,
        store: &Store,
        kind: SchedulerKind,
        options: SessionOptions,
        now_ms: u64,
    ) -> Self {
        Self::new_with_deps(cards, store, kind, options, Vec::new(), now_ms)
    }

    /// Like [`new`](Self::new) but with per-card dependency ranks: the queue is
    /// additionally ordered so cards of lower-ranked (prerequisite) decks come
    /// before higher-ranked ones, keeping scheduler order within each rank.
    /// `dep_ranks` is parallel to `cards`; an empty slice means no ordering.
    pub fn new_with_deps(
        cards: Vec<Card>,
        store: &Store,
        kind: SchedulerKind,
        options: SessionOptions,
        dep_ranks: Vec<usize>,
        now_ms: u64,
    ) -> Self {
        let scheduler = kind.scheduler();
        let queue = build_queue(
            &cards,
            store,
            &*scheduler,
            kind,
            options,
            &dep_ranks,
            now_ms,
        );
        let initial_size = queue.len();

        Self {
            cards,
            queue,
            scheduler,
            kind,
            options,
            dep_ranks,
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
        let queue = build_queue(
            &self.cards,
            store,
            &*self.scheduler,
            self.kind,
            self.options,
            &self.dep_ranks,
            now_ms,
        );
        if queue.is_empty() {
            return false;
        }
        self.initial_size = queue.len();
        self.queue = queue;
        self.stats = SessionStats::default();
        true
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

        self.stats.reviews += 1;
        if grade.passed() {
            self.stats.passed += 1;
        } else {
            self.stats.failed += 1;
            self.queue.push_back(index);
        }
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
    /// Returns clones of every dropped card (the current one first), or an empty
    /// vec if the queue was empty. The store is left untouched; pruning the
    /// cards' progress is the caller's job once the deck file is rewritten.
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
}

/// Builds the review queue: due cards in scheduler order, then up to
/// `max_new` unseen cards, capped by `limit`, with cloze siblings separated.
fn build_queue(
    cards: &[Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    kind: SchedulerKind,
    options: SessionOptions,
    dep_ranks: &[usize],
    now_ms: u64,
) -> VecDeque<usize> {
    let mut due: Vec<usize> = Vec::new();
    let mut fresh: Vec<usize> = Vec::new();

    for (i, card) in cards.iter().enumerate() {
        match store.get(card.id()) {
            Some(state) => {
                if options.cram || scheduler.is_due(state, now_ms) {
                    due.push(i);
                }
            }
            None => fresh.push(i),
        }
    }

    // Order due cards.
    match kind {
        SchedulerKind::Leitner => {
            // Higher stages first, then by how long they have been waiting.
            due.sort_by_key(|&i| {
                let state = store.get(cards[i].id()).unwrap();
                (std::cmp::Reverse(state.stage), state.stage_entered_ms)
            });
        }
        SchedulerKind::Sm2 => {
            due.sort_by_key(|&i| {
                let state = store.get(cards[i].id()).unwrap();
                scheduler.due_at(state)
            });
        }
    }

    let mut order: Vec<usize> = due;
    order.extend(fresh.into_iter().take(options.max_new));
    if options.order == Order::Sequential {
        // Card indices follow deck/file order, so sorting restores it while
        // keeping the due/new selection above.
        order.sort_unstable();
    }
    // Order-first by dependency rank: a stable sort keeps the scheduler order
    // within each deck while moving prerequisite decks ahead of dependents.
    if !dep_ranks.is_empty() {
        order.sort_by_key(|&i| dep_ranks[i]);
    }
    if let Some(limit) = options.limit {
        order.truncate(limit);
    }
    separate_siblings(order, cards)
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

    #[test]
    fn new_cards_enter_up_to_max_new() {
        let (store, _dir) = empty_store();
        let session = Session::new(
            cards(20),
            &store,
            SchedulerKind::Leitner,
            SessionOptions {
                max_new: 5,
                ..Default::default()
            },
            1000,
        );
        assert_eq!(5, session.initial_size);
    }

    #[test]
    fn due_cards_take_priority_over_new_under_limit() {
        let (mut store, _dir) = empty_store();
        let all = cards(10);
        // Cards 7, 8, 9 were seen and are due (stage 1, cooldown 0).
        for c in &all[7..] {
            store.get_or_insert(c.id(), 0);
        }
        let session = Session::new(
            all.clone(),
            &store,
            SchedulerKind::Leitner,
            SessionOptions {
                max_new: 10,
                limit: Some(3),
                cram: false,
                order: Order::Scheduled,
            },
            1000,
        );
        assert_eq!(3, session.initial_size);
        // The queue holds exactly the due cards, not the new ones.
        assert_eq!("front 7", session.current().unwrap().front);
    }

    #[test]
    fn leitner_orders_higher_stages_first() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        // Card 0: stage 2, entered long ago -> due. Card 1: stage 5, entered
        // long ago -> due. Card 2: new.
        store.get_or_insert(all[0].id(), 0).stage = 2;
        store.get_or_insert(all[1].id(), 0).stage = 5;

        let now = 2 * 604_800_000; // two weeks later, everything is due
        let mut session = Session::new(
            all,
            &store,
            SchedulerKind::Leitner,
            SessionOptions::default(),
            now,
        );
        assert_eq!("front 1", session.current().unwrap().front); // stage 5
        session.grade(&mut store, Grade::Pass, now);
        assert_eq!("front 0", session.current().unwrap().front); // stage 2
        session.grade(&mut store, Grade::Pass, now);
        assert_eq!("front 2", session.current().unwrap().front); // new
    }

    #[test]
    fn sequential_order_follows_deck_order() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        // Same setup as the Leitner test: by stage, card 1 (s5) would lead,
        // then card 0 (s2), then the new card 2. Sequential ignores that.
        store.get_or_insert(all[0].id(), 0).stage = 2;
        store.get_or_insert(all[1].id(), 0).stage = 5;

        let now = 2 * 604_800_000;
        let mut session = Session::new(
            all,
            &store,
            SchedulerKind::Leitner,
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
    fn dependency_rank_orders_prerequisites_first() {
        let (mut store, _dir) = empty_store();
        // First two cards belong to the dependent deck (rank 1), last two to
        // the prerequisite deck (rank 0). All are new.
        let all = vec![
            card("adv.txt", 0),
            card("adv.txt", 1),
            card("basics.txt", 2),
            card("basics.txt", 3),
        ];
        let ranks = vec![1, 1, 0, 0];
        let mut session = Session::new_with_deps(
            all,
            &store,
            SchedulerKind::Leitner,
            SessionOptions {
                max_new: 10,
                ..Default::default()
            },
            ranks,
            1000,
        );
        // The prerequisite deck's cards come first despite appearing later.
        assert_eq!("basics.txt", &*session.current().unwrap().subject);
        session.grade(&mut store, Grade::Pass, 1000);
        assert_eq!("basics.txt", &*session.current().unwrap().subject);
        session.grade(&mut store, Grade::Pass, 1000);
        assert_eq!("adv.txt", &*session.current().unwrap().subject);
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
            SchedulerKind::Leitner,
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
            SchedulerKind::Leitner,
            SessionOptions {
                max_new: 0,
                limit: None,
                cram: true,
                order: Order::Scheduled,
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
            SchedulerKind::Leitner,
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
    fn grading_updates_store_stages() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id();
        let mut session = Session::new(
            all,
            &store,
            SchedulerKind::Leitner,
            SessionOptions::default(),
            1000,
        );

        session.grade(&mut store, Grade::Pass, 1000);
        // New card starts at stage 1; a pass moves it to stage 2.
        assert_eq!(2, store.get(id).unwrap().stage);
    }

    #[test]
    fn skip_rotates_queue() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(
            cards(2),
            &store,
            SchedulerKind::Leitner,
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
            SchedulerKind::Leitner,
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
        let mut all = vec![card("deck.txt", 1), card("deck.txt", 1), card("deck.txt", 2)];
        all[0].back = vec!["hole a".into()];
        all[1].back = vec!["hole b".into()];
        let mut session = Session::new(
            all,
            &store,
            SchedulerKind::Leitner,
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
            SchedulerKind::Leitner,
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
            SchedulerKind::Leitner,
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
            SchedulerKind::Leitner,
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
            SchedulerKind::Leitner,
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
    fn next_due_at_reports_earliest_due_time() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        // Stage 2 entered at t=1000 -> due at 1000 + 1h.
        let mut session = Session::new(
            all,
            &store,
            SchedulerKind::Leitner,
            SessionOptions::default(),
            1000,
        );
        assert_eq!(None, session.next_due_at(&store)); // nothing seen yet
        session.grade(&mut store, Grade::Pass, 1000);
        assert_eq!(Some(1000 + 3_600_000), session.next_due_at(&store));
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
}
