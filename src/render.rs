use serde::{Deserialize, Serialize};

use crate::{
    card::Card,
    inline::DisplayProjector,
    parser::{BLANK, HIDDEN},
};

// Struct variants (not newtype) because serde's internal tagging can't tag
// newtype variants; wire shape is `{"kind": ..., "text": ...}` (docs/API.md).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ContentUnit {
    Sentence {
        text: String,
        runs: Vec<crate::inline::InlineRun>,
    },
    Code {
        lines: Vec<String>,
    },
    /// A frozen mermaid fence, rendered: occupies the fence's own slot in
    /// the stream. `src` is an absolute file path in the library projection;
    /// the HTTP layer rewrites it to a `/img/<key>` URL. `width`/`height`
    /// are LOGICAL pixels (the raster is 2x); `alt` carries the fence
    /// source, the accessible representation while nothing is masked.
    Diagram {
        src: String,
        width: u32,
        height: u32,
        alt: String,
        /// Overlay regions in RASTER pixel space (the PNG's own pixels, the
        /// space `naturalWidth` placement uses); absent on an unmasked
        /// diagram, so the committed 6a wire shape stands.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        regions: Vec<crate::review::RegionView>,
        /// The post-answer accessible text: asked labels revealed, sibling
        /// and cover labels still masked. Absent when nothing is masked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revealed_alt: Option<String>,
    },
    Checklist {
        items: Vec<ChecklistItem>,
    },
    /// A bare pipe table (one no mapping claimed), aligned per the GFM
    /// delimiter-row colons. Rows are padded or truncated to the header
    /// width for display; only the mapped-card table enforces widths.
    Table {
        aligns: Vec<CellAlign>,
        header: Vec<Vec<crate::inline::InlineRun>>,
        rows: Vec<Vec<Vec<crate::inline::InlineRun>>>,
    },
    /// A bare blockquote run: quoted content, never a note. It carries its own
    /// units so a quotation can hold prose, code, or a list, and it is one
    /// block rather than a sequence of gradeable lines.
    Quote {
        units: Vec<ContentUnit>,
    },
}

