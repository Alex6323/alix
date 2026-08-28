use serde::{Deserialize, Serialize};

use crate::{answer::Mode, augment::AugmentCache, card::Card};

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
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "recognize" => Some(Self::Recognize),
            "recall" => Some(Self::Recall),
            "reconstruct" => Some(Self::Reconstruct),
            _ => None,
        }
    }

    // Propagation targets: a pass proves every shallower ability, so credit
    // flows to each of them, never upward (ADR 0033).
    pub fn shallower(self) -> impl Iterator<Item = Depth> {
        match self {
            Depth::Recognize => [].as_slice(),
            Depth::Recall => [Depth::Recognize].as_slice(),
            Depth::Reconstruct => [Depth::Recall, Depth::Recognize].as_slice(),
        }
        .iter()
        .copied()
    }
}

pub fn depth_name(depth: Depth) -> &'static str {
    match depth {
        Depth::Recognize => "recognize",
        Depth::Recall => "recall",
        Depth::Reconstruct => "reconstruct",
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Reveal {
    #[default]
    Flip,
    Line,
}

impl Reveal {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "flip" => Some(Self::Flip),
            "line" => Some(Self::Line),
            _ => None,
        }
    }
}

/// How many answer lines a Reconstruct check would grade, counted in the
/// space that check grades: the deck's AUTHORED answer, never the `format`
/// augment's reshape. A cloze card's target is its hidden spans, however many
/// it asks, not its back lines.
fn gradeable_count(card: &Card) -> usize {
    if card.hole.is_some() {
        return 1;
    }
    crate::render::card_block_flags(card, crate::card::AnswerSpace::Authored)
        .iter()
        .filter(|in_block| !**in_block)
        .count()
}

pub fn check_for(reveal: Reveal, depth: Depth, card: &Card) -> Mode {
    match depth {
        Depth::Recognize => Mode::Choice,
        Depth::Recall => match reveal {
            Reveal::Flip => Mode::Flip,
            Reveal::Line => Mode::LineByLine,
        },
        Depth::Reconstruct => match gradeable_count(card) {
            0 => Mode::Explain,
            1 if reveal == Reveal::Flip => Mode::Typing,
            _ if reveal == Reveal::Line => Mode::TypeLine,
            _ => Mode::Explain,
        },
    }
}

pub fn card_recognizable(card: &Card, cache: &AugmentCache, deck_cards: &[Card]) -> bool {
    // Region cards are deliberately excluded, even when distractors are
    // cached or authored.
    if card.region.is_some() {
        return false;
    }
    // Select-all builds only from its authored option set; AI and sampled
    // pools are single-answer-shaped.
    if card.multiple_choice {
        return !card.authored_distractors.is_empty();
    }
    if !card.authored_distractors.is_empty() {
        return true;
    }
    if card
        .id()
        .and_then(|id| cache.distractors(&id, card.content_fingerprint))
        .is_some_and(|ai| crate::choice::can_build(card, ai))
    {
        return true;
    }
    crate::choice::can_sample(card, deck_cards)
}

pub fn deck_recognizable(cards: &[Card], cache: &AugmentCache) -> bool {
    cards.iter().any(|c| card_recognizable(c, cache, cards))
}

