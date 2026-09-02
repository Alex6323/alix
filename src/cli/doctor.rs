use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use alix::{
    deck::{AtRewrite, Deck},
    source::{CitationIntegrity, SourceBase},
    trace::{DriftKind, Trace},
    workspace,
};
use anyhow::{Context, Result, bail};

use crate::{
    DoctorArgs,
    common::{one_line, truncate},
};

fn val_name<T: clap::ValueEnum>(value: T) -> String {
    value
        .to_possible_value()
        .map(|p| p.get_name().to_string())
        .unwrap_or_default()
}

#[derive(Default)]
struct Report {
    errors: Vec<String>,
    warnings: Vec<String>,
    notes: Vec<String>,
}

impl Report {
    fn error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }
    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
    fn note(&mut self, msg: impl Into<String>) {
        self.notes.push(msg.into());
    }
    fn render(&self) -> bool {
        for e in &self.errors {
            eprintln!("error: {e}");
        }
        for w in &self.warnings {
            eprintln!("warning: {w}");
        }
        for note in &self.notes {
            eprintln!("note: {note}");
        }
        if !self.errors.is_empty() || !self.warnings.is_empty() {
            eprintln!(
                "{} error(s), {} warning(s)",
                self.errors.len(),
                self.warnings.len()
            );
        }
        !self.errors.is_empty()
    }
}

fn deck_findings(path: &Path, report: &mut Report) {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deck.md");
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            report.error(format!("{}: {e}", path.display()));
            return;
        }
    };
    let deck = match alix::parser::parse(name, &text) {
        Ok(deck) => deck,
        Err(e) => {
            if alix::parser::is_deck_content(&text) {
                report.error(format!("{}: {e}", path.display()));
            }
            return;
        }
    };
    let initialized = deck.deck_token.is_some();
    if !initialized && alix::parser::is_deck_content(&text) {
        report.warn(format!(
            "{}: deck-like Markdown is not initialized; run `alix deck init {}` if this is an intended deck",
            path.display(),
            path.display()
        ));
    }

    if deck.frontmatter.sampling.is_some() && deck.tables.is_empty() {
        report.warn(format!(
            "{}: `sampling:` has no effect here; the deck holds no card table",
            path.display()
        ));
    }
    for card in &deck.cards {
        if card.sampling.is_some() && card.row.is_none() {
            report.warn(format!(
                "{}: line {}: `sampling:` has no effect on a card that is not a table row",
                path.display(),
                card.line
            ));
            break;
        }
    }

    invisible_byte_findings(path, &text, report);

    let augment = Deck::load(path)
        .ok()
        .and_then(|deck| alix::augment::AugmentCache::open_for_deck(&deck).ok());
    for diagnostic in alix::math::diagnostics(&deck.cards, augment.as_ref()) {
        report.warn(format!("{}: {diagnostic}", path.display()));
    }
    inline_emphasis_findings(&deck.cards, report);
    span_anchor_findings(path, &deck.cards, report);

    for lint in &deck.lints {
        let msg = lint_message(path, lint);
        if matches!(lint.kind, alix::parser::LintKind::BadValue { .. }) {
            report.error(msg);
        } else {
            report.warn(msg);
        }
    }

    let mut tokens: Vec<String> = Vec::new();
    if let Some(t) = &deck.deck_token {
        tokens.push(t.clone());
    }
    for card in &deck.cards {
        if let Some(t) = card.token.as_deref()
            && !tokens.iter().any(|x| x == t)
        {
            tokens.push(t.to_string());
        }
    }
    for tok in &tokens {
        if let Some((_, token, _, _, _)) = alix::token::parse_id(tok)
            && !alix::token::is_canonical(token)
        {
            report.warn(format!(
                "{}: id `{tok}` is valid but its token is not canonical (not 26 base32 chars)",
                path.display()
            ));
        }
    }

    for line in alix::stamp::misplaced_id_markers(&text) {
        report.warn(format!(
            "{}: card id marker at line {line} is not the last line of its card; move it \
             there (the position stamping mints at)",
            path.display()
        ));
    }

    if deck.deck_token.is_none() && deck.frontmatter_span.is_some() && deck.frontmatter.unspliceable
    {
        report.warn(format!(
            "{}: cannot stamp: frontmatter is not a block mapping, so no `id:` can be spliced in",
            path.display()
        ));
    }

    // Cloze holes and a reversed twin share one heading; dedup by line so one
    // stamp isn't counted twice.
    let mut unstamped: Vec<usize> = deck
        .cards
        .iter()
        .filter(|c| c.token.is_none())
        .map(|c| c.line)
        .collect();
    unstamped.sort_unstable();
    unstamped.dedup();
    if initialized && !unstamped.is_empty() {
        report.warn(format!(
            "{}: {} entries are card content without ids; open the deck to assign them",
            path.display(),
            unstamped.len()
        ));
    }

    deck_diagram_findings(
        path,
        deck.deck_token.as_deref(),
        &deck.cards,
        &text,
        deck.frontmatter_span,
        report,
    );
    if let Ok(deck) = Deck::load(path) {
        deck_resource_findings(&deck, report);
    }
}

fn span_anchor_findings(path: &Path, cards: &[alix::card::Card], report: &mut Report) {
    let mut seen = HashSet::new();
    for card in cards {
        for region in &card.span_regions {
            let (Some(minted), Some(bound)) = (region.minted_position, region.bound_position)
            else {
                continue;
            };
            if minted == bound || !seen.insert(region.line) {
                continue;
            }
            let hidden = region.hidden.as_deref().unwrap_or_default();
            let keep_old = match region.minted_occurrence {
                Some(occurrence) => {
                    format!("; to keep the old target instead, set `occurrence={occurrence}`")
                }
                None => String::new(),
            };
            report.warn(format!(
                "{}:{} the span `{hidden}` binds at position:{bound} but its anchor says \
                 position:{minted}; keep the authored occurrence with `alix doctor \
                 --repair-positions` or set position:{bound} yourself{keep_old}",
                path.display(),
                region.line,
            ));
        }
    }
}

fn invisible_byte_findings(path: &Path, text: &str, report: &mut Report) {
    let found = alix::invisible::survey_prose(text.lines());
    if found.stray_tags > 0 {
        report.warn(format!(
            "{}: {} tag character(s) outside a flag sequence: they encode invisible text",
            path.display(),
            found.stray_tags
        ));
    }
    if let Some(line) = alix::invisible::first_fenced_reversal_override(text) {
        report.warn(format!(
            "{}: line {line}: a fence contains bidirectional override characters; \
             rendered order may differ from stored order",
            path.display()
        ));
    }
    let named: Vec<String> = found
        .counts
        .iter()
        .filter(|(label, _)| **label != "TAG")
        .map(|(label, n)| format!("{label} {n}"))
        .collect();
    if !named.is_empty() {
        report.note(format!(
            "{}: invisible characters kept as authored: {} (rendered, never graded)",
            path.display(),
            named.join(", ")
        ));
    }
}

fn inline_emphasis_findings(cards: &[alix::card::Card], report: &mut Report) {
    let mut seen_lines = HashSet::new();
    for card in cards {
        if !seen_lines.insert(card.line) {
            continue;
        }
        let emphasized = std::iter::once(&card.front)
            .chain(card.back.iter())
            .flat_map(|text| alix::inline::parse_inline(text))
            .find(|run| run.bold || run.italic);
        if let Some(run) = emphasized {
            let front = truncate(&one_line(&card.front), 72);
            let snippet = truncate(&one_line(&run.text), 72);
            report.note(format!(
                "inline emphasis will render in \"{front}\": {snippet} \
                 (escape or backtick if unintended)"
            ));
        }
    }
}

/// `source:` must point at real material, never into `assets/`: asset objects
/// are excerpt fragments, and an examiner or tutor grounded on fragments would
/// confidently fill the gaps around them.
fn source_points_into_assets(source: &str) -> bool {
    if alix::deck::is_url(source) {
        return false;
    }
    let root = format!("{}/", alix::assets::ROOT);
    let source = source.trim();
    source.starts_with(&root) || source.contains(&format!("/{root}"))
}

/// Diagram stamps and fences must agree with each other and with their
/// frozen objects. Standalone decks are exempt: they never freeze, and a
/// bare fence there is the designed fallback, not a finding.
fn deck_diagram_findings(
    path: &Path,
    deck_token: Option<&str>,
    cards: &[alix::card::Card],
    text: &str,
    frontmatter_span: Option<(usize, usize)>,
    report: &mut Report,
) {
    let Some(deck_token) = deck_token else {
        return;
    };
    if workspace::root_for_deck(path).is_none() {
        return;
    }
    let found = alix::diagram::fences_in_document(text, frontmatter_span);
    if found.unclosed {
        report.warn(format!(
            "{}: an unclosed mermaid fence cannot be frozen",
            path.display()
        ));
    }
    // A stamp ATTACHED to a fence belongs to it (freeze replaces it in
    // place when stale); a stamp attached to nothing is an orphan only
    // `--repair-diagrams` can remove. The same per-attachment freshness
    // accounting decides the deck-load warning.
    let attached = alix::diagram::attached_stamp_freshness(&found, text);
    for stamp in cards.iter().flat_map(|card| &card.diagrams) {
        match attached.get(&stamp.line) {
            None => report.warn(format!(
                "{}:{}: diagram stamp `{}` is attached to no fence; `alix doctor --repair-diagrams` removes it",
                path.display(),
                stamp.line,
                stamp.fingerprint
            )),
            Some(false) => report.warn(format!(
                "{}:{}: diagram stamp `{}` is stale (its fence was edited); re-run `alix deck init {}` to re-freeze",
                path.display(),
                stamp.line,
                stamp.fingerprint,
                path.display()
            )),
            Some(true) => {
                if let Err(error) = geometry_agreement(path, deck_token, stamp) {
                    report.error(format!(
                        "{}:{}: frozen diagram is inconsistent: {error:#}",
                        path.display(),
                        stamp.line
                    ));
                }
            }
        }
    }
    let unfrozen = found
        .fences
        .iter()
        .filter(|fence| fence.stamp.is_none())
        .count();
    if unfrozen > 0 {
        report.warn(format!(
            "{}: {unfrozen} mermaid fence(s) not frozen; run `alix deck init {}` (needs {})",
            path.display(),
            path.display(),
            alix::diagram::COMMAND
        ));
    }
    // Review silently falls back on a span that cannot project; doctor is
    // the loud channel and judges from the STAMPED geometry itself, so a
    // geometry the load path already refused still gets its precise
    // finding. Records ride every card of a block, so findings dedupe.
    let mut bind_findings: std::collections::BTreeSet<String> = Default::default();
    for card in cards {
        for fence in &card.answer_fences {
            let Some(stamp) = card
                .diagrams
                .iter()
                .find(|stamp| stamp.fingerprint == fence.fingerprint)
            else {
                continue;
            };
            let Ok(geometry) = stamped_geometry(path, deck_token, stamp) else {
                // geometry_agreement above already speaks for unreadable or
                // disagreeing objects.
                continue;
            };
            if let Err(failure) = alix::diagram::validate_label_sources(&geometry, &fence.interior)
            {
                bind_findings.insert(format!(
                    "{}: frozen diagram {}: {failure}; re-run `alix deck init {}` to re-freeze",
                    path.display(),
                    fence.fingerprint,
                    path.display()
                ));
                continue;
            }
            for span in &fence.spans {
                if let Err(failure) =
                    alix::diagram::bind_span(&geometry, span.line, span.start, span.end)
                {
                    bind_findings.insert(format!("{}: {failure}", path.display()));
                }
            }
        }
    }
    for finding in bind_findings {
        report.warn(finding);
    }
}

