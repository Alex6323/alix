//! `alix explore` — goal-driven exploration of a source (first slice).
//!
//! Where [`crate::trace::suggest`] is the flat recon menu of candidate *traces*,
//! `explore` is goal-driven exploration: given a source and a learning **goal**,
//! it manufactures the ordered set of **means** — fact *decks* and *traces* —
//! that, worked through, would reach the goal. The means are chosen by the shape
//! of the knowledge (edges → traces, nodes → decks), sized to the goal by
//! saturation, and ordered by prerequisite. It **prints the plan**, and with
//! `--into <dir>` also **materializes** it into a workspace folder — a
//! `alix.toml` plus one stub deck/trace file per item, wired by `% requires:`.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

use crate::{
    ask,
    config::{AskConfig, TraceConfig},
    deck::{Deck, is_url},
    title,
    trace::{self, build_run_config, clean_to_cards, resolve_source},
    workspace,
};

/// Explore a source toward `goal` and return an ordered learning plan — the
/// decks and traces worth authoring, each tagged and dependency-ordered. One
/// read-only exploration pass (the same tools and cwd as [`crate::trace::build`]);
/// discovers nothing in depth and writes nothing. `source` is a scope directly
/// (a repo `.`, a directory, a file, or a URL), not a deck.
pub fn explore(source: &str, goal: &str, cfg: &TraceConfig, ask_cfg: &AskConfig) -> Result<String> {
    let url = is_url(source);
    let cwd = if url {
        None
    } else {
        let (base_dir, _) = resolve_source(None, Some(source));
        Some(base_dir)
    };
    let prompt = explore_prompt(source, goal, url, cfg);
    let run_cfg = build_run_config(cfg, ask_cfg, cwd, url);
    let raw = ask::run(&run_cfg, &prompt, &[])?;
    let plan = raw.trim().to_string();
    if plan.is_empty() {
        bail!("the exploration produced no plan");
    }
    Ok(plan)
}

