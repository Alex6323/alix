//! A workspace: a folder that groups decks, sharing directives and a title.
//!
//! Workspaces let related decks (e.g. all the English-vocab decks) live in one
//! folder, be reviewed together, and inherit a common set of directives without
//! repeating them in every file. Membership is **folder-implicit**: a workspace
//! is any folder containing `*.md` decks. An optional [`MANIFEST`]
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
/// keeps it out of the member scan automatically.
pub const MANIFEST: &str = "alix.toml";

/// The progress store a workspace uses when its manifest declares no `store`
/// override: `progress.json` inside the workspace folder, so progress travels
/// with the workspace.
pub const STORE_FILE: &str = "progress.json";

/// The `alix.toml` manifest: a display `title`, a one-line `description` (e.g.
/// the learning goal the workspace generation was given), an optional `store` path (where
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
    /// `None`. A workspace `alix generate` writes the learning goal here.
    pub description: Option<String>,
    /// Shared directive defaults from the manifest, folded below each member
    /// deck's own directives.
    pub settings: DeckSettings,
    /// Member deck paths: the folder's `*.md` files, sorted by name.
    pub members: Vec<PathBuf>,
    /// The resolved icon file shown in the picker (manifest `icon`, else a
    /// conventional `assets/icon.*`), or `None` for the chevron fallback.
    pub icon: Option<PathBuf>,
}

impl Workspace {
    /// Loads the workspace rooted at `dir`: its `*.md` members and, if
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

/// `true` for conventional non-deck file names a repo-adjacent decks folder
/// carries (`README.md`, `LICENSE.md`, any-case, any extension): excluded from
/// deck enumeration so a project readme never lists (or gets stamped) as a
/// deck.
pub fn is_conventional_non_deck(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    stem.eq_ignore_ascii_case("readme") || stem.eq_ignore_ascii_case("license")
}

/// A name a file-syncing or backup tool produces for a conflicted/backup copy
/// (spec §2.4), never a real deck: excluded from every deck scan (checked
/// before the content predicate, cheaper, so a conflict copy never lists,
/// stamps, or errors). A CLOSED list: Syncthing's `.sync-conflict-`, the
/// `(conflicted copy` / `(Conflict` parenthetical of Dropbox / Nextcloud /
/// ownCloud, and the `.bak` / `.orig` / `~` backup suffixes. The load-bearing
/// entries still end in `.md` (`deck.sync-conflict-x.md`); the bare backup
/// suffixes are already dropped by the `.md`-extension filter and sit here as
/// documentation-by-code, so the one closed list answers "is this a conflict
/// copy?" for both the scans and doctor.
pub fn is_conflict_name(name: &str) -> bool {
    name.contains(".sync-conflict-")
        || name.contains(" (conflicted copy")
        || name.contains(" (Conflict")
        || name.ends_with(".bak")
        || name.ends_with(".orig")
        || name.ends_with('~')
}

/// The `.md` deck files directly in `dir` (one level), the shared enumeration
/// behind the pickers, dedup, and doctor: `*.md`, sorted by name, with
/// conventional non-deck names, conflict/backup copies, and prose files (no
/// card, no frontmatter) excluded. An unreadable directory yields an empty
/// list.
pub fn deck_files(dir: &Path) -> Vec<PathBuf> {
    members(dir).unwrap_or_default()
}

/// Whether the `.md` file at `path` enumerates as a deck (spec §3.1.3): a
/// shared, content-aware predicate over the three deck-scan sites (this
/// module's [`members`], the picker's `dir_candidates`, the listing's
/// `list_root`). It reads and cheaply parses the file, so a prose `.md` with
/// neither a `## ` card nor frontmatter never lists (nor gets stamped). A
/// file it cannot read is treated as a deck, so a listing degrades a broken
/// deck into a visible row rather than dropping it silently.
pub fn file_is_deck(path: &Path) -> bool {
    match std::fs::read_to_string(path) {
        Ok(text) => crate::l1::is_deck_content(&text),
        Err(_) => true,
    }
}

/// The `*.md` decks directly inside `dir`, sorted by name (one level deep).
/// Conventional non-deck names (`README.*`, `LICENSE.*`) and prose `.md` files
/// (no card, no frontmatter) are excluded.
fn members(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|r| r.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md"))
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| is_conventional_non_deck(n) || is_conflict_name(n))
        })
        .filter(|p| file_is_deck(p))
        .collect();
    paths.sort();
    Ok(paths)
}

