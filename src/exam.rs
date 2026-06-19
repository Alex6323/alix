//! The AI exam — frontend-agnostic engine.
//!
//! A deck's mechanical drill *loads* its material; this exam *verifies
//! understanding* and gates progression. It grades against the deck's declared
//! `% source:` (a URL Claude reads with WebFetch, or a local file embedded in
//! the prompt) — never the cards, which avoids circularity. Three Claude calls,
//! each through the same CLI runner [`crate::ask::run`] that `generate` uses:
//!
//! 1. [`generate_questions`] — fresh open understanding questions from the
//!    source, each with the key points a correct answer must contain.
//! 2. [`grade_answers`] — a strict examiner grades the typed answers against
//!    those points and returns an overall pass/fail by threshold.
//! 3. [`remediation_cards`] — on a fail, turns the missed concepts into cards
//!    (cloze/plain for facts, `% mode: explain` for concepts), as deck-format
//!    text ready to append.
//!
//! The engine is pure: it builds prompts, calls the CLI and parses JSON. A CLI
//! consumer (`flash exam`) drives the terminal Q&A; a web exam surface can
//! reuse the same three functions.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::{
    ask,
    config::{AskConfig, ExamConfig, Strictness},
    deck::{self, Deck},
};

/// Largest embedded local source file, in bytes. Larger files are truncated
/// (with a marker) so the prompt stays within the model's context.
const MAX_SOURCE_BYTES: usize = 100_000;

/// One open exam question with the key points a correct answer must cover.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ExamQuestion {
    /// The question shown to the student.
    pub prompt: String,
    /// The points a full answer must demonstrate (the grading rubric).
    pub points: Vec<String>,
}

/// How well one answer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Covered the key points.
    Pass,
    /// Partially correct — some points missed.
    Partial,
    /// Did not demonstrate understanding.
    Fail,
}

impl Verdict {
    /// The label shown to the student.
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Partial => "PARTIAL",
            Verdict::Fail => "FAIL",
        }
    }
}

/// The grade for one answer.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AnswerGrade {
    pub verdict: Verdict,
    /// One or two sentences explaining the verdict.
    pub feedback: String,
    /// The specific points the answer missed (empty on a clean pass).
    #[serde(default)]
    pub missed: Vec<String>,
}

/// The outcome of a sitting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamResult {
    /// Whether the sitting passed (enough full passes to clear the threshold).
    pub passed: bool,
    /// Per-question grades, in question order.
    pub grades: Vec<AnswerGrade>,
}

impl ExamResult {
    /// The concepts to remediate: every missed point from non-passing answers,
    /// de-duplicated in first-seen order.
    pub fn gaps(&self) -> Vec<String> {
        let mut gaps = Vec::new();
        for grade in &self.grades {
            if grade.verdict == Verdict::Pass {
                continue;
            }
            for point in &grade.missed {
                let point = point.trim();
                if !point.is_empty() && !gaps.iter().any(|g: &String| g == point) {
                    gaps.push(point.to_string());
                }
            }
        }
        gaps
    }
}