/// Builds the exploration prompt: explore the source and emit an ordered,
/// prerequisite-sorted plan of means (decks + traces) sized to the goal by
/// saturation. The counterpart to [`crate::trace::suggest`]'s recon prompt, one
/// tier up.
fn explore_prompt(source: &str, goal: &str, url: bool, cfg: &TraceConfig) -> String {
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
        "the whole source `.`, or a narrower path (a subdirectory or single file) \
         for a tightly-scoped item"
    };
    let mut p = format!(
        "You are EXPLORING a source for a learner whose GOAL is:\n\n    {goal}\n\n\
         Produce the ordered SET OF MEANS — facts decks and traces — that, \
         worked through in order, would achieve that goal. Do NOT build any deck or \
         trace in depth (no cards, no checkpoints) — that is a separate step the \
         learner runs later on each item. Output a PLAN: an ordered list of \
         items.\n\n\
         Source (the scope): {source}\n{explore}\n\n\
         TWO KINDS OF MEANS, chosen by the SHAPE of the knowledge:\n\
         - a TRACE drills an EDGE — a path predicted hop by hop, \"how X becomes \
         Y\": a data flow, a control flow, or a derivation, a real sequence with two \
         ends. Use a trace where the knowledge IS a path.\n\
         - a DECK drills NODES — a table of related facts with no path to predict (a \
         config's knobs, an on-disk format, a glossary of terms). Use a deck where \
         the knowledge is a SET OF FACTS.\n\
         Pick the form that fits each part; never force facts into a fake path, nor \
         a real path into loose facts.\n\n\
         COVERAGE is sized to the GOAL, not to a number. Identify the parts of the \
         source the goal requires understanding, and emit one item per part, \
         choosing trace or deck by shape. STOP at SATURATION: when one more item \
         would teach no new mechanism or concept the learner has not already met. A \
         broad goal (\"understand the whole X\") covers every major subsystem; a \
         narrow goal (\"how Y works\") covers only Y's parts. Do not pad to look \
         thorough; do not drop a part the goal needs.\n\n\
         ORDER by PREREQUISITE. Sort so that whatever must be understood FIRST comes \
         first — the foundational data model and parsing before the flow that uses \
         them before the outer surfaces. Give each item a `requires:` list naming \
         the earlier item numbers it builds on (none for the foundations). The order \
         must be a valid TOPOLOGICAL order: every item's requirements appear above \
         it.\n\n\
         FORMAT — output ONLY the plan, no preamble, no code fences. Start with \
         three heading lines, then the numbered items:\n\n\
         Goal    {goal}\n\
         Source  <one line: what this source is>\n\
         Spine   <the single most central path, arrow-joined nouns>\n\n\
         1. [deck]  <a short topic noun phrase, e.g. the deck format>\n   \
         requires: none\n   % source: <{scope}>\n\
         2. [trace] <the path-question, e.g. how deck text becomes a list of Cards>\n   \
         requires: 1\n   % source: <{scope}>\n\
         3. …\n\n\
         Tag EVERY item [trace] or [deck]. Keep each title SHORT — one line, a \
         handful of words: a [deck] is a noun phrase naming the topic (`the crate \
         surface`, `error taxonomy`), a [trace] is a terse path-question (`how a \
         request becomes a profile`). Do NOT pack the contents into the title — no \
         `X: a, b, and c` enumerations, no parenthetical lists; the cards hold the \
         detail. Do not resolve line numbers or write cards/checkpoints — later \
         steps do that. Use as many items as the goal needs (stop at saturation), \
         ordered by prerequisite.\n\n\
         `% source:` FORMAT: a [deck]'s source must be actual FILE path(s) (the \
         exam reads them) — never a bare directory. For several files, join them \
         with ` + `, writing the FIRST as a full path and the rest RELATIVE to its \
         directory, e.g. `% source: <root>/README.md + src/lib.rs`. A [trace] may \
         use a single directory or file as its locator base."
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

/// Generate the explore walk: a short predict-verify trace over a source's
/// SHAPE — what it is → its parts → its entry → its spine → what to trace first —
/// each hop citing real structural evidence. Returns the checkpoint cards
/// (header-less); the caller wraps them in a `% trace:`/`% source:` deck. Reuses
/// the same read-only exploration as [`explore`].
pub fn walk(source: &str, goal: &str, cfg: &TraceConfig, ask_cfg: &AskConfig) -> Result<String> {
    let url = is_url(source);
    let cwd = if url {
        None
    } else {
        let (base_dir, _) = resolve_source(None, Some(source));
        Some(base_dir)
    };
    let prompt = walk_prompt(source, goal, url, cfg);
    let run_cfg = build_run_config(cfg, ask_cfg, cwd, url);
    let raw = ask::run(&run_cfg, &prompt, &[])?;
    let cards = clean_to_cards(&raw);
    if cards.trim().is_empty() {
        bail!("the explore walk produced no checkpoints");
    }
    Ok(cards)
}

/// Builds the explore-walk prompt: produce trace checkpoints about the
/// source's *shape* (manifest → nouns → entry → spine → what to trace), each
/// citing real structural evidence, in the standard trace checkpoint format.
fn walk_prompt(source: &str, goal: &str, url: bool, cfg: &TraceConfig) -> String {
    let explore = if url {
        format!("Read the source page at {source} with the WebFetch tool (fetch it once).")
    } else {
        "Your working directory is the source root. Explore it with the Read, Glob \
         and Grep tools — read the manifest, the module/file names, the entry point, \
         and the most central file. You have no write or shell access."
            .to_string()
    };
    let locator = if url {
        "a short quoted span from the page — the exact words the answer rests on"
    } else {
        "ONE contiguous range, `file:start-end`, relative to the source root (e.g. \
         `Cargo.toml:8-20` or `src/lib.rs:12-33`) — never comma-separated"
    };
    let mut p = format!(
        "You are building an EXPLORE walk: a short predict-and-verify trace that \
         teaches a newcomer the SHAPE of a source by making them PREDICT its \
         structure before each reveal. The learner's aim:\n\n    {goal}\n\n{explore}\n\n\
         Walk it the way you read a codebase cold, one hop per step:\n\
         1. from the manifest / dependencies — what KIND of thing is this?\n\
         2. from the module / file names — what are its core DOMAIN NOUNS (the model)?\n\
         3. from the entry point — how is it DRIVEN (its commands / surfaces)?\n\
         4. from the most central file — what is the SPINE (the main path data takes)?\n\
         5. (last) given that shape, what are the first PATHS worth tracing next? — \
         name 2-4 concrete candidate traces (the menu).\n\n\
         Each hop must CITE REAL STRUCTURAL EVIDENCE as its `% at:` locator — the \
         actual lines the answer rests on (the manifest's dependency list, the \
         module-declaration lines, the entry enum, the central function's signature). \
         The reveal is the real text; the source is the oracle, never invented. Every \
         hop has a locator.\n\n\
         FORMAT — output ONLY the checkpoint cards: no header, no `% trace:`/`% source:`, \
         no preamble, no code fences. Each checkpoint (key-point and directive lines \
         are indented with a TAB):\n\n    \
         # <the shape question, asked plainly>\n\t<a key point a correct answer \
         hits>\n\t<another key point>\n\t% at: <locator>\n\t! <one connecting \
         insight>\n\n\
         where `% at:` is {locator}.\n\n\
         RULES: each question reasons FORWARD from the previous reveal (hop 1 has no \
         prior); carry the STATE, never \"checkpoint N\"; ask plainly — do NOT prefix \
         with \"Predict\"; keep the answer out of the question; every key point must be \
         GROUNDED in the cited lines; the LAST hop lands on the candidate traces (the \
         menu of what to trace first). Keep each question one or two sentences and each \
         key point one line. Use 4-6 checkpoints."
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

/// Explore a source once (one CLI session), then RESUME that session to fill
/// every plan item with real content using the understanding just built —
/// checkpoints for each `[trace]`, fact cards for each `[deck]`. Returns the plan
/// and a map from item number to its filled body. Exploring once (not per item)
/// keeps the items coherent (each aware of the others) and amortizes the read.
pub fn explore_and_fill(
    source: &str,
    goal: &str,
    cfg: &TraceConfig,
    ask_cfg: &AskConfig,
) -> Result<(String, HashMap<usize, String>)> {
    let url = is_url(source);
    let cwd = if url {
        None
    } else {
        let (base_dir, _) = resolve_source(None, Some(source));
        Some(base_dir)
    };
    let run_cfg = build_run_config(cfg, ask_cfg, cwd, url);
    let mut session = ask::CliSession::new();

    // 1. Explore — establishes the session and its understanding of the source.
    let plan = ask::run(
        &run_cfg,
        &explore_prompt(source, goal, url, cfg),
        &session.args(),
    )?
    .trim()
    .to_string();
    session.started = true;
    if plan.is_empty() {
        bail!("the exploration produced no plan");
    }
    let items = parse_plan(&plan);
    if items.is_empty() {
        bail!("the plan has no items to fill");
    }

    // 2. Resume — fill every item from what the session already learned.
    let filled = ask::run(&run_cfg, &fill_prompt(&items), &session.args())?;
    Ok((plan, parse_filled(&filled)))
}

/// Builds the fill prompt for the RESUMED explore session: write the full content
/// for every plan item, keyed by `=== item N ===` delimiters, reusing the
/// understanding from the exploration just done.
fn fill_prompt(items: &[Item]) -> String {
    let mut list = String::new();
    for item in items {
        let kind = match item.kind {
            Kind::Trace => "trace",
            Kind::Deck => "deck",
        };
        list.push_str(&format!("{}. [{}] {}\n", item.num, kind, item.title));
    }
    format!(
        "You just explored this source and produced the plan below. Now WRITE THE \
         FULL CONTENT for EVERY item, reusing what you already learned — only read \
         more if you must verify a line number. Because you are writing the whole \
         set at once, make it COHERENT: an item must NOT re-teach what an earlier \
         item (its prerequisite) covers — build on it, keep terminology consistent, \
         and don't overlap.\n\n\
         For each item, emit a delimiter line exactly `=== item <N> ===` (its plan \
         number) followed by its content:\n\
         - a [trace] → the predict-verify CHECKPOINT cards: each is a `# ` question \
         at column 0, then TAB-indented key points, a `% at: file:start-end` locator \
         citing the REAL lines, and an optional `! ` note. Each hop opens on the \
         previous reveal, predicts forward, and its key points are grounded in the \
         cited lines.\n\
         - a [deck] → FACT cards: each is a `# ` front at column 0, then TAB-indented \
         back line(s), plus a `% at: file:start-end` locator citing the REAL lines \
         whenever the fact maps to a specific range (so the card can show its source \
         on reveal; omit it when the fact synthesizes across several places). One \
         fact per card, concise and recall-oriented.\n\n\
         Do NOT repeat any header directive (`% trace:`, `% title:`, `% source:`, \
         `% requires:`) — those are already written; output only the `# ` cards. \
         Output ONLY the delimited item bodies: no preamble, no code fences, nothing \
         between the last card of one item and the next `=== item ===` line.\n\n\
         The plan:\n{list}"
    )
}

/// Splits the fill response into per-item bodies on `=== item N ===` delimiters.
/// Lenient: text before the first delimiter is dropped; an item with no body is
/// omitted (so it stays a stub).
pub(crate) fn parse_filled(raw: &str) -> HashMap<usize, String> {
    let mut out: HashMap<usize, String> = HashMap::new();
    let mut current: Option<usize> = None;
    let mut buf = String::new();
    for line in raw.lines() {
        if let Some(num) = parse_item_delimiter(line) {
            if let Some(n) = current.take() {
                let body = buf.trim();
                if !body.is_empty() {
                    out.insert(n, body.to_string());
                }
            }
            buf.clear();
            current = Some(num);
        } else if current.is_some() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if let Some(n) = current {
        let body = buf.trim();
        if !body.is_empty() {
            out.insert(n, body.to_string());
        }
    }
    out
}

/// Matches a `=== item N ===` delimiter line, returning N.
fn parse_item_delimiter(line: &str) -> Option<usize> {
    line.trim()
        .strip_prefix("=== item")?
        .trim()
        .strip_suffix("===")?
        .trim()
        .parse()
        .ok()
}

/// One item of an exploration plan: a facts deck or a trace to author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    pub num: usize,
    pub kind: Kind,
    pub title: String,
    pub requires: Vec<usize>,
    pub source: String,
}

/// Whether a plan item is a trace (an edge) or a facts deck (nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Trace,
    Deck,
}

/// What [`materialize`] wrote.
pub struct Materialized {
    pub dir: PathBuf,
    pub traces: usize,
    pub decks: usize,
    /// How many items were written with real content (vs. left as stubs).
    pub filled: usize,
}

/// Parses the printed plan back into items (lenient — unrecognized lines, the
/// header, and prose are skipped). An item starts at a `N. [trace|deck] <title>`
/// line and absorbs the following `requires:` and `% source:` lines.
pub(crate) fn parse_plan(plan: &str) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for line in plan.lines() {
        let t = line.trim();
        if let Some((num, kind, title)) = parse_item_header(t) {
            items.push(Item {
                num,
                kind,
                title,
                requires: Vec::new(),
                source: String::new(),
            });
        } else if let Some(item) = items.last_mut() {
            if let Some(rest) = t.strip_prefix("requires:") {
                item.requires = rest
                    .split(|c: char| !c.is_ascii_digit())
                    .filter_map(|p| p.parse().ok())
                    .collect();
            } else if let Some(rest) = t.strip_prefix("% source:") {
                item.source = rest.trim().to_string();
            }
        }
    }
    items
}

/// Matches a `N. [trace|deck] <title>` header line, returning its number, kind
/// and title; `None` for any other line.
fn parse_item_header(t: &str) -> Option<(usize, Kind, String)> {
    let dot = t.find('.')?;
    let num: usize = t[..dot].trim().parse().ok()?;
    let rest = t[dot + 1..].trim_start().strip_prefix('[')?;
    let close = rest.find(']')?;
    let kind = match rest[..close].trim() {
        "trace" => Kind::Trace,
        "deck" => Kind::Deck,
        _ => return None,
    };
    let title = rest[close + 1..].trim().to_string();
    (!title.is_empty()).then_some((num, kind, title))
}

/// Scaffolds the plan into a workspace folder `dir`: an `alix.toml` (the goal +
/// an empty `[defaults]`) and one stub file per item — a `% trace:` deck for a
/// trace, a `% title:` facts deck for a deck — wired by `% requires:` (item
/// numbers mapped to the member file names), with each `% source:` rewritten
/// absolute against the source root. Refuses a non-empty `dir` unless `force`.
#[expect(clippy::too_many_arguments)] // a manifest + a plan + per-call knobs
pub fn materialize(
    plan: &str,
    dir: &Path,
    goal: &str,
    title: Option<&str>,
    unlock_stage: Option<u8>,
    source: &str,
    force: bool,
    filled: Option<&HashMap<usize, String>>,
) -> Result<Materialized> {
    let mut items = parse_plan(plan);
    if items.is_empty() {
        bail!("the plan has no items to materialize");
    }
    // The model ignores the prompt's "keep titles short" guidance, so enforce it
    // here: condense each title before it becomes the header AND the file name.
    for item in &mut items {
        item.title = title::condense(&item.title);
    }
    if dir.exists() {
        let non_empty = fs::read_dir(dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        if non_empty && !force {
            bail!(
                "{} already has files — choose a new directory or pass --force",
                dir.display()
            );
        }
    } else {
        fs::create_dir_all(dir)?;
    }

    let root = if is_url(source) {
        None
    } else {
        Some(fs::canonicalize(source).unwrap_or_else(|_| PathBuf::from(source)))
    };
    let names: Vec<String> = items.iter().map(file_name).collect();
    let by_num: HashMap<usize, &String> = items
        .iter()
        .zip(&names)
        .map(|(it, n)| (it.num, n))
        .collect();

    // `title` names the workspace (omitted → the folder name is used); the goal
    // becomes its one-line `description`.
    let mut manifest = String::from("# Generated by `alix explore`.\n");
    if let Some(title) = title {
        manifest.push_str(&format!("title = \"{}\"\n", toml_escape(title)));
    }
    manifest.push_str(&format!(
        "description = \"{}\"\n\n[defaults]\n",
        toml_escape(goal)
    ));
    // A workspace-wide unlock stage: members gate (exam / unlock dependents) once
    // every card reaches it, without retiring early.
    if let Some(unlock_stage) = unlock_stage {
        manifest.push_str(&format!("unlock-stage = {unlock_stage}\n"));
    }
    // The live source root the frozen `% at:` provenance resolves against — the
    // tutor grounds here for context, and drift detection reads it. Cascades into
    // every member deck (`% origin:`), overridable per deck/card.
    if let Some(root) = &root {
        manifest.push_str(&format!(
            "origin = \"{}\"\n",
            toml_escape(&root.display().to_string())
        ));
    }
    fs::write(dir.join(crate::workspace::MANIFEST), manifest)?;

    let mut traces = 0;
    let mut decks = 0;
    let mut filled_count = 0;
    for (item, name) in items.iter().zip(&names) {
        let mut body = String::new();
        match item.kind {
            Kind::Trace => {
                traces += 1;
                body.push_str(&format!("% trace: {}\n", item.title));
            }
            Kind::Deck => {
                decks += 1;
                body.push_str(&format!("% title: {}\n", item.title));
            }
        }
        body.push_str(&format!(
            "% source: {}\n",
            rewrite_scope(&item.source, root.as_deref())
        ));
        for req in &item.requires {
            if let Some(dep) = by_num.get(req) {
                body.push_str(&format!("% requires: {dep}\n"));
            }
        }
        body.push('\n');
        match filled
            .and_then(|f| f.get(&item.num))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            // `--build` filled this item with real checkpoints / cards.
            Some(content) => {
                filled_count += 1;
                body.push_str(content);
                body.push('\n');
            }
            // A stub: header only, to be filled later.
            None => body.push_str(match item.kind {
                Kind::Trace => {
                    "% Stub from `alix explore`. Discover the path:  alix trace --build <this file>\n"
                }
                Kind::Deck => {
                    "% Stub from `alix explore`. Author cards here, or `alix generate` from the source.\n"
                }
            }),
        }
        fs::write(dir.join(name), body)?;
    }

    Ok(Materialized {
        dir: dir.to_path_buf(),
        traces,
        decks,
        filled: filled_count,
    })
}

/// Outcome of freezing a workspace: how many decks and files were frozen, plus
/// any *cited* decks that froze nothing. A cited deck that can't be frozen is
/// almost always a broken or stale `% source:` — surfaced here so the caller can
/// warn instead of leaving the user with a silently empty `assets/`.
#[derive(Debug, Default)]
pub struct SnapshotSummary {
    pub decks: usize,
    pub files: usize,
    /// `"<member>: <reason>"` for each cited deck whose excerpts couldn't be read.
    pub failed: Vec<String>,
}

/// Freezes each cited deck's source excerpts into the workspace's `assets/` — the
/// default final step of `alix explore --into --build`, so the workspace is
/// self-contained and its line-number locators never drift. A cited deck is a
/// trace (its checkpoints) or a fact deck whose cards carry `% at:`. A deck with
/// no citations (a plain fact deck, a URL source) is skipped; a deck that DOES
/// cite local excerpts but whose source can't be read is recorded in
/// [`SnapshotSummary::failed`], never silently dropped.
pub fn snapshot_workspace(dir: &Path) -> Result<SnapshotSummary> {
    let mut summary = SnapshotSummary::default();
    let ws = workspace::Workspace::load(dir)?;
    // The workspace-wide origin (`alix.toml [defaults] origin`): a deck whose
    // source root matches it inherits it; one that diverges writes its own.
    let workspace_origin = ws.settings.origin.as_deref().map(PathBuf::from);
    for member in ws.members {
        let Ok(deck) = Deck::load(&member) else {
            continue;
        };
        if !(deck.is_trace() || deck.cards.iter().any(|c| c.at.is_some())) {
            continue;
        }
        // `summary.files` is the running snippet count, passed as the start so each
        // deck's snippets get unique names in the shared `assets/`.
        match trace::snapshot(&deck, summary.files, workspace_origin.as_deref()) {
            Ok(report) => {
                summary.decks += 1;
                summary.files += report.copied.len();
                for missing in &report.missing {
                    eprintln!(
                        "warning: {}: cited file not found, not frozen: {missing}",
                        member.display()
                    );
                }
            }
            // The deck cites local excerpts but none could be frozen — almost
            // always a broken/stale `% source:`. Record it; don't swallow it.
            Err(e) => summary.failed.push(format!("{}: {e:#}", member.display())),
        }
    }
    Ok(summary)
}

/// The member file name for an item: `NN-<slug>.txt`, the zero-padded number
/// (preserving plan order) plus a slug of the title.
fn file_name(item: &Item) -> String {
    format!("{:02}-{}.txt", item.num, slug(&item.title))
}

/// A short kebab slug from a title: up to the first six alphanumeric words.
fn slug(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(6)
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        "item".to_string()
    } else {
        words.join("-")
    }
}

