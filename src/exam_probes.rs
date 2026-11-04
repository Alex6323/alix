use serde::Deserialize;

use crate::{
    ask::{parse_json, run},
    config::{AskConfig, Config, ExamConfig, Strictness},
    deck::Deck,
    exam::{self, ExamQuestion, ExamSection},
};

const DECKS_VAR: &str = "ALIX_EXAM_PROBE_DECKS";

fn probe_decks() -> Vec<Deck> {
    let listed = std::env::var(DECKS_VAR).unwrap_or_else(|_| {
        panic!("set {DECKS_VAR} to a colon-separated list of initialized decks with `exam:` files")
    });
    listed
        .split(':')
        .filter(|path| !path.is_empty())
        .map(|path| exam::open_for_exam(std::path::Path::new(path)).unwrap())
        .collect()
}

fn configs() -> (ExamConfig, AskConfig) {
    let config = Config::load(None).unwrap();
    (config.exam, config.ask)
}

fn judge(ask: &AskConfig, prompt: &str) -> String {
    let judge = AskConfig {
        timeout_secs: 600,
        source_access: false,
        cwd: None,
        ..ask.clone()
    };
    run(&judge, prompt, &[]).unwrap()
}

fn card_list(section: &ExamSection) -> String {
    section
        .cards
        .iter()
        .map(|card| format!("- {}\n", card.replace('\n', "\n  ")))
        .collect()
}

fn answerability_is_within_bar(answerable: usize, total: usize) -> bool {
    answerable * 5 <= total
}

fn independent_source(deck: &Deck) -> String {
    let root = crate::workspace::content_root(&deck.path);
    let layers = deck.exam_layers();
    layers
        .own
        .iter()
        .chain(&layers.workspace)
        .filter(|entry| !crate::deck::is_url(entry))
        .map(|entry| {
            let text = crate::source::source_path(entry, Some(&root))
                .and_then(|path| std::fs::read_to_string(path).ok())
                .unwrap_or_default();
            format!("=== {entry} ===\n{text}\n")
        })
        .collect()
}

#[derive(Deserialize)]
struct Answerable {
    answerable: Vec<bool>,
}

#[test]
#[ignore = "costed: real examiner and judge calls over the decks in ALIX_EXAM_PROBE_DECKS"]
fn per_section_questions_are_not_answerable_from_an_existing_card() {
    let (exam_cfg, ask) = configs();
    let mut yes = 0;
    let mut total = 0;
    for deck in probe_decks() {
        let sections = exam::exam_sections(&deck);
        let questions = exam::generate_questions(&deck, &exam_cfg, &ask).unwrap();
        for (at, section) in sections.iter().enumerate() {
            let asked: Vec<&ExamQuestion> = questions.iter().filter(|q| q.section == at).collect();
            if asked.is_empty() {
                continue;
            }
            let listing: String = asked
                .iter()
                .enumerate()
                .map(|(i, q)| format!("Question {}: {}\n\n", i + 1, q.prompt))
                .collect();
            let reply = judge(
                &ask,
                &format!(
                    "Below are flashcards and exam questions about the same section. For each \
                     question, say whether a student could answer it by reciting one flashcard's \
                     answer (true) or whether it needs reasoning beyond any single card (false).\n\n\
                     Flashcards:\n{}\nQuestions:\n{listing}Output ONLY JSON: \
                     {{\"answerable\": [true|false, ...]}} with one entry per question, in order.",
                    card_list(section)
                ),
            );
            let verdicts: Answerable = parse_json(&reply).unwrap();
            assert_eq!(
                asked.len(),
                verdicts.answerable.len(),
                "one verdict per question"
            );
            let section_yes = verdicts.answerable.iter().filter(|v| **v).count();
            println!(
                "probe: deck={} section={:?} questions={} answerable_from_a_card={section_yes}",
                deck.display_name(),
                section.label,
                asked.len()
            );
            yes += section_yes;
            total += asked.len();
        }
    }
    println!("probe: answerable {yes} of {total}");
    assert!(total > 0, "no questions were generated");
    assert!(
        answerability_is_within_bar(yes, total),
        "{yes} of {total} questions are answerable from an existing card (the bar is at most one in five)"
    );
}

#[derive(Deserialize)]
struct IndependentModels {
    sections: Vec<SectionModels>,
}

#[derive(Deserialize)]
struct SectionModels {
    models: Vec<String>,
}

#[derive(Deserialize)]
struct Uncovered {
    uncovered: Vec<String>,
}

