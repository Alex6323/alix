use std::{path::PathBuf, sync::Arc};

use crate::{answer::Input, depth::Reveal, token};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
pub enum Direction {
    #[default]
    Forward,
    Reverse,
    Both,
}

impl Direction {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "forward" => Some(Self::Forward),
            "reverse" => Some(Self::Reverse),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

/// One member of a region group: its stamp (None while unstamped) and what
/// its mask hides. Provenance stays member-level so the future MC-family
/// spec can see shape, never a flattened list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupMember {
    pub stamp: Option<Arc<str>>,
    pub hidden: Option<String>,
    /// The member's directive line: the removal address of exactly this
    /// region, never a card-block boundary.
    pub line: usize,
}

/// What a region card asks (ADR 0034): one region, or a named group asking
/// every member. An unstamped region (or a group with one) has no usable id
/// and its card is excluded at the session boundary like any token-less card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionSlot {
    Single {
        stamp: Option<Arc<str>>,
        hidden: Option<String>,
        line: usize,
    },
    Group {
        name: String,
        hash: Option<Arc<str>>,
        members: Vec<GroupMember>,
    },
}

impl RegionSlot {
    /// The directive line(s) this card owns in the file: what removal
    /// deletes instead of a card block.
    pub fn directive_lines(&self) -> Vec<usize> {
        match self {
            RegionSlot::Single { line, .. } => vec![*line],
            RegionSlot::Group { members, .. } => members.iter().map(|m| m.line).collect(),
        }
    }

