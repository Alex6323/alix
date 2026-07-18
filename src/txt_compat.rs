//! Standalone reader for the pre-L1 `.txt` deck format. Conversion aid only:
//! deleted at the end of Arc A once the user's decks are converted.
//!
//! This module is deliberately decoupled from [`crate::card::Card`] and the
//! rest of the deck model: those types change shape as the L1 format lands,
//! while the throwaway converter (a later task) still needs to read decks
//! written in the old format exactly as the old parser (`crate::parser`) did.
//! It transcribes that parser's line loop rather than reusing it, so it never
//! has to track those changes.

use thiserror::Error;

/// Markup indicating the front side of a card.
const MARKUP_FRONT: char = '#';
/// Markup indicating a comment or `% key: value` directive line.
const MARKUP_COMMENT: char = '%';
/// Markup indicating a note.
const MARKUP_NOTE: char = '!';
/// Escape character in case markup and card data collide.
const MARKUP_ESCAPE: char = '\\';
/// All markup characters that can be escaped.
const MARKUP: [char; 3] = [MARKUP_FRONT, MARKUP_COMMENT, MARKUP_NOTE];

/// A parse error, pointing at the offending line of the deck file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TxtError {
    #[error("line {0}: answer or note line appears before any '#' card front")]
    BackBeforeFront(usize),
}

/// A whole `.txt`-era deck: its header metadata plus its cards, in file order.
#[derive(Debug, PartialEq, Eq)]
pub struct TxtDeck {
    /// The deck's display title (`% title:`), if it declares one.
    pub title: Option<String>,
    /// Every other `% key: value` line before the first card, in order.
    pub header: Vec<(String, String)>,
    /// The deck's cards, in file order.
    pub cards: Vec<TxtCard>,
}

/// One card from a `.txt`-era deck, before any `Card`/`Reveal` interpretation.
#[derive(Debug, PartialEq, Eq)]
pub struct TxtCard {
    /// The card's front line.
    pub front: String,
    /// The card's answer lines, in order, escapes already stripped.
    pub back: Vec<String>,
    /// The card's note, if it has one (consecutive `!` lines joined by `\n`).
    pub note: Option<String>,
    /// Per-card `% key: value` lines, in order, uninterpreted.
    pub directives: Vec<(String, String)>,
    /// The 1-based line number of the card's front line.
    pub line: usize,
}

/// One piece of a cloze-split answer line: literal text or a hole's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxtSegment {
    Text(String),
    Hole(String),
}

/// Splits an answer line on old-style cloze markers: `{{` opens a hole, the
/// FIRST `}}` after it closes it, brace-agnostic (nested `{`/`}` inside a hole
/// are just hole content). An unclosed `{{` is not a hole: the rest of the
/// line, including the unmatched marker, is kept as literal text.
pub fn cloze_segments(line: &str) -> Vec<TxtSegment> {
    let mut segments = Vec::new();
    let mut rest = line;

    while let Some(open) = rest.find("{{") {
        let before = &rest[..open];
        let after_open = &rest[open + 2..];
        match after_open.find("}}") {
            Some(close) => {
                if !before.is_empty() {
                    segments.push(TxtSegment::Text(before.to_string()));
                }
                segments.push(TxtSegment::Hole(after_open[..close].to_string()));
                rest = &after_open[close + 2..];
            }
            // No closing marker anywhere after this `{{`: stop looking and
            // fall through to keep the remainder (below) as literal text.
            None => break,
        }
    }
    if !rest.is_empty() {
        segments.push(TxtSegment::Text(rest.to_string()));
    }
    segments
}

/// Parses a `% key: value` line (already trimmed, still carrying its leading
/// `%`) into a lower-cased key and trimmed value. Returns `None` for lines
/// without a colon or with an empty key/value.
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

/// Parses a whole `.txt`-era deck. Errors if an answer or note line appears
/// before the deck's first `#` card front.
pub fn parse_txt(text: &str) -> Result<TxtDeck, TxtError> {
    let mut title = None;
    let mut header = Vec::new();
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
        // A front marker only counts at column 0 of the raw line; an indented
        // `#` is answer content (shell comments, Rust attributes...).
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
                match &mut current {
                    // Inside a card, every directive is a per-card override,
                    // recorded uninterpreted; a later task acts on it.
                    Some(card) => card.directives.push((key, value)),
                    // Before the first card, `% title:` is split out into its
                    // own field; every other directive joins the header, in
                    // order.
                    None => {
                        if key == "title" {
                            title = title.or(Some(value));
                        } else {
                            header.push((key, value));
                        }
                    }
                }
            }
            MARKUP_NOTE => {
                let card = current.as_mut().ok_or(TxtError::BackBeforeFront(lineno))?;
                // Strip the marker and one separating space, but keep any
                // further leading whitespace so indented note content (code
                // inside a fence) survives. Trailing space is dropped.
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
                // Strip a leading backslash only when it escapes a markup
                // character, so an ordinary backslash is left untouched.
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
    fn empty_text_yields_an_empty_deck() {
        let deck = parse_txt("").unwrap();
        assert_eq!(None, deck.title);
        assert!(deck.header.is_empty());
        assert!(deck.cards.is_empty());
    }
}
