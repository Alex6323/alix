# e2e fixtures

`decks/animals/` is a tiny, deterministic workspace served during the
Playwright run. It's shared across every `alix` web client the suite drives —
only the server config (`kids.toml`, and siblings as more clients get their
own spec) differs:

- `alix.toml` — makes the folder a workspace titled "Animals".
- `wild.txt` — a 2-card deck (tab-indented answers, `!` notes).
- `augment.json` — a **frozen** multiple-choice distractor cache, generated
  once with a real Claude call:

  ```sh
  alix deck augment e2e/fixtures/decks/animals/wild.txt --target choices \
    --store /tmp/alix-e2e-augment-seed/progress.json
  ```

  (point `--store` at a throwaway path — it's only there to satisfy the
  flag; `augment.json` always lands beside `wild.txt` regardless, and no
  progress store belongs in this fixture, see below). It's committed so the
  suite needs no AI backend and no network at test time — Recognize only
  renders tap-the-answer buttons when distractors are cached, and the
  headline test depends on that.

  The cache is keyed by each card's id (`XxHash64(deck file name + back
  lines)`), so it stays valid as long as `wild.txt`'s filename and answer
  (back) lines don't change. If you edit an answer line, or add/remove a
  card, regenerate `augment.json` with the command above and commit the new
  file.

- **No `progress.json` here, ever** (`.gitignore` blocks it under
  `decks/**/`, and `prepare-fixtures.cjs` refuses to copy one even if it
  somehow shows up locally). A progress store is per-run state, not a
  fixture: every run must start from a deck nobody has reviewed yet, so it
  actually exercises the never-seen (*acquire*) path — that's a real kid's
  first session, and it's the path a real shipped bug hid in (a card that
  skipped its attempt entirely). Committing a pre-warmed store would also be
  a ticking clock: `acquired_ms`/due-ness are computed against wall time, so
  a frozen timestamp changes the suite's behaviour as it ages.

`kids.toml` sets `[serve] audience = "kids"` so `/` serves the kids
frontend — one server config per client under test, all pointed at the same
decks fixture. Playwright's `globalSetup` (and, redundantly, the `webServer`
command itself — see `prepare-fixtures.cjs`) copies this whole
`fixtures/decks` folder into a scratch `e2e/.tmp/decks` before each run, so
the server's `progress.json`/`recent.json` writes never land in the repo.
