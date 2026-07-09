//! `alix stats` / `list` / `reset`: per-deck progress reporting and clearing
//! stored review state. Each command resolves its target (deck, folder, or
//! workspace) to member decks and the store they actually use.

use std::{collections::HashMap, path::Path};

use alix::{
    card::Card,
    config::Config,
    deck::{Deck, DeckState},
    depth::Depth,
    scheduler::{Fsrs, Scheduler},
    store::Store,
    time::{humanize_ms, now_ms},
};
use anyhow::{Result, bail};

use crate::{
    DeckArgs, ResetArgs,
    common::{confirm, expand_target, load_decks, open_store, store_path_for},
};

/// The `list` label for one depth's schedule: the FSRS state name when the
/// card has a schedule at that depth, `-` when it has none.
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
    let config = Config::load(None)?;
    let now = now_ms();

    let target = expand_target(&args.target)?;
    for path in &target.decks {
        // Each deck reads its own store — a workspace deck's progress lives in the
        // workspace, not the global store.
        let store = target.store_for_deck(path, args.store.as_deref())?;
        let deck = Deck::load(path)?;
        // …and its own pacing: a workspace deck honors its `alix.local.toml`.
        let review = config
            .review
            .for_workspace(path.parent().unwrap_or_else(|| Path::new("")));
        let scheduler = Fsrs::new(review.retention);

        let mut due_now = 0usize;
        let mut due_24h = 0usize;
        let mut due_now_reconstruct = 0usize;
        let mut reviews = 0u32;
        let mut passes = 0u32;
        for card in &deck.cards {
            if let Some(state) = store.get(card.id()) {
                // Retired cards are resting, so they don't count as due (they
                // still count toward the review totals below).
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
        // 24h), never toward the card count below — they aren't deck content.
        due_now += alix::session::count_reviewable_virtual(
            &store,
            &deck.subject,
            &scheduler,
            now,
            review.retire_after_days,
        );
        // Virtual cards are Recall-only, so the recall figure IS the due-now
        // aggregate — derived, not re-counted, so the two lines can't diverge.
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
    let config = Config::load(None)?;
    let now = now_ms();

    let target = expand_target(&args.target)?;
    for path in &target.decks {
        // Each deck reads its own store (workspace store for a workspace deck).
        let store = target.store_for_deck(path, args.store.as_deref())?;
        let deck = Deck::load(path)?;
        // …and its own pacing (workspace `alix.local.toml` override).
        let review = config
            .review
            .for_workspace(path.parent().unwrap_or_else(|| Path::new("")));
        let scheduler = Fsrs::new(review.retention);
        println!("{}", deck.display_name());
        for card in &deck.cards {
            let (recall_label, recon_label, recognized_mark, due) = match store.get(card.id()) {
                Some(state) => {
                    // Retired cards rest until `alix reset`; their due time is
                    // moot, so say so instead of showing a misleading interval.
                    let due = if alix::session::is_retired(card, &store, review.retire_after_days) {
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

/// `(id, front)` pairs to reset from `cards`: all of them when `card` is
/// `None`, otherwise only the matches. A numeric `card` matches by
/// `Card::id()`; any other text matches cards whose front contains it
/// (case-insensitive) — a cloze card's holes share a front, so that resets the
/// whole card.
fn select_reset_ids(cards: &[Card], card: Option<&str>) -> Vec<(u64, String)> {
    let by_id = card.and_then(|c| c.parse::<u64>().ok());
    let needle = card.map(str::to_lowercase);
    cards
        .iter()
        .filter(|c| match (by_id, &needle) {
            (Some(id), _) => c.id() == id,
            (None, Some(text)) => c.front.to_lowercase().contains(text),
            (None, None) => true,
        })
        .map(|c| (c.id(), c.front.clone()))
        .collect()
}

pub(crate) fn reset(args: ResetArgs) -> Result<()> {
    // `--all` / a numeric `--card` operate on the global store (or `--store`);
    // a deck-scoped reset re-resolves to the deck's workspace store below.
    let mut store = open_store(args.store.clone())?;

    // `--all`: wipe everything; no decks needed, count up front for the prompt.
    // `store.len()` now counts virtual schedules too (they live in `store.cards`),
    // so a store holding only virtual cards still reports something to reset.
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

    // A numeric `--card` with no target can be removed without loading anything.
    let numeric_id = args.card.as_deref().and_then(|c| c.parse::<u64>().ok());
    if let Some(id) = numeric_id.filter(|_| args.target.is_none()) {
        return reset_ids(
            &mut store,
            vec![(id, String::new())],
            format!("card {id}"),
            args.card.as_deref(),
            false,
            args.yes,
        );
    }

    // Otherwise a reset needs an explicit target — there is no interactive
    // deck picker. Name a deck/folder/workspace (optionally with `--card`),
    // or pass `--all`.
    let Some(target_path) = &args.target else {
        bail!("name a deck, folder, or workspace to reset, or pass `--card <id>` or `--all`");
    };
    let target = expand_target(target_path)?;
    let deck_paths = target.decks.clone();

    // Reset against the target's store: `--store` > the members' shared
    // workspace store > a scoped folder's own store > the global default —
    // the launcher's rule, so the reset hits the progress that serving uses.
    let mut store = open_store(
        args.store
            .clone()
            .or_else(|| store_path_for(&deck_paths, None))
            .or_else(|| target.default_store.clone()),
    )?;

    let (cards, label, _, _) = load_decks(&deck_paths, &HashMap::new())?;

    // A full-deck reset (no `--card` subset) resets authored-card progress, the
    // decks' "mastered" exam flag, and their virtual (remediation) cards
    // together, atomically under one confirmation — a declined/failed prompt
    // must leave the store on disk untouched by any of it (not just the
    // authored-card part).
    if args.card.is_none() {
        // Load the decks once, up front, and use that same load for both the
        // confirm-prompt count and the wipe below — counting from one load and
        // wiping from a later, separate one let a deck edited on disk while the
        // prompt waits silently diverge (a renamed back line changes
        // `Card::id()`, orphaning the old schedule).
        let decks_full: Vec<Deck> = deck_paths
            .iter()
            .map(Deck::load)
            .collect::<Result<Vec<_>, _>>()?;

        let present: Vec<(u64, String)> = decks_full
            .iter()
            .flat_map(|deck| &deck.cards)
            .filter(|c| store.get(c.id()).is_some())
            .map(|c| (c.id(), c.front.clone()))
            .collect();
        let mastered = decks_full
            .iter()
            .any(|deck| store.deck_mastered(&deck.subject));
        // A virtual card's content is in the sidecar and its schedule in
        // `store.cards` (both keyed by the same id) — a reset drops both.
        let virtual_ids: Vec<u64> = decks_full
            .iter()
            .flat_map(|deck| store.virtual_cards_for(&deck.subject))
            .map(|vc| vc.id)
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

    // A `--card` subset over the named decks: match by numeric id or front text
    // (a full-deck reset is handled above).
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

/// Removes the `(id, front)` targets that have stored progress, after a `y/N`
/// confirmation — unless `from_picker` (the picker's Enter already confirmed)
/// or `yes`. Saves and reports the count.
fn reset_ids(
    store: &mut Store,
    targets: Vec<(u64, String)>,
    scope: String,
    card_query: Option<&str>,
    from_picker: bool,
    yes: bool,
) -> Result<()> {
    let present: Vec<(u64, String)> = targets
        .into_iter()
        .filter(|(id, _)| store.get(*id).is_some())
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
        store.remove(*id);
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
        Card::plain(Arc::from("d.txt"), front.into(), vec![back.into()], None, 1)
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
    fn reset_matches_a_numeric_id_exactly() {
        let cards = vec![card("A", "1"), card("B", "2")];
        let id = cards[1].id();
        assert_eq!(
            vec![(id, "B".to_string())],
            select_reset_ids(&cards, Some(&id.to_string()))
        );
    }

    #[test]
    fn reset_front_match_resets_all_cards_sharing_it() {
        // Cloze holes share a front but have distinct ids; one match clears all.
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
