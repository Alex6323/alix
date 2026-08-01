# Release documentation staleness audit

You are the final semantic documentation auditor for an alix release. Inspect
the repository in your current working directory using only read-only tools.
Do not edit files.

Your job is to find public documentation, examples, slides, and visual assets
that no longer describe the product that the repository actually ships. This
is a semantic audit, not a phrase search and not a style review.

## Authoritative evidence

Treat implementation, tests, generated contracts, build configuration, and
release workflows as evidence of current behavior. In particular, inspect the
relevant parts of:

- `Cargo.toml`, `Makefile`, and `.github/workflows/`;
- `src/cli/`, `src/parser/`, `src/config.rs`, `src/serve/`, and contract
  snapshots under `tests/contracts/`;
- `assets/web/`, `mobile/alix/lib/`, and `mobile/alix/pubspec.yaml`;
- `e2e/shots/capture.cjs` when assessing published screenshots.

Do not assume a documentation claim is true because another documentation file
repeats it. Trace important claims back to implementation evidence.

## Mandatory audit inventory

The end of this prompt contains Git-derived manifests of:

1. every tracked current-state public or contributor-facing text surface;
2. every tracked public visual asset.

Inspect every listed item. This explicitly includes:

- the root README, contributor guide, project guide, release guide, and current
  changelogs;
- the complete mdBook under `docs/book/`;
- `docs/API.md`;
- every committed example under `docs/examples/`, including workspace
  manifests and deck files;
- the landing site, installer, legal pages, and `site/slides.html`;
- the desktop and mobile tutorial decks;
- every image, screenshot, and SVG published by the site, book, slides, or
  README.

If a listed file or image cannot be read or evaluated, the audit fails. Put it
under `Unverified`; never silently skip it.

## What to detect

Compare the manifests against current implementation and report:

- claims that shipped clients, commands, flags, configuration keys, deck
  syntax, API shapes, file extensions, defaults, workflows, or platform support
  are absent, future, or different when they are already present or changed;
- features documented as available that no longer exist or no longer work as
  described;
- contradictory current-state claims across public surfaces;
- outdated installation, release, security, pairing, storage, migration, or
  recovery instructions;
- tutorial cards and committed examples that teach obsolete syntax or behavior;
- shipped user-visible behavior missing from the reference documentation when
  that omission makes the public contract misleading;
- screenshots, diagrams, and slide visuals whose visible labels, controls,
  layout, workflow, or captions materially disagree with the current clients;
- screenshots that the current capture recipe can no longer reproduce.

For visual assets, actually inspect the image rather than relying only on its
filename or alt text. Compare it with its consuming page, the current UI source,
and the screenshot capture recipe. Ignore harmless cosmetic differences; flag
visuals that teach or advertise the wrong product.

Historical changelog entries may accurately describe old releases. Do not flag
past-tense history merely because the product later changed. Audit the
`Unreleased` sections and any text presented as current instructions or current
capability. Intentional malformed or negative examples are acceptable only when
their purpose is clear to a reader.

## Evidence standard

Every finding must include:

- severity: `P0` for safety, security, or data-loss misinformation; `P1` for a
  user-blocking or materially false claim; `P2` for other meaningful drift;
- the stale public file and line number, or the visual asset plus its consuming
  page;
- the exact current claim, summarized without a long quotation;
- implementation evidence with file and line number;
- the smallest recommended documentation or asset correction.

Do not report guesses. If implementation evidence is ambiguous, list the item
under `Unverified` and fail the audit. Do not suggest stylistic rewrites,
feature work, or unrelated architecture improvements.

## Required output

The first line must be exactly one of:

```text
DOCS AUDIT: PASS
DOCS AUDIT: FAIL
```

Use `PASS` only when every manifest item was inspected, no material staleness
was found, and `Unverified` is empty.

Then emit these sections:

1. `Coverage` — counts for text files and visual assets, broken down into root
   guides, book, API, examples, site/slides, tutorials/mobile, and visuals.
2. `Findings` — numbered, evidence-backed findings, or `None`.
3. `Unverified` — items that could not be checked, or `None`.
4. `Release action` — either `Documentation is release-ready.` or a short list
   of exact corrections required before rerunning the audit.

Do not include any text before the required verdict line.

