use super::{WHITESPACE, trim_ws};

/// A run of `>` lines in a personal file, addressed to a card by the
/// `<!-- note: -->` marker that opens it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidecarNote {
    pub card: String,
    pub lines: Vec<String>,
}

pub fn notes(text: &str) -> Vec<SidecarNote> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(card) = marker_target(trim_ws(line)) else {
            continue;
        };
        out.push(SidecarNote {
            card: card.to_string(),
            lines: lines[index + 1..quoted_run_end(&lines, index)]
                .iter()
                .map(|line| quoted_text(line).unwrap_or_default().to_string())
                .collect(),
        });
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
        dropped[index] = true;
        dropped[index + 1..quoted_run_end(&lines, index)].fill(true);
        // A label the reader wrote above their own note; alix never writes one.
        if index > 0 && trim_ws(lines[index - 1]).starts_with("## ") {
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

    #[test]
    fn a_marker_carries_the_quoted_lines_below_it() {
        let text = "<!-- note: card-abc -->\n> not the same as realizar\n> really\n";
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                lines: vec!["not the same as realizar".into(), "really".into()],
            }],
            notes(text)
        );
    }

    #[test]
    fn a_blank_line_ends_the_run_so_later_quotes_are_not_swept_in() {
        let text = "<!-- note: card-abc -->\n> mine\n\n> a later quote\n";
        assert_eq!(vec!["mine".to_string()], notes(text)[0].lines);
    }

    #[test]
    fn blocks_are_returned_in_file_order_across_intervening_content() {
        let text = "<!-- note: card-one -->\n> first\n\n\
                    ## a personal card <!-- id: card-xyz -->\nan answer\n\n\
                    <!-- note: card-two -->\n> second\n";
        assert_eq!(
            vec!["card-one".to_string(), "card-two".to_string()],
            notes(text)
                .iter()
                .map(|n| n.card.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_marker_with_nothing_below_it_is_still_a_block_so_doctor_can_report_it() {
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                lines: Vec::new(),
            }],
            notes("<!-- note: card-abc -->\n")
        );
    }

    #[test]
    fn a_marker_carrying_anything_beyond_the_id_is_not_a_marker() {
        assert_eq!(
            Vec::<SidecarNote>::new(),
            notes("<!-- note: card-abc (a hint) -->\n> mine\n"),
            "the retired hinted form is not recognized, it is invalid input"
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
