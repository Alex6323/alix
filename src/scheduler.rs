//! Review scheduling.
//!
//! Two schedulers are available:
//!
//! - [`Leitner`]: a 6-stage box system. Stage cooldowns are 5m / 1h / 6h / 24h / 1w. Passing moves
//!   a card up one stage (it stays in stage 5 once there), failing sends it back to stage 1 (the 5m
//!   cooldown is a short relearn gap before the next session, not an in-session delay).
//! - [`Sm2`]: a SuperMemo-2 style scheduler with per-card ease factors and growing intervals.
//!
//! Both schedulers keep the card's Leitner `stage` field up to date, so it is
//! safe to switch between them at any time.

use crate::store::{CardState, MAX_STAGE, Sm2State};

/// The outcome of reviewing a card. Three honest outcomes, shared by fact-deck
/// review and the trace walk: **failed** / **partly** / **got it**.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Grade {
    /// Wrong (or the user asked for hints / graded "failed"). Resets to stage 1.
    Fail,
    /// Only partly right — a soft miss that demotes the card *one* stage instead
    /// of resetting it. Not a clean pass: it does not advance the streak.
    Partial,
    /// Fully correct ("got it"). Advances one stage.
    Pass,
}

impl Grade {
    /// Whether this grade counts as a clean pass (advances the streak). A
    /// `Partial` is deliberately *not* a clean pass — it kept most of your
    /// progress, but you didn't fully have it.
    pub fn passed(self) -> bool {
        matches!(self, Grade::Pass)
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

/// Which scheduling algorithm to use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum SchedulerKind {
    /// A 6-stage Leitner box (cooldowns 5m / 1h / 6h / 24h / 1w).
    #[default]
    Leitner,
    /// SuperMemo-2 style intervals with per-card ease factors.
    Sm2,
}

/// A scheduling algorithm.
pub trait Scheduler {
    /// When the card is due next (Unix ms).
    fn due_at(&self, state: &CardState) -> u64;

    /// Applies a review outcome to the card state at time `now_ms`.
    fn apply(&self, state: &mut CardState, grade: Grade, now_ms: u64);

