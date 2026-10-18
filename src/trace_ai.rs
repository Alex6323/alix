//! Split out of `trace` so the core trace compiles without the AI backend.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::{
    ask,
    backend::{backend_for, ensure_source_reachable},
    cite::GeneratedRoot,
    config::{AskConfig, TraceConfig},
    deck::{Deck, is_url},
    source::resolve_source,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltTrace {
    pub cards: String,
    pub cite: Option<(String, String)>,
    pub passages: Vec<FrozenPassage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenPassage {
    pub object: String,
    pub text: String,
}

impl BuiltTrace {
    pub fn deck_text(&self, path: &Path, existing: &str) -> Result<String> {
        let text = crate::deck::trace_checkpoint_text(path, existing, &self.cards)?;
        Ok(match &self.cite {
            Some((key, value)) => crate::deck::with_added_cite(&text, key, value)?,
            None => text,
        })
    }

    pub fn write_passages(&self, deck_path: &Path) -> Result<()> {
        if self.passages.is_empty() {
            return Ok(());
        }
        let workspace = crate::workspace::root_for_deck(deck_path).ok_or_else(|| {
            anyhow!(
                "{} is not a workspace member, so its passages have no asset folder",
                deck_path.display()
            )
        })?;
        let deck = Deck::load(deck_path)?;
        let deck_id = deck
            .deck_token
            .as_deref()
            .ok_or_else(|| anyhow!("{} has no deck id to own its passages", deck_path.display()))?;
        for passage in &self.passages {
            crate::assets::write_object(&workspace, deck_id, passage.text.as_bytes(), "txt")?;
        }
        Ok(())
    }
}

pub(crate) struct TraceRoot {
    pub cwd: Option<PathBuf>,
    pub root: GeneratedRoot,
    pub passage_token: Option<String>,
}

pub(crate) enum Citing<'a> {
    Lines(&'a GeneratedRoot),
    Passages { key: &'a str, token: &'a str },
}

impl Citing<'_> {
    pub(crate) fn is_url(&self) -> bool {
        matches!(self, Citing::Passages { .. })
    }
}

pub(crate) fn passage_rule(key: &str, token: &str) -> String {
    format!(
        "exactly `{key}`, the page's citation key. The passage itself goes on the \
         lines directly below that locator line: first a line `PASSAGE {token}`, then \
         the passage the key points rest on, copied VERBATIM from the page as one or \
         more whole lines, then a line `END PASSAGE {token}`. Write both marker lines \
         exactly as shown, each alone on its line. Every checkpoint carries one such \
         passage, placed before its `> [!NOTE]` note"
    )
}

impl TraceRoot {
    pub(crate) fn url(&self) -> bool {
        self.passage_token.is_some()
    }

    pub(crate) fn citing(&self) -> Citing<'_> {
        match &self.passage_token {
            Some(token) => Citing::Passages {
                key: &self.root.key,
                token,
            },
            None => Citing::Lines(&self.root),
        }
    }

    pub(crate) fn cite(&self) -> Option<(String, String)> {
        Some((self.root.key.clone(), self.root.declare.clone()?))
    }

    pub(crate) fn finish(&self, raw: &str, empty: &str) -> Result<BuiltTrace> {
        let (text, passages) = match &self.passage_token {
            Some(token) => cut_passages(raw, &self.root.key, token)?,
            None => (raw.to_string(), Vec::new()),
        };
        let cards = clean_to_cards(&text);
        if cards.trim().is_empty() {
            bail!("{empty}");
        }
        Ok(BuiltTrace {
            cards,
            cite: self.cite(),
            passages,
        })
    }
}

