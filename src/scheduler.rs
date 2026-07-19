use chrono::{DateTime, Utc};
use rs_fsrs::{Card as FsrsCard, FSRS, Parameters, Rating, State as RawState};

use crate::{
    depth::Depth,
    store::{CardState, FsrsState},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Grade {
    Fail,
    Partial,
    Pass,
}

impl Grade {
    pub fn passed(self) -> bool {
        matches!(self, Grade::Pass | Grade::Partial)
    }
}

/// A `total` of 0 is a `Pass`: there's no rubric to miss.
pub fn keypoint_grade(covered: usize, total: usize) -> Grade {
    if total == 0 || covered >= total {
        Grade::Pass
    } else if covered == 0 {
        Grade::Fail
    } else {
        Grade::Partial
    }
}

/// `Send + Sync` so a `Session` (which boxes one) can cross threads (e.g. the
/// frb mobile bridge).
pub trait Scheduler: Send + Sync {
    fn due_at(&self, state: &CardState, depth: Depth) -> u64;

    /// `propagated` marks a review credited down from a pass at a higher
    /// depth, not answered directly; the schedule math is identical either way.
    fn apply(
        &self,
        state: &mut CardState,
        depth: Depth,
        grade: Grade,
        now_ms: u64,
        propagated: bool,
    );

    /// A card with no FSRS state yet at `depth` is left untouched: there's no
    /// interval to preserve.
    fn reanchor(&self, state: &mut CardState, depth: Depth, now_ms: u64);

    fn is_due(&self, state: &CardState, depth: Depth, now_ms: u64) -> bool {
        self.due_at(state, depth) <= now_ms
    }

    fn acquire_cooldown_ms(&self) -> u64 {
        DEFAULT_ACQUIRE_COOLDOWN_MS
    }
}

/// Doubles as `Session`'s same-card re-serve floor: one knob moves both gaps.
pub const DEFAULT_ACQUIRE_COOLDOWN_MS: u64 = 5 * 60 * 1000;

/// Deliberately a constant, not a config key: an expert knob with no
/// pre-exam feedback loop; base `retention` is the pressure valve.
pub const DEADLINE_RETENTION: f64 = 0.95;

/// The due ceiling compensates an rs-fsrs quirk: it clamps the interval
/// BEFORE separating grades, so a mature card graded Pass can still land
/// past `max_interval_days`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeadlineTuning {
    pub retention: f64,
    pub max_interval_days: i32,
    pub due_ceiling_ms: u64,
}

/// `None` past the date is deliberate: the ramp releases rather than
/// erroring. `today`/`due_ceiling_ms` are injected, not computed inline, so
/// this stays deterministic for tests.
pub fn deadline_tuning(
    deadline: chrono::NaiveDate,
    ramp_days: u32,
    base_retention: f64,
    today: chrono::NaiveDate,
    due_ceiling_ms: u64,
) -> Option<DeadlineTuning> {
    let days_left = (deadline - today).num_days();
    if days_left < 0 {
        return None;
    }
    let retention = if ramp_days == 0 || days_left >= i64::from(ramp_days) {
        base_retention
    } else {
        let w = f64::from(ramp_days);
        let progressed = (w - days_left as f64) / w;
        (base_retention + (DEADLINE_RETENTION - base_retention) * progressed).max(base_retention)
    };
    Some(DeadlineTuning {
        retention,
        max_interval_days: days_left.max(1) as i32,
        due_ceiling_ms,
    })
}

const DAY_MS: u64 = 86_400 * 1000;

/// Hold step for a card that passed once but hasn't earned its second Good yet
/// (10 min, matching FSRS's New+Good step).
const LEARNING_HOLD_MS: u64 = 10 * 60 * 1000;

fn ms_to_dt(ms: u64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms as i64).unwrap_or_default()
}

fn dt_to_ms(dt: DateTime<Utc>) -> u64 {
    dt.timestamp_millis().max(0) as u64
}

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

/// Always seeds `New`, ignoring `_state`: there is no pre-FSRS carry-over to
/// derive from.
fn seed_card(_state: &CardState, now_ms: u64) -> FsrsCard {
    let mut c = FsrsCard::new();
    c.last_review = ms_to_dt(now_ms);
    c.due = ms_to_dt(now_ms);
    c
}

