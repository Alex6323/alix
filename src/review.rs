//! The presentation-agnostic view of a review session: the ONE review
//! contract every client renders. The embedded mobile client consumes these
//! types directly over FFI; the gated web `StateDto` is a thin wire envelope
//! derived from [`state`] (naming, phase, serve-held context), never a
//! re-derivation. The builders live here as free functions over a
//! [`Session`] plus its [`Store`] and [`AugmentCache`]; nothing in this
//! module is transport- or markup-flavored.
use serde::{Deserialize, Serialize};

use crate::{
    answer::{self, Input, Mode},
    augment::AugmentCache,
    card::Card,
    choice::{self, ChoiceQuestion},
    depth::{self, Depth},
    render::{self, NoteUnit},
    session::{self, Session},
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
    /// True when `back` is a reshaped answer (a `format` augment's
    /// `display_back`), never for the card's own authored lines.
    pub reshaped: bool,
    /// The note (`!` lines plus any augment-merged trivia) decomposed into
    /// display units, shown after the reveal.
    pub note: Vec<NoteUnit>,
    pub image: Option<String>,
    pub image_back: Option<String>,
    /// The raw authored `% at:` source locator. Resolving it to an excerpt
    /// (and any display relabeling) is the consumer's business: it takes a
    /// source base and a filesystem read.
    pub at: Option<String>,
}

impl From<&Card> for CardView {
    fn from(card: &Card) -> Self {
        CardView {
            front: card.front.clone(),
            context: card.context.clone(),
            back: card.back_for_display().to_vec(),
            reshaped: card.display_back.is_some(),
            note: render::note_units(card),
            image: card.image.as_ref().map(|p| p.display().to_string()),
            image_back: card.image_back.as_ref().map(|p| p.display().to_string()),
            at: card.at.clone(),
        }
    }
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
    /// The Explain checklist rubric: the cached keypoints augment when
    /// present, else the card's authored back lines. Only for an Explain
    /// check past acquire; `None` everywhere else.
    pub keypoints: Option<Vec<String>>,
    /// How the learner answers (`% input:`): typed (default) or drawn.
    pub input: Input,
    pub finished: bool,
    pub remaining: u32,
    /// Distinct cards that entered the roster at session start.
    pub initial: u32,
    /// Grades given this session (a re-served card counts again).
    pub reviews: u32,
    pub passed: u32,
    pub failed: u32,
    /// Whether a restart right now would find due (or new) cards, so a
    /// summary screen can offer another round.
    pub can_restart: bool,
    /// The current card is a virtual (remediation) card the client may offer
    /// to promote into the deck.
    pub promotable: bool,
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

/// Builds the state a client renders right now. `now_ms` feeds the
/// restartability check and defaults to the wall clock; tests inject it.
pub fn state(
    session: &Session,
    store: &Store,
    augment: &AugmentCache,
    now_ms: Option<u64>,
) -> ReviewState {
    let now = now_ms.unwrap_or_else(session::now_ms);
    let card = session.current();
    let depth = session.depth();
    let mode = card
        .map(|c| depth::check_for(c.reveal.unwrap_or_default(), depth, c))
        .unwrap_or_default();
    let acquire = session.current_unseen(store);
    let choices = current_question(session, store, augment).map(|q| q.options);
    // Explain reveals the key points as a tick-each-line checklist whose
    // coverage derives the grade: the cached keypoints when present, else the
    // card's AUTHORED back lines (never a reshaped display back). Any other
    // check keeps the plain reveal; never on a first encounter, where
    // acquiring just reveals the answer.
    let keypoints = if !acquire && mode == Mode::Explain {
        card.map(|c| {
            augment
                .keypoints(c.id())
                .map(<[String]>::to_vec)
                .unwrap_or_else(|| c.back.clone())
        })
    } else {
        None
    };
    ReviewState {
        card: card.map(CardView::from),
        mode,
        depth,
        acquire,
        choices,
        keypoints,
        input: card.and_then(|c| c.input).unwrap_or_default(),
        finished: session.is_finished(),
        remaining: session.remaining() as u32,
        initial: session.initial_size as u32,
        reviews: session.stats.reviews as u32,
        passed: session.stats.passed as u32,
        failed: session.stats.failed as u32,
        can_restart: session.has_due_now(store, now),
        promotable: session.current_is_virtual(store),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        answer::Mode,
        augment::AugmentCache,
        card::Card,
        depth::Depth,
        parser,
        scheduler::{Fsrs, Grade},
        session::{Session, SessionOptions},
        store::{Store, VirtualCard, VirtualKind},
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
            assert_eq!(
                state(&session, &store, &augment, Some(NOW)).mode,
                want,
                "{depth:?}"
            );
        }
    }

