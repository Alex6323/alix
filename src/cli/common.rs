//! Helpers every CLI command shares: confirmation prompts, source-tree
//! preflight, target expansion (deck/folder/workspace → member decks), and
//! progress-store resolution. Nothing command-specific lives here — a new
//! addition must argue its way in.

use std::{
    collections::HashMap,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
};

use alix::{
    card::Card,
    config::Config,
    deck::{Deck, DeckSettings},
    preflight,
    session::DeckInfo,
    store::{Store, default_store_path},
    trace::SourceBase,
    workspace,
};
use anyhow::{Context, Result, bail};

/// A stats/list/reset target expanded to deck files, plus the store fallback
/// for decks that belong to no workspace: a plain served folder keeps its own
/// `progress.json` beside its decks; `None` falls through to the global store.
pub(crate) struct Target {
    pub(crate) decks: Vec<PathBuf>,
    pub(crate) default_store: Option<PathBuf>,
}

impl Target {
    /// The store for one member deck: `--store` > its workspace's store > the
    /// target's own store file (scoped folder) > the global default — the same
    /// rule the launcher serves by, so every command sees the same progress.
    pub(crate) fn store_for_deck(&self, deck: &Path, cli_override: Option<&Path>) -> Result<Store> {
        let path = cli_override
            .map(Path::to_path_buf)
            .or_else(|| store_path_for(std::slice::from_ref(&deck.to_path_buf()), None))
            .or_else(|| self.default_store.clone());
        open_store(path)
    }
}

/// Expands a command target — a deck file, a workspace, or a plain folder —
/// into its member decks (sorted by name for stable output).
pub(crate) fn expand_target(path: &Path) -> Result<Target> {
    if path.is_file() {
        return Ok(Target {
            decks: vec![path.to_path_buf()],
            default_store: None,
        });
    }
    if !path.is_dir() {
        bail!("`{}` is neither a deck file nor a folder", path.display());
    }
    let mut decks: Vec<PathBuf> = std::fs::read_dir(path)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "txt"))
        .collect();
    decks.sort();
    if decks.is_empty() {
        bail!("no decks in `{}`", path.display());
    }
    let default_store = if workspace::is_workspace(path) {
        None // members resolve to the workspace's own store anyway
    } else {
        let scoped = path.join(workspace::STORE_FILE);
        scoped.exists().then_some(scoped)
    };
    Ok(Target {
        decks,
        default_store,
    })
}

/// Opens the progress store (creating an empty one on first use).
pub(crate) fn open_store(path: Option<PathBuf>) -> Result<Store> {
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
pub(crate) fn store_path_for(decks: &[PathBuf], cli_override: Option<&Path>) -> Option<PathBuf> {
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
pub(crate) fn store_for(decks: &[PathBuf], cli_override: Option<PathBuf>) -> Result<Store> {
    open_store(store_path_for(decks, cli_override.as_deref()))
}

/// The cards of all loaded decks, a header label, the per-subject deck info
/// for the web session, and the per-deck `% key: value` settings.
pub(crate) type LoadedDecks = (
    Vec<Card>,
    String,
    std::collections::HashMap<String, DeckInfo>,
    Vec<DeckSettings>,
);

/// Loads all decks and returns their cards, a label for the header, the
/// per-subject deck info (file path and reference links) for the web session,
/// and the per-deck `% key: value` settings.
pub(crate) fn load_decks(
    paths: &[PathBuf],
    defaults: &HashMap<String, DeckSettings>,
) -> Result<LoadedDecks> {
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

pub(crate) fn confirm(prompt: &str, yes: bool) -> Result<bool> {
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
pub(crate) fn preflight_source(source: &str, threshold: u64, yes: bool) -> Result<()> {
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

/// Where a single generated/imported deck lands: the `--workspace <dir>` when
/// given (it must exist — `alix workspace init` creates one), else the decks
/// directory.
pub(crate) fn deck_out_dir(workspace: Option<&Path>, config: &Config) -> Result<PathBuf> {
    match workspace {
        Some(dir) => {
            if !dir.is_dir() {
                bail!(
                    "no folder at {} — create the workspace first: alix workspace init {}",
                    dir.display(),
                    dir.display()
                );
            }
            Ok(dir.to_path_buf())
        }
        None => config
            .decks_dir()
            .context("cannot determine the decks directory"),
    }
}

/// Collapses whitespace runs (incl. newlines) onto one line.
pub(crate) fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncates `s` to at most `max` chars, appending an ellipsis when it was cut.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_collapses_whitespace_runs_including_newlines() {
        assert_eq!("a b c", one_line("a\n  b\tc"));
    }

    #[test]
    fn truncate_only_appends_an_ellipsis_when_the_text_is_cut() {
        assert_eq!("hello", truncate("hello", 10));
        assert_eq!("hel…", truncate("hello world", 4));
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
}
