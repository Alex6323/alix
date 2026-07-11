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
    ffi::OsString,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::Duration,
};

use alix::{
    assemble::{AssembleConfig, Pacing},
    config::{Audience, Config},
    parser,
    recent::RecentDecks,
    serve::{self, PairInfo, ReviewOptions},
    store::Store,
};
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
    // Keeps the fixture tempdir alive for the server thread's whole lifetime;
    // also lets a test reach into the fixture's files (e.g. a workspace's own
    // `progress.json`) via `dir()`.
    dir: TempDir,
}

impl Guard {
    /// The fixture's decks dir — the same path passed to the server as
    /// `decks_dir`, so a test can locate files it wrote there (or that a
    /// session wrote, like a workspace's own store).
    fn dir(&self) -> &Path {
        self.dir.path()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(handle) = self.handle.take() {
            // Propagate a server-thread panic instead of swallowing it —
            // otherwise a bug in `run_review` would fail silently, with the
            // test that triggered it reporting green. `thread::panicking()`
            // skips the resume when the current thread (the test itself) is
            // already unwinding, so this doesn't turn one panic into a
            // double-panic abort.
            if let Err(e) = handle.join()
                && !thread::panicking()
            {
                std::panic::resume_unwind(e);
            }
        }
    }
}

/// A minimal two-card fixture deck — enough for a grade→next-state sequence
/// (grading the first card away still leaves the session in `"review"` phase
/// on the second, rather than jumping straight to `"done"`) — and enough to
/// make `run_review`'s store resolution (`assemble::store_for`, via
/// `cfg.instance_store`) do real work if a test picks it via `/api/select`.
const FIXTURE_DECK: &str = "# 2 + 2\n\t4\n\n# 3 + 3\n\t6\n";

/// Builds the `run_review` options over one fixture deck living in `dir`,
/// mirroring (in miniature) what `src/cli/launch.rs` wires up for the real
/// CLI — enough for a test to drive `/api/select`, `/api/browse`, etc. in
/// later tests, not just the deck-agnostic endpoints exercised here.
/// `auth` mirrors `ReviewOptions::auth`: `None` leaves `/api/*` open, `Some`
/// requires that token.
fn review_options(base: &str, auth: Option<String>) -> ReviewOptions {
    let config = Config::default();
    ReviewOptions {
        keys: config.keys,
        picker: config.picker,
        browse: config.browse,
        exam: config.exam,
        ai: config.ai,
        generate: config.generate,
        // The adult default — the same wiring `src/cli/launch.rs` uses. A kids
        // server differs only in which page `/` serves and the tutor's voice;
        // every `/api/*` route below is audience-agnostic.
        audience: config.serve.audience,
        auth,
        config_path: None,
        pair: PairInfo {
            url: base.to_string(),
            lan: false,
        },
        scoped: true,
        // Callers always overwrite this via a `..` struct-update once they
        // know the fixture's own store path — see `spawn_test_server_fixture`
        // / `spawn_full_server`.
        cfg: AssembleConfig {
            review: config.review,
            ask: config.ask,
            trace_auto_grade: false,
            pacing: Pacing {
                max_new: 10,
                limit: None,
            },
            instance_store: None,
        },
    }
}

/// Spins up a real `run_review` server on an OS-assigned loopback port,
/// backed by a temp store and [`FIXTURE_DECK`], and returns its base URL
/// (`http://127.0.0.1:<port>`) plus a [`Guard`] that stops it on drop. `/api/*`
/// is open (no token) — see [`spawn_test_server_with`] for a guarded instance.
fn spawn_test_server() -> (String, Guard) {
    spawn_test_server_with(None)
}

/// Like [`spawn_test_server`], but requires `token` (when `Some`) on `/api/*`,
/// exactly like a real `--lan`/`--token` launch — for exercising the 401 path
/// over real HTTP.
fn spawn_test_server_with(token: Option<&str>) -> (String, Guard) {
    spawn_test_server_fixture(token, |_dir| {})
}

/// Like [`spawn_test_server_with`], but runs `extra` against the decks dir
/// right after [`FIXTURE_DECK`] is written and before the server starts —
/// lets a test add its own fixture files (e.g. a workspace folder) alongside
/// `sample.txt`.
fn spawn_test_server_fixture(token: Option<&str>, extra: impl FnOnce(&Path)) -> (String, Guard) {
    let dir = TempDir::new().unwrap();
    let deck_path = dir.path().join("sample.txt");
    std::fs::write(&deck_path, FIXTURE_DECK).unwrap();
    extra(dir.path());
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
    let opts = review_options(&base, token.map(str::to_string));
    // `/api/select` now runs the real classifier/assembler (`assemble::select`)
    // instead of a hand-rolled stub — give it the same pacing default the old
    // stub's `session_options` used (`max_new: 10`), and pin the instance store
    // to this fixture's own file.
    let opts = ReviewOptions {
        cfg: AssembleConfig {
            trace_auto_grade: false,
            pacing: Pacing {
                max_new: 10,
                limit: None,
            },
            instance_store: Some(store_path),
            ..opts.cfg
        },
        ..opts
    };

    let stop_handle = Arc::clone(&server);
    let handle = thread::spawn(move || {
        let _ = serve::run_review(store, recent, decks_dir, server, opts);
    });

    (
        base,
        Guard {
            server: stop_handle,
            handle: Some(handle),
            dir,
        },
    )
}

/// Five single-line, all-distinct-answer cards — enough sibling pool for a
/// real offline multiple-choice question (`src/choice.rs::build` needs
/// `NUM_OPTIONS - 1 == 3` distinct distractors) without any AI augmentation.
const CHOICE_DECK: &str =
    "# 1 + 1\n\t2\n\n# 2 + 2\n\t4\n\n# 3 + 3\n\t6\n\n# 4 + 4\n\t8\n\n# 5 + 5\n\t10\n";

/// [`CHOICE_DECK`]'s authored front → back, so a test can find which option is
/// correct without hard-coding a queue order `choice::build`'s sibling-pool
/// sampling doesn't actually guarantee.
fn choice_answer(front: &str) -> &'static str {
    match front {
        "1 + 1" => "2",
        "2 + 2" => "4",
        "3 + 3" => "6",
        "4 + 4" => "8",
        "5 + 5" => "10",
        other => panic!("not a CHOICE_DECK front: {other}"),
    }
}

/// A two-hop predict-and-verify trace over [`TRACE_SOURCE`], for the walk and
/// (trace) exam endpoint families — mirrors `src/serve/tests.rs`'s
/// `walk_deck` fixture in miniature (kept to two hops; that's enough to
/// exercise a hop transition without a bigger fixture to maintain).
const TRACE_DECK: &str = "% trace: how it works\n\
% source: source.txt\n\
# Predict the first hop\n\
\t% given: line — the input line\n\
\tit reads the first line\n\
\t% at: 1\n\
# Predict the second hop\n\
\tit reads line two\n\
\t% at: 2\n";
const TRACE_SOURCE: &str = "first\nsecond\nthird\n";

