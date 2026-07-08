# alix JSON API — the thin-client contract

This documents the HTTP+JSON surface that `alix`'s own web app consumes, as the
contract for any other client (native mobile, alternative web UIs). The library
crate is the single source of logic; this server surface is a thin consumer, and
so is every client.

> **Teeth.** Every response shape below is pinned by full-object snapshot
> tests (`mod contract` in `src/serve.rs`); the pinned examples are emitted to
> `tests/contracts/*.json` — the canonical machine-readable examples, and the
> input corpus for client-model codegen (e.g. quicktype → Dart). A shape
> change fails CI and names the section here to update. Code, this file, and
> the CHANGELOG move in one commit.

## 0. Stability & versioning

- The contract version is the crate version at the commit you are reading this
  file from — the doc has no version of its own; git does that job.
- alix is **pre-1.0**: breaking changes are allowed and land as **Breaking**
  entries under `CHANGELOG.md`'s Unreleased/release sections. Code, this doc,
  and the CHANGELOG move in the same commit.
- **Clients MUST ignore unknown fields.** New response fields appear without
  notice; that is not a break.
- **Enum vocabularies are open sets unless marked closed.** New `mode`, `state`,
  augment-target, or phase values may appear; switch statements need a default
  arm. Closed sets are marked *(closed)* below.
- Fields marked *(presentational)* exist for alix's own web page (tree glyphs,
  icons, key hints). They are on the wire and stable enough to render, but
  native clients should not build logic on them; they may change with UI work.

## 1. Connecting

- Default bind: `127.0.0.1`, port `7777` (config `[serve] port`, `--port`).
- `--lan` binds `0.0.0.0` and auto-generates a pairing token (16 random bytes,
  hex). Startup prints the pairing URL and triple:

  ```
  On another device, open in a browser:
    http://<this-machine's-IP>:<port>/?token=<hex>
  Or pair the app with:  host <IP>  port <port>  token <hex>
  ```

- There is no discovery endpoint (no mDNS, no `/api/ping`); host/port/token are
  handed to a client out of band from this printout — which also renders the
  pairing URL as a terminal QR code, with the machine's real IP.
- Several instances can serve side by side: `alix <dir>` scopes an instance to
  one decks folder with its own state (`--lan --port <p>` per instance). Each
  instance is its own host/port/token triple to pair against.
- `GET /api/version` → `{"version": "0.3.0"}` is the cheap "am I talking to
  alix, and which one" check.

## 2. Authentication

- With no token configured (the localhost default), everything is open.
- With a token (`--token`, `[serve] token`, or `--lan` auto-generation), **only
  `/api/*` is guarded**. The page shell (`/`, `/theme.css`, `/theme.js`,
  `/alix-logo.js`) and `/img/<key>` stay open — the browser must bootstrap from
  the `?token=` URL, and `<img>` tags cannot send headers. Treat `/img` URLs as
  unauthenticated on the LAN by design.
- Present the token either way (checked in this order):
  1. `Authorization: Bearer <token>` header — preferred for native clients;
  2. `?token=<token>` query parameter — the URL-bootstrap fallback.
- Comparison is constant-time. Failure → **401** with an empty body.

## 3. Conventions

- JSON keys are the Rust field names verbatim; no renaming layer.
- **Every documented key is always present.** Optional values serialize as
  `null` — they are never omitted. ("nullable" in the tables means
  null-possible, not sometimes-absent.)
- Responses: `Content-Type: application/json; charset=utf-8` and
  `Cache-Control: no-store` on all JSON. No ETags, no conditional requests.
- **No CORS headers.** A browser-based client must be served by alix itself
  (same origin). Native clients are unaffected.
- **Errors are bare status codes with empty bodies.** There is no error DTO.
  `400` is overloaded (malformed body, unknown deck name, store failure —
  per-endpoint meaning in §5). `409` = "no active session/exam/walk of the
  kind this endpoint needs". `401` = bad/missing token. `404` = unknown route
  or image. Clients should not assume bodies stay empty forever — a JSON
  `{"error": ...}` body may be added pre-1.0.
