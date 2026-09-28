use super::{WHITESPACE, trim_ws};
use crate::card::{Badge, Note};

/// A note in a personal file, addressed to a card by the `<!-- note: -->`
/// marker that opens it: the deck's own note grammar, a `>` run whose first
/// line is a badge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidecarNote {
    pub card: String,
    pub note: Note,
}

/// A marker whose run opens with no badge line, by marker line number
/// (1-based) and card; it is no note, and doctor names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnbadgedNote {
    pub line: usize,
    pub card: String,
}

pub fn notes(text: &str) -> Vec<SidecarNote> {
    runs(text)
        .into_iter()
        .filter_map(|(_, card, run)| {
            let (first, rest) = run.split_first()?;
            let badge = Badge::parse(first)?;
            Some(SidecarNote {
                card,
                note: Note {
                    badge,
                    body: rest.join("\n"),
                },
            })
        })
        .collect()
}

pub fn unbadged_notes(text: &str) -> Vec<UnbadgedNote> {
    runs(text)
        .into_iter()
        .filter(|(_, _, run)| {
            run.first()
                .is_none_or(|first| Badge::parse(first).is_none())
        })
        .map(|(index, card, _)| UnbadgedNote {
            line: index + 1,
            card,
        })
        .collect()
}

/// Every marker with the quoted text of the run under it, by marker index.
fn runs(text: &str) -> Vec<(usize, String, Vec<String>)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(card) = marker_target(trim_ws(line)) else {
            continue;
        };
        let run = lines[index + 1..quoted_run_end(&lines, index)]
            .iter()
            .map(|line| quoted_text(line).unwrap_or_default().to_string())
            .collect();
        out.push((index, card.to_string(), run));
    }
    out
}

