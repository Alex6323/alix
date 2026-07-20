use super::{Lint, LintKind, ParseError, canonical::hash64, collapse, trim_ws};
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
        } else if let Some(after) = rest.strip_prefix("\\\\image") {
            text.push_str("\\image");
            rest = after;
        } else if let Some(after) = rest.strip_prefix("\\\\audio") {
            text.push_str("\\audio");
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
        } else if let Some(after) = rest.strip_prefix("\\image") {
            rest = scan_image(rest, after, lineno, &mut text, &mut segments, lints);
        } else if let Some(after) = rest.strip_prefix("\\audio") {
            rest = scan_audio(rest, after, lineno, &mut text, lints);
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
    start: &'a str,
    after_name: &'a str,
    lineno: usize,
    text: &mut String,
    segments: &mut Vec<Seg>,
    lints: &mut Vec<Lint>,
) -> &'a str {
    let Some(arg) = after_name.strip_prefix('{') else {
        text.push_str("\\image");
        return after_name;
    };
    let Ok((content, mut rest)) = scan_group(arg, lineno) else {
        malformed(lints, lineno, "image");
        text.push_str(start);
        return "";
    };
    let src = trim_ws(&content);
    let dropped = src.is_empty();
    if dropped {
        malformed(lints, lineno, "image");
    }
    let mut alt: Option<String> = None;
    while let Some(arg) = rest.strip_prefix('{') {
        let Ok((group, after_group)) = scan_group(arg, lineno) else {
            malformed(lints, lineno, "image");
            break;
        };
        rest = after_group;
        if dropped {
            continue;
        }
        let Some((raw_key, raw_value)) = group.split_once(':') else {
            bad_option(lints, lineno, trim_ws(&group));
            continue;
        };
        let key = trim_ws(raw_key);
        if key == "alt" && alt.is_none() {
            alt = Some(trim_ws(raw_value).to_string());
        } else {
            bad_option(lints, lineno, key);
        }
    }
    if !dropped {
        if !text.is_empty() {
            segments.push(Seg::Text(std::mem::take(text)));
        }
        segments.push(Seg::Image {
            src: src.to_string(),
            alt,
        });
    }
    rest
}

fn scan_audio<'a>(
    start: &'a str,
    after_name: &'a str,
    lineno: usize,
    text: &mut String,
    lints: &mut Vec<Lint>,
) -> &'a str {
    if !after_name.starts_with('{') {
        text.push_str("\\audio");
        return after_name;
    }
    lints.push(Lint {
        line: lineno,
        kind: LintKind::AudioNotSupported,
    });
    let mut rest = after_name;
    let mut primary = true;
    while let Some(arg) = rest.strip_prefix('{') {
        match scan_group(arg, lineno) {
            Ok((content, after_group)) => {
                if primary && trim_ws(&content).is_empty() {
                    malformed(lints, lineno, "audio");
                }
                primary = false;
                rest = after_group;
            }
            Err(_) => {
                malformed(lints, lineno, "audio");
                rest = "";
            }
        }
    }
    text.push_str(&start[..start.len() - rest.len()]);
    rest
}

fn malformed(lints: &mut Vec<Lint>, lineno: usize, name: &str) {
    lints.push(Lint {
        line: lineno,
        kind: LintKind::MarkerMalformed { name: name.into() },
    });
}

