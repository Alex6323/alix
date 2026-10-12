use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use crate::config::AskConfig;

/// Serializes EVERY process creation in this test binary, not only the
/// fake-CLI ones: a fork anywhere inherits the briefly write-open script fd
/// of a concurrent `fake_cli`, whose own `exec` then fails with `ETXTBSY`.
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

const STALE_CLOZE_TEACHING: [&str; 7] = [
    "NEVER on the front",
    "below answer lines",
    "below its answer lines",
    "directly below the answer",
    "an answer line that states the fact",
    "Each answer line is full plain prose",
    "short instruction",
];

pub(crate) fn assert_teaches_answerless_cloze(name: &str, prompt: &str) {
    for stale in STALE_CLOZE_TEACHING {
        assert!(
            !prompt.contains(stale),
            "{name} teaches a cloze card with an answer section ({stale:?}): {prompt}"
        );
    }
    let lines: Vec<&str> = prompt.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if !line.starts_with("<!-- blank:") {
            continue;
        }
        let above = index.checked_sub(1).map_or("", |above| lines[above]);
        assert!(
            above.starts_with("## ") || above.starts_with("<!-- blank:"),
            "{name} puts a line between a cloze card's `##` sentence and its blank ({above:?}): {prompt}"
        );
        let sentence_line = lines[..index]
            .iter()
            .rposition(|candidate| candidate.starts_with("## "))
            .expect("a blank line sits under a `##` line, checked above");
        let example = lines[sentence_line..=index].join("\n");
        let cards = crate::parser::parse_str("prompt-example.md", &example)
            .unwrap_or_else(|error| panic!("{name} teaches an unparseable cloze: {error}"));
        assert!(
            cards.iter().all(|card| card.is_blank_card()),
            "{name} teaches a cloze example that is not a cloze card: {example}"
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_fake_cli_config_bounds_the_child_to_ten_seconds() {
        assert_eq!(
            10,
            super::ask_config(std::path::Path::new("fake")).timeout_secs
        );
    }
}
