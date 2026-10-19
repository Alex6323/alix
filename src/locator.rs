use thiserror::Error;

const OPEN: &str = "<!-- at: ";
const CLOSE: &str = " -->";
const FINGERPRINT: &str = " fingerprint:";
const ASSET: &str = " asset:";
const TERMINATORS: [&str; 3] = [FINGERPRINT, ASSET, CLOSE];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locator {
    pub key: String,
    pub target: Target,
}

impl std::fmt::Display for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key)?;
        let Target::File { path, range } = &self.target else {
            return Ok(());
        };
        write!(f, ":{}", quote_if_needed(path))?;
        match range {
            Some((start, end)) => write!(f, ":{}", format_range(*start, *end)),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    File {
        path: String,
        range: Option<(u32, u32)>,
    },
    Passage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatorFields {
    pub locator: Locator,
    pub fingerprint: Option<String>,
    pub asset: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum LocatorError {
    #[error("`{text}` {problem}")]
    Spelling { text: String, problem: &'static str },
    #[error("key `{0}` must match `[a-z][a-z0-9_-]*` and not be `true`, `false` or `null`")]
    Key(String),
    #[error("path `{path}` {problem}")]
    Path { path: String, problem: &'static str },
    #[error(
        "range `{0}` must be `N` or `N-M` in decimal digits without leading zeros, `N` at least 1 and not greater than `M`"
    )]
    Range(String),
    #[error("path `{path}` {problem}")]
    PathShape { path: String, problem: &'static str },
}

fn spelling(text: &str, problem: &'static str) -> LocatorError {
    LocatorError::Spelling {
        text: text.to_string(),
        problem,
    }
}

pub fn parse_locator_comment(comment: &str) -> Result<LocatorFields, LocatorError> {
    let inner = comment
        .strip_prefix(OPEN)
        .and_then(|rest| rest.strip_suffix(CLOSE))
        .ok_or_else(|| spelling(comment, "is not spelled `<!-- at: <value> -->`"))?;
    let (value, tail) = split_value(inner);
    let locator = parse_value(value)?;
    let (fingerprint, tail) = take_field(tail, FINGERPRINT)?;
    let (asset, tail) = take_field(tail, ASSET)?;
    if !tail.is_empty() {
        return Err(spelling(
            tail,
            "is not a locator field; after the value come ` fingerprint: <hex>`, then ` asset: <object>`, then ` -->`",
        ));
    }
    Ok(LocatorFields {
        locator,
        fingerprint,
        asset,
    })
}

fn split_value(inner: &str) -> (&str, &str) {
    let mut index = 0;
    let mut key_done = false;
    while index < inner.len() {
        let rest = &inner[index..];
        if TERMINATORS
            .iter()
            .any(|terminator| rest.starts_with(terminator))
        {
            break;
        }
        if !key_done && rest.starts_with(":\"") {
            key_done = true;
            index += rest[2..].find('"').map_or(rest.len(), |close| 3 + close);
            continue;
        }
        key_done |= rest.starts_with(':');
        index += rest.chars().next().map_or(1, char::len_utf8);
    }
    inner.split_at(index)
}

fn parse_value(value: &str) -> Result<Locator, LocatorError> {
    if value.is_empty() {
        return Err(spelling("at:", "has no value"));
    }
    if value.starts_with(' ') || value.ends_with(' ') {
        return Err(spelling(value, "has a double space"));
    }
    let Some((key, rest)) = value.split_once(':') else {
        return Ok(Locator {
            key: parse_key(value)?,
            target: Target::Passage,
        });
    };
    Ok(Locator {
        key: parse_key(key)?,
        target: parse_file_target(rest)?,
    })
}

fn parse_file_target(rest: &str) -> Result<Target, LocatorError> {
    let (path, range) = match rest.strip_prefix('"') {
        Some(quoted) => split_quoted(quoted, rest)?,
        None => split_unquoted(rest)?,
    };
    check_path(path)?;
    let range = range.map(parse_range).transpose()?;
    Ok(Target::File {
        path: path.to_string(),
        range,
    })
}

fn split_quoted<'a>(
    quoted: &'a str,
    rest: &str,
) -> Result<(&'a str, Option<&'a str>), LocatorError> {
    let Some((path, after)) = quoted.split_once('"') else {
        return Err(spelling(rest, "has a quoted path with no closing quote"));
    };
    if after.is_empty() {
        return Ok((path, None));
    }
    match after.strip_prefix(':') {
        Some(tail) if is_range_shaped(tail) => Ok((path, Some(tail))),
        _ => Err(spelling(
            after,
            "follows the closing quote; only `:<range>` may",
        )),
    }
}

fn split_unquoted(rest: &str) -> Result<(&str, Option<&str>), LocatorError> {
    let (path, range) = match rest.rsplit_once(':') {
        Some((path, tail)) if is_range_shaped(tail) => (path, Some(tail)),
        _ => (rest, None),
    };
    if path.contains(' ') {
        return Err(path_error(path, "contains a space and is not quoted"));
    }
    Ok((path, range))
}

pub fn parse_key(text: &str) -> Result<String, LocatorError> {
    let mut characters = text.chars();
    let well_formed = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        });
    if well_formed && !matches!(text, "true" | "false" | "null") {
        Ok(text.to_string())
    } else {
        Err(LocatorError::Key(text.to_string()))
    }
}

