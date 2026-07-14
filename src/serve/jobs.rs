//! The stateful screens/jobs a connection can own: review, ask-Claude, the AI
//! exam, deck augmentation, deck generation, a wormhole send/receive, a trace
//! walk, and browse — one `Option<…>` local each in the server's dispatch
//! loop, built when its screen opens and dropped when it closes.
//! Each type only polls a background worker's channel for a finished result;
//! the thread itself is spawned by the lib crate it calls (`ask::spawn`,
//! `augment_ai::spawn`, …), never by these types.

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
    config::{AiConfig, AskConfig, Audience},
    exam, generate,
    session::{Session, now_ms},
    share,
    trace::{self, Delta, SourceBase, Walk},
    trace_ai,
};

/// The server's live review state once decks are chosen. Its absence (`None`)
/// means the page is in the deck-selection phase.
pub(super) struct Reviewing {
    pub(super) session: Session,
    pub(super) label: String,
    pub(super) files: DeckFiles,
    pub(super) images: HashMap<String, PathBuf>,
    /// Ask-Claude tutor (the CLI conversation, transcript and in-flight call),
    /// shared with the trace walk; the per-subject `% link:` links and source
    /// roots that ground it stay here (they're keyed by deck subject).
    pub(super) ask: Ask,
    pub(super) links: HashMap<String, Vec<String>>,
    /// Subject → `% source:` project root, for the grounded tutor (opt-in).
    pub(super) source_roots: HashMap<String, PathBuf>,
    /// Subject → source base, for resolving a card's `% at:` citation excerpt.
    pub(super) source_bases: HashMap<String, SourceBase>,
    /// AI distractors for choice cards, read when building a choice question
    /// (generated ahead of time by `alix deck augment`; empty → offline).
    pub(super) augment: AugmentCache,
    /// A per-presentation counter (seeded from the clock) that rotates a reworded
    /// question variant in each time a card is shown (`--target questions`).
    pub(super) present_seq: u64,
    /// The authored front of each card we've rotated a variant into, so the
    /// original phrasing stays in the rotation (generated variants drop it).
    pub(super) original_fronts: HashMap<u64, String>,
    /// The resolved topology name when this session is topology-ordered (used to
    /// fetch the topology from `augment` for the orientation breadcrumb); `None`
    /// otherwise.
    pub(super) topology_name: Option<String>,
}

/// An in-flight ask-Claude call: the channel the background thread answers on,
/// what it is for, and the card it is about (snapshotted so a late reply still
/// refers to the right card).
pub(super) struct Pending {
    pub(super) rx: Receiver<Reply>,
    pub(super) purpose: Purpose,
    pub(super) card: Card,
}

/// What a pending CLI call will do with its answer.
pub(super) enum Purpose {
    /// A question; holds the text to record in the transcript on success.
    Question(String),
    /// Condense the conversation into note lines appended to the deck file.
    Condense,
    /// Distill the conversation into one draft card, surfaced on the ask DTO.
    DraftCard,
}

/// What a review-tutor call should do with its turn.
pub(super) enum AskAction {
    /// Answer a question; the text is recorded in the transcript on success.
    Question(String),
    /// Condense the conversation into note lines appended to the deck.
    Condense,
    /// Distill the conversation into one draft card, surfaced on the ask DTO.
    DraftCard,
}

/// The ask-Claude tutor's state, shared by a review session and a trace walk: the
/// CLI conversation spanning the session, the running transcript shown for the
/// current subject (a review card or a walk checkpoint), and any in-flight call.
/// It is agnostic to *what* is being studied — the consumer supplies the subject
/// [`Card`], its `% link:` links and source root per call, and (on a "save note"
/// condense) writes the resulting note where the subject lives.
pub(super) struct Ask {
    pub(super) cli: CliSession,
    pub(super) transcript: Vec<Exchange>,
    /// The subject id the displayed transcript belongs to; cleared when the
    /// subject changes (the CLI conversation itself still spans the whole
    /// session, so Claude keeps the full context).
    pub(super) subject: Option<u64>,
    pub(super) pending: Option<Pending>,
    /// The last card drafted from the conversation (`AskAction::DraftCard`),
    /// surfaced on the ask DTO until the subject changes.
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

