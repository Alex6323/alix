//! A deck is a parsed flashcard file.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{
    answer::Input,
    card::{Card, Direction},
    config::Strictness,
    depth::Reveal,
    l1::{self, L1Error},
    session::{self, Order},
    store::Store,
};

/// Per-deck defaults declared with `% key: value` header directives, e.g.
/// `% reveal: line` or `% order: sequential`. Each is `None` unless the deck
/// sets it; an explicit CLI flag always takes precedence. Unknown keys and
/// unparseable values are ignored, so the directives never break a deck.
#[derive(Debug, Default, Clone)]
pub struct DeckSettings {
    /// Default reveal-method for this deck (`% reveal: ...`).
    pub reveal: Option<Reveal>,
    /// Default input method for this deck (`% input: ...`).
    pub input: Option<Input>,
    /// Default card order for this deck (`% order: ...`).
    pub order: Option<Order>,
    /// Default review direction for this deck (`% direction: ...`).
    pub direction: Option<Direction>,
    /// Directory that card `% img:` / `% img-back:` filenames resolve against
    /// (`% img-dir: ...`). Absolute, or relative to the deck file's folder.
    pub img_dir: Option<PathBuf>,
    /// How strictly this deck's AI exam grades answers (`% strictness: ...`).
    /// `None` uses the `[exam]` config default.
    pub exam_strictness: Option<Strictness>,
    /// The live source root a frozen deck's `% at:` snapshots came from
    /// (`% origin: <crate>`). Cascades workspace `[defaults]` → deck → card; the
    /// tutor grounds in it for context and drift detection reads it. `None` for a
    /// non-frozen deck.
    pub origin: Option<String>,
}

impl DeckSettings {
    /// Interprets the recognized directives; ignores the rest.
    pub fn from_directives(directives: &[(String, String)]) -> Self {
        let mut settings = Self::default();
        for (key, value) in directives {
            match key.as_str() {
                "reveal" => settings.reveal = Reveal::parse(value),
                "input" => settings.input = Input::parse(value),
                "order" => settings.order = Order::parse(value),
                "direction" => settings.direction = Direction::parse(value),
                "img-dir" => settings.img_dir = Some(PathBuf::from(value)),
                "strictness" => settings.exam_strictness = Strictness::parse(value),
                "origin" => {
                    let v = value.trim();
                    if !v.is_empty() {
                        settings.origin = Some(v.to_string());
                    }
                }
                _ => {}
            }
        }
        settings
    }

    /// Interprets the typed L1 frontmatter into per-deck settings. The
    /// frontmatter equivalent of [`from_directives`](Self::from_directives),
    /// which stays for the workspace manifest's string `[defaults]` table.
    pub fn from_frontmatter(frontmatter: &l1::Frontmatter) -> Self {
        Self {
            reveal: frontmatter.reveal,
            input: frontmatter.input,
            order: frontmatter.order,
            direction: frontmatter.direction,
            img_dir: frontmatter.img_dir.clone(),
            exam_strictness: frontmatter.strictness,
            origin: frontmatter.origin.clone(),
        }
    }

    /// Fills each unset field from `defaults` (a workspace's shared settings),
    /// so the deck's own directives win and the workspace fills the gaps —
    /// precedence deck > workspace.
    fn fill_from(&mut self, defaults: &DeckSettings) {
        self.reveal = self.reveal.or(defaults.reveal);
        self.input = self.input.or(defaults.input);
        self.order = self.order.or(defaults.order);
        self.direction = self.direction.or(defaults.direction);
        self.img_dir = self.img_dir.clone().or_else(|| defaults.img_dir.clone());
        self.exam_strictness = self.exam_strictness.or(defaults.exam_strictness);
        self.origin = self.origin.clone().or_else(|| defaults.origin.clone());
    }
}

/// How far through a deck the user is, derived from whether its cards have
/// graduated (reached FSRS `Review`) and, for `% source:` decks, the exam result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeckState {
    /// No card has been reviewed yet.
    NotStarted,
    /// Some cards reviewed, but not all have graduated.
    Started,
    /// Every card has graduated, but the deck declares `% source:` and
    /// the AI exam hasn't been passed yet — drilled, ready to be examined.
    ExamDue,
    /// Done: every card has graduated, and (for a `% source:` deck) the exam
    /// passed. This is what unlocks dependents.
    Finished,
}

/// A deck of flashcards loaded from a file.
#[derive(Debug)]
pub struct Deck {
    /// The path the deck was loaded from.
    pub path: PathBuf,
    /// The subject (= file name).
    pub subject: String,
    /// The deck's identity token (the frontmatter `id:`), minted at the first
    /// stamp. `None` until the deck has been stamped. Identifies which of a
    /// shared augment cache's topologies belong to this deck.
    pub deck_token: Option<String>,
    /// The cards, in file order.
    pub cards: Vec<Card>,
    /// Deck-level reference links (`% link: <url>` lines).
    pub links: Vec<String>,
    /// Prerequisite decks (`% requires: <deck>` lines), as written.
    pub requires: Vec<String>,
    /// Exam sources (`% source: <url-or-path>` lines) — the ground truth the AI
    /// exam grades against. A deck with sources is "mastered" (and unlocks
    /// dependents) only after passing the exam, not merely drilling its cards.
    pub sources: Vec<String>,
    /// Per-deck defaults from `% key: value` directives.
    pub settings: DeckSettings,
    /// Display title (the `# H1`), independent of the file name. `None` falls
    /// back to the file name (minus `.md`). Display-only; not part of identity.
    pub title: Option<String>,
    /// What this deck traces (`% trace:`) — a path description, if any. Its
    /// presence makes the deck a **trace** — a predict-and-verify walk (see
    /// [`crate::trace`]) rather than a card deck — which is what the web walk
    /// walks and what makes the generic AI exam refuse it. `None` for an
    /// ordinary deck.
    pub trace: Option<String>,
}

/// An error loading a deck file.
#[derive(Debug, Error)]
pub enum DeckError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: L1Error,
    },
    #[error("{path}: file name is not valid UTF-8")]
    InvalidFileName { path: PathBuf },
}

