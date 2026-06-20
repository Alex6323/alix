//! Traces — guided predict-and-verify walks along a path through a source.
//!
//! A trace deck (`% trace:` + a sequence of checkpoint cards, each with an
//! `% at:` locator into a `% source:`) is walked hop by hop: at each checkpoint
//! you commit a **prediction**, then the real **excerpt** from the source is
//! revealed alongside the key points a good prediction should hit, and you
//! judge the **delta** (Got / Partial / Missed). The miss is recorded for SRS —
//! a weak edge resurfaces sooner — but never derails the chain; you advance
//! from the revealed truth. After the last hop you **compress** the whole path
//! into a couple of sentences, which is the trace's own exam.
//!
//! This module is the frontend-agnostic engine: it builds the [`Trace`] from a
//! [`Deck`], resolves each locator to a live [`Excerpt`] (read fresh from the
//! source, the oracle), and drives the [`Walk`] state machine + per-checkpoint
//! scheduling. The CLI (`flash trace`) is a thin reader over it. Grading is
//! self-judged and offline — no model calls — so the mechanic can be validated
//! cheaply; live Claude grading (`--grade`) is a later layer.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::{
    deck::{Deck, is_url},
    scheduler::{Grade, SchedulerKind},
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
    /// The prediction covered the key points.
    Got,
    /// Partly right — something important was missing or wrong.
    Partial,
    /// The prediction missed the point.
    Missed,
}

impl Delta {
    /// The single-letter answer a learner types to record this delta.
    pub fn from_key(c: char) -> Option<Delta> {
        match c.to_ascii_lowercase() {
            'g' => Some(Delta::Got),
            'p' => Some(Delta::Partial),
            'm' => Some(Delta::Missed),
            _ => None,
        }
    }

    /// The label shown to the learner.
    pub fn label(self) -> &'static str {
        match self {
            Delta::Got => "GOT IT",
            Delta::Partial => "PARTIAL",
            Delta::Missed => "MISSED",
        }
    }

    /// How this delta schedules the checkpoint. A nailed hop advances (and
    /// fades); a partial or missed one is a **weak edge** that resets so it
    /// resurfaces sooner — recorded, not punished (the walk still continues).
    pub fn grade(self) -> Grade {
        match self {
            Delta::Got => Grade::Pass,
            Delta::Partial | Delta::Missed => Grade::Fail,
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
    /// The connective insight shown after the reveal (the card note).
    pub note: Option<String>,
    /// The `% at:` locator into the source, if the checkpoint declares one.
    pub locator: Option<String>,
    /// The card's identity hash — the key its per-checkpoint SRS hangs off.
    pub card_id: u64,
}

/// A trace built from a deck: what it walks, the ordered checkpoints, and where
/// their locators resolve.
#[derive(Clone, Debug)]
pub struct Trace {
    /// What the trace walks (`% trace:`) — a path description ("how X becomes
    /// Y").
    pub description: String,
    /// The path origin (`% source:`), shown to the learner. `None` if the deck
    /// declares none (locators then need an explicit `file:` part and a base).
    pub source: Option<String>,
    /// The checkpoints, in file order — the path, walked top to bottom.
    pub checkpoints: Vec<Checkpoint>,
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
                note: c.note.clone(),
                locator: c.at.clone(),
                card_id: c.id(),
            })
            .collect();
        let source = deck.sources.first().cloned();
        let (base_dir, source_file) = resolve_source(deck.path.parent(), source.as_deref());
        Ok(Trace {
            description,
            source,
            checkpoints,
            base_dir,
            source_file,
        })
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
        let (file, spec) = parse_locator(locator);
        let path = match file {
            Some(f) => self.base_dir.join(f),
            None => self.source_file.clone().ok_or_else(|| {
                anyhow!(
                    "locator `{locator}` gives only line numbers, but `% source:` \
                     is not a single file — write it as `file:lines`"
                )
            })?,
        };
        read_excerpt(&path, spec.as_deref())
    }
}

/// The phase of a [`Walk`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Awaiting the learner's prediction for the current checkpoint.
    Predict,
    /// The excerpt + key points are shown; awaiting the self-judged delta.
    Reveal,
    /// Every checkpoint walked; awaiting the final compression of the path.
    Compress,
    /// The walk is finished.
    Done,
}

/// One in-progress walk of a trace — a small frontend-agnostic state machine.
/// The CLI (and, later, a web surface) drive it: show the current checkpoint,
/// take a [`predict`](Walk::predict), reveal the [`excerpt`](Trace::excerpt),
/// take the self-judged [`grade`](Walk::grade) (which schedules the
/// checkpoint), and finally [`compress`](Walk::compress) the path.
pub struct Walk {
    trace: Trace,
    scheduler: SchedulerKind,
    current: usize,
    phase: Phase,
    predictions: Vec<String>,
    deltas: Vec<Option<Delta>>,
    compression: Option<String>,
}

