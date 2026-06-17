//! The flashcard model and its identity hash.

use std::{hash::Hasher, sync::Arc};

use twox_hash::XxHash64;

use crate::answer::Mode;

/// Which way a card is reviewed. Set per card (or per deck) with
/// `% direction:`; `both` generates a forward and a reversed card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Direction {
    /// The card as written: front asks, back answers (the default).
    #[default]
    Forward,
    /// Only the swapped card: back asks, front answers.
    Reverse,
    /// Both the forward and the reversed card.
    Both,
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
    /// plain cards (which hash their back lines). Cloze sub-cards hash the raw
    /// marked-up lines plus a hole index so their identity survives rewording
    /// the front and stays unique even when two holes contain the same text.
    pub hash_lines: Option<Vec<String>>,
    /// Per-card answer-mode override (`% mode:` on the card, else the deck's
    /// `% mode:`). `None` falls back to the CLI flag / built-in default. Not
    /// part of the identity hash — mode is a review property, not content.
    pub mode: Option<Mode>,
    /// Declared review direction (`% direction:`), consumed when the deck is
    /// loaded to expand `both`/`reverse` into cards. `None` means forward. Not
    /// part of the identity hash.
    pub direction: Option<Direction>,
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
            mode: None,
            direction: None,
        }
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
        card.mode = self.mode;
        card
    }

    /// Returns the identity hash of this card.
    ///
    /// Plain cards hash the subject bytes followed by the bytes of each
    /// (trimmed) back line with an unseeded `XxHash64`, ignoring front and
    /// note, so progress survives rewording the front and adding notes. This
    /// value keys the progress store and must stay stable across versions, or
    /// existing progress would be orphaned.
    pub fn id(&self) -> u64 {
        let mut hasher = XxHash64::default();
        hasher.write(self.subject.as_bytes());
        for line in self.hash_lines.as_ref().unwrap_or(&self.back) {
            hasher.write(line.as_bytes());
        }
        hasher.finish()
    }
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
    fn id_ignores_mode() {
        // Mode is a review property, not content — it must not change identity.
        let mut a = card("subject1", "hello", &["world"], None);
        let b = card("subject1", "hello", &["world"], None);
        a.mode = Some(Mode::Typing);
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn reversed_swaps_sides_keeps_note_and_line() {
        let mut fwd = card("vocab.txt", "purported", &["angeblich"], Some("a note"));
        fwd.mode = Some(Mode::Typing);
        let rev = fwd.reversed();
        assert_eq!("angeblich", rev.front);
        assert_eq!(vec!["purported"], rev.back);
        assert_eq!(fwd.note, rev.note);
        assert_eq!(fwd.line, rev.line); // same source line -> sibling group
        assert_eq!(fwd.mode, rev.mode);
        assert_ne!(fwd.id(), rev.id()); // distinct identity (hashes new back)
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
}
