# Releasing alix

alix is solo-built and pre-1.0. Releases are **driven by what's ready, not by a
calendar** — there is no fixed release train. The pipeline is automated, so
cutting a release is cheap; a version communicates *what changed*, not *when*.

## When to release

- **Per milestone (default).** When a coherent batch of work has landed on `main`
  — a feature theme, or a set of fixes worth updating for — cut a release. The
  themed sections in `PLAN.md`'s gantt are the natural milestones.
- **Monthly heartbeat (backstop, not a train).** If a month passes and
  `## [Unreleased]` in `CHANGELOG.md` has accumulated user-facing work, cut a
  release anyway. Don't let shipped work sit unreleased — it stays out of users'
  reach, and the changelog and roadmap drift out of sync with `main`.
  - *What reminds you:* **`make heartbeat`** (run at the start of a session, per
    `CLAUDE.md`) prints the unreleased-entry count + days since the last tag and
    flags when a release looks due (≥ 28 days with unreleased work). There's no CI
    cron — this session-start check is the reminder, chosen because the work flows
    through Claude sessions, so it nudges exactly when you're in context to act.
- Internal-only changes (refactors, tests, tooling) don't justify a release on
  their own; they ride along with the next user-facing one.

Revisit a fixed cadence only once there are real users who benefit from
predictable dates, or contributors to coordinate — likely after a 1.0.

## Versioning (pre-1.0)

`0.MINOR.PATCH`, following SemVer's 0.x convention (anything may change in 0.x):

- **MINOR** (`0.1 → 0.2`) — any feature batch, and any **breaking** change
  (renamed/removed flags, directives, commands, config keys). Breaking changes are
  free pre-1.0 — we carry no compat shims — and are recorded under
  `## [Unreleased]` → Changed as a **Breaking** note (see `CLAUDE.md`).
- **PATCH** (`0.2.0 → 0.2.1`) — bug fixes / docs only, no new surface.
- A `1.0` waits for real adoption and a surface worth stabilizing.

## How to cut a release

The GitHub Actions `release` workflow fires on a **`v*` tag**: it creates the
GitHub Release (notes pulled from the changelog) and cross-builds + uploads the
four binaries — `alix-aarch64-apple-darwin.tar.gz`,
`alix-x86_64-apple-darwin.tar.gz`, `alix-x86_64-unknown-linux-gnu.tar.gz`,
`alix-x86_64-pc-windows-msvc.zip` — that `alix.study` and `site/install.sh` point
at. **crates.io is not automated.**

1. **Green gate.** `make check` (clippy + tests) and `make fmt` clean; README,
   `docs/book`, and `CHANGELOG.md` in sync with the work being shipped.
2. **Bump the version.** Set `version = "X.Y.Z"` in `Cargo.toml`; refresh
   `Cargo.lock` (`cargo build`).
3. **Finalize the changelog.** Rename `## [Unreleased]` → `## [X.Y.Z] - YYYY-MM-DD`,
   then add a fresh empty `## [Unreleased]` (Added / Changed / Fixed) above it.
   The release notes come from this section, so its heading must match the tag.
4. **Commit** the bump + changelog: `Release vX.Y.Z`.
5. **Tag & push:** `git tag vX.Y.Z && git push origin main --tags`. CI creates the
   GitHub Release and attaches the binaries.
6. **Publish to crates.io (manual):** `cargo publish` (the package stays lean via
   `Cargo.toml`'s `exclude`; `cargo publish --dry-run` first if unsure).
7. **Verify reach.** The `pages` workflow redeploys `alix.study` + the mdBook on
   the `main` push automatically — confirm the site, the download buttons, and
   `install.sh` resolve the new asset names.

## After a release

Resync the local (gitignored) planning files: strike shipped items in `ROADMAP.md`
and update `PLAN.md` (re-audit, set the next milestone).
