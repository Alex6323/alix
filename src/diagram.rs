//! Rendering mermaid fences to SVG by shelling out to the `sekien` CLI.
//!
//! The renderer is an authoring-time tool: its output is frozen as a
//! deck-owned asset and every client just displays an SVG. Nothing here runs
//! during review.

use std::hash::Hasher;
#[cfg(feature = "full")]
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::Duration,
};

#[cfg(feature = "full")]
use anyhow::{Context, Result, bail};
#[cfg(feature = "full")]
use resvg::usvg;
use twox_hash::XxHash64;

/// The CLI alix shells out to. Named here rather than configured: a second
/// renderer would produce different pictures for the same deck.
pub const COMMAND: &str = "sekien";

#[cfg(feature = "full")]
/// Checked before the pixmap is allocated, so a huge graph fails as one
/// authoring error instead of an unbounded allocation. 4096 squared.
const PIXEL_CAP: u64 = 4096 * 4096;

#[cfg(feature = "full")]
/// A long `flowchart LR` grows in one dimension and can pass the area cap
/// while exceeding mobile decoder and GPU texture limits, so each side is
/// bounded independently.
const SIDE_CAP: u32 = 8192;

#[cfg(feature = "full")]
/// The ground for diagrams whose root style declares a light base fill:
/// mermaid's dark themes assume a dark page and their `background` theme
/// variable is this value.
const DARK_GROUND: (u8, u8, u8) = (0x33, 0x33, 0x33);

#[cfg(feature = "full")]
/// One fence's outcome: the SVG, or the renderer's own message for it.
pub type Rendered = Result<String, String>;

/// The info string that marks a fence as a diagram.
const LANGUAGE: &str = "mermaid";

/// A mermaid fence inside one block.
#[derive(Debug, PartialEq, Eq)]
pub struct Fence {
    /// The interior, verbatim and without the delimiter lines. This is the
    /// renderer's input and the whole fingerprint preimage.
    pub source: String,
    /// Index into the block's lines of the opening delimiter.
    pub opener: usize,
}

/// Every mermaid fence in `lines`, in document order.
///
/// The info string is compared case-insensitively after trimming, so
/// ```` ```mermaid ````, ```` ``` mermaid ```` and ```` ~~~MERMAID ```` all
/// count; any other language does not. An unclosed fence runs to the end of
/// the block, matching how the stream already treats fenced interiors.
pub fn fences(lines: &[String]) -> Vec<Fence> {
    let mut found = Vec::new();
    // `opener` is None while a NON-mermaid fence is open: such a fence still
    // has to be consumed, or its interior could be read as a diagram, but it
    // never yields one. An Option carries that where a magic index would not:
    // both exit paths must handle it, so neither can drop the distinction.
    let mut open: Option<(char, usize, Option<usize>, Vec<String>)> = None;
    let close = |opener: Option<usize>, body: Vec<String>, found: &mut Vec<Fence>| {
        if let Some(opener) = opener {
            found.push(Fence {
                source: body.join("\n"),
                opener,
            });
        }
    };
    for (index, line) in lines.iter().enumerate() {
        match &mut open {
            Some((ch, len, _, body)) => {
                if crate::parser::closes_fence(line, *ch, *len) {
                    let (_, _, opener, body) = open.take().expect("the fence is open");
                    close(opener, body, &mut found);
                } else {
                    body.push(line.clone());
                }
            }
            None => {
                if let Some((ch, len)) = crate::parser::fence_opener(line) {
                    let info = line.trim_start_matches(ch).trim();
                    let opener = info.eq_ignore_ascii_case(LANGUAGE).then_some(index);
                    open = Some((ch, len, opener, Vec::new()));
                }
            }
        }
    }
    if let Some((_, _, opener, body)) = open {
        close(opener, body, &mut found);
    }
    found
}

/// The frozen-forever preimage: the fence's interior bytes and nothing else.
///
/// Deliberately excludes the renderer version, the mermaid version, and the
/// theme. A frozen SVG is evidence, not a cache: putting a version in here
/// would invalidate every diagram in every shared deck on any upgrade, for a
/// recipient who may not even have the renderer installed. Re-rendering stays
/// a deliberate authoring act.
/// CRLF terminators are normalized to LF before hashing: the scanner
/// reads raw lines and the parser reads
/// terminator-stripped ones, so a byte-sensitive preimage would give one
/// fence two fingerprints depending on who asked. A lone CR is NOT
/// normalized: it is content, deliberately.
pub fn fingerprint(source: &str) -> String {
    let mut hasher = XxHash64::default();
    hasher.write(source.replace("\r\n", "\n").as_bytes());
    format!("xxh64-{:016x}", hasher.finish())
}