#[test]
#[ignore = "costed: real examiner, independent listing and judge calls over ALIX_EXAM_PROBE_DECKS"]
fn an_independent_model_list_finds_few_models_no_question_covers() {
    let (exam_cfg, ask) = configs();
    let mut sections_with_gaps = 0;
    let mut sections_seen = 0;
    for deck in probe_decks() {
        let sections = exam::exam_sections(&deck);
        let questions = exam::generate_questions(&deck, &exam_cfg, &ask).unwrap();
        let source = independent_source(&deck);
        let headings: String = sections
            .iter()
            .enumerate()
            .map(|(i, s)| format!("Section {}: {}\n{}", i + 1, s.label, card_list(s)))
            .collect();
        let listed: IndependentModels = parse_json(&judge(
            &ask,
            &format!(
                "{source}\nFrom the source above alone, list for each section below the \
                 distinct ideas a student must understand to master it.\n\n{headings}\nOutput \
                 ONLY JSON: {{\"sections\": [{{\"models\": [\"...\"]}}]}} with one entry per \
                 section, in order."
            ),
        ))
        .unwrap();
        assert_eq!(
            sections.len(),
            listed.sections.len(),
            "one model list per section"
        );
        for (at, (section, independent)) in sections.iter().zip(&listed.sections).enumerate() {
            let asked: String = questions
                .iter()
                .filter(|q| q.section == at)
                .map(|q| format!("- {}\n", q.prompt.replace('\n', "\n  ")))
                .collect();
            let models: String = independent
                .models
                .iter()
                .map(|m| format!("- {m}\n"))
                .collect();
            let gaps: Uncovered = parse_json(&judge(
                &ask,
                &format!(
                    "Ideas:\n{models}\nQuestions:\n{asked}\nWhich ideas does no question test? \
                     Output ONLY JSON: {{\"uncovered\": [\"...\"]}} (empty when every idea is \
                     tested)."
                ),
            ))
            .unwrap();
            println!(
                "probe: deck={} section={:?} independent_models={} uncovered={:?}",
                deck.display_name(),
                section.label,
                independent.models.len(),
                gaps.uncovered
            );
            sections_seen += 1;
            if !gaps.uncovered.is_empty() {
                sections_with_gaps += 1;
            }
        }
    }
    println!("probe: sections with an uncovered model {sections_with_gaps} of {sections_seen}");
    assert!(sections_seen > 0, "no sections were probed");
    assert!(
        sections_with_gaps * 5 <= sections_seen,
        "{sections_with_gaps} of {sections_seen} sections have an idea no question tests (the \
         bar is at most one in five; above it the independent check moves into the runtime)"
    );
}

#[test]
#[ignore = "costed: real examiner and one grade call over the largest deck in ALIX_EXAM_PROBE_DECKS"]
fn one_grade_call_grades_every_answer_of_a_long_sitting() {
    let (exam_cfg, ask) = configs();
    let deck = probe_decks()
        .into_iter()
        .max_by_key(|deck| deck.cards.len())
        .unwrap();
    let questions = exam::generate_questions(&deck, &exam_cfg, &ask).unwrap();
    println!(
        "probe: deck={} questions={}",
        deck.display_name(),
        questions.len()
    );
    assert!(
        questions.len() >= 20,
        "the longest probe deck yielded {} questions; pick a deck that yields 20 or more",
        questions.len()
    );
    let answers: Vec<String> = questions
        .iter()
        .map(|_| "I am not sure.".to_string())
        .collect();
    let result =
        exam::grade_answers(&questions, &answers, Strictness::Balanced, &exam_cfg, &ask).unwrap();
    assert_eq!(
        questions.len(),
        result.grades.len(),
        "every answer graded in one call"
    );
}

#[test]
fn the_answerability_bar_allows_exactly_one_question_in_five() {
    assert!(answerability_is_within_bar(1, 5));
    assert!(!answerability_is_within_bar(2, 5));
}

#[test]
fn the_independent_model_probe_reads_workspace_exam_sources() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("alix.toml"),
        "exam = \"sources/workspace.md\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("sources")).unwrap();
    std::fs::write(
        dir.path().join("sources/workspace.md"),
        "workspace-only model",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("decks")).unwrap();
    let deck_path = dir.path().join("decks/d.md");
    std::fs::write(&deck_path, "## q\na\n").unwrap();
    let deck = Deck::load(&deck_path).unwrap();

    let source = independent_source(&deck);

    assert!(
        source.contains("workspace-only model"),
        "the independent list must inspect every source generation inspects: {source:?}"
    );
}
