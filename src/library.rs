use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    augment::{self, AugmentCache},
    deck::Deck,
    parser, stamp,
    store::Store,
};

#[derive(Debug)]
pub struct Placed {
    pub path: PathBuf,
    pub cards: usize,
    pub parse_error: Option<String>,
}

pub fn place_deck(dir: &Path, name: &str, text: &str) -> Result<Placed> {
    let stem = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deck");
    let stem = stem.strip_suffix(".md").unwrap_or(stem);
    let file = format!("{stem}.md");
    let path = dir.join(&file);
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    let parsed = parser::parse_str(&file, text);
    write_body(&path, text)?;
    Ok(match parsed {
        Ok(cards) => {
            if let Err(e) = stamp::stamp_deck(&path) {
                eprintln!("warning: cannot stamp {}: {e}", path.display());
            }
            Placed {
                path,
                cards: cards.len(),
                parse_error: None,
            }
        }
        Err(e) => Placed {
            path,
            cards: 0,
            parse_error: Some(format!("{e:#}")),
        },
    })
}

fn write_body(path: &Path, text: &str) -> Result<()> {
    let body = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    };
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("deck");
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    let tmp = parent.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, body).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

#[derive(Debug)]
pub struct ReplaceReport {
    pub minted: usize,
    pub wiped_cards: usize,
}

pub fn replace_deck(
    dir: &Path,
    name: &str,
    text: &str,
    store: &mut Store,
) -> Result<ReplaceReport> {
    let stem = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deck");
    let stem = stem.strip_suffix(".md").unwrap_or(stem);
    let file = format!("{stem}.md");
    let path = dir.join(&file);

    if let Err(e) = parser::parse(&file, text) {
        let rej = dir.join(format!("{stem}.rej"));
        write_body(&rej, text)?;
        bail!(
            "the replacement for {} does not parse; wrote it aside to {} and left the existing file untouched: {e:#}",
            path.display(),
            rej.display()
        );
    }

    // Lenient on the OLD file: a corrupt old deck under-wipes rather than
    // blocking its replacement.
    let mut old_card_tokens: HashSet<String> = HashSet::new();
    let mut old_deck_tokens: HashSet<String> = HashSet::new();
    let old_text = std::fs::read_to_string(&path).unwrap_or_default();
    if let Ok(old) = parser::parse(&file, &old_text) {
        for card in &old.cards {
            if let Some(token) = card.token.as_deref() {
                old_card_tokens.insert(token.to_string());
            }
        }
        if let Some(token) = old.deck_token {
            old_deck_tokens.insert(token);
        }
    }

    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    if path.exists() {
        let bak = dir.join(format!("{file}.bak"));
        std::fs::rename(&path, &bak)
            .with_context(|| format!("cannot keep {} as {}", path.display(), bak.display()))?;
    }
    write_body(&path, text)?;

    // Loud but non-fatal; review-open stamps again.
    let minted = match stamp::stamp_deck(&path) {
        Ok(outcome) => outcome.minted_cards.len(),
        Err(e) => {
            eprintln!("warning: cannot stamp {}: {e}", path.display());
            0
        }
    };

    // The store saves first: a failed augment save then strands only
    // unreachable cache entries, never store orphans.
    let wiped_cards = store.wipe_deck(&old_card_tokens, &file);
    store
        .save()
        .context("saving the store after replacing a deck")?;
    let cache_path = augment::augment_path_for(store.path());
    if cache_path.exists() {
        let mut cache = AugmentCache::open(&cache_path);
        if cache.wipe_tokens(&old_card_tokens, &old_deck_tokens) {
            cache
                .save()
                .with_context(|| format!("cannot save {}", cache_path.display()))?;
        }
    }

    Ok(ReplaceReport {
        minted,
        wiped_cards,
    })
}

