use anyhow::{Result, bail};
use serde_json::Value;

use super::{Access, Backend, ProgressUpdate, PromptDelivery, RunOpts};

pub struct ClaudeBackend;

impl Backend for ClaudeBackend {
    fn command(&self) -> &str {
        "claude"
    }

    fn build_argv(&self, opts: &RunOpts) -> Vec<String> {
        let output_format = if opts.progress { "stream-json" } else { "text" };
        let mut argv = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            output_format.to_string(),
        ];
        if opts.progress {
            argv.push("--verbose".to_string());
            argv.push("--include-partial-messages".to_string());
        }

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

    fn extract_progress(&self, stdout: &str) -> Result<String> {
        let mut saw_stream_event = false;
        for line in stdout.lines() {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(event_type) = claude_event_type(&event) else {
                continue;
            };
            saw_stream_event = true;
            if event_type != "result" {
                continue;
            }
            let result = event
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if event.get("is_error").and_then(Value::as_bool) == Some(true)
                || event.get("subtype").and_then(Value::as_str) != Some("success")
            {
                bail!("Claude ended without a usable result: {}", result.trim());
            }
            return Ok(result.trim().to_string());
        }
        if saw_stream_event {
            bail!("Claude's progress stream ended without a result");
        }
        self.extract(stdout)
    }

    fn structured_progress(&self) -> bool {
        true
    }

    fn progress_update(&self, line: &str) -> ProgressUpdate {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            let activity = !line.trim().is_empty();
            return ProgressUpdate {
                activity,
                message: activity.then(|| "Claude: producing a response...".to_string()),
            };
        };
        let Some(event_type) = claude_event_type(&event) else {
            return ProgressUpdate {
                activity: true,
                message: Some("Claude: producing a response...".to_string()),
            };
        };
        let message = match event_type {
            "system" if event.get("subtype").and_then(Value::as_str) == Some("init") => {
                Some("Claude: started.".to_string())
            }
            "assistant" => claude_assistant_progress(&event),
            "user" if contains_content_type(&event, "tool_result") => {
                Some("Claude: tool finished.".to_string())
            }
            "result" => Some("Claude: finished.".to_string()),
            _ => None,
        };
        ProgressUpdate {
            activity: true,
            message,
        }
    }

    fn required_help_flags(&self) -> &'static [&'static str] {
        &[
            "-p",
            "--allowedTools",
            "--permission-mode",
            "--output-format",
            "--verbose",
            "--include-partial-messages",
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

fn claude_event_type(event: &Value) -> Option<&str> {
    let event_type = event.get("type").and_then(Value::as_str)?;
    matches!(
        event_type,
        "system" | "assistant" | "user" | "result" | "stream_event" | "rate_limit_event"
    )
    .then_some(event_type)
}

fn claude_assistant_progress(event: &Value) -> Option<String> {
    let content = event
        .pointer("/message/content")
        .and_then(Value::as_array)?;
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let message = match block.get("name").and_then(Value::as_str).unwrap_or("tool") {
            "WebFetch" => "Claude: fetching the source...",
            "WebSearch" => "Claude: searching the web...",
            "Read" => "Claude: reading source files...",
            "Glob" | "Grep" => "Claude: exploring source files...",
            _ => "Claude: using a tool...",
        };
        return Some(message.to_string());
    }
    if content
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("text"))
    {
        return Some("Claude: drafting the response...".to_string());
    }
    if content
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("thinking"))
    {
        return Some("Claude: reasoning...".to_string());
    }
    None
}

