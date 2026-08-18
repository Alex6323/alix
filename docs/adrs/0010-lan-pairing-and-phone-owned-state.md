# 0010: LAN pairing and phone-owned state

- Status: Accepted
- Evidence: remote_endpoints_never_write_the_server_store in tests/api.rs
- Recorded: 2026-07-24
- Retrospective: Yes

## Decision history

Commit `c1690c1` guarded `/api/*` with a pairing token on 2026-07-03, and
commits `328ffe8` and `b30f02f` completed browser bootstrap and security
hardening. Commit `2627f26` added the remote tutor and exam surface on
2026-07-17. Commit `a500d07` verified that the remote surface never writes the
server store, and `48c455e` made the phone apply remote exam outcomes to its
own store.

## Context

The mobile app owns offline decks and progress but may borrow desktop-only AI
capabilities while both devices share a trusted local network. Exposing the
desktop server implicitly would leak private learning material. Letting remote
AI handlers mutate desktop progress would create two state owners and make
retries, disconnections, and synchronization ambiguous.

The existing server is a local application server, not an internet identity,
TLS, or multi-user authorization service.

## Decision

The server binds for localhost use by default. LAN exposure is an explicit
opt-in that produces a pairing URL and token. When a token is configured,
requests under `/api/*` require it through a bearer header or URL bootstrap,
and comparison is constant-time.

The static page shell and opaque `/img/<key>` URLs remain unauthenticated so a
browser can bootstrap and image elements can load. This is a bounded
trusted-LAN tradeoff documented in the API, not a general authorization model.

Remote request bodies are capped. Long-running remote AI families are
single-flight and separate from browser jobs. The pairing token protects
against casual unpaired use on a trusted LAN; it does not make the plain HTTP
server suitable for hostile networks.

For remote AI, the desktop computes but the phone owns state. Remote handlers
return tutor, exam, remediation, or generated-deck results without mutating the
desktop progress store. The phone validates and applies progress, exam, and
virtual-card outcomes to its local store.

## Consequences

- Local desktop use has no pairing ceremony.
- LAN access is visible and deliberately enabled.
- A paired phone can use desktop AI while retaining offline state ownership.
- A second remote client can collide with an in-flight single-flight job.
- Anyone on the LAN who obtains the token can use the guarded API.
- Page assets and addressed images may be read without the token.
- Internet exposure requires an operator-managed trusted tunnel or reverse
  proxy and is outside Alix's server.

## Alternatives considered

### Implicit LAN binding

This would make deck and AI surfaces reachable whenever the application
starts, without a deliberate exposure decision.

### Accounts and hosted synchronization

Accounts could support internet clients and coordinated state, but they would
introduce a hosted identity and operations boundary contrary to ADR 0001.

### Let remote handlers update the desktop store

This would make the desktop and phone competing owners of the same review
outcome. Network retries and later folder synchronization could double-apply
or overwrite progress.

### Make the phone a permanently server-dependent thin client

This would remove standalone offline review and conflict with ADRs 0006 and
0008.

### Treat the pairing token as internet-grade authentication

The connection is plain HTTP, assets have documented unauthenticated paths,
and the server has no users, revocation, audit, or TLS. Claiming a stronger
boundary would be misleading.

## Compatibility

Pairing bootstrap, bearer-token support, remote DTOs, and opaque image URLs are
documented client contracts in `docs/API.md`. State-application payloads must
remain versioned with the mobile bridge and core store semantics.

## Security

The supported threat model is a trusted LAN with explicit pairing. Tokens must
be random, compared without timing leakage, and excluded from logs where
possible. Request caps bound memory use. The open shell and image routes are
known disclosure surfaces.

Untrusted networks, internet exposure, several users, token revocation, or
fine-grained authorization require a superseding design rather than incremental
claims around the current token.

## Verification

- `src/serve/respond.rs` owns route guarding, token extraction, and
  constant-time comparison.
- `src/serve/mod.rs` caps remote bodies, separates single-flight jobs, and
  keeps remote handlers away from the desktop store.
- `src/serve/tests.rs` covers guarded API and open bootstrap routes.
- Remote API tests assert complete round trips and an unchanged server store.
- `mobile/alix/rust/src/api/review.rs` tests applying outcomes to the phone's
  local store.
- `docs/API.md` records the route and threat-model boundaries.

## Reversal

Supersede this ADR before supporting hostile-network access, multi-user
authorization, or internet-native operation. The replacement must define
transport security, identity, revocation, state ownership, retry idempotency,
and migration for existing paired clients.
