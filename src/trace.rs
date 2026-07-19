use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::{
    deck::{Deck, is_url},
    depth::Depth,
    scheduler::{Fsrs, Grade, Scheduler},
    store::Store,
};

/// Truncated (with a marker) beyond this, so a huge locator never floods the
/// screen.
const MAX_EXCERPT_LINES: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delta {
    Passed,
    Partial,
    Failed,
}

impl Delta {
    pub fn from_key(c: char) -> Option<Delta> {
        match c.to_ascii_lowercase() {
            'n' => Some(Delta::Passed),
            'p' => Some(Delta::Partial),
            'f' => Some(Delta::Failed),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Delta::Passed => "Got it",
            Delta::Partial => "Partly",
            Delta::Failed => "Missed it",
        }
    }

    /// A `Partial` schedules as FSRS `Hard` (a weak pass, resurfaces sooner); a
    /// `Failed` schedules as `Again` but never derails the walk.
    pub fn grade(self) -> Grade {
        match self {
            Delta::Passed => Grade::Pass,
            Delta::Partial => Grade::Partial,
            Delta::Failed => Grade::Fail,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub prompt: String,
    pub points: Vec<String>,
    pub givens: Vec<String>,
    pub note: Option<String>,
    pub locator: Option<String>,
    pub at_origin: Option<String>,
    pub card_id: String,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Trace {
    pub description: String,
    pub subject: String,
    pub source: Option<String>,
    pub checkpoints: Vec<Checkpoint>,
    pub deck_path: PathBuf,
    pub origin: Option<PathBuf>,
    base_dir: PathBuf,
    source_file: Option<PathBuf>,
}

impl Trace {
    pub fn from_deck(deck: &Deck) -> Result<Trace> {
        let description = deck
            .trace
            .clone()
            .ok_or_else(|| anyhow!("{} is not a trace: it declares no `trace:`", deck.subject))?;
        if deck.cards.is_empty() {
            bail!("the trace `{}` has no checkpoints", deck.subject);
        }
        // A checkpoint needs a stable id (the deck is stamped at open); an
        // unstamped card carries none and is skipped defensively.
        let checkpoints = deck
            .cards
            .iter()
            .filter_map(|c| {
                Some(Checkpoint {
                    prompt: c.front.clone(),
                    points: c.back.clone(),
                    givens: c.givens.clone(),
                    note: c.note.clone(),
                    locator: c.at.clone(),
                    at_origin: c.at_origin.clone(),
                    card_id: c.id()?,
                    line: c.line,
                })
            })
            .collect();
        let source = deck.sources.first().cloned();
        let (base_dir, source_file) = resolve_source(deck.path.parent(), source.as_deref());
        Ok(Trace {
            description,
            subject: deck.subject.clone(),
            source,
            checkpoints,
            deck_path: deck.path.clone(),
            origin: deck.source_root(),
            base_dir,
            source_file,
        })
    }

    /// Drawn from the checkpoints (which already paraphrase the source), so
    /// grading needs no source read.
    pub fn compression_rubric(&self) -> Vec<String> {
        self.checkpoints
            .iter()
            .flat_map(|cp| cp.points.iter().cloned())
            .collect()
    }

    pub fn excerpt(&self, checkpoint: &Checkpoint) -> Result<Excerpt> {
        let locator = checkpoint
            .locator
            .as_deref()
            .ok_or_else(|| anyhow!("this checkpoint has no `at:` locator to reveal"))?;
        excerpt_at(&self.base_dir, self.source_file.as_deref(), locator)
    }

    pub fn frozen_block(&self, checkpoint: &Checkpoint) -> Option<String> {
        checkpoint.at_origin.as_deref()?;
        let excerpt = self.excerpt(checkpoint).ok()?;
        Some(render_frozen_block(
            excerpt,
            checkpoint.at_origin.as_deref(),
        ))
    }

    pub fn lint_locators(&self) -> Vec<LocatorIssue> {
        let mut issues = Vec::new();
        let url_source = self.source.as_deref().is_some_and(is_url);
        for (i, cp) in self.checkpoints.iter().enumerate() {
            let Some(locator) = cp.locator.as_deref() else {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: "no `at:` locator — a walk can't reveal its source".to_string(),
                });
                continue;
            };
            if url_source {
                continue;
            }
            let (file, spec) = parse_locator(locator);
            let Some(path) =
                locator_path(&self.base_dir, self.source_file.as_deref(), file.as_deref())
            else {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!(
                        "locator `{locator}` gives only line numbers, but `source:` \
                         is not a single file — write it as `file:lines`"
                    ),
                });
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!(
                        "locator `{locator}` → `{}`: file not found or unreadable",
                        path.display()
                    ),
                });
                continue;
            };
            let Some(spec) = spec else { continue };
            let (start, end) = parse_line_range(&spec);
            let n = text.lines().count();
            if start > n {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!(
                        "locator `{locator}` starts at line {start}, but `{}` has only {n} \
                         lines — the source changed; re-point it",
                        path.display()
                    ),
                });
            } else if end > n {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!(
                        "locator `{locator}` ends at line {end}, but `{}` has only {n} lines \
                         — the excerpt is clamped short; re-point it",
                        path.display()
                    ),
                });
            }
        }
        issues
    }
}

