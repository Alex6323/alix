//! `alix doctor`'s checks: report-only by default; mutations exist only
//! behind explicit CLI flags (`--repair-source-locators`,
//! `--remove-backup-files`), never in these functions.

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
            format!("{}: {e:#}", path.display()),
            "a progress document is unreadable; move it aside to start fresh, or restore the folder from your own backup (your decks are plain files — back them up like any folder)",
        ),
    }
}

pub fn check_log(path: Option<PathBuf>) -> Finding {
    match path {
        Some(path) => Finding::ok("log", path.display().to_string()),
        None => Finding::bad(
            "log",
            Status::Fail,
            "cannot determine the state directory",
            "set HOME so alix has somewhere to keep its local server log",
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
    let found = match workspace::classify_deck_files(decks_dir) {
        Ok(found) => found,
        Err(source) => {
            return Finding::bad(
                "decks",
                Status::Fail,
                format!("cannot read {}: {source}", decks_dir.display()),
                "check the folder's permissions, or serve another folder (`alix <dir>`)",
            );
        }
    };
    let mut deck_files = found.initialized;
    let mut uninitialized = found.uninitialized;
    let mut unreadable: Vec<String> = found
        .unreadable
        .iter()
        .map(|(path, source)| format!("{}: {source}", path.display()))
        .collect();
    let direct_workspace = workspace::is_workspace(decks_dir);
    let mut dirs = usize::from(direct_workspace);
    if !direct_workspace {
        let mut children: Vec<PathBuf> = std::fs::read_dir(decks_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        children.sort();
        let mut walked = workspace::SeenPaths::default();
        for path in children {
            if !walked.first_visit(&path) {
                continue;
            }
            let found = match workspace::classify_deck_files(&path) {
                Ok(found) => found,
                Err(source) => {
                    unreadable.push(format!("{}: {source}", path.display()));
                    continue;
                }
            };
            if !found.initialized.is_empty() {
                dirs += 1;
            }
            deck_files.extend(found.initialized);
            uninitialized.extend(found.uninitialized);
            unreadable.extend(
                found
                    .unreadable
                    .iter()
                    .map(|(path, source)| format!("{}: {source}", path.display())),
            );
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
    if broken.is_empty()
        && malformed_math.is_empty()
        && uninitialized.is_empty()
        && unreadable.is_empty()
    {
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
        let unreadable_detail = if unreadable.is_empty() {
            String::new()
        } else {
            format!(
                "; {} path(s) alix cannot read, first: {}",
                unreadable.len(),
                unreadable[0]
            )
        };
        let remedy = if !unreadable.is_empty() {
            "check the permissions on the paths alix cannot read, then rerun"
        } else if uninitialized.is_empty() {
            "run `alix doctor <file>` for the exact deck diagnostics"
        } else {
            "run `alix deck init <file>` for each intended deck; leave ordinary Markdown \
             unchanged"
        };
        Finding::bad(
            "decks",
            Status::Warn,
            format!("{counts}{parse_detail}{math_detail}{uninitialized_detail}{unreadable_detail}"),
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
        Finding::ok(name, format!("`{cmd}` found: {purpose}"))
    } else {
        Finding::bad(
            name,
            Status::Warn,
            format!("`{cmd}` not found: {purpose} unavailable"),
            remedy,
        )
    }
}

/// Every `*.bak` under `root`, recursively: the backups `alix deck restore`
/// swaps in, left behind by overwrites (`deck import --force`, deck
/// regeneration). Dot-directories are skipped.
pub fn backup_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut walked = workspace::SeenPaths::default();
    let mut collected = workspace::SeenPaths::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if !walked.first_visit(&dir) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !name.starts_with('.') {
                    stack.push(path);
                }
            } else if name.ends_with(".bak") && collected.first_visit(&path) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// A finding only when backups exist: their absence is the healthy state
/// and prints nothing.
pub fn check_backups(root: &Path) -> Option<Finding> {
    let files = backup_files(root);
    if files.is_empty() {
        return None;
    }
    let bytes: u64 = files
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| m.len())
        .sum();
    Some(Finding {
        name: "backups",
        status: Status::Warn,
        detail: format!(
            "{} backup file(s), {} KiB, from overwrites",
            files.len(),
            bytes.div_ceil(1024)
        ),
        remedy: Some(
            "swap one back: `alix deck restore <deck>` — or delete all: \
             `alix doctor <dir> --remove-backup-files`"
                .into(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A confirmed cleanup that deletes the file and then fails on its second
    /// spelling leaves the user unable to tell whether the cleanup completed.
    #[cfg(unix)]
    #[test]
    fn backup_scanning_counts_one_physical_file_once_through_an_alias() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("facts.md.bak"), "x").unwrap();

        for (shape, link, target) in [
            ("a directory alias", root.path().join("alias"), real.clone()),
            (
                "a file alias",
                root.path().join("copy.md.bak"),
                real.join("facts.md.bak"),
            ),
        ] {
            std::os::unix::fs::symlink(&target, &link).unwrap();

            let found = backup_files(root.path());

            assert_eq!(
                1,
                found.len(),
                "{shape}: one physical backup reached under two names is one file to delete: {found:?}"
            );
            std::fs::remove_file(&link).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_aliases_do_not_count_one_unreadable_workspace_twice() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let workspace_dir = root.path().join("nested");
        let decks = workspace_dir.join(workspace::DECKS);
        std::fs::create_dir_all(&decks).unwrap();
        std::fs::write(workspace_dir.join(workspace::MANIFEST), "").unwrap();
        let deck = decks.join("locked.md");
        std::fs::write(&deck, "## q\na\n").unwrap();
        std::fs::set_permissions(&deck, std::fs::Permissions::from_mode(0o000)).unwrap();
        std::os::unix::fs::symlink(&workspace_dir, root.path().join("alias")).unwrap();
        std::os::unix::fs::symlink(root.path(), workspace_dir.join("back-to-root")).unwrap();

        let finding = check_decks(root.path());

        std::fs::set_permissions(&deck, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(Status::Warn, finding.status, "{finding:?}");
        assert!(
            finding.detail.contains("1 path(s) alix cannot read"),
            "one physical workspace reachable by a symlink alias and cycle must contribute its unreadable member once: {finding:?}"
        );
    }

    #[test]
    fn backup_scanning_finds_nested_baks_and_stays_silent_when_clean() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            check_backups(dir.path()).is_none(),
            "a clean tree produces no finding"
        );

        std::fs::create_dir_all(dir.path().join("progress")).unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join("a.md"), "live").unwrap();
        std::fs::write(dir.path().join("a.md.bak"), "bak").unwrap();
        std::fs::write(dir.path().join("progress/d.json.bak"), "bak").unwrap();
        std::fs::write(dir.path().join(".git/ref.bak"), "hidden").unwrap();

        let files = backup_files(dir.path());
        assert_eq!(2, files.len(), "dot-dirs are skipped: {files:?}");

        let finding = check_backups(dir.path()).unwrap();
        assert_eq!(Status::Warn, finding.status, "advice, never a failure");
        assert!(
            finding.detail.contains("2 backup file(s)"),
            "{}",
            finding.detail
        );
        let remedy = finding.remedy.as_deref().unwrap_or_default();
        assert!(remedy.contains("deck restore"), "{remedy}");
        assert!(remedy.contains("--remove-backup-files"), "{remedy}");
    }

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
            "---\nformat-version: 1\nid: \"deck-good\"\n---\n## f\nb\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("bad.md"),
            "---\nformat-version: 1\nid: \"deck-bad\"\n---\n## front with no answer\n",
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
            "---\nformat-version: 1\nid: \"deck-good\"\n---\n## valid $x^2$\n$5 and $10 with unmatched $x\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("bad.md"),
            "---\nformat-version: 1\nid: \"deck-bad\"\n---\n## q\n$\\frac{1$\n> $\\sqrt{$\n",
        )
        .unwrap();

        let finding = check_decks(dir.path());
        assert_eq!(Status::Warn, finding.status);
        assert!(
            finding.detail.contains("2 malformed LaTeX formula(s)"),
            "{}",
            finding.detail
        );
        assert!(finding.detail.contains("bad.md: card at line 5"));
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
    fn a_deck_with_an_unknown_key_is_an_ordinary_uninitialized_candidate() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("old.md"),
            "---\norigin: ../src\n---\n## q\na\n",
        )
        .unwrap();

        let finding = check_decks(dir.path());

        assert_eq!(Status::Warn, finding.status);
        assert!(
            finding.detail.contains("ignored until initialized"),
            "{}",
            finding.detail
        );
        assert!(
            !finding.detail.contains("un-converted"),
            "{}",
            finding.detail
        );
        let remedy = finding.remedy.as_deref().unwrap();
        assert!(remedy.contains("deck init"), "{remedy}");
    }

    #[test]
    fn a_direct_workspace_counts_each_member_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(workspace::MANIFEST), "").unwrap();
        std::fs::create_dir(dir.path().join(workspace::DECKS)).unwrap();
        std::fs::write(
            dir.path().join("decks/member.md"),
            "---\nformat-version: 1\nid: \"deck-member\"\n---\n## q\nanswer\n",
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
    fn a_collection_counts_only_child_folders_that_contain_decks() {
        let dir = tempfile::tempdir().unwrap();
        let populated = dir.path().join("populated");
        let also_populated = dir.path().join("also-populated");
        let empty = dir.path().join("empty");
        std::fs::create_dir(&populated).unwrap();
        std::fs::create_dir(&also_populated).unwrap();
        std::fs::create_dir(&empty).unwrap();
        std::fs::write(dir.path().join("ordinary-file"), "not a folder").unwrap();
        std::fs::write(
            populated.join("member.md"),
            "---\nformat-version: 1\nid: \"deck-member\"\n---\n## q\nanswer\n",
        )
        .unwrap();
        std::fs::write(
            also_populated.join("member.md"),
            "---\nformat-version: 1\nid: \"deck-other\"\n---\n## q\nanswer\n",
        )
        .unwrap();

        let finding = check_decks(dir.path());

        assert_eq!(Status::Ok, finding.status);
        assert!(
            finding
                .detail
                .starts_with("2 decks across 2 folders/workspaces"),
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
        #[cfg(all(unix, feature = "full"))]
        let _lock = crate::testutil::exec_lock();
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
