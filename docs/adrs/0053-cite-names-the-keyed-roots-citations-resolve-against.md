# 0053: `cite:` names the keyed roots that card citations resolve against

- Status: Proposed
- Recorded: 2026-10-17
- Retrospective: No

## Context

Terms used below:

- A **deck** is a Markdown file of cards with a YAML frontmatter block. A
  **workspace** is a folder with an `alix.toml` manifest (TOML) and a `decks/`
  folder holding its **member decks**; a deck outside one is a **loose deck**.
- A card proves a fact with a **locator**, a comment after its answer:
  `<!-- at: <path>:<lines> fingerprint: xxh64-<hex> asset: sha256-<hex>.<ext> -->`
  (ADR 0020, ADR 0026). `fingerprint:` detects a change in the cited text.
- **Freezing** (`alix deck init` on a member deck, ADR 0015) copies each cited
  excerpt into the deck's asset folder, `assets/deck-<token>/`, under a
  content-addressed name, and writes it as `asset:`. Review then shows the
  frozen copy. `alix doctor` compares frozen copies with the live files and
  reports **drift**.
- A **trace deck** walks one path through a source: each card asks the
  learner to predict the next step and reveals the cited excerpt.
- The **AI exam** asks questions grounded in the deck's sources. The **tutor**
  answers questions about a card; with the `source_access` setting on, it may
  browse one local folder with read-only tools.

Today a locator resolves against the deck's local `source:` entries, the same
key that names the exam's ground truth and also accepts URLs. That one key
does two jobs, and citation breaks once a deck has more than one source:

- Freezing rewrites the path relative to its own source (a single-file source
  keeps only the bare file name) and drops which source it came from. Two
  sources that each hold `x.txt` both freeze to `at: x.txt:1`.
- The drift check joins every frozen path onto the first local source, so a
  citation into a later source warns "source file is gone" or "no longer
  found" while the file is unchanged.
- A path the first source lacks is retried against every ancestor folder up
  to the filesystem root, then by a downward search for the bare file name.
  A mistyped directory freezes whichever same-named file the search finds
  first.
- A path containing a space cannot be cited: ADR 0026 tokenizes the locator
  by single-space splits.
- A trace deck over a URL cites a quoted sentence of the page in `at:`. The
  sentence contains spaces, so the build writes a deck that does not parse.

Locators are written into deck files, so their grammar is decided here.

## Decision

### Keys and their jobs

1. `cite:` lists **keyed roots**: the places locators resolve against and
   freezing copies from. A root is a local directory or a URL. `source:` is
   renamed `exam:`, in decks and in the manifest: any path or URL the exam
   grounds in, unkeyed. `link:` (reading for the tutor that the exam
   ignores) is unchanged. One key, one job: `exam:` decides what the exam
   reads, `cite:` where card evidence comes from, `link:` extra reading for
   the tutor only. The tutor also lists `exam:` entries as context and may
   browse a `cite:` root (15).

2. Every root has a key, even when there is only one. In a deck:

   ```yaml
   cite:
     - code: crates/core
     - guide: https://example.org/guide
   ```

   Each list item is a one-key mapping. In the manifest, `cite` is a TOML
   table, where TOML itself rejects a repeated key:

   ```toml
   [cite]
   code = "crates/core"
   ```

   `cite` is not accepted inside the manifest's `[defaults]`.

3. A key matches `[a-z][a-z0-9_-]*` and is not `true`, `false` or `null`, in
   the deck and in the manifest alike. A key that breaks this, a YAML key that
   is not a string, and a key used twice in one deck's list are errors.

4. A member deck sees the manifest's keys plus its own. A deck key that
   repeats a manifest key is an error, never an override.

