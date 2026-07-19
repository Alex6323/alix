mod catalog;
mod dto;
mod jobs;
mod respond;

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

use anyhow::{Result, anyhow};
use catalog::*;
use dto::*;
use jobs::*;
use respond::*;
use serde::Deserialize;
use tiny_http::{Method, Server};

pub use crate::assemble::SelectOptions;
use crate::{
    assemble::{self, CardsBuild, SessionBuild},
    augment::{self, AugmentCache},
    cache::DeckCache,
    config::{
        AiConfig, Audience, Bindings, BrowseBindings, ExamConfig, GenerateDeckConfig, PickerKeys,
    },
    deck::{self, Deck},
    doctor, exam, generate, import,
    recent::RecentDecks,
    review,
    session::now_ms,
    share,
    store::{self, Store},
    trace::{self, Walk},
};

const REVIEW_HTML: &str = include_str!("../../assets/web/review.html");
const KIDS_HTML: &str = include_str!("../../assets/web/kids/kids.html");
const THEME_CSS: &str = include_str!("../../assets/web/theme.css");
const THEME_JS: &str = include_str!("../../assets/web/theme.js");
const ALIX_LOGO_JS: &str = include_str!("../../assets/web/alix-logo.js");
const HEAD_HTML: &str = include_str!("../../assets/web/_head.html");
const BRAND_HTML: &str = include_str!("../../assets/web/_brand.html");

const MAX_REMOTE_BODY: usize = 256 * 1024;

const PLEX_SANS_400: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-sans-400.woff2");
const PLEX_SANS_500: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-sans-500.woff2");
const PLEX_SANS_600: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-sans-600.woff2");
const PLEX_SANS_700: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-sans-700.woff2");
const PLEX_MONO_400: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-mono-400.woff2");
const PLEX_MONO_500: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-mono-500.woff2");
const PLEX_MONO_600: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-mono-600.woff2");
const PLEX_MONO_700: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-mono-700.woff2");

const BALOO2_400: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-400.woff2");
const BALOO2_500: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-500.woff2");
const BALOO2_600: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-600.woff2");
const BALOO2_700: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-700.woff2");
const BALOO2_800: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-800.woff2");

fn font_bytes(name: &str) -> Option<&'static [u8]> {
    match name {
        "ibm-plex-sans-400.woff2" => Some(PLEX_SANS_400),
        "ibm-plex-sans-500.woff2" => Some(PLEX_SANS_500),
        "ibm-plex-sans-600.woff2" => Some(PLEX_SANS_600),
        "ibm-plex-sans-700.woff2" => Some(PLEX_SANS_700),
        "ibm-plex-mono-400.woff2" => Some(PLEX_MONO_400),
        "ibm-plex-mono-500.woff2" => Some(PLEX_MONO_500),
        "ibm-plex-mono-600.woff2" => Some(PLEX_MONO_600),
        "ibm-plex-mono-700.woff2" => Some(PLEX_MONO_700),
        "baloo2-400.woff2" => Some(BALOO2_400),
        "baloo2-500.woff2" => Some(BALOO2_500),
        "baloo2-600.woff2" => Some(BALOO2_600),
        "baloo2-700.woff2" => Some(BALOO2_700),
        "baloo2-800.woff2" => Some(BALOO2_800),
        _ => None,
    }
}

static REVIEW_PAGE: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| compose_page(REVIEW_HTML));

static KIDS_PAGE: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| compose_page(KIDS_HTML));

fn compose_page(html: &str) -> String {
    html.replace("<!--%head%-->", HEAD_HTML)
        .replace("<!--%brand%-->", BRAND_HTML)
}

fn app_page(audience: Audience) -> &'static str {
    match audience {
        Audience::Adult => &REVIEW_PAGE,
        Audience::Kids => &KIDS_PAGE,
    }
}

pub struct ReviewOptions {
    pub keys: Bindings,
    pub picker: PickerKeys,
    pub browse: BrowseBindings,
    pub exam: ExamConfig,
    pub ai: AiConfig,
    pub generate: GenerateDeckConfig,
    pub audience: Audience,
    pub auth: Option<String>,
    pub config_path: Option<PathBuf>,
    pub pair: PairInfo,
    pub scoped: bool,
    pub cfg: assemble::AssembleConfig,
}

pub struct PairInfo {
    pub url: String,
    pub lan: bool,
}

pub fn bind(addr: SocketAddr) -> Result<Server> {
    Server::http(addr).map_err(|e| {
        anyhow!(
            "cannot start the server on {addr}: {e} — is another alix using this port? try --port"
        )
    })
}

// Connection workers pull parsed requests off tiny_http's queue in parallel,
// so an idle kept-alive socket can't starve the rest.
const WORKERS: usize = 16;

