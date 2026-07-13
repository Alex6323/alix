# Unified dev commands for alix. Run `make <target>`.
#
# Formatting goes through the NIGHTLY toolchain on purpose: this repo's
# rustfmt.toml uses nightly-only options, so plain `cargo fmt` (stable) can't
# apply the config and reformats by different rules. (A cargo alias can't pick a
# toolchain — `+nightly` is handled by rustup before cargo sees it — which is
# why these live in a Makefile rather than .cargo/config.toml.)

.PHONY: build build-core test lint lint-js fmt fmt-check fmt-roadmap roadmap check ci coverage coverage-lcov calibrate run web phone tablet desktop frb-check push-decks book site slides install clean sdd-clean heartbeat check-backends e2e shots

# Compile the workspace.
build:
	cargo build

# Compile the lean core only: no AI backends, no web server. This is the guard
# that keeps the lib buildable AI/server-free for the future mobile client (see
# CONTRIBUTING.md).
build-core:
	cargo build --no-default-features --lib

# Run the test suite — the primary gate.
test:
	cargo test

# Lint, including tests and examples.
lint:
	cargo clippy --all-targets

# Syntax-check the JS embedded in the served HTML assets (assets/web/*.html).
# That JS is shipped as static strings, so cargo never parses it — this catches a
# syntax error the Rust gates can't see. Needs node; a no-op (not a failure) when
# node isn't installed, so it never blocks a cargo-only contributor or CI. Not
# wired into `check` for that reason — run it deliberately, like fmt.
lint-js:
	@if command -v node >/dev/null 2>&1; then \
		node scripts/lint-js.js; \
	else \
		echo "lint-js: node not found — skipping JS asset check"; \
	fi

# Format with the nightly toolchain (NOT stable `cargo fmt`).
fmt:
	cargo +nightly fmt

# Verify formatting without writing.
fmt-check:
	cargo +nightly fmt --check

# Wrap over-long lines in ROADMAP.md (or files passed via ARGS) onto the
# roadmap's 13-space continuation indent. Wrap-only: lines already within
# width are never touched, so hand-made breaks don't churn. Stdlib python3.
fmt-roadmap:
	python3 scripts/fmt-roadmap.py $(ARGS)

# Roadmap stats, read-only: items by state (done/partial/open) and the open
# items split by priority. The deterministic half of a roadmap audit; whether
# an "open" item is secretly already shipped still needs a reader (see
# CLAUDE.md's roadmap-drift note). Run from a checkout that has ROADMAP.md.
roadmap:
	@python3 scripts/fmt-roadmap.py --stats $(ARGS)

# The gates that must stay green before work is done. (fmt is intentionally
# separate — formatting uses nightly and is run deliberately, not as a gate.)
check: lint test

# Full CI parity, run the way GitHub does (see .github/workflows/ci.yml): the
# nightly fmt check, then clippy + tests under `-Dwarnings`, then coverage with
# the warnings gate cleared (coverage instruments its own flags). A green
# `make ci` predicts a green CI — so this is the pre-push / pre-release gate.
# It's heavier than `make check` (adds nightly fmt + a full coverage run, and the
# -Dwarnings/coverage flag split forces a recompile between steps), which is why
# `make check` stays the fast, lenient inner-loop gate rather than carrying these.
ci:
	$(MAKE) fmt-check
	RUSTFLAGS="-Dwarnings" $(MAKE) check
	RUSTFLAGS="-Dwarnings" $(MAKE) build-core
	RUSTFLAGS= $(MAKE) coverage

# Test coverage (needs cargo-llvm-cov: `cargo install cargo-llvm-cov`). Prints a
# per-file summary and writes a browsable report to target/llvm-cov/html/. A
# flashlight for untested branches — especially the AI plumbing's error paths —
# not a gate to chase a number. Nightly toolchain (matching coverage-lcov
# below): a few genuinely-untestable arms are marked
# `#[cfg_attr(coverage_nightly, coverage(off))]` (see `src/lib.rs`), which only
# takes effect under nightly — running this on stable would silently count
# those lines as missed instead of excluded.
coverage:
	cargo +nightly llvm-cov --workspace --html
	@echo "HTML report -> target/llvm-cov/html/index.html"

# Coverage in lcov format for the Codecov upload (see .github/workflows/ci.yml
# and codecov.yml). Same nightly toolchain as `coverage` above. Writes
# lcov.info at the repo root (gitignored).
coverage-lcov:
	cargo +nightly llvm-cov --workspace --lcov --output-path lcov.info

# Grader calibration (tests/calibrate.rs): the REAL grade prompt vs labeled
# adversarial answers, to catch a lenient grader. Needs the claude CLI logged in;
# makes real, costed calls. Off the normal gate: run before shipping a change
# to grade_prompt.
calibrate:
	cargo test --test calibrate -- --ignored --nocapture --test-threads=1

# Run the binary, e.g. `make run ARGS="stats mydeck.txt"`.
run:
	cargo run -- $(ARGS)

