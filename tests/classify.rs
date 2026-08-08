//! Property and unit tests for `classify`, written against the frozen spec
//! and the public API stub only (no implementation access).
//!
//! Two points in the spec are genuinely underspecified and are called out
//! at the point they matter below:
//!   - whether `Finding::DuplicateCardId::deck` is the deck's file name or its `deck_id` when they
//!     could differ (see `hedge` and `duplicate_card_id_round_trip`);
//!   - which file a `DuplicateCardId` finding sorts by (see `finding_sort_file`).

use std::collections::HashSet;

use alix::{FileEntry, Finding, Role, classify};
use proptest::prelude::*;

fn fe(name: &str, deck_id: Option<&str>, personal_for: Option<&str>, cards: &[&str]) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        deck_id: deck_id.map(str::to_string),
        personal_for: personal_for.map(str::to_string),
        card_ids: cards.iter().map(|s| s.to_string()).collect(),
    }
}

// ===========================================================================
// Focused unit tests for named boundaries from the spec.
// ===========================================================================

#[test]
fn empty_listing() {
    let (roles, findings) = classify(&[]);
    assert_eq!(roles, vec![]);
    assert_eq!(findings, vec![]);
}

#[test]
fn plain_deck_has_no_claim_and_no_findings() {
    let files = vec![fe("spanish.md", Some("spanish"), None, &[])];
    let (roles, findings) = classify(&files);
    assert_eq!(roles, vec![Role::Deck]);
    assert!(findings.is_empty());
}

#[test]
fn matching_sidecar_is_clean() {
    let files = vec![
        fe("spanish.md", Some("spanish"), None, &["c1"]),
        fe("spanish.personal.md", None, Some("spanish"), &["c2"]),
    ];
    let (roles, findings) = classify(&files);
    assert_eq!(
        roles,
        vec![
            Role::Deck,
            Role::Sidecar {
                parent: "spanish".into()
            }
        ]
    );
    assert!(
        findings.is_empty(),
        "no overlap, no mismatch: nothing to report"
    );
}

#[test]
fn personal_md_with_no_personal_for_is_parent_missing() {
    // Rule 1: "A `.personal.md` file with no `personal_for` at all is also this [ParentMissing]."
    let files = vec![fe("x.personal.md", None, None, &[])];
    let (roles, findings) = classify(&files);
    assert_eq!(roles, vec![Role::Unpaired]);
    assert_eq!(
        findings,
        vec![Finding::ParentMissing {
            file: "x.personal.md".into()
        }]
    );
}

#[test]
fn personal_for_naming_an_absent_deck_id_is_parent_missing() {
    let files = vec![fe("x.personal.md", None, Some("ghost"), &[])];
    let (roles, findings) = classify(&files);
    assert_eq!(roles, vec![Role::Unpaired]);
    assert_eq!(
        findings,
        vec![Finding::ParentMissing {
            file: "x.personal.md".into()
        }]
    );
}

#[test]
fn personal_for_resolves_elsewhere_but_disagrees_with_implied_neighbour_is_mismatch() {
    // "x.personal.md" implies neighbour "x.md" (id "x-id"), but personal_for
    // actually names "y-id" (a real deck, "y.md"). Both mechanisms disagree,
    // so a ParentMismatch is reported -- yet attachment still succeeds to
    // whatever personal_for actually named, per rule 1's literal wording.
    let files = vec![
        fe("x.md", Some("x-id"), None, &[]),
        fe("y.md", Some("y-id"), None, &[]),
        fe("x.personal.md", None, Some("y-id"), &[]),
    ];
    let (roles, findings) = classify(&files);
    assert_eq!(
        roles,
        vec![
            Role::Deck,
            Role::Deck,
            Role::Sidecar {
                parent: "y-id".into()
            }
        ]
    );
    assert_eq!(
        findings,
        vec![Finding::ParentMismatch {
            file: "x.personal.md".into(),
            named: "y-id".into(),
            neighbour: "x-id".into(),
        }]
    );
}