    /// Drops the displayed transcript when the subject (card/checkpoint) changes,
    /// so the ask view shows only the current subject's exchanges.
    fn align(&mut self, subject: Option<u64>) {
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

    /// Starts a call about `card`: a question, a condense-into-note, or a
    /// draft-a-card distillation (`action`). `links`/`root` ground the tutor
    /// exactly as a review does. Returns `false` (no-op) if a call is already
    /// pending, or a condense/draft has nothing to work from.
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
        // A new subject starts a fresh visible transcript (and a subject-scoped
        // condense/draft), even though the CLI conversation continues.
        self.align(Some(card.id()));
        // nothing to condense or draft without a transcript
        if matches!(action, AskAction::Condense | AskAction::DraftCard)
            && self.transcript.is_empty()
        {
            return false;
        }
        let run_cfg = match root {
            Some(r) => ask::with_source_root(cfg, r),
            None => cfg.clone(),
        };
        // Reconcile the session with this call's cwd *before* building the prompt:
        // a cwd change starts a fresh conversation, so `started` then reports this
        // as a first message (full subject context).
        let args = self.cli.args_in(run_cfg.cwd.as_deref());
        // Claude keeps the running conversation via `--resume`, so its follow-up
        // prompt stays short. A backend without a session (Task 7) runs each turn
        // statelessly, so re-inline the prior transcript to restore memory.
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

    /// Completes a question with `answer` right away, without spawning the
    /// backend or ever creating a [`Pending`] — used when serve already knows,
    /// at prompt-build time, that the model would only be asked to echo
    /// [`ask::SOURCE_NOT_FOUND`] verbatim (`{#source-not-found-reply}`: a frozen
    /// card whose live source root can't be resolved). Applies the same
    /// subject-alignment and transcript push as [`Ask::poll`]'s
    /// `(Reply::Answer, Purpose::Question)` arm, so the resulting `AskDto` is
    /// indistinguishable from a real reply — except it's already there,
    /// `thinking: false`, on the very next read. Unlike `poll`, it leaves
    /// `self.cli` untouched: no real turn happened, so a later real question
    /// still starts/resumes the CLI session correctly. Returns `false` (no-op)
    /// if a call is already pending.
    fn answer_immediately(&mut self, card: &Card, question: String, answer: String) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.align(Some(card.id()));
        // `self.cli` is deliberately untouched: no real turn happened, so a
        // later question must still start/resume the CLI session correctly.
        self.transcript.push((question, answer));
        true
    }

