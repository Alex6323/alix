use yaml_rust2::{Yaml, YamlLoader};

use super::{LineSpan, Lint, LintKind, ParseError, WHITESPACE, trim_ws};
use crate::{answer::Input, card::Direction, depth::Reveal, session::Order, token};

/// Pinned at 1 pre-1.0: a break rewrites old decks outside the repository
/// rather than bumping. Read to refuse a foreign document, never to adapt
/// to one.
pub const DECK_FORMAT_VERSION: u32 = 1;
/// One spelling for the personal file's link to its deck, shared by the
/// parser, the header writer, and every message that names it.
pub const PERSONAL_PARENT_KEY: &str = "for";

/// A named content mapping: what a bare shape becomes when invoked, and
/// the `plain` escape that keeps the shape literal under a deck default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapping {
    Plain,
    ChoicesSingle,
    ChoicesMultiple,
    Cards,
}

/// The kind of block a trailing invocation is allowed to bind, tracked as
/// the scanner passes it rather than rediscovered by reading upward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MappableBlock {
    Checklist,
    Divider,
}

impl Mapping {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "plain" => Some(Self::Plain),
            "choices-single" => Some(Self::ChoicesSingle),
            "choices-multiple" => Some(Self::ChoicesMultiple),
            "cards" => Some(Self::Cards),
            _ => None,
        }
    }

    pub(crate) fn binds(self, block: Option<MappableBlock>) -> bool {
        matches!(
            (self, block),
            (Self::Plain, Some(_))
                | (
                    Self::ChoicesSingle | Self::ChoicesMultiple,
                    Some(MappableBlock::Checklist)
                )
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub id: Option<String>,
    pub authors: Vec<String>,
    pub license: Option<String>,
    pub created_at: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub source: Vec<String>,
    pub requires: Vec<String>,
    pub link: Vec<String>,
    pub trace: Option<String>,
    pub reveal: Option<Reveal>,
    pub order: Option<Order>,
    pub input: Option<Input>,
    pub direction: Option<Direction>,
    pub sampling: Option<bool>,
    pub tasklist: Option<Mapping>,
    pub table: Option<Mapping>,
    pub unspliceable: bool,
    pub personal_for: Option<String>,
}

// Leading indentation doesn't match: a `---` inside a YAML block scalar can't
// accidentally close the frontmatter.
pub(super) fn closes_frontmatter(line: &str) -> bool {
    line.strip_prefix("---")
        .is_some_and(|rest| rest.chars().all(|c| WHITESPACE.contains(&c)))
}

pub(super) fn parse_frontmatter(
    lines: &[&str],
    lints: &mut Vec<Lint>,
) -> Result<(Frontmatter, usize, Option<LineSpan>), ParseError> {
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
        return Err(ParseError::UnclosedFrontmatter(open + 1));
    };
    let frontmatter = load_frontmatter(&lines[open + 1..close], open + 2, lints)?;
    Ok((frontmatter, close + 1, Some((open + 1, close + 1))))
}

