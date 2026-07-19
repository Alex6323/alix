#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
pub enum Mode {
    #[default]
    Flip,
    Typing,
    #[cfg_attr(feature = "full", value(name = "typeline"))]
    #[serde(rename = "typeline")]
    TypeLine,
    Choice,
    #[cfg_attr(feature = "full", value(name = "line"))]
    LineByLine,
    Explain,
}

impl Mode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "flip" => Some(Self::Flip),
            "typing" => Some(Self::Typing),
            "typeline" => Some(Self::TypeLine),
            "choice" => Some(Self::Choice),
            "line" => Some(Self::LineByLine),
            "explain" => Some(Self::Explain),
            _ => None,
        }
    }
}

#[cfg(feature = "full")]
pub(crate) fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Flip => "flip",
        Mode::Typing => "typing",
        Mode::TypeLine => "typeline",
        Mode::Choice => "choice",
        Mode::LineByLine => "line",
        Mode::Explain => "explain",
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
pub enum Input {
    #[default]
    #[cfg_attr(feature = "full", value(name = "type"))]
    Type,
    #[cfg_attr(feature = "full", value(name = "draw"))]
    Draw,
}

impl Input {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "type" => Some(Self::Type),
            "draw" => Some(Self::Draw),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Typed {
    pub ch: char,
    pub correct: bool,
}

#[derive(Clone, Debug)]
pub struct TypingValidator {
    expected: Vec<char>,
    typed: Vec<Typed>,
    hints_used: usize,
    typos: usize,
    hint_revealed: usize,
}

pub const HINT_CHARS: usize = 2;

impl TypingValidator {
    pub fn new(expected: &str) -> Self {
        Self {
            expected: expected.chars().collect(),
            typed: Vec::new(),
            hints_used: 0,
            typos: 0,
            hint_revealed: 0,
        }
    }

    pub fn expected(&self) -> String {
        self.expected.iter().collect()
    }

    pub fn typed(&self) -> &[Typed] {
        &self.typed
    }

    pub fn type_char(&mut self, ch: char) -> bool {
        self.hint_revealed = 0;
        if self.typed.len() >= self.expected.len() {
            return false;
        }
        let correct = self.expected[self.typed.len()] == ch;
        if !correct {
            self.typos += 1;
        }
        self.typed.push(Typed { ch, correct });
        correct
    }

    pub fn backspace(&mut self) -> bool {
        self.hint_revealed = 0;
        self.typed.pop().is_some()
    }

    pub fn hint(&mut self) -> String {
        self.hints_used += 1;
        if let Some(first_bad) = self.typed.iter().position(|t| !t.correct) {
            self.typed.truncate(first_bad);
        }
        let remaining = self.expected.len() - self.typed.len();
        self.hint_revealed = (self.hint_revealed + HINT_CHARS).min(remaining);
        self.expected
            .iter()
            .skip(self.typed.len())
            .take(self.hint_revealed)
            .collect()
    }

    pub fn set_expected(&mut self, expected: &str) {
        self.expected = expected.chars().collect();
        for (i, t) in self.typed.iter_mut().enumerate() {
            t.correct = self.expected.get(i) == Some(&t.ch);
        }
    }

    pub fn remaining(&self) -> String {
        self.expected.iter().skip(self.typed.len()).collect()
    }

    pub fn is_complete(&self) -> bool {
        self.typed.len() == self.expected.len() && self.typed.iter().all(|t| t.correct)
    }

    pub fn passed(&self) -> bool {
        self.is_complete() && self.hints_used == 0
    }

    pub fn hints_used(&self) -> usize {
        self.hints_used
    }

