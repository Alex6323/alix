use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};

use crate::{ask, config::AskConfig, deck::Deck, workspace::Workspace};

pub fn generate(dir: &Path, guidance: Option<&str>, ask_cfg: &AskConfig) -> Result<PathBuf> {
    let ws = Workspace::load(dir).context("loading the workspace to ground its icon")?;
    let topics = member_topics(&ws);
    let prompt = build_prompt(
        &ws.display_name(),
        ws.description.as_deref().unwrap_or(""),
        &topics,
        guidance,
    );
    let run_cfg = icon_run_config(ask_cfg);
    let svg = draw(&run_cfg, &prompt).or_else(|_| draw(&run_cfg, &prompt))?;
    let out = dir.join("assets").join("icon.svg");
    clear_existing_icons(dir);
    write_atomic(&out, svg.as_bytes())?;
    Ok(out)
}

pub fn install(dir: &Path, src: &Path) -> Result<PathBuf> {
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    let out = dir.join("assets").join(format!("icon.{ext}"));
    clear_existing_icons(dir);
    if ext == "svg" {
        let raw = fs::read_to_string(src).with_context(|| format!("reading {}", src.display()))?;
        let svg =
            sanitize_svg(&raw).ok_or_else(|| anyhow!("{} is not a usable svg", src.display()))?;
        write_atomic(&out, svg.as_bytes())?;
    } else {
        let bytes = fs::read(src).with_context(|| format!("reading {}", src.display()))?;
        write_atomic(&out, &bytes)?;
    }
    Ok(out)
}

fn member_topics(ws: &Workspace) -> Vec<String> {
    ws.members
        .iter()
        .filter_map(|m| Deck::load(m).ok())
        .map(|d| d.trace.clone().unwrap_or_else(|| d.subject.clone()))
        .collect()
}

fn build_prompt(
    title: &str,
    description: &str,
    topics: &[String],
    guidance: Option<&str>,
) -> String {
    let topics = if topics.is_empty() {
        String::new()
    } else {
        format!("\nIts decks and traces cover: {}.", topics.join("; "))
    };
    let topics = match guidance {
        Some(g) => format!("{topics}\nStyle guidance from the user: {}.", g.trim()),
        None => topics,
    };
    format!(
        "Design one flat, abstract SVG emblem representing the subject of a study \
         workspace, for use as a small icon.\n\n\
         Workspace: \"{title}\"\n\
         What it is for: {description}{topics}\n\n\
         Requirements:\n\
         - Output ONLY the SVG markup — no prose, no code fence.\n\
         - A single <svg> root with viewBox=\"0 0 24 24\" and no width/height.\n\
         - Abstract and emblematic, not a literal picture, and no text or letters.\n\
         - Monochrome: use currentColor for every fill/stroke and a transparent \
         background (it is tinted by the theme).\n\
         - Keep it SMALL and cheap to draw: at most 6 shapes, and PREFER primitives \
         (<circle>, <rect>, <line>, <polygon>) over <path> — a short path of a few \
         points is fine, but never dozens of bezier coordinates.\n\
         - Use whole or half-integer coordinates on the 24 grid; no long decimals.\n\
         - No <script>, event handlers, external references, gradients, filters, or \
         embedded raster images."
    )
}

fn draw(run_cfg: &AskConfig, prompt: &str) -> Result<String> {
    let raw = ask::run(run_cfg, prompt, &[])?;
    sanitize_svg(&raw).ok_or_else(|| anyhow!("the model returned no usable svg"))
}

fn icon_run_config(ask_cfg: &AskConfig) -> AskConfig {
    AskConfig {
        allowed_tools: Vec::new(),
        source_access: false,
        ..ask_cfg.clone()
    }
}

