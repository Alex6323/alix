use std::path::Path;

use crate::{deck::Deck, dedup::TopicDupe, token};

/// One `#` heading block, or the leading block before the first heading
/// (`heading` and `line` are `None` there).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Topic {
    pub heading: Option<String>,
    /// The bare token of the topic's `<!-- id: topic-<token> -->` marker.
    pub token: Option<String>,
    pub line: Option<usize>,
    pub marker_line: Option<usize>,
    pub prose: Vec<String>,
}

impl Topic {
    pub fn id(&self, deck_id: &str) -> Option<String> {
        match self.heading {
            None => (!deck_id.is_empty()).then(|| deck_id.to_string()),
            Some(_) => self.token.as_deref().map(token::format_topic_id),
        }
    }
}

pub fn duplicated_topics(deck: &Deck, workspace_root: Option<&Path>) -> Vec<TopicDupe> {
    let mut found = same_deck_duplicates(deck);
    if let Some(root) = workspace_root {
        let ids: Vec<String> = deck.topic_ids().into_iter().flatten().collect();
        found.extend(
            crate::dedup::scan_dir(root)
                .topic_dupes
                .into_iter()
                .filter(|dupe| ids.contains(&dupe.id)),
        );
    }
    found.sort_by(|a, b| (&a.id, &a.copies).cmp(&(&b.id, &b.copies)));
    found
}

fn same_deck_duplicates(deck: &Deck) -> Vec<TopicDupe> {
    let mut found: Vec<TopicDupe> = Vec::new();
    for topic in &deck.topics {
        let (Some(token), Some(line)) = (topic.token.as_deref(), topic.marker_line) else {
            continue;
        };
        let id = token::format_topic_id(token);
        let site = (deck.path.clone(), line);
        match found.iter_mut().find(|dupe| dupe.id == id) {
            Some(dupe) => dupe.copies.push(site),
            None => found.push(TopicDupe {
                id,
                keeper: site,
                copies: Vec::new(),
            }),
        }
    }
    found.retain(|dupe| !dupe.copies.is_empty());
    found
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        assemble::{self, AssembleConfig, SelectOptions, Selected},
        parser::{self, LintKind},
    };

    const REPEATED: &str =
        "# A\n<!-- id: topic-a -->\n## q\na\n# B\n<!-- id: topic-a -->\n## r\nb\n";

    fn config() -> AssembleConfig {
        AssembleConfig {
            review: Default::default(),
            ask: Default::default(),
            pacing: assemble::Pacing {
                max_session: 10,
                new_cards_percent: 30,
            },
            instance_store: None,
        }
    }

    fn open_review(path: &Path) -> usize {
        let mut store = assemble::store_for(&[path.to_path_buf()], None).unwrap();
        let Selected::Review(build) = assemble::select(
            vec![path.to_path_buf()],
            &mut store,
            &config(),
            &SelectOptions::default(),
        )
        .unwrap() else {
            panic!("a fact deck selects a review session");
        };
        build.session.cards().len()
    }

    #[test]
    fn a_topic_id_repeated_in_one_deck_names_the_later_topic_and_review_still_opens() {
        let parsed = parser::parse("d.md", REPEATED).unwrap();
        let repeats: Vec<(usize, &LintKind)> = parsed
            .lints
            .iter()
            .filter(|lint| matches!(lint.kind, LintKind::DuplicateTopicId { .. }))
            .map(|lint| (lint.line, &lint.kind))
            .collect();
        assert_eq!(
            vec![(6, &LintKind::DuplicateTopicId { first_line: 2 })],
            repeats,
            "parse: the later marker carries the lint"
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, format!("---\nid: \"deck-d\"\n---\n{REPEATED}")).unwrap();
        let deck = Deck::load(&path).unwrap();
        let found = duplicated_topics(&deck, None);
        assert_eq!(
            vec![TopicDupe {
                id: "topic-a".to_string(),
                keeper: (path.clone(), 5),
                copies: vec![(path.clone(), 9)],
            }],
            found,
            "findings: the first topic keeps the id"
        );
        let text = found[0].to_string();
        for site in [":5;", ":9 "] {
            assert!(text.contains(site), "display names {site}: {text}");
        }

        assert_eq!(2, open_review(&path), "review: both cards are served");
    }

    fn write_pair(dir: &Path) -> (PathBuf, PathBuf) {
        let one = dir.join("one.md");
        let two = dir.join("two.md");
        std::fs::write(
            &one,
            "---\nid: \"deck-one\"\n---\n# A\n<!-- id: topic-shared -->\n## q\na\n<!-- id: card-shared -->\n",
        )
        .unwrap();
        std::fs::write(
            &two,
            "---\nid: \"deck-two\"\n---\n# B\n<!-- id: topic-shared -->\n## r\nb\n<!-- id: card-shared -->\n",
        )
        .unwrap();
        (one, two)
    }

    #[test]
    fn opening_a_deck_never_remints_a_duplicate_topic_id() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two) = write_pair(dir.path());
        for path in [&one, &two] {
            open_review(path);
        }
        let texts: Vec<String> = [&one, &two]
            .iter()
            .map(|path| std::fs::read_to_string(path).unwrap())
            .collect();
        assert_eq!(
            1,
            texts
                .iter()
                .filter(|text| text.contains("card-shared"))
                .count(),
            "precondition: the open repaired the duplicate card token: {texts:?}"
        );
        for text in &texts {
            assert!(
                text.contains("<!-- id: topic-shared -->"),
                "the topic id survives the open: {text}"
            );
        }
    }

    #[test]
    fn a_workspace_finding_names_the_copy_in_the_other_deck() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two) = write_pair(dir.path());
        let expected = vec![TopicDupe {
            id: "topic-shared".to_string(),
            keeper: (one.clone(), 5),
            copies: vec![(two.clone(), 5)],
        }];
        for path in [&one, &two] {
            let deck = Deck::load(path).unwrap();
            assert_eq!(
                expected,
                duplicated_topics(&deck, Some(dir.path())),
                "the same finding from either deck"
            );
            assert!(
                duplicated_topics(&deck, None).is_empty(),
                "no root: no cross-deck scan"
            );
        }
    }
}
