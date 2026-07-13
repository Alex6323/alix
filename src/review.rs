//! The presentation-agnostic view of a review session's current state: the
//! client contract both the web server and the frb mobile client render. Kept
//! minimal (a flip card's front and back); modes and stats extend it later.
use serde::{Deserialize, Serialize};

/// One card as a client renders it, independent of any transport.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardView {
    pub front: String,
    pub back: Vec<String>,
}

/// The current position in a review session: the card to show (or none when
/// finished), whether the session is done, and how many cards remain.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewState {
    pub card: Option<CardView>,
    pub finished: bool,
    pub remaining: u32,
}
