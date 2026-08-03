use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use super::*;
#[cfg(unix)]
use crate::source::SourceBase;
use crate::{
    answer::{Input, Mode, mode_name},
    ask::{self, Reply},
    augment::AugmentCache,
    cache::DeckCache,
    card::{Card, CardImage},
    choice,
    config::{AskConfig, ReviewConfig},
    deck::Deck,
    depth::Depth,
    picker,
    recent::RecentDecks,
    render::NoteUnit,
    scheduler::{Fsrs, Grade},
    session::{CardTier, Session, now_ms},
    store::Store,
    trace::{Delta, Walk},
};

/// A panicked owner must drain an idle server by itself: the trip unblocks
/// tiny_http directly instead of waiting for a next request to notice the
/// flag. Bounded receives keep a regression a failure, never a hang.
#[test]
fn a_panicked_owner_trips_the_failure_and_unblocks_an_idle_server() {
    let server = Arc::new(crate::serve::bind("127.0.0.1:0".parse().unwrap()).unwrap());
    let failure = OwnerFailure::new(Arc::clone(&server));

    let owner = supervised(failure.clone(), || panic!("injected owner failure"));
    assert!(owner.join().is_err(), "the panic must propagate to join");
    assert!(failure.tripped());

    let (tx, rx) = std::sync::mpsc::channel();
    let recv_server = Arc::clone(&server);
    std::thread::spawn(move || {
        let _ = tx.send(recv_server.recv().is_err());
    });
    let unblocked = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("recv must return, not block: the trip queued an unblock");
    assert!(unblocked, "the drained recv reports Err, not a request");
}

#[test]
fn run_review_remains_alive_until_the_server_is_unblocked() {
    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join("progress.json");
    let store = Store::open(&store_path).unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));
    let decks_dir = dir.path().to_path_buf();
    let server = Arc::new(crate::serve::bind("127.0.0.1:0".parse().unwrap()).unwrap());
    let port = server
        .server_addr()
        .to_ip()
        .expect("bound to a loopback IP")
        .port();
    let config = crate::config::Config::default();
    let audience = config.serve.audience;
    let opts = ReviewOptions {
        keys: config.keys,
        picker: config.picker,
        browse: config.browse,
        exam: config.exam,
        ai: config.ai,
        generate: config.generate,
        audience,
        auth: None,
        config_path: None,
        pair: PairInfo {
            url: format!("http://127.0.0.1:{port}"),
            lan: false,
        },
        scoped: true,
        cfg: assemble::AssembleConfig {
            review: config.review,
            ask: config.ask,
            trace_auto_grade: false,
            pacing: assemble::Pacing {
                max_session: 10,
                new_cards_percent: 30,
            },
            instance_store: Some(store_path),
        },
    };

    let stop = Arc::clone(&server);
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(run_review(store, recent, decks_dir, server, opts));
    });

    assert!(
        matches!(
            rx.recv_timeout(std::time::Duration::from_millis(200)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "run_review returned before the server was asked to stop"
    );

    stop.unblock();
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("run_review must finish after the server is unblocked")
        .expect("run_review shutdown must succeed");
    handle.join().expect("run_review thread must not panic");
}

fn write_initialized(path: &Path, text: &str) {
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("deck")
        .replace('-', "");
    std::fs::write(
        path,
        format!("---\nformat-version: 1\nid: \"deck-{id}\"\n---\n{text}"),
    )
    .unwrap();
}

#[test]
fn unconfigured_token_leaves_everything_open() {
    assert!(is_authorized("/api/decks", None, None, None));
    assert!(is_authorized("/", None, None, None));
}

#[test]
fn tutor_url_source_context_requires_a_fetch_capable_grant() {
    assert!(can_fetch_url_sources(&AskConfig::default()));

    let no_fetch = AskConfig {
        allowed_tools: Vec::new(),
        ..AskConfig::default()
    };
    assert!(!can_fetch_url_sources(&no_fetch));

    let codex = AskConfig {
        backend: crate::config::BackendKind::Codex,
        ..AskConfig::default()
    };
    assert!(!can_fetch_url_sources(&codex));
}

#[test]
fn token_guards_only_the_api() {
    let t = Some("secret");
    assert!(is_authorized("/", None, None, t));
    assert!(is_authorized("/img/deadbeef", None, None, t));
    assert!(is_authorized("/theme.css", None, None, t));
    assert!(is_authorized("/review.css", None, None, t));
    assert!(is_authorized("/review.js", None, None, t));
    assert!(!is_authorized("/api/decks", None, None, t));
    assert!(!is_authorized("/api/decks", Some("Bearer wrong"), None, t));
    assert!(is_authorized("/api/decks", Some("Bearer secret"), None, t));
    assert!(is_authorized("/api/decks", None, Some("secret"), t));
}

#[test]
fn constant_time_token_equality_rejects_every_same_length_difference() {
    let cases: &[(&[u8], &[u8], bool)] = &[
        (b"", b"", true),
        (b"same", b"same", true),
        (b"same", b"sand", false),
        (b"\0\0", b"\x01\x01", false),
        (b"short", b"longer", false),
    ];

    for &(left, right, expected) in cases {
        assert_eq!(
            expected,
            ct_eq(left, right),
            "left={left:?}, right={right:?}"
        );
    }
}

#[test]
fn icon_field_registers_an_svg_and_flags_it() {
    let mut icons = HashMap::new();
    let (url, is_svg) = icon_field(Some(Path::new("/ws/assets/icon.svg")), &mut icons);
    let url = url.unwrap();
    assert!(url.starts_with("/img/"));
    assert!(is_svg);
    assert_eq!(icons.len(), 1);
    assert!(icons.values().any(|p| p.ends_with("assets/icon.svg")));

    let (none, flag) = icon_field(None, &mut icons);
    assert!(none.is_none() && !flag);
    assert_eq!(icons.len(), 1);
}

#[test]
fn a_non_ascii_deck_name_yields_an_ascii_download_filename() {
    let name = download_filename("mövenpick-decks.zip");
    assert!(name.is_ascii());
    assert!(name.ends_with(".zip"));
}

#[test]
fn a_fully_non_ascii_name_falls_back_to_a_generic_filename() {
    assert_eq!(download_filename("日本語.zip"), "decks.zip");
}

#[test]
fn quotes_and_backslashes_are_stripped_from_download_filenames() {
    let name = download_filename("weird\"na\\me.zip");
    assert!(!name.contains('"'));
    assert!(!name.contains('\\'));
}

#[test]
fn each_forbidden_download_filename_character_is_removed_independently() {
    for (input, expected) in [
        ("quo\"te.zip", "quote.zip"),
        ("back\\slash.zip", "backslash.zip"),
        ("line\nbreak.zip", "linebreak.zip"),
    ] {
        assert_eq!(expected, download_filename(input), "input={input:?}");
    }
}

#[test]
fn an_uppercase_md_extension_is_lowered_before_placing() {
    let name = "FILE.MD";
    let lower = name.to_ascii_lowercase();
    assert_eq!(normalize_md_extension(name, &lower), "FILE.md");
}

#[test]
fn a_lowercase_md_extension_passes_through_unchanged() {
    let name = "deck.md";
    let lower = name.to_ascii_lowercase();
    assert_eq!(normalize_md_extension(name, &lower), "deck.md");
}

#[test]
fn a_tsv_name_is_left_untouched_by_the_md_normalizer() {
    let name = "EXPORT.TSV";
    let lower = name.to_ascii_lowercase();
    assert_eq!(normalize_md_extension(name, &lower), "EXPORT.TSV");
}

#[test]
fn card_dto_structures_the_note() {
    let note = "Intro here.\n```\nfn main() {}\n```";
    let card = Card::plain(
        Arc::from("s.md"),
        "the front".to_string(),
        vec!["the back".to_string()],
        Some(note.to_string()),
        1,
    );
    let dto = card_dto((&card).into(), card.id());

    assert_eq!(dto.front, "the front");
    assert_eq!(dto.back, vec!["the back".to_string()]);
    assert_eq!(dto.note.len(), 2);
    match &dto.note[0] {
        NoteUnit::Sentence { text, .. } => assert_eq!(text, "Intro here."),
        other => panic!("expected a sentence, got {other:?}"),
    }
    match &dto.note[1] {
        NoteUnit::Code { lines } => assert_eq!(lines, &vec!["fn main() {}".to_string()]),
        other => panic!("expected a code block, got {other:?}"),
    }
}

#[test]
fn card_dto_exposes_image_urls_and_registry_matches() {
    let mut card = Card::plain(
        Arc::from("s.md"),
        "q".to_string(),
        vec!["a".to_string()],
        None,
        1,
    );
    card.images = vec![CardImage {
        src: PathBuf::from("/imgs/moon.png"),
        alt: None,
    }];
    card.images_back = vec![CardImage {
        src: PathBuf::from("/imgs/tab.png"),
        alt: None,
    }];

    let dto = card_dto((&card).into(), card.id());
    let img = &dto.images.first().expect("front image url").src;
    let img_back = &dto.images_back.first().expect("back image url").src;
    assert!(img.starts_with("/img/"));
    assert!(img_back.starts_with("/img/") && img_back != img);

    let images = collect_images(std::slice::from_ref(&card));
    assert_eq!(
        images.get(img.strip_prefix("/img/").unwrap()),
        Some(&PathBuf::from("/imgs/moon.png"))
    );
    assert_eq!(
        images.get(img_back.strip_prefix("/img/").unwrap()),
        Some(&PathBuf::from("/imgs/tab.png"))
    );
}

#[test]
fn plain_card_has_no_image_urls() {
    let card = Card::plain(
        Arc::from("s.md"),
        "q".to_string(),
        vec!["a".to_string()],
        None,
        1,
    );
    let dto = card_dto((&card).into(), card.id());
    assert!(dto.images.is_empty() && dto.images_back.is_empty());
    assert!(collect_images(std::slice::from_ref(&card)).is_empty());
}

#[test]
fn content_type_by_extension() {
    for (path, expected) in [
        ("a.png", "image/png"),
        ("a.JPG", "image/jpeg"),
        ("a.jpeg", "image/jpeg"),
        ("a.gif", "image/gif"),
        ("a.WEBP", "image/webp"),
        ("a.svg", "image/svg+xml"),
        ("a.bin", "application/octet-stream"),
    ] {
        assert_eq!(expected, content_type(Path::new(path)), "path={path}");
    }
}

#[test]
fn fonts_route_serves_woff2() {
    // No live-HTTP harness here (tiny_http's TestRequest writes to io::sink()),
    // so the route's lookup logic is exercised directly.
    for name in [
        "ibm-plex-sans-400.woff2",
        "ibm-plex-sans-500.woff2",
        "ibm-plex-sans-600.woff2",
        "ibm-plex-sans-700.woff2",
        "ibm-plex-mono-400.woff2",
        "ibm-plex-mono-500.woff2",
        "ibm-plex-mono-600.woff2",
        "ibm-plex-mono-700.woff2",
        "baloo2-400.woff2",
        "baloo2-500.woff2",
        "baloo2-600.woff2",
        "baloo2-700.woff2",
        "baloo2-800.woff2",
    ] {
        let bytes = font_bytes(name).unwrap_or_else(|| panic!("{name} should resolve"));
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[0..4], b"wOF2", "{name} is not a woff2 file");
    }
    assert!(font_bytes("nope.woff2").is_none());
    assert!(font_bytes("ibm-plex-sans-400.woff").is_none());
}