fn stamped_geometry(
    path: &Path,
    deck_id: &str,
    stamp: &alix::card::DiagramStamp,
) -> anyhow::Result<alix::diagram::DiagramGeometry> {
    let root = workspace::root_for_deck(path).context("not a workspace member")?;
    let owned = alix::assets::deck_dir(&root, deck_id)?;
    let bytes = std::fs::read(owned.join(&stamp.geometry))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// The stamp names the raster twice: directly (`asset:`) and through the
/// geometry's `image` field. Both routes must name the same object, and the
/// geometry must parse, or serving would pair a raster with the wrong map.
fn geometry_agreement(
    path: &Path,
    deck_id: &str,
    stamp: &alix::card::DiagramStamp,
) -> anyhow::Result<()> {
    let root = workspace::root_for_deck(path).context("not a workspace member")?;
    let owned = alix::assets::deck_dir(&root, deck_id)?;
    if !owned.join(&stamp.asset).is_file() {
        anyhow::bail!("image `{}` is missing from the deck's assets", stamp.asset);
    }
    // Content addressing is only a guarantee if someone re-hashes: existence
    // and name agreement pass for an object whose bytes were replaced.
    for name in [&stamp.asset, &stamp.geometry] {
        alix::assets::verify_object(&owned.join(name))
            .with_context(|| format!("object `{name}`"))?;
    }
    let bytes = std::fs::read(owned.join(&stamp.geometry))
        .with_context(|| format!("geometry `{}` cannot be read", stamp.geometry))?;
    let geometry: alix::diagram::DiagramGeometry = serde_json::from_slice(&bytes)
        .with_context(|| format!("geometry `{}` does not parse", stamp.geometry))?;
    if geometry.image != stamp.asset {
        anyhow::bail!(
            "the geometry names image `{}` but the stamp says `{}`",
            geometry.image,
            stamp.asset
        );
    }
    Ok(())
}

fn deck_resource_findings(deck: &Deck, report: &mut Report) {
    let managed = workspace::root_for_deck(&deck.path).is_some() && deck.deck_token.is_some();
    for source in &deck.sources {
        if source_points_into_assets(source) {
            report.warn(format!(
                "{}: `source:` points into `assets/` (`{source}`); a frozen deck keeps its \
                 real source, not its asset objects",
                deck.path.display()
            ));
        }
    }
    let mut frozen_assets_valid = true;
    let mut has_live_citations = false;
    if managed && !deck.sources.is_empty() {
        let live = deck
            .cards
            .iter()
            .flat_map(|card| &card.citations)
            .filter(|citation| citation.asset.is_none())
            .count();
        if live > 0 {
            report.error(format!(
                "{}: {live} live `at:` citation(s) in an initialized workspace member; initialize or update it to freeze deck-owned excerpts",
                deck.path.display()
            ));
            has_live_citations = true;
        }
        if let Err(error) = alix::assets::validate_member(deck) {
            report.error(format!(
                "{}: frozen source assets are invalid: {error:#}",
                deck.path.display()
            ));
            frozen_assets_valid = false;
        }
    }
    if managed && let Ok(text) = std::fs::read_to_string(&deck.path) {
        for image in alix::parser::image_references(&text) {
            if alix::deck::is_url(&image.source) {
                continue;
            }
            if let Err(error) = alix::assets::validate_image(deck, &image.source) {
                report.error(format!(
                    "{}: image `{}` is not a valid deck-owned asset: {error:#}",
                    deck.path.display(),
                    image.source
                ));
            }
        }
    }
    for card in &deck.cards {
        for image in card.images.iter().chain(&card.images_back) {
            if !image.src.exists() {
                report.warn(format!(
                    "{}: card at line {} references a missing image: {}",
                    deck.subject,
                    card.line,
                    image.src.display()
                ));
            }
        }
    }
    for drift in alix::trace::drifted_cards(deck) {
        let what = match drift.kind {
            DriftKind::SourceGone => "source file is gone".to_string(),
            DriftKind::Rewritten => "no longer found in the source".to_string(),
            DriftKind::Ambiguous => "occurs more than once, so its lines are unclear".to_string(),
            DriftKind::Moved { start, len } => format!(
                "is intact but now at lines {start}-{}; `--repair-source-locators` rebases it",
                start + len.saturating_sub(1)
            ),
        };
        report.warn(format!(
            "{}: card at line {}: frozen excerpt {} ({})",
            deck.subject, drift.line, what, drift.at
        ));
    }
    if deck.is_trace() && !deck.cards.is_empty() {
        match Trace::from_deck(deck) {
            Ok(trace) => {
                for issue in trace.lint_locators() {
                    let line = deck.cards.get(issue.checkpoint).map_or(0, |c| c.line);
                    report.warn(format!(
                        "{}: checkpoint at line {}: {}",
                        deck.subject, line, issue.message
                    ));
                }
            }
            Err(e) => report.warn(format!("{}: {e:#}", deck.subject)),
        }
    }
    if frozen_assets_valid {
        let base = SourceBase::for_deck(deck);
        // One authored `at:` line serves every card its block produced (a
        // table's rows, a cloze block's holes), so the report is per authored
        // citation. Its identity is its own directive line: a card may carry
        // several `at:` lines, and keying on the card would let a healthy one
        // hide a stale sibling.
        let mut reported = std::collections::HashSet::new();
        for card in &deck.cards {
            for citation in &card.citations {
                if !reported.insert(citation.line) {
                    continue;
                }

                let counted_by_the_live_error = has_live_citations && citation.asset.is_none();
                let detail = match base.inspect_citation(citation) {
                    Ok(CitationIntegrity::Current(_)) => None,
                    Ok(CitationIntegrity::Unfingerprinted { .. }) => (!counted_by_the_live_error)
                        .then(|| {
                            "has no excerpt fingerprint; review it, then run \
                             `alix doctor --repair-source-locators`"
                                .to_string()
                        }),
                    Ok(CitationIntegrity::Relocated { locator, .. }) => Some(format!(
                        "the exact excerpt moved to `{locator}`; run \
                         `alix doctor --repair-source-locators` to rebase it"
                    )),
                    Ok(CitationIntegrity::Changed) => {
                        Some("the excerpt changed or disappeared; review it manually".to_string())
                    }
                    Ok(CitationIntegrity::Ambiguous { locators }) => Some(format!(
                        "the excerpt matches several ranges ({}); review it manually",
                        locators.join(", ")
                    )),
                    Err(error) => Some(format!("{error:#}")),
                };
                if let Some(detail) = detail {
                    report.warn(format!(
                        "{}: card at line {}: `at: {}` {detail}",
                        deck.subject, card.line, citation.locator
                    ));
                }
            }
        }
    }
    if deck.sources.len() > 3 {
        report.warn(format!(
            "{}: `source:` lists {} expressions; point at their common root instead \
             (a directory source covers its files)",
            deck.path.display(),
            deck.sources.len()
        ));
    }
    for prereq in alix::deck::nongating_prerequisites(deck) {
        report.warn(format!(
            "{}: requires ungrounded `{prereq}`: this edge doesn't gate its exam; \
             give it a `source:` (deck or workspace) to make it a real prerequisite",
            deck.subject
        ));
    }
    let dir = deck.path.parent();
    let unparseable_prereq = |report: &mut Report, req: &str, path: &Path| {
        if Deck::load(path).is_err() {
            report.warn(format!(
                "{}: requires `{req}` but that deck fails to parse; its errors name the fix",
                deck.subject
            ));
        }
    };
    for req in &deck.requires {
        match alix::deck::classify_require(req) {
            alix::deck::RequiresMode::DeckId => {
                match alix::deck::resolve_dep_by_id(req, dir, dir) {
                    None => report.warn(format!(
                        "{}: requires deck id `{req}` but no deck here carries it \
                         (dangling prerequisite); the prerequisite may have been deleted \
                         or live elsewhere; if you meant a file of that name, write the \
                         `.md` extension (`{req}.md`)",
                        deck.subject
                    )),
                    Some(path) => unparseable_prereq(report, req, &path),
                }
                if let Some(shadow) = alix::deck::resolve_dep(req, dir, dir) {
                    report.warn(format!(
                        "{}: the file {} is named like the required deck id `{req}`; \
                         the id wins, the file is not the prerequisite (write `{req}.md` \
                         to require the file itself)",
                        deck.subject,
                        shadow.display()
                    ));
                }
            }
            alix::deck::RequiresMode::WrongTypeCardId => {
                report.error(format!(
                    "{}: requires `{req}` which is a card id, likely pasted by mistake; \
                     a card is never a prerequisite, name the deck's `deck-<token>` id or \
                     its filename",
                    deck.subject
                ));
            }
            alix::deck::RequiresMode::Filename => match alix::deck::resolve_dep(req, dir, dir) {
                None => {
                    report.warn(format!(
                        "{}: requires `{req}` but no such deck exists here (dangling \
                             prerequisite); a filename edge breaks when the prerequisite is \
                             renamed or deleted (a `deck-<token>` id edge survives renames)",
                        deck.subject
                    ));
                    if req.starts_with("deck-") {
                        report.note(format!(
                            "{}: `requires: {req}` looks like a truncated or malformed \
                                 deck id (an id is `deck-` plus 26 base32 chars); it was read \
                                 as a filename",
                            deck.subject
                        ));
                    }
                }
                Some(path) => unparseable_prereq(report, req, &path),
            },
        }
    }
}

fn lint_message(path: &Path, lint: &alix::parser::Lint) -> String {
    use alix::parser::LintKind;
    let detail = match &lint.kind {
        LintKind::UnknownKey { key } => format!("unknown key `{key}` (ignored)"),
        LintKind::BadValue { key, value } => format!("`{key}` has an invalid value `{value}`"),
        LintKind::EmptyValue { key } => format!("`{key}` has an empty value"),
        LintKind::IndentedH2 => {
            "an indented `##` line is content, not a card front (likely a mistype)".to_string()
        }
        LintKind::NoteContainsBlankAnswer { blank, answer } => format!(
            "the note contains the text of blank {blank}'s answer (`{answer}`); \
             the block's other cards show this note"
        ),
        LintKind::NoteNamesNoBlank { name } => format!(
            "`{name}:` names no named blank of this block, so the line is \
             shown as the block note"
        ),
        LintKind::UntypableSpan { answer } => format!(
            "the hidden span `{answer}` asks the learner to type a LaTeX \
             command; hide a piece that can be typed, or let it be drawn"
        ),
        LintKind::UnclosedComment => {
            "a `<!--` line that never closes with `-->` stays content".to_string()
        }
        LintKind::UnrecognizedComment => "a deck's `<!-- -->` is alix machinery, and this \
             one is not recognized, so it is ignored; write a directive (`key: value`), a \
             locator, or an invocation (`plain`, `cards`, `choices: single`, \
             `choices: multiple`), and put prose in `description:` or a card"
            .to_string(),
        LintKind::BadgeShape { text } => format!(
            "`{text}` is not one of the five alert badges, so this blockquote is a quote \
             and not a note; write `[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, or \
             `[!CAUTION]` alone on the line"
        ),
        LintKind::EmptyNote => "this alert badge opens a note with no body, so the card \
             shows nothing for it"
            .to_string(),
        LintKind::UnclosedFence => "a fence opened here never closes; everything after it \
             (cards included) was swallowed as its content"
            .to_string(),
        LintKind::ImageMalformed => "an image embed here is malformed; write \
             `![alt](file.png)` (or escape a literal `![` as `\\![`)"
            .to_string(),
        LintKind::UndecidedTable => "this table declares no mapping; add \
             `<!-- cards -->` below it to make each row a card, or \
             `<!-- plain -->` to keep it a literal table"
            .to_string(),
        LintKind::ChoiceAnswerMixed => {
            "task-list choices are mixed with other answer content; treating the card as plain"
                .to_string()
        }
        LintKind::ChoiceNeedsBothSides => {
            "a checkbox card needs one checked answer and at least one unchecked distractor"
                .to_string()
        }
        LintKind::DuplicateChoiceOption => {
            "a checkbox option repeats earlier option text; keeping the first".to_string()
        }
        LintKind::ChoiceMultiCorrectUnsupported => {
            "multiple checked answers are not supported yet; treating the card as plain".to_string()
        }
        LintKind::ChoiceNoteNamesPosition => {
            "the note identifies an option by number, letter, or position, but choices are \
             shuffled; name the option's claim or mistaken premise instead"
                .to_string()
        }
    };
    format!("{}: line {}: {detail}", path.display(), lint.line)
}

fn sidecar_findings(dir: &Path, report: &mut Report) {
    let paths = alix::workspace::listing_with_sidecars(dir).unwrap_or_default();
    let mut entries = Vec::new();
    for path in &paths {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        entries.push(alix::FileEntry {
            name: name.to_string(),
            deck_id: alix::parser::deck_identity(&text).ok().flatten(),
            personal_for: alix::parser::personal_parent(&text).ok().flatten(),
            card_ids: alix::parser::parse_str(name, &alix::parser::without_notes(&text))
                .unwrap_or_default()
                .iter()
                .filter_map(alix::card::Card::id)
                .collect(),
        });
    }

    for finding in alix::classify(&entries).1 {
        report.warn(match finding {
            alix::Finding::ParentMissing { file } => {
                format!("{file}: names a deck that is not in this folder")
            }
            alix::Finding::ParentMismatch {
                file,
                named,
                neighbour,
            } => format!(
                "{file}: `{key}: {named}` but the deck it sits beside is {neighbour}",
                key = alix::parser::PERSONAL_PARENT_KEY
            ),
            alix::Finding::DuplicateCardId {
                deck,
                sidecar,
                card,
            } => format!("{sidecar}: card `{card}` is already in {deck}; one schedule, two cards"),
            alix::Finding::SuffixMissing { file } => {
                format!(
                    "{file}: carries `{key}:` but is not named `<deck>.personal.md`",
                    key = alix::parser::PERSONAL_PARENT_KEY
                )
            }
        });
    }

    for path in paths.iter().filter(|path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(alix::workspace::is_sidecar_name)
    }) {
        orphan_note_findings(path, report);
    }
}

fn orphan_note_findings(sidecar: &Path, report: &mut Report) {
    let deck_path = sidecar.with_file_name(
        sidecar
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".personal.md"))
            .map(|stem| format!("{stem}.md"))
            .unwrap_or_default(),
    );
    let Ok(deck) = Deck::load(&deck_path) else {
        return;
    };
    let personal = alix::personal::read(&deck_path, &deck.subject);
    let deck_blocks: Vec<alix::DeckCard> = deck
        .cards
        .iter()
        .filter_map(|card| {
            Some(alix::DeckCard {
                id: card.id()?,
                notes: Vec::new(),
            })
        })
        .collect();
    let (_, orphans) = alix::merge(&deck_blocks, &personal.blocks());
    for orphan in orphans {
        report.warn(format!(
            "{}: a note addresses `{}`, which is in neither the deck nor this file",
            sidecar.display(),
            orphan.card
        ));
    }
}

