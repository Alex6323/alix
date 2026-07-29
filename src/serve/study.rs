//! The Study/Progress owner: ADR 0027's one physical owner thread for the
//! active session (review, browse, exam, walk, tutor transcript) and every
//! progress document mutation. HTTP workers parse and resolve, then send one
//! typed command and block on its typed reply; the owner never sees a raw
//! request and workers never see the store.

use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
};

use super::{dto::*, jobs::*};
use crate::{
    assemble::{self, AssembleConfig, SelectOptions},
    augment::AugmentCache,
    config::{Audience, ExamConfig, ReviewConfig},
    deck::{self, Deck},
    exam, review,
    session::now_ms,
    store::{self, Store},
    trace::{self, Walk},
    workspace,
};

pub(super) struct StudyConfig {
    pub(super) cfg: AssembleConfig,
    pub(super) exam_cfg: ExamConfig,
    pub(super) review_cfg: ReviewConfig,
    pub(super) audience: Audience,
}

pub(super) struct StudyState {
    pub(super) config: StudyConfig,
    pub(super) store: Store,
    pub(super) store_dirty: bool,
    pub(super) save_error: Option<String>,
    pub(super) reviewing: Option<Reviewing>,
    pub(super) browsing: Option<Browsing>,
    pub(super) examining: Option<Examining>,
    pub(super) walking: Option<Walking>,
    // Owned here (not by Jobs yet) because opening an augment session
    // replaces the active store, and the store has exactly one owner.
    pub(super) augmenting: Option<Augmenting>,
}

pub(super) enum SessionSnapshot {
    Review(Box<StateDto>),
    Browse(BrowseDto),
}

pub(super) enum SelectedDto {
    Walk(WalkDto),
    Review(StateDto),
}

pub(super) enum Feedback<T> {
    Ok(T),
    NoSession,
    Bad,
}

pub(super) enum CreateOutcome {
    Ok(CreateCardResp),
    NoSession,
    Invalid,
    MintFailed,
}

/// A command that replaces the active progress document: refused whenever
/// the dirty store's flush fails, so an unsaved mutation is never dropped
/// (ADR 0027: no force-discard path); repeating the request retries the
/// flush against the untouched state.
pub(super) enum Transition<T> {
    Done(T),
    Rejected,
    FlushFailed,
}

pub(super) enum WalkGradeReply {
    Dto(Box<WalkDto>),
    NoWalk,
    NoDelta,
}

pub(super) enum ImageSource {
    Active(Option<PathBuf>),
    NoActive,
}

/// A walk-tutor start: a question begins a new exchange only when one was
/// given; a note condenses the transcript unconditionally.
pub(super) enum WalkAskAction {
    Question(Option<String>),
    Note,
}

type Reply<T> = mpsc::Sender<T>;

pub(super) enum StudyCommand {
    State(Reply<SessionSnapshot>),
    Select {
        paths: Vec<PathBuf>,
        opts: SelectOptions,
        reply: Reply<Transition<(SelectedDto, Option<Vec<PathBuf>>)>>,
    },
    Browse {
        paths: Vec<PathBuf>,
        reply: Reply<Transition<(BrowseDto, Vec<PathBuf>)>>,
    },
    DeckDrawer {
        path: PathBuf,
        reply: Reply<DeckDrawerDto>,
    },
    Reset {
        name: String,
        paths: Vec<PathBuf>,
        reply: Reply<Transition<ResetDto>>,
    },
    Deselect(Reply<Transition<StateDto>>),
    Grade {
        grade: crate::scheduler::Grade,
        reply: Reply<Option<StateDto>>,
    },
    Skip(Reply<Option<StateDto>>),
    Acquire(Reply<Option<StateDto>>),
    Restart(Reply<Option<StateDto>>),
    Check {
        lines: Vec<String>,
        reply: Reply<Feedback<review::CheckFeedback>>,
    },
    Choose {
        index: usize,
        reply: Reply<Feedback<review::ChoiceFeedback>>,
    },
    Remove(Reply<Option<StateDto>>),
    Promote(Reply<Feedback<StateDto>>),
    AskPoll(Reply<Option<AskDto>>),
    AskCreate {
        req: CreateCardReq,
        reply: Reply<CreateOutcome>,
    },
    ExamStart {
        path: PathBuf,
        decks_root: PathBuf,
        ask_cfg: crate::config::AskConfig,
        reply: Reply<Transition<Box<ExamDto>>>,
    },
    ExamPoll(Reply<Option<ExamDto>>),
    ExamAnswer {
        text: String,
        goto: Option<usize>,
        reply: Reply<Option<ExamDto>>,
    },
    ExamGrade {
        text: String,
        reply: Reply<Option<ExamDto>>,
    },
    ExamRemediate(Reply<Option<ExamDto>>),
    ExamClose(Reply<Transition<StateDto>>),
    WalkPoll(Reply<Option<WalkDto>>),
    WalkPredict {
        text: String,
        reply: Reply<Option<WalkDto>>,
    },
    WalkGrade {
        self_delta: Option<crate::trace::Delta>,
        reply: Reply<WalkGradeReply>,
    },
    WalkRestart(Reply<Option<WalkDto>>),
    WalkAsk {
        action: WalkAskAction,
        ask_cfg: crate::config::AskConfig,
        reply: Reply<Option<AskDto>>,
    },
    WalkAskPoll(Reply<Option<AskDto>>),
    WalkLeave(Reply<Transition<StateDto>>),
    TutorStart {
        action: Option<AskAction>,
        ask_cfg: crate::config::AskConfig,
        reply: Reply<Option<AskDto>>,
    },
    AugmentOpen {
        name: String,
        files: Vec<PathBuf>,
        workspace_dir: Option<PathBuf>,
        decks_root: PathBuf,
        reply: Reply<Transition<AugmentDto>>,
    },
    AugmentGenerate {
        targets: Option<Vec<(String, Option<String>)>>,
        ai_cfg: crate::config::AiConfig,
        ask_cfg: crate::config::AskConfig,
        reply: Reply<Option<AugmentDto>>,
    },
    AugmentPoll {
        ai_cfg: crate::config::AiConfig,
        ask_cfg: crate::config::AskConfig,
        reply: Reply<Option<AugmentDto>>,
    },
    AugmentRemove {
        target: String,
        topology: Option<String>,
        reply: Reply<Option<AugmentDto>>,
    },
    AugmentClose(Reply<Transition<StateDto>>),
    ImagePath {
        key: String,
        reply: Reply<ImageSource>,
    },
    StorePath(Reply<PathBuf>),
    Projection(Reply<Arc<Store>>),
}

