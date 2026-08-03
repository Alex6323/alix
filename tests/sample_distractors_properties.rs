use std::{collections::BTreeSet, process::Command};

use alix::choice::sample::sample_distractors;
use proptest::prelude::*;

const PROCESS_SNAPSHOT_ENV: &str = "ALIX_SAMPLE_DISTRACTORS_PROCESS_SNAPSHOT";
const PROCESS_SNAPSHOT_MARKER: &str = "ALIX_SAMPLE_DISTRACTORS_OUTPUT=";

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn distinct_non_correct<'a>(correct: &str, pool: &'a [String]) -> BTreeSet<&'a str> {
    pool.iter()
        .map(String::as_str)
        .filter(|candidate| *candidate != correct)
        .collect()
}

fn deduplicate_preserving_first_occurrence(pool: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduplicated = Vec::new();

    for candidate in pool {
        if seen.insert(candidate.as_str()) {
            deduplicated.push(candidate.clone());
        }
    }

    deduplicated
}

fn is_ascii_digit_string_of_len(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn numeric_shape_case() -> impl Strategy<Value = (String, Vec<String>, u64, usize)> {
    (
        1_usize..=6,
        any::<u64>(),
        any::<u64>(),
        1_usize..=8,
        proptest::collection::vec(any::<String>(), 0..16),
    )
        .prop_map(|(len, raw_correct, seed, raw_want, mut noise)| {
            let modulus = 10_u64.pow(len as u32);
            let correct_number = raw_correct % modulus;
            let correct = format!("{correct_number:0len$}");
            let maximum_want = usize::try_from((modulus - 1).min(8)).unwrap();
            let want = 1 + (raw_want - 1) % maximum_want;

            noise.extend([
                "1.5".to_owned(),
                "not a number".to_owned(),
                "\u{0661}\u{0662}\u{0663}".to_owned(),
                "9".repeat(len + 1),
            ]);

            for offset in 1..=maximum_want as u64 {
                let candidate_number = (correct_number + offset) % modulus;
                let candidate = format!("{candidate_number:0len$}");
                noise.push(candidate.clone());
                noise.push(candidate);
            }

            (correct, noise, seed, want)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn returned_values_obey_size_exclusion_membership_distinctness_and_capacity(
        correct in any::<String>(),
        pool in proptest::collection::vec(any::<String>(), 0..24),
        seed in any::<u64>(),
        want in 0_usize..28,
    ) {
        let available = distinct_non_correct(&correct, &pool);
        let result = sample_distractors(&correct, &pool, seed, want);

        match result {
            Some(chosen) => {
                prop_assert_eq!(chosen.len(), want);
                prop_assert!(available.len() >= want);
                prop_assert!(chosen.iter().all(|candidate| candidate != &correct));
                prop_assert!(chosen.iter().all(|candidate| pool.contains(candidate)));

                let distinct_chosen: BTreeSet<&str> =
                    chosen.iter().map(String::as_str).collect();
                prop_assert_eq!(distinct_chosen.len(), chosen.len());
            }
            None => prop_assert!(available.len() < want),
        }
    }

    #[test]
    fn equal_inputs_return_equal_ordered_outputs_across_calls(
        correct in any::<String>(),
        pool in proptest::collection::vec(any::<String>(), 0..24),
        seed in any::<u64>(),
        want in 0_usize..28,
    ) {
        let first = sample_distractors(&correct, &pool, seed, want);
        let second = sample_distractors(&correct, &pool, seed, want);

        prop_assert_eq!(first, second);
    }

    #[test]
    fn duplicate_entries_have_the_same_effect_as_one_occurrence(
        correct in any::<String>(),
        pool in proptest::collection::vec(any::<String>(), 0..24),
        seed in any::<u64>(),
        want in 0_usize..28,
    ) {
        let deduplicated = deduplicate_preserving_first_occurrence(&pool);

        prop_assert_eq!(
            sample_distractors(&correct, &pool, seed, want),
            sample_distractors(&correct, &deduplicated, seed, want),
        );
    }

    #[test]
    fn enough_same_length_ascii_digit_candidates_exclude_every_other_shape(
        (correct, pool, seed, want) in numeric_shape_case(),
    ) {
        let chosen = sample_distractors(&correct, &pool, seed, want)
            .expect("the generated pool always has enough matching numeric candidates");

        prop_assert_eq!(chosen.len(), want);
        prop_assert!(
            chosen
                .iter()
                .all(|candidate| is_ascii_digit_string_of_len(candidate, correct.len()))
        );
    }
}

#[test]
fn zero_wanted_returns_an_empty_vector_even_when_the_pool_is_empty() {
    assert_eq!(sample_distractors("correct", &[], 17, 0), Some(vec![]));
}

#[test]
fn duplicate_candidates_cannot_satisfy_a_request_for_distinct_items() {
    let pool = strings(&["one", "one", "correct", "correct"]);

    assert_eq!(sample_distractors("correct", &pool, 17, 2), None);
}

#[test]
fn exclusion_uses_exact_equality_without_trimming_or_case_folding() {
    let pool = strings(&["answer", " answer", "answer ", "Answer"]);
    let chosen = sample_distractors("answer", &pool, 17, 3)
        .expect("the three exact non-matches are distinct candidates");
    let chosen: BTreeSet<String> = chosen.into_iter().collect();
    let expected: BTreeSet<String> = strings(&[" answer", "answer ", "Answer"])
        .into_iter()
        .collect();

    assert_eq!(chosen, expected);
}

#[test]
fn exclusion_does_not_apply_unicode_normalization() {
    let pool = strings(&["\u{00e9}", "e\u{0301}"]);

    assert_eq!(
        sample_distractors("\u{00e9}", &pool, 17, 1),
        Some(strings(&["e\u{0301}"])),
    );
}

#[test]
fn numeric_shape_is_ascii_only_and_does_not_include_decimals_or_wrong_lengths() {
    let pool = strings(&[
        "014",
        "016",
        "999",
        "014",
        "15",
        "0015",
        "1.5",
        "\u{0660}\u{0661}\u{0665}",
        "015",
    ]);
    let chosen = sample_distractors("015", &pool, 17, 3)
        .expect("there are exactly three distinct candidates with the required numeric shape");
    let chosen: BTreeSet<String> = chosen.into_iter().collect();
    let expected: BTreeSet<String> = strings(&["014", "016", "999"]).into_iter().collect();

    assert_eq!(chosen, expected);
}

#[test]
fn equal_inputs_return_equal_ordered_outputs_across_processes() {
    let first = sample_output_in_fresh_process();

    for _ in 0..3 {
        assert_eq!(sample_output_in_fresh_process(), first);
    }
}

fn sample_output_in_fresh_process() -> String {
    let output = Command::new(std::env::current_exe().expect("the test executable has a path"))
        .args(["--exact", "process_snapshot_helper", "--nocapture"])
        .env(PROCESS_SNAPSHOT_ENV, "1")
        .output()
        .expect("the test executable can be launched as a child process");

    assert!(
        output.status.success(),
        "child test process failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("the test harness emits UTF-8");
    let marker_start = stdout
        .find(PROCESS_SNAPSHOT_MARKER)
        .expect("the child test process emitted its sample snapshot");

    stdout[marker_start..]
        .lines()
        .next()
        .expect("the sample snapshot occupies one line")
        .to_owned()
}

#[test]
fn process_snapshot_helper() {
    if std::env::var_os(PROCESS_SNAPSHOT_ENV).is_none() {
        return;
    }

    let pool = strings(&[
        "mercury", "venus", "earth", "mars", "jupiter", "saturn", "uranus", "neptune", "ceres",
        "eris", "haumea", "makemake",
    ]);
    let chosen = sample_distractors("pluto", &pool, 0x5eed, 7);

    println!("{PROCESS_SNAPSHOT_MARKER}{chosen:?}");
}
