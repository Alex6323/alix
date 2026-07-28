# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The empty "Nothing due." session screen now shows one quiet line saying when
  the next card is due (for example "Next due in 4 min." during an acquire
  cooldown), so an empty sitting explains itself instead of only reading
  "Nothing due." Adult web and mobile; the finished session payload
  (`StateDto` / `ReviewState`) gains an additive `next_due_ms` instant.
- `alix deck copy <deck> <workspace>` and
  `alix deck move <deck> <workspace>` transfer one initialized workspace member
  with its owned frozen assets and augmentation. Both reuse the same public
  bundle boundary as wormhole sharing; copy excludes progress, while confirmed
  move carries progress between distinct user roots and removes the source only
  after the destination is complete.
- `alix workspace update <dir>` reconciles frozen source-backed members with
  their live local sources. It stages an exact sibling workspace for
  review, then `--apply` publishes those same bytes without another model call
  or `--discard` removes them. Retained IDs require unchanged learning content;
  changed and obsolete cards retire with their IDs, while replacements receive
  fresh IDs during staging.
- Plain fact cards can carry multiple `<!-- at: ... -->` citations. The adult
  source view resolves every locator and stacks the editor-style excerpts in
  authored order inside one scrollable answer region.
- `alix doctor --repair-source-locators` explicitly fingerprints reviewed
  source citations and rebases a uniquely relocated exact excerpt while
  preserving deck and card IDs. Plain doctor remains read-only.
- A private vulnerability-reporting policy and a tracked threat model covering
  local files, LAN pairing, AI providers, sharing, persistence, mobile, and
  release boundaries.
- `alix profile`: define and launch a named alix instance per person (its own
  decks, port, and adult/kids frontend), reachable on your LAN with a stable
  token so phones can bookmark it. `alix profile add/list/remove`, `alix
  profile <name>` to launch, `alix profile default` to pick what bare `alix`
  launches, and `alix --launch-all` to boot every profile at once.
- Multiple-choice cards you author directly: write the answer as a GitHub task
  list (`- [x]` correct, `- [ ]` distractors). It renders as a checklist in any
  Markdown previewer and drives the Recognize quiz from your own options, with
  no AI distractor pass needed. Task lists in notes and card fronts render as
  checkboxes too.
- card text now renders inline Markdown: `**bold**`, `*italic*`/`_italic_`, and
  `` `code` ``.
- LaTeX math in cards: `$...$` renders inline and a whole-line `$$...$$`
  renders as centered display math in adult web, kids web, and mobile. Formula
  clozes support `\blank{...}`, and RaTeX chemistry remains available through
  `\ce{...}`. The Rust core produces one self-contained SVG shared by every
  graphical client while decks, grading, fingerprints, and progress retain only
  the authored source.
- Committed manual-QA examples for graphical math rendering and a
  self-contained workspace with a frozen Rust ownership trace.
- the picker's focus drawer now shows a deck's preamble (the prose written
  under its `#` title), which was parsed but never surfaced before
- `alix doctor` flags a dangling `requires:` (one naming a deck that does not
  exist), so a renamed or deleted prerequisite is reported instead of silently
  dropping the gating edge.
- `alix doctor` warns when a card's `<!-- id: -->` marker is not the card's
  closing line (the position stamping mints at), so a hand-placed marker drifts
  back to the canonical shape instead of scattering through the deck.

### Changed

- **Breaking (pre-1.0):** deck ids are now self-describing and prefixed. A deck
  declares `id: "deck-<token>"` (the `id:` frontmatter key replaces `alix-id:`),
  and every card marker is `<!-- id: card-<token> -->` (`card-<token>-N` for a
  cloze hole, `card-<token>-r` for a reversed twin). The same prefixed id names
  the deck's state documents (`progress/deck-<token>.json`,
  `augment/deck-<token>.json`) and asset directory (`assets/deck-<token>/`), and
  `requires:` accepts a `deck-<token>` id so a prerequisite survives a rename.
  Source citations use named fields, `<!-- at: <src>:<lines> fingerprint:
  xxh64-<hex> asset: sha256-<hex>.<ext> -->`, replacing the old ` @ xxh64:`
  form: `at:` is always the real source path, and `asset:` (present only on a
  frozen citation) holds the cited excerpt exactly. Freezing no longer rewrites
  `source:` and stamps nothing; a frozen deck keeps its real source. The
  `origin:` key in deck frontmatter and the workspace manifest merged into the
  multi-valued `source:`, and the card-level `origin:` directive was retired
  with no replacement; a workspace manifest may declare a
  top-level `source` (the material the workspace is about), which the tutor and
  examiner receive as layered supporting context under the deck's own sources
  and which `has_exam` counts. A `source:` value is one expression (a URL, a
  file, or a directory); the `" + "`-joined form is retired and now fails to
  parse with a one-source-per-list-entry hint. Old-format decks fail to load
  loudly and the deck conversion tool rewrites them; there is no runtime reader
  for the old shape. `alix doctor` flags an un-converted bare-token state
  document or a `source:` that points into `assets/`.
