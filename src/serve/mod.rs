mod catalog;
mod catalog_owner;
mod dto;
mod jobs;
mod jobs_owner;
mod respond;
mod study;
mod web_assets;

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use catalog::*;
use catalog_owner::*;
use dto::*;
use jobs::*;
use jobs_owner::*;
use respond::*;
use serde::Deserialize;
use study::*;
use tiny_http::{Method, Request, Server};
use web_assets::*;

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
    workspace,
};

const MAX_REMOTE_BODY: usize = 256 * 1024;

/// Every JSON body is read through this, never straight off the socket:
/// `from_reader` keeps reading while a client keeps sending, so one request
/// could grow the server's memory without bound. Generous next to what these
/// routes carry (deck names, a card, a tutor question).
const MAX_JSON_BODY: usize = 256 * 1024;

fn json_body<T: serde::de::DeserializeOwned>(request: &mut Request) -> Option<T> {
    let bytes = read_capped(request.as_reader(), MAX_JSON_BODY)?;
    serde_json::from_slice(&bytes).ok()
}

struct RequestTiming {
    worker: usize,
    popped: Instant,
    since_start: Duration,
}

impl Drop for RequestTiming {
    fn drop(&mut self) {
        crate::log::emit(
            crate::log::Target::Http,
            format_args!(
                "at={}ms took={}ms w={}",
                self.since_start.as_millis(),
                self.popped.elapsed().as_millis(),
                self.worker,
            ),
        );
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
    pub log_path: Option<PathBuf>,
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
            "cannot start the server on {addr}: {e}. Is another alix using this port? try --port"
        )
    })
}

// Connection workers pull parsed requests off tiny_http's queue in parallel,
// so an idle kept-alive socket can't starve the rest.
const WORKERS: usize = 16;

// Owner failure is application failure: a panicked owner trips this, which
// unblocks the server immediately (an idle server must drain too, not wait
// for a next request) and marks every worker to answer 503 while draining.
// Per run, never global: parallel in-process test servers stay independent.
#[derive(Clone)]
pub(super) struct OwnerFailure {
    failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    server: Arc<Server>,
}

impl OwnerFailure {
    pub(super) fn new(server: Arc<Server>) -> Self {
        OwnerFailure {
            failed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            server,
        }
    }

    pub(super) fn trip(&self) {
        self.failed.store(true, std::sync::atomic::Ordering::SeqCst);
        self.server.unblock();
    }

    pub(super) fn tripped(&self) -> bool {
        self.failed.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Runs an owner loop under panic supervision: a panic trips the failure
/// (draining the server) and is re-raised so the join in `run_review`
/// propagates it to the caller.
pub(super) fn supervised(
    failure: OwnerFailure,
    body: impl FnOnce() + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
            failure.trip();
            std::panic::resume_unwind(panic);
        }
    })
}

/// The revision a card-relative mutation echoes back. Missing or malformed
/// is the client's error (400); staleness is decided by the Study owner.
fn echoed_revision(request: &tiny_http::Request) -> Option<u64> {
    header_value(request, "X-Alix-Study-Revision")?.parse().ok()
}

