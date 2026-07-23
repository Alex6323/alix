use std::collections::HashMap;

use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_svg::{SvgOptions, render_to_svg};
use serde::{Deserialize, Serialize};

use crate::parser::{BLANK, HIDDEN};

#[cfg(test)]
thread_local! {
    static THREAD_RENDER_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MathView {
    pub display: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub svg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Default)]
pub struct MathRenderer {
    cache: HashMap<String, Result<String, String>>,
    #[cfg(test)]
    render_count: usize,
}

impl MathRenderer {
    pub fn view(&mut self, source: &str, display: bool, context: bool) -> MathView {
        let rendered_source = if context {
            substitute_context_holes(source)
        } else {
            source.to_string()
        };
        let rendered = if let Some(cached) = self.cache.get(&rendered_source) {
            cached.clone()
        } else {
            #[cfg(test)]
            {
                self.render_count += 1;
            }
            let rendered = render_svg(&rendered_source);
            self.cache.insert(rendered_source, rendered.clone());
            rendered
        };
        match rendered {
            Ok(svg) => MathView {
                display,
                svg: Some(svg),
                error: None,
            },
            Err(error) => MathView {
                display,
                svg: None,
                error: Some(error),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn render_count(&self) -> usize {
        self.render_count
    }
}

fn substitute_context_holes(source: &str) -> String {
    source
        .replace(BLANK, r"\underline{\hspace{2em}}")
        .replace(HIDDEN, r"\cdots")
}

fn render_svg(source: &str) -> Result<String, String> {
    #[cfg(test)]
    THREAD_RENDER_COUNT.with(|count| count.set(count.get() + 1));
    let ast = parse(source).map_err(|error| error.to_string())?;
    let layout_box = layout(&ast, &LayoutOptions::default());
    let display_list = to_display_list(&layout_box);
    let svg = render_to_svg(
        &display_list,
        &SvgOptions {
            embed_glyphs: true,
            ..SvgOptions::default()
        },
    );
    if svg_is_safe(&svg) {
        Ok(svg)
    } else {
        Err("ratex produced unsupported svg output".to_string())
    }
}

#[cfg(test)]
pub(crate) fn thread_render_count() -> usize {
    THREAD_RENDER_COUNT.with(std::cell::Cell::get)
}

fn svg_is_safe(svg: &str) -> bool {
    if !svg.starts_with("<svg") || !svg.contains("xmlns=\"http://www.w3.org/2000/svg\"") {
        return false;
    }
    let lower = svg.to_ascii_lowercase();
    let forbidden = [
        "<script",
        "<foreignobject",
        "<text",
        "<image",
        "font-family",
        "javascript:",
        "data:text/html",
        "url(",
        "href=",
    ];
    !forbidden.iter().any(|needle| lower.contains(needle)) && !has_event_attribute(&lower)
}

fn has_event_attribute(svg: &str) -> bool {
    svg.split_ascii_whitespace().any(|part| {
        let attribute = part.trim_start_matches('<');
        attribute.starts_with("on") && attribute.contains('=')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROADMAP_FIXTURES: [&str; 7] = [
        r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        r"\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}",
        r"\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}",
        r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
        r"\alpha_i^2 + \beta_j",
        r"\lim_{x \to 0} \frac{\sin x}{x} = 1",
        r"\nabla \times \mathbf{E} = -\frac{\partial \mathbf{B}}{\partial t}",
    ];

    #[test]
    fn roadmap_formulas_render_as_safe_standalone_path_svgs() {
        let mut renderer = MathRenderer::default();
        for source in ROADMAP_FIXTURES {
            let view = renderer.view(source, false, false);
            let svg = view.svg.as_ref().unwrap_or_else(|| {
                panic!(
                    "{source} failed: {}",
                    view.error.as_deref().unwrap_or_default()
                )
            });
            assert!(svg.starts_with("<svg"), "{source}");
            assert!(svg.contains("<path"), "{source}");
            assert!(svg_is_safe(svg), "{source}");
            assert!(!view.display);
            assert!(view.error.is_none());
        }
    }

    #[test]
    fn malformed_math_is_an_error_view_without_partial_svg() {
        let mut renderer = MathRenderer::default();
        let view = renderer.view(r"\frac{1", false, false);
        assert!(view.svg.is_none());
        assert!(view.error.is_some());
    }

    #[test]
    fn display_mode_is_view_metadata_not_a_second_render() {
        let mut renderer = MathRenderer::default();
        let inline = renderer.view("x^2", false, false);
        let display = renderer.view("x^2", true, false);
        assert!(!inline.display);
        assert!(display.display);
        assert_eq!(inline.svg, display.svg);
        assert_eq!(renderer.render_count(), 1);
    }

    #[test]
    fn repeated_errors_are_cached_too() {
        let mut renderer = MathRenderer::default();
        renderer.view(r"\frac{1", false, false);
        renderer.view(r"\frac{1", true, false);
        assert_eq!(renderer.render_count(), 1);
    }

    #[test]
    fn context_holes_render_without_source_answers() {
        let source = r"x = ____ + […]";
        let substituted = substitute_context_holes(source);
        assert_eq!(
            substituted,
            r"x = \underline{\hspace{2em}} + \cdots"
        );
        let mut renderer = MathRenderer::default();
        let view = renderer.view(source, false, true);
        assert!(view.svg.is_some(), "{}", view.error.unwrap_or_default());
        assert_eq!(renderer.render_count(), 1);
    }

    #[test]
    fn unsafe_svg_features_are_rejected() {
        let unsafe_fragments = [
            r#"<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text>x</text></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><path onload="x"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><use href="https://x"/></svg>"#,
        ];
        for svg in unsafe_fragments {
            assert!(!svg_is_safe(svg), "{svg}");
        }
        assert!(svg_is_safe(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#
        ));
    }
}