#[cfg(feature = "full")]
/// Rasterizes a rendered SVG to PNG bytes at `zoom` times its intrinsic size.
///
/// PNG rather than SVG is load-bearing: a mermaid SVG carries every label as
/// selectable `<text>`, so an overlay on it hides nothing (view-source,
/// Ctrl+F and screen readers all read the answer). Pixels under an overlay
/// are the trust model image occlusion already ships.
///
/// `family` must name a concrete font that both this rasterizer and the
/// renderer measured with; a text-bearing SVG whose family is missing fails
/// loudly here, because usvg would otherwise drop every label and freeze a
/// silently textless diagram.
#[derive(Debug)]
pub struct Raster {
    pub image: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[cfg(feature = "full")]
pub fn rasterize(svg: &str, family: &str, zoom: f32) -> Result<Raster> {
    let pixmap = raster(svg, family, zoom)?;
    Ok(Raster {
        width: pixmap.width(),
        height: pixmap.height(),
        image: pixmap.encode_png().context("the PNG cannot be encoded")?,
    })
}

#[cfg(feature = "full")]
fn check_raster_size(width: u32, height: u32) -> Result<()> {
    if width > SIDE_CAP || height > SIDE_CAP {
        bail!(
            "the diagram would rasterize to {width}x{height} pixels, over the {SIDE_CAP} per-side cap"
        );
    }
    if u64::from(width) * u64::from(height) > PIXEL_CAP {
        bail!("the diagram would rasterize to {width}x{height} pixels, over the {PIXEL_CAP} cap");
    }
    Ok(())
}

#[cfg(feature = "full")]
fn raster(svg: &str, family: &str, zoom: f32) -> Result<resvg::tiny_skia::Pixmap> {
    let mut db = system_fonts().clone();
    db.set_sans_serif_family(family);
    let query = usvg::fontdb::Query {
        families: &[usvg::fontdb::Family::Name(family)],
        ..Default::default()
    };
    if svg.contains("<text") && db.query(&query).is_none() {
        bail!("font family '{family}' is not installed, so the diagram's text cannot be drawn");
    }
    let options = usvg::Options {
        font_family: family.to_string(),
        fontdb: std::sync::Arc::new(db),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).context("the SVG cannot be parsed")?;
    let width = (tree.size().width() * zoom).ceil() as u32;
    let height = (tree.size().height() * zoom).ceil() as u32;
    check_raster_size(width, height)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .with_context(|| format!("the diagram has no drawable size at zoom {zoom}"))?;
    let (red, green, blue) = ground(svg)?;
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(red, green, blue, 255));
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(zoom, zoom),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

#[cfg(feature = "full")]
/// The opaque ground the SVG is composited onto, derived from the diagram's
/// own theme: every mermaid theme declares its base text fill in the root
/// style rule (`#d1{...fill:#ccc}` for dark themes, `fill:#333` for light),
/// and the ground with the higher WCAG contrast against that fill wins
/// (channel-mean thresholds put bright saturated ink on the wrong ground). A
/// marker this derivation cannot read is an error, never a silent white:
/// sekien is not version-pinned, and a CSS serialization change in an
/// upgrade must fail the freeze loudly instead of erasing content.
fn ground(svg: &str) -> Result<(u8, u8, u8)> {
    let style = svg
        .split_once("<style")
        .and_then(|(_, rest)| rest.split_once('>'))
        .and_then(|(_, rest)| rest.split_once("</style").map(|(inner, _)| inner));
    let Some(style) = style else {
        bail!("the diagram carries no theme marker (no style block), so no safe ground exists");
    };
    let Some(fill) = fill_declaration(style) else {
        bail!("the diagram's theme declares no base fill, so no safe ground exists");
    };
    let Some(fill) = hex_color(fill) else {
        bail!("the theme's base fill '{fill}' is not a color the ground derivation recognizes");
    };
    let ink = relative_luminance(fill);
    let dark = relative_luminance(DARK_GROUND);
    Ok(if contrast(ink, dark) > contrast(ink, 1.0) {
        DARK_GROUND
    } else {
        (255, 255, 255)
    })
}

#[cfg(feature = "full")]
/// WCAG sRGB relative luminance: channel mean misclassifies saturated ink
/// (`#00ff00` has mean 85 yet is among the brightest colors there are), and
/// the gamma linearization matters even for grays.
fn relative_luminance((red, green, blue): (u8, u8, u8)) -> f64 {
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
}

#[cfg(feature = "full")]
fn contrast(a: f64, b: f64) -> f64 {
    let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

#[cfg(feature = "full")]
/// The value of the first `fill:` declaration, tolerating whitespace around
/// the colon. Skips property names that merely start with `fill`
/// (`fill-opacity`), whose next non-space character is not a colon.
fn fill_declaration(style: &str) -> Option<&str> {
    let mut rest = style;
    while let Some(at) = rest.find("fill") {
        let after = &rest[at + "fill".len()..];
        if let Some(value) = after.trim_start().strip_prefix(':') {
            let value = value.trim_start();
            let end = value.find([';', '}']).unwrap_or(value.len());
            return Some(value[..end].trim_end());
        }
        rest = after;
    }
    None
}

#[cfg(feature = "full")]
fn hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?.as_bytes();
    let expand = |part: &[u8]| u8::from_str_radix(std::str::from_utf8(part).ok()?, 16).ok();
    match hex.len() {
        3 => Some((
            expand(&hex[..1])? * 17,
            expand(&hex[1..2])? * 17,
            expand(&hex[2..3])? * 17,
        )),
        6 => Some((expand(&hex[..2])?, expand(&hex[2..4])?, expand(&hex[4..6])?)),
        _ => None,
    }
}

#[cfg(feature = "full")]
/// One authoring batch gets this budget; ~1.3s startup plus ~19ms per
/// diagram measured, so a minute covers decks far past any real size.
pub const RENDER_TIMEOUT: Duration = Duration::from_secs(60);

/// One remedy string, shared by the doctor row and the freeze warning so
/// "the same message doctor shows" holds by construction.
pub const REMEDY: &str =
    "install sekien (`cargo install sekien`); on Linux it also needs webkit2gtk, gtk3 and xvfb";

/// A mermaid fence found in a whole deck document, with the byte offsets a
/// stamp rewrite needs.
#[derive(Debug, PartialEq)]
pub struct RawFence {
    pub source: String,
    /// Byte offset where a new stamp line is inserted: the start of the line
    /// after the fence's closing delimiter.
    pub insert_at: usize,
    /// An existing stamp on the next line: its byte range (without the
    /// trailing newline) and its fingerprint field.
    pub stamp: Option<(std::ops::Range<usize>, String)>,
}

#[derive(Debug, Default)]
pub struct DocumentFences {
    pub fences: Vec<RawFence>,
    /// True when a mermaid fence never closed; it is skipped, and the caller
    /// warns rather than stamping a guess.
    pub unclosed: bool,
}

/// Line -> freshness for every stamp ATTACHED to a fence: true iff the
/// stamp's fingerprint still matches its own fence's source. Freshness is
/// per attachment, never per document: an identical fingerprint on a
/// DIFFERENT fence (a duplicated diagram whose sibling was edited) cannot
/// validate this stamp.
pub fn attached_stamp_freshness(
    found: &DocumentFences,
    text: &str,
) -> std::collections::HashMap<usize, bool> {
    found
        .fences
        .iter()
        .filter_map(|fence| {
            let (range, stamped) = fence.stamp.as_ref()?;
            let line = text[..range.start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            Some((line, *stamped == fingerprint(&fence.source)))
        })
        .collect()
}

/// Every closed mermaid fence in a deck document, in order, with any stamp
/// already on the line after it. Frontmatter lines are skipped.
pub fn fences_in_document(text: &str, frontmatter: Option<(usize, usize)>) -> DocumentFences {
    let mut found = DocumentFences::default();
    let mut open: Option<(char, usize, Option<Vec<String>>)> = None;
    let mut pending: Option<RawFence> = None;
    let mut offset = 0;
    for (index, raw_line) in text.split_inclusive('\n').enumerate() {
        let line_number = index + 1;
        let line_start = offset;
        offset += raw_line.len();
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some((open_line, close_line)) = frontmatter
            && line_number >= open_line
            && line_number <= close_line
        {
            continue;
        }
        if let Some(mut fence) = pending.take() {
            // The next line counts as this fence's stamp ONLY when the card
            // parser would produce the same DiagramStamp from it: a looser
            // rule here would let a malformed near-stamp with a current
            // fingerprint silently suppress freezing. The range stays the
            // whole original line, so a rewrite normalizes it.
            if let Some(stamp) = crate::parser::diagram_stamp_on_line(line) {
                fence.stamp = Some((line_start..line_start + line.len(), stamp.fingerprint));
                found.fences.push(fence);
                continue;
            }
            found.fences.push(fence);
        }
        match &mut open {
            Some((ch, len, body)) => {
                if crate::parser::closes_fence(line, *ch, *len) {
                    let (_, _, body) = open.take().expect("the fence is open");
                    if let Some(body) = body {
                        pending = Some(RawFence {
                            source: body.join("\n"),
                            insert_at: line_start + raw_line.len(),
                            stamp: None,
                        });
                    }
                } else if let Some(body) = body {
                    body.push(line.to_string());
                }
            }
            None => {
                if let Some((ch, len)) = crate::parser::fence_opener(line) {
                    let info = line.trim_start_matches(ch).trim();
                    let body = info.eq_ignore_ascii_case(LANGUAGE).then(Vec::new);
                    open = Some((ch, len, body));
                }
            }
        }
    }
    if let Some(fence) = pending {
        found.fences.push(fence);
    }
    if let Some((_, _, body)) = open {
        found.unclosed = body.is_some();
    }
    found
}

#[cfg(feature = "full")]
/// The full freeze computation for one rendered fence: extract the label
/// map, rasterize at ZOOM, and prove every label's ink lies inside its
/// emitted box (a per-label render diff, failing closed on any miss).
/// `sources` is the marker-probe assignment map (label id -> interior
/// byte range); a label it does not name is unbindable.
pub fn freeze_fence(
    svg: &str,
    family: &str,
    sources: &std::collections::HashMap<String, (u32, u32)>,
) -> Result<FrozenDiagram> {
    let found = geometry(svg)?;
    let full = raster(svg, family, ZOOM)?;
    let raster_size = (full.width(), full.height());
    let mut labels = Vec::with_capacity(found.labels.len());
    for label in &found.labels {
        let bounds = pixel_box(label, found.view_box, ZOOM, raster_size);
        let stripped = raster(&strip_label_texts(svg, &label.id)?, family, ZOOM)?;
        ink_within(&full, &stripped, bounds)
            .with_context(|| format!("label '{}' failed the ink-containment check", label.id))?;
        labels.push(GeometryLabel {
            id: label.id.clone(),
            text: label.text.clone(),
            source: match sources.get(&label.id) {
                Some((start, end)) => LabelSource::Range {
                    start: *start,
                    end: *end,
                },
                None => LabelSource::Unbindable,
            },
            bounds,
        });
    }
    let png = full.encode_png().context("the PNG cannot be encoded")?;
    let geometry = DiagramGeometry {
        image: crate::assets::object_name(&png, "png"),
        image_width: full.width(),
        image_height: full.height(),
        logical_width: found.view_box.width.ceil() as u32,
        logical_height: found.view_box.height.ceil() as u32,
        labels,
    };
    Ok(FrozenDiagram {
        image: png,
        geometry,
    })
}

#[cfg(feature = "full")]
pub struct FrozenDiagram {
    pub image: Vec<u8>,
    pub geometry: DiagramGeometry,
}

#[cfg(feature = "full")]
/// The per-fence cap on marker probes. Deterministic: probes beyond it are
/// dropped, the caller warns, and the unprobed occurrences stay unassigned
/// (their labels come out unbindable) instead of stalling authoring.
pub const PROBE_BUDGET: usize = 128;

#[cfg(feature = "full")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceProbe {
    pub start: usize,
    pub end: usize,
    /// The interior with this occurrence replaced by the marker.
    pub source: String,
}

#[cfg(feature = "full")]
/// Rule 8's linear probe plan: one probe per unique candidate occurrence per
/// DISTINCT rendered label text, in source order, never one pass per label.
/// Returns the marker, the probes, and whether the budget truncated them.
pub fn probe_plan(interior: &str, labels: &[Label]) -> (String, Vec<SourceProbe>, bool) {
    let mut texts: Vec<&str> = Vec::new();
    for label in labels {
        if !label.text.is_empty() && !texts.contains(&label.text.as_str()) {
            texts.push(&label.text);
        }
    }
    let marker = (1..)
        .map(|n| format!("xq{n}"))
        .find(|s| !interior.contains(s.as_str()) && !texts.contains(&s.as_str()))
        .expect("an unbounded counter leaves some marker unused");
    let mut probes = Vec::new();
    for text in &texts {
        for (start, _) in interior.match_indices(text) {
            let end = start + text.len();
            let mut source = String::with_capacity(interior.len());
            source.push_str(&interior[..start]);
            source.push_str(&marker);
            source.push_str(&interior[end..]);
            probes.push(SourceProbe { start, end, source });
        }
    }
    probes.sort_by_key(|probe| (probe.start, probe.end));
    let truncated = probes.len() > PROBE_BUDGET;
    probes.truncate(PROBE_BUDGET);
    (marker, probes, truncated)
}

#[cfg(feature = "full")]
/// The assignment law: a probe assigns iff EXACTLY ONE label with an
/// unchanged semantic id carries the marker as its rendered text. Zero
/// leaves the occurrence unassigned; multiple is ambiguous and assigns
/// nothing.
pub fn probe_assignment(original: &[Label], marker: &str, variant: &[Label]) -> Option<String> {
    let known = |id: &str| original.iter().any(|label| label.id == id);
    let mut hits = variant
        .iter()
        .filter(|label| label.text == marker && known(&label.id));
    match (hits.next(), hits.next()) {
        (Some(hit), None) => Some(hit.id.clone()),
        _ => None,
    }
}

#[cfg(feature = "full")]
/// Collates probe assignments into the per-label source map: a label
/// assigned by zero probes or by more than one stays absent (unbindable),
/// and a range that cannot convert to u32 is dropped (checked, per the
/// producer-side conversion law).
pub fn collate_assignments(
    assigned: &[(String, usize, usize)],
) -> std::collections::HashMap<String, (u32, u32)> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (id, ..) in assigned {
        *counts.entry(id).or_default() += 1;
    }
    assigned
        .iter()
        .filter(|(id, ..)| counts.get(id.as_str()) == Some(&1))
        .filter_map(|(id, start, end)| {
            let start = u32::try_from(*start).ok()?;
            let end = u32::try_from(*end).ok()?;
            Some((id.clone(), (start, end)))
        })
        .collect()
}

#[cfg(feature = "full")]
/// The bare-node signature, two-sided. Variant side: a plain probe of a
/// bare occurrence RENAMES the node, so a label's id and text are both the
/// marker. Original side: the occurrence text must itself name a bare
/// label (id == text == occurrence), or the marker node was fabricated by
/// replacing a bare ID REFERENCE to some labeled node, and a retry would
/// bind that node's visible label to id bytes.
pub fn bare_rename(occurrence: &str, original: &[Label], marker: &str, variant: &[Label]) -> bool {
    original
        .iter()
        .any(|label| label.id == occurrence && label.text == occurrence)
        && variant
            .iter()
            .any(|label| label.id == marker && label.text == marker)
}

#[cfg(feature = "full")]
/// The pass-2 probe for a bare occurrence: `text[marker]` relabels the
/// node in place instead of renaming it, so the unchanged-id law can
/// accept the same occurrence the plain probe had to reject.
pub fn bracket_probe(interior: &str, probe: &SourceProbe, marker: &str) -> SourceProbe {
    let text = &interior[probe.start..probe.end];
    let mut source = String::with_capacity(interior.len() + marker.len() + 2);
    source.push_str(&interior[..probe.start]);
    source.push_str(text);
    source.push('[');
    source.push_str(marker);
    source.push(']');
    source.push_str(&interior[probe.end..]);
    SourceProbe {
        start: probe.start,
        end: probe.end,
        source,
    }
}

#[cfg(feature = "full")]
/// The scale frozen rasters render at; the geometry records both raster and
/// logical dimensions so clients display at logical size.
pub const ZOOM: f32 = 2.0;

#[cfg(feature = "full")]
/// Emitted boxes are inflated by this many SVG units per side: measured ink
/// margins are ~3 units on edge labels, and the pad insures against fonts
/// with slightly wider metrics than the measured corpus.
const BOX_PAD: f32 = 2.0;

#[cfg(feature = "full")]
/// Families tried in order for the freeze; sekien measures and resvg draws
/// with the same one, and the ink-containment check verifies the result.
const FAMILIES: &[&str] = &[
    "DejaVu Sans",
    "Liberation Sans",
    "Noto Sans",
    "Arial",
    "Helvetica",
];

#[cfg(feature = "full")]
/// The first preference-list family the system fontdb can resolve.
pub fn available_family() -> Result<&'static str> {
    let db = system_fonts();
    FAMILIES
        .iter()
        .copied()
        .find(|family| {
            let query = usvg::fontdb::Query {
                families: &[usvg::fontdb::Family::Name(family)],
                ..Default::default()
            };
            db.query(&query).is_some()
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "none of the diagram font families ({}) is installed",
                FAMILIES.join(", ")
            )
        })
}