#[test]
fn app_page_dispatches_the_kids_page_for_kids_and_review_for_adult() {
    assert_ne!(app_page(Audience::Adult), app_page(Audience::Kids));
    assert!(app_page(Audience::Adult).contains("<title>alix</title>"));
    assert!(app_page(Audience::Adult).contains("href=\"/review.css\""));
    assert!(app_page(Audience::Adult).contains("src=\"/review.js\""));
    assert!(!app_page(Audience::Adult).contains("<style>"));
    let kids = app_page(Audience::Kids);
    assert!(kids.contains("alix kids"));
    assert!(kids.contains("href=\"/kids.css\""));
    assert!(kids.contains("src=\"/kids.js\""));
    assert!(!kids.contains("<style>"));
    assert!(!kids.contains("<script>"));
}

#[test]
fn adult_asset_manifest_matches_the_exact_composition_order() {
    let manifest: serde_json::Value = serde_json::from_str(REVIEW_ASSET_MANIFEST).unwrap();
    assert_eq!(
        REVIEW_CSS_SOURCES,
        &[
            "shell.css",
            "dom.css",
            "sheets.css",
            "study.css",
            "picker.css",
            "tutor.css",
            "exam.css",
            "augment.css",
            "walk.css",
        ]
    );
    assert_eq!(serde_json::json!(REVIEW_CSS_SOURCES), manifest["css"]);
    assert_eq!(serde_json::json!(REVIEW_JS_SOURCES), manifest["javascript"]);
    assert_eq!(Some(&"app.js"), REVIEW_JS_SOURCES.last());

    let (css, css_type) = web_asset("/review.css").unwrap();
    assert_eq!("text/css; charset=utf-8", css_type);
    assert!(css.contains(":root"));
    let (javascript, javascript_type) = web_asset("/review.js").unwrap();
    assert_eq!("application/javascript; charset=utf-8", javascript_type);
    assert!(javascript.contains("boot()"));

    for path in [
        "/review/shell.css",
        "/review/app.js",
        "/review.css/extra",
        "/review.js/extra",
    ] {
        assert!(web_asset(path).is_none(), "path: {path}");
    }
}

#[test]
fn kids_asset_manifest_matches_the_exact_composition_order() {
    let manifest: serde_json::Value = serde_json::from_str(KIDS_ASSET_MANIFEST).unwrap();
    assert_eq!(
        KIDS_CSS_SOURCES,
        &[
            "shell.css",
            "dom.css",
            "picker.css",
            "study.css",
            "tutor.css",
            "settings.css",
        ]
    );
    assert_eq!(
        KIDS_JS_SOURCES,
        &[
            "api.js",
            "model.js",
            "dom.js",
            "theme.js",
            "picker.js",
            "study.js",
            "tutor.js",
            "settings.js",
            "app.js",
        ]
    );
    assert_eq!(serde_json::json!(KIDS_CSS_SOURCES), manifest["css"]);
    assert_eq!(serde_json::json!(KIDS_JS_SOURCES), manifest["javascript"]);
    assert_eq!(Some(&"app.js"), KIDS_JS_SOURCES.last());

    let (css, css_type) = web_asset("/kids.css").unwrap();
    assert_eq!("text/css; charset=utf-8", css_type);
    assert!(css.contains(":root"));
    let (javascript, javascript_type) = web_asset("/kids.js").unwrap();
    assert_eq!("application/javascript; charset=utf-8", javascript_type);
    assert!(javascript.contains("createKidsPicker"));

    for path in [
        "/kids/shell.css",
        "/kids/app.js",
        "/kids.css/extra",
        "/kids.js/extra",
    ] {
        assert!(web_asset(path).is_none(), "path: {path}");
    }
}

#[test]
fn resolve_row_resolves_a_unique_bare_deck_name() {
    let dir = tempfile::tempdir().unwrap();
    write_initialized(&dir.path().join("solo.md"), "## f\nb\n");
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        Resolved::One(dir.path().join("solo.md")),
        resolve_row("solo.md", dir.path(), &recent, &mut DeckCache::default())
    );
}

#[test]
fn resolve_row_resolves_an_unknown_name_to_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        Resolved::Unknown,
        resolve_row(
            "../etc/passwd",
            dir.path(),
            &recent,
            &mut DeckCache::default()
        )
    );
}

#[test]
fn resolve_row_resolves_a_workspace_row_to_many_with_every_member_file() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    write_initialized(&ws.join("decks/a.md"), "## a\nb\n");
    write_initialized(&ws.join("decks/b.md"), "## c\nd\n");
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        Resolved::Many {
            dir: ws.clone(),
            files: vec![ws.join("decks/a.md"), ws.join("decks/b.md")],
        },
        resolve_row("english", dir.path(), &recent, &mut DeckCache::default())
    );
}

#[test]
fn resolve_row_keeps_a_manifest_only_workspace_addressable() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("empty-ws");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"Empty\"\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    let entries = picker::catalog(dir.path(), &recent, &mut DeckCache::default()).unwrap();
    assert_eq!(1, entries.len());
    assert!(entries[0].is_workspace);
    assert!(entries[0].members.is_empty());
    assert_eq!(
        Resolved::One(ws),
        resolve_row("empty-ws", dir.path(), &recent, &mut DeckCache::default())
    );
}

#[test]
fn library_targets_require_real_files_and_manifested_directories_in_every_resolution_shape() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("deck.md");
    std::fs::write(&deck, "deck\n").unwrap();
    assert!(matches!(
        library_target("deck.md".into(), Resolved::One(deck.clone())),
        Some(LibraryTarget::Deck { path, .. }) if path == deck
    ));
    assert!(
        library_target(
            "missing.md".into(),
            Resolved::One(dir.path().join("missing.md"))
        )
        .is_none()
    );

    let plain = dir.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    assert!(
        library_target("plain".into(), Resolved::One(plain.clone())).is_none(),
        "a directory is not a workspace without its manifest"
    );

    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join(crate::workspace::DECKS)).unwrap();
    std::fs::write(
        workspace.join(crate::workspace::MANIFEST),
        "title = \"Workspace\"\n",
    )
    .unwrap();
    assert!(matches!(
        library_target("workspace".into(), Resolved::One(workspace.clone())),
        Some(LibraryTarget::Workspace { root, members, .. })
            if root == workspace && members.is_empty()
    ));

    let supplied = vec![dir.path().join("supplied.md")];
    assert!(
        library_target(
            "plain-many".into(),
            Resolved::Many {
                dir: plain,
                files: supplied.clone(),
            },
        )
        .is_none(),
        "a many-row is not a workspace without its manifest"
    );
    assert!(matches!(
        library_target(
            "workspace-many".into(),
            Resolved::Many {
                dir: workspace.clone(),
                files: supplied.clone(),
            },
        ),
        Some(LibraryTarget::Workspace { root, members, .. })
            if root == workspace && members == supplied
    ));
}

#[test]
fn resolve_row_rejects_a_bare_name_duplicated_across_two_containers() {
    let dir = tempfile::tempdir().unwrap();
    write_initialized(&dir.path().join("a.md"), "## f\nb\n");
    let elsewhere = tempfile::tempdir().unwrap();
    write_initialized(&elsewhere.path().join("a.md"), "## g\nh\n");
    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[elsewhere.path().join("a.md")], 1000);

    assert_eq!(
        Resolved::Ambiguous,
        resolve_row("a.md", dir.path(), &recent, &mut DeckCache::default())
    );
}

