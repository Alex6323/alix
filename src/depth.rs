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

fn answer_is_atomic(card: &Card) -> bool {
    card.back.len() == 1
}

pub fn check_for(reveal: Reveal, depth: Depth, card: &Card) -> Mode {
    match depth {
        Depth::Recognize => Mode::Choice,
        Depth::Recall => match reveal {
            Reveal::Flip => Mode::Flip,
            Reveal::Line => Mode::LineByLine,
        },
        Depth::Reconstruct => match reveal {
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

pub fn card_recognizable(card: &Card, cache: &AugmentCache, deck_cards: &[Card]) -> bool {
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
        let text = format!("## q <!-- id: card-q{slug}x -->\n{back}\n");
        parser::parse_str("t.md", &text).unwrap().remove(0)
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
        let text = "| w | m |\n|---|---|\n| a | alpha | <!-- r:aaaaaa -->\n| b | beta | <!-- r:bbbbbb -->\n| c | gamma | <!-- r:cccccc -->\n| d | delta | <!-- r:dddddd -->\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n";
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
        let text = "| w | m |\n|---|---|\n| a | alpha | <!-- r:aaaaaa -->\n| b | beta | <!-- r:bbbbbb -->\n| c | gamma | <!-- r:cccccc -->\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n";
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
                "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n{deck_key}---\n| w | m |\n|---|---|\n{rows}{table_directive}<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n"
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
