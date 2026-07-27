//! `alix doctor`: report-only, never fixes anything.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{config::Config, deck::Deck, workspace};

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
    let progress = crate::state::UserFiles::new(&path).progress();
    match crate::store::Store::open(progress) {
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
            "a progress document is unreadable; move it aside to start fresh, or restore the folder from your own backup (your decks are plain files — back them up like any folder)",
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
    let (mut deck_files, mut uninitialized) =
        workspace::classified_deck_files(decks_dir).unwrap_or_default();
    let direct_workspace = workspace::is_workspace(decks_dir);
    let mut dirs = usize::from(direct_workspace);
    if !direct_workspace {
        for entry in std::fs::read_dir(decks_dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let (members, candidates) = workspace::classified_deck_files(&path).unwrap_or_default();
            if !members.is_empty() {
                dirs += 1;
            }
            deck_files.extend(members);
            uninitialized.extend(candidates);
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
                let augment = crate::augment::AugmentCache::open_for_deck(&deck).ok();
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
    if broken.is_empty() && malformed_math.is_empty() && uninitialized.is_empty() {
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
        let uninitialized_detail = if uninitialized.is_empty() {
            String::new()
        } else {
            let names = uninitialized
                .iter()
                .take(3)
                .map(|path| {
                    path.strip_prefix(decks_dir)
                        .unwrap_or(path)
                        .display()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "; {} deck-like Markdown file(s) ignored until initialized: {names}",
                uninitialized.len()
            )
        };
        let parse_detail = if broken.is_empty() {
            String::new()
        } else {
            format!("; {} won't parse: {named}", broken.len())
        };
        let remedy = if uninitialized.is_empty() {
            "run `alix doctor <file>` for the exact deck diagnostics"
        } else {
            "run `alix deck init <file>` for each intended deck; leave ordinary Markdown unchanged"
        };
        Finding::bad(
            "decks",
            Status::Warn,
            format!("{counts}{parse_detail}{math_detail}{uninitialized_detail}"),
            remedy,
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
        std::fs::create_dir(dir.path().join("progress")).unwrap();
        let path = dir.path().join("progress/deck1.json");
        std::fs::write(&path, "not json at all").unwrap();
        let finding = check_store(Some(dir.path().to_path_buf()));
        assert_eq!(Status::Fail, finding.status);
        assert!(finding.remedy.is_some());
    }

    #[test]
    fn a_readable_store_reports_its_entry_count() {
        let dir = tempfile::tempdir().unwrap();
        let finding = check_store(Some(dir.path().to_path_buf()));
        assert_eq!(Status::Ok, finding.status);
        assert!(finding.detail.contains("0 card entries"));
    }

    #[test]
    fn a_broken_deck_warns_and_points_at_deck_check() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("good.md"),
            "---\nalix-id: \"good\"\n---\n## f\nb\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("bad.md"),
            "---\nalix-id: \"bad\"\n---\n## front with no answer\n",
        )
        .unwrap();
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
            "---\nalix-id: \"good\"\n---\n## valid $x^2$\n$5 and $10 with unmatched $x\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("bad.md"),
            "---\nalix-id: \"bad\"\n---\n## q\n$\\frac{1$\n> $\\sqrt{$\n",
        )
        .unwrap();

        let finding = check_decks(dir.path());
        assert_eq!(Status::Warn, finding.status);
        assert!(
            finding.detail.contains("2 malformed LaTeX formula(s)"),
            "{}",
            finding.detail
        );
        assert!(finding.detail.contains("bad.md: card at line 4"));
        assert!(finding.detail.contains("\\frac{1"));
    }

    #[test]
    fn deck_like_markdown_is_reported_as_ignored_without_being_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        let original = "# Notes\n\n## Design\nordinary prose\n";
        std::fs::write(&path, original).unwrap();

        let finding = check_decks(dir.path());

        assert_eq!(Status::Warn, finding.status);
        assert!(finding.detail.contains("ignored until initialized"));
        assert!(finding.detail.contains("notes.md"));
        assert!(finding.remedy.as_deref().unwrap().contains("deck init"));
        assert_eq!(original, std::fs::read_to_string(path).unwrap());
    }

    #[test]
    fn a_direct_workspace_counts_each_member_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(workspace::MANIFEST), "").unwrap();
        std::fs::create_dir(dir.path().join(workspace::DECKS)).unwrap();
        std::fs::write(
            dir.path().join("decks/member.md"),
            "---\nalix-id: \"member\"\n---\n## q\nanswer\n",
        )
        .unwrap();

        let finding = check_decks(dir.path());

        assert_eq!(Status::Ok, finding.status);
        assert!(
            finding.detail.starts_with("1 decks across 1"),
            "{}",
            finding.detail
        );
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
