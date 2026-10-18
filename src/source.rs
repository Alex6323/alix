use std::{
    hash::Hasher,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow, bail};
use twox_hash::XxHash64;

use crate::{
    card::SourceCitation,
    cite::{CitationRoots, ResolveError, Resolved, Root},
    deck::Deck,
    locator::{Locator, Target, format_range},
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
    Relocated { excerpt: Excerpt, locator: Locator },
    Changed,
    Ambiguous { locators: Vec<Locator> },
}

pub fn excerpt_fingerprint(excerpt: &Excerpt) -> u64 {
    fingerprint_lines(excerpt.lines.iter().map(|(_, line)| line.as_str()))
}

pub fn text_fingerprint(text: &str) -> u64 {
    fingerprint_lines(text.lines())
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
    let roots = CitationRoots::for_deck(&deck);
    let mut stamped = 0;
    let mut rewrites = Vec::new();
    for card in &deck.cards {
        for citation in &card.citations {
            let fingerprint = match roots.inspect_citation(citation)? {
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
                    labels(&locators)
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

pub fn labels(locators: &[Locator]) -> String {
    locators
        .iter()
        .map(|locator| format!("`{locator}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(feature = "full")]
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
    if crate::deck::is_url(source) {
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

#[cfg(feature = "full")]
/// A source value is one expression: a URL, a file, or a directory (ADR 0026).
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

impl CitationRoots {
    pub fn live_excerpt(&self, locator: &Locator) -> Result<Excerpt> {
        let Resolved::File { path, range, .. } = self.resolve(locator, false)? else {
            return Err(ResolveError::PassageWithoutAsset {
                key: locator.key.clone(),
            }
            .into());
        };
        let text = read_text(&path)?;
        let file_lines: Vec<&str> = text.lines().collect();
        Ok(select_lines(&path, locator, &file_lines, range)?)
    }

    /// A frozen citation reads its object in full; `at:` never indexes into
    /// the asset, it only supplies the display numbering (ADR 0026).
    pub(crate) fn citation_excerpt(&self, citation: &SourceCitation) -> Result<Excerpt> {
        let Some(asset) = citation.asset.as_deref() else {
            return self.live_excerpt(&citation.locator);
        };
        self.check(&citation.locator, true)?;
        if !crate::assets::is_object_name(asset) {
            bail!("frozen asset `{asset}` is not a content-addressed object name");
        }
        let asset_dir = self.asset_dir().ok_or_else(|| {
            anyhow!("citation names frozen asset `{asset}`, but the deck owns no asset directory")
        })?;
        let path = asset_dir.join(asset);
        let text = read_text(&path)?;
        let file_lines: Vec<&str> = text.lines().collect();
        let start = match &citation.locator.target {
            Target::File {
                range: Some((start, _)),
                ..
            } => *start as usize,
            _ => 1,
        };
        Ok(excerpt_from_lines(
            &path,
            &file_lines,
            1,
            file_lines.len(),
            start,
        ))
    }

    pub fn inspect_citation(&self, citation: &SourceCitation) -> Result<CitationIntegrity> {
        if citation.asset.is_some() {
            let excerpt = self.citation_excerpt(citation)?;
            return Ok(match citation.fingerprint {
                None => CitationIntegrity::Unfingerprinted {
                    fingerprint: excerpt_fingerprint(&excerpt),
                    excerpt,
                },
                Some(expected) if excerpt_fingerprint(&excerpt) == expected => {
                    CitationIntegrity::Current(excerpt)
                }
                Some(_) => CitationIntegrity::Changed,
            });
        }
        let locator = &citation.locator;
        let Resolved::File { path, range, .. } = self.resolve(locator, false)? else {
            return Err(ResolveError::PassageWithoutAsset {
                key: locator.key.clone(),
            }
            .into());
        };
        let text = read_text(&path)?;
        let file_lines: Vec<&str> = text.lines().collect();
        let current = select_lines(&path, locator, &file_lines, range);
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
        let Some((start, end)) = range else {
            current?;
            return Ok(CitationIntegrity::Changed);
        };
        let range_len = (end - start) as usize + 1;
        if file_lines.len() < range_len {
            return Ok(CitationIntegrity::Changed);
        }
        let mut matches = Vec::new();
        for offset in 0..=file_lines.len() - range_len {
            let window = &file_lines[offset..offset + range_len];
            if fingerprint_lines(window.iter().copied()) != expected {
                continue;
            }
            let Some(relocated) = relocated(locator, offset + 1, range_len) else {
                continue;
            };
            matches.push((
                excerpt_from_lines(&path, &file_lines, offset + 1, range_len, offset + 1),
                relocated,
            ));
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

    /// The excerpt as a reader sees it: a frozen excerpt's path is the cited
    /// path (or the url root of a passage), never the asset object.
    pub fn checked_excerpt(&self, citation: &SourceCitation) -> Result<Excerpt> {
        match self.inspect_citation(citation)? {
            CitationIntegrity::Current(mut excerpt) => {
                if citation.asset.is_some() {
                    excerpt.path = self.display_path(&citation.locator);
                }
                Ok(excerpt)
            }
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
                labels(&locators)
            ),
        }
    }

    fn display_path(&self, locator: &Locator) -> PathBuf {
        match (&locator.target, self.root_of(&locator.key)) {
            (Target::File { path, .. }, _) => PathBuf::from(path),
            (Target::Passage, Some(Root::Url(url))) => PathBuf::from(url),
            (Target::Passage, _) => PathBuf::from(&locator.key),
        }
    }
}

pub fn relocated(locator: &Locator, start: usize, len: usize) -> Option<Locator> {
    let Target::File { path, .. } = &locator.target else {
        return None;
    };
    let start = u32::try_from(start).ok()?;
    let end = start.checked_add(u32::try_from(len).ok()?.checked_sub(1)?)?;
    Some(Locator {
        key: locator.key.clone(),
        target: Target::File {
            path: path.clone(),
            range: Some((start, end)),
        },
    })
}

fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map_err(|error| anyhow!("cannot read the source `{}`: {error}", path.display()))
}

fn select_lines(
    path: &Path,
    locator: &Locator,
    file_lines: &[&str],
    range: Option<(u32, u32)>,
) -> Result<Excerpt, ResolveError> {
    let (start, end) = match range {
        Some((start, end)) => (start as usize, end as usize),
        None => (1, file_lines.len()),
    };
    if end > file_lines.len() || file_lines.is_empty() {
        let cited = match &locator.target {
            Target::File { path, .. } => path.clone(),
            Target::Passage => locator.key.clone(),
        };
        let range = match range {
            Some((start, end)) => format_range(start, end),
            None => "1".to_string(),
        };
        return Err(ResolveError::LinesMissing {
            path: cited,
            range,
            lines: file_lines.len(),
        });
    }
    Ok(excerpt_from_lines(
        path,
        file_lines,
        start,
        end - start + 1,
        start,
    ))
}

fn excerpt_from_lines(
    path: &Path,
    file_lines: &[&str],
    start: usize,
    len: usize,
    numbered_from: usize,
) -> Excerpt {
    let lines = file_lines
        .iter()
        .skip(start.saturating_sub(1))
        .take(len)
        .enumerate()
        .map(|(index, line)| (numbered_from + index, (*line).to_string()))
        .collect();
    Excerpt {
        path: path.to_path_buf(),
        lines,
        truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cite::DeckPlace;

    fn write(directory: &Path, name: &str, body: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn locator(text: &str) -> Locator {
        crate::locator::parse_locator_comment(&format!("<!-- at: {text} -->"))
            .unwrap()
            .locator
    }

    /// Roots `src` at `directory`, with frozen objects in `asset_dir` when given.
    fn roots(directory: &Path, asset_dir: Option<PathBuf>) -> CitationRoots {
        CitationRoots::from_place(
            &[("src".to_string(), ".".to_string())],
            DeckPlace::Loose { folder: directory },
            asset_dir,
        )
    }

    fn live(text: &str, fingerprint: Option<u64>) -> SourceCitation {
        SourceCitation {
            locator: locator(text),
            fingerprint,
            asset: None,
            line: 4,
        }
    }

    fn excerpt_of(lines: &[&str]) -> Excerpt {
        Excerpt {
            path: PathBuf::from("ignored"),
            lines: lines
                .iter()
                .enumerate()
                .map(|(index, line)| (index + 1, (*line).to_string()))
                .collect(),
            truncated: false,
        }
    }

    #[test]
    fn a_live_locator_whose_range_runs_past_the_file_is_an_error_not_a_clamp() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "f.txt", "a\nb\nc\n");
        let roots = roots(directory.path(), None);

        let error = roots.live_excerpt(&locator("src:f.txt:2-99")).unwrap_err();
        assert_eq!(
            Some(&ResolveError::LinesMissing {
                path: "f.txt".into(),
                range: "2-99".into(),
                lines: 3,
            }),
            error.downcast_ref::<ResolveError>(),
            "step read: {error:#}"
        );
        let error = roots
            .inspect_citation(&live("src:f.txt:2-99", None))
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("run past its end"),
            "step inspect without a fingerprint: {error:#}"
        );

        let fingerprint = excerpt_fingerprint(&excerpt_of(&["b", "c"]));
        let CitationIntegrity::Relocated { locator, excerpt } = roots
            .inspect_citation(&live("src:f.txt:3-4", Some(fingerprint)))
            .unwrap()
        else {
            panic!("step inspect with a fingerprint: a stale range past the end still relocates")
        };
        assert_eq!("src:f.txt:2-3", locator.to_string());
        assert_eq!(
            vec![(2, "b".to_string()), (3, "c".to_string())],
            excerpt.lines
        );
        assert!(
            matches!(
                roots
                    .inspect_citation(&live("src:f.txt:2-99", Some(fingerprint)))
                    .unwrap(),
                CitationIntegrity::Changed
            ),
            "step inspect with a window longer than the file: changed, never clamped"
        );
    }

    #[test]
    fn a_live_excerpt_reads_a_contiguous_span_and_a_long_source_in_full() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "f.txt", "a\nb\nc\nd\ne\n");
        let body: String = (1..=100).map(|line| format!("line {line}\n")).collect();
        write(directory.path(), "big.txt", &body);
        let roots = roots(directory.path(), None);

        let excerpt = roots.live_excerpt(&locator("src:f.txt:2-4")).unwrap();
        assert_eq!(
            vec![
                (2, "b".to_string()),
                (3, "c".to_string()),
                (4, "d".to_string())
            ],
            excerpt.lines
        );
        assert!(!excerpt.truncated);
        assert_eq!(
            vec![(1, "a".to_string())],
            roots.live_excerpt(&locator("src:f.txt:1")).unwrap().lines
        );

        let excerpt = roots.live_excerpt(&locator("src:big.txt")).unwrap();
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
            "---\nformat-version: 1\nid: \"deck-deck1\"\ncite:\n  - src: .\n---\n\
             ## q\nanswer\n<!-- at: src:big.rs:1-100 -->\n<!-- id: card-card1 -->\n",
        );
        assert_eq!(1, stamp_citations(&deck_path).unwrap());
        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(
            text.contains("<!-- at: src:big.rs:1-100 fingerprint: xxh64-"),
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
            CitationRoots::for_deck(&deck)
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
        let roots = roots(directory.path(), None);
        let citation = live(
            "src:code.rs:1-100",
            Some(fingerprint_lines(block.iter().map(String::as_str))),
        );
        let CitationIntegrity::Relocated { locator, excerpt } =
            roots.inspect_citation(&citation).unwrap()
        else {
            panic!("the full moved excerpt should relocate");
        };
        assert_eq!("src:code.rs:2-101", locator.to_string());
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
    fn excerpt_fingerprints_keep_line_boundaries() {
        assert_ne!(
            excerpt_fingerprint(&excerpt_of(&["ab", "c"])),
            excerpt_fingerprint(&excerpt_of(&["a", "bc"])),
            "the same bytes split at another line boundary are another excerpt"
        );
        assert_ne!(
            excerpt_fingerprint(&excerpt_of(&["ab", "c"])),
            excerpt_fingerprint(&excerpt_of(&["abc"])),
            "two lines are not their concatenation"
        );
    }

    #[test]
    fn a_moved_excerpt_is_found_by_fingerprint() {
        let directory = tempfile::tempdir().unwrap();
        write(
            directory.path(),
            "code.rs",
            "inserted\nalpha\nfn answer() {\n    42\n}\nomega\n",
        );
        let roots = roots(directory.path(), None);
        let expected = excerpt_of(&["fn answer() {", "    42", "}"]);
        let citation = live("src:code.rs:2-4", Some(excerpt_fingerprint(&expected)));
        let CitationIntegrity::Relocated { locator, excerpt } =
            roots.inspect_citation(&citation).unwrap()
        else {
            panic!("the exact excerpt should relocate");
        };
        assert_eq!("src:code.rs:3-5", locator.to_string());
        assert_eq!("fn answer() {", excerpt.lines[0].1);
    }

    #[test]
    fn a_changed_excerpt_is_not_relocated() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "code.rs", "alpha\nchanged\nomega\n");
        let roots = roots(directory.path(), None);
        let citation = live(
            "src:code.rs:2",
            Some(excerpt_fingerprint(&excerpt_of(&["original"]))),
        );
        assert!(matches!(
            roots.inspect_citation(&citation).unwrap(),
            CitationIntegrity::Changed
        ));
    }

    #[test]
    fn duplicate_exact_excerpts_are_ambiguous() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "code.rs", "same\nother\nsame\n");
        let roots = roots(directory.path(), None);
        let citation = live(
            "src:code.rs:2",
            Some(excerpt_fingerprint(&excerpt_of(&["same"]))),
        );
        let CitationIntegrity::Ambiguous { locators } = roots.inspect_citation(&citation).unwrap()
        else {
            panic!("duplicate excerpts should be ambiguous");
        };
        assert_eq!("`src:code.rs:1`, `src:code.rs:3`", labels(&locators));
    }

    /// Roots whose live tree is `directory` and whose frozen objects live in
    /// `directory/objects`, holding one excerpt-shaped object of `bytes`.
    fn frozen_roots(directory: &Path, bytes: &str) -> (CitationRoots, String) {
        let objects = directory.join("objects");
        std::fs::create_dir_all(&objects).unwrap();
        let name = crate::assets::object_name(bytes.as_bytes(), "rs");
        std::fs::write(objects.join(&name), bytes).unwrap();
        (roots(directory, Some(objects)), name)
    }

    fn frozen(text: &str, fingerprint: Option<u64>, asset: &str) -> SourceCitation {
        SourceCitation {
            locator: locator(text),
            fingerprint,
            asset: Some(asset.to_string()),
            line: 4,
        }
    }

    #[test]
    fn an_asset_backed_citation_reads_the_frozen_bytes_not_the_live_file() {
        let directory = tempfile::tempdir().unwrap();
        let (roots, asset) = frozen_roots(directory.path(), "beta\ngamma\n");
        write(directory.path(), "code.rs", "MUTATED\nLIVE\nFILE\n");
        let citation = frozen(
            "src:code.rs:2-3",
            Some(excerpt_fingerprint(&excerpt_of(&["beta", "gamma"]))),
            &asset,
        );
        let CitationIntegrity::Current(excerpt) = roots.inspect_citation(&citation).unwrap() else {
            panic!("the frozen citation must verify against the asset bytes");
        };
        assert_eq!(
            vec![(2, "beta".to_string()), (3, "gamma".to_string())],
            excerpt.lines
        );
        assert_eq!(
            PathBuf::from("code.rs"),
            roots.checked_excerpt(&citation).unwrap().path,
            "a reader sees the cited path, never the object"
        );
    }

    #[test]
    fn an_asset_backed_excerpt_is_numbered_from_the_at_start_line() {
        let directory = tempfile::tempdir().unwrap();
        let (roots, asset) = frozen_roots(directory.path(), "alpha\nbeta\ngamma\n");
        let citation = frozen(
            "src:x.rs:46-48",
            Some(excerpt_fingerprint(&excerpt_of(&[
                "alpha", "beta", "gamma",
            ]))),
            &asset,
        );
        let CitationIntegrity::Current(excerpt) = roots.inspect_citation(&citation).unwrap() else {
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
        let (roots, asset) = frozen_roots(directory.path(), "alpha\nbeta\n");
        let citation = frozen("src:x.rs", None, &asset);
        let CitationIntegrity::Unfingerprinted { excerpt, .. } =
            roots.inspect_citation(&citation).unwrap()
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
        let (roots, asset) = frozen_roots(directory.path(), "alpha\nbeta\ngamma\n");
        write(directory.path(), "code.rs", "inserted\nwanted\n");
        let citation = frozen(
            "src:code.rs:1",
            Some(excerpt_fingerprint(&excerpt_of(&["wanted"]))),
            &asset,
        );
        assert!(matches!(
            roots.inspect_citation(&citation).unwrap(),
            CitationIntegrity::Changed
        ));
    }

    #[test]
    fn a_frozen_citation_still_needs_a_visible_key() {
        let directory = tempfile::tempdir().unwrap();
        let (roots, asset) = frozen_roots(directory.path(), "a\n");
        let error = roots
            .inspect_citation(&frozen("other:code.rs:1", None, &asset))
            .unwrap_err();
        assert_eq!(
            Some(&ResolveError::UnknownKey {
                key: "other".into()
            }),
            error.downcast_ref::<ResolveError>(),
            "{error:#}"
        );
    }

    #[test]
    fn an_asset_name_that_is_not_content_addressed_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (roots, _) = frozen_roots(directory.path(), "a\n");
        let error = roots
            .inspect_citation(&frozen("src:code.rs:1", None, "../../escape.rs"))
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("not a content-addressed object name"),
            "{error:#}"
        );
    }

    #[test]
    fn an_asset_backed_citation_without_an_asset_directory_errors() {
        let directory = tempfile::tempdir().unwrap();
        let roots = roots(directory.path(), None);
        let citation = frozen(
            "src:code.rs:1",
            None,
            &crate::assets::object_name(b"a\n", "rs"),
        );
        let error = roots.inspect_citation(&citation).unwrap_err();
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
            "---\nformat-version: 1\nid: \"deck-deck1\"\ncite:\n  - src: .\n---\n\
             ## q\nanswer\n<!-- at: src:code.rs:2-3 -->\n<!-- id: card-card1 -->\n",
        );
        assert_eq!(1, stamp_citations(&deck_path).unwrap());
        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(text.contains("<!-- at: src:code.rs:2-3 fingerprint: xxh64-"));
        let deck = Deck::load(&deck_path).unwrap();
        assert_eq!(Some("card-card1".to_string()), deck.cards[0].id());
        assert!(matches!(
            CitationRoots::for_deck(&deck)
                .inspect_citation(&deck.cards[0].citations[0])
                .unwrap(),
            CitationIntegrity::Current(_)
        ));
    }

    #[test]
    fn stamping_a_deck_whose_citations_are_all_current_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "code.rs", "alpha\nbeta\ngamma\n");
        let deck_path = write(
            directory.path(),
            "deck.md",
            "---\nformat-version: 1\nid: \"deck-deck1\"\ncite:\n  - src: .\n---\n\
             ## q\nanswer\n<!-- at: src:code.rs:2-3 -->\n<!-- id: card-card1 -->\n",
        );
        assert_eq!(1, stamp_citations(&deck_path).unwrap());
        let stamped = std::fs::read_to_string(&deck_path).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        std::fs::File::options()
            .write(true)
            .open(&deck_path)
            .unwrap()
            .set_modified(old)
            .unwrap();

        assert_eq!(0, stamp_citations(&deck_path).unwrap());

        assert_eq!(stamped, std::fs::read_to_string(&deck_path).unwrap());
        assert_eq!(
            old,
            std::fs::metadata(&deck_path).unwrap().modified().unwrap(),
            "a deck with nothing to stamp is not rewritten"
        );
    }

    #[test]
    fn a_deck_cites_several_roots_and_a_member_sees_the_manifest_roots() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("docs")).unwrap();
        std::fs::create_dir_all(directory.path().join("src")).unwrap();
        write(directory.path(), "docs/README.md", "r1\nr2\nr3\n");
        write(directory.path(), "src/lib.rs", "l1\nl2\nl3\nl4\n");
        let deck_path = write(
            directory.path(),
            "facts.md",
            "---\ncite:\n  - docs: docs\n  - code: src\n---\n\
             ## q1\na1\n<!-- at: docs:README.md:1-2 -->\n\
             ## q2\na2\n<!-- at: code:lib.rs:3-4 -->\n",
        );
        let deck = Deck::load(&deck_path).unwrap();
        let roots = CitationRoots::for_deck(&deck);
        assert_eq!(
            vec![(1, "r1".to_string()), (2, "r2".to_string())],
            roots
                .live_excerpt(&deck.cards[0].citations[0].locator)
                .unwrap()
                .lines
        );
        assert_eq!(
            vec![(3, "l3".to_string()), (4, "l4".to_string())],
            roots
                .live_excerpt(&deck.cards[1].citations[0].locator)
                .unwrap()
                .lines
        );
        assert!(roots.live_excerpt(&locator("code:README.md:1")).is_err());

        std::fs::write(
            directory.path().join("alix.toml"),
            "[cite]\nws = \"docs\"\n",
        )
        .unwrap();
        std::fs::create_dir(directory.path().join("decks")).unwrap();
        let member = write(
            directory.path(),
            "decks/facts.md",
            "---\nformat-version: 1\nid: deck-deck1\ncite:\n  - own: src\n---\n\
             ## q\na\n<!-- at: ws:README.md:2 -->\n<!-- at: own:lib.rs:1 -->\n<!-- id: card-card1 -->\n",
        );
        let deck = Deck::load(&member).unwrap();
        let roots = CitationRoots::for_deck(&deck);
        assert_eq!(
            vec![(2, "r2".to_string())],
            roots
                .live_excerpt(&deck.cards[0].citations[0].locator)
                .unwrap()
                .lines
        );
        assert_eq!(
            vec![(1, "l1".to_string())],
            roots
                .live_excerpt(&deck.cards[0].citations[1].locator)
                .unwrap()
                .lines
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

    #[cfg(feature = "full")]
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

    #[test]
    fn an_excerpt_at_exactly_the_display_cap_is_not_marked_truncated() {
        let lines: Vec<(usize, String)> = (1..=MAX_EXCERPT_LINES)
            .map(|n| (n, format!("line {n}")))
            .collect();
        let exact = Excerpt {
            path: PathBuf::from("x.md"),
            lines: lines.clone(),
            truncated: false,
        }
        .capped_for_display();
        assert!(!exact.truncated);
        assert_eq!(MAX_EXCERPT_LINES, exact.lines.len());

        let mut over_lines = lines;
        over_lines.push((MAX_EXCERPT_LINES + 1, "one more".into()));
        let over = Excerpt {
            path: PathBuf::from("x.md"),
            lines: over_lines,
            truncated: false,
        }
        .capped_for_display();
        assert!(over.truncated);
        assert_eq!(MAX_EXCERPT_LINES, over.lines.len());
    }

    #[test]
    fn a_citation_spanning_the_whole_file_is_current_and_a_shifted_one_relocates() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "notes.txt", "a\nb\nc\n");
        let roots = roots(directory.path(), None);
        let fingerprint = excerpt_fingerprint(&excerpt_of(&["a", "b", "c"]));
        assert!(matches!(
            roots
                .inspect_citation(&live("src:notes.txt:1-3", Some(fingerprint)))
                .unwrap(),
            CitationIntegrity::Current(_)
        ));
        assert!(matches!(
            roots
                .inspect_citation(&live("src:notes.txt:2-4", Some(fingerprint)))
                .unwrap(),
            CitationIntegrity::Relocated { .. }
        ));
    }
}
