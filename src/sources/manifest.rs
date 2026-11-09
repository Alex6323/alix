use std::{
    collections::{BTreeMap, BTreeSet},
    fs::Metadata,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    Binding, DisplayPath, MANIFEST, MANIFEST_VERSION, SourceError, copy_name, io_error,
    is_copy_name, is_source_id, line_count, parse_binding, read_versioned, source_id,
};
use crate::{deck::Deck, token, workspace::WorkspaceFiles};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidRow<'a> {
    pub id: &'a str,
    pub row: &'a Row,
    pub display: DisplayPath,
    pub bindings: Vec<Binding>,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Option<Self>, SourceError> {
        read_versioned(
            &dir.join(MANIFEST),
            |manifest: &Self| manifest.version,
            MANIFEST_VERSION,
        )
    }

    pub fn validate(&self) -> Result<Vec<ValidRow<'_>>, SourceError> {
        let mut paths: BTreeSet<&str> = BTreeSet::new();
        let mut rows = Vec::with_capacity(self.sources.len());
        for (id, row) in &self.sources {
            if !is_source_id(id) {
                return Err(SourceError::BadId(id.clone()));
            }
            let display = display_of(id, row)?;
            if !paths.insert(&row.path) {
                return Err(SourceError::DuplicatePath(row.path.clone()));
            }
            if row.bound_to.is_empty() {
                return Err(SourceError::EmptyBinding(id.clone()));
            }
            let mut topics: BTreeSet<&str> = BTreeSet::new();
            let mut bindings = Vec::with_capacity(row.bound_to.len());
            for entry in &row.bound_to {
                let binding = parse_binding(id, entry)?;
                if !topics.insert(topic_of(entry)) {
                    return Err(SourceError::DuplicateTopicEntry {
                        id: id.clone(),
                        topic: binding.topic,
                    });
                }
                bindings.push(binding);
            }
            rows.push(ValidRow {
                id,
                row,
                display,
                bindings,
            });
        }
        Ok(rows)
    }

    pub fn to_text(&self) -> String {
        let mut text = format!("version = {}\n\n[sources]\n", self.version);
        for (id, row) in &self.sources {
            let bound: Vec<String> = row
                .bound_to
                .iter()
                .map(|entry| toml_string(entry))
                .collect();
            text.push_str(&format!(
                "{id} = {{ path = {}, bound-to = [{}] }}\n",
                toml_string(&row.path),
                bound.join(", ")
            ));
        }
        text
    }
}

fn toml_string(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            control if control < ' ' || control == '\u{7f}' => {
                quoted.push_str(&format!("\\u{:04X}", u32::from(control)));
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
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

pub(super) fn topic_of(entry: &str) -> &str {
    entry.rsplit_once('|').map_or(entry, |(_, topic)| topic)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckSources {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub copies: BTreeMap<String, Vec<u8>>,
}

pub fn load_verified(
    workspace_root: &Path,
    deck_id: &str,
) -> Result<Option<DeckSources>, SourceError> {
    if !matches!(token::parse_id(deck_id), Some((token::Kind::Deck, ..))) {
        return Err(SourceError::BadDeckId(deck_id.to_string()));
    }
    let files = WorkspaceFiles::new(workspace_root);
    let dir = files.sources_for(deck_id);
    if !folder_exists(&files.sources())? || !folder_exists(&dir)? {
        return Ok(None);
    }
    let Some(manifest) = Manifest::load(&dir)? else {
        return Ok(None);
    };
    let rows = manifest.validate()?;
    let copies = verify_copies(&dir, &rows)?;
    Ok(Some(DeckSources {
        dir,
        manifest,
        copies,
    }))
}

pub(super) fn folder_exists(path: &Path) -> Result<bool, SourceError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(SourceError::LinkedFolder(path.to_path_buf()))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(path)(error)),
    }
}

