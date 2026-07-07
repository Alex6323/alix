//! Parser for the plain-text deck format.
//!
//! The format, line by line (each line is trimmed first, empty lines are
//! skipped):
//!
//! - `# <text>`  at column 0 starts a new card; the text is the front side. An *indented* `#` is
//!   answer content (code comments, Rust attributes, markdown headers), not a card front.
//! - `% reveal: cloze` on a card turns it into a cloze card; `{{...}}` in its answer lines are
//!   holes (see the [`cloze`](crate::cloze) module).
//! - `% <text>`  is a comment and ignored (any indentation).
//! - `% link: <url>` is still a comment to the card parser, but the URL is collected as a
//!   deck-level reference link (see [`parse_links`]); the ask-Claude view offers these to Claude as
//!   background material.
//! - `! <text>`  is a note attached to the current card (any indentation, after its back). Several
//!   consecutive `!` lines form one multi-line note.
//! - any other line is a back line of the current card.
//! - a leading `\` escapes a markup character (`#`, `%`, `!`) so a back line can start with one;
//!   the backslash is stripped.
//!
//! A card consists of one front line, one or more back lines, and an
//! optional note after the back lines. Malformed files yield errors with
//! line numbers rather than panicking.

use std::{path::PathBuf, sync::Arc};

use clap::ValueEnum;
use thiserror::Error;

use crate::{
    answer::Input,
    card::{Card, Direction},
    cloze,
    level::Reveal,
};

/// Markup indicating the front side of a card.
const MARKUP_FRONT: char = '#';
/// Markup indicating a comment line.
const MARKUP_COMMENT: char = '%';
/// Markup indicating a note.
const MARKUP_NOTE: char = '!';
/// Escape character in case markup and card data collide.
const MARKUP_ESCAPE: char = '\\';
/// All markup characters that can be escaped.
const MARKUP: [char; 3] = [MARKUP_FRONT, MARKUP_COMMENT, MARKUP_NOTE];

/// A parse error, pointing at the offending line of the deck file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("line {0}: answer line appears before any '#' card front")]
    BackBeforeFront(usize),
    #[error("line {0}: note appears before any '#' card front")]
    NoteBeforeFront(usize),
    #[error("line {0}: note must come after the card's answer lines")]
    NoteBeforeBack(usize),
    #[error("line {0}: answer lines are not allowed after a note")]
    BackAfterNote(usize),
    #[error("line {0}: card front without an answer")]
    FrontWithoutBack(usize),
    #[error("line {0}: card front is empty")]
    EmptyFront(usize),
    #[error("line {0}: cloze card ('% reveal: cloze') has no {{{{...}}}} holes in its answer")]
    ClozeWithoutHoles(usize),
    #[error(
        "line {0}: cloze answer is one hole with no surrounding text — there's nothing to recall it from; use a plain '#' card"
    )]
    ClozeWithoutContext(usize),
    #[error("line {0}: empty cloze hole '{{{{}}}}'")]
    EmptyClozeHole(usize),
    #[error("line {0}: unclosed cloze hole (missing the closing '}}}}')")]
    UnclosedClozeHole(usize),
    #[error("line {0}: nested cloze hole")]
    NestedClozeHole(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing parsed yet for the current card.
    Init,
    /// A front line has been parsed.
    Front,
    /// At least one back line has been parsed.
    Back,
    /// At least one note line has been parsed; further `!` lines extend the
    /// note, but no more answer lines may follow.
    Note,
}

