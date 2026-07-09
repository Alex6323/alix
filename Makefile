# Unified dev commands for alix. Run `make <target>`.
#
# Formatting goes through the NIGHTLY toolchain on purpose: this repo's
# rustfmt.toml uses nightly-only options, so plain `cargo fmt` (stable) can't
# apply the config and reformats by different rules. (A cargo alias can't pick a
# toolchain — `+nightly` is handled by rustup before cargo sees it — which is
# why these live in a Makefile rather than .cargo/config.toml.)

.PHONY: build test lint lint-js fmt fmt-check fmt-roadmap check ci coverage coverage-lcov eval run serve book site slides install clean sdd-clean heartbeat check-backends

# Compile the workspace.
build:
	cargo build

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

# Grader-calibration evals (tests/eval.rs): the REAL grade prompt vs labeled
# adversarial answers, to catch a lenient grader. Needs the claude CLI logged in;
# makes real, costed calls. Off the normal gate — run before shipping a change
# to grade_prompt.
eval:
	cargo test --test eval -- --ignored --nocapture --test-threads=1

# Run the binary, e.g. `make run ARGS="exam mydeck.txt"`.
run:
	cargo run -- $(ARGS)

# Run the web frontend: the in-browser deck picker. ARGS may name a decks
# folder or workspace to serve as a scoped root, plus launcher flags,
# e.g. `make serve ARGS="~/decks-test --lan"`.
serve:
	cargo run -- $(ARGS) --port 7780

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

# Local, gitignored maintainer-only targets (e.g. wish-triage). The leading `-`
# makes this a silent no-op for anyone whose tree doesn't have the file.
-include docs/product/local.mk
