use std::path::Path;

use thiserror::Error;

use super::{
    MANIFEST, Manifest, ROOTS, Roots, SourceError, io_error, is_copy_name, manifest::folder_exists,
    source_id, stray_copies,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub manifest: Manifest,
    pub new_copies: Vec<(String, Vec<u8>)>,
    pub roots: Option<Roots>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitReport {
    pub written: Vec<String>,
    pub collected: Vec<String>,
}

#[derive(Debug, Error)]
#[error("cannot commit the deck's sources")]
pub struct CommitError {
    pub manifest_replaced: bool,
    #[source]
    pub source: SourceError,
}

fn before(source: SourceError) -> CommitError {
    CommitError {
        manifest_replaced: false,
        source,
    }
}

pub fn commit(dir: &Path, candidate: Candidate) -> Result<CommitReport, CommitError> {
    refuse_links(dir).map_err(before)?;
    candidate.manifest.validate().map_err(before)?;
    crate::fsio::create_dir_all(dir)
        .map_err(io_error(dir))
        .map_err(before)?;
    if let Some(roots) = &candidate.roots {
        roots.save(dir).map_err(before)?;
    }
    let mut written = Vec::with_capacity(candidate.new_copies.len());
    for (name, bytes) in &candidate.new_copies {
        write_copy(dir, name, bytes).map_err(before)?;
        written.push(name.clone());
    }
    let path = dir.join(MANIFEST);
    crate::fsio::replace_file_report(
        &dir.join(format!(".{MANIFEST}.tmp")),
        &path,
        candidate.manifest.to_text().as_bytes(),
    )
    .map_err(|error| CommitError {
        manifest_replaced: error.replaced(),
        source: io_error(&path)(error.into_source()),
    })?;
    let collected = collect(dir).map_err(|source| CommitError {
        manifest_replaced: true,
        source,
    })?;
    Ok(CommitReport { written, collected })
}

pub(super) fn save_roots(dir: &Path, roots: &Roots) -> Result<(), SourceError> {
    refuse_links(dir)?;
    roots.save(dir)
}

fn refuse_links(dir: &Path) -> Result<(), SourceError> {
    if let Some(parent) = dir.parent() {
        folder_exists(parent)?;
    }
    folder_exists(dir)?;
    for name in [MANIFEST, ROOTS] {
        refuse_linked_file(&dir.join(name))?;
    }
    Ok(())
}

fn refuse_linked_file(path: &Path) -> Result<(), SourceError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            Err(SourceError::NotRegularFile(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path)(error)),
    }
}

fn write_copy(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), SourceError> {
    let path = dir.join(name);
    let id = match name.rsplit_once('.') {
        Some((id, _)) if is_copy_name(name) => id,
        _ => return Err(SourceError::BadId(name.to_string())),
    };
    refuse_linked_file(&path)?;
    crate::fsio::replace_file(&dir.join(format!(".{name}.tmp")), &path, bytes)
        .map_err(io_error(&path))?;
    let written = std::fs::read(&path).map_err(io_error(&path))?;
    if source_id(&written) != id {
        return Err(SourceError::DigestMismatch(path));
    }
    Ok(())
}

fn collect(dir: &Path) -> Result<Vec<String>, SourceError> {
    let strays = stray_copies(dir)?;
    for name in strays.iter().chain(&leftover_temporaries(dir)?) {
        let path = dir.join(name);
        crate::fsio::remove_file(&path).map_err(io_error(&path))?;
    }
    Ok(strays)
}

fn is_leftover_temporary(name: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|rest| rest.strip_suffix(".tmp"))
        .is_some_and(|target| target == MANIFEST || target == ROOTS || is_copy_name(target))
}