fn workspace_findings(dir: &Path) -> Report {
    let mut report = Report::default();
    for root in alix::workspace::roots_under(dir) {
        let nested = findings_in(&root);
        report.errors.extend(nested.errors);
        report.warnings.extend(nested.warnings);
        report.notes.extend(nested.notes);
    }
    report
}

fn findings_in(dir: &Path) -> Report {
    let mut report = Report::default();
    if alix::workspace::has_manifest(dir)
        && let Err(error) = alix::workspace::Workspace::load(dir)
    {
        report.error(format!("{error}"));
    }
    for path in alix::workspace::misplaced_deck_files(dir).unwrap_or_default() {
        report.warn(format!(
            "{}: workspace decks belong in {}; this file is not discovered",
            path.display(),
            dir.join(alix::workspace::DECKS).display()
        ));
    }
    let found = alix::workspace::classify_deck_files(dir).unwrap_or_default();
    let (deck_files, uninitialized) = (found.initialized, found.uninitialized);
    for path in &deck_files {
        deck_findings(path, &mut report);
    }
    for path in uninitialized {
        deck_findings(&path, &mut report);
    }

    sidecar_findings(dir, &mut report);

    let map = alix::dedup::scan_dir(dir);
    for (kept, excluded, token) in &map.excluded_decks {
        report.warn(format!(
            "duplicate deck token `{token}`: {} is excluded (kept {}); delete the `id:` line in the copy",
            excluded.display(),
            kept.display()
        ));
    }
    for dupe in &map.card_dupes {
        let losers: Vec<String> = dupe
            .losers
            .iter()
            .map(|(p, l)| format!("{}:{}", p.display(), l))
            .collect();
        report.warn(format!(
            "duplicate card token `{}`: {}:{} keeps the progress; also at {}",
            dupe.base,
            dupe.keeper.0.display(),
            dupe.keeper.1,
            losers.join(", ")
        ));
    }

    let store_path = alix::workspace::root_store_path(dir);
    let mut known_deck_ids = HashSet::new();
    for path in &deck_files {
        if let Ok(deck) = Deck::load(path)
            && let Some(deck_id) = deck.deck_token
        {
            known_deck_ids.insert(deck_id);
        }
    }
    match alix::state::open_aggregate_store(&store_path) {
        Ok(store) => {
            let mut known_cards: HashSet<String> = HashSet::new();
            let mut any_fresh = false;
            for path in &deck_files {
                if let Ok(deck) = Deck::load(path) {
                    for card in &deck.cards {
                        if let Some(id) = card.id() {
                            if store.get(&id).is_none() {
                                any_fresh = true;
                            }
                            known_cards.insert(id);
                        }
                    }
                    known_cards.extend(alix::personal::card_ids(&deck));
                    known_cards.extend(deck.dormant_base_ids());
                }
            }
            let orphans = store.orphans(&known_cards, &known_deck_ids);
            for key in &orphans.cards {
                report.warn(format!(
                    "orphaned store key (card) `{key}` matches no card in {}",
                    dir.display()
                ));
            }
            for key in &orphans.decks {
                report.warn(format!(
                    "orphaned store key (deck) `{key}` matches no deck in {}",
                    dir.display()
                ));
            }
            if !orphans.cards.is_empty() && any_fresh {
                report.warn(
                    "orphaned card progress exists and fresh tokens were minted: a card may have \
                 lost its `<!-- id: -->` comment (e.g. a formatter stripped it) and been \
                 re-stamped, orphaning its old progress; the old progress stays until you run \
                 `alix reset --orphans`"
                        .to_string(),
                );
            }
        }
        Err(error) => report.error(format!("user files: {error}")),
    }
    if let Err(error) = alix::augment::AugmentCache::open_for_workspace(dir) {
        report.error(format!("workspace augmentation: {error}"));
    }
    let user_files = alix::state::UserFiles::new(&store_path);
    let workspace_files = alix::workspace::WorkspaceFiles::new(dir);
    for (kind, data_dir) in [
        ("progress", user_files.progress()),
        ("augmentation", workspace_files.augment()),
    ] {
        for entry in std::fs::read_dir(data_dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if !path.is_file()
                || path.extension().is_none_or(|extension| extension != "json")
                || alix::workspace::is_conflict_name(name)
            {
                continue;
            }
            let Some(deck_id) = alix::state::deck_id_from_document(&path) else {
                report.warn(format!("unrecognized {kind} document {}", path.display()));
                continue;
            };
            let document_error = match kind {
                "progress" => alix::store::Store::open_deck(&path, deck_id, "")
                    .err()
                    .map(|error| error.to_string()),
                _ => alix::augment::AugmentCache::open_deck(&path, deck_id)
                    .err()
                    .map(|error| error.to_string()),
            };
            if let Some(error) = document_error {
                report.error(format!("{kind} document: {error}"));
                continue;
            }
            if !known_deck_ids.contains(deck_id) {
                report.warn(format!(
                    "orphaned {kind} document {} belongs to no deck in {}",
                    path.display(),
                    dir.display()
                ));
            }
        }
    }
    for conflict in alix::store::sync_conflicts(&store_path) {
        report.warn(format!(
            "synchronization conflict needs deliberate recovery: {}",
            conflict.display()
        ));
    }
    for conflict in alix::augment::sync_conflicts(dir) {
        report.warn(format!(
            "synchronization conflict needs deliberate recovery: {}",
            conflict.display()
        ));
    }
    report
}

fn check(decks: Vec<PathBuf>) -> Result<()> {
    let mut report = Report::default();
    for path in &decks {
        // Deck::load would error on a directory, so a workspace target is
        // handled separately here.
        if path.is_dir() && path.join(alix::workspace::MANIFEST).is_file() {
            if let Some(rel) = alix::workspace::manifest_icon(path)
                && !path.join(&rel).is_file()
            {
                report.warn(format!(
                    "{}: `icon = \"{rel}\"` points at a missing file",
                    path.display()
                ));
            }
            for complaint in alix::config::local_review_lint(path) {
                report.warn(format!("{}: {complaint}", path.display()));
            }
            continue;
        }
        if let Ok(deck) = Deck::load(path) {
            println!("{}: {} cards", deck.subject, deck.cards.len());
            if std::fs::read_to_string(path).is_ok_and(|text| {
                matches!(alix::parser::normalize(&text), std::borrow::Cow::Owned(_))
            }) {
                report.warn(format!(
                    "{}: not in canonical bytes; run `alix doctor <dir-or-deck> --normalize`",
                    path.display()
                ));
            }
            let s = &deck.settings;
            let declared: Vec<String> = [
                s.reveal.map(|r| format!("reveal: {}", val_name(r))),
                s.order.map(|o| format!("review: {}", val_name(o))),
                s.exam_strictness
                    .map(|v| format!("strictness: {}", val_name(v))),
            ]
            .into_iter()
            .flatten()
            .collect();
            if !declared.is_empty() {
                println!("  settings: {}", declared.join(", "));
            }
            if !deck.requires.is_empty() {
                println!("  requires: {}", deck.requires.join(", "));
            }
            if !deck.sources.is_empty() {
                println!("  sources:  {}", deck.sources.join(", "));
            }
            if let Some(desc) = &deck.trace {
                println!("  trace:    {desc}");
            }
        }
        deck_findings(path, &mut report);
    }
    if report.render() {
        bail!("{} error(s) found", report.errors.len());
    }
    Ok(())
}

/// The parsed `position:` token's byte offset in a directive line: outside
/// quoted values (a hidden answer may look exactly like the token) and
/// bounded so `position:1` never matches inside `position:13`.
fn anchor_token_offset(line: &str, token: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut escaped = false;
    for (at, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            _ if in_quote => {}
            _ => {
                if line[at..].starts_with(token)
                    && (at == 0 || line[..at].ends_with(char::is_whitespace))
                    && !line[at + token.len()..].starts_with(|c: char| c.is_ascii_digit())
                {
                    return Some(at);
                }
            }
        }
    }
    None
}

fn repair_positions(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        let deck = Deck::load(path)?;
        let mut edits: Vec<(usize, u32, u32)> = Vec::new();
        let mut seen = HashSet::new();
        for card in &deck.cards {
            for region in &card.span_regions {
                if let (Some(minted), Some(bound)) = (region.minted_position, region.bound_position)
                    && minted != bound
                    && seen.insert(region.line)
                {
                    edits.push((region.line, minted, bound));
                }
            }
        }
        if edits.is_empty() {
            continue;
        }
        let text = std::fs::read_to_string(path)?;
        let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
        for (line, minted, bound) in &edits {
            let Some(raw) = lines.get_mut(line - 1) else {
                bail!("{}:{line} disappeared while repairing", path.display());
            };
            let old_token = format!("position:{minted}");
            let Some(at) = anchor_token_offset(raw, &old_token) else {
                bail!(
                    "{}:{line} lost `{old_token}` while repairing",
                    path.display()
                );
            };
            raw.replace_range(at..at + old_token.len(), &format!("position:{bound}"));
            println!(
                "rebased {}:{line} position:{minted} -> position:{bound}",
                path.display()
            );
        }
        let repaired: String = lines.concat();
        alix::parser::parse(
            path.file_stem().and_then(|s| s.to_str()).unwrap_or("deck"),
            &repaired,
        )
        .with_context(|| format!("{}: the repair would not parse", path.display()))?;
        alix::deck::write_deck_text(path, &repaired)?;
    }
    Ok(())
}

fn normalize_decks(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        let text = std::fs::read_to_string(path)?;
        let std::borrow::Cow::Owned(normalized) = alix::parser::normalize(&text) else {
            continue;
        };
        alix::parser::parse(
            path.file_stem().and_then(|s| s.to_str()).unwrap_or("deck"),
            &normalized,
        )
        .with_context(|| format!("{}: the normalized deck would not parse", path.display()))?;
        alix::deck::write_deck_text(path, &normalized)?;
        println!("normalized {}", path.display());
    }
    Ok(())
}