- **The polling pattern** (ask, exam, augment, walk auto-grade): a POST kicks a
  background thread and returns immediately with `thinking`/`busy` true; poll
  the matching GET until it clears, then read `error` or the results. alix's
  own client polls at ~400 ms. `elapsed` (seconds, nullable) is progress
  feedback for the in-flight call.

## 4. Flows

### 4.1 The review loop

1. `GET /api/decks` → the picker catalog (`DeckListDto`). Deck `name` values
   are the only keys `/api/select` accepts — names never contain filesystem
   paths, and requests cannot construct paths.
2. `POST /api/select {deck, topology?, region?, depth?, cram?, max_new?,
   limit?}` builds a session. **The response is either a `StateDto` or a
   `WalkDto` — branch on `kind` (`"review"` | `"walk"`) before anything
   else.** A trace deck walks; a fact deck reviews. `depth` is
   `"recognize" | "recall" | "reconstruct"` *(closed)*; omitted → the deck's
   remembered last depth. `cram` (default false) also queues cards that
   aren't due — a due card still grades as a normal review. `max_new` /
   `limit` override the instance's session pacing for this launch.
3. Render from `StateDto` (`phase:"review"`, `card`, `mode`, `depth`, counts).
   For typed checks call `POST /api/check {lines, ordered?}`; for a
   multiple-choice pick call `POST /api/choose {index}`. **Both are evidence
   only** — they report input-vs-expected and leave the session on the same
   card. Nothing is recorded yet.
4. `POST /api/grade` is **authoritative**: `{grade: "failed"|"partly"|"passed"}`
   *(closed)*, or `{covered, total}` for the explain-keypoints rubric. It
   applies the grade, saves progress, and returns the next `StateDto`.
   Other transitions: `/api/skip`, `/api/acquire` (acknowledge a never-seen
   card), `/api/remove` (mark for deck-file removal), `/api/promote`
   (virtual→deck file), `/api/restart`.
5. `GET /api/state` re-checks server-side due-ness (a missed card can cool back
   in) — poll it on the summary screen. Session end is `phase:"done"` on the
   same `StateDto`; there is no separate finished flag.
6. `POST /api/deselect` returns to the select phase.

### 4.2 The walk (trace decks)

`WalkDto` drives a predict-and-verify loop: `phase` cycles
`"predict"` → `"reveal"` → … → `"done"` *(closed)*. Submit a prediction with
`POST /api/walk/predict {text}`; grade with `POST /api/walk/grade {delta}`
where `delta` is a single key `"g"|"p"|"m"` (got it / partly / missed). With
auto-grade on, poll `GET /api/walk` while `thinking`. `POST /api/walk/restart`
rewalks; `POST /api/walk/leave` exits and, like every closer, returns the
picker `StateDto`.

### 4.3 The exam

`POST /api/exam/start {deck}` → `ExamDto` with `phase` walking
`"generating"` → `"answering"` → `"grading"` → `"results"`
(→ `"remediating"` → `"remediated"`). Poll `GET /api/exam` while `thinking`.
`POST /api/exam/answer {text, goto?}` saves and moves; `POST /api/exam/grade
{text}` submits the last answer and starts grading; `POST /api/exam/remediate`
generates remediation cards after a fail; `POST /api/exam/close` → `StateDto`.
When a trace re-sit is cooling down, `/api/exam/start` returns an `ExamDto`
in the `"cooldown"` phase with `cooldown_ms` set — one shape per endpoint.

### 4.4 Augment

`POST /api/augment/open {deck}` → `AugmentDto` (coverage rows per target).
`POST /api/augment/generate {target, with?}` kicks generation (poll
`GET /api/augment` while `busy`); `POST /api/augment/remove {target,
topology?}` deletes cached content; `POST /api/augment/close` → `StateDto`.
Target names are an open set (currently include `choices`, `notes`,
`keypoints`, `format`).

### 4.5 Ask (the tutor)

`POST /api/ask {question}` starts a call; poll `GET /api/ask` while
`thinking`; the growing `transcript` carries the whole exchange.
`POST /api/ask/note` condenses the exchange into a deck note. The walk has its
own mirror: `/api/walk/ask`, `/api/walk/ask/note`, `GET /api/walk/ask`.

## 5. Endpoint reference

