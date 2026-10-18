use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

use crate::{
    card::SourceCitation,
    cite::{CitationRoots, Resolved, Root},
    deck::Deck,
    depth::Depth,
    locator::{Locator, Target},
    scheduler::{Fsrs, Grade, Scheduler},
    source::Excerpt,
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
    /// `Failed` schedules as `Again` but never derails the trace.
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
    pub locator: Option<Locator>,
    pub fingerprint: Option<u64>,
    pub asset: Option<String>,
    pub card_id: String,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Trace {
    pub description: String,
    pub subject: String,
    pub source: Option<String>,
    pub links: Vec<String>,
    pub exam_layers: crate::deck::ExamLayers,
    pub checkpoints: Vec<Checkpoint>,
    pub deck_path: PathBuf,
    roots: CitationRoots,
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
                    points: crate::render::gradeable_answer_lines(
                        c,
                        crate::card::AnswerSpace::Authored,
                    ),
                    givens: c.givens.clone(),
                    note: c.notes_text(),
                    locator: citation.map(|citation| citation.locator.clone()),
                    fingerprint: citation.and_then(|citation| citation.fingerprint),
                    asset: citation.and_then(|citation| citation.asset.clone()),
                    card_id: c.id()?,
                    line: c.line,
                })
            })
            .collect();
        let source = deck.exam.first().cloned();
        Ok(Trace {
            description,
            subject: deck.subject.clone(),
            source,
            links: deck.reference_links(),
            exam_layers: deck.exam_layers(),
            checkpoints,
            deck_path: deck.path.clone(),
            roots: CitationRoots::for_deck(deck),
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
            .as_ref()
            .ok_or_else(|| anyhow!("this checkpoint has no `at:` locator to reveal"))?;
        self.roots.checked_excerpt(&SourceCitation {
            locator: locator.clone(),
            fingerprint: checkpoint.fingerprint,
            asset: checkpoint.asset.clone(),
            line: checkpoint.line,
        })
    }

    pub fn browse_root(&self, checkpoint: &Checkpoint) -> Option<PathBuf> {
        self.roots.browse_root(checkpoint.locator.as_ref())
    }

    pub fn frozen_block(&self, checkpoint: &Checkpoint) -> Option<String> {
        checkpoint.asset.as_deref()?;
        let excerpt = self.excerpt(checkpoint).ok()?;
        Some(render_frozen_block(excerpt, checkpoint.locator.as_ref()?))
    }

    pub fn lint_locators(&self) -> Vec<LocatorIssue> {
        let mut issues = Vec::new();
        for (i, cp) in self.checkpoints.iter().enumerate() {
            let Some(locator) = cp.locator.as_ref() else {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: "no `at:` locator, so a trace can't reveal its source".to_string(),
                });
                continue;
            };
            let outcome = if cp.asset.is_some() {
                self.roots
                    .check(locator, true)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            } else {
                self.roots
                    .live_excerpt(locator)
                    .map(|_| ())
                    .map_err(|error| format!("{error:#}"))
            };
            if let Err(message) = outcome {
                issues.push(LocatorIssue {
                    checkpoint: i,
                    message: format!("locator `{locator}`: {message}"),
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
    /// Every checkpoint traced; verification is the trace's separate
    /// AI-graded exam, not an ungraded compression step here.
    Done,
}

pub struct TraceSession {
    trace: Trace,
    scheduler: Fsrs,
    current: usize,
    phase: Phase,
    predictions: Vec<String>,
    deltas: Vec<Option<Delta>>,
}

impl TraceSession {
    pub fn new(trace: Trace, scheduler: Fsrs) -> TraceSession {
        let n = trace.checkpoints.len();
        TraceSession {
            trace,
            scheduler,
            current: 0,
            phase: Phase::Predict,
            predictions: vec![String::new(); n],
            deltas: vec![None; n],
        }
    }

    pub fn restart(&mut self) {
        let n = self.trace.checkpoints.len();
        self.current = 0;
        self.phase = Phase::Predict;
        self.predictions = vec![String::new(); n];
        self.deltas = vec![None; n];
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
            let state = store.get_or_insert(&checkpoint.card_id);
            self.scheduler
                .apply(state, Depth::Recall, delta.grade(), now_ms, false);
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

fn render_frozen_block(excerpt: Excerpt, locator: &Locator) -> String {
    let mut s = format!("{locator}\n");
    for (n, text) in &excerpt.lines {
        s.push_str(&format!("{n}\t{text}\n"));
    }
    s
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriftKind {
    /// The file named by `at:` is not there.
    SourceGone,
    /// The frozen bytes are nowhere in the file; only a reader can say whether
    /// the card still teaches the truth.
    Rewritten,
    /// The frozen bytes are intact, at a line `at:` does not name. The evidence
    /// still holds, so only the address is wrong.
    Moved { start: usize, len: usize },
    /// The frozen bytes occur more than once, so which one `at:` means is not
    /// decidable here.
    Ambiguous,
}

#[derive(Clone, Debug)]
pub struct Drift {
    pub line: usize,
    pub at_line: usize,
    pub at: Locator,
    pub kind: DriftKind,
}

/// Reports a moved excerpt as well as a lost one: an `at:` range that no longer
/// holds its own evidence points a reader at unrelated code, and says nothing
/// while doing it. Each frozen locator is judged against its own key's root;
/// a passage under a url root has no live side and never drifts.
pub fn drifted_cards(deck: &Deck) -> Vec<Drift> {
    let roots = CitationRoots::for_deck(deck);
    let mut out = Vec::new();
    for card in &deck.cards {
        for citation in &card.citations {
            if citation.asset.is_none() {
                continue;
            }
            let locator = &citation.locator;
            let Ok(root) = roots.check(locator, true) else {
                continue;
            };
            if matches!(root.root, Root::Url(_)) {
                continue;
            }
            let Ok(frozen) = roots.citation_excerpt(citation) else {
                continue;
            };
            let block: Vec<String> = frozen
                .lines
                .iter()
                .map(|(_, text)| text.trim_end().to_string())
                .collect();
            let mut drift = |kind| {
                out.push(Drift {
                    line: card.line,
                    at_line: citation.line,
                    at: locator.clone(),
                    kind,
                })
            };
            let live = match roots.resolve(locator, true) {
                Ok(Resolved::File { path, .. }) => std::fs::read_to_string(path).ok(),
                _ => None,
            };
            let Some(live) = live else {
                drift(DriftKind::SourceGone);
                continue;
            };
            let live_lines: Vec<&str> = live.lines().collect();
            let Target::File {
                range: Some((recorded, _)),
                ..
            } = locator.target
            else {
                if !block
                    .iter()
                    .map(String::as_str)
                    .eq(live_lines.iter().map(|line| line.trim_end()))
                {
                    drift(DriftKind::Rewritten);
                }
                continue;
            };
            match locate_block(&block, &live_lines) {
                Placement::Everywhere => {}
                Placement::Nowhere => drift(DriftKind::Rewritten),
                Placement::Many => drift(DriftKind::Ambiguous),
                Placement::Once(start) => {
                    if recorded as usize != start {
                        drift(DriftKind::Moved {
                            start,
                            len: block.len(),
                        });
                    }
                }
            }
        }
    }
    out
}

enum Placement {
    /// A blank excerpt pins nothing, so it can never be misplaced.
    Everywhere,
    Nowhere,
    Once(usize),
    Many,
}

/// Trailing whitespace is ignored, so reformatted line endings don't read as
/// drift. Matching is line-aligned: a block whose first line is only a suffix
/// of some live line is not a match.
fn locate_block(block: &[String], live: &[&str]) -> Placement {
    if block.iter().all(|line| line.trim().is_empty()) {
        return Placement::Everywhere;
    }
    if live.len() < block.len() {
        return Placement::Nowhere;
    }
    let mut first = None;
    for offset in 0..=live.len() - block.len() {
        let matched = live[offset..offset + block.len()]
            .iter()
            .zip(block)
            .all(|(live, frozen)| live.trim_end() == frozen.trim_end());
        if !matched {
            continue;
        }
        if first.is_some() {
            return Placement::Many;
        }
        first = Some(offset + 1);
    }
    first.map_or(Placement::Nowhere, Placement::Once)
}

pub fn frozen_excerpt_block(citation: &SourceCitation, roots: &CitationRoots) -> Option<String> {
    citation.asset.as_deref()?;
    let excerpt = roots.checked_excerpt(citation).ok()?;
    Some(render_frozen_block(excerpt, &citation.locator))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// The scan window is `0..=live.len() - block.len()`, so a block that
    /// cannot fit has to be turned away before the subtraction, and a block
    /// that fits exactly still has to be looked for.
    #[test]
    fn a_block_that_cannot_fit_is_nowhere_and_one_that_fills_the_file_is_found() {
        let block = vec!["a".to_string(), "b".to_string()];
        assert!(matches!(locate_block(&block, &["a"]), Placement::Nowhere));
        assert!(matches!(
            locate_block(&block, &["a", "b"]),
            Placement::Once(1)
        ));
    }

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
    fn tracing_a_trace_drills_but_does_not_master_it() {
        use crate::{deck::DeckState, store::Store};
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.rs", "a\nb\nc\n");
        let deck_path = dir.path().join("t.md");
        std::fs::write(
            &deck_path,
            format!(
                "---\ntrace: how a moves\nexam: {}\ncite:\n  - src: .\n---\n## what happens?\nit advances\n<!-- at: src:src.rs:1-2 -->\n",
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
        let mut session = TraceSession::new(Trace::from_deck(&deck).unwrap(), Fsrs::default());
        session.predict("p".to_string());
        session.grade(&mut store, Delta::Passed, 1);
        assert_eq!(Phase::Done, session.phase());
        if let Some(f) = store.get_or_insert(&card0).recall.as_mut() {
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
             exam: source.txt\ncite:\n  - src: .\n---\n\
             ## Predict the first hop\n\
             it reads the first line\n\
             > [!NOTE]\n\
             > the entry point\n\
             <!-- given: line — the current input line -->\n\
             <!-- at: src:source.txt:1 -->\n\
             ## Predict the second hop\n\
             it reads lines two and three\n\
             <!-- at: src:source.txt:2-3 -->\n",
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
        assert_eq!(
            Some("src:source.txt:1".to_string()),
            trace.checkpoints[0]
                .locator
                .as_ref()
                .map(ToString::to_string)
        );
        assert_eq!(
            Some("the entry point".to_string()),
            trace.checkpoints[0].note
        );

        let plain = write(dir.path(), "p.md", "## q\na\n");
        let err = Trace::from_deck(&Deck::load(&plain).unwrap()).unwrap_err();
        assert!(format!("{err:#}").contains("not a trace"));
    }

    #[test]
    fn compression_rubric_preserves_every_checkpoint_point_in_trace_order() {
        let dir = tempfile::tempdir().unwrap();
        let trace = Trace::from_deck(&trace_deck(dir.path())).unwrap();

        assert_eq!(
            vec![
                "it reads the first line".to_string(),
                "it reads lines two and three".to_string(),
            ],
            trace.compression_rubric()
        );
    }

    #[test]
    fn trace_points_and_exam_rubric_exclude_supporting_quotations() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "quoted.md",
            "---\ntrace: how it works\n---\n\
             ## Predict the hop\n\
             the checkpoint claim\n\
             > supporting quotation\n\
             > continued quotation\n",
        );
        crate::stamp::stamp_deck(&path).unwrap();
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();

        assert_eq!(trace.checkpoints[0].points, ["the checkpoint claim"]);
        assert_eq!(
            trace.compression_rubric(),
            ["the checkpoint claim"],
            "the final trace exam grades claims, not supporting source prose"
        );
    }

    #[test]
    fn a_trace_checkpoint_rejects_multiple_source_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nexam: source.txt\ncite:\n  - src: .\n---\n\
             ## q\na\n<!-- at: src:source.txt:1 -->\n<!-- at: src:source.txt:2 -->\n",
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
        assert!(format!("{error:#}").contains("moved to `src:source.txt:2`"));
    }

    #[test]
    fn tutor_grounding_drops_a_changed_frozen_excerpt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "").unwrap();
        let assets = dir.path().join("assets/deck-deck1");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(dir.path().join("decks")).unwrap();
        let name = crate::assets::object_name(b"fn expected() {}\n", "rs");
        write(&assets, &name, "fn expected() {}\n");
        let fingerprint = crate::source::format_locator_fingerprint(
            crate::source::excerpt_fingerprint(&Excerpt {
                path: PathBuf::from("src/lib.rs"),
                lines: vec![(1, "fn expected() {}".to_string())],
                truncated: false,
            }),
        );
        let deck_path = write(
            &dir.path().join("decks"),
            "t.md",
            &format!(
                "---\nformat-version: 1\nid: \"deck-deck1\"\nexam: src/lib.rs\ncite:\n  - src: src\n---\n\
                 ## q\na\n<!-- at: src:lib.rs:1 fingerprint: {fingerprint} asset: {name} -->\n"
            ),
        );
        let deck = Deck::load(&deck_path).unwrap();
        assert!(
            frozen_excerpt_block(&deck.cards[0].citations[0], &CitationRoots::for_deck(&deck))
                .is_some()
        );
        write(&assets, &name, "fn changed() {}\n");
        let deck = Deck::load(&deck_path).unwrap();
        let base = CitationRoots::for_deck(&deck);
        assert!(frozen_excerpt_block(&deck.cards[0].citations[0], &base).is_none());
    }

    #[test]
    fn a_frozen_block_numbers_the_excerpt_from_the_at_start_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "").unwrap();
        let assets = dir.path().join("assets/deck-deck1");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(dir.path().join("decks")).unwrap();
        let name = crate::assets::object_name(b"alpha\nbeta\ngamma\n", "rs");
        write(&assets, &name, "alpha\nbeta\ngamma\n");
        let fingerprint = crate::source::format_locator_fingerprint(
            crate::source::excerpt_fingerprint(&Excerpt {
                path: PathBuf::from("src/lib.rs"),
                lines: vec![
                    (46, "alpha".to_string()),
                    (47, "beta".to_string()),
                    (48, "gamma".to_string()),
                ],
                truncated: false,
            }),
        );
        let deck_path = write(
            &dir.path().join("decks"),
            "t.md",
            &format!(
                "---\nformat-version: 1\nid: \"deck-deck1\"\nexam: src\ncite:\n  - code: .\n---\n\
                 ## q\na\n<!-- at: code:src/lib.rs:46-48 fingerprint: {fingerprint} asset: {name} -->\n"
            ),
        );
        let deck = Deck::load(&deck_path).unwrap();
        let block =
            frozen_excerpt_block(&deck.cards[0].citations[0], &CitationRoots::for_deck(&deck))
                .unwrap();
        assert!(block.contains("code:src/lib.rs:46-48"), "{block}");
        assert!(block.contains("46\talpha"), "{block}");
        assert!(block.contains("48\tgamma"), "{block}");
        assert!(!block.contains("1\talpha"), "{block}");
    }

    #[cfg(feature = "full")]
    #[test]
    fn trace_frozen_block_returns_the_owned_excerpt_not_a_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = frozen_workspace(dir.path());
        crate::assets::initialize(&deck_path).unwrap();
        let trace = Trace::from_deck(&Deck::load(&deck_path).unwrap()).unwrap();

        let block = trace.frozen_block(&trace.checkpoints[0]).unwrap();
        assert!(block.contains("src:a.rs:2-3"), "{block}");
        assert!(block.contains("2\tbeta"), "{block}");
        assert!(block.contains("3\tgamma"), "{block}");
    }

    #[test]
    fn a_checkpoint_under_an_unknown_key_cannot_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nexam: .\n---\n## q\na\n<!-- at: src:x.txt:1 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let err = trace.excerpt(&trace.checkpoints[0]).unwrap_err();
        assert!(
            format!("{err:#}").contains("names no `cite:` root"),
            "{err:#}"
        );
    }

    #[test]
    fn lint_locators_passes_a_valid_trace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\nfour\nfive\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nexam: src.txt\ncite:\n  - src: .\n---\n## q\na\n<!-- at: src:src.txt:2-3 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        assert!(trace.lint_locators().is_empty());
    }

    #[test]
    fn lint_locators_accepts_a_range_ending_on_the_sources_last_line() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nexam: src.txt\ncite:\n  - src: .\n---\n## q\na\n<!-- at: src:src.txt:3 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();

        assert!(trace.lint_locators().is_empty());
    }

    #[test]
    fn lint_locators_never_reads_a_url_root_and_wants_its_passage_frozen() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "").unwrap();
        let assets = dir.path().join("assets/deck-deck1");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(dir.path().join("decks")).unwrap();
        let name = crate::assets::object_name(b"the cited passage\n", "md");
        write(&assets, &name, "the cited passage\n");
        let fingerprint = crate::source::format_locator_fingerprint(
            crate::source::excerpt_fingerprint(&Excerpt {
                path: PathBuf::new(),
                lines: vec![(1, "the cited passage".to_string())],
                truncated: false,
            }),
        );
        let head = "---\nformat-version: 1\nid: \"deck-deck1\"\ntrace: g\n\
                    exam: https://example.com/guide\ncite:\n  - guide: https://example.com/guide\n---\n";
        let frozen = write(
            &dir.path().join("decks"),
            "frozen.md",
            &format!(
                "{head}## q\na\n<!-- at: guide fingerprint: {fingerprint} asset: {name} -->\n"
            ),
        );
        let trace = Trace::from_deck(&Deck::load(&frozen).unwrap()).unwrap();
        assert!(
            trace.lint_locators().is_empty(),
            "{:?}",
            trace.lint_locators()
        );
        assert!(
            trace.excerpt(&trace.checkpoints[0]).is_ok(),
            "the frozen passage reveals without any fetch"
        );

        let live = write(
            &dir.path().join("decks"),
            "live.md",
            &format!("{head}## q\na\n<!-- at: guide -->\n"),
        );
        let trace = Trace::from_deck(&Deck::load(&live).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len(), "{issues:?}");
        assert!(issues[0].message.contains("no `asset:`"), "{issues:?}");
    }

    #[cfg(feature = "full")]
    #[test]
    fn lint_locators_trusts_owned_assets_after_the_live_source_disappears() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = frozen_workspace(dir.path());
        crate::assets::initialize(&deck_path).unwrap();
        std::fs::remove_dir_all(dir.path().join("src")).unwrap();
        let trace = Trace::from_deck(&Deck::load(&deck_path).unwrap()).unwrap();

        assert!(trace.lint_locators().is_empty());
    }

    #[test]
    fn lint_locators_flags_a_start_past_eof() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nexam: src.txt\ncite:\n  - src: .\n---\n## q\na\n<!-- at: src:src.txt:5-6 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert_eq!(0, issues[0].checkpoint);
        assert!(
            issues[0].message.contains("lines 5-6 run past its end"),
            "{issues:?}"
        );
        assert!(issues[0].message.contains("3 line(s)"), "{issues:?}");
    }

    #[test]
    fn lint_locators_flags_an_end_past_eof() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\nthree\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nexam: src.txt\ncite:\n  - src: .\n---\n## q\na\n<!-- at: src:src.txt:2-9 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(
            issues[0].message.contains("lines 2-9 run past its end"),
            "{issues:?}"
        );
    }

    #[test]
    fn lint_locators_flags_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nexam: .\ncite:\n  - src: .\n---\n## q\na\n<!-- at: src:nope.rs:1-2 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("not found"), "{issues:?}");
    }

    #[test]
    fn lint_locators_flags_a_missing_locator() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src.txt", "one\ntwo\n");
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nexam: src.txt\n---\n## q\na\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(issues[0].message.contains("no `at:`"));
    }

    #[test]
    fn lint_locators_flags_an_unknown_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "t.md",
            "---\ntrace: g\nexam: .\n---\n## q\na\n<!-- at: src:x.txt:1 -->\n",
        );
        let trace = Trace::from_deck(&Deck::load(&path).unwrap()).unwrap();
        let issues = trace.lint_locators();
        assert_eq!(1, issues.len());
        assert!(
            issues[0].message.contains("names no `cite:` root"),
            "{issues:?}"
        );
    }

    #[test]
    fn trace_runs_predict_reveal_grade_to_done() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let card0 = trace.checkpoints[0].card_id.clone();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut session = TraceSession::new(trace, crate::scheduler::Fsrs::default());

        assert_eq!(Phase::Predict, session.phase());
        assert_eq!(2, session.total());

        session.predict("my guess".to_string());
        assert_eq!(Phase::Reveal, session.phase());
        assert_eq!(Some("my guess"), session.prediction(0));
        session.grade(&mut store, Delta::Passed, 1000);
        assert_eq!(Phase::Predict, session.phase());
        assert_eq!(1, session.current_index());
        assert!(store.get(&card0).unwrap().recall.is_some());

        let card1 = session.checkpoint().unwrap().card_id.clone();
        session.predict(String::new());
        session.grade(&mut store, Delta::Failed, 1001);
        assert_eq!(Phase::Done, session.phase());
        assert_eq!(0, store.get(&card1).unwrap().streak);

        let summary = session.summary();
        assert_eq!(1, summary.passed);
        assert_eq!(1, summary.failed);
        assert_eq!(vec![1], summary.weak);
    }

    #[test]
    fn a_trace_grade_creates_an_entry_with_records() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let card0 = trace.checkpoints[0].card_id.clone();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        assert!(store.get(&card0).is_none(), "no entry before the grade");
        let mut session = TraceSession::new(trace, crate::scheduler::Fsrs::default());
        session.predict("guess".to_string());
        session.grade(&mut store, Delta::Passed, 1000);
        assert!(store.get(&card0).is_some(), "the grade created the entry");
    }

    #[test]
    fn partly_records_a_weak_success_on_a_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut session = TraceSession::new(trace, crate::scheduler::Fsrs::default());
        let card0 = session.checkpoint().unwrap().card_id.clone();
        session.predict("guess".to_string());
        session.grade(&mut store, Delta::Partial, 1000);
        let state = store.get(&card0).unwrap();
        assert!(state.recall.is_some());
        assert_eq!(1, state.total_passes);
        assert_eq!(1, session.summary().partly);
    }

    #[test]
    fn grade_is_a_noop_outside_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let deck = trace_deck(dir.path());
        let trace = Trace::from_deck(&deck).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut session = TraceSession::new(trace, crate::scheduler::Fsrs::default());
        session.grade(&mut store, Delta::Passed, 1000);
        assert_eq!(Phase::Predict, session.phase());
        assert_eq!(0, session.current_index());
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
            "title = \"W\"\nexam = \"../src\"\n",
        );
        write(
            &root.join("ws/decks"),
            "t.md",
            "---\ntrace: how it works\n\
             exam: ../src\ncite:\n  - src: ../src\n---\n\
             ## hop 1\nit reads a\n<!-- at: src:a.rs:2-3 -->\n\
             ## hop 2\nit reads b\n<!-- at: src:b.rs:1 -->\n",
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
        assert_eq!(DriftKind::Rewritten, d[0].kind);
        assert_eq!("src:a.rs:2-3", d[0].at.to_string());

        std::fs::remove_file(root.join("src/a.rs")).unwrap();
        let d = drifted_cards(&Deck::load(&deck_path).unwrap());
        assert!(
            d.iter()
                .any(|x| x.kind == DriftKind::SourceGone && x.at.to_string() == "src:a.rs:2-3"),
            "{d:?}"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn an_unchanged_frozen_whole_file_citation_does_not_drift() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        write(&root.join("src"), "a.rs", "alpha\nbeta\ngamma\n");
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(&root.join("ws"), "alix.toml", "title = \"W\"\n");
        let deck_path = write(
            &root.join("ws/decks"),
            "whole.md",
            "---\ncite:\n  - src: ../src\n---\n\
             ## whole file\nit cites all of a.rs\n<!-- at: src:a.rs -->\n",
        );

        crate::assets::initialize(&deck_path).unwrap();

        let drift = drifted_cards(&Deck::load(&deck_path).unwrap());
        assert!(
            drift.is_empty(),
            "an unchanged whole-file citation is current, not moved: {drift:?}"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn a_grown_frozen_whole_file_citation_is_rewritten_never_moved_to_a_range() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        write(&root.join("src"), "a.rs", "alpha\nbeta\n");
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(&root.join("ws"), "alix.toml", "title = \"W\"\n");
        let deck_path = write(
            &root.join("ws/decks"),
            "whole.md",
            "---\ncite:\n  - src: ../src\n---\n\
             ## whole file\nit cites all of a.rs\n<!-- at: src:a.rs -->\n",
        );
        crate::assets::initialize(&deck_path).unwrap();
        write(&root.join("src"), "a.rs", "new\nalpha\nbeta\n");

        let kinds: Vec<DriftKind> = drifted_cards(&Deck::load(&deck_path).unwrap())
            .into_iter()
            .map(|drift| drift.kind)
            .collect();
        assert_eq!(
            vec![DriftKind::Rewritten],
            kinds,
            "a whole-file citation's text changed; a range would narrow it"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn drifted_cards_reports_an_intact_excerpt_that_moved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deck_path = frozen_workspace(root);
        crate::assets::initialize(&deck_path).unwrap();
        assert!(drifted_cards(&Deck::load(&deck_path).unwrap()).is_empty());

        // The cited bytes are untouched; two lines were inserted above them.
        let moved = format!(
            "pushed\ndown\n{}",
            std::fs::read_to_string(root.join("src/a.rs")).unwrap()
        );
        std::fs::write(root.join("src/a.rs"), moved).unwrap();

        let d = drifted_cards(&Deck::load(&deck_path).unwrap());
        assert_eq!(
            vec![DriftKind::Moved { start: 4, len: 2 }],
            d.iter().map(|x| x.kind.clone()).collect::<Vec<_>>(),
            "an address that no longer holds its own evidence is drift: {d:?}"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn drift_is_judged_against_each_locators_own_root_and_a_url_passage_never_drifts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        write(&root.join("src"), "a.rs", "alpha\nbeta\ngamma\n");
        write(&root.join("other"), "x.txt", "one\ntwo\n");
        write(root, "secret.rs", "alpha\nbeta\ngamma\n");
        std::fs::create_dir_all(root.join("ws/decks")).unwrap();
        write(
            &root.join("ws"),
            "alix.toml",
            "[cite]\ncode = \"../src\"\nother = \"../other\"\nguide = \"https://example.org/guide\"\n",
        );
        let assets = root.join("ws/assets/deck-deck1");
        std::fs::create_dir_all(&assets).unwrap();
        let passage = crate::assets::object_name(b"the passage\n", "md");
        write(&assets, &passage, "the passage\n");
        let deck_path = write(
            &root.join("ws/decks"),
            "t.md",
            &format!(
                "---\nformat-version: 1\nid: \"deck-deck1\"\n---\n\
                 ## a\na\n<!-- at: code:a.rs:2-3 -->\n\
                 ## x\nx\n<!-- at: other:x.txt:1 -->\n\
                 ## g\ng\n<!-- at: guide asset: {passage} -->\n"
            ),
        );
        crate::assets::initialize(&deck_path).unwrap();
        let drifts = |label: &str| {
            let drifts = drifted_cards(&Deck::load(&deck_path).unwrap());
            let mut found: Vec<(String, DriftKind)> = drifts
                .into_iter()
                .map(|drift| (drift.at.to_string(), drift.kind))
                .collect();
            found.sort_by(|a, b| a.0.cmp(&b.0));
            println!("step {label}: {found:?}");
            found
        };

        assert!(drifts("frozen").is_empty());

        std::fs::write(root.join("other/x.txt"), "CHANGED\n").unwrap();
        assert_eq!(
            vec![("other:x.txt:1".to_string(), DriftKind::Rewritten)],
            drifts("other rewritten"),
            "only the locator under the changed root drifts"
        );

        std::fs::write(root.join("other/x.txt"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("src/a.rs"), "pushed\nalpha\nbeta\ngamma\n").unwrap();
        assert_eq!(
            vec![(
                "code:a.rs:2-3".to_string(),
                DriftKind::Moved { start: 3, len: 2 }
            )],
            drifts("code moved")
        );

        std::fs::remove_file(root.join("src/a.rs")).unwrap();
        assert_eq!(
            vec![("code:a.rs:2-3".to_string(), DriftKind::SourceGone)],
            drifts("code gone")
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("secret.rs"), root.join("src/a.rs")).unwrap();
            assert_eq!(
                vec![("code:a.rs:2-3".to_string(), DriftKind::SourceGone)],
                drifts("code escapes"),
                "a live file escaping its root is drift and is never read"
            );
        }
    }
}
