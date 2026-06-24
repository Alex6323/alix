//! AI deck augmentation: a deliberate layer (`alix deck augment`) that lets an
//! LLM enrich a card's *presentation* without touching its identity or progress.
//! Three kinds, all generated up front and stored in one id-keyed cache, then
//! read at review time: choice-mode **distractors** (with the offline sampler in
//! [`crate::choice`] as fallback), a **note** (merged with the card's deck note
//! on reveal), and a pool of reworded question **variants** (a fresh one rotated
//! in as the front each time a card is shown). Each is an additive field on
//! [`Augmentation`].
//!
//! Everything here is **regenerable**, so the cache is best-effort: a missing,
//! corrupt, or future-versioned file just yields an empty cache rather than an
//! error — a bad cache must never block a review. It is keyed by the card's
//! identity hash, so editing a card's answer (which changes its id) is a cache
//! miss and silently regenerates, never serving stale options.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ask,
    config::{AiConfig, AskConfig},
};

/// The on-disk cache-format version. Bumped only if the persisted shape changes
/// incompatibly; because the cache is regenerable, a newer version is ignored
/// (an empty cache is returned) rather than refused.
const CURRENT_VERSION: u32 = 1;

/// The AI-derived presentation data for a single card, keyed in the cache by the
/// card's identity hash. Fields are additive: new augmentation kinds (e.g.
/// morphed question variants) become new optional fields here so old caches keep
/// loading.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Augmentation {
    /// Generated wrong-answer options for choice mode, in no particular order.
    /// Empty when none were generated (the caller falls back to offline
    /// sampling).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub distractors: Vec<String>,
    /// A generated note (trivia / context / a mnemonic), merged with the card's
    /// own deck note on reveal. `None` when none was generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Reworded phrasings of the question (each keeping the same answer), one of
    /// which is shown in place of the card's front at review time so the card
    /// can't be answered by recognizing a fixed wording. Empty when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<String>,
}

/// On-disk representation of the cache.
#[derive(Serialize, Deserialize)]
struct AugmentFile {
    /// Format version; defaults to the oldest for a file written before the
    /// field existed.
    #[serde(default = "default_version")]
    version: u32,
    /// Augmentations keyed by the decimal string of the card's identity hash
    /// (JSON object keys must be strings).
    cards: HashMap<String, Augmentation>,
}

/// A best-effort, id-keyed cache of AI augmentations for cards.
pub struct AugmentCache {
    path: PathBuf,
    cards: HashMap<u64, Augmentation>,
}

/// An error *saving* the cache. Loading never errors (see the module docs): a
/// cache that can't be read is simply treated as empty.
#[derive(Debug, Error)]
pub enum AugmentError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Format {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl AugmentCache {
    /// Opens the cache at `path`. A missing, unreadable, malformed, or
    /// newer-than-understood file yields an empty cache — the data is
    /// regenerable, so a bad cache must never fail a review.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let cards = load(&path).unwrap_or_default();
        Self { path, cards }
    }