#[cfg(feature = "full")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[cfg(feature = "full")]
/// One bindable label: a flowchart node (`id` is the mermaid node id) or an
/// edge label (`id` is the renderer's stable `L_<src>_<tgt>_<n>`).
/// Coordinates are absolute SVG units after transform accumulation.
#[derive(Debug, PartialEq)]
pub struct Label {
    pub id: String,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[cfg(feature = "full")]
#[derive(Debug)]
pub struct Geometry {
    pub view_box: ViewBox,
    pub labels: Vec<Label>,
}

#[cfg(feature = "full")]
/// Every bindable label in a rendered SVG, with the root viewBox needed to
/// map SVG units into raster pixels. Only translate() transforms are
/// accepted: any other transform above a label would silently misplace its
/// box, so it fails closed instead.
pub fn geometry(svg: &str) -> Result<Geometry> {
    let document =
        usvg::roxmltree::Document::parse(svg).context("the SVG cannot be parsed as XML")?;
    let root = document.root_element();
    let view_box = parse_view_box(root.attribute("viewBox"))?;
    let mut labels = Vec::new();
    collect_labels(root, 0.0, 0.0, &mut labels)?;
    Ok(Geometry { view_box, labels })
}

#[cfg(feature = "full")]
fn parse_view_box(attribute: Option<&str>) -> Result<ViewBox> {
    let raw = attribute.unwrap_or_default();
    let parts: Vec<f32> = raw
        .split_whitespace()
        .filter_map(|part| part.parse().ok())
        .collect();
    let [x, y, width, height] = parts[..] else {
        bail!("the SVG root has no usable viewBox ('{raw}')");
    };
    Ok(ViewBox {
        x,
        y,
        width,
        height,
    })
}

#[cfg(feature = "full")]
fn translation(node: usvg::roxmltree::Node) -> Result<(f32, f32)> {
    let Some(raw) = node.attribute("transform") else {
        return Ok((0.0, 0.0));
    };
    let inner = raw
        .trim()
        .strip_prefix("translate(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| anyhow::anyhow!("unsupported transform '{raw}' above a diagram label"))?;
    let mut parts = inner.split([',', ' ']).filter(|part| !part.is_empty());
    let x: f32 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("unsupported transform '{raw}' above a diagram label"))?;
    let y: f32 = match parts.next() {
        Some(part) => part
            .parse()
            .map_err(|_| anyhow::anyhow!("unsupported transform '{raw}' above a diagram label"))?,
        None => 0.0,
    };
    Ok((x, y))
}

#[cfg(feature = "full")]
fn text_of(node: usvg::roxmltree::Node) -> String {
    node.descendants()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect()
}

#[cfg(feature = "full")]
fn element_class<'a>(node: usvg::roxmltree::Node<'a, 'a>) -> &'a str {
    node.attribute("class").unwrap_or_default()
}

#[cfg(feature = "full")]
fn collect_labels(
    node: usvg::roxmltree::Node,
    x: f32,
    y: f32,
    labels: &mut Vec<Label>,
) -> Result<()> {
    // <defs> content renders only through <use>, never in place, so its
    // transforms (sequence actor icons keep scale()d paths there) cannot
    // sit above a rendered label; everything rendered stays fail-closed.
    if node.tag_name().name() == "defs" {
        return Ok(());
    }
    let (dx, dy) = translation(node)?;
    let (x, y) = (x + dx, y + dy);
    let class = element_class(node);
    if class.split_whitespace().any(|word| word == "node")
        && let Some(id) = node.attribute("id").and_then(flowchart_node_id)
    {
        let container = node
            .children()
            .filter(|child| child.is_element())
            .find(|child| {
                element_class(*child)
                    .split_whitespace()
                    .any(|word| word == "label-container")
            })
            .ok_or_else(|| {
                anyhow::anyhow!("node '{id}' has no label-container this extraction recognizes")
            })?;
        let (left, top, width, height) = container_bounds(container)
            .with_context(|| format!("node '{id}' has an unmeasurable label-container"))?;
        labels.push(Label {
            id,
            text: text_of(node).trim().to_string(),
            x: x + left,
            y: y + top,
            width,
            height,
        });
        return Ok(());
    }
    if class == "edgeLabel" {
        for child in node.children() {
            if element_class(child) == "label"
                && let Some(id) = child.attribute("data-id")
            {
                let (lx, ly) = translation(child)?;
                let Some(rect) = descendant_rect(child, |rect| element_class(rect) == "background")
                else {
                    continue;
                };
                labels.push(label_from_rect(
                    id.to_string(),
                    text_of(child),
                    rect,
                    x + lx,
                    y + ly,
                )?);
            }
        }
        return Ok(());
    }
    for child in node.children().filter(|child| child.is_element()) {
        collect_labels(child, x, y, labels)?;
    }
    Ok(())
}

#[cfg(feature = "full")]
/// `d1-flowchart-A-0` -> `A`; the leading svg id varies per render and the
/// trailing counter is the renderer's, so neither is part of the identity.
fn flowchart_node_id(raw: &str) -> Option<String> {
    let (_, tail) = raw.split_once("-flowchart-")?;
    let (id, counter) = tail.rsplit_once('-')?;
    counter
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| id.to_string())
}

#[cfg(feature = "full")]
/// The bounding box of one label-container shape, in its parent node's
/// frame (the shape's own translate included). Mermaid draws nodes as
/// rects, circles, ellipses, polygons, paths (`[(db)]`), or groups of
/// paths (`([stadium])` in some looks); anything else fails closed so a
/// future shape errors instead of silently vanishing from the map.
fn container_bounds(shape: usvg::roxmltree::Node) -> Result<(f32, f32, f32, f32)> {
    let (dx, dy) = translation(shape)?;
    let attr = |name: &str| -> Result<f32> {
        shape
            .attribute(name)
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("missing numeric '{name}'"))
    };
    let optional = |name: &str| shape.attribute(name).and_then(|value| value.parse().ok());
    let from_extent = |bounds: (f32, f32, f32, f32)| {
        let (min_x, min_y, max_x, max_y) = bounds;
        (dx + min_x, dy + min_y, max_x - min_x, max_y - min_y)
    };
    Ok(match shape.tag_name().name() {
        "rect" => (
            dx + optional("x").unwrap_or(0.0),
            dy + optional("y").unwrap_or(0.0),
            attr("width")?,
            attr("height")?,
        ),
        "circle" => {
            let r = attr("r")?;
            (
                dx + optional("cx").unwrap_or(0.0) - r,
                dy + optional("cy").unwrap_or(0.0) - r,
                2.0 * r,
                2.0 * r,
            )
        }
        "ellipse" => {
            let (rx, ry) = (attr("rx")?, attr("ry")?);
            (
                dx + optional("cx").unwrap_or(0.0) - rx,
                dy + optional("cy").unwrap_or(0.0) - ry,
                2.0 * rx,
                2.0 * ry,
            )
        }
        "polygon" => {
            let points = shape.attribute("points").unwrap_or_default();
            let values: Vec<f32> = points
                .split([',', ' '])
                .filter(|part| !part.is_empty())
                .map(|part| part.parse::<f32>())
                .collect::<Result<_, _>>()
                .map_err(|_| anyhow::anyhow!("unparseable polygon points '{points}'"))?;
            from_extent(extent_of_pairs(&values)?)
        }
        "path" => from_extent(path_extent(
            shape
                .attribute("d")
                .ok_or_else(|| anyhow::anyhow!("path without 'd'"))?,
        )?),
        "g" => {
            let mut children = shape.children().filter(|child| child.is_element());
            let mut union = children
                .next()
                .map(container_bounds)
                .transpose()?
                .ok_or_else(|| anyhow::anyhow!("empty label-container group"))?;
            for child in children {
                let (x, y, width, height) = container_bounds(child)?;
                let right = (union.0 + union.2).max(x + width);
                let bottom = (union.1 + union.3).max(y + height);
                union.0 = union.0.min(x);
                union.1 = union.1.min(y);
                union.2 = right - union.0;
                union.3 = bottom - union.1;
            }
            (dx + union.0, dy + union.1, union.2, union.3)
        }
        other => bail!("unsupported label-container shape <{other}>"),
    })
}

#[cfg(feature = "full")]
fn extent_of_pairs(values: &[f32]) -> Result<(f32, f32, f32, f32)> {
    if values.len() < 2 || !values.len().is_multiple_of(2) {
        bail!("expected coordinate pairs, got {} values", values.len());
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for pair in values.chunks(2) {
        min_x = min_x.min(pair[0]);
        max_x = max_x.max(pair[0]);
        min_y = min_y.min(pair[1]);
        max_y = max_y.max(pair[1]);
    }
    Ok((min_x, min_y, max_x, max_y))
}

#[cfg(feature = "full")]
/// A path's extent over every visited point (endpoints and curve control
/// points; arc endpoints only, so an arc's bulge can undershoot by its
/// radius). Good enough for a mask box: the ink-containment proof measures
/// the TEXT, which sits well inside every mermaid shape, and refuses the
/// freeze if it ever does not.
fn path_extent(d: &str) -> Result<(f32, f32, f32, f32)> {
    let mut numbers: Vec<f32> = Vec::new();
    let mut command = None;
    let mut points: Vec<f32> = Vec::new();
    let mut cursor = (0.0f32, 0.0f32);
    let mut start = (0.0f32, 0.0f32);
    let mut buffer = String::new();
    let flush = |buffer: &mut String, numbers: &mut Vec<f32>| -> Result<()> {
        if !buffer.is_empty() {
            numbers.push(
                buffer
                    .parse()
                    .map_err(|_| anyhow::anyhow!("unparseable path number '{buffer}'"))?,
            );
            buffer.clear();
        }
        Ok(())
    };
    let mut apply = |command: char, numbers: &mut Vec<f32>| -> Result<()> {
        let relative = command.is_ascii_lowercase();
        let arity = match command.to_ascii_uppercase() {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            'Z' => 0,
            other => bail!("unsupported path command '{other}'"),
        };
        if command.eq_ignore_ascii_case(&'Z') {
            cursor = start;
            return Ok(());
        }
        if numbers.is_empty() || !numbers.len().is_multiple_of(arity) {
            bail!("path command '{command}' with {} numbers", numbers.len());
        }
        for group in numbers.chunks(arity) {
            let base = if relative { cursor } else { (0.0, 0.0) };
            match command.to_ascii_uppercase() {
                'H' => cursor.0 = base.0 + group[0],
                'V' => cursor.1 = base.1 + group[0],
                'A' => {
                    cursor = (base.0 + group[5], base.1 + group[6]);
                }
                _ => {
                    for pair in group.chunks(2) {
                        cursor = (base.0 + pair[0], base.1 + pair[1]);
                        points.push(cursor.0);
                        points.push(cursor.1);
                    }
                }
            }
            points.push(cursor.0);
            points.push(cursor.1);
            if command.eq_ignore_ascii_case(&'M') {
                start = cursor;
            }
        }
        numbers.clear();
        Ok(())
    };
    for character in d.chars() {
        let is_command = character.is_ascii_alphabetic() && character != 'e' && character != 'E';
        if is_command {
            flush(&mut buffer, &mut numbers)?;
            if let Some(previous) = command {
                apply(previous, &mut numbers)?;
            }
            command = Some(character);
        } else if character == ',' || character.is_whitespace() {
            flush(&mut buffer, &mut numbers)?;
        } else if character == '-' && !buffer.is_empty() && !buffer.ends_with('e') {
            flush(&mut buffer, &mut numbers)?;
            buffer.push(character);
        } else {
            buffer.push(character);
        }
    }
    flush(&mut buffer, &mut numbers)?;
    if let Some(previous) = command {
        apply(previous, &mut numbers)?;
    }
    if points.is_empty() {
        bail!("path with no coordinates");
    }
    extent_of_pairs(&points)
}

#[cfg(feature = "full")]
fn descendant_rect<'a>(
    node: usvg::roxmltree::Node<'a, 'a>,
    keep: impl Fn(usvg::roxmltree::Node) -> bool,
) -> Option<usvg::roxmltree::Node<'a, 'a>> {
    node.descendants()
        .find(|child| child.has_tag_name("rect") && keep(*child))
}

