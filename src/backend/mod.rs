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
}

pub trait Backend: Send + Sync {
    fn command(&self) -> &str;

    fn build_argv(&self, opts: &RunOpts) -> Vec<String>;

    fn prompt_delivery(&self) -> PromptDelivery;

    fn extract(&self, stdout: &str) -> anyhow::Result<String>;

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
}

pub fn backend_for(cfg: &AskConfig) -> anyhow::Result<Box<dyn Backend>> {
    match cfg.backend {
        BackendKind::Claude => Ok(Box::new(ClaudeBackend)),
        BackendKind::Gemini => Ok(Box::new(GeminiBackend)),
        BackendKind::Codex => Ok(Box::new(CodexBackend)),
        BackendKind::Copilot => Ok(Box::new(CopilotBackend)),
    }
}

pub fn ensure_source_reachable(cfg: &AskConfig, is_url: bool) -> anyhow::Result<()> {
    let backend = backend_for(cfg)?;
    if is_url && !backend.can_fetch_web() {
        anyhow::bail!(
            "the {} backend can't fetch a url under read-only — point % source: at a local file, \
             or use a backend that can fetch",
            backend.name()
        );
    }
    if !is_url && !backend.can_read_source() {
        anyhow::bail!(
            "the {} backend can't read a local source — point % source: at a url, or use a backend \
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