pub struct Fsrs {
    fsrs: FSRS,
    acquire_cooldown_ms: u64,
    due_ceiling_ms: Option<u64>,
}

impl Fsrs {
    pub fn new(retention: f64, acquire_cooldown_ms: u64) -> Self {
        Self::tuned(retention, acquire_cooldown_ms, None)
    }

    pub fn tuned(
        retention: f64,
        acquire_cooldown_ms: u64,
        deadline: Option<DeadlineTuning>,
    ) -> Self {
        let parameters = Parameters {
            request_retention: deadline.map_or(retention, |t| t.retention),
            maximum_interval: deadline.map_or(36500, |t| t.max_interval_days),
            enable_short_term: true,
            ..Parameters::default()
        };
        Self {
            fsrs: FSRS::new(parameters),
            acquire_cooldown_ms,
            due_ceiling_ms: deadline.map(|t| t.due_ceiling_ms),
        }
    }
}

impl Default for Fsrs {
    fn default() -> Self {
        Self::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS)
    }
}

impl Scheduler for Fsrs {
    fn due_at(&self, state: &CardState, depth: Depth) -> u64 {
        match state.schedule(depth) {
            Some(s) => s.due_ms,
            // Established at the other depth: due now, skipping the acquire
            // warm-up (its own schedule is created lazily on the first grade).
            None if state.recall.is_some() || state.reconstruct.is_some() => 0,
            None => state.acquired_ms.saturating_add(self.acquire_cooldown_ms),
        }
    }

    fn acquire_cooldown_ms(&self) -> u64 {
        self.acquire_cooldown_ms
    }

    fn apply(
        &self,
        state: &mut CardState,
        depth: Depth,
        grade: Grade,
        now_ms: u64,
        propagated: bool,
    ) {
        let current = state.schedule(depth).copied();
        let (card, pre_state, prev_goods) = match &current {
            Some(s) => (to_fsrs_card(s), s.state, s.learning_goods),
            None => (seed_card(state, now_ms), 0, 0),
        };
        let info = self.fsrs.next(card, ms_to_dt(now_ms), rating_for(grade));
        let mut next = from_fsrs_card(&info.card);

        // Graduation gate: acquisition (state 0|1) needs two full Goods before
        // Review; a Fail resets the count, Relearning re-graduates on one Good.
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

        // Only clamps dues beyond the ceiling; `reanchor` is deliberately not
        // bounded by it.
        if let Some(ceiling) = self.due_ceiling_ms
            && next.due_ms > ceiling
        {
            next.due_ms = ceiling;
            next.scheduled_days = (ceiling.saturating_sub(now_ms) / DAY_MS) as u32;
        }

        // `Recognize` has no slot (unscheduled + boolean): a silent no-op,
        // never reached since Recognize is graded by pick, not `apply`.
        let Some(slot) = state.schedule_slot(depth) else {
            return;
        };
        *slot = Some(next);
        state.record_review(now_ms, grade, depth, propagated);
    }

    fn reanchor(&self, state: &mut CardState, depth: Depth, now_ms: u64) {
        if let Some(Some(f)) = state.schedule_slot(depth) {
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
        let mut s = CardState::new(1000);
        assert_eq!(
            sched.due_at(&s, Depth::Recall),
            1000 + DEFAULT_ACQUIRE_COOLDOWN_MS
        );
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 1000, false);
        assert!(s.recall.is_some());
        assert!(sched.due_at(&s, Depth::Recall) != 1000 + DEFAULT_ACQUIRE_COOLDOWN_MS);
    }

    #[test]
    fn due_at_is_immediate_at_a_depth_scheduled_elsewhere() {
        let sched = Fsrs::default();
        let mut s = CardState::new(1_000);
        s.recall = Some(FsrsState {
            due_ms: u64::MAX,
            ..Default::default()
        });
        assert_eq!(0, sched.due_at(&s, Depth::Reconstruct), "immediately due");
        assert!(sched.is_due(&s, Depth::Reconstruct, 1));
        assert_eq!(u64::MAX, sched.due_at(&s, Depth::Recall));
        let fresh = CardState::new(1_000);
        assert_eq!(
            1_000 + DEFAULT_ACQUIRE_COOLDOWN_MS,
            sched.due_at(&fresh, Depth::Reconstruct)
        );
    }

