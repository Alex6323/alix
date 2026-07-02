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
mod codex;
mod gemini;

pub use claude::ClaudeBackend;
pub use codex::CodexBackend;
pub use gemini::GeminiBackend;

use crate::config::{AskConfig, BackendKind};

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

impl Access {
    /// Derives the abstract tool grant from a Claude-style allowlist, so a
    /// caller can hand a backend a CLI-independent grant. `files` is true when
    /// any of `Read`, `Glob`, or `Grep` appear; `fetch` maps to `WebFetch`;
    /// `search` maps to `WebSearch`. An empty allowlist yields `Access::None`.
    pub fn from_allowed_tools(tools: &[String]) -> Self {
        let has = |name: &str| tools.iter().any(|t| t == name);
        let files = has("Read") || has("Glob") || has("Grep");
        let fetch = has("WebFetch");
        let search = has("WebSearch");
        if !files && !fetch && !search {
            Access::None
        } else {
            Access::ReadOnly {
                files,
                fetch,
                search,
            }
        }
    }
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

    /// A backend-specific strong model for **trace building**, or `None` to
    /// inherit the CLI's own default. Trace is agentic and correctness-critical,
    /// so a backend that has a stronger model names it here; the config default
    /// is left unset so each backend can pick its own.
    fn default_trace_model(&self) -> Option<&'static str> {
        None
    }
}

/// Selects the backend for a config, returning an error for backends that are
/// not yet wired (Copilot — a later task flips its arm).
pub fn backend_for(cfg: &AskConfig) -> anyhow::Result<Box<dyn Backend>> {
    match cfg.backend {
        BackendKind::Claude => Ok(Box::new(ClaudeBackend)),
        BackendKind::Gemini => Ok(Box::new(GeminiBackend)),
        BackendKind::Codex => Ok(Box::new(CodexBackend)),
        BackendKind::Copilot => anyhow::bail!("the copilot backend isn't available yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AskConfig;

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn backend_for_wires_claude_gemini_codex_copilot_not_yet() {
        let mut cfg = AskConfig::default();
        assert!(backend_for(&cfg).is_ok(), "claude should be wired");

        cfg.backend = BackendKind::Gemini;
        assert!(backend_for(&cfg).is_ok(), "gemini should be wired");

        cfg.backend = BackendKind::Codex;
        assert!(backend_for(&cfg).is_ok(), "codex should be wired");

        cfg.backend = BackendKind::Copilot;
        let err = backend_for(&cfg).err().expect("copilot should error");
        assert!(
            format!("{err}").contains("copilot"),
            "error should name the backend"
        );
    }

    #[test]
    fn access_from_askconfig_maps_tools_to_grant() {
        // Read + Glob + Grep + WebFetch → files + fetch, no search
        let a = Access::from_allowed_tools(&tools(&["Read", "Glob", "Grep", "WebFetch"]));
        assert!(matches!(
            a,
            Access::ReadOnly {
                files: true,
                fetch: true,
                search: false
            }
        ));

        // WebFetch + WebSearch → no files, fetch + search
        let b = Access::from_allowed_tools(&tools(&["WebFetch", "WebSearch"]));
        assert!(matches!(
            b,
            Access::ReadOnly {
                files: false,
                fetch: true,
                search: true
            }
        ));

        // Read only → files, no fetch, no search
        let c = Access::from_allowed_tools(&tools(&["Read"]));
        assert!(matches!(
            c,
            Access::ReadOnly {
                files: true,
                fetch: false,
                search: false
            }
        ));

        // Empty → no tools at all
        let d = Access::from_allowed_tools(&[]);
        assert!(matches!(d, Access::None));
    }
}
