//! Card-shape evaluation: does the generator actually FOLLOW the rule?
//!
//! `docs/include/card-shapes.md` says which shape suits which material, and
//! `src/generate.rs` builds its prompt from that file. A deterministic test
//! can prove the rule reached the prompt. Only a real model call can prove
//! the rule steers, and a prompt that reads well while steering badly is the
//! exact failure this catches.
//!
//! Every test here is `#[ignore]`d, so `cargo test` and CI compile them and
//! run none. Run them deliberately with `make shape-eval`: it needs the
//! `claude` CLI installed and logged in, and makes real, costed calls. It
//! joins `make calibrate` and `make docs-audit` as a release gate, never CI.
//!
//! Two rules keep these robust against a model's nondeterminism, borrowed
//! from the calibration harness. Sources are unambiguous rather than
//! borderline: a vocabulary list is paired material by any reading, and a
//! procedure is ordered by any reading. And each probe asserts the ONE
//! structural property its row promises, never the wording, the card count,
//! or the choice of examples.
//!
//! A failure here is not a code bug. It means the rule stopped steering, and
//! either the rule's wording or the prompt around it needs work.

use alix::{
    config::{AskConfig, GenerateCardStyle, GenerateDeckConfig},
    generate::{self, GenerationSpec},
};

/// Write a source file into a temp dir and generate a deck from it.
fn src_root(dir: &std::path::Path) -> alix::cite::GeneratedRoot {
    alix::cite::GeneratedRoot {
        key: "src".into(),
        path_prefix: String::new(),
        declare: Some(dir.display().to_string()),
    }
}

fn generate_mixed(
    source: &std::path::Path,
    root_dir: &std::path::Path,
    diagrams: bool,
) -> anyhow::Result<alix::generate::DeckReply> {
    let cfg = GenerateDeckConfig {
        card_style: GenerateCardStyle::Mixed,
        ..Default::default()
    };
    let spec = GenerationSpec {
        diagrams,
        ..GenerationSpec::from_config("learn this material", &cfg)
    };
    generate::generate_deck(
        source.to_str().unwrap(),
        Some(&src_root(root_dir)),
        &cfg,
        &AskConfig::default(),
        &spec,
    )
}

fn deck_from(source_text: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.md");
    std::fs::write(&source, source_text).unwrap();
    generate_mixed(&source, dir.path(), true)
        .expect("the generator returned no deck")
        .text
}

const VOCABULARY: &str = "\
German verbs of arguing:
befürworten means to advocate.
einräumen means to concede.
widerlegen means to refute.
unterstellen means to allege.
bestätigen means to corroborate.
abtun means to dismiss.
";

const PROCEDURE: &str = "\
Opening a TCP connection, in order:
1. The client sends a SYN segment.
2. The server replies with SYN-ACK.
3. The client sends an ACK.
Only then may either side send data.
";

const SET_MEMBERSHIP: &str = "\
The eight planets of the solar system and their kind:
Mercury, Venus, Earth and Mars are rocky planets.
Jupiter and Saturn are gas giants.
Uranus and Neptune are ice giants.
Only Earth is known to have liquid water on its surface.
";

/// Paired material must become a card table. This is the structural row with
/// the clearest payoff: a table's rows sample their own column for Recognize
/// options, which no other shape gets for free.
#[test]
#[ignore = "costed: real model call, run via `make shape-eval`"]
fn paired_vocabulary_generates_a_card_table() {
    let deck = deck_from(VOCABULARY);
    let delimiter = deck
        .lines()
        .any(|line| line.trim_start().starts_with('|') && line.contains("---"));
    assert!(
        delimiter,
        "a vocabulary source must generate a card table, got:\n{deck}"
    );
}

/// Ordered steps must become a line reveal, so order is graded.
#[test]
#[ignore = "costed: real model call, run via `make shape-eval`"]
fn an_ordered_procedure_generates_a_line_reveal() {
    let deck = deck_from(PROCEDURE);
    assert!(
        deck.contains("reveal: line"),
        "an ordered procedure must generate a line reveal, got:\n{deck}"
    );
}

