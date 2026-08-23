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
    #[serde(default, skip_serializing_if = "is_false")]
    pub strike: bool,
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

    pub(crate) fn project_display_math(&mut self, source: &str) -> Vec<InlineRun> {
        let math = self.renderer.view(source, true, false);
        vec![InlineRun {
            text: source.to_string(),
            math: Some(math),
            ..InlineRun::default()
        }]
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
    delimiter_len: usize,
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

struct LineClassification {
    spans: Vec<MathSpan>,
    glyphs: Vec<Glyph>,
    bold: Vec<bool>,
    italic: Vec<bool>,
    strike: Vec<bool>,
    removed: Vec<bool>,
}

fn classify_line(text: &str) -> LineClassification {
    let chars: Vec<char> = text.chars().collect();
    let spans = math_spans(&chars);
    let glyphs = scan_glyphs(&chars, &spans);
    let delimiters = emphasis_delimiters(&glyphs);
    let mut bold = vec![false; glyphs.len()];
    let mut italic = vec![false; glyphs.len()];
    let mut strike = vec![false; glyphs.len()];
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
            let strike_pair = delimiter.marker == '~';
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
                    if strike_pair { &mut strike } else { &mut bold },
                );
            }
            if !strike_pair && remaining[opener_index] > 0 && remaining[delimiter_index] > 0 {
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
    LineClassification {
        spans,
        glyphs,
        bold,
        italic,
        strike,
        removed,
    }
}

fn project_line(
    text: &str,
    mut renderer: Option<&mut MathRenderer>,
    context: bool,
) -> Vec<InlineRun> {
    let LineClassification {
        spans,
        glyphs,
        bold,
        italic,
        strike,
        removed,
    } = classify_line(text);
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
        push_run(
            &mut runs,
            InlineRun {
                text: glyph.ch.to_string(),
                bold: bold[index],
                italic: italic[index],
                strike: strike[index],
                code: glyph.code,
                math: None,
            },
        );
        index += 1;
    }
    runs
}

/// One visible piece of a line for the canonical maskable stream (ADR 0034):
/// maximal same-style visible text carrying, per visible char, the authored
/// char range a masking splice must replace (an escaped char's range starts
/// at its backslash, or the splice would leave a stray escape before the
/// mask marker).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LinePiece {
    pub text: String,
    /// Authored char index where each visible char's splice starts.
    pub starts: Vec<usize>,
    /// Authored char index one past each visible char.
    pub ends: Vec<usize>,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub math: bool,
}

pub(crate) fn line_pieces(text: &str, excluded: &[std::ops::Range<usize>]) -> Vec<LinePiece> {
    let LineClassification {
        glyphs,
        bold,
        italic,
        removed,
        ..
    } = classify_line(text);
    let linked = link_syntax_mask(&glyphs);
    let byte_at: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    let mut pieces: Vec<LinePiece> = Vec::new();
    // A dropped glyph (link syntax, or an excluded authored range such as a
    // cloze hole footprint) splits the surrounding text into separate
    // pieces, so no match and no splice can quietly span the gap.
    let mut gap = false;
    // The math span the last pushed piece belongs to, so one formula stays
    // one piece while a gap still splits it.
    let mut last_math: Option<usize> = None;
    for (index, glyph) in glyphs.iter().enumerate() {
        if glyph.math.is_none() && removed[index] {
            continue;
        }
        if linked[index]
            || excluded
                .iter()
                .any(|range| range.contains(&byte_at[glyph.raw_index]))
        {
            gap = true;
            continue;
        }
        let splice_start = glyph.raw_index - usize::from(glyph.escaped && !glyph.code);
        if let Some(span_index) = glyph.math {
            match pieces.last_mut() {
                Some(piece) if piece.math && last_math == Some(span_index) && !gap => {
                    piece.text.push(glyph.ch);
                    piece.starts.push(glyph.raw_index);
                    piece.ends.push(glyph.raw_index + 1);
                }
                _ => {
                    pieces.push(LinePiece {
                        text: glyph.ch.to_string(),
                        starts: vec![glyph.raw_index],
                        ends: vec![glyph.raw_index + 1],
                        bold: false,
                        italic: false,
                        code: false,
                        math: true,
                    });
                    last_math = Some(span_index);
                }
            }
            gap = false;
            continue;
        }
        let style = (bold[index], italic[index], glyph.code);
        match pieces.last_mut() {
            Some(piece)
                if !piece.math && (piece.bold, piece.italic, piece.code) == style && !gap =>
            {
                piece.text.push(glyph.ch);
                piece.starts.push(splice_start);
                piece.ends.push(glyph.raw_index + 1);
            }
            _ => {
                pieces.push(LinePiece {
                    text: glyph.ch.to_string(),
                    starts: vec![splice_start],
                    ends: vec![glyph.raw_index + 1],
                    bold: style.0,
                    italic: style.1,
                    code: style.2,
                    math: false,
                });
                last_math = None;
            }
        }
        gap = false;
    }
    pieces
}

