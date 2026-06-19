# flash — project guide

`flash` is an **AI-augmented** spaced-repetition learning tool in Rust, with a
terminal (TUI) and a web frontend (`flash serve`). On top of a plain-text
flashcard core, Claude is woven in: an ask-Claude tutor on any card, AI deck
generation (`flash generate`), and the **AI exam** (`flash exam`) that gates
progression on verified understanding. The tool is increasingly AI-centric —
weight that when prioritizing. The **library crate is the single source of
logic**; the TUI, the web server, and the CLI are thin consumers. Put behavior
in the lib (`src/`), not in a frontend, so both surfaces share it.

## Dev commands — use the Makefile

| Command | What it does |
| --- | --- |
| `make build` | Compile. |
| `make test` | Run the test suite (the primary gate). |
| `make lint` | `cargo clippy --all-targets`. |
| `make fmt` | Format — **nightly** rustfmt (see below). |
| `make fmt-check` | Verify formatting without writing. |
| `make check` | `lint` + `test` — run before considering work done. |
| `make run ARGS="exam mydeck.txt"` | Run the binary with args. |

## Formatting is nightly-only

`rustfmt.toml` uses nightly-only options, so **formatting must go through the
nightly toolchain** (`make fmt` → `cargo +nightly fmt`). Do **not** run plain
`cargo fmt` (stable): it can't apply the config and reformats by different rules,
producing a large bogus diff. The tree also has some pre-existing rustfmt drift,
so don't reformat unrelated files as part of a change — keep your diff to what
you touched.

## Conventions

- **Tests and clippy must be green** before a change is done (`make check`).
  Formatting is run deliberately with `make fmt`, not enforced as a gate.
- Don't commit unless asked; never push without permission.
- The deck format, every `%`/card directive, and all features are documented in
  `README.md` (start with the "Directives at a glance" table). Keep it in sync
  when you add a directive or feature.
- Roadmap and design rationale live in `ROADMAP.md` (gitignored, local).
