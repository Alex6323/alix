use std::{
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
};

use alix::{
    assemble, config::Config, recent::RecentDecks, serve, state::UserFiles, tutorial, workspace,
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
const MIN_LAN_TOKEN_CHARS: usize = 16;

fn resolve_serve_token(cli: Option<String>, lan: bool, config: &Config) -> Result<Option<String>> {
    if let Some(t) = cli
        .or_else(|| config.serve.token.clone())
        .filter(|t| !t.is_empty())
    {
        let chars = t.chars().count();
        if lan && chars < MIN_LAN_TOKEN_CHARS {
            bail!(
                "the pairing token has {chars} characters; `--lan` needs at least \
                 {MIN_LAN_TOKEN_CHARS} (drop `--token` and `[serve] token` to have one minted)"
            );
        }
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

fn log_settings(config: &Config, targets: &[alix::log::Target]) -> alix::log::Settings {
    alix::log::Settings {
        max_bytes: config.log.max_bytes,
        verbose: config.log.verbose || !targets.is_empty(),
        stderr: alix::log::Targets::from_slice(targets),
    }
}

pub(crate) fn launch(args: LaunchArgs, instance: &str) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let logging = log_settings(&config, &args.log);
    if let Err(error) = alix::log::init(instance, logging) {
        eprintln!("warning: could not open the server log: {error}");
    }
    let log_path = alix::log::log_path(instance);
    let scoped = args.dir.is_some();
    let (decks_dir, user_root) = match &args.dir {
        None => {
            let dir = config.decks_dir().context("cannot determine ~/decks")?;
            tutorial::seed_new_decks_dir(&dir);
            let user_root = workspace::root_store_path(&dir);
            (dir, user_root)
        }
        Some(path) if path.is_file() => bail!(
            "`alix <deck>` was removed. Run `alix` and pick the deck there, \
             or serve its folder: `alix {}`",
            path.parent().unwrap_or_else(|| Path::new(".")).display()
        ),
        Some(path) if !path.is_dir() => bail!("`{}` is not a folder", path.display()),
        Some(path) => (path.clone(), workspace::root_store_path(path)),
    };
    let recent = RecentDecks::load(UserFiles::new(&user_root).recent());
    let instance_store = Some(user_root);
    let store = assemble::open_store_tolerant(instance_store.clone())?;
    let addr = serve_addr(args.port, args.lan, &config);
    // Bind before announcing: a taken port errors here rather than after printing a success URL.
    // `Arc`-shared so `run_review` can be stopped from outside its own thread.
    let server = Arc::new(serve::bind(addr)?);
    // Announce what the kernel bound, not what was asked: `--port 0` prints
    // the assigned port instead of an unreachable `:0` URL.
    let addr = server.server_addr().to_ip().unwrap_or(addr);
    let stopper = Arc::clone(&server);
    // Ctrl-C/SIGTERM drains the workers via the unblock relay; `run_review`
    // then flushes and returns, so the process exits cleanly instead of dying
    // mid-write.
    ctrlc::set_handler(move || stopper.unblock()).context("cannot install the shutdown handler")?;
    let pacing = assemble::Pacing {
        max_session: args
            .session
            .or(config.review.max_session)
            .unwrap_or(alix::session::DEFAULT_MAX_SESSION),
        new_cards_percent: config
            .review
            .new_cards_percent
            .unwrap_or(alix::session::DEFAULT_NEW_CARDS_PERCENT),
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
        log_path,
        pair,
        scoped,
        cfg: assemble::AssembleConfig {
            review: config.review,
            ask: config.ask.clone(),
            pacing,
            instance_store: instance_store.clone(),
        },
    };
    serve::run_review(store, recent, decks_dir, server, opts)
}

// Lenient on purpose: std's `println!` panics on EPIPE, and a long-running
// server must not die because the consumer of its stdout went away
// (`alix | head`, a pipe closed after reading the URL line).
macro_rules! say {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

fn announce(addr: SocketAddr, lan: bool, token: Option<&str>, root: &Path) -> serve::PairInfo {
    announce_with(addr, lan, lan.then(local_lan_ip).flatten(), token, root)
}

/// The host lookup is the caller's, so the pairing URL is decided by a value
/// rather than by whatever the machine's routing table answers.
fn announce_with(
    addr: SocketAddr,
    lan: bool,
    lan_ip: Option<std::net::IpAddr>,
    token: Option<&str>,
    root: &Path,
) -> serve::PairInfo {
    let root = abbreviate_home(root);
    let port = addr.port();
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
                say!("Serving {root} at http://{ip}:{port}");
                say!("On another device, open in a browser (or scan):");
                say!("  {}", pair.url);
                print_qr(&pair.url);
                say!("Or pair the app with:  host {ip}  port {port}  token {t}");
            }
            None => {
                say!("Serving {root} on all interfaces, port {port}.");
                say!("On another device, open in a browser:");
                say!("  http://<this-machine's-IP>:{port}/?token={t}");
                say!("Or pair the app with:  host <this-machine's-IP>  port {port}  token {t}");
            }
        },
        (true, None) => {
            say!("Serving {root} on all interfaces, port {port}.");
            say!("warning: no authentication — anyone on your network can reach this.");
        }
        (false, _) => {
            say!("Serving {root} at http://127.0.0.1:{port}. Open it in your browser.")
        }
    }
    say!("Press Ctrl-C to stop.");
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
pub(crate) fn local_lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Silently skipped when the text is too long; the printed URL above still works.
#[cfg_attr(coverage_nightly, coverage(off))]
fn print_qr(text: &str) {
    if let Some(q) = alix::qr::terminal_blocks(text) {
        use std::io::Write;
        let _ = write!(std::io::stdout(), "{q}");
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
        assert!(
            resolve_serve_token(None, true, &cfg)
                .unwrap()
                .is_some_and(|t| !t.is_empty())
        );
    }

    #[test]
    fn an_explicit_token_shorter_than_sixteen_characters_is_refused_only_on_the_lan() {
        let long = "sixteen-chars-ok";
        let sixteen_multibyte = "äöüäöüäöüäöüäöüä";
        let rows = [
            ("abc", false, true),
            ("abc", true, false),
            ("fifteen-chars-x", true, false),
            (long, true, true),
            (sixteen_multibyte, true, true),
        ];
        for (token, lan, accepted) in rows {
            let mut cfg = Config::default();
            cfg.serve.token = Some(token.to_string());
            for (label, cli, config) in [
                ("cli", Some(token.to_string()), &Config::default()),
                ("config", None, &cfg),
            ] {
                let result = resolve_serve_token(cli, lan, config);
                match (accepted, result) {
                    (true, Ok(Some(t))) => assert_eq!(token, t, "{label} {token:?} lan={lan}"),
                    (false, Err(error)) => assert!(
                        error.to_string().contains("needs at least 16")
                            && !error.to_string().contains(token),
                        "{label} {token:?} lan={lan}: {error}"
                    ),
                    (_, other) => panic!("{label} {token:?} lan={lan}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn generated_tokens_are_distinct_128_bit_lowercase_hex() {
        let first = generate_token().unwrap();
        let second = generate_token().unwrap();
        for token in [&first, &second] {
            assert_eq!(32, token.len(), "{token}");
            assert!(
                token
                    .chars()
                    .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch)),
                "{token}"
            );
        }
        assert_ne!(first, second);
    }

    #[test]
    fn naming_a_live_log_target_also_enables_verbose_file_records() {
        let config = Config::default();
        assert!(!log_settings(&config, &[]).verbose);
        assert!(log_settings(&config, &[alix::log::Target::Http]).verbose);
    }

    #[test]
    fn qr_output_child() {
        if std::env::var_os("ALIX_QR_OUTPUT_CHILD").is_none() {
            return;
        }
        print_qr("http://127.0.0.1:4321/?token=0123456789abcdef");
    }

    #[test]
    fn print_qr_emits_the_terminal_rendering() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "launch::tests::qr_output_child", "--nocapture"])
            .env("ALIX_QR_OUTPUT_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains('█'), "{stdout}");
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

    #[test]
    fn the_pairing_url_follows_the_supplied_host_address() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let addr = SocketAddr::from(([127, 0, 0, 1], 7780));
        let root = std::path::Path::new("/tmp/decks");
        let lan_ip = Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42)));

        // A token and a resolved address pair over the LAN.
        let paired = announce_with(addr, true, lan_ip, Some("tok"), root);
        assert_eq!("http://192.168.1.42:7780/?token=tok", paired.url);
        assert!(paired.lan);

        // No address resolved: fall back to loopback rather than inventing one.
        let unresolved = announce_with(addr, true, None, Some("tok"), root);
        assert_eq!("http://127.0.0.1:7780/", unresolved.url);
        assert!(!unresolved.lan);

        // An address without a token is not a pairing URL either.
        let tokenless = announce_with(addr, true, lan_ip, None, root);
        assert_eq!("http://127.0.0.1:7780/", tokenless.url);
        assert!(!tokenless.lan);
    }
}
