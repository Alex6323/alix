use serde::{Deserialize, Serialize};

use crate::{
    answer::{self, Input, Mode},
    augment::AugmentCache,
    card::Card,
    choice::{self, ChoiceQuestion},
    depth::{self, Depth},
    inline::{DisplayProjector, InlineRun},
    render::{self, NoteUnit},
    session::{self, Session},
    store::Store,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageView {
    pub src: String,
    pub alt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardView {
    pub front: String,
    #[serde(default)]
    pub front_runs: Vec<InlineRun>,
    #[serde(default)]
    pub front_units: Option<Vec<NoteUnit>>,
    pub context: Vec<String>,
    #[serde(default)]
    pub context_runs: Vec<Vec<InlineRun>>,
    pub back: Vec<String>,
    #[serde(default)]
    pub back_runs: Vec<Vec<InlineRun>>,
    pub reshaped: bool,
    pub note: Vec<NoteUnit>,
    pub images: Vec<ImageView>,
    pub images_back: Vec<ImageView>,
    pub at: Option<String>,
}

fn image_views(images: &[crate::card::CardImage]) -> Vec<ImageView> {
    images
        .iter()
        .map(|i| ImageView {
            src: i.src.display().to_string(),
            alt: i.alt.clone(),
        })
        .collect()
}

impl From<&Card> for CardView {
    fn from(card: &Card) -> Self {
        let mut projector = DisplayProjector::default();
        CardView::project(card, &mut projector)
    }
}

impl CardView {
    pub fn project(card: &Card, projector: &mut DisplayProjector) -> Self {
        let (front, front_runs) = project_block(&card.front, projector);
        let front_units = render::front_units_with(&card.front, projector);
        let context_runs = card
            .context
            .iter()
            .map(|line| projector.project_context(line))
            .collect();
        let (back, back_runs) = project_lines(card.back_for_display(), projector);
        CardView {
            front,
            front_runs,
            front_units,
            context: card.context.clone(),
            context_runs,
            back,
            back_runs,
            reshaped: card.display_back.is_some(),
            note: render::note_units_with(card, projector),
            images: image_views(&card.images),
            images_back: image_views(&card.images_back),
            at: card.at.clone(),
        }
    }
}

fn project_block(text: &str, projector: &mut DisplayProjector) -> (String, Vec<InlineRun>) {
    let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let (content_lines, line_runs) = project_lines(&lines, projector);
    let mut runs = Vec::new();
    for (index, mut projected) in line_runs.into_iter().enumerate() {
        if index > 0 {
            runs.push(InlineRun {
                text: "\n".to_string(),
                ..InlineRun::default()
            });
        }
        runs.append(&mut projected);
    }
    (content_lines.join("\n"), runs)
}

fn project_lines(
    lines: &[String],
    projector: &mut DisplayProjector,
) -> (Vec<String>, Vec<Vec<InlineRun>>) {
    let mut content = Vec::with_capacity(lines.len());
    let mut display = Vec::with_capacity(lines.len());
    let mut code_fence = None;
    for line in lines {
        let marker = fence_marker(line);
        let fence = marker.is_some_and(|marker| code_fence.is_none() || code_fence == Some(marker));
        let runs = if code_fence.is_some() || fence {
            literal_runs(line)
        } else {
            projector.project(line)
        };
        content.push(runs.iter().map(|run| run.text.as_str()).collect());
        display.push(runs);
        if fence {
            code_fence = if code_fence.is_some() { None } else { marker };
        }
    }
    (content, display)
}

fn fence_marker(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    trimmed
        .starts_with("```")
        .then_some('`')
        .or_else(|| trimmed.starts_with("~~~").then_some('~'))
}

fn literal_runs(text: &str) -> Vec<InlineRun> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![InlineRun {
            text: text.to_string(),
            ..InlineRun::default()
        }]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewState {
    pub card: Option<CardView>,
    pub mode: Mode,
    pub depth: Depth,
    pub acquire: bool,
    /// The correct index is deliberately absent here: it only travels in
    /// [`ChoiceFeedback`], so this payload can never leak the answer.
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub choice_runs: Option<Vec<Vec<InlineRun>>>,
    pub keypoints: Option<Vec<String>>,
    #[serde(default)]
    pub keypoint_runs: Option<Vec<Vec<InlineRun>>>,
    pub input: Input,
    pub finished: bool,
    pub remaining: u32,
    pub initial: u32,
    pub reviews: u32,
    pub passed: u32,
    pub failed: u32,
    // Distinguishes an acquire-only sitting: without it, a first pass over a
    // fresh deck reads as "reviewed 0".
    pub acquired: u32,
    pub can_restart: bool,
    pub promotable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoiceFeedback {
    pub chosen: usize,
    pub correct: usize,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckFeedback {
    pub results: Vec<answer::TypedResult>,
    pub passed: bool,
}

pub fn state(
    session: &Session,
    store: &Store,
    augment: &AugmentCache,
    now_ms: Option<u64>,
) -> ReviewState {
    let now = now_ms.unwrap_or_else(session::now_ms);
    let card = session.current();
    let depth = session.depth();
    let base_mode = card
        .map(|c| depth::check_for(c.reveal.unwrap_or_default(), depth, c))
        .unwrap_or_default();
    let acquire = session.current_unseen(store);
    let choices = current_question(session, store, augment).map(|q| q.options);
    // Falls back to Flip when no pick can be built (no distractors): claiming
    // a choice with nothing to choose would strand the card.
    let mode = if base_mode == Mode::Choice && choices.is_none() {
        Mode::Flip
    } else {
        base_mode
    };
    // Falls back to the card's AUTHORED back lines, never the reshaped
    // display_back, so the checklist rubric stays truthful.
    let keypoints = if !acquire && mode == Mode::Explain {
        card.map(|c| {
            c.id()
                .and_then(|id| {
                    augment
                        .keypoints(&id, c.content_fingerprint)
                        .map(<[String]>::to_vec)
                })
                .unwrap_or_else(|| c.back.clone())
        })
    } else {
        None
    };
    let mut projector = DisplayProjector::default();
    let card_view = card.map(|card| CardView::project(card, &mut projector));
    let choice_runs = choices.as_ref().map(|choices| {
        choices
            .iter()
            .map(|choice| projector.project(choice))
            .collect()
    });
    let keypoint_runs = keypoints.as_ref().map(|keypoints| {
        keypoints
            .iter()
            .map(|keypoint| projector.project(keypoint))
            .collect()
    });
    ReviewState {
        card: card_view,
        mode,
        depth,
        acquire,
        choices,
        choice_runs,
        keypoints,
        keypoint_runs,
        input: card.and_then(|c| c.input).unwrap_or_default(),
        finished: session.is_finished(),
        remaining: session.remaining() as u32,
        initial: session.initial_size as u32,
        reviews: session.stats.reviews as u32,
        passed: session.stats.passed as u32,
        failed: session.stats.failed as u32,
        acquired: session.stats.acquired as u32,
        can_restart: session.has_due_now(store, now),
        promotable: session.current_is_virtual(store),
    }
}

// The single place a question is built: `state`'s options and `choose`'s
// correct index must both come from here, or they drift out of lockstep.
pub fn current_question(
    session: &Session,
    store: &Store,
    augment: &AugmentCache,
) -> Option<ChoiceQuestion> {
    let card = session.current()?;
    let id = card.id()?;
    let seed = choice::seed_for(&id, session.appearance(&id));
    if session.depth() == Depth::Recognize {
        if !card.authored_distractors.is_empty() {
            return choice::build_authored(card, seed, &card.authored_distractors);
        }
        let ai = augment.distractors(&id, card.content_fingerprint)?;
        return choice::build(card, seed, ai);
    }
    if store.get(&id).is_none() {
        if !card.authored_distractors.is_empty() {
            return choice::build_authored(card, seed, &card.authored_distractors);
        }
        let ai = augment.distractors(&id, card.content_fingerprint);
        return choice::recognition_question(card, seed, ai);
    }
    None
}

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

pub fn check_typed(session: &Session, lines: &[String]) -> Option<CheckFeedback> {
    let card = session.current()?;
    let mode = depth::check_for(card.reveal.unwrap_or_default(), session.depth(), card);
    let expected: Vec<String> = card
        .back
        .iter()
        .map(|line| crate::inline::strip_inline(line))
        .collect();
    let results = if mode == Mode::TypeLine {
        answer::grade_lines_ordered(lines, &expected)
    } else {
        answer::grade_lines_unordered(lines, &expected)
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

    // NOW must stay past T0 + the acquire cooldown, or seen cards won't be
    // servable.
    const T0: u64 = 1_000_000;
    const NOW: u64 = T0 + crate::scheduler::DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;

    // Stamps each card with a distinct token (cloze sub-cards share their
    // card's token) so store/augment lookups below key on real ids.
    fn parse(text: &str) -> Vec<Card> {
        let mut cards = parser::parse_str("deck.md", text).unwrap();
        let mut n = 0;
        let mut last_line = 0;
        for card in &mut cards {
            if card.line != last_line {
                n += 1;
                last_line = card.line;
            }
            card.token = Some(std::sync::Arc::from(format!("tok{n}").as_str()));
        }
        cards
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
            store.get_or_insert(&card.id().unwrap(), T0);
        }
    }

    fn arm(augment: &mut AugmentCache, cards: &[Card]) {
        for card in cards {
            augment.set_distractors(
                &card.id().unwrap(),
                vec!["w1".to_string(), "w2".to_string(), "w3".to_string()],
                card.content_fingerprint,
            );
        }
    }

    #[test]
    fn mode_follows_the_depth_and_reveal_matrix() {
        let (mut store, mut augment, _dir) = fixtures();
        let flip = parse("## q\na\n");
        let line = parse("## q <!-- reveal: line -->\none\ntwo\n");
        let many = parse(FOUR);
        seen(&mut store, &flip);
        seen(&mut store, &line);
        seen(&mut store, &many);
        arm(&mut augment, &many);

        let cases = [
            (flip.clone(), Depth::Recall, Mode::Flip),
            (line.clone(), Depth::Recall, Mode::LineByLine),
            (flip.clone(), Depth::Reconstruct, Mode::Typing),
            (line.clone(), Depth::Reconstruct, Mode::TypeLine),
            (many, Depth::Recognize, Mode::Choice),
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
        let cards = parse("## q\na\n");
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
        let mut cards = parse("## q\nthe \\blank{answer} is here\n> a note line\n");
        cards[0].images = vec![crate::card::CardImage {
            src: "/pics/front.png".into(),
            alt: None,
        }];
        cards[0].images_back = vec![crate::card::CardImage {
            src: "/pics/back.png".into(),
            alt: None,
        }];
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
                text: "a note line".into(),
                runs: crate::inline::parse_inline("a note line"),
            }]
        );
        assert_eq!(
            card.images
                .iter()
                .map(|i| i.src.as_str())
                .collect::<Vec<_>>(),
            ["/pics/front.png"]
        );
        assert_eq!(
            card.images_back
                .iter()
                .map(|i| i.src.as_str())
                .collect::<Vec<_>>(),
            ["/pics/back.png"]
        );
    }

    #[test]
    fn card_view_projects_math_across_every_card_surface() {
        let mut card = Card::plain(
            std::sync::Arc::from("deck.md"),
            "Find $x^2$".to_string(),
            vec![
                "$$x^2$$".to_string(),
                "```".to_string(),
                "$x^2$".to_string(),
                "```".to_string(),
            ],
            Some("Remember $x^2$.".to_string()),
            1,
        );
        card.context = vec!["Use $____ + […]$".to_string()];

        let view = CardView::from(&card);
        assert_eq!(view.front, "Find x^2");
        assert!(view.front_runs[1].math.is_some());
        assert_eq!(view.context, ["Use $____ + […]$"]);
        let context_math = view.context_runs[0]
            .iter()
            .find(|run| run.math.is_some())
            .unwrap();
        assert_eq!(context_math.text, "____ + […]");
        assert!(context_math.math.as_ref().unwrap().svg.is_some());
        assert_eq!(view.back[0], "x^2");
        assert!(view.back_runs[0][0].math.as_ref().unwrap().display);
        assert_eq!(view.back[2], "$x^2$");
        assert!(view.back_runs[2].iter().all(|run| run.math.is_none()));
        let NoteUnit::Sentence { runs, .. } = &view.note[0] else {
            panic!("note should remain a sentence");
        };
        assert!(runs.iter().any(|run| run.math.is_some()));
    }

    #[test]
    fn one_card_view_renders_one_copy_of_a_repeated_formula() {
        let mut card = Card::plain(
            std::sync::Arc::from("deck.md"),
            "$x^2$".to_string(),
            vec!["$x^2$".to_string()],
            Some("$x^2$".to_string()),
            1,
        );
        card.context = vec!["$x^2$".to_string()];
        let before = crate::math::thread_render_count();
        let view = CardView::from(&card);
        assert_eq!(crate::math::thread_render_count() - before, 1);
        assert!(view.front_runs[0].math.is_some());
    }

    #[test]
    fn card_view_structures_the_note_and_flags_a_reshape() {
        let mut cards = parse("## q\nan answer\n> Intro here.\n> ```\n> let x = 1;\n> ```\n");
        let plain = CardView::from(&cards[0]);
        assert_eq!(
            plain.note,
            [
                NoteUnit::Sentence {
                    text: "Intro here.".into(),
                    runs: crate::inline::parse_inline("Intro here."),
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
    fn an_edited_card_ignores_its_stale_format_reshape() {
        let dir = tempfile::tempdir().unwrap();
        let mut card = parse("## q\nthe authored answer\n").remove(0);
        let id = card.id().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        cache.set_format(
            &id,
            crate::augment::Format {
                back: vec!["a stale reshaped line".into()],
                ..Default::default()
            },
            card.content_fingerprint ^ 1,
        );

        cache.apply_format(&mut card);
        let stale = CardView::from(&card);
        assert!(!stale.reshaped);
        assert_eq!(["the authored answer"], stale.back.as_slice());

        let mut fresh = card.clone();
        cache.set_format(
            &id,
            crate::augment::Format {
                back: vec!["a fresh reshaped line".into()],
                ..Default::default()
            },
            fresh.content_fingerprint,
        );
        cache.apply_format(&mut fresh);
        let fresh = CardView::from(&fresh);
        assert!(fresh.reshaped);
        assert_eq!(["a fresh reshaped line"], fresh.back.as_slice());
    }

    #[test]
    fn card_view_carries_the_raw_at_locator() {
        let cards = parse("## q\n<!-- at: src/lib.rs:10-20 -->\na\n");
        let view = CardView::from(&cards[0]);
        assert_eq!(view.at.as_deref(), Some("src/lib.rs:10-20"));
    }

    const FOUR: &str = "## q1\na1\n## q2\na2\n## q3\na3\n## q4\na4\n";

    #[test]
    fn choices_appear_only_at_recognize_or_the_acquire_bar() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards = parse(FOUR);
        seen(&mut store, &cards);
        arm(&mut augment, &cards);

        let recall = session_at(cards.clone(), &store, Depth::Recall, NOW);
        assert_eq!(state(&recall, &store, &augment, Some(NOW)).choices, None);

        let recognize = session_at(cards.clone(), &store, Depth::Recognize, NOW);
        let options = state(&recognize, &store, &augment, Some(NOW))
            .choices
            .expect("cached distractors arm the Recognize pick");
        assert_eq!(options.len(), crate::choice::NUM_OPTIONS);

        let fresh_store = Store::open(_dir.path().join("fresh.json")).unwrap();
        let empty_augment = AugmentCache::open(_dir.path().join("empty.json"));
        let acquire = session_at(cards.clone(), &fresh_store, Depth::Recall, NOW);
        let bare = state(&acquire, &fresh_store, &empty_augment, Some(NOW));
        assert!(bare.acquire);
        assert_eq!(bare.choices, None, "no distractors, no acquire pick");

        let armed = state(&acquire, &fresh_store, &augment, Some(NOW));
        assert!(armed.choices.is_some(), "full distractors arm the pick");
    }

    #[test]
    fn a_recognize_card_with_no_buildable_pick_falls_back_to_flip() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## lone q\nlone a\n");
        seen(&mut store, &cards);
        let recognize = session_at(cards, &store, Depth::Recognize, NOW);
        let s = state(&recognize, &store, &augment, Some(NOW));
        assert_eq!(s.choices, None, "no siblings, no pick");
        assert_eq!(s.mode, Mode::Flip, "a choiceless Recognize card is a flip");
    }

    #[test]
    fn authored_distractors_replace_ai_choices_at_recognize() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards = parse("## capital\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n");
        seen(&mut store, &cards);
        arm(&mut augment, &cards);
        let session = session_at(cards, &store, Depth::Recognize, NOW);
        let question = current_question(&session, &store, &augment).expect("an authored pick");
        assert_eq!(3, question.options.len());
        assert_eq!("Paris", question.options[question.correct]);
        assert!(
            question
                .options
                .iter()
                .all(|option| !option.starts_with('w'))
        );
    }

    #[test]
    fn authored_distractors_drive_the_never_seen_acquire_attempt() {
        let (store, mut augment, _dir) = fixtures();
        let cards = parse("## capital\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n");
        // AI distractors exist in the cache but must be ignored for an authored card.
        arm(&mut augment, &cards);
        // Never seen (no `seen(...)`) and depth is Recall, not Recognize: this is the
        // first-meeting acquire attempt, which must still use the authored options.
        let session = session_at(cards, &store, Depth::Recall, NOW);
        let question =
            current_question(&session, &store, &augment).expect("acquire MC from authored options");
        assert_eq!(
            3,
            question.options.len(),
            "authored options, not padded to the AI four"
        );
        assert_eq!("Paris", question.options[question.correct]);
    }

    #[test]
    fn state_options_and_choose_agree_and_hold_still() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards = parse(FOUR);
        seen(&mut store, &cards);
        arm(&mut augment, &cards);
        let session = session_at(cards, &store, Depth::Recognize, NOW);

        let question = current_question(&session, &store, &augment).expect("a pick");
        let served = state(&session, &store, &augment, Some(NOW));
        let shown = served.choices.as_ref().expect("options");
        assert_eq!(shown, &question.options, "state serves the same options");
        assert_eq!(
            served.choice_runs.as_ref().map(Vec::len),
            Some(question.options.len())
        );
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
        let line = parse("## q <!-- reveal: line -->\none\ntwo\n");
        seen(&mut store, &line);
        let typeline = session_at(line, &store, Depth::Reconstruct, NOW);
        let swapped = vec!["two".to_string(), "one".to_string()];
        let ordered = check_typed(&typeline, &swapped).expect("feedback");
        assert!(!ordered.passed, "typeline is position-sensitive");

        let multi = parse("## q\none\ntwo\n");
        seen(&mut store, &multi);
        let unordered_session = session_at(multi, &store, Depth::Reconstruct, NOW);
        let unordered = check_typed(&unordered_session, &swapped).expect("feedback");
        assert!(unordered.passed, "any order matches the same lines");
        assert_eq!(unordered.results.len(), 2);
    }

    #[test]
    fn typed_grading_accepts_plain_content_for_a_formatted_answer() {
        let (mut store, _augment, _dir) = fixtures();
        let cards = parse("## capital\n**Paris**\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &store, Depth::Reconstruct, NOW);
        let feedback = check_typed(&session, &["Paris".to_string()]).expect("feedback");
        assert!(feedback.passed);
        assert_eq!("Paris", feedback.results[0].expected);
    }

    #[test]
    fn typed_grading_accepts_latex_source_without_math_delimiters() {
        let (mut store, _augment, _dir) = fixtures();
        let cards = parse("## square\n$x^2$\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &store, Depth::Reconstruct, NOW);
        let feedback = check_typed(&session, &["x^2".to_string()]).expect("feedback");
        assert!(feedback.passed);
        assert_eq!("x^2", feedback.results[0].expected);
    }

    #[test]
    fn cloze_grading_uses_the_formatted_holes_plain_content() {
        let (mut store, _augment, _dir) = fixtures();
        let cards = parse("## capital\n\\blank{**Paris**}\n");
        assert_eq!(["**Paris**"], cards[0].back.as_slice());
        seen(&mut store, &cards);
        let session = session_at(cards, &store, Depth::Reconstruct, NOW);
        let feedback = check_typed(&session, &["Paris".to_string()]).expect("feedback");
        assert!(feedback.passed);
        assert_eq!("Paris", feedback.results[0].expected);
    }

    #[test]
    fn keypoints_appear_only_for_an_explain_check_past_acquire() {
        let (mut store, mut augment, _dir) = fixtures();
        let mut cards = parse("## q\nfirst fact\nsecond fact\n");
        seen(&mut store, &cards);

        cards[0].display_back = Some(vec!["a reshaped answer".into()]);
        let session = session_at(cards.clone(), &store, Depth::Reconstruct, NOW);
        let fallback = state(&session, &store, &augment, Some(NOW));
        assert_eq!(fallback.mode, Mode::Explain);
        assert_eq!(
            fallback.keypoints,
            Some(vec!["first fact".to_string(), "second fact".to_string()])
        );
        assert_eq!(fallback.keypoint_runs.as_ref().map(Vec::len), Some(2));

        augment.set_keypoints(
            &cards[0].id().unwrap(),
            vec!["one claim".to_string()],
            cards[0].content_fingerprint,
        );
        let cached = state(&session, &store, &augment, Some(NOW));
        assert_eq!(cached.keypoints, Some(vec!["one claim".to_string()]));
        assert_eq!(cached.keypoint_runs.as_ref().map(Vec::len), Some(1));

        let recall = session_at(cards.clone(), &store, Depth::Recall, NOW);
        assert_eq!(state(&recall, &store, &augment, Some(NOW)).keypoints, None);

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
    fn an_acquire_only_sitting_reports_its_acquired_count() {
        let (_store, augment, _dir) = fixtures();
        let cards = parse(FOUR);
        let mut fresh = Store::open(_dir.path().join("fresh.json")).unwrap();
        let mut session = session_at(cards, &fresh, Depth::Recall, NOW);
        session.acquire_current(&mut fresh, NOW);
        session.acquire_current(&mut fresh, NOW);
        let s = state(&session, &fresh, &augment, Some(NOW));
        assert_eq!(s.acquired, 2, "the summary must know new cards were met");
        assert_eq!((s.reviews, s.passed, s.failed), (0, 0, 0));
    }

    #[test]
    fn promotable_flags_a_virtual_card_only() {
        let (mut store, augment, _dir) = fixtures();
        let text = "## virtual front <!-- id: vq1 -->\nvirtual back\n";
        let mut synth = parser::parse_str("deck.md", text).unwrap().remove(0);
        synth.line = 1_000_000;
        store.insert_virtual(VirtualCard {
            id: synth.id().unwrap(),
            kind: VirtualKind::Remediation,
            parent: "deck.md".to_string(),
            text: text.to_string(),
            created_ms: T0,
        });
        store.get_or_insert(&synth.id().unwrap(), T0);
        let session = session_at(vec![synth], &store, Depth::Recall, NOW);
        assert!(state(&session, &store, &augment, Some(NOW)).promotable);

        let regular = parse("## q\na\n");
        seen(&mut store, &regular);
        let plain = session_at(regular, &store, Depth::Recall, NOW);
        assert!(!state(&plain, &store, &augment, Some(NOW)).promotable);
    }

    #[test]
    fn can_restart_flips_with_the_injected_clock() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q\na\n");
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
        let cards = parse("## q <!-- input: draw -->\na\n");
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
