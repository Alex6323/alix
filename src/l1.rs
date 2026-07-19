//! The L1 Markdown deck parser (spec §3). Built aside; wired as THE parser in
//! the flip task.
//!
//! An L1 deck is a Markdown file: optional YAML frontmatter between `---`
//! fences, an optional `# H1` title plus preamble prose, then one card per
//! `## ` heading (column 0, outside a code fence). Inside a card, the first
//! `---` line preceded by a blank line or the heading divides a multi-line
//! front from the answer, `> ` lines form the note, `<!-- key: value -->`
//! comments are directives (including the identity token), and `\cloze{...}`
//! holes make the card cloze. ``` and `~~~` fences shield their content from
//! all structure except the `\cloze` marker, which is active everywhere.
//!
//! Hard failures (bad token charset, a non-string `id:`, cloze grammar
//! errors) are line-numbered [`L1Error`]s; soft findings (unknown keys,
//! retired values, indented `##`) collect in [`L1Deck::lints`] for doctor to
//! render, never printed here.

use std::{hash::Hasher, path::PathBuf, sync::Arc};

use thiserror::Error;
use twox_hash::XxHash64;
use yaml_rust2::{Yaml, YamlLoader};

use crate::{
    answer::Input,
    card::{Card, Direction},
    cloze::{BLANK, HIDDEN},
    config::Strictness,
    depth::Reveal,
    session::Order,
    token,
};

/// The closed six-char ASCII whitespace set prose is trimmed and collapsed
/// over (spec §3.5): tab, LF, VT, FF, CR, space. Deliberately not Unicode
/// whitespace; anything outside this set is content.
const WHITESPACE: [char; 6] = ['\t', '\n', '\x0B', '\x0C', '\r', ' '];

/// Line-leading markers a backslash escape renders literal (spec §3.5):
/// heading, note, divider, directive comment, and the two fence openers.
const ESCAPABLE: [&str; 6] = ["##", ">", "---", "<!--", "```", "~~~"];

/// A 1-based inclusive `(open, close)` line span (e.g. the two frontmatter
/// `---` fences).
pub type LineSpan = (usize, usize);

/// A parsed L1 deck: identity, display metadata, cards, and soft findings.
#[derive(Debug)]
pub struct L1Deck {
    /// The deck's identity token from frontmatter `id:`, charset-validated.
    /// `None` for an unstamped deck (always loadable, spec §3.6).
    pub deck_token: Option<String>,
    /// The `# H1` display title from the preamble, if present.
    pub title: Option<String>,
    /// The typed frontmatter (the §3.6 closed key set).
    pub frontmatter: Frontmatter,
    /// The cards in file order, cloze cards expanded into per-hole sub-cards.
    pub cards: Vec<Card>,
    /// Soft findings for doctor: unknown keys, retired values, indented `##`.
    pub lints: Vec<Lint>,
    /// The 1-based line span `(open, close)` of the frontmatter's two `---`
    /// fences, or `None` when the deck has no frontmatter. The stamp writer
    /// needs it to tell an absent block (prepend a canonical one) from a
    /// present one (splice a minted `id:` after its opener), spec §2.3.
    pub frontmatter_span: Option<LineSpan>,
}

/// The §3.6 closed frontmatter key set as typed fields. Unknown keys lint;
/// the reserved set (`tags`, `license`, `author`, `language`, `revision`,
/// `generated-by`, `generated-at`) is ignored without a lint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    /// The deck token (`id:`), which must be a quoted YAML string.
    pub id: Option<String>,
    /// Exam ground-truth sources (`source:`), a scalar or list.
    pub source: Vec<String>,
    /// Prerequisite decks (`requires:`), a scalar or list.
    pub requires: Vec<String>,
    /// Reference links for the tutor (`link:`), a scalar or list.
    pub link: Vec<String>,
    /// What a trace deck walks (`trace:`); marks the deck as a trace.
    pub trace: Option<String>,
    /// Deck-default reveal method (`reveal:`); `cloze` is retired here.
    pub reveal: Option<Reveal>,
    /// Deck-default card order (`order:`).
    pub order: Option<Order>,
    /// Deck-default input method (`input:`).
    pub input: Option<Input>,
    /// Deck-default review direction (`direction:`).
    pub direction: Option<Direction>,
    /// Directory that card `img:` / `img-back:` filenames resolve against
    /// (`img-dir:`). Absolute, or relative to the deck file's folder.
    pub img_dir: Option<PathBuf>,
    /// How strictly this deck's AI exam grades answers (`strictness:`). `None`
    /// uses the `[exam]` config default.
    pub strictness: Option<Strictness>,
    /// The live source root a frozen deck's `at:` snapshots came from
    /// (`origin:`). Cascades workspace `[defaults]` → deck → card; the tutor
    /// grounds in it for context and drift detection reads it. `None` for a
    /// non-frozen deck.
    pub origin: Option<String>,
    /// True when frontmatter exists but is not a YAML block mapping (e.g. a
    /// flow mapping `{source: [a]}`): still loadable, but the stamp writer
    /// can never splice an `id:` line in and must exclude the deck loudly
    /// (spec §2.3). Absent frontmatter stays spliceable (a canonical block is
    /// prepended instead).
    pub unspliceable: bool,
}

/// A soft, non-fatal finding surfaced through [`L1Deck::lints`] for doctor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    /// The 1-based line the finding points at.
    pub line: usize,
    /// What was found.
    pub kind: LintKind,
}

/// What a [`Lint`] found. Doctor renders these; `BadValue` is the one doctor
/// reports as an error (a known key whose value did not parse, spec §3.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintKind {
    /// A frontmatter or directive key outside the closed sets.
    UnknownKey {
        /// The unrecognized key, lowercased.
        key: String,
    },
    /// A known key whose value did not parse (includes the retired
    /// `reveal: cloze`). Doctor reports this as an error.
    BadValue {
        /// The key whose value failed.
        key: String,
        /// The offending value (or its YAML node kind).
        value: String,
    },
    /// A known per-card directive key given an empty value (e.g.
    /// `<!-- id: -->`): consumed silently otherwise, so a half-typed token
    /// would vanish with no signal.
    EmptyValue {
        /// The key with the empty value.
        key: String,
    },
    /// A `reveal:` directive on a card with cloze holes: linted, not obeyed
    /// (the holes are the trigger, spec §3.4).
    RevealOnCloze,
    /// An indented `##` line: content, but probably a mistyped card front.
    IndentedH2,
    /// A literal `\cloze` inside a hole: hole content is never re-scanned.
    ClozeInHole,
    /// A `<!--` line that does not close with `-->`: directives are single
    /// line; the line stays content.
    UnclosedComment,
    /// A fence opened but never closed by EOF: everything after it, cards
    /// included, was swallowed as its verbatim content (spec §3.5).
    UnclosedFence,
}

/// A hard parse failure, pointing at the offending line of the deck file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum L1Error {
    #[error("no `## ` card fronts: not a deck")]
    NotADeck,
    #[error("line {0}: frontmatter never closes (missing the terminating `---`)")]
    UnclosedFrontmatter(usize),
    #[error("line {line}: frontmatter is not valid yaml: {message}")]
    FrontmatterSyntax { line: usize, message: String },
    #[error("line {line}: `id:` must be a quoted string (`id: \"...\"`), got {found}")]
    NonStringId { line: usize, found: &'static str },
    #[error("line {line}: token `{token}` fails the charset `^[0-9a-z]+$`")]
    InvalidToken { line: usize, token: String },
    #[error("line {line}: control character {found} outside the whitespace set")]
    ControlChar { line: usize, found: String },
    #[error("line {0}: card front is empty")]
    EmptyFront(usize),
    #[error("line {0}: card front without an answer")]
    FrontWithoutAnswer(usize),
    #[error("line {0}: `\\cloze[` is reserved for a future per-hole pin; write `\\cloze{{...}}`")]
    ClozeBracketReserved(usize),
    #[error("line {0}: unclosed cloze hole (missing the closing `}}`)")]
    UnclosedHole(usize),
    #[error("line {0}: empty cloze hole")]
    EmptyHole(usize),
}

