use std::{
    collections::{HashMap, HashSet},
    io::{IsTerminal, Write},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use flash::{
    answer::Mode,
    browse,
    card::{Card, Frontend},
    config::{self, Config, Strictness},
    deck::{Deck, DeckSettings, DeckState},
    generate, parser, picker,
    recent::{self, RecentDecks},
    scheduler::SchedulerKind,
    serve,
    session::{Order, Session, SessionOptions, histogram},
    store::{Store, default_store_path},
    time::{humanize_ms, now_ms},
    trace::{Phase, Trace, Walk},
    tui::{self, App},
    workspace,
};

/// An AI-augmented spaced-repetition learning tool for the terminal and the
/// web.
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
    /// Generate a deck from a web page using the Claude CLI.
    #[command(visible_alias = "gen")]
    Generate(GenerateArgs),
    /// Sit the AI exam for a deck: open questions from its `% source:`, graded
    /// by Claude. Passing marks the deck mastered and unlocks its dependents.
    Exam(ExamArgs),
    /// Walk a trace: a predict-and-verify path through a `% source:` that
    /// builds understanding. At each checkpoint you predict, then the real
    /// excerpt is revealed and you judge the gap; the path ends with a
    /// compression.
    Trace(TraceArgs),
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
struct GenerateArgs {
    /// URL of the page to turn into a deck.
    url: String,

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
    /// The trace deck to walk (must declare a `% trace:`).
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

    /// Scheduler used to schedule the checkpoints. Overrides the deck's
    /// `% scheduler:` directive; defaults to leitner.
    #[arg(short, long, value_enum)]
    scheduler: Option<SchedulerKind>,

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
    let cli = Cli::parse();
    match cli.command {
        None => review(cli.review),
        Some(Command::Review(args)) => review(args),
        Some(Command::Stats(args)) => stats(args),
        Some(Command::List(args)) => list(args),
        Some(Command::Reset(args)) => reset(args),
        Some(Command::Check { decks }) => check(decks),
        Some(Command::Browse(args)) => browse(args),
        Some(Command::Generate(args)) => generate_cmd(args),
        Some(Command::Exam(args)) => exam_cmd(args),
        Some(Command::Trace(args)) => trace_cmd(args),
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
        if workspace::is_workspace(path) {
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
fn pick_decks_if_empty(
    decks: Vec<PathBuf>,
    config: &Config,
    recent: &RecentDecks,
    store: &Store,
    enforce_locks: bool,
) -> Result<Option<Vec<PathBuf>>> {
    if !decks.is_empty() {
        return Ok(Some(decks));
    }
    if !std::io::stdout().is_terminal() {
        bail!("no deck files given; try `flash <deck.txt>...` or `flash --help`");
    }
    let decks_dir = config.decks_dir().context("cannot determine ~/decks")?;
    let picked = picker::pick(&decks_dir, recent, store, enforce_locks)?;
    Ok((!picked.is_empty()).then_some(picked))
}

/// Expands `initial` decks with their `% requires:` prerequisites, returning
/// the decks in dependency order (every prerequisite before the deck that
/// needs it), de-duplicated, plus whether any prerequisites were declared.
fn resolve_deck_order(
    initial: &[PathBuf],
    decks_dir: Option<&Path>,
) -> Result<(Vec<PathBuf>, bool)> {
    let mut ordered = Vec::new();
    let mut done = HashSet::new();
    let mut on_stack = HashSet::new();
    let mut any_requires = false;
    for path in initial {
        visit_dep(
            path,
            decks_dir,
            &mut ordered,
            &mut done,
            &mut on_stack,
            &mut any_requires,
        )?;
    }
    Ok((ordered, any_requires))
}

/// Post-order DFS: a deck is appended to `ordered` only after its
/// prerequisites, so the result lists foundations first. `on_stack` catches
/// dependency cycles; `done` de-duplicates shared prerequisites.
fn visit_dep(
    path: &Path,
    decks_dir: Option<&Path>,
    ordered: &mut Vec<PathBuf>,
    done: &mut HashSet<PathBuf>,
    on_stack: &mut HashSet<PathBuf>,
    any_requires: &mut bool,
) -> Result<()> {
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if done.contains(&key) {
        return Ok(());
    }
    if !on_stack.insert(key.clone()) {
        bail!("dependency cycle detected at {}", path.display());
    }
    let deck = Deck::load(path)?;
    if !deck.requires.is_empty() {
        *any_requires = true;
    }
    let parent = path.parent();
    for req in &deck.requires {
        let dep = flash::deck::resolve_dep(req, decks_dir, parent)
            .ok_or_else(|| anyhow!("{} requires '{}', which was not found", deck.subject, req))?;
        visit_dep(&dep, decks_dir, ordered, done, on_stack, any_requires)?;
    }
    on_stack.remove(&key);
    done.insert(key.clone());
    ordered.push(key);
    Ok(())
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
}

/// Loads the decks named (or picked) for a review, resolves prerequisites and
/// the mode/scheduler/order settings, and builds the session and store. Shared
/// by `flash review` (TUI) and `flash serve` (web). Returns `Ok(None)` when the
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
    // workspace's shared directives), then pull in prerequisites, foundations
    // first.
    let expanded = expand_workspaces(&deck_paths)?;
    let decks_dir = config.decks_dir();
    let (resolved, deps_used) = resolve_deck_order(&expanded.decks, decks_dir.as_deref())?;

    let (cards, deck_label, decks, settings) = load_decks(&resolved, &expanded.defaults)?;
    // A single workspace shows its own title as the session label.
    let label = expanded.label.unwrap_or(deck_label);

    // Keep only the cards reviewable in the target frontend; a card declares
    // `Any` (default), or its specific frontend. Image cards are web-only, so
    // they drop out of the TUI here (and the caller reports the count).
    let total = cards.len();
    let cards: Vec<_> = cards
        .into_iter()
        .filter(|c| matches!(c.frontend(), Frontend::Any) || c.frontend() == target)
        .collect();
    let hidden = total - cards.len();

    // Directives (mode/scheduler/order) come from the requested deck(s) only,
    // not the pulled-in prerequisites — a prerequisite must not override the
    // mode you chose for the deck you actually want to study.
    let target_subjects: HashSet<&str> = expanded
        .decks
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
        .collect();
    let target_settings: Vec<&DeckSettings> = resolved
        .iter()
        .zip(&settings)
        .filter(|(path, _)| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| target_subjects.contains(n))
        })
        .map(|(_, s)| s)
        .collect();

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

    // When prerequisites were pulled in, rank each card by its deck's position
    // in the dependency order so the session presents foundations first.
    let dep_ranks: Vec<usize> = if deps_used {
        let mut rank_of: HashMap<String, usize> = HashMap::new();
        for (rank, path) in resolved.iter().enumerate() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                rank_of.entry(name.to_string()).or_insert(rank);
            }
        }
        cards
            .iter()
            .map(|c| *rank_of.get(&*c.subject).unwrap_or(&0))
            .collect()
    } else {
        Vec::new()
    };

    let options = SessionOptions {
        max_new: args.new,
        limit: args.limit,
        cram: args.cram,
        order,
    };
    let session = Session::new_with_deps(cards, store, scheduler, options, dep_ranks, now_ms());

    // Remember these decks for next time's picker.
    recent.record(&deck_paths, now_ms());
    let _ = recent.save();

    Ok(ReviewBuild {
        session,
        label,
        decks,
        hidden,
    })
}

