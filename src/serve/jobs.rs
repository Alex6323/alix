use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
    time::Instant,
};

use anyhow::{Result, bail};

use super::{
    CardsBuild, SessionBuild,
    catalog::{DeckFiles, collect_images},
    dto::*,
};
use crate::{
    ask::{self, CliSession, Exchange, Reply},
    augment::{self, AugmentCache},
    augment_ai,
    card::Card,
    config::{AiConfig, AskConfig, Audience, GenerateDeckConfig},
    exam, generate, parser,
    session::{Session, now_ms},
    share,
    trace::{self, Delta, SourceBase, Walk},
    trace_ai,
};

pub(super) struct Reviewing {
    pub(super) session: Session,
    pub(super) label: String,
    pub(super) files: DeckFiles,
    pub(super) images: HashMap<String, PathBuf>,
    pub(super) ask: Ask,
    pub(super) links: HashMap<String, Vec<String>>,
    pub(super) source_roots: HashMap<String, PathBuf>,
    pub(super) source_bases: HashMap<String, SourceBase>,
    pub(super) augment: AugmentCache,
    pub(super) present_seq: u64,
    pub(super) original_fronts: HashMap<String, String>,
    pub(super) topology_name: Option<String>,
}

pub(super) struct Pending {
    pub(super) rx: Receiver<Reply>,
    pub(super) purpose: Purpose,
    pub(super) card: Card,
}

pub(super) enum Purpose {
    Question(String),
    Condense,
    DraftCard,
}

pub(super) enum AskAction {
    Question(String),
    Condense,
    DraftCard,
}

pub(super) struct Ask {
    pub(super) cli: CliSession,
    pub(super) transcript: Vec<Exchange>,
    pub(super) subject: Option<String>,
    pub(super) pending: Option<Pending>,
    pub(super) draft: Option<ask::DraftCard>,
}

impl Ask {
    fn new() -> Self {
        Self {
            cli: CliSession::new(),
            transcript: Vec::new(),
            subject: None,
            pending: None,
            draft: None,
        }
    }

    fn align(&mut self, subject: Option<String>) {
        if self.subject != subject {
            self.transcript.clear();
            self.draft = None;
            self.subject = subject;
        }
    }

    fn dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        AskDto {
            transcript: self
                .transcript
                .iter()
                .map(|(q, a)| ExchangeDto {
                    q: q.clone(),
                    a: a.clone(),
                })
                .collect(),
            thinking: self.pending.is_some(),
            status,
            error,
            draft: self.draft.as_ref().map(|d| DraftCardDto {
                front: d.front.clone(),
                back: d.back.clone(),
            }),
        }
    }

    #[expect(clippy::too_many_arguments)] // each is a distinct, named tutor input
    fn start(
        &mut self,
        cfg: &AskConfig,
        audience: Audience,
        card: &Card,
        links: &[String],
        root: Option<&Path>,
        frozen: Option<&str>,
        action: AskAction,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.align(card.id());
        if matches!(action, AskAction::Condense | AskAction::DraftCard)
            && self.transcript.is_empty()
        {
            return false;
        }
        let run_cfg = match root {
            Some(r) => ask::with_source_root(cfg, r),
            None => cfg.clone(),
        };
        let args = self.cli.args_in(run_cfg.cwd.as_deref());
        let keeps_session = crate::backend::backend_for(&run_cfg)
            .map(|b| b.supports_session())
            .unwrap_or(false);
        let (prompt, purpose) = match action {
            AskAction::Question(q) => {
                let prompt = if keeps_session {
                    ask::question_prompt(card, audience, links, &q, !self.cli.started, root, frozen)
                } else {
                    ask::question_prompt_with_history(
                        card,
                        audience,
                        links,
                        &self.transcript,
                        &q,
                        root,
                        frozen,
                    )
                };
                (prompt, Purpose::Question(q))
            }
            AskAction::Condense => (
                ask::condense_prompt(card, &self.transcript),
                Purpose::Condense,
            ),
            AskAction::DraftCard => (
                ask::draft_card_prompt(card, &self.transcript),
                Purpose::DraftCard,
            ),
        };
        let rx = ask::spawn(run_cfg, prompt, args);
        self.pending = Some(Pending {
            rx,
            purpose,
            card: card.clone(),
        });
        true
    }

    fn answer_immediately(&mut self, card: &Card, question: String, answer: String) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.align(card.id());
        // self.cli is deliberately left untouched: no real turn happened.
        self.transcript.push((question, answer));
        true
    }

    fn poll(
        &mut self,
        save: impl FnOnce(&Card, &[String]) -> Result<(), String>,
    ) -> (Option<String>, Option<String>) {
        let reply = match &self.pending {
            None => return (None, None),
            Some(p) => match p.rx.try_recv() {
                Ok(reply) => reply,
                Err(TryRecvError::Empty) => return (None, None),
                Err(TryRecvError::Disconnected) => {
                    Reply::Error("the ask helper exited unexpectedly".to_string())
                }
            },
        };
        let pending = self.pending.take().expect("pending was present");
        match (reply, pending.purpose) {
            (Reply::Answer(answer), Purpose::Question(question)) => {
                self.cli.started = true;
                self.transcript.push((question, answer));
                (None, None)
            }
            (Reply::Answer(text), Purpose::Condense) => {
                self.cli.started = true;
                let notes = ask::extract_note_lines(&text);
                if notes.is_empty() {
                    return (Some("nothing to save".to_string()), None);
                }
                match save(&pending.card, &notes) {
                    Ok(()) => (Some("note saved".to_string()), None),
                    Err(e) => (None, Some(e)),
                }
            }
            (Reply::Answer(text), Purpose::DraftCard) => {
                self.cli.started = true;
                match ask::parse_drafted_card(&text) {
                    Ok(card) => {
                        self.draft = Some(card);
                        (Some("card drafted".to_string()), None)
                    }
                    Err(e) => (None, Some(e.to_string())),
                }
            }
            // never resume a session in an unknown state; start fresh next time
            (Reply::Error(e), _) => {
                self.cli = CliSession::new();
                (None, Some(e))
            }
        }
    }
}

