use std::{path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::{answer::Input, depth::Reveal, token};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
pub enum Direction {
    #[default]
    Forward,
    Reverse,
    Both,
}

impl Direction {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "forward" => Some(Self::Forward),
            "reverse" => Some(Self::Reverse),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

/// One member of a region group: its stamp (None while unstamped) and what
/// its mask hides. Provenance stays member-level so the future MC-family
/// spec can see shape, never a flattened list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupMember {
    pub stamp: Option<Arc<str>>,
    pub hidden: Option<String>,
    /// The member's directive line: the removal address of exactly this
    /// region, never a card-block boundary.
    pub line: usize,
}

/// What a region card asks (ADR 0034): one region, or a named group asking
/// every member. An unstamped region (or a group with one) has no usable id
/// and its card is excluded at the session boundary like any token-less card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionSlot {
    Single {
        stamp: Option<Arc<str>>,
        hidden: Option<String>,
        line: usize,
        name: Option<String>,
    },
    Group {
        name: String,
        hash: Option<Arc<str>>,
        members: Vec<GroupMember>,
    },
}

impl RegionSlot {
    /// The directive line(s) this card owns in the file: what removal
    /// deletes instead of a card block.
    pub fn directive_lines(&self) -> Vec<usize> {
        match self {
            RegionSlot::Single { line, .. } => vec![*line],
            RegionSlot::Group { members, .. } => members.iter().map(|m| m.line).collect(),
        }
    }

    pub fn first_line(&self) -> usize {
        self.directive_lines().into_iter().min().unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardImage {
    pub src: PathBuf,
    pub alt: Option<String>,
    /// Regions bound to this media element (ADR 0034), in file order.
    pub regions: Vec<crate::parser::region::RawRegion>,
    /// The one optional viewport onto this media element.
    pub crop: Option<crate::parser::region::RawCrop>,
}

/// A frozen diagram's stamp: the fence's fingerprint plus the two
/// content-addressed artifacts (raster and geometry) minted at freeze time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramStamp {
    pub fingerprint: String,
    pub asset: String,
    pub geometry: String,
    pub line: usize,
}

/// One closed mermaid fence in the block's answer, in block order, captured
/// at parse time because a region card's context holds the fence MASKED:
/// neither the unmasked fingerprint nor a span's authored offsets can be
/// recovered from the displayed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerFence {
    /// Fingerprint of the unmasked LF-normalized interior.
    pub fingerprint: String,
    /// The unmasked LF-normalized interior itself, shared across the
    /// block's synthesized cards: consumers validate every persisted label
    /// range (bounds, UTF-8 boundaries, overlap) against these bytes
    /// before any slice, which the masked context can never supply.
    pub interior: std::sync::Arc<str>,
    /// Every bound span splicing into this fence's interior.
    pub spans: Vec<AnswerFenceSpan>,
}

