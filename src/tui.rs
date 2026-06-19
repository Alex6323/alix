//! The interactive review TUI built on ratatui.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

use anyhow::Result;
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    answer::{FuzzyResult, Mode, TypingValidator, best_prefix_match, grade_fuzzy},
    ask,
    card::Card,
    choice::{self, ChoiceQuestion},
    config::{AskConfig, Bindings, ExamConfig, Key, KeyPattern, Strictness},
    deck::{self, Deck, DeckState},
    exam,
    render::{self, ContextSpan, NoteUnit},
    scheduler::Grade,
    session::{Session, SessionStats},
    store::Store,
    time,
};

pub(crate) const HEADER_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Cyan);

/// What the user is currently doing.
enum Phase {
    /// Typing the answer character by character. Multi-line answers are graded
    /// order-independently: a row is matched to whichever expected line it
    /// completes, so the items can be typed in any order.
    Typing {
        /// The expected back lines.
        expected: Vec<String>,
        /// Which expected lines a completed row has already claimed (parallel
        /// to `expected`).
        claimed: Vec<bool>,
        /// Completed rows, in entry order — for grading and the feedback view.
        done: Vec<FuzzyResult>,
        /// The row being typed; its target is re-chosen as the user types.
        current: TypingValidator,
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
    /// Understanding card: type an explanation (optional, free text — not
    /// checked), reveal the back lines (the key points), then self-grade on
    /// whether you covered them. `input` is the typed reconstruction.
    Explain { input: String, revealed: bool },
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
    /// Showing the result of the answered card (typing or fuzzy mode). `mode`
    /// is the mode the card was answered in, so the view can label and render
    /// the lines correctly; `results` holds one graded line per back line.
    Feedback {
        card: Card,
        grade: Grade,
        mode: Mode,
        results: Vec<FuzzyResult>,
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
    /// CLI `--mode` override, applied to every card. `None` lets each card use
    /// its own mode (card `% mode:` > deck `% mode:` > built-in default).
    pub mode_override: Option<Mode>,
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
    /// Cards marked for removal: deck subject → front line numbers, applied to
    /// the deck files when the run ends.
    removed_lines: HashMap<String, BTreeSet<usize>>,
    /// Identity hashes of removed cards, pruned from the store at the end.
    removed_ids: HashSet<u64>,
    /// Set at the summary when the user chooses to sit the exam of a deck that
    /// became `exam due` this session; `run` returns it so `main` launches the
    /// exam after the review app exits.
    exam_request: Option<PathBuf>,
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
            removed_lines: HashMap::new(),
            removed_ids: HashSet::new(),
            exam_request: None,
            quit: false,
        };
        app.start_card();
        app
    }

