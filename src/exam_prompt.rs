use std::fmt;

use crate::{
    config::{BackendKind, ExamConfig},
    exam_ideas::QUESTIONS_PER_TOPIC,
    exam_status::ExamTopic,
    sources::SourceText,
};

pub const IDEA_LINE_MAX_CHARS: usize = 300;
pub const RETRY_REASON_MAX_CHARS: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    Characters,
    EstimatedTokens {
        bytes_per_token_hundredths: u64,
        overhead: u64,
    },
}

impl Measure {
    pub fn label(self) -> &'static str {
        match self {
            Measure::Characters => "characters",
            Measure::EstimatedTokens { .. } => "estimated tokens",
        }
    }

    pub fn of(self, prompt: &str) -> u64 {
        match self {
            Measure::Characters => prompt.chars().count() as u64,
            Measure::EstimatedTokens {
                bytes_per_token_hundredths,
                overhead,
            } => (prompt.len() as u64 * 100).div_ceil(bytes_per_token_hundredths) + overhead,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptLimit {
    pub measure: Measure,
    pub limit: u64,
}

// Gemini is unmeasured; the smallest measured limit stands in.
pub fn prompt_limit(kind: BackendKind) -> PromptLimit {
    match kind {
        BackendKind::Codex => PromptLimit {
            measure: Measure::Characters,
            limit: 1_048_576,
        },
        BackendKind::Copilot | BackendKind::Gemini => PromptLimit {
            measure: Measure::EstimatedTokens {
                bytes_per_token_hundredths: 250,
                overhead: 2_200,
            },
            limit: 128_000,
        },
        BackendKind::Claude => PromptLimit {
            measure: Measure::EstimatedTokens {
                bytes_per_token_hundredths: 251,
                overhead: 0,
            },
            limit: 1_000_000,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopicMeasure {
    pub backend: BackendKind,
    pub size: u64,
    pub limit: PromptLimit,
}

impl TopicMeasure {
    pub fn of(backend: BackendKind, prompt: &str) -> Self {
        let limit = prompt_limit(backend);
        Self {
            backend,
            size: limit.measure.of(prompt),
            limit,
        }
    }

    pub fn exceeds(self) -> bool {
        self.size > self.limit.limit
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreflightError {
    pub topic: String,
    pub measure: TopicMeasure,
}

impl fmt::Display for PreflightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let TopicMeasure {
            backend,
            size,
            limit,
        } = self.measure;
        write!(
            f,
            "topic `{}` exceeds {}'s limit: {size} of {} {}",
            self.topic,
            backend.name(),
            limit.limit,
            limit.measure.label()
        )
    }
}

impl std::error::Error for PreflightError {}

pub fn measured_prompt(topic: &ExamTopic, sources: &[SourceText], cfg: &ExamConfig) -> String {
    let ideas = vec![widest(IDEA_LINE_MAX_CHARS); QUESTIONS_PER_TOPIC];
    retry_prompt(
        &questions_prompt(topic, sources, &ideas, cfg),
        &widest(RETRY_REASON_MAX_CHARS),
    )
}

// Four bytes per char: the heaviest a char-capped line weighs in bytes.
fn widest(chars: usize) -> String {
    "\u{10000}".repeat(chars)
}

pub fn retry_prompt(prompt: &str, reason: &str) -> String {
    format!(
        "{prompt}\n\nYour previous answer was rejected: {}. Write the whole answer \
         again and follow every rule above.",
        truncate_chars(reason, RETRY_REASON_MAX_CHARS)
    )
}

pub(crate) fn truncate_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((at, _)) => &s[..at],
        None => s,
    }
}

pub fn measure_topic(
    topic: &ExamTopic,
    sources: &[SourceText],
    cfg: &ExamConfig,
    backend: BackendKind,
) -> TopicMeasure {
    TopicMeasure::of(backend, &measured_prompt(topic, sources, cfg))
}

pub(crate) fn source_section(sources: &[SourceText]) -> String {
    let mut out = String::new();
    for source in sources {
        out.push_str(&format!("\nSource `{}`:\n<<<SOURCE\n", source.display));
        out.push_str(&source.text);
        if !source.text.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("SOURCE\n");
    }
    out
}

fn topic_scope(topic: &ExamTopic) -> String {
    let mut scope = format!("Topic: {}\n", topic.label);
    for line in topic.context.iter().skip(1) {
        scope.push_str(line);
        scope.push('\n');
    }
    scope.push_str("Cards:\n");
    for card in &topic.cards {
        scope.push_str(&format!("- {}\n", card.replace('\n', "\n  ")));
    }
    scope.push('\n');
    scope
}

pub fn ideas_prompt(topic: &ExamTopic, sources: &[SourceText], cfg: &ExamConfig) -> String {
    let sources = source_section(sources);
    let scope = topic_scope(topic);
    let mut prompt = format!(
        "You are preparing an oral exam that verifies a student truly UNDERSTANDS a \
         topic.\n\n\
         {sources}\n\
         The student drilled flashcards on this material. The topic below is marked out \
         by its heading, with the prose under it when there is any. Its cards show only \
         what the student has drilled so far; they do not mark the edge of the topic.\n\n\
         {scope}\
         List the topic's ideas: the distinct ideas a student must understand to master \
         this topic, as the sources above teach them. Cover everything the sources teach \
         within the topic, whether or not a card states it. List as many as the topic \
         really holds, at least one; name each idea in a short phrase.\n\n\
         Output ONLY JSON in exactly this shape, no prose, no code fences:\n\
         {{\"ideas\": [\"...\", \"...\"]}}"
    );
    push_extra(&mut prompt, cfg);
    prompt
}

fn push_extra(prompt: &mut String, cfg: &ExamConfig) {
    if let Some(extra) = cfg
        .extra
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        prompt.push_str("\n\nAdditional instructions:\n");
        prompt.push_str(extra);
    }
}

pub fn questions_prompt(
    topic: &ExamTopic,
    sources: &[SourceText],
    ideas: &[String],
    cfg: &ExamConfig,
) -> String {
    let sources = source_section(sources);
    let mut scope = String::from(
        "The student drilled flashcards on this material. The topic's cards are SCOPE \
         ONLY: they show what the student already drilled. NEVER ask what a card states, \
         and never rephrase a card; test the mental model behind the topic instead.\n\n",
    );
    scope.push_str(&topic_scope(topic));
    scope.push_str("The ideas this exam examines, numbered:\n");
    for (n, idea) in ideas.iter().enumerate() {
        scope.push_str(&format!("{}. {idea}\n", n + 1));
    }
    scope.push('\n');
    let ask = format!(
        "{scope}Write EXACTLY one question for every idea listed, no more. An idea a \
         card states is still examined: ask the student to apply it, never to recite the \
         card. Start with an instance and then take the three shapes in turn:\n\
         - instance: a concrete case the sources support but no card shows \
         (\"given this program, what does the compiler report?\");\n\
         - boundary: when does the topic's idea stop holding, or what changes \
         if a condition flips;\n\
         - fix: a short statement or snippet with exactly one seeded mistake about \
         the topic's idea; the student must find and correct it.\n\
         Every question must be answerable from the sources above and demand \
         reasoning in the student's own words. Tag every question with its shape and \
         the number of the idea it tests.\n\n"
    );
    let shape = "{\"questions\": [{\"shape\": \"instance|boundary|fix\", \
                 \"idea\": 1, \"prompt\": \"...\", \
                 \"points\": [\"...\", \"...\"]}]}";
    let mut prompt = format!(
        "You are a strict examiner writing an oral exam that verifies a student \
         truly UNDERSTANDS a topic, not whether they memorized isolated facts.\n\n\
         {sources}\n\
         {ask}\
         {QUESTION_FORM}\
         For each question, list the key points a correct answer MUST contain: \
         the specific, source-grounded facts or reasoning steps you will grade \
         against. Be concrete; these are the rubric.\n\n\
         Output ONLY JSON in exactly this shape, no prose, no code fences around \
         it (a fence inside a `prompt` string is fine):\n\
         {shape}"
    );
    push_extra(&mut prompt, cfg);
    prompt
}

const QUESTION_FORM: &str = "\
How each question reads:
- It stands alone. The student sees only the question, never the source: \
reproduce any example, listing or claim you build on in full. Never point at \
what the student cannot see (\"the book's shadowing example\", \"the \
Reference shows\", \"the idiom's form\"); naming the source is fine only \
beside the reproduced content, never instead of it.
- Code goes in a fenced block (```rust and so on) after the prose, one \
statement per line, so the student can refer to its lines; a quoted claim \
goes in a `>` quote. Inline backticks only for a short name or expression \
inside a sentence.
- Write only the finished question: no thinking aloud and no self-correction \
(\"wait\", \"actually\") in the question or in a claim it quotes.
- Never give the answer away. A fix question names neither the mistake, nor \
its line, nor the rule it breaks; any question avoids wording that states \
what the student must work out.

";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_backend_carries_the_probed_prompt_limit_and_measures_in_its_unit() {
        let tokens = |bytes_per_token_hundredths, overhead| Measure::EstimatedTokens {
            bytes_per_token_hundredths,
            overhead,
        };
        let copilot = PromptLimit {
            measure: tokens(250, 2_200),
            limit: 128_000,
        };
        type Row = (BackendKind, PromptLimit, &'static [(&'static str, u64)]);
        let rows: [Row; 4] = [
            (
                BackendKind::Codex,
                PromptLimit {
                    measure: Measure::Characters,
                    limit: 1_048_576,
                },
                &[("", 0), ("héllo", 5), ("日本語\n", 4)],
            ),
            (
                BackendKind::Copilot,
                copilot,
                &[("", 2_200), ("x", 2_201), ("x", 2_201)],
            ),
            (
                BackendKind::Claude,
                PromptLimit {
                    measure: tokens(251, 0),
                    limit: 1_000_000,
                },
                &[("", 0), ("x", 1)],
            ),
            (BackendKind::Gemini, copilot, &[("", 2_200), ("x", 2_201)]),
        ];
        for (kind, limit, samples) in rows {
            assert_eq!(limit, prompt_limit(kind), "{kind:?}: limit");
            for (prompt, size) in samples {
                assert_eq!(
                    *size,
                    TopicMeasure::of(kind, prompt).size,
                    "{kind:?}: {prompt:?}"
                );
            }
        }
        let ascii = |n: usize| "a".repeat(n);
        for (ratio, overhead, bytes, size) in [
            (250, 2_200, 250, 2_300),
            (250, 2_200, 251, 2_301),
            (250, 2_200, 500, 2_400),
            (250, 2_200, 501, 2_401),
            (250, 2_200, 314_500, 128_000),
            (250, 2_200, 314_501, 128_001),
            (251, 0, 251, 100),
            (251, 0, 252, 101),
            (251, 0, 502, 200),
            (251, 0, 503, 201),
        ] {
            assert_eq!(
                size,
                tokens(ratio, overhead).of(&ascii(bytes)),
                "{bytes} bytes at {ratio} hundredths plus {overhead} round up"
            );
        }
        assert_eq!("characters", Measure::Characters.label());
        assert_eq!("estimated tokens", tokens(1, 0).label());
    }

    #[test]
    fn a_retry_carries_at_most_the_capped_reason_cut_on_a_char_boundary() {
        let reason = "é".repeat(RETRY_REASON_MAX_CHARS + 1);
        let retry = retry_prompt("ask", &reason);
        assert_eq!(
            format!(
                "ask\n\nYour previous answer was rejected: {}. Write the whole answer again and \
                 follow every rule above.",
                "é".repeat(RETRY_REASON_MAX_CHARS)
            ),
            retry
        );
        assert_eq!("ab", truncate_chars("abc", 2));
        assert_eq!("abc", truncate_chars("abc", 3));
        assert_eq!("日本", truncate_chars("日本語", 2));
    }
}
