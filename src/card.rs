//! The flashcard model and its identity.

use std::{path::PathBuf, sync::Arc};

use crate::{answer::Input, depth::Reveal, token};

/// Which way a card is reviewed. Set per card (or per deck) with
/// `% direction:`; `both` generates a forward and a reversed card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
pub enum Direction {
    /// The card as written: front asks, back answers (the default).
    #[default]
    Forward,
    /// Only the swapped card: back asks, front answers.
    Reverse,
    /// Both the forward and the reversed card.
    Both,
}

impl Direction {
    /// Parses the directive value name (case-insensitive), mirroring the clap
    /// value names; the gated parity test keeps the two in step.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "forward" => Some(Self::Forward),
            "reverse" => Some(Self::Reverse),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

/// A single flashcard.
#[derive(Clone, Debug)]
pub struct Card {
    /// The subject this card belongs to. This is the file name of the deck it
    /// was parsed from (e.g. `golang.md`).
    pub subject: Arc<str>,
    /// The front side: the question or task description.
    pub front: String,
    /// Extra display lines shown below the front. Used by cloze cards for
    /// the masked text; empty for plain cards.
    pub context: Vec<String>,
    /// The back side: the answer lines the user has to produce.
    pub back: Vec<String>,
    /// An optional note providing helpful context, shown after answering.
    pub note: Option<String>,
    /// The 1-based line number of the front side in the deck file.
    pub line: usize,
    /// Lines hashed for the card's identity instead of `back`. `None` for
    /// plain cards (which hash their back lines). Cloze sub-cards hash each
    /// line's text with the cloze delimiters stripped (so restyling the markup
    /// never reshuffles ids), plus a per-hole index — keeping their identity
    /// stable across rewording the front and unique even when two holes hold the
    /// same text.
    pub hash_lines: Option<Vec<String>>,
    /// Per-card reveal-method (`% reveal:` on the card, else the deck's
    /// `% reveal:`) — how the answer is uncovered (flip / cloze / line),
    /// independent of depth. `None` falls back to `Reveal::Flip`. Not part of
    /// the identity hash — how the answer is revealed is a review property, not
    /// content.
    pub reveal: Option<Reveal>,
    /// Per-card input method (`% input:` on the card, else the deck's). `None`
    /// falls back to `Input::Type`. Not part of the identity hash — how you
    /// answer is a review property, not content.
    pub input: Option<Input>,
    /// Declared review direction (`% direction:`), consumed when the deck is
    /// loaded to expand `both`/`reverse` into cards. `None` means forward. Not
    /// part of the identity hash.
    pub direction: Option<Direction>,
    /// Question-side image (`% img:`). Holds the raw value as written after
    /// parsing; rewritten to an absolute path when the deck is loaded. Not
    /// part of the identity hash.
    pub image: Option<PathBuf>,
    /// Answer-side image (`% img-back:`), shown with the revealed back. Same
    /// lifecycle as `image`. Not part of the identity hash.
    pub image_back: Option<PathBuf>,
    /// Source locator for a trace checkpoint (`% at:`): where in the
    /// `% source:` the revealed ground truth lives (e.g. `card.rs:151-158`).
    /// Read live when walking a trace; see [`crate::trace`]. Not part of the
    /// identity hash — it points at the source, it is not card content.
    pub at: Option<String>,
    /// Where a frozen `% at:` snapshot came from in the live source, relative to
    /// the effective `% origin:` (`% at: 29.rs from src/caching.rs:46-66` →
    /// `src/caching.rs:46-66`). Drives display relabeling, tutor grounding, and
    /// drift detection. Not part of the identity hash.
    pub at_origin: Option<String>,
    /// Per-card `% origin:` override of the deck/workspace origin (the crate root
    /// this card's frozen source lives in). Effective origin = card else deck else
    /// workspace `[defaults]`. Not part of the identity hash.
    pub origin: Option<String>,
    /// Trace-checkpoint "givens" (`% given:` lines, repeatable): the off-screen
    /// symbols the question/excerpt leans on, each as `name — meaning`. Shown
    /// as a list under the question when walking (scaffolding so the
    /// excerpt can stay tight; see [`crate::trace`]). Not part of the
    /// identity hash.
    pub givens: Vec<String>,
    /// A display-only reshape of the answer lines from `augment --target format`.
    /// When `Some`, every reveal/render path uses these instead of `back`. NOT
    /// part of the identity hash — `id()` always hashes the original `back`.
    pub display_back: Option<Vec<String>>,
    /// The minted identity token (L1 spec §1). `None` until the deck is
    /// stamped. Dormant: [`Card::id`] ignores it until the flip task.
    pub token: Option<Arc<str>>,
    /// For a cloze sub-card: this hole's 0-based document-order index.
    /// Dormant like `token`.
    pub hole: Option<u32>,
    /// For a cloze card: EVERY hole's realignment fingerprint, in document
    /// order, the same block-level vector on every sub-card of the block (the
    /// parser fills it), so any one sub-card can rebuild the whole card's
    /// [`crate::store::CardRecords`]. Empty for a plain card. Store-internal,
    /// never part of the identity hash.
    pub block_holes: Vec<crate::store::HoleFingerprint>,
    /// True for the swapped half of a `direction: both`/`reverse` card.
    /// Dormant like `token`.
    pub reversed: bool,
    /// The §7 canonical-content fingerprint of this card
    /// ([`crate::l1::content_fingerprint`] over front + answer, with cloze
    /// markers kept as literal text). NOT part of the identity token: it is the
    /// content-identity key the remediation/tutor dedup and the deck-side dedup
    /// baseline compare on. Every sub-card of a cloze block carries the *block*
    /// fingerprint (the parser sets it), so a multi-hole block dedups and revives
    /// as one unit; a plain card whose visible words merely repeat a hole's hidden
    /// text does not collide, because the block's answer carries the literal
    /// `\cloze{...}` markers and the plain answer does not.
    pub content_fingerprint: u64,
}

impl Card {
    /// Creates a plain (non-cloze) card.
    pub fn plain(
        subject: Arc<str>,
        front: String,
        back: Vec<String>,
        note: Option<String>,
        line: usize,
    ) -> Self {
        // The default fingerprint is this card's own front + answer, correct for
        // every non-cloze card. The parser overrides it for cloze sub-cards with
        // the block-level value (front + raw answer, markers literal) so all holes
        // of one block share a fingerprint.
        let content_fingerprint = crate::l1::content_fingerprint(&front, &back);
        Self {
            subject,
            front,
            context: Vec::new(),
            back,
            note,
            line,
            hash_lines: None,
            reveal: None,
            input: None,
            direction: None,
            image: None,
            image_back: None,
            at: None,
            at_origin: None,
            origin: None,
            givens: Vec::new(),
            display_back: None,
            token: None,
            hole: None,
            block_holes: Vec::new(),
            reversed: false,
            content_fingerprint,
        }
    }