pub fn default_depth(cards: &[Card], cache: &AugmentCache) -> Depth {
    if deck_recognizable(cards, cache) {
        Depth::Recognize
    } else {
        Depth::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{answer::Mode, parser};

    fn card(back: &str) -> crate::card::Card {
        let slug: String = back
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let text = format!("## q\n{back}\n<!-- id: card-q{slug}x -->\n");
        parser::parse_str("t.md", &text).unwrap().remove(0)
    }

    #[test]
    fn a_region_card_is_never_recognizable_even_with_cached_distractors() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let text = "## q\n![](a.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 hidden=\"ans\" b:a1b2c3 -->\n\n---\nback\n<!-- id: card-qregionx -->\n";
        let cards = parser::parse_str("t.md", text).unwrap();
        let region_card = cards
            .iter()
            .find(|card| card.region.is_some())
            .expect("the blank produced a region card");
        cache.set_distractors(
            &region_card.id().unwrap(),
            vec!["x".into(), "y".into(), "z".into()],
            region_card.content_fingerprint,
        );
        assert!(
            !card_recognizable(region_card, &cache, &cards),
            "the choice gate holds even against cached distractors"
        );
        assert_eq!(Depth::Recall, default_depth(&cards, &cache));
    }

    #[test]
    fn default_depth_is_recognize_when_any_card_is_recognizable() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let covered = card("a");
        let uncovered = card("b");
        cache.set_distractors(
            &covered.id().unwrap(),
            vec!["x".into(), "y".into(), "z".into()],
            covered.content_fingerprint,
        );
        let cards = vec![covered, uncovered];
        assert_eq!(Depth::Recognize, default_depth(&cards, &cache));
    }

    #[test]
    fn default_depth_stays_recall_when_distractors_cannot_build_a_pick() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let covered = card("a");
        cache.set_distractors(
            &covered.id().unwrap(),
            vec!["x".into(), "y".into()],
            covered.content_fingerprint,
        );
        let cards = vec![covered];
        assert_eq!(Depth::Recall, default_depth(&cards, &cache));
    }

    #[test]
    fn default_depth_stays_recall_without_any_cached_distractors() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("deck1.json"));
        let cards = vec![card("a"), card("b")];
        assert_eq!(Depth::Recall, default_depth(&cards, &cache));
    }

    #[test]
    fn default_depth_is_recognize_for_authored_distractors_without_a_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("deck1.json"));
        let mut authored = card("a");
        authored.authored_distractors = vec!["b".into()];
        assert_eq!(Depth::Recognize, default_depth(&[authored], &cache));
    }

    #[test]
    fn a_table_deck_is_recognizable_by_column_sampling_alone() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("deck1.json"));
        let text = "| w | m |\n|---|---|\n| a | alpha | <!-- r:aaaaaa -->\n| b | beta | <!-- r:bbbbbb -->\n| c | gamma | <!-- r:cccccc -->\n| d | delta | <!-- r:dddddd -->\n<!-- cards -->\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n";
        let cards = parser::parse_str("t.md", text).unwrap();
        assert!(
            cards
                .iter()
                .all(|card| card_recognizable(card, &cache, &cards)),
            "four rows give every card a three-value pool"
        );
        assert_eq!(Depth::Recognize, default_depth(&cards, &cache));
    }

    #[test]
    fn a_three_row_table_cannot_fill_a_pick_and_stays_recall() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("deck1.json"));
        let text = "| w | m |\n|---|---|\n| a | alpha | <!-- r:aaaaaa -->\n| b | beta | <!-- r:bbbbbb -->\n| c | gamma | <!-- r:cccccc -->\n<!-- cards -->\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n";
        let cards = parser::parse_str("t.md", text).unwrap();
        assert!(
            cards
                .iter()
                .all(|card| !card_recognizable(card, &cache, &cards)),
            "two sibling values cannot fill three distractor slots"
        );
        assert_eq!(Depth::Recall, default_depth(&cards, &cache));
    }

    #[test]
    fn the_sampling_switch_resolves_table_over_deck_in_both_directions() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("deck1.json"));
        let rows = "| a | alpha | <!-- r:aaaaaa -->\n| b | beta | <!-- r:bbbbbb -->\n| c | gamma | <!-- r:cccccc -->\n| d | delta | <!-- r:dddddd -->\n";
        // (frontmatter, table directive, expected recognizable)
        let cases = [
            ("", "", true),
            ("sampling: off\n", "", false),
            ("sampling: on\n", "<!-- sampling: off -->\n", false),
            ("sampling: off\n", "<!-- sampling: on -->\n", true),
        ];
        for (deck_key, table_directive, expected) in cases {
            let text = format!(
                "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\ntable: cards\n{deck_key}---\n| w | m |\n|---|---|\n{rows}{table_directive}<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n"
            );
            let path = dir.path().join("t.md");
            std::fs::write(&path, &text).unwrap();
            let cards = crate::deck::Deck::load(&path).unwrap().cards;
            assert_eq!(
                expected,
                cards
                    .iter()
                    .all(|card| card_recognizable(card, &cache, &cards)),
                "deck {deck_key:?} table {table_directive:?}"
            );
        }
    }

    #[test]
    fn default_depth_stays_recall_for_an_empty_deck() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("deck1.json"));
        assert_eq!(Depth::Recall, default_depth(&[], &cache));
    }

    #[test]
    fn recognize_depth_always_renders_a_choice_check() {
        for reveal in [Reveal::Flip, Reveal::Line] {
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
            Mode::TypeLine,
            check_for(Reveal::Line, Depth::Reconstruct, &card("a\n    b"))
        );
    }

    /// A quotation is not part of the typed target, so it can never be what a
    /// Reconstruct check asks the learner to produce.
    #[test]
    fn reconstruct_depth_counts_only_the_answer_the_learner_must_produce() {
        let rows: Vec<(Reveal, &str, Mode, &str)> = vec![
            (
                Reveal::Flip,
                "> the whole answer is a quotation",
                Mode::Explain,
                "nothing gradeable is left, so no typing mode",
            ),
            (
                Reveal::Line,
                "> the whole answer is a quotation",
                Mode::Explain,
                "the same holds line by line",
            ),
            (
                Reveal::Flip,
                "> a quoted passage\nthe answer's own prose",
                Mode::Typing,
                "one gradeable line is an atom, whatever stands beside it",
            ),
            (
                Reveal::Line,
                "> a quoted passage\npoint a\npoint b",
                Mode::TypeLine,
                "two gradeable lines still type line by line",
            ),
            (
                Reveal::Line,
                "| a | b |\n| --- | --- |\n| 1 | 2 |",
                Mode::Explain,
                "a table is never typed, so a table-only answer has nothing to type",
            ),
            (
                Reveal::Line,
                "the answer's own prose\n| a | b |\n| --- | --- |\n| 1 | 2 |",
                Mode::TypeLine,
                "the prose beside a table is still the learner's own claim",
            ),
        ];
        for (reveal, back, expected, why) in rows {
            assert_eq!(
                expected,
                check_for(reveal, Depth::Reconstruct, &card(back)),
                "{why}"
            );
        }
    }

    /// A reshape replaces the authored answer on every surface a learner
    /// sees, so the check has to be chosen for what is shown.
    #[test]
    fn reconstruct_depth_counts_the_authored_answer_whatever_the_reshape_displays() {
        let mut collapsed = card("first fact\nsecond fact");
        collapsed.display_back = Some(vec!["one reshaped line".into()]);
        assert_eq!(
            Mode::Explain,
            check_for(Reveal::Flip, Depth::Reconstruct, &collapsed),
            "two authored lines are key points, however few the reshape shows"
        );

        let mut expanded = card("A, B, C");
        expanded.display_back = Some(vec!["A".into(), "B".into(), "C".into()]);
        assert_eq!(
            Mode::Typing,
            check_for(Reveal::Flip, Depth::Reconstruct, &expanded),
            "and one authored line is an atom, however many the reshape shows"
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

    #[test]
    fn a_multiple_card_is_recognizable_only_with_authored_distractors() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = crate::augment::AugmentCache::open(dir.path().join("a.json"));
        let mut multi = card("a\nb");
        multi.multiple_choice = true;
        cache.set_distractors(
            &multi.id().unwrap(),
            vec!["w1".into(), "w2".into(), "w3".into()],
            multi.content_fingerprint,
        );
        let deck = vec![multi.clone(), card("other")];
        assert!(
            !card_recognizable(&multi, &cache, &deck),
            "cached AI distractors must not admit a select-all card"
        );
        multi.authored_distractors = vec!["x".into()];
        assert!(card_recognizable(&multi, &cache, &deck));
    }
}

#[cfg(all(test, feature = "full"))]
mod clap_parity {
    use clap::ValueEnum;

    use super::*;

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