#[test]
fn resolve_row_resolves_a_qualified_member_name_even_when_its_bare_workspace_name_is_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    write_initialized(&ws.join("decks/a.md"), "## a\nb\n");
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();

    let other_ws = tempfile::tempdir().unwrap();
    let other_english = other_ws.path().join("english");
    std::fs::create_dir_all(other_english.join("decks")).unwrap();
    write_initialized(&other_english.join("decks/z.md"), "## z\ny\n");
    std::fs::write(
        other_english.join(crate::workspace::MANIFEST),
        "title = \"Other English\"\n",
    )
    .unwrap();

    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[other_english], 1000);

    assert_eq!(
        Resolved::Ambiguous,
        resolve_row("english", dir.path(), &recent, &mut DeckCache::default())
    );
    assert_eq!(
        Resolved::One(ws.join("decks/a.md")),
        resolve_row(
            "english/a.md",
            dir.path(),
            &recent,
            &mut DeckCache::default()
        )
    );
}

#[test]
fn a_qualified_member_name_duplicated_across_two_same_named_containers_is_ambiguous() {
    // Both qualified keys collide too: must reject, not last-wins (dangerous
    // behind /api/reset's delete-by-path).
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    write_initialized(&ws.join("decks/a.md"), "## a\nb\n");
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();

    let other_ws = tempfile::tempdir().unwrap();
    let other_english = other_ws.path().join("english");
    std::fs::create_dir_all(other_english.join("decks")).unwrap();
    write_initialized(&other_english.join("decks/a.md"), "## z\ny\n");
    std::fs::write(
        other_english.join(crate::workspace::MANIFEST),
        "title = \"Other English\"\n",
    )
    .unwrap();

    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[other_english], 1000);

    assert_eq!(
        Resolved::Ambiguous,
        resolve_row("english", dir.path(), &recent, &mut DeckCache::default())
    );
    assert_eq!(
        Resolved::Ambiguous,
        resolve_row(
            "english/a.md",
            dir.path(),
            &recent,
            &mut DeckCache::default()
        )
    );
}

#[test]
fn resolve_row_reuses_a_shared_cache_instead_of_reparsing_on_a_second_call() {
    // A same-length, same-mtime rewrite to garbage carries no card marker
    // (see `parser::is_deck_content`), so a fresh reparse would drop the row;
    // resolving from the shared cache must keep the pre-rewrite answer.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("solo.md");
    write_initialized(&path, "## f <!-- id: card-s1 -->\nb\n");
    let recent = RecentDecks::load(dir.path().join("recent.json"));
    let mut cache = DeckCache::default();

    assert_eq!(
        Resolved::One(path.clone()),
        resolve_row("solo.md", dir.path(), &recent, &mut cache)
    );

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

    assert_eq!(
        Resolved::One(path),
        resolve_row("solo.md", dir.path(), &recent, &mut cache),
        "an unchanged (mtime, size) must resolve from the shared cache, not a fresh reparse"
    );
}

#[test]
fn a_drained_job_ignores_further_messages_without_replacing() {
    let dest = tempfile::tempdir().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();

    tx.send(Ok("## f\nb\n".to_string())).unwrap();
    let mut g = Generating {
        rx,
        url: "https://example.com/some-article".to_string(),
        dest: dest.path().to_path_buf(),
        started: Instant::now(),
        outcome: None,
    };

    g.poll();
    assert!(g.outcome.is_some());
    let files_after_poll_1: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
    assert_eq!(1, files_after_poll_1.len());

    tx.send(Ok("## other\nanswer\n".to_string())).unwrap();

    let first_outcome = g.outcome.clone();
    g.poll();
    assert_eq!(first_outcome, g.outcome, "outcome must stay unchanged");
    let files_after_poll_2: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
    assert_eq!(1, files_after_poll_2.len(), "still only one placed file");

    assert!(
        g.rx.try_recv().is_ok(),
        "guard short-circuited before draining the second message"
    );
}

#[test]
fn generation_rejects_invalid_math_before_creating_or_replacing_a_deck() {
    let dest = tempfile::tempdir().unwrap();
    let existing = dest.path().join("some-article.md");
    std::fs::write(&existing, "original bytes\n").unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(Ok("## q\n$\\frac{1$\n".to_string())).unwrap();
    let mut generating = Generating {
        rx,
        url: "https://example.com/some-article".to_string(),
        dest: dest.path().to_path_buf(),
        started: Instant::now(),
        outcome: None,
    };

    generating.poll();

    let outcome = generating.outcome.unwrap().unwrap_err();
    assert!(outcome.contains("invalid LaTeX math"), "{outcome}");
    assert_eq!(
        "original bytes\n",
        std::fs::read_to_string(existing).unwrap()
    );
    assert_eq!(1, std::fs::read_dir(dest.path()).unwrap().count());

    let empty = tempfile::tempdir().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(Ok("## q\n$\\frac{1$\n".to_string())).unwrap();
    let mut generating = Generating {
        rx,
        url: "https://example.com/new-deck".to_string(),
        dest: empty.path().to_path_buf(),
        started: Instant::now(),
        outcome: None,
    };
    generating.poll();
    assert!(generating.outcome.unwrap().is_err());
    assert_eq!(0, std::fs::read_dir(empty.path()).unwrap().count());
}

#[test]
fn generation_still_places_text_that_does_not_parse() {
    let dest = tempfile::tempdir().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(Ok("## missing answer\n".to_string())).unwrap();
    let mut generating = Generating {
        rx,
        url: "https://example.com/draft".to_string(),
        dest: dest.path().to_path_buf(),
        started: Instant::now(),
        outcome: None,
    };

    generating.poll();

    let outcome = generating.outcome.unwrap().unwrap_err();
    assert!(outcome.contains("saved draft.md, but it does not parse yet"));
    assert!(dest.path().join("draft.md").exists());
}

#[test]
fn the_zip_upload_cap_accepts_the_boundary_and_rejects_one_past_it() {
    const CAP: usize = 8;
    let at_cap = read_capped(&[7u8; CAP][..], CAP);
    assert_eq!(Some(CAP), at_cap.map(|b| b.len()));

    assert!(read_capped(&[7u8; CAP + 1][..], CAP).is_none());

    // An endless/lying-length reader is still capped at `cap + 1` bytes by
    // `take()`, never read to exhaustion.
    assert!(read_capped(std::io::repeat(7), CAP).is_none());
}

#[test]
fn resolve_dest_falls_back_to_decks_dir_and_rejects_unknown_names() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    write_initialized(&ws.join("a.md"), "## a\nb\n");
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        resolve_dest(None, dir.path(), &recent, &mut DeckCache::default()),
        Some(dir.path().to_path_buf())
    );
    assert_eq!(
        resolve_dest(Some(""), dir.path(), &recent, &mut DeckCache::default()),
        Some(dir.path().to_path_buf())
    );
    assert_eq!(
        resolve_dest(
            Some("english"),
            dir.path(),
            &recent,
            &mut DeckCache::default()
        ),
        Some(ws.clone())
    );
    assert_eq!(
        resolve_dest(
            Some("no-such-workspace"),
            dir.path(),
            &recent,
            &mut DeckCache::default()
        ),
        None
    );
    assert_eq!(
        resolve_dest(
            Some("../etc"),
            dir.path(),
            &recent,
            &mut DeckCache::default()
        ),
        None
    );
}

#[test]
fn resolve_dest_rejects_a_dir_name_duplicated_across_two_containers() {
    // Same collision class as resolve_row: picking either container silently
    // would be the same bug.
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    write_initialized(&ws.join("a.md"), "## a\nb\n");
    let elsewhere = tempfile::tempdir().unwrap();
    let other_english = elsewhere.path().join("english");
    std::fs::create_dir(&other_english).unwrap();
    write_initialized(&other_english.join("z.md"), "## z\ny\n");
    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[other_english], 1000);

    assert_eq!(
        resolve_dest(
            Some("english"),
            dir.path(),
            &recent,
            &mut DeckCache::default()
        ),
        None
    );
}

#[test]
fn workspace_members_fall_back_to_the_workspaces_own_store_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("animals");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Animals\"\n").unwrap();
    write_initialized(&ws.join("decks/one.md"), "## q1 <!-- id: card-qa -->\na1\n");

    // Saved to the workspace's own store, with nothing retained in memory: the
    // only way to see it is to open that store from disk.
    let paths = crate::workspace::deck_files(&ws);
    let ws_root = crate::workspace::store_path(&ws);
    let mut ws_store = crate::state::open_stores(&paths, &ws_root).unwrap();
    let now = now_ms();
    let id = Deck::load(ws.join("decks/one.md")).unwrap().cards[0]
        .id()
        .unwrap();
    ws_store.get_or_insert(&id, now).recognized_ms = Some(now);
    ws_store.save().unwrap();

    let recent = RecentDecks::load(dir.path().join("recent.json"));
    // Deliberately a different store that does not cover the workspace root.
    let global_store = Store::open(dir.path().join("global.json")).unwrap();
    let mut icons = HashMap::new();
    let dto = deck_catalog(
        dir.path(),
        &recent,
        &global_store,
        &HashMap::new(),
        true,
        &mut icons,
        ReviewConfig::default(),
        &mut DeckCache::default(),
    )
    .unwrap();

    let animals = dto
        .workspaces
        .iter()
        .find(|w| w.name == "animals")
        .expect("animals workspace row");
    let member = animals.members.first().expect("one member");
    assert_eq!(
        "started", member.state,
        "the workspace's own on-disk store must reach the member row: {member:?}"
    );
    assert!(
        !member.new_cards,
        "without that store the member reads as untouched: {member:?}"
    );
}