/// Rewrites a plan `% source:` scope to point at the real source: absolute under
/// the source root for a local path (`.` → the root itself), left as-is for a
/// URL or when there is no local root.
///
/// The model sometimes writes the scope relative to the PROJECT ROOT *above* the
/// `--source` (e.g. `crates/x/src/lib.rs` while `root` already IS `…/crates/x`).
/// A plain `root.join(scope)` would double the overlap into a path that doesn't
/// exist (`…/crates/x/crates/x/src/lib.rs`) — which silently broke both freezing
/// and citation reads. So the first ` + ` part is anchored overlap-aware via
/// [`trace::resolve_under_base`] (the write-time twin of how `% at:` locators
/// resolve on read), and any further ` + ` parts stay relative to it.
fn rewrite_scope(scope: &str, root: Option<&Path>) -> String {
    let scope = scope.trim();
    let Some(root) = root else {
        return scope.to_string();
    };
    if is_url(scope) {
        return scope.to_string();
    }
    let (first, rest) = match scope.split_once(" + ") {
        Some((a, b)) => (a.trim(), Some(b.trim())),
        None => (scope, None),
    };
    let anchored = if first == "." {
        root.to_path_buf()
    } else if Path::new(first).is_absolute() {
        PathBuf::from(first)
    } else {
        trace::resolve_under_base(root, first)
    };
    match rest {
        Some(rest) => format!("{} + {}", anchored.display(), rest),
        None => anchored.display().to_string(),
    }
}

