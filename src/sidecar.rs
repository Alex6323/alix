//! Pairing rules for personal sidecars, as a pure function over an already
//! read folder listing: name-based discovery and the `for:` link are
//! two independent mechanisms, so classification also reports where they
//! disagree.

use std::collections::{HashMap, HashSet};

/// One file in a folder listing, already read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// File name only, never a path, e.g. "spanish.personal.md".
    pub name: String,
    /// The file's own `id:` frontmatter value, when it has one.
    pub deck_id: Option<String>,
    /// The file's `for:` frontmatter value, when it has one.
    pub personal_for: Option<String>,
    /// Every card id the file carries, in file order.
    pub card_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// Offered in the picker, reviewable on its own.
    Deck,
    /// Never offered; belongs to the deck whose id this names.
    Sidecar { parent: String },
    /// Claims a parent, but no valid attachment could be made.
    Unpaired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    ParentMissing {
        file: String,
    },
    ParentMismatch {
        file: String,
        named: String,
        neighbour: String,
    },
    DuplicateCardId {
        deck: String,
        sidecar: String,
        card: String,
    },
    SuffixMissing {
        file: String,
    },
}

/// Classify a listing: one role per entry in input order, plus every finding
/// that applies, sorted by the file name it concerns and deduplicated.
pub fn classify(files: &[FileEntry]) -> (Vec<Role>, Vec<Finding>) {
    let index = Index::build(files);
    let roles: Vec<Role> = files.iter().map(|entry| role_of(entry, &index)).collect();

    let mut findings = Vec::new();
    for entry in files {
        pairing_findings(entry, &index, &mut findings);
    }
    duplicate_card_findings(files, &roles, &index, &mut findings);
    findings.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    findings.dedup();

    (roles, findings)
}

const SIDECAR_SUFFIX: &str = ".personal.md";
const DECK_SUFFIX: &str = ".md";

struct Index<'a> {
    /// Entry positions per `deck_id`, in input order.
    by_deck_id: HashMap<&'a str, Vec<usize>>,
    /// Deck ids per file name with `.md` cut off, so a sidecar finds the deck
    /// its own name implies without building that name.
    ids_by_stem: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> Index<'a> {
    fn build(files: &'a [FileEntry]) -> Self {
        let mut by_deck_id: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut ids_by_stem: HashMap<&str, Vec<&str>> = HashMap::new();
        for (position, entry) in files.iter().enumerate() {
            let Some(id) = entry.deck_id.as_deref() else {
                continue;
            };
            by_deck_id.entry(id).or_default().push(position);
            if let Some(stem) = entry.name.strip_suffix(DECK_SUFFIX) {
                ids_by_stem.entry(stem).or_default().push(id);
            }
        }
        Self {
            by_deck_id,
            ids_by_stem,
        }
    }

    fn resolves(&self, id: &str) -> bool {
        self.by_deck_id.contains_key(id)
    }

    fn implied_ids(&self, stem: &str) -> &[&'a str] {
        self.ids_by_stem.get(stem).map_or(&[], Vec::as_slice)
    }
}

fn role_of(entry: &FileEntry, index: &Index) -> Role {
    let suffixed = entry.name.ends_with(SIDECAR_SUFFIX);
    if suffixed
        && let Some(parent) = entry.personal_for.as_deref()
        && index.resolves(parent)
    {
        return Role::Sidecar {
            parent: parent.to_string(),
        };
    }
    if suffixed || entry.personal_for.is_some() {
        Role::Unpaired
    } else {
        Role::Deck
    }
}