fn contains_content_type(event: &Value, expected: &str) -> bool {
    event
        .pointer("/message/content")
        .and_then(Value::as_array)
        .is_some_and(|content| {
            content
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some(expected))
        })
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
            progress: false,
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
            progress: false,
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
            progress: false,
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
            progress: false,
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
            progress: false,
        });
        assert!(!argv.iter().any(|a| a == "--permission-mode"));
    }

    #[test]
    fn claude_progress_uses_stream_json_and_extracts_the_result() {
        let argv = ClaudeBackend.build_argv(&RunOpts {
            model: None,
            effort: None,
            permission_mode: Some("dontAsk"),
            access: Access::ReadOnly {
                files: false,
                fetch: true,
                search: false,
            },
            session_args: &[],
            progress: true,
        });
        assert!(
            argv.windows(2)
                .any(|w| w == ["--output-format", "stream-json"])
        );
        assert!(argv.iter().any(|a| a == "--verbose"));
        assert!(argv.iter().any(|a| a == "--include-partial-messages"));

        let tool = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"WebFetch","input":{"url":"https://example.org"}}]}}"#;
        let update = ClaudeBackend.progress_update(tool);
        assert!(update.activity);
        assert_eq!(
            Some("Claude: fetching the source..."),
            update.message.as_deref()
        );

        let stream = concat!(
            "{\"type\":\"system\",\"subtype\":\"init\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"draft\"}]}}\n",
            "{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"## Q\\nA\\n\"}\n",
        );
        assert_eq!("## Q\nA", ClaudeBackend.extract_progress(stream).unwrap());
    }

    #[test]
    fn claude_plain_json_answer_is_not_mistaken_for_a_progress_stream() {
        for answer in [
            r#"{"0":["point one","point two"]}"#,
            r#"{"type":"deck","cards":[]}"#,
        ] {
            assert_eq!(
                answer,
                ClaudeBackend.extract_progress(answer).unwrap(),
                "answer: {answer}"
            );
        }
    }

    #[test]
    fn claude_rejects_each_independent_result_failure_signal() {
        for event in [
            r#"{"type":"result","subtype":"error_max_turns","is_error":false,"result":"partial"}"#,
            r#"{"type":"result","subtype":"success","is_error":true,"result":"partial"}"#,
        ] {
            assert!(
                ClaudeBackend.extract_progress(event).is_err(),
                "event should fail: {event}"
            );
        }
    }

    #[test]
    fn claude_progress_events_map_to_calm_statuses() {
        for (event, activity, message) in [
            (
                r#"{"type":"system","subtype":"init"}"#,
                true,
                Some("Claude: started."),
            ),
            (r#"{"type":"system","subtype":"status"}"#, true, None),
            (
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
                true,
                Some("Claude: reading source files..."),
            ),
            (
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"WebSearch"}]}}"#,
                true,
                Some("Claude: searching the web..."),
            ),
            (
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Glob"}]}}"#,
                true,
                Some("Claude: exploring source files..."),
            ),
            (
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"secret draft"}]}}"#,
                true,
                Some("Claude: drafting the response..."),
            ),
            (
                r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"secret reasoning"}]}}"#,
                true,
                Some("Claude: reasoning..."),
            ),
            (
                r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"secret result"}]}}"#,
                true,
                Some("Claude: tool finished."),
            ),
            (
                r#"{"type":"user","message":{"content":[{"type":"text","text":"not a tool result"}]}}"#,
                true,
                None,
            ),
            (
                r#"{"type":"result","subtype":"success","result":"secret deck"}"#,
                true,
                Some("Claude: finished."),
            ),
            (
                "malformed but active",
                true,
                Some("Claude: producing a response..."),
            ),
            (" \n", false, None),
        ] {
            let update = ClaudeBackend.progress_update(event);
            assert_eq!(activity, update.activity, "event: {event}");
            assert_eq!(message, update.message.as_deref(), "event: {event}");
        }
        assert!(ClaudeBackend.structured_progress());
    }

    #[test]
    fn claude_help_contract_covers_every_progress_flag() {
        assert_eq!(
            &[
                "-p",
                "--allowedTools",
                "--permission-mode",
                "--output-format",
                "--verbose",
                "--include-partial-messages",
            ],
            ClaudeBackend.required_help_flags()
        );
    }
}