/// Generates `cfg.num_questions` open understanding questions from the deck's
/// `% source:`. Blocks until the CLI replies or times out.
pub fn generate_questions(
    deck: &Deck,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<Vec<ExamQuestion>> {
    if deck.sources.is_empty() {
        bail!("the deck declares no `% source:` to examine against");
    }
    let prompt = questions_prompt(deck, cfg)?;
    let raw = ask::run(&run_config(cfg, ask_cfg), &prompt, &[])?;
    let parsed: QuestionsDto = parse_json(&raw).context("parsing the generated questions")?;
    if parsed.questions.is_empty() {
        bail!("the model returned no questions");
    }
    for q in &parsed.questions {
        if q.prompt.trim().is_empty() || q.points.is_empty() {
            bail!("the model returned a malformed question (empty prompt or no points)");
        }
    }
    Ok(parsed.questions)
}

/// Grades the typed `answers` (one per question, same order) against each
/// question's points, at the given `strictness`, and returns the overall
/// result. A question "passes" when its verdict is [`Verdict::Pass`]; the
/// sitting passes when the fraction of passes is at least `cfg.pass_threshold`.
pub fn grade_answers(
    questions: &[ExamQuestion],
    answers: &[String],
    strictness: Strictness,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<ExamResult> {
    if questions.len() != answers.len() {
        bail!(
            "expected {} answers, got {}",
            questions.len(),
            answers.len()
        );
    }
    let prompt = grade_prompt(questions, answers, strictness);
    let raw = ask::run(&run_config(cfg, ask_cfg), &prompt, &[])?;
    let parsed: GradesDto = parse_json(&raw).context("parsing the grades")?;
    if parsed.grades.len() != questions.len() {
        bail!(
            "the model graded {} of {} answers",
            parsed.grades.len(),
            questions.len()
        );
    }
    Ok(ExamResult {
        passed: passed(&parsed.grades, cfg.pass_threshold),
        grades: parsed.grades,
    })
}

/// Turns the missed `gaps` into cards — a cloze/plain card for a missed fact, a
/// `% mode: explain` card for a missed concept — and returns the cleaned
/// deck-format text, ready to append to the deck file.
pub fn remediation_cards(gaps: &[String], cfg: &ExamConfig, ask_cfg: &AskConfig) -> Result<String> {
    if gaps.is_empty() {
        bail!("no gaps to remediate");
    }
    let prompt = remediation_prompt(gaps);
    let raw = ask::run(&run_config(cfg, ask_cfg), &prompt, &[])?;
    let cards = clean_deck_output(&raw);
    if cards.trim().is_empty() {
        bail!("the model returned no remediation cards");
    }
    Ok(cards)
}

/// `true` when the fraction of full passes meets the threshold.
fn passed(grades: &[AnswerGrade], threshold: f64) -> bool {
    if grades.is_empty() {
        return false;
    }
    let passes = grades.iter().filter(|g| g.verdict == Verdict::Pass).count();
    (passes as f64) / (grades.len() as f64) >= threshold
}

/// The CLI runner config for the exam: the ask command/permission/tools with
/// the exam's own model and (longer) timeout.
fn run_config(cfg: &ExamConfig, ask_cfg: &AskConfig) -> AskConfig {
    AskConfig {
        command: ask_cfg.command.clone(),
        permission_mode: ask_cfg.permission_mode.clone(),
        allowed_tools: ask_cfg.allowed_tools.clone(),
        model: cfg.model.clone().or_else(|| ask_cfg.model.clone()),
        timeout_secs: cfg.timeout_secs,
    }
}

/// Renders the deck's sources into a prompt section: URLs become WebFetch
/// instructions, local files are read and embedded (bounded). Relative file
/// paths resolve against the deck file's folder.
fn source_section(deck: &Deck) -> Result<String> {
    let base = deck.path.parent();
    let mut urls = Vec::new();
    let mut files = Vec::new();
    for src in &deck.sources {
        if deck::is_url(src) {
            urls.push(src.clone());
        } else {
            let path = match base {
                Some(dir) => dir.join(src),
                None => std::path::PathBuf::from(src),
            };
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read source file {}", path.display()))?;
            files.push((src.clone(), truncate(&text)));
        }
    }

    let mut out = String::new();
    if !urls.is_empty() {
        out.push_str(
            "Read these source pages with the WebFetch tool (fetch each once) — \
             they are the ground truth for this exam:\n",
        );
        for url in &urls {
            out.push_str("  - ");
            out.push_str(url);
            out.push('\n');
        }
    }
    for (label, text) in &files {
        out.push_str(&format!(
            "\nSource file `{label}` (the ground truth for this exam):\n<<<SOURCE\n{text}\nSOURCE\n"
        ));
    }
    Ok(out)
}

/// Truncates source text to [`MAX_SOURCE_BYTES`] on a char boundary, appending
/// a marker when it had to cut.
fn truncate(text: &str) -> String {
    if text.len() <= MAX_SOURCE_BYTES {
        return text.to_string();
    }
    let mut end = MAX_SOURCE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[... source truncated ...]", &text[..end])
}

/// Builds the question-generation prompt.
fn questions_prompt(deck: &Deck, cfg: &ExamConfig) -> Result<String> {
    let sources = source_section(deck)?;
    let mut prompt = format!(
        "You are a strict examiner writing an oral exam that verifies a student \
         truly UNDERSTANDS a topic — not whether they memorized isolated facts.\n\n\
         {sources}\n\
         Write exactly {n} open-ended questions grounded in the source above. \
         Favor APPLICATION (\"given X, what happens / what would you do?\") and \
         CONNECTIONS (how ideas relate, contrast or build on each other) over \
         plain recall. Each question must be answerable from the source and \
         demand reasoning in the student's own words.\n\n\
         For each question, list the key points a correct answer MUST contain — \
         the specific, source-grounded facts or reasoning steps you will grade \
         against. Be concrete; these are the rubric.\n\n\
         Output ONLY JSON in exactly this shape, no prose, no code fences:\n\
         {{\"questions\": [{{\"prompt\": \"...\", \"points\": [\"...\", \"...\"]}}]}}",
        n = cfg.num_questions,
    );
    if let Some(extra) = cfg
        .extra
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        prompt.push_str("\n\nAdditional instructions:\n");
        prompt.push_str(extra);
    }
    Ok(prompt)
}

/// The grading criteria for a strictness level — the part of the grade prompt
/// that decides how generously a typed answer is judged against the rubric and
/// what counts as a `missed` point (which drives remediation).
fn strictness_criteria(strictness: Strictness) -> &'static str {
    match strictness {
        Strictness::Strict => {
            "\
Grade strictly for COMPLETENESS — treat the rubric as a checklist. The answer \
passes only if it covers EVERY key point (wording may differ, but each point \
must be present). List in `missed` every key point the answer omits or gets \
wrong; here an omitted point IS a gap, because this material requires recalling \
all of it.\n\
- \"pass\": every key point is present.\n\
- \"partial\": some present, others missing or wrong.\n\
- \"fail\": most key points missing or wrong."
        }
        Strictness::Balanced => {
            "\
Grade for UNDERSTANDING, not completeness of phrasing. A key point counts as \
covered if the answer shows the student grasps it — briefly, in their own \
words, or by clear implication. Put a point in `missed` ONLY if the answer is \
wrong about it or shows the student genuinely does not grasp it — NEVER merely \
because they didn't spell it out or skipped a secondary detail.\n\
- \"pass\": the answer demonstrates the question's core idea.\n\
- \"partial\": part understood, but something important is wrong or absent.\n\
- \"fail\": little or no understanding."
        }
        Strictness::Lenient => {
            "\
Grade generously — the goal is only to catch real misunderstandings. Give the \
student the benefit of the doubt on anything plausibly correct or partially \
stated. Put a point in `missed` ONLY if the answer is clearly wrong about it or \
the question was essentially not answered.\n\
- \"pass\": broadly correct, even if thin.\n\
- \"partial\": partly right but with a clear error.\n\
- \"fail\": wrong or essentially unanswered."
        }
    }
}

