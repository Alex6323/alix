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
    config::{AskConfig, Audience},
};

pub type Exchange = (String, String);

pub enum Reply {
    Answer(String),
    Error(String),
}

#[derive(Clone)]
pub struct CliSession {
    id: String,
    pub started: bool,
    cwd: Option<PathBuf>,
}

impl CliSession {
    pub fn new() -> Self {
        Self {
            id: random_uuid(),
            started: false,
            cwd: None,
        }
    }

    pub fn args(&self) -> Vec<String> {
        if self.started {
            vec!["--resume".to_string(), self.id.clone()]
        } else {
            vec!["--session-id".to_string(), self.id.clone()]
        }
    }

    // A cwd change resets the session: Claude can't --resume a conversation from a different
    // working directory.
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

pub const SOURCE_NOT_FOUND: &str =
    "I couldn't find the source material of this card to provide a grounded answer.";

pub fn question_prompt(
    card: &Card,
    audience: Audience,
    links: &[String],
    question: &str,
    first: bool,
    source_root: Option<&Path>,
    frozen: Option<&str>,
) -> String {
    let mut p = question_context(card, audience, links, first, source_root, frozen);
    p.push_str("\nThe user's question: ");
    p.push_str(question);
    p
}

// Backends without a session replay the whole history each turn: there's no server-side memory to
// resume.
pub fn question_prompt_with_history(
    card: &Card,
    audience: Audience,
    links: &[String],
    prior: &[Exchange],
    question: &str,
    source_root: Option<&Path>,
    frozen: Option<&str>,
) -> String {
    let mut p = question_context(card, audience, links, true, source_root, frozen);
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

// Kept byte-for-byte: tests and callers depend on this exact wording.
const ADULT_PREAMBLE: &str = "You are a concise tutor inside a terminal flashcard application. \
     The user reviews flashcards and asks you questions about them; \
     this conversation continues across several cards. Always answer \
     in plain text without any markdown formatting, in at most six \
     short sentences, specific to the card at hand.\n";

const KIDS_PREAMBLE: &str = "You are a kind helper for a kid around 10 years old who is using a flashcard \
     app to learn. Use simple words and short sentences, and sound warm and \
     encouraging. Only talk about the flashcard they're looking at right now — \
     help them understand this one card, and don't wander into other topics. \
     Answer in plain text without any markdown formatting, in at most four short \
     sentences. If they ask something that isn't about the card, gently steer \
     them back to it. If they ask about anything grown-up, unsafe, or otherwise \
     inappropriate, kindly say you can't help with that and bring them back to \
     the flashcard, without lecturing or going into detail about why.\n";

fn preamble(audience: Audience) -> &'static str {
    match audience {
        Audience::Adult => ADULT_PREAMBLE,
        Audience::Kids => KIDS_PREAMBLE,
    }
}

fn question_context(
    card: &Card,
    audience: Audience,
    links: &[String],
    first: bool,
    source_root: Option<&Path>,
    frozen: Option<&str>,
) -> String {
    let mut p = String::new();
    if first {
        p.push_str(preamble(audience));
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
                // serve.rs's start_ask already short-circuits this case; this arm is the lib-level
                // fallback for other callers (e.g. the trace-walk tutor).
                p.push_str(&format!(
                    "\nThe live source this came from is unavailable, so reply \
                     exactly: \"{SOURCE_NOT_FOUND}\"\n"
                ));
            }
        }
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

#[derive(Clone, Debug, PartialEq)]
pub struct DraftCard {
    pub front: String,
    pub back: Vec<String>,
}

pub fn draft_card_prompt(card: &Card, transcript: &[Exchange]) -> String {
    let mut p = String::new();
    p.push_str(
        "From the conversation below, write ONE focused flashcard that captures the \
         single most useful thing to remember. Output ONLY a card in this exact format, \
         with no fences, preamble, or commentary:\n\n\
         ## <the question>\n<the answer>\n\n\
         The `## ` front is at column 0; the answer is the plain (unindented) line(s) \
         below it. Keep the question short and the answer to one or a few lines. Base \
         it strictly on the conversation; do not invent facts.\n\n",
    );
    p.push_str(&format!("The card under review:\n## {}\n", card.front));
    for b in &card.back {
        p.push_str(&format!("{b}\n"));
    }
    p.push_str("\nThe conversation:\n");
    for (q, a) in transcript {
        p.push_str(&format!("Q: {q}\nA: {a}\n"));
    }
    p
}

pub fn parse_drafted_card(reply: &str) -> Result<DraftCard> {
    let body = reply
        .trim()
        .trim_start_matches("```markdown")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let cards = crate::parser::parse_str("draft", body)
        .map_err(|e| anyhow::anyhow!("the tutor's reply was not a valid card: {e}"))?;
    let [card] = cards.as_slice() else {
        bail!("the tutor did not return exactly one card");
    };
    // Defense-in-depth check; parse_str already rejects empty fronts and frontless blocks.
    if card.front.trim().is_empty() || card.back.iter().all(|l| l.trim().is_empty()) {
        bail!("the drafted card has an empty side");
    }
    Ok(DraftCard {
        front: card.front.trim().to_string(),
        back: card
            .back
            .iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    })
}

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

// The default WebFetch/WebSearch allowlist under dontAsk lets Claude consult deck links without
// blocking on an unanswerable permission prompt.
pub(crate) fn run(config: &AskConfig, prompt: &str, extra_args: &[String]) -> Result<String> {
    let backend = backend_for(config)?;
    // Session flags are Claude-specific; forwarding them to a backend without a session mechanism
    // would error on an unknown flag.
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
    // Arg-delivery backends take the prompt as a positional arg, not stdin, so it's appended here
    // instead of in build_argv.
    if matches!(
        backend.prompt_delivery(),
        PromptDelivery::Arg | PromptDelivery::ExecArg
    ) {
        argv.push(prompt.to_string());
    }

    let mut cmd = Command::new(&config.command);
    cmd.args(&argv);
    // Trace building runs in the source root so Claude explores it with relative paths; other
    // callers inherit this process's directory.
    if let Some(dir) = &config.cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run '{}' — is it installed?", config.command))?;

    let stdin = child.stdin.take().expect("stdin was piped");
    match backend.prompt_delivery() {
        PromptDelivery::Stdin => {
            let mut stdin = stdin;
            stdin
                .write_all(prompt.as_bytes())
                .context("cannot write the prompt")?;
        }
        // stdin is closed immediately here so the CLI (which takes the prompt as an arg) doesn't
        // hang waiting on it.
        PromptDelivery::Arg | PromptDelivery::ExecArg => drop(stdin),
    }

    // Reader threads drain output so the child never deadlocks on a full pipe while this thread
    // watches the deadline.
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
        let p = question_prompt(
            &card(),
            Audience::Adult,
            &links,
            "and why that?",
            true,
            None,
            None,
        );
        assert!(p.contains("concise tutor"));
        assert!(!p.contains("working directory"));
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
        let p = question_prompt(
            &card(),
            Audience::Adult,
            &links,
            "next q",
            false,
            None,
            None,
        );
        assert!(!p.contains("concise tutor"));
        assert!(!p.contains("docs.rs"));
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
        let p = question_prompt_with_history(
            &card(),
            Audience::Adult,
            &[],
            &prior,
            "and lifetimes?",
            None,
            None,
        );
        assert!(p.contains("concise tutor"), "{p}");
        assert!(p.contains("Front: Why?"), "{p}");
        let q1 = p.find("what is ownership?").expect("first question");
        let a1 = p.find("who frees the value").expect("first answer");
        let q2 = p.find("and borrowing?").expect("second question");
        let a2 = p.find("temporary access").expect("second answer");
        let new_q = p.find("and lifetimes?").expect("new question");
        assert!(
            q1 < a1 && a1 < q2 && q2 < a2 && a2 < new_q,
            "out of order: {p}"
        );
        assert!(p.ends_with("The user's question: and lifetimes?"), "{p}");

        let links = vec!["https://docs.rs/tokio".to_string()];
        let first = question_prompt(
            &card(),
            Audience::Adult,
            &links,
            "and lifetimes?",
            true,
            None,
            None,
        );
        let empty = question_prompt_with_history(
            &card(),
            Audience::Adult,
            &links,
            &[],
            "and lifetimes?",
            None,
            None,
        );
        assert_eq!(
            first, empty,
            "empty history must match the first-turn prompt"
        );
    }

