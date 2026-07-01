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
//! scheduling. The CLI (`alix trace`) is a thin reader over it. Grading is
//! self-judged and offline — no model calls — so the mechanic can be validated
//! cheaply; live Claude grading (`--grade`) is a later layer.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, channel},
};

use anyhow::{Context, Result, anyhow, bail};

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
    /// passed hop advances (and fades); a **partly** one is a weak edge that
    /// drops a stage so it resurfaces sooner; a **failed** one resets — recorded,
    /// not punished (the walk still continues).
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

// ── Building (`alix trace --build`) ─────────────────────────────────────────
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

// ── Snapshotting the source ──────────────────────────────────────────────────
//
// Line-number locators read the live source, so editing a traced file silently
// shifts every excerpt. Snapshotting freezes just the cited excerpts into the
// workspace's `assets/` (one small file per checkpoint) and repoints `% source:`
// + each `% at:` at them: the excerpts then never drift, and the workspace is
// self-contained — without copying whole (possibly huge) source files. The
// re-based excerpt loses its original line numbers, which don't matter once the
// span is frozen. It's the default last step of `alix explore --into --build`;
// a loose trace over a live path is left untouched. The source is any text file.

/// The directory under a workspace where a snapshotted trace's excerpts are
/// frozen.
pub(crate) const SNAPSHOT_DIR: &str = "assets";

/// What [`snapshot`] froze.
#[derive(Debug)]
pub(crate) struct SnapshotReport {
    /// The excerpt snippet files written into `assets/`.
    pub copied: Vec<String>,
    /// Locators whose excerpt couldn't be read and were left as-is.
    pub missing: Vec<String>,
}

/// Freezes a deck's cited **excerpts** into `<workspace>/assets/` — one small
/// snippet file per `% at:` citation, holding just the lines it reveals — and
/// repoints `% source:` + every `% at:` at them. The locators then never drift
/// when the upstream source is edited, and nothing huge is copied. The frozen
/// excerpt is re-based to line 1 (the original line numbers are lost, which is
/// fine for a frozen span). Works for a trace (its checkpoints) or a fact deck
/// (its cited cards); `start` is how many snippets earlier decks in the same
/// workspace already wrote, so names stay unique in the shared `assets/`.
///
/// Requires a deck whose `% source:` is local (not a URL) and whose folder is a
/// workspace. The freeze is one-way — there is no "un-snapshot"; the workspace is
/// either long-lived stable material or a throwaway.
pub(crate) fn snapshot(
    deck: &Deck,
    start: usize,
    workspace_origin: Option<&Path>,
) -> Result<SnapshotReport> {
    let deck_dir = deck.path.parent().unwrap_or_else(|| Path::new("."));
    if !crate::workspace::is_workspace(deck_dir) {
        bail!(
            "a deck snapshots into its workspace's `assets/`, but {} is not in a \
             workspace (no `alix.toml`).",
            deck.path.display()
        );
    }
    let source = deck
        .sources
        .first()
        .ok_or_else(|| anyhow!("{} declares no `% source:` to snapshot", deck.subject))?;
    if is_url(source) {
        bail!("`{source}` is a URL — there are no local excerpts to snapshot");
    }

    // The live crate this deck froze from — its `% at:` provenance is recorded
    // relative to it so the tutor + drift can find the files. The deck's own
    // resolved source root is the authority; a deck-level `% origin:` is written
    // only when it diverges from the workspace `[defaults] origin`.
    let origin_root = deck.source_root();
    let deck_origin = match (&origin_root, workspace_origin) {
        (Some(o), Some(ws)) if same_path(o, ws) => None, // workspace default covers it
        (Some(o), _) => Some(o.display().to_string()),
        (None, _) => None,
    };

    // Resolve `% at:` locators exactly as the review path does — including a
    // ` + `-joined multi-file `% source:`, which must be split, not treated as one
    // literal path. Sharing `SourceBase` keeps freeze and review in lock-step.
    let source_base = SourceBase::for_deck(deck);
    let assets_dir = deck_dir.join(SNAPSHOT_DIR);
    let mut copied = Vec::new();
    let mut missing = Vec::new();
    // The rewrite for each `% at:` line, in file order. Both a trace's
    // checkpoints and a fact deck's cards cite via `% at:`, so iterating the
    // deck's cards freezes either.
    let mut ats: Vec<crate::deck::AtRewrite> = Vec::new();

    for card in &deck.cards {
        let Some(locator) = card.at.as_deref() else {
            continue;
        };
        match source_base.excerpt(locator) {
            Ok(excerpt) => {
                // `NN.<ext>` — the cited file's extension keeps the snippet
                // readable; `start` keeps the number unique across the shared
                // workspace `assets/` when several decks snapshot into it.
                let n = start + copied.len() + 1;
                let ext = excerpt_ext(&excerpt);
                let name = format!("{n:02}.{ext}");
                write_snippet(&assets_dir.join(&name), &excerpt)?;
                copied.push(name.clone());
                ats.push(crate::deck::AtRewrite {
                    at: name,
                    // The origin-relative `file:lines` rides the `% at:` line as
                    // ` from …`, so the live source stays locatable.
                    origin: excerpt_provenance(&excerpt, origin_root.as_deref()),
                });
            }
            // Keep the original locator if the excerpt can't be read; warn later.
            Err(_) => {
                missing.push(locator.to_string());
                ats.push(crate::deck::AtRewrite {
                    at: locator.to_string(),
                    origin: None,
                });
            }
        }
    }
    if copied.is_empty() {
        bail!(
            "{} has no readable `% at:` excerpts to snapshot",
            deck.subject
        );
    }

    crate::deck::set_trace_snapshot(&deck.path, SNAPSHOT_DIR, deck_origin.as_deref(), &ats)?;

    Ok(SnapshotReport { copied, missing })
}

