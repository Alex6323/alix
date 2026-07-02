//! The GitHub Copilot CLI backend (`copilot -p`, headless). Prompt is passed
//! as the argument to `-p` (delivery [`Arg`](PromptDelivery::Arg) — put `-p`
//! last in argv so `ask::run` appends the prompt text immediately after it).
//! Tool access uses `--deny-tool` to block destructive capabilities and
//! `--available-tools` to restrict to the read/fetch subset.
//!
//! Flags and tool names verified against the GitHub Copilot CLI docs on
//! 2026-07-02:
//! - headless `-p PROMPT`, `-s` (silent/clean output), `--model`: <https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference>
//! - tool names (`shell`, `write`, `read`, `url`, `memory`) and
//!   `--deny-tool`/`--allow-tool`/`--available-tools`/`--excluded-tools`:
//!   <https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-programmatic-reference>
//! - read-only via `--deny-tool='shell,write'` (permits `read` + `url`): <https://docs.github.com/en/copilot/how-tos/copilot-cli/automate-copilot-cli/run-cli-programmatically>
//!
//! These names drift; a nightly `--help` check lands in Task 9.

use anyhow::Result;

use super::{Access, Backend, PromptDelivery, RunOpts};

/// The GitHub Copilot CLI backend.
pub struct CopilotBackend;

impl Backend for CopilotBackend {
    fn command(&self) -> &str {
        "copilot"
    }

    fn build_argv(&self, opts: &RunOpts) -> Vec<String> {
        // `-s` suppresses session statistics and decoration so stdout carries
        // only the agent's response — required for programmatic extraction.
        let mut argv = vec!["-s".to_string()];

        // Deny shell and write in every call — nothing destructive runs
        // headless. This applies even when Access::None: the deny list stops
        // Copilot from accidentally running shell or file-write tools if the
        // model decides to try them despite a text-only prompt.
        argv.push("--deny-tool".to_string());
        argv.push("shell,write".to_string());

        // When the grant requests read-only access, restrict available tools
        // to the read/fetch/search subset using `--available-tools`. Without
        // this flag all non-denied tools remain available; with it only the
        // listed ones are offered to the model.
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
                // `url` covers both URL fetching and web search in Copilot's
                // tool model — there is no separate search tool name.
                available.push("url");
            }
            if !available.is_empty() {
                argv.push("--available-tools".to_string());
                argv.push(available.join(","));
            }
        }

        // `--model=MODEL` is supported (docs: "Specify the AI model").
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
        // `-p PROMPT` — the prompt is the value of the `-p` flag; `ask::run`
        // appends the prompt string after build_argv, which ends with `-p`.
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
        &["-p", "--deny-tool", "--available-tools"]
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

    /// Returns the value that immediately follows `flag` in `argv`, or panics.
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
        // Full read-only grant: files + fetch + search.
        // argv must contain `-p` (headless), `--deny-tool` with shell+write,
        // and `--available-tools` restricting to read+url.
        let argv = CopilotBackend.build_argv(&opts(
            Access::ReadOnly {
                files: true,
                fetch: true,
                search: true,
            },
            &[],
        ));

        // Headless flag present and last (so ask::run appends prompt after it).
        assert!(argv.iter().any(|a| a == "-p"), "must contain -p: {argv:?}");
        assert_eq!(argv.last().unwrap(), "-p", "-p must be last: {argv:?}");

        // Destructive tools are always denied.
        let denied = flag_value(&argv, "--deny-tool");
        assert!(
            denied.contains("shell"),
            "--deny-tool must include shell: {denied}"
        );
        assert!(
            denied.contains("write"),
            "--deny-tool must include write: {denied}"
        );

        // Available tools are restricted to the read/url subset.
        let available = flag_value(&argv, "--available-tools");
        assert!(
            available.contains("read"),
            "--available-tools must include read: {available}"
        );
        assert!(
            available.contains("url"),
            "--available-tools must include url for fetch+search: {available}"
        );

        // No auto-approval flags that could bypass the deny list.
        assert!(
            !argv.iter().any(|a| a == "--allow-all-tools"),
            "must not use --allow-all-tools: {argv:?}"
        );
    }

    #[test]
    fn copilot_files_only_grant_omits_url() {
        // files=true, fetch=false, search=false → available=read only.
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
        // fetch=true, files=false → url is in available, read is not.
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
        // Access::None → no --available-tools, but shell+write still denied.
        let argv = CopilotBackend.build_argv(&opts(Access::None, &[]));
        assert!(
            !argv.iter().any(|a| a == "--available-tools"),
            "--available-tools must be absent with no grant: {argv:?}"
        );
        let denied = flag_value(&argv, "--deny-tool");
        assert!(denied.contains("shell"));
        assert!(denied.contains("write"));
        // Still ends with -p.
        assert_eq!(argv.last().unwrap(), "-p");
    }

    #[test]
    fn copilot_model_flag_uses_equals_form() {
        let argv = CopilotBackend.build_argv(&RunOpts {
            model: Some("claude-sonnet-4-6"),
            effort: Some("high"), // no Copilot equivalent — must be dropped
            permission_mode: Some("x"), // Claude-only — must be dropped
            access: Access::None,
            session_args: &[],
        });
        assert!(
            argv.iter().any(|a| a == "--model=claude-sonnet-4-6"),
            "--model= form must be present: {argv:?}"
        );
        // Effort and permission-mode have no Copilot flag.
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
        // `-s` suppresses decoration; must appear for all grant levels.
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
