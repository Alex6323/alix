use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::locator::{Locator, Target};

#[derive(Debug, Clone, Copy)]
pub enum DeckPlace<'a> {
    Member {
        workspace: &'a Path,
        manifest_cite: &'a BTreeMap<String, String>,
    },
    Loose {
        folder: &'a Path,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Root {
    Local(PathBuf),
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleRoot {
    pub key: String,
    pub root: Root,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum CiteViolation {
    #[error("`cite:` key `{key}` repeats a key of the workspace `[cite]` table; rename one")]
    RepeatsManifestKey { key: String },
    #[error("`cite:` root `{key}` is a file ({}); a root is a directory or a url", .path.display())]
    FileRoot { key: String, path: PathBuf },
    #[error(
        "`cite:` roots `{outer}` and `{inner}` nest ({} contains {}); roots may not contain one another",
        .outer_path.display(),
        .inner_path.display()
    )]
    NestedRoots {
        outer: String,
        inner: String,
        outer_path: PathBuf,
        inner_path: PathBuf,
    },
    #[error("`cite:` root `{key}` is a url, which only a workspace member deck may declare")]
    UrlRootInLooseDeck { key: String },
    #[error("`cite:` root `{key}` is empty; name a directory or a url")]
    EmptyRoot { key: String },
}

pub fn visible_roots(
    deck_cite: &[(String, String)],
    place: DeckPlace,
) -> Result<Vec<VisibleRoot>, Vec<CiteViolation>> {
    let mut violations = Vec::new();
    let (base, manifest_cite) = match place {
        DeckPlace::Member {
            workspace,
            manifest_cite,
        } => (workspace, Some(manifest_cite)),
        DeckPlace::Loose { folder } => (folder, None),
    };
    let mut roots = Vec::new();
    for (key, value) in manifest_cite.into_iter().flatten() {
        if value.trim().is_empty() {
            violations.push(CiteViolation::EmptyRoot { key: key.clone() });
            continue;
        }
        roots.push(visible_root(key, value, base));
    }
    for (key, value) in deck_cite {
        if manifest_cite.is_some_and(|manifest| manifest.contains_key(key)) {
            violations.push(CiteViolation::RepeatsManifestKey { key: key.clone() });
            continue;
        }
        if manifest_cite.is_none() && crate::deck::is_url(value) {
            violations.push(CiteViolation::UrlRootInLooseDeck { key: key.clone() });
            continue;
        }
        roots.push(visible_root(key, value, base));
    }
    let mut directories = Vec::new();
    for root in &roots {
        let Root::Local(path) = &root.root else {
            continue;
        };
        if path.is_file() {
            violations.push(CiteViolation::FileRoot {
                key: root.key.clone(),
                path: path.clone(),
            });
        } else if let Ok(resolved) = std::fs::canonicalize(path) {
            directories.push((&root.key, resolved));
        }
    }
    for (index, (first_key, first)) in directories.iter().enumerate() {
        for (second_key, second) in &directories[index + 1..] {
            let (outer, inner) = if second.starts_with(first) {
                ((first_key, first), (second_key, second))
            } else if first.starts_with(second) {
                ((second_key, second), (first_key, first))
            } else {
                continue;
            };
            violations.push(CiteViolation::NestedRoots {
                outer: outer.0.to_string(),
                inner: inner.0.to_string(),
                outer_path: outer.1.clone(),
                inner_path: inner.1.clone(),
            });
        }
    }
    if violations.is_empty() {
        Ok(roots)
    } else {
        Err(violations)
    }
}

pub fn manifest_violations(workspace: &Path) -> Vec<CiteViolation> {
    let manifest_cite = crate::workspace::manifest_cite(workspace);
    visible_roots(
        &[],
        DeckPlace::Member {
            workspace,
            manifest_cite: &manifest_cite,
        },
    )
    .err()
    .unwrap_or_default()
}