Statuses: all endpoints can additionally return 401 (token) — omitted below.

### Meta & config

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| GET | `/api/version` | – | `VersionDto` | – |
| GET | `/api/doctor` | – | `DoctorDto` | – |
| GET | `/api/pair` | – | `PairDto` | – |
| GET | `/api/decks` | – | `DeckListDto` | – |
| GET | `/api/state` | – | `StateDto` (or `BrowseDto` while browsing) | – |
| GET | `/api/ask-info` | – | `AskInfoDto` | – |
| GET | `/api/keys` | – | web-private (§7) | – |
| GET | `/api/picker-keys` | – | web-private (§7) | – |
| GET | `/api/browse-keys` | – | web-private (§7) | – |

### Review session

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/select` | `{deck, topology?, region?, depth?, cram?, max_new?, limit?}` | `StateDto` \| `WalkDto` (branch on `kind`) | 400 bad body / unknown deck / build failure |
| POST | `/api/browse` | `{deck}` | `BrowseDto` | 400 (same causes) |
| POST | `/api/deck-topology` | `{deck}` | `DeckTopologyDto` | never errors — empty DTO on any failure |
| POST | `/api/deselect` | – | `StateDto` | – |
| POST | `/api/grade` | `{grade}` or `{covered, total}` | `StateDto` | 400 neither shape; 409 no session |
| POST | `/api/skip` | – | `StateDto` | 409 |
| POST | `/api/acquire` | – | `StateDto` | 409 |
| POST | `/api/check` | `{lines: [string], ordered?}` | `CheckFeedbackDto` | 400 bad body / no card; 409 |
| POST | `/api/choose` | `{index}` | `ChooseFeedbackDto` | 400 bad body / no question; 409 |
| POST | `/api/remove` | – | `StateDto` | 409 |
| POST | `/api/promote` | – | `StateDto` | 400 not a virtual card / promote failed; 409 |
| POST | `/api/restart` | – | `StateDto` | 409 |

### Ask

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/ask` | `{question}` | `AskDto` (empty question: 200, no call started) | 409 |
| GET | `/api/ask` | – | `AskDto` (poll) | 409 |
| POST | `/api/ask/note` | – | `AskDto` | 409 |

### Exam

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/exam/start` | `{deck}` | `ExamDto` (phase `cooldown` when a re-sit is cooling down) | 400 unknown deck; 409 not examable / locked |
| GET | `/api/exam` | – | `ExamDto` (poll) | 409 |
| POST | `/api/exam/answer` | `{text, goto?}` | `ExamDto` | 409 |
| POST | `/api/exam/grade` | `{text}` | `ExamDto` | 409 |
| POST | `/api/exam/remediate` | – | `ExamDto` | 409 |
| POST | `/api/exam/close` | – | `StateDto` | – |

### Augment

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/augment/open` | `{deck}` | `AugmentDto` | 400 unknown deck; 409 load failure |
| POST | `/api/augment/generate` | `{target, with?}` | `AugmentDto` | 409 |
| GET | `/api/augment` | – | `AugmentDto` (poll) | 409 |
| POST | `/api/augment/remove` | `{target, topology?}` | `AugmentDto` | 409 |
| POST | `/api/augment/close` | – | `StateDto` | – |

### Walk

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| GET | `/api/walk` | – | `WalkDto` (poll) | 409 not walking |
| POST | `/api/walk/predict` | `{text}` | `WalkDto` | 409 |
| POST | `/api/walk/grade` | `{delta: "g"\|"p"\|"m"}` | `WalkDto` | 400 no delta; 409 |
| POST | `/api/walk/restart` | – | `WalkDto` | 409 |
| POST | `/api/walk/ask` | `{question}` | `AskDto` | 409 |
| GET | `/api/walk/ask` | – | `AskDto` | 409 |
| POST | `/api/walk/ask/note` | – | `AskDto` | 409 |
| POST | `/api/walk/leave` | – | `StateDto` | 409 |

### Images

| Method | Path | Response |
|---|---|---|
| GET | `/img/<key>` | image bytes (content type by extension); 404 unknown key |

`<key>` is an opaque 16-hex hash; the URLs arrive inside `CardDto.img` /
`img_back` and `DeckItemDto.icon`. Unauthenticated (see §2). Part of the
contract — native clients need it to show card images.

