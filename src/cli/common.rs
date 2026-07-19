use std::{
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
};

use alix::{
    assemble::{open_store, store_path_for},
    config::Config,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TreeSize {
    pub(crate) files: usize,
    pub(crate) bytes: u64,
}

impl TreeSize {
    pub(crate) fn human_bytes(&self) -> String {
        let b = self.bytes;
        if b < 1_024 {
            format!("{b} B")
        } else if b < 1_024 * 1_024 {
            format!("{:.1} KB", b as f64 / 1_024.0)
        } else {
            format!("{:.1} MB", b as f64 / (1_024.0 * 1_024.0))
        }
    }
}

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules"];

pub(crate) fn tree_size(root: &Path) -> TreeSize {
    let mut files: usize = 0;
    let mut bytes: u64 = 0;
    walk(root, &mut files, &mut bytes);
    TreeSize { files, bytes }
}

fn walk(dir: &Path, files: &mut usize, bytes: &mut u64) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            let name = entry.file_name();
            if SKIP_DIRS.iter().any(|skip| name.to_str() == Some(skip)) {
                continue;
            }
            walk(&path, files, bytes);
        } else if meta.is_file() {
            *files += 1;
            *bytes += meta.len();
        }
    }
}

pub(crate) fn is_oversized(bytes: u64, threshold: u64) -> bool {
    bytes > threshold
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
    let size = tree_size(path);
    if !is_oversized(size.bytes, threshold) {
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
    use std::fs;

    use tempfile::TempDir;

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

    fn make_file(dir: &Path, name: &str, size: usize) {
        fs::write(dir.join(name), vec![0u8; size]).unwrap();
    }

    #[test]
    fn tree_size_counts_files_and_bytes() {
        let dir = TempDir::new().unwrap();
        make_file(dir.path(), "a.txt", 100);
        make_file(dir.path(), "b.txt", 200);
        let size = tree_size(dir.path());
        assert_eq!(2, size.files);
        assert_eq!(300, size.bytes);
    }

    #[test]
    fn tree_size_recurses_into_subdirs() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        make_file(&sub, "c.txt", 50);
        make_file(dir.path(), "root.txt", 10);
        let size = tree_size(dir.path());
        assert_eq!(2, size.files);
        assert_eq!(60, size.bytes);
    }

    #[test]
    fn tree_size_skips_git_target_node_modules() {
        let dir = TempDir::new().unwrap();
        make_file(dir.path(), "real.txt", 10);

        for skip in [".git", "target", "node_modules"] {
            let d = dir.path().join(skip);
            fs::create_dir(&d).unwrap();
            make_file(&d, "hidden.txt", 999);
        }

        let size = tree_size(dir.path());
        assert_eq!(1, size.files);
        assert_eq!(10, size.bytes);
    }

    #[test]
    fn tree_size_is_zero_for_empty_dir() {
        let dir = TempDir::new().unwrap();
        let size = tree_size(dir.path());
        assert_eq!(0, size.files);
        assert_eq!(0, size.bytes);
    }

    #[test]
    fn is_oversized_uses_strict_greater_than() {
        assert!(!is_oversized(5_000_000, 5_000_000));
        assert!(is_oversized(5_000_001, 5_000_000));
        assert!(!is_oversized(0, 5_000_000));
    }

    #[test]
    fn human_bytes_formats_correctly() {
        assert_eq!(
            "512 B",
            TreeSize {
                files: 1,
                bytes: 512
            }
            .human_bytes()
        );
        assert_eq!(
            "1.0 KB",
            TreeSize {
                files: 1,
                bytes: 1_024
            }
            .human_bytes()
        );
        assert_eq!(
            "1.0 MB",
            TreeSize {
                files: 1,
                bytes: 1_024 * 1_024
            }
            .human_bytes()
        );
        assert_eq!(
            "4.8 MB",
            TreeSize {
                files: 1,
                bytes: 5_000_000
            }
            .human_bytes()
        );
    }
}
