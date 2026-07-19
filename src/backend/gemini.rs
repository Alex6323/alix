//! The Gemini CLI backend (`gemini -p`, headless). Only read tools are
//! allowlisted, so a write/shell tool errors instead of hanging on a
//! confirmation prompt that never comes.

use anyhow::Result;

use super::{Access, Backend, PromptDelivery, RunOpts};

const FILE_TOOLS: &[&str] = &[
    "read_file",
    "read_many_files",
    "list_directory",
    "glob",
    "search_file_content",
];

pub struct GeminiBackend;

impl Backend for GeminiBackend {
    fn command(&self) -> &str {
        "gemini"
    }

    fn build_argv(&self, opts: &RunOpts) -> Vec<String> {
        let mut argv = vec!["-p".to_string()];

        let mut tools: Vec<&str> = Vec::new();
        if let Access::ReadOnly {
            files,
            fetch,
            search,
        } = opts.access
        {
            if files {
                tools.extend(FILE_TOOLS);
            }
            if fetch {
                tools.push("web_fetch");
            }
            if search {
                tools.push("google_web_search");
            }
        }
        for tool in tools {
            argv.push("--allowed-tools".to_string());
            argv.push(tool.to_string());
        }

        // Gemini has no `--effort` equivalent, so effort is dropped here.
        if let Some(model) = opts.model {
            argv.push("--model".to_string());
            argv.push(model.to_string());
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

    fn can_fetch_web(&self) -> bool {
        true
    }

    fn required_help_flags(&self) -> &'static [&'static str] {
        &["-p", "--allowed-tools", "--model"]
    }

    fn name(&self) -> &'static str {
        "gemini"
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
    fn gemini_read_only_grant_flags() {
        let argv = GeminiBackend.build_argv(&opts(
            Access::ReadOnly {
                files: true,
                fetch: true,
                search: true,
            },
            &[],
        ));
        assert_eq!(argv[0], "-p");
        for tool in FILE_TOOLS {
            assert!(
                argv.iter().any(|a| a == tool),
                "read tool {tool} should be allowlisted"
            );
        }
        assert!(argv.iter().any(|a| a == "web_fetch"));
        assert!(argv.iter().any(|a| a == "google_web_search"));
        assert!(argv.iter().any(|a| a == "--allowed-tools"));
        assert!(!argv.iter().any(|a| a == "--yolo" || a == "-y"));
        assert!(!argv.iter().any(|a| a == "--approval-mode"));
    }

    #[test]
    fn gemini_fetch_without_search_omits_web_search() {
        let argv = GeminiBackend.build_argv(&opts(
            Access::ReadOnly {
                files: true,
                fetch: true,
                search: false,
            },
            &[],
        ));
        assert!(argv.iter().any(|a| a == "web_fetch"));
        assert!(!argv.iter().any(|a| a == "google_web_search"));
        assert!(argv.iter().any(|a| a == "read_file"));
    }

    #[test]
    fn gemini_no_grant_omits_tool_flags() {
        let argv = GeminiBackend.build_argv(&opts(Access::None, &[]));
        assert!(!argv.iter().any(|a| a == "--allowed-tools"));
        assert!(!argv.iter().any(|a| a == "web_fetch"));
        assert_eq!(vec!["-p"], argv);
    }

    #[test]
    fn gemini_model_flag() {
        let argv = GeminiBackend.build_argv(&RunOpts {
            model: Some("gemini-2.5-pro"),
            effort: Some("high"),
            permission_mode: Some("dontAsk"),
            access: Access::None,
            session_args: &[],
        });
        let model_at = argv
            .iter()
            .position(|a| a == "--model")
            .expect("model flag present");
        assert_eq!(argv[model_at + 1], "gemini-2.5-pro");
        assert!(!argv.iter().any(|a| a == "--effort" || a == "high"));
        assert!(
            !argv
                .iter()
                .any(|a| a == "--permission-mode" || a == "dontAsk")
        );
    }

    #[test]
    fn gemini_extract_trims_output() {
        assert_eq!(
            "the answer",
            GeminiBackend.extract("  the answer\n").unwrap()
        );
    }

    #[test]
    fn gemini_caps_and_help_flags() {
        assert!(GeminiBackend.can_fetch_web());
        let flags = GeminiBackend.required_help_flags();
        assert!(flags.contains(&"-p"));
        assert!(flags.contains(&"--allowed-tools"));
    }
}