fn remote_card(c: &RemoteCard) -> Card {
    let mut card = Card::plain(
        Arc::from(c.subject.as_str()),
        c.front.clone(),
        c.back.clone(),
        None,
        0,
    );
    card.at = c.at.clone();
    card
}

enum RemoteAskPurpose {
    Question,
    Draft,
    Note,
}

enum RemoteAskOutcome {
    Answer(String),
    Draft(ask::DraftCard),
    // an empty vec is a settled success, not an error
    Note(Vec<String>),
    Error(String),
}

pub(super) struct RemoteAsk {
    rx: Receiver<Reply>,
    purpose: RemoteAskPurpose,
    started_ms: u64,
    outcome: Option<RemoteAskOutcome>,
}

impl RemoteAsk {
    pub(super) fn ask(
        cfg: &AskConfig,
        card: &RemoteCard,
        history: Vec<RemoteTurn>,
        question: &str,
    ) -> Self {
        let card = remote_card(card);
        let prior: Vec<Exchange> = history.into_iter().map(|t| (t.q, t.a)).collect();
        let prompt = ask::question_prompt_with_history(
            &card,
            Audience::Adult,
            &[],
            &prior,
            question,
            None,
            None,
        );
        Self::spawn(cfg, prompt, RemoteAskPurpose::Question)
    }

    pub(super) fn draft(cfg: &AskConfig, card: &RemoteCard, history: Vec<RemoteTurn>) -> Self {
        let card = remote_card(card);
        let prior: Vec<Exchange> = history.into_iter().map(|t| (t.q, t.a)).collect();
        let prompt = ask::draft_card_prompt(&card, &prior);
        Self::spawn(cfg, prompt, RemoteAskPurpose::Draft)
    }

    pub(super) fn note(cfg: &AskConfig, card: &RemoteCard, history: Vec<RemoteTurn>) -> Self {
        let card = remote_card(card);
        let prior: Vec<Exchange> = history.into_iter().map(|t| (t.q, t.a)).collect();
        let prompt = ask::condense_prompt(&card, &prior);
        Self::spawn(cfg, prompt, RemoteAskPurpose::Note)
    }

    fn spawn(cfg: &AskConfig, prompt: String, purpose: RemoteAskPurpose) -> Self {
        let rx = ask::spawn(cfg.clone(), prompt, Vec::new());
        Self {
            rx,
            purpose,
            started_ms: now_ms(),
            outcome: None,
        }
    }

    pub(super) fn thinking(&self) -> bool {
        self.outcome.is_none()
    }

