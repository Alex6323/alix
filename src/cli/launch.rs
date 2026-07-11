//! The `alix` launcher: binds the web server, announces the pairing URL and
//! QR, resolves per-instance scoped state, and builds the review/browse
//! sessions the server calls back into for each pick from the picker.

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use alix::{
    assemble::{self, expand_workspaces, load_decks, open_store, store_path_for, subject_paths},
    augment::{self, AugmentCache},
    card::Card,
    config::Config,
    recent::{self, RecentDecks},
    serve,
    session::DeckInfo,
    time::now_ms,
    workspace,
};
use anyhow::{Context, Result, bail};

use crate::{LaunchArgs, common::store_for};

/// Builds the browse card list from explicit `deck_paths` (no picker). Mirrors
/// [`assemble::select`]'s review path for the read-only browse view: loads
/// decks, but builds no scheduler session.
fn build_browse(deck_paths: Vec<PathBuf>, recent: &mut RecentDecks) -> Result<BrowseBuild> {
    // One deck file per browse — no merging loose decks or whole workspaces.
    let [deck] = deck_paths.as_slice() else {
        bail!("browse one deck at a time (merging decks was removed)");
    };
    if workspace::has_decks(deck) {
        bail!(
            "`{}` is a workspace — browse a deck inside it, or open it with `alix workspace`",
            deck.display()
        );
    }
    let expanded = expand_workspaces(&deck_paths)?;
    let (mut cards, deck_label, decks, _) = load_decks(&expanded.decks, &expanded.defaults)?;
    let label = deck_label;

    // Merge in the display augmentations review shows, from the decks' own store
    // (a workspace's when they share one) — so browse renders the same view, not
    // the raw deck. The raw card stays in the deck file; this is display-only.
    let store = store_for(&expanded.decks, None)?;
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for card in &mut cards {
        // Reshape first (re-renders the deck note, front, answer, mode) …
        augment.apply_format(card);
        // … then stack the notes-target trivia on top of the reshaped note.
        if let Some(note) = augment.note(card.id()) {
            card.append_note(&[note.to_string()]);
        }
    }

    recent.record(&deck_paths, now_ms());
    let _ = recent.save();
    Ok(BrowseBuild {
        cards,
        label,
        decks,
    })
}

/// Browse cards built from an explicit set of deck paths.
struct BrowseBuild {
    cards: Vec<Card>,
    label: String,
    decks: HashMap<String, DeckInfo>,
}

/// The IP/port to serve on: localhost unless `--lan`, and the `--port` flag or
/// the configured `[serve]` port.
fn serve_addr(port: Option<u16>, lan: bool, config: &Config) -> SocketAddr {
    let ip = if lan {
        Ipv4Addr::UNSPECIFIED // 0.0.0.0 — reachable from the local network
    } else {
        Ipv4Addr::LOCALHOST // 127.0.0.1 — this machine only
    };
    SocketAddr::from((ip, port.unwrap_or(config.serve.port)))
}

/// Resolves the serve pairing token: an explicit `--token` or `[serve] token`
/// wins; otherwise `--lan` generates one. Localhost (no `--lan`) stays open
/// (`None`). Fails **closed** — if `--lan` needs a token but generation fails,
/// this errors rather than leaving the network API open.
fn resolve_serve_token(cli: Option<String>, lan: bool, config: &Config) -> Result<Option<String>> {
    if let Some(t) = cli
        .or_else(|| config.serve.token.clone())
        .filter(|t| !t.is_empty())
    {
        return Ok(Some(t));
    }
    if lan {
        return Ok(Some(generate_token()?));
    }
    Ok(None)
}

