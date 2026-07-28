use std::{
    hash::Hasher,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow, bail};
use twox_hash::XxHash64;

use crate::{
    card::SourceCitation,
    deck::{Deck, is_url},
};

/// The display cap: [`Excerpt::capped_for_display`] truncates (with a marker)
/// beyond this so a huge excerpt never floods the screen. Evidence reads,
/// freezing, and fingerprints are never capped.
const MAX_EXCERPT_LINES: usize = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Excerpt {
    pub path: PathBuf,
    /// `(1-based line number, content)`, contiguous (a locator is a single
    /// span, so an excerpt never has gaps).
    pub lines: Vec<(usize, String)>,
    pub truncated: bool,
}

impl Excerpt {
    /// The rendering copy only: the full excerpt stays the evidence that is
    /// frozen and fingerprinted.
    pub fn capped_for_display(mut self) -> Excerpt {
        if self.lines.len() > MAX_EXCERPT_LINES {
            self.lines.truncate(MAX_EXCERPT_LINES);
            self.truncated = true;
        }
        self
    }
}

#[derive(Debug)]
pub enum CitationIntegrity {
    Current(Excerpt),
    Unfingerprinted { excerpt: Excerpt, fingerprint: u64 },
    Relocated { excerpt: Excerpt, locator: String },
    Changed,
    Ambiguous { locators: Vec<String> },
}

pub fn excerpt_fingerprint(excerpt: &Excerpt) -> u64 {
    fingerprint_lines(excerpt.lines.iter().map(|(_, line)| line.as_str()))
}

fn fingerprint_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> u64 {
    let mut hasher = XxHash64::default();
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            hasher.write_u8(b'\n');
        }
        hasher.write(line.trim_end().as_bytes());
    }
    hasher.finish()
}

/// The ADR 0026 locator fingerprint field value, e.g. `xxh64-0123456789abcdef`.
pub fn format_locator_fingerprint(fingerprint: u64) -> String {
    format!("xxh64-{fingerprint:016x}")
}

/// Accepts only the canonical dash form `format_locator_fingerprint` emits:
/// exactly 16 lowercase hex digits after `xxh64-`.
pub fn parse_locator_fingerprint(value: &str) -> Option<u64> {
    let hex = value.strip_prefix("xxh64-")?;
    let canonical = hex.len() == 16
        && hex
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character));
    if !canonical {
        return None;
    }
    u64::from_str_radix(hex, 16).ok()
}

pub fn stamp_citations(path: &Path) -> Result<usize> {
    let deck = Deck::load(path)?;
    let base = SourceBase::for_deck(&deck);
    let mut stamped = 0;
    let mut rewrites = Vec::new();
    for card in &deck.cards {
        for citation in &card.citations {
            let fingerprint = match base.inspect_citation(citation)? {
                CitationIntegrity::Current(_) => citation.fingerprint,
                CitationIntegrity::Unfingerprinted { fingerprint, .. } => {
                    stamped += 1;
                    Some(fingerprint)
                }
                CitationIntegrity::Relocated { locator, .. } => {
                    bail!("source excerpt moved to `{locator}` before it could be stamped")
                }
                CitationIntegrity::Changed => {
                    bail!("source excerpt changed before it could be stamped")
                }
                CitationIntegrity::Ambiguous { locators } => bail!(
                    "source excerpt matches several ranges before it could be stamped: {}",
                    locators.join(", ")
                ),
            };
            rewrites.push(crate::deck::AtRewrite {
                at: citation.locator.clone(),
                fingerprint,
                asset: citation.asset.clone(),
                line: citation.line,
            });
        }
    }
    if stamped > 0 {
        crate::deck::set_source_citations(path, &rewrites)?;
    }
    Ok(stamped)
}

/// A URL or absent source yields the deck's own folder as the base and no
/// source file.
pub(crate) fn resolve_source(
    deck_dir: Option<&Path>,
    source: Option<&str>,
) -> (PathBuf, Option<PathBuf>) {
    let deck_dir = deck_dir.unwrap_or_else(|| Path::new(".")).to_path_buf();
    let Some(source) = source else {
        return (deck_dir, None);
    };
    if is_url(source) {
        return (deck_dir, None);
    }
    let path = if Path::new(source).is_absolute() {
        PathBuf::from(source)
    } else {
        deck_dir.join(source)
    };
    if path.is_file() {
        let base = path.parent().map(Path::to_path_buf).unwrap_or(deck_dir);
        (base, Some(path))
    } else {
        (path, None)
    }
}

/// Computed once per deck load, so a frontend can read a card's cited
/// excerpt on reveal without re-loading the deck.
#[derive(Clone, Debug)]
pub struct SourceBase {
    base_dir: PathBuf,
    source_file: Option<PathBuf>,
    asset_dir: Option<PathBuf>,
}

impl SourceBase {
    pub fn for_deck(deck: &Deck) -> Self {
        let layers = deck.source_layers();
        let locals = layers.base_locals();
        let first = locals.first().map(|source| source.as_str());
        // With several local sources a lines-only locator is ambiguous, so no
        // single source file is exposed.
        let multi = locals.len() > 1;
        let content_root = crate::workspace::content_root(&deck.path);
        let (base_dir, source_file) = resolve_source(Some(&content_root), first);
        Self {
            base_dir,
            source_file: if multi { None } else { source_file },
            asset_dir: crate::workspace::root_for_deck(&deck.path)
                .zip(deck.deck_token.as_deref())
                .and_then(|(root, deck_id)| crate::assets::deck_dir(root, deck_id).ok()),
        }
    }

    pub fn excerpt(&self, locator: &str) -> Result<Excerpt> {
        excerpt_at(&self.base_dir, self.source_file.as_deref(), locator)
    }

