use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    deck::{AtRewrite, Deck, DeckError},
    source::{CitationIntegrity, Excerpt, SourceBase},
};

pub const ROOT: &str = "assets";
const DIGEST_PREFIX: &str = "sha256-";

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("deck ID `{0}` is not valid")]
    InvalidDeckId(String),
    #[error("asset name `{0}` is not content-addressed")]
    InvalidName(String),
    #[error("asset `{path}` does not match its content address")]
    DigestMismatch { path: PathBuf },
    #[error("{0} is not a member of an Alix workspace")]
    NotWorkspaceMember(PathBuf),
    #[error("{0} has no stable deck ID")]
    MissingDeckId(PathBuf),
    #[error("source `{0}` is missing or is not a regular file or directory")]
    MissingSource(PathBuf),
    #[error("frozen asset `{0}` is missing")]
    MissingAsset(PathBuf),
    #[error("source `{0}` is not UTF-8 text")]
    NonTextSource(PathBuf),
    #[error("citation `{locator}` is outside every declared source")]
    CitationOutsideSource { locator: String },
    #[error("cannot freeze citation `{locator}`: {message}")]
    Citation { locator: String, message: String },
    #[error("image `{0}` is outside the workspace and declared source boundaries")]
    ImageOutsideBoundary(PathBuf),
    #[error("image `{0}` is missing or is not a regular file")]
    MissingImage(PathBuf),
    #[error("{path}: {source}")]
    Deck {
        path: PathBuf,
        #[source]
        source: DeckError,
    },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum InitializeError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Stamp(#[from] crate::stamp::StampError),
    #[error(transparent)]
    Freeze(#[from] AssetError),
    #[error("cannot restore {path} after initialization failed: {source}")]
    Restore {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FreezeReport {
    pub evidence: usize,
    pub images: usize,
}

#[derive(Debug)]
pub struct InitializeReport {
    pub stamp: crate::stamp::StampOutcome,
    pub freeze: Option<FreezeReport>,
}

enum SourceInput {
    File { path: PathBuf },
    Directory { path: PathBuf },
}

pub fn deck_dir(workspace_root: &Path, deck_id: &str) -> Result<PathBuf, AssetError> {
    if !matches!(
        crate::token::parse_id(deck_id),
        Some((crate::token::Kind::Deck, ..))
    ) {
        return Err(AssetError::InvalidDeckId(deck_id.to_string()));
    }
    Ok(crate::workspace::WorkspaceFiles::new(workspace_root).assets_for(deck_id))
}

pub fn normalized_extension(path: &Path, text: bool) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|value| {
            !value.is_empty()
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        });
    extension.unwrap_or_else(|| if text { "txt" } else { "bin" }.to_string())
}

pub fn object_name(bytes: &[u8], extension: &str) -> String {
    let digest = Sha256::digest(bytes);
    format!(
        "{DIGEST_PREFIX}{}.{extension}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

pub fn write_object(
    workspace_root: &Path,
    deck_id: &str,
    bytes: &[u8],
    extension: &str,
) -> Result<PathBuf, AssetError> {
    let directory = deck_dir(workspace_root, deck_id)?;
    let name = object_name(bytes, extension);
    let path = directory.join(&name);
    if path.is_file() {
        verify_object(&path)?;
        return Ok(path);
    }
    std::fs::create_dir_all(&directory).map_err(|source| AssetError::Io {
        path: directory.clone(),
        source,
    })?;
    let tmp = directory.join(format!(".{name}.tmp"));
    crate::fsio::replace_file(&tmp, &path, bytes).map_err(|source| AssetError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn freeze_member(path: &Path) -> Result<FreezeReport, AssetError> {
    let workspace_root = crate::workspace::root_for_deck(path)
        .ok_or_else(|| AssetError::NotWorkspaceMember(path.to_path_buf()))?;
    let (_, _, defaults, _) =
        crate::workspace::read_manifest(&workspace_root.join(crate::workspace::MANIFEST));
    let deck = Deck::load_with_defaults(path, &defaults).map_err(|source| AssetError::Deck {
        path: path.to_path_buf(),
        source,
    })?;
    let deck_id = deck
        .deck_token
        .as_deref()
        .ok_or_else(|| AssetError::MissingDeckId(path.to_path_buf()))?;
    let owned_dir = deck_dir(workspace_root, deck_id)?;
    let owned_dir_existed = owned_dir.exists();
    let root_existed = workspace_root.join(ROOT).exists();
    let result = freeze_member_inner(workspace_root, &deck);
    if result.is_err() && !owned_dir_existed {
        let _ = std::fs::remove_dir_all(&owned_dir);
        if !root_existed {
            let _ = std::fs::remove_dir(workspace_root.join(ROOT));
        }
    }
    result
}

pub fn initialize(path: &Path) -> Result<InitializeReport, InitializeError> {
    let original = std::fs::read(path).map_err(|source| InitializeError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let stamp = crate::stamp::stamp_deck(path)?;
    let freeze = if crate::workspace::root_for_deck(path).is_some() {
        match freeze_member(path) {
            Ok(report) => Some(report),
            Err(error) => {
                let tmp = path.with_extension("md.restore.tmp");
                crate::fsio::replace_file(&tmp, path, &original).map_err(|source| {
                    InitializeError::Restore {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
                return Err(error.into());
            }
        }
    } else {
        None
    };
    Ok(InitializeReport { stamp, freeze })
}

fn freeze_member_inner(workspace_root: &Path, deck: &Deck) -> Result<FreezeReport, AssetError> {
    let text = read_text(&deck.path)?;
    let parsed = crate::parser::parse(&deck.subject, &text).map_err(|source| AssetError::Deck {
        path: deck.path.clone(),
        source: DeckError::Parse {
            path: deck.path.clone(),
            source,
        },
    })?;
    let deck_id = deck
        .deck_token
        .as_deref()
        .ok_or_else(|| AssetError::MissingDeckId(deck.path.clone()))?;
    if deck_dir(workspace_root, deck_id)?.is_dir() {
        validate_owned_dir(workspace_root, deck_id)?;
    }
    let inputs = declared_source_inputs(workspace_root, deck)?;
    let source_base = SourceBase::for_deck(deck);
    let mut evidence: Vec<String> = Vec::new();
    let mut evidence_seen: HashSet<String> = HashSet::new();
    let mut citations = Vec::new();

    for card in &deck.cards {
        for citation in &card.citations {
            if let Some(asset) = &citation.asset {
                if evidence_seen.insert(asset.clone()) {
                    evidence.push(asset.clone());
                }
                continue;
            }
            let excerpt = citation_excerpt(&source_base, citation)?;
            let input_index = source_owner(&inputs, &excerpt).ok_or_else(|| {
                AssetError::CitationOutsideSource {
                    locator: citation.locator.clone(),
                }
            })?;
            let bytes = canonical_excerpt_bytes(&excerpt);
            let extension = normalized_extension(&excerpt.path, true);
            let object = write_object(workspace_root, deck_id, &bytes, &extension)?;
            let asset = file_name(&object)?;
            if evidence_seen.insert(asset.clone()) {
                evidence.push(asset.clone());
            }
            citations.push(AtRewrite {
                at: excerpt_provenance(&excerpt, &inputs[input_index]),
                fingerprint: Some(crate::source::excerpt_fingerprint(&excerpt)),
                asset: Some(asset),
                line: citation.line,
            });
        }
    }

    let mut image_rewrites = Vec::new();
    let mut image_objects = HashSet::new();
    let boundaries = source_boundaries(workspace_root, &inputs);
    for image in crate::parser::image_references(&text) {
        if crate::deck::is_url(&image.source) {
            continue;
        }
        let source = resolve_image_source(workspace_root, &image.source)?;
        if !boundaries
            .iter()
            .any(|boundary| source.starts_with(boundary))
        {
            return Err(AssetError::ImageOutsideBoundary(source));
        }
        if !source.is_file() {
            return Err(AssetError::MissingImage(source));
        }
        let bytes = read_bytes(&source)?;
        let extension = normalized_extension(&source, false);
        let object = write_object(workspace_root, deck_id, &bytes, &extension)?;
        let name = file_name(&object)?;
        image_objects.insert(name.clone());
        image_rewrites.push((image.destination, format!("{ROOT}/{deck_id}/{name}")));
    }

    let rewritten = crate::deck::rewrite_frozen_assets(
        &text,
        parsed.frontmatter_span,
        None,
        &citations,
        &image_rewrites,
    )
    .map_err(|source| AssetError::Deck {
        path: deck.path.clone(),
        source,
    })?;
    let verified =
        crate::parser::parse(&deck.subject, &rewritten).map_err(|source| AssetError::Deck {
            path: deck.path.clone(),
            source: DeckError::Parse {
                path: deck.path.clone(),
                source,
            },
        })?;
    if verified.deck_token.as_deref() != Some(deck_id) {
        return Err(AssetError::MissingDeckId(deck.path.clone()));
    }
    crate::deck::write_deck_text(&deck.path, &rewritten).map_err(|source| AssetError::Deck {
        path: deck.path.clone(),
        source,
    })?;
    Ok(FreezeReport {
        evidence: evidence.len(),
        images: image_objects.len(),
    })
}

// A URL source grounds the exam and tutor but holds no freezable bytes;
// `base_locals` keeps only local sources, layered exactly like citation
// resolution (ADR 0026).
fn declared_source_inputs(
    workspace_root: &Path,
    deck: &Deck,
) -> Result<Vec<SourceInput>, AssetError> {
    let mut inputs = Vec::new();
    let layers = deck.source_layers();
    for source in layers.base_locals() {
        if let Some(path) = crate::source::source_path(source, Some(workspace_root)) {
            let path = path
                .canonicalize()
                .map_err(|_| AssetError::MissingSource(path.clone()))?;
            if path.is_file() {
                let bytes = read_bytes(&path)?;
                std::str::from_utf8(&bytes).map_err(|_| AssetError::NonTextSource(path.clone()))?;
                inputs.push(SourceInput::File { path });
            } else if path.is_dir() {
                inputs.push(SourceInput::Directory { path });
            } else {
                return Err(AssetError::MissingSource(path));
            }
        }
    }
    Ok(inputs)
}

fn citation_excerpt(
    source_base: &SourceBase,
    citation: &crate::card::SourceCitation,
) -> Result<Excerpt, AssetError> {
    match source_base
        .inspect_citation(citation)
        .map_err(|error| AssetError::Citation {
            locator: citation.locator.clone(),
            message: format!("{error:#}"),
        })? {
        CitationIntegrity::Current(excerpt)
        | CitationIntegrity::Unfingerprinted { excerpt, .. } => Ok(excerpt),
        CitationIntegrity::Relocated { locator, .. } => Err(AssetError::Citation {
            locator: citation.locator.clone(),
            message: format!("the excerpt moved to `{locator}`"),
        }),
        CitationIntegrity::Changed => Err(AssetError::Citation {
            locator: citation.locator.clone(),
            message: "the excerpt changed or disappeared".to_string(),
        }),
        CitationIntegrity::Ambiguous { locators } => Err(AssetError::Citation {
            locator: citation.locator.clone(),
            message: format!(
                "the excerpt matches several ranges: {}",
                locators.join(", ")
            ),
        }),
    }
}

fn source_owner(inputs: &[SourceInput], excerpt: &Excerpt) -> Option<usize> {
    let path = excerpt.path.canonicalize().ok()?;
    inputs.iter().position(|input| match input {
        SourceInput::File { path: source, .. } => path == *source,
        SourceInput::Directory { path: source, .. } => path.starts_with(source),
    })
}

fn canonical_excerpt_bytes(excerpt: &Excerpt) -> Vec<u8> {
    let mut text = excerpt
        .lines
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text.into_bytes()
}

fn excerpt_provenance(excerpt: &Excerpt, input: &SourceInput) -> String {
    let first = excerpt.lines.first().map(|line| line.0).unwrap_or(1);
    let last = excerpt.lines.last().map(|line| line.0).unwrap_or(first);
    let path = match input {
        SourceInput::File { path, .. } => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        SourceInput::Directory { path, .. } => excerpt
            .path
            .canonicalize()
            .unwrap_or_else(|_| excerpt.path.clone())
            .strip_prefix(path)
            .unwrap_or(&excerpt.path)
            .to_string_lossy()
            .into_owned(),
    };
    if first == last {
        format!("{path}:{first}")
    } else {
        format!("{path}:{first}-{last}")
    }
}

fn source_boundaries(workspace_root: &Path, inputs: &[SourceInput]) -> Vec<PathBuf> {
    let mut boundaries = vec![
        workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf()),
    ];
    for input in inputs {
        let boundary = match input {
            SourceInput::File { path, .. } => path.parent().unwrap_or(path),
            SourceInput::Directory { path, .. } => path,
        };
        if !boundaries.contains(&boundary.to_path_buf()) {
            boundaries.push(boundary.to_path_buf());
        }
    }
    boundaries
}

fn resolve_image_source(workspace_root: &Path, source: &str) -> Result<PathBuf, AssetError> {
    let path = Path::new(source);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    path.canonicalize()
        .map_err(|_| AssetError::MissingImage(path))
}

pub fn validate_member(deck: &Deck) -> Result<(), AssetError> {
    let workspace_root = crate::workspace::root_for_deck(&deck.path)
        .ok_or_else(|| AssetError::NotWorkspaceMember(deck.path.clone()))?;
    validate_at_root(deck, workspace_root)
}

pub fn validate_at_root(deck: &Deck, root: &Path) -> Result<(), AssetError> {
    let deck_id = deck
        .deck_token
        .as_deref()
        .ok_or_else(|| AssetError::MissingDeckId(deck.path.clone()))?;
    let owned = deck_dir(root, deck_id)?;
    for card in &deck.cards {
        for citation in &card.citations {
            if let Some(asset) = citation.asset.as_deref() {
                let path = owned.join(asset);
                if !path.is_file() {
                    return Err(AssetError::MissingAsset(path));
                }
            }
        }
    }
    if owned.is_dir() {
        validate_owned_dir(root, deck_id)?;
    }
    Ok(())
}

pub fn validate_owned_dir(root: &Path, deck_id: &str) -> Result<(), AssetError> {
    let owned = deck_dir(root, deck_id)?;
    for entry in std::fs::read_dir(&owned).map_err(|source| AssetError::Io {
        path: owned.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| AssetError::Io {
            path: owned.clone(),
            source,
        })?;
        let path = entry.path();
        if !path.is_file() {
            return Err(AssetError::InvalidName(path.display().to_string()));
        }
        verify_object(&path)?;
    }
    Ok(())
}

pub fn validate_image(deck: &Deck, source: &str) -> Result<(), AssetError> {
    let workspace_root = crate::workspace::root_for_deck(&deck.path)
        .ok_or_else(|| AssetError::NotWorkspaceMember(deck.path.clone()))?;
    validate_image_at_root(deck, workspace_root, source)
}

pub fn validate_image_at_root(deck: &Deck, root: &Path, source: &str) -> Result<(), AssetError> {
    let deck_id = deck
        .deck_token
        .as_deref()
        .ok_or_else(|| AssetError::MissingDeckId(deck.path.clone()))?;
    let path = resolve_image_source(root, source)?;
    let owned = deck_dir(root, deck_id)?;
    let canonical_owned = owned.canonicalize().map_err(|source| AssetError::Io {
        path: owned,
        source,
    })?;
    if !path.starts_with(&canonical_owned) {
        return Err(AssetError::ImageOutsideBoundary(path));
    }
    verify_object(&path)
}

fn read_text(path: &Path) -> Result<String, AssetError> {
    std::fs::read_to_string(path).map_err(|source| AssetError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, AssetError> {
    std::fs::read(path).map_err(|source| AssetError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn file_name(path: &Path) -> Result<String, AssetError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| AssetError::InvalidName(path.display().to_string()))
}

pub fn verify_object(path: &Path) -> Result<(), AssetError> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AssetError::InvalidName(path.display().to_string()))?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AssetError::InvalidName(name.to_string()))?;
    let stem = name
        .strip_suffix(&format!(".{extension}"))
        .and_then(|value| value.strip_prefix(DIGEST_PREFIX))
        .filter(|value| value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| AssetError::InvalidName(name.to_string()))?;
    let bytes = std::fs::read(path).map_err(|source| AssetError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let expected = object_name(&bytes, extension);
    if expected == name && stem.chars().all(|c| !c.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(AssetError::DigestMismatch {
            path: path.to_path_buf(),
        })
    }
}

pub fn is_object_name(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let Some(digest) = stem.strip_prefix(DIGEST_PREFIX) else {
        return false;
    };
    digest.len() == 64
        && digest
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        && !extension.is_empty()
        && extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric() && !character.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("decks")).unwrap();
        std::fs::write(directory.path().join("alix.toml"), "").unwrap();
        directory
    }

    #[test]
    fn object_names_are_exact_byte_sha256_addresses() {
        assert_eq!(
            "sha256-ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.rs",
            object_name(b"abc", "rs")
        );
        assert_ne!(object_name(b"abc", "rs"), object_name(b"abc\n", "rs"));
    }

    #[test]
    fn extensions_are_normalized_and_untrusted_values_fall_back() {
        assert_eq!("png", normalized_extension(Path::new("diagram.PNG"), false));
        assert_eq!(
            "txt",
            normalized_extension(Path::new("source.weird-name"), true)
        );
        assert_eq!("bin", normalized_extension(Path::new("image"), false));
    }

    #[test]
    fn identical_objects_in_one_deck_reuse_the_same_path() {
        let directory = tempfile::tempdir().unwrap();
        let first = write_object(directory.path(), "deck-deck1", b"same", "txt").unwrap();
        let second = write_object(directory.path(), "deck-deck1", b"same", "txt").unwrap();

        assert_eq!(first, second);
        assert_eq!(b"same", std::fs::read(first).unwrap().as_slice());
    }

    #[test]
    fn identical_objects_in_two_decks_have_distinct_owned_paths() {
        let directory = tempfile::tempdir().unwrap();
        let first = write_object(directory.path(), "deck-deck1", b"same", "txt").unwrap();
        let second = write_object(directory.path(), "deck-deck2", b"same", "txt").unwrap();

        assert_ne!(first, second);
        assert_eq!(first.file_name(), second.file_name());
    }

    #[test]
    fn an_existing_corrupt_object_is_rejected_instead_of_reused() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_object(directory.path(), "deck-deck1", b"original", "txt").unwrap();
        std::fs::write(&path, "changed").unwrap();

        let error = write_object(directory.path(), "deck-deck1", b"original", "txt").unwrap_err();
        assert!(matches!(error, AssetError::DigestMismatch { .. }));
    }

    #[test]
    fn object_validation_rejects_noncanonical_names() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("human-name.png");
        std::fs::write(&path, "bytes").unwrap();

        assert!(matches!(
            verify_object(&path),
            Err(AssetError::InvalidName(_))
        ));
        assert!(!is_object_name("human-name.png"));
    }

    #[test]
    fn freezing_a_file_source_freezes_the_cited_excerpt_and_ingests_images() {
        let directory = workspace();
        let source = directory.path().join("notes.md");
        std::fs::write(&source, "one \r\ntwo\r\nthree\r\n").unwrap();
        std::fs::write(directory.path().join("diagram.PNG"), [0, 1, 2, 255]).unwrap();
        let path = directory.path().join("decks/facts.md");
        let text = format!(
            "---\nid: \"deck-deck1\"\nsource: {}\n---\n## q\n![d](diagram.PNG)\na\n<!-- at: notes.md:2 -->\n",
            crate::parser::yaml_quote(&source.display().to_string())
        );
        std::fs::write(&path, &text).unwrap();

        let report = freeze_member(&path).unwrap();
        let frozen = std::fs::read_to_string(&path).unwrap();
        let deck = Deck::load(&path).unwrap();

        assert_eq!(
            FreezeReport {
                evidence: 1,
                images: 1
            },
            report
        );
        assert!(deck.is_frozen());
        assert_eq!(
            vec![source.display().to_string()],
            deck.sources,
            "the live source declaration stays untouched"
        );
        assert!(
            !frozen.contains("origin:"),
            "freezing stamps nothing: {frozen}"
        );
        let image_name = object_name(&[0, 1, 2, 255], "png");
        assert!(frozen.contains(&format!("](assets/deck-deck1/{image_name})")));
        assert_eq!(
            [0, 1, 2, 255],
            std::fs::read(
                directory
                    .path()
                    .join(format!("assets/deck-deck1/{image_name}"))
            )
            .unwrap()
            .as_slice()
        );
        let whole_name = object_name(b"one \r\ntwo\r\nthree\r\n", "md");
        assert!(
            !directory
                .path()
                .join(format!("assets/deck-deck1/{whole_name}"))
                .exists(),
            "no whole-file object is written"
        );
        let excerpt_name = object_name(b"two\n", "md");
        assert!(
            frozen.contains("<!-- at: notes.md:2 fingerprint: xxh64-")
                && frozen.contains(&format!(" asset: {excerpt_name} -->")),
            "{frozen}"
        );
        assert_eq!(
            b"two\n",
            std::fs::read(
                directory
                    .path()
                    .join(format!("assets/deck-deck1/{excerpt_name}"))
            )
            .unwrap()
            .as_slice()
        );
    }

    #[test]
    fn a_frozen_file_source_citation_reads_asset_bytes_after_the_live_file_mutates() {
        let directory = workspace();
        let source = directory.path().join("notes.md");
        std::fs::write(&source, "one\ntwo\nthree\n").unwrap();
        let path = directory.path().join("decks/facts.md");
        std::fs::write(
            &path,
            format!(
                "---\nid: \"deck-deck5\"\nsource: {}\n---\n## q\na\n<!-- at: notes.md:2 -->\n",
                crate::parser::yaml_quote(&source.display().to_string())
            ),
        )
        .unwrap();
        freeze_member(&path).unwrap();
        std::fs::write(&source, "MUTATED\nLIVE\nFILE\n").unwrap();

        let deck = Deck::load(&path).unwrap();
        let citation = &deck.cards[0].citations[0];
        assert_eq!("notes.md:2", citation.locator);
        assert!(citation.asset.is_some());
        let CitationIntegrity::Current(excerpt) = SourceBase::for_deck(&deck)
            .inspect_citation(citation)
            .unwrap()
        else {
            panic!("the frozen citation must stay Current against the asset");
        };
        assert_eq!(vec![(2, "two".to_string())], excerpt.lines);
    }

    #[test]
    fn freezing_a_directory_stores_only_the_cited_lines_of_cited_files() {
        let directory = workspace();
        let source = directory.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("lib.rs"), "a  \nb\nc\n").unwrap();
        std::fs::write(source.join("uncited.rs"), "secret\n").unwrap();
        let path = directory.path().join("decks/trace.md");
        std::fs::write(
            &path,
            format!(
                "---\nid: \"deck-deck2\"\nsource: {}\n---\n## q\np\n<!-- at: lib.rs:2-3 -->\n",
                crate::parser::yaml_quote(&source.display().to_string())
            ),
        )
        .unwrap();

        let report = freeze_member(&path).unwrap();
        let deck = Deck::load(&path).unwrap();

        assert_eq!(
            FreezeReport {
                evidence: 1,
                images: 0
            },
            report
        );
        assert_eq!(
            vec![source.display().to_string()],
            deck.sources,
            "the live source declaration stays untouched"
        );
        let evidence_name = object_name(b"b\nc\n", "rs");
        let object = directory
            .path()
            .join(format!("assets/deck-deck2/{evidence_name}"));
        assert_eq!(b"b\nc\n", std::fs::read(&object).unwrap().as_slice());
        let frozen_asset = std::fs::read_to_string(&object).unwrap();
        assert!(!frozen_asset.contains("a  "));
        let frozen = std::fs::read_to_string(&path).unwrap();
        assert!(!frozen.contains("uncited.rs"));
        assert!(!frozen.contains("secret"));
        assert!(
            frozen.contains("<!-- at: lib.rs:2-3 fingerprint: xxh64-")
                && frozen.contains(&format!(" asset: {evidence_name} -->")),
            "{frozen}"
        );
        assert_eq!(
            1,
            std::fs::read_dir(directory.path().join("assets/deck-deck2"))
                .unwrap()
                .count()
        );
    }

    #[test]
    fn a_frozen_directory_source_citation_reads_asset_bytes_after_the_live_file_mutates() {
        let directory = workspace();
        let source = directory.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("lib.rs"), "a\nb\nc\n").unwrap();
        let path = directory.path().join("decks/trace.md");
        std::fs::write(
            &path,
            format!(
                "---\nid: \"deck-deck6\"\nsource: {}\n---\n## q\np\n<!-- at: lib.rs:2-3 -->\n",
                crate::parser::yaml_quote(&source.display().to_string())
            ),
        )
        .unwrap();
        freeze_member(&path).unwrap();
        std::fs::write(source.join("lib.rs"), "REWRITTEN\n").unwrap();

        let deck = Deck::load(&path).unwrap();
        let citation = &deck.cards[0].citations[0];
        assert_eq!("lib.rs:2-3", citation.locator);
        let CitationIntegrity::Current(excerpt) = SourceBase::for_deck(&deck)
            .inspect_citation(citation)
            .unwrap()
        else {
            panic!("the frozen citation must stay Current against the asset");
        };
        assert_eq!(
            vec![(2, "b".to_string()), (3, "c".to_string())],
            excerpt.lines
        );
    }

    #[test]
    fn freezing_a_citation_beyond_the_display_cap_keeps_every_line_and_the_authored_range() {
        let directory = workspace();
        let source = directory.path().join("notes.md");
        let body: String = (1..=120).map(|line| format!("line {line}\n")).collect();
        std::fs::write(&source, &body).unwrap();
        let path = directory.path().join("decks/big.md");
        std::fs::write(
            &path,
            format!(
                "---\nid: \"deck-deck8\"\nsource: {}\n---\n## q\na\n<!-- at: notes.md:5-104 -->\n",
                crate::parser::yaml_quote(&source.display().to_string())
            ),
        )
        .unwrap();

        freeze_member(&path).unwrap();

        let cited: String = (5..=104).map(|line| format!("line {line}\n")).collect();
        let evidence_name = object_name(cited.as_bytes(), "md");
        assert_eq!(
            cited.as_bytes(),
            std::fs::read(
                directory
                    .path()
                    .join(format!("assets/deck-deck8/{evidence_name}"))
            )
            .unwrap()
            .as_slice()
        );
        let frozen = std::fs::read_to_string(&path).unwrap();
        assert!(
            frozen.contains("<!-- at: notes.md:5-104 fingerprint: xxh64-")
                && frozen.contains(&format!(" asset: {evidence_name} -->")),
            "the authored range must survive freezing verbatim: {frozen}"
        );

        std::fs::write(&source, "MUTATED\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        let CitationIntegrity::Current(excerpt) = SourceBase::for_deck(&deck)
            .inspect_citation(&deck.cards[0].citations[0])
            .unwrap()
        else {
            panic!("the frozen citation must verify over all 100 lines");
        };
        assert_eq!(100, excerpt.lines.len());
        assert_eq!((5, "line 5".to_string()), excerpt.lines[0]);
        assert_eq!((104, "line 104".to_string()), excerpt.lines[99]);
    }

    #[test]
    fn freezing_without_citations_writes_no_assets_and_keeps_the_source_live() {
        let directory = workspace();
        let source = directory.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("lib.rs"), "a\n").unwrap();
        let path = directory.path().join("decks/plain.md");
        let text = format!(
            "---\nid: \"deck-deck3\"\nsource: {}\n---\n## q\na\n",
            crate::parser::yaml_quote(&source.display().to_string())
        );
        std::fs::write(&path, &text).unwrap();

        let report = freeze_member(&path).unwrap();

        assert_eq!(
            FreezeReport {
                evidence: 0,
                images: 0
            },
            report
        );
        assert!(!directory.path().join("assets/deck-deck3").exists());
        let deck = Deck::load(&path).unwrap();
        assert!(!deck.is_frozen());
        assert_eq!(
            vec![source.display().to_string()],
            deck.sources,
            "the live source declaration stays untouched"
        );
    }

    #[test]
    fn a_member_with_no_own_source_initializes_and_freezes_from_the_workspace_source() {
        let directory = workspace();
        std::fs::write(
            directory.path().join("alix.toml"),
            "source = \"notes.md\"\n",
        )
        .unwrap();
        std::fs::write(directory.path().join("notes.md"), "one\ntwo\nthree\n").unwrap();
        let path = directory.path().join("decks/facts.md");
        std::fs::write(
            &path,
            "---\nid: \"deck-deck7\"\n---\n## q\na\n<!-- at: notes.md:2 -->\n<!-- id: card-card7 -->\n",
        )
        .unwrap();

        let report = initialize(&path).unwrap();

        assert_eq!(
            Some(FreezeReport {
                evidence: 1,
                images: 0
            }),
            report.freeze
        );
        let excerpt_name = object_name(b"two\n", "md");
        assert_eq!(
            b"two\n",
            std::fs::read(
                directory
                    .path()
                    .join(format!("assets/deck-deck7/{excerpt_name}"))
            )
            .unwrap()
            .as_slice()
        );
        let frozen = std::fs::read_to_string(&path).unwrap();
        assert!(
            frozen.contains("<!-- at: notes.md:2 fingerprint: xxh64-")
                && frozen.contains(&format!(" asset: {excerpt_name} -->")),
            "{frozen}"
        );
    }

    #[test]
    fn a_missing_image_keeps_the_deck_and_removes_new_assets() {
        let directory = workspace();
        let source = directory.path().join("notes.md");
        std::fs::write(&source, "a\n").unwrap();
        let path = directory.path().join("decks/facts.md");
        let text = format!(
            "---\nid: \"deck-deck4\"\nsource: {}\n---\n## q\n![d](missing.png)\na\n",
            crate::parser::yaml_quote(&source.display().to_string())
        );
        std::fs::write(&path, &text).unwrap();

        let error = freeze_member(&path).unwrap_err();

        assert!(matches!(error, AssetError::MissingImage(_)));
        assert_eq!(text, std::fs::read_to_string(&path).unwrap());
        assert!(!directory.path().join("assets/deck-deck4").exists());
    }

    #[test]
    fn failed_initialization_restores_the_exact_uninitialized_deck() {
        let directory = workspace();
        let path = directory.path().join("decks/remote.md");
        let original =
            b"---\nsource: https://example.test/source.md\n---\n## q\na\n<!-- at: notes.md:1 -->\n";
        std::fs::write(&path, original).unwrap();

        let error = initialize(&path).unwrap_err();

        assert!(matches!(
            error,
            InitializeError::Freeze(AssetError::Citation { .. })
        ));
        assert_eq!(original, std::fs::read(&path).unwrap().as_slice());
        assert_eq!(None, Deck::load(&path).unwrap().deck_token);
        assert!(!directory.path().join(ROOT).exists());
    }

    #[test]
    fn a_url_source_blocks_no_initialization_and_contributes_no_frozen_inputs() {
        let directory = workspace();
        std::fs::write(directory.path().join("notes.md"), "one\ntwo\n").unwrap();
        let path = directory.path().join("decks/mixed.md");
        std::fs::write(
            &path,
            "---\nid: \"deck-deck1\"\nsource:\n  - https://example.test/page\n  - notes.md\n---\n\
             ## q\na\n<!-- at: notes.md:2 -->\n",
        )
        .unwrap();

        let report = freeze_member(&path).unwrap();

        assert_eq!(
            FreezeReport {
                evidence: 1,
                images: 0
            },
            report
        );
        assert!(Deck::load(&path).unwrap().is_frozen());

        let url_only = directory.path().join("decks/url-only.md");
        std::fs::write(
            &url_only,
            "---\nsource: https://example.test/page\n---\n## q\na\n",
        )
        .unwrap();
        let report = initialize(&url_only).unwrap();
        assert_eq!(
            Some(FreezeReport {
                evidence: 0,
                images: 0
            }),
            report.freeze
        );
    }
}
