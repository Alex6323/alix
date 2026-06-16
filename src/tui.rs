//! The interactive review TUI built on ratatui.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

use anyhow::Result;
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    answer::{FuzzyResult, Mode, TypingValidator, grade_fuzzy},
    ask,
    card::Card,
    choice::{self, ChoiceQuestion},
    config::{AskConfig, Bindings, Key, KeyPattern},
    deck,
    scheduler::Grade,
    session::{Session, SessionStats},
    store::Store,
    time,
};

pub(crate) const HEADER_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Cyan);

/// What the user is currently doing.
enum Phase {
    /// Typing the answer character by character.
    Typing {
        validators: Vec<TypingValidator>,
        line: usize,
        /// Currently displayed hint text, cleared on the next keystroke.
        hint: Option<String>,
    },
    /// Typing whole lines, submitted with Enter and graded fuzzily.
    Fuzzy {
        results: Vec<FuzzyResult>,
        input: String,
    },
    /// Looking at the front, answer hidden until revealed.
    Flip { revealed: bool },
    /// Revealing the back one line at a time. `revealed` is the number of
    /// back lines shown so far; once it reaches the line count the card is
    /// fully uncovered and graded like flip mode.
    LineByLine { revealed: usize },
    /// Picking one of several offered answers. `selected` is set once the
    /// user chose (the grade is applied at that moment); the card is a
    /// snapshot because the session advances on grading.
    Choice {
        card: Card,
        question: ChoiceQuestion,
        selected: Option<usize>,
    },
    /// Showing the result of the answered card.
    Feedback {
        card: Card,
        grade: Grade,
        fuzzy: Vec<FuzzyResult>,
    },
    /// Asking Claude about a card; entered from a post-answer screen and
    /// returned from with Esc. `return_to` restores that screen.
    Ask {
        return_to: Box<Phase>,
        card: Card,
        /// Completed question/answer exchanges.
        transcript: Vec<ask::Exchange>,
        /// The question currently being typed.
        input: String,
        /// Caret position within `input`, counted in characters (0..=len).
        cursor: usize,
        /// A pending CLI call, if any.
        waiting: Option<Waiting>,
        /// Scroll offset of the transcript (clamped while drawing).
        scroll: u16,
        /// Transient status line (errors, "note saved").
        status: Option<String>,
    },
    /// The session is over; showing totals.
    Summary,
}

/// A pending background CLI call.
struct Waiting {
    rx: Receiver<ask::Reply>,
    purpose: Purpose,
}

/// What a pending CLI call is for.
enum Purpose {
    /// A question; holds the text to add to the transcript on success.
    Question(String),
    /// Condensing the conversation into note lines for the deck file.
    Condense,
}

/// Per-deck information the TUI needs, keyed by subject.
pub struct DeckInfo {
    /// The deck file, for saving notes from the ask view.
    pub path: PathBuf,
    /// Reference links (`% link:` lines) offered to Claude as background.
    pub links: Vec<String>,
}

/// Static settings of a review run.
pub struct Options {
    /// The answer mode.
    pub mode: Mode,
    /// Fuzzy-mode typo tolerance per line.
    pub max_typos: usize,
    /// Deck names shown in the header.
    pub deck_label: String,
    /// Key bindings.
    pub keys: Bindings,
    /// Ask-Claude settings.
    pub ask: AskConfig,
    /// Loaded decks by subject.
    pub decks: HashMap<String, DeckInfo>,
}

/// The review application.
pub struct App {
    session: Session,
    store: Store,
    options: Options,
    phase: Phase,
    /// Counters across all sessions of this run (restarts included).
    totals: SessionStats,
    /// Set when a restart found nothing due; shown on the summary screen.
    nothing_due: bool,
    /// Largest useful transcript scroll offset, cached while drawing.
    ask_max_scroll: std::cell::Cell<u16>,
    /// The CLI conversation spanning this run; created lazily, reset on
    /// errors.
    ask_session: ask::CliSession,
    quit: bool,
}

impl App {
    /// Creates the app and primes the first card.
    pub fn new(session: Session, store: Store, options: Options) -> Self {
        let mut app = Self {
            session,
            store,
            options,
            phase: Phase::Summary,
            totals: SessionStats::default(),
            nothing_due: false,
            ask_max_scroll: std::cell::Cell::new(0),
            ask_session: ask::CliSession::new(),
            quit: false,
        };
        app.start_card();
        app
    }

    /// Runs the TUI until the user quits. Returns the counters accumulated
    /// over all sessions of this run.
    pub fn run(mut self) -> Result<SessionStats> {
        let mut terminal = ratatui::init();
        let result = self.event_loop(&mut terminal);
        ratatui::restore();
        // Progress is saved after every grade, but save once more in case
        // the loop exited between grades.
        self.store.save()?;
        result?;
        Ok(self.totals)
    }

    /// Starts a new session over the same decks, or flags that nothing is
    /// due yet so the summary can say so.
    fn try_restart(&mut self) {
        if self.session.restart(&self.store, time::now_ms()) {
            self.nothing_due = false;
            self.start_card();
        } else {
            self.nothing_due = true;
        }
    }