fn bad_option(lints: &mut Vec<Lint>, lineno: usize, key: &str) {
    lints.push(Lint {
        line: lineno,
        kind: LintKind::MarkerBadOption { key: key.into() },
    });
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

// The round-trip reconstruction renderer; display paths use seg_display, so only tests reach it.
#[allow(dead_code)]
pub(super) fn seg_text(segments: &[Seg]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            Seg::Text(text) => out.push_str(text),
            Seg::Hole(hole) => {
                out.push_str("\\cloze{");
                out.push_str(hole);
                out.push('}');
            }
            Seg::Image { src, alt } => push_image(&mut out, src, alt.as_deref()),
        }
    }
    out
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

    fn bad_option_lint(key: &str) -> Lint {
        Lint {
            line: 7,
            kind: LintKind::MarkerBadOption { key: key.into() },
        }
    }

    fn malformed_lint(name: &str) -> Lint {
        Lint {
            line: 7,
            kind: LintKind::MarkerMalformed { name: name.into() },
        }
    }

    fn audio_unsupported() -> Lint {
        Lint {
            line: 7,
            kind: LintKind::AudioNotSupported,
        }
    }

    #[test]
    fn image_with_one_group_yields_src_and_no_alt() {
        let (segments, lints) = answer("\\image{moon.png}");
        assert_eq!(vec![image("moon.png", None)], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn image_alt_option_is_captured() {
        let (segments, lints) = answer("\\image{moon.png}{alt: a moon}");
        assert_eq!(vec![image("moon.png", Some("a moon"))], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn image_src_is_trimmed() {
        let (segments, _) = answer("\\image{ moon.png }");
        assert_eq!(vec![image("moon.png", None)], segments);
    }

    #[test]
    fn option_value_splits_on_the_first_colon_only() {
        let (segments, _) = answer("\\image{x.png}{alt: 3:2 ratio}");
        assert_eq!(vec![image("x.png", Some("3:2 ratio"))], segments);
    }

    #[test]
    fn option_value_unescapes_braces() {
        let (segments, _) = answer("\\image{x.png}{alt: a \\{b\\} c}");
        assert_eq!(vec![image("x.png", Some("a {b} c"))], segments);
    }

    #[test]
    fn escaped_image_marker_stays_literal() {
        let (segments, lints) = answer("\\\\image{x}");
        assert_eq!(vec![text("\\image{x}")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn image_without_a_group_is_literal() {
        let (segments, lints) = answer("\\image");
        assert_eq!(vec![text("\\image")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn separated_group_is_literal_text_not_an_option() {
        let (segments, lints) = answer("\\image{a.png} {alt: x}");
        assert_eq!(vec![image("a.png", None), text(" {alt: x}")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn option_without_a_colon_lints_and_the_image_still_yields() {
        let (segments, lints) = answer("\\image{x.png}{center}");
        assert_eq!(vec![image("x.png", None)], segments);
        assert_eq!(vec![bad_option_lint("center")], lints);
    }

    #[test]
    fn empty_option_group_lints() {
        let (segments, lints) = answer("\\image{x.png}{}");
        assert_eq!(vec![image("x.png", None)], segments);
        assert_eq!(vec![bad_option_lint("")], lints);
    }

    #[test]
    fn unknown_option_key_lints() {
        let (segments, lints) = answer("\\image{x.png}{title: t}");
        assert_eq!(vec![image("x.png", None)], segments);
        assert_eq!(vec![bad_option_lint("title")], lints);
    }

    #[test]
    fn duplicate_option_key_lints_and_the_first_value_wins() {
        let (segments, lints) = answer("\\image{x.png}{alt: a}{alt: b}");
        assert_eq!(vec![image("x.png", Some("a"))], segments);
        assert_eq!(vec![bad_option_lint("alt")], lints);
    }

    #[test]
    fn text_around_a_marker_is_preserved() {
        let (segments, _) = answer("see \\image{x} here");
        assert_eq!(
            vec![text("see "), image("x", None), text(" here")],
            segments
        );
    }

    #[test]
    fn unclosed_image_degrades_to_literal_with_a_lint() {
        let (segments, lints) = answer("\\image{oops");
        assert_eq!(vec![text("\\image{oops")], segments);
        assert_eq!(vec![malformed_lint("image")], lints);
    }

    #[test]
    fn empty_image_is_dropped_with_a_lint() {
        let (segments, lints) = answer("\\image{}");
        assert!(segments.is_empty());
        assert_eq!(vec![malformed_lint("image")], lints);
    }

    #[test]
    fn whitespace_only_image_src_counts_as_empty() {
        let (segments, lints) = answer("\\image{  }");
        assert!(segments.is_empty());
        assert_eq!(vec![malformed_lint("image")], lints);
    }

    #[test]
    fn a_dropped_image_consumes_its_option_groups() {
        let (segments, lints) = answer("\\image{}{alt: x}");
        assert!(segments.is_empty());
        assert_eq!(vec![malformed_lint("image")], lints);
    }

    #[test]
    fn audio_lints_unsupported_and_stays_literal() {
        let (segments, lints) = answer("\\audio{x.mp3}");
        assert_eq!(vec![text("\\audio{x.mp3}")], segments);
        assert_eq!(vec![audio_unsupported()], lints);
    }

    #[test]
    fn audio_claims_its_groups_without_validating_options() {
        let (segments, lints) = answer("\\audio{a.mp3}{from: 0:10}");
        assert_eq!(vec![text("\\audio{a.mp3}{from: 0:10}")], segments);
        assert_eq!(vec![audio_unsupported()], lints);
    }

    #[test]
    fn audio_without_a_group_is_literal_without_a_lint() {
        let (segments, lints) = answer("\\audio");
        assert_eq!(vec![text("\\audio")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn unclosed_audio_stays_literal_and_lints_malformed_too() {
        let (segments, lints) = answer("\\audio{oops");
        assert_eq!(vec![text("\\audio{oops")], segments);
        assert_eq!(vec![audio_unsupported(), malformed_lint("audio")], lints);
    }

    #[test]
    fn empty_audio_lints_unsupported_and_malformed_too() {
        let (segments, lints) = answer("\\audio{}");
        assert_eq!(vec![text("\\audio{}")], segments);
        assert_eq!(vec![audio_unsupported(), malformed_lint("audio")], lints);
    }

    #[test]
    fn audio_is_recognized_in_the_front_region_too() {
        let (segments, lints) = scan("\\audio{x.mp3}", Region::Front);
        assert_eq!(vec![text("\\audio{x.mp3}")], segments);
        assert_eq!(vec![audio_unsupported()], lints);
    }

    #[test]
    fn escaped_audio_marker_stays_literal_without_a_lint() {
        let (segments, lints) = answer("\\\\audio{x.mp3}");
        assert_eq!(vec![text("\\audio{x.mp3}")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn unknown_backslash_command_stays_literal() {
        let (segments, lints) = answer("\\frac{1}{2}");
        assert_eq!(vec![text("\\frac{1}{2}")], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn a_marker_name_typo_stays_literal() {
        let (segments, lints) = answer("\\imagee{x}");
        assert_eq!(vec![text("\\imagee{x}")], segments);
        assert!(lints.is_empty());
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
    fn image_in_the_front_region_yields() {
        let (segments, _) = scan("\\image{x}", Region::Front);
        assert_eq!(vec![image("x", None)], segments);
    }

    #[test]
    fn two_adjacent_images_both_yield() {
        let (segments, _) = answer("\\image{a}\\image{b}");
        assert_eq!(vec![image("a", None), image("b", None)], segments);
    }

    #[test]
    fn an_image_can_share_a_line_with_a_hole() {
        let (segments, _) = answer("\\cloze{a} and \\image{x}");
        assert_eq!(vec![hole("a"), text(" and "), image("x", None)], segments);
    }

    #[test]
    fn capitalized_alt_key_lints_and_no_alt_is_set() {
        let (segments, lints) = answer("\\image{x}{Alt: y}");
        assert_eq!(vec![image("x", None)], segments);
        assert_eq!(vec![bad_option_lint("Alt")], lints);
    }

    #[test]
    fn all_caps_alt_key_lints_and_no_alt_is_set() {
        let (segments, lints) = answer("\\image{x}{ALT: y}");
        assert_eq!(vec![image("x", None)], segments);
        assert_eq!(vec![bad_option_lint("ALT")], lints);
    }

    #[test]
    fn option_key_is_trimmed_but_case_sensitive() {
        let (segments, lints) = answer("\\image{x}{ alt : y}");
        assert_eq!(vec![image("x", Some("y"))], segments);
        assert!(lints.is_empty());
    }

    #[test]
    fn option_values_are_trimmed_on_both_ends() {
        let (segments, _) = answer("\\image{x}{alt:  y  }");
        assert_eq!(vec![image("x", Some("y"))], segments);
    }

    #[test]
    fn an_empty_option_value_stays_an_empty_alt() {
        let (segments, _) = answer("\\image{x}{alt:}");
        assert_eq!(vec![image("x", Some(""))], segments);
    }

    #[test]
    fn escaped_braces_in_the_src_are_unescaped() {
        let (segments, _) = answer("\\image{a\\{b\\}.png}");
        assert_eq!(vec![image("a{b}.png", None)], segments);
    }

    #[test]
    fn an_unclosed_option_group_stays_literal_and_lints_malformed() {
        let (segments, lints) = answer("\\image{x}{alt: oops");
        assert_eq!(vec![image("x", None), text("{alt: oops")], segments);
        assert_eq!(vec![malformed_lint("image")], lints);
    }

    #[test]
    fn seg_text_round_trips_an_image() {
        let (segments, _) = answer("see \\image{m.png}{alt: a} here");
        assert_eq!("see \\image{m.png}{alt: a} here", seg_text(&segments));
    }

    #[test]
    fn hash_repr_wraps_an_image_in_sentinels() {
        let (segments, _) = answer("\\image{m.png}");
        assert_eq!("\u{1f}image\u{1f}m.png\u{1f}", hash_repr(&segments));
    }

    #[test]
    fn hash_repr_image_does_not_collide_with_the_escaped_literal_text() {
        let image_segments = vec![Seg::Image {
            src: "x".into(),
            alt: None,
        }];
        let literal_segments = vec![Seg::Text("\\image{x}".into())];
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
        let (with_image, _) = answer("\\cloze{a} \\image{x}");
        let (without_image, _) = answer("\\cloze{a}");
        let with_image = hole_fingerprints(&[with_image], &holes);
        let without_image = hole_fingerprints(&[without_image], &holes);
        assert_ne!(with_image[0].line_fp, without_image[0].line_fp);
    }
}