    pub(super) fn poll(&mut self) {
        if self.outcome.is_some() {
            return;
        }
        let reply = match self.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Reply::Error("the ask helper exited unexpectedly".to_string())
            }
        };
        self.outcome = Some(match (reply, &self.purpose) {
            (Reply::Answer(text), RemoteAskPurpose::Question) => RemoteAskOutcome::Answer(text),
            (Reply::Answer(text), RemoteAskPurpose::Draft) => {
                match ask::parse_drafted_card(&text) {
                    Ok(card) => RemoteAskOutcome::Draft(card),
                    Err(e) => RemoteAskOutcome::Error(e.to_string()),
                }
            }
            (Reply::Answer(text), RemoteAskPurpose::Note) => {
                RemoteAskOutcome::Note(ask::extract_note_lines(&text))
            }
            (Reply::Error(e), _) => RemoteAskOutcome::Error(e),
        });
    }

    pub(super) fn dto(&self) -> RemoteAskDto {
        match &self.outcome {
            None => RemoteAskDto {
                thinking: true,
                answer: None,
                draft: None,
                note: None,
                error: None,
                elapsed: Some(now_ms().saturating_sub(self.started_ms) / 1000),
            },
            Some(RemoteAskOutcome::Answer(a)) => RemoteAskDto {
                thinking: false,
                answer: Some(a.clone()),
                draft: None,
                note: None,
                error: None,
                elapsed: None,
            },
            Some(RemoteAskOutcome::Draft(d)) => RemoteAskDto {
                thinking: false,
                answer: None,
                draft: Some(DraftCardDto {
                    front: d.front.clone(),
                    back: d.back.clone(),
                }),
                note: None,
                error: None,
                elapsed: None,
            },
            Some(RemoteAskOutcome::Note(lines)) => RemoteAskDto {
                thinking: false,
                answer: None,
                draft: None,
                note: Some(lines.clone()),
                error: None,
                elapsed: None,
            },
            Some(RemoteAskOutcome::Error(e)) => RemoteAskDto {
                thinking: false,
                answer: None,
                draft: None,
                note: None,
                error: Some(e.clone()),
                elapsed: None,
            },
        }
    }
}

pub(super) struct RemoteGenerating {
    rx: Receiver<Result<String, String>>,
    url: String,
    started_ms: u64,
    outcome: Option<Result<String, String>>,
}

impl RemoteGenerating {
    pub(super) fn start(url: String, cfg: GenerateDeckConfig, ask_cfg: AskConfig) -> Self {
        let rx = generate::spawn(url.clone(), cfg, ask_cfg);
        Self {
            rx,
            url,
            started_ms: now_ms(),
            outcome: None,
        }
    }

    pub(super) fn thinking(&self) -> bool {
        self.outcome.is_none()
    }

    pub(super) fn poll(&mut self) {
        if self.outcome.is_some() {
            return;
        }
        let result = match self.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("the generate worker exited unexpectedly".to_string())
            }
        };
        self.outcome = Some(result);
    }

    // Mirrors `library::place_deck`'s normalization; never calls it, since
    // the server must not place files here.
    fn suggested_filename(&self) -> String {
        let name = generate::deck_name(&self.url);
        let stem = name.strip_suffix(".md").unwrap_or(&name);
        format!("{stem}.md")
    }

    pub(super) fn dto(&self) -> RemoteGenerateDto {
        match &self.outcome {
            None => RemoteGenerateDto {
                phase: "generating",
                deck: None,
                filename: None,
                cards: None,
                elapsed: Some(now_ms().saturating_sub(self.started_ms) / 1000),
                error: None,
            },
            Some(Ok(text)) => {
                let filename = self.suggested_filename();
                let cards = parser::parse_str(&filename, text).ok().map(|c| c.len());
                RemoteGenerateDto {
                    phase: "done",
                    deck: Some(text.clone()),
                    filename: Some(filename),
                    cards,
                    elapsed: None,
                    error: None,
                }
            }
            Some(Err(e)) => RemoteGenerateDto {
                phase: "error",
                deck: None,
                filename: None,
                cards: None,
                elapsed: None,
                error: Some(e.clone()),
            },
        }
    }
}

impl Reviewing {
    pub(super) fn new(build: SessionBuild) -> Self {
        let images = collect_images(build.session.cards());
        Self {
            session: build.session,
            label: build.label,
            files: DeckFiles::new(build.decks),
            images,
            ask: Ask::new(),
            links: build.links,
            source_roots: build.source_roots,
            source_bases: build.source_bases,
            // placeholder until `open_augment` sets the real store path
            augment: AugmentCache::open(Path::new("")),
            present_seq: now_ms(),
            original_fronts: HashMap::new(),
            topology_name: build.topology_name,
        }
    }

    pub(super) fn open_augment(&mut self, store_path: &Path) {
        self.augment = AugmentCache::open(augment::augment_path_for(store_path));
    }

    pub(super) fn rotate_variant(&mut self) {
        let Some(id) = self.session.current().and_then(|c| c.id()) else {
            return;
        };
        if self.augment.variants(&id).is_none() {
            return;
        }
        if !self.original_fronts.contains_key(&id)
            && let Some(card) = self.session.current()
        {
            self.original_fronts.insert(id.clone(), card.front.clone());
        }
        let original = self.original_fronts.get(&id).cloned().unwrap_or_default();
        let seed = self.present_seq;
        self.present_seq = self.present_seq.wrapping_add(1);
        if let Some(chosen) = self.augment.pick_front(&id, &original, seed)
            && let Some(card) = self.session.current_mut()
        {
            card.front = chosen;
        }
    }

    pub(super) fn align_transcript(&mut self) {
        self.ask.align(self.session.current().and_then(|c| c.id()));
    }

