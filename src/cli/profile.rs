use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub(crate) const RESERVED: [&str; 4] = ["add", "list", "remove", "default"];

pub(crate) fn profiles_dir() -> Result<PathBuf> {
    alix::config::profiles_dir().context("cannot determine the alix config directory")
}

pub(crate) fn config_path_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.toml"))
}

pub(crate) fn default_marker_in(dir: &Path) -> PathBuf {
    dir.join("default")
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(['/', '\\', '.']) {
        bail!("a profile name cannot be empty or contain `/`, `\\`, or `.`");
    }
    if RESERVED.contains(&name) {
        bail!(
            "`{name}` is a reserved profile command; pick another name (reserved: {})",
            RESERVED.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_verbs_are_rejected_as_names() {
        for verb in RESERVED {
            assert!(validate_name(verb).is_err(), "{verb} should be reserved");
        }
        assert!(validate_name("timmy").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a/b").is_err());
    }

    #[test]
    fn config_path_is_name_dot_toml_under_the_dir() {
        let dir = Path::new("/cfg/profiles");
        assert_eq!(dir.join("timmy.toml"), config_path_in(dir, "timmy"));
        assert_eq!(dir.join("default"), default_marker_in(dir));
    }
}
