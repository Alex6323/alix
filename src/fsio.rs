use std::{
    fs::File,
    io::{Error, Result, Write},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Resolve,
    CreateTemp,
    WriteTemp,
    SyncTemp,
    Rename,
    SetPermissions,
    CreateDirectory,
    #[cfg(unix)]
    OpenDirectory,
    #[cfg(unix)]
    SyncDirectory,
}

/// The sibling temporary this path's replacement writes through, matching
/// the hidden-dot convention every caller already uses.
fn temp_beside(path: &Path) -> Option<std::path::PathBuf> {
    let name = path.file_name()?.to_str()?;
    Some(path.with_file_name(format!(".{name}.tmp")))
}

fn operation<T>(_operation: Operation, run: impl FnOnce() -> Result<T>) -> Result<T> {
    #[cfg(test)]
    fault::trip_operation(_operation)?;
    run()
}

#[derive(Debug)]
pub(crate) struct ReplaceError {
    source: Error,
    replaced: bool,
}

impl ReplaceError {
    pub(crate) fn replaced(&self) -> bool {
        self.replaced
    }

    pub(crate) fn into_source(self) -> Error {
        self.source
    }
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<()> {
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    let file = operation(Operation::OpenDirectory, || File::open(dir))?;
    operation(Operation::SyncDirectory, || file.sync_all())
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
    replace_file_report(tmp, path, contents).map_err(ReplaceError::into_source)
}

pub(crate) fn replace_file_report(
    tmp: &Path,
    path: &Path,
    contents: &[u8],
) -> std::result::Result<(), ReplaceError> {
    let before = |source| ReplaceError {
        source,
        replaced: false,
    };
    // A symlink names the entry the user chose to expose, so the replacement
    // has to land on its target: renaming over the link itself would turn it
    // into a regular copy and silently fork the two paths.
    // A link whose target does not resolve is still a link: failing to
    // resolve must not fall back to the path that renames over the entry.
    let resolved = match std::fs::symlink_metadata(path) {
        Ok(entry) if entry.file_type().is_symlink() => {
            Some(operation(Operation::Resolve, || std::fs::canonicalize(path)).map_err(before)?)
        }
        _ => None,
    };
    let path = resolved.as_deref().unwrap_or(path);
    let sibling;
    let tmp = match resolved.as_deref().and_then(temp_beside) {
        Some(beside) => {
            sibling = beside;
            sibling.as_path()
        }
        None => tmp,
    };

    let mut file = operation(Operation::CreateTemp, || File::create(tmp)).map_err(before)?;
    // The temporary is a new file, so it would otherwise carry the umask's
    // permissions rather than the deck's: a private deck must not widen.
    if let Ok(existing) = std::fs::metadata(path) {
        operation(Operation::SetPermissions, || {
            std::fs::set_permissions(tmp, existing.permissions())
        })
        .map_err(|source| {
            let _ = std::fs::remove_file(tmp);
            before(source)
        })?;
    }
    operation(Operation::WriteTemp, || file.write_all(contents)).map_err(before)?;
    #[cfg(test)]
    fault::trip(fault::After::TmpWrite).map_err(before)?;
    operation(Operation::SyncTemp, || file.sync_all()).map_err(before)?;
    #[cfg(test)]
    fault::trip(fault::After::Sync).map_err(before)?;
    operation(Operation::Rename, || std::fs::rename(tmp, path)).map_err(before)?;
    let after = |source| ReplaceError {
        source,
        replaced: true,
    };
    #[cfg(test)]
    fault::trip(fault::After::Rename).map_err(after)?;
    if let Some(dir) = path.parent() {
        sync_dir(dir).map_err(after)?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod replacement_contract {
    use std::os::unix::fs::PermissionsExt;

    /// Every caller of the shared replacement inherits these two properties,
    /// so they are pinned once here rather than per call site.
    #[test]
    fn a_replacement_keeps_the_targets_permissions_and_repairs_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("private.md");
        std::fs::write(&private, "before\n").unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();

        let tmp = dir.path().join(".private.md.tmp");
        super::replace_file(&tmp, &private, b"after\n").unwrap();

        assert_eq!(
            0o600,
            std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            "a replacement must not widen a private file to the umask"
        );
        assert_eq!("after\n", std::fs::read_to_string(&private).unwrap());

        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink(&private, &link).unwrap();
        let link_tmp = dir.path().join(".link.md.tmp");
        super::replace_file(&link_tmp, &link, b"through\n").unwrap();

        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the replacement must repair through the link, not replace it"
        );
        assert_eq!(
            "through\n",
            std::fs::read_to_string(&private).unwrap(),
            "the link's target carries the new content"
        );
        assert!(
            !link_tmp.exists(),
            "the caller's temporary name is unused when the path resolves elsewhere"
        );
    }

    #[test]
    fn a_replacement_does_not_destroy_a_dangling_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("temporarily-unavailable.md");
        let link = dir.path().join("deck.md");
        std::os::unix::fs::symlink(&missing, &link).unwrap();
        let tmp = dir.path().join(".deck.md.tmp");

        let result = super::replace_file(&tmp, &link, b"replacement\n");

        assert!(
            result.is_err()
                && std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
                && std::fs::read_link(&link).unwrap() == missing,
            "an unavailable target must fail without replacing the user's symlink: {result:?}"
        );
    }
}