    pub(super) fn ask_dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        self.ask.dto(status, error)
    }

    pub(super) fn start_ask(
        &mut self,
        cfg: &AskConfig,
        audience: Audience,
        action: AskAction,
    ) -> bool {
        let Some(card) = self.session.current().cloned() else {
            return false;
        };
        let links = self.links.get(&*card.subject).cloned().unwrap_or_default();
        let root = self.source_roots.get(&*card.subject).map(|deck_root| {
            card.origin
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| deck_root.clone())
        });
        let frozen = root.as_ref().and_then(|_| {
            let at = card.at.as_deref()?;
            let base = self.source_bases.get(&*card.subject)?;
            trace::frozen_excerpt_block(at, card.at_origin.as_deref(), base)
        });
        let live_root = root.as_deref().filter(|r| r.exists());
        // answering here avoids a round trip that would just have the model
        // echo SOURCE_NOT_FOUND verbatim
        if let AskAction::Question(q) = &action
            && frozen.is_some()
            && live_root.is_none()
        {
            return self.ask.answer_immediately(
                &card,
                q.clone(),
                ask::SOURCE_NOT_FOUND.to_string(),
            );
        }
        self.ask.start(
            cfg,
            audience,
            &card,
            &links,
            live_root,
            frozen.as_deref(),
            action,
        )
    }

    pub(super) fn poll_ask(&mut self) -> (Option<String>, Option<String>) {
        self.align_transcript();
        let Self {
            ask,
            files,
            session,
            ..
        } = self;
        ask.poll(|card, notes| {
            files.append_note(&card.subject, card.line, notes)?;
            if let Some(cur) = session.current_mut()
                && cur.id() == card.id()
            {
                cur.append_note(notes);
            }
            Ok(())
        })
    }
}

pub(super) struct Examining {
    pub(super) sitting: exam::Sitting,
    pub(super) deck_path: PathBuf,
}

pub(super) struct RemoteExamining {
    pub(super) sitting: exam::Sitting,
    pub(super) cards: Option<String>,
}

impl RemoteExamining {
    pub(super) fn advance(&mut self) {
        match self.sitting.advance(now_ms()) {
            Some(exam::Effect::RemediationCards(text)) => self.cards = Some(text),
            Some(exam::Effect::Passed | exam::Effect::TraceFailed) | None => {}
        }
    }

    pub(super) fn dto(&self) -> RemoteExamDto {
        let s = &self.sitting;
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
        RemoteExamDto {
            phase: exam_phase_name(s.phase()),
            deck: s.subject().to_string(),
            strictness: strictness_name(s.strictness()),
            // prompts only: the rubric never leaves the server outside a
            // graded result
            questions: s.questions().iter().map(|q| q.prompt.clone()).collect(),
            passed: result.map(|r| r.passed),
            grades,
            gaps: s.gaps(),
            can_remediate: s.can_remediate(),
            cards: self.cards.clone(),
            is_trace: s.kind() == exam::SittingKind::Trace,
            thinking: s.thinking(),
            elapsed: s.elapsed_secs(),
            error: s.error().map(str::to_string),
        }
    }
}

pub(super) fn remote_exam_idle_dto() -> RemoteExamDto {
    RemoteExamDto {
        phase: "idle",
        deck: String::new(),
        strictness: "balanced",
        questions: Vec::new(),
        passed: None,
        grades: Vec::new(),
        gaps: Vec::new(),
        can_remediate: false,
        cards: None,
        is_trace: false,
        thinking: false,
        elapsed: None,
        error: None,
    }
}

pub(super) struct Augmenting {
    pub(super) deck: String,
    pub(super) cards: Vec<Card>,
    pub(super) deck_ids: HashSet<String>,
    deck_tokens: HashSet<String>,
    primary_token: String,
    pub(super) cache: AugmentCache,
    pub(super) workspace_dir: Option<PathBuf>,
    conversation: Option<augment_ai::BatchConversation>,
    pub(super) pending: Option<AugmentPending>,
    pub(super) error: Option<String>,
    pub(super) queue: VecDeque<(String, Option<String>)>,
    pub(super) done: Vec<&'static str>,
    pub(super) failed: Vec<(&'static str, String)>,
}

pub(super) struct AugmentPending {
    pub(super) rx: Receiver<Result<augment_ai::Outcome, String>>,
    pub(super) target: &'static str,
    pub(super) started: Instant,
    sessionful: bool,
}

fn target_label(target: &str) -> Option<&'static str> {
    match target {
        "choices" => Some("choices"),
        "notes" => Some("notes"),
        "questions" => Some("questions"),
        "keypoints" => Some("keypoints"),
        "format" => Some("format"),
        "topology" => Some("topology"),
        "icon" => Some("icon"),
        _ => None,
    }
}

