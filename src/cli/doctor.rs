//! `alix doctor`: health checks for the config, progress store, decks folder,
//! and optional external CLIs — plus the per-deck lint (`check`) it shares
//! with a deck-file target.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use alix::{
    deck::Deck,
    store::Store,
    trace::{SourceBase, Trace},
    workspace,
};
use anyhow::{Context, Result, bail};

use crate::DoctorArgs;

/// The canonical CLI name of a value-enum value (e.g. `Mode::LineByLine` →
/// `"line"`), for echoing a deck's declared settings.
fn val_name<T: clap::ValueEnum>(value: T) -> String {
    value
        .to_possible_value()
        .map(|p| p.get_name().to_string())
        .unwrap_or_default()
}

/// Doctor findings bucketed by severity (spec §5): an error fails the run;
/// warnings and infos are advisory. Collected without printing so the CLI
/// renders them and a test can assert on them.
#[derive(Default)]
struct Report {
    errors: Vec<String>,
    warnings: Vec<String>,
    infos: Vec<String>,
}

impl Report {
    fn error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }
    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
    fn info(&mut self, msg: impl Into<String>) {
        self.infos.push(msg.into());
    }
    /// Prints findings with the CLI's `error:`/`warning:` prefixes (infos
    /// plain) and a count summary. Returns whether any error was found.
    fn render(&self) -> bool {
        for e in &self.errors {
            eprintln!("error: {e}");
        }
        for w in &self.warnings {
            eprintln!("warning: {w}");
        }
        for i in &self.infos {
            println!("{i}");
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

/// One deck file's §5 findings: the parse, the L1 lints (a bad directive value
/// is an error, the rest warnings), non-canonical tokens, an
/// unspliceable-frontmatter stamp block, the read-only unstamped-cards info,
/// and the dangling image / `at:` / trace-locator / dead-prerequisite checks.
/// `strict` is the explicit `deck check <file>` target: a deck that won't parse
/// at all is an error there (the user asked to lint exactly that file). In the
/// whole-folder scan (`strict` false), a non-charset parse failure only warns
/// (a broken deck only breaks itself, spec §5); an invalid-charset token is an
/// error in both.
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
    let deck = match alix::l1::parse_l1(name, &text) {
        Ok(deck) => deck,
        Err(e) => {
            let charset = matches!(e, alix::l1::L1Error::InvalidToken { .. });
            if strict || charset {
                report.error(format!("{}: {e}", path.display()));
            } else {
                report.warn(format!("{}: {e}", path.display()));
            }
            return;
        }
    };

    for lint in &deck.lints {
        let msg = lint_message(path, lint);
        if matches!(lint.kind, alix::l1::LintKind::BadValue { .. }) {
            report.error(msg);
        } else {
            report.warn(msg);
        }
    }

    // Valid-but-non-canonical tokens (accepted, warned; spec §1.6).
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

    // A deck needing an `id:` splice whose frontmatter is not a block mapping:
    // the stamp writer excludes it loudly (spec §2.3), surfaced here per deck.
    if deck.deck_token.is_none() && deck.frontmatter_span.is_some() && deck.frontmatter.unspliceable
    {
        report.warn(format!(
            "{}: cannot stamp: frontmatter is not a block mapping, so no `id:` can be spliced in",
            path.display()
        ));
    }

    // The read-only unstamped report (spec §2.2): cards awaiting a token.
    // Cloze holes and a reversed twin share one heading, so count distinct
    // lines (what one open stamps).
    let mut unstamped: Vec<usize> = deck
        .cards
        .iter()
        .filter(|c| c.token.is_none())
        .map(|c| c.line)
        .collect();
    unstamped.sort_unstable();
    unstamped.dedup();
    if !unstamped.is_empty() {
        report.info(format!(
            "{}: {} card(s) need a stamp (minted at next review open)",
            path.display(),
            unstamped.len()
        ));
    }

    // The resolved-Deck advisory checks (paths resolved against `img-dir`, the
    // source, etc.).
    if let Ok(deck) = Deck::load(path) {
        deck_resource_findings(&deck, report);
    }
}

/// The resolved-Deck advisory checks (all warnings, the deck still works): a
/// card citing a missing image, a frozen excerpt drifted from its source, a
/// trace locator that no longer resolves, a fact-card `at:` citation that
/// doesn't resolve, and a `% requires:` edge to a source-less deck that never
/// gates.
fn deck_resource_findings(deck: &Deck, report: &mut Report) {
    // A resolved image path that doesn't exist (advisory: the deck still works,
    // the web server just 404s the image).
    for card in &deck.cards {
        for image in [&card.image, &card.image_back].into_iter().flatten() {
            if !image.exists() {
                report.warn(format!(
                    "{}: card at line {} references a missing image: {}",
                    deck.subject,
                    card.line,
                    image.display()
                ));
            }
        }
    }
    // Frozen decks: a card's snapshot no longer matches the live source.
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
    // Trace decks: each `at:` locator must resolve into the live `source:`.
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
    // Fact decks: a card's `at:` citation that doesn't resolve.
    if !deck.is_trace() {
        let base = SourceBase::for_deck(deck);
        for card in &deck.cards {
            if let Some(at) = card.at.as_deref()
                && let Err(e) = base.excerpt(at)
            {
                report.warn(format!(
                    "{}: card at line {}: `at: {at}`: {e:#}",
                    deck.subject, card.line
                ));
            }
        }
    }
    // A `% requires:` to a source-less deck never gates this deck's exam.
    for prereq in alix::deck::nongating_prerequisites(deck) {
        report.warn(format!(
            "{}: requires source-less `{prereq}`: this edge doesn't gate its exam; \
             add a `source:` to `{prereq}` to make it a real prerequisite",
            deck.subject
        ));
    }
}

/// A one-line description of an L1 lint, prefixed with the deck path and line.
fn lint_message(path: &Path, lint: &alix::l1::Lint) -> String {
    use alix::l1::LintKind;
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
            "a `\\cloze` inside a cloze hole is literal text, not a nested hole".to_string()
        }
        LintKind::UnclosedComment => {
            "a `<!--` line that never closes with `-->` stays content".to_string()
        }
        LintKind::UnclosedFence => "a fence opened here never closes; everything after it \
             (cards included) was swallowed as its content"
            .to_string(),
    };
    format!("{}: line {}: {detail}", path.display(), lint.line)
}

