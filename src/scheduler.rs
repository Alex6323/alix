//! Review scheduling.
//!
//! One scheduler: [`Fsrs`], FSRS-5 via the `rs-fsrs` crate. Short-term modeling is on, so FSRS owns
//! both the learning steps (a New card graded Good is due ~10 min out in `Learning`) and the
//! long-term DSR review that follows — one model across the short and the long term, no separate
//! box tiers to switch between. Graduation to `Review` always takes **two** full Goods in the acquisition
//! phase (a Fail resets that progress rather than fast-tracking it — see [`Fsrs::apply`]).
//!
//! The legacy Leitner `stage` field is gone entirely. `acquired_ms` marks when a card was first
//! shown, and `seed_card` always seeds fresh FSRS state as `New` — there is no pre-FSRS carry-over
//! to derive from anymore.

use chrono::{DateTime, Utc};
use rs_fsrs::{Card as FsrsCard, FSRS, Parameters, Rating, State as RawState};

use crate::{
    level::Level,
    store::{CardState, FsrsState},
};

/// The outcome of reviewing a card. Three honest outcomes, shared by fact-deck
/// review and the trace walk: **failed** / **partly** / **got it**.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Grade {
    /// Wrong (or the user asked for hints / graded "failed"). Maps to FSRS `Again`.
    Fail,
    /// Only partly right — a *weak success*, mapped to FSRS `Hard`: it keeps drilling but still
    /// counts as a pass (advances the streak, counts in the stats).
    Partial,
    /// Fully correct ("got it"). Maps to FSRS `Good`.
    Pass,
}

impl Grade {
    /// Whether this grade counts as a pass (advances the streak, counts in the stats). Both a clean
    /// `Pass` and a `Partial` (a weak success) count — only a `Fail` doesn't.
    pub fn passed(self) -> bool {
        matches!(self, Grade::Pass | Grade::Partial)
    }
}

/// Derives a [`Grade`] from how many of a card's key points a reconstruction
/// covered (the Explain-mode checklist): none → `Fail`, all → `Pass`, some →
/// `Partial`. A `total` of 0 is a `Pass` — there's no rubric to miss. This is the
/// one place the rule lives; both frontends call it.
pub fn keypoint_grade(covered: usize, total: usize) -> Grade {
    if total == 0 || covered >= total {
        Grade::Pass
    } else if covered == 0 {
        Grade::Fail
    } else {
        Grade::Partial
    }
}

/// A scheduling algorithm. Every method is routed through an explicit
/// `level`: `Recall` and `Reconstruct` each own an independent schedule on
/// `CardState` (see `CardState::schedule`/`schedule_slot`), so nothing here
/// implicitly means "the Recall schedule" — the caller says which.
pub trait Scheduler {
    /// When the card is due next (Unix ms) at `level`.
    fn due_at(&self, state: &CardState, level: Level) -> u64;

    /// Applies a review outcome to the card state at `level`, at time `now_ms`.
    /// `propagated` marks the recorded review as credit flowed down from a pass
    /// at a higher level rather than answered directly (see `Session::grade`);
    /// the schedule math is identical either way.
    fn apply(
        &self,
        state: &mut CardState,
        level: Level,
        grade: Grade,
        now_ms: u64,
        propagated: bool,
    );

    /// A *cram refresh*: keep the card's memory (stability, difficulty, interval)
    /// exactly as-is but push its due date out by its current interval, so a correct
    /// cram answer refreshes without rewarding. No review is recorded. A card with no
    /// FSRS state yet at `level` is left untouched — there's no interval to preserve.
    fn reanchor(&self, state: &mut CardState, level: Level, now_ms: u64);

    /// Whether the card is due at `level` at `now_ms`.
    fn is_due(&self, state: &CardState, level: Level, now_ms: u64) -> bool {
        self.due_at(state, level) <= now_ms
    }
}

/// Fixed settle gap between acquiring a card (its answer is shown, acknowledged)
/// and its first real quiz — ~1 min, gating only the next session/restart. An
/// in-session retry is position-based and unaffected. Was the stage-1 cooldown.
pub const ACQUIRE_COOLDOWN_MS: u64 = 60 * 1000;

/// One day in milliseconds — converts between FSRS's `scheduled_days` and alix's ms timestamps
/// in [`Fsrs::apply`] and [`Fsrs::reanchor`].
const DAY_MS: u64 = 86_400 * 1000;

