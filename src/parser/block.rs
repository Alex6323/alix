use std::ops::Range;

use super::{closes_fence, fence_opener, trim_ws};

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "bound into region binding by the next ADR 0052 slice"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineKind {
    Heading,
    Media,
    Text,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "bound into region binding by the next ADR 0052 slice"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BodyLine<'a> {
    pub number: usize,
    pub text: &'a str,
    pub kind: LineKind,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "bound into region binding by the next ADR 0052 slice"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlockKind {
    Heading,
    Media,
    Fence,
    Math,
    Run,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "bound into region binding by the next ADR 0052 slice"
    )
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Block {
    pub kind: BlockKind,
    /// Indices into the body the blocks were cut from.
    pub range: Range<usize>,
}

/// The blocks of a card body in order. `definition_lines`
/// are the link-definition lines the scanner lifted out of the body: a run
/// continues across them, and ends at any other line the body skips.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "bound into region binding by the next ADR 0052 slice"
    )
)]
pub(super) fn blocks(body: &[BodyLine], definition_lines: &[usize]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut container: Option<Container> = None;
    for (index, line) in body.iter().enumerate() {
        let inside = match container {
            Some(Container::Fence { ch, open }) => {
                if closes_fence(line.text, ch, open) {
                    container = None;
                }
                true
            }
            Some(Container::Math) => {
                if trim_ws(line.text) == "$$" {
                    container = None;
                }
                true
            }
            None => false,
        };
        if inside && let Some(last) = blocks.last_mut() {
            last.range.end = index + 1;
            continue;
        }
        let kind = match line.kind {
            LineKind::Heading => BlockKind::Heading,
            LineKind::Media => BlockKind::Media,
            LineKind::Text => {
                if let Some((ch, open)) = fence_opener(line.text) {
                    container = Some(Container::Fence { ch, open });
                    BlockKind::Fence
                } else if trim_ws(line.text) == "$$" {
                    container = Some(Container::Math);
                    BlockKind::Math
                } else {
                    BlockKind::Run
                }
            }
        };
        if kind == BlockKind::Run
            && let Some(last) = blocks.last_mut()
            && last.kind == BlockKind::Run
            && (body[last.range.end - 1].number + 1..line.number)
                .all(|skipped| definition_lines.contains(&skipped))
        {
            last.range.end = index + 1;
            continue;
        }
        blocks.push(Block {
            kind,
            range: index..index + 1,
        });
    }
    blocks
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "bound into region binding by the next ADR 0052 slice"
    )
)]
#[derive(Clone, Copy)]
enum Container {
    Fence { ch: char, open: usize },
    Math,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(lines: &[(usize, &'static str, LineKind)]) -> Vec<BodyLine<'static>> {
        lines
            .iter()
            .map(|(number, text, kind)| BodyLine {
                number: *number,
                text,
                kind: *kind,
            })
            .collect()
    }

    fn assert_partition(body: &[BodyLine], blocks: &[Block]) {
        let mut covered = 0;
        for (index, block) in blocks.iter().enumerate() {
            assert_eq!(
                covered, block.range.start,
                "block {index} ({:?}) starts where the previous block ended",
                block.kind
            );
            assert!(
                block.range.end > block.range.start,
                "block {index} ({:?}) holds at least one line",
                block.kind
            );
            covered = block.range.end;
        }
        assert_eq!(body.len(), covered, "every content line sits in a block");
    }

    #[test]
    fn the_block_classifier_partitions_every_content_line_exactly_once() {
        use BlockKind as B;
        use LineKind as L;
        // Line 4 is a comment, 7 a link definition, 24 a blank line: the
        // scanner skips all three, and only the definition joins its run.
        let body = body(&[
            (1, "Name the parts", L::Heading),
            (2, "run a", L::Text),
            (3, "run b", L::Text),
            (5, "run c", L::Text),
            (6, "run d", L::Text),
            (8, "run e", L::Text),
            (9, "![](a.png)", L::Media),
            (10, "run f", L::Text),
            (11, "```", L::Text),
            (12, "![](inside.png)", L::Media),
            (13, "$$", L::Text),
            (14, "", L::Text),
            (15, "## not a heading", L::Text),
            (16, "```", L::Text),
            (17, "$$", L::Text),
            (18, "x = 1", L::Text),
            (19, "", L::Text),
            (20, "```", L::Text),
            (21, "$$", L::Text),
            (22, "> a quotation", L::Text),
            (23, "run g", L::Text),
            (25, "- item two", L::Text),
        ]);
        let blocks = blocks(&body, &[7]);
        assert_partition(&body, &blocks);
        let expected: Vec<Block> = [
            (B::Heading, 0..1),
            (B::Run, 1..3),
            (B::Run, 3..6),
            (B::Media, 6..7),
            (B::Run, 7..8),
            (B::Fence, 8..14),
            (B::Math, 14..19),
            (B::Run, 19..21),
            (B::Run, 21..22),
        ]
        .into_iter()
        .map(|(kind, range)| Block { kind, range })
        .collect();
        assert_eq!(
            expected, blocks,
            "a comment ends a run, a definition joins it, media splits it, a quotation joins it, fence and math interiors are never other blocks"
        );
    }

    #[test]
    fn an_unclosed_container_holds_every_line_to_the_end_of_the_body() {
        use BlockKind as B;
        use LineKind as L;
        for (label, opener, kind) in [("fence", "~~~", B::Fence), ("math", "$$", B::Math)] {
            let body = body(&[
                (1, "q", L::Heading),
                (2, opener, L::Text),
                (3, "![](a.png)", L::Media),
                (5, "after a skipped line", L::Text),
            ]);
            let blocks = blocks(&body, &[]);
            assert_partition(&body, &blocks);
            assert_eq!(
                vec![
                    Block {
                        kind: B::Heading,
                        range: 0..1
                    },
                    Block { kind, range: 1..4 }
                ],
                blocks,
                "{label}: an unclosed {label} swallows media and skipped lines alike"
            );
        }
    }
}