## 6. DTO reference

Types are JSON types; `?` = nullable (still always present). Anchors are the
Rust struct names — `grep 'struct StateDto' src/serve.rs` finds the other
side. Canonical example payloads live in `tests/contracts/<Anchor>.json`,
emitted by the very tests that pin these shapes — always in sync by
construction.

### StateDto

The review-session payload; returned by every review action.

| Key | Type | Meaning |
|---|---|---|
| `kind` | string | Always `"review"` — the discriminator vs `WalkDto`. |
| `phase` | string | `"select"` \| `"review"` \| `"done"` *(closed)*. `done` is session end. |
| `card` | CardDto? | Null in select phase and when done. |
| `choices` | [string]? | Multiple-choice options; the correct index is never sent (see `ChooseFeedbackDto`). |
| `keypoints` | [string]? | Explain-check rubric lines. |
| `acquire` | bool | Never-seen card: show, then `/api/acquire` — no grading. |
| `mode` | string | The check being rendered: `flip` \| `typing` \| `typeline` \| `choice` \| `line` \| `explain` (open set). |
| `depth` | string | `recognize` \| `recall` \| `reconstruct` *(closed)*. |
| `input` | string | `type` \| `draw`. |
| `remaining` / `initial` / `reviews` / `passed` / `failed` | number | Session counters. |
| `exam_due` | [string] | Deck names whose exam unlocked; populated at `done`. |
| `can_restart` | bool | Anything due/new right now. |
| `promotable` | bool | Current card is a virtual (remediation) card. |
| `label` | string | Session header label *(presentational)*. |

Select-phase baseline: `phase:"select"`, `card:null`, `mode:"flip"`,
`depth:"recall"`, `input:"type"`, counters 0.

### CardDto

| Key | Type | Meaning |
|---|---|---|
| `front` | string | The question. Plain text — a planned highlight feature will add a parallel field, not markup here. |
| `context` | [string] | Cloze context lines. |
| `back` | [string] | Answer lines as displayed (may be a reshaped view). |
| `reshaped` | bool | `back` is the `format` augment's display shape. |
| `note` | [NoteUnitDto] | Post-answer note, as a tagged union. |
| `img` / `img_back` | string? | `/img/<key>` URLs. |
| `at` | string? | `% at:` citation locator. |
| `citation` | ExcerptDto? | Resolved citation excerpt. |
| `citation_error` | string? | Why `at` failed to resolve. |
| `crumb` | CrumbDto? | Topology breadcrumb (region heatmap). |

### NoteUnitDto *(tagged union — its `kind` is unrelated to StateDto's)*

`{"kind":"sentence", "text": string}` or `{"kind":"code", "lines": [string]}`.

### ExcerptDto / LineDto

`ExcerptDto`: `path: string`, `lines: [{n: number, text: string}]`,
`truncated: bool`.

### CrumbDto

`regions: [string]`, `current: number`, `cells: [[number]]` (0..=1 strengths,
per region per card) *(presentational)*.

### BrowseDto

`phase: "select"|"browse"`, `label: string`, `cards: [CardDto]`.

### DeckListDto

`workspaces: [DeckItemDto]`, `recent: [DeckItemDto]`, `folders: [DeckItemDto]`.

### DeckItemDto