    /// The answer lines to reveal/render: the `display_back` reshape when present
    /// (from `augment --target format`), otherwise the card's own `back`.
    pub fn back_for_display(&self) -> &[String] {
        self.display_back.as_deref().unwrap_or(&self.back)
    }

    /// The swapped card for dual-direction review: the question becomes the old
    /// answer and the answer becomes the old front. It keeps the same source
    /// `line`, so it shares the forward card's sibling group (the session keeps
    /// them apart and removes them together). Its identity differs naturally
    /// because it carries the same token with the `-r` suffix. Only meaningful
    /// for plain cards.
    pub fn reversed(&self) -> Card {
        let mut card = Card::plain(
            Arc::clone(&self.subject),
            self.back.join("\n"),
            vec![self.front.clone()],
            self.note.clone(),
            self.line,
        );
        card.reveal = self.reveal;
        card.input = self.input;
        // Swap the image sides: a question-side image becomes the answer's, and
        // vice versa, so a `direction: both` visual card reverses sensibly.
        card.image = self.image_back.clone();
        card.image_back = self.image.clone();
        // The swapped half shares the forward card's token and marks itself
        // reversed, so the token flip can compose `<token>-r` ids.
        card.token = self.token.clone();
        card.reversed = true;
        // Carry the forward card's content fingerprint rather than recomputing
        // over the swapped sides (the cloze block-fingerprint override in
        // `plain` is the same precedent): one authored card is one §7 content
        // unit, so records, the reclaim wanted-set, and the reclaiming stamp all
        // key on a single value.
        card.content_fingerprint = self.content_fingerprint;
        card
    }

