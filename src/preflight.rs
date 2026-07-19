// This module only measures; the interactive confirm prompt lives at the CLI boundary in main.rs.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeSize {
    pub files: usize,
    pub bytes: u64,
}

impl TreeSize {
    pub fn human_bytes(&self) -> String {
        let b = self.bytes;
        if b < 1_024 {
            format!("{b} B")
        } else if b < 1_024 * 1_024 {
            format!("{:.1} KB", b as f64 / 1_024.0)
        } else {
            format!("{:.1} MB", b as f64 / (1_024.0 * 1_024.0))
        }
    }
}

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules"];

pub fn tree_size(root: &Path) -> TreeSize {
    let mut files: usize = 0;
    let mut bytes: u64 = 0;
    walk(root, &mut files, &mut bytes);
    TreeSize { files, bytes }
}

fn walk(dir: &Path, files: &mut usize, bytes: &mut u64) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            let name = entry.file_name();
            if SKIP_DIRS.iter().any(|skip| name.to_str() == Some(skip)) {
                continue;
            }
            walk(&path, files, bytes);
        } else if meta.is_file() {
            *files += 1;
            *bytes += meta.len();
        }
    }
}

pub fn is_oversized(bytes: u64, threshold: u64) -> bool {
    bytes > threshold
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn make_file(dir: &Path, name: &str, size: usize) {
        fs::write(dir.join(name), vec![0u8; size]).unwrap();
    }

    #[test]
    fn tree_size_counts_files_and_bytes() {
        let dir = TempDir::new().unwrap();
        make_file(dir.path(), "a.txt", 100);
        make_file(dir.path(), "b.txt", 200);
        let size = tree_size(dir.path());
        assert_eq!(2, size.files);
        assert_eq!(300, size.bytes);
    }

    #[test]
    fn tree_size_recurses_into_subdirs() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        make_file(&sub, "c.txt", 50);
        make_file(dir.path(), "root.txt", 10);
        let size = tree_size(dir.path());
        assert_eq!(2, size.files);
        assert_eq!(60, size.bytes);
    }

    #[test]
    fn tree_size_skips_git_target_node_modules() {
        let dir = TempDir::new().unwrap();
        make_file(dir.path(), "real.txt", 10);

        for skip in [".git", "target", "node_modules"] {
            let d = dir.path().join(skip);
            fs::create_dir(&d).unwrap();
            make_file(&d, "hidden.txt", 999);
        }

        let size = tree_size(dir.path());
        assert_eq!(1, size.files);
        assert_eq!(10, size.bytes);
    }

    #[test]
    fn tree_size_is_zero_for_empty_dir() {
        let dir = TempDir::new().unwrap();
        let size = tree_size(dir.path());
        assert_eq!(0, size.files);
        assert_eq!(0, size.bytes);
    }

    #[test]
    fn is_oversized_uses_strict_greater_than() {
        assert!(!is_oversized(5_000_000, 5_000_000));
        assert!(is_oversized(5_000_001, 5_000_000));
        assert!(!is_oversized(0, 5_000_000));
    }

    #[test]
    fn human_bytes_formats_correctly() {
        assert_eq!(
            "512 B",
            TreeSize {
                files: 1,
                bytes: 512
            }
            .human_bytes()
        );
        assert_eq!(
            "1.0 KB",
            TreeSize {
                files: 1,
                bytes: 1_024
            }
            .human_bytes()
        );
        assert_eq!(
            "1.0 MB",
            TreeSize {
                files: 1,
                bytes: 1_024 * 1_024
            }
            .human_bytes()
        );
        assert_eq!(
            "4.8 MB",
            TreeSize {
                files: 1,
                bytes: 5_000_000
            }
            .human_bytes()
        );
    }
}
