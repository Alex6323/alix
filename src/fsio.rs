use std::{
    fs::File,
    io::{Error, Result, Write},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    CreateTemp,
    WriteTemp,
    SyncTemp,
    Rename,
    CreateDirectory,
    #[cfg(unix)]
    OpenDirectory,
    #[cfg(unix)]
    SyncDirectory,
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
    let mut file = operation(Operation::CreateTemp, || File::create(tmp)).map_err(before)?;
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
    }

    pub(crate) struct FaultGuard {
        not_send: PhantomData<Rc<()>>,
    }

    pub(crate) fn fail_on_nth_operation(nth: usize) -> FaultGuard {
        assert!(nth > 0, "fault operation index is one-based");
        FAILURE.with(|failure| {
            let mut failure = failure.borrow_mut();
            assert!(
                failure.nth.is_none(),
                "nested filesystem faults are unsupported"
            );
            *failure = Failure {
                nth: Some(nth),
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
            Err(std::io::Error::other(format!(
                "injected fault at filesystem operation {nth} ({operation:?})"
            )))
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
            fault::Operation::WriteTemp,
            fault::Operation::SyncTemp,
            fault::Operation::Rename,
            fault::Operation::OpenDirectory,
            fault::Operation::SyncDirectory,
        ];
        #[cfg(not(unix))]
        let expected = vec![
            fault::Operation::CreateTemp,
            fault::Operation::WriteTemp,
            fault::Operation::SyncTemp,
            fault::Operation::Rename,
        ];
        assert_eq!(
            expected, covered,
            "the law must visit every operation in one replacement"
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