/// Marks the glyphs of every complete `[label](destination)` link: the
/// brackets, the parentheses, and the destination drop from the maskable
/// stream while the label stays visible text. An incomplete pattern is
/// ordinary prose. Display projection never consults this: clients still
/// receive the raw syntax until link styling lands.
fn link_syntax_mask(glyphs: &[Glyph]) -> Vec<bool> {
    let plain = |glyph: &Glyph| !glyph.escaped && !glyph.code && glyph.math.is_none();
    let mut mask = vec![false; glyphs.len()];
    let mut index = 0;
    while index < glyphs.len() {
        if !(plain(&glyphs[index]) && glyphs[index].ch == '[') {
            index += 1;
            continue;
        }
        let close = (index + 1..glyphs.len())
            .find(|&at| plain(&glyphs[at]) && matches!(glyphs[at].ch, ']' | '['));
        let Some(close) = close.filter(|&at| glyphs[at].ch == ']') else {
            index += 1;
            continue;
        };
        if glyphs.get(close + 1).map(|glyph| glyph.ch) != Some('(') {
            index = close + 1;
            continue;
        }
        let mut depth = 1usize;
        let Some(close_paren) = (close + 2..glyphs.len()).find(|&at| {
            if !plain(&glyphs[at]) {
                return false;
            }
            match glyphs[at].ch {
                '(' => {
                    depth += 1;
                    false
                }
                ')' => {
                    depth -= 1;
                    depth == 0
                }
                _ => false,
            }
        }) else {
            index = close + 1;
            continue;
        };
        mask[index] = true;
        mask[close..=close_paren]
            .iter_mut()
            .for_each(|dropped| *dropped = true);
        index = close_paren + 1;
    }
    mask
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
        && (
            previous.bold,
            previous.italic,
            previous.strike,
            previous.code,
        ) == (run.bold, run.italic, run.strike, run.code)
    {
        previous.text.push_str(&run.text);
    } else {
        runs.push(run);
    }
}

/// Whether `marker`'s first occurrence in `text` sits inside a math span.
/// The cloze parser asks this of a hole to learn whether its answer is a
/// piece of a formula rather than prose.
pub fn math_encloses(text: &str, marker: &str) -> bool {
    let Some(byte_index) = text.find(marker) else {
        return false;
    };
    let index = text[..byte_index].chars().count();
    let chars: Vec<char> = text.chars().collect();
    math_spans(&chars)
        .iter()
        .any(|span| span.content_start <= index && index < span.content_end)
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
        if chars.get(index + 1) == Some(&'`') {
            if let Some(close) = backtick_anchored_close(chars, index + 2) {
                spans.push(MathSpan {
                    content_start: index + 2,
                    content_end: close,
                    display: false,
                    delimiter_len: 2,
                });
                index = close + 2;
            } else {
                index += 1;
            }
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
            delimiter_len: 1,
        });
        index = close + 1;
    }
    spans
}

