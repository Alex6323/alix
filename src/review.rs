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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RegionRole {
    /// A region THIS card asks: masked until answered, then revealed.
    Asked,
    /// Another card's blank on the same media: masked, never revealed here.
    Mask,
    /// A cover: masked while its content could give an answer away; whether
    /// it reveals on answer travels in `reveal_on_answer`, never in the role.
    Cover,
}

/// One drawable region (ADR 0034). Geometry is JSON numbers in the unit the
/// author wrote.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RegionView {
    pub role: RegionRole,
    /// Whether local answer reveal unmasks this region: an asked blank and a
    /// plain card's cover do, a sibling mask and a region card's cover never
    /// do. Carried per region because the role alone cannot say (ADR 0034,
    /// cover reveal split).
    pub reveal_on_answer: bool,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// `"px"` or `"%"`, one per region by the per-media unit law.
    pub unit: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CropView {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub unit: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageView {
    pub src: String,
    pub alt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<RegionView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<CropView>,
}

/// One note as a client renders it: its badge, when a blockquote opened it,
/// and the display units of its body.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NoteView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<crate::card::Badge>,
    pub units: Vec<NoteUnit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CardView {
    pub front: String,
    #[serde(default)]
    pub front_runs: Vec<InlineRun>,
    #[serde(default)]
    pub front_units: Option<Vec<NoteUnit>>,
    /// The card's section: its `# ` heading and that section's prose, shown
    /// only on demand. Image syntax inside it stays prose.
    #[serde(default)]
    pub section_context: Vec<String>,
    #[serde(default)]
    pub section_context_runs: Vec<Vec<InlineRun>>,
    /// Fence-shaped units only, in fence order, exactly as `context_units`.
    #[serde(default)]
    pub section_context_units: Vec<NoteUnit>,
    pub context: Vec<String>,
    #[serde(default)]
    pub context_leads: bool,
    #[serde(default)]
    pub context_runs: Vec<Vec<InlineRun>>,
    /// The context's fence-shaped units only, in fence order (the nth raw
    /// fence consumes the nth unit); context prose keeps its line rendering.
    #[serde(default)]
    pub context_units: Vec<NoteUnit>,
    pub back: Vec<String>,
    #[serde(default)]
    pub back_runs: Vec<Vec<InlineRun>>,
    #[serde(default)]
    pub back_units: Vec<NoteUnit>,
    pub reshaped: bool,
    pub note: Vec<NoteView>,
    pub images: Vec<ImageView>,
    pub images_back: Vec<ImageView>,
    pub citations: Vec<String>,
}

fn image_views(
    images: &[crate::card::CardImage],
    asked: &dyn Fn(Option<&str>) -> bool,
    covers_reveal: bool,
) -> Vec<ImageView> {
    use crate::parser::region::{RegionGeometry, RegionKind};
    let unit = |percent: bool| if percent { "%" } else { "px" }.to_string();
    images
        .iter()
        .map(|image| ImageView {
            src: image.src.display().to_string(),
            alt: image.alt.clone(),
            regions: image
                .regions
                .iter()
                .filter_map(|region| {
                    let RegionGeometry::Rect {
                        x,
                        y,
                        width,
                        height,
                    } = &region.geometry
                    else {
                        return None;
                    };
                    let role = match region.kind {
                        RegionKind::Cover => RegionRole::Cover,
                        RegionKind::Blank if asked(region.stamp.as_deref()) => RegionRole::Asked,
                        RegionKind::Blank => RegionRole::Mask,
                    };
                    let reveal_on_answer = match role {
                        RegionRole::Asked => true,
                        RegionRole::Mask => false,
                        RegionRole::Cover => covers_reveal,
                    };
                    Some(RegionView {
                        role,
                        reveal_on_answer,
                        x: x.value,
                        y: y.value,
                        width: width.value,
                        height: height.value,
                        unit: unit(x.percent),
                    })
                })
                .collect(),
            crop: image.crop.as_ref().map(|crop| CropView {
                x: crop.x.value,
                y: crop.y.value,
                width: crop.width.value,
                height: crop.height.value,
                unit: unit(crop.x.percent),
            }),
        })
        .collect()
}

/// A cover reveals with the answer only on a card whose block poses no
/// sibling questions the cover could give away: neither a region card nor a
/// cloze sub-card.
pub(crate) fn covers_reveal(card: &Card) -> bool {
    card.region.is_none() && card.hole.is_none()
}

/// Which stamps the projected card asks: a single region's own stamp, a
/// group's member stamps, and none for an ordinary card.
fn asked_stamps(card: &Card) -> Vec<std::sync::Arc<str>> {
    match &card.region {
        None => Vec::new(),
        Some(crate::card::RegionSlot::Single { stamp, .. }) => stamp.iter().cloned().collect(),
        Some(crate::card::RegionSlot::Group { members, .. }) => {
            members.iter().filter_map(|m| m.stamp.clone()).collect()
        }
    }
}

impl From<&Card> for CardView {
    fn from(card: &Card) -> Self {
        let mut projector = DisplayProjector::default();
        CardView::project(card, &mut projector)
    }
}

impl CardView {
    pub fn project(card: &Card, projector: &mut DisplayProjector) -> Self {
        projector.set_definitions(card.definitions.clone());
        let (front, front_runs) = project_block(&card.front, projector);
        let front_units = render::front_units_with(&card.front, projector, &card.resolved_diagrams);
        let context_runs = card
            .context
            .iter()
            .map(|line| projector.project_context(line))
            .collect();
        let (back, back_runs) = project_lines(card.back_for_display(), projector);
        let back_units =
            render::answer_units_with(card.back_for_display(), projector, &card.resolved_diagrams);
        let section_context_runs = card
            .section_context
            .iter()
            .map(|line| projector.project_context(line))
            .collect();
        CardView {
            front,
            front_runs,
            front_units,
            section_context: card.section_context.clone(),
            section_context_runs,
            section_context_units: render::section_units(&card.section_context),
            context: card.context.clone(),
            context_leads: card.context_leads,
            context_runs,
            context_units: render::context_units_with(card),
            back,
            back_runs,
            back_units,
            reshaped: card.display_back.is_some(),
            note: render::note_views_with(card, projector),
            images: {
                let stamps = asked_stamps(card);
                let asked = move |stamp: Option<&str>| {
                    stamp.is_some_and(|s| stamps.iter().any(|a| a.as_ref() == s))
                };
                image_views(&card.images, &asked, covers_reveal(card))
            },
            images_back: {
                let stamps = asked_stamps(card);
                let asked = move |stamp: Option<&str>| {
                    stamp.is_some_and(|s| stamps.iter().any(|a| a.as_ref() == s))
                };
                image_views(&card.images_back, &asked, covers_reveal(card))
            },
            citations: card
                .citations
                .iter()
                .map(|citation| citation.locator.clone())
                .collect(),
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
    let mut code_fence: Option<(char, usize)> = None;
    for line in lines {
        let marker = fence_marker(line);
        let fence = match code_fence {
            None => marker.is_some(),
            Some((open, len)) => crate::parser::closes_fence(line.trim_start(), open, len),
        };
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

fn fence_marker(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let ch = if trimmed.starts_with("```") {
        '`'
    } else if trimmed.starts_with("~~~") {
        '~'
    } else {
        return None;
    };
    Some((ch, trimmed.chars().take_while(|c| *c == ch).count()))
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ReviewState {
    pub card: Option<CardView>,
    pub mode: Mode,
    pub depth: Depth,
    pub introducing: bool,
    /// The correct index is deliberately absent here: it only travels in
    /// [`ChoiceFeedback`], so this payload can never leak the answer.
    pub choices: Option<Vec<String>>,
    /// `Some(true)` when the served pick is select-all-that-apply
    /// (`choices-multiple`); absent on a single pick and off `choice` mode.
    #[serde(default)]
    pub choices_multiple: Option<bool>,
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
    // Distinguishes an introduction-only sitting: without it, a first pass over a
    // fresh deck reads as "reviewed 0".
    pub introduced: u32,
    pub partial: u32,
    pub can_restart: bool,
    pub next_due_ms: Option<u64>,
    // The uncapped backlog beyond this sitting, populated only at done: how many
    // due (or met-but-unrecognized) and never-met cards a chained sitting would
    // still find.
    pub due_left: u32,
    pub new_left: u32,
    // Deck-wide lifetime standing, populated only at done: how many of the
    // deck's cards have ever been met, out of how many it holds.
    pub met_total: u32,
    pub deck_total: u32,
    /// Present only on an exhausted Recognize done: what the depth filter hid,
    /// so the summary can point at Recall or at augmenting instead of at
    /// nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognize_gap: Option<session::RecognizeGap>,
    /// A failed progress save, kept until one succeeds; review continues in
    /// memory (non-fatal, mirroring the serve loop's banner semantics). The
    /// builder leaves it `None`; a stateful caller stamps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_error: Option<String>,
    /// Deck-load diagnostics (a stamped diagram that did not resolve). The
    /// builder leaves it empty; a stateful caller stamps it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoiceFeedback {
    pub chosen: usize,
    pub correct: usize,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MultiChoiceFeedback {
    pub chosen: Vec<usize>,
    pub correct: Vec<usize>,
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
    let introducing = session.current_fresh(store);
    let question = current_question(session, store, augment);
    let choices_multiple = question
        .as_ref()
        .is_some_and(|q| q.multiple)
        .then_some(true);
    let choices = question.map(|q| q.options);
    // Falls back to Flip when no pick can be built (no distractors): claiming
    // a choice with nothing to choose would strand the card.
    let mode = if base_mode == Mode::Choice && choices.is_none() {
        Mode::Flip
    } else {
        base_mode
    };
    // Falls back to the card's AUTHORED back lines, never the reshaped
    // display_back, so the checklist rubric stays truthful.
    let keypoints = if !introducing && mode == Mode::Explain {
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
    let finished = session.is_finished();
    // Only the done screen reads the backlog split, and it re-scans every card,
    // so keep it off the live path.
    let (due_left, new_left) = if finished {
        session.remaining_split(store, now)
    } else {
        (0, 0)
    };
    let (met_total, deck_total) = if finished {
        session.deck_progress(store)
    } else {
        (0, 0)
    };
    ReviewState {
        card: card_view,
        mode,
        depth,
        introducing,
        choices,
        choices_multiple,
        choice_runs,
        keypoints,
        keypoint_runs,
        // Last in the chain, after the card's own directive and the deck's:
        // the rule fills a gap and never overrules what an author wrote.
        input: card
            .and_then(|c| c.input.or(c.math_hole.then_some(Input::Draw)))
            .unwrap_or_default(),
        finished,
        remaining: session.remaining() as u32,
        initial: session.initial_size as u32,
        reviews: session.stats.reviews as u32,
        passed: session.stats.passed as u32,
        failed: session.stats.failed as u32,
        introduced: session.stats.introduced as u32,
        partial: session.stats.partial as u32,
        can_restart: session.has_due_now(store, now),
        // Both scopes, since a card met in an earlier sitting is not in this
        // roster and can open well before anything this sitting cooled.
        next_due_ms: finished
            .then(|| {
                [
                    session.next_servable_at(store, now),
                    session.next_due_at(store),
                ]
                .into_iter()
                .flatten()
                .filter(|&t| t > now)
                .min()
            })
            .flatten(),
        due_left: due_left as u32,
        new_left: new_left as u32,
        met_total: met_total as u32,
        deck_total: deck_total as u32,
        recognize_gap: finished
            .then(|| session.recognize_gap(store, now))
            .flatten(),
        save_error: None,
        load_warnings: Vec::new(),
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
    let seed = choice::seed_for(&id, session.choice_seed(), session.appearance(&id));
    if card.multiple_choice {
        // Select-all builds only from the authored option set: AI and sampled
        // distractor pools are shaped for one correct answer.
        let fresh = session.current_fresh(store);
        if session.depth() != Depth::Recognize && !fresh {
            return None;
        }
        return choice::build_authored_multi(card, seed, &card.authored_distractors);
    }
    if session.depth() == Depth::Recognize {
        if !card.authored_distractors.is_empty() {
            return choice::build_authored(card, seed, &card.authored_distractors);
        }
        if let Some(ai) = augment.distractors(&id, card.content_fingerprint)
            && let Some(question) = choice::build(card, seed, ai)
        {
            return Some(question);
        }
        return choice::build_sampled(card, seed, session.cards());
    }
    // `current_fresh`, not a bare store check: a card revealed this sitting
    // is already engaged in the store but keeps its introduction question.
    if session.current_fresh(store) {
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
    if question.multiple {
        return None;
    }
    Some(ChoiceFeedback {
        chosen,
        correct: question.correct,
        passed: chosen == question.correct,
    })
}

pub fn choose_multi(
    session: &Session,
    store: &Store,
    augment: &AugmentCache,
    chosen: &[usize],
) -> Option<MultiChoiceFeedback> {
    let question = current_question(session, store, augment)?;
    if !question.multiple {
        return None;
    }
    let mut set: Vec<usize> = chosen.to_vec();
    set.sort_unstable();
    set.dedup();
    if set.iter().any(|&index| index >= question.options.len()) {
        return None;
    }
    let passed = set == question.correct_set;
    Some(MultiChoiceFeedback {
        chosen: set,
        correct: question.correct_set,
        passed,
    })
}

pub fn check_typed(session: &Session, lines: &[String]) -> Option<CheckFeedback> {
    let card = session.current()?;
    let mode = depth::check_for(card.reveal.unwrap_or_default(), session.depth(), card);
    // A quotation is supporting content, not the answer's own prose: typing it
    // back tests transcription rather than understanding.
    let quoted = crate::render::quote_line_flags(&card.back);
    if quoted.iter().all(|line_is_quote| *line_is_quote) {
        return Some(CheckFeedback {
            results: Vec::new(),
            passed: false,
        });
    }
    let stripped: Vec<String> = card
        .back
        .iter()
        .map(|line| crate::inline::strip_inline_with(line, &card.definitions))
        .collect();
    let results = if mode == Mode::TypeLine {
        let graded: Vec<bool> = quoted.iter().map(|line_is_quote| !line_is_quote).collect();
        answer::grade_lines_ordered(lines, &stripped, &graded)
    } else {
        let expected: Vec<String> = stripped
            .iter()
            .zip(&quoted)
            .filter(|(_, line_is_quote)| !**line_is_quote)
            .map(|(line, _)| line.clone())
            .collect();
        answer::grade_lines_unordered(lines, &expected)
    };
    let passed = results.iter().all(|r| r.passed);
    Some(CheckFeedback { results, passed })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_reference_link_resolves_against_its_own_decks_definitions() {
        let deck = parser::parse(
            "d.md",
            "## Q\nsee [the ref][r] here\n[r]: https://alix.study\n",
        )
        .unwrap();
        let view = CardView::from(&deck.cards[0]);
        assert!(
            view.back_runs[0]
                .iter()
                .any(|run| run.link && run.text == "the ref"),
            "the card's own table styles its reference: {:?}",
            view.back_runs
        );
        let stray = parser::parse("other.md", "## Q\nsee [the ref][r] here\nanswer\n").unwrap();
        let stray_view = CardView::from(&stray.cards[0]);
        assert!(
            stray_view.back_runs[0].iter().all(|run| !run.link),
            "a deck without the definition keeps the form prose: {:?}",
            stray_view.back_runs
        );
    }
    use crate::{
        answer::Mode,
        augment::AugmentCache,
        card::Card,
        depth::Depth,
        parser,
        scheduler::{Fsrs, Grade},
        session::{Session, SessionOptions},
        store::Store,
    };

    // NOW must stay past T0 + the introduction cooldown, or seen cards won't be
    // servable.
    const T0: u64 = 1_000_000;
    const NOW: u64 = T0 + crate::scheduler::DEFAULT_INTRODUCTION_COOLDOWN_MS + 1_000;

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

    #[test]
    fn multiline_projection_keeps_line_breaks_in_text_and_runs() {
        let mut projector = DisplayProjector::default();
        let (text, runs) = project_block("first\nsecond", &mut projector);
        assert_eq!("first\nsecond", text);
        assert_eq!(
            "first\nsecond",
            runs.iter().map(|run| run.text.as_str()).collect::<String>()
        );
    }

    #[test]
    fn only_the_matching_marker_closes_a_projected_code_fence() {
        let lines = ["```", "$x$", "~~~", "$y$", "```", "$z$"]
            .map(str::to_string)
            .to_vec();
        let mut projector = DisplayProjector::default();
        let (_, runs) = project_lines(&lines, &mut projector);
        assert!(runs[..5].iter().flatten().all(|run| run.math.is_none()));
        assert!(runs[5].iter().any(|run| run.math.is_some()));
    }

    #[test]
    fn a_shorter_marker_does_not_close_a_longer_projected_fence() {
        let lines = ["````", "$x$", "```", "$y$", "````", "$z$"]
            .map(str::to_string)
            .to_vec();
        let mut projector = DisplayProjector::default();
        let (_, runs) = project_lines(&lines, &mut projector);
        assert!(runs[..5].iter().flatten().all(|run| run.math.is_none()));
        assert!(runs[5].iter().any(|run| run.math.is_some()));
    }

    /// Spec law 14, the projection half: a section arrives as raw lines plus
    /// inline runs, its fences collapse into one code unit each, and image
    /// syntax inside it stays prose rather than becoming a media element.
    #[test]
    fn a_section_projects_as_prose_runs_and_code_fences_never_as_media() {
        let text = "# *Ownership* and $x$\n\nProse before the card.\n\n```rust\nlet x = 1;\n```\n\n![](assets/diagram.png)\n\n## q\na\n";
        let cards = crate::parser::parse_str("t", text).expect("the fixture parses");
        let card = &cards[0];
        let view = CardView::from(card);

        assert_eq!(
            card.section_context, view.section_context,
            "the raw section lines ride the view untouched"
        );
        assert!(
            view.section_context
                .iter()
                .any(|line| line.contains("assets/diagram.png")),
            "the image line is carried as text: {:?}",
            view.section_context
        );
        assert_eq!(
            view.section_context.len(),
            view.section_context_runs.len(),
            "every section line gets its inline runs"
        );
        assert!(
            view.section_context_runs
                .iter()
                .flatten()
                .any(|run| run.math.is_some()),
            "math in a section heading projects like any other context line"
        );
        assert_eq!(
            1,
            view.section_context_units.len(),
            "the rust fence is one unit: {:?}",
            view.section_context_units
        );
        let NoteUnit::Code { lines } = &view.section_context_units[0] else {
            panic!(
                "a section fence is code, never a diagram: {:?}",
                view.section_context_units
            );
        };
        assert_eq!(vec!["let x = 1;".to_string()], *lines);
        assert!(
            !view
                .section_context_units
                .iter()
                .any(|unit| matches!(unit, NoteUnit::Diagram { .. })),
            "nothing freezes a section fence, so it can never resolve to media"
        );
        assert!(
            view.images.is_empty() && view.images_back.is_empty(),
            "a section image line adds no media element"
        );
    }

    fn session_at(cards: Vec<Card>, store: &mut Store, depth: Depth, now: u64) -> Session {
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
            store
                .get_or_insert(&card.id().unwrap())
                .introduced_ms
                .get_or_insert(T0);
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
        let line = parse("## q\none\ntwo\n<!-- reveal: line -->\n");
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
            let session = session_at(cards, &mut store, depth, NOW);
            assert!(session.current().is_some(), "{depth:?} serves the card");
            assert_eq!(
                state(&session, &store, &augment, Some(NOW)).mode,
                want,
                "{depth:?}"
            );
        }
    }

    #[test]
    fn the_next_opening_is_the_decks_earliest_not_this_sittings() {
        let cooldown = crate::scheduler::DEFAULT_INTRODUCTION_COOLDOWN_MS;
        let (mut store, augment, _dir) = fixtures();
        let both = parse("## a\n1\n\n## b\n2\n");
        let earlier = both[0].id().unwrap();
        let later = both[1].id().unwrap();
        store.get_or_insert(&earlier).introduced_ms = Some(T0);

        // A sitting one minute on: the earlier card is still cooling, so only
        // the never-met one is rostered, and it cools a minute behind it.
        let now = T0 + 60_000;
        let mut session = session_at(both, &mut store, Depth::Recall, now);
        assert_eq!(
            Some(later),
            session.current().and_then(Card::id),
            "the cooling card stays out of this sitting"
        );
        session.introduce_current(&mut store, now);
        assert!(session.is_finished());

        assert_eq!(
            Some(T0 + cooldown),
            state(&session, &store, &augment, Some(now)).next_due_ms,
            "the summary counts down to the earliest card in the deck"
        );
    }

    #[test]
    fn introducing_flags_a_first_encounter_only() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q\na\n");
        let fresh = session_at(cards.clone(), &mut store, Depth::Recall, NOW);
        assert!(
            state(&fresh, &store, &augment, Some(NOW)).introducing,
            "never-seen card"
        );

        seen(&mut store, &cards);
        let again = session_at(cards, &mut store, Depth::Recall, NOW);
        assert!(
            !state(&again, &store, &augment, Some(NOW)).introducing,
            "seen card"
        );
    }

    #[test]
    fn a_presented_but_unacknowledged_card_keeps_its_introduction_on_ramp() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q\na\n");
        let first = session_at(cards.clone(), &mut store, Depth::Recall, NOW);
        assert!(state(&first, &store, &augment, Some(NOW)).introducing);
        drop(first);

        let again = session_at(cards, &mut store, Depth::Recall, NOW + 1);
        assert!(
            state(&again, &store, &augment, Some(NOW + 1)).introducing,
            "presentation persists nothing, so the reopened card is still introducing"
        );
    }

    #[test]
    fn card_view_carries_context_note_and_images() {
        let (mut store, augment, _dir) = fixtures();
        let mut cards = parse("## q\nthe \\blank{answer} is here\n> [!NOTE]\n> a note line\n");
        cards[0].images = vec![crate::card::CardImage {
            src: "/pics/front.png".into(),
            alt: None,
            regions: Vec::new(),
            crop: None,
        }];
        cards[0].images_back = vec![crate::card::CardImage {
            src: "/pics/back.png".into(),
            alt: None,
            regions: Vec::new(),
            crop: None,
        }];
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recall, NOW);
        let card = state(&session, &store, &augment, Some(NOW))
            .card
            .expect("a card");
        assert!(
            card.context.iter().any(|l| l.contains("⍰")),
            "cloze context blanks the hole: {:?}",
            card.context
        );
        assert_eq!(card.back, ["answer"], "the gap text is the answer");
        assert_eq!(
            card.note,
            [NoteView {
                badge: Some(crate::card::Badge::Note),
                units: vec![NoteUnit::Sentence {
                    text: "a note line".into(),
                    runs: crate::inline::parse_inline("a note line"),
                }],
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

    /// The end of the path the parser starts: a hole cut from a formula
    /// reaches the client as a rendered math run, not as the characters the
    /// author typed. Without this the learner reveals `\pm` and reads source.
    #[test]
    fn a_revealed_math_hole_carries_a_rendered_run() {
        let cards = parser::parse_str("d.md", "## q\n---\n$x = -b \\blank{\\pm} \\sqrt{d}$\n")
            .expect("the deck parses");
        let view = CardView::from(&cards[0]);

        // Projection strips the delimiters into the run, so the text a client
        // shows is unchanged; what changes is that it now carries a rendering.
        assert_eq!(view.back, ["\\pm"]);
        let run = &view.back_runs[0][0];
        assert_eq!("\\pm", run.text);
        assert!(
            run.math.as_ref().is_some_and(|math| math.svg.is_some()),
            "the revealed hole must be rendered, got {:?}",
            run.math
        );
    }

    fn math_hole_answer() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("x".to_string()),
            Just("x_1".to_string()),
            Just(r"\pm".to_string()),
            Just(r"\alpha".to_string()),
            Just(r"\frac{1}{2}".to_string()),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        #[test]
        fn every_generated_formula_hole_reveals_as_math(answer in math_hole_answer()) {
            let math_deck = format!("## q\n$x + \\blank{{{answer}}} + y$\n");
            let math_cards = parser::parse_str("d.md", &math_deck).expect("the math deck parses");
            prop_assert_eq!(1, math_cards.len());
            prop_assert!(math_cards[0].math_hole, "card: {:?}", math_cards[0]);

            let math_view = CardView::from(&math_cards[0]);
            prop_assert_eq!(1, math_view.back.len());
            prop_assert_eq!(answer.as_str(), math_view.back[0].as_str());
            let rendered = math_view
                .back_runs
                .first()
                .and_then(|runs| runs.first())
                .and_then(|run| run.math.as_ref());
            prop_assert!(
                rendered.and_then(|math| math.svg.as_ref()).is_some(),
                "answer: {answer:?}; runs: {:?}",
                math_view.back_runs
            );

            let prose_deck = format!("## q\nbefore \\blank{{{answer}}} after\n");
            let prose_cards = parser::parse_str("d.md", &prose_deck).expect("the prose deck parses");
            prop_assert_eq!(1, prose_cards.len());
            prop_assert!(!prose_cards[0].math_hole, "card: {:?}", prose_cards[0]);

            let prose_view = CardView::from(&prose_cards[0]);
            prop_assert!(
                prose_view.back_runs.iter().flatten().all(|run| run.math.is_none()),
                "answer: {answer:?}; runs: {:?}",
                prose_view.back_runs
            );
        }
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
            vec![crate::card::Note::bare("Remember $x^2$.".to_string())],
            1,
        );
        card.context = vec!["Use $⍰ + ⬚$".to_string()];

        let view = CardView::from(&card);
        assert_eq!(view.front, "Find x^2");
        assert!(view.front_runs[1].math.is_some());
        assert_eq!(view.context, ["Use $⍰ + ⬚$"]);
        let context_math = view.context_runs[0]
            .iter()
            .find(|run| run.math.is_some())
            .unwrap();
        assert_eq!(context_math.text, "⍰ + ⬚");
        assert!(context_math.math.as_ref().unwrap().svg.is_some());
        assert_eq!(view.back[0], "x^2");
        assert!(view.back_runs[0][0].math.as_ref().unwrap().display);
        assert_eq!(view.back[2], "$x^2$");
        assert!(view.back_runs[2].iter().all(|run| run.math.is_none()));
        let [NoteUnit::Sentence { runs, .. }] = view.note[0].units.as_slice() else {
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
            vec![crate::card::Note::bare("$x^2$".to_string())],
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
        let mut cards =
            parse("## q\nan answer\n> [!NOTE]\n> Intro here.\n> ```\n> let x = 1;\n> ```\n");
        let plain = CardView::from(&cards[0]);
        assert_eq!(
            plain.note,
            [NoteView {
                badge: Some(crate::card::Badge::Note),
                units: vec![
                    NoteUnit::Sentence {
                        text: "Intro here.".into(),
                        runs: crate::inline::parse_inline("Intro here."),
                    },
                    NoteUnit::Code {
                        lines: vec!["let x = 1;".into()]
                    },
                ],
            }],
            "one badged blockquote is one note, and its badge rides the units"
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
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.set_format(
            &id,
            crate::augment::Format {
                back: vec!["a stale reshaped line".into()],
                ..Default::default()
            },
            card.format_fingerprint() ^ 1,
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
            fresh.format_fingerprint(),
        );
        cache.apply_format(&mut fresh);
        let fresh = CardView::from(&fresh);
        assert!(fresh.reshaped);
        assert_eq!(["a fresh reshaped line"], fresh.back.as_slice());
    }

    #[test]
    fn card_view_carries_all_raw_at_locators_in_authored_order() {
        let cards = parse(
            "## q\n\
             a\n\
             <!-- at: src/lib.rs:10-20 -->\n\
             <!-- at: src/store.rs:30-40 -->\n",
        );
        let view = CardView::from(&cards[0]);
        assert_eq!(view.citations, ["src/lib.rs:10-20", "src/store.rs:30-40"]);
    }

    const FOUR: &str = "## q1\na1\n## q2\na2\n## q3\na3\n## q4\na4\n";

    #[test]
    fn choices_appear_only_at_recognize_or_the_introduction_bar() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards = parse(FOUR);
        seen(&mut store, &cards);
        arm(&mut augment, &cards);

        let recall = session_at(cards.clone(), &mut store, Depth::Recall, NOW);
        assert_eq!(state(&recall, &store, &augment, Some(NOW)).choices, None);

        let recognize = session_at(cards.clone(), &mut store, Depth::Recognize, NOW);
        let options = state(&recognize, &store, &augment, Some(NOW))
            .choices
            .expect("cached distractors arm the Recognize pick");
        assert_eq!(options.len(), crate::choice::NUM_OPTIONS);

        let mut fresh_store = Store::open(_dir.path().join("fresh.json")).unwrap();
        let empty_augment = AugmentCache::open(_dir.path().join("empty.json"));
        let introduction = session_at(cards.clone(), &mut fresh_store, Depth::Recall, NOW);
        let bare = state(&introduction, &fresh_store, &empty_augment, Some(NOW));
        assert!(bare.introducing);
        assert_eq!(bare.choices, None, "no distractors, no introduction pick");

        let armed = state(&introduction, &fresh_store, &augment, Some(NOW));
        assert!(armed.choices.is_some(), "full distractors arm the pick");
    }

    #[test]
    fn a_recognize_card_with_no_buildable_pick_falls_back_to_flip() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## lone q\nlone a\n");
        seen(&mut store, &cards);
        let recognize = session_at(cards, &mut store, Depth::Recognize, NOW);
        let s = state(&recognize, &store, &augment, Some(NOW));
        assert_eq!(s.choices, None, "no siblings, no pick");
        assert_eq!(s.mode, Mode::Flip, "a choiceless Recognize card is a flip");
    }

    const TABLE_DECK: &str = "| w | m |\n|---|---|\n| a | alpha | <!-- r:aaaaaa -->\n| b | beta | <!-- r:bbbbbb -->\n| c | gamma | <!-- r:cccccc -->\n| d | delta | <!-- r:dddddd -->\n<!-- cards -->\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n";

    #[test]
    fn a_table_card_samples_its_column_without_any_authored_or_cached_source() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parser::parse_str("deck.md", TABLE_DECK).unwrap();
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recognize, NOW);
        let question = current_question(&session, &store, &augment).expect("a sampled pick");
        assert_eq!(4, question.options.len());
        let column = ["alpha", "beta", "gamma", "delta"];
        assert!(
            question
                .options
                .iter()
                .all(|option| column.contains(&option.as_str())),
            "every option comes from the meaning column: {:?}",
            question.options
        );
        let distinct: std::collections::HashSet<&str> =
            question.options.iter().map(String::as_str).collect();
        assert_eq!(4, distinct.len(), "four distinct column values");
    }

    #[test]
    fn a_cached_ai_source_outranks_column_sampling() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards = parser::parse_str("deck.md", TABLE_DECK).unwrap();
        seen(&mut store, &cards);
        arm(&mut augment, &cards);
        let session = session_at(cards, &mut store, Depth::Recognize, NOW);
        let question = current_question(&session, &store, &augment).expect("an AI pick");
        for (index, option) in question.options.iter().enumerate() {
            if index != question.correct {
                assert!(
                    option.starts_with('w'),
                    "a cached distractor, not a column value: {option:?}"
                );
            }
        }
    }

    #[test]
    fn an_unbuildable_ai_cache_falls_back_to_table_column_sampling() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards = parser::parse_str("deck.md", TABLE_DECK).unwrap();
        seen(&mut store, &cards);
        for card in &cards {
            augment.set_distractors(
                &card.id().unwrap(),
                vec!["only one usable distractor".into()],
                card.content_fingerprint,
            );
        }
        assert!(crate::depth::deck_recognizable(&cards, &augment));
        let session = session_at(cards, &mut store, Depth::Recognize, NOW);

        let question = current_question(&session, &store, &augment)
            .expect("the buildable table column must backstop an incomplete AI cache");

        assert_eq!(4, question.options.len());
    }

    #[test]
    fn authored_distractors_replace_ai_choices_at_recognize() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards =
            parse("## capital\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n<!-- choices-single -->\n");
        seen(&mut store, &cards);
        arm(&mut augment, &cards);
        let session = session_at(cards, &mut store, Depth::Recognize, NOW);
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
    fn a_cooled_card_coming_back_cannot_regrade_the_pick_shown_for_another() {
        use crate::scheduler::DEFAULT_INTRODUCTION_COOLDOWN_MS;
        let (mut store, mut augment, _dir) = fixtures();
        let cards = parse(
            "## The classic test pyramid, bottom to top\n\\blank{Unit} tests, \\blank{integration} tests, \\blank{end-to-end} tests.\n",
        );
        seen(&mut store, &cards);
        for (card, distractors) in cards.iter().zip([
            ["Acceptance", "Smoke", "Manual"],
            ["component", "contract", "system"],
            ["load", "regression", "exploratory"],
        ]) {
            augment.set_distractors(
                &card.id().unwrap(),
                distractors.iter().map(|d| d.to_string()).collect(),
                card.content_fingerprint,
            );
        }
        // After the introduction cooldown from `seen`'s T0, so the met cards are due.
        let now = T0 + DEFAULT_INTRODUCTION_COOLDOWN_MS + 60_000;
        let mut session = session_at(cards, &mut store, Depth::Recognize, now);

        // Miss the first hole, which floors it and moves the learner to the next.
        session.grade(&mut store, crate::scheduler::Grade::Fail, now);
        let shown = current_question(&session, &store, &augment).expect("a pick");
        let displayed = shown.options.clone();
        let picked = shown.correct;

        // The floored hole comes back off cooldown. Reading state must not swap
        // the card underneath the learner: the client still shows `displayed`
        // and gets only an index back, so a swap regrades one card's pick
        // against another card's answer key.
        let shown_card = session.current().and_then(|c| c.id());
        session.poll(&mut store, now + DEFAULT_INTRODUCTION_COOLDOWN_MS);

        // Asserting on `passed` alone is not enough: two questions can put the
        // answer at the same index, hiding the swap behind a coincidence.
        assert_eq!(
            shown_card,
            session.current().and_then(|c| c.id()),
            "the card being graded must be the card the learner was shown"
        );
        let feedback = choose(&session, &store, &augment, picked).expect("feedback");
        assert!(
            feedback.passed,
            "learner saw {displayed:?} and picked {picked}, server flagged {} correct",
            feedback.correct
        );
    }

    #[test]
    fn every_cloze_hole_marks_its_own_authored_answer_correct() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards = parse(
            "## The classic test pyramid, bottom to top\n\\blank{Unit} tests, \\blank{integration} tests, \\blank{end-to-end} tests.\n",
        );
        seen(&mut store, &cards);
        // Cached sets copied from a real deck, per hole.
        let per_hole = [
            ("Unit", ["Acceptance", "Smoke", "Manual"]),
            ("integration", ["component", "contract", "system"]),
            ("end-to-end", ["load", "regression", "exploratory"]),
        ];
        for (card, (_, distractors)) in cards.iter().zip(per_hole.iter()) {
            augment.set_distractors(
                &card.id().unwrap(),
                distractors.iter().map(|d| d.to_string()).collect(),
                card.content_fingerprint,
            );
        }
        assert_eq!(3, cards.len(), "three holes, three sub-cards");

        for (index, (answer, _)) in per_hole.iter().enumerate() {
            let session = session_at(
                vec![cards[index].clone()],
                &mut store,
                Depth::Recognize,
                NOW,
            );
            let q = current_question(&session, &store, &augment).expect("a pick");
            assert_eq!(
                *answer, q.options[q.correct],
                "hole {index}: options {:?} flagged {} correct",
                q.options, q.correct
            );
        }
    }

    #[test]
    fn the_displayed_answer_still_passes_after_the_session_is_polled() {
        let (mut store, augment, _dir) = fixtures();
        let cards =
            parse("## capital\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n<!-- choices-single -->\n");
        seen(&mut store, &cards);
        let mut session = session_at(cards, &mut store, Depth::Recognize, NOW);
        let shown = current_question(&session, &store, &augment).expect("a pick");
        // The learner clicks the option the client is displaying as the answer.
        let picked = shown.correct;

        // Reading session state must not reshuffle the question underneath the
        // learner: the client keeps the options it already rendered and only
        // receives an index back.
        session.poll(&mut store, NOW + 1);

        let feedback = choose(&session, &store, &augment, picked).expect("feedback");
        assert!(
            feedback.passed,
            "picking the displayed answer must pass; options were {:?}, server now says {} is correct",
            shown.options, feedback.correct
        );
    }

    #[test]
    fn authored_choices_vary_between_study_sessions() {
        let (mut store, augment, _dir) = fixtures();
        let cards =
            parse("## capital\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n<!-- choices-single -->\n");
        seen(&mut store, &cards);
        let first = current_question(
            &session_at(cards.clone(), &mut store, Depth::Recognize, NOW),
            &store,
            &augment,
        )
        .expect("an authored pick")
        .options;

        let later_session_varies = (1..12).any(|offset| {
            current_question(
                &session_at(cards.clone(), &mut store, Depth::Recognize, NOW + offset),
                &store,
                &augment,
            )
            .expect("an authored pick")
            .options
                != first
        });
        assert!(
            later_session_varies,
            "a fresh study session must not repeat a memorized option order"
        );
    }

    #[test]
    fn authored_distractors_drive_the_never_seen_introduction_attempt() {
        let (mut store, mut augment, _dir) = fixtures();
        let cards =
            parse("## capital\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n<!-- choices-single -->\n");
        // AI distractors exist in the cache but must be ignored for an authored card.
        arm(&mut augment, &cards);
        // Never seen (no `seen(...)`) and depth is Recall, not Recognize: this is the
        // first-meeting introduction attempt, which must still use the authored options.
        let session = session_at(cards, &mut store, Depth::Recall, NOW);
        let question = current_question(&session, &store, &augment)
            .expect("introduction MC from authored options");
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
        let session = session_at(cards, &mut store, Depth::Recognize, NOW);

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
    fn check_typed_fails_a_grouped_card_answered_only_in_part() {
        let (mut store, _augment, _dir) = fixtures();
        let cards = parse("## carpals\nThe \\blank[w]{scaphoid} and the \\blank[w]{lunate}.\n");
        assert_eq!(1, cards.len(), "one grouped card");
        assert_eq!(2, cards[0].back.len(), "asking two spans");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Reconstruct, NOW);
        let feedback = check_typed(&session, &["scaphoid".to_string()]).expect("feedback");
        assert!(
            !feedback.passed,
            "one submitted line cannot pass a two-span card"
        );
    }

    #[test]
    fn a_quotation_is_not_part_of_the_typed_target() {
        let (mut store, _augment, _dir) = fixtures();
        let cards = parse("## q\nthe answer's own prose\n> a quoted passage\n> its second line\n");
        assert_eq!(
            3,
            cards[0].back.len(),
            "the quote is answer content and stays in back: {:?}",
            cards[0].back
        );
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Reconstruct, NOW);

        let feedback =
            check_typed(&session, &["the answer's own prose".to_string()]).expect("feedback");
        assert!(
            feedback.passed,
            "typing the answer's prose passes without transcribing the quotation: {:?}",
            feedback.results
        );
        assert_eq!(
            1,
            feedback.results.len(),
            "one gradeable line, not three: {:?}",
            feedback.results
        );
    }

    #[test]
    fn a_blank_submission_never_proves_reconstruction_of_a_quote_only_answer() {
        let (mut store, _augment, _dir) = fixtures();
        let cards = parse("## q\n> the whole answer is a quotation\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Reconstruct, NOW);

        let feedback = check_typed(&session, &[String::new()]).expect("feedback");
        assert!(
            !feedback.passed,
            "a blank submission must not prove reconstruction: {:?}",
            feedback.results
        );
    }

    #[test]
    fn a_leading_quotation_does_not_shift_the_typed_fields() {
        let (mut store, _augment, _dir) = fixtures();
        let cards =
            parse("## q\n> a quoted passage\nthe answer's own prose\n<!-- reveal: line -->\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Reconstruct, NOW);

        let typed = vec![String::new(), "the answer's own prose".to_string()];
        let feedback = check_typed(&session, &typed).expect("feedback");
        assert!(
            feedback.passed,
            "the learner left the quote's field blank and typed the prose into its own field: {:?}",
            feedback.results
        );
    }

    #[test]
    fn a_quote_marker_inside_a_fence_is_still_typed_content() {
        let (mut store, _augment, _dir) = fixtures();
        let cards = parse("## q\n```text\n> not a quotation\n```\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Reconstruct, NOW);

        let feedback = check_typed(&session, &["```text".to_string()]).expect("feedback");
        assert_eq!(
            3,
            feedback.results.len(),
            "a fence's interior is source, so its `>` line is graded: {:?}",
            feedback.results
        );
    }

    #[test]
    fn check_typed_orders_only_for_typeline() {
        let (mut store, _augment, _dir) = fixtures();
        let line = parse("## q\none\ntwo\n<!-- reveal: line -->\n");
        seen(&mut store, &line);
        let typeline = session_at(line, &mut store, Depth::Reconstruct, NOW);
        let swapped = vec!["two".to_string(), "one".to_string()];
        let ordered = check_typed(&typeline, &swapped).expect("feedback");
        assert!(!ordered.passed, "typeline is position-sensitive");

        let multi = parse("## q\none\ntwo\n");
        seen(&mut store, &multi);
        let unordered_session = session_at(multi, &mut store, Depth::Reconstruct, NOW);
        let unordered = check_typed(&unordered_session, &swapped).expect("feedback");
        assert!(unordered.passed, "any order matches the same lines");
        assert_eq!(unordered.results.len(), 2);
    }

    #[test]
    fn typed_grading_accepts_plain_content_for_a_formatted_answer() {
        let (mut store, _augment, _dir) = fixtures();
        let cards = parse("## capital\n**Paris**\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Reconstruct, NOW);
        let feedback = check_typed(&session, &["Paris".to_string()]).expect("feedback");
        assert!(feedback.passed);
        assert_eq!("Paris", feedback.results[0].expected);
    }

    #[test]
    fn typed_grading_accepts_latex_source_without_math_delimiters() {
        let (mut store, _augment, _dir) = fixtures();
        let cards = parse("## square\n$x^2$\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Reconstruct, NOW);
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
        let session = session_at(cards, &mut store, Depth::Reconstruct, NOW);
        let feedback = check_typed(&session, &["Paris".to_string()]).expect("feedback");
        assert!(feedback.passed);
        assert_eq!("Paris", feedback.results[0].expected);
    }

    #[test]
    fn keypoints_appear_only_for_an_explain_check_past_introduction() {
        let (mut store, mut augment, _dir) = fixtures();
        let mut cards = parse("## q\nfirst fact\nsecond fact\n");
        seen(&mut store, &cards);

        cards[0].display_back = Some(vec!["a reshaped answer".into()]);
        let session = session_at(cards.clone(), &mut store, Depth::Reconstruct, NOW);
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

        let recall = session_at(cards.clone(), &mut store, Depth::Recall, NOW);
        assert_eq!(state(&recall, &store, &augment, Some(NOW)).keypoints, None);

        let mut fresh = Store::open(_dir.path().join("fresh.json")).unwrap();
        let introduction = session_at(cards, &mut fresh, Depth::Reconstruct, NOW);
        let introduced = state(&introduction, &fresh, &augment, Some(NOW));
        assert!(introduced.introducing);
        assert_eq!(introduced.keypoints, None);
    }

    #[test]
    fn session_counters_mirror_the_session() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse(FOUR);
        seen(&mut store, &cards);
        let mut session = session_at(cards, &mut store, Depth::Recall, NOW);
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
    fn an_introduction_only_sitting_reports_its_introduced_count() {
        let (_store, augment, _dir) = fixtures();
        let cards = parse(FOUR);
        let mut fresh = Store::open(_dir.path().join("fresh.json")).unwrap();
        let mut session = session_at(cards, &mut fresh, Depth::Recall, NOW);
        session.introduce_current(&mut fresh, NOW);
        session.introduce_current(&mut fresh, NOW);
        let s = state(&session, &fresh, &augment, Some(NOW));
        assert_eq!(s.introduced, 2, "the summary must know new cards were met");
        assert_eq!((s.reviews, s.passed, s.failed), (0, 0, 0));
    }

    #[test]
    fn an_empty_finished_session_reports_the_soonest_next_due() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q1\na1\n## q2\na2\n");
        let sooner = cards[0].id().unwrap();
        let later = cards[1].id().unwrap();
        // Both cards met and still cooling: `sooner` comes due before `later`.
        store.get_or_insert(&sooner).introduced_ms = Some(NOW);
        store.get_or_insert(&later).introduced_ms = Some(NOW + 10_000);
        let session = session_at(cards, &mut store, Depth::Recall, NOW);
        assert!(
            session.is_finished(),
            "every card is still inside its introduction cooldown"
        );
        let s = state(&session, &store, &augment, Some(NOW));
        let cooldown = crate::scheduler::DEFAULT_INTRODUCTION_COOLDOWN_MS;
        assert_eq!(
            s.next_due_ms,
            Some(NOW + cooldown),
            "the soonest due instant (the sooner card), not the latest"
        );
    }

    #[test]
    fn an_empty_finished_session_with_nothing_scheduled_has_no_next_due() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q1\na1\n");
        // No card has stored state and new intake is off, so the roster is
        // empty and there is nothing to be due.
        let session = Session::new(
            cards,
            &mut store,
            Box::new(Fsrs::default()),
            SessionOptions {
                new_cards_percent: 0,
                max_session: 0,
                ..Default::default()
            },
            NOW,
        );
        assert!(session.is_finished());
        let s = state(&session, &store, &augment, Some(NOW));
        assert_eq!(s.next_due_ms, None);
    }

    #[test]
    fn a_due_instant_equal_to_now_is_not_a_future_wakeup() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q1\na1\n");
        let id = cards[0].id().unwrap();
        store.get_or_insert(&id).introduced_ms = Some(T0);
        let now = T0 + crate::scheduler::DEFAULT_INTRODUCTION_COOLDOWN_MS;
        let session = Session::new(
            cards,
            &mut store,
            Box::new(Fsrs::default()),
            SessionOptions {
                max_session: 0,
                new_cards_percent: 0,
                ..Default::default()
            },
            now,
        );
        assert!(session.is_finished());

        assert_eq!(
            None,
            state(&session, &store, &augment, Some(now)).next_due_ms,
            "an opening at the present instant is due now, not a future countdown"
        );
    }

    #[test]
    fn an_active_session_carries_no_next_due() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse(FOUR);
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recall, NOW);
        assert!(!session.is_finished());
        let s = state(&session, &store, &augment, Some(NOW));
        assert_eq!(s.next_due_ms, None, "only the finished payload carries it");
    }

    #[test]
    fn a_sitting_that_only_introduced_reports_when_those_cards_return() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse(FOUR);
        let card_count = cards.len();
        let mut session = session_at(cards, &mut store, Depth::Recall, NOW);
        for index in 0..card_count {
            assert!(
                !session.is_finished(),
                "session finished after only {index} of {card_count} introductions"
            );
            session.introduce_current(&mut store, NOW);
        }
        assert!(
            session.is_finished(),
            "session did not finish after introducing all {card_count} cards once"
        );
        let s = state(&session, &store, &augment, Some(NOW));

        assert!(s.introduced > 0, "the sitting introduced cards");
        assert_eq!(0, s.reviews, "and graded none");
        assert_eq!(
            Some(NOW + crate::scheduler::DEFAULT_INTRODUCTION_COOLDOWN_MS),
            s.next_due_ms,
            "the client cannot say when they come back without this"
        );
    }

    #[test]
    fn can_restart_flips_with_the_injected_clock() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q\na\n");
        seen(&mut store, &cards);
        let mut session = session_at(cards, &mut store, Depth::Recall, NOW);
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
        let cards = parse("## q\na\n<!-- input: draw -->\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recall, NOW);
        assert_eq!(
            state(&session, &store, &augment, Some(NOW)).input,
            Input::Draw
        );
    }

    /// A piece of a formula cannot be typed as itself: the hole's content is
    /// the expected answer, so a formula hole would ask for LaTeX source.
    /// Drawing is what a learner does with a symbol, so the rule fills in
    /// where nothing was authored.
    #[test]
    fn a_hole_cut_from_a_formula_is_drawn_rather_than_typed() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q\n---\n$x = -b \\blank{\\pm} \\sqrt{d}$\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recall, NOW);
        assert_eq!(
            state(&session, &store, &augment, Some(NOW)).input,
            Input::Draw
        );
    }

    #[test]
    fn a_hole_cut_from_prose_is_still_typed() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q\n---\nthe value is \\blank{dropped}\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recall, NOW);
        assert_eq!(
            state(&session, &store, &augment, Some(NOW)).input,
            Input::Type
        );
    }

    /// The rule fills a gap and never overrules. A deck-level `input:` lands
    /// on the card before this point (deck.rs), so pinning it on the card
    /// covers both ways an author can say it.
    #[test]
    fn an_authored_input_beats_the_formula_rule() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse("## q\n---\n$x = \\blank{\\pm} y$\n<!-- input: type -->\n");
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recall, NOW);
        assert_eq!(
            state(&session, &store, &augment, Some(NOW)).input,
            Input::Type
        );
    }

    #[test]
    fn a_finished_session_reports_no_card_and_no_choices() {
        let (mut store, augment, _dir) = fixtures();
        let session = session_at(Vec::new(), &mut store, Depth::Recall, NOW);
        let state = state(&session, &store, &augment, Some(NOW));
        assert!(state.finished);
        assert!(state.card.is_none());
        assert_eq!(state.choices, None);
        assert!(!state.introducing);
        assert_eq!(state.remaining, 0);
        assert_eq!(check_typed(&session, &["x".to_string()]), None);
        assert_eq!(choose(&session, &store, &augment, 0), None);
    }

    #[test]
    fn a_region_cards_view_classifies_asked_sibling_and_cover_masks() {
        let text = "## bones\n![](hand.png)\n                    <!-- blank: rect x=10 y=20 width=30 height=40 hidden=\"lunate\" b:a1b2c3 -->\n                    <!-- blank: rect x=50 y=60 width=30 height=40 hidden=\"hamate\" b:d4e5f6 -->\n                    <!-- cover: rect x=1 y=2 width=3 height=4 -->\n                    <!-- crop: rect x=0 y=0 width=90 height=90 -->\n\n---\nthe carpals\n<!-- id: card-bonesbonesbonesbonesbones -->\n";
        let cards = crate::parser::parse_str("t.md", text).unwrap();
        let lunate_card = cards
            .iter()
            .find(|card| card.back == vec!["lunate".to_string()])
            .expect("the first blank produced a card");
        let view = CardView::from(lunate_card);
        let image = &view.images[0];

        let roles: Vec<RegionRole> = image.regions.iter().map(|r| r.role).collect();
        assert_eq!(
            vec![RegionRole::Asked, RegionRole::Mask, RegionRole::Cover],
            roles,
            "own blank asked, the sibling's masked, the cover covered"
        );
        let reveals: Vec<bool> = image.regions.iter().map(|r| r.reveal_on_answer).collect();
        assert_eq!(
            vec![true, false, false],
            reveals,
            "on a region card only the asked blank reveals; the cover protects siblings"
        );
        assert_eq!(10.0, image.regions[0].x);
        assert_eq!(40.0, image.regions[0].height);
        assert_eq!("px", image.regions[0].unit);
        let crop = image.crop.as_ref().expect("the crop rides the view");
        assert_eq!(90.0, crop.width);

        assert!(
            cards.iter().all(|card| card.region.is_some()),
            "a blank-bearing block is a template: no plain card exists"
        );
        let hamate_card = cards
            .iter()
            .find(|card| card.back == vec!["hamate".to_string()])
            .expect("the second blank produced a card");
        let sibling_roles: Vec<RegionRole> = CardView::from(hamate_card).images[0]
            .regions
            .iter()
            .map(|r| r.role)
            .collect();
        assert_eq!(
            vec![RegionRole::Mask, RegionRole::Asked, RegionRole::Cover],
            sibling_roles,
            "each region card asks its own blank and masks the sibling's"
        );
    }

    fn masked_fence_deck() -> Vec<crate::card::Card> {
        let text = concat!(
            "## the request path\n",
            "```mermaid\n",
            "flowchart LR\n",
            "  Cache[store] --> B[Cache]\n",
            "```\n",
            "<!-- blank: span hidden=\"store\" -->\n",
            "<!-- blank: span hidden=\"Cache\" occurrence=2 -->\n",
        );
        crate::parser::parse_str("t.md", text).unwrap()
    }

    fn masked_fence_geometry(
        extra: Vec<crate::diagram::GeometryLabel>,
    ) -> crate::card::ResolvedDiagram {
        let interior = "flowchart LR\n  Cache[store] --> B[Cache]";
        let store = interior.find("store").unwrap() as u32;
        let cache = interior.rfind("Cache").unwrap() as u32;
        let label =
            |id: &str, text: &str, start: u32, end: u32, y: u32| crate::diagram::GeometryLabel {
                id: id.into(),
                text: text.into(),
                source: crate::diagram::LabelSource::Range { start, end },
                bounds: crate::diagram::PixelBox {
                    x: 10,
                    y,
                    width: 100,
                    height: 40,
                },
            };
        let mut labels = vec![
            label("Cache", "store", store, store + 5, 10),
            label("B", "Cache", cache, cache + 5, 50),
        ];
        labels.extend(extra);
        crate::card::ResolvedDiagram {
            fingerprint: crate::diagram::fingerprint(interior),
            png: std::path::PathBuf::from("/ws/assets/deck-x/sha256-aa.png"),
            geometry: crate::diagram::DiagramGeometry {
                image: "sha256-aa.png".to_string(),
                image_width: 376,
                image_height: 228,
                logical_width: 188,
                logical_height: 114,
                labels,
            },
        }
    }

    #[test]
    fn a_bound_span_projects_regions_and_a_leak_free_label_inventory() {
        let mut cards = masked_fence_deck();
        assert_eq!(2, cards.len(), "two blanks make two region cards");
        let card = cards
            .iter_mut()
            .find(|card| card.back == ["Cache"])
            .expect("the card asking the Cache label");
        card.resolved_diagrams = vec![masked_fence_geometry(Vec::new())];
        let view = CardView::from(&*card);
        let NoteUnit::Diagram {
            regions,
            alt,
            revealed_alt,
            ..
        } = &view.context_units[0]
        else {
            panic!("the fence projects as a diagram: {:?}", view.context_units);
        };
        assert_eq!(2, regions.len(), "asked plus sibling: {regions:?}");
        let asked = regions
            .iter()
            .find(|region| region.role == RegionRole::Asked)
            .expect("the asked region");
        assert!(asked.reveal_on_answer, "the asked mask lifts on answer");
        assert_eq!(
            (50.0, 100.0),
            (asked.y, asked.width),
            "the asked box is the Cache label's raster box"
        );
        let sibling = regions
            .iter()
            .find(|region| region.role == RegionRole::Mask)
            .expect("the sibling region");
        assert!(!sibling.reveal_on_answer, "the sibling stays masked");
        assert_eq!(
            "diagram labels: …, …", alt,
            "pre-reveal accessible text names NO label"
        );
        assert!(
            !alt.contains("Cache"),
            "the id channel must not speak the answer"
        );
        assert_eq!(
            &Some("diagram labels: …, Cache".to_string()),
            revealed_alt,
            "post-reveal exposes only the asked label; the sibling stays masked"
        );
    }

    #[test]
    fn an_unbound_span_or_invalid_geometry_falls_back_to_masked_source() {
        // The span binds the stream's FIRST occurrence of `Cache`: the node
        // id, which is no label's source range.
        let text = concat!(
            "## q\n",
            "```mermaid\n",
            "flowchart LR\n",
            "  Cache[store] --> B[Cache]\n",
            "```\n",
            "<!-- blank: span hidden=\"Cache\" -->\n",
        );
        let mut cards = crate::parser::parse_str("t.md", text).unwrap();
        let card = &mut cards[0];
        card.resolved_diagrams = vec![masked_fence_geometry(Vec::new())];
        let view = CardView::from(&*card);
        assert!(
            matches!(&view.context_units[0], NoteUnit::Code { .. }),
            "an id-position span never masks a box: {:?}",
            view.context_units
        );

        // An UNRELATED label's hostile range fails the whole geometry before
        // anything slices, even though the asked span's own label is fine.
        let mut cards = masked_fence_deck();
        let card = cards
            .iter_mut()
            .find(|card| card.back == ["Cache"])
            .expect("the card asking the Cache label");
        card.resolved_diagrams = vec![masked_fence_geometry(vec![crate::diagram::GeometryLabel {
            id: "X".into(),
            text: "ghost".into(),
            source: crate::diagram::LabelSource::Range {
                start: 0,
                end: 9999,
            },
            bounds: crate::diagram::PixelBox {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        }])];
        let view = CardView::from(&*card);
        assert!(
            matches!(&view.context_units[0], NoteUnit::Code { .. }),
            "an out-of-bounds unrelated range falls back: {:?}",
            view.context_units
        );
    }

    #[test]
    fn a_plain_cards_cover_reveals_with_the_answer() {
        let text = "## marque\n![](car.png)\n<!-- cover: rect x=1 y=2 width=3 height=4 -->\n\n---\nBMW\n<!-- id: card-autosautosautosautosauto -->\n";
        let cards = crate::parser::parse_str("t.md", text).unwrap();
        let plain = &cards[0];
        assert!(
            plain.region.is_none(),
            "a cover-only block keeps its plain card"
        );
        let view = CardView::from(plain);
        let cover = &view.images[0].regions[0];
        assert_eq!(RegionRole::Cover, cover.role);
        assert!(
            cover.reveal_on_answer,
            "a plain card has no sibling questions to protect, so its cover reveals"
        );
    }

    #[test]
    fn a_cloze_cards_cover_stays_masked_to_protect_its_sibling_question() {
        let text = "## diagram\n![](parts.png)\n<!-- cover: rect x=1 y=2 width=3 height=4 -->\n\n---\nThe first is \\blank{alpha}; the second is \\blank{beta}.\n";
        let cards = crate::parser::parse_str("t.md", text).unwrap();
        assert_eq!(2, cards.len(), "the block produces sibling hole cards");

        for card in &cards {
            assert!(
                card.hole.is_some(),
                "this is a cloze card, not a plain card"
            );
            assert!(card.region.is_none(), "a text hole is not an image region");
            let cover = &CardView::from(card).images[0].regions[0];
            assert_eq!(RegionRole::Cover, cover.role);
            assert!(
                !cover.reveal_on_answer,
                "answering one hole must not uncover material that protects its sibling hole"
            );
        }
    }

    const MULTI: &str =
        "---\ntasklist: choices-multiple\n---\n## evens\n- [x] 2\n- [x] 4\n- [ ] 3\n- [ ] 5\n";

    #[test]
    fn a_multiple_card_serves_its_full_option_set_and_flags_the_wire() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse(MULTI);
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recognize, NOW);
        let s = state(&session, &store, &augment, Some(NOW));
        let options = s
            .choices
            .expect("an authored multiple card builds its pick");
        assert_eq!(
            4,
            options.len(),
            "both corrects and both distractors: {options:?}"
        );
        assert_eq!(Some(true), s.choices_multiple);
        assert_eq!(Mode::Choice, s.mode);
    }

    #[test]
    fn choose_multi_passes_only_the_exact_correct_set() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse(MULTI);
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recognize, NOW);
        let q = current_question(&session, &store, &augment).expect("multi question");
        assert!(q.multiple);
        let correct = q.correct_set.clone();
        assert_eq!(2, correct.len());

        let exact = choose_multi(&session, &store, &augment, &correct).unwrap();
        assert!(exact.passed, "exact set passes");
        assert_eq!(correct, exact.correct);

        let mut noisy: Vec<usize> = correct.iter().rev().copied().collect();
        noisy.push(correct[0]);
        let normalized = choose_multi(&session, &store, &augment, &noisy).unwrap();
        assert!(
            normalized.passed,
            "order and duplicates normalize before grading"
        );
        assert_eq!(
            correct, normalized.chosen,
            "feedback echoes the normalized set"
        );

        assert!(
            !choose_multi(&session, &store, &augment, &correct[..1])
                .unwrap()
                .passed,
            "a subset fails"
        );
        let distractor = (0..q.options.len()).find(|i| !correct.contains(i)).unwrap();
        let mut superset = correct.clone();
        superset.push(distractor);
        assert!(
            !choose_multi(&session, &store, &augment, &superset)
                .unwrap()
                .passed,
            "a superset fails"
        );
        assert!(
            choose_multi(&session, &store, &augment, &[q.options.len()]).is_none(),
            "an out-of-range index is refused, not graded"
        );
    }

    #[test]
    fn single_and_multi_picks_refuse_each_others_questions() {
        let (mut store, augment, _dir) = fixtures();
        let cards = parse(MULTI);
        seen(&mut store, &cards);
        let session = session_at(cards, &mut store, Depth::Recognize, NOW);
        assert!(
            choose(&session, &store, &augment, 0).is_none(),
            "a single pick against a select-all question is refused"
        );

        let (mut store2, mut augment2, _dir2) = fixtures();
        let single = parse(FOUR);
        seen(&mut store2, &single);
        arm(&mut augment2, &single);
        let single_session = session_at(single, &mut store2, Depth::Recognize, NOW);
        assert!(
            choose_multi(&single_session, &store2, &augment2, &[0]).is_none(),
            "a multi pick against a single question is refused"
        );
    }

    #[test]
    fn a_distractorless_multiple_card_serves_no_pick() {
        let (store, augment, _dir) = fixtures();
        let mut fresh_store = Store::open(_dir.path().join("fresh.json")).unwrap();
        let cards = parse("---\ntasklist: choices-multiple\n---\n## all\n- [x] a\n- [x] b\n");
        let session = session_at(cards, &mut fresh_store, Depth::Recall, NOW);
        let s = state(&session, &fresh_store, &augment, Some(NOW));
        assert!(s.introducing);
        assert_eq!(None, s.choices, "nothing to choose against, so no pick");
        assert_eq!(None, s.choices_multiple);
        drop(store);
    }
}
