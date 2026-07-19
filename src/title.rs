// "To be" and pronouns are intentionally absent: `Is`/`Its` stay capitalized.
const MINOR_WORDS: &[&str] = &[
    "a", "an", "the", "and", "but", "or", "nor", "for", "of", "to", "in", "on", "at", "by", "as",
    "per", "via", "with", "from", "into", "onto", "vs",
];

const MAX_TITLE_WORDS: usize = 12;

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
}
