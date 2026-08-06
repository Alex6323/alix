use proptest::prelude::*;

const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
const DECK_HEAD: &str = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n";

fn alphabet_string(len: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(0usize..32, len)
        .prop_map(|idx| idx.into_iter().map(|i| ALPHABET[i] as char).collect())
}

// Cell/front/answer text: printable, single-line, free of every structural
// marker the grammar assigns meaning to, and never delimiter-shaped (an
// all-dash cell could make a data row read as a delimiter row).
fn safe_text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9][a-zA-Z0-9 ,.:;!?()_$]{0,18}"
        .prop_map(|s| s.trim().to_string())
        .prop_filter("nonempty after trim", |s| !s.is_empty())
}

#[derive(Debug, Clone)]
struct GenRow {
    cells: Vec<String>,
    stamp: Option<String>,
}

#[derive(Debug, Clone)]
struct GenTable {
    title: Option<String>,
    header: Vec<String>,
    rows: Vec<GenRow>,
    container: Option<String>,
}

fn gen_table() -> impl Strategy<Value = GenTable> {
    (2usize..=3, 1usize..=6, any::<bool>()).prop_flat_map(|(cols, nrows, stamped)| {
        let header = proptest::collection::vec(safe_text(), cols);
        let rows = proptest::collection::vec(proptest::collection::vec(safe_text(), cols), nrows);
        let stamps = proptest::collection::btree_set(alphabet_string(6), nrows);
        let container = alphabet_string(26);
        let title = proptest::option::of(safe_text());
        (header, rows, stamps, container, title).prop_map(
            move |(header, rows, stamps, container, title)| {
                let stamps: Vec<String> = stamps.into_iter().collect();
                GenTable {
                    title,
                    header,
                    rows: rows
                        .into_iter()
                        .enumerate()
                        .map(|(i, cells)| GenRow {
                            cells,
                            stamp: stamped.then(|| stamps[i].clone()),
                        })
                        .collect(),
                    container: stamped.then(|| format!("card-{container}")),
                }
            },
        )
    })
}

fn render_table(table: &GenTable) -> String {
    let mut out = String::new();
    if let Some(title) = &table.title {
        out.push_str(&format!("## {title}\n"));
    }
    out.push_str(&format!("| {} |\n", table.header.join(" | ")));
    out.push_str(&format!(
        "|{}\n",
        table.header.iter().map(|_| "---|").collect::<String>()
    ));
    for row in &table.rows {
        let mut line = format!("| {} |", row.cells.join(" | "));
        if let Some(stamp) = &row.stamp {
            line.push_str(&format!(" <!-- r:{stamp} -->"));
        }
        out.push_str(&line);
        out.push('\n');
    }
    if let Some(container) = &table.container {
        out.push_str(&format!("<!-- id: {container} -->\n"));
    }
    out
}

#[derive(Debug, Clone)]
enum CardShape {
    Plain,
    Cloze { holes: usize },
    Choice { distractors: usize },
}

#[derive(Debug, Clone)]
struct GenCard {
    front: String,
    answers: Vec<String>,
    note: Option<String>,
    shape: CardShape,
    token: Option<String>,
}

fn gen_shape() -> impl Strategy<Value = CardShape> {
    prop_oneof![
        3 => Just(CardShape::Plain),
        1 => (1usize..=3).prop_map(|holes| CardShape::Cloze { holes }),
        1 => (1usize..=3).prop_map(|distractors| CardShape::Choice { distractors }),
    ]
}

fn gen_card() -> impl Strategy<Value = GenCard> {
    (
        safe_text(),
        proptest::collection::vec(safe_text(), 1..=2),
        proptest::option::of(safe_text()),
        gen_shape(),
        proptest::option::of(alphabet_string(26)),
    )
        .prop_map(|(front, answers, note, shape, token)| GenCard {
            front,
            answers,
            note,
            shape,
            token: token.map(|t| format!("card-{t}")),
        })
}

fn render_card(card: &GenCard) -> String {
    let mut out = format!("## {}\n", card.front);
    match &card.shape {
        CardShape::Plain => {
            for answer in &card.answers {
                out.push_str(answer);
                out.push('\n');
            }
        }
        CardShape::Cloze { holes } => {
            let gaps: Vec<String> = (0..*holes).map(|i| format!("\\blank{{gap{i}}}")).collect();
            out.push_str(&format!("{} {}\n", card.answers[0], gaps.join(" and ")));
        }
        CardShape::Choice { distractors } => {
            out.push_str(&format!("- [x] {}\n", card.answers[0]));
            for i in 0..*distractors {
                out.push_str(&format!("- [ ] wrong option {i}\n"));
            }
        }
    }
    if let Some(note) = &card.note {
        out.push_str(&format!("> {note}\n"));
    }
    if let Some(token) = &card.token {
        out.push_str(&format!("<!-- id: {token} -->\n"));
    }
    out
}

#[derive(Debug, Clone)]
enum GenBlock {
    Card(GenCard),
    Table(GenTable),
}

fn gen_deck() -> impl Strategy<Value = (Vec<GenBlock>, bool)> {
    (
        proptest::collection::vec(
            prop_oneof![
                gen_card().prop_map(GenBlock::Card),
                gen_table().prop_map(GenBlock::Table)
            ],
            1..=4,
        ),
        any::<bool>(),
    )
}

fn render_deck(blocks: &[GenBlock], crlf: bool) -> String {
    let mut out = String::from(DECK_HEAD);
    for block in blocks {
        out.push('\n');
        match block {
            GenBlock::Card(card) => out.push_str(&render_card(card)),
            GenBlock::Table(table) => out.push_str(&render_table(table)),
        }
    }
    if crlf { out.replace('\n', "\r\n") } else { out }
}

