use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use alix::{
    deck::{AtRewrite, Deck},
    source::{CitationIntegrity, SourceBase},
    trace::Trace,
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

fn deck_findings(path: &Path, strict: bool, report: &mut Report) {
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
            let charset = matches!(e, alix::parser::ParseError::InvalidToken { .. });
            if strict || charset {
                report.error(format!("{}: {e}", path.display()));
            } else {
                report.warn(format!("{}: {e}", path.display()));
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

    let augment = path.parent().and_then(|dir| {
        deck.deck_token.as_deref().and_then(|deck_id| {
            alix::state::open_augment_read_only(deck_id, &alix::workspace::root_store_path(dir))
                .ok()
        })
    });
    for diagnostic in alix::math::diagnostics(&deck.cards, augment.as_ref()) {
        report.warn(format!("{}: {diagnostic}", path.display()));
    }
    inline_emphasis_findings(&deck.cards, report);

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
        if alix::token::is_valid(tok) && !alix::token::is_canonical(tok) {
            report.warn(format!(
                "{}: token `{tok}` is valid but not canonical (not 26 base32 chars)",
                path.display()
            ));
        }
    }

    if deck.deck_token.is_none() && deck.frontmatter_span.is_some() && deck.frontmatter.unspliceable
    {
        report.warn(format!(
            "{}: cannot stamp: frontmatter is not a block mapping, so no `alix-id:` can be spliced in",
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

    if let Ok(deck) = Deck::load(path) {
        deck_resource_findings(&deck, report);
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

fn deck_resource_findings(deck: &Deck, report: &mut Report) {
    // Advisory only: the deck still works, the web server just 404s the image.
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
        let what = if drift.gone {
            "source file is gone"
        } else {
            "no longer found in the source"
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
    let base = SourceBase::for_deck(deck);
    for card in &deck.cards {
        for citation in &card.citations {
            let detail = match base.inspect_citation(citation) {
                Ok(CitationIntegrity::Current(_)) => None,
                Ok(CitationIntegrity::Unfingerprinted { .. }) => Some(
                    "has no excerpt fingerprint; review it, then run \
                     `alix doctor --repair-source-locators`"
                        .to_string(),
                ),
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
    for prereq in alix::deck::nongating_prerequisites(deck) {
        report.warn(format!(
            "{}: requires source-less `{prereq}`: this edge doesn't gate its exam; \
             add a `source:` to `{prereq}` to make it a real prerequisite",
            deck.subject
        ));
    }
}

fn lint_message(path: &Path, lint: &alix::parser::Lint) -> String {
    use alix::parser::LintKind;
    let detail = match &lint.kind {
        LintKind::UnknownKey { key } => format!("unknown key `{key}` (ignored)"),
        LintKind::BadValue { key, value } => format!("`{key}` has an invalid value `{value}`"),
        LintKind::EmptyValue { key } => format!("`{key}` has an empty value"),
        LintKind::RevealOnCloze => {
            "`reveal:` on a cloze card is ignored (the holes are the reveal)".to_string()
        }
        LintKind::IndentedH2 => {
            "an indented `##` line is content, not a card front (likely a mistype)".to_string()
        }
        LintKind::ClozeInHole => {
            "a `\\blank` inside a cloze hole is literal text, not a nested hole".to_string()
        }
        LintKind::UnclosedComment => {
            "a `<!--` line that never closes with `-->` stays content".to_string()
        }
        LintKind::UnclosedFence => "a fence opened here never closes; everything after it \
             (cards included) was swallowed as its content"
            .to_string(),
        LintKind::ImageMalformed => "an image embed here is malformed; write \
             `![alt](file.png)` (or escape a literal `![` as `\\![`)"
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
    };
    format!("{}: line {}: {detail}", path.display(), lint.line)
}

fn workspace_findings(dir: &Path) -> Report {
    let mut report = Report::default();
    for path in alix::workspace::misplaced_deck_files(dir).unwrap_or_default() {
        report.warn(format!(
            "{}: workspace decks belong in {}; this file is not discovered",
            path.display(),
            dir.join(alix::workspace::DECKS).display()
        ));
    }
    let (deck_files, uninitialized) =
        alix::workspace::classified_deck_files(dir).unwrap_or_default();
    for path in &deck_files {
        deck_findings(path, false, &mut report);
    }
    for path in uninitialized {
        deck_findings(&path, false, &mut report);
    }

    let map = alix::dedup::scan_dir(dir);
    for (kept, excluded, token) in &map.excluded_decks {
        report.warn(format!(
            "duplicate deck token `{token}`: {} is excluded (kept {}); delete the `alix-id:` line in the copy",
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
            dupe.token,
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
            let mut known_subjects: HashSet<String> = HashSet::new();
            let mut any_fresh = false;
            for path in &deck_files {
                if let Ok(deck) = Deck::load(path) {
                    known_subjects.insert(deck.subject.clone());
                    for card in &deck.cards {
                        if let Some(id) = card.id() {
                            if store.get(&id).is_none() {
                                any_fresh = true;
                            }
                            known_cards.insert(id);
                        }
                    }
                }
            }
            let orphans = store.orphans(&known_cards, &known_subjects);
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
        Err(error) => report.error(format!("state layout: {error}")),
    }
    if let Err(error) = alix::augment::AugmentCache::open_for_store(&store_path) {
        report.error(format!("augmentation state: {error}"));
    }
    let layout = alix::state::Layout::new(&store_path);
    for (kind, state_dir) in [
        ("progress", layout.progress.as_path()),
        ("augmentation", layout.augment.as_path()),
    ] {
        for entry in std::fs::read_dir(state_dir).into_iter().flatten().flatten() {
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
                report.warn(format!(
                    "unrecognized {kind} state document {}",
                    path.display()
                ));
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
                report.error(format!("{kind} state document: {error}"));
                continue;
            }
            if !known_deck_ids.contains(deck_id) {
                report.warn(format!(
                    "orphaned {kind} state document {} belongs to no deck in {}",
                    path.display(),
                    dir.display()
                ));
            }
        }
    }
    for name in ["progress.json", "augment.json"] {
        let stray = store_path.join(name);
        if stray.is_file() {
            report.warn(format!(
                "{}: aggregate state file is never read (state lives in per-deck documents \
                 under `progress/` and `augment/`); back it up and delete it",
                stray.display()
            ));
        }
    }
    for conflict in alix::store::sync_conflicts(&store_path) {
        report.warn(format!(
            "synchronization conflict needs deliberate recovery: {}",
            conflict.display()
        ));
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() && alix::workspace::is_workspace(&path) {
            let nested = workspace_findings(&path);
            report.errors.extend(nested.errors);
            report.warnings.extend(nested.warnings);
            report.notes.extend(nested.notes);
        }
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
            let s = &deck.settings;
            let declared: Vec<String> = [
                s.reveal.map(|r| format!("reveal: {}", val_name(r))),
                s.order.map(|o| format!("order: {}", val_name(o))),
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
        deck_findings(path, true, &mut report);
    }
    if report.render() {
        bail!("{} error(s) found", report.errors.len());
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
        for card in &deck.cards {
            for citation in &card.citations {
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
                    origin: citation.origin.clone(),
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

// Exits non-zero only on a hard fail; a missing optional binary (a warn)
// never breaks a script.
pub(crate) fn doctor_cmd(args: DoctorArgs) -> Result<()> {
    use alix::doctor::{self, Status};
    if let Some(path) = &args.dir {
        if path.is_file() {
            if args.repair_source_locators {
                repair_source_locators(std::slice::from_ref(path))?;
            }
            return check(vec![path.clone()]);
        }
        if alix::workspace::is_workspace(path) {
            if args.repair_source_locators {
                repair_source_locators(&alix::workspace::deck_files(path))?;
            }
            check(vec![path.clone()])?;
        }
    }
    let (config_finding, config) = doctor::check_config(args.config.as_deref());
    let mut findings = vec![config_finding];
    let (decks_dir, store_path) = match &args.dir {
        Some(path) => (path.clone(), workspace::root_store_path(path)),
        None => {
            let dir = config.decks_dir().context("cannot determine ~/decks")?;
            let store = workspace::root_store_path(&dir);
            (dir, store)
        }
    };
    if args.repair_source_locators
        && !args
            .dir
            .as_deref()
            .is_some_and(alix::workspace::is_workspace)
    {
        repair_source_locators(&alix::workspace::deck_files(&decks_dir))?;
    }
    findings.push(doctor::check_store(Some(store_path)));
    findings.push(doctor::check_decks(&decks_dir));
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
            "---\nalix-id: misplaced\n---\n## q <!-- id: q1 -->\na\n",
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
        deck_findings(&affected, true, &mut affected_report);
        assert_eq!(affected_report.notes.len(), 1);
        assert!(affected_report.notes[0].contains("Multiply three numbers"));
        assert!(affected_report.notes[0].contains(": 3 "));

        let mut plain_report = Report::default();
        deck_findings(&plain, true, &mut plain_report);
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
        deck_findings(&malformed, true, &mut report);
        deck_findings(&valid, true, &mut report);

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
            "---\nalix-id: doctormathdeck\n---\n## q <!-- id: doctormath1 -->\na\n",
        );
        let parsed =
            alix::parser::parse("cached.md", &std::fs::read_to_string(&path).unwrap()).unwrap();
        let card = &parsed.cards[0];
        let id = card.id().unwrap();
        let mut augment = alix::state::open_augment(&path, dir.path()).unwrap();
        augment.set_distractors(
            &id,
            vec![r"$\frac{1$".to_string()],
            card.content_fingerprint,
        );
        augment.set_keypoints(&id, vec![r"$\sqrt{$".to_string()], card.content_fingerprint);
        augment.save().unwrap();

        let mut report = Report::default();
        deck_findings(&path, true, &mut report);

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
    fn doctor_flags_the_full_check_set() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let decks = dir.join("decks");
        std::fs::create_dir(&decks).unwrap();
        w(dir, "alix.toml", "title = \"Check Set\"\n");
        w(&decks, "bad-token.md", "## q <!-- id: BAD1 -->\na\n");
        w(
            &decks,
            "bad-value.md",
            "---\nreveal: bogus\n---\n## q <!-- id: bv1 -->\na\n",
        );
        w(
            &decks,
            "dup-deck.md",
            "---\nalix-id: dupdeck\n---\n## q <!-- id: dd1 -->\na\n",
        );
        w(
            &decks,
            "dup-deck copy.md",
            "---\nalix-id: dupdeck\n---\n## q <!-- id: dd1 -->\na\n",
        );
        w(
            &decks,
            "card-dup.md",
            "---\nalix-id: cda\n---\n## q <!-- id: cshared -->\na\n",
        );
        w(
            &decks,
            "card-dup copy.md",
            "---\nalix-id: cdb\n---\n## q <!-- id: cshared -->\nb\n",
        );
        w(
            &decks,
            "unspliceable.md",
            "---\n{source: [a]}\n---\n## q <!-- id: uq1 -->\nb\n",
        );
        w(
            &decks,
            "cloze.md",
            "## Fill <!-- id: clz1 -->\n<!-- reveal: line -->\nthe \\blank{a} gap\n",
        );
        w(
            &decks,
            "indented.md",
            "## real <!-- id: ind1 -->\n  ## not a front\nanswer\n",
        );
        w(
            &decks,
            "imgcard.md",
            "## pic <!-- id: img1 -->\nphoto\n![](missing.png)\n",
        );
        w(
            &decks,
            "fresh.md",
            "---\nalix-id: \"fresh\"\n---\n## q\na\n",
        );
        w(
            &decks,
            "trace-bad.md",
            "---\ntrace: a walk\nsource: trace-src.txt\n---\n## hop <!-- id: thop1 -->\nstep\n<!-- at: 5-6 -->\n",
        );
        w(dir, "trace-src.txt", "one\ntwo\n");
        w(
            &decks,
            "at-dangling.md",
            "---\nsource: .\n---\n## cited <!-- id: atd1 -->\nb\n<!-- at: missing.rs:1-2 -->\n",
        );
        w(&decks, "sourceless.md", "## a <!-- id: sla1 -->\n1\n");
        w(
            &decks,
            "gated.md",
            "---\nsource: https://example.test\nrequires: sourceless\n---\n## b <!-- id: gtd1 -->\n1\n",
        );

        let mut store = alix::store::Store::open_deck(
            dir.join("progress/orphan-owner.json"),
            "orphan-owner",
            "orphan-owner.md",
        )
        .unwrap();
        store.get_or_insert("orphancard", 0);
        store.set_last_depth("ghostdeck.md", alix::depth::Depth::Recall);
        store.save().unwrap();

        let report = workspace_findings(dir);
        let errors = report.errors.join("\n");
        let warnings = report.warnings.join("\n");

        assert!(
            errors.contains("fails the charset"),
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
        assert!(
            warnings.contains("cloze card is ignored"),
            "reveal-on-cloze: {warnings}"
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
            warnings.contains("requires source-less") && warnings.contains("`sourceless`"),
            "dead `% requires:`: {warnings}"
        );
        assert!(
            warnings.contains("card content without ids"),
            "unstamped warning: {warnings}"
        );
    }

    #[test]
    fn doctor_reports_orphaned_state_documents_and_sync_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        w(
            dir.path(),
            "deck.md",
            "---\nalix-id: deck1\n---\n## q <!-- id: card1 -->\na\n",
        );
        let state_root = dir.path();
        alix::state::open_store(&dir.path().join("deck.md"), state_root)
            .unwrap()
            .save()
            .unwrap();
        let orphan_path = dir.path().join("augment/orphan.json");
        alix::augment::AugmentCache::open_deck(&orphan_path, "orphan")
            .unwrap()
            .save()
            .unwrap();
        let conflict = dir
            .path()
            .join("progress/deck1.sync-conflict-20260725-phone.json");
        w(
            dir.path(),
            "progress/deck1.sync-conflict-20260725-phone.json",
            "{}",
        );

        let report = workspace_findings(dir.path());
        let warnings = report.warnings.join("\n");

        assert!(warnings.contains("orphaned augmentation state document"));
        assert!(warnings.contains(&conflict.display().to_string()));
    }

    #[test]
    fn doctor_flags_unread_aggregate_state_files() {
        let dir = tempfile::tempdir().unwrap();
        w(
            dir.path(),
            "deck.md",
            "---\nalix-id: deck1\n---\n## q <!-- id: card1 -->\na\n",
        );
        w(dir.path(), "progress.json", r#"{"version":1,"cards":{}}"#);
        w(dir.path(), "augment.json", r#"{"version":1,"cards":{}}"#);

        let report = workspace_findings(dir.path());
        let warnings = report.warnings.join("\n");

        assert!(
            warnings.contains("progress.json") && warnings.contains("never read"),
            "aggregate progress warning: {warnings}"
        );
        assert!(
            warnings.contains("augment.json"),
            "aggregate augment warning: {warnings}"
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
            alix::source::format_excerpt_fingerprint(alix::source::excerpt_fingerprint(&expected));
        let deck_path = dir.path().join("deck.md");
        w(
            dir.path(),
            "deck.md",
            &format!(
                "---\nalix-id: \"deck1\"\nsource: .\n---\n\
                 ## q\nanswer\n<!-- at: code.rs:2-3 @ {fingerprint} -->\n\
                 <!-- id: card1 -->\n"
            ),
        );

        let before = std::fs::read_to_string(&deck_path).unwrap();
        let mut report = Report::default();
        deck_findings(&deck_path, true, &mut report);
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
        assert!(after.contains(&format!("at: code.rs:3-4 @ {fingerprint}")));
        assert!(after.contains("<!-- id: card1 -->"));
        assert_eq!(
            Some("card1".to_string()),
            Deck::load(&deck_path).unwrap().cards[0].id()
        );
    }
}
