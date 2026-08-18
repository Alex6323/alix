#![cfg(unix)]

use std::process::Command;

use tempfile::TempDir;

#[test]
fn reset_orphans_does_not_prune_progress_for_a_malformed_live_deck() {
    let dir = TempDir::new().unwrap();
    let deck = dir.path().join("live.md");
    std::fs::write(
        &deck,
        "---\nformat-version: 1\nid: deck-live\n---\n## question <!-- id: card-live1 -->\nanswer\n",
    )
    .unwrap();
    let mut store = alix::state::open_store(&deck, dir.path()).unwrap();
    store.get_or_insert("card-live1");
    store.save().unwrap();
    let progress = store.path().to_path_buf();
    let before = std::fs::read_to_string(&progress).unwrap();

    std::fs::write(
        &deck,
        "---\nformat-version: 1\nid: deck-live\n---\n## question <!-- id: card-live1 -->\n",
    )
    .unwrap();
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

    assert_eq!(
        before,
        std::fs::read_to_string(&progress).unwrap(),
        "progress for the malformed live deck was pruned; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