/// The TUI review path: pick decks if none were given, then build the session
/// — unless a single chosen deck is `exam due`, in which case it resolves to
/// that deck's exam instead of a (cardless) review.
fn load_review_session(args: &ReviewArgs, target: Frontend) -> Result<Option<Started>> {
    let config = Config::load(args.config.as_deref())?;
    let mut recent = RecentDecks::load(
        recent::default_recent_path().context("cannot determine the data directory")?,
    );
    let store = open_store(args.store.clone())?;
    // Review enforces locking — a deck whose prerequisites aren't finished
    // can't be started from the picker.
    let Some(deck_paths) = pick_decks_if_empty(args.decks.clone(), &config, &recent, &store, true)?
    else {
        return Ok(None); // picker cancelled or nothing selected
    };
    // A single exam-due deck launches its exam rather than an empty review.
    if let [path] = deck_paths.as_slice()
        && let Ok(deck) = Deck::load(path)
        && !deck.sources.is_empty()
        && deck.state(&store) == DeckState::ExamDue
    {
        return Ok(Some(Started::Exam {
            deck: Box::new(deck),
            store,
            config: Box::new(config),
        }));
    }
    let build = build_review(deck_paths, args, &config, &store, &mut recent, target)?;
    Ok(Some(Started::Review(Box::new(ReviewSession {
        session: build.session,
        store,
        mode_override: args.mode,
        label: build.label,
        decks: build.decks,
        config,
        hidden: build.hidden,
    }))))
}

