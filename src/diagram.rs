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
use resvg::usvg;
use twox_hash::XxHash64;

/// The CLI alix shells out to. Named here rather than configured: a second
/// renderer would produce different pictures for the same deck.
pub const COMMAND: &str = "sekien";

/// Checked before the pixmap is allocated, so a huge graph fails as one
/// authoring error instead of an unbounded allocation. 4096 squared.
const PIXEL_CAP: u64 = 4096 * 4096;

/// A long `flowchart LR` grows in one dimension and can pass the area cap
/// while exceeding mobile decoder and GPU texture limits, so each side is
/// bounded independently.
const SIDE_CAP: u32 = 8192;

/// The ground for diagrams whose root style declares a light base fill:
/// mermaid's dark themes assume a dark page and their `background` theme
/// variable is this value.
const DARK_GROUND: (u8, u8, u8) = (0x33, 0x33, 0x33);

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

/// Rasterizes a rendered SVG to PNG bytes at `zoom` times its intrinsic size.
///
/// PNG rather than SVG is load-bearing: a mermaid SVG carries every label as
/// selectable `<text>`, so an overlay on it hides nothing (view-source,
/// Ctrl+F and screen readers all read the answer). Pixels under an overlay
/// are the trust model image occlusion already ships.
///
/// `family` must name a concrete font that both this rasterizer and the
/// renderer measured with; a text-bearing SVG whose family is missing fails
/// loudly here, because usvg would otherwise drop every label and freeze a
/// silently textless diagram.
pub fn rasterize(svg: &str, family: &str, zoom: f32) -> Result<Vec<u8>> {
    raster(svg, family, zoom)?
        .encode_png()
        .context("the PNG cannot be encoded")
}