    fn event_loop(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
        while !self.quit {
            terminal.draw(|frame| self.draw(frame))?;
            // Poll instead of blocking so pending CLI replies (and the
            // spinner) are picked up while no key is pressed.
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key(key)?;
            }
            self.poll_ask();
        }
        Ok(())
    }

    /// Checks whether a pending ask-Claude call has delivered its reply.
    fn poll_ask(&mut self) {
        let Phase::Ask {
            card,
            transcript,
            waiting,
            scroll,
            status,
            ..
        } = &mut self.phase
        else {
            return;
        };
        let Some(w) = waiting else {
            return;
        };
        let reply = match w.rx.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                ask::Reply::Error("the ask thread died unexpectedly".to_string())
            }
        };
        match (reply, &w.purpose) {
            (ask::Reply::Answer(answer), Purpose::Question(question)) => {
                // The CLI call succeeded, so the session now exists and
                // later calls resume it.
                self.ask_session.started = true;
                transcript.push((question.clone(), answer));
                // Jump to the bottom of the transcript (clamped on draw).
                *scroll = self.ask_max_scroll.get().saturating_add(100);
            }
            (ask::Reply::Answer(text), Purpose::Condense) => {
                self.ask_session.started = true;
                let notes = ask::extract_note_lines(&text);
                *status = Some(match self.options.decks.get(&*card.subject) {
                    None => format!("no deck file known for {}", card.subject),
                    Some(info) => {
                        match deck::append_note(&info.path, card.line, &notes) {
                            Ok(()) if notes.is_empty() => "nothing to save".to_string(),
                            Ok(()) => {
                                // Show the note on this card immediately, too.
                                let addition = notes.join("\n");
                                match &mut card.note {
                                    Some(note) => {
                                        note.push('\n');
                                        note.push_str(&addition);
                                    }
                                    note @ None => *note = Some(addition),
                                }
                                format!("note saved to {}", info.path.display())
                            }
                            Err(e) => format!("cannot save note: {e}"),
                        }
                    }
                });
            }
            (ask::Reply::Error(e), _) => {
                *status = Some(e);
                // Don't try to resume a session in an unknown state; the
                // next question starts a fresh one.
                self.ask_session = ask::CliSession::new();
            }
        }
        *waiting = None;
    }

    /// Sets up the phase for the next card, or the summary if none is left.
    fn start_card(&mut self) {
        let Some(card) = self.session.current() else {
            self.phase = Phase::Summary;
            return;
        };
        self.phase = match self.options.mode {
            Mode::Typing => Phase::Typing {
                validators: card.back.iter().map(|l| TypingValidator::new(l)).collect(),
                line: 0,
                hint: None,
            },
            Mode::Fuzzy => Phase::Fuzzy {
                results: Vec::new(),
                input: String::new(),
            },
            Mode::Flip => Phase::Flip { revealed: false },
            Mode::LineByLine => Phase::LineByLine { revealed: 0 },
            Mode::Choice => {
                match choice::build(card, self.session.cards(), time::now_ms()) {
                    Some(question) => Phase::Choice {
                        card: card.clone(),
                        question,
                        selected: None,
                    },
                    // Not enough distinct answers in the session to build
                    // distractors; fall back to flip mode for this card.
                    None => Phase::Flip { revealed: false },
                }
            }
        };
    }

    /// Applies a grade for the current card and persists the progress.
    fn apply_grade(&mut self, grade: Grade) -> Result<()> {
        self.session.grade(&mut self.store, grade, time::now_ms());
        self.totals.reviews += 1;
        if grade.passed() {
            self.totals.passed += 1;
        } else {
            self.totals.failed += 1;
        }
        Ok(self.store.save()?)
    }

    /// Grades the current card and moves to the feedback view (typing and
    /// fuzzy mode).
    fn finish_card(&mut self, grade: Grade, fuzzy: Vec<FuzzyResult>) -> Result<()> {
        let card = self.session.current().expect("a card is active").clone();
        self.apply_grade(grade)?;
        self.phase = Phase::Feedback { card, grade, fuzzy };
        Ok(())
    }

    /// Grades the current card and goes straight to the next one (flip mode:
    /// the answer was already revealed, a feedback screen would only cost an
    /// extra keypress).
    fn finish_card_and_advance(&mut self, grade: Grade) -> Result<()> {
        self.apply_grade(grade)?;
        self.start_card();
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        // The ask view has its own key handling (free-text input, and Esc
        // means "back", not "quit").
        if matches!(self.phase, Phase::Ask { .. }) {
            self.handle_ask_key(key);
            return Ok(());
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let pattern = key_pattern(&key);

        // While typing an answer, plain character bindings must not shadow
        // text input, so only ctrl-/special-key bindings are honored there.
        let text_input = matches!(self.phase, Phase::Typing { .. } | Phase::Fuzzy { .. });
        let hit = |list: &[KeyPattern]| {
            pattern.is_some_and(|p| {
                list.iter()
                    .any(|b| *b == p && !(text_input && b.is_plain_char()))
            })
        };

        // Bindings that apply across phases.
        let quit_hit = hit(&self.options.keys.quit);
        let skip_hit = hit(&self.options.keys.skip);
        let restart_hit = hit(&self.options.keys.restart);
        let ask_hit = hit(&self.options.keys.ask);
        let hint_hit = hit(&self.options.keys.hint);
        let submit_hit = hit(&self.options.keys.submit);
        let reveal_hit = hit(&self.options.keys.reveal);
        let again_hit = hit(&self.options.keys.again);
        let good_hit = hit(&self.options.keys.good);
        let easy_hit = hit(&self.options.keys.easy);
        let cont_hit = hit(&self.options.keys.cont);

        if quit_hit {
            self.quit = true;
            return Ok(());
        }
        // Skipping only makes sense while a card is still unanswered.
        let answerable = !matches!(
            self.phase,
            Phase::Summary
                | Phase::Feedback { .. }
                | Phase::Choice {
                    selected: Some(_),
                    ..
                }
        );
        if skip_hit && answerable {
            self.session.skip();
            self.start_card();
            return Ok(());
        }

        match &mut self.phase {
            Phase::Typing {
                validators,
                line,
                hint,
            } => {
                let v = &mut validators[*line];
                if hint_hit {
                    // Default bindings: Tab, Ctrl-H, and Ctrl-Backspace
                    // (legacy terminals deliver Ctrl-H as the latter).
                    *hint = Some(v.hint());
                    return Ok(());
                }
                match key.code {
                    KeyCode::Backspace if !ctrl => {
                        *hint = None;
                        v.backspace();
                    }
                    KeyCode::Char(c) if !ctrl => {
                        *hint = None;
                        v.type_char(c);
                        if v.is_complete() {
                            if *line + 1 < validators.len() {
                                *line += 1;
                            } else {
                                let passed = validators.iter().all(|v| v.passed());
                                let grade = if passed { Grade::Pass } else { Grade::Fail };
                                self.finish_card(grade, Vec::new())?;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Phase::Fuzzy { results, input } => {
                if submit_hit {
                    let card = self.session.current().expect("a card is active");
                    let expected = &card.back[results.len()];
                    results.push(grade_fuzzy(input, expected, self.options.max_typos));
                    input.clear();
                    if results.len() == card.back.len() {
                        let passed = results.iter().all(|r| r.passed);
                        let grade = if passed { Grade::Pass } else { Grade::Fail };
                        let results = std::mem::take(results);
                        self.finish_card(grade, results)?;
                    }
                    return Ok(());
                }
                match key.code {
                    KeyCode::Backspace if !ctrl => {
                        input.pop();
                    }
                    KeyCode::Char(c) if !ctrl => input.push(c),
                    _ => {}
                }
            }
            Phase::Flip { revealed } => {
                if !*revealed {
                    if reveal_hit {
                        *revealed = true;
                    }
                } else if again_hit {
                    self.finish_card_and_advance(Grade::Fail)?;
                } else if good_hit {
                    self.finish_card_and_advance(Grade::Pass)?;
                } else if easy_hit {
                    self.finish_card_and_advance(Grade::Easy)?;
                } else if ask_hit {
                    self.enter_ask();
                }
            }
            Phase::LineByLine { revealed } => {
                let total = self.session.current().map_or(0, |c| c.back.len());
                if *revealed < total {
                    if reveal_hit {
                        *revealed += 1;
                    }
                } else if again_hit {
                    self.finish_card_and_advance(Grade::Fail)?;
                } else if good_hit {
                    self.finish_card_and_advance(Grade::Pass)?;
                } else if easy_hit {
                    self.finish_card_and_advance(Grade::Easy)?;
                } else if ask_hit {
                    self.enter_ask();
                }
            }
            Phase::Choice {
                question, selected, ..
            } => {
                if selected.is_none() {
                    if let KeyCode::Char(c @ '1'..='9') = key.code {
                        let index = c as usize - '1' as usize;
                        if index < question.options.len() {
                            *selected = Some(index);
                            let grade = if index == question.correct {
                                Grade::Pass
                            } else {
                                Grade::Fail
                            };
                            self.apply_grade(grade)?;
                        }
                    }
                } else if cont_hit {
                    self.start_card();
                } else if ask_hit {
                    self.enter_ask();
                }
            }
            Phase::Feedback { .. } => {
                if cont_hit {
                    self.start_card();
                } else if ask_hit {
                    self.enter_ask();
                }
            }
            // Handled by handle_ask_key before reaching this match.
            Phase::Ask { .. } => {}
            Phase::Summary => {
                if restart_hit {
                    self.try_restart();
                } else {
                    self.quit = true;
                }
            }
        }
        Ok(())
    }

    /// Switches to the ask view for the card on screen, remembering the
    /// current screen to return to.
    fn enter_ask(&mut self) {
        let card = match &self.phase {
            Phase::Feedback { card, .. } | Phase::Choice { card, .. } => card.clone(),
            Phase::Flip { .. } | Phase::LineByLine { .. } => match self.session.current() {
                Some(card) => card.clone(),
                None => return,
            },
            _ => return,
        };
        let return_to = Box::new(std::mem::replace(&mut self.phase, Phase::Summary));
        self.phase = Phase::Ask {
            return_to,
            card,
            transcript: Vec::new(),
            input: String::new(),
            cursor: 0,
            waiting: None,
            scroll: 0,
            status: None,
        };
    }

    /// Key handling inside the ask view.
    fn handle_ask_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let pattern = key_pattern(&key);
        // Free-text input: only ctrl-/special-key bindings may act.
        let hit = |list: &[KeyPattern]| {
            pattern.is_some_and(|p| list.iter().any(|b| *b == p && !b.is_plain_char()))
        };
        let save_hit = hit(&self.options.keys.save_note);
        let max_scroll = self.ask_max_scroll.get();

        // Esc leaves the ask view (abandoning a pending call); Ctrl-C still
        // quits the whole app.
        if key.code == KeyCode::Esc {
            if let Phase::Ask { return_to, .. } = std::mem::replace(&mut self.phase, Phase::Summary)
            {
                self.phase = *return_to;
            }
            return;
        }
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }

        let Phase::Ask {
            card,
            transcript,
            input,
            cursor,
            waiting,
            scroll,
            status,
            ..
        } = &mut self.phase
        else {
            return;
        };

        if save_hit {
            if !transcript.is_empty() && waiting.is_none() {
                let prompt = ask::condense_prompt(card, transcript);
                *status = None;
                *waiting = Some(Waiting {
                    rx: ask::spawn(self.options.ask.clone(), prompt, self.ask_session.args()),
                    purpose: Purpose::Condense,
                });
            }
            return;
        }

        match key.code {
            KeyCode::Enter if waiting.is_none() && !input.trim().is_empty() => {
                let question = std::mem::take(input);
                *cursor = 0;
                let links = self
                    .options
                    .decks
                    .get(&*card.subject)
                    .map(|info| info.links.as_slice())
                    .unwrap_or(&[]);
                let prompt =
                    ask::question_prompt(card, links, &question, !self.ask_session.started);
                *status = None;
                *waiting = Some(Waiting {
                    rx: ask::spawn(self.options.ask.clone(), prompt, self.ask_session.args()),
                    purpose: Purpose::Question(question),
                });
            }
            KeyCode::Backspace if !ctrl => {
                // Delete the character to the left of the caret.
                if *cursor > 0 {
                    let byte = char_byte(input, *cursor - 1);
                    input.remove(byte);
                    *cursor -= 1;
                }
            }
            KeyCode::Delete if !ctrl => {
                // Delete the character under the caret.
                if let Some((byte, _)) = input.char_indices().nth(*cursor) {
                    input.remove(byte);
                }
            }
            KeyCode::Left => *cursor = cursor.saturating_sub(1),
            KeyCode::Right => *cursor = (*cursor + 1).min(input.chars().count()),
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = input.chars().count(),
            KeyCode::Char('a') if ctrl => *cursor = 0,
            KeyCode::Char('e') if ctrl => *cursor = input.chars().count(),
            KeyCode::PageUp => *scroll = (*scroll).min(max_scroll).saturating_sub(5),
            KeyCode::PageDown => *scroll = (*scroll).saturating_add(5).min(max_scroll),
            KeyCode::Char('u') if ctrl => *scroll = (*scroll).min(max_scroll).saturating_sub(5),
            KeyCode::Char('d') if ctrl => *scroll = (*scroll).saturating_add(5).min(max_scroll),
            KeyCode::Char(c) if !ctrl => {
                // Insert at the caret and step over it.
                let byte = char_byte(input, *cursor);
                input.insert(byte, c);
                *cursor += 1;
            }
            _ => {}
        }
    }

    // ---- rendering -----------------------------------------------------

    fn draw(&self, frame: &mut Frame) {
        let [header, _, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        self.draw_header(frame, header);
        self.draw_footer(frame, footer);
        match &self.phase {
            Phase::Ask { .. } => self.draw_ask(frame, body),
            Phase::Summary => self.draw_summary(frame, body),
            _ => self.draw_card(frame, body),
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let left = format!(
            " flash {} │ {}",
            env!("CARGO_PKG_VERSION"),
            self.options.deck_label
        );
        let h = self.session.stage_histogram(&self.store);
        let right = format!(
            "{}|{}|{}|{}|{}|{}  left: {} ",
            h[0],
            h[1],
            h[2],
            h[3],
            h[4],
            h[5],
            self.session.remaining()
        );
        frame.render_widget(bar(&left, &right, area.width), area);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let k = &self.options.keys;
        let l = Bindings::label;
        let keys = match &self.phase {
            Phase::Typing { .. } => {
                format!(
                    "{} hint │ {} skip │ {} quit",
                    l(&k.hint),
                    l(&k.skip),
                    l(&k.quit)
                )
            }
            Phase::Fuzzy { .. } => format!(
                "{} submit line │ {} skip │ {} quit",
                l(&k.submit),
                l(&k.skip),
                l(&k.quit)
            ),
            Phase::Flip { revealed: false } => {
                format!(
                    "{} reveal │ {} skip │ {} quit",
                    l(&k.reveal),
                    l(&k.skip),
                    l(&k.quit)
                )
            }
            Phase::Flip { revealed: true } => format!(
                "{} again │ {} good │ {} easy │ {} ask │ {} quit",
                l(&k.again),
                l(&k.good),
                l(&k.easy),
                l(&k.ask),
                l(&k.quit)
            ),
            Phase::LineByLine { revealed } => {
                let total = self.session.current().map_or(0, |c| c.back.len());
                if *revealed < total {
                    format!(
                        "{} reveal next │ {} skip │ {} quit",
                        l(&k.reveal),
                        l(&k.skip),
                        l(&k.quit)
                    )
                } else {
                    format!(
                        "{} again │ {} good │ {} easy │ {} ask │ {} quit",
                        l(&k.again),
                        l(&k.good),
                        l(&k.easy),
                        l(&k.ask),
                        l(&k.quit)
                    )
                }
            }
            Phase::Choice {
                question,
                selected: None,
                ..
            } => {
                format!(
                    "1-{} select │ {} skip │ {} quit",
                    question.options.len(),
                    l(&k.skip),
                    l(&k.quit)
                )
            }
            Phase::Choice {
                selected: Some(_), ..
            }
            | Phase::Feedback { .. } => {
                format!(
                    "{} continue │ {} ask │ {} quit",
                    l(&k.cont),
                    l(&k.ask),
                    l(&k.quit)
                )
            }
            Phase::Ask {
                waiting: Some(_), ..
            } => "thinking… │ ESC back".to_string(),
            Phase::Ask { .. } => format!(
                "ENTER send │ {} save note │ PgUp/PgDn scroll │ ESC back",
                l(&k.save_note)
            ),
            Phase::Summary => {
                format!("{} new session │ any other key to exit", l(&k.restart))
            }
        };
        let left = format!(" {keys}");
        let right = format!(
            "{}✓ {}✗ ",
            self.session.stats.passed, self.session.stats.failed
        );
        frame.render_widget(bar(&left, &right, area.width), area);
    }

    fn draw_card(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();
        let mut cursor: Option<(u16, u16)> = None;

        // During feedback (and an answered choice) the queue has already
        // advanced, so take the card from the phase.
        let card = match &self.phase {
            Phase::Feedback { card, .. } | Phase::Choice { card, .. } => card,
            _ => match self.session.current() {
                Some(card) => card,
                None => return,
            },
        };

        lines.push(Line::from(card.front.clone().bold()));
        // Cloze cards show their masked answer text below the front.
        for ctx in &card.context {
            lines.push(Line::from(Span::styled(
                format!("  {ctx}"),
                Style::new().fg(Color::Cyan),
            )));
        }
        lines.push(Line::default());

        match &self.phase {
            Phase::Typing {
                validators,
                line,
                hint,
            } => {
                for (i, v) in validators.iter().enumerate() {
                    let mut spans = vec![Span::raw("> ")];
                    let mut width = 2u16;
                    for t in v.typed() {
                        let color = if t.correct { Color::Green } else { Color::Red };
                        spans.push(Span::styled(t.ch.to_string(), Style::new().fg(color)));
                        width += 1;
                    }
                    if i == *line {
                        cursor = Some((area.x + width, area.y + lines.len() as u16));
                        if let Some(hint) = hint {
                            spans.push(Span::styled(hint.clone(), Style::new().fg(Color::Yellow)));
                        }
                    }
                    if i <= *line {
                        lines.push(Line::from(spans));
                    } else {
                        lines.push(Line::from("> ".to_string()));
                    }
                }
            }
            Phase::Fuzzy { results, input } => {
                for r in results {
                    let color = if r.passed { Color::Green } else { Color::Red };
                    let mut spans = vec![
                        Span::raw("> "),
                        Span::styled(r.input.clone(), Style::new().fg(color)),
                    ];
                    // Show the exact answer whenever the input differed,
                    // even on a pass within tolerance.
                    if r.distance > 0 {
                        spans.push(Span::styled(
                            format!("  (expected: {})", r.expected),
                            Style::new().fg(Color::Yellow),
                        ));
                    }
                    lines.push(Line::from(spans));
                }
                if results.len() < card.back.len() {
                    cursor = Some((
                        area.x + 2 + input.chars().count() as u16,
                        area.y + lines.len() as u16,
                    ));
                    lines.push(Line::from(format!("> {input}")));
                }
            }
            Phase::Flip { revealed } => {
                if *revealed {
                    for back in &card.back {
                        lines.push(Line::from(Span::styled(
                            format!("  {back}"),
                            Style::new().fg(Color::Green),
                        )));
                    }
                    push_note(&mut lines, card, area.width);
                    lines.push(Line::default());
                    lines.push(Line::from("How well did you know it?".italic()));
                } else {
                    let reveal = Bindings::label(&self.options.keys.reveal);
                    lines.push(Line::from(
                        format!("[ press {reveal} to reveal the answer ]").dim(),
                    ));
                }
            }
            Phase::LineByLine { revealed } => {
                for back in card.back.iter().take(*revealed) {
                    lines.push(Line::from(Span::styled(
                        format!("  {back}"),
                        Style::new().fg(Color::Green),
                    )));
                }
                if *revealed < card.back.len() {
                    let reveal = Bindings::label(&self.options.keys.reveal);
                    lines.push(Line::from(
                        format!("[ press {reveal} to reveal the next line ]").dim(),
                    ));
                } else {
                    push_note(&mut lines, card, area.width);
                    lines.push(Line::default());
                    lines.push(Line::from("How well did you know it?".italic()));
                }
            }
            Phase::Choice {
                question, selected, ..
            } => {
                for (i, option) in question.options.iter().enumerate() {
                    let style = match selected {
                        None => Style::new(),
                        Some(_) if i == question.correct => Style::new().fg(Color::Green),
                        Some(s) if i == *s => Style::new().fg(Color::Red),
                        Some(_) => Style::new().fg(Color::DarkGray),
                    };
                    for (j, text) in option.lines().enumerate() {
                        let prefix = if j == 0 {
                            format!("  {}) ", i + 1)
                        } else {
                            "     ".into()
                        };
                        lines.push(Line::from(Span::styled(format!("{prefix}{text}"), style)));
                    }
                }
                if let Some(selected) = selected {
                    push_note(&mut lines, card, area.width);
                    lines.push(Line::default());
                    lines.push(if selected == &question.correct {
                        Line::from(" PASSED ".bold().fg(Color::Black).bg(Color::Green))
                    } else {
                        Line::from(" FAILED ".bold().fg(Color::Black).bg(Color::Red))
                    });
                }
            }
            Phase::Feedback { card, grade, fuzzy } => {
                if fuzzy.is_empty() {
                    for back in &card.back {
                        lines.push(Line::from(Span::styled(
                            format!("> {back}"),
                            Style::new().fg(Color::Green),
                        )));
                    }
                } else {
                    for r in fuzzy {
                        let color = if r.passed { Color::Green } else { Color::Red };
                        let mut spans = vec![
                            Span::raw("> "),
                            Span::styled(r.input.clone(), Style::new().fg(color)),
                        ];
                        if !r.passed {
                            spans.push(Span::styled(
                                format!("  (expected: {})", r.expected),
                                Style::new().fg(Color::Yellow),
                            ));
                        }
                        lines.push(Line::from(spans));
                    }
                }
                push_note(&mut lines, card, area.width);
                lines.push(Line::default());
                lines.push(if grade.passed() {
                    Line::from(" PASSED ".bold().fg(Color::Black).bg(Color::Green))
                } else {
                    Line::from(" FAILED ".bold().fg(Color::Black).bg(Color::Red))
                });
            }
            Phase::Ask { .. } | Phase::Summary => unreachable!(),
        }

        let mut paragraph = Paragraph::new(lines);
        if cursor.is_none() {
            // Wrap long lines (notes, options) — but never while an input
            // cursor is shown, because wrapping would shift its position.
            paragraph = paragraph.wrap(Wrap { trim: false });
        }
        frame.render_widget(paragraph, area);
        if let Some((x, y)) = cursor {
            frame.set_cursor_position(Position::new(x, y));
        }
    }

    fn draw_ask(&self, frame: &mut Frame, area: Rect) {
        let Phase::Ask {
            card,
            transcript,
            input,
            cursor,
            waiting,
            scroll,
            status,
            ..
        } = &self.phase
        else {
            return;
        };

        let [content, input_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

        // Compact card recap.
        let mut lines = vec![Line::from(card.front.clone().bold())];
        for ctx in &card.context {
            lines.push(Line::from(Span::styled(
                format!("  {ctx}"),
                Style::new().fg(Color::Cyan),
            )));
        }
        for back in &card.back {
            lines.push(Line::from(Span::styled(
                format!("> {back}"),
                Style::new().fg(Color::Green),
            )));
        }
        push_note(&mut lines, card, area.width);
        lines.push(Line::from("─── ask claude ───".dim()));
        lines.push(Line::default());

        for (question, answer) in transcript {
            lines.push(Line::from(
                format!("You: {question}").fg(Color::Cyan).bold(),
            ));
            lines.push(Line::default());
            for l in answer.lines() {
                lines.push(Line::from(l.to_string()));
            }
            lines.push(Line::default());
        }

        if waiting.is_some() {
            lines.push(Line::from(
                format!("Claude is thinking {}", spinner()).italic().dim(),
            ));
        }
        if let Some(status) = status {
            lines.push(Line::from(Span::styled(
                status.clone(),
                Style::new().fg(Color::Yellow),
            )));
        }

        // Scroll the transcript; remember the maximum so the key handler
        // can clamp paging. The wrapped height is estimated conservatively
        // (word wrap may break lines a bit earlier than the full width).
        let usable = content.width.saturating_sub(5).max(1) as usize;
        let total: usize = lines
            .iter()
            .map(|line| line.width().div_ceil(usable).max(1))
            .sum();
        let max_scroll = (total as u16).saturating_sub(content.height);
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        self.ask_max_scroll.set(max_scroll);
        frame.render_widget(paragraph.scroll(((*scroll).min(max_scroll), 0)), content);

        // The input line. Scroll horizontally so the caret stays visible when
        // the question outgrows the width: keep it at the right edge once it
        // moves past `visible`, otherwise show from the start.
        let visible = (input_area.width as usize).saturating_sub(3);
        let offset = cursor.saturating_sub(visible);
        let shown: String = input.chars().skip(offset).take(visible).collect();
        frame.render_widget(Paragraph::new(format!("> {shown}")), input_area);
        if waiting.is_none() {
            frame.set_cursor_position(Position::new(
                input_area.x + 2 + (cursor - offset) as u16,
                input_area.y,
            ));
        }
    }

    fn draw_summary(&self, frame: &mut Frame, area: Rect) {
        let stats = self.session.stats;
        let accuracy = if stats.reviews > 0 {
            format!("{:.0}%", 100.0 * stats.passed as f64 / stats.reviews as f64)
        } else {
            "-".to_string()
        };
        let h = self.session.stage_histogram(&self.store);

        let mut lines = vec![
            Line::from("Session complete!".bold()),
            Line::default(),
            Line::from(format!(
                "  reviews:  {} ({} passed, {} failed)",
                stats.reviews, stats.passed, stats.failed
            )),
            Line::from(format!("  accuracy: {accuracy}")),
            Line::default(),
            Line::from(format!(
                "  stages:   new {} │ s1 {} │ s2 {} │ s3 {} │ s4 {} │ s5 {}",
                h[0], h[1], h[2], h[3], h[4], h[5]
            )),
        ];

        if self.nothing_due {
            lines.push(Line::default());
            let now = time::now_ms();
            let next = self
                .session
                .next_due_at(&self.store)
                .filter(|&due| due > now)
                .map(|due| format!(" — next card in {}", time::humanize_ms(due - now)))
                .unwrap_or_default();
            lines.push(Line::from(
                format!("  Nothing due right now{next}.").fg(Color::Yellow),
            ));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }
}

/// Converts a crossterm key event into a bindable [`KeyPattern`]. Keys we
/// don't support binding (function keys, arrows, ...) yield `None`.
pub(crate) fn key_pattern(key: &KeyEvent) -> Option<KeyPattern> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let k = match key.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        _ => return None,
    };
    Some(KeyPattern { key: k, ctrl })
}

/// A spinner frame derived from the wall clock (the event loop redraws
/// every ~100ms while a CLI call is pending).
fn spinner() -> char {
    const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    FRAMES[(time::now_ms() / 100) as usize % FRAMES.len()]
}

/// Renders a full-width colored bar with left- and right-aligned text.
pub(crate) fn bar(left: &str, right: &str, width: u16) -> Paragraph<'static> {
    let pad = (width as usize)
        .saturating_sub(left.chars().count())
        .saturating_sub(right.chars().count());
    let text = format!("{left}{}{right}", " ".repeat(pad));
    Paragraph::new(text).style(HEADER_STYLE)
}

/// Appends the card's note, if any, as a yellow quoted block with a left
/// bar. Prose is split into sentences (on `.`), each its own paragraph with
/// a blank line between them; long rows are word-wrapped and keep the bar.
/// A ```` ``` ```` fenced block is rendered verbatim — no sentence split, no
/// wrapping — so code keeps its indentation and reads as written.
pub(crate) fn push_note(lines: &mut Vec<Line>, card: &Card, width: u16) {
    let Some(note) = &card.note else {
        return;
    };
    lines.push(Line::default());
    const GUTTER: &str = "  │ ";
    const BAR: &str = "  │"; // gutter without the trailing space, for blanks
    let bar_style = Style::new().fg(Color::Yellow);
    let text_width = (width as usize).saturating_sub(GUTTER.chars().count());

    // Prose may be hard-wrapped across several `!` lines, so consecutive
    // prose lines are joined into one paragraph buffer and only then split
    // into real sentences — otherwise every wrapped line would look like its
    // own sentence. A blank bar line separates units (each sentence, and each
    // code block), owned by the start of a unit so boundaries get one blank.
    let mut in_code = false;
    let mut code_started = false; // first line of the current code block seen?
    let mut emitted = false; // any line pushed for this note yet?
    let mut prose = String::new(); // prose accumulated since the last flush

    for logical in note.lines() {
        if logical.trim_start().starts_with("```") {
            // Fence delimiter: toggle code mode; the ``` line is not rendered.
            if in_code {
                in_code = false;
            } else {
                flush_prose(lines, &mut prose, text_width, &mut emitted);
                in_code = true;
                code_started = false;
            }
            continue;
        }
        if in_code {
            if !code_started {
                if emitted {
                    lines.push(Line::from(Span::styled(BAR, bar_style)));
                }
                code_started = true;
            }
            // Verbatim: indentation preserved, no sentence split, no wrap.
            lines.push(Line::from(vec![
                Span::styled(GUTTER, bar_style),
                Span::styled(logical.to_string(), Style::new().fg(Color::Gray)),
            ]));
            emitted = true;
            continue;
        }
        let trimmed = logical.trim();
        if !trimmed.is_empty() {
            if !prose.is_empty() {
                prose.push(' ');
            }
            prose.push_str(trimmed);
        }
    }
    flush_prose(lines, &mut prose, text_width, &mut emitted);
}

/// Emits accumulated prose as one paragraph per sentence, separated by a
/// blank bar line, then clears the buffer. `emitted` tracks whether any note
/// line has been pushed yet, so the first unit is not preceded by a blank.
fn flush_prose(lines: &mut Vec<Line>, prose: &mut String, width: usize, emitted: &mut bool) {
    const GUTTER: &str = "  │ ";
    const BAR: &str = "  │";
    let bar_style = Style::new().fg(Color::Yellow);
    for sentence in split_sentences(prose) {
        if sentence.is_empty() {
            continue;
        }
        if *emitted {
            lines.push(Line::from(Span::styled(BAR, bar_style)));
        }
        for row in wrap_text(&sentence, width) {
            lines.push(Line::from(vec![
                Span::styled(GUTTER, bar_style),
                Span::styled(row, bar_style),
            ]));
        }
        *emitted = true;
    }
    prose.clear();
}

/// Byte offset of the `n`th character in `s`, or `s.len()` if `n` is at or
/// past the end. Lets a character-based caret edit a UTF-8 string safely.
fn char_byte(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map_or(s.len(), |(b, _)| b)
}

/// Splits a note line into sentences, breaking after a period that is
/// followed by whitespace or the end of the line. A period followed by a
/// non-space (as in "2.1") does not split, so numbers stay intact. The
/// terminating period stays attached to its sentence.
fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut sentences = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        let ends_sentence = chars[i] == '.' && chars.get(i + 1).is_none_or(|c| c.is_whitespace());
        if ends_sentence {
            let sentence: String = chars[start..=i].iter().collect();
            if !sentence.trim().is_empty() {
                sentences.push(sentence.trim().to_string());
            }
            start = i + 1;
        }
    }
    if start < chars.len() {
        let tail: String = chars[start..].iter().collect();
        if !tail.trim().is_empty() {
            sentences.push(tail.trim().to_string());
        }
    }
    if sentences.is_empty() {
        sentences.push(String::new());
    }
    sentences
}

/// Greedy word-wrap to `width` columns (counted in chars). Returns at least
/// one row, so a blank note line still renders the gutter. A word longer
/// than `width` (e.g. a long Move type path) is hard-broken across rows.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let wlen = word.chars().count();
        if line.is_empty() {
            // place `word` below
        } else if line.chars().count() + 1 + wlen <= width {
            line.push(' ');
            line.push_str(word);
            continue;
        } else {
            rows.push(std::mem::take(&mut line));
        }
        if wlen <= width {
            line.push_str(word);
        } else {
            for ch in word.chars() {
                if line.chars().count() == width {
                    rows.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
    }
    rows.push(line);
    rows
}

#[cfg(test)]
mod tests {
    use super::{char_byte, wrap_text};

    #[test]
    fn char_byte_maps_caret_to_utf8_offset() {
        // ASCII: caret index equals byte offset.
        assert_eq!(char_byte("hello", 0), 0);
        assert_eq!(char_byte("hello", 3), 3);
        // Past the end clamps to the byte length (append position).
        assert_eq!(char_byte("hello", 9), 5);
        // Multi-byte chars: the 'é' is two bytes, so later carets shift.
        assert_eq!(char_byte("héllo", 1), 1);
        assert_eq!(char_byte("héllo", 2), 3);
        assert_eq!(char_byte("héllo", 5), 6);
    }

    #[test]
    fn short_line_is_one_row() {
        assert_eq!(wrap_text("a short note", 40), vec!["a short note"]);
    }

    #[test]
    fn wraps_on_word_boundaries() {
        assert_eq!(wrap_text("a bb ccc", 4), vec!["a bb", "ccc"]);
    }

    #[test]
    fn hard_breaks_a_word_longer_than_width() {
        assert_eq!(wrap_text("ab supercali", 5), vec!["ab", "super", "cali"]);
    }

    #[test]
    fn empty_line_yields_one_empty_row() {
        // Keeps blank lines inside a multi-paragraph note as gutter-only rows.
        assert_eq!(wrap_text("", 10), vec![""]);
    }

    #[test]
    fn zero_width_does_not_panic() {
        assert_eq!(
            wrap_text("hi there", 0),
            vec!["h", "i", "t", "h", "e", "r", "e"]
        );
    }
}
