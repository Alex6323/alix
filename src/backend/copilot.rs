use anyhow::Result;

use super::{Access, Backend, PromptDelivery, RunOpts};

/// The GitHub Copilot CLI backend.
pub struct CopilotBackend;

impl Backend for CopilotBackend {
    fn command(&self) -> &str {
        "copilot"
    }

    fn build_argv(&self, opts: &RunOpts) -> Vec<String> {
        // `-s` suppresses decoration so stdout is just the response text.
        let mut argv = vec!["-s".to_string()];

        // Always denies shell/write, even for Access::None, so the model can't
        // run a destructive tool despite a text-only prompt.
        argv.push("--deny-tool".to_string());
        argv.push("shell,write".to_string());

        if let Access::ReadOnly {
            files,
            fetch,
            search,
        } = opts.access
        {
            let mut available: Vec<&str> = Vec::new();
            if files {
                available.push("read");
            }
            if fetch || search {
                // `url` covers both fetch and search; Copilot has no separate search tool.
                available.push("url");
            }
            if !available.is_empty() {
                // Omitting `--available-tools` leaves every non-denied tool
                // available, so this is only pushed when the read-only grant
                // actually restricts.
                argv.push("--available-tools".to_string());
                argv.push(available.join(","));
            }
        }

        if let Some(model) = opts.model {
            argv.push(format!("--model={model}"));
        }
        argv.extend(opts.session_args.iter().cloned());

        // `-p` must be the final flag so that `ask::run` (PromptDelivery::Arg)
        // appends the prompt text as the very next argument.
        argv.push("-p".to_string());
        argv
    }

    fn prompt_delivery(&self) -> PromptDelivery {
        PromptDelivery::Arg
    }

    fn extract(&self, stdout: &str) -> Result<String> {
        // `-s` already strips stats/decoration; trim any surrounding whitespace.
        Ok(stdout.trim().to_string())
    }

    fn can_fetch_web(&self) -> bool {
        true
    }

    fn required_help_flags(&self) -> &'static [&'static str] {
        &["-p", "-s", "--model", "--deny-tool", "--available-tools"]
    }

    fn name(&self) -> &'static str {
        "copilot"
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

    fn flag_value<'a>(argv: &'a [String], flag: &str) -> &'a str {
        let at = argv
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} should be present in {argv:?}"));
        argv.get(at + 1)
            .unwrap_or_else(|| panic!("{flag} has no following value in {argv:?}"))
    }

    #[test]
    fn copilot_grant_allows_read_denies_shell() {
        let argv = CopilotBackend.build_argv(&opts(
            Access::ReadOnly {
                files: true,
                fetch: true,
                search: true,
            },
            &[],
        ));

        assert!(argv.iter().any(|a| a == "-p"), "must contain -p: {argv:?}");
        assert_eq!(argv.last().unwrap(), "-p", "-p must be last: {argv:?}");

        let denied = flag_value(&argv, "--deny-tool");
        assert!(
            denied.contains("shell"),
            "--deny-tool must include shell: {denied}"
        );
        assert!(
            denied.contains("write"),
            "--deny-tool must include write: {denied}"
        );

        let available = flag_value(&argv, "--available-tools");
        assert!(
            available.contains("read"),
            "--available-tools must include read: {available}"
        );
        assert!(
            available.contains("url"),
            "--available-tools must include url for fetch+search: {available}"
        );

        assert!(
            !argv.iter().any(|a| a == "--allow-all-tools"),
            "must not use --allow-all-tools: {argv:?}"
        );
    }

    #[test]
    fn copilot_files_only_grant_omits_url() {
        let argv = CopilotBackend.build_argv(&opts(
            Access::ReadOnly {
                files: true,
                fetch: false,
                search: false,
            },
            &[],
        ));
        let available = flag_value(&argv, "--available-tools");
        assert!(available.contains("read"));
        assert!(
            !available.contains("url"),
            "url should be absent: {available}"
        );
    }

    #[test]
    fn copilot_fetch_without_files_includes_url() {
        let argv = CopilotBackend.build_argv(&opts(
            Access::ReadOnly {
                files: false,
                fetch: true,
                search: false,
            },
            &[],
        ));
        let available = flag_value(&argv, "--available-tools");
        assert!(available.contains("url"));
        assert!(
            !available.contains("read"),
            "read should be absent: {available}"
        );
    }

    #[test]
    fn copilot_no_grant_omits_available_tools() {
        let argv = CopilotBackend.build_argv(&opts(Access::None, &[]));
        assert!(
            !argv.iter().any(|a| a == "--available-tools"),
            "--available-tools must be absent with no grant: {argv:?}"
        );
        let denied = flag_value(&argv, "--deny-tool");
        assert!(denied.contains("shell"));
        assert!(denied.contains("write"));
        assert_eq!(argv.last().unwrap(), "-p");
    }

    #[test]
    fn copilot_model_flag_uses_equals_form() {
        let argv = CopilotBackend.build_argv(&RunOpts {
            model: Some("claude-sonnet-4-6"),
            effort: Some("high"),
            permission_mode: Some("x"),
            access: Access::None,
            session_args: &[],
        });
        assert!(
            argv.iter().any(|a| a == "--model=claude-sonnet-4-6"),
            "--model= form must be present: {argv:?}"
        );
        assert!(!argv.iter().any(|a| a == "--effort" || a == "high"));
        assert!(!argv.iter().any(|a| a == "--permission-mode" || a == "x"));
    }

    #[test]
    fn copilot_extract_trims_output() {
        assert_eq!(
            "the answer",
            CopilotBackend.extract("  the answer\n").unwrap()
        );
    }

    #[test]
    fn copilot_caps_and_help_flags() {
        assert!(CopilotBackend.can_fetch_web());
        let flags = CopilotBackend.required_help_flags();
        assert!(flags.contains(&"-p"));
        assert!(flags.contains(&"-s"));
        assert!(flags.contains(&"--model"));
        assert!(flags.contains(&"--deny-tool"));
        assert!(flags.contains(&"--available-tools"));
    }

    #[test]
    fn copilot_prompt_delivery_is_arg() {
        assert!(matches!(
            CopilotBackend.prompt_delivery(),
            PromptDelivery::Arg
        ));
    }

    #[test]
    fn copilot_silent_flag_is_always_present() {
        for access in [
            Access::None,
            Access::ReadOnly {
                files: true,
                fetch: true,
                search: true,
            },
        ] {
            let argv = CopilotBackend.build_argv(&opts(access, &[]));
            assert!(
                argv.iter().any(|a| a == "-s"),
                "-s must always be present: {argv:?}"
            );
        }
    }
}
