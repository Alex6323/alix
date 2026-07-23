use serde::{Deserialize, Serialize};

use crate::math::{MathRenderer, MathView};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub math: Option<MathView>,
}

#[derive(Default)]
pub struct DisplayProjector {
    renderer: MathRenderer,
}

impl DisplayProjector {
    pub fn project(&mut self, text: &str) -> Vec<InlineRun> {
        project_text(text, Some(&mut self.renderer), false)
    }

    pub fn project_context(&mut self, text: &str) -> Vec<InlineRun> {
        project_text(text, Some(&mut self.renderer), true)
    }

    #[cfg(test)]
    pub(crate) fn render_count(&self) -> usize {
        self.renderer.render_count()
    }
}

#[derive(Clone, Copy)]
struct Glyph {
    ch: char,
    raw_index: usize,
    escaped: bool,
    code: bool,
    math: Option<usize>,
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
    DisplayProjector::default().project(text)
}

pub fn strip_inline(text: &str) -> String {
    project_text(text, None, false)
        .into_iter()
        .map(|run| run.text)
        .collect()
}

pub fn is_display_math_line(text: &str) -> bool {
    display_math_span(&text.chars().collect::<Vec<_>>()).is_some()
}

#[derive(Clone, Copy)]
struct MathSpan {
    content_start: usize,
    content_end: usize,
    display: bool,
}

fn project_text(
    text: &str,
    mut renderer: Option<&mut MathRenderer>,
    context: bool,
) -> Vec<InlineRun> {
    let mut runs = Vec::new();
    for chunk in text.split_inclusive('\n') {
        let (line, newline) = chunk
            .strip_suffix('\n')
            .map_or((chunk, false), |line| (line, true));
        let mut line_runs = project_line(line, renderer.as_deref_mut(), context);
        append_runs(&mut runs, &mut line_runs);
        if newline {
            push_run(
                &mut runs,
                InlineRun {
                    text: "\n".to_string(),
                    ..InlineRun::default()
                },
            );
        }
    }
    runs
}

fn project_line(
    text: &str,
    mut renderer: Option<&mut MathRenderer>,
    context: bool,
) -> Vec<InlineRun> {
    let chars: Vec<char> = text.chars().collect();
    let spans = math_spans(&chars);
    let glyphs = scan_glyphs(&chars, &spans);
    let delimiters = emphasis_delimiters(&glyphs);
    let mut bold = vec![false; glyphs.len()];
    let mut italic = vec![false; glyphs.len()];
    let mut removed = vec![false; glyphs.len()];
    let mut remaining: Vec<usize> = delimiters.iter().map(|delimiter| delimiter.len).collect();
    let mut consumed_left = vec![0; delimiters.len()];
    let mut consumed_right = vec![0; delimiters.len()];
    let mut open: Vec<usize> = Vec::new();

    for (delimiter_index, delimiter) in delimiters.iter().enumerate() {
        while delimiter.can_close && remaining[delimiter_index] > 0 {
            let Some(open_pos) = open.iter().rposition(|candidate| {
                let opener = delimiters[*candidate];
                opener.marker == delimiter.marker && remaining[*candidate] > 0
            }) else {
                break;
            };
            let opener_index = open[open_pos];
            while remaining[opener_index] >= 2 && remaining[delimiter_index] >= 2 {
                consume_delimiters(
                    &delimiters,
                    opener_index,
                    delimiter_index,
                    2,
                    &mut remaining,
                    &mut consumed_left,
                    &mut consumed_right,
                    &mut removed,
                    &mut bold,
                );
            }
            if remaining[opener_index] > 0 && remaining[delimiter_index] > 0 {
                consume_delimiters(
                    &delimiters,
                    opener_index,
                    delimiter_index,
                    1,
                    &mut remaining,
                    &mut consumed_left,
                    &mut consumed_right,
                    &mut removed,
                    &mut italic,
                );
            }
            if remaining[opener_index] == 0 {
                open.remove(open_pos);
            }
        }
        if delimiter.can_open && remaining[delimiter_index] > 0 {
            open.push(delimiter_index);
        }
    }

    let mut runs = Vec::new();
    let mut index = 0;
    while index < glyphs.len() {
        if let Some(span_index) = glyphs[index].math {
            let start = index;
            while index < glyphs.len() && glyphs[index].math == Some(span_index) {
                index += 1;
            }
            let source: String = glyphs[start..index].iter().map(|glyph| glyph.ch).collect();
            let math = renderer
                .as_deref_mut()
                .map(|renderer| renderer.view(&source, spans[span_index].display, context));
            push_run(
                &mut runs,
                InlineRun {
                    text: source,
                    math,
                    ..InlineRun::default()
                },
            );
            continue;
        }
        let glyph = glyphs[index];
        if removed[index] {
            index += 1;
            continue;
        }
        let style = (bold[index], italic[index], glyph.code);
        push_run(
            &mut runs,
            InlineRun {
                text: glyph.ch.to_string(),
                bold: style.0,
                italic: style.1,
                code: style.2,
                math: None,
            },
        );
        index += 1;
    }
    runs
}

