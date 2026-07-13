//! The presentation-agnostic view of a review session: the client contract a
//! non-web frontend (the frb mobile client) renders, and the seed the gated
//! web `StateDto` converges onto later. The builders live here as free
//! functions over a [`Session`] plus its [`Store`] and [`AugmentCache`], the
//! same three the web's state builder reads; nothing in this module is
//! transport- or markup-flavored.
use serde::{Deserialize, Serialize};

use crate::{
    answer::{self, Mode},
    augment::AugmentCache,
    card::Card,
    choice::{self, ChoiceQuestion},
    depth::{self, Depth},
    session::Session,
    store::Store,
};

/// One card as a client renders it, independent of any transport. Image
/// fields carry plain paths: an embedded client reads the files directly.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardView {
    pub front: String,
    /// Cloze context: the answer lines with the hole blanked; empty for
    /// non-cloze cards.
    pub context: Vec<String>,
    pub back: Vec<String>,
    /// Note lines (`!` lines plus any augment-merged trivia), shown after the
    /// reveal.
    pub note: Vec<String>,
    pub image: Option<String>,
    pub image_back: Option<String>,
}

/// The current position in a review session: the card to show (or none when
/// finished), how it is checked, and what the client needs to run that check.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewState {
    pub card: Option<CardView>,
    /// The concrete check, derived from the session's depth and the card's
    /// authored `% reveal:` (never stored on the card).
    pub mode: Mode,
    pub depth: Depth,
    /// A first encounter: the card is acquired (shown, acknowledged), not
    /// quizzed cold or graded.
    pub acquire: bool,
    /// Multiple-choice options when the card renders as a pick (a Recognize
    /// session, or the acquire recognition on-ramp). The correct index is
    /// deliberately NOT here: it only travels in [`ChoiceFeedback`], so a
    /// state payload can never leak the answer.
    pub choices: Option<Vec<String>>,
    pub finished: bool,
    pub remaining: u32,
}

/// What a pick revealed: which option was chosen, which was right, and
/// whether they agree. The grade is still the client's separate call.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoiceFeedback {
    pub chosen: usize,
    pub correct: usize,
    pub passed: bool,
}

/// The evidence from checking typed lines: per-line results plus whether
/// every line matched. The learner-final grade stays a separate call.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckFeedback {
    pub results: Vec<answer::TypedResult>,
    pub passed: bool,
}

/// Builds the state a client renders right now.
pub fn state(session: &Session, store: &Store, augment: &AugmentCache) -> ReviewState {
    let card = session.current();
    let depth = session.depth();
    let mode = card
        .map(|c| depth::check_for(c.reveal.unwrap_or_default(), depth, c))
        .unwrap_or_default();
    let acquire = session.current_unseen(store);
    let choices = current_question(session, store, augment).map(|q| q.options);
    ReviewState {
        card: card.map(card_view),
        mode,
        depth,
        acquire,
        choices,
        finished: session.is_finished(),
        remaining: session.remaining() as u32,
    }
}

/// The multiple-choice question for the current card, or `None` when it does
/// not render as a pick. The one place the question is built, so the options
/// [`state`] serves and the correct index [`choose`] grades stay in lockstep:
/// the seed combines the card id with its appearance count this session, so
/// it holds still while the card is up and reshuffles the next time it is
/// served.
///
/// A first encounter renders a recognition pick only under the strict bar
/// (atomic answer plus a full set of cached AI distractors); a Recognize
/// session picks among sibling backs; any other depth renders no pick.
pub fn current_question(
    session: &Session,
    store: &Store,
    augment: &AugmentCache,
) -> Option<ChoiceQuestion> {
    let card = session.current()?;
    let ai = augment.distractors(card.id());
    let seed = choice::seed_for(card.id(), session.appearance(card.id()));
    if store.get(card.id()).is_none() {
        return choice::recognition_question(card, session.cards(), seed, ai);
    }
    if session.depth() != Depth::Recognize {
        return None;
    }
    choice::build(card, session.cards(), seed, ai)
}

/// Grades a pick against the same question [`state`] served. `None` when no
/// pick is up (no card, or the card does not render as a choice).
pub fn choose(
    session: &Session,
    store: &Store,
    augment: &AugmentCache,
    chosen: usize,
) -> Option<ChoiceFeedback> {
    let question = current_question(session, store, augment)?;
    Some(ChoiceFeedback {
        chosen,
        correct: question.correct,
        passed: chosen == question.correct,
    })
}

