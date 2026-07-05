//! Anki TSV import — turn a tab-separated `front<TAB>back` export into the
//! alix plain-text deck format.
//!
//! Aimed at Anki's "Notes in Plain Text" export (fields separated by a tab):
//! the first field is the front, the second is the back, further fields are
//! ignored. Anki's `#`-prefixed header lines (`#separator:tab`, `#html:true`,
//! …) are skipped, `<br>` tags become answer-line breaks, a few common HTML
//! entities are decoded, and a back line that would otherwise read as an alix
//! comment (`%`) or note (`!`) is backslash-escaped. The result is plain text
//! the caller validates with [`crate::parser::parse_str`] and writes — no card
//! identity or scheduling is involved.

use anyhow::{Result, bail};

/// Converts Anki-style TSV `text` into alix deck format. Errors only when no
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

        // The front is a single `#` line, so collapse any internal breaks
        // (a `<br>` became a newline) into spaces.
        let front_line = front.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push_str("# ");
        out.push_str(&front_line);
        out.push('\n');
        // Each back line is indented (so a leading `#` is plain content) and a
        // leading `%`/`!` — a comment/note at any indent — is escaped.
        for bl in back.lines() {
            let bl = bl.trim();
            if bl.is_empty() {
                continue;
            }
            out.push('\t');
            out.push_str(&escape_leading_markup(bl));
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

/// Backslash-escapes a back line that would otherwise be read as an alix comment
/// (`%`) or note (`!`). A leading `#` needs no escape: the line is indented, so
/// it's answer content, not a card front (those only count at column 0).
///
/// `pub(crate)`: also reused by [`crate::store::promote_virtual`] when
/// rendering a virtual card's back lines to deck text.
pub(crate) fn escape_leading_markup(line: &str) -> String {
    if line.starts_with('%') || line.starts_with('!') {
        format!("\\{line}")
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_str;

    #[test]
    fn one_row_becomes_one_card() {
        let deck = tsv_to_deck("bonjour\thello\n").unwrap();
        assert_eq!("# bonjour\n\thello\n\n", deck);
    }

    #[test]
    fn skips_headers_blanks_and_half_rows() {
        let tsv = "#separator:tab\n#html:true\n\nbonjour\thello\nlonely\nmerci\tthanks\n";
        let deck = tsv_to_deck(tsv).unwrap();
        // Only the two complete rows survive (the header, blank, and the
        // front-only `lonely` row are dropped).
        let cards = parse_str("fr.txt", &deck).unwrap();
        assert_eq!(2, cards.len());
    }

    #[test]
    fn br_tags_split_the_back_into_lines() {
        let deck = tsv_to_deck("q\tone<br>two<br/>three\n").unwrap();
        assert_eq!("# q\n\tone\n\ttwo\n\tthree\n\n", deck);
    }

    #[test]
    fn decodes_common_entities() {
        let deck = tsv_to_deck("a &amp; b\tx &lt; y\n").unwrap();
        assert_eq!("# a & b\n\tx < y\n\n", deck);
    }

    #[test]
    fn escapes_a_back_line_that_starts_with_a_directive_char() {
        let deck = tsv_to_deck("q\t% literal percent\n").unwrap();
        assert_eq!("# q\n\t\\% literal percent\n\n", deck);
        // And it round-trips: the escaped line is answer content, not a comment.
        let cards = parse_str("d.txt", &deck).unwrap();
        assert_eq!(vec!["% literal percent".to_string()], cards[0].back);
    }

    #[test]
    fn output_parses_back_into_the_original_cards() {
        let deck = tsv_to_deck("bonjour\thello\nmerci\tthank you\n").unwrap();
        let cards = parse_str("fr.txt", &deck).unwrap();
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
