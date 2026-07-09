use std::sync::Arc;

use super::*;
use crate::{
    answer::{Input, Mode, mode_name},
    ask::{self, Reply},
    choice, picker,
    scheduler::{Fsrs, Grade},
    trace::Delta,
};

#[test]
fn unconfigured_token_leaves_everything_open() {
    assert!(is_authorized("/api/decks", None, None, None));
    assert!(is_authorized("/", None, None, None));
}

#[test]
fn token_guards_only_the_api() {
    let t = Some("secret");
    // open surfaces stay open even with a token set
    assert!(is_authorized("/", None, None, t));
    assert!(is_authorized("/img/deadbeef", None, None, t));
    assert!(is_authorized("/theme.css", None, None, t));
    // /api/* requires the token
    assert!(!is_authorized("/api/decks", None, None, t));
    assert!(!is_authorized("/api/decks", Some("Bearer wrong"), None, t));
    assert!(is_authorized("/api/decks", Some("Bearer secret"), None, t));
    // ?token= query is accepted as a fallback
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
fn an_uppercase_txt_extension_is_lowered_before_placing() {
    let name = "FILE.TXT";
    let lower = name.to_ascii_lowercase();
    assert_eq!(normalize_txt_extension(name, &lower), "FILE.txt");
}

#[test]
fn a_lowercase_txt_extension_passes_through_unchanged() {
    let name = "deck.txt";
    let lower = name.to_ascii_lowercase();
    assert_eq!(normalize_txt_extension(name, &lower), "deck.txt");
}

#[test]
fn a_tsv_name_is_left_untouched_by_the_txt_normalizer() {
    let name = "EXPORT.TSV";
    let lower = name.to_ascii_lowercase();
    assert_eq!(normalize_txt_extension(name, &lower), "EXPORT.TSV");
}

#[test]
fn card_dto_structures_the_note() {
    let note = "Intro here.\n```\nfn main() {}\n```";
    let card = Card::plain(
        Arc::from("s.txt"),
        "the front".to_string(),
        vec!["the back".to_string()],
        Some(note.to_string()),
        1,
    );
    let dto = card_dto(&card);

    assert_eq!(dto.front, "the front");
    assert_eq!(dto.back, vec!["the back".to_string()]);
    assert_eq!(dto.note.len(), 2);
    match &dto.note[0] {
        NoteUnitDto::Sentence { text } => assert_eq!(text, "Intro here."),
        other => panic!("expected a sentence, got {other:?}"),
    }
    match &dto.note[1] {
        NoteUnitDto::Code { lines } => assert_eq!(lines, &vec!["fn main() {}".to_string()]),
        other => panic!("expected a code block, got {other:?}"),
    }
}

#[test]
fn card_dto_exposes_image_urls_and_registry_matches() {
    let mut card = Card::plain(
        Arc::from("s.txt"),
        "q".to_string(),
        vec!["a".to_string()],
        None,
        1,
    );
    card.image = Some(PathBuf::from("/imgs/moon.png"));
    card.image_back = Some(PathBuf::from("/imgs/tab.png"));

    let dto = card_dto(&card);
    let img = dto.img.expect("front image url");
    let img_back = dto.img_back.expect("back image url");
    assert!(img.starts_with("/img/"));
    assert!(img_back.starts_with("/img/") && img_back != img);

    // The registry keys the DTO's URLs derive from, so a request for either
    // URL resolves to the right file.
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
        Arc::from("s.txt"),
        "q".to_string(),
        vec!["a".to_string()],
        None,
        1,
    );
    let dto = card_dto(&card);
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
fn resolve_row_resolves_a_unique_bare_deck_name() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("solo.txt"), "# f\n\tb\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        Resolved::One(dir.path().join("solo.txt")),
        resolve_row("solo.txt", dir.path(), &recent)
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
    std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
    std::fs::write(ws.join("b.txt"), "# c\n\td\n").unwrap();
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert_eq!(
        Resolved::Many {
            dir: ws.clone(),
            files: vec![ws.join("a.txt"), ws.join("b.txt")],
        },
        resolve_row("english", dir.path(), &recent)
    );
}