#[test]
fn parent_mismatch_downgrades_to_parent_missing_when_implied_neighbour_is_absent() {
    // Rule 2: "If the implied neighbour is not in the listing, this is
    // `ParentMissing` instead, not a mismatch." Here personal_for still
    // resolves to a real deck ("y-id"/"y.md"), so the file is still a valid
    // Sidecar -- but no "x.md" exists at all, so the finding is
    // ParentMissing, not ParentMismatch. This is the subtlest rule in the
    // spec (role success and finding co-occur) and the riskiest to get
    // wrong, hence its own dedicated test.
    let files = vec![
        fe("y.md", Some("y-id"), None, &[]),
        fe("x.personal.md", None, Some("y-id"), &[]),
    ];
    let (roles, findings) = classify(&files);
    assert_eq!(
        roles,
        vec![
            Role::Deck,
            Role::Sidecar {
                parent: "y-id".into()
            }
        ]
    );
    assert_eq!(
        findings,
        vec![Finding::ParentMissing {
            file: "x.personal.md".into()
        }]
    );
}

#[test]
fn self_referential_sidecar_resolves_against_itself_but_has_no_named_neighbour() {
    // Nothing in the spec excludes a file from satisfying its own
    // personal_for via its own deck_id. There is no "loop.md" file, so the
    // implied-neighbour override applies: ParentMissing despite resolving.
    let files = vec![fe(
        "loop.personal.md",
        Some("loop-id"),
        Some("loop-id"),
        &[],
    )];
    let (roles, findings) = classify(&files);
    assert_eq!(
        roles,
        vec![Role::Sidecar {
            parent: "loop-id".into()
        }]
    );
    assert_eq!(
        findings,
        vec![Finding::ParentMissing {
            file: "loop.personal.md".into()
        }]
    );
}

#[test]
fn personal_for_without_suffix_is_suffix_missing_and_unpaired() {
    let files = vec![fe("x.md", None, Some("y"), &[])];
    let (roles, findings) = classify(&files);
    assert_eq!(roles, vec![Role::Unpaired]);
    assert_eq!(
        findings,
        vec![Finding::SuffixMissing {
            file: "x.md".into()
        }]
    );
}

#[test]
fn duplicate_card_id_between_deck_and_its_sidecar() {
    // Hedge against the deck-name-vs-deck-id ambiguity in
    // `Finding::DuplicateCardId::deck`: make the deck's name and its own
    // deck_id the same string, so the expected value is right either way.
    let files = vec![
        fe("spanish.md", Some("spanish.md"), None, &["c1", "c2"]),
        fe(
            "spanish.personal.md",
            None,
            Some("spanish.md"),
            &["c2", "c3"],
        ),
    ];
    let (roles, findings) = classify(&files);
    assert_eq!(
        roles,
        vec![
            Role::Deck,
            Role::Sidecar {
                parent: "spanish.md".into()
            }
        ]
    );
    assert_eq!(
        findings,
        vec![Finding::DuplicateCardId {
            deck: "spanish.md".into(),
            sidecar: "spanish.personal.md".into(),
            card: "c2".into(),
        }]
    );
}

#[test]
fn exact_duplicate_findings_are_deduplicated_but_roles_are_one_per_entry() {
    // Totality: "may repeat a file name." Two fully identical entries
    // produce two roles (one per input entry) but only one ParentMissing
    // finding, since the two instances are exact duplicates.
    let files = vec![
        fe("x.personal.md", None, None, &[]),
        fe("x.personal.md", None, None, &[]),
    ];
    let (roles, findings) = classify(&files);
    assert_eq!(roles, vec![Role::Unpaired, Role::Unpaired]);
    assert_eq!(
        findings,
        vec![Finding::ParentMissing {
            file: "x.personal.md".into()
        }]
    );
}

#[test]
fn mixed_listing_end_to_end_roles_in_input_order_findings_globally_sorted() {
    let files = vec![
        fe("zz.personal.md", None, None, &[]), // ParentMissing (no personal_for)
        fe("b.md", Some("b.md"), None, &["k1"]),
        fe("aa.md", None, Some("nope"), &[]), // SuffixMissing (claims, wrong suffix)
        fe("b.personal.md", None, Some("b.md"), &["k1", "k2"]), // clean pair + dup card k1
        fe("c.personal.md", None, Some("y-id"), &[]), // resolves to "other.md", mismatches "c.md"
        fe("other.md", Some("y-id"), None, &[]),
        fe("c.md", Some("x-id"), None, &[]),
    ];
    let (roles, findings) = classify(&files);

    assert_eq!(
        roles,
        vec![
            Role::Unpaired,
            Role::Deck,
            Role::Unpaired,
            Role::Sidecar {
                parent: "b.md".into()
            },
            Role::Sidecar {
                parent: "y-id".into()
            },
            Role::Deck,
            Role::Deck,
        ]
    );

    // Sorted by (file name concerned, enum order): aa.md, b.personal.md,
    // c.personal.md, zz.personal.md.
    assert_eq!(
        findings,
        vec![
            Finding::SuffixMissing {
                file: "aa.md".into()
            },
            Finding::DuplicateCardId {
                deck: "b.md".into(),
                sidecar: "b.personal.md".into(),
                card: "k1".into(),
            },
            Finding::ParentMismatch {
                file: "c.personal.md".into(),
                named: "y-id".into(),
                neighbour: "x-id".into(),
            },
            Finding::ParentMissing {
                file: "zz.personal.md".into()
            },
        ]
    );
}

