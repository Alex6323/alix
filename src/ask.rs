use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

use crate::{
    backend::{Access, Backend, PromptDelivery, RunOpts, backend_for},
    card::Card,
    config::{AskConfig, Audience},
};

pub type Exchange = (String, String);

pub const FROZEN_ONLY_WARNING: &str =
    "The tutor has the frozen excerpts, but not the full original source context.";

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

/// Everything the tutor is grounded on, layered per ADR 0026: the deck's own
/// sources are the primary grounding, the workspace source supporting context.
#[derive(Clone, Copy, Debug)]
pub struct TutorContext<'a> {
    pub links: &'a [String],
    pub sources: &'a crate::deck::SourceLayers,
    pub root: Option<&'a Path>,
    pub frozen: Option<&'a str>,
}

pub fn question_prompt(
    card: &Card,
    audience: Audience,
    context: &TutorContext,
    question: &str,
    first: bool,
) -> String {
    let mut p = question_context(card, audience, context, first);
    p.push_str("\nThe user's question: ");
    p.push_str(question);
    p
}

// Backends without a session replay the whole history each turn: there's no server-side memory to
// resume.
pub fn question_prompt_with_history(
    card: &Card,
    audience: Audience,
    context: &TutorContext,
    prior: &[Exchange],
    question: &str,
) -> String {
    let mut p = question_context(card, audience, context, true);
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

fn push_link_group(p: &mut String, heading: &str, links: &[&String]) {
    if links.is_empty() {
        return;
    }
    p.push_str(heading);
    p.push('\n');
    for link in links {
        p.push_str(link);
        p.push('\n');
    }
}

fn question_context(
    card: &Card,
    audience: Audience,
    context: &TutorContext,
    first: bool,
) -> String {
    let TutorContext {
        links,
        sources,
        root,
        frozen,
    } = *context;
    let mut p = String::new();
    if first {
        p.push_str(preamble(audience));
        // Both layers, told apart like the exam's source section (ADR 0026),
        // local paths and URLs alike.
        let own: Vec<&String> = sources.own.iter().collect();
        let workspace: Vec<&String> = sources.workspace.iter().collect();
        let other: Vec<&String> = links
            .iter()
            .filter(|link| !own.contains(link) && !workspace.contains(link))
            .collect();
        if !own.is_empty() || !workspace.is_empty() || !other.is_empty() {
            p.push_str(
                "\nReference material for this deck; fetch links with WebFetch \
                 and read local paths with Read/Glob/Grep when they can improve \
                 an answer; you only need to read each once:\n",
            );
            push_link_group(&mut p, "Deck sources (the primary grounding):", &own);
            push_link_group(&mut p, "Workspace source (supporting context):", &workspace);
            push_link_group(&mut p, "Related links:", &other);
        }
        p.push('\n');
    }
    p.push_str("The card being reviewed:\n\n");
    push_card(&mut p, card);
    match (frozen, root) {
        (Some(excerpt), Some(root)) => {
            p.push_str(
                "\nThe exact code this card is about, frozen when the card was made \
                 is the evidence the learner sees. Treat it as the GROUND TRUTH for \
                 what the card teaches:\n\n",
            );
            p.push_str(excerpt);
            p.push_str(&format!(
                "\nThe broader original source is available at {}, your working \
                 directory. READ it for surrounding context and to check whether the \
                 frozen evidence or card has become outdated. Do not silently replace \
                 the frozen evidence. If the current source contradicts it, clearly \
                 tell the learner that the card may be outdated.\n",
                root.display()
            ));
        }
        (Some(excerpt), None) => {
            p.push_str(
                "\nThe exact code this card is about, frozen when the card was made \
                 is the evidence the learner sees. Treat it as the GROUND TRUTH for \
                 what the card teaches:\n\n",
            );
            p.push_str(excerpt);
            p.push_str(
                "\nUse that frozen evidence and any source link listed above to explain \
                 the card. If a live source contradicts the frozen evidence, clearly \
                 tell the learner that the card may be outdated.\n",
            );
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

/// A handle onto a spawned ask's child process: cancel kills and reaps it
/// synchronously, so no AI subprocess outlives its owner (a dropped pending
/// exchange or the application shutdown).
#[derive(Clone, Default)]
pub struct AskJob {
    child: std::sync::Arc<std::sync::Mutex<ChildSlot>>,
}

#[derive(Default)]
enum ChildSlot {
    #[default]
    NotStarted,
    Running(std::process::Child),
    Finished,
    Cancelled,
}

impl AskJob {
    pub fn cancel(&self) {
        let mut slot = self.child.lock().unwrap_or_else(|p| p.into_inner());
        if let ChildSlot::Running(child) = &mut *slot {
            kill_tree(child);
        }
        *slot = ChildSlot::Cancelled;
    }
}

// The child is spawned as its own process-group leader, so descendants the
// backend CLI spawns (node, browsers, git) die with it; a direct kill would
// orphan them with quota and source access. The child is unreaped when this
// runs, so its pid cannot have been recycled. kill(1) is POSIX; if spawning
// it fails, the direct kill below still reaps the child itself.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-9", "--", &format!("-{}", child.id())])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn spawn(
    config: AskConfig,
    prompt: String,
    extra_args: Vec<String>,
) -> (Receiver<Reply>, AskJob) {
    let (tx, rx) = channel();
    let job = AskJob::default();
    let slot = std::sync::Arc::clone(&job.child);
    std::thread::spawn(move || {
        let reply = match run_supervised(&config, &prompt, &extra_args, &slot) {
            Ok(answer) => Reply::Answer(answer),
            Err(e) => Reply::Error(format!("{e:#}")),
        };
        // The receiver may be gone if the user left the ask view.
        let _ = tx.send(reply);
    });
    (rx, job)
}

// The default WebFetch/WebSearch allowlist under dontAsk lets Claude consult deck links without
// blocking on an unanswerable permission prompt.
pub(crate) fn run(config: &AskConfig, prompt: &str, extra_args: &[String]) -> Result<String> {
    run_supervised(config, prompt, extra_args, &Default::default())
}

fn run_supervised(
    config: &AskConfig,
    prompt: &str,
    extra_args: &[String],
    slot: &std::sync::Mutex<ChildSlot>,
) -> Result<String> {
    let backend = backend_for(config)?;
    // Session flags are Claude-specific; forwarding them to a backend without a session mechanism
    // would error on an unknown flag.
    let session_args: &[String] = if backend.supports_session() {
        extra_args
    } else {
        &[]
    };
    let model = crate::backend::resolved_ask_model(config);
    let effort = crate::backend::resolved_ask_effort(config);
    let opts = RunOpts {
        model: model.as_deref(),
        effort: effort.as_deref(),
        permission_mode: if config.permission_mode.is_empty() {
            None
        } else {
            Some(config.permission_mode.as_str())
        },
        access: Access::from_allowed_tools(&config.allowed_tools),
        session_args,
        progress: config.progress,
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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run '{}' — is it installed?", config.command))?;

    let stdin = child.stdin.take().expect("stdin was piped");
    let stdout_pipe = child.stdout.take().expect("stdout was piped");
    let stderr_pipe = child.stderr.take().expect("stderr was piped");
    {
        let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
        if matches!(*guard, ChildSlot::Cancelled) {
            kill_tree(&mut child);
            bail!("the ask was cancelled");
        }
        *guard = ChildSlot::Running(child);
    }
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

    // Reader threads drain output so the child never deadlocks on a full pipe.
    let (pipe_tx, pipe_rx) = channel();
    let out = drain_pipe(stdout_pipe, Pipe::Stdout, pipe_tx.clone());
    let err = drain_pipe(stderr_pipe, Pipe::Stderr, pipe_tx);

    let started = Instant::now();
    let deadline = started + Duration::from_secs(config.timeout_secs);
    let idle_timeout =
        effective_idle_timeout(backend.as_ref(), config.progress, config.idle_timeout_secs);
    let mut progress = ProgressState::new(started);
    let status = loop {
        let now = Instant::now();
        progress.receive(&pipe_rx, backend.as_ref(), config.progress, now);

        {
            let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
            match &mut *guard {
                ChildSlot::Running(child) => {
                    if let Some(status) = child.try_wait().context("cannot wait for the CLI")? {
                        *guard = ChildSlot::Finished;
                        break status;
                    }
                    match timeout_kind(now, deadline, progress.last_activity, idle_timeout) {
                        Some(TimeoutKind::Absolute) => {
                            kill_tree(child);
                            *guard = ChildSlot::Finished;
                            drop(guard);
                            bail!(
                                "'{}' timed out after {}s",
                                config.command,
                                config.timeout_secs
                            );
                        }
                        Some(TimeoutKind::Idle) => {
                            kill_tree(child);
                            *guard = ChildSlot::Finished;
                            drop(guard);
                            bail!(
                                "'{}' made no progress for {}s ({}s elapsed)",
                                config.command,
                                idle_timeout.map_or(0, |timeout| timeout.as_secs()),
                                now.duration_since(started).as_secs()
                            );
                        }
                        None => {}
                    }
                }
                // cancel() killed and reaped it synchronously.
                _ => bail!("the ask was cancelled"),
            }
        }
        if heartbeat_due(config.progress, now.duration_since(progress.last_report)) {
            eprintln!(
                "{}: still working ({}s elapsed, {}s since last activity).",
                backend_label(backend.name()),
                now.duration_since(started).as_secs(),
                now.duration_since(progress.last_activity).as_secs()
            );
            progress.last_report = now;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    progress.receive(&pipe_rx, backend.as_ref(), config.progress, Instant::now());
    if !status.success() {
        let progress_detail = config
            .progress
            .then(|| backend.extract_progress(&stdout).err())
            .flatten()
            .map(|error| format!("{error:#}"));
        let detail = [
            Some(stderr.trim()),
            progress_detail.as_deref(),
            Some(stdout.trim()),
        ]
        .into_iter()
        .flatten()
        .find(|detail| !detail.is_empty())
        .unwrap_or_default();
        bail!("{}", map_run_failure(&config.command, detail));
    }
    let answer = if config.progress {
        backend.extract_progress(&stdout)?
    } else {
        backend.extract(&stdout)?
    };
    if answer.is_empty() {
        bail!("'{}' returned an empty answer", config.command);
    }
    Ok(answer)
}

#[derive(Clone, Copy)]
enum Pipe {
    Stdout,
    Stderr,
}

struct PipeEvent {
    pipe: Pipe,
    line: String,
}

/// What each backend last reported it was actually running, learned from its
/// own stream. The tutor panel names this instead of "default"; empty until a
/// first streamed answer, because a backend that was never asked has not said.
static OBSERVED_MODELS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>,
> = std::sync::OnceLock::new();

fn note_observed_model(backend: &str, model: &str) {
    if let Ok(mut seen) = OBSERVED_MODELS.get_or_init(Default::default).lock() {
        seen.insert(backend.to_string(), model.to_string());
    }
}

pub fn observed_model(backend: &str) -> Option<String> {
    OBSERVED_MODELS.get()?.lock().ok()?.get(backend).cloned()
}

struct ProgressState {
    last_activity: Instant,
    last_report: Instant,
    last_message: Option<String>,
}

impl ProgressState {
    fn new(started: Instant) -> Self {
        Self {
            last_activity: started,
            last_report: started,
            last_message: None,
        }
    }

    fn receive(
        &mut self,
        events: &Receiver<PipeEvent>,
        backend: &dyn Backend,
        show: bool,
        now: Instant,
    ) {
        while let Ok(event) = events.try_recv() {
            if event.line.trim().is_empty() {
                continue;
            }
            match event.pipe {
                Pipe::Stdout => {
                    let update = backend.progress_update(&event.line);
                    if update.activity {
                        self.last_activity = now;
                    }
                    if let Some(model) = &update.model {
                        note_observed_model(backend.name(), model);
                    }
                    if !show {
                        continue;
                    }
                    if let Some(message) = update.message {
                        self.report(message, now);
                    } else if !backend.structured_progress() {
                        self.report(
                            format!("{}: producing a response...", backend_label(backend.name())),
                            now,
                        );
                    }
                }
                Pipe::Stderr => {
                    self.last_activity = now;
                    if show {
                        let detail = event.line.trim().replace(['\r', '\n'], " ");
                        let detail = truncate(&detail, 180);
                        self.report(format!("{}: {detail}", backend_label(backend.name())), now);
                    }
                }
            }
        }
    }

    fn report(&mut self, message: String, now: Instant) {
        if self.last_message.as_deref() == Some(&message) {
            return;
        }
        eprintln!("{message}");
        self.last_message = Some(message);
        self.last_report = now;
    }
}

fn drain_pipe<R: Read + Send + 'static>(
    pipe: R,
    kind: Pipe,
    events: Sender<PipeEvent>,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut output = String::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    output.push_str(&line);
                    let _ = events.send(PipeEvent { pipe: kind, line });
                }
            }
        }
        output
    })
}

