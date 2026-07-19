use std::path::PathBuf;

use yaml_rust2::{Yaml, YamlLoader};

use super::{LineSpan, Lint, LintKind, ParseError, WHITESPACE, trim_ws};
use crate::{answer::Input, card::Direction, depth::Reveal, session::Order, token};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub id: Option<String>,
    pub source: Vec<String>,
    pub requires: Vec<String>,
    pub link: Vec<String>,
    pub trace: Option<String>,
    pub reveal: Option<Reveal>,
    pub order: Option<Order>,
    pub input: Option<Input>,
    pub direction: Option<Direction>,
    pub img_dir: Option<PathBuf>,
    pub origin: Option<String>,
    pub unspliceable: bool,
}

// Leading indentation doesn't match: a `---` inside a YAML block scalar can't
// accidentally close the frontmatter.
fn closes_frontmatter(line: &str) -> bool {
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
                    if !token::is_valid(s) {
                        return Err(ParseError::InvalidToken {
                            line,
                            token: s.clone(),
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
pub(super) fn parse_reveal(value: &str) -> Option<Reveal> {
    Reveal::parse(value).filter(|reveal| *reveal != Reveal::Cloze)
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