    /// When the citation is asset-backed, bytes come from the frozen object,
    /// which holds exactly the excerpt, read in full; `at:` never indexes
    /// into the asset, it only supplies the real-source path and the display
    /// numbering (ADR 0026).
    pub(crate) fn citation_excerpt(&self, citation: &SourceCitation) -> Result<Excerpt> {
        let Some(asset) = citation.asset.as_deref() else {
            return self.excerpt(&citation.locator);
        };
        if !crate::assets::is_object_name(asset) {
            bail!("frozen asset `{asset}` is not a content-addressed object name");
        }
        let asset_dir = self.asset_dir.as_deref().ok_or_else(|| {
            anyhow!("citation names frozen asset `{asset}`, but the deck owns no asset directory")
        })?;
        let mut excerpt = read_excerpt(&asset_dir.join(asset), None)?;
        let (_, spec) = parse_locator(&citation.locator);
        let start = spec
            .as_deref()
            .map(|spec| parse_line_range(spec).0)
            .unwrap_or(1);
        for (index, line) in excerpt.lines.iter_mut().enumerate() {
            line.0 = start + index;
        }
        Ok(excerpt)
    }

    pub fn inspect_citation(&self, citation: &SourceCitation) -> Result<CitationIntegrity> {
        let current = self.citation_excerpt(citation);
        let Some(expected) = citation.fingerprint else {
            let excerpt = current?;
            return Ok(CitationIntegrity::Unfingerprinted {
                fingerprint: excerpt_fingerprint(&excerpt),
                excerpt,
            });
        };
        if let Ok(excerpt) = &current
            && excerpt_fingerprint(excerpt) == expected
        {
            return Ok(CitationIntegrity::Current(excerpt.clone()));
        }
        // A frozen object is immutable: a mismatch is corrupt evidence, and a
        // relocation scan against the live `at:` path must never re-derive the
        // frozen fingerprint (ADR 0026).
        if citation.asset.is_some() {
            return Ok(CitationIntegrity::Changed);
        }

        let (file, spec) = parse_locator(&citation.locator);
        let Some(spec) = spec else {
            return Ok(CitationIntegrity::Changed);
        };
        let path = self
            .locator_path(file.as_deref())
            .ok_or_else(|| anyhow!("cannot resolve source locator `{}`", citation.locator))?;
        let text = std::fs::read_to_string(&path)
            .map_err(|error| anyhow!("cannot read the source `{}`: {error}", path.display()))?;
        let file_lines: Vec<&str> = text.lines().collect();
        let (range_start, range_end) = parse_line_range(&spec);
        let range_len = range_end.saturating_sub(range_start) + 1;
        if file_lines.len() < range_len {
            return Ok(CitationIntegrity::Changed);
        }

        let mut matches = Vec::new();
        for offset in 0..=file_lines.len() - range_len {
            let window = &file_lines[offset..offset + range_len];
            if fingerprint_lines(window.iter().copied()) == expected {
                matches.push((
                    excerpt_from_lines(&path, &file_lines, offset + 1, range_len),
                    relocated_locator(file.as_deref(), offset + 1, range_len),
                ));
            }
        }
        match matches.len() {
            0 => Ok(CitationIntegrity::Changed),
            1 => {
                let (excerpt, locator) = matches.remove(0);
                Ok(CitationIntegrity::Relocated { excerpt, locator })
            }
            _ => Ok(CitationIntegrity::Ambiguous {
                locators: matches.into_iter().map(|(_, locator)| locator).collect(),
            }),
        }
    }

    pub fn checked_excerpt(&self, citation: &SourceCitation) -> Result<Excerpt> {
        match self.inspect_citation(citation)? {
            CitationIntegrity::Current(excerpt) => Ok(excerpt),
            CitationIntegrity::Unfingerprinted { .. } => bail!(
                "source excerpt has no fingerprint; review it, then run \
                 `alix doctor --repair-source-locators`"
            ),
            CitationIntegrity::Relocated { locator, .. } => bail!(
                "source excerpt moved to `{locator}`; run \
                 `alix doctor --repair-source-locators` to rebase it"
            ),
            CitationIntegrity::Changed => {
                bail!(
                    "source excerpt changed or disappeared; review the citation before updating it"
                )
            }
            CitationIntegrity::Ambiguous { locators } => bail!(
                "source excerpt matches several ranges ({}); review the citation before updating it",
                locators.join(", ")
            ),
        }
    }

    pub(crate) fn locator_path(&self, file: Option<&str>) -> Option<PathBuf> {
        locator_path(&self.base_dir, self.source_file.as_deref(), file)
    }
}

/// A source value is one expression: a URL, a file, or a directory (ADR 0026;
/// the " + " join is a parse error).
pub(crate) fn source_path(value: &str, base: Option<&Path>) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let path = Path::new(value);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.map(|directory| directory.join(path))
            .unwrap_or_else(|| path.to_path_buf())
    })
}

/// A locator is a single span, never comma-separated, so a stitched,
/// misleading excerpt is impossible.
pub(crate) fn parse_locator(locator: &str) -> (Option<String>, Option<String>) {
    let locator = locator.trim();
    if let Some((file, spec)) = locator.rsplit_once(':')
        && is_line_spec(spec)
    {
        return (Some(file.trim().to_string()), Some(spec.trim().to_string()));
    }
    if is_line_spec(locator) {
        return (None, Some(locator.to_string()));
    }
    (Some(locator.to_string()), None)
}

/// The parsed named-field locator, ADR 0026 ("Locator fields"): `at` is the
/// real `<src>:<lines>` (or lines-only, or whole-file) form; `fingerprint` is
/// the `xxh64-<hex>` change-detector; `asset` is the `sha256-<hex>.<ext>`
/// frozen object name, present only on a frozen citation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatorFields {
    pub at: String,
    pub fingerprint: Option<String>,
    pub asset: Option<String>,
}

