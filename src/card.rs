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

#[derive(Clone, Debug)]
pub struct Card {
    pub subject: Arc<str>,
    pub front: String,
    pub context: Vec<String>,
    pub back: Vec<String>,
    pub note: Option<String>,
    pub line: usize,
    pub hash_lines: Option<Vec<String>>,
    pub reveal: Option<Reveal>,
    pub input: Option<Input>,
    pub direction: Option<Direction>,
    pub image: Option<PathBuf>,
    pub image_back: Option<PathBuf>,
    pub at: Option<String>,
    pub at_origin: Option<String>,
    pub origin: Option<String>,
    pub givens: Vec<String>,
    pub display_back: Option<Vec<String>>,
    pub token: Option<Arc<str>>,
    pub hole: Option<u32>,
    pub block_holes: Vec<crate::store::HoleFingerprint>,
    pub reversed: bool,
    pub content_fingerprint: u64,
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
        card.reveal = self.reveal;
        card.input = self.input;
        card.image = self.image_back.clone();
        card.image_back = self.image.clone();
        // The reversed half keeps the same token so id() can compose the "-r" suffix from it.
        card.token = self.token.clone();
        card.reversed = true;
        // Reuses the forward card's fingerprint instead of recomputing over swapped sides: one
        // authored card is one content unit.
        card.content_fingerprint = self.content_fingerprint;
        card
    }

    pub fn id(&self) -> Option<String> {
        self.token
            .as_deref()
            .map(|token| token::card_id(token, self.hole, self.reversed))
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
    fn reversed_swaps_image_sides() {
        let mut fwd = card("g.md", "name this chord", &["G major"], None);
        fwd.image_back = Some(PathBuf::from("/tabs/g.png"));
        let rev = fwd.reversed();
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
