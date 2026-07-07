use std::{
    collections::HashMap,
    io::{IsTerminal, Write},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use alix::{
    augment::{self, AugmentCache, Topology, TopologyOrder},
    card::Card,
    config::{self, Config},
    deck::{Deck, DeckSettings, DeckState},
    generate, import,
    level::Level,
    parser, preflight,
    recent::{self, RecentDecks},
    scheduler::{Fsrs, Scheduler},
    serve,
    session::{DeckInfo, Order, Session, SessionOptions},
    store::{Store, VirtualCard, default_store_path},
    time::{humanize_ms, now_ms},
    trace::{Phase, SourceBase, Trace, Walk},
    workspace,
};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

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
    /// List all cards of decks with their state and due time.
    List(DeckArgs),
    /// Clear stored progress for decks, a single card, or everything.
    Reset(ResetArgs),
    /// Create, augment, or validate decks.
    #[command(subcommand)]
    Deck(DeckAction),
    /// Import an Anki TSV export (tab-separated `front<TAB>back` lines) into a
    /// alix deck.
    Import(ImportArgs),
    /// Walk a trace: a predict-and-verify path through a `% source:` that
    /// builds understanding. At each checkpoint you predict, then the real
    /// excerpt is revealed and you judge the gap; the path ends with a
    /// compression.
    Trace(TraceArgs),
    /// Explore a source (a repo, directory, file, or URL) and print an ordered
    /// learning plan toward a goal: the facts decks and traces worth authoring,
    /// each tagged and dependency-ordered. Read-only; writes nothing.
    Explore(ExploreArgs),
    /// Open a workspace folder in the browser: its decks and traces, reached
    /// through the web picker.
    Workspace(WorkspaceArgs),
    /// Show the configuration (key bindings) or create the config file.
    Config {
        /// Write a config file with the default bindings to edit.
        #[arg(long)]
        init: bool,
    },
    /// Probe or inspect AI backends.
    #[command(subcommand)]
    Backend(BackendAction),
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

    /// With --into, write into the directory even if it already contains files.
    #[arg(long, requires = "into")]
    force: bool,

    /// With --into, fill every stub in one explore session — checkpoints for
    /// traces, cards for facts decks — instead of leaving them empty. One coherent
    /// pass (the items know about each other); more model work.
    #[arg(long, requires = "into", conflicts_with = "walk")]
    build: bool,

    /// With --build, use this image file as the workspace icon instead of letting
    /// Claude draw one. Copied into the workspace's `assets/`. SVG or raster.
    #[arg(long, requires = "build")]
    icon: Option<PathBuf>,

    /// Build an explore walk instead of a plan: a predict-verify trace over the
    /// source's shape (what it is → its parts → entry → spine → what to trace),
    /// written to a file and walked right away.
    #[arg(long, conflicts_with = "into")]
    walk: bool,

    /// With --walk, the file to write the explore walk to (default explore.txt).
    #[arg(short, long, requires = "walk")]
    output: Option<PathBuf>,

    /// Skip the pre-flight size confirmation for a large local source tree.
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
    /// Generate a facts deck with Claude from a source — a web page URL or a
    /// local file/directory path. (The deck-side mirror of `alix trace`.)
    Generate(GenerateDeckArgs),
    /// Augment an existing deck with Claude — multiple-choice distractors, or
    /// trivia notes. Augmentations are deliberate and persisted, so review stays
    /// instant and fully offline.
    Augment(AugmentArgs),
    /// Check deck files for syntax errors and duplicate cards.
    Check {
        /// Deck files to check.
        #[arg(required = true)]
        decks: Vec<PathBuf>,
    },
}

/// The `alix backend` subcommands: inspect or probe AI backends.
#[derive(Subcommand)]
enum BackendAction {
    /// Probe the configured backend (or all four with `--all`): send a trivial
    /// request and report whether it is installed, signed in, and responding.
    /// This makes a real (tiny) AI call — the only reliable way to confirm the
    /// whole path works end-to-end.
    Check {
        /// Probe all four supported backends instead of the configured one.
        #[arg(short, long)]
        all: bool,

        /// Path of the config file (default: platform config dir).
        #[arg(long)]
        config: Option<PathBuf>,
    },
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

    /// Skip the pre-flight size confirmation for a large local source tree
    /// (applies to --build and --suggest).
    #[arg(short, long)]
    yes: bool,

    /// Path of the progress store (default: platform data dir).
    #[arg(long)]
    store: Option<PathBuf>,

    /// Path of the config file (default: platform config dir).
    #[arg(long)]
    config: Option<PathBuf>,
}

/// Options for serving the review activity in the browser. Flattened into
/// `review`. `--port`/`--lan` require `--serve`, so they cannot be given
/// without it.
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

    /// Pairing token required on `/api/*` when serving to the network. Defaults
    /// to a value auto-generated (and printed) for `--lan`.
    #[arg(long, requires = "serve")]
    token: Option<String>,
}

#[derive(Args)]
struct DeckArgs {
    /// Deck files.
    #[arg(required = true)]
    decks: Vec<PathBuf>,

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

    /// Order cards are shown in. Overrides a deck's `% order:` directive;
    /// defaults to scheduled.
    #[arg(short, long, value_enum)]
    order: Option<Order>,

    /// Reorder the due set by a stored AI topology of this name (see `alix deck
    /// augment --target topology`). With no name, a deck's single cached topology
    /// is used automatically.
    #[arg(long)]
    topology: Option<String>,