fn verify_copies(
    dir: &Path,
    rows: &[ValidRow<'_>],
) -> Result<BTreeMap<String, Vec<u8>>, SourceError> {
    let names = read_names(dir).map_err(io_error(dir))?;
    let mut copies = BTreeMap::new();
    for valid in rows {
        let expected = copy_name(valid.id, &valid.display);
        let path = dir.join(&expected);
        let inspected = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(missing_copy(valid.id, expected, &names));
            }
            Err(error) => return Err(io_error(&path)(error)),
        };
        if !inspected.file_type().is_file() {
            return Err(SourceError::NotRegularFile(path));
        }
        let bytes = read_inspected_file(&path, &inspected)?;
        if bytes.is_empty() {
            return Err(SourceError::EmptyCopy(path));
        }
        if source_id(&bytes) != valid.id {
            return Err(SourceError::DigestMismatch(path));
        }
        let lines = line_count(&bytes);
        for (entry, binding) in valid.row.bound_to.iter().zip(&valid.bindings) {
            if binding.ranges.iter().any(|&(_, end)| end as usize > lines) {
                return Err(SourceError::RangeOutOfBounds {
                    id: valid.id.to_string(),
                    entry: entry.clone(),
                    lines,
                });
            }
        }
        copies.insert(valid.id.to_string(), bytes);
    }
    Ok(copies)
}

fn missing_copy(id: &str, expected: String, names: &BTreeSet<String>) -> SourceError {
    let prefix = format!("{id}.");
    match names.iter().find(|name| name.starts_with(&prefix)) {
        Some(found) => SourceError::WrongName {
            expected,
            found: found.clone(),
        },
        None => SourceError::MissingCopy(id.to_string()),
    }
}

