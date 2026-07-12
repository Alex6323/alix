//! Traces — guided predict-and-verify walks along a path through a source.
//!
//! A trace deck (`% trace:` + a sequence of checkpoint cards, each with an
//! `% at:` locator into a `% source:`) is walked hop by hop: at each checkpoint
//! you commit a **prediction**, then the real **excerpt** from the source is
//! revealed alongside the key points a good prediction should hit, and you
//! judge the **delta** (passed / partly / failed). The miss is recorded for SRS —
//! a weak edge resurfaces sooner — but never derails the chain; you advance
//! from the revealed truth. After the last hop you **compress** the whole path
//! into a couple of sentences, which is the trace's own exam.
//!
//! This module is the frontend-agnostic engine: it builds the [`Trace`] from a
//! [`Deck`], resolves each locator to a live [`Excerpt`] (read fresh from the
//! source, the oracle), and drives the [`Walk`] state machine + per-checkpoint
//! scheduling. The web walk is a thin reader over it. Grading is
//! self-judged and offline — no model calls — so the mechanic can be validated
//! cheaply; live Claude grading (`--grade`) is a later layer.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::{
    deck::{Deck, is_url},
    depth::Depth,
    scheduler::{Fsrs, Grade, Scheduler},
    store::Store,
};

/// Largest excerpt read for one checkpoint, in lines. A locator spanning more
/// than this is truncated (with a marker) so a huge range never floods the
/// screen.
const MAX_EXCERPT_LINES: usize = 60;

/// The learner's self-judged gap between their prediction and the revealed
/// truth at a checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delta {
    /// The prediction covered the key points ("passed it").
    Passed,
    /// Partly right — something important was missing or wrong.
    Partial,
    /// The prediction missed the point ("failed").
    Failed,
}

impl Delta {
    /// The single-letter answer a learner types to record this delta.
    pub fn from_key(c: char) -> Option<Delta> {
        match c.to_ascii_lowercase() {
            'n' => Some(Delta::Passed),
            'p' => Some(Delta::Partial),
            'f' => Some(Delta::Failed),
            _ => None,
        }
    }

    /// The label shown to the learner.
    pub fn label(self) -> &'static str {
        match self {
            Delta::Passed => "Got it",
            Delta::Partial => "Partly",
            Delta::Failed => "Missed it",
        }
    }

    /// How this delta schedules the checkpoint, sharing the review grades. A
    /// passed hop advances (and fades); a **partly** one maps to FSRS `Hard` —
    /// a weak pass that resurfaces sooner than a full Good; a **failed** one
    /// resets to FSRS `Again` — recorded, not punished (the walk still continues).
    pub fn grade(self) -> Grade {
        match self {
            Delta::Passed => Grade::Pass,
            Delta::Partial => Grade::Partial,
            Delta::Failed => Grade::Fail,
        }
    }
}

/// One hop of a trace: the predict prompt, the key points a good prediction
/// should hit (the rubric, shown on reveal), an optional connective insight,
/// and the locator into the real source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// The predict prompt — the card front; you commit a guess before anything
    /// reveals.
    pub prompt: String,
    /// The key points the revealed truth makes — the card's back lines.
    pub points: Vec<String>,
    /// Named "givens" (`% given:` lines): off-screen symbols the question leans
    /// on, shown as a list under the prompt before predicting.
    pub givens: Vec<String>,
    /// The connective insight shown after the reveal (the card note).
    pub note: Option<String>,
    /// The `% at:` locator into the source, if the checkpoint declares one.
    pub locator: Option<String>,
    /// The frozen `% at:`'s ` from <file>:<lines>` origin provenance, if any —
    /// drives display relabeling and tutor grounding.
    pub at_origin: Option<String>,
    /// The card's identity hash — the key its per-checkpoint SRS hangs off.
    pub card_id: u64,
    /// The 1-based line of the checkpoint's front in the deck file, so an
    /// ask-Claude "save note" can append a `!` line to the right checkpoint.
    pub line: usize,
}

/// A trace built from a deck: what it walks, the ordered checkpoints, and where
/// their locators resolve.
#[derive(Clone, Debug)]
pub struct Trace {
    /// What the trace walks (`% trace:`) — a path description ("how X becomes
    /// Y").
    pub description: String,
    /// The deck's subject (its file name) — keys the trace's mastery in the
    /// store, just like a fact deck's exam mastery.
    pub subject: String,
    /// The path origin (`% source:`), shown to the learner. `None` if the deck
    /// declares none (locators then need an explicit `file:` part and a base).
    pub source: Option<String>,
    /// The checkpoints, in file order — the path, walked top to bottom.
    pub checkpoints: Vec<Checkpoint>,
    /// The deck file this trace was loaded from — for appending an ask-Claude
    /// "save note" to a checkpoint.
    pub deck_path: PathBuf,
    /// The live source root the tutor grounds in (the deck's `% origin:` for a
    /// frozen trace, else its `% source:` project root). `None` when there's no
    /// local source.
    pub origin: Option<PathBuf>,
    /// Directory a locator's `file:` part resolves against.
    base_dir: PathBuf,
    /// The single source file, when `% source:` is one file — then a locator
    /// may omit the filename and give only line numbers.
    source_file: Option<PathBuf>,
}

