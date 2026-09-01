//! The kept-invisible survey behind doctor's calm note and its two
//! anomaly warnings.

use std::collections::BTreeMap;

/// The ink line: a byte that moves no ink horizontally cannot be typed from
/// what the screen shows, so it never grades; joiners, variant selectors, and
/// direction controls are the learner's device, not the learner. MVS is
/// deliberately absent: it spaces, so it grades.
pub fn paints_no_ink(c: char) -> bool {
    class_label(c).is_some()
}

fn class_label(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{00AD}' => "SHY",
        '\u{200B}' => "ZWSP",
        '\u{200C}' => "ZWNJ",
        '\u{200D}' => "ZWJ",
        '\u{2060}' => "WJ",
        '\u{FE0E}' => "VS15",
        '\u{FE0F}' => "VS16",
        '\u{202A}'..='\u{202C}' => "bidi embedding",
        '\u{2066}'..='\u{2069}' => "bidi isolate",
        '\u{E0000}'..='\u{E007F}' => "TAG",
        _ => return None,
    })
}

/// What a prose survey found: per-class counts of invisible bytes standing
/// outside any well-formed emoji sequence, and the tag characters among them
/// (the invisible-payload shape doctor warns about pointedly).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Survey {
    pub counts: BTreeMap<&'static str, usize>,
    pub stray_tags: usize,
}

impl Survey {
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty() && self.stray_tags == 0
    }

    pub fn absorb(&mut self, other: Survey) {
        for (label, n) in other.counts {
            *self.counts.entry(label).or_default() += n;
        }
        self.stray_tags += other.stray_tags;
    }
}

pub fn survey(text: &str) -> Survey {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut found = Survey::default();
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        if c == '\u{1F3F4}'
            && let Some(end) = subdivision_flag_end(&chars, i)
        {
            i = end;
            continue;
        }
        let Some(label) = class_label(c) else {
            i += 1;
            continue;
        };
        if matches!(c, '\u{E0000}'..='\u{E007F}') {
            found.stray_tags += 1;
            *found.counts.entry(label).or_default() += 1;
            i += 1;
            continue;
        }
        let inside_emoji = match c {
            '\u{200D}' => joins_pictographs(text, at),
            '\u{FE0E}' | '\u{FE0F}' => sits_on_emoji_base(&chars, i),
            _ => false,
        };
        if !inside_emoji {
            *found.counts.entry(label).or_default() += 1;
        }
        i += 1;
    }
    found
}

/// `U+1F3F4`, at least one tag letter, then the cancel: the only shape tag
/// characters legitimately take (a subdivision flag such as Scotland's).
fn subdivision_flag_end(chars: &[(usize, char)], base: usize) -> Option<usize> {
    let mut i = base + 1;
    let mut letters = 0;
    while matches!(chars.get(i), Some((_, '\u{E0061}'..='\u{E007A}'))) {
        letters += 1;
        i += 1;
    }
    (letters > 0 && matches!(chars.get(i), Some((_, '\u{E007F}')))).then_some(i + 1)
}

/// GB11 decides, via the grapheme segmenter, so Extended_Pictographic stays
/// the segmenter's table, never a hand-rolled one: the join succeeded exactly
/// when no grapheme boundary follows the ZWJ.
fn joins_pictographs(text: &str, zwj_at: usize) -> bool {
    use unicode_segmentation::UnicodeSegmentation;
    let after = zwj_at + '\u{200D}'.len_utf8();
    after < text.len() && !text.grapheme_indices(true).any(|(start, _)| start == after)
}

fn sits_on_emoji_base(chars: &[(usize, char)], selector: usize) -> bool {
    let Some((_, base)) = selector.checked_sub(1).and_then(|i| chars.get(i)) else {
        return false;
    };
    is_emoji(*base) || matches!(base, '0'..='9' | '#' | '*')
}

fn is_emoji(c: char) -> bool {
    use unicode_properties::emoji::UnicodeEmoji;
    c.is_emoji_char()
}

/// Survey answer lines the way every layer of the design reads them: a fence
/// keeps its bytes and stays out of the count.
pub fn survey_prose<'a, I: IntoIterator<Item = &'a str>>(lines: I) -> Survey {
    let mut found = Survey::default();
    let mut fence = None;
    for line in lines {
        match fence {
            Some((ch, open)) => {
                if crate::parser::closes_fence(line, ch, open) {
                    fence = None;
                }
            }
            None => match crate::parser::fence_opener(line) {
                Some(opened) => fence = Some(opened),
                None => found.absorb(survey(line)),
            },
        }
    }
    found
}

