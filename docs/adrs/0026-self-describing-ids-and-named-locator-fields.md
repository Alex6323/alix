# 0026: Self-describing ids and named locator fields

- Status: Proposed
- Recorded: 2026-07-28
- Retrospective: No

## Context

The deck-id rekey (ADR 0017 lineage) made deck-level state follow a deck's
stable id rather than its filename. That work exposed two problems the current
format cannot express:

1. `requires:` still names a prerequisite by filename, the one cross-deck edge
   still tied to a mutable, renameable name.
2. Deck ids and card ids are drawn from one 26-ish-char lowercase-alnum token
   space and are indistinguishable out of context. A token pasted into
   `requires:`, an error message, a filename, or an API field carries no
   evidence of what it identifies.

Pre-1.0 is the only cheap window to change the id grammar; after 1.0 it freezes.
Several independent format breaks (id key rename, id prefixes, locator field
restructuring) are batched into one migration so testers pay one break, one
tool, one documentation pass.

An adversarial pass over the draft design (2026-07-28, three lenses) found that
the naive form of these changes silently re-mints ids, orphans frozen assets,
and launders old locators into fresh-but-wrong fingerprints. This record locks
the grammar so those failure modes are impossible, not merely unlikely.

## Decision

### Id grammar (freeze-forever)

An id is `<kind>-<token>` where `<kind>` is `deck` or `card` and `<token>` is
`[0-9a-z]+`. The load-bearing invariant is **dash-exclusion**: the token charset
contains no `-`, so the first `-` always separates the kind prefix and any
trailing `-` separates a card sub-id suffix. The 26-char Crockford canonical
form remains a `doctor` *warning* only (hand-typed and third-party short tokens
stay valid), exactly as today; length is not part of the frozen grammar.

Card sub-ids: `card-<token>-<n>` for cloze hole `n` (canonical decimal, no
leading zeros) and `card-<token>-r` for the reversed half. A card id carries at
most one suffix; cloze and reversed never combine (already enforced:
`deck.rs` keys reversal on hole absence, `token.rs` debug-asserts it).

The prefix is **required** everywhere an id is read. A value that does not begin
with the correct `deck-`/`card-` prefix is not a valid id; parsing it is a hard
error (see Compatibility), never a silent downgrade to "bare token" or
"unstamped".

### Authoring surface

- Deck frontmatter key is `id` (replacing `alix-id`), value `deck-<token>`.
- Card marker stays `<!-- id: ... -->`, value `card-<token>[-<n>|-r]`.
- The prefixed value is canonical and byte-identical **everywhere** the id
  appears (Model A): frontmatter, card marker, `requires:`, `progress/<id>.json`
  and `augment/<id>.json` filenames, the in-memory and on-disk store keys, the
  `assets/<id>/` directory name, embedded `source:` and `](assets/<id>/...)`
  image links, the deck-transfer bundle layout, error messages, and every
  id-bearing JSON API field. There is no bare-token representation of an id.

The key `id` is generic; the value's prefix carries the type. This is product
neutral (`alix-id` would freeze the product name into the format) and least
redundant.

### `requires:` (dual mode)

`requires:` accepts a filename **or** a deck id. A value is id-mode **iff** it
is `deck-` followed by exactly 26 characters of the canonical Crockford mint
alphabet and nothing else; every other value is filename-mode. Classification
is a pure function of the text, never of which files exist, so an edge means
the same thing on every machine. The classifier is deliberately stricter than
the id grammar itself: minted ids are always canonical, so id-mode authoring
loses nothing (a scan of all real decks found only canonical tokens), while
natural `deck-*.md` filenames (`deck-basics`) fail the 26-canonical test and
remain referenceable by filename. Gating resolves an id-mode value to the deck
whose id matches and checks completion by that id (rename-proof); a
filename-mode value resolves as today via `resolve_dep`.

Collisions are decided, not left implicit:

- Only a file literally named `deck-<canonical-26-token>.md` can shadow an id.
  The id wins; `doctor` reports the shadowing file; a `./`-prefixed path
  (`requires: ./deck-<...>`) is the explicit filename escape (`resolve_dep`
  already treats it as a raw path).
