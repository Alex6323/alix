use super::{Lint, LintKind, ParseError, WHITESPACE, trim_ws};

// Reserved codepoints, not prose: the parser rejects them in authored
// text, so a mask signal can never be counterfeited and literal ____ and
// [...] stay ordinary prose everywhere.
pub const BLANK: &str = "⍰";

pub const HIDDEN: &str = "⬚";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Seg {
    Text(String),
    Image { src: String, alt: Option<String> },
}

pub(super) fn scan_markers(
    line_text: &str,
    lineno: usize,
    lints: &mut Vec<Lint>,
) -> Result<Vec<Seg>, ParseError> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut rest = line_text;
    while !rest.is_empty() {
        if rest.starts_with('\\') && rest.trim_start_matches('\\').starts_with("![") {
            let run = rest.chars().take_while(|ch| *ch == '\\').count();
            let escaped = run % 2 == 1;
            for _ in 0..run - usize::from(escaped) {
                text.push('\\');
            }
            if escaped {
                text.push_str("![");
                rest = &rest[run + 2..];
            } else {
                rest = scan_image(&rest[run + 2..], lineno, &mut text, &mut segments, lints);
            }
        } else if let Some(after) = rest.strip_prefix("![") {
            rest = scan_image(after, lineno, &mut text, &mut segments, lints);
        } else if let Some(ch) = rest.chars().next() {
            text.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    if !text.is_empty() {
        segments.push(Seg::Text(text));
    }
    Ok(segments)
}

fn scan_image<'a>(
    inner: &'a str,
    lineno: usize,
    text: &mut String,
    segments: &mut Vec<Seg>,
    lints: &mut Vec<Lint>,
) -> &'a str {
    if let Some((raw_alt, after_alt)) = inner.split_once(']')
        && let Some(paren) = after_alt.strip_prefix('(')
        && let Some((src, _, after)) = scan_src(paren)
    {
        if src.is_empty() {
            lints.push(image_malformed(lineno));
            return after;
        }
        if !text.is_empty() {
            segments.push(Seg::Text(std::mem::take(text)));
        }
        let alt = trim_ws(raw_alt);
        segments.push(Seg::Image {
            src,
            alt: (!alt.is_empty()).then(|| alt.to_string()),
        });
        return after;
    }
    lints.push(image_malformed(lineno));
    text.push_str("![");
    inner
}

// The range is the destination's authored bytes within `paren`, the span
// asset rewriting replaces; the decoded source and the range differ
// whenever escapes or delimiters are involved.
pub(super) fn scan_src(paren: &str) -> Option<(String, std::ops::Range<usize>, &str)> {
    match paren.strip_prefix('<') {
        Some(bracketed) => {
            bracketed_src(bracketed).map(|(src, interior, after)| (src, 1..1 + interior, after))
        }
        None => unbracketed_src(paren),
    }
}

