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
#[error("{source}")]
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
    for name in &strays {
        let path = dir.join(name);
        crate::fsio::remove_file(&path).map_err(io_error(&path))?;
    }
    Ok(strays)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::sources::{
        commands::{bind, list},
        load_verified,
        testkit::{DECK, TOPIC_A, TOPIC_B, Workspace},
    };

    const FIRST: &[u8] = b"first\n";

    fn fixture_with_one_source() -> Workspace {
        let workspace = Workspace::new();
        workspace.write_live("first.rs", FIRST);
        workspace.write_live("second.md", b"second\n");
        workspace.write_live("third.txt", b"third\n");
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

    #[test]
    fn a_fault_at_every_step_leaves_old_or_new_manifest_with_every_named_copy_intact_and_the_next_command_collects_residue()
     {
        let reference = fixture_with_one_source();
        let before = manifest_of(&reference);
        reference
            .add(&["second.md", "third.txt"], &[TOPIC_B])
            .unwrap();
        let after = manifest_of(&reference);
        assert_ne!(before, after, "precondition: the add changes the manifest");

        let mut replaced_after_rename = false;
        let mut covered = Vec::new();
        for n in 1.. {
            let workspace = fixture_with_one_source();
            let fault = crate::fsio::fault::fail_on_nth_operation(n);
            let result = workspace.add(&["second.md", "third.txt"], &[TOPIC_B]);
            let triggered = fault.triggered_operation();
            drop(fault);
            let Some(operation) = triggered else {
                assert!(result.is_ok(), "uninjected run {n} failed: {result:?}");
                break;
            };
            covered.push(operation);
            let Err(error) = result else {
                panic!("fault {n} ({operation:?}) was swallowed");
            };
            let loaded = load_verified(workspace.root(), DECK)
                .unwrap_or_else(|error| panic!("fault {n} ({operation:?}): {error}"))
                .expect("a manifest survives");
            assert!(
                loaded.manifest == before || loaded.manifest == after,
                "fault {n} ({operation:?}): manifest is neither old nor new"
            );
            let commit_error = error.downcast_ref::<CommitError>().unwrap_or_else(|| {
                panic!("fault {n} ({operation:?}): not a CommitError: {error:#}")
            });
            assert_eq!(
                loaded.manifest == after,
                commit_error.manifest_replaced,
                "fault {n} ({operation:?}): manifest_replaced disagrees with the manifest on disk"
            );
            replaced_after_rename |= commit_error.manifest_replaced;
            let residue = loaded.residue().unwrap();
            list(&workspace.deck()).unwrap();
            assert_eq!(
                residue,
                loaded.residue().unwrap(),
                "fault {n} ({operation:?}): list collected residue"
            );
            bind(&workspace.deck(), &source_id(FIRST), &[TOPIC_A.to_string()])
                .unwrap_or_else(|error| panic!("fault {n} ({operation:?}): bind: {error:#}"));
            let collected = load_verified(workspace.root(), DECK).unwrap().unwrap();
            assert!(
                collected.residue().unwrap().is_empty(),
                "fault {n} ({operation:?}): residue {residue:?} not collected"
            );
        }
        assert!(
            covered.contains(&crate::fsio::Operation::RemoveFile),
            "the collection step was faulted: {covered:?}"
        );
        if cfg!(unix) {
            assert!(
                replaced_after_rename,
                "some fault after the manifest rename reports manifest_replaced"
            );
        }
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
