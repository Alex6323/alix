//! The published card-identity algorithm (freeze-forever, third-party-reproducible).
//!
//! This module is the **normative** definition of a card's id: the algorithm is
//! the code, and the golden vectors in the tests are its conformance suite. Any
//! third-party tool (catalog builder, CI, a linter in another language) computes
//! the same id by reproducing these steps; prose in the book is explanatory.
//!
//! An id is `SHA-256(canonical input)` truncated to the first 128 bits, rendered
//! lowercase hex (32 chars). The canonical input is a sequence of typed fields
//! joined as **netstrings** (`<byte-len>:<bytes>,`), which is injection-proof:
//! no field's content can imitate a field boundary. The fields, in order, are:
//!
//! 1. `card-id-v1` - a format-version tag, so a future algorithm revision is distinguishable and
//!    never silently collides with a v1 id.
//! 2. the role tag (`plain` / `cloze` / `reverse`), so e.g. a reversed card cannot collide with a
//!    plain card that happens to share the same back text.
//! 3. the deck id (the deck's frontmatter `id:`, the identity namespace - never the filename).
//! 4. the content field(s): the front (question) then the joined answer for plain/reversed cards,
//!    so two cards sharing an answer but asking different questions stay distinct; or, for cloze,
//!    one fenced line per answer line followed by `#cloze:N` (the 0-based index of the hole this
//!    sub-card asks). A cloze card has no separately-authored front - its prompt is its content -
//!    so no front field is added to it.
//!
//! Every field is NFC-normalized then whitespace-collapsed over a **closed ASCII
//! set** ([`normalize_ws`]) before framing. NFC makes the id depend on the visual
//! string, not its byte encoding (a filesystem may hand back `Übung` as NFD on
//! macOS, NFC on Linux). We rely on the Unicode **Normalization Stability
//! Policy** (the NFC form of an *assigned* code point never changes across
//! Unicode versions) rather than pinning a version string; the residual gap
//! (an *unassigned* code point later gaining a decomposition) is closed upstream
//! by rejecting unassigned code points at parse time, not here. The closed ASCII
//! whitespace set is chosen so a naive re-implementation agrees byte-for-byte:
//! notably Python's `str.split()` would eat the U+001F cloze sentinel, so we must
//! NOT use "Unicode whitespace" here.
//!
//! **Cloze is taken as structure, not a pre-built string.** [`cloze_id_hex`]
//! receives the parsed segments (text and holes) and draws the U+001F hole fences
//! itself, rejecting any raw C0 control (including a literal U+001F) in deck
//! content. This makes hole forgery *impossible by construction*: a hand-typed
//! U+001F can never masquerade as a real `{{ }}` hole, because only genuine holes
//! ever become fences.

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

/// The format-version tag, framed as the first field. Bump ONLY for a deliberate
/// algorithm change that intends to re-key every id.
const VERSION_TAG: &str = "card-id-v1";

/// The identity role of a card, framed as the second field so cards of different
/// roles never collide even with identical remaining fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Plain,
    Cloze,
    Reversed,
}

impl Role {
    fn tag(self) -> &'static str {
        match self {
            Role::Plain => "plain",
            Role::Cloze => "cloze",
            Role::Reversed => "reverse",
        }
    }
}

/// One segment of a parsed cloze answer line: literal text, or a hole's content
/// (what was written inside `{{ }}`). [`cloze_id_hex`] fences each hole itself.
#[derive(Clone, Copy, Debug)]
pub enum ClozeSegment<'a> {
    Text(&'a str),
    Hole(&'a str),
}

/// Why a card id could not be computed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// Deck content contains a forbidden C0 control character (one outside the
    /// whitespace set collapsed by [`normalize_ws`], e.g. the U+001F cloze
    /// sentinel). Rejected because it could forge cloze hole structure.
    #[error("deck content contains a forbidden control character U+{0:04X}")]
    ControlChar(u32),
}

/// The closed whitespace set collapsed by [`normalize_ws`]: ASCII tab, LF, VT,
/// FF, CR, space. **Deliberately not** Unicode `White_Space` - that set is a
/// cross-language and cross-Unicode-version hazard (Python's `str.split()`
/// additionally treats the U+001F cloze sentinel as whitespace and would delete
/// it, silently diverging on every cloze card).
const WHITESPACE: [char; 6] = ['\u{09}', '\u{0a}', '\u{0b}', '\u{0c}', '\u{0d}', '\u{20}'];

