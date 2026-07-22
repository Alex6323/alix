use std::{
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
};

use alix::{
    assemble::{self, open_store},
    config::Config,
    recent::RecentDecks,
    serve, tutorial, workspace,
};
use anyhow::{Context, Result, bail};

use crate::LaunchArgs;

fn serve_addr(port: Option<u16>, lan: bool, config: &Config) -> SocketAddr {
    let ip = if lan {
        Ipv4Addr::UNSPECIFIED
    } else {
        Ipv4Addr::LOCALHOST
    };
    SocketAddr::from((ip, port.unwrap_or(config.serve.port)))
}

/// Fails closed: if `--lan` needs a token but generation fails, this errors
/// rather than leaving the network API open.
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

pub(crate) fn generate_token() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)
        .map_err(|e| anyhow::anyhow!("could not generate a serve pairing token: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

pub(crate) fn launch(args: LaunchArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let scoped = args.dir.is_some();
    let (decks_dir, instance_store, recent_path) = match &args.dir {
        None => {
            let dir = config.decks_dir().context("cannot determine ~/decks")?;
            tutorial::seed_new_decks_dir(&dir);
            let store = workspace::root_store_path(&dir);
            let recent = dir.join("recent.json");
            (dir, Some(store), recent)
        }
        Some(path) if path.is_file() => bail!(
            "`alix <deck>` was removed — run `alix` and pick the deck there, \
             or serve its folder: `alix {}`",
            path.parent().unwrap_or_else(|| Path::new(".")).display()
        ),
        Some(path) if !path.is_dir() => bail!("`{}` is not a folder", path.display()),
        Some(path) => (
            path.clone(),
            Some(workspace::root_store_path(path)),
            path.join("recent.json"),
        ),
    };
    let recent = RecentDecks::load(recent_path);
    let store = open_store(instance_store.clone())?;
    let addr = serve_addr(args.port, args.lan, &config);
    // Bind before announcing: a taken port errors here rather than after printing a success URL.
    // `Arc`-shared so `run_review` can be stopped from outside its own thread.
    let server = Arc::new(serve::bind(addr)?);
    let pacing = assemble::Pacing {
        max_new: args.new.or(config.review.max_new).unwrap_or(10),
        limit: args.limit.or(config.review.limit),
    };

    let token = resolve_serve_token(args.token.clone(), args.lan, &config)?;
    let pair = announce(addr, args.lan, token.as_deref(), &decks_dir);

    let opts = serve::ReviewOptions {
        keys: config.keys.clone(),
        picker: config.picker.clone(),
        browse: config.browse.clone(),
        exam: config.exam.clone(),
        ai: config.ai.clone(),
        generate: config.generate.clone(),
        audience: config.serve.audience,
        auth: token,
        config_path: args.config.clone(),
        pair,
        scoped,
        cfg: assemble::AssembleConfig {
            review: config.review,
            ask: config.ask.clone(),
            trace_auto_grade: config.trace.auto_grade,
            pacing,
            instance_store: instance_store.clone(),
        },
    };
    serve::run_review(store, recent, decks_dir, server, opts)
}

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

fn abbreviate_home(path: &Path) -> String {
    if let Some(dirs) = directories::BaseDirs::new()
        && let Ok(rest) = path.strip_prefix(dirs.home_dir())
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// A UDP `connect` only resolves a route via the OS routing table; no packet
/// is actually sent.
// Called only from `announce`; additionally depends on a real OS routing
// table via a live UDP socket, which is not deterministic across CI network
// sandboxes even with a server harness.
#[cfg_attr(coverage_nightly, coverage(off))]
fn local_lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Silently skipped when the text is too long; the printed URL above still works.
// Print-only, two-line delegation to `qr::terminal_blocks`: nothing to
// assert without a stdout-capture idiom, which this codebase doesn't have.
#[cfg_attr(coverage_nightly, coverage(off))]
fn print_qr(text: &str) {
    if let Some(q) = alix::qr::terminal_blocks(text) {
        print!("{q}");
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn serve_token_is_generated_only_when_exposed() {
        let cfg = Config::default();
        assert_eq!(resolve_serve_token(None, false, &cfg).unwrap(), None);
        assert_eq!(
            resolve_serve_token(Some("abc".into()), true, &cfg).unwrap(),
            Some("abc".into())
        );
        assert!(
            resolve_serve_token(None, true, &cfg)
                .unwrap()
                .is_some_and(|t| !t.is_empty())
        );
    }

    #[test]
    fn announce_local_only_returns_a_loopback_pair_regardless_of_token() {
        // lan=false never touches local_lan_ip/print_qr, so this is
        // deterministic regardless of token.
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
            // No resolvable home dir here; nothing to verify.
            return;
        };
        let path = dirs.home_dir().join("decks").join("rust.md");
        let expected = format!("~/{}", Path::new("decks").join("rust.md").display());
        assert_eq!(abbreviate_home(&path), expected);
    }

    #[test]
    fn abbreviate_home_leaves_a_path_outside_home_unchanged() {
        let outside = PathBuf::from("/definitely-not-the-home-dir-xyz/decks/rust.md");
        if let Some(dirs) = directories::BaseDirs::new()
            && outside.starts_with(dirs.home_dir())
        {
            // Pathological environment (home dir at/above this path): skip
            // rather than assert something untrue here.
            return;
        }
        assert_eq!(abbreviate_home(&outside), outside.display().to_string());
    }
}
