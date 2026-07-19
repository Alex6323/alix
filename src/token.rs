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

pub fn parse_card_id(id: &str) -> Option<(&str, Option<u32>, bool)> {
    match id.split_once('-') {
        None => Some((id, None, false)),
        Some((token, suffix)) => {
            if token.is_empty() {
                None
            } else if suffix == "r" {
                Some((token, None, true))
            } else if is_canonical_decimal(suffix) {
                let hole: u32 = suffix.parse().ok()?;
                Some((token, Some(hole), false))
            } else {
                None
            }
        }
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
    fn parse_card_id_round_trips_and_rejects_junk() {
        assert_eq!(parse_card_id("t0"), Some(("t0", None, false)));
        assert_eq!(parse_card_id("t0-2"), Some(("t0", Some(2), false)));
        assert_eq!(parse_card_id("t0-r"), Some(("t0", None, true)));

        assert_eq!(parse_card_id("t0-x"), None);
        assert_eq!(parse_card_id("t0-"), None);
        assert_eq!(parse_card_id("-r"), None);
        assert_eq!(parse_card_id("t0-1-2"), None);
        assert_eq!(parse_card_id("t0-12"), Some(("t0", Some(12), false)));
    }

    #[test]
    fn a_leading_zero_hole_suffix_is_rejected() {
        assert_eq!(parse_card_id("t0-01"), None);
        assert_eq!(parse_card_id("t0-00"), None);
        assert_eq!(parse_card_id("t0-0"), Some(("t0", Some(0), false)));
        assert_eq!(parse_card_id("t0-10"), Some(("t0", Some(10), false)));
    }
}
