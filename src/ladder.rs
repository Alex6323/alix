//! The difficulty ladder: a card's depth **rung** (how deeply it's retrieved)
//! and its **reveal-method** (how the answer is uncovered), plus the resolution
//! from `(reveal, rung, answer shape)` to a concrete review check.
//!
//! Two axes the old `% mode:` directive conflated (see the ladder spec §8):
//! depth is learner-chosen (a config `[review] target` the scheduler climbs toward);
//! reveal is authored per card (`% reveal:`). v1 schedules only Recall/Reconstruction;
//! Recognition is the unscheduled acquire on-ramp.

use serde::{Deserialize, Serialize};

use crate::{answer::Mode, card::Card};

/// Retrieval depth. Nested: reconstruction ⊇ recall ⊇ recognition.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Serialize, Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Rung {
    /// Recognize — the unscheduled acquire on-ramp in v1 (L1-as-target is v2).
    Recognize,
    /// Recall — produce the answer from the cue. The v1 default target.
    #[default]
    Recall,
    /// Reconstruct — regenerate or explain the answer in full.
    Reconstruct,
}

impl Rung {
    /// The next rung up, saturating at `Reconstruct`.
    pub fn climb(self) -> Rung {
        match self {
            Rung::Recognize => Rung::Recall,
            Rung::Recall => Rung::Reconstruct,
            Rung::Reconstruct => Rung::Reconstruct,
        }
    }

    /// One rung down, floored at `Recall` (v1 never schedules `Recognize`).
    pub fn descend(self) -> Rung {
        match self {
            Rung::Reconstruct => Rung::Recall,
            Rung::Recall | Rung::Recognize => Rung::Recall,
        }
    }
}

/// The label of a rung, matching `Rung`'s serde wire form — the canonical
/// lowercase verbs (`"recognize"`/`"recall"`/`"reconstruct"`), not a noun like
/// "reconstruction". `pub(crate)`: reused by [`crate::serve`] to report a
/// card's rung badge in its JSON state. Mirrors [`crate::answer::mode_name`].
pub(crate) fn rung_name(rung: Rung) -> &'static str {
    match rung {
        Rung::Recognize => "recognize",
        Rung::Recall => "recall",
        Rung::Reconstruct => "reconstruct",
    }
}

/// The learner's depth target, clamped to what v1 schedules (L1-as-target is
/// v2). `Recognize` never becomes a live scheduling target — it's the
/// unscheduled acquire on-ramp — so it clamps up to `Recall`.
pub fn effective_target(cfg: &crate::config::ReviewConfig) -> Rung {
    match cfg.target {
        Rung::Recognize => Rung::Recall,
        other => other,
    }
}

/// How a card's answer is presented / uncovered — authored (`% reveal:`),
/// independent of depth. Composes with any rung.
#[derive(
    Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Reveal {
    /// Reveal the whole answer at once (default).
    #[default]
    Flip,
    /// Reveal by gap-fill in context (`{{}}` marks the gaps).
    Cloze,
    /// Reveal progressively, line by line (ordered material).
    Line,
}

/// Whether an answer is atomic (a single short line → typed exactly) vs rich
/// (multi-line / long → explained). The v1 structural heuristic (spec §3.9/§8),
/// no AI. Mirrors `choice::recognition_question`'s "atomic = single-line" bar.
fn answer_is_atomic(card: &Card) -> bool {
    card.back.len() == 1
}

