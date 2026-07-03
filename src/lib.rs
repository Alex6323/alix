//! Your personal AI tutor — built for understanding, not just remembering.
//!
//! Decks are plain-text files. On top of the flashcard basics it offers a
//! ratatui TUI, an optional local web frontend (`alix serve`), Leitner and
//! SM-2 schedulers, several answer modes (flip, typing, fuzzy, multiple choice,
//! line-by-line, explain), cloze and dual-direction cards, deck dependencies,
//! and per-card review statistics. The configured model CLI is woven in: a
//! tutor on any card, AI deck generation, and an AI exam (`alix exam`) that
//! gates progression on verified understanding.

pub mod answer;
pub mod ask;
pub mod augment;
pub mod backend;
pub mod browse;
pub mod card;
pub mod choice;
pub mod cloze;
pub mod config;
pub mod deck;
pub mod exam;
pub mod explore;
pub mod generate;
pub mod icon;
pub mod import;
pub mod parser;
pub mod picker;
pub mod preflight;
pub mod recent;
pub mod render;
pub mod scheduler;
pub mod serve;
pub mod session;
pub mod store;
pub mod time;
pub mod title;
pub mod trace;
pub mod tui;
pub mod workspace;

#[cfg(test)]
mod testutil;
