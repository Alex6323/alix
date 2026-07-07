# 16 · Configuration

`alix` works out of the box; the config file is for when you want to change key
bindings, point at a different decks directory, or tune the AI features. It lives
at `~/.config/alix/config.toml` — create it with `alix config --init`, and
inspect the active key bindings with `alix config`.

## Key bindings

All keybindings live under `[keys]`, one subtable per surface: `[keys.review]`
(the review screen), `[keys.picker]` (the deck picker), and `[keys.browse]`
(the browse overlay). Every action takes a list of keys (the first is shown in the
footer). To grade self-graded cards with `j`/`k`/`l`:

```toml
[keys.review]
failed = ["j"]
partly = ["k"]
passed = ["l"]
```

Keys are a single character (`"j"`), a special name (`"space"`, `"enter"`, `"tab"`,
`"esc"`, `"backspace"`), or either with a `ctrl-` prefix (`"ctrl-s"`). The
rebindable `[keys.review]` actions are `failed`, `partly`, `passed`, `reveal`, `hint`, `submit`, `skip`,
`remove` (default `ctrl-x`), `continue`, `restart` (default `r`), and `quit`. While
you're typing an answer (a reconstruct check), plain-character bindings are ignored so
they can't shadow your input — use `ctrl-`/special keys for `hint`, `skip`, and
`quit` there. Pass a different file with `--config <path>`.

The picker's navigation is `[keys.picker]` (`up`, `down`, `open`, `back`,
`filter`, `mastered`, plus `depth` to open the depth menu and
`recognize`/`recall`/`reconstruct` to pick within it — defaults `v` and
`1`/`2`/`3`), the browse overlay has its own `[keys.browse]` bindings, and
the web server reads its default port from `[serve]`:

```toml
[keys.browse]
next = ["l", "n", "space"]
prev = ["h", "p"]
remove = ["x"]
quit = ["q", "esc", "ctrl-c"]

[serve]
port = 7777
```

(Jump-to-first/last stays fixed at `g`/`G`, and the arrow keys always move.)

## Review pacing

The `[review]` section tunes the FSRS scheduler shared by the Recall and
Reconstruct depths:

```toml
[review]
retention = 0.9         # FSRS target recall probability (0.70–0.99); higher = shorter intervals
retire_after = "1y"     # a card rests once its Recall interval reaches this ("2w", "6m", "30d", or "never")
```

`retention` is the recall probability FSRS schedules for. `retire_after` is when
a card retires (rests until `alix reset`); `"never"` keeps it in rotation forever.
A workspace can override either key for its own decks in an `alix.local.toml` —
see [Workspaces](08-workspaces.md).

How deeply you drill is never configuration: it's the **session depth** you
pick per review (`--depth`, or the picker's Learn ▾ menu) — see
[Reveal & session depths](04-review-modes.md). The old `[review] depth` config
key (and the per-deck `[review.deck."<file>"]` override) — a *dial* that fixed
the drilling depth from config — is gone, not renamed; a config that still
sets either now fails to load.

## Backends

By default all AI calls go through the [Claude Code](https://www.anthropic.com/claude-code)
CLI. You can switch to one of the other supported CLIs with `backend` in `[ask]`:

```toml
[ask]
backend = "claude"   # default — Claude Code CLI
# backend = "gemini"  # Google Gemini CLI
# backend = "codex"   # OpenAI Codex CLI
# backend = "copilot" # GitHub Copilot CLI
```

Auth is each CLI's own login — alix stores no API keys. Install whichever CLI
you want to use and run its login command once.

Each backend is granted **read-only tools only** (file reading; web fetch where
the backend supports it). Codex runs under a network-blocking sandbox rather
than a tool allowlist, so it can read local source files but can't fetch URLs
— a URL-based exam or `deck generate` will refuse and tell you to use a local
file instead, or switch backends.

Run `alix backend check` to send a quick test request to the configured
backend and confirm it's installed, signed in, and responding. `--all` probes
all four.

The multi-turn tutor works on every backend: Claude uses its native session
flags (`--session-id` / `--resume`) for efficient continuation; other backends
re-inline the accumulated Q&A transcript into each follow-up so the context
carries over (the prompt grows with the conversation rather than being resumed
efficiently).

## The AI sections

Each AI feature has its own section, all reusing the `[ask]` command and permission
settings:

- **`[ask]`** — the tutor: `command` (how to invoke the CLI), `backend`,
  `permission_mode`, the tool allowlist, a `model` override, `timeout_secs`,
  and an `effort`.
- **`[generate]`** — `alix deck`: `model`, `timeout_secs` (300), `max_cards` (30),
  `extra`, a full `prompt` override, and `review`.
- **`[exam]`** — the AI exam: `model`, `timeout_secs` (300), `num_questions` (5),
  `pass_threshold` (1.0), `strictness` (`balanced`), `extra`.
- **`[trace]`** — `alix trace --build` / `--suggest`: defaults `model = "opus"`
  and `effort = "high"` (the build is correctness-critical and amortized); also
  `timeout_secs`. `--grade` instead uses the `[ask]` tier.

## Decks directory and storage

By default `alix` looks for decks in `~/decks`; set `decks_dir` to change it.
Progress is stored at `~/.local/share/alix/progress.json` (a workspace keeps its
own inside its folder; `--store <path>` overrides).

Card identity is an XxHash64 over the deck **file name** plus the card's **back
lines** — so your progress survives editing a front or adding notes, but renaming a
deck file or changing a back line resets the affected cards. The hash is
**whitespace-insensitive**: it depends on the back lines' words, not their line
breaks, indentation, or repeated spaces, so reflowing or reindenting an answer
keeps a card's history. (That's the "editing is safe" rule from
[chapter 3](03-the-deck-format.md), stated precisely.)

`alix reset <deck>` clears progress so cards go "new" again — a whole deck, a
single card (`--card <id-or-front>`), or the entire store (`--all`); it confirms
first unless you pass `-y`.
