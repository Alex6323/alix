//! `alix doctor`: report-only, never fixes anything.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{config::Config, deck::Deck, store::Store, workspace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug)]
pub struct Finding {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub remedy: Option<String>,
}

impl Finding {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Ok,
            detail: detail.into(),
            remedy: None,
        }
    }

    fn bad(
        name: &'static str,
        status: Status,
        detail: impl Into<String>,
        remedy: impl Into<String>,
    ) -> Self {
        Self {
            name,
            status,
            detail: detail.into(),
            remedy: Some(remedy.into()),
        }
    }
}

/// The one check that must not assume the config loads (unlike every other
/// alix command).
pub fn check_config(path: Option<&Path>) -> (Finding, Config) {
    match Config::load(path) {
        Ok(config) => (Finding::ok("config", "loads fine"), config),
        Err(e) => (
            Finding::bad(
                "config",
                Status::Fail,
                format!("{e:#}"),
                "fix or remove the offending key in the config file (`alix config` shows the active one)",
            ),
            Config::default(),
        ),
    }
}

pub fn check_store(path: Option<PathBuf>) -> Finding {
    let path = match path.or_else(crate::store::default_store_path) {
        Some(p) => p,
        None => {
            return Finding::bad(
                "store",
                Status::Fail,
                "cannot determine the data directory",
                "set HOME/XDG_DATA_HOME so alix has somewhere to keep progress",
            );
        }
    };
    match Store::open(&path) {
        Ok(store) => Finding::ok(
            "store",
            format!(
                "readable ({} card entries) — {}",
                store.len(),
                path.display()
            ),
        ),
        Err(e) => Finding::bad(
            "store",
            Status::Fail,
            format!("{} — {e:#}", path.display()),
            "the store JSON is unreadable — restore it from a backup, or move it aside to start fresh",
        ),
    }
}

/// A broken deck only warns, it breaks itself, not the whole setup.
pub fn check_decks(decks_dir: &Path) -> Finding {
    if !decks_dir.is_dir() {
        // A fresh install: warn with a fix, not a failure (nothing is broken yet).
        return Finding::bad(
            "decks",
            Status::Warn,
            format!("{} does not exist", decks_dir.display()),
            "create it, serve another folder (`alix <dir>`), or set `decks_dir` in the config",
        );
    }
    let mut deck_files: Vec<PathBuf> = Vec::new();
    let mut dirs = 0usize;
    for entry in std::fs::read_dir(decks_dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "md") {
            deck_files.push(path);
        } else if workspace::has_decks(&path) {
            dirs += 1;
            for sub in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                let p = sub.path();
                if p.is_file() && p.extension().is_some_and(|e| e == "md") {
                    deck_files.push(p);
                }
            }
        }
    }
    let mut broken = Vec::new();
    let mut malformed_math = Vec::new();
    for path in &deck_files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        match Deck::load(path) {
            Ok(deck) => {
                let augment = path.parent().map(|dir| {
                    crate::augment::AugmentCache::open(crate::augment::augment_path_for(
                        &workspace::root_store_path(dir),
                    ))
                });
                for diagnostic in crate::math::diagnostics(&deck.cards, augment.as_ref()) {
                    malformed_math.push(format!("{name}: {diagnostic}"));
                }
            }
            Err(_) => broken.push(name),
        }
    }
    let counts = format!(
        "{} decks across {} folders/workspaces — {}",
        deck_files.len(),
        dirs,
        decks_dir.display()
    );
    if broken.is_empty() && malformed_math.is_empty() {
        Finding::ok("decks", counts)
    } else {
        let named = broken
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let math_detail = malformed_math
            .first()
            .map(|diagnostic| {
                format!(
                    "; {} malformed LaTeX formula(s), first: {diagnostic}",
                    malformed_math.len()
                )
            })
            .unwrap_or_default();
        Finding::bad(
            "decks",
            Status::Warn,
            if broken.is_empty() {
                format!("{counts}{math_detail}")
            } else {
                format!(
                    "{counts}; {} won't parse: {named}{math_detail}",
                    broken.len()
                )
            },
            "run `alix doctor <file>` for the exact deck diagnostics",
        )
    }
}

/// Spawns `<cmd> --version` only (no network, no cost).
pub fn check_binary(name: &'static str, cmd: &str, purpose: &str, remedy: &str) -> Finding {
    let found = Command::new(cmd)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    if found {
        Finding::ok(name, format!("`{cmd}` found — {purpose}"))
    } else {
        Finding::bad(
            name,
            Status::Warn,
            format!("`{cmd}` not found — {purpose} unavailable"),
            remedy,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_typo_reports_fail_with_a_remedy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[review]\nfrobnicate = 1\n").unwrap();
        let (finding, _) = check_config(Some(&path));
        assert_eq!(Status::Fail, finding.status);
        assert!(finding.remedy.is_some());
    }

    #[test]
    fn an_explicitly_named_missing_config_fails() {
        // Only the *default* config path may be absent (defaults apply); a
        // `--config` the user pointed at must exist.
        let dir = tempfile::tempdir().unwrap();
        let (finding, _) = check_config(Some(&dir.path().join("nope.toml")));
        assert_eq!(Status::Fail, finding.status);
    }

    #[test]
    fn a_corrupt_store_reports_fail_with_a_remedy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "not json at all").unwrap();
        let finding = check_store(Some(path));
        assert_eq!(Status::Fail, finding.status);
        assert!(finding.remedy.is_some());
    }

    #[test]
    fn a_readable_store_reports_its_entry_count() {
        let dir = tempfile::tempdir().unwrap();
        let finding = check_store(Some(dir.path().join("progress.json")));
        assert_eq!(Status::Ok, finding.status);
        assert!(finding.detail.contains("0 card entries"));
    }

    #[test]
    fn a_broken_deck_warns_and_points_at_deck_check() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("good.md"), "## f\nb\n").unwrap();
        std::fs::write(dir.path().join("bad.md"), "## front with no answer\n").unwrap();
        let finding = check_decks(dir.path());
        assert_eq!(Status::Warn, finding.status);
        assert!(finding.detail.contains("bad.md"), "{}", finding.detail);
        assert!(finding.remedy.as_deref().unwrap().contains("doctor"));
    }

    #[test]
    fn malformed_math_warns_with_a_bounded_formula_and_valid_or_literal_math_does_not() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("good.md"),
            "## valid $x^2$\n$5 and $10 with unmatched $x\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("bad.md"), "## q\n$\\frac{1$\n> $\\sqrt{$\n").unwrap();

        let finding = check_decks(dir.path());
        assert_eq!(Status::Warn, finding.status);
        assert!(
            finding.detail.contains("2 malformed LaTeX formula(s)"),
            "{}",
            finding.detail
        );
        assert!(finding.detail.contains("bad.md: card at line 1"));
        assert!(finding.detail.contains("\\frac{1"));
    }

    #[test]
    fn a_missing_decks_dir_warns_with_the_fix() {
        let dir = tempfile::tempdir().unwrap();
        let finding = check_decks(&dir.path().join("absent"));
        assert_eq!(Status::Warn, finding.status);
        assert!(finding.remedy.is_some());
    }

    #[test]
    fn a_missing_binary_warns_with_its_remedy() {
        let finding = check_binary(
            "share",
            "definitely-not-a-real-binary-xyz",
            "workspace sharing",
            "install magic-wormhole",
        );
        assert_eq!(Status::Warn, finding.status);
        assert_eq!(Some("install magic-wormhole".to_string()), finding.remedy);
    }
}
