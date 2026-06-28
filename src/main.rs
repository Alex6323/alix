use std::{
    collections::HashMap,
    io::{IsTerminal, Write},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};

use alix::{
    answer::Mode,
    augment::{self, AugmentCache, Topology, TopologyOrder},
    browse,
    card::{Card, Frontend},
    config::{self, Config, Strictness},
    deck::{Deck, DeckSettings, DeckState},
    generate, import, parser, picker,
    recent::{self, RecentDecks},
    scheduler::SchedulerKind,
    serve,
    session::{Order, Session, SessionOptions, histogram},
    store::{Store, default_store_path},
    time::{humanize_ms, now_ms},
    trace::{Phase, SourceBase, Trace, Walk},
    tui::{self, AfterReview, App},
    workspace,
};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use ratatui::DefaultTerminal;

/// Your personal AI tutor — built for understanding, not just remembering.
///
/// Decks are plain text files: `# question` starts a card, the indented
/// lines below it are the answer, `! text` adds a note, `% text` is a
/// comment. Without a subcommand, a review session is started.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    review: ReviewArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Review due cards (the default when deck files are given).
    Review(ReviewArgs),
    /// Show progress statistics for decks.
    Stats(DeckArgs),
    /// List all cards of decks with their stage and due time.
    List(DeckArgs),
    /// Clear stored progress for decks, a single card, or everything.
    Reset(ResetArgs),
    /// Check deck files for syntax errors and duplicate cards.
    Check {
        /// Deck files to check.
        #[arg(required = true)]
        decks: Vec<PathBuf>,
    },
    /// Read through decks card by card without grading (no progress is saved).
    Browse(BrowseArgs),
    /// Create or augment decks with Claude.
    #[command(subcommand)]
    Deck(DeckAction),
    /// Import an Anki TSV export (tab-separated `front<TAB>back` lines) into a
    /// alix deck.
    Import(ImportArgs),
    /// Sit the AI exam for a deck: open questions from its `% source:`, graded
    /// by Claude. Passing marks the deck mastered and unlocks its dependents.
    Exam(ExamArgs),
    /// Walk a trace: a predict-and-verify path through a `% source:` that
    /// builds understanding. At each checkpoint you predict, then the real
    /// excerpt is revealed and you judge the gap; the path ends with a
    /// compression.
    Trace(TraceArgs),
    /// Explore a source (a repo, directory, file, or URL) and print an ordered
    /// learning plan toward a goal: the facts decks and traces worth authoring,
    /// each tagged and dependency-ordered. Read-only; writes nothing.
    Explore(ExploreArgs),
    /// Open a workspace folder: pick a facts deck (→ review) or a trace deck
    /// (→ walk) from its members; you return to the picker when done.
    Workspace(WorkspaceArgs),
    /// Edit a deck's prerequisite decks (`% requires:`) with a checkbox picker.
    #[command(visible_alias = "require")]
    Deps {
        /// The deck whose prerequisites to edit.
        deck: PathBuf,
    },
    /// Show the configuration (key bindings) or create the config file.
    Config {
        /// Write a config file with the default bindings to edit.
        #[arg(long)]
        init: bool,
    },
}

#[derive(Args)]
struct WorkspaceArgs {
    /// The workspace folder to open.
    dir: PathBuf,

