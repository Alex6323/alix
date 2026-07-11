//! Helpers every CLI command shares: confirmation prompts, source-tree
//! preflight, target expansion (deck/folder/workspace → member decks), and
//! progress-store resolution. Nothing command-specific lives here — a new
//! addition must argue its way in.

use std::{
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
};

use alix::{
    assemble::{open_store, store_path_for},
    config::Config,
    preflight,
    store::Store,
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

/// Opens the progress store for `decks`, honoring `--store`. See
/// [`alix::assemble::store_path_for`].
pub(crate) fn store_for(decks: &[PathBuf], cli_override: Option<PathBuf>) -> Result<Store> {
    open_store(store_path_for(decks, cli_override.as_deref()))
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
}
