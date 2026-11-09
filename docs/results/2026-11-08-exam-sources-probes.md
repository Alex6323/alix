# Exam sources: probe results

Recorded 2026-11-09 from the repo build (`cargo test --release`, a temporary
ignored test calling `ask::run` with the production argv and spawn path).
CLI versions: codex-cli 0.167.0, GitHub Copilot CLI 1.0.107. Gemini and
Claude were not run here.

## 0a. Prompt on stdin, one-line question

Assumption: Codex (`exec ... -`) and Copilot (no `-p`) answer a prompt
delivered on stdin.

Falsifier: `ask::run` with `Reply with exactly: OK`, once per backend.

Result: Codex answered `OK`. Copilot answered `OK`.

## 0a. Prompt on stdin, 200 KiB prompt

Assumption: a 200 KiB prompt starts on both backends (it failed with `E2BIG`
as an argv string).

Falsifier: 204800 bytes of `a`, then ` Reply with exactly: OK`.

Result: Copilot answered `OK`. Codex started (no spawn error) but gave no
answer within the 180 s timeout; the filler is one repeated character, so
this is not evidence of a limit and is left to the per-backend limit probe.
The offline law test (`every_backend_delivers_the_prompt_on_stdin_and_a_200_kib_prompt_starts`)
covers delivery itself.

## 0a. Copilot read confinement through `view`

Assumption: with `--available-tools view --allow-tool view
--disallow-temp-dir` and the working folder set to a root, Copilot reads a
file inside the root and is refused outside paths.

Falsifier: a root holding `inside.txt`; beside it `outside.txt`; inside the
root a symlink `link.txt` to `../outside.txt`. Prompt: read the named file
with the view tool and reply with its exact content, else `REFUSED`. Run
through `ask::run` with a files grant and the root as the working folder.

Result: `inside.txt` returned its content. `../outside.txt` returned
`REFUSED`. `link.txt` (symlink to the outside file) returned `REFUSED`.

## 0b. Per-backend prompt limit

Recorded 2026-11-09 through the production spawn path (`ask::spawn`, the
same supervised run as `ask::run`), one call at a time, each backend on the
default model alix selects; failures re-run directly with alix's argv to
read the cause. 108 calls in all.

Assumption A1: each backend's threshold is a count the lib can compute
offline over the prompt text.

Falsifier: `<N bytes of filler>` plus `Reply with exactly: OK`, N doubled
from 64 KiB to the first failure, then bisected to 4 KiB, once with an
ASCII filler and once with a 3-byte UTF-8 filler.

| Backend | Model, CLI | ASCII last OK / first fail (bytes) | UTF-8 last OK / first fail (bytes) | Cause | Measure |
| --- | --- | --- | --- | --- | --- |
| Codex | gpt-5.6-sol, codex-cli 0.167.0 | 1,044,480 / 1,048,576 | 3,141,632 / 3,145,728 | client-side `input_too_large`, `max_chars` 1048576 | characters |
| Copilot | claude-haiku-4.5, CLI 1.0.107 | 761,856 / 765,952 | 315,392 / 319,488 | "exceeds the model's prompt budget", limit 128,000 tokens | tokens |
| Claude | claude-fable-5-1, CLI 2.1.319 | 2,510,848 / 2,513,920 (Rust source) | unmeasured | "Prompt is too long", ~1,000,707 tokens against a limit of 1,000,000 | tokens |
| Gemini | CLI 0.67.0 | blocked | blocked | argv rejected: "Not enough arguments following: p" | none |

Result:

- A1 holds for Codex: a character count, exact and offline.
- A1 fails for Copilot and Claude: their counts exist only in the error
  after a request. The fallback applies (bytes divided by the worst
  observed bytes-per-token ratio): Copilot 2.53 on CJK text plus about
  2,200 tokens of fixed overhead; Claude 2.51 on Rust source with the
  system prompt included.
- Claude's first byte boundaries (1,486,848 ASCII, 561,152 UTF-8) were
  safety-classifier refusals of the synthetic filler, not length limits.
  The UTF-8 series with real text mapped to CJK characters was refused the
  same way at its first call, so Claude's UTF-8 threshold is unmeasured.
- Through alix, a Claude failure shows only unrelated stderr warnings, and
  a Codex failure keeps only the first 300 characters of stderr, cutting the
  cause. Named as `{#backend-failure-cause-hidden}`.
- Codex is refused in a working folder with no `.git` ancestor because
  alix's argv omits `--skip-git-repo-check`; the probe ran from a folder
  set up with `git init`. Named as `{#codex-needs-skip-git-repo-check}`.
