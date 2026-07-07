//! User configuration, loaded from a TOML file
//! (`~/.config/alix/config.toml` on Linux).
//!
//! Currently this configures key bindings. Every action takes a list of
//! keys; the first one is shown in the footer of the TUI. A key is written
//! as a single character (`"j"`), a special key name (`"space"`, `"enter"`,
//! `"tab"`, `"esc"`, `"backspace"`), or either with a `ctrl-` prefix
//! (`"ctrl-s"`).
//!
//! Plain-character bindings are ignored while you are typing an answer
//! (typing and fuzzy mode), so they cannot shadow text input; use `ctrl-` or
//! special keys for actions that must be reachable there (hint, skip, quit).

use std::{
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::Deserialize;

use crate::level::Level;

/// A key without modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Esc,
    Backspace,
}

/// A key plus the Ctrl modifier flag; what a binding matches against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPattern {
    pub key: Key,
    pub ctrl: bool,
}

impl KeyPattern {
    /// `true` if this pattern would swallow plain text input.
    pub fn is_plain_char(&self) -> bool {
        matches!(self.key, Key::Char(_)) && !self.ctrl
    }
}

impl fmt::Display for KeyPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl-")?;
        }
        match self.key {
            Key::Char(' ') => write!(f, "SPACE"),
            // After "Ctrl-" the letter reads better uppercased (Ctrl-S).
            Key::Char(c) if self.ctrl => write!(f, "{}", c.to_uppercase()),
            Key::Char(c) => write!(f, "{c}"),
            Key::Enter => write!(f, "ENTER"),
            Key::Tab => write!(f, "TAB"),
            Key::Esc => write!(f, "ESC"),
            Key::Backspace => write!(f, "BACKSPACE"),
        }
    }
}

/// Parses a key description like `"j"`, `"space"` or `"ctrl-s"`.
pub fn parse_key(s: &str) -> Result<KeyPattern> {
    let lower = s.trim().to_lowercase();
    let (ctrl, name) = match lower.strip_prefix("ctrl-") {
        Some(rest) => (true, rest),
        None => (false, lower.as_str()),
    };
    let key = match name {
        "space" => Key::Char(' '),
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Esc,
        "backspace" => Key::Backspace,
        _ => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c),
                _ => bail!(
                    "invalid key {s:?}: expected a single character, \
                     space/enter/tab/esc/backspace, or a ctrl- prefix"
                ),
            }
        }
    };
    Ok(KeyPattern { key, ctrl })
}

/// All rebindable actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    /// Self-graded modes: grade as failed (FSRS `Again` — resets learning progress).
    pub failed: Vec<KeyPattern>,
    /// Self-graded modes: grade as partly (FSRS `Hard` — a weak pass, still drilling).
    pub partly: Vec<KeyPattern>,
    /// Self-graded modes: grade as passed (FSRS `Good` — advances toward graduation).
    pub passed: Vec<KeyPattern>,
    /// Flip mode: reveal the answer.
    pub reveal: Vec<KeyPattern>,
    /// Typing mode: reveal a hint (fails the card).
    pub hint: Vec<KeyPattern>,
    /// Fuzzy mode: submit the current line.
    pub submit: Vec<KeyPattern>,
    /// Put the current card at the end of the queue without grading.
    pub skip: Vec<KeyPattern>,
    /// Mark the current card for removal from its deck file (applied when the
    /// session ends).
    pub remove: Vec<KeyPattern>,
    /// Promote the current virtual (remediation) card into its deck file,
    /// dropping the virtual copy — the new deck card carries over its review
    /// schedule rather than starting fresh. Offered only while reviewing a
    /// virtual card.
    pub promote: Vec<KeyPattern>,
    /// Leave the feedback screen.
    pub cont: Vec<KeyPattern>,
    /// Start a new session from the summary screen.
    pub restart: Vec<KeyPattern>,
    /// Open the ask-Claude view on an answered card.
    pub ask: Vec<KeyPattern>,
    /// Ask view: condense the conversation and save it as a card note.
    pub save_note: Vec<KeyPattern>,
    /// Quit the session.
    pub quit: Vec<KeyPattern>,
}

impl Default for Bindings {
    fn default() -> Self {
        let keys = |list: &[&str]| list.iter().map(|s| parse_key(s).unwrap()).collect();
        Self {
            failed: keys(&["1", "f"]),
            partly: keys(&["2", "p"]),
            passed: keys(&["3", "n"]),
            reveal: keys(&["space", "enter"]),
            hint: keys(&["tab", "ctrl-h", "ctrl-backspace"]),
            submit: keys(&["enter"]),
            skip: keys(&["ctrl-s"]),
            remove: keys(&["ctrl-x"]),
            promote: keys(&["ctrl-p"]),
            cont: keys(&["enter", "space"]),
            restart: keys(&["r"]),
            ask: keys(&["?"]),
            save_note: keys(&["ctrl-n"]),
            quit: keys(&["esc", "ctrl-c"]),
        }
    }
}

impl Bindings {
    /// The key shown in the footer for an action (its first binding).
    pub fn label(list: &[KeyPattern]) -> String {
        list.first()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "?".to_string())
    }
}

/// Rebindable navigation keys for the deck picker, configured in the
/// `[keys.picker]` section. Vim-style by default. The arrow keys, `Enter` (open)
/// and `Esc` (back) always work regardless of these; jumping to the first/last
/// row stays fixed at `g`/`G`/Home/End (letter bindings are case-insensitive, so
/// `g` and `G` can't be told apart — same as the `[keys.browse]` pager).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerKeys {
    /// Move up / down the list.
    pub up: Vec<KeyPattern>,
    pub down: Vec<KeyPattern>,
    /// Open the focused row (a deck reviews, a workspace drills in).
    pub open: Vec<KeyPattern>,
    /// Step back / cancel.
    pub back: Vec<KeyPattern>,
    /// Enter filter mode.
    pub filter: Vec<KeyPattern>,
    /// Open the Mastered window.
    pub mastered: Vec<KeyPattern>,
}

impl Default for PickerKeys {
    fn default() -> Self {
        let keys = |list: &[&str]| list.iter().map(|s| parse_key(s).unwrap()).collect();
        Self {
            up: keys(&["k"]),
            down: keys(&["j"]),
            open: keys(&["l"]),
            back: keys(&["h"]),
            filter: keys(&["/", "ctrl-f"]),
            mastered: keys(&["m"]),
        }
    }
}

/// Key bindings for the read-only browser (`alix browse`), configured in the
/// `[keys.browse]` section. Jumping to the first/last card stays fixed at
/// `g`/`G`/Home/End — letter bindings are case-insensitive, so `g` and `G`
/// cannot be told apart — and the arrow keys always work for next/previous.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowseBindings {
    /// Move to the next card.
    pub next: Vec<KeyPattern>,
    /// Move to the previous card.
    pub prev: Vec<KeyPattern>,
    /// Mark the current card for removal from its deck file (applied on quit).
    pub remove: Vec<KeyPattern>,
    /// Leave the browser.
    pub quit: Vec<KeyPattern>,
}

impl Default for BrowseBindings {
    fn default() -> Self {
        let keys = |list: &[&str]| list.iter().map(|s| parse_key(s).unwrap()).collect();
        Self {
            next: keys(&["l", "n", "space"]),
            prev: keys(&["h", "p"]),
            remove: keys(&["x"]),
            quit: keys(&["q", "esc", "ctrl-c"]),
        }
    }
}