pub fn backend_label(name: &str) -> &str {
    match name {
        "claude" => "Claude",
        "codex" => "Codex",
        "gemini" => "Gemini",
        "copilot" => "Copilot",
        other => other,
    }
}

fn heartbeat_due(progress: bool, since_last: Duration) -> bool {
    progress && since_last >= Duration::from_secs(15)
}

fn effective_idle_timeout(
    backend: &dyn Backend,
    progress: bool,
    seconds: Option<u64>,
) -> Option<Duration> {
    if progress && backend.structured_progress() {
        seconds.map(Duration::from_secs)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimeoutKind {
    Absolute,
    Idle,
}

fn timeout_kind(
    now: Instant,
    deadline: Instant,
    last_activity: Instant,
    idle_timeout: Option<Duration>,
) -> Option<TimeoutKind> {
    if now >= deadline {
        Some(TimeoutKind::Absolute)
    } else if idle_timeout.is_some_and(|timeout| now.duration_since(last_activity) >= timeout) {
        Some(TimeoutKind::Idle)
    } else {
        None
    }
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

#[cfg(all(test, unix))]
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

    static NO_SOURCES: crate::deck::SourceLayers = crate::deck::SourceLayers {
        own: Vec::new(),
        workspace: Vec::new(),
    };

    fn ctx<'a>(
        links: &'a [String],
        root: Option<&'a Path>,
        frozen: Option<&'a str>,
    ) -> TutorContext<'a> {
        TutorContext {
            links,
            sources: &NO_SOURCES,
            root,
            frozen,
        }
    }

    #[test]
    fn first_question_prompt_has_instructions_and_links() {
        let links = vec!["https://docs.rs/tokio".to_string()];
        let p = question_prompt(
            &card(),
            Audience::Adult,
            &ctx(&links, None, None),
            "and why that?",
            true,
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
    fn first_prompt_labels_deck_and_workspace_sources_apart_from_links() {
        let links = vec![
            "https://own.example".to_string(),
            "https://context.example".to_string(),
            "https://docs.rs/tokio".to_string(),
        ];
        let sources = crate::deck::SourceLayers {
            own: vec!["https://own.example".to_string()],
            workspace: vec!["https://context.example".to_string()],
        };
        let context = TutorContext {
            links: &links,
            sources: &sources,
            root: None,
            frozen: None,
        };
        let p = question_prompt(&card(), Audience::Adult, &context, "why?", true);
        let own = p.find("Deck sources (the primary grounding):").unwrap();
        let workspace = p.find("Workspace source (supporting context):").unwrap();
        let other = p.find("Related links:").unwrap();
        assert!(own < workspace && workspace < other, "{p}");
        assert!(p.find("https://own.example").unwrap() < workspace, "{p}");
        assert!(p.find("https://docs.rs/tokio").unwrap() > other, "{p}");
        assert_eq!(1, p.matches("https://own.example").count(), "{p}");
    }

    #[test]
    fn first_prompt_labels_both_layers_for_local_sources_too() {
        let sources = crate::deck::SourceLayers {
            own: vec!["notes/own.md".to_string()],
            workspace: vec!["ws-material".to_string()],
        };
        let context = TutorContext {
            links: &[],
            sources: &sources,
            root: None,
            frozen: None,
        };
        let p = question_prompt(&card(), Audience::Adult, &context, "why?", true);
        let own = p.find("Deck sources (the primary grounding):").unwrap();
        let workspace = p.find("Workspace source (supporting context):").unwrap();
        assert!(own < workspace, "{p}");
        assert!(p.find("notes/own.md").unwrap() < workspace, "{p}");
        assert!(p.find("ws-material").unwrap() > workspace, "{p}");
        assert!(p.contains("read local paths with Read/Glob/Grep"), "{p}");
    }

    #[test]
    fn first_prompt_offers_reference_material_when_any_one_layer_is_present() {
        for (own, workspace, links, expected_heading) in [
            (
                vec!["own.md".to_string()],
                Vec::new(),
                Vec::new(),
                "Deck sources (the primary grounding):",
            ),
            (
                Vec::new(),
                vec!["workspace.md".to_string()],
                Vec::new(),
                "Workspace source (supporting context):",
            ),
            (
                Vec::new(),
                Vec::new(),
                vec!["https://example.test/related".to_string()],
                "Related links:",
            ),
        ] {
            let sources = crate::deck::SourceLayers { own, workspace };
            let context = TutorContext {
                links: &links,
                sources: &sources,
                root: None,
                frozen: None,
            };

            let prompt = question_prompt(&card(), Audience::Adult, &context, "why?", true);

            assert!(prompt.contains("Reference material"), "{prompt}");
            assert!(prompt.contains(expected_heading), "{prompt}");
        }
    }

    #[test]
    fn followup_prompt_is_short_but_carries_the_card() {
        let links = vec!["https://docs.rs/tokio".to_string()];
        let p = question_prompt(
            &card(),
            Audience::Adult,
            &ctx(&links, None, None),
            "next q",
            false,
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
            &ctx(&[], None, None),
            &prior,
            "and lifetimes?",
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
            &ctx(&links, None, None),
            "and lifetimes?",
            true,
        );
        let empty = question_prompt_with_history(
            &card(),
            Audience::Adult,
            &ctx(&links, None, None),
            &[],
            "and lifetimes?",
        );
        assert_eq!(
            first, empty,
            "empty history must match the first-turn prompt"
        );
    }

    #[test]
    fn an_empty_history_prompt_is_exactly_the_first_turn_prompt() {
        let links = vec!["https://docs.rs/tokio".to_string()];
        let first = question_prompt(
            &card(),
            Audience::Adult,
            &ctx(&links, None, None),
            "why?",
            true,
        );
        let empty = question_prompt_with_history(
            &card(),
            Audience::Adult,
            &ctx(&links, None, None),
            &[],
            "why?",
        );
        assert_eq!(first, empty, "empty history must equal the first turn");

        let root = Some(Path::new("/repo/x"));
        let frozen = Some("src/caching.rs:46-66\n46\tfn get_object() {}\n");
        let first = question_prompt(
            &card(),
            Audience::Adult,
            &ctx(&[], root, frozen),
            "why?",
            true,
        );
        let empty = question_prompt_with_history(
            &card(),
            Audience::Adult,
            &ctx(&[], root, frozen),
            &[],
            "why?",
        );
        assert_eq!(
            first, empty,
            "grounded empty history must equal the first turn"
        );
    }

    #[test]
    fn source_access_grounds_every_prompt_in_the_declared_root() {
        let p = question_prompt(
            &card(),
            Audience::Adult,
            &ctx(&[], Some(Path::new("/repo/x")), None),
            "is that right?",
            false,
        );
        assert!(p.contains("/repo/x"));
        assert!(p.contains("READ the actual files"));
        assert!(p.ends_with("The user's question: is that right?"));
    }

    #[test]
    fn frozen_prompt_uses_the_asset_and_the_available_source_context() {
        let block = "src/caching.rs:46-66\n46\tfn get_object() {}\n";
        let p = question_prompt(
            &card(),
            Audience::Adult,
            &ctx(&[], Some(Path::new("/crate")), Some(block)),
            "explain",
            true,
        );
        assert!(p.contains("GROUND TRUTH"), "{p}");
        assert!(p.contains("src/caching.rs:46-66"), "{p}");
        assert!(p.contains("/crate"), "{p}");
        assert!(p.contains("READ it for surrounding context"), "{p}");
        assert!(p.contains("card may be outdated"), "{p}");
        let portable = question_prompt(
            &card(),
            Audience::Adult,
            &ctx(&[], None, Some(block)),
            "explain",
            true,
        );
        assert!(portable.contains("GROUND TRUTH"), "{portable}");
        assert!(portable.contains("any source link"), "{portable}");
        assert!(!portable.contains("/crate"), "{portable}");
    }

    #[test]
    fn first_prompt_without_links_offers_none() {
        let p = question_prompt(&card(), Audience::Adult, &ctx(&[], None, None), "q", true);
        assert!(!p.contains("Reference material"));
    }

    #[test]
    fn kids_audience_uses_the_kid_safe_preamble() {
        let adult = question_prompt(
            &card(),
            Audience::Adult,
            &ctx(&[], None, None),
            "why?",
            true,
        );
        let kids = question_prompt(&card(), Audience::Kids, &ctx(&[], None, None), "why?", true);
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
    fn session_id_payload_bits_are_not_constant() {
        let ids: Vec<Vec<u8>> = (0..512)
            .map(|_| {
                let compact = CliSession::new().id.replace('-', "");
                (0..16)
                    .map(|index| {
                        u8::from_str_radix(&compact[index * 2..index * 2 + 2], 16).unwrap()
                    })
                    .collect()
            })
            .collect();

        for bit in 0..128 {
            // UUID version and variant bits are fixed by the format.
            if matches!(bit, 48..=51 | 64..=65) {
                continue;
            }
            let mask = 1 << (7 - bit % 8);
            let byte = bit / 8;
            assert!(
                ids.iter().any(|id| id[byte] & mask == 0),
                "bit {bit} is always one"
            );
            assert!(
                ids.iter().any(|id| id[byte] & mask != 0),
                "bit {bit} is always zero"
            );
        }
    }

    #[test]
    fn source_root_grounding_preserves_config_and_adds_each_read_tool_once() {
        let config = AskConfig {
            backend: crate::config::BackendKind::Codex,
            command: "custom-agent".to_string(),
            model: Some("model-x".to_string()),
            effort: Some("high".to_string()),
            timeout_secs: 37,
            progress: true,
            idle_timeout_secs: Some(11),
            permission_mode: "custom-mode".to_string(),
            allowed_tools: vec!["Read".to_string(), "WebFetch".to_string()],
            cwd: Some(PathBuf::from("/old")),
            source_access: true,
            preflight_threshold: 123,
        };

        let grounded = with_source_root(&config, Path::new("/source"));

        assert_eq!(Some(PathBuf::from("/source")), grounded.cwd);
        assert_eq!(
            ["Read", "WebFetch", "Glob", "Grep"],
            grounded
                .allowed_tools
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
        );
        assert_eq!(config.backend, grounded.backend);
        assert_eq!(config.command, grounded.command);
        assert_eq!(config.model, grounded.model);
        assert_eq!(config.effort, grounded.effort);
        assert_eq!(config.timeout_secs, grounded.timeout_secs);
        assert_eq!(config.progress, grounded.progress);
        assert_eq!(config.idle_timeout_secs, grounded.idle_timeout_secs);
        assert_eq!(config.permission_mode, grounded.permission_mode);
        assert_eq!(config.source_access, grounded.source_access);
        assert_eq!(config.preflight_threshold, grounded.preflight_threshold);
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
    fn draft_card_prompt_teaches_only_the_current_deck_shape() {
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
    fn run_stops_when_a_progress_stream_goes_idle() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("blocked");
        assert!(
            Command::new("mkfifo")
                .arg(&blocked)
                .status()
                .unwrap()
                .success(),
            "creating the blocked-process fixture"
        );
        let cli = fake_cli(
            dir.path(),
            &format!("cat >/dev/null; cat {}", blocked.display()),
        );
        let config = AskConfig {
            progress: true,
            idle_timeout_secs: Some(0),
            ..config(&cli, 10)
        };
        // The child blocks on a fifo nothing writes to, so the idle timeout is
        // the only thing that ends this run. Bound the wait: without it, a
        // defect in the timeout logic hangs the suite instead of failing it.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run(&config, "x", &[]).map_err(|err| format!("{err:#}")));
        });
        let outcome = rx.recv_timeout(Duration::from_secs(20));
        if outcome.is_err() {
            // Unblock the child so it cannot outlive the test.
            let _ = std::fs::write(&blocked, "x");
        }
        let err = match outcome {
            Ok(Err(err)) => err,
            Ok(Ok(answer)) => panic!("expected an idle timeout, got {answer:?}"),
            Err(_) => panic!("run did not return within 20s: the idle timeout never fired"),
        };
        assert!(err.contains("made no progress for 0s"), "{err}");
    }

    #[test]
    fn progress_timeout_policy_resets_on_activity_and_prefers_the_absolute_limit() {
        let started = Instant::now();
        let deadline = started + Duration::from_secs(10);
        for (now, last_activity, idle, expected) in [
            (9, 0, None, None),
            (10, 9, Some(2), Some(TimeoutKind::Absolute)),
            (9, 7, Some(2), Some(TimeoutKind::Idle)),
            (9, 8, Some(2), None),
            (0, 0, Some(0), Some(TimeoutKind::Idle)),
        ] {
            assert_eq!(
                expected,
                timeout_kind(
                    started + Duration::from_secs(now),
                    deadline,
                    started + Duration::from_secs(last_activity),
                    idle.map(Duration::from_secs),
                ),
                "now={now}, last_activity={last_activity}, idle={idle:?}"
            );
        }
    }

    #[test]
    fn inactivity_requires_a_structured_progress_stream() {
        assert_eq!(
            Some(Duration::from_secs(5)),
            effective_idle_timeout(&crate::backend::ClaudeBackend, true, Some(5))
        );
        assert_eq!(
            None,
            effective_idle_timeout(&crate::backend::ClaudeBackend, false, Some(5))
        );
        assert_eq!(
            None,
            effective_idle_timeout(&crate::backend::GeminiBackend, true, Some(5))
        );
        assert_eq!(
            None,
            effective_idle_timeout(&crate::backend::ClaudeBackend, true, None)
        );
    }

    #[test]
    fn every_nonempty_pipe_event_resets_the_idle_clock() {
        let started = Instant::now();
        let mut progress = ProgressState::new(started);
        let (tx, rx) = channel();
        for (index, pipe, line, resets) in [
            (1, Pipe::Stdout, "provider output", true),
            (2, Pipe::Stdout, " \n", false),
            (3, Pipe::Stderr, "provider diagnostic", true),
        ] {
            tx.send(PipeEvent {
                pipe,
                line: line.to_string(),
            })
            .unwrap();
            let now = started + Duration::from_secs(index);
            let before = progress.last_activity;
            progress.receive(&rx, &crate::backend::ClaudeBackend, false, now);
            assert_eq!(
                if resets { now } else { before },
                progress.last_activity,
                "pipe event {index}: {line:?}"
            );
        }
    }

    #[test]
    fn the_observed_model_is_recorded_from_the_stream_and_read_back() {
        let started = Instant::now();
        let mut progress = ProgressState::new(started);
        let (tx, rx) = channel();
        tx.send(PipeEvent {
            pipe: Pipe::Stdout,
            line: r#"{"type":"system","subtype":"init","model":"claude-probe-model"}"#.to_string(),
        })
        .unwrap();
        progress.receive(
            &rx,
            &crate::backend::ClaudeBackend,
            true,
            started + Duration::from_secs(1),
        );
        assert_eq!(
            Some("claude-probe-model".to_string()),
            observed_model("claude"),
            "the readout must be able to name what the backend said it loaded"
        );
    }

    #[test]
    fn a_pipe_event_is_reported_only_when_progress_is_shown() {
        let started = Instant::now();
        for (pipe, show, reported) in [
            (Pipe::Stdout, true, true),
            (Pipe::Stdout, false, false),
            (Pipe::Stderr, true, true),
            (Pipe::Stderr, false, false),
        ] {
            let mut progress = ProgressState::new(started);
            let (tx, rx) = channel();
            tx.send(PipeEvent {
                pipe,
                line: "provider output".to_string(),
            })
            .unwrap();
            progress.receive(
                &rx,
                &crate::backend::GeminiBackend,
                show,
                started + Duration::from_secs(1),
            );
            assert_eq!(
                reported,
                progress.last_message.is_some(),
                "show={show}: whether a pipe event is reported must follow the show flag"
            );
        }
    }

    #[test]
    fn progress_labels_and_heartbeat_policy_cover_every_backend() {
        for (progress, elapsed_secs, expected) in [
            (false, 16, false),
            (true, 14, false),
            (true, 15, true),
            (true, 16, true),
        ] {
            assert_eq!(
                expected,
                heartbeat_due(progress, Duration::from_secs(elapsed_secs)),
                "progress={progress}, elapsed_secs={elapsed_secs}"
            );
        }

        for (name, expected) in [
            ("claude", "Claude"),
            ("codex", "Codex"),
            ("gemini", "Gemini"),
            ("copilot", "Copilot"),
            ("custom", "custom"),
        ] {
            assert_eq!(expected, backend_label(name), "backend={name}");
        }
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
        let (rx, _job) = spawn(config(&cli, 10), "ping".to_string(), Vec::new());
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
            &ctx(&[], None, None),
            "why is it Because?",
            true,
        );
        let (rx, _job) = spawn(config(&cli, 10), prompt, Vec::new());
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
    fn parse_drafted_card_errors_on_an_empty_back() {
        assert!(parse_drafted_card("## question with no answer?\n").is_err());
    }

    #[test]
    fn parse_drafted_card_errors_on_two_cards() {
        let reply = "## q1?\na1\n## q2?\na2\n";
        assert!(parse_drafted_card(reply).is_err());
    }
}
