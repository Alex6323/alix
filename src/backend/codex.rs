//! The OpenAI Codex CLI backend (`codex exec`, headless). Unlike Claude and
//! Gemini, Codex takes the prompt as a **command-line argument** (delivery
//! [`ExecArg`](PromptDelivery::ExecArg)), not on stdin, and its tool access is
//! governed by a **sandbox** rather than a per-tool allowlist: `--sandbox
//! read-only` lets it read files but blocks writes, shell escalation, and the
//! network, so the abstract [`Access`] grant maps to the sandbox as a whole
//! rather than to individual tool flags.
//!
//! Flags verified against the OpenAI Codex CLI docs and `codex exec --help`
//! (codex-cli 0.47.0) on 2026-07-02:
//! - non-interactive `codex exec` runs a single headless turn, printing the
//!   final agent message to stdout (progress → stderr):
//!   <https://developers.openai.com/codex/noninteractive>
//! - `--sandbox read-only` (read-only is the default sandbox); `-m/--model`; `-a/--ask-for-approval
//!   never` for unattended runs: <https://developers.openai.com/codex/cli/reference>
//! - **ordering quirk:** in codex-cli 0.47.0 `--ask-for-approval` is a *global* flag and is
//!   rejected after the `exec` subcommand, so it precedes `exec` (`codex --ask-for-approval never
//!   exec --sandbox read-only … <prompt>`). `--sandbox` is accepted after `exec`; the prompt is
//!   appended last by [`ask::run`](crate::ask::run).
//!
//! These names drift; a nightly `--help` check lands in Task 9.

use anyhow::Result;

use super::{Backend, PromptDelivery, RunOpts};

/// The OpenAI Codex CLI backend.
pub struct CodexBackend;

impl Backend for CodexBackend {
    fn command(&self) -> &str {
        "codex"
    }

    fn build_argv(&self, opts: &RunOpts) -> Vec<String> {
        // `--ask-for-approval never` is a global flag and must precede the
        // `exec` subcommand; `--sandbox read-only` follows it. Read-only is the
        // sandbox that maps the `Access` grant — it permits file reads but blocks
        // writes, shell escalation, and the network, so no per-tool flags exist
        // (a `fetch`/`search` request can't be honoured; see `can_fetch_web`).
        let mut argv = vec![
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "exec".to_string(),
            "--sandbox".to_string(),
            "read-only".to_string(),
        ];

        // Codex has no `--effort`/`--permission-mode` equivalent (those are
        // Claude-only), and the sandbox — not a tool allowlist — governs access.
        if let Some(model) = opts.model {
            argv.push("-m".to_string());
            argv.push(model.to_string());
        }
        argv.extend(opts.session_args.iter().cloned());
        argv
    }

    fn prompt_delivery(&self) -> PromptDelivery {
        PromptDelivery::ExecArg
    }

    fn extract(&self, stdout: &str) -> Result<String> {
        // `codex exec` prints only the final agent message to stdout.
        Ok(stdout.trim().to_string())
    }

    fn can_fetch_web(&self) -> bool {
        // The read-only sandbox blocks network access, so Codex can't fetch or
        // search the web under this profile.
        false
    }

    fn required_help_flags(&self) -> &'static [&'static str] {
        &["exec", "--sandbox", "--ask-for-approval"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Access;

    fn opts<'a>(access: Access, session_args: &'a [String]) -> RunOpts<'a> {
        RunOpts {
            model: None,
            effort: None,
            permission_mode: None,
            access,
            session_args,
        }
    }

    /// Finds `flag` in `argv` and asserts the value immediately after it equals
    /// `value`.
    fn assert_flag_value(argv: &[String], flag: &str, value: &str) {
        let at = argv
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} should be present in {argv:?}"));
        assert_eq!(argv[at + 1], value, "{flag} value in {argv:?}");
    }

    #[test]
    fn codex_uses_exec_subcommand_and_readonly_sandbox() {
        let argv = CodexBackend.build_argv(&opts(
            Access::ReadOnly {
                files: true,
                fetch: false,
                search: false,
            },
            &[],
        ));
        // The delivery is ExecArg: the prompt is appended by ask::run, not here.
        assert!(matches!(
            CodexBackend.prompt_delivery(),
            PromptDelivery::ExecArg
        ));
        // `exec` is present; the sandbox is read-only; approval is never.
        assert!(argv.iter().any(|a| a == "exec"), "argv: {argv:?}");
        assert_flag_value(&argv, "--sandbox", "read-only");
        assert_flag_value(&argv, "--ask-for-approval", "never");
        // The global approval flag must precede the `exec` subcommand (0.47.0).
        let approval_at = argv.iter().position(|a| a == "--ask-for-approval").unwrap();
        let exec_at = argv.iter().position(|a| a == "exec").unwrap();
        assert!(
            approval_at < exec_at,
            "--ask-for-approval must come before exec: {argv:?}"
        );
        // No Claude-only flags leak in.
        assert!(!argv.iter().any(|a| a == "--allowedTools"));
        assert!(!argv.iter().any(|a| a == "--permission-mode"));
    }

    #[test]
    fn codex_model_flag_uses_short_form() {
        let argv = CodexBackend.build_argv(&RunOpts {
            model: Some("gpt-5"),
            effort: Some("high"), // no Codex equivalent — must be dropped
            permission_mode: Some("dontAsk"), // Claude-only — must be dropped
            access: Access::None,
            session_args: &[],
        });
        assert_flag_value(&argv, "-m", "gpt-5");
        assert!(!argv.iter().any(|a| a == "--effort" || a == "high"));
        assert!(
            !argv
                .iter()
                .any(|a| a == "--permission-mode" || a == "dontAsk")
        );
    }

    #[test]
    fn codex_grant_does_not_change_argv() {
        // The sandbox is the read-only mechanism, so a files/fetch/search grant
        // renders no per-tool flags — the argv is the same as with no grant.
        let none = CodexBackend.build_argv(&opts(Access::None, &[]));
        let full = CodexBackend.build_argv(&opts(
            Access::ReadOnly {
                files: true,
                fetch: true,
                search: true,
            },
            &[],
        ));
        assert_eq!(none, full);
    }

    #[test]
    fn codex_cannot_fetch_web() {
        // Read-only sandbox blocks the network.
        assert!(!CodexBackend.can_fetch_web());
        // But it can read local source files.
        assert!(CodexBackend.can_read_source());
    }

    #[test]
    fn codex_extract_trims_final_message() {
        assert_eq!(
            "the final answer",
            CodexBackend.extract("  the final answer\n").unwrap()
        );
    }

    #[test]
    fn codex_help_flags() {
        let flags = CodexBackend.required_help_flags();
        assert!(flags.contains(&"exec"));
        assert!(flags.contains(&"--sandbox"));
        assert!(flags.contains(&"--ask-for-approval"));
    }
}
