//! The mobile review surface: an opaque handle around a live alix session,
//! its store, and its augment cache. Dart holds the handle and calls into it;
//! all review logic stays in the embedded core (`alix::review`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

pub use alix::answer::{Input, Mode, TypedResult};
pub use alix::depth::Depth;
pub use alix::render::NoteUnit;
pub use alix::review::{CardView, CheckFeedback, ChoiceFeedback, ReviewState};
/// Renamed on re-export so the walk's own mirror (below) reads as its own
/// concept rather than a bare "Phase" borrowed from the review vocabulary.
pub use alix::trace::Phase as WalkPhase;

/// frb mirrors of the core contract types (they live in the `alix` crate,
/// which frb does not scan): field-for-field copies that teach the generator
/// their shape so Dart gets real classes and enums, not opaque handles. Keep
/// in lock step with `alix::review`, `alix::answer`, and `alix::depth`.
#[flutter_rust_bridge::frb(mirror(Mode))]
pub enum _Mode {
    Flip,
    Typing,
    TypeLine,
    Choice,
    LineByLine,
    Explain,
}

#[flutter_rust_bridge::frb(mirror(Depth))]
pub enum _Depth {
    Recognize,
    Recall,
    Reconstruct,
}

#[flutter_rust_bridge::frb(mirror(Input))]
pub enum _Input {
    Type,
    Draw,
}

#[flutter_rust_bridge::frb(mirror(NoteUnit))]
pub enum _NoteUnit {
    Sentence { text: String },
    Code { lines: Vec<String> },
}

#[flutter_rust_bridge::frb(mirror(CardView))]
pub struct _CardView {
    pub front: String,
    pub context: Vec<String>,
    pub back: Vec<String>,
    pub reshaped: bool,
    pub note: Vec<NoteUnit>,
    pub image: Option<String>,
    pub image_back: Option<String>,
    pub at: Option<String>,
}

#[flutter_rust_bridge::frb(mirror(ReviewState))]
pub struct _ReviewState {
    pub card: Option<CardView>,
    pub mode: Mode,
    pub depth: Depth,
    pub acquire: bool,
    pub choices: Option<Vec<String>>,
    pub keypoints: Option<Vec<String>>,
    pub input: Input,
    pub finished: bool,
    pub remaining: u32,
    pub initial: u32,
    pub reviews: u32,
    pub passed: u32,
    pub failed: u32,
    pub acquired: u32,
    pub can_restart: bool,
    pub promotable: bool,
}

#[flutter_rust_bridge::frb(mirror(ChoiceFeedback))]
pub struct _ChoiceFeedback {
    pub chosen: usize,
    pub correct: usize,
    pub passed: bool,
}

#[flutter_rust_bridge::frb(mirror(TypedResult))]
pub struct _TypedResult {
    pub input: String,
    pub expected: String,
    pub passed: bool,
}

#[flutter_rust_bridge::frb(mirror(CheckFeedback))]
pub struct _CheckFeedback {
    pub results: Vec<TypedResult>,
    pub passed: bool,
}

#[flutter_rust_bridge::frb(mirror(WalkPhase))]
pub enum _WalkPhase {
    Predict,
    Reveal,
    Done,
}

/// The learner's self-grade, mirrored so frb bridges it from this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    Fail,
    Partial,
    Pass,
}

impl From<Grade> for alix::scheduler::Grade {
    fn from(g: Grade) -> Self {
        match g {
            Grade::Fail => alix::scheduler::Grade::Fail,
            Grade::Partial => alix::scheduler::Grade::Partial,
            Grade::Pass => alix::scheduler::Grade::Pass,
        }
    }
}

impl From<alix::scheduler::Grade> for Grade {
    fn from(g: alix::scheduler::Grade) -> Self {
        match g {
            alix::scheduler::Grade::Fail => Grade::Fail,
            alix::scheduler::Grade::Partial => Grade::Partial,
            alix::scheduler::Grade::Pass => Grade::Pass,
        }
    }
}

/// The learner's self-judged trace-walk delta, mirrored the same way
/// [`Grade`] is: `alix::trace::Delta` lives in the core crate frb doesn't
/// scan, so this is a field-for-field bridge copy with explicit conversions
/// both ways, not a `#[frb(mirror(..))]` teaching shim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkDelta {
    Missed,
    Partly,
    Got,
}

impl From<WalkDelta> for alix::trace::Delta {
    fn from(d: WalkDelta) -> Self {
        match d {
            WalkDelta::Missed => alix::trace::Delta::Failed,
            WalkDelta::Partly => alix::trace::Delta::Partial,
            WalkDelta::Got => alix::trace::Delta::Passed,
        }
    }
}

impl From<alix::trace::Delta> for WalkDelta {
    fn from(d: alix::trace::Delta) -> Self {
        match d {
            alix::trace::Delta::Failed => WalkDelta::Missed,
            alix::trace::Delta::Partial => WalkDelta::Partly,
            alix::trace::Delta::Passed => WalkDelta::Got,
        }
    }
}

/// The Explain checklist tally as a grade: none covered fails, all pass,
/// some is a partial. The rule lives in core (`scheduler::keypoint_grade`);
/// this is only the bridge.
#[flutter_rust_bridge::frb(sync)]
pub fn keypoint_grade(covered: u32, total: u32) -> Grade {
    alix::scheduler::keypoint_grade(covered as usize, total as usize).into()
}

/// Test/dev support: cache a full set of choice distractors on every card of
/// `deck_path`, into the same augment sidecar the review session reads (routed
/// like [`ReviewSession::open`]). The phone never generates distractors itself
/// — they sync from the desktop's `alix deck augment` — so a widget or bridge
/// test uses this to drive the pick-only Recognize path without a live
/// augmentation, keeping card-id computation in the lib (never in Dart).
#[flutter_rust_bridge::frb(sync)]
pub fn seed_choice_distractors(deck_path: String, root_dir: String) -> Result<()> {
    let deck_pb = PathBuf::from(&deck_path);
    let deck = alix::deck::Deck::load(&deck_pb)?;
    let root_store = alix::workspace::root_store_path(Path::new(&root_dir));
    let store = alix::assemble::store_for(std::slice::from_ref(&deck_pb), Some(&root_store))?;
    let mut cache =
        alix::augment::AugmentCache::open(alix::augment::augment_path_for(store.path()));
    for card in &deck.cards {
        if let Some(id) = card.id() {
            cache.set_distractors(
                &id,
                vec!["one".to_string(), "two".to_string(), "three".to_string()],
            );
        }
    }
    cache.save()?;
    Ok(())
}

/// Another device's recent write of this session's store (see
/// [`ReviewSession::foreign_writer`]): the roaming-discipline banner's data.
pub struct ForeignWriter {
    /// The other device's label.
    pub device: String,
    /// How long ago it wrote, in ms.
    pub age_ms: u64,
}

/// The current card's fields exactly as authored, for the remote tutor to
/// ground its answer on, never the masked [`CardView`] a cloze review
/// renders (its `context` blanks the hole under review; the tutor needs the
/// real text). See [`ReviewSession::tutor_card`].
pub struct TutorCard {
    pub subject: String,
    pub front: String,
    pub back: Vec<String>,
    pub at: Option<String>,
    /// The 1-based deck-file line of this card's front (`Card::line`): the
    /// append anchor [`ReviewSession::apply_card_note`] targets when the
    /// tutor's condensed note comes back.
    pub line: usize,
}