impl Trace {
    /// Builds a trace from a loaded deck. Errors if the deck is not a trace (no
    /// `% trace:`) or has no checkpoints.
    pub fn from_deck(deck: &Deck) -> Result<Trace> {
        let description = deck
            .trace
            .clone()
            .ok_or_else(|| anyhow!("{} is not a trace: it declares no `% trace:`", deck.subject))?;
        if deck.cards.is_empty() {
            bail!("the trace `{}` has no checkpoints", deck.subject);
        }
        let checkpoints = deck
            .cards
            .iter()
            .map(|c| Checkpoint {
                prompt: c.front.clone(),
                points: c.back.clone(),
                givens: c.givens.clone(),
                note: c.note.clone(),
                locator: c.at.clone(),
                at_origin: c.at_origin.clone(),
                card_id: c.id(),
                line: c.line,
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

    /// The rubric the trace **exam** grades a learner's compression against —
    /// every checkpoint's key points, in path order. The exam asks the learner
    /// to retrace the whole path ([`description`](Trace::description)) in a
    /// couple of sentences and judges whether that re-derives these points; it's
    /// drawn from the checkpoints (which already paraphrase the real source), so
    /// grading needs no source read.
    pub fn compression_rubric(&self) -> Vec<String> {
        self.checkpoints
            .iter()
            .flat_map(|cp| cp.points.iter().cloned())
            .collect()
    }

    /// Reads the live excerpt for a checkpoint's locator from the source.
    /// Errors when the checkpoint has no `% at:`, the locator can't be
    /// resolved to a file (e.g. a URL source, or a line-only locator
    /// without a single source file), or the file can't be read.
    pub fn excerpt(&self, checkpoint: &Checkpoint) -> Result<Excerpt> {
        let locator = checkpoint
            .locator
            .as_deref()
            .ok_or_else(|| anyhow!("this checkpoint has no `% at:` locator to reveal"))?;
        excerpt_at(&self.base_dir, self.source_file.as_deref(), locator)
    }

    /// The inline frozen-excerpt block for a checkpoint's tutor prompt, or `None`
    /// for a live (non-frozen) checkpoint or an unreadable excerpt.
    pub fn frozen_block(&self, checkpoint: &Checkpoint) -> Option<String> {
        checkpoint.at_origin.as_deref()?;
        let excerpt = self.excerpt(checkpoint).ok()?;
        Some(render_frozen_block(
            excerpt,
            checkpoint.at_origin.as_deref(),
        ))
    }

    /// Validates every checkpoint's `% at:` locator against the live source, for
    /// `alix check`. Returns one [`LocatorIssue`] per problem (empty = all
    /// resolve): a checkpoint with no locator, a `file:` that doesn't exist, a
    /// line range past the end of the file (the drift symptom — the source
    /// shrank or moved), or a line-only locator without a single-file source. A
    /// URL `% source:` has no local line ranges, so its locators are skipped.
    pub fn lint_locators(&self) -> Vec<LocatorIssue> {
        let mut issues = Vec::new();
        let url_source = self.source.as_deref().is_some_and(is_url);
        for (i, cp) in self.checkpoints.iter().enumerate() {
            let Some(locator) = cp.locator.as_deref() else {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: "no `% at:` locator — a walk can't reveal its source".to_string(),
                });
                continue;
            };
            if url_source {
                continue; // a remote source has no local line ranges to check
            }
            let (file, spec) = parse_locator(locator);
            let Some(path) =
                locator_path(&self.base_dir, self.source_file.as_deref(), file.as_deref())
            else {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!(
                        "locator `{locator}` gives only line numbers, but `% source:` \
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
            let Some(spec) = spec else { continue }; // whole-file locator: always valid
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

/// A problem `alix check` found with a checkpoint's `% at:` locator (see
/// [`Trace::lint_locators`]). `checkpoint` is the 0-based index into the path —
/// and into the deck's cards, which a trace's checkpoints mirror 1:1 — so the
/// caller can map it back to a deck line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatorIssue {
    /// Which checkpoint (0-based) the issue is on.
    pub checkpoint: usize,
    /// The problem, ready to print.
    pub message: String,
}

/// The directory under a workspace where a snapshotted trace's excerpts are
/// frozen.
pub(crate) const SNAPSHOT_DIR: &str = "assets";

/// The phase of a [`Walk`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Awaiting the learner's prediction for the current checkpoint.
    Predict,
    /// The excerpt + key points are shown; awaiting the self-judged delta.
    Reveal,
    /// Every checkpoint walked — the drill is done. Verification (and mastery)
    /// is the trace's **exam**: retracing the whole path in a couple of
    /// sentences, AI-graded (`alix exam <trace>` / the picker's "Take exam" /
    /// the walk's capstone). The walk itself no longer asks for an (ungraded)
    /// compression.
    Done,
}

/// One in-progress walk of a trace — a small frontend-agnostic state machine.
/// The CLI and the web walk surface drive it: show the current checkpoint,
/// take a [`predict`](Walk::predict), reveal the [`excerpt`](Trace::excerpt),
/// and take the self-judged [`grade`](Walk::grade) (which schedules the
/// checkpoint). After the last checkpoint the walk is [`Phase::Done`] — the
/// drill is complete; mastery comes from the trace's separate AI-graded **exam**
/// (the compression), not from the walk.
pub struct Walk {
    trace: Trace,
    current: usize,
    phase: Phase,
    predictions: Vec<String>,
    deltas: Vec<Option<Delta>>,
}

impl Walk {
    /// Starts a walk of `trace`. Checkpoints are scheduled with the FSRS scheduler.
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
    /// The 0-based index of the checkpoint being walked.
    pub fn current_index(&self) -> usize {
        self.current
    }
    /// The checkpoint being walked, or `None` once past the last one.
    pub fn checkpoint(&self) -> Option<&Checkpoint> {
        self.trace.checkpoints.get(self.current)
    }

    /// Commits the learner's prediction for the current checkpoint and moves to
    /// the reveal. No-op outside [`Phase::Predict`].
    pub fn predict(&mut self, text: String) {
        if self.phase != Phase::Predict {
            return;
        }
        if let Some(slot) = self.predictions.get_mut(self.current) {
            *slot = text;
        }
        self.phase = Phase::Reveal;
    }

    /// Records the self-judged delta for the current checkpoint, schedules it
    /// in `store`, and advances — to the next checkpoint's [`Phase::Predict`],
    /// or to [`Phase::Done`] after the last one. No-op outside [`Phase::Reveal`].
    /// The store is updated but not saved (the caller saves).
    ///
    /// The walk is the **drill**; it no longer masters the trace. Mastery is the
    /// trace's separate AI-graded **exam** (the compression) — passing it sets
    /// `deck_mastered`, exactly like a fact deck.
    pub fn grade(&mut self, store: &mut Store, delta: Delta, now_ms: u64) {
        if self.phase != Phase::Reveal {
            return;
        }
        if let Some(checkpoint) = self.trace.checkpoints.get(self.current) {
            let state = store.get_or_insert(checkpoint.card_id, now_ms);
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

    /// The prediction typed at checkpoint `i`, if any.
    pub fn prediction(&self, i: usize) -> Option<&str> {
        self.predictions.get(i).map(String::as_str)
    }
    /// The judged delta for checkpoint `i`, once it has been graded.
    pub fn delta(&self, i: usize) -> Option<Delta> {
        self.deltas.get(i).copied().flatten()
    }

    /// A tally of the deltas recorded so far.
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

/// The outcome of a walk: how the checkpoints landed and which were weak.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub passed: usize,
    pub partly: usize,
    pub failed: usize,
    /// 0-based indices of the checkpoints judged partly or failed — the weak
    /// edges that SRS will resurface sooner.
    pub weak: Vec<usize>,
}

/// A live excerpt read from a source for a checkpoint reveal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Excerpt {
    /// The file it was read from.
    pub path: PathBuf,
    /// The selected lines as `(1-based line number, content)`, contiguous and
    /// in order — a locator is a single span, so an excerpt never has gaps.
    pub lines: Vec<(usize, String)>,
    /// Whether the selection was cut to [`MAX_EXCERPT_LINES`].
    pub truncated: bool,
}

/// Resolves a `% source:` to the base directory a locator's `file:` part joins
/// onto, and the single source file (when the source is one file, so a locator
/// may omit the filename). A URL or absent source yields the deck's own folder
/// as the base and no source file.
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

/// The source base a fact deck's per-card `% at:` citations resolve against,
/// computed once from the deck's `% source:` so a frontend can read a card's
/// cited excerpt on reveal without re-loading the deck. Mirrors how a [`Trace`]
/// resolves its checkpoint locators — the same machinery, for plain fact cards.
#[derive(Clone, Debug)]
pub struct SourceBase {
    base_dir: PathBuf,
    source_file: Option<PathBuf>,
}

impl SourceBase {
    /// Resolves the base from a deck's directory and its first `% source:`.
    pub fn for_deck(deck: &Deck) -> Self {
        // A `% source:` may name several files joined by ` + ` (the first a full
        // path, the rest relative to its directory). A per-card `% at: file:lines`
        // locator resolves against the first file's directory, so base the
        // resolution on that first part — not the whole joined string, which is
        // not itself a path.
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

    /// Reads the live excerpt a card's `% at:` `locator` points at. Errors the
    /// same way a trace does — an unreadable/missing file, a line range past the
    /// file's end (the drift symptom), or a line-only locator with no single
    /// `% source:` file.
    pub fn excerpt(&self, locator: &str) -> Result<Excerpt> {
        excerpt_at(&self.base_dir, self.source_file.as_deref(), locator)
    }
}

/// The project root the grounded ask-tutor reads: the nearest directory above a
/// deck's `% source:` files that looks like a project (holds a `Cargo.toml`,
/// `.git`, `package.json`, `go.mod`, or `pyproject.toml`), so the tutor can read
/// the **whole** crate, not just the cited files. Falls back to the sources'
/// common-ancestor directory, and to `None` when the deck has no local source
/// (a URL source, or nothing on disk). `deck_dir` resolves relative sources.
pub(crate) fn project_root(sources: &[String], deck_dir: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = sources
        .iter()
        .filter(|s| !is_url(s))
        .flat_map(|s| source_paths(s, Some(deck_dir)))
        .filter(|p| p.exists())
        // A cited file contributes its containing directory.
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

/// Splits a `% source:` value into the file/dir paths it names. Most values are a
/// single path, but the deck generator sometimes joins several with " + " where
/// the first is a full path and the rest are relative to its directory (e.g.
/// `<crate>/README.md + src/lib.rs` → both files under `<crate>`). A relative
/// part resolves against the first part's directory when that exists, else
/// against `base` (the deck's folder).
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

/// The first file/dir a (possibly ` + `-joined) `% source:` value names, trimmed
/// — the path a deck's `% at:` citations resolve their base against.
fn first_source(value: &str) -> &str {
    value.split(" + ").next().unwrap_or(value).trim()
}

/// The deepest directory that is an ancestor of every path in `dirs`.
fn common_ancestor(dirs: &[PathBuf]) -> Option<PathBuf> {
    let mut common = dirs.first()?.clone();
    for d in &dirs[1..] {
        while !d.starts_with(&common) {
            common = common.parent()?.to_path_buf();
        }
    }
    Some(common)
}

/// Walks up from `dir` (inclusive) to the first ancestor holding a project
/// marker, or `None` if none is found before the filesystem root.
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

/// Splits a locator into its optional `file:` part and optional line range.
/// `card.rs:1-9` → (`card.rs`, `1-9`); `1-9` → (none, `1-9`); `card.rs` →
/// (`card.rs`, none, the whole file). A locator is a single span — `N` or
/// `N-M`, never comma-separated — so a stitched, misleading excerpt is
/// impossible. The split is on the last colon whose suffix is a valid range, so
/// paths with colons stay intact.
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

/// Whether `s` is a single line range: `N` or `N-M`, all digits.
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

/// Resolves a parsed locator's `file:` part to the file it reads. `None` = a
/// line-only locator with no single-file source (the caller reports it). Shared
/// by [`excerpt_at`] and [`Trace::lint_locators`] so the two never disagree.
///
/// Two ways a `% at:` path can be written relative to a different root than the
/// `% source:` scope, both handled here:
/// - A **single-file** `% source:` (`source_file`) IS that one file, so the locator reads it
///   directly and any `file:` part is redundant (it may repeat the path from the crate root, which
///   would otherwise duplicate — `…/src/executor/src/executor/env.rs`).
/// - A **directory** `% source:` whose locators are written relative to a project root *above* it
///   (the generator does this — `% source: …/crate/src/executor` but `% at:
///   src/executor/local_vm.rs`): see [`resolve_under_base`].
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

/// Resolves a locator's `file:` part under a **directory** `% source:` `base_dir`.
/// Normally `file` is relative to `base_dir`, but the deck generator sometimes
/// writes it relative to a PROJECT ROOT *above* `% source:` — e.g. `% source:`
/// scopes to `…/crate/src/executor` while the locator is the crate-root-relative
/// `src/executor/local_vm.rs`. Joining that straight onto the deeper base doubles
/// the overlap (`…/src/executor/src/executor/local_vm.rs`, "no such file"). So:
/// the direct join when it exists, else the first ancestor of `base_dir` at which
/// `file` resolves, else fall back to the direct join (so a genuine miss still
/// names the expected path in the error).
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
    // Last resort: the locator dropped (or added) a leading subdirectory, so
    // neither the direct join nor any ancestor resolves — e.g. a `% at:
    // ch12-03-…md` whose file actually lives at `<base>/src/ch12-03-…md`. Search
    // the source subtree for a file of the same name; a hit recovers it. Only
    // reached once path-relative resolution has failed, so it is best-effort,
    // taking the first name match if several exist.
    if let Some(name) = Path::new(file).file_name()
        && let Some(found) = find_under(base_dir, name)
    {
        return found;
    }
    direct
}

/// Depth-first search under `root` for the first entry named `name`, skipping
/// version-control and build directories so a large tree stays cheap — the
/// fallback for [`resolve_under_base`] when a locator's relative path is written
/// against a different subtree than the source base.
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

/// Relabels a freshly-read `excerpt` to its ORIGINAL source for *display*, using
/// the card's `at_origin` (the ` from <file>:<lines>` recorded on the `% at:`
/// line when the excerpt was frozen). A frozen asset (`assets/30.rs`) holds
/// content re-based to line 1; when `at_origin` is present this repoints the
/// excerpt's path at the real file (`src/caching.rs`), renumbers its lines from
/// the original start, and returns `file:start-end` to show as the "at" label —
/// so the learner sees real source and line numbers, not the opaque asset. With
/// no `at_origin` (a live trace) it returns the excerpt unchanged and `None`.
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

/// Renders a frozen card's excerpt as the inline "exact code" block for the
/// tutor: the relabeled origin label (`src/caching.rs:46-66`) and the snippet
/// lines with their real line numbers. The asset is the anchor — what the learner
/// sees — so the tutor reasons about this, using the live source only for context.
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

/// A frozen card whose snapshot no longer matches the live source — so the
/// learner can update or drop it. `at` labels the card (its origin location).
#[derive(Debug)]
pub struct Drift {
    /// The card's front line in the deck file.
    pub line: usize,
    /// The original location label (`src/caching.rs:46-66`).
    pub at: String,
    /// Why it drifted: the source file is gone, or the snippet changed/moved out.
    pub gone: bool,
}

/// Detects frozen cards whose frozen excerpt no longer exists in the live source
/// (the file is gone, or its lines changed). A *moved* excerpt that's otherwise
/// unchanged is NOT flagged — the block is searched across the whole file. Empty
/// for a non-frozen deck or one with no readable origin.
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
        // The frozen snippet (the asset) vs the live file it came from.
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

/// Whether the frozen excerpt's lines still appear as a contiguous block in
/// `live` (trailing whitespace ignored, so reformatting the file's line endings
/// doesn't read as drift). A moved-but-unchanged block still matches.
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

/// The inline frozen-excerpt block for a fact card's tutor prompt, read from the
/// deck's snapshot asset. `None` for a live card (no `at_origin`) or an
/// unreadable asset.
pub fn frozen_excerpt_block(
    at: &str,
    at_origin: Option<&str>,
    source_base: &SourceBase,
) -> Option<String> {
    at_origin?;
    let excerpt = source_base.excerpt(at).ok()?;
    Some(render_frozen_block(excerpt, at_origin))
}

/// Parses a frozen `% at:`'s `at_origin` provenance (`src/caching.rs:46-66`) into
/// the real source file and its 1-based start line. The split is on the LAST
/// colon, so a path with directories (`src/caching.rs`) stays intact.
pub(crate) fn parse_at_origin(at_origin: Option<&str>) -> Option<(String, usize)> {
    let spec = at_origin?.trim();
    let (file, lines) = spec.rsplit_once(':')?;
    let start = lines.split('-').next()?.trim().parse().ok()?;
    (!file.trim().is_empty()).then(|| (file.trim().to_string(), start))
}

fn excerpt_at(base_dir: &Path, source_file: Option<&Path>, locator: &str) -> Result<Excerpt> {
    let (file, spec) = parse_locator(locator);
    // A relative `file:` part is joined onto `base_dir`; if that base no longer
    // exists, the join fabricates a misleading `…/missing-base/file` path. Fail on
    // the real cause — the `% source:` base is gone — rather than the phantom path.
    let joins_onto_base =
        source_file.is_none() && file.as_deref().is_some_and(|f| !Path::new(f).is_absolute());
    if joins_onto_base && !base_dir.is_dir() {
        bail!(
            "the `% source:` base `{}` does not exist — the deck's source path is \
             likely stale or wrong",
            base_dir.display()
        );
    }
    let path = locator_path(base_dir, source_file, file.as_deref()).ok_or_else(|| {
        anyhow!(
            "locator `{locator}` gives only line numbers, but `% source:` \
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
        // A single-file `% source:` whose checkpoint repeats the path relative to
        // the crate root must still read the source file — not join the path onto
        // the file's own directory and duplicate it
        // (`…/src/executor/src/executor/env.rs`).
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/executor")).unwrap();
        let file = write(dir.path(), "src/executor/env.rs", "a\nb\nc\nd\n");
        let base_dir = file.parent().unwrap();
        let ex = excerpt_at(base_dir, Some(&file), "src/executor/env.rs:2-3").unwrap();
        assert_eq!(vec![(2, "b".to_string()), (3, "c".to_string())], ex.lines);
    }

    #[test]
    fn excerpt_at_reports_a_missing_source_base_clearly() {
        // A directory `% source:` whose base no longer exists (a moved/stale path)
        // must error on the SOURCE itself, not fabricate a `…/missing-base/file`
        // path by joining the locator onto it.
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
        // `% source:` scopes to a SUBDIR (`<root>/src/executor`), but the locator
        // is written relative to the crate root (`src/executor/local_vm.rs`).
        // Joining it onto the deeper base would double the overlap
        // (`…/src/executor/src/executor/local_vm.rs`); resolution must instead walk
        // up and find it at the crate root.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/executor")).unwrap();
        write(dir.path(), "src/executor/local_vm.rs", "a\nb\nc\nd\n");
        let base_dir = dir.path().join("src/executor"); // the `% source:` dir
        let ex = excerpt_at(&base_dir, None, "src/executor/local_vm.rs:2-3").unwrap();
        assert_eq!(vec![(2, "b".to_string()), (3, "c".to_string())], ex.lines);
    }

    #[test]
    fn excerpt_at_recovers_a_dropped_subdirectory_via_basename_search() {
        // The locator DROPPED its `src/` prefix (`chapter.md` when the file lives
        // at `<root>/src/chapter.md`) — the exact shape the Rust-book explore hit.
        // Neither the direct join nor an ancestor resolves, so a basename search
        // under the source root must recover it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        write(dir.path(), "src/chapter.md", "a\nb\nc\nd\n");
        let base_dir = dir.path().to_path_buf(); // the source root; chapter.md is a level down
        let ex = excerpt_at(&base_dir, None, "chapter.md:2-3").unwrap();
        assert_eq!(vec![(2, "b".to_string()), (3, "c".to_string())], ex.lines);
    }

    #[test]
    fn relabel_for_display_uses_the_at_origin() {
        // A frozen asset's excerpt (path `30.rs`, re-based lines 1-3) is relabeled
        // to the real source + line numbers from the `% at:` ` from …` origin.
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
        // The label reflects the lines actually shown (here 3, not the full range).
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
        assert_eq!(None, relabel_for_display(ex, None).1); // no note: unchanged
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
        // No colon / no provenance → nothing to relabel.
        assert_eq!(None, parse_at_origin(Some("just an insight")));
        assert_eq!(None, parse_at_origin(None));
    }

    #[test]
    fn source_base_reads_a_fact_cards_citation() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "notes.md", "one\ntwo\nthree\nfour\n");
        let deck_path = dir.path().join("facts.txt");
        // A plain fact deck (no `% trace:`) whose card carries a `% at:`.
        std::fs::write(
            &deck_path,
            "% source: notes.md\n# q\n\ta\n\t% at: notes.md:2-3\n",
        )
        .unwrap();
        let deck = crate::deck::Deck::load(&deck_path).unwrap();
        let base = SourceBase::for_deck(&deck);
        let locator = deck.cards[0].at.as_deref().expect("card carries % at:");
        assert_eq!(
            vec![(2, "two".to_string()), (3, "three".to_string())],
            base.excerpt(locator).unwrap().lines
        );
        // A single-file `% source:` also lets a line-only locator resolve.
        assert_eq!(
            vec![(3, "three".to_string())],
            base.excerpt("3").unwrap().lines
        );
    }

    #[test]
    fn source_base_reads_a_multi_file_citation() {
        // A `% source:` that joins several files with ` + ` (the first a full
        // path, the rest relative to its dir). Each card's `% at: file:lines`
        // must resolve to the right file, not be appended to the joined string.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", "r1\nr2\nr3\n");
        std::fs::create_dir(dir.path().join("src")).unwrap();
        write(dir.path(), "src/lib.rs", "l1\nl2\nl3\nl4\n");
        let readme = dir.path().join("README.md");
        let deck_path = dir.path().join("facts.txt");
        std::fs::write(
            &deck_path,
            format!(
                "% source: {} + src/lib.rs\n\
                 # q1\n\ta1\n\t% at: README.md:1-2\n\
                 # q2\n\ta2\n\t% at: src/lib.rs:3-4\n",
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
        // A bare-line locator is ambiguous with several files — it must error
        // rather than silently read the first one.
        assert!(base.excerpt("2").is_err());
    }

    #[test]
    fn delta_keys_and_grades() {
        assert_eq!(Some(Delta::Passed), Delta::from_key('N'));
        assert_eq!(Some(Delta::Partial), Delta::from_key('p'));
        assert_eq!(Some(Delta::Failed), Delta::from_key('f'));
        assert_eq!(None, Delta::from_key('x'));
        // Passed advances; partly maps to FSRS Hard; failed resets to FSRS Again —
        // each shares the review grade, so a partly is a distinct, gentler outcome.
        assert_eq!(Grade::Pass, Delta::Passed.grade());
        assert_eq!(Grade::Partial, Delta::Partial.grade());
        assert_eq!(Grade::Fail, Delta::Failed.grade());
    }

    #[test]
    fn walking_a_trace_drills_but_does_not_master_it() {
        use crate::{deck::DeckState, store::Store};
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.rs", "a\nb\nc\n");
        let deck_path = dir.path().join("t.txt");
        std::fs::write(
            &deck_path,
            format!(
                "% trace: how a moves\n% source: {}\n# what happens?\n\tit advances\n\t% at: 1-2\n",
                dir.path().join("src.rs").display()
            ),
        )
        .unwrap();
        let deck = crate::deck::Deck::load(&deck_path).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        assert!(!store.deck_mastered(&deck.subject));
        assert_eq!(DeckState::NotStarted, deck.state(&store));

        // Walk the single checkpoint, then graduate it (FSRS `Review`) so the deck's
        // unlock gate — every card graduated — is met.
        let card0 = Trace::from_deck(&deck).unwrap().checkpoints[0].card_id;
        let mut walk = Walk::new(Trace::from_deck(&deck).unwrap());
        walk.predict("p".to_string());
        walk.grade(&mut store, Delta::Passed, 1);
        assert_eq!(Phase::Done, walk.phase()); // no compress step
        if let Some(f) = store.get_or_insert(card0, 0).recall.as_mut() {
            f.state = 2; // Review — graduated
        }

        // The walk is the DRILL, not the exam: drilling all checkpoints does NOT
        // master the trace — it becomes `ExamDue` (its compression exam is next),
        // so it stays locked for dependents until that exam is passed.
        assert!(!store.deck_mastered(&deck.subject));
        assert_eq!(DeckState::ExamDue, deck.state(&store));

        // Passing the trace exam is what masters it (→ Finished, unlocks).
        store.set_deck_mastered(&deck.subject, 99);
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn parse_locator_splits_file_and_spec() {
        assert_eq!(
            (Some("card.rs".to_string()), Some("1-9".to_string())),
            parse_locator("card.rs:1-9")
        );
        // A path with a directory separator still splits on the line colon.
        assert_eq!(
            (
                Some("src/serve.rs".to_string()),
                Some("682-689".to_string())
            ),
            parse_locator("src/serve.rs:682-689")
        );
        // A comma is not a valid range, so `file:N,M` is treated as a bare file.
        assert_eq!(
            (Some("src/serve.rs:544,980".to_string()), None),
            parse_locator("src/serve.rs:544,980")
        );
        // Line-only and bare-file forms.
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
        // A reversed range is normalized.
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
        // A single line.
        let ex = read_excerpt(&path, Some("1")).unwrap();
        assert_eq!(vec![(1, "a".to_string())], ex.lines);
    }

    #[test]
    fn read_excerpt_clamps_out_of_range_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "f.txt", "a\nb\nc\n");
        // 2-99 clamps to the file's 3 lines; 99 alone points outside -> error.
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

    /// Builds a trace deck in `dir` over a single source file and returns it.
    fn trace_deck(dir: &Path) -> Deck {
        write(dir, "source.txt", "first\nsecond\nthird\nfourth\n");
        let path = write(
            dir,
            "t.txt",
            "% trace: how it works\n\
             % source: source.txt\n\
             # Predict the first hop\n\
             \t% given: line — the current input line\n\
             \tit reads the first line\n\
             \t% at: 1\n\
             \t! the entry point\n\
             # Predict the second hop\n\
             \tit reads lines two and three\n\
             \t% at: 2-3\n",
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

        // A plain deck (no `% trace:`) is not a trace.
        let plain = write(dir.path(), "p.txt", "# q\n\ta\n");
        let err = Trace::from_deck(&Deck::load(&plain).unwrap()).unwrap_err();
        assert!(format!("{err:#}").contains("not a trace"));
    }

    #[test]
    fn excerpt_reads_live_from_the_single_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        // The line-only locator resolves against the single `% source:` file.
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
        // `% source:` is a directory, so a bare `% at: 1` cannot resolve.
        let path = write(
            dir.path(),
            "t.txt",
            "% trace: g\n% source: .\n# q\n\ta\n\t% at: 1\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let err = trace.excerpt(&trace.checkpoints[0]).unwrap_err();
        assert!(format!("{err:#}").contains("not a single file"));
    }

    /// A trace whose `% at:` locators all resolve in range lints clean.
    #[test]
    fn lint_locators_passes_a_valid_trace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\nfour\nfive\n");
        let path = write(
            dir.path(),
            "t.txt",
            "% trace: g\n% source: src.txt\n# q\n\ta\n\t% at: 2-3\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        assert!(trace.lint_locators().is_empty());
    }

    /// A range starting past EOF — the source shrank — is flagged.
    #[test]
    fn lint_locators_flags_a_start_past_eof() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\n");
        let path = write(
            dir.path(),
            "t.txt",
            "% trace: g\n% source: src.txt\n# q\n\ta\n\t% at: 5-6\n",
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
            "t.txt",
            "% trace: g\n% source: src.txt\n# q\n\ta\n\t% at: 2-9\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("clamped"));
    }

    /// A `file:` part that names a missing file is flagged.
    #[test]
    fn lint_locators_flags_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.txt",
            "% trace: g\n% source: .\n# q\n\ta\n\t% at: nope.rs:1-2\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("not found"));
    }

    /// A checkpoint with no `% at:` line at all is flagged (a walk can't reveal
    /// its source).
    #[test]
    fn lint_locators_flags_a_missing_locator() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\n");
        let path = write(
            dir.path(),
            "t.txt",
            "% trace: g\n% source: src.txt\n# q\n\ta\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("no `% at:`"));
    }

    /// A bare line-only locator with a directory source can't resolve — flagged.
    #[test]
    fn lint_locators_flags_line_only_without_a_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.txt",
            "% trace: g\n% source: .\n# q\n\ta\n\t% at: 1\n",
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
        let card0 = trace.checkpoints[0].card_id;
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace);

        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(2, walk.total());

        // Hop 1: predict -> reveal -> Passed schedules the checkpoint under FSRS.
        walk.predict("my guess".to_string());
        assert_eq!(Phase::Reveal, walk.phase());
        assert_eq!(Some("my guess"), walk.prediction(0));
        walk.grade(&mut store, Delta::Passed, 1000);
        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(1, walk.current_index());
        assert!(store.get(card0).unwrap().recall.is_some()); // Passed -> FSRS review recorded

        // Hop 2 (last): a Failed records a lapse and finishes the walk (no compress
        // step — verification is the separate trace exam).
        let card1 = walk.checkpoint().unwrap().card_id;
        walk.predict(String::new());
        walk.grade(&mut store, Delta::Failed, 1001);
        assert_eq!(Phase::Done, walk.phase());
        assert_eq!(0, store.get(card1).unwrap().streak); // Failed -> streak reset

        let summary = walk.summary();
        assert_eq!(1, summary.passed);
        assert_eq!(1, summary.failed);
        assert_eq!(vec![1], summary.weak); // the failed hop is the weak edge
    }

    #[test]
    fn partly_records_a_weak_success_on_a_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace);
        let card0 = walk.checkpoint().unwrap().card_id;
        walk.predict("guess".to_string());
        walk.grade(&mut store, Delta::Partial, 1000);
        // Partly is a weak success under FSRS: it schedules the card and counts as a pass.
        let state = store.get(card0).unwrap();
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
        // In Predict phase, grading does nothing.
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

        // README.md (root) + src/lib.rs → common ancestor is the crate root,
        // which holds Cargo.toml.
        let both = vec![
            s(crate_dir.join("README.md")),
            s(crate_dir.join("src/lib.rs")),
        ];
        assert_eq!(Some(crate_dir.clone()), project_root(&both, dir.path()));

        // A single nested file still walks up to the Cargo.toml root.
        let only = vec![s(crate_dir.join("src/lib.rs"))];
        assert_eq!(Some(crate_dir.clone()), project_root(&only, dir.path()));

        // A URL source has no local root.
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

        // `<crate>/README.md + src/lib.rs`: the relative part anchors to the
        // first file's directory (the crate), not the deck folder.
        let value = format!("{}/README.md + src/lib.rs", crate_dir.display());
        assert_eq!(
            vec![crate_dir.join("README.md"), crate_dir.join("src/lib.rs")],
            source_paths(&value, Some(dir.path()))
        );

        // A single path is returned unchanged.
        let one = crate_dir.join("src/lib.rs");
        assert_eq!(
            vec![one.clone()],
            source_paths(&one.to_string_lossy(), None)
        );
    }

    /// A workspace (`alix.toml` + deck) at `root/ws` whose trace cites files in
    /// a sibling source tree at `root/src`.
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
            "t.txt",
            "% trace: how it works\n\
             % source: ../src\n\
             # hop 1\n\tit reads a\n\t% at: a.rs:2-3\n\
             # hop 2\n\tit reads b\n\t% at: b.rs:1\n",
        )
    }

    #[cfg(feature = "full")]
    #[test]
    fn drifted_cards_flags_a_changed_or_missing_excerpt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = snapshot_workspace(root);
        crate::trace_ai::snapshot(&Deck::load(&deck_path).unwrap(), 0, None).unwrap();

        // Intact source → no drift.
        assert!(drifted_cards(&Deck::load(&deck_path).unwrap()).is_empty());

        // The cited excerpt's lines change → drift (file still there).
        std::fs::write(root.join("src/a.rs"), "alpha\nCHANGED\nLINES\n").unwrap();
        let d = drifted_cards(&Deck::load(&deck_path).unwrap());
        assert_eq!(1, d.len(), "{d:?}");
        assert!(!d[0].gone);
        assert_eq!("a.rs:2-3", d[0].at);

        // The whole file is gone → drift (gone).
        std::fs::remove_file(root.join("src/a.rs")).unwrap();
        let d = drifted_cards(&Deck::load(&deck_path).unwrap());
        assert!(d.iter().any(|x| x.gone && x.at == "a.rs:2-3"), "{d:?}");
    }
}
