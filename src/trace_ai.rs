//! Split out of `trace` so the core (the walk, citation resolution) compiles
//! without the AI backend.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, channel},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    ask,
    backend::{backend_for, ensure_source_reachable},
    config::{AskConfig, TraceConfig},
    deck::{Deck, is_url},
    trace::{Checkpoint, Delta, Excerpt, SNAPSHOT_DIR, SourceBase, resolve_source},
};

pub fn build(deck: &Deck, cfg: &TraceConfig, ask_cfg: &AskConfig) -> Result<String> {
    let description = deck
        .trace
        .as_deref()
        .ok_or_else(|| anyhow!("{} declares no `trace:` to build", deck.subject))?;
    let source = deck
        .sources
        .first()
        .ok_or_else(|| anyhow!("{} declares no `source:` scope to trace", deck.subject))?;
    let url = is_url(source);
    ensure_source_reachable(ask_cfg, url)?;
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

#[derive(Debug)]
pub(crate) struct SnapshotReport {
    pub copied: Vec<String>,
    pub missing: Vec<String>,
}

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
        .ok_or_else(|| anyhow!("{} declares no `source:` to snapshot", deck.subject))?;
    if is_url(source) {
        bail!("`{source}` is a URL — there are no local excerpts to snapshot");
    }

    let origin_root = deck.source_root();
    let deck_origin = match (&origin_root, workspace_origin) {
        (Some(o), Some(ws)) if same_path(o, ws) => None, // workspace default covers it
        (Some(o), _) => Some(o.display().to_string()),
        (None, _) => None,
    };

    let source_base = SourceBase::for_deck(deck);
    let assets_dir = deck_dir.join(SNAPSHOT_DIR);
    let mut copied = Vec::new();
    let mut missing = Vec::new();
    let mut ats: Vec<crate::deck::AtRewrite> = Vec::new();

    for card in &deck.cards {
        let Some(locator) = card.at.as_deref() else {
            continue;
        };
        match source_base.excerpt(locator) {
            Ok(excerpt) => {
                let n = start + copied.len() + 1;
                let ext = excerpt_ext(&excerpt);
                let name = format!("{n:02}.{ext}");
                write_snippet(&assets_dir.join(&name), &excerpt)?;
                copied.push(name.clone());
                ats.push(crate::deck::AtRewrite {
                    at: name,
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
            "{} has no readable `at:` excerpts to snapshot",
            deck.subject
        );
    }

    crate::deck::set_trace_snapshot(&deck.path, SNAPSHOT_DIR, deck_origin.as_deref(), &ats)?;

    Ok(SnapshotReport { copied, missing })
}

fn excerpt_ext(excerpt: &Excerpt) -> String {
    excerpt
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt")
        .to_string()
}

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

fn path_relative_to(path: &Path, root: &Path) -> Option<String> {
    let path = path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    path.strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

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

pub fn suggest(source: &str, cfg: &TraceConfig, ask_cfg: &AskConfig) -> Result<String> {
    let url = is_url(source);
    ensure_source_reachable(ask_cfg, url)?;
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
    let model = cfg
        .model
        .clone()
        .or_else(|| ask_cfg.model.clone())
        .or_else(|| {
            backend_for(ask_cfg)
                .ok()
                .and_then(|b| b.default_trace_model().map(str::to_string))
        });
    AskConfig {
        allowed_tools,
        model,
        effort: cfg.effort.clone().or_else(|| ask_cfg.effort.clone()),
        timeout_secs: cfg.timeout_secs,
        cwd,
        source_access: false,
        ..ask_cfg.clone()
    }
}

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
         relative to the source root, e.g. `src/session.rs:682-689` — NEVER \
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
         FORMAT — output ONLY the checkpoint cards: no frontmatter, no `trace:` or \
         `source:` key, no preamble, no code fences. Each checkpoint is:\n\n\
         ## <the question for this hop, asked plainly>\n\
         <!-- given: <name> — <what it is> -->\n\
         <a key point a correct answer hits>\n\
         <another key point>\n\
         <!-- at: <locator> -->\n\
         > <one connecting insight, shown after the reveal>\n\n\
         The `## ` front (column 0, never indented) is the QUESTION. The \
         `<!-- given: ... -->` lines (repeatable, optional) name off-screen symbols \
         the question leans on — alix lists them under the question before the \
         learner predicts. The plain (unindented) lines under it are the key points \
         the revealed source makes (the rubric). `<!-- at: ... -->` is the \
         locator: {locator} — it must point at the REAL lines/passage the key points \
         paraphrase, because alix reads them live at review time as the ground \
         truth. Cite accurately. The `> ` line is an optional note.\n\n\
         SCOPE EACH HOP TO A SELF-CONTAINED UNIT, AND GLOSS WHAT YOU DON'T SHOW. The \
         reader sees ONLY the lines you cite, so an excerpt must read on its own. \
         Prefer hops that are a whole SMALL function/method — its inputs are its \
         parameters, so nothing dangles. Do NOT dissect one big function into several \
         checkpoints: a big function on the path is ONE black-box hop — cite its \
         signature plus the load-bearing line(s) and describe what it does in the key \
         points; if its internals are themselves worth understanding, that is a \
         SEPARATE trace, not more hops here.\n\
         The `at:` locator is ONE CONTIGUOUS RANGE. NEVER stitch several ranges together \
         (no commas): collapsing the gaps makes lines from different branches/places \
         look adjacent, which misleads. If a hop cannot be shown in one contiguous \
         span, it spans more than one region — that is the signal it is too big: \
         split it, or black-box the function. \n\
         GLOSS — completely and correctly. A `given:` names a free variable: a \
         symbol the cited span USES but does NOT BIND within those lines — typically \
         a function PARAMETER (declared in the signature, above the body you cite) or \
         a value from an enclosing/earlier scope. Apply a mechanical test to each \
         symbol: if its binding (a `let`, an assignment, a `for x in`, the parameter \
         itself) is INSIDE the cited lines, it is NOT a given — the reader sees it, \
         do not gloss it; if the span uses it but its binding is OUTSIDE, it IS a \
         given. (E.g. for a function body excerpt, the parameters are givens; a \
         `let x = …` on a cited line is not.) Name each given with a \
         `<!-- given: ... -->` line, one per symbol, `name — what it is` (e.g. \
         `<!-- given: defaults — the workspace directive defaults, a parameter -->`). \
         Check BOTH directions: every \
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
         source: <{scope}>\n\
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

pub(crate) fn clean_to_cards(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(start) = lines.iter().position(|l| l.starts_with("## ")) else {
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

/// Errors on anything other than PASSED/PARTLY/FAILED: never fabricates a
/// grade the model didn't actually give.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::Trace;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        if name.ends_with(".md") {
            let _ = crate::stamp::stamp_deck(&path);
        }
        path
    }

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
        assert!(parse_grade("hmm not sure").is_err());
        assert!(parse_grade("").is_err());
    }

    #[test]
    fn build_prompt_carries_goal_source_format_and_rules() {
        let p = build_prompt("how X becomes Y", ".", false, &TraceConfig::default());
        assert!(p.contains("how X becomes Y"));
        assert!(p.contains("Source (the scope): ."));
        assert!(p.contains("Read, Glob"));
        assert!(p.contains("file:start-end"));
        assert!(p.contains("ONE CONTIGUOUS RANGE"));
        assert!(p.contains("## <the question"));
        assert!(p.contains("<!-- at:"));
        assert!(p.contains("black-box hop"));
        assert!(p.contains("free variable"));
        assert!(p.contains("<!-- given:"));
        assert!(p.contains("MUST be COMPLETE"));
        assert!(p.contains("does NOT BIND"));
        assert!(p.contains("KEY POINTS MUST BE GROUNDED"));
        assert!(p.contains("One path, not a set"));
        assert!(p.contains("Carry the STATE"));
        assert!(p.contains("Do NOT prefix fronts with \"Predict\""));
        assert!(p.contains("Dives must return"));
        assert!(p.contains("must TEACH"));
        assert!(p.contains("stay on the SPINE"));
        assert!(p.contains("EVERY instance travels"));
        assert!(p.contains("self-documenting"));
        assert!(!p.contains("WebFetch"));
    }

    #[test]
    fn suggest_prompt_recons_for_a_menu_without_tracing() {
        let p = suggest_prompt(".", false, &TraceConfig::default());
        assert!(p.contains("RECON"));
        assert!(p.contains("Do NOT trace any path in depth"));
        assert!(p.contains("ranked MENU"));
        assert!(p.contains("path-QUESTION"));
        assert!(p.contains("COVERAGE, NOT A COUNT"));
        assert!(p.contains("per major subsystem"));
        assert!(p.contains("EDGES vs NODES"));
        assert!(p.contains("Skipped (node-shaped"));
        assert!(!p.contains("5–8"));
        assert!(p.contains("by centrality"));
        assert!(p.contains("spine:"));
        assert!(p.contains("NOT cited checkpoints"));
        assert!(p.contains("a \"goal\""));
        assert!(p.contains("Read, Glob"));
        assert!(!p.contains("WebFetch"));
    }

    #[test]
    fn suggest_prompt_url_uses_webfetch() {
        let p = suggest_prompt("https://x", true, &TraceConfig::default());
        assert!(p.contains("WebFetch"));
        assert!(!p.contains("Glob"));
    }

    #[test]
    fn build_prompt_url_uses_webfetch_and_quoted_span() {
        let p = build_prompt("how X", "https://x", true, &TraceConfig::default());
        assert!(p.contains("WebFetch"));
        assert!(p.contains("quoted span"));
        assert!(!p.contains("Glob"));
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
        assert_eq!(600, cfg.timeout_secs);
    }

    #[test]
    fn trace_defaults_to_opus_on_claude_none_elsewhere() {
        use crate::config::BackendKind;

        let claude = build_run_config(&TraceConfig::default(), &AskConfig::default(), None, false);
        assert_eq!(Some("opus".to_string()), claude.model);

        let gemini_ask = AskConfig {
            backend: BackendKind::Gemini,
            ..AskConfig::default()
        };
        let gemini = build_run_config(&TraceConfig::default(), &gemini_ask, None, false);
        assert_eq!(None, gemini.model);

        let pinned = TraceConfig {
            model: Some("sonnet".to_string()),
            ..TraceConfig::default()
        };
        let cfg = build_run_config(&pinned, &AskConfig::default(), None, false);
        assert_eq!(Some("sonnet".to_string()), cfg.model);

        let ask = AskConfig {
            model: Some("haiku".to_string()),
            ..AskConfig::default()
        };
        let cfg = build_run_config(&TraceConfig::default(), &ask, None, false);
        assert_eq!(Some("haiku".to_string()), cfg.model);
    }

    #[test]
    fn clean_to_cards_strips_fence_and_preamble() {
        let raw = "Here is the trace:\n```text\n## Q1\np\n<!-- at: 1 -->\n```";
        assert_eq!("## Q1\np\n<!-- at: 1 -->", clean_to_cards(raw));
    }

    use crate::testutil::{ask_config, exec_lock, fake_reply};

    #[test]
    fn build_end_to_end_returns_cleaned_cards() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            "## Q1\np1\n<!-- at: 1 -->\n## Q2\np2\n<!-- at: 2 -->\n",
        );
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: how it works\nsource: .\n---\n",
        );
        let deck = Deck::load(&path).unwrap();
        let cards = build(&deck, &TraceConfig::default(), &ask_config(&cli)).unwrap();
        assert!(cards.starts_with("## Q1"));
        assert!(cards.contains("## Q2"));
        assert!(cards.contains("<!-- at: 2 -->"));
    }

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
            "---\ntrace: how it works\nsource: ../src\n---\n\
             ## hop 1\nit reads a\n<!-- at: a.rs:2-3 -->\n\
             ## hop 2\nit reads b\n<!-- at: b.rs:1 -->\n",
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
        assert!(root.join("ws/assets/01.rs").is_file());
        assert!(root.join("ws/assets/02.rs").is_file());
        assert!(!root.join("ws/assets/a.rs").exists());
        assert_eq!(
            "beta\ngamma\n",
            std::fs::read_to_string(root.join("ws/assets/01.rs")).unwrap()
        );

        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(text.contains("source: assets\n"), "{text}");
        assert!(text.contains("origin: "), "{text}");
        assert!(
            text.contains("<!-- at: 01.rs from a.rs:2-3 -->\n"),
            "{text}"
        );
        assert!(text.contains("<!-- at: 02.rs from b.rs:1 -->\n"), "{text}");
        assert!(!text.contains("> from"), "{text}");

        let frozen = Deck::load(&deck_path).unwrap();
        let trace = Trace::from_deck(&frozen).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(
            vec![(1, "beta".to_string()), (2, "gamma".to_string())],
            ex.lines
        );
    }

    #[test]
    fn snapshot_drift_is_gone_after_editing_upstream() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = snapshot_workspace(root);
        snapshot(&Deck::load(&deck_path).unwrap(), 0, None).unwrap();
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
            "t.md",
            "---\ntrace: t\nsource: ../notes.md\n---\n## hop\np\n<!-- at: 2 -->\n",
        );
        let report = snapshot(&Deck::load(&deck_path).unwrap(), 0, None).unwrap();
        assert_eq!(1, report.copied.len());
        assert!(root.join("ws/assets/01.md").is_file());
        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(text.contains("source: assets\n"), "{text}");
        assert!(
            text.contains("<!-- at: 01.md from notes.md:2 -->\n"),
            "{text}"
        );
        assert!(!text.contains("> from"), "{text}");

        let frozen = Deck::load(&deck_path).unwrap();
        let trace = Trace::from_deck(&frozen).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(vec![(1, "L2".to_string())], ex.lines);
    }

    #[test]
    fn snapshot_freezes_a_multi_file_plus_joined_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        write(&root.join("src"), "a.rs", "alpha\nbeta\ngamma\n");
        write(&root.join("src"), "b.rs", "one\ntwo\n");
        std::fs::create_dir_all(root.join("ws")).unwrap();
        write(&root.join("ws"), "alix.toml", "[defaults]\n");
        let deck_path = write(
            &root.join("ws"),
            "d.md",
            "---\nsource: ../src/a.rs + b.rs\n---\n\
             ## q1\np\n<!-- at: a.rs:2-3 -->\n\
             ## q2\np\n<!-- at: b.rs:1 -->\n",
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
        let loose = write(
            root,
            "t.md",
            "---\ntrace: t\nsource: .\n---\n## h\np\n<!-- at: x.rs:1 -->\n",
        );
        let err = snapshot(&Deck::load(&loose).unwrap(), 0, None).unwrap_err();
        assert!(format!("{err:#}").contains("not in a workspace"), "{err:#}");

        std::fs::create_dir_all(root.join("ws")).unwrap();
        write(&root.join("ws"), "alix.toml", "[defaults]\n");
        let url = write(
            &root.join("ws"),
            "u.md",
            "---\ntrace: t\nsource: https://example.com/p\n---\n## h\np\n<!-- at: 1 -->\n",
        );
        let err = snapshot(&Deck::load(&url).unwrap(), 0, None).unwrap_err();
        assert!(format!("{err:#}").contains("URL"), "{err:#}");
    }
}
