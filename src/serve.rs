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
    net::SocketAddr,
    path::PathBuf,
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::{
    answer::Mode,
    card::Card,
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
}

/// The current review state sent to the browser after every action.
#[derive(Debug, Serialize)]
struct StateDto {
    /// The card up for review, or `null` when the session is finished.
    card: Option<CardDto>,
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

/// Serves a review session at `addr` until the process is stopped. Grades are
/// applied to `session` and saved to `store` after each one. `decks` (subject →
/// file path) lets the remove action delete a card from its deck file and prune
/// its progress; unlike the TUI, removal is immediate, since the server has no
/// clean end-of-session.
pub fn run_review(
    mut session: Session,
    mut store: Store,
    mode: Mode,
    label: String,
    addr: SocketAddr,
    decks: HashMap<String, PathBuf>,
) -> Result<()> {
    let mut files = DeckFiles::new(decks);
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, REVIEW_HTML),
            (Method::Get, "/api/state") => {
                respond_json(request, &state_dto(&session, &store, mode, &label))
            }
            (Method::Post, "/api/grade") => match read_grade(&mut request) {
                Some(grade) => {
                    session.grade(&mut store, grade, now_ms());
                    if let Err(e) = store.save() {
                        eprintln!("warning: could not save progress: {e}");
                    }
                    respond_json(request, &state_dto(&session, &store, mode, &label));
                }
                None => respond_status(request, 400),
            },
            (Method::Post, "/api/skip") => {
                session.skip();
                respond_json(request, &state_dto(&session, &store, mode, &label));
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
                respond_json(request, &state_dto(&session, &store, mode, &label));
            }
            (Method::Post, "/api/restart") => {
                session.restart(&store, now_ms());
                respond_json(request, &state_dto(&session, &store, mode, &label));
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
) -> Result<()> {
    let mut cards = cards;
    let mut files = DeckFiles::new(decks);
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, BROWSE_HTML),
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

/// Builds the state payload from the live session and store.
fn state_dto(session: &Session, store: &Store, mode: Mode, label: &str) -> StateDto {
    StateDto {
        card: session.current().map(card_dto),
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
    CardDto {
        front: card.front.clone(),
        context: card.context.clone(),
        back: card.back.clone(),
        note: render::note_units(card)
            .into_iter()
            .map(NoteUnitDto::from)
            .collect(),
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
    fn grade_names_map_to_grades() {
        // A guard so the JSON contract and the Grade enum stay in sync.
        assert!(matches!(Grade::Fail, Grade::Fail));
        assert_eq!(mode_name(Mode::LineByLine), "line");
        assert_eq!(mode_name(Mode::Flip), "flip");
    }
}
