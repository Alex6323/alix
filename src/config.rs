use std::{
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Esc,
    Backspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPattern {
    pub key: Key,
    pub ctrl: bool,
}

impl KeyPattern {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    pub failed: Vec<KeyPattern>,
    pub partly: Vec<KeyPattern>,
    pub passed: Vec<KeyPattern>,
    pub up: Vec<KeyPattern>,
    pub down: Vec<KeyPattern>,
    pub reveal: Vec<KeyPattern>,
    pub hint: Vec<KeyPattern>,
    pub submit: Vec<KeyPattern>,
    pub skip: Vec<KeyPattern>,
    pub remove: Vec<KeyPattern>,
    pub promote: Vec<KeyPattern>,
    pub cont: Vec<KeyPattern>,
    pub restart: Vec<KeyPattern>,
    pub ask: Vec<KeyPattern>,
    pub make_note: Vec<KeyPattern>,
    pub make_card: Vec<KeyPattern>,
    pub quit: Vec<KeyPattern>,
}

impl Default for Bindings {
    fn default() -> Self {
        let keys = |list: &[&str]| list.iter().map(|s| parse_key(s).unwrap()).collect();
        Self {
            failed: keys(&["1", "f"]),
            partly: keys(&["2", "p"]),
            passed: keys(&["3", "n"]),
            up: keys(&["k"]),
            down: keys(&["j"]),
            reveal: keys(&["space", "enter"]),
            hint: keys(&["tab", "ctrl-h", "ctrl-backspace"]),
            submit: keys(&["enter"]),
            skip: keys(&["ctrl-s"]),
            remove: keys(&["ctrl-x"]),
            promote: keys(&["ctrl-p"]),
            cont: keys(&["enter", "space"]),
            restart: keys(&["r"]),
            ask: keys(&["?"]),
            make_note: keys(&["ctrl-n"]),
            make_card: keys(&["ctrl-d"]),
            quit: keys(&["esc", "ctrl-c"]),
        }
    }
}

impl Bindings {
    pub fn label(list: &[KeyPattern]) -> String {
        list.first()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "?".to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerKeys {
    pub up: Vec<KeyPattern>,
    pub down: Vec<KeyPattern>,
    pub open: Vec<KeyPattern>,
    pub back: Vec<KeyPattern>,
    pub filter: Vec<KeyPattern>,
    pub mastered: Vec<KeyPattern>,
    pub depth: Vec<KeyPattern>,
    pub recognize: Vec<KeyPattern>,
    pub recall: Vec<KeyPattern>,
    pub reconstruct: Vec<KeyPattern>,
    pub cram: Vec<KeyPattern>,
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
            depth: keys(&["v"]),
            recognize: keys(&["1"]),
            recall: keys(&["2"]),
            reconstruct: keys(&["3"]),
            cram: keys(&["c"]),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowseBindings {
    pub next: Vec<KeyPattern>,
    pub prev: Vec<KeyPattern>,
    pub remove: Vec<KeyPattern>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackendKind {
    #[default]
    Claude,
    Gemini,
    Codex,
    Copilot,
}

impl BackendKind {
    pub fn name(self) -> &'static str {
        match self {
            BackendKind::Claude => "claude",
            BackendKind::Gemini => "gemini",
            BackendKind::Codex => "codex",
            BackendKind::Copilot => "copilot",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskConfig {
    pub backend: BackendKind,
    pub command: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub timeout_secs: u64,
    pub permission_mode: String,
    pub allowed_tools: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub source_access: bool,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerateDeckConfig {
    pub model: Option<String>,
    pub timeout_secs: u64,
    pub max_cards: usize,
    pub extra: Option<String>,
    pub prompt: Option<String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
pub enum Strictness {
    Strict,
    #[default]
    Balanced,
    Lenient,
}

impl Strictness {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "strict" => Some(Self::Strict),
            "balanced" => Some(Self::Balanced),
            "lenient" => Some(Self::Lenient),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExamConfig {
    pub model: Option<String>,
    pub timeout_secs: u64,
    pub num_questions: usize,
    pub pass_threshold: f64,
    pub strictness: Strictness,
    pub retry_cooldown_secs: u64,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceConfig {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub timeout_secs: u64,
    pub extra: Option<String>,
    pub auto_grade: bool,
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            model: None,
            effort: Some("high".to_string()),
            timeout_secs: 600,
            extra: None,
            auto_grade: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiConfig {
    pub model: Option<String>,
    pub distractor_count: usize,
    pub variant_count: usize,
    pub keypoint_count: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    #[default]
    Adult,
    Kids,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeConfig {
    pub port: u16,
    pub token: Option<String>,
    pub audience: Audience,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            port: 7777,
            token: None,
            audience: Audience::default(),
        }
    }
}

// Not `Eq`: `retention` is an `f64`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReviewConfig {
    pub retention: f64,
    pub retire_after_days: Option<u32>,
    pub acquire_cooldown_ms: u64,
    pub max_new: Option<usize>,
    pub limit: Option<usize>,
    pub deadline: Option<chrono::NaiveDate>,
    pub deadline_ramp_days: u32,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            retention: 0.9,
            retire_after_days: Some(crate::session::DEFAULT_RETIRE_AFTER_DAYS),
            acquire_cooldown_ms: crate::scheduler::DEFAULT_ACQUIRE_COOLDOWN_MS,
            max_new: None,
            limit: None,
            deadline: None,
            deadline_ramp_days: 14,
        }
    }
}

pub const LOCAL_MANIFEST: &str = "alix.local.toml";

impl ReviewConfig {
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
        if let Some(cooldown) = raw.review.acquire_cooldown
            && let Ok(ms) = parse_acquire_cooldown(&cooldown)
        {
            review.acquire_cooldown_ms = ms;
        }
        if let Some(n) = raw.review.max_new {
            review.max_new = Some(n);
        }
        if let Some(n) = raw.review.limit {
            review.limit = Some(n);
        }
        // Deadline/ramp apply only inside a real workspace: a bare folder has no
        // chip or lint to explain a ramping retention.
        if crate::workspace::is_workspace(workspace_dir) {
            if let Some(date) = raw.review.deadline
                && let Ok(d) = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            {
                review.deadline = Some(d);
            }
            if let Some(ramp) = raw.review.deadline_ramp
                && let Ok(days) = parse_ramp_days(&ramp)
            {
                review.deadline_ramp_days = days;
            }
        }
        review
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub keys: Bindings,
    pub picker: PickerKeys,
    pub browse: BrowseBindings,
    pub ask: AskConfig,
    pub generate: GenerateDeckConfig,
    pub exam: ExamConfig,
    pub trace: TraceConfig,
    pub ai: AiConfig,
    pub serve: ServeConfig,
    pub review: ReviewConfig,
    pub decks_dir: Option<PathBuf>,
}

impl Config {
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

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawReviewConfig {
    retention: Option<f64>,
    retire_after: Option<String>,
    acquire_cooldown: Option<String>,
    max_new: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawLocalReviewConfig {
    retention: Option<f64>,
    retire_after: Option<String>,
    acquire_cooldown: Option<String>,
    max_new: Option<usize>,
    limit: Option<usize>,
    deadline: Option<String>,
    deadline_ramp: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawLocalConfig {
    #[serde(default)]
    review: RawLocalReviewConfig,
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
    audience: Option<Audience>,
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
    auto_grade: Option<bool>,
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
    depth: Option<Vec<String>>,
    recognize: Option<Vec<String>>,
    recall: Option<Vec<String>>,
    reconstruct: Option<Vec<String>>,
    cram: Option<Vec<String>>,
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
    up: Option<Vec<String>>,
    down: Option<Vec<String>>,
    reveal: Option<Vec<String>>,
    hint: Option<Vec<String>>,
    submit: Option<Vec<String>>,
    skip: Option<Vec<String>>,
    remove: Option<Vec<String>>,
    promote: Option<Vec<String>>,
    r#continue: Option<Vec<String>>,
    restart: Option<Vec<String>>,
    ask: Option<Vec<String>>,
    make_note: Option<Vec<String>>,
    make_card: Option<Vec<String>>,
    quit: Option<Vec<String>>,
}

impl Config {
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
        assign(&mut keys.up, review.up, "review.up")?;
        assign(&mut keys.down, review.down, "review.down")?;
        assign(&mut keys.reveal, review.reveal, "review.reveal")?;
        assign(&mut keys.hint, review.hint, "review.hint")?;
        assign(&mut keys.submit, review.submit, "review.submit")?;
        assign(&mut keys.skip, review.skip, "review.skip")?;
        assign(&mut keys.remove, review.remove, "review.remove")?;
        assign(&mut keys.promote, review.promote, "review.promote")?;
        assign(&mut keys.cont, review.r#continue, "review.continue")?;
        assign(&mut keys.restart, review.restart, "review.restart")?;
        assign(&mut keys.ask, review.ask, "review.ask")?;
        assign(&mut keys.make_note, review.make_note, "review.make_note")?;
        assign(&mut keys.make_card, review.make_card, "review.make_card")?;
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
        assign(&mut picker.depth, raw.keys.picker.depth, "picker.depth")?;
        assign(
            &mut picker.recognize,
            raw.keys.picker.recognize,
            "picker.recognize",
        )?;
        assign(&mut picker.recall, raw.keys.picker.recall, "picker.recall")?;
        assign(
            &mut picker.reconstruct,
            raw.keys.picker.reconstruct,
            "picker.reconstruct",
        )?;
        assign(&mut picker.cram, raw.keys.picker.cram, "picker.cram")?;

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
            match Strictness::parse(s.trim()) {
                Some(v) => exam.strictness = v,
                None => {
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
        if let Some(auto_grade) = raw.trace.auto_grade {
            trace.auto_grade = auto_grade;
        }

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
        if let Some(audience) = raw.serve.audience {
            serve.audience = audience;
        }

        let mut review = ReviewConfig::default();
        if let Some(retention) = raw.review.retention {
            review.retention = retention.clamp(MIN_RETENTION, MAX_RETENTION);
        }
        if let Some(retire_after) = raw.review.retire_after {
            review.retire_after_days =
                parse_retire_after(&retire_after).context("in [review] retire_after")?;
        }
        if let Some(cooldown) = raw.review.acquire_cooldown {
            review.acquire_cooldown_ms =
                parse_acquire_cooldown(&cooldown).context("in [review] acquire_cooldown")?;
        }
        review.max_new = raw.review.max_new;
        review.limit = raw.review.limit;

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

pub fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix").map(|dirs| dirs.config_dir().join("config.toml"))
}

/// The directory holding launch-profile configs: `<config-dir>/profiles`.
pub fn profiles_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix").map(|dirs| dirs.config_dir().join("profiles"))
}

pub fn default_decks_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().join("decks"))
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(dirs) = directories::BaseDirs::new()
    {
        return dirs.home_dir().join(rest);
    }
    PathBuf::from(path)
}

/// Retention is clamped to a sane FSRS band; outside it the schedule degenerates.
const MIN_RETENTION: f64 = 0.70;
const MAX_RETENTION: f64 = 0.99;

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

fn parse_acquire_cooldown(s: &str) -> Result<u64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let Ok(n) = num.trim().parse::<u64>() else {
        bail!("invalid acquire_cooldown {s:?}: expected e.g. \"90s\", \"5m\", or \"1h\"");
    };
    let ms = match unit.trim().to_ascii_lowercase().as_str() {
        "s" => n.saturating_mul(1000),
        "" | "m" => n.saturating_mul(60 * 1000),
        "h" => n.saturating_mul(60 * 60 * 1000),
        other => {
            bail!("invalid acquire_cooldown unit {other:?}: expected s, m, or h")
        }
    };
    Ok(ms)
}

fn parse_ramp_days(s: &str) -> Result<u32> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let Ok(n) = num.trim().parse::<u32>() else {
        bail!("invalid deadline_ramp {s:?}: expected e.g. \"14d\", \"2w\", or \"0\"");
    };
    match unit.trim().to_ascii_lowercase().as_str() {
        "" | "d" => Ok(n),
        "w" => Ok(n.saturating_mul(7)),
        other => bail!("invalid deadline_ramp unit {other:?}: expected d or w"),
    }
}

pub fn local_review_lint(dir: &Path) -> Vec<String> {
    let path = dir.join(LOCAL_MANIFEST);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let raw: RawLocalConfig = match toml::from_str(&text) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut complaints = Vec::new();

    if let Some(date) = &raw.review.deadline
        && let Err(e) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
    {
        complaints.push(format!(
            "deadline = {date:?}: invalid date (expected YYYY-MM-DD). {e}"
        ));
    }

    if let Some(ramp) = &raw.review.deadline_ramp
        && let Err(e) = parse_ramp_days(ramp)
    {
        complaints.push(format!("deadline_ramp = {ramp:?}: {e}"));
    }

    complaints
}

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
# Note: while you are typing an answer (typing and typeline mode), plain
# character bindings are ignored so they cannot shadow text input; use
# ctrl-/special keys for hint, skip and quit.

# Directory the startup picker lists decks from (when `alix` is launched
# without deck arguments). A leading ~ is expanded. Defaults to ~/decks.
# decks_dir = "~/decks"

# Review key bindings (flip / typing / typeline / choice modes).
[keys.review]
# failed = ["1", "f"]           # self-graded: grade as failed (reset)
# partly = ["2", "p"]           # self-graded: grade as partly (FSRS Hard, still a pass)
# passed = ["3", "n"]           # self-graded: grade as passed (advance)
# reveal = ["space", "enter"]   # flip mode: show the answer
# hint = ["tab", "ctrl-h", "ctrl-backspace"]  # typing mode (fails the card)
# submit = ["enter"]            # typeline mode: submit the current line
# skip = ["ctrl-s"]             # requeue the current card without grading
# remove = ["ctrl-x"]           # mark the card for removal from the deck file
# promote = ["ctrl-p"]          # promote a virtual (remediation) card into its deck file
# continue = ["enter", "space"] # leave the feedback screen
# restart = ["r"]               # start a new session from the summary screen
# ask = ["?"]                   # ask the tutor about an answered card
# make_note = ["ctrl-n"]        # ask view: condense the conversation into a note
# make_card = ["ctrl-d"]        # ask view: distill the conversation into a card
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
# depth = ["v"]                 # open the focused deck's depth menu (Esc closes it too)
# recognize = ["1"]             # with the menu open, start at that depth;
# recall = ["2"]                #   Enter always starts the highlighted
# reconstruct = ["3"]           #   (last-used) one
# cram = ["c"]                  # toggle the menu's cram tick-box (serve not-yet-due cards too)

# Key bindings for the read-only Browse overlay (b in the picker). Jumping to the first
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

# Trace building (`alix generate <trace-stub>` / `--trace`). Explores the `% source:`
# to discover the path and writes the checkpoints back. Reuses the [ask] command,
# but runs with read-only file tools (Read/Glob/Grep, + WebFetch for a URL
# source) and the source root as the working directory — never a write/shell tool.
[trace]
# model = ""                    # empty = each backend's strong model (Claude: opus), then [ask] / CLI default
# effort = "high"               # default; --effort: low|medium|high|xhigh|max
# timeout_secs = 600            # exploring a source is the slowest call
# extra = ""                    # extra guidance appended to the build prompt
# auto_grade = false            # AI-grade walk predictions (a model call per hop)

# AI deck augmentation (`alix deck augment <deck>`). Generates choice-mode
# distractors (and notes) into a sidecar cache beside your progress; review
# reads them. Reuses the [ask] command; generation is a tool-free text call.
[ai]
# model = ""                    # --model override; empty = use [ask] / CLI default
# distractor_count = 3          # wrong options generated per choice card
# variant_count = 4             # reworded question variants generated per card
# keypoint_count = 5            # max key points per card (explain-mode checklist)
# timeout_secs = 300            # a whole-deck batch is a big call; wait this long

# The web server bare `alix` starts. Binds to localhost by default; `--lan`
# exposes it to the network and `--port` overrides the port set here.
[serve]
# port = 7777                   # default port (--port overrides per instance)
# audience = "adult"            # or "kids" — which frontend `/` serves, and the tutor's voice

# Review pacing (how the FSRS scheduler paces you). Personal — a workspace can
# override these in its own alix.local.toml (which is never shared).
[review]
# retention = 0.9               # FSRS target retrievability (0.70–0.99); higher = shorter intervals
# retire_after = "1y"           # a card rests once its interval reaches this ("2w", "6m", "30d", or "never")
# acquire_cooldown = "5m"       # settle gap before a new card's first quiz, and the same-card retry floor ("90s", "10m"; "0" = none)
# max_new = 10                  # max never-seen cards a session introduces (--new overrides per instance)
# limit = 40                    # session size cap (--limit overrides; unset = no cap)
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
        assert_eq!(
            crate::scheduler::DEFAULT_ACQUIRE_COOLDOWN_MS,
            config.review.acquire_cooldown_ms
        );
    }

    #[test]
    fn review_parses_acquire_cooldown_units() {
        let load = |v: &str| {
            Config::from_toml(&format!("[review]\nacquire_cooldown = \"{v}\"\n"))
                .unwrap()
                .review
                .acquire_cooldown_ms
        };
        assert_eq!(90_000, load("90s"));
        assert_eq!(600_000, load("10m"));
        assert_eq!(3_600_000, load("1h"));
        assert_eq!(120_000, load("2"), "a bare number is minutes");
        assert_eq!(0, load("0"), "zero disables the cooldown");
    }

    #[test]
    fn review_rejects_a_malformed_acquire_cooldown() {
        assert!(Config::from_toml("[review]\nacquire_cooldown = \"soon\"\n").is_err());
        assert!(Config::from_toml("[review]\nacquire_cooldown = \"5x\"\n").is_err());
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
    fn for_workspace_overlays_the_acquire_cooldown() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("alix.local.toml"),
            "[review]\nacquire_cooldown = \"90s\"\n",
        )
        .unwrap();
        let resolved = ReviewConfig::default().for_workspace(dir.path());
        assert_eq!(90_000, resolved.acquire_cooldown_ms);
        assert_eq!(0.9, resolved.retention);
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
        assert_eq!(Some(30), resolved.retire_after_days);
    }

    #[test]
    fn review_pacing_keys_parse_and_default_to_unset() {
        let config = Config::from_toml("[review]\nmax_new = 5\nlimit = 40\n").unwrap();
        assert_eq!(Some(5), config.review.max_new);
        assert_eq!(Some(40), config.review.limit);
        let bare = Config::from_toml("").unwrap();
        assert_eq!(None, bare.review.max_new);
        assert_eq!(None, bare.review.limit);
    }

    #[test]
    fn a_review_depth_key_is_now_rejected() {
        assert!(Config::from_toml("[review]\ndepth = 2\n").is_err());
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
    fn review_deadline_defaults_to_none_and_ramp_to_14() {
        let review = ReviewConfig::default();
        assert_eq!(None, review.deadline);
        assert_eq!(14, review.deadline_ramp_days);
    }

    #[test]
    fn for_workspace_overlays_deadline_and_ramp() {
        let dir = tempfile::tempdir().unwrap();
        // The deadline overlay only fires inside a real workspace (manifest + a deck).
        std::fs::write(dir.path().join("alix.toml"), "title = \"W\"\n").unwrap();
        std::fs::write(dir.path().join("a.md"), "## q\na\n").unwrap();
        std::fs::write(
            dir.path().join("alix.local.toml"),
            "[review]\ndeadline = \"2026-09-01\"\ndeadline_ramp = \"3w\"\n",
        )
        .unwrap();
        let resolved = ReviewConfig::default().for_workspace(dir.path());
        assert_eq!(
            Some(chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()),
            resolved.deadline
        );
        assert_eq!(21, resolved.deadline_ramp_days);
    }

    #[test]
    fn for_workspace_ignores_a_malformed_deadline_but_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "title = \"W\"\n").unwrap();
        std::fs::write(dir.path().join("a.md"), "## q\na\n").unwrap();
        std::fs::write(
            dir.path().join("alix.local.toml"),
            "[review]\ndeadline = \"soonish\"\nretention = 0.85\n",
        )
        .unwrap();
        let resolved = ReviewConfig::default().for_workspace(dir.path());
        assert_eq!(None, resolved.deadline);
        assert_eq!(0.85, resolved.retention);
    }

    #[test]
    fn a_plain_folder_gets_no_deadline_overlay() {
        let dir = tempfile::tempdir().unwrap();
        // Deliberately no alix.toml manifest: a bare tempdir is not a workspace.
        std::fs::write(
            dir.path().join("alix.local.toml"),
            "[review]\ndeadline = \"2026-09-01\"\nretention = 0.85\n",
        )
        .unwrap();
        let resolved = ReviewConfig::default().for_workspace(dir.path());
        assert_eq!(None, resolved.deadline, "a plain folder's deadline is dead");
        assert_eq!(0.85, resolved.retention, "pacing keys stay folder-friendly");
    }

    #[test]
    fn ramp_days_parse_units_and_zero() {
        assert_eq!(14, parse_ramp_days("14d").unwrap());
        assert_eq!(21, parse_ramp_days("3w").unwrap());
        assert_eq!(10, parse_ramp_days("10").unwrap(), "bare number is days");
        assert_eq!(0, parse_ramp_days("0").unwrap(), "0 = cap only, no ramp");
        assert!(parse_ramp_days("soon").is_err());
        assert!(parse_ramp_days("5m").is_err(), "months are not a ramp unit");
    }

    #[test]
    fn the_global_config_rejects_deadline_keys() {
        // A deadline is a per-workspace concept; the global [review] table
        // must refuse it loudly (serde deny_unknown_fields).
        assert!(Config::from_toml("[review]\ndeadline = \"2026-09-01\"\n").is_err());
        assert!(Config::from_toml("[review]\ndeadline_ramp = \"14d\"\n").is_err());
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
        // Hermetic: a tempdir, never the user's real config dir (`Config::load(None)`).
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
        assert_eq!(Bindings::default().quit, config.keys.quit);
    }

    #[test]
    fn continue_is_a_valid_table_key() {
        let config = Config::from_toml("[keys.review]\ncontinue = [\"ctrl-n\"]\n").unwrap();
        assert_eq!(vec![parse_key("ctrl-n").unwrap()], config.keys.cont);
    }

    #[test]
    fn review_nav_keys_default_to_k_j_and_can_be_rebound() {
        assert_eq!(vec![parse_key("k").unwrap()], Bindings::default().up);
        assert_eq!(vec![parse_key("j").unwrap()], Bindings::default().down);
        let config = Config::from_toml("[keys.review]\nup = [\"w\"]\ndown = [\"s\"]\n").unwrap();
        assert_eq!(vec![parse_key("w").unwrap()], config.keys.up);
        assert_eq!(vec![parse_key("s").unwrap()], config.keys.down);
    }

    #[test]
    fn tutor_distill_keys_default_and_can_be_rebound() {
        assert_eq!(
            vec![parse_key("ctrl-n").unwrap()],
            Bindings::default().make_note
        );
        assert_eq!(
            vec![parse_key("ctrl-d").unwrap()],
            Bindings::default().make_card
        );
        let config = Config::from_toml(
            "[keys.review]\nmake_note = [\"ctrl-y\"]\nmake_card = [\"ctrl-u\"]\n",
        )
        .unwrap();
        assert_eq!(vec![parse_key("ctrl-y").unwrap()], config.keys.make_note);
        assert_eq!(vec![parse_key("ctrl-u").unwrap()], config.keys.make_card);
    }

    #[test]
    fn picker_keys_override_and_default() {
        let config = Config::from_toml("[keys.picker]\ndown = [\"n\"]\nopen = [\"o\"]\n").unwrap();
        assert_eq!(vec![parse_key("n").unwrap()], config.picker.down);
        assert_eq!(vec![parse_key("o").unwrap()], config.picker.open);
        assert_eq!(PickerKeys::default().up, config.picker.up);
        assert_eq!(PickerKeys::default().mastered, config.picker.mastered);
    }

    #[test]
    fn depth_menu_keys_override_and_default() {
        let config =
            Config::from_toml("[keys.picker]\ndepth = [\"s\"]\nreconstruct = [\"3\", \"e\"]\n")
                .unwrap();
        assert_eq!(vec![parse_key("s").unwrap()], config.picker.depth);
        assert_eq!(
            vec![parse_key("3").unwrap(), parse_key("e").unwrap()],
            config.picker.reconstruct
        );
        assert_eq!(vec![parse_key("1").unwrap()], config.picker.recognize);
        assert_eq!(vec![parse_key("2").unwrap()], config.picker.recall);
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
        let config = Config::from_toml(default_config_toml()).unwrap();
        assert_eq!(Config::default(), config);
    }

    #[test]
    fn template_commented_values_equal_defaults() {
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

    // Excludes decks_dir/max_new/limit: their template values are effective
    // defaults, not the struct's actual `None`.
    fn is_setting_line(s: &str) -> bool {
        let Some((key, _)) = s.split_once('=') else {
            return false;
        };
        let key = key.trim();
        !key.is_empty()
            && !matches!(key, "decks_dir" | "max_new" | "limit")
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
        assert_eq!("dontAsk", config.ask.permission_mode);
        assert_eq!(vec!["WebFetch", "WebSearch"], config.ask.allowed_tools);
    }

    #[test]
    fn trace_defaults_to_high_effort_and_backend_chosen_model() {
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
    fn serve_audience_defaults_to_adult_and_parses_kids() {
        let def = Config::default();
        assert_eq!(def.serve.audience, Audience::Adult);
        let c = Config::from_toml("[serve]\naudience = \"kids\"\n").unwrap();
        assert_eq!(c.serve.audience, Audience::Kids);
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
        let defaults = BrowseBindings::default();
        assert_eq!(parse_key("l").unwrap(), defaults.next[0]);
        assert_eq!(parse_key("h").unwrap(), defaults.prev[0]);

        let config = Config::from_toml("[keys.browse]\nnext = [\"j\"]\nprev = [\"k\"]\n").unwrap();
        assert_eq!(vec![parse_key("j").unwrap()], config.browse.next);
        assert_eq!(vec![parse_key("k").unwrap()], config.browse.prev);
        assert_eq!(defaults.quit, config.browse.quit);
    }

    #[test]
    fn unknown_browse_setting_is_rejected() {
        assert!(Config::from_toml("[keys.browse]\nfirst = [\"g\"]\n").is_err());
    }

    #[test]
    fn backend_defaults_to_claude() {
        assert_eq!(BackendKind::Claude, Config::default().ask.backend);
        let config = Config::from_toml("").unwrap();
        assert_eq!(BackendKind::Claude, config.ask.backend);
    }

    #[test]
    fn backend_gemini_parses() {
        let config = Config::from_toml("[ask]\nbackend = \"gemini\"\n").unwrap();
        assert_eq!(BackendKind::Gemini, config.ask.backend);
        assert_eq!("claude", config.ask.command);
    }

    #[test]
    fn backend_names_round_trip_through_the_config_parser() {
        for kind in [
            BackendKind::Claude,
            BackendKind::Gemini,
            BackendKind::Codex,
            BackendKind::Copilot,
        ] {
            let toml = format!("[ask]\nbackend = \"{}\"\n", kind.name());
            let config = Config::from_toml(&toml).unwrap();
            assert_eq!(
                kind, config.ask.backend,
                "name() must be the parser's inverse"
            );
        }
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

    #[test]
    fn local_review_lint_flags_malformed_deadline_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(LOCAL_MANIFEST),
            "[review]\ndeadline = \"soonish\"\ndeadline_ramp = \"5m\"\n",
        )
        .unwrap();
        let complaints = local_review_lint(dir.path());
        assert_eq!(2, complaints.len());
        assert!(complaints[0].contains("deadline"));

        std::fs::write(
            dir.path().join(LOCAL_MANIFEST),
            "[review]\ndeadline = \"2026-09-01\"\n",
        )
        .unwrap();
        assert!(local_review_lint(dir.path()).is_empty());
        let empty = tempfile::tempdir().unwrap();
        assert!(local_review_lint(empty.path()).is_empty());
    }
}

#[cfg(all(test, feature = "full"))]
mod clap_parity {
    use clap::ValueEnum;

    use super::*;

    #[test]
    fn parse_matches_the_clap_value_names() {
        for variant in Strictness::value_variants() {
            let name = variant.to_possible_value().expect("a value name");
            assert_eq!(
                Some(*variant),
                Strictness::parse(name.get_name()),
                "{name:?}"
            );
        }
        assert_eq!(None, Strictness::parse("no-such-value"));
    }
}
