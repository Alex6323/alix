// A separate bin crate from the lib needs its own crate-root attr for
// `#[coverage(off)]` under nightly llvm-cov (see src/lib.rs for the lib's).
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

mod bug_report;
mod common;
mod deck;
mod doctor;
mod generate;
mod launch;
mod profile;
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

    /// Cards a single sitting serves (default: the `[review] max_session`
    /// config key, else 10). Its new-card share is `[review]
    /// new_cards_percent`.
    #[arg(long)]
    session: Option<usize>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,

    /// Mirror comma-separated log targets to stderr and record verbose targets.
    #[arg(long, value_delimiter = ',', value_name = "TARGETS")]
    log: Vec<alix::log::Target>,

    /// Launch every profile at once, each on its own port and bound to the LAN.
    /// Runs in the foreground; Ctrl-C (or closing the window) stops them all.
    #[arg(long, conflicts_with_all = ["dir", "config"])]
    launch_all: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Define and launch named per-person instances (their decks, port, adult/kids flavor).
    #[command(subcommand)]
    Profile(ProfileCommand),
    /// Check this setup's health, with a one-line fix per problem.
    ///
    /// Covers the config, the progress store, the decks folder, and the
    /// optional external CLIs. Add `--backends` to also probe the configured
    /// AI backend end to end (one real, tiny request).
    Doctor(DoctorArgs),
    /// Generate learning material with AI: say `deck` or `workspace`.
    ///
    /// `deck` turns one source into one deck: a web page, a file, or a
    /// directory taken as a whole. `workspace` explores a directory for a
    /// learning plan and builds a workspace from it. Which one you get is
    /// what you asked for, never what the plan turned out to be.
    #[command(subcommand)]
    Generate(GenerateAction),
    /// Show progress statistics for a deck, a folder, or a workspace.
    ///
    /// The target is a path: a single deck file reports that deck; a folder
    /// or workspace reports every deck inside it, each against the store it
    /// actually uses. E.g. `alix stats spanish.md` or `alix stats
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
    /// Initialize, augment, or import decks.
    #[command(subcommand)]
    Deck(DeckAction),
    /// Create and grow workspaces.
    #[command(subcommand)]
    Workspace(WorkspaceAction),
    /// Share a deck, folder, or workspace: over magic-wormhole, or as a .zip.
    ///
    /// Either way, what travels is a staged copy without your personal state
    /// (progress, recent list, local pacing). The default sends through the
    /// `wormhole` binary. Tell the receiver the code it prints; `--zip`
    /// writes an archive to pass along however you like instead.
    Share(ShareArgs),
    /// Receive a shared deck, folder, or workspace: by wormhole code, or from a .zip.
    ///
    /// A received deck lands in the decks directory (or `--workspace <dir>`);
    /// a received folder lands beside your other decks under its own name.
    /// Leaked personal files are stripped either way.
    Receive(ReceiveArgs),
    /// Write a private, reviewable diagnostics archive to attach to a bug report.
    BugReport(BugReportArgs),
    /// Show the configuration (key bindings) or create the config file.
    Config {
        /// Write a config file with the default bindings to edit.
        #[arg(long)]
        init: bool,
    },
}

#[derive(Args)]
struct BugReportArgs {
    /// Directory to write the archive into (default: current directory).
    #[arg(long, default_value = ".")]
    out: PathBuf,
    /// Include one deck verbatim after reviewing its private contents.
    #[arg(long)]
    include_deck: Option<PathBuf>,
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Create a profile: a named config for one person's instance.
    Add(ProfileAddArgs),
    /// List profiles (name, flavor, port, decks).
    List,
    /// Delete a profile's config.
    Remove(ProfileNameArgs),
    /// Set, show (no name), or clear (--clear) the default profile that bare `alix` launches.
    Default(ProfileDefaultArgs),
    /// Launch the named profile (bound to the LAN with its stored token).
    #[command(external_subcommand)]
    Launch(Vec<String>),
}