fn pairing_findings(entry: &FileEntry, index: &Index, findings: &mut Vec<Finding>) {
    let Some(stem) = entry.name.strip_suffix(SIDECAR_SUFFIX) else {
        if entry.personal_for.is_some() {
            findings.push(Finding::SuffixMissing {
                file: entry.name.clone(),
            });
        }
        return;
    };
    let named = entry
        .personal_for
        .as_deref()
        .filter(|id| index.resolves(id));
    let missing_parent = Finding::ParentMissing {
        file: entry.name.clone(),
    };
    let Some(named) = named else {
        findings.push(missing_parent);
        return;
    };
    let implied = index.implied_ids(stem);
    match implied.first() {
        None => findings.push(missing_parent),
        Some(neighbour) if !implied.contains(&named) => findings.push(Finding::ParentMismatch {
            file: entry.name.clone(),
            named: named.to_string(),
            neighbour: (*neighbour).to_string(),
        }),
        Some(_) => {}
    }
}

fn duplicate_card_findings(
    files: &[FileEntry],
    roles: &[Role],
    index: &Index,
    findings: &mut Vec<Finding>,
) {
    for (position, role) in roles.iter().enumerate() {
        let Role::Sidecar { parent } = role else {
            continue;
        };
        let sidecar = &files[position];
        let own: HashSet<&str> = sidecar.card_ids.iter().map(String::as_str).collect();
        if own.is_empty() {
            continue;
        }
        for &deck in index.by_deck_id.get(parent.as_str()).into_iter().flatten() {
            for card in &files[deck].card_ids {
                if own.contains(card.as_str()) {
                    findings.push(Finding::DuplicateCardId {
                        deck: files[deck].name.clone(),
                        sidecar: sidecar.name.clone(),
                        card: card.clone(),
                    });
                }
            }
        }
    }
}