fn backtick_anchored_close(chars: &[char], body_start: usize) -> Option<usize> {
    let close = (body_start..chars.len()).find(|&index| chars[index] == '`')?;
    if chars.get(close + 1) != Some(&'$') {
        return None;
    }
    let body = &chars[body_start..close];
    (!body.iter().all(|ch| ch.is_whitespace())).then_some(close)
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
        delimiter_len: 2,
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

/// The column (1-based) of the first reserved tag-shape angle run.
/// Precedence per angle run: comment, autolink, styled subset,
/// tag-shape, literal prose. None means the line is legal.
pub(crate) fn tag_shape_column(text: &str) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut open_subset: [Option<usize>; 3] = [None; 3];
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' && !is_escaped(&chars, index) {
            index = code_span_end(&chars, index);
            continue;
        }
        if chars[index] != '<' || is_escaped(&chars, index) {
            index += 1;
            continue;
        }
        if index >= 2
            && chars[index - 1] == '('
            && chars[index - 2] == ']'
            && let Some(close) = (index + 1..chars.len()).find(|&i| chars[i] == '>')
        {
            index = close + 1;
            continue;
        }
        if matches_at(&chars, index, "<!--") {
            match find_at(&chars, index + 4, "-->") {
                Some(close) => {
                    index = close + 3;
                    continue;
                }
                None => break,
            }
        }
        if let Some(end) = autolink_end(&chars, index) {
            index = end + 1;
            continue;
        }
        if let Some((slot, len, closing)) = subset_tag(&chars, index) {
            if closing {
                if open_subset[slot].take().is_some() {
                    index += len;
                    continue;
                }
                return Some(index + 1);
            }
            if open_subset[slot].is_none() {
                open_subset[slot] = Some(index + 1);
                index += len;
                continue;
            }
            return Some(index + 1);
        }
        let tag = match chars.get(index + 1) {
            Some('/') => chars
                .get(index + 2)
                .is_some_and(|ch| ch.is_ascii_alphabetic()),
            Some(ch) => ch.is_ascii_alphabetic(),
            None => false,
        };
        if tag {
            return Some(index + 1);
        }
        index += 1;
    }
    open_subset.into_iter().flatten().min()
}

/// The index just past a code span opened at `start` (a backtick run
/// closes only on a run of exactly its own length), or past the
/// unmatched opening run when no close exists.
fn code_span_end(chars: &[char], start: usize) -> usize {
    let run = chars[start..].iter().take_while(|ch| **ch == '`').count();
    let mut index = start + run;
    while index < chars.len() {
        if chars[index] != '`' {
            index += 1;
            continue;
        }
        let close = chars[index..].iter().take_while(|ch| **ch == '`').count();
        index += close;
        if close == run {
            return index;
        }
    }
    start + run
}

fn matches_at(chars: &[char], start: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(offset, ch)| chars.get(start + offset) == Some(&ch))
}

fn find_at(chars: &[char], start: usize, needle: &str) -> Option<usize> {
    (start..chars.len()).find(|&index| matches_at(chars, index, needle))
}

const SUBSET_TAGS: [&str; 3] = ["sub", "sup", "ins"];

fn subset_tag(chars: &[char], start: usize) -> Option<(usize, usize, bool)> {
    let closing = chars.get(start + 1) == Some(&'/');
    let name_start = if closing { start + 2 } else { start + 1 };
    for (slot, name) in SUBSET_TAGS.iter().enumerate() {
        if matches_at(chars, name_start, name) && chars.get(name_start + name.len()) == Some(&'>') {
            return Some((slot, name.len() + if closing { 3 } else { 2 }, closing));
        }
    }
    None
}

fn autolink_end(chars: &[char], start: usize) -> Option<usize> {
    let close = (start + 1..chars.len()).find(|&index| chars[index] == '>')?;
    let body: String = chars[start + 1..close].iter().collect();
    (is_uri_autolink(&body) || is_email_autolink(&body)).then_some(close)
}

fn is_uri_autolink(body: &str) -> bool {
    let Some((scheme, rest)) = body.split_once(':') else {
        return false;
    };
    (2..=32).contains(&scheme.len())
        && scheme.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
        && !rest.is_empty()
        && rest
            .chars()
            .all(|ch| !ch.is_whitespace() && ch != '<' && ch != '>')
}