/// Escapes a string for a double-quoted TOML value.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explore_prompt_carries_goal_means_coverage_and_order() {
        let p = explore_prompt(
            ".",
            "understand the alix repo",
            false,
            &TraceConfig::default(),
        );
        assert!(p.contains("understand the alix repo")); // the goal is echoed
        assert!(p.contains("SET OF MEANS")); // decks + traces, not just traces
        assert!(p.contains("[trace]")); // both kinds tagged
        assert!(p.contains("[deck]"));
        assert!(p.contains("SHAPE")); // chosen by edge-vs-node shape
        assert!(p.contains("SATURATION")); // stop rule, not a count
        assert!(p.contains("requires:")); // prerequisite edges
        assert!(p.contains("TOPOLOGICAL order")); // dependency-ordered
        assert!(p.contains("no cards, no checkpoints")); // a plan, not built items
        assert!(p.contains("Read, Glob")); // read-only local exploration
        assert!(!p.contains("WebFetch")); // local source needs no web tool
    }

    #[test]
    fn explore_prompt_url_uses_webfetch() {
        let p = explore_prompt(
            "https://x",
            "understand the page",
            true,
            &TraceConfig::default(),
        );
        assert!(p.contains("WebFetch"));
        assert!(!p.contains("Glob")); // no local file tools for a URL source
    }

    #[test]
    fn fill_prompt_asks_fact_cards_to_cite_their_source() {
        let items = vec![
            Item {
                num: 1,
                kind: Kind::Deck,
                title: "Basics".to_string(),
                requires: Vec::new(),
                source: ".".to_string(),
            },
            Item {
                num: 2,
                kind: Kind::Trace,
                title: "The path".to_string(),
                requires: vec![1],
                source: ".".to_string(),
            },
        ];
        let p = fill_prompt(&items);
        assert!(p.contains("FACT cards"));
        // Fact cards get a `% at:` citation so the card can show its source.
        assert!(p.contains("% at: file:start-end"));
        assert!(p.contains("show its source on reveal"));
    }

    #[test]
    fn walk_prompt_predicts_shape_with_evidence() {
        let p = walk_prompt(".", "understand the repo", false, &TraceConfig::default());
        assert!(p.contains("EXPLORE walk")); // a predict-verify walk, not a plan dump
        assert!(p.contains("PREDICT its")); // the learner predicts the shape
        assert!(p.contains("DOMAIN NOUNS")); // nouns hop
        assert!(p.contains("SPINE")); // spine hop
        assert!(p.contains("CITE REAL STRUCTURAL EVIDENCE")); // grounded reveals
        assert!(p.contains("candidate traces")); // last hop lands on the menu
        assert!(p.contains("% at:")); // standard trace locator format
        assert!(p.contains("Read, Glob")); // read-only local exploration
        assert!(!p.contains("WebFetch"));
    }

    const SAMPLE_PLAN: &str = "\
Goal    understand X
Source  a thing
Spine   a -> b

