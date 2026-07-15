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

1. **Green gate.** `make fmt`, then `make preflight` (the strict gate: CI's
   blocking jobs under `-Dwarnings` plus a clean-tree check). Do NOT rely on
   `make check` here: it is lenient (no `-Dwarnings`), so it passes on warnings
   that CI rejects. README, `docs/book`, and `CHANGELOG.md` in sync with the work
   being shipped. Move any mobile-app-only entries out of the crate `CHANGELOG.md`
   into `apps/mobile/CHANGELOG.md` (they ship on `mobile-v*` tags, not here). (The
   README coverage badge is live Codecov since 0.4.0, tracking `main` by itself,
   no per-release refresh.)
2. **Bump the version.** Set `version = "X.Y.Z"` in `Cargo.toml`; refresh
   `Cargo.lock` (`cargo build`).
3. **Finalize the changelog.** Rename `## [Unreleased]` → `## [X.Y.Z] - YYYY-MM-DD`,
   then add a fresh empty `## [Unreleased]` (Added / Changed / Fixed) above it.
   The release notes come from this section, so its heading must match the tag.
4. **Stage everything the bump touched, then commit.** The version bump
   regenerates files beyond `Cargo.toml`: the `tests/contracts/VersionDto.json`
   snapshot and the mobile `Cargo.lock` both pick up the new version once the
   suite runs. Run `make preflight` again: its clean-tree step lists anything
   still unstaged. Then `git add -A` (stage ALL of it, never a hand-picked
   list), commit `Release vX.Y.Z`, and confirm a final `make preflight` is green
   with a clean tree.
5. **Tag & push:** push `main` first and let CI go green, then
   `git tag vX.Y.Z && git push origin vX.Y.Z`. Tagging fires the release workflow
   immediately (it is not gated on CI), so tag only after CI is green on the
   release commit. The workflow creates the GitHub Release and attaches the
   binaries.
6. **Publish to crates.io (manual):** `cargo publish` (the package stays lean via
   `Cargo.toml`'s `exclude`; `cargo publish --dry-run` first if unsure).
7. **Verify reach.** The `pages` workflow redeploys `alix.study` + the mdBook on
   the `main` push automatically — confirm the site, the download buttons, and
   `install.sh` resolve the new asset names.

## The mobile app (apps/mobile)

The Android app releases on its **own version stream**: pubspec `version:
X.Y.Z+N` and a **`mobile-vX.Y.Z` tag** (disjoint from the crate's `v*`
pattern, so neither track ever triggers the other). The `mobile-release`
workflow builds one signed arm64 APK (`alix-arm64-v8a.apk`) and attaches it
to a GitHub Release, notes lifted from `apps/mobile/CHANGELOG.md`'s matching
section.

Rules:

- **`+N` (the Android versionCode) is monotonic forever**: bump it by one
  every release, never reset it. Android orders installs by it, and a
  lower-or-equal N refuses to update.
- `X.Y.Z` follows the same pre-1.0 semver spirit as the crate, on the app's
  own clock. The About screen shows both the app and the embedded core
  version.

To cut one:

1. **Green gate:** `make check && make frb-check && make mobile-test &&
   make apk`, then install the APK on a real phone
   (`adb install -r apps/mobile/build/app/outputs/flutter-apk/app-release.apk`)
   and run one review.
2. **Finalize** `apps/mobile/CHANGELOG.md` (rename `## [Unreleased]` to
   `## [X.Y.Z] - date`, fresh empty Unreleased above) and bump pubspec's
   `version:` to `X.Y.Z+N`.
3. **Commit** as `Release mobile-vX.Y.Z`, then
   `git tag mobile-vX.Y.Z && git push origin main --tags`.
4. The workflow **fails loud** on: tag/pubspec mismatch, a debug-signed APK
   (missing secrets), non-16KB-aligned native libs, or a missing changelog
   section.

**One-time keystore setup** (the release signature; losing the keystore
orphans every installed app, so back it up privately):

```sh
keytool -genkeypair -v -keystore ~/keys/alix-release.jks -alias alix \
        -keyalg RSA -keysize 4096 -validity 10950
base64 -w0 ~/keys/alix-release.jks | gh secret set ANDROID_KEYSTORE_BASE64
gh secret set ANDROID_KEYSTORE_PASSWORD   # the -storepass you chose
gh secret set ANDROID_KEY_ALIAS --body alix
gh secret set ANDROID_KEY_PASSWORD        # the -keypass (often = storepass)
```

Local `make apk` builds are debug-signed while `android/key.properties` is
absent; that file (gitignored) can point at the keystore for local signed
builds if ever needed.

## After a release

Resync the local (gitignored) planning files: strike shipped items in `ROADMAP.md`
and update `PLAN.md` (re-audit, set the next milestone).