#[test]
fn totality_smoke_no_panic_on_pathological_input() {
    // Totality: empty names, names that aren't valid paths, unicode, an
    // exact-suffix-only name (empty stem), and duplicate names with
    // divergent content. Content is deliberately not asserted here (exact
    // behaviour under duplicate names with divergent fields is not pinned
    // by the spec); this only checks the function is total.
    let files = vec![
        fe("", None, None, &[]),
        fe(
            "weird/name\\with:chars",
            Some(""),
            Some(""),
            &["", "dup", "dup"],
        ),
        fe(".personal.md", None, Some("x"), &[]),
        fe("a.personal.md", Some("a"), Some("a"), &["x"]),
        fe("a.personal.md", Some("b"), None, &["x"]),
        fe(
            "\u{1F389}.personal.md",
            Some("\u{1F389}"),
            Some("\u{1F389}"),
            &["\u{1F389}"],
        ),
    ];
    let (roles, findings) = classify(&files);
    assert_eq!(roles.len(), files.len());
    let _ = findings;
}

// ===========================================================================
// Property-based tests.
// ===========================================================================

fn finding_rank(f: &Finding) -> u8 {
    match f {
        Finding::ParentMissing { .. } => 0,
        Finding::ParentMismatch { .. } => 1,
        Finding::DuplicateCardId { .. } => 2,
        Finding::SuffixMissing { .. } => 3,
    }
}

// Best-effort reading of "the file name they concern" for DuplicateCardId:
// every other finding variant's `file` field names the .personal.md side
// (the file making the claim), never the plain deck side, so DuplicateCardId
// is read the same way here (sorts by `sidecar`). Not settled by the spec
// text; flagged rather than asserted silently.
fn finding_sort_file(f: &Finding) -> &str {
    match f {
        Finding::ParentMissing { file } => file,
        Finding::ParentMismatch { file, .. } => file,
        Finding::DuplicateCardId { sidecar, .. } => sidecar,
        Finding::SuffixMissing { file } => file,
    }
}

fn ends_personal(name: &str) -> bool {
    name.ends_with(".personal.md")
}

fn resolves(files: &[FileEntry], personal_for: &Option<String>) -> bool {
    match personal_for {
        None => false,
        Some(pf) => files
            .iter()
            .any(|g| g.deck_id.as_deref() == Some(pf.as_str())),
    }
}

fn expected_role(files: &[FileEntry], f: &FileEntry) -> Role {
    let suffix_ok = ends_personal(&f.name);
    if suffix_ok && resolves(files, &f.personal_for) {
        Role::Sidecar {
            parent: f.personal_for.clone().unwrap(),
        }
    } else if suffix_ok || f.personal_for.is_some() {
        Role::Unpaired
    } else {
        Role::Deck
    }
}

fn expected_roles(files: &[FileEntry]) -> Vec<Role> {
    files.iter().map(|f| expected_role(files, f)).collect()
}

/// `Some(deck_id_of_that_file)` if exactly-named `name` exists in `files`,
/// `None` if no such file exists. Only meaningful when names are unique.
fn neighbour_deck_id(files: &[FileEntry], name: &str) -> Option<Option<String>> {
    files
        .iter()
        .find(|g| g.name == name)
        .map(|g| g.deck_id.clone())
}

fn expected_parent_missing(files: &[FileEntry], f: &FileEntry) -> bool {
    if !ends_personal(&f.name) {
        return false;
    }
    if !resolves(files, &f.personal_for) {
        return true;
    }
    let stem = f.name.strip_suffix(".personal.md").unwrap();
    let neighbour_name = format!("{stem}.md");
    match neighbour_deck_id(files, &neighbour_name) {
        None => true,       // implied neighbour file absent -> ParentMissing per spec override
        Some(None) => true, // neighbour exists but has no deck_id -> no String to build a Mismatch
        Some(Some(_)) => false,
    }
}

