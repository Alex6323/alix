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
            println!("✓ {name} ({cmd}) — ready");
            Ok(())
        }
        Err(e) => {
            eprintln!("✗ {name} ({cmd}) — {e}");
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
            Ok(_) => println!("✓ {name:<name_width$}  ({cmd:<cmd_width$}) — ready"),
            Err(e) => {
                eprintln!("✗ {name:<name_width$}  ({cmd:<cmd_width$}) — {e}");
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
    let probe_cfg = AskConfig {
        // No tools: pure reasoning works across every backend's capabilities
        // and completes quickly.
        allowed_tools: vec![],
        timeout_secs: cfg.timeout_secs.min(15),
        ..cfg.clone()
    };
    ask::run(&probe_cfg, "Reply with exactly: OK", &[])
}