/// The "where am I" region breadcrumb for a topology-ordered session: the
/// topology's region names in walk order, the index of the current card's
/// region, and each region's per-card strength for a heatmap bar. Mirrors
/// the web's `CrumbDto` (`src/serve/dto.rs`). See [`ReviewSession::crumb`].
pub struct CrumbState {
    pub regions: Vec<String>,
    pub current: u32,
    /// Per-region, per-card strength (`0..=1`, outer index aligns with
    /// `regions`), red (weak) to green (strong).
    pub cells: Vec<Vec<f32>>,
}

/// A live review session running in Rust: the alix session plus its open
/// store and augment cache. Dart holds this as an opaque handle.
pub struct ReviewSession {
    session: alix::session::Session,
    store: alix::store::Store,
    augment: alix::augment::AugmentCache,
    /// The resolved topology name when this session is topology-ordered
    /// (`assemble::SessionBuild::topology_name`), else `None`. Captured at
    /// open time so [`ReviewSession::crumb`] can pick this session's own
    /// topology out of a (possibly shared) augment cache.
    topology_name: Option<String>,
    /// This deck's own file path, captured at open time so
    /// [`ReviewSession::apply_card_note`] can append to the right file
    /// without the caller passing it again.
    deck_path: PathBuf,
    /// The deck's file-name subject exactly as the lib derived it when this
    /// deck's cards were parsed (`Card::id` hashes it). Captured straight off
    /// the loaded `Deck`, never re-derived from `deck_path` by hand: a
    /// hand-derived subject that differs even by extension or case silently
    /// yields DIFFERENT ids, so dedup stops deduping and progress forks.
    subject: String,
    /// This deck's own cards' §7 content fingerprints at open time: the dedup
    /// baseline for remediation (mirrors `exam::Sitting::deck_fingerprints`,
    /// captured the same way, off a freshly loaded `Deck`, not the live session
    /// roster). Content, not id: a fresh remediation/tutor card carries a new
    /// random token, so it can only be recognized by content.
    deck_fingerprints: HashSet<u64>,
    /// Whether this deck sits an AI exam (`Deck::has_exam`, lean and
    /// canonical). `ReviewSession` only ever opens a non-trace deck (a lone
    /// trace walks instead, via [`WalkSession`]), so in practice this is
    /// exactly "has a `% source:`".
    has_exam: bool,
}

impl ReviewSession {
    /// Open a deck of the decks folder `root_dir` at `depth` (default:
    /// the deck's last depth, else Recall). The progress store is routed the
    /// way the web and CLI route it: a workspace member reviews into its
    /// workspace's own store, everything else into the root's shared store.
    /// `now_ms` injects the session clock (tests); `None` is the wall clock.
    /// `device` names this device in the store's last-writer marker (the
    /// app passes its settings.json label); `None` keeps whatever the core
    /// derived for this machine.
    #[flutter_rust_bridge::frb(sync)]
    pub fn open(
        deck_path: String,
        root_dir: String,
        depth: Option<Depth>,
        now_ms: Option<u64>,
        device: Option<String>,
    ) -> Result<ReviewSession> {
        let deck = PathBuf::from(deck_path);
        // The deck's own parse, captured once so the remediation/mint/exam
        // bridge calls below dedup and mark mastery under the SAME subject
        // `assemble::select` derives for the session itself (see the struct
        // fields' docs; a hand-derived subject silently forks progress).
        let loaded = alix::deck::Deck::load(&deck)?;
        let subject = loaded.subject.clone();
        let deck_fingerprints: HashSet<u64> = loaded
            .cards
            .iter()
            .map(|c| alix::l1::content_fingerprint(&c.front, &c.back))
            .collect();
        // The lean, canonical predicate (`Deck::has_exam`, shared with the
        // server and the picker), equivalent here since this session only
        // ever opens a non-trace deck; a lone trace walks and is examined
        // through `WalkSession` instead.
        let has_exam = loaded.has_exam();
        // Captured before `deck` moves into `assemble::select` below.
        let deck_path = deck.clone();

        let root_store = alix::workspace::root_store_path(Path::new(&root_dir));
        let mut store =
            alix::assemble::store_for(std::slice::from_ref(&deck), Some(&root_store))?;
        if device.is_some() {
            store.device = device;
        }
        // The instance config a CLI/server launch would carry, at its built-in
        // defaults (`AssembleConfig` has no `Default`; pacing matches launch.rs).
        let cfg = alix::assemble::AssembleConfig {
            review: alix::config::ReviewConfig::default(),
            ask: alix::config::AskConfig::default(),
            trace_auto_grade: false,
            pacing: alix::assemble::Pacing {
                max_new: 10,
                limit: None,
            },
            instance_store: None,
        };
        let opts = alix::assemble::SelectOptions {
            depth,
            now_ms,
            ..Default::default()
        };
        let selected = alix::assemble::select(vec![deck], &mut store, &cfg, &opts)?;
        let build = match selected {
            alix::assemble::Selected::Review(build) => build,
            alix::assemble::Selected::Walk(_) => {
                bail!("milestone 2 reviews a facts deck, not a trace")
            }
        };
        Ok(ReviewSession {
            session: build.session,
            store,
            augment: build.augment,
            topology_name: build.topology_name,
            deck_path,
            subject,
            deck_fingerprints,
            has_exam,
        })
    }

    /// The current review position, for the screen to render. `now_ms`
    /// injects the clock behind the restartability check (tests); `None` is
    /// the wall clock.
    #[flutter_rust_bridge::frb(sync)]
    pub fn state(&self, now_ms: Option<u64>) -> ReviewState {
        alix::review::state(&self.session, &self.store, &self.augment, now_ms)
    }

    /// Grade a pick against the same options `state` served; `None` when no
    /// pick is up. The learner-final grade is still a separate `grade` call.
    #[flutter_rust_bridge::frb(sync)]
    pub fn choose(&self, chosen: u32) -> Option<ChoiceFeedback> {
        alix::review::choose(&self.session, &self.store, &self.augment, chosen as usize)
    }

    /// Check typed lines against the current card (pure evidence; the
    /// learner-final grade is still a separate `grade` call).
    #[flutter_rust_bridge::frb(sync)]
    pub fn check(&self, lines: Vec<String>) -> Option<CheckFeedback> {
        alix::review::check_typed(&self.session, &lines)
    }

    /// Grade the current card and persist, returning the next position.
    #[flutter_rust_bridge::frb(sync)]
    pub fn grade(&mut self, grade: Grade, now_ms: Option<u64>) -> Result<ReviewState> {
        let now = now_ms.unwrap_or_else(alix::time::now_ms);
        self.session.grade(&mut self.store, grade.into(), now);
        self.store.save()?;
        self.session.poll(&self.store, now);
        Ok(self.state(Some(now)))
    }

    /// Mark the current never-seen card as acquired (first exposure, no
    /// grade) and persist, returning the next position.
    #[flutter_rust_bridge::frb(sync)]
    pub fn acquire(&mut self, now_ms: Option<u64>) -> Result<ReviewState> {
        let now = now_ms.unwrap_or_else(alix::time::now_ms);
        self.session.acquire_current(&mut self.store, now);
        self.store.save()?;
        self.session.poll(&self.store, now);
        Ok(self.state(Some(now)))
    }

    /// The device that last wrote this session's store, when it was another
    /// one within the lib's warn window: the "review on one device at a
    /// time" banner's data. `now_ms` injects the clock (tests).
    #[flutter_rust_bridge::frb(sync)]
    pub fn foreign_writer(&self, now_ms: Option<u64>) -> Option<ForeignWriter> {
        let now = now_ms.unwrap_or_else(alix::time::now_ms);
        let mine = self.store.device.as_deref()?;
        self.store
            .recent_foreign_writer(mine, now)
            .map(|(device, age_ms)| ForeignWriter { device, age_ms })
    }

