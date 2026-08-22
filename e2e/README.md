# e2e

A Playwright smoke suite that drives the real `alix` binary through a
browser, covering both web clients: the adult app (`web/alix/review.html`)
and the kids app (`web/alix-kids/kids.html`). `npx playwright test --list`
already prints what each test checks (the names are full sentences), and the
project instructions require the two docs (book + README) to describe
*behavior*, not this suite — so this file only covers what the test list
can't say for itself.

## Why this exists

Three real bugs shipped past unit tests, code review, and the server's own
contract suite, and were only ever found by a human clicking:

1. A button POSTed a *workspace* name to `/api/select` (the server rejects
   that, 400) — and because the client's `api()` helper had no `.catch`, the
   button silently did nothing.
2. A never-seen (*introduction*) card skipped its attempt entirely, so the depth a
   learner chose changed nothing.
3. A multi-line answer got flattened into one run-on string
   (`back.join(" ")`), turning an ordered sequence ("Egg / Caterpillar /
   Chrysalis / Butterfly") into nonsense.

All three are the same shape: something *looked* fine — a screen rendered, no
exception was thrown where anyone was looking — while the control was
actually dead or wrong. Every test here asserts the full chain (a click
causes the expected request, the expected response, and the expected
resulting screen — never just the screen), and a shared auto-fixture
(`tests/helpers.ts`'s `pageErrors`) fails any test that logs an uncaught
`pageerror` or `console.error`, so a silently-swallowed failure has nowhere
to hide.

## Running it

```sh
make e2e
```

This installs the pinned npm deps (`npm ci` — see "Dependency hygiene"
below), installs the Chromium browser Playwright needs, then runs
`playwright test`. Playwright starts both servers itself (kids on `:7788`,
adult on `:7789`, each over its own scratch decks copy — see "The fixture
contract"), waits for them to answer, runs the suite, and tears them down.
Nothing needs to be started by hand, and nothing should be — don't
background-launch either server yourself; let Playwright's `webServer` own
their lifecycle.

Output:

- Console output is the `list` reporter (pass/fail per test, printed as it
  runs).
- `e2e/test-results/` holds traces for any failing test
  (`trace: "retain-on-failure"`) — open one with `npx playwright show-trace
  e2e/test-results/<test>/trace.zip`.
- Both are pinned under `e2e/` explicitly in `playwright.config.ts`
  (`outputDir`), so even an ad-hoc `npx playwright test --config=e2e/
  playwright.config.ts` run from the repo root can't scatter artifacts
  outside this directory.

## The boundary — what this suite does NOT cover, and why

This is a smoke suite, not exhaustive coverage. Known gaps, and why each is
where it is rather than fixed:

- **The honest-Recognize-grading rule** ("a wrong Recognize pick can only
  record failed, never passed", fix `c46dad5`) has a real test body, but it's
  marked `test.fixme` in `tests/kids-review.spec.ts` (see the annotation
  there) — reaching it needs a card that is past introduction *and* due again, and
  the real introduction cooldown is a server-side gap (5 min default; configurable
  since 2026-07-14 via `[review] introduction_cooldown`, `"0"` = none — a fixture
  config with a zero cooldown is now the cleanest route to close this gap).
  Reaching that deterministically without the knob
  would mean either a real wait (this suite avoids real-time waits — the
  fixed exception is the adult grading test's pre-seeded card, see below,
  which sidesteps the wait rather than taking it) or committing pre-warmed
  progress state, which the fixture contract forbids. The `test.fixme`
  reports the gap on every run instead of it rotting in this paragraph.
- **Ask Alix's real model call** is never made. Adult tutor interaction is
  exercised with a deterministic intercepted `AskDto`, including its
  unsaved-conversation guard and return to the originating card; the kids
  tutor and an actual AI round trip remain uncovered because they would make
  the suite slow, non-deterministic, and backend-dependent.
- **Done/restart.** Neither client's "review again" control is clicked. The
  adult grading test does reach the summary screen (`.summary`) as a
  side effect of proving a grade ends the session, but stops there.
- **Themes** (the settings-menu colour swatches, both clients) are never
  opened or applied.
- **`prefers-reduced-motion`** behavior is never exercised (no test emulates
  that media feature).
- **The pairing-token gate** (a 401 under `alix --lan`) never fires — neither
  fixture server runs with `--lan`, so no token is required.
- Anything AI-backed beyond the tutor (the exam, `augment`, `generate`) is
  adult-only and out of scope for kids v1 regardless, and isn't driven here
  either, for the same non-determinism reason as Ask Alix.

## The fixture contract

- `fixtures/decks/animals/` is a tiny, deterministic workspace ("Animals"),
  shared by both clients:
    * `alix.toml` — makes the folder a workspace.
    * `decks/wild.md`: an initialized 2-card L1 deck (a frontmatter `id`,
      `## ` fronts with literal `<!-- id: -->` tokens, and `> ` notes). Both
      cards are always genuinely never-seen at the start of a run, so the
      suite exercises a real first session, introduction path included.
    * `decks/cats.md`: one card with a two-line answer ("Lion" / "Tiger"), in
      its own file so editing it can never disturb `wild.md`. Exists solely
      for the multi-line regression test (`tests/kids-multiline.spec.ts`).
    * `decks/fronts.md`, `decks/math.md`, `decks/source-fact.md`,
      `decks/source-stale.md`, `decks/trace-inline.md`, `decks/sectioned.md`,
      `decks/section-context-pill.md`, and `decks/gated.md`: one deck per
      regression surface — front rendering, LaTeX math (`adult-math.spec.ts`,
      `kids-math.spec.ts`), live and stale source excerpts
      (`adult-source-excerpt.spec.ts`), the inline trace walk
      (`adult-walk-inline.spec.ts`), section context (`kids-review.spec.ts`),
      the adult context-pill layout, and the sub-card graduation lock
      (`adult-review.spec.ts`).
    * `source-fact.rs`, `source-stale.rs`, and `trace-source.txt`: the live
      source files those decks cite; `source-stale.rs` deliberately drifts
      from its deck's frozen excerpt.
    * `assets/deck-…/`: the frozen, content-addressed excerpt objects for the
      two source-citing decks, so the walk and excerpt views work offline.
    * `augment/deck-00000000000000000000000008.json` (and its sibling
      `…07.json` for `math.md`): the **frozen**, deck-owned multiple-choice
      distractor cache for `wild.md`, generated once with a real Claude call:

      ```sh
      alix deck augment e2e/fixtures/decks/animals/decks/wild.md \
        --target choices --store e2e/fixtures/decks/animals
      ```

      It's committed so the suite needs no AI backend and no network at test
      time — Recognize only renders tap-the-answer buttons when distractors
      are cached. The cache is keyed by each card's literal `<!-- id: -->`
      token, so it stays valid as long as those tokens in `decks/wild.md`
      don't change; if you change a token, or add/remove a card in that file,
      regenerate the matching augmentation document with the command above
      and commit it.
- `fixtures/kids.toml` / `fixtures/adult.toml` each set `[serve] audience`
  explicitly (`"kids"` / `"adult"`) — one server config per client, both
  pointed at their own copy of the same decks fixture. `--config` is always
  passed explicitly (never omitted): without it, `alix` would read the
  developer's real platform config — their real decks dir and AI backend.
- **No `progress/` or `recent.json` under `fixtures/`, ever.**
  `.gitignore` blocks progress documents under `fixtures/decks/**/`, and
  `prepare-fixtures.cjs` refuses to copy private state even if it somehow
  shows up locally. Progress is per-run state, not a
  fixture: every run must start from a deck nobody has reviewed yet, so it
  actually exercises the never-seen (*introduction*) path. Committing a pre-warmed
  store would also be a ticking clock: due-ness is computed against wall
  time, so a frozen timestamp would change the suite's behavior as it ages.
- **No exceptions, and no synthesising progress at setup either.** Card and
  deck IDs are authored by Rust and remain the only persistence keys; browser
  fixture code must not recreate or guess them.
- Playwright's `globalSetup` (and, redundantly, each server's own
  `webServer` command — see `prepare-fixtures.cjs`) copies the whole
  `fixtures/decks` folder into a scratch `.tmp/<kids|adult>/decks` before
  each run, so the servers' `progress/` and `recent.json` writes never land
  in the repo, and the two clients' stores never collide.

## Dependency hygiene

- `make e2e` runs `npm --prefix e2e ci`, not `npm install` — a lockfile
  mismatch fails loudly instead of silently resolving fresh versions and
  rewriting the lockfile. Only the browser-download step
  (`playwright install --with-deps`) falls back to a plain install, and only
  for the browser's *system* dependencies — an environment concern, not a
  package-resolution one.
- `@playwright/test` is pinned to a patch-level range
  (see `package.json`) matching what's actually in `package-lock.json`.
  Bumping it is a deliberate, reviewed action: change the range, run
  `npm --prefix e2e install` once, and commit the regenerated lockfile.

## Adding a test

Mutation-test it before trusting it: temporarily reintroduce the bug the
test is meant to catch, run `make e2e` and confirm the new test actually
fails, then restore the code (`git checkout -- <file>`) and confirm it's
green again. A test that can't fail isn't coverage — it's decoration. Every
test added for this suite (see the git history for `e2e/`) was checked this
way; do the same for the next one.