#[test]
fn workspace_members_prefer_a_retained_store_over_reopening_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("animals");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Animals\"\n").unwrap();
    write_initialized(&ws.join("decks/one.md"), "## q1 <!-- id: card-qa -->\na1\n");

    // Recognized only in memory, never saved: on-disk the card is untouched.
    // The owner's retained projection is authoritative, so a member built from
    // it must see this; one rebuilt by reopening the store from disk cannot.
    let paths = crate::workspace::deck_files(&ws);
    let ws_root = crate::workspace::store_path(&ws);
    let mut retained_store = crate::state::open_stores(&paths, &ws_root).unwrap();
    let now = now_ms();
    let id = Deck::load(ws.join("decks/one.md")).unwrap().cards[0]
        .id()
        .unwrap();
    retained_store.get_or_insert(&id, now).recognized_ms = Some(now);

    let mut retained: HashMap<PathBuf, Arc<Store>> = HashMap::new();
    retained.insert(ws_root, Arc::new(retained_store));

    let recent = RecentDecks::load(dir.path().join("recent.json"));
    let global_store = Store::open(dir.path().join("global.json")).unwrap();
    let mut icons = HashMap::new();
    let dto = deck_catalog(
        dir.path(),
        &recent,
        &global_store,
        &retained,
        true,
        &mut icons,
        ReviewConfig::default(),
        &mut DeckCache::default(),
    )
    .unwrap();

    let animals = dto
        .workspaces
        .iter()
        .find(|w| w.name == "animals")
        .expect("animals workspace row");
    let member = animals.members.first().expect("one member");
    assert_eq!(
        "started", member.state,
        "the owner's unflushed recognition must reach the member row: {member:?}"
    );
    assert!(
        !member.new_cards,
        "reopening the store from disk would miss the unsaved entry: {member:?}"
    );
}

#[test]
fn a_group_row_aggregates_member_reviewability_instead_of_hardcoding_true() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("animals");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Animals\"\n").unwrap();
    write_initialized(&ws.join("decks/one.md"), "## q1 <!-- id: card-qa -->\na1\n");
    write_initialized(&ws.join("decks/two.md"), "## q2 <!-- id: card-qb -->\na2\n");

    let paths = crate::workspace::deck_files(&ws);
    let mut ws_store =
        crate::state::open_stores(&paths, &crate::workspace::store_path(&ws)).unwrap();
    let now = now_ms();
    for name in ["one.md", "two.md"] {
        let deck = Deck::load(ws.join("decks").join(name)).unwrap();
        let id = deck.cards[0].id().unwrap();
        let future = crate::store::FsrsState {
            state: 2,
            scheduled_days: 30,
            due_ms: now + 30 * 86_400_000,
            ..Default::default()
        };
        let entry = ws_store.get_or_insert(&id, now);
        entry.recognized_ms = Some(now);
        entry.recall = Some(future);
        entry.reconstruct = Some(future);
    }
    let one_token = Deck::load(ws.join("decks/one.md"))
        .unwrap()
        .deck_token
        .unwrap();
    ws_store.set_last_depth(&one_token, crate::depth::Depth::Reconstruct);
    ws_store.save().unwrap();

    let recent = RecentDecks::load(dir.path().join("recent.json"));
    // Irrelevant to a workspace group row: workspace_members always reads the
    // workspace's own store from disk, never this one.
    let global_store = Store::open(dir.path().join("global.json")).unwrap();
    let mut icons = HashMap::new();
    let dto = deck_catalog(
        dir.path(),
        &recent,
        &global_store,
        &HashMap::new(),
        true,
        &mut icons,
        ReviewConfig::default(),
        &mut DeckCache::default(),
    )
    .unwrap();

    let animals = dto
        .workspaces
        .iter()
        .find(|w| w.name == "animals")
        .expect("animals workspace row");
    assert!(!animals.selectable, "row: {animals:?}");
    assert!(!animals.reviewable, "row: {animals:?}");
    assert!(!animals.reviewable_recognize, "row: {animals:?}");
    assert!(!animals.reviewable_recall, "row: {animals:?}");
    assert!(!animals.reviewable_reconstruct, "row: {animals:?}");
    assert_eq!(2, animals.members.len(), "row: {animals:?}");
    for m in &animals.members {
        assert!(m.selectable, "member {} should stay selectable", m.name);
    }
    let one = animals
        .members
        .iter()
        .find(|m| m.name.contains("one"))
        .expect("member one");
    assert_eq!(
        "reconstruct", one.last_depth,
        "a member row reads the workspace store's recorded depth, not the default"
    );
    let two = animals
        .members
        .iter()
        .find(|m| m.name.contains("two"))
        .expect("member two");
    assert_eq!(
        "recall", two.last_depth,
        "no recorded depth and nothing recognizable falls to the recall default"
    );
}

#[test]
fn a_plain_folders_member_badge_reads_the_served_instance_store_not_the_global_default() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("letters");
    std::fs::create_dir(&folder).unwrap();
    write_initialized(&folder.join("a.md"), "## q <!-- id: card-qa -->\na\n");

    let mut instance_store = Store::open(dir.path().join("instance.json")).unwrap();
    let deck = Deck::load(folder.join("a.md")).unwrap();
    let id = deck.cards[0].id().unwrap();
    let now = now_ms();
    let future = crate::store::FsrsState {
        state: 2,
        scheduled_days: 30,
        due_ms: now + 30 * 86_400_000,
        ..Default::default()
    };
    let entry = instance_store.get_or_insert(&id, now);
    entry.recognized_ms = Some(now);
    entry.recall = Some(future);
    entry.reconstruct = Some(future);
    instance_store.save().unwrap();

    let recent = RecentDecks::load(dir.path().join("recent.json"));
    let mut icons = HashMap::new();
    let dto = deck_catalog(
        dir.path(),
        &recent,
        &instance_store,
        &HashMap::new(),
        true,
        &mut icons,
        ReviewConfig::default(),
        &mut DeckCache::default(),
    )
    .unwrap();

    let letters = dto
        .folders
        .iter()
        .find(|f| f.name == "letters")
        .expect("letters folder row");
    assert_eq!(1, letters.members.len(), "row: {letters:?}");
    let member = &letters.members[0];
    assert!(
        !member.reviewable,
        "member badge must reflect the seeded instance store, not an empty \
         global default: {member:?}"
    );
}

#[test]
fn a_deck_that_fails_to_load_reports_nothing_reviewable_but_stays_selectable() {
    let dir = tempfile::tempdir().unwrap();
    // An unclosed cloze hole fails to parse: `Deck::load` errors.
    write_initialized(
        &dir.path().join("broken.md"),
        "## front\nbad \\blank{oops\n",
    );
    let recent = RecentDecks::load(dir.path().join("recent.json"));
    let entry = picker::catalog(dir.path(), &recent, &mut DeckCache::default())
        .unwrap()
        .into_iter()
        .find(|e| e.name == "broken.md")
        .expect("catalog lists the broken deck file even though it won't parse");
    assert!(
        Deck::load(&entry.path).is_err(),
        "fixture must actually fail to load"
    );

    let store = Store::open(dir.path().join("progress/deck1.json")).unwrap();
    let augment = AugmentCache::open_for_workspace(dir.path()).unwrap();
    let dto = deck_item_dto(
        &entry,
        &store,
        dir.path(),
        true,
        &augment,
        ReviewConfig::default(),
        &mut DeckCache::default(),
    );

    assert!(dto.selectable, "row: {dto:?}");
    assert!(!dto.reviewable, "row: {dto:?}");
    assert!(!dto.reviewable_recognize, "row: {dto:?}");
    assert!(!dto.reviewable_recall, "row: {dto:?}");
    assert!(!dto.reviewable_reconstruct, "row: {dto:?}");
}

#[test]
fn browse_payload_select_phase_has_no_cards() {
    let dto = browse_payload(None);
    assert_eq!(dto.phase, "select");
    assert!(dto.cards.is_empty());
}

#[test]
fn browse_payload_renders_a_repeated_formula_once() {
    let card = Card::plain(
        Arc::from("s.md"),
        "$x^2$".to_string(),
        vec!["$x^2$".to_string()],
        Some("$x^2$".to_string()),
        1,
    );
    let browsing = Browsing {
        cards: vec![card.clone(), card],
        label: "math".to_string(),
        images: HashMap::new(),
    };
    let before = crate::math::thread_render_count();
    let dto = browse_payload(Some(&browsing));
    assert_eq!(dto.cards.len(), 2);
    assert_eq!(crate::math::thread_render_count() - before, 1);
}

