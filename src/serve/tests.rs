use std::sync::Arc;

use super::*;
use crate::{
    answer::{Input, Mode, mode_name},
    ask::{self, Reply},
    card::Card,
    choice,
    config::{AskConfig, ReviewConfig},
    depth::Depth,
    picker,
    render::NoteUnit,
    scheduler::{Fsrs, Grade},
    session::Session,
    trace::{Delta, SourceBase},
};

#[test]
fn unconfigured_token_leaves_everything_open() {
    assert!(is_authorized("/api/decks", None, None, None));
    assert!(is_authorized("/", None, None, None));
}

#[test]
fn token_guards_only_the_api() {
    let t = Some("secret");
    assert!(is_authorized("/", None, None, t));
    assert!(is_authorized("/img/deadbeef", None, None, t));
    assert!(is_authorized("/theme.css", None, None, t));
    assert!(!is_authorized("/api/decks", None, None, t));
    assert!(!is_authorized("/api/decks", Some("Bearer wrong"), None, t));
    assert!(is_authorized("/api/decks", Some("Bearer secret"), None, t));
    assert!(is_authorized("/api/decks", None, Some("secret"), t));
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
    let dto = card_dto((&card).into());

    assert_eq!(dto.front, "the front");
    assert_eq!(dto.back, vec!["the back".to_string()]);
    assert_eq!(dto.note.len(), 2);
    match &dto.note[0] {
        NoteUnit::Sentence { text } => assert_eq!(text, "Intro here."),
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
    card.image = Some(PathBuf::from("/imgs/moon.png"));
    card.image_back = Some(PathBuf::from("/imgs/tab.png"));

    let dto = card_dto((&card).into());
    let img = dto.img.expect("front image url");
    let img_back = dto.img_back.expect("back image url");
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
    let dto = card_dto((&card).into());
    assert!(dto.img.is_none() && dto.img_back.is_none());
    assert!(collect_images(std::slice::from_ref(&card)).is_empty());
}

#[test]
fn content_type_by_extension() {
    assert_eq!(content_type(Path::new("a.png")), "image/png");
    assert_eq!(content_type(Path::new("a.JPG")), "image/jpeg");
    assert_eq!(content_type(Path::new("a.jpeg")), "image/jpeg");
    assert_eq!(content_type(Path::new("a.svg")), "image/svg+xml");
    assert_eq!(content_type(Path::new("a.bin")), "application/octet-stream");
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
    assert!(app_page(Audience::Kids).contains("alix kids"));
}

#[test]
fn resolve_row_resolves_a_unique_bare_deck_name() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("solo.md"), "## f\nb\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        Resolved::One(dir.path().join("solo.md")),
        resolve_row("solo.md", dir.path(), &recent)
    );
}

#[test]
fn resolve_row_resolves_an_unknown_name_to_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        Resolved::Unknown,
        resolve_row("../etc/passwd", dir.path(), &recent)
    );
}

#[test]
fn resolve_row_resolves_a_workspace_row_to_many_with_every_member_file() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("a.md"), "## a\nb\n").unwrap();
    std::fs::write(ws.join("b.md"), "## c\nd\n").unwrap();
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        Resolved::Many {
            dir: ws.clone(),
            files: vec![ws.join("a.md"), ws.join("b.md")],
        },
        resolve_row("english", dir.path(), &recent)
    );
}

#[test]
fn resolve_row_resolves_a_manifest_only_dir_with_no_members_to_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("empty-ws");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"Empty\"\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert!(picker::catalog(dir.path(), &recent, &mut DeckCache::default()).is_empty());
    assert_eq!(
        Resolved::Unknown,
        resolve_row("empty-ws", dir.path(), &recent)
    );
}

#[test]
fn resolve_row_rejects_a_bare_name_duplicated_across_two_containers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.md"), "## f\nb\n").unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("a.md"), "## g\nh\n").unwrap();
    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[elsewhere.path().join("a.md")], 1000);

    assert_eq!(
        Resolved::Ambiguous,
        resolve_row("a.md", dir.path(), &recent)
    );
}

#[test]
fn resolve_row_resolves_a_qualified_member_name_even_when_its_bare_workspace_name_is_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("a.md"), "## a\nb\n").unwrap();
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();

    let other_ws = tempfile::tempdir().unwrap();
    let other_english = other_ws.path().join("english");
    std::fs::create_dir(&other_english).unwrap();
    std::fs::write(other_english.join("z.md"), "## z\ny\n").unwrap();
    std::fs::write(
        other_english.join(crate::workspace::MANIFEST),
        "title = \"Other English\"\n",
    )
    .unwrap();

    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[other_english], 1000);

    assert_eq!(
        Resolved::Ambiguous,
        resolve_row("english", dir.path(), &recent)
    );
    assert_eq!(
        Resolved::One(ws.join("a.md")),
        resolve_row("english/a.md", dir.path(), &recent)
    );
}