    /// The "where am I" region breadcrumb, when this session is
    /// topology-ordered and the current card sits in a region of its
    /// topology; `None` otherwise (no topology, no current card, or the
    /// current card isn't grouped into any region). A faithful mirror of the
    /// web's crumb build (`src/serve/dto.rs`): a shared augment cache can
    /// hold several like-named topologies (decks sharing a store), so this
    /// picks the one of the resolved name that actually contains the
    /// current card. `now_ms` injects the clock behind the strength heatmap
    /// (tests); `None` is the wall clock.
    #[flutter_rust_bridge::frb(sync)]
    pub fn crumb(&self, now_ms: Option<u64>) -> Option<CrumbState> {
        let now = now_ms.unwrap_or_else(alix::time::now_ms);
        let card = self.session.current()?;
        let name = self.topology_name.as_ref()?;
        let (topo, regions, current) = self
            .augment
            .topologies()
            .iter()
            .filter(|t| t.name == *name)
            .find_map(|t| {
                card.id()
                    .as_deref()
                    .and_then(|id| t.region_path(id))
                    .map(|(rg, cur)| (t, rg, cur))
            })?;
        Some(CrumbState {
            regions: regions.into_iter().map(str::to_string).collect(),
            current: current as u32,
            cells: topo
                .regions
                .iter()
                .map(|reg| alix::session::card_strengths(&reg.cards, &self.store, now))
                .collect(),
        })
    }

    /// The current card's authored fields for the remote tutor to ground its
    /// answer on, never the masked [`CardView`] a cloze review renders.
    /// `None` when no card is current.
    #[flutter_rust_bridge::frb(sync)]
    pub fn tutor_card(&self) -> Option<TutorCard> {
        let card = self.session.current()?;
        Some(TutorCard {
            subject: card.subject.to_string(),
            front: card.front.clone(),
            back: card.back.clone(),
            at: card.at.clone(),
            line: card.line,
        })
    }

    /// Mints a free-standing Tutor virtual card from an edited front/back,
    /// mirroring the web mint handler (`POST /api/ask/card/create`,
    /// `src/serve/mod.rs`): same validation and the same dedup against the
    /// session's own deck cards and any already-minted virtuals
    /// (`alix::store::mint_tutor_card`), then saves. Errors (malformed
    /// input, a duplicate of an existing card, or no card current to mint
    /// against) surface as the message text. Returns the new card's id,
    /// rendered as a string (the handler exposes nothing richer).
    #[flutter_rust_bridge::frb(sync)]
    pub fn mint_tutor_card(
        &mut self,
        front: String,
        back: Vec<String>,
        now_ms: u64,
    ) -> Result<String> {
        let Some(card) = self.session.current() else {
            bail!("no card is current to mint a tutor card against");
        };
        let subject = card.subject.to_string();
        // Dedup by canonical content, not id: a mint carries a fresh random
        // token, so identical content is caught by its §7 fingerprint.
        let deck_fingerprints: HashSet<u64> = self
            .session
            .cards()
            .iter()
            .map(|c| alix::l1::content_fingerprint(&c.front, &c.back))
            .collect();
        let id = alix::store::mint_tutor_card(
            &mut self.store,
            &subject,
            &front,
            &back,
            now_ms,
            &deck_fingerprints,
        )?;
        self.store.save()?;
        Ok(id)
    }

    /// Appends condensed tutor-note `notes` as `!` lines under the card at
    /// `line` of this session's deck file (`alix::deck::append_note`, atomic
    /// tmp+rename), mirroring the web note flow (`jobs.rs`'s `poll_ask`
    /// condense). Note lines are never hashed, so every card's id, and thus
    /// its review history, survives the append unchanged. An empty `notes`
    /// is a no-op, returning `Ok(())` without touching the file, mirroring
    /// the web's "nothing to save" guard. Mirrors the note onto the live
    /// session's current card so it shows without a reopen, guarded on the
    /// anchor line still matching the current card (mirroring the web's own
    /// card-identity guard, `jobs.rs`'s `poll_ask`); this is a deck-file
    /// edit, not progress, so the store is never saved here.
    #[flutter_rust_bridge::frb(sync)]
    pub fn apply_card_note(&mut self, line: u32, notes: Vec<String>) -> Result<()> {
        if notes.is_empty() {
            return Ok(());
        }
        alix::deck::append_note(&self.deck_path, line as usize, &notes)?;
        if let Some(cur) = self
            .session
            .current_mut()
            .filter(|cur| cur.line == line as usize)
        {
            cur.append_note(&notes);
        }
        Ok(())
    }

    /// Whether this deck sits an AI exam (the flag `open` captured).
    #[flutter_rust_bridge::frb(sync)]
    pub fn deck_has_exam(&self) -> bool {
        self.has_exam
    }

    /// Records a PASSED remote exam sitting as this deck's mastery, mirroring
    /// the browser exam's own persistence. Callers must never call this on a
    /// fail: a failed fact-deck exam writes nothing on the phone.
    #[flutter_rust_bridge::frb(sync)]
    pub fn apply_exam_passed(&mut self, now_ms: u64) -> Result<()> {
        self.store.set_deck_mastered(&self.subject, now_ms);
        self.store.save()?;
        Ok(())
    }

    /// Turns cleaned remediation deck-text (a failed remote exam's gaps)
    /// into virtual cards in the phone's own store, deduping against this
    /// deck's own cards and any already-stored virtuals
    /// (`alix::store::store_remediation_cards`, which saves internally, not
    /// saved again here). Returns how many cards were created or revived.
    ///
    /// `retire_after`: the bridge has no way today to read a session's
    /// resolved `[review] retire_after` cap back out of `alix::session::Session`
    /// (it holds no public accessor), so this passes `None`: the phone
    /// applies no retire cap in v1, rather than guess a value.
    #[flutter_rust_bridge::frb(sync)]
    pub fn apply_remediation(&mut self, cards_text: String, now_ms: u64) -> Result<u32> {
        let count = alix::store::store_remediation_cards(
            &mut self.store,
            &self.subject,
            &self.deck_fingerprints,
            &cards_text,
            now_ms,
            None,
        )?;
        Ok(count as u32)
    }
}

/// One line of a revealed excerpt: its file line number and text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkLine {
    pub n: u32,
    pub text: String,
}

/// A revealed source excerpt for the walk screen: line-numbered,
/// contiguous. Mirrors the web's `ExcerptDto` (`src/serve/dto.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkExcerpt {
    pub path: String,
    pub lines: Vec<WalkLine>,
    pub truncated: bool,
}

/// The walk tally shown on the done screen. Mirrors the web's `SummaryDto`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkSummary {
    pub passed: u32,
    pub partly: u32,
    pub failed: u32,
    /// 1-based hop numbers judged partly or failed.
    pub weak: Vec<u32>,
    pub total: u32,
}

/// The current position in an on-device trace walk, for the screen to
/// render. Mirrors the web's `WalkDto` (`src/serve/dto.rs`) minus the hop
/// rail and the live (`--grade`) fields: the phone walk is always
/// self-graded, so there is no path rail to draw and no auto-grade to poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkState {
    pub phase: WalkPhase,
    pub description: String,
    pub source: Option<String>,
    pub total: u32,
    /// 1-based index of the hop being walked.
    pub current: u32,
    // predict + reveal
    pub prompt: Option<String>,
    pub givens: Vec<String>,
    pub locator: Option<String>,
    /// What the learner predicted (shown on reveal).
    pub prediction: Option<String>,
    // reveal
    pub excerpt: Option<WalkExcerpt>,
    /// The honest fallback when the checkpoint's source can't be revealed (a
    /// URL `% source:`, none at all, or a resolution failure).
    pub excerpt_error: Option<String>,
    pub points: Vec<String>,
    pub note: Option<String>,
    // done
    pub summary: Option<WalkSummary>,
}

