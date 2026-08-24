# Security

This directory contains Alix's tracked threat model. It describes the current
pre-1.0 product, not an aspirational security architecture. Update it whenever a
change affects networking, persistence, sharing/import, AI execution, source
access, rendering of untrusted content, mobile boundaries, or release
provenance. Review it during the required pre-release `make docs-audit`.

Vulnerabilities are reported privately through [`SECURITY.md`](../../SECURITY.md).

## Supported deployment model

Alix supports:

- one user controlling the local OS account and deck directories;
- a server bound to loopback by default;
- optional pairing across a network the user trusts;
- one active writer for each per-deck progress document;
- optional external AI provider CLIs selected and authenticated by the user;
- local or received Markdown decks and frozen source excerpts; and
- the lean Rust core embedded in the mobile client.

Alix does not claim to provide:

- isolation from a malicious process or person with the same local filesystem
  permissions;
- internet-grade hosting, TLS termination, account authentication, user roles,
  audit logging, or hostile-LAN protection;
- safe simultaneous writes to one deck's progress from multiple devices;
- containment of a compromised AI CLI, provider account, operating system,
  toolchain, or dependency; or
- confidentiality for content the user deliberately shares or submits to an AI
  provider.

## Assets

| Asset | Primary harm if compromised |
| --- | --- |
| Decks, notes, images, source citations, and frozen excerpts | Private learning or source material is disclosed or altered. |
| `progress/<deck-id>.json`, `recent.json`, and `alix.local.toml` | Learning history, device-local settings, or scheduling state is disclosed or corrupted. |
| Pairing tokens and profile configuration | An unintended LAN client can invoke guarded API operations. |
| User-created bug-report archive | Redacted diagnostics or metadata are disclosed after the user attaches the file. |
| Explicit `source` trees | A grounded AI call reads files outside the evidence the learner expected to share. |
| AI CLI session and provider account | Prompts or local reads are disclosed; model-backed actions are performed as the user. |
| Share-transfer process and received archive | An external transfer tool or hostile archive discloses data or writes unexpected files. |
| Authored deck files versus generated output | Untrusted generated or received content is mistaken for reviewed authored material. |

Availability matters, but preserving user-authored decks and progress takes
priority over keeping an individual session running.

## Trust boundaries and controls

### Local files and processes

Alix runs with the user's filesystem permissions. A progress save serializes
one deck's complete replacement document, writes a sibling temporary file,
checks its loaded revision against the canonical file, then renames the
replacement into place (`src/store.rs`). Every such replacement goes through
one helper (`src/fsio.rs`), which carries the existing file's permissions onto
the temporary before the rename and resolves a symlinked path to its target
first. Both properties are the file's own, not the writer's: a deck the user
restricted to owner-only keeps that mode instead of taking the process umask,
and a deck exposed through a symlink is repaired in place rather than having
the link replaced by a divergent regular copy. Detecting the link and
resolving it are separate states: a link whose target does not resolve, such
as one pointing at unmounted storage, fails the save with the link untouched
rather than falling back to renaming over the entry. A path that does not
exist yet is created with the umask, which is the intended default for a new
file. A writer marker and
synchronization-conflict detection warn about likely same-deck concurrency;
none of these mechanisms authenticates a writer or merges concurrent changes.
An error after the rename remains visible, but progress and augmentation
bookkeeping advances to the revision that already committed, keeping later
saves retryable.

Deck stamping has a separate local write boundary. A valid deck ID under the
`id` key in opening YAML frontmatter marks a file as initialized and
authorizes automatic maintenance of missing card IDs. The retired `alix-id`
key is an ordinary unknown frontmatter key: it lints and grants nothing,
because only `id: deck-<token>` marks a file initialized. `alix deck init <file>` is
the explicit opt-in that may mint the initial deck ID. Discovery and automatic
review or augmentation never grant that authority from `##` headings alone, so
ordinary Markdown remains byte-identical. Generated, imported, received,
library, and tutorial creation workflows initialize the output they
deliberately create (`src/parser/mod.rs`, `src/stamp.rs`, `src/workspace.rs`).

Source citations fail closed behind a content fingerprint. Review, trace, and
tutor grounding reveal a cited range only when its normalized displayed text
matches the stored xxHash64 value. A unique exact relocation may be proposed,
but ordinary doctor remains read-only; the explicit
`--repair-source-locators` flag may stamp a reviewed range or apply that exact
rebase. Changed and ambiguous excerpts are never substituted automatically.
The hash detects accidental drift and does not authenticate a deck or defend
against a malicious author who controls both content and metadata
(`src/source.rs`, `src/cli/doctor.rs`).

The mobile build excludes desktop server listeners, sharing, and provider
subprocesses, but the embedded core is not a sandbox. Parsers and filesystem
code still process content supplied to the app.