/// `true` if `path` is an **explicit workspace**: a directory with an `alix.toml`
/// manifest *and* at least one `*.md` deck. A folder of decks without a manifest
/// is a plain "folder" (see [`has_decks`]) — reviewable, but not a workspace.
pub fn is_workspace(path: &Path) -> bool {
    has_decks(path) && path.join(MANIFEST).is_file()
}

/// `true` if `path` is a directory holding at least one `*.md` deck — a
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

/// The progress store for a **served root folder**: a workspace keeps its own
/// (manifest `store =` respected), a plain folder uses `<dir>/progress.json`.
/// One place for the resolution the launcher and `doctor` both apply to a root.
pub fn root_store_path(dir: &Path) -> PathBuf {
    if is_workspace(dir) {
        store_path(dir)
    } else {
        dir.join(STORE_FILE)
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

/// Sets, moves, or clears (`None`) the workspace's personal `deadline` in its
/// `alix.local.toml`, preserving everything else in the file byte-for-byte
/// (toml_edit). Creates the file when setting into a bare workspace; clearing
/// leaves an existing file in place. Atomic write (tmp + rename).
pub fn set_deadline(dir: &Path, date: Option<chrono::NaiveDate>) -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    let path = dir.join(crate::config::LOCAL_MANIFEST);
    // Clearing a deadline that was never set, with no file to touch, is a
    // true no-op: don't create the manifest as a side effect.
    if date.is_none() && !path.is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("cannot parse {}", path.display()))?;
    match date {
        Some(d) => {
            // A hand-edited `review = 5` (not a table) can't be indexed into
            // safely; error rather than panic on `doc["review"]["deadline"]`.
            if let Some(review) = doc.get("review")
                && review.as_table().is_none()
                && review.as_inline_table().is_none()
            {
                bail!("[review] in {} is not a table", path.display());
            }
            // Ensure we have a proper [review] section, not an inline table.
            if !doc.contains_key("review") {
                doc["review"] = toml_edit::table();
            }
            doc["review"]["deadline"] = toml_edit::value(d.format("%Y-%m-%d").to_string());
        }
        None => {
            // Handle both inline tables and proper tables safely. A non-table
            // `review` (e.g. `review = 5`) has no deadline key to remove, so
            // this is a silent no-op rather than an error.
            if let Some(review) = doc.get_mut("review") {
                if let Some(table) = review.as_table_mut() {
                    table.remove("deadline");
                } else if let Some(inline) = review.as_inline_table_mut() {
                    inline.remove("deadline");
                }
            }
        }
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())
        .and_then(|()| std::fs::rename(&tmp, &path))
        .with_context(|| format!("cannot write {}", path.display()))
}

