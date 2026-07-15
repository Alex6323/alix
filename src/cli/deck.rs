//! `alix deck augment`/`import` and `alix workspace init`: deck and workspace
//! file curation. Augmenting calls Claude for a deliberate, cached addition
//! (distractors, notes, key points, topology, or format); import and
//! workspace-init write new files without touching a store.

use std::{path::Path, sync::Arc};

use alix::{
    assemble::{VIRTUAL_LINE_BASE, synthesize_virtual},
    augment::{self, AugmentCache},
    augment_ai,
    card::Card,
    config::{self, Config},
    deck::Deck,
    generate, import, library, parser, workspace,
};
use anyhow::{Context, Result, bail};
use chrono::NaiveDate;

use crate::{
    AugmentArgs, AugmentTarget, ImportArgs, WorkspaceInitArgs, WorkspaceDeadlineArgs,
    common::{deck_out_dir, one_line, store_for, truncate},
};

/// `alix deck augment`: deliberately generate AI augmentations for a deck into
/// the sidecar cache (`augment.json`), which review then reads. Foreground, so
/// any Claude error surfaces here rather than mid-review.
pub(crate) fn augment_cmd(args: AugmentArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let deck = Deck::load(&args.deck)?;
    let ask_cfg = augment_ai::run_config(&config.ai, &config.ask);
    let guidance = args.with.as_deref();

    // The cache sits beside whatever store the deck reviews against, so a
    // workspace deck's augmentations live with the workspace.
    let store = store_for(
        std::slice::from_ref(&args.deck),
        args.store.clone(),
        &config,
    )?;
    let cache_path = augment::augment_path_for(store.path());
    let mut cache = AugmentCache::open(&cache_path);

    // The Claude call below is one batched, foreground request that can take a
    // while, so say what's happening rather than hang silently.
    let what = match args.target {
        AugmentTarget::Choices => "multiple-choice distractors",
        AugmentTarget::Notes => "trivia / mnemonic notes",
        AugmentTarget::Questions => "reworded question variants",
        AugmentTarget::Keypoints => "answer key points",
        AugmentTarget::Topology => "a topology",
        AugmentTarget::Format => "card formatting",
    };
    let model = config
        .ai
        .model
        .as_deref()
        .or(config.ask.model.as_deref())
        .unwrap_or("the default model");
    eprintln!(
        "Generating {what} for \"{}\" with Claude ({model}) — one batched call, \
         this can take a moment…",
        deck.subject
    );

    let (made, total, kind) = match args.target {
        AugmentTarget::Choices => {
            let items = warm_items(&deck.cards);
            if items.is_empty() {
                bail!("the deck has no cards to augment");
            }
            let total = items.len();
            let map =
                augment_ai::generate(&items, config.ai.distractor_count, guidance, &ask_cfg, None)?;
            for (id, distractors) in &map {
                cache.set_distractors(*id, distractors.clone());
            }
            (map.len(), total, "distractors")
        }
        AugmentTarget::Notes => {
            let items = warm_items(&deck.cards);
            if items.is_empty() {
                bail!("the deck has no cards to augment");
            }
            let total = items.len();
            let map = augment_ai::generate_notes(&items, guidance, &ask_cfg, None)?;
            for (id, note) in &map {
                cache.set_note(*id, note.clone());
            }
            (map.len(), total, "notes")
        }
        AugmentTarget::Questions => {
            // Morphing the front only makes sense for plain cards — a cloze
            // card's front is its title, with the fill-in-the-blank in the body.
            let plain: Vec<Card> = deck
                .cards
                .iter()
                .filter(|c| c.hash_lines.is_none())
                .cloned()
                .collect();
            let items = warm_items(&plain);
            if items.is_empty() {
                bail!("the deck has no plain (non-cloze) cards to add question variants to");
            }
            let total = items.len();
            let map = augment_ai::generate_variants(
                &items,
                config.ai.variant_count,
                guidance,
                &ask_cfg,
                None,
            )?;
            for (id, variants) in &map {
                cache.set_variants(*id, variants.clone());
            }
            (map.len(), total, "question variants")
        }
        AugmentTarget::Keypoints => {
            let items = warm_items(&deck.cards);
            if items.is_empty() {
                bail!("the deck has no cards to break into key points");
            }
            let total = items.len();
            let map = augment_ai::generate_keypoints(
                &items,
                config.ai.keypoint_count,
                guidance,
                &ask_cfg,
                None,
            )?;
            for (id, keypoints) in &map {
                cache.set_keypoints(*id, keypoints.clone());
            }
            (map.len(), total, "key points")
        }
        AugmentTarget::Topology => {
            let items = warm_items(&deck.cards);
            if items.is_empty() {
                bail!("the deck has no cards to build a topology over");
            }
            let total = items.len();
            let topo = augment_ai::generate_topology(&items, guidance, &ask_cfg, None)?;
            print_topology(&topo, &deck.cards);
            let walked = topo.walk.len();
            cache.add_topology(topo);
            // Count only this deck's topologies — the cache may be shared with
            // other decks that share a store.
            let deck_ids: std::collections::HashSet<u64> =
                deck.cards.iter().map(|c| c.id()).collect();
            let n = cache.topologies_for(&deck_ids).len();
            println!(
                "({n} topolog{} stored for this deck)",
                if n == 1 { "y" } else { "ies" }
            );
            (walked, total, "a topology")
        }
        AugmentTarget::Format => {
            // Reshaping is for plain cards — a cloze card's masked body must not
            // be restructured. Include this deck's synthesized virtual
            // (remediation) cards alongside its authored ones: `set_format` keys
            // by the synth card's real `Card::id`, so the cached entry is exactly
            // what `apply_format` finds at review time (§8.2).
            //
            // Mirror `assemble::select`'s injection filters: a partial cloze promote
            // (see `store::promote_virtual`) can leave an orphaned sidecar entry
            // whose id collides with a real deck card, and a retired card is
            // resting — neither should be warmed a second time or at all.
            let subject: Arc<str> = Arc::from(deck.subject.as_str());
            let deck_ids: std::collections::HashSet<u64> =
                deck.cards.iter().map(Card::id).collect();
            let retire_after_days = config
                .review
                .for_workspace(deck.path.parent().unwrap_or_else(|| Path::new("")))
                .retire_after_days;
            let mut plain: Vec<Card> = deck
                .cards
                .iter()
                .filter(|c| c.hash_lines.is_none())
                .cloned()
                .collect();
            for (k, vc) in store
                .virtual_cards_for(&deck.subject)
                .into_iter()
                .filter(|v| !deck_ids.contains(&v.id))
                .filter(|v| !alix::session::is_retired_id(v.id, &store, retire_after_days))
                .enumerate()
            {
                if let Some(card) = synthesize_virtual(vc, &subject, VIRTUAL_LINE_BASE + k)
                    && card.hash_lines.is_none()
                {
                    plain.push(card);
                }
            }
            let items = warm_items(&plain);
            if items.is_empty() {
                bail!("the deck has no plain (non-cloze) cards to format");
            }
            let total = items.len();
            let map = augment_ai::generate_format(&items, guidance, &ask_cfg, None)?;
            for (id, fmt) in &map {
                cache.set_format(*id, fmt.clone());
            }
            (map.len(), total, "card formats")
        }
    };
    cache.save()?;

    println!(
        "augmented {made} of {total} cards with {kind} → {}",
        cache_path.display()
    );
    Ok(())
}

