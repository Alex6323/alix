use std::path::{Path, PathBuf};

use alix::{
    config::Config,
    sources::commands::{self, AddRequest},
};
use anyhow::{Result, bail};
use clap::{Args, Subcommand};

/// The frozen exam material of a workspace deck: copies of local text files,
/// each bound to the topics it grounds.
#[derive(Subcommand)]
pub(crate) enum SourceAction {
    /// Freeze local text files as sources of a deck.
    Add(AddArgs),
    /// Drop sources from a deck; their copies are deleted.
    Remove(RemoveArgs),
    /// Refreeze one source from a file, keeping its bindings.
    Update(UpdateArgs),
    /// Bind a source to more topics, or replace a topic's ranges.
    Bind(BindArgs),
    /// Unbind a source from topics.
    Unbind(UnbindArgs),
    /// Save or forget where a key's live files are on this machine.
    #[command(subcommand)]
    Root(RootAction),
    /// Show a deck's topics and sources.
    List(DeckArg),
}

#[derive(Subcommand)]
pub(crate) enum RootAction {
    /// Save the folder a key's live files are in.
    Set(RootSetArgs),
    /// Forget a key's folder.
    Unset(RootUnsetArgs),
}

#[derive(Args)]
pub(crate) struct DeckArg {
    /// A workspace deck: its file, or a unique prefix of its `deck-` id.
    deck: String,
}

#[derive(Args)]
pub(crate) struct AddArgs {
    /// A workspace deck: its file, or a unique prefix of its `deck-` id.
    deck: String,
    /// The local text files to freeze.
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Save `<dir>` as the folder of key `<key>` before adding.
    #[arg(long, value_name = "KEY=DIR", value_parser = parse_root)]
    root: Option<(String, PathBuf)>,
    /// The saved root the files are under, when more than one holds them.
    #[arg(long)]
    key: Option<String>,
    /// Topics the files ground, each `[<ranges>|]<topic>` with a unique id
    /// prefix; optional only for a deck without `#` topics.
    #[arg(long = "topic", num_args = 1..)]
    topics: Vec<String>,
}

#[derive(Args)]
pub(crate) struct RemoveArgs {
    /// A workspace deck: its file, or a unique prefix of its `deck-` id.
    deck: String,
    /// Sources to drop, each a unique prefix of its `source-` id.
    #[arg(required = true)]
    sources: Vec<String>,
}

#[derive(Args)]
pub(crate) struct UpdateArgs {
    /// A workspace deck: its file, or a unique prefix of its `deck-` id.
    deck: String,
    /// The source to refreeze, a unique prefix of its `source-` id.
    source: String,
    /// The file holding its new bytes.
    file: PathBuf,
    /// The saved root the file is under, when more than one holds it.
    #[arg(long)]
    key: Option<String>,
}

#[derive(Args)]
pub(crate) struct BindArgs {
    /// A workspace deck: its file, or a unique prefix of its `deck-` id.
    deck: String,
    /// The source, a unique prefix of its `source-` id.
    source: String,
    /// Topics, each `[<ranges>|]<topic>` with a unique id prefix.
    #[arg(required = true)]
    topics: Vec<String>,
}

#[derive(Args)]
pub(crate) struct UnbindArgs {
    /// A workspace deck: its file, or a unique prefix of its `deck-` id.
    deck: String,
    /// The source, a unique prefix of its `source-` id.
    source: String,
    /// Topics to unbind, each a unique prefix of a `topic-` id or the deck
    /// id, without ranges.
    #[arg(required = true)]
    topics: Vec<String>,
}

#[derive(Args)]
pub(crate) struct RootSetArgs {
    /// A workspace deck: its file, or a unique prefix of its `deck-` id.
    deck: String,
    /// The key, as display paths spell it before the colon.
    key: String,
    /// The folder holding that key's live files.
    dir: PathBuf,
}

#[derive(Args)]
pub(crate) struct RootUnsetArgs {
    /// A workspace deck: its file, or a unique prefix of its `deck-` id.
    deck: String,
    /// The key to forget.
    key: String,
}

fn parse_root(text: &str) -> Result<(String, PathBuf), String> {
    match text.split_once('=') {
        Some((key, dir)) if !key.is_empty() && !dir.is_empty() => {
            Ok((key.to_string(), PathBuf::from(dir)))
        }
        _ => Err("expected `<key>=<dir>`".to_string()),
    }
}

fn resolve(deck: &str) -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let decks_dir = if cwd.join(deck).is_file() {
        None
    } else {
        Config::load(None)?.decks_dir()
    };
    commands::resolve_deck(deck, &cwd, decks_dir.as_deref())
}

fn print_collected(collected: &[String]) {
    for name in collected {
        println!("deleted unused copy {name}");
    }
}

fn shown(path: Option<&Path>) -> String {
    path.map_or_else(|| "(none)".to_string(), |path| path.display().to_string())
}

pub(crate) fn run(action: SourceAction) -> Result<()> {
    match action {
        SourceAction::Add(args) => {
            let deck = resolve(&args.deck)?;
            let report = commands::add(AddRequest {
                deck: &deck,
                files: args.files,
                root: args.root,
                key: args.key,
                topics: args.topics,
            })?;
            for (id, path) in &report.added {
                println!("added {id} {path}");
            }
            print_collected(&report.collected);
        }
        SourceAction::Remove(args) => {
            let deck = resolve(&args.deck)?;
            let report = commands::remove(&deck, &args.sources)?;
            for (id, path) in &report.removed {
                println!("removed {id} {path}");
            }
            print_collected(&report.collected);
        }
        SourceAction::Update(args) => {
            let deck = resolve(&args.deck)?;
            let report = commands::update(&deck, &args.source, &args.file, args.key.as_deref())?;
            println!(
                "updated {} -> {} {}",
                report.old_id, report.new_id, report.path
            );
            for entry in &report.kept_ranges {
                println!("kept range, review it: {entry}");
            }
            print_collected(&report.collected);
        }
        SourceAction::Bind(args) => {
            let deck = resolve(&args.deck)?;
            commands::bind(&deck, &args.source, &args.topics)?;
        }
        SourceAction::Unbind(args) => {
            let deck = resolve(&args.deck)?;
            commands::unbind(&deck, &args.source, &args.topics)?;
        }
        SourceAction::Root(RootAction::Set(args)) => {
            let deck = resolve(&args.deck)?;
            let change = commands::root_set(&deck, &args.key, &args.dir)?;
            println!(
                "root {}: {} -> {}",
                args.key,
                shown(change.old.as_deref()),
                change.new.display()
            );
        }
        SourceAction::Root(RootAction::Unset(args)) => {
            let deck = resolve(&args.deck)?;
            let old = commands::root_unset(&deck, &args.key)?;
            if old.is_none() {
                bail!("no root `{}` is saved", args.key);
            }
            println!("root {}: {} -> (none)", args.key, shown(old.as_deref()));
        }
        SourceAction::List(args) => {
            let deck = resolve(&args.deck)?;
            let listing = commands::list(&deck)?;
            println!("topics:");
            for topic in &listing.topics {
                println!(
                    "  {}  {}  {} source(s), {} bytes",
                    topic.id, topic.label, topic.source_count, topic.source_bytes
                );
            }
            println!("sources:");
            for source in &listing.sources {
                println!(
                    "  {}  {} bytes  {}  {}  bound to {}",
                    source.id,
                    source.bytes,
                    source.path,
                    source.drift.label(),
                    source.bound.join(", ")
                );
            }
        }
    }
    Ok(())
}
