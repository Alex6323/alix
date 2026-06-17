use std::{
    collections::{HashMap, HashSet},
    io::IsTerminal,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use flash::{
    answer::Mode,
    browse,
    card::Card,
    config::{self, Config},
    deck::{Deck, DeckSettings},
    generate, parser, picker,
    recent::{self, RecentDecks},
    scheduler::SchedulerKind,
    serve,
    session::{Order, Session, SessionOptions, histogram},
    store::{Store, default_store_path},
    time::{humanize_ms, now_ms},
    tui::{self, App},
};

/// A spaced-repetition flashcard trainer for the terminal.
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
        Some(Command::Check { decks }) => check(decks),
        Some(Command::Browse(args)) => browse(args),
        Some(Command::Generate(args)) => generate_cmd(args),
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
fn load_decks(paths: &[PathBuf]) -> Result<LoadedDecks> {
    let mut cards = Vec::new();
    let mut names = Vec::new();
    let mut decks = std::collections::HashMap::new();
    let mut settings = Vec::new();
    for path in paths {
        let deck = Deck::load(path)?;
        names.push(deck.subject.clone());
        decks.insert(
            deck.subject.clone(),
            tui::DeckInfo {
                path: deck.path.clone(),
                links: deck.links.clone(),
            },
        );
        settings.push(deck.settings);
        cards.extend(deck.cards);
    }
    Ok((cards, names.join(", "), decks, settings))
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
) -> Result<Option<Vec<PathBuf>>> {
    if !decks.is_empty() {
        return Ok(Some(decks));
    }
    if !std::io::stdout().is_terminal() {
        bail!("no deck files given; try `flash <deck.txt>...` or `flash --help`");
    }
    let decks_dir = config.decks_dir().context("cannot determine ~/decks")?;
    let picked = picker::pick(&decks_dir, recent)?;
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
        let dep = resolve_dep(req, decks_dir, parent)
            .ok_or_else(|| anyhow!("{} requires '{}', which was not found", deck.subject, req))?;
        visit_dep(&dep, decks_dir, ordered, done, on_stack, any_requires)?;
    }
    on_stack.remove(&key);
    done.insert(key.clone());
    ordered.push(key);
    Ok(())
}

/// Finds the file a `% requires:` value refers to: as given, next to the
/// requiring deck, or in the decks directory; with or without a `.txt` suffix.
fn resolve_dep(
    req: &str,
    decks_dir: Option<&Path>,
    requiring_dir: Option<&Path>,
) -> Option<PathBuf> {
    let with_txt = |p: &Path| -> PathBuf {
        if p.extension().is_some() {
            p.to_path_buf()
        } else {
            p.with_extension("txt")
        }
    };
    let mut candidates = vec![PathBuf::from(req), with_txt(Path::new(req))];
    for dir in [requiring_dir, decks_dir].into_iter().flatten() {
        candidates.push(dir.join(req));
        candidates.push(with_txt(&dir.join(req)));
    }
    candidates.into_iter().find(|p| p.is_file())
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
}