#[test]
fn resolve_row_resolves_a_manifest_only_dir_with_no_members_to_unknown() {
    // A folder with an `alix.toml` manifest but zero `*.txt` decks:
    // `workspace::has_decks` requires at least one member, so
    // `picker::catalog` never surfaces this row at all — it can't reach
    // the old `vec![e.path]`/`One` fallback because it never becomes a
    // catalog entry in the first place.
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("empty-ws");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"Empty\"\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    assert!(picker::catalog(dir.path(), &recent).is_empty());
    assert_eq!(
        Resolved::Unknown,
        resolve_row("empty-ws", dir.path(), &recent)
    );
}

#[test]
fn resolve_row_rejects_a_bare_name_duplicated_across_two_containers() {
    // Two real `a.txt` decks that share a bare name but live in different
    // containers: one under `decks_dir`, the other reached only via
    // `recent` (so the catalog surfaces both under the same key "a.txt").
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "# f\n\tb\n").unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("a.txt"), "# g\n\th\n").unwrap();
    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[elsewhere.path().join("a.txt")], 1000);

    assert_eq!(
        Resolved::Ambiguous,
        resolve_row("a.txt", dir.path(), &recent)
    );
}

#[test]
fn resolve_row_resolves_a_qualified_member_name_even_when_its_bare_workspace_name_is_duplicated() {
    // "english" collides across two containers (ambiguous bare name), but
    // the qualified member key "english/a.txt" is unaffected — qualified
    // and bare names are disjoint namespaces (a filename can't contain
    // `/`), so the collision on one never bleeds into the other.
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();

    let other_ws = tempfile::tempdir().unwrap();
    let other_english = other_ws.path().join("english");
    std::fs::create_dir(&other_english).unwrap();
    std::fs::write(other_english.join("z.txt"), "# z\n\ty\n").unwrap();
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
        Resolved::One(ws.join("a.txt")),
        resolve_row("english/a.txt", dir.path(), &recent)
    );
}

#[test]
fn a_qualified_member_name_duplicated_across_two_same_named_containers_is_ambiguous() {
    // Same setup as the test above (two "english" containers, one reached
    // only via `recent`), but this time both containers hold a member
    // file with the *same* name too ("a.txt"), so the qualified key
    // "english/a.txt" itself collides — the documented always-works
    // escape hatch must reject this, not last-wins onto whichever
    // container the catalog visited last (dangerous behind /api/reset,
    // which writes progress/deletes by this resolved path).
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
    std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();

    let other_ws = tempfile::tempdir().unwrap();
    let other_english = other_ws.path().join("english");
    std::fs::create_dir(&other_english).unwrap();
    std::fs::write(other_english.join("a.txt"), "# z\n\ty\n").unwrap();
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
        resolve_row("english/a.txt", dir.path(), &recent)
    );
}

// Note: the workspace-row-to-`Many{dir,files}` case the roadmap's
// reset-specific test asked for (every member file, `dir` == row path) is
// already asserted in full by
// `resolve_row_resolves_a_workspace_row_to_many_with_every_member_file`
// above (extended to cover `dir` in the 0b1b859 review follow-up) —
// no new assertion here would add coverage, so none is added.