// One lock guards the whole struct: workers receive connections in parallel,
// but each handler runs while holding the lock, so handlers never interleave.
struct ServeState {
    store: Store,
    store_dirty: bool,
    recent: RecentDecks,
    decks_dir: PathBuf,
    cache: DeckCache,
    reviewing: Option<Reviewing>,
    browsing: Option<Browsing>,
    examining: Option<Examining>,
    augmenting: Option<Augmenting>,
    generating: Option<Generating>,
    sharing: Option<Sharing>,
    receiving: Option<Receiving>,
    walking: Option<Walking>,
    launcher_icons: HashMap<String, PathBuf>,
    // Kept separate from `reviewing`/`examining` so a phone can never see or
    // kill a browser session, and vice versa; nothing under `/api/remote/*`
    // touches `store` (the phone owns its own state).
    remote_ask: Option<RemoteAsk>,
    remote_exam: Option<RemoteExamining>,
    remote_generate: Option<RemoteGenerating>,
}

// Must run before every `*store =` replacement and before any handler opens
// a store fresh from disk for a mutating operation (reset): a deferred dirty
// store that is replaced or shadowed unflushed silently loses the session.
fn flush_store(store: &Store, dirty: &mut bool) {
    if !*dirty {
        return;
    }
    match store.save() {
        Ok(()) => *dirty = false,
        Err(e) => eprintln!("warning: could not save progress: {e}"),
    }
}