#[derive(Args)]
struct ProfileAddArgs {
    /// The profile name (also the config filename). Not add/list/remove/default.
    name: String,
    /// The decks folder this profile serves. Default: the configured decks dir.
    #[arg(long)]
    decks: Option<PathBuf>,
    /// The port this profile's server listens on. Default: the `[serve]` default (7777).
    #[arg(long)]
    port: Option<u16>,
    /// Serve the kids frontend for this profile (default: the adult frontend).
    #[arg(long, conflicts_with = "adult")]
    kids: bool,
    /// Serve the adult frontend for this profile (the default; explicit form).
    #[arg(long)]
    adult: bool,
}

#[derive(Args)]
struct ProfileNameArgs {
    name: String,
    /// Skip the confirmation prompt.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Args)]
struct ProfileDefaultArgs {
    /// The profile to make default. Omit to print the current default.
    name: Option<String>,
    /// Clear the default (bare `alix` reverts to the global config).
    #[arg(long, conflicts_with = "name")]
    clear: bool,
}

#[derive(Args)]
struct DoctorArgs {
    /// What to check instead of the configured setup: a decks folder or
    /// workspace root (with its own store, like `alix <dir>` serves it), or a
    /// single deck file to lint in depth.
    dir: Option<PathBuf>,

    /// Also probe the configured AI backend end to end. This sends one real
    /// (tiny) request, the only reliable way to confirm login + reachability.
    #[arg(long)]
    backends: bool,

    /// Probe all four supported backends (one real request each).
    #[arg(long, conflicts_with = "backends")]
    all_backends: bool,

    /// Spot-check the configured model's exam grading against the hand-labeled
    /// calibration probes (a few real, costed calls, batched by strictness):
    /// does a wrong answer fail, does a correct one pass? A spot check, not a
    /// certification.
    #[arg(long)]
    grading: bool,

    /// Stamp reviewed source excerpts and rebase exact excerpts that moved
    /// uniquely within their current file.
    #[arg(long)]
    repair_source_locators: bool,

    /// Rewrite each diverged span `position:` anchor to where the span's
    /// authored occurrence binds today (the keep-authored-occurrence edit).
    #[arg(long)]
    repair_positions: bool,

    /// Remove diagram stamps whose fence is gone, then re-freeze any stale
    /// or unfrozen mermaid fences. Workspace members only: standalone decks
    /// never freeze diagrams.
    #[arg(long)]
    repair_diagrams: bool,

    /// Rewrite each checked deck's frontmatter into the canonical key order:
    /// authored keys first, machine lines last. Opt-in only; an author's own
    /// order is never diagnosed against it.
    #[arg(long)]
    repair_frontmatter_order: bool,

    /// Rewrite each checked deck's trailing comment machinery into the
    /// canonical order (invocation, directives, regions, locator, id last).
    /// Opt-in only; an author's own order is never diagnosed against it.
    #[arg(long)]
    repair_comment_order: bool,

    /// Delete every backup (`*.bak`) file under the checked folder, after one
    /// confirmation. The backups are what `alix deck restore` swaps in.
    #[arg(long)]
    remove_backup_files: bool,

    /// Skip the backup-removal confirmation.
    #[arg(long, requires = "remove_backup_files")]
    yes: bool,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

/// The `alix workspace` subcommands: create and grow workspaces.
#[derive(Subcommand)]
enum WorkspaceAction {
    /// Initialize an empty workspace: a folder with an `alix.toml` and an
    /// `assets/` dir, no decks yet. Grow it with `alix generate deck … --into
    /// <dir>` or `alix deck import … --workspace <dir>`.
    Init(WorkspaceInitArgs),
    /// Reconcile frozen source-backed decks with their live sources. The first
    /// run stages an exact proposal; inspect it, then rerun with `--apply`.
    Update(WorkspaceUpdateArgs),
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
struct WorkspaceUpdateArgs {
    /// The workspace directory.
    dir: PathBuf,

    /// Publish the exact existing staged proposal without another AI call.
    #[arg(long, conflicts_with = "discard")]
    apply: bool,

    /// Delete the existing staged proposal without changing the workspace.
    #[arg(long, conflicts_with = "apply")]
    discard: bool,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
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

