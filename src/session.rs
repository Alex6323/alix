use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
};

use rs_fsrs::Parameters;

use crate::{
    augment::TopologyOrder,
    card::Card,
    depth::Depth,
    scheduler::{Grade, Scheduler},
    store::{Store, VirtualCard},
    time,
    trace::SourceBase,
};

pub struct DeckInfo {
    pub path: PathBuf,
    pub deck_token: Option<String>,
    pub links: Vec<String>,
    pub source_root: Option<PathBuf>,
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

#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub max_new: usize,
    pub limit: Option<usize>,
    pub cram: bool,
    pub order: Order,
    pub topology: Option<TopologyOrder>,
    pub retire_after_days: Option<u32>,
    pub depth: Depth,
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
            depth: Depth::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SessionStats {
    pub reviews: usize,
    pub passed: usize,
    pub failed: usize,
    pub acquired: usize,
}

pub struct Session {
    cards: Vec<Card>,
    roster: Vec<usize>,
    current_idx: Option<usize>,
    remaining_now: usize,
    floors: HashMap<String, u64>,
    appearances: Vec<u32>,
    scheduler: Box<dyn Scheduler>,
    options: SessionOptions,
    pub initial_size: usize,
    pub stats: SessionStats,
}

impl Session {
    pub fn new(
        cards: Vec<Card>,
        store: &Store,
        scheduler: Box<dyn Scheduler>,
        options: SessionOptions,
        now_ms: u64,
    ) -> Self {
        let roster: Vec<usize> = build_queue(&cards, store, &*scheduler, &options, now_ms).into();
        let initial_size = roster.len();
        let appearances = vec![0; cards.len()];

        let mut session = Self {
            cards,
            roster,
            current_idx: None,
            remaining_now: 0,
            floors: HashMap::new(),
            appearances,
            scheduler,
            options,
            initial_size,
            stats: SessionStats::default(),
        };
        session.advance(store, now_ms);
        session
    }

    pub fn restart(&mut self, store: &Store, now_ms: u64) -> bool {
        let roster: Vec<usize> =
            build_queue(&self.cards, store, &*self.scheduler, &self.options, now_ms).into();
        if roster.is_empty() {
            return false;
        }
        self.initial_size = roster.len();
        self.roster = roster;
        self.stats = SessionStats::default();
        self.floors.clear();
        self.advance(store, now_ms);
        true
    }

    pub fn has_due_now(&self, store: &Store, now_ms: u64) -> bool {
        !build_queue(&self.cards, store, &*self.scheduler, &self.options, now_ms).is_empty()
    }

    pub fn next_due_at(&self, store: &Store) -> Option<u64> {
        if self.options.depth == Depth::Recognize {
            return None;
        }
        self.cards
            .iter()
            .filter_map(|c| c.id())
            .filter_map(|id| store.get(&id))
            .map(|state| self.scheduler.due_at(state, self.options.depth))
            .min()
    }

