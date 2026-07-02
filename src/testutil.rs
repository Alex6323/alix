//! Shared test helpers for the AI features.
//!
//! `ask`, `exam`, and `trace` all drive Claude through [`crate::ask::run`] and
//! fake it with a tiny on-disk `claude` script. This centralises the helpers
//! they used to copy-paste, and bakes in the two things that have bitten us:
//! every reply CLI drains stdin (or `ask::run`'s prompt write races the child
//! into a broken pipe under load), and the exec lock is poison-tolerant (a
//! panicking test must not cascade `PoisonError`s into the others).

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use crate::config::AskConfig;

/// Serializes tests that write + exec a fake CLI: a concurrent fork would
/// inherit the briefly write-open script fd and fail `exec` with `ETXTBSY`.
static EXEC_LOCK: Mutex<()> = Mutex::new(());

/// Acquires [`EXEC_LOCK`], tolerating poison. A test that panics while holding
/// the guard would otherwise poison the mutex and cascade into spurious
/// `PoisonError`s in every other test that shares it.
pub(crate) fn exec_lock() -> MutexGuard<'static, ()> {
    EXEC_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// Writes an executable fake `claude` at `<dir>/fake-claude` running the shell
/// `body`, and returns its path. The body **must consume stdin** — `ask::run`
/// feeds the prompt through stdin, and a body that exits without reading races
/// the write into a broken pipe. For the common "reply with fixed text" case,
/// use [`fake_reply`], which drains stdin for you.
pub(crate) fn fake_cli(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("fake-claude");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A fake CLI that drains stdin, then prints `reply` verbatim. The reply is
/// `cat`-ed from a file so it needs no shell escaping (handy for JSON), and
/// draining first makes it immune to the broken-pipe race regardless of timing.
pub(crate) fn fake_reply(dir: &Path, reply: &str) -> PathBuf {
    let out = dir.join("fake-reply");
    std::fs::write(&out, reply).unwrap();
    fake_cli(dir, &format!("cat >/dev/null; cat {}", out.display()))
}

/// A fake CLI for **arg-delivery** backends (Codex `exec`): it prints `reply`
/// verbatim and needs no stdin, because the prompt arrives as a command-line
/// argument, not on stdin. `ask::run` closes stdin immediately for arg
/// delivery, so unlike [`fake_reply`] this script deliberately does *not* drain
/// it. The reply is `cat`-ed from a file so it needs no shell escaping.
pub(crate) fn fake_arg_reply(dir: &Path, reply: &str) -> PathBuf {
    let out = dir.join("fake-arg-reply");
    std::fs::write(&out, reply).unwrap();
    fake_cli(dir, &format!("cat {}", out.display()))
}

/// An [`AskConfig`] pointing at `command`, with a short test timeout.
pub(crate) fn ask_config(command: &Path) -> AskConfig {
    AskConfig {
        command: command.to_str().unwrap().to_string(),
        timeout_secs: 10,
        ..AskConfig::default()
    }
}
