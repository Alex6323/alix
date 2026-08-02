mod claude;
mod codex;
mod copilot;
mod gemini;
pub mod health;

pub use claude::ClaudeBackend;
pub use codex::CodexBackend;
pub use copilot::CopilotBackend;
pub use gemini::GeminiBackend;

use crate::config::{AskConfig, BackendKind};

pub enum PromptDelivery {
    Stdin,
    Arg,
    ExecArg,
}

pub enum Access {
    None,
    ReadOnly {
        files: bool,
        fetch: bool,
        search: bool,
    },
}

impl Access {
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

pub struct RunOpts<'a> {
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub permission_mode: Option<&'a str>,
    pub access: Access,
    pub session_args: &'a [String],
    pub progress: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProgressUpdate {
    pub activity: bool,
    pub message: Option<String>,
    /// The model the backend reports it actually loaded, when it says so.
    /// Discovered, never assumed: alix does not choose it and cannot name it
    /// from configuration alone.
    pub model: Option<String>,
}

pub trait Backend: Send + Sync {
    fn command(&self) -> &str;

    fn build_argv(&self, opts: &RunOpts) -> Vec<String>;

    fn prompt_delivery(&self) -> PromptDelivery;

    fn extract(&self, stdout: &str) -> anyhow::Result<String>;

    fn extract_progress(&self, stdout: &str) -> anyhow::Result<String> {
        self.extract(stdout)
    }

    fn structured_progress(&self) -> bool {
        false
    }

    fn progress_update(&self, line: &str) -> ProgressUpdate {
        ProgressUpdate {
            activity: !line.trim().is_empty(),
            message: None,
            model: None,
        }
    }

    fn agentic(&self) -> bool {
        true
    }

    fn can_fetch_web(&self) -> bool {
        true
    }

    fn can_read_source(&self) -> bool {
        true
    }

    fn supports_session(&self) -> bool {
        false
    }

    fn name(&self) -> &'static str;

    fn required_help_flags(&self) -> &'static [&'static str];

    fn default_trace_model(&self) -> Option<&'static str> {
        None
    }

    /// What the tutor pins when `[ask] model` is unset. Without a pin alix
    /// cannot name what answered, because the backend CLI chooses and never
    /// reports back.
    fn default_ask_model(&self) -> Option<&'static str> {
        None
    }

    fn default_ask_effort(&self) -> Option<&'static str> {
        None
    }
}

pub fn backend_for(cfg: &AskConfig) -> anyhow::Result<Box<dyn Backend>> {
    match cfg.backend {
        BackendKind::Claude => Ok(Box::new(ClaudeBackend)),
        BackendKind::Gemini => Ok(Box::new(GeminiBackend)),
        BackendKind::Codex => Ok(Box::new(CodexBackend)),
        BackendKind::Copilot => Ok(Box::new(CopilotBackend)),
    }
}

pub fn supports_structured_progress(cfg: &AskConfig) -> bool {
    backend_for(cfg).is_ok_and(|backend| backend.structured_progress())
}

/// The single resolution both the invocation and the readout use, so what the
/// tutor panel names is always what was actually passed.
pub fn resolved_ask_model(cfg: &AskConfig) -> Option<String> {
    cfg.model.clone().or_else(|| {
        backend_for(cfg)
            .ok()
            .and_then(|backend| backend.default_ask_model())
            .map(str::to_string)
    })
}

pub fn resolved_ask_effort(cfg: &AskConfig) -> Option<String> {
    cfg.effort.clone().or_else(|| {
        backend_for(cfg)
            .ok()
            .and_then(|backend| backend.default_ask_effort())
            .map(str::to_string)
    })
}