/// Which AI CLI backend to use for assistant calls.
///
/// All four variants are wired; `backend_for` returns `Ok` for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackendKind {
    /// The Claude Code CLI (`claude -p`). The default.
    #[default]
    Claude,
    /// Google Gemini CLI (`gemini -p`, headless).
    Gemini,
    /// OpenAI Codex CLI (`codex exec`, headless).
    Codex,
    /// GitHub Copilot CLI (`copilot -p`, headless).
    Copilot,
}

/// Settings for the ask-Claude integration (`[ask]` in the config file).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskConfig {
    /// Which AI CLI backend to use.
    pub backend: BackendKind,
    /// The CLI executable to run.
    pub command: String,
    /// Model passed as `--model`; `None` uses the CLI's own default.
    pub model: Option<String>,
    /// `--effort` level (`low`/`medium`/`high`/`xhigh`/`max`); `None` omits the
    /// flag and uses the CLI's default. Trace operations default this to
    /// `high`.
    pub effort: Option<String>,
    /// How long to wait for an answer before giving up.
    pub timeout_secs: u64,
    /// `--permission-mode` for the headless CLI. The default `"dontAsk"`
    /// silently denies any tool not in `allowed_tools` instead of waiting
    /// for an interactive approval that `-p` mode cannot provide (which
    /// would hang). An empty string omits the flag.
    pub permission_mode: String,
    /// Tools the assistant may use (`--allowedTools`). Combined with
    /// `dontAsk`, this is an exclusive allowlist: everything else is denied,
    /// so a malicious deck-link page cannot make the tutor run commands.
    pub allowed_tools: Vec<String>,
    /// Working directory for the CLI process (`current_dir`). `None` inherits
    /// the caller's. Not a user setting: trace building sets it to the
    /// `% source:` root so Claude explores the source with relative paths.
    pub cwd: Option<PathBuf>,
    /// Opt-in: let the ask-Claude tutor **read the card's source** to verify its
    /// answer (Read/Glob/Grep, working directory at the deck's `% source:`
    /// project root) instead of answering from memory. Off by default because it
    /// grants the served tutor file-read access — only enable it on a machine and
    /// network you trust (especially with `alix serve --lan`).
    pub source_access: bool,
    /// Source-tree byte threshold for the pre-flight size guard. When a local
    /// source tree (for `deck generate`, `trace --build`/`--suggest`, `explore`)
    /// exceeds this many bytes, `alix` warns and asks for confirmation before
    /// spending a potentially large model call. Default is 5 MB (5_000_000 bytes).
    /// Set to 0 to disable the guard (always proceed without confirming).
    pub preflight_threshold: u64,
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            backend: BackendKind::Claude,
            command: "claude".to_string(),
            model: None,
            effort: None,
            timeout_secs: 120,
            permission_mode: "dontAsk".to_string(),
            allowed_tools: vec!["WebFetch".to_string(), "WebSearch".to_string()],
            cwd: None,
            source_access: false,
            preflight_threshold: 5_000_000,
        }
    }
}

/// Settings for AI deck generation (`alix deck`, the `[generate]` section).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerateDeckConfig {
    /// Model passed as `--model`; `None` falls back to the `[ask]` model, then
    /// the CLI's own default.
    pub model: Option<String>,
    /// How long to wait for the deck before giving up (generation is a bigger
    /// call than a single question, so this is larger than the ask timeout).
    pub timeout_secs: u64,
    /// Upper bound on the number of cards to generate.
    pub max_cards: usize,
    /// Extra guidance appended to the instruction prompt (the common way to
    /// tweak generation, e.g. "focus on the public API").
    pub extra: Option<String>,
    /// A full replacement for the built-in instruction prompt. May use the
    /// `{url}` and `{max_cards}` placeholders.
    pub prompt: Option<String>,
    /// Run a second Claude pass that reviews the draft and removes redundant
    /// cards. `--review` forces it on for a single run.
    pub review: bool,
}

impl Default for GenerateDeckConfig {
    fn default() -> Self {
        Self {
            model: None,
            timeout_secs: 300,
            max_cards: 30,
            extra: None,
            prompt: None,
            review: false,
        }
    }
}

/// How strictly the AI exam grades a typed answer against a question's rubric
/// points. This is a per-deck choice (set with `% strictness:`, the `[exam]`
/// default, or `alix exam --strictness`) because some material demands
/// recalling everything while other material is about grasping the idea.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, ValueEnum)]
pub enum Strictness {
    /// Completeness required: every rubric point must be present, so omitting
    /// one is a gap. For procedures, exact syntax, security — where knowing
    /// most of it isn't enough.
    Strict,
    /// Judge understanding, not phrasing: a point is covered if the answer
    /// shows the student grasps it (even briefly, in their own words); only
    /// a wrong or genuinely-absent idea is a gap.
    #[default]
    Balanced,
    /// Benefit of the doubt: only clearly wrong or unanswered points are gaps.
    /// For breadth or casual learning.
    Lenient,
}

/// Settings for the AI exam (`alix exam`, the `[exam]` section). Like
/// generate, it reuses the `[ask]` command, permission mode and tool allowlist
/// (WebFetch reads a `% source:` URL). The exam grades open understanding
/// questions generated from the deck's `% source:`, never the cards.
#[derive(Clone, Debug, PartialEq)]
pub struct ExamConfig {
    /// Model passed as `--model`; `None` falls back to the `[ask]` model, then
    /// the CLI's own default.
    pub model: Option<String>,
    /// How long to wait for each exam call (question generation, grading and
    /// remediation are bigger calls than a single question, like generate).
    pub timeout_secs: u64,
    /// How many questions a sitting asks.
    pub num_questions: usize,
    /// Fraction of questions that must be a full Pass for the exam to pass
    /// (1.0 = every question, the v1 default).
    pub pass_threshold: f64,
    /// How strictly each typed answer is graded against the rubric.
    pub strictness: Strictness,
    /// How long (seconds) a *failed* trace exam blocks a re-sit, so the graded
    /// feedback can't be pasted straight back into the one fixed question.
    /// `0` disables the cooldown. Trace exams only (a fact exam regenerates
    /// fresh questions each sitting).
    pub retry_cooldown_secs: u64,
    /// Extra guidance appended to the question-generation prompt (e.g. "focus
    /// on the borrow checker").
    pub extra: Option<String>,
}

impl Default for ExamConfig {
    fn default() -> Self {
        Self {
            model: None,
            timeout_secs: 300,
            num_questions: 5,
            pass_threshold: 1.0,
            strictness: Strictness::default(),
            retry_cooldown_secs: 3600,
            extra: None,
        }
    }
}

/// Settings for trace building (`alix trace --build`, the `[trace]` section).
/// Building explores the deck's `% source:` to discover the path, so — unlike
/// the other AI calls — it runs the CLI with **read-only** file tools (`Read`,
/// `Glob`, `Grep`, plus `WebFetch` for a URL source) and the source root as the
/// working directory. No write or shell tool is ever granted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceConfig {
    /// Model passed as `--model`. Left unset by default so each backend picks
    /// its own strong model for trace ([`Backend::default_trace_model`] — Claude
    /// → `opus`); an explicit value here or in `[ask]` overrides it. Trace
    /// building is agentic, correctness-critical and one-shot, so it wants the
    /// strong model rather than the CLI's cheap default.
    ///
    /// [`Backend::default_trace_model`]: crate::backend::Backend::default_trace_model
    pub model: Option<String>,
    /// `--effort` level; defaults to `"high"` for the same reason. `None` omits
    /// the flag. Shared by `--build`, `--suggest` and `--grade`.
    pub effort: Option<String>,
    /// How long to wait for the build before giving up. Exploring a source and
    /// tracing a path is the biggest call, so this is larger than the others.
    pub timeout_secs: u64,
    /// Extra guidance appended to the build prompt (e.g. "trace the read path,
    /// not the write path").
    pub extra: Option<String>,
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            // Trace building is one-shot, amortized over many reviews, and a weak
            // model fails silently (a parseable but loose chain), so it wants a
            // strong model + high effort. The *model* is left unset here so each
            // backend picks its own strong model (`Backend::default_trace_model`);
            // effort still defaults high across backends.
            model: None,
            effort: Some("high".to_string()),
            timeout_secs: 600,
            extra: None,
        }
    }
}

