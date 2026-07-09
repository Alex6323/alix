mod common;
mod launch;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use alix::{
    augment::{self, AugmentCache},
    card::Card,
    config::{self, Config},
    deck::{Deck, DeckState},
    depth::Depth,
    generate, import, library, parser,
    scheduler::{Fsrs, Scheduler},
    store::Store,
    time::{humanize_ms, now_ms},
    trace::{SourceBase, Trace},
    workspace,
};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use common::*;
use launch::*;

/// A learning tool built for understanding, not just remembering.
///
/// Without a subcommand, alix serves its web app: the in-browser deck
/// picker over your decks directory, or over the folder you name.
/// Manual: https://alix.study/book
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    launch: LaunchArgs,
}

/// The bare `alix [dir]` launcher: everything is picked in the browser, so the
/// top level carries only what it takes to spin up the server itself.
#[derive(Args)]
struct LaunchArgs {
    /// A decks folder or a workspace to serve as this instance's own root:
    /// scoped to that folder, with its own progress and recent state inside it.
    /// Default: the configured decks directory with the global state.
    dir: Option<PathBuf>,

    /// Port to listen on (default: the `[serve]` config port, 7777).
    #[arg(long)]
    port: Option<u16>,

    /// Listen on all network interfaces so phones and tablets on the same
    /// network can reach it; generates and prints a pairing token (and QR).
    #[arg(long)]
    lan: bool,

    /// Pairing token required on `/api/*`. Defaults to a value auto-generated
    /// (and printed) for `--lan`.
    #[arg(long)]
    token: Option<String>,

    /// Max new (never-seen) cards a session introduces (default: the
    /// `[review] max_new` config key, else 10).
    #[arg(long)]
    new: Option<usize>,

    /// Max cards per session (default: the `[review] limit` config key,
    /// else no cap).
    #[arg(long)]
    limit: Option<usize>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Check this setup's health, with a one-line fix per problem.
    ///
    /// Covers the config, the progress store, the decks folder, and the
    /// optional external CLIs. Add `--backends` to also probe the configured
    /// AI backend end to end (one real, tiny request).
    Doctor(DoctorArgs),
    /// Generate learning material with AI: a deck, a trace, or a workspace.
    ///
    /// A web page or file source becomes one deck. A directory source is
    /// explored first: a one-item plan becomes a deck, a bigger plan becomes
    /// a workspace (shown and confirmed before building). `--plan` previews,
    /// `--trace` authors a trace, and naming an existing `% trace:` stub
    /// builds its checkpoints in place.
    Generate(GenerateArgs),
    /// Show progress statistics for a deck, a folder, or a workspace.
    ///
    /// The target is a path: a single deck file reports that deck; a folder
    /// or workspace reports every deck inside it, each against the store it
    /// actually uses. E.g. `alix stats spanish.txt` or `alix stats
    /// ~/decks/flutter`.
    Stats(DeckArgs),
    /// List all cards with their state and due time (deck, folder, or workspace).
    ///
    /// The target is a path: a single deck file lists its cards; a folder or
    /// workspace lists every member deck's, grouped per deck.
    List(DeckArgs),
    /// Clear stored progress for a deck, a folder/workspace, a card, or everything.
    ///
    /// The target is a path: a single deck file clears that deck; a folder
    /// or workspace clears every deck inside it (cards, remediation cards,
    /// and mastered flags) after one confirmation. `--card` narrows to one
    /// card; `--all` wipes the whole store instead of a path.
    Reset(ResetArgs),
    /// Augment or import decks.
    #[command(subcommand)]
    Deck(DeckAction),
    /// Create and grow workspaces.
    #[command(subcommand)]
    Workspace(WorkspaceAction),
    /// Share a deck, folder, or workspace — over magic-wormhole, or as a .zip.
    ///
    /// Either way, what travels is a staged copy without your personal state
    /// (progress, recent list, local pacing). The default sends through the
    /// `wormhole` binary — tell the receiver the code it prints; `--zip`
    /// writes an archive to pass along however you like instead.
    Share(ShareArgs),
    /// Receive a shared deck or folder — by wormhole code, or from a .zip.
    ///
    /// A received deck lands in the decks directory (or `--workspace <dir>`);
    /// a received folder lands beside your other decks under its own name.
    /// Leaked personal files are stripped either way.
    Receive(ReceiveArgs),
    /// Show the configuration (key bindings) or create the config file.
    Config {
        /// Write a config file with the default bindings to edit.
        #[arg(long)]
        init: bool,
    },
}

#[derive(Args)]
struct DoctorArgs {
    /// What to check instead of the configured setup: a decks folder or
    /// workspace root (with its own store, like `alix <dir>` serves it), or a
    /// single deck file to lint in depth.
    dir: Option<PathBuf>,

    /// Also probe the configured AI backend end to end. This sends one real
    /// (tiny) request — the only reliable way to confirm login + reachability.
    #[arg(long)]
    backends: bool,

    /// Probe all four supported backends (one real request each).
    #[arg(long, conflicts_with = "backends")]
    all_backends: bool,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

/// The `alix workspace` subcommands: create and grow workspaces.
#[derive(Subcommand)]
enum WorkspaceAction {
    /// Initialize an empty workspace: a folder with an `alix.toml` and an
    /// `assets/` dir, no decks yet. Grow it with `alix generate … --workspace
    /// <dir>` or `alix deck import … --workspace <dir>`.
    Init(WorkspaceInitArgs),
}

#[derive(Args)]
struct WorkspaceInitArgs {
    /// The folder to create (or to convert, when it exists without an alix.toml).
    dir: PathBuf,

    /// The workspace's display title (default: the folder name).
    #[arg(long)]
    title: Option<String>,
}

#[derive(Args)]
struct ShareArgs {
    /// What to send: a deck file, a plain decks folder, or a workspace.
    path: PathBuf,

    /// Write a .zip archive instead of sending over wormhole — the offline
    /// fallback (mail it, put it on a stick).
    #[arg(long)]
    zip: bool,

    /// With --zip: where to write the archive — a file name, or a directory
    /// to put `<name>.zip` in (default: the current directory).
    #[arg(long, requires = "zip")]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct ReceiveArgs {
    /// A wormhole code the sender read to you (e.g. `7-crossover-clockwork`),
    /// or a path to a `.zip` made by `alix share --zip`.
    #[arg(value_name = "CODE|ZIP")]
    code: String,

    /// Put a received DECK into this workspace instead of the decks directory.
    #[arg(long)]
    workspace: Option<PathBuf>,