/// The overrides still pin one shape for a whole run, which is what makes
/// them useful to a learner who wants uniformity. A rule that steers per
/// card must not quietly overrule an explicit instruction.
#[test]
#[ignore = "costed: real model call, run via `make shape-eval`"]
fn an_explicit_style_still_overrules_the_rule() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.md");
    std::fs::write(&source, VOCABULARY).unwrap();

    let cfg = GenerateDeckConfig {
        card_style: GenerateCardStyle::Cloze,
        ..Default::default()
    };
    let spec = GenerationSpec::from_config("learn this material", &cfg);
    let deck = generate::generate_deck(
        source.to_str().unwrap(),
        Some(&src_root(dir.path())),
        &cfg,
        &AskConfig::default(),
        &spec,
    )
    .expect("the generator returned no deck")
    .text;

    let cards = alix::parser::parse_str("generated.md", &deck).expect("the generated deck parses");
    assert!(
        cards.iter().any(alix::card::Card::is_blank_card),
        "an explicit cloze style must produce span-derived cards, got:\n{deck}"
    );
    assert!(
        !deck.contains("\\blank{"),
        "a cloze deck hides text with span directives, never inline marker syntax, got:\n{deck}"
    );
}

/// Several members sharing a property must become a select-all card, so each
/// option is judged on its own instead of stopping at the first match.
#[test]
#[ignore = "costed: real model call, run via `make shape-eval`"]
fn a_set_with_several_members_generates_a_select_all() {
    let deck = deck_from(SET_MEMBERSHIP);
    assert!(
        deck.contains("choices: multiple"),
        "a set with several members sharing a property must generate a select-all card, got:\n{deck}"
    );
}

mod census {
    #[derive(Debug, Default, PartialEq, Eq)]
    pub struct Census {
        pub parses: bool,
        pub cards: usize,
        pub plain: usize,
        pub cloze: usize,
        pub choice_single: usize,
        pub choice_multiple: usize,
        pub table_rows: usize,
        pub line_reveal: usize,
        pub draw: usize,
        pub both_directions: usize,
        pub mermaid_fences: usize,
        pub at_lines: usize,
        pub task_lists_without_choices: usize,
        pub unbadged_closing_quotes: usize,
        pub sections_without_prose: usize,
        pub frontmatter_keys: Vec<String>,
    }

    pub fn census(deck: &str) -> Census {
        let mut counts = Census {
            mermaid_fences: deck
                .lines()
                .filter(|line| line.trim_start().starts_with("```mermaid"))
                .count(),
            at_lines: deck
                .lines()
                .filter(|line| line.trim_start().starts_with("<!-- at:"))
                .count(),
            task_lists_without_choices: task_lists_without_choices(deck),
            frontmatter_keys: frontmatter_keys(deck),
            ..Census::default()
        };
        if let Ok(parsed) = alix::parser::parse("census.md", deck) {
            counts.parses = true;
            counts.sections_without_prose = parsed
                .topics
                .iter()
                .filter(|topic| {
                    topic.heading.is_some() && topic.prose.iter().all(|line| line.trim().is_empty())
                })
                .count();
            count_cards(&mut counts, &parsed.cards);
        }
        counts
    }

    fn count_cards(counts: &mut Census, cards: &[alix::card::Card]) {
        use alix::{answer::Input, card::Direction, depth::Reveal};

        counts.cards = cards.len();
        for card in cards {
            if card.is_blank_card() {
                counts.cloze += 1;
            } else if card.block_line != card.line {
                counts.table_rows += 1;
            } else if !card.authored_distractors.is_empty() && card.multiple_choice {
                counts.choice_multiple += 1;
            } else if !card.authored_distractors.is_empty() {
                counts.choice_single += 1;
            } else {
                counts.plain += 1;
            }
            counts.line_reveal += usize::from(card.reveal == Some(Reveal::Line));
            counts.draw += usize::from(card.input == Some(Input::Draw));
            counts.both_directions += usize::from(card.direction == Some(Direction::Both));
            counts.unbadged_closing_quotes += usize::from(
                card.back
                    .iter()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .is_some_and(|line| line.starts_with('>')),
            );
        }
    }

