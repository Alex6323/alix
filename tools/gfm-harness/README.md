# gfm-harness

A measuring tool: it runs the official CommonMark and GFM spec example
corpora through the alix parser and records one compact outcome line per
example. Its job is to make parser changes visible, not to score
conformance. alix is deliberately not a general Markdown renderer, so
many spec examples are supposed to error or diverge here; the baseline
records what alix does today, and `make gfm-measure` regenerates it so
a parser change shows its corpus-wide effect as a reviewable git diff.

## Usage

    make gfm-measure

runs both corpora and prints which baseline files drifted. Review the
diff; commit intentional changes together with the parser change that
caused them.

Direct invocation:

    cargo run --manifest-path tools/gfm-harness/Cargo.toml -- \
        [--digest] CORPUS INPUT OUTPUT

Without `--digest` it writes full per-example measurement JSON (cards,
runs, images, reasons) for ad-hoc inspection; put those files in `out/`
(gitignored).

## Baseline format

`baseline/*.jsonl`, one JSON object per spec example, keys elided when
empty:

- `e`: spec example number
- `g`: open-decision groups the example touches; the numbering is assigned by
  `decision_groups` in `src/main.rs`, which is where to read what each number
  covers
- `err`: alix parse error, when the example fails to load
- `lints`: lint kinds raised
- `cards`: cards parsed (absent when `err`)
- `back`: primary answer vs the raw markdown lines: `exact`, `ws`
  (whitespace-equivalent), or `div` (diverges)

## Corpora provenance

- `corpora/commonmark-0.31.2.json`: the official `spec.json` of the
  CommonMark spec, version 0.31.2 (spec text is CC-BY-SA 4.0).
- `corpora/gfm-499789b49373bfa045d0e7547e5ee63444c77bca-spec.txt`:
  `spec.txt` from `github/cmark-gfm` at the commit named in the file
  (CC-BY-SA 4.0).

The corpora are committed so measurements are reproducible offline; they
are not part of the published alix package (the root `Cargo.toml`
`include` allowlist keeps `tools/` out).

## Crate shape

Standalone on purpose: not a workspace member, so `make check`, CI, and
the published crate never build it. Its `Cargo.lock` is gitignored; the
path dependency on the root crate keeps it honest against the current
parser.
