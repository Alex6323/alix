use std::{
    io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

pub const TUTORIAL_FILE: &str = "tutorial.md";
pub const SAMPLE_FILE: &str = "tutorial-sample.py";

pub const TUTORIAL_DECK: &str = include_str!("../assets/decks/tutorial.md");
pub const TUTORIAL_SAMPLE: &str = include_str!("../assets/decks/tutorial-sample.py");

pub fn data_root() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix")
        .map(|dirs| dirs.state_dir().unwrap_or(dirs.data_dir()).to_path_buf())
}

pub fn base_dir(data_root: &Path, profile: Option<&str>) -> PathBuf {
    let tutorial = data_root.join("tutorial");
    match profile.filter(|name| crate::config::validate_profile_name(name).is_ok()) {
        Some(name) => tutorial.join("profiles").join(name),
        None => tutorial.join("default"),
    }
}

pub fn materialize(base: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(base).with_context(|| format!("cannot create {}", base.display()))?;
    let sample = base.join(SAMPLE_FILE);
    write_if_changed(&sample, TUTORIAL_SAMPLE)
        .with_context(|| format!("cannot write {}", sample.display()))?;
    let deck = base.join(TUTORIAL_FILE);
    write_if_changed(&deck, TUTORIAL_DECK)
        .with_context(|| format!("cannot write {}", deck.display()))?;
    Ok(deck)
}