| Key | Type | Meaning |
|---|---|---|
| `name` | string | The stable selection key — send back to `/api/select`. |
| `label` | string | Display title. |
| `meta` | string? | Badge text like `3/20`, `done ✓` *(presentational — parse nothing from it)*. |
| `state` | string | `new` \| `started` \| `finished` \| `examdue` for decks; `workspace` \| `folder` for groups (open set). |
| `locked` | bool | A `% requires:` prerequisite isn't passed (exam-gating only — drilling stays allowed). |
| `reviewable` | bool | Anything to do at any depth (or trace/exam special cases). |
| `reviewable_recognize` / `reviewable_recall` / `reviewable_reconstruct` | bool | Per-depth honest due-ness — gate depth choices on these. |
| `mastered` | bool | Exam passed. |
| `is_trace` | bool | Selecting it walks instead of reviewing. |
| `examable` | bool | Its exam can be sat right now. |
| `has_exam` | bool | Has an exam at all (even if locked). |
| `recent` | bool | Belongs in a recents view. |
| `is_workspace` | bool | Group flavor. |
| `description` | string? | Workspace goal line. |
| `members` | [MemberDto] | Workspace/folder members. |
| `path` | string? | Location hint *(presentational)*. |
| `icon` | string? | `/img/<key>` emblem URL. |
| `icon_svg` | bool | *(presentational)* |
| `has_topology` | bool | A focus drawer (region view) is available. |
| `badge_depth` | string? | Highest badged depth (`recognize`\|`recall`\|`reconstruct`). |
| `badge_dotted` | bool | The badge lapsed (render dotted) *(presentational)*. |
| `new_cards` | bool | Fresh material since badging. |
| `last_depth` | string | The deck's remembered session depth (default `recall`). |

### MemberDto

`DeckItemDto`'s fields minus the group-only ones, plus `indent: number` and
`tree: string` (the `├─`/`└─` branch prefix) — both *(presentational)*.

### DeckTopologyDto

`topologies: [{name, principle, regions: [{name, cells: [number],
due: number}]}]`, `deck_due: number`.

### CheckFeedbackDto

`results: [{input, expected, passed}]`, `passed: bool`. Evidence only — grade
separately.

### ChooseFeedbackDto

`chosen: number`, `correct: number`, `passed: bool`. This is where the correct
choice index is disclosed.

### AskDto / ExchangeDto / AskInfoDto

`AskDto`: `transcript: [{q, a}]`, `thinking: bool`, `status: string?`,
`error: string?`. `AskInfoDto`: `model: string`, `effort: string` (literal
`"default"` when unset).

### VersionDto

`version: string` (the crate version).

### DoctorDto / DoctorRowDto

The web doctor report (`GET /api/doctor`): the CLI's free checks (config,
store, decks, backend, share), serialized in that order. The costed
`--backends` end-to-end probe stays CLI-only — this endpoint never makes a
network call.

`DoctorDto`: `rows: [DoctorRowDto]`.

| Key | Type | Meaning |
|---|---|---|
| `name` | string | The check's name: `config` \| `store` \| `decks` \| `backend` \| `share` (open set — mirrors `alix doctor`'s rows). |
| `status` | string | `ok` \| `warn` \| `fail` (open set). |
| `detail` | string | What was found, one line. |
| `remedy` | string? | The fix; present whenever `status` isn't `ok`. |

Example (from the pinned test, illustrating the shape, not a real report):
`{"name":"config","status":"ok","detail":"~/.config/alix/config.toml parses","remedy":null}`
and ``{"name":"wormhole","status":"warn","detail":"`wormhole` not found on PATH","remedy":"pipx install magic-wormhole"}``.

### PairDto

The pairing sheet (`GET /api/pair`): the URL another device should open to
reach this instance, and a QR of it to scan.

