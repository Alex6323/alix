//! Anki TSV import — turn a tab-separated `front<TAB>back` export into the
//! alix L1 deck format.
//!
//! Aimed at Anki's "Notes in Plain Text" export (fields separated by a tab):
//! the first field is the front, the second is the back, further fields are
//! ignored. Anki's `#`-prefixed header lines (`#separator:tab`, `#html:true`,
//! …) are skipped, `<br>` tags become answer-line breaks, a few common HTML
//! entities are decoded, and a back line that would otherwise read as L1
//! structure (a `## ` front, a `> ` note, a `---` divider, a `<!--` comment,
//! a code fence) is backslash-escaped. The result is plain text the caller
//! validates with [`crate::l1::parse_str`] and writes — no card identity or
//! scheduling is involved (the placed deck is stamped by its creation path).

use anyhow::{Result, bail};

/// Converts Anki-style TSV `text` into the L1 deck format. Errors only when no
/// usable `front<TAB>back` row is found, so the caller never writes an empty
/// deck.
pub fn tsv_to_deck(text: &str) -> Result<String> {
    let mut out = String::new();
    let mut cards = 0usize;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r'); // tolerate CRLF
        if line.trim().is_empty() || line.starts_with('#') {
            continue; // blank, or an Anki `#header:` line
        }
        let mut fields = line.split('\t');
        let front = clean_field(fields.next().unwrap_or(""));
        let back = clean_field(fields.next().unwrap_or(""));
        // A row missing either side isn't a card; skip it rather than emit a
        // half card that won't parse.
        if front.trim().is_empty() || back.trim().is_empty() {
            continue;
        }

        // The front is a single `## ` heading, so collapse any internal breaks
        // (a `<br>` became a newline) into spaces.
        let front_line = front.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push_str("## ");
        out.push_str(&front_line);
        out.push('\n');
        // Back lines are plain (unindented) under the front; anything that
        // would read as structure is escaped so it stays literal content.
        for bl in back.lines() {
            let bl = bl.trim();
            if bl.is_empty() {
                continue;
            }
            out.push_str(&escape_structure(bl));
            out.push('\n');
        }
        out.push('\n');
        cards += 1;
    }
    if cards == 0 {
        bail!("no cards found — expected tab-separated `front<TAB>back` lines");
    }
    Ok(out)
}

/// Decodes the Anki-isms in one field: `<br>` variants to newlines and the few
/// HTML entities a plain-text export commonly leaves in. Other HTML is left as
/// is for the user to clean up.
fn clean_field(field: &str) -> String {
    let mut s = field.trim().to_string();
    for br in ["<br>", "<br/>", "<br />", "<BR>", "<BR/>", "<BR />"] {
        s = s.replace(br, "\n");
    }
    // `&amp;` is decoded last so an encoded entity like `&amp;lt;` is not
    // turned into a live `<`.
    s = s
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    s
}

/// The line-leading markers the L1 parser treats as structure; a back line
/// starting with one is escaped with a backslash so it stays literal content
/// (mirrors the parser's escapable set).
const STRUCTURAL: [&str; 6] = ["##", ">", "---", "<!--", "```", "~~~"];

/// Escapes a back line so it can never be read as L1 structure: a leading
/// structural marker gains a backslash, and any `\cloze` in the line doubles
/// its backslash (the marker is active anywhere in answer content, and
/// imported prose is never a deliberate hole).
///
/// Only used within this module, by [`tsv_to_deck`].
fn escape_structure(line: &str) -> String {
    let line = line.replace("\\cloze", "\\\\cloze");
    if STRUCTURAL.iter().any(|marker| line.starts_with(marker)) {
        format!("\\{line}")
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l1::parse_str;

    #[test]
    fn one_row_becomes_one_card() {
        let deck = tsv_to_deck("bonjour\thello\n").unwrap();
        assert_eq!("## bonjour\nhello\n\n", deck);
    }

    #[test]
    fn skips_headers_blanks_and_half_rows() {
        let tsv = "#separator:tab\n#html:true\n\nbonjour\thello\nlonely\nmerci\tthanks\n";
        let deck = tsv_to_deck(tsv).unwrap();
        // Only the two complete rows survive (the header, blank, and the
        // front-only `lonely` row are dropped).
        let cards = parse_str("fr.md", &deck).unwrap();
        assert_eq!(2, cards.len());
    }

    #[test]
    fn br_tags_split_the_back_into_lines() {
        let deck = tsv_to_deck("q\tone<br>two<br/>three\n").unwrap();
        assert_eq!("## q\none\ntwo\nthree\n\n", deck);
    }

    #[test]
    fn decodes_common_entities() {
        let deck = tsv_to_deck("a &amp; b\tx &lt; y\n").unwrap();
        assert_eq!("## a & b\nx < y\n\n", deck);
    }

    #[test]
    fn escapes_a_back_line_that_would_read_as_structure() {
        // A decoded `>` at the start of a back line would otherwise become a
        // note; the escape keeps it answer content.
        let deck = tsv_to_deck("q\t&gt; quoted reply\n").unwrap();
        assert_eq!("## q\n\\> quoted reply\n\n", deck);
        let cards = parse_str("d.md", &deck).unwrap();
        assert_eq!(vec!["> quoted reply".to_string()], cards[0].back);

        // A `---` right under the front would otherwise divide front from
        // answer; escaped, it stays a literal line.
        let deck = tsv_to_deck("q\t--- dashes\n").unwrap();
        assert_eq!("## q\n\\--- dashes\n\n", deck);
        let cards = parse_str("d.md", &deck).unwrap();
        assert_eq!(vec!["--- dashes".to_string()], cards[0].back);
    }

    #[test]
    fn a_literal_cloze_marker_is_doubled_not_a_hole() {
        // Imported prose mentioning `\cloze{x}` must not mint a gap-fill card.
        let deck = tsv_to_deck("q\tthe \\cloze{x} marker\n").unwrap();
        let cards = parse_str("d.md", &deck).unwrap();
        assert_eq!(1, cards.len());
        assert!(cards[0].hole.is_none(), "no hole from imported prose");
        assert_eq!(vec!["the \\cloze{x} marker".to_string()], cards[0].back);
    }

    #[test]
    fn output_parses_back_into_the_original_cards() {
        let deck = tsv_to_deck("bonjour\thello\nmerci\tthank you\n").unwrap();
        let cards = parse_str("fr.md", &deck).unwrap();
        assert_eq!(2, cards.len());
        assert_eq!("bonjour", cards[0].front);
        assert_eq!(vec!["hello".to_string()], cards[0].back);
        assert_eq!("merci", cards[1].front);
    }

    #[test]
    fn empty_or_headers_only_is_an_error() {
        assert!(tsv_to_deck("").is_err());
        assert!(tsv_to_deck("#separator:tab\n#html:true\n").is_err());
    }
}
