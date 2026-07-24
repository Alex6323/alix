# 0007: Client-neutral review contract

- Status: Accepted
- Recorded: 2026-07-24
- Retrospective: Yes

## Decision history

Commit `9e1fa51` introduced presentation-neutral `ReviewState` on 2026-07-13,
and `8d2e956` completed the initial review contract. Commits `49ff297`,
`7f2e412`, and `c16a5b0` moved session facts, state projection, choice
construction, and checking into the core. The HTTP snapshot discipline
predates that refactor in commit `e55229c` from 2026-07-08.

## Context

The web page and embedded mobile app need the same card projection and review
decisions through different transports. If JavaScript, Dart, HTTP handlers, or
bridge glue reconstructs modes, choices, grading transitions, or session
facts, the clients can disagree while each appears locally correct.

At the same time, HTTP needs transport-specific fields such as image URLs and
server-held context that do not belong in a lean embedded core.

## Decision

`ReviewState` and `CardView` are presentation-neutral core views. The core owns
the current card projection, mode and depth, acquisition state, choices,
checklists, session counters, restartability, and review transitions.

Embedded clients consume these types directly through bindings. The HTTP
server converts them into thin DTO envelopes. A DTO may add transport-specific
information such as image URLs, citations, or server-held navigation metadata,
but it must not reimplement a review decision.

The JSON API is a client contract:

- full-object snapshots pin response shapes;
- code, `docs/API.md`, snapshots, and the changelog change together;
- clients ignore unknown fields;
- response fields may be added compatibly;
- enum vocabularies are open unless explicitly documented as closed.

## Consequences

- A domain fix reaches web and mobile through one implementation.
- Core types cannot contain HTTP URLs, browser key hints, or server-owned
  objects.
- DTO adapters remain necessary even when many fields map directly.
- Client UIs decide layout and interaction mechanics, but not learning
  semantics.
- Contract evolution needs snapshot and documentation work.

## Alternatives considered

### Put review decisions in each client

This allows client-specific iteration but duplicates the most important
learning behavior and guarantees eventual drift.

### Use the HTTP DTO as the core model

This would pull server-specific URLs, citations, and wire evolution into the
embedded core and make offline mobile depend conceptually on HTTP.

### Define a separate FRB domain model

A bridge-only model would create another translation surface where fields and
decisions could diverge from both the core and HTTP.

### Expose raw `Card` and `Session` internals

Raw domain objects do not provide a stable presentation contract and would
force clients to infer display and transition rules.

## Compatibility

`CardView` and `ReviewState` are source contracts for embedded consumers. HTTP
DTOs and enum vocabularies are pre-1.0 wire contracts governed by
`docs/API.md`. Additive response fields are compatible only because clients
must tolerate unknown fields.

## Security

The projection must not disclose answer-only data before reveal. For example,
`ReviewState` carries choice options but not the correct choice; correctness
appears in feedback after an answer. Transport DTOs must preserve these
information boundaries.

## Verification

- `src/review.rs` owns core projection and review decision tests.
- `src/serve/dto.rs` visibly adapts `CardView` instead of rebuilding it.
- `src/serve/contract.rs` pins complete JSON response shapes and emits
  `tests/contracts/*.json`.
- `docs/API.md` defines evolution and unknown-field rules.
- `apps/mobile/rust/src/api/review.rs` mirrors the core types and keeps opaque
  Rust session handles.

## Reversal

Supersede this decision only if a client has a measured requirement the shared
contract cannot express without corrupting the core boundary. A replacement
must identify the new owner of each review decision, migrate both transports,
and preserve answer-disclosure and file-format semantics.