/// The extension to give a snippet — the cited file's (so `01.rs` stays
/// recognizable), or `txt` when it has none.
fn excerpt_ext(excerpt: &Excerpt) -> String {
    excerpt
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt")
        .to_string()
}

/// The original `file:lines` of an excerpt for the ` from …` provenance, as a
/// path **relative to `origin_root`** (`src/caching.rs:46-66`) so the tutor and
/// drift detection can locate the live file. Falls back to the basename when the
/// excerpt isn't under the origin. Always emitted now (unlike the old basename
/// note) — the origin path matters even when the line numbers didn't shift.
fn excerpt_provenance(excerpt: &Excerpt, origin_root: Option<&Path>) -> Option<String> {
    let first = excerpt.lines.first()?.0;
    let last = excerpt.lines.last()?.0;
    let rel = origin_root
        .and_then(|root| path_relative_to(&excerpt.path, root))
        .or_else(|| {
            excerpt
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "source".to_string());
    Some(if first == last {
        format!("{rel}:{first}")
    } else {
        format!("{rel}:{first}-{last}")
    })
}

/// `path` expressed relative to `root`, canonicalizing both (the files exist at
/// freeze time) so `./` segments and symlinks don't defeat the prefix match.
fn path_relative_to(path: &Path, root: &Path) -> Option<String> {
    let path = path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    path.strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Whether two paths point at the same location, canonicalizing when possible so
/// `./` and symlink differences don't read as divergence.
fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Writes an excerpt's lines (content only, re-based to line 1) to a snippet
/// file, creating `assets/` if needed.
fn write_snippet(dest: &Path, excerpt: &Excerpt) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut body: String = excerpt
        .lines
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    body.push('\n');
    std::fs::write(dest, body).with_context(|| format!("cannot write {}", dest.display()))?;
    Ok(())
}

/// Recon a source and return a ranked menu of candidate traces to author — each
/// a path description, a one-line spine sketch, and a suggested `% source:`
/// scope. Unlike [`build`], it discovers nothing in depth and writes nothing: it
/// surveys the scope once (the same read-only tools and cwd) and proposes
/// *what* is worth tracing, leaving the expensive path discovery to a later
/// `--build` of whichever the learner picks. `source` is a scope directly (a
/// repo `.`, a directory, a file, or a URL), not a deck.
pub fn suggest(source: &str, cfg: &TraceConfig, ask_cfg: &AskConfig) -> Result<String> {
    let url = is_url(source);
    let cwd = if url {
        None
    } else {
        let (base_dir, _) = resolve_source(None, Some(source));
        Some(base_dir)
    };
    let prompt = suggest_prompt(source, url, cfg);
    let run_cfg = build_run_config(cfg, ask_cfg, cwd, url);
    let raw = ask::run(&run_cfg, &prompt, &[])?;
    let menu = raw.trim().to_string();
    if menu.is_empty() {
        bail!("the recon produced no suggestions");
    }
    Ok(menu)
}

/// The CLI runner config for a build: the ask command/permission with trace's
/// own model and (longer) timeout, **read-only** exploration tools, and the
/// source root as the working directory.
pub(crate) fn build_run_config(
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
        effort: cfg.effort.clone().or_else(|| ask_cfg.effort.clone()),
        timeout_secs: cfg.timeout_secs,
        cwd,
        source_access: false,
    }
}