fn path_error(path: &str, problem: &'static str) -> LocatorError {
    LocatorError::Path {
        path: path.to_string(),
        problem,
    }
}

fn check_path(path: &str) -> Result<(), LocatorError> {
    if let Some(problem) = path_problem(path) {
        return Err(path_error(path, problem));
    }
    match path_shape_problem(path) {
        Some(problem) => Err(LocatorError::PathShape {
            path: path.to_string(),
            problem,
        }),
        None => Ok(()),
    }
}

fn path_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        Some("is empty")
    } else if path.contains('"') {
        Some("contains a `\"`")
    } else if path.contains("-->") {
        Some("contains `-->`")
    } else if path
        .chars()
        .any(|character| character < ' ' || character == '\u{7f}')
    {
        Some("contains a control character")
    } else {
        None
    }
}

fn path_shape_problem(path: &str) -> Option<&'static str> {
    if path.contains('\\') {
        return Some("contains `\\`");
    }
    if matches!(path.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic()) {
        return Some("starts with a drive letter");
    }
    path.split('/').find_map(|segment| match segment {
        "" => Some("has an empty segment: a leading, trailing or doubled `/`"),
        "." => Some("has a `.` segment"),
        ".." => Some("has a `..` segment"),
        _ => None,
    })
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_range_shaped(text: &str) -> bool {
    match text.split_once('-') {
        Some((start, end)) => is_digits(start) && is_digits(end),
        None => is_digits(text),
    }
}

fn parse_range(text: &str) -> Result<(u32, u32), LocatorError> {
    let number = |digits: &str| -> Option<u32> {
        if digits.starts_with('0') || !is_digits(digits) {
            return None;
        }
        digits.parse().ok()
    };
    let bounds = match text.split_once('-') {
        Some((start, end)) => number(start).zip(number(end)),
        None => number(text).map(|line| (line, line)),
    };
    match bounds {
        Some((start, end)) if start <= end => Ok((start, end)),
        _ => Err(LocatorError::Range(text.to_string())),
    }
}

fn take_field<'a>(
    tail: &'a str,
    field: &'static str,
) -> Result<(Option<String>, &'a str), LocatorError> {
    let Some(rest) = tail.strip_prefix(field) else {
        return Ok((None, tail));
    };
    let name = field.trim_start();
    let rest = rest
        .strip_prefix(' ')
        .ok_or_else(|| spelling(name, "must be followed by one space and a value"))?;
    if rest.starts_with(' ') {
        return Err(spelling(name, "is followed by a double space"));
    }
    let (value, tail) = rest.split_at(rest.find(' ').unwrap_or(rest.len()));
    if value.is_empty() {
        return Err(spelling(name, "has no value"));
    }
    Ok((Some(value.to_string()), tail))
}

pub fn format_locator(locator: &Locator) -> Result<String, LocatorError> {
    let key = parse_key(&locator.key)?;
    let Target::File { path, range } = &locator.target else {
        return Ok(key);
    };
    check_path(path)?;
    if let Some((start, end)) = range
        && (*start < 1 || start > end)
    {
        return Err(LocatorError::Range(format!("{start}-{end}")));
    }
    Ok(locator.to_string())
}

