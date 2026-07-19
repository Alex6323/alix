use std::path::{Path, PathBuf};

use alix::{config::Config, deck::Deck, generate, l1, library};
use anyhow::{Context, Result, bail};

use crate::{
    GenerateArgs,
    common::{confirm, deck_out_dir, preflight_source, store_for},
};

pub(crate) fn generate_cmd(args: GenerateArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let goal = args
        .goal
        .as_deref()
        .unwrap_or("understand the whole source");
    let src_path = PathBuf::from(&args.source);

    if src_path.is_file()
        && src_path.extension().is_some_and(|e| e == "md")
        && std::fs::read_to_string(&src_path).is_ok_and(|t| {
            alix::l1::parse_l1("stub.md", &t).is_ok_and(|d| d.frontmatter.trace.is_some())
        })
    {
        let deck = Deck::load(&src_path)?;
        return trace_build(&src_path, &deck, args.yes, args.force, &config);
    }

    if args.trace {
        if args.plan {
            return trace_suggest(&args.source, args.yes, &config);
        }
        return generate_trace_walk(&args, &config, goal);
    }

    if src_path.is_dir() && !args.deck {
        let source = canonical_source(&args.source);
        // Confirm before any exploration call, so a decline never spends a
        // paid backend request.
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

/// Dot-prefixed so `picker::dir_candidates` skips it: a staging dir kept on
/// a merge conflict never leaks into the picker as a bogus workspace.
fn staging_dir_for(dir: &Path) -> PathBuf {
    let staging_name = format!(
        ".{}.building",
        dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
    );
    dir.with_file_name(staging_name)
}

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
    let mut store = alix::store::Store::open(alix::workspace::root_store_path(&dir))
        .with_context(|| format!("opening the store for {}", dir.display()))?;
    let merged = alix::explore::merge_built(&staging, &dir, args.force, &mut store)?;

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

fn generate_single_deck(args: &GenerateArgs, config: &Config) -> Result<()> {
    let mut gen_cfg = config.generate.clone();
    if let Some(cards) = args.cards {
        gen_cfg.max_cards = cards;
    }

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

    // Parse against the final name (part of every card's id hash); a parse
    // error still saves the output rather than losing the generation.
    let name = match &args.output {
        Some(name) => name.clone(),
        None => generate::deck_name(&source),
    };
    let name = if name.ends_with(".md") {
        name
    } else {
        format!("{name}.md")
    };

    if args.print {
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
        match l1::parse_str(&name, &text) {
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
        let mut store = store_for(std::slice::from_ref(&target), None, config)?;
        let report = library::replace_deck(&dir, &name, &text, &mut store)?;
        println!(
            "Replaced {}: {} cards, wiped progress for {} card(s).",
            target.display(),
            report.minted,
            report.wiped_cards
        );
        return Ok(());
    }
    let placed = library::place_deck(&dir, &name, &text)?;
    match placed.parse_error {
        None => {
            println!("Wrote {} cards to {}", placed.cards, placed.path.display());
            Ok(())
        }
        Some(e) => bail!(
            "Saved the generated deck to {}, but it does not parse yet:\n  {e}\n\
             Fix that line and run `alix doctor {}`.",
            placed.path.display(),
            placed.path.display()
        ),
    }
}

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn trace_build(
    deck_path: &Path,
    deck: &Deck,
    yes: bool,
    force: bool,
    config: &Config,
) -> Result<()> {
    if !deck.is_trace() {
        bail!(
            "{} declares no `trace:` — add the path you want to understand \
             (e.g. a frontmatter `trace: how X becomes Y`), then build it",
            deck.subject
        );
    }
    if deck.sources.is_empty() {
        bail!(
            "{} declares no `source:` — add the scope to trace (a repo `.`, a \
             directory, a file, or a URL)",
            deck.subject
        );
    }
    let rebuild = !deck.cards.is_empty();
    if rebuild && !force {
        bail!(
            "{} already has checkpoints; pass --force to rebuild (this wipes their progress)",
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

    if rebuild {
        let existing = std::fs::read_to_string(deck_path)
            .with_context(|| format!("cannot read {}", deck_path.display()))?;
        let new_text = alix::deck::trace_checkpoint_text(deck_path, &existing, &cards)?;
        let dir = deck_path.parent().unwrap_or_else(|| Path::new("."));
        let name = deck_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("trace.md");
        let mut store = store_for(std::slice::from_ref(&deck_path.to_path_buf()), None, config)?;
        let report = library::replace_deck(dir, name, &new_text, &mut store)?;
        println!(
            "Rebuilt {}: {} checkpoints, wiped progress for {} card(s). Review them \
             and their `at:` locators, then walk it from the picker.",
            deck_path.display(),
            report.minted,
            report.wiped_cards
        );
        return Ok(());
    }

    alix::deck::set_trace_checkpoints(deck_path, &cards)?;
    // Stamp at birth (mints token ids); failure is loud but non-fatal since
    // review-open stamps again.
    if let Err(e) = alix::stamp::stamp_deck(deck_path) {
        eprintln!("warning: cannot stamp {}: {e}", deck_path.display());
    }

    let n = alix::l1::parse_str(&deck.subject, &cards)
        .map(|c| c.len())
        .unwrap_or(0);
    println!(
        "Wrote {n} checkpoints to {}. Review them and their `at:` locators, \
         then walk it from the picker: run `alix` and pick it.",
        deck_path.display()
    );
    Ok(())
}

fn trace_suggest(source: &str, yes: bool, config: &Config) -> Result<()> {
    preflight_source(source, config.ask.preflight_threshold, yes)?;
    eprintln!(
        "Reconning {source} for traces worth tracing (one exploration pass — this \
         can take a minute)…"
    );
    let menu = alix::trace_ai::suggest(source, &config.trace, &config.ask)?;
    println!("{menu}");
    println!(
        "\n{DIM}Paste a suggestion into a new deck's frontmatter (its `trace:` + \
         `source:` keys), then build it:  alix generate <deck>{RESET}"
    );
    Ok(())
}

fn generate_trace_walk(args: &GenerateArgs, config: &Config, goal: &str) -> Result<()> {
    let source = canonical_source(&args.source);
    preflight_source(&source, config.ask.preflight_threshold, args.yes)?;
    eprintln!(
        "Exploring {source} to build an explore walk (one pass — this can take a \
         minute)…"
    );
    let checkpoints = alix::explore::walk(&source, goal, &config.trace, &config.ask)?;

    let name = Path::new(&source)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(source.as_str());
    // Quote through the L1 quoter: a name/source with `"`/`\` or `:` would
    // otherwise break the YAML mapping.
    let trace = l1::yaml_quote(&format!(
        "exploring {name} — what it is, its parts, and its spine"
    ));
    let deck_text = format!(
        "---\ntrace: {trace}\nsource: {}\n---\n\n{checkpoints}\n",
        l1::yaml_quote(&source)
    );
    let dir = deck_out_dir(args.workspace.as_deref(), config)?;
    let raw = PathBuf::from(args.output.clone().unwrap_or_else(|| "explore.md".into()));
    let out = if args.workspace.is_some() {
        dir.join(&raw)
    } else {
        raw
    };
    let out_dir = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let name = out
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("explore.md")
        .to_string();
    if out.exists() {
        if !args.force {
            bail!(
                "{} already exists; pass --force to overwrite",
                out.display()
            );
        }
        let mut store = store_for(std::slice::from_ref(&out), None, config)?;
        let report = library::replace_deck(&out_dir, &name, &deck_text, &mut store)?;
        println!(
            "Rebuilt the explore walk at {}: {} checkpoints, wiped progress for {} card(s).",
            out.display(),
            report.minted,
            report.wiped_cards
        );
        return Ok(());
    }
    let placed = library::place_deck(&out_dir, &name, &deck_text)?;
    if let Some(e) = &placed.parse_error {
        eprintln!("warning: the explore walk does not parse yet: {e}");
    }
    println!(
        "Wrote the explore walk to {} — walk it from the picker: run `alix` and \
         pick it.",
        placed.path.display()
    );
    Ok(())
}