/// Parses L1 deck text into cards. `subject` is the deck's file name; it
/// becomes each card's display subject (and, until the flip task, still
/// feeds the legacy identity hash).
pub fn parse_l1(subject: &str, text: &str) -> Result<L1Deck, L1Error> {
    let document = parse_document(text)?;
    if document.cards.is_empty() {
        return Err(L1Error::NotADeck);
    }
    let subject: Arc<str> = Arc::from(subject);
    let mut lints = document.lints;
    let mut cards = Vec::new();
    for raw in document.cards {
        build_card(&subject, raw, &mut cards, &mut lints)?;
    }
    Ok(L1Deck {
        deck_token: document.frontmatter.id.clone(),
        title: document.title,
        frontmatter: document.frontmatter,
        cards,
        lints,
        frontmatter_span: document.frontmatter_span,
    })
}

/// The §7 canonical content of a card: front + answer, prose collapsed over
/// the closed 6-char ASCII whitespace set, fenced content verbatim, cloze
/// markers as literal text. Changes to this function are deliberate behavior
/// changes: they stale every persisted fingerprint (the store version byte
/// owns that), never silent refactors.
pub fn canonical_content(front: &str, back: &[String]) -> String {
    let mut out = collapse(front);
    let mut fence: Option<char> = None;
    let mut prose = String::new();
    for line in back {
        if let Some(ch) = fence {
            out.push('\n');
            out.push_str(line);
            if closes_fence(line, ch) {
                fence = None;
            }
        } else if let Some(ch) = fence_opener(line) {
            if !prose.is_empty() {
                out.push('\n');
                out.push_str(&prose);
                prose.clear();
            }
            out.push('\n');
            out.push_str(line);
            fence = Some(ch);
        } else {
            let collapsed = collapse(line);
            if !collapsed.is_empty() {
                if !prose.is_empty() {
                    prose.push(' ');
                }
                prose.push_str(&collapsed);
            }
        }
    }
    if !prose.is_empty() {
        out.push('\n');
        out.push_str(&prose);
    }
    out
}

/// 64-bit fingerprint of canonical content (twox-hash XxHash64).
pub fn content_fingerprint(front: &str, back: &[String]) -> u64 {
    let mut hasher = XxHash64::default();
    hasher.write(canonical_content(front, back).as_bytes());
    hasher.finish()
}

// ── Internal representation ──

/// The scanned document before cards are built: what the directives
/// snapshot test pins.
struct Document {
    frontmatter: Frontmatter,
    title: Option<String>,
    cards: Vec<RawCard>,
    lints: Vec<Lint>,
    /// The 1-based `(open, close)` line span of the frontmatter fences, or
    /// `None` when there is no frontmatter (see [`L1Deck::frontmatter_span`]).
    frontmatter_span: Option<LineSpan>,
}

/// One card as scanned: heading, routed body lines, note, directives.
struct RawCard {
    /// The heading's 1-based line number.
    line: usize,
    /// The heading text, trailing directives and hash run stripped.
    front: String,
    /// Content lines before the divider (the extra front lines when a
    /// divider exists; the whole answer when none does).
    front_extra: Vec<(usize, String)>,
    /// Content lines after the divider.
    back: Vec<(usize, String)>,
    /// Whether the divider has been seen.
    divided: bool,
    /// The concatenated `> ` note lines.
    note: Option<String>,
    /// The card's parsed §3.7 directive set.
    directives: CardDirectives,
}

/// The §3.7 per-card directive closed set, fully typed. The snapshot test
/// asserts this structure literally so silent key loss cannot hide.
#[derive(Debug, Default, PartialEq)]
struct CardDirectives {
    /// The identity token (`id:`), charset-validated at parse.
    token: Option<String>,
    /// `reveal:`; `cloze` is retired (holes are the trigger).
    reveal: Option<Reveal>,
    /// The line the `reveal:` directive sits on, for the cloze lint.
    reveal_line: Option<usize>,
    /// `input:` override.
    input: Option<Input>,
    /// `direction:` declaration (expanded at deck load, not here).
    direction: Option<Direction>,
    /// `img:` question-side image, raw value.
    img: Option<String>,
    /// `img-back:` answer-side image, raw value.
    img_back: Option<String>,
    /// `at:` trace locator, the `<asset>` part.
    at: Option<String>,
    /// The ` from <origin>` provenance split off a frozen `at:`.
    at_origin: Option<String>,
    /// `origin:` override.
    origin: Option<String>,
    /// `given:` lines, repeatable, in order.
    givens: Vec<String>,
    /// `math:` render hint, parsed but consumed nowhere until Arc B.
    math: Option<String>,
}

/// A piece of a cloze-scanned answer line: literal text or a hole.
enum Seg {
    Text(String),
    Hole(String),
}

/// Byte prep + frontmatter + line scan, stopping short of card building.
fn parse_document(text: &str) -> Result<Document, L1Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = prepare(text)?;
    let mut lints = Vec::new();
    let (frontmatter, body_start, frontmatter_span) = parse_frontmatter(&lines, &mut lints)?;
    let (title, cards) = scan(&lines, body_start, &mut lints)?;
    Ok(Document {
        frontmatter,
        title,
        cards,
        lints,
        frontmatter_span,
    })
}

/// Splits into lines, dropping one trailing CR per line (CRLF input) and
/// rejecting C0 controls outside the closed whitespace set.
fn prepare(text: &str) -> Result<Vec<&str>, L1Error> {
    let mut lines = Vec::new();
    for (idx, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(ch) = line
            .chars()
            .find(|c| (*c as u32) < 0x20 && !WHITESPACE.contains(c))
        {
            return Err(L1Error::ControlChar {
                line: idx + 1,
                found: format!("U+{:04X}", ch as u32),
            });
        }
        lines.push(line);
    }
    Ok(lines)
}

/// Trims over the closed whitespace set only, never Unicode whitespace.
fn trim_ws(s: &str) -> &str {
    s.trim_matches(&WHITESPACE[..])
}