fn bracketed_src(arg: &str) -> Option<(String, usize, &str)> {
    let mut src = String::new();
    let mut rest = arg;
    while let Some(ch) = rest.chars().next() {
        match ch {
            '\\' => {
                let after = &rest[1..];
                if let Some(escaped) = after
                    .chars()
                    .next()
                    .filter(|c| matches!(c, '<' | '>' | '\\'))
                {
                    src.push(escaped);
                    rest = &after[escaped.len_utf8()..];
                } else {
                    src.push('\\');
                    rest = after;
                }
            }
            '<' | '\n' => return None,
            '>' => {
                let interior = arg.len() - rest.len();
                return title_tail(&rest[1..]).map(|after| (src, interior, after));
            }
            _ => {
                src.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    None
}

fn unbracketed_src(arg: &str) -> Option<(String, std::ops::Range<usize>, &str)> {
    let mut src = String::new();
    let mut depth = 1usize;
    let mut rest = arg;
    let mut content_start = None;
    while let Some(ch) = rest.chars().next() {
        match ch {
            '\\' => {
                content_start.get_or_insert(arg.len() - rest.len());
                let after = &rest[1..];
                if let Some(escaped) = after
                    .chars()
                    .next()
                    .filter(|c| matches!(c, '(' | ')' | '\\'))
                {
                    src.push(escaped);
                    rest = &after[escaped.len_utf8()..];
                } else {
                    src.push('\\');
                    rest = after;
                }
            }
            '(' => {
                content_start.get_or_insert(arg.len() - rest.len());
                depth += 1;
                src.push('(');
                rest = &rest[1..];
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let end = arg.len() - rest.len();
                    return Some((src, content_start.unwrap_or(end)..end, &rest[1..]));
                }
                content_start.get_or_insert(arg.len() - rest.len());
                src.push(')');
                rest = &rest[1..];
            }
            ch if WHITESPACE.contains(&ch) => {
                if src.is_empty() {
                    rest = &rest[ch.len_utf8()..];
                } else {
                    if depth != 1 {
                        return None;
                    }
                    let span = content_start.unwrap_or(0)..arg.len() - rest.len();
                    return title_tail(rest).map(|after| (src, span, after));
                }
            }
            _ => {
                content_start.get_or_insert(arg.len() - rest.len());
                src.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    None
}

// Mirrors `inline::link_tail_end`'s GFM title grammar; the tail-agreement
// law test keeps the two copies from drifting apart.
fn title_tail(rest: &str) -> Option<&str> {
    let after_ws = rest.trim_start_matches(|c: char| WHITESPACE.contains(&c));
    let separated = after_ws.len() != rest.len();
    match after_ws.chars().next() {
        Some(')') => Some(&after_ws[1..]),
        Some(open @ ('"' | '\'' | '(')) if separated => {
            let closer = if open == '(' { ')' } else { open };
            let mut inner = &after_ws[1..];
            loop {
                let ch = inner.chars().next()?;
                if ch == '\\' {
                    inner = &inner[1..];
                    if let Some(escaped) = inner.chars().next() {
                        inner = &inner[escaped.len_utf8()..];
                    }
                    continue;
                }
                if ch == closer {
                    inner = &inner[1..];
                    break;
                }
                if open == '(' && ch == '(' {
                    return None;
                }
                inner = &inner[ch.len_utf8()..];
            }
            let after_title = inner.trim_start_matches(|c: char| WHITESPACE.contains(&c));
            after_title.strip_prefix(')')
        }
        _ => None,
    }
}

fn image_malformed(lineno: usize) -> Lint {
    Lint {
        line: lineno,
        kind: LintKind::ImageMalformed,
    }
}

pub(super) fn seg_display(segments: &[Seg]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            Seg::Text(text) => out.push_str(text),
            Seg::Image { .. } => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(line: &str) -> (Vec<Seg>, Vec<Lint>) {
        let mut lints = Vec::new();
        let segments = scan_markers(line, 7, &mut lints).unwrap();
        (segments, lints)
    }

    fn text(t: &str) -> Seg {
        Seg::Text(t.into())
    }

    fn image(src: &str, alt: Option<&str>) -> Seg {
        Seg::Image {
            src: src.into(),
            alt: alt.map(Into::into),
        }
    }

    fn image_malformed() -> Lint {
        Lint {
            line: 7,
            kind: LintKind::ImageMalformed,
        }
    }

    #[test]
    fn a_markdown_image_yields_src_and_alt() {
        let (segments, lints) = answer("![a moon](moon.png)");
        assert_eq!(vec![image("moon.png", Some("a moon"))], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn an_empty_bracket_yields_no_alt() {
        let (segments, lints) = answer("![](moon.png)");
        assert_eq!(vec![image("moon.png", None)], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn the_image_src_is_trimmed() {
        let (segments, _) = answer("![](  moon.png  )");
        assert_eq!(vec![image("moon.png", None)], segments);
    }

    #[test]
    fn the_image_alt_is_trimmed() {
        let (segments, _) = answer("![ a moon ](x.png)");
        assert_eq!(vec![image("x.png", Some("a moon"))], segments);
    }

    #[test]
    fn a_whitespace_only_alt_counts_as_no_alt() {
        let (segments, _) = answer("![   ](x.png)");
        assert_eq!(vec![image("x.png", None)], segments);
    }

    #[test]
    fn text_around_an_image_is_preserved() {
        let (segments, _) = answer("see ![](x.png) here");
        assert_eq!(
            vec![text("see "), image("x.png", None), text(" here")],
            segments
        );
    }

    #[test]
    fn an_image_needs_no_word_boundary() {
        let (segments, _) = answer("wow![](x.png)");
        assert_eq!(vec![text("wow"), image("x.png", None)], segments);
    }

    #[test]
    fn two_images_yield_in_order() {
        let (segments, _) = answer("![](a.png) ![](b.png)");
        assert_eq!(
            vec![image("a.png", None), text(" "), image("b.png", None)],
            segments
        );
    }

    #[test]
    fn the_image_tail_and_the_link_tail_accept_the_same_gfm_tails() {
        let rows = [
            ("x.png)", true, "a bare destination"),
            ("x.png )", true, "trailing whitespace before the close"),
            ("x.png \"t\")", true, "a double-quoted title"),
            ("x.png 't')", true, "a single-quoted title"),
            ("x.png (t))", true, "a paren title"),
            ("x.png \"a) b\")", true, "a close paren inside a title"),
            (
                "x.png \"say \\\"hi\\\"\")",
                true,
                "an escaped quote in a title",
            ),
            ("<a b> \"t\")", true, "a bracketed destination plus title"),
            (
                "x.png\"t\")",
                true,
                "no separation: the quote is destination content",
            ),
            ("x.png \"t\" x)", false, "junk after a closed title"),
            (
                "x.png (t(t))",
                false,
                "a nested open paren in a paren title",
            ),
            ("x.png \"t", false, "an unclosed title"),
            ("a b.png)", false, "unbracketed whitespace with no title"),
        ];
        for (tail, accepted, why) in rows {
            let (segments, _) = answer(&format!("![a]({tail}"));
            let image = segments
                .iter()
                .any(|segment| matches!(segment, Seg::Image { .. }));
            assert_eq!(accepted, image, "image tail, {why}: {tail}");
            let flat: String = crate::inline::parse_inline(&format!("[a]({tail}"))
                .iter()
                .map(|run| run.text.as_str())
                .collect();
            assert_eq!(accepted, flat == "a", "link tail, {why}: {tail}");
        }
    }

    #[test]
    fn every_image_title_tail_preserves_its_separator_and_escape_contract() {
        for (tail, expected, case) in [
            (")", Some(""), "a destination without a title"),
            (" \"title\")", Some(""), "a separated title"),
            ("\"title\")", None, "an unseparated quote"),
            (
                " \"say \\\"hi\\\"\")",
                Some(""),
                "escaped ascii title delimiters",
            ),
            (
                " \"say \\é now\")",
                Some(""),
                "an escaped multibyte character",
            ),
            (" \"trailing \\", None, "a trailing escape"),
        ] {
            assert_eq!(expected, title_tail(tail), "{case}: {tail:?}");
        }
    }

    #[test]
    fn a_double_quoted_title_parses_and_is_dropped() {
        let (segments, lints) = answer("![a moon](moon.png \"The Moon\")");
        assert_eq!(vec![image("moon.png", Some("a moon"))], segments);
        assert!(lints.is_empty(), "{lints:?}");
    }

    #[test]
    fn single_quoted_and_paren_titles_parse_like_the_link_tail() {
        let (segments, lints) = answer("![](a.png 'one') ![](b.png (two))");
        assert_eq!(
            vec![image("a.png", None), text(" "), image("b.png", None)],
            segments
        );
        assert!(lints.is_empty(), "{lints:?}");
    }

    #[test]
    fn a_bracketed_destination_carries_spaces_and_a_title() {
        let (segments, lints) = answer("![m](<my file.png> \"The Moon\")");
        assert_eq!(vec![image("my file.png", Some("m"))], segments);
        assert!(lints.is_empty(), "{lints:?}");
    }

    #[test]
    fn an_escaped_quote_stays_inside_its_title() {
        let (segments, lints) = answer("![a](x.png \"say \\\"hi\\\"\")");
        assert_eq!(vec![image("x.png", Some("a"))], segments);
        assert!(lints.is_empty(), "{lints:?}");
    }

    #[test]
    fn an_unseparated_quote_is_destination_content_not_a_title() {
        let (segments, lints) = answer("![a](x.png\"t\")");
        assert_eq!(vec![image("x.png\"t\"", Some("a"))], segments);
        assert!(lints.is_empty(), "{lints:?}");
    }

    #[test]
    fn an_unclosed_title_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![a](x.png \"t");
        assert_eq!(vec![text("![a](x.png \"t")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn junk_after_a_closed_title_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![a](x.png \"t\" x)");
        assert_eq!(vec![text("![a](x.png \"t\" x)")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn a_paren_title_rejects_a_nested_open_paren() {
        let (segments, lints) = answer("![a](x.png (t(t))");
        assert_eq!(vec![text("![a](x.png (t(t))")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn an_unclosed_paren_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![alt](moon.png");
        assert_eq!(vec![text("![alt](moon.png")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn a_bracket_without_parens_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![alt]");
        assert_eq!(vec![text("![alt]")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn an_unclosed_bracket_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![oops");
        assert_eq!(vec![text("![oops")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn a_space_between_bracket_and_parens_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![alt] (x.png)");
        assert_eq!(vec![text("![alt] (x.png)")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn an_empty_src_lints_and_drops_the_image() {
        let (segments, lints) = answer("![alt]()");
        assert!(segments.is_empty());
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn a_whitespace_only_src_counts_as_empty() {
        let (segments, lints) = answer("![](  )");
        assert!(segments.is_empty());
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn a_src_with_inner_whitespace_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![](my moon.png)");
        assert_eq!(vec![text("![](my moon.png)")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn balanced_inner_parens_stay_in_the_src() {
        let (segments, lints) = answer("![](a(b)c.png)");
        assert_eq!(vec![image("a(b)c.png", None)], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn real_filenames_with_parens_parse_to_their_exact_srcs() {
        for name in [
            "Bremen_Wappen(Klein).svg.png",
            "Coat_of_arms_of_Mecklenburg-Western_Pomerania_(small).svg.png",
        ] {
            let (segments, lints) = answer(&format!("![]({name})"));
            assert_eq!(vec![image(name, None)], segments);
            assert!(lints.is_empty());
        }
    }

    #[test]
    fn a_non_ascii_filename_parses() {
        let name = "Lesser_coat_of_arms_of_Baden-Württemberg.svg.png";
        let (segments, lints) = answer(&format!("![]({name})"));
        assert_eq!(vec![image(name, None)], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn a_bracketed_src_keeps_a_space_verbatim() {
        let (segments, lints) = answer("![](<a b.png>)");
        assert_eq!(vec![image("a b.png", None)], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn a_bracketed_src_keeps_parens_verbatim() {
        let (segments, lints) = answer("![](<a(b).png>)");
        assert_eq!(vec![image("a(b).png", None)], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn escaped_parens_in_the_src_are_literal_and_do_not_count_for_depth() {
        let (segments, lints) = answer("![](a\\(b\\).png)");
        assert_eq!(vec![image("a(b).png", None)], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn unbalanced_parens_in_the_src_degrade_to_literal_with_a_lint() {
        let (segments, lints) = answer("![](a(b.png)");
        assert_eq!(vec![text("![](a(b.png)")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn an_unclosed_angle_bracket_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![](<a b.png)");
        assert_eq!(vec![text("![](<a b.png)")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn a_reference_style_image_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![alt][ref]");
        assert_eq!(vec![text("![alt][ref]")], segments);
        assert_eq!(vec![image_malformed()], lints);
    }

    #[test]
    fn a_trailing_option_map_after_an_image_is_literal_text() {
        let (segments, lints) = answer("![](x.png){crop: 10,20}");
        assert_eq!(vec![image("x.png", None), text("{crop: 10,20}")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn an_escaped_image_start_is_literal() {
        let (segments, lints) = answer("\\![alt](x)");
        assert_eq!(vec![text("![alt](x)")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn unknown_backslash_command_stays_literal() {
        let (segments, lints) = answer("\\frac{1}{2}");
        assert_eq!(vec![text("\\frac{1}{2}")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn a_blank_command_stays_literal() {
        let (segments, lints) = answer("\\blank{mut}");
        assert_eq!(vec![text("\\blank{mut}")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn every_backslash_run_preserves_its_image_marker_parity() {
        for run in 1..=4 {
            let prefix = "\\".repeat(run);
            let input = format!("{prefix}![a](x.png)");
            let (segments, lints) = answer(&input);
            let expected = if run % 2 == 1 {
                vec![text(&format!("{}![a](x.png)", "\\".repeat(run - 1)))]
            } else {
                vec![text(&prefix), image("x.png", Some("a"))]
            };
            assert_eq!(expected, segments, "run of {run} backslashes");
            assert!(lints.is_empty(), "run of {run} backslashes: {lints:?}");
        }
    }

    #[test]
    fn image_source_scanning_preserves_authored_ranges_and_rejects_invalid_brackets() {
        for (input, src, range, after) in [
            ("<a b.png>)tail", "a b.png", 1..8, "tail"),
            ("a\\(b\\).png)tail", "a(b).png", 0..10, "tail"),
            ("a(b)c.png)tail", "a(b)c.png", 0..9, "tail"),
            ("  \\(b.png)tail", "(b.png", 2..9, "tail"),
            ("  (a)b.png)tail", "(a)b.png", 2..10, "tail"),
        ] {
            assert_eq!(
                Some((src.to_string(), range, after)),
                scan_src(input),
                "{input}"
            );
        }
        for input in ["<a<b>)", "<a\nb>)"] {
            assert_eq!(None, scan_src(input), "{input:?}");
        }
    }
}
