//! Review scheduling.
//!
//! One scheduler: [`Fsrs`], FSRS-5 via the `rs-fsrs` crate. Short-term modeling is on, so FSRS owns
//! both the learning steps (a New card graded Good is due ~10 min out in `Learning`; a second Good
//! graduates it to `Review`) and the long-term DSR review that follows — one model across the short
//! and the long term, no box ladder to switch between.
//!
//! The legacy Leitner `stage` field is retained only as an acquire marker and for the one-time
//! lazy-derive that seeds FSRS state from a pre-FSRS card's stage on its first FSRS review; it is
//! no longer live scheduling state.

use chrono::{DateTime, Utc};
use rs_fsrs::{Card as FsrsCard, FSRS, Parameters, Rating, State as RawState};

use crate::store::{CardState, FsrsState, MAX_STAGE};

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

/// A scheduling algorithm.
pub trait Scheduler {
    /// When the card is due next (Unix ms).
    fn due_at(&self, state: &CardState) -> u64;

    /// Applies a review outcome to the card state at time `now_ms`.
    fn apply(&self, state: &mut CardState, grade: Grade, now_ms: u64);

    /// A *cram refresh*: keep the card's memory (stability, difficulty, interval)
    /// exactly as-is but push its due date out by its current interval, so a correct
    /// cram answer refreshes without rewarding. No review is recorded. A card with no
    /// FSRS state yet is left untouched — there's no interval to preserve.
    fn reanchor(&self, state: &mut CardState, now_ms: u64);

    /// Whether the card is due at `now_ms`.
    fn is_due(&self, state: &CardState, now_ms: u64) -> bool {
        self.due_at(state) <= now_ms
    }
}

/// Cooldowns per Leitner stage in milliseconds, indexed by `stage - 1`.
///
/// Stage 1 is a short **relearn/settle gap**: a newly acquired or freshly failed
/// card is due ~5 min out, gating only the *next* session/restart. An in-session
/// retry of a failed card is position-based (pushed to the back of the queue and
/// served by position, not by due time), so it is unaffected.
pub const STAGE_COOLDOWNS_MS: [u64; MAX_STAGE as usize] = [
    5 * 60 * 1000,  // stage 1: ~5 min (relearn/settle gap)
    3_600 * 1000,   // stage 2: 1 hour
    21_600 * 1000,  // stage 3: 6 hours
    86_400 * 1000,  // stage 4: 1 day
    604_800 * 1000, // stage 5: 1 week
];

/// Returns the cooldown of a stage (1..=5) in milliseconds.
pub fn stage_cooldown_ms(stage: u8) -> u64 {
    STAGE_COOLDOWNS_MS[(stage.clamp(1, MAX_STAGE) - 1) as usize]
}

/// One day in milliseconds — the unit the legacy stage-cooldown lazy-derive converts to FSRS days.
const DAY_MS: u64 = 86_400 * 1000;

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
    }
}

/// Seeds an `rs-fsrs` `Card` for a card with no FSRS state yet. A never-reviewed card
/// starts `New` (FSRS seeds its initial stability from the grade); a card carrying legacy
/// Leitner progress is seeded `Review` with a stability derived from its stage cooldown,
/// so prior progress carries over roughly — no migration needed.
fn seed_card(state: &CardState, now_ms: u64) -> FsrsCard {
    let mut c = FsrsCard::new();
    c.last_review = ms_to_dt(now_ms);
    c.due = ms_to_dt(now_ms);
    if state.total_reviews > 0 {
        let days = (stage_cooldown_ms(state.stage) as f64 / DAY_MS as f64).max(0.1);
        c.stability = days;
        c.difficulty = 5.0;
        c.scheduled_days = days.round() as i64;
        c.state = RawState::Review;
        c.last_review = ms_to_dt(state.stage_entered_ms.max(1));
    }
    c
}