#[cfg(feature = "full")]
fn label_from_rect(
    id: String,
    text: String,
    rect: usvg::roxmltree::Node,
    x: f32,
    y: f32,
) -> Result<Label> {
    let field = |name: &str| -> Result<f32> {
        rect.attribute(name)
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("label '{id}' has a rect without a numeric '{name}'"))
    };
    Ok(Label {
        x: x + field("x")?,
        y: y + field("y")?,
        width: field("width")?,
        height: field("height")?,
        id,
        text: text.trim().to_string(),
    })
}

#[cfg(feature = "full")]
/// A label's box in raster pixel space: shifted by the viewBox origin,
/// inflated by BOX_PAD per side, scaled by `zoom`, rounded outward, and
/// clamped to the raster.
pub fn pixel_box(label: &Label, view_box: ViewBox, zoom: f32, raster: (u32, u32)) -> PixelBox {
    let scale = |value: f32| value * zoom;
    let left = scale(label.x - view_box.x - BOX_PAD).floor().max(0.0) as u32;
    let top = scale(label.y - view_box.y - BOX_PAD).floor().max(0.0) as u32;
    let right = scale(label.x - view_box.x + label.width + BOX_PAD).ceil() as u32;
    let bottom = scale(label.y - view_box.y + label.height + BOX_PAD).ceil() as u32;
    let (image_width, image_height) = raster;
    let right = right.min(image_width);
    let bottom = bottom.min(image_height);
    PixelBox {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The persisted half of a frozen diagram: the PNG's object name, both
/// dimension pairs, and the complete bindable-label map. Complete rather
/// than selected-only: retargeting a span is an ordinary edit that must not
/// invalidate the frozen pair.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagramGeometry {
    pub image: String,
    pub image_width: u32,
    pub image_height: u32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub labels: Vec<GeometryLabel>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryLabel {
    pub id: String,
    pub text: String,
    pub source: LabelSource,
    #[serde(flatten)]
    pub bounds: PixelBox,
}

/// Where the label's text lives in the fence source: a byte range in the
/// LF-normalized interior (end-exclusive), or nothing bindable (multiline,
/// entity- or markdown-processed, bare-id labels, ambiguous probes).
/// Required, so a geometry frozen before this field fails loud as ordinary
/// invalid input. Consumers validate a range against the interior bytes
/// (bounds, char boundaries, overlap) before any slice.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum LabelSource {
    Range { start: u32, end: u32 },
    Unbindable,
}

/// Why a fence's spans cannot project onto its frozen raster. Review
/// silently falls back to the masked source; doctor and the deck-load
/// diagnostic are the loud channels that speak these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindFailure {
    /// A persisted range fails bounds, ordering, or a char boundary against
    /// the interior; a corrupt or hostile geometry lands here instead of
    /// panicking review.
    InvalidLabelRange {
        label: String,
    },
    OverlappingLabelRanges {
        first: String,
        second: String,
    },
    /// The span's range is a proper part of one label's range; a diagram
    /// span must cover the complete label.
    SpanIncompleteLabel {
        line: usize,
        label: String,
    },
    /// The span's range matches no label's source position (an id, a
    /// comment, an unbindable label's text).
    SpanOutsideLabels {
        line: usize,
    },
}

impl std::fmt::Display for BindFailure {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindFailure::InvalidLabelRange { label } => {
                write!(out, "label '{label}' carries an invalid source range")
            }
            BindFailure::OverlappingLabelRanges { first, second } => {
                write!(
                    out,
                    "labels '{first}' and '{second}' claim overlapping source ranges"
                )
            }
            BindFailure::SpanIncompleteLabel { line, label } => write!(
                out,
                "the span at line {line} covers part of label '{label}'; a diagram span must cover the complete label"
            ),
            BindFailure::SpanOutsideLabels { line } => write!(
                out,
                "the span at line {line} does not cover a diagram label (ids, arrows, and keywords are never maskable)"
            ),
        }
    }
}

/// Validates EVERY persisted label range against the interior bytes before
/// anything slices: checked bounds, start < end, char boundaries, and
/// cross-label overlap, including labels no card span targets.
pub fn validate_label_sources(
    geometry: &DiagramGeometry,
    interior: &str,
) -> Result<(), BindFailure> {
    let mut ranges: Vec<(usize, usize, &str)> = Vec::new();
    for label in &geometry.labels {
        let LabelSource::Range { start, end } = label.source else {
            continue;
        };
        let (start, end) = (start as usize, end as usize);
        let valid = start < end
            && end <= interior.len()
            && interior.is_char_boundary(start)
            && interior.is_char_boundary(end);
        if !valid {
            return Err(BindFailure::InvalidLabelRange {
                label: label.id.clone(),
            });
        }
        ranges.push((start, end, &label.id));
    }
    ranges.sort_unstable();
    for pair in ranges.windows(2) {
        let [(_, first_end, first), (second_start, _, second)] = pair else {
            continue;
        };
        if second_start < first_end {
            return Err(BindFailure::OverlappingLabelRanges {
                first: (*first).to_string(),
                second: (*second).to_string(),
            });
        }
    }
    Ok(())
}

/// A span binds iff its interior byte range EQUALS one label's source
/// range (complete-label-only). Run `validate_label_sources` first; this
/// only compares.
pub fn bind_span(
    geometry: &DiagramGeometry,
    line: usize,
    start: usize,
    end: usize,
) -> Result<usize, BindFailure> {
    for (index, label) in geometry.labels.iter().enumerate() {
        let LabelSource::Range {
            start: label_start,
            end: label_end,
        } = label.source
        else {
            continue;
        };
        let (label_start, label_end) = (label_start as usize, label_end as usize);
        if (start, end) == (label_start, label_end) {
            return Ok(index);
        }
        if start >= label_start && end <= label_end {
            return Err(BindFailure::SpanIncompleteLabel {
                line,
                label: label.id.clone(),
            });
        }
    }
    Err(BindFailure::SpanOutsideLabels { line })
}

#[cfg(feature = "full")]
/// The SVG with the given label's `<text>` elements removed, for the
/// ink-containment diff. The target is a node's `id` or an edge label's
/// `data-id`.
pub(crate) fn strip_label_texts(svg: &str, label_id: &str) -> Result<String> {
    let document =
        usvg::roxmltree::Document::parse(svg).context("the SVG cannot be parsed as XML")?;
    let target = document
        .descendants()
        .find(|node| {
            node.attribute("id")
                .and_then(flowchart_node_id)
                .is_some_and(|id| id == label_id)
                || (node.attribute("data-id") == Some(label_id) && element_class(*node) == "label")
        })
        .ok_or_else(|| anyhow::anyhow!("label '{label_id}' is not in the SVG"))?;
    let mut ranges: Vec<std::ops::Range<usize>> = target
        .descendants()
        .filter(|node| node.has_tag_name("text"))
        .map(|node| node.range())
        .collect();
    if ranges.is_empty() {
        bail!("label '{label_id}' has no text to strip");
    }
    ranges.sort_by_key(|range| range.start);
    let mut stripped = String::with_capacity(svg.len());
    let mut cursor = 0;
    for range in ranges {
        stripped.push_str(&svg[cursor..range.start]);
        cursor = range.end;
    }
    stripped.push_str(&svg[cursor..]);
    Ok(stripped)
}

#[cfg(feature = "full")]
/// The containment law: the label must leave ink (a fontless or dropped
/// render is a corrupt freeze), and every pixel it changes must lie inside
/// its emitted box (ink outside would survive the mask).
pub(crate) fn ink_within(
    full: &resvg::tiny_skia::Pixmap,
    stripped: &resvg::tiny_skia::Pixmap,
    bounds: PixelBox,
) -> Result<()> {
    if full.width() != stripped.width() || full.height() != stripped.height() {
        bail!("the stripped render changed size, so the diff is meaningless");
    }
    let mut changed = 0u32;
    for y in 0..full.height() {
        for x in 0..full.width() {
            if full.pixel(x, y) != stripped.pixel(x, y) {
                changed += 1;
                let inside = x >= bounds.x
                    && x < bounds.x + bounds.width
                    && y >= bounds.y
                    && y < bounds.y + bounds.height;
                if !inside {
                    bail!(
                        "label ink at {x},{y} falls outside its box, where a mask cannot cover it"
                    );
                }
            }
        }
    }
    if changed == 0 {
        bail!("the label left no ink in the render, so its text was dropped");
    }
    Ok(())
}

#[cfg(feature = "full")]
fn system_fonts() -> &'static usvg::fontdb::Database {
    static FONTS: std::sync::OnceLock<usvg::fontdb::Database> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

#[cfg(feature = "full")]
/// Renders `sources` in one long-lived process, returning one outcome per
/// input in input order.
///
/// Two protocol facts drive this, both measured against sekien 0.4.1 rather
/// than assumed: a diagram that fails emits NOTHING on stdout, so inputs and
/// SVGs cannot be paired positionally, and the process exits 0 even when
/// diagrams fail, so the exit code is never a per-diagram verdict. Both
/// streams are therefore correlated by the `--meta` id.
pub fn render_batch(
    command: &str,
    font: Option<&str>,
    sources: &[String],
    timeout: Duration,
) -> Result<Vec<Rendered>> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let mut command_line = Command::new(command);
    command_line.arg("--meta");
    if let Some(font) = font {
        command_line.args(["--font", font]);
    }
    let mut child = command_line
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run '{command}' — is it installed?"))?;

    let mut stdin = child.stdin.take().expect("stdin was piped");
    let stdout_pipe = child.stdout.take().expect("stdout was piped");
    let stderr_pipe = child.stderr.take().expect("stderr was piped");

    let payload = sources.join("\0").into_bytes();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&payload);
    });
    // Both pipes drain concurrently so a full one cannot deadlock the child.
    let out = std::thread::spawn(move || drain(stdout_pipe));
    let err = std::thread::spawn(move || drain(stderr_pipe));

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("'{command}' did not finish within {}s", timeout.as_secs());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(error).context("cannot wait for the renderer"),
        }
    }
    let _ = writer.join();
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();

    let svgs = by_meta_id(&stdout);
    let errors = by_meta_id(&stderr);
    let mut outcomes = Vec::with_capacity(sources.len());
    for index in 0..sources.len() {
        // sekien's ids are 1-based and count inputs, not outputs.
        let id = index + 1;
        let svg = svgs.iter().find(|(found, _)| *found == id);
        let error = errors.iter().find(|(found, _)| *found == id);
        outcomes.push(match (svg, error) {
            (Some((_, svg)), _) if !svg.is_empty() => Ok(svg.clone()),
            (_, Some((_, message))) => Err(message.clone()),
            _ => Err(NO_RENDER_OUTPUT.to_string()),
        });
    }
    Ok(outcomes)
}

