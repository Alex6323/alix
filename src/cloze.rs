//! Cloze (fill-in-the-blank) cards.
//!
//! A card whose front marker is `#?` (instead of `#`) is a cloze card: every
//! `{{...}}` in its answer lines is a hole, and the card expands into one
//! sub-card per hole. Each sub-card shows the answer text with its hole
//! blanked out (`____`) while the other holes are hidden (`[…]`), so no
//! sub-card reveals its siblings' answers. The user produces only the
//! blanked text.
//!
//! Only the doubled `{{` / `}}` are special; a lone `{` or `}` is literal, so a
//! cloze card can hold code like `let p = Foo {};` untouched. A literal `{{`
//! can be written `\{\{` if ever needed.
//!
//! Identity: a sub-card hashes the parsed structure of its answer lines (text
//! plus hole contents, with the delimiters removed) plus its hole index (see
//! [`Card::hash_lines`]), so progress survives rewording the front or changing
//! the hole markup, and two holes with identical text get distinct identities.

use std::sync::Arc;

use crate::{card::Card, parser::ParseError};

/// What a hole is replaced with when it is the one being asked.
pub const BLANK: &str = "____";

/// What the other (sibling) holes are replaced with. Their content is never
/// shown while answering, otherwise reviewing one sub-card would reveal the
/// answers of its siblings.
pub const HIDDEN: &str = "[…]";

/// A piece of a cloze line: literal text or a hole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Hole(String),
}

/// Parses one answer line of a cloze card into segments. `lineno` is used
/// for error reporting.
pub fn parse_line(line: &str, lineno: usize) -> Result<Vec<Segment>, ParseError> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut hole: Option<String> = None;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' if matches!(chars.peek(), Some('{' | '}' | '\\')) => {
                let escaped = chars.next().unwrap();
                match &mut hole {
                    Some(h) => h.push(escaped),
                    None => text.push(escaped),
                }
            }
            // `{{` opens a hole; a lone `{` is literal.
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                if hole.is_some() {
                    return Err(ParseError::NestedClozeHole(lineno));
                }
                if !text.is_empty() {
                    segments.push(Segment::Text(std::mem::take(&mut text)));
                }
                hole = Some(String::new());
            }
            // `}}` closes the open hole; a lone `}` (or `}}` outside a hole) is
            // literal.
            '}' if hole.is_some() && chars.peek() == Some(&'}') => {
                chars.next();
                let h = hole.take().unwrap();
                if h.trim().is_empty() {
                    return Err(ParseError::EmptyClozeHole(lineno));
                }
                segments.push(Segment::Hole(h));
            }
            c => match &mut hole {
                Some(h) => h.push(c),
                None => text.push(c),
            },
        }
    }

    if hole.is_some() {
        return Err(ParseError::UnclosedClozeHole(lineno));
    }
    if !text.is_empty() {
        segments.push(Segment::Text(text));
    }
    Ok(segments)
}

/// The delimiter-free representation of a parsed line used for identity: text
/// runs verbatim, each hole's content fenced by a unit-separator byte that
/// can't occur in deck input. Hashing this instead of the raw `{{...}}` text
/// means the hole markup can change without orphaning progress.
fn hash_repr(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            Segment::Text(t) => out.push_str(t),
            Segment::Hole(h) => {
                out.push('\u{1f}');
                out.push_str(h);
                out.push('\u{1f}');
            }
        }
    }
    out
}

