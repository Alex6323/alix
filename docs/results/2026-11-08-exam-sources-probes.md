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
  does the counting.
