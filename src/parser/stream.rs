use std::ops::Range;

use super::cloze::Seg;
use crate::inline::line_pieces;

/// The canonical maskable stream (ADR 0034): the learner-visible text of a
/// block's answer lines, one LF join, with a source map back to authored
/// positions and a matchability map. Binding, position minting, repair,
/// masking, and error counts all consume this one text.
pub(super) struct MaskableStream {
    pub text: String,
    lines: Vec<StreamLine>,
}

struct StreamLine {
    answer_index: usize,
    pieces: Vec<StreamPiece>,
}

struct StreamPiece {
    /// Visible byte range within the stream text.
    range: Range<usize>,
    matchable: bool,
    /// Per visible char: the authored byte range in the answer line that a
    /// masking splice replaces (an escaped char's range starts at its
    /// backslash; a styled piece's map excludes the markers around it).
    map: Vec<(usize, usize)>,
}

/// Image lines (their own lines, by the format rule) contribute no stream
/// text; fence delimiter lines and lines without visible text likewise. An
/// excluded line leaves NO slot in the join, so a minted `position:` never
/// counts anything the learner cannot see and a purely visual edit never
/// shifts an offset. Fenced interiors
/// are one matchable piece each, taken verbatim (no inline parsing inside
/// code); math is visible but never matchable in v1.
/// A piece ready for the join: (visible text, matchable, per-char map).
type BuiltPiece = (String, bool, Vec<(usize, usize)>);