fn is_email_autolink(body: &str) -> bool {
    let Some((local, domain)) = body.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(ch));
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    local_ok && !domain.is_empty() && domain.split('.').all(label_ok)
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
            removed[span.content_start - span.delimiter_len..span.content_start].fill(true);
            removed[span.content_end..span.content_end + span.delimiter_len].fill(true);
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
            && next.is_ascii_punctuation()
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
        if glyph.escaped
            || glyph.code
            || glyph.math.is_some()
            || !matches!(glyph.ch, '*' | '_' | '~')
        {
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
        // GFM strikethrough is the double-tilde pair alone; any other tilde
        // run is ordinary text, never a delimiter.
        if glyph.ch == '~' && len != 2 {
            index = end;
            continue;
        }
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

    #[test]
    fn line_pieces_map_plain_text_identically() {
        let pieces = line_pieces("plain text", &[]);
        assert_eq!(1, pieces.len());
        assert_eq!("plain text", pieces[0].text);
        assert_eq!((0..10).collect::<Vec<_>>(), pieces[0].starts);
        assert_eq!((1..11).collect::<Vec<_>>(), pieces[0].ends);
        assert!(!pieces[0].bold && !pieces[0].code && !pieces[0].math);
    }

    #[test]
    fn line_pieces_split_styled_from_plain_and_skip_the_markers() {
        let pieces = line_pieces("**New** York", &[]);
        assert_eq!(2, pieces.len());
        assert_eq!(("New", true), (pieces[0].text.as_str(), pieces[0].bold));
        assert_eq!(vec![2, 3, 4], pieces[0].starts);
        assert_eq!((" York", false), (pieces[1].text.as_str(), pieces[1].bold));
        assert_eq!(vec![7, 8, 9, 10, 11], pieces[1].starts);
    }

    #[test]
    fn line_pieces_anchor_an_escaped_char_at_its_backslash() {
        let pieces = line_pieces("a\\*b", &[]);
        assert_eq!(1, pieces.len(), "{pieces:?}");
        assert_eq!("a*b", pieces[0].text);
        assert_eq!(
            vec![0, 1, 3],
            pieces[0].starts,
            "the * splices from its backslash"
        );
        assert_eq!(vec![1, 3, 4], pieces[0].ends);
    }

    #[test]
    fn line_pieces_drop_link_syntax_and_keep_the_label_as_its_own_piece() {
        let pieces = line_pieces("see [the RFC](https://x) now", &[]);
        assert_eq!(3, pieces.len(), "{pieces:?}");
        assert_eq!("see ", pieces[0].text);
        assert_eq!("the RFC", pieces[1].text);
        assert_eq!(
            vec![5, 6, 7, 8, 9, 10, 11],
            pieces[1].starts,
            "the label maps to its authored bytes inside the brackets"
        );
        assert_eq!(
            " now", pieces[2].text,
            "the destination and parens are gone"
        );
    }

    #[test]
    fn an_incomplete_link_pattern_stays_ordinary_prose() {
        let pieces = line_pieces("just [brackets] here", &[]);
        assert_eq!(1, pieces.len(), "{pieces:?}");
        assert_eq!("just [brackets] here", pieces[0].text);
    }

    #[test]
    fn an_excluded_range_splits_the_piece_at_the_gap() {
        let pieces = line_pieces("aa XX bb", std::slice::from_ref(&(3..5)));
        assert_eq!(2, pieces.len(), "{pieces:?}");
        assert_eq!("aa ", pieces[0].text);
        assert_eq!(" bb", pieces[1].text, "no match or splice may span the gap");
    }

    #[test]
    fn line_pieces_mark_code_contents_and_exclude_backticks() {
        let pieces = line_pieces("a `code` b", &[]);
        assert_eq!(3, pieces.len(), "{pieces:?}");
        assert_eq!(("code", true), (pieces[1].text.as_str(), pieces[1].code));
        assert_eq!(vec![3, 4, 5, 6], pieces[1].starts);
    }

    #[test]
    fn line_pieces_mark_math_source_and_exclude_dollar_delimiters() {
        let pieces = line_pieces("sum $x+y$ here", &[]);
        assert_eq!(3, pieces.len(), "{pieces:?}");
        assert_eq!(("x+y", true), (pieces[1].text.as_str(), pieces[1].math));
        assert_eq!(vec![5, 6, 7], pieces[1].starts);
        assert!(!pieces[0].math && !pieces[2].math);
    }
    use proptest::prelude::*;

    use super::*;

    fn inline_text() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 ,.;:!?]{1,16}"
    }

    fn inline_code_body() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 *_$]{1,24}"
    }

    fn formatted_text() -> impl Strategy<Value = String> {
        ("[a-zA-Z0-9]", "[a-zA-Z0-9 ,.;:!?]{0,14}", "[a-zA-Z0-9]")
            .prop_map(|(first, middle, last)| format!("{first}{middle}{last}"))
    }

    fn inline_fragment() -> impl Strategy<Value = (String, String)> {
        prop_oneof![
            4 => inline_text().prop_map(|text| (text.clone(), text)),
            2 => formatted_text().prop_map(|text| (format!("**{text}**"), text)),
            2 => formatted_text().prop_map(|text| (format!("*{text}*"), text)),
            2 => formatted_text().prop_map(|text| (format!("_{text}_"), text)),
            2 => inline_code_body().prop_map(|text| (format!("`{text}`"), text)),
            1 => prop::sample::select(vec!['*', '_', '$', '`', '\\'])
                .prop_map(|marker| (format!("\\{marker}"), marker.to_string())),
        ]
    }

    fn inline_source() -> impl Strategy<Value = (String, String)> {
        prop::collection::vec(inline_fragment(), 0..10).prop_map(|parts| {
            let mut source = String::new();
            let mut expected = String::new();
            for (index, (markup, content)) in parts.into_iter().enumerate() {
                if index > 0 {
                    source.push('|');
                    expected.push('|');
                }
                source.push_str(&markup);
                expected.push_str(&content);
            }
            (source, expected)
        })
    }

    fn math_atom() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("x".to_string()),
            Just("y".to_string()),
            Just("n".to_string()),
            Just("2".to_string()),
            Just("x^2".to_string()),
            Just(r"\alpha".to_string()),
            Just(r"\frac{1}{2}".to_string()),
        ]
    }

    fn math_operator() -> impl Strategy<Value = String> {
        prop_oneof![
            Just(" + ".to_string()),
            Just(" - ".to_string()),
            Just(r" \cdot ".to_string()),
        ]
    }

    fn math_formula() -> impl Strategy<Value = String> {
        (
            math_atom(),
            prop::collection::vec((math_operator(), math_atom()), 0..4),
        )
            .prop_map(|(first, rest)| {
                rest.into_iter().fold(first, |formula, (operator, atom)| {
                    format!("{formula}{operator}{atom}")
                })
            })
    }

    fn plain(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: false,
            italic: false,
            strike: false,
            code: false,
            math: None,
        }
    }

    fn bold(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: true,
            italic: false,
            strike: false,
            code: false,
            math: None,
        }
    }

    fn italic(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: false,
            italic: true,
            strike: false,
            code: false,
            math: None,
        }
    }

    fn code(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: false,
            italic: false,
            strike: false,
            code: true,
            math: None,
        }
    }

    fn strike(s: &str) -> InlineRun {
        InlineRun {
            text: s.to_string(),
            strike: true,
            ..InlineRun::default()
        }
    }

    fn strike_bold(s: &str) -> InlineRun {
        InlineRun {
            text: s.to_string(),
            strike: true,
            bold: true,
            ..InlineRun::default()
        }
    }

    fn bold_italic(s: &str) -> InlineRun {
        InlineRun {
            text: s.into(),
            bold: true,
            italic: true,
            strike: false,
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
    fn double_tilde_strikes_and_the_markers_leave_content() {
        assert_eq!(vec![strike("gone")], parse_inline("~~gone~~"));
        assert_eq!("gone", strip_inline("~~gone~~"));
    }

    #[test]
    fn a_strike_boundary_survives_run_merging() {
        assert_eq!(
            vec![strike("ab"), plain("cd")],
            parse_inline("~~ab~~cd"),
            "trailing plain text must not inherit the strike"
        );
        assert_eq!(
            vec![plain("ab"), strike("cd")],
            parse_inline("ab~~cd~~"),
            "a struck tail must not flatten into the plain head"
        );
    }

    #[test]
    fn strikethrough_nests_with_emphasis() {
        assert_eq!(
            vec![strike("a "), strike_bold("b"), strike(" c")],
            parse_inline("~~a **b** c~~"),
        );
    }

    #[test]
    fn other_tilde_runs_stay_literal() {
        for (label, input) in [
            ("a single pair", "~one~"),
            ("a triple pair", "~~~three~~~"),
            ("an unclosed opener", "a ~~ b"),
        ] {
            assert_eq!(vec![plain(input)], parse_inline(input), "{label}");
        }
    }

    #[test]
    fn an_escaped_tilde_breaks_the_strike_pair() {
        assert_eq!(vec![plain("~~kept~~")], parse_inline("\\~~kept~~"));
    }

    #[test]
    fn backslash_before_any_ascii_punctuation_yields_the_literal() {
        for ch in (0u8..=127)
            .map(char::from)
            .filter(char::is_ascii_punctuation)
        {
            let input = format!("a\\{ch}b");
            let expected = format!("a{ch}b");
            assert_eq!(
                vec![plain(&expected)],
                parse_inline(&input),
                "backslash-{ch} must yield the literal {ch}"
            );
        }
    }

    #[test]
    fn backslash_before_anything_else_stays_literal() {
        for (label, input, expected) in [
            ("a letter keeps \\blank intact", "\\blank", "\\blank"),
            ("a digit", "a\\1b", "a\\1b"),
            ("a space", "a\\ b", "a\\ b"),
            ("line end", "tail\\", "tail\\"),
        ] {
            assert_eq!(vec![plain(expected)], parse_inline(input), "{label}");
        }
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
    fn tag_shapes_error_and_their_neighbors_stay_legal() {
        let error_rows = [
            ("<div>", 1, "an open tag"),
            ("</div>", 1, "a close tag"),
            (
                "before <span class=x> after",
                8,
                "a mid-line tag with attributes",
            ),
            ("<B>", 1, "uppercase is still a letter"),
            ("<sub>unclosed", 1, "an unpaired subset open"),
            ("</sub>", 1, "a bare subset close"),
            ("<sub><sub>x</sub></sub>", 6, "a doubled subset open"),
            ("<sub><div></sub>", 6, "a tag inside a subset pair"),
            ("<SUB>x</SUB>", 1, "the subset is lowercase-exact"),
            ("<http//no-colon>", 1, "a malformed autolink near-miss"),
            ("<mailto:with space>", 1, "whitespace kills the uri form"),
            ("![d](<div", 6, "an unclosed destination is a near-miss"),
        ];
        for (text, column, why) in error_rows {
            assert_eq!(tag_shape_column(text), Some(column), "{why}: {text}");
        }
        let legal_rows = [
            ("a < b", "spaced comparison"),
            ("a<3 and 2<4", "digits after the bracket"),
            ("x <= y", "an operator"),
            ("<", "a lone bracket at line end"),
            (r"\<div>", "the escaped bracket"),
            ("`<div>`", "a code span"),
            ("``<div> and <span>``", "a double-backtick code span"),
            ("<!-- cards -->", "the comment channel"),
            (
                "text <!-- <div> inside --> tail",
                "a tag inside a closed comment",
            ),
            ("text <!-- <div> unclosed", "a tag inside an open comment"),
            ("<https://alix.study/deck>", "a uri autolink"),
            ("<mailto:hi@alix.study>", "a mailto autolink"),
            ("<hi@alix.study>", "an email autolink"),
            ("<sub>2</sub>", "a paired subscript"),
            ("<sup>2</sup>", "a paired superscript"),
            ("<ins>new</ins>", "a paired insertion"),
            (
                "H<sub>2</sub>O and E=mc<sup>2</sup>",
                "two pairs on one line",
            ),
            ("<sub>i<sup>2</sup></sub>", "nested distinct subset pairs"),
            ("![d](<old image.png>)", "an image destination in angles"),
            ("[t](<a b c>)", "a link destination in angles"),
        ];
        for (text, why) in legal_rows {
            assert_eq!(tag_shape_column(text), None, "{why}: {text}");
        }
    }

    #[test]
    fn backtick_anchored_inline_math_renders_like_bare_dollars() {
        let runs = parse_inline("Why does $`a^2 + b^2 = c^2`$ hold?");
        assert_eq!(runs.len(), 3, "prose, math, prose: {runs:?}");
        assert_eq!(runs[1].text, "a^2 + b^2 = c^2");
        let math = runs[1].math.as_ref().unwrap();
        assert!(!math.display);
        assert!(
            math.svg
                .as_deref()
                .is_some_and(|svg| svg.starts_with("<svg"))
        );
        assert_eq!(
            strip_inline("Why does $`a^2 + b^2 = c^2`$ hold?"),
            "Why does a^2 + b^2 = c^2 hold?",
            "the four anchor chars are markers, not content"
        );
        let styled = parse_inline("$`**x**`$");
        assert_eq!(styled.len(), 1);
        assert_eq!(styled[0].text, "**x**", "the body is verbatim math source");
        assert!(styled[0].math.is_some());
        assert!(!styled[0].bold, "math is protected from emphasis");
    }

    #[test]
    fn backtick_anchored_math_boundaries_stay_literal() {
        assert_eq!(parse_inline("$`x"), vec![plain("$`x")]);
        for text in ["$``$", "$` `$", "$`x` y", r"\$`x`$"] {
            let runs = parse_inline(text);
            assert!(
                runs.iter().all(|run| run.math.is_none()),
                "an empty or whitespace anchored body is not math: {runs:?}"
            );
            assert!(
                runs.first().is_some_and(|run| run.text.starts_with('$')),
                "the dollars stay literal text: {runs:?}"
            );
        }
        let code_first = parse_inline("`a $` b `$ c`");
        assert!(
            code_first.iter().all(|run| run.math.is_none()),
            "a code span opening before the dollar keeps priority: {code_first:?}"
        );
    }

    #[test]
    fn a_marker_starting_at_the_closing_delimiter_is_outside_math() {
        assert!(!math_encloses("$x$outside", "$outside"));
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

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn projected_runs_are_a_normalized_content_projection(
            (source, expected) in inline_source()
        ) {
            let runs = parse_inline(&source);
            let projected: String = runs.iter().map(|run| run.text.as_str()).collect();
            let stripped = strip_inline(&source);

            prop_assert_eq!(
                projected.as_str(),
                expected.as_str(),
                "source: {:?}; runs: {:?}",
                source,
                runs
            );
            prop_assert_eq!(
                stripped.as_str(),
                expected.as_str(),
                "source: {:?}",
                source
            );
            prop_assert!(
                runs.iter().all(|run| !run.text.is_empty()),
                "empty run for source: {source:?}; runs: {runs:?}"
            );
            for pair in runs.windows(2) {
                let mergeable = pair[0].math.is_none()
                    && pair[1].math.is_none()
                    && (pair[0].bold, pair[0].italic, pair[0].code)
                        == (pair[1].bold, pair[1].italic, pair[1].code);
                prop_assert!(
                    !mergeable,
                    "adjacent runs should have coalesced for source: {source:?}; pair: {pair:?}"
                );
            }
        }

        #[test]
        fn inline_code_preserves_generated_markup_verbatim(body in inline_code_body()) {
            let source = format!("`{body}`");
            prop_assert_eq!(vec![code(&body)], parse_inline(&source));
        }

        #[test]
        fn escaping_preserves_a_generated_marker_as_plain_text(
            marker in prop::sample::select(vec!['*', '_', '$', '`', '\\']),
            word in "[a-zA-Z]{1,12}",
        ) {
            let source = format!("\\{marker}{word}\\{marker}");
            let expected = format!("{marker}{word}{marker}");
            prop_assert_eq!(vec![plain(&expected)], parse_inline(&source));
        }

        #[test]
        fn generated_math_keeps_its_source_as_one_rendered_run(formula in math_formula()) {
            let source = format!("before ${formula}$ after");
            let runs = parse_inline(&source);
            let math_runs: Vec<&InlineRun> =
                runs.iter().filter(|run| run.math.is_some()).collect();

            prop_assert_eq!(
                1,
                math_runs.len(),
                "source: {:?}; runs: {:?}",
                source,
                runs
            );
            prop_assert_eq!(formula.as_str(), math_runs[0].text.as_str());
            prop_assert!(
                math_runs[0].math.as_ref().and_then(|math| math.svg.as_ref()).is_some(),
                "generated formula did not render: {formula:?}; run: {:?}",
                math_runs[0]
            );
            prop_assert_eq!(
                format!("before {formula} after"),
                strip_inline(&source)
            );
        }

        #[test]
        fn generated_math_boundaries_classify_the_same_marker_inside_and_outside(
            formula in math_formula()
        ) {
            let marker = crate::parser::BLANK;
            let inside = format!("${formula} + {marker}$");
            let outside = format!("${formula}$ + {marker}");
            let code = format!("`${formula} + {marker}$`");

            prop_assert!(math_encloses(&inside, marker), "inside: {inside:?}");
            prop_assert!(!math_encloses(&outside, marker), "outside: {outside:?}");
            prop_assert!(!math_encloses(&code, marker), "code: {code:?}");
        }
    }
}
