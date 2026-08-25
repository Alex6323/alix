use std::sync::Arc;

use alix::{
    assemble,
    augment::{self, AugmentCache},
    augment_ai,
    card::Card,
    config::Config,
    generate, import, library, parser, workspace,
};
use anyhow::{Context, Result, bail};
use chrono::NaiveDate;

use crate::{
    AugmentArgs, AugmentTarget, DeckInitArgs, DeckMoveArgs, DeckRemoveArgs, DeckRestoreArgs,
    DeckTransferArgs, ImportArgs, WorkspaceDeadlineArgs, WorkspaceInitArgs, WorkspaceUpdateArgs,
    common::{confirm, deck_out_dir, one_line, store_for, truncate},
};

pub(crate) fn remove_cmd(args: DeckRemoveArgs) -> Result<()> {
    let deck = &args.deck;
    if deck.is_dir() {
        bail!(
            "{} is a folder; deck remove takes a single deck file",
            deck.display()
        );
    }
    if !deck.is_file() {
        bail!("no deck at {}", deck.display());
    }
    let root = removal_store_root(deck, args.store.as_deref())?;
    let store = alix::state::open_store(deck, &root)?;
    let preview = library::removal_preview(deck, &store);

    println!("Removing {}:", deck.display());
    let since = preview
        .earliest_review_ms
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms as i64))
        .map(|t| format!(", reviewed since {}", t.format("%Y-%m-%d")))
        .unwrap_or_default();
    println!(
        "  {} card(s) with recorded progress{since}",
        preview.cards_with_progress
    );
    for file in &preview.files {
        println!("  {}", file.display());
    }
    for dir in &preview.directories {
        println!("  {}{}", dir.display(), std::path::MAIN_SEPARATOR);
    }
    if !preview.dependents.is_empty() {
        println!(
            "warning: required by {}; they will unlock, not break",
            preview.dependents.join(", ")
        );
    }
    if !confirm("Remove permanently? This cannot be undone.", args.yes)? {
        println!("Removal cancelled.");
        return Ok(());
    }
    let report = library::remove_deck(deck, &store)?;
    println!("Removed {} file(s); nothing was backed up.", {
        report.removed.len()
    });
    Ok(())
}

pub(crate) fn restore_cmd(args: DeckRestoreArgs) -> Result<()> {
    let deck = &args.deck;
    let root = removal_store_root(deck, args.store.as_deref())?;
    let report = library::restore_deck(deck, &root)?;
    let describe = |swapped: bool| {
        if swapped {
            "swapped"
        } else {
            "no backup found"
        }
    };
    println!(
        "Restored {} (review history: {}; augmentations: {}).",
        deck.display(),
        describe(report.progress),
        describe(report.augment)
    );
    println!("The previous state is the new backup; restore again to swap back.");
    Ok(())
}

fn removal_store_root(
    deck: &std::path::Path,
    cli_override: Option<&std::path::Path>,
) -> Result<std::path::PathBuf> {
    assemble::store_path_for(std::slice::from_ref(&deck.to_path_buf()), cli_override)
        .or_else(|| {
            Config::load(None)
                .ok()
                .and_then(|config| config.decks_dir())
                .map(|dir| workspace::root_store_path(&dir))
        })
        .or_else(alix::store::default_store_path)
        .context("cannot determine the progress store for this deck")
}

pub(crate) fn workspace_update_cmd(args: WorkspaceUpdateArgs) -> Result<()> {
    if args.discard {
        if alix::workspace_update::discard(&args.dir)? {
            println!(
                "Discarded {}.",
                alix::workspace_update::staging_path(
                    &args.dir.canonicalize().unwrap_or_else(|_| args.dir.clone())
                )
                .display()
            );
        } else {
            println!("No staged workspace update exists.");
        }
        return Ok(());
    }
    if args.apply {
        let report = alix::workspace_update::apply(&args.dir)?;
        print_workspace_update_report("Applied", &report);
        return Ok(());
    }

    let config = Config::load(args.config.as_deref())?;
    eprintln!(
        "Reconciling {} against its live sources (one AI call per source-backed deck)…",
        args.dir.display()
    );
    let report = alix::workspace_update::stage(&args.dir, &config.generate, &config.ask)?;
    print_workspace_update_report("Staged", &report);
    println!(
        "Review the exact proposal at {}. Apply it with `alix workspace update {} --apply`, or discard it with `--discard`.",
        report.staging.display(),
        args.dir.display()
    );
    Ok(())
}

