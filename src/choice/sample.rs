use std::collections::HashSet;

use super::{Rng, shuffle};

/// Deterministic distractor pick: `want` items from `pool` for
/// `correct`. `None` when the deduplicated pool cannot supply
/// `want` distinct candidates.
pub fn sample_distractors(
    correct: &str,
    pool: &[String],
    seed: u64,
    want: usize,
) -> Option<Vec<String>> {
    let mut candidates = distinct_candidates(correct, pool);
    if candidates.len() < want {
        return None;
    }
    if let Some(digits) = digit_len(correct) {
        let same_length: Vec<&str> = candidates
            .iter()
            .copied()
            .filter(|candidate| digit_len(candidate) == Some(digits))
            .collect();
        if same_length.len() >= want {
            candidates = same_length;
        }
    }
    candidates.sort_by_key(|candidate| strsim::levenshtein(correct, candidate));
    candidates.truncate(want * 2);
    shuffle(&mut candidates, &mut Rng::new(seed));
    candidates.truncate(want);
    Some(candidates.into_iter().map(str::to_owned).collect())
}

fn distinct_candidates<'pool>(correct: &str, pool: &'pool [String]) -> Vec<&'pool str> {
    let mut seen: HashSet<&str> = HashSet::from([correct]);
    pool.iter()
        .filter(|candidate| seen.insert(candidate.as_str()))
        .map(String::as_str)
        .collect()
}