impl Deck {
    /// Loads and parses a deck file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DeckError> {
        Self::load_with_defaults(path, &DeckSettings::default())
    }

    /// Like [`Deck::load`], but fills any directive the deck leaves unset from
    /// `defaults` (a workspace's shared settings). The merge happens before the
    /// per-card folds and direction expansion, so workspace defaults flow into
    /// the cards exactly as the deck's own directives would. Precedence:
    /// card > deck > `defaults`.
    pub fn load_with_defaults(
        path: impl AsRef<Path>,
        defaults: &DeckSettings,
    ) -> Result<Self, DeckError> {
        let path = path.as_ref().to_path_buf();
        let subject = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| DeckError::InvalidFileName { path: path.clone() })?
            .to_string();
        let text = std::fs::read_to_string(&path).map_err(|source| DeckError::Io {
            path: path.clone(),
            source,
        })?;
        let l1deck = l1::parse_l1(&subject, &text).map_err(|source| DeckError::Parse {
            path: path.clone(),
            source,
        })?;
        // Deck-level metadata now comes from the typed L1 frontmatter (and the
        // `# H1` title) rather than the old header `%` directives.
        let links = l1deck.frontmatter.link.clone();
        let requires = l1deck.frontmatter.requires.clone();
        let sources = l1deck.frontmatter.source.clone();
        let title = l1deck.title.clone();
        let trace = l1deck.frontmatter.trace.clone();
        let deck_token = l1deck.deck_token.clone();
        let mut settings = DeckSettings::from_frontmatter(&l1deck.frontmatter);
        let mut cards = l1deck.cards;
        // Fold the workspace's shared directives in below the deck's own.
        settings.fill_from(defaults);
        // A card without its own `% reveal:` inherits the deck's reveal-method,
        // so each card carries its effective declared reveal (card override,
        // else deck).
        for card in &mut cards {
            card.reveal = card.reveal.or(settings.reveal);
            card.input = card.input.or(settings.input);
        }
        // Resolve each card's image filenames to absolute paths against the
        // deck's `img-dir` (or the deck file's own folder when none is set). No
        // filesystem check: a missing image must not stop the deck from loading.
        let base_dir = image_base_dir(&path, settings.img_dir.as_deref());
        for card in &mut cards {
            card.image = card.image.take().map(|p| resolve_image(&base_dir, p));
            card.image_back = card.image_back.take().map(|p| resolve_image(&base_dir, p));
        }
        // Expand the declared direction (card override, else deck) into cards.
        // `reverse` swaps the card, `both` adds the swapped one alongside; the
        // reversed card keeps the source line, so the session treats the pair as
        // siblings. Direction doesn't apply to cloze cards.
        let mut expanded = Vec::with_capacity(cards.len());
        for card in cards {
            let direction = card.direction.or(settings.direction).unwrap_or_default();
            // A cloze sub-card (`hole.is_some()`) never reverses, whatever the
            // deck-level direction: it carries `direction: None`, so keying on
            // the hole (not the direction) is what keeps a deck-wide
            // `direction: both` from minting a bogus reversed cloze twin.
            if card.hole.is_some() || direction == Direction::Forward {
                expanded.push(card);
            } else {
                let reversed = card.reversed();
                match direction {
                    Direction::Reverse => expanded.push(reversed),
                    Direction::Both => {
                        expanded.push(card);
                        expanded.push(reversed);
                    }
                    Direction::Forward => unreachable!("handled above"),
                }
            }
        }
        let cards = expanded;
        Ok(Self {
            path,
            subject,
            deck_token,
            cards,
            links,
            requires,
            sources,
            settings,
            title,
            trace,
        })
    }

    /// Whether this deck is a **trace** (it declares a `% trace:`): a guided
    /// predict-and-verify walk rather than a card deck. A trace's exam is its
    /// **compression** — retracing the path in a couple of sentences, graded
    /// against the checkpoints — not the generic source-wide exam a fact deck
    /// sits.
    pub fn is_trace(&self) -> bool {
        self.trace.is_some()
    }

    /// Whether the deck has an AI exam that gates mastery: a **trace** (its exam
    /// is the graded compression) or any deck with a `% source:` (a fact deck's
    /// exam, generated from the source). A source-less fact deck has none — it is
    /// `Finished` the moment it is drilled. The single definition shared by
    /// [`state`](Deck::state), [`is_locked`] and the picker.
    pub fn has_exam(&self) -> bool {
        self.is_trace() || !self.sources.is_empty()
    }

    /// The deck's display name: its `# H1` title if set, else — for a trace deck —
    /// its `trace:` path description (a trace's natural name), else the file
    /// name with the `.md` extension stripped.
    pub fn display_name(&self) -> String {
        self.title
            .clone()
            .or_else(|| self.trace.clone())
            .unwrap_or_else(|| {
                self.subject
                    .strip_suffix(".md")
                    .unwrap_or(&self.subject)
                    .to_string()
            })
    }

    /// The deck's completion state, derived from its cards' FSRS maturity (see
    /// [`session::has_graduated`]) and, for `% source:` decks, its exam result:
    /// `NotStarted` while no card has been reviewed, `Started` in between, and once
    /// every card has *graduated* (reached FSRS `Review`) either `ExamDue` (a sourced
    /// deck whose exam hasn't been passed) or `Finished`. A source-less deck has no
    /// exam, so it is `Finished` as soon as every card graduates. An empty deck is
    /// `NotStarted`.
    pub fn state(&self, store: &Store) -> DeckState {
        let total = self.cards.len();
        if total == 0 {
            return DeckState::NotStarted;
        }
        // A passed AI exam masters the deck outright — you can test out of the
        // drilling — so it counts as `Finished` (and unlocks its dependents)
        // however many cards are still un-graduated.
        if store.deck_mastered(&self.subject) {
            return DeckState::Finished;
        }
        // The graduation gate: every card has reached FSRS `Review` (past the initial
        // learning steps), so the exam opens well before a card's year-long retirement.
        let gated = self.cards.iter().all(|c| session::has_graduated(c, store));
        if gated {
            // Drilled enough but not yet mastered: a deck with an exam (a sourced
            // fact deck, or a trace — whose exam is its graded compression) is
            // `ExamDue`; a source-less fact deck has no exam, so it's `Finished`.
            if self.has_exam() {
                DeckState::ExamDue
            } else {
                DeckState::Finished
            }
        } else if self
            .cards
            .iter()
            .all(|c| c.id().and_then(|id| store.get(&id)).is_none())
        {
            DeckState::NotStarted
        } else {
            DeckState::Started
        }
    }

    /// Reference URLs offered to the ask-Claude tutor for this deck: the
    /// `% link:` URLs plus any `% source:` that is itself a URL — a source is
    /// also a reference the tutor may consult, so you needn't repeat it as a
    /// `% link:`. Links come first, then source URLs not already listed; local
    /// file sources are omitted (ask-Claude fetches references over the web).
    /// The reverse does not hold: a `% link:` never becomes an exam source.
    pub fn reference_links(&self) -> Vec<String> {
        let mut out = self.links.clone();
        for src in &self.sources {
            if is_url(src) && !out.contains(src) {
                out.push(src.clone());
            }
        }
        out
    }

    /// The live source root the grounded ask-tutor reads. For a **frozen** deck
    /// this is its `% origin:` (the real crate the `% at:` snapshots came from —
    /// `% source:` itself points at the opaque `assets/`); otherwise the project
    /// root behind the deck's local `% source:` (see [`crate::trace::project_root`]).
    /// `None` for a URL-only / source-less deck.
    pub fn source_root(&self) -> Option<PathBuf> {
        if let Some(origin) = &self.settings.origin {
            return Some(PathBuf::from(origin));
        }
        let deck_dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        // A frozen deck loaded WITHOUT its workspace defaults folded (bare
        // `Deck::load`) has no `settings.origin` yet — recover it straight from the
        // workspace manifest, so grounding works regardless of the load path.
        if self.is_frozen()
            && let Ok(ws) = crate::workspace::Workspace::load(deck_dir)
            && let Some(origin) = ws.settings.origin
        {
            return Some(PathBuf::from(origin));
        }
        crate::trace::project_root(&self.sources, deck_dir)
    }

    /// Whether this deck reads from frozen `assets/` snapshots (`% source: assets`)
    /// rather than a live source — i.e. it has been through `snapshot`.
    pub fn is_frozen(&self) -> bool {
        self.sources
            .first()
            .is_some_and(|s| s == crate::trace::SNAPSHOT_DIR)
    }
}

// `Deck::duplicates()` and its duplicate-answer doctor job are RETIRED with the
// move to token identity: unstamped cards all share the id `0`, so a content
// hash can no longer tell two cards apart. The surviving signal is duplicate
// *token* handling (spec §2.4), which lands in a later task.

