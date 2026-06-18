//! A local web frontend.
//!
//! `flash serve` starts a small synchronous HTTP server (one request at a
//! time — correct for a single user) that serves an embedded web page and a
//! JSON API. It is a third consumer of the same logic the TUI and browser use:
//! the [`Session`]/[`Store`] drive review, and cards are sent to the browser as
//! a DTO built from [`render::note_units`], so the note structuring lives in
//! one place. Grades persist to the same progress store, so studying in the
//! browser and on the command line share one history.
//!
//! It is deliberately local-only: no accounts, no database. By default it
//! binds to `127.0.0.1`; `--lan` binds all interfaces so a phone or tablet on
//! the same network can reach it (there is no authentication, so that is
//! opt-in).

use std::{
    collections::{BTreeSet, HashMap},
    hash::Hasher,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server};
use twox_hash::XxHash64;

use crate::{
    answer::{Mode, grade_lines_unordered},
    card::Card,
    choice,
    config::{Bindings, BrowseBindings, Key, KeyPattern},
    deck::{self, Deck, DeckState},
    picker,
    recent::RecentDecks,
    render::{self, NoteUnit},
    scheduler::Grade,
    session::{Session, now_ms},
    store::{MAX_STAGE, Store},
};

/// Per-deck data the server needs to apply a removal: the file path, plus the
/// file's original text so removals can be re-derived from a fixed snapshot
/// (see [`deck::rewrite_without_cards`]).
struct DeckFiles {
    /// Subject → file path.
    paths: HashMap<String, PathBuf>,
    /// Subject → original file text (decks whose text could not be read are
    /// absent, and simply cannot have cards removed).
    snapshots: HashMap<String, String>,
    /// Subject → the 1-based front lines removed so far this run.
    removed: HashMap<String, BTreeSet<usize>>,
}

impl DeckFiles {
    fn new(paths: HashMap<String, PathBuf>) -> Self {
        let snapshots = paths
            .iter()
            .filter_map(|(subject, path)| {
                std::fs::read_to_string(path)
                    .ok()
                    .map(|text| (subject.clone(), text))
            })
            .collect();
        Self {
            paths,
            snapshots,
            removed: HashMap::new(),
        }
    }

    /// Records that the card block at `line` of `subject` was removed and
    /// rewrites the deck file from its snapshot. Best-effort.
    fn remove_block(&mut self, subject: &str, line: usize) {
        let lines = self.removed.entry(subject.to_string()).or_default();
        lines.insert(line);
        if let (Some(path), Some(original)) =
            (self.paths.get(subject), self.snapshots.get(subject))
        {
            let lines: Vec<usize> = lines.iter().copied().collect();
            if let Err(e) = deck::rewrite_without_cards(path, original, &lines) {
                eprintln!("warning: could not update {}: {e}", path.display());
            }
        }
    }
}

const REVIEW_HTML: &str = include_str!("../assets/serve/review.html");
const BROWSE_HTML: &str = include_str!("../assets/serve/browse.html");

/// One display unit of a card's note, ready for JSON. Mirrors
/// [`render::NoteUnit`]; the web page renders `sentence` as a paragraph and
/// `code` as a verbatim block.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum NoteUnitDto {
    Sentence { text: String },
    Code { lines: Vec<String> },
}

impl From<NoteUnit> for NoteUnitDto {
    fn from(unit: NoteUnit) -> Self {
        match unit {
            NoteUnit::Sentence(text) => NoteUnitDto::Sentence { text },
            NoteUnit::Code(lines) => NoteUnitDto::Code { lines },
        }
    }
}

/// A card serialized for the browser.
#[derive(Debug, Serialize)]
struct CardDto {
    front: String,
    context: Vec<String>,
    back: Vec<String>,
    note: Vec<NoteUnitDto>,
    /// `/img/<key>` URL for the question-side image, or `null`.
    img: Option<String>,
    /// `/img/<key>` URL for the answer-side image, shown on reveal, or `null`.
    img_back: Option<String>,
}

