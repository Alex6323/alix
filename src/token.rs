/// User-ratified Crockford base32, lowercase, excluding i/l/o/u. Frozen
/// forever: every existing token's meaning depends on this exact alphabet.
pub const TOKEN_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// 26 chars carrying 128 bits of entropy. Frozen forever.
pub const TOKEN_LEN: usize = 26;

/// Table-row stamps: 6 chars, always machine-minted, unique within one
/// table by a mint-time check. Frozen forever.
pub const ROW_TOKEN_LEN: usize = 6;

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

pub fn mint_row() -> Result<String, getrandom::Error> {
    let mut buf = [0u8; 4];
    getrandom::getrandom(&mut buf)?;
    Ok(row_from_bytes(buf))
}

/// Split from its caller so the packing is a pure function of its bytes.
fn row_from_bytes(buf: [u8; 4]) -> String {
    let n = u32::from_be_bytes(buf);
    (0..ROW_TOKEN_LEN)
        .rev()
        .map(|i| TOKEN_ALPHABET[((n >> (5 * i)) & 31) as usize] as char)
        .collect()
}

// Strict, unlike base tokens: every row stamp is machine-minted, so only
// the canonical shape is ever legal.
pub fn is_valid_row(token: &str) -> bool {
    token.len() == ROW_TOKEN_LEN && token.bytes().all(|b| TOKEN_ALPHABET.contains(&b))
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

/// Region stamps (ADR 0034): 6 chars, minted or author-copied, unique
/// within one parent card. Frozen forever.
pub const REGION_STAMP_LEN: usize = 6;

/// A derived group id's digit count: 13 Crockford digits carry a 64-bit
/// hash. Frozen forever.
pub const GROUP_HASH_LEN: usize = 13;

/// A region suffix on a card id: `-b<stamp>` for a single region,
/// `-g<hash>` for a derived group (ADR 0034).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionRef<'a> {
    Single(&'a str),
    Group(&'a str),
}

// Strict like row stamps: the minter emits only this shape.
pub fn is_valid_region_stamp(stamp: &str) -> bool {
    stamp.len() == REGION_STAMP_LEN && stamp.bytes().all(|b| TOKEN_ALPHABET.contains(&b))
}

fn is_valid_group_hash(hash: &str) -> bool {
    hash.len() == GROUP_HASH_LEN && hash.bytes().all(|b| TOKEN_ALPHABET.contains(&b))
}

/// The frozen ADR 0034 group derivation: member stamps sorted ascending by
/// their ASCII bytes, joined with one `-`, hashed with XxHash64 seed 0, and
/// rendered as 13 digits most-significant-first (one implicit leading zero
/// bit).
pub fn derive_group_hash(stamps: &[&str]) -> String {
    use std::hash::Hasher;
    let mut sorted: Vec<&str> = stamps.to_vec();
    sorted.sort_unstable();
    let mut hasher = twox_hash::XxHash64::default();
    hasher.write(sorted.join("-").as_bytes());
    let n = hasher.finish();
    (0..GROUP_HASH_LEN)
        .rev()
        .map(|i| TOKEN_ALPHABET[((n >> (5 * i)) & 31) as usize] as char)
        .collect()
}

pub fn format_region_card_id(token: &str, region: RegionRef<'_>) -> String {
    match region {
        RegionRef::Single(stamp) => format!("card-{token}-b{stamp}"),
        RegionRef::Group(hash) => format!("card-{token}-g{hash}"),
    }
}

pub fn card_id(
    token: &str,
    row: Option<&str>,
    hole: Option<u32>,
    reversed: bool,
    region: Option<RegionRef<'_>>,
) -> String {
    debug_assert!(
        hole.is_none() || !reversed,
        "a cloze sub-card never reverses"
    );
    debug_assert!(
        row.is_none() || hole.is_none(),
        "a table row card is never cloze"
    );
    debug_assert!(
        region.is_none() || (row.is_none() && hole.is_none() && !reversed),
        "a region card carries no other suffix"
    );
    if let Some(region) = region {
        return match region {
            RegionRef::Single(stamp) => format!("{token}-b{stamp}"),
            RegionRef::Group(hash) => format!("{token}-g{hash}"),
        };
    }
    if let Some(row) = row {
        if reversed {
            format!("{token}-t{row}-r")
        } else {
            format!("{token}-t{row}")
        }
    } else if let Some(n) = hole {
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

pub fn format_card_id(token: &str, row: Option<&str>, hole: Option<u32>, reversed: bool) -> String {
    format!("card-{}", card_id(token, row, hole, reversed, None))
}

pub type ParsedCardSuffix<'a> = (
    &'a str,
    Option<&'a str>,
    Option<u32>,
    bool,
    Option<RegionRef<'a>>,
);

pub fn parse_prefixed_card_id(id: &str) -> Option<ParsedCardSuffix<'_>> {
    let rest = id.strip_prefix("card-")?;
    let (token, row, hole, reversed, region) = match rest.split_once('-') {
        None => (rest, None, None, false, None),
        Some((token, "r")) => (token, None, None, true, None),
        Some((token, suffix)) if is_canonical_decimal(suffix) => {
            (token, None, Some(suffix.parse().ok()?), false, None)
        }
        Some((token, suffix)) => match suffix.as_bytes().first() {
            Some(b't') => match suffix[1..].split_once('-') {
                Some((row, "r")) if is_valid_row(row) => (token, Some(row), None, true, None),
                None if is_valid_row(&suffix[1..]) => {
                    (token, Some(&suffix[1..]), None, false, None)
                }
                _ => return None,
            },
            Some(b'b') if is_valid_region_stamp(&suffix[1..]) => (
                token,
                None,
                None,
                false,
                Some(RegionRef::Single(&suffix[1..])),
            ),
            Some(b'g') if is_valid_group_hash(&suffix[1..]) => (
                token,
                None,
                None,
                false,
                Some(RegionRef::Group(&suffix[1..])),
            ),
            _ => return None,
        },
    };
    if !is_valid(token) {
        return None;
    }
    let base = &id[.."card-".len() + token.len()];
    Some((base, row, hole, reversed, region))
}

pub type ParsedId<'a> = (
    Kind,
    &'a str,
    Option<&'a str>,
    Option<u32>,
    bool,
    Option<RegionRef<'a>>,
);

pub fn parse_id(id: &str) -> Option<ParsedId<'_>> {
    if let Some(token) = id.strip_prefix("deck-") {
        return is_valid(token).then_some((Kind::Deck, token, None, None, false, None));
    }
    let (base, row, hole, reversed, region) = parse_prefixed_card_id(id)?;
    Some((
        Kind::Card,
        base.strip_prefix("card-")?,
        row,
        hole,
        reversed,
        region,
    ))
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
    fn card_id_composes_token_row_hole_and_reversed() {
        assert_eq!(card_id("t0", None, None, false, None), "t0");
        assert_eq!(card_id("t0", None, Some(2), false, None), "t0-2");
        assert_eq!(card_id("t0", None, None, true, None), "t0-r");
        assert_eq!(
            card_id("t0", Some("abcdef"), None, false, None),
            "t0-tabcdef"
        );
        assert_eq!(
            card_id("t0", Some("abcdef"), None, true, None),
            "t0-tabcdef-r"
        );
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
            Some(("card-t0", None, Some(12), false, None))
        );
    }

    #[test]
    fn a_leading_zero_hole_suffix_is_rejected() {
        assert_eq!(parse_prefixed_card_id("card-t0-01"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-00"), None);
        assert_eq!(
            parse_prefixed_card_id("card-t0-0"),
            Some(("card-t0", None, Some(0), false, None))
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-10"),
            Some(("card-t0", None, Some(10), false, None))
        );
    }

    #[test]
    fn parse_id_reads_the_kind_prefix_and_base_token() {
        assert_eq!(
            parse_id("deck-t0"),
            Some((Kind::Deck, "t0", None, None, false, None))
        );
        assert_eq!(
            parse_id("card-t0"),
            Some((Kind::Card, "t0", None, None, false, None))
        );
    }

    #[test]
    fn parse_id_reads_card_sub_id_forms() {
        assert_eq!(
            parse_id("card-t0-0"),
            Some((Kind::Card, "t0", None, Some(0), false, None))
        );
        assert_eq!(
            parse_id("card-t0-12"),
            Some((Kind::Card, "t0", None, Some(12), false, None))
        );
        assert_eq!(
            parse_id("card-t0-r"),
            Some((Kind::Card, "t0", None, None, true, None))
        );
        assert_eq!(
            parse_id("card-t0-tabcdef"),
            Some((Kind::Card, "t0", Some("abcdef"), None, false, None))
        );
        assert_eq!(
            parse_id("card-t0-tabcdef-r"),
            Some((Kind::Card, "t0", Some("abcdef"), None, true, None))
        );
    }

    #[test]
    fn the_adr_0034_group_derivation_reproduces_the_frozen_vector() {
        assert_eq!("chsbz14b1a30x", derive_group_hash(&["a1b2c3", "d4e5f6"]));
        assert_eq!(
            "chsbz14b1a30x",
            derive_group_hash(&["d4e5f6", "a1b2c3"]),
            "sorting is load-bearing: member order in the file never matters"
        );
        assert_ne!(
            derive_group_hash(&["a1b2c3"]),
            derive_group_hash(&["a1b2c3", "d4e5f6"])
        );
        assert_ne!(
            derive_group_hash(&["a1b2c3", "d4e5f6"]),
            derive_group_hash(&["a1b2c3", "d4e5f6", "g7h8j9"]),
            "membership is the identity: adding a member changes the id"
        );
    }

    #[test]
    fn region_card_ids_format_and_parse_round_trip() {
        let single = format_region_card_id("t0", RegionRef::Single("a1b2c3"));
        assert_eq!("card-t0-ba1b2c3", single);
        assert_eq!(
            parse_prefixed_card_id(&single),
            Some((
                "card-t0",
                None,
                None,
                false,
                Some(RegionRef::Single("a1b2c3"))
            ))
        );
        let group = format_region_card_id("t0", RegionRef::Group("chsbz14b1a30x"));
        assert_eq!("card-t0-gchsbz14b1a30x", group);
        assert_eq!(
            parse_prefixed_card_id(&group),
            Some((
                "card-t0",
                None,
                None,
                false,
                Some(RegionRef::Group("chsbz14b1a30x"))
            ))
        );
    }

    #[test]
    fn malformed_region_suffixes_are_rejected() {
        assert_eq!(
            parse_prefixed_card_id("card-t0-ba1b2c"),
            None,
            "five-char stamp"
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-ba1b2c3d"),
            None,
            "seven-char stamp"
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-bi1b2c3"),
            None,
            "excluded letter"
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-gchsbz14b1a30"),
            None,
            "twelve-digit group"
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-ba1b2c3-r"),
            None,
            "a region never reverses"
        );
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
            Some(("card-t0", None, Some(2), false, None))
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-r"),
            Some(("card-t0", None, None, true, None))
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0"),
            Some(("card-t0", None, None, false, None))
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-tabcdef"),
            Some(("card-t0", Some("abcdef"), None, false, None))
        );
        assert_eq!(
            parse_prefixed_card_id("card-t0-tabcdef-r"),
            Some(("card-t0", Some("abcdef"), None, true, None))
        );
    }

    #[test]
    fn format_deck_id_and_format_card_id_emit_the_prefixed_forms() {
        assert_eq!(format_deck_id("t0"), "deck-t0");
        assert_eq!(format_card_id("t0", None, None, false), "card-t0");
        assert_eq!(format_card_id("t0", None, Some(2), false), "card-t0-2");
        assert_eq!(format_card_id("t0", None, None, true), "card-t0-r");
        assert_eq!(
            format_card_id("t0", Some("abcdef"), None, false),
            "card-t0-tabcdef"
        );
        assert_eq!(
            format_card_id("t0", Some("abcdef"), None, true),
            "card-t0-tabcdef-r"
        );
    }

    #[test]
    fn is_valid_prefixed_id_accepts_well_formed_ids_and_rejects_the_rest() {
        assert!(is_valid_prefixed_id("deck-t0"));
        assert!(is_valid_prefixed_id("card-t0-2"));
        assert!(is_valid_prefixed_id("card-t0-r"));
        assert!(is_valid_prefixed_id("card-t0-tabcdef"));
        assert!(is_valid_prefixed_id("card-t0-tabcdef-r"));
        assert!(!is_valid_prefixed_id("t0"));
        assert!(!is_valid_prefixed_id("note-t0"));
        assert!(!is_valid_prefixed_id("deck-"));
        assert!(!is_valid_prefixed_id("card-t0-0-r"));
    }

    #[test]
    fn minted_row_stamps_are_valid_and_vary() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let row = mint_row().unwrap();
            assert!(is_valid_row(&row), "not a valid row stamp: {row}");
            seen.insert(row);
        }
        assert!(seen.len() > 1, "mint_row produced a constant");
    }

    #[test]
    fn row_packing_is_pinned_for_known_bytes() {
        // Exact output for fixed input: a changed shift or multiplier is
        // visible rather than merely "still six valid characters".
        for (buf, want) in [
            ([0u8, 0, 0, 0], "000000"),
            ([0xDE, 0xAD, 0xBE, 0xEF], "favfqf"),
            ([0xFF, 0xFF, 0xFF, 0xFF], "zzzzzz"),
        ] {
            assert_eq!(want, row_from_bytes(buf), "bytes {buf:?}");
        }
    }

    #[test]
    fn is_valid_row_demands_exactly_six_alphabet_chars() {
        assert!(is_valid_row("abcdef"));
        assert!(is_valid_row("012345"));
        assert!(!is_valid_row("abcde"));
        assert!(!is_valid_row("abcdefg"));
        assert!(!is_valid_row(""));
        assert!(!is_valid_row("abcdeu"));
        assert!(!is_valid_row("ABCDEF"));
        assert!(!is_valid_row("abc-de"));
    }

    #[test]
    fn row_ids_round_trip_through_format_and_parse() {
        for reversed in [false, true] {
            let row = mint_row().unwrap();
            let id = format_card_id("t0", Some(&row), None, reversed);
            let (base, parsed_row, hole, parsed_reversed, _) =
                parse_prefixed_card_id(&id).expect("a formatted row id must parse");
            assert_eq!(base, "card-t0", "reversed={reversed}");
            assert_eq!(parsed_row, Some(row.as_str()), "reversed={reversed}");
            assert_eq!(hole, None, "reversed={reversed}");
            assert_eq!(parsed_reversed, reversed);
        }
    }

    #[test]
    fn malformed_row_suffixes_are_rejected() {
        assert_eq!(parse_prefixed_card_id("card-t0-t"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-tabcde"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-tabcdefg"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-tabcdef-x"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-tabcdef-r-r"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-tabcde-r"), None);
        assert_eq!(parse_prefixed_card_id("card-t0-tABCDEF"), None);
        assert_eq!(parse_id("deck-t0-tabcdef"), None);
    }
}
