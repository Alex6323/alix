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
`filter`, `mastered`, plus `depth` to open the depth menu,
`recognize`/`recall`/`reconstruct` to pick within it, and `cram` to toggle its
tick-box — defaults `v`, `1`/`2`/`3`, and `c`), the browse overlay has its own `[keys.browse]` bindings, and
the web server reads its default port from `[serve]`:

```toml
[keys.browse]
next = ["l", "n", "space"]
prev = ["h", "p"]
remove = ["x"]
quit = ["q", "esc", "ctrl-c"]

[serve]
port = 7777
# token = "..."   # pairing token required on /api/*; --lan auto-generates one (printed, with a QR)
```

(Jump-to-first/last stays fixed at `g`/`G`, and the arrow keys always move.)

## Review pacing

The `[review]` section tunes the FSRS scheduler shared by the Recall and
Reconstruct depths:

```toml
[review]
retention = 0.9         # FSRS target recall probability (0.70–0.99); higher = shorter intervals
retire_after = "1y"     # a card rests once its Recall interval reaches this ("2w", "6m", "30d", or "never")
max_new = 10            # max never-seen cards a session introduces (default 10)
limit = 40              # cap on total cards per session (default: no cap)
```

`retention` is the recall probability FSRS schedules for. `retire_after` is when
a card retires (rests until `alix reset`); `"never"` keeps it in rotation forever.
A workspace can override any of these keys for its own decks in an
`alix.local.toml` — see [Workspaces](08-workspaces.md). `max_new` and `limit`
pace a session; the precedence is `--new`/`--limit` on the launch > these
config keys > the built-ins (10 new, no cap).

How deeply you drill is never configuration: it's the **session depth** you
pick per review (the picker's Learn ▾ menu) — see
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
— a URL-based exam or `alix generate` will refuse and tell you to use a local
file instead, or switch backends.

Run `alix doctor --backends` to send a quick test request to the configured
backend and confirm it's installed, signed in, and responding. `--all-backends`
probes all four.

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
- **`[generate]`** — `alix generate`'s deck drafting: `model`, `timeout_secs` (300), `max_cards` (30),
  `extra`, a full `prompt` override, and `review`.
- **`[exam]`** — the AI exam: `model`, `timeout_secs` (300), `num_questions` (5),
  `pass_threshold` (1.0), `strictness` (`balanced`), `extra`.
- **`[trace]`** — `alix generate`'s trace and plan passes: defaults `model = "opus"`
  and `effort = "high"` (the build is correctness-critical and amortized); also
  `timeout_secs`. `auto_grade` (default `false`) has the model grade your typed
  predictions during a [trace walk](13-trace-decks.md) — a model call per hop,
  at the `[ask]` tier.

## Decks directory and storage

By default `alix` looks for decks in `~/decks`; set `decks_dir` to change it.
Progress is stored at `~/.local/share/alix/progress.json` (a workspace — or any
folder you serve with `alix <dir>` — keeps its own inside the folder; the
`stats`/`list`/`reset` commands take that deck, folder, or workspace as their
target and resolve its store the same way, with `--store <path>` as an
override).

Card identity is an XxHash64 over the deck **file name** plus the card's **back
lines** — so your progress survives editing a front or adding notes, but renaming a
deck file or changing a back line resets the affected cards. The hash is
**whitespace-insensitive**: it depends on the back lines' words, not their line
breaks, indentation, or repeated spaces, so reflowing or reindenting an answer
keeps a card's history. (That's the "editing is safe" rule from
[chapter 3](03-the-deck-format.md), stated precisely.)

`alix reset <target>` clears progress so cards go "new" again — a whole deck, a
folder or workspace (every member deck, plus a workspace's mastered flags and
virtual cards), a single card (`--card <id-or-front>`), or the entire store
(`--all`); it confirms once unless you pass `-y`.
