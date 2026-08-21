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
    inline::{DisplayProjector, InlineRun},
    render::NoteUnit,
    review::{self, CardView},
    session::{CardTier, now_ms},
    source::{Excerpt, relabel_for_display},
    store::Store,
    trace::{Delta, Phase},
};

#[derive(Debug, Serialize)]
pub(super) struct CardDto {
    pub(super) id: Option<String>,
    pub(super) front: String,
    pub(super) front_runs: Vec<InlineRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) front_units: Option<Vec<NoteUnit>>,
    pub(super) context: Vec<String>,
    pub(super) context_leads: bool,
    pub(super) context_runs: Vec<Vec<InlineRun>>,
    pub(super) context_units: Vec<NoteUnit>,
    pub(super) back: Vec<String>,
    pub(super) back_runs: Vec<Vec<InlineRun>>,
    pub(super) back_units: Vec<NoteUnit>,
    pub(super) reshaped: bool,
    pub(super) note: Vec<NoteUnit>,
    pub(super) images: Vec<ImageDto>,
    pub(super) images_back: Vec<ImageDto>,
    pub(super) citations: Vec<CitationDto>,
    pub(super) crumb: Option<CrumbDto>,
}

#[derive(Debug, Serialize)]
pub(super) struct CitationDto {
    pub(super) locator: String,
    pub(super) excerpt: Option<ExcerptDto>,
    pub(super) error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ImageDto {
    pub(super) src: String,
    pub(super) alt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) regions: Vec<review::RegionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) crop: Option<review::CropView>,
}

#[derive(Debug, Serialize)]
pub(super) struct CrumbDto {
    pub(super) regions: Vec<String>,
    pub(super) current: usize,
    pub(super) cells: Vec<Vec<CardTier>>,
}

#[derive(Debug, Serialize, Default)]
pub(super) struct DeckDrawerDto {
    pub(super) preamble: Option<String>,
    pub(super) heatmap: Vec<CardTier>,
    pub(super) topologies: Vec<TopologyInfoDto>,
    /// Total cards in the deck. Not derivable from `heatmap.len()`, which counts
    /// only stamped cards.
    pub(super) total: usize,
    /// A nested funnel over `total`: `retired <= graduated <= seen <= total`.
    /// `seen` is any card with a store entry, `graduated` reaches FSRS review
    /// (surfaced to the user as "learned"), `retired` is past the retire cap.
    pub(super) seen: usize,
    pub(super) graduated: usize,
    pub(super) retired: usize,
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
    pub(super) cells: Vec<CardTier>,
}

#[derive(Debug, Serialize)]
pub(super) struct StateDto {
    pub(super) kind: &'static str,
    /// Monotonic identity of the current review transition. Card-relative
    /// mutations echo it in `X-Alix-Study-Revision`; a stale echo is a 409.
    pub(super) study_revision: u64,
    /// No separate `finished` flag: a finished session is just the `done` phase.
    pub(super) phase: &'static str,
    pub(super) card: Option<CardDto>,
    pub(super) choices: Option<Vec<String>>,
    pub(super) choice_runs: Option<Vec<Vec<InlineRun>>>,
    pub(super) keypoints: Option<Vec<String>>,
    pub(super) keypoint_runs: Option<Vec<Vec<InlineRun>>>,
    pub(super) introducing: bool,
    pub(super) mode: &'static str,
    pub(super) depth: &'static str,
    pub(super) input: &'static str,
    pub(super) remaining: u32,
    pub(super) initial: u32,
    pub(super) reviews: u32,
    pub(super) passed: u32,
    pub(super) failed: u32,
    pub(super) introduced: u32,
    pub(super) partial: u32,
    pub(super) exam_due: Vec<String>,
    pub(super) can_restart: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_due_ms: Option<u64>,
    pub(super) due_left: u32,
    pub(super) new_left: u32,
    pub(super) met_total: u32,
    pub(super) deck_total: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) recognize_gap: Option<crate::session::RecognizeGap>,
    pub(super) label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) save_error: Option<String>,
    /// Deck-load diagnostics for the open session (a stamped diagram that
    /// did not resolve); absent when there are none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) load_warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct BrowseDto {
    pub(super) phase: &'static str,
    pub(super) label: String,
    pub(super) cards: Vec<CardDto>,
}

