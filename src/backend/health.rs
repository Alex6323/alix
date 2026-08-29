use anyhow::Result;

use crate::{
    ask,
    backend::backend_for,
    config::{AskConfig, BackendKind},
};

pub fn check(cfg: &AskConfig, all: bool) -> Result<()> {
    if all { check_all(cfg) } else { check_one(cfg) }
}

fn check_one(cfg: &AskConfig) -> Result<()> {
    let backend = backend_for(cfg)?;
    let name = backend.name();
    let cmd = &cfg.command;
    match probe(cfg) {
        Ok(_) => {
            println!("✓ {name} ({cmd}): ready");
            Ok(())
        }
        Err(e) => {
            eprintln!("✗ {name} ({cmd}): {e}");
            anyhow::bail!("backend check failed")
        }
    }
}

fn check_all(cfg: &AskConfig) -> Result<()> {
    let kinds = [
        BackendKind::Claude,
        BackendKind::Gemini,
        BackendKind::Codex,
        BackendKind::Copilot,
    ];

    let mut rows: Vec<(BackendKind, String, String)> = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let per_kind = AskConfig {
            backend: kind,
            ..cfg.clone()
        };
        let backend = backend_for(&per_kind)?;
        let name = backend.name().to_string();
        let cmd = backend.command().to_string();
        rows.push((kind, name, cmd));
    }

    let name_width = rows.iter().map(|(_, n, _)| n.len()).max().unwrap_or(0);
    let cmd_width = rows.iter().map(|(_, _, c)| c.len()).max().unwrap_or(0);

    let mut any_failed = false;
    for (kind, name, cmd) in &rows {
        let per_kind = AskConfig {
            backend: *kind,
            ..cfg.clone()
        };
        match probe(&per_kind) {
            Ok(_) => println!("✓ {name:<name_width$}  ({cmd:<cmd_width$}): ready"),
            Err(e) => {
                eprintln!("✗ {name:<name_width$}  ({cmd:<cmd_width$}): {e}");
                any_failed = true;
            }
        }
    }

    if any_failed {
        anyhow::bail!("one or more backends failed the health check")
    } else {
        Ok(())
    }
}

// ask::run already maps the failure to a user-facing message, so don't
// reformat it here.
fn probe(cfg: &AskConfig) -> Result<String> {
    ask::run(&probe_config(cfg), "Reply with exactly: OK", &[])
}

fn probe_config(cfg: &AskConfig) -> AskConfig {
    AskConfig {
        // No tools: pure reasoning works across every backend's capabilities
        // and completes quickly.
        allowed_tools: vec![],
        timeout_secs: cfg.timeout_secs.min(15),
        ..cfg.clone()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::testutil::{ask_config, exec_lock, fake_cli};

    #[test]
    fn the_probe_run_is_the_parent_with_no_tools_and_a_fifteen_second_ceiling() {
        for (parent_timeout, expected_timeout) in [(9, 9), (30, 15)] {
            let parent = AskConfig {
                allowed_tools: vec!["ParentTool".to_string()],
                timeout_secs: parent_timeout,
                cwd: Some(PathBuf::from("/parent")),
                source_access: true,
                model: Some("parent-model".to_string()),
                ..AskConfig::default()
            };
            let expected = AskConfig {
                allowed_tools: Vec::new(),
                timeout_secs: expected_timeout,
                ..parent.clone()
            };

            assert_eq!(expected, probe_config(&parent));
        }
    }

    #[test]
    fn the_probe_process_observes_the_parent_working_directory() {
        let _lock = exec_lock();
        let cwd = tempfile::tempdir().unwrap();
        // `pwd` reports the child's resolved directory, and on macOS the temp
        // path traverses the /var -> /private/var symlink, so the expectation
        // has to be the canonical form the child will print.
        let expected = cwd.path().canonicalize().unwrap();
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(cli_dir.path(), "cat >/dev/null; pwd");
        let cfg = AskConfig {
            cwd: Some(cwd.path().to_path_buf()),
            ..ask_config(&cli)
        };

        assert_eq!(expected.to_string_lossy(), probe(&cfg).unwrap().trim());
    }
}