/// One span's bound range inside its fence, in bytes of the LF-normalized
/// interior; `line` is the directive line, the same identity `RegionSlot`
/// and `masked_context` match spans by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerFenceSpan {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// A stamp whose frozen pair checked out at deck load: both objects
/// present, the geometry parses, and it names the stamp's raster. Only
/// resolution this shallow runs on the load path; doctor re-hashes bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDiagram {
    pub fingerprint: String,
    pub png: std::path::PathBuf,
    pub geometry: crate::diagram::DiagramGeometry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCitation {
    /// The `at:` field: the real source path and lines, never an asset name.
    pub locator: String,
    pub fingerprint: Option<u64>,
    /// The `asset:` field: the frozen `sha256-<hex>.<ext>` object name,
    /// present only on a frozen citation.
    pub asset: Option<String>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub subject: Arc<str>,
    /// The owning deck's stable id; empty when the deck has none
    /// (uninitialized deck, or a card built outside deck context).
    pub deck_id: Arc<str>,
    /// The owning deck's folded link-definition table, shared by every card
    /// of the deck; empty outside deck context. Labels must never resolve
    /// across decks, which is why the table rides the card, not the session.
    pub definitions: Arc<crate::inline::LinkDefinitions>,
    pub front: String,
    /// The card's section: its `# ` heading and that section's prose. Shown
    /// only on demand and never part of identity, so re-filing a card
    /// changes what it explains, never what it is.
    pub section_context: Vec<String>,
    pub context: Vec<String>,
    /// Whether `context` is the question (a cloze sentence, which leads) or a
    /// label for the front (a table title, which steps back).
    pub context_leads: bool,
    pub back: Vec<String>,
    pub notes: Vec<Note>,
    pub line: usize,
    /// The authored block's first line: a heading card's own line, a table
    /// row's table line. Every review unit one block expands to shares it.
    pub block_line: usize,
    /// The `block_line` of the `## `/`### ` block this sub-card hangs under.
    /// `None` for a top-level card, so gating never asks about it.
    pub parent_block: Option<usize>,
    pub reveal: Option<Reveal>,
    pub input: Option<Input>,
    pub direction: Option<Direction>,
    pub images: Vec<CardImage>,
    pub images_back: Vec<CardImage>,
    pub citations: Vec<SourceCitation>,
    pub diagrams: Vec<DiagramStamp>,
    pub resolved_diagrams: Vec<ResolvedDiagram>,
    pub givens: Vec<String>,
    pub display_back: Option<Vec<String>>,
    pub token: Option<Arc<str>>,
    pub row: Option<Arc<str>>,
    /// Resolved table-over-deck at parse time; None means the default (on).
    pub sampling: Option<bool>,
    pub reversed: bool,
    pub content_fingerprint: u64,
    /// The block-level dedup key (front + cover-masked raw answer lines):
    /// every card of one authored block shares it, so remediation and
    /// personal-sidecar dedup can address the block while
    /// `content_fingerprint` stays the card's own effective question.
    pub block_fingerprint: u64,
    pub authored_distractors: Vec<String>,
    /// `choices: multiple`: every line of `back` is a correct option and the
    /// reviewer selects all that apply; false means one correct answer.
    pub multiple_choice: bool,
    /// `span`-shaped regions bound to the answer block (ADR 0034), in file
    /// order; geometric regions ride their media element in `images`.
    pub span_regions: Vec<crate::parser::region::RawRegion>,
    /// Set on a synthesized region card: which region(s) this card asks.
    pub region: Option<RegionSlot>,
    pub answer_fences: Vec<AnswerFence>,
}

/// GitHub's five alert badges, the closed set whose presence on a
/// blockquote's first line makes it a note rather than a quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Badge {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl Badge {
    /// The exact GitHub spellings; any other casing is a quote there, so
    /// accepting one here would diverge from the rendering alix mirrors.
    pub fn parse(line: &str) -> Option<Self> {
        match line {
            "[!NOTE]" => Some(Self::Note),
            "[!TIP]" => Some(Self::Tip),
            "[!IMPORTANT]" => Some(Self::Important),
            "[!WARNING]" => Some(Self::Warning),
            "[!CAUTION]" => Some(Self::Caution),
            _ => None,
        }
    }

    /// True for a first blockquote line shaped like a badge that is not one
    /// of the five, which stays a quote and is worth a doctor warning.
    pub fn is_misspelled(line: &str) -> bool {
        line.starts_with("[!") && line.contains(']') && Self::parse(line).is_none()
    }
}

/// The badge is absent for a note no blockquote opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub badge: Option<Badge>,
    pub body: String,
}

impl Note {
    pub fn bare(body: String) -> Self {
        Self { badge: None, body }
    }
}

/// Which answer a client is served. A typed check grades exact text, so it
/// gets the deck's authored words: the `format` augment reshapes how a card
/// is SHOWN, and a typing surface shows blank fields, so a reshape the
/// learner never saw must never become what they have to reproduce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerSpace {
    Displayed,
    Authored,
}

impl Card {
    pub fn plain(
        subject: Arc<str>,
        front: String,
        back: Vec<String>,
        notes: Vec<Note>,
        line: usize,
    ) -> Self {
        // The parser overrides this for cloze sub-cards with a shared block-level fingerprint; this
        // default fits every other card.
        let content_fingerprint = crate::parser::content_fingerprint(&front, &back);
        let block_fingerprint = content_fingerprint;
        Self {
            subject,
            deck_id: Arc::from(""),
            definitions: Arc::default(),
            front,
            section_context: Vec::new(),
            context: Vec::new(),
            context_leads: false,
            back,
            notes,
            line,
            block_line: line,
            parent_block: None,
            reveal: None,
            input: None,
            direction: None,
            images: Vec::new(),
            images_back: Vec::new(),
            citations: Vec::new(),
            diagrams: Vec::new(),
            resolved_diagrams: Vec::new(),
            givens: Vec::new(),
            display_back: None,
            token: None,
            row: None,
            sampling: None,
            reversed: false,
            content_fingerprint,
            block_fingerprint,
            authored_distractors: Vec::new(),
            multiple_choice: false,
            span_regions: Vec::new(),
            region: None,
            answer_fences: Vec::new(),
        }
    }

