use std::{env, fs, path::Path};

use alix::{card::Card, inline, parser};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct SpecExample {
    markdown: String,
    html: String,
    example: usize,
    start_line: usize,
    end_line: usize,
    section: String,
    #[serde(default)]
    extensions: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Measurement {
    corpus: String,
    example: usize,
    start_line: usize,
    end_line: usize,
    section: String,
    extensions: Vec<String>,
    disabled_by_reference_runner: bool,
    decision_groups: Vec<u8>,
    markdown: String,
    expected_html: String,
    parse_error: Option<String>,
    lints: Vec<String>,
    tables: usize,
    cards: Vec<CardMeasurement>,
    expected_back: Vec<String>,
    primary_back_exact: bool,
    primary_back_whitespace_equivalent: bool,
    candidate_reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CardMeasurement {
    front: String,
    back: Vec<String>,
    front_plain: String,
    back_plain: Vec<String>,
    note: Option<String>,
    section_context: Vec<String>,
    context: Vec<String>,
    authored_distractors: Vec<String>,
    images: Vec<ImageMeasurement>,
    images_back: Vec<ImageMeasurement>,
    input: Option<String>,
    reveal: Option<String>,
    line: usize,
    parent_block: Option<usize>,
    span_regions: usize,
    answer_fences: usize,
    view: alix::review::CardView,
}

#[derive(Debug, Serialize)]
struct ImageMeasurement {
    src: String,
    alt: Option<String>,
}

fn decision_groups(section: &str, markdown: &str) -> Vec<u8> {
    let mut groups = Vec::new();
    let lower = section.to_ascii_lowercase();
    let lines: Vec<_> = markdown.lines().map(str::trim).collect();

    if lower.contains("thematic break")
        || (lower.contains("setext") && lines.iter().any(|line| *line == "---"))
    {
        groups.push(1);
    }
    if lower.contains("block quote") {
        groups.push(12);
    }
    if lines.iter().any(|line| {
        matches!(
            *line,
            "> [!NOTE]" | "> [!TIP]" | "> [!IMPORTANT]" | "> [!WARNING]" | "> [!CAUTION]"
        )
    }) {
        groups.push(2);
    }
    if lower.contains("list") || lower.contains("indented code") {
        groups.extend([3, 12]);
    }
    if lower.contains("paragraph") || lower.contains("blank line") {
        groups.push(3);
    }
    if lower.contains("fenced code") {
        groups.push(4);
    }
    if lower.contains("backslash escape") || lower.contains("hard line break") {
        groups.push(5);
    }
    if markdown.contains("<!--") {
        groups.push(6);
    }
    if lower.contains("strikethrough") {
        groups.push(7);
    }
    if lower.contains("link") || lower.contains("autolink") {
        groups.push(8);
    }
    if lower.contains("table") {
        groups.push(9);
    }
    if lower.contains("html") || lower.contains("entity") || lower.contains("character reference") {
        groups.extend([10, 12]);
    }
    if lower.contains("math")
        || lines.iter().any(|line| {
            line.strip_prefix("```")
                .or_else(|| line.strip_prefix("~~~"))
                .is_some_and(|info| info.trim().eq_ignore_ascii_case("math"))
        })
        || markdown.contains("$`")
    {
        groups.push(11);
    }
    if lower.contains("atx heading") || lower.contains("setext") {
        groups.push(13);
    }
    if lower.contains("image") || markdown.lines().any(|line| line.trim() == "#") {
        groups.push(14);
    }
    if lines.iter().any(|line| {
        line.strip_prefix("```")
            .or_else(|| line.strip_prefix("~~~"))
            .is_some_and(|info| {
                matches!(
                    info.trim().to_ascii_lowercase().as_str(),
                    "geojson" | "topojson" | "stl"
                )
            })
    }) {
        groups.push(15);
    }

    groups.sort_unstable();
    groups.dedup();
    groups
}

fn image_measurements(images: &[alix::card::CardImage]) -> Vec<ImageMeasurement> {
    images
        .iter()
        .map(|image| ImageMeasurement {
            src: image.src.display().to_string(),
            alt: image.alt.clone(),
        })
        .collect()
}

fn card_measurement(card: &Card) -> CardMeasurement {
    CardMeasurement {
        front: card.front.clone(),
        back: card.back.clone(),
        front_plain: inline::strip_inline(&card.front),
        back_plain: card
            .back
            .iter()
            .map(|line| inline::strip_inline(line))
            .collect(),
        note: card.note.clone(),
        section_context: card.section_context.clone(),
        context: card.context.clone(),
        authored_distractors: card.authored_distractors.clone(),
        images: image_measurements(&card.images),
        images_back: image_measurements(&card.images_back),
        input: card.input.map(|input| format!("{input:?}")),
        reveal: card.reveal.map(|reveal| format!("{reveal:?}")),
        line: card.line,
        parent_block: card.parent_block,
        span_regions: card.span_regions.len(),
        answer_fences: card.answer_fences.len(),
        view: alix::review::CardView::from(card),
    }
}

fn expected_back(markdown: &str) -> Vec<String> {
    markdown
        .strip_suffix('\n')
        .unwrap_or(markdown)
        .split('\n')
        .map(ToOwned::to_owned)
        .collect()
}

fn without_whitespace(lines: &[String]) -> String {
    lines
        .iter()
        .flat_map(|line| line.chars())
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn measure(corpus: &str, example: SpecExample) -> Measurement {
    let groups = decision_groups(&example.section, &example.markdown);
    let disabled_by_reference_runner = example.extensions.iter().any(|value| value == "disabled");
    let extensions = example.extensions.clone();
    let expected = expected_back(&example.markdown);
    let wrapped = format!("## Corpus probe\n{}", example.markdown);

    match parser::parse("corpus.md", &wrapped) {
        Err(error) => Measurement {
            corpus: corpus.to_owned(),
            example: example.example,
            start_line: example.start_line,
            end_line: example.end_line,
            section: example.section,
            extensions,
            disabled_by_reference_runner,
            decision_groups: groups,
            markdown: example.markdown,
            expected_html: example.html,
            parse_error: Some(error.to_string()),
            lints: Vec::new(),
            tables: 0,
            cards: Vec::new(),
            expected_back: expected,
            primary_back_exact: false,
            primary_back_whitespace_equivalent: false,
            candidate_reasons: vec!["hard parse error".to_owned()],
        },
        Ok(deck) => {
            let cards: Vec<_> = deck.cards.iter().map(card_measurement).collect();
            let primary = cards.iter().find(|card| card.front == "Corpus probe");
            let primary_back_exact = primary.is_some_and(|card| card.back == expected);
            let primary_back_whitespace_equivalent = primary.is_some_and(|card| {
                without_whitespace(&card.back) == without_whitespace(&expected)
            });
            let mut reasons = Vec::new();

            if cards.len() != 1 {
                reasons.push(format!(
                    "wrapper produced {} cards instead of 1",
                    cards.len()
                ));
            }
            if primary.is_none() {
                reasons.push("wrapper card disappeared".to_owned());
            } else if !primary_back_exact && primary_back_whitespace_equivalent {
                reasons.push("wrapper answer whitespace changed".to_owned());
            } else if !primary_back_exact {
                reasons.push("wrapper answer non-whitespace content changed".to_owned());
            }
            if !deck.lints.is_empty() {
                reasons.push(format!("{} lint(s)", deck.lints.len()));
            }
            if !deck.tables.is_empty() {
                reasons.push(format!("{} card table(s)", deck.tables.len()));
            }
            if cards.iter().any(|card| card.note.is_some()) {
                reasons.push("blockquote content became an Alix note".to_owned());
            }
            if cards
                .iter()
                .any(|card| !card.authored_distractors.is_empty())
            {
                reasons.push("task-list content became authored choices".to_owned());
            }
            if cards
                .iter()
                .any(|card| !card.images.is_empty() || !card.images_back.is_empty())
            {
                reasons.push("image syntax became Alix media".to_owned());
            }
            if cards
                .iter()
                .any(|card| !card.section_context.is_empty() || card.parent_block.is_some())
            {
                reasons.push("heading content changed Alix hierarchy".to_owned());
            }

            Measurement {
                corpus: corpus.to_owned(),
                example: example.example,
                start_line: example.start_line,
                end_line: example.end_line,
                section: example.section,
                extensions,
                disabled_by_reference_runner,
                decision_groups: groups,
                markdown: example.markdown,
                expected_html: example.html,
                parse_error: None,
                lints: deck.lints.iter().map(|lint| format!("{lint:?}")).collect(),
                tables: deck.tables.len(),
                cards,
                expected_back: expected,
                primary_back_exact,
                primary_back_whitespace_equivalent,
                candidate_reasons: reasons,
            }
        }
    }
}

fn parse_spec_text(text: &str) -> Vec<SpecExample> {
    const FENCE: &str = "````````````````````````````````";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum State {
        Prose,
        Markdown,
        Html,
    }

    let mut state = State::Prose;
    let mut section = String::new();
    let mut markdown = String::new();
    let mut html = String::new();
    let mut extensions = Vec::new();
    let mut start_line = 0;
    let mut example = 0;
    let mut examples = Vec::new();

    for (line_index, line) in text.split_inclusive('\n').enumerate() {
        let line_number = line_index + 1;
        let stripped = line.trim();
        if let Some(suffix) = stripped.strip_prefix(&format!("{FENCE} example")) {
            state = State::Markdown;
            extensions = suffix.split_whitespace().map(ToOwned::to_owned).collect();
            continue;
        }
        if stripped == FENCE {
            assert_ne!(
                state,
                State::Prose,
                "unexpected example fence at line {line_number}"
            );
            example += 1;
            examples.push(SpecExample {
                markdown: std::mem::take(&mut markdown).replace('→', "\t"),
                html: std::mem::take(&mut html).replace('→', "\t"),
                example,
                start_line,
                end_line: line_number,
                section: section.clone(),
                extensions: std::mem::take(&mut extensions),
            });
            state = State::Prose;
            start_line = 0;
            continue;
        }
        if stripped == "." && state == State::Markdown {
            state = State::Html;
            continue;
        }
        match state {
            State::Markdown => {
                if start_line == 0 {
                    start_line = line_number - 1;
                }
                markdown.push_str(line);
            }
            State::Html => html.push_str(line),
            State::Prose => {
                let heading = line.trim_end().trim_start_matches('#');
                if line.starts_with('#') && heading.starts_with(' ') {
                    section = heading.trim().to_owned();
                }
            }
        }
    }

    assert_eq!(state, State::Prose, "unterminated example at end of spec");
    examples
}

fn load_examples(input: &Path) -> Vec<SpecExample> {
    let bytes = fs::read(input).unwrap_or_else(|error| panic!("read {}: {error}", input.display()));
    if input
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("parse {}: {error}", input.display()))
    } else {
        parse_spec_text(
            std::str::from_utf8(&bytes)
                .unwrap_or_else(|error| panic!("decode {}: {error}", input.display())),
        )
    }
}

/// One committed-baseline line per corpus example: only the fields whose
/// drift a parser change should surface, keys elided when empty so the
/// baseline stays compact and diffs stay readable.
#[derive(Debug, Serialize)]
struct DigestLine {
    e: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    g: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    err: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cards: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    back: Option<&'static str>,
}

fn digest_line(measurement: &Measurement) -> DigestLine {
    let parsed = measurement.parse_error.is_none();
    DigestLine {
        e: measurement.example,
        g: measurement.decision_groups.clone(),
        err: measurement.parse_error.clone(),
        lints: measurement.lints.clone(),
        cards: parsed.then_some(measurement.cards.len()),
        back: parsed.then(|| {
            if measurement.primary_back_exact {
                "exact"
            } else if measurement.primary_back_whitespace_equivalent {
                "ws"
            } else {
                "div"
            }
        }),
    }
}

const USAGE: &str = "usage: harness [--digest] CORPUS INPUT OUTPUT";

fn main() {
    let mut args: Vec<std::ffi::OsString> = env::args_os().skip(1).collect();
    let digest = args.first().is_some_and(|arg| arg == "--digest");
    if digest {
        args.remove(0);
    }
    let [corpus, input, output] = <[_; 3]>::try_from(args).expect(USAGE);

    let examples = load_examples(Path::new(&input));
    let corpus = corpus.to_string_lossy();
    let measurements: Vec<_> = examples
        .into_iter()
        .map(|example| measure(&corpus, example))
        .collect();
    let encoded = if digest {
        let mut lines = String::new();
        for measurement in &measurements {
            let line =
                serde_json::to_string(&digest_line(measurement)).expect("serialize digest line");
            lines.push_str(&line);
            lines.push('\n');
        }
        lines.into_bytes()
    } else {
        serde_json::to_vec_pretty(&measurements).expect("serialize measurements")
    };
    fs::write(&output, encoded)
        .unwrap_or_else(|error| panic!("write {}: {error}", Path::new(&output).display()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(markdown: &str, section: &str) -> SpecExample {
        SpecExample {
            markdown: markdown.to_owned(),
            html: String::new(),
            example: 1,
            start_line: 1,
            end_line: markdown.lines().count(),
            section: section.to_owned(),
            extensions: Vec::new(),
        }
    }

    #[test]
    fn digest_lines_keep_error_and_clean_examples_distinguishable() {
        let clean = measure("test", example("just an answer\n", "Paragraphs"));
        let line = digest_line(&clean);
        assert!(line.err.is_none(), "clean example: {line:?}");
        assert_eq!(Some(1), line.cards);
        assert_eq!(Some("exact"), line.back);

        let broken = measure("test", example("##### five\nbody\n", "ATX headings"));
        let line = digest_line(&broken);
        assert!(line.err.is_some(), "deep heading errors today: {line:?}");
        assert_eq!(None, line.cards);
        assert_eq!(None, line.back);
    }

    #[test]
    fn gfm_only_and_cross_cutting_sections_map_to_the_open_decisions() {
        assert_eq!(decision_groups("Paragraphs", "a\n\nb\n"), vec![3]);
        assert_eq!(
            decision_groups("Strikethrough (extension)", "~~a~~\n"),
            vec![7]
        );
        assert_eq!(decision_groups("Tables (extension)", "a | b\n"), vec![9]);
        assert_eq!(decision_groups("ATX headings", "#\n"), vec![13, 14]);
        assert_eq!(decision_groups("Block quotes", "> ordinary\n"), vec![12]);
        assert_eq!(
            decision_groups("Block quotes", "> [!WARNING]\n> body\n"),
            vec![2, 12]
        );
        assert_eq!(
            decision_groups("Fenced code blocks", "```\na\n```\n"),
            vec![4]
        );
        assert_eq!(
            decision_groups("Fenced code blocks", "```math\na\n```\n"),
            vec![4, 11]
        );
        assert_eq!(
            decision_groups("Fenced code blocks", "```geojson\na\n```\n"),
            vec![4, 15]
        );
    }

    #[test]
    fn multiline_setext_and_blank_surrounded_thematic_break_are_not_conflated() {
        // Re-pinned after the ruled `---` grammar landed on main: both shapes
        // now fail loudly, at distinct lines through distinct rules.
        let setext = measure("test", example("Foo\nBar\n---\n", "Setext headings"));
        let setext_error = setext.parse_error.as_deref().unwrap_or_default();
        assert!(
            setext_error.starts_with("line 4: this `---` neither divides"),
            "trailing --- takes the divider diagnosis: {setext_error:?}"
        );

        let thematic = measure(
            "test",
            example("Foo\nbar\n\n---\n\nbaz\n", "Setext headings"),
        );
        let thematic_error = thematic.parse_error.as_deref().unwrap_or_default();
        assert!(
            thematic_error.starts_with("line 7: prose after a `---` section terminator"),
            "blank-surrounded --- takes the terminator diagnosis: {thematic_error:?}"
        );
    }

    #[test]
    fn official_spec_text_shape_keeps_extensions_and_disabled_examples() {
        let spec = concat!(
            "# Prose\n\n",
            "```````````````````````````````` example table\n",
            "a | b\n",
            ".\n",
            "<table></table>\n",
            "````````````````````````````````\n\n",
            "## Tasks\n\n",
            "```````````````````````````````` example disabled\n",
            "- [ ] a\n",
            ".\n",
            "<ul></ul>\n",
            "````````````````````````````````\n",
        );
        let examples = parse_spec_text(spec);
        assert_eq!(examples.len(), 2);
        assert_eq!(examples[0].section, "Prose");
        assert_eq!(examples[0].extensions, ["table"]);
        assert_eq!(examples[1].section, "Tasks");
        assert_eq!(examples[1].extensions, ["disabled"]);
    }
}