/// The FSRS scheduler, backed by `rs-fsrs`, built for a desired retention. Short-term
/// modeling is **on** (`enable_short_term = true`), so FSRS's built-in learning steps own
/// acquisition (a New card graded Good is due ~10 min out in `Learning`; a second Good
/// graduates it to `Review`) and the DSR model owns long-term review — one scheduler
/// across both the short and the long term.
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
    fn due_at(&self, state: &CardState) -> u64 {
        match &state.fsrs {
            Some(s) => s.due_ms,
            // Not yet reviewed under FSRS — fall back to the Leitner cooldown so a legacy
            // card still surfaces until its first FSRS review derives real state.
            None => state
                .stage_entered_ms
                .saturating_add(stage_cooldown_ms(state.stage)),
        }
    }

    fn apply(&self, state: &mut CardState, grade: Grade, now_ms: u64) {
        let card = match &state.fsrs {
            Some(s) => to_fsrs_card(s),
            None => seed_card(state, now_ms),
        };
        let info = self.fsrs.next(card, ms_to_dt(now_ms), rating_for(grade));
        state.fsrs = Some(from_fsrs_card(&info.card));
        state.record_review(now_ms, grade);
    }

    fn reanchor(&self, state: &mut CardState, now_ms: u64) {
        if let Some(f) = state.fsrs.as_mut() {
            let interval_ms = u64::from(f.scheduled_days) * DAY_MS;
            f.due_ms = now_ms.saturating_add(interval_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_cooldowns_are_stable() {
        assert_eq!(5 * 60 * 1000, stage_cooldown_ms(1));
        assert_eq!(3_600_000, stage_cooldown_ms(2));
        assert_eq!(21_600_000, stage_cooldown_ms(3));
        assert_eq!(86_400_000, stage_cooldown_ms(4));
        assert_eq!(604_800_000, stage_cooldown_ms(5));
    }

    #[test]
    fn partly_counts_as_a_pass() {
        // A partial is a weak success: it advances the streak and counts as a pass, but is still
        // logged as `Partial` (not `Pass`).
        let mut s = CardState::new(0);
        s.record_review(0, Grade::Pass); // build a streak
        assert_eq!(1, s.streak);
        assert_eq!(1, s.total_passes);
        s.record_review(1000, Grade::Partial);
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
        sched.apply(&mut s, Grade::Pass, 0);
        let f = s.fsrs.expect("fsrs state set");
        assert!(f.stability > 0.0, "stability should be positive");
        assert!(sched.due_at(&s) > 0, "should be scheduled into the future");
        assert_eq!(1, s.total_reviews);
        assert!(s.history.last().unwrap().grade.passed());
    }

    #[test]
    fn fsrs_partly_grows_stability_less_than_got_it() {
        // partly → Hard (a weak success) grows stability less than Pass → Good.
        let sched = Fsrs::new(0.9);
        let mut good = CardState::new(0);
        sched.apply(&mut good, Grade::Pass, 0);
        let mut hard = CardState::new(0);
        sched.apply(&mut hard, Grade::Partial, 0);
        assert!(good.fsrs.unwrap().stability > hard.fsrs.unwrap().stability);
    }

    #[test]
    fn fsrs_a_miss_shortens_the_next_interval() {
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Grade::Pass, 0);
        let pass_interval = sched.due_at(&s); // reviewed at t = 0
        sched.apply(&mut s, Grade::Fail, pass_interval); // miss it when due
        let fail_interval = sched.due_at(&s).saturating_sub(pass_interval);
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
        sched.apply(&mut s, Grade::Pass, 0);
        let f = s.fsrs.expect("fsrs state set");
        assert_eq!(1, f.state, "a first Good enters Learning, not Review");
        let due = sched.due_at(&s);
        assert!(due > 0 && due < DAY_MS, "learning step is sub-day (got {due} ms)");
    }

    #[test]
    fn fsrs_two_goods_graduate_to_review() {
        // A second Good graduates the card out of the learning steps into Review, now
        // scheduled inter-day.
        let sched = Fsrs::new(0.9);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Grade::Pass, 0);
        let step_due = sched.due_at(&s);
        sched.apply(&mut s, Grade::Pass, step_due);
        assert_eq!(2, s.fsrs.expect("fsrs state").state, "second Good reaches Review");
        assert!(
            sched.due_at(&s) - step_due >= DAY_MS,
            "a graduated card is scheduled at least a day out"
        );
    }

    #[test]
    fn fsrs_overdue_recall_beats_on_time_recall() {
        // The spacing effect (long-term / Review state): recalling a more-overdue card
        // (lower retrievability) grows stability more. Graduate two identical cards to
        // Review first, then re-pass at different lateness.
        let sched = Fsrs::new(0.9);
        let mut early = CardState::new(0);
        sched.apply(&mut early, Grade::Pass, 0); // New -> Learning
        let step_due = sched.due_at(&early);
        sched.apply(&mut early, Grade::Pass, step_due); // Learning -> Review
        let due = sched.due_at(&early);
        let mut late = early.clone();
        sched.apply(&mut early, Grade::Pass, due); // on time
        sched.apply(&mut late, Grade::Pass, due * 3); // well overdue
        assert!(
            late.fsrs.unwrap().stability > early.fsrs.unwrap().stability,
            "an overdue-but-recalled card should gain more stability"
        );
    }
}