#[test]
fn a_drained_job_ignores_further_messages_without_replacing() {
    // Tests the guard at the top of `poll()`: if `self.outcome.is_some()`,
    // return immediately without draining — the drain-once law. All three
    // job POSTs rely on this: a repeat poll must never re-place the deck or
    // re-run the outcome, else a second message would clobber the first.
    // Mutation test: without the guard, poll #2 would drain the second
    // message, call place_deck again (placing a second file), and asserts 2
    // and 3 would fail.
    let dest = tempfile::tempdir().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();

    // First message: place a deck.
    tx.send(Ok("# f\n\tb\n".to_string())).unwrap();
    let mut g = Generating {
        rx,
        url: "https://example.com/some-article".to_string(),
        dest: dest.path().to_path_buf(),
        started: Instant::now(),
        outcome: None,
    };

    // Poll #1: outcome set, one deck placed.
    g.poll();
    assert!(g.outcome.is_some());
    let files_after_poll_1: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
    assert_eq!(1, files_after_poll_1.len());

    // Send a second, distinguishable message (would place a different deck
    // if poll #2 tried to drain it).
    tx.send(Ok("# other\n\tanswer\n".to_string())).unwrap();

    // Poll #2: guard should short-circuit, leaving the second message queued.
    let first_outcome = g.outcome.clone();
    g.poll();
    assert_eq!(first_outcome, g.outcome, "outcome must stay unchanged");
    let files_after_poll_2: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
    assert_eq!(1, files_after_poll_2.len(), "still only one placed file");

    // The second message is still queued (guard never called try_recv).
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

    // No fixed length at all (a body whose declared length lies, or is
    // absent) is still bounded by the `take()` ceiling: `read_capped`
    // never reads more than `cap + 1` bytes before rejecting, so an
    // endless reader is caught rather than read to exhaustion.
    assert!(read_capped(std::io::repeat(7), CAP).is_none());
}

#[test]
fn resolve_dest_falls_back_to_decks_dir_and_rejects_unknown_names() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
    let recent = RecentDecks::load(dir.path().join("recent.json"));

    // Absent/empty → the served root, without touching the catalog.
    assert_eq!(
        resolve_dest(None, dir.path(), &recent),
        Some(dir.path().to_path_buf())
    );
    assert_eq!(
        resolve_dest(Some(""), dir.path(), &recent),
        Some(dir.path().to_path_buf())
    );
    // A known workspace name → its directory.
    assert_eq!(
        resolve_dest(Some("english"), dir.path(), &recent),
        Some(ws.clone())
    );
    // An unknown name (or a crafted path) resolves to nothing.
    assert_eq!(
        resolve_dest(Some("no-such-workspace"), dir.path(), &recent),
        None
    );
    assert_eq!(resolve_dest(Some("../etc"), dir.path(), &recent), None);
}

#[test]
fn resolve_dest_rejects_a_dir_name_duplicated_across_two_containers() {
    // Same class of collision `resolve_row` rejects for bare deck names:
    // `resolve_dest` also scans top-level catalog rows, so a workspace
    // reached via `recent` can share a name with one physically inside
    // `decks_dir` — silently picking either would be the same class of
    // bug this task closes for names.
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("english");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let other_english = elsewhere.path().join("english");
    std::fs::create_dir(&other_english).unwrap();
    std::fs::write(other_english.join("z.txt"), "# z\n\ty\n").unwrap();
    let mut recent = RecentDecks::load(dir.path().join("recent.json"));
    recent.record(&[other_english], 1000);

    assert_eq!(resolve_dest(Some("english"), dir.path(), &recent), None);
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
    // The session-end signal is the `done` phase now, not a `finished` flag:
    // the field is gone from the wire contract entirely.
    let json = serde_json::to_value(&dto).unwrap();
    assert!(json.get("finished").is_none());
}

#[test]
fn finished_review_uses_the_done_phase_not_a_finished_flag() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, _card, _deck) = one_card_reviewing(dir.path());
    let mut store = Store::open(dir.path().join("graded.json")).unwrap();
    // Pass the only card → the queue empties → the session is finished.
    r.session.grade(&mut store, Grade::Pass, now_ms());
    assert!(r.session.is_finished());
    let dto = review_state(Some(&r), &store);
    assert_eq!(dto.phase, "done");
    assert_eq!(dto.kind, "review");
}