/// Builds the path-discovery prompt: the goal, the scope, how to explore it,
/// the checkpoint format, and the chain-not-a-set rules.
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
         optional) name off-screen symbols the question leans on — alix lists them \
         under the question before the learner predicts. The indented lines under it \
         are the key points the revealed source makes (the rubric). `% at:` is the \
         locator: {locator} — it must point at the REAL lines/passage the key points \
         paraphrase, because alix reads them live at review time as the ground \
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
         used-but-unbound symbol is a CANDIDATE given, and never gloss one the span \
         binds itself. But gloss only what the reader can't DERIVE: a given earns \
         its place when the symbol's meaning or origin is genuinely off-screen and \
         not self-evident. A self-documenting field or parameter whose name already \
         says what it is (`self.subject` on a `Card`) needs none — glossing the \
         obvious just enumerates the answer's ingredients and shrinks the predict \
         gap to nothing. The list MUST be COMPLETE in the honesty sense — no \
         UNEXPLAINED dangling symbol — but that means what the span can't be read \
         without, not glossing every name. alix shows them under the question, \
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
         1. One path, not a set — and stay on the SPINE. Each hop is a step along \
         one chain; if two checkpoints could be reordered without breaking, they \
         are a set — re-trace the spine. Trace the path EVERY instance travels: a \
         step that fires only for some inputs (a conditional branch, an optional \
         transform like direction `both`/`reverse` that a plain forward card skips) \
         is a side-branch, not a spine hop. Keep the main path on what all \
         instances do; a branch worth understanding becomes a SEPARATE (nested) \
         trace, not a detour most instances never take.\n\
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
         7. Each hop must TEACH — answer with the mechanism, not a deferral. The key \
         points must say what ACTUALLY happens, never restate the question or hand off \
         to a callee: \"it calls build_queue to produce the queue\" does NOT answer \
         \"how is the order decided?\" — it just says another function decides. When \
         the real work is inside a function the span merely CALLS, that callee is the \
         hop: dive into it and ask the question THERE; do not frame the call-site's \
         question as if it does the work. A thin delegating layer is folded into an \
         honest handoff (\"the constructor hands queue-building to build_queue\") or \
         skipped. There must be a real gap between a sensible guess and the reveal; if \
         the answer is obvious or circular, cut or re-aim the hop.\n\
         8. The last hop reaches the outcome the path was tracing toward.\n\n\
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

