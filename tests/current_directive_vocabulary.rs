#[test]
fn executable_fixtures_use_the_current_directive_vocabulary() {
    let surfaces = [
        (
            "adult math E2E deck",
            include_str!("../e2e/fixtures/decks/animals/decks/math.md"),
        ),
        (
            "adult multiple-choice E2E deck",
            include_str!("../e2e/fixtures/decks/animals/decks/multiple.md"),
        ),
        (
            "adult sequential multiple-choice E2E deck",
            include_str!("../e2e/fixtures/decks/animals/decks/multiple-sequence.md"),
        ),
        (
            "section context E2E deck",
            include_str!("../e2e/fixtures/decks/animals/decks/section-context-pill.md"),
        ),
        (
            "mobile Rust bridge test",
            include_str!("../mobile/alix/rust/src/api/review.rs"),
        ),
        (
            "session assembly warning labels",
            include_str!("../src/assemble.rs"),
        ),
    ];
    let undefined = [
        "<!-- choices-single -->",
        "<!-- choices-multiple -->",
        "order: sequential",
        "\"order\"",
    ];
    let stale: Vec<String> = surfaces
        .into_iter()
        .flat_map(|(surface, text)| {
            undefined
                .into_iter()
                .filter(move |word| text.contains(word))
                .map(move |word| format!("{surface}: {word}"))
        })
        .collect();

    assert!(
        stale.is_empty(),
        "executable fixtures use spellings the current grammar does not define: {stale:?}"
    );
}

#[test]
fn unreleased_changelog_claims_use_the_current_directive_vocabulary() {
    let changelog = include_str!("../CHANGELOG.md");
    let unreleased = changelog
        .split_once("\n## [0.7.0]")
        .map_or(changelog, |(unreleased, _)| unreleased);
    let stale_claims = [
        "Named mapping invocations: `<!-- choices-single -->`",
        "`<!-- choices-single -->`, `<!-- choices-multiple -->`) now sits on",
        "generated decks declare `tasklist: choices-single` themselves",
        "a `choices-single` card whose correct option was quoted",
        "a `choices-multiple` card lost every",
    ];
    let stale: Vec<&str> = stale_claims
        .into_iter()
        .filter(|claim| unreleased.contains(claim))
        .collect();

    assert!(
        stale.is_empty(),
        "Unreleased presents spellings the current grammar does not define as current: {stale:?}"
    );
}