fn append_runs(target: &mut Vec<InlineRun>, source: &mut Vec<InlineRun>) {
    for run in source.drain(..) {
        push_run(target, run);
    }
}

fn push_run(runs: &mut Vec<InlineRun>, run: InlineRun) {
    if run.text.is_empty() {
        return;
    }
    if run.math.is_none()
        && let Some(previous) = runs.last_mut()
        && previous.math.is_none()
        && (previous.bold, previous.italic, previous.code) == (run.bold, run.italic, run.code)
    {
        previous.text.push_str(&run.text);
    } else {
        runs.push(run);
    }
}

fn math_spans(chars: &[char]) -> Vec<MathSpan> {
    if let Some(display) = display_math_span(chars) {
        return vec![display];
    }
    let mut spans = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`'
            && !is_escaped(chars, index)
            && let Some(end) = find_unescaped(chars, index + 1, '`')
        {
            index = end + 1;
            continue;
        }
        if chars[index] != '$' || is_escaped(chars, index) {
            index += 1;
            continue;
        }
        if chars.get(index + 1) == Some(&'$') {
            index = find_double_close(chars, index + 2).map_or(index + 2, |end| end + 2);
            continue;
        }
        if chars.get(index + 1).is_none_or(|next| next.is_whitespace()) {
            index += 1;
            continue;
        }
        let Some(close) = find_inline_close(chars, index + 1) else {
            index += 1;
            continue;
        };
        spans.push(MathSpan {
            content_start: index + 1,
            content_end: close,
            display: false,
        });
        index = close + 1;
    }
    spans
}

fn display_math_span(chars: &[char]) -> Option<MathSpan> {
    let start = chars.iter().position(|ch| !ch.is_whitespace())?;
    let end = chars.iter().rposition(|ch| !ch.is_whitespace())? + 1;
    if end.saturating_sub(start) < 5
        || chars.get(start) != Some(&'$')
        || chars.get(start + 1) != Some(&'$')
        || chars.get(end - 2) != Some(&'$')
        || chars.get(end - 1) != Some(&'$')
        || chars.get(start + 2).is_none_or(|ch| ch.is_whitespace())
        || chars.get(end - 3).is_none_or(|ch| ch.is_whitespace())
    {
        return None;
    }
    let close = find_double_close(chars, start + 2)?;
    (close + 2 == end).then_some(MathSpan {
        content_start: start + 2,
        content_end: close,
        display: true,
    })
}