    /// Saves the cache atomically (write to a temp file, then rename).
    pub fn save(&self) -> Result<(), AugmentError> {
        let io_err = |source| AugmentError::Io {
            path: self.path.clone(),
            source,
        };

        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err)?;
        }

        let file = AugmentFile {
            version: CURRENT_VERSION,
            cards: self
                .cards
                .iter()
                .map(|(id, aug)| (id.to_string(), aug.clone()))
                .collect(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|source| AugmentError::Format {
            path: self.path.clone(),
            source,
        })?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(io_err)?;
        std::fs::rename(&tmp, &self.path).map_err(io_err)?;
        Ok(())
    }

    /// The cached distractors for a card, or `None` when absent or empty — so
    /// the caller can fall back to offline sampling with a single check.
    pub fn distractors(&self, card_id: u64) -> Option<&[String]> {
        self.cards
            .get(&card_id)
            .map(|aug| aug.distractors.as_slice())
            .filter(|d| !d.is_empty())
    }

    /// Whether an augmentation is already cached for a card. Used to warm only
    /// the cards that still need it.
    pub fn contains(&self, card_id: u64) -> bool {
        self.cards.contains_key(&card_id)
    }

    /// Stores the distractors for a card, replacing any previous ones. Does not
    /// save.
    pub fn set_distractors(&mut self, card_id: u64, distractors: Vec<String>) {
        self.cards.entry(card_id).or_default().distractors = distractors;
    }

    /// The cached note for a card, if any.
    pub fn note(&self, card_id: u64) -> Option<&str> {
        self.cards.get(&card_id).and_then(|aug| aug.note.as_deref())
    }

    /// Stores a generated note for a card, replacing any previous one. Does not
    /// save.
    pub fn set_note(&mut self, card_id: u64, note: String) {
        self.cards.entry(card_id).or_default().note = Some(note);
    }

    /// The cached question variants for a card (a pool to rotate through), or
    /// `None` when absent or empty.
    pub fn variants(&self, card_id: u64) -> Option<&[String]> {
        self.cards
            .get(&card_id)
            .map(|aug| aug.variants.as_slice())
            .filter(|v| !v.is_empty())
    }

    /// Picks a question phrasing for a card from the pool of the authored
    /// `original` plus the cached variants, rotating by `seed` (a plain modulo).
    /// The original sits at index 0, so it stays in the rotation. `None` when no
    /// variants are cached — the caller then keeps the original front unchanged.
    pub fn pick_front(&self, card_id: u64, original: &str, seed: u64) -> Option<String> {
        let variants = self.variants(card_id)?;
        let pool_len = variants.len() + 1; // + the original at index 0
        let idx = (seed % pool_len as u64) as usize;
        Some(if idx == 0 {
            original.to_string()
        } else {
            variants[idx - 1].clone()
        })
    }

    /// Stores the question variants for a card, replacing any previous ones.
    /// Does not save.
    pub fn set_variants(&mut self, card_id: u64, variants: Vec<String>) {
        self.cards.entry(card_id).or_default().variants = variants;
    }

    /// The number of cards with cached augmentations.
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    /// Returns `true` if nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }
}

/// Loads the cache, returning `None` on any problem (missing/corrupt/newer file)
/// so [`AugmentCache::open`] can fall back to empty.
fn load(path: &Path) -> Option<HashMap<u64, Augmentation>> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: AugmentFile = serde_json::from_str(&text).ok()?;
    // A cache from a newer alix may hold a shape we'd mangle — ignore it and
    // regenerate rather than risk serving wrong options.
    if file.version > CURRENT_VERSION {
        return None;
    }
    let mut cards = HashMap::with_capacity(file.cards.len());
    for (key, aug) in file.cards {
        // Skip any key that isn't a u64 hash rather than discarding the whole
        // cache for one bad entry.
        if let Ok(id) = key.parse::<u64>() {
            cards.insert(id, aug);
        }
    }
    Some(cards)
}

/// Serde default for a cache file with no `version` field: the oldest format.
fn default_version() -> u32 {
    1
}

/// The cache path co-located with a given progress store, so the augmentations
/// live next to whatever store the review path uses (honoring `--store` and any
/// future per-workspace store).
pub fn augment_path_for(store_path: &Path) -> PathBuf {
    store_path.with_file_name("augment.json")
}

// ── Generation ───────────────────────────────────────────────────────────────
//
// Distractors come from one batched, tool-free Claude call over the cards that
// still need them, mirroring the exam's generate/grade shape: a synchronous core
// ([`generate`]) the interactive frontends run on a thread via [`spawn`]. The
// call is pure text transformation — no web or file tools — so its allowlist is
// cleared like exam remediation.

/// One card to generate distractors for.
#[derive(Clone, Debug)]
pub struct WarmItem {
    /// The card's identity hash (the cache key).
    pub id: u64,
    /// The question shown to the learner (the card front).
    pub question: String,
    /// The correct answer the distractors must be wrong about and distinct from.
    pub answer: String,
}

