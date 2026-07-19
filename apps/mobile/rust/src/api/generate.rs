//! Placing a remotely generated deck on-device: the phone half of remote
//! generate. The server half (T2.1) returns deck text plus a suggested file
//! name and never places a file itself (the iron rule for that endpoint) --
//! placement is always a local, on-device decision. This module makes that
//! decision: it reuses the lean `alix::library::place_deck` writer (the same
//! tail `alix generate`, `alix deck import`, and the web equivalents share)
//! so a generated deck lands exactly like a locally authored one, and it
//! contributes the one thing the server can't do for it -- stemming the
//! suggested name against whatever is already in the phone's decks folder so
//! a generated deck never silently overwrites an existing file.

use std::path::Path;

use anyhow::Result;

/// Writes `text` into `decks_dir`, choosing a collision-free name from
/// `filename`: strip a trailing `.txt` to get the base stem, then try
/// `<stem>.txt`, `<stem>-2.txt`, `<stem>-3.txt`, ... until one is free. A
/// single-user phone has no concurrent writer, so an exists-check loop
/// followed by one `place_deck` call is enough; `place_deck` re-checks and
/// still bails on a genuine TOCTOU race, which is left to propagate as an
/// error rather than retried. Returns the actual written file name (never
/// the pre-checked candidate) so Dart can show "saved as <name>" and refresh
/// the picker. A freshly generated deck that fails to parse is still saved
/// by `place_deck` (lenient by design, fixable by hand) -- this function
/// does not fail or otherwise surface that case, matching the spec's plain
/// `String` return.
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
    while dir.join(format!("{candidate}.txt")).exists() {
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

        // `place_deck` (the shared writer this delegates to) always normalizes
        // to `.md`, regardless of the suggested name's own suffix.
        assert_eq!(name, "topic.md");
        let deck = alix::deck::Deck::load(dir.path().join("topic.md")).unwrap();
        assert_eq!(deck.cards.len(), 2);
    }

    // KNOWN BUG (not fixed here — production code, out of this task's scope):
    // this function's own collision loop probes `<candidate>.txt` for an
    // existing file (line 39), but the writer it delegates to
    // (`alix::library::place_deck`) always writes `.md` (src/library.rs:35).
    // Since every real deck on disk is `.md` today, the probe never sees a
    // real collision; `place_deck`'s own `path.exists()` guard then hard-errors
    // instead of the intended `-2`/`-3` fallback. The first collision below
    // still resolves by accident (the pre-existing file is written directly
    // as `foo.txt`, which the stale probe does see), but the second one hits
    // the freshly-written `foo-2.md` and errors, so this test still fails
    // after fixing only its fixture content/expectations. Left failing
    // (not weakened) so it keeps pointing at the real gap.
    #[test]
    fn a_colliding_filename_stems_to_dash_2_leaving_the_original_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let original = "# Original\n\n## q\na\n";
        std::fs::write(dir.path().join("foo.txt"), original).unwrap();

        let name = apply_generated_deck(
            dir_str(&dir),
            "foo.txt".to_string(),
            "## new\nb\n".to_string(),
        )
        .unwrap();
        assert_eq!(name, "foo-2.md");
        assert_eq!(
            std::fs::read(dir.path().join("foo.txt")).unwrap(),
            original.as_bytes(),
            "the pre-existing deck must survive byte for byte"
        );

        let name = apply_generated_deck(
            dir_str(&dir),
            "foo.txt".to_string(),
            "## newer\nc\n".to_string(),
        )
        .unwrap();
        assert_eq!(name, "foo-3.md");
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