    pub fn depth(&self) -> Depth {
        self.options.depth
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

    pub fn current_is_virtual(&self, store: &Store) -> bool {
        self.current()
            .and_then(Card::id)
            .is_some_and(|id| store.is_virtual(&id))
    }

    pub fn current_unseen(&self, store: &Store) -> bool {
        self.current()
            .and_then(Card::id)
            .is_some_and(|id| store.get(&id).is_none())
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

    pub fn grade(&mut self, store: &mut Store, grade: Grade, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        let Some(id) = self.cards[index].id() else {
            self.advance(store, now_ms);
            return;
        };
        let depth = self.options.depth;

        let state = store.get_or_insert(&id, now_ms);
        if grade == Grade::Pass && state.recognized_ms.is_none() {
            state.recognized_ms = Some(now_ms);
        }

        if depth == Depth::Recognize {
            state.record_review(now_ms, grade, Depth::Recognize, false);
            if grade == Grade::Pass {
                self.roster
                    .retain(|&i| self.cards[i].id().as_deref() != Some(id.as_str()));
            }
            self.floor(&id, now_ms);
            self.advance(store, now_ms);
            return;
        }

        let was_due = self.scheduler.is_due(state, depth, now_ms);
        if self.options.cram && grade.passed() && !was_due {
            self.scheduler.reanchor(state, depth, now_ms);
        } else {
            self.scheduler.apply(state, depth, grade, now_ms, false);
        }

        if depth == Depth::Reconstruct
            && grade == Grade::Pass
            && (!self.options.cram || was_due)
            && state.recall.is_some()
        {
            if self.scheduler.is_due(state, Depth::Recall, now_ms) {
                self.scheduler
                    .apply(state, Depth::Recall, Grade::Pass, now_ms, true);
            } else {
                self.scheduler.reanchor(state, Depth::Recall, now_ms);
            }
        }

        self.stats.reviews += 1;
        let passed = grade.passed();
        if passed {
            self.stats.passed += 1;
        } else {
            self.stats.failed += 1;
        }
        if passed || self.options.cram {
            self.roster.retain(|&i| i != index);
        }
        self.floor(&id, now_ms);
        self.advance(store, now_ms);
    }

    pub fn acquire_current(&mut self, store: &mut Store, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        let Some(id) = self.cards[index].id() else {
            self.advance(store, now_ms);
            return;
        };
        store.get_or_insert(&id, now_ms);
        self.stats.acquired += 1;
        self.floor(&id, now_ms);
        self.advance(store, now_ms);
    }

    pub fn skip(&mut self, store: &Store, now_ms: u64) {
        let Some(index) = self.current_idx else {
            return;
        };
        self.roster.retain(|&i| i != index);
        self.roster.push(index);
        self.advance(store, now_ms);
    }

    pub fn remove_current(&mut self, store: &Store, now_ms: u64) -> Vec<Card> {
        let Some(index) = self.current_idx else {
            return Vec::new();
        };
        let group = sibling_group(&self.cards[index]);
        let mut removed = vec![self.cards[index].clone()];
        let mut kept: Vec<usize> = Vec::with_capacity(self.roster.len());
        for &i in &self.roster {
            if i == index {
                continue;
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

    pub fn poll(&mut self, store: &Store, now_ms: u64) -> bool {
        self.advance(store, now_ms);
        self.current_idx.is_some()
    }

    fn servable(&self, i: usize, store: &Store, now_ms: u64) -> bool {
        let card = &self.cards[i];
        if is_retired(card, store, self.options.retire_after_days) {
            return false;
        }
        let Some(id) = card.id() else {
            return false;
        };
        let depth = self.options.depth;
        let due = if depth == Depth::Recognize {
            self.options.cram || store.get(&id).is_none_or(|s| s.recognized_ms.is_none())
        } else {
            match store.get(&id) {
                Some(state) => self.options.cram || self.scheduler.is_due(state, depth, now_ms),
                None => true,
            }
        };
        if !due {
            return false;
        }
        match self.floors.get(&id) {
            Some(&transition_ms) => {
                now_ms >= transition_ms.saturating_add(self.scheduler.acquire_cooldown_ms())
            }
            None => true,
        }
    }

    fn floor(&mut self, id: &str, now_ms: u64) {
        let cooldown_ms = self.scheduler.acquire_cooldown_ms();
        self.floors
            .retain(|_, &mut t| now_ms < t.saturating_add(cooldown_ms));
        self.floors.insert(id.to_string(), now_ms);
    }

    fn advance(&mut self, store: &Store, now_ms: u64) {
        let next = self
            .roster
            .iter()
            .copied()
            .find(|&i| self.servable(i, store, now_ms));
        if let Some(i) = next
            && next != self.current_idx
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
    }
}

fn build_queue(
    cards: &[Card],
    store: &Store,
    scheduler: &dyn Scheduler,
    options: &SessionOptions,
    now_ms: u64,
) -> VecDeque<usize> {
    if options.depth == Depth::Recognize {
        let mut order: Vec<usize> = (0..cards.len())
            .filter(|&i| !is_retired(&cards[i], store, options.retire_after_days))
            .filter(|&i| {
                options.cram
                    || cards[i]
                        .id()
                        .and_then(|id| store.get(&id))
                        .is_none_or(|s| s.recognized_ms.is_none())
            })
            .collect();
        if let Some(limit) = options.limit {
            order.truncate(limit);
        }
        return separate_siblings(order, cards);
    }

    let depth = options.depth;
    let mut due: Vec<usize> = Vec::new();
    let mut fresh: Vec<usize> = Vec::new();

    for (i, card) in cards.iter().enumerate() {
        match card.id().and_then(|id| store.get(&id)) {
            Some(_) if is_retired(card, store, options.retire_after_days) => {}
            Some(state) => {
                if options.cram || scheduler.is_due(state, depth, now_ms) {
                    due.push(i);
                }
            }
            None => fresh.push(i),
        }
    }

    due.sort_by_key(|&i| {
        cards[i]
            .id()
            .and_then(|id| store.get(&id))
            .map_or(u64::MAX, |s| scheduler.due_at(s, depth))
    });

    let mut fresh: Vec<usize> = fresh.into_iter().take(options.max_new).collect();

    if let Some(topo) = &options.topology {
        let rank = |&i: &usize| {
            cards[i]
                .id()
                .as_deref()
                .and_then(|id| topo.rank_of(id))
                .unwrap_or(usize::MAX)
        };
        due.sort_by_key(rank);
        fresh.sort_by_key(rank);
    }

    let mut order: Vec<usize> = due;
    order.extend(fresh);

    if options.order == Order::Sequential {
        order.sort_unstable();
    }
    if let Some(limit) = options.limit {
        order.truncate(limit);
    }

    if options.topology.is_some() {
        order.into()
    } else {
        separate_siblings(order, cards)
    }
}

fn sibling_group(card: &Card) -> (&str, usize) {
    (card.subject.as_ref(), card.line)
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

pub fn is_virtual_reviewable(
    vc: &VirtualCard,
    store: &Store,
    scheduler: &dyn Scheduler,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> bool {
    !is_retired_id(&vc.id, store, retire_after_days)
        && store
            .get(&vc.id)
            .is_some_and(|s| scheduler.is_due(s, Depth::Recall, now_ms))
}

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
        .filter(|vc| !is_retired_id(&vc.id, store, retire_after_days))
        .filter(|vc| {
            store.get(&vc.id).is_some_and(|s| {
                let due = scheduler.due_at(s, Depth::Recall);
                due > now_ms && due <= now_ms + window_ms
            })
        })
        .count()
}

pub fn has_graduated(card: &Card, store: &Store) -> bool {
    card.id()
        .and_then(|id| store.get(&id))
        .and_then(|s| s.schedule(Depth::Recall))
        .is_some_and(|f| f.graduated())
}

pub fn card_strengths(card_ids: &[String], store: &Store, now_ms: u64) -> Vec<f32> {
    card_ids
        .iter()
        .map(|id| retrievability(store, id, now_ms))
        .collect()
}

fn retrievability(store: &Store, card_id: &str, now_ms: u64) -> f32 {
    let Some(f) = store.get(card_id).and_then(|s| s.schedule(Depth::Recall)) else {
        return 0.0;
    };
    if f.stability <= 0.0 {
        return 0.0;
    }
    let elapsed_days = now_ms.saturating_sub(f.last_review_ms) as f64 / 86_400_000.0;
    Parameters::forgetting_curve(elapsed_days, f.stability).clamp(0.0, 1.0) as f32
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
    let Some(id) = card.id() else {
        return false;
    };
    if depth == Depth::Recognize {
        return store.get(&id).is_none_or(|s| s.recognized_ms.is_none());
    }
    match store.get(&id) {
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
        scheduler::DEFAULT_ACQUIRE_COOLDOWN_MS,
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

    fn insert_virtual(store: &mut Store, parent: &str, back: &str, created_ms: u64) -> Card {
        let slug: String = back
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let text = format!("## virtual front <!-- id: v{slug} -->\n{back}\n");
        let mut card = crate::l1::parse_str(parent, &text).unwrap().remove(0);
        card.line = 1_000_000;
        let id = card.id().unwrap();
        store.insert_virtual(VirtualCard {
            id: id.clone(),
            kind: crate::store::VirtualKind::Remediation,
            parent: parent.to_string(),
            text,
            created_ms,
        });
        store.get_or_insert(&id, created_ms);
        card
    }

    #[test]
    fn serve_loop_invariants_hold_under_a_fuzzed_grade_sequence() {
        let (mut store, _dir) = empty_store();
        let n = 12;
        let mut session = Session::new(cards(n), &store, sched(), SessionOptions::default(), 0);

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
            session.poll(&store, now);

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
        let id = all[0].id().unwrap();
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 1000);
        assert!(session.current_unseen(&store));

        session.acquire_current(&mut store, 1000);

        let state = store.get(&id).expect("acquired card is recorded");
        assert!(
            state.recall.is_none(),
            "acquiring does not schedule under FSRS"
        );
        assert_eq!(1000, state.acquired_ms, "acquire stamps the acquire time");
        assert!(state.history.is_empty());
        assert_eq!(0, state.total_reviews);
        assert_eq!(1, session.stats.acquired);
        assert_eq!(0, session.stats.reviews);
        assert!(session.is_finished());
    }

    #[test]
    fn acquired_cards_are_not_due_until_the_relearn_cooldown() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(cards(1), &store, sched(), SessionOptions::default(), 1000);
        session.acquire_current(&mut store, 1000);

        assert!(!session.has_due_now(&store, 1000));
        assert!(!session.has_due_now(&store, 1000 + DEFAULT_ACQUIRE_COOLDOWN_MS - 1));
        assert!(session.has_due_now(&store, 1000 + DEFAULT_ACQUIRE_COOLDOWN_MS));
    }

    #[test]
    fn an_acquired_card_returns_in_session_after_its_cooldown() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(cards(1), &store, sched(), SessionOptions::default(), 1000);
        let id = session.current().unwrap().id();
        session.acquire_current(&mut store, 1000);
        assert!(session.is_finished());
        assert!(session.poll(&store, 1000 + DEFAULT_ACQUIRE_COOLDOWN_MS));
        assert_eq!(session.current().map(|c| c.id()), Some(id));
        assert!(!session.current_unseen(&store));
    }

    #[test]
    fn a_missed_card_is_not_re_served_before_its_fsrs_due() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        for c in &all {
            store.get_or_insert(&c.id().unwrap(), 0);
        }
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);

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
            store.get_or_insert(&c.id().unwrap(), 0);
        }
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);

        assert_eq!(first_id, session.current().unwrap().id());
        session.grade(&mut store, Grade::Fail, now);
        session.grade(&mut store, Grade::Pass, now + 1000);
        session.poll(&store, now + DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000);
        assert_eq!(first_id, session.current().unwrap().id());
    }