    pub fn typos(&self) -> usize {
        self.typos
    }
}

pub fn normalize_answer(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['.', ',', ';', ':', '!', '?'])
        .to_lowercase()
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TypedResult {
    pub input: String,
    pub expected: String,
    pub passed: bool,
}

/// No edit-distance tolerance here: a typo and a wrong answer both fail,
/// and it's the learner who decides which was which.
pub fn grade_typed(input: &str, expected: &str) -> TypedResult {
    TypedResult {
        input: input.trim().to_string(),
        expected: expected.to_string(),
        passed: normalize_answer(input) == normalize_answer(expected),
    }
}

/// The Levenshtein distance here only decides pairing (which expected line an
/// input maps to), not pass/fail tolerance.
pub fn grade_lines_unordered(inputs: &[String], expected: &[String]) -> Vec<TypedResult> {
    let mut claimed = vec![false; expected.len()];
    let mut results = Vec::with_capacity(inputs.len());
    for input in inputs {
        let normalized_input = normalize_answer(input);
        let best = expected
            .iter()
            .enumerate()
            .filter(|(i, _)| !claimed[*i])
            .map(|(i, exp)| {
                (
                    i,
                    strsim::levenshtein(&normalized_input, &normalize_answer(exp)),
                )
            })
            .min_by_key(|(_, distance)| *distance);
        match best {
            Some((i, _)) => {
                claimed[i] = true;
                results.push(grade_typed(input, &expected[i]));
            }
            // More inputs than expected lines: an extra input matches nothing.
            None => results.push(grade_typed(input, "")),
        }
    }
    results
}

pub fn grade_lines_ordered(inputs: &[String], expected: &[String]) -> Vec<TypedResult> {
    inputs
        .iter()
        .enumerate()
        .map(|(i, input)| grade_typed(input, expected.get(i).map_or("", String::as_str)))
        .collect()
}

pub fn best_prefix_match(typed: &str, candidates: &[&str]) -> Option<usize> {
    (0..candidates.len()).min_by_key(|&i| {
        let cand = candidates[i];
        let shared = typed
            .chars()
            .zip(cand.chars())
            .take_while(|(a, b)| a == b)
            .count();
        (std::cmp::Reverse(shared), cand.chars().count(), i)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_parses_its_value_names_and_defaults_to_type() {
        assert_eq!(Some(Input::Draw), Input::parse("draw"));
        assert_eq!(Some(Input::Type), Input::parse("TYPE"));
        assert_eq!(None, Input::parse("scribble"));
        assert_eq!(Input::Type, Input::default());
    }

    #[test]
    fn typing_correct_line_passes() {
        let mut v = TypingValidator::new("hello");
        for c in "hello".chars() {
            assert!(v.type_char(c));
        }
        assert!(v.is_complete());
        assert!(v.passed());
        assert_eq!(0, v.typos());
    }

    #[test]
    fn typing_incomplete_line_is_not_complete() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        assert!(!v.is_complete());
        assert_eq!("ello", v.remaining());
    }

    #[test]
    fn typing_wrong_char_blocks_completion_until_fixed() {
        let mut v = TypingValidator::new("hello");
        assert!(v.type_char('h'));
        assert!(!v.type_char('3'));
        v.type_char('l');
        v.type_char('l');
        v.type_char('o');
        assert!(!v.is_complete());

        for _ in 0..4 {
            v.backspace();
        }
        for c in "ello".chars() {
            assert!(v.type_char(c));
        }
        assert!(v.is_complete());
        assert!(v.passed());
        assert_eq!(1, v.typos());
    }

    #[test]
    fn typing_input_beyond_length_ignored() {
        let mut v = TypingValidator::new("hi");
        v.type_char('h');
        v.type_char('i');
        assert!(!v.type_char('!'));
        assert!(v.is_complete());
        assert_eq!(2, v.typed().len());
    }

    #[test]
    fn backspace_on_empty_returns_false() {
        let mut v = TypingValidator::new("hi");
        assert!(!v.backspace());
    }

    #[test]
    fn hint_reveals_next_chars_and_fails_the_line() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        assert_eq!("el", v.hint());
        for c in "ello".chars() {
            v.type_char(c);
        }
        assert!(v.is_complete());
        assert!(!v.passed());
        assert_eq!(1, v.hints_used());
    }

    #[test]
    fn repeated_hints_reveal_more_until_full() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        assert_eq!("el", v.hint());
        assert_eq!("ello", v.hint());
        assert_eq!("ello", v.hint());
        v.type_char('e');
        assert_eq!("ll", v.hint());
    }

