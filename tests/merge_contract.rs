use alix::{
    DeckCard, Orphan, SessionCard, SidecarBlock,
    card::{Badge, Note},
    merge,
};
use proptest::prelude::*;

fn ids() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::sample::select(vec!["", "a", "b", "shared"]).prop_map(str::to_owned),
        1 => any::<String>(),
    ]
}

fn badges() -> impl Strategy<Value = Badge> {
    prop::sample::select(vec![
        Badge::Note,
        Badge::Tip,
        Badge::Important,
        Badge::Warning,
        Badge::Caution,
    ])
}

fn notes() -> impl Strategy<Value = Note> {
    (badges(), any::<String>()).prop_map(|(badge, body)| Note { badge, body })
}

fn note_stacks() -> impl Strategy<Value = Vec<Note>> {
    prop::collection::vec(notes(), 0..5)
}

fn deck_cards() -> impl Strategy<Value = Vec<DeckCard>> {
    prop::collection::vec(
        (ids(), note_stacks()).prop_map(|(id, notes)| DeckCard { id, notes }),
        0..8,
    )
}

fn sidecar_blocks() -> impl Strategy<Value = Vec<SidecarBlock>> {
    prop::collection::vec(
        prop_oneof![
            (ids(), notes()).prop_map(|(card, note)| SidecarBlock::Note { card, note }),
            (ids(), note_stacks()).prop_map(|(id, notes)| SidecarBlock::Card { id, notes }),
        ],
        0..12,
    )
}

fn inputs() -> impl Strategy<Value = (Vec<DeckCard>, Vec<SidecarBlock>)> {
    (deck_cards(), sidecar_blocks())
}

fn addressed_notes(id: &str, sidecar: &[SidecarBlock]) -> Vec<Note> {
    sidecar
        .iter()
        .filter_map(|block| match block {
            SidecarBlock::Note { card, note } if card == id => Some(note.clone()),
            SidecarBlock::Note { .. } | SidecarBlock::Card { .. } => None,
        })
        .collect()
}

fn expected_cards(deck: &[DeckCard], sidecar: &[SidecarBlock]) -> Vec<SessionCard> {
    let deck_cards = deck.iter().map(|card| {
        let mut notes = card.notes.clone();
        notes.extend(addressed_notes(&card.id, sidecar));
        SessionCard {
            id: card.id.clone(),
            notes,
            personal: false,
        }
    });

    let personal_cards = sidecar.iter().filter_map(|block| match block {
        SidecarBlock::Card { id, notes } => {
            let mut merged_notes = notes.clone();
            merged_notes.extend(addressed_notes(id, sidecar));
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
            SidecarBlock::Note { card, note } if !id_exists(card) => Some(Orphan {
                card: card.clone(),
                note: note.clone(),
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

fn plain(bodies: &[&str]) -> Vec<Note> {
    bodies
        .iter()
        .map(|body| Note::plain((*body).to_owned()))
        .collect()
}

fn note(card: &str, body: &str) -> SidecarBlock {
    SidecarBlock::Note {
        card: card.to_owned(),
        note: Note::plain(body.to_owned()),
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
            notes: plain(&["d1"]),
        },
        DeckCard {
            id: "deck-2".to_owned(),
            notes: plain(&["d2"]),
        },
    ];
    let sidecar = vec![
        SidecarBlock::Card {
            id: "personal-1".to_owned(),
            notes: plain(&["p1"]),
        },
        note("deck-1", "attached"),
        SidecarBlock::Card {
            id: "personal-2".to_owned(),
            notes: plain(&["p2"]),
        },
    ];

    let (cards, orphans) = merge(&deck, &sidecar);

    assert_eq!(
        cards,
        vec![
            SessionCard {
                id: "deck-1".to_owned(),
                notes: plain(&["d1", "attached"]),
                personal: false,
            },
            SessionCard {
                id: "deck-2".to_owned(),
                notes: plain(&["d2"]),
                personal: false,
            },
            SessionCard {
                id: "personal-1".to_owned(),
                notes: plain(&["p1"]),
                personal: true,
            },
            SessionCard {
                id: "personal-2".to_owned(),
                notes: plain(&["p2"]),
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
        notes: plain(&["duplicate", "own-last"]),
    }];
    let sidecar = vec![
        note("same", "duplicate"),
        note("same", "sidecar-1"),
        note("same", "sidecar-2"),
        note("same", "duplicate"),
    ];

    let (cards, orphans) = merge(&deck, &sidecar);

    assert_eq!(
        cards[0].notes,
        plain(&[
            "duplicate",
            "own-last",
            "duplicate",
            "sidecar-1",
            "sidecar-2",
            "duplicate",
        ])
    );
    assert!(orphans.is_empty());
}

#[test]
fn personal_cards_receive_notes_even_when_the_note_comes_first() {
    let sidecar = vec![
        note("personal", "before"),
        SidecarBlock::Card {
            id: "personal".to_owned(),
            notes: plain(&["own"]),
        },
        note("personal", "after"),
    ];

    assert_eq!(
        merge(&[], &sidecar),
        (
            vec![SessionCard {
                id: "personal".to_owned(),
                notes: plain(&["own", "before", "after"]),
                personal: true,
            }],
            vec![],
        )
    );
}

#[test]
fn orphans_preserve_sidecar_order_badge_and_empty_content() {
    let deck = vec![DeckCard {
        id: "known".to_owned(),
        notes: vec![],
    }];
    let sidecar = vec![
        SidecarBlock::Note {
            card: "missing-1".to_owned(),
            note: Note {
                badge: Badge::Tip,
                body: "line-1".to_owned(),
            },
        },
        note("known", "attached"),
        note("missing-2", ""),
    ];

    let (_, orphans) = merge(&deck, &sidecar);

    assert_eq!(
        orphans,
        vec![
            Orphan {
                card: "missing-1".to_owned(),
                note: Note {
                    badge: Badge::Tip,
                    body: "line-1".to_owned(),
                },
            },
            Orphan {
                card: "missing-2".to_owned(),
                note: Note::plain(String::new()),
            },
        ]
    );
}

#[test]
fn repeated_ids_keep_every_card_position_and_attach_to_each_one() {
    let deck = vec![
        DeckCard {
            id: "repeated".to_owned(),
            notes: plain(&["deck-1"]),
        },
        DeckCard {
            id: "repeated".to_owned(),
            notes: plain(&["deck-2"]),
        },
    ];
    let sidecar = vec![
        SidecarBlock::Card {
            id: "repeated".to_owned(),
            notes: plain(&["personal-1"]),
        },
        note("repeated", "shared-note"),
        SidecarBlock::Card {
            id: "repeated".to_owned(),
            notes: plain(&["personal-2"]),
        },
    ];

    let (cards, orphans) = merge(&deck, &sidecar);

    assert_eq!(
        cards,
        vec![
            SessionCard {
                id: "repeated".to_owned(),
                notes: plain(&["deck-1", "shared-note"]),
                personal: false,
            },
            SessionCard {
                id: "repeated".to_owned(),
                notes: plain(&["deck-2", "shared-note"]),
                personal: false,
            },
            SessionCard {
                id: "repeated".to_owned(),
                notes: plain(&["personal-1", "shared-note"]),
                personal: true,
            },
            SessionCard {
                id: "repeated".to_owned(),
                notes: plain(&["personal-2", "shared-note"]),
                personal: true,
            },
        ]
    );
    assert!(orphans.is_empty());
}