/// Builds the recon prompt for `--suggest`: survey the scope and propose a
/// ranked menu of candidate traces (path + spine sketch + scope) WITHOUT tracing
/// any of them in depth. The cheap counterpart to [`build_prompt`].
fn suggest_prompt(source: &str, url: bool, cfg: &TraceConfig) -> String {
    let explore = if url {
        format!("Read the source page at {source} with the WebFetch tool (fetch it once).")
    } else {
        "Your working directory is the source root. Explore it with the Read, Glob \
         and Grep tools — read it the way you would cold: the manifest (what kind of \
         thing + its stack), then the module names (the domain nouns), then the \
         entry point, then the main path. You can read any file under the source; \
         you have no write or shell access."
            .to_string()
    };
    let scope = if url {
        "the same URL"
    } else {
        "the whole source `.`, or a NARROWER scope (a subdirectory or single file) \
         when a tight path lives in one place"
    };
    let mut p = format!(
        "You are doing RECON on a source so a learner can decide WHAT to understand \
         in it. Do NOT trace any path in depth and do NOT write checkpoints — that \
         is a separate, expensive step the learner runs later on whichever \
         suggestion they pick. Your one job is to SURVEY the scope in a single \
         pass and propose a ranked MENU of the most central traces to START from — \
         the entry points into understanding the source. This is deliberately the \
         STARTING set (the central paths), NOT an exhaustive set that fully covers \
         the source.\n\n\
         Source (the scope): {source}\n{explore}\n\n\
         A *trace* is a path-QUESTION — \"how X becomes Y\" — a real SEQUENCE from a \
         trigger to an outcome (a data flow, a control flow, or a derivation), the \
         kind of thread a learner predicts step by step. It is NOT a topic, a \
         feature list, or a \"goal\" (a bigger, long-term aim that lives at the \
         workspace level, like \"understand this crate\"); each suggestion must \
         name a concrete path with two ends.\n\n\
         COVERAGE, NOT A COUNT — this decides HOW MANY. Do not aim for a number. \
         First identify the major subsystems of the source (its modules, domains, \
         top-level parts). Then emit ONE candidate per major subsystem — its single \
         most load-bearing path — plus the central spine that threads them \
         together. STOP when every major subsystem is covered once: the list is \
         exactly as long as that takes (a source with twelve subsystems yields \
         about twelve; one with four yields about four). Do NOT pad to look \
         thorough, and do NOT drop a real subsystem to look concise. EXCLUDE the \
         local, leaf paths INSIDE a subsystem — those are deeper dives for a later \
         step, not starting points. Each candidate must be sized to be ONE trace — \
         a single spine, not \"understand this whole module\"; if a path is large, \
         narrow its scope rather than widening the question. RANK by centrality: \
         the spine first, then each subsystem's main path.\n\n\
         EDGES vs NODES — a trace drills *edges* (a path predicted hop by hop); some \
         subsystems are mostly *nodes*: a table of facts with no path to predict (a \
         config's knobs, a store's on-disk format). Do NOT force those into traces — \
         that manufactures a fake path; they are better learned as plain facts decks. \
         But make the skip VISIBLE: after the candidates, list the node-shaped \
         subsystems you deliberately left out, one line each with why. Skip trivial \
         utilities silently; only call out real subsystems a learner might expect to \
         see.\n\n\
         FORMAT — output ONLY the menu, no preamble, no code fences. Start with two \
         heading lines, then the numbered candidates:\n\n\
         Source  <one line: what this source is>\n\
         Spine   <the single most central path, as arrow-joined nouns>\n\n\
         1. <the path-question, e.g. how a keypress becomes a saved grade>\n   \
         spine: <3–6 rough hop labels joined by arrows — NOT cited checkpoints>\n   \
         % source: <{scope}>\n\
         2. …\n\n\
         Skipped (node-shaped — facts-deck material, not traces):\n  \
         - <subsystem> — <why it's facts, not a path>\n  \
         - …\n\n\
         The `spine:` is a rough sketch (hop labels only) so the learner can judge \
         the trace at a glance and predict it; do not resolve line numbers, cite \
         excerpts, or write key points — that is what `--build` does next. Keep \
         each path-question one line."
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
pub(crate) fn clean_to_cards(raw: &str) -> String {
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

/// Grades a learner's prediction at a checkpoint with Claude (`alix trace
/// --grade`): compares it to the checkpoint's key points and returns the
/// [`Delta`] plus one line of feedback. Pure reasoning over the supplied text —
/// no tools. Unlike the one-shot `--build`/`--suggest`, this is a light,
/// interactive, per-hop judgment, so it runs at the tutor tier — the `[ask]`
/// model, effort and timeout — not trace's heavy opus + high-effort defaults.
pub fn grade_prediction(
    checkpoint: &Checkpoint,
    prediction: &str,
    ask_cfg: &AskConfig,
) -> Result<(Delta, String)> {
    let run_cfg = AskConfig {
        allowed_tools: Vec::new(), // grading needs no tools, just the text
        cwd: None,
        ..ask_cfg.clone()
    };
    let raw = ask::run(&run_cfg, &grade_prompt(checkpoint, prediction), &[])?;
    parse_grade(&raw)
}

/// Background variant of [`grade_prediction`]: runs the grade on a thread and
/// delivers `(Delta, feedback)` (or an error string) on the returned channel.
/// The web walk server polls it while the reveal shows "grading…", exactly like
/// [`crate::exam::spawn_grade`]; inputs are owned so the thread is `'static`.
pub fn spawn_grade(
    checkpoint: Checkpoint,
    prediction: String,
    ask_cfg: AskConfig,
) -> Receiver<Result<(Delta, String), String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let reply =
            grade_prediction(&checkpoint, &prediction, &ask_cfg).map_err(|e| format!("{e:#}"));
        let _ = tx.send(reply);
    });
    rx
}

