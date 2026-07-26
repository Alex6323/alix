use std::{
    fs::File,
    io::{Result, Write},
    path::Path,
};

// write+rename alone leaves the bytes in the page cache: power loss can
// persist the rename while dropping the data, leaving the only copy empty.
// Sync the file before the rename and the directory entry after it (unix
// only: std cannot open a directory for syncing on Windows).
pub(crate) fn replace_file(tmp: &Path, path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = File::create(tmp)?;
    file.write_all(contents)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    #[cfg(unix)]
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        File::open(dir)?.sync_all()?;
    }
    Ok(())
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
}
