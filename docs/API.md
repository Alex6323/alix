# alix JSON API — the thin-client contract

This documents the HTTP+JSON surface that `alix`'s own web app consumes, as the
contract for any other client (native mobile, alternative web UIs). The library
crate is the single source of logic; this server surface is a thin consumer, and
so is every client.

> **Teeth.** Every response shape below is pinned by full-object snapshot
> tests (`mod contract` in `src/serve/contract.rs`); the pinned examples are emitted to
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
  kind this endpoint needs". `401` = bad/missing token. `403` = an adult-only
  endpoint (§4.5) called while `[serve] audience = "kids"`. `404` = unknown
  route or image. Clients should not assume bodies stay empty forever — a JSON
  `{"error": ...}` body may be added pre-1.0.
- **A bare deck name that occurs in more than one container is a 400**
  (ambiguous) — use the qualified `<workspace>/<file>` key instead, which
  always resolves.
- **The polling pattern** (ask, exam, augment, walk auto-grade): a POST kicks a
  background thread and returns immediately with `thinking`/`busy` true; poll
  the matching GET until it clears, then read `error` or the results. alix's
  own client polls at ~400 ms. `elapsed` (seconds, nullable) is progress
  feedback for the in-flight call.

## 4. Flows

### 4.1 The review loop

1. `GET /api/decks` → the picker catalog (`DeckListDto`). Names never contain
   filesystem paths, and requests cannot construct paths — but only a
   **deck** row's `name` is a key `/api/select` accepts; a group row's name
   (`is_workspace`, or a folder) is a container, not a session (see
   `DeckItemDto.name`). `DeckItemDto.selectable` says this on the wire
   directly, so a client no longer has to infer it from `is_workspace`.
2. `POST /api/select {deck, topology?, region?, depth?, cram?, max_new?,
   limit?}` builds a session. **The response is either a `StateDto` or a
   `WalkDto` — branch on `kind` (`"review"` | `"walk"`) before anything
   else.** A trace deck walks; a fact deck reviews. `depth` is
   `"recognize" | "recall" | "reconstruct"` *(closed)*; omitted → the deck's
   remembered last depth. `cram` (default false) also queues cards that
   aren't due — a due card still grades as a normal review. `max_new` /
   `limit` override the instance's session pacing for this launch.
3. Render from `StateDto` (`phase:"review"`, `card`, `mode`, `depth`, counts).
   For typed checks call `POST /api/check {lines}`; whether the lines pair
   by position (`typeline`) or match in any order is derived server-side
   from the card's mode, never sent by the client. For a multiple-choice
   pick call `POST /api/choose {index}`. **Both are evidence only**: they
   report input-vs-expected and leave the session on the same card. Nothing
   is recorded yet.
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
where `delta` is a single key `"n"|"p"|"f"` (got it / partly / missed it —
the same letters as the review grade keys). With
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
`deck` may also name a **workspace or folder**: the screen then covers the
union of every member's cards, and the rows gain the workspace-only `icon`
target (0/1-covered by whether an emblem exists).
`POST /api/augment/generate {targets: [{target, with?}]}` kicks a batch, each
entry carrying its own optional guidance steer (poll `GET /api/augment` while
`busy`); the `AugmentDto` reports batch progress via `queued`, `done`, and
`failed` as the targets run one at a time.
`POST /api/augment/remove {target, topology?}` deletes cached content;
`POST /api/augment/close` → `StateDto`. Target names are an open set
(currently include `choices`, `notes`, `keypoints`, `format`, `icon`).

### 4.5 Ask (the tutor)

`POST /api/ask {question}` starts a call; poll `GET /api/ask` while
`thinking`; the growing `transcript` carries the whole exchange.
`POST /api/ask/note` condenses the exchange into a deck note. The walk has its
own mirror: `/api/walk/ask`, `/api/walk/ask/note`, `GET /api/walk/ask`.

`POST /api/ask/card/draft` asks the tutor to distill the conversation into one
draft card, following the same polling pattern: it starts a background call
and returns immediately, `thinking` while it's in flight; poll the shared
`GET /api/ask` and read the result off the response's `draft` field
(`DraftCardDto`), which persists there until the subject changes. `POST
/api/ask/card/create {front, back}` (`CreateCardReq`) mints the learner's
edited version of that draft as a free-standing virtual card on the current
deck; synchronous, no polling. Both endpoints are adult-only (`403` when
`[serve] audience = "kids"`) and both require an active review (`409`);
`/api/ask/card/create` further `409`s when the review has no current card,
and rejects an unparseable body with `400`. Success is `200` with `{"id":
"<decimal string>"}` (`CreateCardResp`), not `201`: alix's JSON responder
always answers `200` on success, so "created" is expressed by the DTO shape,
not the status line. A duplicate (the card's id already exists in the deck,
authored or virtual) or malformed front/back (empty after trimming, or not
exactly one well-formed card) is `422`.