/// Builds the per-card generation input from `cards`.
fn warm_items(cards: &[Card]) -> Vec<augment::WarmItem> {
    cards.iter().map(augment::WarmItem::from_card).collect()
}

/// Prints a generated topology as its suggested walk — each card with the reason
/// it follows the previous one — so a person can judge whether the order reads as
/// "good follow-up" rather than random. The eyeball test for the topology probe.
fn print_topology(topo: &augment::Topology, cards: &[Card]) {
    let fronts: std::collections::HashMap<u64, String> = cards
        .iter()
        .map(|c| (c.id(), truncate(&one_line(&c.front), 72)))
        .collect();
    let unknown = "<card not in deck>".to_string();

    println!(
        "\ntopology '{}': {}\n({} cards walked, {} edges)\n",
        topo.name,
        topo.principle,
        topo.walk.len(),
        topo.edges.len()
    );
    let mut prev: Option<u64> = None;
    for (i, id) in topo.walk.iter().enumerate() {
        let front = fronts.get(id).unwrap_or(&unknown);
        match prev {
            None => println!("{:>3}. {front}", i + 1),
            Some(p) => {
                let why = topo
                    .edges
                    .iter()
                    .find(|e| e.from == p && e.to == *id)
                    .map(|e| e.label.as_str())
                    .unwrap_or("—");
                println!("{:>3}. ↳ [{why}]  {front}", i + 1);
            }
        }
        prev = Some(*id);
    }
    println!();
}

