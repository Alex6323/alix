use std::{path::Path, sync::Arc};

use alix::{
    assemble::{self, VIRTUAL_LINE_BASE, synthesize_virtual},
    augment::{self, AugmentCache},
    augment_ai,
    card::Card,
    config::{self, Config},
    generate, import, library, parser, workspace,
};
use anyhow::{Context, Result, bail};
use chrono::NaiveDate;

use crate::{
    AugmentArgs, AugmentTarget, ImportArgs, WorkspaceDeadlineArgs, WorkspaceInitArgs,
    common::{deck_out_dir, one_line, store_for, truncate},
};

/// Foreground: any Claude error surfaces here, not mid-review.
pub(crate) fn augment_cmd(args: AugmentArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    // Must stamp before the cache is keyed by `Card::id`: unstamped cards all
    // hash to id 0, collapsing the cache and orphaning the spend.
    let deck = assemble::stamp_and_load_deck(&args.deck)?;
    let ask_cfg = augment_ai::run_config(&config.ai, &config.ask);
    let guidance = args.with.as_deref();

    let store = store_for(
        std::slice::from_ref(&args.deck),
        args.store.clone(),
        &config,
    )?;
    let cache_path = augment::augment_path_for(store.path());
    let mut cache = AugmentCache::open(&cache_path);

    let what = match args.target {
        AugmentTarget::Choices => "multiple-choice distractors",
        AugmentTarget::Notes => "trivia / mnemonic notes",
        AugmentTarget::Questions => "reworded question variants",
        AugmentTarget::Keypoints => "answer key points",
        AugmentTarget::Topology => "a review order",
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
                cache.set_distractors(id, distractors.clone());
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
                cache.set_note(id, note.clone());
            }
            (map.len(), total, "notes")
        }
        AugmentTarget::Questions => {
            // Cloze cards are excluded: their front is the title, not a
            // question to reword.
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
                cache.set_variants(id, variants.clone());
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
                cache.set_keypoints(id, keypoints.clone());
            }
            (map.len(), total, "key points")
        }
        AugmentTarget::Topology => {
            let items = warm_items(&deck.cards);
            if items.is_empty() {
                bail!("the deck has no cards to build an order over");
            }
            let total = items.len();
            let deck_token = deck.deck_token.clone().unwrap_or_default();
            let topo =
                augment_ai::generate_topology(&items, guidance, &deck_token, &ask_cfg, None)?;
            print_topology(&topo, &deck.cards);
            let walked = topo.walk.len();
            cache.add_topology(topo);
            // Scoped to this deck: the cache may be shared with other decks.
            let deck_tokens: std::collections::HashSet<String> =
                deck.deck_token.iter().cloned().collect();
            let n = cache.topologies_for(&deck_tokens).len();
            println!(
                "({n} order{} stored for this deck)",
                if n == 1 { "" } else { "s" }
            );
            (walked, total, "a review order")
        }
        AugmentTarget::Format => {
            // Cloze, promoted, and retired cards are excluded (mirrors the
            // review's injection filters), so a card is never formatted twice
            // or after resting.
            let subject: Arc<str> = Arc::from(deck.subject.as_str());
            let deck_ids: std::collections::HashSet<String> =
                deck.cards.iter().filter_map(Card::id).collect();
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
                .filter(|v| !alix::session::is_retired_id(&v.id, &store, retire_after_days))
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
                cache.set_format(id, fmt.clone());
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

fn print_topology(topo: &augment::Topology, cards: &[Card]) {
    let fronts: std::collections::HashMap<String, String> = cards
        .iter()
        .filter_map(|c| Some((c.id()?, truncate(&one_line(&c.front), 72))))
        .collect();
    let unknown = "<card not in deck>".to_string();

    println!(
        "\norder '{}': {}\n({} cards walked, {} edges)\n",
        topo.name,
        topo.principle,
        topo.walk.len(),
        topo.edges.len()
    );
    let mut prev: Option<&str> = None;
    for (i, id) in topo.walk.iter().enumerate() {
        let front = fronts.get(id).unwrap_or(&unknown);
        match prev {
            None => println!("{:>3}. {front}", i + 1),
            Some(p) => {
                let why = topo
                    .edges
                    .iter()
                    .find(|e| e.from == p && e.to.as_str() == id.as_str())
                    .map(|e| e.label.as_str())
                    .unwrap_or("-");
                println!("{:>3}. ↳ [{why}]  {front}", i + 1);
            }
        }
        prev = Some(id.as_str());
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
    // Section headers stay uncommented: a key uncommented outside its table
    // would silently be ignored by the lenient parser.
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
    let name = if name.ends_with(".md") {
        name
    } else {
        format!("{name}.md")
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
        let mut store = store_for(std::slice::from_ref(&target), None, &config)?;
        let report = library::replace_deck(&dir, &name, &text, &mut store)?;
        println!(
            "Replaced {}: {} cards, wiped progress for {} card(s).",
            target.display(),
            report.minted,
            report.wiped_cards
        );
        return Ok(());
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
    // A deadline only has a product surface inside a real workspace, so a
    // plain folder errors here rather than silently accepting a setting it'd
    // ignore.
    if !workspace::is_workspace(dir) {
        bail!(
            "{} is not a workspace; make it one first: alix workspace init {}",
            dir.display(),
            dir.display()
        );
    }
    match args.date.as_deref() {
        None => {
            let review = Config::load(args.config.as_deref())?
                .review
                .for_workspace(dir);
            match review.deadline {
                Some(d) => {
                    let days = (d - alix::time::local_date(alix::time::now_ms())).num_days();
                    if days < 0 {
                        let past = -days;
                        let unit = if past == 1 { "day" } else { "days" };
                        println!("{d} (was due {past} {unit} ago)");
                    } else {
                        let unit = if days == 1 { "day" } else { "days" };
                        println!("{d} ({days} {unit} left)");
                    }
                }
                None => println!(
                    "no deadline set (set one: alix workspace deadline {} 2026-09-01)",
                    dir.display()
                ),
            }
        }
        Some("clear") => workspace::set_deadline(dir, None)?,
        Some(s) => {
            let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
                anyhow::anyhow!("invalid date {s:?}: expected YYYY-MM-DD (or \"clear\")")
            })?;
            workspace::set_deadline(dir, Some(date))?;
        }
    }
    Ok(())
}
