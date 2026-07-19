use thiserror::Error;

const MARKUP_FRONT: char = '#';
const MARKUP_COMMENT: char = '%';
const MARKUP_NOTE: char = '!';
const MARKUP_ESCAPE: char = '\\';
const MARKUP: [char; 3] = [MARKUP_FRONT, MARKUP_COMMENT, MARKUP_NOTE];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TxtError {
    #[error("line {0}: answer or note line appears before any '#' card front")]
    BackBeforeFront(usize),
}

#[derive(Debug, PartialEq, Eq)]
pub struct TxtDeck {
    pub title: Option<String>,
    pub header: Vec<(String, String)>,
    pub cards: Vec<TxtCard>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TxtCard {
    pub front: String,
    pub back: Vec<String>,
    pub note: Option<String>,
    pub directives: Vec<(String, String)>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxtSegment {
    Text(String),
    Hole(String),
}

pub fn cloze_segments(line: &str) -> Vec<TxtSegment> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut hole: Option<String> = None;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' if matches!(chars.peek(), Some('{' | '}' | '\\')) => {
                if let Some(escaped) = chars.next() {
                    match &mut hole {
                        Some(h) => h.push(escaped),
                        None => text.push(escaped),
                    }
                }
            }
            '{' if hole.is_none() && chars.peek() == Some(&'{') => {
                chars.next();
                hole = Some(String::new());
            }
            '}' if hole.is_some() && chars.peek() == Some(&'}') => {
                chars.next();
                if !text.is_empty() {
                    segments.push(TxtSegment::Text(std::mem::take(&mut text)));
                }
                segments.push(TxtSegment::Hole(hole.take().unwrap_or_default()));
            }
            c => match &mut hole {
                Some(h) => h.push(c),
                None => text.push(c),
            },
        }
    }

    if let Some(h) = hole {
        text.push_str("{{");
        text.push_str(&h);
    }
    if !text.is_empty() {
        segments.push(TxtSegment::Text(text));
    }
    segments
}

const DECK_LEVEL_KEYS: [&str; 5] = ["link", "requires", "source", "title", "trace"];

fn directive(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(MARKUP_COMMENT)?;
    let (key, value) = rest.split_once(':')?;
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    let value = value.trim();
    (!value.is_empty()).then(|| (key, value.to_string()))
}