pub(super) fn maskable_stream(answer: &[(usize, String)], parsed: &[Vec<Seg>]) -> MaskableStream {
    let mut text = String::new();
    let mut lines = Vec::new();
    let mut fence: Option<char> = None;
    let mut first = true;
    let mut include =
        |built: Vec<BuiltPiece>, index: usize, text: &mut String, lines: &mut Vec<StreamLine>| {
            if !first {
                text.push('\n');
            }
            first = false;
            let mut pieces = Vec::new();
            for (visible, matchable, map) in built {
                let range = text.len()..text.len() + visible.len();
                text.push_str(&visible);
                pieces.push(StreamPiece {
                    range,
                    matchable,
                    map,
                });
            }
            lines.push(StreamLine {
                answer_index: index,
                pieces,
            });
        };
    for (index, ((_, raw), segments)) in answer.iter().zip(parsed).enumerate() {
        if let Some(ch) = fence {
            if super::closes_fence(raw, ch) {
                fence = None;
                continue;
            }
            if raw.trim().is_empty() {
                continue;
            }
            let map = raw
                .char_indices()
                .map(|(byte, ch)| (byte, byte + ch.len_utf8()))
                .collect();
            include(vec![(raw.clone(), true, map)], index, &mut text, &mut lines);
            continue;
        }
        if let Some(ch) = super::fence_opener(raw) {
            fence = Some(ch);
            continue;
        }
        if segments.iter().any(|seg| matches!(seg, Seg::Image { .. })) {
            continue;
        }
        let holes = if segments.iter().any(|seg| matches!(seg, Seg::Hole { .. })) {
            super::cloze::hole_footprints(raw)
        } else {
            Vec::new()
        };
        let byte_of: Vec<usize> = raw
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([raw.len()])
            .collect();
        let built: Vec<BuiltPiece> = line_pieces(raw, &holes)
            .into_iter()
            .map(|piece| {
                let map = piece
                    .starts
                    .iter()
                    .zip(&piece.ends)
                    .map(|(start, end)| (byte_of[*start], byte_of[*end]))
                    .collect();
                (piece.text, !piece.math, map)
            })
            .collect();
        if built.iter().all(|(visible, ..)| visible.trim().is_empty()) {
            continue;
        }
        include(built, index, &mut text, &mut lines);
    }
    MaskableStream { text, lines }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RangeClass {
    Matchable,
    /// Inside one piece, but the piece is visible-unmatchable (math in v1).
    Math,
    /// Crosses a piece boundary: a style node edge or a line break.
    Crossing,
}

impl MaskableStream {
    /// Whether `range` (bytes into the stream text) lies wholly inside one
    /// matchable piece decides binding: the cross-node and math rules both
    /// fall out of this classification.
    pub fn classify(&self, range: &Range<usize>) -> RangeClass {
        match self.locate(range) {
            Some((_, piece)) if piece.matchable => RangeClass::Matchable,
            Some(_) => RangeClass::Math,
            None => RangeClass::Crossing,
        }
    }

    /// The answer-line index owning the stream byte `at`, by its join slot.
    pub fn line_of(&self, at: usize) -> Option<usize> {
        let slot = self.text[..at].matches('\n').count();
        self.lines.get(slot).map(|line| line.answer_index)
    }

    /// The authored location of `range`: the answer-line index and the
    /// authored byte range a masking splice replaces. None when the range
    /// crosses piece boundaries or lands outside any visible piece.
    // Production wiring lands with the masking slice; an #[expect] would be
    // unfulfilled under cfg(test), where the stream tests already use this.
    #[allow(dead_code)]
    pub fn splice(&self, range: &Range<usize>) -> Option<(usize, Range<usize>)> {
        let (line, piece) = self.locate(range)?;
        let visible = &self.text[piece.range.clone()];
        let chars: Vec<usize> = visible
            .char_indices()
            .map(|(byte, _)| piece.range.start + byte)
            .collect();
        let first = chars.iter().position(|byte| *byte == range.start)?;
        let last = chars.iter().rposition(|byte| *byte < range.end)?;
        Some((line.answer_index, piece.map[first].0..piece.map[last].1))
    }

    fn locate(&self, range: &Range<usize>) -> Option<(&StreamLine, &StreamPiece)> {
        if range.is_empty() {
            return None;
        }
        self.lines.iter().find_map(|line| {
            line.pieces
                .iter()
                .find(|piece| range.start >= piece.range.start && range.end <= piece.range.end)
                .map(|piece| (line, piece))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(lines: &[&str]) -> MaskableStream {
        let answer: Vec<(usize, String)> = lines
            .iter()
            .enumerate()
            .map(|(index, line)| (index + 1, line.to_string()))
            .collect();
        let mut lints = Vec::new();
        let parsed: Vec<Vec<Seg>> = answer
            .iter()
            .map(|(lineno, line)| {
                super::super::cloze::scan_markers(
                    line,
                    *lineno,
                    super::super::cloze::Side::Answer,
                    &mut lints,
                )
                .unwrap()
            })
            .collect();
        maskable_stream(&answer, &parsed)
    }

    #[test]
    fn plain_lines_join_with_one_lf_and_map_identically() {
        let s = stream(&["one two", "three"]);
        assert_eq!("one two\nthree", s.text);
        assert_eq!(RangeClass::Matchable, s.classify(&(0..3)));
        assert_eq!(Some((0, 0..3)), s.splice(&(0..3)));
        assert_eq!(
            Some((1, 0..5)),
            s.splice(&(8..13)),
            "the second line maps to its own bytes"
        );
    }

    #[test]
    fn styled_text_is_visible_without_markers_and_splices_to_authored_bytes() {
        let s = stream(&["**New** York"]);
        assert_eq!("New York", s.text);
        assert_eq!(
            RangeClass::Matchable,
            s.classify(&(0..3)),
            "bold contents are matchable"
        );
        assert_eq!(
            Some((0, 2..5)),
            s.splice(&(0..3)),
            "the splice addresses the authored bytes inside the markers"
        );
        assert_eq!(
            RangeClass::Crossing,
            s.classify(&(0..8)),
            "a match crossing the bold boundary crosses pieces"
        );
        assert_eq!(None, s.splice(&(0..8)));
    }

    #[test]
    fn an_escaped_chars_splice_starts_at_its_backslash() {
        let s = stream(&[r"a\*b"]);
        assert_eq!("a*b", s.text);
        assert_eq!(
            Some((0, 1..4)),
            s.splice(&(1..3)),
            "masking `*b` must take the backslash with it"
        );
    }

    #[test]
    fn image_lines_leave_no_slot_so_positions_skip_them() {
        let s = stream(&["before", "![alt](hand.png)", "after"]);
        assert_eq!("before\nafter", s.text);
        assert_eq!(
            Some((2, 0..5)),
            s.splice(&(7..12)),
            "after maps to answer line 2"
        );
    }

    #[test]
    fn fence_interiors_are_matchable_verbatim_and_delimiters_leave_no_slot() {
        let s = stream(&["```rust", "let x = 1;", "```", "prose"]);
        assert_eq!("let x = 1;\nprose", s.text);
        assert_eq!(
            RangeClass::Matchable,
            s.classify(&(0..3)),
            "fenced code contents are matchable"
        );
        assert_eq!(Some((1, 4..5)), s.splice(&(4..5)));
    }

    #[test]
    fn math_is_visible_but_never_matchable() {
        let s = stream(&["sum $x+y$ here"]);
        assert_eq!("sum x+y here", s.text);
        assert_eq!(
            RangeClass::Math,
            s.classify(&(4..7)),
            "math source is visible-unmatchable in v1"
        );
        assert_eq!(RangeClass::Matchable, s.classify(&(8..12)));
    }

    #[test]
    fn multibyte_text_maps_by_bytes_not_chars() {
        let s = stream(&["der Bär brummt"]);
        assert_eq!("der Bär brummt", s.text);
        assert_eq!(
            Some((0, 4..8)),
            s.splice(&(4..8)),
            "Bär spans four bytes and the map speaks bytes"
        );
    }

    #[test]
    fn lines_without_visible_text_leave_no_slot() {
        let s = stream(&["a", "", "   ", "b"]);
        assert_eq!(
            "a\nb", s.text,
            "an author counting positions never counts a line they cannot see"
        );
        assert_eq!(Some((3, 0..1)), s.splice(&(2..3)));
    }
}
