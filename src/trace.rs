use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

use crate::{
    card::SourceCitation,
    deck::{Deck, is_url},
    depth::Depth,
    scheduler::{Fsrs, Grade, Scheduler},
    source::{
        Excerpt, SourceBase, parse_at_origin, parse_line_range, parse_locator, relabel_for_display,
    },
    store::Store,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delta {
    Passed,
    Partial,
    Failed,
}

impl Delta {
    pub fn from_key(c: char) -> Option<Delta> {
        match c.to_ascii_lowercase() {
            'n' => Some(Delta::Passed),
            'p' => Some(Delta::Partial),
            'f' => Some(Delta::Failed),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Delta::Passed => "Got it",
            Delta::Partial => "Partly",
            Delta::Failed => "Missed it",
        }
    }

    /// A `Partial` schedules as FSRS `Hard` (a weak pass, resurfaces sooner); a
    /// `Failed` schedules as `Again` but never derails the walk.
    pub fn grade(self) -> Grade {
        match self {
            Delta::Passed => Grade::Pass,
            Delta::Partial => Grade::Partial,
            Delta::Failed => Grade::Fail,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub prompt: String,
    pub points: Vec<String>,
    pub givens: Vec<String>,
    pub note: Option<String>,
    pub locator: Option<String>,
    pub fingerprint: Option<u64>,
    pub at_origin: Option<String>,
    pub card_id: String,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Trace {
    pub description: String,
    pub subject: String,
    pub source: Option<String>,
    pub links: Vec<String>,
    pub origin_url: Option<String>,
    pub checkpoints: Vec<Checkpoint>,
    pub deck_path: PathBuf,
    pub origin_root: Option<PathBuf>,
    source_base: SourceBase,
}

impl Trace {
    pub fn from_deck(deck: &Deck) -> Result<Trace> {
        let description = deck
            .trace
            .clone()
            .ok_or_else(|| anyhow!("{} is not a trace: it declares no `trace:`", deck.subject))?;
        if deck.cards.is_empty() {
            bail!("the trace `{}` has no checkpoints", deck.subject);
        }
        if let Some(card) = deck.cards.iter().find(|card| card.citations.len() > 1) {
            bail!(
                "trace checkpoint at line {} has multiple `at:` locators; \
                 a checkpoint reveals one contiguous source range",
                card.line
            );
        }
        // A checkpoint needs a stable id (the deck is stamped at open); an
        // unstamped card carries none and is skipped defensively.
        let checkpoints = deck
            .cards
            .iter()
            .filter_map(|c| {
                let citation = c.citations.first();
                Some(Checkpoint {
                    prompt: c.front.clone(),
                    points: c.back.clone(),
                    givens: c.givens.clone(),
                    note: c.note.clone(),
                    locator: citation.map(|citation| citation.locator.clone()),
                    fingerprint: citation.and_then(|citation| citation.fingerprint),
                    at_origin: citation.and_then(|citation| citation.origin.clone()),
                    card_id: c.id()?,
                    line: c.line,
                })
            })
            .collect();
        let source = deck.sources.first().cloned();
        Ok(Trace {
            description,
            subject: deck.subject.clone(),
            source,
            links: deck.reference_links(),
            origin_url: deck.origin_url(),
            checkpoints,
            deck_path: deck.path.clone(),
            origin_root: deck.origin_root(),
            source_base: SourceBase::for_deck(deck),
        })
    }

    /// Drawn from the checkpoints (which already paraphrase the source), so
    /// grading needs no source read.
    pub fn compression_rubric(&self) -> Vec<String> {
        self.checkpoints
            .iter()
            .flat_map(|cp| cp.points.iter().cloned())
            .collect()
    }

    pub fn excerpt(&self, checkpoint: &Checkpoint) -> Result<Excerpt> {
        let locator = checkpoint
            .locator
            .as_deref()
            .ok_or_else(|| anyhow!("this checkpoint has no `at:` locator to reveal"))?;
        self.source_base.checked_excerpt(&SourceCitation {
            locator: locator.to_string(),
            fingerprint: checkpoint.fingerprint,
            origin: checkpoint.at_origin.clone(),
            line: checkpoint.line,
        })
    }

    pub fn frozen_block(&self, checkpoint: &Checkpoint) -> Option<String> {
        checkpoint.at_origin.as_deref()?;
        let excerpt = self.excerpt(checkpoint).ok()?;
        Some(render_frozen_block(
            excerpt,
            checkpoint.at_origin.as_deref(),
        ))
    }

    pub fn lint_locators(&self) -> Vec<LocatorIssue> {
        let mut issues = Vec::new();
        let url_source = self.source.as_deref().is_some_and(is_url);
        for (i, cp) in self.checkpoints.iter().enumerate() {
            let Some(locator) = cp.locator.as_deref() else {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: "no `at:` locator — a walk can't reveal its source".to_string(),
                });
                continue;
            };
            if url_source {
                continue;
            }
            let (file, spec) = parse_locator(locator);
            let Some(path) = self.source_base.locator_path(file.as_deref()) else {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!(
                        "locator `{locator}` gives only line numbers, but `source:` \
                         is not a single file — write it as `file:lines`"
                    ),
                });
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!(
                        "locator `{locator}` → `{}`: file not found or unreadable",
                        path.display()
                    ),
                });
                continue;
            };
            let Some(spec) = spec else { continue };
            let (start, end) = parse_line_range(&spec);
            let n = text.lines().count();
            if start > n {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!(
                        "locator `{locator}` starts at line {start}, but `{}` has only {n} \
                         lines — the source changed; re-point it",
                        path.display()
                    ),
                });
            } else if end > n {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!(
                        "locator `{locator}` ends at line {end}, but `{}` has only {n} lines \
                         — the excerpt is clamped short; re-point it",
                        path.display()
                    ),
                });
            }
        }
        issues
    }
}