/// `checkpoint` is a 0-based index that mirrors the deck's cards 1:1, so a
/// caller can map it back to a deck line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatorIssue {
    pub checkpoint: usize,
    pub message: String,
}

/// The directory under a workspace where a snapshotted trace's excerpts are
/// frozen.
pub(crate) const SNAPSHOT_DIR: &str = "assets";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Predict,
    Reveal,
    /// Every checkpoint walked; verification is the trace's separate
    /// AI-graded exam, not an ungraded compression step here.
    Done,
}

pub struct Walk {
    trace: Trace,
    current: usize,
    phase: Phase,
    predictions: Vec<String>,
    deltas: Vec<Option<Delta>>,
}

impl Walk {
    pub fn new(trace: Trace) -> Walk {
        let n = trace.checkpoints.len();
        Walk {
            trace,
            current: 0,
            phase: Phase::Predict,
            predictions: vec![String::new(); n],
            deltas: vec![None; n],
        }
    }

    pub fn trace(&self) -> &Trace {
        &self.trace
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn total(&self) -> usize {
        self.trace.checkpoints.len()
    }
    pub fn current_index(&self) -> usize {
        self.current
    }
    pub fn checkpoint(&self) -> Option<&Checkpoint> {
        self.trace.checkpoints.get(self.current)
    }

    /// No-op outside [`Phase::Predict`].
    pub fn predict(&mut self, text: String) {
        if self.phase != Phase::Predict {
            return;
        }
        if let Some(slot) = self.predictions.get_mut(self.current) {
            *slot = text;
        }
        self.phase = Phase::Reveal;
    }

    /// No-op outside [`Phase::Reveal`]. Updates `store` but does not save it
    /// (the caller saves).
    pub fn grade(&mut self, store: &mut Store, delta: Delta, now_ms: u64) {
        if self.phase != Phase::Reveal {
            return;
        }
        if let Some(checkpoint) = self.trace.checkpoints.get(self.current) {
            // The walk grades with no Session, so it's itself an
            // entry-creation site: write records before the schedule entry.
            store.ensure_records_raw(&checkpoint.card_id, &[]);
            let state = store.get_or_insert(&checkpoint.card_id, now_ms);
            Fsrs::default().apply(state, Depth::Recall, delta.grade(), now_ms, false);
        }
        self.deltas[self.current] = Some(delta);
        if self.current + 1 < self.trace.checkpoints.len() {
            self.current += 1;
            self.phase = Phase::Predict;
        } else {
            self.phase = Phase::Done;
        }
    }

    pub fn prediction(&self, i: usize) -> Option<&str> {
        self.predictions.get(i).map(String::as_str)
    }
    pub fn delta(&self, i: usize) -> Option<Delta> {
        self.deltas.get(i).copied().flatten()
    }

    pub fn summary(&self) -> Summary {
        let mut s = Summary::default();
        for (i, delta) in self.deltas.iter().enumerate() {
            match delta {
                Some(Delta::Passed) => s.passed += 1,
                Some(Delta::Partial) => {
                    s.partly += 1;
                    s.weak.push(i);
                }
                Some(Delta::Failed) => {
                    s.failed += 1;
                    s.weak.push(i);
                }
                None => {}
            }
        }
        s
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub passed: usize,
    pub partly: usize,
    pub failed: usize,
    /// Indices of checkpoints judged partly or failed (SRS resurfaces them
    /// sooner).
    pub weak: Vec<usize>,
}

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
    let p = if Path::new(source).is_absolute() {
        PathBuf::from(source)
    } else {
        deck_dir.join(source)
    };
    if p.is_file() {
        let base = p.parent().map(Path::to_path_buf).unwrap_or(deck_dir);
        (base, Some(p))
    } else {
        // A directory (or `.`); locators must name a file within it.
        (p, None)
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
        // A multi-file source (joined by ` + `) isn't itself a path, so the
        // base resolves from just the first file's directory.
        let first = deck.sources.first();
        let multi = first.is_some_and(|s| s.contains(" + "));
        let (base_dir, source_file) =
            resolve_source(deck.path.parent(), first.map(|s| first_source(s)));
        Self {
            base_dir,
            // With several source files a bare-line locator is ambiguous, so drop
            // the single-file shortcut and require `file:lines`.
            source_file: if multi { None } else { source_file },
        }
    }

    pub fn excerpt(&self, locator: &str) -> Result<Excerpt> {
        excerpt_at(&self.base_dir, self.source_file.as_deref(), locator)
    }
}

/// Walks up to the nearest project root (`Cargo.toml`, `.git`, etc.) so the
/// tutor can read the whole crate, not just the cited files.
pub(crate) fn project_root(sources: &[String], deck_dir: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = sources
        .iter()
        .filter(|s| !is_url(s))
        .flat_map(|s| source_paths(s, Some(deck_dir)))
        .filter(|p| p.exists())
        .map(|p| {
            if p.is_file() {
                p.parent().map(Path::to_path_buf).unwrap_or(p)
            } else {
                p
            }
        })
        .collect();
    dirs.sort();
    dirs.dedup();
    let base = common_ancestor(&dirs)?;
    Some(find_project_root(&base).unwrap_or(base))
}

/// A value may join several paths with " + " (first a full path, rest
/// relative to its directory, e.g. `<crate>/README.md + src/lib.rs`).
pub(crate) fn source_paths(value: &str, base: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut anchor: Option<PathBuf> = None;
    for part in value.split(" + ").map(str::trim).filter(|p| !p.is_empty()) {
        let p = Path::new(part);
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            anchor
                .as_ref()
                .map(|a| a.join(p))
                .filter(|candidate| candidate.exists())
                .or_else(|| base.map(|d| d.join(p)))
                .unwrap_or_else(|| p.to_path_buf())
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

fn common_ancestor(dirs: &[PathBuf]) -> Option<PathBuf> {
    let mut common = dirs.first()?.clone();
    for d in &dirs[1..] {
        while !d.starts_with(&common) {
            common = common.parent()?.to_path_buf();
        }
    }
    Some(common)
}

fn find_project_root(dir: &Path) -> Option<PathBuf> {
    const MARKERS: [&str; 5] = [
        "Cargo.toml",
        ".git",
        "package.json",
        "go.mod",
        "pyproject.toml",
    ];
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if MARKERS.iter().any(|m| d.join(m).exists()) {
            return Some(d.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

/// A locator is a single span, never comma-separated, so a stitched,
/// misleading excerpt is impossible.
fn parse_locator(locator: &str) -> (Option<String>, Option<String>) {
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

fn is_line_spec(s: &str) -> bool {
    let s = s.trim();
    match s.split_once('-') {
        Some((a, b)) => is_number(a) && is_number(b),
        None => is_number(s),
    }
}

fn is_number(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Parses a validated single range into inclusive `(start, end)` (a lone `N` is
/// `(N, N)`; a reversed range is normalized).
fn parse_line_range(spec: &str) -> (usize, usize) {
    let parse = |s: &str| s.trim().parse::<usize>().unwrap_or(1);
    let (a, b) = match spec.trim().split_once('-') {
        Some((a, b)) => (parse(a), parse(b)),
        None => {
            let n = parse(spec);
            (n, n)
        }
    };
    if a <= b { (a, b) } else { (b, a) }
}

/// Shared by [`excerpt_at`] and [`Trace::lint_locators`] so the two never
/// disagree.
fn locator_path(
    base_dir: &Path,
    source_file: Option<&Path>,
    file: Option<&str>,
) -> Option<PathBuf> {
    match source_file {
        Some(sf) => Some(sf.to_path_buf()),
        None => file.map(|f| resolve_under_base(base_dir, f)),
    }
}

/// A locator may be written relative to a project root ABOVE `base_dir`
/// (not just `base_dir` itself); a direct join would double the overlap.
pub(crate) fn resolve_under_base(base_dir: &Path, file: &str) -> PathBuf {
    let direct = base_dir.join(file);
    if direct.exists() {
        return direct;
    }
    let mut ancestor = base_dir.parent();
    while let Some(dir) = ancestor {
        let candidate = dir.join(file);
        if candidate.exists() {
            return candidate;
        }
        ancestor = dir.parent();
    }
    // Last resort: a basename search under `base_dir` (best-effort, first
    // match), for a locator that dropped/added a leading subdirectory.
    if let Some(name) = Path::new(file).file_name()
        && let Some(found) = find_under(base_dir, name)
    {
        return found;
    }
    direct
}

/// Skips version-control and build directories so a large tree stays cheap.
fn find_under(root: &Path, name: &std::ffi::OsStr) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let skip = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|b| b.starts_with('.') || matches!(b, "target" | "node_modules"));
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
    for (i, line) in excerpt.lines.iter_mut().enumerate() {
        line.0 = start + i;
    }
    let label = match (excerpt.lines.first(), excerpt.lines.last()) {
        (Some((a, _)), Some((b, _))) if a != b => format!("{file}:{a}-{b}"),
        (Some((a, _)), _) => format!("{file}:{a}"),
        _ => file,
    };
    (excerpt, Some(label))
}

/// The frozen asset is the anchor the tutor reasons from; live source is
/// only for context.
fn render_frozen_block(excerpt: Excerpt, at_origin: Option<&str>) -> String {
    let (excerpt, label) = relabel_for_display(excerpt, at_origin);
    let mut s = String::new();
    if let Some(label) = label {
        s.push_str(&label);
        s.push('\n');
    }
    for (n, text) in &excerpt.lines {
        s.push_str(&format!("{n}\t{text}\n"));
    }
    s
}

#[derive(Debug)]
pub struct Drift {
    pub line: usize,
    pub at: String,
    pub gone: bool,
}

/// A *moved* excerpt that's otherwise unchanged is NOT flagged (the block is
/// searched across the whole file).
pub fn drifted_cards(deck: &Deck) -> Vec<Drift> {
    let Some(origin_root) = deck.source_root() else {
        return Vec::new();
    };
    let source_base = SourceBase::for_deck(deck);
    let mut out = Vec::new();
    for card in &deck.cards {
        let (Some(at), Some(at_origin)) = (card.at.as_deref(), card.at_origin.as_deref()) else {
            continue;
        };
        let Some((file, _)) = parse_at_origin(Some(at_origin)) else {
            continue;
        };
        let Ok(frozen) = source_base.excerpt(at) else {
            continue;
        };
        match std::fs::read_to_string(origin_root.join(&file)) {
            Err(_) => out.push(Drift {
                line: card.line,
                at: at_origin.to_string(),
                gone: true,
            }),
            Ok(live) if !excerpt_occurs_in(&frozen, &live) => out.push(Drift {
                line: card.line,
                at: at_origin.to_string(),
                gone: false,
            }),
            Ok(_) => {}
        }
    }
    out
}

/// Trailing whitespace is ignored, so reformatted line endings don't read
/// as drift.
fn excerpt_occurs_in(frozen: &Excerpt, live: &str) -> bool {
    let block = frozen
        .lines
        .iter()
        .map(|(_, t)| t.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    if block.trim().is_empty() {
        return true;
    }
    let live_norm = live
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    live_norm.contains(&block)
}

pub fn frozen_excerpt_block(
    at: &str,
    at_origin: Option<&str>,
    source_base: &SourceBase,
) -> Option<String> {
    at_origin?;
    let excerpt = source_base.excerpt(at).ok()?;
    Some(render_frozen_block(excerpt, at_origin))
}

/// Splits on the LAST colon, so a path with directories stays intact.
pub(crate) fn parse_at_origin(at_origin: Option<&str>) -> Option<(String, usize)> {
    let spec = at_origin?.trim();
    let (file, lines) = spec.rsplit_once(':')?;
    let start = lines.split('-').next()?.trim().parse().ok()?;
    (!file.trim().is_empty()).then(|| (file.trim().to_string(), start))
}

fn excerpt_at(base_dir: &Path, source_file: Option<&Path>, locator: &str) -> Result<Excerpt> {
    let (file, spec) = parse_locator(locator);
    // If `base_dir` no longer exists, a plain join fabricates a misleading
    // phantom path; fail on the real cause (the source base is gone) instead.
    let joins_onto_base =
        source_file.is_none() && file.as_deref().is_some_and(|f| !Path::new(f).is_absolute());
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
        .map_err(|e| anyhow!("cannot read the source `{}`: {e}", path.display()))?;
    let file_lines: Vec<&str> = text.lines().collect();

    // The span to take, clamped to the file so a stale line number never panics.
    let (start, end) = match spec {
        None => (1, file_lines.len()),
        Some(spec) => parse_line_range(spec),
    };
    let start = start.max(1);
    let end = end.min(file_lines.len());

    let mut selected: Vec<(usize, String)> = Vec::new();
    let mut truncated = false;
    for no in start..=end {
        if selected.len() >= MAX_EXCERPT_LINES {
            truncated = true;
            break;
        }
        selected.push((no, file_lines[no - 1].to_string()));
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

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        // Mirrors production stamping (a trace's per-checkpoint SRS keys on
        // the card token); source files are left untouched.
        if name.ends_with(".md") {
            let _ = crate::stamp::stamp_deck(&path);
        }
        path
    }

    #[test]
    fn delta_label_names_each_grade_for_the_learner() {
        assert_eq!("Got it", Delta::Passed.label());
        assert_eq!("Partly", Delta::Partial.label());
        assert_eq!("Missed it", Delta::Failed.label());
    }

    #[test]
    fn excerpt_at_resolves_a_file_and_line_locator() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "notes.md", "alpha\nbeta\ngamma\ndelta\n");
        let ex = excerpt_at(dir.path(), None, "notes.md:2-3").unwrap();
        assert_eq!(
            vec![(2, "beta".to_string()), (3, "gamma".to_string())],
            ex.lines
        );
    }

    #[test]
    fn excerpt_at_resolves_a_line_only_locator_against_the_single_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "notes.md", "alpha\nbeta\ngamma\n");
        let ex = excerpt_at(dir.path(), Some(&file), "2").unwrap();
        assert_eq!(vec![(2, "beta".to_string())], ex.lines);
    }

    #[test]
    fn excerpt_at_rejects_a_line_only_locator_without_a_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = excerpt_at(dir.path(), None, "2-3").unwrap_err();
        assert!(format!("{err:#}").contains("only line numbers"));
    }

    #[test]
    fn excerpt_at_single_file_source_ignores_a_redundant_file_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/executor")).unwrap();
        let file = write(dir.path(), "src/executor/env.rs", "a\nb\nc\nd\n");
        let base_dir = file.parent().unwrap();
        let ex = excerpt_at(base_dir, Some(&file), "src/executor/env.rs:2-3").unwrap();
        assert_eq!(vec![(2, "b".to_string()), (3, "c".to_string())], ex.lines);
    }

    #[test]
    fn excerpt_at_reports_a_missing_source_base_clearly() {
        let base = std::env::temp_dir().join(format!("alix-nobase-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let err = format!(
            "{:#}",
            excerpt_at(&base, None, "src/lib.rs:1-3").unwrap_err()
        );
        assert!(err.contains(&base.display().to_string()), "{err}");
        assert!(err.contains("does not exist"), "{err}");
        assert!(
            !err.contains("src/lib.rs"),
            "must not name a locator joined onto the missing base: {err}"
        );
    }

    #[test]
    fn excerpt_at_resolves_a_crate_root_locator_against_a_subdir_source() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/executor")).unwrap();
        write(dir.path(), "src/executor/local_vm.rs", "a\nb\nc\nd\n");
        let base_dir = dir.path().join("src/executor");
        let ex = excerpt_at(&base_dir, None, "src/executor/local_vm.rs:2-3").unwrap();
        assert_eq!(vec![(2, "b".to_string()), (3, "c".to_string())], ex.lines);
    }

    #[test]
    fn excerpt_at_recovers_a_dropped_subdirectory_via_basename_search() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        write(dir.path(), "src/chapter.md", "a\nb\nc\nd\n");
        let base_dir = dir.path().to_path_buf();
        let ex = excerpt_at(&base_dir, None, "chapter.md:2-3").unwrap();
        assert_eq!(vec![(2, "b".to_string()), (3, "c".to_string())], ex.lines);
    }

    #[test]
    fn relabel_for_display_uses_the_at_origin() {
        let ex = Excerpt {
            path: PathBuf::from("/ws/assets/30.rs"),
            lines: vec![
                (1, "a".to_string()),
                (2, "b".to_string()),
                (3, "c".to_string()),
            ],
            truncated: false,
        };
        let (ex, label) = relabel_for_display(ex, Some("src/caching.rs:106-120"));
        assert_eq!("src/caching.rs", ex.path.to_str().unwrap());
        assert_eq!(
            vec![
                (106, "a".to_string()),
                (107, "b".to_string()),
                (108, "c".to_string())
            ],
            ex.lines
        );
        assert_eq!(Some("src/caching.rs:106-108".to_string()), label);
    }

    #[test]
    fn relabel_for_display_is_a_noop_without_provenance() {
        let ex = Excerpt {
            path: PathBuf::from("/src/foo.rs"),
            lines: vec![(10, "x".to_string())],
            truncated: false,
        };
        let (ex2, label) = relabel_for_display(ex.clone(), Some("just an insight"));
        assert_eq!(ex.path, ex2.path);
        assert_eq!(ex.lines, ex2.lines);
        assert_eq!(None, label);
        assert_eq!(None, relabel_for_display(ex, None).1);
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
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "notes.md", "one\ntwo\nthree\nfour\n");
        let deck_path = dir.path().join("facts.md");
        std::fs::write(
            &deck_path,
            "---\nsource: notes.md\n---\n## q\na\n<!-- at: notes.md:2-3 -->\n",
        )
        .unwrap();
        let deck = crate::deck::Deck::load(&deck_path).unwrap();
        let base = SourceBase::for_deck(&deck);
        let locator = deck.cards[0].at.as_deref().expect("card carries % at:");
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
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", "r1\nr2\nr3\n");
        std::fs::create_dir(dir.path().join("src")).unwrap();
        write(dir.path(), "src/lib.rs", "l1\nl2\nl3\nl4\n");
        let readme = dir.path().join("README.md");
        let deck_path = dir.path().join("facts.md");
        std::fs::write(
            &deck_path,
            format!(
                "---\nsource: {} + src/lib.rs\n---\n\
                 ## q1\na1\n<!-- at: README.md:1-2 -->\n\
                 ## q2\na2\n<!-- at: src/lib.rs:3-4 -->\n",
                readme.display()
            ),
        )
        .unwrap();
        let deck = crate::deck::Deck::load(&deck_path).unwrap();
        let base = SourceBase::for_deck(&deck);

        let readme_at = deck.cards[0].at.as_deref().unwrap();
        assert_eq!(
            vec![(1, "r1".to_string()), (2, "r2".to_string())],
            base.excerpt(readme_at).unwrap().lines
        );
        let lib_at = deck.cards[1].at.as_deref().unwrap();
        assert_eq!(
            vec![(3, "l3".to_string()), (4, "l4".to_string())],
            base.excerpt(lib_at).unwrap().lines
        );
        assert!(base.excerpt("2").is_err());
    }

