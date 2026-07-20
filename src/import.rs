use anyhow::{Result, bail};

pub fn tsv_to_deck(text: &str) -> Result<String> {
    let mut out = String::new();
    let mut cards = 0usize;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue; // blank, or an Anki `#header:` line
        }
        let mut fields = line.split('\t');
        let front = clean_field(fields.next().unwrap_or(""));
        let back = clean_field(fields.next().unwrap_or(""));
        // A row missing either side isn't a card; skip it rather than emit a half card that won't
        // parse.
        if front.trim().is_empty() || back.trim().is_empty() {
            continue;
        }

        // The front is a single "## " heading, so any <br>-turned newline is collapsed into a space
        // here.
        let front_line = front.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push_str("## ");
        out.push_str(&front_line);
        out.push('\n');
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

fn clean_field(field: &str) -> String {
    let mut s = field.trim().to_string();
    for br in ["<br>", "<br/>", "<br />", "<BR>", "<BR/>", "<BR />"] {
        s = s.replace(br, "\n");
    }
    // &amp; is decoded last so an encoded entity like &amp;lt; is not turned into a live <.
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

const STRUCTURAL: [&str; 6] = ["##", ">", "---", "<!--", "```", "~~~"];

// \cloze is escaped anywhere in the line (unlike the leading-only structural markers) since
// imported prose is never a deliberate hole.
fn escape_structure(line: &str) -> String {
    let line = line.replace("\\blank", "\\\\blank");
    if STRUCTURAL.iter().any(|marker| line.starts_with(marker)) {
        format!("\\{line}")
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_str;

    #[test]
    fn one_row_becomes_one_card() {
        let deck = tsv_to_deck("bonjour\thello\n").unwrap();
        assert_eq!("## bonjour\nhello\n\n", deck);
    }

    #[test]
    fn skips_headers_blanks_and_half_rows() {
        let tsv = "#separator:tab\n#html:true\n\nbonjour\thello\nlonely\nmerci\tthanks\n";
        let deck = tsv_to_deck(tsv).unwrap();
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
        let deck = tsv_to_deck("q\t&gt; quoted reply\n").unwrap();
        assert_eq!("## q\n\\> quoted reply\n\n", deck);
        let cards = parse_str("d.md", &deck).unwrap();
        assert_eq!(vec!["> quoted reply".to_string()], cards[0].back);

        let deck = tsv_to_deck("q\t--- dashes\n").unwrap();
        assert_eq!("## q\n\\--- dashes\n\n", deck);
        let cards = parse_str("d.md", &deck).unwrap();
        assert_eq!(vec!["--- dashes".to_string()], cards[0].back);
    }

    #[test]
    fn a_literal_cloze_marker_is_doubled_not_a_hole() {
        let deck = tsv_to_deck("q\tthe \\blank{x} marker\n").unwrap();
        let cards = parse_str("d.md", &deck).unwrap();
        assert_eq!(1, cards.len());
        assert!(cards[0].hole.is_none(), "no hole from imported prose");
        assert_eq!(vec!["the \\blank{x} marker".to_string()], cards[0].back);
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
