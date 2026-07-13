//! The mobile review surface: an opaque handle around a live alix session,
//! its store, and its augment cache. Dart holds the handle and calls into it;
//! all review logic stays in the embedded core (`alix::review`).

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

pub use alix::answer::{Mode, TypedResult};
pub use alix::depth::Depth;
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

#[flutter_rust_bridge::frb(mirror(CardView))]
pub struct _CardView {
    pub front: String,
    pub context: Vec<String>,
    pub back: Vec<String>,
    pub note: Vec<String>,
    pub image: Option<String>,
    pub image_back: Option<String>,
}

#[flutter_rust_bridge::frb(mirror(ReviewState))]
pub struct _ReviewState {
    pub card: Option<CardView>,
    pub mode: Mode,
    pub depth: Depth,
    pub acquire: bool,
    pub choices: Option<Vec<String>>,
    pub finished: bool,
    pub remaining: u32,
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

/// A live review session running in Rust: the alix session plus its open
/// store and augment cache. Dart holds this as an opaque handle.
pub struct ReviewSession {
    session: alix::session::Session,
    store: alix::store::Store,
    augment: alix::augment::AugmentCache,
}

impl ReviewSession {
    /// Open a deck of the decks folder `root_dir` at `depth` (default:
    /// the deck's last depth, else Recall). The progress store is routed the
    /// way the web and CLI route it: a workspace member reviews into its
    /// workspace's own store, everything else into the root's shared store.
    /// `now_ms` injects the session clock (tests); `None` is the wall clock.
    #[flutter_rust_bridge::frb(sync)]
    pub fn open(
        deck_path: String,
        root_dir: String,
        depth: Option<Depth>,
        now_ms: Option<u64>,
    ) -> Result<ReviewSession> {
        let deck = PathBuf::from(deck_path);
        let root_store = alix::workspace::root_store_path(Path::new(&root_dir));
        let mut store =
            alix::assemble::store_for(std::slice::from_ref(&deck), Some(&root_store))?;
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
        })
    }

    /// The current review position, for the screen to render.
    #[flutter_rust_bridge::frb(sync)]
    pub fn state(&self) -> ReviewState {
        alix::review::state(&self.session, &self.store, &self.augment)
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
        Ok(self.state())
    }

    /// Mark the current never-seen card as acquired (first exposure, no
    /// grade) and persist, returning the next position.
    #[flutter_rust_bridge::frb(sync)]
    pub fn acquire(&mut self, now_ms: Option<u64>) -> Result<ReviewState> {
        let now = now_ms.unwrap_or_else(alix::time::now_ms);
        self.session.acquire_current(&mut self.store, now);
        self.store.save()?;
        self.session.poll(&self.store, now);
        Ok(self.state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000;
    /// Past the acquire cooldown.
    const LATER: u64 = T0 + 61_000;

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
        )
        .unwrap();
        while s.state().acquire {
            s.acquire(Some(T0)).unwrap();
        }
        ReviewSession::open(
            deck.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            depth,
            Some(LATER),
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
            assert!(!s.state().acquire, "past the cooldown this is a quiz");
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
        let state = s.state();
        assert_eq!(state.mode, Mode::Choice);
        let options = state.choices.expect("a recognize pick");
        assert_eq!(options.len(), 4);
        let feedback = s.choose(0).expect("feedback");
        let correct = feedback.correct;
        assert!(s.choose(correct as u32).expect("feedback").passed);
        assert_eq!(s.state().choices.as_deref(), Some(&options[..]));
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
}
