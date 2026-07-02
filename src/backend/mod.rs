//! AI backend abstraction: the per-CLI parts of an assistant invocation
//! behind one trait, so alix can drive different agent CLIs (Claude today;
//! Gemini/Codex/Copilot later) through the same [`ask::run`](crate::ask::run)
//! plumbing.
//!
//! A [`Backend`] owns the CLI-specific bits — the argv it wants for a given
//! [`RunOpts`], how the prompt reaches it ([`PromptDelivery`]), and how to pull
//! the answer out of stdout. The shared spawn/drain/timeout machinery stays in
//! `ask::run`. Tool access is expressed abstractly as an [`Access`] grant and
//! each backend renders it into its own flags, so a caller never names a
//! CLI-specific tool.

mod claude;

pub use claude::ClaudeBackend;

use crate::config::AskConfig;

/// How a backend receives the prompt text.
pub enum PromptDelivery {
    /// Feed the prompt on stdin (Claude's `-p` print mode).
    Stdin,
    /// Pass the prompt as a trailing command-line argument.
    Arg,
    /// A leading `exec` subcommand followed by the prompt argument.
    ExecArg,
}

/// The tool access a call grants the assistant, expressed independently of any
/// one CLI's tool names. Each backend renders it into its own flags.
pub enum Access {
    /// No tools — pure reasoning over the supplied text.
    None,
    /// Read-only access, opting into source reading and/or the web.
    ReadOnly {
        /// Read local files (Claude: `Read`, `Glob`, `Grep`).
        files: bool,
        /// Fetch a known URL (Claude: `WebFetch`).
        fetch: bool,
        /// Search the web (Claude: `WebSearch`).
        search: bool,
    },
}

/// The per-call knobs a backend turns into argv: model, effort, the tool
/// [`Access`] grant, and any session arguments (Claude `--session-id`/
/// `--resume`; ignored by backends without sessions).
pub struct RunOpts<'a> {
    /// Model passed through (`--model`); `None` uses the CLI's default.
    pub model: Option<&'a str>,
    /// Effort level (`--effort`); `None` omits the flag.
    pub effort: Option<&'a str>,
    /// Permission mode (`--permission-mode`); `None` omits the flag.
    pub permission_mode: Option<&'a str>,
    /// The tool access this call grants.
    pub access: Access,
    /// Session arguments; forwarded verbatim by backends that support them.
    pub session_args: &'a [String],
}

/// One assistant CLI, reduced to the parts that differ between CLIs. The shared
/// spawn/drain/timeout plumbing lives in [`ask::run`](crate::ask::run); a
/// backend only says what to run, how to hand over the prompt, and how to read
/// the answer back.
pub trait Backend: Send + Sync {
    /// The default executable name for this backend.
    fn command(&self) -> &str;

    /// The full argument vector for a call with these options.
    fn build_argv(&self, opts: &RunOpts) -> Vec<String>;

    /// How the prompt reaches the CLI.
    fn prompt_delivery(&self) -> PromptDelivery;

    /// Pulls the answer out of the CLI's stdout. Returns the trimmed text;
    /// an empty result is handled by the caller.
    fn extract(&self, stdout: &str) -> anyhow::Result<String>;

    /// Whether the backend can use tools (an agent), vs. a plain text model.
    fn agentic(&self) -> bool {
        true
    }

    /// Whether the backend can fetch web pages.
    fn can_fetch_web(&self) -> bool {
        true
    }

    /// Whether the backend can read local source files.
    fn can_read_source(&self) -> bool {
        true
    }

    /// Flags whose presence in `--help` confirms this is the expected CLI.
    fn required_help_flags(&self) -> &'static [&'static str];
}

/// Selects the backend for a config. Today it always returns the Claude
/// backend; Task 3 adds the match on a configured backend name.
pub fn backend_for(_cfg: &AskConfig) -> Box<dyn Backend> {
    Box::new(ClaudeBackend)
}