# Run the web frontend: the in-browser deck picker. ARGS may name a decks
# folder or workspace to serve as a scoped root, plus launcher flags,
# e.g. `make web ARGS="~/decks-test --lan"`.
web:
	cargo run -- $(ARGS) --port 7780

# The mobile siblings of `web`: run the alix mobile app (apps/mobile) on a
# phone or tablet emulator (booting that AVD first if needed; the script
# resolves AVDs by name, so both can run side by side), or as a native Linux
# desktop window (fastest loop: hot reload, no emulator). flutter compiles the
# embedded Rust core through cargokit either way. Needs the frb toolchain and,
# for the emulators, ANDROID_HOME (see docs/dev/frb-bridge-setup.md).
phone:
	@sh scripts/mobile-run.sh alix_phone
tablet:
	@sh scripts/mobile-run.sh alix_tablet
desktop:
	cd apps/mobile && flutter run -d linux

# Assert the frb toolchain-alignment invariants (codegen/Dart/Rust version
# pins, the two template patches, the NDK the build uses) and fail on drift.
# Cheap and local; the mobile CI runs it before building.
frb-check:
	@sh scripts/frb-check.sh

# One-way copy of a host decks folder into the running emulator's alix app
# (dev-only, debug build). Restart the app to re-list. Progress made on the
# emulator never syncs back.
push-decks:
	@sh scripts/push-decks.sh $(DIR)

# Serve the user manual (docs/book) with live reload and open it in the browser.
# Requires mdBook: `cargo install mdbook`.
book:
	mdbook serve docs/book --open

# Preview the presentation slides (site/slides.html) locally. Same server as
# `make site`, pointed at the slides URL.
slides:
	@echo "Slides -> http://localhost:8000/slides.html  (Ctrl-C to stop)"
	python3 -m http.server -b 127.0.0.1 -d site 8000

# Serve the alix.study landing page locally for a quick preview (static files from
# site/). Needs python3. The /book/ link only resolves on the deployed Pages
# site — use `make book` to preview the manual itself.
site:
	@echo "Landing page -> http://localhost:8000  (Ctrl-C to stop)"
	python3 -m http.server -b 127.0.0.1 -d site 8000

# Install `alix` from this checkout.
install:
	cargo install --path .

# Remove build artifacts.
clean:
	cargo clean

# Remove spent subagent-driven-development scratch (task briefs/reports, review
# packages, the ledger) but keep the dir + its .gitignore. The artifacts are
# gitignored and recoverable from git log, so this is safe to run anytime — it's
# how an SDD run cleans up after its branch lands (see CLAUDE.md).
sdd-clean:
	@find .superpowers/sdd -maxdepth 1 -type f ! -name .gitignore -delete 2>/dev/null; \
		echo "cleaned .superpowers/sdd/ (kept the dir + .gitignore)"

# Release heartbeat: report whether shipped work has piled up unreleased (entries
# under CHANGELOG's [Unreleased] + days since the last vX.Y.Z tag) and flag when a
# release looks due. Informational; run at the start of a session (see CLAUDE.md).
# The policy it backstops is in RELEASING.md.
heartbeat:
	@sh scripts/heartbeat.sh

# Probe all four backends end-to-end (real tiny request through each installed
# CLI). Needs each CLI installed and the maintainer's own logins configured.
check-backends:
	cargo run --quiet -- doctor --all-backends

# Playwright end-to-end smoke suite for the alix web clients (e2e/): drives
# real `alix` servers (Chromium only) and asserts a click reaches the server —
# request, response, and screen, with zero uncaught page errors. Deliberately
# NOT part of `check` (needs Node + a browser download, and is slower) — run
# it deliberately, like `calibrate`. See e2e/README.md.
e2e:
	npm --prefix e2e ci
	npx --prefix e2e playwright install --with-deps chromium || npx --prefix e2e playwright install chromium
	npx --prefix e2e playwright test --config=e2e/playwright.config.ts

# Regenerate the landing-page carousel screenshots (site/img/shot-*.png) from
# a fresh copy of ~/alix-demo and ~/alix-kids — never the originals directly
# (see e2e/shots/capture.cjs's own header). Makes real Claude calls (deck
# augmentation, the tutor, the exam): a few minutes, and needs the `claude`
# CLI logged in. Not part of `make e2e`/CI — run it deliberately, around a
# release, when the app's look has changed enough that the carousel is
# stale. `make shots ONLY=6` re-shoots one slide (comma-separated for
# several); ARGS="--fresh" forces a clean re-copy of the demo/kids dirs.
shots:
	npm --prefix e2e ci
	npx --prefix e2e playwright install chromium
	node e2e/shots/capture.cjs $(if $(ONLY),--only=$(ONLY)) $(ARGS)

# Local, gitignored maintainer-only targets (e.g. wish-triage). The leading `-`
# makes this a silent no-op for anyone whose tree doesn't have the file.
-include docs/product/local.mk
