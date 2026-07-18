//! Tool-minted identity tokens (spec §1). The constants here are frozen forever.

/// Crockford-style lowercase base32 alphabet (excludes i, l, o, u to avoid
/// visual confusion). Frozen forever: every existing token's meaning depends
/// on this exact 32-byte literal never changing.
pub const TOKEN_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Canonical token length in chars. Frozen forever: 128 bits of entropy at
/// 5 bits/char, sized for the end-state so it never needs to grow.
pub const TOKEN_LEN: usize = 26;

/// Mint a fresh 128-bit random token, rendered as 26 base32-lowercase chars.
///
/// Draws 16 bytes from the OS CSPRNG (`getrandom`) and renders them 5 bits at
/// a time, most significant first. Propagates the CSPRNG error rather than
/// unwrapping it: a caller with no meaningful fallback should hear about it.
pub fn mint() -> Result<String, getrandom::Error> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)?;
    let n = u128::from_be_bytes(buf);
    let token: String = (0..26)
        .rev()
        .map(|i| TOKEN_ALPHABET[((n >> (5 * i)) & 31) as usize] as char)
        .collect();
    Ok(token)
}

/// Accepted charset at parse time: `^[0-9a-z]+$` (spec §1.8). Uniqueness
/// matters, shape does not: a hand-typed or third-party-minted token need not
/// be canonical (26 chars, `TOKEN_ALPHABET` only) to be accepted.
pub fn is_valid(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
}

/// Canonical shape (doctor warning when false on a valid token): 26 chars,
/// all in `TOKEN_ALPHABET`.
pub fn is_canonical(token: &str) -> bool {
    token.len() == TOKEN_LEN && token.bytes().all(|b| TOKEN_ALPHABET.contains(&b))
}

/// Compose a full card id from its parts. `hole` and `reversed` are mutually
/// exclusive by construction upstream (cloze cards never reverse).
pub fn card_id(token: &str, hole: Option<u32>, reversed: bool) -> String {
    debug_assert!(hole.is_none() || !reversed, "a cloze sub-card never reverses");
    if let Some(n) = hole {
        format!("{token}-{n}")
    } else if reversed {
        format!("{token}-r")
    } else {
        token.to_string()
    }
}

/// Split a full card id back into `(token, hole, reversed)`. `-` cannot occur
/// inside a token, so the first `-` is always the suffix boundary. `None` if
/// the shape is invalid.
///
/// A numeric hole suffix is accepted only in canonical decimal: exactly
/// `"0"`, or a first digit `1`-`9` followed by digits. A leading zero
/// (`"01"`) is rejected as a second spelling of `"1"`.
///
/// This validates only the id's shape; it does not check the token's
/// charset (that's `is_valid`'s job at the parse site).
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

/// True if `s` is `"0"`, or a nonzero digit followed by digits: a canonical
/// decimal integer with no leading zero.
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
