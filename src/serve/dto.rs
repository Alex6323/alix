use std::{collections::HashSet, path::Path};

use serde::{Deserialize, Serialize};

use super::{Browsing, Examining, Reviewing, Walking, catalog::img_key};
use crate::{
    answer::{Input, Mode, mode_name},
    augment::AugmentCache,
    config::{AskConfig, Bindings, BrowseBindings, Key, KeyPattern, PickerKeys, Strictness},
    deck::{self, Deck, DeckState},
    depth::{Depth, depth_name},
    doctor, exam,
    inline::{InlineRun, parse_inline},
    render::NoteUnit,
    review::{self, CardView},
    session::now_ms,
    store::Store,
    trace::{self, Delta, Excerpt, Phase},
};

#[derive(Debug, Serialize)]
pub(super) struct CardDto {
    pub(super) front: String,
    pub(super) front_runs: Vec<InlineRun>,
    pub(super) context: Vec<String>,
    pub(super) back: Vec<String>,
    pub(super) back_runs: Vec<Vec<InlineRun>>,
    pub(super) reshaped: bool,
    pub(super) note: Vec<NoteUnit>,
    pub(super) images: Vec<ImageDto>,
    pub(super) images_back: Vec<ImageDto>,
    pub(super) at: Option<String>,
    pub(super) citation: Option<ExcerptDto>,
    pub(super) citation_error: Option<String>,
    pub(super) crumb: Option<CrumbDto>,
}

