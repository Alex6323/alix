use std::{
    cmp::Ordering,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{locator, token};

mod manifest;
mod roots;

pub use manifest::{DeckSources, Manifest, Row, load_verified, stray_copies};
pub use roots::Roots;

pub const ROOT: &str = "sources";
pub const MANIFEST: &str = "manifest.toml";
pub const ROOTS: &str = "manifest.local.toml";
pub const MANIFEST_VERSION: u32 = 1;
pub const ROOTS_VERSION: u32 = 1;

const ID_PREFIX: &str = "source-";

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("sources file version {found} is unknown")]
    Version { found: u32 },
    #[error("`{0}` is not a source id")]
    BadId(String),
    #[error("{}display path `{path}` {problem}", source_prefix(.id))]
    BadPath {
        id: Option<String>,
        path: String,
        problem: &'static str,
    },
    #[error("display path `{0}` is on two rows")]
    DuplicatePath(String),
    #[error("source `{0}` has an empty bound-to")]
    EmptyBinding(String),
    #[error("source `{id}` is bound to `{topic}`, which names no topic of the deck")]
    UnknownTopic { id: String, topic: String },
    #[error("topic id `{0}` is on two topics; nothing binds to it")]
    DuplicateTopicId(String),
    #[error("source copy `{0}` is not a regular file")]
    NotRegularFile(PathBuf),
    #[error("source copy `{found}` should be named `{expected}`")]
    WrongName { expected: String, found: String },
    #[error("source copy `{0}` does not match its id")]
    DigestMismatch(PathBuf),
    #[error("source copy `{0}` is empty")]
    EmptyCopy(PathBuf),
    #[error("the copy of source `{0}` is missing")]
    MissingCopy(String),
    #[error("source `{id}` binding `{entry}` {problem}")]
    BadBinding {
        id: String,
        entry: String,
        problem: &'static str,
    },
    #[error("source `{id}` binding `{entry}` reaches beyond its {lines} lines")]
    RangeOutOfBounds {
        id: String,
        entry: String,
        lines: usize,
    },
    #[error("source `{id}` binds `{topic}` twice")]
    DuplicateTopicEntry { id: String, topic: String },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("{path}: {source}")]
    Serialize {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },
}

fn source_prefix(id: &Option<String>) -> String {
    id.as_ref()
        .map_or_else(String::new, |id| format!("source `{id}`: "))
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> SourceError + '_ {
    move |source| SourceError::Io {
        path: path.to_path_buf(),
        source,
    }
}

pub fn source_id(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    format!("{ID_PREFIX}{}", token::base32_256(&digest))
}

pub fn is_source_id(text: &str) -> bool {
    text.strip_prefix(ID_PREFIX)
        .is_some_and(token::is_base32_256)
}

pub fn is_copy_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(id, extension)| is_source_id(id) && is_extension(extension))
}

fn is_extension(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

pub fn copy_name(id: &str, display: &DisplayPath) -> String {
    let extension = crate::assets::normalized_extension(Path::new(&display.path), true);
    format!("{id}.{extension}")
}

pub fn line_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&byte| byte == b'\n').count()
        + usize::from(bytes.last().is_some_and(|&byte| byte != b'\n'))
}

pub fn topic_fingerprint(bindings: &[&str]) -> String {
    let mut sorted: Vec<&str> = bindings.to_vec();
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    for binding in sorted {
        hasher.update(binding.as_bytes());
        hasher.update(b"\n");
    }
    let digest: [u8; 32] = hasher.finalize().into();
    token::base32_256(&digest)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayPath {
    pub key: String,
    pub path: String,
}

impl DisplayPath {
    fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.key
            .bytes()
            .chain(std::iter::once(b':'))
            .chain(self.path.bytes())
    }
}

impl Ord for DisplayPath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.bytes().cmp(other.bytes())
    }
}

impl PartialOrd for DisplayPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for DisplayPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.key, self.path)
    }
}

impl FromStr for DisplayPath {
    type Err = SourceError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = |problem| SourceError::BadPath {
            id: None,
            path: text.to_string(),
            problem,
        };
        let Some((key, path)) = text.split_once(':') else {
            return Err(bad("has no key"));
        };
        let key = locator::parse_key(key).map_err(|_| bad("has a malformed key"))?;
        if let Some(problem) = path_problem(path) {
            return Err(bad(problem));
        }
        Ok(Self {
            key,
            path: path.to_string(),
        })
    }
}