/// Finds the file a `requires:` value refers to: as given, next to the
/// requiring deck, or in the decks directory; with or without a `.md` suffix.
/// Any explicit extension the value carries is stripped first, so a legacy
/// `requires: basics.txt` still resolves the `basics.md` deck.
pub fn resolve_dep(
    req: &str,
    decks_dir: Option<&Path>,
    requiring_dir: Option<&Path>,
) -> Option<PathBuf> {
    // Strip any extension the value carries, then match the deck extension.
    let stem = Path::new(req)
        .with_extension("")
        .to_string_lossy()
        .into_owned();
    let with_md = |p: &Path| -> PathBuf { p.with_extension("md") };
    let mut candidates = vec![PathBuf::from(req), with_md(Path::new(&stem))];
    for dir in [requiring_dir, decks_dir].into_iter().flatten() {
        candidates.push(dir.join(req));
        candidates.push(with_md(&dir.join(&stem)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Whether `deck`'s **exam** is locked: any of its transitive **sourced**
/// `% requires:` prerequisites has not yet passed its exam (is not
/// [`Finished`](DeckState::Finished)). A source-less prerequisite has no exam to
/// pass, so it never gates — it is seen *through* to any sourced ancestor behind
/// it (its `% requires:` edge is purely informational). This gates only sitting
/// the exam, never drilling: a deck may be reviewed at any time regardless of
/// its prerequisites. `decks_dir` resolves prerequisite names. A missing
/// prerequisite, an unreadable file, or a dependency cycle is treated as
/// non-blocking (a broken graph never hides a deck).
pub fn is_locked(deck: &Deck, decks_dir: Option<&Path>, store: &Store) -> bool {
    fn prereqs_finished(
        deck: &Deck,
        decks_dir: Option<&Path>,
        store: &Store,
        visited: &mut HashSet<PathBuf>,
    ) -> bool {
        for req in &deck.requires {
            let Some(path) = resolve_dep(req, decks_dir, deck.path.parent()) else {
                continue; // missing prerequisite: don't lock on it
            };
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !visited.insert(key) {
                continue; // already checked, or a cycle: stop recursing
            }
            let Ok(prereq) = Deck::load(&path) else {
                continue; // unreadable prerequisite: don't lock on it
            };
            // A prerequisite with an exam gates: it must be passed (mastered ⇒
            // `Finished`) — a sourced fact deck or a trace (its exam is the graded
            // compression). A source-less fact deck has no exam, so it never gates
            // — but a sourced ancestor behind it still does, so recurse through it
            // either way.
            if prereq.has_exam() && prereq.state(store) != DeckState::Finished {
                return false;
            }
            if !prereqs_finished(&prereq, decks_dir, store, visited) {
                return false;
            }
        }
        true
    }
    !prereqs_finished(deck, decks_dir, store, &mut HashSet::new())
}

/// A deck's non-gating `% requires:` edges: a prerequisite that resolves to a
/// readable but source-less deck (no exam of its own) is seen through by
/// `is_locked` and never gates, so listing it is almost certainly a mistake.
/// Returns each such `% requires:` string. Empty when this deck has no exam of its
/// own or every prerequisite gates; missing/unreadable prerequisites are skipped
/// (they don't gate either, but aren't a fixable "add a `% source:`" case).
pub fn nongating_prerequisites(deck: &Deck) -> Vec<String> {
    if !deck.has_exam() {
        return Vec::new();
    }
    let dir = deck.path.parent();
    let mut out = Vec::new();
    for req in &deck.requires {
        let sourceless = resolve_dep(req, dir, dir)
            .and_then(|path| Deck::load(&path).ok())
            .is_some_and(|prereq| !prereq.has_exam());
        if sourceless {
            out.push(req.clone());
        }
    }
    out
}

/// Whether `s` looks like an http(s) URL (vs a local file path). Used to tell a
/// fetchable `% source:`/`% link:` from a file path.
pub(crate) fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Subjects of decks in `decks_dir` that directly `% requires:` the deck at
/// `target` (its dependents). Used to report what an exam pass unlocks. Decks
/// that fail to load are skipped; the result is sorted for stable output.
pub fn dependents(target: &Path, decks_dir: &Path) -> Vec<String> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let target = canon(target);
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(decks_dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(deck) = Deck::load(&path) else {
            continue;
        };
        let requires_target = deck.requires.iter().any(|req| {
            resolve_dep(req, Some(decks_dir), path.parent())
                .is_some_and(|dep| canon(&dep) == target)
        });
        if requires_target {
            names.push(deck.subject);
        }
    }
    names.sort();
    names
}

/// The directory that card image filenames resolve against: the deck's
/// `% img-dir:` if set (made absolute against the deck file's folder when it is
/// itself relative), else the deck file's own folder.
fn image_base_dir(deck_path: &Path, img_dir: Option<&Path>) -> PathBuf {
    let deck_dir = deck_path.parent().unwrap_or_else(|| Path::new("."));
    match img_dir {
        Some(dir) if dir.is_absolute() => dir.to_path_buf(),
        Some(dir) => deck_dir.join(dir),
        None => deck_dir.to_path_buf(),
    }
}

/// Resolves one card image: an absolute value is used as-is; otherwise it is
/// joined onto the deck's image base directory.
fn resolve_image(base: &Path, image: PathBuf) -> PathBuf {
    if image.is_absolute() {
        image
    } else {
        base.join(image)
    }
}

/// Atomically replaces the deck file at `path` with `text` (temp sibling +
/// rename), the shared write tail of the file-surgery helpers below.
fn write_deck_text(path: &Path, text: &str) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, text).map_err(io_err)?;
    std::fs::rename(&tmp, path).map_err(io_err)?;
    Ok(())
}

/// The deck's parsed `## ` card-front line numbers, for the surgery helpers:
/// parse knowledge, so a fenced `## ` (content) can never be mistaken for a
/// card boundary the way a naive line scan would.
fn front_lines_of(path: &Path, text: &str) -> Result<Vec<usize>, DeckError> {
    l1::card_front_lines(text).map_err(|source| DeckError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Appends `notes` as `> ` note lines to the card whose front is at the
/// 1-based `front_line` of the deck file at `path`. The file is rewritten
/// atomically (temp file + rename); on reload the parser merges the new lines
/// into the card's (possibly multi-line) note. Card identities don't change —
/// notes are never identity.
pub fn append_note(path: &Path, front_line: usize, notes: &[String]) -> Result<(), DeckError> {
    if notes.is_empty() {
        return Ok(());
    }
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let text = std::fs::read_to_string(path).map_err(io_err)?;
    let fronts = front_lines_of(path, &text)?;
    let new_text = insert_note_lines(&text, &fronts, front_line, notes);
    write_deck_text(path, &new_text)
}

/// Appends L1 `cards` text to the end of the deck file at `path`, ensuring a
/// blank line separates them from the existing content. Written atomically
/// (temp + rename). Used to add AI exam remediation cards; the deck format is
/// append-safe (a new `## ` front at column 0 starts a new card), so existing
/// cards and their identities are untouched. Callers pass already-stamped
/// text (each card carries its `<!-- id: -->` token).
pub fn append_cards(path: &Path, cards: &str) -> Result<(), DeckError> {
    let cards = cards.trim_end();
    if cards.is_empty() {
        return Ok(());
    }
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };

    let existing = std::fs::read_to_string(path).map_err(io_err)?;
    let mut new_text = existing.trim_end().to_string();
    if !new_text.is_empty() {
        new_text.push_str("\n\n");
    }
    new_text.push_str(cards);
    new_text.push('\n');
    write_deck_text(path, &new_text)
}

/// Replaces a trace deck's checkpoint cards with `cards`, keeping the header —
/// everything before the first card front (the frontmatter with its `trace:`
/// and `source:` keys, the `# H1`, any preamble prose). Used by the trace
/// build (`alix generate <stub>`) to write the discovered path back into the
/// deck (overwriting a previous build), via an atomic temp-file rename. A deck
/// with no card front yet is all header, so the cards are simply appended
/// after it.
pub fn set_trace_checkpoints(path: &Path, cards: &str) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let existing = std::fs::read_to_string(path).map_err(io_err)?;
    let fronts = front_lines_of(path, &existing)?;
    let new_text = replace_after_header(&existing, &fronts, cards);
    write_deck_text(path, &new_text)
}

/// Returns the header of `text` (every line before the first `## ` card
/// front, trailing blanks trimmed) followed by `cards`, separated by a blank
/// line. `fronts` is the parsed card-front list, so a fenced `## ` (content)
/// never truncates the header.
fn replace_after_header(text: &str, fronts: &[usize], cards: &str) -> String {
    let cards = cards.trim_end();
    let header: Vec<&str> = match fronts.first() {
        Some(&first) => text.lines().take(first.saturating_sub(1)).collect(),
        None => text.lines().collect(),
    };
    let header = header.join("\n");
    let header = header.trim_end();
    let mut out = String::new();
    if !header.is_empty() {
        out.push_str(header);
        if !cards.is_empty() {
            out.push_str("\n\n");
        }
    }
    out.push_str(cards);
    out.push('\n');
    out
}

/// A snapshot's rewrite for one `<!-- at: -->` directive: the frozen asset
/// locator and the origin-relative provenance (`src/caching.rs:46-66`)
/// appended as ` from …` — so the original location survives on the locator
/// itself, not smuggled into a user note.
pub struct AtRewrite {
    pub at: String,
    pub origin: Option<String>,
}

/// Repoints a snapshotted trace in place (atomic temp + rename): replaces the
/// frontmatter's `source:` value with `source` (and adds an `origin:` key when
/// `origin` is set — the live crate root this deck froze from), and each
/// standalone `<!-- at: ... -->` directive line (in file order) with the
/// matching `ats` entry — its frozen asset plus a ` from
/// <origin-relative>:<lines>` suffix. The `trace:` key, key points, notes, and
/// everything else are preserved verbatim; identity tokens are untouched, so
/// card identities are unaffected. Used when snapshotting into `assets/`.
pub fn set_trace_snapshot(
    path: &Path,
    source: &str,
    origin: Option<&str>,
    ats: &[AtRewrite],
) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let existing = std::fs::read_to_string(path).map_err(io_err)?;
    let span = l1::parse_l1("deck.md", &existing)
        .map_err(|source| DeckError::Parse {
            path: path.to_path_buf(),
            source,
        })?
        .frontmatter_span;
    let new_text = rewrite_trace_snapshot(&existing, span, source, origin, ats);
    write_deck_text(path, &new_text)
}