/// The current review state sent to the browser after every action.
#[derive(Debug, Serialize)]
struct StateDto {
    /// `"select"` while choosing decks (no session yet), else `"review"`.
    phase: &'static str,
    /// The card up for review, or `null` when finished or in the select phase.
    card: Option<CardDto>,
    /// For `choice` mode, the multiple-choice options (one is correct); `null`
    /// otherwise, or when the card has too few distractors (the page then
    /// falls back to reveal). The correct index is never sent here.
    choices: Option<Vec<String>>,
    /// The answer mode name (`flip`, `line`, …); the page reveals
    /// line-by-line for `line` and flip-style otherwise.
    mode: &'static str,
    remaining: usize,
    initial: usize,
    reviews: usize,
    passed: usize,
    failed: usize,
    /// Per-stage counts (index 0 = unseen).
    histogram: [usize; 6],
    finished: bool,
    /// Whether a restart would find any due/new cards right now. The summary
    /// disables "New session" and shows a "nothing due" note when this is false.
    can_restart: bool,
    label: String,
}

/// The payload of the browse view: every (remaining) card, in deck order, or
/// an empty list in the `select` phase.
#[derive(Debug, Serialize)]
struct BrowseDto {
    /// `"select"` while choosing decks, else `"browse"`.
    phase: &'static str,
    label: String,
    cards: Vec<CardDto>,
}

/// The deck-selection catalog sent to the browser picker.
#[derive(Debug, Serialize)]
struct DeckListDto {
    decks: Vec<DeckItemDto>,
}

/// One deck offered in the selection screen: its file name, a completion-state
/// label (`new` / `m/total` / `done ✓`), a machine-readable `state`
/// (`new`/`started`/`finished`) for styling, and whether it is locked by an
/// unfinished `% requires:` prerequisite. Mirrors the TUI picker rows.
#[derive(Debug, Serialize)]
struct DeckItemDto {
    name: String,
    meta: Option<String>,
    state: &'static str,
    locked: bool,
}

/// The result of answering a choice card: which option was picked, which is
/// correct, and whether they match. The page highlights the options with this.
#[derive(Debug, Serialize)]
struct ChooseFeedbackDto {
    chosen: usize,
    correct: usize,
    passed: bool,
}

/// One typed line graded against the expected answer (typing / fuzzy mode).
#[derive(Debug, Serialize)]
struct LineResultDto {
    input: String,
    expected: String,
    passed: bool,
    distance: usize,
}

/// The result of submitting a typed answer: a result per back line plus whether
/// they all passed.
#[derive(Debug, Serialize)]
struct CheckFeedbackDto {
    results: Vec<LineResultDto>,
    passed: bool,
}

/// One configured key, as the browser sees it: `k` is the `KeyboardEvent.key`
/// value (`" "`, `"Enter"`, `"j"`, …) and `ctrl` whether Ctrl must be held.
#[derive(Debug, Serialize)]
struct KeyDto {
    k: String,
    ctrl: bool,
}

fn key_dto(p: &KeyPattern) -> KeyDto {
    let k = match p.key {
        Key::Char(' ') => " ".to_string(),
        Key::Char(c) => c.to_string(),
        Key::Enter => "Enter".to_string(),
        Key::Tab => "Tab".to_string(),
        Key::Esc => "Escape".to_string(),
        Key::Backspace => "Backspace".to_string(),
    };
    KeyDto { k, ctrl: p.ctrl }
}

fn key_list(list: &[KeyPattern]) -> Vec<KeyDto> {
    list.iter().map(key_dto).collect()
}

