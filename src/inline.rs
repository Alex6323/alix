use serde::{Deserialize, Serialize};

use crate::{
    entities::entity_at,
    math::{MathRenderer, MathView},
};

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
    /// An inert autolink: the URL is both label and content; clients
    /// style it as a link but attach no navigation.
    #[serde(default, skip_serializing_if = "is_false")]
    pub link: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sub: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sup: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ins: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub math: Option<MathView>,
}

#[derive(Default)]
pub struct DisplayProjector {
    renderer: MathRenderer,
    definitions: Option<LinkDefinitions>,
}

impl DisplayProjector {
    pub fn with_definitions<I, S>(labels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            definitions: Some(LinkDefinitions::new(labels)),
            ..Self::default()
        }
    }

    pub fn project(&mut self, text: &str) -> Vec<InlineRun> {
        project_text(
            text,
            Some(&mut self.renderer),
            false,
            self.definitions.as_ref(),
        )
    }

    pub fn project_context(&mut self, text: &str) -> Vec<InlineRun> {
        project_text(
            text,
            Some(&mut self.renderer),
            true,
            self.definitions.as_ref(),
        )
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
    link: bool,
    subset: Option<usize>,
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

pub fn parse_inline_with(text: &str, definitions: &LinkDefinitions) -> Vec<InlineRun> {
    project_text(text, None, false, Some(definitions))
}

pub fn strip_inline(text: &str) -> String {
    project_text(text, None, false, None)
        .into_iter()
        .map(|run| run.text)
        .collect()
}

pub fn strip_inline_with(text: &str, definitions: &LinkDefinitions) -> String {
    parse_inline_with(text, definitions)
        .into_iter()
        .map(|run| run.text)
        .collect()
}

/// A deck's link-definition labels, folded once for reference matching.
pub struct LinkDefinitions(std::collections::HashSet<String>);

impl LinkDefinitions {
    pub fn new<I, S>(labels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self(
            labels
                .into_iter()
                .map(|label| fold_label(label.as_ref()))
                .collect(),
        )
    }

    fn contains(&self, candidate: &str) -> bool {
        self.0.contains(&fold_label(candidate))
    }
}

/// CommonMark label matching: case-insensitive with interior whitespace
/// collapsed.
fn fold_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
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
    definitions: Option<&LinkDefinitions>,
) -> Vec<InlineRun> {
    let mut runs = Vec::new();
    for chunk in text.split_inclusive('\n') {
        let (line, newline) = chunk
            .strip_suffix('\n')
            .map_or((chunk, false), |line| (line, true));
        let mut line_runs = project_line(line, renderer.as_deref_mut(), context, definitions);
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

fn classify_line(
    text: &str,
    style_links: bool,
    definitions: Option<&LinkDefinitions>,
) -> LineClassification {
    let chars: Vec<char> = text.chars().collect();
    let spans = math_spans(&chars);
    let mut glyphs = scan_glyphs(&chars, &spans);
    // The display drops link syntax and styles the label; the maskable
    // stream keeps the raw glyphs and applies `link_syntax_mask` itself,
    // because its gap rule needs the dropped spans still present.
    let mut labels = Vec::new();
    if style_links {
        let links = bracket_links(&glyphs, definitions);
        let mut dropped = vec![false; glyphs.len()];
        let mut label = vec![false; glyphs.len()];
        for link in &links {
            dropped[link.open] = true;
            dropped[link.close..=link.syntax_end]
                .iter_mut()
                .for_each(|flag| *flag = true);
            label[link.open + 1..link.close]
                .iter_mut()
                .for_each(|flag| *flag = true);
        }
        let mut kept = Vec::with_capacity(glyphs.len());
        for (index, glyph) in glyphs.into_iter().enumerate() {
            if dropped[index] {
                continue;
            }
            if label[index] {
                labels.push(kept.len());
            }
            kept.push(glyph);
        }
        glyphs = kept;
    }
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
    // After emphasis: a link-marked glyph never delimits (the autolink
    // rule), but a bracket label takes emphasis like any prose.
    for index in labels {
        glyphs[index].link = true;
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
    definitions: Option<&LinkDefinitions>,
) -> Vec<InlineRun> {
    let LineClassification {
        spans,
        glyphs,
        bold,
        italic,
        strike,
        removed,
    } = classify_line(text, true, definitions);
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
                link: glyph.link,
                sub: glyph.subset == Some(0),
                sup: glyph.subset == Some(1),
                ins: glyph.subset == Some(2),
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
    } = classify_line(text, false, None);
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

struct BracketLink {
    open: usize,
    close: usize,
    syntax_end: usize,
}

/// Finds every complete link span in the glyph stream: the inline
/// `[label](destination)` form always, and with a definition table the
/// reference forms `[label][name]`, `[label][]`, and `[label]` whose
/// folded name is defined. An unmatched pattern is ordinary prose. One
/// grammar, two consumers: `link_syntax_mask` drops the syntax from the
/// maskable stream, and `classify_line` drops it from the display while
/// styling the label.
fn bracket_links(glyphs: &[Glyph], definitions: Option<&LinkDefinitions>) -> Vec<BracketLink> {
    let plain = |glyph: &Glyph| !glyph.escaped && !glyph.code && glyph.math.is_none();
    let bracket_close = |from: usize| {
        (from..glyphs.len())
            .find(|&at| plain(&glyphs[at]) && matches!(glyphs[at].ch, ']' | '['))
            .filter(|&at| glyphs[at].ch == ']')
    };
    let mut links = Vec::new();
    let mut index = 0;
    while index < glyphs.len() {
        if !(plain(&glyphs[index]) && glyphs[index].ch == '[') {
            index += 1;
            continue;
        }
        let Some(close) = bracket_close(index + 1) else {
            index += 1;
            continue;
        };
        if glyphs.get(close + 1).map(|glyph| glyph.ch) == Some('(') {
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
            links.push(BracketLink {
                open: index,
                close,
                syntax_end: close_paren,
            });
            index = close_paren + 1;
            continue;
        }
        let Some(definitions) = definitions else {
            index = close + 1;
            continue;
        };
        let label: String = glyphs[index + 1..close]
            .iter()
            .map(|glyph| glyph.ch)
            .collect();
        if glyphs.get(close + 1).map(|glyph| glyph.ch) == Some('[') {
            let Some(reference_close) = bracket_close(close + 2) else {
                index = close + 1;
                continue;
            };
            let reference: String = glyphs[close + 2..reference_close]
                .iter()
                .map(|glyph| glyph.ch)
                .collect();
            let name = if reference.trim().is_empty() {
                &label
            } else {
                &reference
            };
            if definitions.contains(name) {
                links.push(BracketLink {
                    open: index,
                    close,
                    syntax_end: reference_close,
                });
                index = reference_close + 1;
            } else {
                index = close + 1;
            }
            continue;
        }
        if definitions.contains(&label) {
            links.push(BracketLink {
                open: index,
                close,
                syntax_end: close,
            });
        }
        index = close + 1;
    }
    links
}

/// Marks the glyphs of every complete `[label](destination)` link: the
/// brackets, the parentheses, and the destination drop from the maskable
/// stream while the label stays visible text.
fn link_syntax_mask(glyphs: &[Glyph]) -> Vec<bool> {
    let mut mask = vec![false; glyphs.len()];
    for link in bracket_links(glyphs, None) {
        mask[link.open] = true;
        mask[link.close..=link.syntax_end]
            .iter_mut()
            .for_each(|dropped| *dropped = true);
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
            previous.link,
            previous.sub,
            previous.sup,
            previous.ins,
        ) == (
            run.bold, run.italic, run.strike, run.code, run.link, run.sub, run.sup, run.ins,
        )
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
/// Precedence per angle run: image destination, autolink, styled
/// subset, tag-shape, literal prose. A whole-line comment is the
/// line-level `<!-- -->` channel and stays unexamined; a mid-line
/// `<!--` is literal prose (`!` is not a letter). None means the
/// line is legal.
pub(crate) fn tag_shape_column(text: &str) -> Option<usize> {
    let trimmed =
        text.trim_matches(|ch: char| matches!(ch, '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' '));
    if let Some(body) = trimmed
        .strip_prefix("<!--")
        .and_then(|rest| rest.strip_suffix("-->"))
        && !body.contains("-->")
    {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    angle_scan(&chars).error_column
}

/// A completed styled-subset pair: the tag glyph ranges are markers,
/// the span between them is the styled interior.
struct SubsetSpan {
    slot: usize,
    open_start: usize,
    interior_start: usize,
    interior_end: usize,
    close_end: usize,
}

#[derive(Default)]
struct AngleScan {
    error_column: Option<usize>,
    subsets: Vec<SubsetSpan>,
}

/// One walk, two consumers: the parser's tag-shape error and the
/// display projection's subset styling read the same angle grammar.
fn angle_scan(chars: &[char]) -> AngleScan {
    let mut scan = AngleScan::default();
    let mut open_subset: Option<(usize, usize)> = None;
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' && !is_escaped(chars, index) {
            index = code_span_end(chars, index);
            continue;
        }
        if chars[index] != '<' || is_escaped(chars, index) {
            index += 1;
            continue;
        }
        if let Some(close) = image_destination_end(chars, index) {
            index = close + 1;
            continue;
        }
        if let Some(end) = autolink_end(chars, index) {
            index = end + 1;
            continue;
        }
        if let Some((slot, len, closing)) = subset_tag(chars, index) {
            match (open_subset, closing) {
                (Some((open_slot, open_start)), true) if open_slot == slot => {
                    open_subset = None;
                    scan.subsets.push(SubsetSpan {
                        slot,
                        open_start,
                        interior_start: open_start + slot_open_len(slot),
                        interior_end: index,
                        close_end: index + len,
                    });
                    index += len;
                    continue;
                }
                (None, false) => {
                    open_subset = Some((slot, index));
                    index += len;
                    continue;
                }
                _ => {
                    scan.error_column = Some(index + 1);
                    return scan;
                }
            }
        }
        let tag = match chars.get(index + 1) {
            Some('/') => chars
                .get(index + 2)
                .is_some_and(|ch| ch.is_ascii_alphabetic()),
            Some(ch) => ch.is_ascii_alphabetic(),
            None => false,
        };
        if tag {
            scan.error_column = Some(index + 1);
            return scan;
        }
        index += 1;
    }
    scan.error_column = open_subset.map(|(_, start)| start + 1);
    scan
}

fn slot_open_len(slot: usize) -> usize {
    SUBSET_TAGS[slot].len() + 2
}

/// The closing `>` of a complete image destination: `start` sits on the
/// `<` of `![label](<...>`, and the span must end `>)` to be exempt,
/// mirroring the parser's image grammar (label is the nearest `![` left
/// of the `]` with no `]` between; `\<` `\>` `\\` escape inside; a raw
/// `<` breaks the form).
fn image_destination_end(chars: &[char], start: usize) -> Option<usize> {
    if start < 2 || chars[start - 1] != '(' || chars[start - 2] != ']' {
        return None;
    }
    let mut label = start - 2;
    loop {
        if label == 0 {
            return None;
        }
        label -= 1;
        match chars[label] {
            ']' => return None,
            '[' if label >= 1 && chars[label - 1] == '!' && !is_escaped(chars, label - 1) => break,
            _ => {}
        }
    }
    let mut index = start + 1;
    while index < chars.len() {
        match chars[index] {
            '\\' if chars
                .get(index + 1)
                .is_some_and(|ch| matches!(ch, '<' | '>' | '\\')) =>
            {
                index += 2;
            }
            '<' => return None,
            '>' => return (chars.get(index + 1) == Some(&')')).then_some(index),
            _ => index += 1,
        }
    }
    None
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
    let mut subset_marker = vec![false; chars.len()];
    let mut subset_slot: Vec<Option<usize>> = vec![None; chars.len()];
    for span in angle_scan(chars).subsets {
        subset_marker[span.open_start..span.interior_start].fill(true);
        subset_marker[span.interior_end..span.close_end].fill(true);
        subset_slot[span.interior_start..span.interior_end].fill(Some(span.slot));
    }
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
                link: false,
                subset: None,
                math: Some(span_index),
            });
            index += 1;
            continue;
        }
        if subset_marker[index] {
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
                link: false,
                subset: subset_slot[index + 1],
                math: None,
            });
            index += 2;
            continue;
        }
        if chars[index] == '<'
            && let Some(close) = autolink_end(chars, index)
            && (index..=close).all(|inner| math[inner].is_none() && !removed[inner])
        {
            glyphs.extend((index + 1..close).map(|raw_index| Glyph {
                ch: chars[raw_index],
                raw_index,
                escaped: false,
                code: false,
                link: true,
                subset: subset_slot[raw_index],
                math: None,
            }));
            index = close + 1;
            continue;
        }
        if chars[index] == '`' {
            let run = chars[index..].iter().take_while(|ch| **ch == '`').count();
            let end = code_span_end(chars, index);
            if end > index + run {
                glyphs.extend((index + run..end - run).map(|raw_index| Glyph {
                    ch: chars[raw_index],
                    raw_index,
                    escaped: true,
                    code: true,
                    link: false,
                    subset: None,
                    math: None,
                }));
            } else {
                glyphs.extend((index..index + run).map(|raw_index| Glyph {
                    ch: chars[raw_index],
                    raw_index,
                    escaped: false,
                    code: false,
                    link: false,
                    subset: subset_slot[raw_index],
                    math: None,
                }));
            }
            index = end;
            continue;
        }
        if chars[index] == '&' {
            // 40 chars covers every entity: the longest name is 31, plus `&;`.
            let bound = (index + 40).min(chars.len());
            let tail: String = chars[index..bound].iter().collect();
            if let Some((consumed, replacement)) = entity_at(&tail) {
                glyphs.extend(replacement.chars().map(|ch| Glyph {
                    ch,
                    raw_index: index,
                    escaped: true,
                    code: false,
                    link: false,
                    subset: subset_slot[index],
                    math: None,
                }));
                // entity source is ASCII, so its byte length is its char count
                index += consumed;
                continue;
            }
        }
        glyphs.push(Glyph {
            ch: chars[index],
            raw_index: index,
            escaped: false,
            code: false,
            link: false,
            subset: subset_slot[index],
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
            || glyph.link
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
            2 => formatted_text().prop_map(|text| (format!("~~{text}~~"), text)),
            1 => "[a-z0-9]{1,8}"
                .prop_map(|host| (format!("<https://{host}.io>"), format!("https://{host}.io"))),
            1 => (
                "[a-zA-Z]{1,8}",
                prop::sample::select(vec!["https://alix.study", "#anchor", "guide/ch3.md"]),
            )
                .prop_map(|(label, dest)| (format!("[{label}]({dest})"), label)),
            1 => (
                prop::sample::select(vec!["sub", "sup", "ins"]),
                formatted_text(),
            )
                .prop_map(|(tag, text)| (format!("<{tag}>{text}</{tag}>"), text)),
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
            link: false,
            sub: false,
            sup: false,
            ins: false,
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
            link: false,
            sub: false,
            sup: false,
            ins: false,
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
            link: false,
            sub: false,
            sup: false,
            ins: false,
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
            link: false,
            sub: false,
            sup: false,
            ins: false,
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
            link: false,
            sub: false,
            sup: false,
            ins: false,
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
            (
                "<sub>i<sup>2</sup></sub>",
                7,
                "a nested distinct subset is a nested form",
            ),
            (
                "<sub><sup>x</sub></sup>",
                6,
                "cross-nesting dies at the inner opener",
            ),
            ("<sub>x</sup>", 7, "a mismatched subset close"),
            (
                "text <!-- <div> inside --> tail",
                11,
                "a tag inside inline comment prose",
            ),
            (
                "text <!-- <div> unclosed",
                11,
                "an unclosed inline comment hides nothing",
            ),
            (
                "<!-- a --> <div> <!-- b -->",
                12,
                "a tag between two comments on one line",
            ),
            ("![d](<d.png>x)", 6, "a destination not sealed by the paren"),
            (r"![d](<a\>b)", 6, "an escaped close leaves the form open"),
            (
                "[t](<a b c>)",
                5,
                "a link destination is not the image form",
            ),
            (r"\![d](<x.png>)", 7, "an escaped image marker is prose"),
            ("![a]b](<x.png>)", 8, "a second bracket breaks the label"),
        ];
        for (text, column, why) in error_rows {
            assert_eq!(tag_shape_column(text), Some(column), "{why}: {text}");
        }
        let legal_rows = [
            ("a < b", "spaced comparison"),
            ("a<3 and 2<4", "digits after the bracket"),
            ("&lt;div&gt;", "an angle entity is never a tag shape"),
            ("x <= y", "an operator"),
            ("<", "a lone bracket at line end"),
            (r"\<div>", "the escaped bracket"),
            ("`<div>`", "a code span"),
            ("``<div> and <span>``", "a double-backtick code span"),
            ("<!-- cards -->", "the comment channel"),
            (
                "<!-- <div> disabled -->",
                "a whole-line comment is the channel",
            ),
            ("  <!-- <q>? -->", "an indented channel line"),
            (
                "text <!-- note --> tail",
                "inline comment prose without a tag",
            ),
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
            ("![d](<old image.png>)", "an image destination in angles"),
        ];
        for (text, why) in legal_rows {
            assert_eq!(tag_shape_column(text), None, "{why}: {text}");
        }
    }

    #[test]
    fn subset_pairs_render_as_styled_runs_without_their_tags() {
        let runs = parse_inline("H<sub>2</sub>O and E=mc<sup>2</sup>");
        let flat: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(flat, "H2O and E=mc2", "tag glyphs are markers, not content");
        assert!(
            runs.iter().any(|run| run.sub && run.text == "2"),
            "{runs:?}"
        );
        assert!(
            runs.iter().any(|run| run.sup && run.text == "2"),
            "{runs:?}"
        );
        let ins = parse_inline("<ins>new</ins>");
        assert_eq!(ins.len(), 1, "{ins:?}");
        assert!(ins[0].ins && !ins[0].sub && !ins[0].sup);
        assert_eq!(ins[0].text, "new");
        assert_eq!(
            strip_inline("H<sub>2</sub>O"),
            "H2O",
            "grading sees inner text"
        );
        assert_eq!(
            strip_inline("Tom &amp; Jerry"),
            "Tom & Jerry",
            "grading equates an authored entity with the typed literal"
        );
    }

    #[test]
    fn subset_interiors_keep_normal_inline_scanning() {
        let styled = parse_inline("<sub>**x**</sub>");
        assert_eq!(styled.len(), 1, "{styled:?}");
        assert!(
            styled[0].sub && styled[0].bold,
            "emphasis works inside a pair"
        );
        assert_eq!(styled[0].text, "x");
        let coded = parse_inline("`<sub>x</sub>`");
        assert_eq!(coded.len(), 1, "{coded:?}");
        assert!(
            coded[0].code && !coded[0].sub,
            "a code span keeps the tags literal"
        );
        assert_eq!(coded[0].text, "<sub>x</sub>");
        let unpaired = parse_inline("<sub>unclosed");
        assert!(
            unpaired.iter().all(|run| !run.sub),
            "an unpaired open stays literal in projection: {unpaired:?}"
        );
        let flat: String = unpaired.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(flat, "<sub>unclosed");
    }

    #[test]
    fn autolinks_render_as_inert_link_runs() {
        let runs = parse_inline("see <https://alix.study/deck> now");
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert_eq!(runs[0], plain("see "));
        assert_eq!(
            runs[1].text, "https://alix.study/deck",
            "the brackets are markers, not content"
        );
        assert!(runs[1].link && !runs[1].bold && !runs[1].italic && !runs[1].code);
        assert_eq!(runs[2], plain(" now"));

        let email = parse_inline("<hi@alix.study>");
        assert_eq!(email.len(), 1, "{email:?}");
        assert!(email[0].link);
        assert_eq!(email[0].text, "hi@alix.study");
    }

    #[test]
    fn autolink_boundaries_stay_literal_or_protected() {
        for (text, why) in [
            (
                r"\<https://alix.study>",
                "an escaped bracket kills the form",
            ),
            ("<http//no-colon>", "a near-miss stays prose"),
            ("a < b and x<3", "non-tag brackets unaffected"),
        ] {
            let runs = parse_inline(text);
            assert!(runs.iter().all(|run| !run.link), "{why}: {runs:?}");
        }
        let code = parse_inline("`<https://alix.study>`");
        assert_eq!(code.len(), 1, "{code:?}");
        assert!(
            code[0].code && !code[0].link,
            "a code span keeps the brackets"
        );
        assert_eq!(code[0].text, "<https://alix.study>");
        let underscored = parse_inline("<https://a_b_c.io>");
        assert_eq!(underscored.len(), 1, "{underscored:?}");
        assert!(
            underscored[0].link && !underscored[0].italic,
            "URL underscores are not emphasis"
        );
        assert_eq!(underscored[0].text, "https://a_b_c.io");
    }

    #[test]
    fn bracket_links_render_as_styled_labels_with_inert_destinations() {
        let runs = parse_inline("see [the docs](https://alix.study) now");
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert_eq!(runs[0], plain("see "));
        assert_eq!(
            runs[1].text, "the docs",
            "the label is the content; brackets, parens, and destination drop"
        );
        assert!(runs[1].link && !runs[1].bold && !runs[1].italic && !runs[1].code);
        assert_eq!(runs[2], plain(" now"));

        for (source, label, why) in [
            (
                "[a](#anchor)",
                "a",
                "an anchor destination styles like any link",
            ),
            (
                "[a](guide/ch3.md)",
                "a",
                "a relative destination is the same display",
            ),
        ] {
            let runs = parse_inline(source);
            assert_eq!(runs.len(), 1, "{why}: {runs:?}");
            assert!(runs[0].link, "{why}");
            assert_eq!(runs[0].text, label, "{why}");
        }

        let inner = parse_inline("[*x*](d)");
        assert_eq!(inner.len(), 1, "{inner:?}");
        assert!(
            inner[0].link && inner[0].italic,
            "emphasis inside a label still renders"
        );
        assert_eq!(inner[0].text, "x");

        let across = parse_inline("*a [b](c) d*");
        assert!(
            across.iter().all(|run| run.italic),
            "emphasis spans across a link: {across:?}"
        );
        assert!(
            across.iter().any(|run| run.link && run.text == "b"),
            "the label inside keeps its link styling: {across:?}"
        );
    }

    #[test]
    fn bracket_link_boundaries_stay_literal_or_protected() {
        for (text, why) in [
            (r"\[a](b)", "an escaped opening bracket kills the form"),
            ("just [brackets] here", "no destination means prose"),
            ("[a] (b)", "a space before the parens breaks the form"),
        ] {
            let runs = parse_inline(text);
            assert!(runs.iter().all(|run| !run.link), "{why}: {runs:?}");
            let joined: String = runs.iter().map(|run| run.text.as_str()).collect();
            assert_eq!(joined, text.replace('\\', ""), "{why}");
        }
        let spans = parse_inline("`[a](b)`");
        assert_eq!(vec![code("[a](b)")], spans, "a code span keeps the syntax");
    }

    #[test]
    fn typed_grading_compares_the_link_label_alone() {
        assert_eq!(
            strip_inline("see [the docs](https://alix.study)"),
            "see the docs"
        );
    }

    #[test]
    fn reference_links_render_as_styled_labels_when_their_label_is_defined() {
        let defs = LinkDefinitions::new(["r", "Spaced  Label"]);
        for (source, label, why) in [
            (
                "see [the ref][r] here",
                "the ref",
                "the full form styles its text",
            ),
            ("see [r][] here", "r", "the collapsed form is its own label"),
            ("see [r] here", "r", "the shortcut form is its own label"),
            ("see [R] here", "R", "labels match by case folding"),
            (
                "see [spaced label] here",
                "spaced label",
                "interior whitespace collapses before matching",
            ),
        ] {
            let runs = parse_inline_with(source, &defs);
            assert_eq!(runs.len(), 3, "{why}: {runs:?}");
            assert_eq!(runs[0], plain("see "));
            assert!(runs[1].link, "{why}");
            assert_eq!(runs[1].text, label, "{why}");
            assert_eq!(runs[2], plain(" here"));
            assert_eq!(
                strip_inline_with(source, &defs),
                format!("see {label} here"),
                "{why}: grading follows the same projection"
            );
        }

        for (source, why) in [
            ("see [nope] here", "an undefined shortcut stays prose"),
            (
                "see [text][nope] here",
                "an undefined reference keeps the whole form prose",
            ),
            (
                "see [nope][] here",
                "an undefined collapsed form stays prose",
            ),
        ] {
            let runs = parse_inline_with(source, &defs);
            assert!(runs.iter().all(|run| !run.link), "{why}: {runs:?}");
            let joined: String = runs.iter().map(|run| run.text.as_str()).collect();
            assert_eq!(joined, source, "{why}");
        }

        let inline_wins = parse_inline_with("[r](https://alix.study)", &defs);
        assert_eq!(inline_wins.len(), 1, "{inline_wins:?}");
        assert_eq!(
            inline_wins[0].text, "r",
            "the inline form owns its destination even when the label is defined"
        );

        let bare = parse_inline("just [r] alone");
        assert!(
            bare.iter().all(|run| !run.link),
            "without definitions nothing changes: {bare:?}"
        );
    }

    #[test]
    fn a_projector_carries_its_deck_definitions_into_every_projection() {
        let mut projector = DisplayProjector::with_definitions(["r"]);
        let runs = projector.project("see [the ref][r] here");
        assert!(
            runs.iter().any(|run| run.link && run.text == "the ref"),
            "{runs:?}"
        );
        let context = projector.project_context("[r] leads");
        assert!(
            context.iter().any(|run| run.link && run.text == "r"),
            "{context:?}"
        );
    }

    #[test]
    fn an_autolink_inside_a_double_backtick_code_span_stays_code() {
        assert_eq!(
            vec![code("<https://alix.study>")],
            parse_inline("``<https://alix.study>``"),
            "the code-span delimiter owns its body before autolink recognition"
        );
    }

    #[test]
    fn entities_decode_in_the_projection_but_never_become_syntax() {
        for (source, expected, why) in [
            ("Tom &amp; Jerry", "Tom & Jerry", "a named entity decodes"),
            ("A&#65;B", "AAB", "a decimal entity decodes"),
            ("A&#x41;B", "AAB", "a hex entity decodes"),
            (
                "&bogus; &#; &#xG; &",
                "&bogus; &#; &#xG; &",
                "invalid forms stay literal",
            ),
            (r"\&amp;", "&amp;", "an escaped ampersand kills the entity"),
            (
                "&#42;x&#42;",
                "*x*",
                "a decoded marker is content, not emphasis",
            ),
        ] {
            let runs = parse_inline(source);
            let projected: String = runs.iter().map(|run| run.text.as_str()).collect();
            assert_eq!(expected, projected, "{why}: {source:?}");
            assert!(
                runs.iter().all(|run| !run.bold && !run.italic),
                "{why}: no styling from decoded characters: {runs:?}"
            );
            assert_eq!(expected, strip_inline(source), "{why} (strip): {source:?}");
        }
        assert_eq!(
            vec![code("&amp;")],
            parse_inline("`&amp;`"),
            "a code span keeps the entity literal"
        );
        let math = parse_inline("$a &amp; b$");
        assert_eq!(math.len(), 1, "{math:?}");
        assert_eq!(
            math[0].text, "a &amp; b",
            "math source is verbatim, entities included"
        );
        let angles = parse_inline("&lt;div&gt;");
        let projected: String = angles.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(
            "<div>", projected,
            "angle entities decode to inert brackets"
        );
    }

    #[test]
    fn code_span_runs_close_on_their_exact_length_in_the_projection() {
        for (source, expected, why) in [
            ("`x`", vec![code("x")], "a single-backtick span"),
            ("``x``", vec![code("x")], "a double-backtick span"),
            ("```x```", vec![code("x")], "a triple-backtick span"),
            (
                "``a`b``",
                vec![code("a`b")],
                "a shorter run inside the body stays body",
            ),
            (
                "`` `x` ``",
                vec![code(" `x` ")],
                "a fully backtick-wrapped body stays body",
            ),
            (
                "a``b",
                vec![plain("a``b")],
                "an unmatched double run stays literal text",
            ),
            (
                "`a``",
                vec![plain("`a``")],
                "a longer run never closes a shorter opener",
            ),
        ] {
            assert_eq!(expected, parse_inline(source), "{why}: {source:?}");
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
                    && (
                        pair[0].bold,
                        pair[0].italic,
                        pair[0].strike,
                        pair[0].code,
                        pair[0].link,
                        pair[0].sub,
                        pair[0].sup,
                        pair[0].ins,
                    ) == (
                        pair[1].bold,
                        pair[1].italic,
                        pair[1].strike,
                        pair[1].code,
                        pair[1].link,
                        pair[1].sub,
                        pair[1].sup,
                        pair[1].ins,
                    );
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
