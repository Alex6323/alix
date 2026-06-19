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
    session,
    store::Store,
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
    /// Deck rows: completion state, used to tint the meta (finished → green).
    /// `None` for non-deck pickers (cards, dependency editor).
    state: Option<DeckState>,
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
/// that fails to load shows a plain row. `enforce_locks` is false for the
/// browse picker — locking gates *review* progression only, so any deck is
/// browsable.
fn deck_item(c: Candidate, store: &Store, decks_dir: &Path, enforce_locks: bool) -> Item<PathBuf> {
    let (meta, locked, state) = match Deck::load(&c.path) {
        Ok(deck) => {
            let st = deck.state(store);
            let total = deck.cards.len();
            let retired = deck
                .cards
                .iter()
                .filter(|card| session::is_retired(card, store))
                .count();
            let label = match st {
                // "mastered" is reserved for passing the exam; a source-less
                // deck that's merely fully drilled stays "done".
                DeckState::Finished if store.deck_mastered(&deck.subject) => {
                    "mastered ✓".to_string()
                }
                DeckState::Finished => "done ✓".to_string(),
                DeckState::ExamDue => "exam due".to_string(),
                DeckState::NotStarted => "new".to_string(),
                DeckState::Started => format!("{retired}/{total}"),
            };
            let locked = enforce_locks && deck::is_locked(&deck, Some(decks_dir), store);
            (Some(format!("· {label}")), locked, Some(st))
        }
        Err(_) => (None, false, None),
    };
    Item {
        key: c.path,
        label: c.name,
        meta,
        locked,
        state,
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

/// The deck catalog the pickers show, as plain data: recent decks first
/// (recency order), then every other `*.txt` in `decks_dir`. Frontend-agnostic,
/// so the web deck-selection screen can present the same list as the TUI
/// picker.
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
/// Runs the startup picker. `enforce_locks` gates launching a deck whose
/// `% requires:` prerequisites aren't finished — true for `review`, false for
/// `browse` (any deck is browsable).
pub fn pick(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
    enforce_locks: bool,
) -> Result<Vec<PathBuf>> {
    let items = build_candidates(decks_dir, recent)
        .into_iter()
        .map(|c| deck_item(c, store, decks_dir, enforce_locks))
        .collect();
    let mut picker = Picker::new(
        items,
        HashSet::new(),
        "select decks".to_string(),
        // Footer is computed per launcher state in `draw`.
        String::new(),
        false,
        no_decks_message(decks_dir),
    );
    picker.launcher = true;
    Ok(launch(picker)?.unwrap_or_default())
}

/// Runs the deck picker for `reset`: the same checkbox UI, but `exact` (an
/// empty tick set means "nothing", never the card under the cursor) and reset
/// wording.
pub fn pick_to_reset(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
) -> Result<Vec<PathBuf>> {
    let items = build_candidates(decks_dir, recent)
        .into_iter()
        .map(|c| deck_item(c, store, decks_dir, false))
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
            state: None,
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
            state: None,
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
    /// Two-phase deck-launcher mode (the startup picker): Enter launches the
    /// focused deck; `Space` ticks decks; `Tab` confirms a multi-deck
    /// selection. Off for the reset / dependency pickers (plain tick +
    /// Enter).
    launcher: bool,
    /// Launcher review sub-state: the list shows only the ticked decks and
    /// Enter starts the merged session.
    confirming: bool,
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
            launcher: false,
            confirming: false,
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
                    // In the launcher's confirm view, Esc steps back to browse;
                    // everywhere else it cancels the picker.
                    KeyCode::Esc if self.confirming => {
                        self.confirming = false;
                        self.refilter();
                    }
                    KeyCode::Esc => self.cancel(),
                    KeyCode::Char('c') if ctrl => self.cancel(),
                    KeyCode::Enter => {
                        // Launcher browse: Enter launches the focused deck, but
                        // never a locked one. Otherwise (confirm view, or the
                        // reset/deps pickers) Enter accepts the selection.
                        if self.launcher && !self.confirming {
                            if self.focused().is_some_and(|item| !item.locked) {
                                self.done = true;
                            }
                        } else {
                            self.done = true;
                        }
                    }
                    // Tab confirms a multi-deck selection (a non-letter key, since
                    // letters are filter input here).
                    KeyCode::Tab if self.launcher && !self.confirming => {
                        if !self.selected.is_empty() {
                            self.enter_confirm();
                        }
                    }
                    KeyCode::Up => self.move_cursor(-1),
                    KeyCode::Down => self.move_cursor(1),
                    KeyCode::Char('p') if ctrl => self.move_cursor(-1),
                    KeyCode::Char('n') if ctrl => self.move_cursor(1),
                    // Ticking and filtering are disabled in the confirm view.
                    KeyCode::Char(' ') if !self.confirming => self.toggle(),
                    KeyCode::Backspace if !self.confirming => {
                        self.filter.pop();
                        self.refilter();
                    }
                    KeyCode::Char(c) if !ctrl && !self.confirming => {
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
        Ok(Some(self.result()))
    }

    /// The item under the cursor, if any.
    fn focused(&self) -> Option<&Item<K>> {
        self.filtered.get(self.cursor).map(|&i| &self.all[i])
    }

    /// The keys of the ticked items, in list order.
    fn ticked(&self) -> Vec<K> {
        self.all
            .iter()
            .filter(|item| self.selected.contains(&item.key))
            .map(|item| item.key.clone())
            .collect()
    }

    /// The chosen keys once the picker is done (not cancelled).
    fn result(&self) -> Vec<K> {
        if self.launcher {
            // Confirm view -> the ticked set (merged session); browse -> the
            // single focused deck that Enter launched.
            return if self.confirming {
                self.ticked()
            } else {
                self.focused()
                    .map(|item| vec![item.key.clone()])
                    .unwrap_or_default()
            };
        }
        let chosen = self.ticked();
        if self.exact || !chosen.is_empty() {
            return chosen;
        }
        // Startup picker with nothing ticked: use the item under the cursor.
        self.focused()
            .map(|item| vec![item.key.clone()])
            .unwrap_or_default()
    }

    /// Enters the launcher's confirm view: the list shows only the ticked
    /// decks.
    fn enter_confirm(&mut self) {
        self.confirming = true;
        self.filtered = (0..self.all.len())
            .filter(|&i| self.selected.contains(&self.all[i].key))
            .collect();
        self.cursor = 0;
        self.offset = 0;
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
            // A locked deck can't be ticked (it isn't startable). Non-deck
            // pickers never set `locked`, so this is a no-op for them.
            if self.all[i].locked {
                return;
            }
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

        let title = if self.confirming {
            "start these decks?"
        } else {
            &self.title
        };
        let left = format!(" flash — {} ({})", title, self.all.len());
        let right = format!("{} selected ", self.selected.len());
        frame.render_widget(bar(&left, &right, header.width), header);

        // Filter line with a cursor — hidden in the confirm view (filtering is
        // disabled there).
        if !self.confirming {
            frame.render_widget(Paragraph::new(format!(" filter: {}", self.filter)), filter);
            frame.set_cursor_position(Position::new(
                filter.x + 9 + self.filter.chars().count() as u16,
                filter.y,
            ));
        }

        self.draw_list(frame, list);

        let footer_text = self.footer_text();
        frame.render_widget(bar(&footer_text, "", footer.width), footer);
    }

    /// The footer hints for the current state. The launcher computes them per
    /// state; other pickers use their fixed `footer`.
    fn footer_text(&self) -> String {
        if !self.launcher {
            return self.footer.clone();
        }
        if self.confirming {
            " ENTER start merged │ ↑↓ move │ ESC back".to_string()
        } else if self.selected.is_empty() {
            " ENTER start │ SPACE tick │ ↑↓ move │ type to filter │ ESC cancel".to_string()
        } else {
            " ENTER start │ SPACE tick │ TAB confirm │ ↑↓ move │ ESC cancel".to_string()
        }
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
            let lock = if item.locked { "🔒 " } else { "" };
            // The confirm view lists only ticked decks, so the checkbox column
            // is redundant there.
            let main = if self.confirming {
                format!("{marker} {lock}{}", item.label)
            } else {
                let check = if checked { "[x]" } else { "[ ]" };
                format!("{marker} {check} {lock}{}", item.label)
            };

            let mut style = Style::new();
            if on_cursor {
                style = style.fg(Color::Black).bg(Color::Cyan);
            } else if checked {
                style = style.fg(Color::Cyan);
            } else if item.locked {
                // Advisory: locked decks are dimmed but still selectable.
                style = style.fg(Color::DarkGray);
            }

            let mut spans = vec![Span::styled(main, style)];
            if let Some(meta) = &item.meta {
                // Tint the state suffix (finished → green, exam due → yellow),
                // but keep the cursor and locked styling dominant where they apply.
                let meta_style = if on_cursor || item.locked {
                    style
                } else {
                    match item.state {
                        Some(DeckState::Finished) => Style::new().fg(Color::Green),
                        Some(DeckState::ExamDue) => Style::new().fg(Color::Yellow),
                        _ => style,
                    }
                };
                spans.push(Span::styled(format!("  {meta}"), meta_style));
            }
            lines.push(Line::from(spans));
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
                state: None,
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

    fn launcher_with(items: &[(&str, bool)]) -> Picker<PathBuf> {
        let all = items
            .iter()
            .map(|(n, locked)| Item {
                key: PathBuf::from(n),
                label: n.to_string(),
                meta: None,
                locked: *locked,
                state: None,
            })
            .collect();
        let mut p = Picker::new(
            all,
            HashSet::new(),
            "t".to_string(),
            String::new(),
            false,
            Vec::new(),
        );
        p.launcher = true;
        p
    }

    #[test]
    fn launcher_enter_returns_focused_deck() {
        let mut p = launcher_with(&[("a.txt", false), ("b.txt", false)]);
        p.cursor = 1;
        assert_eq!(vec![PathBuf::from("b.txt")], p.result());
    }

    #[test]
    fn launcher_confirm_returns_ticked_set() {
        let mut p = launcher_with(&[("a.txt", false), ("b.txt", false), ("c.txt", false)]);
        p.selected.insert(PathBuf::from("a.txt"));
        p.selected.insert(PathBuf::from("c.txt"));
        p.enter_confirm();
        assert!(p.confirming);
        assert_eq!(2, p.filtered.len()); // confirm view shows only ticked decks
        assert_eq!(
            vec![PathBuf::from("a.txt"), PathBuf::from("c.txt")],
            p.result()
        );
    }

    #[test]
    fn launcher_does_not_tick_locked_decks() {
        let mut p = launcher_with(&[("locked.txt", true)]);
        p.cursor = 0;
        p.toggle();
        assert!(p.selected.is_empty());
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