/// Collapses every whitespace run (the closed set) to one space and trims.
fn collapse(s: &str) -> String {
    s.split(&WHITESPACE[..])
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The fence a column-0 line opens (``` or ~~~), if any.
fn fence_opener(line: &str) -> Option<char> {
    if line.starts_with("```") {
        Some('`')
    } else if line.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

/// Whether a column-0 line closes the open fence: a run of three or more of
/// its character with only whitespace after.
fn closes_fence(line: &str, ch: char) -> bool {
    let run = line.chars().take_while(|c| *c == ch).count();
    run >= 3 && line.chars().skip(run).all(|c| WHITESPACE.contains(&c))
}

// ── Frontmatter ──

/// Whether a line closes the frontmatter block (spec §3.1, amended): the
/// exact `---` run, followed only by closed-set whitespace. A line with
/// leading indentation (` ---`) does not match: content, not a closer, so a
/// `---` inside a YAML block scalar can't accidentally end the frontmatter.
fn closes_frontmatter(line: &str) -> bool {
    line.strip_prefix("---")
        .is_some_and(|rest| rest.chars().all(|c| WHITESPACE.contains(&c)))
}

/// Locates and loads the optional frontmatter (spec §3.1): it opens only if
/// the file's first content line is exactly `---`, and a missing close is a
/// hard error. Returns the frontmatter, the 0-based index scanning resumes
/// at, and the frontmatter's 1-based `(open, close)` fence-line span (`None`
/// when there is no frontmatter) for the stamp writer.
fn parse_frontmatter(
    lines: &[&str],
    lints: &mut Vec<Lint>,
) -> Result<(Frontmatter, usize, Option<LineSpan>), L1Error> {
    let Some(open) = lines.iter().position(|line| !trim_ws(line).is_empty()) else {
        return Ok((Frontmatter::default(), lines.len(), None));
    };
    if lines[open] != "---" {
        return Ok((Frontmatter::default(), 0, None));
    }
    let Some(close) = lines[open + 1..]
        .iter()
        .position(|line| closes_frontmatter(line))
        .map(|i| open + 1 + i)
    else {
        return Err(L1Error::UnclosedFrontmatter(open + 1));
    };
    let frontmatter = load_frontmatter(&lines[open + 1..close], open + 2, lints)?;
    Ok((frontmatter, close + 1, Some((open + 1, close + 1))))
}

/// Hands the frontmatter block to the YAML parser and types the §3.6 closed
/// key set. `first_line` is the block's first 1-based line number.
fn load_frontmatter(
    block: &[&str],
    first_line: usize,
    lints: &mut Vec<Lint>,
) -> Result<Frontmatter, L1Error> {
    let mut frontmatter = Frontmatter::default();
    let text = block.join("\n");
    if trim_ws(&text).is_empty() {
        return Ok(frontmatter);
    }
    let docs = YamlLoader::load_from_str(&text).map_err(|e| L1Error::FrontmatterSyntax {
        line: first_line + e.marker().line().saturating_sub(1),
        message: e.info().to_string(),
    })?;
    let Some(root) = docs.into_iter().next() else {
        return Ok(frontmatter);
    };
    // A null-scalar root (`null` or `~`) loads, but is not a block mapping: a
    // spliced `id:` line would land in front of the bare scalar, which
    // yaml-rust2 hard-rejects ("simple key expected"). Exclude it from
    // splicing the same way a flow mapping or non-mapping root is (spec §2.3).
    if root == Yaml::Null {
        frontmatter.unspliceable = true;
        return Ok(frontmatter);
    }
    let Yaml::Hash(mapping) = root else {
        frontmatter.unspliceable = true;
        return Ok(frontmatter);
    };
    // A flow mapping (`{key: value}`) loads, but offers no per-key line for
    // the stamp writer to splice a minted `id:` into (spec §2.3).
    if trim_ws(&text).starts_with('{') {
        frontmatter.unspliceable = true;
    }
    for (key_node, value) in &mapping {
        let Yaml::String(key) = key_node else {
            lints.push(Lint {
                line: first_line,
                kind: LintKind::UnknownKey {
                    key: format!("{key_node:?}"),
                },
            });
            continue;
        };
        let line = key_line(block, first_line, key);
        match key.as_str() {
            "id" => match value {
                Yaml::String(s) => {
                    if !token::is_valid(s) {
                        return Err(L1Error::InvalidToken {
                            line,
                            token: s.clone(),
                        });
                    }
                    frontmatter.id = Some(s.clone());
                }
                other => {
                    return Err(L1Error::NonStringId {
                        line,
                        found: yaml_kind(other),
                    });
                }
            },
            "source" => frontmatter.source = string_list(key, value, line, lints),
            "requires" => frontmatter.requires = string_list(key, value, line, lints),
            "link" => frontmatter.link = string_list(key, value, line, lints),
            "trace" => match value {
                Yaml::String(s) => frontmatter.trace = Some(s.clone()),
                other => lints.push(bad_value(line, key, yaml_kind(other).to_string())),
            },
            "reveal" => match value.as_str().and_then(parse_reveal) {
                Some(reveal) => frontmatter.reveal = Some(reveal),
                None => lints.push(bad_value(line, key, describe(value))),
            },
            "order" => match value.as_str().and_then(Order::parse) {
                Some(order) => frontmatter.order = Some(order),
                None => lints.push(bad_value(line, key, describe(value))),
            },
            "input" => match value.as_str().and_then(Input::parse) {
                Some(input) => frontmatter.input = Some(input),
                None => lints.push(bad_value(line, key, describe(value))),
            },
            "direction" => match value.as_str().and_then(Direction::parse) {
                Some(direction) => frontmatter.direction = Some(direction),
                None => lints.push(bad_value(line, key, describe(value))),
            },
            "img-dir" => match value {
                Yaml::String(s) => frontmatter.img_dir = Some(PathBuf::from(s)),
                other => lints.push(bad_value(line, key, yaml_kind(other).to_string())),
            },
            "strictness" => match value.as_str().and_then(Strictness::parse) {
                Some(strictness) => frontmatter.strictness = Some(strictness),
                None => lints.push(bad_value(line, key, describe(value))),
            },
            "origin" => match value {
                Yaml::String(s) => {
                    let v = trim_ws(s);
                    if !v.is_empty() {
                        frontmatter.origin = Some(v.to_string());
                    }
                }
                other => lints.push(bad_value(line, key, yaml_kind(other).to_string())),
            },
            // Reserved for future deck metadata: ignored without a lint.
            "tags" | "license" | "author" | "language" | "revision" | "generated-by"
            | "generated-at" => {}
            _ => lints.push(Lint {
                line,
                kind: LintKind::UnknownKey { key: key.clone() },
            }),
        }
    }
    Ok(frontmatter)
}

/// `reveal:` values in L1: `cloze` is retired (holes are the trigger).
fn parse_reveal(value: &str) -> Option<Reveal> {
    Reveal::parse(value).filter(|reveal| *reveal != Reveal::Cloze)
}

/// A scalar's text for a lint message, or its node kind when it has none.
fn describe(value: &Yaml) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| yaml_kind(value).to_string())
}

/// The kind of a YAML node, phrased for an error message.
fn yaml_kind(node: &Yaml) -> &'static str {
    match node {
        Yaml::Null => "null",
        Yaml::Boolean(_) => "a boolean",
        Yaml::Integer(_) => "an integer",
        Yaml::Real(_) => "a float",
        Yaml::String(_) => "a string",
        Yaml::Array(_) => "a sequence",
        Yaml::Hash(_) => "a mapping",
        _ => "an unsupported node",
    }
}

/// A `source:`/`requires:`/`link:` value: a scalar string or a list of them.
fn string_list(key: &str, value: &Yaml, line: usize, lints: &mut Vec<Lint>) -> Vec<String> {
    match value {
        Yaml::String(s) => vec![s.clone()],
        Yaml::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    Yaml::String(s) => out.push(s.clone()),
                    other => lints.push(bad_value(line, key, yaml_kind(other).to_string())),
                }
            }
            out
        }
        other => {
            lints.push(bad_value(line, key, yaml_kind(other).to_string()));
            Vec::new()
        }
    }
}

/// Best-effort line locator for a frontmatter key: the block-mapping line
/// starting with `key:`, else any line containing the key (flow mappings),
/// else the block's first line.
fn key_line(block: &[&str], first_line: usize, key: &str) -> usize {
    for (i, line) in block.iter().enumerate() {
        if let Some(rest) = trim_ws(line).strip_prefix(key)
            && rest.trim_start_matches(&WHITESPACE[..]).starts_with(':')
        {
            return first_line + i;
        }
    }
    for (i, line) in block.iter().enumerate() {
        if line.contains(key) {
            return first_line + i;
        }
    }
    first_line
}

/// A `BadValue` lint (doctor reports these as errors).
fn bad_value(line: usize, key: &str, value: String) -> Lint {
    Lint {
        line,
        kind: LintKind::BadValue {
            key: key.to_string(),
            value,
        },
    }
}

// ── The line scanner ──

