use std::cmp::Ordering;

// "To be" and pronouns are intentionally absent: `Is`/`Its` stay capitalized.
const MINOR_WORDS: &[&str] = &[
    "a", "an", "the", "and", "but", "or", "nor", "for", "of", "to", "in", "on", "at", "by", "as",
    "per", "via", "with", "from", "into", "onto", "vs",
];

const MAX_TITLE_WORDS: usize = 12;

/// Orders display titles by the numeric value of adjacent ASCII digit runs.
///
/// Comparing digit runs by length and bytes avoids integer overflow, including
/// for generated or imported titles with identifiers wider than `u128`.
pub(crate) fn natural_cmp(left: &str, right: &str) -> Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let (mut li, mut ri) = (0, 0);

    while li < left.len() && ri < right.len() {
        if left[li].is_ascii_digit() && right[ri].is_ascii_digit() {
            let le = li + left[li..].iter().take_while(|b| b.is_ascii_digit()).count();
            let re = ri
                + right[ri..]
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .count();
            let left_digits = &left[li..le];
            let right_digits = &right[ri..re];
            let left_number = left_digits
                .iter()
                .position(|b| *b != b'0')
                .map_or(&left_digits[left_digits.len()..], |start| {
                    &left_digits[start..]
                });
            let right_number = right_digits
                .iter()
                .position(|b| *b != b'0')
                .map_or(&right_digits[right_digits.len()..], |start| {
                    &right_digits[start..]
                });

            let number_order = left_number
                .len()
                .cmp(&right_number.len())
                .then_with(|| left_number.cmp(right_number))
                .then_with(|| left_digits.len().cmp(&right_digits.len()));
            if number_order != Ordering::Equal {
                return number_order;
            }
            li = le;
            ri = re;
            continue;
        }

        let byte_order = left[li].cmp(&right[ri]);
        if byte_order != Ordering::Equal {
            return byte_order;
        }
        li += 1;
        ri += 1;
    }

    left.len().cmp(&right.len())
}

pub fn condense(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(i) = s.find([':', ';', '—', '–']) {
        s = s[..i].trim_end();
    }
    if let Some(i) = s.find(" (") {
        s = s[..i].trim_end();
    }
    let words: Vec<&str> = s.split_whitespace().collect();
    let truncated = words.len() > MAX_TITLE_WORDS;
    let kept = if truncated {
        &words[..MAX_TITLE_WORDS]
    } else {
        &words[..]
    };
    let last = kept.len().saturating_sub(1);
    let mut out = kept
        .iter()
        .enumerate()
        .map(|(i, w)| title_word(w, i == 0 || i == last))
        .collect::<Vec<_>>()
        .join(" ");
    if truncated {
        out.push('…');
    }
    out
}

fn title_word(w: &str, force_cap: bool) -> String {
    if is_code_token(w) {
        return w.to_string();
    }
    let lower = w.to_lowercase();
    if !force_cap && MINOR_WORDS.contains(&lower.as_str()) {
        return lower;
    }
    w.split_inclusive(['-', '/'])
        .map(capitalize_first)
        .collect()
}

fn is_code_token(w: &str) -> bool {
    w.contains('`')
        || w.contains('_')
        || w.contains('(')
        // skip(1): a leading capital doesn't count, only one after the first char does.
        || w.chars().skip(1).any(|c| c.is_ascii_uppercase())
}

fn capitalize_first(seg: &str) -> String {
    let mut chars = seg.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_the_enumeration_and_title_cases() {
        assert_eq!(
            "What the Crate Is and Its Public Surface",
            condense(
                "what the crate is and its public surface: library role, the \
                 three-part Store/Execute/Inspect model, the features"
            )
        );
        assert_eq!(
            "The Object-Store Data Model",
            condense("the object-store data model: the four-method `Store` trait")
        );
    }

    #[test]
    fn leaves_code_spans_untouched() {
        assert_eq!(
            "How a `TransactionData` Becomes an `ExecutionResult`",
            condense("how a `TransactionData` becomes an `ExecutionResult`: the spine")
        );
        assert_eq!(
            "The `grpc`/`graphql`/`tracing` Features",
            condense("the `grpc`/`graphql`/`tracing` features")
        );
        assert_eq!(
            "How the VM Reads execute_signed",
            condense("how the VM reads execute_signed")
        );
        assert_eq!(
            "The iOS Build Reads foo()",
            condense("the iOS build reads foo()"),
            "an inner capital and a call paren each mark a code token on their own"
        );
    }

    #[test]
    fn word_caps_when_there_is_no_separator() {
        let raw = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi";
        let out = condense(raw);
        assert!(out.ends_with('…'), "{out}");
        assert_eq!(12, out.trim_end_matches('…').split_whitespace().count());
        assert!(out.starts_with("Alpha Beta Gamma"), "{out}");
    }

    #[test]
    fn drops_a_trailing_parenthetical() {
        assert_eq!(
            "The Typed Error Surface",
            condense("the typed error surface (validation, store, execution)")
        );
    }

    #[test]
    fn natural_order_compares_numeric_runs_without_parsing_them() {
        let mut titles = [
            "11. Blocking Frontend Static Analysis",
            "100. Private, Reviewable Bug Reports",
            "10. PDF-Native Sourcing",
            "999999999999999999999999999999. Wide Identifier",
            "02. Padded",
            "2. Plain",
        ];

        titles.sort_by(|left, right| natural_cmp(left, right));

        assert_eq!(
            [
                "2. Plain",
                "02. Padded",
                "10. PDF-Native Sourcing",
                "11. Blocking Frontend Static Analysis",
                "100. Private, Reviewable Bug Reports",
                "999999999999999999999999999999. Wide Identifier",
            ],
            titles
        );
        assert_eq!(Ordering::Less, natural_cmp("ch1-part9", "ch1-part10"));
        assert_eq!(Ordering::Less, natural_cmp("7. Plain", "007. Padded"));
        assert_eq!(Ordering::Less, natural_cmp("7a", "7b"));
        assert_eq!(Ordering::Less, natural_cmp("10. ASCII", "１０. Fullwidth"));
    }

    #[test]
    fn natural_order_is_total_at_equal_and_prefix_boundaries() {
        assert_eq!(Ordering::Equal, natural_cmp("", ""));
        assert_eq!(Ordering::Equal, natural_cmp("alpha", "alpha"));
        assert_eq!(Ordering::Less, natural_cmp("alpha", "alphabet"));
        assert_eq!(Ordering::Greater, natural_cmp("alphabet", "alpha"));
    }

    #[test]
    fn title_word_cap_truncates_only_after_the_twelfth_word() {
        let twelve = "one two three four five six seven eight nine ten eleven twelve";
        let thirteen = format!("{twelve} thirteen");

        assert!(!condense(twelve).ends_with('…'));
        assert!(condense(&thirteen).ends_with('…'));
    }
}
