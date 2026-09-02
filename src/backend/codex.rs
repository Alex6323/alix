use anyhow::{Result, bail};
use serde_json::Value;

use super::{Backend, ProgressUpdate, PromptDelivery, RunOpts};

pub struct CodexBackend;

impl Backend for CodexBackend {
    fn command(&self) -> &str {
        "codex"
    }

    fn build_argv(&self, opts: &RunOpts) -> Vec<String> {
        // --ask-for-approval must precede `exec` (codex rejects it afterward).
        // The sandbox, not opts.access, governs tool access, so it's unused here.
        // A zero cap is what stops AGENTS.md from shaping a reply alix parses
        // strictly; `--ignore-user-config` does not, it only skips config.toml.
        let mut argv = vec![
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "exec".to_string(),
            "--sandbox".to_string(),
            "read-only".to_string(),
            "-c".to_string(),
            "project_doc_max_bytes=0".to_string(),
        ];
        if opts.progress {
            argv.push("--json".to_string());
        }

        // Codex has no --permission-mode equivalent, so it is dropped.
        if let Some(model) = opts.model {
            argv.push("-m".to_string());
            argv.push(model.to_string());
        }
        if let Some(effort) = opts.effort {
            argv.push("-c".to_string());
            argv.push(format!("model_reasoning_effort={effort}"));
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

    fn extract_progress(&self, stdout: &str) -> Result<String> {
        let mut saw_stream_event = false;
        let mut answer = None;
        for line in stdout.lines() {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(event_type) = codex_event_type(&event) else {
                continue;
            };
            saw_stream_event = true;
            if event_type == "item.completed"
                && event.pointer("/item/type").and_then(Value::as_str) == Some("agent_message")
                && let Some(text) = event.pointer("/item/text").and_then(Value::as_str)
            {
                answer = Some(text.trim().to_string());
            }
        }
        if let Some(answer) = answer {
            return Ok(answer);
        }
        if saw_stream_event {
            bail!("Codex's progress stream ended without a final agent message");
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
                message: activity.then(|| "Codex: producing a response...".to_string()),
                model: None,
            };
        };
        let Some(event_type) = codex_event_type(&event) else {
            return ProgressUpdate {
                activity: true,
                message: Some("Codex: producing a response...".to_string()),
                model: None,
            };
        };
        let message = match event_type {
            "thread.started" | "turn.started" => Some("Codex: started.".to_string()),
            "item.started"
                if event.pointer("/item/type").and_then(Value::as_str)
                    == Some("command_execution") =>
            {
                Some("Codex: reading the source...".to_string())
            }
            "item.started" => Some("Codex: working...".to_string()),
            "item.completed"
                if event.pointer("/item/type").and_then(Value::as_str) == Some("agent_message") =>
            {
                Some("Codex: drafted the response.".to_string())
            }
            "turn.completed" => Some("Codex: finished.".to_string()),
            _ => None,
        };
        ProgressUpdate {
            activity: true,
            message,
            model: None,
        }
    }

    fn can_fetch_web(&self) -> bool {
        false
    }

    fn required_help_flags(&self) -> &'static [&'static str] {
        &["exec", "--sandbox", "--ask-for-approval", "--json", "-c"]
    }

    fn name(&self) -> &'static str {
        "codex"
    }
}

fn codex_event_type(event: &Value) -> Option<&str> {
    let event_type = event.get("type").and_then(Value::as_str)?;
    (event_type.starts_with("thread.")
        || event_type.starts_with("turn.")
        || event_type.starts_with("item.")
        || event_type == "error")
        .then_some(event_type)
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
            progress: false,
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
            effort: None,
            permission_mode: Some("dontAsk"), // Claude-only, must be dropped
            access: Access::None,
            session_args: &[],
            progress: false,
        });
        assert_flag_value(&argv, "-m", "gpt-5");
        assert!(!argv.iter().any(|a| a.contains("model_reasoning_effort")));
        assert!(
            !argv
                .iter()
                .any(|a| a == "--permission-mode" || a == "dontAsk")
        );
    }

    #[test]
    fn codex_effort_maps_to_the_reasoning_config_key() {
        let argv = CodexBackend.build_argv(&RunOpts {
            model: None,
            effort: Some("minimal"),
            permission_mode: None,
            access: Access::None,
            session_args: &[],
            progress: false,
        });
        assert_flag_value(&argv, "-c", "project_doc_max_bytes=0");
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=minimal"),
            "effort must reach codex as its reasoning config: {argv:?}"
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
        assert!(flags.contains(&"--json"));
    }

    #[test]
    fn codex_progress_uses_json_events_and_extracts_the_agent_message() {
        let argv = CodexBackend.build_argv(&RunOpts {
            model: None,
            effort: None,
            permission_mode: None,
            access: Access::None,
            session_args: &[],
            progress: true,
        });
        assert!(argv.iter().any(|a| a == "--json"), "argv: {argv:?}");

        let event =
            r#"{"type":"item.started","item":{"type":"command_execution","command":"rg source"}}"#;
        let update = CodexBackend.progress_update(event);
        assert!(update.activity);
        assert_eq!(
            Some("Codex: reading the source..."),
            update.message.as_deref()
        );

        let stream = concat!(
            "{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"## Q\\nA\\n\"}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{}}\n",
        );
        assert_eq!("## Q\nA", CodexBackend.extract_progress(stream).unwrap());
    }

    #[test]
    fn codex_plain_json_answer_is_not_mistaken_for_a_progress_stream() {
        for answer in [
            r#"{"0":["point one","point two"]}"#,
            r#"{"type":"deck","cards":[]}"#,
        ] {
            assert_eq!(
                answer,
                CodexBackend.extract_progress(answer).unwrap(),
                "answer: {answer}"
            );
        }
    }

    #[test]
    fn codex_progress_events_map_to_calm_statuses() {
        for (event, message) in [
            (
                r#"{"type":"thread.started","thread_id":"t"}"#,
                Some("Codex: started."),
            ),
            (
                r#"{"type":"item.started","item":{"type":"command_execution","command":"rg source"}}"#,
                Some("Codex: reading the source..."),
            ),
            (
                r#"{"type":"item.started","item":{"type":"web_search","query":"source"}}"#,
                Some("Codex: working..."),
            ),
            (
                r#"{"type":"item.completed","item":{"type":"agent_message","text":"secret draft"}}"#,
                Some("Codex: drafted the response."),
            ),
            (
                r#"{"type":"item.completed","item":{"type":"command_execution","output":"secret source"}}"#,
                None,
            ),
            (
                r#"{"type":"turn.completed","usage":{}}"#,
                Some("Codex: finished."),
            ),
            (
                "malformed but active",
                Some("Codex: producing a response..."),
            ),
        ] {
            let update = CodexBackend.progress_update(event);
            assert!(update.activity, "event: {event}");
            assert_eq!(message, update.message.as_deref(), "event: {event}");
        }
        assert_eq!(
            ProgressUpdate::default(),
            CodexBackend.progress_update(" \n")
        );
        assert!(CodexBackend.structured_progress());
    }
}