    /// Drains a finished reply: a question lands in the transcript; a condense's
    /// note lines are handed to `save` (the consumer writes them where the subject
    /// lives — a deck card, or a trace checkpoint). Returns a one-shot
    /// `(status, error)` to show once.
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
                self.cli.started = true; // later calls --resume this conversation
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
            // Don't resume a session in an unknown state; the next question starts
            // a fresh one.
            (Reply::Error(e), _) => {
                self.cli = CliSession::new();
                (None, Some(e))
            }
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
            // The real cache is opened by `open_augment` once the active store
            // path is known; until then an empty cache (offline only).
            augment: AugmentCache::open(Path::new("")),
            present_seq: now_ms(),
            original_fronts: HashMap::new(),
            topology_name: build.topology_name,
        }
    }

    /// Opens the distractor cache co-located with the active `store_path` (the
    /// active store changes per selection). Distractors are generated ahead of
    /// time by `alix deck augment`; review only reads them.
    pub(super) fn open_augment(&mut self, store_path: &Path) {
        self.augment = AugmentCache::open(augment::augment_path_for(store_path));
    }

    /// Rotates the current card's question through the pool of its authored front
    /// plus any cached variants (`alix deck augment --target questions`), a fresh
    /// phrasing each time a card is presented. The answer is unchanged, so
    /// identity (which ignores the front) is untouched. Called on card advance.
    pub(super) fn rotate_variant(&mut self) {
        let Some(id) = self.session.current().map(|c| c.id()) else {
            return;
        };
        if self.augment.variants(id).is_none() {
            return;
        }
        // Capture the authored front the first time, before we overwrite it, so
        // it stays in the rotation alongside the generated variants.
        if !self.original_fronts.contains_key(&id)
            && let Some(card) = self.session.current()
        {
            self.original_fronts.insert(id, card.front.clone());
        }
        let original = self.original_fronts.get(&id).cloned().unwrap_or_default();
        let seed = self.present_seq;
        self.present_seq = self.present_seq.wrapping_add(1);
        if let Some(chosen) = self.augment.pick_front(id, &original, seed)
            && let Some(card) = self.session.current_mut()
        {
            card.front = chosen;
        }
    }

    /// Drops the displayed transcript when the current card changed, so the ask
    /// view shows only this card's exchanges. The CLI session (`cli`) is
    /// untouched, so Claude still has the whole conversation as context.
    pub(super) fn align_transcript(&mut self) {
        self.ask.align(self.session.current().map(|c| c.id()));
    }

    /// The ask-view payload, with an optional one-shot status/error.
    pub(super) fn ask_dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        self.ask.dto(status, error)
    }

    /// Starts an ask-Claude call about the current card: a question, a
    /// condense-into-note, or a draft-a-card distillation (`action`). Returns
    /// `false` (no-op) if a call is already pending, nothing is reviewable, or
    /// there is nothing to condense/draft. Grounds the tutor in the card's deck
    /// source when that deck opted into `[ask] source_access` (`source_roots`).
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
        // The grounded source root (opt-in via `source_access`): a per-card
        // `% origin:` override, else the deck/workspace root.
        let root = self.source_roots.get(&*card.subject).map(|deck_root| {
            card.origin
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| deck_root.clone())
        });
        // A frozen card inlines its snapshot excerpt as the anchor; the live
        // source is read for context. A recorded-but-missing source → the canned
        // "couldn't find" reply (no cwd handed to the subprocess).
        let frozen = root.as_ref().and_then(|_| {
            let at = card.at.as_deref()?;
            let base = self.source_bases.get(&*card.subject)?;
            trace::frozen_excerpt_block(at, card.at_origin.as_deref(), base)
        });
        let live_root = root.as_deref().filter(|r| r.exists());
        // `{#source-not-found-reply}`: this is exactly the condition under which
        // `ask::question_context`'s `(Some(excerpt), None)` arm would tell the
        // model to reply `SOURCE_NOT_FOUND` verbatim — a round trip that spends
        // real latency (and a chance of the model paraphrasing) to echo a
        // constant. Answer it here instead: deterministic wording, zero cost.
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

    /// Drains a finished CLI reply into the transcript (a question) or the deck
    /// file (a "save note" condense). Returns a transient `(status, error)`.
    pub(super) fn poll_ask(&mut self) -> (Option<String>, Option<String>) {
        // Opening the ask view on a new card (the page polls `/api/ask`) drops
        // the previous card's exchanges from the display.
        self.align_transcript();
        // Field-split the borrow so the save closure can touch `files`/`session`
        // while `ask` drives the poll.
        let Self {
            ask,
            files,
            session,
            ..
        } = self;
        ask.poll(|card, notes| {
            files.append_note(&card.subject, card.line, notes)?;
            // Mirror the note onto the in-memory card so returning to it shows the
            // note at once, without re-reading the deck.
            if let Some(cur) = session.current_mut()
                && cur.id() == card.id()
            {
                cur.append_note(notes);
            }
            Ok(())
        })
    }
}

/// The server's live AI-exam state: one in-progress [`exam::Sitting`] plus the
/// path of the deck under exam (to resolve what a pass unlocks).
pub(super) struct Examining {
    pub(super) sitting: exam::Sitting,
    pub(super) deck_path: PathBuf,
}

