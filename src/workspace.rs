//! A workspace: a folder that groups decks, sharing directives and a title.
//!
//! Workspaces let related decks (e.g. all the English-vocab decks) live in one
//! folder, be reviewed together, and inherit a common set of directives without
//! repeating them in every file. Membership is **folder-implicit**: a workspace
//! is any folder containing `*.txt` decks. An optional [`MANIFEST`]
//! (`flash.toml`) sets a `title` and a `[defaults]` table of shared directives,
//! mirroring the local [`Config`](crate::config::Config) (`config.toml`) but
//! scoped to the folder. The `[defaults]` keys are the deck directive names,
//! fed through the same interpreter ([`DeckSettings::from_directives`]), then
//! folded below each member deck's own directives (see
//! [`crate::deck::Deck::load_with_defaults`]) — precedence card > deck >
//! workspace > default.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::deck::DeckSettings;

/// The reserved manifest file in a workspace folder. Its `.toml` extension
/// keeps it out of the `*.txt` member scan automatically.
pub const MANIFEST: &str = "flash.toml";

/// The `flash.toml` manifest: a display `title` and a `[defaults]` table of
/// shared directives (keyed by directive name). Unknown keys/sections are
/// ignored, so the format stays forgiving and forward-compatible.
#[derive(Deserialize, Default)]
struct Manifest {
    title: Option<String>,
    #[serde(default)]
    defaults: BTreeMap<String, toml::Value>,
}

/// A folder of decks reviewed as a unit, with shared directive defaults.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// The workspace folder.
    pub path: PathBuf,
    /// Display title (manifest `title`), or `None` to use the folder name.
    pub title: Option<String>,
    /// Shared directive defaults from the manifest, folded below each member
    /// deck's own directives.
    pub settings: DeckSettings,
    /// Member deck paths: the folder's `*.txt` files, sorted by name.
    pub members: Vec<PathBuf>,
}

impl Workspace {
    /// Loads the workspace rooted at `dir`: its `*.txt` members and, if
    /// present, the `flash.toml` manifest (title + shared directives). A
    /// folder without a manifest — or with a malformed one — is still a
    /// workspace, with default settings, so a bad manifest never stops it
    /// from loading.
    pub fn load(dir: impl AsRef<Path>) -> io::Result<Workspace> {
        let path = dir.as_ref().to_path_buf();
        let members = members(&path)?;
        let (title, settings) = read_manifest(&path.join(MANIFEST));
        Ok(Workspace {
            path,
            title,
            settings,
            members,
        })
    }

    /// The workspace's display name: its manifest `title` if set, else the
    /// folder name.
    pub fn display_name(&self) -> String {
        self.title.clone().unwrap_or_else(|| {
            self.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
    }
}

/// Reads the manifest's title and shared directive defaults. A missing or
/// malformed file yields no title and default settings. The `[defaults]` table
/// is interpreted by [`DeckSettings::from_directives`], so its keys mean
/// exactly what the matching `% key: value` deck directives mean.
fn read_manifest(path: &Path) -> (Option<String>, DeckSettings) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (None, DeckSettings::default());
    };
    let Ok(manifest) = toml::from_str::<Manifest>(&text) else {
        return (None, DeckSettings::default());
    };
    let directives: Vec<(String, String)> = manifest
        .defaults
        .iter()
        .map(|(key, value)| (key.clone(), value_to_string(value)))
        .collect();
    (manifest.title, DeckSettings::from_directives(&directives))
}

/// A TOML value as the plain string the directive interpreter expects
/// (`"both"` → `both`, `3` → `3`).
fn value_to_string(value: &toml::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

/// The `*.txt` decks directly inside `dir`, sorted by name (one level deep).
fn members(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|r| r.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "txt"))
        .collect();
    paths.sort();
    Ok(paths)
}

/// `true` if `path` is an **explicit workspace**: a directory with a `flash.toml`
/// manifest *and* at least one `*.txt` deck. A folder of decks without a manifest
/// is a plain "folder" (see [`has_decks`]) — reviewable, but not a workspace.
pub fn is_workspace(path: &Path) -> bool {
    has_decks(path) && path.join(MANIFEST).is_file()
}

/// `true` if `path` is a directory holding at least one `*.txt` deck — a
/// drillable folder in the pickers, whether or not it is a workspace.
pub fn has_decks(path: &Path) -> bool {
    path.is_dir() && members(path).map(|m| !m.is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer::Mode;

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn load_discovers_members_and_parses_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a.txt"), "# a\n\t1\n");
        write(&dir.path().join("b.txt"), "# b\n\t2\n");
        write(
            &dir.path().join(MANIFEST),
            "title = \"English\"\n\n[defaults]\nmode = \"typing\"\ndirection = \"both\"\nmax-stage = 3\n",
        );

        let ws = Workspace::load(dir.path()).unwrap();
        assert_eq!(Some("English".to_string()), ws.title);
        assert_eq!("English", ws.display_name());
        assert_eq!(Some(Mode::Typing), ws.settings.mode);
        assert_eq!(Some(3), ws.settings.max_stage); // an int value parses too
        // The manifest is not a `.txt`, so it is never a member.
        let names: Vec<_> = ws
            .members
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(vec!["a.txt".to_string(), "b.txt".to_string()], names);
    }

    #[test]
    fn manifest_optional_title_defaults_to_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("rust");
        std::fs::create_dir(&folder).unwrap();
        write(&folder.join("a.txt"), "# a\n\t1\n");

        let ws = Workspace::load(&folder).unwrap();
        assert_eq!(None, ws.title);
        assert_eq!("rust", ws.display_name());
        assert!(ws.settings.mode.is_none());
        assert_eq!(1, ws.members.len());
    }

    #[test]
    fn malformed_manifest_is_forgiving() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a.txt"), "# a\n\t1\n");
        write(&dir.path().join(MANIFEST), "this is not = = valid toml\n");
        // A bad manifest doesn't stop the folder from being a workspace.
        let ws = Workspace::load(dir.path()).unwrap();
        assert_eq!(None, ws.title);
        assert!(ws.settings.mode.is_none());
        assert_eq!(1, ws.members.len());
    }

    #[test]
    fn is_workspace_requires_a_deck() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(!is_workspace(&empty)); // no decks

        write(&empty.join("a.txt"), "# a\n\t1\n");
        assert!(has_decks(&empty)); // a drillable folder...
        assert!(!is_workspace(&empty)); // ...but not a workspace without a manifest

        write(&empty.join(MANIFEST), "title = \"x\"\n");
        assert!(is_workspace(&empty)); // manifest present → an explicit workspace

        // A plain file is neither.
        let file = dir.path().join("loose.txt");
        write(&file, "# a\n\t1\n");
        assert!(!is_workspace(&file));
        assert!(!has_decks(&file));
    }
}
