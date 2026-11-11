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
    catalog_owner::CatalogHandle,
    dto::*,
};
use crate::{
    ask::{self, CliSession, Exchange, Reply},
    assemble::{AssembleConfig, tutor_deck},
    augment::{self, AugmentCache},
    augment_ai,
    card::Card,
    cite::CitationRoots,
    config::{AiConfig, AskConfig, Audience, GenerateDeckConfig},
    exam, generate, math, parser,
    session::{Session, now_ms},
    share,
    trace::{self, TraceSession},
    walk::WalkSession,
};

pub(super) fn can_fetch_url_sources(cfg: &AskConfig) -> bool {
    cfg.allowed_tools.iter().any(|tool| tool == "WebFetch")
        && crate::backend::ensure_source_reachable(cfg, true).is_ok()
}

struct TutorInfo {
    links: Vec<String>,
    sources: ask::TutorSources,
    root: Option<PathBuf>,
    source_base: Option<CitationRoots>,
}

struct TutorContextData {
    links: Vec<String>,
    sources: ask::TutorSources,
    root: Option<PathBuf>,
    frozen: Option<String>,
}

impl TutorContextData {
    fn context(&self) -> ask::TutorContext<'_> {
        ask::TutorContext {
            links: &self.links,
            sources: &self.sources,
            root: self.root.as_deref(),
            frozen: self.frozen.as_deref(),
        }
    }
}

fn tutor_context_for(card: &Card, info: TutorInfo, cfg: &AskConfig) -> (TutorContextData, bool) {
    let frozen = info.source_base.as_ref().and_then(|base| {
        let blocks = card
            .citations
            .iter()
            .filter_map(|citation| trace::frozen_excerpt_block(citation, base))
            .collect::<Vec<_>>();
        (!blocks.is_empty()).then(|| blocks.join("\n\n"))
    });
    let root = info
        .root
        .filter(|path| path.exists())
        .filter(|_| !matches!(info.sources, ask::TutorSources::Browse { .. }));
    let has_url_source = info.links.iter().any(|link| crate::deck::is_url(link));
    let has_frozen_sources = matches!(
        info.sources,
        ask::TutorSources::Browse { .. } | ask::TutorSources::Pasted(_)
    );
    let has_source_context =
        root.is_some() || has_frozen_sources || (has_url_source && can_fetch_url_sources(cfg));
    (
        TutorContextData {
            links: info.links,
            sources: info.sources,
            root,
            frozen,
        },
        has_source_context,
    )
}

fn deck_tutor_sources(
    card: &Card,
    info: &crate::session::DeckInfo,
    cfg: &AskConfig,
) -> ask::TutorSources {
    let topic_id = info.topic_ids.get(card.topic).cloned().flatten();
    match info.deck_token.as_deref() {
        Some(deck_id) => ask::tutor_sources_at(card, topic_id.as_deref(), &info.path, deck_id, cfg),
        None => ask::TutorSources::None,
    }
}

pub(super) struct Reviewing {
    pub(super) session: Session,
    pub(super) label: String,
    pub(super) load_warnings: Vec<String>,
    pub(super) topic_ids: HashMap<String, Vec<Option<String>>>,
    pub(super) files: DeckFiles,
    pub(super) images: HashMap<String, PathBuf>,
    pub(super) ask: Ask,
    pub(super) links: HashMap<String, Vec<String>>,
    pub(super) browsable: std::collections::HashSet<String>,
    pub(super) source_bases: HashMap<String, CitationRoots>,
    pub(super) augment: AugmentCache,
    pub(super) present_seq: u64,
    pub(super) original_fronts: HashMap<String, String>,
    pub(super) topology_name: Option<String>,
    pub(super) region_name: Option<String>,
}

pub(super) struct Pending {
    pub(super) rx: Receiver<Reply>,
    pub(super) purpose: Purpose,
    pub(super) card: Card,
    pub(super) subject: Option<String>,
    pub(super) job: ask::AskJob,
}

// A dropped pending exchange is ineligible (a question or draft left behind
// by a card advance, tutor replaced, owner shutdown): reap its subprocess
// synchronously so no AI child ever outlives its owner or keeps burning quota
// for an answer nobody applies.
impl Drop for Pending {
    fn drop(&mut self) {
        self.job.cancel();
    }
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
    transcript_units: Vec<Vec<crate::render::ContentUnit>>,
    pub(super) noted_through: usize,
    pub(super) subject: Option<String>,
    pub(super) pending: Option<Pending>,
    pub(super) draft: Option<ask::DraftCard>,
    pub(super) context_warning: Option<String>,
    visible_line: Option<String>,
    notice: ask::CardOnlyNotice,
    session_paste: Option<Vec<crate::sources::SourceText>>,
}

impl Ask {
    fn new() -> Self {
        Self {
            cli: CliSession::new(),
            transcript: Vec::new(),
            transcript_units: Vec::new(),
            noted_through: 0,
            subject: None,
            pending: None,
            draft: None,
            context_warning: None,
            visible_line: None,
            notice: ask::CardOnlyNotice::default(),
            session_paste: None,
        }
    }

    fn align(&mut self, subject: Option<String>) {
        if self.subject != subject {
            self.transcript.clear();
            self.transcript_units.clear();
            self.noted_through = 0;
            self.draft = None;
            self.context_warning = None;
            self.notice = ask::CardOnlyNotice::default();
            // A note still condensing settles on the card it was asked on
            // (ADR 0056); a late answer or draft is dropped (ADR 0027).
            self.pending = self
                .pending
                .take()
                .filter(|p| matches!(p.purpose, Purpose::Condense));
            self.subject = subject;
        }
    }

