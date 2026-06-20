//! `flash explore` — the orient tier (first slice).
//!
//! Where [`crate::trace::suggest`] is the flat recon menu of candidate *traces*,
//! `explore` is goal-driven orientation: given a source and a learning **goal**,
//! it manufactures the ordered set of **means** — fact *decks* and *traces* —
//! that, worked through, would reach the goal. The means are chosen by the shape
//! of the knowledge (edges → traces, nodes → decks), sized to the goal by
//! saturation, and ordered by prerequisite. This first slice **prints the plan**
//! and writes nothing; materializing it into a workspace comes later.

use anyhow::{Result, bail};

use crate::{
    ask,
    config::{AskConfig, TraceConfig},
    deck::is_url,
    trace::{build_run_config, resolve_source},
};

/// Explore a source toward `goal` and return an ordered learning plan — the
/// decks and traces worth authoring, each tagged and dependency-ordered. One
/// read-only exploration pass (the same tools and cwd as [`crate::trace::build`]);
/// discovers nothing in depth and writes nothing. `source` is a scope directly
/// (a repo `.`, a directory, a file, or a URL), not a deck.
pub fn explore(
    source: &str,
    goal: &str,
    cfg: &TraceConfig,
    ask_cfg: &AskConfig,
) -> Result<String> {
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

/// Builds the orientation prompt: explore the source and emit an ordered,
/// prerequisite-sorted plan of means (decks + traces) sized to the goal by
/// saturation. The counterpart to [`crate::trace::suggest`]'s recon prompt, one
/// tier up (see `docs/traces.md`, "Goals and orientation").
fn explore_prompt(source: &str, goal: &str, url: bool, cfg: &TraceConfig) -> String {
    let explore = if url {
        format!("Read the source page at {source} with the WebFetch tool (fetch it once).")
    } else {
        "Your working directory is the source root. Explore it with the Read, Glob \
         and Grep tools — orient the way you would cold: the manifest (what kind of \
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
        "You are ORIENTING a learner whose GOAL is:\n\n    {goal}\n\nover this \
         source. Produce the ordered SET OF MEANS — fact decks and traces — that, \
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
         three orienting lines, then the numbered items:\n\n\
         Goal    {goal}\n\
         Source  <one line: what this source is>\n\
         Spine   <the single most central path, arrow-joined nouns>\n\n\
         1. [deck]  <the fact topic, e.g. the deck format: directives and markers>\n   \
         requires: none\n   % source: <{scope}>\n\
         2. [trace] <the path-question, e.g. how deck text becomes a list of Cards>\n   \
         requires: 1\n   % source: <{scope}>\n\
         3. …\n\n\
         Tag EVERY item [trace] or [deck]. A [trace] title is a path-question with \
         two ends; a [deck] title names a coherent set of facts. Keep each title one \
         line; do not resolve line numbers or write cards/checkpoints — later steps \
         do that. Use as many items as the goal needs (stop at saturation), ordered \
         by prerequisite."
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explore_prompt_carries_goal_means_coverage_and_order() {
        let p = explore_prompt(".", "understand the flash repo", false, &TraceConfig::default());
        assert!(p.contains("understand the flash repo")); // the goal is echoed
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
        let p = explore_prompt("https://x", "understand the page", true, &TraceConfig::default());
        assert!(p.contains("WebFetch"));
        assert!(!p.contains("Glob")); // no local file tools for a URL source
    }
}
