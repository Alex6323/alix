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
    augment::{self, AugmentCache, Topology, TopologyOrder},
    card::Card,
    config::Config,
    deck::{Deck, DeckSettings},
    parser,
    recent::{self, RecentDecks},
    scheduler::Fsrs,
    serve,
    session::{DeckInfo, Order, Session, SessionOptions},
    store::{Store, VirtualCard},
    time::now_ms,
    trace::{Trace, Walk},
    workspace,
};
use anyhow::{Context, Result, bail};

use crate::{
    LaunchArgs,
    common::{load_decks, open_store, store_for, store_path_for},
};

/// The per-session pacing an instance applies to every session it builds:
/// CLI flag > `[review]` config key > built-in default.
#[derive(Clone, Copy)]
struct Pacing {
    max_new: usize,
    limit: Option<usize>,
}

/// The result of [`expand_workspaces`]: the deck file(s) to load and the per-deck
/// workspace directive defaults (keyed by file name).
struct Expanded {
    decks: Vec<PathBuf>,
    defaults: HashMap<String, DeckSettings>,
}

/// Resolves each deck file's workspace context: a member file whose parent folder
/// is a workspace inherits that workspace's shared directive defaults (keyed by
/// file name); plain files pass through untagged. A review/browse target is a
/// single deck *file* (whole-workspace review was removed), so this no longer
/// expands a folder — it just tags the file with its workspace's directives.
fn expand_workspaces(deck_paths: &[PathBuf]) -> Result<Expanded> {
    let mut decks = Vec::new();
    let mut defaults: HashMap<String, DeckSettings> = HashMap::new();
    for path in deck_paths {
        // A deck file inside a workspace folder inherits its shared directives.
        if let Some(parent) = path.parent()
            && parent.join(workspace::MANIFEST).is_file()
            && let Ok(ws) = workspace::Workspace::load(parent)
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            defaults.insert(name.to_string(), ws.settings);
        }
        decks.push(path.clone());
    }
    Ok(Expanded { decks, defaults })
}

/// Resolves a per-run setting from three sources, most specific first: an
/// explicit CLI flag, then a value declared by the loaded decks (used when
/// they agree), then the built-in default. Decks that disagree fall back to
/// the default with a warning.
fn resolve<T: Copy + PartialEq>(
    name: &str,
    cli: Option<T>,
    declared: impl Iterator<Item = Option<T>>,
    default: T,
) -> T {
    if let Some(value) = cli {
        return value;
    }
    let mut distinct: Vec<T> = Vec::new();
    for value in declared.flatten() {
        if !distinct.contains(&value) {
            distinct.push(value);
        }
    }
    match distinct.as_slice() {
        [] => default,
        [only] => *only,
        _ => {
            eprintln!("warning: decks disagree on `{name}`; using the default");
            default
        }
    }
}

/// A review session built from an explicit set of deck paths, ready for the web
/// frontend. Produced by [`build_review`] and consumed by [`review_serve`].
struct ReviewBuild {
    session: Session,
    label: String,
    decks: HashMap<String, DeckInfo>,
    /// The resolved topology's name, if the session is topology-ordered, so a
    /// frontend can fetch it from the augment cache to show the connective cue.
    topology_name: Option<String>,
}

/// Base line number for a synthesized virtual card ([`synthesize_virtual`]) —
/// far past any real deck's line count, so a virtual card's `line` never
/// collides with (and so never shares a sibling group with) a real card's
/// front line.
pub(crate) const VIRTUAL_LINE_BASE: usize = 1_000_000;