### Browser and LAN client

The server binds to `127.0.0.1` unless LAN mode is explicitly selected. LAN
launch generates a random 16-byte token unless a token is configured. Guarded
`/api/*` requests accept a bearer header or bootstrap query token and compare it
in constant time (`src/cli/launch.rs`, `src/serve/respond.rs`).

The server uses plain HTTP. The HTML/application shell and `/img/<key>` are
intentionally unauthenticated so a browser can bootstrap; the token protects
only `/api/*`. A token placed in a URL can appear in browser history, logs,
screenshots, or copied links. Every JSON request body reads through one
central 256 KiB cap (`json_body`), remote AI request bodies use their own
256 KiB cap, and ZIP uploads a 50 MiB cap (`src/serve/mod.rs`). Both cap
directions are pinned by `tests/api.rs`: oversized bodies are refused, and a
kilobytes-scale body must reach its handler rather than die at the cap
(2026-08-02: `/api/choose` briefly read its body uncapped straight off the
socket; fixed by routing it through `json_body`, regression in the same
test).

Remote tutor inputs are supplied by the client, remote exams resolve a selected
desktop deck, and remote generation accepts web URLs rather than a
client-selected desktop path. The remote exam handlers deliberately avoid the
poll path that writes progress. These controls do not turn pairing into a
multi-user authorization system.

### Local server log

Each running server writes its own size-capped log under the platform state
directory, falling back to the data directory on platforms without a state
directory. The file is outside the decks tree and is never included by share
or receive. Alix only writes it: no behavior, recovery path, or diagnostic
reads it back, and Alix has no upload or transmission path for it. Sending a
log is always the user's deliberate attachment to a bug report.

Log records may contain minted deck and card IDs. Default records cover card
selection plus content-free panic locations, AI backend failures, parse-error
classes and line numbers, and HTTP method, status, and broad request area.
They never contain card fronts, backs, notes, tutor or exam text, deck names or
titles, request paths, or deck and source paths, even when verbose targets are
enabled. No logging facade is installed, so dependencies cannot add their own
records. The verbose end-to-end content laws and the real-process rotation law
in `tests/cli.rs` enforce those boundaries.

### Bug-report archive

`alix bug-report` and `GET /api/bug-report` call one library implementation.
The endpoint is under the existing `/api/*` token guard and returns
`Cache-Control: no-store`. Both surfaces write or download a local ZIP only;
Alix has no submission or upload path. The user must inspect and attach it.

The archive includes bounded current logs and their one rollover, version and
platform data, a config rendered after recursively removing every `token`,
`prompt`, and `extra` key, and deck counts keyed by a SHA-256 hash of the stable
deck ID. By default it includes no deck text. `--include-deck <path>` adds only
that deck verbatim, including its card text and authored notes, and names it in
`report.md`. Personal sidecars, prompts, and responses are always excluded.
Every generated text passes through one redactor that removes configured token
values, the home path, and the home directory's user name. Structured logs
admit only known-safe fields; a newly added field is excluded until reviewed.
The archive remains potentially identifying through stable deck and card
hashes and operational history, which is why user review remains required.

A paired or localhost API client can invoke irreversible library removal with
the server process's filesystem authority. Client input names only a catalog
row; the Catalog owner resolves it to a validated loose deck, workspace member,
or manifested workspace, and rejects ambiguous names and plain folders. The
Study owner flushes progress first and refuses removal during an active session.
Responses expose only semantic artifact labels and do not persist exact failing
paths in the local log. The adult web app's type-the-name step protects against an
accidental click, not a malicious token holder. Treat a pairing token as
authority to delete served decks and their progress.

### AI provider

AI features execute a provider CLI as a subprocess under the local user's
account. Tool grants are translated into each provider's command-line controls;
provider behavior and enforcement are part of Alix's trusted computing base.
Prompts and supplied context leave the machine for the selected provider.

During `alix generate`, Claude and Codex return structured event streams. Alix
prints fixed status labels instead of model text, tool inputs, tool results, or
partial generated content. It retains stdout for final extraction and
validation, forwards each provider stderr line as a bounded local diagnostic,
and kills the provider process group when the configured absolute or inactivity
limit expires. The inactivity guard is armed only when the selected backend
provides structured progress events; silence from an unstructured backend does
not prove that it is stuck. For deck generation, those backends instead cap the
absolute limit at the configured inactivity value, preserving a bounded wedge
without claiming to observe activity. They expose only generic stdout activity
unless they write a diagnostic to stderr
(`src/ask.rs`, `src/backend/claude.rs`, `src/backend/codex.rs`).