fn has_icon(dir: &Path) -> bool {
    crate::workspace::Workspace::load(dir).is_ok_and(|ws| ws.icon.is_some())
}

fn gap_items(target: &str, cards: &[Card], cache: &AugmentCache) -> Option<Vec<augment::WarmItem>> {
    match target {
        "choices" => Some(cache.missing_choices(cards)),
        "notes" => Some(cache.missing_notes(cards)),
        "questions" => Some(cache.missing_questions(cards)),
        "keypoints" => Some(cache.missing_keypoints(cards)),
        "format" => Some(cache.missing_format(cards)),
        "topology" => Some(cards.iter().map(augment::WarmItem::from_card).collect()),
        _ => None,
    }
}

impl Augmenting {
    pub(super) fn open(
        deck: String,
        cards: Vec<Card>,
        deck_tokens: Vec<String>,
        cache_path: PathBuf,
        workspace_dir: Option<PathBuf>,
    ) -> Self {
        let deck_ids = cards.iter().filter_map(Card::id).collect();
        // deliberate: a workspace topology is owner-tagged with the FIRST
        // member's token for stability
        let primary_token = deck_tokens.first().cloned().unwrap_or_default();
        let deck_tokens = deck_tokens.into_iter().collect();
        Self {
            deck,
            cards,
            deck_ids,
            deck_tokens,
            primary_token,
            cache: AugmentCache::open(cache_path),
            workspace_dir,
            conversation: None,
            pending: None,
            error: None,
            queue: VecDeque::new(),
            done: Vec::new(),
            failed: Vec::new(),
        }
    }

    pub(super) fn dto(&self) -> AugmentDto {
        let s = self.cache.summarize(&self.cards, &self.deck_tokens);
        let busy = self.pending.as_ref().map(|p| p.target);
        let card_row =
            |kind: &'static str, label: &'static str, c: augment::Coverage| AugmentRowDto {
                kind,
                label,
                covered: c.covered,
                eligible: c.eligible,
                items: Vec::new(),
                busy: busy == Some(kind),
            };
        let mut rows = vec![
            card_row("choices", "Choices", s.choices),
            card_row("notes", "Notes", s.notes),
            card_row("questions", "Questions", s.questions),
            card_row("keypoints", "Key points", s.keypoints),
            card_row("format", "Formatting", s.format),
            AugmentRowDto {
                kind: "topology",
                label: "Topology",
                covered: 0,
                eligible: 0,
                items: s.topologies,
                busy: busy == Some("topology"),
            },
        ];
        if let Some(dir) = &self.workspace_dir {
            rows.push(AugmentRowDto {
                kind: "icon",
                label: "Icon",
                covered: has_icon(dir) as usize,
                eligible: 1,
                items: Vec::new(),
                busy: busy == Some("icon"),
            });
        }
        AugmentDto {
            deck: self.deck.clone(),
            cards: self.cards.len(),
            rows,
            busy,
            elapsed: self.pending.as_ref().map(|p| p.started.elapsed().as_secs()),
            error: self.error.clone(),
            queued: self
                .queue
                .iter()
                .filter_map(|(t, _)| target_label(t))
                .collect(),
            done: self.done.clone(),
            failed: self
                .failed
                .iter()
                .map(|(t, e)| FailedTargetDto {
                    target: t,
                    error: e.clone(),
                })
                .collect(),
        }
    }

    pub(super) fn generate_batch(
        &mut self,
        targets: Vec<(String, Option<String>)>,
        ai: &AiConfig,
        ask: &AskConfig,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.error = None;
        self.done.clear();
        self.failed.clear();
        self.queue = targets.into_iter().collect();
        self.conversation = None;
        let gap_sets: Vec<Vec<augment::WarmItem>> = self
            .queue
            .iter()
            .filter_map(|(target, _)| gap_items(target, &self.cards, &self.cache))
            .filter(|items| !items.is_empty())
            .collect();
        if gap_sets.len() >= 2 {
            let mut seen = HashSet::new();
            let roster: Vec<augment::WarmItem> = gap_sets
                .into_iter()
                .flatten()
                .filter(|item| seen.insert(item.id.clone()))
                .collect();
            let cfg = augment_ai::run_config(ai, ask);
            self.conversation = augment_ai::BatchConversation::new(&cfg, roster);
        }
        self.start_next(ai, ask)
    }

