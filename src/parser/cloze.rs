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
        && let Some((src, after)) = scan_src(paren)
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

pub(super) fn scan_src(paren: &str) -> Option<(String, &str)> {
    match paren.strip_prefix('<') {
        Some(bracketed) => bracketed_src(bracketed),
        None => unbracketed_src(paren),
    }
}

fn bracketed_src(arg: &str) -> Option<(String, &str)> {
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
            '>' => return rest[1..].strip_prefix(')').map(|after| (src, after)),
            _ => {
                src.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    None
}

fn unbracketed_src(arg: &str) -> Option<(String, &str)> {
    let mut src = String::new();
    let mut depth = 1usize;
    let mut rest = arg;
    while let Some(ch) = rest.chars().next() {
        match ch {
            '\\' => {
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
                depth += 1;
                src.push('(');
                rest = &rest[1..];
            }
            ')' => {
                depth -= 1;
                rest = &rest[1..];
                if depth == 0 {
                    let trimmed = trim_ws(&src);
                    // Inner whitespace would start a Markdown title
                    // (unsupported); the bracketed <src> form carries spaces.
                    if trimmed.contains(&WHITESPACE[..]) {
                        return None;
                    }
                    return Some((trimmed.to_string(), rest));
                }
                src.push(')');
            }
            _ => {
                src.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    None
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
    fn a_markdown_title_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("![a](moon.png \"the moon\")");
        assert_eq!(vec![text("![a](moon.png \"the moon\")")], segments);
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
}
