//! The flashcard model and its identity hash.

use std::{hash::Hasher, path::PathBuf, sync::Arc};

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

/// Which frontend a card can be reviewed in. Set per card (or per deck) with
/// `% frontend:`. A card carrying an image is web-only on its own (the TUI
/// can't draw images); this directive can also force it explicitly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Frontend {
    /// Reviewable in either frontend (the default).
    #[default]
    Any,
    /// Terminal only; the web frontend skips it.
    Tui,
    /// Browser only; the TUI skips it.
    Web,
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
    /// Question-side image (`% img:`). Holds the raw value as written after
    /// parsing; rewritten to an absolute path when the deck is loaded. Rendered
    /// by the web frontend only. Not part of the identity hash.
    pub image: Option<PathBuf>,
    /// Answer-side image (`% img-back:`), shown with the revealed back. Same
    /// lifecycle as `image`. Not part of the identity hash.
    pub image_back: Option<PathBuf>,
    /// Declared frontend (`% frontend:`, card override else deck), folded at
    /// load. `None` defers to `frontend()` (image cards are web-only). Not part
    /// of the identity hash.
    pub frontend: Option<Frontend>,
    /// Deck-wide top Leitner stage (`% max-stage:`), folded at load. `None`
    /// means the global `MAX_STAGE`. Reaching it retires the card (it rests
    /// until `flash reset`). A review property, not part of the identity hash.
    pub max_stage: Option<u8>,
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
            image: None,
            image_back: None,
            frontend: None,
            max_stage: None,
        }
    }

    /// Which frontend this card can be reviewed in. An explicit `% frontend:`
    /// wins; otherwise a card with any image is web-only; otherwise `Any`.
    pub fn frontend(&self) -> Frontend {
        self.frontend.unwrap_or(
            if self.image.is_some() || self.image_back.is_some() {
                Frontend::Web
            } else {
                Frontend::Any
            },
        )
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
        card.frontend = self.frontend;
        card.max_stage = self.max_stage;
        // Swap the image sides: a question-side image becomes the answer's, and
        // vice versa, so a `direction: both` visual card reverses sensibly.
        card.image = self.image_back.clone();
        card.image_back = self.image.clone();
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
    fn frontend_is_web_for_image_cards_unless_overridden() {
        let mut c = card("s", "f", &["b"], None);
        assert_eq!(Frontend::Any, c.frontend()); // no image -> any
        c.image = Some(PathBuf::from("/imgs/a.png"));
        assert_eq!(Frontend::Web, c.frontend()); // image -> web
        c.frontend = Some(Frontend::Tui); // explicit override wins
        assert_eq!(Frontend::Tui, c.frontend());
    }

    #[test]
    fn reversed_swaps_image_sides_and_keeps_frontend() {
        let mut fwd = card("g.txt", "name this chord", &["G major"], None);
        fwd.image_back = Some(PathBuf::from("/tabs/g.png"));
        fwd.frontend = Some(Frontend::Web);
        let rev = fwd.reversed();
        // The answer-side image becomes the question-side image and vice versa.
        assert_eq!(Some(PathBuf::from("/tabs/g.png")), rev.image);
        assert_eq!(None, rev.image_back);
        assert_eq!(Some(Frontend::Web), rev.frontend);
    }

    #[test]
    fn id_ignores_image_and_frontend() {
        let mut a = card("s", "f", &["b"], None);
        let b = card("s", "f", &["b"], None);
        a.image = Some(PathBuf::from("/imgs/a.png"));
        a.frontend = Some(Frontend::Web);
        a.max_stage = Some(2);
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
}
