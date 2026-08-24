//! The Study/Progress owner: ADR 0027's one physical owner thread for the
//! active session (review, browse, exam, walk, tutor transcript) and every
//! progress document mutation. HTTP workers parse and resolve, then send one
//! typed command and block on its typed reply; the owner never sees a raw
//! request and workers never see the store.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
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
    // Snapshots of swapped-out documents, keyed by store path. The owner is
    // their single writer, so its memory is at least as fresh as disk; the
    // catalog reads these so a briefly parked or replaced progress file
    // cannot resurrect a known deck as new.
    pub(super) retained: HashMap<PathBuf, Arc<Store>>,
    pub(super) store_dirty: bool,
    pub(super) save_error: Option<String>,
    // The instance progress directory's last observed (name, len, mtime)
    // stamp: a cheap stat scan per idle listing detects documents that
    // changed, appeared, or vanished after the store was opened, and any
    // change triggers a flush + tolerant reopen, which both discovers new
    // damage and heals repaired documents. None forces one initial scan.
    pub(super) progress_stamp: Option<u64>,
    pub(super) reviewing: Option<Reviewing>,
    // Monotonic identity of the review transition: bumped whenever the
    // current card can change, checked against every card-relative
    // mutation's echoed header before the mutation applies.
    pub(super) revision: u64,
    // Progress-content version: bumped on every store mutation or
    // replacement, so catalog builds can refuse to coalesce requests
    // carrying different progress states.
    pub(super) writes: u64,
    pub(super) browsing: Option<Browsing>,
    pub(super) examining: Option<Examining>,
    pub(super) walking: Option<Walking>,
    // Owned here (not by Jobs yet) because opening an augment session
    // replaces the active store, and the store has exactly one owner.
    pub(super) augmenting: Option<Augmenting>,
}

pub(super) struct StudyProjection {
    pub(super) store: Arc<Store>,
    pub(super) retained: HashMap<PathBuf, Arc<Store>>,
    pub(super) writes: u64,
}

pub(super) enum SessionSnapshot {
    Review(Box<StateDto>),
    Browse(BrowseDto),
}

pub(super) enum SelectedDto {
    Walk(Box<WalkDto>),
    Review(Box<StateDto>),
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

#[derive(Clone)]
pub(super) enum LibraryTarget {
    Deck {
        name: String,
        path: PathBuf,
    },
    Workspace {
        name: String,
        root: PathBuf,
        members: Vec<PathBuf>,
    },
}

impl LibraryTarget {
    pub(super) fn recent_paths(&self) -> Vec<PathBuf> {
        match self {
            Self::Deck { path, .. } => vec![path.clone()],
            Self::Workspace { root, members, .. } => {
                let mut paths = members.clone();
                paths.push(root.clone());
                paths
            }
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Deck { name, .. } | Self::Workspace { name, .. } => name,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Deck { .. } => "deck",
            Self::Workspace { .. } => "workspace",
        }
    }

    fn display_root(&self) -> PathBuf {
        match self {
            Self::Deck { path, .. } => workspace::root_for_deck(path)
                .or_else(|| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| path.clone()),
            Self::Workspace { root, .. } => root.clone(),
        }
    }

    fn label(&self, path: &Path) -> String {
        if path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "progress")
            && let Some(name) = path.file_name()
        {
            return format!("progress/{}", name.to_string_lossy());
        }
        if let Ok(relative) = path.strip_prefix(self.display_root()) {
            if relative.as_os_str().is_empty() {
                return self.name().to_string();
            }
            return relative.to_string_lossy().replace('\\', "/");
        }
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.name().to_string())
    }

    fn labels(&self, paths: &[PathBuf]) -> Vec<String> {
        paths.iter().map(|path| self.label(path)).collect()
    }

    fn covered_store_paths(&self, store_path: &Path) -> Vec<PathBuf> {
        let mut paths = vec![store_path.to_path_buf()];
        let Some(progress_root) = progress_root_for_store(store_path) else {
            return paths;
        };
        let user_files = crate::state::UserFiles::new(
            progress_root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.display_root()),
        );
        let deck_paths: Vec<&PathBuf> = match self {
            Self::Deck { path, .. } => vec![path],
            Self::Workspace { members, .. } => members.iter().collect(),
        };
        paths.extend(deck_paths.into_iter().filter_map(|path| {
            Deck::load(path)
                .ok()
                .and_then(|deck| deck.deck_token)
                .map(|deck_id| user_files.progress_for(&deck_id))
        }));
        paths
    }
}

