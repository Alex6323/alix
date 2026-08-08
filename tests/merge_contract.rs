use alix::{DeckCard, Orphan, SessionCard, SidecarBlock, merge};
use proptest::prelude::*;

fn ids() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::sample::select(vec!["", "a", "b", "shared"]).prop_map(str::to_owned),
        1 => any::<String>(),
    ]
}

fn note_lines() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(any::<String>(), 0..5)
}

fn deck_cards() -> impl Strategy<Value = Vec<DeckCard>> {
    prop::collection::vec(
        (ids(), note_lines()).prop_map(|(id, notes)| DeckCard { id, notes }),
        0..8,
    )
}

fn sidecar_blocks() -> impl Strategy<Value = Vec<SidecarBlock>> {
    prop::collection::vec(
        prop_oneof![
            (ids(), note_lines()).prop_map(|(card, lines)| SidecarBlock::Note { card, lines }),
            (ids(), note_lines()).prop_map(|(id, notes)| SidecarBlock::Card { id, notes }),
        ],
        0..12,
    )
}

fn inputs() -> impl Strategy<Value = (Vec<DeckCard>, Vec<SidecarBlock>)> {
    (deck_cards(), sidecar_blocks())
}

fn addressed_lines(id: &str, sidecar: &[SidecarBlock]) -> Vec<String> {
    sidecar
        .iter()
        .filter_map(|block| match block {
            SidecarBlock::Note { card, lines, .. } if card == id => Some(lines),
            SidecarBlock::Note { .. } | SidecarBlock::Card { .. } => None,
        })
        .flatten()
        .cloned()
        .collect()
}

fn expected_cards(deck: &[DeckCard], sidecar: &[SidecarBlock]) -> Vec<SessionCard> {
    let deck_cards = deck.iter().map(|card| {
        let mut notes = card.notes.clone();
        notes.extend(addressed_lines(&card.id, sidecar));
        SessionCard {
            id: card.id.clone(),
            notes,
            personal: false,
        }
    });

    let personal_cards = sidecar.iter().filter_map(|block| match block {
        SidecarBlock::Card { id, notes } => {
            let mut merged_notes = notes.clone();
            merged_notes.extend(addressed_lines(id, sidecar));
            Some(SessionCard {
                id: id.clone(),
                notes: merged_notes,
                personal: true,
            })
        }
        SidecarBlock::Note { .. } => None,
    });

    deck_cards.chain(personal_cards).collect()
}