/// Builds the grading prompt: the strictness criteria, then each question, its
/// rubric points and the student's answer.
fn grade_prompt(questions: &[ExamQuestion], answers: &[String], strictness: Strictness) -> String {
    let mut prompt = String::from(
        "You are an examiner grading an understanding exam. For each question you \
         are given the key points a correct answer covers (the rubric) and the \
         student's answer.\n",
    );
    prompt.push_str(strictness_criteria(strictness));
    prompt.push_str(
        "\nKeep `feedback` to one or two sentences, and cite the specific missed \
         point(s) in `missed` per the rule above.\n\n",
    );
    for (i, (q, a)) in questions.iter().zip(answers).enumerate() {
        prompt.push_str(&format!("Question {}: {}\n", i + 1, q.prompt));
        prompt.push_str("Key points (rubric):\n");
        for point in &q.points {
            prompt.push_str("  - ");
            prompt.push_str(point);
            prompt.push('\n');
        }
        let answer = a.trim();
        let answer = if answer.is_empty() {
            "(no answer given)"
        } else {
            answer
        };
        prompt.push_str("Student's answer: ");
        prompt.push_str(answer);
        prompt.push_str("\n\n");
    }
    prompt.push_str(
        "Output ONLY JSON in exactly this shape, one grade per question in order, \
         no prose, no code fences:\n\
         {\"grades\": [{\"verdict\": \"pass|partial|fail\", \"feedback\": \"...\", \
         \"missed\": [\"...\"]}]}",
    );
    prompt
}

