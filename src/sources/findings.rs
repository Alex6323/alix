use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use super::{
    DeckSources, DisplayPath, Manifest, Roots, SourceError,
    commands::{Drift, live_state, over_limit, topic_measures},
    load_verified,
    roots::{root_fix, root_issue},
    stray_copies,
};
use crate::{config::ExamConfig, deck::Deck, workspace::WorkspaceFiles};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Findings {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

pub fn deck_findings(
    workspace_root: &Path,
    deck: &Deck,
    check_drift: bool,
    exam: &ExamConfig,
) -> Findings {
    let mut findings = Findings::default();
    let Some(deck_id) = deck.deck_token.as_deref() else {
        return findings;
    };
    let dir = WorkspaceFiles::new(workspace_root).sources_for(deck_id);
    let manifest = match load_verified(workspace_root, deck_id) {
        Err(SourceError::LinkedFolder(path))
            if path == WorkspaceFiles::new(workspace_root).sources() =>
        {
            return findings;
        }
        Err(error) => {
            findings.errors.push(error.to_string());
            return findings;
        }
        Ok(None) => {
            residue_notes(&dir, stray_copies(&dir), &mut findings);
            return findings;
        }
        Ok(Some(sources)) => {
            match sources.validate_bindings(deck) {
                // Doctor's topic lint already reports a duplicated id.
                Ok(()) | Err(SourceError::DuplicateTopicId(_)) => {}
                Err(error) => findings.errors.push(error.to_string()),
            }
            residue_notes(&dir, sources.residue(), &mut findings);
            match topic_measures(deck, &sources, exam) {
                Ok(measured) => findings
                    .warnings
                    .extend(over_limit(&measured).iter().map(ToString::to_string)),
                Err(error) => findings.errors.push(error.to_string()),
            }
            let DeckSources { manifest, .. } = sources;
            manifest
        }
    };
    let roots = match Roots::load(&dir) {
        Ok(roots) => roots.map(|roots| roots.roots).unwrap_or_default(),
        Err(error) => {
            findings.errors.push(error.to_string());
            return findings;
        }
    };
    if check_drift {
        drift_findings(deck_id, &manifest, &roots, &mut findings);
    }
    findings
}

fn residue_notes(dir: &Path, residue: Result<Vec<String>, SourceError>, findings: &mut Findings) {
    match residue {
        Ok(names) => {
            for name in names {
                findings.notes.push(format!(
                    "`{}` belongs to no row; the next source command removes it",
                    dir.join(name).display()
                ));
            }
        }
        Err(error) => findings.errors.push(error.to_string()),
    }
}

fn drift_findings(
    deck_id: &str,
    manifest: &Manifest,
    roots: &BTreeMap<String, PathBuf>,
    findings: &mut Findings,
) {
    let mut rows: Vec<(DisplayPath, &str)> = manifest
        .sources
        .iter()
        .filter_map(|(id, row)| Some((row.path.parse().ok()?, id.as_str())))
        .collect();
    rows.sort();
    let mut unchecked: BTreeMap<&str, usize> = BTreeMap::new();
    for (display, id) in &rows {
        let usable = roots
            .get(&display.key)
            .filter(|_| root_issue(roots, &display.key).is_none());
        let Some(dir) = usable else {
            *unchecked.entry(display.key.as_str()).or_default() += 1;
            continue;
        };
        let (key, path) = (&display.key, display.to_string());
        let line = match live_state(id, display, dir).0 {
            Drift::Same | Drift::Unchecked => continue,
            Drift::Changed => format!(
                "source `{path}` changed since it was frozen; `alix doctor --repair-sources` \
                 refreezes it"
            ),
            Drift::Missing => format!(
                "source `{path}` is missing from root `{key}`; the frozen copy stays until \
                 `alix deck source remove` drops it"
            ),
            Drift::Linked => format!(
                "source `{path}` is a symbolic link or leaves root `{key}`, so it is not \
                 checked for drift"
            ),
            Drift::Unreadable => {
                format!("source `{path}` cannot be read, so it is not checked for drift")
            }
        };
        findings.warnings.push(line);
    }
    for (key, count) in unchecked {
        let sources = if count == 1 { "source" } else { "sources" };
        let issue = root_issue(roots, key).unwrap_or_default();
        let fix = root_fix(deck_id, key);
        findings.warnings.push(format!(
            "`{key}`: {issue}, {count} {sources} not checked for drift {fix}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;
    use crate::sources::{
        MANIFEST, ROOTS, ROOTS_VERSION, commands,
        manifest::tests::{DECK, faults, fixture},
        source_id,
        testkit::{TOPIC_A, Workspace},
    };

    const FIXTURE_DECK: &str =
        "---\nid: \"deck-fixture\"\n---\n# A\n<!-- id: topic-aaaa -->\n## q\na\n";

    #[test]
    fn doctor_reports_every_source_integrity_fault_as_an_error() {
        for (name, apply, _) in faults() {
            let fixture = fixture();
            let deck_path = fixture.root.path().join("d.md");
            fs::write(&deck_path, FIXTURE_DECK).unwrap();
            let deck = Deck::load(&deck_path).unwrap();
            assert_eq!(
                Findings::default(),
                deck_findings(fixture.root.path(), &deck, false, &ExamConfig::default()),
                "{name}: the intact fixture draws no finding"
            );
            apply(&fixture);
            let refused = load_verified(fixture.root.path(), DECK)
                .expect_err("the fault table only holds faults")
                .to_string();
            let findings = deck_findings(fixture.root.path(), &deck, false, &ExamConfig::default());
            let expected = if name == "sources root replaced by a symlink" {
                Vec::new()
            } else {
                vec![refused]
            };
            assert_eq!(
                expected, findings.errors,
                "{name}: the verified load's refusal is the one error, except a linked \
                 `sources/` root, which the workspace sweep reports once"
            );
        }
    }

    #[test]
    fn a_copy_folder_without_a_manifest_declares_nothing_and_is_reported() {
        let fixture = fixture();
        let deck_path = fixture.root.path().join("d.md");
        fs::write(&deck_path, FIXTURE_DECK).unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        fs::remove_file(fixture.dir.join(MANIFEST)).unwrap();
        let copies: Vec<PathBuf> = stray_copies(&fixture.dir)
            .unwrap()
            .into_iter()
            .map(|name| fixture.dir.join(name))
            .collect();
        assert_eq!(2, copies.len(), "precondition: both copies remain");

        let findings = deck_findings(fixture.root.path(), &deck, true, &ExamConfig::default());

        assert!(
            findings.errors.is_empty() && findings.warnings.is_empty(),
            "no manifest declares no source, so nothing is damaged or drifted: {findings:?}"
        );
        assert_eq!(
            copies
                .iter()
                .map(|copy| format!(
                    "`{}` belongs to no row; the next source command removes it",
                    copy.display()
                ))
                .collect::<Vec<_>>(),
            findings.notes,
            "every copy is residue"
        );
    }

    #[test]
    fn drift_is_reported_only_with_the_flag_and_names_keys_without_a_root_with_the_fix() {
        let workspace = Workspace::new();
        workspace.write_live("a.rs", b"a\n");
        workspace.write_live("b.rs", b"b\n");
        workspace.write_live("same.rs", b"same\n");
        workspace
            .add(&["a.rs", "b.rs", "same.rs"], &[TOPIC_A])
            .unwrap();
        for (key, names) in [("dl", &["p.txt", "q.txt"][..]), ("web", &["w.txt"][..])] {
            let dir = workspace.root().join(key);
            let mut files = Vec::new();
            for name in names {
                fs::create_dir_all(&dir).unwrap();
                fs::write(dir.join(name), format!("{key} {name}\n")).unwrap();
                files.push(dir.join(name));
            }
            commands::add(
                commands::AddRequest {
                    deck: &workspace.deck(),
                    files,
                    root: Some((key.to_string(), dir)),
                    key: None,
                    topics: vec![TOPIC_A.to_string()],
                },
                &ExamConfig::default(),
            )
            .unwrap();
        }
        commands::root_unset(&workspace.deck(), "dl").unwrap();
        let mut roots = Roots::load(&workspace.dir()).unwrap().unwrap();
        roots
            .roots
            .insert("web".to_string(), PathBuf::from("relative/web"));
        roots.save(&workspace.dir()).unwrap();
        workspace.write_live("a.rs", b"a, edited\n");
        fs::remove_file(workspace.live().join("b.rs")).unwrap();
        let deck = Deck::load(workspace.deck()).unwrap();

        let quiet = deck_findings(workspace.root(), &deck, false, &ExamConfig::default());
        assert_eq!(
            Findings::default(),
            quiet,
            "without the flag: no drift line"
        );

        let loud = deck_findings(workspace.root(), &deck, true, &ExamConfig::default());
        assert_eq!(
            vec![
                "source `sdk:a.rs` changed since it was frozen; `alix doctor --repair-sources` \
                 refreezes it"
                    .to_string(),
                "source `sdk:b.rs` is missing from root `sdk`; the frozen copy stays until `alix \
                 deck source remove` drops it"
                    .to_string(),
                "`dl`: no root on this machine, 2 sources not checked for drift (alix deck source \
                 root set deck-fixture dl <dir>)"
                    .to_string(),
                "`web`: root `relative/web` is not an absolute path, 1 source not checked for \
                 drift (alix deck source root set deck-fixture web <dir>)"
                    .to_string(),
            ],
            loud.warnings,
            "with the flag: one line per drifted or missing source, then one per unchecked key"
        );
        assert!(
            loud.errors.is_empty() && loud.notes.is_empty(),
            "drift is never an error: {loud:?}"
        );

        let gone = workspace.root().join("gone");
        roots.roots.insert("web".to_string(), gone.clone());
        roots.save(&workspace.dir()).unwrap();
        let line = deck_findings(workspace.root(), &deck, true, &ExamConfig::default())
            .warnings
            .pop()
            .unwrap();
        assert_eq!(
            format!(
                "`web`: root `{}` is not an existing folder, 1 source not checked for drift \
                 (alix deck source root set deck-fixture web <dir>)",
                gone.display()
            ),
            line,
            "a saved root that is gone is named, not crashed on"
        );
    }

    #[test]
    fn doctor_reports_bindings_to_no_topic_and_an_unreadable_roots_file_as_errors() {
        let workspace = Workspace::new();
        workspace.write_live("a.rs", b"a\n");
        workspace.add(&["a.rs"], &[TOPIC_A]).unwrap();
        let deck = Deck::load(workspace.deck()).unwrap();
        workspace.rewrite_manifest("\"topic-alpha\"", "\"topic-gone\"");
        assert_eq!(
            vec![format!(
                "source `{}` is bound to `topic-gone`, which names no topic of the deck",
                source_id(b"a\n")
            )],
            deck_findings(workspace.root(), &deck, false, &ExamConfig::default()).errors,
            "a binding to no topic"
        );
        workspace.rewrite_manifest("\"topic-gone\"", "\"topic-alpha\"");
        fs::write(
            workspace.dir().join(ROOTS),
            format!("version = {ROOTS_VERSION}\nnote = 1\n\n[roots]\n"),
        )
        .unwrap();
        let errors = deck_findings(workspace.root(), &deck, false, &ExamConfig::default()).errors;
        assert_eq!(
            1,
            errors.len(),
            "a roots file that fails to load: {errors:?}"
        );
        assert!(
            errors[0].contains(ROOTS),
            "the roots file is named: {errors:?}"
        );
    }
}
