use std::path::Path;

use anyhow::Result;

#[flutter_rust_bridge::frb(sync)]
pub fn apply_generated_deck(decks_dir: String, filename: String, text: String) -> Result<String> {
    let dir = Path::new(&decks_dir);
    let raw = Path::new(&filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deck");
    let stem = raw.strip_suffix(".txt").unwrap_or(raw);

    let mut candidate = stem.to_string();
    let mut n = 2;
    while dir.join(format!("{candidate}.md")).exists() {
        candidate = format!("{stem}-{n}");
        n += 1;
    }

    let placed = alix::library::place_deck(dir, &candidate, &text)?;
    Ok(placed
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&candidate)
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_str(dir: &tempfile::TempDir) -> String {
        dir.path().to_string_lossy().into_owned()
    }

    #[test]
    fn apply_generated_deck_writes_the_stem_and_parses_the_expected_card_count() {
        let dir = tempfile::tempdir().unwrap();
        let text = "## q1\na1\n\n## q2\na2\n";
        let name =
            apply_generated_deck(dir_str(&dir), "topic.txt".to_string(), text.to_string()).unwrap();

        assert_eq!(name, "topic.md");
        let deck = alix::deck::Deck::load(dir.path().join("topic.md")).unwrap();
        assert_eq!(deck.cards.len(), 2);
    }

    #[test]
    fn a_filename_without_the_txt_suffix_still_lands_on_stem_md() {
        let dir = tempfile::tempdir().unwrap();
        let name =
            apply_generated_deck(dir_str(&dir), "topic".to_string(), "## q\na\n".to_string())
                .unwrap();
        assert_eq!(name, "topic.md");
    }

    #[test]
    fn a_stem_containing_a_dot_is_not_double_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let name = apply_generated_deck(
            dir_str(&dir),
            "v2.1.txt".to_string(),
            "## q\na\n".to_string(),
        )
        .unwrap();
        assert_eq!(name, "v2.1.md");
    }

    #[test]
    fn text_without_a_trailing_newline_gets_one() {
        let dir = tempfile::tempdir().unwrap();
        let name = apply_generated_deck(
            dir_str(&dir),
            "nolf.txt".to_string(),
            "## q\na".to_string(),
        )
        .unwrap();
        let written = std::fs::read_to_string(dir.path().join(&name)).unwrap();
        assert!(written.ends_with('\n'));
    }
}