The development-only dual-agent orchestrator also executes provider CLIs as the
local user. It gives Claude Code edit permission and Codex workspace-write
permission inside isolated experiment worktrees, but it is not a security
sandbox. Specs, plans, target repositories, provider configuration, and agent
commands must be trusted. Run evidence may contain source code, prompts, model
output, command output, and filesystem paths, so its external run directory
should be protected like the target checkout (`orchestrator/`).

Filesystem grounding is opt-in. `[ask] source_access = true` is effective only
when the deck or workspace declares a `source:`, and the grant covers exactly
that declared tree; `at:` locators remain evidence pointers and never infer a
wider project root (`src/deck.rs`, `src/source.rs`). This is the boundary for Alix's built-in grant. A user can
deliberately widen the provider CLI's permissions through `permission_mode`,
`allowed_tools`, or provider-specific configuration, and then owns that wider
trust decision.

URL sources are current external context rather than captured evidence. Alix
supplies frozen excerpts regardless; it fetches a URL source only when the
backend supports `WebFetch` and that tool is allowed. Without a usable source,
the tutor continues from frozen evidence and reports that it cannot verify the
full current source.

Received workspace manifests are untrusted configuration. Before using AI on a
received workspace, inspect `source_access`, `source`, links, citations, and
frozen excerpts. A portable manifest can request source access, and a source
path that exists on the receiving machine may expose more than the sender
intended.

### Decks, generation, sharing, and receiving

Authored and generated Markdown pass through the same parser and validation
rules before use. Model output is untrusted until it parses and the user
saves it. Rendering code must continue to treat authored text as
data rather than executable HTML.

`alix share` stages content locally and invokes the separately installed
`magic-wormhole` CLI for transfer. That executable and its protocol
implementation are part of the sharing trust boundary. Staging excludes the
entire `progress/` tree, recent state, local overrides, temporary and
backup-shaped files, hidden files, and synchronization conflicts. Receive
strips private state recursively even if the sender used another tool
(`src/share.rs`). Matching `augment/<deck-id>.json`
documents are intentionally shareable generated deck material; unrelated or
orphaned augmentation documents stay home. Frozen `assets/` are intentionally
shareable evidence and may contain proprietary or private source excerpts;
review the staged workspace before sending it.

Local `alix deck copy` and `alix deck move` call the same single-deck bundle
builder, sanitizer, validator, and installer as wormhole sharing. Copy cannot
carry progress because progress is outside that public bundle. Move handles the
matching progress document separately, publishes and validates the destination
before deleting the source deck, and refuses occupied destination paths or
stable deck-ID collisions (`src/share.rs`, `src/deck_transfer.rs`).

Received archives, decks, images, URLs, manifests, and source locators remain
untrusted. Sanitizing personal state does not certify the remaining content as
safe or accurate.

## Principal abuse and failure cases

| Scenario | Existing mitigation | Residual risk / operator action |
| --- | --- | --- |
| Another LAN device discovers Alix | Loopback default; LAN is explicit; `/api/*` needs a random token. | Use only a trusted LAN or put Alix behind a VPN/TLS reverse proxy. Replace a disclosed configured token and restart. |
| Pairing token leaks through a URL | API accepts a bearer header after bootstrap. | Treat the URL as a credential; do not publish screenshots, history, logs, or bookmarks containing it. |
| A paired client removes the wrong library row | Removal resolves only catalog-owned names, refuses active sessions, previews the exact Alix-owned set, and reports partial failures without disclosing host paths. | The token authorizes irreversible deletion. Verify the typed name and preview, keep independent backups, and run `alix doctor` after a partial failure. |
| A deck or page attempts prompt injection | Headless AI runs use explicit tool grants; source reads require a declared `source`. | Provider enforcement varies. Review sources and do not enable source access for untrusted workspaces. |
| A broad `source` exposes unrelated files | No root is inferred; the grant must be explicit. | Keep `source` as narrow as practical and inspect inherited workspace defaults. |
| Syncthing or another tool creates concurrent same-deck progress writes | Per-deck atomic replacement, revision checks, writer warnings, and conflict-file detection. | Different decks are independent; for one deck, keep one active writer, resolve conflict copies manually, and back up before recovery. |
| Sharing leaks personal state | Share filters it; receive strips it again. | Frozen excerpts and ordinary deck contents are still intentionally shared. |
| A support archive leaks credentials or unintended learning content | Token and prompt keys are dropped, generated text is redacted, deck identities are hashed, and personal sidecars are never read into the archive. `--include-deck` deliberately includes the one named deck verbatim. Deterministic tests open and inspect every archive entry. | Stable hashes, operational history, and any explicitly included deck can identify a learner or subject. Review the local archive before attaching it. |
| Ordinary Markdown resembles a deck | Discovery requires a valid opening-frontmatter `id` (`deck-<token>`); the retired `alix-id` is an ordinary unknown key that lints and grants nothing, and automatic stamping refuses an uninitialized file without writing. | Run `alix deck init <file>` only for an intended deck. Doctor reports deck-like files that remain ignored. |
| A numeric source range slides onto unrelated text | Every complete citation fingerprints the normalized excerpt and source consumers fail closed on a mismatch. | Review doctor findings before using explicit locator repair; fingerprints detect drift but do not prove semantic support. |
| A received ZIP attempts path traversal | The `zip` crate's extraction rejects unsafe enclosed paths; receive then strips personal-state files. | Treat the archive and external transfer tool as untrusted; inspect received content before opening or enabling AI. |
| Malformed or hostile input exhausts resources | JSON bodies share one central cap, and excerpts, remote AI bodies, and ZIP uploads have targeted caps; authored text is rendered as data and generated math SVG passes an allowlist. | Not every API route, local file, or operation has a global resource quota; avoid untrusted oversized collections. |
| A release or dependency is compromised | CI tests source changes; production toolchains are exact and direct Action references use immutable SHAs. Desktop release assets upload with per-asset SHA-256 checksums, and RustSec advisories are scanned nightly (advisory-drift) plus as a blocking `make audit` step at tag time, over both lockfiles. | Hosted runner images, operating-system packages, transitive Action behavior, signed artifacts, SBOMs, and full provenance are not yet a complete release guarantee. |