/// Collapses every run of one-or-more [`WHITESPACE`] code points to a single
/// space and trims the ends, leaving every other code point (including U+00A0
/// NBSP and the U+001F cloze sentinel) untouched.
fn normalize_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    let mut emitted = false;
    for c in s.chars() {
        if WHITESPACE.contains(&c) {
            pending_space = true;
        } else {
            if pending_space && emitted {
                out.push(' ');
            }
            out.push(c);
            pending_space = false;
            emitted = true;
        }
    }
    out
}

/// Rejects any C0 control character that is not part of the whitespace set (so
/// tab/LF/VT/FF/CR pass, everything else in U+0000..U+001F does not). This is the
/// tripwire that makes the U+001F cloze fence unforgeable in deck content.
fn reject_controls(s: &str) -> Result<(), IdError> {
    for c in s.chars() {
        let u = c as u32;
        if u < 0x20 && !WHITESPACE.contains(&c) {
            return Err(IdError::ControlChar(u));
        }
    }
    Ok(())
}

/// Frames fields as netstrings (`<byte-len>:<bytes>,`), the injection-proof join
/// used by the canonical input: length-prefixing means no field's content can
/// imitate a field boundary. Factored out so a round-trip property test can prove
/// the framing cannot collide two distinct field lists.
fn netstring_encode(fields: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for field in fields {
        out.extend_from_slice(format!("{}:", field.len()).as_bytes());
        out.extend_from_slice(field);
        out.push(b',');
    }
    out
}

/// A single field's canonical bytes: NFC-normalized, whitespace-collapsed, UTF-8.
fn normalized_field(raw: &str) -> Vec<u8> {
    normalize_ws(&raw.nfc().collect::<String>()).into_bytes()
}