/// The review actions the web page binds, mirroring the configured `[keys]`.
#[derive(Debug, Serialize)]
struct ReviewKeys {
    reveal: Vec<KeyDto>,
    again: Vec<KeyDto>,
    good: Vec<KeyDto>,
    easy: Vec<KeyDto>,
    skip: Vec<KeyDto>,
    remove: Vec<KeyDto>,
    restart: Vec<KeyDto>,
}

impl ReviewKeys {
    fn from(b: &Bindings) -> Self {
        Self {
            reveal: key_list(&b.reveal),
            again: key_list(&b.again),
            good: key_list(&b.good),
            easy: key_list(&b.easy),
            skip: key_list(&b.skip),
            remove: key_list(&b.remove),
            restart: key_list(&b.restart),
        }
    }
}

/// The browse actions the web page binds, mirroring the configured `[browse]`.
#[derive(Debug, Serialize)]
struct BrowseKeys {
    next: Vec<KeyDto>,
    prev: Vec<KeyDto>,
    remove: Vec<KeyDto>,
}

impl BrowseKeys {
    fn from(b: &BrowseBindings) -> Self {
        Self {
            next: key_list(&b.next),
            prev: key_list(&b.prev),
            remove: key_list(&b.remove),
        }
    }
}

/// Global options for a served review, independent of which decks are chosen
/// (the per-session label and deck paths come from [`SessionBuild`]).
pub struct ReviewOptions {
    /// CLI `--mode` override; `None` lets each card use its own mode.
    pub mode_override: Option<Mode>,
    pub keys: Bindings,
    /// Fuzzy-mode typo tolerance per line.
    pub max_typos: usize,
}

/// A review session ready to serve: the session, its header label, and the
/// subject → deck file path map used for card removal. Produced by the caller's
/// builder closure when decks are chosen (on the CLI or in the browser picker).
pub struct SessionBuild {
    pub session: Session,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
}

/// A browse card list ready to serve, with its label and deck paths.
pub struct CardsBuild {
    pub cards: Vec<Card>,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
}

/// The server's live review state once decks are chosen. Its absence (`None`)
/// means the page is in the deck-selection phase.
struct Reviewing {
    session: Session,
    label: String,
    files: DeckFiles,
    images: HashMap<String, PathBuf>,
}

impl Reviewing {
    fn new(build: SessionBuild) -> Self {
        let images = collect_images(build.session.cards());
        Self {
            session: build.session,
            label: build.label,
            files: DeckFiles::new(build.decks),
            images,
        }
    }
}

