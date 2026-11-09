use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    DisplayPath, MANIFEST, MANIFEST_VERSION, SourceError, copy_name, io_error, is_copy_name,
    is_source_id, line_count, parse_binding, read_versioned, source_id,
};
use crate::{deck::Deck, workspace::WorkspaceFiles};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub sources: BTreeMap<String, Row>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub path: String,
    #[serde(rename = "bound-to")]
    pub bound_to: Vec<String>,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Option<Self>, SourceError> {
        read_versioned(
            &dir.join(MANIFEST),
            |manifest: &Self| manifest.version,
            MANIFEST_VERSION,
        )
    }

    pub fn validate(&self) -> Result<(), SourceError> {
        let mut paths: BTreeSet<&str> = BTreeSet::new();
        for (id, row) in &self.sources {
            if !is_source_id(id) {
                return Err(SourceError::BadId(id.clone()));
            }
            display_of(id, row)?;
            if !paths.insert(&row.path) {
                return Err(SourceError::DuplicatePath(row.path.clone()));
            }
            if row.bound_to.is_empty() {
                return Err(SourceError::EmptyBinding(id.clone()));
            }
            let mut topics: BTreeSet<String> = BTreeSet::new();
            for entry in &row.bound_to {
                let binding = parse_binding(id, entry)?;
                if !topics.insert(binding.topic.clone()) {
                    return Err(SourceError::DuplicateTopicEntry {
                        id: id.clone(),
                        topic: binding.topic,
                    });
                }
            }
        }
        Ok(())
    }
}

fn display_of(id: &str, row: &Row) -> Result<DisplayPath, SourceError> {
    row.path.parse().map_err(|error| match error {
        SourceError::BadPath { path, problem, .. } => SourceError::BadPath {
            id: Some(id.to_string()),
            path,
            problem,
        },
        other => other,
    })
}