- Progress and augmentation documents now reject an unrecognized field instead
  of ignoring it. Before, renaming or removing a field in a future build would
  let old documents load with that field silently dropped, losing that data on
  the next save; now such a document fails to load loudly and is left on disk
  for external conversion, with no format version bump.
- Review progress now persists as it happens: every grade, acquire, exam
  flag, badge, and card mutation writes the deck's progress document before
  the response returns, so closing the browser or killing the server
  mid-session no longer loses the sitting. The former session-batched flush
  (one write per transition) is gone; transition flushes remain as backstops.
- The server drains its workers, flushes any unsaved state, and exits cleanly
  on Ctrl-C or SIGTERM instead of dying mid-request.
- Dependency changes now pass a reviewed duplicate-family gate through
  `make deps-check`, preventing an avoidable second compiled version from
  entering the graph unnoticed.
- **Breaking (pre-1.0):** initializing a source-backed workspace member now
  freezes each cited source excerpt and every local card image before success.
  A citation's frozen object holds exactly its excerpt under
  `assets/deck-<token>/sha256-<digest>.<ext>` (uncited lines never enter an
  asset), and a citation-less member initializes with no freezing at all;
  `source:` stays the deck's real material, and runtime source consumers fail
  closed on live, cross-deck, missing, or corrupted assets. A URL-valued
  `source:` grounds the exam and tutor but holds no freezable bytes; citations
  must land in a local source.
- Single-deck sharing now carries and validates the complete deck-owned asset
  directory plus matching augmentation while continuing to exclude progress.
  Generated workspaces freeze in hidden staging before publication, and merges
  add immutable objects without replacing unrelated decks' assets.
- **Breaking (pre-1.0):** typed `WorkspaceFiles` and `UserFiles` owners now
  separate shareable deck material from private learning state. Assets and
  per-deck augmentation stay with the workspace content; `--store` and a
  workspace `store` setting relocate only progress and recent history. Sharing
  carries matching assets and augmentation while excluding progress and local
  configuration.
- Tutor and exam grounding now combines frozen evidence with the live
  `source:` values, deck and workspace layered apart (deck sources are the
  primary grounding, the workspace source supporting context). URL sources are
  fetched only when the backend and tool grant permit it; local sources still
  require explicit source access. The tutor continues from frozen evidence with
  a visible warning when the full current source is unavailable.
  `alix generate --source-url <URL>` records a portable public URL as an
  additional deck source or the generated workspace's `source`.
- **Breaking (web/mobile APIs):** `CardDto` and the shared mobile `CardView`
  replace the single `at` citation with ordered `citations`; web citation
  entries carry their resolved excerpt or per-locator error. Both views also
  gain `back_units`, the core projection used to render ordinary answer prose
  independently of authored physical line wrapping.
- **Breaking (pre-1.0):** every complete `<!-- at: ... -->` citation now carries
  a named `fingerprint: xxh64-...` field. Review, trace, tutor grounding, and
  grading fail closed when the addressed text does not match, showing a warning
  instead of unrelated source. Generated and frozen citations are stamped at
  creation; hand-authored citations remain incomplete until explicitly
  reviewed and stamped.
- **Breaking (pre-1.0):** workspace member decks now live only under direct
  `decks/*.md` children. Manifests, assets, and augmentation remain at the
  workspace root; private progress is colocated there by default but may use a
  separate user-files root. Relative member sources and images anchor
  at the workspace. Workspace creation and every generation/import/receive
  surface write the new shape, while `alix doctor` reports initialized root
  decks that are not discovered. Existing workspaces must move their deck files
  into `decks/` without changing its `id:` or card ids.
