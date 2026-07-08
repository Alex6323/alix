//! Environment health checks (`alix doctor`): is this setup able to do its
//! job — config, progress store, decks, and the optional external CLIs — with
//! a one-line remedy per finding. Report-only: doctor never fixes anything.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{config::Config, deck::Deck, store::Store, workspace};

/// How much a finding matters: `Fail` breaks the core loop; `Warn` only limits
/// an optional feature (an AI backend, sharing) — the core still works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

/// One line of the doctor's report.
#[derive(Debug)]
pub struct Finding {
    /// What was checked (short, e.g. "config").
    pub name: &'static str,
    pub status: Status,
    /// What was found, one line.
    pub detail: String,
    /// The fix, shown under a non-Ok finding.
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

/// Loads the config, reporting a parse/validation failure instead of dying —
/// the one check that must not assume what every other alix command assumes.
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

/// Opens the progress store at `path` (`None` → the platform default) and
/// reports whether it parses, with its entry count.
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

/// Scans the decks root: top-level decks plus one level of folder/workspace
/// members (the picker's reach), parsing each. Broken decks only break
/// themselves, so they warn — with a pointer at `alix deck check`.
pub fn check_decks(decks_dir: &Path) -> Finding {
    if !decks_dir.is_dir() {
        // The expected state of a fresh install — a warn with the fix, not a
        // failure (nothing is broken; there is just nothing to drill yet).
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
        if path.is_file() && path.extension().is_some_and(|e| e == "txt") {
            deck_files.push(path);
        } else if workspace::has_decks(&path) {
            dirs += 1;
            for sub in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                let p = sub.path();
                if p.is_file() && p.extension().is_some_and(|e| e == "txt") {
                    deck_files.push(p);
                }
            }
        }
    }
    let broken: Vec<String> = deck_files
        .iter()
        .filter(|p| Deck::load(p).is_err())
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let counts = format!(
        "{} decks across {} folders/workspaces — {}",
        deck_files.len(),
        dirs,
        decks_dir.display()
    );
    if broken.is_empty() {
        Finding::ok("decks", counts)
    } else {
        let named = broken
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        Finding::bad(
            "decks",
            Status::Warn,
            format!("{counts}; {} won't parse: {named}", broken.len()),
            "run `alix deck check <file>` on each for the exact line",
        )
    }
}

/// Reports whether an external CLI is on the PATH by spawning
/// `<cmd> --version` (output discarded; no network, no cost). `purpose` says
/// which alix feature needs it — a missing binary only warns.
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
        std::fs::write(dir.path().join("good.txt"), "# f\n\tb\n").unwrap();
        std::fs::write(dir.path().join("bad.txt"), "# front with no answer\n").unwrap();
        let finding = check_decks(dir.path());
        assert_eq!(Status::Warn, finding.status);
        assert!(finding.detail.contains("bad.txt"), "{}", finding.detail);
        assert!(finding.remedy.as_deref().unwrap().contains("deck check"));
    }

    #[test]
    fn a_missing_decks_dir_warns_with_the_fix() {
        // The expected state of a fresh install — nothing is broken yet.
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