/// One step of an answer as a client walks it: a line the learner must
/// produce, or a quotation run that reveals as one block and is never typed.
/// A quote carries its own units so a client never parses quote syntax, and
/// spans are half-open into the DISPLAYED back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AnswerStep {
    Line {
        back_from: usize,
        back_to: usize,
    },
    Quote {
        back_from: usize,
        back_to: usize,
        units: Vec<ContentUnit>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CellAlign {
    None,
    Left,
    Center,
    Right,
}

fn cell_align(delimiter_cell: &str) -> CellAlign {
    let left = delimiter_cell.starts_with(':');
    let right = delimiter_cell.ends_with(':');
    match (left, right) {
        (true, true) => CellAlign::Center,
        (true, false) => CellAlign::Left,
        (false, true) => CellAlign::Right,
        (false, false) => CellAlign::None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub checked: bool,
    pub text: String,
    pub runs: Vec<crate::inline::InlineRun>,
}

pub fn checklist_items(lines: &[&str]) -> Option<Vec<ChecklistItem>> {
    let mut projector = DisplayProjector::default();
    checklist_items_with(lines, &mut projector)
}

fn checklist_items_with(
    lines: &[&str],
    projector: &mut DisplayProjector,
) -> Option<Vec<ChecklistItem>> {
    let mut items = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let (checked, raw) = crate::parser::checklist::parse_line(line)?;
        let raw = raw.trim();
        items.push(ChecklistItem {
            checked,
            text: crate::inline::strip_inline(raw),
            runs: projector.project(raw),
        });
    }
    (!items.is_empty()).then_some(items)
}

pub fn note_views(card: &Card) -> Vec<crate::review::NoteView> {
    let mut projector = DisplayProjector::default();
    note_views_with(card, &mut projector)
}

pub(crate) fn note_views_with(
    card: &Card,
    projector: &mut DisplayProjector,
) -> Vec<crate::review::NoteView> {
    card.notes
        .iter()
        .map(|note| crate::review::NoteView {
            badge: note.badge,
            units: text_units_with(&note.body, projector, true, &card.resolved_diagrams),
        })
        .collect()
}

pub(crate) fn answer_units_with(
    lines: &[String],
    projector: &mut DisplayProjector,
    diagrams: &[crate::card::ResolvedDiagram],
) -> Vec<ContentUnit> {
    text_units_with(&lines.join("\n"), projector, false, diagrams)
}

/// The fence's own slot gets the rendered diagram when (and only when) a
/// resolved stamp fingerprints to the fence's interior; everything else
/// stays a Code unit, so an unfrozen or stale fence falls back to source
/// in place, never somewhere else.
fn fence_unit(
    info: &str,
    lines: Vec<String>,
    diagrams: &[crate::card::ResolvedDiagram],
    projector: &mut DisplayProjector,
) -> ContentUnit {
    if info.eq_ignore_ascii_case("math") && lines.iter().any(|line| !line.trim().is_empty()) {
        let source = lines.join("\n");
        return ContentUnit::Sentence {
            runs: projector.project_display_math(&source),
            text: source,
        };
    }
    if info.eq_ignore_ascii_case("mermaid") && !lines.is_empty() {
        let source = lines.join("\n");
        let print = crate::diagram::fingerprint(&source);
        if let Some(resolved) = diagrams.iter().find(|d| d.fingerprint == print) {
            return ContentUnit::Diagram {
                src: resolved.png.display().to_string(),
                width: resolved.geometry.logical_width,
                height: resolved.geometry.logical_height,
                alt: source,
                regions: Vec::new(),
                revealed_alt: None,
            };
        }
    }
    ContentUnit::Code { lines }
}

/// Which answer lines a quote block owns. A `>` inside a fence or a display
/// math block is that block's source, so it is answer text like any other.
pub(crate) fn quote_line_flags(lines: &[String]) -> Vec<bool> {
    let mut flags = Vec::with_capacity(lines.len());
    let mut fence: Option<(char, usize)> = None;
    let mut math = false;
    for line in lines {
        if let Some((marker, run)) = fence_marker(line) {
            fence = match fence {
                Some((open, open_run))
                    if crate::parser::closes_fence(line.trim_start(), open, open_run) =>
                {
                    None
                }
                None => Some((marker, run)),
                open => open,
            };
            flags.push(false);
            continue;
        }
        if fence.is_some() {
            flags.push(false);
            continue;
        }
        if line.trim() == "$$" {
            math = !math;
            flags.push(false);
            continue;
        }
        flags.push(!math && quote_body(line).is_some());
    }
    flags
}

/// Which of a card's answer lines are quotation rather than its own prose. A
/// cloze card's back lines are its hidden spans, so a span that IS `>` is
/// exact text the learner must reproduce, never authored quotation syntax.
pub(crate) fn card_quote_flags(
    card: &crate::card::Card,
    space: crate::card::AnswerSpace,
) -> Vec<bool> {
    let back = card.answer_lines(space);
    if card.hole.is_some() {
        return vec![false; back.len()];
    }
    quote_line_flags(back)
}

/// The answer lines a check may grade: the learner's own claims, with
/// supporting quotations dropped. One place decides, so the typed target,
/// the Explain checklist, an authored select-all, and a trace's points can
/// never disagree about what a quotation is.
pub(crate) fn gradeable_answer_lines(
    card: &crate::card::Card,
    space: crate::card::AnswerSpace,
) -> Vec<String> {
    let quoted = card_quote_flags(card, space);
    card.answer_lines(space)
        .iter()
        .zip(&quoted)
        .filter(|(_, line_is_quote)| !**line_is_quote)
        .map(|(line, _)| line.clone())
        .collect()
}

/// The answer as a learner READS it: every line kept, a quotation's marker
/// dropped. For a surface that shows the whole answer as one string rather
/// than grading its parts, where dropping the quotation would show a
/// truncated answer.
pub(crate) fn readable_answer_lines(
    card: &crate::card::Card,
    space: crate::card::AnswerSpace,
) -> Vec<String> {
    let quoted = card_quote_flags(card, space);
    card.answer_lines(space)
        .iter()
        .zip(&quoted)
        .map(|(line, line_is_quote)| {
            if *line_is_quote {
                quote_body(line).unwrap_or(line).to_string()
            } else {
                line.clone()
            }
        })
        .collect()
}

/// The steps a client walks a card's answer in, over the lines it is SHOWN.
/// One place decides what is quoted, so the displayed stream, the reveal
/// count, and the typed target can never disagree.
pub(crate) fn card_answer_steps(
    card: &crate::card::Card,
    projector: &mut DisplayProjector,
    space: crate::card::AnswerSpace,
) -> Vec<AnswerStep> {
    let lines = card.answer_lines(space);
    let quoted = card_quote_flags(card, space);
    let mut steps = Vec::new();
    let mut index = 0;
    while index < quoted.len() {
        if !quoted[index] {
            steps.push(AnswerStep::Line {
                back_from: index,
                back_to: index + 1,
            });
            index += 1;
            continue;
        }
        let back_from = index;
        let mut body = Vec::new();
        while index < quoted.len() && quoted[index] {
            body.push(
                lines
                    .get(index)
                    .and_then(|line| quote_body(line))
                    .unwrap_or_default()
                    .to_string(),
            );
            index += 1;
        }
        steps.push(AnswerStep::Quote {
            back_from,
            back_to: index,
            units: text_units_with(&body.join("\n"), projector, false, &card.resolved_diagrams),
        });
    }
    steps
}

/// A quote line's body. Only a bare `>` reaches an answer: a badge opens a
/// note, which the parser routes away from content.
pub(crate) fn quote_body(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn text_units_with(
    text: &str,
    projector: &mut DisplayProjector,
    split_prose_sentences: bool,
    diagrams: &[crate::card::ResolvedDiagram],
) -> Vec<ContentUnit> {
    let mut units = Vec::new();
    let mut code_fence: Option<(char, usize, String)> = None;
    let mut code: Vec<String> = Vec::new();
    let mut math_block: Option<Vec<String>> = None;
    let mut prose = String::new();
    let mut checklist = Vec::new();

    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let logical = lines[index];
        index += 1;
        if let Some((marker, run)) = fence_marker(logical) {
            match &code_fence {
                Some((open_marker, open_run, info))
                    if crate::parser::closes_fence(
                        logical.trim_start(),
                        *open_marker,
                        *open_run,
                    ) =>
                {
                    // An empty fence keeps its unit slot: clients consume one
                    // fence-shaped unit per closed raw fence.
                    let block = std::mem::take(&mut code);
                    units.push(fence_unit(info, block, diagrams, projector));
                    code_fence = None;
                }
                None => {
                    flush_checklist(&mut checklist, &mut units);
                    flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
                    let info = fence_info(logical, marker);
                    code_fence = Some((marker, run, info.to_string()));
                    code.clear();
                }
                Some(_) => {
                    code.push(logical.to_string());
                }
            }
            continue;
        }
        if code_fence.is_some() {
            code.push(logical.to_string());
            continue;
        }
        if let Some(body) = math_block.as_mut() {
            if logical.trim() == "$$" {
                let source = std::mem::take(body).join("\n");
                math_block = None;
                if !source.trim().is_empty() {
                    units.push(ContentUnit::Sentence {
                        runs: projector.project_display_math(&source),
                        text: source,
                    });
                }
            } else {
                body.push(logical.to_string());
            }
            continue;
        }
        if logical.trim() == "$$" {
            flush_checklist(&mut checklist, &mut units);
            flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
            math_block = Some(Vec::new());
            continue;
        }
        if logical.starts_with('|')
            && lines
                .get(index)
                .is_some_and(|next| crate::parser::is_delimiter_row(next))
            && let Some(header) = crate::parser::split_cells(logical)
            && let Some(delimiter) = crate::parser::split_cells(lines[index])
            && delimiter.len() == header.len()
        {
            flush_checklist(&mut checklist, &mut units);
            flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
            let aligns = delimiter.iter().map(|cell| cell_align(cell)).collect();
            index += 1;
            let mut rows = Vec::new();
            while let Some(cells) = lines
                .get(index)
                .filter(|line| line.starts_with('|'))
                .and_then(|line| crate::parser::split_cells(line))
            {
                let mut cells = cells;
                cells.resize(header.len(), String::new());
                rows.push(
                    cells
                        .iter()
                        .map(|cell| projector.project(cell))
                        .collect::<Vec<_>>(),
                );
                index += 1;
            }
            units.push(ContentUnit::Table {
                aligns,
                header: header.iter().map(|cell| projector.project(cell)).collect(),
                rows,
            });
            continue;
        }
        if let Some(first) = quote_body(logical) {
            flush_checklist(&mut checklist, &mut units);
            flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
            let mut body = vec![first.to_string()];
            while let Some(next) = lines.get(index).and_then(|line| quote_body(line)) {
                body.push(next.to_string());
                index += 1;
            }
            units.push(ContentUnit::Quote {
                units: text_units_with(
                    &body.join("\n"),
                    projector,
                    split_prose_sentences,
                    diagrams,
                ),
            });
            continue;
        }
        let trimmed = logical.trim();
        if trimmed.is_empty() {
            flush_checklist(&mut checklist, &mut units);
            continue;
        }
        if let Some(mut items) = checklist_items_with(&[logical], projector) {
            flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
            checklist.append(&mut items);
            continue;
        }
        if crate::inline::is_display_math_line(trimmed) {
            flush_checklist(&mut checklist, &mut units);
            flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
            units.push(ContentUnit::Sentence {
                text: trimmed.to_string(),
                runs: projector.project(trimmed),
            });
            continue;
        }
        flush_checklist(&mut checklist, &mut units);
        if !prose.is_empty() {
            prose.push(' ');
        }
        prose.push_str(trimmed);
    }

    flush_checklist(&mut checklist, &mut units);
    flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
    // The parser rejects an unclosed card `$$`, so this tail only fires on
    // free text: the gathered lines degrade to a code block, like the
    // unterminated fence below.
    if let Some(body) = math_block.take()
        && !body.is_empty()
    {
        units.push(ContentUnit::Code { lines: body });
    }
    // An unterminated code fence still yields its gathered lines.
    if !code.is_empty() {
        let info = code_fence.map(|(_, _, info)| info).unwrap_or_default();
        units.push(fence_unit(&info, code, diagrams, projector));
    }
    units
}

pub fn front_units(front: &str) -> Option<Vec<ContentUnit>> {
    let mut projector = DisplayProjector::default();
    front_units_with(front, &mut projector, &[])
}

pub(crate) fn front_units_with(
    front: &str,
    projector: &mut DisplayProjector,
    diagrams: &[crate::card::ResolvedDiagram],
) -> Option<Vec<ContentUnit>> {
    let units = text_units_with(front, projector, true, diagrams);
    units
        .iter()
        .any(|unit| match unit {
            ContentUnit::Checklist { .. } | ContentUnit::Table { .. } => true,
            ContentUnit::Sentence { runs, .. } => runs
                .iter()
                .any(|run| run.math.as_ref().is_some_and(|math| math.display)),
            ContentUnit::Code { .. } | ContentUnit::Diagram { .. } | ContentUnit::Quote { .. } => {
                true
            }
        })
        .then_some(units)
}

/// The structural units of a card's context, in source order. Context
/// renders prose line-by-line on every client, so this stream carries only
/// raw fences and closed nonempty bare-math blocks. The
/// context text is MASKED, so a mermaid fence resolves through its parse
/// record's unmasked fingerprint, never through the displayed text; a
/// record carrying spans or holes stays code until span binding ships.
/// A math fence needs no record: its displayed (masked) body projects
/// directly.
pub(crate) fn context_units_with(card: &Card) -> Vec<ContentUnit> {
    let context = &card.context;
    let diagrams = &card.resolved_diagrams;
    let fences = &card.answer_fences;
    let mut projector = DisplayProjector::default();
    let mut records = fences.iter();
    structural_units_with(context, &mut projector, |info, lines, projector| {
        if info.eq_ignore_ascii_case("math") && lines.iter().any(|line| !line.trim().is_empty()) {
            let source = lines.join("\n");
            ContentUnit::Sentence {
                runs: projector.project_display_math(&source),
                text: source,
            }
        } else {
            let mermaid = info.eq_ignore_ascii_case("mermaid");
            let record = if mermaid { records.next() } else { None };
            context_fence_unit(card, mermaid, lines, record, diagrams)
        }
    })
}

fn structural_units_with(
    lines: &[String],
    projector: &mut DisplayProjector,
    mut fence_unit: impl FnMut(&str, Vec<String>, &mut DisplayProjector) -> ContentUnit,
) -> Vec<ContentUnit> {
    let mut units = Vec::new();
    let mut open: Option<(char, usize, String)> = None;
    let mut interior: Vec<String> = Vec::new();
    let mut math_block: Option<Vec<String>> = None;
    for line in lines {
        if let Some(body) = math_block.as_mut() {
            if line.trim() == "$$" {
                let source = std::mem::take(body).join("\n");
                math_block = None;
                if !source.trim().is_empty() {
                    units.push(ContentUnit::Sentence {
                        runs: projector.project_display_math(&source),
                        text: source,
                    });
                }
            } else {
                body.push(line.clone());
            }
            continue;
        }
        match (open.as_ref(), fence_marker(line)) {
            (Some((ch, len, _)), Some(_))
                if crate::parser::closes_fence(line.trim_start(), *ch, *len) =>
            {
                let (_, _, info) = open.take().expect("matched Some above");
                units.push(fence_unit(&info, std::mem::take(&mut interior), projector));
            }
            (Some(_), _) => interior.push(line.clone()),
            (None, Some((marker, run))) => {
                open = Some((marker, run, fence_info(line, marker).to_string()));
            }
            (None, None) if line.trim() == "$$" => math_block = Some(Vec::new()),
            (None, None) => {}
        }
    }
    if !interior.is_empty() {
        units.push(ContentUnit::Code { lines: interior });
    }
    units
}

/// Section prose is never a diagram: nothing freezes a section's fence, so
/// there is no record to resolve a mermaid body against and it stays code.
/// Math fences and closed nonempty bare-math blocks need no record.
pub(crate) fn section_units(lines: &[String]) -> Vec<ContentUnit> {
    let mut projector = DisplayProjector::default();
    structural_units_with(lines, &mut projector, |info, lines, projector| {
        fence_unit(info, lines, &[], projector)
    })
}

fn context_fence_unit(
    card: &Card,
    mermaid: bool,
    lines: Vec<String>,
    record: Option<&crate::card::AnswerFence>,
    diagrams: &[crate::card::ResolvedDiagram],
) -> ContentUnit {
    if mermaid
        && let Some(record) = record
        && !record.holes
        && let Some(resolved) = diagrams
            .iter()
            .find(|d| d.fingerprint == record.fingerprint)
    {
        if record.spans.is_empty() {
            return ContentUnit::Diagram {
                src: resolved.png.display().to_string(),
                width: resolved.geometry.logical_width,
                height: resolved.geometry.logical_height,
                // The record's interior, not the displayed lines: the
                // authored source is what was frozen and rendered.
                alt: record.interior.to_string(),
                regions: Vec::new(),
                revealed_alt: None,
            };
        }
        if let Some(unit) = masked_diagram_unit(card, record, resolved) {
            return unit;
        }
    }
    ContentUnit::Code { lines }
}

/// The masked projection: every span in the fence must validate and bind
/// (complete-label-only), or the whole fence stays the masked-source code
/// unit for this card; a partial diagram would mask the wrong thing.
fn masked_diagram_unit(
    card: &Card,
    record: &crate::card::AnswerFence,
    resolved: &crate::card::ResolvedDiagram,
) -> Option<ContentUnit> {
    use crate::{
        parser::region::RegionKind,
        review::{RegionRole, RegionView},
    };

    let geometry = &resolved.geometry;
    crate::diagram::validate_label_sources(geometry, &record.interior).ok()?;
    let asked: Vec<usize> = match &card.region {
        None => Vec::new(),
        Some(crate::card::RegionSlot::Single { line, .. }) => vec![*line],
        Some(crate::card::RegionSlot::Group { members, .. }) => {
            members.iter().map(|member| member.line).collect()
        }
    };
    let covers_reveal = crate::review::covers_reveal(card);
    // label index -> (role, reveal) for every bound span; a label bound
    // twice cannot happen (spans never overlap, ranges never overlap).
    let mut bound: Vec<(usize, RegionRole, bool)> = Vec::new();
    for span in &record.spans {
        let index = crate::diagram::bind_span(geometry, span.line, span.start, span.end).ok()?;
        let kind = card
            .span_regions
            .iter()
            .find(|region| region.line == span.line)
            .map(|region| region.kind)?;
        let (role, reveal) = match kind {
            RegionKind::Cover => (RegionRole::Cover, covers_reveal),
            RegionKind::Blank if asked.contains(&span.line) => (RegionRole::Asked, true),
            RegionKind::Blank => (RegionRole::Mask, false),
        };
        bound.push((index, role, reveal));
    }
    let masked = |index: usize| bound.iter().any(|(bound_index, ..)| *bound_index == index);
    let revealed = |index: usize| {
        bound
            .iter()
            .any(|(bound_index, role, _)| *bound_index == index && *role == RegionRole::Asked)
    };
    // The accessible text is the RENDERED label inventory in reading order
    // (box y, then x), never the source: source ids can spell out a masked
    // label's text.
    let mut order: Vec<usize> = (0..geometry.labels.len()).collect();
    order.sort_by_key(|&index| {
        let bounds = &geometry.labels[index].bounds;
        (bounds.y, bounds.x)
    });
    let inventory = |shows: &dyn Fn(usize) -> bool| {
        let entries: Vec<&str> = order
            .iter()
            .map(|&index| {
                if shows(index) {
                    geometry.labels[index].text.as_str()
                } else {
                    "…"
                }
            })
            .collect();
        format!("diagram labels: {}", entries.join(", "))
    };
    let regions = bound
        .iter()
        .map(|&(index, role, reveal_on_answer)| {
            let bounds = &geometry.labels[index].bounds;
            RegionView {
                role,
                reveal_on_answer,
                x: f64::from(bounds.x),
                y: f64::from(bounds.y),
                width: f64::from(bounds.width),
                height: f64::from(bounds.height),
                unit: "px".to_string(),
            }
        })
        .collect();
    Some(ContentUnit::Diagram {
        src: resolved.png.display().to_string(),
        width: geometry.logical_width,
        height: geometry.logical_height,
        alt: inventory(&|index| !masked(index)),
        regions,
        revealed_alt: Some(inventory(&|index| !masked(index) || revealed(index))),
    })
}

pub(crate) fn fence_marker(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let ch = if trimmed.starts_with("```") {
        '`'
    } else if trimmed.starts_with("~~~") {
        '~'
    } else {
        return None;
    };
    Some((ch, trimmed.chars().take_while(|c| *c == ch).count()))
}

pub(crate) fn fence_info(line: &str, marker: char) -> &str {
    line.trim_start().trim_start_matches(marker).trim()
}

fn flush_checklist(checklist: &mut Vec<ChecklistItem>, units: &mut Vec<ContentUnit>) {
    if !checklist.is_empty() {
        units.push(ContentUnit::Checklist {
            items: std::mem::take(checklist),
        });
    }
}

fn flush_prose(
    prose: &mut String,
    units: &mut Vec<ContentUnit>,
    projector: &mut DisplayProjector,
    split_prose_sentences: bool,
) {
    let chunks = if split_prose_sentences {
        split_sentences(prose)
    } else if prose.trim().is_empty() {
        Vec::new()
    } else {
        vec![prose.trim().to_string()]
    };
    for sentence in chunks {
        if !sentence.is_empty() {
            let runs = projector.project(&sentence);
            units.push(ContentUnit::Sentence {
                text: sentence,
                runs,
            });
        }
    }
    prose.clear();
}

pub fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut sentences = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        let ends_sentence = chars[i] == '.' && chars.get(i + 1).is_none_or(|c| c.is_whitespace());
        if ends_sentence {
            let sentence: String = chars[start..=i].iter().collect();
            if !sentence.trim().is_empty() {
                sentences.push(sentence.trim().to_string());
            }
            start = i + 1;
        }
    }
    let tail: String = chars[start..].iter().collect();
    if !tail.trim().is_empty() {
        sentences.push(tail.trim().to_string());
    }
    sentences
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextSpan {
    Text(String),
    Blank(String),
    Hidden(String),
}