fn topic_of(entry: &str) -> &str {
    entry.rsplit_once('|').map_or(entry, |(_, topic)| topic)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckSources {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

pub fn load_verified(
    workspace_root: &Path,
    deck_id: &str,
) -> Result<Option<DeckSources>, SourceError> {
    let dir = WorkspaceFiles::new(workspace_root).sources_for(deck_id);
    let Some(manifest) = Manifest::load(&dir)? else {
        return Ok(None);
    };
    manifest.validate()?;
    verify_copies(&dir, &manifest)?;
    Ok(Some(DeckSources { dir, manifest }))
}

fn verify_copies(dir: &Path, manifest: &Manifest) -> Result<(), SourceError> {
    let names = read_names(dir).map_err(io_error(dir))?;
    for (id, row) in &manifest.sources {
        let expected = copy_name(id, &display_of(id, row)?);
        let path = dir.join(&expected);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => return Err(SourceError::NotRegularFile(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let prefix = format!("{id}.");
                return Err(match names.iter().find(|name| name.starts_with(&prefix)) {
                    Some(found) => SourceError::WrongName {
                        expected,
                        found: found.clone(),
                    },
                    None => SourceError::MissingCopy(id.clone()),
                });
            }
            Err(error) => return Err(io_error(&path)(error)),
        }
        let bytes = std::fs::read(&path).map_err(io_error(&path))?;
        if bytes.is_empty() {
            return Err(SourceError::EmptyCopy(path));
        }
        if source_id(&bytes) != *id {
            return Err(SourceError::DigestMismatch(path));
        }
        let lines = line_count(&bytes);
        for entry in &row.bound_to {
            let binding = parse_binding(id, entry)?;
            if binding.ranges.iter().any(|&(_, end)| end as usize > lines) {
                return Err(SourceError::RangeOutOfBounds {
                    id: id.clone(),
                    entry: entry.clone(),
                    lines,
                });
            }
        }
    }
    Ok(())
}

fn read_names(dir: &Path) -> std::io::Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(dir)? {
        if let Some(name) = entry?.file_name().to_str() {
            names.insert(name.to_string());
        }
    }
    Ok(names)
}

fn strays(dir: &Path, manifest: Option<&Manifest>) -> Vec<String> {
    let named: BTreeSet<String> = manifest
        .map(|manifest| {
            manifest
                .sources
                .iter()
                .filter_map(|(id, row)| Some(copy_name(id, &display_of(id, row).ok()?)))
                .collect()
        })
        .unwrap_or_default();
    read_names(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|name| is_copy_name(name) && !named.contains(name))
        .collect()
}

pub fn stray_copies(dir: &Path) -> Vec<String> {
    strays(dir, Manifest::load(dir).ok().flatten().as_ref())
}

impl DeckSources {
    pub fn residue(&self) -> Vec<String> {
        strays(&self.dir, Some(&self.manifest))
    }

    pub fn validate_bindings(&self, deck: &Deck) -> Result<(), SourceError> {
        let ids: Vec<String> = deck.topic_ids().into_iter().flatten().collect();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        if let Some(repeated) = ids.iter().find(|id| !seen.insert(id.as_str())) {
            return Err(SourceError::DuplicateTopicId(repeated.clone()));
        }
        for (id, row) in &self.manifest.sources {
            for entry in &row.bound_to {
                let topic = topic_of(entry);
                if !seen.contains(topic) {
                    return Err(SourceError::UnknownTopic {
                        id: id.clone(),
                        topic: topic.to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn sources_of(&self, topic_id: &str) -> Vec<(&str, &Row)> {
        let mut rows: Vec<(&str, &Row)> = self
            .manifest
            .sources
            .iter()
            .filter(|(_, row)| row.bound_to.iter().any(|entry| topic_of(entry) == topic_id))
            .map(|(id, row)| (id.as_str(), row))
            .collect();
        rows.sort_by(|a, b| a.1.path.cmp(&b.1.path));
        rows
    }

    pub fn sourced_topics(&self) -> BTreeSet<String> {
        self.manifest
            .sources
            .values()
            .flat_map(|row| row.bound_to.iter().map(|entry| topic_of(entry).to_string()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
    };

    use tempfile::TempDir;

    use super::*;
    use crate::{
        deck::Deck,
        sources::{MANIFEST, ROOTS, SourceError, source_id},
        workspace::WorkspaceFiles,
    };

    const DECK: &str = "deck-fixture";
    const A: &[u8] = b"fn main() {}\n";
    const B: &[u8] = b"# RFC 9110\nHTTP Semantics\r\n";

    struct Fixture {
        root: TempDir,
        dir: PathBuf,
        a: String,
        b: String,
    }

    impl Fixture {
        fn manifest_text(&self) -> String {
            let (a, b) = (&self.a, &self.b);
            format!(
                "version = 1\n\n[sources]\n{a} = {{ path = \"sdk:src/main.rs\", bound-to = \
                 [\"deck-fixture\"] }}\n{b} = {{ path = \"dl:rfc9110.txt\", bound-to = \
                 [\"topic-aaaa\", \"1-2|deck-fixture\"] }}\n"
            )
        }

        fn rewrite_manifest(&self, from: &str, to: &str) {
            let text = self.manifest_text();
            assert!(text.contains(from), "fixture holds {from:?}");
            fs::write(self.dir.join(MANIFEST), text.replacen(from, to, 1)).unwrap();
        }

        fn copy_a(&self) -> PathBuf {
            self.dir.join(format!("{}.rs", self.a))
        }

        fn load(&self) -> Result<Option<DeckSources>, SourceError> {
            load_verified(self.root.path(), DECK)
        }
    }

    fn fixture() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("sources").join(DECK);
        fs::create_dir_all(&dir).unwrap();
        let fixture = Fixture {
            dir,
            a: source_id(A),
            b: source_id(B),
            root,
        };
        fs::write(fixture.copy_a(), A).unwrap();
        fs::write(fixture.dir.join(format!("{}.txt", fixture.b)), B).unwrap();
        fs::write(fixture.dir.join(MANIFEST), fixture.manifest_text()).unwrap();
        fs::write(
            fixture.dir.join(ROOTS),
            "version = 1\n\n[roots]\nsdk = \"/\"\n",
        )
        .unwrap();
        fixture
    }

    #[test]
    fn a_valid_folder_loads_with_no_residue_and_an_absent_one_is_none() {
        let fixture = fixture();
        assert_eq!(
            fixture.dir,
            WorkspaceFiles::new(fixture.root.path()).sources_for(DECK),
            "the folder is sources/<deck-id> at the workspace root"
        );
        let sources = fixture.load().unwrap().expect("a manifest is present");
        assert_eq!(fixture.dir, sources.dir);
        assert_eq!(2, sources.manifest.sources.len());
        assert_eq!(Vec::<String>::new(), sources.residue(), "no stray copy");
        assert_eq!(
            Vec::<String>::new(),
            stray_copies(&fixture.dir),
            "stray_copies agrees with residue"
        );

        assert!(
            load_verified(fixture.root.path(), "deck-other")
                .unwrap()
                .is_none(),
            "no folder: None"
        );
        fs::remove_file(fixture.dir.join(MANIFEST)).unwrap();
        assert!(fixture.load().unwrap().is_none(), "no manifest: None");
        assert_eq!(
            vec![format!("{}.rs", fixture.a), format!("{}.txt", fixture.b)],
            stray_copies(&fixture.dir),
            "without a manifest every copy is stray"
        );
        assert_eq!(
            Vec::<String>::new(),
            stray_copies(&fixture.root.path().join("sources").join("deck-other")),
            "no folder: no strays"
        );
    }

    #[test]
    fn load_verified_fails_closed_on_every_integrity_fault() {
        type Fault = (
            &'static str,
            fn(&Fixture),
            fn(&Fixture, &SourceError) -> bool,
        );
        let mut faults: Vec<Fault> = vec![
            (
                "version 2",
                |f| f.rewrite_manifest("version = 1", "version = 2"),
                |_, e| matches!(e, SourceError::Version { found: 2 }),
            ),
            (
                "key not an id",
                |f| f.rewrite_manifest(&f.a, "source-not-an-id"),
                |_, e| matches!(e, SourceError::BadId(id) if id == "source-not-an-id"),
            ),
            (
                "path without key",
                |f| f.rewrite_manifest("sdk:src/main.rs", "src/main.rs"),
                |f, e| {
                    matches!(e, SourceError::BadPath { id: Some(id), path, problem: "has no key" }
                        if *id == f.a && path == "src/main.rs")
                },
            ),
            (
                "duplicate path",
                |f| f.rewrite_manifest("dl:rfc9110.txt", "sdk:src/main.rs"),
                |_, e| matches!(e, SourceError::DuplicatePath(p) if p == "sdk:src/main.rs"),
            ),
            (
                "empty bound-to",
                |f| f.rewrite_manifest("bound-to = [\"deck-fixture\"]", "bound-to = []"),
                |f, e| matches!(e, SourceError::EmptyBinding(id) if *id == f.a),
            ),
            (
                "unknown field in a row",
                |f| {
                    f.rewrite_manifest(
                        "path = \"sdk:src/main.rs\"",
                        "path = \"sdk:src/main.rs\", note = 1",
                    )
                },
                |_, e| matches!(e, SourceError::Toml { .. }),
            ),
            (
                "unknown top-level field",
                |f| f.rewrite_manifest("version = 1\n", "version = 1\nnote = 1\n"),
                |_, e| matches!(e, SourceError::Toml { .. }),
            ),
            (
                "copy with one byte flipped",
                |f| {
                    let mut bytes = A.to_vec();
                    bytes[0] ^= 1;
                    fs::write(f.copy_a(), bytes).unwrap();
                },
                |f, e| matches!(e, SourceError::DigestMismatch(p) if *p == f.copy_a()),
            ),
            (
                "copy renamed to a wrong extension",
                |f| fs::rename(f.copy_a(), f.dir.join(format!("{}.md", f.a))).unwrap(),
                |f, e| {
                    matches!(e, SourceError::WrongName { expected, found }
                        if *expected == format!("{}.rs", f.a) && *found == format!("{}.md", f.a))
                },
            ),
            (
                "copy missing",
                |f| fs::remove_file(f.copy_a()).unwrap(),
                |f, e| matches!(e, SourceError::MissingCopy(id) if *id == f.a),
            ),
            (
                "copy is a directory",
                |f| {
                    fs::remove_file(f.copy_a()).unwrap();
                    fs::create_dir(f.copy_a()).unwrap();
                },
                |f, e| matches!(e, SourceError::NotRegularFile(p) if *p == f.copy_a()),
            ),
            (
                "empty copy",
                |f| {
                    let empty = source_id(b"");
                    fs::write(f.dir.join(format!("{empty}.txt")), b"").unwrap();
                    f.rewrite_manifest(
                        "[sources]\n",
                        &format!(
                            "[sources]\n{empty} = {{ path = \"sdk:empty.txt\", bound-to = \
                             [\"deck-fixture\"] }}\n"
                        ),
                    );
                },
                |f, e| {
                    matches!(e, SourceError::EmptyCopy(p)
                        if *p == f.dir.join(format!("{}.txt", source_id(b""))))
                },
            ),
            (
                "range beyond the copy",
                |f| f.rewrite_manifest("1-2|deck-fixture", "1-3|deck-fixture"),
                |f, e| {
                    matches!(e, SourceError::RangeOutOfBounds { id, entry, lines: 2 }
                        if *id == f.b && entry == "1-3|deck-fixture")
                },
            ),
            (
                "topic bound twice on one row",
                |f| f.rewrite_manifest("\"topic-aaaa\", ", "\"deck-fixture\", "),
                |f, e| {
                    matches!(e, SourceError::DuplicateTopicEntry { id, topic }
                        if *id == f.b && topic == "deck-fixture")
                },
            ),
            (
                "malformed binding",
                |f| f.rewrite_manifest("1-2|deck-fixture", "2-1|deck-fixture"),
                |f, e| {
                    matches!(e, SourceError::BadBinding { id, entry, problem: "has a malformed range" }
                        if *id == f.b && entry == "2-1|deck-fixture")
                },
            ),
        ];
        #[cfg(unix)]
        faults.push((
            "copy replaced by a symlink to an outside file with the same bytes",
            |f| {
                let outside = f.root.path().join("outside.rs");
                fs::write(&outside, A).unwrap();
                fs::remove_file(f.copy_a()).unwrap();
                std::os::unix::fs::symlink(&outside, f.copy_a()).unwrap();
            },
            |f, e| matches!(e, SourceError::NotRegularFile(p) if *p == f.copy_a()),
        ));

        for (name, apply, expected) in faults {
            let fixture = fixture();
            apply(&fixture);
            let error = match fixture.load() {
                Err(error) => error,
                Ok(loaded) => panic!("{name}: loaded {loaded:?} instead of failing"),
            };
            assert!(
                expected(&fixture, &error),
                "{name}: unexpected error {error:?}"
            );
            assert!(!error.to_string().is_empty(), "{name}: the error has text");
        }
    }

    #[test]
    fn a_stray_copy_loads_and_shows_as_residue() {
        let fixture = fixture();
        let stray = format!("{}.txt", source_id(b"other\n"));
        fs::write(fixture.dir.join(&stray), b"other\n").unwrap();
        fs::write(fixture.dir.join("notes.txt"), b"not copy-named").unwrap();
        let sources = fixture.load().unwrap().expect("a stray is never an error");
        assert_eq!(vec![stray.clone()], sources.residue());
        assert_eq!(vec![stray], stray_copies(&fixture.dir));
    }

    fn write_deck(dir: &Path, text: &str) -> Deck {
        let path = dir.join("d.md");
        fs::write(&path, format!("---\nid: \"{DECK}\"\n---\n{text}")).unwrap();
        Deck::load(&path).unwrap()
    }

    fn sources_with(rows: &[(&str, &str, &[&str])]) -> DeckSources {
        DeckSources {
            dir: PathBuf::new(),
            manifest: Manifest {
                version: MANIFEST_VERSION,
                sources: rows
                    .iter()
                    .map(|(id, path, bound)| {
                        (
                            id.to_string(),
                            Row {
                                path: path.to_string(),
                                bound_to: bound.iter().map(|b| b.to_string()).collect(),
                            },
                        )
                    })
                    .collect(),
            },
        }
    }

    #[test]
    fn validate_bindings_accepts_the_decks_topics_and_refuses_unknown_and_duplicated_ids() {
        let dir = tempfile::tempdir().unwrap();
        let deck = write_deck(
            dir.path(),
            "## lead\nx\n# A\n<!-- id: topic-aaaa -->\n## q\na\n# B\n<!-- id: topic-bbbb -->\n",
        );
        let a = source_id(A);
        let b = source_id(B);
        let bound = sources_with(&[
            (&a, "sdk:src/main.rs", &["deck-fixture", "3|topic-bbbb"]),
            (&b, "dl:rfc9110.txt", &["topic-aaaa", "1-2|deck-fixture"]),
        ]);
        bound
            .validate_bindings(&deck)
            .expect("the deck id and both topics are bindable");

        let unknown = sources_with(&[(&b, "dl:rfc9110.txt", &["topic-aaaa", "1-2|topic-zzzz"])]);
        assert!(
            matches!(
                unknown.validate_bindings(&deck),
                Err(SourceError::UnknownTopic { id, topic }) if id == b && topic == "topic-zzzz"
            ),
            "a binding to a topic the deck lacks is refused"
        );

        let other_deck = sources_with(&[(&a, "sdk:src/main.rs", &["deck-other"])]);
        assert!(
            matches!(
                other_deck.validate_bindings(&deck),
                Err(SourceError::UnknownTopic { topic, .. }) if topic == "deck-other"
            ),
            "another deck's id names no topic here"
        );

        let repeated = write_deck(
            dir.path(),
            "# A\n<!-- id: topic-aaaa -->\n## q\na\n# B\n<!-- id: topic-aaaa -->\n## r\nb\n",
        );
        assert!(
            matches!(
                bound.validate_bindings(&repeated),
                Err(SourceError::DuplicateTopicId(topic)) if topic == "topic-aaaa"
            ),
            "a duplicated topic id refuses the bindings"
        );
    }

    #[test]
    fn sources_of_sorts_by_display_path_bytes_and_sourced_topics_collects_every_binding() {
        let (x, y, z) = (source_id(b"x"), source_id(b"y"), source_id(b"z"));
        let sources = sources_with(&[
            (&x, "sdk:readme.md", &["topic-aaaa"]),
            (&y, "sdk:README.md", &["2-3|topic-aaaa", "topic-bbbb"]),
            (&z, "dl:z.txt", &["deck-fixture"]),
        ]);
        let of_a: Vec<&str> = sources
            .sources_of("topic-aaaa")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(vec![y.as_str(), x.as_str()], of_a, "`R` before `r`");
        assert!(sources.sources_of("topic-zzzz").is_empty());
        assert_eq!(
            ["deck-fixture", "topic-aaaa", "topic-bbbb"]
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<String>>(),
            sources.sourced_topics()
        );
    }

    #[test]
    fn the_adr_example_manifest_parses_and_validates() {
        let text = "version = 1\n\n[sources]\n\
                    source-18pgda5qxze0435914h3awa04ej3cmvhk8hvwef6g09ybdv1czkc = { path = \
                    \"sdk:src/lib.rs\", bound-to = [\"topic-c9ns6n7s0k9hzm7eb0e7hqn35j\", \
                    \"100-400,512|topic-3whzpgpjp1z1rsfgcvxx4984hn\"] }\n";
        let manifest: Manifest = toml::from_str(text).unwrap();
        manifest.validate().unwrap();
        let row = &manifest.sources["source-18pgda5qxze0435914h3awa04ej3cmvhk8hvwef6g09ybdv1czkc"];
        assert_eq!("sdk:src/lib.rs", row.path);
        assert_eq!(2, row.bound_to.len());
    }

    #[test]
    fn a_manifest_round_trips_through_toml_with_the_bound_to_key() {
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            sources: BTreeMap::from([(
                source_id(A),
                Row {
                    path: "sdk:src/main.rs".to_string(),
                    bound_to: vec!["deck-fixture".to_string(), "1|topic-aaaa".to_string()],
                },
            )]),
        };
        let text = toml::to_string(&manifest).unwrap();
        assert!(text.starts_with("version = 1\n"), "{text}");
        assert!(
            text.contains("bound-to = ["),
            "the key is spelled bound-to: {text}"
        );
        assert_eq!(manifest, toml::from_str(&text).unwrap());
    }
}