fn repair_frontmatter_order(paths: &[PathBuf]) -> Result<()> {
    use alix::parser::Reorder;
    for path in paths {
        let text = std::fs::read_to_string(path)?;
        match alix::parser::reorder_frontmatter(&text) {
            Reorder::Unchanged => {}
            Reorder::Reordered(repaired) => {
                alix::parser::parse(
                    path.file_stem().and_then(|s| s.to_str()).unwrap_or("deck"),
                    &repaired,
                )
                .with_context(|| format!("{}: the repair would not parse", path.display()))?;
                alix::deck::write_deck_text(path, &repaired)?;
                println!("reordered frontmatter in {}", path.display());
            }
            Reorder::Skipped(reason) => {
                eprintln!(
                    "warning: {}: frontmatter left as-is: {reason}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

fn repair_comment_order(paths: &[PathBuf]) -> Result<()> {
    use alix::parser::Reorder;
    for path in paths {
        let text = std::fs::read_to_string(path)?;
        match alix::parser::reorder_card_comments(&text) {
            Reorder::Unchanged => {}
            Reorder::Reordered(repaired) => {
                alix::parser::parse(
                    path.file_stem().and_then(|s| s.to_str()).unwrap_or("deck"),
                    &repaired,
                )
                .with_context(|| format!("{}: the repair would not parse", path.display()))?;
                alix::deck::write_deck_text(path, &repaired)?;
                println!("reordered comment machinery in {}", path.display());
            }
            Reorder::Skipped(reason) => {
                eprintln!("warning: {}: comments left as-is: {reason}", path.display());
            }
        }
    }
    Ok(())
}

fn repair_diagrams(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        let (removed, report) = alix::assets::repair_diagrams(path)?;
        if removed > 0 {
            println!(
                "removed {removed} orphan diagram stamp(s) from {}",
                path.display()
            );
        }
        if report.diagrams > 0 {
            println!(
                "re-froze {} diagram(s) in {}",
                report.diagrams,
                path.display()
            );
        }
        for warning in &report.diagram_warnings {
            eprintln!("warning: {}: {warning}", path.display());
        }
    }
    Ok(())
}

fn repair_source_locators(paths: &[PathBuf]) -> Result<()> {
    let mut unresolved = 0;
    for path in paths {
        let deck = Deck::load(path)?;
        let base = SourceBase::for_deck(&deck);
        let mut rewrites = Vec::new();
        let mut changed = false;
        // A frozen citation's fingerprint verifies the asset, not the source, so
        // `inspect_citation` cannot see that its `at:` range moved. Rebasing the
        // address while the evidence stays byte-identical is the one repair that
        // adds no claim: ADR 0026 forbids rewriting the frozen fingerprint, not
        // correcting the provenance lines.
        let moved: HashMap<String, (usize, usize)> = alix::trace::drifted_cards(&deck)
            .into_iter()
            .filter_map(|drift| match drift.kind {
                DriftKind::Moved { start, len } => Some((drift.at, (start, len))),
                _ => None,
            })
            .collect();
        for card in &deck.cards {
            for citation in &card.citations {
                if let Some((start, len)) = moved.get(&citation.locator) {
                    let (file, _) = alix::source::parse_locator(&citation.locator);
                    let at = alix::source::relocated_locator(file.as_deref(), *start, *len);
                    println!(
                        "rebased {}:{} `{}` -> `{at}`",
                        path.display(),
                        citation.line,
                        citation.locator
                    );
                    changed = true;
                    rewrites.push(AtRewrite {
                        at,
                        fingerprint: citation.fingerprint,
                        asset: citation.asset.clone(),
                        line: citation.line,
                    });
                    continue;
                }
                let (at, fingerprint) = match base.inspect_citation(citation)? {
                    CitationIntegrity::Current(_) => {
                        (citation.locator.clone(), citation.fingerprint)
                    }
                    CitationIntegrity::Unfingerprinted { fingerprint, .. } => {
                        println!(
                            "stamped {}:{} `{}`",
                            path.display(),
                            citation.line,
                            citation.locator
                        );
                        changed = true;
                        (citation.locator.clone(), Some(fingerprint))
                    }
                    CitationIntegrity::Relocated { locator, .. } => {
                        println!(
                            "rebased {}:{} `{}` -> `{locator}`",
                            path.display(),
                            citation.line,
                            citation.locator
                        );
                        changed = true;
                        (locator, citation.fingerprint)
                    }
                    CitationIntegrity::Changed => {
                        eprintln!(
                            "warning: {}:{} `{}` changed or disappeared; not repaired",
                            path.display(),
                            citation.line,
                            citation.locator
                        );
                        unresolved += 1;
                        (citation.locator.clone(), citation.fingerprint)
                    }
                    CitationIntegrity::Ambiguous { locators } => {
                        eprintln!(
                            "warning: {}:{} `{}` matches several ranges ({}); not repaired",
                            path.display(),
                            citation.line,
                            citation.locator,
                            locators.join(", ")
                        );
                        unresolved += 1;
                        (citation.locator.clone(), citation.fingerprint)
                    }
                };
                rewrites.push(AtRewrite {
                    at,
                    fingerprint,
                    asset: citation.asset.clone(),
                    line: citation.line,
                });
            }
        }
        if changed {
            alix::deck::set_source_citations(path, &rewrites)?;
        }
    }
    if unresolved > 0 {
        bail!("{unresolved} source citation(s) need manual review");
    }
    Ok(())
}

fn repair_after_explicit_path(path: Option<&Path>) -> bool {
    path.is_none_or(|path| !workspace::is_workspace(path))
}

// Exits non-zero only on a hard fail; a missing optional binary (a warn)
// never breaks a script.
pub(crate) fn doctor_cmd(args: DoctorArgs) -> Result<()> {
    use alix::doctor::{self, Status};
    let repair_after_explicit_path = repair_after_explicit_path(args.dir.as_deref());
    if let Some(path) = &args.dir {
        if path.is_file() {
            if args.normalize {
                normalize_decks(std::slice::from_ref(path))?;
            }
            if args.repair_source_locators {
                repair_source_locators(std::slice::from_ref(path))?;
            }
            if args.repair_positions {
                repair_positions(std::slice::from_ref(path))?;
            }
            if args.repair_diagrams {
                repair_diagrams(std::slice::from_ref(path))?;
            }
            if args.repair_frontmatter_order {
                repair_frontmatter_order(std::slice::from_ref(path))?;
            }
            if args.repair_comment_order {
                repair_comment_order(std::slice::from_ref(path))?;
            }
            return check(vec![path.clone()]);
        }
        if alix::workspace::is_workspace(path) {
            if args.normalize {
                normalize_decks(&alix::workspace::diagnosable_deck_files(path))?;
            }
            if args.repair_source_locators {
                repair_source_locators(&alix::workspace::diagnosable_deck_files(path))?;
            }
            if args.repair_positions {
                repair_positions(&alix::workspace::diagnosable_deck_files(path))?;
            }
            if args.repair_diagrams {
                repair_diagrams(&alix::workspace::diagnosable_deck_files(path))?;
            }
            if args.repair_frontmatter_order {
                repair_frontmatter_order(&alix::workspace::diagnosable_deck_files(path))?;
            }
            if args.repair_comment_order {
                repair_comment_order(&alix::workspace::diagnosable_deck_files(path))?;
            }
            check(vec![path.clone()])?;
        }
    }
    let (config_finding, config) = doctor::check_config(args.config.as_deref());
    let mut findings = vec![config_finding];
    let instance =
        crate::profile::instance_name_for_launch(args.config.as_deref(), args.dir.as_deref());
    findings.push(doctor::check_log(alix::log::log_path(&instance)));
    let (decks_dir, store_path) = match &args.dir {
        Some(path) => (path.clone(), workspace::root_store_path(path)),
        None => {
            let dir = config.decks_dir().context("cannot determine ~/decks")?;
            let store = workspace::root_store_path(&dir);
            (dir, store)
        }
    };
    if args.normalize && repair_after_explicit_path {
        normalize_decks(&alix::workspace::diagnosable_deck_files(&decks_dir))?;
    }
    if args.repair_source_locators && repair_after_explicit_path {
        repair_source_locators(&alix::workspace::diagnosable_deck_files(&decks_dir))?;
    }
    if args.repair_positions && repair_after_explicit_path {
        repair_positions(&alix::workspace::diagnosable_deck_files(&decks_dir))?;
    }
    if args.repair_diagrams && repair_after_explicit_path {
        repair_diagrams(&alix::workspace::diagnosable_deck_files(&decks_dir))?;
    }
    if args.repair_frontmatter_order && repair_after_explicit_path {
        repair_frontmatter_order(&alix::workspace::diagnosable_deck_files(&decks_dir))?;
    }
    if args.repair_comment_order && repair_after_explicit_path {
        repair_comment_order(&alix::workspace::diagnosable_deck_files(&decks_dir))?;
    }
    if args.remove_backup_files {
        let baks = doctor::backup_files(&decks_dir);
        if baks.is_empty() {
            println!("No backup files under {}.", decks_dir.display());
        } else {
            println!("{} backup file(s):", baks.len());
            for bak in &baks {
                println!("  {}", bak.display());
            }
            if crate::common::confirm(
                "Delete them? `alix deck restore` will then have nothing to swap in.",
                args.yes,
            )? {
                for bak in &baks {
                    std::fs::remove_file(bak)
                        .with_context(|| format!("cannot remove {}", bak.display()))?;
                }
                println!("Deleted {} backup file(s).", baks.len());
            } else {
                println!("Kept them.");
            }
        }
    }
    findings.push(doctor::check_store(Some(store_path)));
    findings.push(doctor::check_decks(&decks_dir));
    if let Some(finding) = doctor::check_backups(&decks_dir) {
        findings.push(finding);
    }
    findings.push(doctor::check_binary(
        "backend",
        &config.ask.command,
        "the AI features (tutor, exam, generate)",
        "install it and log in — or switch `[ask] backend` in the config",
    ));
    findings.push(doctor::check_binary(
        "share",
        "wormhole",
        "sharing (`alix share`/`receive`)",
        "install magic-wormhole (e.g. `pipx install magic-wormhole`, or your package manager)",
    ));
    findings.push(doctor::check_binary(
        "diagrams",
        alix::diagram::COMMAND,
        "rendering mermaid diagrams into decks",
        alix::diagram::REMEDY,
    ));
    let mut failed = false;
    for f in &findings {
        let glyph = match f.status {
            Status::Ok => "✓",
            Status::Warn => "!",
            Status::Fail => {
                failed = true;
                "✗"
            }
        };
        println!("{glyph} {:<8} {}", f.name, f.detail);
        if let Some(remedy) = &f.remedy {
            println!("  ↳ {remedy}");
        }
    }
    // A standalone deck-file target returned above and skips this: it stays
    // dedup-blind by design.
    if workspace_findings(&decks_dir).render() {
        failed = true;
    }
    if args.backends || args.all_backends {
        println!();
        alix::backend::health::check(&config.ask, args.all_backends)?;
    }
    // Probe outcomes never flip the exit code: they're a spot check on the
    // model, not a broken setup. Only an infrastructure error fails the run.
    if args.grading {
        println!();
        grading_spot_check(&config)?;
    }
    if failed {
        bail!("doctor found problems (✗ above)");
    }
    Ok(())
}

fn grading_spot_check(config: &alix::config::Config) -> Result<()> {
    use alix::calibrate::{self, ProbeKind};
    println!(
        "grading spot-check: {} probes, 3 real calls to the configured backend…",
        calibrate::PROBES.len()
    );
    let results = calibrate::run(&config.exam, &config.ask)?;
    let (mut safety_bad, mut fairness_bad) = (0, 0);
    for r in &results {
        let expect = match r.kind {
            ProbeKind::Safety => "must not pass",
            ProbeKind::Fairness => "should pass",
        };
        let glyph = if r.ok {
            "✓"
        } else {
            match r.kind {
                ProbeKind::Safety => {
                    safety_bad += 1;
                    "✗"
                }
                ProbeKind::Fairness => {
                    fairness_bad += 1;
                    "!"
                }
            }
        };
        println!("{glyph} {:<20} {expect}: got {:?}", r.name, r.verdict);
    }
    if safety_bad > 0 {
        println!(
            "✗ the model passed {safety_bad} answer(s) that must not pass: exam grades from \
             this model may be too lenient; consider a stronger `[ask]` model for grading"
        );
    } else if fairness_bad > 0 {
        println!(
            "! safe, but stricter than intended: {fairness_bad} should-pass probe(s) did not \
             pass. Passing an exam stays honest; it may just be harder than calibrated."
        );
    } else {
        println!("✓ grading looks trustworthy (a spot check, not a guarantee)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANCHOR_DECK_HEAD: &str =
        "---\nformat-version: 1\nid: \"deck-regionregionregionregion\"\n---\n\n";

    fn anchor_deck(dir: &tempfile::TempDir, body: &str) -> PathBuf {
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            format!(
                "{ANCHOR_DECK_HEAD}## q\n---\n{body}<!-- id: card-regionregionregionregionre -->\n"
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn kept_invisibles_draw_one_calm_note_and_nothing_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(&dir, "an\u{200B}swer with a soft\u{00AD}hyphen\n");
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        let note = report
            .notes
            .iter()
            .find(|n| n.contains("invisible"))
            .expect("one calm note names the invisible bytes");
        assert!(
            note.contains("ZSWP 1") || note.contains("ZWSP 1"),
            "counts by class: {note}"
        );
        assert!(note.contains("SHY 1"), "counts by class: {note}");
        assert!(
            report.warnings.iter().all(|w| !w.contains("invisible")),
            "legitimate invisibles never warn: {:?}",
            report.warnings
        );
    }

    #[test]
    fn an_emoji_rich_deck_draws_no_invisible_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(
            &dir,
            "\u{1F469}\u{200D}\u{1F680} and \u{1F44D}\u{FE0F} and \u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}\n",
        );
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        assert!(
            report.notes.iter().all(|n| !n.contains("invisible")),
            "an emoji-only deck is supposed to look like that: {:?}",
            report.notes
        );
    }

    #[test]
    fn stray_tag_characters_warn_about_invisible_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(&dir, "pay\u{E0067}\u{E0061}load\n");
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        assert!(
            report.warnings.iter().any(|w| w.contains("invisible text")),
            "tag characters outside a flag are the stealth shape: {:?}",
            report.warnings
        );
    }

    #[test]
    fn a_fence_holding_reversal_overrides_warns_and_embeddings_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(&dir, "```\nx\u{202E}y\n```\n");
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        assert!(
            report.warnings.iter().any(|w| w.contains("rendered order")),
            "the Trojan Source shape warns factually: {:?}",
            report.warnings
        );

        let calm = anchor_deck(&dir, "```\nx\u{202A}y\u{202C}\n```\n");
        let mut calm_report = Report::default();
        deck_findings(&calm, &mut calm_report);
        assert!(
            calm_report
                .warnings
                .iter()
                .all(|w| !w.contains("rendered order")),
            "embeddings in a fence are not the reversal shape: {:?}",
            calm_report.warnings
        );
    }

    #[test]
    fn explicit_workspace_repairs_are_not_scheduled_twice() {
        let workspace_dir = tempfile::tempdir().unwrap();
        std::fs::write(workspace_dir.path().join(workspace::MANIFEST), "").unwrap();
        let plain_dir = tempfile::tempdir().unwrap();

        assert!(!repair_after_explicit_path(Some(workspace_dir.path())));
        assert!(repair_after_explicit_path(Some(plain_dir.path())));
        assert!(repair_after_explicit_path(None));
    }

    /// Ruled D13(ii): a filename-named deck is sanctioned, so doctor must
    /// stay silent about a missing `title:`. A finding here would nag on
    /// 444 of the 621 initialized decks in the maintainer's own library.
    #[test]
    fn an_untitled_deck_draws_no_missing_title_finding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Eng-Sayings.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: \"deck-3nmmy2qkrw2trvvrmbm7ajry9t\"\n---\n## q <!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\na\n",
        )
        .unwrap();
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        let mentions_title = report
            .warnings
            .iter()
            .chain(report.errors.iter())
            .any(|line| line.to_lowercase().contains("title"));
        assert!(
            !mentions_title,
            "no title finding may exist: {:?} {:?}",
            report.warnings, report.errors
        );
    }

    #[test]
    fn a_diverged_anchor_is_reported_with_both_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(
            &dir,
            "target and target\n<!-- blank: span hidden=\"target\" occurrence=2 b:a1b2c3 position:1 -->\n",
        );
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        let warning = report
            .warnings
            .iter()
            .find(|w| w.contains("--repair-positions"))
            .expect("the divergence warns");
        assert!(warning.contains("position:12"), "{warning}");
        assert!(
            warning.contains("`occurrence=1`"),
            "the anchor still starts occurrence 1, so the keep-old-target edit prints: {warning}"
        );
    }

    #[test]
    fn a_stale_anchor_offers_only_the_keep_authored_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(
            &dir,
            "alpha target\n<!-- blank: span hidden=\"target\" b:a1b2c3 position:2 -->\n",
        );
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        let warning = report
            .warnings
            .iter()
            .find(|w| w.contains("--repair-positions"))
            .expect("the stale anchor warns");
        assert!(
            !warning.contains("occurrence="),
            "a stale offset has no old target to keep: {warning}"
        );
    }

    #[test]
    fn an_aligned_anchor_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(
            &dir,
            "prose target\n<!-- blank: span hidden=\"target\" b:a1b2c3 position:7 -->\n",
        );
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| w.contains("--repair-positions")),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn repair_positions_applies_the_keep_authored_edit_and_settles() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(
            &dir,
            "target and target\n<!-- blank: span hidden=\"target\" occurrence=2 b:a1b2c3 position:1 -->\n",
        );
        repair_positions(std::slice::from_ref(&path)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("occurrence=2 b:a1b2c3 position:12 -->"),
            "{text}"
        );
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| w.contains("--repair-positions")),
            "repair settles the anchor: {:?}",
            report.warnings
        );
    }

    #[test]
    fn repair_positions_rewrites_only_the_anchor_token_when_hidden_contains_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(
            &dir,
            "position:13 position:1\n<!-- blank: span hidden=\"position:1\" b:a1b2c3 position:1 -->\n",
        );

        repair_positions(std::slice::from_ref(&path)).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("<!-- blank: span hidden=\"position:1\" b:a1b2c3 position:13 -->"),
            "repair must preserve the authored hidden text and rewrite the parsed anchor: {text}"
        );
    }

    #[test]
    fn repair_positions_accepts_the_same_unicode_whitespace_as_region_parser() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(
            &dir,
            "target and target\n<!-- blank: span hidden=\"target\" occurrence=2 b:a1b2c3\u{a0}position:1 -->\n",
        );

        repair_positions(std::slice::from_ref(&path)).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("b:a1b2c3\u{a0}position:12 -->"), "{text}");
    }

    #[test]
    fn every_offered_keep_old_target_edit_keeps_the_deck_parseable() {
        let dir = tempfile::tempdir().unwrap();
        let path = anchor_deck(
            &dir,
            "target and target\n<!-- blank: span hidden=\"target\" occurrence=2 b:a1b2c3 position:1 -->\n<!-- blank: span hidden=\"target\" occurrence=1 b:d4e5f6 position:1 -->\n",
        );
        let mut report = Report::default();

        deck_findings(&path, &mut report);

        let warning = report
            .warnings
            .iter()
            .find(|warning| warning.contains("--repair-positions"))
            .expect("the first span diverges");
        if warning.contains("set `occurrence=1`") {
            let text = std::fs::read_to_string(&path).unwrap();
            let edited = text.replacen("occurrence=2", "occurrence=1", 1);
            alix::parser::parse("d", &edited).expect(
                "an edit doctor offers as the keep-old-target resolution must keep the deck parseable",
            );
        }
    }

    #[test]
    fn value_names_are_the_documented_clap_spellings() {
        assert_eq!("typeline", val_name(alix::answer::Mode::TypeLine));
        assert_eq!("sequential", val_name(alix::session::Order::Sequential));
    }

    #[test]
    fn report_render_child() {
        if std::env::var_os("ALIX_DOCTOR_REPORT_RENDER_CHILD").is_none() {
            return;
        }

        let mut errors = Report::default();
        errors.error("broken");
        assert!(errors.render());

        let mut warnings = Report::default();
        warnings.warn("risky");
        assert!(!warnings.render());

        let mut notes = Report::default();
        notes.note("informational");
        assert!(!notes.render());
    }

    #[test]
    fn report_summaries_cover_errors_and_warnings_but_not_notes() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "doctor::tests::report_render_child",
                "--nocapture",
            ])
            .env("ALIX_DOCTOR_REPORT_RENDER_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(stderr.contains("1 error(s), 0 warning(s)"), "{stderr}");
        assert!(stderr.contains("0 error(s), 1 warning(s)"), "{stderr}");
        assert_eq!(2, stderr.matches("error(s),").count(), "{stderr}");
    }

    /// A decks folder holds what the user puts there. Doctor reports a
    /// parse failure only for a file that claims to be a deck.
    #[test]
    fn a_parse_failure_is_reported_only_for_files_that_claim_deckhood() {
        for (name, text, reported) in [
            ("notes.md", "Just my notes.\n\nNothing about alix.\n", false),
            (
                "broken.md",
                "---\ntitle: Broken\n---\n\nstray prose\n\n## q\na\n",
                true,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            w(dir.path(), name, text);
            let mut report = Report::default();
            deck_findings(&dir.path().join(name), &mut report);
            assert_eq!(
                reported,
                !report.errors.is_empty(),
                "{name} should{} report, got {:#?}",
                if reported { "" } else { " not" },
                report.errors
            );
        }
    }

    #[test]
    fn initialized_deck_content_is_not_reported_as_uninitialized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ready.md");
        w(
            dir.path(),
            "ready.md",
            "---\nformat-version: 1\nid: deck-ready\n---\n## q <!-- id: card-ready -->\na\n",
        );

        let mut report = Report::default();
        deck_findings(&path, &mut report);

        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("deck-like Markdown is not initialized")),
            "{:#?}",
            report.warnings
        );
    }

    #[test]
    fn unique_noncanonical_card_tokens_are_still_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.md");
        w(
            dir.path(),
            "tokens.md",
            "---\nformat-version: 1\nid: deck-ready\n---\n## q\na\n<!-- id: card-short -->\n",
        );

        let mut report = Report::default();
        deck_findings(&path, &mut report);

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("id `card-short`")
                    && warning.contains("not canonical")),
            "{:#?}",
            report.warnings
        );
    }

    #[test]
    fn initialized_flow_frontmatter_is_not_reported_as_unspliceable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flow.md");
        w(
            dir.path(),
            "flow.md",
            "---\n{format-version: 1, id: deck-ready}\n---\n## q <!-- id: card-ready -->\na\n",
        );

        let mut report = Report::default();
        deck_findings(&path, &mut report);

        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("cannot stamp: frontmatter")),
            "{:#?}",
            report.warnings
        );
    }

    #[test]
    fn initialized_fully_stamped_decks_have_no_unstamped_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stamped.md");
        w(
            dir.path(),
            "stamped.md",
            "---\nformat-version: 1\nid: deck-ready\n---\n## q <!-- id: card-ready -->\na\n",
        );

        let mut report = Report::default();
        deck_findings(&path, &mut report);

        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("card content without ids")),
            "{:#?}",
            report.warnings
        );
    }

    #[test]
    fn only_local_paths_inside_assets_are_asset_sources() {
        assert!(source_points_into_assets("assets/deck-one/object.md"));
        assert!(source_points_into_assets(
            "workspace/assets/deck-one/object.md"
        ));
        assert!(!source_points_into_assets("notes/assets-overview.md"));
        assert!(!source_points_into_assets(
            "https://example.test/assets/deck-one/object.md"
        ));
    }

    #[test]
    fn trace_and_source_count_warnings_obey_their_exact_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let ordinary = dir.path().join("ordinary.md");
        w(dir.path(), "ordinary.md", "## q\na\n");
        let mut ordinary_report = Report::default();
        deck_findings(&ordinary, &mut ordinary_report);
        assert!(
            ordinary_report
                .warnings
                .iter()
                .all(|warning| !warning.contains("not a trace")),
            "{:#?}",
            ordinary_report.warnings
        );

        let three = dir.path().join("three.md");
        w(
            dir.path(),
            "three.md",
            "---\nsource: [a, b, c]\n---\n## q\na\n",
        );
        let four = dir.path().join("four.md");
        w(
            dir.path(),
            "four.md",
            "---\nsource: [a, b, c, d]\n---\n## q\na\n",
        );
        let mut report = Report::default();
        deck_findings(&three, &mut report);
        deck_findings(&four, &mut report);
        let common_root: Vec<&String> = report
            .warnings
            .iter()
            .filter(|warning| warning.contains("point at their common root"))
            .collect();
        assert_eq!(1, common_root.len(), "{:#?}", report.warnings);
        assert!(common_root[0].contains("four.md"), "{common_root:?}");
    }

    #[test]
    fn coarse_restamp_warning_needs_both_orphans_and_fresh_cards() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("facts.md");
        w(
            dir.path(),
            "facts.md",
            "---\nformat-version: 1\nid: deck-facts\n---\n## q\na\n<!-- id: card-live -->\n",
        );
        let mut store = alix::state::open_store(&deck, dir.path()).unwrap();
        store.get_or_insert("card-live");
        store.get_or_insert("card-orphan");
        store.save().unwrap();

        let report = workspace_findings(dir.path());

        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("fresh tokens were minted")),
            "{:#?}",
            report.warnings
        );
    }

    #[test]
    fn document_scan_skips_directories_non_json_files_and_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("progress/folder.json")).unwrap();
        w(&dir.path().join("progress"), "notes.txt", "not a document");
        w(
            &dir.path().join("progress"),
            "deck-x.sync-conflict-1.json",
            "not json",
        );

        let report = workspace_findings(dir.path());
        let findings = report
            .warnings
            .iter()
            .chain(&report.errors)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!findings.contains("folder.json"), "{findings}");
        assert!(!findings.contains("notes.txt"), "{findings}");
        assert!(
            !findings.contains("unrecognized progress document"),
            "{findings}"
        );
    }

    #[test]
    fn an_augmentation_document_in_progress_is_validated_as_progress() {
        let dir = tempfile::tempdir().unwrap();
        let progress = dir.path().join("progress/deck-orphan.json");
        alix::augment::AugmentCache::open_deck(&progress, "deck-orphan")
            .unwrap()
            .save()
            .unwrap();

        let report = workspace_findings(dir.path());

        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("progress document")),
            "{:#?}",
            report.errors
        );
    }

    #[test]
    fn plain_nested_directories_are_not_recursed_as_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("plain-folder");
        std::fs::create_dir(&nested).unwrap();
        w(&nested, "loose.md", "## q\na\n");

        let report = workspace_findings(dir.path());

        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("plain-folder")),
            "{:#?}",
            report.warnings
        );
    }

    #[test]
    fn check_rejects_a_directory_without_a_workspace_manifest() {
        let dir = tempfile::tempdir().unwrap();

        assert!(check(vec![dir.path().to_path_buf()]).is_err());
    }

    #[test]
    fn check_output_child() {
        if std::env::var_os("ALIX_DOCTOR_CHECK_OUTPUT_CHILD").is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let declared = dir.path().join("declared.md");
        w(
            dir.path(),
            "declared.md",
            "---\nformat-version: 1\nid: deck-declared\nreveal: line\nreview: sequential\nrequires: ghost\nsource: https://example.test/source\n---\n## q\na\n<!-- id: card-q -->\n",
        );
        let plain = dir.path().join("plain.md");
        w(dir.path(), "plain.md", "## q\na\n");
        let empty = dir.path().join("empty.md");
        w(
            dir.path(),
            "empty.md",
            "---\nformat-version: 1\nid: deck-empty\n---\n## q\na\n<!-- id: card-qempty -->\n",
        );
        check(vec![declared, empty, plain]).unwrap();

        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        w(
            &workspace,
            alix::workspace::MANIFEST,
            "icon = \"gone.svg\"\n",
        );
        check(vec![workspace]).unwrap();
    }

    #[test]
    fn check_prints_only_declared_sections_and_warns_for_a_missing_icon() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "doctor::tests::check_output_child",
                "--nocapture",
            ])
            .env("ALIX_DOCTOR_CHECK_OUTPUT_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert_eq!(1, stdout.matches("  settings:").count(), "{stdout}");
        assert!(
            stdout.contains("reveal: line, review: sequential"),
            "{stdout}"
        );
        assert_eq!(1, stdout.matches("  requires:").count(), "{stdout}");
        assert_eq!(1, stdout.matches("  sources:").count(), "{stdout}");
        assert!(stderr.contains("points at a missing file"), "{stderr}");
    }

    fn one_line_fingerprint(text: &str) -> String {
        let excerpt = alix::source::Excerpt {
            path: PathBuf::from("source.txt"),
            lines: vec![(1, text.to_string())],
            truncated: false,
        };
        alix::source::format_locator_fingerprint(alix::source::excerpt_fingerprint(&excerpt))
    }

    #[test]
    fn one_authored_locator_is_diagnosed_once_however_many_cards_it_serves() {
        let dir = tempfile::tempdir().unwrap();
        w(dir.path(), "source.txt", "first\nsecond\n");
        w(
            dir.path(),
            "cloze.md",
            "---\nformat-version: 1\nid: deck-cloze\nsource: .\n---\n\
             ## q\nthe first and the second\n\
             <!-- blank: span hidden=\"first\" b:a1b2c3 -->\n\
             <!-- blank: span hidden=\"second\" b:d4e5f6 -->\n\
             <!-- at: source.txt:1-2 -->\n<!-- id: card-q -->\n",
        );
        let deck = Deck::load(dir.path().join("cloze.md")).unwrap();
        assert_eq!(2, deck.cards.len(), "one card per span: {deck:?}");

        let mut report = Report::default();
        deck_resource_findings(&deck, &mut report);

        let citation_warnings = report
            .warnings
            .iter()
            .filter(|warning| warning.contains("at: source.txt:1-2"))
            .count();
        assert_eq!(
            1, citation_warnings,
            "two cards share one authored `at:` line, so it is reported once: {:#?}",
            report.warnings
        );
    }

    #[test]
    fn a_live_citation_never_silences_the_drift_check_on_its_frozen_neighbour() {
        let dir = tempfile::tempdir().unwrap();
        w(dir.path(), workspace::MANIFEST, "title = \"WS\"\n");
        std::fs::create_dir(dir.path().join(workspace::DECKS)).unwrap();
        w(dir.path(), "source.txt", "first\n");
        let deck_path = dir.path().join(workspace::DECKS).join("member.md");
        std::fs::write(
            &deck_path,
            format!(
                "---\nid: deck-1xpgnc8f1mypv80cgzyxrn2cqf\nsource: ..\n---\n\
                 ## stale\na\n<!-- at: source.txt:1 fingerprint: {} -->\n\
                 <!-- id: card-1xpgnc8f1mypv80cgzyxrn2cqf -->\n\n\
                 ## live\nb\n<!-- at: source.txt:1 -->\n\
                 <!-- id: card-2xpgnc8f1mypv80cgzyxrn2cqf -->\n",
                one_line_fingerprint("gone"),
            ),
        )
        .unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        assert!(
            deck.deck_token.is_some(),
            "the fixture is a member: {deck:?}"
        );

        let mut report = Report::default();
        deck_resource_findings(&deck, &mut report);

        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("live `at:` citation")),
            "the member still reports its unfrozen citations: {:#?}",
            report.errors
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("changed or disappeared")),
            "a live citation elsewhere in the deck must not turn off drift \
             detection for a fingerprinted one: {:#?}",
            report.warnings
        );
        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("has no excerpt fingerprint")),
            "the live citations are already one error, not one warning each: {:#?}",
            report.warnings
        );
    }

    #[test]
    fn a_frozen_citation_that_lost_its_fingerprint_is_not_covered_by_the_live_error() {
        let dir = tempfile::tempdir().unwrap();
        w(dir.path(), workspace::MANIFEST, "title = \"WS\"\n");
        std::fs::create_dir(dir.path().join(workspace::DECKS)).unwrap();
        w(dir.path(), "source.txt", "first\nsecond\n");
        let deck_path = dir.path().join(workspace::DECKS).join("member.md");
        std::fs::write(
            &deck_path,
            "---\nid: deck-1xpgnc8f1mypv80cgzyxrn2cqf\nsource: ..\n---\n\
             ## frozen\na\n<!-- at: source.txt:1 -->\n\
             <!-- id: card-1xpgnc8f1mypv80cgzyxrn2cqf -->\n\n\
             ## live\nb\n<!-- at: source.txt:2 -->\n\
             <!-- id: card-2xpgnc8f1mypv80cgzyxrn2cqf -->\n",
        )
        .unwrap();
        alix::assets::freeze_member(&deck_path).unwrap();

        let frozen = std::fs::read_to_string(&deck_path).unwrap();
        let unfingerprinted: String = frozen
            .lines()
            .map(|line| match line.split_once(" fingerprint: ") {
                Some((head, rest)) if line.contains("source.txt:1") => {
                    let asset = rest
                        .split_once(" asset: ")
                        .expect("freezing wrote an asset")
                        .1;
                    format!("{head} asset: {asset}")
                }
                _ if line.contains("source.txt:2") => {
                    let (head, _) = line.split_once(" fingerprint: ").expect("frozen too");
                    format!("{head} -->")
                }
                _ => line.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&deck_path, format!("{unfingerprinted}\n")).unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut report = Report::default();
        deck_resource_findings(&deck, &mut report);

        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("1 live `at:` citation")),
            "one citation lost both fields, so exactly one is live: {:#?}",
            report.errors
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("has no excerpt fingerprint")),
            "the citation that kept its asset is not one of the live ones, so the \
             deck-level error does not cover it: {:#?}",
            report.warnings
        );
    }

    #[test]
    fn a_healthy_citation_never_hides_a_stale_one_on_the_same_card() {
        let dir = tempfile::tempdir().unwrap();
        w(dir.path(), "source.txt", "first\n");
        w(
            dir.path(),
            "twin.md",
            &format!(
                "---\nformat-version: 1\nid: deck-twin\nsource: .\n---\n\
                 ## q\na\n<!-- at: source.txt:1 fingerprint: {} -->\n\
                 <!-- at: source.txt:1 fingerprint: {} -->\n<!-- id: card-q -->\n",
                one_line_fingerprint("first"),
                one_line_fingerprint("gone"),
            ),
        );
        let deck = Deck::load(dir.path().join("twin.md")).unwrap();

        let mut report = Report::default();
        deck_resource_findings(&deck, &mut report);

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("changed or disappeared")),
            "the healthy first citation must not silence the stale second \
             authored line: {:#?}",
            report.warnings
        );
    }

    #[test]
    fn repair_counts_a_changed_citation_as_one_unresolved_item() {
        let dir = tempfile::tempdir().unwrap();
        w(dir.path(), "source.txt", "changed\n");
        let deck = dir.path().join("changed.md");
        w(
            dir.path(),
            "changed.md",
            &format!(
                "---\nformat-version: 1\nid: deck-changed\nsource: .\n---\n## q\na\n<!-- at: source.txt:1 fingerprint: {} -->\n<!-- id: card-q -->\n",
                one_line_fingerprint("gone")
            ),
        );

        let error = repair_source_locators(std::slice::from_ref(&deck)).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("1 source citation(s) need manual review"),
            "{error:#}"
        );
    }

    #[test]
    fn repair_counts_an_ambiguous_citation_as_one_unresolved_item() {
        let dir = tempfile::tempdir().unwrap();
        w(dir.path(), "source.txt", "other\ntarget\nother\ntarget\n");
        let deck = dir.path().join("ambiguous.md");
        w(
            dir.path(),
            "ambiguous.md",
            &format!(
                "---\nformat-version: 1\nid: deck-ambiguous\nsource: .\n---\n## q\na\n<!-- at: source.txt:1 fingerprint: {} -->\n<!-- id: card-q -->\n",
                one_line_fingerprint("target")
            ),
        );

        let error = repair_source_locators(std::slice::from_ref(&deck)).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("1 source citation(s) need manual review"),
            "{error:#}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn grading_spot_check_child() {
        use std::os::unix::fs::PermissionsExt;

        let Some(mode) = std::env::var_os("ALIX_GRADING_SPOT_CHECK_CHILD") else {
            return;
        };
        let mode = mode.to_string_lossy();
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-grader");
        let body = format!(
            r#"#!/bin/sh
cat > "{dir}/prompt.$$"
echo x >> "{dir}/calls.log"
call=$(wc -l < "{dir}/calls.log")
n=$(grep -c '^Question ' "{dir}/prompt.$$")
printf '{{"grades":['
i=1
while [ "$i" -le "$n" ]; do
  [ "$i" -gt 1 ] && printf ','
  verdict="{mode}"
  if [ "$verdict" = mixed ]; then
    if [ "$n" -gt 1 ]; then
      case "$i" in
        4|5|6) verdict=pass ;;
        *) verdict=partial ;;
      esac
    elif [ "$call" -eq 2 ]; then
      verdict=partial
    else
      verdict=pass
    fi
  fi
  printf '{{"verdict":"%s","feedback":"f","missed":[]}}' "$verdict"
  i=$((i+1))