/// The concrete review check for a card at its frontier `rung`, presented via
/// its `reveal`-method. Depth is derived, never authored (spec §8): the learner
/// sets the deck target, the scheduler drives the rung, and this maps
/// `(reveal, rung, answer shape)` to the existing [`Mode`] rendering.
pub fn check_for(reveal: Reveal, rung: Rung, card: &Card) -> Mode {
    match rung {
        // Recognition is the unscheduled acquire on-ramp in v1; if ever asked,
        // fall back to the recall check (choice rendering is acquire-only).
        Rung::Recognize | Rung::Recall => match reveal {
            // Cloze at Recall renders the gap and self-grades — represented by
            // `Flip` for grading; the cloze *presentation* is driven by the card
            // being a cloze sub-card, unchanged, so `check_for` only picks the
            // grade/interaction mode.
            Reveal::Flip | Reveal::Cloze => Mode::Flip,
            Reveal::Line => Mode::LineByLine,
        },
        Rung::Reconstruct => match reveal {
            Reveal::Cloze => Mode::Typing,
            Reveal::Flip | Reveal::Line => {
                if answer_is_atomic(card) {
                    Mode::Typing
                } else {
                    Mode::Explain
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rung_orders_recognition_below_recall_below_reconstruction() {
        assert!(Rung::Recognize < Rung::Recall);
        assert!(Rung::Recall < Rung::Reconstruct);
    }

    #[test]
    fn default_rung_is_recall() {
        assert_eq!(Rung::default(), Rung::Recall);
    }

    #[test]
    fn climb_saturates_at_reconstruction() {
        assert_eq!(Rung::Recognize.climb(), Rung::Recall);
        assert_eq!(Rung::Recall.climb(), Rung::Reconstruct);
        assert_eq!(Rung::Reconstruct.climb(), Rung::Reconstruct);
    }

    #[test]
    fn descend_floors_at_recall_in_v1() {
        // v1 never schedules L1: a fall from Reconstruction lands on Recall, and
        // Recall is the floor (a recall miss relearns in place; Recognize — never a
        // live rung in v1 — also resolves to Recall).
        assert_eq!(Rung::Reconstruct.descend(), Rung::Recall);
        assert_eq!(Rung::Recall.descend(), Rung::Recall);
        assert_eq!(Rung::Recognize.descend(), Rung::Recall);
    }

    #[test]
    fn default_reveal_is_flip() {
        assert_eq!(Reveal::default(), Reveal::Flip);
    }

    #[test]
    fn rung_name_matches_the_serde_wire_verbs() {
        assert_eq!(rung_name(Rung::Recognize), "recognize");
        assert_eq!(rung_name(Rung::Recall), "recall");
        assert_eq!(rung_name(Rung::Reconstruct), "reconstruct");
    }

    #[test]
    fn enum_wire_strings_are_the_canonical_lowercase_verbs() {
        // Later tasks (store round-trip, [review] config parsing) depend on these.
        for (rung, s) in [
            (Rung::Recognize, "\"recognize\""),
            (Rung::Recall, "\"recall\""),
            (Rung::Reconstruct, "\"reconstruct\""),
        ] {
            assert_eq!(serde_json::to_string(&rung).unwrap(), s);
            assert_eq!(serde_json::from_str::<Rung>(s).unwrap(), rung);
        }
        for (reveal, s) in [
            (Reveal::Flip, "\"flip\""),
            (Reveal::Cloze, "\"cloze\""),
            (Reveal::Line, "\"line\""),
        ] {
            assert_eq!(serde_json::to_string(&reveal).unwrap(), s);
            assert_eq!(serde_json::from_str::<Reveal>(s).unwrap(), reveal);
        }
    }

    #[test]
    fn recall_rung_maps_reveal_to_its_self_graded_check() {
        let flip = Card::plain("d".into(), "q".into(), vec!["a".into()], None, 1);
        assert_eq!(check_for(Reveal::Flip, Rung::Recall, &flip), Mode::Flip);
        assert_eq!(check_for(Reveal::Line, Rung::Recall, &flip), Mode::LineByLine);
    }

    #[test]
    fn reconstruction_rung_types_atomic_and_explains_rich() {
        let atomic = Card::plain("d".into(), "q".into(), vec!["Paris".into()], None, 1);
        let rich = Card::plain(
            "d".into(),
            "q".into(),
            vec!["a".into(), "b".into(), "c".into()],
            None,
            1,
        );
        assert_eq!(
            check_for(Reveal::Flip, Rung::Reconstruct, &atomic),
            Mode::Typing
        );
        assert_eq!(
            check_for(Reveal::Flip, Rung::Reconstruct, &rich),
            Mode::Explain
        );
    }

    #[test]
    fn effective_target_clamps_recognize_to_recall_in_v1() {
        let cfg = crate::config::ReviewConfig {
            target: Rung::Recognize,
            ..Default::default()
        };
        assert_eq!(effective_target(&cfg), Rung::Recall);
    }

    #[test]
    fn reconstruction_cloze_types_the_gap() {
        let cloze = Card::plain("d".into(), "q".into(), vec!["type the {{gap}}".into()], None, 1);
        assert_eq!(
            check_for(Reveal::Cloze, Rung::Reconstruct, &cloze),
            Mode::Typing
        );
    }
}
