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

/// The Explain checklist tally as a grade: none covered fails, all pass,
/// some is a partial. The rule lives in core (`scheduler::keypoint_grade`);
/// this is only the bridge.
#[flutter_rust_bridge::frb(sync)]
pub fn keypoint_grade(covered: u32, total: u32) -> Grade {
    alix::scheduler::keypoint_grade(covered as usize, total as usize).into()
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
}

/// A live review session running in Rust: the alix session plus its open
/// store and augment cache. Dart holds this as an opaque handle.
pub struct ReviewSession {
    session: alix::session::Session,
    store: alix::store::Store,
    augment: alix::augment::AugmentCache,
    /// The deck's file-name subject exactly as the lib derived it when this
    /// deck's cards were parsed (`Card::id` hashes it). Captured straight off
    /// the loaded `Deck`, never re-derived from `deck_path` by hand: a
    /// hand-derived subject that differs even by extension or case silently
    /// yields DIFFERENT ids, so dedup stops deduping and progress forks.
    subject: String,
    /// This deck's own card ids at open time: the dedup baseline for
    /// remediation (mirrors `exam::Sitting::deck_card_ids`, captured the same
    /// way, off a freshly loaded `Deck`, not the live session roster).
    deck_card_ids: HashSet<u64>,
    /// Whether this deck sits an AI exam: a fact deck with at least one
    /// `% source:`, never a trace (mirrors the server's
    /// `/api/remote/exam/start` predicate; a phone has no walk to sit).
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
        let deck_card_ids: HashSet<u64> = loaded.cards.iter().map(|c| c.id()).collect();
        // Mirrors the server's `/api/remote/exam/start` predicate: a trace's
        // exam is a graded compression the phone does not sit, and a
        // source-less fact deck has no exam at all.
        let has_exam = !loaded.is_trace() && !loaded.sources.is_empty();

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
            subject,
            deck_card_ids,
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
        let deck_ids: HashSet<u64> = self.session.cards().iter().map(|c| c.id()).collect();
        let id = alix::store::mint_tutor_card(
            &mut self.store,
            &subject,
            &front,
            &back,
            now_ms,
            &deck_ids,
        )?;
        self.store.save()?;
        Ok(id.to_string())
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
            &self.deck_card_ids,
            &cards_text,
            now_ms,
            None,
        )?;
        Ok(count as u32)
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
    fn choose_agrees_with_the_served_options() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("d.txt"),
            "# q1\n\ta1\n# q2\n\ta2\n# q3\n\ta3\n# q4\n\ta4\n",
        );
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
}