    /// Focus the session on one topology region (e.g. "persistence") — only that
    /// region's cards are reviewed, to drill a weak area. Needs a topology.
    #[arg(long)]
    region: Option<String>,

    /// Maximum number of new (never-seen) cards to introduce.
    #[arg(short, long, default_value_t = 10)]
    new: usize,

    /// Maximum number of cards in this session.
    #[arg(short, long)]
    limit: Option<usize>,

    /// Ignore due times and review all previously seen cards.
    #[arg(long)]
    cram: bool,

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
        Some(Command::Deck(action)) => match action {
            DeckAction::Generate(args) => deck_cmd(args),
            DeckAction::Augment(args) => augment_cmd(args),
            DeckAction::Check { decks } => check(decks),
        },
        Some(Command::Import(args)) => import_cmd(args),
        Some(Command::Trace(args)) => trace_cmd(args),
        Some(Command::Explore(args)) => explore_cmd(args),
        Some(Command::Workspace(args)) => workspace_cmd(args),
        Some(Command::Config { init }) => config_cmd(init),
        Some(Command::Backend(action)) => match action {
            BackendAction::Check { all, config } => backend_check_cmd(all, config),
        },
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
    std::collections::HashMap<String, DeckInfo>,
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
            DeckInfo {
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

/// The result of [`expand_workspaces`]: the deck file(s) to load and the per-deck
/// workspace directive defaults (keyed by file name).
struct Expanded {
    decks: Vec<PathBuf>,
    defaults: HashMap<String, DeckSettings>,
}

/// Resolves each deck file's workspace context: a member file whose parent folder
/// is a workspace inherits that workspace's shared directive defaults (keyed by
/// file name); plain files pass through untagged. A review/browse target is a
/// single deck *file* (whole-workspace review was removed), so this no longer
/// expands a folder — it just tags the file with its workspace's directives.
fn expand_workspaces(deck_paths: &[PathBuf]) -> Result<Expanded> {
    let mut decks = Vec::new();
    let mut defaults: HashMap<String, DeckSettings> = HashMap::new();
    for path in deck_paths {
        // A deck file inside a workspace folder inherits its shared directives.
        if let Some(parent) = path.parent()
            && parent.join(workspace::MANIFEST).is_file()
            && let Ok(ws) = workspace::Workspace::load(parent)
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            defaults.insert(name.to_string(), ws.settings);
        }
        decks.push(path.clone());
    }
    Ok(Expanded { decks, defaults })
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

/// A review session built from an explicit set of deck paths, ready for the web
/// frontend. Produced by [`build_review`] and consumed by [`review_serve`].
struct ReviewBuild {
    session: Session,
    label: String,
    decks: HashMap<String, DeckInfo>,
    /// The resolved topology's name, if the session is topology-ordered, so a
    /// frontend can fetch it from the augment cache to show the connective cue.
    topology_name: Option<String>,
}

/// Base line number for a synthesized virtual card ([`synthesize_virtual`]) —
/// far past any real deck's line count, so a virtual card's `line` never
/// collides with (and so never shares a sibling group with) a real card's
/// front line.
const VIRTUAL_LINE_BASE: usize = 1_000_000;

/// Synthesizes a virtual card's stored deck-format `text` into the real `Card`
/// it stands for — the one in `parse(vc.parent, vc.text)` whose `Card::id`
/// matches `vc.id` (a cloze block yields several sub-cards; the id picks the
/// right hole). `subject` MUST equal `vc.parent`, or the id won't reproduce
/// (`Card::id` hashes the subject). `line` places it far past any real deck
/// line so it never shares a sibling group with a deck card — id-neutral, since
/// `Card::id` ignores `line`. Returns `None` if the text can't be parsed or no
/// card matches (defensive — impossible in practice, but no `unwrap` here).
fn synthesize_virtual(vc: &VirtualCard, subject: &Arc<str>, line: usize) -> Option<Card> {
    let mut card = parser::parse_str(subject, &vc.text)
        .ok()?
        .into_iter()
        .find(|c| c.id() == vc.id)?;
    card.line = line;
    Some(card)
}

/// Builds a review session from explicit `deck_paths` (no interactive picker):
/// resolves `% requires:` prerequisites, applies deck directives, builds the
/// `Session`, and records the decks as recent. The store is borrowed (the
/// caller owns it), so the web server can reuse one store across repeated
/// selections.
fn build_review(
    deck_paths: Vec<PathBuf>,
    args: &ReviewArgs,
    config: &Config,
    store: &Store,
    recent: &mut RecentDecks,
    // Topology + region focus, resolved here rather than read from `args` so the
    // web picker's focus drawer can override them per-launch (the CLI passes
    // `args.topology` / `args.region`).
    topology_sel: Option<&str>,
    region_sel: Option<&str>,
) -> Result<ReviewBuild> {
    // A session is exactly one deck file's cards — no merging of several loose
    // decks, and no reviewing a whole workspace at once. Workspaces are an
    // organizing layer: review their members one at a time (the picker drills in;
    // `alix workspace <dir>` opens that picker).
    let [deck] = deck_paths.as_slice() else {
        bail!("review one deck at a time (merging decks was removed)");
    };
    if workspace::has_decks(deck) {
        bail!(
            "`{}` is a workspace — review a deck inside it, or open it with `alix workspace`",
            deck.display()
        );
    }
    // Resolve the deck's workspace context (a member file inherits its workspace's
    // shared directives). `% requires:` prerequisites are NOT pulled in — the
    // dependency graph gates exams, not what a review session contains.
    let expanded = expand_workspaces(&deck_paths)?;
    let (mut cards, deck_label, mut decks, settings) =
        load_decks(&expanded.decks, &expanded.defaults)?;
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
    // One deck per session, so the label is the deck's own subject.
    let label = deck_label;

    // Every card id in this deck — used to pick out *this* deck's topologies from
    // a cache that may be shared with other decks (one store).
    let deck_ids: std::collections::HashSet<u64> = cards.iter().map(|c| c.id()).collect();

    // Merge in any AI-generated notes from the sidecar cache (`alix deck augment
    // --target notes`) — shown with the card's own deck note on reveal. (Question
    // variants are rotated in per-presentation by the frontends, and distractors
    // are read when a choice question is built.)
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for card in &mut cards {
        // Reshape first (re-renders the deck note, front, answer, mode) …
        augment.apply_format(card);
        // … then stack the notes-target trivia on top of the reshaped note.
        if let Some(note) = augment.note(card.id()) {
            card.append_note(&[note.to_string()]);
        }
    }

    // Resolve the topology that reorders this session (if any) and project it to
    // a session-ready order. The resolved name travels on `ReviewBuild` so the
    // web frontend can show the "why this card follows the last" cue from the
    // same topology.
    let topology = resolve_topology(topology_sel, &augment, &deck_ids)?;
    let topology_name = topology.map(|t| t.name.clone());
    let topology_order = topology.map(|t| TopologyOrder::from_walk(&t.walk));

    // `--region` focuses the session on one region of the topology — drill a
    // weak area. SRS still picks what's due *within* that region.
    if let Some(region_name) = region_sel {
        let Some(topology) = topology else {
            bail!("--region needs a topology — pass --topology, or augment one for this deck");
        };
        let Some(region_ids) = topology.region_cards(region_name) else {
            bail!(
                "no region named `{region_name}` in topology `{}`",
                topology.name
            );
        };
        let ids: std::collections::HashSet<u64> = region_ids.iter().copied().collect();
        cards.retain(|c| ids.contains(&c.id()));
    }

    // A workspace member drills under that workspace's `alix.local.toml` pacing
    // override (retention + retirement), else the global `[review]` config.
    let review = config
        .review
        .for_workspace(deck.parent().unwrap_or_else(|| Path::new("")));

    // Inject this deck's virtual (remediation) cards alongside its authored
    // ones, so both are drilled by the same FSRS-due queue — but not under a
    // `--region` focus: a region is a deck-topology drill, and virtual cards
    // aren't part of any topology. `decks` has exactly this one deck's entry
    // (one deck per session), keyed by its subject — the same string a
    // virtual card's `parent` is set to.
    let subject: Arc<str> = decks
        .keys()
        .next()
        .map(|s| Arc::from(s.as_str()))
        .unwrap_or_else(|| Arc::from(label.as_str()));
    if region_sel.is_none() {
        for (k, vc) in store
            .virtual_cards_for(subject.as_ref())
            .into_iter()
            .filter(|v| !alix::session::is_retired_id(v.id, store, review.retire_after_days))
            .filter(|v| !deck_ids.contains(&v.id)) // collision belt-and-suspenders
            .enumerate()
        {
            if let Some(mut card) = synthesize_virtual(vc, &subject, VIRTUAL_LINE_BASE + k) {
                // Reshape/note a synth card exactly as deck cards are above
                // (§8.1) — this loop runs after that one, so it must repeat the
                // same two steps rather than widening the earlier loop's range.
                augment.apply_format(&mut card);
                if let Some(note) = augment.note(card.id()) {
                    card.append_note(&[note.to_string()]);
                }
                cards.push(card);
            }
        }
    }

    // Directives (order) come from the session's decks.
    let target_settings: Vec<&DeckSettings> = settings.iter().collect();

    // `order` is deck/session-level: CLI flag > deck directive > default. `mode`
    // is now per-card (resolved at review time from the card's own `% mode:`), so
    // only the CLI override is carried here.
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
        retire_after_days: review.retire_after_days,
        // TODO(task 8): the config depth dial is gone (Task 4) — this stopgap
        // always drills at Recall until session-level selection (which level
        // a CLI/web session runs at) is wired up.
        level: Level::Recall,
    };
    let session = Session::new(
        cards,
        store,
        Box::new(Fsrs::new(review.retention)),
        options,
        now_ms(),
    );

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
    deck_ids: &std::collections::HashSet<u64>,
) -> Result<Option<&'a Topology>> {
    // Only this deck's topologies — a shared cache (decks sharing a store) holds
    // others', which must not be auto-applied or named here.
    let mine = augment.topologies_for(deck_ids);
    match name {
        Some(name) => match mine.into_iter().find(|t| t.name == name) {
            Some(topology) => Ok(Some(topology)),
            None => bail!(
                "no topology named `{name}` is cached for this deck — run `alix deck augment <deck> --target topology`"
            ),
        },
        None => Ok(match mine.as_slice() {
            [single] => Some(*single),
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

/// Builds the browse card list from explicit `deck_paths` (no picker). Mirrors
/// [`build_review`] for the read-only browse view: loads decks, but builds no
/// scheduler session.
fn build_browse(deck_paths: Vec<PathBuf>, recent: &mut RecentDecks) -> Result<BrowseBuild> {
    // One deck file per browse — no merging loose decks or whole workspaces.
    let [deck] = deck_paths.as_slice() else {
        bail!("browse one deck at a time (merging decks was removed)");
    };
    if workspace::has_decks(deck) {
        bail!(
            "`{}` is a workspace — browse a deck inside it, or open it with `alix workspace`",
            deck.display()
        );
    }
    let expanded = expand_workspaces(&deck_paths)?;
    let (mut cards, deck_label, decks, _) = load_decks(&expanded.decks, &expanded.defaults)?;
    let label = deck_label;

    // Merge in the display augmentations review shows, from the decks' own store
    // (a workspace's when they share one) — so browse renders the same view, not
    // the raw deck. The raw card stays in the deck file; this is display-only.
    let store = store_for(&expanded.decks, None)?;
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for card in &mut cards {
        // Reshape first (re-renders the deck note, front, answer, mode) …
        augment.apply_format(card);
        // … then stack the notes-target trivia on top of the reshaped note.
        if let Some(note) = augment.note(card.id()) {
            card.append_note(&[note.to_string()]);
        }
    }

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
    decks: HashMap<String, DeckInfo>,
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

/// Resolves the serve pairing token: an explicit `--token` or `[serve] token`
/// wins; otherwise `--lan` generates one. Localhost (no `--lan`) stays open
/// (`None`). Fails **closed** — if `--lan` needs a token but generation fails,
/// this errors rather than leaving the network API open.
fn resolve_serve_token(cli: Option<String>, lan: bool, config: &Config) -> Result<Option<String>> {
    if let Some(t) = cli
        .or_else(|| config.serve.token.clone())
        .filter(|t| !t.is_empty())
    {
        return Ok(Some(t));
    }
    if lan {
        return Ok(Some(generate_token()?));
    }
    Ok(None)
}

/// A cryptographically secure random pairing token (16 bytes, hex), drawn from
/// the OS CSPRNG via `getrandom` (portable across Linux/macOS/Windows).
fn generate_token() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)
        .map_err(|e| anyhow::anyhow!("could not generate a serve pairing token: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Subject → deck file path, for the web frontend's card removal.
fn subject_paths(decks: HashMap<String, DeckInfo>) -> HashMap<String, PathBuf> {
    decks
        .into_iter()
        .map(|(subject, info)| (subject, info.path))
        .collect()
}

/// Reviewing always runs in the browser (alix is web-first): bare `alix` opens
/// the in-browser deck picker, and `alix <deck>` goes straight to that deck's
/// web review. The `--serve`/`--port`/`--lan` flags stay meaningful (LAN,
/// custom port); either way this routes to the local web server, which prints
/// its URL. A single trace still walks and a single exam-due deck still opens
/// its exam — both via the same web app.
fn review(args: ReviewArgs) -> Result<()> {
    review_serve(args, false)
}

/// The web review path. With no decks given it opens at the in-browser
/// deck-selection screen; otherwise it goes straight to review. The server
/// builds new sessions on demand (when the user picks decks) via the builder
/// closure, reusing one store and recent-decks list.
/// Serves the unified web app. `browse_mode` makes a CLI-named deck open
/// directly in the read-only browse overlay (the `alix browse --serve` entry)
/// rather than a review session; the in-browser picker is identical either way.
fn review_serve(args: ReviewArgs, browse_mode: bool) -> Result<()> {
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

    // The read-only browse card builder, reused for a CLI browse launch and the
    // picker's "Browse" action (same builder, one source of truth).
    let to_cards = |b: BrowseBuild| serve::CardsBuild {
        cards: b.cards,
        label: b.label,
        decks: subject_paths(b.decks),
    };
    // What the server opens on. Decks named on the CLI: a read-only browse list
    // when `browse_mode`, else a review session built up front. None: the picker.
    let (initial, label) = if args.decks.is_empty() {
        (serve::Launch::Picker, "select decks".to_string())
    } else if browse_mode {
        let cards = to_cards(build_browse(args.decks.clone(), &mut recent)?);
        let label = format!("{} (browse)", cards.label);
        (serve::Launch::Browse(cards), label)
    } else {
        let b = to_build(build_review(
            args.decks.clone(),
            &args,
            &config,
            &store,
            &mut recent,
            args.topology.as_deref(),
            args.region.as_deref(),
        )?);
        let label = b.label.clone();
        (serve::Launch::Review(Box::new(b)), label)
    };
    let token = resolve_serve_token(args.serve.token.clone(), args.serve.lan, &config)?;
    announce(addr, args.serve.lan, token.as_deref(), &label);

    let opts = serve::ReviewOptions {
        keys: config.keys.clone(),
        picker: config.picker.clone(),
        browse: config.browse.clone(),
        review: config.review,
        ask: config.ask.clone(),
        exam: config.exam.clone(),
        ai: config.ai.clone(),
        auth: token,
    };
    let build = |paths: Vec<PathBuf>,
                 topology: Option<&str>,
                 region: Option<&str>,
                 store: &Store,
                 recent: &mut RecentDecks| {
        build_review(paths, &args, &config, store, recent, topology, region).map(to_build)
    };
    // A single trace picked from the in-browser picker walks (predict → verify),
    // mirroring the terminal picker; a trace named on the CLI took the `initial`
    // path above and still flattens to a card review.
    let build_walk = |paths: &[PathBuf]| -> Result<Option<serve::WalkBuild>> {
        match single_trace_to_walk(true, paths) {
            Some(deck) => {
                let trace = Trace::from_deck(&deck)?;
                Ok(Some(serve::WalkBuild {
                    walk: Walk::new(trace),
                }))
            }
            None => Ok(None),
        }
    };
    // Picks the right store for whatever decks a selection resolves to (`&[]` →
    // the global store), so the server can switch per session like the TUI.
    let store_for_sel = |paths: &[PathBuf]| store_for(paths, args.store.clone());
    let build_browse_sel =
        |paths: Vec<PathBuf>, recent: &mut RecentDecks| build_browse(paths, recent).map(to_cards);
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

/// Prints where the web frontend is reachable, plus pairing info (host/port/
/// token) when it is exposed to the network with a token — or a warning when
/// exposed without one.
fn announce(addr: SocketAddr, lan: bool, token: Option<&str>, label: &str) {
    println!("Serving {label} in the browser.");
    match (lan, token) {
        (true, Some(t)) => {
            let port = addr.port();
            println!("On another device, open in a browser:");
            println!("  http://<this-machine's-IP>:{port}/?token={t}");
            println!("Or pair the app with:  host <this-machine's-IP>  port {port}  token {t}");
        }
        (true, None) => {
            println!("Listening on all interfaces, port {}.", addr.port());
            println!("warning: no authentication — anyone on your network can reach this.");
        }
        (false, _) => println!("Open http://127.0.0.1:{} in your browser.", addr.port()),
    }
    println!("Press Ctrl-C to stop.");
}

fn stats(args: DeckArgs) -> Result<()> {
    let config = Config::load(None)?;
    let now = now_ms();

    for path in &args.decks {
        // Each deck reads its own store — a workspace deck's progress lives in the
        // workspace, not the global store.
        let store = store_for(std::slice::from_ref(path), args.store.clone())?;
        let deck = Deck::load(path)?;
        // …and its own pacing: a workspace deck honors its `alix.local.toml`.
        let review = config
            .review
            .for_workspace(path.parent().unwrap_or_else(|| Path::new("")));
        let scheduler = Fsrs::new(review.retention);

        let mut due_now = 0usize;
        let mut due_24h = 0usize;
        let mut reviews = 0u32;
        let mut passes = 0u32;
        for card in &deck.cards {
            if let Some(state) = store.get(card.id()) {
                // Retired cards are resting, so they don't count as due (they
                // still count toward the review totals below).
                if !alix::session::is_retired(card, &store, review.retire_after_days) {
                    let due = scheduler.due_at(state, Level::Recall);
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
        // Virtual (remediation) cards count toward "due" (now and within
        // 24h), never toward the card count below — they aren't deck content.
        due_now += alix::session::count_reviewable_virtual(
            &store,
            &deck.subject,
            &scheduler,
            now,
            review.retire_after_days,
        );
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

    for path in &args.decks {
        // Each deck reads its own store (workspace store for a workspace deck).
        let store = store_for(std::slice::from_ref(path), args.store.clone())?;
        let deck = Deck::load(path)?;
        // …and its own pacing (workspace `alix.local.toml` override).
        let review = config
            .review
            .for_workspace(path.parent().unwrap_or_else(|| Path::new("")));
        let scheduler = Fsrs::new(review.retention);
        println!("{}", deck.display_name());
        for card in &deck.cards {
            let (label, due) = match store.get(card.id()) {
                Some(state) => {
                    // Retired cards rest until `alix reset`; their due time is
                    // moot, so say so instead of showing a misleading interval.
                    let due = if alix::session::is_retired(card, &store, review.retire_after_days) {
                        "resting".to_string()
                    } else {
                        let due = scheduler.due_at(state, Level::Recall);
                        if due <= now {
                            "due now".to_string()
                        } else {
                            format!("due in {}", humanize_ms(due - now))
                        }
                    };
                    let label = match state.recall.as_ref().map(|f| f.state) {
                        Some(1) => "learning",
                        Some(2) => "review",
                        Some(3) => "relearning",
                        _ => "new",
                    };
                    (label.to_string(), due)
                }
                None => ("new".to_string(), "-".to_string()),
            };
            let front: String = card.front.chars().take(60).collect();
            println!("  [{label:>10}] {front:<60} {due}");
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

/// Returns `true` if `source` looks like an HTTP/HTTPS URL.
fn is_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

/// Runs the pre-flight size guard for agentic commands that hand a local
/// source tree to the model. If the tree is oversized and `yes` is false,
/// either asks for interactive confirmation (when a TTY is available) or bails
/// (no TTY). Does nothing when the source is a URL or when the threshold is 0.
fn preflight_source(source: &str, threshold: u64, yes: bool) -> Result<()> {
    // URLs are measured server-side (WebFetch); only local paths need a guard.
    if is_url(source) || threshold == 0 {
        return Ok(());
    }
    let path = std::path::Path::new(source);
    if !path.exists() {
        return Ok(());
    }
    let size = preflight::tree_size(path);
    if !preflight::is_oversized(size.bytes, threshold) {
        return Ok(());
    }
    let msg = format!(
        "source tree is {} files / {} — this may be a large model call",
        size.files,
        size.human_bytes()
    );
    if yes {
        eprintln!("warning: {msg}; proceeding (--yes)");
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        bail!(
            "large source tree ({} files / {}); pass --yes to proceed",
            size.files,
            size.human_bytes()
        );
    }
    print!("{msg}. Proceed? [y/N] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_lowercase();
    if answer != "y" && answer != "yes" {
        bail!("aborted by user");
    }
    Ok(())
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

    // Otherwise a reset needs an explicit target — there is no interactive
    // deck picker. Name a deck (optionally with `--card`), or pass `--all`.
    if args.decks.is_empty() {
        bail!("name a deck to reset, or pass `--card <id>` or `--all`");
    }
    let deck_paths = args.decks.clone();

    // Reset against the decks' store (the workspace's own, if they all live in
    // one).
    let mut store = store_for(&deck_paths, args.store.clone())?;

    let (cards, label, decks, _) = load_decks(&deck_paths, &HashMap::new())?;

    // A full-deck reset (no `--card` subset) resets authored-card progress, the
    // decks' "mastered" exam flag, and their virtual (remediation) cards
    // together, atomically under one confirmation — a declined/failed prompt
    // must leave the store on disk untouched by any of it (not just the
    // authored-card part).
    if args.card.is_none() {
        let present: Vec<(u64, String)> = select_reset_ids(&cards, None)
            .into_iter()
            .filter(|(id, _)| store.get(*id).is_some())
            .collect();
        let mastered = decks.keys().any(|subject| store.deck_mastered(subject));
        // A virtual card's content is in the sidecar and its schedule in
        // `store.cards` (both keyed by the same id) — a reset drops both.
        let virtual_ids: Vec<u64> = decks
            .keys()
            .flat_map(|subject| {
                store
                    .virtual_cards_for(subject)
                    .iter()
                    .map(|vc| vc.id)
                    .collect::<Vec<_>>()
            })
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

        for subject in decks.keys() {
            store.clear_deck_mastered(subject);
        }
        for id in &virtual_ids {
            store.remove_virtual(*id); // drop sidecar content …
            store.remove(*id); // … and the schedule now in `store.cards`
        }
        for (id, _) in &present {
            store.remove(*id);
        }
        store.save()?;
        println!("Reset {n} card(s).");
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

    // A trace in a workspace tracks its progress in that workspace's own store.
    let store = store_for(std::slice::from_ref(&args.deck), args.store.clone())?;
    let config = Config::load(args.config.as_deref())?;

    if !std::io::stdin().is_terminal() {
        bail!("`alix trace` needs a terminal");
    }
    let mut store = store;
    let grade = args.grade.then_some(&config);
    let completed = run_walk(trace, &mut store, grade)?;
    // A full walk earns the trace's exam — the compression that verifies (and
    // masters) it. The exam is sat in the browser: run `alix` and pick this
    // trace to take it.
    if completed {
        println!(
            "{DIM}Walk complete — take this trace's exam in the browser: run `alix` \
             and pick it.{RESET}"
        );
    }
    Ok(())
}

/// Runs a trace walk in the terminal — predict → reveal → grade each checkpoint
/// — scheduling each checkpoint in `store`. The walk is the **drill**; mastery
/// is the trace's separate exam (the compression). Shared by `alix trace` and
/// `alix explore --walk`. Returns whether every checkpoint was walked (the walk
/// reached [`Phase::Done`] rather than being quit early), so the caller can offer
/// the exam as a capstone.
fn run_walk(trace: Trace, store: &mut Store, grade: Option<&Config>) -> Result<bool> {
    let mut walk = Walk::new(trace);
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
                        let (excerpt, _) = alix::trace::relabel_for_display(
                            excerpt,
                            checkpoint.at_origin.as_deref(),
                        );
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
                // The note is the learner's own (provenance now rides the `% at:`
                // line), so show it as-is.
                if let Some(note) = &checkpoint.note {
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
    preflight_source(source, config.ask.preflight_threshold, args.yes)?;
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
    preflight_source(&source, config.ask.preflight_threshold, args.yes)?;
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

    preflight_source(&source, config.ask.preflight_threshold, args.yes)?;

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
            Ok(summary) => {
                if summary.decks > 0 {
                    println!(
                        "{DIM}Froze {} excerpt(s) from {} deck(s) into {}/assets — \
                         the citations won't drift.{RESET}",
                        summary.files,
                        summary.decks,
                        report.dir.display(),
                    );
                }
                // A cited deck that froze nothing is a broken/stale `% source:` —
                // surface it so the empty `assets/` isn't a silent mystery.
                for failed in &summary.failed {
                    eprintln!("warning: could not freeze {failed}");
                }
            }
            Err(e) => eprintln!("warning: could not snapshot the source: {e:#}"),
        }
        // A workspace icon: the user's file if given, else an abstract emblem the
        // model draws from what it just built. Best-effort — never fails the build.
        match args.icon.as_deref() {
            Some(src) => match alix::icon::install(&report.dir, src) {
                Ok(_) => println!(
                    "{DIM}Installed the workspace icon into {}/assets.{RESET}",
                    report.dir.display()
                ),
                Err(e) => eprintln!("warning: could not install the workspace icon: {e:#}"),
            },
            None => match alix::icon::generate(&report.dir, &config.ask) {
                Ok(_) => println!(
                    "{DIM}Drew a workspace icon into {}/assets.{RESET}",
                    report.dir.display()
                ),
                Err(e) => eprintln!("warning: could not draw a workspace icon: {e:#}"),
            },
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
    let mut store = store_for(std::slice::from_ref(&out), None)?;
    run_walk(trace, &mut store, None).map(|_| ())
}

/// `alix workspace <dir>`: open a workspace in the browser. alix is web-first,
/// so this validates the folder is a workspace (has an `alix.toml`) and then
/// serves the web app; its decks and traces are reached through the in-browser
/// picker (which routes a facts deck to review and a trace to a walk).
fn workspace_cmd(args: WorkspaceArgs) -> Result<()> {
    if !workspace::is_workspace(&args.dir) {
        if workspace::has_decks(&args.dir) {
            bail!(
                "{} is a folder of decks, not a workspace — add an `alix.toml` to \
                 make it one, or `alix review {}` to review its decks",
                args.dir.display(),
                args.dir.display(),
            );
        }
        bail!("{} is not a workspace (no `alix.toml`)", args.dir.display());
    }
    // Web-first: hand off to the browser deck picker; there is no CLI member
    // picker. The `--serve` machinery serves the same web app bare `alix` opens.
    review_serve(
        ReviewArgs {
            decks: Vec::new(),
            order: None,
            topology: None,
            region: None,
            new: 10,
            limit: None,
            cram: false,
            store: args.store,
            config: args.config,
            serve: ServeOpts {
                serve: true,
                port: None,
                lan: false,
                token: None,
            },
        },
        false,
    )
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
    let graded = s.passed + s.partly + s.failed;
    if graded == 0 {
        println!("\n{DIM}Left the trace early — no checkpoints recorded.{RESET}");
        return;
    }
    println!(
        "\n{BOLD}Walk complete{RESET}  {} got it · {} partly · {} missed it",
        s.passed, s.partly, s.failed
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

/// Prompts for the self-judged delta, re-asking until it gets `g`/`p`/`f`.
/// Returns `None` to quit (a leading `q`, or EOF).
fn read_delta() -> Result<Option<alix::trace::Delta>> {
    loop {
        let prompt = format!("{DIM}  gap?  [n]ailed · [p]artly · [f]ailed  (q to quit) >{RESET} ");
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
        println!("{DIM}  answer n, p, or f (or q to quit).{RESET}");
    }
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

fn backend_check_cmd(all: bool, config_path: Option<PathBuf>) -> Result<()> {
    let config = Config::load(config_path.as_deref())?;
    alix::backend::health::check(&config.ask, all)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alix::answer::Mode;

    use super::*;

    fn card(front: &str, back: &str) -> Card {
        Card::plain(Arc::from("d.txt"), front.into(), vec![back.into()], None, 1)
    }

    #[test]
    fn serve_token_is_generated_only_when_exposed() {
        let cfg = Config::default();
        // localhost, nothing configured → open (no token)
        assert_eq!(resolve_serve_token(None, false, &cfg).unwrap(), None);
        // an explicit --token always wins
        assert_eq!(
            resolve_serve_token(Some("abc".into()), true, &cfg).unwrap(),
            Some("abc".into())
        );
        // --lan with nothing configured → a token is generated (fails closed, so
        // on this platform it must succeed)
        assert!(
            resolve_serve_token(None, true, &cfg)
                .unwrap()
                .is_some_and(|t| !t.is_empty())
        );
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
    fn build_browse_loads_from_explicit_paths_including_image_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        // A normal card and an image card — both render in the web frontend.
        std::fs::write(
            &path,
            "% img-dir: /imgs\n# plain\n\tanswer\n# pic\n% img: a.png\n\tphoto\n",
        )
        .unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));

        let build = build_browse(vec![path], &mut recent).unwrap();
        assert_eq!(2, build.cards.len());
    }

    #[test]
    fn build_browse_applies_a_cached_format_reshape() {
        // A deck in a workspace, so `build_browse` resolves the workspace's own
        // store (a deterministic temp path) rather than the global store.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Eng\"\n").unwrap();
        let path = ws.join("d.txt");
        std::fs::write(&path, "# List the parts\n\tA, B, C\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));

        // Without a cached format, browse shows the raw deck answer.
        let raw = build_browse(vec![path.clone()], &mut recent).unwrap();
        let id = raw.cards[0].id();
        assert_eq!(raw.cards[0].back_for_display(), ["A, B, C"]);

        // Cache a format reshape (and a notes-target trivia) for that card in the
        // workspace's augment sidecar.
        let store = store_for(std::slice::from_ref(&path), None).unwrap();
        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.set_format(
            id,
            augment::Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: None,
            },
        );
        cache.set_note(id, "the parts are well known".to_string());
        cache.save().unwrap();

        // Browsing now shows the reshaped front/answer and the trivia note.
        let merged = build_browse(vec![path], &mut recent).unwrap();
        assert_eq!(merged.cards[0].front, "Name the parts");
        assert_eq!(merged.cards[0].back_for_display(), ["A", "B", "C"]);
        let note = merged.cards[0].note.clone().unwrap_or_default();
        assert!(note.contains("the parts are well known"), "{note}");
    }

    #[test]
    fn build_browse_rejects_multiple_decks() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "# q\n\ta\n").unwrap();
        std::fs::write(&b, "# q\n\tb\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let err = build_browse(vec![a, b], &mut recent).err().unwrap();
        assert!(format!("{err}").contains("one deck"), "{err}");
    }

    #[test]
    fn build_browse_rejects_a_workspace_directory() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("m.txt"), "# q\n\ta\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let err = build_browse(vec![ws], &mut recent).err().unwrap();
        assert!(format!("{err}").contains("workspace"), "{err}");
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

    /// A bare-bones `ReviewArgs` for a `build_review` test: no CLI overrides,
    /// the built-in `new` default, no `--serve`.
    fn review_args(decks: Vec<PathBuf>, region: Option<&str>) -> ReviewArgs {
        ReviewArgs {
            decks,
            order: None,
            topology: None,
            region: region.map(str::to_string),
            new: 10,
            limit: None,
            cram: false,
            store: None,
            config: None,
            serve: ServeOpts {
                serve: false,
                port: None,
                lan: false,
                token: None,
            },
        }
    }

    /// Inserts a virtual (remediation) card for deck `subject` into `store` the
    /// way the substrate does — sidecar content keyed by its `Card::id`, plus a
    /// fresh schedule seeded at `t=0` (so it's due, not treated as unseen).
    fn insert_virtual_card(store: &mut Store, subject: &str) {
        use alix::store::VirtualKind;
        let text = "# virtual front\n\tvirtual back\n".to_string();
        let id = parser::parse_str(subject, &text).unwrap()[0].id();
        store.insert_virtual(VirtualCard {
            id,
            kind: VirtualKind::Remediation,
            parent: subject.to_string(),
            text,
            created_ms: 0,
        });
        store.get_or_insert(id, 0);
    }

    #[test]
    fn build_review_injects_a_decks_virtual_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.txt");
        std::fs::write(&path, "# q1\n\ta1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override — a
        // bare `None` here would fall through to the real global data dir.
        let store_path = Some(dir.path().join("store.json"));
        let mut store = store_for(std::slice::from_ref(&path), store_path).unwrap();
        insert_virtual_card(&mut store, "rust.txt");

        let config = Config::default();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let args = review_args(vec![], None);
        let build =
            build_review(vec![path], &args, &config, &store, &mut recent, None, None).unwrap();
        // The deck's one (new) card, plus the injected due virtual card.
        assert_eq!(2, build.session.initial_size);
    }

    #[test]
    fn region_focus_excludes_virtual_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.txt");
        std::fs::write(&path, "# q1\n\ta1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override — a
        // bare `None` here would fall through to the real global data dir.
        let store_path = Some(dir.path().join("store.json"));
        let mut store = store_for(std::slice::from_ref(&path), store_path).unwrap();

        let deck = Deck::load(&path).unwrap();
        let card_id = deck.cards[0].id();

        // Cache a one-region topology covering this deck's one card.
        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.add_topology(Topology {
            name: "auto".to_string(),
            principle: "test".to_string(),
            edges: vec![],
            walk: vec![card_id],
            regions: vec![augment::TopologyRegion {
                name: "r1".to_string(),
                cards: vec![card_id],
            }],
        });
        cache.save().unwrap();

        // A matching virtual card for this deck.
        insert_virtual_card(&mut store, "rust.txt");

        let config = Config::default();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let args = review_args(vec![], Some("r1"));
        let build = build_review(
            vec![path],
            &args,
            &config,
            &store,
            &mut recent,
            None,
            Some("r1"),
        )
        .unwrap();
        // Only the region's one real card — a `--region` focus is a
        // deck-topology drill, and virtual cards aren't part of any topology.
        assert_eq!(1, build.session.initial_size);
    }

    #[test]
    fn a_format_cache_entry_applies_to_a_synthesized_virtual_card() {
        // A synthesized virtual card has a real `Card::id`, so an existing
        // format-cache entry for that id applies with no change to
        // `apply_format` itself — the "free" half of augment-for-virtuals (§8.1).
        let subject: Arc<str> = Arc::from("rust.txt");
        let text = "# List the parts\n\tA, B, C\n".to_string();
        let id = parser::parse_str(&subject, &text).unwrap()[0].id();
        let vc = VirtualCard {
            id,
            kind: alix::store::VirtualKind::Remediation,
            parent: subject.to_string(),
            text,
            created_ms: 0,
        };
        let mut synth = synthesize_virtual(&vc, &subject, VIRTUAL_LINE_BASE).unwrap();

        let mut cache =
            AugmentCache::open(std::env::temp_dir().join("nonexistent-augment-virtual.json"));
        cache.set_format(
            id,
            augment::Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: Some(Mode::LineByLine),
            },
        );
        cache.apply_format(&mut synth);

        assert_eq!("Name the parts", synth.front);
        assert_eq!(["A", "B", "C"], *synth.back_for_display());
        assert_eq!(id, synth.id(), "reshaping must not change identity");
    }

    #[test]
    fn build_review_applies_a_cached_format_to_an_injected_virtual_card() {
        // The display half of augment-for-virtuals (§8.1): `build_review` must
        // reshape an injected synth card the same way it reshapes deck cards.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.txt");
        std::fs::write(&path, "# q1\n\ta1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override — a
        // bare `None` here would fall through to the real global data dir.
        let store_path = Some(dir.path().join("store.json"));
        let mut store = store_for(std::slice::from_ref(&path), store_path).unwrap();
        insert_virtual_card(&mut store, "rust.txt");
        let virtual_id =
            parser::parse_str("rust.txt", "# virtual front\n\tvirtual back\n").unwrap()[0].id();

        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.set_format(
            virtual_id,
            augment::Format {
                front: Some("Reshaped virtual front".to_string()),
                back: vec!["Reshaped virtual back".to_string()],
                note: None,
                mode: None,
            },
        );
        cache.save().unwrap();

        let config = Config::default();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let args = review_args(vec![], None);
        let build =
            build_review(vec![path], &args, &config, &store, &mut recent, None, None).unwrap();

        let synth = build
            .session
            .cards()
            .iter()
            .find(|c| c.id() == virtual_id)
            .expect("the injected virtual card should be in the session");
        assert_eq!("Reshaped virtual front", synth.front);
        assert_eq!(["Reshaped virtual back"], *synth.back_for_display());
    }
}