    pub fn first_line(&self) -> usize {
        self.directive_lines().into_iter().min().unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardImage {
    pub src: PathBuf,
    pub alt: Option<String>,
    /// Regions bound to this media element (ADR 0034), in file order.
    pub regions: Vec<crate::parser::region::RawRegion>,
    /// The one optional viewport onto this media element.
    pub crop: Option<crate::parser::region::RawCrop>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCitation {
    /// The `at:` field: the real source path and lines, never an asset name.
    pub locator: String,
    pub fingerprint: Option<u64>,
    /// The `asset:` field: the frozen `sha256-<hex>.<ext>` object name,
    /// present only on a frozen citation.
    pub asset: Option<String>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub subject: Arc<str>,
    /// The owning deck's stable id; empty when the deck has none
    /// (uninitialized deck, or a card built outside deck context).
    pub deck_id: Arc<str>,
    pub front: String,
    pub context: Vec<String>,
    /// Whether `context` is the question (a cloze sentence, which leads) or a
    /// label for the front (a table title, which steps back).
    pub context_leads: bool,
    pub back: Vec<String>,
    pub note: Option<String>,
    pub line: usize,
    pub hash_lines: Option<Vec<String>>,
    pub reveal: Option<Reveal>,
    pub input: Option<Input>,
    pub direction: Option<Direction>,
    pub images: Vec<CardImage>,
    pub images_back: Vec<CardImage>,
    pub citations: Vec<SourceCitation>,
    pub givens: Vec<String>,
    pub display_back: Option<Vec<String>>,
    pub token: Option<Arc<str>>,
    pub row: Option<Arc<str>>,
    /// Resolved table-over-deck at parse time; None means the default (on).
    pub sampling: Option<bool>,
    pub hole: Option<u32>,
    /// The hole's authored name, addressing it within its own block for a
    /// per-hole payload. Never an identity: see ADR 0032.
    pub hole_name: Option<String>,
    /// Set when this card's hole was cut out of a formula, which decides how
    /// the answer is asked for: a formula's piece is drawn, not typed.
    pub math_hole: bool,
    pub block_holes: Vec<crate::store::HoleFingerprint>,
    pub reversed: bool,
    pub content_fingerprint: u64,
    pub authored_distractors: Vec<String>,
    /// `span`-shaped regions bound to the answer block (ADR 0034), in file
    /// order; geometric regions ride their media element in `images`.
    pub span_regions: Vec<crate::parser::region::RawRegion>,
    /// Set on a synthesized region card: which region(s) this card asks.
    pub region: Option<RegionSlot>,
}

impl Card {
    pub fn plain(
        subject: Arc<str>,
        front: String,
        back: Vec<String>,
        note: Option<String>,
        line: usize,
    ) -> Self {
        // The parser overrides this for cloze sub-cards with a shared block-level fingerprint; this
        // default fits every other card.
        let content_fingerprint = crate::parser::content_fingerprint(&front, &back);
        Self {
            subject,
            deck_id: Arc::from(""),
            front,
            context: Vec::new(),
            context_leads: false,
            back,
            note,
            line,
            hash_lines: None,
            reveal: None,
            input: None,
            direction: None,
            images: Vec::new(),
            images_back: Vec::new(),
            citations: Vec::new(),
            givens: Vec::new(),
            display_back: None,
            token: None,
            row: None,
            sampling: None,
            hole: None,
            hole_name: None,
            math_hole: false,
            block_holes: Vec::new(),
            reversed: false,
            content_fingerprint,
            authored_distractors: Vec::new(),
            span_regions: Vec::new(),
            region: None,
        }
    }

    pub fn back_for_display(&self) -> &[String] {
        self.display_back.as_deref().unwrap_or(&self.back)
    }

    pub fn reversed(&self) -> Card {
        let mut card = Card::plain(
            Arc::clone(&self.subject),
            self.back.join("\n"),
            vec![self.front.clone()],
            self.note.clone(),
            self.line,
        );
        card.deck_id = Arc::clone(&self.deck_id);
        card.reveal = self.reveal;
        card.input = self.input;
        card.images = self.images_back.clone();
        card.images_back = self.images.clone();
        card.citations = self.citations.clone();
        // The reversed half keeps the same token so id() can compose the "-r" suffix from it.
        card.token = self.token.clone();
        card.row = self.row.clone();
        card.sampling = self.sampling;
        card.context = self.context.clone();
        card.context_leads = self.context_leads;
        card.reversed = true;
        // Reuses the forward card's fingerprint instead of recomputing over swapped sides: one
        // authored card is one content unit.
        card.content_fingerprint = self.content_fingerprint;
        card
    }

    pub fn id(&self) -> Option<String> {
        let token = self.token.as_deref()?;
        let region = match &self.region {
            None => None,
            Some(RegionSlot::Single { stamp, .. }) => {
                Some(token::RegionRef::Single(stamp.as_deref()?))
            }
            Some(RegionSlot::Group { hash, .. }) => Some(token::RegionRef::Group(hash.as_deref()?)),
        };
        Some(token::card_id(
            token,
            self.row.as_deref(),
            self.hole,
            self.reversed,
            region,
        ))
    }

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
// Equality is (token, hole, reversed) only; unstamped cards (token: None) compare equal, which is
// harmless since the session/store boundary excludes them first.
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

    fn stamped(subject: &str, front: &str, back: &[&str], note: Option<&str>, token: &str) -> Card {
        let mut c = card(subject, front, back, note);
        c.token = Some(Arc::from(token));
        c
    }

    #[test]
    fn an_unstamped_cards_id_is_none() {
        let c = card("subject1", "hello", &["world"], None);
        assert_eq!(None, c.id());
    }

    #[test]
    fn a_stamped_cards_id_is_its_token_verbatim() {
        let c = stamped("s", "f", &["b"], None, "9w2c7xkq");
        assert_eq!(Some("9w2c7xkq".to_string()), c.id());
    }

    #[test]
    fn sub_ids_carry_hole_and_reversed_suffixes() {
        let mut c = stamped("s", "f", &["b"], None, "q1");
        c.hole = Some(2);
        assert_eq!(Some("q1-2".to_string()), c.id());
        c.hole = None;
        c.reversed = true;
        assert_eq!(Some("q1-r".to_string()), c.id());
    }

    #[test]
    fn distinct_tokens_yield_distinct_ids() {
        let a = stamped("s", "f", &["b"], None, "q1");
        let b = stamped("s", "f", &["b"], None, "q2");
        let a2 = stamped("s", "different front", &["different back"], None, "q1");
        assert_ne!(a.id(), b.id());
        assert_eq!(a.id(), a2.id());
    }

    #[test]
    fn id_ignores_front_and_note() {
        let a = stamped("subject1", "hello", &["world"], None, "q1");
        let b = stamped("subject1", "hi there", &["world"], Some("a note"), "q1");
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn equality_is_exactly_token_hole_and_direction() {
        let base = stamped("deck", "front", &["back"], None, "tok");
        let mut presentation_only = base.clone();
        presentation_only.front = "different".to_string();
        assert_eq!(base, presentation_only);

        let mut token = base.clone();
        token.token = Some("other".into());
        assert_ne!(base, token);
        let mut hole = base.clone();
        hole.hole = Some(1);
        assert_ne!(base, hole);
        let mut reversed = base.clone();
        reversed.reversed = true;
        assert_ne!(base, reversed);
    }

    #[test]
    fn append_note_creates_then_joins_with_newlines() {
        let mut c = card("d.md", "front", &["back"], None);
        c.append_note(&[]);
        assert_eq!(None, c.note);
        c.append_note(&["first".to_string()]);
        assert_eq!(Some("first".to_string()), c.note);
        c.append_note(&["second".to_string(), "third".to_string()]);
        assert_eq!(Some("first\nsecond\nthird".to_string()), c.note);
    }

    #[test]
    fn id_ignores_reveal() {
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
        assert_eq!(fwd.line, rev.line);
        assert_eq!(fwd.reveal, rev.reveal);
        assert_ne!(fwd.id(), rev.id());
        assert_eq!(Some("q1".to_string()), fwd.id());
        assert_eq!(Some("q1-r".to_string()), rev.id());
        assert!(!fwd.reversed);
        assert!(rev.reversed);
        assert_eq!(fwd.token, rev.token);
    }

    #[test]
    fn reversed_keeps_the_owning_decks_id() {
        let mut fwd = stamped("vocab.md", "purported", &["angeblich"], None, "q1");
        fwd.deck_id = Arc::from("a-deck-token");
        let rev = fwd.reversed();
        assert_eq!(fwd.deck_id, rev.deck_id);
    }

    #[test]
    fn reversed_swaps_image_sides() {
        let mut fwd = card("g.md", "name this chord", &["G major"], None);
        fwd.images_back = vec![CardImage {
            src: PathBuf::from("/tabs/g.png"),
            alt: None,
            regions: Vec::new(),
            crop: None,
        }];
        let rev = fwd.reversed();
        assert_eq!(
            vec![PathBuf::from("/tabs/g.png")],
            rev.images.iter().map(|i| i.src.clone()).collect::<Vec<_>>()
        );
        assert!(rev.images_back.is_empty());
    }

    #[test]
    fn id_ignores_image() {
        let mut a = stamped("s", "f", &["b"], None, "q1");
        let b = stamped("s", "f", &["b"], None, "q1");
        a.images = vec![CardImage {
            src: PathBuf::from("/imgs/a.png"),
            alt: None,
            regions: Vec::new(),
            crop: None,
        }];
        a.citations.push(SourceCitation {
            locator: "card.rs:1-9".to_string(),
            fingerprint: Some(42),
            asset: None,
            line: 4,
        });
        a.givens = vec!["state — the parser position".to_string()];
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn display_back_overrides_render_but_not_identity() {
        let mut c = stamped("d.md", "f", &["Chain, Version"], None, "q1");
        let before = c.id();
        c.display_back = Some(vec!["Protocol: Chain".into(), "Version".into()]);
        assert_eq!(c.back_for_display(), ["Protocol: Chain", "Version"]);
        assert_eq!(c.id(), before);
    }

    #[test]
    fn input_does_not_affect_card_identity() {
        let mut a = stamped("d.md", "front", &["the answer"], None, "q1");
        let mut b = stamped("d.md", "front", &["the answer"], None, "q1");
        a.input = Some(Input::Draw);
        b.input = None;
        assert_eq!(a.id(), b.id());
    }
}

#[cfg(all(test, feature = "full"))]
mod clap_parity {
    use clap::ValueEnum;

    use super::*;

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