fn sort_key(finding: &Finding) -> (&str, u8, &str, &str) {
    match finding {
        Finding::ParentMissing { file } => (file, 0, "", ""),
        Finding::ParentMismatch {
            file,
            named,
            neighbour,
        } => (file, 1, named, neighbour),
        Finding::DuplicateCardId {
            deck,
            sidecar,
            card,
        } => (sidecar, 2, deck, card),
        Finding::SuffixMissing { file } => (file, 3, "", ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(
        name: &str,
        deck_id: Option<&str>,
        personal_for: Option<&str>,
        cards: &[&str],
    ) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            deck_id: deck_id.map(str::to_string),
            personal_for: personal_for.map(str::to_string),
            card_ids: cards.iter().map(|c| c.to_string()).collect(),
        }
    }

    fn deck(name: &str, id: &str) -> FileEntry {
        file(name, Some(id), None, &[])
    }

    fn sidecar(name: &str, parent: &str) -> FileEntry {
        file(name, None, Some(parent), &[])
    }

    fn plain(name: &str) -> FileEntry {
        file(name, None, None, &[])
    }

    fn missing(name: &str) -> Finding {
        Finding::ParentMissing {
            file: name.to_string(),
        }
    }

    fn mismatch(name: &str, named: &str, neighbour: &str) -> Finding {
        Finding::ParentMismatch {
            file: name.to_string(),
            named: named.to_string(),
            neighbour: neighbour.to_string(),
        }
    }

    fn duplicate(deck: &str, sidecar: &str, card: &str) -> Finding {
        Finding::DuplicateCardId {
            deck: deck.to_string(),
            sidecar: sidecar.to_string(),
            card: card.to_string(),
        }
    }

    fn suffix_missing(name: &str) -> Finding {
        Finding::SuffixMissing {
            file: name.to_string(),
        }
    }

    fn attached(parent: &str) -> Role {
        Role::Sidecar {
            parent: parent.to_string(),
        }
    }

    #[test]
    fn a_pair_is_a_deck_and_the_sidecar_attached_to_it() {
        let files = vec![
            deck("spanish.md", "deck-1"),
            sidecar("spanish.personal.md", "deck-1"),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles,
            vec![Role::Deck, attached("deck-1")],
            "roles of a clean pair"
        );
        assert_eq!(findings, vec![], "a clean pair reports nothing");
    }

    #[test]
    fn a_role_follows_the_suffix_and_the_named_parent() {
        let parent = deck("spanish.md", "deck-1");
        let cases: &[(&str, Option<&str>, Role)] = &[
            ("spanish.personal.md", Some("deck-1"), attached("deck-1")),
            ("spanish.personal.md", Some("deck-9"), Role::Unpaired),
            ("spanish.personal.md", None, Role::Unpaired),
            ("mine.md", Some("deck-1"), Role::Unpaired),
            ("mine.md", Some("deck-9"), Role::Unpaired),
            ("mine.md", None, Role::Deck),
        ];

        for (name, personal_for, expected) in cases {
            let files = vec![parent.clone(), file(name, None, *personal_for, &[])];

            let (roles, _) = classify(&files);

            assert_eq!(
                roles.len(),
                2,
                "one role per entry for {name} claiming {personal_for:?}"
            );
            assert_eq!(
                roles[0],
                Role::Deck,
                "the parent of {name} claiming {personal_for:?}"
            );
            assert_eq!(
                &roles[1], expected,
                "the role of {name} claiming {personal_for:?}"
            );
        }
    }

    #[test]
    fn roles_come_back_one_per_entry_in_input_order() {
        let listings: &[Vec<FileEntry>] = &[
            vec![],
            vec![plain("a.md")],
            vec![
                deck("spanish.md", "deck-1"),
                sidecar("spanish.personal.md", "deck-1"),
                plain("french.md"),
            ],
            vec![
                sidecar("orphan.personal.md", "deck-9"),
                file("mine.md", None, Some("deck-9"), &[]),
            ],
        ];

        for files in listings {
            let (roles, _) = classify(files);

            assert_eq!(roles.len(), files.len(), "one role per entry for {files:?}");
        }
    }

    #[test]
    fn a_suffixed_file_without_the_key_is_missing_its_parent() {
        let files = vec![deck("spanish.md", "deck-1"), plain("spanish.personal.md")];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles[1],
            Role::Unpaired,
            "a sidecar naming no parent cannot attach"
        );
        assert_eq!(
            findings,
            vec![missing("spanish.personal.md")],
            "findings for a keyless sidecar"
        );
    }

    #[test]
    fn a_suffixed_file_naming_an_absent_id_is_missing_its_parent() {
        let files = vec![
            deck("spanish.md", "deck-1"),
            sidecar("spanish.personal.md", "deck-9"),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles[1],
            Role::Unpaired,
            "a sidecar naming an absent id cannot attach"
        );
        assert_eq!(
            findings,
            vec![missing("spanish.personal.md")],
            "findings for an absent parent"
        );
    }

    #[test]
    fn a_sidecar_naming_another_deck_mismatches_its_implied_neighbour() {
        let files = vec![
            deck("spanish.md", "deck-1"),
            deck("french.md", "deck-2"),
            sidecar("spanish.personal.md", "deck-2"),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles[2],
            attached("deck-2"),
            "a mismatched sidecar attaches to the id it names"
        );
        assert_eq!(
            findings,
            vec![mismatch("spanish.personal.md", "deck-2", "deck-1")],
            "findings name the id claimed and the id implied"
        );
    }

    #[test]
    fn a_named_parent_without_the_implied_neighbour_is_missing_not_mismatched() {
        let files = vec![
            deck("french.md", "deck-2"),
            sidecar("spanish.personal.md", "deck-2"),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles[1],
            attached("deck-2"),
            "the named parent is present, so it attaches"
        );
        assert_eq!(
            findings,
            vec![missing("spanish.personal.md")],
            "an absent neighbour is a missing parent, never a mismatch"
        );
    }

    #[test]
    fn an_implied_neighbour_without_an_id_is_no_implied_deck() {
        let files = vec![
            plain("spanish.md"),
            deck("french.md", "deck-2"),
            sidecar("spanish.personal.md", "deck-2"),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![missing("spanish.personal.md")],
            "a neighbour carrying no id is no deck to mismatch against"
        );
    }

    #[test]
    fn the_key_without_the_suffix_reports_the_missing_suffix() {
        let files = vec![
            deck("spanish.md", "deck-1"),
            file("mine.md", None, Some("deck-1"), &[]),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles[1],
            Role::Unpaired,
            "a file discovery would offer cannot attach"
        );
        assert_eq!(
            findings,
            vec![suffix_missing("mine.md")],
            "findings for a suffixless claim"
        );
    }

    #[test]
    fn a_missing_suffix_never_also_reports_a_missing_parent() {
        let files = vec![file("mine.md", None, Some("deck-9"), &[])];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![suffix_missing("mine.md")],
            "a missing parent is only reported for a suffixed file"
        );
    }

    #[test]
    fn a_card_id_in_both_files_of_a_pair_is_reported() {
        let files = vec![
            file("spanish.md", Some("deck-1"), None, &["card-a", "card-b"]),
            file(
                "spanish.personal.md",
                None,
                Some("deck-1"),
                &["card-b", "card-c"],
            ),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![duplicate("spanish.md", "spanish.personal.md", "card-b")],
            "the shared card id is reported with the pair"
        );
    }

    #[test]
    fn a_duplicate_names_the_deck_by_its_file_name_never_by_its_id() {
        let files = vec![
            file("a.personal.md", None, Some("deck-1"), &["card-a"]),
            file("other.md", Some("deck-1"), None, &["card-a"]),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![
                missing("a.personal.md"),
                duplicate("other.md", "a.personal.md", "card-a"),
            ],
            "the deck side of a duplicate is the file name, not the id it named"
        );
    }

    #[test]
    fn a_card_id_shared_with_an_unattached_deck_is_not_reported() {
        let files = vec![
            file("spanish.md", Some("deck-1"), None, &["card-a"]),
            file("french.md", Some("deck-2"), None, &["card-b"]),
            file("spanish.personal.md", None, Some("deck-1"), &["card-b"]),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![],
            "a card id shared across unpaired files is no finding"
        );
    }

    #[test]
    fn an_unpaired_file_never_reports_a_duplicate_card_id() {
        let files = vec![
            file("spanish.md", Some("deck-1"), None, &["card-a"]),
            file("spanish.personal.md", None, Some("deck-9"), &["card-a"]),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(roles[1], Role::Unpaired, "the named parent is absent");
        assert_eq!(
            findings,
            vec![missing("spanish.personal.md")],
            "a card id is only shared through an attachment"
        );
    }

    #[test]
    fn a_card_id_repeated_inside_one_file_is_no_pairing_finding() {
        let files = vec![
            file("spanish.md", Some("deck-1"), None, &["card-a", "card-a"]),
            file("spanish.personal.md", None, Some("deck-1"), &["card-b"]),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![],
            "a file repeating its own card id is not a pairing finding"
        );
    }

    #[test]
    fn a_repeated_card_id_is_reported_once_per_pair() {
        let files = vec![
            file("spanish.md", Some("deck-1"), None, &["card-a", "card-a"]),
            file(
                "spanish.personal.md",
                None,
                Some("deck-1"),
                &["card-a", "card-a"],
            ),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![duplicate("spanish.md", "spanish.personal.md", "card-a")],
            "one shared card id is one finding however often it repeats"
        );
    }

    #[test]
    fn two_files_carrying_the_named_id_are_reported_separately() {
        let files = vec![
            file("spanish.md", Some("deck-1"), None, &["card-a"]),
            file("copy.md", Some("deck-1"), None, &["card-a", "card-b"]),
            file(
                "spanish.personal.md",
                None,
                Some("deck-1"),
                &["card-a", "card-b"],
            ),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![
                duplicate("copy.md", "spanish.personal.md", "card-a"),
                duplicate("copy.md", "spanish.personal.md", "card-b"),
                duplicate("spanish.md", "spanish.personal.md", "card-a"),
            ],
            "each deck file that shares a card is its own finding"
        );
    }

    #[test]
    fn a_file_that_is_its_own_parent_shares_every_card_id_with_itself() {
        let files = vec![file(
            "spanish.personal.md",
            Some("deck-1"),
            Some("deck-1"),
            &["card-a"],
        )];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles,
            vec![attached("deck-1")],
            "a self-naming file still attaches"
        );
        assert_eq!(
            findings,
            vec![
                missing("spanish.personal.md"),
                duplicate("spanish.personal.md", "spanish.personal.md", "card-a"),
            ],
            "a file attached to itself carries its card ids on both sides of the pair"
        );
    }

    #[test]
    fn every_applicable_finding_on_one_file_is_reported() {
        let files = vec![
            file("mine.md", Some("deck-1"), Some("deck-9"), &["card-a"]),
            file("mine.personal.md", None, Some("deck-1"), &["card-a"]),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![
                suffix_missing("mine.md"),
                duplicate("mine.md", "mine.personal.md", "card-a"),
            ],
            "both findings are reported, each under the file name it concerns"
        );
    }

    #[test]
    fn findings_are_sorted_by_file_name_then_by_enum_order() {
        let files = vec![
            file("mine.md", Some("deck-1"), Some("deck-9"), &["card-a"]),
            file("mine.personal.md", None, Some("deck-1"), &["card-a"]),
            plain("apple.personal.md"),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![
                missing("apple.personal.md"),
                suffix_missing("mine.md"),
                duplicate("mine.md", "mine.personal.md", "card-a"),
            ],
            "findings sort by file name first and by enum order second"
        );
    }

    #[test]
    fn exact_duplicate_findings_appear_once() {
        let files = vec![plain("spanish.personal.md"), plain("spanish.personal.md")];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles,
            vec![Role::Unpaired, Role::Unpaired],
            "each entry keeps its own role"
        );
        assert_eq!(
            findings,
            vec![missing("spanish.personal.md")],
            "two entries with the same defect report it once"
        );
    }

    #[test]
    fn permuting_the_listing_moves_the_roles_and_leaves_the_findings() {
        let files = vec![
            file("spanish.md", Some("deck-1"), None, &["card-a"]),
            file("spanish.personal.md", None, Some("deck-1"), &["card-a"]),
            file("french.md", Some("deck-2"), None, &[]),
            file("french.personal.md", None, Some("deck-1"), &[]),
            file("mine.md", None, Some("deck-2"), &[]),
            plain("orphan.personal.md"),
        ];
        let (expected_roles, expected_findings) = classify(&files);

        for rotation in 1..files.len() {
            let mut rotated = files.clone();
            rotated.rotate_left(rotation);
            let mut expected_roles = expected_roles.clone();
            expected_roles.rotate_left(rotation);

            let (roles, findings) = classify(&rotated);

            assert_eq!(roles, expected_roles, "roles after rotating by {rotation}");
            assert_eq!(
                findings, expected_findings,
                "findings after rotating by {rotation}"
            );
        }
    }

    #[test]
    fn classifying_the_same_listing_twice_gives_the_same_answer() {
        let files = vec![
            file("spanish.md", Some("deck-1"), None, &["card-a", "card-b"]),
            file("spanish.personal.md", None, Some("deck-2"), &["card-a"]),
            file("french.md", Some("deck-2"), None, &["card-b"]),
            file("french.personal.md", None, Some("deck-2"), &["card-b"]),
            file("mine.md", None, Some("deck-1"), &[]),
        ];

        let first = classify(&files);
        let second = classify(&files);

        assert_eq!(
            first, second,
            "classification is a function of the input alone"
        );
    }

    #[test]
    fn an_empty_listing_classifies_to_nothing() {
        let (roles, findings) = classify(&[]);

        assert_eq!(roles, vec![], "no entries, no roles");
        assert_eq!(findings, vec![], "no entries, no findings");
    }

    #[test]
    fn files_claiming_each_other_are_each_classified() {
        let files = vec![
            file("a.personal.md", Some("id-b"), Some("id-a"), &[]),
            file("b.personal.md", Some("id-a"), Some("id-b"), &[]),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles,
            vec![attached("id-a"), attached("id-b")],
            "each attaches to the other"
        );
        assert_eq!(
            findings,
            vec![missing("a.personal.md"), missing("b.personal.md")],
            "neither implied neighbour is in the listing"
        );
    }

    #[test]
    fn a_repeated_deck_name_is_classified_per_entry() {
        let files = vec![
            file("spanish.md", Some("deck-1"), None, &["card-a"]),
            file("spanish.md", Some("deck-2"), None, &["card-a"]),
            file("spanish.personal.md", None, Some("deck-2"), &["card-a"]),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles,
            vec![Role::Deck, Role::Deck, attached("deck-2")],
            "a repeated name leaves both entries decks"
        );
        assert_eq!(
            findings,
            vec![duplicate("spanish.md", "spanish.personal.md", "card-a")],
            "the sidecar names one of the neighbours, so nothing mismatches"
        );
    }

    #[test]
    fn a_mismatch_names_the_first_implied_neighbour_in_input_order() {
        let files = vec![
            deck("spanish.md", "deck-1"),
            deck("spanish.md", "deck-2"),
            deck("french.md", "deck-3"),
            sidecar("spanish.personal.md", "deck-3"),
        ];

        let (_, findings) = classify(&files);

        assert_eq!(
            findings,
            vec![mismatch("spanish.personal.md", "deck-3", "deck-1")],
            "the first neighbour carrying an id is the one reported"
        );
    }

    #[test]
    fn a_name_is_read_as_a_sidecar_by_its_suffix_alone() {
        let files = vec![
            deck(".md", "deck-1"),
            plain(".personal.md"),
            plain("../weird name/x.personal.md"),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles,
            vec![Role::Deck, Role::Unpaired, Role::Unpaired],
            "an odd name is still classified by its suffix"
        );
        assert_eq!(
            findings,
            vec![
                missing("../weird name/x.personal.md"),
                missing(".personal.md")
            ],
            "both keyless sidecars report a missing parent"
        );
    }

    #[test]
    fn a_doubled_suffix_implies_the_sidecar_next_to_it() {
        let files = vec![
            deck("x.personal.md", "deck-1"),
            sidecar("x.personal.personal.md", "deck-1"),
        ];

        let (roles, findings) = classify(&files);

        assert_eq!(
            roles,
            vec![Role::Unpaired, attached("deck-1")],
            "the outer file claims a parent by suffix alone"
        );
        assert_eq!(
            findings,
            vec![missing("x.personal.md")],
            "only the file naming no parent is reported"
        );
    }
}