    #[test]
    fn delta_keys_and_grades() {
        assert_eq!(Some(Delta::Passed), Delta::from_key('N'));
        assert_eq!(Some(Delta::Partial), Delta::from_key('p'));
        assert_eq!(Some(Delta::Failed), Delta::from_key('f'));
        assert_eq!(None, Delta::from_key('x'));
        assert_eq!(Grade::Pass, Delta::Passed.grade());
        assert_eq!(Grade::Partial, Delta::Partial.grade());
        assert_eq!(Grade::Fail, Delta::Failed.grade());
    }

    #[test]
    fn walking_a_trace_drills_but_does_not_master_it() {
        use crate::{deck::DeckState, store::Store};
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.rs", "a\nb\nc\n");
        let deck_path = dir.path().join("t.md");
        std::fs::write(
            &deck_path,
            format!(
                "---\ntrace: how a moves\nsource: {}\n---\n## what happens?\nit advances\n<!-- at: 1-2 -->\n",
                dir.path().join("src.rs").display()
            ),
        )
        .unwrap();
        // Stamped at open in production, so the checkpoint carries a token id.
        crate::stamp::stamp_deck(&deck_path).unwrap();
        let deck = crate::deck::Deck::load(&deck_path).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        assert!(!store.deck_mastered(&deck.subject));
        assert_eq!(DeckState::NotStarted, deck.state(&store));

        // Manually graduates the checkpoint (FSRS `Review`) to satisfy the
        // deck's unlock gate (every card graduated).
        let card0 = Trace::from_deck(&deck).unwrap().checkpoints[0]
            .card_id
            .clone();
        let mut walk = Walk::new(Trace::from_deck(&deck).unwrap());
        walk.predict("p".to_string());
        walk.grade(&mut store, Delta::Passed, 1);
        assert_eq!(Phase::Done, walk.phase());
        if let Some(f) = store.get_or_insert(&card0, 0).recall.as_mut() {
            f.state = 2; // Review state (graduated)
        }

        assert!(!store.deck_mastered(&deck.subject));
        assert_eq!(DeckState::ExamDue, deck.state(&store));

        store.set_deck_mastered(&deck.subject, 99);
        assert_eq!(DeckState::Finished, deck.state(&store));
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
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "f.txt", "a\nb\nc\nd\ne\n");
        let ex = read_excerpt(&path, Some("2-4")).unwrap();
        assert_eq!(
            vec![
                (2, "b".to_string()),
                (3, "c".to_string()),
                (4, "d".to_string())
            ],
            ex.lines
        );
        assert!(!ex.truncated);
        let ex = read_excerpt(&path, Some("1")).unwrap();
        assert_eq!(vec![(1, "a".to_string())], ex.lines);
    }

    #[test]
    fn read_excerpt_clamps_out_of_range_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "f.txt", "a\nb\nc\n");
        let ex = read_excerpt(&path, Some("2-99")).unwrap();
        assert_eq!(vec![(2, "b".to_string()), (3, "c".to_string())], ex.lines);
        assert!(read_excerpt(&path, Some("99")).is_err());
    }

    #[test]
    fn read_excerpt_whole_file_caps_long_sources() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let path = write(dir.path(), "big.txt", &body);
        let ex = read_excerpt(&path, None).unwrap();
        assert_eq!(MAX_EXCERPT_LINES, ex.lines.len());
        assert!(ex.truncated);
    }

    fn trace_deck(dir: &Path) -> Deck {
        write(dir, "source.txt", "first\nsecond\nthird\nfourth\n");
        let path = write(
            dir,
            "t.md",
            "---\ntrace: how it works\n\
             source: source.txt\n---\n\
             ## Predict the first hop\n\
             <!-- given: line — the current input line -->\n\
             it reads the first line\n\
             <!-- at: 1 -->\n\
             > the entry point\n\
             ## Predict the second hop\n\
             it reads lines two and three\n\
             <!-- at: 2-3 -->\n",
        );
        Deck::load(&path).unwrap()
    }

    #[test]
    fn from_deck_builds_checkpoints_and_rejects_non_traces() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        assert_eq!("how it works", trace.description);
        assert_eq!(2, trace.checkpoints.len());
        assert_eq!("Predict the first hop", trace.checkpoints[0].prompt);
        assert_eq!(
            vec!["line — the current input line".to_string()],
            trace.checkpoints[0].givens
        );
        assert!(trace.checkpoints[1].givens.is_empty());
        assert_eq!(Some("1".to_string()), trace.checkpoints[0].locator);
        assert_eq!(
            Some("the entry point".to_string()),
            trace.checkpoints[0].note
        );

        let plain = write(dir.path(), "p.md", "## q\na\n");
        let err = Trace::from_deck(&Deck::load(&plain).unwrap()).unwrap_err();
        assert!(format!("{err:#}").contains("not a trace"));
    }

    #[test]
    fn excerpt_reads_live_from_the_single_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(vec![(1, "first".to_string())], ex.lines);
        let ex = trace.excerpt(&trace.checkpoints[1]).unwrap();
        assert_eq!(
            vec![(2, "second".to_string()), (3, "third".to_string())],
            ex.lines
        );
    }

    #[test]
    fn line_only_locator_needs_a_single_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: .\n---\n## q\na\n<!-- at: 1 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let err = trace.excerpt(&trace.checkpoints[0]).unwrap_err();
        assert!(format!("{err:#}").contains("not a single file"));
    }

    #[test]
    fn lint_locators_passes_a_valid_trace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\nfour\nfive\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: src.txt\n---\n## q\na\n<!-- at: 2-3 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        assert!(trace.lint_locators().is_empty());
    }

    #[test]
    fn lint_locators_flags_a_start_past_eof() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: src.txt\n---\n## q\na\n<!-- at: 5-6 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert_eq!(0, issues[0].checkpoint);
        assert!(issues[0].message.contains("only 3"));
    }

    /// A range whose end runs past EOF is silently clamped at walk time, so
    /// `check` flags it too.
    #[test]
    fn lint_locators_flags_a_clamped_end() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: src.txt\n---\n## q\na\n<!-- at: 2-9 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("clamped"));
    }

    #[test]
    fn lint_locators_flags_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: .\n---\n## q\na\n<!-- at: nope.rs:1-2 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("not found"));
    }

    #[test]
    fn lint_locators_flags_a_missing_locator() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: src.txt\n---\n## q\na\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("no `at:`"));
    }

    #[test]
    fn lint_locators_flags_line_only_without_a_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: .\n---\n## q\na\n<!-- at: 1 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("not a single file"));
    }

    #[test]
    fn walk_runs_predict_reveal_grade_to_done() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let card0 = trace.checkpoints[0].card_id.clone();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace);

        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(2, walk.total());

        walk.predict("my guess".to_string());
        assert_eq!(Phase::Reveal, walk.phase());
        assert_eq!(Some("my guess"), walk.prediction(0));
        walk.grade(&mut store, Delta::Passed, 1000);
        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(1, walk.current_index());
        assert!(store.get(&card0).unwrap().recall.is_some());

        let card1 = walk.checkpoint().unwrap().card_id.clone();
        walk.predict(String::new());
        walk.grade(&mut store, Delta::Failed, 1001);
        assert_eq!(Phase::Done, walk.phase());
        assert_eq!(0, store.get(&card1).unwrap().streak);

        let summary = walk.summary();
        assert_eq!(1, summary.passed);
        assert_eq!(1, summary.failed);
        assert_eq!(vec![1], summary.weak);
    }

    #[test]
    fn a_trace_walk_grade_creates_an_entry_with_records() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let card0 = trace.checkpoints[0].card_id.clone();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        assert!(store.get(&card0).is_none(), "no entry before the grade");
        let mut walk = Walk::new(trace);
        walk.predict("guess".to_string());
        walk.grade(&mut store, Delta::Passed, 1000);
        assert!(store.get(&card0).is_some(), "the grade created the entry");
        let rec = store.records(&card0).expect("records exist alongside it");
        assert_eq!(crate::store::FP_VERSION, rec.version);
        assert!(rec.holes.is_empty(), "a trace card is a plain card");
    }

    #[test]
    fn partly_records_a_weak_success_on_a_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace);
        let card0 = walk.checkpoint().unwrap().card_id.clone();
        walk.predict("guess".to_string());
        walk.grade(&mut store, Delta::Partial, 1000);
        let state = store.get(&card0).unwrap();
        assert!(state.recall.is_some());
        assert_eq!(1, state.total_passes);
        assert_eq!(1, walk.summary().partly);
    }

    #[test]
    fn grade_is_a_noop_outside_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace);
        walk.grade(&mut store, Delta::Passed, 1000);
        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(0, walk.current_index());
        assert!(store.is_empty());
    }

    #[test]
    fn project_root_walks_up_to_the_crate_root() {
        let dir = tempfile::tempdir().unwrap();
        let crate_dir = dir.path().join("mycrate");
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(crate_dir.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        std::fs::write(crate_dir.join("README.md"), "# x\n").unwrap();
        std::fs::write(crate_dir.join("src/lib.rs"), "// lib\n").unwrap();
        let s = |p: PathBuf| p.to_string_lossy().into_owned();

        let both = vec![
            s(crate_dir.join("README.md")),
            s(crate_dir.join("src/lib.rs")),
        ];
        assert_eq!(Some(crate_dir.clone()), project_root(&both, dir.path()));

        let only = vec![s(crate_dir.join("src/lib.rs"))];
        assert_eq!(Some(crate_dir.clone()), project_root(&only, dir.path()));

        assert_eq!(
            None,
            project_root(&["https://example.com".to_string()], dir.path())
        );
    }

    #[test]
    fn source_paths_splits_plus_and_anchors_relative_parts() {
        let dir = tempfile::tempdir().unwrap();
        let crate_dir = dir.path().join("crate");
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(crate_dir.join("README.md"), "r").unwrap();
        std::fs::write(crate_dir.join("src/lib.rs"), "l").unwrap();

        let value = format!("{}/README.md + src/lib.rs", crate_dir.display());
        assert_eq!(
            vec![crate_dir.join("README.md"), crate_dir.join("src/lib.rs")],
            source_paths(&value, Some(dir.path()))
        );

        let one = crate_dir.join("src/lib.rs");
        assert_eq!(
            vec![one.clone()],
            source_paths(&one.to_string_lossy(), None)
        );
    }

    #[cfg(feature = "full")]
    fn snapshot_workspace(root: &Path) -> PathBuf {
        std::fs::create_dir_all(root.join("src")).unwrap();
        write(&root.join("src"), "a.rs", "alpha\nbeta\ngamma\n");
        write(&root.join("src"), "b.rs", "one\ntwo\n");
        std::fs::create_dir_all(root.join("ws")).unwrap();
        write(
            &root.join("ws"),
            "alix.toml",
            "title = \"W\"\n\n[defaults]\n",
        );
        write(
            &root.join("ws"),
            "t.md",
            "---\ntrace: how it works\n\
             source: ../src\n---\n\
             ## hop 1\nit reads a\n<!-- at: a.rs:2-3 -->\n\
             ## hop 2\nit reads b\n<!-- at: b.rs:1 -->\n",
        )
    }

    #[cfg(feature = "full")]
    #[test]
    fn drifted_cards_flags_a_changed_or_missing_excerpt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = snapshot_workspace(root);
        crate::trace_ai::snapshot(&Deck::load(&deck_path).unwrap(), 0, None).unwrap();

        assert!(drifted_cards(&Deck::load(&deck_path).unwrap()).is_empty());

        std::fs::write(root.join("src/a.rs"), "alpha\nCHANGED\nLINES\n").unwrap();
        let d = drifted_cards(&Deck::load(&deck_path).unwrap());
        assert_eq!(1, d.len(), "{d:?}");
        assert!(!d[0].gone);
        assert_eq!("a.rs:2-3", d[0].at);

        std::fs::remove_file(root.join("src/a.rs")).unwrap();
        let d = drifted_cards(&Deck::load(&deck_path).unwrap());
        assert!(d.iter().any(|x| x.gone && x.at == "a.rs:2-3"), "{d:?}");
    }
}