/// Only the reversal overrides: the Trojan Source shape layer 1 cannot reach
/// inside a fence.
pub fn has_reversal_override(text: &str) -> bool {
    text.contains(['\u{202D}', '\u{202E}'])
}

/// 1-based line of the first fenced line carrying a reversal override, the
/// one in-fence anomaly doctor warns about.
pub fn first_fenced_reversal_override(text: &str) -> Option<usize> {
    let mut fence = None;
    for (idx, line) in text.lines().enumerate() {
        match fence {
            Some((ch, open)) => {
                if crate::parser::closes_fence(line, ch, open) {
                    fence = None;
                } else if has_reversal_override(line) {
                    return Some(idx + 1);
                }
            }
            None => {
                if let Some(opened) = crate::parser::fence_opener(line) {
                    fence = Some(opened);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_survey_counts_by_class_and_stays_silent_inside_wellformed_emoji() {
        type Row = (
            &'static str,
            &'static [(&'static str, usize)],
            usize,
            &'static str,
        );
        let rows: &[Row] = &[
            ("plain text", &[], 0, "visible prose has nothing to count"),
            ("a\u{200B}b", &[("ZWSP", 1)], 0, "a zero-width space counts"),
            ("a\u{00AD}b", &[("SHY", 1)], 0, "a soft hyphen counts"),
            ("a\u{2060}b", &[("WJ", 1)], 0, "a word joiner counts"),
            (
                "a\u{200C}b",
                &[("ZWNJ", 1)],
                0,
                "a ZWNJ between Latin letters counts",
            ),
            (
                "a\u{200D}b",
                &[("ZWJ", 1)],
                0,
                "a ZWJ between Latin letters counts",
            ),
            (
                "\u{1F469}\u{200D}\u{1F680}",
                &[],
                0,
                "a ZWJ joining two pictographic characters is a visible emoji, not a finding",
            ),
            (
                "\u{2764}\u{FE0F}\u{200D}\u{1F525}",
                &[],
                0,
                "a variation selector between the ZWJ and its base is transparent",
            ),
            (
                "\u{2388}\u{200D}\u{2388}",
                &[],
                0,
                "extended pictographs that are not Emoji=Yes still join: GB11, one grapheme",
            ),
            (
                "\u{1F44D}\u{FE0F}",
                &[],
                0,
                "VS16 on an emoji base is presentation, not payload",
            ),
            (
                "1\u{FE0F}",
                &[],
                0,
                "VS16 on a keycap base is presentation too",
            ),
            (
                "a\u{FE0F}b",
                &[("VS16", 1)],
                0,
                "VS16 on a Latin base counts",
            ),
            (
                "a\u{FE0E}b",
                &[("VS15", 1)],
                0,
                "VS15 on a Latin base counts",
            ),
            (
                "\u{2066}\u{0633}\u{0644}\u{0627}\u{0645}\u{2069}",
                &[("bidi isolate", 2)],
                0,
                "isolates count calmly; they are legitimate and the note only informs",
            ),
            (
                "\u{202A}x\u{202C}",
                &[("bidi embedding", 2)],
                0,
                "embeddings count the same way",
            ),
            (
                "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
                &[],
                0,
                "a well-formed subdivision flag is a visible emoji and stays silent",
            ),
            (
                "a\u{E0067}b",
                &[("TAG", 1)],
                1,
                "a tag letter outside a flag is stray payload",
            ),
            (
                "\u{1F3F4}\u{E0067}",
                &[("TAG", 1)],
                1,
                "a flag base without the cancel is not well-formed, so its tag is stray",
            ),
        ];
        for (text, expected, stray, why) in rows {
            let got = survey(text);
            let want: BTreeMap<&str, usize> = expected.iter().copied().collect();
            assert_eq!(got.counts, want, "counts for {text:?}: {why}");
            assert_eq!(got.stray_tags, *stray, "stray tags for {text:?}: {why}");
        }
    }

    #[test]
    fn a_fence_stays_out_of_the_survey() {
        let lines = [
            "before\u{200B}",
            "```",
            "in\u{200B}side",
            "```",
            "after\u{200B}",
        ];
        let found = survey_prose(lines);
        assert_eq!(
            Some(&2),
            found.counts.get("ZWSP"),
            "the two prose ZWSPs count, the fenced one does not"
        );
    }

    #[test]
    fn reversal_overrides_are_detected_and_embeddings_are_not() {
        assert!(has_reversal_override("x\u{202E}y"));
        assert!(has_reversal_override("x\u{202D}y"));
        assert!(!has_reversal_override("x\u{202A}y\u{202C}"));
    }
}