## Known security gaps

- LAN pairing has no TLS, accounts, roles, revocation list, rate limiting, or
  security audit log.
- The public shell and image route are outside pairing-token authentication.
- Browser security headers are not applied centrally.
- Profile tokens may be stable and URL bootstrap can expose them through normal
  browser tooling.
- A received workspace can carry AI/source-access configuration that needs
  human review.
- Per-deck state has fsynced atomic replacement, owner and revision checks,
  and conflict warnings, but no general same-deck merge or multi-writer
  transaction protocol; fail-on-Nth fault injection (`src/fsio.rs`'s fault
  seam and the progress and augmentation laws) runs on every platform,
  while the Windows
  directory-entry flush (`sync_dir` is a no-op there) and real power-loss
  testing remain open.
- Provider sandboxes and tool flags differ, and Alix cannot independently prove
  that a provider CLI honored them.
- Exact direct toolchain and Action pins now reduce release drift, but runner
  images and transitive build inputs are not hermetic. Release signing,
  checksums, SBOMs, and provenance are not yet complete across every
  distribution channel.

## Security regression evidence

The most relevant deterministic checks currently live beside their controls:

- `src/serve/tests.rs`: token scope and authorization behavior;
- `src/serve/respond.rs`: constant-time token comparison and capped reads;
- `src/log.rs` and `tests/cli.rs`: local-only bounded logging, content and
  name exclusion, and per-instance file separation;
- `src/bug_report.rs`, `tests/cli.rs`, and `tests/api.rs`: bundle redaction,
  content exclusion, deterministic layout, token-guarded download, and
  cross-surface byte identity;
- `src/library.rs`, `src/serve/study.rs`, and `tests/api.rs`: catalog-resolved
  irreversible removal, owner-serialized progress invalidation, display-safe
  failures, and retained-snapshot regression coverage;
- `src/deck.rs`: explicit-source precedence and no origin-root inference;
- `src/parser/mod.rs`, `src/stamp.rs`, and `src/workspace.rs`: explicit deck
  identity, byte-preserving refusal, and initialized-only discovery;
- `src/source.rs` and `src/cli/doctor.rs`: fail-closed citation integrity and
  explicit exact locator repair;
- `src/fsio.rs` and `tests/cli.rs`: permission-preserving, symlink-respecting
  file replacement shared by every deck and progress writer, including each
  `alix doctor` repair;
- `src/share.rs`: outgoing filtering and defensive receive sanitization;
- `src/deck_transfer.rs`: local transfer preflight, private progress handling,
  destination-first publication, and source-deletion rollback;
- `src/state.rs` and `src/workspace.rs`: typed user-file and workspace-file
  routing by stable deck ID;
- `src/store.rs` and `src/augment.rs`: per-deck atomic replacement, committed
  revision tracking, retryable partial aggregate saves, writer markers, and
  sync conflicts;
- `src/fsio.rs`: durable file replacement (data and directory-entry sync
  around the rename), committed-error reporting, and deterministic
  fail-on-Nth operation injection shared by every state, deck, and manifest
  writer;
- `src/ask.rs`, `src/backend/claude.rs`, and `src/backend/codex.rs`: bounded
  generation diagnostics, partial-output redaction, event-driven inactivity,
  and provider process-group termination;
- `src/trace_ai.rs`: generated snapshot provenance; and
- `src/math.rs` and renderer tests: validation and sanitization of generated
  math SVG.

A change to a listed boundary must update its regression tests, this threat
model, the public manual when behavior changes, and an ADR when it changes a
load-bearing security decision.