#[derive(Clone)]
pub(super) struct StudyHandle {
    tx: mpsc::Sender<StudyCommand>,
}

impl StudyHandle {
    fn call<R>(&self, build: impl FnOnce(Reply<R>) -> StudyCommand) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(build(tx)).ok()?;
        rx.recv().ok()
    }

    pub(super) fn state(&self) -> Option<SessionSnapshot> {
        self.call(StudyCommand::State)
    }
    pub(super) fn select(
        &self,
        paths: Vec<PathBuf>,
        opts: SelectOptions,
    ) -> Option<Transition<(SelectedDto, Option<Vec<PathBuf>>)>> {
        self.call(|reply| StudyCommand::Select { paths, opts, reply })
    }
    pub(super) fn browse(
        &self,
        paths: Vec<PathBuf>,
    ) -> Option<Transition<(BrowseDto, Vec<PathBuf>)>> {
        self.call(|reply| StudyCommand::Browse { paths, reply })
    }
    pub(super) fn deck_drawer(&self, path: PathBuf) -> Option<DeckDrawerDto> {
        self.call(|reply| StudyCommand::DeckDrawer { path, reply })
    }
    pub(super) fn reset(&self, name: String, paths: Vec<PathBuf>) -> Option<Transition<ResetDto>> {
        self.call(|reply| StudyCommand::Reset { name, paths, reply })
    }
    pub(super) fn deselect(&self) -> Option<Transition<StateDto>> {
        self.call(StudyCommand::Deselect)
    }
    pub(super) fn grade(&self, grade: crate::scheduler::Grade) -> Option<Option<StateDto>> {
        self.call(|reply| StudyCommand::Grade { grade, reply })
    }
    pub(super) fn skip(&self) -> Option<Option<StateDto>> {
        self.call(StudyCommand::Skip)
    }
    pub(super) fn acquire(&self) -> Option<Option<StateDto>> {
        self.call(StudyCommand::Acquire)
    }
    pub(super) fn restart(&self) -> Option<Option<StateDto>> {
        self.call(StudyCommand::Restart)
    }
    pub(super) fn check(&self, lines: Vec<String>) -> Option<Feedback<review::CheckFeedback>> {
        self.call(|reply| StudyCommand::Check { lines, reply })
    }
    pub(super) fn choose(&self, index: usize) -> Option<Feedback<review::ChoiceFeedback>> {
        self.call(|reply| StudyCommand::Choose { index, reply })
    }
    pub(super) fn remove(&self) -> Option<Option<StateDto>> {
        self.call(StudyCommand::Remove)
    }
    pub(super) fn promote(&self) -> Option<Feedback<StateDto>> {
        self.call(StudyCommand::Promote)
    }
    pub(super) fn ask_start(
        &self,
        action: Option<AskAction>,
        ask_cfg: crate::config::AskConfig,
    ) -> Option<Option<AskDto>> {
        self.call(|reply| StudyCommand::TutorStart {
            action,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn augment_open(
        &self,
        name: String,
        files: Vec<PathBuf>,
        workspace_dir: Option<PathBuf>,
        decks_root: PathBuf,
    ) -> Option<Transition<AugmentDto>> {
        self.call(|reply| StudyCommand::AugmentOpen {
            name,
            files,
            workspace_dir,
            decks_root,
            reply,
        })
    }
    pub(super) fn augment_generate(
        &self,
        targets: Option<Vec<(String, Option<String>)>>,
        ai_cfg: crate::config::AiConfig,
        ask_cfg: crate::config::AskConfig,
    ) -> Option<Option<AugmentDto>> {
        self.call(|reply| StudyCommand::AugmentGenerate {
            targets,
            ai_cfg,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn augment_poll(
        &self,
        ai_cfg: crate::config::AiConfig,
        ask_cfg: crate::config::AskConfig,
    ) -> Option<Option<AugmentDto>> {
        self.call(|reply| StudyCommand::AugmentPoll {
            ai_cfg,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn augment_remove(
        &self,
        target: String,
        topology: Option<String>,
    ) -> Option<Option<AugmentDto>> {
        self.call(|reply| StudyCommand::AugmentRemove {
            target,
            topology,
            reply,
        })
    }
    pub(super) fn augment_close(&self) -> Option<Transition<StateDto>> {
        self.call(StudyCommand::AugmentClose)
    }
    pub(super) fn store_path(&self) -> Option<PathBuf> {
        self.call(StudyCommand::StorePath)
    }
    pub(super) fn ask_poll(&self) -> Option<Option<AskDto>> {
        self.call(StudyCommand::AskPoll)
    }
    pub(super) fn ask_create(&self, req: CreateCardReq) -> Option<CreateOutcome> {
        self.call(|reply| StudyCommand::AskCreate { req, reply })
    }
    pub(super) fn exam_start(
        &self,
        path: PathBuf,
        decks_root: PathBuf,
        ask_cfg: crate::config::AskConfig,
    ) -> Option<Transition<Box<ExamDto>>> {
        self.call(|reply| StudyCommand::ExamStart {
            path,
            decks_root,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn exam_poll(&self) -> Option<Option<ExamDto>> {
        self.call(StudyCommand::ExamPoll)
    }
    pub(super) fn exam_answer(&self, text: String, goto: Option<usize>) -> Option<Option<ExamDto>> {
        self.call(|reply| StudyCommand::ExamAnswer { text, goto, reply })
    }
    pub(super) fn exam_grade(&self, text: String) -> Option<Option<ExamDto>> {
        self.call(|reply| StudyCommand::ExamGrade { text, reply })
    }
    pub(super) fn exam_remediate(&self) -> Option<Option<ExamDto>> {
        self.call(StudyCommand::ExamRemediate)
    }
    pub(super) fn exam_close(&self) -> Option<Transition<StateDto>> {
        self.call(StudyCommand::ExamClose)
    }
    pub(super) fn walk_poll(&self) -> Option<Option<WalkDto>> {
        self.call(StudyCommand::WalkPoll)
    }
    pub(super) fn walk_predict(&self, text: String) -> Option<Option<WalkDto>> {
        self.call(|reply| StudyCommand::WalkPredict { text, reply })
    }
    pub(super) fn walk_grade(
        &self,
        self_delta: Option<crate::trace::Delta>,
    ) -> Option<WalkGradeReply> {
        self.call(|reply| StudyCommand::WalkGrade { self_delta, reply })
    }
    pub(super) fn walk_restart(&self) -> Option<Option<WalkDto>> {
        self.call(StudyCommand::WalkRestart)
    }
    pub(super) fn walk_ask(
        &self,
        action: WalkAskAction,
        ask_cfg: crate::config::AskConfig,
    ) -> Option<Option<AskDto>> {
        self.call(|reply| StudyCommand::WalkAsk {
            action,
            ask_cfg,
            reply,
        })
    }
    pub(super) fn walk_ask_poll(&self) -> Option<Option<AskDto>> {
        self.call(StudyCommand::WalkAskPoll)
    }
    pub(super) fn walk_leave(&self) -> Option<Transition<StateDto>> {
        self.call(StudyCommand::WalkLeave)
    }
    pub(super) fn image_path(&self, key: String) -> Option<ImageSource> {
        self.call(|reply| StudyCommand::ImagePath { key, reply })
    }
    pub(super) fn projection(&self) -> Option<Arc<Store>> {
        self.call(StudyCommand::Projection)
    }
}

pub(super) fn spawn(state: StudyState) -> (StudyHandle, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(state, rx);
        })) {
            super::OWNER_FAILED.store(true, std::sync::atomic::Ordering::SeqCst);
            std::panic::resume_unwind(panic);
        }
    });
    (StudyHandle { tx }, handle)
}

fn run(mut s: StudyState, rx: mpsc::Receiver<StudyCommand>) {
    for cmd in rx {
        s.handle(cmd);
    }
    // Handles are gone (workers drained): one last flush covers any mutation
    // whose own save failed transiently.
    let _ = flush_store(&s.store, &mut s.store_dirty, &mut s.save_error);
}

// Must run before every store replacement and before any command opens a
// store fresh from disk for a mutating operation (reset): a deferred dirty
// store that is replaced or shadowed unflushed silently loses the session.
pub(super) fn flush_store(store: &Store, dirty: &mut bool, save_error: &mut Option<String>) -> bool {
    if !*dirty {
        return true;
    }
    match store.save() {
        Ok(()) => {
            *dirty = false;
            *save_error = None;
            true
        }
        Err(e) => {
            eprintln!("warning: could not save progress: {e}");
            *save_error = Some(e.to_string());
            false
        }
    }
}

// Runs on every store mutation: the grade (or exam flag, badge, removal) is
// on disk before its response returns. A failed save lands in `save_error`
// for the state DTO; the transition-time flushes stay as backstops.
pub(super) fn flush_mutation(store: &Store, dirty: &mut bool, save_error: &mut Option<String>) {
    *dirty = true;
    let _ = flush_store(store, dirty, save_error);
}

fn flush_presented(
    r: &mut Reviewing,
    store: &Store,
    dirty: &mut bool,
    save_error: &mut Option<String>,
) {
    if r.session.take_presented_stamped() {
        flush_mutation(store, dirty, save_error);
    }
}

impl StudyState {
    fn review_dto(&self) -> StateDto {
        review_state(
            self.reviewing.as_ref(),
            &self.store,
            self.save_error.as_deref(),
        )
    }

    fn handle(&mut self, cmd: StudyCommand) {
        match cmd {
            StudyCommand::State(reply) => {
                let snapshot = if let Some(b) = &self.browsing {
                    SessionSnapshot::Browse(browse_payload(Some(b)))
                } else {
                    if let Some(r) = self.reviewing.as_mut() {
                        r.session.poll(&mut self.store, now_ms());
                        flush_presented(
                            r,
                            &self.store,
                            &mut self.store_dirty,
                            &mut self.save_error,
                        );
                    }
                    SessionSnapshot::Review(Box::new(self.review_dto()))
                };
                let _ = reply.send(snapshot);
            }
            StudyCommand::Select { paths, opts, reply } => {
                let _ = reply.send(self.select(paths, opts));
            }
            StudyCommand::Browse { paths, reply } => {
                let _ = reply.send(self.browse(paths));
            }
            StudyCommand::DeckDrawer { path, reply } => {
                let _ = reply.send(self.deck_drawer(path));
            }
            StudyCommand::Reset { name, paths, reply } => {
                let _ = reply.send(self.reset(name, paths));
            }
            StudyCommand::Deselect(reply) => {
                let out = if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error)
                {
                    Transition::FlushFailed
                } else {
                    self.reviewing = None;
                    self.walking = None;
                    self.browsing = None;
                    if let Ok(s) =
                        assemble::store_for(&[], self.config.cfg.instance_store.as_deref())
                    {
                        self.store = s;
                    }
                    Transition::Done(self.review_dto())
                };
                let _ = reply.send(out);
            }
            StudyCommand::Grade { grade, reply } => {
                let _ = reply.send(self.grade(grade));
            }
            StudyCommand::Skip(reply) => {
                let dto = match self.reviewing.as_mut() {
                    None => None,
                    Some(r) => {
                        r.session.skip(&mut self.store, now_ms());
                        flush_presented(
                            r,
                            &self.store,
                            &mut self.store_dirty,
                            &mut self.save_error,
                        );
                        r.rotate_variant();
                        Some(())
                    }
                }
                .map(|()| self.review_dto());
                let _ = reply.send(dto);
            }
            StudyCommand::Acquire(reply) => {
                let dto = match self.reviewing.as_mut() {
                    None => None,
                    Some(r) => {
                        r.session.acquire_current(&mut self.store, now_ms());
                        let _ = r.session.take_presented_stamped();
                        flush_mutation(&self.store, &mut self.store_dirty, &mut self.save_error);
                        r.rotate_variant();
                        Some(())
                    }
                }
                .map(|()| self.review_dto());
                let _ = reply.send(dto);
            }
            StudyCommand::Restart(reply) => {
                let dto = match self.reviewing.as_mut() {
                    None => None,
                    Some(r) => {
                        r.session.restart(&mut self.store, now_ms());
                        flush_presented(
                            r,
                            &self.store,
                            &mut self.store_dirty,
                            &mut self.save_error,
                        );
                        r.rotate_variant();
                        Some(())
                    }
                }
                .map(|()| self.review_dto());
                let _ = reply.send(dto);
            }
            StudyCommand::Check { lines, reply } => {
                let out = match self.reviewing.as_ref() {
                    None => Feedback::NoSession,
                    Some(r) => match review::check_typed(&r.session, &lines) {
                        Some(f) => Feedback::Ok(f),
                        None => Feedback::Bad,
                    },
                };
                let _ = reply.send(out);
            }
            StudyCommand::Choose { index, reply } => {
                let out = match self.reviewing.as_ref() {
                    None => Feedback::NoSession,
                    Some(r) => match review::choose(&r.session, &self.store, &r.augment, index) {
                        Some(f) => Feedback::Ok(f),
                        None => Feedback::Bad,
                    },
                };
                let _ = reply.send(out);
            }
            StudyCommand::Remove(reply) => {
                let _ = reply.send(self.remove());
            }
            StudyCommand::Promote(reply) => {
                let _ = reply.send(self.promote());
            }
            StudyCommand::TutorStart {
                action,
                ask_cfg,
                reply,
            } => {
                let audience = self.config.audience;
                let dto = self.reviewing.as_mut().map(|r| {
                    if let Some(action) = action {
                        r.start_ask(&ask_cfg, audience, action);
                    }
                    r.ask_dto(None, None)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::AugmentOpen {
                name,
                files,
                workspace_dir,
                decks_root,
                reply,
            } => {
                let _ = reply.send(self.augment_open(name, files, workspace_dir, decks_root));
            }
            StudyCommand::AugmentGenerate {
                targets,
                ai_cfg,
                ask_cfg,
                reply,
            } => {
                let dto = self.augmenting.as_mut().map(|aug| {
                    if let Some(targets) = targets {
                        aug.generate_batch(targets, &ai_cfg, &ask_cfg);
                    }
                    aug.dto()
                });
                let _ = reply.send(dto);
            }
            StudyCommand::AugmentPoll {
                ai_cfg,
                ask_cfg,
                reply,
            } => {
                let dto = self.augmenting.as_mut().map(|aug| {
                    aug.poll(&ai_cfg, &ask_cfg);
                    aug.dto()
                });
                let _ = reply.send(dto);
            }
            StudyCommand::AugmentRemove {
                target,
                topology,
                reply,
            } => {
                let dto = self.augmenting.as_mut().map(|aug| {
                    aug.remove(&target, topology.as_deref());
                    aug.dto()
                });
                let _ = reply.send(dto);
            }
            StudyCommand::AugmentClose(reply) => {
                let out = if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error)
                {
                    Transition::FlushFailed
                } else {
                    self.augmenting = None;
                    if let Ok(s) =
                        assemble::store_for(&[], self.config.cfg.instance_store.as_deref())
                    {
                        self.store = s;
                    }
                    Transition::Done(self.review_dto())
                };
                let _ = reply.send(out);
            }
            StudyCommand::AskPoll(reply) => {
                let dto = self.reviewing.as_mut().map(|r| {
                    let (status, error) = r.poll_ask();
                    r.ask_dto(status, error)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::AskCreate { req, reply } => {
                let _ = reply.send(self.ask_create(req));
            }
            StudyCommand::ExamStart {
                path,
                decks_root,
                ask_cfg,
                reply,
            } => {
                let _ = reply.send(self.exam_start(path, decks_root, ask_cfg));
            }
            StudyCommand::ExamPoll(reply) => {
                let dto = match self.examining.as_mut() {
                    None => None,
                    Some(ex) => {
                        let root = workspace::content_root(&ex.deck_path);
                        let retire_after_days = self
                            .config
                            .review_cfg
                            .for_workspace(&root)
                            .retire_after_days;
                        let poll = ex.sitting.poll(&mut self.store, now_ms(), retire_after_days);
                        if poll.store_mutated {
                            flush_mutation(
                                &self.store,
                                &mut self.store_dirty,
                                &mut self.save_error,
                            );
                        }
                        Some(exam_dto(ex))
                    }
                };
                let _ = reply.send(dto);
            }
            StudyCommand::ExamAnswer { text, goto, reply } => {
                let dto = self.examining.as_mut().map(|ex| {
                    ex.sitting.set_answer(text);
                    if let Some(i) = goto {
                        ex.sitting.goto(i);
                    }
                    exam_dto(ex)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::ExamGrade { text, reply } => {
                let dto = self.examining.as_mut().map(|ex| {
                    ex.sitting.set_answer(text);
                    ex.sitting.submit();
                    exam_dto(ex)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::ExamRemediate(reply) => {
                let dto = self.examining.as_mut().map(|ex| {
                    ex.sitting.remediate();
                    exam_dto(ex)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::ExamClose(reply) => {
                let out = if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error)
                {
                    Transition::FlushFailed
                } else {
                    self.examining = None;
                    if let Ok(s) =
                        assemble::store_for(&[], self.config.cfg.instance_store.as_deref())
                    {
                        self.store = s;
                    }
                    Transition::Done(self.review_dto())
                };
                let _ = reply.send(out);
            }
            StudyCommand::WalkPoll(reply) => {
                let dto = self.walking.as_mut().map(|w| {
                    w.poll();
                    walk_dto(w)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::WalkPredict { text, reply } => {
                let dto = self.walking.as_mut().map(|w| {
                    w.walk.predict(text);
                    w.start_grade();
                    walk_dto(w)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::WalkGrade { self_delta, reply } => {
                let out = match self.walking.as_mut() {
                    None => WalkGradeReply::NoWalk,
                    Some(w) => {
                        let delta = w.grade_result.as_ref().map(|(d, _)| *d).or(self_delta);
                        match delta {
                            Some(delta) => {
                                w.walk.grade(&mut self.store, delta, now_ms());
                                flush_mutation(
                                    &self.store,
                                    &mut self.store_dirty,
                                    &mut self.save_error,
                                );
                                w.clear_grade();
                                WalkGradeReply::Dto(Box::new(walk_dto(w)))
                            }
                            None => WalkGradeReply::NoDelta,
                        }
                    }
                };
                let _ = reply.send(out);
            }
            StudyCommand::WalkRestart(reply) => {
                let dto = self.walking.as_mut().map(|w| {
                    let fresh = Walk::new(w.walk.trace().clone());
                    let grade = w.grade.take();
                    *w = Walking::new(fresh, grade);
                    walk_dto(w)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::WalkAsk {
                action,
                ask_cfg,
                reply,
            } => {
                let audience = self.config.audience;
                let dto = self.walking.as_mut().map(|w| {
                    match action {
                        WalkAskAction::Question(Some(q)) => {
                            w.start_ask(&ask_cfg, audience, Some(q));
                        }
                        WalkAskAction::Question(None) => {}
                        WalkAskAction::Note => {
                            w.start_ask(&ask_cfg, audience, None);
                        }
                    }
                    w.ask_dto(None, None)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::WalkAskPoll(reply) => {
                let dto = self.walking.as_mut().map(|w| {
                    let (status, error) = w.poll_ask();
                    w.ask_dto(status, error)
                });
                let _ = reply.send(dto);
            }
            StudyCommand::WalkLeave(reply) => {
                let out = if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error)
                {
                    Transition::FlushFailed
                } else {
                    self.walking = None;
                    if let Ok(s) =
                        assemble::store_for(&[], self.config.cfg.instance_store.as_deref())
                    {
                        self.store = s;
                    }
                    Transition::Done(self.review_dto())
                };
                let _ = reply.send(out);
            }
            StudyCommand::ImagePath { key, reply } => {
                let out = if let Some(r) = &self.reviewing {
                    ImageSource::Active(r.images.get(&key).cloned())
                } else if let Some(b) = &self.browsing {
                    ImageSource::Active(b.images.get(&key).cloned())
                } else {
                    ImageSource::NoActive
                };
                let _ = reply.send(out);
            }
            StudyCommand::StorePath(reply) => {
                let _ = reply.send(self.store.path().to_path_buf());
            }
            StudyCommand::Projection(reply) => {
                let _ = reply.send(Arc::new(self.store.clone()));
            }
        }
    }

    fn augment_open(
        &mut self,
        name: String,
        files: Vec<PathBuf>,
        workspace_dir: Option<PathBuf>,
        decks_root: PathBuf,
    ) -> Transition<AugmentDto> {
        if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error) {
            return Transition::FlushFailed;
        }
        if let Ok(s) = assemble::store_for(&files, self.config.cfg.instance_store.as_deref()) {
            self.store = s;
        }
        // Stamp before loading: unstamped ids collapse the cache to key 0,
        // orphaning the spend at the first real stamp.
        let Ok(cards) = assemble::stamp_and_load_cards(&files) else {
            return Transition::Rejected;
        };
        let decks: Vec<_> = files
            .iter()
            .filter_map(|path| crate::deck::Deck::load(path).ok())
            .collect();
        let deck_tokens: Vec<String> = decks
            .iter()
            .filter_map(|deck| deck.deck_token.clone())
            .collect();
        let workspace_root = workspace_dir
            .clone()
            .or_else(|| {
                files
                    .first()
                    .map(|path| crate::workspace::content_root(path))
            })
            .unwrap_or(decks_root);
        let Ok(cache) = AugmentCache::open_for_decks(&workspace_root, &decks) else {
            return Transition::Rejected;
        };
        let aug = Augmenting::open(name, cards, deck_tokens, cache, workspace_dir);
        let dto = aug.dto();
        self.augmenting = Some(aug);
        Transition::Done(dto)
    }

    fn select(
        &mut self,
        paths: Vec<PathBuf>,
        opts: SelectOptions,
    ) -> Transition<(SelectedDto, Option<Vec<PathBuf>>)> {
        if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error) {
            return Transition::FlushFailed;
        }
        if let Err(e) = assemble::store_for(&paths, self.config.cfg.instance_store.as_deref())
            .map(|s| self.store = s)
        {
            eprintln!("warning: could not open the progress store: {e}");
            return Transition::Rejected;
        }
        let recorded_paths = paths.clone();
        match assemble::select(paths, &mut self.store, &self.config.cfg, &opts) {
            Ok(assemble::Selected::Walk(wb)) => {
                let w = Walking::new(wb.walk, wb.grade);
                let dto = walk_dto(&w);
                self.walking = Some(w);
                self.reviewing = None;
                self.examining = None;
                Transition::Done((SelectedDto::Walk(dto), None))
            }
            Ok(assemble::Selected::Review(b)) => {
                let record = (!b.session.is_finished()).then_some(recorded_paths);
                let mut r = Reviewing::new(b);
                // `assemble::select` already saved the store, stamp included.
                let _ = r.session.take_presented_stamped();
                r.rotate_variant();
                self.reviewing = Some(r);
                self.walking = None;
                Transition::Done((SelectedDto::Review(self.review_dto()), record))
            }
            Err(e) => {
                eprintln!("warning: could not load the selected decks: {e}");
                Transition::Rejected
            }
        }
    }

    fn browse(&mut self, paths: Vec<PathBuf>) -> Transition<(BrowseDto, Vec<PathBuf>)> {
        if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error) {
            return Transition::FlushFailed;
        }
        if let Err(e) = assemble::store_for(&paths, self.config.cfg.instance_store.as_deref())
            .map(|s| self.store = s)
        {
            eprintln!("warning: could not open the progress store: {e}");
            return Transition::Rejected;
        }
        let recorded_paths = paths.clone();
        match assemble::browse(paths, self.config.cfg.instance_store.as_deref()) {
            Ok(b) => {
                self.browsing = Some(Browsing::new(b));
                self.reviewing = None;
                self.walking = None;
                self.examining = None;
                Transition::Done((browse_payload(self.browsing.as_ref()), recorded_paths))
            }
            Err(e) => {
                eprintln!("warning: could not load the selected decks: {e}");
                Transition::Rejected
            }
        }
    }

    fn deck_drawer(&mut self, path: PathBuf) -> DeckDrawerDto {
        match (
            Deck::load(&path),
            assemble::store_for(
                std::slice::from_ref(&path),
                self.config.cfg.instance_store.as_deref(),
            ),
        ) {
            (Ok(deck), Ok(s)) => {
                let Ok(augment) = AugmentCache::open_for_deck(&deck) else {
                    return DeckDrawerDto::default();
                };
                let root = workspace::content_root(&path);
                let retire_after_days = self
                    .config
                    .review_cfg
                    .for_workspace(&root)
                    .retire_after_days;
                deck_drawer_dto(&augment, &s, &deck, retire_after_days)
            }
            _ => DeckDrawerDto::default(),
        }
    }

    fn reset(&mut self, name: String, paths: Vec<PathBuf>) -> Transition<ResetDto> {
        if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error) {
            return Transition::FlushFailed;
        }
        let Ok(decks) = paths
            .iter()
            .map(Deck::load)
            .collect::<Result<Vec<Deck>, _>>()
        else {
            return Transition::Rejected;
        };
        let Ok(cleared) = assemble::store_for(&paths, self.config.cfg.instance_store.as_deref())
            .and_then(|mut s| crate::library::reset_decks(&mut s, decks.iter()))
        else {
            return Transition::Rejected;
        };
        if let Ok(s) = assemble::store_for(&[], self.config.cfg.instance_store.as_deref()) {
            self.store = s;
        }
        Transition::Done(ResetDto {
            deck: name,
            cards_cleared: cleared,
        })
    }

    fn grade(&mut self, grade: crate::scheduler::Grade) -> Option<StateDto> {
        let r = self.reviewing.as_mut()?;
        let now = now_ms();
        r.session.grade(&mut self.store, grade, now);
        if let Some(deck_id) = r
            .files
            .paths
            .keys()
            .next()
            .filter(|id| !id.is_empty())
            .cloned()
        {
            store::note_badges(&mut self.store, &deck_id, r.session.cards(), now);
        }
        let _ = r.session.take_presented_stamped();
        flush_mutation(&self.store, &mut self.store_dirty, &mut self.save_error);
        r.rotate_variant();
        Some(self.review_dto())
    }

    fn remove(&mut self) -> Option<StateDto> {
        let r = self.reviewing.as_mut()?;
        let dropped = r.session.remove_current(&mut self.store, now_ms());
        if let Some(first) = dropped.first() {
            let deck_id = first.deck_id.to_string();
            let line = first.line;
            for card in &dropped {
                if let Some(id) = card.id() {
                    self.store.remove(&id);
                }
            }
            let _ = r.session.take_presented_stamped();
            flush_mutation(&self.store, &mut self.store_dirty, &mut self.save_error);
            r.files.remove_block(&deck_id, line);
        }
        flush_presented(r, &self.store, &mut self.store_dirty, &mut self.save_error);
        Some(self.review_dto())
    }

    fn promote(&mut self) -> Feedback<StateDto> {
        let Some(r) = self.reviewing.as_mut() else {
            return Feedback::NoSession;
        };
        if !r.session.current_is_virtual(&self.store) {
            return Feedback::Bad;
        }
        let Some(id) = r.session.current_id() else {
            return Feedback::Bad;
        };
        let Some(deck_id) = r.session.current().map(|c| c.deck_id.to_string()) else {
            return Feedback::Bad;
        };
        let Some(path) = r.files.paths.get(&deck_id).cloned() else {
            return Feedback::Bad;
        };
        if store::promote_virtual(&mut self.store, &id, &path).is_err() {
            return Feedback::Bad;
        }
        flush_mutation(&self.store, &mut self.store_dirty, &mut self.save_error);
        r.session.poll(&mut self.store, now_ms());
        flush_presented(r, &self.store, &mut self.store_dirty, &mut self.save_error);
        Feedback::Ok(self.review_dto())
    }

    fn ask_create(&mut self, req: CreateCardReq) -> CreateOutcome {
        let Some(r) = self.reviewing.as_mut() else {
            return CreateOutcome::NoSession;
        };
        if r.session.current().is_none() {
            return CreateOutcome::NoSession;
        }
        let Some(deck_id) = r
            .files
            .paths
            .keys()
            .next()
            .filter(|id| !id.is_empty())
            .cloned()
        else {
            return CreateOutcome::NoSession;
        };
        // Dedup by content fingerprint, not id: a mint carries a fresh random
        // token, so identical content still collides.
        let deck_fingerprints: std::collections::HashSet<u64> = r
            .session
            .cards()
            .iter()
            .map(|c| c.content_fingerprint)
            .collect();
        let now = now_ms();
        match store::mint_tutor_card(
            &mut self.store,
            &deck_id,
            &req.front,
            &req.back,
            now,
            &deck_fingerprints,
        ) {
            Ok(id) => {
                flush_mutation(&self.store, &mut self.store_dirty, &mut self.save_error);
                CreateOutcome::Ok(CreateCardResp { id })
            }
            Err(store::MintError::Duplicate | store::MintError::Malformed(_)) => {
                CreateOutcome::Invalid
            }
            Err(store::MintError::Mint(_)) => CreateOutcome::MintFailed,
        }
    }

    fn exam_start(
        &mut self,
        path: PathBuf,
        decks_root: PathBuf,
        ask_cfg: crate::config::AskConfig,
    ) -> Transition<Box<ExamDto>> {
        if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error) {
            return Transition::FlushFailed;
        }
        if let Ok(s) = assemble::store_for(
            std::slice::from_ref(&path),
            self.config.cfg.instance_store.as_deref(),
        ) {
            self.store = s;
        }
        match Deck::load(&path) {
            Ok(deck)
                if deck.has_exam()
                    && !deck::is_locked(&deck, Some(decks_root.as_path()), &self.store) =>
            {
                let strictness = deck
                    .settings
                    .exam_strictness
                    .unwrap_or(self.config.exam_cfg.strictness);
                let sitting = if deck.is_trace() {
                    match trace::Trace::from_deck(&deck) {
                        Ok(t) => {
                            if let Some(ms) = exam::cooldown_remaining_ms(
                                &self.store,
                                deck.deck_token.as_deref().unwrap_or_default(),
                                self.config.exam_cfg.retry_cooldown_secs,
                                now_ms(),
                            ) {
                                // One response shape per endpoint: the cooldown
                                // is an ExamDto phase, not untagged.
                                return Transition::Done(Box::new(cooldown_dto(
                                    &deck.subject,
                                    ms,
                                )));
                            }
                            exam::Sitting::start_trace(
                                t.description.clone(),
                                t.compression_rubric(),
                                deck.subject.clone(),
                                deck.deck_token.clone().unwrap_or_default(),
                                strictness,
                                self.config.exam_cfg.clone(),
                                ask_cfg,
                            )
                        }
                        Err(_) => return Transition::Rejected,
                    }
                } else {
                    // Check backend capability before starting, so a gap is a
                    // clean refusal, not a mid-exam poll error.
                    if exam::ensure_backend_can_examine(&deck, &ask_cfg).is_err() {
                        return Transition::Rejected;
                    }
                    exam::Sitting::start(&deck, strictness, self.config.exam_cfg.clone(), ask_cfg)
                };
                let ex = Examining {
                    sitting,
                    deck_path: path,
                };
                let dto = exam_dto(&ex);
                self.examining = Some(ex);
                Transition::Done(Box::new(dto))
            }
            _ => Transition::Rejected,
        }
    }
}