pub(crate) fn quote_if_needed(path: &str) -> String {
    let range_tail = path
        .rsplit_once(':')
        .is_some_and(|(_, tail)| is_range_shaped(tail));
    if path.contains(' ') || range_tail {
        format!("\"{path}\"")
    } else {
        path.to_string()
    }
}

pub(crate) fn format_range(start: u32, end: u32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

pub fn format_locator_comment(fields: &LocatorFields) -> Result<String, LocatorError> {
    let mut comment = format!("{OPEN}{}", format_locator(&fields.locator)?);
    for (field, value) in [(FINGERPRINT, &fields.fingerprint), (ASSET, &fields.asset)] {
        let Some(value) = value else {
            continue;
        };
        if value.is_empty() || value.contains(' ') || value.contains("-->") {
            return Err(spelling(value, "is not a single space-free field value"));
        }
        comment.push_str(field);
        comment.push(' ');
        comment.push_str(value);
    }
    comment.push_str(CLOSE);
    Ok(comment)
}

#[cfg(test)]
impl LocatorError {
    fn rule(&self) -> u8 {
        match self {
            Self::Key(_) => 3,
            Self::Spelling { .. } => 7,
            Self::Path { .. } => 8,
            Self::Range(_) => 9,
            Self::PathShape { .. } => 10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value: &str) -> String {
        format!("<!-- at: {value} -->")
    }

    fn file(key: &str, path: &str, range: Option<(u32, u32)>) -> Locator {
        Locator {
            key: key.into(),
            target: Target::File {
                path: path.into(),
                range,
            },
        }
    }

    fn passage(key: &str) -> Locator {
        Locator {
            key: key.into(),
            target: Target::Passage,
        }
    }

    fn fields(locator: Locator, fingerprint: Option<&str>, asset: Option<&str>) -> LocatorFields {
        LocatorFields {
            locator,
            fingerprint: fingerprint.map(str::to_string),
            asset: asset.map(str::to_string),
        }
    }

    fn bare(locator: Locator) -> Result<LocatorFields, u8> {
        Ok(fields(locator, None, None))
    }

    fn frozen(
        locator: Locator,
        fingerprint: Option<&str>,
        asset: Option<&str>,
    ) -> Result<LocatorFields, u8> {
        Ok(fields(locator, fingerprint, asset))
    }

    fn table() -> Vec<(String, Result<LocatorFields, u8>)> {
        vec![
            // rule 7 shapes and rule 9 ranges
            (at("code:x.txt"), bare(file("code", "x.txt", None))),
            (
                at("code:x.txt:3"),
                bare(file("code", "x.txt", Some((3, 3)))),
            ),
            (
                at("code:x.txt:3-5"),
                bare(file("code", "x.txt", Some((3, 5)))),
            ),
            (
                at("code:x.txt:3-3"),
                bare(file("code", "x.txt", Some((3, 3)))),
            ),
            (at("code:x.txt:03"), Err(9)),
            (at("code:x.txt:0"), Err(9)),
            (at("code:x.txt:5-3"), Err(9)),
            (at("code:x.txt:1-02"), Err(9)),
            (at("code:x.txt:4294967296"), Err(9)),
            (at("code:x.txt:3-"), bare(file("code", "x.txt:3-", None))),
            (at("code:ab:1:2"), bare(file("code", "ab:1", Some((2, 2))))),
            (at("code:1-2"), bare(file("code", "1-2", None))),
            (at("code"), bare(passage("code"))),
            (at("code_1-x"), bare(passage("code_1-x"))),
            // rule 7 and 8 quoting
            (at("code:\"a b.txt\""), bare(file("code", "a b.txt", None))),
            (
                at("code:\"a b.txt\":3"),
                bare(file("code", "a b.txt", Some((3, 3)))),
            ),
            (at("code:\"x.txt\""), bare(file("code", "x.txt", None))),
            (at("code:\"ab:c\""), bare(file("code", "ab:c", None))),
            (
                at("code:\"x.txt:5-3\""),
                bare(file("code", "x.txt:5-3", None)),
            ),
            (
                at("code:\"x asset: y\""),
                bare(file("code", "x asset: y", None)),
            ),
            (
                at("code:\"ü ö.txt\":2"),
                bare(file("code", "ü ö.txt", Some((2, 2)))),
            ),
            (at("code:é.txt"), bare(file("code", "é.txt", None))),
            (at("code:\"a\"b"), Err(7)),
            (at("code:\"a"), Err(7)),
            (at("code:\"a\":3-"), Err(7)),
            (at("code:\"a\":"), Err(7)),
            (at("code:a\"b"), Err(8)),
            (at("code:\"\""), Err(8)),
            (at("code:"), Err(8)),
            (at("code:a b.txt"), Err(8)),
            (at("code:a\tb"), Err(8)),
            (at("code:\"a\u{7f}b\""), Err(8)),
            (at("code:a-->b"), Err(8)),
            (at("code:\"a --> b\""), Err(8)),
            // rule 3 keys
            (at(":x.txt"), Err(3)),
            (at("Code:x.txt"), Err(3)),
            (at("true:x.txt"), Err(3)),
            (at("false:x.txt"), Err(3)),
            (at("null:x.txt"), Err(3)),
            (at("1code:x.txt"), Err(3)),
            (at("co de:x.txt"), Err(3)),
            (at("Code"), Err(3)),
            // rule 10 path shape
            (at("code:a/c:x.txt"), bare(file("code", "a/c:x.txt", None))),
            (at("code:c:x.txt"), Err(10)),
            (at("code:x:1:2"), Err(10)),
            (at("code:\"a:b\""), Err(10)),
            (at("code:/x"), Err(10)),
            (at("code:./x"), Err(10)),
            (at("code:a//b"), Err(10)),
            (at("code:a/"), Err(10)),
            (at("code:a/../b"), Err(10)),
            (at("code:.."), Err(10)),
            (at("code:a\\b"), Err(10)),
            // rule 7 fields
            (
                at("code:x.txt:3 fingerprint: xxh64-0123456789abcdef"),
                frozen(
                    file("code", "x.txt", Some((3, 3))),
                    Some("xxh64-0123456789abcdef"),
                    None,
                ),
            ),
            (
                at("code:x.txt:3 fingerprint: xxh64-0123456789abcdef asset: sha256-abc.txt"),
                frozen(
                    file("code", "x.txt", Some((3, 3))),
                    Some("xxh64-0123456789abcdef"),
                    Some("sha256-abc.txt"),
                ),
            ),
            (
                at("code:x.txt asset: sha256-abc.txt"),
                frozen(file("code", "x.txt", None), None, Some("sha256-abc.txt")),
            ),
            (
                at("guide asset: sha256-abc.txt"),
                frozen(passage("guide"), None, Some("sha256-abc.txt")),
            ),
            (
                at("code:\"a fingerprint: b.txt\":3 fingerprint: xxh64-1"),
                frozen(
                    file("code", "a fingerprint: b.txt", Some((3, 3))),
                    Some("xxh64-1"),
                    None,
                ),
            ),
            (
                at("code:x.txt asset: sha256-abc.txt fingerprint: xxh64-1"),
                Err(7),
            ),
            (at("code:x.txt fingerprint:"), Err(7)),
            (at("code:x.txt fingerprint:xxh64-1"), Err(7)),
            (at("code:x.txt fingerprint: a fingerprint: b"), Err(7)),
            (at("code:x.txt flavor: cherry"), Err(8)),
            // rule 7 spacing
            (at(" code:x.txt"), Err(7)),
            (at("code:x.txt "), Err(7)),
            (at("code:\"a b\" "), Err(7)),
            (at("code:x.txt  fingerprint: xxh64-1"), Err(7)),
            (at("code:x.txt fingerprint:  xxh64-1"), Err(7)),
            (at("code:x --> extra"), Err(7)),
            ("<!--  at: code:x.txt -->".into(), Err(7)),
            ("<!--at: code:x.txt -->".into(), Err(7)),
            ("<!-- at: code:x.txt-->".into(), Err(7)),
            ("<!-- at:code:x.txt -->".into(), Err(7)),
            ("<!-- At: code:x.txt -->".into(), Err(7)),
            ("<!-- at: -->".into(), Err(7)),
            ("<!-- at:  -->".into(), Err(7)),
        ]
    }

    #[test]
    fn every_locator_example_in_adr_0053_rules_3_and_7_to_10_parses_or_fails_as_the_table_says() {
        for (comment, expected) in table() {
            let actual = parse_locator_comment(&comment);
            let outcome = actual.clone().map_err(|error| error.rule());
            assert_eq!(expected, outcome, "row `{comment}` gave {actual:?}");
        }
    }

    #[test]
    fn formatting_then_parsing_any_valid_locator_returns_it_unchanged() {
        let valid = table()
            .into_iter()
            .filter_map(|(comment, expected)| expected.ok().map(|fields| (comment, fields)));
        for (comment, fields) in valid {
            let formatted = format_locator_comment(&fields).unwrap();
            assert_eq!(
                Ok(fields.clone()),
                parse_locator_comment(&formatted),
                "row `{comment}` formatted as `{formatted}`"
            );
        }
        let awkward = [
            "a b.txt",
            "x.txt:12",
            "x.txt:1-2",
            "x.txt:3-",
            "x.txt:03",
            "x.txt:",
            "ab:c",
            "a/b:c",
            "1-2",
            "a b:3",
            " a",
            "ü ö.txt",
            "a fingerprint: b",
            "x asset: y",
        ];
        for path in awkward {
            for range in [None, Some((3, 3)), Some((3, 5))] {
                let fields = fields(file("code", path, range), None, None);
                let formatted = format_locator_comment(&fields).unwrap();
                assert_eq!(
                    Ok(fields.clone()),
                    parse_locator_comment(&formatted),
                    "path `{path}` with {range:?} formatted as `{formatted}`"
                );
            }
        }
    }

    #[test]
    fn alix_writers_quote_a_path_only_when_it_contains_a_space_or_ends_in_a_range_shaped_tail() {
        let cases = [
            ("x.txt", None, "code:x.txt"),
            ("a/b:c", None, "code:a/b:c"),
            ("ab:c", None, "code:ab:c"),
            ("x.txt:3-", None, "code:x.txt:3-"),
            ("x.txt:", None, "code:x.txt:"),
            ("x.txt", Some((3, 3)), "code:x.txt:3"),
            ("x.txt", Some((3, 5)), "code:x.txt:3-5"),
            ("a b.txt", None, "code:\"a b.txt\""),
            ("a b.txt", Some((3, 5)), "code:\"a b.txt\":3-5"),
            ("x.txt:12", None, "code:\"x.txt:12\""),
            ("x.txt:1-2", Some((3, 3)), "code:\"x.txt:1-2\":3"),
            ("x.txt:03", None, "code:\"x.txt:03\""),
        ];
        for (path, range, expected) in cases {
            assert_eq!(
                Ok(expected.to_string()),
                format_locator(&file("code", path, range)),
                "path `{path}` with {range:?}"
            );
        }
        assert_eq!(Ok("guide".to_string()), format_locator(&passage("guide")));
        assert_eq!(
            Ok("<!-- at: guide fingerprint: xxh64-1 asset: sha256-a.txt -->".to_string()),
            format_locator_comment(&fields(
                passage("guide"),
                Some("xxh64-1"),
                Some("sha256-a.txt")
            ))
        );
    }

    #[test]
    fn the_writer_refuses_every_locator_the_reader_would_reject() {
        let rejected = [
            (file("code", "a\"b", None), 8),
            (file("code", "", None), 8),
            (file("code", "a-->b", None), 8),
            (file("code", "a\tb", None), 8),
            (file("code", "a/../b", None), 10),
            (file("code", "c:x", None), 10),
            (file("True", "x", None), 3),
            (passage("null"), 3),
            (file("code", "x", Some((5, 3))), 9),
            (file("code", "x", Some((0, 1))), 9),
        ];
        for (locator, rule) in rejected {
            let error = format_locator(&locator).unwrap_err();
            assert_eq!(rule, error.rule(), "{locator:?} gave {error}");
        }
        for value in ["", "xxh64 1", "a-->b"] {
            let error =
                format_locator_comment(&fields(passage("guide"), Some(value), None)).unwrap_err();
            assert_eq!(7, error.rule(), "fingerprint `{value}` gave {error}");
        }
    }
}
