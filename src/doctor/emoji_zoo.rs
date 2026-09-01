use unicode_segmentation::UnicodeSegmentation;

use crate::invisible::{Survey, survey};

#[derive(Clone, Copy)]
enum Target {
    Zwj,
    VariationSelector,
    Tag,
    Mvs,
}

impl Target {
    fn contains(self, ch: char) -> bool {
        match self {
            Self::Zwj => ch == '\u{200d}',
            Self::VariationSelector => matches!(ch, '\u{fe0e}' | '\u{fe0f}'),
            Self::Tag => ('\u{e0020}'..='\u{e007f}').contains(&ch),
            Self::Mvs => ch == '\u{180e}',
        }
    }

    fn reported(self, found: &Survey) -> bool {
        match self {
            Self::Zwj => found.counts.contains_key("ZWJ"),
            Self::VariationSelector => {
                found.counts.contains_key("VS15") || found.counts.contains_key("VS16")
            }
            Self::Tag => found.stray_tags > 0,
            Self::Mvs => found.counts.contains_key("MVS"),
        }
    }
}

struct Row {
    label: &'static str,
    text: &'static str,
    target: Target,
    anomaly: bool,
}

#[test]
fn the_emoji_zoo_flags_only_invisibles_outside_well_formed_sequences() {
    let rows = [
        Row {
            label: "ZWJ woman technologist",
            text: "\u{1f469}\u{200d}\u{1f4bb}",
            target: Target::Zwj,
            anomaly: false,
        },
        Row {
            label: "ZWJ family chain",
            text: "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}",
            target: Target::Zwj,
            anomaly: false,
        },
        Row {
            label: "ZWJ heart on fire after VS16",
            text: "\u{2764}\u{fe0f}\u{200d}\u{1f525}",
            target: Target::Zwj,
            anomaly: false,
        },
        Row {
            label: "ZWJ skin-tone technologist",
            text: "\u{1f469}\u{1f3fd}\u{200d}\u{1f4bb}",
            target: Target::Zwj,
            anomaly: false,
        },
        Row {
            label: "ZWJ orphan",
            text: "\u{200d}",
            target: Target::Zwj,
            anomaly: true,
        },
        Row {
            label: "ZWJ after a non-emoji base",
            text: "\u{41}\u{200d}\u{1f4bb}",
            target: Target::Zwj,
            anomaly: true,
        },
        Row {
            label: "ZWJ before a non-emoji base",
            text: "\u{1f469}\u{200d}\u{41}",
            target: Target::Zwj,
            anomaly: true,
        },
        Row {
            label: "ZWJ between Extended_Pictographic symbols outside Emoji=Yes",
            text: "\u{2388}\u{200d}\u{2388}",
            target: Target::Zwj,
            anomaly: false,
        },
        Row {
            label: "ZWJ after an orphan skin-tone component",
            text: "\u{1f3fd}\u{200d}\u{1f4bb}",
            target: Target::Zwj,
            anomaly: true,
        },
        Row {
            label: "ZWJ after a keycap digit without its enclosing keycap",
            text: "\u{31}\u{200d}\u{1f4bb}",
            target: Target::Zwj,
            anomaly: true,
        },
        Row {
            label: "ZWJ between regional-indicator components",
            text: "\u{1f1e6}\u{200d}\u{1f1e7}",
            target: Target::Zwj,
            anomaly: true,
        },
        Row {
            label: "ZWJ followed by a variation selector before the pictograph",
            text: "\u{1f469}\u{200d}\u{fe0f}\u{1f4bb}",
            target: Target::Zwj,
            anomaly: true,
        },
        Row {
            label: "VS15 heart text presentation",
            text: "\u{2764}\u{fe0e}",
            target: Target::VariationSelector,
            anomaly: false,
        },
        Row {
            label: "VS16 heart emoji presentation",
            text: "\u{2764}\u{fe0f}",
            target: Target::VariationSelector,
            anomaly: false,
        },
        Row {
            label: "VS16 coffee emoji presentation",
            text: "\u{2615}\u{fe0f}",
            target: Target::VariationSelector,
            anomaly: false,
        },
        Row {
            label: "VS15 after a non-emoji base",
            text: "\u{41}\u{fe0e}",
            target: Target::VariationSelector,
            anomaly: true,
        },
        Row {
            label: "VS16 after a non-emoji base",
            text: "\u{41}\u{fe0f}",
            target: Target::VariationSelector,
            anomaly: true,
        },
        Row {
            label: "VS16 orphan",
            text: "\u{fe0f}",
            target: Target::VariationSelector,
            anomaly: true,
        },
        Row {
            label: "TAG England flag",
            text: "\u{1f3f4}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}",
            target: Target::Tag,
            anomaly: false,
        },
        Row {
            label: "TAG Scotland flag",
            text: "\u{1f3f4}\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}",
            target: Target::Tag,
            anomaly: false,
        },
        Row {
            label: "TAG Wales flag",
            text: "\u{1f3f4}\u{e0067}\u{e0062}\u{e0077}\u{e006c}\u{e0073}\u{e007f}",
            target: Target::Tag,
            anomaly: false,
        },
        Row {
            label: "TAG letters outside a flag",
            text: "\u{41}\u{e0061}\u{e0062}\u{e007f}",
            target: Target::Tag,
            anomaly: true,
        },
        Row {
            label: "TAG flag without cancel",
            text: "\u{1f3f4}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}",
            target: Target::Tag,
            anomaly: true,
        },
        Row {
            label: "TAG cancel outside a flag",
            text: "\u{e007f}",
            target: Target::Tag,
            anomaly: true,
        },
        Row {
            label: "TAG sequence after the wrong flag base",
            text: "\u{1f3f3}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}",
            target: Target::Tag,
            anomaly: true,
        },
        Row {
            label: "MVS inside Mongolian text",
            text: "\u{182c}\u{180e}\u{1820}",
            target: Target::Mvs,
            anomaly: false,
        },
        Row {
            label: "MVS alone still grades and never reports",
            text: "\u{180e}",
            target: Target::Mvs,
            anomaly: false,
        },
    ];

    let mut wrong = Vec::new();
    for row in rows {
        let targets = row
            .text
            .chars()
            .filter(|ch| row.target.contains(*ch))
            .count();
        assert!(targets > 0, "{}: row has no target scalar", row.label);

        if row.label.contains("Extended_Pictographic") {
            assert_eq!(
                row.text.graphemes(true).count(),
                1,
                "{}: GB11 must recognize the pictographic ZWJ grapheme",
                row.label
            );
        }

        let found = survey(row.text);
        let got = row.target.reported(&found);
        if got != row.anomaly {
            wrong.push(format!(
                "{}: expected reported {}, got {} from {:?}",
                row.label, row.anomaly, got, found
            ));
        }
    }

    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn the_emoji_zoo_source_is_printable_ascii_plus_lf() {
    let wrong = include_bytes!("emoji_zoo.rs")
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, byte)| *byte != b'\n' && !(b' '..=b'~').contains(byte))
        .collect::<Vec<_>>();

    assert!(wrong.is_empty(), "non-ASCII source bytes: {wrong:?}");
}