// create_dir_all leaves each new directory entry in its parent's page cache, so
// power loss can drop a freshly-created `progress/` (and the file just written
// into it) even after the caller's save returned Ok. Create each missing
// component and fsync its parent so the entry is durable before a file lands.
pub(crate) fn create_dir_all(dir: &Path) -> Result<()> {
    if dir.exists() {
        return Ok(());
    }
    for component in missing_directories(dir) {
        match operation(Operation::CreateDirectory, || {
            std::fs::create_dir(component)
        }) {
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

fn missing_directories(dir: &Path) -> Vec<&Path> {
    let mut missing = Vec::new();
    let mut cursor = Some(dir);
    while let Some(component) = cursor {
        if component.as_os_str().is_empty() || component.exists() {
            break;
        }
        missing.push(component);
        cursor = component.parent();
    }
    missing.reverse();
    missing
}

// Test-only named kill points plus fail-on-Nth filesystem operations.
#[cfg(test)]
pub(crate) mod fault {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
        rc::Rc,
    };

    pub(crate) use super::Operation;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum After {
        TmpWrite,
        Sync,
        Rename,
    }

    thread_local!(static POINT: Cell<Option<After>> = const { Cell::new(None) });
    thread_local!(static FAILURE: RefCell<Failure> = RefCell::new(Failure::default()));

    #[derive(Default)]
    struct Failure {
        nth: Option<usize>,
        seen: usize,
        triggered: Option<Operation>,
        error_kind: Option<std::io::ErrorKind>,
    }

    pub(crate) struct FaultGuard {
        not_send: PhantomData<Rc<()>>,
    }

    pub(crate) fn fail_on_nth_operation(nth: usize) -> FaultGuard {
        fail_on_nth_operation_with_kind(nth, std::io::ErrorKind::Other)
    }

    pub(crate) fn fail_on_nth_operation_with_kind(
        nth: usize,
        error_kind: std::io::ErrorKind,
    ) -> FaultGuard {
        assert!(nth > 0, "fault operation index is one-based");
        FAILURE.with(|failure| {
            let mut failure = failure.borrow_mut();
            assert!(
                failure.nth.is_none(),
                "nested filesystem faults are unsupported"
            );
            *failure = Failure {
                nth: Some(nth),
                error_kind: Some(error_kind),
                ..Failure::default()
            };
        });
        FaultGuard {
            not_send: PhantomData,
        }
    }

    impl FaultGuard {
        pub(crate) fn triggered_operation(&self) -> Option<Operation> {
            FAILURE.with(|failure| failure.borrow().triggered)
        }
    }

    impl Drop for FaultGuard {
        fn drop(&mut self) {
            FAILURE.with(|failure| *failure.borrow_mut() = Failure::default());
        }
    }

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

    pub(super) fn trip_operation(operation: Operation) -> std::io::Result<()> {
        FAILURE.with(|failure| {
            let mut failure = failure.borrow_mut();
            let Some(nth) = failure.nth else {
                return Ok(());
            };
            failure.seen += 1;
            if failure.seen != nth {
                return Ok(());
            }
            failure.nth = None;
            failure.triggered = Some(operation);
            let kind = failure
                .error_kind
                .take()
                .unwrap_or(std::io::ErrorKind::Other);
            Err(std::io::Error::new(
                kind,
                format!("injected fault at filesystem operation {nth} ({operation:?})"),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resolved_target_gets_its_temporary_beside_the_target() {
        let root = tempfile::tempdir().unwrap();
        let caller = root.path().join("alias/deck.md");
        let target = root.path().join("elsewhere/deck.md");

        assert_eq!(
            Some(root.path().join("elsewhere/.deck.md.tmp")),
            temp_beside(&target),
            "the resolved target's directory, not the caller's, owns the temporary"
        );
        assert_ne!(temp_beside(&target), temp_beside(&caller));
    }

    #[test]
    fn the_missing_directory_walk_stops_at_an_existing_ancestor_and_an_empty_parent() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = first.join("second");
        assert_eq!(
            vec![first.as_path(), second.as_path()],
            missing_directories(&second),
            "an existing absolute ancestor is never returned for creation"
        );

        let relative = Path::new("fsio-relative-missing-law");
        assert!(!relative.exists(), "the relative fixture must stay missing");
        assert_eq!(
            vec![relative],
            missing_directories(relative),
            "the empty parent of a relative path is never returned for creation"
        );
    }

    #[test]
    fn create_directory_continues_only_after_an_already_exists_race() {
        for (kind, succeeds) in [
            (std::io::ErrorKind::AlreadyExists, true),
            (std::io::ErrorKind::PermissionDenied, false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("new");
            let fault = fault::fail_on_nth_operation_with_kind(1, kind);

            let result = create_dir_all(&target);

            assert_eq!(
                Some(fault::Operation::CreateDirectory),
                fault.triggered_operation(),
                "the {kind:?} row must fault the directory creation itself"
            );
            drop(fault);
            if succeeds {
                assert!(
                    result.is_ok(),
                    "an AlreadyExists race means another creator won: {result:?}"
                );
            } else {
                assert_eq!(
                    kind,
                    result.unwrap_err().kind(),
                    "a non-race error must surface"
                );
            }
        }
    }

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

    #[test]
    fn failing_each_filesystem_operation_keeps_atomic_replacement_retryable() {
        let mut covered = Vec::new();
        for nth in 1..=7 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("state.json");
            let tmp = dir.path().join("state.json.tmp");
            std::fs::write(&path, "old").unwrap();

            let fault = fault::fail_on_nth_operation(nth);
            let result = replace_file(&tmp, &path, b"new");
            let operation = fault.triggered_operation();
            drop(fault);

            let Some(operation) = operation else {
                assert!(
                    result.is_ok(),
                    "operation {nth}: an uninjected replacement failed"
                );
                break;
            };
            covered.push(operation);
            assert!(
                result.is_err(),
                "operation {nth} ({operation:?}): the fault was swallowed"
            );
            #[cfg(unix)]
            let committed = matches!(
                operation,
                fault::Operation::OpenDirectory | fault::Operation::SyncDirectory
            );
            #[cfg(not(unix))]
            let committed = false;
            let expected = if committed { "new" } else { "old" };
            assert_eq!(
                expected,
                std::fs::read_to_string(&path).unwrap(),
                "operation {nth} ({operation:?}): target is neither the last committed state"
            );

            replace_file(&tmp, &path, b"new").unwrap();
            assert_eq!("new", std::fs::read_to_string(&path).unwrap());
            assert!(
                !tmp.exists(),
                "operation {nth} ({operation:?}): retry left a temp file"
            );
        }
        #[cfg(unix)]
        let expected = vec![
            fault::Operation::CreateTemp,
            fault::Operation::SetPermissions,
            fault::Operation::WriteTemp,
            fault::Operation::SyncTemp,
            fault::Operation::Rename,
            fault::Operation::OpenDirectory,
            fault::Operation::SyncDirectory,
        ];
        #[cfg(not(unix))]
        let expected = vec![
            fault::Operation::CreateTemp,
            fault::Operation::SetPermissions,
            fault::Operation::WriteTemp,
            fault::Operation::SyncTemp,
            fault::Operation::Rename,
        ];
        assert_eq!(
            expected, covered,
            "the law must visit every operation a regular-file replacement performs"
        );
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
