//! `alix generate`: AI authoring — a single deck, a multi-deck workspace, or
//! a trace over a source's shape. Routes on the source (file, URL, directory,
//! or an existing `% trace:` stub) and on `--trace`/`--plan`.

use std::path::{Path, PathBuf};

use alix::{config::Config, deck::Deck, generate, library, parser};
use anyhow::{Context, Result, bail};

use crate::{
    GenerateArgs,
    common::{confirm, deck_out_dir, preflight_source},
};

/// `alix generate`: one entry for all AI authoring. Routes by what the source
/// is — an existing `% trace:` stub builds in place; `--trace` authors a trace
/// over a source; a directory is explored first and the plan's size decides
/// deck vs workspace (confirmed before the expensive build); anything else
/// becomes a single deck.
pub(crate) fn generate_cmd(args: GenerateArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let goal = args
        .goal
        .as_deref()
        .unwrap_or("understand the whole source");
    let src_path = PathBuf::from(&args.source);

    // Naming an existing trace stub (`% trace:`) builds its checkpoints in
    // place; a plain text file without the directive is treated as source
    // material below.
    if src_path.is_file()
        && src_path.extension().is_some_and(|e| e == "txt")
        && std::fs::read_to_string(&src_path)
            .is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("% trace:")))
    {
        let deck = Deck::load(&src_path)?;
        return trace_build(&src_path, &deck, args.yes, &config);
    }

    // `--trace`: author a trace over the source — a suggestions menu with
    // `--plan`, else the explore walk written as a trace deck.
    if args.trace {
        if args.plan {
            return trace_suggest(&args.source, args.yes, &config);
        }
        return generate_trace_walk(&args, &config, goal);
    }

    // A directory source is explored first; the plan's size decides.
    if src_path.is_dir() && !args.deck {
        let source = canonical_source(&args.source);
        // A leftover staging dir from a previous build holds merge-conflict
        // files that only exist there — confirm before a rebuild wipes them,
        // and do it before any exploration call so a decline never spends a
        // backend request. `--plan` never builds (and so never wipes), so it
        // skips the question.
        if !args.plan {
            let staging = staging_dir_for(&workspace_dest(&args, &config, &source)?);
            if !confirm_stale_staging(&staging, args.yes)? {
                println!("Cancelled.");
                return Ok(());
            }
            let _ = std::fs::remove_dir_all(&staging);
        }
        preflight_source(&source, config.ask.preflight_threshold, args.yes)?;
        eprintln!(
            "Exploring {source} for a learning plan toward \"{goal}\" (one pass — \
             this can take a minute)…"
        );
        let plan = alix::explore::explore(&source, goal, &config.trace, &config.ask)?;
        let items = alix::explore::parse_plan(&plan).len();
        println!("{plan}");
        if args.plan {
            return Ok(());
        }
        if items > 1 {
            return build_workspace(&args, &config, &source, goal, items);
        }
        eprintln!("The plan has one item — generating a single deck.");
    }
    generate_single_deck(&args, &config)
}

/// A local source as an absolute path (so written `% source:` lines resolve
/// from anywhere); a URL passes through unchanged.
fn canonical_source(source: &str) -> String {
    let path = Path::new(source);
    if path.exists() {
        std::fs::canonicalize(path)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| source.to_string())
    } else {
        source.to_string()
    }
}

/// Where a directory source's workspace lands: `--workspace` when given, else
/// a folder named after the source under the decks directory.
fn workspace_dest(args: &GenerateArgs, config: &Config, source: &str) -> Result<PathBuf> {
    Ok(match &args.workspace {
        Some(dir) => dir.clone(),
        None => {
            let name = Path::new(source)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace");
            config
                .decks_dir()
                .context("cannot determine the decks directory")?
                .join(name)
        }
    })
}

