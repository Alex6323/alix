//! A personal sidecar is never offered as a deck of its own, by any route that
//! discovers decks in a folder.

use std::path::Path;

use alix::{cache::DeckCache, config::ReviewConfig, picker, recent::RecentDecks, workspace};

const DECK: &str = "---\nformat-version: 1\nid: deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\n---\n\n\
## what is a subjunctive <!-- id: card-4b7k2m9q1x5z8t3v6n0d4f7h2j -->\na mood\n";

const SIDECAR: &str = "---\nformat-version: 1\n\
for: deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\n---\n\n\
<!-- note: card-4b7k2m9q1x5z8t3v6n0d4f7h2j -->\n\
> my own note\n";

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("spanish.md"), DECK).unwrap();
    std::fs::write(dir.path().join("spanish.personal.md"), SIDECAR).unwrap();
    dir
}

/// Every name a folder listing can offer the user, from every route that builds
/// one. A new lister joins this list; it does not get its own test.
fn every_discovered_name(dir: &Path) -> Vec<(&'static str, Vec<String>)> {
    let found = workspace::classify_deck_files(dir).unwrap();
    let mut cache = DeckCache::default();
    let recent = RecentDecks::default();
    vec![
        (
            "listing::list_root",
            alix::listing::list_root(dir, &ReviewConfig::default(), 0)
                .iter()
                .map(|deck| deck.title.clone())
                .collect(),
        ),
        (
            "workspace::classify_deck_files (initialized)",
            file_names(&found.initialized),
        ),
        (
            "workspace::classify_deck_files (uninitialized)",
            file_names(&found.uninitialized),
        ),
        (
            "picker::catalog",
            picker::catalog(dir, &recent, &mut cache)
                .unwrap()
                .iter()
                .map(|entry| entry.name.clone())
                .collect(),
        ),
    ]
}

fn file_names(paths: &[std::path::PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
        .map(str::to_string)
        .collect()
}

#[test]
fn no_discovery_route_offers_a_sidecar_as_a_deck() {
    let dir = fixture();
    for (route, names) in every_discovered_name(dir.path()) {
        assert!(
            !names.iter().any(|name| name.contains("personal")),
            "{route} offered the sidecar as a deck: {names:?}"
        );
    }
}

#[test]
fn excluding_the_sidecar_does_not_hide_the_deck_beside_it() {
    let dir = fixture();
    for (route, names) in every_discovered_name(dir.path()) {
        if route.ends_with("(uninitialized)") {
            continue;
        }
        assert!(
            names.iter().any(|name| name.starts_with("spanish")),
            "{route} lost the authored deck: {names:?}"
        );
    }
}

/// One physical workspace reached under two names is one place to study: both
/// spellings open the same decks and write the same progress, so a listing that
/// offers both offers a choice that does not exist.
#[cfg(unix)]
#[test]
fn no_folder_listing_offers_one_physical_workspace_twice() {
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("nested");
    let decks = nested.join(workspace::DECKS);
    std::fs::create_dir_all(&decks).unwrap();
    std::fs::write(nested.join(workspace::MANIFEST), "title = \"Nested\"\n").unwrap();
    std::fs::write(decks.join("spanish.md"), DECK).unwrap();
    std::os::unix::fs::symlink(&nested, root.path().join("alias")).unwrap();
    std::os::unix::fs::symlink(decks.join("spanish.md"), decks.join("spanish-alias.md")).unwrap();

    let listed = alix::listing::list_root(root.path(), &ReviewConfig::default(), 0);
    assert_eq!(
        1,
        listed.len(),
        "listing::list_root offered the same workspace under both names: {:?}",
        listed
            .iter()
            .map(|deck| deck.title.clone())
            .collect::<Vec<_>>()
    );

    let mut cache = DeckCache::default();
    let catalog = picker::catalog(root.path(), &RecentDecks::default(), &mut cache).unwrap();
    let folders: Vec<String> = catalog
        .iter()
        .filter(|entry| entry.is_workspace)
        .map(|entry| entry.name.clone())
        .collect();
    assert_eq!(
        1,
        folders.len(),
        "picker::catalog offered the same workspace under both names: {folders:?}"
    );

    let members: Vec<String> = catalog
        .iter()
        .filter(|entry| entry.is_workspace)
        .flat_map(|entry| entry.members.iter().map(|member| member.name.clone()))
        .collect();
    assert_eq!(
        1,
        members.len(),
        "picker::catalog offered the same deck file under both names: {members:?}"
    );

    let listed_members =
        alix::listing::list_members(root.path(), &nested, &ReviewConfig::default(), 0);
    assert_eq!(
        1,
        listed_members.len(),
        "listing::list_members offered the same deck file under both names: {:?}",
        listed_members
            .iter()
            .map(|deck| deck.title.clone())
            .collect::<Vec<_>>()
    );
}
