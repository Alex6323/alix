# 0011: Shared RaTeX math rendering

- Status: Accepted
- Evidence: pub struct MathView in src/math.rs
- Recorded: 2026-07-24
- Retrospective: Yes

## Decision history

Commit `5c75476` added the RaTeX SVG renderer on 2026-07-23. Commit `ff91279`
projected LaTeX spans from Markdown, and commits `aa1fb88` and `63ae0bf`
rendered the same artifact in web and mobile. Commit `37806bf` added authored
formula validation before generated deck placement.

The shipped implementation uses exact RaTeX dependency versions. An
investigation into a slim fork did not change this accepted design.

## Context

Markdown decks need inline and display mathematics in web and embedded mobile.
Using a JavaScript renderer in the browser and a different Flutter renderer on
mobile would produce divergent syntax, spacing, fonts, errors, and safety
behavior.

Rendered output must not become canonical content. A renderer upgrade should
not reset card identity, invalidate grading text, or rewrite learners' decks.

## Decision

Authored LaTeX between supported Markdown delimiters is canonical content.
The Rust core parses and renders it through RaTeX into a self-contained SVG
display artifact. Web and mobile consume that shared artifact rather than
interpreting formulas with client-specific engines.

Content and display remain separate:

- content projection strips delimiters but keeps authored formula source for
  grading and fingerprints;
- display projection carries the source plus a rendered `MathView`;
- rendering errors are display diagnostics rather than replacement content;
- SVG is never persisted into decks or progress.

RaTeX parser, layout, and SVG crates are exactly pinned. Glyphs and fonts are
embedded to keep rendering self-contained. Generated SVG passes a restrictive
safety validator before reaching a client.

The renderer and embedded fonts are part of the mobile native artifact and
therefore have application-size consequences that must be measured on release
builds.

## Consequences

- Formula behavior and diagnostics match across supported clients.
- Mobile can render math offline without a WebView or server image.
- The Rust core carries RaTeX and embedded-font dependencies.
- Renderer upgrades require coordinated visual, safety, and size validation.
- Display caches can avoid repeated rendering without becoming persisted
  truth.
- Raw LaTeX remains readable and portable outside Alix.

## Alternatives considered

### KaTeX in web and a Flutter math package on mobile

Two engines would create syntax and layout drift and duplicate validation.

### Server-rendered image URLs

This would make mobile math depend on a reachable server, complicate caching
and authorization, and break offline review.

### Persist SVG beside cards or in progress

Persisted output would become stale when rendering changes, bloat authored or
personal state, and blur the canonical-content boundary.

### Render LaTeX independently in each client

Client ownership would violate the shared projection contract in ADR 0007.

### Maintain a private slim RaTeX fork immediately

A fork could reduce size, but it adds permanent dependency ownership. Release
artifact measurements did not justify that cost as part of the initial
decision.

## Compatibility

Authored LaTeX and delimiter rules are deck-format compatibility surfaces.
SVG structure is a client display contract but not persisted learner data.
Changing the renderer must preserve raw and content projections even when
pixels change.

## Security

Formula input is untrusted. The core rejects output containing script,
foreign-object, image, text, event-handler, external-reference, or URL-bearing
constructs. Clients must render the validated artifact without re-enabling
external resource loading.

## Verification

- `src/math.rs` owns rendering, caching, safety validation, and diagnostics.
- `src/inline.rs` tests raw, content, and display separation and proves content
  projection does not invoke RaTeX.
- `src/review.rs` projects math into shared `CardView` runs.
- Web and mobile rendering tests consume `MathView` rather than parse LaTeX.
- `Cargo.toml` exactly pins RaTeX and enables standalone embedded fonts.
- Release builds measure real mobile artifact size.

## Reversal

Replace RaTeX when syntax coverage, rendering quality, safety, maintenance, or
measured mobile size fails a product requirement. A replacement must keep
authored LaTeX canonical, produce one shared safe offline artifact, preserve
content fingerprints and grading text, and pass cross-client visual fixtures.