    /// Overwrite an existing deck file of the same name (folders never
    /// overwrite — move the old one aside first).
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct GenerateArgs {
    /// What to generate from: a web page URL, a local file, or a directory —
    /// or an existing `% trace:` stub deck, whose checkpoints are then built
    /// in place.
    source: String,

    /// The learning goal that scopes what is generated (default: understand
    /// the whole source).
    #[arg(long)]
    goal: Option<String>,

    /// Print the plan (directory source) or the trace suggestions (--trace)
    /// and stop — generate nothing.
    #[arg(long)]
    plan: bool,

    /// Author a trace over the source instead of facts decks: a short
    /// predict-and-verify walk over its shape, written as a trace deck.
    #[arg(long)]
    trace: bool,

    /// Force a single deck from a directory source (skip the plan pass).
    #[arg(long, conflicts_with = "trace")]
    deck: bool,

    /// The workspace this lands in: the build destination for a directory
    /// source (default: a folder under the decks dir), or the folder a single
    /// generated deck is written into.
    #[arg(long)]
    workspace: Option<PathBuf>,

    /// Single deck: output name (default: derived from the source). A `.txt`
    /// extension is added if missing.
    #[arg(short, long)]
    output: Option<String>,

    /// Single deck: maximum number of cards (overrides the configured default).
    #[arg(long)]
    cards: Option<usize>,

    /// Single deck: run a second AI pass that reviews the draft and removes
    /// redundant cards (an extra call; also `generate.review` in the config).
    #[arg(long)]
    review: bool,

    /// Single deck: print it to stdout instead of writing a file.
    #[arg(long)]
    print: bool,

    /// Overwrite existing output (a deck file, or a non-empty workspace dir).
    #[arg(long)]
    force: bool,

    /// Workspace build: its display title (default: the folder name).
    #[arg(long)]
    title: Option<String>,

    /// Workspace build: use this image as the workspace icon instead of
    /// letting the model draw one. Copied into `assets/`.
    #[arg(long)]
    icon: Option<PathBuf>,

    /// Skip confirmations: the large-source pre-flight, and the
    /// workspace-build go-ahead.
    #[arg(short, long)]
    yes: bool,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct GenerateDeckArgs {
    /// The source to turn into a facts deck: a web page URL, or a local file or
    /// directory path.
    source: String,

    /// Output deck name (default: a slug derived from the URL). Written into
    /// the decks directory; a `.txt` extension is added if missing.
    #[arg(short, long)]
    output: Option<String>,

    /// Maximum number of cards (overrides the configured default).
    #[arg(long)]
    cards: Option<usize>,

    /// Run a second Claude pass that reviews the draft and removes redundant
    /// cards (an extra call; can also be enabled with `generate.review`).
    #[arg(long)]
    review: bool,

    /// Print the generated deck to stdout instead of writing a file.
    #[arg(long)]
    print: bool,

    /// Overwrite the output file if it already exists.
    #[arg(long)]
    force: bool,

    /// Skip the pre-flight size confirmation for a large local source tree.
    #[arg(short, long)]
    yes: bool,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

/// The `alix deck` subcommands: create, augment, or validate a deck.
#[derive(Subcommand)]
enum DeckAction {
    /// Augment an existing deck with Claude — multiple-choice distractors, or
    /// trivia notes. Augmentations are deliberate and persisted, so review stays
    /// instant and fully offline.
    Augment(AugmentArgs),
    /// Import an Anki TSV export into an alix deck.
    ///
    /// Expects tab-separated `front<TAB>back` lines.
    Import(ImportArgs),
}

#[derive(Args)]
struct AugmentArgs {
    /// The deck file to augment.
    deck: PathBuf,

    /// What to augment — mirrors the review concepts: `choices` (distractors),
    /// `notes` (trivia / mnemonics), `questions` (reworded phrasings rotated at
    /// review), or `topology` (a graph of how the cards relate + a suggested
    /// walk; experimental). All are cached beside your progress, never written
    /// into the deck; review reads them.
    #[arg(long, value_enum)]
    target: AugmentTarget,

    /// Free-text guidance for *how* to augment, woven into the prompt (e.g.
    /// "use common misconceptions", "add a surprising historical fact").
    #[arg(long)]
    with: Option<String>,

    /// Path of the progress store the augmentation cache sits beside (default:
    /// platform data dir).
    #[arg(long)]
    store: Option<PathBuf>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

/// What `alix deck augment` generates, named after the review concept it feeds.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum AugmentTarget {
    /// Multiple-choice distractors.
    Choices,
    /// Trivia / mnemonic notes, shown with the card's deck note on reveal.
    Notes,
    /// Reworded question variants, rotated at review time so the card can't be
    /// answered by recognizing one fixed wording. Plain (non-cloze) cards only.
    Questions,
    /// Key points: the load-bearing claims a card's answer makes, so Explain mode
    /// can check a reconstruction against them and derive the grade. Atomic
    /// answers (nothing to decompose) are skipped.
    Keypoints,
    /// A deck-level topology: a graph of how the cards relate plus a suggested
    /// walk, so review can present them in a connected order. Experimental —
    /// prints the walk so you can judge whether it lands. `--with` steers the
    /// organizing principle (e.g. "by module and type dependency").
    Topology,
    /// A display-only reshape of a badly-shaped card — restructured front/answer/
    /// note and a suggested mode — applied at review without touching the deck.
    /// Plain (non-cloze) cards only.
    Format,
}

#[derive(Args)]
struct ImportArgs {
    /// The Anki TSV file to import (tab-separated `front<TAB>back` lines).
    file: PathBuf,

    /// Output deck name (default: a slug from the file name). Written into the
    /// decks directory; a `.txt` extension is added if missing.
    #[arg(short, long)]
    output: Option<String>,

    /// The workspace folder to import the deck into (default: the decks dir).
    #[arg(long)]
    workspace: Option<PathBuf>,

    /// Print the deck to stdout instead of writing a file.
    #[arg(long)]
    print: bool,

    /// Overwrite the output file if it already exists.
    #[arg(long)]
    force: bool,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct DeckArgs {
    /// A path: one deck file (just that deck), or a folder/workspace
    /// (every deck inside it) — e.g. `spanish.txt` or `~/decks/flutter`.
    #[arg(value_name = "DECK|FOLDER|WORKSPACE")]
    target: PathBuf,

    /// Path of the progress store (default: resolved from the target).
    #[arg(long)]
    store: Option<PathBuf>,
}

#[derive(Args)]
struct ResetArgs {
    /// What to clear, as a path: one deck file, or a folder/workspace
    /// (every deck inside it) — e.g. `spanish.txt` or `~/decks/flutter`.
    #[arg(value_name = "DECK|FOLDER|WORKSPACE")]
    target: Option<PathBuf>,

    /// Reset one card: its numeric id, or text matching its front (searched
    /// within the target's decks).
    #[arg(long)]
    card: Option<String>,

    /// Clear progress for every card in the store.
    #[arg(long, conflicts_with_all = ["target", "card"])]
    all: bool,

    /// Skip the confirmation prompt (for scripts / test loops).
    #[arg(short = 'y', long)]
    yes: bool,

    /// Path of the progress store (default: platform data dir).
    #[arg(long)]
    store: Option<PathBuf>,
}

fn main() -> Result<()> {
    // One-time: adopt a pre-rename `flash` data dir so existing progress survives.
    alix::store::migrate_legacy_data_dir();
    let cli = Cli::parse();
    match cli.command {
        None => launch(cli.launch),
        Some(Command::Stats(args)) => stats(args),
        Some(Command::List(args)) => list(args),
        Some(Command::Reset(args)) => reset(args),
        Some(Command::Generate(args)) => generate_cmd(args),
        Some(Command::Deck(action)) => match action {
            DeckAction::Augment(args) => augment_cmd(args),
            DeckAction::Import(args) => import_cmd(args),
        },
        Some(Command::Workspace(action)) => match action {
            WorkspaceAction::Init(args) => workspace_init_cmd(args),
        },
        Some(Command::Share(args)) => share_cmd(args),
        Some(Command::Receive(args)) => receive_cmd(args),
        Some(Command::Config { init }) => config_cmd(init),
        Some(Command::Doctor(args)) => doctor_cmd(args),
    }
}

/// The `list` label for one depth's schedule: the FSRS state name when the
/// card has a schedule at that depth, `-` when it has none.
fn state_label(fsrs_state: Option<u8>) -> &'static str {
    match fsrs_state {
        Some(1) => "learning",
        Some(2) => "review",
        Some(3) => "relearning",
        Some(_) => "new",
        None => "-",
    }
}

fn stats(args: DeckArgs) -> Result<()> {
    let config = Config::load(None)?;
    let now = now_ms();

    let target = expand_target(&args.target)?;
    for path in &target.decks {
        // Each deck reads its own store — a workspace deck's progress lives in the
        // workspace, not the global store.
        let store = target.store_for_deck(path, args.store.as_deref())?;
        let deck = Deck::load(path)?;
        // …and its own pacing: a workspace deck honors its `alix.local.toml`.
        let review = config
            .review
            .for_workspace(path.parent().unwrap_or_else(|| Path::new("")));
        let scheduler = Fsrs::new(review.retention);

        let mut due_now = 0usize;
        let mut due_24h = 0usize;
        let mut due_now_reconstruct = 0usize;
        let mut reviews = 0u32;
        let mut passes = 0u32;
        for card in &deck.cards {
            if let Some(state) = store.get(card.id()) {
                // Retired cards are resting, so they don't count as due (they
                // still count toward the review totals below).
                if !alix::session::is_retired(card, &store, review.retire_after_days) {
                    let due = scheduler.due_at(state, Depth::Recall);
                    if due <= now {
                        due_now += 1;
                    } else if due <= now + 86_400_000 {
                        due_24h += 1;
                    }
                    if scheduler.is_due(state, Depth::Reconstruct, now) {
                        due_now_reconstruct += 1;
                    }
                }
                reviews += state.total_reviews;
                passes += state.total_passes;
            }
        }
        // Virtual (remediation) cards count toward "due" (now and within
        // 24h), never toward the card count below — they aren't deck content.
        due_now += alix::session::count_reviewable_virtual(
            &store,
            &deck.subject,
            &scheduler,
            now,
            review.retire_after_days,
        );
        // Virtual cards are Recall-only, so the recall figure IS the due-now
        // aggregate — derived, not re-counted, so the two lines can't diverge.
        let due_now_recall = due_now;
        due_24h += alix::session::count_due_soon_virtual(
            &store,
            &deck.subject,
            &scheduler,
            now,
            86_400_000,
            review.retire_after_days,
        );

        let state = match deck.state(&store) {
            DeckState::NotStarted => "not started",
            DeckState::Started => "in progress",
            DeckState::ExamDue => "exam due",
            DeckState::Finished if store.deck_mastered(&deck.subject) => "mastered ✓",
            DeckState::Finished => "finished ✓",
        };
        println!("{} ({} cards)", deck.display_name(), deck.cards.len());
        println!("  state:   {state}");
        println!("  due:     {due_now} now, {due_24h} within 24h");
        println!("  due now (recall):      {due_now_recall}");
        println!("  due now (reconstruct): {due_now_reconstruct}");
        if reviews > 0 {
            println!(
                "  reviews: {reviews} total, {:.0}% passed",
                100.0 * passes as f64 / reviews as f64
            );
        }
    }
    Ok(())
}

fn list(args: DeckArgs) -> Result<()> {
    let config = Config::load(None)?;
    let now = now_ms();

    let target = expand_target(&args.target)?;
    for path in &target.decks {
        // Each deck reads its own store (workspace store for a workspace deck).
        let store = target.store_for_deck(path, args.store.as_deref())?;
        let deck = Deck::load(path)?;
        // …and its own pacing (workspace `alix.local.toml` override).
        let review = config
            .review
            .for_workspace(path.parent().unwrap_or_else(|| Path::new("")));
        let scheduler = Fsrs::new(review.retention);
        println!("{}", deck.display_name());
        for card in &deck.cards {
            let (recall_label, recon_label, recognized_mark, due) = match store.get(card.id()) {
                Some(state) => {
                    // Retired cards rest until `alix reset`; their due time is
                    // moot, so say so instead of showing a misleading interval.
                    let due = if alix::session::is_retired(card, &store, review.retire_after_days) {
                        "resting".to_string()
                    } else {
                        let due = scheduler.due_at(state, Depth::Recall);
                        if due <= now {
                            "due now".to_string()
                        } else {
                            format!("due in {}", humanize_ms(due - now))
                        }
                    };
                    let recall_label = state_label(state.recall.as_ref().map(|f| f.state));
                    let recon_label = state_label(state.reconstruct.as_ref().map(|f| f.state));
                    let recognized_mark = if state.recognized_ms.is_some() {
                        "✓"
                    } else {
                        " "
                    };
                    (recall_label, recon_label, recognized_mark, due)
                }
                None => (state_label(None), state_label(None), " ", "-".to_string()),
            };
            let front: String = card.front.chars().take(60).collect();
            println!("  [{recall_label:>10}|{recon_label:>10}]{recognized_mark} {front:<60} {due}");
        }
    }
    Ok(())
}

/// `(id, front)` pairs to reset from `cards`: all of them when `card` is
/// `None`, otherwise only the matches. A numeric `card` matches by
/// `Card::id()`; any other text matches cards whose front contains it
/// (case-insensitive) — a cloze card's holes share a front, so that resets the
/// whole card.
fn select_reset_ids(cards: &[Card], card: Option<&str>) -> Vec<(u64, String)> {
    let by_id = card.and_then(|c| c.parse::<u64>().ok());
    let needle = card.map(str::to_lowercase);
    cards
        .iter()
        .filter(|c| match (by_id, &needle) {
            (Some(id), _) => c.id() == id,
            (None, Some(text)) => c.front.to_lowercase().contains(text),
            (None, None) => true,
        })
        .map(|c| (c.id(), c.front.clone()))
        .collect()
}

fn reset(args: ResetArgs) -> Result<()> {
    // `--all` / a numeric `--card` operate on the global store (or `--store`);
    // a deck-scoped reset re-resolves to the deck's workspace store below.
    let mut store = open_store(args.store.clone())?;

    // `--all`: wipe everything; no decks needed, count up front for the prompt.
    // `store.len()` now counts virtual schedules too (they live in `store.cards`),
    // so a store holding only virtual cards still reports something to reset.
    if args.all {
        let n = store.len();
        if n == 0 {
            println!("No stored progress to reset.");
            return Ok(());
        }
        if !confirm(&format!("Reset progress for all {n} card(s)?"), args.yes)? {
            println!("Cancelled.");
            return Ok(());
        }
        store.clear();
        store.save()?;
        println!("Reset {n} card(s).");
        return Ok(());
    }

    // A numeric `--card` with no target can be removed without loading anything.
    let numeric_id = args.card.as_deref().and_then(|c| c.parse::<u64>().ok());
    if let Some(id) = numeric_id.filter(|_| args.target.is_none()) {
        return reset_ids(
            &mut store,
            vec![(id, String::new())],
            format!("card {id}"),
            args.card.as_deref(),
            false,
            args.yes,
        );
    }

    // Otherwise a reset needs an explicit target — there is no interactive
    // deck picker. Name a deck/folder/workspace (optionally with `--card`),
    // or pass `--all`.
    let Some(target_path) = &args.target else {
        bail!("name a deck, folder, or workspace to reset, or pass `--card <id>` or `--all`");
    };
    let target = expand_target(target_path)?;
    let deck_paths = target.decks.clone();

    // Reset against the target's store: `--store` > the members' shared
    // workspace store > a scoped folder's own store > the global default —
    // the launcher's rule, so the reset hits the progress that serving uses.
    let mut store = open_store(
        args.store
            .clone()
            .or_else(|| store_path_for(&deck_paths, None))
            .or_else(|| target.default_store.clone()),
    )?;

    let (cards, label, _, _) = load_decks(&deck_paths, &HashMap::new())?;

    // A full-deck reset (no `--card` subset) resets authored-card progress, the
    // decks' "mastered" exam flag, and their virtual (remediation) cards
    // together, atomically under one confirmation — a declined/failed prompt
    // must leave the store on disk untouched by any of it (not just the
    // authored-card part).
    if args.card.is_none() {
        // Load the decks once, up front, and use that same load for both the
        // confirm-prompt count and the wipe below — counting from one load and
        // wiping from a later, separate one let a deck edited on disk while the
        // prompt waits silently diverge (a renamed back line changes
        // `Card::id()`, orphaning the old schedule).
        let decks_full: Vec<Deck> = deck_paths
            .iter()
            .map(Deck::load)
            .collect::<Result<Vec<_>, _>>()?;

        let present: Vec<(u64, String)> = decks_full
            .iter()
            .flat_map(|deck| &deck.cards)
            .filter(|c| store.get(c.id()).is_some())
            .map(|c| (c.id(), c.front.clone()))
            .collect();
        let mastered = decks_full
            .iter()
            .any(|deck| store.deck_mastered(&deck.subject));
        // A virtual card's content is in the sidecar and its schedule in
        // `store.cards` (both keyed by the same id) — a reset drops both.
        let virtual_ids: Vec<u64> = decks_full
            .iter()
            .flat_map(|deck| store.virtual_cards_for(&deck.subject))
            .map(|vc| vc.id)
            .collect();

        if present.is_empty() && !mastered && virtual_ids.is_empty() {
            println!("No stored progress to reset in {label}.");
            return Ok(());
        }

        let n = present.len();
        if !confirm(
            &format!("Reset progress for {n} card(s) in {label}?"),
            args.yes,
        )? {
            println!("Cancelled.");
            return Ok(());
        }

        let wiped = alix::library::reset_decks(&mut store, decks_full.iter())?;
        println!("Reset {wiped} card(s).");
        return Ok(());
    }

    // A `--card` subset over the named decks: match by numeric id or front text
    // (a full-deck reset is handled above).
    let targets = select_reset_ids(&cards, args.card.as_deref());
    reset_ids(
        &mut store,
        targets,
        label,
        args.card.as_deref(),
        false,
        args.yes,
    )
}

/// Removes the `(id, front)` targets that have stored progress, after a `y/N`
/// confirmation — unless `from_picker` (the picker's Enter already confirmed)
/// or `yes`. Saves and reports the count.
fn reset_ids(
    store: &mut Store,
    targets: Vec<(u64, String)>,
    scope: String,
    card_query: Option<&str>,
    from_picker: bool,
    yes: bool,
) -> Result<()> {
    let present: Vec<(u64, String)> = targets
        .into_iter()
        .filter(|(id, _)| store.get(*id).is_some())
        .collect();
    if present.is_empty() {
        match card_query {
            Some(query) => println!("No stored progress matching {query:?}."),
            None => println!("No stored progress to reset in {scope}."),
        }
        return Ok(());
    }

    let n = present.len();
    if !from_picker {
        let what = if card_query.is_some() {
            let fronts: Vec<String> = present
                .iter()
                .map(|(_, f)| f.chars().take(60).collect())
                .filter(|f: &String| !f.is_empty())
                .collect();
            if fronts.is_empty() {
                scope
            } else {
                fronts.join("; ")
            }
        } else {
            format!("{n} card(s) in {scope}")
        };
        if !confirm(&format!("Reset progress for {what}?"), yes)? {
            println!("Cancelled.");
            return Ok(());
        }
    }

    for (id, _) in &present {
        store.remove(*id);
    }
    store.save()?;
    println!("Reset {n} card(s).");
    Ok(())
}

/// `alix deck augment`: deliberately generate AI augmentations for a deck into
/// the sidecar cache (`augment.json`), which review then reads. Foreground, so
/// any Claude error surfaces here rather than mid-review.
fn augment_cmd(args: AugmentArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let deck = Deck::load(&args.deck)?;
    let ask_cfg = augment::run_config(&config.ai, &config.ask);
    let guidance = args.with.as_deref();

    // The cache sits beside whatever store the deck reviews against, so a
    // workspace deck's augmentations live with the workspace.
    let store = store_for(std::slice::from_ref(&args.deck), args.store.clone())?;
    let cache_path = augment::augment_path_for(store.path());
    let mut cache = AugmentCache::open(&cache_path);

    // The Claude call below is one batched, foreground request that can take a
    // while, so say what's happening rather than hang silently.
    let what = match args.target {
        AugmentTarget::Choices => "multiple-choice distractors",
        AugmentTarget::Notes => "trivia / mnemonic notes",
        AugmentTarget::Questions => "reworded question variants",
        AugmentTarget::Keypoints => "answer key points",
        AugmentTarget::Topology => "a topology",
        AugmentTarget::Format => "card formatting",
    };
    let model = config
        .ai
        .model
        .as_deref()
        .or(config.ask.model.as_deref())
        .unwrap_or("the default model");
    eprintln!(
        "Generating {what} for \"{}\" with Claude ({model}) — one batched call, \
         this can take a moment…",
        deck.subject
    );

    let (made, total, kind) = match args.target {
        AugmentTarget::Choices => {
            let items = warm_items(&deck.cards);
            if items.is_empty() {
                bail!("the deck has no cards to augment");
            }
            let total = items.len();
            let map = augment::generate(&items, config.ai.distractor_count, guidance, &ask_cfg)?;
            for (id, distractors) in &map {
                cache.set_distractors(*id, distractors.clone());
            }
            (map.len(), total, "distractors")
        }
        AugmentTarget::Notes => {
            let items = warm_items(&deck.cards);
            if items.is_empty() {
                bail!("the deck has no cards to augment");
            }
            let total = items.len();
            let map = augment::generate_notes(&items, guidance, &ask_cfg)?;
            for (id, note) in &map {
                cache.set_note(*id, note.clone());
            }
            (map.len(), total, "notes")
        }
        AugmentTarget::Questions => {
            // Morphing the front only makes sense for plain cards — a cloze
            // card's front is its title, with the fill-in-the-blank in the body.
            let plain: Vec<Card> = deck
                .cards
                .iter()
                .filter(|c| c.hash_lines.is_none())
                .cloned()
                .collect();
            let items = warm_items(&plain);
            if items.is_empty() {
                bail!("the deck has no plain (non-cloze) cards to add question variants to");
            }
            let total = items.len();
            let map =
                augment::generate_variants(&items, config.ai.variant_count, guidance, &ask_cfg)?;
            for (id, variants) in &map {
                cache.set_variants(*id, variants.clone());
            }
            (map.len(), total, "question variants")
        }
        AugmentTarget::Keypoints => {
            let items = warm_items(&deck.cards);
            if items.is_empty() {
                bail!("the deck has no cards to break into key points");
            }
            let total = items.len();
            let map =
                augment::generate_keypoints(&items, config.ai.keypoint_count, guidance, &ask_cfg)?;
            for (id, keypoints) in &map {
                cache.set_keypoints(*id, keypoints.clone());
            }
            (map.len(), total, "key points")
        }
        AugmentTarget::Topology => {
            let items = warm_items(&deck.cards);
            if items.is_empty() {
                bail!("the deck has no cards to build a topology over");
            }
            let total = items.len();
            let topo = augment::generate_topology(&items, guidance, &ask_cfg)?;
            print_topology(&topo, &deck.cards);
            let walked = topo.walk.len();
            cache.add_topology(topo);
            // Count only this deck's topologies — the cache may be shared with
            // other decks that share a store.
            let deck_ids: std::collections::HashSet<u64> =
                deck.cards.iter().map(|c| c.id()).collect();
            let n = cache.topologies_for(&deck_ids).len();
            println!(
                "({n} topolog{} stored for this deck)",
                if n == 1 { "y" } else { "ies" }
            );
            (walked, total, "a topology")
        }
        AugmentTarget::Format => {
            // Reshaping is for plain cards — a cloze card's masked body must not
            // be restructured. Include this deck's synthesized virtual
            // (remediation) cards alongside its authored ones: `set_format` keys
            // by the synth card's real `Card::id`, so the cached entry is exactly
            // what `apply_format` finds at review time (§8.2).
            //
            // Mirror `build_review`'s injection filters: a partial cloze promote
            // (see `store::promote_virtual`) can leave an orphaned sidecar entry
            // whose id collides with a real deck card, and a retired card is
            // resting — neither should be warmed a second time or at all.
            let subject: Arc<str> = Arc::from(deck.subject.as_str());
            let deck_ids: std::collections::HashSet<u64> =
                deck.cards.iter().map(Card::id).collect();
            let retire_after_days = config
                .review
                .for_workspace(deck.path.parent().unwrap_or_else(|| Path::new("")))
                .retire_after_days;
            let mut plain: Vec<Card> = deck
                .cards
                .iter()
                .filter(|c| c.hash_lines.is_none())
                .cloned()
                .collect();
            for (k, vc) in store
                .virtual_cards_for(&deck.subject)
                .into_iter()
                .filter(|v| !deck_ids.contains(&v.id))
                .filter(|v| !alix::session::is_retired_id(v.id, &store, retire_after_days))
                .enumerate()
            {
                if let Some(card) = synthesize_virtual(vc, &subject, VIRTUAL_LINE_BASE + k)
                    && card.hash_lines.is_none()
                {
                    plain.push(card);
                }
            }
            let items = warm_items(&plain);
            if items.is_empty() {
                bail!("the deck has no plain (non-cloze) cards to format");
            }
            let total = items.len();
            let map = augment::generate_format(&items, guidance, &ask_cfg)?;
            for (id, fmt) in &map {
                cache.set_format(*id, fmt.clone());
            }
            (map.len(), total, "card formats")
        }
    };
    cache.save()?;

    println!(
        "augmented {made} of {total} cards with {kind} → {}",
        cache_path.display()
    );
    Ok(())
}

/// Builds the per-card generation input from `cards`.
fn warm_items(cards: &[Card]) -> Vec<augment::WarmItem> {
    cards.iter().map(augment::WarmItem::from_card).collect()
}

/// Prints a generated topology as its suggested walk — each card with the reason
/// it follows the previous one — so a person can judge whether the order reads as
/// "good follow-up" rather than random. The eyeball test for the topology probe.
fn print_topology(topo: &augment::Topology, cards: &[Card]) {
    let fronts: std::collections::HashMap<u64, String> = cards
        .iter()
        .map(|c| (c.id(), truncate(&one_line(&c.front), 72)))
        .collect();
    let unknown = "<card not in deck>".to_string();

    println!(
        "\ntopology '{}': {}\n({} cards walked, {} edges)\n",
        topo.name,
        topo.principle,
        topo.walk.len(),
        topo.edges.len()
    );
    let mut prev: Option<u64> = None;
    for (i, id) in topo.walk.iter().enumerate() {
        let front = fronts.get(id).unwrap_or(&unknown);
        match prev {
            None => println!("{:>3}. {front}", i + 1),
            Some(p) => {
                let why = topo
                    .edges
                    .iter()
                    .find(|e| e.from == p && e.to == *id)
                    .map(|e| e.label.as_str())
                    .unwrap_or("—");
                println!("{:>3}. ↳ [{why}]  {front}", i + 1);
            }
        }
        prev = Some(*id);
    }
    println!();
}

/// `alix generate`: one entry for all AI authoring. Routes by what the source
/// is — an existing `% trace:` stub builds in place; `--trace` authors a trace
/// over a source; a directory is explored first and the plan's size decides
/// deck vs workspace (confirmed before the expensive build); anything else
/// becomes a single deck.
fn generate_cmd(args: GenerateArgs) -> Result<()> {
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
        None => match alix::icon::generate(&dir, &config.ask) {
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

/// `alix share`: stage a personal-state-free copy and hand it to wormhole.
/// The wormhole binary prints the code mnemonic and the progress itself.
fn share_cmd(args: ShareArgs) -> Result<()> {
    let path = &args.path;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shared-decks")
        .to_string();

    // A single deck has no personal state and travels as-is (its augmentations
    // live in a shared per-store cache and stay home). A folder is staged
    // first, so progress and personal config never leave.
    let tmp = tempfile::tempdir().context("cannot create a staging directory")?;
    let (to_send, staged) = if path.is_file() {
        (path.clone(), 1)
    } else {
        if !path.is_dir() {
            bail!("`{}` is neither a deck file nor a folder", path.display());
        }
        if !workspace::has_decks(path) {
            bail!("no decks in `{}` — nothing to share", path.display());
        }
        let stage = tmp.path().join(&name);
        let staged = alix::share::stage_dir(path, &stage)?;
        (stage, staged)
    };

    // `--zip`: the offline fallback — write an archive instead of sending.
    if args.zip {
        let stem = name.strip_suffix(".txt").unwrap_or(&name);
        let out = match &args.output {
            Some(p) if p.is_dir() => p.join(format!("{stem}.zip")),
            Some(p) => p.clone(),
            None => PathBuf::from(format!("{stem}.zip")),
        };
        let entries = alix::share::zip_to(&to_send, &out)?;
        println!(
            "Wrote {} ({entries} files — progress and personal config stay home).",
            out.display()
        );
        return Ok(());
    }

    println!(
        "Sharing {name} ({staged} files — progress and personal config stay home). \
         Tell the receiver the code below."
    );
    alix::share::wormhole(&["send", &to_send.to_string_lossy()], None)
}

/// `alix receive`: run wormhole in a scratch dir, strip any leaked personal
/// files, and move the result where it belongs.
fn receive_cmd(args: ReceiveArgs) -> Result<()> {
    let config = Config::load(None)?;
    let tmp = tempfile::tempdir().context("cannot create a receiving directory")?;
    // A `.zip` path skips the wormhole entirely — same staging, same landing.
    let zip_path = Path::new(&args.code);
    if args.code.ends_with(".zip") && zip_path.is_file() {
        alix::share::unzip_to(zip_path, tmp.path())?;
    } else {
        alix::share::wormhole(&["receive", "--accept-file", &args.code], Some(tmp.path()))?;
    }

    // Whatever arrived is the single new entry in the scratch dir.
    let mut entries: Vec<PathBuf> = std::fs::read_dir(tmp.path())?
        .flatten()
        .map(|e| e.path())
        .collect();
    let Some(got) = entries.pop().filter(|_| entries.is_empty()) else {
        bail!("expected exactly one received file or folder");
    };
    let name = got
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("received")
        .to_string();

    if got.is_dir() {
        if args.workspace.is_some() {
            bail!(
                "--workspace places a received deck; a folder lands under the decks dir as `{name}`"
            );
        }
        let removed = alix::share::sanitize_received(&got)?;
        for r in &removed {
            println!("stripped a leaked personal file: {r}");
        }
        let dest = config
            .decks_dir()
            .context("cannot determine the decks directory")?
            .join(&name);
        if dest.exists() {
            bail!(
                "{} already exists — move it aside first (folders are never overwritten)",
                dest.display()
            );
        }
        alix::share::move_into(&got, &dest)?;
        println!(
            "Received {} — open it:  alix {}",
            dest.display(),
            dest.display()
        );
    } else {
        let dest_dir = deck_out_dir(args.workspace.as_deref(), &config)?;
        std::fs::create_dir_all(&dest_dir)
            .with_context(|| format!("cannot create {}", dest_dir.display()))?;
        let dest = dest_dir.join(&name);
        if dest.exists() && !args.force {
            bail!(
                "{} already exists; pass --force to overwrite",
                dest.display()
            );
        }
        alix::share::move_into(&got, &dest)?;
        println!(
            "Received {} — it shows up in the picker (`alix`).",
            dest.display()
        );
    }
    Ok(())
}

/// `alix workspace init`: an empty workspace — `alix.toml` + `assets/`, no
/// decks. Grow it with `alix generate/deck import … --workspace <dir>`.
fn workspace_init_cmd(args: WorkspaceInitArgs) -> Result<()> {
    if workspace::is_workspace(&args.dir) {
        bail!("{} is already a workspace", args.dir.display());
    }
    std::fs::create_dir_all(args.dir.join("assets"))
        .with_context(|| format!("cannot create {}", args.dir.display()))?;
    let title = match &args.title {
        Some(t) => t.clone(),
        None => args
            .dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
            .to_string(),
    };
    // Both files are written fully commented (except what must be set), so
    // they document their own keys — the section headers stay UNcommented
    // because both parse leniently: a key uncommented outside its table would
    // be silently ignored.
    let manifest = format!(
        "# This workspace's shared manifest — it travels when the folder is shared.\n\
         \n\
         title = {title:?}\n\
         \n\
         # description = \"one line shown under the title in the picker\"\n\
         # icon = \"assets/icon.svg\"     # picker emblem (svg/png/jpg/webp); default: assets/icon.*\n\
         \n\
         # Deck directives every member deck inherits (a deck's own line wins):\n\
         \n\
         [defaults]\n\
         \n\
         # reveal = \"flip\"              # flip | cloze | line\n\
         # order = \"scheduled\"          # scheduled | sequential\n"
    );
    std::fs::write(args.dir.join("alix.toml"), manifest)
        .with_context(|| format!("cannot write {}/alix.toml", args.dir.display()))?;
    let local = "# Personal pacing for THIS workspace — never shared (`alix share` leaves it\n\
         # home). Uncomment a key to override your global [review] config here.\n\
         \n\
         [review]\n\
         \n\
         # retention = 0.9              # FSRS target recall probability (0.70–0.99)\n\
         # retire_after = \"1y\"          # a card rests at this interval (\"never\" disables)\n\
         # max_new = 10                 # max never-seen cards a session introduces\n\
         # limit = 40                   # cap on total cards per session\n";
    std::fs::write(args.dir.join(config::LOCAL_MANIFEST), local)
        .with_context(|| format!("cannot write {}/alix.local.toml", args.dir.display()))?;
    println!(
        "Initialized {} — alix.toml (shared manifest) and alix.local.toml (your\n\
         personal pacing, never shared) document their keys inline. Add decks:\n\
         alix generate <source> --workspace {}   or   alix deck import <file.tsv> --workspace {}",
        args.dir.display(),
        args.dir.display(),
        args.dir.display(),
    );
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

fn import_cmd(args: ImportArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let tsv = std::fs::read_to_string(&args.file)
        .with_context(|| format!("cannot read {}", args.file.display()))?;
    let text = import::tsv_to_deck(&tsv)?;

    // The file name is part of every card's identity hash, so parse against the
    // final name.
    let name = match &args.output {
        Some(name) => name.clone(),
        None => generate::deck_name(&args.file.to_string_lossy()),
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

    let dir = deck_out_dir(args.workspace.as_deref(), &config)?;
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
            println!(
                "Imported {} cards into {}",
                placed.cards,
                placed.path.display()
            );
            Ok(())
        }
        // Saved, but not yet valid: tell the user exactly what to fix.
        Some(e) => bail!(
            "Saved the deck to {}, but it does not parse yet:\n  {e}\n\
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
    let cards = alix::trace::build(deck, &config.trace, &config.ask)?;
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
    let menu = alix::trace::suggest(source, &config.trace, &config.ask)?;
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

/// The canonical CLI name of a value-enum value (e.g. `Mode::LineByLine` →
/// `"line"`), for echoing a deck's declared settings.
fn val_name<T: clap::ValueEnum>(value: T) -> String {
    value
        .to_possible_value()
        .map(|p| p.get_name().to_string())
        .unwrap_or_default()
}

fn check(decks: Vec<PathBuf>) -> Result<()> {
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for path in &decks {
        // A workspace directory: validate its declared icon, then skip the
        // deck-load (which would error on a directory).
        if path.is_dir() && alix::workspace::is_workspace(path) {
            if let Some(rel) = alix::workspace::manifest_icon(path)
                && !path.join(&rel).is_file()
            {
                warnings += 1;
                eprintln!(
                    "warning: {}: `icon = \"{rel}\"` points at a missing file",
                    path.display()
                );
            }
            continue;
        }
        match Deck::load(path) {
            Err(e) => {
                errors += 1;
                eprintln!("error: {e}");
            }
            Ok(deck) => {
                println!("{}: {} cards", deck.subject, deck.cards.len());
                let s = &deck.settings;
                let declared: Vec<String> = [
                    s.reveal.map(|r| format!("reveal: {}", val_name(r))),
                    s.order.map(|o| format!("order: {}", val_name(o))),
                    s.exam_strictness
                        .map(|v| format!("strictness: {}", val_name(v))),
                ]
                .into_iter()
                .flatten()
                .collect();
                if !declared.is_empty() {
                    println!("  settings: {}", declared.join(", "));
                }
                if !deck.requires.is_empty() {
                    println!("  requires: {}", deck.requires.join(", "));
                }
                if !deck.sources.is_empty() {
                    println!("  sources:  {}", deck.sources.join(", "));
                }
                if let Some(desc) = &deck.trace {
                    println!("  trace:    {desc}");
                }
                for (a, b) in deck.duplicates() {
                    warnings += 1;
                    eprintln!(
                        "warning: {}: cards at lines {} and {} have identical answers \
                         and share their learning progress",
                        deck.subject, a.line, b.line
                    );
                }
                // Image paths are resolved but never checked at load time, so a
                // missing file is reported here (advisory: the deck still works,
                // the web server just 404s the image).
                for card in &deck.cards {
                    for image in [&card.image, &card.image_back].into_iter().flatten() {
                        if !image.exists() {
                            warnings += 1;
                            eprintln!(
                                "warning: {}: card at line {} references a missing image: {}",
                                deck.subject,
                                card.line,
                                image.display()
                            );
                        }
                    }
                }

                // Frozen decks: warn when a card's snapshot no longer matches the
                // live source (the file changed or is gone), so the learner can
                // update or drop that card.
                for drift in alix::trace::drifted_cards(&deck) {
                    warnings += 1;
                    let what = if drift.gone {
                        "source file is gone"
                    } else {
                        "no longer found in the source"
                    };
                    eprintln!(
                        "warning: {}: card at line {} — frozen excerpt {} ({})",
                        deck.subject, drift.line, what, drift.at
                    );
                }

                // Trace decks: validate each `% at:` locator resolves into the
                // live `% source:` — catches drift (a file that shrank or was
                // renamed) before a walk hits it, like the duplicate/image checks.
                if deck.is_trace() && !deck.cards.is_empty() {
                    match Trace::from_deck(&deck) {
                        Ok(trace) => {
                            for issue in trace.lint_locators() {
                                warnings += 1;
                                let line = deck.cards.get(issue.checkpoint).map_or(0, |c| c.line);
                                eprintln!(
                                    "warning: {}: checkpoint at line {}: {}",
                                    deck.subject, line, issue.message
                                );
                            }
                        }
                        Err(e) => {
                            warnings += 1;
                            eprintln!("warning: {}: {e:#}", deck.subject);
                        }
                    }
                }

                // Fact decks: a card may also cite its source with `% at:`; warn
                // when a citation doesn't resolve (a moved/shrunk file), so a
                // hand-written or generated citation is caught before review.
                if !deck.is_trace() {
                    let base = SourceBase::for_deck(&deck);
                    for card in &deck.cards {
                        if let Some(at) = card.at.as_deref()
                            && let Err(e) = base.excerpt(at)
                        {
                            warnings += 1;
                            eprintln!(
                                "warning: {}: card at line {}: `% at: {at}` — {e:#}",
                                deck.subject, card.line
                            );
                        }
                    }
                }

                // A `% requires:` to a source-less deck never gates this deck's exam
                // (`is_locked` sees through an exam-less prerequisite), so a sourced
                // deck listing one likely meant it to gate — flag the dead edge.
                for prereq in alix::deck::nongating_prerequisites(&deck) {
                    warnings += 1;
                    eprintln!(
                        "warning: {}: requires source-less `{prereq}` — this edge \
                         doesn't gate its exam; add a `% source:` to `{prereq}` to \
                         make it a real prerequisite",
                        deck.subject
                    );
                }
            }
        }
    }
    // Warnings (e.g. duplicate answers) are advisory and don't fail the check;
    // only a deck that won't parse is an error.
    if errors > 0 || warnings > 0 {
        eprintln!("{errors} error(s), {warnings} warning(s)");
    }
    if errors > 0 {
        bail!("{errors} error(s) found");
    }
    Ok(())
}

/// Runs the health checks and prints the report: `✓` ok, `!` warn (an optional
/// feature is limited), `✗` fail (the core loop is broken). Exits non-zero only
/// on a fail, so a missing optional binary never breaks a script.
fn doctor_cmd(args: DoctorArgs) -> Result<()> {
    use alix::doctor::{self, Status};
    // A deck-file target = lint exactly that deck (syntax, duplicate answers,
    // trace locators) — the old `deck check`, now one more thing doctor checks.
    if let Some(path) = &args.dir
        && path.is_file()
    {
        return check(vec![path.clone()]);
    }
    let (config_finding, config) = doctor::check_config(args.config.as_deref());
    let mut findings = vec![config_finding];
    // The same root/store resolution the launcher applies to `alix <dir>`.
    let (decks_dir, store_path) = match &args.dir {
        Some(path) => {
            let store = if workspace::is_workspace(path) {
                workspace::store_path(path)
            } else {
                path.join(workspace::STORE_FILE)
            };
            (path.clone(), Some(store))
        }
        None => (
            config.decks_dir().context("cannot determine ~/decks")?,
            None,
        ),
    };
    findings.push(doctor::check_store(store_path));
    findings.push(doctor::check_decks(&decks_dir));
    findings.push(doctor::check_binary(
        "backend",
        &config.ask.command,
        "the AI features (tutor, exam, generate)",
        "install it and log in — or switch `[ask] backend` in the config",
    ));
    findings.push(doctor::check_binary(
        "share",
        "wormhole",
        "sharing (`alix share`/`receive`)",
        "install magic-wormhole (e.g. `pipx install magic-wormhole`, or your package manager)",
    ));
    let mut failed = false;
    for f in &findings {
        let glyph = match f.status {
            Status::Ok => "✓",
            Status::Warn => "!",
            Status::Fail => {
                failed = true;
                "✗"
            }
        };
        println!("{glyph} {:<8} {}", f.name, f.detail);
        if let Some(remedy) = &f.remedy {
            println!("  ↳ {remedy}");
        }
    }
    // The costed end-to-end probe is opt-in: one real (tiny) request to the
    // configured backend, or one per backend with --all-backends.
    if args.backends || args.all_backends {
        println!();
        alix::backend::health::check(&config.ask, args.all_backends)?;
    }
    if failed {
        bail!("doctor found problems (✗ above)");
    }
    Ok(())
}

fn config_cmd(init: bool) -> Result<()> {
    let path = config::default_config_path().context("cannot determine the config directory")?;

    if init {
        if path.exists() {
            bail!("{} already exists; edit it directly", path.display());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        std::fs::write(&path, config::default_config_toml())
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {}", path.display());
        return Ok(());
    }

    if path.exists() {
        println!("config file: {}", path.display());
    } else {
        println!(
            "no config file at {} — using defaults; create one with \
             `alix config --init`",
            path.display()
        );
    }
    // Loading validates the file (or yields the defaults if there is none).
    let config = Config::load(None)?;
    let keys = &config.keys;
    let show = |action: &str, list: &[config::KeyPattern]| {
        let keys: Vec<String> = list.iter().map(|p| p.to_string()).collect();
        println!("  {action:<9} {}", keys.join(", "));
    };
    println!("key bindings:");
    show("failed", &keys.failed);
    show("partly", &keys.partly);
    show("passed", &keys.passed);
    show("reveal", &keys.reveal);
    show("hint", &keys.hint);
    show("submit", &keys.submit);
    show("skip", &keys.skip);
    show("remove", &keys.remove);
    show("continue", &keys.cont);
    show("restart", &keys.restart);
    show("ask", &keys.ask);
    show("save_note", &keys.save_note);
    show("quit", &keys.quit);
    println!("browse bindings (first/last fixed: g/G/Home/End):");
    show("next", &config.browse.next);
    show("prev", &config.browse.prev);
    show("remove", &config.browse.remove);
    show("quit", &config.browse.quit);
    println!("ask:");
    println!("  command     {}", config.ask.command);
    println!(
        "  model       {}",
        config.ask.model.as_deref().unwrap_or("(CLI default)")
    );
    println!("  timeout     {}s", config.ask.timeout_secs);
    println!("  permission  {}", config.ask.permission_mode);
    println!("  tools       {}", config.ask.allowed_tools.join(", "));
    println!("generate:");
    println!(
        "  model       {}",
        config
            .generate
            .model
            .as_deref()
            .unwrap_or("(ask / CLI default)")
    );
    println!("  timeout     {}s", config.generate.timeout_secs);
    println!("  max_cards   {}", config.generate.max_cards);
    println!("  review      {}", config.generate.review);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn card(front: &str, back: &str) -> Card {
        Card::plain(Arc::from("d.txt"), front.into(), vec![back.into()], None, 1)
    }

    #[test]
    fn check_warns_on_a_missing_workspace_icon() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "icon = \"assets/gone.svg\"\n").unwrap();
        std::fs::write(dir.path().join("a.txt"), "# a\n\t1\n").unwrap();
        // Warnings don't fail the check; the missing-icon path just adds one.
        assert!(check(vec![dir.path().to_path_buf()]).is_ok());
    }

    #[test]
    fn reset_selects_all_without_a_filter() {
        let cards = vec![card("A", "1"), card("B", "2")];
        assert_eq!(2, select_reset_ids(&cards, None).len());
    }

    #[test]
    fn reset_matches_front_substring_case_insensitively() {
        let cards = vec![
            card("Capital of Japan?", "Tokyo"),
            card("Largest planet?", "Jupiter"),
        ];
        let got = select_reset_ids(&cards, Some("japan"));
        assert_eq!(1, got.len());
        assert_eq!("Capital of Japan?", got[0].1);
    }

    #[test]
    fn reset_matches_a_numeric_id_exactly() {
        let cards = vec![card("A", "1"), card("B", "2")];
        let id = cards[1].id();
        assert_eq!(
            vec![(id, "B".to_string())],
            select_reset_ids(&cards, Some(&id.to_string()))
        );
    }

    #[test]
    fn reset_front_match_resets_all_cards_sharing_it() {
        // Cloze holes share a front but have distinct ids; one match clears all.
        let cards = vec![
            card("verb forms", "a"),
            card("verb forms", "b"),
            card("noun", "c"),
        ];
        let got = select_reset_ids(&cards, Some("verb forms"));
        assert_eq!(2, got.len());
        assert_ne!(got[0].0, got[1].0);
    }
}