struct PartialCard {
    front: String,
    /// Answer lines with their 1-based line numbers in the deck file.
    back: Vec<(usize, String)>,
    note: Option<String>,
    line: usize,
    cloze: bool,
    /// Per-card `% reveal:` method, if the card declares one.
    reveal: Option<Reveal>,
    /// Per-card `% input:` override, if the card declares one.
    input: Option<Input>,
    /// Per-card `% direction:`, if the card declares one.
    direction: Option<Direction>,
    /// Per-card `% img:` (question side), raw value as written.
    image: Option<String>,
    /// Per-card `% img-back:` (answer side), raw value as written.
    image_back: Option<String>,
    /// Per-card `% at:` trace locator (a source position), raw value as
    /// written — the asset locator, with any ` from <origin>` suffix split off.
    at: Option<String>,
    /// The ` from <origin>` suffix of a frozen `% at:` (origin-relative
    /// `src/caching.rs:46-66`), if present.
    at_origin: Option<String>,
    /// Per-card `% origin:` override (the crate root the frozen source lives in).
    origin: Option<String>,
    /// Per-card `% given:` lines (repeatable): a trace checkpoint's named
    /// "givens", in order.
    givens: Vec<String>,
}

impl PartialCard {
    /// Turns the finished partial into one card (plain) or several (cloze).
    fn build(self, subject: &Arc<str>, cards: &mut Vec<Card>) -> Result<(), ParseError> {
        if self.cloze {
            // A cloze card's per-card reveal applies to each of its sub-cards.
            for mut card in cloze::expand(
                subject,
                &self.front,
                &self.back,
                self.note.as_deref(),
                self.line,
            )? {
                card.reveal = self.reveal;
                card.input = self.input;
                cards.push(card);
            }
        } else {
            let mut card = Card::plain(
                Arc::clone(subject),
                self.front,
                self.back.into_iter().map(|(_, text)| text).collect(),
                self.note,
                self.line,
            );
            card.reveal = self.reveal;
            card.input = self.input;
            card.direction = self.direction;
            card.image = self.image.map(PathBuf::from);
            card.image_back = self.image_back.map(PathBuf::from);
            card.at = self.at;
            card.at_origin = self.at_origin;
            card.origin = self.origin;
            card.givens = self.givens;
            cards.push(card);
        }
        Ok(())
    }
}

/// Collects deck-level reference links: comment lines of the form
/// `% link: <url>`. They are invisible to the card parser (every `%` line
/// is), so they do not affect card hashes.
pub fn parse_links(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|raw| {
            let line = raw.trim();
            let rest = line.strip_prefix(MARKUP_COMMENT)?;
            let url = rest.trim().strip_prefix("link:")?.trim();
            (!url.is_empty()).then(|| url.to_string())
        })
        .collect()
}

/// Collects a deck's prerequisite decks: comment lines `% requires: <deck>`
/// (repeatable). Like links, they are invisible to the card parser. The value
/// is a deck name or path, resolved by the caller.
pub fn parse_requires(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|raw| {
            let rest = raw.trim().strip_prefix(MARKUP_COMMENT)?;
            let dep = rest.trim().strip_prefix("requires:")?.trim();
            (!dep.is_empty()).then(|| dep.to_string())
        })
        .collect()
}

/// Collects a deck's exam sources: comment lines `% source: <url-or-path>`
/// (repeatable) — the ground truth the AI exam grades against. Like links and
/// requires, they are invisible to the card parser.
pub fn parse_sources(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|raw| {
            let rest = raw.trim().strip_prefix(MARKUP_COMMENT)?;
            let src = rest.trim().strip_prefix("source:")?.trim();
            (!src.is_empty()).then(|| src.to_string())
        })
        .collect()
}

/// The deck's display title (`% title: <text>`), if it declares one. A
/// display-only name independent of the file name; the first such line wins.
/// Invisible to the card parser, so it never affects card hashes.
pub fn parse_title(text: &str) -> Option<String> {
    text.lines().find_map(|raw| {
        let rest = raw.trim().strip_prefix(MARKUP_COMMENT)?;
        let title = rest.trim().strip_prefix("title:")?.trim();
        (!title.is_empty()).then(|| title.to_string())
    })
}

/// What a trace walks (`% trace: <path description>`), if the deck declares it.
/// A `% trace:` marks the deck as a **trace** — a guided predict-and-verify
/// walk (see [`crate::trace`]) rather than a plain card deck. The first such
/// line wins. Invisible to the card parser, so it never affects card hashes.
pub fn parse_trace(text: &str) -> Option<String> {
    text.lines().find_map(|raw| {
        let rest = raw.trim().strip_prefix(MARKUP_COMMENT)?;
        let desc = rest.trim().strip_prefix("trace:")?.trim();
        (!desc.is_empty()).then(|| desc.to_string())
    })
}

