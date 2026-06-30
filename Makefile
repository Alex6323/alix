# Unified dev commands for alix. Run `make <target>`.
#
# Formatting goes through the NIGHTLY toolchain on purpose: this repo's
# rustfmt.toml uses nightly-only options, so plain `cargo fmt` (stable) can't
# apply the config and reformats by different rules. (A cargo alias can't pick a
# toolchain — `+nightly` is handled by rustup before cargo sees it — which is
# why these live in a Makefile rather than .cargo/config.toml.)

.PHONY: build test lint lint-js fmt fmt-check check coverage eval run serve book site install clean heartbeat

# Compile the workspace.
build:
	cargo build

# Run the test suite — the primary gate.
test:
	cargo test

# Lint, including tests and examples.
lint:
	cargo clippy --all-targets

# Syntax-check the JS embedded in the served HTML assets (assets/serve/*.html).
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

# The gates that must stay green before work is done. (fmt is intentionally
# separate — formatting uses nightly and is run deliberately, not as a gate.)
check: lint test

# Test coverage (needs cargo-llvm-cov: `cargo install cargo-llvm-cov`). Prints a
# per-file summary and writes a browsable report to target/llvm-cov/html/. A
# flashlight for untested branches — especially the AI plumbing's error paths —
# not a gate to chase a number.
coverage:
	cargo llvm-cov --html
	@echo "HTML report -> target/llvm-cov/html/index.html"

# Grader-calibration evals (tests/eval.rs): the REAL grade prompt vs labeled
# adversarial answers, to catch a lenient grader. Needs the claude CLI logged in;
# makes real, costed calls. Off the normal gate — run before shipping a change
# to grade_prompt.
eval:
	cargo test --test eval -- --ignored --nocapture --test-threads=1

# Run the binary, e.g. `make run ARGS="exam mydeck.txt"`.
run:
	cargo run -- $(ARGS)

# Run the web frontend. With no ARGS, opens the in-browser deck picker;
# pass a session via ARGS, e.g. `make serve ARGS="review mydeck.txt --port 8080"`.
# `--serve` trails ARGS because it's a review/browse flag, not a global one.
serve:
	cargo run -- $(ARGS) --serve --port 7780

# Serve the user manual (docs/book) with live reload and open it in the browser.
# Requires mdBook: `cargo install mdbook`.
book:
	mdbook serve docs/book --open

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

# Release heartbeat: report whether shipped work has piled up unreleased (entries
# under CHANGELOG's [Unreleased] + days since the last vX.Y.Z tag) and flag when a
# release looks due. Informational; run at the start of a session (see CLAUDE.md).
# The policy it backstops is in RELEASING.md.
heartbeat:
	@sh scripts/heartbeat.sh

# Local, gitignored maintainer-only targets (e.g. wish-triage). The leading `-`
# makes this a silent no-op for anyone whose tree doesn't have the file.
-include docs/product/local.mk
