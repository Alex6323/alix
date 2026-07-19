use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use alix::{
    assemble::{load_decks, open_store, store_path_for},
    card::Card,
    config::Config,
    deck::{Deck, DeckState},
    depth::Depth,
    scheduler::{Fsrs, Scheduler},
    store::Store,
    time::{humanize_ms, now_ms},
    workspace,
};
use anyhow::{Context, Result, bail};

use crate::{
    DeckArgs, ResetArgs,
    common::{confirm, expand_target},
};

fn state_label(fsrs_state: Option<u8>) -> &'static str {
    match fsrs_state {
        Some(1) => "learning",
        Some(2) => "review",
        Some(3) => "relearning",
        Some(_) => "new",
        None => "-",
    }
}

pub(crate) fn stats(args: DeckArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let now = now_ms();

    let target = expand_target(&args.target, &config)?;
    for path in &target.decks {
        let store = target.store_for_deck(path, args.store.as_deref())?;
        let deck = Deck::load(path)?;
        let review = config
            .review
            .for_workspace(path.parent().unwrap_or_else(|| Path::new("")));
        let scheduler = Fsrs::new(review.retention, review.acquire_cooldown_ms);

        let mut due_now = 0usize;
        let mut due_24h = 0usize;
        let mut due_now_reconstruct = 0usize;
        let mut reviews = 0u32;
        let mut passes = 0u32;
        for card in &deck.cards {
            if let Some(state) = card.id().and_then(|id| store.get(&id)) {
                // Retired cards don't count as due, but still count toward
                // the review totals below.
                if !alix::session::is_retired(card, &store, review.retire_after_days) {
                    let due = scheduler.due_at(state, Depth::Recall);
                    if due <= now {
                        due_now += 1;
                    } else if due <= now + 86_400_000 {
                        due_24h += 1;
                    }
                    if scheduler.is_due(state, Depth::Reconstruct, now) {
                        due_now_reconstruct += 1;
                    }
                }
                reviews += state.total_reviews;
                passes += state.total_passes;
            }
        }
        // Virtual (remediation) cards count toward "due" (now and within
        // 24h), never toward the card count below: they aren't deck content.
        due_now += alix::session::count_reviewable_virtual(
            &store,
            &deck.subject,
            &scheduler,
            now,
            review.retire_after_days,
        );
        // Derived, not independently counted, so the two figures can't diverge.
        let due_now_recall = due_now;
        due_24h += alix::session::count_due_soon_virtual(
            &store,
            &deck.subject,
            &scheduler,
            now,
            86_400_000,
            review.retire_after_days,
        );

        let state = match deck.state(&store) {
            DeckState::NotStarted => "not started",
            DeckState::Started => "in progress",
            DeckState::ExamDue => "exam due",
            DeckState::Finished if store.deck_mastered(&deck.subject) => "mastered ✓",
            DeckState::Finished => "finished ✓",
        };
        println!("{} ({} cards)", deck.display_name(), deck.cards.len());
        println!("  state:   {state}");
        println!("  due:     {due_now} now, {due_24h} within 24h");
        println!("  due now (recall):      {due_now_recall}");
        println!("  due now (reconstruct): {due_now_reconstruct}");
        if reviews > 0 {
            println!(
                "  reviews: {reviews} total, {:.0}% passed",
                100.0 * passes as f64 / reviews as f64
            );
        }
    }
    Ok(())
}

pub(crate) fn list(args: DeckArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let now = now_ms();

    let target = expand_target(&args.target, &config)?;
    for path in &target.decks {
        let store = target.store_for_deck(path, args.store.as_deref())?;
        let deck = Deck::load(path)?;
        let review = config
            .review
            .for_workspace(path.parent().unwrap_or_else(|| Path::new("")));
        let scheduler = Fsrs::new(review.retention, review.acquire_cooldown_ms);
        println!("{}", deck.display_name());
        for card in &deck.cards {
            let (recall_label, recon_label, recognized_mark, due) =
                match card.id().and_then(|id| store.get(&id)) {
                    Some(state) => {
                        // A retired card's due time is moot until `alix reset`.
                        let due =
                            if alix::session::is_retired(card, &store, review.retire_after_days) {
                                "resting".to_string()
                            } else {
                                let due = scheduler.due_at(state, Depth::Recall);
                                if due <= now {
                                    "due now".to_string()
                                } else {
                                    format!("due in {}", humanize_ms(due - now))
                                }
                            };
                        let recall_label = state_label(state.recall.as_ref().map(|f| f.state));
                        let recon_label = state_label(state.reconstruct.as_ref().map(|f| f.state));
                        let recognized_mark = if state.recognized_ms.is_some() {
                            "✓"
                        } else {
                            " "
                        };
                        (recall_label, recon_label, recognized_mark, due)
                    }
                    None => (state_label(None), state_label(None), " ", "-".to_string()),
                };
            let front: String = card.front.chars().take(60).collect();
            println!("  [{recall_label:>10}|{recon_label:>10}]{recognized_mark} {front:<60} {due}");
        }
    }
    Ok(())
}