- A `card-<canonical-26-token>` value in `requires:` is a wrong-type error (a
  pasted card id; a card is never a prerequisite), distinct from "dangling".
  A non-canonical value like `card-tricks` stays a filename.
- A filename-mode value beginning `deck-` that resolves to no file gets a
  `doctor` hint that it looks like a truncated or malformed id.
- A deck whose own id was hand-typed short cannot be referenced by id (the
  value classifies as filename-mode); filename reference still works and
  `doctor` notes it.
- `doctor`'s wrong-type and dangling-id checks fire only on values that
  classified as id-mode.

### Locator fields (freeze-forever)

A source citation is `<!-- at: <src>:<lines> fingerprint: xxh64-<hex> asset: sha256-<hex>.<ext> -->`.

- The value is tokenized by single-space splits into strictly alternating
  known-key / value pairs. Keys are `at`, `fingerprint`, `asset` (lowercase,
  each at most once). Canonical order is `at` then `fingerprint` then `asset`;
  `doctor` rewrites to that order. No field value may contain a space. Any
  leftover or unpairable token, any unknown key, any duplicate key makes the
  whole locator a bad-value error and the citation untrusted: there is no
  partial extraction. (Extensibility means the grammar may grow in a later
  version, not that an old parser skips unknown keys.)
- `at:` is the real source path and line range (`<src>:<lines>`); the
  lines-only (`at: 12-18`) and whole-file (`at: notes.md`) forms remain legal.
  `fingerprint:` is the xxh64 excerpt change-detector. `asset:` is the sha256
  content-addressed frozen object name and is present **only** on a
  frozen/asset-backed citation.
- Field roles are inverted from the old grammar: the old `at:` held the frozen
  asset name and the old ` from ` tail held the real path. In the new grammar
  the real path is always `at:` and the frozen object moves to `asset:`.
- What a frozen asset IS (decided 2026-07-28): the excerpt exactly, nothing
  more. Freezing stores only what a citation requires; uncited lines never
  enter the asset. The frozen excerpt is display evidence and the drift
  baseline (what the card was authored against), NOT a substitute source for
  the AI: the tutor and examiner ground against the live source (a local path
  or a fetchable origin URL) and must tell the user when it is unavailable
  rather than silently degrade. A citation's `asset:` object is ALWAYS
  excerpt-shaped, for every source kind: one uniform reader rule, no
  object-shape heuristic, no whole-file object concept (an excerpt spans the
  whole file only when the citation cites the whole file). And `source:`
  NEVER points into `assets/` (ruled 2026-07-28): assets are fragments by
  construction, and an examiner or tutor grounded on fragments will
  confidently fill gaps, which must be prevented at all costs. Freezing
  therefore does not rewrite `source:`; a frozen deck keeps its real origin
  (path or URL). Offline, an exam on a frozen deck reports the missing live
  source instead of running; review and walks stay offline on frozen
  excerpts (display, not AI grounding).
- Reader semantics: when `asset:` is present, the asset's bytes are read in
  full and fingerprint verification targets them (integrity of the evidence);
  display numbers the excerpt from `at:`'s start line (presentation
  arithmetic; asset-local numbers are never shown). `at:`'s path and lines are
  the real-source provenance. Relocation scanning for an asset-backed citation
  never rewrites the frozen fingerprint.

### One source concept: `origin:` merges into `source:` (ruled 2026-07-28)

There is one key for "where this deck's material lives": `source:`,
multi-valued, each value a URL or a local path. The separate `origin:` key
(deck frontmatter and workspace manifest) is retired. The historical reason
for two keys, freezing rewrote `source:` to the frozen snapshot while
`origin:` kept the real pointer, was abolished by the source-never-assets
ruling, and the code already unioned them for examination.

- The workspace manifest may declare a default `source` that members inherit
  when they declare none (replacing the manifest `origin`).