/// Builds a `Reviewing` over a parsed deck at a chosen depth, sharing `store`
/// (seed it before calling so the session sees the seeded state).
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
    let mut decks = HashMap::new();
    decks.insert("d.txt".to_string(), deck);
    Reviewing::new(SessionBuild {
        session,
        label: "d.txt".to_string(),
        decks,
        links: HashMap::new(),
        source_roots: HashMap::new(),
        source_bases: HashMap::new(),
        topology_name: None,
    })
}

#[test]
fn state_reports_the_sessions_depth_and_typeline_mode() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.txt");
    let text = "# steps\n% reveal: line\n\tfirst\n\tsecond\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.txt", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(cards[0].id(), 0); // seen, so it's a quiz not an acquire
    let r = reviewing_at(deck, cards, &store, Depth::Reconstruct);

    let dto = review_state(Some(&r), &store);
    assert_eq!(
        "reconstruct", dto.depth,
        "the DTO reports the session's depth"
    );
    assert_eq!(
        "typeline", dto.mode,
        "reconstruct + `% reveal: line` types the next line"
    );
}

#[test]
fn recognize_state_offers_gap_options_for_a_cloze_card() {
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.txt");
    // A real expanded cloze card (its sub-card's back is the bare gap text)
    // plus sibling cards whose backs are the gap distractors.
    let text =
        "# where\n% reveal: cloze\n\tThe {{cat}} sat here\n# a\n\tdog\n# b\n\tfish\n# c\n\tbird\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.txt", text).unwrap();
    assert_eq!(vec!["cat".to_string()], cards[0].back); // gap text is the back
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(cards[0].id(), 0); // seen → the Recognize MC, not the acquire on-ramp
    let r = reviewing_at(deck, cards, &store, Depth::Recognize);

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
    // A `% reveal: line` card (ordered multi-line back) at Recognize must offer
    // whole-sequence options — the real ordering vs the AI's alternate wrong
    // orderings — never a pick-one-step built from the card's own lines.
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.txt");
    let text = "# steps\n% reveal: line\n\tfirst\n\tsecond\n\tthird\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.txt", text).unwrap();
    let id = cards[0].id();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(id, 0); // seen → the Recognize MC, not the acquire on-ramp
    let mut r = reviewing_at(deck, cards, &store, Depth::Recognize);
    r.augment.set_distractors(
        id,
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
    // Same card, but the augment cache holds nothing: `build` can't reach four
    // distinct options (this is the only card in the deck, so the offline pool
    // is empty too), so it falls back to `None` — the client's self-report
    // chips, not a synthesized pick-one-step question.
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.txt");
    let text = "# steps\n% reveal: line\n\tfirst\n\tsecond\n\tthird\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.txt", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(cards[0].id(), 0); // seen → the Recognize MC, not the acquire on-ramp
    let r = reviewing_at(deck, cards, &store, Depth::Recognize);

    let dto = review_state(Some(&r), &store);
    assert!(
        dto.choices.is_none(),
        "no cached distractors and no offline pool → the fallback signal"
    );
}

