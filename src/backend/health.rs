//! Backend health probe: sends a trivial tool-free prompt to the configured
//! backend (or to all four) so a user can confirm that a backend is installed,
//! signed in, and responding before running a longer AI operation.

use anyhow::Result;

use crate::{
    ask,
    backend::backend_for,
    config::{AskConfig, BackendKind},
};

/// Probes the configured backend by default, or all four [`BackendKind`]s when
/// `all` is `true`. For each backend a trivial tool-free prompt is sent; the
/// result is classified and printed as a status line. Returns `Ok(())` when
/// every probed backend replied successfully, or an error when at least one
/// failed.
pub fn check(cfg: &AskConfig, all: bool) -> Result<()> {
    if all {
        check_all(cfg)
    } else {
        check_one(cfg)
    }
}

/// Probes the single backend named in `cfg`, printing one status line and
/// returning `Ok(())` on a successful reply.
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

/// Probes all four backends and prints a status line per backend. Returns
/// `Ok(())` when every backend succeeded, or an error summary when at least
/// one failed.
fn check_all(cfg: &AskConfig) -> Result<()> {
    let kinds = [
        BackendKind::Claude,
        BackendKind::Gemini,
        BackendKind::Codex,
        BackendKind::Copilot,
    ];

    // Collect each backend's command name so the table can be pre-aligned.
    let rows: Vec<(BackendKind, String, String)> = kinds
        .iter()
        .map(|&kind| {
            let per_kind = AskConfig {
                backend: kind,
                ..cfg.clone()
            };
            let backend = backend_for(&per_kind)
                .expect("all four kinds are wired in backend_for");
            let name = backend.name().to_string();
            let cmd = backend.command().to_string();
            (kind, name, cmd)
        })
        .collect();

    let name_width = rows.iter().map(|(_, n, _)| n.len()).max().unwrap_or(0);
    let cmd_width = rows.iter().map(|(_, _, c)| c.len()).max().unwrap_or(0);

    let mut any_failed = false;
    for (kind, name, cmd) in &rows {
        let per_kind = AskConfig {
            backend: *kind,
            ..cfg.clone()
        };
        match probe(&per_kind) {
            Ok(_) => println!(
                "✓ {name:<name_width$}  ({cmd:<cmd_width$}) — ready"
            ),
            Err(e) => {
                eprintln!(
                    "✗ {name:<name_width$}  ({cmd:<cmd_width$}) — {e}"
                );
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

/// Sends a trivial tool-free prompt to the backend described by `cfg` and
/// returns the reply on success, or the mapped failure message on error.
/// The `ask::run` plumbing already applies `map_run_failure` (Task 7) before
/// returning the error, so the message here is already user-facing.
fn probe(cfg: &AskConfig) -> Result<String> {
    let probe_cfg = AskConfig {
        // No tools — pure reasoning, so the call works regardless of the
        // backend's tool capabilities and completes quickly.
        allowed_tools: vec![],
        // Use a short timeout: this is a health check, not a real query.
        timeout_secs: cfg.timeout_secs.min(15),
        ..cfg.clone()
    };
    ask::run(&probe_cfg, "Reply with exactly: OK", &[])
}
