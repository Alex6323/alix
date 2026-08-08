use super::{WHITESPACE, trim_ws};

/// A `>` note in a sidecar, addressed to a card by id. The hint is display
/// only: alix rewrites it when the card's front changes, and never reads it
/// back as identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidecarNote {
    pub card: String,
    pub hint: Option<String>,
    pub lines: Vec<String>,
}

pub fn notes(text: &str) -> Vec<SidecarNote> {
    let mut out: Vec<SidecarNote> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let line = trim_ws(line);
        if let Some(value) = marker_value(line) {
            let (card, hint) = split_hint(value);
            out.push(SidecarNote {
                card,
                hint,
                lines: Vec::new(),
            });
            open = true;
        } else if let Some(rest) = open.then(|| line.strip_prefix('>')).flatten() {
            if let Some(note) = out.last_mut() {
                note.lines.push(trim_ws(rest).to_string());
            }
        } else {
            open = false;
        }
    }
    out
}

/// The same text with every note block removed, so a card block cannot claim
/// a following note's `>` lines as its own answer or note.
pub fn without_notes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut open = false;
    for line in text.lines() {
        let trimmed = trim_ws(line);
        if marker_value(trimmed).is_some() {
            open = true;
            continue;
        }
        if open && trimmed.starts_with('>') {
            continue;
        }
        open = false;
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn marker_value(line: &str) -> Option<&str> {
    let body = line.strip_prefix("<!--")?.strip_suffix("-->")?;
    Some(trim_ws(trim_ws(body).strip_prefix("for:")?))
}

fn split_hint(value: &str) -> (String, Option<String>) {
    let (card, rest) = match value.find(WHITESPACE) {
        Some(end) => (&value[..end], trim_ws(&value[end..])),
        None => (value, ""),
    };
    let hint = rest
        .strip_prefix('(')
        .and_then(|inner| inner.strip_suffix(')'))
        .map(|hint| trim_ws(hint).to_string())
        .filter(|hint| !hint.is_empty());
    (card.to_string(), hint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_carries_its_following_quote_lines() {
        let text = "<!-- for: card-abc (darse cuenta) -->\n> not the same as realizar\n> really\n";
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                hint: Some("darse cuenta".into()),
                lines: vec!["not the same as realizar".into(), "really".into()],
            }],
            notes(text)
        );
    }

    #[test]
    fn a_hint_is_optional_and_the_id_is_still_the_first_token() {
        let text = "<!-- for: card-abc -->\n> plain\n";
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                hint: None,
                lines: vec!["plain".into()],
            }],
            notes(text)
        );
    }

    #[test]
    fn a_blank_line_closes_a_block_so_later_quotes_are_not_swept_in() {
        let text = "<!-- for: card-abc -->\n> mine\n\n> a stray quote\n";
        assert_eq!(vec!["mine".to_string()], notes(text)[0].lines);
        assert_eq!(1, notes(text).len());
    }

    #[test]
    fn blocks_are_returned_in_file_order_across_intervening_content() {
        let text = "<!-- for: card-one -->\n> first\n\n\
                    ## a personal card <!-- id: card-xyz -->\nan answer\n\n\
                    <!-- for: card-two (a hint) -->\n> second\n";
        let found = notes(text);
        assert_eq!(
            vec!["card-one".to_string(), "card-two".to_string()],
            found.iter().map(|n| n.card.clone()).collect::<Vec<_>>()
        );
        assert_eq!(Some("a hint".to_string()), found[1].hint);
    }

    #[test]
    fn a_marker_without_notes_is_still_a_block_so_doctor_can_report_it() {
        assert_eq!(
            vec![SidecarNote {
                card: "card-abc".into(),
                hint: None,
                lines: Vec::new(),
            }],
            notes("<!-- for: card-abc -->\n")
        );
    }

    #[test]
    fn without_notes_drops_a_note_block_and_keeps_a_cards_own_note() {
        let text = "## a personal card <!-- id: card-xyz -->\nan answer\n> its own note\n\n\
                    <!-- for: card-abc -->\n> addressed elsewhere\n";
        assert_eq!(
            "## a personal card <!-- id: card-xyz -->\nan answer\n> its own note\n\n",
            without_notes(text)
        );
    }

    #[test]
    fn without_notes_leaves_a_quote_that_follows_a_closed_block_alone() {
        let text = "<!-- for: card-abc -->\n> mine\n\n> a stray quote\n";
        assert_eq!("\n> a stray quote\n", without_notes(text));
    }

    #[test]
    fn text_carrying_no_marker_yields_nothing() {
        assert_eq!(Vec::<SidecarNote>::new(), notes("## q\na\n> a deck note\n"));
    }
}