    /// Path of the progress store (default: platform data dir).
    #[arg(long)]
    store: Option<PathBuf>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct ExploreArgs {
    /// The source to explore: a repo `.`, a directory, a single file, or a URL.
    source: PathBuf,

    /// The learning goal that scopes the plan (default: understand the whole
    /// source). A broad goal covers every subsystem; a narrow one only its parts.
    #[arg(long)]
    goal: Option<String>,

    /// Scaffold the plan into a workspace folder at this path: an alix.toml plus
    /// a stub deck/trace file per item, wired by `% requires:`. Writes files.
    #[arg(long)]
    into: Option<PathBuf>,

    /// With --into, the workspace's display title (its `alix.toml` `title`).
    /// Omitted, the folder name is used; `--goal` becomes the description.
    #[arg(long, requires = "into")]
    title: Option<String>,

    /// With --into, the workspace's `[defaults]` `unlock-stage` (1–5): a member
    /// deck's exam/unlock opens once every card reaches this stage, without
    /// retiring the cards early.
    #[arg(long, requires = "into", value_parser = clap::value_parser!(u8).range(1..=5))]
    unlock_stage: Option<u8>,

    /// With --into, write into the directory even if it already contains files.
    #[arg(long, requires = "into")]
    force: bool,

    /// With --into, fill every stub in one explore session — checkpoints for
    /// traces, cards for facts decks — instead of leaving them empty. One coherent
    /// pass (the items know about each other); more model work.
    #[arg(long, requires = "into", conflicts_with = "walk")]
    build: bool,

    /// Build an explore walk instead of a plan: a predict-verify trace over the
    /// source's shape (what it is → its parts → entry → spine → what to trace),
    /// written to a file and walked right away.
    #[arg(long, conflicts_with = "into")]
    walk: bool,

    /// With --walk, the file to write the explore walk to (default explore.txt).
    #[arg(short, long, requires = "walk")]
    output: Option<PathBuf>,

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

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

/// The `alix deck` subcommands: create a deck, or augment an existing one.
#[derive(Subcommand)]
enum DeckAction {
    /// Generate a facts deck with Claude from a source — a web page URL or a
    /// local file/directory path. (The deck-side mirror of `alix trace`.)
    Generate(GenerateDeckArgs),
    /// Augment an existing deck with Claude — multiple-choice distractors, or
    /// trivia notes. Augmentations are deliberate and persisted, so review stays
    /// instant and fully offline.
    Augment(AugmentArgs),
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
    /// A deck-level topology: a graph of how the cards relate plus a suggested
    /// walk, so review can present them in a connected order. Experimental —
    /// prints the walk so you can judge whether it lands. `--with` steers the
    /// organizing principle (e.g. "by module and type dependency").
    Topology,
}

#[derive(Args)]
struct ImportArgs {
    /// The Anki TSV file to import (tab-separated `front<TAB>back` lines).
    file: PathBuf,

    /// Output deck name (default: a slug from the file name). Written into the
    /// decks directory; a `.txt` extension is added if missing.
    #[arg(short, long)]
    output: Option<String>,

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
struct ExamArgs {
    /// The deck to examine (must declare at least one `% source:`).
    deck: PathBuf,

    /// Number of questions (overrides the configured default).
    #[arg(long)]
    questions: Option<usize>,

    /// Grading strictness (overrides the deck's `% strictness:` and the
    /// `[exam]` default): strict, balanced, or lenient.
    #[arg(long, value_enum)]
    strictness: Option<Strictness>,

    /// Path of the progress store (default: platform data dir).
    #[arg(long)]
    store: Option<PathBuf>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct TraceArgs {
    /// The trace deck to walk (must declare a `% trace:`). With `--suggest`,
    /// this positional is instead a *source* to recon (a repo `.`, a directory,
    /// a file, or a URL) — not a deck.
    deck: PathBuf,

    /// Print the path — each checkpoint's prompt, key points and locator —
    /// without quizzing, then exit.
    #[arg(long)]
    map: bool,

    /// Build the trace: explore the `% source:` with Claude to discover the
    /// path, and write the checkpoints back into the deck file. Read-only
    /// exploration; overwrites a previous build.
    #[arg(long, conflicts_with = "map")]
    build: bool,

    /// Recon a SOURCE (the positional, here a repo/dir/file/URL — not a deck)
    /// and print a ranked menu of candidate traces to author. Read-only; writes
    /// nothing. Paste a suggestion into a new deck, then `--build` it.
    #[arg(long, conflicts_with_all = ["map", "build"])]
    suggest: bool,

    /// Have Claude grade each prediction against the checkpoint's key points
    /// (live grading) instead of self-grading. Costs a model call per hop.
    #[arg(long)]
    grade: bool,

    /// Scheduler used to schedule the checkpoints. Overrides the deck's
    /// `% scheduler:` directive; defaults to leitner.
    #[arg(short, long, value_enum)]
    scheduler: Option<SchedulerKind>,

    /// Walk the trace in the browser instead of the terminal.
    #[command(flatten)]
    serve: ServeOpts,

    /// Path of the progress store (default: platform data dir).
    #[arg(long)]
    store: Option<PathBuf>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

/// Options for serving an activity in the browser instead of the terminal.
/// Flattened into `review` and `browse`. `--port`/`--lan` require `--serve`, so
/// they cannot be given without it.
#[derive(Args)]
struct ServeOpts {
    /// Run in the browser (a local web page) instead of the terminal.
    #[arg(long)]
    serve: bool,

    /// Port to listen on with `--serve` (default: the `[serve]` config port,
    /// 7777).
    #[arg(long, requires = "serve")]
    port: Option<u16>,

    /// With `--serve`, listen on all network interfaces so phones and tablets
    /// on the same network can reach it (no authentication — opt-in).
    #[arg(long, requires = "serve")]
    lan: bool,
}

#[derive(Args)]
struct BrowseArgs {
    /// Deck files to browse (omit to pick interactively).
    decks: Vec<PathBuf>,

    #[command(flatten)]
    serve: ServeOpts,
}

#[derive(Args)]
struct DeckArgs {
    /// Deck files.
    #[arg(required = true)]
    decks: Vec<PathBuf>,

    /// Scheduler used to compute due times.
    #[arg(short, long, value_enum, default_value_t)]
    scheduler: SchedulerKind,

    /// Path of the progress store (default: platform data dir).
    #[arg(long)]
    store: Option<PathBuf>,
}

#[derive(Args)]
struct ResetArgs {
    /// Deck files whose card progress to clear.
    decks: Vec<PathBuf>,

    /// Reset one card: its numeric id, or text matching its front (searched
    /// within the given decks).
    #[arg(long)]
    card: Option<String>,

    /// Pick cards to reset from a checkbox list (over the given decks, or decks
    /// chosen interactively).
    #[arg(long, conflicts_with_all = ["card", "all"])]
    cards: bool,

    /// Clear progress for every card in the store.
    #[arg(long, conflicts_with_all = ["decks", "card"])]
    all: bool,

    /// Skip the confirmation prompt (for scripts / test loops).
    #[arg(short = 'y', long)]
    yes: bool,

    /// Path of the progress store (default: platform data dir).
    #[arg(long)]
    store: Option<PathBuf>,
}

#[derive(Args)]
struct ReviewArgs {
    /// Deck files to review.
    decks: Vec<PathBuf>,

    /// How answers are checked. Overrides a deck's `% mode:` directive;
    /// defaults to flip.
    #[arg(short, long, value_enum)]
    mode: Option<Mode>,

    /// Scheduling algorithm. Overrides a deck's `% scheduler:` directive;
    /// defaults to leitner.
    #[arg(short, long, value_enum)]
    scheduler: Option<SchedulerKind>,

    /// Order cards are shown in. Overrides a deck's `% order:` directive;
    /// defaults to scheduled.
    #[arg(short, long, value_enum)]
    order: Option<Order>,

    /// Reorder the due set by a stored AI topology of this name (see `alix deck
    /// augment --target topology`). With no name, a deck's single cached topology
    /// is used automatically.
    #[arg(long)]
    topology: Option<String>,

    /// Maximum number of new (never-seen) cards to introduce.
    #[arg(short, long, default_value_t = 10)]
    new: usize,

    /// Maximum number of cards in this session.
    #[arg(short, long)]
    limit: Option<usize>,

    /// Ignore due times and review all previously seen cards.
    #[arg(long)]
    cram: bool,

    /// Tolerated typos (Levenshtein distance) per line in fuzzy mode.
    #[arg(long, default_value_t = 2)]
    max_typos: usize,

    /// Path of the progress store (default: platform data dir).
    #[arg(long)]
    store: Option<PathBuf>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,

    #[command(flatten)]
    serve: ServeOpts,
}

fn main() -> Result<()> {
    // One-time: adopt a pre-rename `flash` data dir so existing progress survives.
    alix::store::migrate_legacy_data_dir();
    let cli = Cli::parse();
    match cli.command {
        None => review(cli.review),
        Some(Command::Review(args)) => review(args),
        Some(Command::Stats(args)) => stats(args),
        Some(Command::List(args)) => list(args),
        Some(Command::Reset(args)) => reset(args),
        Some(Command::Check { decks }) => check(decks),
        Some(Command::Browse(args)) => browse(args),
        Some(Command::Deck(action)) => match action {
            DeckAction::Generate(args) => deck_cmd(args),
            DeckAction::Augment(args) => augment_cmd(args),
        },
        Some(Command::Import(args)) => import_cmd(args),
        Some(Command::Exam(args)) => exam_cmd(args),
        Some(Command::Trace(args)) => trace_cmd(args),
        Some(Command::Explore(args)) => explore_cmd(args),
        Some(Command::Workspace(args)) => workspace_cmd(args),
        Some(Command::Deps { deck }) => deps_cmd(deck),
        Some(Command::Config { init }) => config_cmd(init),
    }
}

/// Opens the progress store (creating an empty one on first use).
fn open_store(path: Option<PathBuf>) -> Result<Store> {
    let path = match path {
        Some(path) => path,
        None => default_store_path().context("cannot determine the data directory")?,
    };
    Store::open(&path).context("cannot open the progress store")
}

/// Which progress store a set of decks should use: the `--store` override, else
/// the single workspace they all share (a deck is "in" a workspace when its
/// parent folder has an `alix.toml`), else the global default (`None`). Loose
/// decks, a plain folder, or decks spanning different workspaces all fall back
/// to the global store — so a workspace's progress lives with the workspace,
/// while everything else shares the one global store.
fn store_path_for(decks: &[PathBuf], cli_override: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = cli_override {
        return Some(path.to_path_buf());
    }
    let mut stores = decks.iter().map(|deck| {
        deck.parent()
            .filter(|p| workspace::is_workspace(p))
            .map(workspace::store_path)
    });
    match stores.next() {
        Some(Some(first)) if stores.all(|s| s.as_ref() == Some(&first)) => Some(first),
        _ => None,
    }
}

/// Opens the progress store for `decks`, honoring `--store`. See
/// [`store_path_for`].
fn store_for(decks: &[PathBuf], cli_override: Option<PathBuf>) -> Result<Store> {
    open_store(store_path_for(decks, cli_override.as_deref()))
}

/// The cards of all loaded decks, a header label, the per-subject deck info
/// for the TUI, and the per-deck `% key: value` settings.
type LoadedDecks = (
    Vec<Card>,
    String,
    std::collections::HashMap<String, tui::DeckInfo>,
    Vec<DeckSettings>,
);

/// Loads all decks and returns their cards, a label for the header, the
/// per-subject deck info (file path and reference links) for the TUI, and the
/// per-deck `% key: value` settings.
fn load_decks(paths: &[PathBuf], defaults: &HashMap<String, DeckSettings>) -> Result<LoadedDecks> {
    let mut cards = Vec::new();
    let mut names = Vec::new();
    let mut decks = std::collections::HashMap::new();
    let mut settings = Vec::new();
    for path in paths {
        // A deck that belongs to a workspace inherits the workspace's shared
        // directives (keyed by file name); others load with no defaults.
        let deck = match path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| defaults.get(n))
        {
            Some(ws) => Deck::load_with_defaults(path, ws)?,
            None => Deck::load(path)?,
        };
        names.push(deck.display_name());
        decks.insert(
            deck.subject.clone(),
            tui::DeckInfo {
                path: deck.path.clone(),
                // Ask-Claude references include the deck's `% link:`s and any
                // URL `% source:` (a source doubles as a reference).
                links: deck.reference_links(),
                // Where the grounded tutor reads this deck's source (opt-in).
                source_root: deck.source_root(),
                // Resolved against the global config in `build_review`.
                source_access: false,
                // For resolving a card's `% at:` citation excerpt on reveal.
                source_base: SourceBase::for_deck(&deck),
            },
        );
        settings.push(deck.settings);
        cards.extend(deck.cards);
    }
    Ok((cards, names.join(", "), decks, settings))
}

/// The result of [`expand_workspaces`]: the member deck files to load, the
/// per-deck workspace directive defaults (keyed by file name), and the session
/// label when a single workspace was requested.
struct Expanded {
    decks: Vec<PathBuf>,
    defaults: HashMap<String, DeckSettings>,
    label: Option<String>,
}

/// Expands any workspace folder in `deck_paths` into its member deck files,
/// tagging each member (by file name) with the workspace's shared directive
/// defaults. Plain file paths pass through untagged. When a single workspace
/// was requested, its display name becomes the session label.
fn expand_workspaces(deck_paths: &[PathBuf]) -> Result<Expanded> {
    let mut decks = Vec::new();
    let mut defaults: HashMap<String, DeckSettings> = HashMap::new();
    let mut label = None;
    for path in deck_paths {
        // Any folder of decks expands (a workspace applies its `alix.toml`
        // defaults; a plain folder loads with defaults).
        if workspace::has_decks(path) {
            let ws = workspace::Workspace::load(path)?;
            if deck_paths.len() == 1 {
                label = Some(ws.display_name());
            }
            for member in ws.members {
                if let Some(name) = member.file_name().and_then(|n| n.to_str()) {
                    defaults.insert(name.to_string(), ws.settings.clone());
                }
                decks.push(member);
            }
        } else {
            // A deck file picked from inside a workspace folder (a subset
            // selection) still inherits that workspace's shared directives.
            if let Some(parent) = path.parent()
                && parent.join(workspace::MANIFEST).is_file()
                && let Ok(ws) = workspace::Workspace::load(parent)
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                defaults.insert(name.to_string(), ws.settings);
            }
            decks.push(path.clone());
        }
    }
    Ok(Expanded {
        decks,
        defaults,
        label,
    })
}

/// Resolves a per-run setting from three sources, most specific first: an
/// explicit CLI flag, then a value declared by the loaded decks (used when
/// they agree), then the built-in default. Decks that disagree fall back to
/// the default with a warning.
fn resolve<T: Copy + PartialEq>(
    name: &str,
    cli: Option<T>,
    declared: impl Iterator<Item = Option<T>>,
    default: T,
) -> T {
    if let Some(value) = cli {
        return value;
    }
    let mut distinct: Vec<T> = Vec::new();
    for value in declared.flatten() {
        if !distinct.contains(&value) {
            distinct.push(value);
        }
    }
    match distinct.as_slice() {
        [] => default,
        [only] => *only,
        _ => {
            eprintln!("warning: decks disagree on `{name}`; using the default");
            default
        }
    }
}

/// Resolves which decks to act on: the given paths, or — when none are given —
/// the interactive picker (recent decks + the decks directory). Returns
/// `Ok(None)` if the picker was cancelled or nothing was selected. The picker
/// needs a terminal.
#[expect(clippy::too_many_arguments)] // a thin picker shim; grouping would obscure
fn pick_decks_if_empty(
    terminal: Option<&mut DefaultTerminal>,
    decks: Vec<PathBuf>,
    config: &Config,
    recent: &RecentDecks,
    store: &Store,
    enforce_locks: bool,
    gate_reviewable: bool,
    start_in: Option<&Path>,
    focus: Option<&Path>,
) -> Result<Option<picker::Picked>> {
    if !decks.is_empty() {
        return Ok(Some(picker::Picked {
            decks,
            workspace: None, // decks named explicitly: no workspace to return to
        }));
    }
    if !std::io::stdout().is_terminal() {
        bail!("no deck files given; try `alix <deck.txt>...` or `alix --help`");
    }
    let terminal = terminal.expect("the interactive picker needs a terminal");
    let decks_dir = config.decks_dir().context("cannot determine ~/decks")?;
    let picked = picker::pick(
        terminal,
        &decks_dir,
        recent,
        store,
        enforce_locks,
        gate_reviewable,
        start_in,
        focus,
        &config.picker,
    )?;
    Ok((!picked.decks.is_empty()).then_some(picked))
}

/// A review session built from the deck selection and settings, ready to be
/// driven by either the TUI or the web frontend. `decks` and `config` are only
/// needed by the TUI (key bindings, ask-Claude, reference links).
struct ReviewSession {
    session: Session,
    store: Store,
    /// CLI `--mode` override (each card otherwise uses its own mode).
    mode_override: Option<Mode>,
    label: String,
    decks: HashMap<String, tui::DeckInfo>,
    config: Config,
    /// How many cards were filtered out as not reviewable in the target
    /// frontend (e.g. image cards excluded from the TUI).
    hidden: usize,
    /// The resolved topology name when topology-ordered, for the TUI breadcrumb.
    topology_name: Option<String>,
}

/// What the TUI picker resolved to: a review session, or — when a single
/// exam-due deck was chosen — that deck's exam.
enum Started {
    Review(Box<ReviewSession>),
    Exam {
        deck: Box<Deck>,
        store: Store,
        config: Box<Config>,
    },
    /// A single trace deck picked interactively: walk it (predict → reveal)
    /// rather than flatten it into a card review.
    Walk {
        trace: Box<Trace>,
        scheduler: SchedulerKind,
        store: Store,
    },
}

/// What [`load_review_session`] resolved: the activity to run, the `workspace` it
/// was drilled into (to return there afterwards), and the `focus` deck to re-land
/// the picker's cursor on so the selection doesn't jump while the user is away.
struct LoadedSession {
    started: Started,
    workspace: Option<PathBuf>,
    focus: Option<PathBuf>,
}

/// Loads the decks named (or picked) for a review, resolves prerequisites and
/// the mode/scheduler/order settings, and builds the session and store. Shared
/// by `alix review` (TUI) and `alix serve` (web). Returns `Ok(None)` when the
/// picker was cancelled.
/// A review session built from an explicit set of deck paths. Shared by the TUI
/// path and the web frontend's `/api/select` (via a builder closure).
struct ReviewBuild {
    session: Session,
    label: String,
    decks: HashMap<String, tui::DeckInfo>,
    /// Cards dropped because they are not reviewable in the target frontend
    /// (e.g. image cards excluded from the TUI).
    hidden: usize,
    /// The resolved topology's name, if the session is topology-ordered, so a
    /// frontend can fetch it from the augment cache to show the connective cue.
    topology_name: Option<String>,
}

/// Builds a review session from explicit `deck_paths` (no interactive picker):
/// resolves `% requires:` prerequisites, applies deck directives and the
/// `target`-frontend filter, builds the `Session`, and records the decks as
/// recent. The store is borrowed (the caller owns it), so the web server can
/// reuse one store across repeated selections.
fn build_review(
    deck_paths: Vec<PathBuf>,
    args: &ReviewArgs,
    config: &Config,
    store: &Store,
    recent: &mut RecentDecks,
    target: Frontend,
) -> Result<ReviewBuild> {
    // Expand any workspace folder into its member decks (tagged with the
    // workspace's shared directives). A deck's `% requires:` prerequisites are
    // NOT pulled into the session — you review only the deck(s) you picked
    // (the dependency graph gates exams, not what a review session contains).
    let expanded = expand_workspaces(&deck_paths)?;
    let (cards, deck_label, mut decks, settings) = load_decks(&expanded.decks, &expanded.defaults)?;
    // Resolve each deck's effective ask-tutor source access: a deck in a
    // workspace takes that workspace's `source_access` override if it sets one,
    // else the global `[ask] source_access`.
    for info in decks.values_mut() {
        let workspace_override = info
            .path
            .parent()
            .filter(|p| workspace::is_workspace(p))
            .and_then(workspace::manifest_source_access);
        info.source_access = workspace_override.unwrap_or(config.ask.source_access);
    }
    // A single workspace shows its own title as the session label.
    let label = expanded.label.unwrap_or(deck_label);

    // Keep only the cards reviewable in the target frontend; a card declares
    // `Any` (default), or its specific frontend. Image cards are web-only, so
    // they drop out of the TUI here (and the caller reports the count).
    let total = cards.len();
    let mut cards: Vec<_> = cards
        .into_iter()
        .filter(|c| matches!(c.frontend(), Frontend::Any) || c.frontend() == target)
        .collect();
    let hidden = total - cards.len();

    // Merge in any AI-generated notes from the sidecar cache (`alix deck augment
    // --target notes`) — shown with the card's own deck note on reveal. (Question
    // variants are rotated in per-presentation by the frontends, and distractors
    // are read when a choice question is built.)
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for card in &mut cards {
        if let Some(note) = augment.note(card.id()) {
            card.append_note(&[note.to_string()]);
        }
    }

    // Resolve the topology that reorders this session (if any) and project it to
    // a session-ready order. The resolved name travels on `ReviewBuild` so the
    // web frontend can show the "why this card follows the last" cue from the
    // same topology.
    let topology = resolve_topology(args.topology.as_deref(), &augment)?;
    let topology_name = topology.map(|t| t.name.clone());
    let topology_order = topology.map(|t| TopologyOrder::from_walk(&t.walk));

    // Directives (scheduler/order) come from the session's decks.
    let target_settings: Vec<&DeckSettings> = settings.iter().collect();

    // scheduler/order are deck/session-level: CLI flag > deck directive >
    // default. `mode` is now per-card (resolved at review time from the card's
    // own `% mode:`), so only the CLI override is carried here.
    let scheduler = resolve(
        "scheduler",
        args.scheduler,
        target_settings.iter().map(|s| s.scheduler),
        SchedulerKind::default(),
    );
    let order = resolve(
        "order",
        args.order,
        target_settings.iter().map(|s| s.order),
        Order::default(),
    );

    let options = SessionOptions {
        max_new: args.new,
        limit: args.limit,
        cram: args.cram,
        order,
        topology: topology_order,
    };
    let session = Session::new(cards, store, scheduler, options, now_ms());

    // Remember these decks for next time's picker — but only when there is
    // actually something to review, so merely opening a deck with nothing due
    // doesn't bump it to the top of the recent list.
    if !session.is_finished() {
        recent.record(&deck_paths, now_ms());
        let _ = recent.save();
    }

    Ok(ReviewBuild {
        session,
        label,
        decks,
        hidden,
        topology_name,
    })
}

/// Resolves which stored topology, if any, reorders this session: an explicit
/// `--topology <name>` must name a cached topology (else an error), no flag with
/// exactly one cached topology auto-uses it, and zero-or-several without a name
/// leaves ordering to the scheduler.
fn resolve_topology<'a>(
    name: Option<&str>,
    augment: &'a AugmentCache,
) -> Result<Option<&'a Topology>> {
    match name {
        Some(name) => match augment.topology(name) {
            Some(topology) => Ok(Some(topology)),
            None => bail!(
                "no topology named `{name}` is cached for this deck — run `alix deck augment <deck> --target topology`"
            ),
        },
        None => Ok(match augment.topologies() {
            [single] => Some(single),
            _ => None,
        }),
    }
}

/// If a single trace deck was **picked** interactively, returns its loaded deck
/// — the signal to walk it rather than flatten it into a card review.
/// `from_picker` gates this: an explicit `alix review <trace>` (decks named on
/// the command line) keeps reviewing, honoring the literal command.
fn single_trace_to_walk(from_picker: bool, deck_paths: &[PathBuf]) -> Option<Deck> {
    if !from_picker {
        return None;
    }
    match deck_paths {
        [path] => Deck::load(path).ok().filter(|deck| deck.is_trace()),
        _ => None,
    }
}

/// The TUI review path: pick decks if none were given, then build the session
/// — unless a single chosen deck is `exam due`, in which case it resolves to
/// that deck's exam instead of a (cardless) review.
fn load_review_session(
    terminal: Option<&mut DefaultTerminal>,
    args: &ReviewArgs,
    target: Frontend,
    start_in: Option<&Path>,
    focus: Option<&Path>,
) -> Result<Option<LoadedSession>> {
    let config = Config::load(args.config.as_deref())?;
    let mut recent = RecentDecks::load(
        recent::default_recent_path().context("cannot determine the data directory")?,
    );
    // Whether the deck list came from the picker (no decks named on the command
    // line): only then does a single trace route to a walk — an explicit
    // `alix review <trace>` still flattens it to a card review.
    let from_picker = args.decks.is_empty();
    // The picker's badges/locks for the top-level list read the global store
    // (its rows are loose decks); a drilled-into workspace shows its own store.
    let store = open_store(args.store.clone())?;
    // Review enforces locking — a deck whose prerequisites aren't finished
    // can't be started from the picker.
    // Gate unreviewable decks in the picker — but not under `--cram`, which
    // ignores cooldowns, so everything seen is fair game.
    let Some(picked) = pick_decks_if_empty(
        terminal,
        args.decks.clone(),
        &config,
        &recent,
        &store,
        true,
        !args.cram,
        start_in,
        focus,
    )?
    else {
        return Ok(None); // picker cancelled or nothing selected
    };
    // `workspace` is the folder the decks were drilled into, if any — the caller
    // returns there after the activity ends.
    let picker::Picked {
        decks: deck_paths,
        workspace,
    } = picked;
    // The deck the user launched — handed back so the caller can re-focus it when
    // the picker re-opens (the selection shouldn't jump while they're away).
    let focus_deck = deck_paths.first().cloned();
    // The session tracks progress in the decks' store — a workspace's own store
    // when they all live in one, else the global store.
    let store = store_for(&deck_paths, args.store.clone())?;
    // A single trace deck picked interactively launches its walk, not a
    // flattened card review.
    if let Some(deck) = single_trace_to_walk(from_picker, &deck_paths) {
        let scheduler = args
            .scheduler
            .or(deck.settings.scheduler)
            .unwrap_or_default();
        let trace = Trace::from_deck(&deck)?;
        return Ok(Some(LoadedSession {
            started: Started::Walk {
                trace: Box::new(trace),
                scheduler,
                store,
            },
            workspace,
            focus: focus_deck,
        }));
    }
    // A single exam-due deck launches its exam rather than an empty review —
    // unless its exam is locked (a sourced prerequisite isn't passed), in which
    // case it falls through to a (possibly empty) review instead of opening an
    // exam the deck isn't allowed to sit yet.
    if let [path] = deck_paths.as_slice()
        && let Ok(deck) = Deck::load(path)
        && !deck.sources.is_empty()
        && deck.state(&store) == DeckState::ExamDue
        && !alix::deck::is_locked(&deck, config.decks_dir().as_deref(), &store)
    {
        return Ok(Some(LoadedSession {
            started: Started::Exam {
                deck: Box::new(deck),
                store,
                config: Box::new(config),
            },
            workspace,
            focus: focus_deck,
        }));
    }
    let build = build_review(deck_paths, args, &config, &store, &mut recent, target)?;
    Ok(Some(LoadedSession {
        started: Started::Review(Box::new(ReviewSession {
            session: build.session,
            store,
            mode_override: args.mode,
            label: build.label,
            decks: build.decks,
            config,
            hidden: build.hidden,
            topology_name: build.topology_name,
        })),
        workspace,
        focus: focus_deck,
    }))
}

/// Launches the interactive exam TUI for `deck`, resolving strictness from the
/// deck's `% strictness:` or the `[exam]` default.
fn run_exam_app(deck: Deck, config: &Config, store: Store) -> Result<()> {
    exam_app(deck, config, store).run()
}

/// Runs the exam TUI on the picker's live `terminal` (no teardown).
fn run_exam_app_on(
    terminal: &mut DefaultTerminal,
    deck: Deck,
    config: &Config,
    store: Store,
) -> Result<()> {
    exam_app(deck, config, store).run_on(terminal)
}

/// Browses a single deck from the session-end summary — on the caller's live
/// `terminal` (the picker flow) when given, else standalone.
fn browse_one(
    path: &std::path::Path,
    store_override: Option<PathBuf>,
    config: &Config,
    terminal: Option<&mut DefaultTerminal>,
) -> Result<()> {
    let mut recent = RecentDecks::load(
        recent::default_recent_path().context("cannot determine the data directory")?,
    );
    let decks = vec![path.to_path_buf()];
    let deck_store = store_for(&decks, store_override)?;
    let build = build_browse(decks, &mut recent, Frontend::Tui)?;
    let paths = subject_paths(build.decks);
    match terminal {
        Some(t) => browse::run_on(
            t,
            build.cards,
            build.label,
            config.browse.clone(),
            paths,
            deck_store,
        ),
        None => browse::run(
            build.cards,
            build.label,
            config.browse.clone(),
            paths,
            deck_store,
        ),
    }
}

/// Builds the exam TUI, resolving strictness from the deck's `% strictness:` or
/// the `[exam]` default.
fn exam_app(deck: Deck, config: &Config, store: Store) -> tui::ExamApp {
    let strictness = deck
        .settings
        .exam_strictness
        .unwrap_or(config.exam.strictness);
    let decks_dir = config.decks_dir();
    tui::ExamApp::new(
        deck,
        strictness,
        config.exam.clone(),
        config.ask.clone(),
        store,
        decks_dir,
    )
}

/// Builds the browse card list from explicit `deck_paths` (no picker). Mirrors
/// [`build_review`] for the read-only browse view: loads decks and applies the
/// `target`-frontend filter, but builds no scheduler session.
fn build_browse(
    deck_paths: Vec<PathBuf>,
    recent: &mut RecentDecks,
    target: Frontend,
) -> Result<BrowseBuild> {
    let expanded = expand_workspaces(&deck_paths)?;
    let (cards, deck_label, decks, _) = load_decks(&expanded.decks, &expanded.defaults)?;
    let label = expanded.label.unwrap_or(deck_label);
    let cards: Vec<_> = cards
        .into_iter()
        .filter(|c| matches!(c.frontend(), Frontend::Any) || c.frontend() == target)
        .collect();
    recent.record(&deck_paths, now_ms());
    let _ = recent.save();
    Ok(BrowseBuild {
        cards,
        label,
        decks,
    })
}

/// Browse cards built from an explicit set of deck paths.
struct BrowseBuild {
    cards: Vec<Card>,
    label: String,
    decks: HashMap<String, tui::DeckInfo>,
}

/// The IP/port to serve on: localhost unless `--lan`, and the `--port` flag or
/// the configured `[serve]` port.
fn serve_addr(port: Option<u16>, lan: bool, config: &Config) -> SocketAddr {
    let ip = if lan {
        Ipv4Addr::UNSPECIFIED // 0.0.0.0 — reachable from the local network
    } else {
        Ipv4Addr::LOCALHOST // 127.0.0.1 — this machine only
    };
    SocketAddr::from((ip, port.unwrap_or(config.serve.port)))
}

/// Subject → deck file path, for the web frontend's card removal.
fn subject_paths(decks: HashMap<String, tui::DeckInfo>) -> HashMap<String, PathBuf> {
    decks
        .into_iter()
        .map(|(subject, info)| (subject, info.path))
        .collect()
}

fn review(args: ReviewArgs) -> Result<()> {
    if args.serve.serve {
        return review_serve(args);
    }
    // Explicit decks: build and run once, standalone — the activity owns its
    // terminal and prints a summary, as before. No picker, no return-loop.
    if !args.decks.is_empty() {
        return match load_review_session(None, &args, Frontend::Tui, None, None)? {
            Some(loaded) => run_started(loaded.started, &args),
            None => Ok(()),
        };
    }
    // Picker flow: one terminal shared by the picker and every activity it
    // launches, so opening a deck and returning to its workspace never tear the
    // TUI down and back up.
    let mut terminal = ratatui::init();
    let result = review_loop(&mut terminal, &args);
    ratatui::restore();
    result
}

/// The picker review loop: pick (or reopen a workspace), run the activity on the
/// shared `terminal`, and — when it came from a workspace — return there for the
/// next. Stays on one live screen the whole time.
fn review_loop(terminal: &mut DefaultTerminal, args: &ReviewArgs) -> Result<()> {
    let mut start_in: Option<PathBuf> = None;
    let mut focus: Option<PathBuf> = None;
    loop {
        let Some(loaded) = load_review_session(
            Some(&mut *terminal),
            args,
            Frontend::Tui,
            start_in.as_deref(),
            focus.as_deref(),
        )?
        else {
            return Ok(()); // picker cancelled / nothing selected
        };
        let LoadedSession {
            started,
            workspace,
            focus: launched,
        } = loaded;
        run_started_on(terminal, started, args)?;
        // Always return to the picker after an activity — a workspace member
        // reopens its workspace, a loose deck the top list. Only an `Esc` at the
        // picker itself (above) quits. Re-focus the deck just launched so the
        // selection stays put under the user.
        start_in = workspace;
        focus = launched;
    }
}

/// Runs one resolved activity — a card review, a trace walk, or an exam — to
/// completion, each managing its own terminal and printing a summary. Used for
/// explicit `alix <deck>` (no picker).
fn run_started(started: Started, args: &ReviewArgs) -> Result<()> {
    match started {
        Started::Exam {
            deck,
            store,
            config,
        } => run_exam_app(*deck, &config, store),
        Started::Walk {
            trace,
            scheduler,
            mut store,
        } => run_walk(*trace, scheduler, &mut store, None).map(|_| ()),
        Started::Review(rs) => run_review_tui(*rs, args),
    }
}

/// Like [`run_started`] but on the picker's live `terminal`, so the activity and
/// the picker share one screen. A trace walk is line-based, so it drops out of
/// the alt screen and re-enters it for the picker.
fn run_started_on(
    terminal: &mut DefaultTerminal,
    started: Started,
    args: &ReviewArgs,
) -> Result<()> {
    match started {
        Started::Exam {
            deck,
            store,
            config,
        } => run_exam_app_on(terminal, *deck, &config, store),
        Started::Walk {
            trace,
            scheduler,
            mut store,
        } => {
            ratatui::restore();
            let result = run_walk(*trace, scheduler, &mut store, None).map(|_| ());
            *terminal = ratatui::init();
            result
        }
        Started::Review(rs) => run_review_on(terminal, *rs, args),
    }
}

/// The shared-terminal review: like [`run_review_tui`] but on the picker's
/// terminal and without the post-run summary print (the App shows its own, and a
/// print would corrupt the alt screen). The picker gates out empty sessions, so
/// this just bails quietly if one slips through.
fn run_review_on(
    terminal: &mut DefaultTerminal,
    rs: ReviewSession,
    args: &ReviewArgs,
) -> Result<()> {
    let ReviewSession {
        session,
        store,
        mode_override,
        label,
        decks,
        config,
        hidden,
        topology_name,
    } = rs;
    if session.is_finished() || (hidden > 0 && session.cards().is_empty()) {
        return Ok(());
    }
    let ui_options = alix::tui::Options {
        mode_override,
        max_typos: args.max_typos,
        deck_label: label,
        keys: config.keys.clone(),
        ask: config.ask.clone(),
        decks,
        topology_name,
    };
    let (_stats, next_action) = App::new(session, store, ui_options).run_on(terminal)?;
    match next_action {
        Some(AfterReview::Exam(path)) => {
            let store = store_for(std::slice::from_ref(&path), args.store.clone())?;
            let deck = Deck::load(&path)?;
            run_exam_app_on(terminal, deck, &config, store)
        }
        Some(AfterReview::Browse(path)) => {
            browse_one(&path, args.store.clone(), &config, Some(terminal))
        }
        None => Ok(()),
    }
}

/// Runs the review TUI for a built session: reports cards that need the browser,
/// the nothing-due case, then the App, its summary, and any follow-on exam.
fn run_review_tui(rs: ReviewSession, args: &ReviewArgs) -> Result<()> {
    let ReviewSession {
        session,
        store,
        mode_override,
        label,
        decks,
        config,
        hidden,
        topology_name,
    } = rs;

    // Some cards can't be shown in the terminal (images need the browser).
    if hidden > 0 {
        if session.cards().is_empty() {
            println!(
                "All {hidden} card(s) in this deck need the browser. \
                 Run the same command with --serve to review them."
            );
            return Ok(());
        }
        println!("{hidden} card(s) need the browser — run with --serve to review them.");
    }

    if session.is_finished() {
        println!("Nothing to review right now — all cards are on cooldown.");
        let now = now_ms();
        if let Some(due) = session.next_due_at(&store).filter(|&due| due > now) {
            println!("Next card is due in {}.", humanize_ms(due - now));
        }
        return Ok(());
    }

    let ui_options = alix::tui::Options {
        mode_override,
        max_typos: args.max_typos,
        deck_label: label,
        keys: config.keys.clone(),
        ask: config.ask.clone(),
        decks,
        topology_name,
    };
    let (stats, next_action) = App::new(session, store, ui_options).run()?;
    println!(
        "Reviewed {} cards: {} passed, {} failed.",
        stats.reviews, stats.passed, stats.failed
    );
    // If a deck became exam-due this session, the user can sit its exam or browse
    // it from the summary; the review app saved the store on exit.
    match next_action {
        Some(AfterReview::Exam(path)) => {
            let store = store_for(std::slice::from_ref(&path), args.store.clone())?;
            let deck = Deck::load(&path)?;
            run_exam_app(deck, &config, store)
        }
        Some(AfterReview::Browse(path)) => browse_one(&path, args.store.clone(), &config, None),
        None => Ok(()),
    }
}

/// The web review path. With no decks given it opens at the in-browser
/// deck-selection screen; otherwise it goes straight to review. The server
/// builds new sessions on demand (when the user picks decks) via the builder
/// closure, reusing one store and recent-decks list.
fn review_serve(args: ReviewArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let mut recent = RecentDecks::load(
        recent::default_recent_path().context("cannot determine the data directory")?,
    );
    // The session writes to the decks' own store — a workspace's `progress.json`
    // when they share one, else the global store — exactly like the TUI. The
    // server starts on the store for any CLI-named decks (else the global one)
    // and switches per selection from the in-browser picker.
    let store = store_for(&args.decks, args.store.clone())?;
    let decks_dir = config.decks_dir().context("cannot determine ~/decks")?;
    let addr = serve_addr(args.serve.port, args.serve.lan, &config);

    // Adapts a built review session to what the server holds (session + label +
    // subject→path map for removal, subject→`% link:` links for ask-Claude).
    let to_build = |b: ReviewBuild| {
        let links = b
            .decks
            .iter()
            .map(|(subject, info)| (subject.clone(), info.links.clone()))
            .collect();
        // Subject → `% source:` project root, but only for decks whose effective
        // source access is on — so the web tutor grounds exactly those.
        let source_roots = b
            .decks
            .iter()
            .filter(|(_, info)| info.source_access)
            .filter_map(|(subject, info)| {
                info.source_root.clone().map(|root| (subject.clone(), root))
            })
            .collect();
        // Subject → source base, so the web can resolve a card's `% at:` citation.
        let source_bases = b
            .decks
            .iter()
            .map(|(subject, info)| (subject.clone(), info.source_base.clone()))
            .collect();
        serve::SessionBuild {
            session: b.session,
            label: b.label,
            decks: subject_paths(b.decks),
            links,
            source_roots,
            source_bases,
            topology_name: b.topology_name,
        }
    };

    // Build the first session up front only when decks were named on the CLI;
    // otherwise start at the selection screen.
    let initial = if args.decks.is_empty() {
        None
    } else {
        let b = build_review(
            args.decks.clone(),
            &args,
            &config,
            &store,
            &mut recent,
            Frontend::Web,
        )?;
        Some(to_build(b))
    };

    let label = initial
        .as_ref()
        .map(|b| b.label.clone())
        .unwrap_or_else(|| "select decks".to_string());
    announce(addr, args.serve.lan, &label);

    let opts = serve::ReviewOptions {
        mode_override: args.mode,
        keys: config.keys.clone(),
        picker: config.picker.clone(),
        max_typos: args.max_typos,
        ask: config.ask.clone(),
        exam: config.exam.clone(),
    };
    let build = |paths: Vec<PathBuf>, store: &Store, recent: &mut RecentDecks| {
        build_review(paths, &args, &config, store, recent, Frontend::Web).map(to_build)
    };
    // A single trace picked from the in-browser picker walks (predict → verify),
    // mirroring the terminal picker; a trace named on the CLI took the `initial`
    // path above and still flattens to a card review.
    let build_walk = |paths: &[PathBuf]| -> Result<Option<serve::WalkBuild>> {
        match single_trace_to_walk(true, paths) {
            Some(deck) => {
                let scheduler = args
                    .scheduler
                    .or(deck.settings.scheduler)
                    .unwrap_or_default();
                let trace = Trace::from_deck(&deck)?;
                Ok(Some(serve::WalkBuild {
                    walk: Walk::new(trace, scheduler),
                    scheduler,
                }))
            }
            None => Ok(None),
        }
    };
    // Picks the right store for whatever decks a selection resolves to (`&[]` →
    // the global store), so the server can switch per session like the TUI.
    let store_for_sel = |paths: &[PathBuf]| store_for(paths, args.store.clone());
    // The picker's "Browse" action builds a read-only card list — the same
    // builder the standalone `alix browse` server uses.
    let to_cards = |b: BrowseBuild| serve::CardsBuild {
        cards: b.cards,
        label: b.label,
        decks: subject_paths(b.decks),
    };
    let build_browse_sel = |paths: Vec<PathBuf>, recent: &mut RecentDecks| {
        build_browse(paths, recent, Frontend::Web).map(to_cards)
    };
    serve::run_review(
        initial,
        store,
        recent,
        decks_dir,
        addr,
        opts,
        build,
        build_walk,
        build_browse_sel,
        store_for_sel,
    )
}

/// Prints where the web frontend is reachable, and a warning when it is
/// exposed to the network.
fn announce(addr: SocketAddr, lan: bool, label: &str) {
    println!("Serving {label} in the browser.");
    if lan {
        println!(
            "Listening on all interfaces, port {}. On another device open",
            addr.port()
        );
        println!("  http://<this-machine's-IP>:{}", addr.port());
        println!("warning: no authentication — anyone on your network can reach this.");
    } else {
        println!("Open http://127.0.0.1:{} in your browser.", addr.port());
    }
    println!("Press Ctrl-C to stop.");
}

fn stats(args: DeckArgs) -> Result<()> {
    let scheduler = args.scheduler.scheduler();
    let now = now_ms();

    for path in &args.decks {
        // Each deck reads its own store — a workspace deck's progress lives in the
        // workspace, not the global store.
        let store = store_for(std::slice::from_ref(path), args.store.clone())?;
        let deck = Deck::load(path)?;
        let h = histogram(&deck.cards, &store);

        let mut due_now = 0usize;
        let mut due_24h = 0usize;
        let mut reviews = 0u32;
        let mut passes = 0u32;
        for card in &deck.cards {
            if let Some(state) = store.get(card.id()) {
                // Retired cards are resting, so they don't count as due (they
                // still count toward the review totals below).
                if !alix::session::is_retired(card, &store) {
                    let due = scheduler.due_at(state);
                    if due <= now {
                        due_now += 1;
                    } else if due <= now + 86_400_000 {
                        due_24h += 1;
                    }
                }
                reviews += state.total_reviews;
                passes += state.total_passes;
            }
        }

        let state = match deck.state(&store) {
            DeckState::NotStarted => "not started",
            DeckState::Started => "in progress",
            DeckState::ExamDue => "exam due",
            DeckState::Finished if store.deck_mastered(&deck.subject) => "mastered ✓",
            DeckState::Finished => "finished ✓",
        };
        let top = alix::store::MAX_STAGE;
        let cell = |s: usize| {
            if s as u8 > top {
                "–".to_string()
            } else {
                h[s].to_string()
            }
        };
        println!("{} ({} cards)", deck.display_name(), deck.cards.len());
        println!("  state:   {state}");
        println!(
            "  stages:  new {} │ s1 {} │ s2 {} │ s3 {} │ s4 {} │ s5 {}",
            h[0],
            cell(1),
            cell(2),
            cell(3),
            cell(4),
            cell(5)
        );
        println!("  due:     {due_now} now, {due_24h} within 24h");
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
    let scheduler = args.scheduler.scheduler();
    let now = now_ms();

    for path in &args.decks {
        // Each deck reads its own store (workspace store for a workspace deck).
        let store = store_for(std::slice::from_ref(path), args.store.clone())?;
        let deck = Deck::load(path)?;
        println!("{}", deck.display_name());
        for card in &deck.cards {
            let (stage, due) = match store.get(card.id()) {
                Some(state) => {
                    // Retired cards rest until `alix reset`; their due time is
                    // moot, so say so instead of showing a misleading interval.
                    let due = if alix::session::is_retired(card, &store) {
                        "resting".to_string()
                    } else {
                        let due = scheduler.due_at(state);
                        if due <= now {
                            "due now".to_string()
                        } else {
                            format!("due in {}", humanize_ms(due - now))
                        }
                    };
                    (format!("s{}", state.stage), due)
                }
                None => ("new".to_string(), "-".to_string()),
            };
            let front: String = card.front.chars().take(60).collect();
            println!("  [{stage:>3}] {front:<60} {due}");
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

/// Asks the user to confirm a destructive action. Returns `true` when `yes` is
/// set or the user types y/yes. Errors (rather than acting silently) when there
/// is no terminal and `yes` was not given.
fn confirm(prompt: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        bail!("{prompt} (refusing without a terminal — pass --yes to proceed)");
    }
    print!("{prompt} [y/N] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_lowercase();
    Ok(answer == "y" || answer == "yes")
}

fn reset(args: ResetArgs) -> Result<()> {
    // `--all` / a numeric `--card` operate on the global store (or `--store`);
    // a deck-scoped reset re-resolves to the deck's workspace store below.
    let mut store = open_store(args.store.clone())?;

    // `--all`: wipe everything; no decks needed, count up front for the prompt.
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

    // A numeric `--card` with no decks can be removed without loading anything.
    let numeric_id = args.card.as_deref().and_then(|c| c.parse::<u64>().ok());
    if let Some(id) = numeric_id.filter(|_| args.decks.is_empty()) {
        return reset_ids(
            &mut store,
            vec![(id, String::new())],
            format!("card {id}"),
            args.card.as_deref(),
            false,
            args.yes,
        );
    }

    // Resolve decks: those named, or chosen from the picker when none are given.
    let (deck_paths, from_deck_picker) = if args.decks.is_empty() {
        if !std::io::stdout().is_terminal() {
            bail!("no deck files given; try `alix reset <deck.txt>...`, `--card <id>`, or `--all`");
        }
        let config = Config::load(None)?;
        let recent = RecentDecks::load(
            recent::default_recent_path().context("cannot determine the data directory")?,
        );
        let decks_dir = config.decks_dir().context("cannot determine ~/decks")?;
        let picked = picker::pick_to_reset(&decks_dir, &recent, &store)?;
        if picked.is_empty() {
            return Ok(()); // cancelled or nothing selected
        }
        (picked, true)
    } else {
        (args.decks.clone(), false)
    };

    // Now that the decks are known, reset against their store (the workspace's
    // own, if they all live in one).
    let mut store = store_for(&deck_paths, args.store.clone())?;

    let (cards, label, decks, _) = load_decks(&deck_paths, &HashMap::new())?;

    // A full-deck reset (no `--card`/`--cards` subset) also drops the decks'
    // "mastered" exam state, so a re-drilled sourced deck must pass its exam
    // again before it re-`Finished`es. Persisted by `reset_ids`' save below.
    if !args.cards && args.card.is_none() {
        for subject in decks.keys() {
            store.clear_deck_mastered(subject);
        }
    }

    // Choose which cards: a checkbox picker (`--cards`), a direct match
    // (`--card`), or every card in the decks.
    let (targets, from_picker): (Vec<(u64, String)>, bool) = if args.cards {
        if !std::io::stdout().is_terminal() {
            bail!("the card picker needs a terminal");
        }
        // Only cards with stored progress are worth listing.
        let rows: Vec<(u64, String, Option<String>)> = cards
            .iter()
            .filter_map(|c| {
                store.get(c.id()).map(|state| {
                    (
                        c.id(),
                        card_label(c),
                        Some(format!("s{} · {}", state.stage, short_id(c.id()))),
                    )
                })
            })
            .collect();
        if rows.is_empty() {
            println!("No stored progress to reset in {label}.");
            return Ok(());
        }
        let chosen: std::collections::HashSet<u64> =
            picker::pick_cards(rows, &format!("select cards to reset — {label}"))?
                .into_iter()
                .collect();
        if chosen.is_empty() {
            return Ok(()); // cancelled or nothing selected
        }
        let targets = cards
            .iter()
            .filter(|c| chosen.contains(&c.id()))
            .map(|c| (c.id(), c.front.clone()))
            .collect();
        (targets, true)
    } else {
        (
            select_reset_ids(&cards, args.card.as_deref()),
            from_deck_picker,
        )
    };

    reset_ids(
        &mut store,
        targets,
        label,
        args.card.as_deref(),
        from_picker,
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

/// A picker label for a card: its front, or — for cloze sub-cards — the masked
/// context, so siblings (which share a front) are distinguishable. Truncated.
fn card_label(card: &Card) -> String {
    let text = if card.context.is_empty() {
        card.front.clone()
    } else {
        card.context.join(" ")
    };
    text.chars().take(70).collect()
}

/// A shortened card id for display, e.g. `9836…4569`.
fn short_id(id: u64) -> String {
    let s = id.to_string();
    if s.len() > 9 {
        format!("{}…{}", &s[..4], &s[s.len() - 4..])
    } else {
        s
    }
}

fn browse(args: BrowseArgs) -> Result<()> {
    if args.serve.serve {
        return browse_serve(args);
    }
    if !std::io::stdout().is_terminal() {
        bail!("`alix browse` needs a terminal");
    }
    let config = Config::load(None)?;
    let mut recent = RecentDecks::load(
        recent::default_recent_path().context("cannot determine the data directory")?,
    );
    let store = open_store(None)?;
    // Explicit decks: browse once, standalone (own terminal).
    if !args.decks.is_empty() {
        let deck_store = store_for(&args.decks, None)?;
        let build = build_browse(args.decks.clone(), &mut recent, Frontend::Tui)?;
        let paths = subject_paths(build.decks);
        return browse::run(build.cards, build.label, config.browse, paths, deck_store);
    }
    // Picker flow: one shared terminal, returning to the workspace afterwards.
    let mut terminal = ratatui::init();
    let result = browse_loop(&mut terminal, &args, &config, &mut recent, &store);
    ratatui::restore();
    result
}

/// The picker browse loop: pick (or reopen a workspace), browse on the shared
/// `terminal`, and return to the workspace for the next.
fn browse_loop(
    terminal: &mut DefaultTerminal,
    args: &BrowseArgs,
    config: &Config,
    recent: &mut RecentDecks,
    store: &Store,
) -> Result<()> {
    let mut start_in: Option<PathBuf> = None;
    let mut focus: Option<PathBuf> = None;
    loop {
        // Browse is read-only traversal — locking gates review only (any deck is
        // browsable), and nothing is gated as unreviewable.
        let Some(picker::Picked {
            decks: deck_paths,
            workspace,
        }) = pick_decks_if_empty(
            Some(&mut *terminal),
            args.decks.clone(),
            config,
            recent,
            store,
            false,
            false,
            start_in.as_deref(),
            focus.as_deref(),
        )?
        else {
            return Ok(()); // picker cancelled or nothing selected
        };
        let launched = deck_paths.first().cloned();
        // Removing a card prunes its progress — from the decks' own store (a
        // workspace's when they all live in one).
        let deck_store = store_for(&deck_paths, None)?;
        let build = build_browse(deck_paths, recent, Frontend::Tui)?;
        let paths = subject_paths(build.decks);
        browse::run_on(
            terminal,
            build.cards,
            build.label,
            config.browse.clone(),
            paths,
            deck_store,
        )?;
        // Always return to the picker (only an `Esc` at the picker quits),
        // re-focused on the deck just browsed so the selection doesn't jump.
        start_in = workspace;
        focus = launched;
    }
}

/// The web browse path: opens at the in-browser deck-selection screen when no
/// decks are given, else browses them directly. New selections rebuild the card
/// list via the builder closure.
fn browse_serve(args: BrowseArgs) -> Result<()> {
    let config = Config::load(None)?;
    let mut recent = RecentDecks::load(
        recent::default_recent_path().context("cannot determine the data directory")?,
    );
    let store = open_store(None)?;
    let decks_dir = config.decks_dir().context("cannot determine ~/decks")?;
    let addr = serve_addr(args.serve.port, args.serve.lan, &config);

    let to_build = |b: BrowseBuild| serve::CardsBuild {
        cards: b.cards,
        label: b.label,
        decks: subject_paths(b.decks),
    };

    let initial = if args.decks.is_empty() {
        None
    } else {
        Some(to_build(build_browse(
            args.decks.clone(),
            &mut recent,
            Frontend::Web,
        )?))
    };

    let label = initial
        .as_ref()
        .map(|b| b.label.clone())
        .unwrap_or_else(|| "select decks".to_string());
    announce(addr, args.serve.lan, &format!("{label} (browse)"));

    let build = |paths: Vec<PathBuf>, recent: &mut RecentDecks| {
        build_browse(paths, recent, Frontend::Web).map(to_build)
    };
    serve::run_browse(
        initial,
        store,
        recent,
        decks_dir,
        addr,
        config.browse,
        config.picker,
        build,
    )
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
            let n = cache.topologies().len();
            println!(
                "({n} topolog{} stored for this deck)",
                if n == 1 { "y" } else { "ies" }
            );
            (walked, total, "a topology")
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
    cards
        .iter()
        .map(|c| augment::WarmItem {
            id: c.id(),
            question: c.front.clone(),
            answer: c.back.join("\n"),
        })
        .collect()
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

/// Collapses whitespace runs (incl. newlines) onto one line.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncates `s` to at most `max` chars, appending an ellipsis when it was cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn deck_cmd(args: GenerateDeckArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
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
    let parsed = parser::parse_str(&name, &text);

    if args.print {
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
        match &parsed {
            Ok(cards) => eprintln!("({} cards — not written; --print)", cards.len()),
            Err(e) => eprintln!("(warning: does not parse yet — {e})"),
        }
        return Ok(());
    }

    let dir = config
        .decks_dir()
        .context("cannot determine the decks directory")?;
    let path = dir.join(&name);
    if path.exists() && !args.force {
        bail!(
            "{} already exists; pass --force to overwrite",
            path.display()
        );
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let body = if text.ends_with('\n') {
        text
    } else {
        format!("{text}\n")
    };
    std::fs::write(&path, body).with_context(|| format!("cannot write {}", path.display()))?;

    match parsed {
        Ok(cards) => {
            println!("Wrote {} cards to {}", cards.len(), path.display());
            Ok(())
        }
        // Saved, but not yet valid: tell the user exactly what to fix.
        Err(e) => bail!(
            "Saved the generated deck to {}, but it does not parse yet:\n  {e}\n\
             Fix that line and run `alix check {}`.",
            path.display(),
            path.display()
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
    let parsed = parser::parse_str(&name, &text);

    if args.print {
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
        match &parsed {
            Ok(cards) => eprintln!("({} cards — not written; --print)", cards.len()),
            Err(e) => eprintln!("(warning: does not parse yet — {e})"),
        }
        return Ok(());
    }

    let dir = config
        .decks_dir()
        .context("cannot determine the decks directory")?;
    let path = dir.join(&name);
    if path.exists() && !args.force {
        bail!(
            "{} already exists; pass --force to overwrite",
            path.display()
        );
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let body = if text.ends_with('\n') {
        text
    } else {
        format!("{text}\n")
    };
    std::fs::write(&path, body).with_context(|| format!("cannot write {}", path.display()))?;

    match parsed {
        Ok(cards) => {
            println!("Imported {} cards into {}", cards.len(), path.display());
            Ok(())
        }
        // Saved, but not yet valid: tell the user exactly what to fix.
        Err(e) => bail!(
            "Saved the deck to {}, but it does not parse yet:\n  {e}\n\
             Fix that line and run `alix check {}`.",
            path.display(),
            path.display()
        ),
    }
}

fn exam_cmd(args: ExamArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let mut exam_cfg = config.exam.clone();
    if let Some(n) = args.questions {
        exam_cfg.num_questions = n;
    }
    // A deck in a workspace is examined against that workspace's own store.
    let store = store_for(std::slice::from_ref(&args.deck), args.store.clone())?;
    let deck = Deck::load(&args.deck)?;

    // Every exam needs something to verify against: a fact deck's `% source:`,
    // or a trace (whose exam is its graded compression). A source-less fact deck
    // has none.
    if !deck.has_exam() {
        bail!(
            "{} declares no `% source:` — add one (a URL or a file path) to \
             examine this deck",
            deck.subject
        );
    }
    // Exams run in dependency order: a deck with unfinished `% requires:` waits
    // until its prerequisites are mastered (pass their exams first). It need NOT
    // be drilled, though — you may test out by sitting the exam early; passing
    // masters it and unlocks its dependents.
    if alix::deck::is_locked(&deck, config.decks_dir().as_deref(), &store) {
        bail!(
            "{}'s prerequisites aren't finished yet — pass their exams first, then sit this one",
            deck.subject
        );
    }
    if !std::io::stdin().is_terminal() {
        bail!("`alix exam` needs a terminal");
    }

    // Grading strictness: CLI flag > the deck's `% strictness:` > the `[exam]`
    // default.
    let strictness = args
        .strictness
        .or(deck.settings.exam_strictness)
        .unwrap_or(config.exam.strictness);

    // A trace's exam is the compression (one fixed question = the `% trace:`),
    // graded against the path; a fact deck's exam generates questions from its
    // source.
    if deck.is_trace() {
        return run_trace_exam(&deck, &config, strictness, store);
    }
    let decks_dir = config.decks_dir();
    tui::ExamApp::new(deck, strictness, exam_cfg, config.ask, store, decks_dir).run()
}

/// Sits a **trace's exam** — the compression. One fixed question (the
/// `% trace:`), graded holistically against the path's checkpoints; a pass
/// masters the trace (unlocking its dependents) just like a fact deck. Bails if
/// the exam is cooling down after a recent fail. Shared by `alix exam <trace>`
/// (test out) and the walk's capstone. The caller has checked the deck is a
/// trace and isn't locked.
fn run_trace_exam(
    deck: &Deck,
    config: &Config,
    strictness: Strictness,
    store: Store,
) -> Result<()> {
    let trace = Trace::from_deck(deck)?;
    if let Some(remaining) = alix::exam::cooldown_remaining_ms(
        &store,
        &deck.subject,
        config.exam.retry_cooldown_secs,
        now_ms(),
    ) {
        bail!(
            "this trace exam is cooling down after a failed attempt — re-walk it \
             and try again in {}",
            humanize_ms(remaining)
        );
    }
    let sitting = alix::exam::Sitting::start_trace(
        trace.description.clone(),
        trace.compression_rubric(),
        deck.subject.clone(),
        deck.path.clone(),
        strictness,
        config.exam.clone(),
        config.ask.clone(),
    );
    tui::ExamApp::from_sitting(sitting, store, deck.path.clone(), config.decks_dir()).run()
}

/// After a full walk, offers the trace's exam (the compression) as a capstone:
/// prompts, and on a yes sits it. Skips when the exam is locked (`% requires:`
/// unmet) or cooling down after a recent fail (it just says so).
fn offer_trace_exam_capstone(deck: &Deck, config: &Config, store: Store) -> Result<()> {
    if alix::deck::is_locked(deck, config.decks_dir().as_deref(), &store) {
        return Ok(()); // its exam is gated on unfinished prerequisites
    }
    if alix::exam::cooldown_remaining_ms(
        &store,
        &deck.subject,
        config.exam.retry_cooldown_secs,
        now_ms(),
    )
    .is_some()
    {
        println!(
            "{DIM}(the trace exam is cooling down after a recent fail — re-sit it later){RESET}"
        );
        return Ok(());
    }
    println!();
    let prompt =
        format!("{DIM}Take the exam to verify you can re-derive the path? [Y/n] >{RESET} ");
    match read_line(&prompt)? {
        Some(ans) if ans.trim().eq_ignore_ascii_case("n") => Ok(()),
        None => Ok(()), // EOF (Ctrl-D): just leave
        _ => {
            let strictness = deck
                .settings
                .exam_strictness
                .unwrap_or(config.exam.strictness);
            run_trace_exam(deck, config, strictness, store)
        }
    }
}

// ANSI styling for the linear `alix trace` flow (it requires a terminal).
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn trace_cmd(args: TraceArgs) -> Result<()> {
    // `--suggest`: the positional is a SOURCE, not a deck — recon it and print a
    // menu of candidate traces. Runs before any deck load.
    if args.suggest {
        return trace_suggest(&args);
    }

    let deck = Deck::load(&args.deck)?;

    // `--build`: discover the path with Claude and write it back (no walk; a
    // fresh trace deck has no checkpoints yet, so this runs before from_deck).
    if args.build {
        return trace_build(&args, &deck);
    }

    let trace = Trace::from_deck(&deck)?;

    // `--map`: just print the path, no quiz, no terminal needed.
    if args.map {
        return print_trace_map(&trace);
    }

    let scheduler = args
        .scheduler
        .or(deck.settings.scheduler)
        .unwrap_or_default();
    // A trace in a workspace tracks its progress in that workspace's own store.
    let store = store_for(std::slice::from_ref(&args.deck), args.store.clone())?;
    let config = Config::load(args.config.as_deref())?;

    // `--serve`: walk it in the browser; otherwise in the terminal.
    if args.serve.serve {
        return trace_serve(trace, scheduler, store, &args, &config);
    }
    if !std::io::stdin().is_terminal() {
        bail!("`alix trace` needs a terminal");
    }
    let mut store = store;
    let grade = args.grade.then_some(&config);
    let completed = run_walk(trace, scheduler, &mut store, grade)?;
    // Capstone: a full walk earns the trace's exam — the compression that
    // verifies (and masters) it. Offered here, also reachable any time with
    // `alix exam <trace>`.
    if completed {
        return offer_trace_exam_capstone(&deck, &config, store);
    }
    Ok(())
}

/// `alix trace --serve`: walk a trace in the browser. Mirrors the terminal
/// walk (predict → reveal → grade each checkpoint); `--grade` enables live Claude
/// grading. One deck, one walk — no deck-selection screen. Verification (the
/// compression exam) is reached afterwards via the picker's "Take exam".
fn trace_serve(
    trace: Trace,
    scheduler: SchedulerKind,
    store: Store,
    args: &TraceArgs,
    config: &Config,
) -> Result<()> {
    let addr = serve_addr(args.serve.port, args.serve.lan, config);
    announce(addr, args.serve.lan, "a trace walk");
    let grade = args.grade.then(|| config.ask.clone());
    let walk = Walk::new(trace, scheduler);
    serve::run_walk(walk, store, addr, scheduler, grade, config.keys.clone())
}

/// Runs a trace walk in the terminal — predict → reveal → grade each checkpoint
/// — scheduling each checkpoint in `store`. The walk is the **drill**; mastery
/// is the trace's separate exam (the compression). Shared by `alix trace` and
/// `alix explore --walk`. Returns whether every checkpoint was walked (the walk
/// reached [`Phase::Done`] rather than being quit early), so the caller can offer
/// the exam as a capstone.
fn run_walk(
    trace: Trace,
    scheduler: SchedulerKind,
    store: &mut Store,
    grade: Option<&Config>,
) -> Result<bool> {
    let mut walk = Walk::new(trace, scheduler);
    let total = walk.total();
    let mut last_prediction = String::new();
    println!("{BOLD}Trace{RESET}  {}", walk.trace().description);
    if let Some(src) = &walk.trace().source {
        println!("{DIM}source: {src}  ·  {total} checkpoints{RESET}");
    }
    println!(
        "{DIM}At each hop, put down a guess before you reveal — even a hunch beats \
         \"I don't know\". The gap between your guess and the truth is the learning.{RESET}"
    );

    'walk: loop {
        match walk.phase() {
            Phase::Predict => {
                let i = walk.current_index();
                let checkpoint = walk
                    .checkpoint()
                    .cloned()
                    .expect("predict has a checkpoint");
                println!("\n{BOLD}── Checkpoint {}/{} ──{RESET}", i + 1, total);
                println!("{}", checkpoint.prompt);
                print_givens(&checkpoint.givens);
                match read_line(&format!("{DIM}predict >{RESET} "))? {
                    None => break 'walk, // EOF (Ctrl-D)
                    Some(text) => {
                        last_prediction = text.clone();
                        walk.predict(text);
                    }
                }
            }
            Phase::Reveal => {
                let checkpoint = walk.checkpoint().cloned().expect("reveal has a checkpoint");
                println!("\n{BOLD}Reveal{RESET}");
                match walk.trace().excerpt(&checkpoint) {
                    Ok(excerpt) => {
                        // A frozen-snapshot asset reads `30.rs`, lines 1-N; relabel
                        // it back to the real `caching.rs:106-120` for display.
                        let (excerpt, _) =
                            alix::trace::relabel_for_display(excerpt, checkpoint.note.as_deref());
                        print_excerpt(&excerpt);
                    }
                    Err(e) => {
                        let loc = checkpoint.locator.as_deref().unwrap_or("(none)");
                        println!("{DIM}  (couldn't read the source — {e})  at: {loc}{RESET}");
                    }
                }
                if !checkpoint.points.is_empty() {
                    println!("{BOLD}  key points{RESET}");
                    for point in &checkpoint.points {
                        println!("    • {point}");
                    }
                }
                // The provenance (`from <file>:<lines>`) is promoted into the
                // excerpt label, so drop it from the note shown here.
                if let Some(note) = alix::trace::note_without_provenance(checkpoint.note.as_deref())
                {
                    println!("{DIM}  ! {note}{RESET}");
                }
                // `--grade`: Claude judges the prediction; otherwise self-grade.
                let delta = match grade {
                    Some(config) => {
                        eprint!("{DIM}  grading…{RESET}");
                        match alix::trace::grade_prediction(
                            &checkpoint,
                            &last_prediction,
                            &config.ask,
                        ) {
                            Ok((delta, feedback)) => {
                                println!("\r{BOLD}  {}{RESET} — {feedback}", delta.label());
                                Some(delta)
                            }
                            Err(e) => {
                                println!(
                                    "\r{DIM}  (grading failed: {e} — grade it yourself){RESET}"
                                );
                                read_delta()?
                            }
                        }
                    }
                    None => read_delta()?,
                };
                match delta {
                    Some(delta) => walk.grade(store, delta, now_ms()),
                    None => break 'walk, // quit
                }
            }
            Phase::Done => break 'walk,
        }
    }

    store.save().context("cannot save progress")?;
    print_trace_summary(&walk);
    Ok(walk.phase() == Phase::Done)
}

/// Discovers the path with Claude (`alix trace --build`) and writes the
/// checkpoints back into the deck file, keeping its `% trace:`/`% source:`
/// header.
fn trace_build(args: &TraceArgs, deck: &Deck) -> Result<()> {
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
    let config = Config::load(args.config.as_deref())?;
    let source = deck.sources.first().map(String::as_str).unwrap_or_default();
    eprintln!(
        "Tracing a path through {source} (exploring the source — this can take a \
         few minutes)…"
    );
    let cards = alix::trace::build(deck, &config.trace, &config.ask)?;
    alix::deck::set_trace_checkpoints(&args.deck, &cards)?;

    let n = parser::parse_str(&deck.subject, &cards)
        .map(|c| c.len())
        .unwrap_or(0);
    let path = args.deck.display();
    println!(
        "Wrote {n} checkpoints to {path}. Review them and their `% at:` locators, \
         then walk it:  alix trace {path}"
    );
    Ok(())
}

/// `--suggest`: recon a source (a repo, directory, file, or URL — the
/// positional, NOT a deck) and print a ranked menu of candidate traces to
/// author. Read-only exploration; writes nothing. The cheap precursor to
/// `--build` — pick a suggestion, paste it into a new deck, then build that.
fn trace_suggest(args: &TraceArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let source = args.deck.to_string_lossy();
    eprintln!(
        "Reconning {source} for traces worth tracing (one exploration pass — this \
         can take a minute)…"
    );
    let menu = alix::trace::suggest(&source, &config.trace, &config.ask)?;
    println!("{menu}");
    println!(
        "\n{DIM}Paste a suggestion into a new deck (its `% trace:` + `% source:`), \
         then:  alix trace --build <deck>{RESET}"
    );
    Ok(())
}

/// `alix explore`: explore a source and print an ordered learning plan toward a
/// goal — the decks and traces worth authoring, dependency-ordered. Read-only
/// exploration; writes nothing (the first slice of `alix explore`).
fn explore_cmd(args: ExploreArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let source = args.source.to_string_lossy();
    let goal = args
        .goal
        .as_deref()
        .unwrap_or("understand the whole source");

    // `--walk`: build an explore walk over the source's shape and walk it.
    if args.walk {
        return explore_walk(&args, &config, &source, goal);
    }

    // `--into --build`: explore once, then fill every stub in the same session.
    if args.build {
        let dir = args.into.as_deref().expect("clap: --build requires --into");
        eprintln!(
            "Exploring {source} and filling the workspace toward \"{goal}\" (explore \
             + fill in one session — this can take a few minutes)…"
        );
        let (plan, filled) =
            alix::explore::explore_and_fill(&source, goal, &config.trace, &config.ask)?;
        println!("{plan}");
        let report = alix::explore::materialize(
            &plan,
            dir,
            goal,
            args.title.as_deref(),
            args.unlock_stage,
            &source,
            args.force,
            Some(&filled),
        )?;
        let total = report.traces + report.decks;
        let stubs = total - report.filled;
        println!(
            "\n{BOLD}Built {total} files{RESET} in {} — {} filled, {stubs} stub(s) \
             ({} traces, {} decks).",
            report.dir.display(),
            report.filled,
            report.traces,
            report.decks,
        );
        // Freeze each cited deck's source into the workspace's `assets/` so its
        // locators never drift and the workspace is self-contained.
        match alix::explore::snapshot_workspace(&report.dir) {
            Ok((decks, files)) if decks > 0 => println!(
                "{DIM}Froze {files} excerpt(s) from {decks} deck(s) into \
                 {}/assets — the citations won't drift.{RESET}",
                report.dir.display(),
            ),
            Ok(_) => {}
            Err(e) => eprintln!("warning: could not snapshot the source: {e:#}"),
        }
        println!(
            "{DIM}Walk a trace:  alix trace {}/<file>   ·   review the set:  \
             alix review {}{RESET}",
            report.dir.display(),
            report.dir.display(),
        );
        return Ok(());
    }

    eprintln!(
        "Exploring {source} for a learning plan toward \"{goal}\" (one pass — this \
         can take a minute)…"
    );
    let plan = alix::explore::explore(&source, goal, &config.trace, &config.ask)?;
    println!("{plan}");
    if let Some(dir) = &args.into {
        let report = alix::explore::materialize(
            &plan,
            dir,
            goal,
            args.title.as_deref(),
            args.unlock_stage,
            &source,
            args.force,
            None,
        )?;
        let total = report.traces + report.decks;
        println!(
            "\n{BOLD}Wrote {total} files{RESET} to {} — {} traces, {} decks, + alix.toml.",
            report.dir.display(),
            report.traces,
            report.decks,
        );
        println!(
            "{DIM}Build a trace:  alix trace --build {}/<file>   ·   review the set:  \
             alix review {}{RESET}",
            report.dir.display(),
            report.dir.display(),
        );
    } else {
        println!(
            "\n{DIM}Each item is a deck or trace to author next — `alix trace --build` \
             a trace, write a deck by hand or with `alix generate`.{RESET}"
        );
    }
    Ok(())
}

/// `alix explore --walk`: build an explore walk over a source's shape and walk
/// it immediately. Writes the trace to a file (default `explore.txt`) with an
/// absolute `% source:` so it re-walks from anywhere, then runs the shared walk.
fn explore_walk(args: &ExploreArgs, config: &Config, source: &str, goal: &str) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("`alix explore --walk` needs a terminal to walk");
    }
    eprintln!(
        "Exploring {source} to build an explore walk (one pass — this can take a \
         minute)…"
    );
    let checkpoints = alix::explore::walk(source, goal, &config.trace, &config.ask)?;

    // Wrap the checkpoints in a trace deck with an absolute `% source:` root so
    // the saved walk reads the right files from anywhere.
    let root = std::fs::canonicalize(&args.source).unwrap_or_else(|_| args.source.clone());
    let name = root.file_name().and_then(|n| n.to_str()).unwrap_or(source);
    let deck_text = format!(
        "% trace: exploring {name} — what it is, its parts, and its spine\n\
         % source: {}\n\n{checkpoints}\n",
        root.display()
    );
    let out = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from("explore.txt"));
    std::fs::write(&out, &deck_text).with_context(|| format!("cannot write {}", out.display()))?;
    println!(
        "{DIM}Wrote the explore walk to {} — re-walk it any time with \
         `alix trace {}`.{RESET}\n",
        out.display(),
        out.display()
    );

    let deck = Deck::load(&out)?;
    let trace = Trace::from_deck(&deck)?;
    let scheduler = deck.settings.scheduler.unwrap_or_default();
    let mut store = store_for(std::slice::from_ref(&out), None)?;
    run_walk(trace, scheduler, &mut store, None).map(|_| ())
}

/// `alix workspace <dir>`: open a workspace into its member picker. Pick a fact
/// deck → review it; pick a trace deck → walk it; back to the picker when done,
/// until you quit. Unlike `alix review <dir>`, which flattens the whole
/// workspace into one review (trace decks degrade to flat cards), this routes
/// each member to the right experience.
fn workspace_cmd(args: WorkspaceArgs) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("`alix workspace` needs a terminal");
    }
    if !workspace::is_workspace(&args.dir) {
        if workspace::has_decks(&args.dir) {
            bail!(
                "{} is a folder of decks, not a workspace — add an `alix.toml` to \
                 make it one, or `alix review {}` to review its decks.",
                args.dir.display(),
                args.dir.display(),
            );
        }
        bail!("{} is not a workspace (no `alix.toml`)", args.dir.display());
    }
    loop {
        // The picker reads the workspace's own store; review / walk re-resolve it
        // from the picked decks (`store_for`).
        let Some(picked) = picker::pick_workspace(&args.dir, true)? else {
            return Ok(()); // quit the workspace
        };
        if picked.is_empty() {
            return Ok(());
        }

        // A single trace deck is walked; anything else is a facts-deck review.
        if let [only] = picked.as_slice() {
            let deck = Deck::load(only)?;
            if deck.is_trace() {
                let trace = Trace::from_deck(&deck)?;
                let scheduler = deck.settings.scheduler.unwrap_or_default();
                let mut store = store_for(std::slice::from_ref(only), args.store.clone())?;
                run_walk(trace, scheduler, &mut store, None)?;
                continue;
            }
        }
        review(ReviewArgs {
            decks: picked,
            mode: None,
            scheduler: None,
            order: None,
            topology: None,
            new: 10,
            limit: None,
            cram: false,
            max_typos: 2,
            store: args.store.clone(),
            config: args.config.clone(),
            serve: ServeOpts {
                serve: false,
                port: None,
                lan: false,
            },
        })?;
    }
}

/// Prints a trace's path (prompts, key points, locators) without quizzing.
fn print_trace_map(trace: &Trace) -> Result<()> {
    println!("{BOLD}Trace{RESET}  {}", trace.description);
    if let Some(src) = &trace.source {
        println!("{DIM}source: {src}{RESET}");
    }
    for (i, checkpoint) in trace.checkpoints.iter().enumerate() {
        println!("\n{BOLD}{}.{RESET} {}", i + 1, checkpoint.prompt);
        for given in &checkpoint.givens {
            println!("   {DIM}given · {given}{RESET}");
        }
        for point in &checkpoint.points {
            println!("   • {point}");
        }
        if let Some(loc) = &checkpoint.locator {
            println!("   {DIM}at {loc}{RESET}");
        }
        if let Some(note) = &checkpoint.note {
            println!("   {DIM}! {note}{RESET}");
        }
    }
    Ok(())
}

/// Prints a checkpoint's `% given:` list under the question, before predicting
/// — the off-screen symbols the excerpt leans on, so it can stay tight.
fn print_givens(givens: &[String]) {
    if givens.is_empty() {
        return;
    }
    println!("{DIM}given{RESET}");
    for given in givens {
        println!("  {DIM}· {given}{RESET}");
    }
}

/// Renders an excerpt (one contiguous span) with a line-number gutter.
fn print_excerpt(excerpt: &alix::trace::Excerpt) {
    println!("{DIM}  {}{RESET}", excerpt.path.display());
    for (no, text) in &excerpt.lines {
        println!("  {DIM}{no:>5}{RESET}  {text}");
    }
    if excerpt.truncated {
        println!("       {DIM}… (truncated){RESET}");
    }
}

/// Prints the end-of-walk tally and which checkpoints came out weak.
fn print_trace_summary(walk: &Walk) {
    let s = walk.summary();
    let graded = s.got + s.partial + s.missed;
    if graded == 0 {
        println!("\n{DIM}Left the trace early — no checkpoints recorded.{RESET}");
        return;
    }
    println!(
        "\n{BOLD}Walk complete{RESET}  {} got · {} partial · {} missed",
        s.got, s.partial, s.missed
    );
    if s.weak.is_empty() {
        println!("{DIM}Every hop landed — the path will fade gently.{RESET}");
    } else {
        let hops: Vec<String> = s.weak.iter().map(|i| (i + 1).to_string()).collect();
        println!(
            "{DIM}Weak edges (resurface sooner): checkpoint {}{RESET}",
            hops.join(", ")
        );
    }
}

/// Reads one line from stdin after printing `prompt`. Returns `None` on EOF
/// (Ctrl-D), which ends the walk.
fn read_line(prompt: &str) -> Result<Option<String>> {
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line)? == 0 {
        println!();
        return Ok(None);
    }
    Ok(Some(line.trim_end().to_string()))
}

/// Prompts for the self-judged delta, re-asking until it gets `g`/`p`/`m`.
/// Returns `None` to quit (a leading `q`, or EOF).
fn read_delta() -> Result<Option<alix::trace::Delta>> {
    loop {
        let prompt = format!("{DIM}  gap?  [g]ot · [p]artial · [m]issed  (q to quit) >{RESET} ");
        let Some(answer) = read_line(&prompt)? else {
            return Ok(None);
        };
        match answer.trim().chars().next() {
            Some('q') | Some('Q') => return Ok(None),
            Some(c) => {
                if let Some(delta) = alix::trace::Delta::from_key(c) {
                    return Ok(Some(delta));
                }
            }
            None => {}
        }
        println!("{DIM}  answer g, p, or m (or q to quit).{RESET}");
    }
}

fn deps_cmd(deck_path: PathBuf) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        bail!("`alix deps` needs a terminal");
    }
    let config = Config::load(None)?;
    let decks_dir = config
        .decks_dir()
        .context("cannot determine the decks directory")?;
    let deck = Deck::load(&deck_path)?;

    let Some(selected) = picker::edit_dependencies(&decks_dir, &deck_path, &deck.requires)? else {
        return Ok(()); // cancelled — leave the file untouched
    };

    // Distinct prerequisite names, in selection order.
    let mut names: Vec<String> = Vec::new();
    for path in &selected {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let name = name.to_string();
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    alix::deck::set_requires(&deck_path, &names)?;
    if names.is_empty() {
        println!("Cleared all prerequisites of {}.", deck.subject);
    } else {
        println!(
            "Set prerequisites of {}: {}",
            deck.subject,
            names.join(", ")
        );
    }
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
        match Deck::load(path) {
            Err(e) => {
                errors += 1;
                eprintln!("error: {e}");
            }
            Ok(deck) => {
                println!("{}: {} cards", deck.subject, deck.cards.len());
                let s = &deck.settings;
                let declared: Vec<String> = [
                    s.mode.map(|m| format!("mode: {}", val_name(m))),
                    s.scheduler.map(|s| format!("scheduler: {}", val_name(s))),
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
    show("again", &keys.again);
    show("good", &keys.good);
    show("easy", &keys.easy);
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
    fn store_path_for_picks_workspace_else_global_else_override() {
        let dir = tempfile::tempdir().unwrap();
        let mk_ws = |name: &str| {
            let ws = dir.path().join(name);
            std::fs::create_dir(&ws).unwrap();
            std::fs::write(ws.join("alix.toml"), "title = \"W\"\n").unwrap();
            std::fs::write(ws.join("a.txt"), "# a\n\t1\n").unwrap();
            std::fs::write(ws.join("b.txt"), "# b\n\t1\n").unwrap();
            ws
        };
        let ws = mk_ws("ws");
        let ws2 = mk_ws("ws2");
        let ws_store = ws.join("progress.json");
        let loose = dir.path().join("loose.txt");
        std::fs::write(&loose, "# c\n\t1\n").unwrap();

        // a deck (or several) in one workspace → that workspace's store
        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("a.txt")], None)
        );
        assert_eq!(
            Some(ws_store.clone()),
            store_path_for(&[ws.join("a.txt"), ws.join("b.txt")], None)
        );
        // loose, mixed loose+workspace, and cross-workspace all → global (None)
        assert_eq!(None, store_path_for(std::slice::from_ref(&loose), None));
        assert_eq!(
            None,
            store_path_for(&[ws.join("a.txt"), loose.clone()], None)
        );
        assert_eq!(
            None,
            store_path_for(&[ws.join("a.txt"), ws2.join("a.txt")], None)
        );
        assert_eq!(None, store_path_for(&[], None));
        // --store wins over everything
        let over = dir.path().join("x.json");
        assert_eq!(
            Some(over.clone()),
            store_path_for(&[ws.join("a.txt")], Some(&over))
        );
    }

    #[test]
    fn single_trace_to_walk_only_for_a_picked_lone_trace() {
        let dir = tempfile::tempdir().unwrap();
        let trace = dir.path().join("t.txt");
        std::fs::write(
            &trace,
            "% trace: how it works\n% source: .\n\n# q\n\tpoint\n\t% at: 1\n",
        )
        .unwrap();
        let fact = dir.path().join("f.txt");
        std::fs::write(&fact, "# q\n\ta\n").unwrap();

        // A lone trace picked interactively → walk it.
        assert!(single_trace_to_walk(true, std::slice::from_ref(&trace)).is_some());
        // The same trace named explicitly (not from the picker) → still review.
        assert!(single_trace_to_walk(false, std::slice::from_ref(&trace)).is_none());
        // A lone facts deck → review, not walk.
        assert!(single_trace_to_walk(true, std::slice::from_ref(&fact)).is_none());
        // A trace alongside other decks isn't a lone trace → review/merge.
        assert!(single_trace_to_walk(true, &[trace, fact]).is_none());
    }

    #[test]
    fn build_browse_loads_from_explicit_paths_and_filters_frontend() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        // A normal card and a web-only image card.
        std::fs::write(
            &path,
            "% img-dir: /imgs\n# plain\n\tanswer\n# pic\n% img: a.png\n\tphoto\n",
        )
        .unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));

        // Tui target drops the image card; Web target keeps both.
        let tui = build_browse(vec![path.clone()], &mut recent, Frontend::Tui).unwrap();
        assert_eq!(1, tui.cards.len());
        assert_eq!("plain", tui.cards[0].front);

        let web = build_browse(vec![path], &mut recent, Frontend::Web).unwrap();
        assert_eq!(2, web.cards.len());
    }

    #[test]
    fn reset_selects_all_without_a_filter() {
        let cards = vec![card("A", "1"), card("B", "2")];
        assert_eq!(2, select_reset_ids(&cards, None).len());
    }

    #[test]
    fn expand_workspaces_member_file_inherits_workspace_settings() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
        std::fs::write(ws.join("alix.toml"), "[defaults]\ndirection = \"both\"\n").unwrap();

        // A member picked as a bare file (a subset selection) still inherits the
        // workspace's directives.
        let exp = expand_workspaces(&[ws.join("a.txt")]).unwrap();
        assert_eq!(1, exp.decks.len());
        assert_eq!(
            Some(alix::card::Direction::Both),
            exp.defaults.get("a.txt").unwrap().direction
        );
        assert!(exp.label.is_none()); // not a single-workspace request
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
