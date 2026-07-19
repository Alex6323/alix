use anyhow::Result;

use super::{Access, Backend, PromptDelivery, RunOpts};

pub struct ClaudeBackend;

impl Backend for ClaudeBackend {
    fn command(&self) -> &str {
        "claude"
    }

    fn build_argv(&self, opts: &RunOpts) -> Vec<String> {
        let mut argv = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "text".to_string(),
        ];

        // Fixed canonical order: equivalent grants always produce identical argv.
        let mut tools: Vec<&str> = Vec::new();
        if let Access::ReadOnly {
            files,
            fetch,
            search,
        } = opts.access
        {
            if files {
                tools.extend(["Read", "Glob", "Grep"]);
            }
            if fetch {
                tools.push("WebFetch");
            }
            if search {
                tools.push("WebSearch");
            }
        }
        if !tools.is_empty() {
            argv.push("--allowedTools".to_string());
            argv.extend(tools.into_iter().map(String::from));
        }
        if let Some(mode) = opts.permission_mode {
            argv.push("--permission-mode".to_string());
            argv.push(mode.to_string());
        }

        if let Some(model) = opts.model {
            argv.push("--model".to_string());
            argv.push(model.to_string());
        }
        if let Some(effort) = opts.effort {
            argv.push("--effort".to_string());
            argv.push(effort.to_string());
        }
        argv.extend(opts.session_args.iter().cloned());
        argv
    }

    fn prompt_delivery(&self) -> PromptDelivery {
        PromptDelivery::Stdin
    }

    fn extract(&self, stdout: &str) -> Result<String> {
        Ok(stdout.trim().to_string())
    }

    fn required_help_flags(&self) -> &'static [&'static str] {
        &[
            "-p",
            "--allowedTools",
            "--permission-mode",
            "--output-format",
        ]
    }

    fn name(&self) -> &'static str {
        "claude"
    }

    fn supports_session(&self) -> bool {
        // Claude's `--session-id`/`--resume` give the tutor multi-turn memory.
        true
    }

    fn default_trace_model(&self) -> Option<&'static str> {
        // Trace building is agentic and correctness-critical, so Claude
        // defaults it to the strong model (other backends inherit the CLI default).
        Some("opus")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts<'a>(access: Access, session_args: &'a [String]) -> RunOpts<'a> {
        RunOpts {
            model: None,
            effort: None,
            permission_mode: None,
            access,
            session_args,
        }
    }

    #[test]
    fn claude_grant_maps_to_canonical_flags() {
        let argv = ClaudeBackend.build_argv(&RunOpts {
            model: Some("opus"),
            effort: Some("high"),
            permission_mode: Some("dontAsk"),
            access: Access::ReadOnly {
                files: true,
                fetch: true,
                search: true,
            },
            session_args: &[],
        });
        assert_eq!(
            vec![
                "-p",
                "--output-format",
                "text",
                "--allowedTools",
                "Read",
                "Glob",
                "Grep",
                "WebFetch",
                "WebSearch",
                "--permission-mode",
                "dontAsk",
                "--model",
                "opus",
                "--effort",
                "high",
            ],
            argv
        );
    }

    #[test]
    fn claude_fetch_without_search() {
        let argv = ClaudeBackend.build_argv(&opts(
            Access::ReadOnly {
                files: true,
                fetch: true,
                search: false,
            },
            &[],
        ));
        let tools_at = argv.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(
            vec!["--allowedTools", "Read", "Glob", "Grep", "WebFetch"],
            argv[tools_at..tools_at + 5]
        );
        assert!(!argv.iter().any(|a| a == "WebSearch"));
    }

    #[test]
    fn claude_no_grant_omits_allowedtools() {
        let argv = ClaudeBackend.build_argv(&opts(Access::None, &[]));
        assert!(!argv.iter().any(|a| a == "--allowedTools"));
        assert!(!argv.iter().any(|a| a == "--permission-mode"));
        assert_eq!(vec!["-p", "--output-format", "text"], argv);
    }

    #[test]
    fn claude_emits_permission_mode_independent_of_grant() {
        let argv = ClaudeBackend.build_argv(&RunOpts {
            model: None,
            effort: None,
            permission_mode: Some("dontAsk"),
            access: Access::None,
            session_args: &[],
        });
        assert!(!argv.iter().any(|a| a == "--allowedTools"));
        assert!(argv.iter().any(|a| a == "--permission-mode"));
        assert!(argv.iter().any(|a| a == "dontAsk"));

        let argv = ClaudeBackend.build_argv(&RunOpts {
            model: None,
            effort: None,
            permission_mode: Some("bypassPermissions"),
            access: Access::ReadOnly {
                files: true,
                fetch: false,
                search: false,
            },
            session_args: &[],
        });
        assert!(argv.iter().any(|a| a == "--allowedTools"));
        let pm_pos = argv.iter().position(|a| a == "--permission-mode").unwrap();
        assert_eq!(argv[pm_pos + 1], "bypassPermissions");

        let argv = ClaudeBackend.build_argv(&RunOpts {
            model: None,
            effort: None,
            permission_mode: None,
            access: Access::None,
            session_args: &[],
        });
        assert!(!argv.iter().any(|a| a == "--permission-mode"));
    }
}