/// Synthesizes a virtual card's stored deck-format `text` into the real `Card`
/// it stands for — the one in `parse(vc.parent, vc.text)` whose `Card::id`
/// matches `vc.id` (a cloze block yields several sub-cards; the id picks the
/// right hole). `subject` MUST equal `vc.parent`, or the id won't reproduce
/// (`Card::id` hashes the subject). `line` places it far past any real deck
/// line so it never shares a sibling group with a deck card — id-neutral, since
/// `Card::id` ignores `line`. Returns `None` if the text can't be parsed or no
/// card matches (defensive — impossible in practice, but no `unwrap` here).
pub(crate) fn synthesize_virtual(
    vc: &VirtualCard,
    subject: &Arc<str>,
    line: usize,
) -> Option<Card> {
    let mut card = parser::parse_str(subject, &vc.text)
        .ok()?
        .into_iter()
        .find(|c| c.id() == vc.id)?;
    card.line = line;
    Some(card)
}

/// Builds a review session from explicit `deck_paths` (no interactive picker):
/// resolves `% requires:` prerequisites, applies deck directives, builds the
/// `Session`, and records the decks as recent. The store is borrowed (the
/// caller owns it), so the web server can reuse one store across repeated
/// selections.
fn build_review(
    deck_paths: Vec<PathBuf>,
    pacing: Pacing,
    config: &Config,
    store: &Store,
    recent: &mut RecentDecks,
    // The picker's per-launch choices: depth, focus-drawer topology/region,
    // the cram tick-box, and optional pacing overrides.
    opts: &serve::SelectOptions,
) -> Result<ReviewBuild> {
    let topology_sel = opts.topology.as_deref();
    let region_sel = opts.region.as_deref();
    let depth_sel = opts.depth;
    // A session is exactly one deck file's cards — no merging of several loose
    // decks, and no reviewing a whole workspace at once. Workspaces are an
    // organizing layer: review their members one at a time (the picker drills in;
    // `alix workspace <dir>` opens that picker).
    let [deck] = deck_paths.as_slice() else {
        bail!("review one deck at a time (merging decks was removed)");
    };
    if workspace::has_decks(deck) {
        bail!(
            "`{}` is a folder — serve it (`alix {}`) and pick a deck inside it",
            deck.display(),
            deck.display()
        );
    }
    // Resolve the deck's workspace context (a member file inherits its workspace's
    // shared directives). `% requires:` prerequisites are NOT pulled in — the
    // dependency graph gates exams, not what a review session contains.
    let expanded = expand_workspaces(&deck_paths)?;
    let (mut cards, deck_label, mut decks, settings) =
        load_decks(&expanded.decks, &expanded.defaults)?;
    // Resolve each deck's effective ask-tutor source access: a deck in a
    // workspace takes that workspace's `source_access` override if it sets one,
    // else the global `[ask] source_access`.
    for info in decks.values_mut() {
        let workspace_override = info
            .path
            .parent()
            .filter(|p| workspace::is_workspace(p))
            .and_then(workspace::manifest_source_access);
        info.source_access = workspace_override.unwrap_or(config.ask.source_access);
    }
    // One deck per session, so the label is the deck's own subject.
    let label = deck_label;

    // Every card id in this deck — used to pick out *this* deck's topologies from
    // a cache that may be shared with other decks (one store).
    let deck_ids: std::collections::HashSet<u64> = cards.iter().map(|c| c.id()).collect();

    // Merge in any AI-generated notes from the sidecar cache (`alix deck augment
    // --target notes`) — shown with the card's own deck note on reveal. (Question
    // variants are rotated in per-presentation by the frontends, and distractors
    // are read when a choice question is built.)
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for card in &mut cards {
        // Reshape first (re-renders the deck note, front, answer, mode) …
        augment.apply_format(card);
        // … then stack the notes-target trivia on top of the reshaped note.
        if let Some(note) = augment.note(card.id()) {
            card.append_note(&[note.to_string()]);
        }
    }

    // Resolve the topology that reorders this session (if any) and project it to
    // a session-ready order. The resolved name travels on `ReviewBuild` so the
    // web frontend can show the "why this card follows the last" cue from the
    // same topology.
    let topology = resolve_topology(topology_sel, &augment, &deck_ids)?;
    let topology_name = topology.map(|t| t.name.clone());
    let topology_order = topology.map(|t| TopologyOrder::from_walk(&t.walk));

    // `--region` focuses the session on one region of the topology — drill a
    // weak area. SRS still picks what's due *within* that region.
    if let Some(region_name) = region_sel {
        let Some(topology) = topology else {
            bail!("--region needs a topology — pass --topology, or augment one for this deck");
        };
        let Some(region_ids) = topology.region_cards(region_name) else {
            bail!(
                "no region named `{region_name}` in topology `{}`",
                topology.name
            );
        };
        let ids: std::collections::HashSet<u64> = region_ids.iter().copied().collect();
        cards.retain(|c| ids.contains(&c.id()));
    }

    // A workspace member drills under that workspace's `alix.local.toml` pacing
    // override (retention + retirement), else the global `[review]` config.
    let review = config
        .review
        .for_workspace(deck.parent().unwrap_or_else(|| Path::new("")));

    // Inject this deck's virtual (remediation) cards alongside its authored
    // ones, so both are drilled by the same FSRS-due queue — but not under a
    // `--region` focus: a region is a deck-topology drill, and virtual cards
    // aren't part of any topology. `decks` has exactly this one deck's entry
    // (one deck per session), keyed by its subject — the same string a
    // virtual card's `parent` is set to.
    let subject: Arc<str> = decks
        .keys()
        .next()
        .map(|s| Arc::from(s.as_str()))
        .unwrap_or_else(|| Arc::from(label.as_str()));
    if region_sel.is_none() {
        for (k, vc) in store
            .virtual_cards_for(subject.as_ref())
            .into_iter()
            .filter(|v| !alix::session::is_retired_id(v.id, store, review.retire_after_days))
            .filter(|v| !deck_ids.contains(&v.id)) // collision belt-and-suspenders
            .enumerate()
        {
            if let Some(mut card) = synthesize_virtual(vc, &subject, VIRTUAL_LINE_BASE + k) {
                // Reshape/note a synth card exactly as deck cards are above
                // (§8.1) — this loop runs after that one, so it must repeat the
                // same two steps rather than widening the earlier loop's range.
                augment.apply_format(&mut card);
                if let Some(note) = augment.note(card.id()) {
                    card.append_note(&[note.to_string()]);
                }
                cards.push(card);
            }
        }
    }

    // Directives (order) come from the session's decks — the `% order:`
    // directive, else the scheduled default (the CLI override is gone; order
    // is authored, not launched).
    let target_settings: Vec<&DeckSettings> = settings.iter().collect();
    let order = resolve(
        "order",
        None,
        target_settings.iter().map(|s| s.order),
        Order::default(),
    );

    // The session depth: an explicit `--depth` / picker choice, else the deck's
    // last-used depth (keyed by deck subject, like the rest of the deck store),
    // else the default (Recall). The web select handler persists the resolved
    // value back to the store so a plain Learn reopens at it.
    let depth = depth_sel
        .or_else(|| store.last_depth(subject.as_ref()))
        .unwrap_or_default();
    // Pacing: the launch's own overrides win over the instance's flag/config
    // values; cram is purely a per-launch choice (the ▾ menu tick-box).
    let options = SessionOptions {
        max_new: opts.max_new.unwrap_or(pacing.max_new),
        limit: opts.limit.or(pacing.limit),
        cram: opts.cram,
        order,
        topology: topology_order,
        retire_after_days: review.retire_after_days,
        depth,
    };
    let session = Session::new(
        cards,
        store,
        Box::new(Fsrs::new(review.retention)),
        options,
        now_ms(),
    );

    // Remember these decks for next time's picker — but only when there is
    // actually something to review, so merely opening a deck with nothing due
    // doesn't bump it to the top of the recent list.
    if !session.is_finished() {
        recent.record(&deck_paths, now_ms());
        let _ = recent.save();
    }

    Ok(ReviewBuild {
        session,
        label,
        decks,
        topology_name,
    })
}

