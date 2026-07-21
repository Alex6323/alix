use serde::{Deserialize, Serialize};

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InlineRun {
    pub text: String,
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub code: bool,
}

#[derive(Clone, Copy)]
struct Glyph {
    ch: char,
    raw_index: usize,
    escaped: bool,
    code: bool,
}

#[derive(Clone, Copy)]
struct Delimiter {
    start: usize,
    len: usize,
    marker: char,
    can_open: bool,
    can_close: bool,
}

pub fn parse_inline(text: &str) -> Vec<InlineRun> {
    let chars: Vec<char> = text.chars().collect();
    let glyphs = scan_glyphs(&chars);
    let delimiters = emphasis_delimiters(&glyphs);
    let mut bold = vec![false; glyphs.len()];
    let mut italic = vec![false; glyphs.len()];
    let mut removed = vec![false; glyphs.len()];
    let mut open: Vec<usize> = Vec::new();

    for (delimiter_index, delimiter) in delimiters.iter().enumerate() {
        let mut matched = false;
        if delimiter.can_close
            && let Some(open_pos) = open.iter().rposition(|candidate| {
                let opener = delimiters[*candidate];
                opener.marker == delimiter.marker && opener.len == delimiter.len
            })
        {
            let opener = delimiters[open.remove(open_pos)];
            removed[opener.start..opener.start + opener.len].fill(true);
            removed[delimiter.start..delimiter.start + delimiter.len].fill(true);
            let coverage = opener.start + opener.len..delimiter.start;
            if delimiter.len == 2 {
                bold[coverage].fill(true);
            } else {
                italic[coverage].fill(true);
            }
            matched = true;
        }
        if !matched && delimiter.can_open {
            open.push(delimiter_index);
        }
    }

    let mut runs: Vec<InlineRun> = Vec::new();
    for (index, glyph) in glyphs.iter().enumerate() {
        if removed[index] {
            continue;
        }
        let style = (bold[index], italic[index], glyph.code);
        if let Some(run) = runs.last_mut()
            && (run.bold, run.italic, run.code) == style
        {
            run.text.push(glyph.ch);
        } else {
            runs.push(InlineRun {
                text: glyph.ch.to_string(),
                bold: style.0,
                italic: style.1,
                code: style.2,
            });
        }
    }
    runs
}

pub fn strip_inline(text: &str) -> String {
    parse_inline(text).into_iter().map(|run| run.text).collect()
}

fn scan_glyphs(chars: &[char]) -> Vec<Glyph> {
    let mut glyphs = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\'
            && let Some(next) = chars.get(index + 1)
            && matches!(next, '*' | '_' | '`' | '\\')
        {
            glyphs.push(Glyph {
                ch: *next,
                raw_index: index + 1,
                escaped: true,
                code: false,
            });
            index += 2;
            continue;
        }
        if chars[index] == '`'
            && let Some(offset) = chars[index + 1..].iter().position(|ch| *ch == '`')
        {
            let end = index + offset + 1;
            glyphs.extend((index + 1..end).map(|raw_index| Glyph {
                ch: chars[raw_index],
                raw_index,
                escaped: true,
                code: true,
            }));
            index = end + 1;
            continue;
        }
        glyphs.push(Glyph {
            ch: chars[index],
            raw_index: index,
            escaped: false,
            code: false,
        });
        index += 1;
    }
    glyphs
}

fn emphasis_delimiters(glyphs: &[Glyph]) -> Vec<Delimiter> {
    let mut delimiters = Vec::new();
    let mut index = 0;
    while index < glyphs.len() {
        let glyph = glyphs[index];
        if glyph.escaped || glyph.code || !matches!(glyph.ch, '*' | '_') {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < glyphs.len()
            && glyphs[end].ch == glyph.ch
            && !glyphs[end].escaped
            && !glyphs[end].code
            && glyphs[end].raw_index == glyphs[end - 1].raw_index + 1
        {
            end += 1;
        }
        let len = end - index;
        if len <= 2 && (glyph.ch == '*' || len == 1) {
            let previous = index.checked_sub(1).and_then(|pos| glyphs.get(pos));
            let next = glyphs.get(end);
            let intraword = glyph.ch == '_'
                && previous.is_some_and(|item| item.ch.is_alphanumeric())
                && next.is_some_and(|item| item.ch.is_alphanumeric());
            delimiters.push(Delimiter {
                start: index,
                len,
                marker: glyph.ch,
                can_open: !intraword && next.is_some_and(|item| !item.ch.is_whitespace()),
                can_close: !intraword
                    && previous.is_some_and(|item| !item.ch.is_whitespace()),
            });
        }
        index = end;
    }
    delimiters
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: false,
            italic: false,
            code: false,
        }
    }

    fn bold(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: true,
            italic: false,
            code: false,
        }
    }

    fn italic(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: false,
            italic: true,
            code: false,
        }
    }

    fn code(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: false,
            italic: false,
            code: true,
        }
    }

    fn bold_italic(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: true,
            italic: true,
            code: false,
        }
    }

    #[test]
    fn plain_text_is_one_run() {
        assert_eq!(vec![plain("plain text")], parse_inline("plain text"));
    }

    #[test]
    fn bold_italic_code_render() {
        assert_eq!(vec![bold("Paris")], parse_inline("**Paris**"));
        assert_eq!(vec![italic("x")], parse_inline("*x*"));
        assert_eq!(vec![italic("x")], parse_inline("_x_"));
        assert_eq!(vec![code("HashMap")], parse_inline("`HashMap`"));
    }

    #[test]
    fn emphasis_splits_surrounding_text() {
        assert_eq!(
            vec![plain("The capital is "), bold("Paris"), plain(".")],
            parse_inline("The capital is **Paris**."),
        );
    }

    #[test]
    fn inline_code_is_verbatim() {
        assert_eq!(vec![code("**x**")], parse_inline("`**x**`"));
    }

    #[test]
    fn spaced_stars_do_not_emphasize() {
        assert_eq!(vec![plain("a * b * c")], parse_inline("a * b * c"));
    }

    #[test]
    fn tight_stars_do_emphasize() {
        assert_eq!(
            vec![plain("2"), italic("3"), plain("4")],
            parse_inline("2*3*4")
        );
    }

    #[test]
    fn intraword_underscore_is_literal() {
        assert_eq!(
            vec![plain("snake_case_word")],
            parse_inline("snake_case_word")
        );
    }

    #[test]
    fn nesting_combines_flags() {
        assert_eq!(
            vec![bold("a "), bold_italic("b"), bold(" c")],
            parse_inline("**a _b_ c**"),
        );
    }

    #[test]
    fn backslash_escapes_a_marker() {
        assert_eq!(
            vec![plain("*literal*")],
            parse_inline("\\*literal\\*")
        );
    }

    #[test]
    fn unmatched_marker_is_literal() {
        assert_eq!(
            vec![plain("func(**kwargs)")],
            parse_inline("func(**kwargs)")
        );
    }

    #[test]
    fn strip_inline_is_the_content_projection() {
        assert_eq!("Paris", strip_inline("**Paris**"));
        assert_eq!(
            "The capital is Paris.",
            strip_inline("The capital is **Paris**.")
        );
        assert_eq!("**x**", strip_inline("`**x**`"));
        assert_eq!("234", strip_inline("2*3*4"));
        assert_eq!("", strip_inline(""));
    }

    #[test]
    fn empty_input_yields_no_runs() {
        assert!(parse_inline("").is_empty());
    }
}
