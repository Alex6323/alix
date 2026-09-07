use std::hash::Hasher;
#[cfg(feature = "full")]
use std::io::Read;

#[cfg(feature = "full")]
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const SYNC_PULL_MANIFEST_VERSION: u32 = 1;

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

#[cfg(feature = "full")]
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
}
