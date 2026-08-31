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
fn deck_from(source_text: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.md");
    std::fs::write(&source, source_text).unwrap();

    let cfg = GenerateDeckConfig {
        card_style: GenerateCardStyle::Mixed,
        ..Default::default()
    };
    let ask_cfg = AskConfig::default();
    let spec = GenerationSpec::from_config("learn this material", &cfg);

    generate::generate_deck(source.to_str().unwrap(), &cfg, &ask_cfg, &spec)
        .expect("the generator returned no deck")
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
    let deck =
        generate::generate_deck(source.to_str().unwrap(), &cfg, &AskConfig::default(), &spec)
            .expect("the generator returned no deck");

    let cards = alix::parser::parse_str("generated.md", &deck).expect("the generated deck parses");
    assert!(
        cards.iter().any(alix::card::Card::is_blank_card),
        "an explicit cloze style must produce span-derived cards, got:\n{deck}"
    );
    assert!(
        !deck.contains("\\blank{"),
        "the retired marker must not appear, got:\n{deck}"
    );
}