/// A deck inside a workspace stays out of `recent`: reachable only via its
/// workspace.
#[derive(Clone, Debug, Serialize)]
pub(super) struct DeckListDto {
    pub(super) workspaces: Vec<DeckItemDto>,
    pub(super) recent: Vec<DeckItemDto>,
    pub(super) folders: Vec<DeckItemDto>,
}

#[derive(Clone, Debug, Serialize)]
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
    pub(super) crammable: bool,
    pub(super) last_depth: &'static str,
    pub(super) deadline: Option<DeadlineDto>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct DeadlineDto {
    pub(super) date: String, // ISO YYYY-MM-DD
    pub(super) days_left: i64,
    pub(super) ready: usize,
    pub(super) total: usize,
}

#[derive(Clone, Debug, Serialize)]
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
    pub(super) crammable: bool,
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

#[derive(Debug, Serialize)]
pub(super) struct RemovalPreviewDto {
    pub(super) target: String,
    pub(super) kind: &'static str,
    pub(super) decks: usize,
    pub(super) cards_with_progress: usize,
    pub(super) earliest_review_ms: Option<u64>,
    pub(super) files: Vec<String>,
    pub(super) directories: Vec<String>,
    pub(super) dependents: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct RemovalDto {
    pub(super) target: String,
    pub(super) kind: &'static str,
    pub(super) removed: Vec<String>,
    pub(super) decks_removed: usize,
    pub(super) directory_removed: bool,
    pub(super) dependents: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct RemovalFailureDto {
    pub(super) target: String,
    pub(super) error: &'static str,
    pub(super) completed: Vec<String>,
    pub(super) failed: String,
    pub(super) recovery: &'static str,
}

/// Unlike `generate`'s lenient save, a non-parsing upload is rejected outright.
#[derive(Serialize)]
pub(super) struct ImportDto {
    pub(super) deck: String,
    pub(super) cards: usize,
}

impl AskInfoDto {
    pub(super) fn from(cfg: &AskConfig) -> Self {
        let or_default = |s: Option<String>| s.unwrap_or_else(|| "default".to_string());
        let backend = cfg.backend.name();
        Self {
            backend,
            // A pinned model is known up front; otherwise report what the
            // backend itself said it loaded, rather than guessing for it.
            model: or_default(
                crate::backend::resolved_ask_model(cfg)
                    .or_else(|| crate::ask::observed_model(backend)),
            ),
            effort: or_default(crate::backend::resolved_ask_effort(cfg)),
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

pub(super) fn exam_dto(ex: &Examining) -> ExamDto {
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
        deck::dependents(&ex.deck_path)
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
    pub(super) description_runs: Vec<InlineRun>,
    pub(super) source: Option<String>,
    pub(super) total: usize,
    pub(super) current: usize,
    pub(super) path: Vec<HopDto>,
    pub(super) prompt: Option<String>,
    pub(super) prompt_runs: Option<Vec<InlineRun>>,
    pub(super) givens: Vec<String>,
    pub(super) given_runs: Vec<Vec<InlineRun>>,
    pub(super) locator: Option<String>,
    pub(super) prediction: Option<String>,
    pub(super) excerpt: Option<ExcerptDto>,
    pub(super) excerpt_error: Option<String>,
    pub(super) points: Vec<String>,
    pub(super) point_runs: Vec<Vec<InlineRun>>,
    pub(super) note: Option<String>,
    pub(super) note_runs: Option<Vec<InlineRun>>,
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
    let mut projector = DisplayProjector::default();

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
        description_runs: projector.project(&trace.description),
        source: trace.source.clone(),
        total: walk.total(),
        current: walk.current_index() + 1,
        path,
        prompt: None,
        prompt_runs: None,
        givens: Vec::new(),
        given_runs: Vec::new(),
        locator: None,
        prediction: None,
        excerpt: None,
        excerpt_error: None,
        points: Vec::new(),
        point_runs: Vec::new(),
        note: None,
        note_runs: None,
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
                dto.prompt_runs = Some(projector.project(&c.prompt));
                dto.givens = c.givens.clone();
                dto.given_runs = c
                    .givens
                    .iter()
                    .map(|given| projector.project(given))
                    .collect();
                dto.locator = c.locator.clone();
            }
        }
        Phase::Reveal => {
            if let Some(c) = walk.checkpoint() {
                dto.prompt = Some(c.prompt.clone());
                dto.prompt_runs = Some(projector.project(&c.prompt));
                dto.givens = c.givens.clone();
                dto.given_runs = c
                    .givens
                    .iter()
                    .map(|given| projector.project(given))
                    .collect();
                dto.locator = c.locator.clone();
                dto.points = c.points.clone();
                dto.point_runs = c
                    .points
                    .iter()
                    .map(|point| projector.project(point))
                    .collect();
                dto.note = c.note.clone();
                dto.note_runs = c.note.as_deref().map(|note| projector.project(note));
                match trace.excerpt(c) {
                    Ok(ex) => {
                        // Repoint a frozen excerpt's asset path at the real
                        // `at:` source path for display.
                        let ex =
                            if let Some(at) = c.locator.as_deref().filter(|_| c.asset.is_some()) {
                                let (ex, label) = relabel_for_display(ex, at);
                                if let Some(label) = label {
                                    dto.locator = Some(label);
                                }
                                ex
                            } else {
                                ex
                            };
                        dto.excerpt = Some(excerpt_dto(&ex.capped_for_display()));
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
        Some(b) => {
            let mut projector = DisplayProjector::default();
            BrowseDto {
                phase: "browse",
                label: b.label.clone(),
                cards: b
                    .cards
                    .iter()
                    .map(|card| card_dto(CardView::project(card, &mut projector), card.id()))
                    .collect(),
            }
        }
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
pub(super) fn review_state(
    reviewing: Option<&Reviewing>,
    store: &Store,
    save_error: Option<&str>,
    study_revision: u64,
) -> StateDto {
    let Some(r) = reviewing else {
        return StateDto {
            kind: "review",
            study_revision,
            phase: "select",
            card: None,
            choices: None,
            choice_runs: None,
            keypoints: None,
            keypoint_runs: None,
            introducing: false,
            mode: mode_name(Mode::default()),
            depth: depth_name(Depth::default()),
            input: input_name(Input::default()),
            remaining: 0,
            initial: 0,
            reviews: 0,
            passed: 0,
            failed: 0,
            introduced: 0,
            partial: 0,
            exam_due: Vec::new(),
            can_restart: false,
            next_due_ms: None,
            due_left: 0,
            new_left: 0,
            met_total: 0,
            deck_total: 0,
            recognize_gap: None,
            label: "select decks".to_string(),
            save_error: save_error.map(str::to_string),
            load_warnings: Vec::new(),
        };
    };
    let session = &r.session;
    // Every fact here comes from the shared `crate::review` contract, the same
    // state embedded mobile renders; this envelope only adds wire naming and
    // serve-held context.
    let s = review::state(session, store, &r.augment, None);
    // Only computed when finished: it reloads decks, so this stays off the hot path.
    let exam_due = if s.finished {
        // `r.files.paths` is keyed by deck_id (routing only); the wire value
        // clients resolve `/api/exam/start` by is the deck's own name, read
        // back off the loaded deck, not the map key.
        let mut due: Vec<String> = r
            .files
            .paths
            .values()
            .filter_map(|path| {
                Deck::load(path)
                    .ok()
                    .filter(|d| d.state(store) == DeckState::ExamDue)
                    .map(|d| d.subject)
            })
            .collect();
        due.sort();
        due
    } else {
        Vec::new()
    };
    let card_with_citation = s.card.zip(session.current()).map(|(view, c)| {
        let mut dto = card_dto(view, c.id());
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
                    .map(|reg| {
                        crate::session::card_tiers(
                            &reg.cards,
                            store,
                            now_ms(),
                            session.retire_after_days(),
                        )
                    })
                    .collect(),
            });
        }
        dto.citations = c
            .citations
            .iter()
            .map(|citation| {
                let mut resolved = CitationDto {
                    locator: citation.locator.clone(),
                    excerpt: None,
                    error: None,
                };
                if let Some(base) = r.source_bases.get(&*c.deck_id) {
                    match base.checked_excerpt(citation) {
                        Ok(ex) => {
                            // Repoint a frozen excerpt's asset path at the real
                            // `at:` source path, so the citation reads
                            // `store.rs:36-66`, not the asset object's path.
                            let ex = if citation.asset.is_some() {
                                let (ex, label) = relabel_for_display(ex, &citation.locator);
                                if let Some(label) = label {
                                    resolved.locator = label;
                                }
                                ex
                            } else {
                                ex
                            };
                            resolved.excerpt = Some(excerpt_dto(&ex.capped_for_display()));
                        }
                        Err(e) => resolved.error = Some(format!("{e:#}")),
                    }
                }
                resolved
            })
            .collect();
        dto
    });
    StateDto {
        kind: "review",
        study_revision,
        phase: if s.finished { "done" } else { "review" },
        card: card_with_citation,
        choices: s.choices,
        choice_runs: s.choice_runs,
        keypoints: s.keypoints,
        keypoint_runs: s.keypoint_runs,
        introducing: s.introducing,
        mode: mode_name(s.mode),
        depth: depth_name(s.depth),
        input: input_name(s.input),
        remaining: s.remaining,
        initial: s.initial,
        reviews: s.reviews,
        passed: s.passed,
        failed: s.failed,
        introduced: s.introduced,
        partial: s.partial,
        exam_due,
        can_restart: s.can_restart,
        next_due_ms: s.next_due_ms,
        due_left: s.due_left,
        new_left: s.new_left,
        met_total: s.met_total,
        deck_total: s.deck_total,
        recognize_gap: s.recognize_gap,
        label: r.label.clone(),
        save_error: save_error.map(str::to_string),
        load_warnings: r.load_warnings.clone(),
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
    retire_after_days: Option<u32>,
) -> DeckDrawerDto {
    let deck_tokens: HashSet<String> = deck.deck_token.iter().cloned().collect();
    let now = now_ms();
    // A flat per-card heatmap over the whole deck, in file order; a topology (if
    // any) re-groups the same signal into named regions below. The learned
    // bands are pinned to Recall retrievability: a deck-wide signal, not
    // per-session.
    let ids: Vec<String> = deck.cards.iter().filter_map(|c| c.id()).collect();
    let heatmap = crate::session::card_tiers(&ids, store, now, retire_after_days);
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
                    cells: crate::session::card_tiers(&r.cards, store, now, retire_after_days),
                })
                .collect(),
        })
        .collect();
    let seen = deck
        .cards
        .iter()
        .filter(|c| c.id().and_then(|id| store.get(&id)).is_some())
        .count();
    let graduated = deck
        .cards
        .iter()
        .filter(|c| crate::session::has_graduated(c, store))
        .count();
    let retired = deck
        .cards
        .iter()
        .filter(|c| crate::session::is_retired(c, store, retire_after_days))
        .count();
    DeckDrawerDto {
        preamble: deck.preamble.clone(),
        heatmap,
        topologies,
        total: deck.cards.len(),
        seen,
        graduated,
        retired,
    }
}

/// A Diagram unit's lib-side `src` is an absolute file path (the mobile
/// projection reads it directly); over HTTP it becomes the same opaque
/// `/img/<key>` URL every card image uses.
fn web_units(units: Vec<crate::render::NoteUnit>) -> Vec<crate::render::NoteUnit> {
    units
        .into_iter()
        .map(|unit| match unit {
            crate::render::NoteUnit::Diagram {
                src,
                width,
                height,
                alt,
                regions,
                revealed_alt,
            } => crate::render::NoteUnit::Diagram {
                src: format!("/img/{}", img_key(Path::new(&src))),
                width,
                height,
                alt,
                regions,
                revealed_alt,
            },
            other => other,
        })
        .collect()
}

pub(super) fn card_dto(view: CardView, id: Option<String>) -> CardDto {
    let img_dto = |i: &review::ImageView| ImageDto {
        src: format!("/img/{}", img_key(Path::new(&i.src))),
        alt: i.alt.clone(),
        regions: i.regions.clone(),
        crop: i.crop.clone(),
    };
    CardDto {
        id,
        images: view.images.iter().map(&img_dto).collect(),
        images_back: view.images_back.iter().map(&img_dto).collect(),
        front: view.front,
        front_runs: view.front_runs,
        front_units: view.front_units.map(web_units),
        context: view.context,
        context_leads: view.context_leads,
        context_runs: view.context_runs,
        context_units: web_units(view.context_units),
        back: view.back,
        back_runs: view.back_runs,
        back_units: web_units(view.back_units),
        reshaped: view.reshaped,
        note: web_units(view.note),
        citations: view
            .citations
            .into_iter()
            .map(|locator| CitationDto {
                locator,
                excerpt: None,
                error: None,
            })
            .collect(),
        crumb: None,
    }
}

pub(super) fn input_name(input: Input) -> &'static str {
    match input {
        Input::Type => "type",
        Input::Draw => "draw",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_drawer_total_counts_unstamped_cards_the_heatmap_omits() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "---\nformat-version: 1\nid: \"deck-rust\"\n---\n## q1 <!-- id: card-q1 -->\na1\n## q2\na2\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let store = Store::open(dir.path().join("deck1.json")).unwrap();
        let augment = AugmentCache::open(dir.path().join("deck1-generated.json"));

        let dto = deck_drawer_dto(&augment, &store, &deck, None);

        assert_eq!(2, dto.total, "both cards count toward the deck size");
        assert_eq!(
            1,
            dto.heatmap.len(),
            "only the stamped card lands in the heatmap"
        );
        assert_eq!(
            (0, 0, 0),
            (dto.seen, dto.graduated, dto.retired),
            "a fresh deck's funnel is all zeros (each hidden client-side)"
        );
    }

    #[test]
    fn deck_drawer_funnel_counts_nest_total_seen_graduated_retired() {
        use crate::scheduler::{Fsrs, Grade, Scheduler};

        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.md");
        std::fs::write(
            &deck_path,
            "---\nformat-version: 1\nid: \"deck-rust\"\n---\n\
             ## a <!-- id: card-a -->\n1\n\
             ## b <!-- id: card-b -->\n2\n\
             ## c <!-- id: card-c -->\n3\n\
             ## d <!-- id: card-d -->\n4\n",
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let augment = AugmentCache::open(dir.path().join("deck1-generated.json"));
        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let now = 1_000_000;
        let sched = Fsrs::default();

        // a: untouched (no store entry) → unseen.
        // b: a bare store entry → seen only.
        store.get_or_insert(&deck.cards[1].id().unwrap());
        // c: two real Recall Goods → graduated (state 2) at a sub-cap interval.
        let c = store.get_or_insert(&deck.cards[2].id().unwrap());
        sched.apply(c, Depth::Recall, Grade::Pass, now, false);
        sched.apply(c, Depth::Recall, Grade::Pass, now, false);
        assert!(
            c.schedule(Depth::Recall).is_some_and(|f| f.graduated()),
            "two Goods must graduate the card"
        );
        // d: a matured Review state past the retire cap → retired (and graduated).
        store.get_or_insert(&deck.cards[3].id().unwrap()).recall = Some(crate::store::FsrsState {
            state: 2,
            stability: 400.0,
            scheduled_days: 400,
            due_ms: now + 400 * 86_400_000,
            ..Default::default()
        });

        let dto = deck_drawer_dto(&augment, &store, &deck, Some(365));

        assert_eq!(4, dto.total);
        assert_eq!(3, dto.seen, "b, c, d have store entries");
        assert_eq!(2, dto.graduated, "c and d reached FSRS review");
        assert_eq!(1, dto.retired, "only d is past the 365-day cap");
        assert!(
            dto.total >= dto.seen && dto.seen >= dto.graduated && dto.graduated >= dto.retired,
            "the funnel must nest: {} >= {} >= {} >= {}",
            dto.total,
            dto.seen,
            dto.graduated,
            dto.retired
        );
    }
}