pub(crate) fn trace_root(
    deck_path: &Path,
    deck_cite: &[(String, String)],
    source: &str,
) -> Result<TraceRoot> {
    if is_url(source) {
        if crate::workspace::root_for_deck(deck_path).is_none() {
            bail!(
                "{} traces the url {source}, whose cited passages are frozen into a workspace; only a workspace member deck can trace a url",
                deck_path.display()
            );
        }
        let root = crate::cite::generated_url_root_for_deck(deck_path, deck_cite, source)?;
        let token = crate::token::mint()
            .map_err(|error| anyhow!("cannot mint a passage token: {error}"))?;
        return Ok(TraceRoot {
            cwd: None,
            root,
            passage_token: Some(token),
        });
    }
    let content_root = crate::workspace::content_root(deck_path);
    let (base_dir, file) = resolve_source(Some(&content_root), Some(source));
    let declare_as = match file {
        Some(_) => Path::new(source)
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .filter(|parent| !parent.is_empty())
            .unwrap_or_else(|| ".".to_string()),
        None => source.to_string(),
    };
    let root = crate::cite::generated_root_for_deck(deck_path, deck_cite, &base_dir, &declare_as)?;
    Ok(TraceRoot {
        cwd: Some(base_dir),
        root,
        passage_token: None,
    })
}

pub(crate) fn cut_passages(
    raw: &str,
    key: &str,
    token: &str,
) -> Result<(String, Vec<FrozenPassage>)> {
    let locator_line = format!("<!-- at: {key} -->");
    let open = format!("PASSAGE {token}");
    let close = format!("END PASSAGE {token}");
    let content = |line: &'_ str| -> String {
        let line = line.strip_suffix('\n').unwrap_or(line);
        line.strip_suffix('\r').unwrap_or(line).to_string()
    };
    let mut kept = String::new();
    let mut passages = Vec::new();
    let mut card: Option<String> = None;
    let mut card_cited = true;
    let mut lines = raw.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let text = content(line);
        if let Some(front) = text.strip_prefix("## ") {
            if let Some(previous) = &card
                && !card_cited
            {
                bail!("checkpoint `{previous}` has no `PASSAGE {token}` passage");
            }
            card = Some(front.trim().to_string());
            card_cited = false;
            kept.push_str(line);
            continue;
        }
        let name = card
            .clone()
            .unwrap_or_else(|| "(before the first checkpoint)".to_string());
        if text == open || text == close {
            bail!("checkpoint `{name}`: a passage marker stands outside a passage");
        }
        if text != locator_line {
            kept.push_str(line);
            continue;
        }
        if card.is_none() {
            bail!("a `{locator_line}` passage sits before the first checkpoint");
        }
        if lines.next().map(content).as_deref() != Some(open.as_str()) {
            bail!("checkpoint `{name}`: the line after `{locator_line}` is not `{open}`");
        }
        let mut passage = String::new();
        loop {
            match lines.next() {
                None => bail!("checkpoint `{name}`: its passage never reaches an `{close}` line"),
                Some(line) if content(line) == close => break,
                Some(line) => passage.push_str(line),
            }
        }
        if passage.trim().is_empty() {
            bail!("checkpoint `{name}`: its passage is empty");
        }
        let object = crate::assets::object_name(passage.as_bytes(), "txt");
        let comment = crate::locator::format_locator_comment(&crate::locator::LocatorFields {
            locator: crate::locator::Locator {
                key: key.to_string(),
                target: crate::locator::Target::Passage,
            },
            fingerprint: Some(crate::source::format_locator_fingerprint(
                crate::source::text_fingerprint(&passage),
            )),
            asset: Some(object.clone()),
        })?;
        kept.push_str(&comment);
        kept.push('\n');
        passages.push(FrozenPassage {
            object,
            text: passage,
        });
        card_cited = true;
    }
    if let Some(last) = &card
        && !card_cited
    {
        bail!("checkpoint `{last}` has no `PASSAGE {token}` passage");
    }
    Ok((kept, passages))
}

