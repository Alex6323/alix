use std::path::{Path, PathBuf};

/// Writes a member deck into the workspace at `workspace`, stamps it, and
/// binds one fresh source to every topic that holds a card, so it has an exam.
pub(crate) fn sourced_member(workspace: &Path, name: &str, text: &str) -> PathBuf {
    std::fs::create_dir_all(workspace.join("decks")).unwrap();
    if !workspace.join("alix.toml").exists() {
        std::fs::write(workspace.join("alix.toml"), "").unwrap();
    }
    let path = workspace.join("decks").join(name);
    std::fs::write(&path, text).unwrap();
    alix::stamp::stamp_deck(&path).unwrap();
    let deck = alix::deck::Deck::load(&path).unwrap();
    let live = workspace.join("live");
    std::fs::create_dir_all(&live).unwrap();
    let file = live.join(format!("{name}.txt"));
    std::fs::write(&file, format!("the source of {name}\n")).unwrap();
    alix::sources::commands::add(alix::sources::commands::AddRequest {
        deck: &path,
        files: vec![file],
        root: Some(("live".to_string(), live)),
        key: None,
        topics: deck
            .topic_ids()
            .into_iter()
            .enumerate()
            .filter(|(at, _)| deck.cards.iter().any(|card| card.topic == *at))
            .filter_map(|(_, id)| id)
            .collect(),
    })
    .unwrap();
    path
}