/// The server's live deck-augmentation state: one deck's augmentation cache and
/// any in-flight generation. Opened from the picker's Augment screen
/// (`/api/augment/open`), it reports coverage, fills gaps, and removes — all
/// scoped to this deck, since the cache may be shared by other decks on the same
/// store. The single in-flight `Job` runs on a background thread (`augment_ai::spawn`)
/// while the page polls `GET /api/augment`.
pub(super) struct Augmenting {
    /// Display name (a workspace member's qualified `<ws>/<file>`, or a deck file).
    pub(super) deck: String,
    pub(super) cards: Vec<Card>,
    /// This deck's card ids, for scoping removals against the shared cache.
    pub(super) deck_ids: HashSet<u64>,
    pub(super) cache: AugmentCache,
    /// `Some(dir)` when this screen was opened on a workspace: targets run
    /// across the union of member cards, and workspace-specific targets (the
    /// icon) become available. `None` for a plain deck.
    pub(super) workspace_dir: Option<PathBuf>,
    pub(super) pending: Option<AugmentPending>,
    /// The last generation/save error, shown until the next action clears it.
    pub(super) error: Option<String>,
    /// Targets still to start in the current batch, in request order, each
    /// paired with its own `--with` steer.
    pub(super) queue: VecDeque<(String, Option<String>)>,
    /// Targets the current batch has finished successfully.
    pub(super) done: Vec<&'static str>,
    /// Targets the current batch attempted and failed, with their error.
    pub(super) failed: Vec<(&'static str, String)>,
}

/// An augmentation generation in flight: the channel the worker delivers on, the
/// target it's filling (for the "busy" row), and when it started (for elapsed).
pub(super) struct AugmentPending {
    pub(super) rx: Receiver<Result<augment_ai::Outcome, String>>,
    pub(super) target: &'static str,
    pub(super) started: Instant,
}

/// Maps a queued target string to its canonical `&'static str` token, so the
/// DTO's `queued` list carries the same static tokens as `rows`/`done`/`failed`
/// rather than the owned `String`s the queue holds.
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

/// Whether the workspace at `dir` has a resolved icon (a manifest `icon` or
/// the conventional `assets/icon.*`) — the icon row's 0/1 coverage.
fn has_icon(dir: &Path) -> bool {
    crate::workspace::Workspace::load(dir).is_ok_and(|ws| ws.icon.is_some())
}

impl Augmenting {
    /// Opens the Augment screen: for a deck, `cards` are its own and
    /// `workspace_dir` is `None`; for a workspace, `cards` are the union of
    /// every member's and `workspace_dir` names its root (which also unlocks
    /// the icon target). The augmentation cache lives beside the store either
    /// way.
    pub(super) fn open(
        deck: String,
        cards: Vec<Card>,
        cache_path: PathBuf,
        workspace_dir: Option<PathBuf>,
    ) -> Self {
        let deck_ids = cards.iter().map(Card::id).collect();
        Self {
            deck,
            cards,
            deck_ids,
            cache: AugmentCache::open(cache_path),
            workspace_dir,
            pending: None,
            error: None,
            queue: VecDeque::new(),
            done: Vec::new(),
            failed: Vec::new(),
        }
    }

    /// Builds the screen payload from the current cache coverage + any in-flight job.
    pub(super) fn dto(&self) -> AugmentDto {
        let s = self.cache.summarize(&self.cards);
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
        // Workspace mode only: the icon target, 0/1-covered by whether a
        // conventional assets/icon.* exists.
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

    /// Starts a batch: `targets` run one at a time in order, each carrying its
    /// own `--with` steer, a per-target failure recorded in `failed` without
    /// aborting the rest. No-op (returns `false`) while a generation is
    /// already in flight. Returns whether a job started (a batch of only
    /// no-gap/unknown targets drains without starting one).
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
        self.start_next(ai, ask)
    }