pub fn build(deck: &Deck, cfg: &TraceConfig, ask_cfg: &AskConfig) -> Result<BuiltTrace> {
    let description = deck
        .trace
        .as_deref()
        .ok_or_else(|| anyhow!("{} declares no `trace:` to build", deck.subject))?;
    let source = deck
        .exam
        .first()
        .ok_or_else(|| anyhow!("{} declares no `exam:` scope to trace", deck.subject))?;
    let root = trace_root(&deck.path, &deck.cite, source)?;
    ensure_source_reachable(ask_cfg, root.url())?;
    let prompt = build_prompt(description, source, &root.citing(), cfg);
    let run_cfg = build_run_config(cfg, ask_cfg, root.cwd.clone(), root.url());
    let raw = ask::run(&run_cfg, &prompt, &[])?;
    root.finish(&raw, "the build produced no checkpoints")
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

fn build_prompt(description: &str, source: &str, citing: &Citing, cfg: &TraceConfig) -> String {
    let explore = if citing.is_url() {
        format!("Read the source page at {source} with the WebFetch tool (fetch it once).")
    } else {
        "Your working directory is the source root. Explore it with the Read, Glob \
         and Grep tools — start at the entry point or the most load-bearing file and \
         follow the references. You can read any file under the source; you have no \
         write or shell access."
            .to_string()
    };
    let locator = match citing {
        Citing::Passages { key, token } => passage_rule(key, token),
        Citing::Lines(root) => format!(
            "ONE contiguous range, `{}` (or `{}` for a single line) where `<file>` is \
             relative to the source root, e.g. `{}`. Write everything around `<file>` \
             exactly as given; a path containing a space is wrapped in double quotes, \
             e.g. `{}`. NEVER comma-separated ranges",
            root.locator("<file>", "start-end"),
            root.locator("<file>", "N"),
            root.locator("src/session.rs", "682-689"),
            root.locator("my notes.md", "3-4"),
        ),
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
         `exam:` key, no preamble, no code fences. Each checkpoint is:\n\n\
         ## <the question for this hop, asked plainly>\n\
         <!-- given: <name> — <what it is> -->\n\
         <a key point a correct answer hits>\n\
         <another key point>\n\
         <!-- at: <locator> -->\n\
         > [!NOTE]\n\
         > <one connecting insight, shown after the reveal>\n\n\
         The `## ` front (column 0, never indented) is the QUESTION. The \
         `<!-- given: ... -->` lines (repeatable, optional) name off-screen symbols \
         the question leans on — alix lists them under the question before the \
         learner predicts. The plain (unindented) lines under it are the key points \
         the revealed source makes (the rubric). `<!-- at: ... -->` is the \
         locator: {locator} — it must point at the REAL lines/passage the key points \
         paraphrase, because alix reads them live at review time as the ground \
         truth. Cite accurately. A `> [!NOTE]` line opens an optional note, with its body on `> ` lines below it.\n\n\
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
    let end = lines[start + 1..]
        .iter()
        .rposition(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("```")
        })
        .map_or(start + 1, |offset| start + offset + 2);
    lines[start..end].join("\n")
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::Path;

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

    fn root(key: &str, path_prefix: &str) -> GeneratedRoot {
        GeneratedRoot {
            key: key.into(),
            path_prefix: path_prefix.into(),
            declare: None,
        }
    }

    #[test]
    fn cut_passages_freezes_each_passage_verbatim_or_fails_naming_its_card() {
        let card = |front: &str, body: &str| {
            format!("## {front}\npoint\n<!-- at: src -->\nPASSAGE tok\n{body}END PASSAGE tok\n")
        };
        type Row = (
            &'static str,
            String,
            Result<Vec<&'static str>, &'static str>,
        );
        let rows: Vec<Row> = vec![
            (
                "a passage containing code fences",
                card("Q", "```rust\nfn a() {}\n```\n"),
                Ok(vec!["```rust\nfn a() {}\n```\n"]),
            ),
            (
                "a passage of `>` lines and a heading",
                card("Q", "> quoted\n## not a card\n"),
                Ok(vec!["> quoted\n## not a card\n"]),
            ),
            (
                "an END PASSAGE line with a different token is content",
                card("Q", "a\nEND PASSAGE other\nb\n"),
                Ok(vec!["a\nEND PASSAGE other\nb\n"]),
            ),
            (
                "blank lines inside and around the passage are kept",
                card("Q", "\nfirst\n\nsecond\n\n"),
                Ok(vec!["\nfirst\n\nsecond\n\n"]),
            ),
            (
                "two checkpoints each freeze their own passage",
                format!("preamble\n{}\n{}", card("Q1", "one\n"), card("Q2", "two\n")),
                Ok(vec!["one\n", "two\n"]),
            ),
            (
                "a passage that never ends",
                "## Q\npoint\n<!-- at: src -->\nPASSAGE tok\nbody\nEND PASSAGE tox\n".into(),
                Err("checkpoint `Q`: its passage never reaches an `END PASSAGE tok` line"),
            ),
            (
                "a wrong token on the opening line",
                "## Q\npoint\n<!-- at: src -->\nPASSAGE tox\nbody\nEND PASSAGE tok\n".into(),
                Err("checkpoint `Q`: the line after `<!-- at: src -->` is not `PASSAGE tok`"),
            ),
            (
                "a checkpoint without a passage",
                format!("{}## Q2\npoint\n", card("Q1", "one\n")),
                Err("checkpoint `Q2` has no `PASSAGE tok` passage"),
            ),
            (
                "an empty passage",
                card("Q", "\n"),
                Err("checkpoint `Q`: its passage is empty"),
            ),
            (
                "a marker line outside a passage",
                format!("{}END PASSAGE tok\n", card("Q", "one\n")),
                Err("checkpoint `Q`: a passage marker stands outside a passage"),
            ),
        ];
        for (label, raw, expected) in rows {
            let cut = cut_passages(&raw, "src", "tok");
            match expected {
                Err(message) => assert_eq!(
                    Some(message.to_string()),
                    cut.err().map(|error| error.to_string()),
                    "row `{label}`"
                ),
                Ok(texts) => {
                    let (kept, passages) =
                        cut.unwrap_or_else(|error| panic!("row `{label}`: {error:#}"));
                    assert_eq!(
                        texts,
                        passages
                            .iter()
                            .map(|passage| passage.text.as_str())
                            .collect::<Vec<_>>(),
                        "row `{label}` passages"
                    );
                    assert!(!kept.contains("PASSAGE tok"), "row `{label}`: {kept}");
                    for passage in &passages {
                        assert_eq!(
                            crate::assets::object_name(passage.text.as_bytes(), "txt"),
                            passage.object,
                            "row `{label}` object is content-addressed"
                        );
                        let locator = format!(
                            "<!-- at: src fingerprint: {} asset: {} -->\n",
                            crate::source::format_locator_fingerprint(
                                crate::source::text_fingerprint(&passage.text)
                            ),
                            passage.object
                        );
                        assert!(kept.contains(&locator), "row `{label}`: {kept}");
                    }
                    let parsed = crate::parser::parse("t.md", &clean_to_cards(&kept))
                        .unwrap_or_else(|error| panic!("row `{label}` parses: {error}"));
                    assert_eq!(
                        passages.len(),
                        parsed
                            .cards
                            .iter()
                            .map(|card| card.citations.len())
                            .sum::<usize>(),
                        "row `{label}` every passage is a citation"
                    );
                }
            }
        }
    }

    #[test]
    fn build_prompt_carries_goal_source_format_and_rules() {
        let p = build_prompt(
            "how X becomes Y",
            ".",
            &Citing::Lines(&root("code", "crates/core/")),
            &TraceConfig::default(),
        );
        assert!(p.contains("how X becomes Y"));
        assert!(p.contains("Source (the scope): ."));
        assert!(p.contains("Read, Glob"));
        assert!(p.contains("`code:crates/core/<file>:start-end`"));
        assert!(p.contains("`code:crates/core/src/session.rs:682-689`"));
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
    fn build_prompt_example_writes_a_badged_note() {
        let prompt = build_prompt(
            "how X becomes Y",
            ".",
            &Citing::Lines(&root("src", "")),
            &TraceConfig::default(),
        );

        assert!(
            prompt.contains("> [!NOTE]\n> <one connecting insight, shown after the reveal>"),
            "the example must model the note grammar the prose requires: {prompt}"
        );
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
    fn suggest_prompt_appends_nonempty_trace_instructions() {
        let cfg = TraceConfig {
            extra: Some("map the ownership boundary".to_string()),
            ..TraceConfig::default()
        };

        let p = suggest_prompt(".", false, &cfg);
        assert!(p.contains("Additional instructions:"));
        assert!(p.contains("map the ownership boundary"));
    }

    #[test]
    fn build_prompt_url_uses_webfetch_and_token_marked_passages() {
        let p = build_prompt(
            "how X",
            "https://x",
            &Citing::Passages {
                key: "src",
                token: "tok",
            },
            &TraceConfig::default(),
        );
        assert!(p.contains("WebFetch"));
        assert!(p.contains("exactly `src`"));
        assert!(p.contains("a line `PASSAGE tok`"));
        assert!(p.contains("a line `END PASSAGE tok`"));
        assert!(!p.contains("Glob"));
    }

    #[test]
    fn build_prompt_quotes_a_prefix_containing_a_space() {
        let p = build_prompt(
            "how X",
            ".",
            &Citing::Lines(&root("code", "my dir/")),
            &TraceConfig::default(),
        );
        assert!(p.contains("`code:\"my dir/<file>\":start-end`"), "{p}");
        assert!(p.contains("`code:\"my dir/my notes.md\":3-4`"), "{p}");
    }

    #[test]
    fn build_prompt_appends_extra() {
        let cfg = TraceConfig {
            extra: Some("trace the read path".to_string()),
            ..TraceConfig::default()
        };
        let p = build_prompt("g", ".", &Citing::Lines(&root("src", "")), &cfg);
        assert!(p.contains("Additional instructions:"));
        assert!(p.contains("trace the read path"));
    }

    #[test]
    fn build_run_config_uses_readonly_tools_and_cwd() {
        let cwd = PathBuf::from("/some/src");
        let trace_cfg = TraceConfig {
            effort: Some("trace-effort".to_string()),
            ..TraceConfig::default()
        };
        let ask_cfg = AskConfig {
            effort: Some("ask-effort".to_string()),
            source_access: true,
            ..AskConfig::default()
        };
        let cfg = build_run_config(&trace_cfg, &ask_cfg, Some(cwd.clone()), false);
        assert_eq!(vec!["Read", "Glob", "Grep"], cfg.allowed_tools);
        assert_eq!(Some(cwd), cfg.cwd);
        assert_eq!(Some("trace-effort".to_string()), cfg.effort);
        assert!(!cfg.source_access);
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

    /// The cleaner trims to the first card front, and a `### ` front is
    /// one: amputating sub-cards would silently drop generated material.
    #[test]
    fn clean_to_cards_keeps_sub_cards() {
        let raw = "chatter\n## a\n1\n### b\n2\n#### c\n3\n";
        let cleaned = clean_to_cards(raw);
        assert!(cleaned.starts_with("## a"), "{cleaned:?}");
        assert!(cleaned.contains("### b"), "{cleaned:?}");
        assert!(cleaned.contains("#### c"), "{cleaned:?}");
    }

    #[test]
    fn clean_to_cards_strips_fence_and_preamble() {
        let raw = "Here is the trace:\n```text\n## Q1\np\n<!-- at: src:a.rs:1 -->\n```";
        assert_eq!("## Q1\np\n<!-- at: src:a.rs:1 -->", clean_to_cards(raw));
    }

    #[test]
    fn clean_to_cards_keeps_a_front_when_every_trailing_line_is_noise() {
        assert_eq!("## Q1", clean_to_cards("preamble\n## Q1\n\n```"));
    }

    use crate::testutil::{ask_config, exec_lock, fake_reply};

    #[test]
    fn build_end_to_end_returns_cleaned_cards() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            "## Q1\np1\n<!-- at: src:a.rs:1 -->\n## Q2\np2\n<!-- at: src:a.rs:2 -->\n",
        );
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: how it works\nexam: .\n---\n",
        );
        let deck = Deck::load(&path).unwrap();
        let built = build(&deck, &TraceConfig::default(), &ask_config(&cli)).unwrap();
        assert!(built.cards.starts_with("## Q1"));
        assert!(built.cards.contains("## Q2"));
        assert!(built.cards.contains("<!-- at: src:a.rs:2 -->"));
        assert_eq!(
            Some(("src".to_string(), ".".to_string())),
            built.cite,
            "a loose deck tracing its own folder declares it as `src`"
        );
    }

    fn freeze_workspace(root: &Path) -> PathBuf {
        std::fs::create_dir_all(root.join("src")).unwrap();
        write(&root.join("src"), "a.rs", "alpha\nbeta\ngamma\n");
        write(&root.join("src"), "b.rs", "one\ntwo\n");
        std::fs::create_dir_all(root.join("ws")).unwrap();
        write(
            &root.join("ws"),
            "alix.toml",
            "title = \"W\"\nexam = \"../src\"\n",
        );
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(
            &root.join("ws/decks"),
            "t.md",
            "---\ntrace: how it works\nexam: ../src\ncite:\n  - src: ../src\n---\n\
             ## hop 1\nit reads a\n<!-- at: src:a.rs:2-3 -->\n\
             ## hop 2\nit reads b\n<!-- at: src:b.rs:1 -->\n",
        )
    }

    #[test]
    fn immediate_freezing_keeps_trace_excerpts_with_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = freeze_workspace(root);
        let report = crate::assets::initialize(&deck_path).unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let deck_id = deck.deck_token.as_deref().unwrap();
        let first = crate::assets::object_name(b"beta\ngamma\n", "rs");
        let second = crate::assets::object_name(b"one\n", "rs");

        assert_eq!(2, report.freeze.unwrap().evidence);
        assert!(root.join(format!("ws/assets/{deck_id}/{first}")).is_file());
        assert!(root.join(format!("ws/assets/{deck_id}/{second}")).is_file());
        assert!(!root.join("ws/assets/a.rs").exists());
        assert_eq!(
            "beta\ngamma\n",
            std::fs::read_to_string(root.join(format!("ws/assets/{deck_id}/{first}"))).unwrap()
        );

        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(
            text.contains("exam: ../src"),
            "the live source declaration stays untouched: {text}"
        );
        assert!(!text.contains("origin:"), "freezing stamps nothing: {text}");
        assert!(
            text.contains("<!-- at: src:a.rs:2-3 fingerprint: xxh64-")
                && text.contains(&format!(" asset: {first} -->\n")),
            "{text}"
        );
        assert!(
            text.contains("<!-- at: src:b.rs:1 fingerprint: xxh64-")
                && text.contains(&format!(" asset: {second} -->\n")),
            "{text}"
        );

        let frozen = Deck::load(&deck_path).unwrap();
        let trace = Trace::from_deck(&frozen).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(
            vec![(2, "beta".to_string()), (3, "gamma".to_string())],
            ex.lines
        );
    }

    #[test]
    fn frozen_trace_is_unchanged_after_editing_upstream() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = freeze_workspace(root);
        crate::assets::initialize(&deck_path).unwrap();
        std::fs::write(root.join("src/a.rs"), "TOTALLY\nDIFFERENT\n").unwrap();
        let trace = Trace::from_deck(&Deck::load(&deck_path).unwrap()).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(
            vec![(2, "beta".to_string()), (3, "gamma".to_string())],
            ex.lines
        );
    }

    #[test]
    fn immediate_freezing_freezes_the_cited_excerpt_of_a_single_file_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "notes.md", "L1\nL2\nL3\n");
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(&root.join("ws"), "alix.toml", "[defaults]\n");
        let deck_path = write(
            &root.join("ws/decks"),
            "t.md",
            "---\ntrace: t\nexam: ../notes.md\ncite:\n  - src: ..\n---\n## hop\np\n<!-- at: src:notes.md:2 -->\n",
        );
        let report = crate::assets::initialize(&deck_path).unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let deck_id = deck.deck_token.as_deref().unwrap();
        let whole_name = crate::assets::object_name(b"L1\nL2\nL3\n", "md");
        let excerpt_name = crate::assets::object_name(b"L2\n", "md");
        assert_eq!(1, report.freeze.unwrap().evidence);
        assert!(
            !root
                .join(format!("ws/assets/{deck_id}/{whole_name}"))
                .exists(),
            "no whole-file object is written"
        );
        assert!(
            root.join(format!("ws/assets/{deck_id}/{excerpt_name}"))
                .is_file()
        );
        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(
            text.contains("exam: ../notes.md"),
            "the live source declaration stays untouched: {text}"
        );
        assert!(
            text.contains("<!-- at: src:notes.md:2 fingerprint: xxh64-")
                && text.contains(&format!(" asset: {excerpt_name} -->\n")),
            "{text}"
        );

        let frozen = Deck::load(&deck_path).unwrap();
        let trace = Trace::from_deck(&frozen).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(vec![(2, "L2".to_string())], ex.lines);
    }

    #[test]
    fn immediate_freezing_freezes_excerpts_from_each_listed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        write(&root.join("src"), "a.rs", "alpha\nbeta\ngamma\n");
        write(&root.join("src"), "b.rs", "one\ntwo\n");
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(&root.join("ws"), "alix.toml", "[defaults]\n");
        let deck_path = write(
            &root.join("ws/decks"),
            "d.md",
            "---\nexam:\n  - ../src/a.rs\n  - ../src/b.rs\ncite:\n  - src: ../src\n---\n\
             ## q1\np\n<!-- at: src:a.rs:2-3 -->\n\
             ## q2\np\n<!-- at: src:b.rs:1 -->\n",
        );
        let report = crate::assets::initialize(&deck_path).unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let deck_id = deck.deck_token.as_deref().unwrap();
        assert_eq!(2, report.freeze.unwrap().evidence);
        assert!(
            root.join(format!(
                "ws/assets/{deck_id}/{}",
                crate::assets::object_name(b"beta\ngamma\n", "rs")
            ))
            .is_file()
        );
        assert!(
            root.join(format!(
                "ws/assets/{deck_id}/{}",
                crate::assets::object_name(b"one\n", "rs")
            ))
            .is_file()
        );
        assert!(
            !root
                .join(format!(
                    "ws/assets/{deck_id}/{}",
                    crate::assets::object_name(b"alpha\nbeta\ngamma\n", "rs")
                ))
                .exists(),
            "no whole-file object is written"
        );
    }

    #[test]
    fn freezing_refuses_non_workspace_and_remote_sources() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let loose = write(
            root,
            "t.md",
            "---\ntrace: t\nexam: .\ncite:\n  - src: .\n---\n## h\np\n<!-- at: src:x.rs:1 -->\n",
        );
        crate::stamp::stamp_deck(&loose).unwrap();
        let err = crate::assets::freeze_member(&loose).unwrap_err();
        assert!(format!("{err:#}").contains("not a member"), "{err:#}");

        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(&root.join("ws"), "alix.toml", "[defaults]\n");
        let url = write(
            &root.join("ws/decks"),
            "u.md",
            "---\ntrace: t\nexam: https://example.com/p\ncite:\n  - src: https://example.com/p\n---\n## h\np\n<!-- at: src -->\n",
        );
        let err = crate::assets::initialize(&url).unwrap_err();
        assert!(
            format!("{err:#}").contains("no `asset:`"),
            "a passage under a url root is never fetched, so an unfrozen one refuses: {err:#}"
        );
    }
}
