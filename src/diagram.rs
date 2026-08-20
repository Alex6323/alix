//! Rendering mermaid fences to SVG by shelling out to the `sekien` CLI.
//!
//! The renderer is an authoring-time tool: its output is frozen as a
//! deck-owned asset and every client just displays an SVG. Nothing here runs
//! during review.

use std::{
    hash::Hasher,
    io::{Read, Write},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use twox_hash::XxHash64;

/// The CLI alix shells out to. Named here rather than configured: a second
/// renderer would produce different pictures for the same deck.
pub const COMMAND: &str = "sekien";

/// One fence's outcome: the SVG, or the renderer's own message for it.
pub type Rendered = Result<String, String>;

/// The info string that marks a fence as a diagram.
const LANGUAGE: &str = "mermaid";

/// A mermaid fence inside one block.
#[derive(Debug, PartialEq, Eq)]
pub struct Fence {
    /// The interior, verbatim and without the delimiter lines. This is the
    /// renderer's input and the whole fingerprint preimage.
    pub source: String,
    /// Index into the block's lines of the opening delimiter.
    pub opener: usize,
}

/// Every mermaid fence in `lines`, in document order.
///
/// The info string is compared case-insensitively after trimming, so
/// ```` ```mermaid ````, ```` ``` mermaid ```` and ```` ~~~MERMAID ```` all
/// count; any other language does not. An unclosed fence runs to the end of
/// the block, matching how the stream already treats fenced interiors.
pub fn fences(lines: &[String]) -> Vec<Fence> {
    let mut found = Vec::new();
    // `opener` is None while a NON-mermaid fence is open: such a fence still
    // has to be consumed, or its interior could be read as a diagram, but it
    // never yields one. An Option carries that where a magic index would not:
    // both exit paths must handle it, so neither can drop the distinction.
    let mut open: Option<(char, Option<usize>, Vec<String>)> = None;
    let close = |opener: Option<usize>, body: Vec<String>, found: &mut Vec<Fence>| {
        if let Some(opener) = opener {
            found.push(Fence {
                source: body.join("\n"),
                opener,
            });
        }
    };
    for (index, line) in lines.iter().enumerate() {
        match &mut open {
            Some((ch, _, body)) => {
                if crate::parser::closes_fence(line, *ch) {
                    let (_, opener, body) = open.take().expect("the fence is open");
                    close(opener, body, &mut found);
                } else {
                    body.push(line.clone());
                }
            }
            None => {
                if let Some(ch) = crate::parser::fence_opener(line) {
                    let info = line.trim_start_matches(ch).trim();
                    let opener = info.eq_ignore_ascii_case(LANGUAGE).then_some(index);
                    open = Some((ch, opener, Vec::new()));
                }
            }
        }
    }
    if let Some((_, opener, body)) = open {
        close(opener, body, &mut found);
    }
    found
}

/// The frozen-forever preimage: the fence's interior bytes and nothing else.
///
/// Deliberately excludes the renderer version, the mermaid version, and the
/// theme. A frozen SVG is evidence, not a cache: putting a version in here
/// would invalidate every diagram in every shared deck on any upgrade, for a
/// recipient who may not even have the renderer installed. Re-rendering stays
/// a deliberate authoring act.
pub fn fingerprint(source: &str) -> String {
    let mut hasher = XxHash64::default();
    hasher.write(source.as_bytes());
    format!("xxh64-{:016x}", hasher.finish())
}

/// Renders `sources` in one long-lived process, returning one outcome per
/// input in input order.
///
/// Two protocol facts drive this, both measured against sekien 0.4.1 rather
/// than assumed: a diagram that fails emits NOTHING on stdout, so inputs and
/// SVGs cannot be paired positionally, and the process exits 0 even when
/// diagrams fail, so the exit code is never a per-diagram verdict. Both
/// streams are therefore correlated by the `--meta` id.
pub fn render_batch(command: &str, sources: &[String], timeout: Duration) -> Result<Vec<Rendered>> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let mut child = Command::new(command)
        .arg("--meta")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run '{command}' — is it installed?"))?;

    let mut stdin = child.stdin.take().expect("stdin was piped");
    let stdout_pipe = child.stdout.take().expect("stdout was piped");
    let stderr_pipe = child.stderr.take().expect("stderr was piped");

    let payload = sources.join("\0").into_bytes();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&payload);
    });
    // Both pipes drain concurrently so a full one cannot deadlock the child.
    let out = std::thread::spawn(move || drain(stdout_pipe));
    let err = std::thread::spawn(move || drain(stderr_pipe));

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("'{command}' did not finish within {}s", timeout.as_secs());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(error).context("cannot wait for the renderer"),
        }
    }
    let _ = writer.join();
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();

    let svgs = by_meta_id(&stdout);
    let errors = by_meta_id(&stderr);
    let mut outcomes = Vec::with_capacity(sources.len());
    for index in 0..sources.len() {
        // sekien's ids are 1-based and count inputs, not outputs.
        let id = index + 1;
        let svg = svgs.iter().find(|(found, _)| *found == id);
        let error = errors.iter().find(|(found, _)| *found == id);
        outcomes.push(match (svg, error) {
            (Some((_, svg)), _) if !svg.is_empty() => Ok(svg.clone()),
            (_, Some((_, message))) => Err(message.clone()),
            _ => Err("the renderer returned nothing for this diagram".to_string()),
        });
    }
    Ok(outcomes)
}

