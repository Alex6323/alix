/// User-ratified Crockford base32, lowercase, excluding i/l/o/u. Frozen
/// forever: every existing token's meaning depends on this exact alphabet.
pub const TOKEN_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// 26 chars carrying 128 bits of entropy. Frozen forever.
pub const TOKEN_LEN: usize = 26;

pub fn mint() -> Result<String, getrandom::Error> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)?;
    let n = u128::from_be_bytes(buf);
    // Emits 5-bit groups most-significant-first, 26 chars for 128 bits.
    let token: String = (0..26)
        .rev()
        .map(|i| TOKEN_ALPHABET[((n >> (5 * i)) & 31) as usize] as char)
        .collect();
    Ok(token)
}

// Any lowercase-alnum token is accepted, not just canonical shape (hand-typed
// or third-party tokens).
pub fn is_valid(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
}

pub fn is_canonical(token: &str) -> bool {
    token.len() == TOKEN_LEN && token.bytes().all(|b| TOKEN_ALPHABET.contains(&b))
}

pub fn card_id(token: &str, hole: Option<u32>, reversed: bool) -> String {
    debug_assert!(
        hole.is_none() || !reversed,
        "a cloze sub-card never reverses"
    );
    if let Some(n) = hole {
        format!("{token}-{n}")
    } else if reversed {
        format!("{token}-r")
    } else {
        token.to_string()
    }
}

