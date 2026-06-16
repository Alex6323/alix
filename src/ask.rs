//! Ask-Claude integration: send questions about the current card to the
//! Claude Code CLI (`claude -p`) and get explanations back, without leaving
//! the review session.
//!
//! The CLI is run in a background thread; the TUI polls the returned channel
//! so the interface stays responsive. One CLI session (see [`CliSession`])
//! spans the whole review run: the first call creates it with
//! `--session-id`, later calls `--resume` it, so Claude remembers earlier
//! cards, questions, and any deck links it fetched.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::card::Card;
use crate::config::AskConfig;

/// One question/answer exchange.
pub type Exchange = (String, String);

/// What the background thread eventually delivers.
pub enum Reply {
    Answer(String),
    Error(String),
}

/// The CLI conversation spanning one review run.
pub struct CliSession {
    id: String,
    /// Whether the session has been created on the CLI side (a first call
    /// succeeded).
    pub started: bool,
}

impl CliSession {
    /// Creates a session with a fresh random ID.
    pub fn new() -> Self {
        Self { id: random_uuid(), started: false }
    }

    /// The CLI arguments that create or resume this session.
    pub fn args(&self) -> Vec<String> {
        if self.started {
            vec!["--resume".to_string(), self.id.clone()]
        } else {
            vec!["--session-id".to_string(), self.id.clone()]
        }
    }
}

impl Default for CliSession {
    fn default() -> Self {
        Self::new()
    }
}

/// A random version-4 UUID, generated without extra dependencies
/// (SplitMix64 over wall clock, process id, and a per-process counter so
/// two ids created in the same millisecond still differ).
fn random_uuid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut state = crate::time::now_ms()
        ^ ((std::process::id() as u64) << 32)
        ^ nonce.wrapping_mul(0xA076_1D64_78BD_642F);
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&next().to_le_bytes());
    bytes[8..].copy_from_slice(&next().to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    let b = bytes;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// Builds the prompt for a question about `card`.
///
/// The first message of a session carries the tutoring instructions and the
/// deck's reference links; follow-ups only need the (possibly new) card and
/// the question, because the CLI session remembers the rest.
pub fn question_prompt(
    card: &Card,
    links: &[String],
    question: &str,
    first: bool,
) -> String {
    let mut p = String::new();
    if first {
        p.push_str(
            "You are a concise tutor inside a terminal flashcard application. \
             The user reviews flashcards and asks you questions about them; \
             this conversation continues across several cards. Always answer \
             in plain text without any markdown formatting, in at most six \
             short sentences, specific to the card at hand.\n",
        );
        if !links.is_empty() {
            p.push_str(
                "\nReference links for this deck — fetch them (WebFetch) when \
                 they can improve an answer; you only need to read each \
                 once:\n",
            );
            for link in links {
                p.push_str(link);
                p.push('\n');
            }
        }
        p.push('\n');
    }
    p.push_str("The card being reviewed:\n\n");
    push_card(&mut p, card);
    p.push_str("\nThe user's question: ");
    p.push_str(question);
    p
}

/// Builds the prompt that condenses a conversation into note lines for the
/// deck file.
pub fn condense_prompt(card: &Card, transcript: &[Exchange]) -> String {
    let mut p = String::from(
        "Below is a flashcard and a conversation the learner had about it. \
         Condense the key insight of the conversation into AT MOST three \
         short lines (each under 100 characters) that are worth rereading \
         the next time this card comes up. Output ONLY those lines: plain \
         text, no markdown, no bullets, no numbering.\n\n",
    );
    push_card(&mut p, card);
    for (q, a) in transcript {
        p.push_str("\nQuestion: ");
        p.push_str(q);
        p.push_str("\nAnswer: ");
        p.push_str(a);
        p.push('\n');
    }
    p
}

fn push_card(p: &mut String, card: &Card) {
    p.push_str("Deck: ");
    p.push_str(&card.subject);
    p.push_str("\nFront: ");
    p.push_str(&card.front);
    p.push_str("\nAnswer:\n");
    for line in &card.back {
        p.push_str(line);
        p.push('\n');
    }
    if let Some(note) = &card.note {
        p.push_str("Note: ");
        p.push_str(note);
        p.push('\n');
    }
}

/// Cleans the condense response into note lines: trims, strips accidental
/// bullets/markup, drops empties, keeps at most three.
pub fn extract_note_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim().trim_start_matches(['!', '-', '*', '•']).trim().to_string())
        .filter(|l| !l.is_empty())
        .take(3)
        .collect()
}