pub fn parse_txt(text: &str) -> Result<TxtDeck, TxtError> {
    let mut title = None;
    let mut header = Vec::new();
    let mut trace_seen = false;
    let mut cards: Vec<TxtCard> = Vec::new();
    let mut current: Option<TxtCard> = None;

    for (lineno, raw) in text.lines().enumerate() {
        let lineno = lineno + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        // The line is non-empty (checked above), so a first character exists.
        let first = line.chars().next().expect("line is non-empty");
        // An indented `#` is answer content (e.g. a shell comment), not a front.
        let is_front = first == MARKUP_FRONT && raw.starts_with(MARKUP_FRONT);

        match first {
            MARKUP_FRONT if is_front => {
                if let Some(card) = current.take() {
                    cards.push(card);
                }
                let front = line[MARKUP_FRONT.len_utf8()..].trim().to_string();
                current = Some(TxtCard {
                    front,
                    back: Vec::new(),
                    note: None,
                    directives: Vec::new(),
                    line: lineno,
                });
            }
            MARKUP_COMMENT => {
                let Some((key, value)) = directive(line) else {
                    continue;
                };
                if key == "title" {
                    title = title.or(Some(value));
                } else if key == "trace" {
                    if !trace_seen {
                        trace_seen = true;
                        header.push((key, value));
                    }
                } else if DECK_LEVEL_KEYS.contains(&key.as_str()) {
                    header.push((key, value));
                } else {
                    match &mut current {
                        Some(card) => card.directives.push((key, value)),
                        None => header.push((key, value)),
                    }
                }
            }
            MARKUP_NOTE => {
                let card = current.as_mut().ok_or(TxtError::BackBeforeFront(lineno))?;
                // Strip one leading space only, so further indentation (code
                // inside a fence) survives.
                let after = &line[MARKUP_NOTE.len_utf8()..];
                let text = after.strip_prefix(' ').unwrap_or(after).trim_end();
                match &mut card.note {
                    Some(note) => {
                        note.push('\n');
                        note.push_str(text);
                    }
                    None => card.note = Some(text.to_string()),
                }
            }
            _ => {
                let card = current.as_mut().ok_or(TxtError::BackBeforeFront(lineno))?;
                let second = line.chars().nth(1);
                let back_line =
                    if first == MARKUP_ESCAPE && second.is_some_and(|c| MARKUP.contains(&c)) {
                        &line[MARKUP_ESCAPE.len_utf8()..]
                    } else {
                        line
                    };
                card.back.push(back_line.to_string());
            }
        }
    }

    if let Some(card) = current.take() {
        cards.push(card);
    }

    Ok(TxtDeck {
        title,
        header,
        cards,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_line_opens_a_card_and_indented_lines_are_its_answer() {
        let deck = parse_txt("# front\n\tback one\n\tback two\n").unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!("front", deck.cards[0].front);
        assert_eq!(vec!["back one", "back two"], deck.cards[0].back);
        assert_eq!(1, deck.cards[0].line);
    }

    #[test]
    fn escaped_hash_percent_and_bang_lose_their_backslash() {
        let deck = parse_txt("# front\n\t\\#x\n\t\\%y\n\t\\!z\n").unwrap();
        assert_eq!(vec!["#x", "%y", "!z"], deck.cards[0].back);
    }

    #[test]
    fn a_lone_backslash_stays_verbatim() {
        let deck = parse_txt("# front\n\t\\n is a newline\n").unwrap();
        assert_eq!(vec!["\\n is a newline"], deck.cards[0].back);
    }

    #[test]
    fn bang_lines_concatenate_into_the_note() {
        let deck = parse_txt("# front\n\tback\n! one\n! two\n! three\n").unwrap();
        assert_eq!(Some("one\ntwo\nthree".to_string()), deck.cards[0].note);
    }

    #[test]
    fn header_directives_are_collected_in_order_and_title_is_split_out() {
        let text = "% reveal: cloze\n\
                    % link: https://example.org\n\
                    % title: My Deck\n\
                    % requires: other.txt\n\
                    # front\n\tback\n";
        let deck = parse_txt(text).unwrap();
        assert_eq!(Some("My Deck".to_string()), deck.title);
        assert_eq!(
            vec![
                ("reveal".to_string(), "cloze".to_string()),
                ("link".to_string(), "https://example.org".to_string()),
                ("requires".to_string(), "other.txt".to_string()),
            ],
            deck.header
        );
    }

    #[test]
    fn per_card_directives_stay_with_their_card() {
        let text = "# a\n% reveal: line\n\tx\n# b\n% input: draw\n\ty\n";
        let deck = parse_txt(text).unwrap();
        assert_eq!(
            vec![("reveal".to_string(), "line".to_string())],
            deck.cards[0].directives
        );
        assert_eq!(
            vec![("input".to_string(), "draw".to_string())],
            deck.cards[1].directives
        );
    }

    #[test]
    fn old_cloze_closes_at_the_first_double_brace() {
        let segments = cloze_segments("a {{b {c}} d");
        assert_eq!(
            vec![
                TxtSegment::Text("a ".to_string()),
                TxtSegment::Hole("b {c".to_string()),
                TxtSegment::Text(" d".to_string()),
            ],
            segments
        );
    }

    #[test]
    fn indented_hash_is_answer_content_not_a_front() {
        let text = "# Dockerfile\n\tFROM rust\n\t# a comment\n\tRUN cargo build\n";
        let deck = parse_txt(text).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!(
            vec!["FROM rust", "# a comment", "RUN cargo build"],
            deck.cards[0].back
        );
    }

    #[test]
    fn a_line_before_any_front_errors() {
        assert_eq!(Err(TxtError::BackBeforeFront(1)), parse_txt("back\n"));
        assert_eq!(Err(TxtError::BackBeforeFront(1)), parse_txt("! note\n"));
        assert_eq!(
            Err(TxtError::BackBeforeFront(1)),
            parse_txt("\t# comment\n")
        );
    }

    #[test]
    fn an_escaped_brace_never_opens_a_hole() {
        let segments = cloze_segments("\\{{a}}");
        assert_eq!(vec![TxtSegment::Text("{{a}}".to_string())], segments);
    }

    #[test]
    fn escaped_braces_unescape_to_literal_text() {
        let segments = cloze_segments("a \\{\\{ b");
        assert_eq!(vec![TxtSegment::Text("a {{ b".to_string())], segments);
    }

    #[test]
    fn an_escaped_brace_inside_a_hole_stays_literal_hole_content() {
        let segments = cloze_segments("{{a \\{ b}}");
        assert_eq!(vec![TxtSegment::Hole("a { b".to_string())], segments);
    }

    #[test]
    fn an_escaped_close_brace_does_not_close_a_hole() {
        let segments = cloze_segments("{{a\\}}b}}");
        assert_eq!(vec![TxtSegment::Hole("a}}b".to_string())], segments);
    }

    #[test]
    fn empty_text_yields_an_empty_deck() {
        let deck = parse_txt("").unwrap();
        assert_eq!(None, deck.title);
        assert!(deck.header.is_empty());
        assert!(deck.cards.is_empty());
    }

    #[test]
    fn deck_level_keys_after_a_card_stay_deck_level() {
        let text = "# front\n\tback\n% source: rustbook.md\n% link: https://example.org\n";
        let deck = parse_txt(text).unwrap();
        assert!(deck.cards[0].directives.is_empty());
        assert_eq!(
            vec![
                ("source".to_string(), "rustbook.md".to_string()),
                ("link".to_string(), "https://example.org".to_string()),
            ],
            deck.header
        );
    }

    #[test]
    fn a_second_title_mirrors_the_old_parsers_pick() {
        let text = "% title: First\n# front\n\tback\n% title: Second\n";
        let deck = parse_txt(text).unwrap();
        assert_eq!(Some("First".to_string()), deck.title);
        assert!(deck.cards[0].directives.is_empty());
        assert!(deck.header.iter().all(|(k, _)| k != "title"));
    }

    #[test]
    fn a_second_trace_line_is_dropped_like_a_second_title() {
        let text = "% trace: First\n# front\n\tback\n% trace: Second\n";
        let deck = parse_txt(text).unwrap();
        assert_eq!(
            vec![("trace".to_string(), "First".to_string())],
            deck.header
        );
        assert!(deck.cards[0].directives.is_empty());
    }
}
