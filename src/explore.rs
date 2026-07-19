use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    ask,
    backend::ensure_source_reachable,
    config::{AskConfig, TraceConfig},
    deck::{Deck, is_url},
    library,
    parser::yaml_quote,
    share,
    store::Store,
    title,
    trace::{self, resolve_source},
    trace_ai::{self, build_run_config, clean_to_cards},
    workspace,
};

/// `source` is a scope directly (a directory, file, or URL), not a deck.
pub fn explore(source: &str, goal: &str, cfg: &TraceConfig, ask_cfg: &AskConfig) -> Result<String> {
    let url = is_url(source);
    ensure_source_reachable(ask_cfg, url)?;
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
         requires: none\n   @source: <{scope}>\n\
         2. [trace] <the path-question, e.g. how deck text becomes a list of Cards>\n   \
         requires: 1\n   @source: <{scope}>\n\
         3. …\n\n\
         Tag EVERY item [trace] or [deck]. Keep each title SHORT — one line, a \
         handful of words: a [deck] is a noun phrase naming the topic (`the crate \
         surface`, `error taxonomy`), a [trace] is a terse path-question (`how a \
         request becomes a profile`). Do NOT pack the contents into the title — no \
         `X: a, b, and c` enumerations, no parenthetical lists; the cards hold the \
         detail. Do not resolve line numbers or write cards/checkpoints — later \
         steps do that. Use as many items as the goal needs (stop at saturation), \
         ordered by prerequisite.\n\n\
         `@source:` FORMAT: a [deck]'s source must be actual FILE path(s) (the \
         exam reads them) — never a bare directory. For several files, join them \
         with ` + `, writing the FIRST as a full path and the rest RELATIVE to its \
         directory, e.g. `@source: <root>/README.md + src/lib.rs`. A [trace] may \
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

pub fn walk(source: &str, goal: &str, cfg: &TraceConfig, ask_cfg: &AskConfig) -> Result<String> {
    let url = is_url(source);
    ensure_source_reachable(ask_cfg, url)?;
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
         Each hop must CITE REAL STRUCTURAL EVIDENCE as its `at:` locator — the \
         actual lines the answer rests on (the manifest's dependency list, the \
         module-declaration lines, the entry enum, the central function's signature). \
         The reveal is the real text; the source is the oracle, never invented. Every \
         hop has a locator.\n\n\
         FORMAT — output ONLY the checkpoint cards: no frontmatter, no `trace:`/\
         `source:` key, no preamble, no code fences. Each checkpoint:\n\n\
         ## <the shape question, asked plainly>\n\
         <a key point a correct answer hits>\n\
         <another key point>\n\
         <!-- at: <locator> -->\n\
         > <one connecting insight>\n\n\
         The `## ` front is at column 0, never indented; the key points are plain \
         unindented lines below it; where the `at:` locator is {locator}.\n\n\
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

/// Resumes one session rather than exploring per item, so items stay
/// coherent and the read is amortized.
// Untested: two real AI calls plus a merge step; its `url == true` branch is
// dead (the one CLI call site is dir-gated).
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn explore_and_fill(
    source: &str,
    goal: &str,
    cfg: &TraceConfig,
    ask_cfg: &AskConfig,
) -> Result<(String, HashMap<usize, String>)> {
    let url = is_url(source);
    ensure_source_reachable(ask_cfg, url)?;
    let cwd = if url {
        None
    } else {
        let (base_dir, _) = resolve_source(None, Some(source));
        Some(base_dir)
    };
    let run_cfg = build_run_config(cfg, ask_cfg, cwd, url);
    let mut session = ask::CliSession::new();

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

    let filled = ask::run(&run_cfg, &fill_prompt(&items), &session.args())?;
    Ok((plan, parse_filled(&filled)))
}

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
         - a [trace] → the predict-verify CHECKPOINT cards: each is a `## ` question \
         at column 0 (never indented), then its key points as plain unindented \
         lines, an `<!-- at: file:start-end -->` locator line citing the REAL \
         lines, and an optional `> ` note. Each hop opens on the previous reveal, \
         predicts forward, and its key points are grounded in the cited lines.\n\
         - a [deck] → FACT cards: each is a `## ` front at column 0, then its back \
         line(s) as plain unindented lines, plus an `<!-- at: file:start-end -->` \
         locator line citing the REAL lines whenever the fact maps to a specific \
         range (so the card can show its source on reveal; omit it when the fact \
         synthesizes across several places). One fact per card, concise and \
         recall-oriented. Do NOT cram an enumeration into one prose answer: if the \
         answer is a list of several items, split it into several one-idea cards \
         (one card per item or group), or give it clean structure with one point \
         per line (no bullet or dash prefix — bullets come later from the format \
         augment); keep an atomic answer atomic.\n\n\
         Every `at:` `file` part MUST be written relative to the SAME root — the \
         source root you explored (your working directory) — as ONE consistent path \
         per file across ALL items; never drop or add a leading directory (always \
         `src/foo.rs`, never sometimes `foo.rs`), so the frozen citations all resolve.\n\n\
         Do NOT emit any frontmatter (`trace:`, `source:`, `requires:` keys) or a \
         `# ` title — those are already written; output only the `## ` cards. \
         Output ONLY the delimited item bodies: no preamble, no code fences, nothing \
         between the last card of one item and the next `=== item ===` line.\n\n\
         The plan:\n{list}"
    )
}

