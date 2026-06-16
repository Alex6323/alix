//! A spaced-repetition flashcard trainer for the terminal.
//!
//! Decks are plain-text files. On top of the basics it offers a ratatui TUI,
//! an optional local web frontend (`flash serve`), Leitner and SM-2
//! schedulers, several answer modes (flip, typing, fuzzy, multiple choice,
//! line-by-line), cloze cards, deck dependencies, an ask-Claude helper, AI
//! deck generation, and per-card review statistics.

pub mod answer;
pub mod ask;
pub mod browse;
pub mod card;
pub mod choice;
pub mod cloze;
pub mod config;
pub mod deck;
pub mod generate;
pub mod parser;
pub mod picker;
pub mod recent;
pub mod render;
pub mod scheduler;
pub mod serve;
pub mod session;
pub mod store;
pub mod time;
pub mod tui;
