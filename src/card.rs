//! The flashcard model and its identity hash.

use std::{hash::Hasher, path::PathBuf, sync::Arc};

use twox_hash::XxHash64;

use crate::{answer::Input, depth::Reveal};

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
    /// was parsed from (e.g. `golang.txt`).
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
    /// because `id()` hashes the new back (the old front). Only meaningful for
    /// plain cards.
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
        card
    }

    /// Returns the identity hash of this card.
    ///
    /// Plain cards hash the subject bytes followed by a whitespace-normalized
    /// version of the answer (all back lines joined and collapsed to single
    /// spaces), ignoring front and note, so progress survives rewording the
    /// front, adding notes, or reformatting the answer across lines. Cloze
    /// cards hash `hash_lines` instead — each line whitespace-normalized — so
    /// restyling the `{{ }}` markup never reshuffles ids. A change in words
    /// still changes the id. This value keys the progress store and must stay
    /// stable across versions, or existing progress would be orphaned.
    pub fn id(&self) -> u64 {
        let mut hasher = XxHash64::default();
        hasher.write(self.subject.as_bytes());
        match &self.hash_lines {
            // Cloze: keep the per-line (per-hole) structure, but normalize each
            // line's internal whitespace so restyling never reshuffles ids.
            Some(lines) => {
                for line in lines {
                    hasher.write(normalize_ws(line).as_bytes());
                }
            }
            // Plain: hash the answer as one whitespace-normalized string, so the
            // same words across any number of lines (or any indentation) give the
            // same id, while a change in words still changes it.
            None => hasher.write(normalize_ws(&self.back.join(" ")).as_bytes()),
        }
        hasher.finish()
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

/// Collapses every run of whitespace (spaces, tabs, newlines) to a single space
/// and trims the ends, so reformatting an answer's layout never changes its
/// identity ([`Card::id`]) while a change in words still does.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Eq for Card {}
impl PartialEq for Card {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
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

    #[test]
    fn id_ignores_front_and_note() {
        let a = card("subject1", "hello", &["world"], None);
        let b = card("subject1", "hi there", &["world"], Some("a note"));
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn append_note_creates_then_joins_with_newlines() {
        let mut c = card("d.txt", "front", &["back"], None);
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
        let mut a = card("subject1", "hello", &["world"], None);
        let mut b = card("subject1", "hello", &["world"], None);
        a.reveal = Some(Reveal::Flip);
        b.reveal = Some(Reveal::Line);
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn reversed_swaps_sides_keeps_note_and_line() {
        let mut fwd = card("vocab.txt", "purported", &["angeblich"], Some("a note"));
        fwd.reveal = Some(Reveal::Line);
        let rev = fwd.reversed();
        assert_eq!("angeblich", rev.front);
        assert_eq!(vec!["purported"], rev.back);
        assert_eq!(fwd.note, rev.note);
        assert_eq!(fwd.line, rev.line); // same source line -> sibling group
        assert_eq!(fwd.reveal, rev.reveal);
        assert_ne!(fwd.id(), rev.id()); // distinct identity (hashes new back)
    }

    #[test]
    fn reversed_swaps_image_sides() {
        let mut fwd = card("g.txt", "name this chord", &["G major"], None);
        fwd.image_back = Some(PathBuf::from("/tabs/g.png"));
        let rev = fwd.reversed();
        // The answer-side image becomes the question-side image and vice versa.
        assert_eq!(Some(PathBuf::from("/tabs/g.png")), rev.image);
        assert_eq!(None, rev.image_back);
    }

    #[test]
    fn id_ignores_image() {
        let mut a = card("s", "f", &["b"], None);
        let b = card("s", "f", &["b"], None);
        a.image = Some(PathBuf::from("/imgs/a.png"));
        a.at = Some("card.rs:1-9".to_string());
        a.givens = vec!["state — the parser position".to_string()];
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn id_depends_on_subject() {
        let a = card("subject1", "hello", &["world"], None);
        let b = card("subject2", "hello", &["world"], None);
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn id_depends_on_back() {
        let a = card("subject1", "hello", &["world"], None);
        let b = card("subject1", "hello", &["worlds"], None);
        let c = card("subject1", "hello", &["world", "again"], None);
        assert_ne!(a.id(), b.id());
        assert_ne!(a.id(), c.id());
    }

    #[test]
    fn id_uses_hash_lines_when_present() {
        let mut a = card("s", "front", &["typed answer"], None);
        let b = card("s", "front", &["typed answer"], None);
        a.hash_lines = Some(vec![
            "raw {typed answer}".to_string(),
            "#cloze:0".to_string(),
        ]);
        assert_ne!(a.id(), b.id());
    }

    /// Pins the identity hash to a known value so it stays stable across
    /// versions — changing it would orphan everyone's stored progress.
    #[test]
    fn id_is_stable() {
        let c = card(
            "sample_box.txt",
            "How to define an executable program",
            &["main"],
            None,
        );
        assert_eq!(9405983226316857161, c.id());
    }

    #[test]
    fn id_is_whitespace_insensitive_across_lines() {
        // The same answer as one line vs. split across several lines (the exact
        // reshape the formatter and generate produce) must be the same card.
        let one_line = card("d.txt", "f", &["A, B, C"], None);
        let many_lines = card("d.txt", "f", &["A,", "B,", "C"], None);
        assert_eq!(one_line.id(), many_lines.id());
    }

    #[test]
    fn id_ignores_irregular_internal_whitespace() {
        let a = card("d.txt", "f", &["foo bar"], None);
        let b = card("d.txt", "f", &["foo    bar"], None);
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn id_still_changes_on_word_edit() {
        let a = card("d.txt", "f", &["A, B, C"], None);
        let b = card("d.txt", "f", &["A, B, D"], None);
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn display_back_overrides_render_but_not_identity() {
        let mut c = card("d.txt", "f", &["Chain, Version"], None);
        let before = c.id();
        c.display_back = Some(vec!["Protocol: Chain".into(), "Version".into()]);
        assert_eq!(c.back_for_display(), ["Protocol: Chain", "Version"]);
        assert_eq!(c.id(), before); // display_back never enters the hash
    }

    #[test]
    fn input_does_not_affect_card_identity() {
        let mut a = card("d.txt", "front", &["the answer"], None);
        let mut b = card("d.txt", "front", &["the answer"], None);
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
