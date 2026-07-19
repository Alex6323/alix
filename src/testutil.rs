use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use crate::config::AskConfig;

/// Serializes tests that write + exec a fake CLI: a concurrent fork would
/// inherit the briefly write-open script fd and fail `exec` with `ETXTBSY`.
static EXEC_LOCK: Mutex<()> = Mutex::new(());

/// Acquires [`EXEC_LOCK`], tolerating poison: a panicking test must not
/// cascade `PoisonError`s into every other test sharing the lock.
pub(crate) fn exec_lock() -> MutexGuard<'static, ()> {
    EXEC_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// Writes an executable fake CLI running `body` and returns its path. `body`
/// must consume stdin, or `ask::run`'s prompt write can race it into a broken pipe.
pub(crate) fn fake_cli(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("fake-claude");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Drains stdin (immune to the broken-pipe race), then prints `reply`. `reply`
/// is written to a file and `cat`-ed so it needs no shell escaping (handy for JSON).
pub(crate) fn fake_reply(dir: &Path, reply: &str) -> PathBuf {
    let out = dir.join("fake-reply");
    std::fs::write(&out, reply).unwrap();
    fake_cli(dir, &format!("cat >/dev/null; cat {}", out.display()))
}

/// Like [`fake_reply`] but for arg-delivery backends: doesn't drain stdin,
/// since `ask::run` closes stdin immediately when the prompt arrives as an argument.
pub(crate) fn fake_arg_reply(dir: &Path, reply: &str) -> PathBuf {
    let out = dir.join("fake-arg-reply");
    std::fs::write(&out, reply).unwrap();
    fake_cli(dir, &format!("cat {}", out.display()))
}

pub(crate) fn ask_config(command: &Path) -> AskConfig {
    AskConfig {
        command: command.to_str().unwrap().to_string(),
        timeout_secs: 10,
        ..AskConfig::default()
    }
}