- Gemini's bare `-p` predates this arc. Named as `{#gemini-bare-p-rejected}`.
- The constants live in `src/exam_prompt.rs`: `prompt_limit` holds each
  backend's `PromptLimit` (Gemini's arm shares Copilot's), and `Measure::of`
  does the counting. Copilot's ratio is coded as 2.50, not the observed
  2.53: at 2.53 the estimate admits up to 318,274 bytes, inside this
  probe's UTF-8 uncertainty band (last OK 315,392, first fail 319,488); at
  2.50 the admit edge is 314,500 bytes, below the last OK size. Claude's
  edge (2,510,000 bytes at 2.51) is already below its last OK.

## 0c. Tutor browse versus paste

Recorded 2026-11-09 through `ask::run` (the tutor's spawn path), one call at
a time, 300 calls: 270 tutor answers and 30 judge calls, none failed. Claude
Code 2.1.319 (claude-fable-5-1), codex-cli 0.167.0 (gpt-5.6-sol).

Assumption A2: on Claude, browsing `sources/<deck-id>/` with the manifest as
a map answers card questions at least as well as pasting the card's topic's
sources; on Codex, pasting answers well enough to ship.

Falsifier: `src/tutor_probes.rs`, the ignored test
`the_tutor_answers_as_well_browsing_frozen_sources_as_with_them_pasted`
(env `ALIX_TUTOR_EVAL_DECKS`): three real decks frozen into a scratch
workspace (prose book chapters, 158 KB; Rust source, 209 KB; Rust, Go and
Swift source, 405 KB), ten cards each, the same three questions per card in
every arm, web tools stripped. One Claude judge call per card sees the
topic's sources, the card and the nine answers shuffled without arm labels,
and marks each grounded, specific and correct; pass is all three.

Expected: Claude browse >= Claude paste minus 5 points; Codex paste >= 80% of
Claude browse.

| Backend | Condition | Passed | Rate |
| --- | --- | --- | --- |
| Claude | Browse | 90 / 90 | 100.0% |
| Claude | Paste | 88 / 90 | 97.8% |
| Codex | Paste | 74 / 90 | 82.2% |

Result: both expectations held; A2 holds as far as one run reaches. The
Codex margin is two answers (72 / 90 was the line). Caveats: a single run,
Claude judges Claude, fixed template questions.

Side findings:

- 11 of Codex's 16 misses answer the counterfactual question from general
  knowledge, judged not grounded.
- Browse answers cite the frozen copy name (`source-<52>.md:137`) in 42 of 90
  answers and never the display path: the browse prompt should ask for the
  display path.
- Browse answers exceed the tutor's six-sentence cap in 33 of 90 (Paste 18,
  Codex 0).
- 60 of 90 Codex answers end with a line the operator's Codex memories ask
  for: alix's Codex argv does not isolate the call from the operator's Codex
  memories. Named as `{#codex-memories-leak}`.
- The 405 KB deck's deck-wide topics measure about 172,000 estimated tokens,
  over Copilot's and Gemini's 128,000 limit; the two largest real `exam:`
  lists (7.2 MB and 5.8 MB) exceed every backend's limit, which bears on 0d.

## 0d. A real deck's largest topic against the limits

Recorded 2026-11-09 with the arc-B build's `alix deck source list`, no model
calls. The three largest of 147 decks whose `exam:` lists name files were
copied into a scratch workspace, their files frozen with `alix deck source
add` and bound to every topic (each list was deck-wide).

Assumption A3: a typical topic's full prompt fits the smallest backend limit.

Falsifier: the per-topic measure `source list` prints, per backend.

| Deck (sources) | Claude (est. tokens) | Codex (characters) | Copilot, Gemini (est. tokens) |
| --- | --- | --- | --- |
| PR review, 7.17 MB, 5 topics | 2,866,902 / 1,000,000 | 7,188,250 / 1,048,576 | 2,880,569 / 128,000 |
| PR review, 5.83 MB, 11 topics | 2,331,028 / 1,000,000 | 5,846,381 / 1,048,576 | 2,342,552 / 128,000 |
| PR review, 2.24 MB, 12 topics | 900,793 / 1,000,000 | 2,257,094 / 1,048,576 | 906,596 / 128,000 |

Result: the falsifier fired. 100 of 112 topic-backend pairs exceed their
limit; only the 2.24 MB deck fits, and only on Claude. A byte estimate over
the 147 file-naming decks finds 2 over Claude's limit, 5 over Codex's and 19
over Copilot's and Gemini's; these are lower bounds, because 347 more decks
name whole directories, which were not sized. Ruled 2026-11-09: arc B merges
with over-limit exams refused at preflight; chunked generation
(`{#chunked-topic-generation}`) is the next arc.