/// Parses a single `% key: value` directive line into a lower-cased key and
/// trimmed value. Returns `None` for non-directive `%` lines: prose comments
/// (key contains whitespace, like `% Then learn with:`), empty key/value, and
/// `link` (handled by [`parse_links`]).
/// Splits a `% at:` value into its asset locator and the optional ` from
/// <origin>` provenance a frozen snapshot appends (`29.rs from src/caching.rs:46-66`
/// → `("29.rs", Some("src/caching.rs:46-66"))`). The separator is spaced, so a
/// path like `from_x.rs` stays intact.
fn split_at_origin(value: &str) -> (String, Option<String>) {
    match value.split_once(" from ") {
        Some((at, origin)) => (at.trim().to_string(), Some(origin.trim().to_string())),
        None => (value.trim().to_string(), None),
    }
}

fn directive(raw: &str) -> Option<(String, String)> {
    let rest = raw.trim().strip_prefix(MARKUP_COMMENT)?;
    let (key, value) = rest.split_once(':')?;
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty() || key.contains(char::is_whitespace) || key == "link" {
        return None;
    }
    let value = value.trim();
    (!value.is_empty()).then(|| (key, value.to_string()))
}

/// Collects deck-level `% key: value` directives (e.g. `% reveal: line`) from the
/// header — the lines before the first card. Directives after a card front are
/// per-card overrides (handled while parsing the card), so deck-level ones must
/// sit at the top. These are invisible to the card parser and don't affect
/// hashes.
pub fn parse_directives(text: &str) -> Vec<(String, String)> {
    text.lines()
        .take_while(|raw| !raw.starts_with(MARKUP_FRONT))
        .filter_map(directive)
        .collect()
}