/// Builds the remediation prompt: turn missed concepts into cards, choosing the
/// type per gap — a cloze/plain card for a missed fact or term, a
/// `% mode: explain` understanding card for a missed concept or connection.
fn remediation_prompt(gaps: &[String]) -> String {
    let mut prompt = String::from(
        "A student failed an understanding exam on the concepts below. Turn them \
         into spaced-repetition cards that close each gap.\n\n\
         FIRST consolidate the list: some of these concepts overlap or restate one \
         another. MERGE overlapping concepts and write ONE card per DISTINCT idea — \
         never two cards that test the same thing. Fewer, non-overlapping cards are \
         better than covering every line verbatim; only merge true duplicates, \
         never drop a distinct idea.\n\n\
         CHOOSE THE CARD TYPE per gap, by what the gap actually is:\n\
         - A missed FACT or TERM (a definition, a value, what lives where) -> a \
         cheap recall card. Prefer a cloze: a `#?` front with a short instruction, \
         and an indented answer line that states the fact with each hidden span \
         wrapped in {{double curly braces}} (a lone single brace is literal). If \
         there is no natural word to blank out, use a plain `# ` card with the \
         answer on the indented line below instead.\n\
         - A missed CONCEPT, MECHANISM or CONNECTION (a \"why\", \"how\" or \"what \
         happens if\") -> an understanding card: a `# ` open prompt, then the next \
         line exactly `% mode: explain`, then indented key points a good answer \
         covers. This forces the student to re-derive the idea, which is what the \
         exam re-tests. When unsure, prefer the understanding card.\n\n\
         CONCEPTS THE STUDENT MISSED:\n",
    );
    for gap in gaps {
        prompt.push_str("  - ");
        prompt.push_str(gap);
        prompt.push('\n');
    }
    prompt.push_str(
        "\nFORMAT — a plain-text deck, cards one after another. A card's answer is \
         on the indented (tab) line(s) below its front; an indented `! ` line after \
         it adds an optional note (a caveat, example or why it matters). One \
         example of each card type:\n\
         #? Recall how a String is laid out in memory.\n\
         \tA String stores a {{pointer}}, {{length}} and {{capacity}} on the stack, \
         and its bytes live on the {{heap}}.\n\
         # What does `drop` do for a String, and when?\n\
         \tIt returns the String's heap buffer to the allocator, at the end of the \
         owning scope.\n\
         # Why does moving a String invalidate the original binding?\n\
         % mode: explain\n\
         \tBoth bindings would otherwise point at the same heap buffer.\n\
         \tDropping both would free it twice (a double free).\n\
         \tSo Rust invalidates the source instead of allowing two owners.\n\
         \t! A move, not a shallow copy you can keep using.\n\n\
         Before finishing, re-read your cards as a set and merge any two that test \
         the same idea, so every card is distinct.\n\
         Output ONLY the deck text — no markdown code fences, no preamble, no \
         closing remarks.",
    );
    prompt
}

/// Extracts the JSON object from a model reply that may be wrapped in code
/// fences or surrounded by prose: the substring from the first `{` to the last
/// `}`. Falls back to the trimmed input.
fn extract_json(raw: &str) -> &str {
    match (raw.find('{'), raw.rfind('}')) {
        (Some(start), Some(end)) if end > start => &raw[start..=end],
        _ => raw.trim(),
    }
}

