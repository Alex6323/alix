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
#[cfg(feature = "full")]
pub mod ask;
pub mod assemble;
pub mod augment;
#[cfg(feature = "full")]
pub mod augment_ai;
#[cfg(feature = "full")]
pub mod backend;
#[cfg(feature = "full")]
pub mod calibrate;
pub mod card;
pub mod choice;
pub mod cloze;
pub mod config;
pub mod deck;
pub mod depth;
#[cfg(feature = "full")]
pub mod doctor;
#[cfg(feature = "full")]
pub mod exam;
#[cfg(feature = "full")]
pub mod explore;
#[cfg(feature = "full")]
pub mod generate;
#[cfg(feature = "full")]
pub mod icon;
#[cfg(feature = "full")]
pub mod import;
#[cfg(feature = "full")]
pub mod library;
pub mod listing;
pub mod parser;
#[cfg(feature = "full")]
pub mod picker;
#[cfg(feature = "full")]
pub mod preflight;
#[cfg(feature = "full")]
pub mod qr;
#[cfg(feature = "full")]
pub mod recent;
pub mod render;
pub mod review;
pub mod scheduler;
#[cfg(feature = "full")]
pub mod serve;
pub mod session;
#[cfg(feature = "full")]
pub mod share;
pub mod store;
pub mod time;
#[cfg(feature = "full")]
pub mod title;
pub mod trace;
#[cfg(feature = "full")]
pub mod trace_ai;
pub mod workspace;

// Only the AI-facing modules (ask, exam, generate, ...) use these fake-CLI
// helpers, and all of those are gated behind `full`, so the module itself
// only needs to exist for a `full` test build.
#[cfg(all(test, feature = "full"))]
mod testutil;