/// Generates up to `count` distractors per card in `items` with one batched,
/// tool-free call, optionally steered by `guidance` (the `--with` text). Returns
/// a map from card id to its validated distractors; cards the model produced
/// nothing usable for are omitted, so review falls back to offline sampling.
pub fn generate(
    items: &[WarmItem],
    count: usize,
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
) -> Result<HashMap<u64, Vec<String>>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let prompt = distractors_prompt(items, count, guidance);
    let raw = ask::run(&tool_free(ask_cfg), &prompt, &[])?;
    let parsed: HashMap<String, Vec<String>> =
        parse_json(&raw).context("parsing the generated distractors")?;

    let mut out = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        let Some(raw_options) = parsed.get(&index.to_string()) else {
            continue;
        };
        let cleaned = clean_distractors(raw_options, &item.answer, count);
        if !cleaned.is_empty() {
            out.insert(item.id, cleaned);
        }
    }
    Ok(out)
}

/// Generates one short note (trivia, context, or a mnemonic) per card in `items`,
/// optionally steered by `guidance`. Returns card id → note, omitting any the
/// model left blank.
pub fn generate_notes(
    items: &[WarmItem],
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
) -> Result<HashMap<u64, String>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let prompt = notes_prompt(items, guidance);
    let raw = ask::run(&tool_free(ask_cfg), &prompt, &[])?;
    let parsed: HashMap<String, String> =
        parse_json(&raw).context("parsing the generated notes")?;

    let mut out = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        if let Some(note) = parsed.get(&index.to_string()) {
            let note = note.trim();
            if !note.is_empty() {
                out.insert(item.id, note.to_string());
            }
        }
    }
    Ok(out)
}

/// Generates up to `count` reworded phrasings of each card's question, steered
/// by `guidance`, each keeping the **exact same answer**. Returns card id → a
/// pool of variants (rotated at review time); cards the model produced nothing
/// usable for are omitted.
pub fn generate_variants(
    items: &[WarmItem],
    count: usize,
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
) -> Result<HashMap<u64, Vec<String>>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let prompt = variants_prompt(items, count, guidance);
    let raw = ask::run(&tool_free(ask_cfg), &prompt, &[])?;
    let parsed: HashMap<String, Vec<String>> =
        parse_json(&raw).context("parsing the generated question variants")?;

    let mut out = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        let Some(raw_variants) = parsed.get(&index.to_string()) else {
            continue;
        };
        let cleaned = clean_variants(raw_variants, &item.question, count);
        if !cleaned.is_empty() {
            out.insert(item.id, cleaned);
        }
    }
    Ok(out)
}

/// A copy of `ask` with the tool allowlist cleared — generation is a pure text
/// call that needs no web or file access (like exam remediation).
fn tool_free(ask: &AskConfig) -> AskConfig {
    let mut cfg = ask.clone();
    cfg.allowed_tools.clear();
    cfg
}

/// Builds the [`AskConfig`] for a generation call from the base `[ask]` config
/// plus the `[ai]` overrides: the AI model (falling back to `[ask]`'s), the AI
/// timeout, and a cleared tool allowlist (generation is a pure text call that
/// needs no web or file access).
pub fn run_config(ai: &AiConfig, ask: &AskConfig) -> AskConfig {
    let mut cfg = ask.clone();
    if ai.model.is_some() {
        cfg.model = ai.model.clone();
    }
    cfg.timeout_secs = ai.timeout_secs;
    cfg.allowed_tools.clear();
    cfg
}

/// Builds the batched distractor prompt: each card as `index. question —
/// answer`, then a strict JSON output shape keyed by that index.
fn distractors_prompt(items: &[WarmItem], count: usize, guidance: Option<&str>) -> String {
    let mut s = format!(
        "You are writing distractors — plausible but incorrect options — for \
         multiple-choice flashcards.\n\n\
         For each card, give exactly {count} wrong answers that:\n\
         - are tempting to someone who only half-knows the material,\n\
         - match the form and length of the correct answer (a year competes \
         with years, a command with commands),\n\
         - are clearly incorrect — never a synonym or restatement of the correct \
         answer,\n\
         - are distinct from each other and from the correct answer.\n"
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nExtra guidance: {}\n", g.trim()));
    }
    s.push_str("\nCards (index. question — correct answer):\n");
    for (i, item) in items.iter().enumerate() {
        s.push_str(&format!(
            "{i}. {} — {}\n",
            one_line(&item.question),
            one_line(&item.answer)
        ));
    }
    let slots = vec!["\"...\""; count].join(", ");
    s.push_str(&format!(
        "\nOutput ONLY JSON in exactly this shape, no prose, no code fences — \
         the key is the card index, the value its {count} distractors:\n\
         {{\"0\": [{slots}], ...}}\n"
    ));
    s
}