#[test]
fn a_qualified_member_name_duplicated_across_two_same_named_containers_is_ambiguous() {
    // Both qualified keys collide too: must reject, not last-wins (dangerous
    // behind /api/reset's delete-by-path).
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("a.md"), "## a\nb\n").unwrap();
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();

    let other_ws = tempfile::tempdir().unwrap();
    let other_english = other_ws.path().join("english");
    std::fs::create_dir(&other_english).unwrap();
    std::fs::write(other_english.join("a.md"), "## z\ny\n").unwrap();
    std::fs::write(
        other_english.join(crate::workspace::MANIFEST),
        "title = \"Other English\"\n",
    )
    .unwrap();

    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[other_english], 1000);

    assert_eq!(
        Resolved::Ambiguous,
        resolve_row("english", dir.path(), &recent)
    );
    assert_eq!(
        Resolved::Ambiguous,
        resolve_row("english/a.md", dir.path(), &recent)
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
    std::fs::write(ws.join("a.md"), "## a\nb\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        resolve_dest(None, dir.path(), &recent),
        Some(dir.path().to_path_buf())
    );
    assert_eq!(
        resolve_dest(Some(""), dir.path(), &recent),
        Some(dir.path().to_path_buf())
    );
    assert_eq!(
        resolve_dest(Some("english"), dir.path(), &recent),
        Some(ws.clone())
    );
    assert_eq!(
        resolve_dest(Some("no-such-workspace"), dir.path(), &recent),
        None
    );
    assert_eq!(resolve_dest(Some("../etc"), dir.path(), &recent), None);
}

#[test]
fn resolve_dest_rejects_a_dir_name_duplicated_across_two_containers() {
    // Same collision class as resolve_row: picking either container silently
    // would be the same bug.
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("a.md"), "## a\nb\n").unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let other_english = elsewhere.path().join("english");
    std::fs::create_dir(&other_english).unwrap();
    std::fs::write(other_english.join("z.md"), "## z\ny\n").unwrap();
    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[other_english], 1000);

    assert_eq!(resolve_dest(Some("english"), dir.path(), &recent), None);
}

#[test]
fn a_group_row_aggregates_member_reviewability_instead_of_hardcoding_true() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("animals");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Animals\"\n").unwrap();
    std::fs::write(ws.join("one.md"), "## q1 <!-- id: qa -->\na1\n").unwrap();
    std::fs::write(ws.join("two.md"), "## q2 <!-- id: qb -->\na2\n").unwrap();

    let mut ws_store = Store::open(crate::workspace::store_path(&ws)).unwrap();
    let now = now_ms();
    for name in ["one.md", "two.md"] {
        let deck = Deck::load(ws.join(name)).unwrap();
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
        true,
        &mut icons,
        ReviewConfig::default(),
        &mut DeckCache::default(),
    );

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
}

#[test]
fn a_plain_folders_member_badge_reads_the_served_instance_store_not_the_global_default() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("letters");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("a.md"), "## q <!-- id: qa -->\na\n").unwrap();

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
        true,
        &mut icons,
        ReviewConfig::default(),
        &mut DeckCache::default(),
    );

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
    std::fs::write(dir.path().join("broken.md"), "## front\nbad \\cloze{oops\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));
    let entry = picker::catalog(dir.path(), &recent, &mut DeckCache::default())
        .into_iter()
        .find(|e| e.name == "broken.md")
        .expect("catalog lists the broken deck file even though it won't parse");
    assert!(
        Deck::load(&entry.path).is_err(),
        "fixture must actually fail to load"
    );

    let store = Store::open(dir.path().join("progress.json")).unwrap();
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
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
fn review_state_select_phase_has_no_card() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("p.json")).unwrap();
    let dto = review_state(None, &store);
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
    let dto = review_state(Some(&r), &store);
    assert_eq!(dto.phase, "done");
    assert_eq!(dto.kind, "review");
}

fn reviewing_at(deck: PathBuf, cards: Vec<Card>, store: &Store, depth: Depth) -> Reviewing {
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
    let augment = crate::augment::AugmentCache::open(deck.with_extension("augment.json"));
    let mut decks = HashMap::new();
    decks.insert("d.md".to_string(), deck);
    Reviewing::new(SessionBuild {
        session,
        label: "d.md".to_string(),
        decks,
        links: HashMap::new(),
        source_roots: HashMap::new(),
        source_bases: HashMap::new(),
        topology_name: None,
        augment,
    })
}