/// `checkpoint` is a 0-based index that mirrors the deck's cards 1:1, so a
/// caller can map it back to a deck line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatorIssue {
    pub checkpoint: usize,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Predict,
    Reveal,
    /// Every checkpoint walked; verification is the trace's separate
    /// AI-graded exam, not an ungraded compression step here.
    Done,
}

pub struct Walk {
    trace: Trace,
    current: usize,
    phase: Phase,
    predictions: Vec<String>,
    deltas: Vec<Option<Delta>>,
}

impl Walk {
    pub fn new(trace: Trace) -> Walk {
        let n = trace.checkpoints.len();
        Walk {
            trace,
            current: 0,
            phase: Phase::Predict,
            predictions: vec![String::new(); n],
            deltas: vec![None; n],
        }
    }

    pub fn trace(&self) -> &Trace {
        &self.trace
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn total(&self) -> usize {
        self.trace.checkpoints.len()
    }
    pub fn current_index(&self) -> usize {
        self.current
    }
    pub fn checkpoint(&self) -> Option<&Checkpoint> {
        self.trace.checkpoints.get(self.current)
    }

    /// No-op outside [`Phase::Predict`].
    pub fn predict(&mut self, text: String) {
        if self.phase != Phase::Predict {
            return;
        }
        if let Some(slot) = self.predictions.get_mut(self.current) {
            *slot = text;
        }
        self.phase = Phase::Reveal;
    }

    /// No-op outside [`Phase::Reveal`]. Updates `store` but does not save it
    /// (the caller saves).
    pub fn grade(&mut self, store: &mut Store, delta: Delta, now_ms: u64) {
        if self.phase != Phase::Reveal {
            return;
        }
        if let Some(checkpoint) = self.trace.checkpoints.get(self.current) {
            // The walk grades with no Session, so it's itself an
            // entry-creation site: write records before the schedule entry.
            store.ensure_records_raw(&checkpoint.card_id, &[]);
            let state = store.get_or_insert(&checkpoint.card_id, now_ms);
            Fsrs::default().apply(state, Depth::Recall, delta.grade(), now_ms, false);
        }
        self.deltas[self.current] = Some(delta);
        if self.current + 1 < self.trace.checkpoints.len() {
            self.current += 1;
            self.phase = Phase::Predict;
        } else {
            self.phase = Phase::Done;
        }
    }

    pub fn prediction(&self, i: usize) -> Option<&str> {
        self.predictions.get(i).map(String::as_str)
    }
    pub fn delta(&self, i: usize) -> Option<Delta> {
        self.deltas.get(i).copied().flatten()
    }

    pub fn summary(&self) -> Summary {
        let mut s = Summary::default();
        for (i, delta) in self.deltas.iter().enumerate() {
            match delta {
                Some(Delta::Passed) => s.passed += 1,
                Some(Delta::Partial) => {
                    s.partly += 1;
                    s.weak.push(i);
                }
                Some(Delta::Failed) => {
                    s.failed += 1;
                    s.weak.push(i);
                }
                None => {}
            }
        }
        s
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub passed: usize,
    pub partly: usize,
    pub failed: usize,
    /// Indices of checkpoints judged partly or failed (SRS resurfaces them
    /// sooner).
    pub weak: Vec<usize>,
}

fn render_frozen_block(excerpt: Excerpt, at_origin: Option<&str>) -> String {
    let (excerpt, label) = relabel_for_display(excerpt, at_origin);
    let mut s = String::new();
    if let Some(label) = label {
        s.push_str(&label);
        s.push('\n');
    }
    for (n, text) in &excerpt.lines {
        s.push_str(&format!("{n}\t{text}\n"));
    }
    s
}

#[derive(Debug)]
pub struct Drift {
    pub line: usize,
    pub at: String,
    pub gone: bool,
}

/// A *moved* excerpt that's otherwise unchanged is NOT flagged (the block is
/// searched across the whole file).
pub fn drifted_cards(deck: &Deck) -> Vec<Drift> {
    let Some(origin_root) = deck.origin_root() else {
        return Vec::new();
    };
    let source_base = SourceBase::for_deck(deck);
    let mut out = Vec::new();
    for card in &deck.cards {
        for citation in &card.citations {
            let Some(at_origin) = citation.origin.as_deref() else {
                continue;
            };
            let Some((file, _)) = parse_at_origin(Some(at_origin)) else {
                continue;
            };
            let Ok(frozen) = source_base.excerpt(&citation.locator) else {
                continue;
            };
            match std::fs::read_to_string(origin_root.join(&file)) {
                Err(_) => out.push(Drift {
                    line: card.line,
                    at: at_origin.to_string(),
                    gone: true,
                }),
                Ok(live) if !excerpt_occurs_in(&frozen, &live) => out.push(Drift {
                    line: card.line,
                    at: at_origin.to_string(),
                    gone: false,
                }),
                Ok(_) => {}
            }
        }
    }
    out
}

/// Trailing whitespace is ignored, so reformatted line endings don't read
/// as drift.
fn excerpt_occurs_in(frozen: &Excerpt, live: &str) -> bool {
    let block = frozen
        .lines
        .iter()
        .map(|(_, t)| t.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    if block.trim().is_empty() {
        return true;
    }
    let live_norm = live
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    live_norm.contains(&block)
}

pub fn frozen_excerpt_block(citation: &SourceCitation, source_base: &SourceBase) -> Option<String> {
    citation.origin.as_deref()?;
    let excerpt = source_base.checked_excerpt(citation).ok()?;
    Some(render_frozen_block(excerpt, citation.origin.as_deref()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        // Mirrors production stamping (a trace's per-checkpoint SRS keys on
        // the card token); source files are left untouched.
        if name.ends_with(".md") {
            let _ = crate::stamp::stamp_deck(&path);
        }
        path
    }

    #[test]
    fn delta_label_names_each_grade_for_the_learner() {
        assert_eq!("Got it", Delta::Passed.label());
        assert_eq!("Partly", Delta::Partial.label());
        assert_eq!("Missed it", Delta::Failed.label());
    }

    #[test]
    fn delta_keys_and_grades() {
        assert_eq!(Some(Delta::Passed), Delta::from_key('N'));
        assert_eq!(Some(Delta::Partial), Delta::from_key('p'));
        assert_eq!(Some(Delta::Failed), Delta::from_key('f'));
        assert_eq!(None, Delta::from_key('x'));
        assert_eq!(Grade::Pass, Delta::Passed.grade());
        assert_eq!(Grade::Partial, Delta::Partial.grade());
        assert_eq!(Grade::Fail, Delta::Failed.grade());
    }

    #[test]
    fn walking_a_trace_drills_but_does_not_master_it() {
        use crate::{deck::DeckState, store::Store};
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.rs", "a\nb\nc\n");
        let deck_path = dir.path().join("t.md");
        std::fs::write(
            &deck_path,
            format!(
                "---\ntrace: how a moves\nsource: {}\n---\n## what happens?\nit advances\n<!-- at: 1-2 -->\n",
                dir.path().join("src.rs").display()
            ),
        )
        .unwrap();
        // Stamped at open in production, so the checkpoint carries a token id.
        crate::stamp::stamp_deck(&deck_path).unwrap();
        let deck = crate::deck::Deck::load(&deck_path).unwrap();
        let deck_id = deck.deck_token.clone().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        assert!(!store.deck_mastered(&deck_id));
        assert_eq!(DeckState::NotStarted, deck.state(&store));

        // Manually graduates the checkpoint (FSRS `Review`) to satisfy the
        // deck's unlock gate (every card graduated).
        let card0 = Trace::from_deck(&deck).unwrap().checkpoints[0]
            .card_id
            .clone();
        let mut walk = Walk::new(Trace::from_deck(&deck).unwrap());
        walk.predict("p".to_string());
        walk.grade(&mut store, Delta::Passed, 1);
        assert_eq!(Phase::Done, walk.phase());
        if let Some(f) = store.get_or_insert(&card0, 0).recall.as_mut() {
            f.state = 2; // Review state (graduated)
        }

        assert!(!store.deck_mastered(&deck_id));
        assert_eq!(DeckState::ExamDue, deck.state(&store));

        store.set_deck_mastered(&deck_id, 99);
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    fn trace_deck(dir: &Path) -> Deck {
        write(dir, "source.txt", "first\nsecond\nthird\nfourth\n");
        let path = write(
            dir,
            "t.md",
            "---\ntrace: how it works\n\
             source: source.txt\n---\n\
             ## Predict the first hop\n\
             <!-- given: line — the current input line -->\n\
             it reads the first line\n\
             <!-- at: 1 -->\n\
             > the entry point\n\
             ## Predict the second hop\n\
             it reads lines two and three\n\
             <!-- at: 2-3 -->\n",
        );
        crate::source::stamp_citations(&path).unwrap();
        Deck::load(&path).unwrap()
    }

    #[test]
    fn from_deck_builds_checkpoints_and_rejects_non_traces() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        assert_eq!("how it works", trace.description);
        assert_eq!(2, trace.checkpoints.len());
        assert_eq!("Predict the first hop", trace.checkpoints[0].prompt);
        assert_eq!(
            vec!["line — the current input line".to_string()],
            trace.checkpoints[0].givens
        );
        assert!(trace.checkpoints[1].givens.is_empty());
        assert_eq!(Some("1".to_string()), trace.checkpoints[0].locator);
        assert_eq!(
            Some("the entry point".to_string()),
            trace.checkpoints[0].note
        );

        let plain = write(dir.path(), "p.md", "## q\na\n");
        let err = Trace::from_deck(&Deck::load(&plain).unwrap()).unwrap_err();
        assert!(format!("{err:#}").contains("not a trace"));
    }

    #[test]
    fn a_trace_checkpoint_rejects_multiple_source_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: source.txt\n---\n\
             ## q\na\n<!-- at: 1 -->\n<!-- at: 2 -->\n",
        );
        write(dir.path(), "source.txt", "one\ntwo\n");
        let err = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap_err();
        assert!(
            format!("{err:#}").contains("multiple `at:` locators"),
            "{err:#}"
        );
    }

    #[test]
    fn excerpt_reads_live_from_the_single_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let ex = trace.excerpt(&trace.checkpoints[0]).unwrap();
        assert_eq!(vec![(1, "first".to_string())], ex.lines);
        let ex = trace.excerpt(&trace.checkpoints[1]).unwrap();
        assert_eq!(
            vec![(2, "second".to_string()), (3, "third".to_string())],
            ex.lines
        );
    }

    #[test]
    fn a_trace_does_not_reveal_a_relocated_excerpt_before_repair() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        write(
            dir.path(),
            "source.txt",
            "inserted\nfirst\nsecond\nthird\nfourth\n",
        );
        let trace = Trace::from_deck(&deck).unwrap();
        let error = trace.excerpt(&trace.checkpoints[0]).unwrap_err();
        assert!(format!("{error:#}").contains("moved to `2`"));
    }