5. A local root is written as a platform path. A relative root resolves
   against the workspace folder, or the deck's own folder for a loose deck.
   A local root that is a file is an error. Two roots a deck sees (its own
   and the manifest's) may not contain one another, compared after resolving
   symlinks; nesting is an error naming both keys.

6. A URL root is provenance only: alix never fetches it. Only a member deck
   may declare one; a loose deck that does is an error. Every locator under a
   URL root carries an `asset:`, written by the build that captured the
   passage; one without it is an error.

### Locator grammar

7. The locator comment is spelled with single spaces: `<!-- at: <value>`,
   then ` fingerprint: <hex>` and ` asset: <object>` when present, then
   ` -->`. Any other spacing is an error. The value runs to the next
   ` fingerprint:`, ` asset:`, or ` -->`; inside a quoted path these do not
   end it. The value has exactly one of
   these shapes, and any other, or a shape that does not fit its key's root,
   is an error:

   ```
   <key>:<path>              whole file under a local root
   <key>:<path>:<range>      lines of a file under a local root
   <key>                     the passage frozen from a URL root
   ```

   The key runs to the first `:`. A quoted path runs from `"` to the next
   `"`; after the closing quote comes either nothing or `:<range>`, anything
   else, or a missing closing quote, is an error. An unquoted path ends at a
   trailing `:` followed by text of range shape, `[0-9]+(-[0-9]+)?`; that text
   is the range and must satisfy 9. Without such a tail the whole remainder is
   the path, so `code:x.txt:3-` cites the file `x.txt:3-`.

8. Any path may be quoted; a path containing a space must be. The quotes are
   not part of the path. A `"` inside a path, `-->` anywhere in a path, a
   control character (below U+0020, or U+007F), and an empty path are
   errors. There are no escape sequences. Alix's own writers
   quote a path only if it contains a space or ends in `:` plus text of range
   shape.

9. A range is `N` or `N-M`: decimal digits without leading zeros, `N` at
   least 1, `N` not greater than `M`.

10. A locator path is relative, `/`-separated on every platform, and
    normalized: no empty, `.` or `..` segment, no trailing `/`, no `\`, no
    leading `/`, and no first segment that starts with an ASCII letter
    followed by `:` (a drive letter on Windows). Paths compare byte for byte.

11. Every locator has a key its deck can see. A missing or unknown key, and
    any violation of 6 to 10, is an error in every command that reads the
    locator, `alix doctor` in all its modes included.

### Resolution

12. A local path is joined onto its key's root. There is no ancestor retry and
    no search for the bare file name. A locator never leaves its root: the
    symlink-resolved file must lie inside the symlink-resolved root.

13. A locator with an `asset:` is served from its frozen copy. Its live side
    (root, file or lines missing, or text changed) is drift: a doctor
    warning, never an error, so a shared frozen deck works without its
    sources. A locator without an `asset:` must resolve live; if its root,
    file or lines are missing, that is an error.

### Who reads what

14. The exam grounds only in `exam:`, the deck's own and the manifest's, as
    `source:` is read today. It never reads `cite:` roots or cited text. A
    deck is exam-eligible only when it or its manifest declares `exam:`. To
    examine a cited file, list it under `exam:` too.

15. With `source_access` on, the tutor browses the root of the first local
    key among the card's locators. A card without a local locator browses a
    root only when the deck sees exactly one local root; otherwise none. A
    URL root is never browsed.

16. A generator that writes locators (`alix workspace generate`, trace
    builds) declares its input as a root under the key `src`;
    `workspace generate` writes it into the manifest. A generator that
    writes none (a facts deck from `alix deck generate`) declares its input
    as `exam:`. When a manifest root
    already contains a local input, the generator reuses that key. An input outside every manifest root while the manifest declares
    `src` is an error. A URL trace build freezes each cited passage under its
    URL root; outside a workspace it is an error.

17. Deck transfer (`alix deck copy|move`) changes neither manifest and does
    not rewrite the deck. It is allowed only when every key the deck's
    locators use resolves in the destination, through the deck's own `cite:`
    or the destination's manifest, to the same root as in the source (local
    roots as canonical absolute paths, URLs as exact strings). Otherwise it
    refuses, naming each key and both roots.

## Consequences

- Citation roots and exam ground truth no longer share one key, so each can
  change without breaking the other.
- A deck that cites but declares no `exam:` cannot be examined, as a deck
  without a source cannot be today. A code deck that should be examined lists
  its files under `exam:` as well as its root under `cite:`. The exam reads
  each `exam:` entry as one file, so a directory entry still fails until the
  exam can read directories.
- Every existing locator lacks a key and stops resolving. Single-file-source
  traces (`at: 1-2`) and URL traces (`at: <sentence>`) are included. This is
  a pre-1.0 break; decks are rewritten by tooling outside the repository,
  which does not exist yet.
- A frozen URL passage is the model's copy of the page as its fetch tool
  returned it, not a byte-exact excerpt.
- Citing never makes text exam evidence: material the exam should read is
  listed under `exam:`.

## Alternatives considered

- **Key only multi-root decks.** Two locator forms, and adding a second root
  forces a rewrite of every existing locator.
- **Name the source in a separate locator field** (`at: x.txt:1 source: b`).
  Repeats a path in every locator and breaks when a root is renamed.
- **Index into the list** (`at: 2:x.txt:1`). Silently points at the wrong
  root when the list is reordered.
- **Also accept a path relative to the deck file, without a key.** Survives a
  card move only between decks in one folder, needs `..`, and is a second
  rule. Manifest roots already let cards move between member decks.
- **Fetch URL roots live.** Pages change or disappear; the frozen passage
  keeps the deck reviewable offline.
- **Snapshot the whole page.** Stores and shares far more than the cited
  passages, and needs an HTTP client and HTML conversion in alix.
- **A separate `quote:` directive for URL traces.** A one-line comment is the
  wrong container for a paragraph, and it is a second citation form.
- **File roots.** Need a lines-only form, which makes `spec:12` mean either
  lines 12 or a file named `12`, decided by the filesystem.
- **Silently normalize ranges and paths** (`10-4`, `./x`, `\`). Each silent fix
  is a second legal spelling forever. Optional quoting is the one accepted
  exception: `"x.txt"` and `x.txt` name the same path, so authors can always
  quote.
- **Escape `"` inside quoted paths.** Adds `\"` and `\\` for rare file names.

## Compatibility

Changes the deck frontmatter (new `cite:`, `source:` renamed `exam:`), the
workspace manifest (new `[cite]`, `source` renamed `exam`), and the locator
grammar. A deck or manifest still using `source` fails as an unknown key. Supersedes the tokenizing rule of ADR
0026's "Locator fields"; the field order and the `fingerprint:` and `asset:`
fields are unchanged. Old locators fail as ordinary invalid input; no reader
accepts the old form.

## Security

- A locator cannot leave its root (12), and the bare-file-name search and the
  ancestor retry are gone, so a typo or a crafted path cannot reach a file
  outside the declared roots.
- A root itself may be any local directory, including an absolute one. A
  received deck can therefore declare a root such as `/` and cite any file
  under it into its frozen assets. Local `exam:` entries carry the same
  exposure for the exam, as `source:` does today. Whether received decks may declare absolute or
  out-of-workspace roots is left to the receive path, not decided here.
- No new network access: URL roots are never fetched.

## Verification

To be named when accepted: parser tests for each rule in 3 and 6 to 11; a
two-root freeze and drift test; a containment test for `..`, absolute paths,
and an escaping symlink; a test that a mistyped path fails instead of
resolving elsewhere; a URL trace build test with a sentence-long passage.

## Reversal

Evidence that keys on single-root decks cost authors more than the rewrite
they prevent would justify optional keys. Moving back requires rewriting every
keyed locator.
