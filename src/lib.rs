//! A learning tool built for understanding, not just remembering.
//!
//! Decks are plain-text files. On top of the flashcard basics it offers a
//! local web frontend, the FSRS
//! scheduler (via `rs-fsrs`), several answer modes (flip, typing, typeline,
//! multiple choice, line-by-line, explain), cloze and dual-direction cards,
//! deck dependencies, and per-card review statistics. The configured model CLI
//! is woven in: a tutor on any card, AI deck generation, and an AI exam
//! (`alix exam`) that gates progression on verified understanding.

// Enables `#[coverage(off)]` under `cargo +nightly llvm-cov`, which sets
// `cfg(coverage_nightly)`. Used sparingly on a handful of functions a
// deterministic test can't meaningfully drive (a live OS route lookup,
// print-only QR output, a two-call AI workspace build) — see each site's
// one-line reason.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
// The contract suite's widest `json!` snapshot (`decklistdto_wire_shape`,
// nesting a many-keyed `DeckItemDto` inside a `DeckListDto`) exceeds the
// default macro recursion limit once a row carries this many fields.
#![recursion_limit = "256"]

pub mod answer;
pub mod ask;
pub mod assemble;
pub mod augment;
pub mod backend;
pub mod card;
pub mod choice;
pub mod cloze;
pub mod config;
pub mod deck;
pub mod depth;
pub mod doctor;
pub mod exam;
pub mod explore;
pub mod generate;
pub mod icon;
pub mod import;
pub mod library;
pub mod parser;
pub mod picker;
pub mod preflight;
pub mod qr;
pub mod recent;
pub mod render;
pub mod scheduler;
pub mod serve;
pub mod session;
pub mod share;
pub mod store;
pub mod time;
pub mod title;
pub mod trace;
pub mod trace_ai;
pub mod workspace;

#[cfg(test)]
mod testutil;