/// Settings for `alix deck augment` (the `[ai]` section). Augmentation is a
/// deliberate command — it generates distractors (and, later, notes) into the
/// sidecar cache — so there is no on/off switch; these just tune the calls.
/// Generation reuses the `[ask]` command but runs tool-free, so no allowlist
/// applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiConfig {
    /// Model passed as `--model`; `None` falls back to the `[ask]` model, then
    /// the CLI's own default.
    pub model: Option<String>,
    /// How many distractors to request per choice card.
    pub distractor_count: usize,
    /// How many reworded question variants to request per card.
    pub variant_count: usize,
    /// The most key points to request when decomposing a card's answer (the
    /// Explain-mode checklist rubric).
    pub keypoint_count: usize,
    /// How long to wait for a generation call before giving up. A whole-deck
    /// batch is a big call (like `[generate]`/`[exam]`), so this is generous.
    pub timeout_secs: u64,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            model: None,
            distractor_count: 3,
            variant_count: 4,
            keypoint_count: 5,
            timeout_secs: 300,
        }
    }
}

/// Settings for the local web frontend (`alix serve`, the `[serve]` section).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeConfig {
    /// Default port to listen on (overridden by `--port`).
    pub port: u16,
    /// Optional pairing token. When set (or auto-generated for `--lan`), the web
    /// server requires it on `/api/*` — see `alix serve --lan`.
    pub token: Option<String>,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            port: 7777,
            token: None,
        }
    }
}

/// The whole user configuration.
// Not `Eq`: `ExamConfig::pass_threshold` is an `f64`.
/// Personal review pacing (`[review]` in the config): the FSRS retention target, the
/// interval past which a card retires, and the ladder depth target. A workspace can
/// override these in its own `alix.local.toml` (never shared).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReviewConfig {
    /// FSRS target retrievability `r` (0.70–0.99). Higher → shorter intervals.
    pub retention: f64,
    /// A card retires once its scheduled interval reaches this many days; `None`
    /// disables retirement (drill forever).
    pub retire_after_days: Option<u32>,
    /// The learner's depth on the difficulty ladder, resolved from the numeric
    /// `[review] depth` (1 = recall, 2 = reconstruct). Learner-space, not an
    /// authored deck directive; recognition (L0) is never a target.
    pub target: Level,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            retention: 0.9,
            retire_after_days: Some(crate::session::DEFAULT_RETIRE_AFTER_DAYS),
            target: Level::default(),
        }
    }
}

/// A workspace's personal, unshared pacing override (sibling of `alix.toml`).
const LOCAL_MANIFEST: &str = "alix.local.toml";

/// Maps an authored `depth` number to the scheduling rung, clamped to the two
/// schedulable depths: 1 = recall, 2 = reconstruct. Out-of-range coerces to the
/// nearest (0/neg → recall, ≥2 → reconstruct) — recognition (L0) is the
/// unscheduled acquire on-ramp, never a target.
fn depth_to_rung(depth: i64) -> crate::level::Level {
    match depth.clamp(1, 2) {
        1 => crate::level::Level::Recall,
        _ => crate::level::Level::Reconstruct,
    }
}

impl ReviewConfig {
    /// Overlays a workspace's `alix.local.toml` `[review]` overrides onto this
    /// (global) config: only the keys present in the local file win. A missing or
    /// malformed file leaves the config unchanged. This file is personal and is
    /// never shared — deliberately separate from the shared `alix.toml`.
    pub fn for_workspace(self, workspace_dir: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(workspace_dir.join(LOCAL_MANIFEST)) else {
            return self;
        };
        let Ok(raw) = toml::from_str::<RawLocalConfig>(&text) else {
            return self;
        };
        let mut review = self;
        if let Some(retention) = raw.review.retention {
            review.retention = retention.clamp(MIN_RETENTION, MAX_RETENTION);
        }
        if let Some(retire_after) = raw.review.retire_after
            && let Ok(days) = parse_retire_after(&retire_after)
        {
            review.retire_after_days = days;
        }
        if let Some(depth) = raw.review.depth {
            review.target = depth_to_rung(depth);
        }
        review
    }

