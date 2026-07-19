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
/// FIRST `}}` after it closes it, brace-agnostic (nested `{`/`}` inside a
/// hole, even a doubled one, are just hole content. This reader never
/// rejects a deck, unlike the strict cloze parser). A backslash before `{`,
/// `}`, or `\` escapes it: the backslash is dropped, the escaped character is
/// kept as literal content (in whichever buffer is open, text or hole) and
/// never counts toward marker detection, mirroring `crate::cloze::parse_line`'s
/// escape handling (`src/cloze.rs:48-53`). An unclosed `{{` (no matching `}}`
/// before the end of the line) is not a hole: its marker and everything
/// captured since are folded back into the literal text that precedes it, as
/// one segment.
pub fn cloze_segments(line: &str) -> Vec<TxtSegment> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut hole: Option<String> = None;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // A backslash escapes the very next character only when that
            // character is itself markup (`{`, `}`, `\`); the pair is
            // consumed together, so the escaped char never triggers the
            // open/close checks below.
            '\\' if matches!(chars.peek(), Some('{' | '}' | '\\')) => {
                if let Some(escaped) = chars.next() {
                    match &mut hole {
                        Some(h) => h.push(escaped),
                        None => text.push(escaped),
                    }
                }
            }
            // `{{` opens a hole, but only outside an already-open one: a
            // doubled brace found while already inside a hole is just hole
            // content (brace-agnostic).
            '{' if hole.is_none() && chars.peek() == Some(&'{') => {
                chars.next();
                hole = Some(String::new());
            }
            // `}}` closes the open hole at the FIRST occurrence, regardless
            // of what braces it contains.
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

    // An unclosed `{{` is not a hole: fold its marker and everything
    // captured since back into the pending literal text, as one segment.
    if let Some(h) = hole {
        text.push_str("{{");
        text.push_str(&h);
    }
    if !text.is_empty() {
        segments.push(TxtSegment::Text(text));
    }
    segments
}

/// Keys the old parser collects with dedicated WHOLE-FILE scans
/// (`parser::parse_links`, `parse_requires`, `parse_sources`, `parse_title`,
/// `parse_trace`, `src/parser.rs:160-218`), independent of the card-by-card
/// state machine: they're deck-level wherever they sit in the file, even
/// after a card front, and are never a per-card override. `title` is one of
/// them but is split out into `TxtDeck.title` rather than joining `header`.
/// `trace` is also first-line-wins (mirrors `parse_trace`'s `find_map`,
/// `src/parser.rs:212-218`), but unlike `title` it still joins `header`,
/// just once, on its first occurrence; a later `% trace:` line is dropped
/// outright.
const DECK_LEVEL_KEYS: [&str; 5] = ["link", "requires", "source", "title", "trace"];

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
                if key == "title" {
                    // Whole-file, first-line-wins, regardless of position
                    // (mirrors `parser::parse_title`'s `find_map` over every
                    // line): never joins `header` or a card's directives.
                    title = title.or(Some(value));
                } else if key == "trace" {
                    // Whole-file, first-line-wins, regardless of position
                    // (mirrors `parser::parse_trace`'s `find_map` over every
                    // line, `src/parser.rs:212-218`). Unlike `title` it still
                    // joins `header`, just once, on its first occurrence; a
                    // later `% trace:` line is dropped outright, never in
                    // `header`, never a card's directives.
                    if !trace_seen {
                        trace_seen = true;
                        header.push((key, value));
                    }
                } else if DECK_LEVEL_KEYS.contains(&key.as_str()) {
                    // Deck-level wherever it sits, even inside a card
                    // (mirrors the dedicated whole-file collectors), never a
                    // per-card override.
                    header.push((key, value));
                } else {
                    match &mut current {
                        // Inside a card, every other directive is a per-card
                        // override, recorded uninterpreted; a later task acts
                        // on it.
                        Some(card) => card.directives.push((key, value)),
                        // Before the first card, it joins the header, in
                        // order.
                        None => header.push((key, value)),
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
    fn an_escaped_brace_never_opens_a_hole() {
        // Empirically confirmed against `crate::cloze::parse_line("\\{{a}}",
        // 1)`: the escaped first `{` loses its backslash but stays a literal
        // char, so it can no longer pair with the second `{` to open a hole;
        // the whole line stays one literal Text segment, no Hole at all.
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
        // Mirrors `crate::cloze::parse_line`'s own escape handling, which
        // doesn't care whether a hole is open: verified via
        // `crate::cloze::parse_line("{{a \\{ b}}", 1)` ==
        // `[Segment::Hole("a { b")]` (the closest existing cloze.rs coverage
        // is `escaped_double_brace_is_literal`, which only exercises the
        // escape outside a hole; this reproduces the same escape arm with a
        // hole open).
        let segments = cloze_segments("{{a \\{ b}}");
        assert_eq!(vec![TxtSegment::Hole("a { b".to_string())], segments);
    }

    #[test]
    fn an_escaped_close_brace_does_not_close_a_hole() {
        // The backslash escapes only the first `}` right after it; that
        // escaped `}` is ordinary hole content and can't pair with the very
        // next (unescaped) `}` to close the hole, since the closing check
        // looks at the CURRENT char plus its peek, not backward. The hole
        // stays open until the real `}}` at the end. Mirrors
        // `crate::cloze::parse_line`'s shared escape arm (`src/cloze.rs:48-53`).
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
        // `parser::parse_title` is a whole-file `find_map`: the first `%
        // title:` line anywhere wins, position-independent. A second one
        // (even after a card) is dropped outright, not stored anywhere.
        let text = "% title: First\n# front\n\tback\n% title: Second\n";
        let deck = parse_txt(text).unwrap();
        assert_eq!(Some("First".to_string()), deck.title);
        assert!(deck.cards[0].directives.is_empty());
        assert!(deck.header.iter().all(|(k, _)| k != "title"));
    }

    #[test]
    fn a_second_trace_line_is_dropped_like_a_second_title() {
        // `parser::parse_trace` (`src/parser.rs:212-218`) is a whole-file
        // `find_map`, exactly like `parse_title`: the first `% trace:` line
        // anywhere wins, position-independent. A second one (even after a
        // card) is dropped outright, not stored anywhere.
        let text = "% trace: First\n# front\n\tback\n% trace: Second\n";
        let deck = parse_txt(text).unwrap();
        assert_eq!(
            vec![("trace".to_string(), "First".to_string())],
            deck.header
        );
        assert!(deck.cards[0].directives.is_empty());
    }
}