/// A card as the deck's author wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckCard {
    pub id: String,
    pub notes: Vec<String>,
}

/// One block of the personal file, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidecarBlock {
    Note { card: String, lines: Vec<String> },
    Card { id: String, notes: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionCard {
    pub id: String,
    pub notes: Vec<String>,
    pub personal: bool,
}

/// A note whose card is in neither list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Orphan {
    pub card: String,
    pub lines: Vec<String>,
}

/// Fold a deck and its personal file into one session: deck cards in deck
/// order, then personal cards in sidecar order, each carrying its own notes
/// followed by every sidecar note addressed to it.
pub fn merge(deck: &[DeckCard], sidecar: &[SidecarBlock]) -> (Vec<SessionCard>, Vec<Orphan>) {
    let mut cards: Vec<SessionCard> = deck
        .iter()
        .map(|card| SessionCard {
            id: card.id.clone(),
            notes: card.notes.clone(),
            personal: false,
        })
        .collect();
    for block in sidecar {
        if let SidecarBlock::Card { id, notes } = block {
            cards.push(SessionCard {
                id: id.clone(),
                notes: notes.clone(),
                personal: true,
            });
        }
    }

    // Every card exists before any note attaches, so a note reaches a personal
    // card declared after it.
    let mut orphans = Vec::new();
    for block in sidecar {
        let SidecarBlock::Note { card, lines } = block else {
            continue;
        };
        let mut attached = false;
        for session in cards.iter_mut().filter(|session| &session.id == card) {
            session.notes.extend(lines.iter().cloned());
            attached = true;
        }
        if !attached {
            orphans.push(Orphan {
                card: card.clone(),
                lines: lines.clone(),
            });
        }
    }
    (cards, orphans)
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn deck_card(id: &str, notes: &[&str]) -> DeckCard {
        DeckCard {
            id: id.to_string(),
            notes: notes.iter().map(|n| n.to_string()).collect(),
        }
    }

    fn note(card: &str, lines: &[&str]) -> SidecarBlock {
        SidecarBlock::Note {
            card: card.to_string(),
            lines: lines.iter().map(|l| l.to_string()).collect(),
        }
    }

    fn own_card(id: &str, notes: &[&str]) -> SidecarBlock {
        SidecarBlock::Card {
            id: id.to_string(),
            notes: notes.iter().map(|n| n.to_string()).collect(),
        }
    }

    fn ids(cards: &[SessionCard]) -> Vec<&str> {
        cards.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn deck_cards_lead_in_deck_order_and_personal_cards_follow_in_sidecar_order() {
        let (cards, orphans) = merge(
            &[deck_card("a", &[]), deck_card("b", &[])],
            &[own_card("y", &[]), own_card("z", &[])],
        );
        assert_eq!(vec!["a", "b", "y", "z"], ids(&cards));
        assert_eq!(
            vec![false, false, true, true],
            cards.iter().map(|c| c.personal).collect::<Vec<_>>()
        );
        assert!(orphans.is_empty());
    }

    #[test]
    fn a_cards_own_notes_come_first_then_sidecar_notes_in_sidecar_order() {
        let (cards, _) = merge(
            &[deck_card("a", &["authored one", "authored two"])],
            &[note("a", &["mine first"]), note("a", &["mine second"])],
        );
        assert_eq!(
            vec!["authored one", "authored two", "mine first", "mine second"],
            cards[0].notes
        );
    }

    #[test]
    fn identical_notes_are_never_deduplicated() {
        let (cards, _) = merge(
            &[deck_card("a", &["same"])],
            &[note("a", &["same"]), note("a", &["same"])],
        );
        assert_eq!(vec!["same", "same", "same"], cards[0].notes);
    }

    #[test]
    fn a_note_for_an_unknown_card_becomes_an_orphan_keeping_its_lines() {
        let (cards, orphans) = merge(&[deck_card("a", &[])], &[note("gone", &["still mine"])]);
        assert!(cards[0].notes.is_empty());
        assert_eq!(
            vec![Orphan {
                card: "gone".to_string(),
                lines: vec!["still mine".to_string()],
            }],
            orphans
        );
    }

    #[test]
    fn a_sidecar_note_may_address_a_personal_card() {
        let (cards, orphans) = merge(
            &[],
            &[own_card("p", &["its own"]), note("p", &["about mine"])],
        );
        assert_eq!(vec!["its own", "about mine"], cards[0].notes);
        assert!(
            orphans.is_empty(),
            "a note on a personal card is not an orphan"
        );
    }

    #[test]
    fn a_note_placed_before_the_personal_card_it_addresses_still_attaches() {
        let (_, orphans) = merge(&[], &[note("p", &["about mine"]), own_card("p", &[])]);
        assert!(
            orphans.is_empty(),
            "attachment does not depend on block order"
        );
    }

    #[test]
    fn a_repeated_id_gives_every_card_carrying_it_the_note() {
        let (cards, orphans) = merge(
            &[deck_card("dup", &[]), deck_card("dup", &[])],
            &[note("dup", &["shared"])],
        );
        assert_eq!(vec!["shared"], cards[0].notes);
        assert_eq!(vec!["shared"], cards[1].notes);
        assert!(
            orphans.is_empty(),
            "an attached note is never also an orphan"
        );
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert_eq!((Vec::new(), Vec::new()), merge(&[], &[]));
    }
}