    fn start_next(&mut self, ai: &AiConfig, ask: &AskConfig) -> bool {
        while let Some((target, guidance)) = self.queue.pop_front() {
            let (job, tgt): (augment_ai::Job, &'static str) = match target.as_str() {
                "choices" => {
                    let items = gap_items("choices", &self.cards, &self.cache).unwrap_or_default();
                    if items.is_empty() {
                        self.done.push("choices");
                        continue;
                    }
                    (
                        augment_ai::Job::Choices {
                            items,
                            count: ai.distractor_count,
                        },
                        "choices",
                    )
                }
                "notes" => {
                    let items = gap_items("notes", &self.cards, &self.cache).unwrap_or_default();
                    if items.is_empty() {
                        self.done.push("notes");
                        continue;
                    }
                    (augment_ai::Job::Notes { items }, "notes")
                }
                "questions" => {
                    let items =
                        gap_items("questions", &self.cards, &self.cache).unwrap_or_default();
                    if items.is_empty() {
                        self.done.push("questions");
                        continue;
                    }
                    (
                        augment_ai::Job::Questions {
                            items,
                            count: ai.variant_count,
                        },
                        "questions",
                    )
                }
                "keypoints" => {
                    let items =
                        gap_items("keypoints", &self.cards, &self.cache).unwrap_or_default();
                    if items.is_empty() {
                        self.done.push("keypoints");
                        continue;
                    }
                    (
                        augment_ai::Job::Keypoints {
                            items,
                            count: ai.keypoint_count,
                        },
                        "keypoints",
                    )
                }
                "format" => {
                    let items = gap_items("format", &self.cards, &self.cache).unwrap_or_default();
                    if items.is_empty() {
                        self.done.push("format");
                        continue;
                    }
                    (augment_ai::Job::Format { items }, "format")
                }
                // Topology always adds a new one (named by its guidance); no gap notion.
                "topology" => (
                    augment_ai::Job::Topology {
                        items: self
                            .cards
                            .iter()
                            .map(augment::WarmItem::from_card)
                            .collect(),
                        deck_token: self.primary_token.clone(),
                    },
                    "topology",
                ),
                // icon regenerates unconditionally: a fresh draw replaces the old emblem
                "icon" => match &self.workspace_dir {
                    Some(dir) => (augment_ai::Job::Icon { dir: dir.clone() }, "icon"),
                    None => continue,
                },
                _ => continue,
            };
            // the icon draw never rides the batch conversation (card-free)
            let sessionful = tgt != "icon" && self.conversation.is_some();
            let conversation = if sessionful {
                self.conversation.clone()
            } else {
                None
            };
            let rx =
                augment_ai::spawn(job, guidance, augment_ai::run_config(ai, ask), conversation);
            self.pending = Some(AugmentPending {
                rx,
                target: tgt,
                started: Instant::now(),
                sessionful,
            });
            return true;
        }
        false
    }

    pub(super) fn poll(&mut self, ai: &AiConfig, ask: &AskConfig) {
        let Some(p) = self.pending.as_ref() else {
            return;
        };
        let outcome = match p.rx.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("the augment helper exited unexpectedly".to_string())
            }
        };
        let target = p.target;
        let sessionful = p.sessionful;
        self.pending = None;
        match outcome {
            Ok(o) => {
                if sessionful && let Some(conversation) = self.conversation.as_mut() {
                    conversation.session.started = true;
                }
                self.apply(o);
                self.save();
                self.done.push(target);
            }
            Err(e) => {
                // never resume a session in an unknown state: start fresh
                // and re-prime the roster
                if sessionful && let Some(conversation) = self.conversation.as_mut() {
                    conversation.reset();
                }
                self.failed.push((target, e));
            }
        }
        self.start_next(ai, ask);
    }

    // writes to the cache; does not save
    fn apply(&mut self, outcome: augment_ai::Outcome) {
        match outcome {
            augment_ai::Outcome::Choices(map) => {
                for (id, v) in map {
                    self.cache.set_distractors(&id, v);
                }
            }
            augment_ai::Outcome::Notes(map) => {
                for (id, v) in map {
                    self.cache.set_note(&id, v);
                }
            }
            augment_ai::Outcome::Questions(map) => {
                for (id, v) in map {
                    self.cache.set_variants(&id, v);
                }
            }
            augment_ai::Outcome::Keypoints(map) => {
                for (id, v) in map {
                    self.cache.set_keypoints(&id, v);
                }
            }
            augment_ai::Outcome::Topology(t) => self.cache.add_topology(t),
            augment_ai::Outcome::Format(map) => {
                for (id, v) in map {
                    self.cache.set_format(&id, v);
                }
            }
            // nothing to cache: the file on disk is the result
            augment_ai::Outcome::Icon(_) => {}
        }
    }

    pub(super) fn remove(&mut self, target: &str, topology: Option<&str>) -> bool {
        match target {
            "choices" => self.cache.clear_distractors(&self.deck_ids),
            "notes" => self.cache.clear_notes(&self.deck_ids),
            "questions" => self.cache.clear_variants(&self.deck_ids),
            "keypoints" => self.cache.clear_keypoints(&self.deck_ids),
            "format" => self.cache.clear_format(&self.deck_ids),
            "topology" => {
                let Some(name) = topology else {
                    return false;
                };
                self.cache.remove_topology(name, &self.deck_tokens);
            }
            "all" => self.cache.clear_all(&self.deck_ids, &self.deck_tokens),
            _ => return false,
        }
        self.error = None;
        self.save();
        true
    }

    fn save(&mut self) {
        if let Err(e) = self.cache.save() {
            self.error = Some(format!("could not save augmentations: {e}"));
        }
    }
}

