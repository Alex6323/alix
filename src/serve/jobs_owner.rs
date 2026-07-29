//! The Jobs owner: generate, share, receive, and the three remote job
//! slots. Workers validate and resolve, then send one typed command; the
//! owner runs polls, spawns, and the receive landings on its own thread, so
//! the landing's collision check-then-act is owner-serialized (the
//! destination-owned command ADR 0027 asks for) instead of leaning on a
//! global lock. On shutdown the owner cancels the wormhole subprocess jobs
//! before it exits.

use std::{path::PathBuf, sync::mpsc, thread, time::Instant};

use anyhow::anyhow;

use super::{catalog_owner::CatalogHandle, dto::*, jobs::*};
use crate::{
    config::{AskConfig, ExamConfig, GenerateDeckConfig},
    deck::Deck,
    exam, generate, share, trace,
};

pub(super) struct JobsState {
    // Invalidated after every landing the owner performs, so the catalog's
    // name map sees received and generated decks without waiting for the
    // next metadata drift check.
    pub(super) catalog: CatalogHandle,
    pub(super) generating: Option<Generating>,
    pub(super) sharing: Option<Sharing>,
    pub(super) receiving: Option<Receiving>,
    pub(super) remote_ask: Option<RemoteAsk>,
    pub(super) remote_exam: Option<RemoteExamining>,
    pub(super) remote_generate: Option<RemoteGenerating>,
}

pub(super) enum Started<T> {
    Dto(T),
    Conflict,
}

pub(super) enum RemoteGradeReply {
    Dto(RemoteExamDto),
    NoSitting,
    WrongPhaseOrCount,
}

type Reply<T> = mpsc::Sender<T>;

pub(super) enum JobsCommand {
    GenerateStart {
        url: String,
        guidance: Option<String>,
        dest: PathBuf,
        generate_cfg: GenerateDeckConfig,
        ask_cfg: AskConfig,
        reply: Reply<Started<GenerateDto>>,
    },
    GeneratePoll(Reply<Option<GenerateDto>>),
    GenerateClose(Reply<()>),
    ShareStart {
        path: PathBuf,
        reply: Reply<Started<ShareDto>>,
    },
    SharePoll(Reply<Option<ShareDto>>),
    ShareClose(Reply<()>),
    ReceiveStart {
        code: String,
        dest: PathBuf,
        reply: Reply<Started<ReceiveDto>>,
    },
    ReceivePoll(Reply<Option<ReceiveDto>>),
    ReceiveClose(Reply<()>),
    ReceiveZip {
        bytes: Vec<u8>,
        dest: PathBuf,
        reply: Reply<Option<ReceiveDto>>,
    },
    ImportDeck {
        dir: PathBuf,
        name: String,
        text: String,
        reply: Reply<Option<ImportDto>>,
    },
    RemoteAsk {
        req: RemoteAskReq,
        ask_cfg: AskConfig,
        reply: Reply<Started<RemoteAskDto>>,
    },
    RemoteDraft {
        req: RemoteDraftReq,
        ask_cfg: AskConfig,
        reply: Reply<Started<RemoteAskDto>>,
    },
    RemoteNote {
        req: RemoteNoteReq,
        ask_cfg: AskConfig,
        reply: Reply<Started<RemoteAskDto>>,
    },
    RemoteAskPoll(Reply<RemoteAskDto>),
    RemoteExamStart {
        path: PathBuf,
        exam_cfg: ExamConfig,
        ask_cfg: AskConfig,
        reply: Reply<Started<RemoteExamDto>>,
    },
    RemoteExamPoll(Reply<RemoteExamDto>),
    RemoteExamGrade {
        answers: Vec<String>,
        reply: Reply<RemoteGradeReply>,
    },
    RemoteExamRemediate(Reply<Option<RemoteExamDto>>),
    RemoteExamClose(Reply<()>),
    RemoteGenerateStart {
        url: String,
        guidance: Option<String>,
        generate_cfg: GenerateDeckConfig,
        ask_cfg: AskConfig,
        reply: Reply<Started<RemoteGenerateDto>>,
    },
    RemoteGeneratePoll(Reply<Option<RemoteGenerateDto>>),
    RemoteGenerateClose(Reply<()>),
}