    /// Pops targets off the queue until one spawns a job. A target with no gap
    /// to fill (or an unrecognized one) is recorded as done in passing and
    /// skipped, without spawning anything. Returns whether a job started.
    fn start_next(&mut self, ai: &AiConfig, ask: &AskConfig) -> bool {
        while let Some((target, guidance)) = self.queue.pop_front() {
            let (job, tgt): (augment_ai::Job, &'static str) = match target.as_str() {
                "choices" => {
                    let items = self.cache.missing_choices(&self.cards);
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
                    let items = self.cache.missing_notes(&self.cards);
                    if items.is_empty() {
                        self.done.push("notes");
                        continue;
                    }
                    (augment_ai::Job::Notes { items }, "notes")
                }
                "questions" => {
                    let items = self.cache.missing_questions(&self.cards);
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
                    let items = self.cache.missing_keypoints(&self.cards);
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
                    let items = self.cache.missing_format(&self.cards);
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
                    },
                    "topology",
                ),
                // The icon regenerates unconditionally (a fresh draw replaces the
                // old emblem); only a workspace has one to draw.
                "icon" => match &self.workspace_dir {
                    Some(dir) => (augment_ai::Job::Icon { dir: dir.clone() }, "icon"),
                    None => continue,
                },
                // Unknown target: nothing to record, try the next queued one.
                _ => continue,
            };
            let rx = augment_ai::spawn(job, guidance, augment_ai::run_config(ai, ask), None);
            self.pending = Some(AugmentPending {
                rx,
                target: tgt,
                started: Instant::now(),
            });
            return true;
        }
        false
    }

    /// Drains a finished generation: applies its [`Outcome`](augment_ai::Outcome) to
    /// the cache and saves it as done, or records the error in `failed` (a
    /// per-target failure never aborts the batch). Then advances the queue.
    /// A no-op while still running.
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
        self.pending = None;
        match outcome {
            Ok(o) => {
                self.apply(o);
                self.save();
                self.done.push(target);
            }
            Err(e) => self.failed.push((target, e)),
        }
        self.start_next(ai, ask);
    }

    /// Writes a finished outcome into the cache (does not save).
    fn apply(&mut self, outcome: augment_ai::Outcome) {
        match outcome {
            augment_ai::Outcome::Choices(map) => {
                for (id, v) in map {
                    self.cache.set_distractors(id, v);
                }
            }
            augment_ai::Outcome::Notes(map) => {
                for (id, v) in map {
                    self.cache.set_note(id, v);
                }
            }
            augment_ai::Outcome::Questions(map) => {
                for (id, v) in map {
                    self.cache.set_variants(id, v);
                }
            }
            augment_ai::Outcome::Keypoints(map) => {
                for (id, v) in map {
                    self.cache.set_keypoints(id, v);
                }
            }
            augment_ai::Outcome::Topology(t) => self.cache.add_topology(t),
            augment_ai::Outcome::Format(map) => {
                for (id, v) in map {
                    self.cache.set_format(id, v);
                }
            }
            // Nothing to cache: icon::generate already wrote assets/icon.svg,
            // and the dto's coverage reads the file system.
            augment_ai::Outcome::Icon(_) => {}
        }
    }

    /// Removes a target's augmentations for this deck, then saves. `topology`
    /// names the one to drop when `target` is `"topology"`; `"all"` clears
    /// everything this deck owns. Returns whether the request was understood.
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
                self.cache.remove_topology(name, &self.deck_ids);
            }
            "all" => self.cache.clear_all(&self.deck_ids),
            _ => return false,
        }
        self.error = None;
        self.save();
        true
    }

    /// Persists the cache, recording any I/O error for the page to surface.
    fn save(&mut self) {
        if let Err(e) = self.cache.save() {
            self.error = Some(format!("could not save augmentations: {e}"));
        }
    }
}

/// A deck generation in flight (or just finished): the worker channel, what
/// was asked, where the deck lands, and the outcome once placed.
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

    /// Drains a finished worker and places the deck (lenient, like the CLI:
    /// a parse problem still saves the file and is reported as the error).
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

