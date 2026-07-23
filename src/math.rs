use std::collections::{HashMap, HashSet};

use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_svg::{SvgOptions, render_to_svg};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    augment::AugmentCache,
    card::Card,
    inline::{DisplayProjector, InlineRun},
    parser::{BLANK, HIDDEN},
    render::NoteUnit,
    review::CardView,
};

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

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("card at line {line}: malformed LaTeX math in {surface} `{formula}`: {dependency_error}")]
pub struct MathDiagnostic {
    pub line: usize,
    pub surface: &'static str,
    pub formula: String,
    pub dependency_error: String,
}

#[derive(Default)]
pub(crate) struct MathRenderer {
    cache: HashMap<String, Result<String, String>>,
    #[cfg(test)]
    render_count: usize,
}

impl MathRenderer {
    pub(crate) fn view(&mut self, source: &str, display: bool, context: bool) -> MathView {
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

pub fn diagnostics(cards: &[Card], augment: Option<&AugmentCache>) -> Vec<MathDiagnostic> {
    let mut projector = DisplayProjector::default();
    let mut diagnostics = Vec::new();
    let mut seen = HashSet::new();
    for card in cards {
        let view = CardView::project(card, &mut projector);
        collect_run_diagnostics(
            card.line,
            "front",
            &view.front_runs,
            &mut seen,
            &mut diagnostics,
        );
        for runs in &view.context_runs {
            collect_run_diagnostics(card.line, "context", runs, &mut seen, &mut diagnostics);
        }
        for runs in &view.back_runs {
            collect_run_diagnostics(card.line, "answer", runs, &mut seen, &mut diagnostics);
        }
        for unit in &view.note {
            match unit {
                NoteUnit::Sentence { runs, .. } => {
                    collect_run_diagnostics(card.line, "note", runs, &mut seen, &mut diagnostics)
                }
                NoteUnit::Checklist { items } => {
                    for item in items {
                        collect_run_diagnostics(
                            card.line,
                            "note checklist",
                            &item.runs,
                            &mut seen,
                            &mut diagnostics,
                        );
                    }
                }
                NoteUnit::Code { .. } => {}
            }
        }
        for distractor in &card.authored_distractors {
            let runs = projector.project(distractor);
            collect_run_diagnostics(card.line, "choice", &runs, &mut seen, &mut diagnostics);
        }
        let Some(augment) = augment else {
            continue;
        };
        let Some(id) = card.id() else {
            continue;
        };
        if let Some(distractors) = augment.distractors(&id, card.content_fingerprint) {
            for distractor in distractors {
                let runs = projector.project(distractor);
                collect_run_diagnostics(
                    card.line,
                    "generated choice",
                    &runs,
                    &mut seen,
                    &mut diagnostics,
                );
            }
        }
        if let Some(keypoints) = augment.keypoints(&id, card.content_fingerprint) {
            for keypoint in keypoints {
                let runs = projector.project(keypoint);
                collect_run_diagnostics(
                    card.line,
                    "generated keypoint",
                    &runs,
                    &mut seen,
                    &mut diagnostics,
                );
            }
        }
    }
    diagnostics
}

pub fn validate_generated(cards: &[Card]) -> Result<(), MathDiagnostic> {
    match diagnostics(cards, None).into_iter().next() {
        Some(diagnostic) => Err(diagnostic),
        None => Ok(()),
    }
}

fn collect_run_diagnostics(
    line: usize,
    surface: &'static str,
    runs: &[InlineRun],
    seen: &mut HashSet<(usize, String, String)>,
    diagnostics: &mut Vec<MathDiagnostic>,
) {
    for run in runs {
        let Some(math) = &run.math else {
            continue;
        };
        let Some(dependency_error) = &math.error else {
            continue;
        };
        let normalized = run.text.replace(BLANK, "<hole>").replace(HIDDEN, "<hole>");
        if !seen.insert((line, normalized, dependency_error.clone())) {
            continue;
        }
        diagnostics.push(MathDiagnostic {
            line,
            surface,
            formula: bounded_snippet(&run.text, 96),
            dependency_error: dependency_error.clone(),
        });
    }
}

fn bounded_snippet(source: &str, max_chars: usize) -> String {
    let mut chars = source.chars();
    let snippet: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{snippet}...")
    } else {
        snippet
    }
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
        assert_eq!(substituted, r"x = \underline{\hspace{2em}} + \cdots");
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

    #[test]
    fn validator_walks_authored_surfaces_and_ignores_literal_dollars_and_code() {
        let parsed = crate::parser::parse(
            "deck.md",
            "## Front $\\frac{1$\n\
             - [x] $\\sqrt{$\n\
             - [ ] $\\left($\n\
             > note $\\begin{pmatrix}$\n\
             > `$\\frac{1$`\n\
             > ```\n\
             > $\\frac{1$\n\
             > ```\n\
             \n\
             ## Literal prices\n\
             $5 and $10 with an unmatched $x\n",
        )
        .unwrap();

        let found = diagnostics(&parsed.cards, None);
        assert_eq!(4, found.len(), "{found:#?}");
        assert_eq!(
            vec!["front", "answer", "note", "choice"],
            found
                .iter()
                .map(|diagnostic| diagnostic.surface)
                .collect::<Vec<_>>()
        );
        assert!(found.iter().all(|diagnostic| diagnostic.line == 1));
    }

    #[test]
    fn validator_accepts_valid_math_on_every_authored_surface() {
        let parsed = crate::parser::parse(
            "deck.md",
            "## Front $x^2$\n\
             - [x] $x$\n\
             - [ ] $y$\n\
             > $$\\int_0^1 x\\,dx$$\n",
        )
        .unwrap();
        assert!(diagnostics(&parsed.cards, None).is_empty());
        assert!(validate_generated(&parsed.cards).is_ok());
    }

    #[test]
    fn validator_deduplicates_the_same_cloze_source_formula() {
        let parsed = crate::parser::parse(
            "deck.md",
            "## Complete it\n$x = \\blank{1} + \\blank{2} + \\frac{1$\n",
        )
        .unwrap();
        assert_eq!(2, parsed.cards.len());

        let found = diagnostics(&parsed.cards, None);
        assert_eq!(1, found.len(), "{found:#?}");
        assert_eq!("context", found[0].surface);
    }

    #[test]
    fn validator_includes_current_generated_choices_and_keypoints() {
        let dir = tempfile::tempdir().unwrap();
        let parsed = crate::parser::parse("deck.md", "## q <!-- id: mathdiag1 -->\na\n").unwrap();
        let card = &parsed.cards[0];
        let id = card.id().unwrap();
        let mut augment = AugmentCache::open(dir.path().join("augment.json"));
        augment.set_distractors(
            &id,
            vec![r"$\frac{1$".to_string()],
            card.content_fingerprint,
        );
        augment.set_keypoints(&id, vec![r"$\sqrt{$".to_string()], card.content_fingerprint);

        let found = diagnostics(&parsed.cards, Some(&augment));
        assert_eq!(2, found.len(), "{found:#?}");
        assert_eq!("generated choice", found[0].surface);
        assert_eq!("generated keypoint", found[1].surface);
    }

    #[test]
    fn malformed_authored_math_stays_loadable_with_an_error_run() {
        let parsed = crate::parser::parse("deck.md", "## q\n$\\frac{1$\n").unwrap();
        let view = CardView::from(&parsed.cards[0]);
        assert!(
            view.back_runs[0][0]
                .math
                .as_ref()
                .is_some_and(|math| math.svg.is_none() && math.error.is_some())
        );
        assert!(validate_generated(&parsed.cards).is_err());
    }

    #[test]
    fn diagnostic_snippets_are_bounded() {
        let source = "x".repeat(120);
        let snippet = bounded_snippet(&source, 16);
        assert_eq!("xxxxxxxxxxxxxxxx...", snippet);
    }
}
