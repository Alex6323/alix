use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamSectionOutcome {
    pub cards: Vec<String>,
    pub pass: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamOutcome {
    pub sections: Vec<ExamSectionOutcome>,
    pub full: bool,
    pub pass: u32,
    pub partial: u32,
    pub total: u32,
}

fn open_store(deck_path: &Path, root_dir: &str, device: Option<String>) -> Result<alix::store::Store> {
    let root_store = alix::workspace::root_store_path(Path::new(root_dir));
    let mut store = alix::assemble::store_for(&[deck_path.to_path_buf()], Some(&root_store))?;
    if device.is_some() {
        store.device = device;
    }
    Ok(store)
}

fn exam_ideas_path(deck_path: &Path, root_dir: &Path) -> Result<PathBuf> {
    let deck = alix::deck::Deck::load(deck_path)?;
    let deck_id = deck.deck_token.context("the deck is not initialized")?;
    let root_store = alix::workspace::root_store_path(root_dir);
    Ok(alix::state::UserFiles::new(root_store)
        .private_root()
        .join("exam")
        .join(format!("{deck_id}.json")))
}

#[flutter_rust_bridge::frb(sync)]
pub fn exam_ideas_load(deck_path: String, root_dir: String) -> Result<Option<String>> {
    let path = exam_ideas_path(Path::new(&deck_path), Path::new(&root_dir))?;
    match std::fs::read_to_string(&path) {
        Ok(json) => Ok(Some(json)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

#[flutter_rust_bridge::frb(sync)]
pub fn exam_ideas_save(deck_path: String, root_dir: String, json: String) -> Result<()> {
    let path = exam_ideas_path(Path::new(&deck_path), Path::new(&root_dir))?;
    let parent = path.parent().context("the exam ideas path has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, json)
        .with_context(|| format!("cannot write {}", temporary.display()))?;
    std::fs::rename(&temporary, &path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

/// `None` asks for a full sitting.
#[flutter_rust_bridge::frb(sync)]
pub fn exam_start_cards(
    deck_path: String,
    root_dir: String,
    needs_work_only: bool,
) -> Result<Option<Vec<String>>> {
    let path = PathBuf::from(deck_path);
    alix::assemble::stamp_for_session(&path)?;
    if !needs_work_only {
        return Ok(None);
    }
    let deck = alix::deck::Deck::load(&path)?;
    let store = open_store(&path, &root_dir, None)?;
    Ok(Some(alix::exam_status::needs_work_card_ids(&deck, &store)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamApplied {
    pub unlocked: bool,
    pub standing: Option<String>,
}

#[flutter_rust_bridge::frb(sync)]
pub fn apply_exam_result(
    deck_path: String,
    root_dir: String,
    outcome: ExamOutcome,
    now_ms: u64,
    device: Option<String>,
) -> Result<ExamApplied> {
    let path = PathBuf::from(deck_path);
    let deck = alix::deck::Deck::load(&path)?;
    let mut store = open_store(&path, &root_dir, device)?;
    let record = alix::store::SittingRecord {
        sections: outcome
            .sections
            .into_iter()
            .map(|section| alix::store::SectionRecord {
                card_ids: section.cards,
                pass: section.pass,
            })
            .collect(),
        full: outcome.full,
        pass: outcome.pass,
        partial: outcome.partial,
        total: outcome.total,
    };
    let unlocked = store.record_exam_result(&deck, &record, now_ms);
    store.save()?;
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    Ok(ExamApplied {
        unlocked,
        standing: alix::exam_status::standing(&store, deck_id, now_ms),
    })
}

// No retire cap on the phone: no accessor reads a resolved `retire_after`.
#[flutter_rust_bridge::frb(sync)]
pub fn apply_exam_remediation(
    deck_path: String,
    root_dir: String,
    cards_text: String,
    device: Option<String>,
) -> Result<u32> {
    let path = PathBuf::from(deck_path);
    let deck = alix::deck::Deck::load(&path)?;
    let fingerprints = deck.cards.iter().map(|card| card.block_fingerprint).collect();
    let deck_token = deck.deck_token.unwrap_or_default();
    let mut store = open_store(&path, &root_dir, device)?;
    let count = alix::store::store_remediation_cards(
        &mut store,
        Some(&path),
        &deck_token,
        &fingerprints,
        &cards_text,
        None,
    )?;
    Ok(count as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000_000;

    fn two_section_deck(root: &Path) -> PathBuf {
        let path = root.join("d.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: deck-d\nexam: source.txt\n---\n\
             # A\n\n## a1\nx\n<!-- id: card-a1 -->\n\n\
             # B\n\n## b1\nx\n<!-- id: card-b1 -->\n",
        )
        .unwrap();
        path
    }

    fn reopened(path: &Path, root: &Path) -> alix::store::Store {
        open_store(path, &root.to_string_lossy(), None).unwrap()
    }

    #[test]
    fn exam_ideas_round_trip_in_the_phone_store() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_section_deck(root);
        let deck_path = path.to_string_lossy().into_owned();
        let root_dir = root.to_string_lossy().into_owned();

        assert_eq!(None, exam_ideas_load(deck_path.clone(), root_dir.clone()).unwrap());

        let first = r#"{"version":1,"material":"xxh64-first","sections":[]}"#;
        exam_ideas_save(deck_path.clone(), root_dir.clone(), first.to_string()).unwrap();
        assert_eq!(
            Some(first.to_string()),
            exam_ideas_load(deck_path.clone(), root_dir.clone()).unwrap()
        );
        assert!(root.join(".alix/exam/deck-d.json").is_file());
        assert!(!root.join(".alix/exam/deck-d.json.tmp").exists());

        let second = r#"{"version":1,"material":"xxh64-second","sections":[]}"#;
        exam_ideas_save(deck_path.clone(), root_dir.clone(), second.to_string()).unwrap();
        assert_eq!(Some(second.to_string()), exam_ideas_load(deck_path, root_dir).unwrap());
    }

    fn root_arg(root: &Path) -> String {
        root.to_string_lossy().into_owned()
    }

    #[test]
    fn apply_exam_result_records_like_the_desktop() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_section_deck(root);
        let deck_path = path.to_string_lossy().into_owned();
        let section = |id: &str, pass| ExamSectionOutcome {
            cards: vec![id.to_string()],
            pass,
        };

        let applied = apply_exam_result(
            deck_path.clone(),
            root_arg(root),
            ExamOutcome {
                sections: vec![section("card-a1", true), section("card-b1", false)],
                full: true,
                pass: 1,
                partial: 0,
                total: 2,
            },
            NOW,
            None,
        )
        .unwrap();
        let store = reopened(&path, root);
        assert!(!applied.unlocked, "full sitting with a failed section: no unlock");
        assert_eq!(None, applied.standing, "neither passed nor mastered: no standing");
        assert_eq!(
            Some(alix::store::FullExamResult {
                at_ms: NOW,
                pass: 1,
                partial: 0,
                total: 2,
            }),
            store.full_exam_result("deck-d"),
            "a full sitting keeps its count"
        );
        assert_eq!(
            Some(false),
            store.get("card-b1").and_then(|c| c.exam_result).map(|r| r.pass),
            "each examined card carries its section's verdict"
        );
        assert_eq!(
            Some("card-b1".to_string()),
            exam_start_cards(deck_path.clone(), root_arg(root), true)
                .unwrap()
                .and_then(|ids| ids.into_iter().next()),
            "the filter names only the section that needs work"
        );

        let applied = apply_exam_result(
            deck_path.clone(),
            root_arg(root),
            ExamOutcome {
                sections: vec![section("card-b1", true)],
                full: false,
                pass: 1,
                partial: 0,
                total: 1,
            },
            NOW + 1,
            None,
        )
        .unwrap();
        let store = reopened(&path, root);
        assert!(applied.unlocked, "a filtered sitting passing the last section unlocks");
        assert!(
            applied.standing.as_deref().is_some_and(|line| line.starts_with("passed (")),
            "the phone words its own standing: {:?}",
            applied.standing
        );
        assert_eq!(Some(NOW + 1), store.deck_passed_at("deck-d"));
        assert_eq!(
            Some(NOW),
            store.full_exam_result("deck-d").map(|r| r.at_ms),
            "a filtered sitting leaves the full count alone"
        );
        assert_eq!(None, store.deck_mastered_at("deck-d"), "mastered needs a full 100% sitting");
    }

    #[test]
    fn exam_start_stamps_new_cards_and_a_full_sitting_names_no_cards() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_section_deck(root);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\n## b2\nnew\n")).unwrap();

        let cards = exam_start_cards(path.to_string_lossy().into_owned(), root_arg(root), false).unwrap();

        assert_eq!(None, cards, "a full sitting sends no card list");
        let deck = alix::deck::Deck::load(&path).unwrap();
        assert!(
            deck.cards.iter().all(|card| card.id().is_some()),
            "exam start stamps the new card"
        );
    }

    #[test]
    fn apply_exam_remediation_creates_personal_cards_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_section_deck(root);
        let deck_path = path.to_string_lossy().into_owned();
        let remediation = "## a1\nx\n\n## why b?\nbecause\n".to_string();

        let created =
            apply_exam_remediation(deck_path.clone(), root_arg(root), remediation.clone(), None).unwrap();
        let again = apply_exam_remediation(deck_path, root_arg(root), remediation, None).unwrap();

        assert_eq!((1, 0), (created, again), "the deck's own card and a repeat are skipped");
        assert_eq!(1, alix::personal::read(&path, "deck-d").cards.len());
    }

    #[test]
    fn apply_exam_remediation_skips_a_block_equal_to_a_cloze_block() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("d.md");
        let block = "## Complete the quote\nTo be or not to be\n<!-- blank: span hidden=\"be\" -->\n";
        std::fs::write(&path, format!("---\nformat-version: 1\nid: deck-d\n---\n{block}")).unwrap();
        alix::stamp::stamp_deck(&path).unwrap();

        let created = apply_exam_remediation(
            path.to_string_lossy().into_owned(),
            root_arg(root),
            block.to_string(),
            None,
        )
        .unwrap();

        assert_eq!(0, created, "the block key, not the cloze question, dedups");
        assert!(!alix::personal::sidecar_path(&path).exists(), "nothing reached disk");
    }
}