/// Richer than [`spawn_test_server`]: the same open (no-token) server, but its
/// decks dir also carries [`CHOICE_DECK`] (pre-seeded "seen" in the store, so
/// a Recognize-depth session quizzes it via the sibling-pool multiple-choice
/// builder instead of the AI-only acquire on-ramp — see `current_question`,
/// `src/serve/dto.rs`) and [`TRACE_DECK`] (routed to a real `Walk` by the real
/// classifier in `assemble::select`, for the walk and trace-exam families).
///
/// `ask_command`, when `Some`, points `[ask] command` at a fake CLI — see this
/// module's `fake_reply` — so a walk picked here auto-grades
/// (`AssembleConfig::trace_auto_grade`) instead of self-grading; `None` keeps every AI
/// path off (self-graded walk, no augmentation), which is what every non-AI
/// test in this family wants.
fn spawn_full_server(ask_command: Option<&Path>) -> (String, Guard) {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("sample.txt"), FIXTURE_DECK).unwrap();
    std::fs::write(dir.path().join("choice.txt"), CHOICE_DECK).unwrap();
    std::fs::write(dir.path().join("trace.txt"), TRACE_DECK).unwrap();
    std::fs::write(dir.path().join("source.txt"), TRACE_SOURCE).unwrap();
    let store_path = dir.path().join("store.json");

    // Pre-seed the choice deck's cards as "seen" (a store entry, no
    // `recognized_ms`) so a Recognize session quizzes them via
    // `choice::build`'s offline sibling-pool sampler rather than the
    // AI-distractor-only acquire on-ramp (`choice::recognition_question`).
    {
        let mut seed = Store::open(&store_path).unwrap();
        for card in parser::parse_str("choice.txt", CHOICE_DECK).unwrap() {
            seed.get_or_insert(card.id(), 0);
        }
        seed.save().unwrap();
    }

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
    let mut opts = review_options(&base, None);
    if let Some(cmd) = ask_command {
        opts.cfg.ask.command = cmd.to_str().unwrap().to_string();
    }
    // A picked trace deck now walks (predict → verify) via the real
    // classifier/assembler (`assemble::select`) instead of a hand-rolled
    // `build_walk` stub — `trace_auto_grade` reproduces what this fixture's
    // old stub computed itself (`ask_command.is_some()`).
    let auto_grade = ask_command.is_some();
    let opts = ReviewOptions {
        cfg: AssembleConfig {
            trace_auto_grade: auto_grade,
            pacing: Pacing {
                max_new: 10,
                limit: None,
            },
            instance_store: Some(store_path),
            ..opts.cfg
        },
        ..opts
    };

    let stop_handle = Arc::clone(&server);
    let handle = thread::spawn(move || {
        let _ = serve::run_review(store, recent, decks_dir, server, opts);
    });

    (
        base,
        Guard {
            server: stop_handle,
            handle: Some(handle),
            dir,
        },
    )
}

/// Serializes tests that write + exec a fake CLI: a concurrent fork would
/// inherit the briefly write-open script fd and fail `exec` with `ETXTBSY` —
/// the same hazard `src/testutil.rs::exec_lock` guards against for the lib's
/// own AI tests. That helper is `pub(crate)` (crate-private) and therefore
/// unreachable from this integration test, so it's replicated here in
/// miniature (this file's own fake-CLI setup).
static EXEC_LOCK: Mutex<()> = Mutex::new(());

fn exec_lock() -> std::sync::MutexGuard<'static, ()> {
    EXEC_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// Writes a fake `claude` CLI at `<dir>/fake-claude` that drains stdin (the