/// Lenient: text before the first delimiter is dropped; an item with no
/// body is omitted (stays a stub).
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

fn parse_item_delimiter(line: &str) -> Option<usize> {
    line.trim()
        .strip_prefix("=== item")?
        .trim()
        .strip_suffix("===")?
        .trim()
        .parse()
        .ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub num: usize,
    pub kind: Kind,
    pub title: String,
    pub requires: Vec<usize>,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Trace,
    Deck,
}

pub struct Materialized {
    pub dir: PathBuf,
    pub traces: usize,
    pub decks: usize,
    pub filled: usize,
}

pub fn parse_plan(plan: &str) -> Vec<Item> {
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
            } else if let Some(rest) = t.strip_prefix("@source:") {
                item.source = rest.trim().to_string();
            }
        }
    }
    items
}

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

/// A populated `dir` is deliberately not refused: callers that write straight
/// into a real destination decide that.
pub fn materialize(
    plan: &str,
    dir: &Path,
    goal: &str,
    title: Option<&str>,
    source: &str,
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
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;

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
    let mut manifest = String::from("# Generated by `alix generate`.\n");
    if let Some(title) = title {
        manifest.push_str(&format!("title = \"{}\"\n", toml_escape(title)));
    }
    manifest.push_str(&format!(
        "description = \"{}\"\n\n[defaults]\n",
        toml_escape(goal)
    ));
    // The source root the tutor grounds against and drift detection reads;
    // cascades into each member deck's `origin`, overridable per deck/card.
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
        let mut body = String::from("---\n");
        match item.kind {
            Kind::Trace => {
                traces += 1;
                body.push_str(&format!("trace: {}\n", yaml_quote(&item.title)));
            }
            Kind::Deck => decks += 1,
        }
        body.push_str(&format!(
            "source: {}\n",
            yaml_quote(&rewrite_scope(&item.source, root.as_deref()))
        ));
        let deps: Vec<&&String> = item
            .requires
            .iter()
            .filter_map(|req| by_num.get(req))
            .collect();
        if !deps.is_empty() {
            body.push_str("requires:\n");
            for dep in deps {
                body.push_str(&format!("  - {}\n", yaml_quote(dep)));
            }
        }
        body.push_str("\n---\n");
        if item.kind == Kind::Deck {
            body.push_str(&format!("# {}\n", item.title));
        }
        body.push('\n');
        match filled
            .and_then(|f| f.get(&item.num))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            Some(content) => {
                filled_count += 1;
                body.push_str(content);
                body.push('\n');
            }
            // A stub: header only; the parser ignores the preamble prose.
            None => body.push_str(match item.kind {
                Kind::Trace => {
                    "Stub from `alix generate`. Build the path:  alix generate <this file>\n"
                }
                Kind::Deck => {
                    "Stub from `alix generate`. Author cards here, or `alix generate` from the source.\n"
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

/// A conflict keeps the user's original; the new file stays in `staging`.
pub struct MergeReport {
    pub moved: usize,
    pub conflicts: Vec<String>,
}

/// A forced `.md` collision routes through [`library::replace_deck`] so the
/// old member's progress is wiped, not orphaned.
pub fn merge_built(
    staging: &Path,
    dest: &Path,
    force: bool,
    store: &mut Store,
) -> Result<MergeReport> {
    fs::create_dir_all(dest).with_context(|| format!("cannot create {}", dest.display()))?;
    let mut moved = 0;
    let mut conflicts = Vec::new();
    for entry in
        fs::read_dir(staging).with_context(|| format!("cannot read {}", staging.display()))?
    {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let from = entry.path();
        let to = dest.join(&name);
        if to.exists() {
            if !force {
                conflicts.push(name);
                continue;
            }
            if to.is_file() && from.is_file() && name.ends_with(".md") {
                let text = fs::read_to_string(&from)
                    .with_context(|| format!("cannot read {}", from.display()))?;
                library::replace_deck(dest, &name, &text, store)?;
                fs::remove_file(&from)
                    .with_context(|| format!("cannot remove {}", from.display()))?;
                moved += 1;
                continue;
            }
            if to.is_dir() {
                fs::remove_dir_all(&to)
                    .with_context(|| format!("cannot remove {}", to.display()))?;
            } else {
                fs::remove_file(&to).with_context(|| format!("cannot remove {}", to.display()))?;
            }
        }
        share::move_into(&from, &to)?;
        moved += 1;
    }
    Ok(MergeReport { moved, conflicts })
}

/// A cited deck that can't be frozen almost always has a broken or stale
/// `source:`, reported here rather than left as a silently empty `assets/`.
#[derive(Debug, Default)]
pub struct SnapshotSummary {
    pub decks: usize,
    pub files: usize,
    pub failed: Vec<String>,
}

/// Skips a deck with no citations; one that cites but can't be read is
/// recorded in [`SnapshotSummary::failed`], never silently dropped.
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
        match trace_ai::snapshot(&deck, summary.files, workspace_origin.as_deref()) {
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
            // The deck cites local excerpts but none could be frozen (almost
            // always a broken/stale `source:`); record it, don't swallow it.
            Err(e) => summary.failed.push(format!("{}: {e:#}", member.display())),
        }
    }
    Ok(summary)
}

fn file_name(item: &Item) -> String {
    format!("{:02}-{}.md", item.num, slug(&item.title))
}

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

/// Anchors overlap-aware via [`trace::resolve_under_base`]: a plain
/// `root.join(scope)` can double an already-rooted scope into a dead path.
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
        assert!(p.contains("understand the alix repo"));
        assert!(p.contains("SET OF MEANS"));
        assert!(p.contains("[trace]"));
        assert!(p.contains("[deck]"));
        assert!(p.contains("SHAPE"));
        assert!(p.contains("SATURATION"));
        assert!(p.contains("requires:"));
        assert!(p.contains("TOPOLOGICAL order"));
        assert!(p.contains("no cards, no checkpoints"));
        assert!(p.contains("Read, Glob"));
        assert!(!p.contains("WebFetch"));
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
        assert!(!p.contains("Glob"));
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
        assert!(p.contains("<!-- at: file:start-end -->"));
        assert!(!p.contains("TAB-indented"));
        assert!(p.contains("show its source on reveal"));
        assert!(p.contains("relative to the SAME root"));
        assert!(p.contains("enumeration"));
        assert!(p.contains("split it"));
    }

    #[test]
    fn walk_prompt_predicts_shape_with_evidence() {
        let p = walk_prompt(".", "understand the repo", false, &TraceConfig::default());
        assert!(p.contains("EXPLORE walk"));
        assert!(p.contains("PREDICT its"));
        assert!(p.contains("DOMAIN NOUNS"));
        assert!(p.contains("SPINE"));
        assert!(p.contains("CITE REAL STRUCTURAL EVIDENCE"));
        assert!(p.contains("candidate traces"));
        assert!(p.contains("<!-- at:"));
        assert!(p.contains("Read, Glob"));
        assert!(!p.contains("WebFetch"));
    }

    const SAMPLE_PLAN: &str = "\
Goal    understand X
Source  a thing
Spine   a -> b

1. [deck]  The deck format: markers and directives
   requires: none
   @source: README.md
2. [trace] How text becomes Cards
   requires: 1
   @source: src/parser.rs
10. [trace] How a request is served
    requires: 1, 2
    @source: src/serve.rs
";

    #[test]
    fn parse_plan_extracts_items_kinds_requires_and_source() {
        let items = parse_plan(SAMPLE_PLAN);
        assert_eq!(3, items.len());
        assert_eq!(Kind::Deck, items[0].kind);
        assert_eq!(1, items[0].num);
        assert!(items[0].title.starts_with("The deck format"));
        assert!(items[0].requires.is_empty());
        assert_eq!("README.md", items[0].source);
        assert_eq!(Kind::Trace, items[1].kind);
        assert_eq!(vec![1], items[1].requires);
        assert_eq!(10, items[2].num);
        assert_eq!(vec![1, 2], items[2].requires);
    }

    #[test]
    fn materialize_writes_manifest_and_wired_stubs() {
        let dir = std::env::temp_dir().join(format!("alix-explore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let report =
            materialize(SAMPLE_PLAN, &dir, "understand the repo", None, ".", None).unwrap();
        assert_eq!(2, report.traces);
        assert_eq!(1, report.decks);
        assert_eq!(0, report.filled);

        let manifest = fs::read_to_string(dir.join("alix.toml")).unwrap();
        assert!(manifest.contains("description = \"understand the repo\""));
        assert!(!manifest.contains("title ="));
        assert!(manifest.contains("[defaults]"));

        let deck = fs::read_to_string(dir.join("01-the-deck-format.md")).unwrap();
        assert!(deck.contains("# The Deck Format"));
        assert!(deck.contains("source: "));

        let trace = fs::read_to_string(dir.join("02-how-text-becomes-cards.md")).unwrap();
        assert!(trace.contains("trace: \"How Text Becomes Cards\""));
        assert!(trace.contains("- \"01-the-deck-format.md\""));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_writes_into_a_pre_existing_populated_dir_without_refusing() {
        let dir = std::env::temp_dir().join(format!("alix-explore-ne-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("keep.txt"), "existing").unwrap();

        materialize(SAMPLE_PLAN, &dir, "g", None, ".", None).unwrap();

        assert_eq!(
            "existing",
            fs::read_to_string(dir.join("keep.txt")).unwrap()
        );
        assert!(dir.join("alix.toml").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_writes_title_and_description() {
        let dir = std::env::temp_dir().join(format!("alix-explore-title-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        materialize(
            SAMPLE_PLAN,
            &dir,
            "the goal",
            Some("Repo Internals"),
            ".",
            None,
        )
        .unwrap();
        let manifest = fs::read_to_string(dir.join("alix.toml")).unwrap();
        assert!(manifest.contains("title = \"Repo Internals\""));
        assert!(manifest.contains("description = \"the goal\""));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_filled_splits_on_item_delimiters() {
        let raw = "\
preamble ignored
=== item 1 ===
## front a
back a
=== item 2 ===
# question
\tkey point
<!-- at: src/x.rs:1-3 -->
=== item 3 ===
";
        let filled = parse_filled(raw);
        assert_eq!(2, filled.len());
        assert!(filled[&1].contains("## front a"));
        assert!(filled[&2].contains("<!-- at: src/x.rs:1-3 -->"));
        assert!(!filled.contains_key(&3));
    }

    #[test]
    fn materialize_writes_filled_content_when_given() {
        let dir = std::env::temp_dir().join(format!("alix-explore-fill-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut filled = HashMap::new();
        filled.insert(
            2usize,
            "## how does text become Cards?\nparse_str runs a state machine\n<!-- at: src/parser.rs:1-9 -->"
                .to_string(),
        );

        let report = materialize(SAMPLE_PLAN, &dir, "g", None, ".", Some(&filled)).unwrap();
        assert_eq!(1, report.filled);

        let trace = fs::read_to_string(dir.join("02-how-text-becomes-cards.md")).unwrap();
        assert!(trace.contains("trace: \"How Text Becomes Cards\""));
        assert!(trace.contains("## how does text become Cards?"));
        assert!(trace.contains("<!-- at: src/parser.rs:1-9 -->"));
        let deck = fs::read_to_string(dir.join("01-the-deck-format.md")).unwrap();
        assert!(deck.contains("Stub from `alix generate`"));

        let _ = fs::remove_dir_all(&dir);
    }

    fn merge_test_dirs(label: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("alix-merge-{label}-{}", std::process::id()));
        let staging = root.join("staging");
        let dest = root.join("dest");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&staging).unwrap();
        (staging, dest)
    }

    #[test]
    fn a_clean_merge_moves_every_entry_and_reports_zero_conflicts() {
        let (staging, dest) = merge_test_dirs("clean");
        fs::write(staging.join("alix.toml"), "[defaults]\n").unwrap();
        fs::write(staging.join("01-a.txt"), "deck a\n").unwrap();

        let mut store = Store::open(dest.join("progress.json")).unwrap();
        let report = merge_built(&staging, &dest, false, &mut store).unwrap();

        assert_eq!(2, report.moved);
        assert!(report.conflicts.is_empty());
        assert_eq!(
            "[defaults]\n",
            fs::read_to_string(dest.join("alix.toml")).unwrap()
        );
        assert_eq!(
            "deck a\n",
            fs::read_to_string(dest.join("01-a.txt")).unwrap()
        );
        assert_eq!(0, fs::read_dir(&staging).unwrap().count());

        let _ = fs::remove_dir_all(staging.parent().unwrap());
    }

    #[test]
    fn a_name_collision_keeps_the_original_content_and_leaves_the_new_file_in_staging() {
        let (staging, dest) = merge_test_dirs("collide");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("01-a.txt"), "the user's own deck\n").unwrap();
        fs::write(staging.join("01-a.txt"), "the freshly generated deck\n").unwrap();

        let mut store = Store::open(dest.join("progress.json")).unwrap();
        let report = merge_built(&staging, &dest, false, &mut store).unwrap();

        assert_eq!(0, report.moved);
        assert_eq!(vec!["01-a.txt".to_string()], report.conflicts);
        assert_eq!(
            "the user's own deck\n",
            fs::read_to_string(dest.join("01-a.txt")).unwrap()
        );
        assert_eq!(
            "the freshly generated deck\n",
            fs::read_to_string(staging.join("01-a.txt")).unwrap()
        );

        let _ = fs::remove_dir_all(staging.parent().unwrap());
    }

    #[test]
    fn force_replaces_a_colliding_file_with_the_new_version() {
        let (staging, dest) = merge_test_dirs("force");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("01-a.txt"), "stale\n").unwrap();
        fs::write(staging.join("01-a.txt"), "fresh\n").unwrap();

        let mut store = Store::open(dest.join("progress.json")).unwrap();
        let report = merge_built(&staging, &dest, true, &mut store).unwrap();

        assert_eq!(1, report.moved);
        assert!(report.conflicts.is_empty());
        assert_eq!(
            "fresh\n",
            fs::read_to_string(dest.join("01-a.txt")).unwrap()
        );

        let _ = fs::remove_dir_all(staging.parent().unwrap());
    }

    #[test]
    fn a_forced_md_collision_routes_through_the_replace_protocol() {
        let (staging, dest) = merge_test_dirs("md-replace");
        fs::create_dir_all(&dest).unwrap();
        fs::write(
            dest.join("01-a.md"),
            "---\nid: \"da1\"\n---\n## old <!-- id: c1 -->\nold\n",
        )
        .unwrap();
        fs::write(staging.join("01-a.md"), "## new q\nnew ans\n").unwrap();
        let mut store = Store::open(dest.join("progress.json")).unwrap();
        store.get_or_insert("c1", 0);
        store.save().unwrap();

        let report = merge_built(&staging, &dest, true, &mut store).unwrap();

        assert_eq!(1, report.moved);
        assert!(store.get("c1").is_none());
        assert!(dest.join("01-a.md.bak").exists());
        assert!(
            fs::read_to_string(dest.join("01-a.md"))
                .unwrap()
                .contains("new q")
        );
        assert!(!staging.join("01-a.md").exists());

        let _ = fs::remove_dir_all(staging.parent().unwrap());
    }

    #[test]
    fn a_directory_entry_merges_as_one_unit_under_the_same_collision_rule() {
        let (staging, dest) = merge_test_dirs("dir");
        fs::create_dir_all(staging.join("assets")).unwrap();
        fs::write(staging.join("assets/img.svg"), "new\n").unwrap();

        let mut store = Store::open(dest.join("progress.json")).unwrap();
        let report = merge_built(&staging, &dest, false, &mut store).unwrap();
        assert_eq!(1, report.moved);
        assert!(report.conflicts.is_empty());
        assert_eq!(
            "new\n",
            fs::read_to_string(dest.join("assets/img.svg")).unwrap()
        );

        fs::create_dir_all(staging.join("assets")).unwrap();
        fs::write(staging.join("assets/img.svg"), "newer\n").unwrap();
        let report = merge_built(&staging, &dest, false, &mut store).unwrap();
        assert_eq!(0, report.moved);
        assert_eq!(vec!["assets".to_string()], report.conflicts);
        assert_eq!(
            "new\n",
            fs::read_to_string(dest.join("assets/img.svg")).unwrap()
        );

        let _ = fs::remove_dir_all(staging.parent().unwrap());
    }

    #[test]
    fn snapshot_workspace_freezes_traces_and_cited_facts() {
        let dir = std::env::temp_dir().join(format!("alix-snap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/a.rs"), "x\ny\nz\n").unwrap();
        fs::write(dir.join("alix.toml"), "[defaults]\n").unwrap();
        let src = dir.join("src");
        fs::write(
            dir.join("01-t.md"),
            format!(
                "---\ntrace: t\nsource: {}\n---\n## h\np\n<!-- at: a.rs:1-2 -->\n",
                src.display()
            ),
        )
        .unwrap();
        fs::write(
            dir.join("02-d.md"),
            format!(
                "---\nsource: {}\n---\n## q\na\n<!-- at: a.rs:3 -->\n",
                src.display()
            ),
        )
        .unwrap();
        fs::write(dir.join("03-plain.md"), "# d\n## q\na\n").unwrap();

        let summary = snapshot_workspace(&dir).unwrap();
        assert_eq!((2, 2), (summary.decks, summary.files));
        assert!(summary.failed.is_empty(), "{:?}", summary.failed);
        assert!(dir.join("assets/01.rs").is_file());
        assert!(dir.join("assets/02.rs").is_file());
        assert!(!dir.join("assets/a.rs").exists());
        let fact = fs::read_to_string(dir.join("02-d.md")).unwrap();
        assert!(fact.contains("source: assets\n"), "{fact}");
        assert!(!fact.contains("<!-- at: a.rs:3 -->"), "{fact}");
        assert!(fact.contains("<!-- at: 0"), "{fact}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_workspace_surfaces_a_deck_whose_source_cannot_be_frozen() {
        let dir = std::env::temp_dir().join(format!("alix-snap-fail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("alix.toml"), "[defaults]\n").unwrap();
        fs::write(
            dir.join("01-broken.md"),
            format!(
                "---\nsource: {}/does-not-exist\n---\n## q\na\n<!-- at: src/x.rs:1 -->\n",
                dir.display()
            ),
        )
        .unwrap();

        let summary = snapshot_workspace(&dir).unwrap();
        assert_eq!(0, summary.decks);
        assert_eq!(1, summary.failed.len(), "{:?}", summary.failed);
        assert!(
            summary.failed[0].contains("01-broken.md"),
            "{:?}",
            summary.failed
        );
        assert!(!dir.join("assets").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rewrite_scope_anchors_a_repo_relative_scope_without_doubling() {
        let root = std::env::temp_dir().join(format!("alix-scope-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let crate_src = root.join("crates/mycrate/src");
        fs::create_dir_all(&crate_src).unwrap();
        fs::write(crate_src.join("lib.rs"), "fn main() {}\n").unwrap();
        let lib = crate_src.join("lib.rs").display().to_string();
        let source = root.join("crates/mycrate");

        assert_eq!(
            lib,
            rewrite_scope("crates/mycrate/src/lib.rs", Some(&source))
        );
        assert_eq!(lib, rewrite_scope("src/lib.rs", Some(&source)));
        assert_eq!(
            source.display().to_string(),
            rewrite_scope(".", Some(&source))
        );
        assert_eq!(lib, rewrite_scope(&lib, Some(&source)));
        assert_eq!(
            format!("{lib} + other.rs"),
            rewrite_scope("crates/mycrate/src/lib.rs + other.rs", Some(&source))
        );

        let _ = fs::remove_dir_all(&root);
    }
}