/// Runs the CLI in a background thread; the reply arrives on the returned
/// channel. The caller polls it with `try_recv`. `extra_args` carries the
/// session arguments (`--session-id`/`--resume`).
pub fn spawn(
    config: AskConfig,
    prompt: String,
    extra_args: Vec<String>,
) -> Receiver<Reply> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let reply = match run(&config, &prompt, &extra_args) {
            Ok(answer) => Reply::Answer(answer),
            Err(e) => Reply::Error(format!("{e:#}")),
        };
        // The receiver may be gone if the user left the ask view.
        let _ = tx.send(reply);
    });
    rx
}

/// Runs the CLI in print mode, feeding the prompt through stdin, with a
/// timeout. The permission mode and tool allowlist come from the config; the
/// default `dontAsk` + `[WebFetch, WebSearch]` lets Claude consult deck links
/// without ever blocking on an (unanswerable) permission prompt, while
/// denying every other tool.
pub(crate) fn run(
    config: &AskConfig,
    prompt: &str,
    extra_args: &[String],
) -> Result<String> {
    let mut cmd = Command::new(&config.command);
    cmd.args(["-p", "--output-format", "text"]);
    if !config.allowed_tools.is_empty() {
        cmd.arg("--allowedTools");
        cmd.args(&config.allowed_tools);
    }
    if !config.permission_mode.is_empty() {
        cmd.args(["--permission-mode", &config.permission_mode]);
    }
    if let Some(model) = &config.model {
        cmd.args(["--model", model]);
    }
    cmd.args(extra_args);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run '{}' — is it installed?", config.command))?;

    // Feed the prompt and close stdin so the CLI starts processing.
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(prompt.as_bytes())
        .context("cannot write the prompt")?;

    // Drain output on reader threads so the child never blocks on a full
    // pipe while this thread watches the deadline.
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let deadline = Instant::now() + Duration::from_secs(config.timeout_secs);
    let status = loop {
        if let Some(status) = child.try_wait().context("cannot wait for the CLI")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("'{}' timed out after {}s", config.command, config.timeout_secs);
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if !status.success() {
        let detail = stderr.trim();
        let detail = if detail.is_empty() { stdout.trim() } else { detail };
        bail!("'{}' failed: {}", config.command, truncate(detail, 300));
    }
    let answer = stdout.trim().to_string();
    if answer.is_empty() {
        bail!("'{}' returned an empty answer", config.command);
    }
    Ok(answer)
}

fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    fn card() -> Card {
        Card::plain(
            Arc::from("deck.txt"),
            "Why?".to_string(),
            vec!["Because.".to_string()],
            Some("a note".to_string()),
            1,
        )
    }

    #[test]
    fn first_question_prompt_has_instructions_and_links() {
        let links = vec!["https://docs.rs/tokio".to_string()];
        let p = question_prompt(&card(), &links, "and why that?", true);
        assert!(p.contains("concise tutor"));
        assert!(p.contains("https://docs.rs/tokio"));
        assert!(p.contains("Deck: deck.txt"));
        assert!(p.contains("Front: Why?"));
        assert!(p.contains("Because."));
        assert!(p.contains("Note: a note"));
        assert!(p.ends_with("The user's question: and why that?"));
    }

    #[test]
    fn followup_prompt_is_short_but_carries_the_card() {
        let links = vec!["https://docs.rs/tokio".to_string()];
        let p = question_prompt(&card(), &links, "next q", false);
        // The session already knows the instructions and the links.
        assert!(!p.contains("concise tutor"));
        assert!(!p.contains("docs.rs"));
        // But the card may have changed, so it is always included.
        assert!(p.contains("Front: Why?"));
        assert!(p.ends_with("The user's question: next q"));
    }

    #[test]
    fn first_prompt_without_links_offers_none() {
        let p = question_prompt(&card(), &[], "q", true);
        assert!(!p.contains("Reference links"));
    }

    #[test]
    fn session_args_create_then_resume() {
        let mut session = CliSession::new();
        let create = session.args();
        assert_eq!("--session-id", create[0]);
        session.started = true;
        let resume = session.args();
        assert_eq!("--resume", resume[0]);
        // Same conversation in both calls.
        assert_eq!(create[1], resume[1]);
    }

    #[test]
    fn session_ids_are_distinct_valid_uuids() {
        let a = CliSession::new();
        let b = CliSession::new();
        assert_ne!(a.id, b.id);
        // 8-4-4-4-12 hex with version 4 and RFC variant.
        let parts: Vec<&str> = a.id.split('-').collect();
        assert_eq!(
            vec![8, 4, 4, 4, 12],
            parts.iter().map(|p| p.len()).collect::<Vec<_>>()
        );
        assert!(a.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].chars().next(), Some('8' | '9' | 'a' | 'b')));
    }

    #[test]
    fn condense_prompt_contains_conversation() {
        let transcript = vec![("q".to_string(), "a".to_string())];
        let p = condense_prompt(&card(), &transcript);
        assert!(p.contains("AT MOST three"));
        assert!(p.contains("Question: q"));
        assert!(p.contains("Answer: a"));
    }

    #[test]
    fn extract_note_lines_cleans_and_caps() {
        let text = "- first insight\n\n* second insight\n! third\nfourth\n";
        assert_eq!(
            vec!["first insight", "second insight", "third"],
            extract_note_lines(text)
        );
        assert!(extract_note_lines("  \n\n").is_empty());
    }

    /// Serializes the tests that write + execute scripts: a concurrent fork
    /// in another test inherits the briefly write-open script fd, and the
    /// exec then fails with ETXTBSY.
    static EXEC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Writes a small fake CLI script and returns its path.
    fn fake_cli(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("fake-claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn config(command: &std::path::Path, timeout_secs: u64) -> AskConfig {
        AskConfig {
            command: command.to_str().unwrap().to_string(),
            model: None,
            timeout_secs,
            ..AskConfig::default()
        }
    }

    #[test]
    fn run_returns_stdout_of_the_cli() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // Echo the prompt back (reads stdin like the real CLI).
        let cli = fake_cli(dir.path(), "cat");
        let answer = run(&config(&cli, 10), "hello there", &[]).unwrap();
        assert_eq!("hello there", answer);
    }

    #[test]
    fn run_passes_session_args_to_the_cli() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // Echo the received arguments instead of the prompt.
        let cli = fake_cli(dir.path(), "echo \"$@\"; cat > /dev/null");
        let extra = vec!["--resume".to_string(), "abc".to_string()];
        let answer = run(&config(&cli, 10), "x", &extra).unwrap();
        assert!(answer.contains("--resume abc"), "args were: {answer}");
        assert!(answer.contains("--allowedTools WebFetch WebSearch"));
        // The permission mode must be passed, or the real CLI hangs in -p
        // mode waiting for an approval it cannot receive.
        assert!(answer.contains("--permission-mode dontAsk"), "args were: {answer}");
    }

    #[test]
    fn run_reports_failures_with_stderr() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "echo 'not logged in' >&2; exit 1");
        let err = run(&config(&cli, 10), "x", &[]).unwrap_err();
        assert!(format!("{err:#}").contains("not logged in"));
    }

    #[test]
    fn run_times_out() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "sleep 30");
        let err = run(&config(&cli, 1), "x", &[]).unwrap_err();
        assert!(format!("{err:#}").contains("timed out"));
    }

    #[test]
    fn run_rejects_missing_command() {
        let config = AskConfig {
            command: "/nonexistent/claude".to_string(),
            model: None,
            timeout_secs: 1,
            ..AskConfig::default()
        };
        assert!(run(&config, "x", &[]).is_err());
    }

    #[test]
    fn spawn_delivers_on_the_channel() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "cat");
        let rx = spawn(config(&cli, 10), "ping".to_string(), Vec::new());
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Reply::Answer(a) => assert_eq!("ping", a),
            Reply::Error(e) => panic!("unexpected error: {e}"),
        }
    }
}
