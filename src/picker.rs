//! A small checkbox TUI for picking one or more items: decks to review (the
//! startup picker, used when `flash` is launched without deck arguments), a
//! deck's prerequisites (the `deps` editor), or cards to reset.
//!
//! Type to filter, Space to (de)select, Enter to confirm, Esc to cancel. The
//! widget is generic over the item key (`PathBuf` for decks, `u64` card id for
//! cards); the deck-specific candidate building lives below.

use std::{
    collections::HashSet,
    hash::Hash,
    path::{Path, PathBuf},
};

use anyhow::Result;
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    deck::{self, Deck, DeckState},
    recent::RecentDecks,
    store::{MAX_STAGE, Store},
};

const HEADER_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Cyan);

/// One selectable row: identified by `key`, matched/displayed by `label`, with
/// an optional dim `meta` suffix (a deck's last-used age, a card's stage/id).
struct Item<K> {
    key: K,
    label: String,
    meta: Option<String>,
    /// Deck rows: locked because a `% requires:` prerequisite isn't finished.
    /// Shown dimmed with a lock glyph, but still selectable (advisory).
    locked: bool,
}

// ---- deck candidates ----------------------------------------------------

/// A selectable deck, before it becomes a picker `Item`.
struct Candidate {
    path: PathBuf,
    name: String,
    /// When last reviewed, if it is a recent deck.
    last_used_ms: Option<u64>,
}

/// Every `*.txt` deck in `decks_dir`, sorted by name.
fn dir_candidates(decks_dir: &Path) -> Vec<Candidate> {
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(decks_dir) {
        Ok(read_dir) => read_dir
            .filter_map(|r| r.ok().map(|d| d.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "txt"))
            .collect(),
        Err(_) => Vec::new(),
    };
    paths.sort();
    paths
        .into_iter()
        .map(|path| Candidate {
            name: file_name(&path),
            path,
            last_used_ms: None,
        })
        .collect()
}

/// Builds the candidate list: existing recent decks first (recency order),
/// then every other `*.txt` in `decks_dir`, sorted by name.
fn build_candidates(decks_dir: &Path, recent: &RecentDecks) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for entry in recent.entries() {
        if entry.path.is_file() {
            out.push(Candidate {
                name: file_name(&entry.path),
                path: entry.path.clone(),
                last_used_ms: Some(entry.last_used_ms),
            });
            seen.insert(entry.path.clone());
        }
    }

    for candidate in dir_candidates(decks_dir) {
        if !seen.contains(&candidate.path) {
            out.push(candidate);
        }
    }
    out
}