/// Hold step for a card that passed once but hasn't earned its second Good yet
/// (10 min, matching FSRS's New+Good step).
const LEARNING_HOLD_MS: u64 = 10 * 60 * 1000;

// ---- FSRS scheduler (backed by the `rs-fsrs` crate) ----
//
// The FSRS *math* lives in `rs-fsrs`; this is only the thin boundary. alix keeps its
// own `u64`-ms timestamps and its own [`FsrsState`]; here we convert to/from the crate's
// `Card`/`DateTime`, map grades, drive the scheduler, and store the result.

fn ms_to_dt(ms: u64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms as i64).unwrap_or_default()
}

fn dt_to_ms(dt: DateTime<Utc>) -> u64 {
    dt.timestamp_millis().max(0) as u64
}

/// alix's three grades → FSRS ratings (no `Easy`): missed → Again, partly → Hard,
/// got it → Good.
fn rating_for(grade: Grade) -> Rating {
    match grade {
        Grade::Fail => Rating::Again,
        Grade::Partial => Rating::Hard,
        Grade::Pass => Rating::Good,
    }
}

fn raw_state(s: u8) -> RawState {
    match s {
        1 => RawState::Learning,
        2 => RawState::Review,
        3 => RawState::Relearning,
        _ => RawState::New,
    }
}

fn to_fsrs_card(s: &FsrsState) -> FsrsCard {
    FsrsCard {
        due: ms_to_dt(s.due_ms),
        stability: s.stability,
        difficulty: s.difficulty,
        elapsed_days: 0, // rs-fsrs recomputes this from `last_review`
        scheduled_days: s.scheduled_days as i64,
        reps: s.reps as i32,
        lapses: s.lapses as i32,
        state: raw_state(s.state),
        last_review: ms_to_dt(s.last_review_ms),
    }
}

fn from_fsrs_card(c: &FsrsCard) -> FsrsState {
    FsrsState {
        stability: c.stability,
        difficulty: c.difficulty,
        reps: c.reps.max(0) as u32,
        lapses: c.lapses.max(0) as u32,
        state: c.state as u8,
        scheduled_days: c.scheduled_days.max(0) as u32,
        last_review_ms: dt_to_ms(c.last_review),
        due_ms: dt_to_ms(c.due),
        // The learning-steps counter is alix's, not rs-fsrs's; `apply` carries it.
        ..Default::default()
    }
}

/// Seeds an `rs-fsrs` `Card` for a card with no FSRS state yet: always `New`,
/// so FSRS derives its initial stability from the first grade. (Pre-FSRS
/// Leitner carry-over was dropped with the stage field — pre-1.0.)
fn seed_card(_state: &CardState, now_ms: u64) -> FsrsCard {
    let mut c = FsrsCard::new();
    c.last_review = ms_to_dt(now_ms);
    c.due = ms_to_dt(now_ms);
    c
}

/// The FSRS scheduler, backed by `rs-fsrs`, built for a desired retention. Short-term
/// modeling is **on** (`enable_short_term = true`), so FSRS's built-in learning steps own
/// acquisition (a New card graded Good is due ~10 min out in `Learning`) and the DSR model owns
/// long-term review — one scheduler across both the short and the long term. [`apply`](Fsrs::apply)
/// gates graduation to `Review` on two full Goods so a fail no longer fast-tracks it.
pub struct Fsrs {
    fsrs: FSRS,
}

impl Fsrs {
    /// An FSRS scheduler targeting `retention` (e.g. 0.9).
    pub fn new(retention: f64) -> Self {
        let parameters = Parameters {
            request_retention: retention,
            enable_short_term: true,
            ..Parameters::default()
        };
        Self {
            fsrs: FSRS::new(parameters),
        }
    }
}

impl Default for Fsrs {
    fn default() -> Self {
        Self::new(0.9)
    }
}

impl Scheduler for Fsrs {
    fn due_at(&self, state: &CardState, level: Level) -> u64 {
        match state.schedule(level) {
            Some(s) => s.due_ms,
            // No schedule yet at this level. If the card is already established at
            // the *other* level, switching levels is immediate — due now (epoch),
            // no second acquire warm-up (spec §4.1: a Recall-drilled deck is at once
            // due at Reconstruct; the schedule is created lazily on the first grade).
            // This is the one place that rule lives, so `is_due`, the queue sort, and
            // the picker helpers can't diverge.
            None if state.recall.is_some() || state.reconstruct.is_some() => 0,
            // A genuinely fresh card (no schedule anywhere) settles for one fixed
            // acquire cooldown before its first quiz.
            None => state.acquired_ms.saturating_add(ACQUIRE_COOLDOWN_MS),
        }
    }