/// Excludes the manifest's own violations, which every member would repeat.
pub fn deck_violations(deck_path: &Path, deck_cite: &[(String, String)]) -> Vec<CiteViolation> {
    let Ok(deck_path) = std::path::absolute(deck_path) else {
        return Vec::new();
    };
    let folder = deck_path.parent().unwrap_or(Path::new("."));
    let Some(workspace) = crate::workspace::root_for_deck(&deck_path) else {
        return visible_roots(deck_cite, DeckPlace::Loose { folder })
            .err()
            .unwrap_or_default();
    };
    let manifest_only = manifest_violations(&workspace);
    let manifest_cite = crate::workspace::manifest_cite(&workspace);
    visible_roots(
        deck_cite,
        DeckPlace::Member {
            workspace: &workspace,
            manifest_cite: &manifest_cite,
        },
    )
    .err()
    .unwrap_or_default()
    .into_iter()
    .filter(|violation| !manifest_only.contains(violation))
    .collect()
}

fn visible_root(key: &str, value: &str, base: &Path) -> VisibleRoot {
    let root = if crate::deck::is_url(value) {
        Root::Url(value.to_string())
    } else {
        Root::Local(
            base.join(value)
                .components()
                .filter(|component| !matches!(component, std::path::Component::CurDir))
                .collect(),
        )
    };
    VisibleRoot {
        key: key.to_string(),
        root,
    }
}

pub const GENERATED_KEY: &str = "src";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedRoot {
    pub key: String,
    pub locator_prefix: String,
    pub declare: Option<String>,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum GeneratedRootError {
    #[error("`cite:` roots are invalid: {}", violations(.0))]
    InvalidRoots(Vec<CiteViolation>),
    #[error("cannot read the input {}: {reason}", .input.display())]
    Unreadable { input: PathBuf, reason: String },
    #[error(
        "the input {} lies outside every `cite:` root, and `src` already names {}; add a root that holds the input, or rename that `src`",
        .input.display(),
        .taken.display()
    )]
    SrcTaken { input: PathBuf, taken: PathBuf },
}

pub fn generated_root(
    deck_cite: &[(String, String)],
    place: DeckPlace,
    input_dir: &Path,
    declare_as: &str,
) -> Result<GeneratedRoot, GeneratedRootError> {
    let roots = visible_roots(deck_cite, place).map_err(GeneratedRootError::InvalidRoots)?;
    let input =
        std::fs::canonicalize(input_dir).map_err(|error| GeneratedRootError::Unreadable {
            input: input_dir.to_path_buf(),
            reason: error.to_string(),
        })?;
    for root in &roots {
        let Root::Local(dir) = &root.root else {
            continue;
        };
        let Some(under) = std::fs::canonicalize(dir)
            .ok()
            .and_then(|dir| input.strip_prefix(dir).ok().map(Path::to_path_buf))
        else {
            continue;
        };
        let under: String = under
            .components()
            .map(|component| format!("{}/", component.as_os_str().to_string_lossy()))
            .collect();
        return Ok(GeneratedRoot {
            key: root.key.clone(),
            locator_prefix: format!("{}:{under}", root.key),
            declare: None,
        });
    }
    if let Some(taken) = roots.iter().find(|root| root.key == GENERATED_KEY) {
        let taken = match &taken.root {
            Root::Local(path) => path.clone(),
            Root::Url(url) => PathBuf::from(url),
        };
        return Err(GeneratedRootError::SrcTaken {
            input: input_dir.to_path_buf(),
            taken,
        });
    }
    let mut declared = deck_cite.to_vec();
    declared.push((GENERATED_KEY.to_string(), declare_as.to_string()));
    visible_roots(&declared, place).map_err(GeneratedRootError::InvalidRoots)?;
    Ok(GeneratedRoot {
        key: GENERATED_KEY.to_string(),
        locator_prefix: format!("{GENERATED_KEY}:"),
        declare: Some(declare_as.to_string()),
    })
}

