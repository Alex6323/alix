mod catalog;
mod catalog_owner;
mod dto;
mod jobs;
mod jobs_owner;
mod respond;
mod study;

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    thread,
};

use anyhow::{Result, anyhow};
use catalog::*;
use catalog_owner::*;
use dto::*;
use jobs::*;
use jobs_owner::*;
use respond::*;
use study::*;
use serde::Deserialize;
use tiny_http::{Method, Server};

pub use crate::assemble::SelectOptions;
use crate::{
    assemble::{self, CardsBuild, SessionBuild},
    config::{
        AiConfig, Audience, Bindings, BrowseBindings, ExamConfig, GenerateDeckConfig, PickerKeys,
    },
    doctor, import,
    recent::RecentDecks,
    share,
    store::Store,
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

// Set when any owner thread panics; workers then stop accepting (503 and
// unblock) so a dead owner can never leave a permanently half-alive server.
pub(super) static OWNER_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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

    let (catalog, catalog_thread) = catalog_owner::spawn(CatalogState::new(
        CatalogConfig {
            scoped,
            config_path: config_path.clone(),
            review_cfg,
        },
        decks_dir,
        recent,
    ));

    let (jobs, jobs_thread) = jobs_owner::spawn(JobsState {
        catalog: catalog.clone(),
        generating: None,
        sharing: None,
        receiving: None,
        remote_ask: None,
        remote_exam: None,
        remote_generate: None,
    });

    // Owner failure is application failure: a panicked owner sets this, the
    // workers stop accepting (503 + unblock), and run_review re-raises the
    // panic after joining. No restart, no permanent partial service.
    let failed = &OWNER_FAILED;
    failed.store(false, std::sync::atomic::Ordering::SeqCst);

    thread::scope(|scope| {
        for _ in 0..WORKERS {
            let study = study.clone();
            let catalog = catalog.clone();
            let jobs = jobs.clone();
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
                if OWNER_FAILED.load(std::sync::atomic::Ordering::SeqCst) {
                    server.unblock();
                    respond_status(request, 503);
                    continue;
                }
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
                        let Some(decks_root) = catalog.decks_root() else {
                            respond_status(request, 503);
                            continue;
                        };
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
                match catalog.list(projection) {
                    None => respond_status(request, 503),
                    Some(Ok(dto)) => respond_json(request, &dto),
                    Some(Err(e)) => {
                        eprintln!("deck listing failed for {e}");
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
                    Some(ImageSource::NoActive) => match catalog.launcher_icon(name.to_string()) {
                        Some(path) => serve_image_path(request, path.as_deref()),
                        None => respond_status(request, 503),
                    },
                    None => respond_status(request, 503),
                }
            }
            (Method::Get, "/api/state") => match study.state() {
                Some(SessionSnapshot::Browse(dto)) => respond_json(request, &dto),
                Some(SessionSnapshot::Review(dto)) => respond_json(request, &dto),
                None => respond_status(request, 503),
            },
            (Method::Post, "/api/select") => {
                let sel = parse_selection(&mut request).and_then(|(name, opts)| {
                    catalog
                        .resolve_path(name)
                        .flatten()
                        .map(|deck| Selection { deck, opts })
                });
                match sel {
                    Some(sel) => match study.select(vec![sel.deck], sel.opts) {
                        None => respond_status(request, 503),
                        Some(None) => respond_status(request, 400),
                        Some(Some((dto, record))) => {
                            if let Some(paths) = record {
                                catalog.record_recent(paths);
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
                let sel = parse_selection(&mut request).and_then(|(name, opts)| {
                    catalog
                        .resolve_path(name)
                        .flatten()
                        .map(|deck| Selection { deck, opts })
                });
                match sel {
                    Some(sel) => match study.browse(vec![sel.deck]) {
                        None => respond_status(request, 503),
                        Some(None) => respond_status(request, 400),
                        Some(Some((dto, record))) => {
                            catalog.record_recent(record);
                            respond_json(request, &dto);
                        }
                    },
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/deck-drawer") => {
                let sel = parse_selection(&mut request).and_then(|(name, opts)| {
                    catalog
                        .resolve_path(name)
                        .flatten()
                        .map(|deck| Selection { deck, opts })
                });
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
                let Some(resolved) = catalog.resolve(body.deck.clone()) else {
                    respond_status(request, 503);
                    continue;
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
                match catalog.set_deadline(body.name, date, projection) {
                    None => respond_status(request, 503),
                    Some(Ok(dto)) => respond_json(request, &dto),
                    Some(Err(SetDeadlineError::BadTarget)) => respond_status(request, 400),
                    Some(Err(SetDeadlineError::WriteFailed)) => respond_status(request, 500),
                    Some(Err(SetDeadlineError::ListFailed(e))) => {
                        eprintln!("deck listing failed for {e}");
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
                let Some(Some(dir)) = catalog.resolve_dest(b.dest.clone()) else {
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
                        catalog.invalidate_content();
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
                let Some(b) =
                    body.filter(|b| b.url.starts_with("http://") || b.url.starts_with("https://"))
                else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(Some(dest)) = catalog.resolve_dest(b.dest.clone()) else {
                    respond_status(request, 400);
                    continue;
                };
                let guidance = b
                    .guidance
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                match jobs.generate_start(b.url, guidance, dest, generate_cfg.clone(), ask_cfg.clone())
                {
                    None => respond_status(request, 503),
                    Some(Started::Conflict) => respond_status(request, 409),
                    Some(Started::Dto(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Get, "/api/generate") => match jobs.generate_poll() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/generate/close") => match jobs.generate_close() {
                Some(()) => respond_status(request, 200),
                None => respond_status(request, 503),
            },
            (Method::Post, "/api/share") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: Option<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let path = match body.and_then(|b| b.deck) {
                    None => catalog.decks_root(),
                    Some(name) => catalog.resolve_path(name).flatten(),
                };
                let Some(path) = path else {
                    respond_status(request, 400);
                    continue;
                };
                match jobs.share_start(path) {
                    None => respond_status(request, 503),
                    Some(Started::Conflict) => respond_status(request, 409),
                    Some(Started::Dto(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Get, "/api/share") => match jobs.share_poll() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/share/close") => match jobs.share_close() {
                Some(()) => respond_status(request, 200),
                None => respond_status(request, 503),
            },
            (Method::Get, "/api/share/zip") => {
                let name = query_param(request.url(), "deck");
                let path = match &name {
                    None => catalog.decks_root(),
                    Some(n) => catalog.resolve_path(n.clone()).flatten(),
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
                let Some(b) = body else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(Some(dest)) = catalog.resolve_dest(b.dest.clone()) else {
                    respond_status(request, 400);
                    continue;
                };
                match jobs.receive_start(b.code, dest) {
                    None => respond_status(request, 503),
                    Some(Started::Conflict) => respond_status(request, 409),
                    Some(Started::Dto(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Get, "/api/receive") => match jobs.receive_poll() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/receive/close") => match jobs.receive_close() {
                Some(()) => respond_status(request, 200),
                None => respond_status(request, 503),
            },
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
                let Some(Some(dest)) = catalog.resolve_dest(dest_name) else {
                    respond_status(request, 400);
                    continue;
                };
                match jobs.receive_zip(bytes, dest) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 400),
                    Some(Some(dto)) => respond_json(request, &dto),
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
                let resolved = catalog
                    .resolve_path(body.deck.clone())
                    .flatten()
                    .and_then(|path| catalog.decks_root().map(|root| (path, root)));
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
                let Some(resolved) = catalog.resolve(body.deck.clone()) else {
                    respond_status(request, 503);
                    continue;
                };
                let Some(decks_root) = catalog.decks_root() else {
                    respond_status(request, 503);
                    continue;
                };
                let (files, workspace_dir) = match resolved {
                    Resolved::One(p) => (vec![p], None),
                    Resolved::Many { dir, files } => (files, Some(dir)),
                    _ => {
                        respond_status(request, 400);
                        continue;
                    }
                };
                match study.augment_open(body.deck, files, workspace_dir, decks_root) {
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
                let Ok(req) = serde_json::from_slice::<RemoteAskReq>(&bytes) else {
                    respond_status(request, 400);
                    continue;
                };
                if req.question.trim().is_empty()
                    || (req.card.front.trim().is_empty()
                        && req.card.back.iter().all(|l| l.trim().is_empty()))
                {
                    respond_status(request, 400);
                    continue;
                }
                match jobs.remote_ask(req, ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(Started::Conflict) => respond_status(request, 409),
                    Some(Started::Dto(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Get, "/api/remote/ask") => match jobs.remote_ask_poll() {
                Some(dto) => respond_json(request, &dto),
                None => respond_status(request, 503),
            },
            (Method::Post, "/api/remote/ask/draft") => {
                if audience == Audience::Kids {
                    respond_status(request, 403);
                    continue;
                }
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(req) = serde_json::from_slice::<RemoteDraftReq>(&bytes) else {
                    respond_status(request, 400);
                    continue;
                };
                if req.history.is_empty() {
                    respond_status(request, 400);
                    continue;
                }
                match jobs.remote_draft(req, ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(Started::Conflict) => respond_status(request, 409),
                    Some(Started::Dto(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/remote/ask/note") => {
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
                let Ok(req) = serde_json::from_slice::<RemoteNoteReq>(&bytes) else {
                    respond_status(request, 400);
                    continue;
                };
                if req.history.is_empty() {
                    respond_status(request, 400);
                    continue;
                }
                match jobs.remote_note(req, ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(Started::Conflict) => respond_status(request, 409),
                    Some(Started::Dto(dto)) => respond_json(request, &dto),
                }
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
                let Some(path) = catalog.resolve_path(body.deck.clone()).flatten() else {
                    respond_status(request, 400);
                    continue;
                };
                match jobs.remote_exam_start(path, exam_cfg.clone(), ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(Started::Conflict) => respond_status(request, 409),
                    Some(Started::Dto(dto)) => respond_json(request, &dto),
                }
            }
            // advance() only, never poll(): poll() writes the store, which
            // remote handlers must never touch.
            (Method::Get, "/api/remote/exam") => match jobs.remote_exam_poll() {
                Some(dto) => respond_json(request, &dto),
                None => respond_status(request, 503),
            },
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
                match jobs.remote_exam_grade(body.answers) {
                    None => respond_status(request, 503),
                    Some(RemoteGradeReply::NoSitting) => respond_status(request, 409),
                    Some(RemoteGradeReply::WrongPhaseOrCount) => respond_status(request, 400),
                    Some(RemoteGradeReply::Dto(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/remote/exam/remediate") => {
                match jobs.remote_exam_remediate() {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/remote/exam/close") => match jobs.remote_exam_close() {
                Some(()) => respond_status(request, 200),
                None => respond_status(request, 503),
            },
            // No dest, no destination-collision check: this returns the
            // deck text, it never places a file (both are the phone's job).
            (Method::Post, "/api/remote/generate") => {
                let Some(bytes) = read_capped(request.as_reader(), MAX_REMOTE_BODY) else {
                    respond_status(request, 400);
                    continue;
                };
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
                let guidance = body
                    .guidance
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                match jobs.remote_generate_start(
                    body.url,
                    guidance,
                    generate_cfg.clone(),
                    ask_cfg.clone(),
                ) {
                    None => respond_status(request, 503),
                    Some(Started::Conflict) => respond_status(request, 409),
                    Some(Started::Dto(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Get, "/api/remote/generate") => match jobs.remote_generate_poll() {
                None => respond_status(request, 503),
                Some(None) => respond_status(request, 409),
                Some(Some(dto)) => respond_json(request, &dto),
            },
            (Method::Post, "/api/remote/generate/close") => match jobs.remote_generate_close() {
                Some(()) => respond_status(request, 200),
                None => respond_status(request, 503),
            },
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
    // Jobs before Catalog: the Jobs owner holds a Catalog handle for its
    // landing invalidations, so the Catalog channel only closes once the
    // Jobs thread is gone.
    drop(jobs);
    if let Err(panic) = jobs_thread.join() {
        std::panic::resume_unwind(panic);
    }
    drop(catalog);
    if let Err(panic) = catalog_thread.join() {
        std::panic::resume_unwind(panic);
    }
    Ok(())
}

#[cfg(test)]
mod contract;
#[cfg(test)]
mod tests;
