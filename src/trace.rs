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
    ask,
    config::{AskConfig, TraceConfig},
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
    /// Named "givens" (`% given:` lines): off-screen symbols the question leans
    /// on, shown as a list under the prompt before predicting.
    pub givens: Vec<String>,
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
                givens: c.givens.clone(),
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

// ── Building (`flash trace --build`) ─────────────────────────────────────────
//
// Discovering the path is a separate, heavier step from walking it: Claude
// explores the `% source:` (read-only file tools, cwd at the source root) and
// emits the checkpoint cards, which the CLI writes back into the deck. Mirrors
// `crate::generate`: build a prompt, run the CLI, clean the output.

/// Explores the deck's `% source:` and returns the discovered checkpoint cards
/// as deck-format text (ready to write back with
/// [`crate::deck::set_trace_checkpoints`]). Blocks until the CLI replies or
/// times out. Errors if the deck declares no `% trace:` or `% source:`.
pub fn build(deck: &Deck, cfg: &TraceConfig, ask_cfg: &AskConfig) -> Result<String> {
    let description = deck
        .trace
        .as_deref()
        .ok_or_else(|| anyhow!("{} declares no `% trace:` to build", deck.subject))?;
    let source = deck
        .sources
        .first()
        .ok_or_else(|| anyhow!("{} declares no `% source:` scope to trace", deck.subject))?;
    let url = is_url(source);
    let cwd = if url {
        None
    } else {
        let (base_dir, _) = resolve_source(deck.path.parent(), Some(source));
        Some(base_dir)
    };
    let prompt = build_prompt(description, source, url, cfg);
    let run_cfg = build_run_config(cfg, ask_cfg, cwd, url);
    let raw = ask::run(&run_cfg, &prompt, &[])?;
    let cards = clean_to_cards(&raw);
    if cards.trim().is_empty() {
        bail!("the build produced no checkpoints");
    }
    Ok(cards)
}

/// The CLI runner config for a build: the ask command/permission with trace's
/// own model and (longer) timeout, **read-only** exploration tools, and the
/// source root as the working directory.
fn build_run_config(
    cfg: &TraceConfig,
    ask_cfg: &AskConfig,
    cwd: Option<PathBuf>,
    url: bool,
) -> AskConfig {
    let mut allowed_tools = vec!["Read".to_string(), "Glob".to_string(), "Grep".to_string()];
    if url {
        allowed_tools.push("WebFetch".to_string());
    }
    AskConfig {
        command: ask_cfg.command.clone(),
        permission_mode: ask_cfg.permission_mode.clone(),
        allowed_tools,
        model: cfg.model.clone().or_else(|| ask_cfg.model.clone()),
        timeout_secs: cfg.timeout_secs,
        cwd,
    }
}