    #[test]
    fn hint_clears_incorrect_tail_first() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        v.type_char('3');
        v.type_char('l');
        assert_eq!("el", v.hint());
        assert_eq!(1, v.typed().len());
        assert_eq!("ello", v.remaining());
    }

    #[test]
    fn hint_at_end_is_empty() {
        let mut v = TypingValidator::new("hi");
        v.type_char('h');
        v.type_char('i');
        assert_eq!("", v.hint());
    }

    #[test]
    fn unicode_is_handled_per_char() {
        let mut v = TypingValidator::new("héllö");
        for c in "héllö".chars() {
            assert!(v.type_char(c));
        }
        assert!(v.passed());
    }

    #[test]
    fn grade_typed_exact_match_passes() {
        let r = grade_typed("hello", "hello");
        assert!(r.passed);
    }

    #[test]
    fn grade_typed_input_is_trimmed() {
        let r = grade_typed("  hello  ", "hello");
        assert!(r.passed);
    }

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unordered_lines_pass_in_any_order() {
        let expected = lines(&["red", "green", "blue"]);
        let inputs = lines(&["blue", "red", "green"]);
        let results = grade_lines_unordered(&inputs, &expected);
        assert!(results.iter().all(|r| r.passed));
        assert_eq!("blue", results[0].expected);
        assert_eq!("red", results[1].expected);
        assert_eq!("green", results[2].expected);
    }

    #[test]
    fn unordered_one_wrong_line_maps_to_its_nearest_expected() {
        let expected = lines(&["red", "green", "blue"]);
        let inputs = lines(&["blue", "gren", "red"]);
        let results = grade_lines_unordered(&inputs, &expected);
        assert!(results[0].passed);
        assert!(!results[1].passed);
        assert_eq!("green", results[1].expected);
        assert!(results[2].passed);
    }

    #[test]
    fn unordered_does_not_claim_one_expected_twice() {
        let expected = lines(&["aa", "ab"]);
        let inputs = lines(&["ab", "aa"]);
        let results = grade_lines_unordered(&inputs, &expected);
        let mut matched: Vec<&str> = results.iter().map(|r| r.expected.as_str()).collect();
        matched.sort_unstable();
        assert_eq!(vec!["aa", "ab"], matched);
        assert!(results.iter().all(|r| r.passed));
    }

    #[test]
    fn best_prefix_match_prefers_longest_shared_prefix() {
        let cands = ["green", "grape", "blue"];
        assert_eq!(Some(0), best_prefix_match("gre", &cands));
        assert_eq!(Some(1), best_prefix_match("gra", &cands));
        assert_eq!(Some(2), best_prefix_match("b", &cands));
        assert_eq!(Some(2), best_prefix_match("x", &cands));
        assert_eq!(None, best_prefix_match("x", &[]));
    }

    #[test]
    fn normalization_forgives_case_whitespace_and_trailing_punctuation() {
        assert!(grade_typed("  Borrow  Checker ", "borrow checker.").passed);
    }

    #[test]
    fn a_one_letter_different_word_is_not_a_typo_and_fails() {
        assert!(!grade_typed("affect", "effect").passed);
    }

    #[test]
    fn unordered_lines_pair_each_input_with_its_closest_expected_line() {
        let inputs = vec!["beta".to_string(), "alpha".to_string()];
        let expected = vec!["alpha".to_string(), "beta".to_string()];
        let results = grade_lines_unordered(&inputs, &expected);
        assert!(results.iter().all(|r| r.passed), "order must not matter");
    }

    #[test]
    fn ordered_line_grading_respects_position() {
        let expected = lines(&["red", "green"]);
        let swapped = grade_lines_ordered(&lines(&["green", "red"]), &expected);
        assert!(!swapped[0].passed, "green vs red");
        assert!(!swapped[1].passed, "red vs green");
        assert_eq!("red", swapped[0].expected);
        assert_eq!("green", swapped[1].expected);
        let in_order = grade_lines_ordered(&expected, &expected);
        assert!(in_order.iter().all(|r| r.passed));
        let extra = grade_lines_ordered(&lines(&["red", "green", "blue"]), &expected);
        assert!(!extra[2].passed);
        assert_eq!("", extra[2].expected);
    }

    #[test]
    fn set_expected_re_evaluates_typed_chars() {
        let mut v = TypingValidator::new("cat");
        for c in "car".chars() {
            v.type_char(c);
        }
        assert!(!v.is_complete());
        v.set_expected("car");
        assert!(v.is_complete());
        assert!(v.passed());
    }
}

#[cfg(all(test, feature = "full"))]
mod clap_parity {
    use clap::ValueEnum;

    use super::*;

    #[test]
    fn parse_matches_the_clap_value_names() {
        for variant in Mode::value_variants() {
            let name = variant.to_possible_value().expect("a value name");
            assert_eq!(Some(*variant), Mode::parse(name.get_name()), "{name:?}");
        }
        assert_eq!(None, Mode::parse("no-such-value"));
        for variant in Input::value_variants() {
            let name = variant.to_possible_value().expect("a value name");
            assert_eq!(Some(*variant), Input::parse(name.get_name()), "{name:?}");
        }
        assert_eq!(None, Input::parse("no-such-value"));
    }
}