/// Parses `raw` (possibly fenced/with preamble) into `T`.
fn parse_json<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T> {
    let json = extract_json(raw);
    serde_json::from_str(json)
        .with_context(|| format!("the model did not return valid JSON:\n{json}"))
}

/// Strips code fences / leading commentary from generated deck text, like
/// [`crate::generate`] does: a deck starts with a `%` comment or a `#` card
/// front, and trailing blank/fence lines are dropped.
fn clean_deck_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(start) = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with('%') || t.starts_with('#')
    }) else {
        return raw.trim().to_string();
    };
    let mut end = lines.len();
    while end > start + 1 {
        let t = lines[end - 1].trim();
        if t.is_empty() || t.starts_with("```") {
            end -= 1;
        } else {
            break;
        }
    }
    lines[start..end].join("\n")
}

#[derive(Debug, Deserialize)]
struct QuestionsDto {
    questions: Vec<ExamQuestion>,
}

#[derive(Debug, Deserialize)]
struct GradesDto {
    grades: Vec<AnswerGrade>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck_with_sources(srcs: &[&str]) -> Deck {
        Deck {
            path: std::path::PathBuf::from("/tmp/d.txt"),
            subject: "d.txt".to_string(),
            cards: Vec::new(),
            links: Vec::new(),
            requires: Vec::new(),
            sources: srcs.iter().map(|s| s.to_string()).collect(),
            settings: Default::default(),
        }
    }

    #[test]
    fn questions_prompt_carries_source_strictness_and_json_shape() {
        let deck = deck_with_sources(&["https://example.org/ownership"]);
        let p = questions_prompt(&deck, &ExamConfig::default()).unwrap();
        assert!(p.contains("https://example.org/ownership"));
        assert!(p.contains("WebFetch"));
        assert!(p.contains("strict examiner"));
        assert!(p.contains("APPLICATION"));
        assert!(p.contains("\"questions\""));
        assert!(p.contains("exactly 5 open-ended questions"));
    }

    #[test]
    fn questions_prompt_honors_num_questions_and_extra() {
        let deck = deck_with_sources(&["https://x"]);
        let cfg = ExamConfig {
            num_questions: 3,
            extra: Some("Focus on lifetimes.".to_string()),
            ..ExamConfig::default()
        };
        let p = questions_prompt(&deck, &cfg).unwrap();
        assert!(p.contains("exactly 3 open-ended questions"));
        assert!(p.contains("Additional instructions:"));
        assert!(p.contains("Focus on lifetimes."));
    }

    #[test]
    fn grade_prompt_includes_rubric_and_answers() {
        let qs = vec![ExamQuestion {
            prompt: "Why move?".to_string(),
            points: vec!["avoids double free".to_string()],
        }];
        let p = grade_prompt(
            &qs,
            &["because ownership".to_string()],
            Strictness::Balanced,
        );
        assert!(p.contains("Question 1: Why move?"));
        assert!(p.contains("avoids double free"));
        assert!(p.contains("because ownership"));
        assert!(p.contains("examiner"));
        assert!(p.contains("\"grades\""));
    }

    #[test]
    fn grade_prompt_marks_empty_answers() {
        let qs = vec![ExamQuestion {
            prompt: "Q".to_string(),
            points: vec!["p".to_string()],
        }];
        let p = grade_prompt(&qs, &["   ".to_string()], Strictness::Balanced);
        assert!(p.contains("(no answer given)"));
    }

    #[test]
    fn grade_prompt_criteria_vary_by_strictness() {
        let qs = vec![ExamQuestion {
            prompt: "Q".to_string(),
            points: vec!["p".to_string()],
        }];
        let a = vec!["a".to_string()];
        let strict = grade_prompt(&qs, &a, Strictness::Strict);
        let balanced = grade_prompt(&qs, &a, Strictness::Balanced);
        let lenient = grade_prompt(&qs, &a, Strictness::Lenient);
        // Strict demands completeness; balanced judges understanding; lenient is
        // generous. Each carries language the others don't.
        assert!(strict.contains("COMPLETENESS"));
        assert!(strict.contains("EVERY key point"));
        assert!(balanced.contains("UNDERSTANDING, not completeness"));
        assert!(!balanced.contains("COMPLETENESS — treat the rubric"));
        assert!(lenient.contains("benefit of the doubt"));
    }