/// The scratch dir `build_workspace` stages a build's new files into before
/// merging them into `dir`: `.<name>.building` beside it. Dot-prefixed, so a
/// staging dir deliberately kept on a merge conflict never leaks into the
/// picker as a bogus workspace (`picker::dir_candidates` skips dot-prefixed
/// entries for exactly this reason).
fn staging_dir_for(dir: &Path) -> PathBuf {
    let staging_name = format!(
        ".{}.building",
        dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
    );
    dir.with_file_name(staging_name)
}

/// `Ok(true)` to proceed — `staging` is absent or empty, so there is nothing
/// to lose. Otherwise a previous build's merge conflicts are the only copy of
/// their new content, so this asks before a rebuild would silently wipe them;
/// `Ok(false)` means the caller should stop without building.
fn confirm_stale_staging(staging: &Path, yes: bool) -> Result<bool> {
    let has_files = std::fs::read_dir(staging).is_ok_and(|mut entries| entries.next().is_some());
    if !has_files {
        return Ok(true);
    }
    confirm(
        &format!(
            "{} holds files from a previous build — wipe them and rebuild?",
            staging.display()
        ),
        yes,
    )
}

/// Builds a workspace from a multi-item plan: confirm, then explore + fill in
/// one session (a second exploration — the coherent fill needs its own pass),
/// materialize into a scratch staging dir, and merge that into the
/// destination — so a populated destination never blocks the build or loses a
/// file (a name collision keeps the user's original; `--force` overwrites).
/// Ported from the old `explore --build`.
fn build_workspace(
    args: &GenerateArgs,
    config: &Config,
    source: &str,
    goal: &str,
    items: usize,
) -> Result<()> {
    let dir = workspace_dest(args, config, source)?;
    if !confirm(
        &format!(
            "Build {items} items into {} (several AI calls, a few minutes)?",
            dir.display()
        ),
        args.yes,
    )? {
        println!("Cancelled — `--plan` prints the plan without building.");
        return Ok(());
    }
    eprintln!(
        "Exploring {source} and filling the workspace toward \"{goal}\" (explore \
         + fill in one session — this can take a few minutes)…"
    );
    let (plan, filled) = alix::explore::explore_and_fill(source, goal, &config.trace, &config.ask)?;
    println!("{plan}");

    // Materialize into a fresh staging dir beside the destination, then merge
    // the new files in one by one: a populated destination never blocks the
    // build or loses a file, and exploration tokens are never wasted on a
    // doomed run — a name collision just keeps the user's original. Any
    // leftover from a previous build was already confirmed-and-wiped (or was
    // empty/absent) by the caller before this build spent a single AI call,
    // so this is a no-op in the common case.
    let staging = staging_dir_for(&dir);
    let _ = std::fs::remove_dir_all(&staging);
    let materialized = alix::explore::materialize(
        &plan,
        &staging,
        goal,
        args.title.as_deref(),
        source,
        Some(&filled),
    )?;
    let merged = alix::explore::merge_built(&staging, &dir, args.force)?;

    let total = materialized.traces + materialized.decks;
    let stubs = total - materialized.filled;
    println!(
        "\n{BOLD}Built {total} files{RESET} in {} — {} filled, {stubs} stub(s) \
         ({} traces, {} decks).",
        dir.display(),
        materialized.filled,
        materialized.traces,
        materialized.decks,
    );
    if merged.conflicts.is_empty() {
        let _ = std::fs::remove_dir_all(&staging);
    } else {
        for name in &merged.conflicts {
            eprintln!(
                "kept yours: {name} — the new version is at {}/{name}",
                staging.display()
            );
        }
        eprintln!("re-run with --force to overwrite, or move them in by hand.");
    }
    // Freeze each cited deck's source into the workspace's `assets/` so its
    // locators never drift and the workspace is self-contained.
    match alix::explore::snapshot_workspace(&dir) {
        Ok(summary) => {
            if summary.decks > 0 {
                println!(
                    "{DIM}Froze {} excerpt(s) from {} deck(s) into {}/assets — \
                     the citations won't drift.{RESET}",
                    summary.files,
                    summary.decks,
                    dir.display(),
                );
            }
            for failed in &summary.failed {
                eprintln!("warning: could not freeze {failed}");
            }
        }
        Err(e) => eprintln!("warning: could not snapshot the source: {e:#}"),
    }
    // A workspace icon: the user's file if given, else an abstract emblem the
    // model draws from what it just built. Best-effort — never fails the build.
    match args.icon.as_deref() {
        Some(src) => match alix::icon::install(&dir, src) {
            Ok(_) => println!(
                "{DIM}Installed the workspace icon into {}/assets.{RESET}",
                dir.display()
            ),
            Err(e) => eprintln!("warning: could not install the workspace icon: {e:#}"),
        },
        None => match alix::icon::generate(&dir, None, &config.ask) {
            Ok(_) => println!(
                "{DIM}Drew a workspace icon into {}/assets.{RESET}",
                dir.display()
            ),
            Err(e) => eprintln!("warning: could not draw a workspace icon: {e:#}"),
        },
    }
    println!("{DIM}Open it:  alix {}{RESET}", dir.display());
    Ok(())
}