/// Builds the grading prompt: the question, the key points (the rubric), and the
/// learner's prediction — asking for a one-line `VERDICT — feedback`.
fn grade_prompt(checkpoint: &Checkpoint, prediction: &str) -> String {
    let points = checkpoint.points.join("\n- ");
    format!(
        "A learner is doing a predict-then-verify walk through a source. At this \
         hop they were asked:\n\n{}\n\nA correct answer hits these KEY POINTS:\n\
         - {points}\n\nThe learner PREDICTED:\n\n{prediction}\n\nGrade the \
         prediction against the key points (minor wording differences are fine; \
         judge the substance). Reply with EXACTLY ONE line: the verdict word, then \
         a dash and ONE short sentence of feedback naming what was right or \
         missing. The verdict is one of:\n\
         PASSED — covers the key points with nothing important wrong\n\
         PARTLY — some right, but an important point is missed, muddled, or \
         stated wrongly\n\
         FAILED — misses the point, or its core claim is wrong\n\
         Do NOT award PASSED to a prediction that asserts something the key points \
         CONTRADICT — a confident error is PARTLY at best (FAILED if the core \
         claim is wrong).\n\
         Example: `PARTLY — right that it reschedules, but you missed the \
         streak reset.`",
        checkpoint.prompt
    )
}