    fn apply(
        &self,
        state: &mut CardState,
        level: Level,
        grade: Grade,
        now_ms: u64,
        propagated: bool,
    ) {
        let current = state.schedule(level).copied();
        let (card, pre_state, prev_goods) = match &current {
            Some(s) => (to_fsrs_card(s), s.state, s.learning_goods),
            None => (seed_card(state, now_ms), 0, 0),
        };
        let info = self.fsrs.next(card, ms_to_dt(now_ms), rating_for(grade));
        let mut next = from_fsrs_card(&info.card);

        // Learning-steps gate: graduation to Review takes two full Goods in the
        // initial acquisition phase (pre-grade New or Learning), so a fail can't
        // fast-track a card past the Good -> Good path. A Fail resets the count; a
        // Partial is neutral (rs-fsrs keeps it in Learning). Relearning (a lapse,
        // pre-grade state Relearning) is not gated — it re-graduates on one Good.
        let acquiring = matches!(pre_state, 0 | 1);
        let mut goods = prev_goods;
        if acquiring {
            match grade {
                Grade::Pass => goods = goods.saturating_add(1),
                Grade::Fail => goods = 0,
                Grade::Partial => {}
            }
        }
        if next.state == 2 && acquiring && goods < 2 {
            // rs-fsrs would graduate on this single Good; hold in Learning instead.
            next.state = 1;
            next.scheduled_days = 0;
            next.due_ms = now_ms.saturating_add(LEARNING_HOLD_MS);
        }
        next.learning_goods = if next.state == 2 { 0 } else { goods };

        // `Recognize` has no slot (unscheduled + boolean) — a silent no-op, never
        // reached in practice since Recognize is graded by pick, not by `apply`.
        let Some(slot) = state.schedule_slot(level) else {
            return;
        };
        *slot = Some(next);
        state.record_review(now_ms, grade, level, propagated);
    }