fn walk_excerpt(excerpt: &alix::trace::Excerpt) -> WalkExcerpt {
    WalkExcerpt {
        path: excerpt.path.display().to_string(),
        lines: excerpt
            .lines
            .iter()
            .map(|(n, text)| WalkLine {
                n: *n as u32,
                text: text.clone(),
            })
            .collect(),
        truncated: excerpt.truncated,
    }
}

/// Builds the current [`WalkState`] off the Walk engine's own accessors,
/// the same ones the web's `walk_dto` reads (`src/serve/dto.rs:1015-1095`),
/// minus the hop rail and live-grade bookkeeping this on-device (self-graded
/// only) walk has no use for.
fn walk_state(walk: &alix::trace::Walk) -> WalkState {
    let trace = walk.trace();
    let phase = walk.phase();

    let mut state = WalkState {
        phase,
        description: trace.description.clone(),
        source: trace.source.clone(),
        total: walk.total() as u32,
        current: walk.current_index() as u32 + 1,
        prompt: None,
        givens: Vec::new(),
        locator: None,
        prediction: None,
        excerpt: None,
        excerpt_error: None,
        points: Vec::new(),
        note: None,
        summary: None,
    };

    match phase {
        WalkPhase::Predict => {
            if let Some(c) = walk.checkpoint() {
                state.prompt = Some(c.prompt.clone());
                state.givens = c.givens.clone();
                state.locator = c.locator.clone();
            }
        }
        WalkPhase::Reveal => {
            if let Some(c) = walk.checkpoint() {
                state.prompt = Some(c.prompt.clone());
                state.givens = c.givens.clone();
                state.locator = c.locator.clone();
                state.points = c.points.clone();
                state.note = c.note.clone();
                match trace.excerpt(c) {
                    Ok(ex) => {
                        // Relabel a frozen-snapshot excerpt to its original
                        // source, exactly as the web reveal does, so the
                        // gutter shows real line numbers, not the asset's.
                        let (ex, label) =
                            alix::trace::relabel_for_display(ex, c.at_origin.as_deref());
                        if let Some(label) = label {
                            state.locator = Some(label);
                        }
                        state.excerpt = Some(walk_excerpt(&ex));
                    }
                    Err(e) => state.excerpt_error = Some(format!("{e:#}")),
                }
            }
            state.prediction = walk
                .prediction(walk.current_index())
                .map(str::to_string)
                .filter(|p| !p.is_empty());
        }
        WalkPhase::Done => {
            let s = walk.summary();
            state.summary = Some(WalkSummary {
                passed: s.passed as u32,
                partly: s.partly as u32,
                failed: s.failed as u32,
                weak: s.weak.iter().map(|i| *i as u32 + 1).collect(),
                total: walk.total() as u32,
            });
        }
    }

    state
}

/// A live on-device trace walk: the [`alix::trace::Walk`] engine plus its
/// open store. Dart holds this as an opaque handle, the walk's sibling of
/// [`ReviewSession`]. The walk runs entirely on-device (self-graded, no
/// server, no AI); only the trace exam (a later screen) needs the paired
/// desktop.
pub struct WalkSession {
    walk: alix::trace::Walk,
    store: alix::store::Store,
    /// The deck's file-name subject, captured off the loaded `Deck` exactly
    /// as `ReviewSession` does (see its own field doc): the exam-mastery and
    /// exam-failed-cooldown store calls key off this.
    subject: String,
    /// This deck's own checkpoints' §7 content fingerprints at open time, held
    /// for parity with `ReviewSession`'s dedup baseline.
    #[expect(dead_code)] // no walk-side remediation flow yet to dedup against
    deck_fingerprints: HashSet<u64>,
    /// Whether this deck sits an AI exam (always true for a trace: its exam
    /// is the graded compression). Captured at open time.
    has_exam: bool,
}

impl WalkSession {
    /// Opens a trace deck of the decks folder `root_dir` for an on-device
    /// walk. The progress store is routed the same way
    /// [`ReviewSession::open`] routes it (a workspace member's own store,
    /// else the root's shared one). `now_ms` injects the session clock
    /// (tests); `None` is the wall clock. `device` names this device in the
    /// store's last-writer marker; `None` keeps whatever the core derived
    /// for this machine. Bails if `deck_path` is not a trace deck: a card
    /// review opens through [`ReviewSession::open`] instead.
    #[flutter_rust_bridge::frb(sync)]
    pub fn open(
        deck_path: String,
        root_dir: String,
        now_ms: Option<u64>,
        device: Option<String>,
    ) -> Result<WalkSession> {
        let deck = PathBuf::from(deck_path);
        let loaded = alix::deck::Deck::load(&deck)?;
        let subject = loaded.subject.clone();
        let deck_fingerprints: HashSet<u64> = loaded
            .cards
            .iter()
            .map(|c| alix::l1::content_fingerprint(&c.front, &c.back))
            .collect();
        let has_exam = loaded.has_exam();

        let root_store = alix::workspace::root_store_path(Path::new(&root_dir));
        let mut store =
            alix::assemble::store_for(std::slice::from_ref(&deck), Some(&root_store))?;
        if device.is_some() {
            store.device = device;
        }
        // The instance config a CLI/server launch would carry, at its built-in
        // defaults, exactly mirroring `ReviewSession::open` (`AssembleConfig`
        // has no `Default`). `trace_auto_grade` stays false: the phone walk
        // is always self-graded, never AI-graded.
        let cfg = alix::assemble::AssembleConfig {
            review: alix::config::ReviewConfig::default(),
            ask: alix::config::AskConfig::default(),
            trace_auto_grade: false,
            pacing: alix::assemble::Pacing {
                max_new: 10,
                limit: None,
            },
            instance_store: None,
        };
        let opts = alix::assemble::SelectOptions {
            now_ms,
            ..Default::default()
        };
        let selected = alix::assemble::select(vec![deck], &mut store, &cfg, &opts)?;
        let build = match selected {
            alix::assemble::Selected::Walk(build) => build,
            alix::assemble::Selected::Review(_) => {
                bail!("this deck is a card review, not a trace walk")
            }
        };
        Ok(WalkSession {
            walk: build.walk,
            store,
            subject,
            deck_fingerprints,
            has_exam,
        })
    }

    /// The current walk position, for the screen to render.
    #[flutter_rust_bridge::frb(sync)]
    pub fn state(&self) -> WalkState {
        walk_state(&self.walk)
    }

    /// Commits the learner's prediction for the current checkpoint and moves
    /// to the reveal.
    #[flutter_rust_bridge::frb(sync)]
    pub fn predict(&mut self, text: String) {
        self.walk.predict(text);
    }

    /// Records the self-judged delta for the current checkpoint, schedules
    /// it in the store (the walk's only SRS write), persists, and returns
    /// the next position.
    #[flutter_rust_bridge::frb(sync)]
    pub fn grade(&mut self, delta: WalkDelta, now_ms: Option<u64>) -> Result<WalkState> {
        let now = now_ms.unwrap_or_else(alix::time::now_ms);
        self.walk.grade(&mut self.store, delta.into(), now);
        self.store.save()?;
        Ok(self.state())
    }

    /// Whether this deck sits an AI exam (the flag `open` captured; always
    /// true for a trace, since its exam is the graded compression).
    #[flutter_rust_bridge::frb(sync)]
    pub fn deck_has_exam(&self) -> bool {
        self.has_exam
    }