    fn previous_card(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| p.subject != self.subject)
    }

    fn dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        AskDto {
            transcript: self
                .transcript
                .iter()
                .enumerate()
                .map(|(index, (q, a))| ExchangeDto {
                    q: q.clone(),
                    a: a.clone(),
                    units: self
                        .transcript_units
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| crate::render::tutor_answer_units(a)),
                })
                .collect(),
            thinking: self.pending.is_some(),
            previous_card: self.previous_card(),
            can_distill: self.can_distill(),
            status: status.or_else(|| self.context_warning.clone()),
            error,
            draft: self.draft.as_ref().map(|d| DraftCardDto {
                front: d.front.clone(),
                back: d.back.clone(),
            }),
        }
    }

    fn can_distill(&self) -> bool {
        self.pending.is_none() && self.transcript.len() > self.noted_through
    }

    // `subject` is the same key the owner's poll realigns with; deriving it
    // from the card here bit once (a trace checkpoint card carries no id, so
    // start aligned on None while poll aligned on the checkpoint id, and the
    // mismatch dropped every pending trace answer).
    #[expect(
        clippy::too_many_arguments,
        reason = "typed tutor inputs keep the owner transition explicit"
    )]
    fn start(
        &mut self,
        cfg: &AskConfig,
        audience: Audience,
        card: &Card,
        subject: Option<String>,
        context: &ask::TutorContext,
        has_source_context: bool,
        action: AskAction,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.align(subject.clone());
        if matches!(action, AskAction::Condense | AskAction::DraftCard) && !self.can_distill() {
            return false;
        }
        self.context_warning = (context.frozen.is_some() && !has_source_context)
            .then(|| ask::FROZEN_ONLY_WARNING.to_string());
        self.visible_line = context.sources.visible_line().map(str::to_string);
        if let (ask::TutorSources::Pasted(texts), AskAction::Question(_)) =
            (context.sources, &action)
        {
            if self.cli.started && self.session_paste.as_ref() != Some(texts) {
                self.cli = CliSession::new();
            }
            self.session_paste = Some(texts.clone());
        }
        let run_cfg = ask::tutor_run_config(cfg, context);
        let args = self.cli.args_in(run_cfg.cwd.as_deref());
        let keeps_session = crate::backend::backend_for(&run_cfg)
            .map(|b| b.supports_session())
            .unwrap_or(false);
        let (prompt, purpose) = match action {
            AskAction::Question(q) => {
                let prompt = if keeps_session {
                    ask::question_prompt(card, audience, context, &q, !self.cli.started)
                } else {
                    ask::question_prompt_with_history(card, audience, context, &self.transcript, &q)
                };
                (prompt, Purpose::Question(q))
            }
            AskAction::Condense => (
                ask::condense_prompt(card, &self.transcript[self.noted_through..]),
                Purpose::Condense,
            ),
            AskAction::DraftCard => (
                ask::draft_card_prompt(card, &self.transcript),
                Purpose::DraftCard,
            ),
        };
        let (rx, job) = ask::spawn(run_cfg, prompt, args);
        self.pending = Some(Pending {
            rx,
            purpose,
            card: card.clone(),
            subject,
            job,
        });
        true
    }

    // `save` receives the subject the exchange was started on: the id the
    // note is addressed to, which a trace checkpoint's card does not carry.
    fn poll(
        &mut self,
        save: impl FnOnce(&Card, Option<&str>, &[String]) -> Result<(), String>,
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
        let mut pending = self.pending.take().expect("pending was present");
        let purpose = std::mem::replace(&mut pending.purpose, Purpose::Condense);
        let pending_card = pending.card.clone();
        let pending_subject = pending.subject.take();
        let on_current = pending_subject == self.subject;
        drop(pending);
        match (reply, purpose) {
            (Reply::Answer(answer), Purpose::Question(question)) => {
                self.cli.started = true;
                let line = self.notice.deliver(self.visible_line.take());
                let answer = ask::with_visible_line(line.as_deref(), answer);
                self.transcript_units
                    .push(crate::render::tutor_answer_units(&answer));
                self.transcript.push((question, answer));
                (None, None)
            }
            (Reply::Answer(text), Purpose::Condense) => {
                self.cli.started = true;
                let notes = ask::extract_note_lines(&text);
                if notes.is_empty() {
                    return (Some("nothing to save".to_string()), None);
                }
                match save(&pending_card, pending_subject.as_deref(), &notes) {
                    Ok(()) => {
                        if on_current {
                            self.noted_through = self.transcript.len();
                        }
                        (Some("note saved".to_string()), None)
                    }
                    Err(e) => (None, Some(e)),
                }
            }
            (Reply::Answer(text), Purpose::DraftCard) => {
                self.cli.started = true;
                match ask::parse_drafted_card(&text) {
                    Ok(card) => {
                        self.draft = Some(card);
                        self.noted_through = self.transcript.len();
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
        Vec::new(),
        0,
    );
    if let Some(at) = &c.at
        && let Ok(fields) = crate::locator::parse_locator_comment(&format!("<!-- at: {at} -->"))
    {
        card.citations.push(crate::card::SourceCitation {
            locator: fields.locator,
            fingerprint: None,
            asset: None,
            line: 0,
        });
    }
    card
}

pub(super) enum RemoteCardResolution {
    Grounded {
        card: Card,
        info: Box<crate::session::DeckInfo>,
    },
    CardOnly(Card),
}

fn remote_resolution_failure(
    card: &RemoteCard,
    cause: impl std::fmt::Display,
) -> RemoteCardResolution {
    eprintln!(
        "error: remote tutor could not resolve deck `{}` card `{}`: {cause}; using request card fields only",
        card.deck_id, card.card_id
    );
    RemoteCardResolution::CardOnly(remote_card(card))
}

pub(super) fn resolve_remote_card(
    catalog: &CatalogHandle,
    cfg: &AssembleConfig,
    requested: &RemoteCard,
) -> RemoteCardResolution {
    match remote_deck_path(catalog, &requested.deck_id) {
        Ok(path) => resolve_remote_target(&path, cfg, requested),
        Err(cause) => remote_resolution_failure(requested, cause),
    }
}

fn remote_deck_path(catalog: &CatalogHandle, deck_id: &str) -> Result<PathBuf, String> {
    let snapshot = match catalog.sync_snapshot() {
        Some(Ok(snapshot)) => snapshot,
        Some(Err(error)) => return Err(error.to_string()),
        None => return Err("catalog owner is unavailable".to_string()),
    };
    match snapshot.catalog.deck(deck_id) {
        crate::sync::DeckLookup::One(target) => Ok(target.path.clone()),
        crate::sync::DeckLookup::Ambiguous => Err("deck id is ambiguous".to_string()),
        crate::sync::DeckLookup::Missing => Err("deck id is missing".to_string()),
    }
}

pub(super) enum RemoteCreateOutcome {
    Ok(RemoteCreateCardResp),
    Missing,
    UnknownCard(String),
    Invalid,
    MintFailed,
}

// Sidecar only; the store entry arrives with the phone's progress push.
pub(super) fn remote_create_card(
    catalog: &CatalogHandle,
    cfg: &AssembleConfig,
    req: &RemoteCreateCardReq,
) -> RemoteCreateOutcome {
    let missing = |cause: &dyn std::fmt::Display| {
        eprintln!(
            "error: remote card create could not resolve deck `{}`: {cause}; nothing written",
            req.deck_id
        );
        RemoteCreateOutcome::Missing
    };
    let path = match remote_deck_path(catalog, &req.deck_id) {
        Ok(path) => path,
        Err(cause) => return missing(&cause),
    };
    let deck = match tutor_deck(&path, cfg) {
        Ok(deck) => deck,
        Err(error) => return missing(&error),
    };
    let Some(deck_id) = deck.info.deck_token.clone() else {
        return missing(&"the served deck carries no id");
    };
    let topic_id = match named_card_topic(&deck, &req.deck_id, &req.card_id) {
        Ok(topic_id) => topic_id,
        Err(error) => return RemoteCreateOutcome::UnknownCard(error),
    };
    let deck_fingerprints: HashSet<u64> = deck
        .authored_cards()
        .iter()
        .map(|c| c.block_fingerprint)
        .collect();
    match crate::store::mint_or_return_kept_card(
        &path,
        &deck_id,
        &topic_id,
        &req.front,
        &req.back,
        &deck_fingerprints,
    ) {
        Ok(minted) => RemoteCreateOutcome::Ok(RemoteCreateCardResp {
            id: minted.id,
            block: minted.block,
        }),
        Err(
            crate::store::MintError::Duplicate
            | crate::store::MintError::Malformed(_)
            | crate::store::MintError::DuplicatedTopic(_),
        ) => RemoteCreateOutcome::Invalid,
        Err(crate::store::MintError::Mint(_) | crate::store::MintError::Deck(_)) => {
            RemoteCreateOutcome::MintFailed
        }
    }
}

fn named_card_topic(
    deck: &crate::assemble::TutorDeck,
    deck_id: &str,
    card_id: &str,
) -> Result<String, String> {
    let Some(card) = deck
        .cards
        .iter()
        .find(|card| card.id().as_deref() == Some(card_id))
    else {
        return Err(format!("card `{card_id}` is not in deck `{deck_id}`"));
    };
    deck.info
        .topic_ids
        .get(card.topic)
        .cloned()
        .flatten()
        .ok_or_else(|| {
            format!(
                "card `{card_id}` sits in a topic with no topic id; run `alix deck init` to mint it"
            )
        })
}

fn resolve_remote_target(
    path: &Path,
    cfg: &AssembleConfig,
    requested: &RemoteCard,
) -> RemoteCardResolution {
    let deck = match tutor_deck(path, cfg) {
        Ok(deck) => deck,
        Err(error) => return remote_resolution_failure(requested, error),
    };
    let Some(card) = deck
        .cards
        .into_iter()
        .find(|card| card.id().as_deref() == Some(requested.card_id.as_str()))
    else {
        return remote_resolution_failure(requested, "card id is missing from the deck");
    };
    RemoteCardResolution::Grounded {
        card,
        info: Box::new(deck.info),
    }
}

impl RemoteCardResolution {
    fn into_card(self) -> (Card, bool) {
        match self {
            Self::Grounded { card, .. } => (card, false),
            Self::CardOnly(card) => (card, true),
        }
    }
}

enum RemoteAskPurpose {
    Question,
    Draft,
    Note,
}

enum RemoteAskOutcome {
    Answer {
        text: String,
        units: Vec<crate::render::ContentUnit>,
    },
    Draft(ask::DraftCard),
    // an empty vec is a settled success, not an error
    Note(Vec<String>),
    Error(String),
}

struct NoteTarget {
    deck_path: PathBuf,
    deck_id: String,
    card_id: String,
}

pub(super) struct RemoteAsk {
    rx: Receiver<Reply>,
    purpose: RemoteAskPurpose,
    card_only: bool,
    note_target: Option<NoteTarget>,
    status: Option<String>,
    visible_line: Option<String>,
    started_ms: u64,
    outcome: Option<RemoteAskOutcome>,
    job: ask::AskJob,
}

impl Drop for RemoteAsk {
    fn drop(&mut self) {
        self.job.cancel();
    }
}

impl RemoteAsk {
    pub(super) fn ask(
        cfg: &AskConfig,
        audience: Audience,
        resolution: RemoteCardResolution,
        history: Vec<RemoteTurn>,
        question: &str,
    ) -> Self {
        let prior: Vec<Exchange> = history.into_iter().map(|t| (t.q, t.a)).collect();
        let (card, context_data, has_source_context, card_only) = match resolution {
            RemoteCardResolution::Grounded { card, info } => {
                let info = *info;
                let root = info.browse_root(&card);
                let sources = deck_tutor_sources(&card, &info, cfg);
                let tutor_info = TutorInfo {
                    links: info.links,
                    sources,
                    root,
                    source_base: Some(info.source_base),
                };
                let (context, has_source_context) = tutor_context_for(&card, tutor_info, cfg);
                (card, context, has_source_context, false)
            }
            RemoteCardResolution::CardOnly(card) => (
                card,
                TutorContextData {
                    links: Vec::new(),
                    sources: ask::TutorSources::None,
                    root: None,
                    frozen: None,
                },
                false,
                true,
            ),
        };
        let context = context_data.context();
        let status = (context.frozen.is_some() && !has_source_context)
            .then(|| ask::FROZEN_ONLY_WARNING.to_string());
        let run_cfg = ask::tutor_run_config(cfg, &context);
        let visible_line = context.sources.visible_line().map(str::to_string);
        let prompt = ask::question_prompt_with_history(&card, audience, &context, &prior, question);
        let mut remote = Self::spawn(
            &run_cfg,
            prompt,
            RemoteAskPurpose::Question,
            card_only,
            None,
            status,
        );
        remote.visible_line = visible_line;
        remote
    }

    pub(super) fn draft(
        cfg: &AskConfig,
        resolution: RemoteCardResolution,
        history: Vec<RemoteTurn>,
    ) -> Self {
        let (card, card_only) = resolution.into_card();
        let prior: Vec<Exchange> = history.into_iter().map(|t| (t.q, t.a)).collect();
        let prompt = ask::draft_card_prompt(&card, &prior);
        Self::spawn(cfg, prompt, RemoteAskPurpose::Draft, card_only, None, None)
    }

    pub(super) fn note(
        cfg: &AskConfig,
        resolution: RemoteCardResolution,
        history: Vec<RemoteTurn>,
    ) -> Self {
        let (card, card_only, note_target) = match resolution {
            RemoteCardResolution::Grounded { card, info } => {
                let note_target = card.id().map(|card_id| NoteTarget {
                    deck_path: info.path.clone(),
                    deck_id: card.deck_id.to_string(),
                    card_id,
                });
                (card, false, note_target)
            }
            RemoteCardResolution::CardOnly(card) => (card, true, None),
        };
        let prior: Vec<Exchange> = history.into_iter().map(|t| (t.q, t.a)).collect();
        let prompt = ask::condense_prompt(&card, &prior);
        Self::spawn(
            cfg,
            prompt,
            RemoteAskPurpose::Note,
            card_only,
            note_target,
            None,
        )
    }

    fn spawn(
        cfg: &AskConfig,
        prompt: String,
        purpose: RemoteAskPurpose,
        card_only: bool,
        note_target: Option<NoteTarget>,
        status: Option<String>,
    ) -> Self {
        let (rx, job) = ask::spawn(cfg.clone(), prompt, Vec::new());
        Self {
            rx,
            purpose,
            card_only,
            note_target,
            status,
            visible_line: None,
            started_ms: now_ms(),
            outcome: None,
            job,
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
            (Reply::Answer(text), RemoteAskPurpose::Question) => {
                let text = ask::with_visible_line(self.visible_line.as_deref(), text);
                RemoteAskOutcome::Answer {
                    units: crate::render::tutor_answer_units(&text),
                    text,
                }
            }
            (Reply::Answer(text), RemoteAskPurpose::Draft) => {
                match ask::parse_drafted_card(&text) {
                    Ok(card) => RemoteAskOutcome::Draft(card),
                    Err(e) => RemoteAskOutcome::Error(e.to_string()),
                }
            }
            (Reply::Answer(text), RemoteAskPurpose::Note) => {
                let notes = ask::extract_note_lines(&text);
                let written = self.note_target.as_ref().map(|target| {
                    crate::personal::append_note(
                        &target.deck_path,
                        &target.deck_id,
                        &target.card_id,
                        &notes,
                    )
                });
                match written {
                    Some(Err(e)) => RemoteAskOutcome::Error(e.to_string()),
                    _ => RemoteAskOutcome::Note(notes),
                }
            }
            (Reply::Error(e), _) => RemoteAskOutcome::Error(e),
        });
    }

    pub(super) fn dto(&self) -> RemoteAskDto {
        match &self.outcome {
            None => RemoteAskDto {
                thinking: true,
                card_only: self.card_only,
                status: self.status.clone(),
                answer: None,
                units: Vec::new(),
                draft: None,
                note: None,
                error: None,
                elapsed: Some(now_ms().saturating_sub(self.started_ms) / 1000),
            },
            Some(RemoteAskOutcome::Answer { text, units }) => RemoteAskDto {
                thinking: false,
                card_only: self.card_only,
                status: self.status.clone(),
                answer: Some(text.clone()),
                units: units.clone(),
                draft: None,
                note: None,
                error: None,
                elapsed: None,
            },
            Some(RemoteAskOutcome::Draft(d)) => RemoteAskDto {
                thinking: false,
                card_only: self.card_only,
                status: self.status.clone(),
                answer: None,
                units: Vec::new(),
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
                card_only: self.card_only,
                status: self.status.clone(),
                answer: None,
                units: Vec::new(),
                draft: None,
                note: Some(lines.clone()),
                error: None,
                elapsed: None,
            },
            Some(RemoteAskOutcome::Error(e)) => RemoteAskDto {
                thinking: false,
                card_only: self.card_only,
                status: self.status.clone(),
                answer: None,
                units: Vec::new(),
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
            load_warnings: build.load_warnings,
            topic_ids: build.topic_ids,
            files: DeckFiles::new(build.decks),
            images,
            ask: Ask::new(),
            links: build.links,
            browsable: build.browsable,
            source_bases: build.source_bases,
            augment: build.augment,
            present_seq: now_ms(),
            original_fronts: HashMap::new(),
            topology_name: build.topology_name,
            region_name: build.region_name,
        }
    }

    pub(super) fn rotate_variant(&mut self) {
        let Some((id, fingerprint)) = self
            .session
            .current()
            .and_then(|c| c.id().map(|id| (id, c.content_fingerprint)))
        else {
            return;
        };
        if self.augment.variants(&id, fingerprint).is_none() {
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
        if let Some(chosen) = self.augment.pick_front(&id, &original, seed, fingerprint)
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
        let source_base = self.source_bases.get(&*card.deck_id).cloned();
        let root = source_base
            .as_ref()
            .filter(|_| self.browsable.contains(&*card.deck_id))
            .and_then(|base| base.browse_root(card.citations.iter().map(|c| &c.locator)));
        let topic_id = self
            .topic_ids
            .get(&*card.deck_id)
            .and_then(|ids| ids.get(card.topic))
            .cloned()
            .flatten();
        let sources = match self.files.path(&card.deck_id) {
            Some(path) => {
                ask::tutor_sources_at(&card, topic_id.as_deref(), path, &card.deck_id, cfg)
            }
            None => ask::TutorSources::None,
        };
        let info = TutorInfo {
            links: self.links.get(&*card.deck_id).cloned().unwrap_or_default(),
            sources,
            root,
            source_base,
        };
        let (context_data, has_source_context) = tutor_context_for(&card, info, cfg);
        let context = context_data.context();
        let subject = card.id();
        self.ask.start(
            cfg,
            audience,
            &card,
            subject,
            &context,
            has_source_context,
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
        ask.poll(|card, _, notes| {
            let card_id = card
                .id()
                .ok_or_else(|| "the card carries no id to attach a note to".to_string())?;
            files.append_note(&card.deck_id, &card_id, notes)?;
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
    pub(super) prepared: Option<PreparedFiles>,
}

pub(super) struct PreparedFiles {
    pub(super) ideas_path: PathBuf,
    pub(super) prepared_path: PathBuf,
    pub(super) prepared: exam::Prepared,
    pub(super) settled: bool,
}

impl Examining {
    pub(super) fn save_answers(&mut self) {
        if let Some(files) = self.prepared.as_mut() {
            files.prepared.answers = self.sitting.answers().to_vec();
            if let Err(error) = files.prepared.save(&files.prepared_path) {
                eprintln!("saving exam answers: {error:#}");
            }
        }
    }

    /// Once graded: the idea history is kept and the prepared exam is used up.
    pub(super) fn settle(&mut self) {
        let Some(files) = self.prepared.as_mut() else {
            return;
        };
        if files.settled || self.sitting.result().is_none() {
            return;
        }
        if let Some(ideas) = self.sitting.ideas()
            && let Err(error) = ideas.save(&files.ideas_path)
        {
            eprintln!("saving exam ideas: {error:#}");
            return;
        }
        match std::fs::remove_file(&files.prepared_path) {
            Ok(()) => files.settled = true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => files.settled = true,
            Err(error) => eprintln!("removing the used exam: {error}"),
        }
    }
}

pub(super) struct Preparing {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) rx: std::sync::mpsc::Receiver<Result<(), String>>,
    pub(super) job: crate::ask::AskJob,
    pub(super) started_ms: u64,
    pub(super) estimate: Option<&'static str>,
    pub(super) outcome: Option<Result<(), String>>,
}

impl Drop for Preparing {
    fn drop(&mut self) {
        if self.outcome.is_none() {
            self.job.cancel();
        }
    }
}

impl Preparing {
    pub(super) fn dto(&mut self) -> PreparingDto {
        if self.outcome.is_none() {
            match self.rx.try_recv() {
                Ok(outcome) => self.outcome = Some(outcome),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.outcome = Some(Err("the exam helper exited unexpectedly".to_string()));
                }
            }
        }
        PreparingDto {
            deck: self.name.clone(),
            state: match &self.outcome {
                None => "generating",
                Some(Ok(())) => "ready",
                Some(Err(_)) => "failed",
            },
            elapsed: now_ms().saturating_sub(self.started_ms) / 1000,
            estimate: self.outcome.is_none().then_some(self.estimate).flatten(),
            error: self.outcome.as_ref().and_then(|o| o.clone().err()),
        }
    }
}

pub(super) struct RemoteExamining {
    pub(super) id: String,
    pub(super) deck: String,
    pub(super) sitting: exam::Sitting,
    pub(super) estimate: Option<&'static str>,
    pub(super) cards_by_topic: Vec<exam::TopicCards>,
    pub(super) unknown_cards: Vec<String>,
}

impl RemoteExamining {
    pub(super) fn advance(&mut self) {
        match self.sitting.advance(now_ms()) {
            Some(exam::Effect::RemediationCards(by_topic)) => self.cards_by_topic = by_topic,
            Some(exam::Effect::Graded) | None => {}
        }
    }

    pub(super) fn dto(&self) -> RemoteExamDto {
        let s = &self.sitting;
        let result = s.result();
        let parts = exam_result_parts(s);
        let grades = exam_grades(s, &parts);
        RemoteExamDto {
            sitting: Some(self.id.clone()),
            phase: exam_phase_name(s.phase()),
            deck: self.deck.clone(),
            strictness: strictness_name(s.strictness()),
            // prompts only: the rubric never leaves the server outside a
            // graded result
            questions: s.questions().iter().map(|q| q.prompt.clone()).collect(),
            questions_units: s
                .questions()
                .iter()
                .map(|q| crate::render::exam_text_units(&q.prompt))
                .collect(),
            grades,
            topics: parts.topics,
            verdicts: parts.verdicts,
            full: s.full(),
            mastered: parts.mastered,
            unknown_cards: self.unknown_cards.clone(),
            gaps: s.gaps(),
            can_remediate: s.can_remediate(),
            cards_by_topic: self
                .cards_by_topic
                .iter()
                .map(|topic| TopicCardsDto {
                    topic_id: topic.topic_id.clone(),
                    cards: topic.cards.clone(),
                })
                .collect(),
            thinking: s.thinking(),
            elapsed: s.elapsed_secs(),
            error: s.error().map(str::to_string),
            estimate: (*s.phase() == exam::Phase::Generating)
                .then_some(self.estimate)
                .flatten(),
            ideas: result.and(s.ideas()).cloned(),
        }
    }
}

pub(super) fn remote_exam_idle_dto() -> RemoteExamDto {
    RemoteExamDto {
        sitting: None,
        phase: "idle",
        deck: String::new(),
        strictness: "balanced",
        questions: Vec::new(),
        questions_units: Vec::new(),
        grades: Vec::new(),
        topics: Vec::new(),
        verdicts: None,
        full: false,
        mastered: false,
        unknown_cards: Vec::new(),
        gaps: Vec::new(),
        can_remediate: false,
        cards_by_topic: Vec::new(),
        thinking: false,
        elapsed: None,
        error: None,
        estimate: None,
        ideas: None,
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
        cache: AugmentCache,
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
            cache,
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
        let fp_by_id: HashMap<String, u64> = self
            .cards
            .iter()
            .filter_map(|c| c.id().map(|id| (id, c.content_fingerprint)))
            .collect();
        let format_fp_by_id: HashMap<String, u64> = self
            .cards
            .iter()
            .filter_map(|c| c.id().map(|id| (id, c.format_fingerprint())))
            .collect();
        match outcome {
            augment_ai::Outcome::Choices(map) => {
                for (id, v) in map {
                    if let Some(&fingerprint) = fp_by_id.get(&id) {
                        self.cache.set_distractors(&id, v, fingerprint);
                    }
                }
            }
            augment_ai::Outcome::Notes(map) => {
                for (id, v) in map {
                    if let Some(&fingerprint) = fp_by_id.get(&id) {
                        self.cache.set_note(&id, v, fingerprint);
                    }
                }
            }
            augment_ai::Outcome::Questions(map) => {
                for (id, v) in map {
                    if let Some(&fingerprint) = fp_by_id.get(&id) {
                        self.cache.set_variants(&id, v, fingerprint);
                    }
                }
            }
            augment_ai::Outcome::Keypoints(map) => {
                for (id, v) in map {
                    if let Some(&fingerprint) = fp_by_id.get(&id) {
                        self.cache.set_keypoints(&id, v, fingerprint);
                    }
                }
            }
            augment_ai::Outcome::Topology(t) => self.cache.add_topology(t),
            augment_ai::Outcome::Format(map) => {
                for (id, v) in map {
                    if let Some(&fingerprint) = format_fp_by_id.get(&id) {
                        self.cache.set_format(&id, v, fingerprint);
                    }
                }
            }
            // nothing to cache: the workspace file is the result, installed
            // here so the write happens on the owner, never on the worker
            augment_ai::Outcome::Icon { dir, svg } => {
                if let Err(e) = crate::icon::install_svg(&dir, &svg) {
                    self.error = Some(format!("could not install the workspace icon: {e:#}"));
                }
            }
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
            if let Ok(deck) = parser::parse(&name, &t) {
                math::validate_generated(&deck.cards).map_err(|diagnostic| {
                    format!("generated deck `{name}` has invalid LaTeX math: {diagnostic}")
                })?;
            }
            match crate::library::place_deck(&self.dest, &name, &t) {
                Ok(p) => {
                    if Path::new(&self.url).exists() {
                        crate::source::stamp_citations(&p.path).map_err(|error| {
                            format!(
                                "saved {}, but could not fingerprint its source citations: \
                                 {error:#}",
                                p.path.display()
                            )
                        })?;
                    }
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
    pub(super) sources_omitted: usize,
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
        let sources_omitted = self.sources_omitted;
        match (&self.outcome, &self.code) {
            (Some(Err(e)), _) => ShareDto {
                phase: "error",
                code: self.code.clone(),
                elapsed,
                error: Some(e.clone()),
                sources_omitted,
            },
            (Some(Ok(())), _) => ShareDto {
                phase: "sent",
                code: self.code.clone(),
                elapsed,
                error: None,
                sources_omitted,
            },
            (None, Some(_)) => ShareDto {
                phase: "code",
                code: self.code.clone(),
                elapsed,
                error: None,
                sources_omitted,
            },
            (None, None) => ShareDto {
                phase: "staging",
                code: None,
                elapsed,
                error: None,
                sources_omitted,
            },
        }
    }
}

pub(super) fn stage_for_share(
    path: &Path,
    tmp: &tempfile::TempDir,
    sources: share::SourcesPolicy,
) -> Result<share::Staged> {
    if !path.is_file() && !crate::workspace::has_decks(path) {
        bail!("no decks in `{}`, nothing to share", path.display());
    }
    share::stage_path(path, tmp.path(), sources)
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

pub(super) struct Tracing {
    pub(super) session: TraceSession,
    pub(super) ask: Ask,
}

impl Tracing {
    pub(super) fn new(session: TraceSession) -> Self {
        Tracing {
            session,
            ask: Ask::new(),
        }
    }

    pub(super) fn restart(&mut self) {
        self.session.restart();
        self.ask = Ask::new();
    }

    pub(super) fn checkpoint_card(&self) -> Option<Card> {
        let trace = self.session.trace();
        let cp = self.session.checkpoint()?;
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
            Vec::from_iter((!note.is_empty()).then(|| crate::card::Note::plain(note))),
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
        let checkpoint = self.session.checkpoint();
        let root = checkpoint
            .filter(|_| cfg.source_access)
            .and_then(|checkpoint| self.session.trace().browse_root(checkpoint));
        let frozen =
            checkpoint.and_then(|checkpoint| self.session.trace().frozen_block(checkpoint));
        let live_root = root.as_deref().filter(|path| path.exists());
        let has_url_source = self
            .session
            .trace()
            .links
            .iter()
            .any(|link| crate::deck::is_url(link));
        let has_source_context =
            live_root.is_some() || (has_url_source && can_fetch_url_sources(cfg));
        let action = match question {
            Some(q) => AskAction::Question(q),
            None => AskAction::Condense,
        };
        let context = ask::TutorContext {
            links: &self.session.trace().links,
            sources: &ask::TutorSources::None,
            root: live_root,
            frozen: frozen.as_deref(),
        };
        let subject = self.session.checkpoint().map(|c| c.card_id.clone());
        self.ask.start(
            cfg,
            audience,
            &card,
            subject,
            &context,
            has_source_context,
            action,
        )
    }

    pub(super) fn poll_ask(&mut self) -> (Option<String>, Option<String>) {
        self.ask
            .align(self.session.checkpoint().map(|c| c.card_id.clone()));
        let deck_path = self.session.trace().deck_path.clone();
        // A checkpoint card is built for the trace and carries no token of its
        // own; the subject is the checkpoint id the note is addressed to.
        self.ask.poll(|card, checkpoint, notes| {
            let card_id = checkpoint
                .ok_or_else(|| "the trace is on no checkpoint to attach a note to".to_string())?;
            crate::personal::append_note(&deck_path, &card.deck_id, card_id, notes)
                .map_err(|e| e.to_string())
        })
    }

    pub(super) fn ask_dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        self.ask.dto(status, error)
    }
}

pub(super) struct Walking {
    pub(super) session: WalkSession,
    pub(super) label: String,
    pub(super) images: HashMap<String, PathBuf>,
    pub(super) info: crate::session::DeckInfo,
    pub(super) ask: Ask,
}

impl Walking {
    pub(super) fn new(build: crate::assemble::WalkBuild) -> Self {
        let images = collect_images(build.session.cards());
        Self {
            session: build.session,
            label: build.label,
            images,
            info: build.info,
            ask: Ask::new(),
        }
    }

    pub(super) fn restart(&mut self, store: &crate::store::Store) {
        self.session.restart(store, now_ms());
        self.ask = Ask::new();
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
        let info = TutorInfo {
            links: self.info.links.clone(),
            sources: deck_tutor_sources(&card, &self.info, cfg),
            root: self.info.browse_root(&card),
            source_base: Some(self.info.source_base.clone()),
        };
        let (context_data, has_source_context) = tutor_context_for(&card, info, cfg);
        let context = context_data.context();
        self.ask.start(
            cfg,
            audience,
            &card,
            card.id(),
            &context,
            has_source_context,
            action,
        )
    }

    pub(super) fn poll_ask(&mut self) -> (Option<String>, Option<String>) {
        self.ask.align(self.session.current().and_then(Card::id));
        let Self {
            ask, session, info, ..
        } = self;
        ask.poll(|card, _, notes| {
            let card_id = card
                .id()
                .ok_or_else(|| "the card carries no id to attach a note to".to_string())?;
            crate::personal::append_note(&info.path, &card.deck_id, &card_id, notes)
                .map_err(|e| e.to_string())?;
            if let Some(current) = session.current_mut()
                && current.id() == card.id()
            {
                current.append_note(notes);
            }
            Ok(())
        })
    }

    pub(super) fn ask_dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        self.ask.dto(status, error)
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn frozen_deck_sources_count_as_source_context_for_the_tutor_warning() {
        let dir = tempfile::tempdir().unwrap();
        let card = augmentation_card();
        let cli = dir.path().join("no-cli");
        let cfg = crate::testutil::ask_config(&cli);
        let text = crate::sources::SourceText {
            display: "ownership.rs".to_string(),
            text: "fn main() {}".to_string(),
        };
        for (label, sources, expected) in [
            ("none", ask::TutorSources::None, false),
            (
                "card only",
                ask::TutorSources::CardOnly {
                    line: "answering from the card alone".to_string(),
                },
                false,
            ),
            ("pasted", ask::TutorSources::Pasted(vec![text]), true),
            (
                "browse",
                ask::TutorSources::Browse {
                    dir: dir.path().to_path_buf(),
                    listing: Vec::new(),
                    focus: Vec::new(),
                },
                true,
            ),
        ] {
            let info = TutorInfo {
                links: Vec::new(),
                sources,
                root: None,
                source_base: None,
            };
            let (_, has_source_context) = tutor_context_for(&card, info, &cfg);
            assert_eq!(
                expected, has_source_context,
                "{label}: frozen deck sources decide whether the excerpts-only warning shows"
            );
        }
    }

    fn augmentation_card() -> Card {
        let mut card = Card::plain(
            std::sync::Arc::from("deck.md"),
            "question".to_string(),
            vec!["answer".to_string()],
            Vec::new(),
            1,
        );
        card.token = Some(std::sync::Arc::from("card-augmentation"));
        card.deck_id = std::sync::Arc::from("deck-augmentation");
        card
    }

    #[cfg(unix)]
    #[test]
    fn a_kept_session_gets_each_topics_pasted_sources_once_across_a_topic_switch() {
        use crate::sources::testkit::{TOPIC_A, TOPIC_B, Workspace};

        let _lock = crate::testutil::exec_lock();
        let outer = tempfile::tempdir().unwrap();
        let unnameable = outer.path().join("p(aren)");
        std::fs::create_dir(&unnameable).unwrap();
        let w = Workspace::new_in(&unnameable);
        w.write_live("a.rs", b"fn alpha_bytes() {}\n");
        w.write_live("b.rs", b"fn beta_bytes() {}\n");
        w.add(&["a.rs"], &[TOPIC_A]).unwrap();
        w.add(&["b.rs"], &[TOPIC_B]).unwrap();
        let deck = crate::deck::Deck::load(w.deck()).unwrap();
        let topic_ids = deck.topic_ids();
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(dir.path(), &["answer"]);
        let cfg = AskConfig {
            backend: crate::config::BackendKind::Claude,
            ..crate::testutil::ask_config(&cli)
        };
        let sources = deck.sources().unwrap().unwrap();
        let mut ask = Ask::new();

        for (turn, front, subject, pasted, absent) in [
            (1, "q1", "a", Some("alpha_bytes"), "beta_bytes"),
            (2, "q1", "a", None, "alpha_bytes"),
            (3, "q3", "b", Some("beta_bytes"), "alpha_bytes"),
            (4, "q3", "b", None, "beta_bytes"),
        ] {
            let card = deck.cards.iter().find(|c| c.front == front).unwrap();
            let topic = topic_ids[card.topic].clone();
            let tutor = ask::tutor_sources_for(card, topic.as_deref(), Some(&sources), &cfg);
            assert!(
                matches!(tutor, ask::TutorSources::Pasted(_)),
                "turn {turn}: precondition, claude on an unnameable folder pastes: {tutor:?}"
            );
            let context = ask::TutorContext {
                links: &[],
                sources: &tutor,
                root: None,
                frozen: None,
            };
            assert!(
                ask.start(
                    &cfg,
                    Audience::Adult,
                    card,
                    Some(subject.to_string()),
                    &context,
                    false,
                    AskAction::Question("why?".to_string()),
                ),
                "turn {turn}: the ask starts"
            );
            let deadline = Instant::now() + std::time::Duration::from_secs(10);
            while ask.pending.is_some() && Instant::now() < deadline {
                let (_, error) = ask.poll(|_, _, _| Ok(()));
                assert!(error.is_none(), "turn {turn}: {error:?}");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let sent = std::fs::read_to_string(dir.path().join(format!("prompt-{turn}"))).unwrap();
            if let Some(pasted) = pasted {
                assert_eq!(
                    1,
                    sent.matches(pasted).count(),
                    "turn {turn}: the topic is pasted once: {sent}"
                );
            }
            assert!(
                !sent.contains(absent),
                "turn {turn}: {absent} is not sent: {sent}"
            );
        }
    }

    #[test]
    fn remote_job_elapsed_values_are_seconds_not_raw_milliseconds() {
        let (_ask_tx, ask_rx) = mpsc::channel();
        let ask = RemoteAsk {
            rx: ask_rx,
            purpose: RemoteAskPurpose::Question,
            card_only: false,
            note_target: None,
            status: None,
            visible_line: None,
            started_ms: now_ms().saturating_sub(2_500),
            outcome: None,
            job: ask::AskJob::default(),
        };
        let (_generate_tx, generate_rx) = mpsc::channel();
        let generating = RemoteGenerating {
            rx: generate_rx,
            url: "https://example.org".to_string(),
            started_ms: now_ms().saturating_sub(2_500),
            outcome: None,
        };

        assert_eq!(Some(2), ask.dto().elapsed);
        assert_eq!(Some(2), generating.dto().elapsed);
    }

    #[cfg(unix)]
    #[test]
    fn a_remote_tutor_reports_source_damage_on_every_answer_that_falls_back() {
        use crate::sources::testkit::{TOPIC_A, Workspace};

        let _lock = crate::testutil::exec_lock();
        let w = Workspace::new();
        w.write_live("a.rs", b"fn alpha_bytes() {}\n");
        w.add(&["a.rs"], &[TOPIC_A]).unwrap();
        crate::stamp::stamp_deck(&w.deck()).unwrap();
        let deck = crate::deck::Deck::load(w.deck()).unwrap();
        let card = deck.cards.iter().find(|card| card.front == "q1").unwrap();
        let requested = RemoteCard {
            deck_id: deck.deck_token.clone().unwrap(),
            card_id: card.id().unwrap(),
            subject: "d.md".to_string(),
            front: card.front.clone(),
            back: card.back.clone(),
            at: None,
        };
        let sources = deck.sources().unwrap().unwrap();
        let (source_id, row) = sources.manifest.sources.iter().next().unwrap();
        let display = row.path.parse::<crate::sources::DisplayPath>().unwrap();
        let copy = sources
            .dir
            .join(crate::sources::copy_name(source_id, &display));
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(
            dir.path(),
            &["grounded", "card only", "still card only"],
        );
        let cfg = AssembleConfig {
            review: crate::config::ReviewConfig::default(),
            ask: AskConfig {
                backend: crate::config::BackendKind::Codex,
                ..crate::testutil::ask_config(&cli)
            },
            pacing: crate::assemble::Pacing {
                max_session: 10,
                new_cards_percent: 30,
            },
            instance_store: None,
        };
        let mut history: Vec<RemoteTurn> = Vec::new();
        let mut answers: Vec<String> = Vec::new();

        for (turn, question) in ["why?", "and now?", "and still?"].into_iter().enumerate() {
            if turn == 1 {
                std::fs::remove_file(&copy).unwrap();
            }
            let resolution = resolve_remote_target(&w.deck(), &cfg, &requested);
            let mut remote = RemoteAsk::ask(
                &cfg.ask,
                Audience::Adult,
                resolution,
                history
                    .iter()
                    .map(|t| RemoteTurn {
                        q: t.q.clone(),
                        a: t.a.clone(),
                    })
                    .collect(),
                question,
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while remote.thinking() && std::time::Instant::now() < deadline {
                remote.poll();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let dto = remote.dto();
            assert_eq!(None, dto.error, "turn {turn}");
            history.push(RemoteTurn {
                q: question.to_string(),
                a: dto.answer.unwrap(),
            });
            answers.push(history.last().unwrap().a.clone());
        }

        assert_eq!("grounded", answers[0], "the first answer used the source");
        let damage = "answering from the card alone: this deck's sources cannot be read: ";
        for (turn, reply) in [(1, "card only"), (2, "still card only")] {
            assert!(
                answers[turn].starts_with(damage),
                "turn {turn}: a card-only answer discloses the fallback: {answers:?}"
            );
            assert!(
                answers[turn].ends_with(&format!("\n\n{reply}")),
                "turn {turn}: the backend answer follows the line: {answers:?}"
            );
        }
    }

    #[test]
    fn a_deck_load_error_resolves_the_remote_card_from_request_fields_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.md");
        std::fs::write(&path, "---\nformat-version: [\n---\n").unwrap();
        let requested = RemoteCard {
            deck_id: "deck-broken".to_string(),
            card_id: "card-broken".to_string(),
            subject: "phone.md".to_string(),
            front: "request front".to_string(),
            back: vec!["request back".to_string()],
            at: None,
        };
        let cfg = AssembleConfig {
            review: crate::config::ReviewConfig::default(),
            ask: AskConfig::default(),
            pacing: crate::assemble::Pacing {
                max_session: 10,
                new_cards_percent: 30,
            },
            instance_store: None,
        };

        let RemoteCardResolution::CardOnly(card) = resolve_remote_target(&path, &cfg, &requested)
        else {
            panic!("a deck load error must use only the request card");
        };
        assert_eq!("request front", card.front);
        assert_eq!(["request back"], card.back.as_slice());
    }

    #[test]
    fn a_named_card_in_a_topic_without_an_id_names_the_card_and_the_remedy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noid.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: \"deck-noid\"\n---\n\
             # Unstamped\n\n## qn\nn\n<!-- id: card-qn -->\n",
        )
        .unwrap();
        let cfg = AssembleConfig {
            review: crate::config::ReviewConfig::default(),
            ask: AskConfig::default(),
            pacing: crate::assemble::Pacing {
                max_session: 10,
                new_cards_percent: 30,
            },
            instance_store: None,
        };
        let deck = tutor_deck(&path, &cfg).unwrap();

        assert_eq!(
            Err(
                "card `card-qn` sits in a topic with no topic id; run `alix deck init` to mint it"
                    .to_string()
            ),
            named_card_topic(&deck, "deck-noid", "card-qn"),
        );
        assert_eq!(
            Err("card `card-nope` is not in deck `deck-noid`".to_string()),
            named_card_topic(&deck, "deck-noid", "card-nope"),
        );
    }

    #[test]
    fn every_augmentation_target_has_its_wire_label() {
        for target in [
            "choices",
            "notes",
            "questions",
            "keypoints",
            "format",
            "topology",
            "icon",
        ] {
            assert_eq!(Some(target), target_label(target), "target={target}");
        }
        assert_eq!(None, target_label("unknown"));
    }

    #[test]
    fn every_card_augmentation_target_has_a_gap_projection() {
        let dir = tempfile::tempdir().unwrap();
        let cards = vec![augmentation_card()];
        let cache = AugmentCache::open(dir.path().join("augment.json"));

        for target in [
            "choices",
            "notes",
            "questions",
            "keypoints",
            "format",
            "topology",
        ] {
            assert!(
                gap_items(target, &cards, &cache).is_some(),
                "target={target}"
            );
        }
        assert!(gap_items("icon", &cards, &cache).is_none());
        assert!(gap_items("unknown", &cards, &cache).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn isolated_format_and_topology_targets_start_non_sessionful_jobs() {
        let _lock = crate::testutil::exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_reply(dir.path(), "{}");
        let ask = crate::testutil::ask_config(&cli);
        let ai = AiConfig::default();

        for target in ["format", "topology"] {
            let mut augmenting = Augmenting::open(
                "deck.md".to_string(),
                vec![augmentation_card()],
                vec!["deck-augmentation".to_string()],
                AugmentCache::open(dir.path().join(format!("{target}.json"))),
                None,
            );
            augmenting.queue.push_back((target.to_string(), None));

            assert!(augmenting.start_next(&ai, &ask), "target={target}");
            let pending = augmenting.pending.take().expect("the target started");
            assert_eq!(target, pending.target);
            assert!(!pending.sessionful, "target={target}");
            assert!(
                pending
                    .rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .is_ok(),
                "target={target}"
            );
        }
    }

    #[test]
    fn augmentation_dto_marks_exactly_the_pending_target_busy() {
        let dir = tempfile::tempdir().unwrap();
        for target in [
            "choices",
            "notes",
            "questions",
            "keypoints",
            "format",
            "topology",
            "icon",
        ] {
            let mut augmenting = Augmenting::open(
                "deck.md".to_string(),
                Vec::new(),
                Vec::new(),
                AugmentCache::open(dir.path().join(format!("{target}.json"))),
                Some(dir.path().to_path_buf()),
            );
            let (_tx, rx) = mpsc::channel();
            augmenting.pending = Some(AugmentPending {
                rx,
                target,
                started: Instant::now(),
                sessionful: false,
            });

            let busy: Vec<_> = augmenting
                .dto()
                .rows
                .into_iter()
                .filter(|row| row.busy)
                .map(|row| row.kind)
                .collect();
            assert_eq!(vec![target], busy, "target={target}");
        }
    }

    #[test]
    fn every_removable_augmentation_target_is_an_accepted_noop_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut augmenting = Augmenting::open(
            "deck.md".to_string(),
            Vec::new(),
            Vec::new(),
            AugmentCache::open(dir.path().join("augment.json")),
            None,
        );

        for target in [
            "choices",
            "notes",
            "questions",
            "keypoints",
            "format",
            "topology",
            "all",
        ] {
            let topology = (target == "topology").then_some("map");
            assert!(augmenting.remove(target, topology), "target={target}");
        }
        assert!(!augmenting.remove("topology", None));
        assert!(!augmenting.remove("unknown", None));
    }

    #[test]
    fn share_staging_accepts_decks_and_deck_folders_but_rejects_empty_folders() {
        let root = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir().unwrap();
        let deck = root.path().join("deck.md");
        std::fs::write(&deck, "## question\nanswer\n").unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(
            workspace.join("member.md"),
            "---\nformat-version: 1\nid: deck-member1\n---\n\
             ## member\nanswer\n<!-- id: card-member1 -->\n",
        )
        .unwrap();
        let empty = root.path().join("empty");
        std::fs::create_dir(&empty).unwrap();

        let omit = share::SourcesPolicy::Omit;
        assert!(stage_for_share(&deck, &stage, omit).is_ok());
        assert!(stage_for_share(&workspace, &stage, omit).is_ok());
        assert!(stage_for_share(&empty, &stage, omit).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn sharing_poll_drains_code_and_completion_events() {
        let (tx, rx) = mpsc::channel();
        let mut sharing = Sharing {
            job: share::test_job(rx),
            _stage: tempfile::tempdir().unwrap(),
            code: None,
            started: Instant::now(),
            outcome: None,
            sources_omitted: 0,
        };
        tx.send(share::ShareEvent::Code("7-alpha-bravo".to_string()))
            .unwrap();
        tx.send(share::ShareEvent::Done).unwrap();

        sharing.poll();

        let dto = sharing.dto();
        assert_eq!("sent", dto.phase);
        assert_eq!(Some("7-alpha-bravo".to_string()), dto.code);
        assert!(dto.error.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn receiving_poll_lands_the_completed_transfer() {
        let root = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dest = root.path().join("decks");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(tmp.path().join("received.md"), "## received\ncard\n").unwrap();
        let (tx, rx) = mpsc::channel();
        let mut receiving = Receiving {
            job: share::test_job(rx),
            tmp,
            dest: dest.clone(),
            started: Instant::now(),
            outcome: None,
        };
        tx.send(share::ShareEvent::Done).unwrap();

        receiving.poll();

        let dto = receiving.dto();
        assert_eq!("done", dto.phase);
        assert_eq!(Some("received.md".to_string()), dto.landed);
        assert_eq!(
            "## received\ncard\n",
            std::fs::read_to_string(dest.join("received.md")).unwrap()
        );
    }
}