    /// The card's identity for the progress store: its minted token, verbatim,
    /// with a suffix for a cloze hole (`token-N`, 0-based document order) or the
    /// reversed half of a dual-direction card (`token-r`). A card's id **is** its
    /// token — editing the front, note, or answer never changes it, while
    /// re-stamping (a new token) does. `None` until the deck is stamped (no token
    /// yet): the session/store boundary ([`crate::assemble::exclude_unstamped`])
    /// drops such cards before they can key any progress.
    pub fn id(&self) -> Option<String> {
        self.token
            .as_deref()
            .map(|token| token::card_id(token, self.hole, self.reversed))
    }

    /// Appends freshly-saved note lines to this card's in-memory `note`, joined
    /// the same way the parser joins a card's `! ` lines. Lets a note saved from
    /// the ask tutor show on the card immediately, without re-reading the deck.
    pub fn append_note(&mut self, notes: &[String]) {
        if notes.is_empty() {
            return;
        }
        let addition = notes.join("\n");
        match &mut self.note {
            Some(note) => {
                note.push('\n');
                note.push_str(&addition);
            }
            slot => *slot = Some(addition),
        }
    }
}

impl Eq for Card {}
// Identity is the token triple `(token, hole, reversed)`, exactly what `id()`
// composes — content (front/back/note) is deliberately not part of it. Two
// UNSTAMPED cards (both `token: None`) compare equal when their hole/reversed
// match; that is harmless because the session/store boundary excludes tokenless
// cards before any equality-keyed step reaches them.
impl PartialEq for Card {
    fn eq(&self, other: &Self) -> bool {
        self.token == other.token && self.hole == other.hole && self.reversed == other.reversed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(subject: &str, front: &str, back: &[&str], note: Option<&str>) -> Card {
        Card::plain(
            Arc::from(subject),
            front.to_string(),
            back.iter().map(|s| s.to_string()).collect(),
            note.map(|s| s.to_string()),
            1,
        )
    }

    /// A card carrying a literal token, so its identity is a real string id
    /// rather than the unstamped `None`/`0` sentinel.
    fn stamped(subject: &str, front: &str, back: &[&str], note: Option<&str>, token: &str) -> Card {
        let mut c = card(subject, front, back, note);
        c.token = Some(Arc::from(token));
        c
    }

    #[test]
    fn an_unstamped_cards_id_is_none() {
        // Identity is the token, so an unstamped card has no id at all; the
        // session/store boundary excludes it before it keys any progress.
        let c = card("subject1", "hello", &["world"], None);
        assert_eq!(None, c.id());
    }

    #[test]
    fn a_stamped_cards_id_is_its_token_verbatim() {
        // A plain stamped card's id IS its token, byte for byte — no hashing,
        // no suffix.
        let c = stamped("s", "f", &["b"], None, "9w2c7xkq");
        assert_eq!(Some("9w2c7xkq".to_string()), c.id());
    }

    #[test]
    fn sub_ids_carry_hole_and_reversed_suffixes() {
        // A cloze hole appends `-N` (0-based document order); the reversed half
        // of a dual-direction card appends `-r`.
        let mut c = stamped("s", "f", &["b"], None, "q1");
        c.hole = Some(2);
        assert_eq!(Some("q1-2".to_string()), c.id());
        c.hole = None;
        c.reversed = true;
        assert_eq!(Some("q1-r".to_string()), c.id());
    }

    #[test]
    fn distinct_tokens_yield_distinct_ids() {
        // Different tokens are different ids; the same token is the same id
        // regardless of content.
        let a = stamped("s", "f", &["b"], None, "q1");
        let b = stamped("s", "f", &["b"], None, "q2");
        let a2 = stamped("s", "different front", &["different back"], None, "q1");
        assert_ne!(a.id(), b.id());
        assert_eq!(a.id(), a2.id()); // identity is the token, not the content
    }

    #[test]
    fn id_ignores_front_and_note() {
        // Same token, so same identity regardless of front/note/back edits.
        let a = stamped("subject1", "hello", &["world"], None, "q1");
        let b = stamped("subject1", "hi there", &["world"], Some("a note"), "q1");
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn append_note_creates_then_joins_with_newlines() {
        let mut c = card("d.md", "front", &["back"], None);
        c.append_note(&[]); // nothing to add
        assert_eq!(None, c.note);
        c.append_note(&["first".to_string()]);
        assert_eq!(Some("first".to_string()), c.note);
        c.append_note(&["second".to_string(), "third".to_string()]);
        assert_eq!(Some("first\nsecond\nthird".to_string()), c.note);
    }

    #[test]
    fn id_ignores_reveal() {
        // The reveal-method is a review property, not content — it must not
        // change identity.
        let mut a = stamped("subject1", "hello", &["world"], None, "q1");
        let mut b = stamped("subject1", "hello", &["world"], None, "q1");
        a.reveal = Some(Reveal::Flip);
        b.reveal = Some(Reveal::Line);
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn reversed_swaps_sides_keeps_note_and_line() {
        let mut fwd = stamped(
            "vocab.md",
            "purported",
            &["angeblich"],
            Some("a note"),
            "q1",
        );
        fwd.reveal = Some(Reveal::Line);
        let rev = fwd.reversed();
        assert_eq!("angeblich", rev.front);
        assert_eq!(vec!["purported"], rev.back);
        assert_eq!(fwd.note, rev.note);
        assert_eq!(fwd.line, rev.line); // same source line -> sibling group
        assert_eq!(fwd.reveal, rev.reveal);
        // Distinct identity: the reversed half carries the same token but the
        // `-r` suffix (`q1` vs `q1-r`).
        assert_ne!(fwd.id(), rev.id());
        assert_eq!(Some("q1".to_string()), fwd.id());
        assert_eq!(Some("q1-r".to_string()), rev.id());
        // Plain cards are unreversed, the swapped half is marked, and the token
        // is carried over.
        assert!(!fwd.reversed);
        assert!(rev.reversed);
        assert_eq!(fwd.token, rev.token);
    }

    #[test]
    fn reversed_swaps_image_sides() {
        let mut fwd = card("g.md", "name this chord", &["G major"], None);
        fwd.image_back = Some(PathBuf::from("/tabs/g.png"));
        let rev = fwd.reversed();
        // The answer-side image becomes the question-side image and vice versa.
        assert_eq!(Some(PathBuf::from("/tabs/g.png")), rev.image);
        assert_eq!(None, rev.image_back);
    }

    #[test]
    fn id_ignores_image() {
        let mut a = stamped("s", "f", &["b"], None, "q1");
        let b = stamped("s", "f", &["b"], None, "q1");
        a.image = Some(PathBuf::from("/imgs/a.png"));
        a.at = Some("card.rs:1-9".to_string());
        a.givens = vec!["state — the parser position".to_string()];
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn display_back_overrides_render_but_not_identity() {
        let mut c = stamped("d.md", "f", &["Chain, Version"], None, "q1");
        let before = c.id();
        c.display_back = Some(vec!["Protocol: Chain".into(), "Version".into()]);
        assert_eq!(c.back_for_display(), ["Protocol: Chain", "Version"]);
        assert_eq!(c.id(), before); // display_back never enters the identity
    }

    #[test]
    fn input_does_not_affect_card_identity() {
        let mut a = stamped("d.md", "front", &["the answer"], None, "q1");
        let mut b = stamped("d.md", "front", &["the answer"], None, "q1");
        a.input = Some(Input::Draw);
        b.input = None;
        assert_eq!(a.id(), b.id()); // input is a review property, not content
    }
}

#[cfg(all(test, feature = "full"))]
mod clap_parity {
    use clap::ValueEnum;

    use super::*;

    /// The hand-written `parse` and the clap value names must agree on every
    /// variant, or a `%` directive would parse differently from the CLI flag.
    #[test]
    fn parse_matches_the_clap_value_names() {
        for variant in Direction::value_variants() {
            let name = variant.to_possible_value().expect("a value name");
            assert_eq!(
                Some(*variant),
                Direction::parse(name.get_name()),
                "{name:?}"
            );
        }
        assert_eq!(None, Direction::parse("no-such-value"));
    }
}