/// Launches the interactive exam TUI for `deck`, resolving strictness from the
/// deck's `% strictness:` or the `[exam]` default.
fn run_exam_app(deck: Deck, config: &Config, store: Store) -> Result<()> {
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
    .run()
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

    let started = match load_review_session(&args, Frontend::Tui)? {
        None => return Ok(()),
        // A single exam-due deck was chosen: run its exam, not a review.
        Some(Started::Exam {
            deck,
            store,
            config,
        }) => return run_exam_app(*deck, &config, store),
        Some(Started::Review(rs)) => rs,
    };
    let ReviewSession {
        session,
        store,
        mode_override,
        label,
        decks,
        config,
        hidden,
    } = *started;

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

    let ui_options = flash::tui::Options {
        mode_override,
        max_typos: args.max_typos,
        deck_label: label,
        keys: config.keys.clone(),
        ask: config.ask.clone(),
        decks,
    };
    let (stats, exam_request) = App::new(session, store, ui_options).run()?;
    println!(
        "Reviewed {} cards: {} passed, {} failed.",
        stats.reviews, stats.passed, stats.failed
    );
    // If a deck became exam-due this session and the user chose to sit it, the
    // review app saved the store on exit; reopen it for the exam.
    if let Some(path) = exam_request {
        let store = open_store(args.store.clone())?;
        let deck = Deck::load(&path)?;
        return run_exam_app(deck, &config, store);
    }
    Ok(())
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
    let store = open_store(args.store.clone())?;
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
        serve::SessionBuild {
            session: b.session,
            label: b.label,
            decks: subject_paths(b.decks),
            links,
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
        max_typos: args.max_typos,
        ask: config.ask.clone(),
        exam: config.exam.clone(),
    };
    let build = |paths: Vec<PathBuf>, store: &Store, recent: &mut RecentDecks| {
        build_review(paths, &args, &config, store, recent, Frontend::Web).map(to_build)
    };
    serve::run_review(initial, store, recent, decks_dir, addr, opts, build)
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
    let store = open_store(args.store)?;
    let scheduler = args.scheduler.scheduler();
    let now = now_ms();

    for path in &args.decks {
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
                if !flash::session::is_retired(card, &store) {
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
        // Stages above the deck's `% max-stage:` are unreachable, shown as `–`.
        let top = flash::session::max_reachable_stage(&deck.cards);
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
    let store = open_store(args.store)?;
    let scheduler = args.scheduler.scheduler();
    let now = now_ms();

    for path in &args.decks {
        let deck = Deck::load(path)?;
        println!("{}", deck.display_name());
        for card in &deck.cards {
            let (stage, due) = match store.get(card.id()) {
                Some(state) => {
                    // Retired cards rest until `flash reset`; their due time is
                    // moot, so say so instead of showing a misleading interval.
                    let due = if flash::session::is_retired(card, &store) {
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
    let mut store = open_store(args.store)?;

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
            bail!(
                "no deck files given; try `flash reset <deck.txt>...`, `--card <id>`, or `--all`"
            );
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
        bail!("`flash browse` needs a terminal");
    }
    let config = Config::load(None)?;
    let mut recent = RecentDecks::load(
        recent::default_recent_path().context("cannot determine the data directory")?,
    );
    let store = open_store(None)?;
    // Browse is read-only traversal — locking gates review only, so any deck is
    // browsable (enforce_locks = false).
    let Some(deck_paths) =
        pick_decks_if_empty(args.decks.clone(), &config, &recent, &store, false)?
    else {
        return Ok(()); // picker cancelled or nothing selected
    };
    let build = build_browse(deck_paths, &mut recent, Frontend::Tui)?;

    // Browse only writes if the user removes a card: it then deletes it from the
    // deck file and prunes its progress. Provide the per-subject paths and store.
    let paths = subject_paths(build.decks);
    browse::run(build.cards, build.label, config.browse, paths, store)
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
        build,
    )
}

fn generate_cmd(args: GenerateArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let mut gen_cfg = config.generate.clone();
    if let Some(cards) = args.cards {
        gen_cfg.max_cards = cards;
    }

    eprintln!(
        "Generating a deck from {} (this can take a minute)…",
        args.url
    );
    let mut text = generate::generate_deck(&args.url, &gen_cfg, &config.ask)?;

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
        None => generate::slug_from_url(&args.url),
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
             Fix that line and run `flash check {}`.",
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
    let store = open_store(args.store.clone())?;
    let deck = Deck::load(&args.deck)?;

    // A trace's verification is its own predict-verify walk + compression, not
    // the source-wide AI exam; the generic exam refuses it (its `% source:` is a
    // locator base, not an exam corpus). Point the user at `flash trace`.
    if deck.is_trace() {
        bail!(
            "{} is a trace (it declares a `% trace:`) — walk it with `flash trace`, \
             not the AI exam",
            deck.subject
        );
    }
    if deck.sources.is_empty() {
        bail!(
            "{} declares no `% source:` — add one (a URL or a file path) to \
             examine this deck",
            deck.subject
        );
    }
    // The exam verifies a deck you have already drilled; until every card is
    // retired there is nothing to examine. A mastered deck may be re-sat.
    if matches!(
        deck.state(&store),
        DeckState::NotStarted | DeckState::Started
    ) {
        let to_go = deck
            .cards
            .iter()
            .filter(|c| !flash::session::is_retired(c, &store))
            .count();
        bail!(
            "drill {}'s cards to the top stage first ({to_go} to go), then sit the exam",
            deck.subject
        );
    }
    if !std::io::stdin().is_terminal() {
        bail!("`flash exam` needs a terminal");
    }

    // Grading strictness: CLI flag > the deck's `% strictness:` > the `[exam]`
    // default.
    let strictness = args
        .strictness
        .or(deck.settings.exam_strictness)
        .unwrap_or(config.exam.strictness);
    let decks_dir = config.decks_dir();
    tui::ExamApp::new(deck, strictness, exam_cfg, config.ask, store, decks_dir).run()
}

// ANSI styling for the linear `flash trace` flow (it requires a terminal).
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn trace_cmd(args: TraceArgs) -> Result<()> {
    let deck = Deck::load(&args.deck)?;

    // `--build`: discover the path with Claude and write it back (no walk; a
    // fresh trace deck has no checkpoints yet, so this runs before from_deck).
    if args.build {
        return trace_build(&args, deck);
    }

    let trace = Trace::from_deck(&deck)?;

    // `--map`: just print the path, no quiz, no terminal needed.
    if args.map {
        return print_trace_map(&trace);
    }
    if !std::io::stdin().is_terminal() {
        bail!("`flash trace` needs a terminal");
    }

    let scheduler = args
        .scheduler
        .or(deck.settings.scheduler)
        .unwrap_or_default();
    let mut store = open_store(args.store.clone())?;
    let mut walk = Walk::new(trace, scheduler);

    let total = walk.total();
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
                match read_line(&format!("{DIM}predict >{RESET} "))? {
                    None => break 'walk, // EOF (Ctrl-D)
                    Some(text) => walk.predict(text),
                }
            }
            Phase::Reveal => {
                let checkpoint = walk.checkpoint().cloned().expect("reveal has a checkpoint");
                println!("\n{BOLD}Reveal{RESET}");
                match walk.trace().excerpt(&checkpoint) {
                    Ok(excerpt) => print_excerpt(&excerpt),
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
                if let Some(note) = &checkpoint.note {
                    println!("{DIM}  ! {note}{RESET}");
                }
                match read_delta()? {
                    Some(delta) => walk.grade(&mut store, delta, now_ms()),
                    None => break 'walk, // quit
                }
            }
            Phase::Compress => {
                println!("\n{BOLD}── Compress ──{RESET}");
                println!("{BOLD}tracing{RESET}  {}", walk.trace().description);
                println!(
                    "Retrace it in two sentences — if you can re-derive the whole \
                     path, you understood it."
                );
                match read_line(&format!("{DIM}compress >{RESET} "))? {
                    None => break 'walk,
                    Some(text) => walk.compress(text),
                }
            }
            Phase::Done => break 'walk,
        }
    }

    store.save().context("cannot save progress")?;
    print_trace_summary(&walk);
    Ok(())
}

/// Discovers the path with Claude (`flash trace --build`) and writes the
/// checkpoints back into the deck file, keeping its `% trace:`/`% source:`
/// header.
fn trace_build(args: &TraceArgs, deck: Deck) -> Result<()> {
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
    let cards = flash::trace::build(&deck, &config.trace, &config.ask)?;
    flash::deck::set_trace_checkpoints(&args.deck, &cards)?;

    let n = parser::parse_str(&deck.subject, &cards)
        .map(|c| c.len())
        .unwrap_or(0);
    let path = args.deck.display();
    println!(
        "Wrote {n} checkpoints to {path}. Review them and their `% at:` locators, \
         then walk it:  flash trace {path}"
    );
    Ok(())
}

/// Prints a trace's path (prompts, key points, locators) without quizzing.
fn print_trace_map(trace: &Trace) -> Result<()> {
    println!("{BOLD}Trace{RESET}  {}", trace.description);
    if let Some(src) = &trace.source {
        println!("{DIM}source: {src}{RESET}");
    }
    for (i, checkpoint) in trace.checkpoints.iter().enumerate() {
        println!("\n{BOLD}{}.{RESET} {}", i + 1, checkpoint.prompt);
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

/// Renders an excerpt with a line-number gutter, marking gaps between
/// non-contiguous ranges.
fn print_excerpt(excerpt: &flash::trace::Excerpt) {
    println!("{DIM}  {}{RESET}", excerpt.path.display());
    let mut prev: Option<usize> = None;
    for (no, text) in &excerpt.lines {
        if prev.is_some_and(|p| *no > p + 1) {
            println!("       {DIM}⋮{RESET}");
        }
        println!("  {DIM}{no:>5}{RESET}  {text}");
        prev = Some(*no);
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
fn read_delta() -> Result<Option<flash::trace::Delta>> {
    loop {
        let prompt = format!("{DIM}  gap?  [g]ot · [p]artial · [m]issed  (q to quit) >{RESET} ");
        let Some(answer) = read_line(&prompt)? else {
            return Ok(None);
        };
        match answer.trim().chars().next() {
            Some('q') | Some('Q') => return Ok(None),
            Some(c) => {
                if let Some(delta) = flash::trace::Delta::from_key(c) {
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
        bail!("`flash deps` needs a terminal");
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
    flash::deck::set_requires(&deck_path, &names)?;
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
             `flash config --init`",
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
        std::fs::write(ws.join("flash.toml"), "[defaults]\ndirection = \"both\"\n").unwrap();

        // A member picked as a bare file (a subset selection) still inherits the
        // workspace's directives.
        let exp = expand_workspaces(&[ws.join("a.txt")]).unwrap();
        assert_eq!(1, exp.decks.len());
        assert_eq!(
            Some(flash::card::Direction::Both),
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
