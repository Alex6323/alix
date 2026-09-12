use std::{hash::Hasher, io::Read};

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const SYNC_PULL_MANIFEST_VERSION: u32 = 1;
pub const ROOT_ID_PREFIX: &str = "root-";

pub fn is_root_id(id: &str) -> bool {
    id.strip_prefix(ROOT_ID_PREFIX)
        .is_some_and(crate::token::is_canonical)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SyncPullManifest {
    pub version: u32,
    pub root_id: String,
    pub entry: String,
    pub kind: String,
    pub files: Vec<SyncFileDto>,
    pub decks: Vec<SyncDeckDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SyncFileDto {
    pub path: String,
    pub bytes: u64,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SyncDeckDto {
    pub path: String,
    pub deck_id: String,
    pub revision: Option<u64>,
}

pub fn digest(bytes: &[u8]) -> String {
    let mut hasher = twox_hash::XxHash64::default();
    hasher.write(bytes);
    format!("xxh64-{:016x}", hasher.finish())
}

/// Wire form (frozen): rows in byte-wise `path` order, each `path`, NUL,
/// `bytes` in decimal, NUL, `digest`, LF.
pub fn entry_digest(files: &[SyncFileDto]) -> String {
    let mut rows: Vec<&SyncFileDto> = files.iter().collect();
    rows.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let mut hasher = twox_hash::XxHash64::default();
    for file in rows {
        hasher.write(file.path.as_bytes());
        hasher.write(&[0]);
        hasher.write(file.bytes.to_string().as_bytes());
        hasher.write(&[0]);
        hasher.write(file.digest.as_bytes());
        hasher.write(b"\n");
    }
    format!("xxh64-{:016x}", hasher.finish())
}

pub fn digest_reader(mut reader: impl Read) -> Result<(u64, String)> {
    let mut hasher = twox_hash::XxHash64::default();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.write(&buffer[..read]);
        bytes = bytes
            .checked_add(read as u64)
            .ok_or_else(|| anyhow::anyhow!("sync input is too large to count"))?;
    }
    Ok((bytes, format!("xxh64-{:016x}", hasher.finish())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_nullable_revisions_and_rejects_unknown_fields() {
        let manifest = SyncPullManifest {
            version: SYNC_PULL_MANIFEST_VERSION,
            root_id: "root-00000000000000000000000000".to_string(),
            entry: "Biology".to_string(),
            kind: "workspace".to_string(),
            files: vec![SyncFileDto {
                path: "decks/cells.md".to_string(),
                bytes: 4,
                digest: digest(b"cell"),
            }],
            decks: vec![SyncDeckDto {
                path: "decks/cells.md".to_string(),
                deck_id: "deck-cells".to_string(),
                revision: None,
            }],
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();

        assert_eq!(
            manifest,
            serde_json::from_slice(&bytes).expect("the shared manifest schema must round trip")
        );
        let with_unknown = br#"{"version":1,"root_id":"root-00000000000000000000000000","entry":"Biology","kind":"workspace","files":[],"decks":[],"extra":true}"#;
        assert!(
            serde_json::from_slice::<SyncPullManifest>(with_unknown).is_err(),
            "unknown manifest fields must fail closed on desktop and phone"
        );
    }

    #[test]
    fn digest_is_the_canonical_xxh64_wire_value() {
        assert_eq!("xxh64-26c7827d889f6da3", digest(b"hello"));
    }

    fn row(path: &str, bytes: u64, digest: &str) -> SyncFileDto {
        SyncFileDto {
            path: path.to_string(),
            bytes,
            digest: digest.to_string(),
        }
    }

    #[test]
    fn entry_digest_hashes_the_frozen_row_form_in_path_order() {
        let rows = [
            row("decks/b.md", 12, "xxh64-00000000000000b0"),
            row("alix.toml", 3, "xxh64-00000000000000a0"),
        ];
        let expected = digest(
            b"alix.toml\x003\x00xxh64-00000000000000a0\ndecks/b.md\x0012\x00xxh64-00000000000000b0\n",
        );
        assert_eq!(expected, entry_digest(&rows), "row form and path order");
        let reversed = [rows[1].clone(), rows[0].clone()];
        assert_eq!(
            expected,
            entry_digest(&reversed),
            "input order is irrelevant"
        );
        assert_eq!(digest(b""), entry_digest(&[]), "no rows hash as no bytes");
    }

    #[test]
    fn entry_digest_moves_when_a_row_is_renamed_resized_or_rewritten() {
        let base = [row("decks/a.md", 5, "xxh64-0000000000000001")];
        let renamed = [row("decks/b.md", 5, "xxh64-0000000000000001")];
        let resized = [row("decks/a.md", 6, "xxh64-0000000000000001")];
        let rewritten = [row("decks/a.md", 5, "xxh64-0000000000000002")];
        let added = [
            base[0].clone(),
            row("decks/b.md", 1, "xxh64-0000000000000003"),
        ];
        let baseline = entry_digest(&base);
        for (label, rows) in [
            ("renamed", &renamed[..]),
            ("resized", &resized[..]),
            ("rewritten", &rewritten[..]),
            ("added", &added[..]),
        ] {
            assert_ne!(
                baseline,
                entry_digest(rows),
                "{label} must change the digest"
            );
        }
    }
}