fn drain<R: Read>(mut pipe: R) -> String {
    let mut buffer = Vec::new();
    let _ = pipe.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Splits a `--meta` stream into `(id, body)` pairs. The marker is
/// `<!-- {"id": N} -->`; anything before the first marker is renderer preamble
/// and is dropped.
fn by_meta_id(stream: &str) -> Vec<(usize, String)> {
    const OPEN: &str = "<!-- {\"id\":";
    let mut found = Vec::new();
    let mut rest = stream;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(close) = after.find("-->") else {
            break;
        };
        let id: usize = match after[..close].trim().trim_end_matches('}').trim().parse() {
            Ok(id) => id,
            Err(_) => {
                rest = &after[close + 3..];
                continue;
            }
        };
        let body_start = start + OPEN.len() + close + 3;
        let body = &rest[body_start..];
        let end = body.find(OPEN).unwrap_or(body.len());
        found.push((id, body[..end].trim_matches(['\0', '\n', ' ']).to_string()));
        rest = &body[end..];
    }
    found
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::testutil::{exec_lock, fake_cli};

    /// A fake sekien: drains stdin, then replays a prepared stdout and stderr.
    /// The correlation logic is what is under test, not the real renderer.
    fn fake_sekien(dir: &Path, stdout: &str, stderr: &str) -> PathBuf {
        let out = dir.join("out");
        let err = dir.join("err");
        std::fs::write(&out, stdout).unwrap();
        std::fs::write(&err, stderr).unwrap();
        fake_cli(
            dir,
            &format!(
                "cat >/dev/null; cat {}; cat {} >&2; exit 0",
                out.display(),
                err.display()
            ),
        )
    }

    fn meta(id: usize, body: &str) -> String {
        format!("<!-- {{\"id\": {id}}} -->\n{body}")
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn only_a_mermaid_fence_is_found_whatever_spells_it() {
        let cases = [
            ("```mermaid", true),
            ("``` mermaid", true),
            ("```MERMaid", true),
            ("~~~mermaid", true),
            ("```mermaid   ", true),
            ("```rust", false),
            ("```", false),
            ("```mermaidish", false),
        ];
        for (opener, expected) in cases {
            let block = lines(&format!("{opener}\nflowchart LR\n A-->B\n```"));
            let found = fences(&block);
            assert_eq!(
                expected,
                !found.is_empty(),
                "opener {opener:?} should {}be a diagram",
                if expected { "" } else { "not " }
            );
        }
    }

    #[test]
    fn a_fence_yields_its_interior_verbatim_without_the_delimiters() {
        let block = lines("prose\n```mermaid\nflowchart LR\n  A[hi] --> B\n```\nmore");
        let found = fences(&block);
        assert_eq!(1, found.len());
        assert_eq!("flowchart LR\n  A[hi] --> B", found[0].source);
        assert_eq!(1, found[0].opener, "the opener line index");
    }

    /// A non-mermaid fence must be consumed, not skipped: otherwise its
    /// interior can be read as a later mermaid fence's content.
    #[test]
    fn a_mermaid_line_inside_another_fence_is_not_a_diagram() {
        let block = lines("```text\n```mermaid\nflowchart LR\n A-->B\n```\n```");
        assert_eq!(Vec::<Fence>::new(), fences(&block));
    }

    #[test]
    fn several_fences_come_back_in_document_order() {
        let block =
            lines("```mermaid\nfirst\n```\n```rust\nlet x = 1;\n```\n```mermaid\nsecond\n```");
        let found = fences(&block);
        assert_eq!(
            vec!["first".to_string(), "second".to_string()],
            found.iter().map(|f| f.source.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_unclosed_fence_runs_to_the_end_of_the_block() {
        let block = lines("```mermaid\nflowchart LR\n A-->B");
        let found = fences(&block);
        assert_eq!(1, found.len());
        assert_eq!("flowchart LR\n A-->B", found[0].source);
    }

    /// The preimage is the interior alone: the same diagram written with a
    /// different fence character or info-string casing is the same picture and
    /// must reuse the same frozen asset.
    #[test]
    fn the_fingerprint_covers_the_interior_and_nothing_else() {
        let backtick = fences(&lines("```mermaid\nflowchart LR\n A-->B\n```"));
        let tilde = fences(&lines("~~~MERMAID\nflowchart LR\n A-->B\n~~~"));
        assert_eq!(
            fingerprint(&backtick[0].source),
            fingerprint(&tilde[0].source),
            "the fence syntax is not part of the picture"
        );
        assert_ne!(
            fingerprint("flowchart LR\n A-->B"),
            fingerprint("flowchart LR\n A-->C"),
            "an edited diagram must miss its old asset"
        );
        assert!(fingerprint("x").starts_with("xxh64-"));
    }

    fn sources(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("flowchart LR\n A{i}-->B{i}"))
            .collect()
    }

    #[test]
    fn every_input_gets_one_outcome_in_input_order() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let stdout = [meta(1, "<svg>one</svg>"), meta(2, "<svg>two</svg>")].join("\0");
        let cli = fake_sekien(dir.path(), &stdout, "");
        let out =
            render_batch(cli.to_str().unwrap(), &sources(2), Duration::from_secs(10)).unwrap();
        assert_eq!(2, out.len());
        assert_eq!(Ok("<svg>one</svg>".to_string()), out[0]);
        assert_eq!(Ok("<svg>two</svg>".to_string()), out[1]);
    }

    /// The law this module exists for: a failed diagram emits nothing on
    /// stdout, so pairing inputs to SVGs positionally misattributes every
    /// later result. Input 2 fails; inputs 1 and 3 must keep their own SVGs.
    #[test]
    fn a_failed_diagram_shifts_nothing_because_ids_correlate_both_streams() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let stdout = [meta(1, "<svg>first</svg>"), meta(3, "<svg>third</svg>")].join("\0");
        let stderr = meta(2, "Parse error on line 2");
        let cli = fake_sekien(dir.path(), &stdout, &stderr);
        let out =
            render_batch(cli.to_str().unwrap(), &sources(3), Duration::from_secs(10)).unwrap();
        assert_eq!(
            vec![
                Ok("<svg>first</svg>".to_string()),
                Err("Parse error on line 2".to_string()),
                Ok("<svg>third</svg>".to_string()),
            ],
            out,
            "a positional pairing would hand input 2 the third SVG"
        );
    }

    #[test]
    fn a_zero_exit_with_failures_is_still_read_per_diagram() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // Every diagram failed, yet the process exits 0 — the measured behavior.
        let stderr = [
            meta(1, "No diagram type detected"),
            meta(2, "Lexical error"),
        ]
        .join("");
        let cli = fake_sekien(dir.path(), "", &stderr);
        let out =
            render_batch(cli.to_str().unwrap(), &sources(2), Duration::from_secs(10)).unwrap();
        assert!(out.iter().all(|outcome| outcome.is_err()), "{out:?}");
        assert_eq!(Err("No diagram type detected".to_string()), out[0]);
    }

    #[test]
    fn a_silent_renderer_leaves_every_input_accounted_for() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_sekien(dir.path(), "", "");
        let out =
            render_batch(cli.to_str().unwrap(), &sources(2), Duration::from_secs(10)).unwrap();
        assert_eq!(2, out.len());
        assert!(out.iter().all(|outcome| outcome.is_err()));
    }

    #[test]
    fn an_empty_batch_never_spawns_the_renderer() {
        // A missing binary would fail the spawn, so reaching Ok proves no spawn.
        let out = render_batch(
            "definitely-not-a-real-binary-xyz",
            &[],
            Duration::from_secs(1),
        );
        assert_eq!(0, out.unwrap().len());
    }

    #[test]
    fn a_missing_renderer_names_the_command() {
        let out = render_batch(
            "definitely-not-a-real-binary-xyz",
            &sources(1),
            Duration::from_secs(1),
        );
        let message = out.unwrap_err().to_string();
        assert!(
            message.contains("definitely-not-a-real-binary-xyz"),
            "{message}"
        );
        assert!(message.contains("is it installed?"), "{message}");
    }

    #[test]
    fn a_hanging_renderer_is_killed_at_the_timeout() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "cat >/dev/null; sleep 60");
        let started = std::time::Instant::now();
        let out = render_batch(
            cli.to_str().unwrap(),
            &sources(1),
            Duration::from_millis(300),
        );
        assert!(out.is_err(), "a hang must not be reported as success");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the timeout did not kill the child"
        );
    }
}