pub fn context_spans(line: &str) -> Vec<ContextSpan> {
    let mut spans = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let blank = rest.find(BLANK);
        let hidden = rest.find(HIDDEN);
        let (pos, marker, is_blank) = match (blank, hidden) {
            (None, None) => {
                spans.push(ContextSpan::Text(rest.to_string()));
                break;
            }
            (Some(b), None) => (b, BLANK, true),
            (None, Some(h)) => (h, HIDDEN, false),
            (Some(b), Some(h)) if b <= h => (b, BLANK, true),
            (Some(_), Some(h)) => (h, HIDDEN, false),
        };
        if pos > 0 {
            spans.push(ContextSpan::Text(rest[..pos].to_string()));
        }
        let seg = marker.to_string();
        spans.push(if is_blank {
            ContextSpan::Blank(seg)
        } else {
            ContextSpan::Hidden(seg)
        });
        rest = &rest[pos + marker.len()..];
    }
    spans
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::card::AnswerSpace;

    fn note_units(card: &Card) -> Vec<ContentUnit> {
        note_views(card)
            .into_iter()
            .next()
            .map(|note| note.units)
            .unwrap_or_default()
    }

    fn card_with_note(note: &str) -> Card {
        Card::plain(
            Arc::from("s.txt"),
            "front".to_string(),
            vec!["back".to_string()],
            vec![crate::card::Note::bare(note.to_string())],
            1,
        )
    }

    fn sentence(text: &str) -> ContentUnit {
        ContentUnit::Sentence {
            text: text.into(),
            runs: crate::inline::parse_inline(text),
        }
    }

    #[test]
    fn a_bare_blockquote_run_is_one_quote_unit_carrying_its_own_units() {
        let mut projector = DisplayProjector::default();
        let lines = vec![
            "the answer".to_string(),
            "> a quoted passage".to_string(),
            "> its second line".to_string(),
        ];

        assert_eq!(
            vec![
                sentence("the answer"),
                ContentUnit::Quote {
                    units: vec![sentence("a quoted passage its second line")],
                },
            ],
            answer_units_with(&lines, &mut projector, &[]),
            "the run is one block, the marker never renders, and the quote's own \
             prose joins as prose does anywhere else"
        );
    }

    /// `quote_line_flags` and the unit scanner must agree about what a quote
    /// is, since the typed target reads one and the display reads the other.
    #[test]
    fn the_typed_target_and_the_display_agree_on_what_a_quote_is() {
        let rows: Vec<(Vec<&str>, bool, &str)> = vec![
            (vec!["plain"], false, "no blockquote at all"),
            (vec!["a", "> q"], true, "a bare run after prose"),
            (vec!["> q", "> r"], true, "a run of two lines"),
            (
                vec!["```text", "> q", "```"],
                false,
                "a fence's interior is source",
            ),
            (vec!["$$", "> q", "$$"], false, "display math is source"),
        ];
        for (lines, expected, why) in rows {
            let owned: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
            let mut projector = DisplayProjector::default();
            let units = answer_units_with(&owned, &mut projector, &[]);
            let has_quote_unit = units
                .iter()
                .any(|unit| matches!(unit, ContentUnit::Quote { .. }));
            let flagged = quote_line_flags(&owned).iter().any(|quoted| *quoted);
            assert_eq!(expected, has_quote_unit, "{why}: display units");
            assert_eq!(
                expected, flagged,
                "{why}: the typed target reads the same lines"
            );
        }
    }

    fn parsed(back: &str) -> Card {
        crate::parser::parse_str("t.md", &format!("## q\n{back}\n"))
            .unwrap()
            .remove(0)
    }

    type Span = (&'static str, usize, usize);

    fn spans(steps: &[AnswerStep]) -> Vec<Span> {
        steps
            .iter()
            .map(|step| match step {
                AnswerStep::Line { back_from, back_to } => ("line", *back_from, *back_to),
                AnswerStep::Quote {
                    back_from, back_to, ..
                } => ("quote", *back_from, *back_to),
            })
            .collect()
    }

    type StepRow = (&'static str, Vec<Span>, &'static str);

    fn step_rows() -> Vec<StepRow> {
        vec![
            ("plain", vec![("line", 0, 1)], "no quotation at all"),
            (
                "a\n> q",
                vec![("line", 0, 1), ("quote", 1, 2)],
                "a quotation last",
            ),
            (
                "> q\n> r\nb",
                vec![("quote", 0, 2), ("line", 2, 3)],
                "a quotation first, spanning two lines",
            ),
            (
                "a\n> q\n> r\nb",
                vec![("line", 0, 1), ("quote", 1, 3), ("line", 3, 4)],
                "a quotation between prose",
            ),
            (
                "a\n> q\nb\n> r",
                vec![
                    ("line", 0, 1),
                    ("quote", 1, 2),
                    ("line", 2, 3),
                    ("quote", 3, 4),
                ],
                "two quotations do not merge across the prose between them",
            ),
            (
                "```text\n> q\n```",
                vec![("line", 0, 1), ("line", 1, 2), ("line", 2, 3)],
                "a fence's interior is source, never quotation",
            ),
        ]
    }

    #[test]
    fn a_quotation_run_is_one_step_wherever_it_sits() {
        for (back, expected, why) in step_rows() {
            let card = parsed(back);
            let mut projector = DisplayProjector::default();
            assert_eq!(
                expected,
                spans(&card_answer_steps(
                    &card,
                    &mut projector,
                    AnswerSpace::Displayed
                )),
                "{why}"
            );
        }
    }

    #[test]
    fn answer_steps_cover_the_displayed_back_exactly_once() {
        for (back, _, why) in step_rows() {
            let card = parsed(back);
            let mut projector = DisplayProjector::default();
            let steps = card_answer_steps(&card, &mut projector, AnswerSpace::Displayed);
            let mut next = 0;
            for (kind, from, to) in spans(&steps) {
                assert_eq!(next, from, "{why}: {kind} starts where the last step ended");
                assert!(from < to, "{why}: {kind} spans at least one line");
                next = to;
            }
            assert_eq!(
                card.back_for_display().len(),
                next,
                "{why}: the steps reach the end of the displayed back"
            );
        }
    }

    #[test]
    fn a_quote_step_carries_the_quotation_units_not_its_marker_lines() {
        let card = parsed("the answer\n> a quoted passage\n> its second line");
        let mut projector = DisplayProjector::default();
        let steps = card_answer_steps(&card, &mut projector, AnswerSpace::Displayed);

        assert_eq!(
            vec![
                AnswerStep::Line {
                    back_from: 0,
                    back_to: 1
                },
                AnswerStep::Quote {
                    back_from: 1,
                    back_to: 3,
                    units: vec![sentence("a quoted passage its second line")],
                },
            ],
            steps,
            "the marker never reaches a client, and the run joins as prose does"
        );
    }

    #[test]
    fn a_cloze_span_that_is_a_greater_than_sign_is_a_gradeable_line() {
        let card = parsed("left \\blank{>} right");
        assert!(card.hole.is_some(), "the fixture is a cloze card");
        let mut projector = DisplayProjector::default();

        assert_eq!(
            vec![("line", 0, 1)],
            spans(&card_answer_steps(
                &card,
                &mut projector,
                AnswerSpace::Displayed
            )),
            "a hidden span is exact text to reproduce, never quotation syntax"
        );
    }

    #[test]
    fn answer_steps_follow_the_displayed_back_when_a_reshape_replaced_it() {
        let mut card = parsed("one authored line");
        card.display_back = Some(vec![
            "reshaped prose".to_string(),
            "> a quotation the author never wrote".to_string(),
        ]);
        let mut projector = DisplayProjector::default();

        assert_eq!(
            vec![("line", 0, 1), ("quote", 1, 2)],
            spans(&card_answer_steps(
                &card,
                &mut projector,
                AnswerSpace::Displayed
            )),
            "a reshape replaces the answer on every surface, so it decides the steps"
        );
    }

    #[test]
    fn no_note_yields_no_units() {
        let card = Card::plain(
            Arc::from("s.txt"),
            "f".into(),
            vec!["b".into()],
            Vec::new(),
            1,
        );
        assert!(note_units(&card).is_empty());
    }

    #[test]
    fn checklist_items_preserve_the_authored_state_and_text() {
        assert_eq!(
            Some(vec![ChecklistItem {
                checked: true,
                text: "keep this".into(),
                runs: crate::inline::parse_inline("keep this"),
            }]),
            checklist_items(&["- [x] keep this"])
        );
    }

    #[test]
    fn prose_splits_into_sentences() {
        let units = note_units(&card_with_note("First one. Second one."));
        assert_eq!(units, vec![sentence("First one."), sentence("Second one.")]);
    }

    #[test]
    fn hard_wrapped_prose_joins_before_splitting() {
        let units = note_units(&card_with_note("A sentence spread\nacross two lines."));
        assert_eq!(units, vec![sentence("A sentence spread across two lines.")]);
    }

    #[test]
    fn ordinary_answer_units_join_authored_soft_wraps() {
        let mut projector = DisplayProjector::default();
        let lines = vec![
            "`state::open_store` loads the initialized deck, requires its".to_string(),
            "stable deck ID, and opens the document.".to_string(),
        ];
        assert_eq!(
            answer_units_with(&lines, &mut projector, &[]),
            vec![sentence(
                "`state::open_store` loads the initialized deck, requires its stable deck ID, and opens the document."
            )]
        );
    }

    #[test]
    fn display_math_line_flushes_surrounding_prose() {
        let units = note_units(&card_with_note("Before.\n$$x^2$$\nAfter."));
        assert_eq!(units.len(), 3);
        assert_eq!(units[0], sentence("Before."));
        let ContentUnit::Sentence { text, runs } = &units[1] else {
            panic!("display math should be a sentence unit");
        };
        assert_eq!(text, "$$x^2$$");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].math.as_ref().unwrap().display);
        assert_eq!(units[2], sentence("After."));
    }

    #[test]
    fn display_math_makes_front_units_structural() {
        let units = front_units("Before\n$$x^2$$\nAfter").unwrap();
        assert_eq!(units.len(), 3);
        let ContentUnit::Sentence { runs, .. } = &units[1] else {
            panic!("display math should be a sentence unit");
        };
        assert!(runs[0].math.as_ref().unwrap().display);
    }

    #[test]
    fn a_multi_line_dollar_block_is_one_display_math_unit() {
        let units = note_units(&card_with_note("Before.\n$$\nx^2 +\ny^2\n$$\nAfter."));
        assert_eq!(units.len(), 3, "{units:?}");
        let ContentUnit::Sentence { text, runs } = &units[1] else {
            panic!("a dollar block should be a sentence unit: {units:?}");
        };
        assert_eq!(text, "x^2 +\ny^2", "the body joins, markers drop");
        assert_eq!(runs.len(), 1);
        let math = runs[0].math.as_ref().unwrap();
        assert!(math.display);
        assert!(
            math.svg
                .as_deref()
                .is_some_and(|svg| svg.starts_with("<svg"))
        );
        assert_eq!(units[2], sentence("After."));
    }

    #[test]
    fn a_multi_line_dollar_body_stays_inert_to_card_markup() {
        let deck = crate::parser::parse(
            "math",
            "## Why is the square positive?\n$$\nx^2\n\n> 0\n$$\n",
        )
        .unwrap();
        let card = &deck.cards[0];
        let mut projector = DisplayProjector::default();
        let units = answer_units_with(&card.back, &mut projector, &[]);
        let [ContentUnit::Sentence { text, runs }] = units.as_slice() else {
            panic!("the complete formula must stay one display unit: {units:?}");
        };
        assert_eq!(
            text, "x^2\n\n> 0",
            "blank and greater-than lines are verbatim math source"
        );
        assert!(runs[0].math.as_ref().is_some_and(|math| math.display));
        assert_eq!(
            card.only_note(),
            None,
            "a greater-than line inside display math must not become a card note"
        );
    }

    #[test]
    fn an_empty_dollar_pair_emits_no_unit() {
        let units = note_units(&card_with_note("Before.\n$$\n$$\nAfter."));
        assert_eq!(
            units,
            vec![sentence("Before."), sentence("After.")],
            "two markers with no content show nothing"
        );
    }

    #[test]
    fn a_dollar_line_inside_a_fence_stays_code() {
        let units = note_units(&card_with_note("```\n$$\nx^2\n$$\n```"));
        assert_eq!(
            units,
            vec![ContentUnit::Code {
                lines: vec!["$$".into(), "x^2".into(), "$$".into()]
            }]
        );
    }

    #[test]
    fn an_unterminated_dollar_block_in_free_text_degrades_to_code() {
        let units = note_units(&card_with_note("$$\nx^2"));
        assert_eq!(
            units,
            vec![ContentUnit::Code {
                lines: vec!["x^2".into()]
            }],
            "only free text can reach this: the parser rejects an unclosed card opener"
        );
    }

    #[test]
    fn a_math_fence_is_one_display_math_unit() {
        let units = note_units(&card_with_note(
            "Before.\n```math\nx^2 + y^2\n= z^2\n```\nAfter.",
        ));
        assert_eq!(units.len(), 3, "{units:?}");
        let ContentUnit::Sentence { text, runs } = &units[1] else {
            panic!("a math fence should be a sentence unit: {units:?}");
        };
        assert_eq!(text, "x^2 + y^2\n= z^2", "the body joins as one source");
        assert_eq!(runs.len(), 1);
        let math = runs[0].math.as_ref().unwrap();
        assert!(math.display);
        assert!(
            math.svg
                .as_deref()
                .is_some_and(|svg| svg.starts_with("<svg")),
            "a multi-line body still renders: {math:?}"
        );
        assert_eq!(units[2], sentence("After."));
    }

    #[test]
    fn a_math_fence_body_is_source_not_dollar_scanned() {
        let units = note_units(&card_with_note("```math\n$x^2$\n```"));
        let ContentUnit::Sentence { text, runs } = &units[0] else {
            panic!("expected a sentence unit: {units:?}");
        };
        assert_eq!(text, "$x^2$", "dollars in the body stay source");
        assert_eq!(runs[0].text, "$x^2$");
        assert!(runs[0].math.as_ref().unwrap().display);
    }

    #[test]
    fn an_empty_math_fence_stays_a_code_block() {
        for note in ["```math\n```", "```math\n   \n```"] {
            let units = note_units(&card_with_note(note));
            assert!(
                matches!(units.as_slice(), [ContentUnit::Code { .. }]),
                "{note}: {units:?}"
            );
        }
    }

    #[test]
    fn a_math_fence_makes_front_units_structural() {
        let units = front_units("Ready?\n```math\nx^2\n```").unwrap();
        assert_eq!(units.len(), 2, "{units:?}");
        let ContentUnit::Sentence { runs, .. } = &units[1] else {
            panic!("expected the math unit: {units:?}");
        };
        assert!(runs[0].math.as_ref().unwrap().display);
    }

    #[test]
    fn a_bare_table_becomes_one_aligned_table_unit() {
        let units = note_units(&card_with_note(
            "before\n| h1 | h2 |\n|:---|---:|\n| **a** | b |\n| c |\ntail",
        ));
        assert_eq!(3, units.len(), "{units:?}");
        let ContentUnit::Table {
            aligns,
            header,
            rows,
        } = &units[1]
        else {
            panic!("the pipe block is a table unit: {units:?}");
        };
        assert_eq!(&[CellAlign::Left, CellAlign::Right], aligns.as_slice());
        assert_eq!(
            vec!["h1", "h2"],
            header
                .iter()
                .map(|runs| runs.iter().map(|run| run.text.as_str()).collect::<String>())
                .collect::<Vec<_>>()
        );
        assert_eq!(2, rows.len(), "{rows:?}");
        assert!(
            rows[0][0].iter().any(|run| run.bold),
            "cell text renders inline styling: {rows:?}"
        );
        assert_eq!(
            2,
            rows[1].len(),
            "a short row pads to the header width: {rows:?}"
        );
        assert!(rows[1][1].is_empty(), "the padded cell is empty: {rows:?}");
        assert!(
            matches!(&units[2], ContentUnit::Sentence { text, .. } if text == "tail"),
            "prose resumes after the table: {units:?}"
        );
    }

    #[test]
    fn a_pipe_line_without_a_delimiter_row_stays_prose() {
        let units = note_units(&card_with_note("| not | a table |\njust prose"));
        assert!(
            units
                .iter()
                .all(|unit| matches!(unit, ContentUnit::Sentence { .. })),
            "{units:?}"
        );
    }

    #[test]
    fn a_table_inside_a_fence_stays_code() {
        let units = note_units(&card_with_note("```\n| a | b |\n|---|---|\n```"));
        assert_eq!(
            units,
            vec![ContentUnit::Code {
                lines: vec!["| a | b |".into(), "|---|---|".into()]
            }]
        );
    }

    #[test]
    fn a_mismatched_delimiter_width_is_not_a_table() {
        let units = note_units(&card_with_note("| a | b |\n|---|\nafter"));
        assert!(
            units
                .iter()
                .all(|unit| matches!(unit, ContentUnit::Sentence { .. })),
            "{units:?}"
        );
    }

    #[test]
    fn a_nested_fence_is_one_note_code_unit_to_its_matching_closer() {
        let units = note_units(&card_with_note("````\n```\ncode\n```\n````"));
        assert_eq!(
            units,
            vec![ContentUnit::Code {
                lines: vec!["```".into(), "code".into(), "```".into()]
            }]
        );
    }

    #[test]
    fn a_fence_shaped_content_line_with_info_does_not_close_the_fence() {
        let units = note_units(&card_with_note("````\n````rust\n$x$\n````"));
        assert_eq!(
            units,
            vec![ContentUnit::Code {
                lines: vec!["````rust".into(), "$x$".into()]
            }]
        );
    }

    #[test]
    fn dollars_in_fenced_code_never_render_as_math() {
        for fence in ["```", "~~~"] {
            let note = format!("{fence}\n$x^2$\n{fence}");
            let units = note_units(&card_with_note(&note));
            assert_eq!(
                units,
                vec![ContentUnit::Code {
                    lines: vec!["$x^2$".into()]
                }]
            );
        }
    }

    #[test]
    fn front_units_are_some_only_when_a_task_list_is_present() {
        assert_eq!(None, front_units("What is the capital of France?"));
        let units = front_units("Given this list:\n- [x] keep\n- [ ] drop").unwrap();
        assert_eq!(
            units,
            vec![
                ContentUnit::Sentence {
                    text: "Given this list:".into(),
                    runs: crate::inline::parse_inline("Given this list:"),
                },
                ContentUnit::Checklist {
                    items: vec![
                        ChecklistItem {
                            checked: true,
                            text: "keep".into(),
                            runs: crate::inline::parse_inline("keep"),
                        },
                        ChecklistItem {
                            checked: false,
                            text: "drop".into(),
                            runs: crate::inline::parse_inline("drop"),
                        },
                    ],
                },
            ]
        );
    }

    #[test]
    fn a_task_list_note_becomes_a_checklist_unit() {
        let units = note_units(&card_with_note("Recall:\n- [x] do this\n- [ ] not that"));
        assert_eq!(
            units,
            vec![
                ContentUnit::Sentence {
                    text: "Recall:".into(),
                    runs: crate::inline::parse_inline("Recall:"),
                },
                ContentUnit::Checklist {
                    items: vec![
                        ChecklistItem {
                            checked: true,
                            text: "do this".into(),
                            runs: crate::inline::parse_inline("do this"),
                        },
                        ChecklistItem {
                            checked: false,
                            text: "not that".into(),
                            runs: crate::inline::parse_inline("not that"),
                        },
                    ],
                },
            ]
        );
    }

    #[test]
    fn code_block_is_verbatim() {
        let note = "Intro here.\n```\nfn main() {\n    let x = 1;\n}\n```";
        let units = note_units(&card_with_note(note));
        assert_eq!(
            units,
            vec![
                sentence("Intro here."),
                ContentUnit::Code {
                    lines: vec!["fn main() {".into(), "    let x = 1;".into(), "}".into(),]
                },
            ]
        );
    }

    #[test]
    fn prose_after_code_is_its_own_unit() {
        let note = "```\ncode\n```\nAfter the block.";
        let units = note_units(&card_with_note(note));
        assert_eq!(
            units,
            vec![
                ContentUnit::Code {
                    lines: vec!["code".into()]
                },
                sentence("After the block."),
            ]
        );
    }

    #[test]
    fn unterminated_fence_still_yields_code() {
        let units = note_units(&card_with_note("```\nlonely line"));
        assert_eq!(
            units,
            vec![ContentUnit::Code {
                lines: vec!["lonely line".into()]
            }]
        );
    }

    #[test]
    fn period_in_number_does_not_split() {
        let units = note_units(&card_with_note("See section 2.1 for details."));
        assert_eq!(units, vec![sentence("See section 2.1 for details.")]);
    }

    #[test]
    fn note_units_serialize_as_the_documented_wire_shape() {
        let units = vec![
            ContentUnit::Sentence {
                text: "One owner.".into(),
                runs: crate::inline::parse_inline("One owner."),
            },
            ContentUnit::Code {
                lines: vec!["let s;".into()],
            },
            ContentUnit::Checklist {
                items: vec![ChecklistItem {
                    checked: true,
                    text: "Own it".into(),
                    runs: crate::inline::parse_inline("**Own** it"),
                }],
            },
        ];
        assert_eq!(
            serde_json::json!([
                {"kind": "sentence", "text": "One owner.", "runs": [{"text": "One owner."}]},
                {"kind": "code", "lines": ["let s;"]},
                {"kind": "checklist", "items": [{
                    "checked": true,
                    "text": "Own it",
                    "runs": [{"text": "Own", "bold": true}, {"text": " it"}]
                }]},
            ]),
            serde_json::to_value(&units).unwrap()
        );
    }

    #[test]
    fn context_spans_split_holes() {
        use ContextSpan::*;
        assert_eq!(context_spans("plain text"), vec![Text("plain text".into())]);
        assert_eq!(
            context_spans("To ⍰ or not to ⬚"),
            vec![
                Text("To ".into()),
                Blank("⍰".into()),
                Text(" or not to ".into()),
                Hidden("⬚".into()),
            ]
        );
        assert_eq!(
            context_spans("⍰ here"),
            vec![Blank("⍰".into()), Text(" here".into())]
        );
        assert_eq!(
            context_spans("⬚ before ⍰"),
            vec![
                Hidden("⬚".into()),
                Text(" before ".into()),
                Blank("⍰".into()),
            ]
        );
    }

    fn resolved(source: &str) -> Vec<crate::card::ResolvedDiagram> {
        vec![crate::card::ResolvedDiagram {
            fingerprint: crate::diagram::fingerprint(source),
            png: std::path::PathBuf::from("/ws/assets/deck-x/sha256-aa.png"),
            geometry: crate::diagram::DiagramGeometry {
                image: "sha256-aa.png".to_string(),
                image_width: 376,
                image_height: 228,
                logical_width: 188,
                logical_height: 114,
                labels: Vec::new(),
            },
        }]
    }

    /// A rendered diagram replaces its own fence in the ordered stream:
    /// surrounding prose keeps its position, and only a matching resolved
    /// stamp swaps.
    #[test]
    fn a_resolved_mermaid_fence_becomes_a_diagram_unit_in_its_own_slot() {
        let source = "flowchart LR\n A-->B";
        let text = "before\n```mermaid\nflowchart LR\n A-->B\n```\nafter";
        let mut projector = DisplayProjector::default();
        let units = text_units_with(text, &mut projector, false, &resolved(source));
        assert_eq!(3, units.len(), "{units:?}");
        assert!(matches!(&units[0], ContentUnit::Sentence { text, .. } if text == "before"));
        let ContentUnit::Diagram {
            src,
            width,
            height,
            alt,
            ..
        } = &units[1]
        else {
            panic!("the fence slot holds the diagram: {units:?}");
        };
        assert_eq!("/ws/assets/deck-x/sha256-aa.png", src);
        assert_eq!(
            (&188, &114),
            (width, height),
            "logical, not raster, dimensions"
        );
        assert_eq!(source, alt, "the fence source is the accessible text");
        assert!(matches!(&units[2], ContentUnit::Sentence { text, .. } if text == "after"));
    }

    #[test]
    fn a_section_fence_closes_only_on_its_own_marker() {
        let lines: Vec<String> = ["```", "~~~", "text", "```"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            vec![ContentUnit::Code {
                lines: vec!["~~~".into(), "text".into()]
            }],
            section_units(&lines),
            "a tilde run never closes a backtick fence"
        );
    }

    #[test]
    fn a_section_fence_shaped_line_with_info_stays_inside() {
        let lines: Vec<String> = ["````", "````rust", "x", "````"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            vec![ContentUnit::Code {
                lines: vec!["````rust".into(), "x".into()]
            }],
            section_units(&lines),
            "an info-carrying line is content, not a closer"
        );
    }

    #[test]
    fn a_math_fence_renders_in_section_and_context_walks() {
        let lines: Vec<String> = ["```math", "x^2", "```"]
            .iter()
            .map(|line| line.to_string())
            .collect();
        let section = section_units(&lines);
        let context = context_units_with(&context_card(&lines, Vec::new(), Vec::new()));
        for (surface, units) in [("section", section), ("context", context)] {
            let [ContentUnit::Sentence { text, runs }] = units.as_slice() else {
                panic!("a math fence in {surface} must be display math: {units:?}");
            };
            assert_eq!(text, "x^2");
            assert!(
                runs[0].math.as_ref().is_some_and(|math| math.display),
                "{surface} must carry a display math run: {runs:?}"
            );
        }
    }

    #[test]
    fn section_and_context_walks_share_the_bare_display_math_boundary() {
        let cases = [
            (
                "closed bare block",
                vec!["$$", "x^2 + y^2", "$$"],
                Some("x^2 + y^2"),
            ),
            (
                "unclosed bare opener stays prose",
                vec!["$$", "x^2 + y^2"],
                None,
            ),
            (
                "math fence control",
                vec!["```math", "x^2 + y^2", "```"],
                Some("x^2 + y^2"),
            ),
        ];

        for (case, source, expected) in cases {
            let lines: Vec<String> = source.into_iter().map(str::to_string).collect();
            let surfaces = [
                ("section", section_units(&lines)),
                (
                    "context",
                    context_units_with(&context_card(&lines, Vec::new(), Vec::new())),
                ),
            ];
            for (surface, units) in surfaces {
                match expected {
                    Some(expected) => {
                        let [ContentUnit::Sentence { text, runs }] = units.as_slice() else {
                            panic!("{case} on {surface} must be display math: {units:?}");
                        };
                        assert_eq!(expected, text, "{case} on {surface}");
                        assert!(
                            runs[0].math.as_ref().is_some_and(|math| math.display),
                            "{case} on {surface} must carry a display run: {runs:?}"
                        );
                    }
                    None => assert!(
                        units.is_empty(),
                        "{case} on {surface} must remain in the raw prose walk: {units:?}"
                    ),
                }
            }
        }
    }

    #[test]
    fn a_context_fence_shaped_line_with_info_stays_inside() {
        let context: Vec<String> = ["````", "````rust", "x", "````"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let units = context_units_with(&context_card(&context, Vec::new(), Vec::new()));
        assert_eq!(
            vec![ContentUnit::Code {
                lines: vec!["````rust".into(), "x".into()]
            }],
            units,
            "an info-carrying line is content, not a closer"
        );
    }

    #[test]
    fn a_nested_context_fence_is_one_code_unit_to_its_matching_closer() {
        let context: Vec<String> = ["````", "a", "```", "b", "````"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let units = context_units_with(&context_card(&context, Vec::new(), Vec::new()));
        assert_eq!(
            units,
            vec![ContentUnit::Code {
                lines: vec!["a".into(), "```".into(), "b".into()]
            }]
        );
    }

    fn context_card(
        context: &[String],
        diagrams: Vec<crate::card::ResolvedDiagram>,
        fences: Vec<crate::card::AnswerFence>,
    ) -> Card {
        let mut card = Card::plain(
            std::sync::Arc::from("deck.md"),
            "q".to_string(),
            vec!["answer".to_string()],
            Vec::new(),
            1,
        );
        card.context = context.to_vec();
        card.resolved_diagrams = diagrams;
        card.answer_fences = fences;
        card
    }

    fn record(source: &str) -> crate::card::AnswerFence {
        crate::card::AnswerFence {
            fingerprint: crate::diagram::fingerprint(source),
            interior: std::sync::Arc::from(source),
            spans: Vec::new(),
            holes: false,
        }
    }

    /// The context stream's law: fence-shaped units only, one per closed
    /// fence in order; a clean record (no spans, no holes) whose unmasked
    /// fingerprint resolves swaps to the diagram, everything else is code.
    #[test]
    fn a_clean_recorded_context_fence_resolves_through_its_record() {
        let source = "flowchart LR\n A-->B";
        let context: Vec<String> = [
            "prose line",
            "```rust",
            "let x = 1;",
            "```",
            "```mermaid",
            "flowchart LR",
            " A-->B",
            "```",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let fences = vec![record(source)];
        let units = context_units_with(&context_card(&context, resolved(source), fences));
        assert_eq!(2, units.len(), "{units:?}");
        assert!(
            matches!(&units[0], ContentUnit::Code { lines } if lines == &["let x = 1;"]),
            "a non-mermaid fence is code and consumes no record: {units:?}"
        );
        let ContentUnit::Diagram { alt, .. } = &units[1] else {
            panic!("the mermaid fence resolves through its record: {units:?}");
        };
        assert_eq!(source, alt, "the clean interior is the accessible text");
    }

    #[test]
    fn a_record_with_spans_or_holes_or_no_resolution_stays_code_in_context() {
        let source = "flowchart LR\n A-->B";
        let context: Vec<String> = ["```mermaid", "flowchart LR", " A-->B", "```"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let with_spans = crate::card::AnswerFence {
            spans: vec![crate::card::AnswerFenceSpan {
                line: 6,
                start: 0,
                end: 4,
            }],
            ..record(source)
        };
        let with_holes = crate::card::AnswerFence {
            holes: true,
            ..record(source)
        };
        let cases: [(Vec<crate::card::AnswerFence>, &str); 4] = [
            (vec![with_spans], "bound spans hold for span binding"),
            (vec![with_holes], "a hole poisons the fence"),
            (vec![record("flowchart LR\n X-->Y")], "no resolution"),
            (Vec::new(), "no record at all"),
        ];
        for (fences, why) in cases {
            let units = context_units_with(&context_card(&context, resolved(source), fences));
            assert!(
                matches!(&units[0], ContentUnit::Code { .. }),
                "{why}: {units:?}"
            );
        }
    }

    #[test]
    fn a_masked_context_interior_resolves_by_record_never_by_its_own_text() {
        let authored = "flowchart LR\n![](x.png)\n A-->B";
        let context: Vec<String> = ["```mermaid", "flowchart LR", " A-->B", "```"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let fences = vec![record(authored)];
        let units = context_units_with(&context_card(&context, resolved(authored), fences));
        let ContentUnit::Diagram { alt, .. } = &units[0] else {
            panic!(
                "the record carries the unmasked fingerprint even where the \
                 displayed interior diverges: {units:?}"
            );
        };
        assert_eq!(
            authored, alt,
            "the accessible text is the authored interior, not the displayed one"
        );
    }

    /// Clients consume one fence-shaped unit per closed raw fence, so an
    /// empty fence must still emit its code unit: skipping it hands this
    /// fence the NEXT fence's unit and a later diagram lands one slot early.
    #[test]
    fn an_empty_fence_keeps_its_unit_slot_so_a_later_diagram_stays_aligned() {
        let source = "flowchart LR\n A-->B";
        let text = "```\n```\n```mermaid\nflowchart LR\n A-->B\n```";
        let mut projector = DisplayProjector::default();
        let units = text_units_with(text, &mut projector, false, &resolved(source));
        assert_eq!(2, units.len(), "{units:?}");
        assert!(
            matches!(&units[0], ContentUnit::Code { lines } if lines.is_empty()),
            "the empty fence holds its slot: {units:?}"
        );
        assert!(
            matches!(&units[1], ContentUnit::Diagram { .. }),
            "{units:?}"
        );
    }

    /// The server resolves availability: no resolved stamp, a stale
    /// fingerprint, or a non-mermaid fence all stay code units in place.
    #[test]
    fn unresolved_stale_and_foreign_fences_stay_code_units() {
        let mut projector = DisplayProjector::default();
        let cases: [(&str, Vec<crate::card::ResolvedDiagram>, &str); 3] = [
            (
                "```mermaid\nflowchart LR\n A-->B\n```",
                Vec::new(),
                "unfrozen",
            ),
            (
                "```mermaid\nflowchart LR\n A-->B\n```",
                resolved("flowchart LR\n A-->EDITED"),
                "stale fingerprint",
            ),
            (
                "```rust\nflowchart LR\n A-->B\n```",
                resolved("flowchart LR\n A-->B"),
                "non-mermaid fence",
            ),
        ];
        for (text, diagrams, why) in cases {
            let units = text_units_with(text, &mut projector, false, &diagrams);
            assert!(
                matches!(&units[0], ContentUnit::Code { .. }),
                "{why}: must stay a code unit, got {units:?}"
            );
        }
    }
}