    pub fn back_for_display(&self) -> &[String] {
        self.display_back.as_deref().unwrap_or(&self.back)
    }

    pub fn answer_lines(&self, space: AnswerSpace) -> &[String] {
        match space {
            AnswerSpace::Displayed => self.back_for_display(),
            AnswerSpace::Authored => &self.back,
        }
    }

    pub fn reversed(&self) -> Card {
        let mut card = Card::plain(
            Arc::clone(&self.subject),
            self.back.join("\n"),
            vec![self.front.clone()],
            self.notes.clone(),
            self.line,
        );
        card.deck_id = Arc::clone(&self.deck_id);
        card.definitions = Arc::clone(&self.definitions);
        // Built after the parser has finished, so the builder's stamp never
        // reaches it: the reverse half must carry the section itself.
        card.section_context = self.section_context.clone();
        card.block_line = self.block_line;
        card.parent_block = self.parent_block;
        card.reveal = self.reveal;
        card.input = self.input;
        card.images = self.images_back.clone();
        card.images_back = self.images.clone();
        card.citations = self.citations.clone();
        card.diagrams = self.diagrams.clone();
        card.resolved_diagrams = self.resolved_diagrams.clone();
        // The reversed half keeps the same token so id() can compose the "-r" suffix from it.
        card.token = self.token.clone();
        card.row = self.row.clone();
        card.sampling = self.sampling;
        card.context = self.context.clone();
        card.context_leads = self.context_leads;
        card.reversed = true;
        // Reuses the forward card's fingerprint instead of recomputing over swapped sides: one
        // authored card is one content unit.
        card.content_fingerprint = self.content_fingerprint;
        card.block_fingerprint = self.block_fingerprint;
        card
    }

    /// A blank-derived study card (built from `blank:` regions), graded and
    /// rendered against its hidden spans rather than its answer lines.
    pub fn is_blank_card(&self) -> bool {
        self.region.is_some()
    }

    /// The choices gate distinguishes the two blank kinds: every member
    /// masking answer text (a `span`) qualifies; one image region disqualifies.
    pub fn is_text_blank_card(&self) -> bool {
        let Some(slot) = &self.region else {
            return false;
        };
        slot.directive_lines()
            .iter()
            .all(|line| self.span_regions.iter().any(|region| region.line == *line))
    }

    pub fn id(&self) -> Option<String> {
        let token = self.token.as_deref()?;
        let region = match &self.region {
            None => None,
            Some(RegionSlot::Single { stamp, .. }) => {
                Some(token::RegionRef::Single(stamp.as_deref()?))
            }
            Some(RegionSlot::Group { hash, .. }) => Some(token::RegionRef::Group(hash.as_deref()?)),
        };
        Some(token::card_id(
            token,
            self.row.as_deref(),
            self.reversed,
            region,
        ))
    }

    /// What a display reshape was generated from: the card's content plus the
    /// notes it was asked to rewrite. `content_fingerprint` is scheduling
    /// identity and deliberately excludes notes, so a note-only edit has to
    /// move this instead.
    pub fn format_fingerprint(&self) -> u64 {
        let mut input = String::new();
        for note in &self.notes {
            input.push_str(match note.badge {
                Some(Badge::Note) => "note",
                Some(Badge::Tip) => "tip",
                Some(Badge::Important) => "important",
                Some(Badge::Warning) => "warning",
                Some(Badge::Caution) => "caution",
                None => "",
            });
            input.push('\u{1f}');
            input.push_str(&note.body);
            input.push('\u{1e}');
        }
        crate::parser::mix_fingerprint(self.content_fingerprint, &input)
    }

