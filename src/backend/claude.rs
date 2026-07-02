//! The Claude Code CLI backend (`claude -p`). Reproduces alix's original,
//! only invocation verbatim: the same fixed `-p --output-format text` prefix,
//! the same `--allowedTools`/`--permission-mode`/`--model`/`--effort` flags,
//! prompt on stdin, answer trimmed off stdout.

use anyhow::{Result, bail};

use super::{Access, Backend, PromptDelivery, RunOpts};

/// The Claude Code CLI backend.
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

        // Render the abstract grant into Claude's tool names in a fixed
        // canonical order, so equivalent grants always produce identical argv.
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
            // `dontAsk` silently denies any tool outside the allowlist instead
            // of hanging on an approval that `-p` mode cannot receive.
            argv.push("--permission-mode".to_string());
            argv.push("dontAsk".to_string());
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
        let answer = stdout.trim().to_string();
        if answer.is_empty() {
            bail!("the assistant returned an empty answer");
        }
        Ok(answer)
    }

    fn required_help_flags(&self) -> &'static [&'static str] {
        &[
            "-p",
            "--allowedTools",
            "--permission-mode",
            "--output-format",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts<'a>(access: Access, session_args: &'a [String]) -> RunOpts<'a> {
        RunOpts {
            model: None,
            effort: None,
            access,
            session_args,
        }
    }

    #[test]
    fn claude_grant_maps_to_canonical_flags() {
        let argv = ClaudeBackend.build_argv(&RunOpts {
            model: Some("opus"),
            effort: Some("high"),
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
        // The trace case: source files + fetch a known URL, but no web search.
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
}