/// Builds the path-discovery prompt: the goal, the scope, how to explore it,
/// the checkpoint format, and the chain-not-a-set rules (see `docs/traces.md`).
fn build_prompt(description: &str, source: &str, url: bool, cfg: &TraceConfig) -> String {
    let explore = if url {
        format!("Read the source page at {source} with the WebFetch tool (fetch it once).")
    } else {
        "Your working directory is the source root. Explore it with the Read, Glob \
         and Grep tools — start at the entry point or the most load-bearing file and \
         follow the references. You can read any file under the source; you have no \
         write or shell access."
            .to_string()
    };
    let locator = if url {
        "a short quoted span from the page — the exact sentence(s) the key points rest on"
    } else {
        "ONE contiguous range, `file:start-end` (or `file:N` for a single line) \
         relative to the source root, e.g. `src/serve.rs:682-689` — NEVER \
         comma-separated ranges"
    };
    let mut p = format!(
        "You are tracing ONE path through a source so a learner can UNDERSTAND it by \
         predicting each step before it is revealed. The path must answer:\n\n    \
         {description}\n\nSource (the scope): {source}\n{explore}\n\nFind the single \
         load-bearing path from the trigger to the outcome named above — a real \
         SEQUENCE (a data flow, a control flow, or a derivation), not a grab-bag of \
         facts about the topic. Then write it as a series of CHECKPOINT cards, one \
         per hop.\n\n\
         FORMAT — output ONLY the checkpoint cards: no header, no `% trace:` or \
         `% source:` line, no preamble, no code fences. Each checkpoint is:\n\n    \
         # <the question for this hop, asked plainly>\n    \t% given: <name> — <what \
         it is>\n    \t<a key point a correct answer hits>\n    \t<another key \
         point>\n    \t% at: <locator>\n    \t! <one connecting insight, shown after \
         the reveal>\n\n\
         The `# ` front (column 0) is the QUESTION. The `% given:` lines (repeatable, \
         optional) name off-screen symbols the question leans on — flash lists them \
         under the question before the learner predicts. The indented lines under it \
         are the key points the revealed source makes (the rubric). `% at:` is the \
         locator: {locator} — it must point at the REAL lines/passage the key points \
         paraphrase, because flash reads them live at review time as the ground \
         truth. Cite accurately. The indented `! ` line is an optional note.\n\n\
         SCOPE EACH HOP TO A SELF-CONTAINED UNIT, AND GLOSS WHAT YOU DON'T SHOW. The \
         reader sees ONLY the lines you cite, so an excerpt must read on its own. \
         Prefer hops that are a whole SMALL function/method — its inputs are its \
         parameters, so nothing dangles. Do NOT dissect one big function into several \
         checkpoints: a big function on the path is ONE black-box hop — cite its \
         signature plus the load-bearing line(s) and describe what it does in the key \
         points; if its internals are themselves worth understanding, that is a \
         SEPARATE trace, not more hops here.\n\
         The `% at:` is ONE CONTIGUOUS RANGE. NEVER stitch several ranges together \
         (no commas): collapsing the gaps makes lines from different branches/places \
         look adjacent, which misleads. If a hop cannot be shown in one contiguous \
         span, it spans more than one region — that is the signal it is too big: \
         split it, or black-box the function. \n\
         GLOSS — completely and correctly. A `% given:` names a free variable: a \
         symbol the cited span USES but does NOT BIND within those lines — typically \
         a function PARAMETER (declared in the signature, above the body you cite) or \
         a value from an enclosing/earlier scope. Apply a mechanical test to each \
         symbol: if its binding (a `let`, an assignment, a `for x in`, the parameter \
         itself) is INSIDE the cited lines, it is NOT a given — the reader sees it, \
         do not gloss it; if the span uses it but its binding is OUTSIDE, it IS a \
         given. (E.g. for a function body excerpt, the parameters are givens; a \
         `let x = …` on a cited line is not.) Name each given with a `% given:` line, \
         one per symbol, `name — what it is` (e.g. `% given: defaults — the workspace \
         directive defaults, a parameter`). Check BOTH directions: every \
         used-but-unbound symbol gets a `% given:`, and never gloss one the span \
         binds itself. The list MUST be COMPLETE — a reader follows the span using \
         ONLY the cited lines plus the givens. flash shows them under the question, \
         so NEVER cram them into the question text. The gloss names the inputs \
         (scaffolding); the cited lines stay the ground truth for the predicted thing \
         — never move the hop's answer into the gloss. More than ~3 givens means the \
         hop is cut too fine: re-scope it.\n\
         KEY POINTS MUST BE GROUNDED in the cited lines. Every key point has to be \
         evident from the excerpt (or a given) — describe ONLY what those lines show, \
         never the rest of the function or file. If a key point asserts behavior that \
         is not in the cited lines (another branch, a later call, the return path), it \
         does not belong to this hop: cite the lines that show it, or drop it and let \
         another hop cover it. A whole-function \"what does it do?\" question whose \
         honest answer needs code you did NOT cite is mis-scoped — either BLACK-BOX it \
         (key points stay at the contract the signature/return actually shows) or \
         SPLIT it into hops that each cite their own region. Before emitting a \
         checkpoint, re-read its key points against ONLY its excerpt + givens and \
         delete any claim you cannot point to.\n\n\
         THE RULES THAT MAKE IT A PATH, NOT A QUIZ — follow every one:\n\
         1. One path, not a set. Each hop is a step along one chain. If two \
         checkpoints could be reordered without breaking, they are a set — re-trace \
         the spine.\n\
         2. Every question opens on the previous reveal: state the conclusion the \
         prior hop established, then ask the next step. (Hop 1 has no prior.)\n\
         3. Carry the STATE, not the bookkeeping: restate what is now true about the \
         system (\"the request carries only the grade, no card id\"), NEVER \"as \
         checkpoint 2 showed\" or \"the last hop\" — each checkpoint is reviewed \
         alone, so an index reference is meaningless.\n\
         4. Ask forward, and just ask: a plain question answerable by reasoning \
         forward from the prior reveal. Do NOT prefix fronts with \"Predict\".\n\
         5. Don't give the answer away: keep a hop's answer out of its own question. \
         Avoid loaded tells (\"it lives ONLY in memory\" hands over \"so save it\"); \
         state the setup neutrally and let the learner reason.\n\
         6. Dives must return: if a hop calls into another function/file, the next \
         hop may dive in, but then return to the caller before going past the call — \
         bridge the return with state, reusing the call-site line so the seam shows.\n\
         7. The last hop reaches the outcome the path was tracing toward.\n\n\
         Keep each question one or two sentences and each key point one line. Use as \
         many checkpoints as the path needs (typically 4-8); never pad."
    );
    if let Some(extra) = cfg
        .extra
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        p.push_str("\n\nAdditional instructions:\n");
        p.push_str(extra);
    }
    p
}