#[test]
fn state_reports_the_sessions_depth_and_typeline_mode() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## steps <!-- reveal: line --> <!-- id: q1 -->\nfirst\nsecond\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::l1::parse_str("d.md", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&cards[0].id().unwrap(), 0);
    let r = reviewing_at(deck, cards, &store, Depth::Reconstruct);

    let dto = review_state(Some(&r), &store);
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
    let text = "## why <!-- id: q1 -->\nfirst fact\nsecond fact\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::l1::parse_str("d.md", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&cards[0].id().unwrap(), 0);
    let mut r = reviewing_at(deck, cards.clone(), &store, Depth::Reconstruct);

    let fallback = review_state(Some(&r), &store);
    assert_eq!(fallback.mode, "explain");
    assert_eq!(
        fallback.keypoints,
        Some(vec!["first fact".to_string(), "second fact".to_string()])
    );

    r.augment
        .set_keypoints(&cards[0].id().unwrap(), vec!["one claim".to_string()]);
    let cached = review_state(Some(&r), &store);
    assert_eq!(cached.keypoints, Some(vec!["one claim".to_string()]));
}

#[test]
fn recognize_state_offers_gap_options_for_a_cloze_card() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## where <!-- id: q1 -->\nThe \\cloze{cat} sat here\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::l1::parse_str("d.md", text).unwrap();
    assert_eq!(vec!["cat".to_string()], cards[0].back);
    let id = cards[0].id().unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&id, 0);
    let mut r = reviewing_at(deck, cards, &store, Depth::Recognize);
    r.augment.set_distractors(
        &id,
        vec!["dog".to_string(), "fish".to_string(), "bird".to_string()],
    );

    let dto = review_state(Some(&r), &store);
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
    let text = "## steps <!-- reveal: line --> <!-- id: q1 -->\nfirst\nsecond\nthird\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::l1::parse_str("d.md", text).unwrap();
    let id = cards[0].id().unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&id, 0);
    let mut r = reviewing_at(deck, cards, &store, Depth::Recognize);
    r.augment.set_distractors(
        &id,
        vec![
            "second\nfirst\nthird".to_string(),
            "third\nsecond\nfirst".to_string(),
            "first\nthird\nsecond".to_string(),
        ],
    );

    let dto = review_state(Some(&r), &store);
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
    let text = "## steps <!-- reveal: line --> <!-- id: q1 -->\nfirst\nsecond\nthird\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::l1::parse_str("d.md", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&cards[0].id().unwrap(), 0);
    let r = reviewing_at(deck, cards, &store, Depth::Recognize);

    let dto = review_state(Some(&r), &store);
    assert!(
        dto.choices.is_none(),
        "no cached distractors and no offline pool → the fallback signal"
    );
}

#[test]
fn recognize_state_reshuffles_choice_options_on_the_next_appearance_but_not_mid_poll() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.md");
    let text = "## q <!-- id: q1 -->\nanswer\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::l1::parse_str("d.md", text).unwrap();
    let id = cards[0].id().unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(&id, 0);
    let mut r = reviewing_at(deck, cards, &store, Depth::Recognize);
    r.augment.set_distractors(
        &id,
        vec![
            "wrong one".to_string(),
            "wrong two".to_string(),
            "wrong three".to_string(),
        ],
    );

    let first = review_state(Some(&r), &store)
        .choices
        .expect("a valid MC from the 3 cached AI distractors");
    let second = review_state(Some(&r), &store)
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
        r.session.poll(&store, now);
        assert_eq!(
            Some(id.clone()),
            r.session.current().and_then(|c| c.id()),
            "past the floor, the card returns"
        );
        let later = review_state(Some(&r), &store)
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
    let text = "## q <!-- id: q1 -->\nanswer\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::l1::parse_str("d.md", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    let state = store.get_or_insert(&cards[0].id().unwrap(), 0);
    state.recognized_ms = Some(500);
    let r = reviewing_at(deck, cards, &store, Depth::Recall);

    let dto = review_state(Some(&r), &store);
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
    std::fs::write(&deck, "## front <!-- id: q1 -->\nback\n").unwrap();
    let store = Store::open(dir.join("p.json")).unwrap();
    let mut card = Card::plain(
        Arc::from("d.md"),
        "front".to_string(),
        vec!["back".to_string()],
        None,
        1,
    );
    card.token = Some(Arc::from("q1"));
    let session = Session::new(
        vec![card.clone()],
        &store,
        Box::new(Fsrs::default()),
        crate::session::SessionOptions::default(),
        now_ms(),
    );
    let mut decks = HashMap::new();
    decks.insert("d.md".to_string(), deck.clone());
    let reviewing = Reviewing::new(SessionBuild {
        session,
        label: "d.md".to_string(),
        decks,
        links: HashMap::new(),
        source_roots: HashMap::new(),
        source_bases: HashMap::new(),
        topology_name: None,
        augment: crate::augment::AugmentCache::open(deck.with_extension("augment.json")),
    });
    (reviewing, card, deck)
}