pub fn generated_root_for_deck(
    deck_path: &Path,
    deck_cite: &[(String, String)],
    input_dir: &Path,
    declare_as: &str,
) -> Result<GeneratedRoot, GeneratedRootError> {
    let deck_path = std::path::absolute(deck_path).unwrap_or_else(|_| deck_path.to_path_buf());
    let folder = deck_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    match crate::workspace::root_for_deck(&deck_path) {
        Some(workspace) => {
            let manifest_cite = crate::workspace::manifest_cite(&workspace);
            let place = DeckPlace::Member {
                workspace: &workspace,
                manifest_cite: &manifest_cite,
            };
            generated_root(deck_cite, place, input_dir, declare_as)
        }
        None => generated_root(
            deck_cite,
            DeckPlace::Loose { folder: &folder },
            input_dir,
            declare_as,
        ),
    }
}

#[derive(Debug, Clone)]
pub struct CitationRoots {
    roots: Result<Vec<VisibleRoot>, Vec<CiteViolation>>,
    asset_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    File {
        root: PathBuf,
        path: PathBuf,
        range: Option<(u32, u32)>,
    },
    Passage {
        key: String,
    },
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ResolveError {
    #[error("`cite:` roots are invalid: {}", violations(.0))]
    InvalidRoots(Vec<CiteViolation>),
    #[error("locator key `{key}` names no `cite:` root of this deck or its workspace")]
    UnknownKey { key: String },
    #[error(
        "`cite:` root `{key}` is a url, so its locator is the bare key `{key}` naming a frozen passage, not a file"
    )]
    FileUnderUrlRoot { key: String },
    #[error("`cite:` root `{key}` is a directory, so its locator names a file as `{key}:<path>`")]
    PassageUnderLocalRoot { key: String },
    #[error(
        "passage locator `{key}` carries no `asset:`; a url root is never fetched, so only a frozen passage can be cited"
    )]
    PassageWithoutAsset { key: String },
    #[error("`cite:` root `{key}` ({}) is not a directory", .root.display())]
    RootMissing { key: String, root: PathBuf },
    #[error("file `{path}` is not found under `cite:` root `{key}` ({})", .root.display())]
    FileMissing {
        key: String,
        path: String,
        root: PathBuf,
    },
    #[error("file `{path}` resolves outside `cite:` root `{key}` ({})", .root.display())]
    Escapes {
        key: String,
        path: String,
        root: PathBuf,
    },
    #[error("`{path}` has {lines} line(s), so lines {range} run past its end")]
    LinesMissing {
        path: String,
        range: String,
        lines: usize,
    },
}