impl Walk {
    /// Starts a walk of `trace`, scheduling checkpoints with `scheduler`.
    pub fn new(trace: Trace, scheduler: SchedulerKind) -> Walk {
        let n = trace.checkpoints.len();
        Walk {
            trace,
            scheduler,
            current: 0,
            phase: Phase::Predict,
            predictions: vec![String::new(); n],
            deltas: vec![None; n],
            compression: None,
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
    /// in `store`, and advances — to the next checkpoint's
    /// [`Phase::Predict`], or to [`Phase::Compress`] after the last one.
    /// No-op outside [`Phase::Reveal`]. The store is updated but not saved
    /// (the caller saves).
    pub fn grade(&mut self, store: &mut Store, delta: Delta, now_ms: u64) {
        if self.phase != Phase::Reveal {
            return;
        }
        if let Some(checkpoint) = self.trace.checkpoints.get(self.current) {
            let state = store.get_or_insert(checkpoint.card_id, now_ms);
            self.scheduler
                .scheduler()
                .apply(state, delta.grade(), now_ms);
        }
        self.deltas[self.current] = Some(delta);
        if self.current + 1 < self.trace.checkpoints.len() {
            self.current += 1;
            self.phase = Phase::Predict;
        } else {
            self.phase = Phase::Compress;
        }
    }

    /// Records the final compression of the path and finishes the walk. No-op
    /// outside [`Phase::Compress`].
    pub fn compress(&mut self, text: String) {
        if self.phase != Phase::Compress {
            return;
        }
        self.compression = Some(text);
        self.phase = Phase::Done;
    }

    /// The prediction typed at checkpoint `i`, if any.
    pub fn prediction(&self, i: usize) -> Option<&str> {
        self.predictions.get(i).map(String::as_str)
    }
    /// The compression text, once the walk is done.
    pub fn compression(&self) -> Option<&str> {
        self.compression.as_deref()
    }

    /// A tally of the deltas recorded so far.
    pub fn summary(&self) -> Summary {
        let mut s = Summary::default();
        for (i, delta) in self.deltas.iter().enumerate() {
            match delta {
                Some(Delta::Got) => s.got += 1,
                Some(Delta::Partial) => {
                    s.partial += 1;
                    s.weak.push(i);
                }
                Some(Delta::Missed) => {
                    s.missed += 1;
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
    pub got: usize,
    pub partial: usize,
    pub missed: usize,
    /// 0-based indices of the checkpoints judged Partial or Missed — the weak
    /// edges that SRS will resurface sooner.
    pub weak: Vec<usize>,
}

/// A live excerpt read from a source for a checkpoint reveal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Excerpt {
    /// The file it was read from.
    pub path: PathBuf,
    /// The selected lines as `(1-based line number, content)`, in order. A jump
    /// of more than one between consecutive entries marks a gap (non-contiguous
    /// ranges), which a frontend can render with an elision marker.
    pub lines: Vec<(usize, String)>,
    /// Whether the selection was cut to [`MAX_EXCERPT_LINES`].
    pub truncated: bool,
}

/// Resolves a `% source:` to the base directory a locator's `file:` part joins
/// onto, and the single source file (when the source is one file, so a locator
/// may omit the filename). A URL or absent source yields the deck's own folder
/// as the base and no source file.
fn resolve_source(deck_dir: Option<&Path>, source: Option<&str>) -> (PathBuf, Option<PathBuf>) {
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

/// Splits a locator into its optional `file:` part and optional line selection.
/// `card.rs:1-9` → (`card.rs`, `1-9`); `1-9` → (none, `1-9`); `card.rs` →
/// (`card.rs`, none, the whole file). The split is on the last colon whose
/// suffix is a valid line selection, so paths with colons stay intact.
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

/// Whether `s` is a line selection: comma-separated `N` or `N-M` items, all
/// digits, at least one item.
fn is_line_spec(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.split(',').all(|item| {
            let item = item.trim();
            match item.split_once('-') {
                Some((a, b)) => is_number(a) && is_number(b),
                None => is_number(item),
            }
        })
}

fn is_number(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Parses a validated line selection into inclusive ranges.
fn parse_line_spec(spec: &str) -> Vec<(usize, usize)> {
    spec.split(',')
        .filter_map(|item| {
            let item = item.trim();
            match item.split_once('-') {
                Some((a, b)) => Some((a.trim().parse().ok()?, b.trim().parse().ok()?)),
                None => {
                    let n = item.parse().ok()?;
                    Some((n, n))
                }
            }
        })
        .map(|(a, b): (usize, usize)| if a <= b { (a, b) } else { (b, a) })
        .collect()
}

/// Reads the selected lines from `path` (the whole file, capped, when `spec` is
/// `None`), returning them with their 1-based line numbers.
fn read_excerpt(path: &Path, spec: Option<&str>) -> Result<Excerpt> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow!("cannot read the source `{}`: {e}", path.display()))?;
    let file_lines: Vec<&str> = text.lines().collect();

    let mut selected: Vec<(usize, String)> = Vec::new();
    let mut truncated = false;
    let push = |no: usize, line: &str, selected: &mut Vec<(usize, String)>| -> bool {
        if selected.len() >= MAX_EXCERPT_LINES {
            return false;
        }
        selected.push((no, line.to_string()));
        true
    };

    match spec {
        None => {
            for (i, line) in file_lines.iter().enumerate() {
                if !push(i + 1, line, &mut selected) {
                    truncated = true;
                    break;
                }
            }
        }
        Some(spec) => {
            'outer: for (start, end) in parse_line_spec(spec) {
                // Clamp to the file; a stale line number can never panic.
                let start = start.max(1);
                let end = end.min(file_lines.len());
                for no in start..=end {
                    let line = file_lines[no - 1];
                    if !push(no, line, &mut selected) {
                        truncated = true;
                        break 'outer;
                    }
                }
            }
        }
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
    fn delta_keys_and_grades() {
        assert_eq!(Some(Delta::Got), Delta::from_key('G'));
        assert_eq!(Some(Delta::Partial), Delta::from_key('p'));
        assert_eq!(Some(Delta::Missed), Delta::from_key('m'));
        assert_eq!(None, Delta::from_key('x'));
        // Got advances; partial/missed are weak edges that reset.
        assert_eq!(Grade::Pass, Delta::Got.grade());
        assert_eq!(Grade::Fail, Delta::Partial.grade());
        assert_eq!(Grade::Fail, Delta::Missed.grade());
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
                Some("544,980-985".to_string())
            ),
            parse_locator("src/serve.rs:544,980-985")
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
    fn parse_line_spec_handles_ranges_singles_and_reversed() {
        assert_eq!(vec![(1, 9)], parse_line_spec("1-9"));
        assert_eq!(vec![(5, 5)], parse_line_spec("5"));
        assert_eq!(vec![(3, 3), (10, 12)], parse_line_spec("3,10-12"));
        // A reversed range is normalized.
        assert_eq!(vec![(8, 12)], parse_line_spec("12-8"));
    }

    #[test]
    fn read_excerpt_selects_ranges_with_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "f.txt", "a\nb\nc\nd\ne\n");
        let ex = read_excerpt(&path, Some("2-3")).unwrap();
        assert_eq!(vec![(2, "b".to_string()), (3, "c".to_string())], ex.lines);
        assert!(!ex.truncated);
        // Non-contiguous selection keeps each line's real number (the gap is
        // visible as the jump 1 -> 4).
        let ex = read_excerpt(&path, Some("1,4")).unwrap();
        assert_eq!(vec![(1, "a".to_string()), (4, "d".to_string())], ex.lines);
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

    #[test]
    fn walk_runs_predict_reveal_grade_then_compress() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let card0 = trace.checkpoints[0].card_id;
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace, SchedulerKind::Leitner);

        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(2, walk.total());

        // Hop 1: predict -> reveal -> Got advances the checkpoint.
        walk.predict("my guess".to_string());
        assert_eq!(Phase::Reveal, walk.phase());
        assert_eq!(Some("my guess"), walk.prediction(0));
        walk.grade(&mut store, Delta::Got, 1000);
        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(1, walk.current_index());
        assert_eq!(2, store.get(card0).unwrap().stage); // Got -> stage up

        // Hop 2 (last): a Missed resets to stage 1 and moves to compression.
        let card1 = walk.checkpoint().unwrap().card_id;
        walk.predict(String::new());
        walk.grade(&mut store, Delta::Missed, 1001);
        assert_eq!(Phase::Compress, walk.phase());
        assert_eq!(1, store.get(card1).unwrap().stage); // Missed -> reset

        walk.compress("the whole path in two sentences".to_string());
        assert_eq!(Phase::Done, walk.phase());
        assert_eq!(Some("the whole path in two sentences"), walk.compression());

        let summary = walk.summary();
        assert_eq!(1, summary.got);
        assert_eq!(1, summary.missed);
        assert_eq!(vec![1], summary.weak); // the missed hop is the weak edge
    }

    #[test]
    fn grade_is_a_noop_outside_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace, SchedulerKind::Leitner);
        // In Predict phase, grading does nothing.
        walk.grade(&mut store, Delta::Got, 1000);
        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(0, walk.current_index());
        assert!(store.is_empty());
    }
}
