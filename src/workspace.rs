use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::deck::DeckSettings;

pub const MANIFEST: &str = "alix.toml";
pub const DECKS: &str = "decks";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceFiles {
    root: PathBuf,
}

impl WorkspaceFiles {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn for_deck(path: &Path) -> Self {
        Self::new(content_root(path))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn decks(&self) -> PathBuf {
        self.root.join(DECKS)
    }

    pub fn assets(&self) -> PathBuf {
        self.root.join(crate::assets::ROOT)
    }

    pub fn assets_for(&self, deck_id: &str) -> PathBuf {
        self.assets().join(deck_id)
    }

    pub fn augment(&self) -> PathBuf {
        self.root.join("augment")
    }

    pub fn augment_for(&self, deck_id: &str) -> PathBuf {
        self.augment().join(format!("{deck_id}.json"))
    }

    pub fn manifest(&self) -> PathBuf {
        self.root.join(MANIFEST)
    }
}

#[derive(Deserialize, Default)]
struct Manifest {
    title: Option<String>,
    description: Option<String>,
    icon: Option<String>,
    store: Option<String>,
    source: Option<ManifestSource>,
    source_access: Option<bool>,
    #[serde(default)]
    defaults: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ManifestSource {
    One(String),
    Many(Vec<String>),
}

impl ManifestSource {
    fn into_values(self) -> Vec<String> {
        let values = match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        };
        values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub path: PathBuf,
    pub title: Option<String>,
    pub description: Option<String>,
    pub settings: DeckSettings,
    pub source: Vec<String>,
    pub members: Vec<PathBuf>,
    pub icon: Option<PathBuf>,
}

impl Workspace {
    pub fn load(dir: impl AsRef<Path>) -> io::Result<Workspace> {
        let path = dir.as_ref().to_path_buf();
        let members = match members(&path) {
            Ok(members) => members,
            Err(error) if error.kind() == io::ErrorKind::NotFound && has_manifest(&path) => {
                Vec::new()
            }
            Err(error) => return Err(error),
        };
        let (title, description, settings, icon_key) = read_manifest(&path.join(MANIFEST));
        let source = manifest_source(&path);
        let icon = resolve_icon(&path, icon_key.as_deref());
        Ok(Workspace {
            path,
            title,
            description,
            settings,
            source,
            members,
            icon,
        })
    }