fn write_if_changed(path: &Path, bytes: &str) -> io::Result<bool> {
    match std::fs::read(path) {
        Ok(current) if current == bytes.as_bytes() => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    crate::fsio::replace_file(&tmp, path, bytes.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{
        answer::Mode,
        assemble::{self, AssembleConfig, Pacing},
        cite::CitationRoots,
        config::{AskConfig, ReviewConfig},
        deck::Deck,
        parser::region::RegionKind,
        review,
        scheduler::Grade,
        walk::Phase,
    };

    fn test_config() -> AssembleConfig {
        AssembleConfig {
            review: ReviewConfig::default(),
            ask: AskConfig::default(),
            pacing: Pacing {
                max_session: 10,
                new_cards_percent: 30,
            },
            instance_store: None,
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    fn sample_excerpt() -> Vec<(usize, String)> {
        let lines: Vec<(usize, String)> = TUTORIAL_SAMPLE
            .lines()
            .take(6)
            .enumerate()
            .map(|(index, line)| (index + 1, line.to_string()))
            .collect();
        assert_eq!(6, lines.len(), "the sample must hold at least six lines");
        lines
    }

    #[test]
    fn law_base_dir_uses_only_a_valid_profile_name() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let rows: [(Option<&str>, &[&str]); 9] = [
            (None, &["tutorial", "default"]),
            (Some("timmy"), &["tutorial", "profiles", "timmy"]),
            (Some("Default"), &["tutorial", "profiles", "Default"]),
            (Some("default"), &["tutorial", "default"]),
            (Some(".."), &["tutorial", "default"]),
            (Some("."), &["tutorial", "default"]),
            (Some(""), &["tutorial", "default"]),
            (Some("a/b"), &["tutorial", "default"]),
            (Some("a\\b"), &["tutorial", "default"]),
        ];
        for (profile, expected) in rows {
            let expected = expected
                .iter()
                .fold(root.to_path_buf(), |path, part| path.join(part));
            let base = base_dir(root, profile);
            assert_eq!(
                expected,
                base,
                "profile {profile:?} must use {}",
                expected.display()
            );
            assert!(
                base.starts_with(root.join("tutorial")),
                "profile {profile:?} left tutorial/: {}",
                base.display()
            );
        }
    }

    #[test]
    fn materialize_then_walk_leaves_the_deck_byte_identical() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("tutorial").join("default");
        let deck = materialize(&base).unwrap();
        assert_eq!(base.join(TUTORIAL_FILE), deck);
        let sample = base.join(SAMPLE_FILE);
        let mut store = assemble::store_for(std::slice::from_ref(&deck), None).unwrap();
        assemble::walk(&deck, &mut store, &test_config(), crate::time::now_ms()).unwrap();
        assert_eq!(TUTORIAL_DECK, read(&deck), "the walk rewrote the deck");
        assert_eq!(
            TUTORIAL_SAMPLE,
            read(&sample),
            "the walk rewrote the sample"
        );
        assert!(
            !write_if_changed(&deck, TUTORIAL_DECK).unwrap(),
            "the deck was rewritten on a second pass"
        );
        assert!(
            !write_if_changed(&sample, TUTORIAL_SAMPLE).unwrap(),
            "the sample was rewritten on a second pass"
        );
    }

    #[test]
    fn materialize_restores_edited_files() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let deck = materialize(&base).unwrap();
        let sample = base.join(SAMPLE_FILE);
        std::fs::write(&deck, "## edited\nby hand\n").unwrap();
        std::fs::write(&sample, "print('edited')\n").unwrap();
        materialize(&base).unwrap();
        assert_eq!(TUTORIAL_DECK, read(&deck), "the edited deck was kept");
        assert_eq!(TUTORIAL_SAMPLE, read(&sample), "the edited sample was kept");
    }

    #[test]
    fn law_every_do_it_card_reaches_its_affordance() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let deck_path = materialize(&base).unwrap();
        let shipped = std::fs::read(&deck_path).unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        let roots = CitationRoots::for_deck(&deck);
        let mut store = assemble::store_for(std::slice::from_ref(&deck_path), None).unwrap();
        let now = crate::time::now_ms();
        let mut walk = assemble::walk(&deck_path, &mut store, &test_config(), now)
            .unwrap()
            .session;
        assert_eq!(9, walk.total(), "the tutorial serves nine cards");
        let mut served = Vec::new();
        while walk.phase() != Phase::Done {
            let card = walk.current().unwrap().clone();
            let number = deck
                .cards
                .iter()
                .position(|authored| authored.line == card.line)
                .map(|index| index + 1)
                .unwrap();
            served.push(number);
            match number {
                1 => {
                    assert_eq!(
                        (
                            "Reveal this answer, then end the card with one of the three buttons under it. What do they mean?",
                            vec!["Missed it, Partly, Got it: how recalling went, in your own honest judgement.".to_string()],
                        ),
                        (card.front.as_str(), card.back.clone()),
                        "card 1: the instruction and answer no longer teach the grade trio"
                    );
                    walk.reveal();
                    assert_eq!(
                        Phase::Answer,
                        walk.phase(),
                        "card 1: Reveal did not open the answer"
                    );
                    for grade in [Grade::Fail, Grade::Partial, Grade::Pass] {
                        assert!(
                            walk.accepts(grade),
                            "card 1: the Answer phase refuses {grade:?}"
                        );
                    }
                }
                2 => assert_eq!(
                    (
                        "This deck has a section: the text that framed it before the first card. Open it again with the Context chip, then reveal.",
                        Some("Your first walk"),
                        true,
                        false,
                    ),
                    (
                        card.front.as_str(),
                        card.section_context.first().map(String::as_str),
                        card.section_context
                            .iter()
                            .any(|line| line.starts_with("Welcome to Alix.")),
                        walk.section_first(),
                    ),
                    "card 2: the instruction, section title, prose, or manual Context state changed"
                ),
                3 => {
                    assert!(!card.hints.is_empty(), "card 3: no hint to open");
                    assert!(
                        walk.open_hint(&card.id().unwrap()),
                        "card 3: the hint chip does not open"
                    );
                    assert_eq!(
                        shipped,
                        std::fs::read(&deck_path).unwrap(),
                        "card 3: opening the hint changed the shipped deck"
                    );
                }
                4 => assert_eq!(Mode::Choice, walk.mode(), "card 4: no choices to pick from"),
                5 => assert_eq!(Mode::LineByLine, walk.mode(), "card 5: not line by line"),
                6 => assert!(
                    card.span_regions
                        .iter()
                        .any(|region| region.kind == RegionKind::Blank),
                    "card 6: no blank"
                ),
                8 => {
                    let views = review::resolve_citations(&card, Some(&roots));
                    let excerpt = views
                        .first()
                        .and_then(|view| view.excerpt.as_ref())
                        .unwrap_or_else(|| panic!("card 8: no excerpt resolved: {views:?}"));
                    let lines: Vec<(usize, String)> = excerpt
                        .lines
                        .iter()
                        .map(|line| (line.n, line.text.clone()))
                        .collect();
                    assert_eq!(
                        sample_excerpt(),
                        lines,
                        "card 8: the excerpt is not lines 1-6 of the sample"
                    );
                }
                _ => {}
            }
            if walk.phase() == Phase::Front {
                if walk.mode() == Mode::Choice {
                    walk.choose(0);
                } else {
                    walk.reveal();
                }
            }
            assert!(
                walk.grade(&mut store, Grade::Fail, now),
                "card {number}: Missed it was refused"
            );
            assert_eq!(
                shipped,
                std::fs::read(&deck_path).unwrap(),
                "card {number}: grading changed the shipped deck"
            );
        }
        assert_eq!(
            (1..=9).collect::<Vec<_>>(),
            served,
            "a first walk serves the cards in deck order"
        );
    }

    #[test]
    fn the_shipped_citation_fingerprint_matches_the_sample() {
        let deck = crate::parser::parse(TUTORIAL_FILE, TUTORIAL_DECK).unwrap();
        let citations: Vec<_> = deck
            .cards
            .iter()
            .flat_map(|card| card.citations.iter())
            .collect();
        assert_eq!(1, citations.len(), "the tutorial cites exactly one range");
        let citation = citations[0];
        assert_eq!(
            format!("sample:{SAMPLE_FILE}:1-6"),
            citation.locator.to_string()
        );
        let text = sample_excerpt()
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            Some(crate::source::text_fingerprint(&text)),
            citation.fingerprint,
            "the shipped fingerprint is stale; run `cargo test --lib -- --ignored stamp_the_shipped_tutorial`"
        );
    }

    fn strip_fingerprints(text: &str) -> String {
        const TOKEN: &str = " fingerprint: xxh64-";
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            match line
                .find(TOKEN)
                .filter(|_| line.trim_start().starts_with("<!-- at:"))
            {
                Some(start) => {
                    let end = (start + TOKEN.len() + 16).min(line.len());
                    out.push_str(&line[..start]);
                    out.push_str(&line[end..]);
                }
                None => out.push_str(line),
            }
        }
        out
    }

    #[test]
    #[ignore = "one-off: re-stamps assets/decks/tutorial.md in place after a content edit"]
    fn stamp_the_shipped_tutorial() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/decks");
        let asset = assets.join(TUTORIAL_FILE);
        let tmp = tempfile::tempdir().unwrap();
        let deck = tmp.path().join(TUTORIAL_FILE);
        std::fs::write(&deck, strip_fingerprints(&read(&asset))).unwrap();
        std::fs::copy(assets.join(SAMPLE_FILE), tmp.path().join(SAMPLE_FILE)).unwrap();
        crate::stamp::stamp_deck(&deck).unwrap();
        crate::source::stamp_citations(&deck).unwrap();
        std::fs::write(&asset, read(&deck)).unwrap();
    }

    #[test]
    fn the_shipped_tutorial_shows_the_note_grammar() {
        let deck = crate::parser::parse(TUTORIAL_FILE, TUTORIAL_DECK).unwrap();
        for card in &deck.cards {
            assert!(
                card.only_note().is_some(),
                "line {}: every tutorial card carries one note, so a grammar change that stops opening one is loud",
                card.line
            );
        }
        assert!(
            deck.cards
                .iter()
                .all(|card| card.back.iter().all(|line| !line.starts_with('>'))),
            "the tutorial authors every blockquote as a note, so one reaching an answer means the deck went stale"
        );
    }
}