/// prompt always arrives that way for the Claude backend — draining first
/// avoids a broken-pipe race) then prints `reply` verbatim, and returns its
/// path. Mirrors `src/testutil.rs::fake_reply` in miniature (see
/// `EXEC_LOCK`'s doc for why that one isn't reachable from here).
fn fake_reply(dir: &Path, reply: &str) -> PathBuf {
    let out = dir.join("fake-reply");
    std::fs::write(&out, reply).unwrap();
    let path = dir.join("fake-claude");
    std::fs::write(
        &path,
        // The script pins its own `PATH` before doing anything else: this
        // test's `EXEC_LOCK` is a *different* lock than `PATH_LOCK` (see
        // below), so a test here can spawn this script concurrently with a
        // `with_empty_path` test that has pinned the process `PATH` to an
        // empty dir. Without a hardcoded `PATH`, `cat` would fail to resolve
        // in that window, skipping the `cat >/dev/null` stdin drain and
        // reopening the EPIPE race this script exists to avoid.
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\ncat {}\n",
            out.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Polls `GET path` (bounded: up to 5s, 20ms apart) until `done` accepts the
/// parsed body, returning it — for the handful of endpoints that kick a
/// background job (`thinking`/a phase change) rather than answering inline.
/// Panics (failing the test) rather than looping forever if a job never
/// settles.
fn poll_until(
    base: &str,
    path: &str,
    done: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    for _ in 0..250 {
        let resp = http(base, "GET", path, &[], &[]);
        let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        if done(&body) {
            return body;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("{path} did not settle within the poll budget");
}

/// Serializes tests that pin `PATH` (magic-wormhole's install-hint tests):
/// `wormhole` is installed on some dev machines but not in CI, so
/// `POST /api/share`/`/api/receive` must see a deliberately empty `PATH` for
/// the call to deterministically hit the "not installed" spawn-failure arm
/// either way. This only serializes the two `with_empty_path` tests against
/// *each other* — it does not make the underlying `env::set_var`/`remove_var`
/// calls sound; see [`PathGuard`] for the honest picture of what risk that
/// leaves.
static PATH_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that restores the process `PATH` (present-or-absent) on drop —
/// including when a panic unwinds through the holding scope. Without this,
/// an assertion failing inside [`with_empty_path`]'s closure would skip a
/// plain post-call restore and leave `PATH` pinned to the empty tempdir for
/// the rest of this test binary's process: tests share one process, and the
/// harness catches the panic in a higher frame, so nothing else would put
/// `PATH` back before every later subprocess-spawning test ran.
struct PathGuard {
    original: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        // SAFETY: not actually sound in general — `std::env::set_var`/
        // `remove_var` are unsafe because Unix-likes have no thread-safe way
        // to *read* the environment, so any concurrent reader anywhere in
        // the process (not just another writer) can race a write. `cargo
        // test` runs this suite's tests concurrently, and other tests do
        // read the environment while this guard is alive elsewhere in the
        // binary. Two reader classes matter here: in-process readers (e.g.
        // every `TempDir::new()` reads `TMPDIR` via `env::var_os`), and child
        // processes spawned while `PATH` is pinned — a spawn resolves its
        // interpreter/binary through the `PATH` it inherits at spawn time
        // (see `src/ask.rs`'s `Command::new`, which inherits the parent
        // environment), so a fake-CLI test spawning concurrently (under its
        // own, different lock — see `EXEC_LOCK`) could land inside this
        // window and fail to resolve its own interpreter. `PATH_LOCK` only
        // keeps the two `with_empty_path` tests from mutating `PATH` at the
        // same time as each other; it does nothing for either reader class.
        // The risk is accepted here rather than eliminated: the mutated
        // window is a handful of instructions, this is test-only code, the
        // race is benign in practice on Linux/glibc (a reader observes
        // either the old or the new value, not a torn one), and avoiding it
        // for real would need subprocess isolation this crate has no
        // dependency budget for. (The fake-CLI script itself is additionally
        // hardened against this — see `fake_reply`'s hardcoded `PATH`.)
        match self.original.take() {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }
    }
}

/// Runs `f` with `PATH` set to `dir` (a directory that deliberately has no
/// `wormhole` executable) for the call's duration, restoring the original
/// `PATH` — even if `f` panics — via [`PathGuard`]'s drop.
fn with_empty_path<R>(dir: &Path, f: impl FnOnce() -> R) -> R {
    let lock = PATH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let original = std::env::var_os("PATH");
    let _guard = PathGuard {
        original,
        _lock: lock,
    };
    // SAFETY: see `PathGuard::drop` — same accepted, documented risk (races
    // concurrent environment *readers* elsewhere in this process; not fully
    // eliminated by `PATH_LOCK`).
    unsafe { std::env::set_var("PATH", dir) };
    f()
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

/// `POST`s a JSON body — the shape every mutating `/api/*` endpoint expects
/// (`Content-Type` doesn't gate anything server-side, but sending it is
/// honest about what's on the wire).
fn post_json(base: &str, path: &str, json: &str) -> HttpResp {
    http(
        base,
        "POST",
        path,
        &[("Content-Type", "application/json")],
        json.as_bytes(),
    )
}

/// Selects [`FIXTURE_DECK`] (by its fixed file name, `sample.txt`) and returns
/// the resulting `StateDto` response — the common first step of every
/// review-loop test below.
fn select_fixture(base: &str) -> HttpResp {
    post_json(base, "/api/select", r#"{"deck":"sample.txt"}"#)
}

#[test]
fn get_api_decks_returns_200_with_the_fixture_deck_in_the_catalog() {
    let (base, _guard) = spawn_test_server();

    let resp = http(&base, "GET", "/api/decks", &[], &[]);

    assert_eq!(200, resp.status);
    assert_eq!(
        Some("application/json; charset=utf-8"),
        resp.header("Content-Type")
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    // A loose deck (not in a workspace/folder) always lands in `recent` —
    // see `deck_catalog` in `src/serve/catalog.rs`.
    let recent = body["recent"].as_array().expect("recent is an array");
    assert!(
        recent.iter().any(|d| d["name"] == "sample.txt"),
        "body: {body}"
    );
}

// ── Decks catalog: workspace rows vs. deck rows ──────────────────────────
//
// `spawn_test_server`'s fixture is a single loose deck — no workspace
// anywhere — so none of these tests can use it. `write_animals_workspace`
// adds a real workspace (an `alix.toml` manifest + two member decks)
// alongside `sample.txt` via `spawn_test_server_fixture`, so `/api/decks`
// actually has a group row to exercise.

/// Writes a workspace `animals/` (with `alix.toml`, so it registers as
/// `is_workspace` — see `workspace::is_workspace`) holding two tiny member
/// decks, into the fixture's decks dir.
fn write_animals_workspace(dir: &Path) {
    let ws = dir.join("animals");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Animals\"\n").unwrap();
    std::fs::write(ws.join("one.txt"), "# q1\n\ta1\n").unwrap();
    std::fs::write(ws.join("two.txt"), "# q2\n\ta2\n").unwrap();
}

#[test]
fn get_api_decks_lists_a_workspace_with_its_member_decks() {
    let (base, _guard) = spawn_test_server_fixture(None, write_animals_workspace);

    let resp = http(&base, "GET", "/api/decks", &[], &[]);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let workspaces = body["workspaces"]
        .as_array()
        .expect("workspaces is an array");
    let animals = workspaces
        .iter()
        .find(|w| w["name"] == "animals")
        .unwrap_or_else(|| panic!("no `animals` workspace row: body: {body}"));
    assert_eq!(true, animals["is_workspace"], "row: {animals}");
    let members = animals["members"].as_array().expect("members is an array");
    assert!(!members.is_empty(), "row: {animals}");
    for m in members {
        assert!(
            m["name"].as_str().is_some_and(|n| !n.is_empty()),
            "a member has an empty name: {m}"
        );
    }
}

/// The invariant real clients depend on: every member `name` `/api/decks`
/// reports must actually select (200, a review `StateDto`). This drives the
/// real server end to end — the real name resolution (`resolve_row`,
/// `src/serve/catalog.rs`) over qualified `<workspace>/<file>` keys, then the
/// real `assemble::select` for each member's `/api/select` — not a stub; the
/// companion unit test for the folder-bail itself is
/// `select_rejects_a_folder_of_decks` in `src/assemble.rs`.
#[test]
fn every_member_deck_name_from_api_decks_is_selectable() {
    let (base, _guard) = spawn_test_server_fixture(None, write_animals_workspace);

    let decks_resp = http(&base, "GET", "/api/decks", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&decks_resp.body).unwrap();
    let animals = body["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "animals")
        .unwrap_or_else(|| panic!("no `animals` workspace row: body: {body}"));
    let members = animals["members"].as_array().unwrap();
    assert!(!members.is_empty(), "row: {animals}");

    for m in members {
        let name = m["name"].as_str().expect("member name is a string");
        assert_eq!(
            true, m["selectable"],
            "member {name:?} should report selectable: true — row: {m}"
        );
        let req = serde_json::json!({ "deck": name }).to_string();
        let resp = post_json(&base, "/api/select", &req);
        assert_eq!(
            200,
            resp.status,
            "selecting member {name:?} failed: {}",
            String::from_utf8_lossy(&resp.body)
        );
        let sel: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(
            "review", sel["kind"],
            "member {name:?} did not select into a review session: {sel}"
        );
    }
}

/// A workspace row's `name` (`"animals"`) is a *resolution* key — valid for
/// `/api/reset` — but a review session is exactly one deck file, so
/// `/api/select` rejects a group row. The authoritative rule and its error
/// message live in `assemble::select` and are unit-tested there
/// (`select_rejects_a_folder_of_decks`); this test only pins the
/// client-visible status code — `/api/select` now runs the real classifier,
/// so the 400 here comes from `select`'s own "is a folder" bail.
#[test]
fn a_workspace_row_name_is_not_selectable() {
    let (base, _guard) = spawn_test_server_fixture(None, write_animals_workspace);

    let decks_resp = http(&base, "GET", "/api/decks", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&decks_resp.body).unwrap();
    let animals = body["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "animals")
        .unwrap_or_else(|| panic!("no `animals` workspace row: body: {body}"));
    assert_eq!(
        false, animals["selectable"],
        "a workspace row must report selectable: false — row: {animals}"
    );

    let resp = post_json(&base, "/api/select", r#"{"deck":"animals"}"#);

    assert_eq!(400, resp.status);
}

/// The store-scoping policy `assemble::store_for` implements, end to end: a
/// workspace member's grade lands in the workspace's own `progress.json`
/// (`workspace::store_path`'s default), not the served instance's global
/// store. The old `store_for` closure this harness stubbed out ignored its
/// `paths` argument and always opened the instance store, so this is the
/// first test able to exercise the real precedence (now wired via
/// `run_review` → `cfg.instance_store` → `assemble::store_for`).
#[test]
fn grading_a_workspace_member_writes_the_workspace_store_not_the_instance_store() {
    let (base, guard) = spawn_test_server_fixture(None, write_animals_workspace);
    let ws_store = guard.dir().join("animals").join("progress.json");
    assert!(!ws_store.exists(), "no review has happened yet");

    let decks_resp = http(&base, "GET", "/api/decks", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&decks_resp.body).unwrap();
    let animals = body["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "animals")
        .unwrap_or_else(|| panic!("no `animals` workspace row: body: {body}"));
    let member = animals["members"][0]["name"]
        .as_str()
        .expect("member name is a string");

    let select_req = serde_json::json!({ "deck": member }).to_string();
    let resp = post_json(&base, "/api/select", &select_req);
    assert_eq!(200, resp.status, "select {member:?} failed");

    let resp = post_json(&base, "/api/grade", r#"{"grade":"passed"}"#);
    assert_eq!(200, resp.status);

    assert!(
        ws_store.exists(),
        "the workspace's own progress.json must receive the grade write"
    );
    assert!(
        !guard.dir().join("store.json").exists(),
        "the instance store must not receive a workspace member's progress"
    );
}

#[test]
fn post_api_select_returns_a_review_state_for_the_fixture_deck() {
    let (base, _guard) = spawn_test_server();

    let resp = select_fixture(&base);

    assert_eq!(200, resp.status);
    assert_eq!(
        Some("application/json; charset=utf-8"),
        resp.header("Content-Type")
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["kind"], "body: {body}");
    assert_eq!("review", body["phase"], "body: {body}");
    assert_eq!("2 + 2", body["card"]["front"], "body: {body}");
    assert_eq!("flip", body["mode"], "body: {body}");
    assert_eq!("recall", body["depth"], "body: {body}");
    assert_eq!(2, body["remaining"], "body: {body}");
    assert_eq!(2, body["initial"], "body: {body}");
}

#[test]
fn get_api_state_reflects_the_active_session_after_select() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = http(&base, "GET", "/api/state", &[], &[]);

    assert_eq!(200, resp.status);
    assert_eq!(
        Some("application/json; charset=utf-8"),
        resp.header("Content-Type")
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["phase"], "body: {body}");
    assert_eq!("2 + 2", body["card"]["front"], "body: {body}");
}

#[test]
fn post_api_grade_passed_returns_the_next_state_dto() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_json(&base, "/api/grade", r#"{"grade":"passed"}"#);

    assert_eq!(200, resp.status);
    assert_eq!(
        Some("application/json; charset=utf-8"),
        resp.header("Content-Type")
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    // The fixture's second card, not "done" — the two-card deck exists
    // precisely so a grade advances within the session instead of ending it.
    assert_eq!("review", body["phase"], "body: {body}");
    assert_eq!("3 + 3", body["card"]["front"], "body: {body}");
    assert_eq!(1, body["passed"], "body: {body}");
    assert_eq!(1, body["remaining"], "body: {body}");
}

#[test]
fn get_api_doctor_returns_200_with_doctor_rows() {
    let (base, _guard) = spawn_test_server();

    let resp = http(&base, "GET", "/api/doctor", &[], &[]);

    assert_eq!(200, resp.status);
    assert_eq!(
        Some("application/json; charset=utf-8"),
        resp.header("Content-Type")
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let rows = body["rows"].as_array().expect("rows is an array");
    assert!(!rows.is_empty(), "body: {body}");
    assert!(rows.iter().any(|r| r["name"] == "config"), "body: {body}");
}

#[test]
fn get_api_pair_returns_200_with_the_pairing_url() {
    let (base, _guard) = spawn_test_server();

    let resp = http(&base, "GET", "/api/pair", &[], &[]);

    assert_eq!(200, resp.status);
    assert_eq!(
        Some("application/json; charset=utf-8"),
        resp.header("Content-Type")
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    // The test harness's `review_options` builds a localhost, non-`--lan`
    // `PairInfo` — no other device could reach it, so no QR is rendered.
    assert_eq!(base, body["url"], "body: {body}");
    assert_eq!(false, body["lan"], "body: {body}");
    assert!(body["svg"].is_null(), "body: {body}");
}

#[test]
fn a_missing_bearer_token_yields_401_with_an_empty_body() {
    let (base, _guard) = spawn_test_server_with(Some("secret"));

    let resp = http(&base, "GET", "/api/state", &[], &[]);

    assert_eq!(401, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

#[test]
fn the_correct_bearer_token_is_accepted() {
    let (base, _guard) = spawn_test_server_with(Some("secret"));

    let resp = http(
        &base,
        "GET",
        "/api/state",
        &[("Authorization", "Bearer secret")],
        &[],
    );

    assert_eq!(200, resp.status);
}

#[test]
fn a_query_token_is_accepted_as_a_fallback_when_no_bearer_is_sent() {
    let (base, _guard) = spawn_test_server_with(Some("secret"));

    let resp = http(&base, "GET", "/api/state?token=secret", &[], &[]);

    assert_eq!(200, resp.status);
}

#[test]
fn a_wrong_bearer_token_yields_401() {
    let (base, _guard) = spawn_test_server_with(Some("secret"));

    let resp = http(
        &base,
        "GET",
        "/api/state",
        &[("Authorization", "Bearer wrong")],
        &[],
    );

    assert_eq!(401, resp.status);
}

#[test]
fn post_api_grade_with_a_malformed_body_yields_400() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    // Neither the `{grade}` nor the `{covered, total}` shape `/api/grade`
    // documents (`docs/API.md` §5) — valid JSON, but not a body it accepts.
    let resp = post_json(&base, "/api/grade", r#"{"nonsense":true}"#);

    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

#[test]
fn post_api_grade_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/grade", r#"{"grade":"passed"}"#);

    assert_eq!(409, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

#[test]
fn get_api_nope_yields_404() {
    let (base, _guard) = spawn_test_server();

    let resp = http(&base, "GET", "/api/nope", &[], &[]);

    assert_eq!(404, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

#[test]
fn get_img_with_an_unknown_key_yields_404() {
    let (base, _guard) = spawn_test_server();

    let resp = http(&base, "GET", "/img/badkey", &[], &[]);

    assert_eq!(404, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

// ── Browse ──────────────────────────────────────────────────────────────

#[test]
fn post_api_browse_returns_a_browse_dto_with_the_fixture_cards() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/browse", r#"{"deck":"sample.txt"}"#);

    assert_eq!(200, resp.status);
    assert_eq!(
        Some("application/json; charset=utf-8"),
        resp.header("Content-Type")
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("browse", body["phase"], "body: {body}");
    let cards = body["cards"].as_array().expect("cards is an array");
    assert_eq!(2, cards.len(), "body: {body}");
    assert_eq!("2 + 2", cards[0]["front"], "body: {body}");
}

#[test]
fn post_api_browse_with_an_unknown_deck_yields_400() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/browse", r#"{"deck":"nope.txt"}"#);

    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

// ── Deck topology ───────────────────────────────────────────────────────

#[test]
fn post_api_deck_topology_reports_the_fixture_decks_due_count() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/deck-topology", r#"{"deck":"sample.txt"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(
        body["topologies"].as_array().unwrap().is_empty(),
        "no augmentation was ever generated: body: {body}"
    );
    // Both fixture cards are new (a fresh store), and a new card counts as
    // reviewable (`session::is_reviewable`'s `None => true` arm).
    assert_eq!(2, body["deck_due"], "body: {body}");
}

#[test]
fn post_api_deck_topology_with_an_unknown_deck_still_returns_the_empty_default_dto() {
    // `/api/deck-topology` never errors (docs/API.md §5) — an unresolvable
    // name still gets 200 with the empty default, not a 400.
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/deck-topology", r#"{"deck":"nope.txt"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(
        body["topologies"].as_array().unwrap().is_empty(),
        "body: {body}"
    );
    assert_eq!(0, body["deck_due"], "body: {body}");
}

// ── Reset ───────────────────────────────────────────────────────────────

#[test]
fn post_api_reset_clears_the_fixture_decks_progress() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);
    // Grade the first card so it has stored progress to clear.
    post_json(&base, "/api/grade", r#"{"grade":"passed"}"#);

    let resp = post_json(&base, "/api/reset", r#"{"deck":"sample.txt"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("sample.txt", body["deck"], "body: {body}");
    assert_eq!(1, body["cards_cleared"], "body: {body}");
}

#[test]
fn post_api_reset_with_an_unknown_deck_yields_400() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/reset", r#"{"deck":"nope.txt"}"#);

    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

// ── Import ──────────────────────────────────────────────────────────────

#[test]
fn post_api_import_lands_a_txt_deck_and_reports_its_card_count() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(
        &base,
        "/api/import",
        r##"{"name":"extra.txt","text":"# f\n\tb\n"}"##,
    );

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("extra.txt", body["deck"], "body: {body}");
    assert_eq!(1, body["cards"], "body: {body}");
}

#[test]
fn post_api_import_converts_a_tsv_upload_to_a_deck() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(
        &base,
        "/api/import",
        r#"{"name":"cards.tsv","text":"Front1\tBack1\nFront2\tBack2\n"}"#,
    );

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(2, body["cards"], "body: {body}");
    assert!(
        body["deck"].as_str().unwrap().ends_with(".txt"),
        "body: {body}"
    );
}

#[test]
fn post_api_import_with_an_unrecognized_extension_yields_400() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(
        &base,
        "/api/import",
        r#"{"name":"cards.csv","text":"whatever"}"#,
    );

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_import_with_unparseable_tsv_yields_400() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(
        &base,
        "/api/import",
        r#"{"name":"bad.tsv","text":"no tabs at all here\n"}"#,
    );

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_import_with_a_malformed_body_yields_400() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/import", r#"{"oops":true}"#);

    assert_eq!(400, resp.status);
}

// ── Check (typed evidence, no grade recorded) ────────────────────────────

#[test]
fn post_api_check_reports_a_correct_typed_answer_without_recording_a_grade() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_json(&base, "/api/check", r#"{"lines":["4"]}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(true, body["passed"], "body: {body}");
    let results = body["results"].as_array().unwrap();
    assert_eq!(1, results.len(), "body: {body}");
    assert_eq!("4", results[0]["input"], "body: {body}");
    assert_eq!("4", results[0]["expected"], "body: {body}");
    assert_eq!(true, results[0]["passed"], "body: {body}");

    // Evidence only: the session is still on the same card, ungraded.
    let state = http(&base, "GET", "/api/state", &[], &[]);
    let state_body: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
    assert_eq!("2 + 2", state_body["card"]["front"], "body: {state_body}");
    assert_eq!(0, state_body["passed"], "body: {state_body}");
}

#[test]
fn post_api_check_with_a_wrong_answer_reports_failure() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_json(&base, "/api/check", r#"{"lines":["5"]}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(false, body["passed"], "body: {body}");
    assert_eq!(false, body["results"][0]["passed"], "body: {body}");
}

#[test]
fn post_api_check_with_a_malformed_body_yields_400() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_json(&base, "/api/check", r#"{"nonsense":true}"#);

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_check_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/check", r#"{"lines":["4"]}"#);

    assert_eq!(409, resp.status);
}

// ── Choose (multiple choice, Recognize depth) ────────────────────────────

#[test]
fn post_api_choose_reports_the_correct_index_for_a_recognize_session() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(
        &base,
        "/api/select",
        r#"{"deck":"choice.txt","depth":"recognize"}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("recognize", body["depth"], "body: {body}");
    assert_eq!("choice", body["mode"], "body: {body}");
    let choices = body["choices"]
        .as_array()
        .expect("a recognize session offers choices");
    assert_eq!(4, choices.len(), "body: {body}");
    let front = body["card"]["front"].as_str().unwrap();
    let expected = choice_answer(front);
    let correct_index = choices
        .iter()
        .position(|c| c.as_str() == Some(expected))
        .unwrap_or_else(|| panic!("the correct answer {expected:?} is among {choices:?}"));

    let resp = post_json(
        &base,
        "/api/choose",
        &format!(r#"{{"index":{correct_index}}}"#),
    );

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(correct_index, body["chosen"], "body: {body}");
    assert_eq!(correct_index, body["correct"], "body: {body}");
    assert_eq!(true, body["passed"], "body: {body}");
}

#[test]
fn post_api_choose_with_a_malformed_body_yields_400() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_json(&base, "/api/choose", r#"{"nonsense":true}"#);

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_choose_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/choose", r#"{"index":0}"#);

    assert_eq!(409, resp.status);
}

// ── Skip / acquire / promote / restart / deselect ────────────────────────

#[test]
fn post_api_skip_defers_the_current_card_without_grading_it() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_json(&base, "/api/skip", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("3 + 3", body["card"]["front"], "body: {body}");
    assert_eq!(0, body["passed"], "body: {body}");
    assert_eq!(0, body["failed"], "body: {body}");
    assert_eq!(2, body["remaining"], "body: {body}");
}

#[test]
fn post_api_skip_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/skip", "{}");

    assert_eq!(409, resp.status);
}

#[test]
fn post_api_acquire_acknowledges_a_never_seen_card_without_grading_it() {
    let (base, _guard) = spawn_test_server();
    let select_resp = select_fixture(&base);
    let select_body: serde_json::Value = serde_json::from_slice(&select_resp.body).unwrap();
    assert_eq!(
        true, select_body["acquire"],
        "a brand-new store has never seen this card: {select_body}"
    );

    let resp = post_json(&base, "/api/acquire", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["phase"], "body: {body}");
    // Acquiring records it (cooling ~1 min, floored out of `remaining`) and
    // moves to the other card, rather than grading it.
    assert_eq!("3 + 3", body["card"]["front"], "body: {body}");
    assert_eq!(0, body["passed"], "body: {body}");
    assert_eq!(0, body["failed"], "body: {body}");
    assert_eq!(1, body["remaining"], "body: {body}");
}

#[test]
fn post_api_acquire_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/acquire", "{}");

    assert_eq!(409, resp.status);
}

#[test]
fn post_api_promote_the_current_card_when_it_is_not_virtual_yields_400() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_json(&base, "/api/promote", "{}");

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_promote_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/promote", "{}");

    assert_eq!(409, resp.status);
}

#[test]
fn post_api_restart_rebuilds_the_queue_and_resets_session_stats() {
    let (base, _guard) = spawn_test_server();
    // `cram` makes `restart`'s queue rebuild deterministic regardless of the
    // FSRS interval a "passed" grade schedules — cram serves every non-retired
    // card, due or not (`session::build_queue`).
    post_json(&base, "/api/select", r#"{"deck":"sample.txt","cram":true}"#);
    let grade_resp = post_json(&base, "/api/grade", r#"{"grade":"passed"}"#);
    let grade_body: serde_json::Value = serde_json::from_slice(&grade_resp.body).unwrap();
    assert_eq!(1, grade_body["passed"], "body: {grade_body}");
    assert_eq!("3 + 3", grade_body["card"]["front"], "body: {grade_body}");

    let resp = post_json(&base, "/api/restart", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(2, body["remaining"], "body: {body}");
    assert_eq!(0, body["passed"], "body: {body}");
    assert_eq!("2 + 2", body["card"]["front"], "body: {body}");
}

#[test]
fn post_api_restart_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/restart", "{}");

    assert_eq!(409, resp.status);
}

#[test]
fn post_api_deselect_returns_to_the_picker_state_dto() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_json(&base, "/api/deselect", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["kind"], "body: {body}");
    assert_eq!("select", body["phase"], "body: {body}");
    assert!(body["card"].is_null(), "body: {body}");
}

// ── Augment (open / remove / close — no AI on this path) ─────────────────

#[test]
fn post_api_augment_open_reports_coverage_for_the_fixture_deck() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/augment/open", r#"{"deck":"sample.txt"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("sample.txt", body["deck"], "body: {body}");
    assert_eq!(2, body["cards"], "body: {body}");
    assert!(body["busy"].is_null(), "body: {body}");
    let rows = body["rows"].as_array().unwrap();
    let choices = rows
        .iter()
        .find(|r| r["kind"] == "choices")
        .expect("a choices row");
    assert_eq!(0, choices["covered"], "body: {body}");
    assert_eq!(2, choices["eligible"], "body: {body}");
}

#[test]
fn post_api_augment_open_with_an_unknown_deck_yields_400() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/augment/open", r#"{"deck":"nope.txt"}"#);

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_augment_remove_on_an_empty_cache_still_succeeds_as_a_noop() {
    let (base, _guard) = spawn_test_server();
    post_json(&base, "/api/augment/open", r#"{"deck":"sample.txt"}"#);

    let resp = post_json(&base, "/api/augment/remove", r#"{"target":"choices"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let choices = body["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "choices")
        .unwrap();
    assert_eq!(0, choices["covered"], "body: {body}");
}

#[test]
fn post_api_augment_close_returns_the_picker_state_dto() {
    let (base, _guard) = spawn_test_server();
    post_json(&base, "/api/augment/open", r#"{"deck":"sample.txt"}"#);

    let resp = post_json(&base, "/api/augment/close", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["kind"], "body: {body}");
    assert_eq!("select", body["phase"], "body: {body}");
}

#[test]
fn get_api_augment_with_no_open_screen_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = http(&base, "GET", "/api/augment", &[], &[]);

    assert_eq!(409, resp.status);
}

// ── Exam (start / close on a trace deck — no AI needed for that path;
// grading is additionally covered end-to-end via the fake backend) ───────

#[test]
fn post_api_exam_start_on_a_trace_deck_opens_directly_in_the_answering_phase() {
    // A trace's "exam" is the graded compression, one fixed question — it
    // opens straight into `answering` with nothing in flight
    // (`exam::Sitting::start_trace`), unlike a fact deck's exam, which would
    // need the AI backend to generate questions.
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/exam/start", r#"{"deck":"trace.txt"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("answering", body["phase"], "body: {body}");
    assert_eq!(true, body["is_trace"], "body: {body}");
    assert_eq!("trace.txt", body["deck"], "body: {body}");
    assert_eq!(1, body["total"], "body: {body}");
    assert_eq!(0, body["current"], "body: {body}");
    assert!(body["question"].as_str().is_some(), "body: {body}");
}

#[test]
fn post_api_exam_close_returns_the_picker_state_dto() {
    let (base, _guard) = spawn_full_server(None);
    post_json(&base, "/api/exam/start", r#"{"deck":"trace.txt"}"#);

    let resp = post_json(&base, "/api/exam/close", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["kind"], "body: {body}");
    assert_eq!("select", body["phase"], "body: {body}");
}

#[test]
fn post_api_exam_start_with_an_unknown_deck_yields_400() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/exam/start", r#"{"deck":"nope.txt"}"#);

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_exam_start_on_a_deck_with_no_exam_yields_409() {
    // `sample.txt` declares no `% source:` and isn't a trace — `has_exam()`
    // is false, so it can never be sat.
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/exam/start", r#"{"deck":"sample.txt"}"#);

    assert_eq!(409, resp.status);
}

#[test]
fn get_api_exam_with_no_active_sitting_yields_409() {
    let (base, _guard) = spawn_full_server(None);

    let resp = http(&base, "GET", "/api/exam", &[], &[]);

    assert_eq!(409, resp.status);
}

#[test]
fn exam_grade_on_a_trace_deck_walks_from_answering_to_a_passing_result_via_the_fake_backend() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(
        scripts.path(),
        r#"{"verdict":"pass","feedback":"nice work retracing it","missed":[]}"#,
    );
    let (base, _guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/exam/start", r#"{"deck":"trace.txt"}"#);

    let resp = post_json(
        &base,
        "/api/exam/grade",
        r#"{"text":"it forwards the value hop by hop, first then second"}"#,
    );

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("grading", body["phase"], "body: {body}");

    let body = poll_until(&base, "/api/exam", |b| b["phase"] != "grading");

    assert_eq!("results", body["phase"], "body: {body}");
    assert_eq!(true, body["passed"], "body: {body}");
    let grades = body["grades"].as_array().unwrap();
    assert_eq!(1, grades.len(), "body: {body}");
    assert_eq!("PASS", grades[0]["verdict"], "body: {body}");
}

// ── Walk (a two-hop trace deck) ───────────────────────────────────────────

/// `/api/select` now classifies through the real `assemble::select` (no more
/// per-fixture `build_walk` stub) — this pins that the trace fixture still
/// round-trips as a walk through that real classifier, not a harness replica.
#[test]
fn selecting_a_trace_deck_returns_a_walk_through_the_real_classifier() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/select", r#"{"deck":"trace.txt"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("walk", body["kind"], "body: {body}");
}

#[test]
fn selecting_a_trace_deck_returns_a_walk_dto_not_a_review_state() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/select", r#"{"deck":"trace.txt"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("walk", body["kind"], "body: {body}");
    assert_eq!("predict", body["phase"], "body: {body}");
    assert_eq!(false, body["auto_grade"], "body: {body}");
    assert_eq!(1, body["current"], "body: {body}");
    assert_eq!(2, body["total"], "body: {body}");
    assert_eq!("Predict the first hop", body["prompt"], "body: {body}");
}

#[test]
fn walk_predict_then_self_grade_reveals_the_excerpt_and_advances_the_hop() {
    let (base, _guard) = spawn_full_server(None);
    post_json(&base, "/api/select", r#"{"deck":"trace.txt"}"#);

    let resp = post_json(&base, "/api/walk/predict", r#"{"text":"my guess"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("reveal", body["phase"], "body: {body}");
    assert_eq!("my guess", body["prediction"], "body: {body}");
    assert_eq!("first", body["excerpt"]["lines"][0]["text"], "body: {body}");

    let resp = post_json(&base, "/api/walk/grade", r#"{"delta":"n"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("predict", body["phase"], "body: {body}");
    assert_eq!(2, body["current"], "body: {body}");
    assert_eq!("passed", body["path"][0]["delta"], "body: {body}");
}

#[test]
fn walk_restart_resets_to_the_first_hop() {
    let (base, _guard) = spawn_full_server(None);
    post_json(&base, "/api/select", r#"{"deck":"trace.txt"}"#);
    post_json(&base, "/api/walk/predict", r#"{"text":"my guess"}"#);
    post_json(&base, "/api/walk/grade", r#"{"delta":"n"}"#); // now on hop 2

    let resp = post_json(&base, "/api/walk/restart", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("predict", body["phase"], "body: {body}");
    assert_eq!(1, body["current"], "body: {body}");
}

#[test]
fn walk_leave_returns_to_the_picker_state_dto() {
    let (base, _guard) = spawn_full_server(None);
    post_json(&base, "/api/select", r#"{"deck":"trace.txt"}"#);

    let resp = post_json(&base, "/api/walk/leave", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["kind"], "body: {body}");
    assert_eq!("select", body["phase"], "body: {body}");
}

#[test]
fn get_api_walk_with_no_active_walk_yields_409() {
    let (base, _guard) = spawn_full_server(None);

    let resp = http(&base, "GET", "/api/walk", &[], &[]);

    assert_eq!(409, resp.status);
}

#[test]
fn walk_predict_with_auto_grade_resolves_a_verdict_via_the_fake_backend() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(scripts.path(), "PASSED — you got hop one right.\n");
    let (base, _guard) = spawn_full_server(Some(&fake));
    let select_resp = post_json(&base, "/api/select", r#"{"deck":"trace.txt"}"#);
    let select_body: serde_json::Value = serde_json::from_slice(&select_resp.body).unwrap();
    assert_eq!(true, select_body["auto_grade"], "body: {select_body}");

    post_json(
        &base,
        "/api/walk/predict",
        r#"{"text":"it forwards the line along"}"#,
    );

    let body = poll_until(&base, "/api/walk", |b| !b["thinking"].as_bool().unwrap());
    assert_eq!(Some("passed"), body["verdict"].as_str(), "body: {body}");
    assert!(
        body["feedback"].as_str().unwrap().contains("hop one right"),
        "body: {body}"
    );

    // No client delta needed: the resolved AI verdict is used.
    let resp = post_json(&base, "/api/walk/grade", "{}");
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("predict", body["phase"], "body: {body}");
    assert_eq!(2, body["current"], "body: {body}");
}

// ── Share / Receive (the "wormhole not installed" error phase) ───────────
//
// `wormhole` is installed on this dev machine but absent in CI, so a test
// relying on either presence or absence via the real `PATH` would be
// nondeterministic across environments. `with_empty_path` pins `PATH` to a
// directory that deliberately has no `wormhole`, so the spawn fails
// deterministically everywhere, hitting the same error-phase arm the real
// "not installed" case would.

#[test]
fn post_api_share_surfaces_an_install_hint_when_wormhole_is_not_on_path() {
    let empty = TempDir::new().unwrap();
    with_empty_path(empty.path(), || {
        let (base, _guard) = spawn_test_server();

        let resp = post_json(&base, "/api/share", "{}");

        assert_eq!(200, resp.status);
        let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!("error", body["phase"], "body: {body}");
        let err = body["error"].as_str().expect("an error message");
        assert!(
            err.contains("magic-wormhole installed"),
            "expected the install hint, got: {err}"
        );
    });
}

#[test]
fn post_api_receive_surfaces_an_install_hint_when_wormhole_is_not_on_path() {
    let empty = TempDir::new().unwrap();
    with_empty_path(empty.path(), || {
        let (base, _guard) = spawn_test_server();

        let resp = post_json(&base, "/api/receive", r#"{"code":"7-alpha-bravo"}"#);

        assert_eq!(200, resp.status);
        let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!("error", body["phase"], "body: {body}");
        let err = body["error"].as_str().expect("an error message");
        assert!(
            err.contains("magic-wormhole installed"),
            "expected the install hint, got: {err}"
        );
    });
}

#[test]
fn get_api_share_with_no_share_in_flight_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = http(&base, "GET", "/api/share", &[], &[]);

    assert_eq!(409, resp.status);
}

#[test]
fn get_api_receive_with_no_receive_in_flight_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = http(&base, "GET", "/api/receive", &[], &[]);

    assert_eq!(409, resp.status);
}

// ── Ask: tutor "make this a card" (draft → create round-trip) ────────────

/// Like [`spawn_test_server`], but serves `[serve] audience = "kids"`, for the
/// `/api/ask/card/draft` and `/api/ask/card/create` refusal tests. The
/// audience gate in both handlers (`src/serve/mod.rs`) runs before the
/// "no active review" check, so no deck needs to be selected for these to 403.
fn spawn_kids_server() -> (String, Guard) {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("sample.txt"), FIXTURE_DECK).unwrap();
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
    let opts = review_options(&base, None);
    let opts = ReviewOptions {
        audience: Audience::Kids,
        cfg: AssembleConfig {
            trace_auto_grade: false,
            pacing: Pacing {
                max_new: 10,
                limit: None,
            },
            instance_store: Some(store_path),
            ..opts.cfg
        },
        ..opts
    };

    let stop_handle = Arc::clone(&server);
    let handle = thread::spawn(move || {
        let _ = serve::run_review(store, recent, decks_dir, server, opts);
    });

    (
        base,
        Guard {
            server: stop_handle,
            handle: Some(handle),
            dir,
        },
    )
}

/// The full tutor "make this a card" round trip against a real server: seed a
/// tutor exchange, draft a card from it, edit the draft, mint it, then prove
/// it is actually drillable (not just stored) by re-selecting the deck and
/// finding it in the queue. One `fake_reply` answers every CLI invocation
/// (the script ignores its own argv, see `fake_reply`'s doc), so the same
/// deck-format block serves both as the seeded question's answer (any
/// non-empty text does, for that step) and, reused for the draft call, as the
/// text `ask::parse_drafted_card` turns into a `DraftCardDto`.
#[test]
fn ask_card_draft_then_create_round_trips_a_learner_edited_card_into_the_queue() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(scripts.path(), "# term?\n\tdefinition\n");
    let (base, _guard) = spawn_full_server(Some(&fake));
    select_fixture(&base);

    // Seed a tutor exchange so the transcript is non-empty before drafting.
    let resp = post_json(
        &base,
        "/api/ask",
        r#"{"question":"why does this matter?"}"#,
    );
    assert_eq!(200, resp.status);
    // The wait idiom this test reuses verbatim: `poll_until` (this file,
    // defined above at the `fn poll_until` declaration), a bounded (up to
    // 5s, 250 * 20ms) loop on the `thinking` condition, the same idiom
    // `exam_grade_on_a_trace_deck_walks_from_answering_to_a_passing_result_via_the_fake_backend`
    // and `walk_predict_with_auto_grade_resolves_a_verdict_via_the_fake_backend`
    // already use to wait on this exact kind of background ask/exam job.
    let body = poll_until(&base, "/api/ask", |b| !b["thinking"].as_bool().unwrap());
    assert_eq!(1, body["transcript"].as_array().unwrap().len(), "body: {body}");

    // Draft a card from the conversation.
    let resp = post_json(&base, "/api/ask/card/draft", "{}");
    assert_eq!(200, resp.status);
    let body = poll_until(&base, "/api/ask", |b| !b["thinking"].as_bool().unwrap());
    assert_eq!("term?", body["draft"]["front"], "body: {body}");
    assert_eq!(
        serde_json::json!(["definition"]),
        body["draft"]["back"],
        "body: {body}"
    );

    // Create the learner's edited version, deliberately different front/back
    // than the draft, to prove `/api/ask/card/create` mints what was posted,
    // not the draft still sitting on the ask DTO.
    let resp = post_json(
        &base,
        "/api/ask/card/create",
        r#"{"front":"edited term?","back":["edited definition"]}"#,
    );
    // 200, not 201: alix's JSON responder always answers 200 on success (see
    // the handler's own comment, `src/serve/mod.rs`); "created" is expressed
    // by `CreateCardResp`'s shape, not the status line (documented in
    // docs/API.md §4.5).
    assert_eq!(
        200,
        resp.status,
        "body: {}",
        String::from_utf8_lossy(&resp.body)
    );
    let create_body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(
        create_body["id"].as_str().is_some(),
        "body: {create_body}"
    );

    // Drillable, not just stored: cram-reselect (the same determinism idiom
    // `post_api_restart_rebuilds_the_queue_and_resets_session_stats` uses)
    // pulls every non-retired card into the queue regardless of due date. The
    // newly minted virtual card already has a store entry (`mint_tutor_card`
    // seeds one), so `build_queue` sorts it into the "due" group, ahead of
    // the two never-graded fixture cards in "fresh": it's the first card the
    // reselected session serves.
    let resp = post_json(&base, "/api/select", r#"{"deck":"sample.txt","cram":true}"#);
    assert_eq!(200, resp.status);
    let select_body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(3, select_body["remaining"], "body: {select_body}");
    assert_eq!("edited term?", select_body["card"]["front"], "body: {select_body}");

    // And it's what `/api/state` reports too, not just the `/api/select`
    // response (the same double-check `get_api_state_reflects_the_active_session_after_select`
    // makes for the fixture's own first card).
    let state = http(&base, "GET", "/api/state", &[], &[]);
    let state_body: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
    assert_eq!("edited term?", state_body["card"]["front"], "body: {state_body}");
}

#[test]
fn ask_card_draft_and_create_are_refused_for_a_kids_audience() {
    let (base, _guard) = spawn_kids_server();

    let draft_resp = post_json(&base, "/api/ask/card/draft", "{}");
    assert_eq!(403, draft_resp.status);

    let create_resp = post_json(
        &base,
        "/api/ask/card/create",
        r#"{"front":"f","back":["b"]}"#,
    );
    assert_eq!(403, create_resp.status);
}

#[test]
fn ask_card_create_with_a_back_matching_an_authored_card_yields_422() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    // A different front, but the same back line as the fixture's "2 + 2"
    // card: `Card::id` hashes the subject plus the normalized back only
    // (front and note are ignored, `src/card.rs::Card::id`), so this
    // collides with the deck's own authored card and must be refused.
    let resp = post_json(
        &base,
        "/api/ask/card/create",
        r#"{"front":"what does 2 plus 2 equal?","back":["4"]}"#,
    );

    assert_eq!(422, resp.status);
}
