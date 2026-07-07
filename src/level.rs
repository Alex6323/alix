//! Session levels — the depth of practice a learner picks per session.
//!
//! Recognize | Recall | Reconstruct are independent session types (spec
//! 2026-07-07-session-levels-spec.md §4): nothing climbs, nothing descends;
//! the level is a property of the session, never of the card. `check_for`
//! derives the concrete check from (reveal, level, answer shape).

use serde::{Deserialize, Serialize};

use crate::{answer::Mode, card::Card};

/// The depth a learner chose for this session. Recognize is unscheduled and
/// boolean; Recall and Reconstruct each own an independent FSRS schedule per
/// card (stationarity: one schedule, one task, forever).
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum Level {
    Recognize,
    #[default]
    Recall,
    Reconstruct,
}

/// The lowercase name of a level, matching its serde/clap rendering — for
/// reporting the session's level in a JSON state payload (see `crate::serve`).
pub fn level_name(level: Level) -> &'static str {
    match level {
        Level::Recognize => "recognize",
        Level::Recall => "recall",
        Level::Reconstruct => "reconstruct",
    }
}

/// How a card's answer is presented / uncovered — authored (`% reveal:`),
/// independent of depth. Composes with any level.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, clap::ValueEnum)]
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
/// (multi-line / long → explained). The structural heuristic (spec §4), no
/// AI. Mirrors `choice::recognition_question`'s "atomic = single-line" bar.
fn answer_is_atomic(card: &Card) -> bool {
    card.back.len() == 1
}

/// The check a card renders at a level: the final matrix of the spec (§4).
/// Recognize always answers "pick it" — whether that becomes real MC or the
/// attempt→reveal fallback is the serve layer's distractor decision.
pub fn check_for(reveal: Reveal, level: Level, card: &Card) -> Mode {
    match level {
        Level::Recognize => Mode::Choice,
        Level::Recall => match reveal {
            Reveal::Flip | Reveal::Cloze => Mode::Flip,
            Reveal::Line => Mode::LineByLine,
        },
        Level::Reconstruct => match reveal {
            Reveal::Cloze => Mode::Typing,
            Reveal::Line => Mode::TypeLine,
            Reveal::Flip => {
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
    use crate::{answer::Mode, parser};

    fn card(back: &str) -> crate::card::Card {
        let text = format!("# q\n{back}");
        parser::parse_str("t", &text).unwrap().remove(0)
    }

    #[test]
    fn recognize_level_always_renders_a_choice_check() {
        for reveal in [Reveal::Flip, Reveal::Cloze, Reveal::Line] {
            assert_eq!(
                Mode::Choice,
                check_for(reveal, Level::Recognize, &card("a"))
            );
        }
    }

    #[test]
    fn recall_level_maps_reveal_to_its_self_graded_check() {
        assert_eq!(
            Mode::Flip,
            check_for(Reveal::Flip, Level::Recall, &card("a"))
        );
        assert_eq!(
            Mode::Flip,
            check_for(Reveal::Cloze, Level::Recall, &card("a"))
        );
        assert_eq!(
            Mode::LineByLine,
            check_for(Reveal::Line, Level::Recall, &card("a"))
        );
    }

    #[test]
    fn reconstruct_level_types_atoms_ticks_rich_and_types_lines() {
        assert_eq!(
            Mode::Typing,
            check_for(Reveal::Flip, Level::Reconstruct, &card("a"))
        );
        assert_eq!(
            Mode::Explain,
            check_for(Reveal::Flip, Level::Reconstruct, &card("a\n    b"))
        );
        assert_eq!(
            Mode::Typing,
            check_for(Reveal::Cloze, Level::Reconstruct, &card("a {{b}}"))
        );
        assert_eq!(
            Mode::TypeLine,
            check_for(Reveal::Line, Level::Reconstruct, &card("a\n    b"))
        );
    }

    #[test]
    fn level_serializes_lowercase_and_defaults_to_recall() {
        assert_eq!(Level::default(), Level::Recall);
        assert_eq!(
            "\"recognize\"",
            serde_json::to_string(&Level::Recognize).unwrap()
        );
    }
}