/// The whole-workspace §5 check set for `dir`: every enumerated deck's
/// [`deck_findings`], plus the cross-deck checks that need the folder as a unit
/// duplicate deck/card tokens, orphaned store keys (with the coarse
/// fresh-mint-while-orphans-exist tell), and the pre-1.0 `.txt`-era detection.
/// Read-only: it opens files and the store but writes nothing.
fn workspace_findings(dir: &Path) -> Report {
    let mut report = Report::default();
    let deck_files = alix::workspace::deck_files(dir);
    for path in &deck_files {
        deck_findings(path, false, &mut report);
    }

    // Duplicate identity tokens across the folder (spec §2.4).
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
            dupe.token,
            dupe.keeper.0.display(),
            dupe.keeper.1,
            losers.join(", ")
        ));
    }

    // Orphaned store keys + the coarse fresh-mint tell (spec §5).
    let store_path = alix::workspace::root_store_path(dir);
    if let Ok(store) = Store::open(&store_path) {
        let mut known_cards: HashSet<String> = HashSet::new();
        let mut known_subjects: HashSet<String> = HashSet::new();
        // The §7 content fingerprints of freshly-minted cards (a token the store
        // has never scheduled): the precise lost-comment tell compares an
        // orphan's recorded content against these.
        let mut fresh_fps: HashSet<u64> = HashSet::new();
        for path in &deck_files {
            if let Ok(deck) = Deck::load(path) {
                known_subjects.insert(deck.subject.clone());
                for card in &deck.cards {
                    if let Some(id) = card.id() {
                        if store.get(&id).is_none() {
                            fresh_fps.insert(card.content_fingerprint);
                        }
                        known_cards.insert(id);
                    }
                }
            }
        }
        let orphans = store.orphans(&known_cards, &known_subjects);
        for key in &orphans.cards {
            if is_pre_l1_leftover(key) {
                report.warn(format!(
                    "orphaned store key `{key}` is a pre-1.0 numeric id (from before token \
                     identity); `alix reset --orphans` clears it"
                ));
            } else {
                report.warn(format!(
                    "orphaned store key (card) `{key}` matches no card in {}",
                    dir.display()
                ));
            }
        }
        for key in &orphans.decks {
            report.warn(format!(
                "orphaned store key (deck) `{key}` matches no deck in {}",
                dir.display()
            ));
        }
        // The fresh-mint-while-orphans-exist tell (spec §5), made precise by the
        // §6 content fingerprints: when an orphaned card's recorded content
        // matches a freshly-minted card, a stripped comment is the likely cause
        // and the progress is RECLAIMABLE (a review-open re-adopts the token);
        // when nothing matches, the non-reclaim is stated, not silent (a reformat
        // that also changed content stales the fingerprint — §1.7).
        if !orphans.cards.is_empty() && !fresh_fps.is_empty() {
            let reclaimable = orphans.cards.iter().any(|key| {
                let base = alix::token::parse_card_id(key).map_or(key.as_str(), |(t, _, _)| t);
                store
                    .records(base)
                    .is_some_and(|rec| fresh_fps.contains(&rec.content_fp))
            });
            if reclaimable {
                report.warn(
                    "a card lost its `<!-- id: -->` comment (e.g. a formatter stripped it): its \
                     old progress can be reclaimed — re-open the deck for review to re-adopt the \
                     token, or `alix reset --orphans` to discard it"
                        .to_string(),
                );
            } else {
                report.warn(
                    "orphaned card progress exists and fresh tokens were minted, but no \
                     fingerprint matched: the old content likely changed too (a reformat), so the \
                     progress cannot be reclaimed; `alix reset --orphans` clears it"
                        .to_string(),
                );
            }
        }
    }

    txt_era_findings(dir, &deck_files, &mut report);
    report
}

