//! A workspace: a folder that groups decks, sharing directives and a title.
//!
//! Workspaces let related decks (e.g. all the English-vocab decks) live in one
//! folder, be reviewed together, and inherit a common set of directives without
//! repeating them in every file. Membership is **folder-implicit**: a workspace
//! is any folder containing `*.txt` decks. An optional [`MANIFEST`]
//! (`alix.toml`) sets a `title` and a `[defaults]` table of shared directives,
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
pub const MANIFEST: &str = "alix.toml";

/// The progress store a workspace uses when its manifest declares no `store`
/// override: `progress.json` inside the workspace folder, so progress travels
/// with the workspace.
pub const STORE_FILE: &str = "progress.json";

/// The `alix.toml` manifest: a display `title`, a one-line `description` (e.g.
/// the learning goal `alix explore` was given), an optional `store` path (where
/// this workspace's progress lives), and a `[defaults]` table of shared
/// directives (keyed by directive name). Unknown keys/sections are ignored, so
/// the format stays forgiving and forward-compatible.
#[derive(Deserialize, Default)]
struct Manifest {
    title: Option<String>,
    /// A short description of what the workspace is for (its learning goal).
    description: Option<String>,
    /// An optional icon for this workspace, shown in the picker. A path relative
    /// to the workspace (or absolute). Unset → a conventional `assets/icon.*` is
    /// used if present.
    icon: Option<String>,
    /// Where this workspace keeps its progress (relative to the workspace, or
    /// absolute). `None` → `<workspace>/progress.json`.
    store: Option<String>,
    /// Per-workspace override of the global `[ask] source_access`: when set, it
    /// decides whether the grounded ask-tutor may read this workspace's decks'
    /// source. `None` → inherit the global config.
    source_access: Option<bool>,
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
    /// A one-line description of the workspace (manifest `description`), or
    /// `None`. `alix explore` writes the learning goal here.
    pub description: Option<String>,
    /// Shared directive defaults from the manifest, folded below each member
    /// deck's own directives.
    pub settings: DeckSettings,
    /// Member deck paths: the folder's `*.txt` files, sorted by name.
    pub members: Vec<PathBuf>,
    /// The resolved icon file shown in the picker (manifest `icon`, else a
    /// conventional `assets/icon.*`), or `None` for the chevron fallback.
    pub icon: Option<PathBuf>,
}