    /// Write a .zip archive instead of sending over wormhole: the offline
    /// fallback (mail it, put it on a stick).
    #[arg(long)]
    zip: bool,

    /// With `--zip`, where to write the archive: a file name, or a directory
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
    /// overwrite; move the old one aside first).
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand)]
enum GenerateAction {
    /// Turn one source into one deck: a web page, a file, or a directory read
    /// as a whole.
    ///
    /// `--trace` authors a predict-and-verify walk instead of facts cards, and
    /// naming a deck that already declares `trace:` builds its checkpoints in
    /// place.
    Deck(GenerateDeckArgs),
    /// Explore a directory for a learning plan and build a workspace from it.
    ///
    /// The plan is shown and confirmed before anything is built. A plan with a
    /// single item still builds a workspace, so this command never hands back
    /// a bare deck.
    Workspace(GenerateWorkspaceArgs),
}

/// The options both kinds of generation take.
#[derive(Args)]
struct GenerateCommonArgs {
    /// Public URL recorded as an additional `source:` (the workspace `source`
    /// for a generated workspace) for tutor context and exam grounding.
    #[arg(long, value_name = "URL")]
    source_url: Option<String>,

    /// The learning goal that scopes what is generated (default: understand
    /// the whole source).
    #[arg(long)]
    goal: Option<String>,

    /// Language for generated fronts, answers, choices, and notes.
    #[arg(long, value_name = "LANGUAGE")]
    language: Option<String>,

    /// Intended learner, used to set vocabulary, assumed knowledge, examples,
    /// and difficulty.
    #[arg(long, value_name = "AUDIENCE")]
    audience: Option<String>,

    /// Card shape for generated facts decks. Workspace trace items retain
    /// their predict-and-verify checkpoint shape.
    #[arg(long, value_enum)]
    card_style: Option<config::GenerateCardStyle>,

    /// Overwrite existing output (a deck file, or a non-empty workspace dir).
    #[arg(long)]
    force: bool,

    /// Skip confirmations: the large-source pre-flight, and any build
    /// go-ahead.
    #[arg(short, long)]
    yes: bool,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct GenerateDeckArgs {
    /// What to turn into a deck: a web page URL, a local file, a directory,
    /// or a deck that declares `trace:` in its frontmatter.
    source: String,

    /// An existing workspace to write the deck into, rather than the decks
    /// dir (`alix workspace init <dir>` creates one).
    #[arg(long, value_name = "DIR")]
    into: Option<PathBuf>,

    /// Author a trace over the source instead of facts cards: a short
    /// predict-and-verify walk over its shape, written as a trace deck.
    #[arg(long)]
    trace: bool,

    /// Print the trace suggestions and stop: generate nothing.
    #[arg(long, requires = "trace")]
    plan: bool,

    /// Output name (default: derived from the source). A `.md` extension is
    /// added if missing.
    #[arg(short, long)]
    output: Option<String>,

    /// Maximum number of cards (overrides the configured default).
    #[arg(long)]
    cards: Option<usize>,

    /// Run a second AI pass that reviews the draft and removes redundant
    /// cards (an extra call; also `generate.review` in the config).
    #[arg(long)]
    review: bool,

    /// Print the deck to stdout instead of writing a file.
    #[arg(long)]
    print: bool,

    #[command(flatten)]
    common: GenerateCommonArgs,
}

#[derive(Args)]
struct GenerateWorkspaceArgs {
    /// The directory to explore for a learning plan.
    source: String,

    /// Where to build the workspace (default: a folder named after the
    /// source, under the decks dir). It is created if it does not exist.
    #[arg(long, value_name = "DIR")]
    into: Option<PathBuf>,

    /// Print the plan and stop: build nothing.
    #[arg(long)]
    plan: bool,

    /// The workspace's display title (default: the folder name).
    #[arg(long)]
    title: Option<String>,

    /// Use this image as the workspace icon instead of letting the model draw
    /// one. Copied into `assets/`.
    #[arg(long)]
    icon: Option<PathBuf>,

