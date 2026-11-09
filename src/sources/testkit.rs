use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;

use super::{
    MANIFEST, ROOTS,
    commands::{self, AddReport, AddRequest},
};
use crate::{config::ExamConfig, workspace::WorkspaceFiles};

pub(crate) const DECK: &str = "deck-fixture";
pub(crate) const TOPIC_A: &str = "topic-alpha";
pub(crate) const TOPIC_B: &str = "topic-beta";

pub(crate) const DECK_TEXT: &str = "---\nid: \"deck-fixture\"\n---\n# Alpha\n<!-- id: topic-alpha \
                                    -->\n\n## q1\na1\n\n# Alpine\n<!-- id: topic-alpine -->\n\n## \
                                    q2\na2\n\n# Beta\n<!-- id: topic-beta -->\n\n## q3\na3\n";

pub(crate) struct Workspace {
    temp: TempDir,
}

impl Workspace {
    pub(crate) fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("alix.toml"), "").unwrap();
        fs::create_dir_all(temp.path().join("decks")).unwrap();
        fs::create_dir_all(temp.path().join("live")).unwrap();
        let workspace = Self { temp };
        fs::write(workspace.deck(), DECK_TEXT).unwrap();
        workspace
    }

    pub(crate) fn root(&self) -> &Path {
        self.temp.path()
    }

    pub(crate) fn deck(&self) -> PathBuf {
        self.root().join("decks").join("d.md")
    }

    pub(crate) fn live(&self) -> PathBuf {
        self.root().join("live")
    }

    pub(crate) fn dir(&self) -> PathBuf {
        WorkspaceFiles::new(self.root()).sources_for(DECK)
    }

    pub(crate) fn write_live(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.live().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    pub(crate) fn add(&self, names: &[&str], topics: &[&str]) -> anyhow::Result<AddReport> {
        commands::add(
            AddRequest {
                deck: &self.deck(),
                files: names.iter().map(|name| self.live().join(name)).collect(),
                root: Some(("sdk".to_string(), self.live())),
                key: None,
                topics: topics.iter().map(|topic| topic.to_string()).collect(),
            },
            &ExamConfig::default(),
        )
    }

    pub(crate) fn add_bare(&self, files: &[PathBuf], topics: &[&str]) -> anyhow::Result<AddReport> {
        commands::add(
            AddRequest {
                deck: &self.deck(),
                files: files.to_vec(),
                root: None,
                key: None,
                topics: topics.iter().map(|topic| topic.to_string()).collect(),
            },
            &ExamConfig::default(),
        )
    }

    pub(crate) fn manifest_text(&self) -> String {
        fs::read_to_string(self.dir().join(MANIFEST)).unwrap()
    }

    pub(crate) fn rewrite_manifest(&self, from: &str, to: &str) {
        let text = self.manifest_text();
        assert!(text.contains(from), "manifest holds {from:?}: {text}");
        fs::write(self.dir().join(MANIFEST), text.replacen(from, to, 1)).unwrap();
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let read = |name: &str| fs::read(self.dir().join(name)).ok();
        let names = fs::read_dir(self.dir())
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        Snapshot {
            manifest: read(MANIFEST),
            roots: read(ROOTS),
            names,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) manifest: Option<Vec<u8>>,
    pub(crate) roots: Option<Vec<u8>>,
    pub(crate) names: BTreeSet<String>,
}

/// A fresh workspace root and its `decks/` folder.
pub(crate) fn workspace_decks() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("alix.toml"), "").unwrap();
    let decks = temp.path().join("decks");
    fs::create_dir_all(&decks).unwrap();
    (temp, decks)
}

/// Stamps the member deck at `path` and binds one fresh source to every one of
/// its topics that holds a card, so it has an exam.
pub(crate) fn sourced(path: PathBuf) -> PathBuf {
    crate::stamp::stamp_deck(&path).unwrap();
    let deck = crate::deck::Deck::load(&path).unwrap();
    let root = crate::workspace::root_for_deck(&path).expect("a workspace member");
    let live = root.join("live");
    fs::create_dir_all(&live).unwrap();
    let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
    let file = live.join(format!("{stem}.txt"));
    fs::write(&file, format!("the source of {stem}\n")).unwrap();
    commands::add(
        AddRequest {
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
        },
        &ExamConfig::default(),
    )
    .unwrap();
    path
}
