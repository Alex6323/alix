use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{ROOTS, ROOTS_VERSION, SourceError, io_error, read_versioned};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roots {
    pub version: u32,
    pub roots: BTreeMap<String, PathBuf>,
}

impl Roots {
    pub fn load(dir: &Path) -> Result<Option<Self>, SourceError> {
        read_versioned(
            &dir.join(ROOTS),
            |roots: &Self| roots.version,
            ROOTS_VERSION,
        )
    }

    pub fn save(&self, dir: &Path) -> Result<(), SourceError> {
        let current = Self {
            version: ROOTS_VERSION,
            roots: self.roots.clone(),
        };
        let path = dir.join(ROOTS);
        let text = toml::to_string(&current).map_err(|source| SourceError::Serialize {
            path: path.clone(),
            source,
        })?;
        crate::fsio::create_dir_all(dir).map_err(io_error(dir))?;
        let tmp = dir.join(format!(".{ROOTS}.tmp"));
        crate::fsio::replace_file(&tmp, &path, text.as_bytes()).map_err(io_error(&path))
    }
}

pub fn canonical_folder(dir: &Path) -> Result<PathBuf, SourceError> {
    let canonical = std::fs::canonicalize(dir).map_err(io_error(dir))?;
    if !canonical.is_dir() {
        return Err(io_error(dir)(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "not a folder",
        )));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use super::*;
    use crate::{
        sources::{ROOTS, ROOTS_VERSION, SourceError},
        workspace,
    };

    #[test]
    fn roots_round_trip_through_the_private_file_and_save_writes_values_as_given() {
        assert!(workspace::is_private_name(ROOTS), "{ROOTS} never travels");
        let temp = tempfile::tempdir().unwrap();
        let live = temp.path().join("live").join("sub");
        fs::create_dir_all(&live).unwrap();
        let dir = temp.path().join("sources").join("deck-fixture");
        let canonical =
            canonical_folder(&temp.path().join("live").join("..").join("live").join("sub"))
                .unwrap();
        assert_eq!(
            fs::canonicalize(&live).unwrap(),
            canonical,
            "canonical_folder resolves `..` and symlinked temp roots"
        );
        let stale = temp.path().join("gone");
        let roots = Roots {
            version: 7,
            roots: BTreeMap::from([
                ("sdk".to_string(), canonical.clone()),
                ("old".to_string(), stale.clone()),
            ]),
        };
        roots.save(&dir).unwrap();

        let text = fs::read_to_string(dir.join(ROOTS)).unwrap();
        assert!(text.starts_with("version = 1\n"), "{text}");
        assert!(text.contains("[roots]\n"), "{text}");
        let loaded = Roots::load(&dir).unwrap().expect("the file was saved");
        assert_eq!(
            BTreeMap::from([("sdk".to_string(), canonical), ("old".to_string(), stale)]),
            loaded.roots,
            "save writes every value as given, a stale folder included"
        );
        assert_eq!(
            ROOTS_VERSION, loaded.version,
            "save writes the current version, whatever the struct carried"
        );
    }

    #[test]
    fn roots_load_is_none_when_absent_and_refuses_another_version_and_unknown_fields() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("sources").join("deck-fixture");
        assert!(Roots::load(&dir).unwrap().is_none(), "no folder: None");
        fs::create_dir_all(&dir).unwrap();
        assert!(Roots::load(&dir).unwrap().is_none(), "no file: None");

        fs::write(dir.join(ROOTS), "version = 2\n\n[roots]\n").unwrap();
        assert!(
            matches!(Roots::load(&dir), Err(SourceError::Version { found: 2 })),
            "version 2 is refused"
        );

        fs::write(dir.join(ROOTS), "version = 1\nnote = 1\n\n[roots]\n").unwrap();
        assert!(
            matches!(Roots::load(&dir), Err(SourceError::Toml { path, .. }) if path == dir.join(ROOTS)),
            "an unknown field is refused"
        );

        fs::write(dir.join(ROOTS), "version = 1\n\n[roots]\nsdk = 3\n").unwrap();
        assert!(
            matches!(Roots::load(&dir), Err(SourceError::Toml { .. })),
            "a root that is not a string is refused"
        );
    }

    #[test]
    fn canonical_folder_refuses_a_missing_folder_and_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("gone");
        assert!(
            matches!(canonical_folder(&missing), Err(SourceError::Io { path, .. }) if path == missing),
            "a missing folder is named"
        );
        let file = temp.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(
            matches!(canonical_folder(&file), Err(SourceError::Io { path, .. }) if path == file),
            "a file is not a folder"
        );
    }
}
