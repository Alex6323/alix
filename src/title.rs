//! Condensing a long, model-written or hand-written phrase into a short,
//! title-cased label.
//!
//! Two callers share this one rule. [`crate::explore`] condenses each plan
//! item's title at *creation* (the plan prompt asks for brevity, but the model
//! ignores it and appends the contents after a colon). The [`crate::picker`]
//! condenses a trace's `% trace:` path-question at *display*, so the picker can
//! label a trace by its description instead of its filename slug — an `explore`
//! trace is already short, but a `--build` or hand-written one can be a whole
//! sentence. Keeping the logic in one place means both surfaces shorten alike.

/// The minor words a title keeps lowercase (unless first or last) — articles and
/// short coordinating conjunctions / prepositions. Forms of "to be" and pronouns
/// are intentionally absent, so `Is`/`Its` stay capitalized.
const MINOR_WORDS: &[&str] = &[
    "a", "an", "the", "and", "but", "or", "nor", "for", "of", "to", "in", "on", "at", "by", "as",
    "per", "via", "with", "from", "into", "onto", "vs",
];

/// The hard ceiling on a condensed title's word count — the backstop that bounds
/// a long title written with no separator to cut at.
const MAX_TITLE_WORDS: usize = 12;

/// Condenses a long title into a short, capitalized one — deterministically: cut
/// the enumeration (everything from the first `:`/`;`/dash, plus a trailing
/// parenthetical), hard-cap the word count as a backstop when no such separator
/// exists, then apply title case that leaves code spans (backticked,
/// `snake_case`, `CamelCase`, `ACRONYM`) intact.
pub fn condense(raw: &str) -> String {
    let mut s = raw.trim();
    // Cut at the first enumeration separator if there is one — but never depend
    // on one: the word cap below bounds a title that has none.
    if let Some(i) = s.find([':', ';', '—', '–']) {
        s = s[..i].trim_end();
    }
    // Drop a trailing parenthetical aside ("… (foo, bar)").
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

/// Title-cases one word: code tokens pass through verbatim, minor words stay
/// lowercase unless they're forced (first/last word), everything else is
/// capitalized per hyphen/slash segment.
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

/// Whether a word is a code identifier to leave exactly as written: a backticked
/// span, `snake_case`, a call (`foo()`), or any token with an internal capital
/// (`CamelCase`, `VM`, `gRPC`).
fn is_code_token(w: &str) -> bool {
    w.contains('`')
        || w.contains('_')
        || w.contains('(')
        || w.chars().skip(1).any(|c| c.is_ascii_uppercase())
}

/// Uppercases the first character of a segment, leaving the rest untouched.
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
        // The model's signature shape: a good short head, a colon, then the
        // enumeration the prompt forbids. Cut at the colon, title-case the head.
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
        // Backticked / snake_case / CamelCase / acronym tokens must survive title
        // casing verbatim — never `Grpc`, `Execute_signed`, `Vm`.
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
        // No colon to cut at: the word cap is the guarantee, so brevity holds.
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