fn load_frontmatter(
    block: &[&str],
    first_line: usize,
    lints: &mut Vec<Lint>,
) -> Result<Frontmatter, ParseError> {
    let mut frontmatter = Frontmatter::default();
    let text = block.join("\n");
    if trim_ws(&text).is_empty() {
        return Ok(frontmatter);
    }
    let docs = YamlLoader::load_from_str(&text).map_err(|e| ParseError::FrontmatterSyntax {
        line: first_line + e.marker().line().saturating_sub(1),
        message: e.info().to_string(),
    })?;
    if docs.len() > 1 {
        return Err(ParseError::FrontmatterSyntax {
            line: first_line,
            message: "the block holds more than one yaml document".into(),
        });
    }
    let Some(root) = docs.into_iter().next() else {
        return Ok(frontmatter);
    };
    // A null-scalar root loads but is not a block mapping; splicing an `id:`
    // in front of a bare scalar would fail (yaml-rust2: "simple key expected").
    if root == Yaml::Null {
        frontmatter.unspliceable = true;
        return Ok(frontmatter);
    }
    let Yaml::Hash(mapping) = root else {
        frontmatter.unspliceable = true;
        return Ok(frontmatter);
    };
    // A flow mapping loads but offers no per-key line to splice a minted
    // `id:` into.
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
                    if !matches!(token::parse_id(s), Some((token::Kind::Deck, ..))) {
                        return Err(ParseError::InvalidDeckId {
                            line,
                            value: s.clone(),
                        });
                    }
                    frontmatter.id = Some(s.clone());
                }
                other => {
                    return Err(ParseError::NonStringId {
                        line,
                        found: yaml_kind(other),
                    });
                }
            },
            "format-version" => match value {
                Yaml::Integer(n) if *n == i64::from(DECK_FORMAT_VERSION) => {}
                Yaml::Integer(n) => {
                    return Err(ParseError::UnsupportedDeckVersion { line, version: *n });
                }
                other => {
                    return Err(ParseError::NonIntegerVersion {
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
            "sampling" => match value.as_str().and_then(parse_sampling) {
                Some(sampling) => frontmatter.sampling = Some(sampling),
                None => lints.push(bad_value(line, key, describe(value))),
            },
            "tasklist" => match value.as_str().and_then(Mapping::parse) {
                Some(m @ (Mapping::ChoicesSingle | Mapping::ChoicesMultiple)) => {
                    frontmatter.tasklist = Some(m);
                }
                _ => lints.push(bad_value(line, key, describe(value))),
            },
            "table" => match value.as_str().and_then(Mapping::parse) {
                Some(Mapping::Cards) => frontmatter.table = Some(Mapping::Cards),
                _ => lints.push(bad_value(line, key, describe(value))),
            },
            "authors" => frontmatter.authors = string_list(key, value, line, lints),
            // The deck's display name: trimmed, non-empty, single line. An
            // empty or multi-line value would become a blank or a reflowing
            // picker row, so it is refused rather than shown.
            "title" => match value {
                Yaml::String(s) if !trim_ws(s).is_empty() && !s.contains('\n') => {
                    frontmatter.title = Some(trim_ws(s).to_string());
                }
                Yaml::String(s) => {
                    return Err(ParseError::InvalidTitle {
                        line,
                        value: s.clone(),
                    });
                }
                other => lints.push(bad_value(line, key, yaml_kind(other).to_string())),
            },
            "description" => match value {
                Yaml::String(s) => frontmatter.description = Some(s.clone()),
                other => lints.push(bad_value(line, key, yaml_kind(other).to_string())),
            },
            "license" => match value {
                Yaml::String(s) => frontmatter.license = Some(s.clone()),
                other => lints.push(bad_value(line, key, yaml_kind(other).to_string())),
            },
            PERSONAL_PARENT_KEY => match value {
                Yaml::String(s) => frontmatter.personal_for = Some(s.clone()),
                other => lints.push(bad_value(line, key, yaml_kind(other).to_string())),
            },
            "created-at" => match value {
                Yaml::String(s) => frontmatter.created_at = Some(s.clone()),
                other => lints.push(bad_value(line, key, yaml_kind(other).to_string())),
            },
            // Reserved for future deck metadata: ignored without a lint.
            "language" | "revision" => {}
            _ => lints.push(Lint {
                line,
                kind: LintKind::UnknownKey { key: key.clone() },
            }),
        }
    }
    Ok(frontmatter)
}

pub(super) fn parse_reveal(value: &str) -> Option<Reveal> {
    Reveal::parse(value)
}

/// `on`/`off` only: the key governs automatic sampling, and a future mode
/// would be a new value, never a truthy spelling.
pub fn parse_sampling(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

fn describe(value: &Yaml) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| yaml_kind(value).to_string())
}

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

pub(super) fn bad_value(line: usize, key: &str, value: String) -> Lint {
    Lint {
        line,
        kind: LintKind::BadValue {
            key: key.to_string(),
            value,
        },
    }
}