fn print_workspace_update_report(action: &str, report: &alix::workspace_update::UpdateReport) {
    let retained = report.decks.iter().map(|deck| deck.retained).sum::<usize>();
    let retired = report.decks.iter().map(|deck| deck.retired).sum::<usize>();
    let added = report.decks.iter().map(|deck| deck.added).sum::<usize>();
    println!(
        "{action} {} deck(s): {retained} retained, {retired} retired, {added} new.",
        report.decks.len()
    );
    for deck in &report.decks {
        println!(
            "  {}: {} retained, {} retired, {} new",
            deck.path.display(),
            deck.retained,
            deck.retired,
            deck.added
        );
    }
    for path in &report.live_only {
        println!(
            "  {}: no source citations, nothing frozen to reconcile; left as is",
            path.display()
        );
    }
}

pub(crate) fn init_cmd(args: DeckInitArgs) -> Result<()> {
    let outcome = alix::assets::initialize(&args.deck)?;
    println!(
        "Initialized {} ({} card ids assigned)",
        args.deck.display(),
        outcome.stamp.minted_cards.len()
    );
    if let Some(freeze) = &outcome.freeze {
        if freeze.diagrams > 0 {
            println!("Frozen {} diagram(s)", freeze.diagrams);
        }
        for warning in &freeze.diagram_warnings {
            eprintln!("warning: {warning}");
        }
    }
    Ok(())
}

pub(crate) fn copy_cmd(args: DeckTransferArgs) -> Result<()> {
    let report = alix::deck_transfer::transfer(
        &args.deck,
        &args.workspace,
        alix::deck_transfer::TransferMode::Copy,
    )?;
    print_transfer_report("Copied", &report);
    Ok(())
}

pub(crate) fn move_cmd(args: DeckMoveArgs) -> Result<()> {
    if !confirm(
        &format!(
            "Move {} to {} and remove the source?",
            args.deck.display(),
            args.workspace.display()
        ),
        args.yes,
    )? {
        println!("Move cancelled.");
        return Ok(());
    }
    let report = alix::deck_transfer::transfer(
        &args.deck,
        &args.workspace,
        alix::deck_transfer::TransferMode::Move,
    )?;
    print_transfer_report("Moved", &report);
    Ok(())
}

fn print_transfer_report(action: &str, report: &alix::deck_transfer::TransferReport) {
    println!(
        "{action} {} to {}.",
        report.source.display(),
        report.destination.display()
    );
    println!(
        "  {} asset file(s), augmentation: {}, progress moved: {}",
        report.assets,
        if report.augmentation { "yes" } else { "no" },
        if report.progress { "yes" } else { "no" }
    );
    for path in &report.leftovers {
        eprintln!("warning: source cleanup left {}", path.display());
    }
}

