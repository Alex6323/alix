# Security Policy

## Supported versions

Alix is pre-1.0 and changes quickly. Security fixes target the latest published
release on each maintained track: the desktop CLI and Android mobile app. Older
releases are not maintained as security-support branches. Reports against
`main` are welcome when the issue has not reached a release yet.

## Report a vulnerability privately

Email [contact@alix.study](mailto:contact@alix.study) with the subject
`[security]`. Do not open a public issue for a suspected vulnerability.

Include what you can without sending sensitive deck content:

- the affected Alix version, platform, and installation method;
- the security impact and who could trigger it;
- minimal reproduction steps or a small redacted reproducer;
- the affected command, route, file format, or trust boundary; and
- whether you believe the issue is already being exploited or publicly known.

If an attachment must contain private source material, ask how to transfer it
before sending it. Ordinary bugs still belong in the
[public issue form](https://github.com/Alex6323/alix/issues/new/choose).

## What happens next

The maintainer will acknowledge the report as soon as practical, validate it,
and coordinate a fix and disclosure with the reporter. Timing depends on impact
and maintainer availability; Alix does not promise a response-time SLA. A
confirmed issue may result in a release, release-note entry, and GitHub security
advisory when those are appropriate.

Alix does not currently operate a bug-bounty program.

## Good-faith research

Use your own accounts, devices, decks, and development instances. Avoid privacy
violations, service disruption, destructive testing, persistence on another
person's device, or access beyond what is needed to demonstrate the issue. Stop
and report if you encounter another person's data.

## Security boundaries

Alix is local-first, but some optional features cross trust boundaries:

- the web server binds to loopback by default; LAN mode is for a trusted local
  network and uses plain HTTP with a bearer-style pairing token;
- AI features run a separately installed provider CLI and may send prompts,
  card content, cited excerpts, or explicitly granted live-source context to
  that provider;
- a `source` citation identifies evidence but does not activate Alix's
  live-source grant; `[ask] source_access = true` also requires an explicit
  deck or workspace `origin`;
- progress files support one active writer; synchronization conflict warnings
  are not authentication or a general multi-writer merge protocol; and
- received decks, archives, workspace manifests, snapshots, and generated
  Markdown are untrusted input and may disclose their authored contents when
  reviewed, shared, or sent to an AI provider.

The tracked [threat model](docs/security/README.md) documents the current
deployment assumptions, implemented controls, and known gaps.