/// Strips anything around the generated checkpoint cards: a leading code fence,
/// commentary, or a stray header before the first `#` card front, and trailing
/// blank/fence lines. Unlike a full deck, a built trace's output is only the
/// cards, so everything before the first column-0 `#` is dropped.
fn clean_to_cards(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(start) = lines.iter().position(|l| l.starts_with('#')) else {
        return raw.trim().to_string();
    };
    let mut end = lines.len();
    while end > start + 1 {
        let t = lines[end - 1].trim();
        if t.is_empty() || t.starts_with("```") {
            end -= 1;
        } else {
            break;
        }
    }
    lines[start..end].join("\n")
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

/// Reads one contiguous span from `path` (the whole file, capped, when `spec`
/// is `None`), returning the lines with their 1-based numbers.
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

    // ── build (`flash trace --build`) ────────────────────────────────────────

    #[test]
    fn build_prompt_carries_goal_source_format_and_rules() {
        let p = build_prompt("how X becomes Y", ".", false, &TraceConfig::default());
        assert!(p.contains("how X becomes Y"));
        assert!(p.contains("Source (the scope): ."));
        assert!(p.contains("Read, Glob")); // local exploration tools
        assert!(p.contains("file:start-end")); // single-range local locator form
        assert!(p.contains("ONE CONTIGUOUS RANGE")); // no stitched multi-range excerpts
        assert!(p.contains("# <the question")); // the checkpoint format
        assert!(p.contains("% at:"));
        assert!(p.contains("black-box hop")); // big function = one black-box hop
        assert!(p.contains("free variable")); // gloss free variables as givens
        assert!(p.contains("% given:")); // givens emitted as a directive, not crammed
        assert!(p.contains("MUST be COMPLETE")); // every off-screen symbol glossed
        assert!(p.contains("does NOT BIND")); // a given is used-but-not-bound (free)
        assert!(p.contains("KEY POINTS MUST BE GROUNDED")); // no claims beyond the excerpt
        assert!(p.contains("One path, not a set"));
        assert!(p.contains("Carry the STATE"));
        assert!(p.contains("Do NOT prefix fronts with \"Predict\""));
        assert!(p.contains("Dives must return"));
        assert!(!p.contains("WebFetch")); // a local source needs no web tool
    }

    #[test]
    fn build_prompt_url_uses_webfetch_and_quoted_span() {
        let p = build_prompt("how X", "https://x", true, &TraceConfig::default());
        assert!(p.contains("WebFetch"));
        assert!(p.contains("quoted span"));
        assert!(!p.contains("Glob")); // no local file tools for a URL source
    }

    #[test]
    fn build_prompt_appends_extra() {
        let cfg = TraceConfig {
            extra: Some("trace the read path".to_string()),
            ..TraceConfig::default()
        };
        let p = build_prompt("g", ".", false, &cfg);
        assert!(p.contains("Additional instructions:"));
        assert!(p.contains("trace the read path"));
    }

    #[test]
    fn build_run_config_uses_readonly_tools_and_cwd() {
        let cwd = PathBuf::from("/some/src");
        let cfg = build_run_config(
            &TraceConfig::default(),
            &AskConfig::default(),
            Some(cwd.clone()),
            false,
        );
        assert_eq!(vec!["Read", "Glob", "Grep"], cfg.allowed_tools);
        assert_eq!(Some(cwd), cfg.cwd);
        assert_eq!(600, cfg.timeout_secs); // the trace timeout, not ask's 120
    }

    #[test]
    fn clean_to_cards_strips_fence_and_preamble() {
        let raw = "Here is the trace:\n```text\n# Q1\n\tp\n\t% at: 1\n```";
        assert_eq!("# Q1\n\tp\n\t% at: 1", clean_to_cards(raw));
    }

    /// Serializes the tests that write + exec a fake CLI (a concurrent fork
    /// would inherit the write-open fd and fail exec with ETXTBSY).
    static EXEC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn fake_cli(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn ask_config(command: &Path) -> AskConfig {
        AskConfig {
            command: command.to_str().unwrap().to_string(),
            timeout_secs: 10,
            ..AskConfig::default()
        }
    }

    #[test]
    fn build_end_to_end_returns_cleaned_cards() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(
            dir.path(),
            "printf '# Q1\\n\\tp1\\n\\t%% at: 1\\n# Q2\\n\\tp2\\n\\t%% at: 2\\n'",
        );
        // A trace deck with `% source: .` (cwd resolves to the temp dir).
        let path = write(dir.path(), "t.txt", "% trace: how it works\n% source: .\n");
        let deck = Deck::load(&path).unwrap();
        let cards = build(&deck, &TraceConfig::default(), &ask_config(&cli)).unwrap();
        assert!(cards.starts_with("# Q1"));
        assert!(cards.contains("# Q2"));
        assert!(cards.contains("% at: 2"));
    }
}