- Derivations: examination and `has_exam` consider the effective sources
  (deliberate behavior change: a local-path source now confers an exam where
  a local `origin:` did not; verified understanding is the product's thesis).
  URL-valued sources feed reference links; the first local-path source is the
  base root for citation resolution, walks, and live-drift display; workspace
  update reconciles against the local-path sources. Freezing stamps nothing:
  a generated or frozen member already declares (or inherits) its source.
- `origin:` in deck frontmatter or the manifest is a recognized-obsolete key:
  a hard error naming the deck conversion tool, exactly like `alix-id:`.

### Loud break at the parser (no compatibility path)

Production reads only the new grammar. There is no dual reader. Detection must
survive `alix`'s own id auto-minting, which fires precisely on "no deck id" and
"unstamped card":

- `alix-id:` is a recognized-obsolete frontmatter key that returns a parse error
  naming the migration tool. It is not an unknown-key lint (which would leave the
  deck id `None` and let review-open mint a fresh `deck-<new>` beside the stale
  `alix-id`, creating two identities and orphaning `assets/<old>/`).
- A `<!-- id: ... -->` whose value lacks the `card-` prefix is a parse error, not
  "unstamped" (which would let the stamper append a second `card-<new>` marker).
- A `requires:`/frontmatter/marker id with a wrong or empty prefix is a parse
  error.
- An `at:` value containing old-grammar residue (` @ `, ` from `) or any
  unpairable token is a bad-value error, so an old locator never parses as a
  fingerprint-less citation that the next stamp would silently re-fingerprint.
- Mint, splice, and stamp refuse to run while any old-format id shape is present
  in the file, so review-open cannot half-migrate before `doctor` is run.
- `doctor` positively detects a bare-token id, marker, `requires:` value,
  locator, or state-document (filename or internal `deck_id`) and names it as
  un-migrated. `#[serde(deny_unknown_fields)]` is not relied on: id and key
  changes are string values, which serde does not validate.

## Consequences

- Every id is self-describing at rest and in transit; a mis-pasted card id in
  `requires:` is a parse-time type error, not a silent dangling edge.
- `requires:` survives a prerequisite rename when authored by id, while keeping
  the ergonomic filename form.
- One representation of an id removes the strip/re-add class of bugs; the
  `assets/<id>/` directory, embedded links, store keys, and API fields all use
  the same string.
- Cost: a one-time migration that touches every deck file, frozen asset
  directory, in-repo fixture, and documentation surface (see Compatibility). The
  frozen-citation reader is rebuilt to read asset bytes.
- Deliberately unsupported: referencing a file literally named after a
  canonical id (`deck-<26-token>.md`) by bare filename in `requires:` (use
  `./`); requiring a deck by a hand-typed non-canonical id; combining cloze
  and reversed suffixes; unknown locator keys in this format version.

## Alternatives considered

- **Prefix only at the authored surface, bare token internally (Model B).**
  Rejected: a generic `id` key plus a bare stored value is fully untyped, and
  `requires:`/filenames need the id to travel prefixed, forcing a strip/re-add
  dance and two sources of truth for one id.
- **Bare `assets/<token>/` directory (a Model A exception).** Rejected: it
  reintroduces a bare-token representation that every asset path must strip; a
  single missed strip is a wrong-path bug. The prefix is a one-time migration
  cost for a permanent consistency gain, consistent with Model A's rationale.
- **`deck-id` / `card-id` frontmatter keys (typed keys).** Rejected in favor of
  a generic `id` with a typed value: product neutral, least redundant, and the
  card marker key is already `id`.
- **26-char canonical token as the frozen rule.** Rejected: the code accepts
  `[0-9a-z]+` and real decks carry short hand-typed tokens; freezing 26-char
  would hard-fail a mechanical migration of `card1` to `card-card1`. Dash-
  exclusion, not length, is the invariant that makes prefixes and sub-ids parse.
- **Wiping the augment sidecar with progress.** Rejected: augment artifacts
  (distractors, notes, keypoints, topology) are paid backend output. The tool
  rewrites augment keys in place (`token` to `card-token`, `deck_token` to
  `deck-<token>`); only progress is reset (it is cheap dev state).
- **Lenient locator/id parsing that skips unrecognized input.** Rejected: it is
  exactly what launders an old artifact into a silently-wrong new one.

## Compatibility

Pre-1.0, no backwards compatibility. Production reads only the new format. A
disposable tool outside the production repository backs up, converts, verifies,
and deletes old artifacts; `doctor` detects an old artifact and names the tool.

Surfaces the migration must cover (enumerated so none is silent):

- Deck files: frontmatter `alix-id` to `id: deck-<token>`; every `<!-- id: -->`
  to `card-<token>`; every `<!-- at: -->` reshaped with field inversion (frozen:
  object to `asset:`, real path+lines from the old `from` tail to `at:`; live:
  `at:` keeps the path, no `asset:`); for a `from`-less old asset locator, emit
  `at:` with the asset's own coordinates and a `doctor` note for manual review.
- Frozen assets: rename `assets/<token>/` to `assets/deck-<token>/`, re-verify
  each object digest after the move, and rewrite the embedded `source:` and
  `](assets/<token>/...)` links in the deck body. Assets are evidence, not
  progress: renamed, never wiped.
- Augment sidecars: rewrite internal card-id keys and `deck_token` in place;
  preserved, not wiped.
- A deck whose `source:` points into `assets/` (the pre-ruling frozen shape):
  the tool rewrites it to the recorded origin (workspace or deck `origin:`
  metadata); with no recorded origin it lists the deck for manual repair, and
  `doctor` flags a `source:` inside `assets/` as un-converted.
- `origin:` merge: the tool appends a deck's `origin:` value into its
  `source:` list when distinct (creates the list when absent), drops it when
  duplicate, deletes the `origin:` line, and converts a manifest `origin` to
  the manifest default `source`.
- Progress: reset (deleted). Old `progress/<bareid>.json` that survives is
  positively flagged by `doctor` as un-migrated.
- In-repo committed decks, test fixtures, and frozen example traces: rewritten
  by hand in the same change and their doctor-resolves guards re-pinned.
- Mobile: on-device decks and progress are unreachable by the host tool; the
  documented flow is re-push migrated decks and accept a device progress reset;
  the embedded core fails loud on old-format decks like any other client.
- Documentation and prompts: `docs/API.md` (id and `CitationDto.locator` wire
  values), the contract snapshots (`serve/contract.rs`, `tests/contracts/`), the
  book chapters teaching the old syntax, the README inline example, `CHANGELOG`,
  and every AI prompt that embeds the grammar (`workspace_update.rs` at minimum)
  and the project `CLAUDE.md` study-deck locator template.

Affected records: this ADR supersedes or extends the card-identity record, ADR
0015 (frozen source snapshots), ADR 0020 (source-excerpt integrity), and ADR
0021 (deck-owned frozen assets); it builds on ADR 0017 (per-deck state). The
implementation plan pins exact numbers and marks each superseded record.

Persisted format version: the per-deck-document format is unreleased v1; this
evolves v1 in place (no version bump, no released user saw the old shape).

## Security

No new trust boundary. Frozen-source integrity is preserved: `asset:` still
names the object by its exact sha256 bytes and the reader still re-verifies the
digest, so renaming the directory does not weaken the guarantee. The wrong-type
and un-migrated detections reduce the chance of a citation silently resolving to
unintended bytes.

## Verification

- Parser tests: a bare id/marker, an `alix-id:` key, an old ` @ `/` from `
  locator, an unknown/duplicate locator key, and a space-containing path each
  produce the specified hard error; a prefixed id and each sub-id form parse.
- `parse_card_id` strips the prefix before the suffix split; mutation-test the
  cloze hole-cascade and duplicate-id detection with prefixed keys.
- Mint/splice/stamp refuse to run in the presence of any old-format shape.
- `doctor`: bare-token detection across ids, markers, `requires:`, locators, and
  state documents; the wrong-type `card-` in `requires:` case; the prefix-aware
  canonical-token check.
- Frozen-citation reader: excerpt and fingerprint target the asset bytes read
  in full, displayed at `at:`-derived numbering; regression-test that every
  source kind freezes excerpt objects, that uncited lines of a cited file
  never enter an asset, and that freezing leaves the deck's `source:` line
  untouched.
- Contract snapshots regenerate to the prefixed id and the inverted locator wire
  value; `docs/API.md` matches.
- The disposable tool is tested on fixtures (including a frozen deck with assets
  and an augmented deck) before it touches real decks, and it backs up first.

## Reversal

Pre-1.0, a later ADR may replace this grammar with another loud break and a new
disposable tool. Evidence that would justify it: the prefix proves ergonomically
costly in authoring, or a third id kind makes the two-prefix scheme insufficient.
Post-1.0 the grammar is frozen and any change follows the then-current migration
policy.
