//! A local web frontend.
//!
//! Bare `alix` starts a small synchronous HTTP server (one request at a
//! time — correct for a single user) that serves an embedded web page and a
//! JSON API — the sole interactive frontend. The [`Session`]/[`Store`] drive
//! review, and cards are sent to the browser as a DTO built from
//! [`render::note_units`], so the note structuring lives in one place. Grades
//! persist to the same progress store the rest of the CLI (`deck`, `trace`,
//! `generate`, …) reads and writes, so studying in the browser and running
//! those commands share one history.
//!
//! It is deliberately local-only: no accounts, no database. By default it
//! binds to `127.0.0.1`; `--lan` binds all interfaces so a phone or tablet on
//! the same network can reach it (there is no authentication, so that is
//! opt-in).

mod catalog;
mod dto;
mod jobs;
mod respond;

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Result, anyhow};
use catalog::*;
use dto::*;
use jobs::*;
use respond::*;
use serde::Deserialize;
use tiny_http::{Method, Server};

use crate::{
    answer::{TypedResult, grade_lines_ordered, grade_lines_unordered},
    augment::{self, AugmentCache},
    card::Card,
    config::{
        AiConfig, AskConfig, Bindings, BrowseBindings, ExamConfig, GenerateDeckConfig, PickerKeys,
        ReviewConfig,
    },
    deck::{self, Deck},
    depth::Depth,
    doctor, exam, generate, import,
    recent::RecentDecks,
    session::{Session, now_ms},
    share,
    store::{self, Store},
    trace::{self, SourceBase, Walk},
};

const REVIEW_HTML: &str = include_str!("../../assets/web/review.html");
const THEME_CSS: &str = include_str!("../../assets/web/theme.css");
const THEME_JS: &str = include_str!("../../assets/web/theme.js");
const ALIX_LOGO_JS: &str = include_str!("../../assets/web/alix-logo.js");
const HEAD_HTML: &str = include_str!("../../assets/web/_head.html");
const BRAND_HTML: &str = include_str!("../../assets/web/_brand.html");

/// The review page with its shared-chrome placeholders filled once, so the head
/// boilerplate (`<!--%head%-->`) and brand mark (`<!--%brand%-->`) live in one place.
static REVIEW_PAGE: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| compose_page(REVIEW_HTML));

/// Fill the shared-chrome placeholders in a served page.
fn compose_page(html: &str) -> String {
    html.replace("<!--%head%-->", HEAD_HTML)
        .replace("<!--%brand%-->", BRAND_HTML)
}

/// Global options for a served review, independent of which decks are chosen
/// (the per-session label and deck paths come from [`SessionBuild`]).
pub struct ReviewOptions {
    pub keys: Bindings,
    /// Deck-picker navigation keys (the `[picker]` section), bound on the
    /// selection screen.
    pub picker: PickerKeys,
    /// Browse-mode keys (the `[browse]` section), bound on the `/browse` page
    /// this server also hosts.
    pub browse: BrowseBindings,
    /// Ask-Claude settings (command, allowlist, timeout, …).
    pub ask: AskConfig,
    /// AI-exam settings (model, question count, default strictness, …).
    pub exam: ExamConfig,
    /// AI augmentation settings (model, per-target counts), for generating
    /// augmentations from the picker's Augment screen.
    pub ai: AiConfig,
    /// AI deck-generation settings (model, timeout, max cards, guidance, …),
    /// for `POST /api/generate`.
    pub generate: GenerateDeckConfig,
    /// Personal review pacing (FSRS retention + retirement interval), for the
    /// selection screen's badges and due counts.
    pub review: ReviewConfig,
    /// Pairing token required on `/api/*` when set (auto-generated for `--lan`);
    /// `None` leaves the server open (the localhost default).
    pub auth: Option<String>,
    /// The `--config` path the launcher loaded config from (`None` → the
    /// platform default), passed through so `/api/doctor` checks the same file.
    pub config_path: Option<PathBuf>,
    /// How this instance is reached, for `/api/pair`'s pairing sheet.
    pub pair: PairInfo,
    /// `true` for a scoped `alix <dir>` launch — its decks dir is pinned to
    /// that folder forever. `false` for the config-derived launch (bare
    /// `alix`), whose `/api/decks` re-resolves the configured dir on every
    /// fetch so a `decks_dir` edit takes effect without a restart.
    pub scoped: bool,
}

/// How this instance is reached, for the pairing sheet. Built by the
/// launcher, which is the only place that knows bind + token + LAN IP.
pub struct PairInfo {
    pub url: String,
    pub lan: bool,
}

/// A review session ready to serve: the session, its header label, the
/// subject → deck file path map used for card removal, and the subject → deck
/// reference links (`% link:`) offered to ask-Claude. Produced by the caller's
/// builder closure when decks are chosen (on the CLI or in the browser picker).
pub struct SessionBuild {
    pub session: Session,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
    pub links: HashMap<String, Vec<String>>,
    /// Subject → its deck's `% source:` project root, for the grounded ask-tutor
    /// (`[ask] source_access`). Only decks with a local source appear.
    pub source_roots: HashMap<String, PathBuf>,
    /// Subject → its deck's source base, for resolving a card's `% at:` citation
    /// excerpt on reveal.
    pub source_bases: HashMap<String, SourceBase>,
    /// The resolved topology name when this session is topology-ordered, so the
    /// server can show the connective cue from that topology. `None` otherwise.
    pub topology_name: Option<String>,
}

/// A trace walk ready to serve, built when a single trace deck is picked from the
/// review server's deck-selection screen. The walk is self-graded (no live
/// `--grade`), matching the terminal picker's trace → walk.
pub struct WalkBuild {
    pub walk: Walk,
    /// AI-grades each prediction when set (`[trace] auto_grade` + the ask
    /// config); `None` = self-graded.
    pub grade: Option<AskConfig>,
}

