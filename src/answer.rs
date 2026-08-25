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
    // Fewer inputs than expected lines: every unclaimed line still owes a
    // result, or a card is passed by answering part of it.
    for (i, exp) in expected.iter().enumerate() {
        if !claimed[i] {
            results.push(grade_typed("", exp));
        }
    }
    results
}

/// `graded[i] == false` marks a position the card does not grade. It still
/// owes a result, so a caller pairing by position keeps one entry per line.
pub fn grade_lines_ordered(
    inputs: &[String],
    expected: &[String],
    graded: &[bool],
) -> Vec<TypedResult> {
    (0..inputs.len().max(expected.len()))
        .map(|i| {
            let input = inputs.get(i).map_or("", String::as_str);
            if graded.get(i).copied().unwrap_or(true) {
                grade_typed(input, expected.get(i).map_or("", String::as_str))
            } else {
                TypedResult {
                    input: input.trim().to_string(),
                    expected: String::new(),
                    passed: true,
                }
            }
        })
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
    fn unordered_grading_fails_an_expected_line_nobody_answered() {
        let expected = lines(&["scaphoid", "lunate"]);
        let results = grade_lines_unordered(&lines(&["scaphoid"]), &expected);
        assert_eq!(
            expected.len(),
            results.len(),
            "every expected line owes a result"
        );
        assert!(
            !results.iter().all(|result| result.passed),
            "one line cannot satisfy a two-answer card"
        );
        let missed = results.iter().find(|r| !r.passed).expect("a failed result");
        assert_eq!("lunate", missed.expected, "the unanswered line is named");
        assert_eq!("", missed.input, "with no input against it");
    }

    #[test]
    fn ordered_grading_fails_an_expected_line_nobody_answered() {
        let expected = lines(&["one", "two", "three"]);
        let results = grade_lines_ordered(&lines(&["one"]), &expected, &[]);
        assert_eq!(expected.len(), results.len());
        assert!(!results.iter().all(|result| result.passed));
        assert_eq!("two", results[1].expected);
        assert_eq!("three", results[2].expected);
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
        let swapped = grade_lines_ordered(&lines(&["green", "red"]), &expected, &[]);
        assert!(!swapped[0].passed, "green vs red");
        assert!(!swapped[1].passed, "red vs green");
        assert_eq!("red", swapped[0].expected);
        assert_eq!("green", swapped[1].expected);
        let in_order = grade_lines_ordered(&expected, &expected, &[]);
        assert!(in_order.iter().all(|r| r.passed));
        let extra = grade_lines_ordered(&lines(&["red", "green", "blue"]), &expected, &[]);
        assert!(!extra[2].passed);
        assert_eq!("", extra[2].expected);
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