impl Workspace {
    /// Loads the workspace rooted at `dir`: its `*.txt` members and, if
    /// present, the `alix.toml` manifest (title + shared directives). A
    /// folder without a manifest — or with a malformed one — is still a
    /// workspace, with default settings, so a bad manifest never stops it
    /// from loading.
    pub fn load(dir: impl AsRef<Path>) -> io::Result<Workspace> {
        let path = dir.as_ref().to_path_buf();
        let members = members(&path)?;
        let (title, description, settings, icon_key) = read_manifest(&path.join(MANIFEST));
        let icon = resolve_icon(&path, icon_key.as_deref());
        Ok(Workspace {
            path,
            title,
            description,
            settings,
            members,
            icon,
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

/// Reads the manifest's title, description, and shared directive defaults. A
/// missing or malformed file yields no title/description and default settings.
/// The `[defaults]` table is interpreted by [`DeckSettings::from_directives`],
/// so its keys mean exactly what the matching `% key: value` deck directives
/// mean.
fn read_manifest(path: &Path) -> (Option<String>, Option<String>, DeckSettings, Option<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (None, None, DeckSettings::default(), None);
    };
    let Ok(manifest) = toml::from_str::<Manifest>(&text) else {
        return (None, None, DeckSettings::default(), None);
    };
    let directives: Vec<(String, String)> = manifest
        .defaults
        .iter()
        .map(|(key, value)| (key.clone(), value_to_string(value)))
        .collect();
    (
        manifest.title,
        manifest.description,
        DeckSettings::from_directives(&directives),
        manifest.icon,
    )
}

/// Resolve a workspace's picker icon: the manifest `icon = "…"` file if it
/// exists, else a conventional `assets/icon.{svg,png,jpg,jpeg,webp}` (first
/// match), else `None`.
pub fn resolve_icon(dir: &Path, manifest_icon: Option<&str>) -> Option<PathBuf> {
    if let Some(rel) = manifest_icon {
        let p = dir.join(rel);
        if p.is_file() {
            return Some(p);
        }
    }
    for ext in ["svg", "png", "jpg", "jpeg", "webp"] {
        let p = dir.join("assets").join(format!("icon.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
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

/// `true` if `path` is an **explicit workspace**: a directory with an `alix.toml`
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

/// Where the workspace at `dir` keeps its progress: the manifest's `store` path
/// (relative to `dir`, or absolute), else `<dir>/progress.json`. So an
/// encapsulated workspace's progress travels with the folder, and a deck inside
/// it is tracked separately from the global store. (Callers use this only for
/// directories that are workspaces.)
pub fn store_path(dir: &Path) -> PathBuf {
    match manifest_store(dir) {
        Some(store) if Path::new(&store).is_absolute() => PathBuf::from(store),
        Some(store) => dir.join(store),
        None => dir.join(STORE_FILE),
    }
}

/// The raw `store = "..."` value from the workspace's manifest, if any.
fn manifest_store(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    toml::from_str::<Manifest>(&text).ok()?.store
}

/// The workspace manifest's `source_access` override, if it sets one — a
/// per-workspace `Some(true)`/`Some(false)` that beats the global `[ask]
/// source_access`. `None` (no manifest, malformed, or unset) → inherit global.
pub fn manifest_source_access(dir: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    toml::from_str::<Manifest>(&text).ok()?.source_access
}

/// The raw `icon = "..."` value from the workspace's manifest, if any.
pub fn manifest_icon(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    toml::from_str::<Manifest>(&text).ok()?.icon
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
            "title = \"English\"\ndescription = \"everyday vocab\"\n\n[defaults]\nmode = \"typing\"\ndirection = \"both\"\nunlock-stage = 3\n",
        );

        let ws = Workspace::load(dir.path()).unwrap();
        assert_eq!(Some("English".to_string()), ws.title);
        assert_eq!(Some("everyday vocab".to_string()), ws.description);
        assert_eq!("English", ws.display_name());
        assert_eq!(Some(Mode::Typing), ws.settings.mode);
        assert_eq!(Some(3), ws.settings.unlock_stage); // an int value parses too
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

    #[test]
    fn store_path_defaults_to_progress_json_in_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(MANIFEST), "title = \"W\"\n");
        assert_eq!(dir.path().join("progress.json"), store_path(dir.path()));
        // No manifest at all → still the in-folder default.
        let bare = dir.path().join("bare");
        std::fs::create_dir(&bare).unwrap();
        assert_eq!(bare.join("progress.json"), store_path(&bare));
    }

    #[test]
    fn store_path_honors_a_relative_or_absolute_override() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(MANIFEST), "store = \"sub/p.json\"\n");
        assert_eq!(dir.path().join("sub/p.json"), store_path(dir.path()));

        let abs = if cfg!(windows) {
            "C:/p.json"
        } else {
            "/tmp/p.json"
        };
        write(&dir.path().join(MANIFEST), &format!("store = \"{abs}\"\n"));
        assert_eq!(PathBuf::from(abs), store_path(dir.path()));
    }

    #[test]
    fn manifest_source_access_override() {
        let dir = tempfile::tempdir().unwrap();
        // Unset (or no manifest) → None, i.e. inherit the global `[ask]` setting.
        assert_eq!(None, manifest_source_access(dir.path()));
        write(&dir.path().join(MANIFEST), "title = \"W\"\n");
        assert_eq!(None, manifest_source_access(dir.path()));
        // Explicit overrides win.
        write(&dir.path().join(MANIFEST), "source_access = true\n");
        assert_eq!(Some(true), manifest_source_access(dir.path()));
        write(&dir.path().join(MANIFEST), "source_access = false\n");
        assert_eq!(Some(false), manifest_source_access(dir.path()));
    }

    #[test]
    fn resolve_icon_prefers_the_manifest_key_then_the_convention() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();

        // Nothing present → None.
        assert_eq!(resolve_icon(dir.path(), None), None);

        // Convention file present → resolves it.
        std::fs::write(assets.join("icon.svg"), "<svg/>").unwrap();
        assert_eq!(
            resolve_icon(dir.path(), None),
            Some(assets.join("icon.svg"))
        );

        // Manifest key pointing at a real file wins over the convention.
        std::fs::write(assets.join("logo.png"), b"x").unwrap();
        assert_eq!(
            resolve_icon(dir.path(), Some("assets/logo.png")),
            Some(assets.join("logo.png"))
        );

        // Manifest key pointing at a missing file falls back to the convention.
        assert_eq!(
            resolve_icon(dir.path(), Some("assets/nope.png")),
            Some(assets.join("icon.svg"))
        );
    }
}