    /// Resolves the depth for a specific deck: the workspace `[review]` overrides
    /// (via [`for_workspace`](Self::for_workspace)), then the deck's own
    /// `[review.deck."<file name>"]` depth if present. Precedence: per-deck >
    /// workspace > global > default.
    pub fn for_deck(self, deck_path: &Path) -> Self {
        let Some(dir) = deck_path.parent().filter(|d| crate::workspace::is_workspace(d)) else {
            return self;
        };
        let mut review = self.for_workspace(dir);
        let Some(name) = deck_path.file_name().and_then(|n| n.to_str()) else {
            return review;
        };
        if let Ok(text) = std::fs::read_to_string(dir.join(LOCAL_MANIFEST))
            && let Ok(raw) = toml::from_str::<RawLocalConfig>(&text)
            && let Some(depth) = raw.review.deck.get(name).and_then(|d| d.depth)
        {
            review.target = depth_to_rung(depth);
        }
        review
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub keys: Bindings,
    /// Navigation keys for the deck picker.
    pub picker: PickerKeys,
    /// Key bindings for `alix browse`.
    pub browse: BrowseBindings,
    pub ask: AskConfig,
    /// AI deck generation settings.
    pub generate: GenerateDeckConfig,
    /// AI exam settings.
    pub exam: ExamConfig,
    /// Trace building settings.
    pub trace: TraceConfig,
    /// Opt-in AI question-augmentation settings (choice-mode distractors).
    pub ai: AiConfig,
    /// Local web frontend settings.
    pub serve: ServeConfig,
    /// Personal review pacing (FSRS retention + retirement interval).
    pub review: ReviewConfig,
    /// Directory the startup picker lists decks from, and resolves bare deck
    /// names against. `None` uses [`default_decks_dir`].
    pub decks_dir: Option<PathBuf>,
}

impl Config {
    /// The decks directory to use (config value or the default `~/decks`).
    pub fn decks_dir(&self) -> Option<PathBuf> {
        self.decks_dir.clone().or_else(default_decks_dir)
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    keys: RawKeys,
    #[serde(default)]
    ask: RawAsk,
    #[serde(default)]
    generate: RawGenerate,
    #[serde(default)]
    exam: RawExam,
    #[serde(default)]
    trace: RawTrace,
    #[serde(default)]
    ai: RawAi,
    #[serde(default)]
    serve: RawServe,
    #[serde(default)]
    review: RawReviewConfig,
    decks_dir: Option<String>,
}

/// The `[review]` section: personal pacing (FSRS retention, when a card retires, and the ladder depth target).
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawReviewConfig {
    retention: Option<f64>,
    retire_after: Option<String>,
    depth: Option<i64>,
    /// Per-deck depth overrides, keyed by deck file name (`[review.deck."<name>"]`).
    #[serde(default)]
    deck: std::collections::HashMap<String, RawDeckReview>,
}

/// One deck's entry under `[review.deck."<name>"]` — currently just a depth.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawDeckReview {
    depth: Option<i64>,
}

/// The `alix.local.toml` schema: personal per-workspace overrides. Currently just
/// `[review]`; lenient (unknown top-level tables are ignored) so it can grow.
#[derive(Deserialize, Default)]
struct RawLocalConfig {
    #[serde(default)]
    review: RawReviewConfig,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawAi {
    model: Option<String>,
    distractor_count: Option<usize>,
    variant_count: Option<usize>,
    keypoint_count: Option<usize>,
    timeout_secs: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawServe {
    port: Option<u16>,
    token: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawGenerate {
    model: Option<String>,
    timeout_secs: Option<u64>,
    max_cards: Option<usize>,
    extra: Option<String>,
    prompt: Option<String>,
    review: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawExam {
    model: Option<String>,
    timeout_secs: Option<u64>,
    num_questions: Option<usize>,
    pass_threshold: Option<f64>,
    strictness: Option<String>,
    retry_cooldown_secs: Option<u64>,
    extra: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawTrace {
    model: Option<String>,
    effort: Option<String>,
    timeout_secs: Option<u64>,
    extra: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawPicker {
    up: Option<Vec<String>>,
    down: Option<Vec<String>>,
    open: Option<Vec<String>>,
    back: Option<Vec<String>>,
    filter: Option<Vec<String>>,
    mastered: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawBrowse {
    next: Option<Vec<String>>,
    prev: Option<Vec<String>>,
    remove: Option<Vec<String>>,
    quit: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawAsk {
    backend: Option<String>,
    command: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    timeout_secs: Option<u64>,
    permission_mode: Option<String>,
    allowed_tools: Option<Vec<String>>,
    source_access: Option<bool>,
    preflight_threshold: Option<u64>,
}

/// The `[keys]` table: one subtable per surface (`[keys.review]`,
/// `[keys.picker]`, `[keys.browse]`), so every keybinding lives under `keys`.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawKeys {
    #[serde(default)]
    review: RawReview,
    #[serde(default)]
    picker: RawPicker,
    #[serde(default)]
    browse: RawBrowse,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawReview {
    failed: Option<Vec<String>>,
    partly: Option<Vec<String>>,
    passed: Option<Vec<String>>,
    reveal: Option<Vec<String>>,
    hint: Option<Vec<String>>,
    submit: Option<Vec<String>>,
    skip: Option<Vec<String>>,
    remove: Option<Vec<String>>,
    promote: Option<Vec<String>>,
    r#continue: Option<Vec<String>>,
    restart: Option<Vec<String>>,
    ask: Option<Vec<String>>,
    save_note: Option<Vec<String>>,
    quit: Option<Vec<String>>,
}

impl Config {
    /// Parses a configuration from TOML text. Actions that are not mentioned
    /// keep their default bindings; an empty list disables an action.
    pub fn from_toml(text: &str) -> Result<Self> {
        let raw: RawConfig = toml::from_str(text).context("invalid config file")?;
        let mut keys = Bindings::default();

        let assign = |target: &mut Vec<KeyPattern>,
                      source: Option<Vec<String>>,
                      action: &str|
         -> Result<()> {
            if let Some(list) = source {
                *target = list
                    .iter()
                    .map(|s| parse_key(s).with_context(|| format!("in binding for {action:?}")))
                    .collect::<Result<_>>()?;
            }
            Ok(())
        };

        let review = raw.keys.review;
        assign(&mut keys.failed, review.failed, "review.failed")?;
        assign(&mut keys.partly, review.partly, "review.partly")?;
        assign(&mut keys.passed, review.passed, "review.passed")?;
        assign(&mut keys.reveal, review.reveal, "review.reveal")?;
        assign(&mut keys.hint, review.hint, "review.hint")?;
        assign(&mut keys.submit, review.submit, "review.submit")?;
        assign(&mut keys.skip, review.skip, "review.skip")?;
        assign(&mut keys.remove, review.remove, "review.remove")?;
        assign(&mut keys.promote, review.promote, "review.promote")?;
        assign(&mut keys.cont, review.r#continue, "review.continue")?;
        assign(&mut keys.restart, review.restart, "review.restart")?;
        assign(&mut keys.ask, review.ask, "review.ask")?;
        assign(&mut keys.save_note, review.save_note, "review.save_note")?;
        assign(&mut keys.quit, review.quit, "review.quit")?;

        let mut picker = PickerKeys::default();
        assign(&mut picker.up, raw.keys.picker.up, "picker.up")?;
        assign(&mut picker.down, raw.keys.picker.down, "picker.down")?;
        assign(&mut picker.open, raw.keys.picker.open, "picker.open")?;
        assign(&mut picker.back, raw.keys.picker.back, "picker.back")?;
        assign(&mut picker.filter, raw.keys.picker.filter, "picker.filter")?;
        assign(
            &mut picker.mastered,
            raw.keys.picker.mastered,
            "picker.mastered",
        )?;

        let mut browse = BrowseBindings::default();
        assign(&mut browse.next, raw.keys.browse.next, "browse.next")?;
        assign(&mut browse.prev, raw.keys.browse.prev, "browse.prev")?;
        assign(&mut browse.remove, raw.keys.browse.remove, "browse.remove")?;
        assign(&mut browse.quit, raw.keys.browse.quit, "browse.quit")?;

        let mut ask = AskConfig::default();
        if let Some(b) = raw.ask.backend.filter(|s| !s.trim().is_empty()) {
            ask.backend = match b.trim().to_ascii_lowercase().as_str() {
                "claude" => BackendKind::Claude,
                "gemini" => BackendKind::Gemini,
                "codex" => BackendKind::Codex,
                "copilot" => BackendKind::Copilot,
                _ => bail!("invalid ask.backend {b:?}: expected claude, gemini, codex, or copilot"),
            };
        }
        if let Some(command) = raw.ask.command {
            ask.command = command;
        }
        // An empty model string means "use the CLI default", like absence.
        if let Some(model) = raw.ask.model.filter(|m| !m.trim().is_empty()) {
            ask.model = Some(model);
        }
        if let Some(effort) = raw.ask.effort.filter(|e| !e.trim().is_empty()) {
            ask.effort = Some(effort);
        }
        if let Some(secs) = raw.ask.timeout_secs {
            ask.timeout_secs = secs;
        }
        if let Some(mode) = raw.ask.permission_mode {
            ask.permission_mode = mode;
        }
        if let Some(tools) = raw.ask.allowed_tools {
            ask.allowed_tools = tools;
        }
        if let Some(source_access) = raw.ask.source_access {
            ask.source_access = source_access;
        }
        if let Some(threshold) = raw.ask.preflight_threshold {
            ask.preflight_threshold = threshold;
        }

        let mut generate = GenerateDeckConfig::default();
        if let Some(model) = raw.generate.model.filter(|m| !m.trim().is_empty()) {
            generate.model = Some(model);
        }
        if let Some(secs) = raw.generate.timeout_secs {
            generate.timeout_secs = secs;
        }
        if let Some(max) = raw.generate.max_cards {
            generate.max_cards = max;
        }
        generate.extra = raw.generate.extra.filter(|s| !s.trim().is_empty());
        generate.prompt = raw.generate.prompt.filter(|s| !s.trim().is_empty());
        if let Some(review) = raw.generate.review {
            generate.review = review;
        }

        let mut exam = ExamConfig::default();
        if let Some(model) = raw.exam.model.filter(|m| !m.trim().is_empty()) {
            exam.model = Some(model);
        }
        if let Some(secs) = raw.exam.timeout_secs {
            exam.timeout_secs = secs;
        }
        if let Some(n) = raw.exam.num_questions {
            exam.num_questions = n;
        }
        if let Some(t) = raw.exam.pass_threshold {
            exam.pass_threshold = t;
        }
        if let Some(secs) = raw.exam.retry_cooldown_secs {
            exam.retry_cooldown_secs = secs;
        }
        if let Some(s) = raw.exam.strictness.filter(|s| !s.trim().is_empty()) {
            match Strictness::from_str(s.trim(), true) {
                Ok(v) => exam.strictness = v,
                Err(_) => {
                    bail!("invalid exam.strictness {s:?}: expected strict, balanced, or lenient")
                }
            }
        }
        exam.extra = raw.exam.extra.filter(|s| !s.trim().is_empty());

        let mut trace = TraceConfig::default();
        if let Some(model) = raw.trace.model.filter(|m| !m.trim().is_empty()) {
            trace.model = Some(model);
        }
        if let Some(effort) = raw.trace.effort.filter(|e| !e.trim().is_empty()) {
            trace.effort = Some(effort);
        }
        if let Some(secs) = raw.trace.timeout_secs {
            trace.timeout_secs = secs;
        }
        trace.extra = raw.trace.extra.filter(|s| !s.trim().is_empty());

        let mut ai = AiConfig::default();
        if let Some(model) = raw.ai.model.filter(|m| !m.trim().is_empty()) {
            ai.model = Some(model);
        }
        if let Some(count) = raw.ai.distractor_count {
            ai.distractor_count = count;
        }
        if let Some(count) = raw.ai.variant_count {
            ai.variant_count = count;
        }
        if let Some(count) = raw.ai.keypoint_count {
            ai.keypoint_count = count;
        }
        if let Some(secs) = raw.ai.timeout_secs {
            ai.timeout_secs = secs;
        }

        let mut serve = ServeConfig::default();
        if let Some(port) = raw.serve.port {
            serve.port = port;
        }
        serve.token = raw.serve.token;

        let mut review = ReviewConfig::default();
        if let Some(retention) = raw.review.retention {
            review.retention = retention.clamp(MIN_RETENTION, MAX_RETENTION);
        }
        if let Some(retire_after) = raw.review.retire_after {
            review.retire_after_days =
                parse_retire_after(&retire_after).context("in [review] retire_after")?;
        }
        if let Some(depth) = raw.review.depth {
            review.target = depth_to_rung(depth);
        }

        let decks_dir = raw.decks_dir.map(|s| expand_tilde(&s));

        Ok(Self {
            keys,
            picker,
            browse,
            ask,
            generate,
            exam,
            trace,
            ai,
            serve,
            review,
            decks_dir,
        })
    }

    /// Loads the configuration.
    ///
    /// With an explicit `path` the file must exist. Without one, the default
    /// location is used if present, otherwise the default configuration is
    /// returned.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = match path {
            Some(path) => path.to_path_buf(),
            None => match default_config_path() {
                Some(path) if path.exists() => path,
                _ => return Ok(Self::default()),
            },
        };
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read config file {}", path.display()))?;
        Self::from_toml(&text).with_context(|| format!("in config file {}", path.display()))
    }
}

/// The default location of the config file
/// (`~/.config/alix/config.toml` on Linux).
pub fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix").map(|dirs| dirs.config_dir().join("config.toml"))
}

/// The default decks directory (`~/decks`).
pub fn default_decks_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().join("decks"))
}

/// Expands a leading `~` to the home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(dirs) = directories::BaseDirs::new()
    {
        return dirs.home_dir().join(rest);
    }
    PathBuf::from(path)
}

/// Retention is clamped to a sane FSRS band — outside it the schedule degenerates.
const MIN_RETENTION: f64 = 0.70;
const MAX_RETENTION: f64 = 0.99;

/// Parses a `retire_after` value: `"never"` → `None` (retirement disabled), or
/// `<n><unit>` with unit `d`/`w`/`m`/`y` (days/weeks/months/years; a bare number is
/// days) → `Some(days)`. Weeks ×7, months ×30, years ×365.
fn parse_retire_after(s: &str) -> Result<Option<u32>> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("never") {
        return Ok(None);
    }
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let Ok(n) = num.trim().parse::<u32>() else {
        bail!("invalid retire_after {s:?}: expected e.g. \"1y\", \"2w\", \"30d\", or \"never\"");
    };
    let days = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "d" => n,
        "w" => n.saturating_mul(7),
        "m" => n.saturating_mul(30),
        "y" => n.saturating_mul(365),
        other => {
            bail!("invalid retire_after unit {other:?}: expected d, w, m, or y (or \"never\")")
        }
    };
    Ok(Some(days))
}

/// A self-documenting template for `config --init`: every option is shown
/// commented out at its default value, so the emitted file overrides nothing
/// (uncomment a line to change it; defaults you leave commented still track
/// future versions). Section headers stay active so a single line can be
/// uncommented beneath one.
pub fn default_config_toml() -> &'static str {
    r#"# alix configuration.
#
# Every option below is shown commented out at its default value, as a
# reference. Uncomment a line and edit it to override that default; lines you
# leave commented keep the built-in default, so improvements to the defaults
# in newer versions still reach you. Keep the section headers ([keys.review],
# [keys.picker], [keys.browse], [ask], [generate], [exam], [trace], [ai],
# [serve]) so an uncommented line lands in the right section.
#
# Keys are written as a single character ("j"), a special key name
# ("space", "enter", "tab", "esc", "backspace"), or either with a "ctrl-"
# prefix ("ctrl-s"). The first key of each list is shown in the footer.
#
# Note: while you are typing an answer (typing and fuzzy mode), plain
# character bindings are ignored so they cannot shadow text input; use
# ctrl-/special keys for hint, skip and quit.

# Directory the startup picker lists decks from (when `alix` is launched
# without deck arguments). A leading ~ is expanded. Defaults to ~/decks.
# decks_dir = "~/decks"

# Review key bindings (flip / typing / fuzzy / choice modes).
[keys.review]
# failed = ["1", "f"]           # self-graded: grade as failed (reset)
# partly = ["2", "p"]           # self-graded: grade as partly (FSRS Hard, still a pass)
# passed = ["3", "n"]           # self-graded: grade as passed (advance)
# reveal = ["space", "enter"]   # flip mode: show the answer
# hint = ["tab", "ctrl-h", "ctrl-backspace"]  # typing mode (fails the card)
# submit = ["enter"]            # fuzzy mode: submit the current line
# skip = ["ctrl-s"]             # requeue the current card without grading
# remove = ["ctrl-x"]           # mark the card for removal from the deck file
# promote = ["ctrl-p"]          # promote a virtual (remediation) card into its deck file
# continue = ["enter", "space"] # leave the feedback screen
# restart = ["r"]               # start a new session from the summary screen
# ask = ["?"]                   # ask the tutor about an answered card
# save_note = ["ctrl-n"]        # ask view: save a condensed note to the deck
# quit = ["esc", "ctrl-c"]      # quit the session

# Navigation keys for the deck picker (Vim-style by default). The arrow keys,
# Enter (open) and Esc (back) always work regardless of these; jumping to the
# first/last row is fixed at g / G / Home / End (like [keys.browse]).
[keys.picker]
# up = ["k"]                    # move up
# down = ["j"]                  # move down
# open = ["l"]                  # open the focused row
# back = ["h"]                  # step back / cancel
# filter = ["/", "ctrl-f"]      # start filtering
# mastered = ["m"]              # open the Mastered window

# Key bindings for `alix browse` (the read-only reader). Jumping to the first
# and last card is fixed to g / G / Home / End, and the arrow keys always
# move next/previous; these three are configurable:
[keys.browse]
# next = ["l", "n", "space"]    # next card
# prev = ["h", "p"]             # previous card
# remove = ["x"]                # mark the card for removal from the deck file
# quit = ["q", "esc", "ctrl-c"] # leave the browser

# Settings for the tutor integration. Questions are sent to the
# command below together with the card as context.
[ask]
# backend = "claude"            # AI CLI backend: claude | gemini | codex | copilot
# command = "claude"            # executable to run
# model = ""                    # --model override; empty = the CLI's default
# effort = ""                   # --effort: low|medium|high|xhigh|max; empty = CLI default
# timeout_secs = 120            # give up waiting after this many seconds
# Permission mode for the headless CLI. "dontAsk" silently denies any tool
# not listed below — no interactive prompt (which would hang -p mode).
# Other values: "bypassPermissions" (allow everything; unsafe), "default"
# (prompts, so it hangs headless). Empty omits the flag.
# permission_mode = "dontAsk"
# Tools the assistant may use. With "dontAsk" this is an exclusive
# allowlist; the defaults let it consult deck links but nothing else.
# allowed_tools = ["WebFetch", "WebSearch"]
# Let the tutor READ the card's source (Read/Glob/Grep at the deck's % source:
# project root) to verify its answer instead of relying on memory. Off by
# default: it grants the (possibly LAN-served) tutor file-read access — only
# enable on a machine and network you trust.
# source_access = false
# Pre-flight size guard: warn and confirm before spending a large model call on
# a local source tree bigger than this many bytes (0 = always proceed silently).
# preflight_threshold = 5000000

# AI deck generation (`alix deck <source>`). Reuses the [ask] command,
# permission mode and tool allowlist (WebFetch reads the page).
[generate]
# model = ""                    # --model override; empty = use [ask] / CLI default
# timeout_secs = 300            # generation is slower than a single question
# max_cards = 30                # upper bound on cards per deck
# extra = ""                    # extra guidance appended to the prompt
# prompt = ""                   # full prompt override; may use {url} and {max_cards}
# review = false                # run a second pass to drop redundant cards (--review)

# AI exam (`alix exam <deck>`). Generates open understanding questions from
# the deck's `% source:` and grades typed answers; passing marks the deck
# "mastered" and unlocks its dependents. Reuses the [ask] command, permission
# mode and tool allowlist (WebFetch reads a source URL).
[exam]
# model = ""                    # --model override; empty = use [ask] / CLI default
# timeout_secs = 300            # each exam call is slower than a single question
# num_questions = 5             # questions asked per sitting
# pass_threshold = 1.0          # fraction of questions that must fully pass (1.0 = all)
# strictness = "balanced"       # answer grading: strict | balanced | lenient
# retry_cooldown_secs = 3600    # wait this long before re-sitting a FAILED trace exam (0 = off)
# extra = ""                    # extra guidance appended to question generation

# Trace building (`alix trace --build <deck>`). Explores the deck's `% source:`
# to discover the path and writes the checkpoints back. Reuses the [ask] command,
# but runs with read-only file tools (Read/Glob/Grep, + WebFetch for a URL
# source) and the source root as the working directory — never a write/shell tool.
[trace]
# model = ""                    # empty = each backend's strong model (Claude: opus), then [ask] / CLI default
# effort = "high"               # default; --effort: low|medium|high|xhigh|max
# timeout_secs = 600            # exploring a source is the slowest call
# extra = ""                    # extra guidance appended to the build prompt

# AI deck augmentation (`alix deck augment <deck>`). Generates choice-mode
# distractors (and notes) into a sidecar cache beside your progress; review
# reads them. Reuses the [ask] command; generation is a tool-free text call.
[ai]
# model = ""                    # --model override; empty = use [ask] / CLI default
# distractor_count = 3          # wrong options generated per choice card
# variant_count = 4             # reworded question variants generated per card
# keypoint_count = 5            # max key points per card (explain-mode checklist)
# timeout_secs = 300            # a whole-deck batch is a big call; wait this long

# Local web frontend (`alix serve`). Binds to localhost by default; `--lan`
# exposes it to the network and `--port` overrides the port set here.
[serve]
# port = 7777                   # default port for `alix serve`

# Review pacing (how the FSRS scheduler paces you). Personal — a workspace can
# override these in its own alix.local.toml (which is never shared).
[review]
# retention = 0.9               # FSRS target retrievability (0.70–0.99); higher = shorter intervals
# retire_after = "1y"           # a card rests once its interval reaches this ("2w", "6m", "30d", or "never")
# depth = 1                     # how deep to drill: 1 = recall (default) · 2 = reconstruct
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_defaults_when_unset() {
        let config = Config::from_toml("").unwrap();
        assert_eq!(0.9, config.review.retention);
        assert_eq!(Some(365), config.review.retire_after_days);
    }