    #[test]
    fn an_empty_history_prompt_is_exactly_the_first_turn_prompt() {
        let links = vec!["https://docs.rs/tokio".to_string()];
        let first = question_prompt(&card(), Audience::Adult, &links, "why?", true, None, None);
        let empty =
            question_prompt_with_history(&card(), Audience::Adult, &links, &[], "why?", None, None);
        assert_eq!(first, empty, "empty history must equal the first turn");

        let root = Some(Path::new("/repo/x"));
        let frozen = Some("src/caching.rs:46-66\n46\tfn get_object() {}\n");
        let first = question_prompt(&card(), Audience::Adult, &[], "why?", true, root, frozen);
        let empty =
            question_prompt_with_history(&card(), Audience::Adult, &[], &[], "why?", root, frozen);
        assert_eq!(
            first, empty,
            "grounded empty history must equal the first turn"
        );
    }

    #[test]
    fn source_access_grounds_every_prompt_in_the_crate_root() {
        let p = question_prompt(
            &card(),
            Audience::Adult,
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
        let block = "src/caching.rs:46-66\n46\tfn get_object() {}\n";
        let p = question_prompt(
            &card(),
            Audience::Adult,
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
        let gone = question_prompt(
            &card(),
            Audience::Adult,
            &[],
            "explain",
            true,
            None,
            Some(block),
        );
        assert!(gone.contains(SOURCE_NOT_FOUND), "{gone}");
    }

    #[test]
    fn first_prompt_without_links_offers_none() {
        let p = question_prompt(&card(), Audience::Adult, &[], "q", true, None, None);
        assert!(!p.contains("Reference links"));
    }

    #[test]
    fn kids_audience_uses_the_kid_safe_preamble() {
        let adult = question_prompt(&card(), Audience::Adult, &[], "why?", true, None, None);
        let kids = question_prompt(&card(), Audience::Kids, &[], "why?", true, None, None);
        assert!(adult.contains("concise tutor"), "{adult}");
        assert!(!kids.contains("concise tutor"), "{kids}");
        assert!(kids.to_lowercase().contains("kid"), "{kids}");
        assert!(kids.contains("Front: Why?"), "{kids}");
        assert!(kids.ends_with("The user's question: why?"), "{kids}");
    }

    #[test]
    fn session_args_create_then_resume() {
        let mut session = CliSession::new();
        let create = session.args();
        assert_eq!("--session-id", create[0]);
        session.started = true;
        let resume = session.args();
        assert_eq!("--resume", resume[0]);
        assert_eq!(create[1], resume[1]);
    }

    #[test]
    fn args_in_resumes_in_the_same_cwd_but_resets_on_a_change() {
        let a = Path::new("/crate/a");
        let b = Path::new("/crate/b");
        let mut session = CliSession::new();
        let create = session.args_in(Some(a));
        assert_eq!("--session-id", create[0]);
        let id = create[1].clone();
        session.started = true;

        let resume = session.args_in(Some(a));
        assert_eq!(["--resume", &id], resume.as_slice());

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
    fn draft_card_prompt_asks_for_l1_shape_and_no_old_syntax() {
        let transcript = vec![("q".to_string(), "a".to_string())];
        let p = draft_card_prompt(&card(), &transcript);
        assert!(p.contains("## <the question>"));
        assert!(p.contains("column 0"));
        assert!(p.contains("## Why?"));
        assert!(p.contains("Q: q\nA: a"));
        assert!(!p.contains("tab-indent"));
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

    use crate::testutil::{exec_lock, fake_arg_reply, fake_cli, fake_reply};

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
        let cli = fake_cli(dir.path(), "cat");
        let answer = run(&config(&cli, 10), "hello there", &[]).unwrap();
        assert_eq!("hello there", answer);
    }

    #[test]
    fn run_passes_session_args_to_the_cli() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "echo \"$@\"; cat > /dev/null");
        let extra = vec!["--resume".to_string(), "abc".to_string()];
        let answer = run(&config(&cli, 10), "x", &extra).unwrap();
        assert!(answer.contains("--resume abc"), "args were: {answer}");
        assert!(answer.contains("--allowedTools WebFetch WebSearch"));
        // Missing --permission-mode would hang the real CLI waiting for an approval it can't
        // receive.
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
        let cli = fake_cli(dir.path(), "echo \"$@\"");
        let config = AskConfig {
            backend: BackendKind::Codex,
            command: cli.to_str().unwrap().to_string(),
            timeout_secs: 10,
            ..AskConfig::default()
        };
        let answer = run(&config, "the-prompt-text", &[]).unwrap();
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
        assert!(!msg.contains("usage limit"), "{msg}");
        assert!(!msg.contains("signed in"), "{msg}");
    }

    #[test]
    fn session_args_are_dropped_for_a_non_session_backend() {
        use crate::config::BackendKind;
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "echo \"$@\"");
        let config = AskConfig {
            backend: BackendKind::Codex,
            command: cli.to_str().unwrap().to_string(),
            timeout_secs: 10,
            ..AskConfig::default()
        };
        let extra = vec!["--resume".to_string(), "sess-123".to_string()];
        let answer = run(&config, "x", &extra).unwrap();
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

    #[test]
    fn kids_audience_ask_runs_through_spawn_and_returns_the_reply() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "sure, let's look at this card together!");
        let prompt = question_prompt(
            &card(),
            Audience::Kids,
            &[],
            "why is it Because?",
            true,
            None,
            None,
        );
        let rx = spawn(config(&cli, 10), prompt, Vec::new());
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Reply::Answer(a) => assert_eq!("sure, let's look at this card together!", a),
            Reply::Error(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn parse_drafted_card_reads_a_deck_format_block() {
        let reply = "## what frees Dart memory?\nA generational garbage collector.\n";
        let card = parse_drafted_card(reply).unwrap();
        assert_eq!(card.front, "what frees Dart memory?");
        assert_eq!(
            card.back,
            vec!["A generational garbage collector.".to_string()]
        );
    }

    #[test]
    fn parse_drafted_card_strips_markdown_fences() {
        let reply = "```\n## term?\ndefinition\n```";
        let card = parse_drafted_card(reply).unwrap();
        assert_eq!(card.front, "term?");
    }

    #[test]
    fn parse_drafted_card_errors_on_junk() {
        assert!(parse_drafted_card("I could not think of a good card, sorry!").is_err());
    }

    #[test]
    fn parse_drafted_card_errors_on_a_frontless_block() {
        assert!(parse_drafted_card("\tjust an answer, no question\n").is_err());
    }

    #[test]
    fn parse_drafted_card_errors_on_an_empty_reply() {
        let reply = "```\n```";
        assert!(parse_drafted_card(reply).is_err());
    }

    #[test]
    fn parse_drafted_card_errors_on_two_cards() {
        let reply = "## q1?\na1\n## q2?\na2\n";
        assert!(parse_drafted_card(reply).is_err());
    }
}
