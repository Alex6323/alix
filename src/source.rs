use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::deck::{Deck, is_url};

/// Truncated (with a marker) beyond this, so a huge locator never floods the
/// screen.
const MAX_EXCERPT_LINES: usize = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Excerpt {
    pub path: PathBuf,
    /// `(1-based line number, content)`, contiguous (a locator is a single
    /// span, so an excerpt never has gaps).
    pub lines: Vec<(usize, String)>,
    pub truncated: bool,
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
}

impl SourceBase {
    pub fn for_deck(deck: &Deck) -> Self {
        let first = deck.sources.first();
        let multi = first.is_some_and(|source| source.contains(" + "));
        let (base_dir, source_file) =
            resolve_source(deck.path.parent(), first.map(|source| first_source(source)));
        Self {
            base_dir,
            source_file: if multi { None } else { source_file },
        }
    }

    pub fn excerpt(&self, locator: &str) -> Result<Excerpt> {
        excerpt_at(&self.base_dir, self.source_file.as_deref(), locator)
    }

    pub(crate) fn locator_path(&self, file: Option<&str>) -> Option<PathBuf> {
        locator_path(&self.base_dir, self.source_file.as_deref(), file)
    }
}

/// A value may join several paths with " + " (first a full path, rest
/// relative to its directory, e.g. `<crate>/README.md + src/lib.rs`).
#[cfg(any(feature = "full", test))]
pub(crate) fn source_paths(value: &str, base: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut anchor: Option<PathBuf> = None;
    for part in value
        .split(" + ")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let path = Path::new(part);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            anchor
                .as_ref()
                .map(|anchor| anchor.join(path))
                .filter(|candidate| candidate.exists())
                .or_else(|| base.map(|directory| directory.join(path)))
                .unwrap_or_else(|| path.to_path_buf())
        };
        if anchor.is_none() {
            anchor = resolved.parent().map(Path::to_path_buf);
        }
        out.push(resolved);
    }
    out
}

fn first_source(value: &str) -> &str {
    value.split(" + ").next().unwrap_or(value).trim()
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

/// Repoints a frozen asset's excerpt at its real source file/lines for
/// display, so the learner sees real source, not the opaque asset path.
pub fn relabel_for_display(
    mut excerpt: Excerpt,
    at_origin: Option<&str>,
) -> (Excerpt, Option<String>) {
    let Some((file, start)) = parse_at_origin(at_origin) else {
        return (excerpt, None);
    };
    excerpt.path = PathBuf::from(&file);
    for (index, line) in excerpt.lines.iter_mut().enumerate() {
        line.0 = start + index;
    }
    let label = match (excerpt.lines.first(), excerpt.lines.last()) {
        (Some((start, _)), Some((end, _))) if start != end => {
            format!("{file}:{start}-{end}")
        }
        (Some((start, _)), _) => format!("{file}:{start}"),
        _ => file,
    };
    (excerpt, Some(label))
}

/// Splits on the last colon, so a path with directories stays intact.
pub(crate) fn parse_at_origin(at_origin: Option<&str>) -> Option<(String, usize)> {
    let spec = at_origin?.trim();
    let (file, lines) = spec.rsplit_once(':')?;
    let start = lines.split('-').next()?.trim().parse().ok()?;
    (!file.trim().is_empty()).then(|| (file.trim().to_string(), start))
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

    let mut selected = Vec::new();
    let mut truncated = false;
    for line_number in start..=end {
        if selected.len() >= MAX_EXCERPT_LINES {
            truncated = true;
            break;
        }
        selected.push((line_number, file_lines[line_number - 1].to_string()));
    }

    if selected.is_empty() {
        bail!(
            "locator points outside `{}` ({} lines)",
            path.display(),
            file_lines.len()
        );
    }
    Ok(Excerpt {
        path: path.to_path_buf(),
        lines: selected,
        truncated,
    })
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
    fn relabel_for_display_uses_the_at_origin() {
        let excerpt = Excerpt {
            path: PathBuf::from("/ws/assets/30.rs"),
            lines: vec![
                (1, "a".to_string()),
                (2, "b".to_string()),
                (3, "c".to_string()),
            ],
            truncated: false,
        };
        let (excerpt, label) = relabel_for_display(excerpt, Some("src/caching.rs:106-120"));
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
    fn relabel_for_display_is_a_noop_without_provenance() {
        let excerpt = Excerpt {
            path: PathBuf::from("/src/foo.rs"),
            lines: vec![(10, "x".to_string())],
            truncated: false,
        };
        let (same, label) = relabel_for_display(excerpt.clone(), Some("just an insight"));
        assert_eq!(excerpt.path, same.path);
        assert_eq!(excerpt.lines, same.lines);
        assert_eq!(None, label);
        assert_eq!(None, relabel_for_display(excerpt, None).1);
    }

    #[test]
    fn parse_at_origin_splits_file_and_start_on_the_last_colon() {
        assert_eq!(
            Some(("src/caching.rs".to_string(), 46)),
            parse_at_origin(Some("src/caching.rs:46-66"))
        );
        assert_eq!(
            Some(("a.rs".to_string(), 1)),
            parse_at_origin(Some("a.rs:1"))
        );
        assert_eq!(None, parse_at_origin(Some("just an insight")));
        assert_eq!(None, parse_at_origin(None));
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
        let locator = deck.cards[0].at.as_deref().unwrap();
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
                "---\nsource: {} + src/lib.rs\n---\n\
                 ## q1\na1\n<!-- at: README.md:1-2 -->\n\
                 ## q2\na2\n<!-- at: src/lib.rs:3-4 -->\n",
                readme.display()
            ),
        );
        let deck = Deck::load(&deck_path).unwrap();
        let base = SourceBase::for_deck(&deck);

        assert_eq!(
            vec![(1, "r1".to_string()), (2, "r2".to_string())],
            base.excerpt(deck.cards[0].at.as_deref().unwrap())
                .unwrap()
                .lines
        );
        assert_eq!(
            vec![(3, "l3".to_string()), (4, "l4".to_string())],
            base.excerpt(deck.cards[1].at.as_deref().unwrap())
                .unwrap()
                .lines
        );
        assert!(base.excerpt("2").is_err());
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
    fn read_excerpt_whole_file_caps_long_sources() {
        let directory = tempfile::tempdir().unwrap();
        let body: String = (1..=100).map(|line| format!("line {line}\n")).collect();
        let path = write(directory.path(), "big.txt", &body);
        let excerpt = read_excerpt(&path, None).unwrap();
        assert_eq!(MAX_EXCERPT_LINES, excerpt.lines.len());
        assert!(excerpt.truncated);
    }

    #[test]
    fn source_paths_splits_plus_and_anchors_relative_parts() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(project.join("README.md"), "r").unwrap();
        std::fs::write(project.join("src/lib.rs"), "l").unwrap();

        let value = format!("{}/README.md + src/lib.rs", project.display());
        assert_eq!(
            vec![project.join("README.md"), project.join("src/lib.rs")],
            source_paths(&value, Some(directory.path()))
        );

        let one = project.join("src/lib.rs");
        assert_eq!(
            vec![one.clone()],
            source_paths(&one.to_string_lossy(), None)
        );
    }
}