    #[test]
    fn review_parses_retention_and_retire_after() {
        let config =
            Config::from_toml("[review]\nretention = 0.95\nretire_after = \"2w\"\n").unwrap();
        assert_eq!(0.95, config.review.retention);
        assert_eq!(Some(14), config.review.retire_after_days);
    }

    #[test]
    fn review_retire_after_never_disables_retirement() {
        let config = Config::from_toml("[review]\nretire_after = \"never\"\n").unwrap();
        assert_eq!(None, config.review.retire_after_days);
    }

    #[test]
    fn review_retention_is_clamped_to_a_sane_band() {
        let config = Config::from_toml("[review]\nretention = 0.5\n").unwrap();
        assert_eq!(MIN_RETENTION, config.review.retention);
    }

    #[test]
    fn review_rejects_a_malformed_retire_after() {
        assert!(Config::from_toml("[review]\nretire_after = \"soon\"\n").is_err());
    }

    #[test]
    fn for_workspace_overlays_local_overrides() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("alix.local.toml"),
            "[review]\nretention = 0.85\nretire_after = \"never\"\n",
        )
        .unwrap();
        let resolved = ReviewConfig::default().for_workspace(dir.path());
        assert_eq!(0.85, resolved.retention);
        assert_eq!(None, resolved.retire_after_days);
    }

    #[test]
    fn for_workspace_keeps_base_for_unset_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("alix.local.toml"),
            "[review]\nretention = 0.85\n",
        )
        .unwrap();
        let base = ReviewConfig {
            retention: 0.9,
            retire_after_days: Some(30),
            ..Default::default()
        };
        let resolved = base.for_workspace(dir.path());
        assert_eq!(0.85, resolved.retention);
        assert_eq!(Some(30), resolved.retire_after_days); // unset key → base kept
    }

    #[test]
    fn review_depth_defaults_to_recall_and_parses_a_number() {
        assert_eq!(ReviewConfig::default().target, Level::Recall);
        let config = Config::from_toml("[review]\ndepth = 2\n").unwrap();
        assert_eq!(config.review.target, Level::Reconstruct);
    }

    #[test]
    fn depth_clamps_out_of_range_values() {
        assert_eq!(depth_to_rung(0), Level::Recall);
        assert_eq!(depth_to_rung(-5), Level::Recall);
        assert_eq!(depth_to_rung(1), Level::Recall);
        assert_eq!(depth_to_rung(2), Level::Reconstruct);
        assert_eq!(depth_to_rung(7), Level::Reconstruct);
    }

    #[test]
    fn a_leftover_target_key_is_rejected() {
        // `[review] target` was renamed to `depth`; deny_unknown_fields makes the
        // old key a hard parse error (the loud migration signal).
        assert!(Config::from_toml("[review]\ntarget = \"recall\"\n").is_err());
    }

    #[test]
    fn a_workspace_local_toml_overrides_the_global_depth() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.local.toml"), "[review]\ndepth = 2\n").unwrap();
        let base = ReviewConfig { target: Level::Recall, ..Default::default() };
        assert_eq!(Level::Reconstruct, base.for_workspace(dir.path()).target);
    }

    #[test]
    fn for_deck_applies_a_per_deck_depth_override() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "title = \"w\"\n").unwrap();
        std::fs::write(dir.path().join("vocab.txt"), "# q\n\ta\n").unwrap();
        std::fs::write(dir.path().join("concepts.txt"), "# q\n\ta\n").unwrap();
        std::fs::write(
            dir.path().join("alix.local.toml"),
            "[review]\ndepth = 1\n\n[review.deck.\"vocab.txt\"]\ndepth = 2\n",
        )
        .unwrap();
        // The named deck goes deeper...
        let vocab = dir.path().join("vocab.txt");
        assert_eq!(Level::Reconstruct, ReviewConfig::default().for_deck(&vocab).target);
        // ...an unnamed deck inherits the workspace depth (1 = recall).
        let other = dir.path().join("concepts.txt");
        assert_eq!(Level::Recall, ReviewConfig::default().for_deck(&other).target);
    }

    #[test]
    fn for_workspace_ignores_a_missing_or_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let base = ReviewConfig::default();
        assert_eq!(base, base.for_workspace(dir.path())); // missing → unchanged
        std::fs::write(dir.path().join("alix.local.toml"), "this is = = not toml\n").unwrap();
        assert_eq!(base, base.for_workspace(dir.path())); // malformed → unchanged
    }

    #[test]
    fn parse_retire_after_units() {
        assert_eq!(Some(365), parse_retire_after("1y").unwrap());
        assert_eq!(Some(180), parse_retire_after("6m").unwrap());
        assert_eq!(Some(14), parse_retire_after("2w").unwrap());
        assert_eq!(Some(30), parse_retire_after("30d").unwrap());
        assert_eq!(Some(45), parse_retire_after("45").unwrap());
        assert_eq!(None, parse_retire_after("never").unwrap());
        assert!(parse_retire_after("").is_err());
        assert!(parse_retire_after("1x").is_err());
    }

    #[test]
    fn parse_single_chars() {
        assert_eq!(
            KeyPattern {
                key: Key::Char('j'),
                ctrl: false
            },
            parse_key("j").unwrap()
        );
        assert_eq!(
            KeyPattern {
                key: Key::Char('1'),
                ctrl: false
            },
            parse_key("1").unwrap()
        );
    }

    #[test]
    fn parse_special_keys() {
        assert_eq!(Key::Char(' '), parse_key("space").unwrap().key);
        assert_eq!(Key::Enter, parse_key("enter").unwrap().key);
        assert_eq!(Key::Enter, parse_key("return").unwrap().key);
        assert_eq!(Key::Tab, parse_key("tab").unwrap().key);
        assert_eq!(Key::Esc, parse_key("ESC").unwrap().key);
        assert_eq!(Key::Backspace, parse_key("backspace").unwrap().key);
    }

    #[test]
    fn parse_ctrl_combinations() {
        assert_eq!(
            KeyPattern {
                key: Key::Char('s'),
                ctrl: true
            },
            parse_key("ctrl-s").unwrap()
        );
        assert_eq!(
            KeyPattern {
                key: Key::Backspace,
                ctrl: true
            },
            parse_key("ctrl-backspace").unwrap()
        );
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_key("jj").is_err());
        assert!(parse_key("").is_err());
        assert!(parse_key("ctrl-").is_err());
        assert!(parse_key("super-x").is_err());
    }

    #[test]
    fn display_labels() {
        assert_eq!("j", parse_key("j").unwrap().to_string());
        assert_eq!("SPACE", parse_key("space").unwrap().to_string());
        assert_eq!("Ctrl-S", parse_key("ctrl-s").unwrap().to_string().as_str());
        assert_eq!("ESC", parse_key("esc").unwrap().to_string());
    }

    #[test]
    fn defaults_are_consistent_and_an_empty_file_loads() {
        // Hermetic: never reads the user's real config dir (`Config::load(None)`
        // would). Defaults must be self-consistent, and a present-but-empty config
        // file loads as all-defaults.
        assert_eq!(Bindings::default(), Config::default().keys);
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "").unwrap();
        assert!(Config::load(Some(&cfg)).is_ok());
    }

    #[test]
    fn rebind_grades_to_jkl() {
        let config = Config::from_toml(
            "[keys.review]\nfailed = [\"j\"]\npartly = [\"k\"]\npassed = [\"l\"]\n",
        )
        .unwrap();
        assert_eq!(vec![parse_key("j").unwrap()], config.keys.failed);
        assert_eq!(vec![parse_key("k").unwrap()], config.keys.partly);
        assert_eq!(vec![parse_key("l").unwrap()], config.keys.passed);
        // Unmentioned actions keep their defaults.
        assert_eq!(Bindings::default().quit, config.keys.quit);
    }

    #[test]
    fn continue_is_a_valid_table_key() {
        let config = Config::from_toml("[keys.review]\ncontinue = [\"ctrl-n\"]\n").unwrap();
        assert_eq!(vec![parse_key("ctrl-n").unwrap()], config.keys.cont);
    }

    #[test]
    fn picker_keys_override_and_default() {
        let config = Config::from_toml("[keys.picker]\ndown = [\"n\"]\nopen = [\"o\"]\n").unwrap();
        assert_eq!(vec![parse_key("n").unwrap()], config.picker.down);
        assert_eq!(vec![parse_key("o").unwrap()], config.picker.open);
        // Unmentioned picker keys keep their Vim defaults.
        assert_eq!(PickerKeys::default().up, config.picker.up);
        assert_eq!(PickerKeys::default().mastered, config.picker.mastered);
    }

    #[test]
    fn unknown_action_is_rejected() {
        assert!(Config::from_toml("[keys.review]\nfrobnicate = [\"x\"]\n").is_err());
    }

    #[test]
    fn unknown_section_is_rejected() {
        assert!(Config::from_toml("[keyz]\nagain = [\"x\"]\n").is_err());
    }

    #[test]
    fn pre_nesting_key_sections_are_rejected() {
        // The old top-level [picker]/[browse] tables moved under [keys]; the old
        // spelling now errors loudly (no compat shim, pre-1.0), pointing the user
        // at the rename.
        assert!(Config::from_toml("[picker]\ndown = [\"n\"]\n").is_err());
        assert!(Config::from_toml("[browse]\nnext = [\"n\"]\n").is_err());
    }

    #[test]
    fn bad_key_in_binding_is_rejected() {
        let err = Config::from_toml("[keys.review]\nfailed = [\"jj\"]\n").unwrap_err();
        assert!(format!("{err:#}").contains("failed"));
    }

    #[test]
    fn template_parses_to_defaults() {
        // As written (all settings commented out), the template overrides
        // nothing.
        let config = Config::from_toml(default_config_toml()).unwrap();
        assert_eq!(Config::default(), config);
    }

    #[test]
    fn template_commented_values_equal_defaults() {
        // Uncomment every `# key = ...` setting line and confirm the result is
        // still exactly the defaults — so the documented example values are
        // correct and valid TOML, not just inert (possibly rotted) comments.
        // `decks_dir` is excluded: its example "~/decks" is the *effective*
        // default but the struct default is `None` ("use ~/decks").
        let uncommented = default_config_toml()
            .lines()
            .map(|line| match line.trim_start().strip_prefix("# ") {
                Some(rest) if is_setting_line(rest) => rest,
                _ => line,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let config = Config::from_toml(&uncommented).unwrap();
        assert_eq!(Config::default(), config);
    }

    /// `true` if `s` looks like a `key = ...` assignment for a rebindable
    /// setting (lower-case/underscore key), other than `decks_dir`.
    fn is_setting_line(s: &str) -> bool {
        let Some((key, _)) = s.split_once('=') else {
            return false;
        };
        let key = key.trim();
        !key.is_empty()
            && key != "decks_dir"
            && key.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    }

    #[test]
    fn ask_section_overrides_defaults() {
        let config = Config::from_toml(
            "[ask]\ncommand = \"my-claude\"\nmodel = \"haiku\"\ntimeout_secs = 30\n",
        )
        .unwrap();
        assert_eq!("my-claude", config.ask.command);
        assert_eq!(Some("haiku".to_string()), config.ask.model);
        assert_eq!(30, config.ask.timeout_secs);
        // Unmentioned fields keep their safe defaults.
        assert_eq!("dontAsk", config.ask.permission_mode);
        assert_eq!(vec!["WebFetch", "WebSearch"], config.ask.allowed_tools);
    }

    #[test]
    fn trace_defaults_to_high_effort_and_backend_chosen_model() {
        // Trace still breaks the inherit-the-CLI-default pattern on effort
        // (correctness-critical, fails silently), but the *model* is now left
        // unset here so each backend picks its own strong model
        // (`Backend::default_trace_model`); the effective model is resolved in
        // `trace::build_run_config`.
        let trace = Config::default().trace;
        assert_eq!(None, trace.model);
        assert_eq!(Some("high".to_string()), trace.effort);
    }

    #[test]
    fn trace_section_overrides_model_and_effort() {
        let config = Config::from_toml("[trace]\nmodel = \"sonnet\"\neffort = \"max\"\n").unwrap();
        assert_eq!(Some("sonnet".to_string()), config.trace.model);
        assert_eq!(Some("max".to_string()), config.trace.effort);
    }

    #[test]
    fn ask_effort_is_off_by_default_and_overridable() {
        assert_eq!(None, Config::default().ask.effort);
        let config = Config::from_toml("[ask]\neffort = \"medium\"\n").unwrap();
        assert_eq!(Some("medium".to_string()), config.ask.effort);
    }

    #[test]
    fn ai_defaults_to_three_distractors_and_cli_model() {
        let ai = Config::default().ai;
        assert_eq!(3, ai.distractor_count);
        assert_eq!(None, ai.model);
    }

    #[test]
    fn ai_section_overrides_defaults() {
        let config = Config::from_toml("[ai]\ndistractor_count = 5\nmodel = \"haiku\"\n").unwrap();
        assert_eq!(5, config.ai.distractor_count);
        assert_eq!(Some("haiku".to_string()), config.ai.model);
    }

    #[test]
    fn ai_empty_model_means_cli_default() {
        let config = Config::from_toml("[ai]\nmodel = \"\"\n").unwrap();
        assert_eq!(None, config.ai.model);
    }

    #[test]
    fn unknown_ai_setting_is_rejected() {
        assert!(Config::from_toml("[ai]\nbogus = 1\n").is_err());
    }

    #[test]
    fn decks_dir_defaults_to_none_and_expands_tilde() {
        assert_eq!(None, Config::default().decks_dir);
        let config = Config::from_toml("decks_dir = \"~/my-decks\"\n").unwrap();
        let dir = config.decks_dir.unwrap();
        assert!(dir.ends_with("my-decks"));
        assert!(!dir.to_string_lossy().contains('~'));
    }

    #[test]
    fn ask_permission_and_tools_are_configurable() {
        let config = Config::from_toml(
            "[ask]\npermission_mode = \"bypassPermissions\"\n\
             allowed_tools = [\"WebFetch\", \"Read\"]\n",
        )
        .unwrap();
        assert_eq!("bypassPermissions", config.ask.permission_mode);
        assert_eq!(vec!["WebFetch", "Read"], config.ask.allowed_tools);
    }

    #[test]
    fn empty_model_means_cli_default() {
        let config = Config::from_toml("[ask]\nmodel = \"\"\n").unwrap();
        assert_eq!(None, config.ask.model);
    }

    #[test]
    fn unknown_ask_setting_is_rejected() {
        assert!(Config::from_toml("[ask]\ntemperature = 1.0\n").is_err());
    }

    #[test]
    fn exam_strictness_defaults_to_balanced_and_parses() {
        assert_eq!(Strictness::Balanced, Config::default().exam.strictness);
        let config = Config::from_toml("[exam]\nstrictness = \"strict\"\n").unwrap();
        assert_eq!(Strictness::Strict, config.exam.strictness);
        // Case-insensitive, and other [exam] fields keep their defaults.
        let config = Config::from_toml("[exam]\nstrictness = \"LENIENT\"\n").unwrap();
        assert_eq!(Strictness::Lenient, config.exam.strictness);
        assert_eq!(5, config.exam.num_questions);
    }

    #[test]
    fn invalid_exam_strictness_is_rejected() {
        let err = Config::from_toml("[exam]\nstrictness = \"harsh\"\n").unwrap_err();
        assert!(format!("{err:#}").contains("strictness"));
    }

    #[test]
    fn browse_keys_default_to_vim_and_are_rebindable() {
        // Defaults: l/n/space next, h/p prev.
        let defaults = BrowseBindings::default();
        assert_eq!(parse_key("l").unwrap(), defaults.next[0]);
        assert_eq!(parse_key("h").unwrap(), defaults.prev[0]);

        let config = Config::from_toml("[keys.browse]\nnext = [\"j\"]\nprev = [\"k\"]\n").unwrap();
        assert_eq!(vec![parse_key("j").unwrap()], config.browse.next);
        assert_eq!(vec![parse_key("k").unwrap()], config.browse.prev);
        // Unmentioned browse actions keep their defaults.
        assert_eq!(defaults.quit, config.browse.quit);
    }

    #[test]
    fn unknown_browse_setting_is_rejected() {
        assert!(Config::from_toml("[keys.browse]\nfirst = [\"g\"]\n").is_err());
    }

    #[test]
    fn backend_defaults_to_claude() {
        assert_eq!(BackendKind::Claude, Config::default().ask.backend);
        // An empty config file also yields the default.
        let config = Config::from_toml("").unwrap();
        assert_eq!(BackendKind::Claude, config.ask.backend);
    }

    #[test]
    fn backend_gemini_parses() {
        let config = Config::from_toml("[ask]\nbackend = \"gemini\"\n").unwrap();
        assert_eq!(BackendKind::Gemini, config.ask.backend);
        // Other [ask] fields keep their defaults.
        assert_eq!("claude", config.ask.command);
    }

    #[test]
    fn backend_parsing_is_case_insensitive() {
        let config = Config::from_toml("[ask]\nbackend = \"Claude\"\n").unwrap();
        assert_eq!(BackendKind::Claude, config.ask.backend);
        let config = Config::from_toml("[ask]\nbackend = \"CODEX\"\n").unwrap();
        assert_eq!(BackendKind::Codex, config.ask.backend);
        let config = Config::from_toml("[ask]\nbackend = \"Copilot\"\n").unwrap();
        assert_eq!(BackendKind::Copilot, config.ask.backend);
    }

    #[test]
    fn unknown_backend_is_rejected() {
        let err = Config::from_toml("[ask]\nbackend = \"grok\"\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("backend"),
            "error should mention 'backend': {msg}"
        );
    }
}