/// Builds the netstring-framed canonical input and hashes it. Callers validate
/// content (via [`reject_controls`]) before calling; this step assumes clean
/// fields.
fn hash_fields(deck_id: &str, role: Role, content: &[String]) -> String {
    let mut fields: Vec<Vec<u8>> = Vec::with_capacity(3 + content.len());
    fields.push(normalized_field(VERSION_TAG));
    fields.push(normalized_field(role.tag()));
    fields.push(normalized_field(deck_id));
    for field in content {
        fields.push(normalized_field(field));
    }
    let refs: Vec<&[u8]> = fields.iter().map(|field| field.as_slice()).collect();
    let digest = Sha256::digest(netstring_encode(&refs));
    let mut hex = String::with_capacity(32);
    for byte in &digest[..16] {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// The id of a plain (front→back) card. `front` is its question; `answer` is its
/// back lines joined by LF. Both are part of the identity, so two cards that share
/// an answer but ask different questions are distinct cards.
pub fn plain_id_hex(deck_id: &str, front: &str, answer: &str) -> Result<String, IdError> {
    front_back_id(deck_id, Role::Plain, front, answer)
}

/// The id of the reversed half of a `direction: both`/`reverse` card. `front` and
/// `answer` are the reversed card's OWN sides (the swap of its forward sibling:
/// front is the original answer, answer is the original front). Tagged `reverse`
/// so it cannot collide with a plain card sharing the same two sides.
pub fn reversed_id_hex(deck_id: &str, front: &str, answer: &str) -> Result<String, IdError> {
    front_back_id(deck_id, Role::Reversed, front, answer)
}

/// Shared body for the two front→back roles: reject forbidden controls in every
/// field, then hash `[front, answer]` as the content (front first).
fn front_back_id(deck_id: &str, role: Role, front: &str, answer: &str) -> Result<String, IdError> {
    reject_controls(deck_id)?;
    reject_controls(front)?;
    reject_controls(answer)?;
    Ok(hash_fields(
        deck_id,
        role,
        &[front.to_string(), answer.to_string()],
    ))
}

/// The id of a cloze sub-card, built from the parsed answer `lines` and the
/// 0-based `hole_index` of the hole this sub-card asks. This function draws the
/// U+001F hole fences itself and rejects any raw C0 control in a segment, so a
/// hand-typed sentinel can never forge a hole.
///
/// Pass the PARSED segments here, never the already-fenced strings the parser
/// stores in `hash_lines` (those carry raw U+001F and would be rejected). Cloze
/// id injectivity assumes the parser never emits an empty hole or an empty text
/// segment (it rejects empty holes and skips empty text), so the empty-segment
/// corner (where two different fencings could coincide) is unreachable from a
/// real deck.
pub fn cloze_id_hex(
    deck_id: &str,
    lines: &[Vec<ClozeSegment>],
    hole_index: usize,
) -> Result<String, IdError> {
    reject_controls(deck_id)?;
    let mut content = Vec::with_capacity(lines.len() + 1);
    for line in lines {
        let mut fenced = String::new();
        for segment in line {
            match segment {
                ClozeSegment::Text(text) => {
                    reject_controls(text)?;
                    fenced.push_str(text);
                }
                ClozeSegment::Hole(hole) => {
                    reject_controls(hole)?;
                    fenced.push('\u{1f}');
                    fenced.push_str(hole);
                    fenced.push('\u{1f}');
                }
            }
        }
        content.push(fenced);
    }
    content.push(format!("#cloze:{hole_index}"));
    Ok(hash_fields(deck_id, Role::Cloze, &content))
}

#[cfg(test)]
mod tests {
    use super::{
        ClozeSegment::{Hole, Text},
        *,
    };

    /// Holds the front fixed at "q" so an invariant test can vary deck/answer
    /// alone; the golden vectors and the front-sensitivity test below pass the
    /// front explicitly.
    fn plain(deck: &str, back: &str) -> String {
        plain_id_hex(deck, "q", back).unwrap()
    }

    #[test]
    fn front_is_part_of_identity() {
        // Same deck, same answer, different question -> different card. This is
        // the front-inclusion decision: identity is the whole (question, answer).
        assert_ne!(
            plain_id_hex("deck", "When did WWII end?", "1945").unwrap(),
            plain_id_hex("deck", "When were the atomic bombs dropped?", "1945").unwrap(),
        );
    }

    #[test]
    fn id_is_32_lowercase_hex_chars() {
        let id = plain("deck", "an answer");
        assert_eq!(32, id.len());
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn id_is_deterministic() {
        assert_eq!(plain("deck", "answer"), plain("deck", "answer"));
    }

    #[test]
    fn a_reversed_card_never_collides_with_a_plain_card_sharing_its_sides() {
        // Same deck, same front ("q") and back ("x"), only the role differs.
        assert_ne!(
            plain("deck", "x"),
            reversed_id_hex("deck", "q", "x").unwrap()
        );
    }

    #[test]
    fn cloze_hole_boundary_placement_changes_the_id() {
        // `ab{{c}}` vs `a{{bc}}` (same letters, same hole index) must differ.
        let x = cloze_id_hex("deck", &[vec![Text("ab"), Hole("c")]], 0).unwrap();
        let y = cloze_id_hex("deck", &[vec![Text("a"), Hole("bc")]], 0).unwrap();
        assert_ne!(x, y);
    }

    #[test]
    fn framing_is_injective_across_line_boundaries() {
        // Two lines must not collide with one line of the joined content.
        let two = cloze_id_hex("deck", &[vec![Hole("a")], vec![Hole("b")]], 0).unwrap();
        let one = cloze_id_hex("deck", &[vec![Hole("ab")]], 0).unwrap();
        assert_ne!(two, one);
    }

    #[test]
    fn framing_separates_deck_id_from_content() {
        assert_ne!(plain("ab", "c"), plain("a", "bc"));
    }

    // --- The forgery the module review caught: a raw U+001F (or any C0 control)
    // in deck content is rejected, so it can never masquerade as a `{{ }}` hole.

    #[test]
    fn a_raw_control_byte_in_cloze_text_is_rejected() {
        // Without this, `[Text("a\u{1f}b\u{1f}")]` would forge the fenced bytes of
        // a real hole `a{{b}}` and collide with it.
        let forged = cloze_id_hex("deck", &[vec![Text("a\u{1f}b\u{1f}")]], 0);
        assert_eq!(Err(IdError::ControlChar(0x1f)), forged);
    }

    #[test]
    fn a_raw_control_byte_in_a_plain_answer_is_rejected() {
        assert_eq!(
            Err(IdError::ControlChar(0x1f)),
            plain_id_hex("deck", "q", "a\u{1f}b")
        );
        assert_eq!(
            Err(IdError::ControlChar(0x00)),
            plain_id_hex("deck", "q", "a\u{0}b")
        );
    }

    #[test]
    fn a_raw_control_byte_in_a_plain_front_is_rejected() {
        // The front is a content field too, so it gets the same tripwire.
        assert_eq!(
            Err(IdError::ControlChar(0x1f)),
            plain_id_hex("deck", "a\u{1f}b", "ans")
        );
    }

    #[test]
    fn whitespace_controls_and_ordinary_text_are_accepted() {
        assert!(plain_id_hex("deck", "q", "a\tb\nc").is_ok()); // tab + LF are allowed
        assert!(plain_id_hex("deck", "q", "normal answer").is_ok());
        assert!(cloze_id_hex("deck", &[vec![Text("hat "), Hole("16")]], 0).is_ok());
    }

    // --- NFC conformance: these RELATIVE checks fail loudly for a from-scratch
    // NFC that only handles the easy pairwise-composition case.

    #[test]
    fn nfc_pairwise_composition_deck_id() {
        // Ü as one char vs U + combining diaeresis.
        assert_eq!(plain("\u{dc}bung", "x"), plain("U\u{308}bung", "x"));
    }

    #[test]
    fn nfc_applies_to_content_not_just_deck_id() {
        assert_eq!(plain("deck", "\u{dc}"), plain("deck", "U\u{308}"));
    }

    #[test]
    fn nfc_singleton_decomposition() {
        // U+2126 OHM SIGN normalizes to U+03A9 GREEK CAPITAL OMEGA (a singleton).
        // A naive "recompose pairs" NFC gets this wrong.
        assert_eq!(plain("deck", "\u{2126}"), plain("deck", "\u{3a9}"));
    }

    #[test]
    fn nfc_composition_exclusion() {
        // U+0958 must DECOMPOSE to U+0915 U+093C and stay decomposed (a
        // composition exclusion), so both spellings hash equal.
        assert_eq!(plain("deck", "\u{958}"), plain("deck", "\u{915}\u{93c}"));
    }

    // --- Whitespace conformance: exercises normalize_ws (the review found the
    // golden vectors never did, so a skip-collapse impl passed the suite).

    #[test]
    fn ascii_whitespace_collapses_but_nbsp_does_not() {
        assert_eq!(plain("deck", "a  b"), plain("deck", "a b"));
        assert_eq!(plain("deck", " a\tb "), plain("deck", "a b"));
        assert_ne!(plain("deck", "a\u{a0}b"), plain("deck", "a b")); // NBSP stays
    }

    #[test]
    fn normalize_ws_trims_and_collapses() {
        assert_eq!("a b", normalize_ws("  a   b  "));
        assert_eq!("a b", normalize_ws("a\t\nb"));
        assert_eq!("", normalize_ws("   "));
        assert_eq!("a\u{a0}b", normalize_ws("a\u{a0}b"));
        assert_eq!("a\u{1f}b", normalize_ws("a\u{1f}b"));
    }

    // --- The frozen golden-vector conformance suite. Reproduced byte-for-byte by
    // an independent Python implementation of these rules (see the module doc), so
    // the id is genuinely third-party-computable. Any edit that moves an id here is
    // a freeze-forever break and must fail CI.

    #[test]
    fn golden_vectors_are_frozen() {
        // Plain: front + answer both hashed (front-inclusion).
        assert_eq!(
            "604dbbb8910c77d1b01ab2fc622f94e5",
            plain_id_hex("git-basics", "How do you stage a file?", "git add").unwrap()
        );
        // Reversed: the swapped sides of the same card, tagged `reverse`.
        assert_eq!(
            "b8d4e2ba8f85bfed448f09357d2ce1a4",
            reversed_id_hex("git-basics", "git add", "How do you stage a file?").unwrap()
        );
        // Cloze: unchanged by front-inclusion (a cloze has no separate front), so
        // this value must equal Build 1's - a regression guard on the cloze path.
        assert_eq!(
            "9beb57c5047bea9c951c9b0fed770289",
            cloze_id_hex(
                "de-states",
                &[vec![
                    Text("Deutschland hat "),
                    Hole("16"),
                    Text(" Bundesl\u{e4}nder.")
                ]],
                0,
            )
            .unwrap(),
        );
        // NFC and NFD of "Übung" produce the SAME frozen id (locks NFC).
        assert_eq!("5178898608db3e9d81dea6737c333140", plain("\u{dc}bung", "x"));
        assert_eq!(
            "5178898608db3e9d81dea6737c333140",
            plain("U\u{308}bung", "x")
        );
        // A whitespace-bearing vector, so a skip-collapse impl computes a
        // different value and fails (the review's "goldens never exercise
        // normalize_ws" gap).
        assert_eq!("562da7540424e3ba840eeccee26ce1c9", plain("deck", "a  b"));
        assert_eq!("562da7540424e3ba840eeccee26ce1c9", plain("deck", "a b"));
    }

    #[test]
    fn role_and_version_tags_are_frozen() {
        // These strings are hashed into every id. Renaming a role tag re-keys
        // every card of that role; changing the version tag re-keys EVERY card.
        // Adding a NEW role is free (a new distinct tag namespaces it without
        // touching these) - but these must never change.
        assert_eq!("card-id-v1", VERSION_TAG);
        assert_eq!("plain", Role::Plain.tag());
        assert_eq!("cloze", Role::Cloze.tag());
        assert_eq!("reverse", Role::Reversed.tag());
    }

    // --- Property tests: fuzz the invariants across arbitrary inputs, to catch
    // collision/normalization bugs the hand-picked vectors above cannot.

    /// Inverse of [`netstring_encode`], used only to prove the framing
    /// round-trips (hence is injective).
    fn netstring_decode(mut bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut fields = Vec::new();
        while !bytes.is_empty() {
            let colon = bytes.iter().position(|&b| b == b':').unwrap();
            let len: usize = std::str::from_utf8(&bytes[..colon])
                .unwrap()
                .parse()
                .unwrap();
            let start = colon + 1;
            fields.push(bytes[start..start + len].to_vec());
            bytes = &bytes[start + len + 1..]; // skip the trailing comma
        }
        fields
    }

    use proptest::prelude::*;

    proptest! {
        /// #1 Netstring framing is injective (it round-trips), so no two distinct
        /// field lists can ever collide - the fatal collision class, ruled out for
        /// all inputs, not just the hand-picked vectors.
        #[test]
        fn netstring_framing_round_trips(
            fields in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..40), 0..8),
        ) {
            let refs: Vec<&[u8]> = fields.iter().map(|f| f.as_slice()).collect();
            prop_assert_eq!(netstring_decode(&netstring_encode(&refs)), fields);
        }

        /// #2 An id is computable exactly when the content has no forbidden C0
        /// control (the exact `reject_controls` rule), across arbitrary strings.
        #[test]
        fn id_ok_iff_no_forbidden_control(s in any::<String>()) {
            // Front held clean ("q"), so the error tracks the answer alone.
            let forbidden = s.chars().any(|c| (c as u32) < 0x20 && !WHITESPACE.contains(&c));
            prop_assert_eq!(plain_id_hex("deck", "q", &s).is_err(), forbidden);
        }

        /// #3 normalize_ws is idempotent and leaves no collapsible whitespace.
        #[test]
        fn normalize_ws_is_idempotent_and_clean(s in any::<String>()) {
            let once = normalize_ws(&s);
            prop_assert_eq!(normalize_ws(&once), once.clone());
            prop_assert!(!once.contains("  "));
            prop_assert!(!once.starts_with(' ') && !once.ends_with(' '));
        }

        /// #4 A plain card and a reversed card sharing the same back never collide.
        #[test]
        fn plain_and_reversed_never_collide(s in "[^\\x00-\\x1f]*") {
            prop_assert_ne!(
                plain_id_hex("deck", "q", &s).unwrap(),
                reversed_id_hex("deck", "q", &s).unwrap(),
            );
        }
    }
}
