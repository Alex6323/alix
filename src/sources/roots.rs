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
        let mut roots = BTreeMap::new();
        for (key, path) in &self.roots {
            roots.insert(
                key.clone(),
                std::fs::canonicalize(path).map_err(io_error(path))?,
            );
        }
        let canonical = Self {
            version: self.version,
            roots,
        };
        let path = dir.join(ROOTS);
        let text = toml::to_string(&canonical).map_err(|source| SourceError::Serialize {
            path: path.clone(),
            source,
        })?;
        crate::fsio::create_dir_all(dir).map_err(io_error(dir))?;
        let tmp = dir.join(format!(".{ROOTS}.tmp"));
        crate::fsio::replace_file(&tmp, &path, text.as_bytes()).map_err(io_error(&path))
    }
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
    fn roots_round_trip_through_the_private_file_with_canonical_folders() {
        assert!(workspace::is_private_name(ROOTS), "{ROOTS} never travels");
        let temp = tempfile::tempdir().unwrap();
        let live = temp.path().join("live").join("sub");
        fs::create_dir_all(&live).unwrap();
        let dir = temp.path().join("sources").join("deck-fixture");
        let roots = Roots {
            version: ROOTS_VERSION,
            roots: BTreeMap::from([(
                "sdk".to_string(),
                temp.path().join("live").join("..").join("live").join("sub"),
            )]),
        };
        roots.save(&dir).unwrap();

        let text = fs::read_to_string(dir.join(ROOTS)).unwrap();
        assert!(text.starts_with("version = 1\n"), "{text}");
        assert!(text.contains("[roots]\n"), "{text}");
        let loaded = Roots::load(&dir).unwrap().expect("the file was saved");
        assert_eq!(
            BTreeMap::from([("sdk".to_string(), fs::canonicalize(&live).unwrap())]),
            loaded.roots,
            "values are canonical, so `..` and symlinked temp roots are resolved"
        );
        assert_eq!(ROOTS_VERSION, loaded.version);
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
    fn saving_roots_refuses_a_folder_that_does_not_exist_and_writes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("sources").join("deck-fixture");
        let missing = temp.path().join("gone");
        let roots = Roots {
            version: ROOTS_VERSION,
            roots: BTreeMap::from([("sdk".to_string(), missing.clone())]),
        };
        assert!(
            matches!(roots.save(&dir), Err(SourceError::Io { path, .. }) if path == missing),
            "canonicalize names the missing folder"
        );
        assert!(!dir.join(ROOTS).exists(), "nothing was written");
    }
}
