use super::{WHITESPACE, trim_ws};

/// A run of `>` lines in a personal file, addressed to a card by the
/// `<!-- for: -->` marker that closes it.
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
            lines: lines[quoted_run_start(&lines, index)..index]
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
        let start = quoted_run_start(&lines, index);
        dropped[start..index].fill(true);
        // A label the reader wrote above their own note; alix never writes one.
        if start > 0 && trim_ws(lines[start - 1]).starts_with("## ") {
            dropped[start - 1] = true;
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

fn quoted_run_start(lines: &[&str], marker: usize) -> usize {
    let mut start = marker;
    while start > 0 && quoted_text(lines[start - 1]).is_some() {
        start -= 1;
    }
    start
}

fn quoted_text(line: &str) -> Option<&str> {
    Some(trim_ws(trim_ws(line).strip_prefix('>')?))
}

/// The grammar is exactly `<!-- for: <card-id> -->`; anything else is not a
/// marker, so its `>` lines stay in the text and fail as ordinary card content.
fn marker_target(line: &str) -> Option<&str> {
    let body = line.strip_prefix("<!--")?.strip_suffix("-->")?;
    let value = trim_ws(trim_ws(body).strip_prefix("for:")?);
    (!value.is_empty() && !value.contains(WHITESPACE)).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_carries_the_quoted_lines_above_it() {
        let text = "> not the same as realizar\n> really\n<!-- for: card-abc -->\n";
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                lines: vec!["not the same as realizar".into(), "really".into()],
            }],
            notes(text)
        );
    }

    #[test]
    fn a_blank_line_ends_the_run_so_earlier_quotes_are_not_swept_in() {
        let text = "> an earlier quote\n\n> mine\n<!-- for: card-abc -->\n";
        assert_eq!(vec!["mine".to_string()], notes(text)[0].lines);
    }

    #[test]
    fn blocks_are_returned_in_file_order_across_intervening_content() {
        let text = "> first\n<!-- for: card-one -->\n\n\
                    ## a personal card <!-- id: card-xyz -->\nan answer\n\n\
                    > second\n<!-- for: card-two -->\n";
        assert_eq!(
            vec!["card-one".to_string(), "card-two".to_string()],
            notes(text)
                .iter()
                .map(|n| n.card.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_marker_with_nothing_above_it_is_still_a_block_so_doctor_can_report_it() {
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                lines: Vec::new(),
            }],
            notes("<!-- for: card-abc -->\n")
        );
    }

    #[test]
    fn a_marker_carrying_anything_beyond_the_id_is_not_a_marker() {
        assert_eq!(
            Vec::<SidecarNote>::new(),
            notes("> mine\n<!-- for: card-abc (a hint) -->\n"),
            "the retired hinted form is not recognized, it is invalid input"
        );
    }

    #[test]
    fn without_notes_drops_a_note_block_and_keeps_a_cards_own_note() {
        let text = "## a personal card <!-- id: card-xyz -->\nan answer\n> its own note\n\n\
                    > addressed elsewhere\n<!-- for: card-abc -->\n";
        assert_eq!(
            "## a personal card <!-- id: card-xyz -->\nan answer\n> its own note\n\n",
            without_notes(text)
        );
    }

    #[test]
    fn without_notes_absorbs_a_label_the_reader_wrote_above_their_note() {
        let text = "## why the None case matters\n> mine\n<!-- for: card-abc -->\n";
        assert_eq!("", without_notes(text));
    }

    #[test]
    fn without_notes_leaves_a_card_whose_answer_precedes_a_stray_marker() {
        let text = "## front\nan answer\n<!-- for: card-abc -->\n";
        assert_eq!(
            "## front\nan answer\n",
            without_notes(text),
            "the answer is a card's, not a note's; doctor reports the ambiguity"
        );
    }

    #[test]
    fn text_carrying_no_marker_yields_nothing() {
        assert_eq!(Vec::<SidecarNote>::new(), notes("## q\na\n> a deck note\n"));
    }
}