    fn is_task_item(line: &str) -> bool {
        [
            "- [ ]", "- [x]", "- [X]", "* [ ]", "* [x]", "* [X]", "+ [ ]", "+ [x]", "+ [X]",
        ]
        .iter()
        .any(|marker| line.starts_with(marker))
    }

    fn task_lists_without_choices(deck: &str) -> usize {
        let lines: Vec<&str> = deck.lines().collect();
        let mut count = 0;
        let mut in_list = false;
        for (index, line) in lines.iter().enumerate() {
            let item = is_task_item(line);
            if in_list && !item {
                count += usize::from(!line.trim_start().starts_with("<!-- choices:"));
            }
            in_list = item;
            if item && index + 1 == lines.len() {
                count += 1;
            }
        }
        count
    }

    fn frontmatter_keys(deck: &str) -> Vec<String> {
        let mut lines = deck.lines().skip_while(|line| line.trim().is_empty());
        if lines.next().map(str::trim_end) != Some("---") {
            return Vec::new();
        }
        lines
            .take_while(|line| line.trim_end() != "---")
            .filter(|line| !line.starts_with(char::is_whitespace))
            .filter_map(|line| line.split_once(':'))
            .map(|(key, _)| key.to_string())
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const ONE_OF_EACH: &str = "\
---
title: Kinds
---
# Section
Some prose for the section.

## Plain question
Plain answer.

## Cloze card
The value is dropped.
<!-- blank: span hidden=\"dropped\" -->

## Pick one
- [ ] a
- [x] b
<!-- choices: single -->

## Pick all
- [x] a
- [x] b
- [ ] c
<!-- choices: multiple -->

## Steps
first
second
<!-- reveal: line -->

## Draw it
a shape
<!-- input: draw -->

## Both ways
x
<!-- direction: both -->

| word | meaning |
|------|---------|
| a    | b       |
| c    | d       |
<!-- cards -->
";

        #[test]
        fn a_deck_with_one_card_of_each_kind_is_counted_per_kind() {
            let counts = census(ONE_OF_EACH);
            assert!(counts.parses, "the fixture deck must parse");
            assert_eq!(
                (1 + 3 + 1 + 1 + 1 + 2, 4, 1, 1, 1, 2, 1, 1, 1),
                (
                    counts.cards,
                    counts.plain,
                    counts.cloze,
                    counts.choice_single,
                    counts.choice_multiple,
                    counts.table_rows,
                    counts.line_reveal,
                    counts.draw,
                    counts.both_directions
                ),
                "cards, plain, cloze, single, multiple, rows, line, draw, both: {counts:?}"
            );
        }

        #[test]
        fn a_task_list_without_its_own_choices_line_is_counted_even_when_the_frontmatter_says_choices_single()
         {
            let deck = "\
---
choices: single
---
## Mapped
- [ ] a
- [x] b
<!-- choices: single -->

## Bare
- [ ] a
- [x] b

## Bare again
- [x] a
- [ ] b
Closing line.
";
            assert_eq!(2, census(deck).task_lists_without_choices);
        }

        #[test]
        fn mermaid_fences_and_at_lines_are_counted() {
            let deck = "\
## Flow
```mermaid
graph TD
  a --> b
```
<!-- at: src:a.md:1-3 fingerprint: 028t5cy4tqkff -->

## Other
Answer.
<!-- at: src:a.md:4-6 fingerprint: 028t5cy4tqkff -->
";
            let counts = census(deck);
            assert_eq!(
                (1, 2),
                (counts.mermaid_fences, counts.at_lines),
                "{counts:?}"
            );
        }

        #[test]
        fn an_unbadged_blockquote_closing_an_answer_is_counted() {
            let deck = "\
## Quoted
Answer.
> a quotation

## Quoted before a locator
Answer.
> another quotation
<!-- at: src:a.md:1-3 fingerprint: 028t5cy4tqkff -->

## Noted
Answer.
> [!NOTE]
> a note

## Quote not closing
Answer.
> a quotation
More answer.
";
            let counts = census(deck);
            assert!(counts.parses, "the fixture deck must parse");
            assert_eq!(2, counts.unbadged_closing_quotes, "{counts:?}");
        }

        #[test]
        fn a_section_without_prose_is_counted() {
            let deck = "\
# Bare heading

## Question one
Answer.

# Heading with prose
This section explains the cards below it.

## Question two
Answer.
";
            let counts = census(deck);
            assert!(counts.parses, "the fixture deck must parse");
            assert_eq!(1, counts.sections_without_prose, "{counts:?}");
        }

        #[test]
        fn frontmatter_keys_are_listed() {
            let deck = "\
---
title: A deck
source: /somewhere
reveal: line
---
## Q
A.
";
            assert_eq!(
                vec!["title", "source", "reveal"],
                census(deck).frontmatter_keys
            );
        }

        #[test]
        fn a_deck_that_does_not_parse_reports_parse_false() {
            let deck = "\
---
title: Broken
---
## Question without an answer
<!-- at: src:a.md:1-3 fingerprint: 028t5cy4tqkff -->
```mermaid
graph TD
```
";
            let counts = census(deck);
            assert!(!counts.parses, "{counts:?}");
            assert_eq!(0, counts.cards);
            assert_eq!(1, counts.mermaid_fences);
            assert_eq!(1, counts.at_lines);
            assert_eq!(vec!["title"], counts.frontmatter_keys);
        }
    }
}

