use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use thiserror::Error;

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
        Root::Local(base.join(value))
    };
    VisibleRoot {
        key: key.to_string(),
        root,
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