pub fn run_review(
    store: Store,
    recent: RecentDecks,
    decks_dir: PathBuf,
    server: Arc<Server>,
    opts: ReviewOptions,
) -> Result<()> {
    let ReviewOptions {
        keys: bindings,
        picker: picker_keys,
        browse: browse_bindings,
        exam: exam_cfg,
        ai: ai_cfg,
        generate: generate_cfg,
        audience,
        auth,
        config_path,
        pair,
        scoped,
        cfg,
    } = opts;
    let ask_cfg = cfg.ask.clone();
    let review_cfg = cfg.review;
    let keys = ReviewKeys::from(&bindings);
    let picker_keys = PickerKeysDto::from(&picker_keys);
    let browse_keys = BrowseKeys::from(&browse_bindings);
    let ask_info = AskInfoDto::from(&ask_cfg);
    let http_log = std::env::var_os("ALIX_HTTP_LOG").is_some();

    let state = Mutex::new(ServeState {
        store,
        store_dirty: false,
        recent,
        decks_dir,
        cache: DeckCache::default(),
        reviewing: None,
        browsing: None,
        examining: None,
        augmenting: None,
        generating: None,
        sharing: None,
        receiving: None,
        walking: None,
        launcher_icons: HashMap::new(),
        remote_ask: None,
        remote_exam: None,
        remote_generate: None,
    });

    thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| loop {
                let mut request = match server.recv() {
                    Ok(r) => r,
                    // tiny_http's `unblock` wakes only one waiter, so relay it
                    // onward; the chain drains every worker on shutdown.
                    Err(_) => {
                        server.unblock();
                        break;
                    }
                };
                let method = request.method().clone();
                let path = request_path(&request);
                if http_log {
                    eprintln!("[http] {method} {path}");
                }
                if !is_authorized(
                    &path,
                    header_value(&request, "Authorization"),
                    query_param(request.url(), "token").as_deref(),
                    auth.as_deref(),
                ) {
                    respond_status(request, 401);
                    continue;
                }
                // Stateless routes are served before the state lock is taken, so a
                // slow locked handler cannot stall the page shell, its assets, or
                // the config-derived key endpoints.
                match (&method, path.as_str()) {
                    (Method::Get, "/") => {
                        respond_html(request, app_page(audience));
                        continue;
                    }
                    (Method::Get, "/theme.css") => {
                        respond_asset(request, THEME_CSS, "text/css; charset=utf-8");
                        continue;
                    }
                    (Method::Get, "/theme.js") => {
                        respond_asset(
                            request,
                            THEME_JS,
                            "application/javascript; charset=utf-8",
                        );
                        continue;
                    }
                    (Method::Get, key) if key.starts_with("/fonts/") => {
                        match font_bytes(&key["/fonts/".len()..]) {
                            Some(bytes) => respond_font(request, bytes),
                            None => respond_status(request, 404),
                        }
                        continue;
                    }
                    (Method::Get, "/alix-logo.js") => {
                        respond_asset(
                            request,
                            ALIX_LOGO_JS,
                            "application/javascript; charset=utf-8",
                        );
                        continue;
                    }
                    (Method::Get, "/api/keys") => {
                        respond_json(request, &keys);
                        continue;
                    }
                    (Method::Get, "/api/version") => {
                        respond_json(
                            request,
                            &VersionDto {
                                version: env!("CARGO_PKG_VERSION"),
                            },
                        );
                        continue;
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
                        );
                        continue;
                    }
                    (Method::Get, "/api/browse-keys") => {
                        respond_json(request, &browse_keys);
                        continue;
                    }
                    (Method::Get, "/api/picker-keys") => {
                        respond_json(request, &picker_keys);
                        continue;
                    }
                    (Method::Get, "/api/ask-info") => {
                        respond_json(request, &ask_info);
                        continue;
                    }
                    _ => {}
                }
                let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
                let ServeState {
                    store,
                    store_dirty,
                    recent,
                    decks_dir,
                    cache,
                    reviewing,
                    browsing,
                    examining,
                    augmenting,
                    generating,
                    sharing,
                    receiving,
                    walking,
                    launcher_icons,
                    remote_ask,
                    remote_exam,
                    remote_generate,
                } = &mut *guard;
        match (&method, path.as_str()) {
            (Method::Get, "/api/doctor") => {
                let (cfg, _) = doctor::check_config(config_path.as_deref());
                let rows = vec![
                    cfg,
                    doctor::check_store(Some(store.path().to_path_buf())),
                    doctor::check_decks(decks_dir),
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
            (Method::Get, "/api/decks") => {
                let catalog = decks_list_dto(
                    scoped,
                    config_path.as_deref(),
                    &mut *decks_dir,
                    recent,
                    store,
                    &mut *launcher_icons,
                    review_cfg,
                    &mut *cache,
                );
                respond_json(request, &catalog)
            }
            (Method::Get, key) if key.starts_with("/img/") => {
                let name = &key["/img/".len()..];
                if let Some(r) = &reviewing {
                    serve_image(request, &r.images, name)
                } else if let Some(b) = &browsing {
                    serve_image(request, &b.images, name)
                } else {
                    serve_image(request, launcher_icons, name)
                }
            }
            (Method::Get, "/api/state") => {
                if let Some(b) = &browsing {
                    respond_json(request, &browse_payload(Some(b)))
                } else {
                    if let Some(r) = reviewing.as_mut() {
                        r.session.poll(store, now_ms());
                    }
                    respond_json(request, &review_state(reviewing.as_ref(), store))
                }
            }
            (Method::Post, "/api/select") => {
                match read_selection(&mut request, decks_dir, recent, &mut *cache) {
                    Some(sel) => {
                        let opts = sel.opts;
                        let paths = vec![sel.deck];
                        flush_store(store, store_dirty);
                        if let Err(e) = assemble::store_for(&paths, cfg.instance_store.as_deref())
                            .map(|s| *store = s)
                        {
                            eprintln!("warning: could not open the progress store: {e}");
                            respond_status(request, 400);
                            continue;
                        }
                        let recorded_paths = paths.clone();
                        match assemble::select(paths, &mut *store, &cfg, &opts) {
                            Ok(assemble::Selected::Walk(wb)) => {
                                let w = Walking::new(wb.walk, wb.grade);
                                let dto = walk_dto(&w);
                                *walking = Some(w);
                                *reviewing = None;
                                *examining = None;
                                respond_json(request, &dto);
                            }
                            Ok(assemble::Selected::Review(b)) => {
                                if !b.session.is_finished() {
                                    recent.record(&recorded_paths, now_ms());
                                    let _ = recent.save();
                                }
                                let mut r = Reviewing::new(b);
                                r.open_augment(store.path());
                                r.rotate_variant();
                                *reviewing = Some(r);
                                *walking = None;
                                respond_json(request, &review_state(reviewing.as_ref(), store));
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
            (Method::Post, "/api/browse") => {
                match read_selection(&mut request, decks_dir, recent, &mut *cache) {
                    Some(sel) => {
                        let paths = vec![sel.deck];
                        flush_store(store, store_dirty);
                        if let Err(e) = assemble::store_for(&paths, cfg.instance_store.as_deref())
                            .map(|s| *store = s)
                        {
                            eprintln!("warning: could not open the progress store: {e}");
                            respond_status(request, 400);
                            continue;
                        }
                        let recorded_paths = paths.clone();
                        match assemble::browse(paths) {
                            Ok(b) => {
                                recent.record(&recorded_paths, now_ms());
                                let _ = recent.save();
                                *browsing = Some(Browsing::new(b));
                                *reviewing = None;
                                *walking = None;
                                *examining = None;
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
            (Method::Post, "/api/deck-topology") => {
                let dto = match read_selection(&mut request, decks_dir, recent, &mut *cache) {
                    Some(sel) => {
                        match (
                            Deck::load(&sel.deck),
                            assemble::store_for(
                                std::slice::from_ref(&sel.deck),
                                cfg.instance_store.as_deref(),
                            ),
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
            (Method::Post, "/api/reset") => {
                flush_store(store, store_dirty);
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let Some(body) = serde_json::from_reader::<_, Body>(request.as_reader()).ok()
                else {
                    respond_status(request, 400);
                    continue;
                };
                let paths = match resolve_row(&body.deck, decks_dir, recent, &mut *cache) {
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
                let cleared = assemble::store_for(&paths, cfg.instance_store.as_deref())
                    .and_then(|mut s| crate::library::reset_decks(&mut s, decks.iter()));
                match cleared {
                    Ok(n) => {
                        if let Ok(s) = assemble::store_for(&[], cfg.instance_store.as_deref()) {
                            *store = s;
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
            (Method::Post, "/api/workspace/deadline") => {
                // A missing `date` key is a 400; an explicit JSON `null` is
                // the clear signal (serde's "double option" idiom).
                fn deserialize_some<'de, D>(
                    deserializer: D,
                ) -> Result<Option<Option<String>>, D::Error>
                where
                    D: serde::Deserializer<'de>,
                {
                    Option::<String>::deserialize(deserializer).map(Some)
                }
                #[derive(Deserialize)]
                struct Body {
                    name: String,
                    #[serde(default, deserialize_with = "deserialize_some")]
                    date: Option<Option<String>>,
                }
                let Some(body) = serde_json::from_reader::<_, Body>(request.as_reader()).ok()
                else {
                    respond_status(request, 400);
                    continue;
                };
                let date = match body.date {
                    None => {
                        respond_status(request, 400);
                        continue;
                    }
                    Some(None) => None,
                    Some(Some(s)) => match chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
                        Ok(d) => Some(d),
                        Err(_) => {
                            respond_status(request, 400);
                            continue;
                        }
                    },
                };
                let dir = match resolve_row(&body.name, decks_dir, recent, &mut *cache) {
                    Resolved::Many { dir, .. } if crate::workspace::is_workspace(&dir) => dir,
                    _ => {
                        respond_status(request, 400);
                        continue;
                    }
                };
                if let Err(e) = crate::workspace::set_deadline(&dir, date) {
                    eprintln!("workspace deadline write failed: {e:#}");
                    respond_status(request, 500);
                    continue;
                }
                let catalog = decks_list_dto(
                    scoped,
                    config_path.as_deref(),
                    &mut *decks_dir,
                    recent,
                    store,
                    &mut *launcher_icons,
                    review_cfg,
                    &mut *cache,
                );
                respond_json(request, &catalog);
            }
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
                let Some(dir) = resolve_dest(b.dest.as_deref(), decks_dir, recent, &mut *cache)
                else {
                    respond_status(request, 400);
                    continue;
                };
                let lower_name = b.name.to_ascii_lowercase();
                let text = if lower_name.ends_with(".tsv") {
                    match import::tsv_to_deck(&b.text) {
                        Ok(t) => t,
                        Err(_) => {
                            respond_status(request, 400);
                            continue;
                        }
                    }
                } else if lower_name.ends_with(".md") {
                    b.text
                } else {
                    respond_status(request, 400);
                    continue;
                };
                let place_name = normalize_md_extension(&b.name, &lower_name);
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
                if let Some(g) = generating.as_mut() {
                    g.poll();
                }
                if generating.as_ref().is_some_and(|g| g.outcome.is_none()) {
                    respond_status(request, 409);
                    continue;
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(b) =
                    body.filter(|b| b.url.starts_with("http://") || b.url.starts_with("https://"))
                else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(dest) = resolve_dest(b.dest.as_deref(), decks_dir, recent, &mut *cache)
                else {
                    respond_status(request, 400);
                    continue;
                };
                // Check for a name collision before spawning the (costed)
                // model call, so a collision never throws away paid work.
                let name = generate::deck_name(&b.url);
                let stem = name.strip_suffix(".md").unwrap_or(&name);
                let file = format!("{stem}.md");
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
                *generating = Some(g);
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
                *generating = None;
                respond_status(request, 200);
            }
            (Method::Post, "/api/share") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: Option<String>,
                }
                if let Some(s) = sharing.as_mut() {
                    s.poll();
                }
                if sharing.as_ref().is_some_and(|s| s.outcome.is_none()) {
                    respond_status(request, 409);
                    continue;
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let path = match body.and_then(|b| b.deck) {
                    None => Some(decks_dir.clone()),
                    Some(name) => resolved_path(resolve_row(&name, decks_dir, recent, &mut *cache)),
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
                        *sharing = Some(s);
                        respond_json(request, &dto);
                    }
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
                if let Some(s) = sharing.take() {
                    s.job.cancel();
                }
                respond_status(request, 200);
            }
            (Method::Get, "/api/share/zip") => {
                let name = query_param(request.url(), "deck");
                let path = match &name {
                    None => Some(decks_dir.clone()),
                    Some(n) => resolved_path(resolve_row(n, decks_dir, recent, &mut *cache)),
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
                if let Some(r) = receiving.as_mut() {
                    r.poll();
                }
                if receiving.as_ref().is_some_and(|r| r.outcome.is_none()) {
                    respond_status(request, 409);
                    continue;
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(b) = body else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(dest) = resolve_dest(b.dest.as_deref(), decks_dir, recent, &mut *cache)
                else {
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
                        *receiving = Some(r);
                        respond_json(request, &dto);
                    }
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
                if let Some(r) = receiving.take() {
                    r.job.cancel();
                }
                respond_status(request, 200);
            }
            (Method::Post, "/api/receive/zip") => {
                const MAX_ZIP: usize = 50 * 1024 * 1024;
                if request.body_length().is_some_and(|l| l > MAX_ZIP) {
                    respond_status(request, 400);
                    continue;
                }
                let Some(dest) = resolve_dest(
                    query_param(request.url(), "dest").as_deref(),
                    decks_dir,
                    recent,
                    &mut *cache,
                ) else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(bytes) = read_capped(request.as_reader(), MAX_ZIP) else {
                    respond_status(request, 400);
                    continue;
                };
                // `land_received`'s collision check is check-then-act: safe
                // only because handlers are serialized behind the state lock.
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
                *reviewing = None;
                *walking = None;
                *browsing = None;
                flush_store(store, store_dirty);
                if let Ok(s) = assemble::store_for(&[], cfg.instance_store.as_deref()) {
                    *store = s;
                }
                respond_json(request, &review_state(reviewing.as_ref(), store));
            }
            (Method::Post, "/api/grade") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                match read_grade(&mut request) {
                    Some(grade) => {
                        let now = now_ms();
                        r.session.grade(&mut *store, grade, now);
                        if let Some(subject) = r.files.paths.keys().next() {
                            store::note_badges(&mut *store, subject, r.session.cards(), now);
                        }
                        *store_dirty = true;
                        r.rotate_variant();
                        respond_json(request, &review_state(reviewing.as_ref(), store));
                    }
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/skip") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.skip(store, now_ms());
                r.rotate_variant();
                respond_json(request, &review_state(reviewing.as_ref(), store));
            }
            (Method::Post, "/api/acquire") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.acquire_current(&mut *store, now_ms());
                *store_dirty = true;
                r.rotate_variant();
                respond_json(request, &review_state(reviewing.as_ref(), store));
            }
            (Method::Post, "/api/check") => {
                let Some(r) = reviewing.as_ref() else {
                    respond_status(request, 409);
                    continue;
                };
                #[derive(Deserialize)]
                struct Body {
                    lines: Vec<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let result = body.and_then(|body| review::check_typed(&r.session, &body.lines));
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
                let picked = read_index(&mut request)
                    .and_then(|chosen| review::choose(&r.session, store, &r.augment, chosen));
                match picked {
                    Some(f) => respond_json(request, &f),
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/remove") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let dropped = r.session.remove_current(store, now_ms());
                if let Some(first) = dropped.first() {
                    let subject = first.subject.to_string();
                    let line = first.line;
                    for card in &dropped {
                        if let Some(id) = card.id() {
                            store.remove(&id);
                        }
                    }
                    *store_dirty = true;
                    r.files.remove_block(&subject, line);
                }
                respond_json(request, &review_state(reviewing.as_ref(), store));
            }
            (Method::Post, "/api/promote") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if !r.session.current_is_virtual(store) {
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
                if store::promote_virtual(&mut *store, &id, &path).is_err() {
                    respond_status(request, 400);
                    continue;
                }
                *store_dirty = true;
                r.session.poll(store, now_ms());
                respond_json(request, &review_state(reviewing.as_ref(), store));
            }
            (Method::Post, "/api/restart") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.restart(store, now_ms());
                r.rotate_variant();
                respond_json(request, &review_state(reviewing.as_ref(), store));
            }
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
                    r.start_ask(&ask_cfg, audience, AskAction::Question(q));
                }
                respond_json(request, &r.ask_dto(None, None));
            }
            (Method::Post, "/api/ask/note") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.start_ask(&ask_cfg, audience, AskAction::Condense);
                respond_json(request, &r.ask_dto(None, None));
            }
            (Method::Post, "/api/ask/card/draft") => {
                if audience == Audience::Kids {
                    respond_status(request, 403);
                    continue;
                }
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.start_ask(&ask_cfg, audience, AskAction::DraftCard);
                respond_json(request, &r.ask_dto(None, None));
            }
            (Method::Post, "/api/ask/card/create") => {
                if audience == Audience::Kids {
                    respond_status(request, 403);
                    continue;
                }
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let Some(req) =
                    serde_json::from_reader::<_, CreateCardReq>(request.as_reader()).ok()
                else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(card) = r.session.current() else {
                    respond_status(request, 409);
                    continue;
                };
                let subject = card.subject.to_string();
                // Dedup by content fingerprint, not id: a mint carries a
                // fresh random token, so identical content still collides.
                let deck_fingerprints: std::collections::HashSet<u64> = r
                    .session
                    .cards()
                    .iter()
                    .map(|c| c.content_fingerprint)
                    .collect();
                let now = now_ms();
                match store::mint_tutor_card(
                    &mut *store,
                    &subject,
                    &req.front,
                    &req.back,
                    now,
                    &deck_fingerprints,
                ) {
                    Ok(id) => {
                        *store_dirty = true;
                        respond_json(request, &CreateCardResp { id });
                    }
                    Err(store::MintError::Duplicate | store::MintError::Malformed(_)) => {
                        respond_status(request, 422);
                    }
                    Err(store::MintError::Mint(_)) => {
                        respond_status(request, 500);
                    }
                }
            }
            (Method::Get, "/api/ask") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let (status, error) = r.poll_ask();
                respond_json(request, &r.ask_dto(status, error));
            }
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
                // A bare name duplicated across containers must 400, not
                // guess: this endpoint gates progression on the result.
                let Some(path) =
                    resolved_path(resolve_row(&body.deck, decks_dir, recent, &mut *cache))
                else {
                    respond_status(request, 400);
                    continue;
                };
                flush_store(store, store_dirty);
                if let Ok(s) =
                    assemble::store_for(std::slice::from_ref(&path), cfg.instance_store.as_deref())
                {
                    *store = s;
                }
                match Deck::load(&path) {
                    Ok(deck)
                        if deck.has_exam()
                            && !deck::is_locked(&deck, Some(decks_dir.as_path()), store) =>
                    {
                        let strictness =
                            deck.settings.exam_strictness.unwrap_or(exam_cfg.strictness);
                        let sitting = if deck.is_trace() {
                            match trace::Trace::from_deck(&deck) {
                                Ok(t) => {
                                    if let Some(ms) = exam::cooldown_remaining_ms(
                                        store,
                                        &deck.subject,
                                        exam_cfg.retry_cooldown_secs,
                                        now_ms(),
                                    ) {
                                        // One response shape per endpoint: the
                                        // cooldown is an ExamDto phase, not untagged.
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
                            // Check backend capability before starting, so a
                            // gap is a clean refusal, not a mid-exam poll error.
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
                        let dto = exam_dto(&ex, decks_dir);
                        *examining = Some(ex);
                        respond_json(request, &dto);
                    }
                    _ => respond_status(request, 409),
                }
            }
            (Method::Get, "/api/exam") => {
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let parent = ex.deck_path.parent().unwrap_or_else(|| Path::new(""));
                let retire_after_days = review_cfg.for_workspace(parent).retire_after_days;
                ex.sitting.poll(&mut *store, now_ms(), retire_after_days);
                respond_json(request, &exam_dto(ex, decks_dir));
            }
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
                respond_json(request, &exam_dto(ex, decks_dir));
            }
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
                respond_json(request, &exam_dto(ex, decks_dir));
            }
            (Method::Post, "/api/exam/remediate") => {
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                ex.sitting.remediate();
                respond_json(request, &exam_dto(ex, decks_dir));
            }
            (Method::Post, "/api/exam/close") => {
                *examining = None;
                flush_store(store, store_dirty);
                if let Ok(s) = assemble::store_for(&[], cfg.instance_store.as_deref()) {
                    *store = s;
                }
                respond_json(request, &review_state(reviewing.as_ref(), store));
            }
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
                let (files, workspace_dir) = match resolve_row(&body.deck, decks_dir, recent, &mut *cache)
                {
                    Resolved::One(p) => (vec![p], None),
                    Resolved::Many { dir, files } => (files, Some(dir)),
                    _ => {
                        respond_status(request, 400);
                        continue;
                    }
                };
                let name = body.deck;
                flush_store(store, store_dirty);
                if let Ok(s) = assemble::store_for(&files, cfg.instance_store.as_deref()) {
                    *store = s;
                }
                // Stamp before loading: unstamped ids collapse the cache to
                // key 0, orphaning the spend at the first real stamp.
                match assemble::stamp_and_load_cards(&files) {
                    Ok(cards) => {
                        let deck_tokens: Vec<String> = files
                            .iter()
                            .filter_map(|p| {
                                crate::deck::Deck::load(p).ok().and_then(|d| d.deck_token)
                            })
                            .collect();
                        let aug = Augmenting::open(
                            name,
                            cards,
                            deck_tokens,
                            augment::augment_path_for(store.path()),
                            workspace_dir,
                        );
                        let dto = aug.dto();
                        *augmenting = Some(aug);
                        respond_json(request, &dto);
                    }
                    Err(_) => respond_status(request, 409),
                }
            }
            (Method::Post, "/api/augment/generate") => {
                #[derive(Deserialize)]
                struct TargetBody {
                    target: String,
                    with: Option<String>,
                }
                #[derive(Deserialize)]
                struct Body {
                    targets: Vec<TargetBody>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(aug) = augmenting.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(b) = body {
                    let targets = b
                        .targets
                        .into_iter()
                        .map(|t| {
                            let guidance = t
                                .with
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty());
                            (t.target, guidance)
                        })
                        .collect();
                    aug.generate_batch(targets, &ai_cfg, &ask_cfg);
                }
                respond_json(request, &aug.dto());
            }
            (Method::Get, "/api/augment") => {
                let Some(aug) = augmenting.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                aug.poll(&ai_cfg, &ask_cfg);
                respond_json(request, &aug.dto());
            }
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
            (Method::Post, "/api/augment/close") => {
                *augmenting = None;
                flush_store(store, store_dirty);
                if let Ok(s) = assemble::store_for(&[], cfg.instance_store.as_deref()) {
                    *store = s;
                }
                respond_json(request, &review_state(reviewing.as_ref(), store));
            }
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
                        w.walk.grade(&mut *store, delta, now_ms());
                        *store_dirty = true;
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
                    w.start_ask(&ask_cfg, audience, Some(q));
                }
                respond_json(request, &w.ask_dto(None, None));
            }
            (Method::Post, "/api/walk/ask/note") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                w.start_ask(&ask_cfg, audience, None);
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
            (Method::Post, "/api/walk/leave") => {
                *walking = None;
                flush_store(store, store_dirty);
                if let Ok(s) = assemble::store_for(&[], cfg.instance_store.as_deref()) {
                    *store = s;
                }
                respond_json(request, &review_state(reviewing.as_ref(), store));
            }
            (Method::Post, "/api/remote/ask") => {
                if let Some(a) = remote_ask.as_mut() {
                    a.poll();
                }
                if remote_ask.as_ref().is_some_and(RemoteAsk::thinking) {
                    respond_status(request, 409);
                    continue;
                }
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(RemoteAskReq {
                    card,
                    history,
                    question,
                }) = serde_json::from_slice::<RemoteAskReq>(&bytes)
                else {
                    respond_status(request, 400);
                    continue;
                };
                if question.trim().is_empty()
                    || (card.front.trim().is_empty()
                        && card.back.iter().all(|l| l.trim().is_empty()))
                {
                    respond_status(request, 400);
                    continue;
                }
                let job = RemoteAsk::ask(&ask_cfg, &card, history, &question);
                let dto = job.dto();
                *remote_ask = Some(job);
                respond_json(request, &dto);
            }
            (Method::Get, "/api/remote/ask") => {
                let dto = match remote_ask.as_mut() {
                    Some(a) => {
                        a.poll();
                        a.dto()
                    }
                    None => RemoteAskDto {
                        thinking: false,
                        answer: None,
                        draft: None,
                        note: None,
                        error: None,
                        elapsed: None,
                    },
                };
                respond_json(request, &dto);
            }
            (Method::Post, "/api/remote/ask/draft") => {
                if audience == Audience::Kids {
                    respond_status(request, 403);
                    continue;
                }
                if let Some(a) = remote_ask.as_mut() {
                    a.poll();
                }
                if remote_ask.as_ref().is_some_and(RemoteAsk::thinking) {
                    respond_status(request, 409);
                    continue;
                }
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(RemoteDraftReq { card, history }) =
                    serde_json::from_slice::<RemoteDraftReq>(&bytes)
                else {
                    respond_status(request, 400);
                    continue;
                };
                if history.is_empty() {
                    respond_status(request, 400);
                    continue;
                }
                let job = RemoteAsk::draft(&ask_cfg, &card, history);
                let dto = job.dto();
                *remote_ask = Some(job);
                respond_json(request, &dto);
            }
            (Method::Post, "/api/remote/ask/note") => {
                if let Some(a) = remote_ask.as_mut() {
                    a.poll();
                }
                if remote_ask.as_ref().is_some_and(RemoteAsk::thinking) {
                    respond_status(request, 409);
                    continue;
                }
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(RemoteNoteReq { card, history }) =
                    serde_json::from_slice::<RemoteNoteReq>(&bytes)
                else {
                    respond_status(request, 400);
                    continue;
                };
                if history.is_empty() {
                    respond_status(request, 400);
                    continue;
                }
                let job = RemoteAsk::note(&ask_cfg, &card, history);
                let dto = job.dto();
                *remote_ask = Some(job);
                respond_json(request, &dto);
            }
            // The requires-lock and the trace re-sit cooldown are the
            // browser's own truth; both are deliberately skipped here.
            (Method::Post, "/api/remote/exam/start") => {
                if remote_exam.is_some() {
                    respond_status(request, 409);
                    continue;
                }
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(body) = serde_json::from_slice::<Body>(&bytes) else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(path) =
                    resolved_path(resolve_row(&body.deck, decks_dir, recent, &mut *cache))
                else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(deck) = Deck::load(&path) else {
                    respond_status(request, 409);
                    continue;
                };
                if !deck.is_trace() && deck.sources.is_empty() {
                    respond_status(request, 409);
                    continue;
                }
                let strictness = deck.settings.exam_strictness.unwrap_or(exam_cfg.strictness);
                let sitting = if deck.is_trace() {
                    match trace::Trace::from_deck(&deck) {
                        Ok(t) => exam::Sitting::start_trace(
                            t.description.clone(),
                            t.compression_rubric(),
                            deck.subject.clone(),
                            strictness,
                            exam_cfg.clone(),
                            ask_cfg.clone(),
                        ),
                        Err(_) => {
                            respond_status(request, 409);
                            continue;
                        }
                    }
                } else {
                    if exam::ensure_backend_can_examine(&deck, &ask_cfg).is_err() {
                        respond_status(request, 409);
                        continue;
                    }
                    exam::Sitting::start(&deck, strictness, exam_cfg.clone(), ask_cfg.clone())
                };
                let ex = RemoteExamining {
                    sitting,
                    cards: None,
                };
                let dto = ex.dto();
                *remote_exam = Some(ex);
                respond_json(request, &dto);
            }
            // advance() only, never poll(): poll() writes the store, which
            // remote handlers must never touch.
            (Method::Get, "/api/remote/exam") => {
                let dto = match remote_exam.as_mut() {
                    Some(ex) => {
                        ex.advance();
                        ex.dto()
                    }
                    None => remote_exam_idle_dto(),
                };
                respond_json(request, &dto);
            }
            (Method::Post, "/api/remote/exam/grade") => {
                #[derive(Deserialize)]
                struct Body {
                    answers: Vec<String>,
                }
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(body) = serde_json::from_slice::<Body>(&bytes) else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(ex) = remote_exam.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if !matches!(ex.sitting.phase(), exam::Phase::Answering) {
                    respond_status(request, 409);
                    continue;
                }
                let got = body.answers.len();
                if !ex.sitting.set_answers(body.answers) {
                    eprintln!(
                        "remote exam grade: expected {} answers, got {got}",
                        ex.sitting.total()
                    );
                    respond_status(request, 400);
                    continue;
                }
                ex.sitting.submit();
                respond_json(request, &ex.dto());
            }
            (Method::Post, "/api/remote/exam/remediate") => {
                let Some(ex) = remote_exam.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if !ex.sitting.can_remediate() {
                    respond_status(request, 409);
                    continue;
                }
                ex.sitting.remediate();
                respond_json(request, &ex.dto());
            }
            // Drop the slot; an in-flight thread just finds its receiver
            // gone and its send fails harmlessly.
            (Method::Post, "/api/remote/exam/close") => {
                *remote_exam = None;
                respond_status(request, 200);
            }
            // No dest, no destination-collision check: this returns the
            // deck text, it never places a file (both are the phone's job).
            (Method::Post, "/api/remote/generate") => {
                if let Some(g) = remote_generate.as_mut() {
                    g.poll();
                }
                if remote_generate
                    .as_ref()
                    .is_some_and(RemoteGenerating::thinking)
                {
                    respond_status(request, 409);
                    continue;
                }
                #[derive(Deserialize)]
                struct Body {
                    url: String,
                    guidance: Option<String>,
                }
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(body) = serde_json::from_slice::<Body>(&bytes) else {
                    respond_status(request, 400);
                    continue;
                };
                if !(body.url.starts_with("http://") || body.url.starts_with("https://")) {
                    respond_status(request, 400);
                    continue;
                }
                let mut cfg = generate_cfg.clone();
                if let Some(g) = body
                    .guidance
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    cfg.extra = Some(g);
                }
                let job = RemoteGenerating::start(body.url, cfg, ask_cfg.clone());
                let dto = job.dto();
                *remote_generate = Some(job);
                respond_json(request, &dto);
            }
            (Method::Get, "/api/remote/generate") => {
                let Some(g) = remote_generate.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                g.poll();
                respond_json(request, &g.dto());
            }
            (Method::Post, "/api/remote/generate/close") => {
                *remote_generate = None;
                respond_status(request, 200);
            }
            _ => respond_status(request, 404),
        }
                }
                );
        }
    });
    Ok(())
}

#[expect(clippy::too_many_arguments)] // the listing entry point takes each piece of served state
fn decks_list_dto(
    scoped: bool,
    config_path: Option<&Path>,
    decks_dir: &mut PathBuf,
    recent: &RecentDecks,
    store: &Store,
    launcher_icons: &mut HashMap<String, PathBuf>,
    review_cfg: crate::config::ReviewConfig,
    cache: &mut DeckCache,
) -> DeckListDto {
    let dir = effective_decks_dir(scoped, config_path, decks_dir);
    if dir != *decks_dir {
        *decks_dir = dir;
    }
    deck_catalog(
        decks_dir,
        recent,
        store,
        true,
        launcher_icons,
        review_cfg,
        cache,
    )
}

#[cfg(test)]
mod contract;
#[cfg(test)]
mod tests;