#[derive(Debug, Serialize)]
pub(super) struct ImageDto {
    pub(super) src: String,
    pub(super) alt: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct CrumbDto {
    pub(super) regions: Vec<String>,
    pub(super) current: usize,
    pub(super) cells: Vec<Vec<f32>>,
}

#[derive(Debug, Serialize, Default)]
pub(super) struct DeckDrawerDto {
    pub(super) preamble: Option<String>,
    pub(super) heatmap: Vec<f32>,
    pub(super) topologies: Vec<TopologyInfoDto>,
}

#[derive(Debug, Serialize)]
pub(super) struct TopologyInfoDto {
    pub(super) name: String,
    pub(super) principle: String,
    pub(super) regions: Vec<RegionInfoDto>,
}

#[derive(Debug, Serialize)]
pub(super) struct RegionInfoDto {
    pub(super) name: String,
    pub(super) cells: Vec<f32>,
}

#[derive(Debug, Serialize)]
pub(super) struct StateDto {
    pub(super) kind: &'static str,
    /// No separate `finished` flag: a finished session is just the `done` phase.
    pub(super) phase: &'static str,
    pub(super) card: Option<CardDto>,
    pub(super) choices: Option<Vec<String>>,
    pub(super) keypoints: Option<Vec<String>>,
    pub(super) acquire: bool,
    pub(super) mode: &'static str,
    pub(super) depth: &'static str,
    pub(super) input: &'static str,
    pub(super) remaining: u32,
    pub(super) initial: u32,
    pub(super) reviews: u32,
    pub(super) passed: u32,
    pub(super) failed: u32,
    pub(super) acquired: u32,
    pub(super) exam_due: Vec<String>,
    pub(super) can_restart: bool,
    pub(super) promotable: bool,
    pub(super) label: String,
}

#[derive(Debug, Serialize)]
pub(super) struct BrowseDto {
    pub(super) phase: &'static str,
    pub(super) label: String,
    pub(super) cards: Vec<CardDto>,
}

/// A deck inside a workspace stays out of `recent`: reachable only via its
/// workspace.
#[derive(Debug, Serialize)]
pub(super) struct DeckListDto {
    pub(super) workspaces: Vec<DeckItemDto>,
    pub(super) recent: Vec<DeckItemDto>,
    pub(super) folders: Vec<DeckItemDto>,
}

#[derive(Debug, Serialize)]
pub(super) struct DeckItemDto {
    pub(super) name: String,
    /// STRUCTURAL: whether `name` is a selectable deck row vs a
    /// workspace/folder group; unlike `reviewable*` this never changes with
    /// progress.
    pub(super) selectable: bool,
    pub(super) label: String,
    pub(super) meta: Option<String>,
    pub(super) state: &'static str,
    pub(super) locked: bool,
    pub(super) reviewable: bool,
    pub(super) reviewable_recognize: bool,
    pub(super) can_recognize: bool,
    pub(super) reviewable_recall: bool,
    pub(super) reviewable_reconstruct: bool,
    pub(super) mastered: bool,
    pub(super) is_trace: bool,
    pub(super) examable: bool,
    pub(super) has_exam: bool,
    pub(super) recent: bool,
    pub(super) is_workspace: bool,
    pub(super) description: Option<String>,
    pub(super) members: Vec<MemberDto>,
    pub(super) path: Option<String>,
    pub(super) icon: Option<String>,
    pub(super) icon_svg: bool,
    pub(super) has_topology: bool,
    pub(super) badge_depth: Option<&'static str>,
    pub(super) badge_dotted: bool,
    pub(super) new_cards: bool,
    pub(super) last_depth: &'static str,
    pub(super) deadline: Option<DeadlineDto>,
}

#[derive(Debug, Serialize)]
pub(super) struct DeadlineDto {
    pub(super) date: String, // ISO YYYY-MM-DD
    pub(super) days_left: i64,
    pub(super) ready: usize,
    pub(super) total: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct MemberDto {
    pub(super) name: String,
    pub(super) selectable: bool,
    pub(super) label: String,
    pub(super) meta: Option<String>,
    pub(super) state: &'static str,
    pub(super) locked: bool,
    pub(super) reviewable: bool,
    pub(super) reviewable_recognize: bool,
    pub(super) can_recognize: bool,
    pub(super) reviewable_recall: bool,
    pub(super) reviewable_reconstruct: bool,
    pub(super) mastered: bool,
    pub(super) is_trace: bool,
    pub(super) examable: bool,
    pub(super) has_exam: bool,
    pub(super) indent: usize,
    pub(super) tree: String,
    pub(super) has_topology: bool,
    pub(super) badge_depth: Option<&'static str>,
    pub(super) badge_dotted: bool,
    pub(super) new_cards: bool,
    pub(super) last_depth: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct ExchangeDto {
    pub(super) q: String,
    pub(super) a: String,
}

#[derive(Debug, Serialize)]
pub(super) struct AskDto {
    pub(super) transcript: Vec<ExchangeDto>,
    pub(super) thinking: bool,
    pub(super) status: Option<String>,
    pub(super) error: Option<String>,
    pub(super) draft: Option<DraftCardDto>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DraftCardDto {
    pub(super) front: String,
    pub(super) back: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateCardReq {
    pub(super) front: String,
    pub(super) back: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct CreateCardResp {
    /// The card's identity token: `token`, or a suffixed `token-N`/`token-r`,
    /// the same string the store keys it by.
    pub(super) id: String,
}

/// `model`/`effort` show `"default"` unless `[ask]` pins one; never the model
/// that built the deck.
#[derive(Debug, Serialize)]
pub(super) struct AskInfoDto {
    pub(super) backend: &'static str,
    pub(super) model: String,
    pub(super) effort: String,
}

#[derive(Serialize)]
pub(super) struct VersionDto {
    pub(super) version: &'static str,
}

#[derive(Serialize)]
pub(super) struct DoctorDto {
    pub(super) rows: Vec<DoctorRowDto>,
}

#[derive(Serialize)]
pub(super) struct DoctorRowDto {
    pub(super) name: &'static str,
    pub(super) status: &'static str,
    pub(super) detail: String,
    pub(super) remedy: Option<String>,
}

impl From<doctor::Finding> for DoctorRowDto {
    fn from(f: doctor::Finding) -> Self {
        DoctorRowDto {
            name: f.name,
            status: match f.status {
                doctor::Status::Ok => "ok",
                doctor::Status::Warn => "warn",
                doctor::Status::Fail => "fail",
            },
            detail: f.detail,
            remedy: f.remedy,
        }
    }
}

#[derive(Serialize)]
pub(super) struct PairDto {
    pub(super) url: String,
    pub(super) svg: Option<String>,
    pub(super) lan: bool,
}

#[derive(Serialize)]
pub(super) struct ResetDto {
    pub(super) deck: String,
    pub(super) cards_cleared: usize,
}

/// Unlike `generate`'s lenient save, a non-parsing upload is rejected outright.
#[derive(Serialize)]
pub(super) struct ImportDto {
    pub(super) deck: String,
    pub(super) cards: usize,
}

impl AskInfoDto {
    pub(super) fn from(cfg: &AskConfig) -> Self {
        let or_default = |s: &Option<String>| s.clone().unwrap_or_else(|| "default".to_string());
        Self {
            backend: cfg.backend.name(),
            model: or_default(&cfg.model),
            effort: or_default(&cfg.effort),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct KeyDto {
    k: String,
    ctrl: bool,
}

pub(super) fn key_dto(p: &KeyPattern) -> KeyDto {
    let k = match p.key {
        Key::Char(' ') => " ".to_string(),
        Key::Char(c) => c.to_string(),
        Key::Enter => "Enter".to_string(),
        Key::Tab => "Tab".to_string(),
        Key::Esc => "Escape".to_string(),
        Key::Backspace => "Backspace".to_string(),
    };
    KeyDto { k, ctrl: p.ctrl }
}

pub(super) fn key_list(list: &[KeyPattern]) -> Vec<KeyDto> {
    list.iter().map(key_dto).collect()
}

#[derive(Debug, Serialize)]
pub(super) struct ReviewKeys {
    reveal: Vec<KeyDto>,
    failed: Vec<KeyDto>,
    partly: Vec<KeyDto>,
    passed: Vec<KeyDto>,
    up: Vec<KeyDto>,
    down: Vec<KeyDto>,
    skip: Vec<KeyDto>,
    remove: Vec<KeyDto>,
    restart: Vec<KeyDto>,
    ask: Vec<KeyDto>,
    make_note: Vec<KeyDto>,
    make_card: Vec<KeyDto>,
}

impl ReviewKeys {
    pub(super) fn from(b: &Bindings) -> Self {
        Self {
            reveal: key_list(&b.reveal),
            failed: key_list(&b.failed),
            partly: key_list(&b.partly),
            passed: key_list(&b.passed),
            up: key_list(&b.up),
            down: key_list(&b.down),
            skip: key_list(&b.skip),
            remove: key_list(&b.remove),
            restart: key_list(&b.restart),
            ask: key_list(&b.ask),
            make_note: key_list(&b.make_note),
            make_card: key_list(&b.make_card),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct PickerKeysDto {
    up: Vec<KeyDto>,
    down: Vec<KeyDto>,
    open: Vec<KeyDto>,
    back: Vec<KeyDto>,
    filter: Vec<KeyDto>,
    mastered: Vec<KeyDto>,
    depth: Vec<KeyDto>,
    recognize: Vec<KeyDto>,
    recall: Vec<KeyDto>,
    reconstruct: Vec<KeyDto>,
    cram: Vec<KeyDto>,
}

impl PickerKeysDto {
    pub(super) fn from(k: &PickerKeys) -> Self {
        Self {
            up: key_list(&k.up),
            down: key_list(&k.down),
            open: key_list(&k.open),
            back: key_list(&k.back),
            filter: key_list(&k.filter),
            mastered: key_list(&k.mastered),
            depth: key_list(&k.depth),
            recognize: key_list(&k.recognize),
            recall: key_list(&k.recall),
            reconstruct: key_list(&k.reconstruct),
            cram: key_list(&k.cram),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct BrowseKeys {
    next: Vec<KeyDto>,
    prev: Vec<KeyDto>,
    remove: Vec<KeyDto>,
}

impl BrowseKeys {
    pub(super) fn from(b: &BrowseBindings) -> Self {
        Self {
            next: key_list(&b.next),
            prev: key_list(&b.prev),
            remove: key_list(&b.remove),
        }
    }
}

#[derive(Serialize)]
pub(super) struct ExamDto {
    pub(super) phase: &'static str,
    pub(super) deck: String,
    pub(super) strictness: &'static str,
    pub(super) total: usize,
    pub(super) current: usize,
    pub(super) question: Option<String>,
    pub(super) answer: String,
    pub(super) on_last: bool,
    pub(super) grades: Vec<ExamGradeDto>,
    pub(super) passed: Option<bool>,
    pub(super) gaps: Vec<String>,
    /// A trace deck is re-walked on fail, never remediated (fact decks only).
    pub(super) can_remediate: bool,
    pub(super) remediated_count: Option<usize>,
    pub(super) is_trace: bool,
    pub(super) unlocks: Vec<String>,
    pub(super) thinking: bool,
    pub(super) error: Option<String>,
    pub(super) elapsed: Option<u64>,
    pub(super) cooldown_ms: Option<u64>,
}

pub(super) fn cooldown_dto(deck: &str, cooldown_ms: u64) -> ExamDto {
    ExamDto {
        phase: "cooldown",
        deck: deck.to_string(),
        strictness: "balanced",
        total: 0,
        current: 0,
        question: None,
        answer: String::new(),
        on_last: false,
        grades: Vec::new(),
        passed: None,
        gaps: Vec::new(),
        can_remediate: false,
        remediated_count: None,
        is_trace: true,
        unlocks: Vec::new(),
        thinking: false,
        error: None,
        elapsed: None,
        cooldown_ms: Some(cooldown_ms),
    }
}

#[derive(Serialize)]
pub(super) struct ExamGradeDto {
    pub(super) question: String,
    pub(super) points: Vec<String>,
    pub(super) answer: String,
    pub(super) verdict: &'static str,
    pub(super) feedback: String,
    pub(super) missed: Vec<String>,
}

pub(super) fn exam_phase_name(phase: &exam::Phase) -> &'static str {
    match phase {
        exam::Phase::Generating => "generating",
        exam::Phase::Answering => "answering",
        exam::Phase::Grading => "grading",
        exam::Phase::Results => "results",
        exam::Phase::Remediating => "remediating",
        exam::Phase::Remediated => "remediated",
    }
}

pub(super) fn strictness_name(s: Strictness) -> &'static str {
    match s {
        Strictness::Strict => "strict",
        Strictness::Balanced => "balanced",
        Strictness::Lenient => "lenient",
    }
}

pub(super) fn exam_dto(ex: &Examining, decks_dir: &Path) -> ExamDto {
    let s = &ex.sitting;
    let result = s.result();
    let grades = result
        .map(|r| {
            s.questions()
                .iter()
                .zip(s.answers())
                .zip(&r.grades)
                .map(|((q, a), g)| ExamGradeDto {
                    question: q.prompt.clone(),
                    points: q.points.clone(),
                    answer: a.clone(),
                    verdict: g.verdict.label(),
                    feedback: g.feedback.clone(),
                    missed: g.missed.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let passed = result.map(|r| r.passed);
    let unlocks = if passed == Some(true) {
        deck::dependents(&ex.deck_path, decks_dir)
    } else {
        Vec::new()
    };
    ExamDto {
        phase: exam_phase_name(s.phase()),
        deck: s.subject().to_string(),
        strictness: strictness_name(s.strictness()),
        total: s.total(),
        current: s.current_index(),
        question: s.question().map(|q| q.prompt.clone()),
        answer: s.answer().to_string(),
        on_last: s.on_last(),
        grades,
        passed,
        gaps: s.gaps(),
        can_remediate: s.can_remediate(),
        remediated_count: s.remediated_count(),
        is_trace: s.kind() == exam::SittingKind::Trace,
        unlocks,
        thinking: s.thinking(),
        error: s.error().map(str::to_string),
        elapsed: s.elapsed_secs(),
        cooldown_ms: None,
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoteCard {
    pub(super) subject: String,
    pub(super) front: String,
    pub(super) back: Vec<String>,
    pub(super) at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoteTurn {
    pub(super) q: String,
    pub(super) a: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoteAskReq {
    pub(super) card: RemoteCard,
    pub(super) history: Vec<RemoteTurn>,
    pub(super) question: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoteDraftReq {
    pub(super) card: RemoteCard,
    pub(super) history: Vec<RemoteTurn>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoteNoteReq {
    pub(super) card: RemoteCard,
    pub(super) history: Vec<RemoteTurn>,
}

#[derive(Debug, Serialize)]
pub(super) struct RemoteAskDto {
    pub(super) thinking: bool,
    pub(super) answer: Option<String>,
    pub(super) draft: Option<DraftCardDto>,
    /// An empty vec is a valid settled result ("nothing to save"), not an error.
    pub(super) note: Option<Vec<String>>,
    pub(super) error: Option<String>,
    pub(super) elapsed: Option<u64>,
}

#[derive(Serialize)]
pub(super) struct RemoteExamDto {
    pub(super) phase: &'static str,
    pub(super) deck: String,
    pub(super) strictness: &'static str,
    /// Prompts only: the rubric never leaves the server.
    pub(super) questions: Vec<String>,
    pub(super) passed: Option<bool>,
    pub(super) grades: Vec<ExamGradeDto>,
    pub(super) gaps: Vec<String>,
    pub(super) can_remediate: bool,
    pub(super) cards: Option<String>,
    pub(super) is_trace: bool,
    pub(super) thinking: bool,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

#[derive(Serialize)]
pub(super) struct RemoteGenerateDto {
    pub(super) phase: &'static str,
    pub(super) deck: Option<String>,
    pub(super) filename: Option<String>,
    /// Unlike `GenerateDto`, a parse failure here doesn't flip `phase` to
    /// `error`: nothing is saved either way.
    pub(super) cards: Option<usize>,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

#[derive(Serialize)]
pub(super) struct AugmentDto {
    pub(super) deck: String,
    pub(super) cards: usize,
    pub(super) rows: Vec<AugmentRowDto>,
    pub(super) busy: Option<&'static str>,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
    pub(super) queued: Vec<&'static str>,
    pub(super) done: Vec<&'static str>,
    /// Partial-failure safe: one target's error doesn't stop the rest.
    pub(super) failed: Vec<FailedTargetDto>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FailedTargetDto {
    pub(super) target: &'static str,
    pub(super) error: String,
}

#[derive(Serialize)]
pub(super) struct AugmentRowDto {
    pub(super) kind: &'static str,
    pub(super) label: &'static str,
    pub(super) covered: usize,
    pub(super) eligible: usize,
    pub(super) items: Vec<String>,
    pub(super) busy: bool,
}

#[derive(Serialize)]
pub(super) struct GenerateDto {
    pub(super) phase: &'static str,
    pub(super) deck: Option<String>,
    pub(super) cards: Option<usize>,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

#[derive(Serialize)]
pub(super) struct ShareDto {
    pub(super) phase: &'static str,
    pub(super) code: Option<String>,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

#[derive(Serialize)]
pub(super) struct ReceiveDto {
    pub(super) phase: &'static str,
    pub(super) landed: Option<String>,
    pub(super) stripped: Vec<String>,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

#[derive(Serialize)]
pub(super) struct HopDto {
    pub(super) prompt: String,
    pub(super) delta: Option<&'static str>,
    pub(super) current: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct ExcerptDto {
    pub(super) path: String,
    pub(super) lines: Vec<LineDto>,
    pub(super) truncated: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct LineDto {
    pub(super) n: usize,
    pub(super) text: String,
}

#[derive(Serialize)]
pub(super) struct SummaryDto {
    pub(super) passed: usize,
    pub(super) partly: usize,
    pub(super) failed: usize,
    pub(super) weak: Vec<usize>,
    pub(super) total: usize,
}

#[derive(Serialize)]
pub(super) struct WalkDto {
    pub(super) kind: &'static str,
    pub(super) phase: &'static str,
    pub(super) description: String,
    pub(super) source: Option<String>,
    pub(super) total: usize,
    pub(super) current: usize,
    pub(super) path: Vec<HopDto>,
    pub(super) prompt: Option<String>,
    pub(super) givens: Vec<String>,
    pub(super) locator: Option<String>,
    pub(super) prediction: Option<String>,
    pub(super) excerpt: Option<ExcerptDto>,
    pub(super) excerpt_error: Option<String>,
    pub(super) points: Vec<String>,
    pub(super) note: Option<String>,
    pub(super) auto_grade: bool,
    pub(super) thinking: bool,
    pub(super) verdict: Option<&'static str>,
    pub(super) feedback: Option<String>,
    pub(super) grade_error: Option<String>,
    pub(super) summary: Option<SummaryDto>,
}

pub(super) fn walk_phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Predict => "predict",
        Phase::Reveal => "reveal",
        Phase::Done => "done",
    }
}

pub(super) fn delta_name(delta: Delta) -> &'static str {
    match delta {
        Delta::Passed => "passed",
        Delta::Partial => "partly",
        Delta::Failed => "failed",
    }
}

pub(super) fn excerpt_dto(excerpt: &Excerpt) -> ExcerptDto {
    ExcerptDto {
        path: excerpt.path.display().to_string(),
        lines: excerpt
            .lines
            .iter()
            .map(|(n, text)| LineDto {
                n: *n,
                text: text.clone(),
            })
            .collect(),
        truncated: excerpt.truncated,
    }
}

pub(super) fn walk_dto(w: &Walking) -> WalkDto {
    let walk = &w.walk;
    let trace = walk.trace();
    let phase = walk.phase();
    let on_a_hop = matches!(phase, Phase::Predict | Phase::Reveal);

    let path = trace
        .checkpoints
        .iter()
        .enumerate()
        .map(|(i, c)| HopDto {
            prompt: c.prompt.clone(),
            delta: walk.delta(i).map(delta_name),
            current: on_a_hop && i == walk.current_index(),
        })
        .collect();

    let mut dto = WalkDto {
        kind: "walk",
        phase: walk_phase_name(phase),
        description: trace.description.clone(),
        source: trace.source.clone(),
        total: walk.total(),
        current: walk.current_index() + 1,
        path,
        prompt: None,
        givens: Vec::new(),
        locator: None,
        prediction: None,
        excerpt: None,
        excerpt_error: None,
        points: Vec::new(),
        note: None,
        auto_grade: w.grade.is_some(),
        thinking: w.pending.is_some(),
        verdict: w.grade_result.as_ref().map(|(d, _)| delta_name(*d)),
        feedback: w.grade_result.as_ref().map(|(_, f)| f.clone()),
        grade_error: w.grade_error.clone(),
        summary: None,
    };

    match phase {
        Phase::Predict => {
            if let Some(c) = walk.checkpoint() {
                dto.prompt = Some(c.prompt.clone());
                dto.givens = c.givens.clone();
                dto.locator = c.locator.clone();
            }
        }
        Phase::Reveal => {
            if let Some(c) = walk.checkpoint() {
                dto.prompt = Some(c.prompt.clone());
                dto.givens = c.givens.clone();
                dto.locator = c.locator.clone();
                dto.points = c.points.clone();
                dto.note = c.note.clone();
                match trace.excerpt(c) {
                    Ok(ex) => {
                        // For a frozen-snapshot asset, relabel to the ORIGINAL
                        // source so the gutter shows real line numbers, not the asset's.
                        let (ex, label) = trace::relabel_for_display(ex, c.at_origin.as_deref());
                        if let Some(label) = label {
                            dto.locator = Some(label);
                        }
                        dto.excerpt = Some(excerpt_dto(&ex));
                    }
                    Err(e) => dto.excerpt_error = Some(format!("{e:#}")),
                }
            }
            dto.prediction = walk
                .prediction(walk.current_index())
                .map(str::to_string)
                .filter(|p| !p.is_empty());
        }
        Phase::Done => {
            let s = walk.summary();
            dto.summary = Some(SummaryDto {
                passed: s.passed,
                partly: s.partly,
                failed: s.failed,
                weak: s.weak.iter().map(|i| i + 1).collect(),
                total: walk.total(),
            });
        }
    }
    dto
}

pub(super) fn browse_payload(browsing: Option<&Browsing>) -> BrowseDto {
    match browsing {
        Some(b) => BrowseDto {
            phase: "browse",
            label: b.label.clone(),
            cards: b.cards.iter().map(|c| card_dto(c.into())).collect(),
        },
        None => BrowseDto {
            phase: "select",
            label: "select decks".to_string(),
            cards: Vec::new(),
        },
    }
}

/// A choice card's options are seeded by the card id plus its appearance
/// count, so they're stable across `/api/state` and `/api/choose` yet
/// reshuffle next time the card is served.
pub(super) fn review_state(reviewing: Option<&Reviewing>, store: &Store) -> StateDto {
    let Some(r) = reviewing else {
        return StateDto {
            kind: "review",
            phase: "select",
            card: None,
            choices: None,
            keypoints: None,
            acquire: false,
            mode: mode_name(Mode::default()),
            depth: depth_name(Depth::default()),
            input: input_name(Input::default()),
            remaining: 0,
            initial: 0,
            reviews: 0,
            passed: 0,
            failed: 0,
            acquired: 0,
            exam_due: Vec::new(),
            can_restart: false,
            promotable: false,
            label: "select decks".to_string(),
        };
    };
    let session = &r.session;
    // Every fact here comes from the shared `crate::review` contract, the same
    // state embedded mobile renders; this envelope only adds wire naming and
    // serve-held context.
    let s = review::state(session, store, &r.augment, None);
    // Only computed when finished: it reloads decks, so this stays off the hot path.
    let exam_due = if s.finished {
        let mut due: Vec<String> = r
            .files
            .paths
            .iter()
            .filter_map(|(subject, path)| {
                Deck::load(path)
                    .ok()
                    .filter(|d| d.state(store) == DeckState::ExamDue)
                    .map(|_| subject.clone())
            })
            .collect();
        due.sort();
        due
    } else {
        Vec::new()
    };
    let card_with_citation = s.card.zip(session.current()).map(|(view, c)| {
        let mut dto = card_dto(view);
        // A cache can hold several like-named topologies (decks sharing a
        // store); the card id disambiguates which one actually applies.
        if let Some(name) = &r.topology_name
            && let Some((topo, regions, current)) = r
                .augment
                .topologies()
                .iter()
                .filter(|t| t.name == *name)
                .find_map(|t| {
                    c.id()
                        .as_deref()
                        .and_then(|id| t.region_path(id))
                        .map(|(rg, cur)| (t, rg, cur))
                })
        {
            dto.crumb = Some(CrumbDto {
                regions: regions.into_iter().map(str::to_string).collect(),
                current,
                cells: topo
                    .regions
                    .iter()
                    .map(|reg| crate::session::card_strengths(&reg.cards, store, now_ms()))
                    .collect(),
            });
        }
        if let Some(locator) = c.at.as_deref() {
            // `dto.at` already carries the raw locator via the core view; a
            // resolved excerpt may relabel it to its display form below.
            if let Some(base) = r.source_bases.get(&*c.subject) {
                match base.excerpt(locator) {
                    Ok(ex) => {
                        // Relabel a frozen-snapshot asset to its real source
                        // and line numbers, so the citation reads
                        // `store.rs:36-66`, not the asset's own numbering.
                        let (ex, label) = trace::relabel_for_display(ex, c.at_origin.as_deref());
                        if let Some(label) = label {
                            dto.at = Some(label);
                        }
                        dto.citation = Some(excerpt_dto(&ex));
                    }
                    Err(e) => dto.citation_error = Some(format!("{e:#}")),
                }
            }
        }
        dto
    });
    StateDto {
        kind: "review",
        phase: if s.finished { "done" } else { "review" },
        card: card_with_citation,
        choices: s.choices,
        keypoints: s.keypoints,
        acquire: s.acquire,
        mode: mode_name(s.mode),
        depth: depth_name(s.depth),
        input: input_name(s.input),
        remaining: s.remaining,
        initial: s.initial,
        reviews: s.reviews,
        passed: s.passed,
        failed: s.failed,
        acquired: s.acquired,
        exam_due,
        can_restart: s.can_restart,
        promotable: s.promotable,
        label: r.label.clone(),
    }
}

pub(super) fn state_name(s: DeckState) -> &'static str {
    match s {
        DeckState::NotStarted => "new",
        DeckState::Started => "started",
        DeckState::Finished => "finished",
        DeckState::ExamDue => "examdue",
    }
}

pub(super) fn deck_drawer_dto(
    augment: &AugmentCache,
    store: &Store,
    deck: &Deck,
) -> DeckDrawerDto {
    let deck_tokens: HashSet<String> = deck.deck_token.iter().cloned().collect();
    let now = now_ms();
    // A flat per-card heatmap over the whole deck, in file order; a topology (if
    // any) re-groups the same signal into named regions below. Retrievability is
    // pinned to Recall: a deck-wide signal, not per-session.
    let ids: Vec<String> = deck.cards.iter().filter_map(|c| c.id()).collect();
    let heatmap = crate::session::card_strengths(&ids, store, now);
    let topologies = augment
        .topologies_for(&deck_tokens)
        .into_iter()
        .map(|t| TopologyInfoDto {
            name: t.name.clone(),
            principle: t.principle.clone(),
            regions: t
                .regions
                .iter()
                .map(|r| RegionInfoDto {
                    name: r.name.clone(),
                    cells: crate::session::card_strengths(&r.cards, store, now),
                })
                .collect(),
        })
        .collect();
    DeckDrawerDto {
        preamble: deck.preamble.clone(),
        heatmap,
        topologies,
    }
}

pub(super) fn card_dto(view: CardView) -> CardDto {
    let img_dto = |i: &review::ImageView| ImageDto {
        src: format!("/img/{}", img_key(Path::new(&i.src))),
        alt: i.alt.clone(),
    };
    let (front, front_runs) = inline_parts(view.front);
    let (back, back_runs) = inline_lines(view.back);
    CardDto {
        images: view.images.iter().map(&img_dto).collect(),
        images_back: view.images_back.iter().map(&img_dto).collect(),
        front,
        front_runs,
        context: view.context,
        back,
        back_runs,
        reshaped: view.reshaped,
        note: view.note,
        at: view.at,
        citation: None,
        citation_error: None,
        crumb: None,
    }
}

fn inline_parts(text: String) -> (String, Vec<InlineRun>) {
    let runs = parse_inline(&text);
    let mut content = String::new();
    for run in &runs {
        content.push_str(&run.text);
    }
    (content, runs)
}

fn literal_parts(text: String) -> (String, Vec<InlineRun>) {
    let runs = if text.is_empty() {
        Vec::new()
    } else {
        vec![InlineRun {
            text: text.clone(),
            ..InlineRun::default()
        }]
    };
    (text, runs)
}

fn inline_lines(lines: Vec<String>) -> (Vec<String>, Vec<Vec<InlineRun>>) {
    let mut content = Vec::with_capacity(lines.len());
    let mut display = Vec::with_capacity(lines.len());
    let mut in_code = false;
    for line in lines {
        let fence = line.trim_start().starts_with("```");
        let (plain, runs) = if in_code || fence {
            literal_parts(line)
        } else {
            inline_parts(line)
        };
        content.push(plain);
        display.push(runs);
        if fence {
            in_code = !in_code;
        }
    }
    (content, display)
}

pub(super) fn input_name(input: Input) -> &'static str {
    match input {
        Input::Type => "type",
        Input::Draw => "draw",
    }
}