    #[test]
    fn acquire_flags_a_first_encounter_only() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("# q\n\ta\n");
        let fresh = session_at(cards.clone(), &store, Depth::Recall, NOW);
        assert!(
            state(&fresh, &store, &augment, Some(NOW)).acquire,
            "never-seen card"
        );

        seen(&mut store, &cards);
        let again = session_at(cards, &store, Depth::Recall, NOW);
        assert!(
            !state(&again, &store, &augment, Some(NOW)).acquire,
            "seen card"
        );
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
        let card = state(&session, &store, &augment, Some(NOW))
            .card
            .expect("a card");
        assert!(
            card.context.iter().any(|l| l.contains("____")),
            "cloze context blanks the hole: {:?}",
            card.context
        );
        assert_eq!(card.back, ["answer"], "the gap text is the answer");
        assert_eq!(
            card.note,
            [NoteUnit::Sentence {
                text: "a note line".into()
            }]
        );
        assert_eq!(card.image.as_deref(), Some("/pics/front.png"));
        assert_eq!(card.image_back.as_deref(), Some("/pics/back.png"));
    }

    #[test]
    fn card_view_structures_the_note_and_flags_a_reshape() {
        let mut cards =
            parse("# q\n\tan answer\n\t! Intro here.\n\t! ```\n\t! let x = 1;\n\t! ```\n");
        let plain = CardView::from(&cards[0]);
        assert_eq!(
            plain.note,
            [
                NoteUnit::Sentence {
                    text: "Intro here.".into()
                },
                NoteUnit::Code {
                    lines: vec!["let x = 1;".into()]
                },
            ]
        );
        assert!(!plain.reshaped, "an authored back is not a reshape");
        assert_eq!(plain.back, ["an answer"]);

        cards[0].display_back = Some(vec!["a reshaped answer".into()]);
        let reshaped = CardView::from(&cards[0]);
        assert!(reshaped.reshaped);
        assert_eq!(
            reshaped.back,
            ["a reshaped answer"],
            "back shows the reshape"
        );
    }

    #[test]
    fn card_view_carries_the_raw_at_locator() {
        let cards = parse("# q\n\t% at: src/lib.rs:10-20\n\ta\n");
        let view = CardView::from(&cards[0]);
        assert_eq!(view.at.as_deref(), Some("src/lib.rs:10-20"));
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
        assert_eq!(state(&recall, &store, &augment, Some(NOW)).choices, None);

        let recognize = session_at(cards.clone(), &store, Depth::Recognize, NOW);
        let options = state(&recognize, &store, &augment, Some(NOW))
            .choices
            .expect("a recognize pick");
        assert_eq!(options.len(), crate::choice::NUM_OPTIONS);

        // A first encounter shows a recognition pick only under the strict
        // bar: without a full set of cached AI distractors it stays a plain
        // attempt-then-reveal.
        let fresh_store = Store::open(_dir.path().join("fresh.json")).unwrap();
        let acquire = session_at(cards.clone(), &fresh_store, Depth::Recall, NOW);
        let bare = state(&acquire, &fresh_store, &augment, Some(NOW));
        assert!(bare.acquire);
        assert_eq!(bare.choices, None, "no distractors, no recognition pick");

        let id = cards[0].id();
        augment.set_distractors(
            id,
            vec!["w1".to_string(), "w2".to_string(), "w3".to_string()],
        );
        let armed = state(&acquire, &fresh_store, &augment, Some(NOW));
        assert!(armed.choices.is_some(), "full distractors arm the pick");
    }

    #[test]
    fn state_options_and_choose_agree_and_hold_still() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse(FOUR);
        seen(&mut store, &cards);
        let session = session_at(cards, &store, Depth::Recognize, NOW);

        let question = current_question(&session, &store, &augment).expect("a pick");
        let shown = state(&session, &store, &augment, Some(NOW))
            .choices
            .expect("options");
        assert_eq!(shown, question.options, "state serves the same options");
        // Stable across repeated builds while the same card is up.
        assert_eq!(
            state(&session, &store, &augment, Some(NOW)).choices,
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
    fn keypoints_appear_only_for_an_explain_check_past_acquire() {
        let (mut store, mut augment, _dir) = fixtures();
        // A seen multi-line flip card at Reconstruct renders as Explain.
        let mut cards = parse("# q\n\tfirst fact\n\tsecond fact\n");
        seen(&mut store, &cards);

        // Uncached: the rubric falls back to the AUTHORED back lines. The
        // reshaped display back must not leak into the checklist.
        cards[0].display_back = Some(vec!["a reshaped answer".into()]);
        let session = session_at(cards.clone(), &store, Depth::Reconstruct, NOW);
        let fallback = state(&session, &store, &augment, Some(NOW));
        assert_eq!(fallback.mode, Mode::Explain);
        assert_eq!(
            fallback.keypoints,
            Some(vec!["first fact".to_string(), "second fact".to_string()])
        );

        // Cached keypoints win over the fallback.
        augment.set_keypoints(cards[0].id(), vec!["one claim".to_string()]);
        let cached = state(&session, &store, &augment, Some(NOW));
        assert_eq!(cached.keypoints, Some(vec!["one claim".to_string()]));

        // Any other check keeps the plain reveal.
        let recall = session_at(cards.clone(), &store, Depth::Recall, NOW);
        assert_eq!(state(&recall, &store, &augment, Some(NOW)).keypoints, None);

        // A first encounter acquires: no checklist even at Reconstruct.
        let fresh = Store::open(_dir.path().join("fresh.json")).unwrap();
        let acquire = session_at(cards, &fresh, Depth::Reconstruct, NOW);
        let acquired = state(&acquire, &fresh, &augment, Some(NOW));
        assert!(acquired.acquire);
        assert_eq!(acquired.keypoints, None);
    }

    #[test]
    fn session_counters_mirror_the_session() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse(FOUR);
        seen(&mut store, &cards);
        let mut session = session_at(cards, &store, Depth::Recall, NOW);
        let start = state(&session, &store, &augment, Some(NOW));
        assert_eq!(start.initial, 4);
        assert_eq!((start.reviews, start.passed, start.failed), (0, 0, 0));

        session.grade(&mut store, Grade::Pass, NOW);
        session.grade(&mut store, Grade::Fail, NOW);
        let later = state(&session, &store, &augment, Some(NOW));
        assert_eq!(later.reviews, 2);
        assert_eq!(later.passed, 1);
        assert_eq!(later.failed, 1);
    }

    #[test]
    fn promotable_flags_a_virtual_card_only() {
        let (mut store, augment, _dir) = fixtures();
        let text = "# virtual front\n\tvirtual back\n";
        let mut synth = parser::parse_str("deck.txt", text).unwrap().remove(0);
        synth.line = 1_000_000;
        store.insert_virtual(VirtualCard {
            id: synth.id(),
            kind: VirtualKind::Remediation,
            parent: "deck.txt".to_string(),
            text: text.to_string(),
            created_ms: T0,
        });
        store.get_or_insert(synth.id(), T0);
        let session = session_at(vec![synth], &store, Depth::Recall, NOW);
        assert!(state(&session, &store, &augment, Some(NOW)).promotable);

        let regular = parse("# q\n\ta\n");
        seen(&mut store, &regular);
        let plain = session_at(regular, &store, Depth::Recall, NOW);
        assert!(!state(&plain, &store, &augment, Some(NOW)).promotable);
    }

    #[test]
    fn can_restart_flips_with_the_injected_clock() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("# q\n\ta\n");
        seen(&mut store, &cards);
        let mut session = session_at(cards, &store, Depth::Recall, NOW);
        session.grade(&mut store, Grade::Pass, NOW);
        assert!(session.is_finished());

        let done = state(&session, &store, &augment, Some(NOW));
        assert!(!done.can_restart, "nothing is due right after the pass");
        let much_later = NOW + 90 * 24 * 3_600_000;
        let again = state(&session, &store, &augment, Some(much_later));
        assert!(again.can_restart, "the card comes due again");
    }

    #[test]
    fn input_follows_the_card() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("# q\n\t% input: draw\n\ta\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &store, Depth::Recall, NOW);
        assert_eq!(
            state(&session, &store, &augment, Some(NOW)).input,
            Input::Draw
        );
    }

    #[test]
    fn a_finished_session_reports_no_card_and_no_choices() {
        let (store, augment, _dir) = fixtures();
        let session = session_at(Vec::new(), &store, Depth::Recall, NOW);
        let state = state(&session, &store, &augment, Some(NOW));
        assert!(state.finished);
        assert!(state.card.is_none());
        assert_eq!(state.choices, None);
        assert!(!state.acquire);
        assert_eq!(state.remaining, 0);
        assert_eq!(check_typed(&session, &["x".to_string()]), None);
        assert_eq!(choose(&session, &store, &augment, 0), None);
    }
}