fn clear_existing_icons(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir.join("assets")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_stem().is_some_and(|s| s == "icon") {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Defense in depth only: icons render in a non-executing context (a CSS
/// mask or an `<img>`), which is what actually prevents script execution.
pub fn sanitize_svg(raw: &str) -> Option<String> {
    if !raw.to_ascii_lowercase().contains("<svg") {
        return None;
    }
    let cleaned = strip_attrs(&remove_blocks(
        &remove_blocks(raw, "script"),
        "foreignObject",
    ));
    let lower = cleaned.to_ascii_lowercase();
    let start = lower.find("<svg")?;
    let end = lower.rfind("</svg>")? + "</svg>".len();
    Some(cleaned[start..end].trim().to_string())
}

fn remove_blocks(s: &str, tag: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let tag = tag.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        if lower[i..].starts_with(&open) {
            match lower[i..].find(&close) {
                Some(rel) => {
                    i += rel + close.len();
                    continue;
                }
                None => break, // unterminated block: drop the remainder
            }
        }
        let ch = match s[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn strip_attrs(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        if matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
            let rest = &lower[i + 1..];
            let drop = rest.starts_with("on")
                || rest.starts_with("href")
                || rest.starts_with("xlink:href");
            if drop && let Some(eq) = lower[i..].find('=') {
                let mut j = i + eq + 1;
                while j < s.len() && bytes[j] == b' ' {
                    j += 1;
                }
                if j < s.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                    let q = bytes[j];
                    j += 1;
                    while j < s.len() && bytes[j] != q {
                        j += 1;
                    }
                    if j < s.len() {
                        j += 1;
                    }
                }
                i = j;
                continue;
            }
        }
        let ch = match s[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{ask_config, exec_lock, fake_cli, fake_reply};

    fn write_workspace(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(
            dir.join("alix.toml"),
            "title = \"Light Client\"\ndescription = \"understand the source\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("a.md"), "# Sync protocol\n## q\na\n").unwrap();
    }

    #[test]
    fn sanitize_svg_strips_scripts_handlers_and_links_but_keeps_shapes() {
        let raw = r#"prose before
            <svg viewBox="0 0 24 24" onload="steal()">
              <script>alert(1)</script>
              <a href="https://evil.example"><circle cx="12" cy="12" r="8"/></a>
              <foreignObject><iframe src="x"></iframe></foreignObject>
            </svg> trailing"#;
        let out = sanitize_svg(raw).expect("has an <svg> root");
        let lower = out.to_ascii_lowercase();
        assert!(out.starts_with("<svg"), "trimmed to the svg span: {out}");
        assert!(out.ends_with("</svg>"));
        assert!(lower.contains("<circle"), "keeps benign shapes");
        assert!(!lower.contains("<script"));
        assert!(!lower.contains("onload"));
        assert!(!lower.contains("href"));
        assert!(!lower.contains("foreignobject"));
    }

    #[test]
    fn sanitize_svg_rejects_non_svg() {
        assert_eq!(sanitize_svg("just text, no markup"), None);
    }

    #[test]
    fn generate_writes_a_sanitized_icon() {
        let _lock = exec_lock();
        let ws = tempfile::tempdir().unwrap();
        write_workspace(ws.path());
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            cli_dir.path(),
            "<svg viewBox=\"0 0 24 24\"><script>x()</script><circle r=\"8\"/></svg>",
        );

        let out = generate(ws.path(), None, &ask_config(&cli)).unwrap();

        assert_eq!(out, ws.path().join("assets").join("icon.svg"));
        let svg = std::fs::read_to_string(&out).unwrap();
        assert!(svg.contains("<circle"));
        assert!(!svg.to_ascii_lowercase().contains("<script"));
    }

    #[test]
    fn generate_errors_when_the_model_returns_no_svg() {
        let _lock = exec_lock();
        let ws = tempfile::tempdir().unwrap();
        write_workspace(ws.path());
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(cli_dir.path(), "sorry, I can't draw that");

        let err = generate(ws.path(), None, &ask_config(&cli)).unwrap_err();
        assert!(err.to_string().contains("no usable svg"));
        assert!(!ws.path().join("assets").join("icon.svg").exists());
    }

    #[test]
    fn build_prompt_bounds_the_output_for_a_fast_draw() {
        let p = build_prompt(
            "Rust",
            "ownership and borrowing",
            &["moves".to_string()],
            None,
        );
        assert!(p.contains("at most 6 shapes"));
        assert!(p.contains("PREFER primitives"));
    }

    #[test]
    fn build_prompt_carries_the_user_steer_only_when_given() {
        let steered = build_prompt("Rust", "", &[], Some(" a compass rose "));
        assert!(steered.contains("Style guidance from the user: a compass rose."));
        let plain = build_prompt("Rust", "", &[], None);
        assert!(!plain.contains("Style guidance"));
    }

    #[test]
    fn generate_retries_once_after_a_failed_draw() {
        let _lock = exec_lock();
        let ws = tempfile::tempdir().unwrap();
        write_workspace(ws.path());
        let cli_dir = tempfile::tempdir().unwrap();
        // Drains stdin first to avoid the broken-pipe race.
        let c = cli_dir.path().join("n");
        let cli = fake_cli(
            cli_dir.path(),
            &format!(
                "cat >/dev/null; n=$(cat {c} 2>/dev/null || echo 0); echo $((n+1)) > {c}; \
                 [ \"$n\" = 0 ] && exit 1; echo '<svg viewBox=\"0 0 24 24\"><circle r=\"8\"/></svg>'",
                c = c.display()
            ),
        );

        let out = generate(ws.path(), None, &ask_config(&cli)).unwrap();
        let svg = std::fs::read_to_string(&out).unwrap();
        assert!(svg.contains("<circle"));
    }

    #[test]
    fn install_copies_a_raster_as_is_and_sanitizes_an_svg() {
        let ws = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();

        let png = src.path().join("logo.png");
        std::fs::write(&png, b"\x89PNG raw bytes").unwrap();
        let out = install(ws.path(), &png).unwrap();
        assert_eq!(out, ws.path().join("assets").join("icon.png"));
        assert_eq!(std::fs::read(&out).unwrap(), b"\x89PNG raw bytes");

        let svg = src.path().join("mark.svg");
        std::fs::write(
            &svg,
            "<svg viewBox=\"0 0 24 24\"><script>x</script><rect/></svg>",
        )
        .unwrap();
        let out = install(ws.path(), &svg).unwrap();
        assert_eq!(out, ws.path().join("assets").join("icon.svg"));
        let body = std::fs::read_to_string(&out).unwrap();
        assert!(body.contains("<rect"));
        assert!(!body.to_ascii_lowercase().contains("<script"));
        assert!(!ws.path().join("assets").join("icon.png").exists());
    }
}
