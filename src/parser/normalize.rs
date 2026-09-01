use std::borrow::Cow;

use super::{closes_fence, fence_opener};

/// A deck file's canonical bytes: no leading BOM, LF line endings, and,
/// outside a code fence, no trailing blanks except the two spaces of a hard
/// line break. A fence keeps the trailing blanks on every line it owns, its
/// opener and closer included: they are code, and the opener is fingerprinted
/// byte for byte.
pub fn normalize(text: &str) -> Cow<'_, str> {
    match rewrite(text) {
        Some(normalized) => Cow::Owned(normalized),
        None => Cow::Borrowed(text),
    }
}

fn rewrite(text: &str) -> Option<String> {
    let mut body = text;
    let mut changed = false;
    while let Some(rest) = body.strip_prefix('\u{feff}') {
        body = rest;
        changed = true;
    }
    let mut out = String::with_capacity(body.len());
    let mut fence = None;
    for (idx, raw) in body.split('\n').enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let line = raw.trim_end_matches('\r');
        changed |= line.len() != raw.len();
        match fence {
            Some((ch, open)) => {
                out.push_str(line);
                if closes_fence(line, ch, open) {
                    fence = None;
                }
            }
            None => match fence_opener(line) {
                Some(opened) => {
                    out.push_str(line);
                    fence = Some(opened);
                }
                None => {
                    let visible = drop_invisible(line);
                    let (kept, hard_break) = split_trailing(&visible);
                    out.push_str(kept);
                    if hard_break {
                        out.push_str("  ");
                    }
                    changed |= line.len() != kept.len() + if hard_break { 2 } else { 0 };
                }
            },
        }
    }
    changed.then_some(out)
}

/// Only the bidi reversal overrides drop; the embeddings LRE/RLE, PDF, and
/// every isolate are kept on purpose, they carry legitimate RTL text.
fn is_dropped(c: char) -> bool {
    matches!(
        c,
        '\u{000C}' | '\u{007F}' | '\u{202D}' | '\u{202E}' | '\u{FEFF}' | '\r'
    )
}

fn drop_invisible(line: &str) -> Cow<'_, str> {
    if line.contains(is_dropped) {
        Cow::Owned(line.chars().filter(|c| !is_dropped(*c)).collect())
    } else {
        Cow::Borrowed(line)
    }
}

