//! `alix doctor`: health checks for the config, progress store, decks folder,
//! and optional external CLIs — plus the per-deck lint (`check`) it shares
//! with a deck-file target.

use std::path::PathBuf;

use alix::{
    deck::Deck,
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

fn check(decks: Vec<PathBuf>) -> Result<()> {
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for path in &decks {
        // A workspace directory: validate its declared icon, then skip the
        // deck-load (which would error on a directory).
        if path.is_dir() && alix::workspace::is_workspace(path) {
            if let Some(rel) = alix::workspace::manifest_icon(path)
                && !path.join(&rel).is_file()
            {
                warnings += 1;
                eprintln!(
                    "warning: {}: `icon = \"{rel}\"` points at a missing file",
                    path.display()
                );
            }
            for complaint in alix::config::local_review_lint(path) {
                warnings += 1;
                eprintln!("warning: {}: {complaint}", path.display());
            }
            continue;
        }
        match Deck::load(path) {
            Err(e) => {
                errors += 1;
                eprintln!("error: {e}");
            }
            Ok(deck) => {
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
                for (a, b) in deck.duplicates() {
                    warnings += 1;
                    eprintln!(
                        "warning: {}: cards at lines {} and {} have identical answers \
                         and share their learning progress",
                        deck.subject, a.line, b.line
                    );
                }
                // Image paths are resolved but never checked at load time, so a
                // missing file is reported here (advisory: the deck still works,
                // the web server just 404s the image).
                for card in &deck.cards {
                    for image in [&card.image, &card.image_back].into_iter().flatten() {
                        if !image.exists() {
                            warnings += 1;
                            eprintln!(
                                "warning: {}: card at line {} references a missing image: {}",
                                deck.subject,
                                card.line,
                                image.display()
                            );
                        }
                    }
                }

                // Frozen decks: warn when a card's snapshot no longer matches the
                // live source (the file changed or is gone), so the learner can
                // update or drop that card.
                for drift in alix::trace::drifted_cards(&deck) {
                    warnings += 1;
                    let what = if drift.gone {
                        "source file is gone"
                    } else {
                        "no longer found in the source"
                    };
                    eprintln!(
                        "warning: {}: card at line {} — frozen excerpt {} ({})",
                        deck.subject, drift.line, what, drift.at
                    );
                }

                // Trace decks: validate each `% at:` locator resolves into the
                // live `% source:` — catches drift (a file that shrank or was
                // renamed) before a walk hits it, like the duplicate/image checks.
                if deck.is_trace() && !deck.cards.is_empty() {
                    match Trace::from_deck(&deck) {
                        Ok(trace) => {
                            for issue in trace.lint_locators() {
                                warnings += 1;
                                let line = deck.cards.get(issue.checkpoint).map_or(0, |c| c.line);
                                eprintln!(
                                    "warning: {}: checkpoint at line {}: {}",
                                    deck.subject, line, issue.message
                                );
                            }
                        }
                        Err(e) => {
                            warnings += 1;
                            eprintln!("warning: {}: {e:#}", deck.subject);
                        }
                    }
                }

                // Fact decks: a card may also cite its source with `% at:`; warn
                // when a citation doesn't resolve (a moved/shrunk file), so a
                // hand-written or generated citation is caught before review.
                if !deck.is_trace() {
                    let base = SourceBase::for_deck(&deck);
                    for card in &deck.cards {
                        if let Some(at) = card.at.as_deref()
                            && let Err(e) = base.excerpt(at)
                        {
                            warnings += 1;
                            eprintln!(
                                "warning: {}: card at line {}: `% at: {at}` — {e:#}",
                                deck.subject, card.line
                            );
                        }
                    }
                }

                // A `% requires:` to a source-less deck never gates this deck's exam
                // (`is_locked` sees through an exam-less prerequisite), so a sourced
                // deck listing one likely meant it to gate — flag the dead edge.
                for prereq in alix::deck::nongating_prerequisites(&deck) {
                    warnings += 1;
                    eprintln!(
                        "warning: {}: requires source-less `{prereq}` — this edge \
                         doesn't gate its exam; add a `% source:` to `{prereq}` to \
                         make it a real prerequisite",
                        deck.subject
                    );
                }
            }
        }
    }
    // Warnings (e.g. duplicate answers) are advisory and don't fail the check;
    // only a deck that won't parse is an error.
    if errors > 0 || warnings > 0 {
        eprintln!("{errors} error(s), {warnings} warning(s)");
    }
    if errors > 0 {
        bail!("{errors} error(s) found");
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
        std::fs::write(dir.path().join("a.txt"), "# a\n\t1\n").unwrap();
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
        std::fs::write(dir.path().join("a.txt"), "# a\n\t1\n").unwrap();
        assert!(check(vec![dir.path().to_path_buf()]).is_ok());
    }
}
