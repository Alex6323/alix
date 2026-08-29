use std::borrow::Cow;

use super::{closes_fence, fence_opener};

/// A deck file's canonical bytes: no leading BOM, LF line endings, and,
/// outside a code fence, no trailing blanks except the two spaces of a hard
/// line break. A fence's own lines keep their trailing blanks, which are code.
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
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        changed |= line.len() != raw.len();
        match fence {
            Some((ch, open)) => {
                out.push_str(line);
                if closes_fence(line, ch, open) {
                    fence = None;
                }
            }
            None => {
                let (kept, hard_break) = split_trailing(line);
                out.push_str(kept);
                if hard_break {
                    out.push_str("  ");
                }
                changed |= line.len() != kept.len() + if hard_break { 2 } else { 0 };
                fence = fence_opener(line);
            }
        }
    }
    changed.then_some(out)
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
            "```` \na \n````\n",
            "````\na \n````\n",
            "the opening line is prose, so it is trimmed before the fence takes over",
        ),
    ];

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
}
