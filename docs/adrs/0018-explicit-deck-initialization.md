# 0018: Explicit deck initialization

- Status: Accepted
- Evidence: maintenance_refuses_an_uninitialized_file_without_writing in src/stamp.rs
- Recorded: 2026-07-25
- Retrospective: No
- Refines: [ADR 0002](0002-markdown-native-decks.md)
- Refined by: [ADR 0020](0020-source-excerpt-integrity.md)
- Details evolved by
  [ADR 0026](0026-self-describing-ids-and-named-locator-fields.md): the deck
  marker is now `id: deck-<token>`; the explicit initialization and local
  write-authority boundary are unchanged. References below to `alix-id`
  describe the originally accepted spelling.

## Context

ADR 0002 makes Markdown the canonical deck format and defines a level-two
heading as a card boundary. That structural grammar is intentionally useful in
ordinary editors, but ordinary Markdown documents also commonly contain
level-two headings.

Alix previously used structural resemblance for discovery. Review and
augmentation then stamped a selected file before loading it, assigning a deck
ID and card IDs. Merely browsing or opening a prose document that happened to
match the grammar could therefore authorize a persistent rewrite.

Deck identity already exists as a stable, machine-maintained token. The missing
decision is whether that identity is an output of any automatic open or an
explicit boundary between ordinary Markdown and Alix-managed content.

## Decision

A valid deck ID under the namespaced `alix-id` key in opening YAML frontmatter
is the durable marker that a Markdown file is initialized as an Alix deck and
authorizes machine-maintained stamping. A generic `id` key has no Alix meaning:
it is common document metadata and must not grant write authority even when its
value happens to satisfy the token grammar.

Hand-authored files cross that boundary through:

```text
alix deck init <file>
```

That explicit command may mint the initial deck ID and missing card IDs.
Workflows that explicitly create a deck, including generation, import, receive,
library installation, and tutorial seeding, may initialize their own output.

Automatic review and augmentation maintenance may assign missing card IDs only
after a valid deck ID exists. It must refuse an uninitialized file without
writing any bytes. Workspace discovery and the picker include initialized decks
only.

Doctor remains read-only. It reports deck-like Markdown that discovery ignores
and explains how to initialize an intended deck.

## Consequences

- Ordinary Markdown can coexist in a deck directory without being listed or
  rewritten because it contains level-two headings.
- Creating a hand-authored deck requires one explicit initialization command.
- Adding cards to an initialized deck keeps the existing convenience: the next
  review or augmentation open assigns missing card IDs.
- Malformed initialized decks remain attributable to Alix and can be surfaced
  for repair.
- The namespaced deck ID carries both identity and mutation-authority meaning,
  avoiding both a second marker that could disagree with it and collisions with
  ordinary YAML metadata.

## Alternatives considered

### Recognize a `.deck.md` suffix

A filename convention is easy to see but breaks existing paths and remains
mutable. Stable identity is already persisted inside the file.

### Add an `alix: deck` frontmatter flag

A second marker can drift from the required deck identity. It adds a concept
without strengthening the boundary.

### Prefix every Alix frontmatter key

Only the identity marker grants permission to rewrite the file, so it needs a
collision-resistant namespace. Other directives are interpreted only after
that opt-in and do not authorize mutation. Prefixing the whole vocabulary would
make decks noisier, break existing files, and discard useful interoperability
for ordinary metadata such as `author`, `license`, `language`, and `tags`
without strengthening the write boundary.

### Enumerate members in `alix.toml`

Manifest membership does not protect loose decks and creates a manually
maintained filename index that becomes stale on rename.

### Confirm initialization in every client

Per-client prompts duplicate a persistence decision and can diverge between
CLI, web, and mobile. A shared core refusal plus one explicit command is the
smaller boundary.

## Compatibility

Existing deck and card token values remain valid, but pre-1.0 decks using the
generic frontmatter key must rename `id:` to `alix-id:` without changing its
value. Alix does not retain `id` as an alias because that would preserve the
ambiguous write-authority boundary. Explicit initialization refuses a generic
`id` rather than minting a replacement and risking lost progress.

Generated, imported, received, library, and tutorial decks initialize with
`alix-id` at creation.

Uninitialized hand-authored decks no longer appear in discovery and no longer
start review automatically. The user must run `alix deck init <file>` once.
This is an intentional pre-1.0 behavior break to prevent silent mutation.

## Security

The `alix-id` deck ID is a local write-authority marker, not authentication and
not proof that content is trusted. Markdown, directives, and referenced sources
remain untrusted input.

Every automatic stamping entry point must fail closed before writing when the
marker is absent. Read-only structural detection may recommend initialization,
but cannot grant write authority.

## Verification

- Parser tests distinguish a valid opening-frontmatter `alix-id` from both
  arbitrary `id` metadata and `id:` text elsewhere in the document.
- Workspace tests exclude uninitialized Markdown and retain initialized,
  malformed decks for diagnosis.
- Stamp tests prove maintenance refusal is byte-preserving and prove that
  initialized decks still receive missing card IDs. Explicit initialization
  also refuses a generic frontmatter `id` without changing the file.
- Review, augmentation, CLI, and doctor tests pin the shared behavior and
  actionable guidance.

## Reversal

Replacing the marker requires another explicit, durable authorization
mechanism that covers loose decks and every client. A migration must preserve
existing deck and card IDs and must prove that ordinary Markdown cannot be
mutated by discovery or open.