/// The pre-1.0 `.txt`-era detection (spec §5): every `.txt` file, and every
/// `.md` that did NOT enumerate as an L1 deck (prose / old-format), whose lines
/// look like old `# ` fronts. A conventional non-deck (`README.*`) and a
/// conflict copy are excluded (they are not stray old decks).
fn txt_era_findings(dir: &Path, deck_files: &[PathBuf], report: &mut Report) {
    let deck_set: HashSet<&Path> = deck_files.iter().map(PathBuf::as_path).collect();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    for path in entries {
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        // A conventional non-deck name (`README.*`) or a sync/backup conflict
        // copy is never a stray old deck, whichever extension it carries.
        if alix::workspace::is_conventional_non_deck(name)
            || alix::workspace::is_conflict_name(name)
        {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let candidate = match ext {
            "txt" => true,
            "md" => !deck_set.contains(path.as_path()),
            _ => false,
        };
        if candidate
            && let Ok(text) = std::fs::read_to_string(&path)
            && looks_txt_era(&text)
        {
            report.warn(format!(
                "{}: pre-1.0 `.txt`-era format; no converter ships: regenerate or hand-convert",
                path.display()
            ));
        }
    }
}

/// Whether `text` looks like a pre-1.0 `.txt`-era deck (spec §5): no L1 `## `
/// card fronts, but two or more column-0 `# ` lines shaped like the old front
/// marker. The two-plus threshold keeps a single-heading prose file (`# Notes`)
/// from tripping it.
fn looks_txt_era(text: &str) -> bool {
    let mut old_fronts = 0;
    for line in text.lines() {
        if line.starts_with("## ") {
            return false;
        }
        if let Some(rest) = line.strip_prefix("# ")
            && !rest.trim().is_empty()
        {
            old_fronts += 1;
        }
    }
    old_fronts >= 2
}

/// Whether a store key is a pre-1.0 numeric (u64-era) id: all ASCII digits and
/// not the 26-char canonical token length. A u64 is at most 20 digits, so a
/// wrong-length all-decimal key is a leftover from before token identity, never
/// a real token.
fn is_pre_l1_leftover(key: &str) -> bool {
    !key.is_empty()
        && key.len() != alix::token::TOKEN_LEN
        && key.bytes().all(|b| b.is_ascii_digit())
}

/// Lints an explicit deck-file (or workspace-dir) target: the `deck check`
/// path: prints each deck's card count and declared settings, then its §5
/// findings, failing only on an error.
fn check(decks: Vec<PathBuf>) -> Result<()> {
    let mut report = Report::default();
    for path in &decks {
        // A workspace directory: validate its declared icon + review config,
        // then skip the deck-load (which would error on a directory).
        if path.is_dir() && alix::workspace::is_workspace(path) {
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
        // The deck-info header (card count + declared settings), then findings.
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

/// Runs the health checks and prints the report: `✓` ok, `!` warn (an optional
/// feature is limited), `✗` fail (the core loop is broken). Exits non-zero only
/// on a fail, so a missing optional binary never breaks a script.
pub(crate) fn doctor_cmd(args: DoctorArgs) -> Result<()> {
    use alix::doctor::{self, Status};
    // A deck-file target = lint exactly that deck (syntax, duplicate answers,
    // trace locators) — the old `deck check`, now one more thing doctor checks.
    if let Some(path) = &args.dir {
        if path.is_file() {
            return check(vec![path.clone()]);
        }
        // A workspace directory: run detailed workspace-level checks.
        if alix::workspace::is_workspace(path) {
            check(vec![path.clone()])?;
        }
    }
    let (config_finding, config) = doctor::check_config(args.config.as_deref());
    let mut findings = vec![config_finding];
    // The same root/store resolution the launcher applies to `alix <dir>`.
    let (decks_dir, store_path) = match &args.dir {
        Some(path) => (path.clone(), workspace::root_store_path(path)),
        None => {
            let dir = config.decks_dir().context("cannot determine ~/decks")?;
            let store = workspace::root_store_path(&dir);
            (dir, store)
        }
    };
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
    // The whole-folder §5 check set: per-deck lints/tokens plus the cross-deck
    // duplicate-token, orphaned-key, and `.txt`-era checks (read-only; a
    // standalone deck file returned above, staying dedup-blind by design).
    if workspace_findings(&decks_dir).render() {
        failed = true;
    }
    // The costed end-to-end probe is opt-in: one real (tiny) request to the
    // configured backend, or one per backend with --all-backends.
    if args.backends || args.all_backends {
        println!();
        alix::backend::health::check(&config.ask, args.all_backends)?;
    }
    // The grading spot-check is opt-in and costed too. Probe outcomes never
    // flip the exit code — they are a spot check on the model, not a broken
    // setup; only an infrastructure error (no CLI, no login) fails the run.
    if args.grading {
        println!();
        grading_spot_check(&config)?;
    }
    if failed {
        bail!("doctor found problems (✗ above)");
    }
    Ok(())
}

/// Runs the grader-calibration probes against the configured backend and
/// prints a per-probe line plus a summary. Safety probes (a wrong answer that
/// must not pass) and fairness probes (a correct answer that should pass) are
/// reported with different weight: a safety failure means exam grades may
/// overstate understanding; a fairness failure only means the grader is harsh.
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
        // Warnings don't fail the check; the missing-icon path just adds one.
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

    fn w(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn doctor_flags_the_full_check_set() {
        // ONE fixture workspace exercising: both parse-error kinds (invalid
        // charset token, a bad directive value); duplicate deck and card
        // tokens; a non-canonical token; unspliceable frontmatter; the
        // reveal-on-cloze and indented-`##` lints; a missing image; the
        // unstamped-cards info; orphaned card/deck store keys plus the coarse
        // fresh-mint tell; a dangling `at:` citation on a fact card; a trace
        // deck whose `at:` locator no longer resolves; a `% requires:` to a
        // source-less deck (never gates); the `.txt`-era detection on BOTH a
        // `.txt` file and a prose `.md` with old `# ` fronts and no `## `
        // cards; and that a conventional non-deck name (`README.*`) is
        // excluded from the txt-era advisory on EITHER extension. NOT covered
        // here: the `UnknownKey`/`EmptyValue`/
        // `ClozeInHole`/`UnclosedComment`/`UnclosedFence` lints, and the
        // frozen-excerpt drift check (`drifted_cards`) — those have no doctor-
        // level test either, but are out of scope for this fixture.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        w(dir, "alix.toml", "title = \"Check Set\"\n");
        // Errors: an invalid-charset token, and a known key with a bad value.
        w(dir, "bad-token.md", "## q <!-- id: BAD1 -->\na\n");
        w(
            dir,
            "bad-value.md",
            "---\nreveal: bogus\n---\n## q <!-- id: bv1 -->\na\n",
        );
        // A duplicate DECK token (a whole-file copy): the decorated copy loses.
        w(
            dir,
            "dup-deck.md",
            "---\nid: dupdeck\n---\n## q <!-- id: dd1 -->\na\n",
        );
        w(
            dir,
            "dup-deck copy.md",
            "---\nid: dupdeck\n---\n## q <!-- id: dd1 -->\na\n",
        );
        // A duplicate CARD token across two DISTINCT decks (a copied card).
        w(
            dir,
            "card-dup.md",
            "---\nid: cda\n---\n## q <!-- id: cshared -->\na\n",
        );
        w(
            dir,
            "card-dup copy.md",
            "---\nid: cdb\n---\n## q <!-- id: cshared -->\nb\n",
        );
        // Unspliceable frontmatter (a flow mapping, no id) the stamp writer excludes.
        w(
            dir,
            "unspliceable.md",
            "---\n{source: [a]}\n---\n## q <!-- id: uq1 -->\nb\n",
        );
        // A `reveal:` on a cloze card (linted, not obeyed).
        w(
            dir,
            "cloze.md",
            "## Fill <!-- id: clz1 -->\n<!-- reveal: line -->\nthe \\cloze{a} gap\n",
        );
        // An indented `##` line (content, likely a mistype).
        w(
            dir,
            "indented.md",
            "## real <!-- id: ind1 -->\n  ## not a front\nanswer\n",
        );
        // A card citing a missing image.
        w(
            dir,
            "imgcard.md",
            "## pic <!-- id: img1 -->\n<!-- img: missing.png -->\nphoto\n",
        );
        // An unstamped deck (its card awaits a token).
        w(dir, "fresh.md", "## q\na\n");
        // A pre-1.0 `.txt`-era file (old `# ` fronts, never enumerated as L1).
        w(dir, "old-deck.txt", "# q1\na1\n# q2\na2\n");
        // A conventional non-deck name carrying old-`#`-shaped lines, but as a
        // `.txt` file: the txt-era check's `.txt` branch must apply the same
        // conventional-non-deck exclusion the `.md` branch already does.
        w(dir, "README.txt", "# q1\na1\n# q2\na2\n");
        // The OTHER `.txt`-era shape: a prose `.md` (no `## ` card, no
        // frontmatter, so it never enumerates as a deck) carrying old `# `
        // fronts — the txt-era check's `.md` branch, distinct from its `.txt`
        // branch above.
        w(
            dir,
            "old-format.md",
            "# First heading\nsome prose\n# Second heading\nmore prose\n",
        );
        // A trace deck whose one `at:` locator no longer resolves (the source
        // shrank): the trace-locator check.
        w(
            dir,
            "trace-bad.md",
            "---\ntrace: a walk\nsource: trace-src.txt\n---\n## hop <!-- id: thop1 -->\nstep\n<!-- at: 5-6 -->\n",
        );
        w(dir, "trace-src.txt", "one\ntwo\n");
        // A fact card whose `at:` citation doesn't resolve: the dangling-`at:`
        // check (distinct code path from the trace-locator check above).
        w(
            dir,
            "at-dangling.md",
            "---\nsource: .\n---\n## cited <!-- id: atd1 -->\nb\n<!-- at: missing.rs:1-2 -->\n",
        );
        // A sourced deck (has an exam) requiring a source-less prerequisite
        // (no exam of its own): the edge never gates, the dead-`% requires:`
        // check.
        w(dir, "sourceless.md", "## a <!-- id: sla1 -->\n1\n");
        w(
            dir,
            "gated.md",
            "---\nsource: https://example.test\nrequires: sourceless\n---\n## b <!-- id: gtd1 -->\n1\n",
        );

        // Seed the workspace store with an orphaned card key and an orphaned
        // deck key (matching no live card/deck).
        let mut store = alix::store::Store::open(dir.join("progress.json")).unwrap();
        store.get_or_insert("orphancard", 0);
        store.set_last_depth("ghostdeck.md", alix::depth::Depth::Recall);
        store.save().unwrap();

        let report = workspace_findings(dir);
        let errors = report.errors.join("\n");
        let warnings = report.warnings.join("\n");
        let infos = report.infos.join("\n");

        // Errors.
        assert!(
            errors.contains("fails the charset"),
            "invalid token: {errors}"
        );
        assert!(
            errors.contains("invalid value"),
            "bad directive value: {errors}"
        );
        // Warnings.
        assert!(warnings.contains("duplicate deck token"), "{warnings}");
        assert!(warnings.contains("duplicate card token"), "{warnings}");
        assert!(
            warnings.contains("not canonical"),
            "non-canonical token: {warnings}"
        );
        assert!(warnings.contains("orphaned store key (card)"), "{warnings}");
        assert!(warnings.contains("orphaned store key (deck)"), "{warnings}");
        // The orphan here (`orphancard`) carries no content records, so no fresh
        // card's content can match it: the precise "no fingerprint matched" tell.
        assert!(
            warnings.contains("no fingerprint matched"),
            "fresh-mint tell (no match): {warnings}"
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
            warnings.contains("`.txt`-era") && warnings.contains("old-deck.txt"),
            "txt-era (.txt branch): {warnings}"
        );
        assert!(
            warnings.contains("`.txt`-era") && warnings.contains("old-format.md"),
            "txt-era (.md branch, zero `##`): {warnings}"
        );
        assert!(
            !warnings.contains("README.txt"),
            "a conventional non-deck name must never trip the txt-era advisory, any extension: {warnings}"
        );
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
        // Info.
        assert!(infos.contains("need a stamp"), "unstamped info: {infos}");
    }

    #[test]
    fn a_freshly_minted_token_matching_an_orphans_content_is_reported_reclaimable() {
        // The refined §5 tell: an orphaned token whose recorded content
        // fingerprint equals a freshly-minted card's content is a stripped-
        // comment case, reported as RECLAIMABLE (distinct from the no-match
        // form). The live card carries a fresh token with no store entry; the
        // orphan token carries the same content's records + a schedule.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        w(
            dir,
            "deck.md",
            "---\nid: abcdefghjkmnpqrstvwxyz6789\n---\n## Q <!-- id: newtoken -->\nA\n",
        );
        let mut store = alix::store::Store::open(dir.join("progress.json")).unwrap();
        // The orphan: a token with a schedule + records whose content matches Q/A.
        let fp = alix::l1::content_fingerprint("Q", &["A".to_string()]);
        let orphan = "orphantoken0000000000000000";
        store.ensure_records_raw(orphan, fp, &[]);
        store.get_or_insert(orphan, 0);
        store.save().unwrap();

        let report = workspace_findings(dir);
        let warnings = report.warnings.join("\n");
        assert!(
            warnings.contains("can be reclaimed"),
            "reclaimable tell expected: {warnings}"
        );
        assert!(
            !warnings.contains("no fingerprint matched"),
            "must not also print the no-match form: {warnings}"
        );
    }

    #[test]
    fn all_decimal_wrong_length_store_keys_are_tagged_pre_l1_leftovers() {
        // A pre-1.0 numeric (u64-era) id lingering in the store matches no live
        // card and is tagged as a leftover, distinct from a generic orphan.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // A canonical (26-char) live card token, so it produces no
        // non-canonical noise and any mention would be a false orphan flag.
        let live = "abcdefghjkmnpqrstvwxyz2345";
        w(
            dir,
            "deck.md",
            &format!("---\nid: abcdefghjkmnpqrstvwxyz6789\n---\n## q <!-- id: {live} -->\na\n"),
        );
        let mut store = alix::store::Store::open(dir.join("progress.json")).unwrap();
        store.get_or_insert(live, 0); // the live card, not an orphan
        store.get_or_insert("1234567890123456", 0); // a u64-era numeric leftover
        store.save().unwrap();

        let report = workspace_findings(dir);
        let warnings = report.warnings.join("\n");
        assert!(
            warnings.contains("pre-1.0 numeric id") && warnings.contains("1234567890123456"),
            "the numeric leftover must be tagged as pre-1.0: {warnings}"
        );
        // The live card is canonical and matched: it is never flagged at all.
        assert!(
            !warnings.contains(live),
            "the live card must not be flagged: {warnings}"
        );
    }
}
