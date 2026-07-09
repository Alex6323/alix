//! HTTP round-trip tests against the *real* `serve::run_review` loop — no
//! subprocess, no mock server. [`spawn_test_server`] binds a real `tiny_http`
//! server on an OS-assigned loopback port, backed by a temp store and a small
//! fixture deck, and runs the actual dispatch loop on a background thread;
//! [`http`] is a tiny `std`-only HTTP/1.1 client (`Connection: close`, so a
//! plain `read_to_end` sees the whole response). This is the highest-value
//! coverage path in the whole crate — the endpoint match in `run_review` was
//! otherwise driven only in-process (`src/serve/tests.rs`), never over the
//! wire.
//!
//! Every test gets its own tempdir, its own port, and its own [`Guard`] that
//! stops the server and joins its thread on drop — so tests can run
//! concurrently (the default `cargo test` behavior) without leaking servers
//! into each other.

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::Arc,
    thread,
};

use alix::{
    config::Config,
    deck::Deck,
    recent::RecentDecks,
    scheduler::Fsrs,
    serve::{self, CardsBuild, PairInfo, ReviewOptions, SelectOptions, SessionBuild, WalkBuild},
    session::{Session, SessionOptions, now_ms},
    store::Store,
};
use anyhow::{Result, anyhow};
use tempfile::TempDir;
use tiny_http::Server;

/// A parsed HTTP response: status code, header name → value (last-wins on a
/// repeated name, which none of these endpoints send), and the raw body.
struct HttpResp {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl HttpResp {
    /// Case-insensitive header lookup (HTTP header names aren't case-sensitive).
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Sends one HTTP/1.1 request over a fresh `TcpStream` and parses the
/// response. `Connection: close` is always sent, so the server closes the
/// socket after replying and a plain `read_to_end` captures the whole
/// response without needing to track `Content-Length` on the way in.
fn http(base: &str, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> HttpResp {
    let host = base
        .strip_prefix("http://")
        .expect("spawn_test_server's base is always an http:// URL");
    let mut stream = TcpStream::connect(host).expect("connect to the test server");

    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    stream
        .write_all(head.as_bytes())
        .expect("write the request head");
    stream.write_all(body).expect("write the request body");

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .expect("read the response to EOF");
    parse_response(&raw)
}

/// Splits a raw response on the first blank line and parses the status line
/// and headers preceding it.
fn parse_response(raw: &[u8]) -> HttpResp {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("the response has a header/body separator");
    let (head, rest) = raw.split_at(split);
    let body = rest[4..].to_vec();

    let head = String::from_utf8_lossy(head);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_string(), value.trim().to_string());
        }
    }
    HttpResp {
        status,
        headers,
        body,
    }
}

/// Stops a spawned test server and joins its thread on drop, so a test can
/// never leak a listening server or a hung background thread into the rest of
/// the suite. `unblock()` is tiny_http's own one-shot stop signal — queued
/// rather than polled, so calling it is race-free regardless of what
/// `run_review` is doing at the time.
struct Guard {
    server: Arc<Server>,
    handle: Option<thread::JoinHandle<()>>,
    // Keeps the fixture tempdir alive for the server thread's whole lifetime.
    _dir: TempDir,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// A minimal single-card fixture deck, just enough to make `run_review`'s
/// closures do real work if a test picks it via `/api/select`.
const FIXTURE_DECK: &str = "# 2 + 2\n\t4\n";

/// Builds the `run_review` closures over one fixture deck living in `dir`,
/// mirroring (in miniature) what `src/cli/launch.rs` wires up for the real
/// CLI — enough for a test to drive `/api/select`, `/api/browse`, etc. in
/// later tests, not just the deck-agnostic endpoints this task exercises.
fn review_options(base: &str) -> ReviewOptions {
    let config = Config::default();
    ReviewOptions {
        keys: config.keys,
        picker: config.picker,
        browse: config.browse,
        ask: config.ask,
        exam: config.exam,
        ai: config.ai,
        generate: config.generate,
        review: config.review,
        auth: None,
        config_path: None,
        pair: PairInfo {
            url: base.to_string(),
            lan: false,
        },
        scoped: true,
    }
}

/// Spins up a real `run_review` server on an OS-assigned loopback port,
/// backed by a temp store and [`FIXTURE_DECK`], and returns its base URL
/// (`http://127.0.0.1:<port>`) plus a [`Guard`] that stops it on drop.
fn spawn_test_server() -> (String, Guard) {
    let dir = TempDir::new().unwrap();
    let deck_path = dir.path().join("sample.txt");
    std::fs::write(&deck_path, FIXTURE_DECK).unwrap();
    let store_path = dir.path().join("store.json");

    let store = Store::open(&store_path).unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));
    let decks_dir = dir.path().to_path_buf();

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let server = Arc::new(serve::bind(addr).unwrap());
    let port = server
        .server_addr()
        .to_ip()
        .expect("bound to a loopback IP")
        .port();
    let base = format!("http://127.0.0.1:{port}");
    let opts = review_options(&base);
    let retention = opts.review.retention;

    // One deck at a time, exactly like the CLI's own `build_review`/`build_browse`
    // (§`src/cli/launch.rs`) — just without the workspace/topology/virtual-card
    // machinery those add, which no fixture deck here needs yet.
    let build = move |paths: Vec<PathBuf>,
                      _opts: &SelectOptions,
                      store: &Store,
                      recent: &mut RecentDecks|
          -> Result<SessionBuild> {
        let path = paths
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("no deck selected"))?;
        let deck = Deck::load(&path)?;
        let subject = deck.subject.clone();
        let session = Session::new(
            deck.cards,
            store,
            Box::new(Fsrs::new(retention)),
            SessionOptions::default(),
            now_ms(),
        );
        recent.record(std::slice::from_ref(&path), now_ms());
        let _ = recent.save();
        Ok(SessionBuild {
            session,
            label: subject.clone(),
            decks: HashMap::from([(subject, path)]),
            links: HashMap::new(),
            source_roots: HashMap::new(),
            source_bases: HashMap::new(),
            topology_name: None,
        })
    };
    let build_walk = |_paths: &[PathBuf]| -> Result<Option<WalkBuild>> { Ok(None) };
    let build_browse = |paths: Vec<PathBuf>, recent: &mut RecentDecks| -> Result<CardsBuild> {
        let path = paths
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("no deck selected"))?;
        let deck = Deck::load(&path)?;
        recent.record(std::slice::from_ref(&path), now_ms());
        let _ = recent.save();
        Ok(CardsBuild {
            cards: deck.cards,
            label: deck.subject.clone(),
            decks: HashMap::from([(deck.subject, path)]),
        })
    };
    let store_for = move |_paths: &[PathBuf]| -> Result<Store> { Ok(Store::open(&store_path)?) };

    let stop_handle = Arc::clone(&server);
    let handle = thread::spawn(move || {
        let _ = serve::run_review(
            store,
            recent,
            decks_dir,
            server,
            opts,
            build,
            build_walk,
            build_browse,
            store_for,
        );
    });

    (
        base,
        Guard {
            server: stop_handle,
            handle: Some(handle),
            _dir: dir,
        },
    )
}

#[test]
fn get_api_version_returns_200_json_with_a_version_field() {
    let (base, _guard) = spawn_test_server();

    let resp = http(&base, "GET", "/api/version", &[], &[]);

    assert_eq!(200, resp.status);
    assert_eq!(
        Some("application/json; charset=utf-8"),
        resp.header("Content-Type")
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(body.get("version").is_some(), "body: {body}");
}
