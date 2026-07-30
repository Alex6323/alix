#![cfg(unix)]

use std::process::Command;

use tempfile::TempDir;

#[test]
fn reset_orphans_clears_a_deleted_last_decks_progress() {
    let dir = TempDir::new().unwrap();
    let progress = alix::state::UserFiles::new(dir.path()).progress_for("deck-deleted");
    let mut store = alix::store::Store::open_deck(&progress, "deck-deleted", "deleted.md").unwrap();
    store.get_or_insert("card-deleted1", 0);
    store.save().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_alix"))
        .args([
            "reset",
            "--orphans",
            dir.path().to_str().unwrap(),
            "--yes",
            "--store",
            dir.path().to_str().unwrap(),
        ])
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .env("XDG_DATA_HOME", dir.path())
        .output()
        .unwrap();

    let after = std::fs::read_to_string(&progress).unwrap();
    assert!(
        !after.contains("card-deleted1"),
        "the deleted last deck's progress survived; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success());
}