    /// Milliseconds left on a re-sit cooldown after a failed trace exam, or
    /// `None` if it can be sat now. The cooldown length reads
    /// `ExamConfig::default()` (the phone carries no `[exam]` config to
    /// override it in this milestone).
    #[flutter_rust_bridge::frb(sync)]
    pub fn exam_cooldown_ms(&self, now_ms: u64) -> Option<u64> {
        alix::store::cooldown_remaining_ms(
            &self.store,
            &self.subject,
            alix::config::ExamConfig::default().retry_cooldown_secs,
            now_ms,
        )
    }

    /// Records a PASSED trace exam as this deck's mastery, mirroring the
    /// browser exam's own persistence.
    #[flutter_rust_bridge::frb(sync)]
    pub fn apply_exam_passed(&mut self, now_ms: u64) -> Result<()> {
        self.store.set_deck_mastered(&self.subject, now_ms);
        self.store.save()?;
        Ok(())
    }

    /// Records a FAILED trace exam so a re-sit waits out the cooldown. The
    /// phone owns this write; the server never persists a trace-exam fail.
    #[flutter_rust_bridge::frb(sync)]
    pub fn apply_exam_failed(&mut self, now_ms: u64) -> Result<()> {
        self.store.set_exam_failed(&self.subject, now_ms);
        self.store.save()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000;
    /// Past the acquire cooldown.
    const LATER: u64 = T0 + alix::scheduler::DEFAULT_ACQUIRE_COOLDOWN_MS + 1_000;

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    /// Acquire every card of a freshly-opened deck at T0, then reopen past
    /// the cooldown so the first real quiz is up. No wall-clock waits.
    fn opened_after_acquire(deck: &Path, root: &Path, depth: Option<Depth>) -> ReviewSession {
        let mut s = ReviewSession::open(
            deck.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            None,
            Some(T0),
            None,
        )
        .unwrap();
        while s.state(Some(T0)).acquire {
            s.acquire(Some(T0)).unwrap();
        }
        ReviewSession::open(
            deck.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            depth,
            Some(LATER),
            None,
        )
        .unwrap()
    }

    #[test]
    fn grades_route_to_the_workspace_and_root_stores() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("loose.txt"), "# 2 plus 2?\n\t4\n");
        std::fs::create_dir(root.join("ws")).unwrap();
        write(&root.join("ws/alix.toml"), "");
        write(&root.join("ws/member.txt"), "# capital of france?\n\tParis\n");