    #[test]
    fn remediation_prompt_asks_for_explain_cards() {
        let p = remediation_prompt(&["the aliasing rule".to_string()]);
        assert!(p.contains("the aliasing rule"));
        assert!(p.contains("% mode: explain"));
        assert!(p.contains("Output ONLY the deck text"));
    }

    #[test]
    fn remediation_prompt_picks_card_type_per_gap() {
        let p = remediation_prompt(&["x".to_string()]);
        // Facts get a cheap recall card (cloze/plain); concepts get explain.
        assert!(p.contains("missed FACT or TERM"));
        assert!(p.contains("missed CONCEPT"));
        assert!(p.contains("#?")); // cloze front for facts
        assert!(p.contains("double curly braces"));
        assert!(p.contains("% mode: explain")); // understanding card for concepts
    }

    #[test]
    fn remediation_prompt_asks_to_dedup_overlapping_gaps() {
        let p = remediation_prompt(&[
            "String stores a pointer, length and capacity on the stack".to_string(),
            "A String keeps its bytes on the heap and a pointer on the stack".to_string(),
        ]);
        // Both near-duplicate gaps still appear, but the model is told to merge.
        assert!(p.contains("String stores a pointer"));
        assert!(p.contains("keeps its bytes on the heap"));
        assert!(p.contains("MERGE overlapping concepts"));
        assert!(p.contains("ONE card per DISTINCT idea"));
        assert!(p.contains("merge any two that test the same idea"));
    }

    #[test]
    fn parse_questions_from_fenced_json() {
        let raw = "Here you go:\n```json\n{\"questions\": [{\"prompt\": \"Q1\", \
                   \"points\": [\"a\", \"b\"]}]}\n```";
        let dto: QuestionsDto = parse_json(raw).unwrap();
        assert_eq!(1, dto.questions.len());
        assert_eq!("Q1", dto.questions[0].prompt);
        assert_eq!(vec!["a", "b"], dto.questions[0].points);
    }

    #[test]
    fn parse_grades_with_verdicts() {
        let raw = "{\"grades\": [\
            {\"verdict\": \"pass\", \"feedback\": \"good\", \"missed\": []},\
            {\"verdict\": \"fail\", \"feedback\": \"no\", \"missed\": [\"x\"]}]}";
        let dto: GradesDto = parse_json(raw).unwrap();
        assert_eq!(Verdict::Pass, dto.grades[0].verdict);
        assert_eq!(Verdict::Fail, dto.grades[1].verdict);
        assert_eq!(vec!["x"], dto.grades[1].missed);
    }

    #[test]
    fn missed_defaults_when_absent() {
        let raw = "{\"grades\": [{\"verdict\": \"pass\", \"feedback\": \"ok\"}]}";
        let dto: GradesDto = parse_json(raw).unwrap();
        assert!(dto.grades[0].missed.is_empty());
    }

    #[test]
    fn invalid_json_is_a_clear_error() {
        let err = parse_json::<GradesDto>("not json at all").unwrap_err();
        assert!(format!("{err:#}").contains("valid JSON"));
    }

    #[test]
    fn threshold_all_pass_passes() {
        let grades = vec![
            AnswerGrade {
                verdict: Verdict::Pass,
                feedback: String::new(),
                missed: Vec::new(),
            },
            AnswerGrade {
                verdict: Verdict::Pass,
                feedback: String::new(),
                missed: Vec::new(),
            },
        ];
        assert!(passed(&grades, 1.0));
    }

    #[test]
    fn threshold_one_partial_fails_strict() {
        let grades = vec![
            AnswerGrade {
                verdict: Verdict::Pass,
                feedback: String::new(),
                missed: Vec::new(),
            },
            AnswerGrade {
                verdict: Verdict::Partial,
                feedback: String::new(),
                missed: vec!["m".to_string()],
            },
        ];
        assert!(!passed(&grades, 1.0));
        // A looser threshold (half) would let it through.
        assert!(passed(&grades, 0.5));
    }