pub fn yaml_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// What `reorder_frontmatter` did with a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reorder {
    Unchanged,
    Reordered(String),
    Skipped(&'static str),
}

/// The canonical order for frontmatter alix itself writes and for the opt-in
/// doctor repair: authored keys first, machine lines last. An author's own
/// order is never diagnosed against it.
const CANONICAL_KEY_ORDER: [&str; 19] = [
    "title",
    "description",
    "trace",
    "authors",
    "license",
    "created-at",
    "language",
    "revision",
    "reveal",
    "order",
    "input",
    "direction",
    "sampling",
    "tasklist",
    "table",
    "source",
    "requires",
    "link",
    "format-version",
];

const UNKNOWN_KEY_RANK: usize = CANONICAL_KEY_ORDER.len();

fn key_rank(key: &str) -> usize {
    match key {
        PERSONAL_PARENT_KEY => UNKNOWN_KEY_RANK + 1,
        "id" => UNKNOWN_KEY_RANK + 2,
        _ => CANONICAL_KEY_ORDER
            .iter()
            .position(|k| *k == key)
            .unwrap_or(UNKNOWN_KEY_RANK),
    }
}

fn line_content(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

fn is_key_line(content: &str) -> Option<&str> {
    let colon = content.find(':')?;
    let key = &content[..colon];
    (!key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then_some(key)
}

pub fn reorder_frontmatter(text: &str) -> Reorder {
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let Some(open) = lines
        .iter()
        .position(|line| !trim_ws(line_content(line)).is_empty())
    else {
        return Reorder::Unchanged;
    };
    if line_content(lines[open]) != "---" {
        return Reorder::Unchanged;
    }
    let Some(close) = lines[open + 1..]
        .iter()
        .position(|line| closes_frontmatter(line_content(line)))
        .map(|i| open + 1 + i)
    else {
        return Reorder::Skipped("unclosed frontmatter");
    };

    let mut blocks: Vec<(usize, Vec<&str>)> = Vec::new();
    for line in &lines[open + 1..close] {
        let content = line_content(line);
        if trim_ws(content).is_empty() {
            return Reorder::Skipped("a blank line inside the frontmatter");
        }
        if content.starts_with('#') {
            return Reorder::Skipped("a comment inside the frontmatter");
        }
        let continuation =
            content.starts_with([' ', '\t']) || content == "-" || content.starts_with("- ");
        if continuation {
            let Some((_, block)) = blocks.last_mut() else {
                return Reorder::Skipped("a continuation line without a key");
            };
            block.push(line);
            continue;
        }
        let Some(key) = is_key_line(content) else {
            return Reorder::Skipped("an unrecognized frontmatter line");
        };
        blocks.push((key_rank(key), vec![line]));
    }

    blocks.sort_by_key(|(rank, _)| *rank);
    let mut out = String::with_capacity(text.len());
    out.push_str(bom);
    out.extend(lines[..=open].iter().copied());
    for (_, block) in &blocks {
        out.extend(block.iter().copied());
    }
    out.extend(lines[close..].iter().copied());
    if out == text {
        Reorder::Unchanged
    } else {
        Reorder::Reordered(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reordered(text: &str) -> String {
        match reorder_frontmatter(text) {
            Reorder::Reordered(out) => out,
            other => panic!("expected Reordered, got {other:?}"),
        }
    }

    #[test]
    fn the_machine_id_line_moves_last_and_title_first() {
        assert_eq!(
            "---\ntitle: T\ndescription: D\nid: \"deck-x\"\n---\n## q\na\n",
            reordered("---\nid: \"deck-x\"\ntitle: T\ndescription: D\n---\n## q\na\n")
        );
    }

    #[test]
    fn a_key_line_needs_a_nonempty_ascii_word_key_before_the_colon() {
        for (line, key) in [
            ("some-key: v", Some("some-key")),
            ("under_key: v", Some("under_key")),
            ("k9: v", Some("k9")),
            (": v", None),
            ("a b: v", None),
            ("käse: v", None),
            ("no colon here", None),
        ] {
            assert_eq!(key, is_key_line(line), "{line:?}");
        }
    }

    #[test]
    fn the_whole_canonical_order_is_pinned_as_one_law() {
        let mut every_key = vec![
            "id",
            "for",
            "format-version",
            "link",
            "requires",
            "source",
            "table",
            "tasklist",
            "sampling",
            "direction",
            "input",
            "order",
            "reveal",
            "revision",
            "language",
            "created-at",
            "license",
            "authors",
            "trace",
            "description",
            "title",
        ];
        every_key.sort_by_key(|key| key_rank(key));
        assert_eq!(
            vec![
                "title",
                "description",
                "trace",
                "authors",
                "license",
                "created-at",
                "language",
                "revision",
                "reveal",
                "order",
                "input",
                "direction",
                "sampling",
                "tasklist",
                "table",
                "source",
                "requires",
                "link",
                "format-version",
                "for",
                "id",
            ],
            every_key
        );
        assert!(key_rank("some-unknown") > key_rank("format-version"));
        assert!(key_rank("some-unknown") < key_rank("for"));
    }

    #[test]
    fn unknown_keys_keep_their_relative_order_between_authored_and_machine() {
        assert_eq!(
            "---\ntitle: T\nzeta: 1\nalpha: 2\nid: \"deck-x\"\n---\n",
            reordered("---\nid: \"deck-x\"\nzeta: 1\ntitle: T\nalpha: 2\n---\n")
        );
    }

    #[test]
    fn a_multi_line_value_travels_with_its_key() {
        assert_eq!(
            "---\ntitle: T\ndescription: >-\n  two\n  lines\nid: \"deck-x\"\n---\n",
            reordered("---\nid: \"deck-x\"\ndescription: >-\n  two\n  lines\ntitle: T\n---\n")
        );
    }

    #[test]
    fn a_column_zero_list_item_travels_with_its_key() {
        assert_eq!(
            "---\ntitle: T\nsource:\n- a\n- b\nid: \"deck-x\"\n---\n",
            reordered("---\nid: \"deck-x\"\nsource:\n- a\n- b\ntitle: T\n---\n")
        );
    }

    #[test]
    fn an_already_canonical_block_reports_unchanged() {
        assert_eq!(
            Reorder::Unchanged,
            reorder_frontmatter("---\ntitle: T\nid: \"deck-x\"\n---\n## q\na\n")
        );
    }

    #[test]
    fn reordering_twice_is_idempotent() {
        let once = reordered("---\nid: \"deck-x\"\ntitle: T\n---\n");
        assert_eq!(Reorder::Unchanged, reorder_frontmatter(&once));
    }

    #[test]
    fn crlf_endings_survive_reordering_byte_for_byte() {
        assert_eq!(
            "---\r\ntitle: T\r\nid: \"deck-x\"\r\n---\r\n## q\r\na\r\n",
            reordered("---\r\nid: \"deck-x\"\r\ntitle: T\r\n---\r\n## q\r\na\r\n")
        );
    }

    #[test]
    fn a_bom_stays_first_through_reordering() {
        assert_eq!(
            "\u{feff}---\ntitle: T\nid: \"deck-x\"\n---\n",
            reordered("\u{feff}---\nid: \"deck-x\"\ntitle: T\n---\n")
        );
    }

    #[test]
    fn a_comment_line_defers_the_repair() {
        assert_eq!(
            Reorder::Skipped("a comment inside the frontmatter"),
            reorder_frontmatter("---\nid: \"deck-x\"\n# mine\ntitle: T\n---\n")
        );
    }

    #[test]
    fn a_blank_line_defers_the_repair() {
        assert_eq!(
            Reorder::Skipped("a blank line inside the frontmatter"),
            reorder_frontmatter("---\nid: \"deck-x\"\n\ntitle: T\n---\n")
        );
    }

    #[test]
    fn a_document_without_frontmatter_reports_unchanged() {
        assert_eq!(Reorder::Unchanged, reorder_frontmatter("## q\na\n"));
    }
}
