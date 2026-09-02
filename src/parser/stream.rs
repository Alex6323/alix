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
/// code); math is one visible piece, typed for the structural-unit policy.
/// A piece ready for the join: (visible text, matchable, per-char map).
type BuiltPiece = (String, bool, Vec<(usize, usize)>);

pub(super) fn maskable_stream(answer: &[(usize, String)], parsed: &[Vec<Seg>]) -> MaskableStream {
    let mut text = String::new();
    let mut lines = Vec::new();
    let mut fence: Option<(char, usize)> = None;
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
        if let Some((ch, open)) = fence {
            if super::closes_fence(raw, ch, open) {
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
        if let Some(opened) = super::fence_opener(raw) {
            fence = Some(opened);
            continue;
        }
        if segments.iter().any(|seg| matches!(seg, Seg::Image { .. })) {
            continue;
        }
        let byte_of: Vec<usize> = raw
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([raw.len()])
            .collect();
        let built: Vec<BuiltPiece> = line_pieces(raw)
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
    /// Inside one math piece: bindable only as a complete structural unit.
    Math,
    /// Crosses a piece boundary: a style node edge or a line break.
    Crossing,
}

impl MaskableStream {
    /// Whether `range` (bytes into the stream text) lies wholly inside one
    /// piece decides binding: prose binds directly, math adds the
    /// structural-unit policy, and a crossing range never binds.
    pub fn classify(&self, range: &Range<usize>) -> RangeClass {
        match self.locate(range) {
            Some((_, piece)) if piece.matchable => RangeClass::Matchable,
            Some(_) => RangeClass::Math,
            None => RangeClass::Crossing,
        }
    }

    /// The 1-based grapheme index of `byte` in the stream text, the
    /// coordinate system of minted `position:` anchors.
    pub fn grapheme_position(&self, byte: usize) -> u32 {
        use unicode_segmentation::UnicodeSegmentation;
        self.text[..byte].graphemes(true).count() as u32 + 1
    }

    /// Whether both endpoints of `range` are grapheme boundaries of the
    /// stream text: anything else cannot be represented by the `position:`
    /// coordinate system and never binds.
    pub fn grapheme_bounded(&self, range: &Range<usize>) -> bool {
        use unicode_segmentation::GraphemeCursor;
        [range.start, range.end].iter().all(|&at| {
            GraphemeCursor::new(at, self.text.len(), true)
                .is_boundary(&self.text, 0)
                .unwrap_or(false)
        })
    }

    /// The byte offset where the 1-based grapheme `position` starts, when it
    /// is in bounds: the reverse of `grapheme_position`.
    pub fn grapheme_byte(&self, position: u32) -> Option<usize> {
        use unicode_segmentation::UnicodeSegmentation;
        (position > 0)
            .then(|| self.text.grapheme_indices(true).nth(position as usize - 1))
            .flatten()
            .map(|(byte, _)| byte)
    }

    /// The stream-text bounds of the math piece containing `range`, when the
    /// range lies wholly inside one non-matchable (math) piece.
    pub fn math_piece(&self, range: &Range<usize>) -> Option<Range<usize>> {
        match self.locate(range) {
            Some((_, piece)) if !piece.matchable => Some(piece.range.clone()),
            _ => None,
        }
    }

    /// Whether `range`'s word edges are bounded as the learner sees them: a
    /// neighbor within the same piece must be non-alphanumeric, and a piece
    /// edge (a hole gap, a link, a style node, a line break) is itself a
    /// boundary, because the learner sees a mask or a rendering seam there.
    pub fn word_bounded(&self, range: &Range<usize>) -> bool {
        let before = match self.locate(&(range.start..range.start + 1)) {
            Some((_, piece)) if range.start > piece.range.start => !self.text
                [piece.range.start..range.start]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric),
            _ => true,
        };
        let after = match self.locate(&(range.end.saturating_sub(1)..range.end)) {
            Some((_, piece)) if range.end < piece.range.end => !self.text
                [range.end..piece.range.end]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric),
            _ => true,
        };
        before && after
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
    use proptest::prelude::*;

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
                super::super::cloze::scan_markers(line, *lineno, &mut lints).unwrap()
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
    fn a_shorter_delimiter_inside_a_longer_fence_is_matchable_content() {
        let s = stream(&["````", "inner", "```", "also", "````"]);
        assert_eq!("inner\n```\nalso", s.text);
    }

    #[test]
    fn math_is_visible_and_typed_for_the_unit_policy() {
        let s = stream(&["sum $x+y$ here"]);
        assert_eq!("sum x+y here", s.text);
        assert_eq!(
            RangeClass::Math,
            s.classify(&(4..7)),
            "math source is typed so binding applies the structural-unit policy"
        );
        assert_eq!(Some(4..7), s.math_piece(&(4..5)));
        assert_eq!(None, s.math_piece(&(0..3)), "prose has no math piece");
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

    // A dense two-letter alphabet with structural tokens, so overlapping
    // occurrences straddling style edges are COMMON, not needle-rare: the
    // differential property must be able to reach the shadowing shapes it
    // guards against.
    fn arbitrary_lines() -> impl Strategy<Value = Vec<String>> {
        let token = prop_oneof![
            Just("a".to_string()),
            Just("b".to_string()),
            Just("ab".to_string()),
            Just("ba".to_string()),
            Just(" ".to_string()),
            Just("\\blank{ab}".to_string()),
            Just("\\blank{a}".to_string()),
            Just("**a**".to_string()),
            Just("**ab**".to_string()),
            Just("`ab`".to_string()),
            Just("[a](b)".to_string()),
            Just("$ab$".to_string()),
            Just("ä".to_string()),
        ];
        proptest::collection::vec(
            proptest::collection::vec(token, 0..8).prop_map(|tokens| tokens.concat()),
            1..4,
        )
    }

    type ParsedBlock = (Vec<(usize, String)>, Vec<Vec<Seg>>);

    fn parsed_or_skip(lines: &[String]) -> Option<ParsedBlock> {
        let answer: Vec<(usize, String)> = lines
            .iter()
            .enumerate()
            .map(|(index, line)| (index + 1, line.clone()))
            .collect();
        let mut lints = Vec::new();
        let parsed: Result<Vec<Vec<Seg>>, _> = answer
            .iter()
            .map(|(lineno, line)| super::super::cloze::scan_markers(line, *lineno, &mut lints))
            .collect();
        parsed.ok().map(|parsed| (answer, parsed))
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn stream_text_is_the_concatenation_of_piece_texts(lines in arbitrary_lines()) {
            let Some((answer, parsed)) = parsed_or_skip(&lines) else {
                return Ok(());
            };
            let s = maskable_stream(&answer, &parsed);
            let mut rebuilt = String::new();
            for (slot, line) in s.lines.iter().enumerate() {
                if slot > 0 {
                    rebuilt.push('\n');
                }
                for piece in &line.pieces {
                    rebuilt.push_str(&s.text[piece.range.clone()]);
                }
            }
            prop_assert_eq!(&rebuilt, &s.text, "lines: {:?}", lines);
        }

        #[test]
        fn every_piece_char_splices_char_aligned_into_its_authored_line(
            lines in arbitrary_lines()
        ) {
            let Some((answer, parsed)) = parsed_or_skip(&lines) else {
                return Ok(());
            };
            let s = maskable_stream(&answer, &parsed);
            for line in &s.lines {
                let raw = &answer[line.answer_index].1;
                for piece in &line.pieces {
                    for (start, end) in &piece.map {
                        prop_assert!(
                            start < end && *end <= raw.len(),
                            "map {start}..{end} outside {raw:?}"
                        );
                        prop_assert!(
                            raw.is_char_boundary(*start) && raw.is_char_boundary(*end),
                            "map {start}..{end} splits a char in {raw:?}"
                        );
                    }
                }
            }
        }

        #[test]
        fn accepted_occurrences_match_a_greedy_scan_over_matchable_text(
            lines in arbitrary_lines(),
            pick in any::<(u16, u16)>(),
        ) {
            let Some((answer, parsed)) = parsed_or_skip(&lines) else {
                return Ok(());
            };
            let s = maskable_stream(&answer, &parsed);
            if s.text.is_empty() {
                return Ok(());
            }
            let starts: Vec<usize> = s
                .text
                .char_indices()
                .map(|(byte, _)| byte)
                .collect();
            let start = starts[pick.0 as usize % starts.len()];
            let rest: Vec<usize> = s.text[start..]
                .char_indices()
                .skip(1)
                .map(|(byte, _)| start + byte)
                .chain([s.text.len()])
                .collect();
            let end = rest[pick.1 as usize % rest.len()];
            let hidden = &s.text[start..end];
            if hidden.is_empty() || hidden.contains('\n') {
                return Ok(());
            }

            let accepted = super::super::region::occurrences_with(
                &s.text,
                hidden,
                &mut |from, to| s.classify(&(from..to)) == RangeClass::Matchable,
            );

            // The independent oracle: greedy left-to-right over ONLY the
            // matchable text occurrences, so a crossing candidate can never
            // shadow an overlapping matchable one.
            let mut expected = Vec::new();
            let mut at = 0;
            while at + hidden.len() <= s.text.len() {
                if !s.text.is_char_boundary(at) || !s.text[at..].starts_with(hidden) {
                    at += 1;
                    continue;
                }
                let range = at..at + hidden.len();
                if s.classify(&range) == RangeClass::Matchable {
                    expected.push((range.start, range.end));
                    at += hidden.len();
                } else {
                    at += 1;
                }
            }
            prop_assert_eq!(
                &accepted, &expected,
                "hidden {:?} in {:?}", hidden, s.text
            );
            for pair in accepted.windows(2) {
                prop_assert!(pair[0].1 <= pair[1].0, "overlap: {accepted:?}");
            }
        }
    }
}