1. [deck]  The deck format: markers and directives
   requires: none
   % source: README.md
2. [trace] How text becomes Cards
   requires: 1
   % source: src/parser.rs
10. [trace] How a request is served
    requires: 1, 2
    % source: src/serve.rs
";

    #[test]
    fn parse_plan_extracts_items_kinds_requires_and_source() {
        let items = parse_plan(SAMPLE_PLAN);
        assert_eq!(3, items.len()); // header lines are skipped
        assert_eq!(Kind::Deck, items[0].kind);
        assert_eq!(1, items[0].num);
        assert!(items[0].title.starts_with("The deck format"));
        assert!(items[0].requires.is_empty()); // "none" → no deps
        assert_eq!("README.md", items[0].source);
        assert_eq!(Kind::Trace, items[1].kind);
        assert_eq!(vec![1], items[1].requires);
        assert_eq!(10, items[2].num); // two-digit number parsed
        assert_eq!(vec![1, 2], items[2].requires);
    }

    #[test]
    fn materialize_writes_manifest_and_wired_stubs() {
        let dir = std::env::temp_dir().join(format!("alix-explore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let report = materialize(
            SAMPLE_PLAN,
            &dir,
            "understand the repo",
            None,
            None,
            ".",
            false,
            None,
        )
        .unwrap();
        assert_eq!(2, report.traces);
        assert_eq!(1, report.decks);
        assert_eq!(0, report.filled); // no fill map → all stubs

        let manifest = fs::read_to_string(dir.join("alix.toml")).unwrap();
        // The goal becomes the description; with no --title, the folder names it.
        assert!(manifest.contains("description = \"understand the repo\""));
        assert!(!manifest.contains("title ="));
        assert!(manifest.contains("[defaults]"));

        let deck = fs::read_to_string(dir.join("01-the-deck-format.txt")).unwrap();
        assert!(deck.contains("% title: The Deck Format")); // condensed + title-cased
        assert!(deck.contains("% source: ")); // rewritten absolute

        let trace = fs::read_to_string(dir.join("02-how-text-becomes-cards.txt")).unwrap();
        assert!(trace.contains("% trace: How Text Becomes Cards"));
        // requires 1 → the first item's file name
        assert!(trace.contains("% requires: 01-the-deck-format.txt"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_refuses_a_non_empty_dir_without_force() {
        let dir = std::env::temp_dir().join(format!("alix-explore-ne-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("keep.txt"), "existing").unwrap();

        assert!(materialize(SAMPLE_PLAN, &dir, "g", None, None, ".", false, None).is_err());
        assert!(materialize(SAMPLE_PLAN, &dir, "g", None, None, ".", true, None).is_ok()); // --force writes anyway

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_writes_title_and_unlock_stage() {
        let dir = std::env::temp_dir().join(format!("alix-explore-title-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        materialize(
            SAMPLE_PLAN,
            &dir,
            "the goal",
            Some("Repo Internals"),
            Some(2),
            ".",
            false,
            None,
        )
        .unwrap();
        let manifest = fs::read_to_string(dir.join("alix.toml")).unwrap();
        assert!(manifest.contains("title = \"Repo Internals\""));
        assert!(manifest.contains("description = \"the goal\""));
        // The unlock-stage lands in [defaults], shared by every member deck.
        assert!(manifest.contains("[defaults]\nunlock-stage = 2"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_filled_splits_on_item_delimiters() {
        let raw = "\
preamble ignored
=== item 1 ===
# front a
\tback a
=== item 2 ===
# question
\tkey point
\t% at: src/x.rs:1-3
=== item 3 ===
";
        let filled = parse_filled(raw);
        assert_eq!(2, filled.len()); // item 3 has no body → omitted
        assert!(filled[&1].contains("# front a"));
        assert!(filled[&2].contains("% at: src/x.rs:1-3"));
        assert!(!filled.contains_key(&3));
    }

    #[test]
    fn materialize_writes_filled_content_when_given() {
        let dir = std::env::temp_dir().join(format!("alix-explore-fill-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut filled = HashMap::new();
        filled.insert(
            2usize,
            "# how does text become Cards?\n\tparse_str runs a state machine\n\t% at: src/parser.rs:1-9"
                .to_string(),
        );

        let report = materialize(
            SAMPLE_PLAN,
            &dir,
            "g",
            None,
            None,
            ".",
            false,
            Some(&filled),
        )
        .unwrap();
        assert_eq!(1, report.filled); // only item 2 was filled

        // item 2 (a trace) keeps its header AND carries the filled checkpoint
        let trace = fs::read_to_string(dir.join("02-how-text-becomes-cards.txt")).unwrap();
        assert!(trace.contains("% trace: How Text Becomes Cards"));
        assert!(trace.contains("# how does text become Cards?"));
        assert!(trace.contains("% at: src/parser.rs:1-9"));
        // item 1 had no fill → still a stub
        let deck = fs::read_to_string(dir.join("01-the-deck-format.txt")).unwrap();
        assert!(deck.contains("% Stub from `alix explore`"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_workspace_freezes_traces_and_cited_facts() {
        let dir = std::env::temp_dir().join(format!("alix-snap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/a.rs"), "x\ny\nz\n").unwrap();
        fs::write(dir.join("alix.toml"), "[defaults]\n").unwrap();
        let src = dir.join("src");
        // a built trace citing src/a.rs (source = the dir)
        fs::write(
            dir.join("01-t.txt"),
            format!(
                "% trace: t\n% source: {}\n# h\n\tp\n\t% at: a.rs:1-2\n",
                src.display()
            ),
        )
        .unwrap();
        // a fact deck citing the same source with `% at:` — also frozen
        fs::write(
            dir.join("02-d.txt"),
            format!("% source: {}\n# q\n\ta\n\t% at: a.rs:3\n", src.display()),
        )
        .unwrap();
        // a fact deck with no citations — skipped
        fs::write(dir.join("03-plain.txt"), "% title: d\n# q\n\ta\n").unwrap();

        let summary = snapshot_workspace(&dir).unwrap();
        assert_eq!((2, 2), (summary.decks, summary.files)); // trace + cited fact, one snippet each
        assert!(summary.failed.is_empty(), "{:?}", summary.failed);
        // Two unique snippet names in the shared assets/ (the offset prevents a
        // collision), and the whole file is never copied.
        assert!(dir.join("assets/01.rs").is_file());
        assert!(dir.join("assets/02.rs").is_file());
        assert!(!dir.join("assets/a.rs").exists());
        // the fact deck was repointed off its live locator onto a frozen snippet
        let fact = fs::read_to_string(dir.join("02-d.txt")).unwrap();
        assert!(fact.contains("% source: assets\n"), "{fact}");
        assert!(!fact.contains("% at: a.rs:3"), "{fact}");
        assert!(fact.contains("% at: 0"), "{fact}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_workspace_surfaces_a_deck_whose_source_cannot_be_frozen() {
        // A cited deck whose `% source:` points nowhere must be REPORTED, not
        // silently swallowed (the bug that left a freshly built workspace with no
        // `assets/` and no warning).
        let dir = std::env::temp_dir().join(format!("alix-snap-fail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("alix.toml"), "[defaults]\n").unwrap();
        fs::write(
            dir.join("01-broken.txt"),
            format!(
                "% source: {}/does-not-exist\n# q\n\ta\n\t% at: src/x.rs:1\n",
                dir.display()
            ),
        )
        .unwrap();

        let summary = snapshot_workspace(&dir).unwrap();
        assert_eq!(0, summary.decks); // nothing frozen
        assert_eq!(1, summary.failed.len(), "{:?}", summary.failed);
        assert!(
            summary.failed[0].contains("01-broken.txt"),
            "{:?}",
            summary.failed
        );
        assert!(!dir.join("assets").exists()); // no empty assets dir left behind

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rewrite_scope_anchors_a_repo_relative_scope_without_doubling() {
        // The `--source` is a subdir (a crate), but the model writes its source
        // scope relative to the project root *above* it. The written `% source:`
        // must be the real file, never `…/mycrate/crates/mycrate/…` (the doubling
        // that silently broke freezing and citation reads).
        let root = std::env::temp_dir().join(format!("alix-scope-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let crate_src = root.join("crates/mycrate/src");
        fs::create_dir_all(&crate_src).unwrap();
        fs::write(crate_src.join("lib.rs"), "fn main() {}\n").unwrap();
        let lib = crate_src.join("lib.rs").display().to_string();
        let source = root.join("crates/mycrate"); // the `--source`

        // repo-root-relative scope → resolved overlap-aware to the real file
        assert_eq!(
            lib,
            rewrite_scope("crates/mycrate/src/lib.rs", Some(&source))
        );
        // a clean crate-relative scope still resolves directly
        assert_eq!(lib, rewrite_scope("src/lib.rs", Some(&source)));
        // `.` → the source root itself; an absolute path is left untouched
        assert_eq!(
            source.display().to_string(),
            rewrite_scope(".", Some(&source))
        );
        assert_eq!(lib, rewrite_scope(&lib, Some(&source)));
        // multi-source: only the first ` + ` part is re-anchored, the rest stays
        assert_eq!(
            format!("{lib} + other.rs"),
            rewrite_scope("crates/mycrate/src/lib.rs + other.rs", Some(&source))
        );

        let _ = fs::remove_dir_all(&root);
    }
}