proptest! {
    #[test]
    fn card_ids_round_trip_for_every_legal_shape(
        token in alphabet_string(26),
        row in proptest::option::of(alphabet_string(6)),
        hole in proptest::option::of(0u32..40),
        reversed in any::<bool>(),
    ) {
        // Deliberate exclusion: row+hole and reversed-hole ids are illegal
        // shapes the rejection tests own; this property covers legal ones.
        let (row, hole) = if row.is_some() { (row, None) } else { (None, hole) };
        let reversed = reversed && hole.is_none();
        let id = alix::token::format_card_id(&token, row.as_deref(), hole, reversed);
        let parsed = alix::token::parse_prefixed_card_id(&id);
        let base = format!("card-{token}");
        prop_assert_eq!(
            Some((base.as_str(), row.as_deref(), hole, reversed)),
            parsed
        );
    }

    #[test]
    fn parsing_arbitrary_line_soup_never_panics(
        lines in proptest::collection::vec(
            prop_oneof![
                safe_text(),
                Just("| a | b |".to_string()),
                Just("|---|---|".to_string()),
                Just("|--|".to_string()),
                Just("| a |".to_string()),
                Just("|".to_string()),
                Just("## front".to_string()),
                Just("> note".to_string()),
                Just("---".to_string()),
                Just("<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->".to_string()),
                Just("<!-- r:4k2x9w -->".to_string()),
                Just("```".to_string()),
                Just(String::new()),
                Just("| é\\|ü | \\![x] |".to_string()),
                Just("- [x] yes".to_string()),
                Just("- [ ] no".to_string()),
                Just("the \\blank{gap} here".to_string()),
                Just("\\## escaped heading".to_string()),
                Just("\\> escaped note".to_string()),
                Just("\\---".to_string()),
                Just("<!-- at: src/lib.rs:1-2 fingerprint: xxh64-0011223344556677 -->".to_string()),
                Just("<!-- direction: both -->".to_string()),
            ],
            0..14,
        )
    ) {
        let text = lines.join("\n");
        let _ = alix::parser::parse("deck.md", &text);
    }

    #[test]
    fn generated_cards_parse_to_their_shape(
        card in gen_card().prop_filter("correct must not collide with generated distractors",
            |c| !c.answers[0].starts_with("wrong"))
    ) {
        let text = format!("{DECK_HEAD}\n{}", render_card(&card));
        let deck = alix::parser::parse("deck.md", &text).expect("a generated card parses");
        match &card.shape {
            CardShape::Plain => {
                prop_assert_eq!(1, deck.cards.len());
                prop_assert_eq!(&card.answers, &deck.cards[0].back);
            }
            CardShape::Cloze { holes } => {
                prop_assert_eq!(*holes, deck.cards.len(), "one sub-card per hole");
                for (n, sub) in deck.cards.iter().enumerate() {
                    prop_assert_eq!(Some(n as u32), sub.hole);
                    prop_assert_eq!(format!("gap{n}"), sub.back[0].clone());
                }
            }
            CardShape::Choice { distractors } => {
                prop_assert_eq!(1, deck.cards.len());
                prop_assert_eq!(*distractors, deck.cards[0].authored_distractors.len());
                prop_assert_eq!(&card.answers[0], &deck.cards[0].back[0]);
            }
        }
        prop_assert_eq!(card.note.as_deref(), deck.cards[0].note.as_deref());
    }

    #[test]
    fn generated_tables_parse_back_to_their_rows(table in gen_table()) {
        let text = format!("{DECK_HEAD}\n{}", render_table(&table));
        let deck = alix::parser::parse("deck.md", &text).expect("a generated table parses");
        prop_assert_eq!(table.rows.len(), deck.cards.len());
        let expected_context: Vec<String> = table.title.iter().cloned().collect();
        for (row, card) in table.rows.iter().zip(&deck.cards) {
            prop_assert_eq!(&row.cells[0], &card.front);
            prop_assert_eq!(&row.cells[1], &card.back[0]);
            prop_assert_eq!(&expected_context, &card.context);
            match (&row.stamp, &table.container) {
                (Some(stamp), Some(container)) => {
                    prop_assert_eq!(Some(format!("{container}-t{stamp}")), card.id());
                }
                _ => prop_assert_eq!(None, card.id()),
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn stamping_generated_decks_is_reconstructible_and_idempotent(
        (blocks, crlf) in gen_deck()
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck.md");
        let original = render_deck(&blocks, crlf);
        std::fs::write(&path, &original).unwrap();

        let outcome = alix::stamp::stamp_deck(&path).expect("a generated deck stamps");
        let stamped = std::fs::read_to_string(&path).unwrap();

        let parsed = alix::parser::parse("deck.md", &stamped).expect("the stamped deck parses");
        prop_assert!(parsed.cards.iter().all(|c| c.token.is_some()), "every card has identity");

        let second = alix::stamp::stamp_deck(&path).expect("restamping succeeds");
        prop_assert_eq!(alix::stamp::StampOutcome::default(), second, "stamping is a fixed point");
        prop_assert_eq!(&stamped, &std::fs::read_to_string(&path).unwrap());

        let newline = if crlf { "\r\n" } else { "\n" };
        let mut reconstructed = stamped;
        for row in &outcome.minted_rows {
            let span = format!(" <!-- r:{row} -->");
            prop_assert_eq!(1, reconstructed.matches(&span).count());
            reconstructed = reconstructed.replacen(&span, "", 1);
        }
        for id in &outcome.minted_cards {
            let span = format!("<!-- id: {id} -->{newline}");
            prop_assert_eq!(1, reconstructed.matches(&span).count());
            reconstructed = reconstructed.replacen(&span, "", 1);
        }
        prop_assert_eq!(original, reconstructed, "stripping every mint restores the original bytes");
    }
}