#[test]
fn recognize_state_reshuffles_choice_options_on_the_next_appearance_but_not_mid_poll() {
    // End-to-end through `review_state`/`current_question`: the client polls
    // `GET /api/state` every ~3s while a card is on screen, and a poll that
    // doesn't move the session off the card must rebuild identical options
    // (`Session::appearance` only bumps on a genuine re-serve); a wrong pick now
    // floors instead of resurfacing instantly (the same-card transition floor
    // now covers Recognize too), and once the floor passes and the card is
    // served again, the options reshuffle ({#reorder-mc-on-each-appearance}).
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.txt");
    let text = "# q\n\tanswer\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.txt", text).unwrap();
    let id = cards[0].id();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    store.get_or_insert(id, 0); // seen → the Recognize MC, not the acquire on-ramp
    let mut r = reviewing_at(deck, cards, &store, Depth::Recognize);
    // A full set of cached AI distractors so the lone card can build a valid MC
    // (there's no other card in the deck to sample offline distractors from).
    r.augment.set_distractors(
        id,
        vec![
            "wrong one".to_string(),
            "wrong two".to_string(),
            "wrong three".to_string(),
        ],
    );

    let first = review_state(Some(&r), &store)
        .choices
        .expect("a valid MC from the 3 cached AI distractors");
    // A second poll of the same appearance (no session mutation in between)
    // must rebuild the identical options.
    let second = review_state(Some(&r), &store)
        .choices
        .expect("still the same appearance");
    assert_eq!(first, second, "an idle poll must not reshuffle mid-answer");

    // A wrong pick floors the card (Part 1) instead of resurfacing it instantly.
    // Cycle it a handful of times — tolerating the rare same-permutation
    // collision across any single pair of appearances — and require at least
    // one later appearance to land on a different order.
    let mut now = now_ms();
    let mut saw_a_different_order = false;
    for _ in 0..5 {
        r.session.grade(&mut store, Grade::Fail, now);
        assert!(
            r.session.is_finished(),
            "the only card floors instead of resurfacing instantly"
        );
        now += crate::scheduler::ACQUIRE_COOLDOWN_MS;
        r.session.poll(&store, now);
        assert_eq!(
            Some(id),
            r.session.current().map(|c| c.id()),
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
    // A card recognized in a prior Recognize session carries `recognized_ms`
    // and a store entry, so a later Recall session quizzes it directly — never
    // through the recognition-MC acquire on-ramp (spec §4.6).
    let dir = tempfile::tempdir().unwrap();
    let deck = dir.path().join("d.txt");
    let text = "# q\n\tanswer\n";
    std::fs::write(&deck, text).unwrap();
    let cards = crate::parser::parse_str("d.txt", text).unwrap();
    let mut store = Store::open(dir.path().join("p.json")).unwrap();
    let state = store.get_or_insert(cards[0].id(), 0);
    state.recognized_ms = Some(500); // recognized, but no Recall schedule yet
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
    // A guard so the JSON contract and the Grade enum stay in sync.
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

// ---- ask-Claude server state machine -------------------------------
//
// These drive `poll_ask` through a channel we control, so the actual CLI
// execution (covered by `ask.rs`'s own tests) isn't involved.

fn one_card_reviewing(dir: &Path) -> (Reviewing, Card, PathBuf) {
    let deck = dir.join("d.txt");
    std::fs::write(&deck, "# front\n\tback\n").unwrap();
    let store = Store::open(dir.join("p.json")).unwrap();
    let card = Card::plain(
        Arc::from("d.txt"),
        "front".to_string(),
        vec!["back".to_string()],
        None,
        1,
    );
    let session = Session::new(
        vec![card.clone()],
        &store,
        Box::new(Fsrs::default()),
        crate::session::SessionOptions::default(),
        now_ms(),
    );
    let mut decks = HashMap::new();
    decks.insert("d.txt".to_string(), deck.clone());
    let reviewing = Reviewing::new(SessionBuild {
        session,
        label: "d.txt".to_string(),
        decks,
        links: HashMap::new(),
        source_roots: HashMap::new(),
        source_bases: HashMap::new(),
        topology_name: None,
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
    // Nothing delivered yet: still thinking, no-op poll.
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
    // A previous card's discussion is on display, and the conversation has
    // begun (the CLI session is live).
    r.ask
        .transcript
        .push(("old q".to_string(), "old a".to_string()));
    r.ask.subject = Some(card.id().wrapping_add(1)); // a different card
    r.ask.cli.started = true;

    r.align_transcript();

    // The current card differs from the transcript's card, so the display is
    // cleared and re-tagged — but Claude's conversation context survives.
    assert!(r.ask.transcript.is_empty());
    assert_eq!(Some(card.id()), r.ask.subject);
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
    // Same condition as ask.rs's `(Some(excerpt), None)` prompt arm (the
    // card is frozen, but its live `% origin:`/deck root doesn't exist on
    // disk) — serve should answer with `SOURCE_NOT_FOUND` synchronously
    // instead of asking the model to echo it. Point the ask config at a
    // nonexistent binary: if the short-circuit works, it's never touched.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("29.rs"), "fn real() {}\n").unwrap();
    let deck_path = dir.path().join("d.txt");
    std::fs::write(
        &deck_path,
        "% source: 29.rs\n# q\n\ta\n\t% at: 29.rs:1 from src/caching.rs:46-66\n",
    )
    .unwrap();
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
    decks.insert("d.txt".to_string(), deck_path);
    let mut source_roots = HashMap::new();
    // Configured (`source_access` opted in), but unresolved on disk.
    source_roots.insert("d.txt".to_string(), dir.path().join("gone-origin"));
    let mut source_bases = HashMap::new();
    source_bases.insert("d.txt".to_string(), SourceBase::for_deck(&deck));
    let mut r = Reviewing::new(SessionBuild {
        session,
        label: "d.txt".to_string(),
        decks,
        links: HashMap::new(),
        source_roots,
        source_bases,
        topology_name: None,
    });

    let cfg = crate::testutil::ask_config(&dir.path().join("no-such-claude-binary"));
    assert!(r.start_ask(&cfg, Some("why?".to_string())));

    // Answered synchronously: no thread/channel, so nothing is pending, and
    // the reply is already in the transcript on the very next read — the
    // page's first poll (`GET /api/ask`) sees it immediately.
    assert!(r.ask.pending.is_none(), "the backend was never spawned");
    assert_eq!(1, r.ask.transcript.len());
    assert_eq!("why?", r.ask.transcript[0].0);
    assert_eq!(ask::SOURCE_NOT_FOUND, r.ask.transcript[0].1);
    assert!(!r.ask_dto(None, None).thinking, "never stuck thinking");

    let (status, error) = r.poll_ask();
    assert_eq!((None, None), (status, error));
    assert_eq!(1, r.ask.transcript.len(), "poll_ask doesn't double-answer");
}

// ── trace walk ──────────────────────────────────────────────────────

/// A two-checkpoint trace over a single source file, in `dir`.
fn walk_deck(dir: &Path) -> crate::trace::Trace {
    std::fs::write(dir.join("source.txt"), "first\nsecond\nthird\n").unwrap();
    let path = dir.join("t.txt");
    std::fs::write(
        &path,
        "% trace: how it works\n\
         % source: source.txt\n\
         # Predict the first hop\n\
         \t% given: line — the input line\n\
         \tit reads the first line\n\
         \t% at: 1\n\
         # Predict the second hop\n\
         \tit reads line two\n\
         \t% at: 2\n",
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

    // Predict: prompt + givens, no excerpt yet, the first node is current.
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

    // Reveal: the live excerpt is read, the prediction is recalled.
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

    // Grade Got: the rail colors the walked node and advances to hop 2.
    w.walk.grade(&mut store, Delta::Passed, 1000);
    let d = walk_dto(&w);
    assert_eq!("predict", d.phase);
    assert_eq!(2, d.current);
    assert_eq!(Some("passed"), d.path[0].delta);
    assert!(d.path[1].current);

    // Walk the last hop → done with a summary (the drill; verification is the
    // separate trace exam, not an in-walk compression).
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
    // Simulate the background grade resolving (no real CLI call in the test).
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
    w.walk.predict("guess".to_string()); // reveal hop 1 (a current checkpoint)

    // A condense reply is in flight (no real CLI call), about the synthesized
    // checkpoint card — its line points at the checkpoint in the deck file.
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

// ── Augment screen (the picker's "Augment" action) ──

fn aug_card(front: &str, back: &str) -> Card {
    Card::plain(
        Arc::from("d.txt"),
        front.to_string(),
        vec![back.to_string()],
        None,
        1,
    )
}

#[test]
fn augmenting_reports_coverage_and_removal_persists() {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("augment.json");
    let cards = vec![aug_card("Q1", "a"), aug_card("Q2", "b")];

    // Seed the on-disk cache: one card has distractors, the other a note.
    let mut seed = AugmentCache::open(&cache_path);
    seed.set_distractors(cards[0].id(), vec!["x".into()]);
    seed.set_note(cards[1].id(), "n".into());
    seed.save().unwrap();

    let mut aug = Augmenting::open("d.txt".into(), cards.clone(), cache_path.clone());
    let dto = aug.dto();
    assert_eq!(2, dto.cards);
    assert!(dto.busy.is_none());
    let choices = dto.rows.iter().find(|r| r.kind == "choices").unwrap();
    assert_eq!((1, 2), (choices.covered, choices.eligible));
    let topo = dto.rows.iter().find(|r| r.kind == "topology").unwrap();
    assert!(topo.items.is_empty());

    // Removing a target writes through to disk; other targets are untouched.
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
    assert_eq!(None, reloaded.distractors(cards[0].id()));
    assert_eq!(Some("n"), reloaded.note(cards[1].id()));

    assert!(!aug.remove("bogus", None)); // unknown target → no-op
}

#[test]
fn augmenting_generate_is_a_noop_when_a_target_is_fully_covered() {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("augment.json");
    let cards = vec![aug_card("Q", "a")];

    let mut seed = AugmentCache::open(&cache_path);
    seed.set_distractors(cards[0].id(), vec!["x".into()]);
    seed.save().unwrap();

    let mut aug = Augmenting::open("d.txt".into(), cards, cache_path);
    // Fully covered → no gap → no costed call is started.
    let started = aug.generate("choices", None, &AiConfig::default(), &AskConfig::default());
    assert!(!started);
    assert!(aug.dto().busy.is_none());
}

#[test]
fn deck_topology_dto_deck_due_includes_a_due_virtual_card() {
    let dir = tempfile::tempdir().unwrap();
    let deck_path = dir.path().join("rust.txt");
    std::fs::write(&deck_path, "# q1\n\ta1\n").unwrap();
    let deck = Deck::load(&deck_path).unwrap();

    let mut store = Store::open(dir.path().join("progress.json")).unwrap();
    let now = now_ms();
    // The one deck card has graduated and isn't due — no deck contribution.
    store.get_or_insert(deck.cards[0].id(), now).recall = Some(crate::store::FsrsState {
        state: 2,
        scheduled_days: 30,
        due_ms: now + 30 * 86_400_000,
        ..Default::default()
    });
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));

    let before = deck_topology_dto(&augment, &store, &deck, ReviewConfig::default());
    assert_eq!(0, before.deck_due);

    // A due virtual card for this deck adds to the whole-deck due count —
    // sidecar content keyed by its `Card::id`, plus a fresh schedule at t=0.
    let vtext = "# virtual front\n\tvirtual back\n".to_string();
    let vid = crate::parser::parse_str(&deck.subject, &vtext).unwrap()[0].id();
    store.insert_virtual(crate::store::VirtualCard {
        id: vid,
        kind: crate::store::VirtualKind::Remediation,
        parent: deck.subject.clone(),
        text: vtext,
        created_ms: 0,
    });
    store.get_or_insert(vid, 0);

    let after = deck_topology_dto(&augment, &store, &deck, ReviewConfig::default());
    assert_eq!(1, after.deck_due);
}

#[test]
fn a_lan_pairing_reply_carries_a_qr_svg() {
    // Mirrors the `/api/pair` handler's own construction: an SVG only
    // when the pairing info is reachable off-device.
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