/// A cryptographically secure random pairing token (16 bytes, hex), drawn from
/// the OS CSPRNG via `getrandom` (portable across Linux/macOS/Windows).
fn generate_token() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)
        .map_err(|e| anyhow::anyhow!("could not generate a serve pairing token: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Serves the web app: everything is picked in the browser (direct deck launch
/// was removed — the picker is the one way into a review). A `dir` argument
/// scopes this instance to that folder as a **self-contained root**: its own
/// catalog, its own `progress.json` and `recent.json` inside it — so several
/// instances (say, one per family member, each `--lan` on its own `--port`)
/// run side by side without sharing any state.
pub(crate) fn launch(args: LaunchArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    // A scoped instance (`alix <dir>`) is pinned to its folder forever; the
    // config-derived instance (bare `alix`) re-resolves `decks_dir` on every
    // `/api/decks` fetch, so an edited config takes effect without a restart.
    let scoped = args.dir.is_some();
    // The served root and this instance's state files. No dir → the configured
    // decks directory with the global store/recent (the classic single-user
    // setup). A dir → that folder, state kept inside it: a plain folder gets
    // `progress.json`/`recent.json` at its top; a workspace root already keeps
    // its store inside by convention (manifest `store =` respected).
    let (decks_dir, instance_store, recent_path) = match &args.dir {
        None => (
            config.decks_dir().context("cannot determine ~/decks")?,
            None,
            recent::default_recent_path().context("cannot determine the data directory")?,
        ),
        Some(path) if path.is_file() => bail!(
            "`alix <deck>` was removed — run `alix` and pick the deck there, \
             or serve its folder: `alix {}`",
            path.parent().unwrap_or_else(|| Path::new(".")).display()
        ),
        Some(path) if !path.is_dir() => bail!("`{}` is not a folder", path.display()),
        Some(path) => {
            let store = if workspace::is_workspace(path) {
                workspace::store_path(path)
            } else {
                path.join(workspace::STORE_FILE)
            };
            (path.clone(), Some(store), path.join("recent.json"))
        }
    };
    let recent = RecentDecks::load(recent_path);
    // Sessions write to the decks' own store — a workspace's `progress.json`
    // when the picked deck lives in one, else this instance's store (the
    // global default, or the scoped root's own file).
    let store = open_store(instance_store.clone())?;
    let addr = serve_addr(args.port, args.lan, &config);
    // Bind before announcing, so a taken port errors instead of printing a
    // success-looking URL first (likely with several instances running).
    // Shared so `run_review` can be stopped from outside its own thread (see
    // its doc); the CLI never stops it — the process exits on Ctrl-C instead.
    let server = Arc::new(serve::bind(addr)?);
    // The instance-wide session pacing: flag > `[review]` config > default.
    let pacing = assemble::Pacing {
        max_new: args.new.or(config.review.max_new).unwrap_or(10),
        limit: args.limit.or(config.review.limit),
    };

    // The read-only browse card builder for the picker's "Browse" action.
    let to_cards = |b: BrowseBuild| serve::CardsBuild {
        cards: b.cards,
        label: b.label,
        decks: subject_paths(b.decks),
    };
    let token = resolve_serve_token(args.token.clone(), args.lan, &config)?;
    let pair = announce(addr, args.lan, token.as_deref(), &decks_dir);

    let opts = serve::ReviewOptions {
        keys: config.keys.clone(),
        picker: config.picker.clone(),
        browse: config.browse.clone(),
        review: config.review,
        ask: config.ask.clone(),
        exam: config.exam.clone(),
        ai: config.ai.clone(),
        generate: config.generate.clone(),
        audience: config.serve.audience,
        auth: token,
        config_path: args.config.clone(),
        pair,
        scoped,
        cfg: assemble::Cfg {
            review: config.review,
            ask: config.ask.clone(),
            trace_auto_grade: config.trace.auto_grade,
            pacing,
            instance_store: instance_store.clone(),
        },
    };
    // Picks the right store for whatever decks a selection resolves to: a
    // workspace member's own store, else this instance's store (`&[]` → the
    // instance store too).
    let store_for_sel = |paths: &[PathBuf]| {
        open_store(store_path_for(paths, None).or_else(|| instance_store.clone()))
    };
    let build_browse_sel =
        |paths: Vec<PathBuf>, recent: &mut RecentDecks| build_browse(paths, recent).map(to_cards);
    serve::run_review(
        store,
        recent,
        decks_dir,
        server,
        opts,
        build_browse_sel,
        store_for_sel,
    )
}

/// Prints what is served (the decks root) and where it is reachable, plus
/// pairing info (host/port/token, and a scannable QR of the pairing URL) when
/// it is exposed to the network with a token — or a warning when exposed
/// without one. Naming the root is what tells side-by-side instances apart.
/// Returns the same pairing info for `/api/pair` — this is the only place
/// bind + token + LAN IP come together.
fn announce(addr: SocketAddr, lan: bool, token: Option<&str>, root: &Path) -> serve::PairInfo {
    let root = abbreviate_home(root);
    let port = addr.port();
    let lan_ip = if lan { local_lan_ip() } else { None };
    let pair = match (token, lan_ip) {
        (Some(t), Some(ip)) => serve::PairInfo {
            url: format!("http://{ip}:{port}/?token={t}"),
            lan: true,
        },
        _ => serve::PairInfo {
            url: format!("http://127.0.0.1:{port}/"),
            lan: false,
        },
    };
    match (lan, token) {
        (true, Some(t)) => match lan_ip {
            Some(ip) => {
                println!("Serving {root} at http://{ip}:{port}");
                println!("On another device, open in a browser (or scan):");
                println!("  {}", pair.url);
                print_qr(&pair.url);
                println!("Or pair the app with:  host {ip}  port {port}  token {t}");
            }
            None => {
                println!("Serving {root} on all interfaces, port {port}.");
                println!("On another device, open in a browser:");
                println!("  http://<this-machine's-IP>:{port}/?token={t}");
                println!("Or pair the app with:  host <this-machine's-IP>  port {port}  token {t}");
            }
        },
        (true, None) => {
            println!("Serving {root} on all interfaces, port {port}.");
            println!("warning: no authentication — anyone on your network can reach this.");
        }
        (false, _) => {
            println!("Serving {root} at http://127.0.0.1:{port} — open it in your browser.")
        }
    }
    println!("Press Ctrl-C to stop.");
    pair
}

/// `path` with the home directory abbreviated to `~`, for the announce line.
fn abbreviate_home(path: &Path) -> String {
    if let Some(dirs) = directories::BaseDirs::new()
        && let Ok(rest) = path.strip_prefix(dirs.home_dir())
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// This machine's LAN-facing IP, found by "connecting" a UDP socket outward —
/// a routing-table lookup only; no packet is ever sent. `None` when it can't
/// be determined (no route), in which case the announce falls back to the
/// `<this-machine's-IP>` placeholder.
// Called only from `announce`; additionally depends on a real OS routing
// table via a live UDP socket, which is not deterministic across CI network
// sandboxes even with a server harness.
#[cfg_attr(coverage_nightly, coverage(off))]
fn local_lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Renders `text` as a terminal QR so a phone pairs by scanning; silently
/// skipped when the text is too long (the printed URL above still works).
// Print-only, two-line delegation to `qr::terminal_blocks` — nothing to
// assert without a stdout-capture idiom, which this codebase doesn't have.
#[cfg_attr(coverage_nightly, coverage(off))]
fn print_qr(text: &str) {
    if let Some(q) = alix::qr::terminal_blocks(text) {
        print!("{q}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_token_is_generated_only_when_exposed() {
        let cfg = Config::default();
        // localhost, nothing configured → open (no token)
        assert_eq!(resolve_serve_token(None, false, &cfg).unwrap(), None);
        // an explicit --token always wins
        assert_eq!(
            resolve_serve_token(Some("abc".into()), true, &cfg).unwrap(),
            Some("abc".into())
        );
        // --lan with nothing configured → a token is generated (fails closed, so
        // on this platform it must succeed)
        assert!(
            resolve_serve_token(None, true, &cfg)
                .unwrap()
                .is_some_and(|t| !t.is_empty())
        );
    }

    #[test]
    fn announce_local_only_returns_a_loopback_pair_regardless_of_token() {
        // `lan=false` never touches `local_lan_ip`/`print_qr` — it always
        // resolves the loopback pair, whether or not a token is configured.
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 4321));
        let root = Path::new("/tmp/does-not-need-to-exist");

        let no_token = announce(addr, false, None, root);
        assert_eq!(no_token.url, "http://127.0.0.1:4321/");
        assert!(!no_token.lan);

        let with_token = announce(addr, false, Some("abc"), root);
        assert_eq!(with_token.url, "http://127.0.0.1:4321/");
        assert!(!with_token.lan);
    }

    #[test]
    fn abbreviate_home_prefixes_a_path_under_home_with_tilde() {
        let Some(dirs) = directories::BaseDirs::new() else {
            // No resolvable home dir in this environment — nothing to verify.
            return;
        };
        let path = dirs.home_dir().join("decks").join("rust.txt");
        let expected = format!("~/{}", Path::new("decks").join("rust.txt").display());
        assert_eq!(abbreviate_home(&path), expected);
    }

    #[test]
    fn abbreviate_home_leaves_a_path_outside_home_unchanged() {
        let outside = PathBuf::from("/definitely-not-the-home-dir-xyz/decks/rust.txt");
        if let Some(dirs) = directories::BaseDirs::new()
            && outside.starts_with(dirs.home_dir())
        {
            // Pathological environment (home dir at/above this path) — skip
            // rather than assert something that isn't actually true here.
            return;
        }
        assert_eq!(abbreviate_home(&outside), outside.display().to_string());
    }

    #[test]
    fn build_browse_loads_from_explicit_paths_including_image_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        // A normal card and an image card — both render in the web frontend.
        std::fs::write(
            &path,
            "% img-dir: /imgs\n# plain\n\tanswer\n# pic\n% img: a.png\n\tphoto\n",
        )
        .unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));

        let build = build_browse(vec![path], &mut recent).unwrap();
        assert_eq!(2, build.cards.len());
    }

    #[test]
    fn build_browse_applies_a_cached_format_reshape() {
        // A deck in a workspace, so `build_browse` resolves the workspace's own
        // store (a deterministic temp path) rather than the global store.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("alix.toml"), "title = \"Eng\"\n").unwrap();
        let path = ws.join("d.txt");
        std::fs::write(&path, "# List the parts\n\tA, B, C\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));

        // Without a cached format, browse shows the raw deck answer.
        let raw = build_browse(vec![path.clone()], &mut recent).unwrap();
        let id = raw.cards[0].id();
        assert_eq!(raw.cards[0].back_for_display(), ["A, B, C"]);

        // Cache a format reshape (and a notes-target trivia) for that card in the
        // workspace's augment sidecar.
        let store = store_for(std::slice::from_ref(&path), None).unwrap();
        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.set_format(
            id,
            augment::Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: None,
            },
        );
        cache.set_note(id, "the parts are well known".to_string());
        cache.save().unwrap();

        // Browsing now shows the reshaped front/answer and the trivia note.
        let merged = build_browse(vec![path], &mut recent).unwrap();
        assert_eq!(merged.cards[0].front, "Name the parts");
        assert_eq!(merged.cards[0].back_for_display(), ["A", "B", "C"]);
        let note = merged.cards[0].note.clone().unwrap_or_default();
        assert!(note.contains("the parts are well known"), "{note}");
    }

    #[test]
    fn build_browse_rejects_multiple_decks() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "# q\n\ta\n").unwrap();
        std::fs::write(&b, "# q\n\tb\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let err = build_browse(vec![a, b], &mut recent).err().unwrap();
        assert!(format!("{err}").contains("one deck"), "{err}");
    }

    #[test]
    fn build_browse_rejects_a_workspace_directory() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("m.txt"), "# q\n\ta\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let err = build_browse(vec![ws], &mut recent).err().unwrap();
        assert!(format!("{err}").contains("workspace"), "{err}");
    }
}