/// [`set_deadline`] taking the wire's `YYYY-MM-DD` form (or `None` to clear),
/// so a thin client (the frb bridge) hands the string through and the parse —
/// and its error — stays in the lib.
pub fn set_deadline_str(dir: &Path, date: Option<&str>) -> anyhow::Result<()> {
    use anyhow::Context;
    let parsed = date
        .map(|d| {
            chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
                .with_context(|| format!("not a YYYY-MM-DD date: {d}"))
        })
        .transpose()?;
    set_deadline(dir, parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::depth::Reveal;

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn load_discovers_members_and_parses_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a.md"), "## a\n1\n");
        write(&dir.path().join("b.md"), "## b\n2\n");
        write(
            &dir.path().join(MANIFEST),
            "title = \"English\"\ndescription = \"everyday vocab\"\n\n[defaults]\nreveal = \"line\"\ndirection = \"both\"\n",
        );

        let ws = Workspace::load(dir.path()).unwrap();
        assert_eq!(Some("English".to_string()), ws.title);
        assert_eq!(Some("everyday vocab".to_string()), ws.description);
        assert_eq!("English", ws.display_name());
        assert_eq!(Some(Reveal::Line), ws.settings.reveal);
        // The manifest is not a `.md`, so it is never a member.
        let names: Vec<_> = ws
            .members
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(vec!["a.md".to_string(), "b.md".to_string()], names);
    }

    #[test]
    fn manifest_optional_title_defaults_to_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("rust");
        std::fs::create_dir(&folder).unwrap();
        write(&folder.join("a.md"), "## a\n1\n");

        let ws = Workspace::load(&folder).unwrap();
        assert_eq!(None, ws.title);
        assert_eq!("rust", ws.display_name());
        assert!(ws.settings.reveal.is_none());
        assert_eq!(1, ws.members.len());
    }

    #[test]
    fn malformed_manifest_is_forgiving() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a.md"), "## a\n1\n");
        write(&dir.path().join(MANIFEST), "this is not = = valid toml\n");
        // A bad manifest doesn't stop the folder from being a workspace.
        let ws = Workspace::load(dir.path()).unwrap();
        assert_eq!(None, ws.title);
        assert!(ws.settings.reveal.is_none());
        assert_eq!(1, ws.members.len());
    }

    #[test]
    fn is_workspace_requires_a_deck() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(!is_workspace(&empty)); // no decks

        write(&empty.join("a.md"), "## a\n1\n");
        assert!(has_decks(&empty)); // a drillable folder...
        assert!(!is_workspace(&empty)); // ...but not a workspace without a manifest

        write(&empty.join(MANIFEST), "title = \"x\"\n");
        assert!(is_workspace(&empty)); // manifest present → an explicit workspace

        // A plain file is neither.
        let file = dir.path().join("loose.md");
        write(&file, "## a\n1\n");
        assert!(!is_workspace(&file));
        assert!(!has_decks(&file));
    }

    #[test]
    fn members_exclude_prose_but_keep_header_only_stubs() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("deck.md"), "## q\na\n");
        write(&dir.path().join("stub.md"), "---\ntrace: a walk\n---\n");
        write(&dir.path().join("notes.md"), "# Notes\n\njust prose\n");

        let names: Vec<String> = members(dir.path())
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // The prose file is excluded; the real deck and the trace stub stay.
        assert_eq!(vec!["deck.md".to_string(), "stub.md".to_string()], names);
    }

    #[test]
    fn a_folder_of_only_prose_has_no_decks() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("prose");
        std::fs::create_dir(&folder).unwrap();
        write(
            &folder.join("notes.md"),
            "# Notes\n\njust prose, no cards\n",
        );
        // No `## ` card and no frontmatter anywhere: not a drillable folder.
        assert!(!has_decks(&folder));
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
    fn root_store_path_uses_in_folder_progress_for_a_plain_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            root_store_path(dir.path()),
            dir.path().join("progress.json")
        );
    }

    #[test]
    fn root_store_path_honors_a_workspace_store_override() {
        let dir = tempfile::tempdir().unwrap();
        // A workspace is a manifest *and* at least one deck, so write both.
        std::fs::write(dir.path().join("d.md"), "## Q\nA\n").unwrap();
        std::fs::write(
            dir.path().join("alix.toml"),
            "title = \"W\"\nstore = \"custom.json\"\n",
        )
        .unwrap();
        // A workspace routes through store_path, so a manifest `store =` wins.
        assert_eq!(root_store_path(dir.path()), dir.path().join("custom.json"));
        assert_eq!(root_store_path(dir.path()), store_path(dir.path()));
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

    #[test]
    fn set_deadline_creates_updates_and_clears_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        set_deadline(dir.path(), Some(date)).unwrap();
        let text = std::fs::read_to_string(dir.path().join(crate::config::LOCAL_MANIFEST)).unwrap();
        assert!(text.contains("deadline = \"2026-09-01\""));

        let moved = chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        set_deadline(dir.path(), Some(moved)).unwrap();
        let text = std::fs::read_to_string(dir.path().join(crate::config::LOCAL_MANIFEST)).unwrap();
        assert!(text.contains("2026-10-01") && !text.contains("2026-09-01"));

        set_deadline(dir.path(), None).unwrap();
        let text = std::fs::read_to_string(dir.path().join(crate::config::LOCAL_MANIFEST)).unwrap();
        assert!(!text.contains("deadline"));
    }

    #[test]
    fn set_deadline_preserves_comments_and_other_keys_byte_for_byte() {
        // The workspace-init scaffold ships commented docs; editing the deadline
        // must not reformat or eat them (spec A3; the reason toml_edit exists).
        let dir = tempfile::tempdir().unwrap();
        let scaffold = "# Personal pacing for THIS workspace\n\n[review]\n\n# retention = 0.9              # FSRS target\nretention = 0.85\n";
        std::fs::write(dir.path().join(crate::config::LOCAL_MANIFEST), scaffold).unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        set_deadline(dir.path(), Some(date)).unwrap();
        let text = std::fs::read_to_string(dir.path().join(crate::config::LOCAL_MANIFEST)).unwrap();
        assert!(text.contains("# Personal pacing for THIS workspace"));
        assert!(text.contains("# retention = 0.9              # FSRS target"));
        assert!(text.contains("retention = 0.85"));
        assert!(text.contains("deadline = \"2026-09-01\""));

        set_deadline(dir.path(), None).unwrap();
        let after =
            std::fs::read_to_string(dir.path().join(crate::config::LOCAL_MANIFEST)).unwrap();
        assert_eq!(
            scaffold, after,
            "clearing restores the file byte-identically"
        );
    }

    #[test]
    fn set_deadline_errors_instead_of_panicking_when_review_is_not_a_table() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join(crate::config::LOCAL_MANIFEST);
        let scaffold = "review = 5\n";
        std::fs::write(&manifest, scaffold).unwrap();

        let date = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        let result = set_deadline(dir.path(), Some(date));

        assert!(
            result.is_err(),
            "a non-table [review] must error, not panic"
        );
        let after = std::fs::read_to_string(&manifest).unwrap();
        assert_eq!(
            scaffold, after,
            "a failed set must leave the file untouched"
        );
    }

    #[test]
    fn clearing_a_deadline_without_a_manifest_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join(crate::config::LOCAL_MANIFEST);
        assert!(!manifest.is_file());

        set_deadline(dir.path(), None).unwrap();

        assert!(
            !manifest.is_file(),
            "clearing with no manifest must be a true no-op"
        );
    }
    #[test]
    fn sync_conflict_names_are_never_decks() {
        // A file-syncing or backup tool drops conflicted/backup copies next to
        // real decks. The closed name-pattern list keeps them out of every
        // scan: they never list, never stamp, never error. The load-bearing
        // entries still end in `.md` (a `.sync-conflict-` deck, a
        // `(conflicted copy)` / `(Conflict)` deck); the bare `.bak`/`.orig`/`~`
        // suffixes are already dropped by the extension filter.
        assert!(is_conflict_name("deck.sync-conflict-20260101-abcdef.md"));
        assert!(is_conflict_name("deck (conflicted copy 2026-01-01).md"));
        assert!(is_conflict_name("deck (Conflict).md"));
        assert!(is_conflict_name("deck.md.bak"));
        assert!(is_conflict_name("deck.md.orig"));
        assert!(is_conflict_name("deck.md~"));
        assert!(!is_conflict_name("deck.md"));
        assert!(!is_conflict_name("my-syncthing-notes.md"));

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.md"), "## q\na\n").unwrap();
        std::fs::write(
            dir.path().join("real.sync-conflict-20260101-abcdef.md"),
            "## q\na\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("real (conflicted copy 2026).md"),
            "## q\na\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("real.md.bak"), "## q\na\n").unwrap();
        let names: Vec<String> = deck_files(dir.path())
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(vec!["real.md".to_string()], names);
    }

    #[test]
    fn readme_and_license_are_not_decks() {
        // A repo-adjacent decks folder carries conventional `.md` files that
        // are not decks; the member scan must never list (or later stamp)
        // them, any case.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.md"), "## q\na\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "about this folder\n").unwrap();
        std::fs::write(dir.path().join("LICENSE.md"), "MIT\n").unwrap();
        std::fs::write(dir.path().join("license.md"), "lower-case too\n").unwrap();
        let ws = Workspace::load(dir.path()).unwrap();
        let names: Vec<String> = ws
            .members
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(vec!["real.md".to_string()], names);
    }
}