/// Generates one facts deck from `args.source` (a URL or a file), writing it
/// into `--workspace <dir>` when given, else the decks directory.
fn generate_single_deck(args: &GenerateArgs, config: &Config) -> Result<()> {
    let mut gen_cfg = config.generate.clone();
    if let Some(cards) = args.cards {
        gen_cfg.max_cards = cards;
    }

    // For a local source, use an absolute path so the deck's `% source:` line
    // resolves later (it's written into the decks dir, not next to the source);
    // a URL stays as-is.
    let source = if std::path::Path::new(&args.source).exists() {
        std::fs::canonicalize(&args.source)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| args.source.clone())
    } else {
        args.source.clone()
    };

    preflight_source(&source, config.ask.preflight_threshold, args.yes)?;
    eprintln!("Generating a deck from {source} (this can take a minute)…");
    let mut text = generate::generate_deck(&source, &gen_cfg, &config.ask)?;

    if args.review || gen_cfg.review {
        eprintln!("Reviewing the deck to remove redundant cards…");
        text = generate::review_deck(&text, &gen_cfg, &config.ask)?;
    }

    // The subject (file name) is part of every card's identity hash, so parse
    // against the final name. A parse problem does not discard the output — the
    // deck is still saved (or printed) so a single bad line can be fixed by
    // hand rather than losing the whole generation.
    let name = match &args.output {
        Some(name) => name.clone(),
        None => generate::deck_name(&source),
    };
    let name = if name.ends_with(".txt") {
        name
    } else {
        format!("{name}.txt")
    };

    if args.print {
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
        match parser::parse_str(&name, &text) {
            Ok(cards) => eprintln!("({} cards — not written; --print)", cards.len()),
            Err(e) => eprintln!("(warning: does not parse yet — {e})"),
        }
        return Ok(());
    }

    let dir = deck_out_dir(args.workspace.as_deref(), config)?;
    let target = dir.join(&name);
    if target.exists() {
        if !args.force {
            bail!(
                "{} already exists; pass --force to overwrite",
                target.display()
            );
        }
        // --force is CLI-only: clear the collision before placing.
        std::fs::remove_file(&target)
            .with_context(|| format!("cannot overwrite {}", target.display()))?;
    }
    let placed = library::place_deck(&dir, &name, &text)?;
    match placed.parse_error {
        None => {
            println!("Wrote {} cards to {}", placed.cards, placed.path.display());
            Ok(())
        }
        // Saved, but not yet valid: tell the user exactly what to fix.
        Some(e) => bail!(
            "Saved the generated deck to {}, but it does not parse yet:\n  {e}\n\
             Fix that line and run `alix doctor {}`.",
            placed.path.display(),
            placed.path.display()
        ),
    }
}