/// A wormhole send in flight: the staged copy (kept alive for the whole
/// transfer), the job, and what it has reported so far.
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

/// Stages a row for sharing into `tmp`: a deck file travels as-is (its
/// augmentations live in the store-side cache and stay home); a folder is
/// copied minus personal state. Returns what to hand to wormhole/zip.
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

/// A wormhole receive in flight: the scratch dir it lands in, where it goes
/// afterwards, and the landing outcome.
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
                    // `land_received`'s collision check is check-then-act;
                    // safe only because this server loop is single-threaded
                    // (one request handled at a time, and `poll()` only ever
                    // runs from inside that loop) — introduce no threads here.
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

// ── Trace walks (in-page, from the picker) ──────────────────────────────────
//
// A single walk of one trace deck: predict → reveal a live excerpt → grade →
// compress. There is no deck-selection screen (one deck, one walk). The
// frontend-agnostic `Walk` state machine carries the logic; this is a thin web
// reader over it. Live Claude grading (`--grade`) is the only async step, so it
// runs on a background thread and the page polls `GET /api/walk` while
// `thinking`, like the exam.

/// The server's live trace-walk state. Holds the [`Walk`], the (optional) live
/// grading config, and the in-flight/just-finished Claude grade for the current
/// reveal.
pub(super) struct Walking {
    pub(super) walk: Walk,
    /// `Some` in `--grade` mode: the `[ask]` config a background grade uses
    /// (grading runs at the tutor tier, not trace's heavy build defaults).
    pub(super) grade: Option<AskConfig>,
    /// A background Claude grade in flight for the current reveal.
    pub(super) pending: Option<Receiver<Result<(Delta, String), String>>>,
    /// The resolved Claude grade for the current reveal (verdict + feedback).
    pub(super) grade_result: Option<(Delta, String)>,
    /// A failed Claude grade — the reveal falls back to self-grading.
    pub(super) grade_error: Option<String>,
    /// Ask-Claude tutor for the current checkpoint — the same machinery a review
    /// uses, its subject the checkpoint instead of a card.
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

    /// The current checkpoint as a tutor [`Card`]: front = the predict prompt,
    /// back = the key points, note = the live source excerpt + the connecting
    /// insight. Its `id()` matches the checkpoint's `card_id` (both hash subject +
    /// back), so the transcript aligns per checkpoint.
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

    /// Starts an ask-Claude call about the current checkpoint (or condenses into a
    /// note with `question: None`). No-op off a checkpoint (the done screen).
    pub(super) fn start_ask(
        &mut self,
        cfg: &AskConfig,
        audience: Audience,
        question: Option<String>,
    ) -> bool {
        let Some(card) = self.checkpoint_card() else {
            return false;
        };
        // Ground the walk tutor in the trace's live source (opt-in), with the
        // current checkpoint's frozen excerpt as the anchor.
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

    /// Drains a finished ask reply; a "save note" condense appends a `!` line to
    /// the current checkpoint in the trace deck file.
    pub(super) fn poll_ask(&mut self) -> (Option<String>, Option<String>) {
        self.ask.align(self.walk.checkpoint().map(|c| c.card_id));
        let deck_path = self.walk.trace().deck_path.clone();
        self.ask.poll(|card, notes| {
            crate::deck::append_note(&deck_path, card.line, notes).map_err(|e| e.to_string())
        })
    }

    pub(super) fn ask_dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        self.ask.dto(status, error)
    }

    /// After a prediction, kick off a background Claude grade — a no-op outside
    /// `--grade` mode. Clears any prior grade state for the fresh reveal.
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

    /// Drains a finished background grade into `grade_result`/`grade_error`.
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

    /// Clears all grade state when leaving a reveal.
    pub(super) fn clear_grade(&mut self) {
        self.pending = None;
        self.grade_result = None;
        self.grade_error = None;
    }
}

/// The server's live browse state once decks are chosen. Its absence (`None`)
/// means the deck-selection phase.
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