    fn reanchor(&self, state: &mut CardState, level: Level, now_ms: u64) {
        if let Some(Some(f)) = state.schedule_slot(level) {
            let interval_ms = u64::from(f.scheduled_days) * DAY_MS;
            f.due_ms = now_ms.saturating_add(interval_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_at_for_an_unscheduled_card_is_one_acquire_cooldown_out() {
        let sched = Fsrs::default();
        let mut s = CardState::new(1000); // acquired_ms = 1000, recall = None
        // No FSRS state yet: due is exactly the fixed acquire cooldown past acquire.
        assert_eq!(sched.due_at(&s, Level::Recall), 1000 + ACQUIRE_COOLDOWN_MS);
        // Once graduated to FSRS, the schedule owns the due date, not the cooldown.
        sched.apply(&mut s, Level::Recall, Grade::Pass, 1000, false);
        assert!(s.recall.is_some());
        assert!(sched.due_at(&s, Level::Recall) != 1000 + ACQUIRE_COOLDOWN_MS);
    }

    #[test]
    fn due_at_is_immediate_at_a_level_scheduled_elsewhere() {
        // The cross-level immediacy rule (spec §4.1), now owned by `due_at`: a card
        // established at Recall is due *now* at Reconstruct, skipping the acquire
        // cooldown a genuinely fresh card would honor.
        let sched = Fsrs::default();
        let mut s = CardState::new(1_000);
        s.recall = Some(FsrsState {
            due_ms: u64::MAX,
            ..Default::default()
        });
        assert_eq!(0, sched.due_at(&s, Level::Reconstruct), "immediately due");
        assert!(sched.is_due(&s, Level::Reconstruct, 1)); // due at any `now`
        // Recall itself still honors its own schedule, not the cross-level rule.
        assert_eq!(u64::MAX, sched.due_at(&s, Level::Recall));
        // A card with no schedule anywhere still waits one acquire cooldown.
        let fresh = CardState::new(1_000);
        assert_eq!(
            1_000 + ACQUIRE_COOLDOWN_MS,
            sched.due_at(&fresh, Level::Reconstruct)
        );
    }

    #[test]
    fn apply_writes_only_the_chosen_levels_schedule() {
        let sched = Fsrs::default();
        let mut st = CardState::new(0);
        sched.apply(&mut st, Level::Reconstruct, Grade::Pass, 1_000, false);
        assert!(st.schedule(Level::Reconstruct).is_some());
        assert!(st.schedule(Level::Recall).is_none(), "no cross-crediting");
        assert_eq!(Level::Reconstruct, st.history[0].level);
    }

    #[test]
    fn apply_on_recognize_is_a_no_op() {
        let sched = Fsrs::default();
        let mut st = CardState::new(0);
        sched.apply(&mut st, Level::Recognize, Grade::Pass, 1_000, false);
        assert!(st.recall.is_none() && st.reconstruct.is_none() && st.history.is_empty());
    }

    #[test]
    fn apply_stores_the_propagated_marker_on_the_review() {
        // The marker rides along into the recorded review; the schedule math is
        // identical either way — only the history entry differs.
        let sched = Fsrs::default();
        let mut direct = CardState::new(0);
        sched.apply(&mut direct, Level::Recall, Grade::Pass, 1_000, false);
        assert!(!direct.history[0].propagated, "a direct review is unmarked");
        let mut credited = CardState::new(0);
        sched.apply(&mut credited, Level::Recall, Grade::Pass, 1_000, true);
        assert!(
            credited.history[0].propagated,
            "a credited review is marked"
        );
        assert_eq!(direct.recall, credited.recall, "same schedule either way");
    }

    #[test]
    fn the_acquire_cooldown_is_stable() {
        // The single acquire cooldown is exactly the old stage-1 gap (~1 min), so
        // freshly-acquired timing is unchanged by the depth-dial removal.
        assert_eq!(60 * 1000, ACQUIRE_COOLDOWN_MS);
    }

    #[test]
    fn partly_counts_as_a_pass() {
        // A partial is a weak success: it advances the streak and counts as a pass, but is still
        // logged as `Partial` (not `Pass`).
        let mut s = CardState::new(0);
        s.record_review(0, Grade::Pass, Level::Recall, false); // build a streak
        assert_eq!(1, s.streak);
        assert_eq!(1, s.total_passes);
        s.record_review(1000, Grade::Partial, Level::Recall, false);
        assert_eq!(2, s.streak); // partly keeps the streak going
        assert_eq!(2, s.total_passes); // ...and counts as a pass
        assert!(s.history.last().unwrap().grade.passed());
        assert_eq!(Grade::Partial, s.history.last().unwrap().grade); // still logged as partial
    }

    #[test]
    fn keypoint_grade_derives_from_coverage() {
        assert_eq!(Grade::Fail, keypoint_grade(0, 5)); // none covered
        assert_eq!(Grade::Partial, keypoint_grade(1, 5)); // any coverage is "some"
        assert_eq!(Grade::Partial, keypoint_grade(4, 5));
        assert_eq!(Grade::Pass, keypoint_grade(5, 5)); // all covered
        assert_eq!(Grade::Pass, keypoint_grade(0, 0)); // no rubric → pass
    }

    #[test]
    fn fsrs_pass_on_a_new_card_sets_stability_and_schedules_out() {
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 0, false);
        let f = s.recall.expect("fsrs state set");
        assert!(f.stability > 0.0, "stability should be positive");
        assert!(
            sched.due_at(&s, Level::Recall) > 0,
            "should be scheduled into the future"
        );
        assert_eq!(1, s.total_reviews);
        assert!(s.history.last().unwrap().grade.passed());
    }

    #[test]
    fn fsrs_partly_grows_stability_less_than_got_it() {
        // partly → Hard (a weak success) grows stability less than Pass → Good.
        let sched = Fsrs::new(0.9);
        let mut good = CardState::new(0);
        sched.apply(&mut good, Level::Recall, Grade::Pass, 0, false);
        let mut hard = CardState::new(0);
        sched.apply(&mut hard, Level::Recall, Grade::Partial, 0, false);
        assert!(good.recall.unwrap().stability > hard.recall.unwrap().stability);
    }

    #[test]
    fn fsrs_a_miss_shortens_the_next_interval() {
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 0, false);
        let pass_interval = sched.due_at(&s, Level::Recall); // reviewed at t = 0
        sched.apply(&mut s, Level::Recall, Grade::Fail, pass_interval, false); // miss it when due
        let fail_interval = sched
            .due_at(&s, Level::Recall)
            .saturating_sub(pass_interval);
        assert!(!s.history.last().unwrap().grade.passed());
        assert!(
            fail_interval < pass_interval,
            "a lapse should shorten the interval (fail {fail_interval} vs pass {pass_interval})"
        );
    }

    #[test]
    fn fsrs_new_card_good_enters_a_learning_step() {
        // With short-term on, a New card graded Good is due a short learning step out
        // (FSRS's built-in ~10 min), in `Learning` — not scheduled days out yet.
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 0, false);
        let f = s.recall.expect("fsrs state set");
        assert_eq!(1, f.state, "a first Good enters Learning, not Review");
        let due = sched.due_at(&s, Level::Recall);
        assert!(
            due > 0 && due < DAY_MS,
            "learning step is sub-day (got {due} ms)"
        );
    }

