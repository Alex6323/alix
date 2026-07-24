# 0009: CLI-backed AI providers

- Status: Accepted
- Recorded: 2026-07-24
- Retrospective: Yes

## Decision history

Commit `f216519` introduced the backend trait and Claude profile on 2026-07-02.
Commits `548d673`, `95fc680`, `7ef9a6d`, and `37c2909` added selection and
profiles for Claude, Gemini, Codex, and Copilot. Commit `913c0da` added
capability refusal and graceful degradation on 2026-07-03. Commit `9d1672b`
added scheduled CLI flag-drift checks.

## Context

Alix uses language models for generation, tutoring, semantic grading, and
source-grounded workflows. Providers expose different command syntax,
permission controls, prompt delivery, session support, and source-access
capabilities. Direct HTTP integrations would also make Alix responsible for
provider credentials, billing configuration, and changing API protocols.

The product needs one internal capability model without pretending the
providers are equivalent.

## Decision

Alix invokes subscription-authenticated, headless provider CLIs as its AI
backend boundary. The user signs in through the provider's own CLI. Alix does
not store provider API keys.

Built-in backend profiles translate a shared request into vendor-specific
arguments, prompt delivery, output extraction, and error mapping. Alix reduces
tool access to an abstract grant:

- no tools; or
- a read-only combination of local file reading, web fetch, and web search.

Each profile translates that intent into the strongest supported vendor
restriction. Required source capabilities that a backend lacks are refused
with a visible error. Session arguments are sent only to backends that support
them.

The vendor CLIs do not provide identical permission granularity. Gemini can
omit its tool allowlist, while Codex uses a read-only sandbox for both abstract
grant levels. Copilot denies shell and write, but its no-grant form can still
leave other non-destructive tools available. These are documented security
limits of the current CLI boundary, not claims of equivalent tool isolation.

Configuration may select a built-in backend and override its executable path,
model, effort, timeout, and documented permission inputs. It does not define an
arbitrary shell command or vendor-argument template in the default
architecture.

## Consequences

- Users can use existing provider subscriptions and login flows.
- Provider processes and versions are external runtime dependencies.
- Alix must maintain backend-specific argument and output adapters.
- Capabilities differ visibly; some source or multi-turn workflows cannot run
  on every provider.
- A text-only request is not a proven tool-free sandbox on every provider.
- CLI upgrades can break hardcoded flags independently of Alix releases.
- No provider API secret is added to Alix configuration or persistence.

## Alternatives considered

### Direct provider HTTP APIs

HTTP integration would offer protocol-level control, but it would require API
keys, metered billing setup, provider SDK or schema maintenance, and a new
secret-storage boundary.

### One provider-specific implementation

This would be simpler but would tie the product to one subscription,
capability set, and failure mode.

### Arbitrary user-defined shell profiles

Arbitrary command templates would make quoting, permissions, output parsing,
and support behavior unbounded. An executable-path override keeps local
installation flexibility while retaining a known profile contract.

### Claim a lowest-common-denominator capability set

Hiding provider differences would either disable useful safe capabilities or
quietly grant more access than the caller requested.

## Compatibility

Backend names and their configuration keys are user-facing configuration.
Provider flags are not under Alix's control, so profile updates may be required
without changing the internal capability model.

## Security

Provider CLIs are subprocesses acting with the user's account and local process
privileges. Alix requests tool-free or read-only operation and supplies a
bounded working directory where relevant, but vendor enforcement remains part
of the trusted computing base. Write and shell capabilities are not part of
the abstract grant. A caller that requires strict tool-free isolation cannot
infer it merely from `Access::None` on every current backend.

Prompts and allowed sources may contain private material. The user chooses the
provider and therefore the external data recipient.

## Verification

- `src/backend/` owns profiles, capability translation, argument construction,
  and extraction tests.
- `src/ask.rs` owns subprocess execution, timeouts, and error mapping.
- Fake-CLI tests assert exact argument and prompt-delivery behavior without
  live provider calls.
- `.github/workflows/backend-drift.yml` checks required flags against installed
  CLI help on a schedule.
- Backend health checks report missing commands and unsupported capabilities.

## Reversal

Add a direct API or plugin backend only when a product requirement cannot be
met safely through authenticated CLIs and the new credential, billing,
permission, and compatibility boundaries are specified. Existing CLI users
must retain a migration or supported path.