    /// Runs the TUI until the user quits. Returns the counters accumulated over
    /// all sessions of this run, plus the deck whose exam the user asked to sit
    /// at the summary (if any).
    pub fn run(mut self) -> Result<(SessionStats, Option<PathBuf>)> {
        let mut terminal = ratatui::init();
        let result = self.event_loop(&mut terminal);
        ratatui::restore();
        // Apply pending card removals to the deck files now that the terminal
        // is back, so any message is visible. This also prunes their progress.
        self.flush_removals();
        // Progress is saved after every grade, but save once more in case
        // the loop exited between grades (and to persist the prunes above).
        self.store.save()?;
        result?;
        Ok((self.totals, self.exam_request.take()))
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

    /// Decks in this session that are now `exam due` (drilled, `% source:`, not
    /// yet mastered) — offered at the summary. Sorted by subject.
    fn exam_due_decks(&self) -> Vec<(String, PathBuf)> {
        let mut out: Vec<(String, PathBuf)> = self
            .options
            .decks
            .iter()
            .filter_map(|(subject, info)| {
                let deck = Deck::load(&info.path).ok()?;
                (!deck.sources.is_empty() && deck.state(&self.store) == DeckState::ExamDue)
                    .then(|| (subject.clone(), info.path.clone()))
            })
            .collect();
        out.sort();
        out
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
            // Reaching the summary: flag up front whether a new session could
            // start, so the screen can say "nothing due" without the user
            // having to press the restart key first.
            self.nothing_due = !self.session.has_due_now(&self.store, time::now_ms());
            self.phase = Phase::Summary;
            return;
        };
        // CLI override wins; otherwise the card's own mode (card > deck), else
        // the built-in default.
        let mode = self.options.mode_override.or(card.mode).unwrap_or_default();
        self.phase = match mode {
            Mode::Typing => {
                let expected = card.back.clone();
                let first = expected.first().cloned().unwrap_or_default();
                Phase::Typing {
                    claimed: vec![false; expected.len()],
                    current: TypingValidator::new(&first),
                    done: Vec::new(),
                    expected,
                    hint: None,
                }
            }
            Mode::Fuzzy => Phase::Fuzzy {
                results: Vec::new(),
                input: String::new(),
            },
            Mode::Flip => Phase::Flip { revealed: false },
            Mode::Explain => Phase::Explain {
                input: String::new(),
                revealed: false,
            },
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
    fn finish_card(&mut self, grade: Grade, mode: Mode, results: Vec<FuzzyResult>) -> Result<()> {
        let card = self.session.current().expect("a card is active").clone();
        self.apply_grade(grade)?;
        self.phase = Phase::Feedback {
            card,
            grade,
            mode,
            results,
        };
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

    /// Marks the current card (and any cloze siblings) for removal from its
    /// deck file, drops it from the queue, and moves on. The file edits and
    /// progress pruning happen together when the run ends, in
    /// [`flush_removals`].
    fn remove_card(&mut self) {
        let removed = self.session.remove_current();
        let Some(first) = removed.first() else {
            return;
        };
        // All removed cards share one source block (same subject and line).
        self.removed_lines
            .entry(first.subject.to_string())
            .or_default()
            .insert(first.line);
        for card in &removed {
            self.removed_ids.insert(card.id());
        }
        self.start_card();
    }

    /// Deletes every card marked for removal from its deck file and prunes the
    /// matching progress entries. Best-effort: a file that cannot be rewritten
    /// is reported but does not abort the others. Called once, after the
    /// terminal is restored.
    fn flush_removals(&mut self) {
        if self.removed_lines.is_empty() {
            return;
        }
        let mut cards = 0;
        let mut files = 0;
        for (subject, lines) in &self.removed_lines {
            let Some(info) = self.options.decks.get(subject) else {
                eprintln!("warning: no deck file known for {subject}; cannot remove cards");
                continue;
            };
            let lines: Vec<usize> = lines.iter().copied().collect();
            cards += lines.len();
            match deck::remove_cards(&info.path, &lines) {
                Ok(()) => files += 1,
                Err(e) => eprintln!("warning: could not update {}: {e}", info.path.display()),
            }
        }
        for id in &self.removed_ids {
            self.store.remove(*id);
        }
        if files > 0 {
            eprintln!("Removed {cards} card(s) from {files} deck file(s).");
        }
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
        // Explain mode takes free text only until the points are revealed.
        let text_input = matches!(
            self.phase,
            Phase::Typing { .. }
                | Phase::Fuzzy { .. }
                | Phase::Explain {
                    revealed: false,
                    ..
                }
        );
        let hit = |list: &[KeyPattern]| {
            pattern.is_some_and(|p| {
                list.iter()
                    .any(|b| *b == p && !(text_input && b.is_plain_char()))
            })
        };

        // Bindings that apply across phases.
        let quit_hit = hit(&self.options.keys.quit);
        let skip_hit = hit(&self.options.keys.skip);
        let remove_hit = hit(&self.options.keys.remove);
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
        // Marking for removal makes sense on the card you'd otherwise answer.
        if remove_hit && answerable {
            self.remove_card();
            return Ok(());
        }

        match &mut self.phase {
            Phase::Typing {
                expected,
                claimed,
                done,
                current,
                hint,
            } => {
                if hint_hit {
                    // Default bindings: Tab, Ctrl-H, and Ctrl-Backspace
                    // (legacy terminals deliver Ctrl-H as the latter).
                    *hint = Some(current.hint());
                    return Ok(());
                }
                match key.code {
                    KeyCode::Backspace if !ctrl => {
                        *hint = None;
                        current.backspace();
                        retarget(current, expected, claimed);
                    }
                    KeyCode::Char(c) if !ctrl => {
                        *hint = None;
                        current.type_char(c);
                        retarget(current, expected, claimed);
                        if current.is_complete() {
                            // Claim the (still-unclaimed) expected line this row
                            // matched.
                            let target = current.expected();
                            if let Some(idx) =
                                (0..expected.len()).find(|&i| !claimed[i] && expected[i] == target)
                            {
                                claimed[idx] = true;
                            }
                            done.push(typing_result(current));
                            if claimed.iter().all(|&c| c) {
                                let results = std::mem::take(done);
                                let passed = results.iter().all(|r| r.passed);
                                let grade = if passed { Grade::Pass } else { Grade::Fail };
                                self.finish_card(grade, Mode::Typing, results)?;
                            } else {
                                // Begin the next row on a still-unclaimed line.
                                let next = expected
                                    .iter()
                                    .enumerate()
                                    .find(|(i, _)| !claimed[*i])
                                    .map(|(_, e)| e.clone())
                                    .unwrap_or_default();
                                *current = TypingValidator::new(&next);
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
                        self.finish_card(grade, Mode::Fuzzy, results)?;
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
            Phase::Explain { input, revealed } => {
                if !*revealed {
                    // Free-text reconstruction; Enter reveals the points (a
                    // plain-char reveal binding would be swallowed by the input).
                    match key.code {
                        KeyCode::Enter => *revealed = true,
                        KeyCode::Backspace if !ctrl => {
                            input.pop();
                        }
                        KeyCode::Char(c) if !ctrl => input.push(c),
                        _ => {}
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
                // `x` sits the exam of a deck that became exam-due this session;
                // with nothing due the restart key is inert (the footer omits
                // it); any other key exits.
                let exam_due = self.exam_due_decks();
                if key.code == KeyCode::Char('x') && !exam_due.is_empty() {
                    self.exam_request = Some(exam_due[0].1.clone());
                    self.quit = true;
                } else if restart_hit && !self.nothing_due {
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
            Phase::Flip { .. } | Phase::LineByLine { .. } | Phase::Explain { .. } => {
                match self.session.current() {
                    Some(card) => card.clone(),
                    None => return,
                }
            }
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
        // The remaining-card count lives in the footer now (bottom-right),
        // mirroring the web frontend; the header keeps just the stage histogram.
        // Stages above the session's top stage are unreachable (every card caps
        // below them via `% max-stage:`), shown as `–` instead of a `0`.
        let c = hist_cells(&h, self.session.top_stage());
        let right = format!("{}|{}|{}|{}|{}|{} ", c[0], c[1], c[2], c[3], c[4], c[5]);
        frame.render_widget(bar(&left, &right, area.width), area);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let k = &self.options.keys;
        let l = Bindings::label;
        let keys = match &self.phase {
            Phase::Typing { .. } => {
                format!(
                    "{} hint │ {} skip │ {} remove │ {} quit",
                    l(&k.hint),
                    l(&k.skip),
                    l(&k.remove),
                    l(&k.quit)
                )
            }
            Phase::Fuzzy { .. } => format!(
                "{} submit line │ {} skip │ {} remove │ {} quit",
                l(&k.submit),
                l(&k.skip),
                l(&k.remove),
                l(&k.quit)
            ),
            Phase::Flip { revealed: false } => {
                format!(
                    "{} reveal │ {} skip │ {} remove │ {} quit",
                    l(&k.reveal),
                    l(&k.skip),
                    l(&k.remove),
                    l(&k.quit)
                )
            }
            Phase::Flip { revealed: true } => format!(
                "{} again │ {} good │ {} easy │ {} remove │ {} ask │ {} quit",
                l(&k.again),
                l(&k.good),
                l(&k.easy),
                l(&k.remove),
                l(&k.ask),
                l(&k.quit)
            ),
            Phase::Explain {
                revealed: false, ..
            } => format!(
                "ENTER reveal │ {} skip │ {} remove │ {} quit",
                l(&k.skip),
                l(&k.remove),
                l(&k.quit)
            ),
            Phase::Explain { revealed: true, .. } => format!(
                "{} again │ {} good │ {} easy │ {} remove │ {} ask │ {} quit",
                l(&k.again),
                l(&k.good),
                l(&k.easy),
                l(&k.remove),
                l(&k.ask),
                l(&k.quit)
            ),
            Phase::LineByLine { revealed } => {
                let total = self.session.current().map_or(0, |c| c.back.len());
                if *revealed < total {
                    format!(
                        "{} reveal next │ {} skip │ {} remove │ {} quit",
                        l(&k.reveal),
                        l(&k.skip),
                        l(&k.remove),
                        l(&k.quit)
                    )
                } else {
                    format!(
                        "{} again │ {} good │ {} easy │ {} remove │ {} ask │ {} quit",
                        l(&k.again),
                        l(&k.good),
                        l(&k.easy),
                        l(&k.remove),
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
                    "1-{} select │ {} skip │ {} remove │ {} quit",
                    question.options.len(),
                    l(&k.skip),
                    l(&k.remove),
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
            Phase::Summary if self.nothing_due => "any key to exit".to_string(),
            Phase::Summary => {
                let exam = if self.exam_due_decks().is_empty() {
                    ""
                } else {
                    "x take exam │ "
                };
                format!(
                    "{exam}{} new session │ any other key to exit",
                    l(&k.restart)
                )
            }
        };
        let left = format!(" {keys}");
        // Passed / failed, then the remaining count with a ↓ arrow — same
        // layout as the web frontend's score line.
        let right = format!(
            "{}✓ {}✗ {}↓ ",
            self.session.stats.passed,
            self.session.stats.failed,
            self.session.remaining()
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
            lines.push(context_line(ctx));
        }
        lines.push(Line::default());

        // A mode badge at the top of the answer section — typing and fuzzy in
        // particular look identical without it (both are an input prompt).
        let mode_tag = match &self.phase {
            Phase::Typing { .. } => "TYPING EXACT",
            Phase::Fuzzy { .. } => "TYPING FUZZY",
            Phase::Flip { .. } => "FLIP",
            Phase::Explain { .. } => "EXPLAIN",
            Phase::LineByLine { .. } => "LINE BY LINE",
            Phase::Choice { .. } => "CHOICE",
            Phase::Feedback {
                mode: Mode::Fuzzy, ..
            } => "TYPING FUZZY",
            Phase::Feedback { .. } => "TYPING EXACT",
            _ => "",
        };
        if !mode_tag.is_empty() {
            lines.push(Line::from(mode_tag.dim()));
            lines.push(Line::default());
        }

        match &self.phase {
            Phase::Typing {
                claimed,
                done,
                current,
                hint,
                ..
            } => {
                // Already-completed rows, in entry order.
                for r in done {
                    let color = if r.passed { Color::Green } else { Color::Red };
                    lines.push(Line::from(vec![
                        Span::raw("> "),
                        Span::styled(r.input.clone(), Style::new().fg(color)),
                    ]));
                }
                // The row being typed, with per-character feedback and cursor.
                let mut spans = vec![Span::raw("> ")];
                let mut width = 2u16;
                for t in current.typed() {
                    let color = if t.correct { Color::Green } else { Color::Red };
                    spans.push(Span::styled(t.ch.to_string(), Style::new().fg(color)));
                    width += 1;
                }
                cursor = Some((area.x + width, area.y + lines.len() as u16));
                if let Some(hint) = hint {
                    spans.push(Span::styled(hint.clone(), Style::new().fg(Color::Yellow)));
                }
                lines.push(Line::from(spans));
                // Placeholder prompts for the remaining unclaimed lines (the one
                // being typed is already shown above).
                let remaining = claimed.iter().filter(|&&c| !c).count();
                for _ in 1..remaining {
                    lines.push(Line::from("> ".to_string()));
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
            Phase::Explain { input, revealed } => {
                if *revealed {
                    // Your reconstruction (if any), then the points to compare it
                    // against, then self-grade.
                    if !input.is_empty() {
                        lines.push(Line::from("your answer:".dim()));
                        lines.push(Line::from(format!("  {input}")));
                        lines.push(Line::default());
                    }
                    lines.push(Line::from("your answer should cover:".dim()));
                    for point in &card.back {
                        lines.push(Line::from(Span::styled(
                            format!("  • {point}"),
                            Style::new().fg(Color::Green),
                        )));
                    }
                    push_note(&mut lines, card, area.width);
                    lines.push(Line::default());
                    lines.push(Line::from("How well did you cover them?".italic()));
                } else {
                    cursor = Some((
                        area.x + 2 + input.chars().count() as u16,
                        area.y + lines.len() as u16,
                    ));
                    lines.push(Line::from(format!("> {input}")));
                    lines.push(Line::default());
                    lines.push(Line::from(
                        "[ type your answer (optional), ENTER to reveal the points ]".dim(),
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
            Phase::Feedback {
                card,
                grade,
                mode,
                results,
            } => {
                for r in results {
                    let color = if r.passed { Color::Green } else { Color::Red };
                    lines.push(Line::from(vec![
                        Span::raw("> "),
                        Span::styled(r.input.clone(), Style::new().fg(color)),
                    ]));
                    // On a wrong line, show the correct answer underneath with a
                    // check mark so the right text — not the mistake — is what
                    // stays on screen.
                    if !r.passed {
                        let correction = match mode {
                            Mode::Fuzzy => format!("  (expected: {})", r.expected),
                            _ => format!("  ✓ {}", r.expected),
                        };
                        lines.push(Line::from(Span::styled(
                            correction,
                            Style::new().fg(Color::Green),
                        )));
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
            lines.push(context_line(ctx));
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

        // Stages above the session's top stage are unreachable (capped by
        // `% max-stage:`), shown dim as `–`.
        let top = self.session.top_stage();
        let mut stage_spans = vec![Span::raw(format!("  stages:   new {} │", h[0]))];
        for s in 1..=5u8 {
            if s > top {
                stage_spans.push(Span::styled(
                    format!(" s{s} –"),
                    Style::new().fg(Color::DarkGray),
                ));
            } else {
                stage_spans.push(Span::raw(format!(" s{s} {}", h[s as usize])));
            }
            if s < 5 {
                stage_spans.push(Span::raw(" │"));
            }
        }

        let mut lines = vec![
            Line::from("Session complete!".bold()),
            Line::default(),
            Line::from(format!(
                "  reviews:  {} ({} passed, {} failed)",
                stats.reviews, stats.passed, stats.failed
            )),
            Line::from(format!("  accuracy: {accuracy}")),
            Line::default(),
            Line::from(stage_spans),
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
        // Decks that became exam-due this session: offer the exam (press `x`).
        for (subject, _) in self.exam_due_decks() {
            lines.push(Line::default());
            lines.push(Line::from(
                format!("  ✦ {subject} is ready for its exam — press x to take it.")
                    .fg(Color::Yellow),
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

/// Appends the card's note, if any, as a yellow quoted block with a left bar.
/// The note's structure (sentence-split prose, verbatim code blocks) comes
/// from [`render::note_units`]; this function only paints it: each unit is
/// separated by a blank bar line, prose is word-wrapped to the width and
/// styled yellow, and code is rendered gray and verbatim.
pub(crate) fn push_note(lines: &mut Vec<Line>, card: &Card, width: u16) {
    let units = render::note_units(card);
    if units.is_empty() {
        return;
    }
    lines.push(Line::default());
    const GUTTER: &str = "  │ ";
    const BAR: &str = "  │"; // gutter without the trailing space, for blanks
    let bar_style = Style::new().fg(Color::Yellow);
    let text_width = (width as usize).saturating_sub(GUTTER.chars().count());

    for (i, unit) in units.iter().enumerate() {
        // One blank bar line between consecutive units, none before the first.
        if i > 0 {
            lines.push(Line::from(Span::styled(BAR, bar_style)));
        }
        match unit {
            NoteUnit::Sentence(sentence) => {
                for row in render::wrap_text(sentence, text_width) {
                    lines.push(Line::from(vec![
                        Span::styled(GUTTER, bar_style),
                        Span::styled(row, bar_style),
                    ]));
                }
            }
            NoteUnit::Code(code) => {
                for line in code {
                    lines.push(Line::from(vec![
                        Span::styled(GUTTER, bar_style),
                        Span::styled(line.clone(), Style::new().fg(Color::Gray)),
                    ]));
                }
            }
        }
    }
}

/// Builds the styled line for a cloze context string (indented two spaces):
/// the active blank is bright and bold, hidden sibling holes are dim, the rest
/// is cyan — matching the web frontend.
pub(crate) fn context_line(ctx: &str) -> Line<'static> {
    let mut spans = vec![Span::raw("  ")];
    for seg in render::context_spans(ctx) {
        let (text, style) = match seg {
            ContextSpan::Text(t) => (t, Style::new().fg(Color::Cyan)),
            ContextSpan::Blank(t) => (
                t,
                Style::new()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            ContextSpan::Hidden(t) => (t, Style::new().fg(Color::DarkGray)),
        };
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

/// The six stage-histogram cells (new, s1..s5) as strings, with stages above
/// `top_stage` shown as `–` — they are unreachable because every card caps
/// below them via `% max-stage:`.
fn hist_cells(h: &[usize; 6], top_stage: u8) -> [String; 6] {
    std::array::from_fn(|i| {
        if i >= 1 && i as u8 > top_stage {
            "–".to_string()
        } else {
            h[i].to_string()
        }
    })
}

/// Points the current typing row at whichever still-unclaimed expected line it
/// best matches as a prefix, so its characters are colored against a concrete
/// target. With a single answer line this always picks that line; with several
/// it lets the user type the items in any order.
fn retarget(current: &mut TypingValidator, expected: &[String], claimed: &[bool]) {
    let typed: String = current.typed().iter().map(|t| t.ch).collect();
    let candidates: Vec<&str> = expected
        .iter()
        .enumerate()
        .filter(|(i, _)| !claimed[*i])
        .map(|(_, e)| e.as_str())
        .collect();
    if let Some(rel) = best_prefix_match(&typed, &candidates) {
        current.set_expected(candidates[rel]);
    }
}

/// Builds a feedback line for a completed typing-mode line from its validator:
/// the text the user typed, the expected text, and whether it passed (a line
/// completed without a hint). `distance` is only a pass/fail flag here.
fn typing_result(v: &TypingValidator) -> FuzzyResult {
    let input: String = v.typed().iter().map(|t| t.ch).collect();
    let passed = v.passed();
    FuzzyResult {
        input,
        expected: v.expected(),
        distance: usize::from(!passed),
        passed,
    }
}

/// Byte offset of the `n`th character in `s`, or `s.len()` if `n` is at or
/// past the end. Lets a character-based caret edit a UTF-8 string safely.
fn char_byte(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map_or(s.len(), |(b, _)| b)
}

// ── AI exam (interactive TUI) ────────────────────────────────────────────────

/// The interactive exam TUI: a sibling of [`App`] that drives one
/// [`exam::Sitting`] (generate → answer one question at a time → grade →
/// results → remediate). It reuses the same background-poll loop and free-text
/// input as review; the deck's mechanical review stays in [`App`].
pub struct ExamApp {
    sitting: exam::Sitting,
    store: Store,
    /// The deck under exam (for resolving what a pass unlocks).
    deck_path: PathBuf,
    decks_dir: Option<PathBuf>,
    /// Scroll offset of the results breakdown.
    scroll: u16,
    quit: bool,
}

impl ExamApp {
    /// Starts an exam for `deck` (the caller has checked it declares a
    /// `% source:` and is drilled). Question generation spawns immediately.
    pub fn new(
        deck: Deck,
        strictness: Strictness,
        exam_cfg: ExamConfig,
        ask_cfg: AskConfig,
        store: Store,
        decks_dir: Option<PathBuf>,
    ) -> Self {
        let deck_path = deck.path.clone();
        let sitting = exam::Sitting::start(&deck, strictness, exam_cfg, ask_cfg);
        Self {
            sitting,
            store,
            deck_path,
            decks_dir,
            scroll: 0,
            quit: false,
        }
    }

    /// Runs the exam TUI until the user leaves. The store is saved (the sitting
    /// also persists "mastered" the moment a pass is graded).
    pub fn run(mut self) -> Result<()> {
        let mut terminal = ratatui::init();
        let result = self.event_loop(&mut terminal);
        ratatui::restore();
        let _ = self.store.save();
        result
    }

    fn event_loop(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
        while !self.quit {
            terminal.draw(|frame| self.draw(frame))?;
            // Poll so the spinner animates and background calls land while no
            // key is pressed.
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key(key);
            }
            self.sitting.poll(&mut self.store, time::now_ms());
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match self.sitting.phase() {
            exam::Phase::Answering => match key.code {
                KeyCode::Esc => self.quit = true,
                // Tab walks forward; on the last question it submits for grading.
                KeyCode::Tab => {
                    if self.sitting.on_last() {
                        self.sitting.submit();
                    } else {
                        self.sitting.next();
                    }
                }
                KeyCode::BackTab => self.sitting.prev(),
                KeyCode::Enter => self.sitting.push_char('\n'),
                KeyCode::Backspace if !ctrl => self.sitting.pop_char(),
                KeyCode::Char(c) if !ctrl => self.sitting.push_char(c),
                _ => {}
            },
            exam::Phase::Results => match key.code {
                KeyCode::Esc | KeyCode::Enter => self.quit = true,
                KeyCode::Char('r') if self.sitting.can_remediate() => self.sitting.remediate(),
                KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
                _ => {}
            },
            exam::Phase::Remediated => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    self.quit = true;
                }
            }
            // Generating / Grading / Remediating: only Esc leaves.
            _ => {
                if key.code == KeyCode::Esc {
                    self.quit = true;
                }
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let [header, _, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let left = format!(
            " flash {} │ exam · {}",
            env!("CARGO_PKG_VERSION"),
            self.sitting.subject()
        );
        let right = format!("{} ", exam_strictness_label(self.sitting.strictness()));
        frame.render_widget(bar(&left, &right, header.width), header);
        self.draw_footer(frame, footer);
        self.draw_body(frame, body);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let keys = match self.sitting.phase() {
            exam::Phase::Answering if self.sitting.on_last() => {
                "TAB submit for grading │ SHIFT-TAB back │ ESC quit"
            }
            exam::Phase::Answering => "TAB next │ SHIFT-TAB back │ ESC quit",
            exam::Phase::Results if self.sitting.can_remediate() => {
                "R add remediation cards │ ↑/↓ scroll │ ESC close"
            }
            exam::Phase::Results => "↑/↓ scroll │ ESC close",
            exam::Phase::Remediated => "ESC close",
            _ => "ESC cancel",
        };
        frame.render_widget(bar(&format!(" {keys}"), "", area.width), area);
    }

    fn draw_body(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();
        match self.sitting.phase() {
            exam::Phase::Generating | exam::Phase::Grading | exam::Phase::Remediating => {
                let msg = match self.sitting.phase() {
                    exam::Phase::Grading => "Grading your answers…",
                    exam::Phase::Remediating => "Writing remediation cards…",
                    _ => "Preparing your exam…",
                };
                if self.sitting.thinking() {
                    lines.push(Line::from(format!("{} {msg}", spinner())));
                } else {
                    // Not thinking in a spinner phase means the call failed; the
                    // error is shown below.
                    lines.push(Line::from("The exam helper could not complete.".dim()));
                }
            }
            exam::Phase::Answering => {
                lines.push(Line::from(
                    format!(
                        "Question {} / {}",
                        self.sitting.current_index() + 1,
                        self.sitting.total()
                    )
                    .dim(),
                ));
                lines.push(Line::default());
                if let Some(q) = self.sitting.question() {
                    lines.push(Line::from(q.prompt.clone().bold()));
                }
                lines.push(Line::default());
                // The answer so far, with a block cursor at the end.
                let answer = self.sitting.answer();
                let shown = format!("{answer}▌");
                for (i, l) in shown.split('\n').enumerate() {
                    lines.push(Line::from(if i == 0 {
                        format!("> {l}")
                    } else {
                        format!("  {l}")
                    }));
                }
                lines.push(Line::default());
                lines.push(Line::from(
                    "(type your answer — ENTER for a new line)".dim(),
                ));
            }
            exam::Phase::Results => {
                if let Some(r) = self.sitting.result() {
                    if r.passed {
                        lines.push(Line::from(
                            "PASSED — deck mastered ✓".bold().fg(Color::Green),
                        ));
                        let unlocks = self.unlocks();
                        if !unlocks.is_empty() {
                            lines.push(Line::from(
                                format!("Unlocks: {}", unlocks.join(", ")).fg(Color::Green),
                            ));
                        }
                    } else {
                        lines.push(Line::from("FAILED".bold().fg(Color::Red)));
                    }
                    lines.push(Line::default());
                    for (i, (q, g)) in self.sitting.questions().iter().zip(&r.grades).enumerate() {
                        lines.push(Line::from(format!("Q{}. {}", i + 1, q.prompt)));
                        let color = match g.verdict {
                            exam::Verdict::Pass => Color::Green,
                            exam::Verdict::Partial => Color::Yellow,
                            exam::Verdict::Fail => Color::Red,
                        };
                        lines.push(Line::from(
                            format!("  {} — {}", g.verdict.label(), g.feedback).fg(color),
                        ));
                        if g.verdict != exam::Verdict::Pass {
                            if !q.points.is_empty() {
                                lines.push(Line::from("  a complete answer covers:".dim()));
                                for p in &q.points {
                                    lines.push(Line::from(format!("    • {p}")));
                                }
                            }
                            for m in &g.missed {
                                lines.push(Line::from(format!("    ✗ {m}").fg(Color::Red)));
                            }
                        }
                        lines.push(Line::default());
                    }
                }
            }
            exam::Phase::Remediated => {
                lines.push(Line::from("Remediation cards added ✓".fg(Color::Green)));
                lines.push(Line::default());
                lines.push(Line::from("Re-drill the deck, then re-sit the exam."));
            }
        }
        if let Some(e) = self.sitting.error() {
            lines.push(Line::default());
            lines.push(Line::from(format!("error: {e}").fg(Color::Red)));
        }
        let scroll = if matches!(self.sitting.phase(), exam::Phase::Results) {
            self.scroll
        } else {
            0
        };
        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        frame.render_widget(paragraph, area);
    }

    /// Decks a pass unlocks (those that `% requires:` this one).
    fn unlocks(&self) -> Vec<String> {
        match &self.decks_dir {
            Some(dir) => deck::dependents(&self.deck_path, dir),
            None => Vec::new(),
        }
    }
}

fn exam_strictness_label(s: Strictness) -> &'static str {
    match s {
        Strictness::Strict => "strict",
        Strictness::Balanced => "balanced",
        Strictness::Lenient => "lenient",
    }
}

#[cfg(test)]
mod tests {
    use super::char_byte;

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
}