- **Breaking (pre-1.0):** progress and AI augmentation now live in independently
  versioned documents per initialized deck:
  `progress/deck-<token>.json` and `augment/deck-<token>.json`. Renaming a deck
  keeps
  its state, reviewing different decks on different synced devices no longer
  rewrites one shared file, and stale local revisions fail instead of silently
  replacing a newer document. This is a clean pre-1.0 format break: production
  reads only version-1 per-deck documents and contains no runtime converter for
  preceding layouts. Same-deck concurrent offline review is still unsupported;
  use `alix doctor <folder>` to surface incompatible or orphaned documents and
  synchronization conflicts.
  Sharing carries matching per-deck augmentation and recursively excludes all
  progress, temporary, backup, and conflict material.
- **Breaking (pre-1.0):** a hand-authored Markdown file must be explicitly
  initialized with `alix deck init <file>` before it appears in the picker or
  can be reviewed or augmented. `alix deck init` stamps a fresh
  `id: "deck-<token>"`; a frontmatter `id:` whose value is not a `deck-<token>`
  id is rejected rather than adopted. Opening an initialized deck still assigns
  ids to newly added cards, while ordinary `.md` prose with `##` headings is
  ignored and never modified.
- Production CI and release workflows now select exact Rust, Flutter, Java,
  Node, Android NDK, FRB codegen, mdBook, and coverage-tool versions. Every
  directly referenced GitHub Action uses an immutable commit SHA, a blocking
  check prevents movable pins from returning, `make install` explicitly uses
  the repository's exact Rust pin and lockfile, and scheduled drift jobs remain
  the explicit non-publishing path for testing current upstream toolchains.
- **Breaking (pre-1.0):** grounded tutor filesystem access now requires an
  explicitly declared deck or workspace `source`; its root is the deck's first
  local-path source (workspace source as fallback). A citation still supplies
  the card's evidence, but alix no longer guesses a wider project root from
  `Cargo.toml`, `.git`, or other markers. A public URL source can supply current
  context when `WebFetch` is available.
- Trace source excerpts now highlight exact, case-sensitive terms that the
  checkpoint author marked as inline code in its key points.
- **Additive (web API):** card display projection now comes from the shared
  Rust core. `InlineRun` gains optional `math`, `CardDto` gains `context_runs`,
  and `StateDto` gains `choice_runs` and `keypoint_runs`; every run list stays
  in index lockstep with its existing text field. `CardDto` continues to expose
  text fallback for clients that ignore the new fields.
- **Additive (web/mobile APIs):** trace walk state now carries inline-run
  projections for its description, checkpoint prompt, givens, key points, and
  note alongside the existing raw strings.
- Mobile review now consumes the core's shared inline runs for bold, italic,
  code, and LaTeX math instead of rendering raw card strings separately.
- Android release builds now size-optimize the embedded Rust core with fat LTO,
  one codegen unit, and stripped symbols. `make aab` produces the Android App
  Bundle for Google Play while `make apk` remains the GitHub-release and
  phone-smoke artifact.
- Inline code (`` `like this` ``) now renders with a distinct, theme-aware
  color for readability.
- **Breaking (pre-1.0):** inline `*`/`_`/`**` in existing card text now renders
  as emphasis; a deck that used them literally (e.g. `2*3*4`) will render/grade
  with the markers stripped. Escape with a backslash (`\*`) or wrap in inline
  code (`` `2*3*4` ``) to keep them literal. Run `alix doctor <deck>` to find
  affected cards.
- **Breaking (web API):** `CardDto.front` and `CardDto.back` now contain
  inline-marker-stripped content, while the new `front_runs` and `back_runs`
  fields carry display formatting. Sentence-shaped `NoteUnit` values also gain
  `runs`.
- the picker's focus drawer now opens for every deck, not only decks with a
  topology augmentation, and shows a per-card retrievability heatmap: a single
  whole-deck bar for a plain deck, split into named regions when the deck has a
  topology. Cards you have never reviewed render as a neutral cell rather than
  red, so a fresh deck reads as unlearned instead of failing
- **Breaking (web API):** the drawer no longer shows a raw due count. `POST
  /api/deck-topology` is renamed `POST /api/deck-drawer`; its response
  `DeckTopologyDto` becomes `DeckDrawerDto` (gains `preamble` and a flat
  `heatmap`, drops `deck_due`), and `RegionInfoDto` drops its `due` field

### Fixed

- The multiple-choice quiz no longer highlights a hovered option like the
  keyboard-focused one; keyboard focus is the single highlight and the mouse
  gives no visual state.