/// Checks typed lines against the current card's authored back: normalized,
/// compared exactly, no edit-distance tolerance. Pure evidence, like the
/// web's check endpoint; the learner-final grade is applied separately.
/// Orderedness derives from the mode in core: TypeLine (the line-reveal
/// reconstruct) pairs line-by-position, everything else matches each input
/// to its closest expected line.
pub fn check_typed(session: &Session, lines: &[String]) -> Option<CheckFeedback> {
    let card = session.current()?;
    let mode = depth::check_for(card.reveal.unwrap_or_default(), session.depth(), card);
    let results = if mode == Mode::TypeLine {
        answer::grade_lines_ordered(lines, &card.back)
    } else {
        answer::grade_lines_unordered(lines, &card.back)
    };
    let passed = results.iter().all(|r| r.passed);
    Some(CheckFeedback { results, passed })
}

fn card_view(card: &Card) -> CardView {
    CardView {
        front: card.front.clone(),
        context: card.context.clone(),
        back: card.back_for_display().to_vec(),
        note: card
            .note
            .as_deref()
            .map(|n| n.lines().map(str::to_string).collect())
            .unwrap_or_default(),
        image: card.image.as_ref().map(|p| p.display().to_string()),
        image_back: card.image_back.as_ref().map(|p| p.display().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        answer::Mode,
        augment::AugmentCache,
        card::Card,
        depth::Depth,
        parser,
        scheduler::Fsrs,
        session::{Session, SessionOptions},
        store::Store,
    };

    /// Cards were acquired at T0 and the session runs at NOW, past the
    /// acquire cooldown, so seen cards are servable.
    const T0: u64 = 1_000_000;
    const NOW: u64 = T0 + 61_000;

    fn parse(text: &str) -> Vec<Card> {
        parser::parse_str("deck.txt", text).unwrap()
    }

    fn fixtures() -> (Store, AugmentCache, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        let augment = AugmentCache::open(dir.path().join("a.json"));
        (store, augment, dir)
    }

    fn session_at(cards: Vec<Card>, store: &Store, depth: Depth, now: u64) -> Session {
        Session::new(
            cards,
            store,
            Box::new(Fsrs::default()),
            SessionOptions {
                depth,
                ..Default::default()
            },
            now,
        )
    }

    fn seen(store: &mut Store, cards: &[Card]) {
        for card in cards {
            store.get_or_insert(card.id(), T0);
        }
    }

    #[test]
    fn mode_follows_the_depth_and_reveal_matrix() {
        let (mut store, augment, _dir) = fixtures();
        let flip = parse("# q\n\ta\n");
        let line = parse("# q\n\t% reveal: line\n\tone\n\ttwo\n");
        seen(&mut store, &flip);
        seen(&mut store, &line);

        let cases = [
            (flip.clone(), Depth::Recall, Mode::Flip),
            (line.clone(), Depth::Recall, Mode::LineByLine),
            (flip.clone(), Depth::Reconstruct, Mode::Typing),
            (line.clone(), Depth::Reconstruct, Mode::TypeLine),
            (flip, Depth::Recognize, Mode::Choice),
        ];
        for (cards, depth, want) in cases {
            let session = session_at(cards, &store, depth, NOW);
            assert!(session.current().is_some(), "{depth:?} serves the card");
            assert_eq!(state(&session, &store, &augment).mode, want, "{depth:?}");
        }
    }

    #[test]
    fn acquire_flags_a_first_encounter_only() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("# q\n\ta\n");
        let fresh = session_at(cards.clone(), &store, Depth::Recall, NOW);
        assert!(state(&fresh, &store, &augment).acquire, "never-seen card");

        seen(&mut store, &cards);
        let again = session_at(cards, &store, Depth::Recall, NOW);
        assert!(!state(&again, &store, &augment).acquire, "seen card");
    }

    #[test]
    fn card_view_carries_context_note_and_images() {
        let (mut store, augment, _dir) = fixtures();
        let mut cards =
            parse("# q\n\t% reveal: cloze\n\tthe {{answer}} is here\n\t! a note line\n");
        cards[0].image = Some("/pics/front.png".into());
        cards[0].image_back = Some("/pics/back.png".into());
        seen(&mut store, &cards);
        let session = session_at(cards, &store, Depth::Recall, NOW);
        let card = state(&session, &store, &augment).card.expect("a card");
        assert!(
            card.context.iter().any(|l| l.contains("____")),
            "cloze context blanks the hole: {:?}",
            card.context
        );
        assert_eq!(card.back, ["answer"], "the gap text is the answer");
        assert_eq!(card.note, ["a note line"]);
        assert_eq!(card.image.as_deref(), Some("/pics/front.png"));
        assert_eq!(card.image_back.as_deref(), Some("/pics/back.png"));
    }

    /// Four seen cards with distinct answers: enough siblings for a plain
    /// Recognize pick with no AI distractors at all.
    const FOUR: &str = "# q1\n\ta1\n# q2\n\ta2\n# q3\n\ta3\n# q4\n\ta4\n";

    #[test]
    fn choices_appear_only_at_recognize_or_the_acquire_bar() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards = parse(FOUR);
        seen(&mut store, &cards);

        let recall = session_at(cards.clone(), &store, Depth::Recall, NOW);
        assert_eq!(state(&recall, &store, &augment).choices, None);

        let recognize = session_at(cards.clone(), &store, Depth::Recognize, NOW);
        let options = state(&recognize, &store, &augment)
            .choices
            .expect("a recognize pick");
        assert_eq!(options.len(), crate::choice::NUM_OPTIONS);

        // A first encounter shows a recognition pick only under the strict
        // bar: without a full set of cached AI distractors it stays a plain
        // attempt-then-reveal.
        let fresh_store = Store::open(_dir.path().join("fresh.json")).unwrap();
        let acquire = session_at(cards.clone(), &fresh_store, Depth::Recall, NOW);
        let bare = state(&acquire, &fresh_store, &augment);
        assert!(bare.acquire);
        assert_eq!(bare.choices, None, "no distractors, no recognition pick");

        let id = cards[0].id();
        augment.set_distractors(
            id,
            vec!["w1".to_string(), "w2".to_string(), "w3".to_string()],
        );
        let armed = state(&acquire, &fresh_store, &augment);
        assert!(armed.choices.is_some(), "full distractors arm the pick");
    }

    #[test]
    fn state_options_and_choose_agree_and_hold_still() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse(FOUR);
        seen(&mut store, &cards);
        let session = session_at(cards, &store, Depth::Recognize, NOW);

        let question = current_question(&session, &store, &augment).expect("a pick");
        let shown = state(&session, &store, &augment).choices.expect("options");
        assert_eq!(shown, question.options, "state serves the same options");
        // Stable across repeated builds while the same card is up.
        assert_eq!(
            state(&session, &store, &augment).choices,
            Some(question.options.clone())
        );

        let right = choose(&session, &store, &augment, question.correct).expect("feedback");
        assert!(right.passed);
        assert_eq!(right.correct, question.correct);
        let wrong_index = (question.correct + 1) % question.options.len();
        let wrong = choose(&session, &store, &augment, wrong_index).expect("feedback");
        assert!(!wrong.passed);
        assert_eq!(wrong.correct, question.correct, "feedback names the answer");
    }

    #[test]
    fn check_typed_orders_only_for_typeline() {
        let (mut store, _augment, _dir) = fixtures();
        // Reconstruct + line reveal = TypeLine: position matters.
        let line = parse("# q\n\t% reveal: line\n\tone\n\ttwo\n");
        seen(&mut store, &line);
        let typeline = session_at(line, &store, Depth::Reconstruct, NOW);
        let swapped = vec!["two".to_string(), "one".to_string()];
        let ordered = check_typed(&typeline, &swapped).expect("feedback");
        assert!(!ordered.passed, "typeline is position-sensitive");

        // Reconstruct + a multi-line flip card checks unordered.
        let multi = parse("# q\n\tone\n\ttwo\n");
        seen(&mut store, &multi);
        let unordered_session = session_at(multi, &store, Depth::Reconstruct, NOW);
        let unordered = check_typed(&unordered_session, &swapped).expect("feedback");
        assert!(unordered.passed, "any order matches the same lines");
        assert_eq!(unordered.results.len(), 2);
    }

    #[test]
    fn a_finished_session_reports_no_card_and_no_choices() {
        let (store, augment, _dir) = fixtures();
        let session = session_at(Vec::new(), &store, Depth::Recall, NOW);
        let state = state(&session, &store, &augment);
        assert!(state.finished);
        assert!(state.card.is_none());
        assert_eq!(state.choices, None);
        assert!(!state.acquire);
        assert_eq!(state.remaining, 0);
        assert_eq!(check_typed(&session, &["x".to_string()]), None);
        assert_eq!(choose(&session, &store, &augment, 0), None);
    }
}