/// Turns a deck candidate into a picker item, deriving its completion-state
/// meta (`new` / `m/total` at the top stage / `done ✓`) and lock status (a
/// `% requires:` prerequisite not yet finished) from the progress store. A deck
/// that fails to load shows a plain row.
fn deck_item(c: Candidate, store: &Store, decks_dir: &Path) -> Item<PathBuf> {
    let (meta, locked) = match Deck::load(&c.path) {
        Ok(deck) => {
            let total = deck.cards.len();
            let maxed = deck
                .cards
                .iter()
                .filter(|card| store.get(card.id()).is_some_and(|s| s.stage >= MAX_STAGE))
                .count();
            let label = match deck.state(store) {
                DeckState::Finished => "done ✓".to_string(),
                DeckState::NotStarted => "new".to_string(),
                DeckState::Started => format!("{maxed}/{total}"),
            };
            let locked = deck::is_locked(&deck, Some(decks_dir), store);
            (Some(format!("· {label}")), locked)
        }
        Err(_) => (None, false),
    };
    Item {
        key: c.path,
        label: c.name,
        meta,
        locked,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A deck name without its `.txt` extension, for matching.
fn stem(name: &str) -> String {
    name.strip_suffix(".txt").unwrap_or(name).to_string()
}

// ---- public entry points ------------------------------------------------

/// One deck offered by [`catalog`]: its file name, full path, and when it was
/// last reviewed (if it is a recent deck).
pub struct DeckEntry {
    pub name: String,
    pub path: PathBuf,
    pub last_used_ms: Option<u64>,
}

/// The deck catalog the pickers show, as plain data: recent decks first (recency
/// order), then every other `*.txt` in `decks_dir`. Frontend-agnostic, so the
/// web deck-selection screen can present the same list as the TUI picker.
pub fn catalog(decks_dir: &Path, recent: &RecentDecks) -> Vec<DeckEntry> {
    build_candidates(decks_dir, recent)
        .into_iter()
        .map(|c| DeckEntry {
            name: c.name,
            path: c.path,
            last_used_ms: c.last_used_ms,
        })
        .collect()
}

/// Runs the startup picker. Returns the chosen deck paths (empty if the user
/// cancelled or there is nothing to pick).
pub fn pick(decks_dir: &Path, recent: &RecentDecks, store: &Store) -> Result<Vec<PathBuf>> {
    let items = build_candidates(decks_dir, recent)
        .into_iter()
        .map(|c| deck_item(c, store, decks_dir))
        .collect();
    let picker = Picker::new(
        items,
        HashSet::new(),
        "select decks".to_string(),
        " SPACE select │ ENTER start │ ↑↓ move │ type to filter │ ESC cancel".to_string(),
        false,
        no_decks_message(decks_dir),
    );
    Ok(launch(picker)?.unwrap_or_default())
}

/// Runs the deck picker for `reset`: the same checkbox UI, but `exact` (an empty
/// tick set means "nothing", never the card under the cursor) and reset wording.
pub fn pick_to_reset(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
) -> Result<Vec<PathBuf>> {
    let items = build_candidates(decks_dir, recent)
        .into_iter()
        .map(|c| deck_item(c, store, decks_dir))
        .collect();
    let picker = Picker::new(
        items,
        HashSet::new(),
        "select decks to reset".to_string(),
        " SPACE select │ ENTER reset │ ↑↓ move │ type to filter │ ESC cancel".to_string(),
        true,
        no_decks_message(decks_dir),
    );
    Ok(launch(picker)?.unwrap_or_default())
}

/// Picks cards to act on from a pre-built `(id, label, meta)` list. Returns the
/// chosen ids (empty if cancelled or nothing ticked).
pub fn pick_cards(items: Vec<(u64, String, Option<String>)>, title: &str) -> Result<Vec<u64>> {
    let items = items
        .into_iter()
        .map(|(key, label, meta)| Item {
            key,
            label,
            meta,
            locked: false,
        })
        .collect();
    let picker = Picker::new(
        items,
        HashSet::new(),
        title.to_string(),
        " SPACE select │ ENTER reset │ ↑↓ move │ type to filter │ ESC cancel".to_string(),
        true,
        vec![Line::from("  No cards.")],
    );
    Ok(launch(picker)?.unwrap_or_default())
}

/// Runs the dependency editor for `target`: the same checkbox UI over the
/// decks in `decks_dir`, pre-ticked to the deck's current prerequisites
/// (`requires`, matched by name). Returns the chosen prerequisite paths
/// (possibly empty, meaning "no dependencies"), or `None` if cancelled.
/// `target` is excluded — a deck can't require itself.
pub fn edit_dependencies(
    decks_dir: &Path,
    target: &Path,
    requires: &[String],
) -> Result<Option<Vec<PathBuf>>> {
    let target_name = file_name(target);
    let mut candidates = dir_candidates(decks_dir);
    candidates.retain(|c| c.name != target_name);

    // Keep any current prerequisite that isn't a deck in the decks dir visible
    // and pre-checked, so saving doesn't silently drop it.
    let listed: HashSet<String> = candidates.iter().map(|c| stem(&c.name)).collect();
    for req in requires {
        if !listed.contains(&stem(req)) {
            candidates.push(Candidate {
                name: req.clone(),
                path: PathBuf::from(req),
                last_used_ms: None,
            });
        }
    }

    let want: HashSet<String> = requires.iter().map(|r| stem(r)).collect();
    let preselected: HashSet<PathBuf> = candidates
        .iter()
        .filter(|c| want.contains(&stem(&c.name)))
        .map(|c| c.path.clone())
        .collect();

    let items = candidates
        .into_iter()
        .map(|c| Item {
            key: c.path,
            label: c.name,
            meta: None,
            locked: false,
        })
        .collect();
    let picker = Picker::new(
        items,
        preselected,
        format!("prerequisites for {target_name}"),
        " SPACE toggle │ ENTER save │ ↑↓ move │ type to filter │ ESC cancel".to_string(),
        true,
        vec![
            Line::default(),
            Line::from(format!("  No other decks in {}.", decks_dir.display())),
        ],
    );
    launch(picker)
}

/// The empty-state shown when there are no decks to list.
fn no_decks_message(decks_dir: &Path) -> Vec<Line<'static>> {
    vec![
        Line::default(),
        Line::from(format!("  No decks found in {}.", decks_dir.display())),
        Line::default(),
        Line::from("  Put .txt decks there, or pass deck files on the command line.".dim()),
    ]
}

/// Sets up the terminal, runs the picker, and restores the terminal.
fn launch<K: Clone + Eq + Hash>(mut picker: Picker<K>) -> Result<Option<Vec<K>>> {
    let mut terminal = ratatui::init();
    let result = picker.run(&mut terminal);
    ratatui::restore();
    result
}

// ---- the widget ---------------------------------------------------------

struct Picker<K> {
    all: Vec<Item<K>>,
    filter: String,
    /// Indices into `all` matching the filter.
    filtered: Vec<usize>,
    /// Cursor position within `filtered`.
    cursor: usize,
    /// Scroll offset within `filtered`.
    offset: usize,
    selected: HashSet<K>,
    /// Header label (after "flash — ").
    title: String,
    /// Footer key hints.
    footer: String,
    /// When true, Enter returns exactly the ticked set (possibly empty); when
    /// false (startup picker), an empty tick set falls back to the item under
    /// the cursor.
    exact: bool,
    /// Lines shown when there are no items.
    empty: Vec<Line<'static>>,
    done: bool,
    cancelled: bool,
}

impl<K: Clone + Eq + Hash> Picker<K> {
    fn new(
        all: Vec<Item<K>>,
        selected: HashSet<K>,
        title: String,
        footer: String,
        exact: bool,
        empty: Vec<Line<'static>>,
    ) -> Self {
        let filtered = (0..all.len()).collect();
        Self {
            all,
            filter: String::new(),
            filtered,
            cursor: 0,
            offset: 0,
            selected,
            title,
            footer,
            exact,
            empty,
            done: false,
            cancelled: false,
        }
    }

    /// Returns `None` if cancelled, else the chosen keys.
    fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<Option<Vec<K>>> {
        while !self.done {
            terminal.draw(|frame| self.draw(frame))?;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Esc => self.cancel(),
                    KeyCode::Char('c') if ctrl => self.cancel(),
                    KeyCode::Enter => self.done = true,
                    KeyCode::Up => self.move_cursor(-1),
                    KeyCode::Down => self.move_cursor(1),
                    KeyCode::Char('p') if ctrl => self.move_cursor(-1),
                    KeyCode::Char('n') if ctrl => self.move_cursor(1),
                    KeyCode::Char(' ') => self.toggle(),
                    KeyCode::Backspace => {
                        self.filter.pop();
                        self.refilter();
                    }
                    KeyCode::Char(c) if !ctrl => {
                        self.filter.push(c);
                        self.refilter();
                    }
                    _ => {}
                }
            }
        }

        if self.cancelled {
            return Ok(None);
        }
        // Ticked items, in list order.
        let chosen: Vec<K> = self
            .all
            .iter()
            .filter(|item| self.selected.contains(&item.key))
            .map(|item| item.key.clone())
            .collect();
        if self.exact || !chosen.is_empty() {
            return Ok(Some(chosen));
        }
        // Startup picker with nothing ticked: use the item under the cursor.
        Ok(Some(
            self.filtered
                .get(self.cursor)
                .map(|&i| vec![self.all[i].key.clone()])
                .unwrap_or_default(),
        ))
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.done = true;
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let last = self.filtered.len() - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last as isize) as usize;
    }

    fn toggle(&mut self) {
        if let Some(&i) = self.filtered.get(self.cursor) {
            let key = &self.all[i].key;
            if !self.selected.remove(key) {
                self.selected.insert(key.clone());
            }
        }
    }

    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, item)| item.label.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.cursor = self.cursor.min(self.filtered.len().saturating_sub(1));
    }

    // ---- rendering -----------------------------------------------------

    fn draw(&mut self, frame: &mut Frame) {
        let [header, filter, list, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let left = format!(" flash — {} ({})", self.title, self.all.len());
        let right = format!("{} selected ", self.selected.len());
        frame.render_widget(bar(&left, &right, header.width), header);

        // Filter line with a cursor.
        frame.render_widget(Paragraph::new(format!(" filter: {}", self.filter)), filter);
        frame.set_cursor_position(Position::new(
            filter.x + 9 + self.filter.chars().count() as u16,
            filter.y,
        ));

        self.draw_list(frame, list);

        frame.render_widget(bar(&self.footer, "", footer.width), footer);
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect) {
        if self.all.is_empty() {
            frame.render_widget(Paragraph::new(self.empty.clone()), area);
            return;
        }

        let height = area.height as usize;
        // Keep the cursor within the visible window.
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }

        let mut lines = Vec::new();
        for (row, &i) in self
            .filtered
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(height)
        {
            let item = &self.all[i];
            let on_cursor = row == self.cursor;
            let checked = self.selected.contains(&item.key);

            let marker = if on_cursor { "›" } else { " " };
            let check = if checked { "[x]" } else { "[ ]" };
            let lock = if item.locked { "🔒 " } else { "" };
            let meta = item
                .meta
                .as_deref()
                .map(|m| format!("  {m}"))
                .unwrap_or_default();
            let text = format!("{marker} {check} {lock}{}{meta}", item.label);

            let mut style = Style::new();
            if on_cursor {
                style = style.fg(Color::Black).bg(Color::Cyan);
            } else if checked {
                style = style.fg(Color::Cyan);
            } else if item.locked {
                // Advisory: locked decks are dimmed but still selectable.
                style = style.fg(Color::DarkGray);
            }
            lines.push(Line::from(Span::styled(text, style)));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }
}

