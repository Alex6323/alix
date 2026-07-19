use anyhow::Result;

use super::{Backend, PromptDelivery, RunOpts};

pub struct CodexBackend;

impl Backend for CodexBackend {
    fn command(&self) -> &str {
        "codex"
    }

    fn build_argv(&self, opts: &RunOpts) -> Vec<String> {
        // --ask-for-approval must precede `exec` (codex rejects it afterward).
        // The sandbox, not opts.access, governs tool access, so it's unused here.
        let mut argv = vec![
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "exec".to_string(),
            "--sandbox".to_string(),
            "read-only".to_string(),
        ];

        // Codex has no --effort/--permission-mode equivalent, so both are dropped.
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
        false
    }

    fn required_help_flags(&self) -> &'static [&'static str] {
        &["exec", "--sandbox", "--ask-for-approval"]
    }

    fn name(&self) -> &'static str {
        "codex"
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
        assert!(matches!(
            CodexBackend.prompt_delivery(),
            PromptDelivery::ExecArg
        ));
        assert!(argv.iter().any(|a| a == "exec"), "argv: {argv:?}");
        assert_flag_value(&argv, "--sandbox", "read-only");
        assert_flag_value(&argv, "--ask-for-approval", "never");
        let approval_at = argv.iter().position(|a| a == "--ask-for-approval").unwrap();
        let exec_at = argv.iter().position(|a| a == "exec").unwrap();
        assert!(
            approval_at < exec_at,
            "--ask-for-approval must come before exec: {argv:?}"
        );
        assert!(!argv.iter().any(|a| a == "--allowedTools"));
        assert!(!argv.iter().any(|a| a == "--permission-mode"));
    }

    #[test]
    fn codex_model_flag_uses_short_form() {
        let argv = CodexBackend.build_argv(&RunOpts {
            model: Some("gpt-5"),
            effort: Some("high"), // no Codex equivalent, must be dropped
            permission_mode: Some("dontAsk"), // Claude-only, must be dropped
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
        assert!(!CodexBackend.can_fetch_web());
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
