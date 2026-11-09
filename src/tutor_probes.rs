use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ask::{TutorContext, parse_json, question_prompt, run, with_source_root},
    card::Card,
    config::{AskConfig, Audience, BackendKind, Config},
    deck::{Deck, ExamLayers},
    exam,
    exam_prompt::TopicMeasure,
    sources::{DeckSources, DisplayPath, SourceText, copy_name},
};

const DECKS_VAR: &str = "ALIX_TUTOR_EVAL_DECKS";
const OUT_VAR: &str = "ALIX_TUTOR_EVAL_OUT";
const CODEX_CWD_VAR: &str = "ALIX_TUTOR_EVAL_CODEX_CWD";
const CARDS_VAR: &str = "ALIX_TUTOR_EVAL_CARDS";
const QUESTIONS_VAR: &str = "ALIX_TUTOR_EVAL_QUESTIONS";
const CALL_TIMEOUT_SECS: u64 = 300;

const QUESTIONS: [&str; 3] = [
    "Explain why the answer on this card is true: what in the sources makes it so?",
    "What would break or change if the claim on this card were false, or if the code or text \
     did the opposite of what the card says?",
    "Give one concrete example from the sources that illustrates this card: name the file or \
     passage and quote or describe the exact spot.",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Condition {
    Browse,
    Paste,
}

#[derive(Clone, Serialize, Deserialize)]
struct Answered {
    deck: String,
    card: String,
    question: usize,
    condition: Condition,
    backend: String,
    answer: Option<String>,
    error: Option<String>,
    secs: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Verdict {
    question: usize,
    condition: Condition,
    backend: String,
    grounded: bool,
    specific: bool,
    correct: bool,
}

impl Verdict {
    fn passed(&self) -> bool {
        self.grounded && self.specific && self.correct
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Judged {
    deck: String,
    card: String,
    verdicts: Vec<Verdict>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum Record {
    Answered(Answered),
    Judged(Judged),
}

struct Log {
    path: PathBuf,
    records: Vec<Record>,
}

impl Log {
    fn open(path: &Path) -> Self {
        let records = fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("a record line"))
            .collect();
        Self {
            path: path.to_path_buf(),
            records,
        }
    }

    fn push(&mut self, record: Record) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .unwrap();
        writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
        self.records.push(record);
    }

    fn answer(
        &self,
        deck: &str,
        card: &str,
        question: usize,
        condition: Condition,
        backend: &str,
    ) -> Option<&Answered> {
        self.records.iter().rev().find_map(|record| match record {
            Record::Answered(answered)
                if answered.deck == deck
                    && answered.card == card
                    && answered.question == question
                    && answered.condition == condition
                    && answered.backend == backend =>
            {
                Some(answered)
            }
            _ => None,
        })
    }

    fn judged(&self, deck: &str, card: &str) -> bool {
        self.records.iter().any(
            |record| matches!(record, Record::Judged(judged) if judged.deck == deck && judged.card == card),
        )
    }

    fn verdicts(&self) -> impl Iterator<Item = (&str, &Verdict)> {
        self.records
            .iter()
            .filter_map(|record| match record {
                Record::Judged(judged) => Some(judged),
                _ => None,
            })
            .flat_map(|judged| {
                judged
                    .verdicts
                    .iter()
                    .map(move |v| (judged.deck.as_str(), v))
            })
    }
}

#[derive(Default)]
struct CostGuard {
    calls: usize,
    failures: usize,
}

impl CostGuard {
    fn note(&mut self, failed: bool) {
        self.calls += 1;
        if failed {
            self.failures += 1;
        }
        assert!(
            !over_failure_budget(self.calls, self.failures),
            "cost guard: {} of {} calls failed for non-answer reasons; stopping",
            self.failures,
            self.calls
        );
    }
}

fn over_failure_budget(calls: usize, failures: usize) -> bool {
    calls >= 10 && failures * 5 > calls
}

fn env_or(name: &str, fallback: usize) -> usize {
    std::env::var(name)
        .ok()
        .map(|value| value.parse().expect("a count"))
        .unwrap_or(fallback)
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
}

fn probe_decks() -> Vec<Deck> {
    required(DECKS_VAR)
        .split(':')
        .filter(|path| !path.is_empty())
        .map(|path| exam::open_for_exam(Path::new(path)).unwrap())
        .collect()
}

fn pick_cards<'a>(deck: &'a Deck, sources: &DeckSources, wanted: usize) -> Vec<&'a Card> {
    let topic_ids = deck.topic_ids();
    let sourced = sources.sourced_topics();
    let eligible: Vec<&Card> = deck
        .cards
        .iter()
        .filter(|card| card.id().is_some())
        .filter(|card| {
            topic_ids[card.topic]
                .as_ref()
                .is_some_and(|id| sourced.contains(id))
        })
        .collect();
    let step = (eligible.len() / wanted).max(1);
    eligible.into_iter().step_by(step).take(wanted).collect()
}

fn insert_before_card(prompt: String, block: &str) -> String {
    const CARD_HEADER: &str = "The card being reviewed:";
    assert_eq!(prompt.matches(CARD_HEADER).count(), 1, "one card header");
    prompt.replacen(CARD_HEADER, &format!("{block}\n{CARD_HEADER}"), 1)
}

fn browse_prompt(card: &Card, sources: &DeckSources, question: &str) -> String {
    let context = TutorContext {
        links: &[],
        sources: &ExamLayers::default(),
        root: Some(&sources.dir),
        frozen: None,
    };
    let base = question_prompt(card, Audience::Adult, &context, question, true);
    let mut rows: Vec<(&String, &String)> = sources
        .manifest
        .sources
        .iter()
        .map(|(id, row)| (&row.path, id))
        .collect();
    rows.sort();
    let mut block = String::from(
        "The deck's frozen sources are the files in your working directory. This map gives \
         each file's original path and the name of its frozen copy there:\n",
    );
    for (path, id) in rows {
        let display: DisplayPath = path.parse().unwrap();
        block.push_str(&format!("{path} -> {}\n", copy_name(id, &display)));
    }
    insert_before_card(base, &block)
}

fn source_block(texts: &[SourceText]) -> String {
    let mut block = String::from(
        "The deck's frozen sources for this card's topic, the primary grounding for your answer:\n",
    );
    for source in texts {
        block.push_str(&format!("\nSource `{}`:\n<<<SOURCE\n", source.display));
        block.push_str(&source.text);
        if !source.text.ends_with('\n') {
            block.push('\n');
        }
        block.push_str("SOURCE\n");
    }
    block
}

fn paste_prompt(card: &Card, texts: &[SourceText], question: &str) -> String {
    let context = TutorContext {
        links: &[],
        sources: &ExamLayers::default(),
        root: None,
        frozen: None,
    };
    let base = question_prompt(card, Audience::Adult, &context, question, true);
    insert_before_card(base, &source_block(texts))
}

fn card_block(card: &Card) -> String {
    let mut block = format!("Front: {}\nAnswer:\n", card.question_text());
    for line in &card.back {
        block.push_str(line);
        block.push('\n');
    }
    for note in &card.notes {
        block.push_str("Note: ");
        block.push_str(&note.body);
        block.push('\n');
    }
    block
}

fn shuffle_key(card: &str, answered: &Answered) -> [u8; 32] {
    Sha256::digest(format!(
        "{card}:{}:{:?}:{}",
        answered.backend, answered.condition, answered.question
    ))
    .into()
}

#[derive(Deserialize)]
struct JudgeReply {
    verdicts: Vec<JudgeVerdict>,
}

#[derive(Deserialize)]
struct JudgeVerdict {
    grounded: bool,
    specific: bool,
    correct: bool,
}

fn judge_prompt(card: &Card, texts: &[SourceText], answers: &[&Answered]) -> String {
    let mut prompt = source_block(texts);
    prompt.push_str(
        "\nA learner reviewing the flashcard below asked a tutor questions about it. Several \
         tutors answered; each numbered answer below is one tutor's answer to one question. \
         Judge every answer against the sources above (the sources are the ground truth; the \
         card summarizes them):\n\
         - grounded: the answer draws on the sources above (facts, names, code, or passages \
         that appear there), not on generic knowledge or invention;\n\
         - specific: the answer addresses this card and this question concretely, not with \
         generalities that would fit any card;\n\
         - correct: every claim in the answer agrees with the sources; one false or \
         unsupported claim makes the answer incorrect.\n\
         An answer that declines, or says it cannot find or read the material, fails all \
         three.\n\nThe card:\n",
    );
    prompt.push_str(&card_block(card));
    prompt.push_str("\nAnswers:\n");
    for (n, answered) in answers.iter().enumerate() {
        prompt.push_str(&format!(
            "\nAnswer {} (to the question: {})\n{}\n",
            n + 1,
            QUESTIONS[answered.question],
            answered.answer.as_deref().unwrap_or_default()
        ));
    }
    prompt.push_str(
        "\nOutput ONLY JSON: {\"verdicts\": [{\"grounded\": true|false, \"specific\": \
         true|false, \"correct\": true|false}, ...]} with one entry per answer, in order.",
    );
    prompt
}

struct Rates {
    judged: usize,
    passed: usize,
    grounded: usize,
    specific: usize,
    correct: usize,
}

impl Rates {
    fn rate(&self) -> f64 {
        if self.judged == 0 {
            0.0
        } else {
            self.passed as f64 / self.judged as f64
        }
    }
}

fn tally<'a>(verdicts: impl Iterator<Item = &'a Verdict>) -> Rates {
    let mut rates = Rates {
        judged: 0,
        passed: 0,
        grounded: 0,
        specific: 0,
        correct: 0,
    };
    for verdict in verdicts {
        rates.judged += 1;
        rates.passed += usize::from(verdict.passed());
        rates.grounded += usize::from(verdict.grounded);
        rates.specific += usize::from(verdict.specific);
        rates.correct += usize::from(verdict.correct);
    }
    rates
}