/// Walks the body lines with fence state, producing the title and the raw
/// cards. Structure is decided per line: headings and fences at column 0,
/// divider/note/directive lines by their trimmed text.
fn scan(
    lines: &[&str],
    start: usize,
    lints: &mut Vec<Lint>,
) -> Result<(Option<String>, Vec<RawCard>), L1Error> {
    let mut title: Option<String> = None;
    let mut cards: Vec<RawCard> = Vec::new();
    let mut current: Option<RawCard> = None;
    // The fence character plus the opener's line number, so an EOF lint can
    // name where a still-open fence began.
    let mut fence: Option<(char, usize)> = None;
    // The divider rule looks at the physically previous line: blank, or the
    // card's heading.
    let mut prev_blank = false;
    let mut prev_heading = false;

    for (idx, raw) in lines.iter().enumerate().skip(start) {
        let lineno = idx + 1;
        let raw = *raw;

        // Inside a fence everything is verbatim content; only the matching
        // closer (column 0) ends it.
        if let Some((ch, _)) = fence {
            if closes_fence(raw, ch) {
                fence = None;
            }
            push_content(&mut current, lineno, raw.to_string());
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        // A fence opener (column 0) starts verbatim mode.
        if let Some(ch) = fence_opener(raw) {
            fence = Some((ch, lineno));
            push_content(&mut current, lineno, raw.to_string());
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        // A card front: `## ` at column 0.
        if let Some(rest) = raw.strip_prefix("## ") {
            if let Some(card) = current.take() {
                cards.push(card);
            }
            let (front, directives) = heading(rest, lineno, lints)?;
            if front.is_empty() {
                return Err(L1Error::EmptyFront(lineno));
            }
            current = Some(RawCard {
                line: lineno,
                front,
                front_extra: Vec::new(),
                back: Vec::new(),
                divided: false,
                note: None,
                directives,
            });
            prev_blank = false;
            prev_heading = true;
            continue;
        }

        let t = trim_ws(raw);

        if t.is_empty() {
            prev_blank = true;
            prev_heading = false;
            continue;
        }

        // An escaped structural marker: drop the backslash, keep the line as
        // plain content. Any other backslash is literal and falls through.
        if let Some(rest) = t.strip_prefix('\\')
            && ESCAPABLE.iter().any(|marker| rest.starts_with(marker))
        {
            push_content(&mut current, lineno, rest.to_string());
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        // The divider (spec §3.3): the first `---` in a card, outside a
        // fence, preceded by a blank line or the heading. Every other `---`
        // is content; in the preamble it is prose.
        if t == "---" {
            let divides =
                current.as_ref().is_some_and(|card| !card.divided) && (prev_blank || prev_heading);
            if divides && let Some(card) = current.as_mut() {
                card.divided = true;
            } else {
                push_content(&mut current, lineno, "---".to_string());
            }
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        // A `>` note line; consecutive lines concatenate.
        if let Some(rest) = t.strip_prefix('>') {
            if let Some(card) = current.as_mut() {
                let text = rest.strip_prefix(' ').unwrap_or(rest);
                append_note(card, text);
            }
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        // A `<!-- key: value -->` directive line (single line). A comment
        // that is no directive is consumed silently; preamble directives are
        // ignored (deck configuration lives in frontmatter).
        if t.starts_with("<!--") {
            if let Some(body) = t.strip_prefix("<!--").and_then(|s| s.strip_suffix("-->")) {
                if let Some((key, value)) = directive(body)
                    && let Some(card) = current.as_mut()
                {
                    apply_directive(&mut card.directives, &key, value, lineno, lints)?;
                }
                prev_blank = false;
                prev_heading = false;
                continue;
            }
            lints.push(Lint {
                line: lineno,
                kind: LintKind::UnclosedComment,
            });
            // The line stays content.
        }

        // An indented `##` is content, but likely a mistyped front.
        if t.starts_with("## ") {
            lints.push(Lint {
                line: lineno,
                kind: LintKind::IndentedH2,
            });
        }

        if current.is_none() {
            // The preamble: the first `# H1` becomes the title, everything
            // else is prose and stays unparsed.
            if title.is_none()
                && let Some(rest) = raw.strip_prefix("# ")
            {
                title = Some(strip_trailing_hashes(trim_ws(rest)).to_string());
            }
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        push_content(&mut current, lineno, t.to_string());
        prev_blank = false;
        prev_heading = false;
    }

    // A fence still open at EOF swallowed everything after it, including any
    // later `## ` headings, as its content: surface that instead of letting
    // cards vanish silently.
    if let Some((_, open_line)) = fence {
        lints.push(Lint {
            line: open_line,
            kind: LintKind::UnclosedFence,
        });
    }
    if let Some(card) = current.take() {
        cards.push(card);
    }
    Ok((title, cards))
}

/// Routes a content line into the current card (front or answer side of the
/// divider); preamble content has no card and is dropped.
fn push_content(current: &mut Option<RawCard>, lineno: usize, text: String) {
    if let Some(card) = current.as_mut() {
        if card.divided {
            card.back.push((lineno, text));
        } else {
            card.front_extra.push((lineno, text));
        }
    }
}

/// Appends one note line, joining consecutive lines with newlines.
fn append_note(card: &mut RawCard, text: &str) {
    match &mut card.note {
        Some(note) => {
            note.push('\n');
            note.push_str(text);
        }
        slot => *slot = Some(text.to_string()),
    }
}

/// Parses a `## ` heading's text: trailing directive comments are extracted
/// and applied, then a trailing hash run is stripped.
fn heading(
    rest: &str,
    lineno: usize,
    lints: &mut Vec<Lint>,
) -> Result<(String, CardDirectives), L1Error> {
    let mut directives = CardDirectives::default();
    let (text, bodies) = split_trailing_comments(rest);
    for body in bodies {
        if let Some((key, value)) = directive(&body) {
            apply_directive(&mut directives, &key, value, lineno, lints)?;
        }
    }
    let front = strip_trailing_hashes(trim_ws(&text)).to_string();
    Ok((front, directives))
}

/// Splits trailing `<!-- ... -->` comments off a heading's text, returning
/// the remaining text and the comment bodies in document order. This is
/// where the stamped `<!-- id: ... -->` placement (spec §1.1) is read.
fn split_trailing_comments(text: &str) -> (String, Vec<String>) {
    let mut text = trim_ws(text);
    let mut bodies = Vec::new();
    loop {
        let Some(prefix) = text.strip_suffix("-->") else {
            break;
        };
        let Some(start) = prefix.rfind("<!--") else {
            break;
        };
        let body = &prefix[start + 4..];
        if body.contains("-->") {
            break;
        }
        bodies.push(body.to_string());
        text = trim_ws(&prefix[..start]);
    }
    bodies.reverse();
    (text.to_string(), bodies)
}

/// Strips a CommonMark closing hash run (`## Foo ##` renders "Foo"): only
/// when the run is preceded by whitespace or is the whole text.
fn strip_trailing_hashes(text: &str) -> &str {
    let stripped = text.trim_end_matches('#');
    if stripped.len() == text.len() {
        text
    } else if stripped.is_empty() || stripped.ends_with(WHITESPACE) {
        trim_ws(stripped)
    } else {
        text
    }
}

/// Parses a comment body as a `key: value` directive: lowercased key,
/// trimmed value (which may come back empty: the caller decides whether an
/// empty value on a known key warrants a lint). `None` for anything without
/// a `key:` shape at all (a prose comment), which is consumed as a plain
/// comment.
fn directive(body: &str) -> Option<(String, String)> {
    let (key, value) = trim_ws(body).split_once(':')?;
    let key = trim_ws(key).to_ascii_lowercase();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    Some((key, trim_ws(value).to_string()))
}

/// The §3.7 per-card directive keys that lint on an empty value, i.e. every
/// match arm in [`apply_directive`] except the reserved and unknown keys.
fn is_known_card_key(key: &str) -> bool {
    matches!(
        key,
        "id" | "reveal"
            | "input"
            | "direction"
            | "img"
            | "img-back"
            | "at"
            | "origin"
            | "given"
            | "math"
    )
}

/// Applies one directive to the card's set: the §3.7 closed keys, the
/// reserved keys silently, anything else linted, and an empty value on a
/// known key linted rather than dropped without a trace. A bad token is the
/// one hard error here.
fn apply_directive(
    directives: &mut CardDirectives,
    key: &str,
    value: String,
    line: usize,
    lints: &mut Vec<Lint>,
) -> Result<(), L1Error> {
    if value.is_empty() && is_known_card_key(key) {
        lints.push(Lint {
            line,
            kind: LintKind::EmptyValue {
                key: key.to_string(),
            },
        });
        return Ok(());
    }
    match key {
        "id" => {
            if !token::is_valid(&value) {
                return Err(L1Error::InvalidToken { line, token: value });
            }
            directives.token = Some(value);
        }
        "reveal" => match parse_reveal(&value) {
            Some(reveal) => {
                directives.reveal = Some(reveal);
                directives.reveal_line = Some(line);
            }
            None => lints.push(bad_value(line, key, value)),
        },
        "input" => match Input::parse(&value) {
            Some(input) => directives.input = Some(input),
            None => lints.push(bad_value(line, key, value)),
        },
        "direction" => match Direction::parse(&value) {
            Some(direction) => directives.direction = Some(direction),
            None => lints.push(bad_value(line, key, value)),
        },
        "img" => directives.img = Some(value),
        "img-back" => directives.img_back = Some(value),
        "at" => {
            let (at, origin) = split_at_origin(&value);
            directives.at = Some(at);
            directives.at_origin = origin;
        }
        "origin" => directives.origin = Some(value),
        "given" => directives.givens.push(value),
        "math" => directives.math = Some(value),
        // Reserved for future card media and occlusion: ignored, no lint.
        "occlude" | "audio" | "audio-back" | "img-alt" => {}
        _ => lints.push(Lint {
            line,
            kind: LintKind::UnknownKey {
                key: key.to_string(),
            },
        }),
    }
    Ok(())
}

/// Splits an `at:` value into its asset locator and the optional ` from
/// <origin>` provenance (`29.rs from src/caching.rs:46-66`). The separator
/// is spaced, so a path like `from_x.rs` stays intact.
fn split_at_origin(value: &str) -> (String, Option<String>) {
    match value.split_once(" from ") {
        Some((at, origin)) => (trim_ws(at).to_string(), Some(trim_ws(origin).to_string())),
        None => (trim_ws(value).to_string(), None),
    }
}

// ── Card building and cloze ──

/// Builds one scanned card into `cards`: plain, or expanded per cloze hole.
fn build_card(
    subject: &Arc<str>,
    raw: RawCard,
    cards: &mut Vec<Card>,
    lints: &mut Vec<Lint>,
) -> Result<(), L1Error> {
    let RawCard {
        line,
        front: heading,
        front_extra,
        back,
        divided,
        note,
        directives,
    } = raw;
    let (front, answer) = if divided {
        let mut front = heading;
        for (_, text) in &front_extra {
            front.push('\n');
            front.push_str(text);
        }
        (front, back)
    } else {
        // No divider: the heading is the whole front, everything else is
        // the answer (spec §3.3).
        (heading, front_extra)
    };
    if answer.is_empty() {
        return Err(L1Error::FrontWithoutAnswer(line));
    }

    // Scan every answer line for cloze holes. Text segments also apply the
    // `\\cloze` escape, so plain cards pass through the same scanner.
    let mut parsed = Vec::with_capacity(answer.len());
    for (lineno, text) in &answer {
        parsed.push(scan_cloze(text, *lineno, lints)?);
    }
    let holes: Vec<(usize, usize, &str)> = parsed
        .iter()
        .enumerate()
        .flat_map(|(li, segments)| {
            segments
                .iter()
                .enumerate()
                .filter_map(move |(si, segment)| match segment {
                    Seg::Hole(h) => Some((li, si, h.as_str())),
                    Seg::Text(_) => None,
                })
        })
        .collect();

    if holes.is_empty() {
        let back_lines: Vec<String> = parsed.iter().map(|segments| seg_text(segments)).collect();
        let mut card = Card::plain(Arc::clone(subject), front, back_lines, note, line);
        card.token = directives.token.as_deref().map(Arc::from);
        card.reveal = directives.reveal;
        card.input = directives.input;
        card.direction = directives.direction;
        card.image = directives.img.map(PathBuf::from);
        card.image_back = directives.img_back.map(PathBuf::from);
        card.at = directives.at;
        card.at_origin = directives.at_origin;
        card.origin = directives.origin;
        card.givens = directives.givens;
        cards.push(card);
        return Ok(());
    }

    // A cloze card. `reveal:` is retired here: the holes are the trigger.
    if directives.reveal.is_some() {
        lints.push(Lint {
            line: directives.reveal_line.unwrap_or(line),
            kind: LintKind::RevealOnCloze,
        });
    }
    let token: Option<Arc<str>> = directives.token.as_deref().map(Arc::from);
    let structure: Vec<String> = parsed.iter().map(|segments| hash_repr(segments)).collect();
    for (n, (hole_line, hole_seg, answer_text)) in holes.iter().enumerate() {
        let context: Vec<String> = parsed
            .iter()
            .enumerate()
            .map(|(li, segments)| {
                let mut rendered = String::new();
                for (si, segment) in segments.iter().enumerate() {
                    match segment {
                        Seg::Text(text) => rendered.push_str(text),
                        Seg::Hole(_) if li == *hole_line && si == *hole_seg => {
                            rendered.push_str(BLANK);
                        }
                        Seg::Hole(_) => rendered.push_str(HIDDEN),
                    }
                }
                rendered
            })
            .collect();
        let mut hash_lines = structure.clone();
        hash_lines.push(format!("#cloze:{n}"));
        let mut card = Card::plain(
            Arc::clone(subject),
            front.clone(),
            vec![answer_text.to_string()],
            note.clone(),
            line,
        );
        card.context = context;
        card.hash_lines = Some(hash_lines);
        card.token = token.clone();
        card.hole = Some(n as u32);
        // A cloze sub-card never reverses and keeps no direction; the
        // per-card `input:` still applies to how each hole is answered.
        card.input = directives.input;
        cards.push(card);
    }
    Ok(())
}

/// Scans one answer line for `\cloze{...}` holes (spec §3.4). The marker is
/// active everywhere, fenced lines included; `\\cloze` renders a literal
/// `\cloze`; `\cloze[` is reserved.
fn scan_cloze(line_text: &str, lineno: usize, lints: &mut Vec<Lint>) -> Result<Vec<Seg>, L1Error> {
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
                    return Err(L1Error::EmptyHole(lineno));
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
                return Err(L1Error::ClozeBracketReserved(lineno));
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

/// Scans a hole's argument after the opening brace: brace-balanced, with
/// `\{`/`\}`/`\\` stripped (escaped braces do not count toward depth), any
/// other backslash literal, single line. Returns the content and the rest
/// of the line after the closing brace.
fn scan_hole(arg: &str, lineno: usize) -> Result<(String, &str), L1Error> {
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
    Err(L1Error::UnclosedHole(lineno))
}

/// Reassembles a scanned line's text segments (a plain line has no holes;
/// a hole is rendered back as a marker defensively).
fn seg_text(segments: &[Seg]) -> String {
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

/// The delimiter-free representation of a scanned line for the legacy
/// identity hash, mirroring [`crate::cloze`]: text runs verbatim, hole
/// content fenced by a unit-separator byte that cannot occur in deck input.
fn hash_repr(segments: &[Seg]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> L1Deck {
        parse_l1("deck.md", text).unwrap()
    }

    fn err(text: &str) -> L1Error {
        parse_l1("deck.md", text).unwrap_err()
    }

    fn unknown(line: usize, key: &str) -> Lint {
        Lint {
            line,
            kind: LintKind::UnknownKey { key: key.into() },
        }
    }

    fn bad(line: usize, key: &str, value: &str) -> Lint {
        Lint {
            line,
            kind: LintKind::BadValue {
                key: key.into(),
                value: value.into(),
            },
        }
    }

    // ── Frontmatter ──

    #[test]
    fn frontmatter_opens_only_as_the_first_content_line() {
        // Leading blank lines are skipped: the first content line opens it.
        let deck = parse("\n---\ntrace: a walk\n---\n## q\n---\na\n");
        assert_eq!(Some("a walk".to_string()), deck.frontmatter.trace);
        assert_eq!(1, deck.cards.len());

        // After any content, a `---` is never frontmatter.
        let deck = parse("intro prose\n---\nid: nope\n---\n## q\na\n");
        assert_eq!(Frontmatter::default(), deck.frontmatter);
        assert_eq!(None, deck.deck_token);
    }

    #[test]
    fn a_missing_frontmatter_close_is_a_hard_error() {
        assert_eq!(
            L1Error::UnclosedFrontmatter(1),
            err("---\nid: \"abc\"\n## q\na\n")
        );
    }

    #[test]
    fn a_frontmatter_closer_tolerates_trailing_whitespace() {
        let deck = parse("---\ntrace: a walk\n--- \n## q\na\n");
        assert_eq!(Some("a walk".to_string()), deck.frontmatter.trace);
        assert_eq!(1, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);

        // Leading indentation stays content (it protects a `---` inside a
        // YAML block scalar), so this frontmatter never closes.
        assert_eq!(
            L1Error::UnclosedFrontmatter(1),
            err("---\ntrace: a walk\n ---\n## q\na\n")
        );
    }

    #[test]
    fn an_unquoted_numeric_id_is_a_hard_error_naming_the_line() {
        assert_eq!(
            L1Error::NonStringId {
                line: 2,
                found: "an integer"
            },
            err("---\nid: 007\n---\n## q\na\n")
        );
    }

    #[test]
    fn a_bool_id_is_a_hard_error() {
        assert_eq!(
            L1Error::NonStringId {
                line: 2,
                found: "a boolean"
            },
            err("---\nid: true\n---\n## q\na\n")
        );
    }

    #[test]
    fn a_quoted_id_parses_verbatim() {
        let deck = parse("---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n");
        assert_eq!(
            Some("9w2c7x4k1m8q3z5t0v6b2n4d8f"),
            deck.deck_token.as_deref()
        );
        assert_eq!(deck.deck_token, deck.frontmatter.id);
        assert!(!deck.frontmatter.unspliceable);
    }

    #[test]
    fn a_flow_mapping_frontmatter_parses_but_is_reported_unspliceable() {
        let deck = parse("---\n{source: [a]}\n---\n## q\nb\n");
        assert_eq!(vec!["a".to_string()], deck.frontmatter.source);
        assert!(deck.frontmatter.unspliceable);
    }

    #[test]
    fn a_null_scalar_frontmatter_is_unspliceable() {
        // A hand-authored `null` scalar loads (an empty frontmatter, in
        // effect), but is not a block mapping a stamp splice can key into.
        let deck = parse("---\nnull\n---\n## q\na\n");
        assert_eq!(None, deck.frontmatter.id);
        assert!(deck.frontmatter.unspliceable);

        // `~` is the other YAML null spelling; same outcome.
        let deck = parse("---\n~\n---\n## q\na\n");
        assert_eq!(None, deck.frontmatter.id);
        assert!(deck.frontmatter.unspliceable);
    }

    #[test]
    fn the_frontmatter_span_locates_the_fences_or_is_none() {
        // No frontmatter: no span (the stamp writer prepends a canonical one).
        assert_eq!(None, parse("## q\na\n").frontmatter_span);
        // A block mapping: the two `---` fence lines, 1-based.
        assert_eq!(
            Some((1, 3)),
            parse("---\nsource: x\n---\n## q\na\n").frontmatter_span
        );
        // Leading blank lines push the opener (and closer) down.
        assert_eq!(
            Some((2, 4)),
            parse("\n---\nsource: x\n---\n## q\na\n").frontmatter_span
        );
        // A flow mapping is present (has a span) but unspliceable.
        let deck = parse("---\n{source: [a]}\n---\n## q\nb\n");
        assert_eq!(Some((1, 3)), deck.frontmatter_span);
        assert!(deck.frontmatter.unspliceable);
    }

    #[test]
    fn an_id_failing_the_charset_is_a_line_numbered_error() {
        assert_eq!(
            L1Error::InvalidToken {
                line: 2,
                token: "ABC".into()
            },
            err("---\nid: \"ABC\"\n---\n## q\na\n")
        );
    }

    #[test]
    fn unknown_frontmatter_keys_are_linted_reserved_keys_are_not() {
        let deck = parse(
            "---\ntags: [x, y]\nlicense: MIT\nauthor: me\nlanguage: de\nrevision: 3\n\
             generated-by: alix\ngenerated-at: sometime\nfnord: 7\n---\n## q\na\n",
        );
        assert_eq!(vec![unknown(9, "fnord")], deck.lints);
    }

    #[test]
    fn invalid_frontmatter_yaml_is_a_hard_error() {
        let e = err("---\nid: [unclosed\n---\n## q\na\n");
        assert!(matches!(e, L1Error::FrontmatterSyntax { .. }), "{e:?}");
    }

    #[test]
    fn an_empty_frontmatter_is_fine() {
        let deck = parse("---\n---\n## q\na\n");
        assert_eq!(Frontmatter::default(), deck.frontmatter);
        assert!(!deck.frontmatter.unspliceable);
    }

    #[test]
    fn frontmatter_lists_accept_a_scalar_as_a_singleton() {
        let deck = parse("---\nsource: notes.md\nrequires: basics\n---\n## q\na\n");
        assert_eq!(vec!["notes.md".to_string()], deck.frontmatter.source);
        assert_eq!(vec!["basics".to_string()], deck.frontmatter.requires);
    }

    // ── Document structure ──

    #[test]
    fn a_file_with_no_h2_fronts_is_not_a_deck() {
        assert_eq!(L1Error::NotADeck, err("# Title\njust prose\n"));
    }

    #[test]
    fn preamble_prose_and_h1_title_precede_the_first_card() {
        let deck = parse("# My Deck\nsome intro prose\n\n## q\n---\na\n");
        assert_eq!(Some("My Deck"), deck.title.as_deref());
        assert_eq!(1, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(vec!["a"], deck.cards[0].back);
    }

    #[test]
    fn a_card_runs_from_its_h2_to_the_next_h2_or_eof() {
        let deck = parse("## first\nalpha\nbeta\n## second\ngamma\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!("first", deck.cards[0].front);
        assert_eq!(vec!["alpha", "beta"], deck.cards[0].back);
        assert_eq!(1, deck.cards[0].line);
        assert_eq!("second", deck.cards[1].front);
        assert_eq!(vec!["gamma"], deck.cards[1].back);
        assert_eq!(4, deck.cards[1].line);
    }

    #[test]
    fn an_h2_inside_a_fence_does_not_open_a_card() {
        let deck = parse("## q\n---\n```\n## not a front\n```\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["```", "## not a front", "```"], deck.cards[0].back);
    }

    #[test]
    fn an_unclosed_fence_at_eof_is_linted() {
        // The trailing empty line from the final `\n` is itself inside the
        // still-open fence, so it is swallowed as content too.
        let deck = parse("## q\n---\na\n```\nb\n");
        assert_eq!(vec!["a", "```", "b", ""], deck.cards[0].back);
        assert_eq!(
            vec![Lint {
                line: 4,
                kind: LintKind::UnclosedFence
            }],
            deck.lints
        );
    }

    #[test]
    fn a_fence_closer_with_trailing_text_stays_inside_the_fence() {
        let deck = parse("## q\n---\nbefore\n```\n```rust\n## x\n```\nafter\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(
            vec!["before", "```", "```rust", "## x", "```", "after"],
            deck.cards[0].back
        );
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);
    }

    #[test]
    fn a_cloze_hole_on_a_fenced_line_is_still_a_hole() {
        let deck = parse("## q\n---\n```\nlet x = \\cloze{5};\n```\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
        assert_eq!(vec!["5"], deck.cards[0].back);
        assert_eq!(vec!["```", "let x = ____;", "```"], deck.cards[0].context);
    }

    #[test]
    fn an_indented_h2_is_content_and_linted() {
        let deck = parse("## q\n  ## indented\n");
        assert_eq!(vec!["## indented"], deck.cards[0].back);
        assert_eq!(
            vec![Lint {
                line: 2,
                kind: LintKind::IndentedH2
            }],
            deck.lints
        );
    }

    #[test]
    fn a_trailing_hash_run_is_stripped_from_the_front() {
        let deck = parse("## Foo ##\nbar\n");
        assert_eq!("Foo", deck.cards[0].front);
    }

    #[test]
    fn a_card_with_no_answer_is_an_error() {
        assert_eq!(L1Error::FrontWithoutAnswer(1), err("## q\n## r\nb\n"));
        assert_eq!(L1Error::FrontWithoutAnswer(1), err("## q\n---\n"));
    }

    // ── Divider, answer, notes ──

    #[test]
    fn the_first_bare_divider_splits_front_from_answer() {
        let deck = parse("## Q\nmore question\n\n---\nthe answer\n");
        assert_eq!("Q\nmore question", deck.cards[0].front);
        assert_eq!(vec!["the answer"], deck.cards[0].back);
    }

    #[test]
    fn a_divider_needs_a_blank_line_or_the_heading_before_it() {
        // Directly after a content line the `---` is content, not a divider.
        let deck = parse("## Q\ntext\n---\nanswer\n");
        assert_eq!("Q", deck.cards[0].front);
        assert_eq!(vec!["text", "---", "answer"], deck.cards[0].back);

        // Directly after the heading it divides.
        let deck = parse("## Q\n---\nanswer\n");
        assert_eq!(vec!["answer"], deck.cards[0].back);
    }

    #[test]
    fn later_dividers_and_four_dashes_are_content() {
        let deck = parse("## Q\n\n---\na\n\n---\n----\nb\n");
        assert_eq!(vec!["a", "---", "----", "b"], deck.cards[0].back);
    }

    #[test]
    fn consecutive_quote_lines_concatenate_into_the_note() {
        let deck = parse("## q\n---\nans\n> one\n> two\n");
        assert_eq!(Some("one\ntwo".to_string()), deck.cards[0].note);
    }

    // ── Directives ──

    #[test]
    fn an_id_directive_yields_the_card_token() {
        // Trailing on the heading (the stamped placement) and on its own line.
        let deck = parse(
            "## q <!-- id: 4jkya9q3m8z0tw5v9y2b4n6d8f -->\n---\na\n## r\n---\nb\n\
             <!-- id: 0m5v2 -->\n",
        );
        assert_eq!(
            Some("4jkya9q3m8z0tw5v9y2b4n6d8f"),
            deck.cards[0].token.as_deref()
        );
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(Some("0m5v2"), deck.cards[1].token.as_deref());
    }

    #[test]
    fn a_token_failing_the_charset_is_a_line_numbered_error() {
        assert_eq!(
            L1Error::InvalidToken {
                line: 4,
                token: "XYZ".into()
            },
            err("## q\n---\na\n<!-- id: XYZ -->\n")
        );
    }

    #[test]
    fn directives_parse_the_closed_set_and_lint_unknown_keys() {
        let deck = parse(
            "## q\n---\na\n<!-- reveal: line -->\n<!-- input: draw -->\n\
             <!-- direction: both -->\n<!-- flavor: cherry -->\n",
        );
        assert_eq!(Some(Reveal::Line), deck.cards[0].reveal);
        assert_eq!(Some(Input::Draw), deck.cards[0].input);
        assert_eq!(Some(Direction::Both), deck.cards[0].direction);
        assert_eq!(vec![unknown(7, "flavor")], deck.lints);
    }

    #[test]
    fn at_keeps_its_asset_from_origin_split() {
        let deck = parse("## q\n---\na\n<!-- at: 29.rs from src/caching.rs:46-66 -->\n");
        assert_eq!(Some("29.rs".to_string()), deck.cards[0].at);
        assert_eq!(
            Some("src/caching.rs:46-66".to_string()),
            deck.cards[0].at_origin
        );

        // Without the spaced ` from ` separator the locator stays whole.
        let deck = parse("## q\n---\na\n<!-- at: src/from_x.rs:1-3 -->\n");
        assert_eq!(Some("src/from_x.rs:1-3".to_string()), deck.cards[0].at);
        assert_eq!(None, deck.cards[0].at_origin);
    }

    #[test]
    fn given_is_repeatable() {
        let deck = parse(
            "## q\n---\na\n<!-- given: state - the parser position -->\n\
             <!-- given: partial - the card -->\n",
        );
        assert_eq!(
            vec![
                "state - the parser position".to_string(),
                "partial - the card".to_string(),
            ],
            deck.cards[0].givens
        );
    }

    #[test]
    fn a_known_directive_key_with_a_bad_value_is_reported() {
        let deck = parse("---\nreveal: cloze\n---\n## q\n---\na\n<!-- direction: sideways -->\n");
        assert_eq!(None, deck.frontmatter.reveal);
        assert_eq!(None, deck.cards[0].direction);
        assert_eq!(
            vec![bad(2, "reveal", "cloze"), bad(7, "direction", "sideways")],
            deck.lints
        );
    }

    #[test]
    fn an_empty_valued_known_directive_key_is_linted() {
        let deck = parse("## q\n---\na\n<!-- id: -->\n");
        assert_eq!(None, deck.cards[0].token);
        assert_eq!(
            vec![Lint {
                line: 4,
                kind: LintKind::EmptyValue { key: "id".into() }
            }],
            deck.lints
        );
    }

    #[test]
    fn img_dir_strictness_and_origin_follow_the_leniency_model_of_comparable_keys() {
        // `strictness`, like `reveal`/`order`/`input`/`direction`, lints a
        // value that fails to parse and leaves the field unset.
        let deck = parse("---\nstrictness: extreme\n---\n## q\na\n");
        assert_eq!(None, deck.frontmatter.strictness);
        assert_eq!(vec![bad(2, "strictness", "extreme")], deck.lints);

        // `img-dir`, like `trace`, lints a non-string value.
        let deck = parse("---\nimg-dir: [a, b]\n---\n## q\na\n");
        assert_eq!(None, deck.frontmatter.img_dir);
        assert_eq!(vec![bad(2, "img-dir", "a sequence")], deck.lints);

        // `origin` mirrors deck.rs's trim-and-ignore-empty: a blank value is
        // silently ignored (no lint), but a non-string value still lints.
        let deck = parse("---\norigin: \"   \"\n---\n## q\na\n");
        assert_eq!(None, deck.frontmatter.origin);
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);

        let deck = parse("---\norigin: 5\n---\n## q\na\n");
        assert_eq!(None, deck.frontmatter.origin);
        assert_eq!(vec![bad(2, "origin", "an integer")], deck.lints);
    }

    // ── Escapes and bytes ──

    #[test]
    fn escaped_structural_markers_render_literal() {
        let deck = parse("## q\n---\n\\## x\n\\> y\n\\---\n\\<!-- z -->\n\\```\n> real note\n");
        assert_eq!(
            vec!["## x", "> y", "---", "<!-- z -->", "```"],
            deck.cards[0].back
        );
        // The escaped fence opener opened nothing: the `>` line is a note.
        assert_eq!(Some("real note".to_string()), deck.cards[0].note);
    }

    #[test]
    fn a_backslash_before_anything_else_is_literal() {
        let deck = parse("## q\n---\n\\d is a digit class\n\\# x\n");
        assert_eq!(vec!["\\d is a digit class", "\\# x"], deck.cards[0].back);
    }

    #[test]
    fn one_leading_bom_is_stripped() {
        let deck = parse("\u{feff}## q\n---\na\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);

        // Only one: a second BOM keeps the heading off column 0.
        assert_eq!(L1Error::NotADeck, err("\u{feff}\u{feff}## q\n---\na\n"));
    }

    #[test]
    fn crlf_line_endings_normalize() {
        let deck = parse("## q\r\n\r\n---\r\nan answer\r\n");
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(vec!["an answer"], deck.cards[0].back);
    }

    #[test]
    fn a_c0_control_outside_whitespace_is_a_line_numbered_error() {
        assert_eq!(
            L1Error::ControlChar {
                line: 3,
                found: "U+0007".into()
            },
            err("## q\n---\na\u{7} bell\n")
        );
        // VT and FF are inside the closed whitespace set: no error.
        assert!(parse_l1("deck.md", "## q\n---\na\u{b}b\n").is_ok());
    }

    #[test]
    fn fenced_content_is_verbatim_and_structurally_inert() {
        let deck = parse(
            "## q\n---\nbefore\n```\n## x\n> quoted\n<!-- id: zz -->\n---\n\\## kept\n```\nafter\n",
        );
        assert_eq!(1, deck.cards.len());
        assert_eq!(
            vec![
                "before",
                "```",
                "## x",
                "> quoted",
                "<!-- id: zz -->",
                "---",
                "\\## kept",
                "```",
                "after",
            ],
            deck.cards[0].back
        );
        assert_eq!(None, deck.cards[0].token);
        assert_eq!(None, deck.cards[0].note);
        assert!(deck.lints.is_empty());
    }

    // ── Cloze ──

    #[test]
    fn a_cloze_marker_makes_the_card_cloze_and_numbers_holes_in_document_order() {
        let deck = parse("## fill\n---\nthe \\cloze{quick} fox\njumps \\cloze{over}\n");
        assert_eq!(2, deck.cards.len());

        assert_eq!("fill", deck.cards[0].front);
        assert_eq!(Some(0), deck.cards[0].hole);
        assert_eq!(vec!["quick"], deck.cards[0].back);
        assert_eq!(vec!["the ____ fox", "jumps […]"], deck.cards[0].context);

        assert_eq!(Some(1), deck.cards[1].hole);
        assert_eq!(vec!["over"], deck.cards[1].back);
        assert_eq!(vec!["the […] fox", "jumps ____"], deck.cards[1].context);
    }

    #[test]
    fn bare_cloze_without_a_brace_is_literal() {
        let deck = parse("## q\n---\na \\cloze marker\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(None, deck.cards[0].hole);
        assert_eq!(vec!["a \\cloze marker"], deck.cards[0].back);
    }

    #[test]
    fn double_backslash_cloze_is_a_literal_marker() {
        let deck = parse("## q\n---\na \\\\cloze{x} b\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(None, deck.cards[0].hole);
        assert_eq!(vec!["a \\cloze{x} b"], deck.cards[0].back);
    }

    #[test]
    fn cloze_bracket_is_a_reserved_parse_error() {
        assert_eq!(
            L1Error::ClozeBracketReserved(3),
            err("## q\n---\na \\cloze[x]{y} b\n")
        );
    }

    #[test]
    fn escaped_braces_inside_a_hole_are_stripped_and_do_not_count() {
        let deck = parse("## q\n---\nw \\cloze{a \\{b\\} c} z\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["a {b} c"], deck.cards[0].back);
        assert_eq!(vec!["w ____ z"], deck.cards[0].context);

        // Unbalanced proof that escapes add no depth: the escaped `{` does
        // not open a level, so the next bare `}` closes the hole.
        let deck = parse("## q\n---\nw \\cloze{a \\{b} c\n");
        assert_eq!(vec!["a {b"], deck.cards[0].back);
        assert_eq!(vec!["w ____ c"], deck.cards[0].context);
    }

    #[test]
    fn backslash_backslash_inside_a_hole_is_a_literal_backslash() {
        let deck = parse("## q\n---\nw \\cloze{a\\\\b} z\n");
        assert_eq!(vec!["a\\b"], deck.cards[0].back);
    }

    #[test]
    fn an_unclosed_hole_is_a_line_numbered_error() {
        assert_eq!(L1Error::UnclosedHole(3), err("## q\n---\nw \\cloze{oops\n"));
    }

    #[test]
    fn an_empty_hole_is_an_error() {
        assert_eq!(L1Error::EmptyHole(3), err("## q\n---\nw \\cloze{} z\n"));
        assert_eq!(L1Error::EmptyHole(3), err("## q\n---\nw \\cloze{  } z\n"));
    }

    #[test]
    fn hole_content_is_not_rescanned() {
        let deck = parse("## q\n---\nw \\cloze{x \\cloze{y}} z\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["x \\cloze{y}"], deck.cards[0].back);
        assert_eq!(
            vec![Lint {
                line: 3,
                kind: LintKind::ClozeInHole
            }],
            deck.lints
        );
    }

    #[test]
    fn nested_balanced_braces_stay_inside_the_hole() {
        let deck = parse("## q\n---\nw \\cloze{f{g}h} z\n");
        assert_eq!(vec!["f{g}h"], deck.cards[0].back);
    }

    #[test]
    fn a_reveal_directive_on_a_cloze_card_is_linted_not_obeyed() {
        let deck = parse("## q\n---\na \\cloze{b} c\n<!-- reveal: line -->\n");
        assert_eq!(None, deck.cards[0].reveal);
        assert_eq!(
            vec![Lint {
                line: 4,
                kind: LintKind::RevealOnCloze
            }],
            deck.lints
        );
    }

    #[test]
    fn cloze_cards_never_produce_a_reversed_twin() {
        // A deck-wide `direction: both` and a per-card one: the hole sub-card
        // still comes out single, unreversed, and direction-free.
        let deck = parse(
            "---\ndirection: both\n---\n## q\n---\na \\cloze{b} c\n<!-- direction: both -->\n",
        );
        assert_eq!(Some(Direction::Both), deck.frontmatter.direction);
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
        assert!(!deck.cards[0].reversed);
        assert_eq!(None, deck.cards[0].direction);
    }

    #[test]
    fn a_plain_cards_direction_is_recorded_not_expanded() {
        // Direction expansion stays a deck-load concern; the parser only
        // records the declaration.
        let deck = parse("---\ndirection: both\n---\n## q\n---\na\n<!-- direction: both -->\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(Direction::Both), deck.cards[0].direction);
        assert!(!deck.cards[0].reversed);
    }

    // ── The directives snapshot ──

    /// The guard against silent key loss: one fixture exercising every §3.6
    /// and §3.7 key, asserted as a literal expected structure (id-based tests
    /// are blind to a dropped key).
    #[test]
    fn a_full_directive_fixture_parses_to_exactly_this_snapshot() {
        let text = r#"---
id: "9w2c7x4k1m8q3z5t0v6b2n4d8f"
source:
  - https://example.org/book
  - notes.md
requires:
  - basics
link:
  - https://docs.rs/tokio
trace: how a keypress becomes a grade
reveal: line
order: sequential
input: draw
direction: both
img-dir: assets
strictness: strict
origin: /crate
tags: [a, b]
license: MIT
author: someone
language: de
revision: 3
generated-by: alix
generated-at: 2026-07-19
---
# The Title

## The question <!-- id: 4jkya9q3m8z0tw5v9y2b4n6d8f -->

---
the answer
<!-- reveal: flip -->
<!-- input: type -->
<!-- direction: reverse -->
<!-- img: moon.png -->
<!-- img-back: phase.png -->
<!-- at: 29.rs from src/caching.rs:46-66 -->
<!-- origin: /crate -->
<!-- given: state - the parser position -->
<!-- given: partial - the card -->
<!-- math: latex -->
<!-- occlude: soon -->
<!-- audio: a.mp3 -->
<!-- audio-back: b.mp3 -->
<!-- img-alt: a moon -->
"#;
        let document = parse_document(text).unwrap();
        assert_eq!(
            Frontmatter {
                id: Some("9w2c7x4k1m8q3z5t0v6b2n4d8f".into()),
                source: vec!["https://example.org/book".into(), "notes.md".into()],
                requires: vec!["basics".into()],
                link: vec!["https://docs.rs/tokio".into()],
                trace: Some("how a keypress becomes a grade".into()),
                reveal: Some(Reveal::Line),
                order: Some(Order::Sequential),
                input: Some(Input::Draw),
                direction: Some(Direction::Both),
                img_dir: Some(PathBuf::from("assets")),
                strictness: Some(Strictness::Strict),
                origin: Some("/crate".into()),
                unspliceable: false,
            },
            document.frontmatter
        );
        assert_eq!(Some("The Title"), document.title.as_deref());
        assert_eq!(1, document.cards.len());
        assert_eq!(
            CardDirectives {
                token: Some("4jkya9q3m8z0tw5v9y2b4n6d8f".into()),
                reveal: Some(Reveal::Flip),
                reveal_line: Some(32),
                input: Some(Input::Type),
                direction: Some(Direction::Reverse),
                img: Some("moon.png".into()),
                img_back: Some("phase.png".into()),
                at: Some("29.rs".into()),
                at_origin: Some("src/caching.rs:46-66".into()),
                origin: Some("/crate".into()),
                givens: vec![
                    "state - the parser position".into(),
                    "partial - the card".into(),
                ],
                math: Some("latex".into()),
            },
            document.cards[0].directives
        );
        // Every key above is known or reserved: nothing may lint.
        assert!(document.lints.is_empty(), "{:?}", document.lints);

        // And the built card carries the directive set through.
        let deck = parse_l1("deck.md", text).unwrap();
        let card = &deck.cards[0];
        assert_eq!("The question", card.front);
        assert_eq!(vec!["the answer"], card.back);
        assert_eq!(Some(Reveal::Flip), card.reveal);
        assert_eq!(Some(Input::Type), card.input);
        assert_eq!(Some(Direction::Reverse), card.direction);
        assert_eq!(Some(PathBuf::from("moon.png")), card.image);
        assert_eq!(Some(PathBuf::from("phase.png")), card.image_back);
        assert_eq!(Some("29.rs".to_string()), card.at);
        assert_eq!(Some("src/caching.rs:46-66".to_string()), card.at_origin);
        assert_eq!(Some("/crate".to_string()), card.origin);
        assert_eq!(2, card.givens.len());
        assert_eq!(Some("4jkya9q3m8z0tw5v9y2b4n6d8f"), card.token.as_deref());
    }

    // ── Canonical content ──

    #[test]
    fn canonical_content_collapses_prose_but_not_fences() {
        let back: Vec<String> = ["a  b", "```rust", "let  x = 1;", "```", "c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            "The front\na b\n```rust\nlet  x = 1;\n```\nc",
            canonical_content("The  front", &back)
        );
    }

    #[test]
    fn content_fingerprint_is_whitespace_insensitive_but_word_sensitive() {
        let spaced = content_fingerprint("f", &["a  b".to_string()]);
        let tabbed = content_fingerprint("f", &["a\tb".to_string()]);
        let split = content_fingerprint("f", &["a".to_string(), "b".to_string()]);
        let reworded = content_fingerprint("f", &["a c".to_string()]);
        assert_eq!(spaced, tabbed);
        assert_eq!(spaced, split);
        assert_ne!(spaced, reworded);
    }
}