    /// Every note's body as one block, for the payloads that carry a card as
    /// plain text.
    pub fn notes_text(&self) -> Option<String> {
        let bodies: Vec<&str> = self.notes.iter().map(|note| note.body.as_str()).collect();
        (!bodies.is_empty()).then(|| bodies.join("\n\n"))
    }

    /// A note alix adds rather than the author: it stands beside theirs
    /// instead of joining a block their badge speaks for.
    pub fn append_note(&mut self, notes: &[String]) {
        if notes.is_empty() {
            return;
        }
        self.notes.push(Note::bare(notes.join("\n")));
    }
}

#[cfg(test)]
impl Card {
    /// Panics on a second note, so a test meaning "the only note" says so.
    pub(crate) fn only_note(&self) -> Option<&str> {
        match self.notes.as_slice() {
            [] => None,
            [note] => Some(&note.body),
            several => panic!("the card carries {} notes: {several:?}", several.len()),
        }
    }
}

/// Base ids of blank-template blocks (ADR 0034): reserved live identities
/// while no plain card exists, so every known-id inventory must include them.
pub fn dormant_base_ids(cards: &[Card]) -> impl Iterator<Item = String> + '_ {
    cards
        .iter()
        .filter(|card| card.region.is_some())
        .filter_map(|card| card.token.as_deref().map(str::to_string))
}

impl Eq for Card {}
// Equality is (token, reversed, region identity) only; unstamped cards (token: None) compare
// equal, which is harmless since the session/store boundary excludes them first. The region
// discriminant matters: a parent and its region cards share the token.
impl PartialEq for Card {
    fn eq(&self, other: &Self) -> bool {
        self.token == other.token
            && self.reversed == other.reversed
            && region_identity(&self.region) == region_identity(&other.region)
    }
}