/// Resolves which stored topology, if any, reorders this session: an explicit
/// `--topology <name>` must name a cached topology (else an error), no flag with
/// exactly one cached topology auto-uses it, and zero-or-several without a name
/// leaves ordering to the scheduler.
fn resolve_topology<'a>(
    name: Option<&str>,
    augment: &'a AugmentCache,
    deck_ids: &std::collections::HashSet<u64>,
) -> Result<Option<&'a Topology>> {
    // Only this deck's topologies — a shared cache (decks sharing a store) holds
    // others', which must not be auto-applied or named here.
    let mine = augment.topologies_for(deck_ids);
    match name {
        Some(name) => match mine.into_iter().find(|t| t.name == name) {
            Some(topology) => Ok(Some(topology)),
            None => bail!(
                "no topology named `{name}` is cached for this deck — run `alix deck augment <deck> --target topology`"
            ),
        },
        None => Ok(match mine.as_slice() {
            [single] => Some(*single),
            _ => None,
        }),
    }
}

/// If a single trace deck was picked, returns its loaded deck — the signal to
/// walk it (predict → verify) rather than flatten it into a card review.
fn single_trace_to_walk(deck_paths: &[PathBuf]) -> Option<Deck> {
    match deck_paths {
        [path] => Deck::load(path).ok().filter(|deck| deck.is_trace()),
        _ => None,
    }
}

