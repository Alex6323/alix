const FILE: &str = include_str!("../docs/include/deck-authoring-guide.md");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Guide,
    Format,
    CardRules,
    Shapes,
    ShapesDiagramRow,
    Diagrams,
    Practice,
    Workflow,
    Sources,
}

impl Part {
    fn anchor(self) -> &'static str {
        match self {
            Part::Guide => "guide",
            Part::Format => "format",
            Part::CardRules => "card-rules",
            Part::Shapes => "shapes",
            Part::ShapesDiagramRow => "shapes-diagram-row",
            Part::Diagrams => "diagrams",
            Part::Practice => "practice",
            Part::Workflow => "workflow",
            Part::Sources => "sources",
        }
    }
}

pub fn part(part: Part) -> String {
    extract(part, None)
}

pub fn part_without(part: Part, omitted: Part) -> String {
    extract(part, Some(omitted))
}

fn is_anchor_line(line: &str) -> bool {
    (line.starts_with("<!-- ANCHOR: ") || line.starts_with("<!-- ANCHOR_END: "))
        && line.ends_with(" -->")
}

fn extract(part: Part, omitted: Option<Part>) -> String {
    let open = format!("<!-- ANCHOR: {} -->", part.anchor());
    let close = format!("<!-- ANCHOR_END: {} -->", part.anchor());
    let omitted_open = omitted.map(|omitted| format!("<!-- ANCHOR: {} -->", omitted.anchor()));
    let omitted_close = omitted.map(|omitted| format!("<!-- ANCHOR_END: {} -->", omitted.anchor()));
    let mut inside = false;
    let mut skipping = false;
    let mut text = String::new();
    for line in FILE.lines() {
        let trimmed = line.trim_end();
        if !inside {
            inside = trimmed == open;
        } else if trimmed == close {
            break;
        } else if Some(trimmed) == omitted_open.as_deref() {
            skipping = true;
        } else if Some(trimmed) == omitted_close.as_deref() {
            skipping = false;
        } else if !skipping && !is_anchor_line(trimmed) {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{Part, part, part_without};

    #[test]
    fn every_guide_part_opens_with_its_first_line_and_carries_no_anchor_line() {
        let rows = [
            (
                Part::Guide,
                "**What you are writing.** An alix deck is one Markdown file of flashcards",
            ),
            (
                Part::Format,
                "**What you are writing.** An alix deck is one Markdown file of flashcards",
            ),
            (Part::CardRules, "Rules that hold whatever the shape:"),
            (
                Part::Shapes,
                "Which card shape suits which material. Read the material first, then",
            ),
            (
                Part::ShapesDiagramRow,
                "| A process that branches or sends work back",
            ),
            (
                Part::Diagrams,
                "**Diagrams.** A fenced code block whose info string is `mermaid` holds a",
            ),
            (
                Part::Practice,
                "**Topic prose is for a reader who knows nothing yet.** Group the cards",
            ),
            (
                Part::Workflow,
                "**Write no ids.** alix gives the deck, every topic and every card a",
            ),
            (
                Part::Sources,
                "**Sources for the exam.** The exam is alix's test of understanding: an AI",
            ),
        ];
        for (which, first_line) in rows {
            let text = part(which);
            assert!(
                text.starts_with(first_line),
                "{which:?} must open with {first_line:?}, got {:?}",
                text.lines().next()
            );
            assert!(
                !text.contains("<!-- ANCHOR"),
                "{which:?} carries an anchor line"
            );
        }
    }

    #[test]
    fn the_guide_part_is_its_six_parts_in_file_order() {
        let joined: String = [
            Part::Format,
            Part::Shapes,
            Part::Diagrams,
            Part::Practice,
            Part::Workflow,
            Part::Sources,
        ]
        .map(part)
        .concat();
        assert_eq!(joined, part(Part::Guide));
    }

    #[test]
    fn the_format_part_ends_with_the_nested_card_rules() {
        assert!(part(Part::Format).ends_with(&part(Part::CardRules)));
    }

    #[test]
    fn no_guide_part_carries_the_file_preamble() {
        for which in [
            Part::Guide,
            Part::Format,
            Part::CardRules,
            Part::Shapes,
            Part::ShapesDiagramRow,
            Part::Diagrams,
            Part::Practice,
            Part::Workflow,
            Part::Sources,
        ] {
            assert!(
                !part(which).contains("This file is the one source for how to write an alix deck"),
                "{which:?} carries the preamble"
            );
        }
    }

    #[test]
    fn the_shapes_part_without_its_diagram_row_loses_exactly_that_row() {
        let whole = part(Part::Shapes);
        let row = part(Part::ShapesDiagramRow);
        let without = part_without(Part::Shapes, Part::ShapesDiagramRow);
        assert_eq!(1, row.lines().count());
        assert_eq!(whole.replacen(&row, "", 1), without);
        assert!(!without.contains("A process that branches"));
        assert!(without.contains("| Ordered steps in one straight line"));
        assert!(without.contains("| An answer that cannot be typed"));
    }
}