    #[command(flatten)]
    common: GenerateCommonArgs,
}

#[derive(Subcommand)]
enum DeckAction {
    /// Initialize a hand-authored Markdown file as an Alix deck.
    Init(DeckInitArgs),
    /// Copy a workspace deck and its shareable files into another workspace.
    Copy(DeckTransferArgs),
    /// Move a workspace deck, its shareable files, and its progress.
    Move(DeckMoveArgs),
    /// Augment an existing deck with AI: multiple-choice distractors, or
    /// trivia notes. Augmentations are deliberate and persisted, so review stays
    /// instant and fully offline.
    Augment(AugmentArgs),
    /// Import an Anki TSV export into an alix deck.
    ///
    /// Expects tab-separated `front<TAB>back` lines.
    Import(ImportArgs),
    /// Remove a deck and everything that is its alone: the file, its review
    /// history, its frozen assets, its augmentations, and any backups.
    ///
    /// Total by design: nothing is backed up and this cannot be undone.
    Remove(DeckRemoveArgs),
    /// Swap a deck with its `.bak` backups (file, review history,
    /// augmentations), undoing the last overwrite.
    ///
    /// Nothing is destroyed: the swapped-away state becomes the new backup,
    /// so running it again swaps back.
    Restore(DeckRestoreArgs),
}

#[derive(Args)]
struct DeckRemoveArgs {
    /// The deck file to remove.
    deck: PathBuf,

    /// Skip the confirmation prompt.
    #[arg(short, long)]
    yes: bool,

    /// Progress store path override (default: resolved per deck).
    #[arg(long)]
    store: Option<PathBuf>,
}

#[derive(Args)]
struct DeckRestoreArgs {
    /// The deck file (or its former path) whose backups to swap in.
    deck: PathBuf,

    /// Progress store path override (default: resolved per deck).
    #[arg(long)]
    store: Option<PathBuf>,
}

#[derive(Args)]
struct DeckInitArgs {
    /// The Markdown file to initialize with stable deck and card IDs.
    deck: PathBuf,
}

#[derive(Args)]
struct DeckTransferArgs {
    /// An initialized direct member of an Alix workspace.
    deck: PathBuf,
    /// The destination Alix workspace.
    workspace: PathBuf,
}

#[derive(Args)]
struct DeckMoveArgs {
    /// An initialized direct member of an Alix workspace.
    deck: PathBuf,
    /// The destination Alix workspace.
    workspace: PathBuf,
    /// Skip the confirmation prompt.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Args)]
struct AugmentArgs {
    /// The deck file to augment.
    deck: PathBuf,

    /// What to augment, mirroring the review concepts. Each value describes
    /// itself below. All are cached in the workspace's augmentation document,
    /// never written into the deck; review reads them.
    #[arg(long, value_enum)]
    target: AugmentTarget,

    /// Free-text guidance for *how* to augment, woven into the prompt (e.g.
    /// "use common misconceptions", "add a surprising historical fact").
    #[arg(long)]
    with: Option<String>,

    /// User-files root used to include personal cards while augmenting. It does
    /// not change where workspace augmentation is written.
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
    /// A deck-level review order: a graph of how the cards relate plus a
    /// suggested walk, so review presents the cards foundations-first.
    /// `--with` steers the organizing principle (e.g. "by module and type
    /// dependency").
    #[value(name = "order")]
    Topology,
    /// A display-only reshape of a badly-shaped card: restructured front/answer/
    /// note and a suggested mode, applied at review without touching the deck.
    /// Plain (non-cloze) cards only.
    Format,
}

#[derive(Args)]
struct ImportArgs {
    /// The Anki TSV file to import (tab-separated `front<TAB>back` lines).
    file: PathBuf,

    /// Output deck name (default: a slug from the file name). Written into the
    /// decks directory; a `.md` extension is added if missing.
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
    /// (every deck inside it), e.g. `spanish.md` or `~/decks/flutter`.
    #[arg(value_name = "DECK|FOLDER|WORKSPACE")]
    target: PathBuf,

