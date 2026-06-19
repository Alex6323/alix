//! An AI-augmented spaced-repetition learning tool for the terminal and the
//! web.
//!
//! Decks are plain-text files. On top of the flashcard basics it offers a
//! ratatui TUI, an optional local web frontend (`flash serve`), Leitner and
//! SM-2 schedulers, several answer modes (flip, typing, fuzzy, multiple choice,
//! line-by-line, explain), cloze and dual-direction cards, deck dependencies,
//! and per-card review statistics. Claude is woven in: an ask-Claude tutor, AI
//! deck generation, and an AI exam (`flash exam`) that gates progression on
//! verified understanding.

pub mod answer;
pub mod ask;
pub mod browse;
pub mod card;
pub mod choice;
pub mod cloze;
pub mod config;
pub mod deck;
pub mod exam;
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
pub mod workspace;