/// A browse card list ready to serve, with its label and deck paths.
pub struct CardsBuild {
    pub cards: Vec<Card>,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
}

/// Serves review on the already-bound `server` until the process is stopped
/// (binding happens at the call site, *before* the URL is announced — so a
/// port clash errors before any success-looking output), opening on the
/// in-browser deck-selection screen; picking decks (`POST /api/select`)
/// calls `build` to construct a session in place.
/// `build` borrows the shared `store` and `recent`, so all sessions write one
/// history and update the recent-decks list, exactly like the CLI.
/// Binds the server socket — separated from [`run_review`] so a port clash
/// errors before the caller announces a URL, and with the multi-instance
/// remedy in the message.
pub fn bind(addr: SocketAddr) -> Result<Server> {
    Server::http(addr).map_err(|e| {
        anyhow!(
            "cannot start the server on {addr}: {e} — is another alix using this port? try --port"
        )
    })
}

#[expect(clippy::too_many_arguments)] // each is a distinct, named server input
pub fn run_review(
    mut store: Store,
    mut recent: RecentDecks,
    mut decks_dir: PathBuf,
    server: Server,
    opts: ReviewOptions,
    mut build: impl FnMut(
        Vec<PathBuf>,
        &SelectOptions,
        &Store,
        &mut RecentDecks,
    ) -> Result<SessionBuild>,
    // Builds a walk when the picked decks are a single trace (else `None`, so the
    // caller flattens to a review); mirrors the terminal picker's trace → walk.
    mut build_walk: impl FnMut(&[PathBuf]) -> Result<Option<WalkBuild>>,
    // Builds a read-only browse card list from the picked decks (the picker's
    // "Browse" action; the page navigates to `/browse`, which this server hosts).
    mut build_browse: impl FnMut(Vec<PathBuf>, &mut RecentDecks) -> Result<CardsBuild>,
    // The progress store the given decks write to — a workspace's own
    // `progress.json` when they share one, else the global store (`&[]` → global),
    // mirroring the terminal `store_for`. The active store is swapped to this when
    // a session launches and reset to the global one back at the picker.
    mut store_for: impl FnMut(&[PathBuf]) -> Result<Store>,
) -> Result<()> {
    let ReviewOptions {
        keys: bindings,
        picker: picker_keys,
        browse: browse_bindings,
        ask: ask_cfg,
        exam: exam_cfg,
        ai: ai_cfg,
        generate: generate_cfg,
        review: review_cfg,
        auth,
        config_path,
        pair,
        scoped,
    } = opts;
    let keys = ReviewKeys::from(&bindings);
    let picker_keys = PickerKeysDto::from(&picker_keys);
    // The `/browse` page this server also hosts needs its own next/prev/remove
    // keys, distinct from the review grade keys served at `/api/keys`.
    let browse_keys = BrowseKeys::from(&browse_bindings);
    let ask_info = AskInfoDto::from(&ask_cfg);
    // The server always opens on the picker; review/browse states are entered
    // from it (`/api/select`, `/api/browse`) — browse is a native mode of the
    // review server, not a separate page.
    let (mut reviewing, mut browsing): (Option<Reviewing>, Option<Browsing>) = (None, None);
    let mut examining: Option<Examining> = None;
    // The picker's "Augment" action opens a deck's augmentation screen here.
    let mut augmenting: Option<Augmenting> = None;
    // The add-sheet's "Generate from URL" action; one deck generation at a time.
    let mut generating: Option<Generating> = None;
    // The picker's "Share" action; one wormhole send in flight at a time. An
    // abandoned/replaced job always drops through `Sharing`/`ShareJob` (never
    // leaked), so its child process is cancelled even without a close call.
    let mut sharing: Option<Sharing> = None;
    // The picker's "Receive" action; one wormhole receive in flight at a time.
    // Same drop-cancels invariant as `sharing` — an abandoned/replaced job
    // always drops through `ShareJob`, cancelling its wormhole child.
    let mut receiving: Option<Receiving> = None;
    // A trace picked from the selection screen walks in-page inside review.html
    // (no navigation to a separate `/walk` page — the walk is an in-page mode).
    let mut walking: Option<Walking> = None;
    // `browsing` (seeded above for a `--serve` browse launch) is also entered
    // from the picker's "Browse" action (POST /api/browse) — in-page, no page nav.
    // Workspace icons resolved while building the picker, served via `/img/` at
    // launcher time (when no review/browse session owns the registry).
    let mut launcher_icons: HashMap<String, PathBuf> = HashMap::new();
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        if !is_authorized(
            &path,
            header_value(&request, "Authorization"),
            query_param(request.url(), "token").as_deref(),
            auth.as_deref(),
        ) {
            respond_status(request, 401);
            continue;
        }
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, &REVIEW_PAGE),
            (Method::Get, "/theme.css") => {
                respond_asset(request, THEME_CSS, "text/css; charset=utf-8")
            }
            (Method::Get, "/theme.js") => {
                respond_asset(request, THEME_JS, "application/javascript; charset=utf-8")
            }
            (Method::Get, "/alix-logo.js") => respond_asset(
                request,
                ALIX_LOGO_JS,
                "application/javascript; charset=utf-8",
            ),
            (Method::Get, "/api/keys") => respond_json(request, &keys),
            (Method::Get, "/api/version") => respond_json(
                request,
                &VersionDto {
                    version: env!("CARGO_PKG_VERSION"),
                },
            ),
            (Method::Get, "/api/doctor") => {
                let (cfg, _) = doctor::check_config(config_path.as_deref());
                let rows = vec![
                    cfg,
                    doctor::check_store(Some(store.path().to_path_buf())),
                    doctor::check_decks(&decks_dir),
                    // Mirrors `main.rs::doctor_cmd`'s binary lines verbatim
                    // (names, purposes, remedies) — the web report must match
                    // the CLI's, minus the costed `--backends` probe.
                    doctor::check_binary(
                        "backend",
                        &ask_cfg.command,
                        "the AI features (tutor, exam, generate)",
                        "install it and log in — or switch `[ask] backend` in the config",
                    ),
                    doctor::check_binary(
                        "share",
                        "wormhole",
                        "sharing (`alix share`/`receive`)",
                        "install magic-wormhole (e.g. `pipx install magic-wormhole`, or your package manager)",
                    ),
                ]
                .into_iter()
                .map(DoctorRowDto::from)
                .collect();
                respond_json(request, &DoctorDto { rows })
            }
            (Method::Get, "/api/pair") => {
                let svg = if pair.lan {
                    crate::qr::svg(&pair.url)
                } else {
                    None
                };
                respond_json(
                    request,
                    &PairDto {
                        url: pair.url.clone(),
                        svg,
                        lan: pair.lan,
                    },
                )
            }
            (Method::Get, "/api/browse-keys") => respond_json(request, &browse_keys),
            (Method::Get, "/api/picker-keys") => respond_json(request, &picker_keys),
            (Method::Get, "/api/ask-info") => respond_json(request, &ask_info),
            (Method::Get, "/api/decks") => {
                // Unscoped instances re-resolve the configured decks dir on
                // every fetch, so an edited `decks_dir` takes effect on the
                // next reload/focus without a restart (`{#page-reload-refetches-decks}`).
                let dir = effective_decks_dir(scoped, config_path.as_deref(), &decks_dir);
                if dir != decks_dir {
                    decks_dir = dir;
                }
                // Review enforces locking; the picker won't start a locked deck.
                let catalog = deck_catalog(
                    &decks_dir,
                    &recent,
                    &store,
                    true,
                    &mut launcher_icons,
                    review_cfg,
                );
                respond_json(request, &catalog)
            }
            // Image cards: served from whichever session is live (review or browse).
            (Method::Get, key) if key.starts_with("/img/") => {
                let name = &key["/img/".len()..];
                if let Some(r) = &reviewing {
                    serve_image(request, &r.images, name)
                } else if let Some(b) = &browsing {
                    serve_image(request, &b.images, name)
                } else {
                    serve_image(request, &launcher_icons, name)
                }
            }
            (Method::Get, "/api/state") => {
                // Browse is an in-page mode: when a browse list is live (a
                // `--serve` browse launch), the page gets the browse payload here
                // and opens the browse overlay instead of a review session.
                if let Some(b) = &browsing {
                    respond_json(request, &browse_payload(Some(b)))
                } else {
                    // A missed card may have cooled back into due-ness since the
                    // last fetch; re-check so it re-enters review on this poll
                    // (stats preserved), no manual restart needed.
                    if let Some(r) = reviewing.as_mut() {
                        r.session.poll(&store, now_ms());
                    }
                    respond_json(request, &review_state(reviewing.as_ref(), &store))
                }
            }
            (Method::Post, "/api/select") => {
                match read_selection(&mut request, &decks_dir, &recent) {
                    Some(sel) => {
                        let opts = sel.opts;
                        let paths = vec![sel.deck];
                        // Write to the deck's own store — a workspace's `progress.json`
                        // when they share one, else the global store — the same store
                        // the picker's badges are read from.
                        if let Err(e) = store_for(&paths).map(|s| store = s) {
                            eprintln!("warning: could not open the progress store: {e}");
                            respond_status(request, 400);
                            continue;
                        }
                        match build_walk(&paths) {
                            Ok(Some(wb)) => {
                                let w = Walking::new(wb.walk, wb.grade);
                                let dto = walk_dto(&w);
                                walking = Some(w);
                                reviewing = None;
                                examining = None;
                                respond_json(request, &dto);
                            }
                            Ok(None) => match build(paths, &opts, &store, &mut recent) {
                                Ok(b) => {
                                    // Remember the resolved depth for this deck so a
                                    // plain Learn next time reopens at it (keyed by
                                    // deck subject, like the rest of the deck store).
                                    let resolved = b.session.depth();
                                    let subject = b.decks.keys().next().cloned();
                                    let mut r = Reviewing::new(b);
                                    r.open_augment(store.path());
                                    r.rotate_variant();
                                    if let Some(subject) = subject {
                                        store.set_last_depth(&subject, resolved);
                                        if let Err(e) = store.save() {
                                            eprintln!("warning: could not save progress: {e}");
                                        }
                                    }
                                    reviewing = Some(r);
                                    walking = None;
                                    respond_json(
                                        request,
                                        &review_state(reviewing.as_ref(), &store),
                                    );
                                }
                                Err(e) => {
                                    eprintln!("warning: could not load the selected decks: {e}");
                                    respond_status(request, 400);
                                }
                            },
                            Err(e) => {
                                eprintln!("warning: could not load the selected trace: {e}");
                                respond_status(request, 400);
                            }
                        }
                    }
                    None => respond_status(request, 400),
                }
            }
            // The picker's "Browse" action: build a read-only card list and return
            // it, so the page opens the browse overlay in place (no page nav).
            (Method::Post, "/api/browse") => {
                match read_selection(&mut request, &decks_dir, &recent) {
                    Some(sel) => {
                        let paths = vec![sel.deck];
                        if let Err(e) = store_for(&paths).map(|s| store = s) {
                            eprintln!("warning: could not open the progress store: {e}");
                            respond_status(request, 400);
                            continue;
                        }
                        match build_browse(paths, &mut recent) {
                            Ok(b) => {
                                browsing = Some(Browsing::new(b));
                                reviewing = None;
                                walking = None;
                                examining = None;
                                respond_json(request, &browse_payload(browsing.as_ref()));
                            }
                            Err(e) => {
                                eprintln!("warning: could not load the selected decks: {e}");
                                respond_status(request, 400);
                            }
                        }
                    }
                    None => respond_status(request, 400),
                }
            }
            // The focus drawer asks for a deck's stored topologies + region
            // heatmaps when it's selected. Read-only: open the deck's own store
            // transiently, never disturbing the active session store.
            (Method::Post, "/api/deck-topology") => {
                let dto = match read_selection(&mut request, &decks_dir, &recent) {
                    Some(sel) => {
                        match (
                            Deck::load(&sel.deck),
                            store_for(std::slice::from_ref(&sel.deck)),
                        ) {
                            (Ok(deck), Ok(s)) => {
                                let augment =
                                    AugmentCache::open(augment::augment_path_for(s.path()));
                                deck_topology_dto(&augment, &s, &deck, review_cfg)
                            }
                            _ => DeckTopologyDto::default(),
                        }
                    }
                    None => DeckTopologyDto::default(),
                };
                respond_json(request, &dto);
            }
            // Wipe a row's review progress (the sheet's typed-name gate is
            // client UX; a token holder is trusted — same class as grading).
            (Method::Post, "/api/reset") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let Some(body) = serde_json::from_reader::<_, Body>(request.as_reader()).ok()
                else {
                    respond_status(request, 400);
                    continue;
                };
                // Rows resolve to their deck files: a workspace/folder row to
                // its members, a deck row to itself.
                let paths = match resolve_row(&body.deck, &decks_dir, &recent) {
                    Resolved::One(p) => vec![p],
                    Resolved::Many { files, .. } => files,
                    Resolved::Ambiguous | Resolved::Unknown => {
                        respond_status(request, 400);
                        continue;
                    }
                };
                let name = body.deck;
                let decks: Vec<Deck> = match paths.iter().map(Deck::load).collect() {
                    Ok(d) => d,
                    Err(_) => {
                        respond_status(request, 400);
                        continue;
                    }
                };
                let cleared = store_for(&paths)
                    .and_then(|mut s| crate::library::reset_decks(&mut s, decks.iter()));
                match cleared {
                    Ok(n) => {
                        // The in-memory global store may now be stale — reload.
                        if let Ok(s) = store_for(&[]) {
                            store = s;
                        }
                        respond_json(
                            request,
                            &ResetDto {
                                deck: name,
                                cards_cleared: n,
                            },
                        );
                    }
                    Err(_) => respond_status(request, 400),
                }
            }
            // Land an uploaded `.tsv`/`.txt` file via `place_deck`. Strict
            // unlike `generate`'s lenient save: an invalid upload is 400 and
            // no file remains — the upload still exists on the user's
            // device, so nothing is lost by refusing to keep a broken copy.
            (Method::Post, "/api/import") => {
                #[derive(Deserialize)]
                struct Body {
                    name: String,
                    text: String,
                    dest: Option<String>,
                }
                let Some(b) = serde_json::from_reader::<_, Body>(request.as_reader()).ok() else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(dir) = resolve_dest(b.dest.as_deref(), &decks_dir, &recent) else {
                    respond_status(request, 400);
                    continue;
                };
                // `.tsv` converts (Anki export); `.txt` is a deck as-is. Case
                // folded so `FILE.TSV` matches — the browser's file picker
                // accept filter offers upper-case extensions too.
                let lower_name = b.name.to_ascii_lowercase();
                let text = if lower_name.ends_with(".tsv") {
                    match import::tsv_to_deck(&b.text) {
                        Ok(t) => t,
                        Err(_) => {
                            respond_status(request, 400);
                            continue;
                        }
                    }
                } else if lower_name.ends_with(".txt") {
                    b.text
                } else {
                    respond_status(request, 400);
                    continue;
                };
                let place_name = normalize_txt_extension(&b.name, &lower_name);
                match crate::library::place_deck(&dir, &place_name, &text) {
                    Ok(p) if p.parse_error.is_none() => {
                        let deck = p
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        respond_json(
                            request,
                            &ImportDto {
                                deck,
                                cards: p.cards,
                            },
                        );
                    }
                    // Uploads are strict: don't keep an invalid deck around.
                    Ok(p) => {
                        std::fs::remove_file(&p.path).ok();
                        respond_status(request, 400);
                    }
                    Err(_) => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/generate") => {
                #[derive(Deserialize)]
                struct Body {
                    url: String,
                    guidance: Option<String>,
                    dest: Option<String>,
                }
                // A worker may have finished while nobody polled (the page
                // went away) — drain it first, so "finished" means finished
                // even without a GET, and only a live worker 409s.
                if let Some(g) = generating.as_mut() {
                    g.poll();
                }
                if generating.as_ref().is_some_and(|g| g.outcome.is_none()) {
                    respond_status(request, 409); // one costed job at a time
                    continue;
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(b) =
                    body.filter(|b| b.url.starts_with("http://") || b.url.starts_with("https://"))
                else {
                    respond_status(request, 400); // the web generates from URLs only
                    continue;
                };
                let Some(dest) = resolve_dest(b.dest.as_deref(), &decks_dir, &recent) else {
                    respond_status(request, 400);
                    continue;
                };
                // A collision discovered only after the (costed) model call
                // would throw away paid work for nothing — check before
                // spawning, mirroring `library::place_deck`'s stem/extension
                // logic (stage-then-merge: fail fast on what's already
                // knowable, same principle as the CLI's destination guard).
                let name = generate::deck_name(&b.url);
                let stem = name.strip_suffix(".txt").unwrap_or(&name);
                let file = format!("{stem}.txt");
                if dest.join(&file).exists() {
                    respond_json(
                        request,
                        &GenerateDto {
                            phase: "error",
                            deck: None,
                            cards: None,
                            elapsed: Some(0),
                            error: Some(format!(
                                "{file} already exists — rename it or generate into another destination"
                            )),
                        },
                    );
                    continue;
                }
                let mut cfg = generate_cfg.clone();
                if let Some(g) = b
                    .guidance
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    cfg.extra = Some(g);
                }
                let g = Generating {
                    rx: generate::spawn(b.url.clone(), cfg, ask_cfg.clone()),
                    url: b.url,
                    dest,
                    started: Instant::now(),
                    outcome: None,
                };
                let dto = g.dto();
                generating = Some(g);
                respond_json(request, &dto);
            }
            (Method::Get, "/api/generate") => {
                let Some(g) = generating.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                g.poll();
                respond_json(request, &g.dto());
            }
            (Method::Post, "/api/generate/close") => {
                generating = None; // a running worker finishes and is discarded
                respond_status(request, 200);
            }
            (Method::Post, "/api/share") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: Option<String>,
                }
                // Drain a finished-but-unpolled job first, so a completed send is
                // replaced by the next POST even without an intervening GET —
                // mirroring the `/api/generate` fix (only a *live* job 409s).
                if let Some(s) = sharing.as_mut() {
                    s.poll();
                }
                if sharing.as_ref().is_some_and(|s| s.outcome.is_none()) {
                    respond_status(request, 409); // one share at a time
                    continue;
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let path = match body.and_then(|b| b.deck) {
                    None => Some(decks_dir.clone()),
                    Some(name) => resolved_path(resolve_row(&name, &decks_dir, &recent)),
                };
                let Some(path) = path else {
                    respond_status(request, 400);
                    continue;
                };
                let started = tempfile::tempdir()
                    .map_err(|e| anyhow!("{e}"))
                    .and_then(|tmp| {
                        let to_send = stage_for_share(&path, &tmp)?;
                        let job = share::send_spawn(&to_send)?;
                        Ok(Sharing {
                            job,
                            _stage: tmp,
                            code: None,
                            started: Instant::now(),
                            outcome: None,
                        })
                    });
                match started {
                    Ok(s) => {
                        let dto = s.dto();
                        sharing = Some(s);
                        respond_json(request, &dto);
                    }
                    // Spawn failures (missing binary) surface as an error-phase
                    // job so the sheet shows the install hint, not a bare 400.
                    Err(e) => respond_json(
                        request,
                        &ShareDto {
                            phase: "error",
                            code: None,
                            elapsed: Some(0),
                            error: Some(format!("{e:#}")),
                        },
                    ),
                }
            }
            (Method::Get, "/api/share") => {
                let Some(s) = sharing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                s.poll();
                respond_json(request, &s.dto());
            }
            (Method::Post, "/api/share/close") => {
                // Dropping the (former) job cancels its wormhole child — see
                // `ShareJob`'s `Drop`; `cancel()` here is just for clarity.
                if let Some(s) = sharing.take() {
                    s.job.cancel();
                }
                respond_status(request, 200);
            }
            (Method::Get, "/api/share/zip") => {
                // `request_path` (used for dispatch) already strips the query
                // string, so the plain literal above matches regardless of
                // `?deck=...` — read the param back off the full URL here.
                let name = query_param(request.url(), "deck");
                let path = match &name {
                    None => Some(decks_dir.clone()),
                    Some(n) => resolved_path(resolve_row(n, &decks_dir, &recent)),
                };
                let Some(path) = path else {
                    respond_status(request, 400);
                    continue;
                };
                let zipped = tempfile::tempdir().ok().and_then(|tmp| {
                    let staged = stage_for_share(&path, &tmp).ok()?;
                    let out = tmp.path().join("share.zip");
                    share::zip_to(&staged, &out).ok()?;
                    std::fs::read(&out).ok()
                });
                match zipped {
                    Some(bytes) => {
                        let stem = name
                            .as_deref()
                            .map(|n| n.rsplit('/').next().unwrap_or(n))
                            .unwrap_or("shared-decks");
                        respond_download(request, bytes, "application/zip", &format!("{stem}.zip"));
                    }
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/receive") => {
                #[derive(Deserialize)]
                struct Body {
                    code: String,
                    dest: Option<String>,
                }
                // Drain a finished-but-unpolled job first — same fix as
                // generate/share: only a *live* job 409s.
                if let Some(r) = receiving.as_mut() {
                    r.poll();
                }
                if receiving.as_ref().is_some_and(|r| r.outcome.is_none()) {
                    respond_status(request, 409); // one receive at a time
                    continue;
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(b) = body else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(dest) = resolve_dest(b.dest.as_deref(), &decks_dir, &recent) else {
                    respond_status(request, 400);
                    continue;
                };
                let started = tempfile::tempdir()
                    .map_err(|e| anyhow!("{e}"))
                    .and_then(|tmp| {
                        let job = share::receive_spawn(&b.code, tmp.path())?;
                        Ok(Receiving {
                            job,
                            tmp,
                            dest,
                            started: Instant::now(),
                            outcome: None,
                        })
                    });
                match started {
                    Ok(r) => {
                        let dto = r.dto();
                        receiving = Some(r);
                        respond_json(request, &dto);
                    }
                    // Spawn failures (missing binary) surface as an error-phase
                    // job so the sheet shows the install hint, not a bare 400.
                    Err(e) => respond_json(
                        request,
                        &ReceiveDto {
                            phase: "error",
                            landed: None,
                            stripped: Vec::new(),
                            elapsed: Some(0),
                            error: Some(format!("{e:#}")),
                        },
                    ),
                }
            }
            (Method::Get, "/api/receive") => {
                let Some(r) = receiving.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.poll();
                respond_json(request, &r.dto());
            }
            (Method::Post, "/api/receive/close") => {
                // Dropping the (former) job cancels its wormhole child — see
                // `ShareJob`'s `Drop`; `cancel()` here is just for clarity.
                if let Some(r) = receiving.take() {
                    r.job.cancel();
                }
                respond_status(request, 200);
            }
            (Method::Post, "/api/receive/zip") => {
                // `request_path` (used for dispatch) already strips the query
                // string (Task 10 confirmed), so the plain literal above
                // matches regardless of `?dest=...` — read the param back off
                // the full URL here, same as `/api/share/zip`.
                const MAX_ZIP: usize = 50 * 1024 * 1024;
                if request.body_length().is_some_and(|l| l > MAX_ZIP) {
                    respond_status(request, 400);
                    continue;
                }
                let Some(dest) = resolve_dest(
                    query_param(request.url(), "dest").as_deref(),
                    &decks_dir,
                    &recent,
                ) else {
                    respond_status(request, 400);
                    continue;
                };
                // `body_length` can lie or be absent, so `read_capped` also
                // bounds the actual read, not just the declared length.
                let Some(bytes) = read_capped(request.as_reader(), MAX_ZIP) else {
                    respond_status(request, 400);
                    continue;
                };
                // `land_received`'s collision check is check-then-act; safe
                // only because this server loop is single-threaded — do not
                // introduce threads here (see `Receiving::poll`'s note).
                let landed = tempfile::tempdir().ok().and_then(|tmp| {
                    let zip_path = tmp.path().join("got.zip");
                    std::fs::write(&zip_path, &bytes).ok()?;
                    let scratch = tmp.path().join("out");
                    std::fs::create_dir_all(&scratch).ok()?;
                    share::unzip_to(&zip_path, &scratch).ok()?;
                    share::land_received(&scratch, &dest).ok()
                });
                match landed {
                    Some((landed, stripped)) => respond_json(
                        request,
                        &ReceiveDto {
                            phase: "done",
                            landed: Some(landed),
                            stripped,
                            elapsed: Some(0),
                            error: None,
                        },
                    ),
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/deselect") => {
                reviewing = None;
                walking = None;
                browsing = None;
                // Back at the picker: read the global store again (loose-deck
                // badges live there, not in any workspace's store).
                if let Ok(s) = store_for(&[]) {
                    store = s;
                }
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            (Method::Post, "/api/grade") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                match read_grade(&mut request) {
                    Some(grade) => {
                        let now = now_ms();
                        r.session.grade(&mut store, grade, now);
                        // Refresh the deck's per-depth badge earn dates from this
                        // session's cards (high-water first-earn marks; badges gate
                        // nothing). Keyed by deck subject, like the rest of the
                        // deck-level store (exam mastery, last depth).
                        if let Some(subject) = r.files.paths.keys().next() {
                            store::note_badges(&mut store, subject, r.session.cards(), now);
                        }
                        if let Err(e) = store.save() {
                            eprintln!("warning: could not save progress: {e}");
                        }
                        r.rotate_variant(); // a fresh phrasing for the next card
                        respond_json(request, &review_state(reviewing.as_ref(), &store));
                    }
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/skip") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.skip(&store, now_ms());
                r.rotate_variant(); // a fresh phrasing for the next card
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            (Method::Post, "/api/acquire") => {
                // Acknowledge a never-seen card: record it as acquired (no grade)
                // and move on. Its first quiz comes back ~1 min later, this session.
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.acquire_current(&mut store, now_ms());
                if let Err(e) = store.save() {
                    eprintln!("warning: could not save progress: {e}");
                }
                r.rotate_variant(); // a fresh phrasing for the next card
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            (Method::Post, "/api/check") => {
                let Some(r) = reviewing.as_ref() else {
                    respond_status(request, 409);
                    continue;
                };
                // Grade the typed lines against the current card: normalized then
                // compared exactly, no edit-distance tolerance. Pure evidence —
                // like choose, this only checks; the learner-final grade is applied
                // separately on Continue via `/api/grade`. `ordered` (TypeLine, the
                // `% reveal: line` reconstruct path) pairs line-by-position; the
                // default matches each input to its closest expected line so a
                // multi-item answer can be entered in any order.
                #[derive(Deserialize)]
                struct Body {
                    lines: Vec<String>,
                    #[serde(default)]
                    ordered: bool,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let result = body.and_then(|body| {
                    let card = r.session.current()?;
                    let results: Vec<TypedResult> = if body.ordered {
                        grade_lines_ordered(&body.lines, &card.back)
                    } else {
                        grade_lines_unordered(&body.lines, &card.back)
                    };
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
                // Just reports which option is correct (the question is rebuilt via
                // `current_question`, seeded from the card id and its appearance
                // count, so it matches the one served by `review_state` for every
                // question shape and appearance). The grade is applied
                // later via /api/grade on Continue, so the session stays on this card
                // during the result — Remove still works on it.
                let picked = read_index(&mut request).and_then(|chosen| {
                    let card = r.session.current()?.clone();
                    let correct = current_question(r, &store, &card)?.correct;
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
                let dropped = r.session.remove_current(&store, now_ms());
                if let Some(first) = dropped.first() {
                    let subject = first.subject.to_string();
                    let line = first.line;
                    for card in &dropped {
                        store.remove(card.id());
                    }
                    let _ = store.save();
                    r.files.remove_block(&subject, line);
                }
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            // Promotes the current virtual (remediation) card into its deck
            // file (`store::promote_virtual` does the append-then-drop; the
            // schedule needs no transfer, since it already lives in
            // `store.cards` under the id the appended deck card hashes to, so
            // the promoted card keeps its earned schedule for free). A clean
            // 400 — never a panic — when the current card isn't virtual or
            // its deck file isn't known.
            (Method::Post, "/api/promote") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if !r.session.current_is_virtual(&store) {
                    respond_status(request, 400);
                    continue;
                }
                let Some(id) = r.session.current_id() else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(subject) = r.session.current().map(|c| c.subject.to_string()) else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(path) = r.files.paths.get(&subject).cloned() else {
                    respond_status(request, 400);
                    continue;
                };
                if store::promote_virtual(&mut store, id, &path).is_err() {
                    respond_status(request, 400);
                    continue;
                }
                r.session.poll(&store, now_ms());
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            (Method::Post, "/api/restart") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.restart(&store, now_ms());
                r.rotate_variant(); // a fresh phrasing for the new session's first card
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            // Ask Claude about the current card — runs the CLI on a background
            // thread (ask::spawn) and returns immediately; the page polls
            // `GET /api/ask` for the answer so the server loop never blocks.
            (Method::Post, "/api/ask") => {
                #[derive(Deserialize)]
                struct Body {
                    question: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let question = body.map(|b| b.question).filter(|q| !q.trim().is_empty());
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(q) = question {
                    r.start_ask(&ask_cfg, Some(q));
                }
                respond_json(request, &r.ask_dto(None, None));
            }
            // Condense the conversation into note lines appended to the deck.
            (Method::Post, "/api/ask/note") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.start_ask(&ask_cfg, None);
                respond_json(request, &r.ask_dto(None, None));
            }
            // Poll for a pending reply; the page calls this every ~400ms while
            // `thinking`.
            (Method::Get, "/api/ask") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let (status, error) = r.poll_ask();
                respond_json(request, &r.ask_dto(status, error));
            }
            // ── AI exam ───────────────────────────────────────────────────
            // Start an exam for one `% source:` deck: validate the name and
            // drill state, then spawn question generation on a background
            // thread; the page polls `GET /api/exam`.
            (Method::Post, "/api/exam/start") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let Some(body) = serde_json::from_reader::<_, Body>(request.as_reader()).ok()
                else {
                    respond_status(request, 400);
                    continue;
                };
                // Include workspace members (by their qualified `<ws>/<file>`
                // name) so an exam can be started on a deck inside a workspace,
                // not just a top-level deck — mirroring `/api/select`. Resolved
                // through the shared catalog lookup, so a bare name duplicated
                // across containers is rejected here too, instead of silently
                // writing mastery to whichever container's row won a last-wins
                // map (this endpoint gates progression, so ambiguity must 400,
                // not guess).
                let Some(path) = resolved_path(resolve_row(&body.deck, &decks_dir, &recent)) else {
                    respond_status(request, 400);
                    continue;
                };
                // The exam reads drill state and writes mastery/unlocks to the
                // deck's own store (a workspace's, or the global one).
                if let Ok(s) = store_for(std::slice::from_ref(&path)) {
                    store = s;
                }
                match Deck::load(&path) {
                    // Examable when it has an exam (a `% source:` fact deck, or a
                    // trace) and its `% requires:` are satisfied — drilled or not
                    // (you may test out early).
                    Ok(deck)
                        if deck.has_exam()
                            && !deck::is_locked(&deck, Some(decks_dir.as_path()), &store) =>
                    {
                        let strictness =
                            deck.settings.exam_strictness.unwrap_or(exam_cfg.strictness);
                        // A trace's exam is the graded compression (one fixed
                        // question), gated by the re-sit cooldown after a fail; a
                        // fact deck's exam generates questions from its source.
                        let sitting = if deck.is_trace() {
                            match trace::Trace::from_deck(&deck) {
                                Ok(t) => {
                                    if let Some(ms) = exam::cooldown_remaining_ms(
                                        &store,
                                        &deck.subject,
                                        exam_cfg.retry_cooldown_secs,
                                        now_ms(),
                                    ) {
                                        // One shape per endpoint: the cooldown is an
                                        // ExamDto in its own phase, not an untagged
                                        // {cooldown_ms} the client must key-sniff.
                                        respond_json(request, &cooldown_dto(&deck.subject, ms));
                                        continue;
                                    }
                                    exam::Sitting::start_trace(
                                        t.description.clone(),
                                        t.compression_rubric(),
                                        deck.subject.clone(),
                                        strictness,
                                        exam_cfg.clone(),
                                        ask_cfg.clone(),
                                    )
                                }
                                Err(_) => {
                                    respond_status(request, 409);
                                    continue;
                                }
                            }
                        } else {
                            // Fact-deck pre-flight: confirm the configured
                            // backend can reach every `% source:` before
                            // starting the sitting, so a capability gap is a
                            // clean refusal at launch, not an error surfaced
                            // mid-exam through the background job's poll.
                            if exam::ensure_backend_can_examine(&deck, &ask_cfg).is_err() {
                                respond_status(request, 409);
                                continue;
                            }
                            exam::Sitting::start(
                                &deck,
                                strictness,
                                exam_cfg.clone(),
                                ask_cfg.clone(),
                            )
                        };
                        let ex = Examining {
                            sitting,
                            deck_path: path,
                        };
                        let dto = exam_dto(&ex, &decks_dir);
                        examining = Some(ex);
                        respond_json(request, &dto);
                    }
                    _ => respond_status(request, 409),
                }
            }
            // Poll the exam: advance any finished background call, return state.
            (Method::Get, "/api/exam") => {
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                // A workspace member's exam remediation honors its own
                // `alix.local.toml` retirement cap, same as its review session.
                let parent = ex.deck_path.parent().unwrap_or_else(|| Path::new(""));
                let retire_after_days = review_cfg.for_workspace(parent).retire_after_days;
                ex.sitting.poll(&mut store, now_ms(), retire_after_days);
                respond_json(request, &exam_dto(ex, &decks_dir));
            }
            // Save the current answer and (optionally) navigate to another question.
            (Method::Post, "/api/exam/answer") => {
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                    goto: Option<usize>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(b) = body {
                    ex.sitting.set_answer(b.text);
                    if let Some(i) = b.goto {
                        ex.sitting.goto(i);
                    }
                }
                respond_json(request, &exam_dto(ex, &decks_dir));
            }
            // Save the last answer and submit everything for grading.
            (Method::Post, "/api/exam/grade") => {
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(b) = body {
                    ex.sitting.set_answer(b.text);
                }
                ex.sitting.submit();
                respond_json(request, &exam_dto(ex, &decks_dir));
            }
            // On a fail, generate remediation cards into the store as virtual cards.
            (Method::Post, "/api/exam/remediate") => {
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                ex.sitting.remediate();
                respond_json(request, &exam_dto(ex, &decks_dir));
            }
            // Leave the exam, back to the deck list / summary.
            (Method::Post, "/api/exam/close") => {
                examining = None;
                if let Ok(s) = store_for(&[]) {
                    store = s;
                }
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            // ── Deck augmentation (the picker's "Augment" action, decks only) ──
            // Open a deck's Augment screen and report what its cache holds. Resolves
            // the deck through the catalog (incl. workspace members) like the exam.
            (Method::Post, "/api/augment/open") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let Some(body) = serde_json::from_reader::<_, Body>(request.as_reader()).ok()
                else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(path) = resolved_path(resolve_row(&body.deck, &decks_dir, &recent)) else {
                    respond_status(request, 400);
                    continue;
                };
                let name = body.deck;
                // The cache lives beside the deck's own store (a workspace's, or the
                // global one), mirroring how review reads it.
                if let Ok(s) = store_for(std::slice::from_ref(&path)) {
                    store = s;
                }
                match Deck::load(&path) {
                    Ok(deck) => {
                        let aug = Augmenting::open(
                            name,
                            deck.cards,
                            augment::augment_path_for(store.path()),
                        );
                        let dto = aug.dto();
                        augmenting = Some(aug);
                        respond_json(request, &dto);
                    }
                    Err(_) => respond_status(request, 409),
                }
            }
            // Start fill-the-gaps generation for one target (a costed background
            // call); the page polls `GET /api/augment`.
            (Method::Post, "/api/augment/generate") => {
                #[derive(Deserialize)]
                struct Body {
                    target: String,
                    with: Option<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(aug) = augmenting.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(b) = body {
                    let guidance = b
                        .with
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    aug.generate(&b.target, guidance, &ai_cfg, &ask_cfg);
                }
                respond_json(request, &aug.dto());
            }
            // Poll the in-flight generation: apply a finished outcome, return state.
            (Method::Get, "/api/augment") => {
                let Some(aug) = augmenting.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                aug.poll();
                respond_json(request, &aug.dto());
            }
            // Remove a target's augmentations (or `all`) for this deck.
            (Method::Post, "/api/augment/remove") => {
                #[derive(Deserialize)]
                struct Body {
                    target: String,
                    topology: Option<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(aug) = augmenting.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(b) = body {
                    aug.remove(&b.target, b.topology.as_deref());
                }
                respond_json(request, &aug.dto());
            }
            // Leave the Augment screen, back to the picker (reset to the global store).
            (Method::Post, "/api/augment/close") => {
                augmenting = None;
                if let Ok(s) = store_for(&[]) {
                    store = s;
                }
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            // ── Trace walk (a single trace picked from the selection screen) ──
            // The web trace-walk flow (predict → reveal → grade), guarded on `walking`.
            (Method::Get, "/api/walk") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                w.poll();
                respond_json(request, &walk_dto(w));
            }
            (Method::Post, "/api/walk/predict") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                if let Some(b) = body {
                    w.walk.predict(b.text);
                    w.start_grade();
                }
                respond_json(request, &walk_dto(w));
            }
            (Method::Post, "/api/walk/grade") => {
                let self_delta = read_delta(&mut request);
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let delta = w.grade_result.as_ref().map(|(d, _)| *d).or(self_delta);
                match delta {
                    Some(delta) => {
                        w.walk.grade(&mut store, delta, now_ms());
                        if let Err(e) = store.save() {
                            eprintln!("warning: could not save progress: {e}");
                        }
                        w.clear_grade();
                        respond_json(request, &walk_dto(w));
                    }
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/walk/restart") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let fresh = Walk::new(w.walk.trace().clone());
                let grade = w.grade.take();
                *w = Walking::new(fresh, grade);
                respond_json(request, &walk_dto(w));
            }
            // Ask-Claude about the current checkpoint — the same tutor a review
            // uses (its subject is the checkpoint). Runs on a background thread;
            // the page polls `GET /api/walk/ask`.
            (Method::Post, "/api/walk/ask") => {
                #[derive(Deserialize)]
                struct Body {
                    question: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let question = body.map(|b| b.question).filter(|q| !q.trim().is_empty());
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(q) = question {
                    w.start_ask(&ask_cfg, Some(q));
                }
                respond_json(request, &w.ask_dto(None, None));
            }
            // Condense the conversation into a `!` note on the checkpoint.
            (Method::Post, "/api/walk/ask/note") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                w.start_ask(&ask_cfg, None);
                respond_json(request, &w.ask_dto(None, None));
            }
            (Method::Get, "/api/walk/ask") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let (status, error) = w.poll_ask();
                respond_json(request, &w.ask_dto(status, error));
            }
            // Back to decks: abandon the walk and return to the picker (global store).
            (Method::Post, "/api/walk/leave") => {
                walking = None;
                if let Ok(s) = store_for(&[]) {
                    store = s;
                }
                // Every closer returns the picker StateDto — one teardown rule.
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
}

/// The per-launch choices a selection carries beyond which deck: the picker's
/// depth pick, focus-drawer topology/region scope, the cram tick-box, and
/// optional pacing overrides (absent → the instance's CLI/config values).
#[derive(Default)]
pub struct SelectOptions {
    pub topology: Option<String>,
    pub region: Option<String>,
    pub depth: Option<Depth>,
    pub cram: bool,
    pub max_new: Option<usize>,
    pub limit: Option<usize>,
}

#[cfg(test)]
mod contract;
#[cfg(test)]
mod tests;
