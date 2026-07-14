//! Session depths — the depth of practice a learner picks per session.
//!
//! Recognize | Recall | Reconstruct are independent session types (spec
//! 2026-07-07-session-levels-spec.md §4): nothing climbs, nothing descends;
//! the depth is a property of the session, never of the card. `check_for`
//! derives the concrete check from (reveal, depth, answer shape).

use serde::{Deserialize, Serialize};

use crate::{answer::Mode, card::Card};

/// The depth a learner chose for this session. Recognize is unscheduled and
/// boolean; Recall and Reconstruct each own an independent FSRS schedule per
/// card (stationarity: one schedule, one task, forever).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
#[cfg_attr(feature = "full", clap(rename_all = "lowercase"))]
#[serde(rename_all = "lowercase")]
pub enum Depth {
    Recognize,
    #[default]
    Recall,
    Reconstruct,
}

impl Depth {
    /// Parses the directive/config value name (case-insensitive), mirroring
    /// the clap value names; the gated parity test keeps the two in step.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "recognize" => Some(Self::Recognize),
            "recall" => Some(Self::Recall),
            "reconstruct" => Some(Self::Reconstruct),
            _ => None,
        }
    }
}

/// The lowercase name of a depth, matching its serde/clap rendering — for
/// reporting the session's depth in a JSON state payload (see `crate::serve`).
pub fn depth_name(depth: Depth) -> &'static str {
    match depth {
        Depth::Recognize => "recognize",
        Depth::Recall => "recall",
        Depth::Reconstruct => "reconstruct",
    }
}

/// How a card's answer is presented / uncovered — authored (`% reveal:`),
/// independent of depth. Composes with any depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
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

impl Reveal {
    /// Parses the directive value name (case-insensitive), mirroring the clap
    /// value names; the gated parity test keeps the two in step.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "flip" => Some(Self::Flip),
            "cloze" => Some(Self::Cloze),
            "line" => Some(Self::Line),
            _ => None,
        }
    }
}

/// Whether an answer is atomic (a single short line → typed exactly) vs rich
/// (multi-line / long → explained). The structural heuristic (spec §4), no
/// AI. Mirrors `choice::recognition_question`'s "atomic = single-line" bar.
fn answer_is_atomic(card: &Card) -> bool {
    card.back.len() == 1
}

/// The check a card renders at a depth: the final matrix of the spec (§4).
/// Recognize always answers "pick it" — whether that becomes real MC or the
/// attempt→reveal fallback is the serve layer's distractor decision.
pub fn check_for(reveal: Reveal, depth: Depth, card: &Card) -> Mode {
    match depth {
        Depth::Recognize => Mode::Choice,
        Depth::Recall => match reveal {
            Reveal::Flip | Reveal::Cloze => Mode::Flip,
            Reveal::Line => Mode::LineByLine,
        },
        Depth::Reconstruct => match reveal {
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
    fn recognize_depth_always_renders_a_choice_check() {
        for reveal in [Reveal::Flip, Reveal::Cloze, Reveal::Line] {
            assert_eq!(
                Mode::Choice,
                check_for(reveal, Depth::Recognize, &card("a"))
            );
        }
    }

    #[test]
    fn recall_depth_maps_reveal_to_its_self_graded_check() {
        assert_eq!(
            Mode::Flip,
            check_for(Reveal::Flip, Depth::Recall, &card("a"))
        );
        assert_eq!(
            Mode::Flip,
            check_for(Reveal::Cloze, Depth::Recall, &card("a"))
        );
        assert_eq!(
            Mode::LineByLine,
            check_for(Reveal::Line, Depth::Recall, &card("a"))
        );
    }

    #[test]
    fn reconstruct_depth_types_atoms_ticks_rich_and_types_lines() {
        assert_eq!(
            Mode::Typing,
            check_for(Reveal::Flip, Depth::Reconstruct, &card("a"))
        );
        assert_eq!(
            Mode::Explain,
            check_for(Reveal::Flip, Depth::Reconstruct, &card("a\n    b"))
        );
        assert_eq!(
            Mode::Typing,
            check_for(Reveal::Cloze, Depth::Reconstruct, &card("a {{b}}"))
        );
        assert_eq!(
            Mode::TypeLine,
            check_for(Reveal::Line, Depth::Reconstruct, &card("a\n    b"))
        );
    }

    #[test]
    fn depth_serializes_lowercase_and_defaults_to_recall() {
        assert_eq!(Depth::default(), Depth::Recall);
        assert_eq!(
            "\"recognize\"",
            serde_json::to_string(&Depth::Recognize).unwrap()
        );
    }
}

#[cfg(all(test, feature = "full"))]
mod clap_parity {
    use clap::ValueEnum;

    use super::*;

    /// The hand-written `parse` and the clap value names must agree on every
    /// variant, or a `%` directive would parse differently from the CLI flag.
    #[test]
    fn parse_matches_the_clap_value_names() {
        for variant in Depth::value_variants() {
            let name = variant.to_possible_value().expect("a value name");
            assert_eq!(Some(*variant), Depth::parse(name.get_name()), "{name:?}");
        }
        assert_eq!(None, Depth::parse("no-such-value"));
        for variant in Reveal::value_variants() {
            let name = variant.to_possible_value().expect("a value name");
            assert_eq!(Some(*variant), Reveal::parse(name.get_name()), "{name:?}");
        }
        assert_eq!(None, Reveal::parse("no-such-value"));
    }
}