/// The identity-bearing half of a region slot: which region(s), never where
/// their directives sit in the file.
fn region_identity(slot: &Option<RegionSlot>) -> Option<(bool, Option<Arc<str>>)> {
    slot.as_ref().map(|slot| match slot {
        RegionSlot::Single { stamp, .. } => (false, stamp.clone()),
        RegionSlot::Group { hash, .. } => (true, hash.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(subject: &str, front: &str, back: &[&str], note: Option<&str>) -> Card {
        Card::plain(
            Arc::from(subject),
            front.to_string(),
            back.iter().map(|s| s.to_string()).collect(),
            Vec::from_iter(note.map(|s| Note::bare(s.to_string()))),
            1,
        )
    }

    fn stamped(subject: &str, front: &str, back: &[&str], note: Option<&str>, token: &str) -> Card {
        let mut c = card(subject, front, back, note);
        c.token = Some(Arc::from(token));
        c
    }

    #[test]
    fn a_parent_and_its_region_cards_are_never_equal() {
        let parent = stamped("s", "f", &["b"], None, "card-tok0");
        let mut single = parent.clone();
        single.region = Some(RegionSlot::Single {
            stamp: Some(Arc::from("a1b2c3")),
            hidden: None,
            line: 3,
            name: None,
        });
        let mut other_single = parent.clone();
        other_single.region = Some(RegionSlot::Single {
            stamp: Some(Arc::from("d4e5f6")),
            hidden: None,
            line: 4,
            name: None,
        });
        let mut group = parent.clone();
        group.region = Some(RegionSlot::Group {
            name: "g".into(),
            hash: Some(Arc::from("chsbz14b1a30x")),
            members: Vec::new(),
        });
        assert_ne!(parent, single, "the shared token must not conflate them");
        assert_ne!(single, other_single, "two singles differ by stamp");
        assert_ne!(single, group, "a single is not a group");
        let mut moved = single.clone();
        if let Some(RegionSlot::Single { line, .. }) = &mut moved.region {
            *line = 9;
        }
        assert_eq!(single, moved, "a directive's file position is not identity");
    }

    #[test]
    fn an_unstamped_cards_id_is_none() {
        let c = card("subject1", "hello", &["world"], None);
        assert_eq!(None, c.id());
    }

    #[test]
    fn a_stamped_cards_id_is_its_token_verbatim() {
        let c = stamped("s", "f", &["b"], None, "9w2c7xkq");
        assert_eq!(Some("9w2c7xkq".to_string()), c.id());
    }

    #[test]
    fn sub_ids_carry_the_reversed_suffix() {
        let mut c = stamped("s", "f", &["b"], None, "q1");
        c.reversed = true;
        assert_eq!(Some("q1-r".to_string()), c.id());
    }

    #[test]
    fn distinct_tokens_yield_distinct_ids() {
        let a = stamped("s", "f", &["b"], None, "q1");
        let b = stamped("s", "f", &["b"], None, "q2");
        let a2 = stamped("s", "different front", &["different back"], None, "q1");
        assert_ne!(a.id(), b.id());
        assert_eq!(a.id(), a2.id());
    }

    #[test]
    fn id_ignores_front_and_note() {
        let a = stamped("subject1", "hello", &["world"], None, "q1");
        let b = stamped("subject1", "hi there", &["world"], Some("a note"), "q1");
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn equality_is_exactly_token_region_and_direction() {
        let base = stamped("deck", "front", &["back"], None, "tok");
        let mut presentation_only = base.clone();
        presentation_only.front = "different".to_string();
        assert_eq!(base, presentation_only);

        let mut token = base.clone();
        token.token = Some("other".into());
        assert_ne!(base, token);
        let mut reversed = base.clone();
        reversed.reversed = true;
        assert_ne!(base, reversed);
    }

    #[test]
    fn append_note_stacks_one_note_per_call() {
        let mut c = card("d.md", "front", &["back"], None);
        c.append_note(&[]);
        assert!(c.notes.is_empty(), "nothing to append adds no note");
        c.append_note(&["first".to_string()]);
        c.append_note(&["second".to_string(), "third".to_string()]);
        assert_eq!(
            vec![
                Note::bare("first".to_string()),
                Note::bare("second\nthird".to_string()),
            ],
            c.notes,
            "each call adds one bare note, and the lines of one call join"
        );
    }

    #[test]
    fn an_appended_note_stands_beside_the_badged_one() {
        let mut c = card("d.md", "front", &["back"], None);
        c.notes.push(Note {
            badge: Some(Badge::Warning),
            body: "authored".to_string(),
        });
        c.append_note(&["from the sidecar".to_string()]);
        assert_eq!(
            vec![
                Note {
                    badge: Some(Badge::Warning),
                    body: "authored".to_string(),
                },
                Note::bare("from the sidecar".to_string()),
            ],
            c.notes,
            "an appended note stands beside the authored one rather than joining a \
             block its badge speaks for"
        );
    }

    #[test]
    fn the_format_fingerprint_separates_a_badge_from_a_body() {
        let stack = |notes: Vec<Note>| {
            let mut c = card("d.md", "front", &["back"], None);
            c.notes = notes;
            c.format_fingerprint()
        };
        let badged = stack(vec![Note {
            badge: Some(Badge::Tip),
            body: "x".to_string(),
        }]);
        let bare = stack(vec![Note::bare("tipx".to_string())]);
        assert_ne!(
            badged, bare,
            "a badge and a body must not run together, or a stale reshape of one \
             stack applies to a different one"
        );
        let split = stack(vec![
            Note::bare("one".to_string()),
            Note::bare("two".to_string()),
        ]);
        let joined = stack(vec![Note::bare("one\ntwo".to_string())]);
        assert_ne!(
            split, joined,
            "two notes and one note carrying both lines are different authored input"
        );
        assert_eq!(
            stack(Vec::new()),
            stack(Vec::new()),
            "a card with no notes still has a stable format fingerprint"
        );
        let two = stack(vec![
            Note {
                badge: Some(Badge::Tip),
                body: "a".to_string(),
            },
            Note::bare("b".to_string()),
        ]);
        let one = stack(vec![Note {
            badge: Some(Badge::Tip),
            body: "a\u{1f}b".to_string(),
        }]);
        assert_ne!(
            two, one,
            "nobody types a unit separator, but without the record separator these \
             two stacks hash the same, and the encoding has to be unambiguous rather \
             than unambiguous for realistic input"
        );
    }

    #[test]
    fn every_note_reaches_the_text_payloads_and_the_projection() {
        let mut c = card("d.md", "front", &["back"], Some("first"));
        c.notes.push(Note {
            badge: Some(Badge::Caution),
            body: "second".to_string(),
        });
        assert_eq!(
            Some("first\n\nsecond".to_string()),
            c.notes_text(),
            "a card carried as plain text carries every note, blank line between"
        );
        let views = crate::render::note_views(&c);
        assert_eq!(
            vec![None, Some(Badge::Caution)],
            views.iter().map(|view| view.badge).collect::<Vec<_>>(),
            "the projection is one view per note, each with its own badge"
        );
    }

    #[test]
    fn id_ignores_reveal() {
        let mut a = stamped("subject1", "hello", &["world"], None, "q1");
        let mut b = stamped("subject1", "hello", &["world"], None, "q1");
        a.reveal = Some(Reveal::Flip);
        b.reveal = Some(Reveal::Line);
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn reversed_swaps_sides_keeps_note_and_line() {
        let mut fwd = stamped(
            "vocab.md",
            "purported",
            &["angeblich"],
            Some("a note"),
            "q1",
        );
        fwd.reveal = Some(Reveal::Line);
        let rev = fwd.reversed();
        assert_eq!("angeblich", rev.front);
        assert_eq!(vec!["purported"], rev.back);
        assert_eq!(fwd.notes, rev.notes);
        assert_eq!(fwd.line, rev.line);
        assert_eq!(fwd.reveal, rev.reveal);
        assert_ne!(fwd.id(), rev.id());
        assert_eq!(Some("q1".to_string()), fwd.id());
        assert_eq!(Some("q1-r".to_string()), rev.id());
        assert!(!fwd.reversed);
        assert!(rev.reversed);
        assert_eq!(fwd.token, rev.token);
    }

    #[test]
    fn reversed_keeps_the_owning_decks_id() {
        let mut fwd = stamped("vocab.md", "purported", &["angeblich"], None, "q1");
        fwd.deck_id = Arc::from("a-deck-token");
        let rev = fwd.reversed();
        assert_eq!(fwd.deck_id, rev.deck_id);
    }

    #[test]
    fn reversed_swaps_image_sides() {
        let mut fwd = card("g.md", "name this chord", &["G major"], None);
        fwd.images_back = vec![CardImage {
            src: PathBuf::from("/tabs/g.png"),
            alt: None,
            regions: Vec::new(),
            crop: None,
        }];
        let rev = fwd.reversed();
        assert_eq!(
            vec![PathBuf::from("/tabs/g.png")],
            rev.images.iter().map(|i| i.src.clone()).collect::<Vec<_>>()
        );
        assert!(rev.images_back.is_empty());
    }

    #[test]
    fn id_ignores_image() {
        let mut a = stamped("s", "f", &["b"], None, "q1");
        let b = stamped("s", "f", &["b"], None, "q1");
        a.images = vec![CardImage {
            src: PathBuf::from("/imgs/a.png"),
            alt: None,
            regions: Vec::new(),
            crop: None,
        }];
        a.citations.push(SourceCitation {
            locator: "card.rs:1-9".to_string(),
            fingerprint: Some(42),
            asset: None,
            line: 4,
        });
        a.givens = vec!["state — the parser position".to_string()];
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn display_back_overrides_render_but_not_identity() {
        let mut c = stamped("d.md", "f", &["Chain, Version"], None, "q1");
        let before = c.id();
        c.display_back = Some(vec!["Protocol: Chain".into(), "Version".into()]);
        assert_eq!(c.back_for_display(), ["Protocol: Chain", "Version"]);
        assert_eq!(c.id(), before);
    }

    #[test]
    fn input_does_not_affect_card_identity() {
        let mut a = stamped("d.md", "front", &["the answer"], None, "q1");
        let mut b = stamped("d.md", "front", &["the answer"], None, "q1");
        a.input = Some(Input::Draw);
        b.input = None;
        assert_eq!(a.id(), b.id());
    }
}

#[cfg(all(test, feature = "full"))]
mod clap_parity {
    use clap::ValueEnum;

    use super::*;

    #[test]
    fn parse_matches_the_clap_value_names() {
        for variant in Direction::value_variants() {
            let name = variant.to_possible_value().expect("a value name");
            assert_eq!(
                Some(*variant),
                Direction::parse(name.get_name()),
                "{name:?}"
            );
        }
        assert_eq!(None, Direction::parse("no-such-value"));
    }
}
