use std::path::{Path, PathBuf};

const WIRE_FILES: &[&str] = &[
    "src/serve/dto.rs",
    "src/serve/contract.rs",
    "mobile/alix/rust/src/api/review.rs",
    "mobile/alix/rust/src/api/exam.rs",
    "mobile/alix/rust/src/api/listing.rs",
    "mobile/alix/rust/src/frb_generated.rs",
    "mobile/alix/lib/src/rust/frb_generated.dart",
    "mobile/alix/lib/src/rust/frb_generated.io.dart",
    "mobile/alix/lib/src/rust/frb_generated.web.dart",
    "mobile/alix/lib/bridge/review_bridge.dart",
    "mobile/alix/lib/bridge/walk_bridge.dart",
    "mobile/alix/lib/bridge/exam_bridge.dart",
    "mobile/alix/lib/server_client.dart",
    "mobile/alix/lib/review/review_models.dart",
    "mobile/alix/lib/walk/walk_models.dart",
    "mobile/alix/lib/review/review_card.dart",
    "mobile/alix/lib/review/review_view.dart",
    "mobile/alix/lib/walk/walk_view.dart",
    "mobile/alix/lib/review_screen.dart",
    "mobile/alix/lib/walk_screen.dart",
    "mobile/alix/lib/exam_screen.dart",
];
const WIRE_DIRS: &[(&str, &str)] = &[
    ("tests/contracts", "json"),
    ("mobile/alix/lib/src/rust/api", "dart"),
];
const WIRE_PROSE: &[&str] = &["docs/API.md"];
// flutter_rust_bridge_codegen writes these region markers into every generated file.
const GENERATOR_MARKER: &str = concat!("// Sec", "tion: ");

fn names_a_wire_spelling(line: &str, needle: &str) -> bool {
    let lower = line.to_lowercase();
    let in_delimited = |delimiter: char| {
        lower
            .split(delimiter)
            .skip(1)
            .step_by(2)
            .any(|span| span.contains(needle))
    };
    if in_delimited('`') || in_delimited('"') {
        return true;
    }
    lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| word.contains(needle) && word != needle && word != format!("{needle}s"))
}

#[test]
fn every_wire_surface_says_topic_never_the_old_heading_word() {
    let needle = concat!("sec", "tion");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> = WIRE_FILES.iter().map(|f| root.join(f)).collect();
    for (dir, ext) in WIRE_DIRS {
        let found: Vec<_> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == *ext))
            .collect();
        assert!(
            !found.is_empty(),
            "{dir}: no .{ext} file, the law lost a surface"
        );
        files.extend(found);
    }
    let mut hits = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap_or_else(|e| {
            panic!(
                "{}: {e} (a listed wire file moved; update the list)",
                file.display()
            )
        });
        let generated = file
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("frb_generated"));
        for (n, line) in text.lines().enumerate() {
            if generated && line.trim_start().starts_with(GENERATOR_MARKER) {
                continue;
            }
            if line.to_lowercase().contains(needle) {
                hits.push(format!(
                    "{}:{}: still says {needle}: {line}",
                    file.display(),
                    n + 1
                ));
            }
        }
    }
    for file in WIRE_PROSE {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        for (n, line) in text.lines().enumerate() {
            if names_a_wire_spelling(line, needle) {
                hits.push(format!(
                    "{file}:{}: wire spelling with {needle}: {line}",
                    n + 1
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "{} lines:\n{}",
        hits.len(),
        hits.join("\n")
    );
}

#[test]
fn a_wire_spelling_is_code_quoted_or_an_identifier_never_bare_prose() {
    let needle = concat!("sec", "tion");
    let rows = [
        ("see section 4.3 below", false),
        ("the sections of this document", false),
        ("the `foo_section` field", true),
        ("a \"section\" key", true),
        ("field section_first is set", true),
        ("type ExamSectionDto", true),
    ];
    for (line, expected) in rows {
        assert_eq!(
            names_a_wire_spelling(line, needle),
            expected,
            "line: {line}"
        );
    }
}