const CENSUS_RUNS: usize = 3;

fn run_census(fixture_name: &str) {
    let arm = std::env::var("SHAPE_CENSUS_ARM").unwrap_or_default();
    assert!(
        !arm.is_empty(),
        "set SHAPE_CENSUS_ARM to the arm's name (for example `baseline` or `merged`) before running the census"
    );
    let home = std::env::var("HOME").expect("HOME is set");
    let date = chrono::Local::now().format("%Y-%m-%d");
    let stem = fixture_name.trim_end_matches(".md");
    let dir = std::path::Path::new(&home)
        .join("tmp")
        .join(format!("shape-census-{date}"))
        .join(&arm)
        .join(stem);
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/shape-eval")
        .join(fixture_name);
    let source = dir.join(fixture_name);
    std::fs::copy(&fixture, &source).unwrap();

    let root = src_root(&dir);
    let runs: Vec<Result<String, String>> = (0..CENSUS_RUNS)
        .map(|_| {
            let reply =
                generate_mixed(&source, &dir, true).map_err(|error| format!("{error:#}"))?;
            generate::declare_root(&reply.text, &root).map_err(|error| format!("{error:#}"))
        })
        .collect();

    println!(
        "census of {fixture_name}, arm {arm}, placed under {}",
        dir.display()
    );
    println!(
        "run | result | parses | cards | plain | cloze | single | multiple | rows | line | draw | both | mermaid | at | bare lists | closing quotes | bare sections | sources list | frontmatter keys"
    );
    for (index, run) in runs.iter().enumerate() {
        let number = index + 1;
        match run {
            Ok(deck) => {
                let c = census::census(deck);
                println!(
                    "{number} | generated | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | n/a | {}",
                    c.parses,
                    c.cards,
                    c.plain,
                    c.cloze,
                    c.choice_single,
                    c.choice_multiple,
                    c.table_rows,
                    c.line_reveal,
                    c.draw,
                    c.both_directions,
                    c.mermaid_fences,
                    c.at_lines,
                    c.task_lists_without_choices,
                    c.unbadged_closing_quotes,
                    c.sections_without_prose,
                    c.frontmatter_keys.join(",")
                );
                std::fs::write(dir.join(format!("run-{number}.md")), deck).unwrap();
            }
            Err(error) => println!("{number} | error: {error}"),
        }
    }

    let mut failures = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        match run {
            Ok(deck) if !census::census(deck).parses => {
                failures.push(format!("run {} does not parse", index + 1));
            }
            Ok(_) => {}
            Err(error) => failures.push(format!("run {} failed: {error}", index + 1)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "costed: real model calls, run via `make shape-eval`"]
fn the_conceptual_fixture_census_prints_its_table() {
    run_census("conceptual.md");
}

#[test]
#[ignore = "costed: real model calls, run via `make shape-eval`"]
fn the_procedure_fixture_census_prints_its_table() {
    run_census("procedure.md");
}
