//! Startup deck picker: a small TUI to choose one or more decks to review,
//! used when `fc` is launched without deck arguments (e.g. from the desktop
//! menu).
//!
//! Recently reviewed decks are listed first, then the rest of the decks in
//! the decks directory. Type to filter by name, Space to (de)select, Enter
//! to start.

use std::{
    collections::HashSet,
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

use crate::{recent::RecentDecks, time};

const HEADER_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Cyan);

/// A selectable deck.
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

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Runs the startup picker. Returns the chosen deck paths (empty if the user
/// cancelled or there is nothing to pick).
pub fn pick(decks_dir: &Path, recent: &RecentDecks) -> Result<Vec<PathBuf>> {
    let candidates = build_candidates(decks_dir, recent);
    let mut picker = Picker::new(candidates, decks_dir.to_path_buf());
    let mut terminal = ratatui::init();
    let result = picker.run(&mut terminal);
    ratatui::restore();
    Ok(result?.unwrap_or_default())
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

    let mut picker = Picker::with(
        candidates,
        decks_dir.to_path_buf(),
        preselected,
        format!("prerequisites for {target_name}"),
        " SPACE toggle │ ENTER save │ ↑↓ move │ type to filter │ ESC cancel".to_string(),
        true,
    );
    let mut terminal = ratatui::init();
    let result = picker.run(&mut terminal);
    ratatui::restore();
    result
}

/// A deck name without its `.txt` extension, for matching.
fn stem(name: &str) -> String {
    name.strip_suffix(".txt").unwrap_or(name).to_string()
}

struct Picker {
    decks_dir: PathBuf,
    all: Vec<Candidate>,
    filter: String,
    /// Indices into `all` matching the filter.
    filtered: Vec<usize>,
    /// Cursor position within `filtered`.
    cursor: usize,
    /// Scroll offset within `filtered`.
    offset: usize,
    selected: HashSet<PathBuf>,
    /// Header label (after "flash — ").
    title: String,
    /// Footer key hints.
    footer: String,
    /// When true, Enter returns exactly the ticked set (possibly empty); when
    /// false (startup picker), an empty tick set falls back to the card under
    /// the cursor.
    exact: bool,
    done: bool,
    cancelled: bool,
}

impl Picker {
    /// The startup picker: nothing pre-selected, cursor fallback on Enter.
    fn new(all: Vec<Candidate>, decks_dir: PathBuf) -> Self {
        Self::with(
            all,
            decks_dir,
            HashSet::new(),
            "select decks".to_string(),
            " SPACE select │ ENTER start │ ↑↓ move │ type to filter │ ESC cancel".to_string(),
            false,
        )
    }

    fn with(
        all: Vec<Candidate>,
        decks_dir: PathBuf,
        selected: HashSet<PathBuf>,
        title: String,
        footer: String,
        exact: bool,
    ) -> Self {
        let filtered = (0..all.len()).collect();
        Self {
            decks_dir,
            all,
            filter: String::new(),
            filtered,
            cursor: 0,
            offset: 0,
            selected,
            title,
            footer,
            exact,
            done: false,
            cancelled: false,
        }
    }

    /// Returns `None` if cancelled, else the chosen paths.
    fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<Option<Vec<PathBuf>>> {
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
        // Selected decks, in candidate order.
        let chosen: Vec<PathBuf> = self
            .all
            .iter()
            .filter(|c| self.selected.contains(&c.path))
            .map(|c| c.path.clone())
            .collect();
        if self.exact || !chosen.is_empty() {
            return Ok(Some(chosen));
        }
        // Startup picker with nothing ticked: use the card under the cursor.
        Ok(Some(
            self.filtered
                .get(self.cursor)
                .map(|&i| vec![self.all[i].path.clone()])
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
            let path = &self.all[i].path;
            if !self.selected.remove(path) {
                self.selected.insert(path.clone());
            }
        }
    }

    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, c)| c.name.to_lowercase().contains(&needle))
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
            frame.render_widget(
                Paragraph::new(vec![
                    Line::default(),
                    Line::from(format!("  No decks found in {}.", self.decks_dir.display())),
                    Line::default(),
                    Line::from(
                        "  Put .txt decks there, or pass deck files on the command line.".dim(),
                    ),
                ]),
                area,
            );
            return;
        }

        let height = area.height as usize;
        // Keep the cursor within the visible window.
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }

        let now = time::now_ms();
        let mut lines = Vec::new();
        for (row, &i) in self
            .filtered
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(height)
        {
            let c = &self.all[i];
            let on_cursor = row == self.cursor;
            let checked = self.selected.contains(&c.path);

            let marker = if on_cursor { "›" } else { " " };
            let check = if checked { "[x]" } else { "[ ]" };
            let age = match c.last_used_ms {
                Some(ts) if ts <= now => {
                    format!("  · {} ago", time::humanize_ms(now - ts))
                }
                Some(_) => "  · recent".to_string(),
                None => String::new(),
            };
            let text = format!("{marker} {check} {}{age}", c.name);

            let mut style = Style::new();
            if on_cursor {
                style = style.fg(Color::Black).bg(Color::Cyan);
            } else if checked {
                style = style.fg(Color::Cyan);
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

    fn candidate(name: &str, recent: Option<u64>) -> Candidate {
        Candidate {
            path: PathBuf::from(name),
            name: name.to_string(),
            last_used_ms: recent,
        }
    }

    fn picker_with(names: &[&str]) -> Picker {
        let all = names.iter().map(|n| candidate(n, None)).collect();
        Picker::new(all, PathBuf::from("/decks"))
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
            .filter(|c| p.selected.contains(&c.path))
            .map(|c| c.path.clone())
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