#[test]
fn poll_ask_records_answer_in_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, card, _deck) = one_card_reviewing(dir.path());
    let (tx, rx) = std::sync::mpsc::channel();
    r.ask.pending = Some(Pending {
        rx,
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
fn poll_ask_condense_appends_note_to_deck() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, card, deck) = one_card_reviewing(dir.path());
    r.ask.transcript.push(("q".to_string(), "a".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    r.ask.pending = Some(Pending {
        rx,
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
}

#[test]
fn poll_ask_error_resets_session() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, card, _deck) = one_card_reviewing(dir.path());
    r.ask.cli.started = true;
    let (tx, rx) = std::sync::mpsc::channel();
    r.ask.pending = Some(Pending {
        rx,
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

#[test]
fn a_frozen_card_with_no_resolvable_source_root_answers_immediately_without_spawning() {
    // Points at a nonexistent CLI binary: if the source-not-found short-circuit
    // works, it's never touched.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("29.rs"), "fn real() {}\n").unwrap();
    let deck_path = dir.path().join("d.md");
    std::fs::write(
        &deck_path,
        "---\nsource: 29.rs\n---\n## q\na\n<!-- at: 29.rs:1 from src/caching.rs:46-66 -->\n",
    )
    .unwrap();
    // Stamped as in production: an unstamped card has no token and is never
    // servable.
    crate::stamp::stamp_deck(&deck_path).unwrap();
    let deck = crate::deck::Deck::load(&deck_path).unwrap();
    let card = deck.cards[0].clone();
    assert!(card.at_origin.is_some(), "the card is frozen");

    let store = Store::open(dir.path().join("p.json")).unwrap();
    let session = Session::new(
        vec![card.clone()],
        &store,
        Box::new(Fsrs::default()),
        crate::session::SessionOptions::default(),
        now_ms(),
    );
    let mut decks = HashMap::new();
    decks.insert("d.md".to_string(), deck_path);
    let mut source_roots = HashMap::new();
    // Configured (`source_access` opted in), but unresolved on disk.
    source_roots.insert("d.md".to_string(), dir.path().join("gone-origin"));
    let mut source_bases = HashMap::new();
    source_bases.insert("d.md".to_string(), SourceBase::for_deck(&deck));
    let mut r = Reviewing::new(SessionBuild {
        session,
        label: "d.md".to_string(),
        decks,
        links: HashMap::new(),
        source_roots,
        source_bases,
        topology_name: None,
        augment: crate::augment::AugmentCache::open(dir.path().join("a.augment.json")),
    });

    let cfg = crate::testutil::ask_config(&dir.path().join("no-such-claude-binary"));
    assert!(r.start_ask(
        &cfg,
        Audience::Adult,
        AskAction::Question("why?".to_string())
    ));

    // Answered synchronously (no thread/channel): the reply is already in the
    // transcript on the very next read.
    assert!(r.ask.pending.is_none(), "the backend was never spawned");
    assert_eq!(1, r.ask.transcript.len());
    assert_eq!("why?", r.ask.transcript[0].0);
    assert_eq!(ask::SOURCE_NOT_FOUND, r.ask.transcript[0].1);
    assert!(!r.ask_dto(None, None).thinking, "never stuck thinking");

    let (status, error) = r.poll_ask();
    assert_eq!((None, None), (status, error));
    assert_eq!(1, r.ask.transcript.len(), "poll_ask doesn't double-answer");
}

#[test]
fn poll_ask_draft_surfaces_a_parsed_card() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, card, _deck) = one_card_reviewing(dir.path());
    r.ask.transcript.push(("q".to_string(), "a".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    r.ask.pending = Some(Pending {
        rx,
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

fn walk_deck(dir: &Path) -> crate::trace::Trace {
    std::fs::write(dir.join("source.txt"), "first\nsecond\nthird\n").unwrap();
    let path = dir.join("t.md");
    std::fs::write(
        &path,
        "---\ntrace: how it works\nsource: source.txt\n---\n\
         ## Predict the first hop <!-- id: t1 -->\n\
         <!-- given: line — the input line -->\n\
         it reads the first line\n\
         <!-- at: 1 -->\n\
         ## Predict the second hop <!-- id: t2 -->\n\
         it reads line two\n\
         <!-- at: 2 -->\n",
    )
    .unwrap();
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
    assert_eq!(Some("Predict the first hop".to_string()), d.prompt);
    assert_eq!(vec!["line — the input line".to_string()], d.givens);
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
    assert_eq!(vec!["it reads the first line".to_string()], d.points);

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
    let (tx, rx) = std::sync::mpsc::channel();
    w.ask.pending = Some(Pending {
        rx,
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
    let cache_path = dir.path().join("augment.json");
    let cards = vec![aug_card("Q1", "a"), aug_card("Q2", "b")];

    let mut seed = AugmentCache::open(&cache_path);
    seed.set_distractors(&cards[0].id().unwrap(), vec!["x".into()]);
    seed.set_note(&cards[1].id().unwrap(), "n".into());
    seed.save().unwrap();

    let mut aug = Augmenting::open(
        "d.md".into(),
        cards.clone(),
        vec![],
        cache_path.clone(),
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
    assert_eq!(None, reloaded.distractors(&cards[0].id().unwrap()));
    assert_eq!(Some("n"), reloaded.note(&cards[1].id().unwrap()));

    assert!(!aug.remove("bogus", None));
}

#[test]
fn augmenting_generate_is_a_noop_when_a_target_is_fully_covered() {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("augment.json");
    let cards = vec![aug_card("Q", "a")];

    let mut seed = AugmentCache::open(&cache_path);
    seed.set_distractors(&cards[0].id().unwrap(), vec!["x".into()]);
    seed.save().unwrap();

    let mut aug = Augmenting::open("d.md".into(), cards, vec![], cache_path, None);
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

#[test]
fn generate_batch_runs_every_target_even_after_one_fails() {
    let _g = crate::testutil::exec_lock();
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("augment.json");
    let cards = vec![aug_card("Q", "a")];
    let mut aug = Augmenting::open("d.md".into(), cards, vec![], cache_path, None);

    let ai = AiConfig::default();
    let cli = crate::testutil::fake_reply(dir.path(), r#"{"0": "a note"}"#);
    let ask = crate::testutil::ask_config(&cli);

    assert!(aug.generate_batch(
        vec![("choices".into(), None), ("notes".into(), None)],
        &ai,
        &ask
    ));

    for _ in 0..1_000_000 {
        aug.poll(&ai, &ask);
        if aug.pending.is_none() && aug.queue.is_empty() {
            break;
        }
        std::thread::yield_now();
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
fn deck_topology_dto_deck_due_includes_a_due_virtual_card() {
    let dir = tempfile::tempdir().unwrap();
    let deck_path = dir.path().join("rust.md");
    std::fs::write(&deck_path, "## q1 <!-- id: qa -->\na1\n").unwrap();
    let deck = Deck::load(&deck_path).unwrap();

    let mut store = Store::open(dir.path().join("progress.json")).unwrap();
    let now = now_ms();
    store
        .get_or_insert(&deck.cards[0].id().unwrap(), now)
        .recall = Some(crate::store::FsrsState {
        state: 2,
        scheduled_days: 30,
        due_ms: now + 30 * 86_400_000,
        ..Default::default()
    });
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));

    let before = deck_topology_dto(&augment, &store, &deck, ReviewConfig::default());
    assert_eq!(0, before.deck_due);

    let vtext = "## virtual front <!-- id: v1 -->\nvirtual back\n".to_string();
    let vid = crate::l1::parse_str(&deck.subject, &vtext).unwrap()[0]
        .id()
        .unwrap();
    store.insert_virtual(crate::store::VirtualCard {
        id: vid.clone(),
        kind: crate::store::VirtualKind::Remediation,
        parent: deck.subject.clone(),
        text: vtext,
        created_ms: 0,
    });
    store.get_or_insert(&vid, 0);

    let after = deck_topology_dto(&augment, &store, &deck, ReviewConfig::default());
    assert_eq!(1, after.deck_due);
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
    std::fs::write(
        &cfg,
        format!("decks_dir = \"{}\"\n", other.path().display()),
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
    std::fs::write(
        &cfg,
        format!("decks_dir = \"{}\"\n", other.path().display()),
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