fn expected_parent_mismatch(files: &[FileEntry], f: &FileEntry) -> Option<Finding> {
    if !ends_personal(&f.name) {
        return None;
    }
    if !resolves(files, &f.personal_for) {
        return None;
    }
    let stem = f.name.strip_suffix(".personal.md").unwrap();
    let neighbour_name = format!("{stem}.md");
    let nid = match neighbour_deck_id(files, &neighbour_name) {
        Some(Some(nid)) => nid,
        _ => return None,
    };
    let named = f.personal_for.clone().unwrap();
    if named == nid {
        None
    } else {
        Some(Finding::ParentMismatch {
            file: f.name.clone(),
            named,
            neighbour: nid,
        })
    }
}

fn expected_suffix_missing(f: &FileEntry) -> bool {
    f.personal_for.is_some() && !ends_personal(&f.name)
}

// --- Strategies -------------------------------------------------------

// Small, colliding vocabulary so personal_for/deck_id/name relationships
// actually match up with non-negligible probability.
fn stem() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("a".to_string()),
        Just("b".to_string()),
        Just("c".to_string())
    ]
}

fn small_name() -> impl Strategy<Value = String> {
    prop_oneof![
        stem().prop_map(|s| format!("{s}.md")),
        stem().prop_map(|s| format!("{s}.personal.md")),
        stem(),
        "[a-z]{0,4}",
    ]
}

fn small_id() -> impl Strategy<Value = Option<String>> {
    prop_oneof![1 => Just(None), 3 => stem().prop_map(Some)]
}

fn small_card() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("c1".to_string()),
        Just("c2".to_string()),
        Just("c3".to_string())
    ]
}

fn small_file_entry() -> impl Strategy<Value = FileEntry> {
    (
        small_name(),
        small_id(),
        small_id(),
        proptest::collection::vec(small_card(), 0..3),
    )
        .prop_map(|(name, deck_id, personal_for, card_ids)| FileEntry {
            name,
            deck_id,
            personal_for,
            card_ids,
        })
}

fn small_files() -> impl Strategy<Value = Vec<FileEntry>> {
    proptest::collection::vec(small_file_entry(), 0..6)
}

// Unique names side-steps a second, independent ambiguity: which deck_id
// "wins" a name-based neighbour lookup when two files share the implied
// neighbour's exact name but disagree on deck_id. That's not the ambiguity
// this suite is about, so it's controlled for here rather than guessed at.
fn small_files_unique_names() -> impl Strategy<Value = Vec<FileEntry>> {
    small_files().prop_map(|files| {
        let mut seen = HashSet::new();
        files
            .into_iter()
            .filter(|f| seen.insert(f.name.clone()))
            .collect()
    })
}

// Broad, uncorrelated generator for structural laws that must hold no
// matter the content (totality, sortedness, permutation invariance).
fn fuzzy_name() -> impl Strategy<Value = String> {
    ".{0,12}"
}

fn fuzzy_id() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), ".{0,6}".prop_map(Some)]
}

fn fuzzy_card() -> impl Strategy<Value = String> {
    ".{0,6}"
}

fn fuzzy_file_entry() -> impl Strategy<Value = FileEntry> {
    (
        fuzzy_name(),
        fuzzy_id(),
        fuzzy_id(),
        proptest::collection::vec(fuzzy_card(), 0..3),
    )
        .prop_map(|(name, deck_id, personal_for, card_ids)| FileEntry {
            name,
            deck_id,
            personal_for,
            card_ids,
        })
}

fn fuzzy_files() -> impl Strategy<Value = Vec<FileEntry>> {
    proptest::collection::vec(fuzzy_file_entry(), 0..8)
}

