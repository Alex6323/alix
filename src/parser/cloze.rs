use super::{Lint, LintKind, ParseError, WHITESPACE, canonical::hash64, collapse, trim_ws};
use crate::store::HoleFingerprint;

// A NUL is safe here: the parser rejects C0 controls outside the whitespace
// set, so it can never occur in real card text.
const HOLE_MASK: &str = "\u{0}";

pub const BLANK: &str = "____";

pub const HIDDEN: &str = "[…]";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Seg {
    Text(String),
    Hole(String),
    Image { src: String, alt: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Region {
    // Test-only until front scanning wires in; an #[expect] would be
    // unfulfilled under cfg(test).
    #[allow(dead_code)]
    Front,
    Answer,
}

pub(super) fn hole_fingerprints(
    parsed: &[Vec<Seg>],
    holes: &[(usize, usize, &str)],
) -> Vec<HoleFingerprint> {
    holes
        .iter()
        .map(|(hole_line, hole_seg, text)| {
            let text_fp = hash64(&collapse(text));
            let mut line = String::new();
            for (si, segment) in parsed[*hole_line].iter().enumerate() {
                match segment {
                    Seg::Text(t) => line.push_str(t),
                    Seg::Hole(_) if si == *hole_seg => line.push_str(HOLE_MASK),
                    Seg::Hole(h) => line.push_str(h),
                    Seg::Image { src, alt } => push_image(&mut line, src, alt.as_deref()),
                }
            }
            HoleFingerprint {
                text_fp,
                line_fp: hash64(&collapse(&line)),
            }
        })
        .collect()
}

pub(super) fn scan_markers(
    line_text: &str,
    lineno: usize,
    region: Region,
    lints: &mut Vec<Lint>,
) -> Result<Vec<Seg>, ParseError> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut rest = line_text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("\\\\cloze") {
            text.push_str("\\cloze");
            rest = after;
        } else if let Some(after) = rest.strip_prefix("\\![") {
            text.push_str("![");
            rest = after;
        } else if region == Region::Answer
            && let Some(after) = rest.strip_prefix("\\cloze")
        {
            if let Some(arg) = after.strip_prefix('{') {
                let (content, after_hole) = scan_group(arg, lineno)?;
                if trim_ws(&content).is_empty() {
                    return Err(ParseError::EmptyHole(lineno));
                }
                if content.contains("\\cloze") {
                    // Hole content is never re-scanned; the inner marker is
                    // literal text.
                    lints.push(Lint {
                        line: lineno,
                        kind: LintKind::ClozeInHole,
                    });
                }
                if !text.is_empty() {
                    segments.push(Seg::Text(std::mem::take(&mut text)));
                }
                segments.push(Seg::Hole(content));
                rest = after_hole;
            } else if after.starts_with('[') {
                return Err(ParseError::ClozeBracketReserved(lineno));
            } else {
                text.push_str("\\cloze");
                rest = after;
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
        && let Some((raw_src, after)) = paren.split_once(')')
    {
        let src = trim_ws(raw_src);
        if src.is_empty() {
            lints.push(image_malformed(lineno));
            return after;
        }
        // Inner whitespace in the src (a Markdown title, a spaced filename) is
        // a deliberate exclusion, kept open for a later release.
        if !src.contains(&WHITESPACE[..]) {
            if !text.is_empty() {
                segments.push(Seg::Text(std::mem::take(text)));
            }
            let alt = trim_ws(raw_alt);
            segments.push(Seg::Image {
                src: src.to_string(),
                alt: (!alt.is_empty()).then(|| alt.to_string()),
            });
            return after;
        }
    }
    lints.push(image_malformed(lineno));
    text.push_str("![");
    inner
}

fn image_malformed(lineno: usize) -> Lint {
    Lint {
        line: lineno,
        kind: LintKind::ImageMalformed,
    }
}

fn scan_group(arg: &str, lineno: usize) -> Result<(String, &str), ParseError> {
    let mut content = String::new();
    let mut depth = 1usize;
    let mut rest = arg;
    while let Some(ch) = rest.chars().next() {
        match ch {
            '\\' => {
                let after = &rest[1..];
                if let Some(escaped) = after
                    .chars()
                    .next()
                    .filter(|c| matches!(c, '{' | '}' | '\\'))
                {
                    content.push(escaped);
                    rest = &after[escaped.len_utf8()..];
                } else {
                    content.push('\\');
                    rest = after;
                }
            }
            '{' => {
                depth += 1;
                content.push('{');
                rest = &rest[1..];
            }
            '}' => {
                depth -= 1;
                rest = &rest[1..];
                if depth == 0 {
                    return Ok((content, rest));
                }
                content.push('}');
            }
            _ => {
                content.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    Err(ParseError::UnclosedHole(lineno))
}

// A stable hash preimage (feeds line_fp), not deck syntax; changing it would
// churn stored hole fingerprints.
pub(super) fn push_image(out: &mut String, src: &str, alt: Option<&str>) {
    out.push_str("\\image{");
    out.push_str(src);
    out.push('}');
    if let Some(alt) = alt {
        out.push_str("{alt: ");
        out.push_str(alt);
        out.push('}');
    }
}

pub(super) fn seg_display(segments: &[Seg]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            Seg::Text(text) => out.push_str(text),
            Seg::Hole(hole) => {
                out.push_str("\\cloze{");
                out.push_str(hole);
                out.push('}');
            }
            Seg::Image { .. } => {}
        }
    }
    out
}

pub(super) fn hash_repr(segments: &[Seg]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            Seg::Text(text) => out.push_str(text),
            Seg::Hole(hole) => {
                out.push('\u{1f}');
                out.push_str(hole);
                out.push('\u{1f}');
            }
            Seg::Image { src, alt } => {
                out.push('\u{1f}');
                out.push_str("image");
                out.push('\u{1f}');
                out.push_str(src);
                if let Some(alt) = alt {
                    out.push('\u{1f}');
                    out.push_str(alt);
                }
                out.push('\u{1f}');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(line: &str, region: Region) -> (Vec<Seg>, Vec<Lint>) {
        let mut lints = Vec::new();
        let segments = scan_markers(line, 7, region, &mut lints).unwrap();
        (segments, lints)
    }

    fn answer(line: &str) -> (Vec<Seg>, Vec<Lint>) {
        scan(line, Region::Answer)
    }

    fn fatal(line: &str) -> ParseError {
        let mut lints = Vec::new();
        scan_markers(line, 7, Region::Answer, &mut lints).unwrap_err()
    }

    fn text(t: &str) -> Seg {
        Seg::Text(t.into())
    }

    fn hole(h: &str) -> Seg {
        Seg::Hole(h.into())
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
    fn an_image_is_recognized_in_the_front_region_too() {
        let (segments, _) = scan("![](x.png)", Region::Front);
        assert_eq!(vec![image("x.png", None)], segments);
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
    fn a_markdown_image_can_share_a_line_with_a_hole() {
        let (segments, _) = answer("\\cloze{a} and ![](x.png)");
        assert_eq!(
            vec![hole("a"), text(" and "), image("x.png", None)],
            segments
        );
    }

    #[test]
    fn unknown_backslash_command_stays_literal() {
        let (segments, lints) = answer("\\frac{1}{2}");
        assert_eq!(vec![text("\\frac{1}{2}")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn the_retired_backslash_markers_are_now_plain_text() {
        for line in ["\\image{moon.png}", "\\audio{x.mp3}", "\\video{x.mp4}"] {
            let (segments, lints) = answer(line);
            assert_eq!(vec![text(line)], segments);
            assert!(lints.is_empty());
        }
    }

    #[test]
    fn cloze_hole_in_the_answer_region_is_unchanged() {
        let (segments, lints) = answer("\\cloze{mut}");
        assert_eq!(vec![hole("mut")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn a_second_group_after_a_cloze_hole_stays_literal() {
        let (segments, _) = answer("\\cloze{a}{b: c}");
        assert_eq!(vec![hole("a"), text("{b: c}")], segments);
    }

    #[test]
    fn empty_cloze_hole_stays_fatal() {
        assert_eq!(ParseError::EmptyHole(7), fatal("\\cloze{}"));
    }

    #[test]
    fn unclosed_cloze_hole_stays_fatal() {
        assert_eq!(ParseError::UnclosedHole(7), fatal("\\cloze{oops"));
    }

    #[test]
    fn cloze_bracket_stays_reserved_in_the_answer_region() {
        assert_eq!(ParseError::ClozeBracketReserved(7), fatal("\\cloze[pin]"));
    }

    #[test]
    fn cloze_in_the_front_region_stays_literal() {
        let (segments, lints) = scan("\\cloze{mut}", Region::Front);
        assert_eq!(vec![text("\\cloze{mut}")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn cloze_bracket_in_the_front_region_stays_literal() {
        let (segments, _) = scan("\\cloze[pin]", Region::Front);
        assert_eq!(vec![text("\\cloze[pin]")], segments);
    }

    #[test]
    fn escaped_cloze_unescapes_in_the_front_region_too() {
        let (segments, _) = scan("\\\\cloze{x}", Region::Front);
        assert_eq!(vec![text("\\cloze{x}")], segments);
    }

    #[test]
    fn hash_repr_wraps_an_image_in_sentinels() {
        let (segments, _) = answer("![](m.png)");
        assert_eq!("\u{1f}image\u{1f}m.png\u{1f}", hash_repr(&segments));
    }

    #[test]
    fn hash_repr_image_does_not_collide_with_the_escaped_literal_text() {
        let image_segments = vec![Seg::Image {
            src: "x".into(),
            alt: None,
        }];
        let literal_segments = vec![Seg::Text("![](x)".into())];
        assert_ne!(hash_repr(&image_segments), hash_repr(&literal_segments));
    }

    #[test]
    fn hash_repr_image_does_not_collide_with_a_hole_that_mentions_image() {
        let image_segments = vec![Seg::Image {
            src: "x".into(),
            alt: None,
        }];
        let hole_segments = vec![Seg::Hole("image x".into())];
        assert_ne!(hash_repr(&image_segments), hash_repr(&hole_segments));
    }

    #[test]
    fn hole_fingerprints_see_an_image_on_the_hole_line() {
        let holes = vec![(0usize, 0usize, "a")];
        let (with_image, _) = answer("\\cloze{a} ![](x.png)");
        let (without_image, _) = answer("\\cloze{a}");
        let with_image = hole_fingerprints(&[with_image], &holes);
        let without_image = hole_fingerprints(&[without_image], &holes);
        assert_ne!(with_image[0].line_fp, without_image[0].line_fp);
    }
}