/// `ends_with("  ")` is GFM's hard break: two spaces or more, after content.
fn split_trailing(line: &str) -> (&str, bool) {
    let kept = line.trim_end_matches([' ', '\t']);
    (kept, !kept.is_empty() && line.ends_with("  "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: &[(&str, &str, &str)] = &[
        (
            "## q\n",
            "## q\n",
            "canonical text is left exactly as authored",
        ),
        ("", "", "an empty document is canonical"),
        ("\u{feff}## q\n", "## q\n", "a leading BOM is dropped"),
        (
            "\u{feff}\u{feff}## q\n",
            "## q\n",
            "every leading BOM goes, so normalizing twice cannot differ from once",
        ),
        ("## q\r\na\r\n", "## q\na\n", "CRLF becomes LF"),
        (
            "a\r",
            "a",
            "a CR ending the last line goes even with no newline after it",
        ),
        (
            "a  \nb\n",
            "a  \nb\n",
            "two trailing spaces are a hard line break and survive",
        ),
        (
            "a     \nb\n",
            "a  \nb\n",
            "a longer run is still one hard break, written as two spaces",
        ),
        (
            "a \nb\n",
            "a\nb\n",
            "one trailing space breaks nothing and goes",
        ),
        ("a\t\nb\n", "a\nb\n", "a trailing tab goes"),
        (
            "a\t  \nb\n",
            "a  \nb\n",
            "a tab before the two spaces is trailing blank, and the break stays",
        ),
        (
            "a  \t\nb\n",
            "a\nb\n",
            "a tab after the spaces means the line does not end in spaces, so there is no break",
        ),
        (
            "   \n",
            "\n",
            "a blank line carries no content, so it is emptied rather than broken",
        ),
        (
            "a  ",
            "a  ",
            "the hard break is kept on a last line with no newline",
        ),
        ("a  \r\n", "a  \n", "the break survives the CRLF rewrite"),
        (
            "```\ncode \n```\n",
            "```\ncode \n```\n",
            "a fence's trailing blanks are code, so they stay",
        ),
        (
            "~~~\ncode \n~~~\na \n",
            "~~~\ncode \n~~~\na\n",
            "a tilde fence protects its own lines and nothing after them",
        ),
        (
            "```\ncode\r\n```\r\n",
            "```\ncode\n```\n",
            "a line ending is never content, so CRLF goes inside a fence too",
        ),
        (
            "```\na \n",
            "```\na \n",
            "an unclosed fence protects to the end of the document",
        ),
        (
            "```rust \nlet x = 1;\n```\n",
            "```rust \nlet x = 1;\n```\n",
            "the opener belongs to the fence, and `canonical_content` fingerprints it byte \
             for byte, so trimming it would move a card without changing what it teaches",
        ),
        (
            "```` \na \n````\n",
            "```` \na \n````\n",
            "which holds however the opener is spelled",
        ),
        (
            "a\r\r\n",
            "a\n",
            "a run of carriage returns is line-ending noise, and leaving one behind \
             would make the stamp write a file mixing both terminators",
        ),
        (
            "```\nlet x = 1;\r\r\n```\n",
            "```\nlet x = 1;\n```\n",
            "which holds inside a fence, where the line ending is still not content",
        ),
        (
            "a\u{000C}b\n",
            "ab\n",
            "a form feed has no rendered role in prose, so it goes",
        ),
        (
            "a\u{007F}b\n",
            "ab\n",
            "DEL is a control byte with no rendered role, so it goes",
        ),
        (
            "a\u{202D}b\n",
            "ab\n",
            "an LRO reversal override in prose goes",
        ),
        (
            "a\u{202E}b\n",
            "ab\n",
            "an RLO reversal override in prose goes for the same reason",
        ),
        (
            "a\u{feff}b\n",
            "ab\n",
            "a BOM interior to a line is paste damage, not a byte-order mark",
        ),
        (
            "## q\n\u{feff}a\n",
            "## q\na\n",
            "a BOM starting any later line is interior to the file and goes",
        ),
        (
            "a\rb\n",
            "ab\n",
            "a CR interior to a line is not a line ending and goes",
        ),
        (
            "a  \u{000C}\nb\n",
            "a  \nb\n",
            "dropping a trailing form feed exposes the hard break the spaces spell",
        ),
        (
            "a\u{200B}b\n",
            "a\u{200B}b\n",
            "a kept invisible (ZWSP here) has a rendered role and is not layer 1's business",
        ),
        (
            "```\na\u{000C}b\u{202E}c\n```\n",
            "```\na\u{000C}b\u{202E}c\n```\n",
            "layer 1 never enters a fence, whatever the byte",
        ),
    ];

    /// The answer half of a one-card deck, and why its bytes are a risk.
    const IDENTITY_ROWS: &[(&str, &str)] = &[
        (
            "```rust \nlet x = 1;\n```\n",
            "a fence opener reaches `back` verbatim and is fingerprinted byte for byte",
        ),
        ("~~~info \ncode\n~~~\n", "which holds for a tilde fence too"),
        ("```\ncode \n```\n", "as does a blank inside the fence body"),
        (
            "plain answer \nmore text\t\n",
            "a plain line is trimmed by the parser, so stripping it changes nothing",
        ),
        (
            "$$\nx^2 \n$$\n",
            "display math reaches `back` verbatim but is normalized before fingerprinting",
        ),
        (
            "answer  \n",
            "a hard break is kept, so it cannot move a card either",
        ),
    ];

    /// Normalization is a formatting change. A card whose fingerprint moves
    /// silently loses its cached distractors, notes, variants, and keypoints,
    /// and nothing reports it.
    #[test]
    fn normalizing_never_moves_a_card_fingerprint() {
        for (answer, why) in IDENTITY_ROWS {
            let deck = format!("---\ntitle: T\n---\n\n# S\n\n## q\n\n{answer}");
            let before = crate::parser::parse("deck.md", &deck).expect("the deck parses");
            let after = crate::parser::parse("deck.md", &normalize(&deck))
                .expect("the normalized deck parses");
            assert_eq!(
                before.cards[0].content_fingerprint, after.cards[0].content_fingerprint,
                "normalizing {answer:?} moved the card's fingerprint: {why}"
            );
        }
    }

    #[test]
    fn every_document_normalizes_to_its_canonical_bytes() {
        for (input, expected, why) in ROWS {
            assert_eq!(
                normalize(input),
                *expected,
                "normalizing {input:?} should give {expected:?}: {why}"
            );
        }
    }

    #[test]
    fn normalizing_canonical_bytes_borrows_them_and_changes_nothing() {
        for (_, expected, why) in ROWS {
            assert!(
                matches!(normalize(expected), Cow::Borrowed(_)),
                "already-normalized {expected:?} should be borrowed, not rewritten: {why}"
            );
        }
    }

    #[test]
    fn normalizing_a_fence_opener_preserves_card_identity_fingerprint_and_locator() {
        let text = "## code?\n```rust \nfn main() {}\n```\n<!-- at: code.rs:1 -->\n<!-- id: card-fence1 -->\n";
        let before = crate::parser::parse("deck.md", text).unwrap();
        let normalized = normalize(text);
        let after = crate::parser::parse("deck.md", &normalized).unwrap();

        assert_eq!(before.cards[0].id(), after.cards[0].id());
        assert_eq!(before.cards[0].citations, after.cards[0].citations);
        assert_eq!(
            before.cards[0].content_fingerprint, after.cards[0].content_fingerprint,
            "normalizing bytes must not reset tutor and remediation dedup"
        );
    }
}