fn progress_root_for_store(store_path: &Path) -> Option<PathBuf> {
    if store_path
        .file_name()
        .is_some_and(|name| name == "progress")
    {
        Some(store_path.to_path_buf())
    } else if store_path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "progress")
    {
        store_path.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

pub(super) enum RemovalOutcome {
    Done(RemovalDto),
    Rejected,
    Busy,
    FlushFailed,
    Failed {
        dto: RemovalFailureDto,
        target_removed: bool,
    },
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
    RemovalPreview {
        target: LibraryTarget,
        reply: Reply<Transition<RemovalPreviewDto>>,
    },
    RemoveLibrary {
        target: LibraryTarget,
        reply: Reply<RemovalOutcome>,
    },
    Deselect(Reply<Transition<StateDto>>),
    Grade {
        grade: crate::scheduler::Grade,
        expected: u64,
        reply: Reply<Option<StateDto>>,
    },
    Skip {
        expected: u64,
        reply: Reply<Option<StateDto>>,
    },
    Introduce {
        expected: u64,
        reply: Reply<Option<StateDto>>,
    },
    Restart {
        expected: u64,
        reply: Reply<Option<StateDto>>,
    },
    Check {
        lines: Vec<String>,
        expected: u64,
        reply: Reply<Feedback<review::CheckFeedback>>,
    },
    Choose {
        index: usize,
        card: String,
        expected: u64,
        reply: Reply<Feedback<review::ChoiceFeedback>>,
    },
    ChooseMulti {
        indices: Vec<usize>,
        card: String,
        expected: u64,
        reply: Reply<Feedback<review::MultiChoiceFeedback>>,
    },
    Remove {
        expected: u64,
        reply: Reply<Option<StateDto>>,
    },
    AskPoll(Reply<Option<AskDto>>),
    AskCreate {
        req: CreateCardReq,
        expected: u64,
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
        expected: u64,
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
    Projection(Reply<StudyProjection>),
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
    pub(super) fn removal_preview(
        &self,
        target: LibraryTarget,
    ) -> Option<Transition<RemovalPreviewDto>> {
        self.call(|reply| StudyCommand::RemovalPreview { target, reply })
    }
    pub(super) fn remove_library(&self, target: LibraryTarget) -> Option<RemovalOutcome> {
        self.call(|reply| StudyCommand::RemoveLibrary { target, reply })
    }
    pub(super) fn deselect(&self) -> Option<Transition<StateDto>> {
        self.call(StudyCommand::Deselect)
    }
    pub(super) fn grade(
        &self,
        grade: crate::scheduler::Grade,
        expected: u64,
    ) -> Option<Option<StateDto>> {
        self.call(|reply| StudyCommand::Grade {
            grade,
            expected,
            reply,
        })
    }
    pub(super) fn skip(&self, expected: u64) -> Option<Option<StateDto>> {
        self.call(|reply| StudyCommand::Skip { expected, reply })
    }
    pub(super) fn introduce(&self, expected: u64) -> Option<Option<StateDto>> {
        self.call(|reply| StudyCommand::Introduce { expected, reply })
    }
    pub(super) fn restart(&self, expected: u64) -> Option<Option<StateDto>> {
        self.call(|reply| StudyCommand::Restart { expected, reply })
    }
    pub(super) fn check(
        &self,
        lines: Vec<String>,
        expected: u64,
    ) -> Option<Feedback<review::CheckFeedback>> {
        self.call(|reply| StudyCommand::Check {
            lines,
            expected,
            reply,
        })
    }
    pub(super) fn choose(
        &self,
        index: usize,
        card: String,
        expected: u64,
    ) -> Option<Feedback<review::ChoiceFeedback>> {
        self.call(|reply| StudyCommand::Choose {
            index,
            card,
            expected,
            reply,
        })
    }
    pub(super) fn choose_multi(
        &self,
        indices: Vec<usize>,
        card: String,
        expected: u64,
    ) -> Option<Feedback<review::MultiChoiceFeedback>> {
        self.call(|reply| StudyCommand::ChooseMulti {
            indices,
            card,
            expected,
            reply,
        })
    }
    pub(super) fn remove(&self, expected: u64) -> Option<Option<StateDto>> {
        self.call(|reply| StudyCommand::Remove { expected, reply })
    }
    pub(super) fn ask_start(
        &self,
        action: Option<AskAction>,
        ask_cfg: crate::config::AskConfig,
        expected: u64,
    ) -> Option<Option<AskDto>> {
        self.call(|reply| StudyCommand::TutorStart {
            action,
            ask_cfg,
            expected,
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
    pub(super) fn ask_create(&self, req: CreateCardReq, expected: u64) -> Option<CreateOutcome> {
        self.call(|reply| StudyCommand::AskCreate {
            req,
            expected,
            reply,
        })
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
    pub(super) fn projection(&self) -> Option<StudyProjection> {
        self.call(StudyCommand::Projection)
    }
}

pub(super) fn spawn(
    failure: super::OwnerFailure,
    state: StudyState,
) -> (StudyHandle, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let handle = super::supervised(failure, move || run(state, rx));
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
pub(super) fn flush_store(
    store: &Store,
    dirty: &mut bool,
    save_error: &mut Option<String>,
) -> bool {
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

impl StudyState {
    /// The listing-time health check: repaired or removed damaged documents
    /// heal, and damage arriving after boot reds out, both on the next
    /// listing. Only the idle owner reopens (an active session's store is
    /// never replaced), only when the progress directory's stat stamp
    /// actually moved, and through the same flush + tolerant reopen as
    /// session teardown, so healthy and dirty state keep their
    /// owner-projection guarantees. A quiet directory costs one stat scan
    /// per listing, never a read.
    fn revalidate_progress_view(&mut self) {
        if !self.idle() || !self.store.is_aggregate() {
            return;
        }
        let stamp = crate::store::progress_dir_stamp(self.store.path());
        if self.progress_stamp == Some(stamp) {
            return;
        }
        if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error) {
            return;
        }
        if let Ok(store) = assemble::store_for(&[], self.config.cfg.instance_store.as_deref()) {
            self.install_store(store);
            self.writes = self.writes.wrapping_add(1);
            self.progress_stamp = Some(crate::store::progress_dir_stamp(self.store.path()));
        }
    }

    fn install_store(&mut self, mut store: Store) {
        store.carry_failed_decks(&self.store);
        let outgoing = std::mem::replace(&mut self.store, store);
        self.retained
            .insert(outgoing.path().to_path_buf(), Arc::new(outgoing));
        let active = self.store.path().to_path_buf();
        self.retained.remove(&active);
    }

    fn review_dto(&self) -> StateDto {
        review_state(
            self.reviewing.as_ref(),
            &self.store,
            self.save_error.as_deref(),
            self.revision,
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
                        self.writes = self.writes.wrapping_add(1);
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
            StudyCommand::RemovalPreview { target, reply } => {
                let _ = reply.send(self.removal_preview(target));
            }
            StudyCommand::RemoveLibrary { target, reply } => {
                let _ = reply.send(self.remove_library(target));
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
                        self.install_store(s);
                        self.writes = self.writes.wrapping_add(1);
                    }
                    self.revision += 1;
                    Transition::Done(self.review_dto())
                };
                let _ = reply.send(out);
            }
            StudyCommand::Grade {
                grade,
                expected,
                reply,
            } => {
                let out = if self.revision != expected {
                    None
                } else {
                    self.grade(grade)
                };
                let _ = reply.send(out);
            }
            StudyCommand::Skip { expected, reply } => {
                let dto = match self.reviewing.as_mut() {
                    _ if self.revision != expected => None,
                    None => None,
                    Some(r) => {
                        r.session.skip(&mut self.store, now_ms());
                        self.writes = self.writes.wrapping_add(1);
                        r.rotate_variant();
                        self.revision += 1;
                        Some(())
                    }
                }
                .map(|()| self.review_dto());
                let _ = reply.send(dto);
            }
            StudyCommand::Introduce { expected, reply } => {
                let dto = match self.reviewing.as_mut() {
                    _ if self.revision != expected => None,
                    None => None,
                    Some(r) => {
                        r.session.introduce_current(&mut self.store, now_ms());
                        flush_mutation(&self.store, &mut self.store_dirty, &mut self.save_error);
                        self.writes = self.writes.wrapping_add(1);
                        r.rotate_variant();
                        self.revision += 1;
                        Some(())
                    }
                }
                .map(|()| self.review_dto());
                let _ = reply.send(dto);
            }
            StudyCommand::Restart { expected, reply } => {
                let dto = match self.reviewing.as_mut() {
                    _ if self.revision != expected => None,
                    None => None,
                    Some(r) => {
                        r.session.restart(&mut self.store, now_ms());
                        self.writes = self.writes.wrapping_add(1);
                        r.rotate_variant();
                        self.revision += 1;
                        Some(())
                    }
                }
                .map(|()| self.review_dto());
                let _ = reply.send(dto);
            }
            StudyCommand::Check {
                lines,
                expected,
                reply,
            } => {
                let out = match self.reviewing.as_ref() {
                    _ if self.revision != expected => Feedback::NoSession,
                    None => Feedback::NoSession,
                    Some(r) => match review::check_typed(&r.session, &lines) {
                        Some(f) => Feedback::Ok(f),
                        None => Feedback::Bad,
                    },
                };
                let _ = reply.send(out);
            }
            StudyCommand::Choose {
                index,
                card,
                expected,
                reply,
            } => {
                let out = match self.reviewing.as_ref() {
                    _ if self.revision != expected => Feedback::NoSession,
                    None => Feedback::NoSession,
                    Some(r) if r.session.current_id().as_deref() != Some(card.as_str()) => {
                        Feedback::NoSession
                    }
                    Some(r) => match review::choose(&r.session, &self.store, &r.augment, index) {
                        Some(f) => Feedback::Ok(f),
                        None => Feedback::Bad,
                    },
                };
                let _ = reply.send(out);
            }
            StudyCommand::ChooseMulti {
                indices,
                card,
                expected,
                reply,
            } => {
                let out = match self.reviewing.as_ref() {
                    _ if self.revision != expected => Feedback::NoSession,
                    None => Feedback::NoSession,
                    Some(r) if r.session.current_id().as_deref() != Some(card.as_str()) => {
                        Feedback::NoSession
                    }
                    Some(r) => {
                        match review::choose_multi(&r.session, &self.store, &r.augment, &indices) {
                            Some(f) => Feedback::Ok(f),
                            None => Feedback::Bad,
                        }
                    }
                };
                let _ = reply.send(out);
            }
            StudyCommand::Remove { expected, reply } => {
                let out = if self.revision != expected {
                    None
                } else {
                    self.remove()
                };
                let _ = reply.send(out);
            }
            StudyCommand::TutorStart {
                action,
                ask_cfg,
                expected,
                reply,
            } => {
                let audience = self.config.audience;
                let stale = self.revision != expected;
                let dto = self.reviewing.as_mut().filter(|_| !stale).map(|r| {
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
                        self.install_store(s);
                        self.writes = self.writes.wrapping_add(1);
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
            StudyCommand::AskCreate {
                req,
                expected,
                reply,
            } => {
                let out = if self.revision != expected {
                    CreateOutcome::NoSession
                } else {
                    self.ask_create(req)
                };
                let _ = reply.send(out);
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
                        let poll = ex
                            .sitting
                            .poll(&mut self.store, now_ms(), retire_after_days);
                        if poll.store_mutated {
                            flush_mutation(
                                &self.store,
                                &mut self.store_dirty,
                                &mut self.save_error,
                            );
                            self.writes = self.writes.wrapping_add(1);
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
                        self.install_store(s);
                        self.writes = self.writes.wrapping_add(1);
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
                                self.writes = self.writes.wrapping_add(1);
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
                        self.install_store(s);
                        self.writes = self.writes.wrapping_add(1);
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
                self.revalidate_progress_view();
                let _ = reply.send(StudyProjection {
                    store: Arc::new(self.store.clone()),
                    retained: self.retained.clone(),
                    writes: self.writes,
                });
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
        let candidate = assemble::store_for(&files, self.config.cfg.instance_store.as_deref()).ok();
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
        if let Some(s) = candidate {
            self.install_store(s);
            self.writes = self.writes.wrapping_add(1);
        }
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
        let mut candidate =
            match assemble::store_for(&paths, self.config.cfg.instance_store.as_deref()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("warning: could not open the progress store: {e}");
                    return Transition::Rejected;
                }
            };
        let recorded_paths = paths.clone();
        match assemble::select(paths, &mut candidate, &self.config.cfg, &opts) {
            Ok(assemble::Selected::Walk(wb)) => {
                self.install_store(candidate);
                self.writes = self.writes.wrapping_add(1);
                let w = Walking::new(wb.walk, wb.grade);
                let dto = walk_dto(&w);
                self.walking = Some(w);
                self.reviewing = None;
                self.examining = None;
                self.revision += 1;
                Transition::Done((SelectedDto::Walk(Box::new(dto)), None))
            }
            Ok(assemble::Selected::Review(b)) => {
                self.install_store(candidate);
                self.writes = self.writes.wrapping_add(1);
                let record = (!b.session.is_finished()).then_some(recorded_paths);
                let mut r = Reviewing::new(b);
                // `assemble::select` already saved the store, stamp included.
                r.rotate_variant();
                self.reviewing = Some(r);
                self.walking = None;
                self.revision += 1;
                Transition::Done((SelectedDto::Review(Box::new(self.review_dto())), record))
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
        let candidate = match assemble::store_for(&paths, self.config.cfg.instance_store.as_deref())
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: could not open the progress store: {e}");
                return Transition::Rejected;
            }
        };
        let recorded_paths = paths.clone();
        match assemble::browse(paths, self.config.cfg.instance_store.as_deref()) {
            Ok(b) => {
                self.install_store(candidate);
                self.writes = self.writes.wrapping_add(1);
                self.browsing = Some(Browsing::new(b));
                self.reviewing = None;
                self.walking = None;
                self.examining = None;
                self.revision += 1;
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

    fn removal_preview(&mut self, target: LibraryTarget) -> Transition<RemovalPreviewDto> {
        if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error) {
            return Transition::FlushFailed;
        }
        let Ok(store) = self.library_store(&target) else {
            return Transition::Rejected;
        };
        let dto = match &target {
            LibraryTarget::Deck { path, .. } => {
                let preview = crate::library::removal_preview(path, &store);
                RemovalPreviewDto {
                    target: target.name().to_string(),
                    kind: target.kind(),
                    decks: 1,
                    cards_with_progress: preview.cards_with_progress,
                    earliest_review_ms: preview.earliest_review_ms,
                    files: target.labels(&preview.files),
                    directories: target.labels(&preview.directories),
                    dependents: preview.dependents,
                }
            }
            LibraryTarget::Workspace { root, .. } => {
                let Ok(preview) = crate::library::workspace_removal_preview(root, &store) else {
                    return Transition::Rejected;
                };
                RemovalPreviewDto {
                    target: target.name().to_string(),
                    kind: target.kind(),
                    decks: preview.decks,
                    cards_with_progress: preview.cards_with_progress,
                    earliest_review_ms: preview.earliest_review_ms,
                    files: target.labels(&preview.files),
                    directories: target.labels(&preview.directories),
                    dependents: preview.dependents,
                }
            }
        };
        Transition::Done(dto)
    }

    fn remove_library(&mut self, target: LibraryTarget) -> RemovalOutcome {
        if !self.idle() {
            return RemovalOutcome::Busy;
        }
        if !flush_store(&self.store, &mut self.store_dirty, &mut self.save_error) {
            return RemovalOutcome::FlushFailed;
        }
        let Ok(store) = self.library_store(&target) else {
            return RemovalOutcome::Rejected;
        };
        let store_path = store.path().to_path_buf();
        let covered_stores = target.covered_store_paths(&store_path);
        let result = match &target {
            LibraryTarget::Deck { path, .. } => {
                crate::library::remove_deck(path, &store).map(|report| RemovalDto {
                    target: target.name().to_string(),
                    kind: target.kind(),
                    removed: target.labels(&report.removed),
                    decks_removed: 1,
                    directory_removed: false,
                    dependents: report.dependents,
                })
            }
            LibraryTarget::Workspace { root, .. } => crate::library::remove_workspace(root, &store)
                .map(|report| RemovalDto {
                    target: target.name().to_string(),
                    kind: target.kind(),
                    removed: target.labels(&report.removed),
                    decks_removed: report.decks_removed,
                    directory_removed: report.root_removed,
                    dependents: report.dependents,
                }),
        };
        match result {
            Ok(dto) => {
                self.finish_library_removal(&covered_stores, &store_path);
                RemovalOutcome::Done(dto)
            }
            Err(error) => {
                eprintln!("warning: library removal failed: {error:#}");
                let Some(failure) = error.downcast_ref::<crate::library::RemovalFailure>() else {
                    return RemovalOutcome::Rejected;
                };
                let dto = RemovalFailureDto {
                    target: target.name().to_string(),
                    error: "removal incomplete",
                    completed: target.labels(&failure.removed),
                    failed: target.label(&failure.failed),
                    recovery: "Run alix doctor to inspect and repair the remaining artifacts.",
                };
                let target_removed = match &target {
                    LibraryTarget::Deck { path, .. } => failure.removed.contains(path),
                    LibraryTarget::Workspace { root, .. } => failure
                        .removed
                        .contains(&crate::workspace::WorkspaceFiles::new(root).manifest()),
                };
                self.finish_library_removal(&covered_stores, &store_path);
                RemovalOutcome::Failed {
                    dto,
                    target_removed,
                }
            }
        }
    }

    fn idle(&self) -> bool {
        self.reviewing.is_none()
            && self.browsing.is_none()
            && self.examining.is_none()
            && self.walking.is_none()
            && self.augmenting.is_none()
    }

    fn library_store(&self, target: &LibraryTarget) -> anyhow::Result<Store> {
        match target {
            LibraryTarget::Deck { path, .. } => {
                let active = crate::library::removal_preview(path, &self.store)
                    .files
                    .iter()
                    .any(|artifact| artifact == self.store.path());
                if active {
                    Ok(self.store.clone())
                } else {
                    assemble::store_for(
                        std::slice::from_ref(path),
                        self.config.cfg.instance_store.as_deref(),
                    )
                }
            }
            LibraryTarget::Workspace { root, members, .. } => {
                crate::state::open_stores(members, &workspace::store_path(root))
                    .map_err(anyhow::Error::from)
            }
        }
    }

    fn finish_library_removal(&mut self, covered_stores: &[PathBuf], store_path: &Path) {
        if let Ok(store) = assemble::store_for(&[], self.config.cfg.instance_store.as_deref()) {
            self.install_store(store);
        }
        let progress_root = progress_root_for_store(store_path);
        self.retained.retain(|path, _| {
            !covered_stores.contains(path) && progress_root.as_ref() != Some(path)
        });
        self.writes = self.writes.wrapping_add(1);
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
        let Ok(mut scoped) = assemble::store_for(&paths, self.config.cfg.instance_store.as_deref())
        else {
            return Transition::Rejected;
        };
        let Ok(cleared) = crate::library::reset_decks(&mut scoped, decks.iter()) else {
            return Transition::Rejected;
        };
        if let Ok(s) = assemble::store_for(&[], self.config.cfg.instance_store.as_deref()) {
            self.install_store(s);
            self.writes = self.writes.wrapping_add(1);
        }
        // Projections prefer a retained snapshot over disk, so every snapshot
        // covering the store this reset just rewrote must go, or listings
        // keep serving the pre-reset records. A member reset rewrites one
        // document, but a parked WORKSPACE AGGREGATE covers it from the
        // progress directory above it, so both keys are evicted.
        let progress_root = progress_root_for_store(scoped.path());
        self.retained
            .retain(|path, _| path != scoped.path() && progress_root.as_deref() != Some(path));
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
            store::record_badges(&mut self.store, &deck_id, r.session.cards(), now);
        }
        flush_mutation(&self.store, &mut self.store_dirty, &mut self.save_error);
        self.writes = self.writes.wrapping_add(1);
        r.rotate_variant();
        self.revision += 1;
        Some(self.review_dto())
    }

    fn remove(&mut self) -> Option<StateDto> {
        let r = self.reviewing.as_mut()?;
        let dropped = r.session.remove_current(&mut self.store, now_ms());
        if let Some(first) = dropped.first() {
            let deck_id = first.deck_id.to_string();
            let line = first.line;
            let region_lines = first.region.as_ref().map(|slot| slot.directive_lines());
            for card in &dropped {
                if let Some(id) = card.id() {
                    self.store.remove(&id);
                }
            }
            flush_mutation(&self.store, &mut self.store_dirty, &mut self.save_error);
            self.writes = self.writes.wrapping_add(1);
            match region_lines {
                Some(lines) => r.files.remove_region_lines(&deck_id, &lines),
                None => r.files.remove_block(&deck_id, line),
            }
        }
        self.writes = self.writes.wrapping_add(1);
        self.revision += 1;
        Some(self.review_dto())
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
        let Some(deck_path) = r.files.paths.get(&deck_id).cloned() else {
            return CreateOutcome::NoSession;
        };
        // Dedup by content fingerprint, not id: a mint carries a fresh random
        // token, so identical content still collides.
        let deck_fingerprints: std::collections::HashSet<u64> = r
            .session
            .cards()
            .iter()
            .map(|c| c.block_fingerprint)
            .collect();
        let now = now_ms();
        match store::mint_tutor_card(
            &mut self.store,
            &deck_path,
            &deck_id,
            &req.front,
            &req.back,
            now,
            &deck_fingerprints,
        ) {
            Ok(id) => {
                flush_mutation(&self.store, &mut self.store_dirty, &mut self.save_error);
                self.writes = self.writes.wrapping_add(1);
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
        // Validate against a candidate store; the active store is replaced
        // only once the sitting definitely starts, so every rejection path
        // leaves the current session writing to its own document.
        let Ok(candidate) = assemble::store_for(
            std::slice::from_ref(&path),
            self.config.cfg.instance_store.as_deref(),
        ) else {
            return Transition::Rejected;
        };
        match Deck::load(&path) {
            Ok(deck)
                if deck.has_exam()
                    && !deck::is_locked(&deck, Some(decks_root.as_path()), &candidate) =>
            {
                let strictness = deck
                    .settings
                    .exam_strictness
                    .unwrap_or(self.config.exam_cfg.strictness);
                let sitting = if deck.is_trace() {
                    match trace::Trace::from_deck(&deck) {
                        Ok(t) => {
                            if let Some(ms) = exam::cooldown_remaining_ms(
                                &candidate,
                                deck.deck_token.as_deref().unwrap_or_default(),
                                self.config.exam_cfg.retry_cooldown_secs,
                                now_ms(),
                            ) {
                                // One response shape per endpoint: the cooldown
                                // is an ExamDto phase, not untagged.
                                return Transition::Done(Box::new(cooldown_dto(&deck.subject, ms)));
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
                self.install_store(candidate);
                self.writes = self.writes.wrapping_add(1);
                let dto = exam_dto(&ex);
                self.examining = Some(ex);
                Transition::Done(Box::new(dto))
            }
            _ => Transition::Rejected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_fixture(
        dir: &Path,
        depth: crate::depth::Depth,
    ) -> (Reviewing, Store, String, String) {
        let deck_id = "deck-studytest".to_string();
        let card_id = "card-studytest".to_string();
        let deck = dir.join("study.md");
        std::fs::write(&deck, "## question <!-- id: card-studytest -->\nanswer\n").unwrap();
        let mut card = crate::card::Card::plain(
            std::sync::Arc::from("study.md"),
            "question".to_string(),
            vec!["answer".to_string()],
            None,
            1,
        );
        card.token = Some(std::sync::Arc::from(card_id.as_str()));
        card.deck_id = std::sync::Arc::from(deck_id.as_str());
        let mut store = Store::open(dir.join("progress.json")).unwrap();
        if depth == crate::depth::Depth::Recognize {
            store.get_or_insert(&card_id).introduced_ms = Some(0);
        }
        let session = crate::session::Session::new(
            vec![card],
            &mut store,
            Box::new(crate::scheduler::Fsrs::default()),
            crate::session::SessionOptions {
                depth,
                ..Default::default()
            },
            now_ms(),
        );
        let mut decks = HashMap::new();
        decks.insert(deck_id.clone(), deck.clone());
        let reviewing = Reviewing::new(super::super::SessionBuild {
            session,
            label: "study.md".to_string(),
            decks,
            load_warnings: Vec::new(),
            links: HashMap::new(),
            source_layers: HashMap::new(),
            base_roots: HashMap::new(),
            source_bases: HashMap::new(),
            topology_name: None,
            augment: AugmentCache::open(deck.with_extension("generated.json")),
        });
        (reviewing, store, card_id, deck_id)
    }

    fn study_state_with_review(dir: &Path, depth: crate::depth::Depth) -> (StudyState, String) {
        let (reviewing, store, _card_id, deck_id) = review_fixture(dir, depth);
        let defaults = crate::config::Config::default();
        let store_path = store.path().to_path_buf();
        (
            StudyState {
                config: StudyConfig {
                    cfg: AssembleConfig {
                        review: defaults.review,
                        ask: defaults.ask,
                        trace_auto_grade: false,
                        pacing: assemble::Pacing {
                            max_session: 10,
                            new_cards_percent: 30,
                        },
                        instance_store: Some(store_path),
                    },
                    exam_cfg: defaults.exam,
                    review_cfg: defaults.review,
                    audience: defaults.serve.audience,
                },
                store,
                retained: HashMap::new(),
                store_dirty: false,
                progress_stamp: None,
                save_error: None,
                reviewing: Some(reviewing),
                revision: 0,
                writes: 0,
                browsing: None,
                examining: None,
                walking: None,
                augmenting: None,
            },
            deck_id,
        )
    }

    #[test]
    fn a_closed_owner_returns_transport_failure_not_default_query_values() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let handle = StudyHandle { tx };

        assert!(handle.store_path().is_none());
        assert!(handle.exam_remediate().is_none());
    }

    #[test]
    fn tutor_create_rejects_an_exact_copy_of_the_current_cloze_block() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, deck_id) = study_state_with_review(dir.path(), crate::depth::Depth::Recall);
        let deck = dir.path().join("study.md");
        let text = "## Complete the sentence\nThe capital is \\blank{Paris}\n<!-- id: card-studytest -->\n";
        std::fs::write(&deck, text).unwrap();
        let cards = crate::parser::parse_str(&deck_id, text).unwrap();
        let session = crate::session::Session::new(
            cards,
            &mut state.store,
            Box::new(crate::scheduler::Fsrs::default()),
            crate::session::SessionOptions::default(),
            now_ms(),
        );
        state.reviewing.as_mut().unwrap().session = session;

        let outcome = state.ask_create(CreateCardReq {
            front: "Complete the sentence".to_string(),
            back: vec!["The capital is \\blank{Paris}".to_string()],
        });

        assert!(
            matches!(outcome, CreateOutcome::Invalid),
            "the exact authored cloze block must be rejected as a duplicate"
        );
        assert!(
            !crate::personal::sidecar_path(&deck).exists(),
            "a duplicate must not create a personal file"
        );
    }

    #[test]
    fn a_single_recognition_pass_no_longer_earns_the_deck_badge() {
        // Under ADR 0033 the recognize badge means mature-at-recognize, the
        // same bar the other depths always had; one pass is a learning-state
        // schedule far below it. The badge write path itself is exercised
        // through `record_badges`, whose maturity laws live in store tests.
        let dir = tempfile::tempdir().unwrap();
        let (mut state, deck_id) =
            study_state_with_review(dir.path(), crate::depth::Depth::Recognize);
        assert_eq!(
            None,
            state
                .store
                .badge_earned(&deck_id, crate::depth::Depth::Recognize)
        );

        assert!(state.grade(crate::scheduler::Grade::Pass).is_some());

        assert_eq!(
            None,
            state
                .store
                .badge_earned(&deck_id, crate::depth::Depth::Recognize),
            "one pass is not maturity; the flag-era instant badge is gone"
        );
    }

    #[test]
    fn progress_store_roots_cover_only_the_aggregate_and_its_direct_documents() {
        let user = Path::new("user");
        let progress = user.join("progress");

        assert_eq!(Some(progress.clone()), progress_root_for_store(&progress));
        assert_eq!(
            Some(progress.clone()),
            progress_root_for_store(&progress.join("deck-example.json"))
        );
        assert_eq!(
            None,
            progress_root_for_store(&user.join("snapshots/deck-example.json"))
        );
    }

    #[test]
    fn finishing_removal_drops_only_covered_snapshots_and_their_aggregate() {
        let dir = tempfile::tempdir().unwrap();
        let active_root = dir.path().join("active");
        let removed_root = dir.path().join("removed/progress");
        let covered = removed_root.join("deck-covered.json");
        let unrelated = dir.path().join("other/progress/deck-unrelated.json");
        let defaults = crate::config::Config::default();
        let mut state = StudyState {
            config: StudyConfig {
                cfg: AssembleConfig {
                    review: defaults.review,
                    ask: defaults.ask,
                    trace_auto_grade: false,
                    pacing: assemble::Pacing {
                        max_session: 10,
                        new_cards_percent: 30,
                    },
                    instance_store: Some(active_root.clone()),
                },
                exam_cfg: defaults.exam,
                review_cfg: defaults.review,
                audience: defaults.serve.audience,
            },
            store: crate::state::open_aggregate_store(&active_root).unwrap(),
            retained: HashMap::new(),
            store_dirty: false,
            progress_stamp: None,
            save_error: None,
            reviewing: None,
            revision: 0,
            writes: 0,
            browsing: None,
            examining: None,
            walking: None,
            augmenting: None,
        };
        let snapshot =
            Arc::new(crate::state::open_aggregate_store(&dir.path().join("snapshot")).unwrap());
        state
            .retained
            .insert(covered.clone(), Arc::clone(&snapshot));
        state
            .retained
            .insert(removed_root.clone(), Arc::clone(&snapshot));
        state
            .retained
            .insert(unrelated.clone(), Arc::clone(&snapshot));

        state.finish_library_removal(std::slice::from_ref(&covered), &covered);

        assert!(!state.retained.contains_key(&covered));
        assert!(!state.retained.contains_key(&removed_root));
        assert!(state.retained.contains_key(&unrelated));
    }
}
