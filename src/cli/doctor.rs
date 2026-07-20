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
}

impl Report {
    fn error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }
    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
    fn render(&self) -> bool {
        for e in &self.errors {
            eprintln!("error: {e}");
        }
        for w in &self.warnings {
            eprintln!("warning: {w}");
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
    if !unstamped.is_empty() {
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
        LintKind::UnknownKey { key } => unknown_key_hint(key),
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
        LintKind::AudioNotSupported => "a `\\audio{}` marker is reserved for a future release; \
             it stays as literal text, not audio"
            .to_string(),
        LintKind::MarkerBadOption { key } if key.is_empty() => "an empty marker option is \
             ignored; write `alt: ...` or drop the braces"
            .to_string(),
        LintKind::MarkerBadOption { key } => format!(
            "marker option `{key}` is not recognized (the only valid option is `alt: ...`, once)"
        ),
        LintKind::MarkerMalformed { name } => format!(
            "the `\\{name}` marker is malformed (empty or an unclosed argument); \
             it renders as literal text, not a marker"
        ),
    };
    format!("{}: line {}: {detail}", path.display(), lint.line)
}

fn unknown_key_hint(key: &str) -> String {
    match key {
        "img" | "img-back" => format!(
            "`{key}:` is gone; use the `\\image{{...}}` marker instead \
             (a line in the card's front or answer)"
        ),
        "math" => "`math:` is retired; it never had any effect, so the line can just be deleted"
            .to_string(),
        "img-dir" => "`img-dir:` is gone; use `image-dir:` instead".to_string(),
        "occlude" | "audio" | "audio-back" | "img-alt" => {
            format!("`{key}:` was a reserved key that has been removed; it never had any effect")
        }
        _ => format!("unknown key `{key}` (ignored)"),
    }
}

fn workspace_findings(dir: &Path) -> Report {
    let mut report = Report::default();
    let deck_files = alix::workspace::deck_files(dir);
    for path in &deck_files {
        deck_findings(path, false, &mut report);
    }

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

    let store_path = alix::workspace::root_store_path(dir);
    if let Ok(store) = Store::open(&store_path) {
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

    report
}

fn check(decks: Vec<PathBuf>) -> Result<()> {
    let mut report = Report::default();
    for path in &decks {
        // Deck::load would error on a directory, so a workspace target is
        // handled separately here.
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

// Exits non-zero only on a hard fail; a missing optional binary (a warn)
// never breaks a script.
pub(crate) fn doctor_cmd(args: DoctorArgs) -> Result<()> {
    use alix::doctor::{self, Status};
    if let Some(path) = &args.dir {
        if path.is_file() {
            return check(vec![path.clone()]);
        }
        if alix::workspace::is_workspace(path) {
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

    fn w(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn doctor_flags_the_full_check_set() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        w(dir, "alix.toml", "title = \"Check Set\"\n");
        w(dir, "bad-token.md", "## q <!-- id: BAD1 -->\na\n");
        w(
            dir,
            "bad-value.md",
            "---\nreveal: bogus\n---\n## q <!-- id: bv1 -->\na\n",
        );
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
        w(
            dir,
            "unspliceable.md",
            "---\n{source: [a]}\n---\n## q <!-- id: uq1 -->\nb\n",
        );
        w(
            dir,
            "cloze.md",
            "## Fill <!-- id: clz1 -->\n<!-- reveal: line -->\nthe \\cloze{a} gap\n",
        );
        w(
            dir,
            "indented.md",
            "## real <!-- id: ind1 -->\n  ## not a front\nanswer\n",
        );
        w(
            dir,
            "imgcard.md",
            "## pic <!-- id: img1 -->\nphoto\n\\image{missing.png}\n",
        );
        w(dir, "fresh.md", "## q\na\n");
        w(
            dir,
            "trace-bad.md",
            "---\ntrace: a walk\nsource: trace-src.txt\n---\n## hop <!-- id: thop1 -->\nstep\n<!-- at: 5-6 -->\n",
        );
        w(dir, "trace-src.txt", "one\ntwo\n");
        w(
            dir,
            "at-dangling.md",
            "---\nsource: .\n---\n## cited <!-- id: atd1 -->\nb\n<!-- at: missing.rs:1-2 -->\n",
        );
        w(dir, "sourceless.md", "## a <!-- id: sla1 -->\n1\n");
        w(
            dir,
            "gated.md",
            "---\nsource: https://example.test\nrequires: sourceless\n---\n## b <!-- id: gtd1 -->\n1\n",
        );

        let mut store = alix::store::Store::open(dir.join("progress.json")).unwrap();
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
}