/// Builds the batched notes prompt: one short note per card, keyed by index.
fn notes_prompt(items: &[WarmItem], guidance: Option<&str>) -> String {
    let mut s = String::from(
        "You are adding a short note to each flashcard — one or two sentences of \
         memorable trivia, context, or a mnemonic that makes the answer easier to \
         recall. Keep each note tight and factual, and do not simply restate the \
         answer.\n",
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nExtra guidance: {}\n", g.trim()));
    }
    s.push_str("\nCards (index. question — answer):\n");
    for (i, item) in items.iter().enumerate() {
        s.push_str(&format!(
            "{i}. {} — {}\n",
            one_line(&item.question),
            one_line(&item.answer)
        ));
    }
    s.push_str(
        "\nOutput ONLY JSON, no prose, no code fences — the key is the card index, \
         the value its note as a single string:\n{\"0\": \"...\", ...}\n",
    );
    s
}

/// Builds the batched variants prompt: rephrase each question, keep the answer.
fn variants_prompt(items: &[WarmItem], count: usize, guidance: Option<&str>) -> String {
    let mut s = format!(
        "You are rephrasing flashcard questions. For each card, give {count} \
         different ways to ask the SAME question — reworded enough that a learner \
         must read and understand it, yet such that the EXACT same answer still \
         applies. Do not change what is being asked, do not add or drop \
         information, and never reveal or hint at the answer.\n"
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nExtra guidance: {}\n", g.trim()));
    }
    s.push_str("\nCards (index. question — the answer it must still have):\n");
    for (i, item) in items.iter().enumerate() {
        s.push_str(&format!(
            "{i}. {} — {}\n",
            one_line(&item.question),
            one_line(&item.answer)
        ));
    }
    let slots = vec!["\"...\""; count].join(", ");
    s.push_str(&format!(
        "\nOutput ONLY JSON in exactly this shape, no prose, no code fences — the \
         key is the card index, the value its {count} rephrasings:\n\
         {{\"0\": [{slots}], ...}}\n"
    ));
    s
}

/// Trims, drops empties, drops a rephrasing identical to the original question
/// (whitespace- and case-insensitively) or to one already kept, and caps at
/// `count`.
fn clean_variants(raw: &[String], original: &str, count: usize) -> Vec<String> {
    let norm = |s: &str| one_line(s).to_lowercase();
    let mut seen = HashSet::new();
    seen.insert(norm(original));
    let mut out = Vec::new();
    for variant in raw {
        let trimmed = variant.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(norm(trimmed)) {
            out.push(trimmed.to_string());
            if out.len() == count {
                break;
            }
        }
    }
    out
}

/// Collapses runs of whitespace (incl. newlines) so a multi-line front or back
/// stays on one line in the prompt listing.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Trims, drops empties, drops anything equal (case-insensitively) to the
/// correct answer or to an already-kept option, and caps the result at `count`.
fn clean_distractors(raw: &[String], answer: &str, count: usize) -> Vec<String> {
    let norm = |s: &str| s.trim().to_lowercase();
    let mut seen = HashSet::new();
    seen.insert(norm(answer));
    let mut out = Vec::new();
    for option in raw {
        let trimmed = option.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(norm(trimmed)) {
            out.push(trimmed.to_string());
            if out.len() == count {
                break;
            }
        }
    }
    out
}

/// The substring from the first `{` to the last `}`, so a JSON object survives
/// code fences or surrounding prose (mirrors the exam parser).
fn extract_json(raw: &str) -> &str {
    match (raw.find('{'), raw.rfind('}')) {
        (Some(a), Some(b)) if b > a => &raw[a..=b],
        _ => raw,
    }
}

