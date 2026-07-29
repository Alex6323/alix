mod catalog;
mod dto;
mod jobs;
mod respond;
mod study;

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
use study::*;
use serde::Deserialize;
use tiny_http::{Method, Server};

pub use crate::assemble::SelectOptions;
use crate::{
    assemble::{self, CardsBuild, SessionBuild},
    cache::DeckCache,
    config::{
        AiConfig, Audience, Bindings, BrowseBindings, ExamConfig, GenerateDeckConfig, PickerKeys,
    },
    deck::Deck,
    doctor, exam, generate, import,
    recent::RecentDecks,
    session::now_ms,
    share,
    store::Store,
    trace::{self},
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

// The catalog-slice fields (root, cache, icons, recent) and the job slots
// that still live behind this lock. The active session and every progress
// document live in the Study owner (`study.rs`); the Catalog and Jobs
// owners take these remaining fields next, and then this struct is deleted.
struct ServeState {
    recent: RecentDecks,
    decks_dir: PathBuf,
    cache: DeckCache,
    launcher_icons: HashMap<String, PathBuf>,
    generating: Option<Generating>,
    sharing: Option<Sharing>,
    receiving: Option<Receiving>,
    // Kept separate from the Study owner's session so a phone can never see
    // or kill a browser session, and vice versa; nothing under
    // `/api/remote/*` touches the progress store (the phone owns its state).
    remote_ask: Option<RemoteAsk>,
    remote_exam: Option<RemoteExamining>,
    remote_generate: Option<RemoteGenerating>,
}

fn lock(state: &Mutex<ServeState>) -> std::sync::MutexGuard<'_, ServeState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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

    let (study, study_thread) = study::spawn(StudyState {
        config: StudyConfig {
            cfg,
            exam_cfg: exam_cfg.clone(),
            review_cfg,
            audience,
        },
        store,
        store_dirty: false,
        save_error: None,
        reviewing: None,
        browsing: None,
        examining: None,
        walking: None,
        augmenting: None,
    });

    let state = Mutex::new(ServeState {
        recent,
        decks_dir,
        cache: DeckCache::default(),
        launcher_icons: HashMap::new(),
        generating: None,
        sharing: None,
        receiving: None,
        remote_ask: None,
        remote_exam: None,
        remote_generate: None,
    });

    thread::scope(|scope| {
        for _ in 0..WORKERS {
            let study = study.clone();
            let state = &state;
            let server = &server;
            let keys = &keys;
            let picker_keys = &picker_keys;
            let browse_keys = &browse_keys;
            let ask_info = &ask_info;
            let ask_cfg = &ask_cfg;
            let ai_cfg = &ai_cfg;
            let generate_cfg = &generate_cfg;
            let exam_cfg = &exam_cfg;
            let pair = &pair;
            let auth = &auth;
            let config_path = &config_path;
            scope.spawn(move || loop {
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
                // Stateless routes are served first, so a slow stateful
                // handler cannot stall the page shell, its assets, or the
                // config-derived key endpoints.
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
                        respond_json(request, keys);
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
                        respond_json(request, browse_keys);
                        continue;
                    }
                    (Method::Get, "/api/picker-keys") => {
                        respond_json(request, picker_keys);
                        continue;
                    }
                    (Method::Get, "/api/ask-info") => {
                        respond_json(request, ask_info);
                        continue;
                    }
                    // The store path comes from the owner and the decks root
                    // from a brief residual lock; the version-probe
                    // subprocesses run without holding either, so a parked
                    // probe cannot stall stateful requests.
                    (Method::Get, "/api/doctor") => {
                        let Some(store_path) = study.store_path() else {
                            respond_status(request, 503);
                            continue;
                        };
                        let decks_root = lock(state).decks_dir.clone();
                        let (cfg, _) = doctor::check_config(config_path.as_deref());
                        let rows = vec![
                            cfg,
                            doctor::check_store(Some(store_path)),
                            doctor::check_decks(&decks_root),
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
                        respond_json(request, &DoctorDto { rows });
                        continue;
                    }
                    _ => {}
                }
                match (&method, path.as_str()) {
            (Method::Get, "/api/decks") => {
                let Some(projection) = study.projection() else {
                    respond_status(request, 503);
                    continue;
                };
                let mut guard = lock(state);
                let ServeState {
                    recent,
                    decks_dir,
                    cache,
                    launcher_icons,
                    ..
                } = &mut *guard;
                match decks_list_dto(
                    scoped,
                    config_path.as_deref(),
                    decks_dir,
                    recent,
                    &projection,
                    launcher_icons,
                    review_cfg,
                    cache,
                ) {
                    Ok(catalog) => respond_json(request, &catalog),
                    Err(e) => {
                        eprintln!("deck listing failed for {}: {e}", decks_dir.display());
                        respond_status(request, 500);
                    }
                }
            }
            (Method::Get, key) if key.starts_with("/img/") => {
                let name = &key["/img/".len()..];
                match study.image_path(name.to_string()) {
                    Some(ImageSource::Active(path)) => {
                        serve_image_path(request, path.as_deref())
                    }
                    Some(ImageSource::NoActive) => {
                        let guard = lock(state);
                        serve_image(request, &guard.launcher_icons, name)
                    }
                    None => respond_status(request, 503),
                }
            }
            (Method::Get, "/api/state") => match study.state() {
                Some(SessionSnapshot::Browse(dto)) => respond_json(request, &dto),
                Some(SessionSnapshot::Review(dto)) => respond_json(request, &dto),
                None => respond_status(request, 503),
            },
            (Method::Post, "/api/select") => {
                let sel = {
                    let mut guard = lock(state);
                    let ServeState {
                        recent,
                        decks_dir,
                        cache,
                        ..
                    } = &mut *guard;
                    read_selection(&mut request, decks_dir, recent, cache)
                };
                match sel {
                    Some(sel) => match study.select(vec![sel.deck], sel.opts) {
                        None => respond_status(request, 503),
                        Some(None) => respond_status(request, 400),
                        Some(Some((dto, record))) => {
                            if let Some(paths) = record {
                                let mut guard = lock(state);
                                guard.recent.record(&paths, now_ms());
                                let _ = guard.recent.save();
                            }
                            match dto {
                                SelectedDto::Walk(dto) => respond_json(request, &dto),
                                SelectedDto::Review(dto) => respond_json(request, &dto),
                            }
                        }
                    },
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/browse") => {
                let sel = {
                    let mut guard = lock(state);
                    let ServeState {
                        recent,
                        decks_dir,
                        cache,
                        ..
                    } = &mut *guard;
                    read_selection(&mut request, decks_dir, recent, cache)
                };
                match sel {
                    Some(sel) => match study.browse(vec![sel.deck]) {
                        None => respond_status(request, 503),
                        Some(None) => respond_status(request, 400),
                        Some(Some((dto, record))) => {
                            let mut guard = lock(state);
                            guard.recent.record(&record, now_ms());
                            let _ = guard.recent.save();
                            drop(guard);
                            respond_json(request, &dto);
                        }
                    },
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/deck-drawer") => {
                let sel = {
                    let mut guard = lock(state);
                    let ServeState {
                        recent,
                        decks_dir,
                        cache,
                        ..
                    } = &mut *guard;
                    read_selection(&mut request, decks_dir, recent, cache)
                };
                match sel {
                    Some(sel) => match study.deck_drawer(sel.deck) {
                        Some(dto) => respond_json(request, &dto),
                        None => respond_status(request, 503),
                    },
                    None => respond_json(request, &DeckDrawerDto::default()),
                }
            }
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
                let resolved = {
                    let mut guard = lock(state);
                    let ServeState {
                        recent,
                        decks_dir,
                        cache,
                        ..
                    } = &mut *guard;
                    resolve_row(&body.deck, decks_dir, recent, cache)
                };
                let paths = match resolved {
                    Resolved::One(p) => vec![p],
                    Resolved::Many { files, .. } => files,
                    Resolved::Ambiguous | Resolved::Unknown => {
                        respond_status(request, 400);
                        continue;
                    }
                };
                match study.reset(body.deck, paths) {
                    None => respond_status(request, 503),
                    Some(Some(dto)) => respond_json(request, &dto),
                    Some(None) => respond_status(request, 400),
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
                let Some(projection) = study.projection() else {
                    respond_status(request, 503);
                    continue;
                };
                let mut guard = lock(state);
                let ServeState {
                    recent,
                    decks_dir,
                    cache,
                    launcher_icons,
                    ..
                } = &mut *guard;
                let dir = match resolve_row(&body.name, decks_dir, recent, cache) {
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
                match decks_list_dto(
                    scoped,
                    config_path.as_deref(),
                    decks_dir,
                    recent,
                    &projection,
                    launcher_icons,
                    review_cfg,
                    cache,
                ) {
                    Ok(catalog) => respond_json(request, &catalog),
                    Err(e) => {
                        eprintln!("deck listing failed for {}: {e}", decks_dir.display());
                        respond_status(request, 500);
                    }
                }
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
                let mut guard = lock(state);
                let ServeState {
                    recent,
                    decks_dir,
                    cache,
                    ..
                } = &mut *guard;
                let Some(dir) = resolve_dest(b.dest.as_deref(), decks_dir, recent, cache)
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
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let mut guard = lock(state);
                let ServeState {
                    recent,
                    decks_dir,
                    cache,
                    generating,
                    ..
                } = &mut *guard;
                if let Some(g) = generating.as_mut() {
                    g.poll();
                }
                if generating.as_ref().is_some_and(|g| g.outcome.is_none()) {
                    respond_status(request, 409);
                    continue;
                }
                let Some(b) =
                    body.filter(|b| b.url.starts_with("http://") || b.url.starts_with("https://"))
                else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(dest) = resolve_dest(b.dest.as_deref(), decks_dir, recent, cache)
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
                let mut guard = lock(state);
                let Some(g) = guard.generating.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                g.poll();
                respond_json(request, &g.dto());
            }
            (Method::Post, "/api/generate/close") => {
                lock(state).generating = None;
                respond_status(request, 200);
            }
            (Method::Post, "/api/share") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: Option<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let mut guard = lock(state);
                let ServeState {
                    recent,
                    decks_dir,
                    cache,
                    sharing,
                    ..
                } = &mut *guard;
                if let Some(s) = sharing.as_mut() {
                    s.poll();
                }
                if sharing.as_ref().is_some_and(|s| s.outcome.is_none()) {
                    respond_status(request, 409);
                    continue;
                }
                let path = match body.and_then(|b| b.deck) {
                    None => Some(decks_dir.clone()),
                    Some(name) => resolved_path(resolve_row(&name, decks_dir, recent, cache)),
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
                let mut guard = lock(state);
                let Some(s) = guard.sharing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                s.poll();
                respond_json(request, &s.dto());
            }
            (Method::Post, "/api/share/close") => {
                if let Some(s) = lock(state).sharing.take() {
                    s.job.cancel();
                }
                respond_status(request, 200);
            }
            (Method::Get, "/api/share/zip") => {
                let name = query_param(request.url(), "deck");
                let mut guard = lock(state);
                let ServeState {
                    recent,
                    decks_dir,
                    cache,
                    ..
                } = &mut *guard;
                let path = match &name {
                    None => Some(decks_dir.clone()),
                    Some(n) => resolved_path(resolve_row(n, decks_dir, recent, cache)),
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
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let mut guard = lock(state);
                let ServeState {
                    recent,
                    decks_dir,
                    cache,
                    receiving,
                    ..
                } = &mut *guard;
                if let Some(r) = receiving.as_mut() {
                    r.poll();
                }
                if receiving.as_ref().is_some_and(|r| r.outcome.is_none()) {
                    respond_status(request, 409);
                    continue;
                }
                let Some(b) = body else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(dest) = resolve_dest(b.dest.as_deref(), decks_dir, recent, cache)
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
                let mut guard = lock(state);
                let Some(r) = guard.receiving.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.poll();
                respond_json(request, &r.dto());
            }
            (Method::Post, "/api/receive/close") => {
                if let Some(r) = lock(state).receiving.take() {
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
                let dest_name = query_param(request.url(), "dest");
                let Some(bytes) = read_capped(request.as_reader(), MAX_ZIP) else {
                    respond_status(request, 400);
                    continue;
                };
                let mut guard = lock(state);
                let ServeState {
                    recent,
                    decks_dir,
                    cache,
                    ..
                } = &mut *guard;
                let Some(dest) = resolve_dest(dest_name.as_deref(), decks_dir, recent, cache)
                else {
                    respond_status(request, 400);
                    continue;
                };
                // `land_received`'s collision check is check-then-act: safe
                // only because this handler holds the residual state lock for
                // its whole body (the destination-owned landing command is
                // the Jobs-slice replacement).
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
            (Method::Post, "/api/deselect") => match study.deselect() {
                Some(dto) => respond_json(request, &dto),
                None => respond_status(request, 503),
            },
            (Method::Post, "/api/grade") => match read_grade(&mut request) {
                Some(grade) => match study.grade(grade) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                },
                None => respond_status(request, 400),
            },
            (Method::Post, "/api/skip") => match study.skip() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/acquire") => match study.acquire() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/check") => {
                #[derive(Deserialize)]
                struct Body {
                    lines: Vec<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(body) = body else {
                    respond_status(request, 400);
                    continue;
                };
                match study.check(body.lines) {
                    None => respond_status(request, 503),
                    Some(Feedback::NoSession) => respond_status(request, 409),
                    Some(Feedback::Bad) => respond_status(request, 400),
                    Some(Feedback::Ok(f)) => respond_json(request, &f),
                }
            }
            (Method::Post, "/api/choose") => match read_index(&mut request) {
                None => respond_status(request, 400),
                Some(chosen) => match study.choose(chosen) {
                    None => respond_status(request, 503),
                    Some(Feedback::NoSession) => respond_status(request, 409),
                    Some(Feedback::Bad) => respond_status(request, 400),
                    Some(Feedback::Ok(f)) => respond_json(request, &f),
                },
            },
            (Method::Post, "/api/remove") => match study.remove() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/promote") => match study.promote() {
                None => respond_status(request, 503),
                Some(Feedback::NoSession) => respond_status(request, 409),
                Some(Feedback::Bad) => respond_status(request, 400),
                Some(Feedback::Ok(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/restart") => match study.restart() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/ask") => {
                #[derive(Deserialize)]
                struct Body {
                    question: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let action = body
                    .map(|b| b.question)
                    .filter(|q| !q.trim().is_empty())
                    .map(AskAction::Question);
                match study.ask_start(action, ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/ask/note") => {
                match study.ask_start(Some(AskAction::Condense), ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/ask/card/draft") => {
                if audience == Audience::Kids {
                    respond_status(request, 403);
                    continue;
                }
                match study.ask_start(Some(AskAction::DraftCard), ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/ask/card/create") => {
                if audience == Audience::Kids {
                    respond_status(request, 403);
                    continue;
                }
                let Some(req) =
                    serde_json::from_reader::<_, CreateCardReq>(request.as_reader()).ok()
                else {
                    respond_status(request, 400);
                    continue;
                };
                match study.ask_create(req) {
                    None => respond_status(request, 503),
                    Some(CreateOutcome::NoSession) => respond_status(request, 409),
                    Some(CreateOutcome::Invalid) => respond_status(request, 422),
                    Some(CreateOutcome::MintFailed) => respond_status(request, 500),
                    Some(CreateOutcome::Ok(resp)) => respond_json(request, &resp),
                }
            }
            (Method::Get, "/api/ask") => match study.ask_poll() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
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
                let resolved = {
                    let mut guard = lock(state);
                    let ServeState {
                        recent,
                        decks_dir,
                        cache,
                        ..
                    } = &mut *guard;
                    resolved_path(resolve_row(&body.deck, decks_dir, recent, cache))
                        .map(|path| (path, decks_dir.clone()))
                };
                let Some((path, decks_root)) = resolved else {
                    respond_status(request, 400);
                    continue;
                };
                match study.exam_start(path, decks_root, ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(ExamStartReply::Dto(dto)) => respond_json(request, &dto),
                    Some(ExamStartReply::Conflict) => respond_status(request, 409),
                }
            }
            (Method::Get, "/api/exam") => match study.exam_poll() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/exam/answer") => {
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                    goto: Option<usize>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let (text, goto) = match body {
                    Some(b) => (b.text, b.goto),
                    None => (String::new(), None),
                };
                match study.exam_answer(text, goto) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/exam/grade") => {
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let text = body.map(|b| b.text).unwrap_or_default();
                match study.exam_grade(text) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/exam/remediate") => match study.exam_remediate() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/exam/close") => match study.exam_close() {
                Some(dto) => respond_json(request, &dto),
                None => respond_status(request, 503),
            },
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
                let resolved = {
                    let mut guard = lock(state);
                    let ServeState {
                        recent,
                        decks_dir,
                        cache,
                        ..
                    } = &mut *guard;
                    (
                        resolve_row(&body.deck, decks_dir, recent, cache),
                        decks_dir.clone(),
                    )
                };
                let (files, workspace_dir) = match resolved.0 {
                    Resolved::One(p) => (vec![p], None),
                    Resolved::Many { dir, files } => (files, Some(dir)),
                    _ => {
                        respond_status(request, 400);
                        continue;
                    }
                };
                match study.augment_open(body.deck, files, workspace_dir, resolved.1) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
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
                let targets = body.map(|b| {
                    b.targets
                        .into_iter()
                        .map(|t| {
                            let guidance = t
                                .with
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty());
                            (t.target, guidance)
                        })
                        .collect()
                });
                match study.augment_generate(targets, ai_cfg.clone(), ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Get, "/api/augment") => {
                match study.augment_poll(ai_cfg.clone(), ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/augment/remove") => {
                #[derive(Deserialize)]
                struct Body {
                    target: String,
                    topology: Option<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(b) = body else {
                    match study.augment_poll(ai_cfg.clone(), ask_cfg.clone()) {
                        None => respond_status(request, 503),
                        Some(None) => respond_status(request, 409),
                        Some(Some(dto)) => respond_json(request, &dto),
                    }
                    continue;
                };
                match study.augment_remove(b.target, b.topology) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/augment/close") => match study.augment_close() {
                Some(dto) => respond_json(request, &dto),
                None => respond_status(request, 503),
            },
            (Method::Get, "/api/walk") => match study.walk_poll() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/walk/predict") => {
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(b) = body else {
                    match study.walk_poll() {
                        None => respond_status(request, 503),
                        Some(None) => respond_status(request, 409),
                        Some(Some(dto)) => respond_json(request, &dto),
                    }
                    continue;
                };
                match study.walk_predict(b.text) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/walk/grade") => {
                let self_delta = read_delta(&mut request);
                match study.walk_grade(self_delta) {
                    None => respond_status(request, 503),
                    Some(WalkGradeReply::NoWalk) => respond_status(request, 409),
                    Some(WalkGradeReply::NoDelta) => respond_status(request, 400),
                    Some(WalkGradeReply::Dto(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/walk/restart") => match study.walk_restart() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/walk/ask") => {
                #[derive(Deserialize)]
                struct Body {
                    question: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let question = body.map(|b| b.question).filter(|q| !q.trim().is_empty());
                match study.walk_ask(WalkAskAction::Question(question), ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/walk/ask/note") => {
                match study.walk_ask(WalkAskAction::Note, ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Get, "/api/walk/ask") => match study.walk_ask_poll() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/walk/leave") => match study.walk_leave() {
                Some(dto) => respond_json(request, &dto),
                None => respond_status(request, 503),
            },
            (Method::Post, "/api/remote/ask") => {
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let mut guard = lock(state);
                let remote_ask = &mut guard.remote_ask;
                if let Some(a) = remote_ask.as_mut() {
                    a.poll();
                }
                if remote_ask.as_ref().is_some_and(RemoteAsk::thinking) {
                    respond_status(request, 409);
                    continue;
                }
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
                let job = RemoteAsk::ask(ask_cfg, &card, history, &question);
                let dto = job.dto();
                *remote_ask = Some(job);
                respond_json(request, &dto);
            }
            (Method::Get, "/api/remote/ask") => {
                let mut guard = lock(state);
                let dto = match guard.remote_ask.as_mut() {
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
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let mut guard = lock(state);
                let remote_ask = &mut guard.remote_ask;
                if let Some(a) = remote_ask.as_mut() {
                    a.poll();
                }
                if remote_ask.as_ref().is_some_and(RemoteAsk::thinking) {
                    respond_status(request, 409);
                    continue;
                }
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
                let job = RemoteAsk::draft(ask_cfg, &card, history);
                let dto = job.dto();
                *remote_ask = Some(job);
                respond_json(request, &dto);
            }
            (Method::Post, "/api/remote/ask/note") => {
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let mut guard = lock(state);
                let remote_ask = &mut guard.remote_ask;
                if let Some(a) = remote_ask.as_mut() {
                    a.poll();
                }
                if remote_ask.as_ref().is_some_and(RemoteAsk::thinking) {
                    respond_status(request, 409);
                    continue;
                }
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
                let job = RemoteAsk::note(ask_cfg, &card, history);
                let dto = job.dto();
                *remote_ask = Some(job);
                respond_json(request, &dto);
            }
            // The requires-lock and the trace re-sit cooldown are the
            // browser's own truth; both are deliberately skipped here.
            (Method::Post, "/api/remote/exam/start") => {
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let Ok(body) = serde_json::from_slice::<Body>(&bytes) else {
                    respond_status(request, 400);
                    continue;
                };
                let mut guard = lock(state);
                let ServeState {
                    recent,
                    decks_dir,
                    cache,
                    remote_exam,
                    ..
                } = &mut *guard;
                if remote_exam.is_some() {
                    respond_status(request, 409);
                    continue;
                }
                let Some(path) =
                    resolved_path(resolve_row(&body.deck, decks_dir, recent, cache))
                else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(deck) = Deck::load(&path) else {
                    respond_status(request, 409);
                    continue;
                };
                if !deck.has_exam() {
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
                            deck.deck_token.clone().unwrap_or_default(),
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
                    if exam::ensure_backend_can_examine(&deck, ask_cfg).is_err() {
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
                let mut guard = lock(state);
                let dto = match guard.remote_exam.as_mut() {
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
                let mut guard = lock(state);
                let Some(ex) = guard.remote_exam.as_mut() else {
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
                let mut guard = lock(state);
                let Some(ex) = guard.remote_exam.as_mut() else {
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
                lock(state).remote_exam = None;
                respond_status(request, 200);
            }
            // No dest, no destination-collision check: this returns the
            // deck text, it never places a file (both are the phone's job).
            (Method::Post, "/api/remote/generate") => {
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let mut guard = lock(state);
                let remote_generate = &mut guard.remote_generate;
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
                let mut guard = lock(state);
                let Some(g) = guard.remote_generate.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                g.poll();
                respond_json(request, &g.dto());
            }
            (Method::Post, "/api/remote/generate/close") => {
                lock(state).remote_generate = None;
                respond_status(request, 200);
            }
            _ => respond_status(request, 404),
        }
            });
        }
    });

    // Workers have drained (the unblock relay), so no more commands can
    // arrive; dropping the last handle lets the owner run its final flush
    // and exit. A panic on the owner thread is the application failing, not
    // a condition to absorb: propagate it to the caller's thread.
    drop(study);
    if let Err(panic) = study_thread.join() {
        std::panic::resume_unwind(panic);
    }
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
) -> Result<DeckListDto, std::io::Error> {
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