fn digit_len(text: &str) -> Option<usize> {
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())).then_some(text.len())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn pool(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn is_digits_of_len(text: &str, len: usize) -> bool {
        text.len() == len && !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
    }

    struct Scenario {
        name: &'static str,
        correct: &'static str,
        candidates: &'static [&'static str],
        want: usize,
    }

    const SCENARIOS: &[Scenario] = &[
        Scenario {
            name: "a wide word pool",
            correct: "alpha",
            candidates: &["beta", "gamma", "delta", "epsilon", "zeta"],
            want: 3,
        },
        Scenario {
            name: "the correct answer repeated in the pool",
            correct: "alpha",
            candidates: &["alpha", "beta", "alpha", "gamma", "delta", "alpha"],
            want: 3,
        },
        Scenario {
            name: "duplicated candidates",
            correct: "alpha",
            candidates: &["beta", "beta", "gamma", "gamma", "delta", "delta"],
            want: 3,
        },
        Scenario {
            name: "exactly as many candidates as wanted",
            correct: "alpha",
            candidates: &["beta", "gamma", "delta"],
            want: 3,
        },
        Scenario {
            name: "digit answers among words",
            correct: "1158",
            candidates: &["1240", "1632", "1806", "1918", "Isar", "1,5"],
            want: 3,
        },
        Scenario {
            name: "an empty candidate is an ordinary candidate",
            correct: "alpha",
            candidates: &["", "beta", "gamma"],
            want: 3,
        },
        Scenario {
            name: "a single wanted distractor",
            correct: "alpha",
            candidates: &["beta", "gamma"],
            want: 1,
        },
        Scenario {
            name: "multi-line and non-ascii answers",
            correct: "line a\nline b",
            candidates: &["Löwe", "x\ny", "z"],
            want: 2,
        },
    ];

    #[test]
    fn a_pick_is_wanted_sized_correct_free_and_drawn_distinctly_from_the_pool() {
        for scenario in SCENARIOS {
            let candidates = pool(scenario.candidates);
            for seed in 0..16 {
                let picks = sample_distractors(scenario.correct, &candidates, seed, scenario.want)
                    .unwrap_or_else(|| panic!("{}: seed {seed} yielded no pick", scenario.name));
                assert_eq!(
                    scenario.want,
                    picks.len(),
                    "{}: seed {seed} returned {picks:?}, wanted {} items",
                    scenario.name,
                    scenario.want
                );
                assert!(
                    !picks.iter().any(|pick| pick == scenario.correct),
                    "{}: seed {seed} returned the correct answer {:?} in {picks:?}",
                    scenario.name,
                    scenario.correct
                );
                for pick in &picks {
                    assert!(
                        candidates.contains(pick),
                        "{}: seed {seed} returned {pick:?}, which is not in the pool",
                        scenario.name
                    );
                }
                let distinct: BTreeSet<&String> = picks.iter().collect();
                assert_eq!(
                    picks.len(),
                    distinct.len(),
                    "{}: seed {seed} repeated an item in {picks:?}",
                    scenario.name
                );
            }
        }
    }

    #[test]
    fn a_pool_short_of_distinct_candidates_yields_none() {
        let candidates = pool(&["beta", "beta", "alpha", "gamma", "alpha"]);
        assert_eq!(
            None,
            sample_distractors("alpha", &candidates, 7, 3),
            "two distinct candidates must not be padded into three"
        );
        assert_eq!(
            Some(2),
            sample_distractors("alpha", &candidates, 7, 2).map(|picks| picks.len()),
            "two distinct candidates must still satisfy a want of two"
        );
    }

    #[test]
    fn an_empty_pool_yields_none_unless_nothing_is_wanted() {
        assert_eq!(None, sample_distractors("alpha", &[], 1, 1));
        assert_eq!(Some(Vec::new()), sample_distractors("alpha", &[], 1, 0));
    }

    #[test]
    fn wanting_no_distractors_yields_an_empty_pick() {
        let candidates = pool(&["beta", "gamma", "delta"]);
        assert_eq!(
            Some(Vec::new()),
            sample_distractors("alpha", &candidates, 3, 0)
        );
    }

    #[test]
    fn the_same_inputs_produce_the_same_pick() {
        let candidates = pool(&["beta", "gamma", "delta", "epsilon", "zeta", "eta"]);
        let first = sample_distractors("alpha", &candidates, 7, 3).unwrap();
        for repeat in 0..8 {
            assert_eq!(
                first,
                sample_distractors("alpha", &candidates, 7, 3).unwrap(),
                "call {repeat} differed from the first pick"
            );
        }
    }

    #[test]
    fn a_fixed_input_pins_a_fixed_pick_across_processes() {
        let candidates = pool(&["beta", "gamma", "delta", "epsilon", "zeta", "eta"]);
        assert_eq!(
            pool(&["gamma", "epsilon", "beta"]),
            sample_distractors("alpha", &candidates, 7, 3).unwrap(),
            "the pick drifted; ambient state (hash-map order) must never reach it"
        );
    }

    #[test]
    fn different_seeds_reach_different_picks() {
        let candidates = pool(&["beta", "gamma", "delta", "epsilon", "zeta", "eta"]);
        let picks: BTreeSet<Vec<String>> = (0..16)
            .filter_map(|seed| sample_distractors("alpha", &candidates, seed, 3))
            .collect();
        assert!(
            picks.len() > 1,
            "the pick never varied across seeds: {picks:?}"
        );
    }

    #[test]
    fn a_digit_answer_only_competes_with_digit_answers_of_its_own_length() {
        let pools = [
            (
                "exactly as many four-digit candidates as wanted",
                pool(&[
                    "1240", "1632", "1806", "158", "12345", "1,5", "Isar", "1158",
                ]),
            ),
            (
                "more four-digit candidates than wanted",
                pool(&[
                    "1240", "1632", "1806", "1918", "158", "12345", "1,5", "Isar", "1158",
                ]),
            ),
        ];
        for (name, candidates) in &pools {
            for seed in 0..32 {
                let picks = sample_distractors("1158", candidates, seed, 3).unwrap();
                for pick in &picks {
                    assert!(
                        is_digits_of_len(pick, 4),
                        "{name}: seed {seed} picked {pick:?} against a four-digit answer"
                    );
                }
            }
        }
    }

    #[test]
    fn a_digit_answer_falls_back_to_any_candidate_when_too_few_share_its_length() {
        let candidates = pool(&["1789", "Isar", "Marienplatz", "1,5"]);
        for seed in 0..32 {
            let picks = sample_distractors("1158", &candidates, seed, 3).unwrap();
            assert_eq!(3, picks.len(), "seed {seed} returned {picks:?}");
            assert!(
                picks.iter().any(|pick| !is_digits_of_len(pick, 4)),
                "seed {seed} conjured four-digit candidates that the pool lacks: {picks:?}"
            );
        }
    }

    #[test]
    fn only_the_candidates_closest_to_the_answer_are_reachable() {
        let near = ["alphx", "alphy", "alphz", "alpha1", "alpha2", "alpha3"];
        let far = [
            "quebec-tango",
            "zulu-kilo-niner",
            "sierra-victor",
            "romeo-foxtrot",
            "hotel-india",
        ];
        let candidates = pool(&[near.as_slice(), far.as_slice()].concat());
        let reached: BTreeSet<String> = (0..64)
            .flat_map(|seed| sample_distractors("alpha", &candidates, seed, 3).unwrap())
            .collect();
        let expected: BTreeSet<String> = pool(&near).into_iter().collect();
        assert_eq!(
            expected, reached,
            "the reachable candidates are not exactly the six nearest"
        );
    }
}