#[test]
#[ignore = "manual release-build math payload measurement"]
fn measure_math_state_and_browse_payloads() {
    let fixtures = [
        r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        r"\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}",
        r"\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}",
        r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
        r"\alpha_i^2 + \beta_j",
        r"\lim_{x \to 0} \frac{\sin x}{x} = 1",
        r"\nabla \times \mathbf{E} = -\frac{\partial \mathbf{B}}{\partial t}",
    ];
    let front = fixtures
        .iter()
        .map(|source| format!("${source}$"))
        .collect::<Vec<_>>()
        .join(" ");
    let current = Card::plain(
        Arc::from("math.md"),
        front,
        vec!["answer".to_string()],
        None,
        1,
    );
    let before = crate::math::thread_render_count();
    let started = Instant::now();
    let view = crate::review::CardView::from(&current);
    let projection = started.elapsed();
    let started = Instant::now();
    let current_bytes = serde_json::to_vec(&view).unwrap().len();
    let serialization = started.elapsed();
    assert_eq!(crate::math::thread_render_count() - before, fixtures.len());
    eprintln!(
        "current formulas={} projection_us={} serialization_us={} bytes={current_bytes}",
        fixtures.len(),
        projection.as_micros(),
        serialization.as_micros()
    );

    let cards = (0..500)
        .map(|index| {
            let formula = format!("$x_{{{index}}}^2$");
            Card::plain(
                Arc::from("math.md"),
                formula.clone(),
                vec![formula],
                None,
                index + 1,
            )
        })
        .collect();
    let browsing = Browsing {
        cards,
        label: "500 math cards".to_string(),
        images: HashMap::new(),
    };
    let before = crate::math::thread_render_count();
    let started = Instant::now();
    let dto = browse_payload(Some(&browsing));
    let projection = started.elapsed();
    let started = Instant::now();
    let browse_bytes = serde_json::to_vec(&dto).unwrap().len();
    let serialization = started.elapsed();
    assert_eq!(crate::math::thread_render_count() - before, 500);
    eprintln!(
        "browse cards=500 formulas=1000 unique=500 projection_us={} serialization_us={} bytes={browse_bytes}",
        projection.as_micros(),
        serialization.as_micros()
    );
}

#[test]
fn review_state_select_phase_has_no_card() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("p.json")).unwrap();
    let dto = review_state(None, &store, None, 0);
    assert_eq!(dto.phase, "select");
    assert_eq!(dto.kind, "review");
    assert!(dto.card.is_none());
    // `done` is the session-end signal; `finished` is deliberately absent
    // from the wire contract.
    let json = serde_json::to_value(&dto).unwrap();
    assert!(json.get("finished").is_none());
}

#[test]
fn finished_review_uses_the_done_phase_not_a_finished_flag() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, _card, _deck) = one_card_reviewing(dir.path());
    let mut store = Store::open(dir.path().join("graded.json")).unwrap();
    r.session.grade(&mut store, Grade::Pass, now_ms());
    assert!(r.session.is_finished());
    let dto = review_state(Some(&r), &store, None, 0);
    assert_eq!(dto.phase, "done");
    assert_eq!(dto.kind, "review");
}

fn reviewing_at(deck: PathBuf, cards: Vec<Card>, store: &mut Store, depth: Depth) -> Reviewing {
    let session = Session::new(
        cards,
        store,
        Box::new(Fsrs::default()),
        crate::session::SessionOptions {
            depth,
            ..Default::default()
        },
        now_ms(),
    );
    let augment = crate::augment::AugmentCache::open(deck.with_extension("generated.json"));
    // Routed by the deck's own id (empty for a fixture with no
    // frontmatter), matching what its parsed cards carry as `deck_id`.
    let deck_id = crate::deck::Deck::load(&deck)
        .ok()
        .and_then(|d| d.deck_token)
        .unwrap_or_default();
    let mut decks = HashMap::new();
    decks.insert(deck_id, deck);
    Reviewing::new(SessionBuild {
        session,
        label: "d.md".to_string(),
        decks,
        links: HashMap::new(),
        source_layers: HashMap::new(),
        base_roots: HashMap::new(),
        source_bases: HashMap::new(),
        topology_name: None,
        augment,
    })
}

#[test]
fn state_reports_the_sessions_depth_and_typeline_mode() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## steps <!-- reveal: line --> <!-- id: card-q1 -->\nfirst\nsecond\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.md", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&cards[0].id().unwrap(), 0);
    let r = reviewing_at(deck, cards, &mut store, Depth::Reconstruct);

    let dto = review_state(Some(&r), &store, None, 0);
    assert_eq!(
        "reconstruct", dto.depth,
        "the DTO reports the session's depth"
    );
    assert_eq!(
        "typeline", dto.mode,
        "reconstruct + `reveal: line` types the next line"
    );
}

#[test]
fn explain_state_serves_the_keypoints_rubric_cached_or_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## why <!-- id: card-q1 -->\nfirst fact\nsecond fact\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.md", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&cards[0].id().unwrap(), 0);
    let mut r = reviewing_at(deck, cards.clone(), &mut store, Depth::Reconstruct);

    let fallback = review_state(Some(&r), &store, None, 0);
    assert_eq!(fallback.mode, "explain");
    assert_eq!(
        fallback.keypoints,
        Some(vec!["first fact".to_string(), "second fact".to_string()])
    );

    r.augment.set_keypoints(
        &cards[0].id().unwrap(),
        vec!["one claim".to_string()],
        cards[0].content_fingerprint,
    );
    let cached = review_state(Some(&r), &store, None, 0);
    assert_eq!(cached.keypoints, Some(vec!["one claim".to_string()]));
}

#[test]
fn recognize_state_offers_gap_options_for_a_cloze_card() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## where <!-- id: card-q1 -->\nThe \\blank{cat} sat here\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.md", text).unwrap();
    assert_eq!(vec!["cat".to_string()], cards[0].back);
    let id = cards[0].id().unwrap();
    let fingerprint = cards[0].content_fingerprint;
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&id, 0);
    let mut r = reviewing_at(deck, cards, &mut store, Depth::Recognize);
    r.augment.set_distractors(
        &id,
        vec!["dog".to_string(), "fish".to_string(), "bird".to_string()],
        fingerprint,
    );

    let dto = review_state(Some(&r), &store, None, 0);
    let opts = dto
        .choices
        .expect("a Recognize cloze card offers gap-filler options");
    assert_eq!(choice::NUM_OPTIONS, opts.len());
    assert!(
        opts.contains(&"cat".to_string()),
        "the gap text is an option"
    );
}

#[test]
fn recognize_state_quizzes_a_line_card_on_the_whole_sequence_not_a_single_step() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## steps <!-- reveal: line --> <!-- id: card-q1 -->\nfirst\nsecond\nthird\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.md", text).unwrap();
    let id = cards[0].id().unwrap();
    let fingerprint = cards[0].content_fingerprint;
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&id, 0);
    let mut r = reviewing_at(deck, cards, &mut store, Depth::Recognize);
    r.augment.set_distractors(
        &id,
        vec![
            "second\nfirst\nthird".to_string(),
            "third\nsecond\nfirst".to_string(),
            "first\nthird\nsecond".to_string(),
        ],
        fingerprint,
    );

    let dto = review_state(Some(&r), &store, None, 0);
    let opts = dto
        .choices
        .expect("cached whole-sequence distractors offer options");
    assert_eq!(choice::NUM_OPTIONS, opts.len());
    assert!(
        opts.contains(&"first\nsecond\nthird".to_string()),
        "the correct option is the whole back joined, matching `choice::build`'s answer_text"
    );
    for opt in &opts {
        assert!(
            opt.contains('\n'),
            "option {opt:?} is a single step, not a whole sequence"
        );
    }
}

#[test]
fn recognize_state_offers_no_choices_for_a_line_card_with_no_cached_distractors() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## steps <!-- reveal: line --> <!-- id: card-q1 -->\nfirst\nsecond\nthird\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.md", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&cards[0].id().unwrap(), 0);
    let r = reviewing_at(deck, cards, &mut store, Depth::Recognize);

    let dto = review_state(Some(&r), &store, None, 0);
    assert!(
        dto.choices.is_none(),
        "no cached distractors and no offline pool → the fallback signal"
    );
}

#[test]
fn recognize_state_reshuffles_choice_options_on_the_next_appearance_but_not_mid_poll() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## q <!-- id: card-q1 -->\nanswer\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.md", text).unwrap();
    let id = cards[0].id().unwrap();
    let fingerprint = cards[0].content_fingerprint;
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&id, 0);
    let mut r = reviewing_at(deck, cards, &mut store, Depth::Recognize);
    r.augment.set_distractors(
        &id,
        vec![
            "wrong one".to_string(),
            "wrong two".to_string(),
            "wrong three".to_string(),
        ],
        fingerprint,
    );

    let first = review_state(Some(&r), &store, None, 0)
        .choices
        .expect("a valid MC from the 3 cached AI distractors");
    let second = review_state(Some(&r), &store, None, 0)
        .choices
        .expect("still the same appearance");
    assert_eq!(first, second, "an idle poll must not reshuffle mid-answer");

    let mut now = now_ms();
    let mut saw_a_different_order = false;
    for _ in 0..5 {
        r.session.grade(&mut store, Grade::Fail, now);
        assert!(
            r.session.is_finished(),
            "the only card floors instead of resurfacing instantly"
        );
        now += crate::scheduler::DEFAULT_ACQUIRE_COOLDOWN_MS;
        r.session.poll(&mut store, now);
        assert_eq!(
            Some(id.clone()),
            r.session.current().and_then(|c| c.id()),
            "past the floor, the card returns"
        );
        let later = review_state(Some(&r), &store, None, 0)
            .choices
            .expect("the next appearance still offers the MC");
        if later != first {
            saw_a_different_order = true;
        }
    }
    assert!(
        saw_a_different_order,
        "no later appearance ever varied the option order"
    );
}