    /// State-root directory (default: resolved from the target).
    #[arg(long)]
    store: Option<PathBuf>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct ResetArgs {
    /// What to clear, as a path: one deck file, or a folder/workspace
    /// (every deck inside it), e.g. `spanish.md` or `~/decks/flutter`.
    #[arg(value_name = "DECK|FOLDER|WORKSPACE")]
    target: Option<PathBuf>,

    /// Reset one card: its numeric id, or text matching its front (searched
    /// within the target's decks).
    #[arg(long)]
    card: Option<String>,

    /// Clear progress for every card in the store.
    #[arg(long, conflicts_with_all = ["target", "card"])]
    all: bool,

    /// Clear only ORPHANED progress: store keys matching no card or deck in the
    /// scanned decks (a stripped `<!-- id: -->` comment, a hand-deleted deck, a
    /// double-mint). Orphans are never auto-pruned (they are evidence), so this
    /// is the explicit opt-in. Scopes to a named folder/workspace, else the
    /// decks-dir root store.
    #[arg(long, conflicts_with_all = ["card", "all"])]
    orphans: bool,

    /// Skip the confirmation prompt (for scripts / test loops).
    #[arg(short = 'y', long)]
    yes: bool,

    /// User-files directory (default: resolved from the target, or the
    /// decks-dir user root for `--all`/`--card` with no target).
    #[arg(long)]
    store: Option<PathBuf>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => {
            if cli.launch.launch_all {
                profile::launch_all(&cli.launch.log)
            } else if cli.launch.dir.is_none() && cli.launch.config.is_none() {
                match profile::resolve_default()? {
                    Some(name) => profile::launch_profile_with_log(&name, cli.launch.log),
                    None => launch(cli.launch, "default"),
                }
            } else {
                let instance = profile::instance_name_for_launch(
                    cli.launch.config.as_deref(),
                    cli.launch.dir.as_deref(),
                );
                launch(cli.launch, &instance)
            }
        }
        Some(Command::Profile(cmd)) => profile::run(cmd),
        Some(Command::Stats(args)) => progress::stats(args),
        Some(Command::List(args)) => progress::list(args),
        Some(Command::Reset(args)) => progress::reset(args),
        Some(Command::Generate(action)) => match action {
            GenerateAction::Deck(args) => generate::deck_cmd(args),
            GenerateAction::Workspace(args) => generate::workspace_cmd(args),
        },
        Some(Command::Deck(action)) => match action {
            DeckAction::Init(args) => deck::init_cmd(args),
            DeckAction::Copy(args) => deck::copy_cmd(args),
            DeckAction::Move(args) => deck::move_cmd(args),
            DeckAction::Augment(args) => deck::augment_cmd(args),
            DeckAction::Import(args) => deck::import_cmd(args),
            DeckAction::Remove(args) => deck::remove_cmd(args),
            DeckAction::Restore(args) => deck::restore_cmd(args),
        },
        Some(Command::Workspace(action)) => match action {
            WorkspaceAction::Init(args) => deck::workspace_init_cmd(args),
            WorkspaceAction::Update(args) => deck::workspace_update_cmd(args),
            WorkspaceAction::Deadline(args) => deck::workspace_deadline_cmd(args),
        },
        Some(Command::Share(args)) => share::share_cmd(args),
        Some(Command::Receive(args)) => share::receive_cmd(args),
        Some(Command::BugReport(args)) => bug_report::bug_report_cmd(args),
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
            "no config file at {}, using defaults; create one with \
             `alix config --init`",
            path.display()
        );
    }
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
    println!("  idle        {}s", config.generate.idle_timeout_secs);
    println!("  max_cards   {}", config.generate.max_cards);
    println!(
        "  language    {}",
        config.generate.language.as_deref().unwrap_or("(source)")
    );
    println!(
        "  audience    {}",
        config.generate.audience.as_deref().unwrap_or("(general)")
    );
    println!("  card_style  {}", config.generate.card_style.as_str());
    println!("  review      {}", config.generate.review);
    Ok(())
}