/// Renders a full-width colored bar with left/right text.
fn bar(left: &str, right: &str, width: u16) -> Paragraph<'static> {
    let pad = (width as usize)
        .saturating_sub(left.chars().count())
        .saturating_sub(right.chars().count());
    Paragraph::new(format!("{left}{}{right}", " ".repeat(pad))).style(HEADER_STYLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker_with(labels: &[&str]) -> Picker<PathBuf> {
        let all = labels
            .iter()
            .map(|n| Item {
                key: PathBuf::from(n),
                label: n.to_string(),
                meta: None,
                locked: false,
            })
            .collect();
        Picker::new(
            all,
            HashSet::new(),
            "t".to_string(),
            "f".to_string(),
            false,
            Vec::new(),
        )
    }

    #[test]
    fn filter_narrows_the_list() {
        let mut p = picker_with(&["rust.txt", "ruby.txt", "go.txt"]);
        assert_eq!(3, p.filtered.len());
        p.filter = "ru".to_string();
        p.refilter();
        assert_eq!(2, p.filtered.len());
        p.filter = "rust".to_string();
        p.refilter();
        assert_eq!(1, p.filtered.len());
    }

    #[test]
    fn filter_is_case_insensitive() {
        let mut p = picker_with(&["Rust.txt", "go.txt"]);
        p.filter = "RUST".to_string();
        p.refilter();
        assert_eq!(1, p.filtered.len());
    }

    #[test]
    fn cursor_clamps_after_filtering() {
        let mut p = picker_with(&["a.txt", "b.txt", "c.txt"]);
        p.cursor = 2;
        p.filter = "a".to_string();
        p.refilter();
        assert_eq!(0, p.cursor);
    }

    #[test]
    fn toggle_selects_under_cursor_returns_selection() {
        let mut p = picker_with(&["a.txt", "b.txt", "c.txt"]);
        p.toggle(); // a
        p.move_cursor(2);
        p.toggle(); // c
        let chosen: Vec<PathBuf> = p
            .all
            .iter()
            .filter(|item| p.selected.contains(&item.key))
            .map(|item| item.key.clone())
            .collect();
        assert_eq!(vec![PathBuf::from("a.txt"), PathBuf::from("c.txt")], chosen);
    }

    #[test]
    fn toggle_is_idempotent_pair() {
        let mut p = picker_with(&["a.txt"]);
        p.toggle();
        assert!(p.selected.contains(&PathBuf::from("a.txt")));
        p.toggle();
        assert!(p.selected.is_empty());
    }

    #[test]
    fn build_candidates_orders_recent_first_then_alpha() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["zeta.txt", "alpha.txt", "mid.txt"] {
            std::fs::write(dir.path().join(n), "# f\n\tb\n").unwrap();
        }
        let recent_path = dir.path().join("recent.json");
        let mut recent = RecentDecks::load(&recent_path);
        recent.record(&[dir.path().join("mid.txt")], 1000);

        let cands = build_candidates(dir.path(), &recent);
        let names: Vec<&str> = cands.iter().map(|c| c.name.as_str()).collect();
        // Recent (mid) first, then the rest alphabetically.
        assert_eq!(vec!["mid.txt", "alpha.txt", "zeta.txt"], names);
        assert!(cands[0].last_used_ms.is_some());
        assert!(cands[1].last_used_ms.is_none());
    }

    #[test]
    fn catalog_mirrors_candidate_order_and_paths() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["zeta.txt", "alpha.txt"] {
            std::fs::write(dir.path().join(n), "# f\n\tb\n").unwrap();
        }
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[dir.path().join("zeta.txt")], 1000);

        let entries = catalog(dir.path(), &recent);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(vec!["zeta.txt", "alpha.txt"], names); // recent first
        assert_eq!(dir.path().join("zeta.txt"), entries[0].path);
        assert!(entries[0].last_used_ms.is_some());
    }

    #[test]
    fn build_candidates_skips_missing_recent_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.txt"), "# f\n\tb\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[dir.path().join("deleted.txt")], 1000);

        let cands = build_candidates(dir.path(), &recent);
        let names: Vec<&str> = cands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(vec!["real.txt"], names);
    }
}
