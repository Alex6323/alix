//! Ask-Claude integration: send questions about the current card to the
//! Claude Code CLI (`claude -p`) and get explanations back, without leaving
//! the review session.
//!
//! The CLI is run in a background thread; the web server polls the returned
//! channel so its request loop stays responsive. One CLI session (see
//! [`CliSession`]) spans the whole review run: the first call creates it with
//! `--session-id`, later calls `--resume` it, so Claude remembers earlier
//! cards, questions, and any deck links it fetched.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

use crate::{
    backend::{Access, PromptDelivery, RunOpts, backend_for},
    card::Card,
    config::AskConfig,
};

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
    /// The working directory the conversation was created in. Claude scopes
    /// conversation history per directory, so `--resume` only finds it when run
    /// in the same `cwd` as the `--session-id` that created it.
    cwd: Option<PathBuf>,
}

impl CliSession {
    /// Creates a session with a fresh random ID.
    pub fn new() -> Self {
        Self {
            id: random_uuid(),
            started: false,
            cwd: None,
        }
    }

    /// The CLI arguments that create or resume this session.
    pub fn args(&self) -> Vec<String> {
        if self.started {
            vec!["--resume".to_string(), self.id.clone()]
        } else {
            vec!["--session-id".to_string(), self.id.clone()]
        }
    }