    #[test]
    fn apply_writes_only_the_chosen_depths_schedule() {
        let sched = Fsrs::default();
        let mut st = CardState::new(0);
        sched.apply(&mut st, Depth::Reconstruct, Grade::Pass, 1_000, false);
        assert!(st.schedule(Depth::Reconstruct).is_some());
        assert!(st.schedule(Depth::Recall).is_none(), "no cross-crediting");
        assert_eq!(Depth::Reconstruct, st.history[0].depth);
    }

    #[test]
    fn apply_on_recognize_is_a_no_op() {
        let sched = Fsrs::default();
        let mut st = CardState::new(0);
        sched.apply(&mut st, Depth::Recognize, Grade::Pass, 1_000, false);
        assert!(st.recall.is_none() && st.reconstruct.is_none() && st.history.is_empty());
    }

    #[test]
    fn apply_stores_the_propagated_marker_on_the_review() {
        let sched = Fsrs::default();
        let mut direct = CardState::new(0);
        sched.apply(&mut direct, Depth::Recall, Grade::Pass, 1_000, false);
        assert!(!direct.history[0].propagated, "a direct review is unmarked");
        let mut credited = CardState::new(0);
        sched.apply(&mut credited, Depth::Recall, Grade::Pass, 1_000, true);
        assert!(
            credited.history[0].propagated,
            "a credited review is marked"
        );
        assert_eq!(direct.recall, credited.recall, "same schedule either way");
    }

    #[test]
    fn the_default_acquire_cooldown_is_five_minutes() {
        // Deliberately pinned: raised from the old 1-min gap on 2026-07-14
        // (user request). Changing it changes every default session's rhythm.
        assert_eq!(5 * 60 * 1000, DEFAULT_ACQUIRE_COOLDOWN_MS);
    }

    #[test]
    fn a_configured_cooldown_moves_the_first_quiz_and_the_floor() {
        let sched = Fsrs::new(0.9, 90_000);
        let fresh = CardState::new(1_000);
        assert_eq!(1_000 + 90_000, sched.due_at(&fresh, Depth::Recall));
        assert_eq!(90_000, sched.acquire_cooldown_ms());
        assert_eq!(
            DEFAULT_ACQUIRE_COOLDOWN_MS,
            Fsrs::default().acquire_cooldown_ms()
        );
    }

    #[test]
    fn partly_counts_as_a_pass() {
        let mut s = CardState::new(0);
        s.record_review(0, Grade::Pass, Depth::Recall, false);
        assert_eq!(1, s.streak);
        assert_eq!(1, s.total_passes);
        s.record_review(1000, Grade::Partial, Depth::Recall, false);
        assert_eq!(2, s.streak);
        assert_eq!(2, s.total_passes);
        assert!(s.history.last().unwrap().grade.passed());
        assert_eq!(Grade::Partial, s.history.last().unwrap().grade);
    }

    #[test]
    fn keypoint_grade_derives_from_coverage() {
        assert_eq!(Grade::Fail, keypoint_grade(0, 5));
        assert_eq!(Grade::Partial, keypoint_grade(1, 5));
        assert_eq!(Grade::Partial, keypoint_grade(4, 5));
        assert_eq!(Grade::Pass, keypoint_grade(5, 5));
        assert_eq!(Grade::Pass, keypoint_grade(0, 0));
    }