/// The same text with every note block removed, so a card block cannot claim
/// a note's `>` lines as its own answer.
pub fn without_notes(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut dropped = vec![false; lines.len()];
    for (index, line) in lines.iter().enumerate() {
        if marker_target(trim_ws(line)).is_none() {
            continue;
        }
        dropped[index..quoted_run_end(&lines, index)].fill(true);
        // A label the reader wrote above their own note; alix never writes one.
        if index > 0
            && super::heading_depth(trim_ws(lines[index - 1]))
                .is_some_and(|(depth, _)| super::is_card_depth(depth))
        {
            dropped[index - 1] = true;
        }
    }
    let mut out = String::with_capacity(text.len());
    for (line, dropped) in lines.iter().zip(dropped) {
        if !dropped {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn quoted_run_end(lines: &[&str], marker: usize) -> usize {
    let start = marker + 1;
    start
        + lines[start..]
            .iter()
            .take_while(|line| quoted_text(line).is_some())
            .count()
}

fn quoted_text(line: &str) -> Option<&str> {
    Some(trim_ws(trim_ws(line).strip_prefix('>')?))
}

/// The grammar is exactly `<!-- note: <card-id> -->`; anything else is not a
/// marker, so its `>` lines stay in the text as ordinary card content.
fn marker_target(line: &str) -> Option<&str> {
    let body = line.strip_prefix("<!--")?.strip_suffix("-->")?;
    let value = trim_ws(trim_ws(body).strip_prefix("note:")?);
    (!value.is_empty() && !value.contains(WHITESPACE)).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader labels a note with whatever heading depth the source card
    /// used. Every depth is absorbed, so the label never reaches the deck
    /// parser, where it would orphan and silently empty the personal file.
    /// Done-list law, exact ownership: a stray deeper heading inside a
    /// personal card stays THAT card's content, rather than being dropped
    /// or opening a card of its own.
    #[test]
    fn a_stray_deep_heading_stays_the_personal_cards_own_content() {
        let text = "## personal\nanswer\n### stray label\ntail\n<!-- id: card-p1 -->\n";
        let cards = crate::parser::parse_sidecar("deck.local.md", text).unwrap();
        assert_eq!(1, cards.len());
        assert_eq!(vec!["answer", "### stray label", "tail"], cards[0].back);
    }

    #[test]
    fn a_note_label_is_absorbed_at_every_card_depth() {
        for label in ["## why", "### why", "#### why", "##### why", "###### why"] {
            let text = format!("{label}\n<!-- note: card-q1 -->\n> because\n");
            let stripped = without_notes(&text);
            assert!(
                !stripped.contains("why"),
                "label {label:?} survived into the deck text: {stripped:?}"
            );
        }
    }

    #[test]
    fn a_depth_six_note_label_never_leaks_into_a_personal_cards_answer() {
        let text = "## personal\nanswer\n<!-- id: card-p1 -->\n\n\
                    ###### why\n<!-- note: card-q1 -->\n> because\n";
        let stripped = without_notes(text);
        let cards = crate::parser::parse_sidecar("deck.local.md", &stripped)
            .expect("a legal note label must not make the personal file unparseable");
        assert_eq!(1, cards.len());
        assert_eq!(Some("card-p1".to_string()), cards[0].id());
        assert_eq!(
            vec!["answer"],
            cards[0].back,
            "the label belongs to the following note, not the preceding personal card"
        );
    }

    #[test]
    fn a_marker_carries_the_badged_note_below_it() {
        let text = "<!-- note: card-abc -->\n> [!NOTE]\n> not the same as realizar\n> really\n";
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                note: Note {
                    badge: Badge::Note,
                    body: "not the same as realizar\nreally".into(),
                },
            }],
            notes(text)
        );
        assert!(unbadged_notes(text).is_empty());
    }

    #[test]
    fn a_marker_trims_spaces_around_its_card_id() {
        let text = "<!-- note:  card-abc  -->\n> [!NOTE]\n> spaced marker\n";
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                note: Note {
                    badge: Badge::Note,
                    body: "spaced marker".into(),
                },
            }],
            notes(text)
        );
    }

    #[test]
    fn every_badge_opens_a_sidecar_note_and_nothing_else_does() {
        for (line, badge) in [
            ("[!NOTE]", Badge::Note),
            ("[!TIP]", Badge::Tip),
            ("[!IMPORTANT]", Badge::Important),
            ("[!WARNING]", Badge::Warning),
            ("[!CAUTION]", Badge::Caution),
        ] {
            let text = format!("<!-- note: card-abc -->\n> {line}\n> body\n");
            assert_eq!(badge, notes(&text)[0].note.badge, "{line}");
        }
        for run in ["> body\n", "> [!note]\n> body\n", "> [!HINT]\n> body\n", ""] {
            let text = format!("<!-- note: card-abc -->\n{run}");
            assert!(notes(&text).is_empty(), "{run:?} is no note");
            assert_eq!(
                vec![UnbadgedNote {
                    line: 1,
                    card: "card-abc".into(),
                }],
                unbadged_notes(&text),
                "{run:?} is reported"
            );
        }
    }

    #[test]
    fn a_blank_line_ends_the_run_so_later_quotes_are_not_swept_in() {
        let text = "<!-- note: card-abc -->\n> [!NOTE]\n> mine\n\n> a later quote\n";
        assert_eq!("mine", notes(text)[0].note.body);
    }

    #[test]
    fn blocks_are_returned_in_file_order_across_intervening_content() {
        let text = "<!-- note: card-one -->\n> [!TIP]\n> first\n\n\
                    ## a personal card <!-- id: card-xyz -->\nan answer\n\n\
                    <!-- note: card-two -->\n> [!NOTE]\n> second\n";
        assert_eq!(
            vec!["card-one".to_string(), "card-two".to_string()],
            notes(text)
                .iter()
                .map(|n| n.card.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_marker_with_only_a_badge_below_it_is_an_empty_note() {
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                note: Note {
                    badge: Badge::Note,
                    body: String::new(),
                },
            }],
            notes("<!-- note: card-abc -->\n> [!NOTE]\n")
        );
    }

    #[test]
    fn a_marker_carrying_anything_beyond_the_id_is_not_a_marker() {
        assert_eq!(
            Vec::<SidecarNote>::new(),
            notes("<!-- note: card-abc (a hint) -->\n> mine\n"),
            "a marker with trailing content is not recognized, it is invalid input"
        );
    }

    #[test]
    fn a_marker_naming_a_kind_alix_does_not_write_yet_is_not_a_note() {
        assert_eq!(
            Vec::<SidecarNote>::new(),
            notes("<!-- hint: card-abc -->\n> mine\n"),
            "each kind is its own keyword, so an unbuilt one is simply not a marker"
        );
    }

    #[test]
    fn without_notes_drops_a_note_block_and_keeps_a_cards_own_note() {
        let text = "## a personal card <!-- id: card-xyz -->\nan answer\n> its own note\n\n\
                    <!-- note: card-abc -->\n> addressed elsewhere\n";
        assert_eq!(
            "## a personal card <!-- id: card-xyz -->\nan answer\n> its own note\n\n",
            without_notes(text)
        );
    }

    #[test]
    fn without_notes_absorbs_a_label_the_reader_wrote_above_their_note() {
        let text = "## why the None case matters\n<!-- note: card-abc -->\n> mine\n";
        assert_eq!("", without_notes(text));
    }

    #[test]
    fn without_notes_leaves_a_card_whose_front_follows_a_stray_marker() {
        let text = "<!-- note: card-abc -->\n## front\nan answer\n";
        assert_eq!(
            "## front\nan answer\n",
            without_notes(text),
            "the front is a card's, not a note's; doctor reports the ambiguity"
        );
    }

    #[test]
    fn text_carrying_no_marker_yields_nothing() {
        assert_eq!(Vec::<SidecarNote>::new(), notes("## q\na\n> a deck note\n"));
    }
}