pub fn reset_decks<'a>(
    store: &mut Store,
    decks: impl IntoIterator<Item = &'a Deck>,
) -> Result<usize> {
    let mut n = 0;
    for deck in decks {
        store.clear_deck_mastered(&deck.subject);
        let virtual_ids: Vec<String> = store
            .virtual_cards_for(&deck.subject)
            .iter()
            .map(|vc| vc.id.clone())
            .collect();
        for id in virtual_ids {
            store.remove_virtual(&id);
            store.remove(&id);
        }
        for card in &deck.cards {
            let Some(id) = card.id() else { continue };
            if store.get(&id).is_some() {
                store.remove(&id);
                n += 1;
            }
        }
    }
    store.save()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placing_a_valid_deck_writes_it_and_counts_cards() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "rust", "## q\na\n").unwrap();
        assert_eq!(dir.path().join("rust.md"), p.path);
        assert_eq!(1, p.cards);
        assert!(p.parse_error.is_none());
        assert!(p.path.exists());
    }

    #[test]
    fn placed_decks_land_stamped() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "rust", "## q\na\n## r\nb\n").unwrap();
        let deck =
            crate::parser::parse("rust.md", &std::fs::read_to_string(&p.path).unwrap()).unwrap();
        assert!(deck.deck_token.is_some(), "deck id minted");
        assert!(
            deck.cards.iter().all(|c| c.id().is_some()),
            "every card stamped"
        );
    }

    #[test]
    fn a_parse_problem_still_writes_the_deck_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "broken.md", "## q with no answer\n").unwrap();
        assert!(p.path.exists());
        assert!(p.parse_error.is_some());
    }

    #[test]
    fn a_name_collision_errors_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rust.md"), "original").unwrap();
        let err = place_deck(dir.path(), "rust", "## q\na\n").unwrap_err();
        assert!(format!("{err:#}").contains("already exists"), "{err:#}");
        assert_eq!(
            "original",
            std::fs::read_to_string(dir.path().join("rust.md")).unwrap()
        );
    }

    #[test]
    fn an_uploaded_name_cannot_traverse_out_of_the_dir() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "../../evil", "## q\na\n").unwrap();
        assert!(p.path.starts_with(dir.path()), "{}", p.path.display());
    }

    #[test]
    fn resetting_a_deck_clears_only_that_decks_progress() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "## qa <!-- id: qa -->\nans-a\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "## qb <!-- id: qb -->\nans-b\n").unwrap();
        let deck_a = Deck::load(dir.path().join("a.md")).unwrap();
        let deck_b = Deck::load(dir.path().join("b.md")).unwrap();

        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert(&deck_a.cards[0].id().unwrap(), 0);
        store.get_or_insert(&deck_b.cards[0].id().unwrap(), 0);
        store.set_deck_mastered(&deck_a.subject, 0);

        let n = reset_decks(&mut store, [&deck_a]).unwrap();
        assert_eq!(1, n);
        assert!(
            store.get(&deck_a.cards[0].id().unwrap()).is_none(),
            "a's schedule wiped"
        );
        assert!(
            store.get(&deck_b.cards[0].id().unwrap()).is_some(),
            "b's schedule intact"
        );
        assert!(!store.deck_mastered(&deck_a.subject));
    }

    fn virtual_card(parent: &str, back: &str) -> crate::store::VirtualCard {
        let slug: String = back.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let text = format!(
            "## front <!-- id: v{} -->\n{back}\n",
            slug.to_ascii_lowercase()
        );
        let id = crate::parser::parse_str(parent, &text).unwrap()[0]
            .id()
            .unwrap();
        crate::store::VirtualCard {
            id,
            kind: crate::store::VirtualKind::Remediation,
            parent: parent.to_string(),
            text,
            created_ms: 0,
        }
    }

    fn write_deck(dir: &Path, name: &str, deck_token: &str, card_token: &str) {
        std::fs::write(
            dir.join(name),
            format!("---\nid: \"{deck_token}\"\n---\n## q <!-- id: {card_token} -->\nans\n"),
        )
        .unwrap();
    }

    #[test]
    fn unparseable_regeneration_aborts_before_touching_the_old_file_and_writes_a_rej() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let orig = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("c1", 0);
        store.save().unwrap();

        let err =
            replace_deck(dir.path(), "a", "## broken with no answer\n", &mut store).unwrap_err();

        assert!(format!("{err:#}").contains("does not parse"), "{err:#}");
        assert_eq!(
            orig,
            std::fs::read_to_string(dir.path().join("a.md")).unwrap()
        );
        assert!(!dir.path().join("a.md.bak").exists());
        let rej = std::fs::read_to_string(dir.path().join("a.rej")).unwrap();
        assert!(rej.contains("## broken with no answer"), "{rej}");
        assert!(store.get("c1").is_some());
    }

    #[test]
    fn the_replaced_deck_is_kept_as_a_bak_and_baks_are_not_decks() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let orig = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut store).unwrap();

        assert_eq!(
            orig,
            std::fs::read_to_string(dir.path().join("a.md.bak")).unwrap()
        );
        let now = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        assert!(now.contains("new q"), "{now}");
        let decks = crate::workspace::deck_files(dir.path());
        assert_eq!(1, decks.len(), "{decks:?}");
        assert!(decks[0].ends_with("a.md"));
    }

    #[test]
    fn replacing_a_deck_wipes_its_progress_augment_entries_and_parented_virtuals() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        write_deck(dir.path(), "b.md", "db1", "cb1");
        let store_path = dir.path().join("p.json");
        let mut store = Store::open(&store_path).unwrap();

        // Deck A: a card schedule, deck-family mastery, records, a parented virtual.
        store.get_or_insert("c1", 0);
        store.set_deck_mastered("a.md", 1);
        store.ensure_records_raw("c1", &[]);
        store.insert_virtual(crate::store::VirtualCard {
            id: "va1".into(),
            kind: crate::store::VirtualKind::Remediation,
            parent: "a.md".into(),
            text: "## v <!-- id: va1 -->\nvans\n".into(),
            created_ms: 0,
        });
        store.get_or_insert("va1", 0);
        // Deck B (shares the store): its own schedule + mastery.
        store.get_or_insert("cb1", 0);
        store.set_deck_mastered("b.md", 1);
        store.save().unwrap();

        // Augment cache beside the store: A's card entry + A's topology, plus B's.
        let cache_path = augment::augment_path_for(&store_path);
        let mut cache = AugmentCache::open(&cache_path);
        cache.set_distractors("c1", vec!["x".into()]);
        cache.add_topology(crate::augment::Topology {
            name: "auto".into(),
            deck_token: "da1".into(),
            ..Default::default()
        });
        cache.set_distractors("cb1", vec!["y".into()]);
        cache.add_topology(crate::augment::Topology {
            name: "auto".into(),
            deck_token: "db1".into(),
            ..Default::default()
        });
        cache.save().unwrap();

        let report = replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut store).unwrap();

        assert_eq!(1, report.wiped_cards);
        assert!(store.get("c1").is_none());
        assert!(!store.deck_mastered("a.md"));
        assert!(store.records("c1").is_none());
        assert!(store.get_virtual("va1").is_none());
        assert!(store.get("va1").is_none());
        assert!(store.get("cb1").is_some());
        assert!(store.deck_mastered("b.md"));

        let cache = AugmentCache::open(&cache_path);
        assert!(cache.distractors("c1").is_none());
        assert!(!cache.has_topology_for(&once("da1")));
        assert!(cache.distractors("cb1").is_some());
        assert!(cache.has_topology_for(&once("db1")));
    }

    #[test]
    fn a_replaced_deck_leaves_no_orphaned_store_keys() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("c1", 0);
        store.set_deck_mastered("a.md", 1);
        store.ensure_records_raw("c1", &[]);
        store.save().unwrap();

        replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut store).unwrap();

        let deck = Deck::load(dir.path().join("a.md")).unwrap();
        let known_ids: HashSet<String> = deck.cards.iter().filter_map(|c| c.id()).collect();
        let orphans = store.orphans(&known_ids, &once("a.md"));
        assert!(orphans.is_empty(), "{orphans:?}");
    }

    #[test]
    fn every_replacement_mints_fresh_tokens() {
        let dir = tempfile::tempdir().unwrap();
        place_deck(dir.path(), "a", "## old q\nold ans\n## old r\nold b\n").unwrap();
        let old_text = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        let old = crate::parser::parse("a.md", &old_text).unwrap();
        let old_tokens: Vec<String> = old
            .cards
            .iter()
            .filter_map(|c| c.token.as_deref().map(str::to_string))
            .chain(old.deck_token.clone())
            .collect();
        assert!(old_tokens.len() >= 3, "{old_tokens:?}");

        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        replace_deck(dir.path(), "a", "## new q\nnew ans\n", &mut store).unwrap();

        let now = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        for tok in &old_tokens {
            assert!(!now.contains(tok.as_str()), "old token {tok} reappeared");
        }
        let bak = std::fs::read_to_string(dir.path().join("a.md.bak")).unwrap();
        assert!(old_tokens.iter().all(|t| bak.contains(t.as_str())));
    }

    #[test]
    fn a_second_replace_overwrites_the_prior_bak() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "da1", "c1");
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        replace_deck(dir.path(), "a", "## first q\nfirst ans\n", &mut store).unwrap();
        let first = std::fs::read_to_string(dir.path().join("a.md")).unwrap();

        replace_deck(dir.path(), "a", "## second q\nsecond ans\n", &mut store).unwrap();

        let baks: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
            .collect();
        assert_eq!(1, baks.len(), "{baks:?}");
        assert_eq!(
            first,
            std::fs::read_to_string(dir.path().join("a.md.bak")).unwrap()
        );
    }

    /// One fixture: frontmatter without `id:`, a divided card (fence + note +
    /// escaped divider + trailing-space front), and a two-hole cloze card.
    const MARKER_FIXTURE: &str = "---\nsource: notes.md\nrequires: basics\n---\n# The Title\nintro prose\n\n## First question \nextra front line\n\n---\nthe answer\n\\--- escaped divider\n> a note\n```\nfenced\n## not a card\n```\ntail prose\n\n## Fill in the blanks\nthe \\blank{alpha} and \\blank{beta} here\n> cloze note\n";

    fn all_tokens(subject: &str, text: &str) -> Vec<String> {
        let deck = crate::parser::parse(subject, text).unwrap();
        let mut toks = Vec::new();
        let mut last_line = None;
        for card in &deck.cards {
            if last_line != Some(card.line) {
                if let Some(t) = card.token.as_deref() {
                    toks.push(t.to_string());
                }
                last_line = Some(card.line);
            }
        }
        toks.extend(deck.deck_token);
        toks
    }

    fn assert_no_duplicate_tokens(subject: &str, text: &str) {
        let toks = all_tokens(subject, text);
        let uniq: HashSet<&String> = toks.iter().collect();
        assert_eq!(
            toks.len(),
            uniq.len(),
            "duplicate token in {subject}: {toks:?}"
        );
    }

    #[test]
    fn every_writer_preserves_tokens_and_text_and_never_duplicates() {
        {
            let dir = tempfile::tempdir().unwrap();
            place_deck(dir.path(), "d", MARKER_FIXTURE).unwrap();
            let text = std::fs::read_to_string(dir.path().join("d.md")).unwrap();
            let deck = crate::parser::parse("d.md", &text).unwrap();
            assert!(deck.cards.iter().all(|c| c.token.is_some()), "all stamped");
            assert!(text.contains("First question"), "front text kept");
            assert!(
                text.contains("alpha") && text.contains("beta"),
                "cloze kept"
            );
            assert_no_duplicate_tokens("d.md", &text);
        }
        {
            let dir = tempfile::tempdir().unwrap();
            place_deck(dir.path(), "d", "## x\ny\n").unwrap();
            let mut store = Store::open(dir.path().join("p.json")).unwrap();
            replace_deck(dir.path(), "d", MARKER_FIXTURE, &mut store).unwrap();
            let text = std::fs::read_to_string(dir.path().join("d.md")).unwrap();
            let deck = crate::parser::parse("d.md", &text).unwrap();
            assert!(deck.cards.iter().all(|c| c.token.is_some()), "all stamped");
            assert!(text.contains("First question"));
            assert_no_duplicate_tokens("d.md", &text);
        }
        {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("d.md");
            std::fs::write(&path, MARKER_FIXTURE).unwrap();
            stamp::stamp_deck(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let deck = crate::parser::parse("d.md", &text).unwrap();
            assert!(deck.cards.iter().all(|c| c.token.is_some()));
            assert!(text.contains("First question"));
            assert_no_duplicate_tokens("d.md", &text);
        }
        {
            let dir = tempfile::tempdir().unwrap();
            let placed = place_deck(dir.path(), "d", "## base\nb\n").unwrap();
            let added = "aaaaaaaaaaaaaaaaaaaaaaaaap";
            crate::deck::append_cards(
                &placed.path,
                &format!("## added <!-- id: {added} -->\nans\n"),
            )
            .unwrap();
            let text = std::fs::read_to_string(&placed.path).unwrap();
            assert!(text.contains(added), "appended token preserved");
            assert_no_duplicate_tokens("d.md", &text);
        }
        {
            let dir = tempfile::tempdir().unwrap();
            let placed = place_deck(dir.path(), "d", "## base\nb\n").unwrap();
            let mut store = Store::open(dir.path().join("p.json")).unwrap();
            let vid = "pvzzzzzzzzzzzzzzzzzzzzzzzzz";
            store.insert_virtual(crate::store::VirtualCard {
                id: vid.into(),
                kind: crate::store::VirtualKind::Tutor,
                parent: "d.md".into(),
                text: format!("## promoted <!-- id: {vid} -->\npans\n"),
                created_ms: 0,
            });
            store.get_or_insert(vid, 0);
            store.save().unwrap();
            crate::store::promote_virtual(&mut store, vid, &placed.path).unwrap();
            let text = std::fs::read_to_string(&placed.path).unwrap();
            assert!(text.contains(vid), "promoted token preserved");
            assert_no_duplicate_tokens("d.md", &text);
        }
    }

    fn once(s: &str) -> HashSet<String> {
        std::iter::once(s.to_string()).collect()
    }

    #[test]
    fn a_trace_rebuild_routes_through_replace_and_wipes_the_old_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let existing = "---\nid: \"da1\"\ntrace: how x becomes y\nsource: notes.md\n---\n## old cp <!-- id: c1 -->\nold\n";
        std::fs::write(dir.path().join("t.md"), existing).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("c1", 0);
        store.save().unwrap();

        let new_text = crate::deck::trace_checkpoint_text(
            &dir.path().join("t.md"),
            existing,
            "## new cp\nnew\n",
        )
        .unwrap();
        assert!(new_text.contains("trace: how x becomes y"));
        assert!(new_text.contains("source: notes.md"));

        replace_deck(dir.path(), "t", &new_text, &mut store).unwrap();

        assert!(store.get("c1").is_none());
        let now = std::fs::read_to_string(dir.path().join("t.md")).unwrap();
        assert!(now.contains("new cp"));
        let rebuilt = crate::parser::parse("t.md", &now).unwrap();
        assert_eq!(1, rebuilt.cards.len());
        assert!(
            rebuilt.cards[0].token.is_some(),
            "the rebuilt checkpoint is stamped"
        );
        assert_ne!(
            Some("c1"),
            rebuilt.cards[0].token.as_deref(),
            "old token must not survive as the rebuilt card id"
        );
    }

    #[test]
    fn resetting_a_deck_drops_its_virtual_cards_but_keeps_anothers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "## qa <!-- id: qa -->\nans-a\n").unwrap();
        let deck_a = Deck::load(dir.path().join("a.md")).unwrap();

        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let vc_a = virtual_card("a.md", "vc-a");
        let vc_other = virtual_card("other.md", "vc-other");
        let (id_a, id_other) = (vc_a.id.clone(), vc_other.id.clone());
        store.insert_virtual(vc_a);
        store.insert_virtual(vc_other);
        store.get_or_insert(&id_a, 0);
        store.get_or_insert(&id_other, 0);

        let n = reset_decks(&mut store, [&deck_a]).unwrap();
        assert_eq!(0, n, "no authored cards had progress");
        assert!(
            store.get_virtual(&id_a).is_none(),
            "a's virtual card dropped"
        );
        assert!(store.get(&id_a).is_none(), "a's virtual schedule dropped");
        assert!(
            store.get_virtual(&id_other).is_some(),
            "another deck's virtual card survives"
        );
    }
}