fn raster(svg: &str, family: &str, zoom: f32) -> Result<resvg::tiny_skia::Pixmap> {
    let mut db = system_fonts().clone();
    db.set_sans_serif_family(family);
    let query = usvg::fontdb::Query {
        families: &[usvg::fontdb::Family::Name(family)],
        ..Default::default()
    };
    if svg.contains("<text") && db.query(&query).is_none() {
        bail!("font family '{family}' is not installed, so the diagram's text cannot be drawn");
    }
    let options = usvg::Options {
        font_family: family.to_string(),
        fontdb: std::sync::Arc::new(db),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).context("the SVG cannot be parsed")?;
    let width = (tree.size().width() * zoom).ceil() as u32;
    let height = (tree.size().height() * zoom).ceil() as u32;
    if width > SIDE_CAP || height > SIDE_CAP {
        bail!(
            "the diagram would rasterize to {width}x{height} pixels, over the {SIDE_CAP} per-side cap"
        );
    }
    if u64::from(width) * u64::from(height) > PIXEL_CAP {
        bail!("the diagram would rasterize to {width}x{height} pixels, over the {PIXEL_CAP} cap");
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .with_context(|| format!("the diagram has no drawable size at zoom {zoom}"))?;
    let (red, green, blue) = ground(svg)?;
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(red, green, blue, 255));
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(zoom, zoom),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// The opaque ground the SVG is composited onto, derived from the diagram's
/// own theme: every mermaid theme declares its base text fill in the root
/// style rule (`#d1{...fill:#ccc}` for dark themes, `fill:#333` for light),
/// and the ground with the higher WCAG contrast against that fill wins
/// (channel-mean thresholds put bright saturated ink on the wrong ground). A
/// marker this derivation cannot read is an error, never a silent white:
/// sekien is not version-pinned, and a CSS serialization change in an
/// upgrade must fail the freeze loudly instead of erasing content.
fn ground(svg: &str) -> Result<(u8, u8, u8)> {
    let style = svg
        .split_once("<style")
        .and_then(|(_, rest)| rest.split_once('>'))
        .and_then(|(_, rest)| rest.split_once("</style").map(|(inner, _)| inner));
    let Some(style) = style else {
        bail!("the diagram carries no theme marker (no style block), so no safe ground exists");
    };
    let Some(fill) = fill_declaration(style) else {
        bail!("the diagram's theme declares no base fill, so no safe ground exists");
    };
    let Some(fill) = hex_color(fill) else {
        bail!("the theme's base fill '{fill}' is not a color the ground derivation recognizes");
    };
    let ink = relative_luminance(fill);
    let dark = relative_luminance(DARK_GROUND);
    Ok(if contrast(ink, dark) > contrast(ink, 1.0) {
        DARK_GROUND
    } else {
        (255, 255, 255)
    })
}

/// WCAG sRGB relative luminance: channel mean misclassifies saturated ink
/// (`#00ff00` has mean 85 yet is among the brightest colors there are), and
/// the gamma linearization matters even for grays.
fn relative_luminance((red, green, blue): (u8, u8, u8)) -> f64 {
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
}

fn contrast(a: f64, b: f64) -> f64 {
    let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// The value of the first `fill:` declaration, tolerating whitespace around
/// the colon. Skips property names that merely start with `fill`
/// (`fill-opacity`), whose next non-space character is not a colon.
fn fill_declaration(style: &str) -> Option<&str> {
    let mut rest = style;
    while let Some(at) = rest.find("fill") {
        let after = &rest[at + "fill".len()..];
        if let Some(value) = after.trim_start().strip_prefix(':') {
            let value = value.trim_start();
            let end = value.find([';', '}']).unwrap_or(value.len());
            return Some(value[..end].trim_end());
        }
        rest = after;
    }
    None
}

fn hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?.as_bytes();
    let expand = |part: &[u8]| u8::from_str_radix(std::str::from_utf8(part).ok()?, 16).ok();
    match hex.len() {
        3 => Some((
            expand(&hex[..1])? * 17,
            expand(&hex[1..2])? * 17,
            expand(&hex[2..3])? * 17,
        )),
        6 => Some((expand(&hex[..2])?, expand(&hex[2..4])?, expand(&hex[4..6])?)),
        _ => None,
    }
}

fn system_fonts() -> &'static usvg::fontdb::Database {
    static FONTS: std::sync::OnceLock<usvg::fontdb::Database> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db
    })
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

    /// PNG magic plus IHDR dimensions, read raw so the tests need no decoder.
    fn png_size(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
        let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        (word(16), word(20))
    }

    const RECT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8" viewBox="0 0 10 8"><style>#d1{fill:#333;}</style><rect width="10" height="8" fill="#ff0000"/></svg>"##;

    #[test]
    fn a_text_free_svg_rasterizes_at_the_requested_scale_with_any_family() {
        let png = rasterize(RECT_SVG, "no-such-family-xyz", 1.0).unwrap();
        assert_eq!((10, 8), png_size(&png), "zoom 1 keeps intrinsic size");
        let png = rasterize(RECT_SVG, "no-such-family-xyz", 2.5).unwrap();
        assert_eq!((25, 20), png_size(&png), "zoom scales both dimensions");
    }

    fn pixel(pixmap: &resvg::tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let p = pixmap.pixel(x, y).unwrap();
        (p.red(), p.green(), p.blue(), p.alpha())
    }

    /// The content must scale WITH the canvas: a mutant that grows the
    /// pixmap but draws at 1x leaves the far corner background-colored.
    #[test]
    fn zoom_scales_the_drawing_not_just_the_canvas() {
        let pixmap = raster(RECT_SVG, "any", 2.5).unwrap();
        assert_eq!(
            (255, 0, 0, 255),
            pixel(&pixmap, 24, 19),
            "the rect must cover the far corner at zoom 2.5"
        );
    }

    /// Real dark-theme sekien output with its `<text>` elements stripped so
    /// no host font is needed; the light `#ccc` strokes remain, which is the
    /// ink a white ground would erase.
    const DARK_THEME_SEKIEN: &str = include_str!("../tests/fixtures/dark-theme-sekien.svg");

    /// The ground must follow the diagram's theme: composited onto white,
    /// a dark theme's light strokes vanish into the ground (Codex's P1).
    #[test]
    fn a_dark_theme_render_sits_on_the_dark_ground_not_white() {
        let pixmap = raster(DARK_THEME_SEKIEN, "any", 1.0).unwrap();
        assert_eq!(
            (0x33, 0x33, 0x33, 255),
            pixel(&pixmap, 0, 0),
            "the uncovered corner must be the dark ground"
        );
        let side = pixmap.width() * pixmap.height();
        let light = (0..side)
            .map(|i| {
                pixmap
                    .pixel(i % pixmap.width(), i / pixmap.width())
                    .unwrap()
            })
            .filter(|p| p.red() > 150 && p.green() > 150 && p.blue() > 150)
            .count();
        assert!(light > 0, "the #ccc strokes must survive as light ink");
    }

    /// The marker is load-bearing, so its recognition must survive the
    /// serializations an unpinned sekien could drift to (Codex's narrowed
    /// P1): whitespace around the colon, case, short and long hex, and a
    /// preceding `fill-opacity` that must not be mistaken for `fill`.
    #[test]
    fn every_root_fill_serialization_variant_finds_the_dark_ground() {
        let variants = [
            "fill:#ccc",
            "fill: #ccc",
            "fill : #CCC",
            "fill:#cccccc",
            "fill-opacity:1;fill:#ccc",
        ];
        for variant in variants {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><style>#d1{{{variant};}}</style></svg>"##
            );
            let pixmap = raster(&svg, "any", 1.0).unwrap();
            assert_eq!(
                (0x33, 0x33, 0x33, 255),
                pixel(&pixmap, 0, 0),
                "variant {variant:?} must be recognized as a dark theme"
            );
        }
    }

    /// Ground choice follows WCAG contrast, not channel mean: bright green
    /// has channel mean 85 ("dark") yet reads at only 1.37:1 on white and
    /// 9.2:1 on the dark ground; mid-gray is the case where skipping the
    /// gamma linearization flips the answer.
    #[test]
    fn ground_choice_follows_contrast_not_channel_mean() {
        let cases = [
            (
                "#00ff00",
                (0x33, 0x33, 0x33, 255),
                "bright saturated green needs the dark ground",
            ),
            (
                "#00008b",
                (255, 255, 255, 255),
                "dark blue needs the white ground",
            ),
            (
                "#808080",
                (255, 255, 255, 255),
                "mid-gray contrasts better with white after linearization",
            ),
        ];
        for (fill, expected, why) in cases {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><style>#d1{{fill:{fill};}}</style></svg>"##
            );
            let pixmap = raster(&svg, "any", 1.0).unwrap();
            assert_eq!(expected, pixel(&pixmap, 0, 0), "{why}");
        }
    }

    /// White is only for an explicitly recognized light theme: a marker the
    /// derivation cannot read errors instead of silently choosing the
    /// ground that erases light ink.
    #[test]
    fn an_unreadable_theme_marker_is_a_loud_error_never_a_white_ground() {
        let cases = [
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"##.to_string(),
                "no style block",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><style>#d1{stroke:#000;}</style></svg>"##.to_string(),
                "no fill declaration",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><style>#d1{fill:rgb(204, 204, 204);}</style></svg>"##.to_string(),
                "unrecognized color spelling",
            ),
        ];
        for (svg, case) in cases {
            let message = rasterize(&svg, "any", 1.0).unwrap_err().to_string();
            assert!(
                message.contains("ground") || message.contains("theme"),
                "{case}: the error must be actionable, got: {message}"
            );
        }
    }

    #[test]
    fn light_ink_under_a_dark_theme_marker_stays_distinguishable() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><style>#d1{fill:#ccc;}</style><path d="M 0 2 H 4" stroke="#ffffff" stroke-width="2"/></svg>"##;
        let pixmap = raster(svg, "any", 1.0).unwrap();
        assert_ne!(
            pixel(&pixmap, 1, 1),
            pixel(&pixmap, 7, 1),
            "a white stroke must not blend into the ground of a dark-themed diagram"
        );
    }

    /// The area cap alone admits this: 100000x1 is only 100k pixels, but a
    /// side that long breaks mobile decoders and GPU texture limits
    /// (Codex's P2).
    #[test]
    fn a_long_thin_diagram_is_refused_per_side_even_under_the_area_cap() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="1"><rect width="100000" height="1" fill="#ff0000"/></svg>"##;
        let message = rasterize(svg, "any", 1.0).unwrap_err().to_string();
        assert!(message.contains("100000x1"), "{message}");
        assert!(message.contains("per-side"), "{message}");
    }

    /// Frozen rasters sit on an opaque ground: a transparent PNG would
    /// show the page theme through the diagram and make dark-theme review
    /// unreadable with the default mermaid colors.
    #[test]
    fn uncovered_canvas_is_opaque_white_not_transparent() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8"><style>#d1{fill:#333;}</style><rect width="2" height="2" fill="#ff0000"/></svg>"##;
        let pixmap = raster(svg, "any", 1.0).unwrap();
        assert_eq!((255, 255, 255, 255), pixel(&pixmap, 9, 7));
        assert_eq!(
            (255, 0, 0, 255),
            pixel(&pixmap, 0, 0),
            "the rect itself still draws"
        );
    }

    /// The trap this guard exists for: usvg drops ALL text when the family
    /// is missing, so without the error a host without the chosen font would
    /// freeze a silently textless diagram.
    #[test]
    fn a_text_bearing_svg_with_a_missing_family_fails_loudly_not_textless() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8"><text x="1" y="6">hi</text></svg>"#;
        let message = rasterize(svg, "no-such-family-xyz", 1.0)
            .unwrap_err()
            .to_string();
        assert!(message.contains("no-such-family-xyz"), "{message}");
        assert!(message.contains("not installed"), "{message}");
    }

    #[test]
    fn a_malformed_svg_is_an_error_not_a_panic() {
        assert!(rasterize("<svg", "any", 1.0).is_err());
        assert!(rasterize("plain text", "any", 1.0).is_err());
    }

    /// The cap must fire BEFORE allocation: without it this test would try
    /// to allocate a multi-gigabyte pixmap instead of returning an error.
    #[test]
    fn an_oversized_render_is_refused_before_allocation() {
        let message = rasterize(RECT_SVG, "any", 10_000.0)
            .unwrap_err()
            .to_string();
        assert!(message.contains("100000x80000"), "{message}");
        assert!(message.contains("cap"), "{message}");
    }

    #[test]
    fn a_zero_sized_render_is_an_error_naming_the_zoom() {
        let message = rasterize(RECT_SVG, "any", 0.0).unwrap_err().to_string();
        assert!(message.contains("zoom 0"), "{message}");
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