    #[test]
    fn fsrs_two_goods_graduate_to_review() {
        // A second Good graduates the card out of the learning steps into Review, now
        // scheduled inter-day.
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 0, false);
        let step_due = sched.due_at(&s, Level::Recall);
        sched.apply(&mut s, Level::Recall, Grade::Pass, step_due, false);
        assert_eq!(
            2,
            s.recall.expect("fsrs state").state,
            "second Good reaches Review"
        );
        assert!(
            sched.due_at(&s, Level::Recall) - step_due >= DAY_MS,
            "a graduated card is scheduled at least a day out"
        );
    }

    #[test]
    fn fail_then_one_good_does_not_graduate() {
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Level::Recall, Grade::Fail, 0, false); // New -> Learning
        sched.apply(&mut s, Level::Recall, Grade::Pass, 60_000, false); // one Good: held, not graduated
        let f = s.recall.unwrap();
        assert_eq!(1, f.state, "one Good after a fail stays in Learning");
        assert_eq!(1, f.learning_goods);
    }

    #[test]
    fn two_goods_after_a_fail_do_graduate() {
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Level::Recall, Grade::Fail, 0, false); // New -> Learning, goods = 0
        sched.apply(&mut s, Level::Recall, Grade::Pass, 60_000, false); // goods = 1, held
        assert_eq!(1, s.recall.unwrap().state);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 700_000, false); // goods = 2 -> Review
        assert_eq!(2, s.recall.unwrap().state, "two Goods graduate");
    }

    #[test]
    fn a_fail_resets_graduation_progress() {
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 0, false); // goods = 1
        sched.apply(&mut s, Level::Recall, Grade::Fail, 60_000, false); // reset -> goods = 0
        sched.apply(&mut s, Level::Recall, Grade::Pass, 120_000, false); // goods = 1, held
        assert_eq!(1, s.recall.unwrap().state, "still Learning after the reset");
        sched.apply(&mut s, Level::Recall, Grade::Pass, 700_000, false); // goods = 2 -> Review
        assert_eq!(2, s.recall.unwrap().state);
    }

    #[test]
    fn partial_is_neutral_for_graduation() {
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 0, false); // goods = 1, Learning
        sched.apply(&mut s, Level::Recall, Grade::Partial, 600_000, false); // neutral: stays Learning, goods = 1
        assert_eq!(1, s.recall.unwrap().state);
        assert_eq!(1, s.recall.unwrap().learning_goods);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 1_200_000, false); // goods = 2 -> Review
        assert_eq!(2, s.recall.unwrap().state);
    }

    #[test]
    fn a_lapsed_card_regraduates_on_one_good() {
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 0, false);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 600_000, false); // -> Review
        sched.apply(&mut s, Level::Recall, Grade::Fail, 1_200_000, false); // lapse -> Relearning
        assert_eq!(3, s.recall.unwrap().state);
        sched.apply(&mut s, Level::Recall, Grade::Pass, 1_800_000, false); // one Good re-graduates (gate skips relearning)
        assert_eq!(2, s.recall.unwrap().state);
    }

    #[test]
    fn fsrs_overdue_recall_beats_on_time_recall() {
        // The spacing effect (long-term / Review state): recalling a more-overdue card
        // (lower retrievability) grows stability more. Graduate two identical cards to
        // Review first, then re-pass at different lateness.
        let sched = Fsrs::new(0.9);
        let mut early = CardState::new(0);
        sched.apply(&mut early, Level::Recall, Grade::Pass, 0, false); // New -> Learning
        let step_due = sched.due_at(&early, Level::Recall);
        sched.apply(&mut early, Level::Recall, Grade::Pass, step_due, false); // Learning -> Review
        let due = sched.due_at(&early, Level::Recall);
        let mut late = early.clone();
        sched.apply(&mut early, Level::Recall, Grade::Pass, due, false); // on time
        sched.apply(&mut late, Level::Recall, Grade::Pass, due * 3, false); // well overdue
        assert!(
            late.recall.unwrap().stability > early.recall.unwrap().stability,
            "an overdue-but-recalled card should gain more stability"
        );
    }
}
