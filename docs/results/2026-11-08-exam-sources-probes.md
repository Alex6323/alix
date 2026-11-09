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
