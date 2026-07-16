// Enables `#[coverage(off)]` under `cargo +nightly llvm-cov` (this is a
// separate bin crate from the lib, so it needs its own crate-root attr — see
// `src/lib.rs` for the matching one and why it's used sparingly).
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

mod common;
mod deck;
mod doctor;
mod generate;
mod launch;
mod progress;
mod share;

use std::path::PathBuf;

use alix::config::{self, Config};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use launch::launch;

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

    /// Spot-check the configured model's exam grading against six hand-labeled
    /// probes (three real, costed calls): does a wrong answer fail, does a
    /// correct one pass? A spot check, not a certification.
    #[arg(long)]
    grading: bool,

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
    /// Show, set, or clear this workspace's personal "ready by" deadline.
    Deadline(WorkspaceDeadlineArgs),
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
struct WorkspaceDeadlineArgs {
    /// The workspace directory.
    dir: PathBuf,
    /// A date (YYYY-MM-DD) to set, `clear` to remove; omit to show.
    date: Option<String>,
    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
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
    /// resolved from the deck, like `stats`/`list`/`reset`).
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

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
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

    /// Path of the progress store (default: resolved from the target, or the
    /// decks-dir root store for `--all`/`--card` with no target).
    #[arg(long)]
    store: Option<PathBuf>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => launch(cli.launch),
        Some(Command::Stats(args)) => progress::stats(args),
        Some(Command::List(args)) => progress::list(args),
        Some(Command::Reset(args)) => progress::reset(args),
        Some(Command::Generate(args)) => generate::generate_cmd(args),
        Some(Command::Deck(action)) => match action {
            DeckAction::Augment(args) => deck::augment_cmd(args),
            DeckAction::Import(args) => deck::import_cmd(args),
        },
        Some(Command::Workspace(action)) => match action {
            WorkspaceAction::Init(args) => deck::workspace_init_cmd(args),
            WorkspaceAction::Deadline(args) => deck::workspace_deadline_cmd(args),
        },
        Some(Command::Share(args)) => share::share_cmd(args),
        Some(Command::Receive(args)) => share::receive_cmd(args),
        Some(Command::Config { init }) => config_cmd(init),
        Some(Command::Doctor(args)) => doctor::doctor_cmd(args),
    }
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
    show("make_note", &keys.make_note);
    show("make_card", &keys.make_card);
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