pub fn ensure_source_reachable(cfg: &AskConfig, is_url: bool) -> anyhow::Result<()> {
    let backend = backend_for(cfg)?;
    if is_url && !backend.can_fetch_web() {
        anyhow::bail!(
            "the {} backend can't fetch a url under read-only; point source: at a local file, \
             or use a backend that can fetch",
            backend.name()
        );
    }
    if !is_url && !backend.can_read_source() {
        anyhow::bail!(
            "the {} backend can't read a local source; point source: at a url, or use a backend \
             that can read files",
            backend.name()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AskConfig;

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn backend_for_wires_all_four_backends() {
        let mut cfg = AskConfig::default();
        assert!(backend_for(&cfg).is_ok(), "claude should be wired");

        cfg.backend = BackendKind::Gemini;
        assert!(backend_for(&cfg).is_ok(), "gemini should be wired");

        cfg.backend = BackendKind::Codex;
        assert!(backend_for(&cfg).is_ok(), "codex should be wired");

        cfg.backend = BackendKind::Copilot;
        assert!(backend_for(&cfg).is_ok(), "copilot should be wired");
    }

    #[test]
    fn codex_backend_refuses_a_url_source_cleanly() {
        let cfg = AskConfig {
            backend: BackendKind::Codex,
            ..AskConfig::default()
        };
        let err = ensure_source_reachable(&cfg, true).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("codex"), "{msg}");
        assert!(msg.contains("can't fetch"), "{msg}");
        assert!(ensure_source_reachable(&cfg, false).is_ok());
    }

    #[test]
    fn fetch_capable_backends_pass_the_url_gate() {
        for backend in [
            BackendKind::Claude,
            BackendKind::Gemini,
            BackendKind::Copilot,
        ] {
            let cfg = AskConfig {
                backend,
                ..AskConfig::default()
            };
            assert!(
                ensure_source_reachable(&cfg, true).is_ok(),
                "{backend:?} should pass the URL gate"
            );
            assert!(ensure_source_reachable(&cfg, false).is_ok());
        }
    }

    #[test]
    fn only_claude_supports_sessions() {
        assert!(ClaudeBackend.supports_session());
        assert!(!GeminiBackend.supports_session());
        assert!(!CodexBackend.supports_session());
        assert!(!CopilotBackend.supports_session());
    }

    #[test]
    fn a_configured_model_and_effort_resolve_verbatim_for_every_backend() {
        for backend in [
            BackendKind::Claude,
            BackendKind::Gemini,
            BackendKind::Codex,
            BackendKind::Copilot,
        ] {
            let cfg = AskConfig {
                backend,
                model: Some("pinned-model".to_string()),
                effort: Some("pinned-effort".to_string()),
                ..AskConfig::default()
            };
            assert_eq!(
                Some("pinned-model".to_string()),
                resolved_ask_model(&cfg),
                "{backend:?}"
            );
            assert_eq!(
                Some("pinned-effort".to_string()),
                resolved_ask_effort(&cfg),
                "{backend:?}"
            );
        }
    }

    #[test]
    fn without_configured_values_no_backend_invents_a_model_or_effort() {
        for backend in [
            BackendKind::Claude,
            BackendKind::Gemini,
            BackendKind::Codex,
            BackendKind::Copilot,
        ] {
            let cfg = AskConfig {
                backend,
                ..AskConfig::default()
            };
            assert_eq!(None, resolved_ask_model(&cfg), "{backend:?}");
            assert_eq!(None, resolved_ask_effort(&cfg), "{backend:?}");
        }
    }

    #[test]
    fn unstructured_backends_use_the_plain_progress_contract() {
        let backend = GeminiBackend;
        assert!(!backend.structured_progress());
        assert_eq!(
            "plain answer",
            backend.extract_progress("  plain answer\n").unwrap()
        );
        assert_eq!(
            ProgressUpdate {
                activity: true,
                message: None,
                model: None,
            },
            backend.progress_update("some output")
        );
        assert_eq!(ProgressUpdate::default(), backend.progress_update(" \n"));
    }

    #[test]
    fn structured_progress_capability_matches_the_backend_contract() {
        for (backend, expected) in [
            (BackendKind::Claude, true),
            (BackendKind::Codex, true),
            (BackendKind::Gemini, false),
            (BackendKind::Copilot, false),
        ] {
            let cfg = AskConfig {
                backend,
                ..AskConfig::default()
            };
            assert_eq!(expected, supports_structured_progress(&cfg), "{backend:?}");
        }
    }

    #[test]
    fn access_from_askconfig_maps_tools_to_grant() {
        let a = Access::from_allowed_tools(&tools(&["Read", "Glob", "Grep", "WebFetch"]));
        assert!(matches!(
            a,
            Access::ReadOnly {
                files: true,
                fetch: true,
                search: false
            }
        ));

        let b = Access::from_allowed_tools(&tools(&["WebFetch", "WebSearch"]));
        assert!(matches!(
            b,
            Access::ReadOnly {
                files: false,
                fetch: true,
                search: true
            }
        ));

        let c = Access::from_allowed_tools(&tools(&["Read"]));
        assert!(matches!(
            c,
            Access::ReadOnly {
                files: true,
                fetch: false,
                search: false
            }
        ));

        let d = Access::from_allowed_tools(&[]);
        assert!(matches!(d, Access::None));
    }
}
