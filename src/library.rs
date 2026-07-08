//! Placing new decks into the library — the shared write tail of `alix
//! generate`, `alix deck import`, and the web equivalents. Validation is
//! lenient on purpose: a generated deck that does not parse yet is still
//! saved (fixable by hand) and the problem is reported, not swallowed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::parser;

/// What [`place_deck`] wrote: where it landed, how many cards parsed, and the
/// parse problem when the text is not a valid deck yet.
#[derive(Debug)]
pub struct Placed {
    pub path: PathBuf,
    pub cards: usize,
    pub parse_error: Option<String>,
}

/// Writes `text` into `dir` as `<name>.txt` (atomically: `.tmp` + rename).
/// Only `name`'s file-name component is used, so an uploaded name can't
/// traverse; a collision is an error — the caller decides about overwriting.
pub fn place_deck(dir: &Path, name: &str, text: &str) -> Result<Placed> {
    let stem = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deck");
    let stem = stem.strip_suffix(".txt").unwrap_or(stem);
    let file = format!("{stem}.txt");
    let path = dir.join(&file);
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    // The file name is part of every card's identity hash — parse against it.
    let parsed = parser::parse_str(&file, text);
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let body = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    };
    let tmp = dir.join(format!(".{file}.tmp"));
    std::fs::write(&tmp, body).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(match parsed {
        Ok(cards) => Placed {
            path,
            cards: cards.len(),
            parse_error: None,
        },
        Err(e) => Placed {
            path,
            cards: 0,
            parse_error: Some(format!("{e:#}")),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placing_a_valid_deck_writes_it_and_counts_cards() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "rust", "# q\n  a\n").unwrap();
        assert_eq!(dir.path().join("rust.txt"), p.path);
        assert_eq!(1, p.cards);
        assert!(p.parse_error.is_none());
        assert!(p.path.exists());
    }

    #[test]
    fn a_parse_problem_still_writes_the_deck_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        // A front with no answer line is invalid but must not be discarded.
        let p = place_deck(dir.path(), "broken.txt", "# q with no answer\n").unwrap();
        assert!(p.path.exists());
        assert!(p.parse_error.is_some());
    }

    #[test]
    fn a_name_collision_errors_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rust.txt"), "original").unwrap();
        let err = place_deck(dir.path(), "rust", "# q\n  a\n").unwrap_err();
        assert!(format!("{err:#}").contains("already exists"), "{err:#}");
        assert_eq!(
            "original",
            std::fs::read_to_string(dir.path().join("rust.txt")).unwrap()
        );
    }

    #[test]
    fn an_uploaded_name_cannot_traverse_out_of_the_dir() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "../../evil", "# q\n  a\n").unwrap();
        assert!(p.path.starts_with(dir.path()), "{}", p.path.display());
    }
}