    pub fn display_name(&self) -> String {
        self.title.clone().unwrap_or_else(|| {
            self.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
    }
}

pub fn manifest_source(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dir.join(MANIFEST)) else {
        return Vec::new();
    };
    let Ok(manifest) = toml::from_str::<Manifest>(&text) else {
        return Vec::new();
    };
    manifest
        .source
        .map(ManifestSource::into_values)
        .unwrap_or_default()
}

/// A missing or malformed manifest yields no title/description and default
/// settings, never an error.
pub(crate) fn read_manifest(
    path: &Path,
) -> (Option<String>, Option<String>, DeckSettings, Option<String>) {
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

fn value_to_string(value: &toml::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

pub fn is_conventional_non_deck(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    stem.eq_ignore_ascii_case("readme") || stem.eq_ignore_ascii_case("license")
}

/// The suffix alone decides discovery, so no file is read to skip a sidecar,
/// and one carrying an `id:` by mistake is still never offered as a deck.
pub fn is_sidecar_name(name: &str) -> bool {
    name.ends_with(".personal.md")
}

/// A closed list of sync/backup name patterns. Dropbox's "conflicted copy"
/// embeds the device name, so the match is substring, not a fixed prefix.
pub fn is_conflict_name(name: &str) -> bool {
    name.contains(".sync-conflict-")
        || name.contains("conflicted copy")
        || name.contains(" (Conflict")
        || name.ends_with(".bak")
        || name.ends_with(".orig")
        || name.ends_with('~')
}

pub fn deck_files(dir: &Path) -> Vec<PathBuf> {
    members(dir).unwrap_or_default()
}

/// `deck_files`, plus every workspace nested under `dir`, which is the reach a
/// check has: a folder of workspaces is a legal target, so a repair stopping at
/// the top level would fix none of what the check just reported.
pub fn deck_files_including_nested(dir: &Path) -> Vec<PathBuf> {
    let mut files = deck_files(dir);
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() && is_workspace(&path) {
            files.extend(deck_files_including_nested(&path));
        }
    }
    files
}

pub fn has_manifest(path: &Path) -> bool {
    path.join(MANIFEST).is_file()
}

pub fn member_dir(path: &Path) -> PathBuf {
    if has_manifest(path) {
        path.join(DECKS)
    } else {
        path.to_path_buf()
    }
}

/// Both take the path as given and make it absolute first: discovery walks
/// parents, and a relative name has fewer of them than it appears to, so
/// `d.md` and `decks` would otherwise resolve against nothing.
pub fn root_for_deck(path: &Path) -> Option<PathBuf> {
    let deck = std::path::absolute(path).ok()?;
    root_holding(deck.parent()?)
}

pub fn root_for_member_dir(path: &Path) -> Option<PathBuf> {
    root_holding(&std::path::absolute(path).ok()?)
}

fn root_holding(members: &Path) -> Option<PathBuf> {
    if members.file_name().and_then(|name| name.to_str()) != Some(DECKS) {
        return None;
    }
    let root = members.parent()?;
    has_manifest(root).then(|| root.to_path_buf())
}

pub fn content_root(path: &Path) -> PathBuf {
    root_for_deck(path)
        .or_else(|| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn file_is_deck(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| crate::parser::deck_identity(&text).ok().flatten())
        .is_some()
}

fn members(dir: &Path) -> io::Result<Vec<PathBuf>> {
    classified_deck_files(dir).map(|(initialized, _)| initialized)
}

pub fn uninitialized_deck_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    classified_deck_files(dir).map(|(_, uninitialized)| uninitialized)
}

pub fn misplaced_deck_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    if !has_manifest(dir) {
        return Ok(Vec::new());
    }
    members_where_in(dir, |_| true).map(|paths| {
        paths
            .into_iter()
            .filter(|path| {
                std::fs::read_to_string(path).ok().is_some_and(|text| {
                    crate::parser::deck_identity(&text).ok().flatten().is_some()
                })
            })
            .collect()
    })
}

pub fn classified_deck_files(dir: &Path) -> io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let candidates = members_where(dir, |_| true)?;
    let mut initialized = Vec::new();
    let mut uninitialized = Vec::new();
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if crate::parser::deck_identity(&text).ok().flatten().is_some() {
            initialized.push(path);
        } else if crate::parser::is_deck_content(&text) {
            uninitialized.push(path);
        }
    }
    Ok((initialized, uninitialized))
}

/// Every `.md` member of a folder, personal files included. `members_where`
/// excludes those by design, so pairing checks need their own listing.
pub fn listing_with_sidecars(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(member_dir(dir))?
        .filter_map(|r| r.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md"))
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| is_conventional_non_deck(n) || is_conflict_name(n))
        })
        .collect();
    paths.sort();
    Ok(paths)
}

pub(crate) fn members_where(
    dir: &Path,
    is_deck: impl FnMut(&Path) -> bool,
) -> io::Result<Vec<PathBuf>> {
    members_where_in(&member_dir(dir), is_deck)
}

fn members_where_in(
    dir: &Path,
    mut is_deck: impl FnMut(&Path) -> bool,
) -> io::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|r| r.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md"))
        .filter(|p| {
            !p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                is_conventional_non_deck(n) || is_conflict_name(n) || is_sidecar_name(n)
            })
        })
        .filter(|p| is_deck(p))
        .collect();
    paths.sort();
    Ok(paths)
}

pub fn is_workspace(path: &Path) -> bool {
    has_manifest(path)
}

pub fn has_decks(path: &Path) -> bool {
    path.is_dir() && members(path).map(|m| !m.is_empty()).unwrap_or(false)
}

/// Assumes `dir` is already a workspace; callers must check first (see
/// [`root_store_path`]).
pub fn store_path(dir: &Path) -> PathBuf {
    match manifest_store(dir) {
        Some(store) if Path::new(&store).is_absolute() => PathBuf::from(store),
        Some(store) => dir.join(store),
        None => dir.to_path_buf(),
    }
}

pub fn root_store_path(dir: &Path) -> PathBuf {
    if has_manifest(dir) {
        return store_path(dir);
    }
    root_for_member_dir(dir)
        .map(|root| store_path(&root))
        .unwrap_or_else(|| dir.to_path_buf())
}

fn manifest_store(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    toml::from_str::<Manifest>(&text).ok()?.store
}

pub fn manifest_source_access(dir: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    toml::from_str::<Manifest>(&text).ok()?.source_access
}

pub fn manifest_icon(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    toml::from_str::<Manifest>(&text).ok()?.icon
}