pub(super) struct Generating {
    pub(super) rx: Receiver<Result<String, String>>,
    pub(super) url: String,
    pub(super) dest: PathBuf,
    pub(super) started: Instant,
    pub(super) outcome: Option<Result<(String, usize), String>>,
}

impl Generating {
    pub(super) fn dto(&self) -> GenerateDto {
        match &self.outcome {
            None => GenerateDto {
                phase: "generating",
                deck: None,
                cards: None,
                elapsed: Some(self.started.elapsed().as_secs()),
                error: None,
            },
            Some(Ok((deck, cards))) => GenerateDto {
                phase: "done",
                deck: Some(deck.clone()),
                cards: Some(*cards),
                elapsed: Some(self.started.elapsed().as_secs()),
                error: None,
            },
            Some(Err(e)) => GenerateDto {
                phase: "error",
                deck: None,
                cards: None,
                elapsed: Some(self.started.elapsed().as_secs()),
                error: Some(e.clone()),
            },
        }
    }

    // lenient like the CLI: a parse problem still saves the file, reported
    // as the error
    pub(super) fn poll(&mut self) {
        if self.outcome.is_some() {
            return;
        }
        let text = match self.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.outcome = Some(Err("the generate worker exited unexpectedly".to_string()));
                return;
            }
        };
        self.outcome = Some(text.and_then(|t| {
            let name = generate::deck_name(&self.url);
            match crate::library::place_deck(&self.dest, &name, &t) {
                Ok(p) => {
                    let deck = p
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    match p.parse_error {
                        None => Ok((deck, p.cards)),
                        Some(e) => Err(format!("saved {deck}, but it does not parse yet: {e}")),
                    }
                }
                Err(e) => Err(format!("{e:#}")),
            }
        }));
    }
}

pub(super) struct Sharing {
    pub(super) job: share::ShareJob,
    pub(super) _stage: tempfile::TempDir,
    pub(super) code: Option<String>,
    pub(super) started: Instant,
    pub(super) outcome: Option<Result<(), String>>,
}

impl Sharing {
    pub(super) fn poll(&mut self) {
        while let Ok(ev) = self.job.events.try_recv() {
            match ev {
                share::ShareEvent::Code(c) => self.code = Some(c),
                share::ShareEvent::Done => self.outcome = Some(Ok(())),
                share::ShareEvent::Error(e) => self.outcome = Some(Err(e)),
            }
        }
    }

    pub(super) fn dto(&self) -> ShareDto {
        let elapsed = Some(self.started.elapsed().as_secs());
        match (&self.outcome, &self.code) {
            (Some(Err(e)), _) => ShareDto {
                phase: "error",
                code: self.code.clone(),
                elapsed,
                error: Some(e.clone()),
            },
            (Some(Ok(())), _) => ShareDto {
                phase: "sent",
                code: self.code.clone(),
                elapsed,
                error: None,
            },
            (None, Some(_)) => ShareDto {
                phase: "code",
                code: self.code.clone(),
                elapsed,
                error: None,
            },
            (None, None) => ShareDto {
                phase: "staging",
                code: None,
                elapsed,
                error: None,
            },
        }
    }
}

pub(super) fn stage_for_share(path: &Path, tmp: &tempfile::TempDir) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if !crate::workspace::has_decks(path) {
        bail!("no decks in `{}` — nothing to share", path.display());
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shared-decks");
    let stage = tmp.path().join(name);
    share::stage_dir(path, &stage)?;
    Ok(stage)
}

pub(super) struct Receiving {
    pub(super) job: share::ShareJob,
    pub(super) tmp: tempfile::TempDir,
    pub(super) dest: PathBuf,
    pub(super) started: Instant,
    pub(super) outcome: Option<Result<(String, Vec<String>), String>>,
}

impl Receiving {
    pub(super) fn poll(&mut self) {
        if self.outcome.is_some() {
            return;
        }
        while let Ok(ev) = self.job.events.try_recv() {
            match ev {
                share::ShareEvent::Code(_) => {} // receive never emits one
                share::ShareEvent::Done => {
                    // check-then-act is safe only because handlers are
                    // serialized behind the state lock; never add threads here
                    self.outcome = Some(
                        share::land_received(self.tmp.path(), &self.dest)
                            .map_err(|e| format!("{e:#}")),
                    );
                }
                share::ShareEvent::Error(e) => self.outcome = Some(Err(e)),
            }
        }
    }

