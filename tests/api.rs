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
//!
//! Unix-only: the fake AI backend is a `/bin/sh` script. The Windows CI job
//! runs the lib persistence suites instead.
#![cfg(unix)]

use std::{
    collections::HashMap,
    ffi::OsString,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use alix::{
    assemble::{AssembleConfig, Pacing},
    augment::AugmentCache,
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
    // also lets a test reach into the fixture's files via `dir()`.
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

/// Deterministic store breakage: the store writes tmp files next to its
/// per-deck documents, so every directory under the state root loses write
/// permission (children first, since a read-only parent blocks recursion).
fn break_state_dir(root: &Path) {
    let entries: Vec<_> = std::fs::read_dir(root)
        .map(|it| it.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    for path in entries {
        if path.is_dir() {
            break_state_dir(&path);
        }
    }
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o555)).unwrap();
}

/// Undoes [`break_state_dir`] (parent first, so children become reachable).
fn repair_state_dir(root: &Path) {
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let entries: Vec<_> = std::fs::read_dir(root)
        .map(|it| it.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    for path in entries {
        if path.is_dir() {
            repair_state_dir(&path);
        }
    }
}

fn state_root(dir: &Path) -> PathBuf {
    dir.join("state")
}

fn open_instance_store(dir: &Path) -> Store {
    let decks = alix::workspace::deck_files(dir);
    alix::state::open_stores(&decks, &state_root(dir)).unwrap()
}

fn open_deck_store(dir: &Path, deck: &str) -> Store {
    alix::state::open_store(&dir.join(deck), &state_root(dir)).unwrap()
}

/// A minimal two-card fixture deck — enough for a grade→next-state sequence
/// (grading the first card away still leaves the session in `"review"` phase
/// on the second, rather than jumping straight to `"done"`) — and enough to
/// make `run_review`'s store resolution (`assemble::store_for`, via
/// `cfg.instance_store`) do real work if a test picks it via `/api/select`.
const FIXTURE_DECK: &str = "---\nformat-version: 1\nid: \"deck-sample\"\n---\n## 2 + 2 <!-- id: card-s1 -->\n4\n\n## 3 + 3 <!-- id: card-s2 -->\n6\n";

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
                max_session: 10,
                new_cards_percent: 30,
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
/// `sample.md`.
fn spawn_test_server_fixture(token: Option<&str>, extra: impl FnOnce(&Path)) -> (String, Guard) {
    let dir = TempDir::new().unwrap();
    let deck_path = dir.path().join("sample.md");
    std::fs::write(&deck_path, FIXTURE_DECK).unwrap();
    extra(dir.path());
    let store_path = state_root(dir.path());
    let store = open_instance_store(dir.path());
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
    // instead of a hand-rolled stub; give it the default pacing (max_session
    // 10, new_cards_percent 30), and pin the instance store to this fixture's
    // own state root.
    let opts = ReviewOptions {
        cfg: AssembleConfig {
            trace_auto_grade: false,
            pacing: Pacing {
                max_session: 10,
                new_cards_percent: 30,
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

/// Five single-line, distinct-answer cards, twice: `choice.md` (for the
/// augment-generation tests, un-augmented) and `choice-armed.md` (for the
/// choose/order tests, with cached AI distractors so a Recognize session
/// builds a real pick). Identity is the token, so the two copies carry
/// DISTINCT literal tokens (c1.. vs ca1..) to keep their ids apart — the
/// filename no longer separates them. Distractors are never sampled from
/// sibling answers, so only the armed copy renders choices. See
/// [`spawn_full_server_fixture`].
const CHOICE_DECK: &str = "---\nformat-version: 1\nid: \"deck-choice\"\n---\n## 1 + 1 <!-- id: card-c1 -->\n2\n\n## 2 + 2 <!-- id: card-c2 -->\n4\n\n\
                           ## 3 + 3 <!-- id: card-c3 -->\n6\n\n## 4 + 4 <!-- id: card-c4 -->\n8\n\n\
                           ## 5 + 5 <!-- id: card-c5 -->\n10\n";
const CHOICE_ARMED_DECK: &str = "---\nformat-version: 1\nid: \"deck-choicearmed\"\n---\n## 1 + 1 <!-- id: card-ca1 -->\n2\n\n## 2 + 2 <!-- id: card-ca2 -->\n4\n\n\
                                 ## 3 + 3 <!-- id: card-ca3 -->\n6\n\n## 4 + 4 <!-- id: card-ca4 -->\n8\n\n\
                                 ## 5 + 5 <!-- id: card-ca5 -->\n10\n";

/// [`CHOICE_DECK`]'s authored front → back, so a test can find which option is
/// correct without hard-coding a queue order the shuffle doesn't guarantee.
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
const TRACE_DECK: &str = "---\nformat-version: 1\nid: \"deck-trace\"\ntrace: how it works\nsource: source.txt\n---\n\
## Predict the first hop <!-- id: card-t1 -->\n\
<!-- given: line — the input line -->\n\
it reads the first line\n\
<!-- at: 1 fingerprint: xxh64-cc98257fe4be8f5a -->\n\
## Predict the second hop <!-- id: card-t2 -->\n\
it reads line two\n\
<!-- at: 2 fingerprint: xxh64-ca7ffbd94d5e0037 -->\n";
const TRACE_SOURCE: &str = "first\nsecond\nthird\n";

/// Richer than [`spawn_test_server`]: the same open (no-token) server, but its
/// decks dir also carries the choice fixture twice — `choice.md` (seen, no
/// cached distractors, for the augment-generation tests) and `choice-armed.md`
/// (seen
/// with cached AI distractors, so a Recognize-depth session quizzes it as a real
/// multiple-choice — see `current_question`, `src/serve/dto.rs`) — and
/// [`TRACE_DECK`] (routed to a real `Walk` by the real
/// classifier in `assemble::select`, for the walk and trace-exam families).
///
/// `ask_command`, when `Some`, points `[ask] command` at a fake CLI — see this
/// module's `fake_reply` — so a walk picked here auto-grades
/// (`AssembleConfig::trace_auto_grade`) instead of self-grading; `None` keeps every AI
/// path off (self-graded walk, no augmentation), which is what every non-AI
/// test in this family wants.
fn spawn_full_server(ask_command: Option<&Path>) -> (String, Guard) {
    spawn_full_server_fixture(ask_command, |_dir| {}, |_opts| {})
}

/// Like [`spawn_full_server`], but runs `extra` against the decks dir before
/// the server starts — lets a test add its own fixture files (e.g. a
/// workspace folder) alongside the standard decks.
fn spawn_full_server_fixture(
    ask_command: Option<&Path>,
    extra: impl FnOnce(&Path),
    tune: impl FnOnce(&mut ReviewOptions),
) -> (String, Guard) {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("sample.md"), FIXTURE_DECK).unwrap();
    std::fs::write(dir.path().join("choice.md"), CHOICE_DECK).unwrap();
    std::fs::write(dir.path().join("choice-armed.md"), CHOICE_ARMED_DECK).unwrap();
    std::fs::write(dir.path().join("trace.md"), TRACE_DECK).unwrap();
    std::fs::write(dir.path().join("source.txt"), TRACE_SOURCE).unwrap();
    extra(dir.path());
    let store_path = state_root(dir.path());

    // Two copies of the choice deck with distinct literal tokens → distinct
    // card ids: `choice.md` is seen but NOT augmented, so the
    // augment-generation tests still have `choices` warm items to build;
    // `choice-armed.md` is seen AND carries a full set of cached AI
    // distractors, so the choose/order tests render a real pick. Distractors
    // are never sampled from siblings, so the cache is the only way to arm a
    // pick. They are non-numeric, so none collides with a card's own numeric
    // answer and gets dropped as a duplicate.
    {
        let decks = alix::workspace::deck_files(dir.path())
            .into_iter()
            .map(|path| alix::deck::Deck::load(path).unwrap())
            .collect::<Vec<_>>();
        let deck_paths = decks
            .iter()
            .map(|deck| deck.path.clone())
            .collect::<Vec<_>>();
        let mut seed = alix::state::open_stores(&deck_paths, &store_path).unwrap();
        let mut aug = AugmentCache::open_for_decks(dir.path(), &decks).unwrap();
        for card in parser::parse_str("choice.md", CHOICE_DECK).unwrap() {
            seed.get_or_insert(&card.id().unwrap(), 0);
        }
        for card in parser::parse_str("choice-armed.md", CHOICE_ARMED_DECK).unwrap() {
            seed.get_or_insert(&card.id().unwrap(), 0);
            aug.set_distractors(
                &card.id().unwrap(),
                vec!["wrong a".into(), "wrong b".into(), "wrong c".into()],
                card.content_fingerprint,
            );
        }
        seed.save().unwrap();
        aug.save().unwrap();
    }

    let store = open_instance_store(dir.path());
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
    let mut opts = ReviewOptions {
        cfg: AssembleConfig {
            trace_auto_grade: auto_grade,
            pacing: Pacing {
                max_session: 10,
                new_cards_percent: 30,
            },
            instance_store: Some(store_path),
            ..opts.cfg
        },
        ..opts
    };
    tune(&mut opts);

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

/// Posts a card-relative mutation with the current `study_revision` echoed,
/// the way both web clients do. Tests that probe missing or stale echoes use
/// raw [`http`] instead.
fn post_gated(base: &str, path: &str, json: &str) -> HttpResp {
    let state = http(base, "GET", "/api/state", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&state.body).unwrap_or_default();
    let revision = body["study_revision"].as_u64().unwrap_or(0).to_string();
    http(
        base,
        "POST",
        path,
        &[
            ("Content-Type", "application/json"),
            ("X-Alix-Study-Revision", &revision),
        ],
        json.as_bytes(),
    )
}

/// Posts a multiple-choice pick the way both web clients do: the revision
/// echoed in the header and the id of the card on screen in the body. Tests
/// that probe a missing or mismatched card id build the body themselves.
fn post_choice(base: &str, index: usize) -> HttpResp {
    let state = http(base, "GET", "/api/state", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&state.body).unwrap_or_default();
    let revision = body["study_revision"].as_u64().unwrap_or(0).to_string();
    let card = body["card"]["id"].as_str().unwrap_or_default();
    http(
        base,
        "POST",
        "/api/choose",
        &[
            ("Content-Type", "application/json"),
            ("X-Alix-Study-Revision", &revision),
        ],
        format!(r#"{{"index":{index},"card":"{card}"}}"#).as_bytes(),
    )
}

/// Selects [`FIXTURE_DECK`] (by its fixed file name, `sample.md`) and returns
/// the resulting `StateDto` response — the common first step of every
/// review-loop test below.
fn select_fixture(base: &str) -> HttpResp {
    post_json(base, "/api/select", r#"{"deck":"sample.md"}"#)
}

#[test]
fn a_rejected_exam_start_keeps_the_active_progress_store() {
    let (base, guard) = spawn_test_server_fixture(None, write_animals_workspace);
    assert_eq!(200, select_fixture(&base).status);

    let progress = state_root(guard.dir()).join("progress/deck-sample.json");
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&progress).unwrap()).unwrap();

    let rejected = post_json(&base, "/api/exam/start", r#"{"deck":"animals/one.md"}"#);
    assert_eq!(409, rejected.status);

    let acquired = post_gated(&base, "/api/acquire", "{}");
    assert_eq!(200, acquired.status);

    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&progress).unwrap()).unwrap();
    assert!(
        after["revision"].as_u64().unwrap() > before["revision"].as_u64().unwrap(),
        "the accepted mutation must save through the still-active store: before={before}, after={after}"
    );
}

#[test]
fn an_active_workspace_listing_reads_the_progress_owner_projection() {
    fn member_row(base: &str) -> serde_json::Value {
        let response = http(base, "GET", "/api/decks", &[], &[]);
        assert_eq!(200, response.status);
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        body["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|workspace| workspace["name"] == "animals")
            .unwrap()
            .get("members")
            .and_then(serde_json::Value::as_array)
            .unwrap()
            .iter()
            .find(|member| member["name"] == "animals/one.md")
            .unwrap()
            .clone()
    }

    let (base, guard) = spawn_test_server_fixture(None, write_animals_workspace);
    let selected = post_json(&base, "/api/select", r#"{"deck":"animals/one.md"}"#);
    assert_eq!(200, selected.status);
    assert_eq!(200, post_gated(&base, "/api/acquire", "{}").status);

    let before = member_row(&base);
    let progress = guard.dir().join("animals/progress/deck-animalone.json");
    let parked = progress.with_extension("json.parked");
    std::fs::rename(&progress, &parked).unwrap();

    let after = member_row(&base);
    std::fs::rename(&parked, &progress).unwrap();
    assert_eq!(
        before, after,
        "the active Progress owner projection must remain authoritative while the on-disk document is temporarily unavailable"
    );
}

#[test]
fn an_inactive_workspace_listing_reads_the_progress_owner_projection() {
    fn member_row(base: &str) -> serde_json::Value {
        let response = http(base, "GET", "/api/decks", &[], &[]);
        assert_eq!(200, response.status);
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        body["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|workspace| workspace["name"] == "animals")
            .unwrap()
            .get("members")
            .and_then(serde_json::Value::as_array)
            .unwrap()
            .iter()
            .find(|member| member["name"] == "animals/one.md")
            .unwrap()
            .clone()
    }

    let (base, guard) = spawn_test_server_fixture(None, write_animals_workspace);
    let selected = post_json(&base, "/api/select", r#"{"deck":"animals/one.md"}"#);
    assert_eq!(200, selected.status);
    assert_eq!(200, post_gated(&base, "/api/acquire", "{}").status);
    assert_eq!(200, post_gated(&base, "/api/deselect", "{}").status);

    let before = member_row(&base);
    let progress = guard.dir().join("animals/progress/deck-animalone.json");
    let parked = progress.with_extension("json.parked");
    std::fs::rename(&progress, &parked).unwrap();

    let after = member_row(&base);
    std::fs::rename(&parked, &progress).unwrap();
    assert_eq!(
        before, after,
        "the Progress owner must project non-active documents too, rather than silently resurrecting their decks as new"
    );
}

#[test]
fn a_reset_reaches_the_listing_past_the_retained_workspace_snapshot() {
    fn member_row(base: &str) -> serde_json::Value {
        let response = http(base, "GET", "/api/decks", &[], &[]);
        assert_eq!(200, response.status);
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        body["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|workspace| workspace["name"] == "animals")
            .unwrap()
            .get("members")
            .and_then(serde_json::Value::as_array)
            .unwrap()
            .iter()
            .find(|member| member["name"] == "animals/one.md")
            .unwrap()
            .clone()
    }

    let (base, _guard) = spawn_test_server_fixture(None, write_animals_workspace);
    let selected = post_json(&base, "/api/select", r#"{"deck":"animals/one.md"}"#);
    assert_eq!(200, selected.status);
    assert_eq!(200, post_gated(&base, "/api/acquire", "{}").status);
    assert_eq!(200, post_gated(&base, "/api/deselect", "{}").status);

    let before = member_row(&base);
    assert_eq!(
        "started", before["state"],
        "the acquire must be visible pre-reset: {before}"
    );

    let reset = post_json(&base, "/api/reset", r#"{"deck":"animals/one.md"}"#);
    assert_eq!(200, reset.status);
    let cleared: serde_json::Value = serde_json::from_slice(&reset.body).unwrap();
    assert_eq!(
        1, cleared["cards_cleared"],
        "the reset must clear the acquired card: {cleared}"
    );

    let after = member_row(&base);
    assert_eq!(
        "new", after["state"],
        "the listing must serve the reset state, not the retained pre-reset snapshot: {after}"
    );
    assert_eq!(
        true, after["reviewable_recall"],
        "the reset deck must be startable again (this gates the picker's row actions): {after}"
    );
}

/// A named import destination resolves to that workspace's member dir, not
/// the served root: a misresolved destination silently misfiles the deck.
#[test]
fn importing_into_a_named_workspace_lands_in_that_workspace() {
    let (base, guard) = spawn_test_server_fixture(None, write_animals_workspace);
    let resp = post_json(
        &base,
        "/api/import",
        r###"{"name":"geo.md","text":"## q?\na\n","dest":"animals"}"###,
    );
    assert_eq!(
        200,
        resp.status,
        "body: {}",
        String::from_utf8_lossy(&resp.body)
    );
    assert!(
        guard.dir().join("animals/decks/geo.md").is_file(),
        "the import must land in the named workspace's member dir"
    );
    assert!(
        !guard.dir().join("geo.md").exists(),
        "the import must not land in the served root"
    );
}

/// Uploads are strict: a deck that does not parse is refused with a 400 and
/// the placed file is removed, never kept half-imported.
#[test]
fn importing_a_malformed_deck_is_refused_and_leaves_no_file() {
    let (base, guard) = spawn_test_server_fixture(None, write_animals_workspace);
    let resp = post_json(
        &base,
        "/api/import",
        r###"{"name":"broken.md","text":"## a question with no answer\n"}"###,
    );
    assert_eq!(400, resp.status);
    assert!(
        !guard.dir().join("broken.md").exists(),
        "a refused import must not leave the file behind"
    );
}

/// A plain folder (a grouping dir without `alix.toml`) has no store of its
/// own: its member rows read the served root store. A wrong store choice
/// leaves every folder member without status.
#[test]
fn a_folder_members_row_reads_the_served_root_store() {
    let (base, _guard) = spawn_test_server_fixture(None, |dir| {
        let folder = dir.join("animals");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("one.md"),
            "---\nformat-version: 1\nid: \"deck-folderone\"\n---\n## q1 <!-- id: card-fq1 -->\na1\n",
        )
        .unwrap();
    });
    let selected = post_json(&base, "/api/select", r#"{"deck":"animals/one.md"}"#);
    assert_eq!(200, selected.status);
    assert_eq!(200, post_gated(&base, "/api/acquire", "{}").status);

    let response = http(&base, "GET", "/api/decks", &[], &[]);
    assert_eq!(200, response.status);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    let member = body["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["label"] == "animals")
        .unwrap_or_else(|| panic!("the folder groups its members; body: {body}"))["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "animals/one.md")
        .unwrap()
        .clone();
    assert_eq!(
        "started", member["state"],
        "a folder member's progress lives in the root store; member: {member}"
    );
}

/// Selecting a workspace member records it in the recent list (loose decks
/// land there by the catalog scan alone, so only a member exercises the
/// recording path), and an unfinished session must record: the writer skips
/// only finished sessions.
#[test]
fn selecting_a_workspace_member_records_it_in_recent() {
    let (base, guard) = spawn_test_server_fixture(None, write_animals_workspace);
    let selected = post_json(&base, "/api/select", r#"{"deck":"animals/one.md"}"#);
    assert_eq!(200, selected.status);

    // Members render under their workspace row, not the recent section, so
    // the observable is the recorded file itself. The listing call is the
    // ordering barrier: the catalog owner processed the select's
    // record-recent command before it answered this list.
    let response = http(&base, "GET", "/api/decks", &[], &[]);
    assert_eq!(200, response.status);
    let recent = std::fs::read_to_string(guard.dir().join("recent.json")).unwrap_or_default();
    assert!(
        recent.contains("one.md"),
        "an unfinished member session must be recorded in recent; recent.json: {recent}"
    );
}

/// `/api/remote/ask`'s 400 guard, both polarities: an empty question is
/// refused whatever the card holds, an all-empty card is refused with a real
/// question, and an empty front with a non-empty back passes (the card is
/// askable). The passing probe matters: a wrongly widened guard turns valid
/// mobile asks into 400s.
#[test]
fn remote_ask_refuses_empty_questions_and_empty_cards_but_not_partial_cards() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(scripts.path(), "fine");
    let (base, _guard) = spawn_full_server(Some(&fake));

    let resp = post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},
            "history":[],"question":"   "}"#,
    );
    assert_eq!(400, resp.status, "an empty question must be refused");

    let resp = post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"sample.md","front":" ","back":[" "],"at":null},
            "history":[],"question":"why?"}"#,
    );
    assert_eq!(400, resp.status, "an all-empty card must be refused");

    let resp = post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"sample.md","front":" ","back":["4"],"at":null},
            "history":[],"question":"why?"}"#,
    );
    assert_eq!(
        200, resp.status,
        "an empty front with a non-empty back is askable"
    );
}

/// The request-body caps are generous by design (256 KiB): a body of a few
/// kilobytes must pass both cap gates and be served on its merits. This is
/// the under-cap direction the oversize tests cannot see.
#[test]
fn the_body_caps_admit_a_kilobytes_scale_body_on_both_route_families() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(scripts.path(), "fine");
    let (base, _guard) = spawn_full_server(Some(&fake));
    let pad = "x".repeat(8 * 1024);

    // json_body family (MAX_JSON_BODY): a parsed-but-mismatched choose is
    // 409, a cap-rejected body 400, so the status discriminates the cap.
    assert_eq!(
        200,
        post_json(&base, "/api/select", r#"{"deck":"sample.md"}"#).status
    );
    let state = http(&base, "GET", "/api/state", &[], &[]);
    let state: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
    let revision = state["study_revision"].as_u64().unwrap_or(0).to_string();
    let choose = format!(r#"{{"index":0,"card":"card-{pad}"}}"#);
    let resp = http(
        &base,
        "POST",
        "/api/choose",
        &[
            ("Content-Type", "application/json"),
            ("X-Alix-Study-Revision", &revision),
        ],
        choose.as_bytes(),
    );
    assert_eq!(
        409, resp.status,
        "an 8 KiB choose body must reach the card-identity check, not die at the cap"
    );

    // remote family (MAX_REMOTE_BODY): a long client-supplied card.
    let body = format!(
        r#"{{"card":{{"subject":"sample.md","front":" ","back":["4 {pad}"],"at":null}},"history":[],"question":"why?"}}"#
    );
    assert_eq!(
        200,
        post_json(&base, "/api/remote/ask", &body).status,
        "an 8 KiB remote body sits far under the 256 KiB cap"
    );
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
        recent.iter().any(|d| d["name"] == "sample.md"),
        "body: {body}"
    );
}

// ── Decks catalog: workspace rows vs. deck rows ──────────────────────────
//
// `spawn_test_server`'s fixture is a single loose deck — no workspace
// anywhere — so none of these tests can use it. `write_animals_workspace`
// adds a real workspace (an `alix.toml` manifest + two member decks)
// alongside `sample.md` via `spawn_test_server_fixture`, so `/api/decks`
// actually has a group row to exercise.

/// Writes a workspace `animals/` (with `alix.toml`, so it registers as
/// `is_workspace` — see `workspace::is_workspace`) holding two tiny member
/// decks, into the fixture's decks dir.
fn write_animals_workspace(dir: &Path) {
    let ws = dir.join("animals");
    let members = ws.join("decks");
    std::fs::create_dir_all(&members).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Animals\"\n").unwrap();
    std::fs::write(
        members.join("one.md"),
        "---\nformat-version: 1\nid: \"deck-animalone\"\n---\n## q1 <!-- id: card-aq1 -->\na1\n",
    )
    .unwrap();
    std::fs::write(
        members.join("two.md"),
        "---\nformat-version: 1\nid: \"deck-animaltwo\"\n---\n## q2 <!-- id: card-aq2 -->\na2\n",
    )
    .unwrap();
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

// ── Decks catalog: the deck cache ────────────────────────────────────────

/// A one-card deck whose H1 title is what `/api/decks` serves as the row's
/// `label`, the field that proves whether the file was (re-)parsed.
const TITLED_DECK: &str = "---\nformat-version: 1\nid: \"deck-titled\"\n---\n# Original Title\n\n## q <!-- id: card-ti1 -->\na\n";

fn write_titled_deck(dir: &Path) {
    std::fs::write(dir.join("titled.md"), TITLED_DECK).unwrap();
}

fn titled_row(base: &str) -> Option<serde_json::Value> {
    let resp = http(base, "GET", "/api/decks", &[], &[]);
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    body["recent"]
        .as_array()
        .expect("recent is an array")
        .iter()
        .find(|d| d["name"] == "titled.md")
        .cloned()
}

#[test]
fn an_unchanged_deck_is_served_from_cache_not_reparsed() {
    let (base, guard) = spawn_test_server_fixture(None, write_titled_deck);
    let row = titled_row(&base).expect("titled.md lists before the overwrite");
    assert_eq!("Original Title", row["label"], "row: {row}");

    // Same-length garbage under the original mtime: (mtime, size) match the
    // cached entry, so a correct cache must serve the old parse, while a
    // re-read would find no title and no cards (the row would degrade or
    // vanish from the listing).
    let path = guard.dir().join("titled.md");
    let meta = std::fs::metadata(&path).unwrap();
    let (mtime, size) = (meta.modified().unwrap(), meta.len());
    std::fs::write(&path, vec![b'z'; size as usize]).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(mtime).unwrap();
    drop(file);
    let meta = std::fs::metadata(&path).unwrap();
    assert_eq!(size, meta.len(), "the garbage must keep the byte length");
    assert_eq!(
        mtime,
        meta.modified().unwrap(),
        "the original mtime must be restored exactly"
    );

    let row = titled_row(&base)
        .expect("an unchanged (mtime, size) must be served from cache, not re-read");
    assert_eq!("Original Title", row["label"], "row: {row}");
}

#[test]
fn a_changed_deck_is_reparsed_on_the_next_listing() {
    let (base, guard) = spawn_test_server_fixture(None, write_titled_deck);
    let row = titled_row(&base).expect("titled.md lists initially");
    assert_eq!("Original Title", row["label"], "row: {row}");

    // A longer rewrite changes the size, which dodges mtime granularity.
    std::fs::write(
        guard.dir().join("titled.md"),
        "---\nformat-version: 1\nid: \"deck-titled\"\n---\n# A Renamed Title Longer Than Before\n\n## q <!-- id: card-ti1 -->\na\n",
    )
    .unwrap();
    std::fs::write(
        guard.dir().join("fresh.md"),
        "---\nformat-version: 1\nid: \"deck-fresh\"\n---\n## new q <!-- id: card-fr1 -->\nb\n",
    )
    .unwrap();

    let row = titled_row(&base).expect("titled.md still lists after the rewrite");
    assert_eq!(
        "A Renamed Title Longer Than Before", row["label"],
        "row: {row}"
    );
    let resp = http(&base, "GET", "/api/decks", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(
        body["recent"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "fresh.md"),
        "a brand-new file must appear (the readdir stays fresh): body: {body}"
    );
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
/// workspace member's grade lands in the workspace's own progress document,
/// not the served instance's state root. The old `store_for` closure this
/// harness stubbed out ignored its
/// `paths` argument and always opened the instance store, so this is the
/// first test able to exercise the real precedence (now wired via
/// `run_review` → `cfg.instance_store` → `assemble::store_for`).
#[test]
fn grading_a_workspace_member_writes_the_workspace_store_not_the_instance_store() {
    let (base, guard) = spawn_test_server_fixture(None, write_animals_workspace);
    let ws_store =
        alix::state::UserFiles::new(guard.dir().join("animals")).progress_for("deck-animalone");
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

    let resp = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);
    assert_eq!(200, resp.status);

    assert!(
        ws_store.exists(),
        "the workspace's own progress document must receive the grade write"
    );
    assert!(
        !alix::state::UserFiles::new(state_root(guard.dir()))
            .progress_for("deck-sample")
            .exists(),
        "the instance state root must not receive a workspace member's progress"
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

    let resp = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);

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
fn a_grade_is_on_disk_before_its_response_returns() {
    let (base, guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);

    assert_eq!(200, resp.status);
    let document = state_root(guard.dir.path()).join("progress/deck-sample.json");
    let json = std::fs::read_to_string(&document).unwrap_or_default();
    assert!(
        json.contains("\"card-s1\"") && json.contains("\"history\""),
        "the graded card persists without waiting for a session transition: {json}"
    );
}

#[test]
fn a_concurrent_writer_surfaces_save_error_in_the_review_state() {
    let (base, guard) = spawn_test_server();
    select_fixture(&base);

    let clean = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);
    let body: serde_json::Value = serde_json::from_slice(&clean.body).unwrap();
    assert!(body.get("save_error").is_none(), "clean session: {body}");

    let document = state_root(guard.dir.path()).join("progress/deck-sample.json");
    let mut other = Store::open_deck(&document, "deck-sample", "sample.md").unwrap();
    other.get_or_insert("card-elsewhere", 1);
    other.save().unwrap();

    let conflicted = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);
    let body: serde_json::Value = serde_json::from_slice(&conflicted.body).unwrap();
    assert!(
        body["save_error"]
            .as_str()
            .is_some_and(|error| error.contains("stale")),
        "conflicted session: {body}"
    );
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

/// The doctor's binary probes spawn subprocesses; a parked probe must not
/// hold the application state hostage. The fake backend signals a marker
/// file, then parks on a FIFO until the test releases it: while parked, an
/// independent `/api/state` request must answer. Condition-gated end to end;
/// the only waits are bounded condition polls and a bounded receive.
#[test]
fn a_parked_doctor_binary_probe_does_not_block_state_requests() {
    let fake_dir = TempDir::new().unwrap();
    let started = fake_dir.path().join("started");
    let fifo = fake_dir.path().join("release.fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs")
            .success()
    );
    let script = fake_dir.path().join("parked-backend");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\n: > {started}\nread _ < {fifo}\n",
            started = started.display(),
            fifo = fifo.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (base, _guard) = spawn_full_server(Some(&script));

    let doctor_base = base.clone();
    let doctor = thread::spawn(move || http(&doctor_base, "GET", "/api/doctor", &[], &[]));

    let probe_started = Instant::now();
    while !started.exists() {
        assert!(
            probe_started.elapsed() < Duration::from_secs(5),
            "the doctor probe never spawned the fake backend"
        );
        thread::sleep(Duration::from_millis(5));
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let state_base = base.clone();
    thread::spawn(move || {
        let _ = tx.send(http(&state_base, "GET", "/api/state", &[], &[]));
    });
    let state = rx.recv_timeout(Duration::from_secs(5));
    // Release the parked probe before asserting, so a regression fails this
    // test instead of wedging the suite on a never-finishing doctor.
    std::fs::write(&fifo, "go\n").unwrap();

    let state = state.expect("/api/state must answer while the doctor probe is parked");
    assert_eq!(200, state.status);
    let doctor = doctor.join().unwrap();
    assert_eq!(200, doctor.status);
}

/// ADR 0027: a dirty store whose flush fails must refuse replacement and
/// keep the session active, so repeating the same request retries the flush
/// once the filesystem is repaired. The store save is broken deterministically
/// by dropping write permission on the state root.
#[test]
fn a_failed_flush_refuses_deselect_until_the_store_saves_again() {
    let (base, guard) = spawn_test_server();
    let resp = select_fixture(&base);
    assert_eq!(200, resp.status);

    let state_dir = state_root(guard.dir());
    break_state_dir(&state_dir);

    // The acquire itself replies 200 with `save_error` set (existing
    // contract); the store is now dirty with an unsaved mutation.
    let resp = post_gated(&base, "/api/acquire", "{}");
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(
        body["save_error"].as_str().is_some(),
        "the broken state root must surface save_error: {body}"
    );

    // Replacement is refused while the dirty flush cannot land.
    let resp = post_json(&base, "/api/deselect", "{}");
    assert_eq!(500, resp.status);
    let resp = http(&base, "GET", "/api/state", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["phase"], "the session must survive: {body}");

    // Repair the filesystem; repeating the request retries the flush.
    repair_state_dir(&state_dir);
    let resp = post_json(&base, "/api/deselect", "{}");
    assert_eq!(200, resp.status);
    let resp = http(&base, "GET", "/api/state", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("select", body["phase"], "{body}");
}

/// Concurrent same-name imports race place_deck's collision check and its
/// per-name temp file unless one owner serializes destination writes. Each
/// round fires two distinct bodies at one fresh name from two threads:
/// exactly one may land, and the landed file must be exactly the winner's
/// text, never a crossover of the loser's bytes.
#[test]
fn concurrent_same_name_imports_land_exactly_one_intact_deck() {
    let (base, guard) = spawn_test_server();
    for round in 0..25 {
        let name = format!("race-{round}.md");
        let texts = [
            format!("## alpha {round}\nfirst body\n"),
            format!("## beta {round}\nsecond body\n"),
        ];
        let mut handles = Vec::new();
        for text in texts.clone() {
            let base = base.clone();
            let name = name.clone();
            handles.push(thread::spawn(move || {
                let body = serde_json::json!({ "name": name, "text": text });
                let resp = post_json(&base, "/api/import", &body.to_string());
                (resp.status, text)
            }));
        }
        let outcomes: Vec<(u16, String)> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let winners: Vec<&(u16, String)> = outcomes
            .iter()
            .filter(|(status, _)| *status == 200)
            .collect();
        assert_eq!(
            1,
            winners.len(),
            "round {round}: exactly one import may land: {outcomes:?}"
        );
        // Landing stamps ids into the file, so assert on the body lines: the
        // winner's text is present and no byte of the loser's ever is.
        let landed = std::fs::read_to_string(guard.dir().join(&name)).unwrap();
        let (_, winner_text) = winners[0];
        let loser_text = outcomes
            .iter()
            .find(|(status, _)| *status != 200)
            .map(|(_, text)| text.clone())
            .unwrap();
        let winner_line = winner_text.lines().last().unwrap();
        let loser_line = loser_text.lines().last().unwrap();
        assert!(
            landed.contains(winner_line),
            "round {round}: winner body missing: {landed:?}"
        );
        assert!(
            !landed.contains(loser_line),
            "round {round}: loser bytes crossed over: {landed:?}"
        );
    }
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
    let resp = post_gated(&base, "/api/grade", r#"{"nonsense":true}"#);

    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

#[test]
fn post_api_grade_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);

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
fn adult_assets_are_public_no_cache_and_allowlisted() {
    let (base, _guard) = spawn_test_server_with(Some("secret"));

    for (path, content_type, marker) in [
        ("/review.css", "text/css; charset=utf-8", ":root"),
        (
            "/review.js",
            "application/javascript; charset=utf-8",
            "boot()",
        ),
    ] {
        let resp = http(&base, "GET", path, &[], &[]);
        assert_eq!(200, resp.status, "path: {path}");
        assert_eq!(
            Some(content_type),
            resp.header("Content-Type"),
            "path: {path}"
        );
        assert_eq!(
            Some("no-cache"),
            resp.header("Cache-Control"),
            "path: {path}"
        );
        assert!(
            String::from_utf8_lossy(&resp.body).contains(marker),
            "path: {path}"
        );
    }

    let resp = http(&base, "GET", "/review/nope.js", &[], &[]);
    assert_eq!(404, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

#[test]
fn kids_assets_are_public_no_cache_and_allowlisted() {
    let (base, _guard) = spawn_test_server_with(Some("secret"));

    for (path, content_type, marker) in [
        ("/kids.css", "text/css; charset=utf-8", ":root"),
        (
            "/kids.js",
            "application/javascript; charset=utf-8",
            "createKidsPicker",
        ),
    ] {
        let resp = http(&base, "GET", path, &[], &[]);
        assert_eq!(200, resp.status, "path: {path}");
        assert_eq!(
            Some(content_type),
            resp.header("Content-Type"),
            "path: {path}"
        );
        assert_eq!(
            Some("no-cache"),
            resp.header("Cache-Control"),
            "path: {path}"
        );
        assert!(
            String::from_utf8_lossy(&resp.body).contains(marker),
            "path: {path}"
        );
    }

    let resp = http(&base, "GET", "/kids/nope.js", &[], &[]);
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

    let resp = post_json(&base, "/api/browse", r#"{"deck":"sample.md"}"#);

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

    let resp = post_json(&base, "/api/browse", r#"{"deck":"nope.md"}"#);

    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

#[test]
fn post_api_browse_serializes_card_images_as_lists_with_alt() {
    // A back card carries two image embeds (the second without an alt); a
    // divided front card carries one. Both sides serialize as ordered
    // `{ src, alt }` lists, and the old scalar `img`/`img_back` keys are gone.
    const IMG_DECK: &str = "---\nformat-version: 1\nid: \"deck-images\"\n---\n## Back images <!-- id: card-bi1 -->\nWaxing\n\
                            ![a moon](moon.png)\n![](crescent.png)\n\n\
                            ## Front image <!-- id: card-fi1 -->\n![the sun](sun.png)\n\n\
                            ---\nThe sun\n";
    let (base, _guard) = spawn_test_server_fixture(None, |dir| {
        std::fs::write(dir.join("images.md"), IMG_DECK).unwrap();
    });

    let resp = post_json(&base, "/api/browse", r#"{"deck":"images.md"}"#);
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let cards = body["cards"].as_array().expect("cards is an array");
    assert_eq!(2, cards.len(), "body: {body}");

    // Card 0: two back images, no front image; each is a `{ src, alt }` object,
    // the src a `/img/<key>` URL as before.
    let back = cards[0]["images_back"]
        .as_array()
        .expect("images_back is a list");
    assert_eq!(2, back.len(), "two back images: {body}");
    assert!(
        back[0]["src"].as_str().unwrap().starts_with("/img/"),
        "src is an /img/ url: {body}"
    );
    assert_eq!("a moon", back[0]["alt"], "first image's alt: {body}");
    assert!(back[1]["alt"].is_null(), "second image has no alt: {body}");
    assert!(
        cards[0]["images"].as_array().unwrap().is_empty(),
        "back card has no front image: {body}"
    );

    // Card 1: one front image, no back image.
    let front = cards[1]["images"].as_array().expect("images is a list");
    assert_eq!(1, front.len(), "one front image: {body}");
    assert_eq!("the sun", front[0]["alt"], "front image alt: {body}");
    assert!(
        cards[1]["images_back"].as_array().unwrap().is_empty(),
        "front card has no back image: {body}"
    );

    // The old scalar keys are gone from the wire.
    assert!(cards[0].get("img").is_none(), "old img key removed: {body}");
    assert!(
        cards[0].get("img_back").is_none(),
        "old img_back key removed: {body}"
    );
}

// ── Deck drawer ─────────────────────────────────────────────────────────

#[test]
fn post_api_deck_drawer_returns_a_flat_heatmap_for_the_fixture_deck() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/deck-drawer", r#"{"deck":"sample.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(
        body["topologies"].as_array().unwrap().is_empty(),
        "no augmentation was ever generated: body: {body}"
    );
    // Both fixture cards are stamped but never shown (a fresh store), so each
    // reads as the untouched tier: one heatmap cell per card.
    assert_eq!(
        serde_json::json!(["untouched", "untouched"]),
        body["heatmap"],
        "body: {body}"
    );
    assert!(body.get("deck_due").is_none(), "deck_due removed: {body}");
    assert!(
        body["preamble"].is_null(),
        "no preamble in the fixture: {body}"
    );
}

#[test]
fn post_api_deck_drawer_tiers_an_acquired_card_above_a_merely_presented_one() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);
    // Acknowledge the first card ("Seen"); the second card then becomes the
    // displayed card, which stamps its presentation but acquires nothing.
    post_gated(&base, "/api/acquire", "{}");

    let resp = post_json(&base, "/api/deck-drawer", r#"{"deck":"sample.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let cells = body["heatmap"].as_array().unwrap();
    assert_eq!(2, cells.len(), "one cell per card: {body}");
    assert!(
        cells.iter().any(|c| c == &serde_json::json!("acquired")),
        "the acknowledged card reads as acquired: {body}"
    );
    assert!(
        cells.iter().any(|c| c == &serde_json::json!("seen")),
        "the shown-but-unacquired card reads as seen: {body}"
    );
    // The nested progress funnel counts both presented cards as seen.
    assert_eq!(2, body["total"], "body: {body}");
    assert_eq!(2, body["seen"], "both cards were presented: {body}");
    assert_eq!(0, body["graduated"], "nothing graduated yet: {body}");
    assert_eq!(0, body["retired"], "nothing retired yet: {body}");
}

#[test]
fn post_api_deck_drawer_counts_a_presented_card_as_seen_after_a_bare_select() {
    let (base, _guard) = spawn_test_server();
    // Selecting shows the first card and nothing else: no grade, no acquire.
    select_fixture(&base);

    let resp = post_json(&base, "/api/deck-drawer", r#"{"deck":"sample.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        serde_json::json!(["seen", "untouched"]),
        body["heatmap"],
        "only the displayed card is seen: {body}"
    );
    assert_eq!(1, body["seen"], "body: {body}");
    let seen = body["seen"].as_u64().unwrap();
    let graduated = body["graduated"].as_u64().unwrap();
    let retired = body["retired"].as_u64().unwrap();
    let total = body["total"].as_u64().unwrap();
    assert!(
        retired <= graduated && graduated <= seen && seen <= total,
        "the funnel still nests under the presented predicate: {body}"
    );
}

#[test]
fn post_api_deck_drawer_with_an_unknown_deck_still_returns_the_empty_default_dto() {
    // `/api/deck-drawer` never errors (docs/API.md): an unresolvable name still
    // gets 200 with the empty default, not a 400.
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/deck-drawer", r#"{"deck":"nope.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(
        body["topologies"].as_array().unwrap().is_empty(),
        "body: {body}"
    );
    assert!(
        body["heatmap"].as_array().unwrap().is_empty(),
        "body: {body}"
    );
    assert!(body["preamble"].is_null(), "body: {body}");
}

// ── Reset ───────────────────────────────────────────────────────────────

#[test]
fn post_api_reset_clears_the_fixture_decks_progress() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);
    // Grade the first card so it has stored progress to clear.
    post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);

    let resp = post_json(&base, "/api/reset", r#"{"deck":"sample.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("sample.md", body["deck"], "body: {body}");
    assert_eq!(
        2, body["cards_cleared"],
        "the graded card plus the next card's presentation stamp: {body}"
    );
}

#[test]
fn post_api_reset_with_an_unknown_deck_yields_400() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/reset", r#"{"deck":"nope.md"}"#);

    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

// ── Import ──────────────────────────────────────────────────────────────

#[test]
fn post_api_import_lands_an_md_deck_and_reports_its_card_count() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(
        &base,
        "/api/import",
        r###"{"name":"extra.md","text":"## f\nb\n"}"###,
    );

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("extra.md", body["deck"], "body: {body}");
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
        body["deck"].as_str().unwrap().ends_with(".md"),
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

    let resp = post_gated(&base, "/api/check", r#"{"lines":["4"]}"#);

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

    let resp = post_gated(&base, "/api/check", r#"{"lines":["5"]}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(false, body["passed"], "body: {body}");
    assert_eq!(false, body["results"][0]["passed"], "body: {body}");
}

#[test]
fn post_api_check_derives_orderedness_from_the_mode_not_the_client() {
    // A `reveal: line` deck at Reconstruct renders TypeLine: the check is
    // position-sensitive by the server's own derivation alone; the request
    // carries no ordering flag.
    let (base, _guard) = spawn_test_server_fixture(None, |dir| {
        std::fs::write(
            dir.join("steps.md"),
            "---\nformat-version: 1\nid: \"deck-steps\"\n---\n## steps <!-- id: card-st1 --> <!-- reveal: line -->\none\ntwo\n",
        )
        .unwrap();
    });
    let resp = post_json(
        &base,
        "/api/select",
        r#"{"deck":"steps.md","depth":"reconstruct"}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("typeline", body["mode"], "body: {body}");

    // The right lines in the wrong order must fail a TypeLine check.
    let resp = post_gated(&base, "/api/check", r#"{"lines":["two","one"]}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(false, body["passed"], "body: {body}");
}

#[test]
fn post_api_check_with_a_malformed_body_yields_400() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_gated(&base, "/api/check", r#"{"nonsense":true}"#);

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_check_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_gated(&base, "/api/check", r#"{"lines":["4"]}"#);

    assert_eq!(409, resp.status);
}

// ── Choose (multiple choice, Recognize depth) ────────────────────────────

#[test]
fn post_api_choose_reports_the_correct_index_for_a_recognize_session() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(
        &base,
        "/api/select",
        r#"{"deck":"choice-armed.md","depth":"recognize"}"#,
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

    let resp = post_choice(&base, correct_index);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(correct_index, body["chosen"], "body: {body}");
    assert_eq!(correct_index, body["correct"], "body: {body}");
    assert_eq!(true, body["passed"], "body: {body}");
}

#[test]
fn choices_keep_their_order_across_state_pulls_while_the_card_is_on_screen() {
    // Returning from the tutor re-pulls /api/state while the client keeps its
    // answered feedback as INDICES (chosen/correct). If the served option
    // order shifts between the answer and that re-pull, the indices decorate
    // the wrong options and a wrong pick renders as "correct" (user report,
    // 2026-07-14).
    let (base, _guard) = spawn_full_server(None);
    let resp = post_json(
        &base,
        "/api/select",
        r#"{"deck":"choice-armed.md","depth":"recognize"}"#,
    );
    let first: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let before = first["choices"].clone();

    post_choice(&base, 0);
    let resp = http(&base, "GET", "/api/state", &[], &[]);
    let after: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();

    assert_eq!(
        first["card"]["front"], after["card"]["front"],
        "the session stays on the answered card while its feedback shows"
    );
    assert_eq!(
        before, after["choices"],
        "the option order must not shift while the card is on screen"
    );
}

#[test]
fn choices_keep_their_order_across_a_full_tutor_round_trip() {
    // The exact user flow of the 2026-07-14 report: answer a choice card, open
    // the tutor, ask a question, save the conversation as a note (which
    // rewrites the deck file and mutates the in-memory card), close the tutor
    // (the client re-pulls /api/state) — the option order must survive it all.
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(scripts.path(), "a condensed tutor note");
    let (base, _guard) = spawn_full_server(Some(&fake));
    let resp = post_json(
        &base,
        "/api/select",
        r#"{"deck":"choice-armed.md","depth":"recognize"}"#,
    );
    let first: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let before = first["choices"].clone();
    assert!(before.is_array(), "body: {first}");

    post_choice(&base, 0);
    post_gated(
        &base,
        "/api/ask",
        r#"{"question":"why is that the answer?"}"#,
    );
    poll_until(&base, "/api/ask", |b| b["thinking"] == false);
    post_gated(&base, "/api/ask/note", "{}");
    poll_until(&base, "/api/ask", |b| b["thinking"] == false);

    let resp = http(&base, "GET", "/api/state", &[], &[]);
    let after: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        first["card"]["front"], after["card"]["front"],
        "the session stays on the answered card through the tutor round trip"
    );
    assert_eq!(
        before, after["choices"],
        "the option order must not shift across the tutor round trip"
    );
}

#[test]
fn recognize_is_unavailable_and_empty_on_an_unaugmented_deck() {
    // Recognize is pick-only: an un-augmented deck can build no pick, so the
    // listing greys it out (`can_recognize` false) and a Recognize session over
    // it schedules nothing at all — no plain-flip fallback.
    let (base, _guard) = spawn_full_server(None);

    let resp = http(&base, "GET", "/api/decks", &[], &[]);
    let decks: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let recent = decks["recent"].as_array().expect("recent decks");
    let find = |name: &str| {
        recent
            .iter()
            .find(|d| d["name"] == name)
            .unwrap_or_else(|| panic!("deck {name} not listed: {decks}"))
    };
    assert_eq!(
        false,
        find("choice.md")["can_recognize"],
        "un-augmented deck can't recognize"
    );
    assert_eq!(
        true,
        find("choice-armed.md")["can_recognize"],
        "the armed deck can recognize"
    );

    // Selecting Recognize on the un-augmented deck schedules nothing.
    let resp = post_json(
        &base,
        "/api/select",
        r#"{"deck":"choice.md","depth":"recognize"}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(body["card"].is_null(), "no card scheduled: {body}");
    assert_eq!("done", body["phase"], "empty recognize session: {body}");
    assert_eq!(0, body["initial"], "nothing entered the roster: {body}");
    assert!(
        body["next_due_ms"].is_null(),
        "nothing is scheduled, so the done payload carries no next-due instant: {body}"
    );
}

#[test]
fn cloze_choice_options_with_ai_distractors_keep_their_order_across_pulls() {
    // High-fidelity shape of the 2026-07-14 report: a two-hole cloze card whose
    // hole has AI distractors cached, served as a choice, answered, then the
    // state re-pulled (the tutor-close pull). The order must hold on both the
    // Recognize path (seen card) and the acquire path (unseen card).
    const CLOZE_DECK: &str = "---\nformat-version: 1\nid: \"deck-frb\"\n---\n## What is frb, in one sentence? <!-- id: card-frb1 -->\n\
        A \\blank{code-generation} tool generating the \\blank{FFI} glue on both sides.\n";
    for seed_store in [true, false] {
        let (base, _guard) = spawn_full_server_fixture(
            None,
            |dir| {
                std::fs::write(dir.join("frb.md"), CLOZE_DECK).unwrap();
                let cards = parser::parse_str("frb.md", CLOZE_DECK).unwrap();
                let deck_path = dir.join("frb.md");
                let fixture_state = state_root(dir);
                let deck = alix::deck::Deck::load(&deck_path).unwrap();
                let mut cache = alix::augment::AugmentCache::open_for_deck(&deck).unwrap();
                for c in &cards {
                    cache.set_distractors(
                        &c.id().unwrap(),
                        vec!["IPC".into(), "RPC".into(), "a REST API".into()],
                        c.content_fingerprint,
                    );
                }
                cache.save().unwrap();
                if seed_store {
                    let mut store = alix::state::open_store(&deck_path, &fixture_state).unwrap();
                    for c in &cards {
                        store.get_or_insert(&c.id().unwrap(), 0);
                    }
                    store.save().unwrap();
                }
            },
            |_opts| {},
        );
        let resp = post_json(
            &base,
            "/api/select",
            r#"{"deck":"frb.md","depth":"recognize"}"#,
        );
        let first: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        let before = first["choices"].clone();
        assert!(
            before.is_array(),
            "expected a choice question (seed_store={seed_store}): {first}"
        );

        post_choice(&base, 0);
        let resp = http(&base, "GET", "/api/state", &[], &[]);
        let after: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(
            first["card"]["front"], after["card"]["front"],
            "same card (seed_store={seed_store})"
        );
        assert_eq!(
            before, after["choices"],
            "option order shifted (seed_store={seed_store})"
        );
    }
}

/// A deck whose Recognize pool is exhausted (every pick-capable card already
/// recognized) must not answer a bare select with an empty done and nothing
/// else: the DTO carries the gap — how many cards wait at Recall, and how
/// many no pick can be built for — so the summary can point at the two real
/// exits (continue at Recall, or augment choices) instead of "come back
/// later" (user report 2026-08-01, deck 59: 2 authored picks recognized, 13
/// cards invisible forever).
#[test]
fn an_exhausted_recognize_deck_reports_the_gap_not_a_bare_empty_done() {
    const MIXED: &str = "---\nformat-version: 1\nid: \"deck-choicemixed\"\n---\n\
        ## pick 1 <!-- id: card-cm1 -->\n- [x] right\n- [ ] wrong-a\n- [ ] wrong-b\n\n\
        ## pick 2 <!-- id: card-cm2 -->\n- [x] yes\n- [ ] no-a\n- [ ] no-b\n\n\
        ## plain 1 <!-- id: card-cm3 -->\nback 3\n\n\
        ## plain 2 <!-- id: card-cm4 -->\nback 4\n\n\
        ## plain 3 <!-- id: card-cm5 -->\nback 5\n";
    let (base, _guard) = spawn_full_server_fixture(
        None,
        |dir| {
            std::fs::write(dir.join("choice-mixed.md"), MIXED).unwrap();
            let cards = parser::parse_str("choice-mixed.md", MIXED).unwrap();
            let deck_path = dir.join("choice-mixed.md");
            let fixture_state = state_root(dir);
            let mut store = alix::state::open_store(&deck_path, &fixture_state).unwrap();
            for c in cards.iter().filter(|c| !c.authored_distractors.is_empty()) {
                let s = store.get_or_insert(&c.id().unwrap(), 1_000);
                s.acquired_ms = Some(1_000);
                s.recognized_ms = Some(2_000);
            }
            store.save().unwrap();
        },
        |_opts| {},
    );

    // A bare select resolves the deck's default depth: it has authored picks,
    // so that is Recognize — whose pool is exhausted.
    let resp = post_json(&base, "/api/select", r#"{"deck":"choice-mixed.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("done", body["phase"], "body: {body}");
    assert_eq!("recognize", body["depth"], "body: {body}");
    let gap = &body["recognize_gap"];
    assert!(
        gap.is_object(),
        "an exhausted Recognize done must carry recognize_gap: {body}"
    );
    assert!(
        gap["recall"].as_u64().unwrap_or(0) >= 3,
        "the three never-seen plain cards wait at Recall: {body}"
    );
    assert_eq!(
        3, gap["unaugmented"],
        "the three plain cards can build no pick: {body}"
    );

    // The gap never leaks onto other depths or unfinished sessions: a Recall
    // select over the same deck serves cards, and its state carries no gap.
    let resp = post_json(
        &base,
        "/api/select",
        r#"{"deck":"choice-mixed.md","depth":"recall"}"#,
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["phase"], "body: {body}");
    assert!(
        body["recognize_gap"].is_null(),
        "no gap outside an exhausted Recognize done: {body}"
    );
}

/// Acquiring every fresh pick of a Recognize sitting parks them behind the
/// acquire floor: the done summary then shows "N still due" beside a disabled
/// Continue — a contradiction unless the state says when one opens (user
/// report 2026-08-01). `next_due_ms` must carry the floor-open instant even
/// at Recognize, where the schedule-wide next-due is undefined.
#[test]
fn a_recognize_done_with_floored_cards_says_when_one_opens() {
    const MIXED: &str = "---\nformat-version: 1\nid: \"deck-choicecool\"\n---\n\
        ## cool 1 <!-- id: card-cc1 -->\n- [x] right\n- [ ] wrong-a\n- [ ] wrong-b\n\n\
        ## cool 2 <!-- id: card-cc2 -->\n- [x] yes\n- [ ] no-a\n- [ ] no-b\n\n\
        ## cool plain <!-- id: card-cc3 -->\nback\n";
    let (base, _guard) = spawn_full_server_fixture(
        None,
        |dir| std::fs::write(dir.join("choice-cool.md"), MIXED).unwrap(),
        |_opts| {},
    );

    let resp = post_json(
        &base,
        "/api/select",
        r#"{"deck":"choice-cool.md","depth":"recognize"}"#,
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["phase"], "two fresh picks serve: {body}");

    post_gated(&base, "/api/acquire", "{}");
    let resp = post_gated(&base, "/api/acquire", "{}");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();

    assert_eq!(
        "done", body["phase"],
        "both picks acquired and floored ends the sitting: {body}"
    );
    assert_eq!(2, body["due_left"], "the floored picks stay due: {body}");
    assert_eq!(
        false, body["can_restart"],
        "nothing is servable while cooling: {body}"
    );
    assert!(
        body["next_due_ms"].as_u64().is_some(),
        "the summary must say when a floored card opens: {body}"
    );
}

/// Revealing a new card's answer IS the encounter: abandoning the session
/// after the reveal must not re-introduce the card as new next time (user
/// rule 2026-08-01). The reveal is reported to the server, which records the
/// engagement without advancing the session.
#[test]
fn a_revealed_then_abandoned_new_card_does_not_return_as_new() {
    const ONE: &str = "---\nformat-version: 1\nid: \"deck-revealone\"\n---\n\
        ## the only card <!-- id: card-rv1 -->\nits answer\n";
    let (base, _guard) = spawn_full_server_fixture(
        None,
        |dir| std::fs::write(dir.join("reveal-one.md"), ONE).unwrap(),
        |_opts| {},
    );

    let resp = post_json(&base, "/api/select", r#"{"deck":"reveal-one.md"}"#);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["phase"], "body: {body}");
    assert_eq!(true, body["acquire"], "a fresh card starts new: {body}");

    let resp = post_gated(&base, "/api/reveal", "{}");
    assert_eq!(200, resp.status);

    post_json(&base, "/api/deselect", "{}");
    let resp = post_json(&base, "/api/select", r#"{"deck":"reveal-one.md"}"#);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        "done", body["phase"],
        "a revealed card is engaged, not new; it cools instead of re-introducing: {body}"
    );
    assert!(
        body["next_due_ms"].as_u64().is_some(),
        "the cooling engagement has a next-open instant: {body}"
    );
}

/// The other half of the rule: leaving BEFORE the reveal keeps the card new.
#[test]
fn an_unrevealed_new_card_stays_new_after_abandoning() {
    const ONE: &str = "---\nformat-version: 1\nid: \"deck-unrevealed\"\n---\n\
        ## the only card <!-- id: card-ur1 -->\nits answer\n";
    let (base, _guard) = spawn_full_server_fixture(
        None,
        |dir| std::fs::write(dir.join("unrevealed.md"), ONE).unwrap(),
        |_opts| {},
    );

    post_json(&base, "/api/select", r#"{"deck":"unrevealed.md"}"#);
    post_json(&base, "/api/deselect", "{}");
    let resp = post_json(&base, "/api/select", r#"{"deck":"unrevealed.md"}"#);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();

    assert_eq!("review", body["phase"], "body: {body}");
    assert_eq!(
        true, body["acquire"],
        "merely being shown is not an encounter; the card stays new: {body}"
    );
}

/// Within the sitting the revealed card stays current and stays the acquire
/// card: the engagement is for FUTURE sessions only. If the live
/// classification flipped, the next state poll would swap the card without a
/// revision bump (the wrong-card-grading class).
#[test]
fn a_reveal_keeps_the_current_card_current_and_new_within_the_sitting() {
    let (base, _guard) = spawn_test_server();
    let resp = select_fixture(&base);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let front = body["card"]["front"].as_str().unwrap().to_string();
    let revision = body["study_revision"].as_u64().unwrap();

    let resp = post_gated(&base, "/api/reveal", "{}");
    assert_eq!(200, resp.status);

    for pull in 0..2 {
        let resp = http(&base, "GET", "/api/state", &[], &[]);
        let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(
            front,
            body["card"]["front"].as_str().unwrap_or_default(),
            "pull {pull}: the revealed card must stay current: {body}"
        );
        assert_eq!(
            true, body["acquire"],
            "pull {pull}: it stays the acquire card this sitting: {body}"
        );
        assert_eq!(
            revision,
            body["study_revision"].as_u64().unwrap_or(0),
            "pull {pull}: a reveal is not a transition; the revision holds: {body}"
        );
    }
}

/// The reveal must not flip the acquire question away mid-card: the store now
/// sees the card engaged, but the recognition pick the client rendered must
/// stay buildable and answerable this sitting.
#[test]
fn a_reveal_keeps_the_acquire_choice_answerable() {
    const MIXED: &str = "---\nformat-version: 1\nid: \"deck-revealpick\"\n---\n\
        ## pick me <!-- id: card-rp1 -->\n- [x] right\n- [ ] wrong-a\n- [ ] wrong-b\n";
    let (base, _guard) = spawn_full_server_fixture(
        None,
        |dir| std::fs::write(dir.join("reveal-pick.md"), MIXED).unwrap(),
        |_opts| {},
    );
    let resp = post_json(
        &base,
        "/api/select",
        r#"{"deck":"reveal-pick.md","depth":"recall"}"#,
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(true, body["acquire"], "body: {body}");
    assert!(body["choices"].is_array(), "body: {body}");

    let resp = post_gated(&base, "/api/reveal", "{}");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(200, resp.status);
    assert!(
        body["choices"].is_array(),
        "the recognition question survives the reveal: {body}"
    );

    let resp = post_choice(&base, 0);
    assert_eq!(
        200, resp.status,
        "the pick stays answerable after the reveal"
    );
}

#[test]
fn post_api_choose_with_a_malformed_body_yields_400() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_gated(&base, "/api/choose", r#"{"nonsense":true}"#);

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_choose_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_gated(&base, "/api/choose", r#"{"index":0,"card":"whatever"}"#);

    assert_eq!(409, resp.status);
}

#[test]
fn post_api_choose_without_a_card_id_yields_400() {
    let (base, _guard) = spawn_full_server(None);
    post_json(
        &base,
        "/api/select",
        r#"{"deck":"choice-armed.md","depth":"recognize"}"#,
    );

    let resp = post_gated(&base, "/api/choose", r#"{"index":0}"#);

    assert_eq!(400, resp.status);
}

/// The revision proves the client saw *a* transition, not that it is looking
/// at the card it is answering: a transition that forgot to bump the revision
/// would let a pick be graded against whatever card the server moved on to
/// (the wrong-answer grading bug of 2026-07-31). Naming the card closes that
/// by construction, so no future transition can reopen it.
#[test]
fn post_api_choose_naming_another_card_yields_409() {
    let (base, _guard) = spawn_full_server(None);
    post_json(
        &base,
        "/api/select",
        r#"{"deck":"choice-armed.md","depth":"recognize"}"#,
    );
    let state = http(&base, "GET", "/api/state", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
    let revision = body["study_revision"]
        .as_u64()
        .expect("revision on state")
        .to_string();
    let current = body["card"]["id"]
        .as_str()
        .expect("the served card carries its id");

    let resp = http(
        &base,
        "POST",
        "/api/choose",
        &[
            ("Content-Type", "application/json"),
            ("X-Alix-Study-Revision", &revision),
        ],
        br#"{"index":0,"card":"0000000000000000000000000"}"#,
    );

    assert_eq!(
        409, resp.status,
        "a pick naming a card other than {current} must be refused"
    );
}

// ── Skip / acquire / promote / restart / deselect ────────────────────────

#[test]
fn post_api_skip_defers_the_current_card_without_grading_it() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_gated(&base, "/api/skip", "{}");

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

    let resp = post_gated(&base, "/api/skip", "{}");

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

    let resp = post_gated(&base, "/api/acquire", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["phase"], "body: {body}");
    // Acquiring records it (cooling ~1 min, floored out of `remaining`) and
    // moves to the other card, rather than grading it.
    assert_eq!("3 + 3", body["card"]["front"], "body: {body}");
    assert_eq!(0, body["passed"], "body: {body}");
    assert_eq!(0, body["failed"], "body: {body}");
    assert_eq!(
        1, body["acquired"],
        "the introduced card is counted: {body}"
    );
    assert_eq!(1, body["remaining"], "body: {body}");
}

#[test]
fn acquiring_every_card_leaves_a_done_session_reporting_the_next_due_instant() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);
    // Acquire both fixture cards: each records into an acquire cooldown, so the
    // sitting finishes with nothing due now but a future next-due instant.
    post_gated(&base, "/api/acquire", "{}");
    let resp = post_gated(&base, "/api/acquire", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        "done", body["phase"],
        "both cards acquired, none due now: {body}"
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let next_due = body["next_due_ms"]
        .as_u64()
        .unwrap_or_else(|| panic!("the done payload must carry next_due_ms: {body}"));
    assert!(
        next_due > now,
        "next due is a future instant (both cards still cooling): {body}"
    );
}

#[test]
fn post_api_acquire_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_gated(&base, "/api/acquire", "{}");

    assert_eq!(409, resp.status);
}

#[test]
fn post_api_promote_the_current_card_when_it_is_not_virtual_yields_400() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_gated(&base, "/api/promote", "{}");

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_promote_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_gated(&base, "/api/promote", "{}");

    assert_eq!(409, resp.status);
}

#[test]
fn post_api_restart_rebuilds_the_queue_and_resets_session_stats() {
    let (base, _guard) = spawn_test_server();
    // `cram` makes `restart`'s queue rebuild deterministic regardless of the
    // FSRS interval a "passed" grade schedules — cram serves every non-retired
    // card, due or not (`session::build_queue`).
    post_json(&base, "/api/select", r#"{"deck":"sample.md","cram":true}"#);
    let grade_resp = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);
    let grade_body: serde_json::Value = serde_json::from_slice(&grade_resp.body).unwrap();
    assert_eq!(1, grade_body["passed"], "body: {grade_body}");
    assert_eq!("3 + 3", grade_body["card"]["front"], "body: {grade_body}");

    let resp = post_gated(&base, "/api/restart", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    // The just-passed "2 + 2" is still cooling behind its floor (floors survive
    // restart), so the rebuilt sitting drops it and serves the untouched card;
    // the stats reset all the same.
    assert_eq!(1, body["remaining"], "body: {body}");
    assert_eq!(0, body["passed"], "body: {body}");
    assert_eq!("3 + 3", body["card"]["front"], "body: {body}");
}

#[test]
fn post_api_restart_with_no_active_session_yields_409() {
    let (base, _guard) = spawn_test_server();

    let resp = post_gated(&base, "/api/restart", "{}");

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

// ── Session-batched store flush ─────────────────────────────────────────

#[test]
fn a_grade_persists_immediately_and_survives_deselect() {
    let (base, guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);
    assert_eq!(200, resp.status);

    let on_disk = open_deck_store(guard.dir(), "sample.md");
    assert!(
        on_disk.last_review_ms().is_some(),
        "the grade reaches disk on the grade itself, not at a later transition"
    );

    let resp = post_json(&base, "/api/deselect", "{}");
    assert_eq!(200, resp.status);
    let on_disk = open_deck_store(guard.dir(), "sample.md");
    assert!(
        on_disk.last_review_ms().is_some(),
        "deselect leaves the flushed card in place"
    );
}

#[test]
fn ending_a_session_flushes_every_session_mutation_kind() {
    let (base, guard) = spawn_test_server();
    select_fixture(&base);
    let resp = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);
    assert_eq!(200, resp.status);
    let resp = post_gated(&base, "/api/acquire", "{}");
    assert_eq!(200, resp.status);

    let resp = post_json(&base, "/api/deselect", "{}");
    assert_eq!(200, resp.status);

    let on_disk = open_deck_store(guard.dir(), "sample.md");
    assert_eq!(
        2,
        on_disk.len(),
        "both the graded and the acquired card must land on disk"
    );
    assert!(
        on_disk.last_review_ms().is_some(),
        "the graded card's review history must land on disk"
    );
}

#[test]
fn selecting_the_next_deck_flushes_the_previous_session() {
    let (base, guard) = spawn_test_server_fixture(None, |dir| {
        std::fs::write(
            dir.join("other.md"),
            "---\nformat-version: 1\nid: \"deck-other\"\n---\n## 7 + 7 <!-- id: card-o1 -->\n14\n",
        )
        .unwrap();
    });
    select_fixture(&base);
    let resp = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);
    assert_eq!(200, resp.status);

    let resp = post_json(&base, "/api/select", r#"{"deck":"other.md"}"#);
    assert_eq!(200, resp.status);

    let on_disk = open_deck_store(guard.dir(), "sample.md");
    assert!(
        on_disk.last_review_ms().is_some(),
        "switching decks without deselecting must flush the previous session's grade"
    );
}

#[test]
fn an_administrative_mutation_still_writes_immediately() {
    let (base, guard) = spawn_test_server();
    select_fixture(&base);
    post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);
    post_json(&base, "/api/deselect", "{}");

    let resp = post_json(&base, "/api/reset", r#"{"deck":"sample.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        2, body["cards_cleared"],
        "the graded card plus the next card's presentation stamp: {body}"
    );
    let on_disk = open_deck_store(guard.dir(), "sample.md");
    assert_eq!(
        None,
        on_disk.last_review_ms(),
        "reset must reach disk right after its response, without a session boundary"
    );
}

#[test]
fn resetting_mid_session_does_not_resurrect_the_cleared_grade() {
    let (base, guard) = spawn_test_server();
    select_fixture(&base);
    let resp = post_gated(&base, "/api/grade", r#"{"grade":"passed"}"#);
    assert_eq!(200, resp.status);

    let resp = post_json(&base, "/api/reset", r#"{"deck":"sample.md"}"#);
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        2, body["cards_cleared"],
        "reset must see the in-flight grade and the presentation stamp: {body}"
    );

    let resp = post_json(&base, "/api/deselect", "{}");
    assert_eq!(200, resp.status);

    let on_disk = open_deck_store(guard.dir(), "sample.md");
    assert_eq!(
        None,
        on_disk.last_review_ms(),
        "ending the session must not write the cleared grade back to disk"
    );
}

// ── Augment (open / remove / close — no AI on this path) ─────────────────

#[test]
fn post_api_augment_open_reports_coverage_for_the_fixture_deck() {
    let (base, _guard) = spawn_test_server();

    let resp = post_json(&base, "/api/augment/open", r#"{"deck":"sample.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("sample.md", body["deck"], "body: {body}");
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

    let resp = post_json(&base, "/api/augment/open", r#"{"deck":"nope.md"}"#);

    assert_eq!(400, resp.status);
}

#[test]
fn a_rejected_augment_open_keeps_the_active_progress_store() {
    let (base, guard) = spawn_test_server_fixture(None, |dir| {
        let workspace = dir.join("dupes");
        let decks = workspace.join("decks");
        std::fs::create_dir_all(&decks).unwrap();
        std::fs::write(workspace.join("alix.toml"), "title = \"Duplicates\"\n").unwrap();
        std::fs::write(
            decks.join("one.md"),
            "---\nformat-version: 1\nid: \"deck-one\"\n---\n## first <!-- id: card-shared -->\na\n",
        )
        .unwrap();
        std::fs::write(
            decks.join("two.md"),
            "---\nformat-version: 1\nid: \"deck-two\"\n---\n## second <!-- id: card-shared -->\nb\n",
        )
        .unwrap();
    });
    assert_eq!(200, select_fixture(&base).status);

    let progress = state_root(guard.dir()).join("progress/deck-sample.json");
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&progress).unwrap()).unwrap();

    let rejected = post_json(&base, "/api/augment/open", r#"{"deck":"dupes"}"#);
    assert_eq!(409, rejected.status);

    let acquired = post_gated(&base, "/api/acquire", "{}");
    assert_eq!(200, acquired.status);

    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&progress).unwrap()).unwrap();
    assert!(
        after["revision"].as_u64().unwrap() > before["revision"].as_u64().unwrap(),
        "the accepted mutation must save through the still-active store: before={before}, after={after}"
    );
}

#[test]
fn post_api_augment_remove_on_an_empty_cache_still_succeeds_as_a_noop() {
    let (base, _guard) = spawn_test_server();
    post_json(&base, "/api/augment/open", r#"{"deck":"sample.md"}"#);

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
    post_json(&base, "/api/augment/open", r#"{"deck":"sample.md"}"#);

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

#[test]
fn post_api_augment_generate_with_a_targets_list_runs_every_target_even_after_one_fails() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    // Notes parses `{"index": "text"}`; choices parses `{"index": ["a", ...]}`.
    // The same fixed reply is a valid note but the wrong shape for choices, so
    // one fake-CLI reply splits the batch into a genuine success and a
    // genuine failure without needing two scripted replies.
    let fake = fake_reply(scripts.path(), r#"{"0": "a note"}"#);
    let (base, _guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/augment/open", r#"{"deck":"choice.md"}"#);

    let resp = post_json(
        &base,
        "/api/augment/generate",
        r#"{"targets":[{"target":"notes"},{"target":"choices"}]}"#,
    );
    assert_eq!(200, resp.status);

    let body = poll_until(&base, "/api/augment", |b| {
        b["busy"].is_null() && b["queued"].as_array().is_some_and(|q| q.is_empty())
    });

    let done = body["done"].as_array().unwrap();
    assert!(
        done.iter().any(|t| t == "notes"),
        "notes should have succeeded: body: {body}"
    );
    let failed = body["failed"].as_array().unwrap();
    let choices_failure = failed
        .iter()
        .find(|f| f["target"] == "choices")
        .unwrap_or_else(|| panic!("choices should have been attempted and failed: body: {body}"));
    assert!(
        !choices_failure["error"].as_str().unwrap().is_empty(),
        "body: {body}"
    );
}

#[test]
fn each_batch_target_carries_its_own_guidance() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    // A capturing fake CLI: append each prompt (stdin) to a log before
    // replying, so the test can see exactly what steer each spawned target
    // received. The batch runs targets one at a time in request order, so the
    // log holds the notes prompt first, then the questions prompt.
    let log = scripts.path().join("prompts.log");
    let reply = scripts.path().join("reply.json");
    std::fs::write(&reply, r#"{"0": "a note"}"#).unwrap();
    let fake = scripts.path().join("fake-claude");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >> {log}\necho '===EOM===' >> {log}\ncat {reply}\n",
            log = log.display(),
            reply = reply.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, _guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/augment/open", r#"{"deck":"choice.md"}"#);

    let resp = post_json(
        &base,
        "/api/augment/generate",
        r#"{"targets":[{"target":"notes","with":"mnemonic style"},
                       {"target":"questions","with":"vary the angle"}]}"#,
    );
    assert_eq!(200, resp.status);
    poll_until(&base, "/api/augment", |b| {
        b["busy"].is_null() && b["queued"].as_array().is_some_and(|q| q.is_empty())
    });

    let captured = std::fs::read_to_string(&log).unwrap();
    let prompts: Vec<&str> = captured.split("===EOM===").collect();
    assert!(prompts.len() >= 2, "expected two prompts, got: {captured}");
    let (notes, questions) = (prompts[0], prompts[1]);
    assert!(
        notes.contains("mnemonic style") && !notes.contains("vary the angle"),
        "the notes prompt must carry only its own steer: {notes}"
    );
    assert!(
        questions.contains("vary the angle") && !questions.contains("mnemonic style"),
        "the questions prompt must carry only its own steer: {questions}"
    );
}

/// A capturing fake CLI for batch-conversation tests: appends each call's argv
/// to `args.log` and its prompt to `prompt-<n>.log`, then replies with the
/// canned `replies[n]`. A `fail-<n>` marker file makes call n exit 1 instead.
fn fake_conversation_cli(scripts: &TempDir, replies: &[&str]) -> PathBuf {
    for (i, reply) in replies.iter().enumerate() {
        std::fs::write(scripts.path().join(format!("reply-{i}")), reply).unwrap();
    }
    let d = scripts.path().display();
    let fake = scripts.path().join("fake-claude");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\n\
             N=$(cat {d}/n 2>/dev/null || echo 0)\n\
             echo \"$@\" >> {d}/args.log\n\
             cat >> {d}/prompt-$N.log\n\
             echo $((N+1)) > {d}/n\n\
             if [ -f {d}/fail-$N ]; then exit 1; fi\n\
             cat {d}/reply-$N\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    fake
}

/// The uuid following `flag` in a logged argv line.
fn session_id_after(line: &str, flag: &str) -> String {
    line.split_whitespace()
        .skip_while(|w| *w != flag)
        .nth(1)
        .unwrap_or_else(|| panic!("no {flag} id in: {line}"))
        .to_string()
}

#[test]
fn a_batch_reuses_one_claude_session_across_targets() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let reply = r#"{"0": ["w1","w2","w3"]}"#;
    let fake = fake_conversation_cli(&scripts, &[reply, reply]);
    let (base, _guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/augment/open", r#"{"deck":"choice.md"}"#);

    let resp = post_json(
        &base,
        "/api/augment/generate",
        r#"{"targets":[{"target":"choices"},{"target":"questions"}]}"#,
    );
    assert_eq!(200, resp.status);
    poll_until(&base, "/api/augment", |b| {
        b["busy"].is_null() && b["queued"].as_array().is_some_and(|q| q.is_empty())
    });

    let args = std::fs::read_to_string(scripts.path().join("args.log")).unwrap();
    let calls: Vec<&str> = args.lines().collect();
    assert_eq!(2, calls.len(), "{args}");
    assert!(calls[0].contains("--session-id"), "{args}");
    assert!(calls[1].contains("--resume"), "{args}");
    let id = session_id_after(calls[0], "--session-id");
    assert!(
        calls[1].contains(&id),
        "one conversation across the batch: {args}"
    );

    let primer = std::fs::read_to_string(scripts.path().join("prompt-0.log")).unwrap();
    let follow_up = std::fs::read_to_string(scripts.path().join("prompt-1.log")).unwrap();
    assert!(
        primer.contains("3 + 3"),
        "the first call lists the cards: {primer}"
    );
    assert!(
        follow_up.contains("already provided in this conversation"),
        "{follow_up}"
    );
    assert!(
        !follow_up.contains("3 + 3"),
        "a follow-up must not re-list the cards: {follow_up}"
    );
}

#[test]
fn a_failed_target_starts_a_fresh_session_for_the_rest_of_the_batch() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let reply = r#"{"0": ["w1","w2","w3"]}"#;
    let fake = fake_conversation_cli(&scripts, &[reply, reply, reply]);
    // The second call (the questions target) dies; the batch must carry on
    // with a FRESH session for keypoints rather than resuming a dead one.
    std::fs::write(scripts.path().join("fail-1"), "").unwrap();
    let (base, _guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/augment/open", r#"{"deck":"choice.md"}"#);

    let resp = post_json(
        &base,
        "/api/augment/generate",
        r#"{"targets":[{"target":"choices"},{"target":"questions"},{"target":"keypoints"}]}"#,
    );
    assert_eq!(200, resp.status);
    poll_until(&base, "/api/augment", |b| {
        b["busy"].is_null()
            && b["queued"].as_array().is_some_and(|q| q.is_empty())
            && b["failed"].as_array().is_some_and(|f| !f.is_empty())
    });

    let args = std::fs::read_to_string(scripts.path().join("args.log")).unwrap();
    let calls: Vec<&str> = args.lines().collect();
    assert_eq!(3, calls.len(), "{args}");
    let first = session_id_after(calls[0], "--session-id");
    assert!(calls[1].contains("--resume"), "{args}");
    let third = session_id_after(calls[2], "--session-id");
    assert_ne!(first, third, "a failed call must not be resumed: {args}");
    let reprime = std::fs::read_to_string(scripts.path().join("prompt-2.log")).unwrap();
    assert!(
        reprime.contains("3 + 3"),
        "the fresh session re-primes the roster: {reprime}"
    );
}

#[test]
fn a_single_target_batch_stays_a_stateless_one_shot() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_conversation_cli(&scripts, &[r#"{"0": "a note"}"#]);
    let (base, _guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/augment/open", r#"{"deck":"choice.md"}"#);

    let resp = post_json(
        &base,
        "/api/augment/generate",
        r#"{"targets":[{"target":"notes"}]}"#,
    );
    assert_eq!(200, resp.status);
    poll_until(&base, "/api/augment", |b| {
        b["busy"].is_null() && b["done"].as_array().is_some_and(|d| !d.is_empty())
    });

    let args = std::fs::read_to_string(scripts.path().join("args.log")).unwrap();
    assert!(
        !args.contains("--session-id") && !args.contains("--resume"),
        "one call gains nothing from a session: {args}"
    );
    let prompt = std::fs::read_to_string(scripts.path().join("prompt-0.log")).unwrap();
    assert!(
        prompt.contains("3 + 3"),
        "the one-shot lists its cards: {prompt}"
    );
}

/// A small two-deck workspace written into the test decks dir by the
/// `spawn_full_server_fixture` closure: 2 + 3 cards, so a union open reports 5.
fn write_workspace_fixture(dir: &Path) {
    let ws = dir.join("ws");
    let members = ws.join("decks");
    std::fs::create_dir_all(&members).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"WS\"\n").unwrap();
    std::fs::write(
        members.join("m1.md"),
        "---\nformat-version: 1\nid: \"deck-workone\"\n---\n## q1 <!-- id: card-w1 -->\na1\n## q2 <!-- id: card-w2 -->\na2\n",
    )
    .unwrap();
    std::fs::write(
        members.join("m2.md"),
        "---\nformat-version: 1\nid: \"deck-worktwo\"\n---\n## q3 <!-- id: card-w3 -->\na3\n## q4 <!-- id: card-w4 -->\na4\n## q5 <!-- id: card-w5 -->\na5\n",
    )
    .unwrap();
}

/// A folder holding a deck but no `alix.toml` manifest. `workspace::has_decks`
/// is still true (it is drillable, `resolve_row` still classifies it as
/// `Resolved::Many` since it has members), but `workspace::is_workspace` is
/// false. This is exactly the row shape the deadline route's
/// `is_workspace(&dir)` guard exists to reject.
fn write_plain_folder_fixture(dir: &Path) {
    let folder = dir.join("plainfolder");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("loose.md"),
        "---\nformat-version: 1\nid: \"deck-loose\"\n---\n## q <!-- id: card-lq1 -->\na\n",
    )
    .unwrap();
}

#[test]
fn augment_open_on_a_workspace_unions_member_cards_and_offers_the_icon_row() {
    let (base, _guard) = spawn_full_server_fixture(None, write_workspace_fixture, |_opts| {});

    let resp = post_json(&base, "/api/augment/open", r#"{"deck":"ws"}"#);
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(5, body["cards"], "union of both members: body: {body}");
    let rows = body["rows"].as_array().unwrap();
    let icon = rows
        .iter()
        .find(|r| r["kind"] == "icon")
        .unwrap_or_else(|| panic!("a workspace open must offer the icon row: {body}"));
    assert_eq!(0, icon["covered"], "no assets/icon.* yet: {body}");
    assert_eq!(1, icon["eligible"], "body: {body}");

    // A plain deck's screen must NOT offer the icon target.
    let resp = post_json(&base, "/api/augment/open", r#"{"deck":"sample.md"}"#);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(
        body["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["kind"] != "icon"),
        "body: {body}"
    );
}

#[test]
fn icon_target_generates_the_workspace_emblem_with_its_steer() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    // A capturing fake CLI (same pattern as the guidance test above): log the
    // prompt, then reply with a minimal SVG.
    let log = scripts.path().join("prompts.log");
    let reply = scripts.path().join("reply.svg");
    std::fs::write(&reply, r#"<svg viewBox="0 0 24 24"><circle r="8"/></svg>"#).unwrap();
    let fake = scripts.path().join("fake-claude");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >> {log}\ncat {reply}\n",
            log = log.display(),
            reply = reply.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, guard) = spawn_full_server_fixture(Some(&fake), write_workspace_fixture, |_opts| {});
    post_json(&base, "/api/augment/open", r#"{"deck":"ws"}"#);

    let resp = post_json(
        &base,
        "/api/augment/generate",
        r#"{"targets":[{"target":"icon","with":"a compass rose"}]}"#,
    );
    assert_eq!(200, resp.status);
    let body = poll_until(&base, "/api/augment", |b| {
        b["busy"].is_null() && b["queued"].as_array().is_some_and(|q| q.is_empty())
    });

    assert!(
        body["done"].as_array().unwrap().iter().any(|t| t == "icon"),
        "body: {body}"
    );
    let icon_path = guard.dir.path().join("ws/assets/icon.svg");
    assert!(
        icon_path.exists(),
        "the emblem was written to the workspace"
    );
    let icon_row = body["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "icon")
        .unwrap()
        .clone();
    assert_eq!(1, icon_row["covered"], "coverage sees the new file: {body}");
    let captured = std::fs::read_to_string(&log).unwrap();
    assert!(
        captured.contains("a compass rose"),
        "the icon prompt must carry the card's steer: {captured}"
    );
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

    let resp = post_json(&base, "/api/exam/start", r#"{"deck":"trace.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("answering", body["phase"], "body: {body}");
    assert_eq!(true, body["is_trace"], "body: {body}");
    assert_eq!("trace.md", body["deck"], "body: {body}");
    assert_eq!(1, body["total"], "body: {body}");
    assert_eq!(0, body["current"], "body: {body}");
    assert!(body["question"].as_str().is_some(), "body: {body}");
}

#[test]
fn post_api_exam_close_returns_the_picker_state_dto() {
    let (base, _guard) = spawn_full_server(None);
    post_json(&base, "/api/exam/start", r#"{"deck":"trace.md"}"#);

    let resp = post_json(&base, "/api/exam/close", "{}");

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("review", body["kind"], "body: {body}");
    assert_eq!("select", body["phase"], "body: {body}");
}

#[test]
fn post_api_exam_start_with_an_unknown_deck_yields_400() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/exam/start", r#"{"deck":"nope.md"}"#);

    assert_eq!(400, resp.status);
}

#[test]
fn post_api_exam_start_on_a_deck_with_no_exam_yields_409() {
    // `sample.md` declares no `source:` and isn't a trace — `has_exam()`
    // is false, so it can never be sat.
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/exam/start", r#"{"deck":"sample.md"}"#);

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
    post_json(&base, "/api/exam/start", r#"{"deck":"trace.md"}"#);

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

/// Coverage round from the mutation gate: /api/remove had no effective
/// end-to-end test (deleting its whole match arm survived the suite).
#[test]
fn remove_drops_the_current_card_from_the_session_and_the_deck_file() {
    let (base, guard) = spawn_test_server();
    select_fixture(&base);
    let state = http(&base, "GET", "/api/state", &[], &[]);
    let before: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
    let front = before["card"]["front"].as_str().unwrap().to_string();

    let resp = post_gated(&base, "/api/remove", "{}");
    assert_eq!(200, resp.status);
    let after: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_ne!(
        front, after["card"]["front"],
        "the session moved on: {after}"
    );
    let deck = std::fs::read_to_string(guard.dir().join("sample.md")).unwrap();
    assert!(
        !deck.contains(&front),
        "the removed card must leave the deck file: {deck}"
    );
}

/// Arm-existence smokes for routes whose deletion survived the gate: the
/// distinguishing status is 409 (no active sitting), never the 404 a
/// deleted arm would produce.
#[test]
fn exam_answer_and_remediate_arms_exist_without_a_sitting() {
    let (base, _guard) = spawn_test_server();
    let resp = post_json(&base, "/api/exam/answer", r#"{"text":"x"}"#);
    assert_eq!(409, resp.status);
    let resp = post_json(&base, "/api/exam/remediate", "{}");
    assert_eq!(409, resp.status);
}

/// The exam answer arm, in-flow: setting the text before grading reaches
/// the sitting (the graded answer is the one set here).
#[test]
fn exam_answer_sets_the_text_the_grade_submits() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(
        scripts.path(),
        r#"{"verdict":"pass","feedback":"good","missed":[]}"#,
    );
    let (base, _guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/exam/start", r#"{"deck":"trace.md"}"#);

    let resp = post_json(&base, "/api/exam/answer", r#"{"text":"my answer"}"#);
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("answering", body["phase"], "body: {body}");

    let resp = post_json(&base, "/api/exam/grade", r#"{"text":"my answer"}"#);
    assert_eq!(200, resp.status);
    let body = poll_until(&base, "/api/exam", |b| b["phase"] != "grading");
    assert_eq!("results", body["phase"], "body: {body}");
}

/// The web generate flow end to end: start answers a generating phase, the
/// poll settles, the deck lands in the destination, close frees the slot
/// (a later poll is 409), and the new deck resolves by name.
#[test]
fn generate_lands_the_deck_then_close_frees_the_slot() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(
        scripts.path(),
        "---\ntitle: Generated\n---\n## q1\na1\n\n## q2\na2\n",
    );
    let (base, guard) = spawn_full_server(Some(&fake));

    let resp = post_json(
        &base,
        "/api/generate",
        r#"{"url":"https://example.org/article"}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("generating", body["phase"], "body: {body}");

    let body = poll_until(&base, "/api/generate", |b| b["phase"] != "generating");
    assert_eq!("done", body["phase"], "body: {body}");
    let deck = body["deck"].as_str().expect("landed deck name").to_string();
    assert!(
        guard.dir().join("decks").join(&deck).exists() || guard.dir().join(&deck).exists(),
        "the generated deck landed: {deck}"
    );

    let resp = post_json(&base, "/api/generate/close", "{}");
    assert_eq!(200, resp.status);
    let resp = http(&base, "GET", "/api/generate", &[], &[]);
    assert_eq!(409, resp.status, "the closed slot polls as 409");
}

/// share/zip produces a real zip of the staged decks, and receive/zip lands
/// it back: the round trip that covers both archive arms.
#[test]
fn share_zip_roundtrips_through_receive_zip() {
    let (base, guard) = spawn_test_server();

    let resp = http(&base, "GET", "/api/share/zip", &[], &[]);
    assert_eq!(200, resp.status);
    assert_eq!(Some("application/zip"), resp.header("Content-Type"));
    assert!(resp.body.len() > 4, "a non-empty archive");
    assert_eq!(&resp.body[..2], b"PK", "a zip magic header");

    let inbox = guard.dir().join("inbox");
    std::fs::create_dir(&inbox).unwrap();
    std::fs::write(inbox.join("alix.toml"), "title = \"Inbox\"\n").unwrap();
    let resp = http(
        &base,
        "POST",
        "/api/receive/zip?dest=inbox",
        &[("Content-Type", "application/zip")],
        &resp.body,
    );
    assert_eq!(
        200,
        resp.status,
        "{:?}",
        String::from_utf8_lossy(&resp.body)
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("done", body["phase"], "body: {body}");
}

/// Close arms answer 200 with no job active (a deleted arm would 404).
#[test]
fn share_and_receive_close_arms_exist() {
    let (base, _guard) = spawn_test_server();
    assert_eq!(200, post_json(&base, "/api/share/close", "{}").status);
    assert_eq!(200, post_json(&base, "/api/receive/close", "{}").status);
    assert_eq!(
        200,
        post_json(&base, "/api/remote/generate/close", "{}").status
    );
}

/// The walk tutor: a question lands in the walk transcript, and a note
/// condensed from it is appended to the trace deck file.
#[test]
fn walk_ask_question_then_note_writes_to_the_checkpoint() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let count = scripts.path().join("calls");
    let fake = scripts.path().join("fake-walk-tutor");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\necho x >> {count}\nif [ \"$(wc -l < {count})\" -gt 1 ]; then echo '- the hop forwards the value'; else echo 'a walk answer'; fi\n",
            count = count.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/select", r#"{"deck":"trace.md"}"#);

    let resp = post_gated(&base, "/api/walk/ask", r#"{"question":"why?"}"#);
    assert_eq!(200, resp.status);
    let body = poll_until(&base, "/api/walk/ask", |b| b["thinking"] == false);
    assert_eq!(1, body["transcript"].as_array().unwrap().len(), "{body}");

    let resp = post_gated(&base, "/api/walk/ask/note", "{}");
    assert_eq!(200, resp.status);
    poll_until(&base, "/api/walk/ask", |b| b["thinking"] == false);
    let deck = std::fs::read_to_string(guard.dir().join("trace.md")).unwrap();
    assert!(
        deck.contains("the hop forwards the value"),
        "the note landed in the trace deck: {deck}"
    );
}

/// Recent history is recorded through the owner and drives the listing
/// order: the most recently selected deck leads the recent section.
#[test]
fn recent_ordering_over_the_wire_follows_selects() {
    let _lock = exec_lock();
    let (base, _guard) = spawn_full_server(None);

    post_json(&base, "/api/select", r#"{"deck":"sample.md"}"#);
    post_gated(&base, "/api/deselect", "{}");
    post_json(&base, "/api/select", r#"{"deck":"choice.md"}"#);
    post_gated(&base, "/api/deselect", "{}");

    let resp = http(&base, "GET", "/api/decks", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let recent: Vec<&str> = body["recent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    let sample = recent.iter().position(|n| *n == "sample.md").unwrap();
    let choice = recent.iter().position(|n| *n == "choice.md").unwrap();
    assert!(
        choice < sample,
        "the later select leads the recent ordering: {recent:?}"
    );
}

/// Every gated route, not just one: a stale echo must 409 and the session
/// must not move. Kills the guard-disabling mutants route by route.
#[test]
fn every_gated_route_rejects_a_stale_revision_and_mutates_nothing() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(scripts.path(), "an answer");
    let (base, _guard) = spawn_full_server(Some(&fake));
    let resp = post_json(&base, "/api/select", r#"{"deck":"choice-armed.md"}"#);
    assert_eq!(200, resp.status);

    let gated: &[(&str, &str)] = &[
        ("/api/grade", r#"{"grade":"passed"}"#),
        ("/api/skip", "{}"),
        ("/api/acquire", "{}"),
        ("/api/check", r#"{"lines":["x"]}"#),
        ("/api/reveal", "{}"),
        ("/api/choose", r#"{"index":0,"card":"CURRENT"}"#),
        ("/api/remove", "{}"),
        ("/api/promote", "{}"),
        ("/api/restart", "{}"),
        ("/api/ask", r#"{"question":"why?"}"#),
        ("/api/ask/note", "{}"),
        ("/api/ask/card/draft", "{}"),
        ("/api/ask/card/create", r#"{"front":"f","back":["b"]}"#),
    ];
    for (path, body) in gated {
        let state = http(&base, "GET", "/api/state", &[], &[]);
        let before: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
        let fresh = before["study_revision"]
            .as_u64()
            .expect("revision on state");
        let stale = fresh.wrapping_sub(1).to_string();
        // The bodies are fixed but the card id is minted per run: `CURRENT`
        // stands in for it, so the stale revision is the only thing wrong.
        let body = body.replace("CURRENT", before["card"]["id"].as_str().unwrap_or_default());

        let resp = http(
            &base,
            "POST",
            path,
            &[
                ("Content-Type", "application/json"),
                ("X-Alix-Study-Revision", &stale),
            ],
            body.as_bytes(),
        );
        assert_eq!(409, resp.status, "{path} must reject a stale echo");

        let state = http(&base, "GET", "/api/state", &[], &[]);
        let after: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
        assert_eq!(
            fresh,
            after["study_revision"].as_u64().unwrap(),
            "{path} must not advance the revision on a stale echo"
        );
        assert_eq!(
            before["card"]["front"], after["card"]["front"],
            "{path} must not move the session on a stale echo"
        );
    }
}

/// Each accepted card-advancing mutation strictly advances the revision, so
/// the previous echo goes stale. Kills the bump-corruption mutants (a
/// revision pinned at its old value re-accepts the replay forever).
#[test]
fn every_accepted_mutation_advances_the_revision() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    // `remove` runs last: it deletes the current card from the deck file, so
    // the earlier routes keep the fixture's full queue.
    for (path, body) in [
        ("/api/skip", "{}"),
        ("/api/acquire", "{}"),
        ("/api/restart", "{}"),
        ("/api/grade", r#"{"grade":"passed"}"#),
        ("/api/remove", "{}"),
    ] {
        let state = http(&base, "GET", "/api/state", &[], &[]);
        let before: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
        let fresh = before["study_revision"].as_u64().unwrap();

        let resp = post_gated(&base, path, body);
        assert_eq!(200, resp.status, "{path}");
        let resp_body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        let bumped = resp_body["study_revision"].as_u64().unwrap();
        assert!(
            bumped > fresh,
            "{path} must strictly advance the revision ({fresh} -> {bumped})"
        );

        // The just-used echo is now stale: the replay is refused.
        let resp = http(
            &base,
            "POST",
            path,
            &[
                ("Content-Type", "application/json"),
                ("X-Alix-Study-Revision", &fresh.to_string()),
            ],
            body.as_bytes(),
        );
        assert_eq!(409, resp.status, "{path} replay with the used echo");
    }
}

/// The transition family: select, deselect, browse, and a trace-deck (walk)
/// select each replace or drop the active session, and every one must
/// strictly advance `study_revision`, or an in-flight card-relative request
/// from the old session could land on the new one. Browse and walk payloads
/// carry no revision, so their bumps are measured through the next select's
/// delta: transition plus select is at least two.
#[test]
fn every_session_transition_strictly_advances_the_revision() {
    // The full server: a trace-deck select routes through the walk classifier.
    let (base, _guard) = spawn_full_server(None);
    let rev = |b: &serde_json::Value| b["study_revision"].as_u64().unwrap();

    let state = http(&base, "GET", "/api/state", &[], &[]);
    let r0 = rev(&serde_json::from_slice(&state.body).unwrap());

    let resp = post_json(&base, "/api/select", r#"{"deck":"sample.md"}"#);
    assert_eq!(200, resp.status);
    let r1 = rev(&serde_json::from_slice(&resp.body).unwrap());
    assert!(r1 > r0, "select must advance the revision ({r0} -> {r1})");

    let resp = post_gated(&base, "/api/deselect", "{}");
    assert_eq!(200, resp.status);
    let r2 = rev(&serde_json::from_slice(&resp.body).unwrap());
    assert!(r2 > r1, "deselect must advance the revision ({r1} -> {r2})");

    let resp = post_json(&base, "/api/select", r#"{"deck":"sample.md"}"#);
    assert_eq!(200, resp.status);
    let r3 = rev(&serde_json::from_slice(&resp.body).unwrap());
    let resp = post_json(&base, "/api/browse", r#"{"deck":"sample.md"}"#);
    assert_eq!(200, resp.status);
    let resp = post_json(&base, "/api/select", r#"{"deck":"sample.md"}"#);
    assert_eq!(200, resp.status);
    let r4 = rev(&serde_json::from_slice(&resp.body).unwrap());
    assert!(
        r4 >= r3 + 2,
        "browse and the following select must each advance the revision ({r3} -> {r4})"
    );

    let resp = post_json(&base, "/api/select", r#"{"deck":"trace.md"}"#);
    assert_eq!(200, resp.status);
    let resp = post_json(&base, "/api/select", r#"{"deck":"sample.md"}"#);
    assert_eq!(200, resp.status);
    let r5 = rev(&serde_json::from_slice(&resp.body).unwrap());
    assert!(
        r5 >= r4 + 2,
        "a walk select and the following select must each advance the revision ({r4} -> {r5})"
    );
}

/// The dropped-reply guard: every card-relative mutation echoes the state's
/// `study_revision`; a replay with the revision that was current when the
/// lost reply's request was accepted must 409 and mutate nothing, so a
/// retried grade can never grade the NEXT card.
#[test]
fn a_replayed_grade_with_a_stale_revision_conflicts_and_mutates_nothing() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let state = http(&base, "GET", "/api/state", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
    let revision = body["study_revision"].as_u64().expect("revision on state");
    let first_front = body["card"]["front"].clone();

    let resp = http(
        &base,
        "POST",
        "/api/acquire",
        &[
            ("Content-Type", "application/json"),
            ("X-Alix-Study-Revision", &revision.to_string()),
        ],
        b"{}",
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let advanced_front = body["card"]["front"].clone();
    assert_ne!(first_front, advanced_front, "the session advanced: {body}");

    // The replay carries the OLD revision (its reply was "lost").
    let resp = http(
        &base,
        "POST",
        "/api/acquire",
        &[
            ("Content-Type", "application/json"),
            ("X-Alix-Study-Revision", &revision.to_string()),
        ],
        b"{}",
    );
    assert_eq!(409, resp.status, "a stale replay must conflict");
    let state = http(&base, "GET", "/api/state", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
    assert_eq!(
        advanced_front, body["card"]["front"],
        "the stale replay must not have advanced the session: {body}"
    );
}

/// Clients that do not speak the revision contract are refused loudly, not
/// silently accepted with no protection.
#[test]
fn an_oversized_json_body_is_refused_instead_of_being_buffered() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    // Just past the 256 KiB cap. The point is not the exact number: an
    // uncapped reader keeps consuming while a client keeps sending, so the
    // limit is what stops one request from growing the server's memory. That
    // ordinary bodies still parse is covered by every other test in this file.
    // A REAL deck plus padding in an ignored field: parsed, this body is
    // valid and the route answers on the deck's own merits. Only its size can
    // make it a 400, so the assertion cannot pass for the wrong reason.
    let padded = format!(
        "{{\"deck\":\"sample.md\",\"pad\":\"{}\"}}",
        "x".repeat(256 * 1024 + 1)
    );
    let oversized = post_json(&base, "/api/exam/start", &padded);

    let small = format!("{{\"deck\":\"sample.md\",\"pad\":\"{}\"}}", "x".repeat(16));
    let ordinary = post_json(&base, "/api/exam/start", &small);

    assert_ne!(
        oversized.status, ordinary.status,
        "the same body must be treated differently once it is oversized \
         (both answered {})",
        ordinary.status
    );
    assert_eq!(
        400, oversized.status,
        "an oversized body must be refused, not read to the end"
    );
}

#[test]
fn a_card_relative_mutation_without_the_revision_header_is_a_400() {
    let (base, _guard) = spawn_test_server();
    select_fixture(&base);

    let resp = post_json(&base, "/api/acquire", "{}");
    assert_eq!(400, resp.status);
}

/// ADR 0027: advancing the card makes an older tutor completion ineligible.
/// The fake backend parks on a FIFO; the card advances while it thinks; the
/// released answer must be dropped, never shown under the new card.
#[test]
fn a_tutor_answer_arriving_after_a_card_advance_is_dropped() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let started = scripts.path().join("started");
    let fifo = scripts.path().join("release.fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs")
            .success()
    );
    let fake = scripts.path().join("parked-tutor");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\n: > {started}\nread _ < {fifo}\necho \"a late answer\"\n",
            started = started.display(),
            fifo = fifo.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, _guard) = spawn_full_server(Some(&fake));
    select_fixture(&base);

    let resp = post_gated(&base, "/api/ask", r#"{"question":"why?"}"#);
    assert_eq!(200, resp.status);
    let probe_started = Instant::now();
    while !started.exists() {
        assert!(
            probe_started.elapsed() < Duration::from_secs(5),
            "the tutor never spawned the fake backend"
        );
        thread::sleep(Duration::from_millis(5));
    }

    let resp = post_gated(&base, "/api/skip", "{}");
    assert_eq!(200, resp.status);
    std::fs::write(&fifo, "go\n").unwrap();

    let body = poll_until(&base, "/api/ask", |b| b["thinking"] == false);
    assert_eq!(
        0,
        body["transcript"].as_array().unwrap().len(),
        "the late answer must not enter the new card's transcript: {body}"
    );
}

#[test]
fn server_shutdown_cancels_the_in_flight_tutor_worker() {
    fn process_exists(pid: u32) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let started = scripts.path().join("started");
    let pid_file = scripts.path().join("pid");
    let fifo = scripts.path().join("release.fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs")
            .success()
    );
    let fake = scripts.path().join("parked-tutor");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\necho $$ > {pid_file}\n: > {started}\nread _ < {fifo}\necho done\n",
            pid_file = pid_file.display(),
            started = started.display(),
            fifo = fifo.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, guard) = spawn_full_server(Some(&fake));
    select_fixture(&base);
    assert_eq!(
        200,
        post_gated(&base, "/api/ask", r#"{"question":"why?"}"#).status
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the tutor worker never started");
        thread::yield_now();
    }
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        process_exists(pid),
        "the parked tutor must still be running"
    );

    drop(guard);
    let survived_shutdown = process_exists(pid);

    if survived_shutdown {
        std::fs::write(&fifo, "go\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while process_exists(pid) && Instant::now() < deadline {
            thread::yield_now();
        }
    }
    assert!(
        !survived_shutdown,
        "shutdown returned while tutor subprocess {pid} was still alive"
    );
}

#[test]
fn server_shutdown_cancels_tutor_descendant_processes() {
    fn process_exists(pid: u32) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let started = scripts.path().join("started");
    let pid_file = scripts.path().join("descendant-pid");
    let fifo = scripts.path().join("release.fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs")
            .success()
    );
    let descendant = scripts.path().join("descendant");
    std::fs::write(
        &descendant,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\necho $$ > {pid_file}\n: > {started}\nread _ < {fifo}\n",
            pid_file = pid_file.display(),
            started = started.display(),
            fifo = fifo.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&descendant, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fake = scripts.path().join("tutor-with-descendant");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\n{descendant} &\nwait\n",
            descendant = descendant.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, guard) = spawn_full_server(Some(&fake));
    select_fixture(&base);
    assert_eq!(
        200,
        post_gated(&base, "/api/ask", r#"{"question":"why?"}"#).status
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.exists() {
        assert!(
            Instant::now() < deadline,
            "the tutor descendant never started"
        );
        thread::yield_now();
    }
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        process_exists(pid),
        "the parked descendant must still be running"
    );

    drop(guard);
    let survived_shutdown = process_exists(pid);

    if survived_shutdown {
        std::fs::write(&fifo, "go\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while process_exists(pid) && Instant::now() < deadline {
            thread::yield_now();
        }
    }
    assert!(
        !survived_shutdown,
        "shutdown returned while tutor descendant {pid} was still alive"
    );
}

#[test]
fn server_shutdown_cancels_the_in_flight_remote_tutor_worker() {
    fn process_exists(pid: u32) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let started = scripts.path().join("started");
    let pid_file = scripts.path().join("pid");
    let fifo = scripts.path().join("release.fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs")
            .success()
    );
    let fake = scripts.path().join("parked-remote-tutor");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\necho $$ > {pid_file}\n: > {started}\nread _ < {fifo}\necho done\n",
            pid_file = pid_file.display(),
            started = started.display(),
            fifo = fifo.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, guard) = spawn_full_server(Some(&fake));
    let response = post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},
            "history":[],"question":"why?"}"#,
    );
    assert_eq!(200, response.status);

    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.exists() {
        assert!(
            Instant::now() < deadline,
            "the remote tutor worker never started"
        );
        thread::yield_now();
    }
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        process_exists(pid),
        "the parked remote tutor must still be running"
    );

    drop(guard);
    let survived_shutdown = process_exists(pid);

    if survived_shutdown {
        std::fs::write(&fifo, "go\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while process_exists(pid) && Instant::now() < deadline {
            thread::yield_now();
        }
    }
    assert!(
        !survived_shutdown,
        "shutdown returned while remote tutor subprocess {pid} was still alive"
    );
}

/// The note variant of the same staleness: a condense completing after the
/// card advanced must not write into the deck file.
#[test]
fn a_tutor_note_arriving_after_a_card_advance_is_not_written() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let started = scripts.path().join("started");
    let fifo = scripts.path().join("release.fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs")
            .success()
    );
    // First call answers immediately (builds the transcript); the second
    // (the condense) parks on the FIFO.
    let count = scripts.path().join("calls");
    let fake = scripts.path().join("parked-note");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\necho x >> {count}\nif [ \"$(wc -l < {count})\" -gt 1 ]; then : > {started}; read _ < {fifo}; echo 'NOTE: a very late note'; else echo 'an answer'; fi\n",
            count = count.display(),
            started = started.display(),
            fifo = fifo.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, guard) = spawn_full_server(Some(&fake));
    select_fixture(&base);

    post_gated(&base, "/api/ask", r#"{"question":"why?"}"#);
    poll_until(&base, "/api/ask", |b| b["thinking"] == false);
    let resp = post_gated(&base, "/api/ask/note", "{}");
    assert_eq!(200, resp.status);
    let probe_started = Instant::now();
    while !started.exists() {
        assert!(
            probe_started.elapsed() < Duration::from_secs(5),
            "the condense never reached the fake backend"
        );
        thread::sleep(Duration::from_millis(5));
    }

    post_gated(&base, "/api/skip", "{}");
    std::fs::write(&fifo, "go\n").unwrap();
    poll_until(&base, "/api/ask", |b| b["thinking"] == false);

    let deck = std::fs::read_to_string(guard.dir().join("sample.md")).unwrap();
    assert!(
        !deck.contains("a very late note"),
        "the stale note must not land in the deck file: {deck}"
    );
}

/// A passed exam's mastery write rides the owner's save accounting: a
/// transient save failure surfaces as `save_error`, and the mastery is
/// retried by the next transition flush instead of being dropped silently
/// with the store replacement.
#[test]
fn a_mastery_save_failure_surfaces_and_the_mastery_survives_repair() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(
        scripts.path(),
        r#"{"verdict":"pass","feedback":"nice work retracing it","missed":[]}"#,
    );
    let (base, guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/exam/start", r#"{"deck":"trace.md"}"#);
    let state_dir = state_root(guard.dir());
    break_state_dir(&state_dir);

    post_json(
        &base,
        "/api/exam/grade",
        r#"{"text":"it forwards the value hop by hop, first then second"}"#,
    );
    let body = poll_until(&base, "/api/exam", |b| b["phase"] != "grading");
    assert_eq!(true, body["passed"], "body: {body}");

    let resp = http(&base, "GET", "/api/state", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert!(
        body["save_error"].as_str().is_some(),
        "the failed mastery save must surface: {body}"
    );

    repair_state_dir(&state_dir);
    let resp = post_json(&base, "/api/exam/close", "{}");
    assert_eq!(200, resp.status);

    let resp = http(&base, "GET", "/api/decks", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let mastered = body["recent"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "trace.md")
        .map(|d| d["mastered"].clone());
    assert_eq!(
        Some(serde_json::Value::Bool(true)),
        mastered,
        "the mastery must survive the repaired flush: {body}"
    );
}

// ── Walk (a two-hop trace deck) ───────────────────────────────────────────

/// `/api/select` now classifies through the real `assemble::select` (no more
/// per-fixture `build_walk` stub) — this pins that the trace fixture still
/// round-trips as a walk through that real classifier, not a harness replica.
#[test]
fn selecting_a_trace_deck_returns_a_walk_through_the_real_classifier() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/select", r#"{"deck":"trace.md"}"#);

    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("walk", body["kind"], "body: {body}");
}

#[test]
fn selecting_a_trace_deck_returns_a_walk_dto_not_a_review_state() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/select", r#"{"deck":"trace.md"}"#);

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
    post_json(&base, "/api/select", r#"{"deck":"trace.md"}"#);

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
    post_json(&base, "/api/select", r#"{"deck":"trace.md"}"#);
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
    post_json(&base, "/api/select", r#"{"deck":"trace.md"}"#);

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
    let select_resp = post_json(&base, "/api/select", r#"{"deck":"trace.md"}"#);
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
    std::fs::write(dir.path().join("sample.md"), FIXTURE_DECK).unwrap();
    let store_path = state_root(dir.path());
    let store = open_instance_store(dir.path());
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
                max_session: 10,
                new_cards_percent: 30,
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
fn ask_card_draft_create_then_promote_round_trips_a_learner_card_into_the_deck() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(scripts.path(), "## term?\ndefinition\n");
    let (base, _guard) = spawn_full_server(Some(&fake));
    select_fixture(&base);

    // Seed a tutor exchange so the transcript is non-empty before drafting.
    let resp = post_gated(&base, "/api/ask", r#"{"question":"why does this matter?"}"#);
    assert_eq!(200, resp.status);
    // The wait idiom this test reuses verbatim: `poll_until` (this file,
    // defined above at the `fn poll_until` declaration), a bounded (up to
    // 5s, 250 * 20ms) loop on the `thinking` condition, the same idiom
    // `exam_grade_on_a_trace_deck_walks_from_answering_to_a_passing_result_via_the_fake_backend`
    // and `walk_predict_with_auto_grade_resolves_a_verdict_via_the_fake_backend`
    // already use to wait on this exact kind of background ask/exam job.
    let body = poll_until(&base, "/api/ask", |b| !b["thinking"].as_bool().unwrap());
    assert_eq!(
        1,
        body["transcript"].as_array().unwrap().len(),
        "body: {body}"
    );

    // Draft a card from the conversation.
    let resp = post_gated(&base, "/api/ask/card/draft", "{}");
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
    let resp = post_gated(
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
    assert!(create_body["id"].as_str().is_some(), "body: {create_body}");

    // Drillable, not just stored: cram-reselect (the same determinism idiom
    // `post_api_restart_rebuilds_the_queue_and_resets_session_stats` uses)
    // pulls every non-retired card into the queue regardless of due date. The
    // newly minted virtual card already has a store entry (`mint_tutor_card`
    // seeds one), so `build_queue` sorts it into the "due" group, ahead of
    // the two never-graded fixture cards in "fresh": it's the first card the
    // reselected session serves.
    let resp = post_json(&base, "/api/select", r#"{"deck":"sample.md","cram":true}"#);
    assert_eq!(200, resp.status);
    let select_body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(3, select_body["remaining"], "body: {select_body}");
    assert_eq!(
        "edited term?", select_body["card"]["front"],
        "body: {select_body}"
    );

    // And it's what `/api/state` reports too, not just the `/api/select`
    // response (the same double-check `get_api_state_reflects_the_active_session_after_select`
    // makes for the fixture's own first card).
    let state = http(&base, "GET", "/api/state", &[], &[]);
    let state_body: serde_json::Value = serde_json::from_slice(&state.body).unwrap();
    assert_eq!(
        "edited term?", state_body["card"]["front"],
        "body: {state_body}"
    );

    // And promotable: the happy promote path lives here because the 400 test
    // for a non-virtual card cannot prove the virtuality check's polarity
    // (an inverted check still rejects, for a different reason); only a
    // succeeding promote can, and this scenario already holds the minted
    // virtual card as the current card.
    let before = state_body["study_revision"].as_u64().unwrap();
    let resp = post_gated(&base, "/api/promote", "{}");
    assert_eq!(
        200,
        resp.status,
        "a virtual card must promote; body: {}",
        String::from_utf8_lossy(&resp.body)
    );
    let promoted: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let after = promoted["study_revision"].as_u64().unwrap();
    assert!(
        after > before,
        "promote must advance the revision ({before} -> {after})"
    );
}

#[test]
fn ask_card_draft_and_create_are_refused_for_a_kids_audience() {
    let (base, _guard) = spawn_kids_server();

    let draft_resp = post_gated(&base, "/api/ask/card/draft", "{}");
    assert_eq!(403, draft_resp.status);

    let create_resp = post_gated(
        &base,
        "/api/ask/card/create",
        r#"{"front":"f","back":["b"]}"#,
    );
    assert_eq!(403, create_resp.status);
}

// ── Workspace deadline ───────────────────────────────────────────────────

/// Set, then clear, a workspace's deadline through `POST
/// /api/workspace/deadline`, checking both the file `set_deadline` writes and
/// the refreshed `/api/decks` payload the endpoint hands back in the same
/// round trip. Pins the catalog's date-arithmetic and workspace-gating path
/// (`deck_catalog`'s `is_ws.then(...)`) under test for the first time: until
/// now only `set_deadline`'s own file-writing unit tests exercised the write
/// side, and no test read the `deadline` readout back off a real workspace.
#[test]
fn workspace_deadline_set_and_clear_round_trip_through_the_file_and_the_decks_readout() {
    let (base, guard) = spawn_test_server_fixture(None, write_workspace_fixture);

    // Set: 200, the file carries the key, and the refreshed decks payload
    // (returned inline, no second fetch needed) carries the readout.
    let resp = post_json(
        &base,
        "/api/workspace/deadline",
        r#"{"name":"ws","date":"2099-01-02"}"#,
    );
    assert_eq!(
        200,
        resp.status,
        "body: {}",
        String::from_utf8_lossy(&resp.body)
    );
    let local = std::fs::read_to_string(guard.dir().join("ws/alix.local.toml")).unwrap();
    assert!(
        local.contains("deadline = \"2099-01-02\""),
        "local: {local}"
    );
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let ws = body["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "ws")
        .unwrap_or_else(|| panic!("no `ws` workspace row in the response: {body}"));
    assert_eq!("2099-01-02", ws["deadline"]["date"], "row: {ws}");
    assert!(
        ws["deadline"]["days_left"].as_i64().unwrap() > 0,
        "row: {ws}"
    );
    assert!(ws["deadline"]["ready"].is_number(), "row: {ws}");
    assert!(ws["deadline"]["total"].is_number(), "row: {ws}");

    // A second fetch (not just the inline response) must agree: the readout
    // is really coming from the file, not a stale in-memory echo.
    let decks_resp = http(&base, "GET", "/api/decks", &[], &[]);
    let decks: serde_json::Value = serde_json::from_slice(&decks_resp.body).unwrap();
    let ws = decks["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "ws")
        .unwrap();
    assert_eq!("2099-01-02", ws["deadline"]["date"], "row: {ws}");

    // Malformed date: 400, file untouched.
    let bad = post_json(
        &base,
        "/api/workspace/deadline",
        r#"{"name":"ws","date":"tomorrow"}"#,
    );
    assert_eq!(400, bad.status);
    let local = std::fs::read_to_string(guard.dir().join("ws/alix.local.toml")).unwrap();
    assert!(
        local.contains("deadline = \"2099-01-02\""),
        "local: {local}"
    );

    // Clear: 200, key gone, readout gone.
    let resp = post_json(
        &base,
        "/api/workspace/deadline",
        r#"{"name":"ws","date":null}"#,
    );
    assert_eq!(200, resp.status);
    let local = std::fs::read_to_string(guard.dir().join("ws/alix.local.toml")).unwrap();
    assert!(!local.contains("deadline"), "local: {local}");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let ws = body["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "ws")
        .unwrap();
    assert!(ws["deadline"].is_null(), "row: {ws}");
}

/// A plain deck row and an unknown name each lack a directory to write a
/// deadline into: `resolve_row` only carries a `dir` on a container row
/// (`Resolved::Many`), so both fall into the same 400 as a malformed date.
#[test]
fn workspace_deadline_rejects_a_plain_deck_row_and_an_unknown_name() {
    let (base, _guard) = spawn_test_server_fixture(None, write_workspace_fixture);

    let resp = post_json(
        &base,
        "/api/workspace/deadline",
        r#"{"name":"sample.md","date":"2099-01-02"}"#,
    );
    assert_eq!(400, resp.status);

    let resp = post_json(
        &base,
        "/api/workspace/deadline",
        r#"{"name":"does-not-exist","date":"2099-01-02"}"#,
    );
    assert_eq!(400, resp.status);
}

/// The row-shape guard (`Resolved::Many { dir, .. } if is_workspace(&dir)`)
/// specifically rejects a `Resolved::Many` row that is NOT a workspace: a
/// plain folder of loose decks with no `alix.toml`. The previous rejection
/// test only covered `Resolved::One` and `Resolved::Unknown`, which hit the
/// same 400 fallback whether or not the `is_workspace` guard was even there;
/// this is the one that actually exercises it.
#[test]
fn workspace_deadline_rejects_a_plain_folder_that_is_not_a_workspace() {
    let (base, guard) = spawn_test_server_fixture(None, |dir| {
        write_workspace_fixture(dir);
        write_plain_folder_fixture(dir);
    });

    let resp = post_json(
        &base,
        "/api/workspace/deadline",
        r#"{"name":"plainfolder","date":"2099-01-02"}"#,
    );
    assert_eq!(400, resp.status);
    assert!(
        !guard.dir().join("plainfolder/alix.local.toml").is_file(),
        "a non-workspace folder must never get a deadline file written into it"
    );
}

/// A missing `date` key must be a 400 (a client bug), never treated the same
/// as an explicit `null` (the real clear signal). Sets a deadline first so a
/// wrongly-lenient parse (reading the missing key as `None`, same as
/// `null`) would be visible as a silent clear instead of just a bad status.
#[test]
fn workspace_deadline_with_a_missing_date_key_is_a_400_not_a_silent_clear() {
    let (base, guard) = spawn_test_server_fixture(None, write_workspace_fixture);

    let resp = post_json(
        &base,
        "/api/workspace/deadline",
        r#"{"name":"ws","date":"2099-01-02"}"#,
    );
    assert_eq!(200, resp.status);

    let resp = post_json(&base, "/api/workspace/deadline", r#"{"name":"ws"}"#);
    assert_eq!(400, resp.status);

    let local = std::fs::read_to_string(guard.dir().join("ws/alix.local.toml")).unwrap();
    assert!(
        local.contains("deadline = \"2099-01-02\""),
        "a missing `date` key must not clear the deadline: local: {local}"
    );
}

/// `workspace::set_deadline` bails when `[review]` exists in the local
/// manifest but is not a table (e.g. a hand-edited `review = 5`); the route
/// must surface that as 500, not silently swallow it into a 200 or a 400.
#[test]
fn workspace_deadline_returns_500_when_the_local_manifest_has_a_non_table_review_key() {
    let (base, guard) = spawn_test_server_fixture(None, write_workspace_fixture);
    std::fs::write(guard.dir().join("ws/alix.local.toml"), "review = 5\n").unwrap();

    let resp = post_json(
        &base,
        "/api/workspace/deadline",
        r#"{"name":"ws","date":"2099-01-02"}"#,
    );
    assert_eq!(500, resp.status);
}

// ── Remote (a paired phone's tutor + exam, /api/remote/*) ────────────────
//
// The desktop plays model backend for a paired phone's tutor and AI exam; the
// phone owns all state (transcript, mastery, cards) and resends it every
// call. THE IRON RULE this family exists to pin: nothing under
// `/api/remote/*` ever touches the server's own store
// (`remote_endpoints_never_write_the_server_store`).

/// A source-backed fact deck (`source:` at a local file) alongside
/// `spawn_full_server`'s other fixtures: enough for the AI exam's
/// generate → answer → grade → remediate walk. One card is enough; the exam
/// grades the source, never the deck's own cards.
fn write_exam_deck_fixture(dir: &Path) {
    std::fs::write(
        dir.join("examdeck.md"),
        "---\nformat-version: 1\nid: \"deck-examdeck\"\nsource: examsource.txt\n---\n## c <!-- id: card-e1 -->\na\n",
    )
    .unwrap();
    std::fs::write(dir.join("examsource.txt"), "c stands for a concept.\n").unwrap();
}

/// A fake CLI for the exam family: branches on the prompt's JSON-shape
/// marker, mirroring `exam.rs`'s own `branching_cli` unit-test helper (not
/// reachable from this integration test, so replicated). `"grades"` reads
/// its reply from `grades_path` (a test can rewrite that file between calls
/// to vary the verdict across sittings); `"questions"` always answers with
/// one fixed question; `compression` (the trace grader's own prompt, which
/// carries no `"grades"` marker, see `grade_compression_prompt`) ALSO reads
/// `grades_path`, so a test drives a trace grade the same way: rewrite the
/// file to a bare `{"verdict": ...}` (not wrapped in `{"grades": [...]}`,
/// [`exam::AnswerGrade`]'s own shape) before the call; anything else (a
/// remediation call, or a tutor ask/draft call) gets a one-card deck-format
/// reply, which is valid input for all three: a tutor question stores it
/// as-is, and both remediation and draft-parsing accept deck-format text.
fn branching_exam_cli(dir: &Path, grades_path: &Path) -> PathBuf {
    let path = dir.join("fake-claude");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ninput=$(cat)\ncase \"$input\" in\n\
             *'\"grades\"'*) cat {grades} ;;\n\
             *'\"questions\"'*) printf '%s' '{{\"questions\":[{{\"prompt\":\"Q1\",\"points\":[\"p1\"]}}]}}' ;;\n\
             *compression*) cat {grades} ;;\n\
             *) printf '## term?\\ndefinition\\n' ;;\n\
             esac\n",
            grades = grades_path.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Releases a fifo-gated fake CLI exactly once, on an explicit call or on
/// drop (including a panic unwind), so a failing assertion in a fifo-gated
/// test can never leave the fake CLI, and this file's global `EXEC_LOCK`,
/// wedged for the rest of the suite.
struct FifoRelease {
    path: PathBuf,
    released: bool,
}

impl FifoRelease {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            released: false,
        }
    }

    fn release(&mut self) {
        if !self.released {
            self.released = true;
            let _ = std::fs::write(&self.path, "go\n");
        }
    }
}

impl Drop for FifoRelease {
    fn drop(&mut self) {
        self.release();
    }
}

/// A round trip for a client-supplied card (no server session at all): ask a
/// question, poll to the settled answer, then a second turn (now with a
/// history entry) proves the slot is fully REPLACED rather than accumulating
/// or leaking the previous turn's answer while the new one is in flight.
#[test]
fn remote_ask_round_trips_an_answer_for_a_client_supplied_card() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(scripts.path(), "because it demonstrates addition");
    let (base, _guard) = spawn_full_server(Some(&fake));

    let resp = post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},
            "history":[],"question":"why does this matter?"}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(true, body["thinking"], "body: {body}");

    let body = poll_until(&base, "/api/remote/ask", |b| {
        !b["thinking"].as_bool().unwrap()
    });
    assert_eq!(
        "because it demonstrates addition", body["answer"],
        "body: {body}"
    );
    assert!(body["error"].is_null(), "body: {body}");

    // A second turn, with a history entry: the settled slot must be replaced
    // outright (thinking flips back to true in the POST's own response), not
    // left showing the first turn's answer until the new one lands.
    let resp = post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},
            "history":[{"q":"why does this matter?","a":"because it demonstrates addition"}],
            "question":"anything else?"}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(true, body["thinking"], "body: {body}");

    let body = poll_until(&base, "/api/remote/ask", |b| {
        !b["thinking"].as_bool().unwrap()
    });
    assert_eq!(
        "because it demonstrates addition", body["answer"],
        "body: {body}"
    );
    assert!(body["error"].is_null(), "body: {body}");
}

/// {#M6-ledger-row-1}: the backend call for a remote tutor turn runs on a
/// background thread, never inline in the request loop, so a second POST
/// while the first is still thinking answers 409 (never blocks waiting for
/// it), AND the loop stays live for every other endpoint the whole time. A
/// fifo-gated fake CLI parks the backend call open-endedly (past the stdin
/// drain, blocked reading a fifo nothing has written to yet) so the test can
/// make both assertions before ever letting the call finish.
#[test]
fn remote_ask_answers_409_while_a_turn_is_thinking_and_the_loop_stays_live() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fifo = scripts.path().join("gate");
    assert!(
        std::process::Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success(),
        "mkfifo {fifo:?} failed"
    );
    let reply = scripts.path().join("reply");
    std::fs::write(&reply, "eventually").unwrap();
    let fake = scripts.path().join("fake-claude");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\ncat {} >/dev/null\ncat {}\n",
            fifo.display(),
            reply.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, _guard) = spawn_full_server(Some(&fake));

    let resp = post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},
            "history":[],"question":"why?"}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(true, body["thinking"], "body: {body}");

    // From here on an assertion could panic; the fifo must still get released
    // so the parked child (and this test's `exec_lock`) never wedges the rest
    // of the suite.
    let mut release = FifoRelease::new(fifo.clone());

    let resp = post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},
            "history":[],"question":"again?"}"#,
    );
    assert_eq!(409, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);

    let started = Instant::now();
    let resp = http(&base, "GET", "/api/version", &[], &[]);
    let elapsed = started.elapsed();
    assert_eq!(200, resp.status);
    assert!(
        elapsed < Duration::from_millis(500),
        "the request loop must not block on a parked backend call: {elapsed:?}"
    );

    release.release();
    poll_until(&base, "/api/remote/ask", |b| {
        !b["thinking"].as_bool().unwrap()
    });
}

#[test]
fn remote_draft_is_refused_for_the_kids_audience() {
    let (base, _guard) = spawn_kids_server();

    let resp = post_json(&base, "/api/remote/ask/draft", "{}");

    assert_eq!(403, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

/// A note-condense round trip: the fake CLI's reply carries bullet prefixes
/// and a fourth line past the three-line cap, so a `note` array of exactly
/// three CLEAN lines (no bullets) proves `extract_note_lines` ran
/// server-side, not just that the raw reply was echoed back.
#[test]
fn remote_note_round_trips_condensed_lines_capped_and_cleaned_server_side() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(
        scripts.path(),
        "- first the key insight\n\
         * second worth rereading\n\
         third plain line\n\
         fourth line dropped by the cap",
    );
    let (base, _guard) = spawn_full_server(Some(&fake));

    let resp = post_json(
        &base,
        "/api/remote/ask/note",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},
            "history":[{"q":"why does this matter?","a":"because it demonstrates addition"},
                       {"q":"anything else?","a":"no, that covers it"}]}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(true, body["thinking"], "body: {body}");

    let body = poll_until(&base, "/api/remote/ask", |b| {
        !b["thinking"].as_bool().unwrap()
    });
    assert_eq!(
        serde_json::json!([
            "first the key insight",
            "second worth rereading",
            "third plain line"
        ]),
        body["note"],
        "body: {body}"
    );
    assert!(body["answer"].is_null(), "body: {body}");
    assert!(body["draft"].is_null(), "body: {body}");
    assert!(body["error"].is_null(), "body: {body}");
}

#[test]
fn remote_note_rejects_empty_history_and_garbage_body_with_400() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(
        &base,
        "/api/remote/ask/note",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},"history":[]}"#,
    );
    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);

    let resp = post_json(&base, "/api/remote/ask/note", "not json");
    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

/// `/api/remote/ask/note` shares the single `remote_ask` slot with
/// `/api/remote/ask` and `/api/remote/ask/draft`: a call into the slot while
/// a note is still thinking collides just like two overlapping questions do.
#[test]
fn remote_note_answers_409_while_a_call_is_thinking_in_the_shared_slot() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fifo = scripts.path().join("gate");
    assert!(
        std::process::Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success(),
        "mkfifo {fifo:?} failed"
    );
    let reply = scripts.path().join("reply");
    std::fs::write(&reply, "eventually").unwrap();
    let fake = scripts.path().join("fake-claude");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\ncat {} >/dev/null\ncat {}\n",
            fifo.display(),
            reply.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, _guard) = spawn_full_server(Some(&fake));

    let resp = post_json(
        &base,
        "/api/remote/ask/note",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},
            "history":[{"q":"why?","a":"because"}]}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(true, body["thinking"], "body: {body}");

    // From here on an assertion could panic; the fifo must still get released
    // so the parked child (and this test's `exec_lock`) never wedges the rest
    // of the suite.
    let mut release = FifoRelease::new(fifo.clone());

    let resp = post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"sample.md","front":"2 + 2","back":["4"],"at":null},
            "history":[],"question":"again?"}"#,
    );
    assert_eq!(409, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);

    release.release();
    poll_until(&base, "/api/remote/ask", |b| {
        !b["thinking"].as_bool().unwrap()
    });
}

/// Generates a deck from a URL and reads back the full deck text, mirroring
/// the web's `/api/generate` round trip but with no `dest` and no saved file:
/// `filename` is only a suggestion, `cards` is the finished text's own parsed
/// count.
#[test]
fn remote_generate_round_trips_deck_text_for_a_url() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(
        scripts.path(),
        "---\nlink: https://example.org\n---\n## Q\nA\n",
    );
    let (base, _guard) = spawn_full_server(Some(&fake));

    let resp = post_json(
        &base,
        "/api/remote/generate",
        r#"{"url":"https://example.org"}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("generating", body["phase"], "body: {body}");

    let body = poll_until(&base, "/api/remote/generate", |b| {
        b["phase"] != "generating"
    });
    assert_eq!("done", body["phase"], "body: {body}");
    let deck = body["deck"].as_str().expect("deck is a string");
    assert!(deck.contains("## Q"), "deck: {deck}");
    assert_eq!("example-org.md", body["filename"], "body: {body}");
    assert_eq!(1, body["cards"], "body: {body}");
    assert!(body["error"].is_null(), "body: {body}");
}

/// A second `POST` while a generation is thinking answers 409; once it
/// settles (confirmed via `GET`), a later `POST` is accepted again: the
/// same finished-but-unpolled-doesn't-409 idiom the tutor endpoints use.
#[test]
fn remote_generate_answers_409_while_thinking_then_a_later_post_after_settle_succeeds() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fifo = scripts.path().join("gate");
    assert!(
        std::process::Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success(),
        "mkfifo {fifo:?} failed"
    );
    let reply = scripts.path().join("reply");
    std::fs::write(&reply, "## Q\nA\n").unwrap();
    let fake = scripts.path().join("fake-claude");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\ncat {} >/dev/null\ncat {}\n",
            fifo.display(),
            reply.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, _guard) = spawn_full_server(Some(&fake));

    let resp = post_json(
        &base,
        "/api/remote/generate",
        r#"{"url":"https://example.org"}"#,
    );
    assert_eq!(200, resp.status);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("generating", body["phase"], "body: {body}");

    // From here on an assertion could panic; the fifo must still get released
    // so the parked child (and this test's `exec_lock`) never wedges the rest
    // of the suite.
    let mut release = FifoRelease::new(fifo.clone());

    let resp = post_json(
        &base,
        "/api/remote/generate",
        r#"{"url":"https://example.org"}"#,
    );
    assert_eq!(409, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);

    release.release();
    let body = poll_until(&base, "/api/remote/generate", |b| {
        b["phase"] != "generating"
    });
    assert_eq!("done", body["phase"], "body: {body}");

    let resp = post_json(
        &base,
        "/api/remote/generate",
        r#"{"url":"https://example.org"}"#,
    );
    assert_eq!(200, resp.status, "a settled job must not 409 the next POST");
}

#[test]
fn remote_generate_rejects_a_non_http_url_and_a_missing_url_with_400() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(
        &base,
        "/api/remote/generate",
        r#"{"url":"file:///etc/passwd"}"#,
    );
    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);

    let resp = post_json(&base, "/api/remote/generate", r#"{"guidance":"only"}"#);
    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

/// The full remote-exam walk over a fact deck: generate → answering (prompts
/// only) → grade → results (a fail, with gaps) → remediate → the deck-format
/// `cards` payload → close → idle.
#[test]
fn remote_exam_walks_generate_answer_grade_fail_remediate_to_cards_payload() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let grades_path = scripts.path().join("grades");
    std::fs::write(
        &grades_path,
        r#"{"grades":[{"verdict":"fail","feedback":"no","missed":["gap one"]}]}"#,
    )
    .unwrap();
    let fake = branching_exam_cli(scripts.path(), &grades_path);
    let (base, _guard) =
        spawn_full_server_fixture(Some(&fake), write_exam_deck_fixture, |_opts| {});

    let resp = post_json(&base, "/api/remote/exam/start", r#"{"deck":"examdeck.md"}"#);
    assert_eq!(200, resp.status);

    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "answering");
    let questions = body["questions"].as_array().expect("questions is an array");
    assert_eq!(1, questions.len(), "body: {body}");
    assert!(
        questions[0].as_str().is_some_and(|q| !q.is_empty()),
        "body: {body}"
    );

    let resp = post_json(&base, "/api/remote/exam/grade", r#"{"answers":["a1"]}"#);
    assert_eq!(200, resp.status);

    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "results");
    assert_eq!(false, body["passed"], "body: {body}");
    assert!(!body["gaps"].as_array().unwrap().is_empty(), "body: {body}");
    assert_eq!(true, body["can_remediate"], "body: {body}");

    let resp = post_json(&base, "/api/remote/exam/remediate", "{}");
    assert_eq!(200, resp.status);

    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "remediated");
    let cards = body["cards"]
        .as_str()
        .expect("cards is a deck-format string");
    assert!(cards.trim_start().starts_with('#'), "cards: {cards}");

    let resp = post_json(&base, "/api/remote/exam/close", "{}");
    assert_eq!(200, resp.status);

    let resp = http(&base, "GET", "/api/remote/exam", &[], &[]);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("idle", body["phase"], "body: {body}");
}

#[test]
fn remote_exam_grade_rejects_wrong_arity_with_400_and_wrong_phase_with_409() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let grades_path = scripts.path().join("grades");
    std::fs::write(
        &grades_path,
        r#"{"grades":[{"verdict":"pass","feedback":"ok","missed":[]}]}"#,
    )
    .unwrap();
    let fake = branching_exam_cli(scripts.path(), &grades_path);
    let (base, _guard) =
        spawn_full_server_fixture(Some(&fake), write_exam_deck_fixture, |_opts| {});

    post_json(&base, "/api/remote/exam/start", r#"{"deck":"examdeck.md"}"#);
    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "answering");
    assert_eq!(
        1,
        body["questions"].as_array().unwrap().len(),
        "body: {body}"
    );

    // Wrong arity while answering: 400, the sitting stays in `answering`.
    let resp = post_json(&base, "/api/remote/exam/grade", r#"{"answers":[]}"#);
    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
    let resp = post_json(&base, "/api/remote/exam/grade", r#"{"answers":["a","b"]}"#);
    assert_eq!(400, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);

    // Close (back to idle), then any grade against the empty slot: 409.
    let resp = post_json(&base, "/api/remote/exam/close", "{}");
    assert_eq!(200, resp.status);
    let resp = post_json(&base, "/api/remote/exam/grade", r#"{"answers":["a"]}"#);
    assert_eq!(409, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

/// A trace deck now starts a remote exam sitting: `start_trace` opens
/// straight in `answering` with the path's one fixed compression question,
/// mirroring the browser's own trace-exam start (mod.rs, near the cooldown
/// gate) minus that gate: a remote sitting checks no cooldown, the phone's
/// own. `can_remediate` is false from the moment it starts (a trace sitting
/// never remediates), not just after a fail.
#[test]
fn remote_exam_start_accepts_a_trace_deck_and_opens_answering() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/remote/exam/start", r#"{"deck":"trace.md"}"#);
    assert_eq!(200, resp.status);

    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!("answering", body["phase"], "body: {body}");
    assert_eq!(
        1,
        body["questions"].as_array().unwrap().len(),
        "body: {body}"
    );
    assert_eq!(true, body["is_trace"], "body: {body}");
    assert_eq!(false, body["can_remediate"], "body: {body}");
}

/// A non-trace deck with no `source:` still has nothing to examine, so it
/// still 409s at start (the refusal `deck.is_trace() || deck.sources.is_empty()`
/// narrowed to `!deck.is_trace() && deck.sources.is_empty()`, not dropped).
#[test]
fn remote_exam_start_still_refuses_a_source_less_non_trace_deck_with_409() {
    let (base, _guard) = spawn_full_server(None);

    let resp = post_json(&base, "/api/remote/exam/start", r#"{"deck":"sample.md"}"#);

    assert_eq!(409, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);
}

/// A trace exam that PASSES: one graded compression settles into `results`
/// with `passed: true`, `is_trace: true`, and `can_remediate` still false,
/// and, the iron rule's other half, the server's own store is untouched (a
/// passing trace exam masters nothing server-side; the phone applies that to
/// its own store).
#[test]
fn remote_exam_trace_grade_pass_settles_to_results_and_writes_no_store() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(
        scripts.path(),
        r#"{"verdict":"pass","feedback":"re-derives the chain","missed":[]}"#,
    );
    let (base, guard) = spawn_full_server(Some(&fake));
    let fixture_state = state_root(guard.dir());
    let before = snapshot_dir(&fixture_state);

    post_json(&base, "/api/remote/exam/start", r#"{"deck":"trace.md"}"#);
    let resp = post_json(
        &base,
        "/api/remote/exam/grade",
        r#"{"answers":["it reads the first line, then the second"]}"#,
    );
    assert_eq!(200, resp.status);

    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "results");
    assert_eq!(true, body["passed"], "body: {body}");
    assert_eq!(true, body["is_trace"], "body: {body}");
    assert_eq!(false, body["can_remediate"], "body: {body}");

    let after = snapshot_dir(&fixture_state);
    assert_eq!(
        before, after,
        "a passing remote trace exam must not write the server's store"
    );
}

/// A trace exam that FAILS: `results` with `passed: false`, and
/// `can_remediate` stays false (a trace is re-walked, not remediated), so
/// `/api/remote/exam/remediate` 409s with no extra guard needed. The re-sit
/// cooldown a failed trace normally starts is store-side and browser-only,
/// unwritten here, the byte-compare's other half.
#[test]
fn remote_exam_trace_grade_fail_refuses_remediation_and_writes_no_store() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = fake_reply(
        scripts.path(),
        r#"{"verdict":"fail","feedback":"missed the second hop","missed":["it reads the second line"]}"#,
    );
    let (base, guard) = spawn_full_server(Some(&fake));
    let fixture_state = state_root(guard.dir());
    let before = snapshot_dir(&fixture_state);

    post_json(&base, "/api/remote/exam/start", r#"{"deck":"trace.md"}"#);
    post_json(
        &base,
        "/api/remote/exam/grade",
        r#"{"answers":["it reads the first line"]}"#,
    );

    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "results");
    assert_eq!(false, body["passed"], "body: {body}");
    assert_eq!(true, body["is_trace"], "body: {body}");
    assert_eq!(false, body["can_remediate"], "body: {body}");

    let resp = post_json(&base, "/api/remote/exam/remediate", "{}");
    assert_eq!(409, resp.status);

    let after = snapshot_dir(&fixture_state);
    assert_eq!(
        before, after,
        "a failed remote trace exam must not write the server's store (no cooldown set remotely)"
    );
}

#[test]
fn remote_endpoints_require_the_token_like_the_rest_of_the_api() {
    let (base, _guard) = spawn_test_server_with(Some("secret"));

    let resp = http(
        &base,
        "POST",
        "/api/remote/ask",
        &[("Content-Type", "application/json")],
        b"not json",
    );
    assert_eq!(401, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);

    let resp = http(
        &base,
        "POST",
        "/api/remote/ask",
        &[
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer secret"),
        ],
        b"not json",
    );
    assert_ne!(401, resp.status, "body: {:?}", resp.body);

    let resp = http(
        &base,
        "POST",
        "/api/remote/ask/note",
        &[("Content-Type", "application/json")],
        b"not json",
    );
    assert_eq!(401, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);

    let resp = http(
        &base,
        "POST",
        "/api/remote/ask/note",
        &[
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer secret"),
        ],
        b"not json",
    );
    assert_ne!(401, resp.status, "body: {:?}", resp.body);

    let resp = http(
        &base,
        "POST",
        "/api/remote/generate",
        &[("Content-Type", "application/json")],
        b"not json",
    );
    assert_eq!(401, resp.status);
    assert!(resp.body.is_empty(), "body: {:?}", resp.body);

    let resp = http(
        &base,
        "POST",
        "/api/remote/generate",
        &[
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer secret"),
        ],
        b"not json",
    );
    assert_ne!(401, resp.status, "body: {:?}", resp.body);
}

/// Recursively reads every regular file under `dir` into a byte-keyed map
/// (path relative to `dir` → contents): a whole-tree byte snapshot for
/// asserting a remote endpoint placed no file anywhere in the decks dir, the
/// other half of THE IRON RULE alongside the store-file diff below.
fn snapshot_dir(dir: &Path) -> HashMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut HashMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = HashMap::new();
    walk(dir, dir, &mut out);
    out
}

/// THE IRON RULE, pinned: nothing under `/api/remote/*` ever writes the
/// server's own store OR places a file into its decks dir. A paired phone
/// owns its mastery/progress/cards (and, for generation, its own
/// destination), and the desktop only computes answers for it. Runs a full
/// PASSING remote exam, a full FAILING-then-remediated remote exam, a
/// PASSING remote trace exam, a FAILING remote trace exam, a tutor
/// ask+draft+note round trip, and a full remote deck generation against ONE
/// server, then diffs the store file's bytes AND a whole-tree snapshot of the
/// decks dir before and after: a passing exam must not mark the deck
/// mastered, a remediation must not create server-side virtual cards, a
/// failed trace must not start the store-side re-sit cooldown, and a
/// generation must not save the deck it returns: any of those would show up
/// here as changed bytes or a new file.
#[test]
fn remote_endpoints_never_write_the_server_store() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let grades_path = scripts.path().join("grades");
    std::fs::write(
        &grades_path,
        r#"{"grades":[{"verdict":"pass","feedback":"ok","missed":[]}]}"#,
    )
    .unwrap();
    let fake = branching_exam_cli(scripts.path(), &grades_path);
    let (base, guard) = spawn_full_server_fixture(Some(&fake), write_exam_deck_fixture, |_opts| {});
    let fixture_state = state_root(guard.dir());
    let before = snapshot_dir(&fixture_state);
    let decks_before = snapshot_dir(guard.dir());

    // (a) a full remote exam that PASSES.
    post_json(&base, "/api/remote/exam/start", r#"{"deck":"examdeck.md"}"#);
    poll_until(&base, "/api/remote/exam", |b| b["phase"] == "answering");
    post_json(&base, "/api/remote/exam/grade", r#"{"answers":["a1"]}"#);
    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "results");
    assert_eq!(true, body["passed"], "body: {body}");
    post_json(&base, "/api/remote/exam/close", "{}");

    // (b) a full remote exam that FAILS and remediates through to cards.
    std::fs::write(
        &grades_path,
        r#"{"grades":[{"verdict":"fail","feedback":"no","missed":["gap one"]}]}"#,
    )
    .unwrap();
    post_json(&base, "/api/remote/exam/start", r#"{"deck":"examdeck.md"}"#);
    poll_until(&base, "/api/remote/exam", |b| b["phase"] == "answering");
    post_json(&base, "/api/remote/exam/grade", r#"{"answers":["a1"]}"#);
    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "results");
    assert_eq!(false, body["passed"], "body: {body}");
    post_json(&base, "/api/remote/exam/remediate", "{}");
    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "remediated");
    assert!(
        body["cards"]
            .as_str()
            .is_some_and(|c| c.trim_start().starts_with('#')),
        "body: {body}"
    );
    post_json(&base, "/api/remote/exam/close", "{}");

    // (c) a remote trace exam that PASSES.
    std::fs::write(
        &grades_path,
        r#"{"verdict":"pass","feedback":"re-derives the chain","missed":[]}"#,
    )
    .unwrap();
    post_json(&base, "/api/remote/exam/start", r#"{"deck":"trace.md"}"#);
    poll_until(&base, "/api/remote/exam", |b| b["phase"] == "answering");
    post_json(
        &base,
        "/api/remote/exam/grade",
        r#"{"answers":["it reads the first line, then the second"]}"#,
    );
    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "results");
    assert_eq!(true, body["passed"], "body: {body}");
    assert_eq!(true, body["is_trace"], "body: {body}");
    post_json(&base, "/api/remote/exam/close", "{}");

    // (d) a remote trace exam that FAILS (no remediation: a trace re-walks).
    std::fs::write(
        &grades_path,
        r#"{"verdict":"fail","feedback":"missed the second hop","missed":["it reads the second line"]}"#,
    )
    .unwrap();
    post_json(&base, "/api/remote/exam/start", r#"{"deck":"trace.md"}"#);
    poll_until(&base, "/api/remote/exam", |b| b["phase"] == "answering");
    post_json(
        &base,
        "/api/remote/exam/grade",
        r#"{"answers":["it reads the first line"]}"#,
    );
    let body = poll_until(&base, "/api/remote/exam", |b| b["phase"] == "results");
    assert_eq!(false, body["passed"], "body: {body}");
    assert_eq!(false, body["can_remediate"], "body: {body}");
    post_json(&base, "/api/remote/exam/close", "{}");

    // (e) a tutor ask + draft round trip.
    post_json(
        &base,
        "/api/remote/ask",
        r#"{"card":{"subject":"examdeck.md","front":"c","back":["a"],"at":null},
            "history":[],"question":"why?"}"#,
    );
    let body = poll_until(&base, "/api/remote/ask", |b| {
        !b["thinking"].as_bool().unwrap()
    });
    assert!(body["error"].is_null(), "body: {body}");
    post_json(
        &base,
        "/api/remote/ask/draft",
        r#"{"card":{"subject":"examdeck.md","front":"c","back":["a"],"at":null},
            "history":[{"q":"why?","a":"because"}]}"#,
    );
    let body = poll_until(&base, "/api/remote/ask", |b| {
        !b["thinking"].as_bool().unwrap()
    });
    assert!(body["draft"].is_object(), "body: {body}");
    post_json(
        &base,
        "/api/remote/ask/note",
        r#"{"card":{"subject":"examdeck.md","front":"c","back":["a"],"at":null},
            "history":[{"q":"why?","a":"because"}]}"#,
    );
    let body = poll_until(&base, "/api/remote/ask", |b| {
        !b["thinking"].as_bool().unwrap()
    });
    assert!(body["note"].is_array(), "body: {body}");

    // (f) a full remote deck generation.
    post_json(
        &base,
        "/api/remote/generate",
        r#"{"url":"https://example.org"}"#,
    );
    let body = poll_until(&base, "/api/remote/generate", |b| {
        b["phase"] != "generating"
    });
    assert_eq!("done", body["phase"], "body: {body}");
    post_json(&base, "/api/remote/generate/close", "{}");

    let after = snapshot_dir(&fixture_state);
    assert_eq!(
        before, after,
        "no /api/remote/* call may write the server's own store"
    );
    let decks_after = snapshot_dir(guard.dir());
    assert_eq!(
        decks_before, decks_after,
        "no /api/remote/* call may place a file into the server's decks dir"
    );
}

/// Sends one keep-alive HTTP/1.1 GET (no `Connection: close`, so the socket
/// stays open after the reply, exactly like a browser's parallel sockets) and
/// reads the status line, headers, and `Content-Length` body back. Returns the
/// status code, or an error string if the bounded read times out.
fn keep_alive_get(base: &str, path: &str, timeout: Duration) -> Result<u16, String> {
    let host = base.strip_prefix("http://").ok_or("base is http://")?;
    let mut stream = TcpStream::connect(host).map_err(|e| format!("connect: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| format!("set_read_timeout: {e}"))?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("write: {e}"))?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..head_end]);
            let status = head
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|c| c.parse::<u16>().ok())
                .ok_or("no status line")?;
            let len: usize = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().ok())?
                })
                .unwrap_or(0);
            if buf.len() >= head_end + 4 + len {
                return Ok(status);
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err("eof before full response".to_string()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(format!("read: {e}")),
        }
    }
}

/// Regression for the keep-alive starvation bug: several browser-style parallel
/// keep-alive sockets must not wedge the server. Pre-worker-pool (a single
/// `recv()` consumer) this batch stalls far past the bound; the worker pool
/// serves every one promptly. The 10s bound is generous so CI never flakes.
#[test]
fn parallel_keep_alive_requests_all_complete_promptly() {
    let (base, _guard) = spawn_test_server();
    let endpoints = [
        "/api/version",
        "/api/keys",
        "/api/decks",
        "/api/state",
        "/api/pair",
        "/api/browse-keys",
        "/api/picker-keys",
        "/api/ask-info",
    ];
    // All sockets fire at the same instant, so the connections really are
    // in flight together rather than one-after-another.
    let barrier = Arc::new(std::sync::Barrier::new(endpoints.len()));
    let start = Instant::now();
    let handles: Vec<_> = endpoints
        .iter()
        .map(|&ep| {
            let base = base.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                keep_alive_get(&base, ep, Duration::from_secs(10))
            })
        })
        .collect();
    for (ep, h) in endpoints.iter().zip(handles) {
        let res = h.join().expect("keep-alive request thread panicked");
        assert!(
            matches!(res, Ok(200)),
            "endpoint {ep} did not return 200 promptly: {res:?}"
        );
    }
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "parallel keep-alive requests wedged: took {:?}",
        start.elapsed()
    );
}

/// The web "Add deck…" flow must stop a provider that produces nothing, at the
/// `[generate] idle_timeout_secs` inactivity limit rather than only at the
/// (one-hour by default) absolute limit.
#[test]
fn web_generate_stops_a_wedged_provider_at_the_inactivity_limit() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = scripts.path().join("wedged-claude");
    std::fs::write(
        &fake,
        "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\nexec sleep 600\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (base, _guard) = spawn_full_server_fixture(
        Some(&fake),
        |_dir| {},
        |opts| {
            opts.generate.timeout_secs = 3;
            opts.generate.idle_timeout_secs = 1;
        },
    );

    let resp = post_json(
        &base,
        "/api/generate",
        r#"{"url":"https://example.org/article"}"#,
    );
    assert_eq!(200, resp.status);

    let body = poll_until(&base, "/api/generate", |b| b["phase"] != "generating");
    let error = body["error"].as_str().unwrap_or_default().to_string();
    assert!(
        error.contains("made no progress"),
        "the serve generate path must honour the inactivity limit: {error}"
    );
}

/// Saving a tutor note writes the deck file underneath a live session. The
/// card the learner was studying must still be the card they return to: the
/// web client keeps its client-side reveal position across the tutor close
/// (`closeAsk` assigns state without `apply`), which is only sound while the
/// card cannot change with the panel open.
#[test]
fn a_tutor_note_leaves_the_learner_on_the_same_card() {
    let _lock = exec_lock();
    let scripts = TempDir::new().unwrap();
    let fake = scripts.path().join("fake-tutor");
    std::fs::write(
        &fake,
        "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\nprintf '%s\\n' 'a tutor answer'\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (base, guard) = spawn_full_server(Some(&fake));
    post_json(&base, "/api/select", r#"{"deck":"choice-armed.md"}"#);

    // The reported sequence answers the card first, so the client sits in its
    // feedback state with the tutor opened over it.
    let resp = post_choice(&base, 0);
    assert_eq!(200, resp.status, "answering the multiple-choice card");
    let resp = http(&base, "GET", "/api/state", &[], &[]);
    let graded: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    let studied = graded["card"]["front"].as_str().unwrap().to_string();

    let resp = post_gated(&base, "/api/ask", r#"{"question":"why?"}"#);
    assert_eq!(200, resp.status, "opening the tutor");
    poll_until(&base, "/api/ask", |b| b["thinking"] == false);

    let resp = post_gated(&base, "/api/ask/note", "{}");
    assert_eq!(200, resp.status, "saving the note");
    poll_until(&base, "/api/ask", |b| b["thinking"] == false);

    let resp = http(&base, "GET", "/api/state", &[], &[]);
    let after: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        studied,
        after["card"]["front"].as_str().unwrap_or_default(),
        "the learner returned to a different card after saving a tutor note; \
         deck on disk:\n{}",
        std::fs::read_to_string(guard.dir().join("sample.md")).unwrap_or_default()
    );
}