#[test]
fn an_already_recognized_card_skips_the_acquire_mc() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## q <!-- id: card-q1 -->\nanswer\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.md", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    let state = store.get_or_insert(&cards[0].id().unwrap(), 0);
    state.recognized_ms = Some(500);
    let r = reviewing_at(deck, cards, &mut store, Depth::Recall);

    let dto = review_state(Some(&r), &store, None, 0);
    assert!(!dto.acquire, "a recognized card isn't acquired cold");
    assert!(
        dto.choices.is_none(),
        "no recognition MC for an already-recognized card"
    );
    assert_eq!("recall", dto.depth);
}

#[test]
fn grade_names_map_to_grades() {
    assert!(matches!(Grade::Fail, Grade::Fail));
    assert_eq!(mode_name(Mode::LineByLine), "line");
    assert_eq!(mode_name(Mode::Flip), "flip");
    assert_eq!(mode_name(Mode::Explain), "explain");
}

#[test]
fn input_name_matches_clap_value_names() {
    assert_eq!(input_name(Input::Type), "type");
    assert_eq!(input_name(Input::Draw), "draw");
}

fn one_card_reviewing(dir: &Path) -> (Reviewing, Card, PathBuf) {
    let deck = dir.join("d.md");
    std::fs::write(&deck, "## front <!-- id: card-q1 -->\nback\n").unwrap();
    let mut store = Store::open(dir.join("p.json")).unwrap();
    let mut card = Card::plain(
        Arc::from("d.md"),
        "front".to_string(),
        vec!["back".to_string()],
        None,
        1,
    );
    card.token = Some(Arc::from("card-q1"));
    // Deliberately not the filename: proves the routing below keys on
    // deck_id, not on `card.subject`.
    card.deck_id = Arc::from("one-card-deck");
    let session = Session::new(
        vec![card.clone()],
        &mut store,
        Box::new(Fsrs::default()),
        crate::session::SessionOptions::default(),
        now_ms(),
    );
    let mut decks = HashMap::new();
    decks.insert("one-card-deck".to_string(), deck.clone());
    let reviewing = Reviewing::new(SessionBuild {
        session,
        label: "d.md".to_string(),
        decks,
        links: HashMap::new(),
        source_layers: HashMap::new(),
        base_roots: HashMap::new(),
        source_bases: HashMap::new(),
        topology_name: None,
        augment: crate::augment::AugmentCache::open(deck.with_extension("generated.json")),
    });
    (reviewing, card, deck)
}

#[test]
fn rotating_question_variants_preserves_and_returns_to_the_authored_front() {
    let dir = tempfile::tempdir().unwrap();
    let (mut reviewing, card, _deck) = one_card_reviewing(dir.path());
    let card_id = card.id().unwrap();
    reviewing.augment.set_variants(
        &card_id,
        vec!["a different question".to_string()],
        card.content_fingerprint,
    );
    reviewing.present_seq = 1;

    reviewing.rotate_variant();
    assert_eq!(
        "a different question",
        reviewing.session.current().unwrap().front
    );
    assert_eq!(
        Some("front"),
        reviewing.original_fronts.get(&card_id).map(String::as_str)
    );

    reviewing.rotate_variant();
    assert_eq!("front", reviewing.session.current().unwrap().front);
}

#[test]
fn poll_ask_records_answer_in_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, card, _deck) = one_card_reviewing(dir.path());
    let (tx, rx) = std::sync::mpsc::channel();
    r.ask.subject = card.id();
    r.ask.pending = Some(Pending {
        rx,
        job: ask::AskJob::default(),
        purpose: Purpose::Question("why is s1 invalid?".to_string()),
        card,
    });
    assert_eq!((None, None), r.poll_ask());
    assert!(r.ask_dto(None, None).thinking);

    tx.send(Reply::Answer("because ownership moved".to_string()))
        .unwrap();
    assert_eq!((None, None), r.poll_ask());
    assert!(r.ask.pending.is_none());
    assert_eq!(1, r.ask.transcript.len());
    assert_eq!("why is s1 invalid?", r.ask.transcript[0].0);
    assert_eq!("because ownership moved", r.ask.transcript[0].1);
    assert!(r.ask.cli.started); // later questions --resume
}

#[test]
fn ask_transcript_resets_when_the_card_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, card, _deck) = one_card_reviewing(dir.path());
    r.ask
        .transcript
        .push(("old q".to_string(), "old a".to_string()));
    r.ask.subject = Some("a-different-card-id".to_string());
    r.ask.cli.started = true;

    r.align_transcript();

    // Cleared and re-tagged, but the underlying Claude session (cli.started)
    // survives.
    assert!(r.ask.transcript.is_empty());
    assert_eq!(card.id(), r.ask.subject);
    assert!(r.ask.cli.started);
}

#[test]
fn poll_ask_condense_appends_note_to_deck_and_live_card() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, card, deck) = one_card_reviewing(dir.path());
    r.ask.transcript.push(("q".to_string(), "a".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    r.ask.subject = card.id();
    r.ask.pending = Some(Pending {
        rx,
        job: ask::AskJob::default(),
        purpose: Purpose::Condense,
        card,
    });
    tx.send(Reply::Answer("- key insight to reread".to_string()))
        .unwrap();
    let (status, error) = r.poll_ask();
    assert_eq!(Some("note saved".to_string()), status);
    assert!(error.is_none());
    let text = std::fs::read_to_string(&deck).unwrap();
    assert!(text.contains("key insight to reread"), "deck:\n{text}");
    assert!(
        r.session
            .current()
            .and_then(|current| current.note.as_deref())
            .is_some_and(|note| note.contains("key insight to reread"))
    );
}

#[test]
fn poll_ask_error_resets_session() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, card, _deck) = one_card_reviewing(dir.path());
    r.ask.cli.started = true;
    let (tx, rx) = std::sync::mpsc::channel();
    r.ask.subject = card.id();
    r.ask.pending = Some(Pending {
        rx,
        job: ask::AskJob::default(),
        purpose: Purpose::Question("q".to_string()),
        card,
    });
    tx.send(Reply::Error("not logged in".to_string())).unwrap();
    let (status, error) = r.poll_ask();
    assert_eq!(Some("not logged in".to_string()), error);
    assert!(status.is_none());
    assert!(r.ask.pending.is_none());
    assert!(!r.ask.cli.started); // a fresh session next time
    assert!(r.ask.transcript.is_empty());
}

