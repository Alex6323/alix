use std::{
    fs::File,
    io::{Result, Write},
    path::Path,
};

#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<()> {
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> Result<()> {
    // std cannot open a directory for syncing on Windows.
    Ok(())
}

// write+rename alone leaves the bytes in the page cache: power loss can
// persist the rename while dropping the data, leaving the only copy empty.
// Sync the file before the rename and the directory entry after it.
pub(crate) fn replace_file(tmp: &Path, path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = File::create(tmp)?;
    file.write_all(contents)?;
    #[cfg(test)]
    fault::trip(fault::After::TmpWrite)?;
    file.sync_all()?;
    #[cfg(test)]
    fault::trip(fault::After::Sync)?;
    std::fs::rename(tmp, path)?;
    #[cfg(test)]
    fault::trip(fault::After::Rename)?;
    if let Some(dir) = path.parent() {
        sync_dir(dir)?;
    }
    Ok(())
}

// create_dir_all leaves each new directory entry in its parent's page cache, so
// power loss can drop a freshly-created `progress/` (and the file just written
// into it) even after the caller's save returned Ok. Create each missing
// component and fsync its parent so the entry is durable before a file lands.
pub(crate) fn create_dir_all(dir: &Path) -> Result<()> {
    if dir.exists() {
        return Ok(());
    }
    let mut missing = Vec::new();
    let mut cursor = Some(dir);
    while let Some(component) = cursor {
        if component.as_os_str().is_empty() || component.exists() {
            break;
        }
        missing.push(component);
        cursor = component.parent();
    }
    for component in missing.iter().rev() {
        match std::fs::create_dir(component) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
        if let Some(parent) = component.parent() {
            sync_dir(parent)?;
        }
    }
    Ok(())
}

// Test-only fault seam: deterministically fail `replace_file` right after a
// named step, to prove the atomic-rename contract holds at every kill point.
// Compiled out of production builds entirely.
#[cfg(test)]
pub(crate) mod fault {
    use std::cell::Cell;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum After {
        TmpWrite,
        Sync,
        Rename,
    }

    thread_local!(static POINT: Cell<Option<After>> = const { Cell::new(None) });

    pub(crate) fn fail_after(point: After) {
        POINT.with(|cell| cell.set(Some(point)));
    }

    pub(crate) fn clear() {
        POINT.with(|cell| cell.set(None));
    }

    pub(super) fn trip(point: After) -> std::io::Result<()> {
        POINT.with(|cell| {
            if cell.get() == Some(point) {
                cell.set(None);
                Err(std::io::Error::other("injected fault"))
            } else {
                Ok(())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_file_swaps_content_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let tmp = dir.path().join("state.json.tmp");
        std::fs::write(&path, "old").unwrap();

        replace_file(&tmp, &path, b"new").unwrap();

        assert_eq!("new", std::fs::read_to_string(&path).unwrap());
        assert!(!tmp.exists());
    }

    #[test]
    fn a_failed_replacement_keeps_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/state.json");
        let tmp = dir.path().join("state.json.tmp");

        assert!(replace_file(&tmp, &path, b"new").is_err());
        assert!(!path.exists());
    }

    // The kill-point matrix: a fault after any step must leave the target
    // readable as EITHER the old or the new content, never a partial write.
    #[test]
    fn a_fault_at_any_write_step_never_leaves_a_partial_target() {
        for (point, expected) in [
            (fault::After::TmpWrite, "old"),
            (fault::After::Sync, "old"),
            (fault::After::Rename, "new"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("state.json");
            let tmp = dir.path().join("state.json.tmp");
            std::fs::write(&path, "old").unwrap();

            fault::fail_after(point);
            let result = replace_file(&tmp, &path, b"new");
            fault::clear();

            assert!(result.is_err(), "the injected fault must surface");
            let got = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                expected, got,
                "before the rename the target stays old; after it, new — never partial"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_readonly_directory_fails_the_save_and_keeps_the_original() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let tmp = dir.path().join("state.json.tmp");
        std::fs::write(&path, "old").unwrap();

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = replace_file(&tmp, &path, b"new");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(result.is_err(), "a read-only directory must fail the save");
        assert_eq!("old", std::fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn create_dir_all_builds_the_missing_chain() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");

        create_dir_all(&nested).unwrap();

        assert!(nested.is_dir());
        // Idempotent on an existing chain.
        create_dir_all(&nested).unwrap();
    }
}