fn take_locator_field(
    tokens: &[&str],
    index: &mut usize,
    key: &str,
    required: bool,
    whole: &str,
) -> Result<Option<String>> {
    if tokens.get(*index) != Some(&key) {
        if required {
            bail!("locator `{whole}` must start with `{key}`");
        }
        return Ok(None);
    }
    let field_value = tokens.get(*index + 1).copied().unwrap_or("");
    if field_value.is_empty() {
        bail!("locator `{whole}` has `{key}` with no value");
    }
    *index += 2;
    Ok(Some(field_value.to_string()))
}

/// The frozen locator tokenizer (ADR 0026): the value is split on single
/// spaces into a strictly alternating `at:`/`fingerprint:`/`asset:` key and
/// value stream, in that canonical order, each key at most once. Any unknown
/// key, duplicate key, unpaired token, or leftover content is a hard error
/// with no partial extraction, so an old ` @ `/` from ` locator can never
/// parse as a partial new one.
pub fn parse_locator_fields(value: &str) -> Result<LocatorFields> {
    let trimmed = value.trim();
    let tokens: Vec<&str> = trimmed.split(' ').collect();
    let mut index = 0;

    let at = take_locator_field(&tokens, &mut index, "at:", true, trimmed)?.unwrap_or_default();
    let fingerprint = take_locator_field(&tokens, &mut index, "fingerprint:", false, trimmed)?;
    let asset = take_locator_field(&tokens, &mut index, "asset:", false, trimmed)?;

    if index != tokens.len() {
        bail!(
            "locator `{trimmed}` has unexpected content starting at `{}`",
            tokens[index]
        );
    }

    Ok(LocatorFields {
        at,
        fingerprint,
        asset,
    })
}

/// Emits the canonical `at: <src>:<lines> fingerprint: xxh64-<hex> asset:
/// sha256-<hex>.<ext>` form, omitting absent optional fields; round-trips
/// with `parse_locator_fields`.
pub fn format_locator_fields(fields: &LocatorFields) -> String {
    let mut formatted = format!("at: {}", fields.at);
    if let Some(fingerprint) = &fields.fingerprint {
        formatted.push_str(&format!(" fingerprint: {fingerprint}"));
    }
    if let Some(asset) = &fields.asset {
        formatted.push_str(&format!(" asset: {asset}"));
    }
    formatted
}

fn is_line_spec(value: &str) -> bool {
    let value = value.trim();
    match value.split_once('-') {
        Some((start, end)) => is_number(start) && is_number(end),
        None => is_number(value),
    }
}

fn is_number(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

/// Parses a validated single range into inclusive `(start, end)` (a lone `N` is
/// `(N, N)`; a reversed range is normalized).
pub(crate) fn parse_line_range(spec: &str) -> (usize, usize) {
    let parse = |value: &str| value.trim().parse::<usize>().unwrap_or(1);
    let (start, end) = match spec.trim().split_once('-') {
        Some((start, end)) => (parse(start), parse(end)),
        None => {
            let line = parse(spec);
            (line, line)
        }
    };
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

pub(crate) fn locator_path(
    base_dir: &Path,
    source_file: Option<&Path>,
    file: Option<&str>,
) -> Option<PathBuf> {
    match source_file {
        Some(source_file) => Some(source_file.to_path_buf()),
        None => file.map(|file| resolve_under_base(base_dir, file)),
    }
}

/// A locator may be written relative to a project root above `base_dir`;
/// a direct join would double the overlap.
pub(crate) fn resolve_under_base(base_dir: &Path, file: &str) -> PathBuf {
    let direct = base_dir.join(file);
    if direct.exists() {
        return direct;
    }
    let mut ancestor = base_dir.parent();
    while let Some(directory) = ancestor {
        let candidate = directory.join(file);
        if candidate.exists() {
            return candidate;
        }
        ancestor = directory.parent();
    }
    if let Some(name) = Path::new(file).file_name()
        && let Some(found) = find_under(base_dir, name)
    {
        return found;
    }
    direct
}

fn find_under(root: &Path, name: &std::ffi::OsStr) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                let skip = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with('.') || matches!(name, "target" | "node_modules")
                    });
                if !skip {
                    stack.push(path);
                }
            } else if path.file_name() == Some(name) {
                return Some(path);
            }
        }
    }
    None
}

/// Repoints a frozen asset's excerpt at the real source path for display, so
/// the learner sees real source, not the opaque asset path. The reader
/// already numbered the lines from `at:`, so only the path and label change.
pub fn relabel_for_display(mut excerpt: Excerpt, at: &str) -> (Excerpt, Option<String>) {
    let (Some(file), _) = parse_locator(at) else {
        return (excerpt, None);
    };
    excerpt.path = PathBuf::from(&file);
    let label = match (excerpt.lines.first(), excerpt.lines.last()) {
        (Some((start, _)), Some((end, _))) if start != end => {
            format!("{file}:{start}-{end}")
        }
        (Some((start, _)), _) => format!("{file}:{start}"),
        _ => file,
    };
    (excerpt, Some(label))
}

fn excerpt_at(base_dir: &Path, source_file: Option<&Path>, locator: &str) -> Result<Excerpt> {
    let (file, spec) = parse_locator(locator);
    let joins_onto_base = source_file.is_none()
        && file
            .as_deref()
            .is_some_and(|file| !Path::new(file).is_absolute());
    if joins_onto_base && !base_dir.is_dir() {
        bail!(
            "the `source:` base `{}` does not exist — the deck's source path is \
             likely stale or wrong",
            base_dir.display()
        );
    }
    let path = locator_path(base_dir, source_file, file.as_deref()).ok_or_else(|| {
        anyhow!(
            "locator `{locator}` gives only line numbers, but `source:` \
             is not a single file — write it as `file:lines`"
        )
    })?;
    read_excerpt(&path, spec.as_deref())
}

