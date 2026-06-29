//! Per-workspace icons: an abstract emblem the picker shows next to a workspace,
//! drawn by the model at `--build` or supplied by the user. Generation is
//! best-effort — a failure never blocks a build.

/// Sanitize an SVG: remove `<script>`/`<foreignObject>` blocks, `on*`/`href`/
/// `xlink:href` attributes, and trim to the `<svg>…</svg>` span. Returns `None`
/// when the input has no `<svg` root.
///
/// This is defense in depth only — icons render in a non-executing context (a
/// CSS mask or an `<img>`), which is what actually prevents script execution.
pub fn sanitize_svg(raw: &str) -> Option<String> {
    if !raw.to_ascii_lowercase().contains("<svg") {
        return None;
    }
    let cleaned = strip_attrs(&remove_blocks(&remove_blocks(raw, "script"), "foreignObject"));
    let lower = cleaned.to_ascii_lowercase();
    let start = lower.find("<svg")?;
    let end = lower.rfind("</svg>")? + "</svg>".len();
    Some(cleaned[start..end].trim().to_string())
}

/// Remove every `<tag …>…</tag>` block (case-insensitive). `tag` is ASCII.
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
                None => break, // unterminated block — drop the remainder
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

/// Remove `on*`, `href`, and `xlink:href` attributes (with their quoted values)
/// from `s`. Conservative and approximate: it only fires at an attribute
/// boundary (after whitespace), which is enough since the real guard is the
/// render context.
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
                        j += 1; // consume the closing quote
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
}
