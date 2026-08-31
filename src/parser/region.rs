//! The region directive grammar frozen by ADR 0034: `blank:`, `cover:`,
//! `crop:`. This module owns every rule decidable from the directive line
//! alone; binding and cross-region rules live with the card builder.

use super::ParseError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    Blank,
    Cover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// The match ends at non-alphanumeric characters: whole words, with
    /// prose punctuation tolerated.
    Word,
    /// The match may sit anywhere: sub-word blanks.
    Char,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionGeometry {
    Rect {
        x: Num,
        y: Num,
        width: Num,
        height: Num,
    },
    Span {
        /// The Nth occurrence of the hidden text, 1-based; defaults to 1.
        occurrence: u32,
        boundary: Boundary,
    },
}

/// A validated numeric literal. The authored text is kept verbatim (the deck
/// file is the source of truth and is never rewritten); `value` is the parsed
/// magnitude for bounds math.
#[derive(Clone, Debug)]
pub struct Num {
    pub literal: String,
    pub value: f64,
    pub percent: bool,
}

impl PartialEq for Num {
    fn eq(&self, other: &Self) -> bool {
        self.literal == other.literal && self.percent == other.percent
    }
}
impl Eq for Num {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRegion {
    pub kind: RegionKind,
    pub geometry: RegionGeometry,
    pub group: Option<String>,
    pub hidden: Option<String>,
    pub stamp: Option<String>,
    /// The machine-minted anchor offset (`position:<n>`, 1-based grapheme
    /// index of the bound occurrence in the block's context text): doctor's
    /// drift signal, never read for binding.
    pub minted_position: Option<u32>,
    /// Computed at bind, never parsed: where the span actually bound, in the
    /// same grapheme coordinates `position:` uses. The stamper mints it when
    /// `position:` is absent and leaves any divergence for doctor.
    pub bound_position: Option<u32>,
    /// Computed at bind, never parsed: when a divergent `position:` still
    /// starts a bindable occurrence of the hidden text, its 1-based
    /// occurrence index, doctor's keep-the-old-target edit.
    pub minted_occurrence: Option<u32>,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawCrop {
    pub x: Num,
    pub y: Num,
    pub width: Num,
    pub height: Num,
    pub line: usize,
}

fn err(line: usize, message: impl Into<String>) -> ParseError {
    ParseError::InvalidRegion {
        line,
        message: message.into(),
    }
}

/// Splits a directive body into tokens, honoring one level of `"` quoting
/// with the frozen escape set. A quoted run stays inside its `key="..."`
/// token.
fn tokens(body: &str, line: usize) -> Result<Vec<String>, ParseError> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = body.chars();
    let mut quoted = false;
    // The last three raw source CHARACTERS seen inside the quoted state
    // (escape-consumed ones excluded), for the dangerous-terminator rule: a
    // raw `>` after `--` or `--!` would close the HTML comment in other
    // renderers and spill the rest as page text.
    let mut tail = ['\0'; 3];
    while let Some(c) = chars.next() {
        if quoted {
            let ends_dash_dash = tail[1] == '-' && tail[2] == '-';
            let ends_bang = tail == ['-', '-', '!'];
            if c == '>' && (ends_dash_dash || ends_bang) {
                return Err(err(
                    line,
                    "a raw `-->` or `--!>` inside a quoted value would close the comment; escape the `>` as `\\>`",
                ));
            }
            tail = [tail[1], tail[2], c];
            match c {
                '\\' => match chars.next() {
                    Some('"') => current.push('"'),
                    Some('\\') => current.push('\\'),
                    Some('>') => current.push('>'),
                    Some(other) => {
                        return Err(err(
                            line,
                            format!(
                                "unknown escape `\\{other}` in a quoted value (the reader accepts `\\\"`, `\\\\` and `\\>`)"
                            ),
                        ));
                    }
                    None => return Err(err(line, "a quoted value never closes")),
                },
                '"' => quoted = false,
                _ => current.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if quoted {
        return Err(err(line, "a quoted value never closes"));
    }
    if !current.is_empty() {
        out.push(current);
    }
    Ok(out)
}

/// The frozen numeric domain: digits with an optional decimal fraction, no
/// exponent, no sign, no separators; an optional `%` suffix.
fn number(raw: &str, key: &str, line: usize) -> Result<Num, ParseError> {
    let (digits, percent) = match raw.strip_suffix('%') {
        Some(rest) => (rest, true),
        None => (raw, false),
    };
    let valid = !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && digits.chars().filter(|c| *c == '.').count() <= 1
        && !digits.starts_with('.')
        && !digits.ends_with('.');
    if !valid {
        return Err(err(
            line,
            format!(
                "`{key}={raw}` is not a plain non-negative number (digits with an optional decimal fraction)"
            ),
        ));
    }
    let value: f64 = digits
        .parse()
        .map_err(|_| err(line, format!("`{key}={raw}` does not parse as a number")))?;
    if percent && value > 100.0 {
        return Err(err(line, format!("`{key}={raw}` exceeds 100%")));
    }
    Ok(Num {
        literal: digits.to_string(),
        value,
        percent,
    })
}

/// `word=`/`char=`: a 1-based positive canonical integer.
fn span_index(raw: &str, key: &str, line: usize) -> Result<u32, ParseError> {
    let canonical =
        !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()) && !raw.starts_with('0');
    if !canonical {
        return Err(err(
            line,
            format!("`{key}={raw}` is not a 1-based positive integer"),
        ));
    }
    raw.parse()
        .map_err(|_| err(line, format!("`{key}={raw}` overflows")))
}

pub(super) fn is_group_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn group_name(raw: &str, line: usize) -> Result<String, ParseError> {
    let name = &raw[1..raw.len() - 1];
    if !is_group_name(name) {
        return Err(err(
            line,
            format!(
                "group name `{raw}` is not one or more of `a-z`, `A-Z`, `0-9`, `_` or `-` in brackets"
            ),
        ));
    }
    Ok(name.to_string())
}

fn stamp(raw: &str, line: usize) -> Result<String, ParseError> {
    let legal = crate::token::is_valid_region_stamp(raw);
    if !legal {
        return Err(err(
            line,
            format!(
                "stamp `b:{raw}` is not six characters of the lowercase Crockford set (no i, l, o, u)"
            ),
        ));
    }
    Ok(raw.to_string())
}

struct Fields {
    shape: Option<String>,
    numbers: Vec<(String, Num)>,
    hidden: Option<String>,
    occurrence: Option<u32>,
    boundary: Option<Boundary>,
    group: Option<String>,
    stamp: Option<String>,
    minted_position: Option<u32>,
}

/// Tokenizes and classifies a directive body against the frozen grammar.
/// `directive` is the keyword for error text; which fields are LEGAL there is
/// the caller's rule.
fn fields(body: &str, directive: &str, line: usize) -> Result<Fields, ParseError> {
    let mut f = Fields {
        shape: None,
        numbers: Vec::new(),
        hidden: None,
        occurrence: None,
        boundary: None,
        group: None,
        stamp: None,
        minted_position: None,
    };
    let mut seen: Vec<String> = Vec::new();
    let mut once = |key: &str| -> Result<(), ParseError> {
        if seen.iter().any(|s| s == key) {
            return Err(err(line, format!("duplicate `{key}` on `{directive}:`")));
        }
        seen.push(key.to_string());
        Ok(())
    };
    for (index, token) in tokens(body, line)?.into_iter().enumerate() {
        if let Some(raw) = token.strip_prefix("b:") {
            once("b")?;
            f.stamp = Some(stamp(raw, line)?);
        } else if let Some(raw) = token.strip_prefix("position:") {
            once("position")?;
            f.minted_position = Some(span_index(raw, "position", line)?);
        } else if token.starts_with('[') && token.ends_with(']') && token.len() >= 2 {
            once("[group]")?;
            f.group = Some(group_name(&token, line)?);
        } else if let Some((key, value)) = token.split_once('=') {
            once(key)?;
            match key {
                "x" | "y" | "width" | "height" | "cx" | "cy" | "rx" | "ry" => {
                    f.numbers.push((key.to_string(), number(value, key, line)?));
                }
                "hidden" => f.hidden = Some(value.to_string()),
                "occurrence" => f.occurrence = Some(span_index(value, key, line)?),
                "boundary" => {
                    f.boundary = Some(match value {
                        "word" => Boundary::Word,
                        "char" => Boundary::Char,
                        other => {
                            return Err(err(
                                line,
                                format!("`boundary={other}` is neither `word` nor `char`"),
                            ));
                        }
                    });
                }
                "from" | "to" => {
                    return Err(err(
                        line,
                        format!(
                            "`{key}=` is a time key for video media, and no medium that carries regions has time in this version"
                        ),
                    ));
                }
                "points" => {
                    return Err(err(
                        line,
                        "`points=` belongs to `polygon`, a reserved shape",
                    ));
                }
                _ => return Err(err(line, format!("unknown key `{key}=` on `{directive}:`"))),
            }
        } else if index == 0 {
            f.shape = Some(token);
        } else {
            return Err(err(
                line,
                format!(
                    "`{token}` is not a `key=value` field, a `[group]`, or a leading shape word on `{directive}:`"
                ),
            ));
        }
    }
    Ok(f)
}

/// Pulls exactly the rect keys out and rejects a missing or extra one.
fn rect_geometry(
    numbers: Vec<(String, Num)>,
    directive: &str,
    line: usize,
) -> Result<RegionGeometry, ParseError> {
    let take = |name: &str| -> Result<Num, ParseError> {
        numbers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, num)| num.clone())
            .ok_or_else(|| err(line, format!("`{directive}: rect` is missing `{name}=`")))
    };
    let (x, y, width, height) = (take("x")?, take("y")?, take("width")?, take("height")?);
    if let Some((key, _)) = numbers
        .iter()
        .find(|(key, _)| !matches!(key.as_str(), "x" | "y" | "width" | "height"))
    {
        return Err(err(line, format!("`{key}=` does not apply to `rect`")));
    }
    for (name, num) in [("width", &width), ("height", &height)] {
        if num.value <= 0.0 {
            return Err(err(line, format!("`{name}=` must be strictly positive")));
        }
    }
    let mixed = [&x, &y, &width, &height]
        .windows(2)
        .any(|pair| pair[0].percent != pair[1].percent);
    if mixed {
        return Err(err(
            line,
            "a region carries one unit: all fields bare pixels, or all `%`",
        ));
    }
    Ok(RegionGeometry::Rect {
        x,
        y,
        width,
        height,
    })
}

fn parse_region(kind: RegionKind, body: &str, line: usize) -> Result<RawRegion, ParseError> {
    let directive = match kind {
        RegionKind::Blank => "blank",
        RegionKind::Cover => "cover",
    };
    let f = fields(body, directive, line)?;
    let shape = f
        .shape
        .as_deref()
        .ok_or_else(|| err(line, format!("`{directive}:` names no shape")))?;
    if kind == RegionKind::Cover {
        if f.group.is_some() {
            return Err(err(
                line,
                "a `cover:` takes no group: it never asks, so it has no card to join",
            ));
        }
        if f.stamp.is_some() {
            return Err(err(
                line,
                "a `cover:` carries no stamp: it creates no card and owns no history",
            ));
        }
    }
    let geometry = match shape {
        "rect" => {
            if f.occurrence.is_some() || f.boundary.is_some() {
                return Err(err(
                    line,
                    "`occurrence=`/`boundary=` locate a `span`, not a `rect`",
                ));
            }
            if f.minted_position.is_some() {
                return Err(err(
                    line,
                    "`position:` anchors a `span` and does not apply to `rect`",
                ));
            }
            rect_geometry(f.numbers, directive, line)?
        }
        "span" => {
            if !f.numbers.is_empty() {
                return Err(err(line, "a `span` carries no geometry keys"));
            }
            if f.hidden.is_none() {
                return Err(err(
                    line,
                    "a `span` requires `hidden=\"...\"`, its anchor and answer",
                ));
            }
            RegionGeometry::Span {
                occurrence: f.occurrence.unwrap_or(1),
                boundary: f.boundary.unwrap_or(Boundary::Word),
            }
        }
        "ellipse" | "polygon" | "clip" => {
            return Err(err(
                line,
                format!(
                    "`{shape}` is reserved and not yet accepted; v1 shapes are `rect` and `span`"
                ),
            ));
        }
        other => return Err(err(line, format!("unknown shape `{other}`"))),
    };
    Ok(RawRegion {
        kind,
        geometry,
        group: f.group,
        hidden: f.hidden,
        stamp: f.stamp,
        minted_position: f.minted_position,
        bound_position: None,
        minted_occurrence: None,
        line,
    })
}

pub(super) fn parse_blank(body: &str, line: usize) -> Result<RawRegion, ParseError> {
    parse_region(RegionKind::Blank, body, line)
}

pub(super) fn parse_cover(body: &str, line: usize) -> Result<RawRegion, ParseError> {
    parse_region(RegionKind::Cover, body, line)
}

pub(super) fn parse_crop(body: &str, line: usize) -> Result<RawCrop, ParseError> {
    let f = fields(body, "crop", line)?;
    match f.shape.as_deref() {
        Some("rect") => {}
        Some(other) => {
            return Err(err(
                line,
                format!("`crop:` takes `rect` only, got `{other}`"),
            ));
        }
        None => return Err(err(line, "`crop:` names no shape (`rect`)")),
    }
    if f.hidden.is_some()
        || f.group.is_some()
        || f.stamp.is_some()
        || f.occurrence.is_some()
        || f.boundary.is_some()
        || f.minted_position.is_some()
    {
        return Err(err(
            line,
            "`crop:` is a viewport: it takes `x y width height` and nothing else",
        ));
    }
    match rect_geometry(f.numbers, "crop", line)? {
        RegionGeometry::Rect {
            x,
            y,
            width,
            height,
        } => Ok(RawCrop {
            x,
            y,
            width,
            height,
            line,
        }),
        RegionGeometry::Span { .. } => unreachable!("crop parses rect keys only"),
    }
}

/// One media element's cross-region rules: unit agreement, then the
/// visible-viewport bounds. Empty covers are dropped, an empty blank is a
/// parse error, and pixel geometry without a crop is left to the client (the
/// only layer holding the source).
pub(crate) fn validate_media(
    regions: &mut Vec<RawRegion>,
    crop: Option<&RawCrop>,
) -> Result<(), ParseError> {
    let mut units: Vec<(bool, usize)> = regions
        .iter()
        .filter_map(|region| match &region.geometry {
            RegionGeometry::Rect { x, .. } => Some((x.percent, region.line)),
            RegionGeometry::Span { .. } => None,
        })
        .collect();
    if let Some(crop) = crop {
        units.push((crop.x.percent, crop.line));
    }
    if let Some(&(first, _)) = units.first()
        && let Some(&(_, line)) = units.iter().find(|(unit, _)| *unit != first)
    {
        return Err(err(
            line,
            "every geometric region and the `crop:` on one media element carry the same unit",
        ));
    }

    let viewport: Option<(f64, f64, f64, f64)> = match crop {
        // A percentage crop is itself bounded by the source: what the learner
        // sees is the crop INTERSECTED with [0,100]x[0,100], so a right-edge
        // crop must not lend visibility it does not have.
        Some(c) if c.x.percent => {
            let width = (c.x.value + c.width.value).min(100.0) - c.x.value;
            let height = (c.y.value + c.height.value).min(100.0) - c.y.value;
            Some((c.x.value, c.y.value, width.max(0.0), height.max(0.0)))
        }
        Some(c) => Some((c.x.value, c.y.value, c.width.value, c.height.value)),
        None if units.first().is_some_and(|(percent, _)| *percent) => {
            Some((0.0, 0.0, 100.0, 100.0))
        }
        None => None,
    };
    let Some((vx, vy, vw, vh)) = viewport else {
        return Ok(());
    };
    let mut failed: Option<usize> = None;
    regions.retain(|region| {
        let RegionGeometry::Rect {
            x,
            y,
            width,
            height,
        } = &region.geometry
        else {
            return true;
        };
        let visible = (x.value + width.value).min(vx + vw) > x.value.max(vx)
            && (y.value + height.value).min(vy + vh) > y.value.max(vy);
        if visible {
            return true;
        }
        match region.kind {
            RegionKind::Cover => false,
            RegionKind::Blank => {
                failed.get_or_insert(region.line);
                true
            }
        }
    });
    match failed {
        Some(line) => Err(err(
            line,
            "this `blank:` has no positive visible area in the viewport (the crop where one exists), so the learner cannot see what is asked",
        )),
        None => Ok(()),
    }
}

/// Byte ranges of the hidden text's occurrences in `text`, in order.
/// `whole_word` bounds each end at a non-alphanumeric character (so prose
/// punctuation does not break a match); substring mode counts every match.
// Production always supplies the stream's piece-aware oracle; the plain
// char-boundary form survives as the test harness for the advance law.
#[cfg(test)]
pub(crate) fn occurrences(text: &str, hidden: &str, whole_word: bool) -> Vec<(usize, usize)> {
    occurrences_with(text, hidden, &mut |start, end| {
        !whole_word
            || (!text[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
                && !text[end..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric))
    })
}

/// The occurrence scan with a caller-supplied word-boundary oracle, so a
/// stream consumer can treat piece edges (a hole gap, a link, a style node)
/// as the boundaries the learner sees; the boundary check stays inside the
/// advance loop so a rejected candidate never consumes an overlapping match.
pub(crate) fn occurrences_with(
    text: &str,
    hidden: &str,
    bounded: &mut dyn FnMut(usize, usize) -> bool,
) -> Vec<(usize, usize)> {
    if hidden.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(hidden) {
        let start = from + at;
        let end = start + hidden.len();
        if bounded(start, end) {
            found.push((start, end));
            from = end;
        } else {
            // A rejected candidate is not an occurrence, so it must not
            // consume a later overlapping bounded match.
            from = start + text[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_word_candidate_does_not_consume_an_overlapping_bounded_match() {
        assert_eq!(
            vec![(13, 22)],
            occurrences("The password word word is duplicated.", "word word", true)
        );
    }

    fn blank(body: &str) -> Result<RawRegion, ParseError> {
        parse_blank(body, 7)
    }

    fn reject(result: Result<impl std::fmt::Debug, ParseError>, needle: &str, case: &str) {
        match result {
            Err(ParseError::InvalidRegion { message, .. }) => assert!(
                message.contains(needle),
                "case `{case}`: error `{message}` does not mention `{needle}`"
            ),
            other => panic!("case `{case}` must be rejected, got {other:?}"),
        }
    }

    #[test]
    fn a_full_rect_blank_parses_with_group_hidden_and_stamp() {
        let region =
            blank(r#"rect [carpals] x=240 y=160 width=600 height=400 hidden="lunate" b:a1b2c3"#)
                .unwrap();
        assert_eq!(region.kind, RegionKind::Blank);
        assert_eq!(region.group.as_deref(), Some("carpals"));
        assert_eq!(region.hidden.as_deref(), Some("lunate"));
        assert_eq!(region.stamp.as_deref(), Some("a1b2c3"));
        let RegionGeometry::Rect { x, height, .. } = &region.geometry else {
            panic!("rect expected");
        };
        assert_eq!(x.literal, "240");
        assert!(!x.percent);
        assert_eq!(height.value, 400.0);
    }

    #[test]
    fn every_reserved_shape_word_is_rejected_and_unknown_shapes_named() {
        for shape in ["ellipse", "polygon", "clip"] {
            reject(blank(&format!("{shape} x=1 y=1")), "reserved", shape);
        }
        reject(blank("circle x=1 y=1"), "unknown shape", "circle");
        reject(
            blank("x=1 y=1 width=2 height=2 rect"),
            "not a `key=value` field",
            "shape word not first",
        );
    }

    #[test]
    fn the_rect_key_set_is_closed_exact_and_duplicate_free() {
        reject(
            blank("rect x=1 y=1 width=2"),
            "missing `height=`",
            "missing key",
        );
        reject(
            blank("rect x=1 y=1 width=2 height=2 cx=3"),
            "does not apply to `rect`",
            "inapplicable ellipse key",
        );
        reject(
            blank("rect x=1 y=1 width=2 height=2 rot=45"),
            "unknown key `rot=`",
            "unknown key",
        );
        reject(
            blank("rect x=1 x=2 y=1 width=2 height=2"),
            "duplicate `x`",
            "duplicate key",
        );
        reject(
            blank("rect x=1 y=1 width=2 height=2 occurrence=1"),
            "locate a `span`",
            "span key on rect",
        );
        reject(
            blank("rect x=1 y=1 width=2 height=2 from=0 to=4"),
            "time key",
            "time keys",
        );
        reject(
            blank("rect x=1 y=1 width=2 height=2 points=1,2"),
            "reserved shape",
            "polygon key",
        );
    }

    #[test]
    fn the_numeric_domain_rejects_every_non_plain_form() {
        for (value, case) in [
            ("1e3", "exponent"),
            ("+4", "leading plus"),
            ("-4", "negative"),
            ("1_000", "separator"),
            (".5", "bare fraction"),
            ("5.", "trailing dot"),
            ("1.2.3", "double dot"),
            ("", "empty"),
            ("abc", "letters"),
        ] {
            reject(
                blank(&format!("rect x={value} y=1 width=2 height=2")),
                "not a plain non-negative number",
                case,
            );
        }
    }

    #[test]
    fn sizes_are_strictly_positive_and_percentages_bounded() {
        reject(
            blank("rect x=0 y=0 width=0 height=2"),
            "strictly positive",
            "zero width",
        );
        reject(
            blank("rect x=0 y=0 width=2 height=0.0"),
            "strictly positive",
            "zero height",
        );
        reject(
            blank("rect x=101% y=0% width=2% height=2%"),
            "exceeds 100%",
            "percent over",
        );
        assert!(
            blank("rect x=0 y=0 width=0.5 height=2").is_ok(),
            "fractional size is legal"
        );
        assert!(
            blank("rect x=100% y=0% width=2% height=2%").is_ok(),
            "100% is in range"
        );
    }

    #[test]
    fn a_region_carries_one_unit_across_its_fields() {
        reject(
            blank("rect x=10% y=0 width=2 height=2"),
            "one unit",
            "mixed px and percent",
        );
        assert!(
            blank("rect x=10% y=0% width=2% height=2%").is_ok(),
            "all percent"
        );
    }

    #[test]
    fn the_span_key_matrix_from_the_adr() {
        let span = |body: &str| blank(body).unwrap();
        let bare = span(r#"span hidden="der""#);
        assert_eq!(
            RegionGeometry::Span {
                occurrence: 1,
                boundary: Boundary::Word
            },
            bare.geometry,
            "both keys default"
        );
        assert!(blank(r#"span hidden="der" occurrence=2"#).is_ok());
        assert!(blank(r#"span hidden="fahr" boundary=char"#).is_ok());
        assert!(blank(r#"span hidden="fahr" boundary=char occurrence=3"#).is_ok());
        reject(
            blank(r#"span hidden="x" boundary=line"#),
            "neither `word` nor `char`",
            "bad boundary",
        );
        for bad in ["0", "-1", "2.5", "02"] {
            reject(
                blank(&format!(r#"span hidden="x" occurrence={bad}"#)),
                "1-based positive integer",
                bad,
            );
        }
        reject(
            blank("span occurrence=2"),
            "requires `hidden=",
            "span without hidden",
        );
        reject(
            blank(r#"span hidden="x" x=4"#),
            "no geometry keys",
            "geometry on span",
        );
    }

    #[test]
    fn quoted_values_escape_strictly_and_round_trip() {
        let hidden = |body: &str| blank(body).unwrap().hidden.unwrap();
        assert_eq!(hidden(r#"span hidden="\"Ja\"""#), "\"Ja\"");
        assert_eq!(hidden(r#"span hidden="a \\ b""#), "a \\ b");
        assert_eq!(hidden(r#"span hidden="--\>""#), "-->");
        assert_eq!(hidden(r#"span hidden="--!\>""#), "--!>");
        assert_eq!(
            hidden(r#"span hidden="a lone > stays bare""#),
            "a lone > stays bare"
        );
        reject(
            blank(r#"span hidden="a \n b""#),
            "unknown escape",
            "unknown escape",
        );
        reject(
            blank(r#"span hidden="open"#),
            "never closes",
            "unterminated",
        );
        reject(
            blank(r#"span hidden="trailing \"#),
            "never closes",
            "trailing backslash",
        );
        reject(
            blank(r#"span hidden="--!>""#),
            "would close the comment",
            "raw comment-end-bang",
        );
    }

    #[test]
    fn the_stamp_grammar_is_exactly_six_of_the_frozen_set() {
        assert!(
            blank("rect x=1 y=1 width=2 height=2 b:a1b2c3").is_ok(),
            "legal stamp"
        );
        for (raw, case) in [
            ("a1b2c", "five chars"),
            ("a1b2c3d", "seven chars"),
            ("A1B2C3", "uppercase"),
            ("i12345", "excluded i"),
            ("l12345", "excluded l"),
            ("o12345", "excluded o"),
            ("u12345", "excluded u"),
        ] {
            reject(
                blank(&format!("rect x=1 y=1 width=2 height=2 b:{raw}")),
                "six characters of the lowercase Crockford set",
                case,
            );
        }
    }

    #[test]
    fn a_cover_takes_no_group_and_no_stamp_but_keeps_inert_hidden() {
        reject(
            parse_cover("rect [legend] x=1 y=1 width=2 height=2", 3),
            "no group",
            "grouped cover",
        );
        reject(
            parse_cover("rect x=1 y=1 width=2 height=2 b:a1b2c3", 3),
            "no stamp",
            "stamped cover",
        );
        let cover =
            parse_cover(r#"rect x=1 y=1 width=2 height=2 hidden="legend text""#, 3).unwrap();
        assert_eq!(cover.kind, RegionKind::Cover);
        assert_eq!(cover.hidden.as_deref(), Some("legend text"));
    }

    #[test]
    fn a_crop_is_a_bare_rect_viewport_and_nothing_else() {
        let crop = parse_crop("rect x=0 y=0 width=800 height=600", 4).unwrap();
        assert_eq!(crop.width.value, 800.0);
        reject(
            parse_crop("span x=0 y=0 width=1 height=1", 4),
            "takes `rect` only",
            "crop span",
        );
        for extra in [
            r#"hidden="x""#,
            "[g]",
            "b:a1b2c3",
            "occurrence=1",
            "position:4",
        ] {
            reject(
                parse_crop(&format!("rect x=0 y=0 width=1 height=1 {extra}"), 4),
                "nothing else",
                extra,
            );
        }
        reject(
            parse_crop("rect x=0 y=0 width=1 height=1 from=0", 4),
            "time key",
            "crop time key",
        );
    }

    #[test]
    fn group_names_share_the_hole_name_grammar() {
        assert!(
            blank("rect [b-2_X] x=1 y=1 width=2 height=2").is_ok(),
            "legal name"
        );
        reject(
            blank("rect [] x=1 y=1 width=2 height=2"),
            "group name",
            "empty",
        );
        reject(
            blank("rect [a b] x=1 y=1 width=2 height=2"),
            "not a `key=value` field",
            "space splits the token",
        );
        reject(
            blank("rect [a.b] x=1 y=1 width=2 height=2"),
            "group name",
            "dot",
        );
        reject(
            blank("rect [x] [y] x=1 y=1 width=2 height=2"),
            "duplicate `[group]`",
            "two groups",
        );
    }

    #[test]
    fn a_blank_names_a_shape_or_is_rejected() {
        reject(blank(""), "names no shape", "empty body");
        reject(
            blank(r#"hidden="x" occurrence=1"#),
            "names no shape",
            "fields without shape",
        );
    }
}