### 4.6 Import

`POST /api/import {name, text, dest?}` → `ImportDto`, synchronous. `name`'s
extension picks the path: `.tsv` converts (Anki export, via
`import::tsv_to_deck`), `.txt` lands as a deck as-is; anything else is 400.
`dest` names a workspace/folder row (absent/empty → the served root) — the
same resolution map `/api/select` uses, never a client-supplied path. Unlike
`generate`'s lenient save, an upload that doesn't parse is rejected outright:
400, and the file is removed rather than kept around invalid — the original
upload still exists on the user's device, so nothing is lost.

### 4.7 Generate

`POST /api/generate {url, guidance?, dest?}` kicks a deck generation from a
web page — the first polled job of this family, following §3's polling
pattern: poll `GET /api/generate` while `phase:"generating"`, then read
`error` or the result. `url` must be `http://`/`https://`; the web surface
generates from URLs only — unlike the CLI's `alix generate`, which also
accepts a local path, since a LAN token holder must not be able to point the
server's AI at the server's own filesystem. `guidance`, when non-empty,
replaces the configured `[generate] extra` steer for this job only. `dest`
resolves exactly like `import`'s (§4.6). Unlike `import`, placement is
**lenient**: a generated deck that fails to parse is still saved, with the
problem reported in `error` — nothing costed by the model call is thrown
away. A destination name collision is checked before the model call, not
just at save time: if `dest` already has a same-named deck, `POST` responds
immediately with an error-phase `GenerateDto` and no job, so the collision
costs nothing. Only one generation runs at a time — `POST` while one is in flight is
409; a **finished** job (an `error` or `done` phase) is replaced by the next
`POST`. `POST /api/generate/close` clears the job unconditionally; a
still-running worker finishes into a discarded channel, the same as leaving
Augment.

### 4.8 Share

`POST /api/share {deck?}` stages a row (a deck file as-is; a folder minus
personal state: progress, recent list, local pacing overrides) and starts a
`wormhole send` on it, following §3's polling pattern: poll `GET /api/share`
while `phase:"staging"`/`"code"`, then read `error` or `"sent"`. `deck` is the
same resolution-map name `/api/select` uses; absent/`null` shares the served
root. `phase` walks `"staging"` (job started, no code yet) → `"code"`
(`code` set, show it to the other side) → `"sent"`, or `"error"` at any
point, including a spawn failure (`wormhole` not installed), which still comes
back as an error-phase `ShareDto` with the install hint, not a bare error
status. Only one share runs at a time: `POST` while one is in flight is 409; a
**finished** job (an `error` or `sent` phase) is replaced by the next `POST`,
mirroring Generate. `POST /api/share/close` cancels an in-flight transfer
(kills the wormhole child) and clears the job unconditionally.
`GET /api/share/zip[?deck]` is the offline fallback: synchronous, no polling,
it stages the same way and streams back a `.zip` instead of a wormhole code.

### 4.9 Receive