#[cfg(unix)]
#[test]
fn a_frozen_card_without_source_context_warns_and_still_uses_the_tutor() {
    let _lock = crate::testutil::exec_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alix.toml"), "").unwrap();
    std::fs::create_dir(dir.path().join("decks")).unwrap();
    std::fs::write(dir.path().join("29.rs"), "fn real() {}\n").unwrap();
    let deck_path = dir.path().join("decks/d.md");
    std::fs::write(
        &deck_path,
        "---\nformat-version: 1\nid: \"deck-frozendeck1\"\nsource: 29.rs\n---\n## q\na\n\
         <!-- at: 29.rs:1 -->\n",
    )
    .unwrap();
    crate::assets::freeze_member(&deck_path).unwrap();
    // Stamped as in production: an unstamped card has no token and is never
    // servable.
    crate::stamp::stamp_deck(&deck_path).unwrap();
    let deck = crate::deck::Deck::load(&deck_path).unwrap();
    let card = deck.cards[0].clone();
    assert!(card.citations[0].asset.is_some(), "the card is frozen");

    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    let session = Session::new(
        vec![card.clone()],
        &mut store,
        Box::new(Fsrs::default()),
        crate::session::SessionOptions::default(),
        now_ms(),
    );
    let mut decks = HashMap::new();
    decks.insert("deck-frozendeck1".to_string(), deck_path);
    let mut base_roots = HashMap::new();
    // Configured (`source_access` opted in), but unresolved on disk.
    base_roots.insert(
        "deck-frozendeck1".to_string(),
        dir.path().join("gone-source"),
    );
    let mut source_bases = HashMap::new();
    source_bases.insert("deck-frozendeck1".to_string(), SourceBase::for_deck(&deck));
    let mut r = Reviewing::new(SessionBuild {
        session,
        label: "d.md".to_string(),
        decks,
        links: HashMap::new(),
        source_layers: HashMap::new(),
        base_roots,
        source_bases,
        topology_name: None,
        augment: crate::augment::AugmentCache::open(dir.path().join("a.generated.json")),
    });

    let cli = crate::testutil::fake_reply(dir.path(), "from the frozen excerpt");
    let cfg = crate::testutil::ask_config(&cli);
    assert!(r.start_ask(
        &cfg,
        Audience::Adult,
        AskAction::Question("why?".to_string())
    ));

    let dto = r.ask_dto(None, None);
    assert!(dto.thinking);
    assert_eq!(Some(ask::FROZEN_ONLY_WARNING.to_string()), dto.status);
    for _ in 0..500 {
        let (_, error) = r.poll_ask();
        assert!(error.is_none(), "{error:?}");
        if !r.ask_dto(None, None).thinking {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        vec![("why?".to_string(), "from the frozen excerpt".to_string())],
        r.ask.transcript
    );
    assert_eq!(
        Some(ask::FROZEN_ONLY_WARNING.to_string()),
        r.ask_dto(None, None).status
    );
}

#[cfg(unix)]
#[test]
fn a_frozen_review_card_with_a_live_local_source_needs_no_fallback_warning() {
    let _lock = crate::testutil::exec_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alix.toml"), "").unwrap();
    std::fs::create_dir(dir.path().join("decks")).unwrap();
    std::fs::write(dir.path().join("source.rs"), "fn live() {}\n").unwrap();
    let deck_path = dir.path().join("decks/d.md");
    std::fs::write(
        &deck_path,
        "---\nformat-version: 1\nid: \"deck-livefrozen1\"\nsource: source.rs\n---\n\
         ## q <!-- id: card-livefrozen1 -->\na\n<!-- at: source.rs:1 -->\n",
    )
    .unwrap();
    crate::assets::freeze_member(&deck_path).unwrap();
    crate::stamp::stamp_deck(&deck_path).unwrap();
    let deck = Deck::load(&deck_path).unwrap();
    let card = deck.cards[0].clone();
    assert!(card.citations[0].asset.is_some());

    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    let session = Session::new(
        vec![card],
        &mut store,
        Box::new(Fsrs::default()),
        crate::session::SessionOptions::default(),
        now_ms(),
    );
    let deck_id = "deck-livefrozen1".to_string();
    let mut decks = HashMap::new();
    decks.insert(deck_id.clone(), deck_path);
    let mut base_roots = HashMap::new();
    base_roots.insert(deck_id.clone(), dir.path().to_path_buf());
    let mut source_bases = HashMap::new();
    source_bases.insert(deck_id, SourceBase::for_deck(&deck));
    let mut reviewing = Reviewing::new(SessionBuild {
        session,
        label: "d.md".to_string(),
        decks,
        links: HashMap::new(),
        source_layers: HashMap::new(),
        base_roots,
        source_bases,
        topology_name: None,
        augment: AugmentCache::open(dir.path().join("augment.json")),
    });
    let cli = crate::testutil::fake_reply(dir.path(), "answer");
    let cfg = crate::testutil::ask_config(&cli);

    assert!(reviewing.start_ask(
        &cfg,
        Audience::Adult,
        AskAction::Question("why?".to_string())
    ));
    assert_eq!(None, reviewing.ask_dto(None, None).status);
}

#[test]
fn poll_ask_draft_surfaces_a_parsed_card() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, card, _deck) = one_card_reviewing(dir.path());
    r.ask.transcript.push(("q".to_string(), "a".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    r.ask.subject = card.id();
    r.ask.pending = Some(Pending {
        rx,
        job: ask::AskJob::default(),
        purpose: Purpose::DraftCard,
        card,
    });
    tx.send(Reply::Answer("## term?\ndefinition\n".to_string()))
        .unwrap();
    let (status, error) = r.poll_ask();
    assert_eq!(Some("card drafted".to_string()), status);
    assert!(error.is_none());
    let draft = r
        .ask_dto(None, None)
        .draft
        .expect("a draft should be surfaced");
    assert_eq!("term?", draft.front);
    assert_eq!(vec!["definition".to_string()], draft.back);
}

#[test]
fn exam_due_reports_the_decks_name_not_its_routing_id() {
    let dir = tempfile::tempdir().unwrap();
    let deck_path = dir.path().join("d.md");
    // The deck id deliberately differs from the filename: `r.files.paths` is
    // keyed by it, but the wire value must stay the resolvable deck name.
    std::fs::write(
        &deck_path,
        "---\nformat-version: 1\nid: \"deck-examduedeck\"\nsource: https://x\n---\n## a <!-- id: card-q1 -->\n1\n",
    )
    .unwrap();
    let deck = crate::deck::Deck::load(&deck_path).unwrap();
    let card = deck.cards[0].clone();
    assert_eq!("deck-examduedeck", card.deck_id.as_ref());

    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    let id = card.id().unwrap();
    // A year-out review: graduated and long past due, so the deck reads as
    // exam-due rather than merely started.
    store.get_or_insert(&id, 0).recall = Some(crate::store::FsrsState {
        state: 2,
        scheduled_days: 100_000,
        ..Default::default()
    });

    let r = reviewing_at(deck_path, vec![card], &mut store, Depth::Recall);
    let dto = review_state(Some(&r), &store, None, 0);
    assert_eq!("done", dto.phase, "expected a finished session: {dto:?}");
    assert_eq!(vec!["d.md".to_string()], dto.exam_due);
}

fn walk_deck(dir: &Path) -> crate::trace::Trace {
    std::fs::write(dir.join("source.txt"), "first\nsecond\nthird\n").unwrap();
    let path = dir.join("t.md");
    std::fs::write(
        &path,
        "---\ntrace: how `it` works\nsource: source.txt\n---\n\
         ## Predict the `first` hop <!-- id: card-t1 -->\n\
         <!-- given: line — the `input` line -->\n\
         it reads the `first` line\n\
         > call `read`\n\
         <!-- at: 1 -->\n\
         ## Predict the second hop <!-- id: card-t2 -->\n\
         it reads line two\n\
         <!-- at: 2 -->\n",
    )
    .unwrap();
    crate::source::stamp_citations(&path).unwrap();
    crate::trace::Trace::from_deck(&Deck::load(&path).unwrap()).unwrap()
}

#[test]
fn walk_dto_tracks_phase_excerpt_and_rail() {
    let dir = tempfile::tempdir().unwrap();
    let trace = walk_deck(dir.path());
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    let walk = Walk::new(trace);
    let mut w = Walking::new(walk, None);

    let d = walk_dto(&w);
    assert_eq!("walk", d.kind);
    assert_eq!("predict", d.phase);
    assert_eq!(1, d.current);
    assert_eq!(2, d.total);
    assert_eq!(Some("Predict the `first` hop".to_string()), d.prompt);
    assert_eq!(vec!["line — the `input` line".to_string()], d.givens);
    assert!(
        d.description_runs
            .iter()
            .any(|run| run.code && run.text == "it")
    );
    assert!(
        d.prompt_runs
            .as_ref()
            .is_some_and(|runs| runs.iter().any(|run| run.code && run.text == "first"))
    );
    assert!(
        d.given_runs[0]
            .iter()
            .any(|run| run.code && run.text == "input")
    );
    assert!(d.excerpt.is_none());
    assert!(!d.auto_grade);
    assert!(d.path[0].current && d.path[0].delta.is_none());

    w.walk.predict("my guess".to_string());
    let d = walk_dto(&w);
    assert_eq!("reveal", d.phase);
    assert_eq!(Some("my guess".to_string()), d.prediction);
    let ex = d.excerpt.expect("reveal reads the source");
    assert_eq!(
        vec![(1, "first".to_string())],
        ex.lines
            .iter()
            .map(|l| (l.n, l.text.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(vec!["it reads the `first` line".to_string()], d.points);
    assert!(
        d.point_runs[0]
            .iter()
            .any(|run| run.code && run.text == "first")
    );
    assert!(
        d.note_runs
            .as_ref()
            .is_some_and(|runs| runs.iter().any(|run| run.code && run.text == "read"))
    );

    w.walk.grade(&mut store, Delta::Passed, 1000);
    let d = walk_dto(&w);
    assert_eq!("predict", d.phase);
    assert_eq!(2, d.current);
    assert_eq!(Some("passed"), d.path[0].delta);
    assert!(d.path[1].current);

    w.walk.predict(String::new());
    w.walk.grade(&mut store, Delta::Failed, 1001);
    let d = walk_dto(&w);
    assert_eq!("done", d.phase);
    let s = d.summary.expect("done has a summary");
    assert_eq!((1, 0, 1), (s.passed, s.partly, s.failed));
    assert_eq!(vec![2], s.weak); // 1-based: the failed second hop
}

#[test]
fn walk_dto_surfaces_a_live_grade_and_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let trace = walk_deck(dir.path());
    let walk = Walk::new(trace);
    let mut w = Walking::new(walk, Some(AskConfig::default()));

    w.walk.predict("g".to_string());
    w.grade_result = Some((Delta::Partial, "right idea, missed a detail".to_string()));
    let d = walk_dto(&w);
    assert!(d.auto_grade);
    assert_eq!(Some("partly"), d.verdict); // machine token, not a display label
    assert_eq!(Some("right idea, missed a detail".to_string()), d.feedback);

    w.clear_grade();
    let d = walk_dto(&w);
    assert!(d.verdict.is_none() && d.feedback.is_none() && !d.thinking);
}

#[test]
fn walk_ask_condense_appends_a_note_to_the_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let trace = walk_deck(dir.path());
    let deck_path = trace.deck_path.clone();
    let walk = Walk::new(trace);
    let mut w = Walking::new(walk, None);
    w.walk.predict("guess".to_string());

    let card = w.checkpoint_card().expect("a checkpoint card");
    assert_eq!(
        Some("Source excerpt:\n1: first\n\ncall `read`"),
        card.note.as_deref()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    w.ask.subject = w.walk.checkpoint().map(|c| c.card_id.clone());
    w.ask.pending = Some(Pending {
        rx,
        job: ask::AskJob::default(),
        purpose: Purpose::Condense,
        card,
    });
    tx.send(Reply::Answer(
        "- the read lock is released first".to_string(),
    ))
    .unwrap();

    let (status, error) = w.poll_ask();
    assert_eq!(Some("note saved".to_string()), status);
    assert!(error.is_none());
    let text = std::fs::read_to_string(&deck_path).unwrap();
    assert!(
        text.contains("the read lock is released first"),
        "deck:\n{text}"
    );
}

#[cfg(unix)]
#[test]
fn a_frozen_walk_checkpoint_with_a_live_local_source_needs_no_fallback_warning() {
    let _lock = crate::testutil::exec_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alix.toml"), "").unwrap();
    let initial = walk_deck(dir.path());
    std::fs::create_dir(dir.path().join("decks")).unwrap();
    let deck_path = dir.path().join("decks/t.md");
    std::fs::rename(&initial.deck_path, &deck_path).unwrap();
    crate::stamp::stamp_deck(&deck_path).unwrap();
    crate::assets::freeze_member(&deck_path).unwrap();
    let trace = crate::trace::Trace::from_deck(&Deck::load(&deck_path).unwrap()).unwrap();
    assert!(trace.base_root.as_deref().is_some_and(Path::exists));
    assert!(
        trace
            .checkpoints
            .first()
            .is_some_and(|checkpoint| trace.frozen_block(checkpoint).is_some())
    );
    let mut walking = Walking::new(Walk::new(trace), None);
    let cli = crate::testutil::fake_reply(dir.path(), "answer");
    let mut cfg = crate::testutil::ask_config(&cli);
    cfg.source_access = true;

    assert!(walking.start_ask(&cfg, Audience::Adult, Some("why?".to_string())));
    assert_eq!(None, walking.ask_dto(None, None).status);
}

#[cfg(unix)]
#[test]
fn a_frozen_walk_checkpoint_without_reachable_source_warns_about_the_fallback() {
    let _lock = crate::testutil::exec_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alix.toml"), "").unwrap();
    let initial = walk_deck(dir.path());
    std::fs::create_dir(dir.path().join("decks")).unwrap();
    let deck_path = dir.path().join("decks/t.md");
    std::fs::rename(&initial.deck_path, &deck_path).unwrap();
    crate::stamp::stamp_deck(&deck_path).unwrap();
    crate::assets::freeze_member(&deck_path).unwrap();
    let mut trace = crate::trace::Trace::from_deck(&Deck::load(&deck_path).unwrap()).unwrap();
    trace.base_root = Some(dir.path().join("gone-source"));
    assert!(
        trace
            .checkpoints
            .first()
            .is_some_and(|checkpoint| trace.frozen_block(checkpoint).is_some())
    );
    let mut walking = Walking::new(Walk::new(trace), None);
    let cli = crate::testutil::fake_reply(dir.path(), "answer");
    let cfg = crate::testutil::ask_config(&cli);

    assert!(walking.start_ask(&cfg, Audience::Adult, Some("why?".to_string())));
    assert_eq!(
        Some(ask::FROZEN_ONLY_WARNING.to_string()),
        walking.ask_dto(None, None).status
    );
}

fn aug_card(front: &str, back: &str) -> Card {
    let mut card = Card::plain(
        Arc::from("d.md"),
        front.to_string(),
        vec![back.to_string()],
        None,
        1,
    );
    card.token = Some(Arc::from(front.to_ascii_lowercase()));
    card
}

#[test]
fn augmenting_reports_coverage_and_removal_persists() {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("deck1.json");
    let cards = vec![aug_card("Q1", "a"), aug_card("Q2", "b")];

    let mut seed = AugmentCache::open(&cache_path);
    seed.set_distractors(
        &cards[0].id().unwrap(),
        vec!["x".into()],
        cards[0].content_fingerprint,
    );
    seed.set_note(
        &cards[1].id().unwrap(),
        "n".into(),
        cards[1].content_fingerprint,
    );
    seed.save().unwrap();

    let mut aug = Augmenting::open(
        "d.md".into(),
        cards.clone(),
        vec![],
        AugmentCache::open(&cache_path),
        None,
    );
    let dto = aug.dto();
    assert_eq!(2, dto.cards);
    assert!(dto.busy.is_none());
    let choices = dto.rows.iter().find(|r| r.kind == "choices").unwrap();
    assert_eq!((1, 2), (choices.covered, choices.eligible));
    let topo = dto.rows.iter().find(|r| r.kind == "topology").unwrap();
    assert!(topo.items.is_empty());

    assert!(aug.remove("choices", None));
    assert_eq!(
        0,
        aug.dto()
            .rows
            .iter()
            .find(|r| r.kind == "choices")
            .unwrap()
            .covered
    );
    let reloaded = AugmentCache::open(&cache_path);
    assert_eq!(
        None,
        reloaded.distractors(&cards[0].id().unwrap(), cards[0].content_fingerprint)
    );
    assert_eq!(
        Some("n"),
        reloaded.note(&cards[1].id().unwrap(), cards[1].content_fingerprint)
    );

    assert!(!aug.remove("bogus", None));
}

#[test]
fn augmenting_generate_is_a_noop_when_a_target_is_fully_covered() {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("deck1.json");
    let cards = vec![aug_card("Q", "a")];

    let mut seed = AugmentCache::open(&cache_path);
    seed.set_distractors(
        &cards[0].id().unwrap(),
        vec!["x".into()],
        cards[0].content_fingerprint,
    );
    seed.save().unwrap();

    let mut aug = Augmenting::open(
        "d.md".into(),
        cards,
        vec![],
        AugmentCache::open(cache_path),
        None,
    );
    let started = aug.generate_batch(
        vec![("choices".into(), None)],
        &AiConfig::default(),
        &AskConfig::default(),
    );
    assert!(!started);
    assert!(aug.dto().busy.is_none());
    assert_eq!(
        vec!["choices"],
        aug.dto().done,
        "no-gap target still counts as done"
    );
}

#[cfg(unix)]
#[test]
fn generate_batch_runs_every_target_even_after_one_fails() {
    let _g = crate::testutil::exec_lock();
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("deck1.json");
    let cards = vec![aug_card("Q", "a")];
    let mut aug = Augmenting::open(
        "d.md".into(),
        cards,
        vec![],
        AugmentCache::open(cache_path),
        None,
    );

    let ai = AiConfig::default();
    let cli = crate::testutil::fake_reply(dir.path(), r#"{"0": "a note"}"#);
    let ask = crate::testutil::ask_config(&cli);

    assert!(aug.generate_batch(
        vec![("choices".into(), None), ("notes".into(), None)],
        &ai,
        &ask
    ));

    // Deadline-based: a fixed yield count can elapse before the fake CLI
    // processes even finish on a slow runner.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        aug.poll(&ai, &ask);
        if aug.pending.is_none() && aug.queue.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        aug.pending.is_none() && aug.queue.is_empty(),
        "batch never finished draining"
    );

    let dto = aug.dto();
    assert_eq!(vec!["notes"], dto.done, "notes succeeded");
    assert_eq!(
        1,
        dto.failed.len(),
        "choices was attempted and failed, not skipped"
    );
    assert_eq!("choices", dto.failed[0].target);
    assert!(!dto.failed[0].error.is_empty());
}

#[test]
fn deck_drawer_dto_exposes_preamble_and_a_flat_heatmap() {
    let dir = tempfile::tempdir().unwrap();
    let deck_path = dir.path().join("rust.md");
    std::fs::write(
        &deck_path,
        "# Rust\nA short intro.\n\n## q1 <!-- id: card-qa -->\na1\n## q2 <!-- id: card-qb -->\na2\n",
    )
    .unwrap();
    let deck = Deck::load(&deck_path).unwrap();

    let store = Store::open(dir.path().join("progress/deck1.json")).unwrap();
    let augment = AugmentCache::open_for_workspace(dir.path()).unwrap();

    let dto = deck_drawer_dto(&augment, &store, &deck, None);
    assert_eq!(Some("A short intro."), dto.preamble.as_deref());
    // One cell per stamped card; a never-presented card is the untouched tier.
    assert_eq!(vec![CardTier::Untouched, CardTier::Untouched], dto.heatmap);
    assert!(dto.topologies.is_empty());
}

#[test]
fn a_lan_pairing_reply_carries_a_qr_svg() {
    let pair = PairInfo {
        url: "http://192.168.1.2:7777/?token=ab".to_string(),
        lan: true,
    };
    let svg = if pair.lan {
        crate::qr::svg(&pair.url)
    } else {
        None
    };
    assert!(svg.unwrap().starts_with("<svg "));
}

#[test]
fn a_scoped_instance_always_keeps_its_current_dir() {
    let current = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let cfg = current.path().join("config.toml");
    // Forward slashes: in a TOML basic string a Windows `\U...` path reads as
    // an (invalid) escape sequence.
    std::fs::write(
        &cfg,
        format!(
            "decks_dir = \"{}\"\n",
            other.path().display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    let dir = effective_decks_dir(true, Some(&cfg), current.path());
    assert_eq!(current.path(), dir);
}

#[test]
fn an_unscoped_instance_follows_a_config_naming_a_different_dir() {
    let current = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let cfg = current.path().join("config.toml");
    // Forward slashes: see a_scoped_instance_always_keeps_its_current_dir.
    std::fs::write(
        &cfg,
        format!(
            "decks_dir = \"{}\"\n",
            other.path().display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    let dir = effective_decks_dir(false, Some(&cfg), current.path());
    assert_eq!(other.path(), dir);
}

#[test]
fn an_unparseable_config_keeps_the_current_dir() {
    let current = tempfile::tempdir().unwrap();
    let cfg = current.path().join("config.toml");
    std::fs::write(&cfg, "not valid toml [[[\n").unwrap();
    let dir = effective_decks_dir(false, Some(&cfg), current.path());
    assert_eq!(current.path(), dir);
}