/// Parses deck text into cards. `subject` is the deck's file name and becomes
/// part of every card's identity hash.
pub fn parse_str(subject: &str, text: &str) -> Result<Vec<Card>, ParseError> {
    let subject: Arc<str> = Arc::from(subject);
    let mut cards = Vec::new();
    let mut state = State::Init;
    let mut partial: Option<PartialCard> = None;

    for (lineno, raw) in text.lines().enumerate() {
        let lineno = lineno + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        // The line is non-empty, so a first character exists.
        let first = line.chars().next().unwrap();

        // A front marker only counts at column 0 of the raw line; indented
        // `#` lines are answer content (shell comments, Rust attributes...).
        let is_front = first == MARKUP_FRONT && raw.starts_with(MARKUP_FRONT);

        match first {
            MARKUP_FRONT if is_front => {
                match state {
                    State::Front => {
                        // Point at the front line that lacks an answer.
                        return Err(ParseError::FrontWithoutBack(partial.unwrap().line));
                    }
                    State::Back | State::Note => {
                        partial.take().unwrap().build(&subject, &mut cards)?;
                    }
                    State::Init => {}
                }
                // A front is always a plain `# ` front; cloze is declared per
                // card with `% reveal: cloze` (which sets `partial.cloze`).
                let front = line[MARKUP_FRONT.len_utf8()..].trim();
                if front.is_empty() {
                    return Err(ParseError::EmptyFront(lineno));
                }
                partial = Some(PartialCard {
                    front: front.to_string(),
                    back: Vec::new(),
                    note: None,
                    line: lineno,
                    cloze: false,
                    reveal: None,
                    input: None,
                    direction: None,
                    image: None,
                    image_back: None,
                    at: None,
                    at_origin: None,
                    origin: None,
                    givens: Vec::new(),
                });
                state = State::Front;
            }
            MARKUP_NOTE => {
                match state {
                    State::Init => return Err(ParseError::NoteBeforeFront(lineno)),
                    State::Front => return Err(ParseError::NoteBeforeBack(lineno)),
                    State::Back | State::Note => {}
                }
                // Strip the marker and one separating space, but keep any
                // further leading whitespace so indented note content (e.g.
                // code inside a ``` fence) survives. Trailing space is dropped.
                let after = &line[MARKUP_NOTE.len_utf8()..];
                let text = after.strip_prefix(' ').unwrap_or(after).trim_end();
                // `partial` is Some whenever state is Back or Note. Further
                // `!` lines extend the note by one line each.
                let note = &mut partial.as_mut().unwrap().note;
                match note {
                    Some(note) => {
                        note.push('\n');
                        note.push_str(text);
                    }
                    None => *note = Some(text.to_string()),
                }
                state = State::Note;
            }
            MARKUP_COMMENT => {
                // A `% key: value` directive inside a card is a per-card
                // override (`reveal`, `direction`, `img`, `img-back`);
                // unrecognized keys are ignored, like deck-level directives. `%`
                // lines before the first card are deck-level (handled by
                // parse_directives).
                if state != State::Init
                    && let Some((key, value)) = directive(line)
                {
                    let partial = partial.as_mut().unwrap();
                    match key.as_str() {
                        "reveal" => {
                            if let Ok(r) = Reveal::from_str(&value, true) {
                                partial.reveal = Some(r);
                                if r == Reveal::Cloze {
                                    partial.cloze = true;
                                }
                            }
                        }
                        "input" => {
                            if let Ok(i) = Input::from_str(&value, true) {
                                partial.input = Some(i);
                            }
                        }
                        "direction" => {
                            if let Ok(d) = Direction::from_str(&value, true) {
                                partial.direction = Some(d);
                            }
                        }
                        "img" => partial.image = Some(value),
                        "img-back" => partial.image_back = Some(value),
                        "at" => {
                            // A frozen `% at:` carries `<asset> from <origin>`; split
                            // the origin provenance off the asset locator.
                            let (at, origin) = split_at_origin(&value);
                            partial.at = Some(at);
                            partial.at_origin = origin;
                        }
                        "origin" => partial.origin = Some(value),
                        "given" => partial.givens.push(value),
                        _ => {}
                    }
                }
            }
            _ => {
                match state {
                    State::Init => return Err(ParseError::BackBeforeFront(lineno)),
                    State::Note => return Err(ParseError::BackAfterNote(lineno)),
                    State::Front | State::Back => {}
                }
                // Strip a leading backslash only when it escapes a markup
                // character, so an ordinary backslash is left untouched.
                let second = line.chars().nth(1);
                let back_line =
                    if first == MARKUP_ESCAPE && second.is_some_and(|c| MARKUP.contains(&c)) {
                        &line[MARKUP_ESCAPE.len_utf8()..]
                    } else {
                        line
                    };
                partial
                    .as_mut()
                    .unwrap()
                    .back
                    .push((lineno, back_line.to_string()));
                state = State::Back;
            }
        }
    }

    if state == State::Front {
        // A trailing front without an answer; point at its line.
        return Err(ParseError::FrontWithoutBack(partial.unwrap().line));
    }
    if let Some(p) = partial.take() {
        p.build(&subject, &mut cards)?;
    }

    Ok(cards)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_card() {
        let cards = parse_str("s", "# front\n\tback").unwrap();
        assert_eq!(1, cards.len());
        assert_eq!("front", cards[0].front);
        assert_eq!(vec!["back"], cards[0].back);
        assert_eq!(None, cards[0].note);
        assert_eq!(1, cards[0].line);
    }

    #[test]
    fn at_splits_off_the_from_origin_provenance() {
        let cards = parse_str("s", "# q\n\tp\n\t% at: 29.rs from src/caching.rs:46-66\n").unwrap();
        assert_eq!(Some("29.rs".to_string()), cards[0].at);
        assert_eq!(Some("src/caching.rs:46-66".to_string()), cards[0].at_origin);
    }

    #[test]
    fn at_without_from_keeps_the_whole_locator() {
        let cards = parse_str("s", "# q\n\tp\n\t% at: src/from_x.rs:1-3\n").unwrap();
        assert_eq!(Some("src/from_x.rs:1-3".to_string()), cards[0].at);
        assert_eq!(None, cards[0].at_origin);
    }

    #[test]
    fn per_card_origin_directive_is_parsed() {
        let cards = parse_str(
            "s",
            "# q\n\tp\n\t% origin: /crate\n\t% at: 1.rs from a.rs:1\n",
        )
        .unwrap();
        assert_eq!(Some("/crate".to_string()), cards[0].origin);
    }

    #[test]
    fn multi_line_back_is_trimmed_per_line() {
        let text = "# front\n\tvar (\n   \t a = 1\n\t)\n";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(vec!["var (", "a = 1", ")"], cards[0].back);
    }

    #[test]
    fn note_after_back() {
        let cards = parse_str("s", "# front\n\tback\n\t! a note").unwrap();
        assert_eq!(Some("a note".to_string()), cards[0].note);
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let text = "% header comment\n\n# front\n\nback\n\n% trailing\n";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(1, cards.len());
        assert_eq!(vec!["back"], cards[0].back);
    }

    #[test]
    fn escaped_markup_in_back() {
        let cards = parse_str("s", "# front\n\t\\%\n\t\\#\n\t\\!").unwrap();
        assert_eq!(vec!["%", "#", "!"], cards[0].back);
    }

    #[test]
    fn backslash_without_markup_kept_verbatim() {
        let cards = parse_str("s", "# front\n\t\\n is a newline").unwrap();
        assert_eq!(vec!["\\n is a newline"], cards[0].back);
    }

    #[test]
    fn several_cards() {
        let text = "# a\n1\n# b\n2\n3\n! note b\n# c\n4\n";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(3, cards.len());
        assert_eq!("a", cards[0].front);
        assert_eq!(vec!["2", "3"], cards[1].back);
        assert_eq!(Some("note b".to_string()), cards[1].note);
        assert_eq!("c", cards[2].front);
        assert_eq!(7, cards[2].line);
    }

    #[test]
    fn error_back_before_front() {
        assert_eq!(Err(ParseError::BackBeforeFront(1)), parse_str("s", "back"));
    }

    #[test]
    fn error_front_without_back() {
        assert_eq!(
            Err(ParseError::FrontWithoutBack(1)),
            parse_str("s", "# a\n# b\nback")
        );
        assert_eq!(
            Err(ParseError::FrontWithoutBack(2)),
            parse_str("s", "% x\n# a\n")
        );
    }

    #[test]
    fn error_note_placement() {
        assert_eq!(
            Err(ParseError::NoteBeforeFront(1)),
            parse_str("s", "! note")
        );
        assert_eq!(
            Err(ParseError::NoteBeforeBack(2)),
            parse_str("s", "# a\n! note")
        );
        assert_eq!(
            Err(ParseError::BackAfterNote(4)),
            parse_str("s", "# a\nback\n! note\nmore back")
        );
    }

    #[test]
    fn links_are_collected_and_stay_comments() {
        let text = "% link: https://docs.rs/tokio\n\
                    % a normal comment\n\
                    %link:https://example.org/spec\n\
                    # front\n\
                    \tback\n\
                    % link:\n";
        assert_eq!(
            vec!["https://docs.rs/tokio", "https://example.org/spec"],
            parse_links(text)
        );
        // The card parser is unaffected.
        let cards = parse_str("s", text).unwrap();
        assert_eq!(1, cards.len());
        assert_eq!(vec!["back"], cards[0].back);
    }

    #[test]
    fn multi_line_note() {
        let cards = parse_str("s", "# a\nback\n! one\n! two\n! three").unwrap();
        assert_eq!(Some("one\ntwo\nthree".to_string()), cards[0].note);
    }

    #[test]
    fn requires_are_collected() {
        let text = "% requires: basics\n\
                    %requires:more.txt\n\
                    % link: https://example.org\n\
                    % a normal comment\n\
                    # front\n\tback\n";
        assert_eq!(vec!["basics", "more.txt"], parse_requires(text));
    }

    #[test]
    fn sources_are_collected_and_stay_comments() {
        let text = "% source: https://doc.rust-lang.org/book/ch04.html\n\
                    %source:notes.md\n\
                    % requires: basics\n\
                    % a normal comment\n\
                    # front\n\tback\n\
                    % source:\n";
        assert_eq!(
            vec!["https://doc.rust-lang.org/book/ch04.html", "notes.md"],
            parse_sources(text)
        );
        // The card parser is unaffected by source lines.
        let cards = parse_str("s", text).unwrap();
        assert_eq!(1, cards.len());
        assert_eq!(vec!["back"], cards[0].back);
    }

    #[test]
    fn directives_are_collected() {
        let text = "% reveal: line\n\
                    %  Order:Sequential \n\
                    % link: https://example.org\n\
                    % Then learn it with: alix\n\
                    % a plain comment\n\
                    # front\n\tback\n";
        assert_eq!(
            vec![
                ("reveal".to_string(), "line".to_string()),
                // Key is lower-cased; value keeps its case.
                ("order".to_string(), "Sequential".to_string()),
            ],
            parse_directives(text)
        );
        // The card parser is unaffected by directive lines.
        let cards = parse_str("s", text).unwrap();
        assert_eq!(1, cards.len());
    }

    #[test]
    fn reveal_cloze_declares_a_cloze_card_with_a_stable_id() {
        // The card that used to be written `#? ...` with {{}} gaps keeps its id
        // when re-declared via a per-card `% reveal: cloze` — ids hash cloze
        // structure, not the retired `#?` marker. `3946244523907553015` is the
        // frozen id the `#? capital?` spelling produced before the marker was
        // retired.
        let new = "# capital?\n% reveal: cloze\n\tParis is the capital of {{France}}\n";
        let new_id = parse_str("geo.txt", new).unwrap()[0].id();
        assert_eq!(
            3946244523907553015, new_id,
            "retiring #? must not reshuffle cloze ids"
        );
    }

    #[test]
    fn reveal_line_sets_the_reveal_method() {
        // A per-card directive (after the front); `parse_str` applies per-card
        // directives, while a header `% reveal:` is deck-level (folded at load).
        let cards = parse_str("d.txt", "# steps?\n% reveal: line\n\ta\n\tb\n").unwrap();
        assert_eq!(cards[0].reveal, Some(Reveal::Line));
    }

    #[test]
    fn mode_directive_is_no_longer_parsed() {
        // `% mode:` is retired; the line is ignored (not an error), no field set.
        let cards = parse_str("d.txt", "% mode: typing\n# q?\n\ta\n").unwrap();
        // A card has no `mode` field anymore; assert the card parsed and is plain.
        assert_eq!(cards[0].front, "q?");
        assert_eq!(None, cards[0].reveal);
    }

    #[test]
    fn per_card_reveal_directive_is_parsed() {
        let text = "# a\n% reveal: line\n\tx\n# b\n\ty\n";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(Some(Reveal::Line), cards[0].reveal);
        assert_eq!(None, cards[1].reveal); // no per-card directive
    }

    #[test]
    fn per_card_input_directive_is_parsed() {
        let cards = parse_str("d.txt", "# a\n% input: draw\n\tx\n# b\n\ty\n").unwrap();
        assert_eq!(Some(Input::Draw), cards[0].input);
        assert_eq!(None, cards[1].input);
    }

    #[test]
    fn per_card_direction_directive_is_parsed() {
        // parse_str records the declared direction; expansion happens at load.
        let cards = parse_str("s", "# a\n% direction: both\n\tx\n# b\n\ty\n").unwrap();
        assert_eq!(Some(Direction::Both), cards[0].direction);
        assert_eq!(None, cards[1].direction);
    }

    #[test]
    fn per_card_image_directives_are_parsed() {
        let text = "# q\n% img: moon.png\n% img-back: phase.png\n\tWaxing\n";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(Some(PathBuf::from("moon.png")), cards[0].image);
        assert_eq!(Some(PathBuf::from("phase.png")), cards[0].image_back);
    }

    #[test]
    fn cloze_card_ignores_image_directive() {
        // `% img:` is only stamped on plain cards; cloze sub-cards keep None.
        let cards = parse_str("s", "# f\n% reveal: cloze\n% img: x.png\n{{a}} b\n").unwrap();
        assert_eq!(None, cards[0].image);
    }

    #[test]
    fn per_card_at_locator_is_parsed_verbatim() {
        // The trace locator keeps its colons and ranges as written.
        let cards = parse_str("s", "# predict\n% at: src/card.rs:151-158\n\tpoint\n").unwrap();
        assert_eq!(Some("src/card.rs:151-158".to_string()), cards[0].at);
        // A card without one stays None.
        let bare = parse_str("s", "# q\n\ta\n").unwrap();
        assert_eq!(None, bare[0].at);
    }

    #[test]
    fn per_card_given_lines_are_collected_in_order() {
        let text = "# q\n\
                    % given: state — the parser position\n\
                    % given: partial — the card being assembled\n\
                    \tkey point\n";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(
            vec![
                "state — the parser position".to_string(),
                "partial — the card being assembled".to_string(),
            ],
            cards[0].givens
        );
        // A card without any `% given:` has an empty list.
        assert!(parse_str("s", "# q\n\ta\n").unwrap()[0].givens.is_empty());
    }

    #[test]
    fn trace_marks_a_deck_and_keeps_the_path() {
        assert_eq!(
            Some("how a keypress becomes a saved grade".to_string()),
            parse_trace("% trace: how a keypress becomes a saved grade\n# f\n\tb\n")
        );
        assert_eq!(None, parse_trace("# f\n\tb\n"));
        assert_eq!(None, parse_trace("% trace:\n# f\n\tb\n")); // empty
    }

    #[test]
    fn title_is_parsed_keeping_spaces_and_colons() {
        assert_eq!(
            Some("Rust: The Book".to_string()),
            parse_title("% title: Rust: The Book\n# f\n\tb\n")
        );
        assert_eq!(None, parse_title("# f\n\tb\n"));
        assert_eq!(None, parse_title("% title:\n# f\n\tb\n")); // empty
    }

    #[test]
    fn directives_are_header_only() {
        // A `% reveal:` after a card front is a per-card override, not deck-level.
        assert!(parse_directives("# a\n% reveal: cloze\n\tx\n").is_empty());
        // In the header it is collected.
        assert_eq!(
            vec![("reveal".to_string(), "line".to_string())],
            parse_directives("% reveal: line\n# a\n\tx\n")
        );
    }

    #[test]
    fn note_preserves_interior_indentation() {
        // Only the marker and one separating space are stripped; further
        // leading whitespace (e.g. indented code in a fence) is kept.
        let text = "# a\nback\n! ```rust\n! fn main() {\n!     let x = 1;\n! }\n! ```";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(
            Some("```rust\nfn main() {\n    let x = 1;\n}\n```".to_string()),
            cards[0].note
        );
    }

    #[test]
    fn note_lines_may_be_separated_by_blanks_and_comments() {
        let text = "# a\nback\n! one\n\n% comment\n! two\n# b\nback b\n";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(2, cards.len());
        assert_eq!(Some("one\ntwo".to_string()), cards[0].note);
        assert_eq!(None, cards[1].note);
    }

    #[test]
    fn multi_line_note_on_cloze_cards() {
        let cards = parse_str("s", "# f\n% reveal: cloze\n{{a}} b\n! one\n! two").unwrap();
        assert_eq!(Some("one\ntwo".to_string()), cards[0].note);
    }

    #[test]
    fn error_empty_front() {
        assert_eq!(Err(ParseError::EmptyFront(1)), parse_str("s", "#\nback"));
    }

    #[test]
    fn indented_hash_is_answer_content() {
        // Code answers contain '#' lines (shell comments, Rust attributes);
        // only a '#' at column 0 starts a new card.
        let text = "# Dockerfile for a bin project\n\
                    \tFROM rust\n\
                    \t# Pre-built dependencies\n\
                    \tRUN cargo build\n\
                    # Which attribute exports an async fn?\n\
                    \t#[uniffi::export(async_runtime = \"tokio\")]\n";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(2, cards.len());
        assert_eq!(
            vec!["FROM rust", "# Pre-built dependencies", "RUN cargo build"],
            cards[0].back
        );
        assert_eq!(
            vec!["#[uniffi::export(async_runtime = \"tokio\")]"],
            cards[1].back
        );
    }

    #[test]
    fn indented_hash_before_any_front_is_an_error() {
        assert_eq!(
            Err(ParseError::BackBeforeFront(1)),
            parse_str("s", "\t# comment\n")
        );
    }

    #[test]
    fn escaped_hash_and_indented_hash_yield_the_same_back_line() {
        // Both spellings must produce identical content (and therefore the
        // same card hash), so a deck using the escape keeps its progress.
        let escaped = parse_str("s", "# f\n\t\\# x\n").unwrap();
        let indented = parse_str("s", "# f\n\t# x\n").unwrap();
        assert_eq!(escaped[0].back, indented[0].back);
        assert_eq!(escaped[0].id(), indented[0].id());
    }

    #[test]
    fn indented_cloze_marker_is_answer_content() {
        let cards = parse_str("s", "# f\n\t#? not a cloze front\n").unwrap();
        assert_eq!(vec!["#? not a cloze front"], cards[0].back);
    }

    #[test]
    fn cloze_front_expands_to_sub_cards() {
        let text =
            "# Complete the quote\n% reveal: cloze\n\tTo {{be}} or not to {{be}}\n\t! Hamlet\n";
        let cards = parse_str("s", text).unwrap();
        assert_eq!(2, cards.len());
        assert_eq!("Complete the quote", cards[0].front);
        assert_eq!(vec!["To ____ or not to […]"], cards[0].context);
        assert_eq!(vec!["be"], cards[0].back);
        assert_eq!(Some("Hamlet".to_string()), cards[0].note);
        assert_eq!(vec!["To […] or not to ____"], cards[1].context);
        assert_ne!(cards[0].id(), cards[1].id());
    }

    #[test]
    fn a_question_mark_front_is_a_plain_card() {
        // With `#?` retired, "# ? ..." is just a plain card whose front is "?...".
        let cards = parse_str("s", "# ? matches one char\n\tglob\n").unwrap();
        assert_eq!(1, cards.len());
        assert_eq!("? matches one char", cards[0].front);
        assert!(cards[0].context.is_empty());
    }

    #[test]
    fn braces_in_plain_cards_stay_literal() {
        // Code answers contain braces; without `% reveal: cloze` they are never
        // treated as cloze holes, and the identity hash (over the back lines
        // verbatim) is unaffected.
        let cards = parse_str("s", "# main\n\tfunc main() {}\n\tx := T{ a, b }\n").unwrap();
        assert_eq!(1, cards.len());
        assert_eq!(vec!["func main() {}", "x := T{ a, b }"], cards[0].back);
        assert!(cards[0].hash_lines.is_none());
    }

    #[test]
    fn cloze_errors_carry_line_numbers() {
        // `ClozeWithoutHoles` points at the front line; the hole errors point at
        // the offending answer line (shifted by the `% reveal: cloze` directive).
        assert_eq!(
            Err(ParseError::ClozeWithoutHoles(1)),
            parse_str("s", "# front\n% reveal: cloze\n\tno holes\n")
        );
        assert_eq!(
            Err(ParseError::UnclosedClozeHole(4)),
            parse_str(
                "s",
                "# front\n% reveal: cloze\n\tok {{fine}}\n\tbad {{oops\n"
            )
        );
        assert_eq!(
            Err(ParseError::EmptyClozeHole(3)),
            parse_str("s", "# front\n% reveal: cloze\n\tbad {{}} here\n")
        );
    }
}