    #[test]
    fn gaps_collects_missed_from_non_passes_deduped() {
        let result = ExamResult {
            passed: false,
            grades: vec![
                AnswerGrade {
                    verdict: Verdict::Pass,
                    feedback: String::new(),
                    missed: vec!["ignored (passed)".to_string()],
                },
                AnswerGrade {
                    verdict: Verdict::Fail,
                    feedback: String::new(),
                    missed: vec!["dup".to_string(), "unique".to_string()],
                },
                AnswerGrade {
                    verdict: Verdict::Partial,
                    feedback: String::new(),
                    missed: vec!["dup".to_string()],
                },
            ],
        };
        assert_eq!(vec!["dup", "unique"], result.gaps());
    }

    #[test]
    fn clean_deck_output_strips_fences() {
        let raw = "```text\n# Q\n% mode: explain\n\tA\n```";
        assert_eq!("# Q\n% mode: explain\n\tA", clean_deck_output(raw));
    }

    /// Serializes tests that write + exec scripts (a concurrent writer's fd
    /// would make exec fail with ETXTBSY), like the ask tests.
    static EXEC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Writes a fake `claude` CLI that emits `body` and returns its path.
    fn fake_cli(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn ask_config(command: &std::path::Path) -> AskConfig {
        AskConfig {
            command: command.to_str().unwrap().to_string(),
            timeout_secs: 10,
            ..AskConfig::default()
        }
    }

    #[test]
    fn generate_questions_end_to_end() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let json = "{\"questions\":[{\"prompt\":\"Why move?\",\
                    \"points\":[\"avoids double free\"]}]}";
        let cli = fake_cli(dir.path(), &format!("printf '%s' '{json}'"));
        let deck = deck_with_sources(&["https://x"]);
        let cfg = ExamConfig {
            num_questions: 1,
            ..ExamConfig::default()
        };
        let qs = generate_questions(&deck, &cfg, &ask_config(&cli)).unwrap();
        assert_eq!(1, qs.len());
        assert_eq!("Why move?", qs[0].prompt);
        assert_eq!(vec!["avoids double free"], qs[0].points);
    }

    #[test]
    fn grade_answers_end_to_end_pass() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let json = "{\"grades\":[{\"verdict\":\"pass\",\"feedback\":\"good\",\"missed\":[]}]}";
        let cli = fake_cli(dir.path(), &format!("printf '%s' '{json}'"));
        let qs = vec![ExamQuestion {
            prompt: "Q".to_string(),
            points: vec!["p".to_string()],
        }];
        let result = grade_answers(
            &qs,
            &["my answer".to_string()],
            Strictness::Balanced,
            &ExamConfig::default(),
            &ask_config(&cli),
        )
        .unwrap();
        assert!(result.passed);
        assert_eq!(Verdict::Pass, result.grades[0].verdict);
    }

    #[test]
    fn remediation_cards_end_to_end() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // A fenced deck; clean_deck_output must strip the fences.
        let cli = fake_cli(
            dir.path(),
            "printf '%s\\n' '```text' '# Why?' '% mode: explain' '  point' '```'",
        );
        let cards = remediation_cards(
            &["the gap".to_string()],
            &ExamConfig::default(),
            &ask_config(&cli),
        )
        .unwrap();
        assert_eq!("# Why?\n% mode: explain\n  point", cards);
    }

    #[test]
    fn local_file_source_is_embedded() {
        let dir = std::env::temp_dir().join(format!("flash-exam-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("notes.md");
        std::fs::write(&src, "the ground truth text").unwrap();
        let deck = Deck {
            path: dir.join("d.txt"),
            subject: "d.txt".to_string(),
            cards: Vec::new(),
            links: Vec::new(),
            requires: Vec::new(),
            sources: vec!["notes.md".to_string()],
            settings: Default::default(),
        };
        let section = source_section(&deck).unwrap();
        assert!(section.contains("the ground truth text"));
        assert!(section.contains("notes.md"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