    pub(super) fn dto(&self) -> ReceiveDto {
        let elapsed = Some(self.started.elapsed().as_secs());
        match &self.outcome {
            None => ReceiveDto {
                phase: "receiving",
                landed: None,
                stripped: Vec::new(),
                elapsed,
                error: None,
            },
            Some(Ok((landed, stripped))) => ReceiveDto {
                phase: "done",
                landed: Some(landed.clone()),
                stripped: stripped.clone(),
                elapsed,
                error: None,
            },
            Some(Err(e)) => ReceiveDto {
                phase: "error",
                landed: None,
                stripped: Vec::new(),
                elapsed,
                error: Some(e.clone()),
            },
        }
    }
}

pub(super) struct Walking {
    pub(super) walk: Walk,
    pub(super) grade: Option<AskConfig>,
    pub(super) pending: Option<Receiver<Result<(Delta, String), String>>>,
    pub(super) grade_result: Option<(Delta, String)>,
    pub(super) grade_error: Option<String>,
    pub(super) ask: Ask,
}

impl Walking {
    pub(super) fn new(walk: Walk, grade: Option<AskConfig>) -> Self {
        Walking {
            walk,
            grade,
            pending: None,
            grade_result: None,
            grade_error: None,
            ask: Ask::new(),
        }
    }

    pub(super) fn checkpoint_card(&self) -> Option<Card> {
        let trace = self.walk.trace();
        let cp = self.walk.checkpoint()?;
        let mut note = String::new();
        if let Ok(ex) = trace.excerpt(cp) {
            note.push_str("Source excerpt:\n");
            for (n, line) in &ex.lines {
                note.push_str(&format!("{n}: {line}\n"));
            }
        }
        if let Some(insight) = &cp.note {
            if !note.is_empty() {
                note.push('\n');
            }
            note.push_str(insight);
        }
        Some(Card::plain(
            Arc::from(trace.subject.as_str()),
            cp.prompt.clone(),
            cp.points.clone(),
            (!note.is_empty()).then_some(note),
            cp.line,
        ))
    }

    pub(super) fn start_ask(
        &mut self,
        cfg: &AskConfig,
        audience: Audience,
        question: Option<String>,
    ) -> bool {
        let Some(card) = self.checkpoint_card() else {
            return false;
        };
        let root = cfg
            .source_access
            .then(|| self.walk.trace().origin.clone())
            .flatten();
        let frozen = root.as_ref().and_then(|_| {
            let c = self.walk.checkpoint()?;
            self.walk.trace().frozen_block(c)
        });
        let live_root = root.as_deref().filter(|r| r.exists());
        let action = match question {
            Some(q) => AskAction::Question(q),
            None => AskAction::Condense,
        };
        self.ask.start(
            cfg,
            audience,
            &card,
            &[],
            live_root,
            frozen.as_deref(),
            action,
        )
    }

    pub(super) fn poll_ask(&mut self) -> (Option<String>, Option<String>) {
        self.ask
            .align(self.walk.checkpoint().map(|c| c.card_id.clone()));
        let deck_path = self.walk.trace().deck_path.clone();
        self.ask.poll(|card, notes| {
            crate::deck::append_note(&deck_path, card.line, notes).map_err(|e| e.to_string())
        })
    }

    pub(super) fn ask_dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        self.ask.dto(status, error)
    }

    pub(super) fn start_grade(&mut self) {
        self.clear_grade();
        let Some(ask_cfg) = self.grade.as_ref() else {
            return;
        };
        let Some(checkpoint) = self.walk.checkpoint() else {
            return;
        };
        let prediction = self
            .walk
            .prediction(self.walk.current_index())
            .unwrap_or("")
            .to_string();
        let rx = trace_ai::spawn_grade(checkpoint.clone(), prediction, ask_cfg.clone());
        self.pending = Some(rx);
    }

    pub(super) fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok(Ok((delta, feedback))) => {
                self.grade_result = Some((delta, feedback));
                self.pending = None;
            }
            Ok(Err(e)) => {
                self.grade_error = Some(e);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.grade_error = Some("the grading thread ended unexpectedly".to_string());
                self.pending = None;
            }
        }
    }

    pub(super) fn clear_grade(&mut self) {
        self.pending = None;
        self.grade_result = None;
        self.grade_error = None;
    }
}

pub(super) struct Browsing {
    pub(super) cards: Vec<Card>,
    pub(super) label: String,
    pub(super) images: HashMap<String, PathBuf>,
}

impl Browsing {
    pub(super) fn new(build: CardsBuild) -> Self {
        let images = collect_images(&build.cards);
        Self {
            cards: build.cards,
            label: build.label,
            images,
        }
    }
}
