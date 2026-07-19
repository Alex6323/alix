// Enables `#[coverage(off)]` under nightly coverage builds, used on a
// handful of functions a deterministic test can't meaningfully drive.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
// Raised for the contract suite's widest `json!` snapshot (a `DeckListDto`
// nesting many-keyed `DeckItemDto` rows), which exceeds the default limit.
#![recursion_limit = "256"]

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

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
pub mod config;
pub mod deck;
pub mod dedup;
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
pub mod l1;
pub mod library;
pub mod listing;
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
pub mod stamp;
pub mod store;
pub mod time;
#[cfg(feature = "full")]
pub mod title;
pub mod token;
pub mod trace;
#[cfg(feature = "full")]
pub mod trace_ai;
pub mod tutorial;
pub mod txt_compat;
pub mod workspace;

// Only the AI-facing modules use these fake-CLI helpers, and they're all
// gated behind `full`.
#[cfg(all(test, feature = "full"))]
mod testutil;