    #[test]
    fn a_graded_card_never_immediately_follows_itself_while_another_is_servable() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let a_id = all[0].id().unwrap();
        let b_id = all[1].id().unwrap();
        for c in &all {
            store.get_or_insert(&c.id().unwrap(), 0);
        }
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);
        assert_eq!(Some(a_id.clone()), session.current().unwrap().id());

        session.grade(&mut store, Grade::Fail, now);
        assert_eq!(
            Some(b_id.clone()),
            session.current().unwrap().id(),
            "the other due card takes over right after the miss"
        );

        store
            .get_or_insert(&a_id, now)
            .recall
            .as_mut()
            .unwrap()
            .due_ms = now + 10_000;

        session.poll(&store, now + 30_000);
        assert_eq!(
            Some(b_id.clone()),
            session.current().unwrap().id(),
            "the floor keeps A from immediately following itself"
        );

        session.poll(&store, now + DEFAULT_ACQUIRE_COOLDOWN_MS);
        assert_eq!(Some(a_id.clone()), session.current().unwrap().id());
    }

    #[test]
    fn the_only_servable_card_may_repeat_once_the_floor_passes() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        store.get_or_insert(&id, 0);
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);

        session.grade(&mut store, Grade::Fail, now);
        assert!(
            session.is_finished(),
            "cooling on its own retry, nothing else to serve"
        );

        store
            .get_or_insert(&id, now)
            .recall
            .as_mut()
            .unwrap()
            .due_ms = now + 1_000;

        session.poll(&store, now + DEFAULT_ACQUIRE_COOLDOWN_MS - 1);
        assert!(session.is_finished(), "the floor delays the repeat");

        session.poll(&store, now + DEFAULT_ACQUIRE_COOLDOWN_MS);
        assert_eq!(Some(id), session.current().and_then(|c| c.id()));
    }

    #[test]
    fn the_transition_floor_follows_the_configured_cooldown() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        store.get_or_insert(&id, 0);
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(
            all,
            &store,
            Box::new(crate::scheduler::Fsrs::new(0.9, 1_000)),
            SessionOptions::default(),
            now,
        );
        session.grade(&mut store, Grade::Fail, now);
        store
            .get_or_insert(&id, now)
            .recall
            .as_mut()
            .unwrap()
            .due_ms = now + 500;
        session.poll(&store, now + 1_000);
        assert_eq!(Some(id), session.current().and_then(|c| c.id()));
    }

    #[test]
    fn a_cards_appearance_count_survives_polls_of_the_same_showing_and_bumps_when_it_returns() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let a_id = all[0].id().unwrap();
        let b_id = all[1].id().unwrap();
        for c in &all {
            store.get_or_insert(&c.id().unwrap(), 0);
        }
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);
        assert_eq!(Some(a_id.clone()), session.current().unwrap().id());
        assert_eq!(
            1,
            session.appearance(&a_id),
            "first showing counts as appearance 1"
        );

        session.poll(&store, now + 1_000);
        session.poll(&store, now + 2_000);
        assert_eq!(1, session.appearance(&a_id), "still the same appearance");

        session.grade(&mut store, Grade::Fail, now);
        assert_eq!(Some(b_id.clone()), session.current().unwrap().id());
        assert_eq!(
            1,
            session.appearance(&a_id),
            "moving off doesn't bump — only being re-served does"
        );

        store
            .get_or_insert(&a_id, now)
            .recall
            .as_mut()
            .unwrap()
            .due_ms = now + DEFAULT_ACQUIRE_COOLDOWN_MS;
        session.grade(&mut store, Grade::Pass, now + 1_000);
        session.poll(&store, now + DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000);
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
        store.get_or_insert(&id, 0);
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);

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
        store.get_or_insert(&all[0].id().unwrap(), 0);
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
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
            store.get_or_insert(&c.id().unwrap(), 0);
        }
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);
        session.grade(&mut store, Grade::Fail, now);
        session.grade(&mut store, Grade::Pass, now);
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
        for c in &all[7..] {
            store.get_or_insert(&c.id().unwrap(), 0);
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
                depth: crate::depth::Depth::default(),
            },
            DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000,
        );
        assert_eq!(3, session.initial_size);
        assert_eq!("front 7", session.current().unwrap().front);
    }

    #[test]
    fn due_cards_are_ordered_by_due_time() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        store.get_or_insert(&all[0].id().unwrap(), 0);
        store.get_or_insert(&all[1].id().unwrap(), 0);

        let now = 2 * 604_800_000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);
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
        store.get_or_insert(&all[0].id().unwrap(), 0);
        store.get_or_insert(&all[1].id().unwrap(), 0);

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
        store.get_or_insert(&all[0].id().unwrap(), now);

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
                depth: crate::depth::Depth::default(),
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
            store.get_or_insert(&c.id().unwrap(), 0);
        }
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), now);
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
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 1000);

        session.grade(&mut store, Grade::Pass, 1000);
        let state = store.get(&id).unwrap();
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
        assert!(store.is_empty());
        let _ = &mut store;
    }

    #[test]
    fn remove_current_also_drops_cloze_siblings() {
        let (store, _dir) = empty_store();
        let mut all = vec![card("deck.md", 1), card("deck.md", 1), card("deck.md", 2)];
        all[0].back = vec!["hole a".into()];
        all[0].hole = Some(0);
        all[1].back = vec!["hole b".into()];
        all[1].hole = Some(1);
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 0);
        assert_eq!(3, session.remaining());
        let removed = session.remove_current(&store, 0);
        assert_eq!(2, removed.len());
        assert_eq!(1, session.remaining());
        assert_eq!(2, session.current().unwrap().line);
    }

    #[test]
    fn cloze_siblings_are_separated() {
        let (store, _dir) = empty_store();
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

    #[test]
    fn lone_sibling_group_still_fully_queued() {
        let (store, _dir) = empty_store();
        let mut all = Vec::new();
        for hole in 1..=3 {
            let mut c = card("deck.md", 1);
            c.back = vec![format!("answer {hole}")];
            c.hole = Some(hole as u32 - 1);
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

        assert!(!session.restart(&store, 1001));
        assert!(session.is_finished());
        assert_eq!(1, session.stats.reviews);
    }

    #[test]
    fn has_due_now_tracks_what_restart_would_find() {
        let (mut store, _dir) = empty_store();
        let mut session = Session::new(cards(1), &store, sched(), SessionOptions::default(), 1000);
        assert!(session.has_due_now(&store, 1000));
        session.grade(&mut store, Grade::Pass, 1000);
        assert!(!session.has_due_now(&store, 1001));
        assert!(!session.restart(&store, 1001));
        assert!(session.has_due_now(&store, 1000 + 3_600_000));
    }

    #[test]
    fn next_due_at_reports_earliest_due_time() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let mut session = Session::new(all, &store, sched(), SessionOptions::default(), 1000);
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
        store.get_or_insert(&c.id().unwrap(), 0).recall = Some(retired_fsrs());
        assert!(is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
        store.get_or_insert(&c.id().unwrap(), 0).recall = Some(crate::store::FsrsState {
            scheduled_days: DEFAULT_RETIRE_AFTER_DAYS - 1,
            ..Default::default()
        });
        assert!(!is_retired(&c, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
        let s = store.get_or_insert(&c.id().unwrap(), 0);
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
        let s = store.get_or_insert(&c.id().unwrap(), now);
        s.streak = 1;
        s.acquired_ms = now;
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

        store.get_or_insert(&c.id().unwrap(), now).recall = Some(retired_fsrs());
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
        store.get_or_insert(&all[0].id().unwrap(), 0).recall = Some(retired_fsrs());

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
                depth: crate::depth::Depth::default(),
            },
            1000,
        );
        assert!(session.is_finished());
    }

    #[test]
    fn a_due_cram_pass_grades_like_a_normal_review() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        store.get_or_insert(&all[0].id().unwrap(), 0).recall = Some(mature_fsrs(1000));
        let now = 40 * 86_400_000;

        let mut session = Session::new(
            all.clone(),
            &store,
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
        store.get_or_insert(&all[0].id().unwrap(), 0).recall = Some(mature_fsrs(40 * 86_400_000));
        let before = store.get(&all[0].id().unwrap()).unwrap().recall.unwrap();

        let mut session = Session::new(
            all.clone(),
            &store,
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
        store.get_or_insert(&all[0].id().unwrap(), 0).recall = Some(crate::store::FsrsState {
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
            store.get_or_insert(&c.id().unwrap(), 0).recall = Some(crate::store::FsrsState {
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
        session.grade(&mut store, Grade::Fail, 10_000);
        session.grade(&mut store, Grade::Pass, 10_000);
        assert!(
            session.is_finished(),
            "cram is a single pass over the roster"
        );
        assert_eq!(1, store.get(&id_a).unwrap().history.len());
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
            store.get_or_insert(&c.id().unwrap(), 0);
        }
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
        store.get_or_insert(&all[0].id().unwrap(), 0);
        store.get_or_insert(&all[1].id().unwrap(), now);
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
            store.get_or_insert(&c.id().unwrap(), 0);
        }
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
        store.get_or_insert(&all[0].id().unwrap(), 0).recall = Some(retired_fsrs());
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
            store.get_or_insert(&c.id().unwrap(), 0);
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
        store.get_or_insert(&all[0].id().unwrap(), 0);
        store.get_or_insert(&all[1].id().unwrap(), 0);
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
        store.get_or_insert(&id.to_string(), 0).recall = Some(FsrsState {
            stability: 10.0,
            state: 2,
            ..Default::default()
        });
        let ten_days_ms = 10 * 86_400_000;
        let fresh = card_strengths(&[id.to_string()], &store, 0);
        let later = card_strengths(&[id.to_string()], &store, ten_days_ms);
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
        assert_eq!(vec![0.0], card_strengths(&[7.to_string()], &store, 0));
        assert!(card_strengths(&[], &store, 0).is_empty());
    }

    #[test]
    fn virtual_card_joins_the_roster_and_is_served() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.md", "virtual back", 0);
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;
        let session = Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);
        assert_eq!(1, session.initial_size);
        assert_eq!("virtual front", session.current().unwrap().front);
        assert!(session.current_is_virtual(&store));
    }

    #[test]
    fn grading_a_virtual_card_updates_store_cards() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.md", "virtual back", 0);
        let id = synth.id().unwrap();
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;
        let mut session =
            Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);

        session.grade(&mut store, Grade::Pass, now);

        let state = store.get(&id).expect("virtual schedule in store.cards");
        assert!(state.recall.is_some());
        assert_eq!(1, state.total_reviews);
        assert!(store.is_virtual(&id));
    }

    #[test]
    fn virtual_card_not_treated_as_unseen() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.md", "virtual back", 0);
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;
        let session = Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);
        assert!(!session.current_unseen(&store));
    }

    #[test]
    fn a_missed_virtual_card_reappears_on_its_fsrs_due() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.md", "virtual back", 0);
        let synth_id = synth.id();
        let deck_card = card("deck.md", 0);
        store.get_or_insert(&deck_card.id().unwrap(), 0);

        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000;
        let mut session = Session::new(
            vec![synth, deck_card],
            &store,
            sched(),
            SessionOptions::default(),
            now,
        );

        assert_eq!(synth_id, session.current().unwrap().id());
        session.grade(&mut store, Grade::Fail, now);
        session.grade(&mut store, Grade::Pass, now + 1000);
        session.poll(&store, now + DEFAULT_ACQUIRE_COOLDOWN_MS + 60_000);
        assert_eq!(synth_id, session.current().unwrap().id());
    }

    #[test]
    fn count_reviewable_virtual_counts_due_excludes_archived() {
        let (mut store, _dir) = empty_store();
        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;
        let cap = Some(DEFAULT_RETIRE_AFTER_DAYS);
        let sched = sched();

        insert_virtual(&mut store, "deck.md", "gap-due", 0);
        insert_virtual(&mut store, "deck.md", "gap-not-due", now);
        let archived = insert_virtual(&mut store, "deck.md", "gap-archived", 0);
        store.get_or_insert(&archived.id().unwrap(), 0).recall = Some(retired_fsrs());

        assert_eq!(
            1,
            count_reviewable_virtual(&store, "deck.md", sched.as_ref(), now, cap)
        );
        assert!(has_reviewable_virtual(
            &store,
            "deck.md",
            sched.as_ref(),
            now,
            cap
        ));
    }

    #[test]
    fn next_due_at_includes_virtual_cards() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.md", "virtual back", 1000);
        let session = Session::new(
            vec![synth],
            &store,
            sched(),
            SessionOptions::default(),
            1000,
        );
        let due = session
            .next_due_at(&store)
            .expect("a virtual card's due time is reported");
        assert_eq!(1000 + DEFAULT_ACQUIRE_COOLDOWN_MS, due);
    }

    #[test]
    fn virtual_card_is_retired_when_interval_reaches_cap() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.md", "virtual back", 0);
        let id = synth.id().unwrap();
        let options = SessionOptions {
            retire_after_days: Some(4),
            ..SessionOptions::default()
        };

        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;
        let mut session = Session::new(vec![synth.clone()], &store, sched(), options.clone(), now);
        session.grade(&mut store, Grade::Pass, now);
        assert!(!is_retired_id(&id, &store, options.retire_after_days));

        let now = 86_460_000;
        let mut session = Session::new(vec![synth.clone()], &store, sched(), options.clone(), now);
        session.grade(&mut store, Grade::Pass, now);

        assert!(is_retired_id(&id, &store, options.retire_after_days));
        let state = store.get(&id).expect("schedule kept, not deleted");
        assert_eq!(4, state.recall.as_ref().unwrap().scheduled_days);
        assert_eq!(2, state.total_reviews);
        assert!(store.is_virtual(&id));

        let session = Session::new(vec![synth], &store, sched(), options.clone(), now);
        assert!(session.is_finished());
        assert_eq!(
            0,
            count_reviewable_virtual(
                &store,
                "deck.md",
                sched().as_ref(),
                now,
                options.retire_after_days
            )
        );
    }

    #[test]
    fn raising_retire_after_un_retires_a_virtual_card() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.md", "virtual back", 0);
        let id = synth.id().unwrap();
        store.get_or_insert(&id, 0).recall = Some(crate::store::FsrsState {
            scheduled_days: 10,
            ..Default::default()
        });
        assert!(is_retired_id(&id, &store, Some(10)));
        assert!(!is_retired_id(&id, &store, Some(20)));

        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;
        let sched = sched();
        assert_eq!(
            0,
            count_reviewable_virtual(&store, "deck.md", sched.as_ref(), now, Some(10))
        );
        assert_eq!(
            1,
            count_reviewable_virtual(&store, "deck.md", sched.as_ref(), now, Some(20))
        );
    }

    #[test]
    fn retired_virtual_card_is_excluded_from_queue_and_counts() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.md", "virtual back", 0);
        let id = synth.id().unwrap();
        store.get_or_insert(&id, 0).recall = Some(retired_fsrs());

        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;
        let session = Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);
        assert!(session.is_finished());

        let cap = Some(DEFAULT_RETIRE_AFTER_DAYS);
        assert_eq!(
            0,
            count_reviewable_virtual(&store, "deck.md", sched().as_ref(), now, cap)
        );
        assert!(store.is_virtual(&id));
    }

    #[test]
    fn retire_only_at_cap_not_below() {
        let (mut store, _dir) = empty_store();
        let synth = insert_virtual(&mut store, "deck.md", "virtual back", 0);
        let id = synth.id().unwrap();

        let now = DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;
        let mut session =
            Session::new(vec![synth], &store, sched(), SessionOptions::default(), now);
        session.grade(&mut store, Grade::Pass, now);

        assert!(!is_retired_id(&id, &store, Some(DEFAULT_RETIRE_AFTER_DAYS)));
    }

    #[test]
    fn a_reconstruct_grade_never_touches_the_recall_schedule() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        store.get_or_insert(&id, 0).recall = Some(FsrsState {
            stability: 30.0,
            state: 2,
            ..Default::default()
        });
        let mut s = Session::new(
            all,
            &store,
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
        store.get_or_insert(&all[0].id().unwrap(), 0).recall = Some(FsrsState {
            stability: 30.0,
            state: 2,
            due_ms: u64::MAX,
            ..Default::default()
        });
        let s = Session::new(
            all,
            &store,
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
            &store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            0,
        );
        s.grade(&mut store, Grade::Pass, 1_000);
        s.grade(&mut store, Grade::Fail, 2_000);
        assert!(store.get(&a).unwrap().recognized_ms.is_some());
        assert!(store.get(&b).is_none_or(|st| st.recognized_ms.is_none()));
        assert!(
            store.get(&a).unwrap().recall.is_none(),
            "recognize never schedules"
        );
        assert_eq!(
            0,
            s.remaining(),
            "the wrong pick re-queues, but the floor holds it back"
        );
        s.poll(&store, 2_000 + DEFAULT_ACQUIRE_COOLDOWN_MS);
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
            &store,
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

        s.poll(&store, 1_000 + DEFAULT_ACQUIRE_COOLDOWN_MS + 500);
        assert_eq!(
            a,
            s.current().unwrap().id(),
            "A's own floor has passed (B's hasn't) — floors are independent per card"
        );
    }

    #[test]
    fn a_recognize_wrong_pick_may_repeat_once_the_floor_passes() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let mut s = Session::new(
            all,
            &store,
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

        s.poll(&store, 1_000 + DEFAULT_ACQUIRE_COOLDOWN_MS - 1);
        assert!(s.is_finished(), "the floor hasn't passed yet");

        s.poll(&store, 1_000 + DEFAULT_ACQUIRE_COOLDOWN_MS);
        assert_eq!(
            Some(id),
            s.current().and_then(|c| c.id()),
            "the floor passed: delayed, not starved"
        );
    }

    #[test]
    fn recognize_queue_holds_only_unrecognized_cards() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        store.get_or_insert(&all[0].id().unwrap(), 0).recognized_ms = Some(5);
        let s = Session::new(
            all,
            &store,
            sched(),
            SessionOptions {
                depth: Depth::Recognize,
                ..Default::default()
            },
            1_000,
        );
        assert_eq!(1, s.remaining());
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

    fn reconstruct_session(all: Vec<Card>, store: &Store, cram: bool, now: u64) -> Session {
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
        store.get_or_insert(&id, 0).recall = Some(mature_fsrs(500));
        let now = 40 * 86_400_000;

        let mut s = reconstruct_session(all, &store, false, now);
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

    #[test]
    fn a_reconstruct_pass_on_a_not_yet_due_recall_reanchors_without_reward() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 1_000_000;
        store.get_or_insert(&id, 0).recall = Some(mature_fsrs(2_000_000));

        let mut s = reconstruct_session(all, &store, false, now);
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
        store.get_or_insert(&id, 0).reconstruct = Some(mature_fsrs(500));

        let mut s = reconstruct_session(all, &store, false, now);
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
            store.get_or_insert(&c.id().unwrap(), 0).recall = Some(mature_fsrs(500));
        }

        let mut s = reconstruct_session(all.clone(), &store, false, now);
        s.grade(&mut store, Grade::Partial, now);
        s.grade(&mut store, Grade::Fail, now);

        for c in &all {
            let st = store.get(&c.id().unwrap()).unwrap();
            assert_eq!(
                mature_fsrs(500),
                st.recall.unwrap(),
                "recall untouched by a partial or a fail"
            );
            assert!(st.recognized_ms.is_none(), "recognized untouched");
            assert!(st.history.iter().all(|r| !r.propagated));
        }
    }

    #[test]
    fn a_due_reconstruct_cram_pass_credits_recall_like_a_normal_review() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 40 * 86_400_000;
        let state = store.get_or_insert(&id, 0);
        state.recall = Some(mature_fsrs(500));
        state.reconstruct = Some(mature_fsrs(500));

        let mut s = reconstruct_session(all, &store, true, now);
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
        assert_eq!(Some(now), st.recognized_ms);
    }

    #[test]
    fn an_early_reconstruct_cram_pass_propagates_nothing() {
        let (mut store, _dir) = empty_store();
        let all = cards(1);
        let id = all[0].id().unwrap();
        let now = 10 * 86_400_000;
        let future = 40 * 86_400_000;
        let state = store.get_or_insert(&id, 0);
        state.recall = Some(mature_fsrs(future));
        state.reconstruct = Some(mature_fsrs(future));

        let mut s = reconstruct_session(all, &store, true, now);
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
        assert_eq!(Some(now), st.recognized_ms);
    }

    #[test]
    fn recognize_cram_serves_already_recognized_cards() {
        let (mut store, _dir) = empty_store();
        let all = cards(2);
        let now = 1_000_000;
        for card in &all {
            store.get_or_insert(&card.id().unwrap(), 0).recognized_ms = Some(1);
        }

        let normal = Session::new(
            all.clone(),
            &store,
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
            &store,
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
    fn any_full_pass_sets_recognized_transitively() {
        let (mut store, _dir) = empty_store();
        let all = cards(3);
        let now = 1_000_000;

        let mut recall = Session::new(
            vec![all[0].clone()],
            &store,
            sched(),
            SessionOptions::default(),
            now,
        );
        recall.grade(&mut store, Grade::Pass, now);
        assert_eq!(
            Some(now),
            store.get(&all[0].id().unwrap()).unwrap().recognized_ms,
            "a recall pass marks recognized"
        );

        let mut reconstruct = reconstruct_session(vec![all[1].clone()], &store, false, now);
        reconstruct.grade(&mut store, Grade::Pass, now);
        assert_eq!(
            Some(now),
            store.get(&all[1].id().unwrap()).unwrap().recognized_ms,
            "a reconstruct pass marks recognized"
        );

        let mut partial = Session::new(
            vec![all[2].clone()],
            &store,
            sched(),
            SessionOptions::default(),
            now,
        );
        partial.grade(&mut store, Grade::Partial, now);
        assert!(
            store
                .get(&all[2].id().unwrap())
                .unwrap()
                .recognized_ms
                .is_none(),
            "a partial never marks recognized"
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
