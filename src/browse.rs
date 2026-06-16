//! A read-only deck browser.
//!
//! Pages through every card of the loaded decks, front and back shown
//! together, in file order. Unlike a review it does not grade, schedule, or
//! touch the progress store — it is purely for a first read-through or for
//! checking a deck's contents. Navigation only: next/previous/first/last.

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
    tui::{bar, key_pattern, push_note},
};

/// Browses `cards` (already in deck order) until the user quits. `label` is
/// the deck name(s) shown in the header; `keys` are the browse bindings.
pub fn run(cards: Vec<Card>, label: String, keys: BrowseBindings) -> Result<()> {
    if cards.is_empty() {
        println!("No cards to browse.");
        return Ok(());
    }
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &cards, &label, &keys);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    cards: &[Card],
    label: &str,
    keys: &BrowseBindings,
) -> Result<()> {
    let mut current = 0usize;
    let last = cards.len() - 1;
    loop {
        terminal.draw(|frame| draw(frame, cards, label, current, keys))?;
        // Blocking read is fine: nothing happens without a keypress.
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let pattern = key_pattern(&key);
        let hit = |list: &[crate::config::KeyPattern]| pattern.is_some_and(|p| list.contains(&p));
        if hit(&keys.quit) {
            break;
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

fn draw(frame: &mut Frame, cards: &[Card], label: &str, current: usize, keys: &BrowseBindings) {
    let [header, _, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let card = &cards[current];

    let left = format!(" flash {} │ {} (browse)", env!("CARGO_PKG_VERSION"), label);
    let right = format!("card {} / {} ", current + 1, cards.len());
    frame.render_widget(bar(&left, &right, header.width), header);

    let footer_text = format!(
        " {} next │ {} prev │ g/G first/last │ {} quit",
        Bindings::label(&keys.next),
        Bindings::label(&keys.prev),
        Bindings::label(&keys.quit),
    );
    frame.render_widget(bar(&footer_text, "", footer.width), footer);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(card.front.clone().bold()));
    // Cloze cards carry their masked answer text as context below the front.
    for ctx in &card.context {
        lines.push(Line::from(Span::styled(
            format!("  {ctx}"),
            Style::new().fg(Color::Cyan),
        )));
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
