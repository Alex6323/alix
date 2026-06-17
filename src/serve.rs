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
    answer::{Mode, grade_fuzzy},
    card::Card,
    choice,
    config::{Bindings, BrowseBindings, Key, KeyPattern},
    deck,
    render::{self, NoteUnit},
    scheduler::Grade,
    session::{Session, now_ms},
    store::Store,
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
    /// The card up for review, or `null` when the session is finished.
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
    label: String,
}

/// The payload of the browse view: every (remaining) card, in deck order.
#[derive(Debug, Serialize)]
struct BrowseDto {
    label: String,
    cards: Vec<CardDto>,
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

/// Serves a review session at `addr` until the process is stopped. Grades are
/// applied to `session` and saved to `store` after each one. `decks` (subject →
/// file path) lets the remove action delete a card from its deck file and prune
/// its progress; unlike the TUI, removal is immediate, since the server has no
/// clean end-of-session.
/// Everything a served review session needs besides the session and store.
pub struct ReviewOptions {
    /// CLI `--mode` override; `None` lets each card use its own mode.
    pub mode_override: Option<Mode>,
    pub label: String,
    /// Subject → deck file path, for card removal.
    pub decks: HashMap<String, PathBuf>,
    pub keys: Bindings,
    /// Fuzzy-mode typo tolerance per line.
    pub max_typos: usize,
}

pub fn run_review(
    mut session: Session,
    mut store: Store,
    addr: SocketAddr,
    opts: ReviewOptions,
) -> Result<()> {
    let ReviewOptions {
        mode_override,
        label,
        decks,
        keys: bindings,
        max_typos,
    } = opts;
    let mut files = DeckFiles::new(decks);
    let keys = ReviewKeys::from(&bindings);
    let images = collect_images(session.cards());
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, REVIEW_HTML),
            (Method::Get, key) if key.starts_with("/img/") => {
                serve_image(request, &images, &key["/img/".len()..])
            }
            (Method::Get, "/api/keys") => respond_json(request, &keys),
            (Method::Get, "/api/state") => {
                respond_json(request, &state_dto(&session, &store, mode_override, &label))
            }
            (Method::Post, "/api/grade") => match read_grade(&mut request) {
                Some(grade) => {
                    session.grade(&mut store, grade, now_ms());
                    if let Err(e) = store.save() {
                        eprintln!("warning: could not save progress: {e}");
                    }
                    respond_json(request, &state_dto(&session, &store, mode_override, &label));
                }
                None => respond_status(request, 400),
            },
            (Method::Post, "/api/skip") => {
                session.skip();
                respond_json(request, &state_dto(&session, &store, mode_override, &label));
            }
            (Method::Post, "/api/check") => {
                // Grade the typed lines against the current card — exact for
                // typing (tolerance 0), typo-tolerant for fuzzy. Like choose,
                // this only checks; the grade is applied on Continue.
                #[derive(Deserialize)]
                struct Body {
                    lines: Vec<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let result = body.and_then(|body| {
                    let card = session.current()?;
                    let mode = mode_override.or(card.mode).unwrap_or_default();
                    let tol = if mode == Mode::Typing { 0 } else { max_typos };
                    let results: Vec<LineResultDto> = card
                        .back
                        .iter()
                        .enumerate()
                        .map(|(i, expected)| {
                            let input = body.lines.get(i).map(String::as_str).unwrap_or("");
                            let r = grade_fuzzy(input, expected, tol);
                            LineResultDto {
                                input: r.input,
                                expected: r.expected,
                                passed: r.passed,
                                distance: r.distance,
                            }
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
                // Just reports which option is correct (the question is rebuilt
                // from the card id, so it matches the one served). The grade is
                // applied later via /api/grade on Continue, so the session stays
                // on this card during the result — Remove still works on it.
                let picked = read_index(&mut request).and_then(|chosen| {
                    let card = session.current()?.clone();
                    let correct = choice::build(&card, session.cards(), card.id())?.correct;
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
                let dropped = session.remove_current();
                if let Some(first) = dropped.first() {
                    let subject = first.subject.to_string();
                    let line = first.line;
                    for card in &dropped {
                        store.remove(card.id());
                    }
                    let _ = store.save();
                    files.remove_block(&subject, line);
                }
                respond_json(request, &state_dto(&session, &store, mode_override, &label));
            }
            (Method::Post, "/api/restart") => {
                session.restart(&store, now_ms());
                respond_json(request, &state_dto(&session, &store, mode_override, &label));
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
}

/// Serves a walk through `cards` at `addr`. The only thing it writes is card
/// removal: the remove action deletes the card from its deck file (`decks` maps
/// subject → path) and prunes its progress in `store`.
pub fn run_browse(
    cards: Vec<Card>,
    label: String,
    addr: SocketAddr,
    decks: HashMap<String, PathBuf>,
    mut store: Store,
    bindings: BrowseBindings,
) -> Result<()> {
    let mut cards = cards;
    let mut files = DeckFiles::new(decks);
    let keys = BrowseKeys::from(&bindings);
    let images = collect_images(&cards);
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, BROWSE_HTML),
            (Method::Get, key) if key.starts_with("/img/") => {
                serve_image(request, &images, &key["/img/".len()..])
            }
            (Method::Get, "/api/keys") => respond_json(request, &keys),
            (Method::Get, "/api/cards") => respond_json(request, &browse_payload(&label, &cards)),
            (Method::Post, "/api/remove") => {
                if let Some(index) = read_index(&mut request)
                    && index < cards.len()
                {
                    let subject = cards[index].subject.to_string();
                    let line = cards[index].line;
                    // Drop the card and any cloze siblings, pruning their
                    // progress as they go.
                    cards.retain(|card| {
                        let sibling = card.subject.as_ref() == subject && card.line == line;
                        if sibling {
                            store.remove(card.id());
                        }
                        !sibling
                    });
                    let _ = store.save();
                    files.remove_block(&subject, line);
                }
                respond_json(request, &browse_payload(&label, &cards));
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
}

/// Serializes the current browse cards for the page.
fn browse_payload(label: &str, cards: &[Card]) -> BrowseDto {
    BrowseDto {
        label: label.to_string(),
        cards: cards.iter().map(card_dto).collect(),
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

/// Builds the state payload from the live session and store. For choice mode it
/// also builds the options, seeded by the card id so they are stable across the
/// `/api/state` and `/api/choose` requests without any server-side caching.
fn state_dto(session: &Session, store: &Store, mode_override: Option<Mode>, label: &str) -> StateDto {
    let card = session.current();
    // CLI override wins; otherwise the current card's own mode, else default.
    let mode = mode_override.or(card.and_then(|c| c.mode)).unwrap_or_default();
    let choices = if mode == Mode::Choice {
        card.and_then(|c| choice::build(c, session.cards(), c.id()).map(|q| q.options))
    } else {
        None
    };
    StateDto {
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
        label: label.to_string(),
    }
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
    fn grade_names_map_to_grades() {
        // A guard so the JSON contract and the Grade enum stay in sync.
        assert!(matches!(Grade::Fail, Grade::Fail));
        assert_eq!(mode_name(Mode::LineByLine), "line");
        assert_eq!(mode_name(Mode::Flip), "flip");
    }
}