| Key | Type | Meaning |
|---|---|---|
| `url` | string | The pairing URL (`http://<lan-ip>:<port>/?token=<t>` when reachable off-device, else `http://127.0.0.1:<port>/`). |
| `svg` | string? | A complete, self-contained inline `<svg>` element encoding `url` as a QR code — safe to inject directly into the page. Rendered black-on-white deliberately (scannability over theme-matching). `null` on a localhost-only instance, since there's nothing another device could reach. |
| `lan` | bool | Whether this instance is reachable off-device (mirrors `svg`'s presence). |

Example (localhost-only, from the pinned test):
`{"url":"http://127.0.0.1:7777/","svg":null,"lan":false}`.

### ExamDto

| Key | Type | Meaning |
|---|---|---|
| `phase` | string | `generating` \| `answering` \| `grading` \| `results` \| `remediating` \| `remediated` \| `cooldown` *(closed)*. |
| `deck` | string | Deck name. |
| `strictness` | string | `strict` \| `balanced` \| `lenient` *(closed)*. |
| `total` / `current` | number | Question count / current index. |
| `question` | string? | The current prompt (answering phase). |
| `answer` | string | Saved answer for the current question. |
| `on_last` | bool | |
| `grades` | [ExamGradeDto] | Populated in results. |
| `passed` | bool? | Null until graded. |
| `gaps` | [string] | Named understanding gaps. |
| `can_remediate` | bool | |
| `remediated_count` | number? | Null until remediation completes. |
| `is_trace` | bool | |
| `unlocks` | [string] | Deck names a pass unlocks. |
| `thinking` | bool | Poll while true. |
| `error` | string? | |
| `elapsed` | number? | Seconds the in-flight call has run. |
| `cooldown_ms` | number? | Milliseconds until a failed trace exam can be re-sat — set only in the `cooldown` phase. |

### ExamGradeDto

`question`, `points: [string]`, `answer`, `verdict`, `feedback`,
`missed: [string]`. **`verdict` is uppercase**: `PASS` \| `PARTIAL` \| `FAIL`
*(closed; note this vocabulary differs from grade tokens — two domains)*.

### AugmentDto / AugmentRowDto

`AugmentDto`: `deck`, `cards: number`, `rows: [AugmentRowDto]`,
`busy: string?` (the generating target), `elapsed: number?`, `error: string?`.
`AugmentRowDto`: `kind`, `label`, `covered: number`, `eligible: number`,
`items: [string]`, `busy: bool`.

### WalkDto

| Key | Type | Meaning |
|---|---|---|
| `kind` | string | Always `"walk"`. |
| `phase` | string | `predict` \| `reveal` \| `done` *(closed)*. |
| `description` / `source` | string / string? | |
| `total` / `current` | number | `current` is **1-based**. |
| `path` | [HopDto] | The rail. |
| `prompt` | string? | |
| `givens` | [string] | |
| `locator` | string? | |
| `prediction` | string? | Echoed on reveal. |
| `excerpt` | ExcerptDto? | |
| `excerpt_error` | string? | |
| `points` | [string] | Key points to self-check against. |
| `note` | string? | |
| `auto_grade` | bool | AI judges the prediction. |
| `thinking` | bool | Poll while an auto-grade is in flight. |
| `verdict` | string? | The auto-grade's verdict: `passed` \| `partly` \| `failed` *(closed — the same tokens as `HopDto.delta`)*. |
| `feedback` | string? | |
| `grade_error` | string? | |
| `summary` | SummaryDto? | Present at `done`. |

### HopDto

`prompt: string`, `delta: string?` (`passed` \| `partly` \| `failed`, null
while unwalked), `current: bool`.

### SummaryDto

`passed`, `partly`, `failed`, `total`: numbers; `weak: [number]` (**1-based**
hop numbers).

## 7. Web-page-private surface

These exist to serve alix's built-in web page and are **out of contract** —
they may change without notice and native clients must not depend on them:

- `GET /` (the SPA shell), `/theme.css`, `/theme.js`, `/alix-logo.js`;
- `GET /api/keys`, `/api/picker-keys`, `/api/browse-keys` — desktop keyboard
  binding maps (`KeyboardEvent.key` values) for the page's shortcut system.

## 8. Known quirks

- **Two verdict vocabularies** remain, deliberately: self-grade tokens
  (`passed`/`partly`/`failed`, lowercase — grades, walk deltas and verdicts)
  and AI-exam verdicts (`PASS`/`PARTIAL`/`FAIL`, uppercase). Distinct
  domains, both closed sets.
- **`/api/deck-topology` never errors** (empty DTO on any failure) and an
  empty `POST /api/ask` silently does nothing — clients cannot distinguish
  "none" from "bad request" there.
- **Request bodies are documented here but not snapshot-tested** (responses
  are). Body field names come from the same Rust-field convention.
- **No CORS** (see §3) — a browser client must be served same-origin by alix.

## 9. Planned surface (additive)

The picker-self-sufficiency wave adds endpoints for: deck **generation from a
URL**, TSV **import upload**, **share** (wormhole code shown in the UI) and
**receive** (paste a code), **reset** from the UI, a **doctor report**
(`GET /api/doctor`-style, free checks only), and a **pairing QR sheet**
(`GET /api/pair` returning the pairing URL + a server-rendered SVG). All
additive under §0's rules — clients that ignore unknown surface are unaffected.