/// Foreground: any backend error surfaces here, not mid-review.
pub(crate) fn augment_cmd(args: AugmentArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    // Must stamp before the cache is keyed by `Card::id`: unstamped cards all
    // hash to id 0, collapsing the cache and orphaning the spend.
    let deck = assemble::stamp_and_load_deck(&args.deck)?;
    let ask_cfg = augment_ai::run_config(&config.ai, &config.ask);
    let guidance = args.with.as_deref();

    let mut cache = AugmentCache::open_for_deck(&deck)?;
    let cache_path = cache.path().to_path_buf();
    let fp_by_id: std::collections::HashMap<String, u64> = deck
        .cards
        .iter()
        .filter_map(|card| card.id().map(|id| (id, card.content_fingerprint)))
        .collect();

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
    let backend = alix::ask::backend_label(ask_cfg.backend.name());
    eprintln!(
        "Generating {what} for \"{}\" with {backend} ({model}) — one batched call, \
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
                if let Some(&fingerprint) = fp_by_id.get(id) {
                    cache.set_distractors(id, distractors.clone(), fingerprint);
                }
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
                if let Some(&fingerprint) = fp_by_id.get(id) {
                    cache.set_note(id, note.clone(), fingerprint);
                }
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
                if let Some(&fingerprint) = fp_by_id.get(id) {
                    cache.set_variants(id, variants.clone(), fingerprint);
                }
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
                if let Some(&fingerprint) = fp_by_id.get(id) {
                    cache.set_keypoints(id, keypoints.clone(), fingerprint);
                }
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
            let store = store_for(
                std::slice::from_ref(&args.deck),
                args.store.clone(),
                &config,
            )?;
            // The filters below mirror the review's injection filters, so a
            // card is never formatted twice or after resting.
            let subject: Arc<str> = Arc::from(deck.subject.as_str());
            let deck_ids: std::collections::HashSet<String> =
                deck.cards.iter().filter_map(Card::id).collect();
            let retire_after_days = config
                .review
                .for_workspace(&alix::workspace::content_root(&deck.path))
                .retire_after_days;
            let mut plain: Vec<Card> = deck
                .cards
                .iter()
                .filter(|c| c.hash_lines.is_none())
                .cloned()
                .collect();
            let deck_id: Arc<str> = Arc::from(deck.deck_token.as_deref().unwrap_or_default());
            let mut personal = alix::personal::read(&deck.path, &deck.subject).cards;
            assemble::bind_personal(&mut personal, &subject, &deck_id);
            plain.extend(
                personal
                    .into_iter()
                    .filter(|c| c.hash_lines.is_none())
                    .filter(|c| c.id().is_some_and(|id| !deck_ids.contains(&id)))
                    .filter(|c| !alix::session::is_retired(c, &store, retire_after_days)),
            );
            let items = warm_items(&plain);
            if items.is_empty() {
                bail!("the deck has no plain (non-cloze) cards to format");
            }
            let total = items.len();
            let map = augment_ai::generate_format(&items, guidance, &ask_cfg, None)?;
            let format_fp_by_id: std::collections::HashMap<String, u64> = plain
                .iter()
                .filter_map(|card| card.id().map(|id| (id, card.format_fingerprint())))
                .collect();
            for (id, fmt) in &map {
                if let Some(&fingerprint) = format_fp_by_id.get(id) {
                    cache.set_format(id, fmt.clone(), fingerprint);
                }
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
    if workspace::has_manifest(&args.dir) {
        bail!("{} is already a workspace", args.dir.display());
    }
    std::fs::create_dir_all(args.dir.join(workspace::DECKS))
        .and_then(|()| std::fs::create_dir_all(args.dir.join("assets")))
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
         # reveal = \"flip\"              # flip | line\n\
         # order = \"scheduled\"          # scheduled | sequential\n"
    );
    let workspace_files = alix::workspace::WorkspaceFiles::new(&args.dir);
    std::fs::write(workspace_files.manifest(), manifest)
        .with_context(|| format!("cannot write {}/alix.toml", args.dir.display()))?;
    let local = "# Personal pacing for THIS workspace — never shared (`alix share` leaves it\n\
         # home). Uncomment a key to override your global [review] config here.\n\
         \n\
         [review]\n\
         \n\
         # retention = 0.9              # FSRS target recall probability (0.70–0.99)\n\
         # retire_after = \"1y\"          # a card rests at this interval (\"never\" disables)\n\
         # introduction_cooldown = \"5m\"      # settle gap before a new card's first quiz (\"90s\", \"0\" = none)\n\
         # max_session = 10             # cards a single sitting serves\n\
         # new_cards_percent = 30       # new-card share of max_session (the rest are due cards)\n\
         # deadline = \"2026-09-01\"     # make me ready by this date (picker readout + drilling ramp)\n\
         # deadline_ramp = \"14d\"       # how early the pre-deadline retention ramp starts (\"2w\"; \"0\" = cap only)\n";
    std::fs::write(
        alix::state::UserFiles::new(&args.dir).local_manifest(),
        local,
    )
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
            Ok(cards) => eprintln!("({} cards, not written; --print)", cards.len()),
            Err(e) => eprintln!("(warning: does not parse yet: {e})"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_store_root_uses_the_owning_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let decks = workspace.join(alix::workspace::DECKS);
        std::fs::create_dir_all(&decks).unwrap();
        std::fs::write(workspace.join(alix::workspace::MANIFEST), "").unwrap();
        let deck = decks.join("facts.md");
        std::fs::write(&deck, "## q\na\n").unwrap();

        assert_eq!(workspace, removal_store_root(&deck, None).unwrap());
    }

    #[test]
    fn topology_edge_output_child() {
        if std::env::var_os("ALIX_TOPOLOGY_EDGE_OUTPUT_CHILD").is_none() {
            return;
        }
        let topology = augment::Topology {
            name: "order".to_string(),
            principle: "test".to_string(),
            edges: vec![
                augment::TopologyEdge {
                    from: "a".to_string(),
                    to: "wrong".to_string(),
                    label: "wrong target".to_string(),
                },
                augment::TopologyEdge {
                    from: "wrong".to_string(),
                    to: "b".to_string(),
                    label: "wrong source".to_string(),
                },
                augment::TopologyEdge {
                    from: "a".to_string(),
                    to: "b".to_string(),
                    label: "correct edge".to_string(),
                },
            ],
            walk: vec!["a".to_string(), "b".to_string()],
            regions: Vec::new(),
            deck_token: "deck-owner".to_string(),
        };
        print_topology(&topology, &[]);
    }

    #[test]
    fn topology_output_labels_only_an_edge_matching_both_endpoints() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "deck::tests::topology_edge_output_child",
                "--nocapture",
            ])
            .env("ALIX_TOPOLOGY_EDGE_OUTPUT_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("[correct edge]"), "{stdout}");
        assert!(!stdout.contains("[wrong target]"), "{stdout}");
        assert!(!stdout.contains("[wrong source]"), "{stdout}");
    }
}