fn is_canonical_decimal(s: &str) -> bool {
    match s.as_bytes() {
        [b'0'] => true,
        [first, rest @ ..] => {
            (b'1'..=b'9').contains(first) && rest.iter().all(|b| b.is_ascii_digit())
        }
        [] => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Deck,
    Card,
}

pub fn format_deck_id(token: &str) -> String {
    format!("deck-{token}")
}

pub fn format_card_id(token: &str, hole: Option<u32>, reversed: bool) -> String {
    format!("card-{}", card_id(token, hole, reversed))
}

pub fn parse_prefixed_card_id(id: &str) -> Option<(&str, Option<u32>, bool)> {
    let rest = id.strip_prefix("card-")?;
    let (token, hole, reversed) = match rest.split_once('-') {
        None => (rest, None, false),
        Some((token, "r")) => (token, None, true),
        Some((token, suffix)) if is_canonical_decimal(suffix) => {
            (token, Some(suffix.parse().ok()?), false)
        }
        Some(_) => return None,
    };
    if !is_valid(token) {
        return None;
    }
    let base = &id[.."card-".len() + token.len()];
    Some((base, hole, reversed))
}

pub fn parse_id(id: &str) -> Option<(Kind, &str, Option<u32>, bool)> {
    if let Some(token) = id.strip_prefix("deck-") {
        return is_valid(token).then_some((Kind::Deck, token, None, false));
    }
    let (base, hole, reversed) = parse_prefixed_card_id(id)?;
    Some((Kind::Card, base.strip_prefix("card-")?, hole, reversed))
}

pub fn is_valid_prefixed_id(id: &str) -> bool {
    parse_id(id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_tokens_are_canonical_and_distinct() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let token = mint().unwrap();
            assert!(is_canonical(&token), "not canonical: {token}");
            assert!(seen.insert(token), "duplicate token minted");
        }
    }

    #[test]
    fn the_alphabet_is_crockford_lowercase() {
        assert_eq!(TOKEN_ALPHABET, b"0123456789abcdefghjkmnpqrstvwxyz");
    }

    #[test]
    fn charset_accepts_any_lowercase_alnum_and_rejects_the_rest() {
        assert!(is_valid("q1"));
        assert!(!is_valid("Q1"));
        assert!(!is_valid("a-b"));
        assert!(!is_valid("a_b"));
        assert!(!is_valid(""));
        assert!(!is_valid("a b"));
    }

    #[test]
    fn card_id_composes_token_hole_and_reversed() {
        assert_eq!(card_id("t0", None, false), "t0");
        assert_eq!(card_id("t0", Some(2), false), "t0-2");
        assert_eq!(card_id("t0", None, true), "t0-r");
    }

    #[test]
    fn parse_prefixed_card_id_rejects_junk_suffixes_and_bare_tokens() {
        assert_eq!(parse_prefixed_card_id("t0"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-x"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-"), None);
        assert_eq!(parse_prefixed_card_id("card--r"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-1-2"), None);
        assert_eq!(
            parse_prefixed_card_id("card-t0-12"),
            Some(("card-t0", Some(12), false))
        );
    }

    #[test]
    fn a_leading_zero_hole_suffix_is_rejected() {
        assert_eq!(parse_prefixed_card_id("card-t0-01"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-00"), None);
        assert_eq!(
            parse_prefixed_card_id("card-t0-0"),
            Some(("card-t0", Some(0), false))
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-10"),
            Some(("card-t0", Some(10), false))
        );
    }

    #[test]
    fn parse_id_reads_the_kind_prefix_and_base_token() {
        assert_eq!(parse_id("deck-t0"), Some((Kind::Deck, "t0", None, false)));
        assert_eq!(parse_id("card-t0"), Some((Kind::Card, "t0", None, false)));
    }

    #[test]
    fn parse_id_reads_card_sub_id_forms() {
        assert_eq!(
            parse_id("card-t0-0"),
            Some((Kind::Card, "t0", Some(0), false))
        );
        assert_eq!(
            parse_id("card-t0-12"),
            Some((Kind::Card, "t0", Some(12), false))
        );
        assert_eq!(parse_id("card-t0-r"), Some((Kind::Card, "t0", None, true)));
    }

    #[test]
    fn parse_id_rejects_a_bare_token_with_no_prefix() {
        assert_eq!(parse_id("t0"), None);
        assert_eq!(parse_id("q1w2e3r4t5y6u7i8o9p0a1s2d3f4g"), None);
    }

    #[test]
    fn parse_id_rejects_an_unrecognized_prefix() {
        assert_eq!(parse_id("note-t0"), None);
        assert_eq!(parse_id("deckt0"), None);
    }

    #[test]
    fn parse_id_rejects_an_empty_token() {
        assert_eq!(parse_id("deck-"), None);
        assert_eq!(parse_id("card-"), None);
    }

    #[test]
    fn parse_id_rejects_a_dot_or_slash_in_the_token() {
        assert_eq!(parse_id("deck-a.b"), None);
        assert_eq!(parse_id("card-a/b"), None);
    }

    #[test]
    fn parse_id_rejects_a_double_suffix() {
        assert_eq!(parse_id("card-t0-0-r"), None);
    }

    #[test]
    fn parse_id_rejects_a_leading_zero_hole_suffix() {
        assert_eq!(parse_id("card-t0-01"), None);
    }

    #[test]
    fn parse_prefixed_card_id_splits_the_prefixed_base_from_the_suffix() {
        assert_eq!(
            parse_prefixed_card_id("card-t0-2"),
            Some(("card-t0", Some(2), false))
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-r"),
            Some(("card-t0", None, true))
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0"),
            Some(("card-t0", None, false))
        );
    }

    #[test]
    fn format_deck_id_and_format_card_id_emit_the_prefixed_forms() {
        assert_eq!(format_deck_id("t0"), "deck-t0");
        assert_eq!(format_card_id("t0", None, false), "card-t0");
        assert_eq!(format_card_id("t0", Some(2), false), "card-t0-2");
        assert_eq!(format_card_id("t0", None, true), "card-t0-r");
    }

    #[test]
    fn is_valid_prefixed_id_accepts_well_formed_ids_and_rejects_the_rest() {
        assert!(is_valid_prefixed_id("deck-t0"));
        assert!(is_valid_prefixed_id("card-t0-2"));
        assert!(is_valid_prefixed_id("card-t0-r"));
        assert!(!is_valid_prefixed_id("t0"));
        assert!(!is_valid_prefixed_id("note-t0"));
        assert!(!is_valid_prefixed_id("deck-"));
        assert!(!is_valid_prefixed_id("card-t0-0-r"));
    }
}