#[cfg(feature = "full")]
/// The batch outcome for a diagram the renderer never answered: an
/// OPERATIONAL failure (truncated stream, dead child), unlike a stderr
/// message, which is the renderer processing and rejecting the input.
pub const NO_RENDER_OUTPUT: &str = "the renderer returned nothing for this diagram";

#[cfg(feature = "full")]
fn drain<R: Read>(mut pipe: R) -> String {
    let mut buffer = Vec::new();
    let _ = pipe.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

#[cfg(feature = "full")]
/// Splits a `--meta` stream into `(id, body)` pairs. The marker is
/// `<!-- {"id": N} -->`; anything before the first marker is renderer preamble
/// and is dropped.
fn by_meta_id(stream: &str) -> Vec<(usize, String)> {
    const OPEN: &str = "<!-- {\"id\":";
    let mut found = Vec::new();
    let mut rest = stream;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(close) = after.find("-->") else {
            break;
        };
        let id: usize = match after[..close].trim().trim_end_matches('}').trim().parse() {
            Ok(id) => id,
            Err(_) => {
                rest = &after[close + 3..];
                continue;
            }
        };
        let body_start = start + OPEN.len() + close + 3;
        let body = &rest[body_start..];
        let end = body.find(OPEN).unwrap_or(body.len());
        found.push((id, body[..end].trim_matches(['\0', '\n', ' ']).to_string()));
        rest = &body[end..];
    }
    found
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::testutil::{exec_lock, fake_cli};

    /// A fake sekien: drains stdin, then replays a prepared stdout and stderr.
    /// The correlation logic is what is under test, not the real renderer.
    fn fake_sekien(dir: &Path, stdout: &str, stderr: &str) -> PathBuf {
        let out = dir.join("out");
        let err = dir.join("err");
        std::fs::write(&out, stdout).unwrap();
        std::fs::write(&err, stderr).unwrap();
        fake_cli(
            dir,
            &format!(
                "cat >/dev/null; cat {}; cat {} >&2; exit 0",
                out.display(),
                err.display()
            ),
        )
    }

    fn meta(id: usize, body: &str) -> String {
        format!("<!-- {{\"id\": {id}}} -->\n{body}")
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn only_a_mermaid_fence_is_found_whatever_spells_it() {
        let cases = [
            ("```mermaid", true),
            ("``` mermaid", true),
            ("```MERMaid", true),
            ("~~~mermaid", true),
            ("```mermaid   ", true),
            ("```rust", false),
            ("```", false),
            ("```mermaidish", false),
        ];
        for (opener, expected) in cases {
            let block = lines(&format!("{opener}\nflowchart LR\n A-->B\n```"));
            let found = fences(&block);
            assert_eq!(
                expected,
                !found.is_empty(),
                "opener {opener:?} should {}be a diagram",
                if expected { "" } else { "not " }
            );
        }
    }

    #[test]
    fn a_fence_yields_its_interior_verbatim_without_the_delimiters() {
        let block = lines("prose\n```mermaid\nflowchart LR\n  A[hi] --> B\n```\nmore");
        let found = fences(&block);
        assert_eq!(1, found.len());
        assert_eq!("flowchart LR\n  A[hi] --> B", found[0].source);
        assert_eq!(1, found[0].opener, "the opener line index");
    }

    /// A non-mermaid fence must be consumed, not skipped: otherwise its
    /// interior can be read as a later mermaid fence's content.
    #[test]
    fn a_mermaid_line_inside_another_fence_is_not_a_diagram() {
        let block = lines("```text\n```mermaid\nflowchart LR\n A-->B\n```\n```");
        assert_eq!(Vec::<Fence>::new(), fences(&block));
    }

    #[test]
    fn a_shorter_delimiter_stays_inside_a_longer_mermaid_fence() {
        let block = lines("````mermaid\nflowchart LR\n```\n A-->B\n````");
        let found = fences(&block);
        assert_eq!(1, found.len(), "{found:?}");
        assert_eq!("flowchart LR\n```\n A-->B", found[0].source);
    }

    #[test]
    fn several_fences_come_back_in_document_order() {
        let block =
            lines("```mermaid\nfirst\n```\n```rust\nlet x = 1;\n```\n```mermaid\nsecond\n```");
        let found = fences(&block);
        assert_eq!(
            vec!["first".to_string(), "second".to_string()],
            found.iter().map(|f| f.source.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_unclosed_fence_runs_to_the_end_of_the_block() {
        let block = lines("```mermaid\nflowchart LR\n A-->B");
        let found = fences(&block);
        assert_eq!(1, found.len());
        assert_eq!("flowchart LR\n A-->B", found[0].source);
    }

    /// The preimage is the interior alone: the same diagram written with a
    /// different fence character or info-string casing is the same picture and
    /// must reuse the same frozen asset.
    /// One fence, one fingerprint, whoever asks: the scanner sees raw CRLF
    /// bytes, the parser sees stripped lines, and the preimage must not
    /// split them. A lone CR stays content (pinned as deliberate).
    #[test]
    fn the_fingerprint_is_line_ending_insensitive_but_lone_cr_is_content() {
        assert_eq!(
            fingerprint("flowchart LR\r\n A-->B"),
            fingerprint("flowchart LR\n A-->B"),
            "CRLF and LF spellings are the same picture"
        );
        assert_ne!(
            fingerprint("flowchart LR\r A-->B"),
            fingerprint("flowchart LR\n A-->B"),
            "a lone CR is content, not a terminator"
        );
    }

    #[test]
    fn the_fingerprint_covers_the_interior_and_nothing_else() {
        let backtick = fences(&lines("```mermaid\nflowchart LR\n A-->B\n```"));
        let tilde = fences(&lines("~~~MERMAID\nflowchart LR\n A-->B\n~~~"));
        assert_eq!(
            fingerprint(&backtick[0].source),
            fingerprint(&tilde[0].source),
            "the fence syntax is not part of the picture"
        );
        assert_ne!(
            fingerprint("flowchart LR\n A-->B"),
            fingerprint("flowchart LR\n A-->C"),
            "an edited diagram must miss its old asset"
        );
        assert!(fingerprint("x").starts_with("xxh64-"));
    }

    fn sources(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("flowchart LR\n A{i}-->B{i}"))
            .collect()
    }

    #[test]
    fn every_input_gets_one_outcome_in_input_order() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let stdout = [meta(1, "<svg>one</svg>"), meta(2, "<svg>two</svg>")].join("\0");
        let cli = fake_sekien(dir.path(), &stdout, "");
        let out = render_batch(
            cli.to_str().unwrap(),
            None,
            &sources(2),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(2, out.len());
        assert_eq!(Ok("<svg>one</svg>".to_string()), out[0]);
        assert_eq!(Ok("<svg>two</svg>".to_string()), out[1]);
    }

    /// The law this module exists for: a failed diagram emits nothing on
    /// stdout, so pairing inputs to SVGs positionally misattributes every
    /// later result. Input 2 fails; inputs 1 and 3 must keep their own SVGs.
    #[test]
    fn a_failed_diagram_shifts_nothing_because_ids_correlate_both_streams() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let stdout = [meta(1, "<svg>first</svg>"), meta(3, "<svg>third</svg>")].join("\0");
        let stderr = meta(2, "Parse error on line 2");
        let cli = fake_sekien(dir.path(), &stdout, &stderr);
        let out = render_batch(
            cli.to_str().unwrap(),
            None,
            &sources(3),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(
            vec![
                Ok("<svg>first</svg>".to_string()),
                Err("Parse error on line 2".to_string()),
                Ok("<svg>third</svg>".to_string()),
            ],
            out,
            "a positional pairing would hand input 2 the third SVG"
        );
    }

    #[test]
    fn a_zero_exit_with_failures_is_still_read_per_diagram() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // Every diagram failed, yet the process exits 0 — the measured behavior.
        let stderr = [
            meta(1, "No diagram type detected"),
            meta(2, "Lexical error"),
        ]
        .join("");
        let cli = fake_sekien(dir.path(), "", &stderr);
        let out = render_batch(
            cli.to_str().unwrap(),
            None,
            &sources(2),
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(out.iter().all(|outcome| outcome.is_err()), "{out:?}");
        assert_eq!(Err("No diagram type detected".to_string()), out[0]);
    }

    #[test]
    fn a_silent_renderer_leaves_every_input_accounted_for() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_sekien(dir.path(), "", "");
        let out = render_batch(
            cli.to_str().unwrap(),
            None,
            &sources(2),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(2, out.len());
        assert!(out.iter().all(|outcome| outcome.is_err()));
    }

    #[test]
    fn an_empty_batch_never_spawns_the_renderer() {
        // A missing binary would fail the spawn, so reaching Ok proves no spawn.
        let out = render_batch(
            "definitely-not-a-real-binary-xyz",
            None,
            &[],
            Duration::from_secs(1),
        );
        assert_eq!(0, out.unwrap().len());
    }

    #[test]
    fn a_missing_renderer_names_the_command() {
        let _guard = exec_lock();
        let out = render_batch(
            "definitely-not-a-real-binary-xyz",
            None,
            &sources(1),
            Duration::from_secs(1),
        );
        let message = out.unwrap_err().to_string();
        assert!(
            message.contains("definitely-not-a-real-binary-xyz"),
            "{message}"
        );
        assert!(message.contains("is it installed?"), "{message}");
    }

    /// PNG magic plus IHDR dimensions, read raw so the tests need no decoder.
    fn png_size(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
        let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        (word(16), word(20))
    }

    const RECT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8" viewBox="0 0 10 8"><style>#d1{fill:#333;}</style><rect width="10" height="8" fill="#ff0000"/></svg>"##;

    #[test]
    fn a_text_free_svg_rasterizes_at_the_requested_scale_with_any_family() {
        let raster = rasterize(RECT_SVG, "no-such-family-xyz", 1.0).unwrap();
        assert_eq!(
            (10, 8),
            png_size(&raster.image),
            "zoom 1 keeps intrinsic size"
        );
        assert_eq!(
            (raster.width, raster.height),
            png_size(&raster.image),
            "reported size is the IHDR size"
        );
        let raster = rasterize(RECT_SVG, "no-such-family-xyz", 2.5).unwrap();
        assert_eq!(
            (25, 20),
            png_size(&raster.image),
            "zoom scales both dimensions"
        );
    }

    fn pixel(pixmap: &resvg::tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let p = pixmap.pixel(x, y).unwrap();
        (p.red(), p.green(), p.blue(), p.alpha())
    }

    /// The content must scale WITH the canvas: a mutant that grows the
    /// pixmap but draws at 1x leaves the far corner background-colored.
    #[test]
    fn zoom_scales_the_drawing_not_just_the_canvas() {
        let pixmap = raster(RECT_SVG, "any", 2.5).unwrap();
        assert_eq!(
            (255, 0, 0, 255),
            pixel(&pixmap, 24, 19),
            "the rect must cover the far corner at zoom 2.5"
        );
    }

    /// Real dark-theme sekien output with its `<text>` elements stripped so
    /// no host font is needed; the light `#ccc` strokes remain, which is the
    /// ink a white ground would erase.
    const DARK_THEME_SEKIEN: &str = include_str!("../tests/fixtures/dark-theme-sekien.svg");

    /// The ground must follow the diagram's theme: composited onto white,
    /// a dark theme's light strokes vanish into the ground (Codex's P1).
    #[test]
    fn a_dark_theme_render_sits_on_the_dark_ground_not_white() {
        let pixmap = raster(DARK_THEME_SEKIEN, "any", 1.0).unwrap();
        assert_eq!(
            (0x33, 0x33, 0x33, 255),
            pixel(&pixmap, 0, 0),
            "the uncovered corner must be the dark ground"
        );
        let side = pixmap.width() * pixmap.height();
        let light = (0..side)
            .map(|i| {
                pixmap
                    .pixel(i % pixmap.width(), i / pixmap.width())
                    .unwrap()
            })
            .filter(|p| p.red() > 150 && p.green() > 150 && p.blue() > 150)
            .count();
        assert!(light > 0, "the #ccc strokes must survive as light ink");
    }

    /// The marker is load-bearing, so its recognition must survive the
    /// serializations an unpinned sekien could drift to (Codex's narrowed
    /// P1): whitespace around the colon, case, short and long hex, and a
    /// preceding `fill-opacity` that must not be mistaken for `fill`.
    #[test]
    fn every_root_fill_serialization_variant_finds_the_dark_ground() {
        let variants = [
            "fill:#ccc",
            "fill: #ccc",
            "fill : #CCC",
            "fill:#cccccc",
            "fill-opacity:1;fill:#ccc",
        ];
        for variant in variants {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><style>#d1{{{variant};}}</style></svg>"##
            );
            let pixmap = raster(&svg, "any", 1.0).unwrap();
            assert_eq!(
                (0x33, 0x33, 0x33, 255),
                pixel(&pixmap, 0, 0),
                "variant {variant:?} must be recognized as a dark theme"
            );
        }
    }

    /// Ground choice follows WCAG contrast, not channel mean: bright green
    /// has channel mean 85 ("dark") yet reads at only 1.37:1 on white and
    /// 9.2:1 on the dark ground; mid-gray is the case where skipping the
    /// gamma linearization flips the answer.
    #[test]
    fn ground_choice_follows_contrast_not_channel_mean() {
        let cases = [
            (
                "#00ff00",
                (0x33, 0x33, 0x33, 255),
                "bright saturated green needs the dark ground",
            ),
            (
                "#00008b",
                (255, 255, 255, 255),
                "dark blue needs the white ground",
            ),
            (
                "#808080",
                (255, 255, 255, 255),
                "mid-gray contrasts better with white after linearization",
            ),
        ];
        for (fill, expected, why) in cases {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><style>#d1{{fill:{fill};}}</style></svg>"##
            );
            let pixmap = raster(&svg, "any", 1.0).unwrap();
            assert_eq!(expected, pixel(&pixmap, 0, 0), "{why}");
        }
    }

    /// White is only for an explicitly recognized light theme: a marker the
    /// derivation cannot read errors instead of silently choosing the
    /// ground that erases light ink.
    #[test]
    fn an_unreadable_theme_marker_is_a_loud_error_never_a_white_ground() {
        let cases = [
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"##.to_string(),
                "no style block",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><style>#d1{stroke:#000;}</style></svg>"##.to_string(),
                "no fill declaration",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><style>#d1{fill:rgb(204, 204, 204);}</style></svg>"##.to_string(),
                "unrecognized color spelling",
            ),
        ];
        for (svg, case) in cases {
            let message = rasterize(&svg, "any", 1.0).unwrap_err().to_string();
            assert!(
                message.contains("ground") || message.contains("theme"),
                "{case}: the error must be actionable, got: {message}"
            );
        }
    }

    #[test]
    fn light_ink_under_a_dark_theme_marker_stays_distinguishable() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><style>#d1{fill:#ccc;}</style><path d="M 0 2 H 4" stroke="#ffffff" stroke-width="2"/></svg>"##;
        let pixmap = raster(svg, "any", 1.0).unwrap();
        assert_ne!(
            pixel(&pixmap, 1, 1),
            pixel(&pixmap, 7, 1),
            "a white stroke must not blend into the ground of a dark-themed diagram"
        );
    }

    /// The area cap alone admits this: 100000x1 is only 100k pixels, but a
    /// side that long breaks mobile decoders and GPU texture limits
    /// (Codex's P2).
    #[test]
    fn a_long_thin_diagram_is_refused_per_side_even_under_the_area_cap() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="1"><rect width="100000" height="1" fill="#ff0000"/></svg>"##;
        let message = rasterize(svg, "any", 1.0).unwrap_err().to_string();
        assert!(message.contains("100000x1"), "{message}");
        assert!(message.contains("per-side"), "{message}");
    }

    #[test]
    fn equal_luminances_have_a_contrast_ratio_of_one() {
        for luminance in [0.0, 0.5, 1.0] {
            assert_eq!(
                1.0,
                contrast(luminance, luminance),
                "identical luminances must not differ in contrast at {luminance}"
            );
        }
    }

    #[test]
    fn the_exact_pixel_area_cap_is_legal_and_one_row_over_is_refused() {
        let side = 4096u32;
        assert_eq!(
            PIXEL_CAP,
            u64::from(side) * u64::from(side),
            "the law assumes a square area cap"
        );
        check_raster_size(side, side).expect("the area cap itself is a legal size");
        let message = check_raster_size(side, side + 1).unwrap_err().to_string();
        assert!(message.contains("4096x4097"), "{message}");
        assert!(message.contains(&PIXEL_CAP.to_string()), "{message}");
    }

    #[test]
    fn a_diagram_exactly_at_the_per_side_cap_is_rasterized_not_refused() {
        for (width, height) in [(SIDE_CAP, 1), (1, SIDE_CAP)] {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"><style>#d1{{fill:#333;}}</style><rect width="{width}" height="{height}" fill="#ff0000"/></svg>"##
            );
            let pixmap = raster(&svg, "any", 1.0).unwrap();
            assert_eq!(width, pixmap.width(), "{width}x{height} is a legal size");
            assert_eq!(height, pixmap.height(), "{width}x{height} is a legal size");
        }
    }

    /// Frozen rasters sit on an opaque ground: a transparent PNG would
    /// show the page theme through the diagram and make dark-theme review
    /// unreadable with the default mermaid colors.
    #[test]
    fn uncovered_canvas_is_opaque_white_not_transparent() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8"><style>#d1{fill:#333;}</style><rect width="2" height="2" fill="#ff0000"/></svg>"##;
        let pixmap = raster(svg, "any", 1.0).unwrap();
        assert_eq!((255, 255, 255, 255), pixel(&pixmap, 9, 7));
        assert_eq!(
            (255, 0, 0, 255),
            pixel(&pixmap, 0, 0),
            "the rect itself still draws"
        );
    }

    /// The trap this guard exists for: usvg drops ALL text when the family
    /// is missing, so without the error a host without the chosen font would
    /// freeze a silently textless diagram.
    #[test]
    fn a_text_bearing_svg_with_a_missing_family_fails_loudly_not_textless() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8"><text x="1" y="6">hi</text></svg>"#;
        let message = rasterize(svg, "no-such-family-xyz", 1.0)
            .unwrap_err()
            .to_string();
        assert!(message.contains("no-such-family-xyz"), "{message}");
        assert!(message.contains("not installed"), "{message}");
    }

    #[test]
    fn a_malformed_svg_is_an_error_not_a_panic() {
        assert!(rasterize("<svg", "any", 1.0).is_err());
        assert!(rasterize("plain text", "any", 1.0).is_err());
    }

    /// The cap must fire BEFORE allocation: without it this test would try
    /// to allocate a multi-gigabyte pixmap instead of returning an error.
    #[test]
    fn an_oversized_render_is_refused_before_allocation() {
        let message = rasterize(RECT_SVG, "any", 10_000.0)
            .unwrap_err()
            .to_string();
        assert!(message.contains("100000x80000"), "{message}");
        assert!(message.contains("cap"), "{message}");
    }

    #[test]
    fn a_zero_sized_render_is_an_error_naming_the_zoom() {
        let message = rasterize(RECT_SVG, "any", 0.0).unwrap_err().to_string();
        assert!(message.contains("zoom 0"), "{message}");
    }

    const LABELED_SEKIEN: &str = include_str!("../tests/fixtures/labeled-sekien.svg");
    const SEQUENCE_SEKIEN: &str = include_str!("../tests/fixtures/sequence-sekien.svg");

    const SHAPES_SEKIEN: &str = include_str!("../tests/fixtures/shapes-sekien.svg");

    /// Real sekien sequence output keeps its scale()d actor icons under
    /// <defs>. Definitions do not render in place, so ignoring them cannot
    /// discard a rendered label, while rendered geometry stays fail-closed.
    #[test]
    fn definition_only_transforms_do_not_fail_the_fence() {
        let found = geometry(SEQUENCE_SEKIEN)
            .expect("a sequence diagram must stay freezable despite scale() decorations");
        assert!(
            found.labels.is_empty(),
            "sequence output carries no flowchart-shaped labels: {:?}",
            found.labels
        );
        let frozen = freeze_fence(
            SEQUENCE_SEKIEN,
            available_family().expect("a system font"),
            &std::collections::HashMap::new(),
        )
        .expect("the full freeze succeeds with an empty label map");
        assert!(frozen.geometry.labels.is_empty());
        assert!(frozen.geometry.image_width > 0);
    }

    /// Codex's P1: shaped nodes are ordinary flowchart syntax, and a map
    /// that silently drops them is incomplete where completeness is the
    /// whole point. Real sekien output, every node-label spelling.
    #[test]
    fn every_node_shape_spelling_is_in_the_complete_label_map() {
        let found = geometry(SHAPES_SEKIEN).unwrap();
        let expected = [
            ("A", "Rect"),
            ("B", "Round"),
            ("C", "Decision"),
            ("D", "Circle"),
            ("E", "Database"),
            ("F", "Stadium"),
        ];
        for (id, text) in expected {
            let label = found
                .labels
                .iter()
                .find(|label| label.id == id)
                .unwrap_or_else(|| panic!("node {id} ({text}) missing from the map"));
            assert_eq!(text, label.text, "{id}");
            assert!(
                label.width > 10.0 && label.height > 10.0,
                "{id} box degenerate: {label:?}"
            );
        }
        let circle = found.labels.iter().find(|label| label.id == "D").unwrap();
        assert!(
            (circle.width - circle.height).abs() < 0.01,
            "a circle's box is square: {circle:?}"
        );
    }

    #[test]
    fn a_path_container_node_is_measured_with_its_own_transform() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 80">
          <g class="nodes">
            <g class="node default" id="d1-flowchart-DB-5" transform="translate(50, 40)">
              <path d="M0,10 a38,10 0,0,0 76,0 a38,10 0,0,0 -76,0 l0,40 a38,10 0,0,0 76,0 l0,-40"
                    class="basic label-container outer-path" transform="translate(-38, -30)"/>
              <g class="label" transform="translate(0, -2)">
                <text><tspan>Postgres</tspan></text>
              </g>
            </g>
          </g>
        </svg>"##;
        let found = geometry(svg).unwrap();
        let label = found
            .labels
            .iter()
            .find(|label| label.id == "DB")
            .expect("the shaped node is in the map");
        assert_eq!("Postgres", label.text);
        assert!(
            (label.x - 12.0).abs() < 0.01 && label.width >= 76.0,
            "50 - 38 = 12 with the path's translate applied: {label:?}"
        );
    }

    /// Fail closed on shapes this extraction has never seen: a silent skip
    /// would repeat the exact defect the shape generalization fixed.
    #[test]
    fn an_unrecognized_node_container_fails_closed_naming_the_node() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><g class="node" id="d1-flowchart-X-0"><image class="label-container" href="x"/></g></svg>"##;
        let message = geometry(svg).unwrap_err().to_string();
        assert!(message.contains("'X'"), "{message}");
        let none = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><g class="node" id="d1-flowchart-X-0"><text>hi</text></g></svg>"##;
        let message = geometry(none).unwrap_err().to_string();
        assert!(message.contains("no label-container"), "{message}");
    }

    /// The equivalence law, both directions: what the parser rejects, the
    /// scanner must too. A nonbreaking space makes the line card CONTENT to
    /// the parser's closed whitespace set, and a truncated comment is a
    /// lint, not a directive; a scanner accepting either would count a
    /// malformed near-stamp as current and silently suppress freezing.
    #[test]
    fn the_scanner_rejects_every_near_stamp_the_parser_rejects() {
        let source = "flowchart LR\n A-->B";
        let print = fingerprint(source);
        let object = |ext: &str| format!("sha256-{}.{ext}", "a".repeat(64));
        let cases = [
            (
                format!(
                    "## q\n```mermaid\n{source}\n```\n{nbsp}<!-- diagram: fingerprint: {print} asset: {} geometry: {} -->\nanswer\n",
                    object("png"),
                    object("json"),
                    nbsp = '\u{00a0}',
                ),
                "a nonbreaking space is content, not indentation",
            ),
            (
                format!(
                    "## q\n```mermaid\n{source}\n```\n<!-- diagram: fingerprint: {print}\nanswer\n"
                ),
                "a truncated comment is a lint, not a stamp",
            ),
        ];
        for (text, why) in cases {
            let parsed = crate::parser::parse("deck.md", &text).unwrap();
            assert!(
                parsed.cards[0].diagrams.is_empty(),
                "{why}: parser baseline"
            );
            let found = fences_in_document(&text, None);
            assert!(
                found.fences[0].stamp.is_none(),
                "{why}: the scanner must agree with the parser"
            );
        }
    }

    /// Codex's P3: the parser trims before matching, so the scanner must
    /// agree or an indented valid stamp re-freezes and stacks.
    #[test]
    fn an_indented_valid_stamp_is_attached_to_its_fence() {
        let source = "flowchart LR\n A-->B";
        let print = fingerprint(source);
        let text = format!(
            "## q\n```mermaid\n{source}\n```\n  <!-- diagram: fingerprint: {print} asset: sha256-{0}.png geometry: sha256-{0}.json -->\nanswer\n",
            "a".repeat(64)
        );
        let found = fences_in_document(&text, None);
        assert_eq!(
            Some(print.as_str()),
            found.fences[0]
                .stamp
                .as_ref()
                .map(|(_, value)| value.as_str())
        );
    }

    #[test]
    fn document_fences_carry_insert_offsets_and_existing_stamps() {
        let text = format!(
            "---\nid: x\n---\n## q\n```mermaid\nflowchart LR\n A-->B\n```\nanswer\n```mermaid\nsecond\n```\n<!-- diagram: fingerprint: xxh64-00000000000000ff asset: sha256-{0}.png geometry: sha256-{0}.json -->\nmore\n",
            "a".repeat(64)
        );
        let text = text.as_str();
        let found = fences_in_document(text, Some((1, 3)));
        assert!(!found.unclosed);
        assert_eq!(2, found.fences.len());
        let first = &found.fences[0];
        assert_eq!("flowchart LR\n A-->B", first.source);
        assert_eq!(None, first.stamp);
        assert!(
            text[first.insert_at..].starts_with("answer"),
            "a new stamp lands at the start of the line after the close"
        );
        let second = &found.fences[1];
        let (range, fingerprint) = second.stamp.as_ref().unwrap();
        assert_eq!("xxh64-00000000000000ff", fingerprint);
        assert!(
            text[range.clone()].starts_with("<!-- diagram:")
                && text[range.clone()].ends_with("-->"),
            "the stamp range is the whole line without its newline"
        );
    }

    #[test]
    fn frontmatter_lines_never_open_a_document_fence() {
        let text = "---\nnote: |\n  ```mermaid\n  x\n---\n## q\na\n";
        let found = fences_in_document(text, Some((1, 5)));
        assert_eq!(0, found.fences.len());
        assert!(!found.unclosed);
    }

    #[test]
    fn an_unclosed_mermaid_fence_is_flagged_and_yields_nothing_to_stamp() {
        let found = fences_in_document("## q\n```mermaid\nflowchart LR\n", None);
        assert!(found.unclosed);
        assert_eq!(0, found.fences.len());
    }

    #[test]
    fn every_bindable_label_is_extracted_with_identity_text_and_box() {
        let found = geometry(LABELED_SEKIEN).unwrap();
        assert_eq!(
            ViewBox {
                x: 0.0,
                y: 0.0,
                width: 375.65625,
                height: 237.0
            },
            found.view_box
        );
        let ids: Vec<(&str, &str)> = found
            .labels
            .iter()
            .map(|label| (label.id.as_str(), label.text.as_str()))
            .collect();
        assert_eq!(
            vec![
                ("L_A_B_0", "fills"),
                ("L_C_B_0", "reads"),
                ("A", "Cache"),
                ("B", "Store"),
                ("C", "Cache"),
            ],
            ids,
            "duplicate 'Cache' labels must stay distinct by semantic identity"
        );
        let node_a = found.labels.iter().find(|label| label.id == "A").unwrap();
        assert!(
            (node_a.x - 43.0).abs() < 0.01
                && (node_a.y - 33.0).abs() < 0.01
                && (node_a.width - 110.0).abs() < 0.01
                && (node_a.height - 49.0).abs() < 0.01,
            "node A box moved: {node_a:?}"
        );
        let edge = found
            .labels
            .iter()
            .find(|label| label.id == "L_C_B_0")
            .unwrap();
        assert!(
            (edge.x - 164.274_96).abs() < 0.01 && (edge.y - 94.71779).abs() < 0.01,
            "edge label box moved: {edge:?}"
        );
    }

    #[test]
    fn a_non_translate_transform_above_a_label_fails_closed() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><g transform="scale(2)"><g class="node" id="d1-flowchart-A-0"><rect x="0" y="0" width="4" height="4"/></g></g></svg>"##;
        let message = geometry(svg).unwrap_err().to_string();
        assert!(message.contains("unsupported transform"), "{message}");
        assert!(message.contains("scale(2)"), "{message}");
    }

    #[test]
    fn unsupported_transform_is_ignored_only_when_its_subtree_has_no_label() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><defs><path transform="scale(.5)" d="M0 0h1v1z"/></defs><g transform="scale(2)"><g class="node" id="d1-flowchart-A-0"><rect class="label-container" x="0" y="0" width="4" height="4"/><g class="label"><text>Hi</text></g></g></g></svg>"##;
        let message = geometry(svg)
            .expect_err("a transform above a recognized label must still fail closed")
            .to_string();
        assert!(message.contains("scale(2)"), "{message}");
    }

    #[test]
    fn a_render_without_flowchart_labels_yields_an_empty_map() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><g class="actor"><rect x="1" y="1" width="4" height="4"/></g></svg>"##;
        assert!(geometry(svg).unwrap().labels.is_empty());
    }

    /// The dark fixture's viewBox starts at -8,-8: a box at SVG 0,0 sits 8
    /// units into the raster, and forgetting the origin shifts every mask.
    #[test]
    fn a_negative_viewbox_origin_shifts_pixel_boxes() {
        let view_box = ViewBox {
            x: -8.0,
            y: -8.0,
            width: 100.0,
            height: 50.0,
        };
        let label = Label {
            id: "A".into(),
            text: "hi".into(),
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 5.0,
        };
        let bounds = pixel_box(&label, view_box, 2.0, (200, 100));
        assert_eq!(
            PixelBox {
                x: 12,
                y: 12,
                width: 28,
                height: 18
            },
            bounds,
            "(0 - -8 - pad) * zoom = 12; (10 + 2*pad + 8 - -8)... width = (8+10+2)*2 - 12 = 28"
        );
    }

    #[test]
    fn pixel_boxes_are_padded_outward_and_clamped_to_the_raster() {
        let view_box = ViewBox {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 10.0,
        };
        let label = Label {
            id: "A".into(),
            text: "hi".into(),
            x: 0.5,
            y: 0.5,
            width: 19.5,
            height: 9.5,
        };
        let bounds = pixel_box(&label, view_box, 1.0, (20, 10));
        assert_eq!(
            PixelBox {
                x: 0,
                y: 0,
                width: 20,
                height: 10
            },
            bounds,
            "the pad pushes past both edges and clamps to the raster"
        );
    }

    #[test]
    fn stripping_removes_exactly_the_targeted_labels_text() {
        let full_count = LABELED_SEKIEN.matches("Cache").count();
        let stripped = strip_label_texts(LABELED_SEKIEN, "A").unwrap();
        assert_eq!(
            full_count - 1,
            stripped.matches("Cache").count(),
            "only node A's text goes; node C keeps its own 'Cache'"
        );
        assert!(stripped.contains("Store"), "other labels stay");
        usvg::roxmltree::Document::parse(&stripped).expect("stripping must keep the XML valid");
        let edge = strip_label_texts(LABELED_SEKIEN, "L_A_B_0").unwrap();
        assert!(!edge.contains("fills"), "edge label text goes");
        assert!(edge.contains("reads"), "the other edge label stays");
        assert!(
            strip_label_texts(LABELED_SEKIEN, "nope")
                .unwrap_err()
                .to_string()
                .contains("not in the SVG")
        );
    }

    fn plain_pixmap(width: u32, height: u32) -> resvg::tiny_skia::Pixmap {
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).unwrap();
        pixmap.fill(resvg::tiny_skia::Color::WHITE);
        pixmap
    }

    #[test]
    fn ink_containment_rejects_stray_and_absent_ink() {
        let full = {
            let mut pixmap = plain_pixmap(4, 4);
            pixmap.pixels_mut()[5] =
                resvg::tiny_skia::PremultipliedColorU8::from_rgba(0, 0, 0, 255).unwrap();
            pixmap
        };
        let stripped = plain_pixmap(4, 4);
        let covering = PixelBox {
            x: 1,
            y: 1,
            width: 1,
            height: 1,
        };
        ink_within(&full, &stripped, covering).expect("ink at (1,1) inside its box");
        let elsewhere = PixelBox {
            x: 2,
            y: 2,
            width: 2,
            height: 2,
        };
        let message = ink_within(&full, &stripped, elsewhere)
            .unwrap_err()
            .to_string();
        assert!(message.contains("outside its box"), "{message}");
        let none = ink_within(&stripped, &stripped, covering)
            .unwrap_err()
            .to_string();
        assert!(none.contains("no ink"), "{message}");
    }

    #[test]
    fn the_geometry_round_trips_and_rejects_unknown_fields() {
        let geometry = DiagramGeometry {
            image: "sha256-ab.png".into(),
            image_width: 200,
            image_height: 100,
            logical_width: 100,
            logical_height: 50,
            labels: vec![GeometryLabel {
                id: "A".into(),
                text: "Cache".into(),
                source: LabelSource::Range { start: 20, end: 25 },
                bounds: PixelBox {
                    x: 1,
                    y: 2,
                    width: 3,
                    height: 4,
                },
            }],
        };
        let json = serde_json::to_string(&geometry).unwrap();
        assert!(
            json.contains(r#""x":1"#),
            "bounds must flatten into the label entry: {json}"
        );
        let back: DiagramGeometry = serde_json::from_str(&json).unwrap();
        assert_eq!(geometry.labels[0].bounds, back.labels[0].bounds);
        let unknown = json.replace(r#""image""#, r#""theme":"dark","image""#);
        assert!(
            serde_json::from_str::<DiagramGeometry>(&unknown).is_err(),
            "an unknown field must fail loud, never be silently dropped"
        );
    }

    #[test]
    fn the_label_source_wire_shape_is_the_ruled_tagged_object() {
        let range = serde_json::to_value(LabelSource::Range { start: 20, end: 25 }).unwrap();
        assert_eq!(
            serde_json::json!({"kind": "range", "start": 20, "end": 25}),
            range
        );
        let unbindable = serde_json::to_value(LabelSource::Unbindable).unwrap();
        assert_eq!(serde_json::json!({"kind": "unbindable"}), unbindable);
        let geometry = serde_json::json!({
            "png": "sha256-ab.png",
            "image_width": 200,
            "image_height": 100,
            "logical_width": 100,
            "logical_height": 50,
            "labels": [{"id": "A", "text": "Cache", "x": 1, "y": 2, "width": 3, "height": 4}],
        });
        assert!(
            serde_json::from_value::<DiagramGeometry>(geometry).is_err(),
            "a label without `source` (a geometry frozen before the field) \
             must fail loud as ordinary invalid input"
        );
    }

    fn geometry_with(labels: Vec<GeometryLabel>) -> DiagramGeometry {
        DiagramGeometry {
            image: "sha256-ab.png".into(),
            image_width: 200,
            image_height: 100,
            logical_width: 100,
            logical_height: 50,
            labels,
        }
    }

    fn ranged(id: &str, text: &str, start: u32, end: u32) -> GeometryLabel {
        GeometryLabel {
            id: id.into(),
            text: text.into(),
            source: LabelSource::Range { start, end },
            bounds: PixelBox {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            },
        }
    }

    #[test]
    fn every_label_range_is_validated_not_only_span_equal_ones() {
        let interior = "flowchart LR\n  A[Löwe] --> B[ok]";
        let a = interior.find("Löwe").unwrap() as u32;
        let cases: [(GeometryLabel, &str); 4] = [
            (
                ranged("B", "ok", 0, interior.len() as u32 + 4),
                "out of bounds",
            ),
            (ranged("B", "ok", 9, 9), "empty range"),
            (ranged("B", "ok", 12, 9), "reversed range"),
            (
                // one byte into the two-byte ö
                ranged("B", "ok", a + 2, a + 4),
                "splits a UTF-8 scalar",
            ),
        ];
        for (bad, why) in cases {
            let geometry = geometry_with(vec![ranged("A", "Löwe", a, a + 5), bad]);
            assert_eq!(
                Err(BindFailure::InvalidLabelRange { label: "B".into() }),
                validate_label_sources(&geometry, interior),
                "an UNRELATED label's bad range must fail the whole geometry ({why})"
            );
        }
        let geometry = geometry_with(vec![
            ranged("A", "Löwe", a, a + 5),
            ranged("B", "we]", a + 3, a + 8),
        ]);
        assert!(
            matches!(
                validate_label_sources(&geometry, interior),
                Err(BindFailure::OverlappingLabelRanges { .. })
            ),
            "overlapping label ranges fail the whole geometry"
        );
        let geometry = geometry_with(vec![ranged("A", "Löwe", a, a + 5)]);
        assert_eq!(Ok(()), validate_label_sources(&geometry, interior));
    }

    #[test]
    fn a_span_binds_only_the_exact_complete_label_range() {
        let interior = "flowchart LR\n  Cache[store] --> B[Cache]";
        let store = interior.find("store").unwrap();
        let second_cache = interior.rfind("Cache").unwrap();
        let geometry = geometry_with(vec![
            ranged("Cache", "store", store as u32, store as u32 + 5),
            ranged("B", "Cache", second_cache as u32, second_cache as u32 + 5),
        ]);
        assert_eq!(
            Ok(1),
            bind_span(&geometry, 6, second_cache, second_cache + 5),
            "the exact range binds its label"
        );
        assert_eq!(
            Err(BindFailure::SpanIncompleteLabel {
                line: 6,
                label: "Cache".into()
            }),
            bind_span(&geometry, 6, store + 1, store + 4),
            "a proper part of a label is incomplete"
        );
        assert_eq!(
            Err(BindFailure::SpanOutsideLabels { line: 6 }),
            bind_span(&geometry, 6, 15, 20),
            "the id occurrence of a label's text is not the label"
        );
    }

    #[test]
    fn only_the_bare_rename_signature_licenses_a_bracket_retry() {
        let label = |id: &str, text: &str| Label {
            id: id.into(),
            text: text.into(),
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let original = [label("Idle", "Idle"), label("Done", "Finished")];
        assert!(
            bare_rename(
                "Idle",
                &original,
                "xq1",
                &[label("xq1", "xq1"), label("Done", "Finished")]
            ),
            "a renamed bare node carries the marker as id AND text, and the \
             occurrence names a bare label in the original"
        );
        assert!(
            !bare_rename(
                "Cache",
                &[label("Cache", "store"), label("B", "Cache")],
                "xq1",
                &[label("xq1", "store"), label("B", "Cache")]
            ),
            "a labeled node's id occurrence keeps its own text: no retry, or \
             the retry would bind id bytes the picture never shows"
        );
        assert!(
            !bare_rename("Idle", &original, "xq1", &[label("Idle", "xq1")]),
            "a relabeled node with an unchanged id is the plain probe's own \
             assignment, never a retry"
        );
        assert!(
            !bare_rename(
                "A",
                &[label("A", "\u{3b2}"), label("X", "A")],
                "xq1",
                &[label("A", "\u{3b2}"), label("X", "A"), label("xq1", "xq1")]
            ),
            "Codex's P1: a bare ID REFERENCE to a labeled node fabricates the \
             variant signature, but the occurrence names no bare label in the \
             ORIGINAL, so no retry; a retry would bind the nonliteral \u{3b2} \
             label to id bytes"
        );
    }

    #[test]
    fn the_bracket_probe_relabels_the_occurrence_in_place() {
        let interior = "flowchart LR\n  Idle --> Done[Finished]";
        let start = interior.find("Idle").unwrap();
        let probe = SourceProbe {
            start,
            end: start + "Idle".len(),
            source: String::new(),
        };
        let bracketed = bracket_probe(interior, &probe, "xq1");
        assert_eq!(
            "flowchart LR\n  Idle[xq1] --> Done[Finished]",
            bracketed.source
        );
        assert_eq!(
            (probe.start, probe.end),
            (bracketed.start, bracketed.end),
            "the retry binds the ORIGINAL occurrence bytes, not the variant's"
        );
    }

    #[test]
    fn probe_plan_is_linear_over_repeated_label_texts() {
        let label = |id: &str, text: &str| Label {
            id: id.into(),
            text: text.into(),
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let interior = "flowchart LR\n  A[yes] --> B[yes]\n  C[yes] --> D[store]";
        let labels = [
            label("A", "yes"),
            label("B", "yes"),
            label("C", "yes"),
            label("D", "store"),
        ];
        let (marker, probes, truncated) = probe_plan(interior, &labels);
        assert!(!truncated);
        assert!(!interior.contains(&marker), "the marker is absent");
        assert_eq!(
            4,
            probes.len(),
            "one probe per unique occurrence per DISTINCT text (3 for the \
             shared `yes`, 1 for `store`), never one pass per label: {probes:?}"
        );
        for probe in &probes {
            assert_eq!(
                format!(
                    "{}{}{}",
                    &interior[..probe.start],
                    marker,
                    &interior[probe.end..]
                ),
                probe.source,
                "a probe substitutes exactly its own occurrence"
            );
        }
        let starts: Vec<usize> = probes.iter().map(|p| p.start).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, starts, "probes ride in source order");
    }

    #[test]
    fn probe_plan_truncates_at_the_budget_and_flags_it() {
        let label = |id: &str, text: &str| Label {
            id: id.into(),
            text: text.into(),
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let interior = "y ".repeat(PROBE_BUDGET + 10);
        let labels = [label("A", "y")];
        let (_, probes, truncated) = probe_plan(&interior, &labels);
        assert!(truncated, "over-budget planning must say so");
        assert_eq!(PROBE_BUDGET, probes.len());
    }

    #[test]
    fn a_probe_assigns_only_an_exactly_one_unchanged_id_marker_hit() {
        let label = |id: &str, text: &str| Label {
            id: id.into(),
            text: text.into(),
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let original = [label("A", "yes"), label("B", "yes")];
        assert_eq!(
            Some("B".to_string()),
            probe_assignment(&original, "xq1", &[label("A", "yes"), label("B", "xq1")]),
            "exactly one unchanged id carries the marker"
        );
        assert_eq!(
            None,
            probe_assignment(&original, "xq1", &[label("A", "yes"), label("B", "yes")]),
            "zero hits assigns nothing (an id position, a comment)"
        );
        assert_eq!(
            None,
            probe_assignment(&original, "xq1", &[label("A", "xq1"), label("B", "xq1")]),
            "multiple hits are ambiguous and assign nothing"
        );
        assert_eq!(
            None,
            probe_assignment(&original, "xq1", &[label("XSENTX", "xq1")]),
            "a renamed node is not an unchanged id"
        );
    }

    #[test]
    fn collate_drops_multiply_assigned_labels() {
        let assigned = [
            ("A".to_string(), 5, 8),
            ("B".to_string(), 12, 15),
            ("B".to_string(), 20, 23),
        ];
        let sources = collate_assignments(&assigned);
        assert_eq!(Some(&(5, 8)), sources.get("A"));
        assert_eq!(
            None,
            sources.get("B"),
            "a label confirmed twice is ambiguous, never two ranges to one box"
        );
    }

    #[test]
    fn a_hanging_renderer_is_killed_at_the_timeout() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "cat >/dev/null; sleep 60");
        let started = std::time::Instant::now();
        let out = render_batch(
            cli.to_str().unwrap(),
            None,
            &sources(1),
            Duration::from_millis(300),
        );
        assert!(out.is_err(), "a hang must not be reported as success");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the timeout did not kill the child"
        );
    }
}