fn find_inline_close(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start;
    while index < chars.len() {
        if chars[index] != '$' || is_escaped(chars, index) {
            index += 1;
            continue;
        }
        if chars.get(index + 1) == Some(&'$') || index > 0 && chars[index - 1] == '$' {
            index += 1;
            continue;
        }
        let previous = chars.get(index.wrapping_sub(1));
        let next = chars.get(index + 1);
        if previous.is_some_and(|ch| !ch.is_whitespace())
            && next.is_none_or(|ch| !ch.is_ascii_digit())
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn find_double_close(chars: &[char], start: usize) -> Option<usize> {
    (start..chars.len().saturating_sub(1)).find(|index| {
        chars[*index] == '$'
            && chars[*index + 1] == '$'
            && !is_escaped(chars, *index)
            && chars
                .get(index.wrapping_sub(1))
                .is_some_and(|ch| !ch.is_whitespace())
    })
}

fn find_unescaped(chars: &[char], start: usize, needle: char) -> Option<usize> {
    (start..chars.len()).find(|index| chars[*index] == needle && !is_escaped(chars, *index))
}

fn is_escaped(chars: &[char], index: usize) -> bool {
    chars[..index]
        .iter()
        .rev()
        .take_while(|ch| **ch == '\\')
        .count()
        % 2
        == 1
}

#[expect(
    clippy::too_many_arguments,
    reason = "delimiter state updates stay atomic"
)]
fn consume_delimiters(
    delimiters: &[Delimiter],
    opener_index: usize,
    closer_index: usize,
    count: usize,
    remaining: &mut [usize],
    consumed_left: &mut [usize],
    consumed_right: &mut [usize],
    removed: &mut [bool],
    style: &mut [bool],
) {
    let opener = delimiters[opener_index];
    let opener_end = opener.start + opener.len - consumed_right[opener_index];
    removed[opener_end - count..opener_end].fill(true);
    consumed_right[opener_index] += count;
    remaining[opener_index] -= count;

    let closer = delimiters[closer_index];
    let closer_start = closer.start + consumed_left[closer_index];
    removed[closer_start..closer_start + count].fill(true);
    consumed_left[closer_index] += count;
    remaining[closer_index] -= count;

    style[opener_end..closer_start].fill(true);
}