// Integer forms of the plan's two expectations, so a result exactly on the
// line counts as holding.
fn browse_holds_against_paste(browse: &Rates, paste: &Rates) -> bool {
    100 * browse.passed * paste.judged + 5 * browse.judged * paste.judged
        >= 100 * paste.passed * browse.judged
}

fn codex_paste_holds_against_claude_browse(codex_paste: &Rates, claude_browse: &Rates) -> bool {
    5 * codex_paste.passed * claude_browse.judged >= 4 * claude_browse.passed * codex_paste.judged
}

fn rates(passed: usize, judged: usize) -> Rates {
    Rates {
        judged,
        passed,
        grounded: 0,
        specific: 0,
        correct: 0,
    }
}

#[test]
#[ignore = "costed: tutor answers on Claude (browse and paste) and Codex (paste), judged by Claude, over ALIX_TUTOR_EVAL_DECKS"]
fn the_tutor_answers_as_well_browsing_frozen_sources_as_with_them_pasted() {
    let out = PathBuf::from(required(OUT_VAR));
    let codex_cwd = PathBuf::from(required(CODEX_CWD_VAR));
    let cards_per_deck = env_or(CARDS_VAR, 10);
    let questions_per_card = env_or(QUESTIONS_VAR, QUESTIONS.len()).min(QUESTIONS.len());
    let mut log = Log::open(&out);
    let config = Config::load(None).unwrap();
    let base = AskConfig {
        allowed_tools: vec![],
        timeout_secs: CALL_TIMEOUT_SECS,
        source_access: false,
        cwd: None,
        ..config.ask
    };
    let claude = AskConfig {
        backend: BackendKind::Claude,
        command: "claude".to_string(),
        ..base.clone()
    };
    let codex = AskConfig {
        backend: BackendKind::Codex,
        command: "codex".to_string(),
        cwd: Some(codex_cwd),
        ..base.clone()
    };
    let judge = AskConfig {
        timeout_secs: 600,
        ..claude.clone()
    };
    let runs: [(Condition, &AskConfig, &str); 3] = [
        (Condition::Browse, &claude, "claude"),
        (Condition::Paste, &claude, "claude"),
        (Condition::Paste, &codex, "codex"),
    ];
    let mut guard = CostGuard::default();

    for deck in probe_decks() {
        let deck_name = deck.display_name();
        let sources = deck
            .sources()
            .unwrap()
            .expect("the deck has frozen sources");
        let topic_ids = deck.topic_ids();
        for card in pick_cards(&deck, &sources, cards_per_deck) {
            let card_id = card.id().unwrap();
            let topic_id = topic_ids[card.topic].clone().unwrap();
            let texts = sources.texts_of(&topic_id).unwrap();
            for (question, text) in QUESTIONS.iter().enumerate().take(questions_per_card) {
                for (condition, cfg, backend) in runs {
                    if log
                        .answer(&deck_name, &card_id, question, condition, backend)
                        .is_some()
                    {
                        continue;
                    }
                    let (prompt, cfg) = match condition {
                        Condition::Browse => (
                            browse_prompt(card, &sources, text),
                            with_source_root(cfg, &sources.dir),
                        ),
                        Condition::Paste => (paste_prompt(card, &texts, text), cfg.clone()),
                    };
                    let measure = TopicMeasure::of(cfg.backend, &prompt);
                    assert!(
                        !measure.exceeds(),
                        "{deck_name}: the {condition:?} prompt exceeds {backend}'s limit"
                    );
                    let started = Instant::now();
                    let result = run(&cfg, &prompt, &[]);
                    let secs = started.elapsed().as_secs();
                    println!(
                        "probe: deck={deck_name} card={card_id} q={question} {condition:?} \
                         {backend} {secs}s {}",
                        match &result {
                            Ok(answer) => format!("ok ({} chars)", answer.chars().count()),
                            Err(error) => format!("ERROR {error:#}"),
                        }
                    );
                    guard.note(result.is_err());
                    log.push(Record::Answered(Answered {
                        deck: deck_name.clone(),
                        card: card_id.clone(),
                        question,
                        condition,
                        backend: backend.to_string(),
                        answer: result.as_ref().ok().cloned(),
                        error: result.err().map(|error| format!("{error:#}")),
                        secs,
                    }));
                }
            }
            if log.judged(&deck_name, &card_id) {
                continue;
            }
            let mut answers: Vec<Answered> = Vec::new();
            for question in 0..questions_per_card {
                for (condition, _, backend) in runs {
                    if let Some(answered) =
                        log.answer(&deck_name, &card_id, question, condition, backend)
                        && answered.answer.is_some()
                    {
                        answers.push(answered.clone());
                    }
                }
            }
            if answers.is_empty() {
                continue;
            }
            answers.sort_by_key(|answered| shuffle_key(&card_id, answered));
            let answers: Vec<&Answered> = answers.iter().collect();
            let reply = run(&judge, &judge_prompt(card, &texts, &answers), &[]);
            let parsed = reply.and_then(|reply| parse_json::<JudgeReply>(&reply));
            guard.note(parsed.is_err());
            let Ok(parsed) = parsed else {
                println!(
                    "probe: judge failed for card={card_id}: {:#}",
                    parsed.err().unwrap()
                );
                continue;
            };
            assert_eq!(
                answers.len(),
                parsed.verdicts.len(),
                "one verdict per answer"
            );
            let verdicts = answers
                .iter()
                .zip(parsed.verdicts)
                .map(|(answered, verdict)| Verdict {
                    question: answered.question,
                    condition: answered.condition,
                    backend: answered.backend.clone(),
                    grounded: verdict.grounded,
                    specific: verdict.specific,
                    correct: verdict.correct,
                })
                .collect();
            log.push(Record::Judged(Judged {
                deck: deck_name.clone(),
                card: card_id.clone(),
                verdicts,
            }));
        }
    }

    let mut by_arm: BTreeMap<(String, Condition), Vec<&Verdict>> = BTreeMap::new();
    let mut by_deck_arm: BTreeMap<(String, String, Condition), Vec<&Verdict>> = BTreeMap::new();
    for (deck, verdict) in log.verdicts() {
        by_arm
            .entry((verdict.backend.clone(), verdict.condition))
            .or_default()
            .push(verdict);
        by_deck_arm
            .entry((deck.to_string(), verdict.backend.clone(), verdict.condition))
            .or_default()
            .push(verdict);
    }
    for ((deck, backend, condition), verdicts) in &by_deck_arm {
        let rates = tally(verdicts.iter().copied());
        println!(
            "probe: deck={deck} {backend} {condition:?}: passed {} of {} (grounded {}, specific {}, correct {})",
            rates.passed, rates.judged, rates.grounded, rates.specific, rates.correct
        );
    }
    let rate_of = |backend: &str, condition: Condition| {
        by_arm
            .get(&(backend.to_string(), condition))
            .map(|verdicts| tally(verdicts.iter().copied()))
            .unwrap_or_else(|| tally(std::iter::empty()))
    };
    let claude_browse = rate_of("claude", Condition::Browse);
    let claude_paste = rate_of("claude", Condition::Paste);
    let codex_paste = rate_of("codex", Condition::Paste);
    for (label, rates) in [
        ("claude Browse", &claude_browse),
        ("claude Paste", &claude_paste),
        ("codex Paste", &codex_paste),
    ] {
        println!(
            "probe: {label}: passed {} of {} ({:.1}%); grounded {} specific {} correct {}",
            rates.passed,
            rates.judged,
            100.0 * rates.rate(),
            rates.grounded,
            rates.specific,
            rates.correct
        );
    }
    println!("probe: calls {} failures {}", guard.calls, guard.failures);
    assert!(claude_browse.judged > 0, "no answers were judged");
    assert!(
        browse_holds_against_paste(&claude_browse, &claude_paste),
        "Claude browse {:.1}% is more than 5 points under Claude paste {:.1}%",
        100.0 * claude_browse.rate(),
        100.0 * claude_paste.rate()
    );
    assert!(
        codex_paste_holds_against_claude_browse(&codex_paste, &claude_browse),
        "Codex paste {:.1}% is under 80% of Claude browse {:.1}%",
        100.0 * codex_paste.rate(),
        100.0 * claude_browse.rate()
    );
}

#[test]
fn the_cost_guard_stops_once_a_fifth_of_ten_or_more_calls_failed() {
    assert!(!over_failure_budget(9, 9), "under ten calls nothing stops");
    assert!(!over_failure_budget(10, 2), "two of ten is the budget");
    assert!(over_failure_budget(10, 3), "three of ten is over it");
}

#[test]
fn the_expectations_match_the_plan() {
    assert!(browse_holds_against_paste(&rates(80, 100), &rates(85, 100)));
    assert!(!browse_holds_against_paste(
        &rates(79, 100),
        &rates(85, 100)
    ));
    assert!(codex_paste_holds_against_claude_browse(
        &rates(64, 100),
        &rates(80, 100)
    ));
    assert!(!codex_paste_holds_against_claude_browse(
        &rates(63, 100),
        &rates(80, 100)
    ));
}