    #[test]
    fn tutor_grounding_drops_a_changed_frozen_excerpt() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join("assets/deck1");
        std::fs::create_dir_all(&assets).unwrap();
        let name = crate::assets::object_name(b"fn expected() {}\n", "rs");
        write(&assets, &name, "fn expected() {}\n");
        let deck_path = write(
            dir.path(),
            "t.md",
            &format!(
                "---\nsource: assets/deck1/{name}\n---\n\
                 ## q\na\n<!-- at: {name}:1 from src/lib.rs:1 -->\n"
            ),
        );
        crate::source::stamp_citations(&deck_path).unwrap();
        write(&assets, &name, "fn changed() {}\n");
        let deck = Deck::load(&deck_path).unwrap();
        let base = SourceBase::for_deck(&deck);
        assert!(frozen_excerpt_block(&deck.cards[0].citations[0], &base).is_none());
    }

    #[test]
    fn line_only_locator_needs_a_single_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: .\n---\n## q\na\n<!-- at: 1 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let err = trace.excerpt(&trace.checkpoints[0]).unwrap_err();
        assert!(format!("{err:#}").contains("not a single file"));
    }

    #[test]
    fn lint_locators_passes_a_valid_trace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\nfour\nfive\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: src.txt\n---\n## q\na\n<!-- at: 2-3 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        assert!(trace.lint_locators().is_empty());
    }

    #[test]
    fn lint_locators_flags_a_start_past_eof() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: src.txt\n---\n## q\na\n<!-- at: 5-6 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert_eq!(0, issues[0].checkpoint);
        assert!(issues[0].message.contains("only 3"));
    }

    /// A range whose end runs past EOF is silently clamped at walk time, so
    /// `check` flags it too.
    #[test]
    fn lint_locators_flags_a_clamped_end() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: src.txt\n---\n## q\na\n<!-- at: 2-9 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("clamped"));
    }

    #[test]
    fn lint_locators_flags_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: .\n---\n## q\na\n<!-- at: nope.rs:1-2 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("not found"));
    }

    #[test]
    fn lint_locators_flags_a_missing_locator() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: src.txt\n---\n## q\na\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("no `at:`"));
    }

    #[test]
    fn lint_locators_flags_line_only_without_a_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nsource: .\n---\n## q\na\n<!-- at: 1 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("not a single file"));
    }

    #[test]
    fn walk_runs_predict_reveal_grade_to_done() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let card0 = trace.checkpoints[0].card_id.clone();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace);

        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(2, walk.total());

        walk.predict("my guess".to_string());
        assert_eq!(Phase::Reveal, walk.phase());
        assert_eq!(Some("my guess"), walk.prediction(0));
        walk.grade(&mut store, Delta::Passed, 1000);
        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(1, walk.current_index());
        assert!(store.get(&card0).unwrap().recall.is_some());

        let card1 = walk.checkpoint().unwrap().card_id.clone();
        walk.predict(String::new());
        walk.grade(&mut store, Delta::Failed, 1001);
        assert_eq!(Phase::Done, walk.phase());
        assert_eq!(0, store.get(&card1).unwrap().streak);

        let summary = walk.summary();
        assert_eq!(1, summary.passed);
        assert_eq!(1, summary.failed);
        assert_eq!(vec![1], summary.weak);
    }

    #[test]
    fn a_trace_walk_grade_creates_an_entry_with_records() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let card0 = trace.checkpoints[0].card_id.clone();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        assert!(store.get(&card0).is_none(), "no entry before the grade");
        let mut walk = Walk::new(trace);
        walk.predict("guess".to_string());
        walk.grade(&mut store, Delta::Passed, 1000);
        assert!(store.get(&card0).is_some(), "the grade created the entry");
        let rec = store.records(&card0).expect("records exist alongside it");
        assert_eq!(crate::store::FP_VERSION, rec.version);
        assert!(rec.holes.is_empty(), "a trace card is a plain card");
    }

    #[test]
    fn partly_records_a_weak_success_on_a_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace);
        let card0 = walk.checkpoint().unwrap().card_id.clone();
        walk.predict("guess".to_string());
        walk.grade(&mut store, Delta::Partial, 1000);
        let state = store.get(&card0).unwrap();
        assert!(state.recall.is_some());
        assert_eq!(1, state.total_passes);
        assert_eq!(1, walk.summary().partly);
    }

    #[test]
    fn grade_is_a_noop_outside_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut walk = Walk::new(trace);
        walk.grade(&mut store, Delta::Passed, 1000);
        assert_eq!(Phase::Predict, walk.phase());
        assert_eq!(0, walk.current_index());
        assert!(store.is_empty());
    }

    #[cfg(feature = "full")]
    fn frozen_workspace(root: &Path) -> PathBuf {
        std::fs::create_dir_all(root.join("src")).unwrap();
        write(&root.join("src"), "a.rs", "alpha\nbeta\ngamma\n");
        write(&root.join("src"), "b.rs", "one\ntwo\n");
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(
            &root.join("ws"),
            "alix.toml",
            "title = \"W\"\n\n[defaults]\norigin = \"../src\"\n",
        );
        write(
            &root.join("ws/decks"),
            "t.md",
            "---\ntrace: how it works\n\
             source: ../src\n---\n\
             ## hop 1\nit reads a\n<!-- at: a.rs:2-3 -->\n\
             ## hop 2\nit reads b\n<!-- at: b.rs:1 -->\n",
        )
    }

    #[cfg(feature = "full")]
    #[test]
    fn drifted_cards_flags_a_changed_or_missing_excerpt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = frozen_workspace(root);
        crate::assets::initialize(&deck_path).unwrap();

        assert!(drifted_cards(&Deck::load(&deck_path).unwrap()).is_empty());

        std::fs::write(root.join("src/a.rs"), "alpha\nCHANGED\nLINES\n").unwrap();
        let d = drifted_cards(&Deck::load(&deck_path).unwrap());
        assert_eq!(1, d.len(), "{d:?}");
        assert!(!d[0].gone);
        assert_eq!("a.rs:2-3", d[0].at);

        std::fs::remove_file(root.join("src/a.rs")).unwrap();
        let d = drifted_cards(&Deck::load(&deck_path).unwrap());
        assert!(d.iter().any(|x| x.gone && x.at == "a.rs:2-3"), "{d:?}");
    }
}