fn path_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("has an empty path");
    }
    if path
        .chars()
        .any(|character| character < ' ' || character == '\u{7f}')
    {
        return Some("contains a control character");
    }
    if path.contains('\\') {
        return Some("contains a backslash");
    }
    if matches!(path.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic()) {
        return Some("starts with a drive letter");
    }
    path.split('/').find_map(|component| match component {
        "" => Some("has an empty component"),
        "." => Some("has a `.` component"),
        ".." => Some("has a `..` component"),
        _ => None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub topic: String,
    pub ranges: Vec<(u32, u32)>,
}

impl Binding {
    pub fn render(&self) -> String {
        if self.ranges.is_empty() {
            self.topic.clone()
        } else {
            format!("{}|{}", render_ranges(&self.ranges), self.topic)
        }
    }

    pub fn fingerprint_entry(&self, source_id: &str) -> String {
        if self.ranges.is_empty() {
            source_id.to_string()
        } else {
            format!("{source_id}:{}", render_ranges(&self.ranges))
        }
    }
}

fn render_ranges(ranges: &[(u32, u32)]) -> String {
    ranges
        .iter()
        .map(|&(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}-{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub fn parse_binding(id: &str, entry: &str) -> Result<Binding, SourceError> {
    binding_of(entry).map_err(|problem| SourceError::BadBinding {
        id: id.to_string(),
        entry: entry.to_string(),
        problem,
    })
}

fn binding_of(entry: &str) -> Result<Binding, &'static str> {
    let (ranges, topic) = match entry.split_once('|') {
        None => (None, entry),
        Some((_, tail)) if tail.contains('|') => return Err("has more than one `|`"),
        Some((ranges, topic)) => (Some(ranges), topic),
    };
    if !matches!(
        token::parse_id(topic),
        Some((token::Kind::Topic | token::Kind::Deck, ..))
    ) {
        return Err("names no topic or deck id");
    }
    let ranges = ranges.map(parse_ranges).transpose()?.unwrap_or_default();
    Ok(Binding {
        topic: topic.to_string(),
        ranges,
    })
}

fn parse_ranges(text: &str) -> Result<Vec<(u32, u32)>, &'static str> {
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for item in text.split(',') {
        let (start, end) = locator::parse_range(item).map_err(|_| "has a malformed range")?;
        if start == end && item.contains('-') {
            return Err("writes a single line as a range");
        }
        if ranges
            .last()
            .is_some_and(|&(_, previous_end)| start <= previous_end)
        {
            return Err("has ranges out of order or overlapping");
        }
        ranges.push((start, end));
    }
    Ok(ranges)
}

fn read_versioned<T: DeserializeOwned>(
    path: &Path,
    version_of: impl Fn(&T) -> u32,
    expected: u32,
) -> Result<Option<T>, SourceError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(path)(error)),
    };
    let document: T = toml::from_str(&text).map_err(|source| SourceError::Toml {
        path: path.to_path_buf(),
        source,
    })?;
    let found = version_of(&document);
    if found != expected {
        return Err(SourceError::Version { found });
    }
    Ok(Some(document))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn source_ids_and_fingerprints_match_the_frozen_vectors() {
        let a = b"fn main() {}\n";
        let b = b"# RFC 9110\nHTTP Semantics\r\n";
        assert_eq!(
            "source-0mvea1nvj28mr91t2assfed9k3w5nrpbv6x05qyg7af1aq55s87m",
            source_id(a)
        );
        assert_eq!(
            "source-0sfcvpgrsxk086zk7v55y3513c2m6tn8dw110m08s3pmx49rhb56",
            source_id(b)
        );
        assert_eq!(
            "1g2qyfy5b4bwnxgb09eeg9cychr7we5xx1zde0446a4sr4y16ed1",
            topic_fingerprint(&[source_id(b).as_str(), source_id(a).as_str()]),
            "order of input must not matter"
        );
        assert_eq!(
            "11vgtp9hpsee7bsm548vqqntbvxpv13q300x04ktzq9nk8a7m5p3",
            topic_fingerprint(&[source_id(a).as_str()])
        );
        assert_eq!(
            "1rxgrh19hz0w2jdfqx68k5qvj917ns0y8s4vjd6a95cs3dw55e2n",
            topic_fingerprint(&[]),
            "SHA-256 of zero bytes"
        );
        assert_eq!(
            "01v0mhhcpadaejd9ha62hh7s88de485zfn5dwzrr4nd9pd5941xr",
            topic_fingerprint(&[format!("{}:10-80,120", source_id(a)).as_str()]),
            "a ranged binding"
        );
        assert_eq!(
            "source-0mvea1nvj28mr91t2assfed9k3w5nrpbv6x05qyg7af1aq55s87m.rs",
            copy_name(&source_id(a), &"sdk:src/main.rs".parse().unwrap())
        );
    }

    #[test]
    fn source_id_and_copy_name_shapes_are_recognized_exactly() {
        let id = source_id(b"x");
        assert_eq!(59, id.len(), "source- plus 52");
        assert!(is_source_id(&id));
        let rejected = [
            id[..58].to_string(),
            format!("{id}0"),
            id.to_ascii_uppercase(),
            id.replacen("source-", "asset-", 1),
            id.replacen("source-0", "source-2", 1)
                .replacen("source-1", "source-2", 1),
            String::new(),
        ];
        for text in &rejected {
            assert!(!is_source_id(text), "{text:?} is not a source id");
        }
        for name in [format!("{id}.rs"), format!("{id}.txt"), format!("{id}.c99")] {
            assert!(is_copy_name(&name), "{name} is a copy name");
        }
        for name in [
            id.clone(),
            format!("{id}."),
            format!("{id}.RS"),
            format!("{id}.tar.gz"),
            format!("{id}.c++"),
            format!(".{id}.rs.tmp"),
            "manifest.toml".to_string(),
        ] {
            assert!(!is_copy_name(&name), "{name} is not a copy name");
        }
    }

    #[test]
    fn display_path_grammar_accepts_and_rejects_by_the_adr_rule() {
        let table: &[(&str, Result<(), &str>)] = &[
            ("sdk:src/lib.rs", Ok(())),
            ("dl:rfc9110.txt", Ok(())),
            ("sdk:docs/Übersicht.md", Ok(())),
            ("sdk:a b", Ok(())),
            ("sdk:ab:c", Ok(())),
            ("sdk:x/a:b", Ok(())),
            ("a-b_1:x", Ok(())),
            ("src/lib.rs", Err("has no key")),
            ("Sdk:x", Err("has a malformed key")),
            ("true:x", Err("has a malformed key")),
            ("false:x", Err("has a malformed key")),
            ("null:x", Err("has a malformed key")),
            (":x", Err("has a malformed key")),
            ("1sdk:x", Err("has a malformed key")),
            ("sdk:", Err("has an empty path")),
            ("sdk:/abs", Err("has an empty component")),
            ("sdk:a/", Err("has an empty component")),
            ("sdk:a//b", Err("has an empty component")),
            ("sdk:a/../b", Err("has a `..` component")),
            ("sdk:a/./b", Err("has a `.` component")),
            ("sdk:..", Err("has a `..` component")),
            ("sdk:a\\b", Err("contains a backslash")),
            ("sdk:C:\\x", Err("contains a backslash")),
            ("sdk:C:/x", Err("starts with a drive letter")),
            ("sdk:c:x", Err("starts with a drive letter")),
            ("sdk:a:b", Err("starts with a drive letter")),
            ("sdk:a\u{7}b", Err("contains a control character")),
            ("sdk:a\u{7f}b", Err("contains a control character")),
            ("sdk:a\nb", Err("contains a control character")),
        ];
        for (input, expected) in table {
            let parsed = input.parse::<DisplayPath>();
            match (expected, &parsed) {
                (Ok(()), Ok(display)) => {
                    assert_eq!(
                        *input,
                        display.to_string(),
                        "{input}: display renders the input"
                    );
                    assert_eq!(
                        (display.key.as_str(), display.path.as_str()),
                        input.split_once(':').unwrap(),
                        "{input}: split at the first colon"
                    );
                }
                (
                    Err(problem),
                    Err(SourceError::BadPath {
                        id: None,
                        path,
                        problem: found,
                    }),
                ) => {
                    assert_eq!(input, path, "{input}: the error carries the input");
                    assert_eq!(problem, found, "{input}: problem");
                }
                _ => panic!("{input}: expected {expected:?}, got {parsed:?}"),
            }
        }
    }

    #[test]
    fn copy_extension_follows_the_adr_rule_for_dotfiles_and_odd_suffixes() {
        let id = source_id(b"x");
        let table = [
            ("sdk:.bashrc", "txt"),
            ("sdk:.env.local", "local"),
            ("dl:notes.tar.gz", "gz"),
            ("sdk:README", "txt"),
            ("sdk:a.R", "r"),
            ("sdk:a.", "txt"),
            ("sdk:a.c++", "txt"),
            ("sdk:dir.v2/README", "txt"),
            ("sdk:docs/Übersicht.md", "md"),
            ("sdk:a.c99", "c99"),
        ];
        for (display, extension) in table {
            let name = copy_name(&id, &display.parse().unwrap());
            assert_eq!(format!("{id}.{extension}"), name, "{display}");
            assert!(is_copy_name(&name), "{display}: {name} is a copy name");
        }
    }

    #[test]
    fn display_paths_compare_as_bytes() {
        let upper: DisplayPath = "sdk:README.md".parse().unwrap();
        let lower: DisplayPath = "sdk:readme.md".parse().unwrap();
        assert!(upper < lower, "`R` sorts before `r`");
        let rows: BTreeSet<DisplayPath> = [upper, lower].into_iter().collect();
        assert_eq!(2, rows.len(), "two rows, not one");

        let hyphen: DisplayPath = "a-b:x".parse().unwrap();
        let bare: DisplayPath = "a:x".parse().unwrap();
        assert!(
            hyphen < bare,
            "the rendered bytes order (`-` before `:`), not the key then path"
        );
    }

    #[test]
    fn binding_grammar_accepts_and_rejects_by_the_adr_rule() {
        let id = source_id(b"x");
        type Accepted = (&'static str, &'static str, Vec<(u32, u32)>, &'static str);
        let accepted: &[Accepted] = &[
            ("topic-aaaa", "topic-aaaa", vec![], ""),
            ("deck-dddd", "deck-dddd", vec![], ""),
            (
                "10-80,120|topic-aaaa",
                "topic-aaaa",
                vec![(10, 80), (120, 120)],
                ":10-80,120",
            ),
            ("5|topic-aaaa", "topic-aaaa", vec![(5, 5)], ":5"),
            (
                "1-2,4-5|deck-dddd",
                "deck-dddd",
                vec![(1, 2), (4, 5)],
                ":1-2,4-5",
            ),
        ];
        for (entry, topic, ranges, suffix) in accepted {
            let binding = parse_binding(&id, entry).unwrap_or_else(|e| panic!("{entry}: {e}"));
            assert_eq!(*topic, binding.topic, "{entry}: topic");
            assert_eq!(*ranges, binding.ranges, "{entry}: ranges");
            assert_eq!(*entry, binding.render(), "{entry}: render is canonical");
            assert_eq!(
                format!("{id}{suffix}"),
                binding.fingerprint_entry(&id),
                "{entry}: fingerprint line"
            );
        }
        let rejected = [
            ("a|b|c", "has more than one `|`"),
            ("1|2|topic-aaaa", "has more than one `|`"),
            ("card-aaaa", "names no topic or deck id"),
            ("topic-AAAA", "names no topic or deck id"),
            ("topic-", "names no topic or deck id"),
            ("1-2|", "names no topic or deck id"),
            ("", "names no topic or deck id"),
            ("|topic-aaaa", "has a malformed range"),
            ("3-1|topic-aaaa", "has a malformed range"),
            ("01-2|topic-aaaa", "has a malformed range"),
            ("0|topic-aaaa", "has a malformed range"),
            ("1-2,|topic-aaaa", "has a malformed range"),
            ("1-2 |topic-aaaa", "has a malformed range"),
            ("1-1|topic-aaaa", "writes a single line as a range"),
            (
                "1-3,3-5|topic-aaaa",
                "has ranges out of order or overlapping",
            ),
            ("5,2|topic-aaaa", "has ranges out of order or overlapping"),
            ("2,2|topic-aaaa", "has ranges out of order or overlapping"),
        ];
        for (entry, problem) in rejected {
            match parse_binding(&id, entry) {
                Err(SourceError::BadBinding {
                    id: found_id,
                    entry: found_entry,
                    problem: found,
                }) => {
                    assert_eq!(id, found_id, "{entry}: the error names the source");
                    assert_eq!(entry, found_entry, "{entry}: the error carries the entry");
                    assert_eq!(problem, found, "{entry}: problem");
                }
                other => panic!("{entry}: expected {problem:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn line_count_matches_str_lines_for_every_terminator_shape() {
        for text in [
            "a",
            "a\n",
            "a\nb",
            "a\nb\n",
            "\n",
            "a\n\n",
            "a\r\nb\r\n",
            "a\rb",
        ] {
            assert_eq!(
                text.lines().count(),
                line_count(text.as_bytes()),
                "{text:?}"
            );
        }
        assert_eq!(0, line_count(b""), "no bytes, no lines");
    }
}