    /// Like [`args`](Self::args), but for a call that will run in `cwd`. Because
    /// Claude stores conversation history per working directory, a `--resume`
    /// only finds the conversation when run in the directory that created it. If
    /// `cwd` differs from where this session started — e.g. moving to a card
    /// grounded in a different `% source:` root, or from a grounded question to
    /// an ungrounded one — the old conversation is unreachable, so start a fresh
    /// session in the new directory instead of emitting a doomed `--resume`.
    /// Callers read [`started`](Self::started) *after* this to decide whether the
    /// prompt is a first message (it is, once a cwd change resets the session).
    pub fn args_in(&mut self, cwd: Option<&Path>) -> Vec<String> {
        if self.started && self.cwd.as_deref() != cwd {
            *self = Self::new();
        }
        self.cwd = cwd.map(Path::to_path_buf);
        self.args()
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
/// the question, because the CLI session remembers the rest. When `source_root`
/// is `Some` (the `[ask] source_access` opt-in), every message reminds Claude it
/// can read the card's source there and must verify against it — the current
/// card's root, so it stays right even as the conversation moves between decks.
/// The reply a frozen card's tutor gives when its source can't be found — the
/// learner can then remove or update the card. Kept verbatim so the frontends and
/// tests agree on the exact wording.
pub const SOURCE_NOT_FOUND: &str =
    "I couldn't find the source material of this card to provide a grounded answer.";

pub fn question_prompt(
    card: &Card,
    links: &[String],
    question: &str,
    first: bool,
    source_root: Option<&Path>,
    frozen: Option<&str>,
) -> String {
    let mut p = question_context(card, links, first, source_root, frozen);
    p.push_str("\nThe user's question: ");
    p.push_str(question);
    p
}

/// The stateless equivalent of Claude's `--resume` follow-up: for backends
/// without a session ([`Backend::supports_session`](crate::backend::Backend::supports_session)
/// is false), each tutor turn is a fresh process, so cross-turn memory is
/// restored by re-inlining the whole conversation. This builds the same
/// first-turn card context as [`question_prompt`] (instructions, links, source
/// grounding), then replays each `prior` `(question, answer)` exchange in order,
/// and finally poses the new `question`. An empty `prior` reduces to exactly the
/// first-turn [`question_prompt`], so a first turn is identical on every backend.
pub fn question_prompt_with_history(
    card: &Card,
    links: &[String],
    prior: &[Exchange],
    question: &str,
    source_root: Option<&Path>,
    frozen: Option<&str>,
) -> String {
    // Every re-inlined turn is a "first" message: the process has no memory, so
    // it needs the full instructions, links, and card context each time.
    let mut p = question_context(card, links, true, source_root, frozen);
    for (q, a) in prior {
        p.push_str("\nThe user's question: ");
        p.push_str(q);
        p.push_str("\nYour answer: ");
        p.push_str(a);
        p.push('\n');
    }
    p.push_str("\nThe user's question: ");
    p.push_str(question);
    p
}

/// The shared card context up to (but not including) the trailing question: the
/// first-turn instructions and deck links (when `first`), the card itself, and
/// any `source_root`/`frozen` grounding. Both [`question_prompt`] and
/// [`question_prompt_with_history`] append their question(s) after this.
fn question_context(
    card: &Card,
    links: &[String],
    first: bool,
    source_root: Option<&Path>,
    frozen: Option<&str>,
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
    match (frozen, source_root) {
        // A frozen card: the snapshot excerpt is the ground truth (it's what the
        // learner sees); the live crate is read only for surrounding context.
        (Some(excerpt), root) => {
            p.push_str(
                "\nThe exact code this card is about, frozen when the card was made \
                 — treat it as the GROUND TRUTH, since it's what the learner sees:\n\n",
            );
            p.push_str(excerpt);
            if let Some(root) = root {
                p.push_str(&format!(
                    "\nYour working directory is the live source at {}. The snippet \
                     above may have moved or changed since; READ the surrounding \
                     source there (Read, Glob, Grep) to explain how this excerpt \
                     fits the rest of the code — but ground your answer in the \
                     snippet above, not a drifted copy. If you cannot find this code \
                     anywhere in the source, reply exactly: \"{SOURCE_NOT_FOUND}\"\n",
                    root.display()
                ));
            } else {
                p.push_str(&format!(
                    "\nThe live source this came from is unavailable, so reply \
                     exactly: \"{SOURCE_NOT_FOUND}\"\n"
                ));
            }
        }
        // A live (non-frozen) source: read it directly and verify.
        (None, Some(root)) => {
            p.push_str(&format!(
                "\nThis card was generated from the source code at {} — your working \
                 directory. Before stating anything specific about the code, READ the \
                 actual files there (Read, Glob, Grep) and verify against them; do not \
                 answer from memory. If the source contradicts the card, say so.\n",
                root.display()
            ));
        }
        (None, None) => {}
    }
    p
}

/// A copy of `cfg` that lets the tutor read the source at `root`: the working
/// directory points there, and the read-only `Read`/`Glob`/`Grep` tools are
/// added to the allowlist. Used when `[ask] source_access` is on, so the web
/// tutor can verify against the real source.
pub fn with_source_root(cfg: &AskConfig, root: &Path) -> AskConfig {
    let mut grounded = cfg.clone();
    grounded.cwd = Some(root.to_path_buf());
    for tool in ["Read", "Glob", "Grep"] {
        if !grounded.allowed_tools.iter().any(|t| t == tool) {
            grounded.allowed_tools.push(tool.to_string());
        }
    }
    grounded
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
        .map(|l| {
            l.trim()
                .trim_start_matches(['!', '-', '*', '•'])
                .trim()
                .to_string()
        })
        .filter(|l| !l.is_empty())
        .take(3)
        .collect()
}

/// Runs the CLI in a background thread; the reply arrives on the returned
/// channel. The caller polls it with `try_recv`. `extra_args` carries the
/// session arguments (`--session-id`/`--resume`).
pub fn spawn(config: AskConfig, prompt: String, extra_args: Vec<String>) -> Receiver<Reply> {
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

/// Runs the assistant CLI with a timeout, delegating the CLI-specific argv,
/// prompt delivery, and answer extraction to the [`Backend`] for `config`;
/// the spawn/drain/timeout plumbing lives here. The tool allowlist becomes an
/// abstract [`Access`] grant the backend renders back into its flags — for
/// Claude, the default `[WebFetch, WebSearch]` under `dontAsk` lets it consult
/// deck links without ever blocking on an (unanswerable) permission prompt,
/// while denying every other tool.
pub(crate) fn run(config: &AskConfig, prompt: &str, extra_args: &[String]) -> Result<String> {
    let backend = backend_for(config)?;
    // Session flags (Claude's `--session-id`/`--resume`) are Claude-specific;
    // forwarding them to a backend without a session mechanism would error on an
    // unknown flag. Drop them there so the tutor runs statelessly. Prior-turn
    // memory is restored by the frontends re-inlining the transcript into the
    // prompt (`question_prompt_with_history`) for these backends.
    let session_args: &[String] = if backend.supports_session() {
        extra_args
    } else {
        &[]
    };
    let opts = RunOpts {
        model: config.model.as_deref(),
        effort: config.effort.as_deref(),
        permission_mode: if config.permission_mode.is_empty() {
            None
        } else {
            Some(config.permission_mode.as_str())
        },
        access: Access::from_allowed_tools(&config.allowed_tools),
        session_args,
    };
    let mut argv = backend.build_argv(&opts);
    // Arg-delivery backends (Codex `exec`) take the prompt as the final
    // positional argument rather than on stdin; append it here so the backend's
    // `build_argv` stays prompt-free.
    if matches!(
        backend.prompt_delivery(),
        PromptDelivery::Arg | PromptDelivery::ExecArg
    ) {
        argv.push(prompt.to_string());
    }

    let mut cmd = Command::new(&config.command);
    cmd.args(&argv);
    // Trace building runs in the `% source:` root so Claude explores it with
    // relative paths; other callers inherit this process's directory.
    if let Some(dir) = &config.cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run '{}' — is it installed?", config.command))?;

    // Feed the prompt and close stdin so the CLI starts processing.
    let stdin = child.stdin.take().expect("stdin was piped");
    match backend.prompt_delivery() {
        PromptDelivery::Stdin => {
            let mut stdin = stdin;
            stdin
                .write_all(prompt.as_bytes())
                .context("cannot write the prompt")?;
        }
        // Backends that take the prompt as an argument carry it in `build_argv`;
        // stdin is closed immediately so the CLI stops waiting on it.
        PromptDelivery::Arg | PromptDelivery::ExecArg => drop(stdin),
    }

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
            bail!(
                "'{}' timed out after {}s",
                config.command,
                config.timeout_secs
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if !status.success() {
        let detail = stderr.trim();
        let detail = if detail.is_empty() {
            stdout.trim()
        } else {
            detail
        };
        bail!("{}", map_run_failure(&config.command, detail));
    }
    let answer = backend.extract(&stdout)?;
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

/// Turns a failed CLI's stderr/stdout `detail` into a clearer message, leading
/// with actionable guidance for the two failures a raw dump hides worst —
/// exhausted quota and a signed-out CLI — while keeping the raw detail appended.
/// Any other failure passes through as `'<command>' failed: <detail>`. The
/// detail is matched case-insensitively; the mapping never turns an error into a
/// success — it only reframes it.
fn map_run_failure(command: &str, detail: &str) -> String {
    let detail = truncate(detail, 300);
    let lower = detail.to_ascii_lowercase();
    let hit = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    if hit(&[
        "rate limit",
        "rate-limit",
        "quota",
        "429",
        "usage limit",
        "too many requests",
    ]) {
        format!(
            "'{command}' hit its usage limit — wait and retry, or switch [ask] backend: {detail}"
        )
    } else if hit(&[
        "not logged in",
        "not signed in",
        "unauthenticated",
        "unauthorized",
        "authentication",
        "401",
        "log in",
        "login",
    ]) {
        format!("'{command}' isn't signed in — run its login once: {detail}")
    } else {
        format!("'{command}' failed: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

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
        let p = question_prompt(&card(), &links, "and why that?", true, None, None);
        assert!(p.contains("concise tutor"));
        assert!(!p.contains("working directory")); // no source access by default
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
        let p = question_prompt(&card(), &links, "next q", false, None, None);
        // The session already knows the instructions and the links.
        assert!(!p.contains("concise tutor"));
        assert!(!p.contains("docs.rs"));
        // But the card may have changed, so it is always included.
        assert!(p.contains("Front: Why?"));
        assert!(p.ends_with("The user's question: next q"));
    }

    #[test]
    fn question_prompt_with_history_includes_prior_exchanges() {
        let prior = vec![
            (
                "what is ownership?".to_string(),
                "who frees the value".to_string(),
            ),
            ("and borrowing?".to_string(), "temporary access".to_string()),
        ];
        let p = question_prompt_with_history(&card(), &[], &prior, "and lifetimes?", None, None);
        // The card context is present, exactly as the first-turn prompt.
        assert!(p.contains("concise tutor"), "{p}");
        assert!(p.contains("Front: Why?"), "{p}");
        // Each prior question AND its answer appear, in order, before the new one.
        let q1 = p.find("what is ownership?").expect("first question");
        let a1 = p.find("who frees the value").expect("first answer");
        let q2 = p.find("and borrowing?").expect("second question");
        let a2 = p.find("temporary access").expect("second answer");
        let new_q = p.find("and lifetimes?").expect("new question");
        assert!(
            q1 < a1 && a1 < q2 && q2 < a2 && a2 < new_q,
            "out of order: {p}"
        );
        // The new question is the last thing the model reads.
        assert!(p.ends_with("The user's question: and lifetimes?"), "{p}");

        // An empty history reduces to the same content as the first-turn prompt.
        let links = vec!["https://docs.rs/tokio".to_string()];
        let first = question_prompt(&card(), &links, "and lifetimes?", true, None, None);
        let empty =
            question_prompt_with_history(&card(), &links, &[], "and lifetimes?", None, None);
        assert_eq!(
            first, empty,
            "empty history must match the first-turn prompt"
        );
    }

    #[test]
    fn source_access_grounds_every_prompt_in_the_crate_root() {
        // Even a follow-up reminds Claude it can read the source and must verify.
        let p = question_prompt(
            &card(),
            &[],
            "is that right?",
            false,
            Some(Path::new("/repo/x")),
            None,
        );
        assert!(p.contains("/repo/x"));
        assert!(p.contains("READ the actual files"));
        assert!(p.ends_with("The user's question: is that right?"));
    }

    #[test]
    fn frozen_prompt_inlines_the_excerpt_and_grounds_for_context() {
        // A frozen card: the snapshot excerpt is the anchor, the live crate is
        // read only for context, and a missing source yields the canned reply.
        let block = "src/caching.rs:46-66\n46\tfn get_object() {}\n";
        let p = question_prompt(
            &card(),
            &[],
            "explain",
            true,
            Some(Path::new("/crate")),
            Some(block),
        );
        assert!(p.contains("GROUND TRUTH"), "{p}");
        assert!(p.contains("src/caching.rs:46-66"), "{p}");
        assert!(p.contains("/crate"), "{p}");
        assert!(p.contains("surrounding source"), "{p}");
        // No live source → the canned "couldn't find" instruction instead.
        let gone = question_prompt(&card(), &[], "explain", true, None, Some(block));
        assert!(gone.contains(SOURCE_NOT_FOUND), "{gone}");
    }

    #[test]
    fn first_prompt_without_links_offers_none() {
        let p = question_prompt(&card(), &[], "q", true, None, None);
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
    fn args_in_resumes_in_the_same_cwd_but_resets_on_a_change() {
        let a = Path::new("/crate/a");
        let b = Path::new("/crate/b");
        let mut session = CliSession::new();
        // First call in cwd a: creates the session there.
        let create = session.args_in(Some(a));
        assert_eq!("--session-id", create[0]);
        let id = create[1].clone();
        session.started = true; // a successful reply marks it started

        // Same cwd: resumes the same conversation.
        let resume = session.args_in(Some(a));
        assert_eq!(["--resume", &id], resume.as_slice());

        // A different cwd can't resume that conversation -> fresh session, and
        // `started` is cleared so the next prompt is a first message.
        let switched = session.args_in(Some(b));
        assert_eq!("--session-id", switched[0]);
        assert_ne!(id, switched[1]);
        assert!(!session.started);
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
        assert!(matches!(
            parts[3].chars().next(),
            Some('8' | '9' | 'a' | 'b')
        ));
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

    use crate::testutil::{exec_lock, fake_arg_reply, fake_cli};

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
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // Echo the prompt back (reads stdin like the real CLI).
        let cli = fake_cli(dir.path(), "cat");
        let answer = run(&config(&cli, 10), "hello there", &[]).unwrap();
        assert_eq!("hello there", answer);
    }

    #[test]
    fn run_passes_session_args_to_the_cli() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // Echo the received arguments instead of the prompt.
        let cli = fake_cli(dir.path(), "echo \"$@\"; cat > /dev/null");
        let extra = vec!["--resume".to_string(), "abc".to_string()];
        let answer = run(&config(&cli, 10), "x", &extra).unwrap();
        assert!(answer.contains("--resume abc"), "args were: {answer}");
        assert!(answer.contains("--allowedTools WebFetch WebSearch"));
        // The permission mode must be passed, or the real CLI hangs in -p
        // mode waiting for an approval it cannot receive.
        assert!(
            answer.contains("--permission-mode dontAsk"),
            "args were: {answer}"
        );
    }

    #[test]
    fn run_passes_effort_when_set() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "echo \"$@\"; cat > /dev/null");
        let config = AskConfig {
            command: cli.to_str().unwrap().to_string(),
            effort: Some("high".to_string()),
            timeout_secs: 10,
            ..AskConfig::default()
        };
        let answer = run(&config, "x", &[]).unwrap();
        assert!(answer.contains("--effort high"), "args were: {answer}");
    }

    #[test]
    fn run_omits_effort_when_unset() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "echo \"$@\"; cat > /dev/null");
        let answer = run(&config(&cli, 10), "x", &[]).unwrap();
        assert!(!answer.contains("--effort"), "args were: {answer}");
    }

    #[test]
    fn run_reports_failures_with_stderr() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(
            dir.path(),
            "cat >/dev/null; echo 'not logged in' >&2; exit 1",
        );
        let err = run(&config(&cli, 10), "x", &[]).unwrap_err();
        assert!(format!("{err:#}").contains("not logged in"));
    }

    #[test]
    fn run_times_out() {
        let _lock = exec_lock();
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
    fn arg_delivery_appends_the_prompt_and_reads_the_reply() {
        use crate::config::BackendKind;
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // An arg-delivery fake that ignores stdin and replies with fixed text.
        let cli = fake_arg_reply(dir.path(), "the codex answer");
        let config = AskConfig {
            backend: BackendKind::Codex,
            command: cli.to_str().unwrap().to_string(),
            timeout_secs: 10,
            ..AskConfig::default()
        };
        let answer = run(&config, "explain this card", &[]).unwrap();
        assert_eq!("the codex answer", answer);
    }

    #[test]
    fn arg_delivery_passes_the_prompt_as_the_final_argument() {
        use crate::config::BackendKind;
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // Echo the received arguments; arg delivery must append the prompt last.
        let cli = fake_cli(dir.path(), "echo \"$@\"");
        let config = AskConfig {
            backend: BackendKind::Codex,
            command: cli.to_str().unwrap().to_string(),
            timeout_secs: 10,
            ..AskConfig::default()
        };
        let answer = run(&config, "the-prompt-text", &[]).unwrap();
        // The Codex invocation, with the prompt as the final positional arg.
        assert!(answer.contains("exec"), "args were: {answer}");
        assert!(
            answer.contains("--sandbox read-only"),
            "args were: {answer}"
        );
        assert!(
            answer.trim().ends_with("the-prompt-text"),
            "args were: {answer}"
        );
    }

    #[test]
    fn rate_limit_stderr_maps_to_the_usage_limit_message() {
        let msg = map_run_failure("claude", "Error: 429 rate limit exceeded, retry later");
        assert!(msg.contains("hit its usage limit"), "{msg}");
        assert!(msg.contains("switch [ask] backend"), "{msg}");
        // The raw detail is kept for the user.
        assert!(msg.contains("429"), "{msg}");
    }

    #[test]
    fn quota_stderr_also_maps_to_the_usage_limit_message() {
        let msg = map_run_failure("gemini", "you have exceeded your quota for this model");
        assert!(msg.contains("hit its usage limit"), "{msg}");
    }

    #[test]
    fn not_signed_in_stderr_maps_to_the_login_message() {
        let msg = map_run_failure("codex", "error: 401 Unauthorized — you are not logged in");
        assert!(msg.contains("isn't signed in"), "{msg}");
        assert!(msg.contains("run its login once"), "{msg}");
        assert!(msg.contains("401"), "{msg}");
    }

    #[test]
    fn other_failures_pass_through_with_the_command() {
        let msg = map_run_failure("claude", "segmentation fault");
        assert!(msg.contains("'claude' failed"), "{msg}");
        assert!(msg.contains("segmentation fault"), "{msg}");
        // Not misclassified as quota or auth.
        assert!(!msg.contains("usage limit"), "{msg}");
        assert!(!msg.contains("signed in"), "{msg}");
    }

    #[test]
    fn session_args_are_dropped_for_a_non_session_backend() {
        use crate::config::BackendKind;
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // Echo the received arguments; the prompt is an arg for Codex, so the
        // session flags — if forwarded — would appear before it.
        let cli = fake_cli(dir.path(), "echo \"$@\"");
        let config = AskConfig {
            backend: BackendKind::Codex,
            command: cli.to_str().unwrap().to_string(),
            timeout_secs: 10,
            ..AskConfig::default()
        };
        let extra = vec!["--resume".to_string(), "sess-123".to_string()];
        let answer = run(&config, "x", &extra).unwrap();
        // Codex has no session mechanism, so the flags are dropped, not passed on.
        assert!(
            !answer.contains("--resume") && !answer.contains("sess-123"),
            "session args must be dropped for codex: {answer}"
        );
    }

    #[test]
    fn session_args_are_forwarded_for_claude() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "echo \"$@\"; cat > /dev/null");
        let extra = vec!["--resume".to_string(), "sess-123".to_string()];
        let answer = run(&config(&cli, 10), "x", &extra).unwrap();
        // Claude supports sessions, so the flags are forwarded.
        assert!(
            answer.contains("--resume sess-123"),
            "session args must reach claude: {answer}"
        );
    }

    #[test]
    fn spawn_delivers_on_the_channel() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "cat");
        let rx = spawn(config(&cli, 10), "ping".to_string(), Vec::new());
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Reply::Answer(a) => assert_eq!("ping", a),
            Reply::Error(e) => panic!("unexpected error: {e}"),
        }
    }
}