fn violations(violations: &[CiteViolation]) -> String {
    violations
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

impl CitationRoots {
    pub fn for_deck(deck: &crate::deck::Deck) -> Self {
        let deck_path = std::path::absolute(&deck.path).unwrap_or_else(|_| deck.path.clone());
        let folder = deck_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let workspace = crate::workspace::root_for_deck(&deck_path);
        let asset_dir = workspace
            .as_deref()
            .zip(deck.deck_token.as_deref())
            .and_then(|(root, deck_id)| crate::assets::deck_dir(root, deck_id).ok());
        match &workspace {
            Some(workspace) => {
                let manifest_cite = crate::workspace::manifest_cite(workspace);
                Self::from_place(
                    &deck.cite,
                    DeckPlace::Member {
                        workspace,
                        manifest_cite: &manifest_cite,
                    },
                    asset_dir,
                )
            }
            None => Self::from_place(&deck.cite, DeckPlace::Loose { folder: &folder }, asset_dir),
        }
    }

    pub(crate) fn from_place(
        deck_cite: &[(String, String)],
        place: DeckPlace,
        asset_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            roots: visible_roots(deck_cite, place),
            asset_dir,
        }
    }

    pub fn visible(&self) -> &[VisibleRoot] {
        self.roots.as_deref().unwrap_or_default()
    }

    pub fn asset_dir(&self) -> Option<&Path> {
        self.asset_dir.as_deref()
    }

    pub fn root_of(&self, key: &str) -> Option<&Root> {
        self.roots
            .as_ref()
            .ok()?
            .iter()
            .find(|root| root.key == key)
            .map(|root| &root.root)
    }

    pub fn check(&self, locator: &Locator, has_asset: bool) -> Result<&VisibleRoot, ResolveError> {
        let key = || locator.key.clone();
        let roots = self
            .roots
            .as_ref()
            .map_err(|violations| ResolveError::InvalidRoots(violations.clone()))?;
        let root = roots
            .iter()
            .find(|root| root.key == locator.key)
            .ok_or_else(|| ResolveError::UnknownKey { key: key() })?;
        match (&root.root, &locator.target) {
            (Root::Url(_), Target::File { .. }) => {
                Err(ResolveError::FileUnderUrlRoot { key: key() })
            }
            (Root::Local(_), Target::Passage) => {
                Err(ResolveError::PassageUnderLocalRoot { key: key() })
            }
            (Root::Url(_), Target::Passage) if !has_asset => {
                Err(ResolveError::PassageWithoutAsset { key: key() })
            }
            _ => Ok(root),
        }
    }

    pub fn resolve(&self, locator: &Locator, has_asset: bool) -> Result<Resolved, ResolveError> {
        let root = self.check(locator, has_asset)?;
        let key = locator.key.clone();
        let (Root::Local(dir), Target::File { path, range }) = (&root.root, &locator.target) else {
            return Ok(Resolved::Passage { key });
        };
        let canonical_root = std::fs::canonicalize(dir)
            .ok()
            .filter(|resolved| resolved.is_dir())
            .ok_or_else(|| ResolveError::RootMissing {
                key: key.clone(),
                root: dir.clone(),
            })?;
        let file = dir.join(path);
        let missing = || ResolveError::FileMissing {
            key: key.clone(),
            path: path.clone(),
            root: dir.clone(),
        };
        if !file.is_file() {
            return Err(missing());
        }
        let canonical_file = std::fs::canonicalize(&file).map_err(|_| missing())?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err(ResolveError::Escapes {
                key,
                path: path.clone(),
                root: dir.clone(),
            });
        }
        Ok(Resolved::File {
            root: dir.clone(),
            path: file,
            range: *range,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        workspace: PathBuf,
        loose: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::TempDir::new().unwrap();
        let workspace = dir.path().join("ws");
        let loose = dir.path().join("loose");
        for folder in ["crates/core/inner", "crates/core-extra", "docs"] {
            std::fs::create_dir_all(workspace.join(folder)).unwrap();
        }
        std::fs::write(workspace.join("notes.txt"), "notes\n").unwrap();
        std::fs::create_dir_all(loose.join("here")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(workspace.join("crates/core/inner"), workspace.join("link"))
            .unwrap();
        Fixture {
            _dir: dir,
            workspace,
            loose,
        }
    }

    fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries
            .iter()
            .map(|(key, root)| (key.to_string(), root.to_string()))
            .collect()
    }

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap()
    }

    type Entries = Vec<(&'static str, &'static str)>;

    enum Expected {
        Roots(Vec<(&'static str, Root)>),
        Violations(Vec<CiteViolation>),
    }

    fn nested(
        fixture: &Fixture,
        outer: &str,
        outer_rel: &str,
        inner: &str,
        inner_rel: &str,
    ) -> CiteViolation {
        CiteViolation::NestedRoots {
            outer: outer.into(),
            inner: inner.into(),
            outer_path: canonical(&fixture.workspace.join(outer_rel)),
            inner_path: canonical(&fixture.workspace.join(inner_rel)),
        }
    }

    fn locator(text: &str) -> Locator {
        crate::locator::parse_locator_comment(&format!("<!-- at: {text} -->"))
            .unwrap()
            .locator
    }

    #[test]
    fn every_locator_resolves_inside_its_root_or_fails_as_the_table_says() {
        let fx = fixture();
        let ws = fx.workspace.clone();
        let core = ws.join("crates/core");
        std::fs::write(core.join("x.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(core.join("inner/y.txt"), "deep\n").unwrap();
        std::fs::write(ws.join("x.txt"), "elsewhere\n").unwrap();
        std::fs::write(ws.join("secret.txt"), "secret\n").unwrap();
        std::fs::write(fx.loose.join("here/h.txt"), "loose\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(ws.join("secret.txt"), core.join("escape.txt")).unwrap();
            std::os::unix::fs::symlink(&ws, core.join("out")).unwrap();
        }
        let manifest_cite: BTreeMap<String, String> =
            pairs(&[("code", "crates/core")]).into_iter().collect();
        let member = CitationRoots::from_place(
            &pairs(&[("guide", "https://example.org/guide"), ("gone", "nowhere")]),
            DeckPlace::Member {
                workspace: &ws,
                manifest_cite: &manifest_cite,
            },
            None,
        );
        let loose = CitationRoots::from_place(
            &pairs(&[("here", "here")]),
            DeckPlace::Loose { folder: &fx.loose },
            None,
        );
        let file = |path: &str, range| Resolved::File {
            root: core.clone(),
            path: core.join(path),
            range,
        };
        type Row<'a> = (
            &'a str,
            &'a CitationRoots,
            &'a str,
            bool,
            Result<Resolved, ResolveError>,
        );
        let mut rows: Vec<Row> = vec![
            (
                "a whole file under a manifest root",
                &member,
                "code:x.txt",
                false,
                Ok(file("x.txt", None)),
            ),
            (
                "a range under a manifest root",
                &member,
                "code:x.txt:2-3",
                false,
                Ok(file("x.txt", Some((2, 3)))),
            ),
            (
                "a nested path",
                &member,
                "code:inner/y.txt",
                false,
                Ok(file("inner/y.txt", None)),
            ),
            (
                "a frozen file still resolves its live side",
                &member,
                "code:x.txt:1",
                true,
                Ok(file("x.txt", Some((1, 1)))),
            ),
            (
                "a frozen passage under a url root",
                &member,
                "guide",
                true,
                Ok(Resolved::Passage {
                    key: "guide".into(),
                }),
            ),
            (
                "a passage without an asset",
                &member,
                "guide",
                false,
                Err(ResolveError::PassageWithoutAsset {
                    key: "guide".into(),
                }),
            ),
            (
                "a file under a url root",
                &member,
                "guide:x.txt",
                true,
                Err(ResolveError::FileUnderUrlRoot {
                    key: "guide".into(),
                }),
            ),
            (
                "a passage under a local root",
                &member,
                "code",
                true,
                Err(ResolveError::PassageUnderLocalRoot { key: "code".into() }),
            ),
            (
                "an unknown key",
                &member,
                "docs:x.txt",
                false,
                Err(ResolveError::UnknownKey { key: "docs".into() }),
            ),
            (
                "a missing root",
                &member,
                "gone:x.txt",
                false,
                Err(ResolveError::RootMissing {
                    key: "gone".into(),
                    root: ws.join("nowhere"),
                }),
            ),
            (
                "a mistyped directory whose basename exists elsewhere",
                &member,
                "code:docs/x.txt",
                false,
                Err(ResolveError::FileMissing {
                    key: "code".into(),
                    path: "docs/x.txt".into(),
                    root: core.clone(),
                }),
            ),
            (
                "a missing file",
                &member,
                "code:nope.txt:1",
                false,
                Err(ResolveError::FileMissing {
                    key: "code".into(),
                    path: "nope.txt".into(),
                    root: core.clone(),
                }),
            ),
            (
                "a loose deck root relative to the deck folder",
                &loose,
                "here:h.txt:1",
                false,
                Ok(Resolved::File {
                    root: fx.loose.join("here"),
                    path: fx.loose.join("here/h.txt"),
                    range: Some((1, 1)),
                }),
            ),
        ];
        if cfg!(unix) {
            rows.push((
                "a symlinked file escaping the root",
                &member,
                "code:escape.txt",
                false,
                Err(ResolveError::Escapes {
                    key: "code".into(),
                    path: "escape.txt".into(),
                    root: core.clone(),
                }),
            ));
            rows.push((
                "a path through a symlinked directory escaping the root",
                &member,
                "code:out/secret.txt:1",
                false,
                Err(ResolveError::Escapes {
                    key: "code".into(),
                    path: "out/secret.txt".into(),
                    root: core.clone(),
                }),
            ));
        }
        for (label, roots, text, has_asset, expected) in rows {
            assert_eq!(
                expected,
                roots.resolve(&locator(text), has_asset),
                "row `{label}`"
            );
        }

        let lines = |text: &str| {
            member
                .live_excerpt(&locator(text))
                .map(|excerpt| excerpt.lines)
                .map_err(|error| error.downcast::<ResolveError>().ok())
        };
        assert_eq!(
            Ok(vec![(2, "two".to_string()), (3, "three".to_string())]),
            lines("code:x.txt:2-3"),
            "row `a range reads its lines`"
        );
        assert_eq!(
            Ok(vec![(3, "three".to_string())]),
            lines("code:x.txt:3"),
            "row `the last line reads`"
        );
        assert_eq!(
            Ok(vec![
                (1, "one".to_string()),
                (2, "two".to_string()),
                (3, "three".to_string())
            ]),
            lines("code:x.txt"),
            "row `a whole file reads every line`"
        );
        for (label, text, range) in [
            ("a range running past the end", "code:x.txt:2-9", "2-9"),
            ("a line past the end", "code:x.txt:4", "4"),
        ] {
            assert_eq!(
                Err(Some(ResolveError::LinesMissing {
                    path: "x.txt".into(),
                    range: range.into(),
                    lines: 3,
                })),
                lines(text),
                "row `{label}`"
            );
        }
    }

    #[test]
    fn a_generator_reuses_the_root_holding_its_input_or_declares_src_or_refuses() {
        let fx = fixture();
        let ws = fx.workspace.clone();
        let reused = |key: &str, prefix: &str| {
            Ok(GeneratedRoot {
                key: key.into(),
                locator_prefix: prefix.into(),
                declare: None,
            })
        };
        let declared = |value: &str| {
            Ok(GeneratedRoot {
                key: GENERATED_KEY.into(),
                locator_prefix: "src:".into(),
                declare: Some(value.into()),
            })
        };
        type Row = (
            &'static str,
            Entries,
            Option<Entries>,
            &'static str,
            Result<GeneratedRoot, GeneratedRootError>,
        );
        let rows: Vec<Row> = vec![
            (
                "a loose deck without roots declares src",
                vec![],
                None,
                "here",
                declared("here"),
            ),
            (
                "a manifest root holding the input is reused with the input's place under it",
                vec![],
                Some(vec![("code", "crates/core")]),
                "crates/core/inner",
                reused("code", "code:inner/"),
            ),
            (
                "a manifest root equal to the input is reused bare",
                vec![],
                Some(vec![("code", "crates/core")]),
                "crates/core",
                reused("code", "code:"),
            ),
            (
                "the deck's own src holding the input is reused, so a rebuild declares nothing",
                vec![("src", "docs")],
                Some(vec![]),
                "docs",
                reused("src", "src:"),
            ),
            (
                "an input outside every root while the manifest declares src",
                vec![],
                Some(vec![("src", "docs")]),
                "crates/core-extra",
                Err(GeneratedRootError::SrcTaken {
                    input: ws.join("crates/core-extra"),
                    taken: ws.join("docs"),
                }),
            ),
            (
                "an input outside every root while the deck declares src",
                vec![("src", "docs")],
                Some(vec![]),
                "crates/core",
                Err(GeneratedRootError::SrcTaken {
                    input: ws.join("crates/core"),
                    taken: ws.join("docs"),
                }),
            ),
            (
                "an input holding a manifest root would nest under src",
                vec![],
                Some(vec![("deep", "crates/core/inner")]),
                "crates/core",
                Err(GeneratedRootError::InvalidRoots(vec![nested(
                    &fx,
                    "src",
                    "crates/core",
                    "deep",
                    "crates/core/inner",
                )])),
            ),
        ];
        for (label, deck, manifest, input, expected) in rows {
            let deck_cite = pairs(&deck);
            let manifest_cite: Option<BTreeMap<String, String>> =
                manifest.map(|entries| pairs(&entries).into_iter().collect());
            let (place, base) = match &manifest_cite {
                Some(manifest_cite) => (
                    DeckPlace::Member {
                        workspace: &ws,
                        manifest_cite,
                    },
                    &ws,
                ),
                None => (DeckPlace::Loose { folder: &fx.loose }, &fx.loose),
            };
            assert_eq!(
                expected,
                generated_root(&deck_cite, place, &base.join(input), input),
                "row `{label}`"
            );
        }
        assert!(
            matches!(
                generated_root(
                    &[],
                    DeckPlace::Loose { folder: &fx.loose },
                    &fx.loose.join("gone"),
                    "gone"
                ),
                Err(GeneratedRootError::Unreadable { .. })
            ),
            "row `a missing input`"
        );
    }

    #[test]
    fn a_member_deck_is_charged_only_with_the_violations_its_own_cite_adds() {
        let fx = fixture();
        std::fs::write(
            fx.workspace.join(crate::workspace::MANIFEST),
            "[cite]\ncode = \"crates/core\"\ndeep = \"crates/core/inner\"\n",
        )
        .unwrap();
        let member = fx.workspace.join("decks/d.md");

        assert_eq!(
            vec![nested(
                &fx,
                "code",
                "crates/core",
                "deep",
                "crates/core/inner"
            )],
            manifest_violations(&fx.workspace),
            "step manifest: the manifest's own nesting is reported once, at the workspace"
        );
        assert_eq!(
            vec![
                nested(&fx, "code", "crates/core", "again", "crates/core/inner"),
                nested(
                    &fx,
                    "deep",
                    "crates/core/inner",
                    "again",
                    "crates/core/inner"
                ),
            ],
            deck_violations(
                &member,
                &pairs(&[("other", "docs"), ("again", "crates/core/inner")])
            ),
            "step member: only violations involving the deck's own roots"
        );
        assert_eq!(
            vec![CiteViolation::UrlRootInLooseDeck {
                key: "guide".into()
            }],
            deck_violations(
                &fx.loose.join("d.md"),
                &pairs(&[("guide", "https://example.org/guide")])
            ),
            "step loose: a deck outside a workspace resolves as loose"
        );
    }

    #[test]
    fn every_visible_root_case_resolves_or_names_its_violations() {
        let fx = fixture();
        let ws = fx.workspace.clone();
        let local = |base: &Path, rel: &str| Root::Local(base.join(rel));
        let guide = "https://example.org/guide";
        let mut rows: Vec<(&str, Entries, Option<Entries>, Expected)> = vec![
            (
                "an empty manifest root is an error, never the workspace folder itself",
                vec![],
                Some(vec![("code", "")]),
                Expected::Violations(vec![CiteViolation::EmptyRoot { key: "code".into() }]),
            ),
            (
                "a member deck sees the manifest roots plus its own, relative to the workspace folder",
                vec![("docs", "docs")],
                Some(vec![("code", "crates/core")]),
                Expected::Roots(vec![
                    ("code", local(&ws, "crates/core")),
                    ("docs", local(&ws, "docs")),
                ]),
            ),
            (
                "a loose deck root resolves against the deck's own folder",
                vec![("here", "here")],
                None,
                Expected::Roots(vec![("here", local(&fx.loose, "here"))]),
            ),
            (
                "a missing local root is not a violation here",
                vec![("gone", "nowhere")],
                Some(vec![]),
                Expected::Roots(vec![("gone", local(&ws, "nowhere"))]),
            ),
            (
                "a url root in a member deck is visible",
                vec![("guide", guide)],
                Some(vec![]),
                Expected::Roots(vec![("guide", Root::Url(guide.into()))]),
            ),
            (
                "sibling directories sharing a name prefix do not nest",
                vec![("a", "crates/core"), ("b", "crates/core-extra")],
                Some(vec![]),
                Expected::Roots(vec![
                    ("a", local(&ws, "crates/core")),
                    ("b", local(&ws, "crates/core-extra")),
                ]),
            ),
            (
                "a deck key repeating a manifest key",
                vec![("code", "docs")],
                Some(vec![("code", "crates/core")]),
                Expected::Violations(vec![CiteViolation::RepeatsManifestKey {
                    key: "code".into(),
                }]),
            ),
            (
                "a local root that is a file",
                vec![("notes", "notes.txt")],
                Some(vec![]),
                Expected::Violations(vec![CiteViolation::FileRoot {
                    key: "notes".into(),
                    path: ws.join("notes.txt"),
                }]),
            ),
            (
                "a deck root inside a manifest root",
                vec![("inner", "crates/core/inner")],
                Some(vec![("code", "crates/core")]),
                Expected::Violations(vec![nested(
                    &fx,
                    "code",
                    "crates/core",
                    "inner",
                    "crates/core/inner",
                )]),
            ),
            (
                "two manifest roots nesting",
                vec![],
                Some(vec![("code", "crates/core"), ("deep", "crates/core/inner")]),
                Expected::Violations(vec![nested(
                    &fx,
                    "code",
                    "crates/core",
                    "deep",
                    "crates/core/inner",
                )]),
            ),
            (
                "one directory under two keys",
                vec![("a", "docs"), ("b", "docs")],
                Some(vec![]),
                Expected::Violations(vec![nested(&fx, "a", "docs", "b", "docs")]),
            ),
            (
                "a url root in a loose deck",
                vec![("guide", guide)],
                None,
                Expected::Violations(vec![CiteViolation::UrlRootInLooseDeck {
                    key: "guide".into(),
                }]),
            ),
        ];
        rows.extend(cfg!(unix).then(|| {
            (
                "a deck root nesting into a manifest root through a symlink",
                vec![("linked", "link")],
                Some(vec![("code", "crates/core")]),
                Expected::Violations(vec![nested(&fx, "code", "crates/core", "linked", "link")]),
            )
        }));

        for (label, deck, manifest, expected) in rows {
            let deck_cite = pairs(&deck);
            let manifest_cite: Option<BTreeMap<String, String>> =
                manifest.map(|entries| pairs(&entries).into_iter().collect());
            let place = match &manifest_cite {
                Some(manifest_cite) => DeckPlace::Member {
                    workspace: &ws,
                    manifest_cite,
                },
                None => DeckPlace::Loose { folder: &fx.loose },
            };
            let actual = visible_roots(&deck_cite, place);
            match expected {
                Expected::Roots(roots) => {
                    let roots: Vec<VisibleRoot> = roots
                        .into_iter()
                        .map(|(key, root)| VisibleRoot {
                            key: key.into(),
                            root,
                        })
                        .collect();
                    assert_eq!(Ok(roots), actual, "row `{label}`");
                }
                Expected::Violations(violations) => {
                    assert_eq!(Err(violations), actual, "row `{label}`");
                }
            }
        }
    }
}
