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

pub(crate) struct Target {
    pub(crate) decks: Vec<PathBuf>,
    pub(crate) default_store: Option<PathBuf>,
}

impl Target {
    pub(crate) fn store_for_deck(&self, deck: &Path, cli_override: Option<&Path>) -> Result<Store> {
        let path = cli_override
            .map(Path::to_path_buf)
            .or_else(|| store_path_for(std::slice::from_ref(&deck.to_path_buf()), None))
            .or_else(|| self.default_store.clone());
        open_store(path)
    }
}

pub(crate) fn expand_target(path: &Path, config: &Config) -> Result<Target> {
    if path.is_file() {
        return Ok(Target {
            decks: vec![path.to_path_buf()],
            // A loose deck defaults to the bare-`alix` root store;
            // `store_for_deck`'s workspace lookup may override this.
            default_store: config.decks_dir().map(|d| workspace::root_store_path(&d)),
        });
    }
    if !path.is_dir() {
        bail!("`{}` is neither a deck file nor a folder", path.display());
    }
    let mut decks: Vec<PathBuf> = std::fs::read_dir(path)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md"))
        .collect();
    decks.sort();
    if decks.is_empty() {
        bail!("no decks in `{}`", path.display());
    }
    let default_store = if workspace::is_workspace(path) {
        None // members resolve to the workspace's own store anyway
    } else {
        Some(workspace::root_store_path(path))
    };
    Ok(Target {
        decks,
        default_store,
    })
}

pub(crate) fn store_for(
    decks: &[PathBuf],
    cli_override: Option<PathBuf>,
    config: &Config,
) -> Result<Store> {
    let path = store_path_for(decks, cli_override.as_deref())
        .or_else(|| config.decks_dir().map(|d| workspace::root_store_path(&d)));
    open_store(path)
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

fn is_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

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

pub(crate) fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

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
    fn store_for_resolves_a_loose_deck_to_the_decks_dir_root_store() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("loose.md");
        std::fs::write(&deck, "## q\na\n").unwrap();
        let config = Config {
            decks_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let store = store_for(std::slice::from_ref(&deck), None, &config).unwrap();

        assert_eq!(store.path(), dir.path().join("progress.json").as_path());
    }

    #[test]
    fn store_for_lets_a_cli_override_win_over_the_decks_dir_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("loose.md");
        std::fs::write(&deck, "## q\na\n").unwrap();
        let override_path = dir.path().join("custom.json");
        let config = Config {
            decks_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let store = store_for(
            std::slice::from_ref(&deck),
            Some(override_path.clone()),
            &config,
        )
        .unwrap();

        assert_eq!(store.path(), override_path.as_path());
    }

    #[test]
    fn store_for_still_resolves_a_workspace_deck_to_the_workspace_store() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("box");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Box\"\n").unwrap();
        let member = ws.join("a.md");
        std::fs::write(&member, "## q\na\n").unwrap();
        let config = Config {
            decks_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let store = store_for(std::slice::from_ref(&member), None, &config).unwrap();

        assert_eq!(store.path(), ws.join("progress.json").as_path());
    }

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