// ANSI styling for the linear `alix trace` flow (it requires a terminal).
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// Discovers the path with Claude (`alix trace --build`) and writes the
/// checkpoints back into the deck file, keeping its `% trace:`/`% source:`
/// header.
fn trace_build(deck_path: &Path, deck: &Deck, yes: bool, config: &Config) -> Result<()> {
    if !deck.is_trace() {
        bail!(
            "{} declares no `% trace:` — add the path you want to understand \
             (e.g. `% trace: how X becomes Y`), then build it",
            deck.subject
        );
    }
    if deck.sources.is_empty() {
        bail!(
            "{} declares no `% source:` — add the scope to trace (a repo `.`, a \
             directory, a file, or a URL)",
            deck.subject
        );
    }
    let source = deck.sources.first().map(String::as_str).unwrap_or_default();
    preflight_source(source, config.ask.preflight_threshold, yes)?;
    eprintln!(
        "Tracing a path through {source} (exploring the source — this can take a \
         few minutes)…"
    );
    let cards = alix::trace_ai::build(deck, &config.trace, &config.ask)?;
    alix::deck::set_trace_checkpoints(deck_path, &cards)?;

    let n = parser::parse_str(&deck.subject, &cards)
        .map(|c| c.len())
        .unwrap_or(0);
    println!(
        "Wrote {n} checkpoints to {}. Review them and their `% at:` locators, \
         then walk it from the picker: run `alix` and pick it.",
        deck_path.display()
    );
    Ok(())
}

/// `--suggest`: recon a source (a repo, directory, file, or URL — the
/// positional, NOT a deck) and print a ranked menu of candidate traces to
/// author. Read-only exploration; writes nothing. The cheap precursor to
/// `--build` — pick a suggestion, paste it into a new deck, then build that.
fn trace_suggest(source: &str, yes: bool, config: &Config) -> Result<()> {
    preflight_source(source, config.ask.preflight_threshold, yes)?;
    eprintln!(
        "Reconning {source} for traces worth tracing (one exploration pass — this \
         can take a minute)…"
    );
    let menu = alix::trace_ai::suggest(source, &config.trace, &config.ask)?;
    println!("{menu}");
    println!(
        "\n{DIM}Paste a suggestion into a new deck (its `% trace:` + `% source:`), \
         then build it:  alix generate <deck>{RESET}"
    );
    Ok(())
}

/// `alix explore --walk`: build an explore walk over a source's shape and walk
/// it immediately. Writes the trace to a file (default `explore.txt`) with an
/// absolute `% source:` so it re-walks from anywhere, then runs the shared walk.
/// Authors a trace over the source's shape (what it is → parts → entry →
/// spine), written as a trace deck. The old explore-walk, minus the terminal
/// walk — walking happens in the browser now.
fn generate_trace_walk(args: &GenerateArgs, config: &Config, goal: &str) -> Result<()> {
    let source = canonical_source(&args.source);
    preflight_source(&source, config.ask.preflight_threshold, args.yes)?;
    eprintln!(
        "Exploring {source} to build an explore walk (one pass — this can take a \
         minute)…"
    );
    let checkpoints = alix::explore::walk(&source, goal, &config.trace, &config.ask)?;

    // Wrap the checkpoints in a trace deck with an absolute `% source:` root so
    // the saved walk reads the right files from anywhere.
    let name = Path::new(&source)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(source.as_str());
    let deck_text = format!(
        "% trace: exploring {name} — what it is, its parts, and its spine\n\
         % source: {source}\n\n{checkpoints}\n"
    );
    let out = PathBuf::from(args.output.clone().unwrap_or_else(|| "explore.txt".into()));
    if out.exists() && !args.force {
        bail!(
            "{} already exists; pass --force to overwrite",
            out.display()
        );
    }
    let dir = deck_out_dir(args.workspace.as_deref(), config)?;
    let out = if args.workspace.is_some() {
        dir.join(out)
    } else {
        out
    };
    std::fs::write(&out, &deck_text).with_context(|| format!("cannot write {}", out.display()))?;
    println!(
        "Wrote the explore walk to {} — walk it from the picker: run `alix` and \
         pick it.",
        out.display()
    );
    Ok(())
}