/// Parses `raw` (possibly fenced / wrapped in prose) into `T`.
fn parse_json<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T> {
    let json = extract_json(raw);
    serde_json::from_str(json)
        .with_context(|| format!("the model did not return valid JSON:\n{json}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{AiConfig, AskConfig},
        testutil::{ask_config, exec_lock, fake_reply},
    };

    #[test]
    fn open_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("augment.json"));
        assert!(cache.is_empty());
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");

        let mut cache = AugmentCache::open(&path);
        cache.set_distractors(42, vec!["wrong a".into(), "wrong b".into()]);
        cache.save().unwrap();

        let reloaded = AugmentCache::open(&path);
        assert_eq!(1, reloaded.len());
        assert_eq!(
            Some(["wrong a".to_string(), "wrong b".to_string()].as_slice()),
            reloaded.distractors(42)
        );
    }

    #[test]
    fn distractors_is_none_when_absent_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        assert_eq!(None, cache.distractors(1)); // absent
        cache.set_distractors(1, Vec::new());
        assert_eq!(None, cache.distractors(1)); // present but empty
        assert!(cache.contains(1));
    }

    #[test]
    fn corrupt_file_yields_empty_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(&path, "this is not json").unwrap();
        let cache = AugmentCache::open(&path);
        assert!(cache.is_empty());
    }

    #[test]
    fn newer_version_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(&path, r#"{"version":999,"cards":{}}"#).unwrap();
        let cache = AugmentCache::open(&path);
        assert!(cache.is_empty());
    }

    #[test]
    fn a_single_bad_key_does_not_drop_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(
            &path,
            r#"{"version":1,"cards":{"not-a-number":{"distractors":["x"]},"7":{"distractors":["y"]}}}"#,
        )
        .unwrap();
        let cache = AugmentCache::open(&path);
        assert_eq!(1, cache.len());
        assert_eq!(Some(["y".to_string()].as_slice()), cache.distractors(7));
    }

    #[test]
    fn file_without_version_field_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(&path, r#"{"cards":{"3":{"distractors":["z"]}}}"#).unwrap();
        let cache = AugmentCache::open(&path);
        assert_eq!(Some(["z".to_string()].as_slice()), cache.distractors(3));
    }

    #[test]
    fn augment_path_is_a_sibling_of_the_store() {
        let p = augment_path_for(Path::new("/data/alix/progress.json"));
        assert_eq!(Path::new("/data/alix/augment.json"), p);
    }

    #[test]
    fn set_distractors_replaces_previous() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        cache.set_distractors(1, vec!["old".into()]);
        cache.set_distractors(1, vec!["new a".into(), "new b".into()]);
        assert_eq!(
            Some(["new a".to_string(), "new b".to_string()].as_slice()),
            cache.distractors(1)
        );
    }

    #[test]
    fn note_roundtrips_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_note(7, "a memorable fact".into());
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(Some("a memorable fact"), reloaded.note(7));
        assert_eq!(None, reloaded.note(8));
    }

    #[test]
    fn variants_roundtrip_and_pick() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_variants(5, vec!["one".into(), "two".into(), "three".into()]);
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(3, reloaded.variants(5).unwrap().len());
        // pool = [original] + 3 variants = 4; idx = seed % 4, original at 0.
        assert_eq!(Some("ORIG".to_string()), reloaded.pick_front(5, "ORIG", 0));
        assert_eq!(Some("one".to_string()), reloaded.pick_front(5, "ORIG", 1));
        assert_eq!(Some("ORIG".to_string()), reloaded.pick_front(5, "ORIG", 4)); // 4 % 4 == 0
        assert_eq!(None, reloaded.pick_front(6, "ORIG", 0)); // no variants
    }

    // ── generation ──

    fn item(id: u64, question: &str, answer: &str) -> WarmItem {
        WarmItem {
            id,
            question: question.into(),
            answer: answer.into(),
        }
    }

    #[test]
    fn generate_parses_and_maps_each_card_by_index() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            r#"{"0": ["w1","w2","w3"], "1": ["x1","x2","x3"]}"#,
        );
        let items = vec![
            item(10, "Capital of France?", "Paris"),
            item(20, "2+2?", "4"),
        ];
        let out = generate(&items, 3, None, &ask_config(&cli)).unwrap();
        assert_eq!(vec!["w1", "w2", "w3"], out[&10]);
        assert_eq!(vec!["x1", "x2", "x3"], out[&20]);
    }

    #[test]
    fn generate_drops_options_equal_to_the_answer_and_dedups() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // "paris"/"Paris" equal the answer (case-insensitively); "Lyon" repeats.
        let cli = fake_reply(
            dir.path(),
            r#"{"0": ["paris","Lyon","Lyon","Nice","Paris"]}"#,
        );
        let out = generate(
            &[item(1, "Capital of France?", "Paris")],
            3,
            None,
            &ask_config(&cli),
        )
        .unwrap();
        assert_eq!(vec!["Lyon", "Nice"], out[&1]);
    }

    #[test]
    fn generate_caps_at_count() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": ["a","b","c","d","e"]}"#);
        let out = generate(&[item(1, "q", "z")], 3, None, &ask_config(&cli)).unwrap();
        assert_eq!(3, out[&1].len());
    }

    #[test]
    fn generate_omits_a_card_with_no_usable_distractor() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // Card 0's options all equal the answer -> nothing usable -> omitted.
        let cli = fake_reply(dir.path(), r#"{"0": ["4","4"], "1": ["x1"]}"#);
        let out = generate(
            &[item(1, "2+2", "4"), item(2, "q", "y")],
            3,
            None,
            &ask_config(&cli),
        )
        .unwrap();
        assert!(!out.contains_key(&1));
        assert_eq!(vec!["x1"], out[&2]);
    }

    #[test]
    fn generate_malformed_json_is_an_error() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "sorry, I can't do that");
        let err = generate(&[item(1, "q", "a")], 3, None, &ask_config(&cli)).unwrap_err();
        assert!(format!("{err:#}").contains("valid JSON"));
    }

    #[test]
    fn generate_with_no_items_makes_no_call() {
        // No real CLI: empty input must short-circuit to an empty map.
        let cfg = ask_config(Path::new("/nonexistent/claude"));
        assert!(generate(&[], 3, None, &cfg).unwrap().is_empty());
    }

    #[test]
    fn generate_notes_parses_each_card() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": "note a", "1": "note b"}"#);
        let items = vec![item(10, "q1", "a1"), item(20, "q2", "a2")];
        let out = generate_notes(&items, None, &ask_config(&cli)).unwrap();
        assert_eq!("note a", out[&10]);
        assert_eq!("note b", out[&20]);
    }

    #[test]
    fn generate_notes_omits_blank_notes() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": "   ", "1": "real note"}"#);
        let items = vec![item(1, "q", "a"), item(2, "q", "a")];
        let out = generate_notes(&items, None, &ask_config(&cli)).unwrap();
        assert!(!out.contains_key(&1));
        assert_eq!("real note", out[&2]);
    }

    #[test]
    fn generate_variants_drops_the_original_phrasing() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // The model echoes the original wording plus two genuine rewordings.
        let cli = fake_reply(
            dir.path(),
            r#"{"0": ["What year?", "In which year?", "Which year was it?"]}"#,
        );
        let out = generate_variants(&[item(1, "What year?", "1589")], 3, None, &ask_config(&cli))
            .unwrap();
        assert_eq!(vec!["In which year?", "Which year was it?"], out[&1]);
    }

    #[test]
    fn run_config_clears_tools_and_applies_ai_overrides() {
        let ask = AskConfig {
            model: Some("sonnet".into()),
            allowed_tools: vec!["WebFetch".into()],
            ..AskConfig::default()
        };
        let ai = AiConfig {
            model: Some("haiku".into()),
            distractor_count: 3,
            variant_count: 4,
            timeout_secs: 42,
        };
        let cfg = run_config(&ai, &ask);
        assert!(cfg.allowed_tools.is_empty());
        assert_eq!(Some("haiku".to_string()), cfg.model);
        assert_eq!(42, cfg.timeout_secs);
    }

    #[test]
    fn run_config_falls_back_to_the_ask_model() {
        let ask = AskConfig {
            model: Some("sonnet".into()),
            ..AskConfig::default()
        };
        let cfg = run_config(&AiConfig::default(), &ask);
        assert_eq!(Some("sonnet".to_string()), cfg.model);
    }
}