/// Expands a cloze card into one sub-card per hole.
///
/// `back` holds the raw answer lines together with their 1-based line
/// numbers in the deck file; `line` is the line number of the card front.
pub fn expand(
    subject: &Arc<str>,
    front: &str,
    back: &[(usize, String)],
    note: Option<&str>,
    line: usize,
) -> Result<Vec<Card>, ParseError> {
    let lines: Vec<Vec<Segment>> = back
        .iter()
        .map(|(lineno, text)| parse_line(text, *lineno))
        .collect::<Result<_, _>>()?;

    let total: usize = lines
        .iter()
        .flatten()
        .filter(|s| matches!(s, Segment::Hole(_)))
        .count();
    if total == 0 {
        return Err(ParseError::ClozeWithoutHoles(line));
    }

    // Identity hashes the parsed structure of each line (text + hole contents,
    // delimiters removed), so changing the hole markup never reshuffles ids.
    let structure: Vec<String> = lines.iter().map(|segments| hash_repr(segments)).collect();

    let mut cards = Vec::with_capacity(total);

    let holes = lines.iter().enumerate().flat_map(|(li, segments)| {
        segments
            .iter()
            .enumerate()
            .filter_map(move |(si, seg)| match seg {
                Segment::Hole(h) => Some((li, si, h.clone())),
                Segment::Text(_) => None,
            })
    });
    for (hole_index, (target_line, target_seg, answer)) in holes.enumerate() {
        let context: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(li, segments)| {
                let mut rendered = String::new();
                for (si, seg) in segments.iter().enumerate() {
                    match seg {
                        Segment::Text(t) => rendered.push_str(t),
                        Segment::Hole(_) if li == target_line && si == target_seg => {
                            rendered.push_str(BLANK)
                        }
                        Segment::Hole(_) => rendered.push_str(HIDDEN),
                    }
                }
                rendered
            })
            .collect();

        // The front is the author's prompt as written; which hole is being
        // asked is shown by the blanked-out (`____`) position in the context,
        // not by a counter.
        let front = front.to_string();

        let mut hash_lines = structure.clone();
        hash_lines.push(format!("#cloze:{hole_index}"));

        cards.push(Card {
            subject: Arc::clone(subject),
            front,
            context,
            back: vec![answer],
            note: note.map(String::from),
            line,
            hash_lines: Some(hash_lines),
            mode: None,
            direction: None,
            image: None,
            image_back: None,
            frontend: None,
            at: None,
            givens: Vec::new(),
        });
    }

    Ok(cards)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Segment {
        Segment::Text(s.to_string())
    }
    fn hole(s: &str) -> Segment {
        Segment::Hole(s.to_string())
    }

    #[test]
    fn parse_line_without_holes() {
        assert_eq!(
            vec![text("plain text")],
            parse_line("plain text", 1).unwrap()
        );
    }

    #[test]
    fn parse_line_with_holes() {
        assert_eq!(
            vec![text("To "), hole("be"), text(" or not to "), hole("be")],
            parse_line("To {{be}} or not to {{be}}", 1).unwrap()
        );
    }

    #[test]
    fn parse_line_hole_at_edges() {
        assert_eq!(
            vec![hole("a"), text(" mid "), hole("b")],
            parse_line("{{a}} mid {{b}}", 1).unwrap()
        );
    }

    #[test]
    fn single_braces_are_literal() {
        // Code with single braces needs no escaping in a cloze card.
        assert_eq!(
            vec![text("fn main() {}")],
            parse_line("fn main() {}", 1).unwrap()
        );
        assert_eq!(
            vec![text("let p = Foo { x: "), hole("1"), text(" };")],
            parse_line("let p = Foo { x: {{1}} };", 1).unwrap()
        );
    }

    #[test]
    fn escaped_double_brace_is_literal() {
        // `\{` escapes a brace, so a literal `{{` can still be written.
        assert_eq!(vec![text("a {{ b")], parse_line("a \\{\\{ b", 1).unwrap());
    }

    #[test]
    fn stray_closing_brace_is_literal() {
        assert_eq!(vec![text("end }")], parse_line("end }", 1).unwrap());
    }

    #[test]
    fn parse_line_errors() {
        assert_eq!(
            Err(ParseError::UnclosedClozeHole(7)),
            parse_line("oops {{unclosed", 7)
        );
        assert_eq!(
            Err(ParseError::EmptyClozeHole(7)),
            parse_line("an {{}} empty", 7)
        );
        assert_eq!(
            Err(ParseError::EmptyClozeHole(7)),
            parse_line("a {{  }} blank", 7)
        );
        assert_eq!(
            Err(ParseError::NestedClozeHole(7)),
            parse_line("a {{nested {{hole}}}}", 7)
        );
    }

    fn subject() -> Arc<str> {
        Arc::from("deck.txt")
    }

    #[test]
    fn expand_single_hole() {
        let back = vec![(2, "To be or not to {{be}}".to_string())];
        let cards = expand(&subject(), "Complete the quote", &back, None, 1).unwrap();
        assert_eq!(1, cards.len());
        // The front is the author's prompt, unchanged.
        assert_eq!("Complete the quote", cards[0].front);
        assert_eq!(vec!["To be or not to ____"], cards[0].context);
        assert_eq!(vec!["be"], cards[0].back);
    }

    #[test]
    fn expand_multiple_holes() {
        let back = vec![(2, "To {{be}} or not to {{bee}}".to_string())];
        let cards = expand(&subject(), "Quote", &back, Some("n"), 1).unwrap();
        assert_eq!(2, cards.len());

        // The front is the same prompt for each sub-card; the active hole is
        // shown by the `____` position, not a counter.
        assert_eq!("Quote", cards[0].front);
        assert_eq!(vec!["To ____ or not to […]"], cards[0].context);
        assert_eq!(vec!["be"], cards[0].back);
        assert_eq!(Some("n".to_string()), cards[0].note);

        assert_eq!("Quote", cards[1].front);
        assert_eq!(vec!["To […] or not to ____"], cards[1].context);
        assert_eq!(vec!["bee"], cards[1].back);
    }

    /// Sibling answers must never appear in any sub-card's context;
    /// otherwise reviewing one sub-card spoils the others.
    #[test]
    fn sibling_hole_content_never_leaks() {
        let back = vec![(2, "a {{alpha}} b {{beta}} c {{gamma}}".to_string())];
        let cards = expand(&subject(), "f", &back, None, 1).unwrap();
        for card in &cards {
            let answer = &card.back[0];
            for other in cards.iter().filter(|c| &c.back[0] != answer) {
                assert!(
                    !card.context[0].contains(&other.back[0]),
                    "context {:?} leaks sibling answer {:?}",
                    card.context[0],
                    other.back[0]
                );
            }
        }
    }

    #[test]
    fn expand_across_lines() {
        let back = vec![
            (2, "first {{alpha}} line".to_string()),
            (3, "second {{beta}} line".to_string()),
        ];
        let cards = expand(&subject(), "f", &back, None, 1).unwrap();
        assert_eq!(2, cards.len());
        assert_eq!(vec!["first ____ line", "second […] line"], cards[0].context);
        assert_eq!(vec!["first […] line", "second ____ line"], cards[1].context);
        assert_eq!(vec!["beta"], cards[1].back);
    }

    #[test]
    fn identical_hole_texts_get_distinct_ids() {
        let back = vec![(2, "To {{be}} or not to {{be}}".to_string())];
        let cards = expand(&subject(), "Quote", &back, None, 1).unwrap();
        assert_eq!(2, cards.len());
        assert_ne!(cards[0].id(), cards[1].id());
    }

    #[test]
    fn ids_survive_front_rewording_but_not_text_changes() {
        let back = vec![(2, "a {{b}} c".to_string())];
        let v1 = expand(&subject(), "front one", &back, None, 1).unwrap();
        let v2 = expand(&subject(), "front two", &back, None, 5).unwrap();
        assert_eq!(v1[0].id(), v2[0].id());

        let changed = vec![(2, "a {{b}} d".to_string())];
        let v3 = expand(&subject(), "front one", &changed, None, 1).unwrap();
        assert_ne!(v1[0].id(), v3[0].id());
    }

    #[test]
    fn expand_without_holes_is_an_error() {
        let back = vec![(2, "no holes here".to_string())];
        assert_eq!(
            Err(ParseError::ClozeWithoutHoles(1)),
            expand(&subject(), "f", &back, None, 1)
        );
    }

    /// Single braces no longer make holes, so an old-style cloze answer now has
    /// none (the hard switch — such decks must be migrated to `{{ }}`).
    #[test]
    fn single_brace_answer_has_no_holes() {
        let back = vec![(2, "the {old} style".to_string())];
        assert_eq!(
            Err(ParseError::ClozeWithoutHoles(1)),
            expand(&subject(), "f", &back, None, 1)
        );
    }
}
