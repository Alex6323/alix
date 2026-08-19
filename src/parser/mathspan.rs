//! The structural-unit policy for span matches inside math source (ADR
//! 0034): a masked range must be a complete structural unit of its formula,
//! so replacing it with a boxed blank can never leave dangling structure.
//! A whole-formula match (compared after trimming outer whitespace) is
//! always such a unit and bypasses the token rules; the renderer re-parse
//! backstop in the binding code still checks it.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Violation {
    EndpointInsideControlSequence(String),
    StructuralToken(char),
    RowBreak,
    GroupSplit,
    ControlSequence(String),
}

impl Violation {
    pub(super) fn message(&self) -> String {
        match self {
            Violation::EndpointInsideControlSequence(name) => {
                format!("a match edge cuts into the control sequence `{name}`")
            }
            Violation::StructuralToken(token) => {
                format!("the match contains the structural token `{token}`")
            }
            Violation::RowBreak => r"the match contains a `\\` row break".to_string(),
            Violation::GroupSplit => "the match splits a brace group".to_string(),
            Violation::ControlSequence(name) => format!(
                "the match contains the control sequence `{name}`; only a whole-formula blank may contain commands"
            ),
        }
    }
}

enum Kind {
    Sequence,
    Open,
    Close,
    Structural(char),
    Other,
}

struct Token {
    span: Range<usize>,
    kind: Kind,
}

fn tokens(payload: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = payload.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        let kind = match ch {
            '\\' => {
                let mut end = at + 1;
                let mut word = false;
                while let Some((next_at, next)) = chars.peek().copied() {
                    if !next.is_ascii_alphabetic() {
                        break;
                    }
                    chars.next();
                    word = true;
                    end = next_at + 1;
                }
                if !word && let Some((next_at, next)) = chars.peek().copied() {
                    chars.next();
                    end = next_at + next.len_utf8();
                }
                out.push(Token {
                    span: at..end,
                    kind: Kind::Sequence,
                });
                continue;
            }
            '{' => Kind::Open,
            '}' => Kind::Close,
            '^' | '_' | '&' | '%' | '#' => Kind::Structural(ch),
            _ => Kind::Other,
        };
        out.push(Token {
            span: at..at + ch.len_utf8(),
            kind,
        });
    }
    out
}

fn trim_extent(payload: &str, range: &Range<usize>) -> Range<usize> {
    let slice = &payload[range.clone()];
    let start = range.start + (slice.len() - slice.trim_start().len());
    start..start + slice.trim().len()
}

pub(super) fn structural_unit(payload: &str, range: &Range<usize>) -> Result<(), Violation> {
    if trim_extent(payload, &(0..payload.len())) == trim_extent(payload, range) {
        return Ok(());
    }

    let tokens = tokens(payload);
    let mut depth_before = vec![0i32; payload.len() + 1];
    let mut depth = 0i32;
    for token in &tokens {
        for byte in token.span.clone() {
            depth_before[byte] = depth;
        }
        match token.kind {
            Kind::Open => depth += 1,
            Kind::Close => depth -= 1,
            _ => {}
        }
    }
    depth_before[payload.len()] = depth;

    for token in &tokens {
        if !matches!(token.kind, Kind::Sequence) {
            continue;
        }
        for edge in [range.start, range.end] {
            if token.span.start < edge && edge < token.span.end {
                return Err(Violation::EndpointInsideControlSequence(
                    payload[token.span.clone()].to_string(),
                ));
            }
        }
    }

    for token in &tokens {
        if token.span.start < range.start || token.span.end > range.end {
            continue;
        }
        match &token.kind {
            Kind::Structural(ch) => return Err(Violation::StructuralToken(*ch)),
            Kind::Sequence => {
                let text = &payload[token.span.clone()];
                if text == r"\\" {
                    return Err(Violation::RowBreak);
                }
                if text != r"\{" && text != r"\}" {
                    return Err(Violation::ControlSequence(text.to_string()));
                }
            }
            _ => {}
        }
    }

    let entry = depth_before[range.start];
    if depth_before[range.end] != entry
        || (range.start + 1..range.end).any(|byte| depth_before[byte] < entry)
    {
        return Err(Violation::GroupSplit);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(payload: &str, hidden: &str) -> Result<(), Violation> {
        let start = payload.find(hidden).expect("fixture hidden text present");
        structural_unit(payload, &(start..start + hidden.len()))
    }

    #[test]
    fn a_whole_payload_match_bypasses_every_token_rule() {
        assert_eq!(Ok(()), unit(r"x^2", r"x^2"));
        assert_eq!(
            Ok(()),
            unit(
                r"\begin{pmatrix}a & b\end{pmatrix}",
                r"\begin{pmatrix}a & b\end{pmatrix}"
            ),
        );
    }

    #[test]
    fn outer_whitespace_trims_on_both_sides_of_the_comparison() {
        assert_eq!(Ok(()), unit(r" \gamma ", r"\gamma"));
        assert_eq!(
            Ok(()),
            structural_unit(" \\gamma ", &(0..8)),
            "a match carrying the outer whitespace behaves like the trimmed form"
        );
    }

    #[test]
    fn every_match_endpoint_inside_a_control_word_is_named() {
        for hidden in ["q", "eq", r"\le"] {
            assert_eq!(
                Err(Violation::EndpointInsideControlSequence(
                    r"\leq".to_string()
                )),
                unit(r"x \leq y", hidden),
                "{hidden}"
            );
        }
    }

    #[test]
    fn every_unescaped_structural_token_is_rejected_inside_a_match() {
        for (payload, hidden, token) in [
            (r"x^2", r"x^", '^'),
            (r"x^2", r"^2", '^'),
            (r"a_i", r"a_", '_'),
            (r"a & b", r"a &", '&'),
            (r"a % b", r"a %", '%'),
            (r"a # b", r"a #", '#'),
        ] {
            assert_eq!(
                Err(Violation::StructuralToken(token)),
                unit(payload, hidden),
                "{payload} / {hidden}"
            );
        }
    }

    #[test]
    fn a_row_break_inside_a_match_is_rejected() {
        assert_eq!(Err(Violation::RowBreak), unit(r"a \\ b", r"a \\"));
    }

    #[test]
    fn a_contained_command_is_rejected_naming_it() {
        assert_eq!(
            Err(Violation::ControlSequence(r"\gamma".to_string())),
            unit(r"x + \gamma + y", r"+ \gamma +")
        );
        assert_eq!(
            Err(Violation::ControlSequence(r"\left".to_string())),
            unit(r"\left( x \right)", r"\left( x")
        );
    }

    #[test]
    fn escaped_braces_are_literal_inside_and_as_matches() {
        assert_eq!(Ok(()), unit(r"A=\{x\}", r"\{x\}"));
        assert_eq!(Ok(()), unit(r"A=\{x\}", r"x"));
    }

    #[test]
    fn a_match_splitting_real_brace_groups_is_rejected() {
        assert_eq!(
            Err(Violation::GroupSplit),
            unit(r"\frac{ab}{cd}", r"ab}{cd")
        );
        assert_eq!(Err(Violation::GroupSplit), unit(r"{ab}", r"{ab"));
        assert_eq!(Err(Violation::GroupSplit), unit(r"a{b}c", r"b}c"));
    }

    #[test]
    fn an_equal_depth_group_interior_is_a_unit() {
        assert_eq!(Ok(()), unit(r"\frac{ab}{cd}", r"ab"));
        assert_eq!(Ok(()), unit(r"x^{n+1}", r"n+1"));
    }
}