/// Loads the decks named (or picked) for a review, resolves prerequisites and
/// the mode/scheduler/order settings, and builds the session and store. Shared
/// by `flash review` (TUI) and `flash serve` (web). Returns `Ok(None)` when the
/// picker was cancelled.
fn load_review_session(args: &ReviewArgs) -> Result<Option<ReviewSession>> {
    let config = Config::load(args.config.as_deref())?;

    let mut recent = RecentDecks::load(
        recent::default_recent_path().context("cannot determine the data directory")?,
    );
    let Some(deck_paths) = pick_decks_if_empty(args.decks.clone(), &config, &recent)? else {
        return Ok(None); // picker cancelled or nothing selected
    };

    // Pull in prerequisite decks (`% requires:`), foundations first.
    let decks_dir = config.decks_dir();
    let (resolved, deps_used) = resolve_deck_order(&deck_paths, decks_dir.as_deref())?;

    let (cards, label, decks, settings) = load_decks(&resolved)?;
    let store = open_store(args.store.clone())?;

    // Directives (mode/scheduler/order) come from the requested deck(s) only,
    // not the pulled-in prerequisites — a prerequisite must not override the
    // mode you chose for the deck you actually want to study.
    let target_subjects: HashSet<&str> = deck_paths
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
    let session = Session::new_with_deps(cards, &store, scheduler, options, dep_ranks, now_ms());

    // Remember these decks for next time's picker.
    recent.record(&deck_paths, now_ms());
    let _ = recent.save();

    Ok(Some(ReviewSession {
        session,
        store,
        mode_override: args.mode,
        label,
        decks,
        config,
    }))
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
    let Some(rs) = load_review_session(&args)? else {
        return Ok(());
    };
    let ReviewSession {
        session,
        store,
        mode_override,
        label,
        decks,
        config,
    } = rs;

    // Serve in the browser instead of the terminal. The session still starts
    // even if nothing is due — the page shows that state itself.
    if args.serve.serve {
        let addr = serve_addr(args.serve.port, args.serve.lan, &config);
        announce(addr, args.serve.lan, &label);
        return serve::run_review(
            session,
            store,
            addr,
            serve::ReviewOptions {
                mode_override,
                label,
                decks: subject_paths(decks),
                keys: config.keys,
                max_typos: args.max_typos,
            },
        );
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
        keys: config.keys,
        ask: config.ask,
        decks,
    };
    let stats = App::new(session, store, ui_options).run()?;
    println!(
        "Reviewed {} cards: {} passed, {} failed.",
        stats.reviews, stats.passed, stats.failed
    );
    Ok(())
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
                let due = scheduler.due_at(state);
                if due <= now {
                    due_now += 1;
                } else if due <= now + 86_400_000 {
                    due_24h += 1;
                }
                reviews += state.total_reviews;
                passes += state.total_passes;
            }
        }

        println!("{} ({} cards)", deck.subject, deck.cards.len());
        println!(
            "  stages:  new {} │ s1 {} │ s2 {} │ s3 {} │ s4 {} │ s5 {}",
            h[0], h[1], h[2], h[3], h[4], h[5]
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
        println!("{}", deck.subject);
        for card in &deck.cards {
            let (stage, due) = match store.get(card.id()) {
                Some(state) => {
                    let due = scheduler.due_at(state);
                    let due = if due <= now {
                        "due now".to_string()
                    } else {
                        format!("due in {}", humanize_ms(due - now))
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

fn browse(args: BrowseArgs) -> Result<()> {
    // The terminal browser needs a TTY; the web one only needs it for the
    // interactive picker (when no decks are given).
    if !args.serve.serve && !std::io::stdout().is_terminal() {
        bail!("`flash browse` needs a terminal");
    }
    let config = Config::load(None)?;
    let mut recent = RecentDecks::load(
        recent::default_recent_path().context("cannot determine the data directory")?,
    );
    let Some(deck_paths) = pick_decks_if_empty(args.decks.clone(), &config, &recent)? else {
        return Ok(()); // picker cancelled or nothing selected
    };

    let (cards, label, decks_info, _) = load_decks(&deck_paths)?;
    recent.record(&deck_paths, now_ms());
    let _ = recent.save();

    // Browse only writes if the user removes a card: it then deletes it from
    // the deck file and prunes its progress. Provide the per-subject paths and
    // the store for that.
    let paths = subject_paths(decks_info);
    let store = open_store(None)?;

    if args.serve.serve {
        let addr = serve_addr(args.serve.port, args.serve.lan, &config);
        announce(addr, args.serve.lan, &format!("{label} (browse)"));
        return serve::run_browse(cards, label, addr, paths, store, config.browse);
    }
    browse::run(cards, label, config.browse, paths, store)
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
    let mut problems = 0usize;
    for path in &decks {
        match Deck::load(path) {
            Err(e) => {
                problems += 1;
                eprintln!("error: {e}");
            }
            Ok(deck) => {
                println!("{}: {} cards", deck.subject, deck.cards.len());
                let s = &deck.settings;
                let declared: Vec<String> = [
                    s.mode.map(|m| format!("mode: {}", val_name(m))),
                    s.scheduler.map(|s| format!("scheduler: {}", val_name(s))),
                    s.order.map(|o| format!("order: {}", val_name(o))),
                ]
                .into_iter()
                .flatten()
                .collect();
                if !declared.is_empty() {
                    println!("  settings: {}", declared.join(", "));
                }
                for (a, b) in deck.duplicates() {
                    problems += 1;
                    eprintln!(
                        "warning: {}: cards at lines {} and {} have identical answers \
                         and share their learning progress",
                        deck.subject, a.line, b.line
                    );
                }
            }
        }
    }
    if problems > 0 {
        bail!("{problems} problem(s) found");
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