#[derive(Clone)]
pub(super) struct JobsHandle {
    tx: mpsc::Sender<JobsCommand>,
}

impl JobsHandle {
    fn call<R>(&self, build: impl FnOnce(Reply<R>) -> JobsCommand) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(build(tx)).ok()?;
        rx.recv().ok()
    }

    pub(super) fn generate_start(
        &self,
        url: String,
        guidance: Option<String>,
        dest: PathBuf,
        generate_cfg: GenerateDeckConfig,
        ask_cfg: AskConfig,
    ) -> Option<Started<GenerateDto>> {
        self.call(|reply| JobsCommand::GenerateStart {
            url,
            guidance,
            dest,
            generate_cfg,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn generate_poll(&self) -> Option<Option<GenerateDto>> {
        self.call(JobsCommand::GeneratePoll)
    }
    pub(super) fn generate_close(&self) -> Option<()> {
        self.call(JobsCommand::GenerateClose)
    }
    pub(super) fn share_start(&self, path: PathBuf) -> Option<Started<ShareDto>> {
        self.call(|reply| JobsCommand::ShareStart { path, reply })
    }
    pub(super) fn share_poll(&self) -> Option<Option<ShareDto>> {
        self.call(JobsCommand::SharePoll)
    }
    pub(super) fn share_close(&self) -> Option<()> {
        self.call(JobsCommand::ShareClose)
    }
    pub(super) fn receive_start(&self, code: String, dest: PathBuf) -> Option<Started<ReceiveDto>> {
        self.call(|reply| JobsCommand::ReceiveStart { code, dest, reply })
    }
    pub(super) fn receive_poll(&self) -> Option<Option<ReceiveDto>> {
        self.call(JobsCommand::ReceivePoll)
    }
    pub(super) fn receive_close(&self) -> Option<()> {
        self.call(JobsCommand::ReceiveClose)
    }
    pub(super) fn receive_zip(&self, bytes: Vec<u8>, dest: PathBuf) -> Option<Option<ReceiveDto>> {
        self.call(|reply| JobsCommand::ReceiveZip { bytes, dest, reply })
    }
    pub(super) fn import_deck(
        &self,
        dir: PathBuf,
        name: String,
        text: String,
    ) -> Option<Option<ImportDto>> {
        self.call(|reply| JobsCommand::ImportDeck {
            dir,
            name,
            text,
            reply,
        })
    }
    pub(super) fn remote_ask(
        &self,
        req: RemoteAskReq,
        ask_cfg: AskConfig,
    ) -> Option<Started<RemoteAskDto>> {
        self.call(|reply| JobsCommand::RemoteAsk {
            req,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn remote_draft(
        &self,
        req: RemoteDraftReq,
        ask_cfg: AskConfig,
    ) -> Option<Started<RemoteAskDto>> {
        self.call(|reply| JobsCommand::RemoteDraft {
            req,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn remote_note(
        &self,
        req: RemoteNoteReq,
        ask_cfg: AskConfig,
    ) -> Option<Started<RemoteAskDto>> {
        self.call(|reply| JobsCommand::RemoteNote {
            req,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn remote_ask_poll(&self) -> Option<RemoteAskDto> {
        self.call(JobsCommand::RemoteAskPoll)
    }
    pub(super) fn remote_exam_start(
        &self,
        path: PathBuf,
        exam_cfg: ExamConfig,
        ask_cfg: AskConfig,
    ) -> Option<Started<RemoteExamDto>> {
        self.call(|reply| JobsCommand::RemoteExamStart {
            path,
            exam_cfg,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn remote_exam_poll(&self) -> Option<RemoteExamDto> {
        self.call(JobsCommand::RemoteExamPoll)
    }
    pub(super) fn remote_exam_grade(&self, answers: Vec<String>) -> Option<RemoteGradeReply> {
        self.call(|reply| JobsCommand::RemoteExamGrade { answers, reply })
    }
    pub(super) fn remote_exam_remediate(&self) -> Option<Option<RemoteExamDto>> {
        self.call(JobsCommand::RemoteExamRemediate)
    }
    pub(super) fn remote_exam_close(&self) -> Option<()> {
        self.call(JobsCommand::RemoteExamClose)
    }
    pub(super) fn remote_generate_start(
        &self,
        url: String,
        guidance: Option<String>,
        generate_cfg: GenerateDeckConfig,
        ask_cfg: AskConfig,
    ) -> Option<Started<RemoteGenerateDto>> {
        self.call(|reply| JobsCommand::RemoteGenerateStart {
            url,
            guidance,
            generate_cfg,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn remote_generate_poll(&self) -> Option<Option<RemoteGenerateDto>> {
        self.call(JobsCommand::RemoteGeneratePoll)
    }
    pub(super) fn remote_generate_close(&self) -> Option<()> {
        self.call(JobsCommand::RemoteGenerateClose)
    }
}

pub(super) fn spawn(
    failure: super::OwnerFailure,
    state: JobsState,
) -> (JobsHandle, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let handle = super::supervised(failure, move || {
        let mut state = state;
        for cmd in rx {
            state.handle(cmd);
        }
        // Handles are gone: cancel the wormhole subprocesses so no child
        // outlives the server; AI job threads find their receivers dropped.
        if let Some(s) = state.sharing.take() {
            s.job.cancel();
        }
        if let Some(r) = state.receiving.take() {
            r.job.cancel();
        }
    });
    (JobsHandle { tx }, handle)
}

impl JobsState {
    fn handle(&mut self, cmd: JobsCommand) {
        match cmd {
            JobsCommand::GenerateStart {
                url,
                guidance,
                dest,
                generate_cfg,
                ask_cfg,
                reply,
            } => {
                let _ = reply.send(self.generate_start(url, guidance, dest, generate_cfg, ask_cfg));
            }
            JobsCommand::GeneratePoll(reply) => {
                let out = self.generating.as_mut().map(|g| {
                    let was_settled = g.outcome.is_some();
                    g.poll();
                    if !was_settled && matches!(g.outcome, Some(Ok(_))) {
                        self.catalog.invalidate_content();
                    }
                    g.dto()
                });
                let _ = reply.send(out);
            }
            JobsCommand::GenerateClose(reply) => {
                self.generating = None;
                let _ = reply.send(());
            }
            JobsCommand::ShareStart { path, reply } => {
                let _ = reply.send(self.share_start(path));
            }
            JobsCommand::SharePoll(reply) => {
                let out = self.sharing.as_mut().map(|s| {
                    s.poll();
                    s.dto()
                });
                let _ = reply.send(out);
            }
            JobsCommand::ShareClose(reply) => {
                if let Some(s) = self.sharing.take() {
                    s.job.cancel();
                }
                let _ = reply.send(());
            }
            JobsCommand::ReceiveStart { code, dest, reply } => {
                let _ = reply.send(self.receive_start(code, dest));
            }
            JobsCommand::ReceivePoll(reply) => {
                let out = self.receiving.as_mut().map(|r| {
                    let was_settled = r.outcome.is_some();
                    r.poll();
                    if !was_settled && matches!(r.outcome, Some(Ok(_))) {
                        self.catalog.invalidate_content();
                    }
                    r.dto()
                });
                let _ = reply.send(out);
            }
            JobsCommand::ReceiveClose(reply) => {
                if let Some(r) = self.receiving.take() {
                    r.job.cancel();
                }
                let _ = reply.send(());
            }
            JobsCommand::ReceiveZip { bytes, dest, reply } => {
                // The landing's collision check is check-then-act: safe
                // because every landing (this one and the receive poll's)
                // runs on this owner thread.
                let landed = tempfile::tempdir().ok().and_then(|tmp| {
                    let zip_path = tmp.path().join("got.zip");
                    std::fs::write(&zip_path, &bytes).ok()?;
                    let scratch = tmp.path().join("out");
                    std::fs::create_dir_all(&scratch).ok()?;
                    share::unzip_to(&zip_path, &scratch).ok()?;
                    share::land_received(&scratch, &dest).ok()
                });
                let out = landed.map(|(landed, stripped)| {
                    self.catalog.invalidate_content();
                    ReceiveDto {
                        phase: "done",
                        landed: Some(landed),
                        stripped,
                        elapsed: Some(0),
                        error: None,
                    }
                });
                let _ = reply.send(out);
            }
            // Every destination write runs on this thread, so place_deck's
            // collision check and per-name temp file never race the receive
            // and generate landings or another import.
            JobsCommand::ImportDeck {
                dir,
                name,
                text,
                reply,
            } => {
                let out = match crate::library::place_deck(&dir, &name, &text) {
                    Ok(p) if p.parse_error.is_none() => {
                        self.catalog.invalidate_content();
                        let deck = p
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        Some(ImportDto {
                            deck,
                            cards: p.cards,
                        })
                    }
                    // Uploads are strict: don't keep an invalid deck around.
                    Ok(p) => {
                        let _ = std::fs::remove_file(&p.path);
                        None
                    }
                    Err(_) => None,
                };
                let _ = reply.send(out);
            }
            JobsCommand::RemoteAsk {
                req,
                ask_cfg,
                reply,
            } => {
                let out = self.remote_slot_free().then(|| {
                    let job = RemoteAsk::ask(&ask_cfg, &req.card, req.history, &req.question);
                    let dto = job.dto();
                    self.remote_ask = Some(job);
                    dto
                });
                let _ = reply.send(match out {
                    Some(dto) => Started::Dto(dto),
                    None => Started::Conflict,
                });
            }
            JobsCommand::RemoteDraft {
                req,
                ask_cfg,
                reply,
            } => {
                let out = self.remote_slot_free().then(|| {
                    let job = RemoteAsk::draft(&ask_cfg, &req.card, req.history);
                    let dto = job.dto();
                    self.remote_ask = Some(job);
                    dto
                });
                let _ = reply.send(match out {
                    Some(dto) => Started::Dto(dto),
                    None => Started::Conflict,
                });
            }
            JobsCommand::RemoteNote {
                req,
                ask_cfg,
                reply,
            } => {
                let out = self.remote_slot_free().then(|| {
                    let job = RemoteAsk::note(&ask_cfg, &req.card, req.history);
                    let dto = job.dto();
                    self.remote_ask = Some(job);
                    dto
                });
                let _ = reply.send(match out {
                    Some(dto) => Started::Dto(dto),
                    None => Started::Conflict,
                });
            }
            JobsCommand::RemoteAskPoll(reply) => {
                let dto = match self.remote_ask.as_mut() {
                    Some(a) => {
                        a.poll();
                        a.dto()
                    }
                    None => RemoteAskDto {
                        thinking: false,
                        answer: None,
                        draft: None,
                        note: None,
                        error: None,
                        elapsed: None,
                    },
                };
                let _ = reply.send(dto);
            }
            JobsCommand::RemoteExamStart {
                path,
                exam_cfg,
                ask_cfg,
                reply,
            } => {
                let _ = reply.send(self.remote_exam_start(path, exam_cfg, ask_cfg));
            }
            JobsCommand::RemoteExamPoll(reply) => {
                // advance() only, never poll(): poll() writes the store,
                // which remote handlers must never touch.
                let dto = match self.remote_exam.as_mut() {
                    Some(ex) => {
                        ex.advance();
                        ex.dto()
                    }
                    None => remote_exam_idle_dto(),
                };
                let _ = reply.send(dto);
            }
            JobsCommand::RemoteExamGrade { answers, reply } => {
                let out = match self.remote_exam.as_mut() {
                    None => RemoteGradeReply::NoSitting,
                    Some(ex) => {
                        if !matches!(ex.sitting.phase(), exam::Phase::Answering) {
                            RemoteGradeReply::NoSitting
                        } else {
                            let got = answers.len();
                            if !ex.sitting.set_answers(answers) {
                                eprintln!(
                                    "remote exam grade: expected {} answers, got {got}",
                                    ex.sitting.total()
                                );
                                RemoteGradeReply::WrongPhaseOrCount
                            } else {
                                ex.sitting.submit();
                                RemoteGradeReply::Dto(ex.dto())
                            }
                        }
                    }
                };
                let _ = reply.send(out);
            }
            JobsCommand::RemoteExamRemediate(reply) => {
                let out = match self.remote_exam.as_mut() {
                    Some(ex) if ex.sitting.can_remediate() => {
                        ex.sitting.remediate();
                        Some(ex.dto())
                    }
                    _ => None,
                };
                let _ = reply.send(out);
            }
            // Drop the slot; an in-flight thread just finds its receiver
            // gone and its send fails harmlessly.
            JobsCommand::RemoteExamClose(reply) => {
                self.remote_exam = None;
                let _ = reply.send(());
            }
            JobsCommand::RemoteGenerateStart {
                url,
                guidance,
                generate_cfg,
                ask_cfg,
                reply,
            } => {
                if let Some(g) = self.remote_generate.as_mut() {
                    g.poll();
                }
                let out = if self
                    .remote_generate
                    .as_ref()
                    .is_some_and(RemoteGenerating::thinking)
                {
                    Started::Conflict
                } else {
                    let mut cfg = generate_cfg;
                    if let Some(g) = guidance {
                        cfg.extra = Some(g);
                    }
                    let job = RemoteGenerating::start(url, cfg, ask_cfg);
                    let dto = job.dto();
                    self.remote_generate = Some(job);
                    Started::Dto(dto)
                };
                let _ = reply.send(out);
            }
            JobsCommand::RemoteGeneratePoll(reply) => {
                let out = self.remote_generate.as_mut().map(|g| {
                    g.poll();
                    g.dto()
                });
                let _ = reply.send(out);
            }
            JobsCommand::RemoteGenerateClose(reply) => {
                self.remote_generate = None;
                let _ = reply.send(());
            }
        }
    }

    fn remote_slot_free(&mut self) -> bool {
        if let Some(a) = self.remote_ask.as_mut() {
            a.poll();
        }
        !self.remote_ask.as_ref().is_some_and(RemoteAsk::thinking)
    }

    fn generate_start(
        &mut self,
        url: String,
        guidance: Option<String>,
        dest: PathBuf,
        generate_cfg: GenerateDeckConfig,
        ask_cfg: AskConfig,
    ) -> Started<GenerateDto> {
        if let Some(g) = self.generating.as_mut() {
            let was_settled = g.outcome.is_some();
            g.poll();
            if !was_settled && matches!(g.outcome, Some(Ok(_))) {
                self.catalog.invalidate_content();
            }
        }
        if self
            .generating
            .as_ref()
            .is_some_and(|g| g.outcome.is_none())
        {
            return Started::Conflict;
        }
        // Check for a name collision before spawning the (costed) model
        // call, so a collision never throws away paid work.
        let name = generate::deck_name(&url);
        let stem = name.strip_suffix(".md").unwrap_or(&name);
        let file = format!("{stem}.md");
        if dest.join(&file).exists() {
            return Started::Dto(GenerateDto {
                phase: "error",
                deck: None,
                cards: None,
                elapsed: Some(0),
                error: Some(format!(
                    "{file} already exists — rename it or generate into another destination"
                )),
            });
        }
        let mut cfg = generate_cfg;
        if let Some(g) = guidance {
            cfg.extra = Some(g);
        }
        let g = Generating {
            rx: generate::spawn(url.clone(), cfg, ask_cfg),
            url,
            dest,
            started: Instant::now(),
            outcome: None,
        };
        let dto = g.dto();
        self.generating = Some(g);
        Started::Dto(dto)
    }

    fn share_start(&mut self, path: PathBuf) -> Started<ShareDto> {
        if let Some(s) = self.sharing.as_mut() {
            s.poll();
        }
        if self.sharing.as_ref().is_some_and(|s| s.outcome.is_none()) {
            return Started::Conflict;
        }
        let started = tempfile::tempdir()
            .map_err(|e| anyhow!("{e}"))
            .and_then(|tmp| {
                let to_send = stage_for_share(&path, &tmp)?;
                let job = share::send_spawn(&to_send)?;
                Ok(Sharing {
                    job,
                    _stage: tmp,
                    code: None,
                    started: Instant::now(),
                    outcome: None,
                })
            });
        match started {
            Ok(s) => {
                let dto = s.dto();
                self.sharing = Some(s);
                Started::Dto(dto)
            }
            Err(e) => Started::Dto(ShareDto {
                phase: "error",
                code: None,
                elapsed: Some(0),
                error: Some(format!("{e:#}")),
            }),
        }
    }

    fn receive_start(&mut self, code: String, dest: PathBuf) -> Started<ReceiveDto> {
        if let Some(r) = self.receiving.as_mut() {
            let was_settled = r.outcome.is_some();
            r.poll();
            if !was_settled && matches!(r.outcome, Some(Ok(_))) {
                self.catalog.invalidate_content();
            }
        }
        if self.receiving.as_ref().is_some_and(|r| r.outcome.is_none()) {
            return Started::Conflict;
        }
        let started = tempfile::tempdir()
            .map_err(|e| anyhow!("{e}"))
            .and_then(|tmp| {
                let job = share::receive_spawn(&code, tmp.path())?;
                Ok(Receiving {
                    job,
                    tmp,
                    dest,
                    started: Instant::now(),
                    outcome: None,
                })
            });
        match started {
            Ok(r) => {
                let dto = r.dto();
                self.receiving = Some(r);
                Started::Dto(dto)
            }
            Err(e) => Started::Dto(ReceiveDto {
                phase: "error",
                landed: None,
                stripped: Vec::new(),
                elapsed: Some(0),
                error: Some(format!("{e:#}")),
            }),
        }
    }

    fn remote_exam_start(
        &mut self,
        path: PathBuf,
        exam_cfg: ExamConfig,
        ask_cfg: AskConfig,
    ) -> Started<RemoteExamDto> {
        if self.remote_exam.is_some() {
            return Started::Conflict;
        }
        let Ok(deck) = Deck::load(&path) else {
            return Started::Conflict;
        };
        if !deck.has_exam() {
            return Started::Conflict;
        }
        let strictness = deck.settings.exam_strictness.unwrap_or(exam_cfg.strictness);
        let sitting = if deck.is_trace() {
            match trace::Trace::from_deck(&deck) {
                Ok(t) => exam::Sitting::start_trace(
                    t.description.clone(),
                    t.compression_rubric(),
                    deck.subject.clone(),
                    deck.deck_token.clone().unwrap_or_default(),
                    strictness,
                    exam_cfg,
                    ask_cfg,
                ),
                Err(_) => return Started::Conflict,
            }
        } else {
            if exam::ensure_backend_can_examine(&deck, &ask_cfg).is_err() {
                return Started::Conflict;
            }
            exam::Sitting::start(&deck, strictness, exam_cfg, ask_cfg)
        };
        let ex = RemoteExamining {
            sitting,
            cards: None,
        };
        let dto = ex.dto();
        self.remote_exam = Some(ex);
        Started::Dto(dto)
    }
}