/// Builds the browse card list from explicit `deck_paths` (no picker). Mirrors
/// [`build_review`] for the read-only browse view: loads decks, but builds no
/// scheduler session.
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

/// Subject → deck file path, for the web frontend's card removal.
fn subject_paths(decks: HashMap<String, DeckInfo>) -> HashMap<String, PathBuf> {
    decks
        .into_iter()
        .map(|(subject, info)| (subject, info.path))
        .collect()
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
    let pacing = Pacing {
        max_new: args.new.or(config.review.max_new).unwrap_or(10),
        limit: args.limit.or(config.review.limit),
    };

    // Adapts a built review session to what the server holds (session + label +
    // subject→path map for removal, subject→`% link:` links for ask-Claude).
    let to_build = |b: ReviewBuild| {
        let links = b
            .decks
            .iter()
            .map(|(subject, info)| (subject.clone(), info.links.clone()))
            .collect();
        // Subject → `% source:` project root, but only for decks whose effective
        // source access is on — so the web tutor grounds exactly those.
        let source_roots = b
            .decks
            .iter()
            .filter(|(_, info)| info.source_access)
            .filter_map(|(subject, info)| {
                info.source_root.clone().map(|root| (subject.clone(), root))
            })
            .collect();
        // Subject → source base, so the web can resolve a card's `% at:` citation.
        let source_bases = b
            .decks
            .iter()
            .map(|(subject, info)| (subject.clone(), info.source_base.clone()))
            .collect();
        serve::SessionBuild {
            session: b.session,
            label: b.label,
            decks: subject_paths(b.decks),
            links,
            source_roots,
            source_bases,
            topology_name: b.topology_name,
        }
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
        auth: token,
        config_path: args.config.clone(),
        pair,
        scoped,
    };
    let build = |paths: Vec<PathBuf>,
                 opts: &serve::SelectOptions,
                 store: &Store,
                 recent: &mut RecentDecks| {
        build_review(paths, pacing, &config, store, recent, opts).map(to_build)
    };
    // A single trace picked from the in-browser picker walks (predict → verify)
    // rather than flattening to a card review.
    let build_walk = |paths: &[PathBuf]| -> Result<Option<serve::WalkBuild>> {
        match single_trace_to_walk(paths) {
            Some(deck) => {
                let trace = Trace::from_deck(&deck)?;
                Ok(Some(serve::WalkBuild {
                    walk: Walk::new(trace),
                    // Opt-in AI grading of predictions (`[trace] auto_grade`).
                    grade: config.trace.auto_grade.then(|| config.ask.clone()),
                }))
            }
            None => Ok(None),
        }
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
        build,
        build_walk,
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
fn local_lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Renders `text` as a terminal QR so a phone pairs by scanning; silently
/// skipped when the text is too long (the printed URL above still works).
fn print_qr(text: &str) {
    if let Some(q) = alix::qr::terminal_blocks(text) {
        print!("{q}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alix::answer::Mode;

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
    fn single_trace_to_walk_only_for_a_lone_trace_deck() {
        let dir = tempfile::tempdir().unwrap();
        let trace = dir.path().join("t.txt");
        std::fs::write(
            &trace,
            "% trace: how it works\n% source: .\n\n# q\n\tpoint\n\t% at: 1\n",
        )
        .unwrap();
        let fact = dir.path().join("f.txt");
        std::fs::write(&fact, "# q\n\ta\n").unwrap();

        // A lone trace → walk it.
        assert!(single_trace_to_walk(std::slice::from_ref(&trace)).is_some());
        // A lone facts deck → review, not walk.
        assert!(single_trace_to_walk(std::slice::from_ref(&fact)).is_none());
        // A trace alongside other decks isn't a lone trace → review/merge.
        assert!(single_trace_to_walk(&[trace, fact]).is_none());
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

    #[test]
    fn expand_workspaces_member_file_inherits_workspace_settings() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("eng");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
        std::fs::write(ws.join("alix.toml"), "[defaults]\ndirection = \"both\"\n").unwrap();

        // A member picked as a bare file (a subset selection) still inherits the
        // workspace's directives.
        let exp = expand_workspaces(&[ws.join("a.txt")]).unwrap();
        assert_eq!(1, exp.decks.len());
        assert_eq!(
            Some(alix::card::Direction::Both),
            exp.defaults.get("a.txt").unwrap().direction
        );
    }

    /// The default per-session pacing for a `build_review` test: the built-in
    /// `max_new`, no session cap.
    fn test_pacing() -> Pacing {
        Pacing {
            max_new: 10,
            limit: None,
        }
    }

    /// Inserts a virtual (remediation) card for deck `subject` into `store` the
    /// way the substrate does — sidecar content keyed by its `Card::id`, plus a
    /// fresh schedule seeded at `t=0` (so it's due, not treated as unseen).
    fn insert_virtual_card(store: &mut Store, subject: &str) {
        use alix::store::VirtualKind;
        let text = "# virtual front\n\tvirtual back\n".to_string();
        let id = parser::parse_str(subject, &text).unwrap()[0].id();
        store.insert_virtual(VirtualCard {
            id,
            kind: VirtualKind::Remediation,
            parent: subject.to_string(),
            text,
            created_ms: 0,
        });
        store.get_or_insert(id, 0);
    }

    #[test]
    fn build_review_injects_a_decks_virtual_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.txt");
        std::fs::write(&path, "# q1\n\ta1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override — a
        // bare `None` here would fall through to the real global data dir.
        let store_path = Some(dir.path().join("store.json"));
        let mut store = store_for(std::slice::from_ref(&path), store_path).unwrap();
        insert_virtual_card(&mut store, "rust.txt");

        let config = Config::default();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let build = build_review(
            vec![path],
            test_pacing(),
            &config,
            &store,
            &mut recent,
            &serve::SelectOptions::default(),
        )
        .unwrap();
        // The deck's one (new) card, plus the injected due virtual card.
        assert_eq!(2, build.session.initial_size);
    }

    #[test]
    fn region_focus_excludes_virtual_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.txt");
        std::fs::write(&path, "# q1\n\ta1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override — a
        // bare `None` here would fall through to the real global data dir.
        let store_path = Some(dir.path().join("store.json"));
        let mut store = store_for(std::slice::from_ref(&path), store_path).unwrap();

        let deck = Deck::load(&path).unwrap();
        let card_id = deck.cards[0].id();

        // Cache a one-region topology covering this deck's one card.
        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.add_topology(Topology {
            name: "auto".to_string(),
            principle: "test".to_string(),
            edges: vec![],
            walk: vec![card_id],
            regions: vec![augment::TopologyRegion {
                name: "r1".to_string(),
                cards: vec![card_id],
            }],
        });
        cache.save().unwrap();

        // A matching virtual card for this deck.
        insert_virtual_card(&mut store, "rust.txt");

        let config = Config::default();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let build = build_review(
            vec![path],
            test_pacing(),
            &config,
            &store,
            &mut recent,
            &serve::SelectOptions {
                region: Some("r1".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        // Only the region's one real card — a `--region` focus is a
        // deck-topology drill, and virtual cards aren't part of any topology.
        assert_eq!(1, build.session.initial_size);
    }

    #[test]
    fn a_format_cache_entry_applies_to_a_synthesized_virtual_card() {
        // A synthesized virtual card has a real `Card::id`, so an existing
        // format-cache entry for that id applies with no change to
        // `apply_format` itself — the "free" half of augment-for-virtuals (§8.1).
        let subject: Arc<str> = Arc::from("rust.txt");
        let text = "# List the parts\n\tA, B, C\n".to_string();
        let id = parser::parse_str(&subject, &text).unwrap()[0].id();
        let vc = VirtualCard {
            id,
            kind: alix::store::VirtualKind::Remediation,
            parent: subject.to_string(),
            text,
            created_ms: 0,
        };
        let mut synth = synthesize_virtual(&vc, &subject, VIRTUAL_LINE_BASE).unwrap();

        let mut cache =
            AugmentCache::open(std::env::temp_dir().join("nonexistent-augment-virtual.json"));
        cache.set_format(
            id,
            augment::Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: Some(Mode::LineByLine),
            },
        );
        cache.apply_format(&mut synth);

        assert_eq!("Name the parts", synth.front);
        assert_eq!(["A", "B", "C"], *synth.back_for_display());
        assert_eq!(id, synth.id(), "reshaping must not change identity");
    }

    #[test]
    fn build_review_applies_a_cached_format_to_an_injected_virtual_card() {
        // The display half of augment-for-virtuals (§8.1): `build_review` must
        // reshape an injected synth card the same way it reshapes deck cards.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust.txt");
        std::fs::write(&path, "# q1\n\ta1\n").unwrap();
        // Not a workspace, so pass an explicit `--store`-style override — a
        // bare `None` here would fall through to the real global data dir.
        let store_path = Some(dir.path().join("store.json"));
        let mut store = store_for(std::slice::from_ref(&path), store_path).unwrap();
        insert_virtual_card(&mut store, "rust.txt");
        let virtual_id =
            parser::parse_str("rust.txt", "# virtual front\n\tvirtual back\n").unwrap()[0].id();

        let mut cache = AugmentCache::open(augment::augment_path_for(store.path()));
        cache.set_format(
            virtual_id,
            augment::Format {
                front: Some("Reshaped virtual front".to_string()),
                back: vec!["Reshaped virtual back".to_string()],
                note: None,
                mode: None,
            },
        );
        cache.save().unwrap();

        let config = Config::default();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        let build = build_review(
            vec![path],
            test_pacing(),
            &config,
            &store,
            &mut recent,
            &serve::SelectOptions::default(),
        )
        .unwrap();

        let synth = build
            .session
            .cards()
            .iter()
            .find(|c| c.id() == virtual_id)
            .expect("the injected virtual card should be in the session");
        assert_eq!("Reshaped virtual front", synth.front);
        assert_eq!(["Reshaped virtual back"], *synth.back_for_display());
    }
}