    /// Whether the card is due at `now_ms`.
    fn is_due(&self, state: &CardState, now_ms: u64) -> bool {
        self.due_at(state) <= now_ms
    }
}

impl SchedulerKind {
    /// Returns the scheduler implementation for this kind.
    pub fn scheduler(self) -> Box<dyn Scheduler> {
        match self {
            SchedulerKind::Leitner => Box::new(Leitner),
            SchedulerKind::Sm2 => Box::new(Sm2),
        }
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

/// The 6-stage Leitner box scheduler.
pub struct Leitner;

impl Scheduler for Leitner {
    fn due_at(&self, state: &CardState) -> u64 {
        state
            .stage_entered_ms
            .saturating_add(stage_cooldown_ms(state.stage))
    }

    fn apply(&self, state: &mut CardState, grade: Grade, now_ms: u64) {
        state.stage = match grade {
            Grade::Fail => 1,
            Grade::Partial => state.stage.saturating_sub(1).max(1),
            Grade::Pass => (state.stage + 1).min(MAX_STAGE),
        };
        state.stage_entered_ms = now_ms;
        state.record_review(now_ms, grade);
    }
}

/// A SuperMemo-2 style scheduler.
///
/// Quality mapping: `Fail` = 2, `Partial` = 3, `Pass` = 4. A failed card resets
/// its repetition count and becomes due in 10 minutes; a passed card follows the
/// classic 1 day / 6 days / `interval * ease` progression; a partial keeps its
/// reps but halves the interval (the SM-2 twin of Leitner's one-stage demotion).
pub struct Sm2;

/// How soon a failed card comes back, in milliseconds (10 minutes).
const RELEARN_MS: u64 = 10 * 60 * 1000;
const DAY_MS: u64 = 86_400 * 1000;
const MIN_EASE: f64 = 1.3;
const DEFAULT_EASE: f64 = 2.5;

impl Sm2 {
    /// Derives an initial SM-2 state from a card's Leitner stage, so that
    /// previously learned cards don't restart from scratch when switching
    /// schedulers. The stage cooldown becomes the current interval.
    fn derive(state: &CardState) -> Sm2State {
        let interval_ms = stage_cooldown_ms(state.stage);
        Sm2State {
            ease: DEFAULT_EASE,
            reps: state.stage.saturating_sub(1) as u32,
            interval_ms,
            due_ms: state.stage_entered_ms.saturating_add(interval_ms),
        }
    }
}

impl Scheduler for Sm2 {
    fn due_at(&self, state: &CardState) -> u64 {
        match &state.sm2 {
            Some(sm2) => sm2.due_ms,
            None => Self::derive(state).due_ms,
        }
    }

    fn apply(&self, state: &mut CardState, grade: Grade, now_ms: u64) {
        let mut sm2 = state.sm2.unwrap_or_else(|| Self::derive(state));

        let quality: f64 = match grade {
            Grade::Fail => 2.0,
            Grade::Partial => 3.0,
            Grade::Pass => 4.0,
        };

        // Classic SM-2 ease update; applied for every review.
        sm2.ease += 0.1 - (5.0 - quality) * (0.08 + (5.0 - quality) * 0.02);
        sm2.ease = sm2.ease.max(MIN_EASE);

        match grade {
            Grade::Fail => {
                sm2.reps = 0;
                sm2.interval_ms = RELEARN_MS;
            }
            // A partial keeps its reps but halves the interval (floored at the
            // relearn gap) — it comes back sooner than a clean pass would allow,
            // without the full relearn a fail triggers.
            Grade::Partial => {
                sm2.interval_ms = (sm2.interval_ms / 2).max(RELEARN_MS);
            }
            Grade::Pass => {
                sm2.reps += 1;
                sm2.interval_ms = match sm2.reps {
                    1 => DAY_MS,
                    2 => 6 * DAY_MS,
                    _ => {
                        let grown = (sm2.interval_ms as f64 * sm2.ease) as u64;
                        grown.max(DAY_MS)
                    }
                };
            }
        }
        sm2.due_ms = now_ms.saturating_add(sm2.interval_ms);
        state.sm2 = Some(sm2);

        // Keep the Leitner stage in sync so switching schedulers stays sane.
        state.stage = match grade {
            Grade::Fail => 1,
            Grade::Partial => state.stage.saturating_sub(1).max(1),
            Grade::Pass => (state.stage + 1).min(MAX_STAGE),
        };
        state.stage_entered_ms = now_ms;
        state.record_review(now_ms, grade);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_at_stage(stage: u8, entered_ms: u64) -> CardState {
        let mut s = CardState::new(entered_ms);
        s.stage = stage;
        s
    }

    #[test]
    fn leitner_cooldowns_are_stable() {
        assert_eq!(5 * 60 * 1000, stage_cooldown_ms(1));
        assert_eq!(3_600_000, stage_cooldown_ms(2));
        assert_eq!(21_600_000, stage_cooldown_ms(3));
        assert_eq!(86_400_000, stage_cooldown_ms(4));
        assert_eq!(604_800_000, stage_cooldown_ms(5));
    }

    #[test]
    fn leitner_due() {
        let s = state_at_stage(2, 1_000_000);
        assert!(!Leitner.is_due(&s, 1_000_000));
        assert!(!Leitner.is_due(&s, 1_000_000 + 3_599_999));
        assert!(Leitner.is_due(&s, 1_000_000 + 3_600_000));

        // Stage 1 carries the short 5-minute relearn/settle gap (so a restart
        // right after acquiring or failing a card doesn't re-serve it instantly).
        let s = state_at_stage(1, 1_000_000);
        assert!(!Leitner.is_due(&s, 1_000_000));
        assert!(!Leitner.is_due(&s, 1_000_000 + 5 * 60 * 1000 - 1));
        assert!(Leitner.is_due(&s, 1_000_000 + 5 * 60 * 1000));
    }

    #[test]
    fn leitner_pass_moves_up_one_stage() {
        let mut s = state_at_stage(1, 0);
        Leitner.apply(&mut s, Grade::Pass, 5000);
        assert_eq!(2, s.stage);
        assert_eq!(5000, s.stage_entered_ms);
        assert_eq!(1, s.total_passes);
    }

    #[test]
    fn leitner_pass_caps_at_stage_5() {
        let mut s = state_at_stage(5, 0);
        Leitner.apply(&mut s, Grade::Pass, 5000);
        assert_eq!(5, s.stage);
        assert_eq!(5000, s.stage_entered_ms);
    }

    #[test]
    fn leitner_partly_drops_one_stage() {
        let mut s = state_at_stage(4, 0);
        Leitner.apply(&mut s, Grade::Partial, 5000);
        assert_eq!(3, s.stage);
        assert_eq!(5000, s.stage_entered_ms);
    }

    #[test]
    fn leitner_partly_floors_at_stage_one() {
        let mut s = state_at_stage(1, 0);
        Leitner.apply(&mut s, Grade::Partial, 5000);
        assert_eq!(1, s.stage);
    }

    #[test]
    fn leitner_fail_resets_to_stage_1() {
        let mut s = state_at_stage(4, 0);
        Leitner.apply(&mut s, Grade::Fail, 5000);
        assert_eq!(1, s.stage);
        assert_eq!(0, s.streak);
    }

    #[test]
    fn partly_is_not_a_clean_pass() {
        let mut s = state_at_stage(3, 0);
        Leitner.apply(&mut s, Grade::Pass, 0); // build a streak
        assert_eq!(1, s.streak);
        assert_eq!(1, s.total_passes);
        Leitner.apply(&mut s, Grade::Partial, 1000);
        assert_eq!(0, s.streak); // partly resets the streak
        assert_eq!(1, s.total_passes); // ...and doesn't count as a pass
        assert!(!s.history.last().unwrap().grade.passed()); // logged as not-passed
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
    fn sm2_derives_from_stage() {
        // A stage-3 card reviewed at t=0 is due after the 6h stage cooldown.
        let s = state_at_stage(3, 0);
        assert_eq!(21_600_000, Sm2.due_at(&s));
        assert!(!Sm2.is_due(&s, 21_599_999));
        assert!(Sm2.is_due(&s, 21_600_000));
    }

    #[test]
    fn sm2_progression() {
        let mut s = state_at_stage(1, 0);

        Sm2.apply(&mut s, Grade::Pass, 0);
        let sm2 = s.sm2.unwrap();
        assert_eq!(1, sm2.reps);
        assert_eq!(DAY_MS, sm2.interval_ms);
        assert_eq!(DAY_MS, sm2.due_ms);

        Sm2.apply(&mut s, Grade::Pass, DAY_MS);
        let sm2 = s.sm2.unwrap();
        assert_eq!(2, sm2.reps);
        assert_eq!(6 * DAY_MS, sm2.interval_ms);

        Sm2.apply(&mut s, Grade::Pass, 7 * DAY_MS);
        let sm2 = s.sm2.unwrap();
        assert_eq!(3, sm2.reps);
        // Third interval grows by the ease factor.
        assert!(sm2.interval_ms > 6 * DAY_MS);
    }

    #[test]
    fn sm2_fail_relearns_quickly() {
        let mut s = state_at_stage(4, 0);
        Sm2.apply(&mut s, Grade::Fail, 1000);
        let sm2 = s.sm2.unwrap();
        assert_eq!(0, sm2.reps);
        assert_eq!(RELEARN_MS, sm2.interval_ms);
        assert_eq!(1000 + RELEARN_MS, sm2.due_ms);
        // Stage stays in sync for scheduler switching.
        assert_eq!(1, s.stage);
    }

    #[test]
    fn sm2_ease_never_below_minimum() {
        let mut s = state_at_stage(1, 0);
        for i in 0..20 {
            Sm2.apply(&mut s, Grade::Fail, i);
        }
        assert!(s.sm2.unwrap().ease >= MIN_EASE);
    }

    #[test]
    fn sm2_partly_trims_interval_without_resetting_reps() {
        let mut s = state_at_stage(1, 0);
        Sm2.apply(&mut s, Grade::Pass, 0); // reps 1, interval 1 day
        Sm2.apply(&mut s, Grade::Pass, DAY_MS); // reps 2, interval 6 days
        assert_eq!(2, s.sm2.unwrap().reps);
        assert_eq!(6 * DAY_MS, s.sm2.unwrap().interval_ms);

        Sm2.apply(&mut s, Grade::Partial, 7 * DAY_MS);
        let after = s.sm2.unwrap();
        assert_eq!(2, after.reps); // reps preserved (not reset, not advanced)
        assert_eq!(3 * DAY_MS, after.interval_ms); // halved
        assert_eq!(2, s.stage); // stage mirror dropped one (3 -> 2)
    }

    #[test]
    fn sm2_partly_nudges_ease_down() {
        let mut s = state_at_stage(3, 0);
        Sm2.apply(&mut s, Grade::Partial, 0);
        assert!(s.sm2.unwrap().ease < DEFAULT_EASE);
    }

    #[test]
    fn sm2_partly_interval_floors_at_relearn() {
        let mut s = state_at_stage(1, 0);
        // Fresh derive at stage 1 has a zero interval; a partial floors it.
        Sm2.apply(&mut s, Grade::Partial, 0);
        assert_eq!(RELEARN_MS, s.sm2.unwrap().interval_ms);
    }
}