fn read_excerpt(path: &Path, spec: Option<&str>) -> Result<Excerpt> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| anyhow!("cannot read the source `{}`: {error}", path.display()))?;
    let file_lines: Vec<&str> = text.lines().collect();
    let (start, end) = match spec {
        None => (1, file_lines.len()),
        Some(spec) => parse_line_range(spec),
    };
    let start = start.max(1);
    let end = end.min(file_lines.len());

    let requested = end.saturating_sub(start) + 1;
    let selected = excerpt_from_lines(path, &file_lines, start, requested);

    if selected.lines.is_empty() {
        bail!(
            "locator points outside `{}` ({} lines)",
            path.display(),
            file_lines.len()
        );
    }
    Ok(selected)
}

fn excerpt_from_lines(path: &Path, file_lines: &[&str], start: usize, len: usize) -> Excerpt {
    let lines = file_lines
        .iter()
        .enumerate()
        .skip(start.saturating_sub(1))
        .take(len)
        .map(|(index, line)| (index + 1, (*line).to_string()))
        .collect();
    Excerpt {
        path: path.to_path_buf(),
        lines,
        truncated: false,
    }
}

fn relocated_locator(file: Option<&str>, start: usize, len: usize) -> String {
    let range = if len == 1 {
        start.to_string()
    } else {
        format!("{start}-{}", start + len - 1)
    };
    match file {
        Some(file) => format!("{file}:{range}"),
        None => range,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(directory: &Path, name: &str, body: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn excerpt_at_resolves_a_file_and_line_locator() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "notes.md", "alpha\nbeta\ngamma\ndelta\n");
        let excerpt = excerpt_at(directory.path(), None, "notes.md:2-3").unwrap();
        assert_eq!(
            vec![(2, "beta".to_string()), (3, "gamma".to_string())],
            excerpt.lines
        );
    }

    #[test]
    fn excerpt_at_resolves_a_line_only_locator_against_the_single_source_file() {
        let directory = tempfile::tempdir().unwrap();
        let file = write(directory.path(), "notes.md", "alpha\nbeta\ngamma\n");
        let excerpt = excerpt_at(directory.path(), Some(&file), "2").unwrap();
        assert_eq!(vec![(2, "beta".to_string())], excerpt.lines);
    }

    #[test]
    fn excerpt_at_rejects_a_line_only_locator_without_a_single_file() {
        let directory = tempfile::tempdir().unwrap();
        let error = excerpt_at(directory.path(), None, "2-3").unwrap_err();
        assert!(format!("{error:#}").contains("only line numbers"));
    }

    #[test]
    fn excerpt_at_single_file_source_ignores_a_redundant_file_path() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("src/executor")).unwrap();
        let file = write(directory.path(), "src/executor/env.rs", "a\nb\nc\nd\n");
        let excerpt = excerpt_at(
            file.parent().unwrap(),
            Some(&file),
            "src/executor/env.rs:2-3",
        )
        .unwrap();
        assert_eq!(
            vec![(2, "b".to_string()), (3, "c".to_string())],
            excerpt.lines
        );
    }

    #[test]
    fn excerpt_at_reports_a_missing_source_base_clearly() {
        let base = std::env::temp_dir().join(format!("alix-nobase-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let error = format!(
            "{:#}",
            excerpt_at(&base, None, "src/lib.rs:1-3").unwrap_err()
        );
        assert!(error.contains(&base.display().to_string()), "{error}");
        assert!(error.contains("does not exist"), "{error}");
        assert!(!error.contains("src/lib.rs"), "{error}");
    }

    #[test]
    fn excerpt_at_resolves_an_ancestor_relative_locator_against_a_subdir_source() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("src/executor")).unwrap();
        write(directory.path(), "src/executor/local_vm.rs", "a\nb\nc\nd\n");
        let base_dir = directory.path().join("src/executor");
        let excerpt = excerpt_at(&base_dir, None, "src/executor/local_vm.rs:2-3").unwrap();
        assert_eq!(
            vec![(2, "b".to_string()), (3, "c".to_string())],
            excerpt.lines
        );
    }

    #[test]
    fn excerpt_at_recovers_a_dropped_subdirectory_via_basename_search() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("src")).unwrap();
        write(directory.path(), "src/chapter.md", "a\nb\nc\nd\n");
        let excerpt = excerpt_at(directory.path(), None, "chapter.md:2-3").unwrap();
        assert_eq!(
            vec![(2, "b".to_string()), (3, "c".to_string())],
            excerpt.lines
        );
    }

    #[test]
    fn relabel_for_display_repoints_the_path_from_at() {
        let excerpt = Excerpt {
            path: PathBuf::from(
                "/ws/assets/deck-deck1/sha256-ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.rs",
            ),
            lines: vec![
                (106, "a".to_string()),
                (107, "b".to_string()),
                (108, "c".to_string()),
            ],
            truncated: false,
        };
        let (excerpt, label) = relabel_for_display(excerpt, "src/caching.rs:106-108");
        assert_eq!("src/caching.rs", excerpt.path.to_str().unwrap());
        assert_eq!(
            vec![
                (106, "a".to_string()),
                (107, "b".to_string()),
                (108, "c".to_string())
            ],
            excerpt.lines
        );
        assert_eq!(Some("src/caching.rs:106-108".to_string()), label);
    }

    #[test]
    fn relabel_for_display_is_a_noop_for_a_lines_only_at() {
        let excerpt = Excerpt {
            path: PathBuf::from("/src/foo.rs"),
            lines: vec![(10, "x".to_string())],
            truncated: false,
        };
        let (same, label) = relabel_for_display(excerpt.clone(), "10-12");
        assert_eq!(excerpt.path, same.path);
        assert_eq!(excerpt.lines, same.lines);
        assert_eq!(None, label);
    }

    #[test]
    fn source_base_reads_a_fact_cards_citation() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "notes.md", "one\ntwo\nthree\nfour\n");
        let deck_path = write(
            directory.path(),
            "facts.md",
            "---\nsource: notes.md\n---\n## q\na\n<!-- at: notes.md:2-3 -->\n",
        );
        let deck = Deck::load(&deck_path).unwrap();
        let base = SourceBase::for_deck(&deck);
        let locator = &deck.cards[0].citations[0].locator;
        assert_eq!(
            vec![(2, "two".to_string()), (3, "three".to_string())],
            base.excerpt(locator).unwrap().lines
        );
        assert_eq!(
            vec![(3, "three".to_string())],
            base.excerpt("3").unwrap().lines
        );
    }

    #[test]
    fn source_base_reads_a_multi_file_citation() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "README.md", "r1\nr2\nr3\n");
        std::fs::create_dir(directory.path().join("src")).unwrap();
        write(directory.path(), "src/lib.rs", "l1\nl2\nl3\nl4\n");
        let readme = directory.path().join("README.md");
        let deck_path = write(
            directory.path(),
            "facts.md",
            &format!(
                "---\nsource:\n  - {}\n  - src/lib.rs\n---\n\
                 ## q1\na1\n<!-- at: README.md:1-2 -->\n\
                 ## q2\na2\n<!-- at: src/lib.rs:3-4 -->\n",
                readme.display()
            ),
        );
        let deck = Deck::load(&deck_path).unwrap();
        let base = SourceBase::for_deck(&deck);

        assert_eq!(
            vec![(1, "r1".to_string()), (2, "r2".to_string())],
            base.excerpt(&deck.cards[0].citations[0].locator)
                .unwrap()
                .lines
        );
        assert_eq!(
            vec![(3, "l3".to_string()), (4, "l4".to_string())],
            base.excerpt(&deck.cards[1].citations[0].locator)
                .unwrap()
                .lines
        );
        assert!(base.excerpt("2").is_err());
    }

    #[test]
    fn a_manifest_source_does_not_break_lines_only_locators_of_a_single_file_member() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("alix.toml"), "source = \"ws.md\"\n").unwrap();
        std::fs::create_dir(directory.path().join("decks")).unwrap();
        write(directory.path(), "ws.md", "w1\nw2\n");
        write(directory.path(), "own.md", "o1\no2\no3\n");
        let deck_path = write(
            directory.path(),
            "decks/facts.md",
            "---\nid: deck-deck1\nsource: own.md\n---\n## q <!-- id: card-card1 -->\na\n<!-- at: 2 -->\n",
        );
        let deck = Deck::load(&deck_path).unwrap();
        let base = SourceBase::for_deck(&deck);
        assert_eq!(
            vec![(2, "o2".to_string())],
            base.excerpt(&deck.cards[0].citations[0].locator)
                .unwrap()
                .lines
        );
    }

    #[test]
    fn parse_locator_splits_file_and_spec() {
        assert_eq!(
            (Some("card.rs".to_string()), Some("1-9".to_string())),
            parse_locator("card.rs:1-9")
        );
        assert_eq!(
            (
                Some("src/serve.rs".to_string()),
                Some("682-689".to_string())
            ),
            parse_locator("src/serve.rs:682-689")
        );
        assert_eq!(
            (Some("src/serve.rs:544,980".to_string()), None),
            parse_locator("src/serve.rs:544,980")
        );
        assert_eq!(
            (None, Some("151-158".to_string())),
            parse_locator("151-158")
        );
        assert_eq!(
            (Some("notes.md".to_string()), None),
            parse_locator("notes.md")
        );
    }

    #[test]
    fn format_locator_fingerprint_uses_the_dash_form() {
        assert_eq!(
            "xxh64-0123456789abcdef",
            format_locator_fingerprint(0x0123456789abcdef)
        );
    }

    #[test]
    fn parse_locator_fingerprint_round_trips_the_dash_form_only() {
        assert_eq!(
            Some(0x0123456789abcdef),
            parse_locator_fingerprint("xxh64-0123456789abcdef")
        );
        assert_eq!(
            Some(42),
            parse_locator_fingerprint(&format_locator_fingerprint(42))
        );
        assert_eq!(None, parse_locator_fingerprint("xxh64:0123456789abcdef"));
        assert_eq!(None, parse_locator_fingerprint("xxh64-0123"));
        assert_eq!(None, parse_locator_fingerprint("xxh64-0123456789ABCDEF"));
        assert_eq!(None, parse_locator_fingerprint("0123456789abcdef"));
    }

    #[test]
    fn parse_locator_fields_parses_the_canonical_three_field_locator() {
        let fields = parse_locator_fields(
            "at: notes.md:12-18 fingerprint: xxh64-0123456789abcdef asset: sha256-abc123.rs",
        )
        .unwrap();
        assert_eq!(
            LocatorFields {
                at: "notes.md:12-18".to_string(),
                fingerprint: Some("xxh64-0123456789abcdef".to_string()),
                asset: Some("sha256-abc123.rs".to_string()),
            },
            fields
        );
    }

    #[test]
    fn parse_locator_fields_accepts_at_only() {
        let fields = parse_locator_fields("at: notes.md:12-18").unwrap();
        assert_eq!(
            LocatorFields {
                at: "notes.md:12-18".to_string(),
                fingerprint: None,
                asset: None,
            },
            fields
        );
    }

    #[test]
    fn parse_locator_fields_accepts_at_and_fingerprint_without_asset() {
        let fields =
            parse_locator_fields("at: notes.md:12-18 fingerprint: xxh64-0123456789abcdef").unwrap();
        assert_eq!(
            LocatorFields {
                at: "notes.md:12-18".to_string(),
                fingerprint: Some("xxh64-0123456789abcdef".to_string()),
                asset: None,
            },
            fields
        );
    }

    #[test]
    fn parse_locator_fields_accepts_a_lines_only_at_value() {
        let fields = parse_locator_fields("at: 12-18").unwrap();
        assert_eq!(
            LocatorFields {
                at: "12-18".to_string(),
                fingerprint: None,
                asset: None,
            },
            fields
        );
    }

    #[test]
    fn parse_locator_fields_accepts_a_whole_file_at_value() {
        let fields = parse_locator_fields("at: notes.md").unwrap();
        assert_eq!(
            LocatorFields {
                at: "notes.md".to_string(),
                fingerprint: None,
                asset: None,
            },
            fields
        );
    }

    #[test]
    fn parse_locator_fields_rejects_an_old_style_locator() {
        let error = parse_locator_fields("at: 29.rs @ xxh64:0123456789abcdef from src/x.rs:1-3")
            .unwrap_err();
        assert!(format!("{error:#}").contains("unexpected content"));
    }

    #[test]
    fn parse_locator_fields_rejects_an_unknown_key() {
        assert!(parse_locator_fields("foo: bar").is_err());
    }

    #[test]
    fn parse_locator_fields_rejects_a_duplicate_key() {
        assert!(parse_locator_fields("at: a:1 at: b:2").is_err());
    }

    #[test]
    fn parse_locator_fields_rejects_a_fingerprint_key_with_no_value() {
        assert!(parse_locator_fields("at: notes.md:12-18 fingerprint:").is_err());
    }

    #[test]
    fn parse_locator_fields_rejects_a_space_in_the_path() {
        assert!(parse_locator_fields("at: my notes.md:1-2").is_err());
    }

    #[test]
    fn parse_locator_fields_rejects_a_missing_at_field() {
        assert!(parse_locator_fields("fingerprint: xxh64-0123456789abcdef").is_err());
    }

    #[test]
    fn parse_locator_fields_rejects_asset_out_of_canonical_order() {
        assert!(
            parse_locator_fields(
                "at: notes.md asset: sha256-abc123.rs fingerprint: xxh64-0123456789abcdef"
            )
            .is_err()
        );
    }

    #[test]
    fn format_locator_fields_round_trips_the_canonical_form() {
        let canonical =
            "at: notes.md:12-18 fingerprint: xxh64-0123456789abcdef asset: sha256-abc123.rs";
        let fields = parse_locator_fields(canonical).unwrap();
        assert_eq!(canonical, format_locator_fields(&fields));
    }

    #[test]
    fn parse_line_range_handles_single_range_and_reversed() {
        assert_eq!((1, 9), parse_line_range("1-9"));
        assert_eq!((5, 5), parse_line_range("5"));
        assert_eq!((8, 12), parse_line_range("12-8"));
    }

    #[test]
    fn read_excerpt_selects_a_contiguous_span_with_line_numbers() {
        let directory = tempfile::tempdir().unwrap();
        let path = write(directory.path(), "f.txt", "a\nb\nc\nd\ne\n");
        let excerpt = read_excerpt(&path, Some("2-4")).unwrap();
        assert_eq!(
            vec![
                (2, "b".to_string()),
                (3, "c".to_string()),
                (4, "d".to_string())
            ],
            excerpt.lines
        );
        assert!(!excerpt.truncated);
        let excerpt = read_excerpt(&path, Some("1")).unwrap();
        assert_eq!(vec![(1, "a".to_string())], excerpt.lines);
    }

    #[test]
    fn read_excerpt_clamps_out_of_range_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = write(directory.path(), "f.txt", "a\nb\nc\n");
        let excerpt = read_excerpt(&path, Some("2-99")).unwrap();
        assert_eq!(
            vec![(2, "b".to_string()), (3, "c".to_string())],
            excerpt.lines
        );
        assert!(read_excerpt(&path, Some("99")).is_err());
    }

    #[test]
    fn read_excerpt_reads_a_long_source_in_full_and_only_display_capping_truncates() {
        let directory = tempfile::tempdir().unwrap();
        let body: String = (1..=100).map(|line| format!("line {line}\n")).collect();
        let path = write(directory.path(), "big.txt", &body);
        let excerpt = read_excerpt(&path, None).unwrap();
        assert_eq!(100, excerpt.lines.len());
        assert!(!excerpt.truncated);

        let display = excerpt.clone().capped_for_display();
        assert_eq!(MAX_EXCERPT_LINES, display.lines.len());
        assert!(display.truncated);
        assert_eq!(excerpt.lines[..MAX_EXCERPT_LINES], display.lines[..]);
    }

    #[test]
    fn stamping_and_drift_detection_cover_lines_beyond_the_display_cap() {
        let directory = tempfile::tempdir().unwrap();
        let body: String = (1..=100).map(|line| format!("line {line}\n")).collect();
        write(directory.path(), "big.rs", &body);
        let deck_path = write(
            directory.path(),
            "deck.md",
            "---\nid: \"deck-deck1\"\nsource: .\n---\n\
             ## q\nanswer\n<!-- at: big.rs:1-100 -->\n<!-- id: card-card1 -->\n",
        );
        assert_eq!(1, stamp_citations(&deck_path).unwrap());
        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(
            text.contains("<!-- at: big.rs:1-100 fingerprint: xxh64-"),
            "the authored range must be stamped verbatim: {text}"
        );

        let drifted: String = (1..=100)
            .map(|line| {
                if line == 90 {
                    "DRIFTED\n".to_string()
                } else {
                    format!("line {line}\n")
                }
            })
            .collect();
        write(directory.path(), "big.rs", &drifted);
        let deck = Deck::load(&deck_path).unwrap();
        assert!(matches!(
            SourceBase::for_deck(&deck)
                .inspect_citation(&deck.cards[0].citations[0])
                .unwrap(),
            CitationIntegrity::Changed
        ));
    }

    #[test]
    fn a_moved_excerpt_longer_than_the_display_cap_relocates_with_its_full_range() {
        let directory = tempfile::tempdir().unwrap();
        let block: Vec<String> = (1..=100).map(|line| format!("line {line}")).collect();
        let body = format!("inserted\n{}\n", block.join("\n"));
        write(directory.path(), "code.rs", &body);
        let source = SourceBase {
            base_dir: directory.path().to_path_buf(),
            source_file: None,
            asset_dir: None,
        };
        let citation = SourceCitation {
            locator: "code.rs:1-100".into(),
            fingerprint: Some(fingerprint_lines(block.iter().map(String::as_str))),
            asset: None,
            line: 4,
        };
        let CitationIntegrity::Relocated { locator, excerpt } =
            source.inspect_citation(&citation).unwrap()
        else {
            panic!("the full moved excerpt should relocate");
        };
        assert_eq!("code.rs:2-101", locator);
        assert_eq!(100, excerpt.lines.len());
        assert_eq!("line 100", excerpt.lines[99].1);
    }

    #[test]
    fn excerpt_fingerprints_ignore_line_numbers_and_trailing_whitespace() {
        let a = Excerpt {
            path: PathBuf::from("a.rs"),
            lines: vec![(4, "fn answer() {  ".into()), (5, "    42".into())],
            truncated: false,
        };
        let b = Excerpt {
            path: PathBuf::from("b.rs"),
            lines: vec![(40, "fn answer() {".into()), (41, "    42".into())],
            truncated: false,
        };
        assert_eq!(excerpt_fingerprint(&a), excerpt_fingerprint(&b));
    }

    #[test]
    fn a_moved_excerpt_is_found_by_fingerprint() {
        let directory = tempfile::tempdir().unwrap();
        write(
            directory.path(),
            "code.rs",
            "inserted\nalpha\nfn answer() {\n    42\n}\nomega\n",
        );
        let source = SourceBase {
            base_dir: directory.path().to_path_buf(),
            source_file: None,
            asset_dir: None,
        };
        let expected = Excerpt {
            path: PathBuf::from("code.rs"),
            lines: vec![
                (2, "fn answer() {".into()),
                (3, "    42".into()),
                (4, "}".into()),
            ],
            truncated: false,
        };
        let citation = SourceCitation {
            locator: "code.rs:2-4".into(),
            fingerprint: Some(excerpt_fingerprint(&expected)),
            asset: None,
            line: 4,
        };
        let CitationIntegrity::Relocated { locator, excerpt } =
            source.inspect_citation(&citation).unwrap()
        else {
            panic!("the exact excerpt should relocate");
        };
        assert_eq!("code.rs:3-5", locator);
        assert_eq!("fn answer() {", excerpt.lines[0].1);
    }

    #[test]
    fn a_changed_excerpt_is_not_relocated() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "code.rs", "alpha\nchanged\nomega\n");
        let source = SourceBase {
            base_dir: directory.path().to_path_buf(),
            source_file: None,
            asset_dir: None,
        };
        let expected = Excerpt {
            path: PathBuf::from("code.rs"),
            lines: vec![(2, "original".into())],
            truncated: false,
        };
        let citation = SourceCitation {
            locator: "code.rs:2".into(),
            fingerprint: Some(excerpt_fingerprint(&expected)),
            asset: None,
            line: 4,
        };
        assert!(matches!(
            source.inspect_citation(&citation).unwrap(),
            CitationIntegrity::Changed
        ));
    }

    #[test]
    fn duplicate_exact_excerpts_are_ambiguous() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "code.rs", "same\nother\nsame\n");
        let source = SourceBase {
            base_dir: directory.path().to_path_buf(),
            source_file: None,
            asset_dir: None,
        };
        let expected = Excerpt {
            path: PathBuf::from("code.rs"),
            lines: vec![(2, "same".into())],
            truncated: false,
        };
        let citation = SourceCitation {
            locator: "code.rs:2".into(),
            fingerprint: Some(excerpt_fingerprint(&expected)),
            asset: None,
            line: 4,
        };
        let CitationIntegrity::Ambiguous { locators } = source.inspect_citation(&citation).unwrap()
        else {
            panic!("duplicate excerpts should be ambiguous");
        };
        assert_eq!(vec!["code.rs:1", "code.rs:3"], locators);
    }

    /// A base whose live tree is `directory` and whose frozen objects live in
    /// `directory/objects`, holding one excerpt-shaped object of `bytes`.
    fn frozen_base(directory: &Path, bytes: &str) -> (SourceBase, String) {
        let objects = directory.join("objects");
        std::fs::create_dir_all(&objects).unwrap();
        let name = crate::assets::object_name(bytes.as_bytes(), "rs");
        std::fs::write(objects.join(&name), bytes).unwrap();
        let base = SourceBase {
            base_dir: directory.to_path_buf(),
            source_file: None,
            asset_dir: Some(objects),
        };
        (base, name)
    }

    #[test]
    fn an_asset_backed_citation_reads_the_frozen_bytes_not_the_live_file() {
        let directory = tempfile::tempdir().unwrap();
        let (base, asset) = frozen_base(directory.path(), "beta\ngamma\n");
        write(directory.path(), "code.rs", "MUTATED\nLIVE\nFILE\n");
        let expected = Excerpt {
            path: PathBuf::from("code.rs"),
            lines: vec![(2, "beta".into()), (3, "gamma".into())],
            truncated: false,
        };
        let citation = SourceCitation {
            locator: "code.rs:2-3".into(),
            fingerprint: Some(excerpt_fingerprint(&expected)),
            asset: Some(asset),
            line: 4,
        };
        let CitationIntegrity::Current(excerpt) = base.inspect_citation(&citation).unwrap() else {
            panic!("the frozen citation must verify against the asset bytes");
        };
        assert_eq!(
            vec![(2, "beta".to_string()), (3, "gamma".to_string())],
            excerpt.lines
        );
    }

    #[test]
    fn an_asset_backed_excerpt_is_numbered_from_the_at_start_line() {
        let directory = tempfile::tempdir().unwrap();
        let (base, asset) = frozen_base(directory.path(), "alpha\nbeta\ngamma\n");
        let content = Excerpt {
            path: PathBuf::from("x.rs"),
            lines: vec![
                (46, "alpha".into()),
                (47, "beta".into()),
                (48, "gamma".into()),
            ],
            truncated: false,
        };
        let citation = SourceCitation {
            locator: "x.rs:46-48".into(),
            fingerprint: Some(excerpt_fingerprint(&content)),
            asset: Some(asset),
            line: 4,
        };
        let CitationIntegrity::Current(excerpt) = base.inspect_citation(&citation).unwrap() else {
            panic!("the frozen citation must verify against the asset bytes");
        };
        assert_eq!(
            vec![
                (46, "alpha".to_string()),
                (47, "beta".to_string()),
                (48, "gamma".to_string())
            ],
            excerpt.lines
        );
    }

    #[test]
    fn an_asset_backed_whole_file_at_keeps_one_based_numbering() {
        let directory = tempfile::tempdir().unwrap();
        let (base, asset) = frozen_base(directory.path(), "alpha\nbeta\n");
        let citation = SourceCitation {
            locator: "x.rs".into(),
            fingerprint: None,
            asset: Some(asset),
            line: 4,
        };
        let CitationIntegrity::Unfingerprinted { excerpt, .. } =
            base.inspect_citation(&citation).unwrap()
        else {
            panic!("an unfingerprinted asset-backed citation still reads the asset");
        };
        assert_eq!(
            vec![(1, "alpha".to_string()), (2, "beta".to_string())],
            excerpt.lines
        );
    }

    #[test]
    fn an_asset_backed_mismatch_is_changed_never_relocated_from_the_live_tree() {
        let directory = tempfile::tempdir().unwrap();
        let (base, asset) = frozen_base(directory.path(), "alpha\nbeta\ngamma\n");
        // The live file holds the cited excerpt at a shifted position; a live
        // relocation scan would find it and re-derive the fingerprint.
        write(directory.path(), "code.rs", "inserted\nwanted\n");
        let wanted = Excerpt {
            path: PathBuf::from("code.rs"),
            lines: vec![(1, "wanted".into())],
            truncated: false,
        };
        let citation = SourceCitation {
            locator: "code.rs:1".into(),
            fingerprint: Some(excerpt_fingerprint(&wanted)),
            asset: Some(asset),
            line: 4,
        };
        assert!(matches!(
            base.inspect_citation(&citation).unwrap(),
            CitationIntegrity::Changed
        ));
    }

    #[test]
    fn an_asset_name_that_is_not_content_addressed_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (base, _) = frozen_base(directory.path(), "a\n");
        let citation = SourceCitation {
            locator: "code.rs:1".into(),
            fingerprint: None,
            asset: Some("../../escape.rs".into()),
            line: 4,
        };
        let error = base.inspect_citation(&citation).unwrap_err();
        assert!(
            format!("{error:#}").contains("not a content-addressed object name"),
            "{error:#}"
        );
    }

    #[test]
    fn an_asset_backed_citation_without_an_asset_directory_errors() {
        let directory = tempfile::tempdir().unwrap();
        let base = SourceBase {
            base_dir: directory.path().to_path_buf(),
            source_file: None,
            asset_dir: None,
        };
        let citation = SourceCitation {
            locator: "code.rs:1".into(),
            fingerprint: None,
            asset: Some(crate::assets::object_name(b"a\n", "rs")),
            line: 4,
        };
        let error = base.inspect_citation(&citation).unwrap_err();
        assert!(
            format!("{error:#}").contains("owns no asset directory"),
            "{error:#}"
        );
    }

    #[test]
    fn stamping_citations_writes_the_current_fingerprint_without_changing_ids() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "code.rs", "alpha\nbeta\ngamma\n");
        let deck_path = write(
            directory.path(),
            "deck.md",
            "---\nid: \"deck-deck1\"\nsource: .\n---\n\
             ## q\nanswer\n<!-- at: code.rs:2-3 -->\n<!-- id: card-card1 -->\n",
        );
        assert_eq!(1, stamp_citations(&deck_path).unwrap());
        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(text.contains("<!-- at: code.rs:2-3 fingerprint: xxh64-"));
        let deck = Deck::load(&deck_path).unwrap();
        assert_eq!(Some("card-card1".to_string()), deck.cards[0].id());
        assert!(matches!(
            SourceBase::for_deck(&deck)
                .inspect_citation(&deck.cards[0].citations[0])
                .unwrap(),
            CitationIntegrity::Current(_)
        ));
    }

    #[test]
    fn source_path_resolves_one_expression_absolute_or_against_the_base() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(project.join("src/lib.rs"), "l").unwrap();

        assert_eq!(
            Some(project.join("src/lib.rs")),
            source_path("project/src/lib.rs", Some(directory.path()))
        );
        let one = project.join("src/lib.rs");
        assert_eq!(Some(one.clone()), source_path(&one.to_string_lossy(), None));
        assert_eq!(None, source_path("   ", None));
    }
}