pub(crate) fn workspace_init_cmd(args: WorkspaceInitArgs) -> Result<()> {
    if workspace::is_workspace(&args.dir) {
        bail!("{} is already a workspace", args.dir.display());
    }
    std::fs::create_dir_all(args.dir.join("assets"))
        .with_context(|| format!("cannot create {}", args.dir.display()))?;
    let title = match &args.title {
        Some(t) => t.clone(),
        None => args
            .dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
            .to_string(),
    };
    // Both files are written fully commented (except what must be set), so
    // they document their own keys — the section headers stay UNcommented
    // because both parse leniently: a key uncommented outside its table would
    // be silently ignored.
    let manifest = format!(
        "# This workspace's shared manifest — it travels when the folder is shared.\n\
         \n\
         title = {title:?}\n\
         \n\
         # description = \"one line shown under the title in the picker\"\n\
         # icon = \"assets/icon.svg\"     # picker emblem (svg/png/jpg/webp); default: assets/icon.*\n\
         \n\
         # Deck directives every member deck inherits (a deck's own line wins):\n\
         \n\
         [defaults]\n\
         \n\
         # reveal = \"flip\"              # flip | cloze | line\n\
         # order = \"scheduled\"          # scheduled | sequential\n"
    );
    std::fs::write(args.dir.join("alix.toml"), manifest)
        .with_context(|| format!("cannot write {}/alix.toml", args.dir.display()))?;
    let local = "# Personal pacing for THIS workspace — never shared (`alix share` leaves it\n\
         # home). Uncomment a key to override your global [review] config here.\n\
         \n\
         [review]\n\
         \n\
         # retention = 0.9              # FSRS target recall probability (0.70–0.99)\n\
         # retire_after = \"1y\"          # a card rests at this interval (\"never\" disables)\n\
         # acquire_cooldown = \"5m\"      # settle gap before a new card's first quiz (\"90s\", \"0\" = none)\n\
         # max_new = 10                 # max never-seen cards a session introduces\n\
         # limit = 40                   # cap on total cards per session\n\
         # deadline = \"2026-09-01\"     # make me ready by this date (picker readout + drilling ramp)\n\
         # deadline_ramp = \"14d\"       # how early the pre-deadline retention ramp starts (\"2w\"; \"0\" = cap only)\n";
    std::fs::write(args.dir.join(config::LOCAL_MANIFEST), local)
        .with_context(|| format!("cannot write {}/alix.local.toml", args.dir.display()))?;
    println!(
        "Initialized {} — alix.toml (shared manifest) and alix.local.toml (your\n\
         personal pacing, never shared) document their keys inline. Add decks:\n\
         alix generate <source> --workspace {}   or   alix deck import <file.tsv> --workspace {}",
        args.dir.display(),
        args.dir.display(),
        args.dir.display(),
    );
    Ok(())
}

pub(crate) fn import_cmd(args: ImportArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let tsv = std::fs::read_to_string(&args.file)
        .with_context(|| format!("cannot read {}", args.file.display()))?;
    let text = import::tsv_to_deck(&tsv)?;

    // The file name is part of every card's identity hash, so parse against the
    // final name.
    let name = match &args.output {
        Some(name) => name.clone(),
        None => generate::deck_name(&args.file.to_string_lossy()),
    };
    let name = if name.ends_with(".txt") {
        name
    } else {
        format!("{name}.txt")
    };

    if args.print {
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
        match parser::parse_str(&name, &text) {
            Ok(cards) => eprintln!("({} cards — not written; --print)", cards.len()),
            Err(e) => eprintln!("(warning: does not parse yet — {e})"),
        }
        return Ok(());
    }

    let dir = deck_out_dir(args.workspace.as_deref(), &config)?;
    let target = dir.join(&name);
    if target.exists() {
        if !args.force {
            bail!(
                "{} already exists; pass --force to overwrite",
                target.display()
            );
        }
        // --force is CLI-only: clear the collision before placing.
        std::fs::remove_file(&target)
            .with_context(|| format!("cannot overwrite {}", target.display()))?;
    }
    let placed = library::place_deck(&dir, &name, &text)?;
    match placed.parse_error {
        None => {
            println!(
                "Imported {} cards into {}",
                placed.cards,
                placed.path.display()
            );
            Ok(())
        }
        // Saved, but not yet valid: tell the user exactly what to fix.
        Some(e) => bail!(
            "Saved the deck to {}, but it does not parse yet:\n  {e}\n\
             Fix that line and run `alix doctor {}`.",
            placed.path.display(),
            placed.path.display()
        ),
    }
}

pub(crate) fn workspace_deadline_cmd(args: WorkspaceDeadlineArgs) -> Result<()> {
    let dir = &args.dir;
    if !workspace::is_workspace(dir) && !workspace::has_decks(dir) {
        bail!("{} is not a workspace or decks folder", dir.display());
    }
    match args.date.as_deref() {
        None => {
            let review = Config::load(None)?.review.for_workspace(dir);
            match review.deadline {
                Some(d) => {
                    let days = (d - alix::time::local_date(alix::time::now_ms())).num_days();
                    if days < 0 {
                        println!("{d} (was due {} days ago)", -days);
                    } else {
                        println!("{d} ({days} days left)");
                    }
                }
                None => println!("no deadline set (set one: alix workspace deadline {} 2026-09-01)", dir.display()),
            }
        }
        Some("clear") => workspace::set_deadline(dir, None)?,
        Some(s) => {
            let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|_| anyhow::anyhow!("invalid date {s:?}: expected YYYY-MM-DD (or \"clear\")"))?;
            workspace::set_deadline(dir, Some(date))?;
        }
    }
    Ok(())
}