fn leftover_temporaries(dir: &Path) -> Result<Vec<String>, SourceError> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io_error(dir))? {
        let entry = entry.map_err(io_error(dir))?;
        if let Some(name) = entry.file_name().to_str()
            && is_leftover_temporary(name)
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        config::ExamConfig,
        sources::{
            commands::{self, bind, list},
            load_verified,
            testkit::{DECK, TOPIC_A, TOPIC_B, Workspace},
        },
    };

    const FIRST: &[u8] = b"first\n";

    fn fixture_with_one_source() -> Workspace {
        let workspace = Workspace::new();
        workspace.write_live("first.rs", FIRST);
        workspace.write_live("second.md", b"second\n");
        workspace.write_live("third.txt", b"third\n");
        fs::create_dir_all(workspace.root().join("dl")).unwrap();
        fs::write(workspace.root().join("dl").join("page.txt"), b"page\n").unwrap();
        workspace.add(&["first.rs"], &[TOPIC_A]).unwrap();
        let stray = source_id(b"stray\n");
        fs::write(workspace.dir().join(format!("{stray}.txt")), b"stray\n").unwrap();
        workspace
    }

    fn manifest_of(workspace: &Workspace) -> Manifest {
        load_verified(workspace.root(), DECK)
            .unwrap()
            .expect("a manifest survives")
            .manifest
    }

    fn add_two_copies(workspace: &Workspace) -> anyhow::Result<()> {
        workspace
            .add(&["second.md", "third.txt"], &[TOPIC_B])
            .map(drop)
    }

    fn add_under_a_new_root(workspace: &Workspace) -> anyhow::Result<()> {
        commands::add(
            commands::AddRequest {
                deck: &workspace.deck(),
                files: vec![workspace.root().join("dl").join("page.txt")],
                root: Some(("dl".to_string(), workspace.root().join("dl"))),
                key: None,
                topics: vec![TOPIC_B.to_string()],
            },
            &ExamConfig::default(),
        )
        .map(drop)
    }

    fn temporaries(workspace: &Workspace) -> Vec<String> {
        workspace
            .snapshot()
            .names
            .into_iter()
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    type Scenario = (&'static str, fn(&Workspace) -> anyhow::Result<()>);

    #[test]
    fn a_fault_at_every_step_leaves_old_or_new_manifest_with_every_named_copy_intact_and_the_next_command_collects_residue()
     {
        let scenarios: [Scenario; 2] = [
            ("two new copies", add_two_copies),
            ("a new root", add_under_a_new_root),
        ];
        for (scenario, mutate) in scenarios {
            let reference = fixture_with_one_source();
            let before = manifest_of(&reference);
            mutate(&reference).unwrap();
            let after = manifest_of(&reference);
            let roots_after = Roots::load(&reference.dir()).unwrap().unwrap();
            assert_eq!(
                scenario == "a new root",
                roots_after.roots.contains_key("dl"),
                "{scenario}: the roots file holds exactly the keys the mutation saved"
            );
            assert_ne!(
                before, after,
                "{scenario}: the mutation changes the manifest"
            );

            let mut replaced_after_rename = false;
            let mut covered = Vec::new();
            let mut left_temporaries = false;
            for n in 1.. {
                let workspace = fixture_with_one_source();
                let fault = crate::fsio::fault::fail_on_nth_operation(n);
                let result = mutate(&workspace);
                let triggered = fault.triggered_operation();
                drop(fault);
                let Some(operation) = triggered else {
                    assert!(
                        result.is_ok(),
                        "{scenario}: uninjected run {n} failed: {result:?}"
                    );
                    break;
                };
                covered.push(operation);
                let step = format!("{scenario}, fault {n} ({operation:?})");
                let Err(error) = result else {
                    panic!("{step}: the fault was swallowed");
                };
                let loaded = load_verified(workspace.root(), DECK)
                    .unwrap_or_else(|error| panic!("{step}: {error}"))
                    .expect("a manifest survives");
                assert!(
                    loaded.manifest == before || loaded.manifest == after,
                    "{step}: manifest is neither old nor new"
                );
                let commit_error = error
                    .downcast_ref::<CommitError>()
                    .unwrap_or_else(|| panic!("{step}: not a CommitError: {error:#}"));
                assert_eq!(
                    loaded.manifest == after,
                    commit_error.manifest_replaced,
                    "{step}: manifest_replaced disagrees with the manifest on disk"
                );
                if loaded.manifest == after {
                    let roots = Roots::load(&workspace.dir()).unwrap().unwrap();
                    assert_eq!(
                        roots_after.roots.keys().collect::<Vec<_>>(),
                        roots.roots.keys().collect::<Vec<_>>(),
                        "{step}: the new manifest is in place before its roots"
                    );
                }
                replaced_after_rename |= commit_error.manifest_replaced;
                left_temporaries |= !temporaries(&workspace).is_empty();
                let residue = loaded.residue().unwrap();
                list(&workspace.deck(), &ExamConfig::default()).unwrap();
                assert_eq!(
                    residue,
                    loaded.residue().unwrap(),
                    "{step}: list collected residue"
                );
                bind(
                    &workspace.deck(),
                    &source_id(FIRST),
                    &[TOPIC_A.to_string()],
                    &ExamConfig::default(),
                )
                .unwrap_or_else(|error| panic!("{step}: bind: {error:#}"));
                let collected = load_verified(workspace.root(), DECK).unwrap().unwrap();
                assert!(
                    collected.residue().unwrap().is_empty(),
                    "{step}: residue {residue:?} not collected"
                );
                assert_eq!(
                    Vec::<String>::new(),
                    temporaries(&workspace),
                    "{step}: a temporary survived the next command"
                );
            }
            assert!(
                left_temporaries,
                "{scenario}: some fault leaves a temporary for the next command to collect"
            );
            assert!(
                covered.contains(&crate::fsio::Operation::RemoveFile),
                "{scenario}: the collection step was faulted: {covered:?}"
            );
            if cfg!(unix) {
                assert!(
                    replaced_after_rename,
                    "{scenario}: some fault after the manifest rename reports manifest_replaced"
                );
            }
        }
    }

    #[test]
    fn a_commit_error_names_the_step_once_and_carries_its_cause() {
        let error = CommitError {
            manifest_replaced: false,
            source: SourceError::MissingCopy("source-x".to_string()),
        };
        let chain = format!("{:#}", anyhow::Error::from(error));
        assert_eq!(
            "cannot commit the deck's sources: the copy of source `source-x` is missing",
            chain
        );
    }

    #[test]
    fn commit_refuses_an_invalid_manifest_before_writing_anything() {
        let workspace = Workspace::new();
        let manifest = Manifest {
            version: crate::sources::MANIFEST_VERSION,
            sources: [(
                "source-bad".to_string(),
                crate::sources::Row {
                    path: "sdk:a".to_string(),
                    bound_to: vec![TOPIC_A.to_string()],
                },
            )]
            .into_iter()
            .collect(),
        };
        let error = commit(
            &workspace.dir(),
            Candidate {
                manifest,
                new_copies: Vec::new(),
                roots: None,
            },
        )
        .unwrap_err();
        assert!(!error.manifest_replaced);
        assert!(matches!(error.source, SourceError::BadId(_)), "{error:?}");
        assert!(!workspace.dir().exists(), "nothing was created");
    }
}