fn scan_glyphs(chars: &[char], spans: &[MathSpan]) -> Vec<Glyph> {
    let mut math = vec![None; chars.len()];
    let mut removed = vec![false; chars.len()];
    if spans.first().is_some_and(|span| span.display) {
        removed.fill(true);
    }
    for (span_index, span) in spans.iter().enumerate() {
        math[span.content_start..span.content_end].fill(Some(span_index));
        removed[span.content_start..span.content_end].fill(false);
        if !span.display {
            removed[span.content_start - 1] = true;
            removed[span.content_end] = true;
        }
    }
    let mut glyphs = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        if removed[index] {
            index += 1;
            continue;
        }
        if let Some(span_index) = math[index] {
            glyphs.push(Glyph {
                ch: chars[index],
                raw_index: index,
                escaped: false,
                code: false,
                math: Some(span_index),
            });
            index += 1;
            continue;
        }
        if chars[index] == '\\'
            && let Some(next) = chars.get(index + 1)
            && matches!(next, '*' | '_' | '`' | '$' | '\\')
            && math[index + 1].is_none()
            && !removed[index + 1]
        {
            glyphs.push(Glyph {
                ch: *next,
                raw_index: index + 1,
                escaped: true,
                code: false,
                math: None,
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
                math: None,
            }));
            index = end + 1;
            continue;
        }
        glyphs.push(Glyph {
            ch: chars[index],
            raw_index: index,
            escaped: false,
            code: false,
            math: None,
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
        if glyph.escaped || glyph.code || glyph.math.is_some() || !matches!(glyph.ch, '*' | '_') {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < glyphs.len()
            && glyphs[end].ch == glyph.ch
            && !glyphs[end].escaped
            && !glyphs[end].code
            && glyphs[end].math.is_none()
            && glyphs[end].raw_index == glyphs[end - 1].raw_index + 1
        {
            end += 1;
        }
        let len = end - index;
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
            can_close: !intraword && previous.is_some_and(|item| !item.ch.is_whitespace()),
        });
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
            math: None,
        }
    }

    fn bold(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: true,
            italic: false,
            code: false,
            math: None,
        }
    }

    fn italic(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: false,
            italic: true,
            code: false,
            math: None,
        }
    }

    fn code(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: false,
            italic: false,
            code: true,
            math: None,
        }
    }

    fn bold_italic(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: true,
            italic: true,
            code: false,
            math: None,
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
    fn double_underscore_is_bold() {
        assert_eq!(vec![bold("bold")], parse_inline("__bold__"));
    }

    #[test]
    fn intraword_double_underscore_is_literal() {
        assert_eq!(vec![plain("a__b__c")], parse_inline("a__b__c"));
    }

    #[test]
    fn triple_marker_is_bold_and_italic() {
        assert_eq!(vec![bold_italic("x")], parse_inline("***x***"));
        assert_eq!(vec![bold_italic("x")], parse_inline("___x___"));
    }

    #[test]
    fn strong_and_emphasis_still_compose() {
        assert_eq!(
            vec![bold("bold "), bold_italic("and italic")],
            parse_inline("**bold _and italic_**"),
        );
        assert_eq!(
            vec![italic("a "), bold_italic("b"), italic(" c")],
            parse_inline("*a **b** c*"),
        );
        assert_eq!(
            vec![plain("a"), bold("b"), plain("c")],
            parse_inline("a**b**c"),
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
        assert_eq!(vec![plain("*literal*")], parse_inline("\\*literal\\*"));
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
    fn inline_math_keeps_source_and_carries_svg() {
        let runs = parse_inline("Why does $a^2 + b^2 = c^2$ hold?");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].text, "a^2 + b^2 = c^2");
        let math = runs[1].math.as_ref().unwrap();
        assert!(!math.display);
        assert!(
            math.svg
                .as_deref()
                .is_some_and(|svg| svg.starts_with("<svg"))
        );
        assert!(math.error.is_none());
        assert!(!runs[1].bold && !runs[1].italic && !runs[1].code);
    }

    #[test]
    fn whole_trimmed_line_is_display_math() {
        let runs = parse_inline("  $$x_1$$  ");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "x_1");
        assert!(runs[0].math.as_ref().unwrap().display);
        assert_eq!(strip_inline("  $$x_1$$  "), "x_1");
        assert!(is_display_math_line("  $$x_1$$  "));
    }

    #[test]
    fn embedded_display_pair_is_wholly_literal() {
        let text = "Compare $$x_1$$ now";
        assert_eq!(parse_inline(text), vec![plain(text)]);
        assert_eq!(strip_inline(text), text);
        assert!(!is_display_math_line(text));
    }

    #[test]
    fn delimiter_rules_avoid_currency_and_whitespace() {
        for text in [
            "The price is $5",
            "$5 and $10",
            "$5 and x$10",
            "$ x $",
            "$x $",
            "$ x$",
        ] {
            assert_eq!(parse_inline(text), vec![plain(text)], "{text}");
        }
        assert_eq!(parse_inline(r"\$x"), vec![plain("$x")]);
    }

    #[test]
    fn unmatched_and_empty_math_stay_literal() {
        for text in ["$x", "x$", "$$", "$$$$", "$ $", "before $$x"] {
            assert_eq!(parse_inline(text), vec![plain(text)], "{text}");
        }
    }

    #[test]
    fn math_is_protected_from_emphasis_but_surrounding_emphasis_pairs() {
        let runs = parse_inline("**energy $E=mc^2$**");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], bold("energy "));
        assert_eq!(runs[1].text, "E=mc^2");
        assert!(runs[1].math.is_some());
        assert!(!runs[1].bold);

        let formula = parse_inline("$x_i * y_j$");
        assert_eq!(formula.len(), 1);
        assert_eq!(formula[0].text, "x_i * y_j");
        assert!(formula[0].math.is_some());
    }

    #[test]
    fn dollars_inside_code_are_literal() {
        assert_eq!(parse_inline("`$x$`"), vec![code("$x$")]);
        assert_eq!(strip_inline("`$x$`"), "$x$");
    }

    #[test]
    fn malformed_recognized_math_is_one_error_run() {
        let runs = parse_inline(r"$\frac{1$");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, r"\frac{1");
        let math = runs[0].math.as_ref().unwrap();
        assert!(math.svg.is_none());
        assert!(math.error.is_some());
    }

    #[test]
    fn strip_inline_never_invokes_ratex() {
        let before = crate::math::thread_render_count();
        assert_eq!(
            strip_inline(r"Answer $x^2$ and $$y^2$$"),
            r"Answer x^2 and $$y^2$$"
        );
        assert_eq!(crate::math::thread_render_count(), before);
    }

    #[test]
    fn logical_lines_recognize_display_math_independently() {
        let runs = parse_inline("before\n$$x^2$$\nafter");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0], plain("before\n"));
        assert_eq!(runs[1].text, "x^2");
        assert!(runs[1].math.as_ref().unwrap().display);
        assert_eq!(runs[2], plain("\nafter"));
    }

    #[test]
    fn repeated_formula_sources_render_once_per_projector() {
        let mut projector = DisplayProjector::default();
        projector.project("$x^2$ and $x^2$");
        projector.project("$$x^2$$");
        assert_eq!(projector.render_count(), 1);
    }

    #[test]
    fn empty_input_yields_no_runs() {
        assert!(parse_inline("").is_empty());
    }
}