`POST /api/receive {code, dest?}` starts a `wormhole receive` for the given
code into a scratch dir, following §3's polling pattern: poll `GET
/api/receive` while `phase:"receiving"`, then read `error` or `"done"` (the
landed name plus any `stripped` personal files — see §4.8's `PERSONAL` list —
that leaked in because the sender didn't use `alix share`). `dest` resolves
exactly like Generate's (§4.7). Landing never overwrites: an existing entry of
the same name at `dest` is an `error`, not a silent replace. Only one receive
runs at a time: `POST` while one is in flight is 409; a **finished** job (an
`error` or `done` phase) is replaced by the next `POST`, mirroring Share.
Spawn failure (`wormhole` not installed) also surfaces as an error-phase
`ReceiveDto` with the install hint, never a bare error status. `POST
/api/receive/close` cancels an in-flight transfer (kills the wormhole child)
and clears the job unconditionally.

`POST /api/receive/zip[?dest]` is the offline fallback: synchronous, no
polling, it takes a `.zip` archive as the raw request body (§8) instead of a
wormhole code, unpacks it, lands it the same way, and responds a `"done"`-
phase `ReceiveDto` (`elapsed: 0`).

### 4.10 The remote surface (paired thin clients)

A paired phone, or any other native client, can borrow the desktop's AI
backend for the tutor, the AI exam, and deck generation over
`/api/remote/*`, instead of needing a model CLI of its own. **The iron rule:
no endpoint under `/api/remote/*` ever writes the server's own store,
session, decks, or recent list.** These handlers only compute an answer and
hand it back: a result leaves the server only as the HTTP response, never
onto disk.

A remote tutor turn is stateless on the server: `POST /api/remote/ask
{card, history, question}` re-sends the whole card and prior exchanges
(`history`, a list of `RemoteTurn`) every time; an empty `history` is
exactly the first turn. Poll `GET /api/remote/ask` while `thinking`, then
read `answer` or `error`, mirroring §4.5's pattern. A settled reply stays
readable on `GET /api/remote/ask` until the next POST replaces the slot,
and a GET before any POST is not an error, just a blank `RemoteAskDto`
(`thinking: false`, everything else `null`). `POST
/api/remote/ask/draft {card, history}` distills the exchange into a draft
card the same way `/api/ask/card/draft` does. Like `/api/ask/card/draft` and
`/api/ask/card/create` (§4.5), it is adult-only (403 under `[serve] audience
= "kids"`); `POST /api/remote/ask` itself carries no such gate. `POST
/api/remote/ask/note {card,
history}` (since 0.6.0) condenses the exchange into at most three note lines
the same way `/api/ask/note` does; unlike the draft call it carries no kids
gate, matching the web's own note route. An empty `history` is refused
(400): there is nothing to condense. All three share the `remote_ask` slot,
so a call into any one while another is thinking answers 409. The server
only returns the condensed lines (`RemoteAskDto.note`); it never appends
them to a deck file, same iron rule as everything else here, and the client
owns placing them.

A remote exam sitting starts with `POST /api/remote/exam/start {deck}`: the
server resolves **its own copy** of the named deck, by the same resolution
`/api/select` uses (a bare name, or a qualified `<workspace>/<file>`). A
trace deck now sits too (since 0.6.0): it opens straight in `answering`
with the path's one fixed compression question, graded the same way the
browser's own trace exam is (`RemoteExamDto.is_trace` tells the two apart,
§6). A non-trace, source-less deck is still refused outright (409): there is
nothing to examine. The browser's `% requires:` deck lock and trace re-sit
cooldown are read from the server's own store, which is not the phone's
truth, so a remote sitting skips both checks: it can start on a deck the web
app would show as locked or cooling down, and the phone applies its own
gates (a failed remote trace exam does not start the store-side cooldown,
so the phone must track its own). Unlike the browser's page-at-a-time
`/api/exam/answer` + `/api/exam/grade`, a remote client answers every
question locally and submits them as one batch: `POST
/api/remote/exam/grade {answers: [string]}` grades the whole sitting in one
call. A failed, remediable result's cards come back as deck-format text on
`RemoteExamDto.cards` (§6) for the **client** to parse and store: the
server generates them but never keeps them. A trace sitting never offers
remediation (`can_remediate` stays false): a failed compression is re-walked
instead.

`POST /api/remote/generate {url, guidance?}` mirrors §4.7's `/api/generate`,
minus everything the iron rule forbids: there is no `dest` (a phone chooses
its own destination) and no server-side collision pre-check (the phone owns
collisions). `url` must be `http://`/`https://`, exactly like the web.
Poll `GET /api/remote/generate` while `phase:"generating"`, then read the
full deck text off `RemoteGenerateDto.deck` and a suggested file name off
`filename` (§6): the client places the file, the server never does, so
unlike `GenerateDto` a parse failure does not flip `phase` to `error`; `cards`
is simply `null` and the client validates its own copy. `POST
/api/remote/generate/close` clears the job unconditionally.

Each family is single-flight and lives in its own slot, kept separate from
the browser's own `ask`/`exam` state: a second client pairing to the same
instance, or a double `POST` while one call is already in flight, gets
`409`. Every `/api/remote/*` POST body is capped at 256 KiB; an over-cap or
malformed body is a bare `400`: a long re-sent transcript is the likely way
to reach it as a conversation grows. Errors are bare status codes here too
(§3), and the pairing token (§2) applies exactly as it does to the rest of
`/api/*`.

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
| POST | `/api/reset` | `{deck}` | `ResetDto` | 400 bad body / unknown deck / load failure |
| POST | `/api/workspace/deadline` | `{name, date}` (`date`: `"YYYY-MM-DD"` \| `null`, required) | `DeckListDto` (refreshed) | 400 bad body / missing or bad date / not a workspace row; 500 write failure |
| POST | `/api/deselect` | – | `StateDto` | – |
| POST | `/api/grade` | `{grade}` or `{covered, total}` | `StateDto` | 400 neither shape; 409 no session |
| POST | `/api/skip` | – | `StateDto` | 409 |
| POST | `/api/acquire` | – | `StateDto` | 409 |
| POST | `/api/check` | `{lines: [string]}` | `CheckFeedbackDto` | 400 bad body / no card; 409 |
| POST | `/api/choose` | `{index}` | `ChooseFeedbackDto` | 400 bad body / no question; 409 |
| POST | `/api/remove` | – | `StateDto` | 409 |
| POST | `/api/promote` | – | `StateDto` | 400 not a virtual card / promote failed; 409 |
| POST | `/api/restart` | – | `StateDto` | 409 |

`/api/reset` wipes a row's stored progress (schedules, virtual cards, mastered
flag) outright — a typed-name confirmation is client UX, not enforced here; a
token holder is trusted to call it, the same trust class as `/api/grade`.

`/api/workspace/deadline` sets, moves, or clears (`date: null`) a workspace's
"ready by" target in its `alix.local.toml` ({#deadlines}). `date` is a
required key: send `null` explicitly to clear a deadline, but omitting the
key entirely is a 400, not a clear (a client bug must never be read as
"clear"). `name` must resolve to a real workspace row (an `alix.toml`
manifest, not a plain folder of decks); anything else, a missing `date` key,
or a `date` that doesn't parse as `YYYY-MM-DD`, is 400 and the file is left
untouched. It responds with the same `DeckListDto` shape `GET /api/decks`
returns, refreshed, so the picker re-renders the `deadline` readout (§6
`DeckItemDto`/`DeadlineDto`) in one round trip instead of a follow-up fetch.

### Import

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/import` | `{name, text, dest?}` | `ImportDto` | 400 bad body / unrecognized extension / bad TSV / unknown `dest` / parse failure |

### Generate

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/generate` | `{url, guidance?, dest?}` | `GenerateDto` | 400 bad body / non-http(s) `url` / unknown `dest`; 409 a generation is already in flight |
| GET | `/api/generate` | – | `GenerateDto` (poll) | 409 no generation |
| POST | `/api/generate/close` | – | 200 | – |

### Share

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/share` | `{deck?}` | `ShareDto` | 400 unknown `deck`; 409 a share is already in flight |
| GET | `/api/share` | – | `ShareDto` (poll) | 409 no share |
| POST | `/api/share/close` | – | 200 | – |
| GET | `/api/share/zip` | – (`?deck=` query, optional) | zip bytes (§8) | 400 unknown `deck` / staging or zip failure |

### Receive

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/receive` | `{code, dest?}` | `ReceiveDto` | 400 bad body / unknown `dest`; 409 a receive is already in flight |
| GET | `/api/receive` | – | `ReceiveDto` (poll) | 409 no receive |
| POST | `/api/receive/close` | – | 200 | – |
| POST | `/api/receive/zip` | raw zip bytes (§8; `?dest=` query, optional) | `ReceiveDto` (`done` phase) | 400 oversized (>50 MB) / not a zip / not exactly one entry / unknown `dest` / landing collision |

### Ask

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/ask` | `{question}` | `AskDto` (empty question: 200, no call started) | 409 |
| GET | `/api/ask` | – | `AskDto` (poll) | 409 |
| POST | `/api/ask/note` | – | `AskDto` | 409 |
| POST | `/api/ask/card/draft` | – | `AskDto` (draft lands on `draft` once polled) | 403 kids; 409 no active review |
| POST | `/api/ask/card/create` | `{front, back}` (`CreateCardReq`) | `CreateCardResp` | 400 bad body; 403 kids; 409 no active review / no current card; 422 duplicate or malformed |

### Exam

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/exam/start` | `{deck}` | `ExamDto` (phase `cooldown` when a re-sit is cooling down) | 400 unknown deck; 409 not examable / locked / backend can't reach the deck's source |
| GET | `/api/exam` | – | `ExamDto` (poll) | 409 |
| POST | `/api/exam/answer` | `{text, goto?}` | `ExamDto` | 409 |
| POST | `/api/exam/grade` | `{text}` | `ExamDto` | 409 |
| POST | `/api/exam/remediate` | – | `ExamDto` | 409 |
| POST | `/api/exam/close` | – | `StateDto` | – |

### Augment

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/augment/open` | `{deck}` | `AugmentDto` | 400 unknown deck; 409 load failure |
| POST | `/api/augment/generate` | `{targets: [{target, with?}]}` | `AugmentDto` | 409 |
| GET | `/api/augment` | – | `AugmentDto` (poll) | 409 |
| POST | `/api/augment/remove` | `{target, topology?}` | `AugmentDto` | 409 |
| POST | `/api/augment/close` | – | `StateDto` | – |

### Walk

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| GET | `/api/walk` | – | `WalkDto` (poll) | 409 not walking |
| POST | `/api/walk/predict` | `{text}` | `WalkDto` | 409 |
| POST | `/api/walk/grade` | `{delta: "n"\|"p"\|"f"}` | `WalkDto` | 400 no delta; 409 |
| POST | `/api/walk/restart` | – | `WalkDto` | 409 |
| POST | `/api/walk/ask` | `{question}` | `AskDto` | 409 |
| GET | `/api/walk/ask` | – | `AskDto` | 409 |
| POST | `/api/walk/ask/note` | – | `AskDto` | 409 |
| POST | `/api/walk/leave` | – | `StateDto` | 409 |

### Remote (paired clients) (since 0.6.0)

Every arm below is `/api/remote/*` (§4.10). `remote_ask` and `remote_exam`
are single-flight slots kept separate from the browser's own `ask`/`exam`
state: a second client pairing to the same instance collides with the
first, same as a double `POST`.

| Method | Path | Body | Response | Errors |
|---|---|---|---|---|
| POST | `/api/remote/ask` | `{card, history, question}` (`RemoteAskReq`) | `RemoteAskDto` | 400 bad/oversized body / empty question / a card with empty front and back; 409 a turn is already thinking |
| GET | `/api/remote/ask` | – | `RemoteAskDto` (poll) | – |
| POST | `/api/remote/ask/draft` | `{card, history}` (`RemoteDraftReq`) | `RemoteAskDto` | 400 bad/oversized body / empty `history`; 403 kids; 409 a turn is already thinking |
| POST | `/api/remote/ask/note` | `{card, history}` (`RemoteNoteReq`) | `RemoteAskDto` | 400 bad/oversized body / empty `history`; 409 a turn is already thinking |
| POST | `/api/remote/exam/start` | `{deck}` | `RemoteExamDto` | 400 bad/oversized body / unknown or ambiguous deck name; 409 a sitting is already open (close it first) / the deck fails to load / a non-trace source-less deck / a trace deck with no checkpoints / the backend can't reach a non-trace deck's source |
| GET | `/api/remote/exam` | – | `RemoteExamDto` (poll; `phase:"idle"` when no sitting is open) | – |
| POST | `/api/remote/exam/grade` | `{answers: [string]}` | `RemoteExamDto` | 400 bad/oversized body / wrong number of answers; 409 no sitting open / not in the answering phase |
| POST | `/api/remote/exam/remediate` | – | `RemoteExamDto` | 409 no sitting open / nothing to remediate |
| POST | `/api/remote/exam/close` | – | 200 | – |
| POST | `/api/remote/generate` | `{url, guidance?}` | `RemoteGenerateDto` | 400 bad/oversized body / `url` not `http://`/`https://`; 409 a generation is already thinking |
| GET | `/api/remote/generate` | – | `RemoteGenerateDto` (poll) | 409 no job |
| POST | `/api/remote/generate/close` | – | 200 | – |

### Images

| Method | Path | Response |
|---|---|---|
| GET | `/img/<key>` | image bytes (content type by extension); 404 unknown key |

`<key>` is an opaque 16-hex hash; the URLs arrive inside `CardDto.img` /
`img_back` and `DeckItemDto.icon`. Unauthenticated (see §2). Part of the
contract — native clients need it to show card images.

## 6. DTO reference

Types are JSON types; `?` = nullable (still always present). Anchors are the
Rust struct names — `grep 'struct StateDto' src/serve/dto.rs` finds the other
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
| `remaining` / `initial` / `reviews` / `passed` / `failed` / `acquired` | number | Session counters; `acquired` counts never-seen cards introduced this sitting (a first pass is acquire-only, so `reviews` alone reads 0). |
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
| `name` | string | The stable resolution key for this row (also used by `/api/reset`, `/api/browse`, and import's/generate's `dest`). **Only deck rows are selectable:** a group row (`is_workspace`, or a folder) is a container — `/api/select` rejects it with 400. Drill into `members` and select one of those. |
| `selectable` | bool | **structural**: this row's `name` is the kind of thing `/api/select` accepts (a deck; false for workspace/folder rows). `reviewable*` is state, not structure. |
| `label` | string | Display title. |
| `meta` | string? | Badge text like `3/20`, `done ✓` *(presentational — parse nothing from it)*. |
| `state` | string | `new` \| `started` \| `finished` \| `examdue` for decks; `workspace` \| `folder` for groups (open set). |
| `locked` | bool | A `% requires:` prerequisite isn't passed (exam-gating only — drilling stays allowed). |
| `reviewable` | bool | Anything to do at any depth (or trace/exam special cases). On a **group** row this aggregates its members — it describes what's due inside, not that the group itself can be selected. |
| `reviewable_recognize` / `reviewable_recall` / `reviewable_reconstruct` | bool | Per-depth honest due-ness — gate depth choices on these. Same group-aggregates-members caveat as `reviewable`. |
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
| `deadline` | DeadlineDto? | A workspace's "ready by" target ({#deadlines}). Present only on a workspace row whose `alix.local.toml` sets one; `null` on every deck/folder row and on a workspace with none set. **Additive**: clients must tolerate its absence/null, same as any other optional field here. |

### DeadlineDto

| Key | Type | Meaning |
|---|---|---|
| `date` | string | ISO `YYYY-MM-DD`. |
| `days_left` | number | `date` minus today, in local days. Goes negative past the date (render "was due", not an error). |
| `ready` | number | Member decks counted ready: mastered, or finished with no exam to pass (source-less). |
| `total` | number | The workspace's member decks. |

### MemberDto

`DeckItemDto`'s fields minus the group-only ones, plus `indent: number` and
`tree: string` (the `├─`/`└─` branch prefix) — both *(presentational)*.
`selectable` is always `true` here — a member row is always a deck file,
never a group.

### DeckTopologyDto

`topologies: [{name, principle, regions: [{name, cells: [number],
due: number}]}]`, `deck_due: number`.

### CheckFeedbackDto

`results: [{input, expected, passed}]`, `passed: bool`. Evidence only — grade
separately.

### ChooseFeedbackDto

`chosen: number`, `correct: number`, `passed: bool`. This is where the correct
choice index is disclosed.

### AskDto / ExchangeDto / AskInfoDto / DraftCardDto

`AskDto`: `transcript: [{q, a}]`, `thinking: bool`, `status: string?`,
`error: string?`, `draft: DraftCardDto?`. `draft` is the last card the tutor
drafted from the conversation (`POST /api/ask/card/draft`, §4.5); it persists
until the subject changes. `DraftCardDto`: `front: string`, `back:
[string]`. `AskInfoDto`: `backend: string` (the configured AI backend's
canonical lowercase name: `"claude"` | `"gemini"` | `"codex"` | `"copilot"`;
clients use it to name who is answering), `model: string`, `effort: string`
(literal `"default"` when unset).

### CreateCardReq / CreateCardResp

`CreateCardReq` is the request body for `POST /api/ask/card/create` (§4.5):
`front: string`, `back: [string]`, the learner's edited draft. It derives
`Deserialize` only, so it is documented here but not snapshot-pinned (§8's
"request bodies aren't snapshot-tested" note). `CreateCardResp`: `id: string`
(decimal, the newly minted virtual card's id, matching how the store keys
ids).

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
and ``{"name":"share","status":"warn","detail":"`wormhole` not found on PATH","remedy":"pipx install magic-wormhole"}``.

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

### ResetDto

The result of `POST /api/reset`: what got wiped.

| Key | Type | Meaning |
|---|---|---|
| `deck` | string | The row's resolved display name (as sent in the request). |
| `cards_cleared` | number | How many card schedules were removed. |

A `deck` naming a workspace/folder row resets every member deck it lists, not
just one file. Example (from the pinned test):
`{"deck":"rust.txt","cards_cleared":17}`.

### ImportDto

The result of `POST /api/import`: the placed file's name and its card count.

| Key | Type | Meaning |
|---|---|---|
| `deck` | string | The written file's name (its `name` after sanitizing to a bare file-name and forcing a `.txt` extension). |
| `cards` | number | How many cards it parsed to. |

Example (from the pinned test): `{"deck":"kanji.txt","cards":40}`.

### GenerateDto

The result of `POST`/`GET /api/generate`, polled per §3's pattern (§4.7).

| Key | Type | Meaning |
|---|---|---|
| `phase` | string | `generating` \| `done` \| `error` (open set). |
| `deck` | string? | The placed file's name — set once `done`. |
| `cards` | number? | Cards it parsed to — set once `done`. |
| `elapsed` | number? | Seconds since the job started (kept ticking even after it finishes). |
| `error` | string? | Set on `error` — including a parse failure on a deck that was still saved. |

Example (from the pinned test):
`{"phase":"done","deck":"rust-ownership.txt","cards":12,"elapsed":41,"error":null}`.

### ShareDto

The result of `POST`/`GET /api/share`, polled per §3's pattern (§4.8).

| Key | Type | Meaning |
|---|---|---|
| `phase` | string | `staging` \| `code` \| `sent` \| `error` (open set). |
| `code` | string? | The wormhole code mnemonic, once received — shown to the other side. |
| `elapsed` | number? | Seconds since the job started (kept ticking even after it finishes). |
| `error` | string? | Set on `error` — including a spawn failure (`wormhole` not installed), which surfaces the install hint here rather than a bare error status. |

Example (from the pinned test): `{"phase":"code","code":"7-alpha-bravo","elapsed":3,"error":null}`.

### ReceiveDto

The result of `POST`/`GET /api/receive` (and the synchronous `POST
/api/receive/zip`), polled per §3's pattern (§4.9).

| Key | Type | Meaning |
|---|---|---|
| `phase` | string | `receiving` \| `done` \| `error` (open set). |
| `landed` | string? | The landed file/folder name — set once `done`. |
| `stripped` | [string] | Personal files (§4.8) stripped from a leaked folder, if any. |
| `elapsed` | number? | Seconds since the job started (kept ticking even after it finishes); always `0` for the synchronous `/api/receive/zip`. |
| `error` | string? | Set on `error` — including a spawn failure (`wormhole` not installed), which surfaces the install hint here rather than a bare error status. |

Example (from the pinned test):
`{"phase":"done","landed":"rust-decks","stripped":["progress.json"],"elapsed":9,"error":null}`.

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
`busy: string?` (the generating target), `elapsed: number?`, `error: string?`,
`queued: [string]` (targets still waiting behind the busy one in the current
batch), `done: [string]` (targets the current batch has already finished
successfully), `failed: [{target, error}]` (targets the current batch
attempted and failed; one target's error doesn't stop the rest from running).
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

### RemoteCard / RemoteTurn / RemoteAskReq / RemoteDraftReq / RemoteNoteReq

Request bodies for the remote surface (§4.10). The server holds no card or
session of its own for a remote call, so the client sends full context every
time; these derive `Deserialize` only, so, like `CreateCardReq`, they are
documented here but not snapshot-pinned (§8).

`RemoteCard`: `subject: string`, `front: string`, `back: [string]`, `at:
string?` (the card's `% at:` citation locator, if any: carried through for
completeness, though the ungrounded tutor prompt doesn't read it).

`RemoteTurn`: one prior tutor exchange the client re-sends: `q: string`,
`a: string`.

`RemoteAskReq` (`POST /api/remote/ask`): `card: RemoteCard`, `history:
[RemoteTurn]`, `question: string`.

`RemoteDraftReq` (`POST /api/remote/ask/draft`): `card: RemoteCard`,
`history: [RemoteTurn]`.

`RemoteNoteReq` (`POST /api/remote/ask/note`; since 0.6.0): `card:
RemoteCard`, `history: [RemoteTurn]` (the same shape as `RemoteDraftReq`,
kept as its own type so the name matches its own endpoint).

Example `RemoteAskReq` body (a real request from `tests/api.rs`'s remote
round-trip suite):
`{"card":{"subject":"sample.txt","front":"2 + 2","back":["4"],"at":null},"history":[],"question":"why does this matter?"}`.

### RemoteAskDto

The reply to a remote tutor call (`POST`/`GET /api/remote/ask`, `POST
/api/remote/ask/draft`, `POST /api/remote/ask/note`; §4.10). Unlike
`AskDto`, it carries no transcript of its own (the client already holds it),
just the newest turn's outcome.

| Key | Type | Meaning |
|---|---|---|
| `thinking` | bool | Poll while true. |
| `answer` | string? | The tutor's reply to a question call. `null` for a draft or note call, or while thinking. |
| `draft` | DraftCardDto? | The drafted card from a draft call. `null` for a question or note call, or while thinking. |
| `note` | [string]? | Condensed note lines (at most three) from a note call, since 0.6.0. `null` for a question/draft call, or while thinking; an empty array is a valid settled outcome ("nothing to save"), not an error. |
| `error` | string? | Set on failure. |
| `elapsed` | number? | Seconds the in-flight call has run; `null` once settled. |

Example, settled with a draft (from the pinned test):
`{"thinking":false,"answer":"so drops are deterministic","draft":{"front":"Why does Rust use one owner per value?","back":["so drops are deterministic","no GC needed"]},"note":null,"error":null,"elapsed":null}`.

Example, settled with a note (from the pinned test):
`{"thinking":false,"answer":null,"draft":null,"note":["ownership drops values deterministically","no GC needed"],"error":null,"elapsed":null}`.

### RemoteExamDto

A paired phone's AI exam sitting (`/api/remote/exam/*`; §4.10). Unlike
`ExamDto`, there is no server-side session: answering happens client-local
as one batch, so there is no `total`/`current`/`question`/`answer`/`on_last`;
the client counts its own remediation cards, so there is no
`remediated_count`; and there is no server-side `cooldown_ms` or `unlocks` (a
trace re-sit is never gated here, and a pass unlocks nothing server-side,
so the phone applies both to its own state). A trace deck sits like a fact
deck, distinguished by `is_trace` (since 0.6.0).

| Key | Type | Meaning |
|---|---|---|
| `phase` | string | `idle` \| `generating` \| `answering` \| `grading` \| `results` \| `remediating` \| `remediated` (open set). `idle` is the baseline when no sitting is open: `ExamDto` has no equivalent, since the browser's exam slot simply doesn't exist between sittings. |
| `deck` | string | Deck name; empty at `idle`. |
| `strictness` | string | `strict` \| `balanced` \| `lenient` *(closed)*. |
| `questions` | [string] | Prompts only: the rubric (`ExamQuestion.points`) never leaves the server. A trace sitting has exactly one, the path's fixed compression question. |
| `passed` | bool? | Null until graded. |
| `grades` | [ExamGradeDto] | Populated in `results`/`remediated`. A trace sitting's is always one holistic grade. |
| `gaps` | [string] | Named understanding gaps. |
| `can_remediate` | bool | Always false for a trace sitting: a failed compression is re-walked, not remediated into cards. |
| `cards` | string? | Deck-format text, set in the `remediated` phase: the client parses and stores these; the server never does. Always null for a trace sitting. |
| `is_trace` | bool | A trace (compression) sitting vs a fact-deck sitting, since 0.6.0. `false` at `idle`. |
| `thinking` | bool | Poll while true. |
| `elapsed` | number? | Seconds the in-flight call has run. |
| `error` | string? | |

Example, remediated (from the pinned test):
`{"phase":"remediated","deck":"rust.txt","strictness":"balanced","questions":["Why does Rust use ownership?"],"passed":false,"grades":[{"question":"Why does Rust use ownership?","points":["memory safety without a GC"],"answer":"it has a garbage collector","verdict":"FAIL","feedback":"Rust has no GC","missed":["memory safety without a GC"]}],"gaps":["ownership and the GC-free memory model"],"can_remediate":false,"cards":"# Why does Rust use ownership?\n\tso drops are deterministic, no GC needed","is_trace":false,"thinking":false,"elapsed":null,"error":null}`.

Example, a trace sitting's failed result (from the pinned test):
`{"phase":"results","deck":"trace.txt","strictness":"balanced","questions":["how it works"],"passed":false,"grades":[{"question":"how it works","points":["it reads the first line"],"answer":"it reads the file","verdict":"FAIL","feedback":"missed the second hop","missed":["it reads the second line"]}],"gaps":["it reads the second line"],"can_remediate":false,"cards":null,"is_trace":true,"thinking":false,"elapsed":null,"error":null}`.

### RemoteGenerateDto (since 0.6.0)

A paired phone's deck generation (`/api/remote/generate*`; §4.10). Mirrors
`GenerateDto` (§4.7), but the server places nothing: there is no saved file
name, only the full deck text and a suggested one for the client to save
under.

| Key | Type | Meaning |
|---|---|---|
| `phase` | string | `generating` \| `done` \| `error` (open set). |
| `deck` | string? | The full generated deck text, set only in `done`. |
| `filename` | string? | A suggested file name (`generate::deck_name`, normalized to a `.txt` stem), set only in `done`; the client decides where and under what name to save it. |
| `cards` | number? | The finished text's own parsed card count, best-effort: `null` if it doesn't parse. Unlike `GenerateDto`, a parse failure does not flip `phase` to `error`, since the server never saves the file either way; the client parses and validates its own copy. |
| `elapsed` | number? | Seconds the in-flight call has run. |
| `error` | string? | |

Example, done (from the pinned test):
`{"phase":"done","deck":"% link: https://example.org\n# Q\n\tA\n","filename":"example-org.txt","cards":1,"elapsed":null,"error":null}`.

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
- **`GET /api/share/zip` is the one non-JSON API response**, deliberately: it
  streams `application/zip` bytes with a `Content-Disposition: attachment`
  header (§4.8's offline fallback to the wormhole flow) rather than a DTO.
- **`POST /api/receive/zip` is the one non-JSON API *request***, deliberately:
  the request body is raw `application/zip` bytes (§4.9), not a JSON envelope
  — avoids a base64 dependency for what's already a binary transfer. Capped at
  50 MB; rejected with 400 before the body is read when `Content-Length` says
  it's over, and the read itself is also capped (the header can lie or be
  absent) so an oversized upload can't be used to exhaust memory.

## 9. Planned surface (additive)

Remaining Maybes, not committed: a **workspace-init button** (`alix workspace
init` stays CLI-only for now) and a **stats panel** (`alix stats`'s per-depth
breakdown, not yet on the wire). Neither is scoped or scheduled.