/// Parses a `VERDICT — feedback` grading reply into a [`Delta`] and the feedback
/// text. The verdict must be one of `PASSED` / `PARTLY` / `FAILED` (what the
/// prompt asks for); any other reply is an **error** — a grader that ignores the
/// instruction (a weak local model, say) must not be papered over with a
/// fabricated grade, so the caller aborts the AI grade rather than scoring a hop
/// the model never actually judged.
fn parse_grade(raw: &str) -> Result<(Delta, String)> {
    let line = raw.trim().lines().next().unwrap_or("").trim();
    let upper = line.to_ascii_uppercase();
    let delta = if upper.starts_with("PASSED") {
        Delta::Passed
    } else if upper.starts_with("PARTLY") {
        Delta::Partial
    } else if upper.starts_with("FAILED") {
        Delta::Failed
    } else {
        bail!("the grader did not return a PASSED, PARTLY, or FAILED verdict: {line:?}");
    };
    let feedback = line
        .split_once(['—', '-'])
        .map(|(_, f)| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| line.to_string());
    Ok((delta, feedback))
}

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
    scheduler: SchedulerKind,
    current: usize,
    phase: Phase,
    predictions: Vec<String>,
    deltas: Vec<Option<Delta>>,
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
            self.scheduler
                .scheduler()
                .apply(state, delta.grade(), now_ms);
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
        // Passed advances; partly drops a stage; failed resets — each shares the
        // review grade, so a partly is now a distinct, gentler outcome.
        assert_eq!(Grade::Pass, Delta::Passed.grade());
        assert_eq!(Grade::Partial, Delta::Partial.grade());
        assert_eq!(Grade::Fail, Delta::Failed.grade());
    }

    #[test]
    fn walking_a_trace_drills_but_does_not_master_it() {
        use crate::{
            deck::DeckState,
            store::{MAX_STAGE, Store},
        };
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

        // Walk it correctly enough times to retire the single checkpoint.
        for round in 0..MAX_STAGE + 1 {
            let mut walk = Walk::new(Trace::from_deck(&deck).unwrap(), SchedulerKind::Leitner);
            walk.predict("p".to_string());
            walk.grade(&mut store, Delta::Passed, u64::from(round) + 1);
            assert_eq!(Phase::Done, walk.phase()); // no compress step
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
        let mut walk = Walk::new(trace, SchedulerKind::Leitner);

        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(2, walk.total());

        // Hop 1: predict -> reveal -> Passed advances the checkpoint.
        walk.predict("my guess".to_string());
        assert_eq!(Phase::Reveal, walk.phase());
        assert_eq!(Some("my guess"), walk.prediction(0));
        walk.grade(&mut store, Delta::Passed, 1000);
        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(1, walk.current_index());
        assert_eq!(2, store.get(card0).unwrap().stage); // Passed -> stage up

        // Hop 2 (last): a Failed resets to stage 1 and finishes the walk (no
        // compress step — verification is the separate trace exam).
        let card1 = walk.checkpoint().unwrap().card_id;
        walk.predict(String::new());
        walk.grade(&mut store, Delta::Failed, 1001);
        assert_eq!(Phase::Done, walk.phase());
        assert_eq!(1, store.get(card1).unwrap().stage); // Failed -> reset

        let summary = walk.summary();
        assert_eq!(1, summary.passed);
        assert_eq!(1, summary.failed);
        assert_eq!(vec![1], summary.weak); // the failed hop is the weak edge
    }

    #[test]
    fn partly_demotes_a_checkpoint_one_stage_not_reset() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace, SchedulerKind::Leitner);
        let card0 = walk.checkpoint().unwrap().card_id;
        // Lift the first checkpoint to stage 3, then grade it Partial.
        store.get_or_insert(card0, 0).stage = 3;
        walk.predict("guess".to_string());
        walk.grade(&mut store, Delta::Partial, 1000);
        assert_eq!(2, store.get(card0).unwrap().stage); // partly: 3 -> 2, not reset
        assert_eq!(1, walk.summary().partly);
    }

    #[test]
    fn grade_is_a_noop_outside_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace, SchedulerKind::Leitner);
        // In Predict phase, grading does nothing.
        walk.grade(&mut store, Delta::Passed, 1000);
        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(0, walk.current_index());
        assert!(store.is_empty());
    }

    // ── build (`alix trace --build`) ────────────────────────────────────────

    #[test]
    fn parse_grade_reads_verdict_and_feedback() {
        let (d, f) =
            parse_grade("PARTLY — right that it reschedules, but missed the clamp.").unwrap();
        assert_eq!(Delta::Partial, d);
        assert_eq!("right that it reschedules, but missed the clamp.", f);
        assert_eq!(Delta::Passed, parse_grade("PASSED — spot on").unwrap().0);
        assert_eq!(
            Delta::Failed,
            parse_grade("FAILED - wrong direction").unwrap().0
        );
    }

    #[test]
    fn parse_grade_errors_on_an_unrecognized_verdict() {
        // A grader that ignores the PASSED/PARTLY/FAILED instruction (e.g. a weak
        // local model) must surface an error — never a fabricated grade the model
        // didn't give. The caller then aborts the AI grade (and self-grades).
        assert!(parse_grade("hmm not sure").is_err());
        assert!(parse_grade("").is_err());
    }

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
        assert!(p.contains("must TEACH")); // no vacuous delegation answers
        assert!(p.contains("stay on the SPINE")); // trace the common path, nest branches
        assert!(p.contains("EVERY instance travels")); // side-branches aren't spine hops
        assert!(p.contains("self-documenting")); // gloss only non-derivable givens
        assert!(!p.contains("WebFetch")); // a local source needs no web tool
    }

    #[test]
    fn suggest_prompt_recons_for_a_menu_without_tracing() {
        let p = suggest_prompt(".", false, &TraceConfig::default());
        assert!(p.contains("RECON")); // recon, not a full trace
        assert!(p.contains("Do NOT trace any path in depth")); // no deep tracing
        assert!(p.contains("ranked MENU")); // a menu of candidates
        assert!(p.contains("path-QUESTION")); // each suggestion is a path, not a topic
        assert!(p.contains("COVERAGE, NOT A COUNT")); // count is emergent, not capped
        assert!(p.contains("per major subsystem")); // stop rule: cover each subsystem once
        assert!(p.contains("EDGES vs NODES")); // trace edges, deck nodes
        assert!(p.contains("Skipped (node-shaped")); // name the fact-shaped skips
        assert!(!p.contains("5–8")); // the old arbitrary cap is gone
        assert!(p.contains("by centrality")); // ranked spine-first
        assert!(p.contains("spine:")); // sketch labels, ...
        assert!(p.contains("NOT cited checkpoints")); // ... not full checkpoints
        assert!(p.contains("a \"goal\"")); // distinguish a trace from a future goal/curriculum
        assert!(p.contains("Read, Glob")); // same read-only exploration
        assert!(!p.contains("WebFetch")); // local source needs no web tool
        assert!(!p.contains("% at:")); // recon never emits locators
    }

    #[test]
    fn suggest_prompt_url_uses_webfetch() {
        let p = suggest_prompt("https://x", true, &TraceConfig::default());
        assert!(p.contains("WebFetch"));
        assert!(!p.contains("Glob")); // no local file tools for a URL source
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

    #[test]
    fn clean_to_cards_strips_fence_and_preamble() {
        let raw = "Here is the trace:\n```text\n# Q1\n\tp\n\t% at: 1\n```";
        assert_eq!("# Q1\n\tp\n\t% at: 1", clean_to_cards(raw));
    }

    use crate::testutil::{ask_config, exec_lock, fake_reply};

    #[test]
    fn build_end_to_end_returns_cleaned_cards() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "# Q1\n\tp1\n\t% at: 1\n# Q2\n\tp2\n\t% at: 2\n");
        // A trace deck with `% source: .` (cwd resolves to the temp dir).
        let path = write(dir.path(), "t.txt", "% trace: how it works\n% source: .\n");
        let deck = Deck::load(&path).unwrap();
        let cards = build(&deck, &TraceConfig::default(), &ask_config(&cli)).unwrap();
        assert!(cards.starts_with("# Q1"));
        assert!(cards.contains("# Q2"));
        assert!(cards.contains("% at: 2"));
    }

    // ── snapshotting ────────────────────────────────────────────────────

    /// A workspace (`alix.toml` + deck) at `root/ws` whose trace cites files in
    /// a sibling source tree at `root/src`.
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

    #[test]
    fn snapshot_freezes_excerpts_with_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = snapshot_workspace(root);
        let deck = Deck::load(&deck_path).unwrap();

        let report = snapshot(&deck, 0, None).unwrap();
        assert_eq!(2, report.copied.len());
        assert!(report.missing.is_empty());
        // one small snippet per checkpoint — NOT the whole files
        assert!(root.join("ws/assets/01.rs").is_file());
        assert!(root.join("ws/assets/02.rs").is_file());
        assert!(!root.join("ws/assets/a.rs").exists());
        // the snippet holds only the cited span (a.rs:2-3 → beta, gamma)
        assert_eq!(
            "beta\ngamma\n",
            std::fs::read_to_string(root.join("ws/assets/01.rs")).unwrap()
        );

        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(text.contains("% source: assets\n"), "{text}");
        assert!(text.contains("% origin: "), "{text}"); // the live source root is recorded
        // The provenance rides the `% at:` line, never a `!` note.
        assert!(text.contains("% at: 01.rs from a.rs:2-3\n"), "{text}");
        assert!(text.contains("% at: 02.rs from b.rs:1\n"), "{text}");
        assert!(!text.contains("! from"), "{text}");

        // the reloaded trace reads the re-based excerpt from the snippet
        let frozen = Deck::load(&deck_path).unwrap();
        let trace = Trace::from_deck(&frozen).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(
            vec![(1, "beta".to_string()), (2, "gamma".to_string())],
            ex.lines
        );
    }

    #[test]
    fn drifted_cards_flags_a_changed_or_missing_excerpt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = snapshot_workspace(root);
        snapshot(&Deck::load(&deck_path).unwrap(), 0, None).unwrap();

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

    #[test]
    fn snapshot_drift_is_gone_after_editing_upstream() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = snapshot_workspace(root);
        snapshot(&Deck::load(&deck_path).unwrap(), 0, None).unwrap();
        // Edit the upstream source — even delete it: the frozen snippet is intact.
        std::fs::write(root.join("src/a.rs"), "TOTALLY\nDIFFERENT\n").unwrap();
        let trace = Trace::from_deck(&Deck::load(&deck_path).unwrap()).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(
            vec![(1, "beta".to_string()), (2, "gamma".to_string())],
            ex.lines
        );
    }

    #[test]
    fn snapshot_freezes_single_file_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "notes.md", "L1\nL2\nL3\n");
        std::fs::create_dir_all(root.join("ws")).unwrap();
        write(&root.join("ws"), "alix.toml", "[defaults]\n");
        let deck_path = write(
            &root.join("ws"),
            "t.txt",
            "% trace: t\n% source: ../notes.md\n# hop\n\tp\n\t% at: 2\n",
        );
        let report = snapshot(&Deck::load(&deck_path).unwrap(), 0, None).unwrap();
        assert_eq!(1, report.copied.len());
        assert!(root.join("ws/assets/01.md").is_file());
        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(text.contains("% source: assets\n"), "{text}");
        assert!(text.contains("% at: 01.md from notes.md:2\n"), "{text}");
        assert!(!text.contains("! from"), "{text}");

        let frozen = Deck::load(&deck_path).unwrap();
        let trace = Trace::from_deck(&frozen).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(vec![(1, "L2".to_string())], ex.lines);
    }

    #[test]
    fn snapshot_freezes_a_multi_file_plus_joined_source() {
        // A `% source:` joining several files with ` + ` (the generator's format)
        // must freeze every cited file — snapshot has to split it the same way the
        // review path does, not treat the whole line as one literal path.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        write(&root.join("src"), "a.rs", "alpha\nbeta\ngamma\n");
        write(&root.join("src"), "b.rs", "one\ntwo\n");
        std::fs::create_dir_all(root.join("ws")).unwrap();
        write(&root.join("ws"), "alix.toml", "[defaults]\n");
        let deck_path = write(
            &root.join("ws"),
            "d.txt",
            "% source: ../src/a.rs + b.rs\n\
             # q1\n\tp\n\t% at: a.rs:2-3\n\
             # q2\n\tp\n\t% at: b.rs:1\n",
        );
        let report = snapshot(&Deck::load(&deck_path).unwrap(), 0, None).unwrap();
        assert_eq!(2, report.copied.len(), "both ` + ` files freeze");
        assert!(report.missing.is_empty(), "{:?}", report.missing);
        assert!(root.join("ws/assets/01.rs").is_file());
        assert!(root.join("ws/assets/02.rs").is_file());
    }

    #[test]
    fn snapshot_refuses_non_workspace_and_url() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // not in a workspace (no alix.toml)
        let loose = write(
            root,
            "t.txt",
            "% trace: t\n% source: .\n# h\n\tp\n\t% at: x.rs:1\n",
        );
        let err = snapshot(&Deck::load(&loose).unwrap(), 0, None).unwrap_err();
        assert!(format!("{err:#}").contains("not in a workspace"), "{err:#}");

        // URL source, in a workspace
        std::fs::create_dir_all(root.join("ws")).unwrap();
        write(&root.join("ws"), "alix.toml", "[defaults]\n");
        let url = write(
            &root.join("ws"),
            "u.txt",
            "% trace: t\n% source: https://example.com/p\n# h\n\tp\n\t% at: 1\n",
        );
        let err = snapshot(&Deck::load(&url).unwrap(), 0, None).unwrap_err();
        assert!(format!("{err:#}").contains("URL"), "{err:#}");
    }
}