pub fn set_deadline(dir: &Path, date: Option<chrono::NaiveDate>) -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    let path = crate::state::UserFiles::new(dir).local_manifest();
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
            if !doc.contains_key("review") {
                doc["review"] = toml_edit::table();
            }
            doc["review"]["deadline"] = toml_edit::value(d.format("%Y-%m-%d").to_string());
        }
        None => {
            // A non-table `review` (e.g. `review = 5`) has no deadline key to
            // remove: a silent no-op here, unlike the error above.
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
    crate::fsio::replace_file(&tmp, &path, doc.to_string().as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))
}

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

    fn deck(id: &str, body: &str) -> String {
        format!("---\nformat-version: 1\nid: \"deck-{id}\"\n---\n{body}")
    }

    /// A repair is offered by a check, so it has to reach every deck the check
    /// walked; a folder of workspaces is where the two scopes came apart.
    #[test]
    fn nested_workspace_decks_are_reachable_from_a_plain_parent_folder() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("loose.md"), &deck("loose", "## a\n1\n"));
        let plain = dir.path().join("plain-folder");
        std::fs::create_dir(&plain).unwrap();
        write(&plain.join("ignored.md"), &deck("ignored", "## b\n2\n"));
        let nested = dir.path().join("ws");
        std::fs::create_dir_all(nested.join(DECKS)).unwrap();
        write(&nested.join(MANIFEST), "title = \"Nested\"\n");
        write(
            &nested.join(DECKS).join("inner.md"),
            &deck("inner", "## c\n3\n"),
        );
        let deeper = nested.join("deeper");
        std::fs::create_dir_all(deeper.join(DECKS)).unwrap();
        write(&deeper.join(MANIFEST), "title = \"Deeper\"\n");
        write(
            &deeper.join(DECKS).join("deep.md"),
            &deck("deep", "## d\n4\n"),
        );

        let mut names: Vec<String> = deck_files_including_nested(dir.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();

        assert_eq!(
            vec![
                "deep.md".to_string(),
                "inner.md".to_string(),
                "loose.md".to_string()
            ],
            names,
            "every workspace under the folder is reached, and a plain directory is not"
        );
    }

    #[test]
    fn load_discovers_members_and_parses_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let members = dir.path().join(DECKS);
        std::fs::create_dir(&members).unwrap();
        write(&members.join("a.md"), &deck("a", "## a\n1\n"));
        write(&members.join("b.md"), &deck("b", "## b\n2\n"));
        write(
            &dir.path().join(MANIFEST),
            "title = \"English\"\ndescription = \"everyday vocab\"\n\n[defaults]\nreveal = \"line\"\ndirection = \"both\"\n",
        );

        let ws = Workspace::load(dir.path()).unwrap();
        assert_eq!(Some("English".to_string()), ws.title);
        assert_eq!(Some("everyday vocab".to_string()), ws.description);
        assert_eq!("English", ws.display_name());
        assert_eq!(Some(Reveal::Line), ws.settings.reveal);
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
        write(&folder.join("a.md"), &deck("a", "## a\n1\n"));

        let ws = Workspace::load(&folder).unwrap();
        assert_eq!(None, ws.title);
        assert_eq!("rust", ws.display_name());
        assert!(ws.settings.reveal.is_none());
        assert_eq!(1, ws.members.len());
    }

    #[test]
    fn load_propagates_a_missing_root_without_a_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let error = Workspace::load(dir.path().join("missing")).unwrap_err();

        assert_eq!(io::ErrorKind::NotFound, error.kind());
    }

    #[test]
    fn load_rejects_a_file_instead_of_treating_it_as_an_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        write(&file, "ordinary notes\n");

        assert!(Workspace::load(file).is_err());
    }

    #[test]
    fn manifest_source_accepts_a_string_or_a_list() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join(MANIFEST),
            "source = \"https://x.example\"\n",
        );
        assert_eq!(
            vec!["https://x.example".to_string()],
            Workspace::load(dir.path()).unwrap().source
        );

        write(
            &dir.path().join(MANIFEST),
            "source = [\"https://x.example\", \"  \", \"notes.md\"]\n",
        );
        assert_eq!(
            vec!["https://x.example".to_string(), "notes.md".to_string()],
            manifest_source(dir.path())
        );

        write(&dir.path().join(MANIFEST), "title = \"W\"\n");
        assert!(Workspace::load(dir.path()).unwrap().source.is_empty());
    }

    #[test]
    fn malformed_manifest_is_forgiving() {
        let dir = tempfile::tempdir().unwrap();
        let members = dir.path().join(DECKS);
        std::fs::create_dir(&members).unwrap();
        write(&members.join("a.md"), &deck("a", "## a\n1\n"));
        write(&dir.path().join(MANIFEST), "this is not = = valid toml\n");
        let ws = Workspace::load(dir.path()).unwrap();
        assert_eq!(None, ws.title);
        assert!(ws.settings.reveal.is_none());
        assert_eq!(1, ws.members.len());
    }

    #[test]
    fn the_manifest_establishes_a_workspace_before_it_has_members() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(!is_workspace(&empty));

        write(&empty.join("a.md"), &deck("a", "## a\n1\n"));
        assert!(has_decks(&empty));
        assert!(!is_workspace(&empty));

        write(&empty.join(MANIFEST), "title = \"x\"\n");
        assert!(!has_decks(&empty));
        assert!(is_workspace(&empty));

        let members = empty.join(DECKS);
        std::fs::create_dir(&members).unwrap();
        write(&members.join("a.md"), &deck("a", "## a\n1\n"));
        assert!(is_workspace(&empty));

        let file = dir.path().join("loose.md");
        write(&file, "## a\n1\n");
        assert!(!is_workspace(&file));
        assert!(!has_decks(&file));
    }

    #[test]
    fn members_include_only_initialized_decks() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("deck.md"),
            "---\nformat-version: 1\nid: \"deck-deck1\"\n---\n## q\na\n",
        );
        write(&dir.path().join("stub.md"), "---\ntrace: a walk\n---\n");
        write(
            &dir.path().join("notes.md"),
            "# Notes\n\n## Design\nordinary prose\n",
        );

        let names: Vec<String> = members(dir.path())
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(vec!["deck.md".to_string()], names);
    }

    #[test]
    fn uninitialized_candidates_find_deck_like_markdown_only() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("deck.md"), "## q\na\n");
        write(&dir.path().join("stub.md"), "---\ntrace: a walk\n---\n");
        write(&dir.path().join("notes.md"), "# Notes\n\njust prose\n");

        let names: Vec<String> = uninitialized_deck_files(dir.path())
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(vec!["deck.md".to_string(), "stub.md".to_string()], names);
    }

    #[test]
    fn an_initialized_deck_with_a_malformed_card_stays_discoverable() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("broken.md"),
            "---\nformat-version: 1\nid: \"deck-broken\"\n---\n## unanswered\n",
        );

        assert_eq!(vec![dir.path().join("broken.md")], deck_files(dir.path()));
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
        assert!(!has_decks(&folder));
    }

    #[test]
    fn store_path_defaults_to_the_workspace_directory() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(MANIFEST), "title = \"W\"\n");
        assert_eq!(dir.path(), store_path(dir.path()));
        let bare = dir.path().join("bare");
        std::fs::create_dir(&bare).unwrap();
        assert_eq!(bare, store_path(&bare));
    }

    #[test]
    fn store_path_honors_a_relative_or_absolute_user_root_override() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(MANIFEST), "store = \"sub/state\"\n");
        assert_eq!(dir.path().join("sub/state"), store_path(dir.path()));

        let abs = if cfg!(windows) {
            "C:/alix-state"
        } else {
            "/tmp/alix-state"
        };
        write(&dir.path().join(MANIFEST), &format!("store = \"{abs}\"\n"));
        assert_eq!(PathBuf::from(abs), store_path(dir.path()));
    }

    #[test]
    fn manifest_source_access_override() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(None, manifest_source_access(dir.path()));
        write(&dir.path().join(MANIFEST), "title = \"W\"\n");
        assert_eq!(None, manifest_source_access(dir.path()));
        write(&dir.path().join(MANIFEST), "source_access = true\n");
        assert_eq!(Some(true), manifest_source_access(dir.path()));
        write(&dir.path().join(MANIFEST), "source_access = false\n");
        assert_eq!(Some(false), manifest_source_access(dir.path()));
    }

    #[test]
    fn root_store_path_uses_the_plain_folder_as_its_user_root() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(root_store_path(dir.path()), dir.path());
    }

    #[test]
    fn root_store_path_honors_a_workspace_store_override() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("alix.toml"),
            "title = \"W\"\nstore = \"custom-state\"\n",
        )
        .unwrap();
        assert_eq!(root_store_path(dir.path()), dir.path().join("custom-state"));
        assert_eq!(root_store_path(dir.path()), store_path(dir.path()));
    }

    #[test]
    fn workspace_files_address_only_shareable_workspace_content() {
        let files = WorkspaceFiles::new("/data/workspace");
        assert_eq!(Path::new("/data/workspace/decks"), files.decks());
        assert_eq!(
            Path::new("/data/workspace/assets/deck1"),
            files.assets_for("deck1")
        );
        assert_eq!(
            Path::new("/data/workspace/augment/deck1.json"),
            files.augment_for("deck1")
        );
        assert_eq!(Path::new("/data/workspace/alix.toml"), files.manifest());
    }

    #[test]
    fn workspace_members_live_only_under_the_decks_directory() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(MANIFEST), "title = \"W\"\n");
        let members = dir.path().join(DECKS);
        std::fs::create_dir(&members).unwrap();
        write(
            &dir.path().join("root.md"),
            &deck("root", "## root\nignored\n"),
        );
        write(
            &members.join("member.md"),
            &deck("member", "## member\nlisted\n"),
        );

        assert_eq!(vec![members.join("member.md")], deck_files(dir.path()));
    }

    #[test]
    fn workspace_root_decks_are_reported_as_misplaced() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(MANIFEST), "");
        write(
            &dir.path().join("root.md"),
            &deck("root", "## root\nanswer\n"),
        );
        write(&dir.path().join("README.md"), "## prose\nnot a deck\n");

        assert_eq!(
            vec![dir.path().join("root.md")],
            misplaced_deck_files(dir.path()).unwrap()
        );
    }

    #[test]
    fn member_decks_resolve_to_their_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(MANIFEST), "title = \"W\"\n");
        let members = dir.path().join(DECKS);
        std::fs::create_dir(&members).unwrap();
        let member = members.join("member.md");
        write(&member, &deck("member", "## member\nlisted\n"));

        assert_eq!(Some(dir.path().to_path_buf()), root_for_deck(&member));
        assert_eq!(
            Some(dir.path().to_path_buf()),
            root_for_member_dir(&members)
        );
        assert_eq!(dir.path(), content_root(&member));

        let loose = dir.path().join("loose.md");
        assert_eq!(None, root_for_deck(&loose));
        assert_eq!(dir.path(), content_root(&loose));
    }

    #[test]
    fn resolve_icon_prefers_the_manifest_key_then_the_convention() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();

        assert_eq!(resolve_icon(dir.path(), None), None);

        std::fs::write(assets.join("icon.svg"), "<svg/>").unwrap();
        assert_eq!(
            resolve_icon(dir.path(), None),
            Some(assets.join("icon.svg"))
        );

        std::fs::write(assets.join("logo.png"), b"x").unwrap();
        assert_eq!(
            resolve_icon(dir.path(), Some("assets/logo.png")),
            Some(assets.join("logo.png"))
        );

        assert_eq!(
            resolve_icon(dir.path(), Some("assets/nope.png")),
            Some(assets.join("icon.svg"))
        );
    }

    /// One fixture, both listings: the only difference between them is the
    /// personal file, and everything either one refuses it refuses for the
    /// same reason.
    #[test]
    fn both_listings_refuse_the_same_non_decks_and_only_one_shows_a_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "spanish.md",
            "spanish.personal.md",
            "README.md",
            "LICENSE.md",
            "spanish.sync-conflict-8FA2.md",
            "notes.txt",
        ] {
            std::fs::write(dir.path().join(name), "## q\na\n").unwrap();
        }
        std::fs::create_dir(dir.path().join("folder.md")).unwrap();

        let names = |paths: Vec<PathBuf>| -> Vec<String> {
            paths
                .iter()
                .filter_map(|p| p.file_name()?.to_str().map(str::to_string))
                .collect()
        };
        assert_eq!(
            vec!["spanish.md", "spanish.personal.md"],
            names(listing_with_sidecars(dir.path()).unwrap()),
            "pairing checks need the personal file"
        );
        assert_eq!(
            vec!["spanish.md"],
            names(members_where(dir.path(), |_| true).unwrap()),
            "deck discovery excludes it by design"
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
        assert!(is_conflict_name("deck.sync-conflict-20260101-abcdef.md"));
        assert!(is_conflict_name("deck (conflicted copy 2026-01-01).md"));
        assert!(is_conflict_name(
            "deck (Alex's conflicted copy 2026-07-19).md"
        ));
        assert!(is_conflict_name("deck (Conflict).md"));
        assert!(is_conflict_name("deck.md.bak"));
        assert!(is_conflict_name("deck.md.orig"));
        assert!(is_conflict_name("deck.md~"));
        assert!(!is_conflict_name("deck.md"));
        assert!(!is_conflict_name("my-syncthing-notes.md"));

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.md"), deck("real", "## q\na\n")).unwrap();
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
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.md"), deck("real", "## q\na\n")).unwrap();
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