#[cfg(unix)]
fn read_inspected_file(path: &Path, inspected: &Metadata) -> Result<Vec<u8>, SourceError> {
    use std::{io::Read, os::unix::fs::MetadataExt};

    let mut file = std::fs::File::open(path).map_err(io_error(path))?;
    let opened = file.metadata().map_err(io_error(path))?;
    if !opened.file_type().is_file()
        || opened.dev() != inspected.dev()
        || opened.ino() != inspected.ino()
    {
        return Err(SourceError::NotRegularFile(path.to_path_buf()));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error(path))?;
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_inspected_file(path: &Path, _inspected: &Metadata) -> Result<Vec<u8>, SourceError> {
    std::fs::read(path).map_err(io_error(path))
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

fn strays(dir: &Path, rows: &[ValidRow<'_>]) -> Result<Vec<String>, SourceError> {
    let named: BTreeSet<String> = rows
        .iter()
        .map(|valid| copy_name(valid.id, &valid.display))
        .collect();
    let names = match read_names(dir) {
        Ok(names) => names,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(dir)(error)),
    };
    Ok(names
        .into_iter()
        .filter(|name| is_copy_name(name) && !named.contains(name))
        .collect())
}

pub fn stray_copies(dir: &Path) -> Result<Vec<String>, SourceError> {
    match Manifest::load(dir)? {
        Some(manifest) => strays(dir, &manifest.validate()?),
        None => strays(dir, &[]),
    }
}

impl DeckSources {
    pub fn residue(&self) -> Result<Vec<String>, SourceError> {
        strays(&self.dir, &self.manifest.validate()?)
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
pub(super) mod tests {
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

    pub(in crate::sources) const DECK: &str = "deck-fixture";
    const A: &[u8] = b"fn main() {}\n";
    const B: &[u8] = b"# RFC 9110\nHTTP Semantics\r\n";

    pub(in crate::sources) struct Fixture {
        pub(in crate::sources) root: TempDir,
        pub(in crate::sources) dir: PathBuf,
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

    pub(in crate::sources) fn fixture() -> Fixture {
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
    fn a_valid_folder_loads_with_its_bytes_and_no_residue_and_an_absent_one_is_none() {
        let fixture = fixture();
        assert_eq!(
            fixture.dir,
            WorkspaceFiles::new(fixture.root.path()).sources_for(DECK),
            "the folder is sources/<deck-id> at the workspace root"
        );
        let sources = fixture.load().unwrap().expect("a manifest is present");
        assert_eq!(fixture.dir, sources.dir);
        assert_eq!(2, sources.manifest.sources.len());
        assert_eq!(
            BTreeMap::from([
                (fixture.a.clone(), A.to_vec()),
                (fixture.b.clone(), B.to_vec())
            ]),
            sources.copies,
            "the verified load hands back the bytes it hashed"
        );
        assert_eq!(
            Vec::<String>::new(),
            sources.residue().unwrap(),
            "no stray copy"
        );
        assert_eq!(
            Vec::<String>::new(),
            stray_copies(&fixture.dir).unwrap(),
            "stray_copies agrees with residue"
        );

        for bad in [
            "deck-fixture/../deck-fixture",
            "card-fixture",
            "fixture",
            "",
        ] {
            assert!(
                matches!(load_verified(fixture.root.path(), bad), Err(SourceError::BadDeckId(id)) if id == bad),
                "{bad:?}: not a deck id"
            );
        }
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
            stray_copies(&fixture.dir).unwrap(),
            "without a manifest every copy is stray"
        );
        assert_eq!(
            Vec::<String>::new(),
            stray_copies(&fixture.root.path().join("sources").join("deck-other")).unwrap(),
            "no folder: no strays"
        );
    }

    pub(in crate::sources) type Fault = (
        &'static str,
        fn(&Fixture),
        fn(&Fixture, &SourceError) -> bool,
    );

    pub(in crate::sources) fn faults() -> Vec<Fault> {
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
        faults.extend(platform_faults());
        faults
    }

    #[cfg(unix)]
    fn platform_faults() -> Vec<Fault> {
        vec![
            (
                "copy replaced by a symlink to an outside file with the same bytes",
                |f| {
                    let outside = f.root.path().join("outside.rs");
                    fs::write(&outside, A).unwrap();
                    fs::remove_file(f.copy_a()).unwrap();
                    std::os::unix::fs::symlink(&outside, f.copy_a()).unwrap();
                },
                |f, e| matches!(e, SourceError::NotRegularFile(p) if *p == f.copy_a()),
            ),
            (
                "deck folder replaced by a symlink to a folder holding the same files",
                |f| {
                    let real = f.root.path().join("real");
                    fs::rename(&f.dir, &real).unwrap();
                    std::os::unix::fs::symlink(&real, &f.dir).unwrap();
                },
                |f, e| matches!(e, SourceError::LinkedFolder(p) if *p == f.dir),
            ),
            (
                "sources root replaced by a symlink",
                |f| {
                    let sources = f.root.path().join("sources");
                    let real = f.root.path().join("real");
                    fs::rename(&sources, &real).unwrap();
                    std::os::unix::fs::symlink(&real, &sources).unwrap();
                },
                |f, e| matches!(e, SourceError::LinkedFolder(p) if *p == f.root.path().join("sources")),
            ),
        ]
    }

    #[cfg(not(unix))]
    fn platform_faults() -> Vec<Fault> {
        Vec::new()
    }

    #[test]
    fn load_verified_fails_closed_on_every_integrity_fault() {
        for (name, apply, expected) in faults() {
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

    #[cfg(unix)]
    #[test]
    fn reading_a_copy_refuses_a_handle_that_is_not_the_inspected_inode() {
        let fixture = fixture();
        let other = fs::symlink_metadata(fixture.dir.join(MANIFEST)).unwrap();
        assert!(
            matches!(
                read_inspected_file(&fixture.copy_a(), &other),
                Err(SourceError::NotRegularFile(p)) if p == fixture.copy_a()
            ),
            "a handle on another inode than the one inspected is refused"
        );
        let same = fs::symlink_metadata(fixture.copy_a()).unwrap();
        assert_eq!(
            A.to_vec(),
            read_inspected_file(&fixture.copy_a(), &same).unwrap(),
            "the inspected inode reads through"
        );
    }

    #[test]
    fn a_stray_copy_loads_and_shows_as_residue_but_a_damaged_manifest_names_no_stray() {
        let fixture = fixture();
        let stray = format!("{}.txt", source_id(b"other\n"));
        fs::write(fixture.dir.join(&stray), b"other\n").unwrap();
        fs::write(fixture.dir.join("notes.txt"), b"not copy-named").unwrap();
        let sources = fixture.load().unwrap().expect("a stray is never an error");
        assert_eq!(vec![stray.clone()], sources.residue().unwrap());
        assert_eq!(vec![stray.clone()], stray_copies(&fixture.dir).unwrap());

        fixture.rewrite_manifest("version = 1", "version = 2");
        assert!(
            matches!(
                stray_copies(&fixture.dir),
                Err(SourceError::Version { found: 2 })
            ),
            "a version-2 manifest is an error, not a list of every copy"
        );
        fs::write(fixture.dir.join(MANIFEST), "version = \n").unwrap();
        assert!(
            matches!(stray_copies(&fixture.dir), Err(SourceError::Toml { .. })),
            "an unparseable manifest is an error"
        );
        fixture.rewrite_manifest("sdk:src/main.rs", "src/main.rs");
        assert!(
            matches!(stray_copies(&fixture.dir), Err(SourceError::BadPath { .. })),
            "a manifest that fails validation is an error"
        );
    }

    fn write_deck(dir: &Path, text: &str) -> Deck {
        let path = dir.join("d.md");
        fs::write(&path, format!("---\nid: \"{DECK}\"\n---\n{text}")).unwrap();
        Deck::load(&path).unwrap()
    }

    fn manifest_with(rows: &[(&str, &str, &[&str])]) -> Manifest {
        Manifest {
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
        }
    }

    fn sources_with(rows: &[(&str, &str, &[&str])]) -> DeckSources {
        DeckSources {
            dir: PathBuf::new(),
            manifest: manifest_with(rows),
            copies: BTreeMap::new(),
        }
    }

    #[test]
    fn validate_keeps_two_paths_that_differ_only_by_case_as_two_rows() {
        let (x, y) = (source_id(b"x"), source_id(b"y"));
        let manifest = manifest_with(&[
            (&x, "sdk:README.md", &["topic-aaaa"]),
            (&y, "sdk:readme.md", &["topic-aaaa"]),
        ]);
        let rows = manifest
            .validate()
            .expect("case variants are distinct display paths");
        assert_eq!(2, rows.len());
        assert_eq!(
            vec![x.as_str(), y.as_str()],
            rows.iter().map(|row| row.id).collect::<Vec<_>>()
        );
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
        let rows = manifest.validate().unwrap();
        assert_eq!(1, rows.len());
        assert_eq!(
            vec![(100, 400), (512, 512)],
            rows[0].bindings[1].ranges,
            "the ranged entry parses into its lines"
        );
        let row = &manifest.sources["source-18pgda5qxze0435914h3awa04ej3cmvhk8hvwef6g09ybdv1czkc"];
        assert_eq!("sdk:src/lib.rs", row.path);
        assert_eq!(2, row.bound_to.len());
    }

    #[test]
    fn the_writer_renders_the_adr_example_layout_byte_for_byte_and_round_trips() {
        let golden = "version = 1\n\n[sources]\n\
                      source-18pgda5qxze0435914h3awa04ej3cmvhk8hvwef6g09ybdv1czkc = { path = \
                      \"sdk:src/lib.rs\", bound-to = [\"topic-c9ns6n7s0k9hzm7eb0e7hqn35j\", \
                      \"100-400,512|topic-3whzpgpjp1z1rsfgcvxx4984hn\"] }\n";
        let manifest: Manifest = toml::from_str(golden).unwrap();
        assert_eq!(golden, manifest.to_text(), "the ADR rule 4 layout");

        let (x, y) = (source_id(b"x"), source_id(b"y"));
        let odd = manifest_with(&[
            (
                &y,
                "sdk:a \"quoted\" \u{e9}.md",
                &["deck-fixture", "1|topic-aaaa"],
            ),
            (&x, "dl:b.txt", &["topic-aaaa"]),
        ]);
        let text = odd.to_text();
        assert_eq!(odd, toml::from_str(&text).unwrap(), "round trip: {text}");
        let x_at = text.find(&x).unwrap();
        let y_at = text.find(&y).unwrap();
        assert_eq!(x < y, x_at < y_at, "rows in id byte order: {text}");
        assert!(
            text.contains("\"sdk:a \\\"quoted\\\" \u{e9}.md\""),
            "a quote is escaped and other text stays literal: {text}"
        );
        assert_eq!(
            "version = 1\n\n[sources]\n",
            manifest_with(&[]).to_text(),
            "an empty manifest keeps its table"
        );
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