        for (deck, store_file) in [
            (root.join("loose.txt"), root.join("progress.json")),
            (root.join("ws/member.txt"), root.join("ws/progress.json")),
        ] {
            let mut s = opened_after_acquire(&deck, root, None);
            assert!(
                !s.state(Some(LATER)).acquire,
                "past the cooldown this is a quiz"
            );
            s.grade(Grade::Pass, Some(LATER)).unwrap();
            let json = std::fs::read_to_string(&store_file).unwrap();
            assert!(
                json.contains("\"recall\"") && json.contains("\"history\""),
                "the grade persists into {store_file:?}"
            );
        }
        // The loose deck's grade must NOT have landed in the workspace store
        // and vice versa: each file holds exactly its own card.
        let root_store = std::fs::read_to_string(root.join("progress.json")).unwrap();
        let ws_store = std::fs::read_to_string(root.join("ws/progress.json")).unwrap();
        assert_eq!(root_store.matches("\"stability\"").count(), 1);
        assert_eq!(ws_store.matches("\"stability\"").count(), 1);
    }

    #[test]
    fn an_on_device_session_honors_the_workspace_deadline_ceiling() {
        // The phone's own session path (ReviewSession::open → assemble::select)
        // must apply a synced workspace's alix.local.toml deadline: a mature
        // card graded Pass three days before the date comes due before it.
        // Closed by execution, not by reading — the bridge builds its own
        // AssembleConfig, so the overlay firing here is the claim under test.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("ws");
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "title = \"W\"\n");
        write(&ws.join("m.txt"), "# q\n\ta\n");
        let deadline = alix::time::local_date(T0) + chrono::Days::new(3);
        write(
            &ws.join("alix.local.toml"),
            &format!("[review]\ndeadline = \"{}\"\n", deadline.format("%Y-%m-%d")),
        );

        // A mature Review-state card that would schedule ~months uncapped.
        let id = alix::deck::Deck::load(ws.join("m.txt")).unwrap().cards[0].id();
        let mut store = alix::store::Store::open(alix::workspace::store_path(&ws)).unwrap();
        store.get_or_insert(id, T0).recall = Some(alix::store::FsrsState {
            stability: 200.0,
            difficulty: 5.0,
            state: 2,
            reps: 10,
            scheduled_days: 90,
            last_review_ms: T0.saturating_sub(90 * 86_400_000),
            due_ms: T0.saturating_sub(1_000), // due now
            ..Default::default()
        });
        store.save().unwrap();

        let mut s = ReviewSession::open(
            ws.join("m.txt").to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            None,
            Some(T0),
            None,
        )
        .unwrap();
        s.grade(Grade::Pass, Some(T0)).unwrap();

        let ceiling = alix::time::end_of_local_day_ms(deadline);
        let store = alix::store::Store::open(alix::workspace::store_path(&ws)).unwrap();
        let due = store.get(id).unwrap().recall.unwrap().due_ms;
        assert!(
            due <= ceiling,
            "due {due} must respect the deadline ceiling {ceiling}"
        );
    }

    #[test]
    fn choose_agrees_with_the_served_options() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("d.txt"),
            "# q1\n\ta1\n# q2\n\ta2\n# q3\n\ta3\n# q4\n\ta4\n",
        );
        // Cache a full set of AI distractors so a Recognize pick can be built;
        // options are never sampled from sibling answers.
        let store_path = alix::workspace::root_store_path(root);
        let mut cache =
            alix::augment::AugmentCache::open(alix::augment::augment_path_for(&store_path));
        for card in &alix::deck::Deck::load(&root.join("d.txt")).unwrap().cards {
            cache.set_distractors(
                card.id(),
                vec!["w1".to_string(), "w2".to_string(), "w3".to_string()],
            );
        }
        cache.save().unwrap();

        let s = opened_after_acquire(&root.join("d.txt"), root, Some(Depth::Recognize));
        let state = s.state(Some(LATER));
        assert_eq!(state.mode, Mode::Choice);
        let options = state.choices.expect("a recognize pick");
        assert_eq!(options.len(), 4);
        let feedback = s.choose(0).expect("feedback");
        let correct = feedback.correct;
        assert!(s.choose(correct as u32).expect("feedback").passed);
        assert_eq!(s.state(Some(LATER)).choices.as_deref(), Some(&options[..]));
    }

    #[test]
    fn keypoint_grade_maps_the_tally_like_core() {
        assert_eq!(keypoint_grade(0, 3), Grade::Fail);
        assert_eq!(keypoint_grade(1, 3), Grade::Partial);
        assert_eq!(keypoint_grade(2, 3), Grade::Partial);
        assert_eq!(keypoint_grade(3, 3), Grade::Pass);
        assert_eq!(keypoint_grade(0, 0), Grade::Pass, "no rubric, nothing to miss");
    }

    #[test]
    fn explain_state_carries_the_keypoints_rubric() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A multi-line back at Reconstruct renders as Explain.
        write(&root.join("d.txt"), "# why\n\tfirst fact\n\tsecond fact\n");
        let s = opened_after_acquire(&root.join("d.txt"), root, Some(Depth::Reconstruct));
        let state = s.state(Some(LATER));
        assert_eq!(state.mode, Mode::Explain);
        assert_eq!(
            state.keypoints,
            Some(vec!["first fact".to_string(), "second fact".to_string()]),
            "no cached keypoints: the rubric is the authored back"
        );

        // Cached keypoints (the augment sidecar the session reads) win.
        let store_path = alix::workspace::root_store_path(root);
        let mut cache =
            alix::augment::AugmentCache::open(alix::augment::augment_path_for(&store_path));
        let deck = alix::deck::Deck::load(&root.join("d.txt")).unwrap();
        cache.set_keypoints(deck.cards[0].id(), vec!["one claim".to_string()]);
        cache.save().unwrap();
        let s = ReviewSession::open(
            root.join("d.txt").to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            Some(Depth::Reconstruct),
            Some(LATER),
            None,
        )
        .unwrap();
        assert_eq!(
            s.state(Some(LATER)).keypoints,
            Some(vec!["one claim".to_string()])
        );
    }

    #[test]
    fn foreign_writer_warns_the_other_device_and_never_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# q\n\ta\n");
        let open_as = |device: &str| {
            ReviewSession::open(
                root.join("d.txt").to_string_lossy().into_owned(),
                root.to_string_lossy().into_owned(),
                None,
                Some(T0),
                Some(device.to_string()),
            )
            .unwrap()
        };
        // Nothing written yet: no marker to warn about. Note that assembly
        // itself saves (it records the last depth), so every `open` below
        // stamps the store as a write by that device.
        assert!(open_as("phone-1").foreign_writer(None).is_none());

        // desk-1 acquires: the store is now desk-1's write.
        let mut desk = open_as("desk-1");
        desk.acquire(Some(T0)).unwrap();
        assert!(
            open_as("desk-1").foreign_writer(None).is_none(),
            "a device's own writes are not foreign"
        );
        let seen = open_as("phone-1")
            .foreign_writer(None)
            .expect("the other device sees the fresh write");
        assert_eq!(seen.device, "desk-1");
        assert!(seen.age_ms < alix::store::FOREIGN_WRITE_WARN_WINDOW_MS);
    }

    #[test]
    fn check_reports_per_line_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# q\n\tParis\n");
        let s = opened_after_acquire(&root.join("d.txt"), root, None);
        let feedback = s.check(vec!["paris".to_string()]).expect("feedback");
        assert!(feedback.passed, "normalized match");
        let wrong = s.check(vec!["london".to_string()]).expect("feedback");
        assert!(!wrong.passed);
        assert_eq!(wrong.results[0].expected, "Paris");
    }

    #[test]
    fn tutor_card_exposes_the_authored_card_not_the_masked_view() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("d.txt"),
            "# capital?\n% reveal: cloze\n\tParis is the capital of {{France}}\n",
        );
        // The authored back, read independently of the session under test,
        // never a hand-typed guess at what the cloze parse produces.
        let authored = alix::deck::Deck::load(root.join("d.txt")).unwrap();
        let authored_back = authored.cards[0].back.clone();

        let s = ReviewSession::open(
            root.join("d.txt").to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            None,
            Some(T0),
            None,
        )
        .unwrap();

        let tutor = s.tutor_card().expect("a card is current");
        assert_eq!(tutor.subject, "d.txt");
        assert_eq!(tutor.back, authored_back);

        let view = s.state(Some(T0)).card.expect("a rendered card");
        assert_ne!(
            view.context, tutor.back,
            "the CardView's context blanks the hole under review; the tutor \
             sees the real answer, not the blanked-out puzzle"
        );
    }

    #[test]
    fn mint_tutor_card_dedups_against_the_deck() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("d.txt"),
            "# capital of france?\n\tParis\n# capital of germany?\n\tBerlin\n",
        );
        let mut s = opened_after_acquire(&root.join("d.txt"), root, None);
        let store_path = alix::workspace::root_store_path(root);

        // Same back as an existing deck card (front may differ; id hashes
        // only subject + back, matching the web handler's own dedup): the
        // web handler rejects this as a duplicate, never minting it.
        let dup = s.mint_tutor_card(
            "what is the capital of france?".to_string(),
            vec!["Paris".to_string()],
            LATER,
        );
        assert!(
            dup.is_err(),
            "a card matching an existing deck card must not mint a duplicate"
        );
        let reopened = alix::store::Store::open(&store_path).unwrap();
        assert_eq!(reopened.virtual_len(), 0, "the duplicate never reached disk");

        // Fresh content: mints a new Tutor virtual, retrievable from disk.
        let id_str = s
            .mint_tutor_card("capital of spain?".to_string(), vec!["Madrid".to_string()], LATER)
            .expect("fresh content mints");
        let id: u64 = id_str.parse().expect("the id renders as a string");
        let reopened = alix::store::Store::open(&store_path).unwrap();
        let vc = reopened
            .get_virtual(id)
            .expect("the fresh mint is retrievable from disk");
        assert_eq!(vc.kind, alix::store::VirtualKind::Tutor);
    }

    #[test]
    fn tutor_card_carries_the_cards_front_line() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A header directive shifts the front off line 1, so a hardcoded `1`
        // would not pass this test by accident.
        write(&root.join("d.txt"), "% link: https://x\n# q\n\ta\n");
        let authored = alix::deck::Deck::load(root.join("d.txt")).unwrap();
        let authored_line = authored.cards[0].line;

        let s = ReviewSession::open(
            root.join("d.txt").to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            None,
            Some(T0),
            None,
        )
        .unwrap();
        let tutor = s.tutor_card().expect("a card is current");
        assert_eq!(tutor.line, authored_line);
    }

    #[test]
    fn apply_card_note_writes_note_lines_and_preserves_the_card_id() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# q\n\ta\n");

        let before = alix::deck::Deck::load(root.join("d.txt")).unwrap();
        let id_before = before.cards[0].id();
        let line = before.cards[0].line;

        let store_path = alix::workspace::root_store_path(root);
        let mut s = opened_after_acquire(&root.join("d.txt"), root, None);
        s.grade(Grade::Pass, Some(LATER)).unwrap();
        let schedule_before = alix::store::Store::open(&store_path)
            .unwrap()
            .get(id_before)
            .and_then(|cs| cs.recall);
        assert!(
            schedule_before.is_some(),
            "grading scheduled the card at Recall before the note append"
        );

        s.apply_card_note(line as u32, vec!["first".to_string(), "second".to_string()])
            .unwrap();

        let text = std::fs::read_to_string(root.join("d.txt")).unwrap();
        assert!(text.contains("! first"), "{text}");
        assert!(text.contains("! second"), "{text}");

        // The load-bearing assertion: reload the deck via the lib and
        // recompute the id independently. A note append must not reshuffle
        // it, or a learner's review history would silently reset.
        let after = alix::deck::Deck::load(root.join("d.txt")).unwrap();
        let id_after = after.cards[0].id();
        assert_eq!(
            id_before, id_after,
            "appending a note must not change the card's id"
        );

        // And the schedule keyed on that (unchanged) id is still there after
        // the append: progress survived.
        let reopened = alix::store::Store::open(&store_path).unwrap();
        assert_eq!(
            reopened.get(id_after).and_then(|cs| cs.recall),
            schedule_before,
            "the id-keyed schedule survives the note append"
        );
    }

    #[test]
    fn apply_card_note_with_empty_notes_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# q\n\ta\n");
        let before_bytes = std::fs::read(root.join("d.txt")).unwrap();

        let mut s = opened_after_acquire(&root.join("d.txt"), root, None);
        s.apply_card_note(1, Vec::new()).unwrap();

        let after_bytes = std::fs::read(root.join("d.txt")).unwrap();
        assert_eq!(
            before_bytes, after_bytes,
            "an empty notes vec is a no-op: not one byte changes"
        );
    }

    #[test]
    fn apply_card_note_mirrors_onto_the_live_session_without_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# q\n\ta\n");
        let mut s = opened_after_acquire(&root.join("d.txt"), root, None);
        let line = s.tutor_card().expect("a card is current").line;

        assert!(
            s.state(Some(LATER))
                .card
                .expect("a rendered card")
                .note
                .is_empty(),
            "no note yet"
        );

        s.apply_card_note(line as u32, vec!["explained".to_string()])
            .unwrap();

        // Read back through the live session, never a reopen: the mirror
        // onto `session.current_mut()` is what makes this visible at once.
        let note = s.state(Some(LATER)).card.expect("a rendered card").note;
        assert!(
            !note.is_empty(),
            "the note shows on the live session without a reopen"
        );
    }

    /// The T3.3-review rider: the in-memory mirror onto `session.current_mut()`
    /// must only fire when the append anchor still targets the current card's
    /// own line, mirroring the web's card-identity guard (`jobs.rs`'s
    /// `poll_ask`). The file append itself stays unconditional (line-keyed).
    #[test]
    fn apply_card_note_mirror_is_guarded_by_the_anchor_line() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# q1\n\ta1\n# q2\n\ta2\n");
        let loaded = alix::deck::Deck::load(root.join("d.txt")).unwrap();
        let line1 = loaded.cards[0].line;
        let line2 = loaded.cards[1].line;

        let mut s = opened_after_acquire(&root.join("d.txt"), root, None);
        let current_line = s.tutor_card().expect("a card is current").line;
        let other_line = if current_line == line1 { line2 } else { line1 };

        s.apply_card_note(other_line as u32, vec!["stale".to_string()])
            .unwrap();

        assert!(
            s.state(Some(LATER))
                .card
                .expect("a rendered card")
                .note
                .is_empty(),
            "a note anchored to a different card's line must not mirror onto \
             the current card"
        );
        let text = std::fs::read_to_string(root.join("d.txt")).unwrap();
        assert!(
            text.contains("! stale"),
            "the file append is unconditional (line-keyed): {text}"
        );
    }

    #[test]
    fn apply_exam_passed_marks_the_phone_store_mastered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# q\n\ta\n");
        let store_path = alix::workspace::root_store_path(root);
        let mut s = opened_after_acquire(&root.join("d.txt"), root, None);
        assert!(
            !alix::store::Store::open(&store_path)
                .unwrap()
                .deck_mastered("d.txt"),
            "fresh store: not mastered"
        );

        s.apply_exam_passed(LATER).unwrap();

        let reopened = alix::store::Store::open(&store_path).unwrap();
        assert!(reopened.deck_mastered("d.txt"));
        assert_eq!(reopened.deck_mastered_at("d.txt"), Some(LATER));
    }

    #[test]
    fn apply_remediation_creates_virtuals_and_dedups_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# capital of france?\n\tParis\n");
        let mut s = opened_after_acquire(&root.join("d.txt"), root, None);
        let store_path = alix::workspace::root_store_path(root);

        let remediation =
            "# capital of france?\n\tParis\n# capital of germany?\n\tBerlin\n".to_string();
        let created = s.apply_remediation(remediation.clone(), LATER).unwrap();
        assert_eq!(created, 1, "the Paris block already matches a deck card");

        let reopened = alix::store::Store::open(&store_path).unwrap();
        assert_eq!(
            reopened.virtual_len(),
            1,
            "only the new Berlin block became a virtual"
        );
        let berlin_id = alix::parser::parse_str("d.txt", "# capital of germany?\n\tBerlin\n")
            .unwrap()[0]
            .id();
        let vc = reopened
            .get_virtual(berlin_id)
            .expect("the berlin block is stored as a virtual");
        assert_eq!(vc.kind, alix::store::VirtualKind::Remediation);

        // Re-run the identical text: no new/duplicate virtuals, count is 0.
        let created_again = s.apply_remediation(remediation, LATER).unwrap();
        assert_eq!(
            created_again, 0,
            "an active dupe is left alone, no schedule reset"
        );
        let reopened_again = alix::store::Store::open(&store_path).unwrap();
        assert_eq!(reopened_again.virtual_len(), 1);
    }

    #[test]
    fn crumb_is_none_for_a_plain_non_topology_deck() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("d.txt"), "# q\n\ta\n");
        let s = opened_after_acquire(&root.join("d.txt"), root, None);
        assert!(
            s.crumb(Some(LATER)).is_none(),
            "no topology cached, so no crumb"
        );
    }

    /// Caches a two-region topology (ids resolved from a real parse, never
    /// hand-derived) into `root`'s augment sidecar, so `ReviewSession::open`'s
    /// own `assemble::select` auto-picks it up (a deck with exactly one
    /// cached topology needs no explicit `--topology` selection). `walk`
    /// controls which card the scheduler serves first among two equally-due
    /// cards (`session.rs` sorts the due set by topology rank), so tests can
    /// pin the "current" card by choosing `walk`'s order.
    fn cache_two_region_topology(root: &Path, walk: Vec<u64>, regions: Vec<(&str, Vec<u64>)>) {
        let store_path = alix::workspace::root_store_path(root);
        let mut cache =
            alix::augment::AugmentCache::open(alix::augment::augment_path_for(&store_path));
        cache.add_topology(alix::augment::Topology {
            name: "auto".to_string(),
            principle: "test order".to_string(),
            walk,
            regions: regions
                .into_iter()
                .map(|(name, cards)| alix::augment::TopologyRegion {
                    name: name.to_string(),
                    cards,
                })
                .collect(),
            ..Default::default()
        });
        cache.save().unwrap();
    }

    #[test]
    fn crumb_reports_the_current_cards_region_in_a_topology_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck = root.join("d.txt");
        write(&deck, "# q1\n\ta1\n# q2\n\ta2\n");
        let loaded = alix::deck::Deck::load(&deck).unwrap();
        let id1 = loaded.cards[0].id();
        let id2 = loaded.cards[1].id();

        cache_two_region_topology(
            root,
            vec![id1, id2],
            vec![("Intro", vec![id1]), ("Body", vec![id2])],
        );

        let s = opened_after_acquire(&deck, root, None);
        let current_front = s.state(Some(LATER)).card.expect("a card is current").front;
        let expected_current = if current_front == "q1" { 0u32 } else { 1u32 };

        let crumb = s
            .crumb(Some(LATER))
            .expect("a topology-ordered session with the card in a region crumbs");
        assert_eq!(crumb.regions, vec!["Intro".to_string(), "Body".to_string()]);
        assert_eq!(crumb.current, expected_current);
        assert_eq!(crumb.cells.len(), 2, "one cell row per region");
        assert_eq!(crumb.cells[0].len(), 1, "Intro holds one card");
        assert_eq!(crumb.cells[1].len(), 1, "Body holds one card");
    }

    #[test]
    fn crumb_is_none_when_the_current_card_has_no_region() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck = root.join("d.txt");
        write(&deck, "# q1\n\ta1\n# q2\n\ta2\n");
        let loaded = alix::deck::Deck::load(&deck).unwrap();
        let id1 = loaded.cards[0].id();
        let id2 = loaded.cards[1].id();

        // id2 ranks first (so it's served first among two equally-due
        // cards), but only id1 has a region: the current card (id2) sits in
        // no region, so no crumb, no panic.
        cache_two_region_topology(root, vec![id2, id1], vec![("Intro", vec![id1])]);

        let s = opened_after_acquire(&deck, root, None);
        let current_front = s.state(Some(LATER)).card.expect("a card is current").front;
        assert_eq!(
            current_front, "q2",
            "topology ranks id2 first among due cards"
        );
        assert!(
            s.crumb(Some(LATER)).is_none(),
            "the current card sits in no region, so no crumb, and no panic"
        );
    }

    /// A two-hop trace over a real in-folder source file, subject `t.txt`.
    fn trace_fixture(root: &Path) -> PathBuf {
        write(&root.join("source.txt"), "first\nsecond\nthird\n");
        let path = root.join("t.txt");
        write(
            &path,
            "% trace: how it works\n\
             % source: source.txt\n\
             # Predict the first hop\n\
             \tit reads the first line\n\
             \t% at: 1\n\
             # Predict the second hop\n\
             \tit reads lines two and three\n\
             \t% at: 2-3\n",
        );
        path
    }

    #[test]
    fn walking_a_trace_predicts_reveals_a_real_excerpt_and_tallies_the_summary() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck = trace_fixture(root);
        let mut s = WalkSession::open(
            deck.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            Some(T0),
            None,
        )
        .unwrap();

        let state = s.state();
        assert_eq!(state.phase, WalkPhase::Predict);
        assert_eq!(state.description, "how it works");
        assert_eq!(state.source.as_deref(), Some("source.txt"));
        assert_eq!(state.total, 2);
        assert_eq!(state.current, 1);
        assert_eq!(state.prompt.as_deref(), Some("Predict the first hop"));
        assert!(state.givens.is_empty());

        s.predict("guess1".to_string());
        let state = s.state();
        assert_eq!(state.phase, WalkPhase::Reveal);
        assert_eq!(state.prediction.as_deref(), Some("guess1"));
        assert!(state.excerpt_error.is_none());
        let excerpt = state.excerpt.expect("a real in-folder source resolves");
        assert!(excerpt.path.ends_with("source.txt"), "{}", excerpt.path);
        assert_eq!(
            excerpt.lines,
            vec![WalkLine {
                n: 1,
                text: "first".to_string()
            }]
        );
        assert_eq!(state.points, vec!["it reads the first line".to_string()]);

        let state = s.grade(WalkDelta::Got, Some(T0)).unwrap();
        assert_eq!(state.phase, WalkPhase::Predict);
        assert_eq!(state.current, 2);
        assert_eq!(state.prompt.as_deref(), Some("Predict the second hop"));

        s.predict("guess2".to_string());
        let state = s.state();
        assert_eq!(state.phase, WalkPhase::Reveal);
        let excerpt = state.excerpt.expect("a real in-folder source resolves");
        assert_eq!(
            excerpt.lines,
            vec![
                WalkLine {
                    n: 2,
                    text: "second".to_string()
                },
                WalkLine {
                    n: 3,
                    text: "third".to_string()
                },
            ]
        );

        let state = s.grade(WalkDelta::Partly, Some(T0)).unwrap();
        assert_eq!(state.phase, WalkPhase::Done);
        let summary = state.summary.expect("the done screen tallies the walk");
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.partly, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.weak, vec![2], "1-based hop numbers");
        assert_eq!(summary.total, 2);
    }

    #[test]
    fn walk_excerpt_resolves_an_in_folder_source_inside_a_workspace_member() {
        // Ledger row 1: a synced workspace member's `% source:` is relative
        // to ITS OWN folder, not the root passed to `open`. This guards
        // against a regression that resolves it against the wrong base.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("box");
        std::fs::create_dir(&ws).unwrap();
        write(&ws.join("alix.toml"), "title = \"Box\"\n");
        write(&ws.join("source.txt"), "alpha\nbeta\ngamma\n");
        write(
            &ws.join("t.txt"),
            "% trace: a member walk\n\
             % source: source.txt\n\
             # Predict\n\
             \tit reads line two\n\
             \t% at: 2\n",
        );

        let mut s = WalkSession::open(
            ws.join("t.txt").to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            Some(T0),
            None,
        )
        .unwrap();
        s.predict("guess".to_string());
        let state = s.state();
        assert_eq!(state.phase, WalkPhase::Reveal);
        assert!(state.excerpt_error.is_none());
        let excerpt = state.excerpt.expect("the member's own source resolves");
        assert!(excerpt.path.ends_with("source.txt"), "{}", excerpt.path);
        assert_eq!(
            excerpt.lines,
            vec![WalkLine {
                n: 2,
                text: "beta".to_string()
            }]
        );
    }

    #[test]
    fn walk_excerpt_error_is_honest_for_a_url_or_absent_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // No `% source:` at all: a bare line-number locator has no file to
        // resolve against.
        let no_source = root.join("no-source.txt");
        write(
            &no_source,
            "% trace: a path with no source\n\
             # Predict something\n\
             \tthe answer\n\
             \t% at: 1\n",
        );
        let mut s = WalkSession::open(
            no_source.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            Some(T0),
            None,
        )
        .unwrap();
        s.predict("guess".to_string());
        let state = s.state();
        assert_eq!(state.phase, WalkPhase::Reveal);
        assert!(state.excerpt.is_none(), "no panic, just an honest fallback");
        assert!(state.excerpt_error.is_some());

        // A URL `% source:` has no local line ranges either.
        let url_source = root.join("url-source.txt");
        write(
            &url_source,
            "% trace: a path with a URL source\n\
             % source: https://example.com/readme.md\n\
             # Predict something\n\
             \tthe answer\n\
             \t% at: 1\n",
        );
        let mut s = WalkSession::open(
            url_source.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            Some(T0),
            None,
        )
        .unwrap();
        s.predict("guess".to_string());
        let state = s.state();
        assert_eq!(state.phase, WalkPhase::Reveal);
        assert!(state.excerpt.is_none(), "no panic, just an honest fallback");
        assert!(state.excerpt_error.is_some());
    }

    #[test]
    fn exam_cooldown_gates_a_resit_after_a_failed_trace_exam_and_a_pass_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck = trace_fixture(root);
        let mut s = WalkSession::open(
            deck.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            Some(T0),
            None,
        )
        .unwrap();
        assert!(s.deck_has_exam(), "a trace always sits an exam");
        assert_eq!(s.exam_cooldown_ms(T0), None, "never failed: no cooldown");

        s.apply_exam_failed(T0).unwrap();
        let cooldown_ms = alix::config::ExamConfig::default().retry_cooldown_secs * 1000;
        assert_eq!(s.exam_cooldown_ms(T0), Some(cooldown_ms));
        assert_eq!(
            s.exam_cooldown_ms(T0 + cooldown_ms + 1),
            None,
            "the cooldown elapsed"
        );

        let store_path = alix::workspace::root_store_path(root);
        assert!(
            !alix::store::Store::open(&store_path)
                .unwrap()
                .deck_mastered("t.txt"),
            "fresh: not yet mastered"
        );
        s.apply_exam_passed(T0 + cooldown_ms + 1).unwrap();
        assert!(
            s.deck_has_exam(),
            "the flag is captured at open, not derived from the store"
        );
        let reopened = alix::store::Store::open(&store_path).unwrap();
        assert!(reopened.deck_mastered("t.txt"));
        assert_eq!(
            reopened.deck_mastered_at("t.txt"),
            Some(T0 + cooldown_ms + 1)
        );
    }

    #[test]
    fn walk_and_review_open_refuse_each_others_deck_kind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let trace = trace_fixture(root);
        let facts = root.join("facts.txt");
        write(&facts, "# q\n\ta\n");

        // `.err()` (not `.unwrap_err()`): the opaque session handles carry no
        // `Debug` impl, which `unwrap_err`'s panic message would require.
        let err = WalkSession::open(
            facts.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            Some(T0),
            None,
        )
        .err()
        .expect("a facts deck is not a trace walk");
        assert!(format!("{err:#}").contains("not a trace walk"), "{err}");

        let err = ReviewSession::open(
            trace.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            None,
            Some(T0),
            None,
        )
        .err()
        .expect("a trace deck is not a card review");
        assert!(format!("{err:#}").contains("not a trace"), "{err}");
    }
}