fn select_reset_ids(cards: &[Card], card: Option<&str>) -> Vec<(String, String)> {
    let stamped: Vec<(String, &Card)> = cards.iter().filter_map(|c| Some((c.id()?, c))).collect();
    // Exact id match only fires when some card actually carries that id, so
    // an ordinary front-substring is never mistaken for an id.
    let exact = card.filter(|q| stamped.iter().any(|(id, _)| id == q));
    let needle = card.filter(|_| exact.is_none()).map(str::to_lowercase);
    stamped
        .into_iter()
        .filter(|(id, c)| match (exact, &needle) {
            (Some(want), _) => id.as_str() == want,
            (None, Some(text)) => c.front.to_lowercase().contains(text),
            (None, None) => true,
        })
        .map(|(id, c)| (id, c.front.clone()))
        .collect()
}

pub(crate) fn reset(args: ResetArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;

    // `--orphans` is explicit opt-in only, never folded into a plain reset.
    if args.orphans {
        return reset_orphans(&args, &config);
    }

    let mut store = open_store(
        args.store
            .clone()
            .or_else(|| config.decks_dir().map(|d| workspace::root_store_path(&d))),
    )?;

    // `store.len()` counts virtual schedules too, so a store holding only
    // virtual cards still reports something to reset.
    if args.all {
        let n = store.len();
        if n == 0 {
            println!("No stored progress to reset.");
            return Ok(());
        }
        if !confirm(&format!("Reset progress for all {n} card(s)?"), args.yes)? {
            println!("Cancelled.");
            return Ok(());
        }
        store.clear();
        store.save()?;
        println!("Reset {n} card(s).");
        return Ok(());
    }

    let exact_id = args.card.as_deref().filter(|c| {
        alix::token::parse_card_id(c).is_some_and(|(token, _, _)| alix::token::is_valid(token))
    });
    if let Some(id) = exact_id.filter(|_| args.target.is_none()) {
        let id = id.to_string();
        return reset_ids(
            &mut store,
            vec![(id.clone(), String::new())],
            format!("card {id}"),
            args.card.as_deref(),
            false,
            args.yes,
        );
    }

    let Some(target_path) = &args.target else {
        bail!("name a deck, folder, or workspace to reset, or pass `--card <id>` or `--all`");
    };
    let target = expand_target(target_path, &config)?;
    let deck_paths = target.decks.clone();

    // Mirrors the launcher's store precedence, so reset hits the same
    // progress that serving uses.
    let mut store = open_store(
        args.store
            .clone()
            .or_else(|| store_path_for(&deck_paths, None))
            .or_else(|| target.default_store.clone()),
    )?;

    let (cards, label, _, _) = load_decks(&deck_paths, &HashMap::new())?;

    // A full-deck reset (no `--card` subset) resets authored-card progress,
    // the "mastered" exam flag, and virtual cards together atomically: a
    // declined/failed prompt must leave the store untouched by all of it.
    if args.card.is_none() {
        // Load the decks once, up front, for both the confirm-prompt count
        // and the wipe below: loading twice would let a deck edited on disk
        // in between silently diverge (a renamed back line changes
        // `Card::id()`, orphaning the old schedule).
        let decks_full: Vec<Deck> = deck_paths
            .iter()
            .map(Deck::load)
            .collect::<Result<Vec<_>, _>>()?;

        let present: Vec<(String, String)> = decks_full
            .iter()
            .flat_map(|deck| &deck.cards)
            .filter_map(|c| Some((c.id()?, c)))
            .filter(|(id, _)| store.get(id).is_some())
            .map(|(id, c)| (id, c.front.clone()))
            .collect();
        let mastered = decks_full
            .iter()
            .any(|deck| store.deck_mastered(&deck.subject));
        let virtual_ids: Vec<String> = decks_full
            .iter()
            .flat_map(|deck| store.virtual_cards_for(&deck.subject))
            .map(|vc| vc.id.clone())
            .collect();

        if present.is_empty() && !mastered && virtual_ids.is_empty() {
            println!("No stored progress to reset in {label}.");
            return Ok(());
        }

        let n = present.len();
        if !confirm(
            &format!("Reset progress for {n} card(s) in {label}?"),
            args.yes,
        )? {
            println!("Cancelled.");
            return Ok(());
        }

        let wiped = alix::library::reset_decks(&mut store, decks_full.iter())?;
        println!("Reset {wiped} card(s).");
        return Ok(());
    }

    let targets = select_reset_ids(&cards, args.card.as_deref());
    reset_ids(
        &mut store,
        targets,
        label,
        args.card.as_deref(),
        false,
        args.yes,
    )
}