fn expected_orphans(deck: &[DeckCard], sidecar: &[SidecarBlock]) -> Vec<Orphan> {
    let id_exists = |id: &str| {
        deck.iter().any(|card| card.id == id)
            || sidecar.iter().any(
                |block| matches!(block, SidecarBlock::Card { id: card_id, .. } if card_id == id),
            )
    };

    sidecar
        .iter()
        .filter_map(|block| match block {
            SidecarBlock::Note { card, lines } if !id_exists(card) => Some(Orphan {
                card: card.clone(),
                lines: lines.clone(),
            }),
            SidecarBlock::Note { .. } | SidecarBlock::Card { .. } => None,
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn cards_follow_order_marking_and_note_laws((deck, sidecar) in inputs()) {
        let (cards, _) = merge(&deck, &sidecar);

        prop_assert_eq!(cards, expected_cards(&deck, &sidecar));
    }

    #[test]
    fn note_blocks_are_attached_or_orphaned_by_id((deck, sidecar) in inputs()) {
        let (_, orphans) = merge(&deck, &sidecar);

        prop_assert_eq!(orphans, expected_orphans(&deck, &sidecar));
    }

    #[test]
    fn merge_is_deterministic_for_all_generated_inputs((deck, sidecar) in inputs()) {
        prop_assert_eq!(merge(&deck, &sidecar), merge(&deck, &sidecar));
    }
}

#[test]
fn empty_inputs_produce_an_empty_session_and_no_orphans() {
    assert_eq!(merge(&[], &[]), (vec![], vec![]));
}

#[test]
fn deck_cards_precede_personal_cards_and_are_marked_by_origin() {
    let deck = vec![
        DeckCard {
            id: "deck-1".to_owned(),
            notes: vec!["d1".to_owned()],
        },
        DeckCard {
            id: "deck-2".to_owned(),
            notes: vec!["d2".to_owned()],
        },
    ];
    let sidecar = vec![
        SidecarBlock::Card {
            id: "personal-1".to_owned(),
            notes: vec!["p1".to_owned()],
        },
        SidecarBlock::Note {
            card: "deck-1".to_owned(),
            lines: vec!["attached".to_owned()],
        },
        SidecarBlock::Card {
            id: "personal-2".to_owned(),
            notes: vec!["p2".to_owned()],
        },
    ];

    let (cards, orphans) = merge(&deck, &sidecar);

    assert_eq!(
        cards,
        vec![
            SessionCard {
                id: "deck-1".to_owned(),
                notes: vec!["d1".to_owned(), "attached".to_owned()],
                personal: false,
            },
            SessionCard {
                id: "deck-2".to_owned(),
                notes: vec!["d2".to_owned()],
                personal: false,
            },
            SessionCard {
                id: "personal-1".to_owned(),
                notes: vec!["p1".to_owned()],
                personal: true,
            },
            SessionCard {
                id: "personal-2".to_owned(),
                notes: vec!["p2".to_owned()],
                personal: true,
            },
        ]
    );
    assert!(orphans.is_empty());
}

#[test]
fn notes_append_in_sidecar_order_without_deduplication() {
    let deck = vec![DeckCard {
        id: "same".to_owned(),
        notes: vec!["duplicate".to_owned(), "own-last".to_owned()],
    }];
    let sidecar = vec![
        SidecarBlock::Note {
            card: "same".to_owned(),
            lines: vec!["duplicate".to_owned(), "sidecar-1".to_owned()],
        },
        SidecarBlock::Note {
            card: "same".to_owned(),
            lines: vec!["sidecar-2".to_owned(), "duplicate".to_owned()],
        },
    ];

    let (cards, orphans) = merge(&deck, &sidecar);

    assert_eq!(
        cards[0].notes,
        vec![
            "duplicate",
            "own-last",
            "duplicate",
            "sidecar-1",
            "sidecar-2",
            "duplicate",
        ]
    );
    assert!(orphans.is_empty());
}

#[test]
fn personal_cards_receive_notes_even_when_the_note_comes_first() {
    let sidecar = vec![
        SidecarBlock::Note {
            card: "personal".to_owned(),
            lines: vec!["before".to_owned()],
        },
        SidecarBlock::Card {
            id: "personal".to_owned(),
            notes: vec!["own".to_owned()],
        },
        SidecarBlock::Note {
            card: "personal".to_owned(),
            lines: vec!["after".to_owned()],
        },
    ];

    assert_eq!(
        merge(&[], &sidecar),
        (
            vec![SessionCard {
                id: "personal".to_owned(),
                notes: vec!["own".to_owned(), "before".to_owned(), "after".to_owned()],
                personal: true,
            }],
            vec![],
        )
    );
}

#[test]
fn orphans_preserve_sidecar_order_lines_and_empty_content() {
    let deck = vec![DeckCard {
        id: "known".to_owned(),
        notes: vec![],
    }];
    let sidecar = vec![
        SidecarBlock::Note {
            card: "missing-1".to_owned(),
            lines: vec!["line-1".to_owned()],
        },
        SidecarBlock::Note {
            card: "known".to_owned(),
            lines: vec!["attached".to_owned()],
        },
        SidecarBlock::Note {
            card: "missing-2".to_owned(),
            lines: vec![],
        },
    ];

    let (_, orphans) = merge(&deck, &sidecar);

    assert_eq!(
        orphans,
        vec![
            Orphan {
                card: "missing-1".to_owned(),
                lines: vec!["line-1".to_owned()],
            },
            Orphan {
                card: "missing-2".to_owned(),
                lines: vec![],
            },
        ]
    );
}

#[test]
fn repeated_ids_keep_every_card_position_and_attach_to_each_one() {
    let deck = vec![
        DeckCard {
            id: "repeated".to_owned(),
            notes: vec!["deck-1".to_owned()],
        },
        DeckCard {
            id: "repeated".to_owned(),
            notes: vec!["deck-2".to_owned()],
        },
    ];
    let sidecar = vec![
        SidecarBlock::Card {
            id: "repeated".to_owned(),
            notes: vec!["personal-1".to_owned()],
        },
        SidecarBlock::Note {
            card: "repeated".to_owned(),
            lines: vec!["shared-note".to_owned()],
        },
        SidecarBlock::Card {
            id: "repeated".to_owned(),
            notes: vec!["personal-2".to_owned()],
        },
    ];

    let (cards, orphans) = merge(&deck, &sidecar);

    assert_eq!(
        cards,
        vec![
            SessionCard {
                id: "repeated".to_owned(),
                notes: vec!["deck-1".to_owned(), "shared-note".to_owned()],
                personal: false,
            },
            SessionCard {
                id: "repeated".to_owned(),
                notes: vec!["deck-2".to_owned(), "shared-note".to_owned()],
                personal: false,
            },
            SessionCard {
                id: "repeated".to_owned(),
                notes: vec!["personal-1".to_owned(), "shared-note".to_owned()],
                personal: true,
            },
            SessionCard {
                id: "repeated".to_owned(),
                notes: vec!["personal-2".to_owned(), "shared-note".to_owned()],
                personal: true,
            },
        ]
    );
    assert!(orphans.is_empty());
}
