use super::{Lint, LintKind, ParseError, canonical::hash64, collapse, trim_ws};
use crate::store::HoleFingerprint;

// A NUL is safe here: the parser rejects C0 controls outside the whitespace
// set, so it can never occur in real card text.
const HOLE_MASK: &str = "\u{0}";

pub const BLANK: &str = "____";

pub const HIDDEN: &str = "[…]";

pub(super) enum Seg {
    Text(String),
    Hole(String),
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
                }
            }
            HoleFingerprint {
                text_fp,
                line_fp: hash64(&collapse(&line)),
            }
        })
        .collect()
}

pub(super) fn scan_cloze(
    line_text: &str,
    lineno: usize,
    lints: &mut Vec<Lint>,
) -> Result<Vec<Seg>, ParseError> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut rest = line_text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("\\\\cloze") {
            text.push_str("\\cloze");
            rest = after;
        } else if let Some(after) = rest.strip_prefix("\\cloze") {
            if let Some(arg) = after.strip_prefix('{') {
                let (content, after_hole) = scan_hole(arg, lineno)?;
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

fn scan_hole(arg: &str, lineno: usize) -> Result<(String, &str), ParseError> {
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
        }
    }
    out
}