/// Serves review at `addr` until the process is stopped. When `initial` is
/// `None` the server opens at the in-browser deck-selection screen; picking
/// decks (`POST /api/select`) calls `build` to construct a session in place.
/// `build` borrows the shared `store` and `recent`, so all sessions write one
/// history and update the recent-decks list, exactly like the CLI.
pub fn run_review(
    initial: Option<SessionBuild>,
    mut store: Store,
    mut recent: RecentDecks,
    decks_dir: PathBuf,
    addr: SocketAddr,
    opts: ReviewOptions,
    mut build: impl FnMut(Vec<PathBuf>, &Store, &mut RecentDecks) -> Result<SessionBuild>,
) -> Result<()> {
    let ReviewOptions {
        mode_override,
        keys: bindings,
        max_typos,
    } = opts;
    let keys = ReviewKeys::from(&bindings);
    let mut reviewing = initial.map(Reviewing::new);
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, REVIEW_HTML),
            (Method::Get, "/api/keys") => respond_json(request, &keys),
            (Method::Get, "/api/decks") => {
                respond_json(request, &deck_catalog(&decks_dir, &recent, &store))
            }
            (Method::Get, key) if key.starts_with("/img/") => match &reviewing {
                Some(r) => serve_image(request, &r.images, &key["/img/".len()..]),
                None => respond_status(request, 404),
            },
            (Method::Get, "/api/state") => {
                respond_json(request, &review_state(reviewing.as_ref(), &store, mode_override))
            }
            (Method::Post, "/api/select") => {
                match select_decks(&mut request, &decks_dir, &recent) {
                    Some(paths) => match build(paths, &store, &mut recent) {
                        Ok(b) => {
                            reviewing = Some(Reviewing::new(b));
                            respond_json(
                                request,
                                &review_state(reviewing.as_ref(), &store, mode_override),
                            );
                        }
                        Err(e) => {
                            eprintln!("warning: could not load the selected decks: {e}");
                            respond_status(request, 400);
                        }
                    },
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/deselect") => {
                reviewing = None;
                respond_json(request, &review_state(reviewing.as_ref(), &store, mode_override));
            }
            (Method::Post, "/api/grade") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                match read_grade(&mut request) {
                    Some(grade) => {
                        r.session.grade(&mut store, grade, now_ms());
                        if let Err(e) = store.save() {
                            eprintln!("warning: could not save progress: {e}");
                        }
                        respond_json(
                            request,
                            &review_state(reviewing.as_ref(), &store, mode_override),
                        );
                    }
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/skip") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.skip();
                respond_json(request, &review_state(reviewing.as_ref(), &store, mode_override));
            }
            (Method::Post, "/api/check") => {
                let Some(r) = reviewing.as_ref() else {
                    respond_status(request, 409);
                    continue;
                };
                // Grade the typed lines against the current card — exact for
                // typing (tolerance 0), typo-tolerant for fuzzy. Like choose,
                // this only checks; the grade is applied on Continue.
                #[derive(Deserialize)]
                struct Body {
                    lines: Vec<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let result = body.and_then(|body| {
                    let card = r.session.current()?;
                    let mode = mode_override.or(card.mode).unwrap_or_default();
                    let tol = if mode == Mode::Typing { 0 } else { max_typos };
                    // Order-independent: a multi-item answer can be typed in any
                    // order, each line matched to its closest expected line.
                    let results: Vec<LineResultDto> =
                        grade_lines_unordered(&body.lines, &card.back, tol)
                            .into_iter()
                            .map(|r| LineResultDto {
                                input: r.input,
                                expected: r.expected,
                                passed: r.passed,
                                distance: r.distance,
                            })
                            .collect();
                    let passed = results.iter().all(|r| r.passed);
                    Some(CheckFeedbackDto { results, passed })
                });
                match result {
                    Some(f) => respond_json(request, &f),
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/choose") => {
                let Some(r) = reviewing.as_ref() else {
                    respond_status(request, 409);
                    continue;
                };
                // Just reports which option is correct (the question is rebuilt
                // from the card id, so it matches the one served). The grade is
                // applied later via /api/grade on Continue, so the session stays
                // on this card during the result — Remove still works on it.
                let picked = read_index(&mut request).and_then(|chosen| {
                    let card = r.session.current()?.clone();
                    let correct = choice::build(&card, r.session.cards(), card.id())?.correct;
                    Some((chosen, correct))
                });
                match picked {
                    Some((chosen, correct)) => respond_json(
                        request,
                        &ChooseFeedbackDto {
                            chosen,
                            correct,
                            passed: chosen == correct,
                        },
                    ),
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/remove") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let dropped = r.session.remove_current();
                if let Some(first) = dropped.first() {
                    let subject = first.subject.to_string();
                    let line = first.line;
                    for card in &dropped {
                        store.remove(card.id());
                    }
                    let _ = store.save();
                    r.files.remove_block(&subject, line);
                }
                respond_json(request, &review_state(reviewing.as_ref(), &store, mode_override));
            }
            (Method::Post, "/api/restart") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.restart(&store, now_ms());
                respond_json(request, &review_state(reviewing.as_ref(), &store, mode_override));
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
}

/// The server's live browse state once decks are chosen. Its absence (`None`)
/// means the deck-selection phase.
struct Browsing {
    cards: Vec<Card>,
    label: String,
    files: DeckFiles,
    images: HashMap<String, PathBuf>,
}

impl Browsing {
    fn new(build: CardsBuild) -> Self {
        let images = collect_images(&build.cards);
        Self {
            cards: build.cards,
            label: build.label,
            files: DeckFiles::new(build.decks),
            images,
        }
    }
}

/// Serves a read-only walk through cards at `addr`. Like [`run_review`], with
/// `initial` `None` it opens at the deck-selection screen; `POST /api/select`
/// builds the card list via `build`. The only thing it writes is card removal
/// (deletes the card from its deck file and prunes its progress in `store`).
pub fn run_browse(
    initial: Option<CardsBuild>,
    mut store: Store,
    mut recent: RecentDecks,
    decks_dir: PathBuf,
    addr: SocketAddr,
    bindings: BrowseBindings,
    mut build: impl FnMut(Vec<PathBuf>, &mut RecentDecks) -> Result<CardsBuild>,
) -> Result<()> {
    let keys = BrowseKeys::from(&bindings);
    let mut browsing = initial.map(Browsing::new);
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, BROWSE_HTML),
            (Method::Get, "/api/keys") => respond_json(request, &keys),
            (Method::Get, "/api/decks") => {
                respond_json(request, &deck_catalog(&decks_dir, &recent, &store))
            }
            (Method::Get, key) if key.starts_with("/img/") => match &browsing {
                Some(b) => serve_image(request, &b.images, &key["/img/".len()..]),
                None => respond_status(request, 404),
            },
            (Method::Get, "/api/cards") => respond_json(request, &browse_payload(browsing.as_ref())),
            (Method::Post, "/api/select") => {
                match select_decks(&mut request, &decks_dir, &recent) {
                    Some(paths) => match build(paths, &mut recent) {
                        Ok(b) => {
                            browsing = Some(Browsing::new(b));
                            respond_json(request, &browse_payload(browsing.as_ref()));
                        }
                        Err(e) => {
                            eprintln!("warning: could not load the selected decks: {e}");
                            respond_status(request, 400);
                        }
                    },
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/deselect") => {
                browsing = None;
                respond_json(request, &browse_payload(browsing.as_ref()));
            }
            (Method::Post, "/api/remove") => {
                let Some(b) = browsing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(index) = read_index(&mut request)
                    && index < b.cards.len()
                {
                    let subject = b.cards[index].subject.to_string();
                    let line = b.cards[index].line;
                    // Drop the card and any cloze siblings, pruning their
                    // progress as they go.
                    b.cards.retain(|card| {
                        let sibling = card.subject.as_ref() == subject && card.line == line;
                        if sibling {
                            store.remove(card.id());
                        }
                        !sibling
                    });
                    let _ = store.save();
                    b.files.remove_block(&subject, line);
                }
                respond_json(request, &browse_payload(browsing.as_ref()));
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
}

/// Serializes the current browse phase for the page: the cards in browse phase,
/// or an empty list flagged `phase: "select"` for the deck-selection screen.
fn browse_payload(browsing: Option<&Browsing>) -> BrowseDto {
    match browsing {
        Some(b) => BrowseDto {
            phase: "browse",
            label: b.label.clone(),
            cards: b.cards.iter().map(card_dto).collect(),
        },
        None => BrowseDto {
            phase: "select",
            label: "select decks".to_string(),
            cards: Vec::new(),
        },
    }
}

/// The path part of a request URL, without any `?query`.
fn request_path(request: &Request) -> String {
    request
        .url()
        .split('?')
        .next()
        .unwrap_or("")
        .to_string()
}

/// Builds the state payload. In the select phase (`reviewing` is `None`) it
/// reports `phase: "select"` with no card; otherwise it serializes the live
/// session and store. For choice mode it also builds the options, seeded by the
/// card id so they are stable across the `/api/state` and `/api/choose` requests
/// without any server-side caching.
fn review_state(reviewing: Option<&Reviewing>, store: &Store, mode_override: Option<Mode>) -> StateDto {
    let Some(r) = reviewing else {
        return StateDto {
            phase: "select",
            card: None,
            choices: None,
            mode: mode_name(Mode::default()),
            remaining: 0,
            initial: 0,
            reviews: 0,
            passed: 0,
            failed: 0,
            histogram: [0; 6],
            finished: false,
            can_restart: false,
            label: "select decks".to_string(),
        };
    };
    let session = &r.session;
    let card = session.current();
    // CLI override wins; otherwise the current card's own mode, else default.
    let mode = mode_override.or(card.and_then(|c| c.mode)).unwrap_or_default();
    let choices = if mode == Mode::Choice {
        card.and_then(|c| choice::build(c, session.cards(), c.id()).map(|q| q.options))
    } else {
        None
    };
    StateDto {
        phase: "review",
        card: card.map(card_dto),
        choices,
        mode: mode_name(mode),
        remaining: session.remaining(),
        initial: session.initial_size,
        reviews: session.stats.reviews,
        passed: session.stats.passed,
        failed: session.stats.failed,
        histogram: session.stage_histogram(store),
        finished: session.is_finished(),
        can_restart: session.has_due_now(store, now_ms()),
        label: r.label.clone(),
    }
}

/// Builds the deck-selection catalog (recent decks first, then `decks_dir`),
/// with each deck's completion state and lock status derived from `store` —
/// mirroring the TUI picker rows.
fn deck_catalog(decks_dir: &Path, recent: &RecentDecks, store: &Store) -> DeckListDto {
    let decks = picker::catalog(decks_dir, recent)
        .into_iter()
        .map(|e| {
            let (state, meta, locked) = match Deck::load(&e.path) {
                Ok(deck) => {
                    let total = deck.cards.len();
                    let maxed = deck
                        .cards
                        .iter()
                        .filter(|c| store.get(c.id()).is_some_and(|s| s.stage >= MAX_STAGE))
                        .count();
                    let (state, label) = match deck.state(store) {
                        DeckState::Finished => ("finished", "done ✓".to_string()),
                        DeckState::NotStarted => ("new", "new".to_string()),
                        DeckState::Started => ("started", format!("{maxed}/{total}")),
                    };
                    let locked = deck::is_locked(&deck, Some(decks_dir), store);
                    (state, Some(label), locked)
                }
                Err(_) => ("new", None, false),
            };
            DeckItemDto {
                name: e.name,
                meta,
                state,
                locked,
            }
        })
        .collect();
    DeckListDto { decks }
}

/// Parses a `{"decks":[name,…]}` selection and resolves each name to its deck
/// path via the live catalog. Returns `None` (→ 400) for an empty or malformed
/// body, or any name not in the catalog — so no filesystem path is ever built
/// from request input, keeping selection safe under `--lan`.
fn select_decks(
    request: &mut Request,
    decks_dir: &Path,
    recent: &RecentDecks,
) -> Option<Vec<PathBuf>> {
    #[derive(Deserialize)]
    struct Body {
        decks: Vec<String>,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    if body.decks.is_empty() {
        return None;
    }
    let known: HashMap<String, PathBuf> = picker::catalog(decks_dir, recent)
        .into_iter()
        .map(|e| (e.name, e.path))
        .collect();
    resolve_names(body.decks, &known)
}

/// Maps each requested deck name to its catalog path. Returns `None` if any name
/// is not in the catalog, so an unknown or crafted name is rejected wholesale
/// rather than reaching the filesystem.
fn resolve_names(names: Vec<String>, known: &HashMap<String, PathBuf>) -> Option<Vec<PathBuf>> {
    names.into_iter().map(|n| known.get(&n).cloned()).collect()
}

/// Serializes a card for the browser, structuring its note via the shared
/// [`render`] model.
fn card_dto(card: &Card) -> CardDto {
    let img_url = |p: &PathBuf| format!("/img/{}", img_key(p));
    CardDto {
        front: card.front.clone(),
        context: card.context.clone(),
        back: card.back.clone(),
        note: render::note_units(card)
            .into_iter()
            .map(NoteUnitDto::from)
            .collect(),
        img: card.image.as_ref().map(img_url),
        img_back: card.image_back.as_ref().map(img_url),
    }
}

/// A stable, opaque URL key for a resolved image path: the hex `XxHash64` of the
/// path. The card DTO and the image registry derive it the same way, so only
/// paths a deck actually references resolve — no user input is joined to a path,
/// which keeps `/img/` safe from traversal even under `--lan`.
fn img_key(path: &Path) -> String {
    let mut hasher = XxHash64::default();
    hasher.write(path.to_string_lossy().as_bytes());
    format!("{:016x}", hasher.finish())
}

/// Builds the `key → absolute path` registry the `/img/` route serves from, by
/// scanning every card's resolved image sides.
fn collect_images(cards: &[Card]) -> HashMap<String, PathBuf> {
    let mut images = HashMap::new();
    for card in cards {
        for path in [&card.image, &card.image_back].into_iter().flatten() {
            images.insert(img_key(path), path.clone());
        }
    }
    images
}

/// The MIME type to serve a card image with, by file extension. Unknown
/// extensions fall back to a generic binary type (the browser still sniffs it).
fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// The CLI/value name of an answer mode, matching `Mode`'s clap names.
fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Flip => "flip",
        Mode::Typing => "typing",
        Mode::Fuzzy => "fuzzy",
        Mode::Choice => "choice",
        Mode::LineByLine => "line",
    }
}

/// Parses a `{"grade":"again|good|easy"}` POST body into a [`Grade`].
fn read_grade(request: &mut Request) -> Option<Grade> {
    #[derive(Deserialize)]
    struct Body {
        grade: String,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    match body.grade.as_str() {
        "again" => Some(Grade::Fail),
        "good" => Some(Grade::Pass),
        "easy" => Some(Grade::Easy),
        _ => None,
    }
}

/// Parses a `{"index": n}` POST body (the browse card to remove).
fn read_index(request: &mut Request) -> Option<usize> {
    #[derive(Deserialize)]
    struct Body {
        index: usize,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    Some(body.index)
}

fn respond_json<T: Serialize>(request: Request, value: &T) {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap();
    let _ = request.respond(Response::from_string(body).with_header(header));
}

fn respond_html(request: Request, html: &str) {
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
    let _ = request.respond(Response::from_string(html.to_string()).with_header(header));
}

fn respond_status(request: Request, code: u16) {
    let _ = request.respond(Response::from_string(String::new()).with_status_code(code));
}

fn respond_bytes(request: Request, bytes: Vec<u8>, content_type: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let _ = request.respond(Response::from_data(bytes).with_header(header));
}

/// Serves the registered image for `key`, or 404 for an unknown key / unreadable
/// file. Shared by the review and browse routes.
fn serve_image(request: Request, images: &HashMap<String, PathBuf>, key: &str) {
    match images.get(key) {
        Some(path) => match std::fs::read(path) {
            Ok(bytes) => respond_bytes(request, bytes, content_type(path)),
            Err(_) => respond_status(request, 404),
        },
        None => respond_status(request, 404),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn card_dto_structures_the_note() {
        let note = "Intro here.\n```\nfn main() {}\n```";
        let card = Card::plain(
            Arc::from("s.txt"),
            "the front".to_string(),
            vec!["the back".to_string()],
            Some(note.to_string()),
            1,
        );
        let dto = card_dto(&card);

        assert_eq!(dto.front, "the front");
        assert_eq!(dto.back, vec!["the back".to_string()]);
        assert_eq!(dto.note.len(), 2);
        match &dto.note[0] {
            NoteUnitDto::Sentence { text } => assert_eq!(text, "Intro here."),
            other => panic!("expected a sentence, got {other:?}"),
        }
        match &dto.note[1] {
            NoteUnitDto::Code { lines } => assert_eq!(lines, &vec!["fn main() {}".to_string()]),
            other => panic!("expected a code block, got {other:?}"),
        }
    }

    #[test]
    fn card_dto_exposes_image_urls_and_registry_matches() {
        let mut card = Card::plain(
            Arc::from("s.txt"),
            "q".to_string(),
            vec!["a".to_string()],
            None,
            1,
        );
        card.image = Some(PathBuf::from("/imgs/moon.png"));
        card.image_back = Some(PathBuf::from("/imgs/tab.png"));

        let dto = card_dto(&card);
        let img = dto.img.expect("front image url");
        let img_back = dto.img_back.expect("back image url");
        assert!(img.starts_with("/img/"));
        assert!(img_back.starts_with("/img/") && img_back != img);

        // The registry keys the DTO's URLs derive from, so a request for either
        // URL resolves to the right file.
        let images = collect_images(std::slice::from_ref(&card));
        assert_eq!(
            images.get(img.strip_prefix("/img/").unwrap()),
            Some(&PathBuf::from("/imgs/moon.png"))
        );
        assert_eq!(
            images.get(img_back.strip_prefix("/img/").unwrap()),
            Some(&PathBuf::from("/imgs/tab.png"))
        );
    }

    #[test]
    fn plain_card_has_no_image_urls() {
        let card = Card::plain(Arc::from("s.txt"), "q".to_string(), vec!["a".to_string()], None, 1);
        let dto = card_dto(&card);
        assert!(dto.img.is_none() && dto.img_back.is_none());
        assert!(collect_images(std::slice::from_ref(&card)).is_empty());
    }

    #[test]
    fn content_type_by_extension() {
        assert_eq!(content_type(Path::new("a.png")), "image/png");
        assert_eq!(content_type(Path::new("a.JPG")), "image/jpeg");
        assert_eq!(content_type(Path::new("a.jpeg")), "image/jpeg");
        assert_eq!(content_type(Path::new("a.svg")), "image/svg+xml");
        assert_eq!(content_type(Path::new("a.bin")), "application/octet-stream");
    }

    #[test]
    fn resolve_names_rejects_unknown_deck() {
        let mut known = HashMap::new();
        known.insert("a.txt".to_string(), PathBuf::from("/decks/a.txt"));
        known.insert("b.txt".to_string(), PathBuf::from("/decks/b.txt"));
        // All known -> resolves to their catalog paths.
        assert_eq!(
            resolve_names(vec!["b.txt".to_string(), "a.txt".to_string()], &known),
            Some(vec![
                PathBuf::from("/decks/b.txt"),
                PathBuf::from("/decks/a.txt")
            ])
        );
        // One unknown name (e.g. a traversal attempt) rejects the whole request.
        assert_eq!(
            resolve_names(vec!["a.txt".to_string(), "../etc/passwd".to_string()], &known),
            None
        );
    }

    #[test]
    fn browse_payload_select_phase_has_no_cards() {
        let dto = browse_payload(None);
        assert_eq!(dto.phase, "select");
        assert!(dto.cards.is_empty());
    }

    #[test]
    fn review_state_select_phase_has_no_card() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        let dto = review_state(None, &store, None);
        assert_eq!(dto.phase, "select");
        assert!(dto.card.is_none());
        assert!(!dto.finished);
    }

    #[test]
    fn grade_names_map_to_grades() {
        // A guard so the JSON contract and the Grade enum stay in sync.
        assert!(matches!(Grade::Fail, Grade::Fail));
        assert_eq!(mode_name(Mode::LineByLine), "line");
        assert_eq!(mode_name(Mode::Flip), "flip");
    }
}