    #[test]
    fn fsrs_pass_on_a_new_card_sets_stability_and_schedules_out() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 0, false);
        let f = s.recall.expect("fsrs state set");
        assert!(f.stability > 0.0, "stability should be positive");
        assert!(
            sched.due_at(&s, Depth::Recall) > 0,
            "should be scheduled into the future"
        );
        assert_eq!(1, s.total_reviews);
        assert!(s.history.last().unwrap().grade.passed());
    }

    #[test]
    fn fsrs_partly_grows_stability_less_than_got_it() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut good = CardState::new(0);
        sched.apply(&mut good, Depth::Recall, Grade::Pass, 0, false);
        let mut hard = CardState::new(0);
        sched.apply(&mut hard, Depth::Recall, Grade::Partial, 0, false);
        assert!(good.recall.unwrap().stability > hard.recall.unwrap().stability);
    }

    #[test]
    fn fsrs_a_miss_shortens_the_next_interval() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 0, false);
        let pass_interval = sched.due_at(&s, Depth::Recall);
        sched.apply(&mut s, Depth::Recall, Grade::Fail, pass_interval, false);
        let fail_interval = sched
            .due_at(&s, Depth::Recall)
            .saturating_sub(pass_interval);
        assert!(!s.history.last().unwrap().grade.passed());
        assert!(
            fail_interval < pass_interval,
            "a lapse should shorten the interval (fail {fail_interval} vs pass {pass_interval})"
        );
    }

    #[test]
    fn fsrs_new_card_good_enters_a_learning_step() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 0, false);
        let f = s.recall.expect("fsrs state set");
        assert_eq!(1, f.state, "a first Good enters Learning, not Review");
        let due = sched.due_at(&s, Depth::Recall);
        assert!(
            due > 0 && due < DAY_MS,
            "learning step is sub-day (got {due} ms)"
        );
    }

    #[test]
    fn fsrs_two_goods_graduate_to_review() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 0, false);
        let step_due = sched.due_at(&s, Depth::Recall);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, step_due, false);
        assert_eq!(
            2,
            s.recall.expect("fsrs state").state,
            "second Good reaches Review"
        );
        assert!(
            sched.due_at(&s, Depth::Recall) - step_due >= DAY_MS,
            "a graduated card is scheduled at least a day out"
        );
    }

    #[test]
    fn fail_then_one_good_does_not_graduate() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Depth::Recall, Grade::Fail, 0, false);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 60_000, false);
        let f = s.recall.unwrap();
        assert_eq!(1, f.state, "one Good after a fail stays in Learning");
        assert_eq!(1, f.learning_goods);
    }

    #[test]
    fn two_goods_after_a_fail_do_graduate() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Depth::Recall, Grade::Fail, 0, false);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 60_000, false);
        assert_eq!(1, s.recall.unwrap().state);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 700_000, false);
        assert_eq!(2, s.recall.unwrap().state, "two Goods graduate");
    }

    #[test]
    fn a_fail_resets_graduation_progress() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 0, false);
        sched.apply(&mut s, Depth::Recall, Grade::Fail, 60_000, false);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 120_000, false);
        assert_eq!(1, s.recall.unwrap().state, "still Learning after the reset");
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 700_000, false);
        assert_eq!(2, s.recall.unwrap().state);
    }

    #[test]
    fn partial_is_neutral_for_graduation() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 0, false);
        sched.apply(&mut s, Depth::Recall, Grade::Partial, 600_000, false);
        assert_eq!(1, s.recall.unwrap().state);
        assert_eq!(1, s.recall.unwrap().learning_goods);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 1_200_000, false);
        assert_eq!(2, s.recall.unwrap().state);
    }

    #[test]
    fn a_lapsed_card_regraduates_on_one_good() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut s = CardState::new(0);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 0, false);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 600_000, false);
        sched.apply(&mut s, Depth::Recall, Grade::Fail, 1_200_000, false);
        assert_eq!(3, s.recall.unwrap().state);
        sched.apply(&mut s, Depth::Recall, Grade::Pass, 1_800_000, false);
        assert_eq!(2, s.recall.unwrap().state);
    }

    #[test]
    fn fsrs_overdue_recall_beats_on_time_recall() {
        let sched = Fsrs::new(0.9, DEFAULT_ACQUIRE_COOLDOWN_MS);
        let mut early = CardState::new(0);
        sched.apply(&mut early, Depth::Recall, Grade::Pass, 0, false);
        let step_due = sched.due_at(&early, Depth::Recall);
        sched.apply(&mut early, Depth::Recall, Grade::Pass, step_due, false);
        let due = sched.due_at(&early, Depth::Recall);
        let mut late = early.clone();
        sched.apply(&mut early, Depth::Recall, Grade::Pass, due, false);
        sched.apply(&mut late, Depth::Recall, Grade::Pass, due * 3, false);
        assert!(
            late.recall.unwrap().stability > early.recall.unwrap().stability,
            "an overdue-but-recalled card should gain more stability"
        );
    }

    #[test]
    fn deadline_tuning_caps_the_interval_at_days_left() {
        let d = |y, m, dd| chrono::NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        let t = deadline_tuning(d(2026, 9, 1), 14, 0.9, d(2026, 8, 12), 1_000).unwrap();
        assert_eq!(20, t.max_interval_days);
        assert_eq!(0.9, t.retention, "outside the window the base holds");
        assert_eq!(1_000, t.due_ceiling_ms);
    }

    #[test]
    fn deadline_tuning_ramps_retention_inside_the_window() {
        let d = |y, m, dd| chrono::NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        let mid = deadline_tuning(d(2026, 9, 1), 14, 0.9, d(2026, 8, 25), 0).unwrap();
        assert!((mid.retention - 0.925).abs() < 1e-9);
        let last = deadline_tuning(d(2026, 9, 1), 14, 0.9, d(2026, 9, 1), 0).unwrap();
        assert_eq!(DEADLINE_RETENTION, last.retention);
        assert_eq!(1, last.max_interval_days);
        let edge = deadline_tuning(d(2026, 9, 1), 14, 0.9, d(2026, 8, 18), 0).unwrap();
        assert_eq!(0.9, edge.retention);
    }

    #[test]
    fn deadline_tuning_never_lowers_a_higher_base_and_zero_ramp_is_cap_only() {
        let d = |y, m, dd| chrono::NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        let high = deadline_tuning(d(2026, 9, 1), 14, 0.97, d(2026, 9, 1), 0).unwrap();
        assert_eq!(0.97, high.retention, "a personal 0.97 is kept");
        let capped = deadline_tuning(d(2026, 9, 1), 0, 0.9, d(2026, 9, 1), 0).unwrap();
        assert_eq!(0.9, capped.retention, "ramp 0 = cap only");
        assert_eq!(1, capped.max_interval_days);
    }

    #[test]
    fn deadline_tuning_releases_past_the_date() {
        let d = |y, m, dd| chrono::NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert!(deadline_tuning(d(2026, 9, 1), 14, 0.9, d(2026, 9, 2), 0).is_none());
    }

    #[test]
    fn a_deadline_ceiling_bounds_the_due_date_where_the_raw_cap_does_not() {
        let now = 100 * 86_400_000u64;
        let ceiling = now + 3 * 86_400_000;
        let sched = Fsrs::tuned(
            0.9,
            1_000,
            Some(DeadlineTuning {
                retention: 0.9,
                max_interval_days: 3,
                due_ceiling_ms: ceiling,
            }),
        );
        let mut st = CardState::new(0);
        st.recall = Some(FsrsState {
            stability: 200.0,
            difficulty: 5.0,
            state: 2,
            reps: 10,
            scheduled_days: 90,
            last_review_ms: 0,
            due_ms: 0,
            ..Default::default()
        });
        sched.apply(&mut st, Depth::Recall, Grade::Pass, now, false);
        let s = st.recall.unwrap();
        assert!(
            s.due_ms <= ceiling,
            "due {} must not pass the ceiling {ceiling}",
            s.due_ms
        );
        assert!(
            s.scheduled_days <= 3,
            "scheduled_days consistent with the ceiling"
        );

        let mut free = CardState::new(0);
        free.recall = Some(FsrsState {
            stability: 200.0,
            difficulty: 5.0,
            state: 2,
            reps: 10,
            scheduled_days: 90,
            last_review_ms: 0,
            due_ms: 0,
            ..Default::default()
        });
        Fsrs::new(0.9, 1_000).apply(&mut free, Depth::Recall, Grade::Pass, now, false);
        assert!(free.recall.unwrap().scheduled_days > 3);
    }
}