fn library_target(name: String, resolved: Resolved) -> Option<LibraryTarget> {
    match resolved {
        Resolved::One(path) if path.is_file() => Some(LibraryTarget::Deck { name, path }),
        Resolved::One(root) if root.is_dir() && workspace::has_manifest(&root) => {
            let members = workspace::classified_deck_files(&root).ok()?.0;
            Some(LibraryTarget::Workspace {
                name,
                root,
                members,
            })
        }
        Resolved::Many { dir, files } if workspace::has_manifest(&dir) => {
            Some(LibraryTarget::Workspace {
                name,
                root: dir,
                members: files,
            })
        }
        Resolved::One(_) | Resolved::Many { .. } | Resolved::Ambiguous | Resolved::Unknown => None,
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
        log_path,
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
    let started_at = Instant::now();
    let report_root = decks_dir.clone();

    let failure = OwnerFailure::new(Arc::clone(&server));

    let (study, study_thread) = study::spawn(
        failure.clone(),
        StudyState {
            config: StudyConfig {
                cfg,
                exam_cfg: exam_cfg.clone(),
                review_cfg,
                audience,
            },
            store,
            retained: HashMap::new(),
            store_dirty: false,
            progress_stamp: None,
            save_error: None,
            reviewing: None,
            revision: 0,
            writes: 0,
            browsing: None,
            examining: None,
            walking: None,
            augmenting: None,
        },
    );

    let (catalog, catalog_thread) = catalog_owner::spawn(
        failure.clone(),
        CatalogState::new(
            CatalogConfig {
                scoped,
                config_path: config_path.clone(),
                review_cfg,
            },
            decks_dir,
            recent,
        ),
    );

    let (jobs, jobs_thread) = jobs_owner::spawn(
        failure.clone(),
        JobsState {
            catalog: catalog.clone(),
            generating: None,
            sharing: None,
            receiving: None,
            remote_ask: None,
            remote_exam: None,
            remote_generate: None,
        },
    );

    // tiny_http can strand an accepted connection: a connection burst racing
    // its task pool's waiter accounting leaves a reader task queued with its
    // request unread in the kernel buffer, until some other connection event
    // makes a pool thread pop the queue (measured: a browser reload against a
    // fresh server stalled one subresource 110s, and a single connect+close
    // released it in <1s). This pump IS that connection event, once a second,
    // bounding any stranding to about one interval. It sends no request, so
    // it never appears in the http log; it ends when the listener does.
    {
        let addr = server.server_addr();
        thread::spawn(move || {
            let port = addr.to_ip().map(|a| a.port()).unwrap_or(0);
            if port == 0 {
                return;
            }
            let target = SocketAddr::from(([127, 0, 0, 1], port));
            loop {
                thread::sleep(Duration::from_millis(1000));
                match std::net::TcpStream::connect_timeout(&target, Duration::from_millis(500)) {
                    Ok(stream) => drop(stream),
                    Err(_) => return,
                }
            }
        });
    }

    thread::scope(|scope| {
        for worker in 0..WORKERS {
            let study = study.clone();
            let catalog = catalog.clone();
            let jobs = jobs.clone();
            let failure = failure.clone();
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
            let log_path = &log_path;
            let report_root = &report_root;
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
                if failure.tripped() {
                    server.unblock();
                    respond_status(request, 503);
                    continue;
                }
                let method = request.method().clone();
                let path = request_path(&request);
                // Dropped at the end of the iteration, however this request
                // exits, so it times the whole pop-to-response span rather than
                // one handler's happy path.
                let _timing = crate::log::enabled(crate::log::Target::Http).then(|| RequestTiming {
                    worker,
                    popped: Instant::now(),
                    since_start: started_at.elapsed(),
                });
                if !is_authorized(
                    &path,
                    header_value(&request, "Authorization"),
                    query_param(request.url(), "token").as_deref(),
                    auth.as_deref(),
                ) {
                    respond_status(request, 401);
                    continue;
                }
                if method == Method::Get
                    && let Some((body, content_type)) = web_asset(&path)
                {
                    respond_asset(request, body, content_type);
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
                    (Method::Get, "/api/bug-report") => {
                        let result = (|| -> Result<(Vec<u8>, String)> {
                            let stage = tempfile::tempdir()?;
                            let home = directories::BaseDirs::new()
                                .ok_or_else(|| anyhow!("cannot determine the home directory"))?;
                            let tokens = auth.iter().cloned().collect::<Vec<_>>();
                            let log_paths = log_path.iter().cloned().collect::<Vec<_>>();
                            let bundle = crate::bug_report::write_bundle_with(
                                &crate::bug_report::BundleOptions {
                                    root: report_root,
                                    out_dir: stage.path(),
                                    config_path: config_path.as_deref(),
                                    log_paths: &log_paths,
                                    include_deck: None,
                                    home: home.home_dir(),
                                    tokens: &tokens,
                                    now_ms: crate::time::now_ms(),
                                },
                            )?;
                            let filename = bundle
                                .path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .ok_or_else(|| anyhow!("bug report filename is not portable"))?
                                .to_string();
                            Ok((std::fs::read(bundle.path)?, filename))
                        })();
                        match result {
                            Ok((bytes, filename)) => respond_download(
                                request,
                                bytes,
                                "application/zip",
                                &filename,
                            ),
                            Err(_) => respond_status(request, 500),
                        }
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
                            doctor::check_log(log_path.clone()),
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
                        Some(Transition::Rejected) => respond_status(request, 400),
                        Some(Transition::FlushFailed) => respond_status(request, 500),
                        Some(Transition::Done((dto, record))) => {
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
                        Some(Transition::Rejected) => respond_status(request, 400),
                        Some(Transition::FlushFailed) => respond_status(request, 500),
                        Some(Transition::Done((dto, record))) => {
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
                let Some(body) = json_body::<Body>(&mut request)
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
                    Some(Transition::Done(dto)) => respond_json(request, &dto),
                    Some(Transition::Rejected) => respond_status(request, 400),
                    Some(Transition::FlushFailed) => respond_status(request, 500),
                }
            }
            (Method::Post, "/api/library/remove/preview") => {
                #[derive(Deserialize)]
                struct Body {
                    name: String,
                }
                let Some(body) = json_body::<Body>(&mut request) else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(resolved) = catalog.resolve(body.name.clone()) else {
                    respond_status(request, 503);
                    continue;
                };
                let Some(target) = library_target(body.name, resolved) else {
                    respond_status(request, 400);
                    continue;
                };
                match study.removal_preview(target) {
                    None => respond_status(request, 503),
                    Some(Transition::Done(dto)) => respond_json(request, &dto),
                    Some(Transition::Rejected) => respond_status(request, 400),
                    Some(Transition::FlushFailed) => respond_status(request, 500),
                }
            }
            (Method::Post, "/api/library/remove") => {
                #[derive(Deserialize)]
                struct Body {
                    name: String,
                }
                let Some(body) = json_body::<Body>(&mut request) else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(resolved) = catalog.resolve(body.name.clone()) else {
                    respond_status(request, 503);
                    continue;
                };
                let Some(target) = library_target(body.name, resolved) else {
                    respond_status(request, 400);
                    continue;
                };
                let recent_paths = target.recent_paths();
                match study.remove_library(target) {
                    None => respond_status(request, 503),
                    Some(RemovalOutcome::Rejected) => respond_status(request, 400),
                    Some(RemovalOutcome::Busy) => respond_status(request, 409),
                    Some(RemovalOutcome::FlushFailed) => respond_status(request, 500),
                    Some(RemovalOutcome::Done(dto)) => match catalog.forget_recent(recent_paths) {
                        None => respond_status(request, 503),
                        Some(Ok(())) => respond_json(request, &dto),
                        Some(Err(error)) => {
                            eprintln!(
                                "warning: could not update recent history after removing {}: {error}",
                                dto.target
                            );
                            respond_json_status(
                                request,
                                500,
                                &RemovalFailureDto {
                                    target: dto.target,
                                    error: "removal incomplete",
                                    completed: dto.removed,
                                    failed: "recent.json".to_string(),
                                    recovery: "Run alix doctor to inspect and repair the remaining artifacts.",
                                },
                            );
                        }
                    },
                    Some(RemovalOutcome::Failed {
                        dto,
                        target_removed,
                    }) => {
                        if target_removed {
                            if let Some(Err(error)) = catalog.forget_recent(recent_paths) {
                                eprintln!(
                                    "warning: could not update recent history after partial removal: {error}"
                                );
                            }
                        } else {
                            catalog.invalidate_content();
                        }
                        respond_json_status(request, 500, &dto);
                    }
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
                let Some(body) = json_body::<Body>(&mut request)
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
                match catalog.set_deadline(body.name, date) {
                    None => respond_status(request, 503),
                    Some(Err(SetDeadlineError::BadTarget)) => respond_status(request, 400),
                    Some(Err(SetDeadlineError::WriteFailed)) => respond_status(request, 500),
                    Some(Ok(())) => match catalog.list(projection) {
                        None => respond_status(request, 503),
                        Some(Ok(dto)) => respond_json(request, &dto),
                        Some(Err(e)) => {
                            eprintln!("deck listing failed for {e}");
                            respond_status(request, 500);
                        }
                    },
                }
            }
            (Method::Post, "/api/import") => {
                #[derive(Deserialize)]
                struct Body {
                    name: String,
                    text: String,
                    dest: Option<String>,
                }
                let Some(b) = json_body::<Body>(&mut request) else {
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
                match jobs.import_deck(dir, place_name, text) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 400),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/generate") => {
                #[derive(Deserialize)]
                struct Body {
                    url: String,
                    guidance: Option<String>,
                    dest: Option<String>,
                }
                let body: Option<Body> = json_body::<Body>(&mut request);
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
                let body: Option<Body> = json_body::<Body>(&mut request);
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
                let body: Option<Body> = json_body::<Body>(&mut request);
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
                None => respond_status(request, 503),
                Some(Transition::Done(dto)) => respond_json(request, &dto),
                Some(Transition::Rejected) | Some(Transition::FlushFailed) => {
                    respond_status(request, 500)
                }
            },
            (Method::Post, "/api/grade") => {
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                match read_grade(&mut request) {
                    Some(grade) => match study.grade(grade, expected) {
                        None => respond_status(request, 503),
                        Some(None) => respond_status(request, 409),
                        Some(Some(dto)) => respond_json(request, &dto),
                    },
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/skip") => {
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                match study.skip(expected) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/introduce") => {
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                match study.introduce(expected) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/check") => {
                #[derive(Deserialize)]
                struct Body {
                    lines: Vec<String>,
                }
                let body: Option<Body> = json_body::<Body>(&mut request);
                let Some(body) = body else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                match study.check(body.lines, expected) {
                    None => respond_status(request, 503),
                    Some(Feedback::NoSession) => respond_status(request, 409),
                    Some(Feedback::Bad) => respond_status(request, 400),
                    Some(Feedback::Ok(f)) => respond_json(request, &f),
                }
            }
            (Method::Post, "/api/choose") => {
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                #[derive(Deserialize)]
                struct Body {
                    index: Option<usize>,
                    indices: Option<Vec<usize>>,
                    card: String,
                }
                match json_body::<Body>(&mut request) {
                    None => respond_status(request, 400),
                    // Exactly one submission shape: `index` grades a single
                    // pick, `indices` a select-all; both or neither is a bad
                    // body, and the cross-shape pair 400s in the lib.
                    Some(b) => match (b.index, b.indices) {
                        (Some(index), None) => match study.choose(index, b.card, expected) {
                            None => respond_status(request, 503),
                            Some(Feedback::NoSession) => respond_status(request, 409),
                            Some(Feedback::Bad) => respond_status(request, 400),
                            Some(Feedback::Ok(f)) => respond_json(request, &f),
                        },
                        (None, Some(indices)) => {
                            match study.choose_multi(indices, b.card, expected) {
                                None => respond_status(request, 503),
                                Some(Feedback::NoSession) => respond_status(request, 409),
                                Some(Feedback::Bad) => respond_status(request, 400),
                                Some(Feedback::Ok(f)) => respond_json(request, &f),
                            }
                        }
                        _ => respond_status(request, 400),
                    },
                }
            }
            (Method::Post, "/api/remove") => {
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                match study.remove(expected) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/restart") => {
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                match study.restart(expected) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/ask") => {
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                #[derive(Deserialize)]
                struct Body {
                    question: String,
                }
                let body: Option<Body> = json_body::<Body>(&mut request);
                let action = body
                    .map(|b| b.question)
                    .filter(|q| !q.trim().is_empty())
                    .map(AskAction::Question);
                match study.ask_start(action, ask_cfg.clone(), expected) {
                    None => respond_status(request, 503),
                    Some(None) => respond_status(request, 409),
                    Some(Some(dto)) => respond_json(request, &dto),
                }
            }
            (Method::Post, "/api/ask/note") => {
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                match study.ask_start(Some(AskAction::Condense), ask_cfg.clone(), expected) {
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
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                match study.ask_start(Some(AskAction::DraftCard), ask_cfg.clone(), expected) {
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
                    json_body::<CreateCardReq>(&mut request)
                else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(expected) = echoed_revision(&request) else {
                    respond_status(request, 400);
                    continue;
                };
                match study.ask_create(req, expected) {
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
                let Some(body) = json_body::<Body>(&mut request)
                else {
                    respond_status(request, 400);
                    continue;
                };
                // A bare name duplicated across containers must 400, not
                // guess: this endpoint gates progression on the result.
                let resolved = catalog
                    .resolve_path(body.deck.clone())
                    .flatten()
                    .zip(catalog.decks_root());
                let Some((path, decks_root)) = resolved else {
                    respond_status(request, 400);
                    continue;
                };
                match study.exam_start(path, decks_root, ask_cfg.clone()) {
                    None => respond_status(request, 503),
                    Some(Transition::Done(dto)) => respond_json(request, &dto),
                    Some(Transition::Rejected) => respond_status(request, 409),
                    Some(Transition::FlushFailed) => respond_status(request, 500),
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
                let body: Option<Body> = json_body::<Body>(&mut request);
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
                let body: Option<Body> = json_body::<Body>(&mut request);
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
                None => respond_status(request, 503),
                Some(Transition::Done(dto)) => respond_json(request, &dto),
                Some(Transition::Rejected) | Some(Transition::FlushFailed) => {
                    respond_status(request, 500)
                }
            },
            (Method::Post, "/api/augment/open") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let Some(body) = json_body::<Body>(&mut request)
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
                    Some(Transition::Rejected) => respond_status(request, 409),
                    Some(Transition::FlushFailed) => respond_status(request, 500),
                    Some(Transition::Done(dto)) => respond_json(request, &dto),
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
                let body: Option<Body> = json_body::<Body>(&mut request);
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
                let body: Option<Body> = json_body::<Body>(&mut request);
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
                None => respond_status(request, 503),
                Some(Transition::Done(dto)) => respond_json(request, &dto),
                Some(Transition::Rejected) | Some(Transition::FlushFailed) => {
                    respond_status(request, 500)
                }
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
                let body: Option<Body> = json_body::<Body>(&mut request);
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
                let body: Option<Body> = json_body::<Body>(&mut request);
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
                None => respond_status(request, 503),
                Some(Transition::Done(dto)) => respond_json(request, &dto),
                Some(Transition::Rejected) | Some(Transition::FlushFailed) => {
                    respond_status(request, 500)
                }
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