/// Pure transform for [`set_trace_snapshot`]: within the frontmatter `span`,
/// replace the first `source:` key line's value with `source` (adding an
/// `origin:` line after it when set); in the body, replace each standalone
/// `<!-- at: ... -->` line's value (in order) with `ats[i].at`, appending
/// ` from <ats[i].origin>` when present. Indentation is preserved; everything
/// else is untouched. (A literal `<!-- at: -->` line inside a code fence would
/// be misread as a directive here — tool-generated trace decks never carry
/// one, and the parser-side card surgery is unaffected.)
fn rewrite_trace_snapshot(
    text: &str,
    frontmatter_span: Option<crate::l1::LineSpan>,
    source: &str,
    origin: Option<&str>,
    ats: &[AtRewrite],
) -> String {
    // A standalone `<!-- at: ... -->` directive line: its leading indentation,
    // or `None` for anything else.
    fn at_indent(line: &str) -> Option<&str> {
        let trimmed = line.trim_start();
        let body = trimmed.strip_prefix("<!--")?.strip_suffix("-->")?;
        let (key, _value) = body.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case("at")
            .then(|| &line[..line.len() - trimmed.len()])
    }
    // A frontmatter block-mapping `source:` key line.
    fn is_source_key(line: &str) -> bool {
        line.strip_prefix("source")
            .is_some_and(|rest| rest.trim_start().starts_with(':'))
    }

    let mut source_replaced = false;
    let mut at_i = 0;
    let mut out: Vec<String> = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let lineno = idx + 1;
        let in_frontmatter =
            frontmatter_span.is_some_and(|(open, close)| lineno > open && lineno < close);
        if in_frontmatter && !source_replaced && is_source_key(line) {
            out.push(format!("source: {source}"));
            if let Some(origin) = origin {
                out.push(format!("origin: {origin}"));
            }
            source_replaced = true;
        } else if !in_frontmatter
            && at_i < ats.len()
            && let Some(indent) = at_indent(line)
        {
            match &ats[at_i].origin {
                Some(o) => out.push(format!("{indent}<!-- at: {} from {o} -->", ats[at_i].at)),
                None => out.push(format!("{indent}<!-- at: {} -->", ats[at_i].at)),
            }
            at_i += 1;
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

// `set_requires`/`rewrite_requires` are DELETED with the flip: `requires` now
// lives in YAML frontmatter, and the helper had no production caller. A
// frontmatter-aware rewrite can be built when a real caller appears (pre-1.0,
// no compat shim).

/// Removes whole card blocks from a deck file: every card whose front sits at
/// one of the 1-based `front_lines` is deleted along with its back lines,
/// notes and trailing blank separator. The block runs from the `## ` front to
/// the next card's front, or the end of the file, by parse knowledge, so a
/// fenced `## ` never splits a block. Passing the front line of any cloze
/// sub-card removes the whole source block, since all of its holes share that
/// line. The file is rewritten atomically (temp + rename). An empty
/// `front_lines` is a no-op.
pub fn remove_cards(path: &Path, front_lines: &[usize]) -> Result<(), DeckError> {
    if front_lines.is_empty() {
        return Ok(());
    }
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let text = std::fs::read_to_string(path).map_err(io_err)?;
    let fronts = front_lines_of(path, &text)?;
    let new_text = remove_card_blocks(&text, &fronts, front_lines);
    write_deck_text(path, &new_text)
}

/// Rewrites `path` to `original` with the card blocks at `front_lines` removed.
/// Unlike [`remove_cards`], the caller supplies the file's *original* content,
/// so the line numbers stay valid however many cards were removed before. The
/// web server uses this: it removes cards immediately but keeps each deck's
/// original text in memory and re-derives the file from the growing set of
/// removed lines, sidestepping the line shifts that repeated in-place edits
/// would cause. Written atomically (temp + rename).
pub fn rewrite_without_cards(
    path: &Path,
    original: &str,
    front_lines: &[usize],
) -> Result<(), DeckError> {
    let fronts = front_lines_of(path, original)?;
    let new_text = remove_card_blocks(original, &fronts, front_lines);
    write_deck_text(path, &new_text)
}

/// Returns `text` with the card blocks starting at the given 1-based front
/// lines removed. `fronts` is the parsed card-front list: a block runs from
/// its front to the line before the next parsed front (or EOF), so the front,
/// back lines, notes and the blank line after it all go, and a fenced `## `
/// (content, absent from `fronts`) can never end a block early. A
/// `front_line` that is not a parsed card front is ignored, so a stale line
/// number can never corrupt the file.
fn remove_card_blocks(text: &str, fronts: &[usize], front_lines: &[usize]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let targets: std::collections::HashSet<usize> = front_lines.iter().copied().collect();

    let mut drop = vec![false; lines.len()];
    for (i, &front) in fronts.iter().enumerate() {
        if !targets.contains(&front) {
            continue;
        }
        // 1-based inclusive block end: the line before the next front, or EOF.
        let end = fronts
            .get(i + 1)
            .map(|next| next.saturating_sub(1))
            .unwrap_or(lines.len());
        for lineno in front..=end.min(lines.len()) {
            if lineno >= 1 {
                drop[lineno - 1] = true;
            }
        }
    }

    let kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop[*i])
        .map(|(_, line)| *line)
        .collect();
    let mut result = kept.join("\n");
    if text.ends_with('\n') && !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Inserts `notes` as `> ` note lines after the last content line of the card
/// whose front sits at the 1-based `front_line`. `fronts` is the parsed
/// card-front list; the card's block ends at the next parsed front (or EOF),
/// so a fenced `## ` inside the answer never truncates the walk.
fn insert_note_lines(text: &str, fronts: &[usize], front_line: usize, notes: &[String]) -> String {
    let lines: Vec<&str> = text.lines().collect();

    // The 0-based exclusive bound of the card's block: the next parsed front
    // after `front_line`, or EOF.
    let bound = fronts
        .iter()
        .find(|&&f| f > front_line)
        .map(|&f| f.saturating_sub(1))
        .unwrap_or(lines.len())
        .min(lines.len());
    let front_index = front_line.saturating_sub(1);
    let mut last_content = front_index;
    for (i, line) in lines.iter().enumerate().take(bound).skip(front_index + 1) {
        if !line.trim().is_empty() {
            last_content = i;
        }
    }

    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for (offset, note) in notes.iter().enumerate() {
        out.insert(last_content + 1 + offset, format!("> {note}"));
    }

    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn write_deck(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn empty_store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        (store, dir)
    }

    /// Marks a card graduated: FSRS `Review`.
    fn graduate(store: &mut Store, id: &str) {
        store.get_or_insert(id, 0).recall = Some(crate::store::FsrsState {
            state: 2, // Review
            ..Default::default()
        });
    }

    /// Marks a card seen but still in a learning step (not yet graduated).
    fn learning(store: &mut Store, id: &str) {
        store.get_or_insert(id, 0).recall = Some(crate::store::FsrsState {
            state: 1, // Learning
            ..Default::default()
        });
    }

    /// Drives a card to retirement: a year-out FSRS interval (also graduated).
    fn retire(store: &mut Store, id: &str) {
        store.get_or_insert(id, 0).recall = Some(crate::store::FsrsState {
            state: 2,                // Review — a year-out card has graduated
            scheduled_days: 100_000, // well past the retirement cap
            ..Default::default()
        });
    }

    /// The parsed card-front lines of `text`, for the pure surgery helpers.
    fn fronts(text: &str) -> Vec<usize> {
        l1::card_front_lines(text).unwrap()
    }

    #[test]
    fn deck_state_progresses_notstarted_started_finished() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "## a <!-- id: q1 -->\n1\n## b <!-- id: q2 -->\n2\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        assert_eq!(DeckState::NotStarted, deck.state(&store));

        // One card seen but still learning (not graduated) -> started.
        learning(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::Started, deck.state(&store));

        // Every card graduated -> finished (source-less, so no exam).
        for card in &deck.cards {
            graduate(&mut store, &card.id().unwrap());
        }
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn sourced_deck_is_examdue_until_mastered() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nsource: https://x\n---\n## a <!-- id: q1 -->\n1\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        // Drilled to retirement, but a sourced deck waits on its exam.
        retire(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::ExamDue, deck.state(&store));

        // Passing the exam (mastered) flips it to Finished.
        store.set_deck_mastered(&deck.subject, 1);
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn a_sourced_deck_is_examdue_once_every_card_graduates() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nsource: https://x\n---\n## a <!-- id: q1 -->\n1\n## b <!-- id: q2 -->\n2\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        // One card graduated, one still learning — the gate isn't met yet.
        graduate(&mut store, &deck.cards[0].id().unwrap());
        learning(&mut store, &deck.cards[1].id().unwrap());
        assert_eq!(DeckState::Started, deck.state(&store));

        // Both graduated (reached Review), well before retirement — the exam opens.
        graduate(&mut store, &deck.cards[1].id().unwrap());
        assert_eq!(DeckState::ExamDue, deck.state(&store));
    }

    #[test]
    fn a_sourceless_deck_finishes_once_every_card_graduates() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## a <!-- id: q1 -->\n1\n");
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        // No `source:`, so graduating every card finishes it (unlocks deps).
        graduate(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn a_deck_still_learning_a_card_is_only_started() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## a <!-- id: q1 -->\n1\n");
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();
        // Seen but still in a learning step (not graduated) — only `Started`.
        learning(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::Started, deck.state(&store));
    }

    #[test]
    fn nongating_prerequisites_flags_a_sourceless_required_deck() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "## a\n1\n"); // source-less: no exam
        write_deck(dir.path(), "c.md", "---\nsource: https://x\n---\n## c\n1\n"); // sourced
        let b_path = write_deck(
            dir.path(),
            "b.md",
            "---\nsource: https://x\nrequires:\n  - a\n  - c\n---\n## b\n1\n",
        );
        let b = Deck::load(&b_path).unwrap();
        // Only the source-less `a` is flagged; the sourced `c` gates fine.
        assert_eq!(vec!["a".to_string()], nongating_prerequisites(&b));
    }

    #[test]
    fn nongating_prerequisites_empty_when_no_exam_or_prereq_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "## a\n1\n");
        // A source-less deck has no exam of its own — nothing to gate.
        let b = write_deck(dir.path(), "b.md", "---\nrequires: a\n---\n## b\n1\n");
        assert!(nongating_prerequisites(&Deck::load(&b).unwrap()).is_empty());
        // A sourced deck requiring a MISSING prereq: skipped (not a fixable edge).
        let c = write_deck(
            dir.path(),
            "c.md",
            "---\nsource: https://x\nrequires: nope\n---\n## c\n1\n",
        );
        assert!(nongating_prerequisites(&Deck::load(&c).unwrap()).is_empty());
    }

    #[test]
    fn passing_the_exam_masters_an_undrilled_deck() {
        // Test out: a sourced deck whose cards aren't drilled is `Started`, but
        // passing its exam masters it outright → `Finished` (so it unlocks
        // dependents without first drilling every card).
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nsource: https://x\n---\n## a <!-- id: q1 -->\n1\n## b <!-- id: q2 -->\n2\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();
        assert_eq!(DeckState::NotStarted, deck.state(&store));

        store.set_deck_mastered(&deck.subject, 1);
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn sourceless_deck_finishes_on_drill_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## a <!-- id: q1 -->\n1\n");
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();
        retire(&mut store, &deck.cards[0].id().unwrap());
        // No `source:` -> no exam -> Finished as soon as it's fully drilled.
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn dependent_stays_locked_until_sourced_prereq_mastered() {
        let dir = tempfile::tempdir().unwrap();
        let basics = write_deck(
            dir.path(),
            "basics.md",
            "---\nsource: https://x\n---\n## a <!-- id: q1 -->\n1\n",
        );
        let adv = write_deck(
            dir.path(),
            "advanced.md",
            "---\nrequires: basics\n---\n## x\ny\n",
        );
        let advanced = Deck::load(&adv).unwrap();
        let basics = Deck::load(&basics).unwrap();
        let (mut store, _s) = empty_store();
        let dd = Some(dir.path());

        // Drilling basics is not enough: it's only ExamDue, not Finished.
        retire(&mut store, &basics.cards[0].id().unwrap());
        assert_eq!(DeckState::ExamDue, basics.state(&store));
        assert!(is_locked(&advanced, dd, &store));

        // Passing basics' exam masters it -> dependent unlocks.
        store.set_deck_mastered(&basics.subject, 1);
        assert!(!is_locked(&advanced, dd, &store));
    }

    #[test]
    fn dependents_lists_requiring_decks() {
        let dir = tempfile::tempdir().unwrap();
        let basics = write_deck(dir.path(), "basics.md", "## a\n1\n");
        write_deck(
            dir.path(),
            "advanced.md",
            "---\nrequires: basics\n---\n## x\ny\n",
        );
        write_deck(
            dir.path(),
            "expert.md",
            "---\nrequires: advanced\n---\n## z\nw\n",
        );
        write_deck(dir.path(), "unrelated.md", "## q\nr\n");

        let deps = dependents(&basics, dir.path());
        assert_eq!(vec!["advanced.md"], deps);
    }

    #[test]
    fn append_cards_appends_with_separation_and_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## one <!-- id: q1 -->\n1\n");
        // Callers pass already-stamped text.
        append_cards(
            &path,
            "## two <!-- id: q2 --> <!-- reveal: line -->\nkey point\n",
        )
        .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            "## one <!-- id: q1 -->\n1\n\n## two <!-- id: q2 --> <!-- reveal: line -->\nkey point\n",
            text
        );
        // The original card's identity survives; the new card is added.
        let cards = l1::parse_str("d.md", &text).unwrap();
        assert_eq!(2, cards.len());
        assert_eq!(Some("q1"), cards[0].token.as_deref());
    }

    #[test]
    fn set_trace_checkpoints_replaces_cards_keeping_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "t.md",
            "---\ntrace: how it works\nsource: .\n---\n\n## old question\nold point\n<!-- at: 1 -->\n",
        );
        set_trace_checkpoints(
            &path,
            "## new q1\np1\n<!-- at: 2 -->\n## new q2\np2\n<!-- at: 3 -->\n",
        )
        .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        // The `trace:`/`source:` frontmatter is kept; the old checkpoint is gone.
        assert!(text.starts_with("---\ntrace: how it works\nsource: .\n---\n"));
        assert!(!text.contains("old question"));
        assert!(text.contains("## new q1"));
        // The header survives a reload; the new checkpoints parse.
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some("how it works".to_string()), deck.trace);
        assert_eq!(2, deck.cards.len());
    }

    #[test]
    fn replace_after_header_appends_when_no_cards_yet() {
        // A fresh trace (header only) gets the cards appended below the header.
        let text = "---\ntrace: how it works\nsource: .\n---\n";
        let out = replace_after_header(text, &fronts(text), "## q\np\n");
        assert_eq!("---\ntrace: how it works\nsource: .\n---\n\n## q\np\n", out);
    }

    #[test]
    fn replace_after_header_is_not_fooled_by_a_fenced_heading() {
        // The header's prose holds a fenced `## `: it is content, not the first
        // card front, so the parsed `fronts` list points past it and the whole
        // fence is kept as header. A naive `starts_with("## ")` scan would
        // truncate the header at the fenced line and drop everything below it.
        let text = "# Preamble\n```\n## not a card\n```\ntail\n\n## real\nold\n";
        let out = replace_after_header(text, &fronts(text), "## new\nfresh\n");
        assert_eq!(
            "# Preamble\n```\n## not a card\n```\ntail\n\n## new\nfresh\n",
            out
        );
    }

    #[test]
    fn empty_deck_is_not_started() {
        let dir = tempfile::tempdir().unwrap();
        // Prose only, zero `## ` fronts: a valid zero-card deck.
        let path = write_deck(dir.path(), "e.md", "only a comment\n");
        let deck = Deck::load(&path).unwrap();
        let (store, _s) = empty_store();
        assert!(deck.cards.is_empty());
        assert_eq!(DeckState::NotStarted, deck.state(&store));
    }

    #[test]
    fn source_less_prerequisite_never_locks() {
        let dir = tempfile::tempdir().unwrap();
        // basics has no `source:`, so no exam — it can never gate.
        write_deck(dir.path(), "basics.md", "## a\n1\n");
        let adv = write_deck(
            dir.path(),
            "advanced.md",
            "---\nrequires: basics\n---\n## x\ny\n",
        );
        let advanced = Deck::load(&adv).unwrap();
        let (store, _s) = empty_store();
        let dd = Some(dir.path());

        // Undrilled, drilled, whatever — a source-less prerequisite is purely
        // informational, so the dependent's exam is never locked by it.
        assert!(!is_locked(&advanced, dd, &store));
    }

    #[test]
    fn lock_sees_through_a_source_less_prereq_to_a_sourced_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        // a (sourced) <- b (source-less) <- c (sourced): the gate is a's exam,
        // seen through b which never gates on its own.
        write_deck(dir.path(), "a.md", "---\nsource: https://x\n---\n## a\n1\n");
        write_deck(
            dir.path(),
            "b.md",
            "---\nrequires: a\n---\n## b <!-- id: q1 -->\n2\n",
        );
        let cpath = write_deck(
            dir.path(),
            "c.md",
            "---\nsource: https://y\nrequires: b\n---\n## c\n3\n",
        );
        let c = Deck::load(&cpath).unwrap();
        let a = Deck::load(dir.path().join("a.md")).unwrap();
        let b = Deck::load(dir.path().join("b.md")).unwrap();
        let (mut store, _s) = empty_store();
        let dd = Some(dir.path());

        // a's exam not passed -> c locked (through the transparent b).
        assert!(is_locked(&c, dd, &store));
        // Drilling/finishing the source-less b changes nothing.
        retire(&mut store, &b.cards[0].id().unwrap());
        assert!(is_locked(&c, dd, &store));
        // Mastering a (its exam passed) unlocks c.
        store.set_deck_mastered(&a.subject, 1);
        assert!(!is_locked(&c, dd, &store));
    }

    #[test]
    fn missing_prerequisite_does_not_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "---\nrequires: nope\n---\n## a\n1\n");
        let deck = Deck::load(&path).unwrap();
        let (store, _s) = empty_store();
        assert!(!is_locked(&deck, Some(dir.path()), &store));
    }

    #[test]
    fn a_prerequisite_cycle_resolves_locked_instead_of_hanging() {
        // a requires b, b requires a: the cycle guard (already-visited paths are
        // skipped) must stop the recursion rather than looping forever, and since
        // neither's exam has passed, the deck stays locked.
        let dir = tempfile::tempdir().unwrap();
        write_deck(
            dir.path(),
            "a.md",
            "---\nsource: https://x\nrequires: b\n---\n## a\n1\n",
        );
        write_deck(
            dir.path(),
            "b.md",
            "---\nsource: https://y\nrequires: a\n---\n## b\n2\n",
        );
        let a = Deck::load(dir.path().join("a.md")).unwrap();
        let (store, _s) = empty_store();
        assert!(is_locked(&a, Some(dir.path()), &store));
    }

    #[test]
    fn resolve_dep_strips_any_extension_and_matches_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "x").unwrap();
        // As given.
        let found = resolve_dep("notes.md", Some(dir.path()), None).unwrap();
        assert_eq!(dir.path().join("notes.md"), found);
        // Bare name gains `.md`.
        let found = resolve_dep("notes", Some(dir.path()), None).unwrap();
        assert_eq!(dir.path().join("notes.md"), found);
        // A legacy `.txt` value still resolves the `.md` deck (extension
        // stripped, `.md` matched).
        let found = resolve_dep("notes.txt", Some(dir.path()), None).unwrap();
        assert_eq!(dir.path().join("notes.md"), found);
    }

    #[test]
    fn load_deck_subject_is_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mydeck.md");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "## front\nback").unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!("mydeck.md", deck.subject);
        assert_eq!(1, deck.cards.len());
        assert_eq!("mydeck.md", &*deck.cards[0].subject);
    }

    #[test]
    fn insert_note_after_existing_card_content() {
        let text = "## one\nback 1\n> old note\n\n## two\nback 2\n";
        let notes = vec!["new a".to_string(), "new b".to_string()];
        let result = insert_note_lines(text, &fronts(text), 1, &notes);
        assert_eq!(
            "## one\nback 1\n> old note\n> new a\n> new b\n\n## two\nback 2\n",
            result
        );
        // The result must still parse, with the note extended.
        let cards = l1::parse_str("s.md", &result).unwrap();
        assert_eq!(Some("old note\nnew a\nnew b".to_string()), cards[0].note);
    }

    #[test]
    fn insert_note_on_last_card_without_note() {
        let text = "## one\nback 1\n";
        let result = insert_note_lines(text, &fronts(text), 1, &["note".to_string()]);
        assert_eq!("## one\nback 1\n> note\n", result);
        let cards = l1::parse_str("s.md", &result).unwrap();
        assert_eq!(Some("note".to_string()), cards[0].note);
    }

    #[test]
    fn insert_note_targets_the_right_card() {
        let text = "## one\nback 1\n\n## two\nback 2\n\n## three\nback 3\n";
        let result = insert_note_lines(text, &fronts(text), 4, &["mid".to_string()]);
        let cards = l1::parse_str("s.md", &result).unwrap();
        assert_eq!(None, cards[0].note);
        assert_eq!(Some("mid".to_string()), cards[1].note);
        assert_eq!(None, cards[2].note);
    }

    #[test]
    fn insert_note_is_not_fooled_by_a_fenced_heading() {
        // The card's answer holds a fenced `## `: the note must land after the
        // fence (still inside card one), not inside it.
        let text = "## one\n```\n## not a card\n```\ntail\n\n## two\nb\n";
        let result = insert_note_lines(text, &fronts(text), 1, &["n".to_string()]);
        assert_eq!(
            "## one\n```\n## not a card\n```\ntail\n> n\n\n## two\nb\n",
            result
        );
    }

    #[test]
    fn append_note_rewrites_the_file_and_card_ids_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## front <!-- id: q1 -->\nanswer\n").unwrap();

        let before = Deck::load(&path).unwrap();
        append_note(&path, 1, &["explained".to_string()]).unwrap();
        let after = Deck::load(&path).unwrap();

        assert_eq!(Some("explained".to_string()), after.cards[0].note);
        // Notes are never identity: the token (and so the id) is unchanged.
        assert_eq!(before.cards[0].id(), after.cards[0].id());
    }

    #[test]
    fn remove_card_block_drops_front_back_and_trailing_blank() {
        let text = "## one\nback 1\n> a note\n\n## two\nback 2\n";
        // Removing the first card takes its note and the blank separator too.
        assert_eq!(
            "## two\nback 2\n",
            remove_card_blocks(text, &fronts(text), &[1])
        );
        // Removing the last card leaves the first intact.
        assert_eq!(
            "## one\nback 1\n> a note\n",
            remove_card_blocks(text, &fronts(text), &[5])
        );
    }

    #[test]
    fn remove_card_block_keeps_header_and_neighbors() {
        let text = "---\nrequires: base\nlink: https://x\n---\n## a\nx\n## b\ny\n## c\nz\n";
        // The middle card goes; the frontmatter and the other two stay.
        assert_eq!(
            "---\nrequires: base\nlink: https://x\n---\n## a\nx\n## c\nz\n",
            remove_card_blocks(text, &fronts(text), &[7])
        );
    }

    #[test]
    fn remove_card_block_is_not_fooled_by_a_fenced_heading() {
        // A fenced `## ` is content, not a boundary: removing card one takes
        // the whole fence with it.
        let text = "## q\n```\n## not a card\n```\n## next\nb\n";
        assert_eq!(
            "## next\nb\n",
            remove_card_blocks(text, &fronts(text), &[1])
        );
    }

    #[test]
    fn remove_multiple_and_stale_line_is_ignored() {
        let text = "## a\nx\n## b\ny\n## c\nz\n";
        // Remove a and c; a line that isn't a front (2) is ignored.
        assert_eq!(
            "## b\ny\n",
            remove_card_blocks(text, &fronts(text), &[1, 2, 5])
        );
        // Removing everything yields an empty file (no stray newline).
        assert_eq!("", remove_card_blocks(text, &fronts(text), &[1, 3, 5]));
    }

    #[test]
    fn remove_cards_rewrites_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## one\nback 1\n\n## two\nback 2\n").unwrap();

        remove_cards(&path, &[1]).unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!("two", deck.cards[0].front);
    }

    #[test]
    fn settings_parsed_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nreveal: line\norder: sequential\ndirection: bogus\n---\n## f\nb\n",
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Reveal::Line), deck.settings.reveal);
        assert_eq!(Some(Order::Sequential), deck.settings.order);
        // An unparseable value is linted (doctor material), not an error.
        assert_eq!(None, deck.settings.direction);
    }

    #[test]
    fn origin_cascades_workspace_then_deck() {
        // The deck's own `origin` wins over the workspace `[defaults]`.
        let mut deck =
            DeckSettings::from_directives(&[("origin".to_string(), "/deck".to_string())]);
        deck.fill_from(&DeckSettings::from_directives(&[(
            "origin".to_string(),
            "/ws".to_string(),
        )]));
        assert_eq!(Some("/deck".to_string()), deck.origin);
        // The workspace default fills in when the deck is silent.
        let mut bare = DeckSettings::default();
        bare.fill_from(&DeckSettings::from_directives(&[(
            "origin".to_string(),
            "/ws".to_string(),
        )]));
        assert_eq!(Some("/ws".to_string()), bare.origin);
    }

    #[test]
    fn reference_links_union_url_sources_excluding_files_and_dupes() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nlink: https://a.example\nsource:\n  - https://b.example\n  - notes.md\n  - https://a.example\n---\n## f\nb\n",
        );
        let deck = Deck::load(&path).unwrap();
        // Links first, then URL sources not already present. The local-file
        // source and the source that duplicates a link are dropped.
        assert_eq!(
            vec!["https://a.example", "https://b.example"],
            deck.reference_links()
        );
    }

    #[test]
    fn strictness_directive_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nstrictness: strict\n---\n## f\nb\n",
        );
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Strictness::Strict), deck.settings.exam_strictness);

        // Absent key leaves it unset (the config default applies later).
        let bare = write_deck(dir.path(), "e.md", "## f\nb\n");
        assert_eq!(None, Deck::load(&bare).unwrap().settings.exam_strictness);

        // An unparseable value is linted, not an error.
        let bad = write_deck(dir.path(), "g.md", "---\nstrictness: harsh\n---\n## f\nb\n");
        assert_eq!(None, Deck::load(&bad).unwrap().settings.exam_strictness);
    }

    #[test]
    fn reveal_directive_parses_and_stamps_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u.md");
        std::fs::write(&path, "---\nreveal: line\n---\n## steps?\none\ntwo\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Reveal::Line), deck.settings.reveal);
        assert_eq!(Some(Reveal::Line), deck.cards[0].reveal); // stamped onto the card
    }

    #[test]
    fn requires_parsed_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "---\nrequires:\n  - basics\n  - x\n---\n## f\nb\n").unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(vec!["basics".to_string(), "x".to_string()], deck.requires);
    }

    #[test]
    fn card_reveal_is_card_override_else_deck_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nreveal: flip\n---\n## a <!-- reveal: line -->\nx\n## b\ny\n",
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Reveal::Line), deck.cards[0].reveal); // card override wins
        assert_eq!(Some(Reveal::Flip), deck.cards[1].reveal); // inherits the deck's
    }

    #[test]
    fn card_input_is_card_override_else_deck_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\ninput: draw\n---\n## a <!-- input: type -->\nx\n## b\ny\n",
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Input::Type), deck.cards[0].input); // card override wins
        assert_eq!(Some(Input::Draw), deck.cards[1].input); // inherits the deck's
    }

    #[test]
    fn cards_have_no_reveal_without_directives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## a\nx\n").unwrap();
        assert_eq!(None, Deck::load(&path).unwrap().cards[0].reveal);
    }

    #[test]
    fn direction_both_expands_to_forward_and_reverse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "## purported <!-- id: q1 --> <!-- direction: both -->\nangeblich\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(2, deck.cards.len());
        assert_eq!("purported", deck.cards[0].front);
        assert_eq!(vec!["angeblich"], deck.cards[0].back);
        assert_eq!("angeblich", deck.cards[1].front);
        assert_eq!(vec!["purported"], deck.cards[1].back);
        assert_eq!(deck.cards[0].line, deck.cards[1].line); // sibling group
        // Distinct identities: `q1` forward, `q1-r` reversed.
        assert_eq!(Some("q1".to_string()), deck.cards[0].id());
        assert_eq!(Some("q1-r".to_string()), deck.cards[1].id());
        assert_ne!(deck.cards[0].id(), deck.cards[1].id());
    }

    #[test]
    fn direction_reverse_keeps_only_the_swapped_card() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## q <!-- direction: reverse -->\na\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!("a", deck.cards[0].front);
        assert_eq!(vec!["q"], deck.cards[0].back);
    }

    #[test]
    fn deck_level_direction_applies_to_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "---\ndirection: both\n---\n## a\nb\n").unwrap();
        assert_eq!(2, Deck::load(&path).unwrap().cards.len());
    }

    #[test]
    fn direction_does_not_apply_to_cloze() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        // Deck-level `both` must not reverse a cloze card (one hole -> one
        // card): the never-reverse guard keys on the hole, not the direction.
        std::fs::write(
            &path,
            "---\ndirection: both\n---\n## fill\nThe \\cloze{x} thing.\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
        assert!(!deck.cards[0].reversed);
    }

    #[test]
    fn image_resolves_against_img_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nimg-dir: /assets/imgs\n---\n## q <!-- img: moon.png -->\nWaxing\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            Some(PathBuf::from("/assets/imgs/moon.png")),
            deck.cards[0].image
        );
    }

    #[test]
    fn image_resolves_against_deck_dir_without_img_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## q <!-- img: moon.png -->\nWaxing\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(dir.path().join("moon.png")), deck.cards[0].image);
    }

    #[test]
    fn absolute_card_image_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nimg-dir: /assets\n---\n## q <!-- img: /elsewhere/moon.png -->\nWaxing\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            Some(PathBuf::from("/elsewhere/moon.png")),
            deck.cards[0].image
        );
    }

    #[test]
    fn workspace_defaults_fill_unset_and_reach_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        // Deck declares no direction/reveal of its own.
        std::fs::write(&path, "## purported\nangeblich\n").unwrap();
        let defaults = DeckSettings {
            direction: Some(Direction::Both),
            reveal: Some(Reveal::Line),
            ..Default::default()
        };
        let deck = Deck::load_with_defaults(&path, &defaults).unwrap();
        // Workspace `direction: both` reached the cards (expanded the pair)...
        assert_eq!(2, deck.cards.len());
        // ...and `reveal` folded onto them.
        assert_eq!(Some(Reveal::Line), deck.cards[0].reveal);
    }

    #[test]
    fn deck_directive_overrides_workspace_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "---\ndirection: forward\n---\n## a\nb\n").unwrap();
        let defaults = DeckSettings {
            direction: Some(Direction::Both),
            ..Default::default()
        };
        let deck = Deck::load_with_defaults(&path, &defaults).unwrap();
        // The deck's own `forward` wins over the workspace's `both`.
        assert_eq!(1, deck.cards.len());
    }

    #[test]
    fn display_name_uses_title_else_stripped_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Eng-Sayings.md");
        std::fs::write(&path, "## a\nb\n").unwrap();
        assert_eq!("Eng-Sayings", Deck::load(&path).unwrap().display_name());

        std::fs::write(&path, "# English Sayings\n\n## a\nb\n").unwrap();
        assert_eq!("English Sayings", Deck::load(&path).unwrap().display_name());

        // A trace deck with no `# H1` shows its `trace:` description.
        std::fs::write(
            &path,
            "---\ntrace: how a keypress becomes a grade\n---\n## a\nb\n",
        )
        .unwrap();
        assert_eq!(
            "how a keypress becomes a grade",
            Deck::load(&path).unwrap().display_name()
        );
    }

    #[test]
    fn no_directives_yields_empty_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "just a comment\n\n## f\nb\n").unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(None, deck.settings.reveal);
        assert_eq!(None, deck.settings.input);
        assert_eq!(None, deck.settings.order);
    }

    #[test]
    fn rewrite_trace_snapshot_repoints_source_origin_and_each_at() {
        let text = "---\ntrace: how X\nsource: ..\n---\n\n## q1\np\n<!-- at: a.rs:90-98 -->\n## q2\np\n<!-- at: b.rs:1 -->\n";
        let span = l1::parse_l1("t.md", text).unwrap().frontmatter_span;
        let ats = [
            AtRewrite {
                at: "01.rs".into(),
                origin: Some("src/a.rs:90-98".into()),
            },
            AtRewrite {
                at: "02.rs".into(),
                origin: Some("src/b.rs:1".into()),
            },
        ];
        let out = rewrite_trace_snapshot(text, span, "assets", Some("/crate"), &ats);
        assert!(out.contains("source: assets\n"), "{out}");
        assert_eq!(1, out.matches("source:").count()); // replaced, not added
        assert!(out.contains("origin: /crate\n"), "{out}"); // origin written after source
        // The provenance rides the `at:` directive as ` from …`.
        assert!(
            out.contains("<!-- at: 01.rs from src/a.rs:90-98 -->\n"),
            "{out}"
        );
        assert!(
            out.contains("<!-- at: 02.rs from src/b.rs:1 -->\n"),
            "{out}"
        );
        assert!(out.contains("trace: how X\n")); // the trace key is kept
        assert!(out.ends_with('\n'));
    }
}
