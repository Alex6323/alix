//! A read-only deck browser.
//!
//! Pages through every card of the loaded decks, front and back shown
//! together, in file order. It does not grade or schedule; the only thing it
//! writes is card removal — pressing the `remove` key marks the current card,
//! and on quit those cards are deleted from their deck files and their progress
//! is pruned. Navigation: next/previous/first/last.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::PathBuf,
};

use anyhow::Result;
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    card::Card,
    config::{Bindings, BrowseBindings},
    deck,
    store::Store,
    tui::{bar, context_line, key_pattern, push_note},
};

/// Browses `cards` (already in deck order) until the user quits. `label` is the
/// deck name(s) shown in the header; `keys` are the browse bindings. `decks`
/// maps each subject to its file path, and `store` is the progress store —
/// both are only touched if the user removes a card.
pub fn run(
    cards: Vec<Card>,
    label: String,
    keys: BrowseBindings,
    decks: HashMap<String, PathBuf>,
    store: Store,
) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = run_on(&mut terminal, cards, label, keys, decks, store);
    ratatui::restore();
    result
}

/// Like [`run`] but on a caller-owned `terminal` (no init/restore), so the picker
/// can browse a deck and resume afterwards without a TUI teardown.
pub fn run_on(
    terminal: &mut ratatui::DefaultTerminal,
    cards: Vec<Card>,
    label: String,
    keys: BrowseBindings,
    decks: HashMap<String, PathBuf>,
    mut store: Store,
) -> Result<()> {
    if cards.is_empty() {
        println!("No cards to browse.");
        return Ok(());
    }
    let mut cards = cards;
    let mut removed_lines: HashMap<String, BTreeSet<usize>> = HashMap::new();
    let mut removed_ids: HashSet<u64> = HashSet::new();

    let result = event_loop(
        terminal,
        &mut cards,
        &label,
        &keys,
        &mut removed_lines,
        &mut removed_ids,
    );

    flush_removals(&removed_lines, &removed_ids, &decks, &mut store);
    result
}

/// Deletes the marked cards from their deck files and prunes their progress.
/// Best-effort, and a no-op if nothing was marked.
fn flush_removals(
    removed_lines: &HashMap<String, BTreeSet<usize>>,
    removed_ids: &HashSet<u64>,
    decks: &HashMap<String, PathBuf>,
    store: &mut Store,
) {
    if removed_lines.is_empty() {
        return;
    }
    let mut files = 0;
    let mut count = 0;
    for (subject, lines) in removed_lines {
        let Some(path) = decks.get(subject) else {
            eprintln!("warning: no deck file known for {subject}; cannot remove cards");
            continue;
        };
        let lines: Vec<usize> = lines.iter().copied().collect();
        count += lines.len();
        match deck::remove_cards(path, &lines) {
            Ok(()) => files += 1,
            Err(e) => eprintln!("warning: could not update {}: {e}", path.display()),
        }
    }
    for id in removed_ids {
        store.remove(*id);
    }
    if !removed_ids.is_empty() {
        let _ = store.save();
    }
    if files > 0 {
        eprintln!("Removed {count} card(s) from {files} deck file(s).");
    }
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    cards: &mut Vec<Card>,
    label: &str,
    keys: &BrowseBindings,
    removed_lines: &mut HashMap<String, BTreeSet<usize>>,
    removed_ids: &mut HashSet<u64>,
) -> Result<()> {
    let mut current = 0usize;
    loop {
        if cards.is_empty() {
            break; // everything was removed
        }
        let last = cards.len() - 1;
        current = current.min(last);
        terminal.draw(|frame| draw(frame, cards, label, current, keys))?;
        // Blocking read is fine: nothing happens without a keypress.
        let key = match event::read()? {
            // Resize with the event's dimensions so the redraw reflows at once.
            Event::Resize(w, h) => {
                terminal.resize(ratatui::layout::Rect::new(0, 0, w, h))?;
                continue;
            }
            Event::Key(key) => key,
            _ => continue,
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let pattern = key_pattern(&key);
        let hit = |list: &[crate::config::KeyPattern]| pattern.is_some_and(|p| list.contains(&p));
        if hit(&keys.quit) {
            break;
        } else if hit(&keys.remove) {
            mark_for_removal(cards, current, removed_lines, removed_ids);
        } else if hit(&keys.next) || matches!(key.code, KeyCode::Right | KeyCode::Down) {
            current = (current + 1).min(last);
        } else if hit(&keys.prev) || matches!(key.code, KeyCode::Left | KeyCode::Up) {
            current = current.saturating_sub(1);
        } else if matches!(key.code, KeyCode::Char('g') | KeyCode::Home) {
            current = 0;
        } else if matches!(key.code, KeyCode::Char('G') | KeyCode::End) {
            current = last;
        }
    }
    Ok(())
}

/// Marks the card at `current` (and any cloze siblings — same subject and line)
/// for removal and drops them from the in-memory list so they vanish at once.
fn mark_for_removal(
    cards: &mut Vec<Card>,
    current: usize,
    removed_lines: &mut HashMap<String, BTreeSet<usize>>,
    removed_ids: &mut HashSet<u64>,
) {
    let subject = cards[current].subject.to_string();
    let line = cards[current].line;
    removed_lines
        .entry(subject.clone())
        .or_default()
        .insert(line);
    cards.retain(|c| {
        let sibling = c.subject.as_ref() == subject && c.line == line;
        if sibling {
            removed_ids.insert(c.id());
        }
        !sibling
    });
}

fn draw(frame: &mut Frame, cards: &[Card], label: &str, current: usize, keys: &BrowseBindings) {
    let [header, _, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let card = &cards[current];

    let left = format!(" alix {} │ {} (browse)", env!("CARGO_PKG_VERSION"), label);
    let right = format!("card {} / {} ", current + 1, cards.len());
    frame.render_widget(bar(&left, &right, header.width), header);

    let footer_text = format!(
        " {} next │ {} prev │ g/G first/last │ {} remove │ {} quit",
        Bindings::label(&keys.next),
        Bindings::label(&keys.prev),
        Bindings::label(&keys.remove),
        Bindings::label(&keys.quit),
    );
    frame.render_widget(bar(&footer_text, "", footer.width), footer);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(card.front.clone().bold()));
    // Cloze cards carry their masked answer text as context below the front.
    for ctx in &card.context {
        lines.push(context_line(ctx));
    }
    lines.push(Line::default());
    for back in &card.back {
        lines.push(Line::from(Span::styled(
            format!("  {back}"),
            Style::new().fg(Color::Green),
        )));
    }
    push_note(&mut lines, card, body.width);

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
}