done
printf ']}}'
"#,
            dir = dir.path().display(),
            mode = mode,
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut config = alix::config::Config::from_toml("").unwrap();
        config.ask.command = script.display().to_string();
        config.ask.timeout_secs = 10;

        grading_spot_check(&config).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn grading_spot_check_counts_each_failure_kind_and_the_all_safe_case() {
        let run = |mode: &str| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "doctor::tests::grading_spot_check_child",
                    "--nocapture",
                ])
                .env("ALIX_GRADING_SPOT_CHECK_CHILD", mode)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap()
        };

        let unsafe_output = run("pass");
        assert!(
            unsafe_output.contains("passed 7 answer(s) that must not pass"),
            "{unsafe_output}"
        );
        let unfair_output = run("partial");
        assert!(
            unfair_output.contains("stricter than intended: 4 should-pass probe(s)"),
            "{unfair_output}"
        );
        let safe_output = run("mixed");
        assert!(
            safe_output.contains("grading looks trustworthy"),
            "{safe_output}"
        );
    }

    #[test]
    fn repair_flag_controls_plain_directory_repair_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        w(
            dir.path(),
            "config.toml",
            "[ask]\ncommand = \"/missing/alix-test-backend\"\n",
        );
        w(dir.path(), "source.txt", "evidence\n");
        let deck = dir.path().join("facts.md");
        w(
            dir.path(),
            "facts.md",
            "---\nformat-version: 1\nid: deck-facts\nsource: .\n---\n## q\na\n<!-- at: source.txt:1 -->\n<!-- id: card-q -->\n<!-- reveal: line -->\n",
        );
        let before = std::fs::read_to_string(&deck).unwrap();
        let args = |repair_source_locators| DoctorArgs {
            normalize: false,
            repair_positions: false,
            repair_diagrams: false,
            repair_frontmatter_order: false,
            repair_comment_order: false,
            dir: Some(dir.path().to_path_buf()),
            backends: false,
            all_backends: false,
            grading: false,
            repair_source_locators,
            remove_backup_files: false,
            yes: false,
            config: Some(dir.path().join("config.toml")),
        };

        doctor_cmd(args(false)).unwrap();
        assert_eq!(before, std::fs::read_to_string(&deck).unwrap());

        doctor_cmd(args(true)).unwrap();
        let repaired = std::fs::read_to_string(&deck).unwrap();
        assert_ne!(before, repaired);
        assert!(repaired.contains("fingerprint: xxh64-"), "{repaired}");
    }

    #[test]
    fn check_warns_on_a_missing_workspace_icon() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "icon = \"assets/gone.svg\"\n").unwrap();
        std::fs::write(dir.path().join("a.md"), "## a\n1\n").unwrap();
        assert!(check(vec![dir.path().to_path_buf()]).is_ok());
    }

    #[test]
    fn check_warns_on_a_malformed_deadline() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "").unwrap();
        std::fs::write(
            dir.path().join("alix.local.toml"),
            "[review]\ndeadline = \"soonish\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("a.md"), "## a\n1\n").unwrap();
        assert!(check(vec![dir.path().to_path_buf()]).is_ok());
    }

    #[test]
    fn workspace_findings_report_a_root_deck_without_reading_it_as_a_member() {
        let dir = tempfile::tempdir().unwrap();
        w(dir.path(), "alix.toml", "");
        w(
            dir.path(),
            "misplaced.md",
            "---\nformat-version: 1\nid: deck-misplaced\n---\n## q <!-- id: card-q1 -->\na\n",
        );

        let report = workspace_findings(dir.path());
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("workspace decks belong in")
                    && warning.contains("not discovered"))
        );
    }

    #[test]
    fn doctor_rejects_an_initialized_workspace_member_with_live_source_without_mutating_it() {
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join(alix::workspace::DECKS);
        std::fs::create_dir(&decks).unwrap();
        w(dir.path(), alix::workspace::MANIFEST, "");
        w(dir.path(), "notes.md", "evidence\n");
        let path = decks.join("facts.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: deck-deck1\nsource: notes.md\n---\n## q\na\n<!-- at: notes.md:1 -->\n<!-- id: card-card1 -->\n",
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();

        let report = workspace_findings(dir.path());

        assert!(
            report.errors.join("\n").contains("live `at:` citation"),
            "{:#?}",
            report.errors
        );
        assert_eq!(before, std::fs::read(&path).unwrap());
        assert!(!dir.path().join(alix::assets::ROOT).exists());
    }

    #[test]
    fn doctor_rejects_a_frozen_asset_whose_bytes_do_not_match_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join(alix::workspace::DECKS);
        let assets = dir.path().join("assets/deck-deck1");
        std::fs::create_dir(&decks).unwrap();
        std::fs::create_dir_all(&assets).unwrap();
        w(dir.path(), alix::workspace::MANIFEST, "");
        let name = alix::assets::object_name(b"expected\n", "txt");
        std::fs::write(assets.join(&name), "changed\n").unwrap();
        std::fs::write(
            decks.join("facts.md"),
            format!(
                "---\nformat-version: 1\nid: deck-deck1\nsource: assets/deck-deck1/{name}\n---\n\
                 ## q\na\n<!-- id: card-card1 -->\n"
            ),
        )
        .unwrap();

        let report = workspace_findings(dir.path());

        assert!(
            report
                .errors
                .join("\n")
                .contains("does not match its content address")
        );
    }

    #[test]
    fn doctor_rejects_an_image_owned_by_another_deck() {
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join(alix::workspace::DECKS);
        let other_assets = dir.path().join("assets/deck-deck2");
        std::fs::create_dir(&decks).unwrap();
        std::fs::create_dir_all(&other_assets).unwrap();
        w(dir.path(), alix::workspace::MANIFEST, "");
        let name = alix::assets::object_name(b"image", "png");
        std::fs::write(other_assets.join(&name), "image").unwrap();
        std::fs::write(
            decks.join("facts.md"),
            format!(
                "---\nformat-version: 1\nid: deck-deck1\n---\n\
                 ## q\n![diagram](assets/deck-deck2/{name})\na\n<!-- id: card-card1 -->\n"
            ),
        )
        .unwrap();

        let report = workspace_findings(dir.path());

        assert!(
            report
                .errors
                .join("\n")
                .contains("is not a valid deck-owned asset")
        );
    }

    fn w(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn doctor_notes_inline_emphasis_but_not_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let affected = dir.path().join("affected.md");
        let plain = dir.path().join("plain.md");
        w(
            dir.path(),
            "affected.md",
            "## Multiply three numbers\n2*3*4\n",
        );
        w(dir.path(), "plain.md", "## Add two numbers\n2 + 3 + 4\n");

        let mut affected_report = Report::default();
        deck_findings(&affected, &mut affected_report);
        assert_eq!(affected_report.notes.len(), 1);
        assert!(affected_report.notes[0].contains("Multiply three numbers"));
        assert!(affected_report.notes[0].contains(": 3 "));

        let mut plain_report = Report::default();
        deck_findings(&plain, &mut plain_report);
        assert!(plain_report.notes.is_empty());
    }

    #[test]
    fn doctor_reports_each_malformed_formula_but_not_valid_or_literal_dollars() {
        let dir = tempfile::tempdir().unwrap();
        let malformed = dir.path().join("malformed.md");
        let valid = dir.path().join("valid.md");
        w(
            dir.path(),
            "malformed.md",
            "## Front $\\frac{1$\n\
             - [x] $\\sqrt{$\n\
             - [ ] $\\left($\n\
             > note $\\begin{pmatrix}$\n",
        );
        w(
            dir.path(),
            "valid.md",
            "## Valid $x^2$ and literal prices\n$5 and $10 with unmatched $x\n",
        );

        let mut report = Report::default();
        deck_findings(&malformed, &mut report);
        deck_findings(&valid, &mut report);

        let math_warnings: Vec<&String> = report
            .warnings
            .iter()
            .filter(|warning| warning.contains("malformed LaTeX math"))
            .collect();
        assert_eq!(4, math_warnings.len(), "{:#?}", report.warnings);
        assert!(
            math_warnings
                .iter()
                .all(|warning| warning.contains("malformed.md: card at line 1"))
        );
        assert!(
            math_warnings
                .iter()
                .any(|warning| warning.contains("\\frac{1"))
        );
        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("valid.md: card at line"))
        );
    }

    #[test]
    fn doctor_reports_malformed_cached_choices_and_keypoints() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cached.md");
        w(
            dir.path(),
            "cached.md",
            "---\nformat-version: 1\nid: deck-doctormathdeck\n---\n## q\na\n<!-- id: card-doctormath1 -->\n",
        );
        let parsed =
            alix::parser::parse("cached.md", &std::fs::read_to_string(&path).unwrap()).unwrap();
        let card = &parsed.cards[0];
        let id = card.id().unwrap();
        let deck = Deck::load(&path).unwrap();
        let mut augment = alix::augment::AugmentCache::open_for_deck(&deck).unwrap();
        augment.set_distractors(
            &id,
            vec![r"$\frac{1$".to_string()],
            card.content_fingerprint,
        );
        augment.set_keypoints(&id, vec![r"$\sqrt{$".to_string()], card.content_fingerprint);
        augment.save().unwrap();

        let mut report = Report::default();
        deck_findings(&path, &mut report);

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("generated choice"))
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("generated keypoint"))
        );
    }

    #[test]
    fn doctor_flags_a_dangling_requires_but_not_a_resolvable_one() {
        let dir = tempfile::tempdir().unwrap();
        w(dir.path(), "base.md", "## a\n1\n");
        let dangling = dir.path().join("dangling.md");
        w(
            dir.path(),
            "dangling.md",
            "---\nrequires: ghost\n---\n## q\na\n",
        );
        let resolvable = dir.path().join("resolvable.md");
        w(
            dir.path(),
            "resolvable.md",
            "---\nrequires: base\n---\n## q\na\n",
        );

        let mut report = Report::default();
        deck_findings(&dangling, &mut report);
        deck_findings(&resolvable, &mut report);

        let dangling_warnings: Vec<&String> = report
            .warnings
            .iter()
            .filter(|warning| warning.contains("dangling prerequisite"))
            .collect();
        assert_eq!(1, dangling_warnings.len(), "{:#?}", report.warnings);
        assert!(
            dangling_warnings[0].contains("`ghost`"),
            "{}",
            dangling_warnings[0]
        );
    }

    const CANONICAL_ID: &str = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f";

    #[test]
    fn doctor_warns_when_a_requires_edge_resolves_to_an_unparseable_deck() {
        let dir = tempfile::tempdir().unwrap();
        w(
            dir.path(),
            "broken.md",
            &format!(
                "---\nformat-version: 1\nid: \"{CANONICAL_ID}\"\n---\n## q\na\n\
                 <!-- at: 29.rs @ xxh64:0123456789abcdef from src/x.rs:1-3 -->\n"
            ),
        );
        let by_name = dir.path().join("by-name.md");
        w(
            dir.path(),
            "by-name.md",
            "---\nrequires: broken\n---\n## q\na\n",
        );
        let by_id = dir.path().join("by-id.md");
        w(
            dir.path(),
            "by-id.md",
            &format!("---\nrequires: {CANONICAL_ID}\n---\n## q\na\n"),
        );

        let mut report = Report::default();
        deck_findings(&by_name, &mut report);
        deck_findings(&by_id, &mut report);

        let parse_warnings: Vec<&String> = report
            .warnings
            .iter()
            .filter(|warning| warning.contains("fails to parse; its errors name the fix"))
            .collect();
        assert_eq!(2, parse_warnings.len(), "{:#?}", report.warnings);
        assert!(parse_warnings[0].contains("`broken`"), "{parse_warnings:?}");
        assert!(
            parse_warnings[1].contains(&format!("`{CANONICAL_ID}`")),
            "{parse_warnings:?}"
        );
        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("dangling prerequisite")),
            "{:#?}",
            report.warnings
        );
    }

    #[test]
    fn a_bare_token_in_requires_is_an_ordinary_dangling_filename_edge() {
        let dir = tempfile::tempdir().unwrap();
        let token = CANONICAL_ID.strip_prefix("deck-").unwrap();
        let path = dir.path().join("d.md");
        w(
            dir.path(),
            "d.md",
            &format!("---\nrequires: {token}\n---\n## q\na\n"),
        );

        let mut report = Report::default();
        deck_findings(&path, &mut report);

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("dangling prerequisite")),
            "{:#?}",
            report.warnings
        );
        assert!(
            report.notes.iter().all(|note| !note.contains(token)),
            "no note singles the bare token out: {:#?}",
            report.notes
        );
    }

    #[test]
    fn doctor_warns_on_a_dangling_id_mode_requires_and_accepts_a_resolvable_one() {
        let dir = tempfile::tempdir().unwrap();
        w(
            dir.path(),
            "base.md",
            &format!(
                "---\nformat-version: 1\nid: \"{CANONICAL_ID}\"\n---\n## a <!-- id: card-b1 -->\n1\n"
            ),
        );
        let resolvable = dir.path().join("resolvable.md");
        w(
            dir.path(),
            "resolvable.md",
            &format!("---\nrequires: {CANONICAL_ID}\n---\n## q\na\n"),
        );
        let dangling = dir.path().join("dangling.md");
        w(
            dir.path(),
            "dangling.md",
            "---\nrequires: deck-zzzzzzzzzzzzzzzzzzzzzzzzzz\n---\n## q\na\n",
        );

        let mut report = Report::default();
        deck_findings(&resolvable, &mut report);
        deck_findings(&dangling, &mut report);

        let dangling_warnings: Vec<&String> = report
            .warnings
            .iter()
            .filter(|warning| warning.contains("dangling prerequisite"))
            .collect();
        assert_eq!(1, dangling_warnings.len(), "{:#?}", report.warnings);
        assert!(
            dangling_warnings[0].contains("deck-zzzzzzzzzzzzzzzzzzzzzzzzzz"),
            "{}",
            dangling_warnings[0]
        );
        assert!(
            dangling_warnings[0]
                .contains("write the `.md` extension (`deck-zzzzzzzzzzzzzzzzzzzzzzzzzz.md`)"),
            "{}",
            dangling_warnings[0]
        );
    }

    #[test]
    fn doctor_rejects_a_pasted_card_id_in_requires_as_wrong_type() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        w(
            dir.path(),
            "d.md",
            "---\nrequires: card-9w2c7x4k1m8q3z5t0v6b2n4d8f\n---\n## q\na\n",
        );

        let mut report = Report::default();
        deck_findings(&path, &mut report);

        let errors = report.errors.join("\n");
        assert!(errors.contains("card id"), "{errors}");
        assert!(errors.contains("never a prerequisite"), "{errors}");
    }

    #[test]
    fn doctor_hints_when_a_filename_mode_requires_looks_like_a_truncated_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        w(
            dir.path(),
            "d.md",
            "---\nrequires: deck-9w2c7x4k1m8q3z5t0v6b2n4d8\n---\n## q\na\n",
        );

        let mut report = Report::default();
        deck_findings(&path, &mut report);

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("dangling prerequisite")),
            "{:#?}",
            report.warnings
        );
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("truncated or malformed")),
            "{:#?}",
            report.notes
        );
    }

    #[test]
    fn doctor_reports_a_file_shadowing_a_required_deck_id() {
        let dir = tempfile::tempdir().unwrap();
        w(
            dir.path(),
            "base.md",
            &format!(
                "---\nformat-version: 1\nid: \"{CANONICAL_ID}\"\n---\n## a <!-- id: card-b1 -->\n1\n"
            ),
        );
        w(
            dir.path(),
            &format!("{CANONICAL_ID}.md"),
            "## impostor\na\n",
        );
        let path = dir.path().join("d.md");
        w(
            dir.path(),
            "d.md",
            &format!("---\nrequires: {CANONICAL_ID}\n---\n## q\na\n"),
        );

        let mut report = Report::default();
        deck_findings(&path, &mut report);

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("the id wins")
                    && warning.contains(&format!("write `{CANONICAL_ID}.md` to require the file"))),
            "{:#?}",
            report.warnings
        );
        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("dangling prerequisite")),
            "{:#?}",
            report.warnings
        );
    }

    #[test]
    fn a_dormant_template_base_id_is_not_an_orphan() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let decks = dir.join("decks");
        std::fs::create_dir(&decks).unwrap();
        w(dir, "alix.toml", "title = \"Regions\"\n");
        w(
            &decks,
            "regions.md",
            "---\nformat-version: 1\nid: deck-regiondoc\n---\n## anatomy\nthe lunate is carpal\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n<!-- id: card-parent1 -->\n",
        );
        let mut store = alix::store::Store::open_deck(
            dir.join("progress/deck-regiondoc.json"),
            "deck-regiondoc",
            "regions.md",
        )
        .unwrap();
        store.get_or_insert("card-parent1");
        store.save().unwrap();

        let report = workspace_findings(dir);
        assert!(
            report.errors.is_empty(),
            "the fixture itself must be clean: {:?}",
            report.errors
        );
        let warnings = report.warnings.join("\n");
        assert!(
            !warnings.contains("orphaned store key (card) `card-parent1`"),
            "a blank template's base id is a reserved live identity: {warnings}"
        );
    }

    #[test]
    fn doctor_flags_the_full_check_set() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let decks = dir.join("decks");
        std::fs::create_dir(&decks).unwrap();
        w(dir, "alix.toml", "title = \"Check Set\"\n");
        w(&decks, "bad-token.md", "## q\na\n<!-- id: BAD1 -->\n");
        w(
            &decks,
            "bad-value.md",
            "---\nreveal: bogus\n---\n## q\na\n<!-- id: card-bv1 -->\n",
        );
        w(
            &decks,
            "dup-deck.md",
            "---\nformat-version: 1\nid: deck-dupdeck\n---\n## q\na\n<!-- id: card-dd1 -->\n",
        );
        w(
            &decks,
            "dup-deck copy.md",
            "---\nformat-version: 1\nid: deck-dupdeck\n---\n## q\na\n<!-- id: card-dd1 -->\n",
        );
        w(
            &decks,
            "card-dup.md",
            "---\nformat-version: 1\nid: deck-cda\n---\n## q\na\n<!-- id: card-cshared -->\n",
        );
        w(
            &decks,
            "card-dup copy.md",
            "---\nformat-version: 1\nid: deck-cdb\n---\n## q\nb\n<!-- id: card-cshared -->\n",
        );
        w(
            &decks,
            "unspliceable.md",
            "---\n{source: [a]}\n---\n## q\nb\n<!-- id: card-uq1 -->\n",
        );
        w(
            &decks,
            "indented.md",
            "## real\n  ## not a front\nanswer\n<!-- id: card-ind1 -->\n",
        );
        w(
            &decks,
            "imgcard.md",
            "## pic\nphoto\n![](missing.png)\n<!-- id: card-img1 -->\n",
        );
        w(
            &decks,
            "fresh.md",
            "---\nformat-version: 1\nid: \"deck-fresh\"\n---\n## q\na\n",
        );
        w(
            &decks,
            "trace-bad.md",
            "---\ntrace: a walk\nsource: trace-src.txt\n---\n## hop\nstep\n<!-- id: card-thop1 -->\n<!-- at: 5-6 -->\n",
        );
        w(dir, "trace-src.txt", "one\ntwo\n");
        w(
            &decks,
            "at-dangling.md",
            "---\nsource: .\n---\n## cited\nb\n<!-- at: missing.rs:1-2 -->\n<!-- id: card-atd1 -->\n",
        );
        w(&decks, "sourceless.md", "## a\n1\n<!-- id: card-sla1 -->\n");
        w(
            &decks,
            "gated.md",
            "---\nsource: https://example.test\nrequires: sourceless\n---\n## b\n1\n<!-- id: card-gtd1 -->\n",
        );

        let mut store = alix::store::Store::open_deck(
            dir.join("progress/orphan-owner.json"),
            "orphan-owner",
            "orphan-owner.md",
        )
        .unwrap();
        store.get_or_insert("orphancard");
        store.set_last_depth("orphan-owner", alix::depth::Depth::Recall);
        store.save().unwrap();

        let report = workspace_findings(dir);
        let errors = report.errors.join("\n");
        let warnings = report.warnings.join("\n");

        assert!(
            errors.contains("must hold a base `card-<token>` id"),
            "invalid token: {errors}"
        );
        assert!(
            errors.contains("invalid value"),
            "bad directive value: {errors}"
        );
        assert!(warnings.contains("duplicate deck token"), "{warnings}");
        assert!(warnings.contains("duplicate card token"), "{warnings}");
        assert!(
            warnings.contains("not canonical"),
            "non-canonical token: {warnings}"
        );
        assert!(warnings.contains("orphaned store key (card)"), "{warnings}");
        assert!(warnings.contains("orphaned store key (deck)"), "{warnings}");
        assert!(
            warnings.contains("fresh tokens were minted"),
            "coarse fresh-mint: {warnings}"
        );
        assert!(
            warnings.contains("not a block mapping"),
            "unspliceable: {warnings}"
        );
        assert!(warnings.contains("indented `##`"), "{warnings}");
        assert!(warnings.contains("missing image"), "{warnings}");
        assert!(
            warnings.contains("checkpoint at line") && warnings.contains("has only 2 lines"),
            "trace-locator: {warnings}"
        );
        assert!(
            warnings.contains("cannot read the source"),
            "dangling `at:` citation: {warnings}"
        );
        assert!(
            warnings.contains("requires ungrounded") && warnings.contains("`sourceless`"),
            "dead `requires:`: {warnings}"
        );
        assert!(
            warnings.contains("card content without ids"),
            "unstamped warning: {warnings}"
        );
        assert!(
            warnings.contains("is not the last line of its card"),
            "misplaced id marker: {warnings}"
        );
    }

    #[test]
    fn doctor_accepts_a_canonically_closed_id_marker_without_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        w(
            dir,
            "facts.md",
            "---\nformat-version: 1\nid: deck-deck1\n---\n## q\na\n<!-- at: notes.md:1 -->\n<!-- id: card-card1 -->\n",
        );
        w(dir, "notes.md", "one\n");

        let report = workspace_findings(dir);

        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("is not the last line of its card")),
            "canonical marker flagged: {:#?}",
            report.warnings
        );
    }

    #[test]
    fn doctor_reports_orphaned_state_documents_and_sync_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        w(
            dir.path(),
            "deck.md",
            "---\nformat-version: 1\nid: deck-deck1\n---\n## q\na\n<!-- id: card-card1 -->\n",
        );
        let user_root = dir.path();
        alix::state::open_store(&dir.path().join("deck.md"), user_root)
            .unwrap()
            .save()
            .unwrap();
        let orphan_path = dir.path().join("augment/deck-orphan.json");
        alix::augment::AugmentCache::open_deck(&orphan_path, "deck-orphan")
            .unwrap()
            .save()
            .unwrap();
        let conflict = dir
            .path()
            .join("progress/deck-deck1.sync-conflict-20260725-phone.json");
        w(
            dir.path(),
            "progress/deck-deck1.sync-conflict-20260725-phone.json",
            "{}",
        );

        let report = workspace_findings(dir.path());
        let warnings = report.warnings.join("\n");

        assert!(warnings.contains("orphaned augmentation document"));
        assert!(warnings.contains(&conflict.display().to_string()));
    }

    #[test]
    fn a_bare_token_state_document_is_reported_generically() {
        let dir = tempfile::tempdir().unwrap();
        w(dir.path(), "alix.toml", "");
        std::fs::create_dir(dir.path().join("progress")).unwrap();
        w(
            &dir.path().join("progress"),
            "mathdeck.json",
            r#"{"version":1,"deck_id":"mathdeck","subject":"math.md","revision":1,"cards":{},"records":{},"writer":null}"#,
        );

        let report = workspace_findings(dir.path());

        assert!(
            report
                .warnings
                .iter()
                .chain(&report.errors)
                .any(|finding| finding.contains("mathdeck.json")),
            "the document must be reported: {:#?} {:#?}",
            report.warnings,
            report.errors
        );
        assert!(
            !report
                .warnings
                .iter()
                .chain(&report.errors)
                .chain(&report.notes)
                .any(|finding| finding.contains("un-converted")),
            "no retired-format vocabulary: {:#?} {:#?}",
            report.warnings,
            report.errors
        );
    }

    #[test]
    fn doctor_flags_a_source_pointing_into_assets() {
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join(alix::workspace::DECKS);
        std::fs::create_dir(&decks).unwrap();
        w(dir.path(), alix::workspace::MANIFEST, "");
        let assets = dir.path().join("assets/deck-deck1");
        std::fs::create_dir_all(&assets).unwrap();
        let name = alix::assets::object_name(b"excerpt\n", "md");
        std::fs::write(assets.join(&name), "excerpt\n").unwrap();
        w(
            &decks,
            "facts.md",
            &format!(
                "---\nformat-version: 1\nid: deck-deck1\nsource: assets/deck-deck1/{name}\n---\n\
                 ## q\na\n<!-- id: card-card1 -->\n"
            ),
        );

        let report = workspace_findings(dir.path());

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("points into `assets/`")
                    && warning.contains("keeps its real source")),
            "source into assets: {:#?}",
            report.warnings
        );
    }

    #[test]
    fn doctor_clean_workspace_has_no_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join(alix::workspace::DECKS);
        std::fs::create_dir(&decks).unwrap();
        w(dir.path(), alix::workspace::MANIFEST, "");
        w(
            &decks,
            "facts.md",
            "---\nformat-version: 1\nid: deck-deck1\n---\n## q <!-- id: card-card1 -->\na\n",
        );
        std::fs::create_dir(dir.path().join("progress")).unwrap();
        w(
            &dir.path().join("progress"),
            "deck-deck1.json",
            r#"{"version":1,"deck_id":"deck-deck1","subject":"facts.md","revision":1,"cards":{},"records":{},"writer":null}"#,
        );

        let report = workspace_findings(dir.path());

        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("un-converted")),
            "clean workspace flagged un-converted: {:#?}",
            report.warnings
        );
        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("points into `assets/`")),
            "clean workspace flagged a source into assets: {:#?}",
            report.warnings
        );
    }

    #[test]
    fn source_locator_repair_is_explicit_and_preserves_card_identity() {
        let dir = tempfile::tempdir().unwrap();
        w(
            dir.path(),
            "code.rs",
            "inserted\nalpha\nfn answer() {\n    42\n}\nomega\n",
        );
        let expected = alix::source::Excerpt {
            path: PathBuf::from("code.rs"),
            lines: vec![(2, "fn answer() {".into()), (3, "    42".into())],
            truncated: false,
        };
        let fingerprint =
            alix::source::format_locator_fingerprint(alix::source::excerpt_fingerprint(&expected));
        let deck_path = dir.path().join("deck.md");
        w(
            dir.path(),
            "deck.md",
            &format!(
                "---\nformat-version: 1\nid: \"deck-deck1\"\nsource: .\n---\n\
                 ## q\nanswer\n<!-- at: code.rs:2-3 fingerprint: {fingerprint} -->\n\
                 <!-- id: card-card1 -->\n"
            ),
        );

        let before = std::fs::read_to_string(&deck_path).unwrap();
        let mut report = Report::default();
        deck_findings(&deck_path, &mut report);
        assert!(
            report
                .warnings
                .join("\n")
                .contains("moved to `code.rs:3-4`"),
            "{:?}",
            report.warnings
        );
        assert_eq!(before, std::fs::read_to_string(&deck_path).unwrap());

        repair_source_locators(std::slice::from_ref(&deck_path)).unwrap();
        let after = std::fs::read_to_string(&deck_path).unwrap();
        assert!(after.contains(&format!(
            "<!-- at: code.rs:3-4 fingerprint: {fingerprint} -->"
        )));
        assert!(after.contains("<!-- id: card-card1 -->"));
        assert_eq!(
            Some("card-card1".to_string()),
            Deck::load(&deck_path).unwrap().cards[0].id()
        );
    }

    #[test]
    fn a_sampling_key_that_can_affect_nothing_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let head = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n";
        let table = "| w | m |\n|---|---|\n| a | alpha | <!-- r:aaaaaa -->\n<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n";
        // (deck body, warning expected)
        let cases = [
            (
                format!("{head}sampling: off\n---\n## q\na\n<!-- id: card-q1x -->\n"),
                true,
            ),
            (format!("{head}sampling: off\n---\n{table}"), false),
            (
                format!("{head}---\n## q\na\n<!-- id: card-q1x -->\n"),
                false,
            ),
            (
                format!("{head}---\n## q\na\n<!-- sampling: off -->\n<!-- id: card-q1x -->\n"),
                true,
            ),
        ];
        for (index, (text, expected)) in cases.iter().enumerate() {
            let path = dir.path().join(format!("d{index}.md"));
            std::fs::write(&path, text).unwrap();
            let mut report = Report::default();
            deck_findings(&path, &mut report);
            let warned = report
                .warnings
                .iter()
                .any(|w| w.contains("`sampling:` has no effect"));
            assert_eq!(*expected, warned, "case {index}: {:?}", report.warnings);
        }
    }

    #[test]
    fn unstamped_table_rows_are_reported_as_content_without_ids() {
        let dir = tempfile::tempdir().unwrap();
        let head = "---\nformat-version: 1\nid: deck-tbl\n---\n";
        let rows = "| a | alpha | <!-- r:aaaaaa -->\n| b | beta |\n| c | gamma |\n";
        let container = "<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n";

        // With a container id, only the stamp-less rows lack identity.
        let path = dir.path().join("partly.md");
        std::fs::write(
            &path,
            format!("{head}| w | m |\n|---|---|\n{rows}<!-- cards -->\n{container}"),
        )
        .unwrap();
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("2 entries are card content without ids")),
            "{:?}",
            report.warnings
        );

        // Without one, no row can compose an id, so every row is reported.
        let path = dir.path().join("none.md");
        std::fs::write(
            &path,
            format!("{head}| w | m |\n|---|---|\n{rows}<!-- cards -->\n"),
        )
        .unwrap();
        let mut report = Report::default();
        deck_findings(&path, &mut report);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("3 entries are card content without ids")),
            "{:?}",
            report.warnings
        );

        // Stamping resolves both: no unstamped warning survives an init.
        for name in ["partly.md", "none.md"] {
            let path = dir.path().join(name);
            alix::stamp::stamp_deck(&path).unwrap();
            let mut after = Report::default();
            deck_findings(&path, &mut after);
            assert!(
                !after
                    .warnings
                    .iter()
                    .any(|w| w.contains("content without ids")),
                "{name}: {:?}",
                after.warnings
            );
        }
    }
}
