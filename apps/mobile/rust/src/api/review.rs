//! The mobile review surface: an opaque handle around a live alix session and
//! its store. Dart holds the handle and calls into it; all review logic stays
//! in the embedded core.

use anyhow::{bail, Result};
use std::path::PathBuf;

pub use alix::review::{CardView, ReviewState};

/// frb mirrors of the core review-state types (they live in the `alix` crate,
/// which frb does not scan): field-for-field copies that teach the generator
/// their shape so Dart gets real classes, not opaque handles. Keep in lock
/// step with `alix::review`.
#[flutter_rust_bridge::frb(mirror(CardView))]
pub struct _CardView {
    pub front: String,
    pub back: Vec<String>,
}

#[flutter_rust_bridge::frb(mirror(ReviewState))]
pub struct _ReviewState {
    pub card: Option<CardView>,
    pub finished: bool,
    pub remaining: u32,
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
/// store. Dart holds this as an opaque handle and calls into it.
pub struct ReviewSession {
    session: alix::session::Session,
    store: alix::store::Store,
}

impl ReviewSession {
    /// Open a deck file, with its progress store in `store_dir`.
    #[flutter_rust_bridge::frb(sync)]
    pub fn open(deck_path: String, store_dir: String) -> Result<ReviewSession> {
        let mut store =
            alix::store::Store::open(PathBuf::from(&store_dir).join("progress.json"))?;
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
        let opts = alix::assemble::SelectOptions::default();
        let selected =
            alix::assemble::select(vec![PathBuf::from(deck_path)], &mut store, &cfg, &opts)?;
        let session = match selected {
            alix::assemble::Selected::Review(build) => build.session,
            alix::assemble::Selected::Walk(_) => {
                bail!("milestone 1 reviews a facts deck, not a trace")
            }
        };
        Ok(ReviewSession { session, store })
    }

    /// The current review position, for the screen to render.
    #[flutter_rust_bridge::frb(sync)]
    pub fn state(&self) -> ReviewState {
        self.session.review_state()
    }

    /// Grade the current card and persist, returning the next position.
    #[flutter_rust_bridge::frb(sync)]
    pub fn grade(&mut self, grade: Grade) -> Result<ReviewState> {
        let now = alix::time::now_ms();
        self.session.grade(&mut self.store, grade.into(), now);
        self.store.save()?;
        self.session.poll(&self.store, now);
        Ok(self.session.review_state())
    }

    /// Whether the current card is a first exposure (never seen). The client
    /// then shows it with its answer and a single "Seen" action ([`Self::acquire`],
    /// attempt-first lifecycle) instead of quizzing it cold.
    #[flutter_rust_bridge::frb(sync)]
    pub fn unseen(&self) -> bool {
        self.session.current_unseen(&self.store)
    }

    /// Mark the current never-seen card as acquired (first exposure, no grade)
    /// and persist, returning the next position.
    #[flutter_rust_bridge::frb(sync)]
    pub fn acquire(&mut self) -> Result<ReviewState> {
        let now = alix::time::now_ms();
        self.session.acquire_current(&mut self.store, now);
        self.store.save()?;
        self.session.poll(&self.store, now);
        Ok(self.session.review_state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_grade_persists_to_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("d.txt");
        std::fs::write(&deck, "# 2 plus 2?\n\t4\n# capital of france?\n\tParis\n").unwrap();
        let mut s = ReviewSession::open(
            deck.to_string_lossy().into_owned(),
            dir.path().to_string_lossy().into_owned(),
        )
        .unwrap();
        assert!(s.state().card.is_some());
        let _ = s.grade(Grade::Pass).unwrap();
        // `open` itself saves once (select persists the resolved depth), so an
        // empty `"cards"` key always exists; only a persisted grade writes the
        // card's FSRS state and history.
        let store_json = std::fs::read_to_string(dir.path().join("progress.json")).unwrap();
        assert!(
            store_json.contains("\"recall\"") && store_json.contains("\"history\""),
            "the graded card's schedule should be persisted, got: {store_json}"
        );
    }
}