proptest! {
    #[test]
    fn role_count_matches_input_len(files in fuzzy_files()) {
        let (roles, _) = classify(&files);
        prop_assert_eq!(roles.len(), files.len());
    }

    #[test]
    fn findings_sorted_and_no_exact_duplicates(files in fuzzy_files()) {
        let (_, findings) = classify(&files);
        for w in findings.windows(2) {
            let ka = (finding_sort_file(&w[0]), finding_rank(&w[0]));
            let kb = (finding_sort_file(&w[1]), finding_rank(&w[1]));
            prop_assert!(ka <= kb);
        }
        for i in 0..findings.len() {
            for j in (i + 1)..findings.len() {
                prop_assert_ne!(&findings[i], &findings[j]);
            }
        }
    }

    #[test]
    fn findings_reference_real_input_files(files in fuzzy_files()) {
        let names: HashSet<&str> = files.iter().map(|f| f.name.as_str()).collect();
        let (_, findings) = classify(&files);
        for f in &findings {
            let referenced = match f {
                Finding::ParentMissing { file } => file,
                Finding::ParentMismatch { file, .. } => file,
                Finding::DuplicateCardId { sidecar, .. } => sidecar,
                Finding::SuffixMissing { file } => file,
            };
            prop_assert!(names.contains(referenced.as_str()));
        }
    }

    #[test]
    fn permutation_invariance((files, keys) in fuzzy_files().prop_flat_map(|files| {
        let len = files.len();
        (Just(files), proptest::collection::vec(any::<u32>(), len))
    })) {
        let mut idx: Vec<usize> = (0..files.len()).collect();
        idx.sort_by_key(|&i| keys[i]);
        let permuted: Vec<FileEntry> = idx.iter().map(|&i| files[i].clone()).collect();

        let (roles_orig, findings_orig) = classify(&files);
        let (roles_perm, findings_perm) = classify(&permuted);

        for (k, &orig_i) in idx.iter().enumerate() {
            prop_assert_eq!(&roles_perm[k], &roles_orig[orig_i]);
        }
        prop_assert_eq!(findings_perm, findings_orig);
    }

    #[test]
    fn role_matches_reference_model(files in small_files_unique_names()) {
        let (roles, _) = classify(&files);
        prop_assert_eq!(roles, expected_roles(&files));
    }

    #[test]
    fn simple_findings_match_reference_model(files in small_files_unique_names()) {
        let (_, findings) = classify(&files);
        for f in &files {
            let has_missing = findings.contains(&Finding::ParentMissing { file: f.name.clone() });
            prop_assert_eq!(has_missing, expected_parent_missing(&files, f));

            let has_suffix_missing = findings.contains(&Finding::SuffixMissing { file: f.name.clone() });
            prop_assert_eq!(has_suffix_missing, expected_suffix_missing(f));

            match expected_parent_mismatch(&files, f) {
                Some(expected) => prop_assert!(findings.contains(&expected)),
                None => {
                    let has_any = findings.iter().any(
                        |x| matches!(x, Finding::ParentMismatch { file, .. } if file == &f.name),
                    );
                    prop_assert!(!has_any);
                }
            }
        }
    }

    #[test]
    fn duplicate_card_id_round_trip(files in small_files_unique_names()) {
        let (roles, findings) = classify(&files);

        // Forward: every genuine overlap between a sidecar and a file
        // sharing its resolved parent id must be reported, regardless of
        // how `deck` spells that file (name or id -- see module docs).
        for (i, f) in files.iter().enumerate() {
            if let Role::Sidecar { parent } = &roles[i] {
                for d in files.iter().filter(|d| d.deck_id.as_deref() == Some(parent.as_str())) {
                    for card in &f.card_ids {
                        if d.card_ids.contains(card) {
                            let reported = findings.iter().any(|x| matches!(
                                x,
                                Finding::DuplicateCardId { sidecar, card: c, .. }
                                    if sidecar == &f.name && c == card
                            ));
                            prop_assert!(reported);
                        }
                    }
                }
            }
        }

        // Backward: every reported pair corresponds to a genuine overlap
        // between a real sidecar and a real file sharing its resolved
        // parent id, however `deck` names that file.
        for finding in &findings {
            if let Finding::DuplicateCardId { deck, sidecar, card } = finding {
                let sidecar_file = files.iter().find(|g| &g.name == sidecar);
                let sidecar_file = match sidecar_file {
                    Some(s) => s,
                    None => { prop_assert!(false, "sidecar must reference a real input file"); continue; }
                };
                let parent = match sidecar_file.personal_for.as_deref() {
                    Some(p) => p,
                    None => { prop_assert!(false, "sidecar finding on a file with no personal_for"); continue; }
                };
                // A directory cannot repeat a file name; this generator can,
                // so any file of that name may be the one reported.
                let named_decks: Vec<_> = files.iter().filter(|g| &g.name == deck).collect();
                prop_assert!(
                    !named_decks.is_empty(),
                    "deck must reference a real input file by name"
                );
                prop_assert!(sidecar_file.card_ids.contains(card));
                let genuine = named_decks
                    .iter()
                    .any(|d| d.deck_id.as_deref() == Some(parent) && d.card_ids.contains(card));
                prop_assert!(genuine, "reported an overlap no named deck carries");
            }
        }
    }
}