fn reset_orphans(args: &ResetArgs, config: &Config) -> Result<()> {
    let (deck_paths, store_path) = match &args.target {
        Some(target) => {
            let target = expand_target(target, config)?;
            let store = args
                .store
                .clone()
                .or_else(|| store_path_for(&target.decks, None))
                .or_else(|| target.default_store.clone());
            (target.decks, store)
        }
        None => {
            let dir = config.decks_dir().context("cannot determine ~/decks")?;
            let store = args
                .store
                .clone()
                .unwrap_or_else(|| workspace::root_store_path(&dir));
            (workspace::deck_files(&dir), Some(store))
        }
    };

    // An unstamped card has no id and no store entry, so it's neither known
    // nor an orphan.
    let mut known_cards: HashSet<String> = HashSet::new();
    let mut known_subjects: HashSet<String> = HashSet::new();
    for path in &deck_paths {
        if let Ok(deck) = Deck::load(path) {
            known_subjects.insert(deck.subject.clone());
            known_cards.extend(deck.cards.iter().filter_map(Card::id));
        }
    }

    let mut store = open_store(store_path)?;
    let orphans = store.orphans(&known_cards, &known_subjects);
    if orphans.is_empty() {
        println!("No orphaned progress to reset.");
        return Ok(());
    }
    let n = orphans.len();
    if !confirm(
        &format!("Reset {n} orphaned key(s) (matching no known card or deck)?"),
        args.yes,
    )? {
        println!("Cancelled.");
        return Ok(());
    }
    let removed = store.prune_orphans(&orphans);
    store.save()?;
    println!("Reset {removed} orphaned key(s).");
    Ok(())
}

fn reset_ids(
    store: &mut Store,
    targets: Vec<(String, String)>,
    scope: String,
    card_query: Option<&str>,
    from_picker: bool,
    yes: bool,
) -> Result<()> {
    let present: Vec<(String, String)> = targets
        .into_iter()
        .filter(|(id, _)| store.get(id).is_some())
        .collect();
    if present.is_empty() {
        match card_query {
            Some(query) => println!("No stored progress matching {query:?}."),
            None => println!("No stored progress to reset in {scope}."),
        }
        return Ok(());
    }

    let n = present.len();
    if !from_picker {
        let what = if card_query.is_some() {
            let fronts: Vec<String> = present
                .iter()
                .map(|(_, f)| f.chars().take(60).collect())
                .filter(|f: &String| !f.is_empty())
                .collect();
            if fronts.is_empty() {
                scope
            } else {
                fronts.join("; ")
            }
        } else {
            format!("{n} card(s) in {scope}")
        };
        if !confirm(&format!("Reset progress for {what}?"), yes)? {
            println!("Cancelled.");
            return Ok(());
        }
    }

    for (id, _) in &present {
        store.remove(id);
    }
    store.save()?;
    println!("Reset {n} card(s).");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn card(front: &str, back: &str) -> Card {
        let mut c = Card::plain(Arc::from("d.md"), front.into(), vec![back.into()], None, 1);
        let slug: String = back
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        c.token = Some(Arc::from(format!("q{slug}").as_str()));
        c
    }

    #[test]
    fn reset_selects_all_without_a_filter() {
        let cards = vec![card("A", "1"), card("B", "2")];
        assert_eq!(2, select_reset_ids(&cards, None).len());
    }

    #[test]
    fn reset_matches_front_substring_case_insensitively() {
        let cards = vec![
            card("Capital of Japan?", "Tokyo"),
            card("Largest planet?", "Jupiter"),
        ];
        let got = select_reset_ids(&cards, Some("japan"));
        assert_eq!(1, got.len());
        assert_eq!("Capital of Japan?", got[0].1);
    }

    #[test]
    fn reset_matches_a_card_id_exactly() {
        let cards = vec![card("A", "1"), card("B", "2")];
        let id = cards[1].id().unwrap();
        assert_eq!(
            vec![(id.clone(), "B".to_string())],
            select_reset_ids(&cards, Some(&id))
        );
    }

    #[test]
    fn reset_front_match_resets_all_cards_sharing_it() {
        let cards = vec![
            card("verb forms", "a"),
            card("verb forms", "b"),
            card("noun", "c"),
        ];
        let got = select_reset_ids(&cards, Some("verb forms"));
        assert_eq!(2, got.len());
        assert_ne!(got[0].0, got[1].0);
    }
}
