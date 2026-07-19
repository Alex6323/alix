# 16 · Configuration

`alix` works out of the box; the config file is for when you want to change key
bindings, point at a different decks directory, or tune the AI features. It lives
at `~/.config/alix/config.toml`, create it with `alix config --init`, and
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
`remove` (default `ctrl-x`), `continue`, `restart` (default `r`), `quit`, `up`/`down`
(defaults `k`/`j`) to move within a multiple-choice or key-point list (the arrow keys always work too),
and the tutor's distill actions `make_note` (default `ctrl-n`) and `make_card` (default `ctrl-d`). While
you're typing an answer (a reconstruct check), plain-character bindings are ignored so
they can't shadow your input: use `ctrl-`/special keys for `hint`, `skip`, and
`quit` there. Pass a different file with `--config <path>`.

The picker's navigation is `[keys.picker]` (`up`, `down`, `open`, `back`,
`filter`, `mastered`, plus `depth` to open the depth menu,
`recognize`/`recall`/`reconstruct` to pick within it, and `cram` to toggle its
tick-box, defaults `v`, `1`/`2`/`3`, and `c`), the browse overlay has its own `[keys.browse]` bindings, and
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
audience = "adult"   # or "kids", which frontend `/` serves, and the tutor's voice (see 15 · The web app)
```

(Jump-to-first/last stays fixed at `g`/`G`, and the arrow keys always move.)

## Review pacing

The `[review]` section tunes the FSRS scheduler shared by the Recall and
Reconstruct depths:

```toml
[review]
retention = 0.9         # FSRS target recall probability (0.70–0.99); higher = shorter intervals
retire_after = "1y"     # a card rests once its Recall interval reaches this ("2w", "6m", "30d", or "never")
acquire_cooldown = "5m" # settle gap before a new card's first quiz ("90s", "10m", "1h"; "0" = none)
max_new = 10            # max never-seen cards a session introduces (default 10)
limit = 40              # cap on total cards per session (default: no cap)
```

`retention` is the recall probability FSRS schedules for. `retire_after` is when
a card retires (rests until `alix reset`); `"never"` keeps it in rotation forever.
`acquire_cooldown` is the settle gap between seeing a new card and its first
graded quiz, and the same floor keeps *any* just-seen card (a miss, a wrong
pick) from returning immediately, so one knob paces both. A bare number is
minutes; `"0"` disables the gap.
A workspace can override any of these keys for its own decks in an
`alix.local.toml`. See [Workspaces](08-workspaces.md). `max_new` and `limit`
pace a session; the precedence is `--new`/`--limit` on the launch > these
config keys > the built-ins (10 new, no cap).

### Ready by a deadline

Two more `[review]` keys exist only in a workspace's `alix.local.toml`, never
in the global config (which rejects both outright):

```toml
[review]
deadline = "2026-09-01"   # a personal "ready by" date; the day itself counts
deadline_ramp = "14d"     # how early the pre-deadline ramp starts ("2w"; "0" = cap only)
```

`deadline` is an ISO date (`YYYY-MM-DD`). `deadline_ramp` takes a bare number
of days, `"<n>d"`, or `"<n>w"`; `"0"` caps intervals at the days left without
ramping retention early. Inside the window the target retention climbs
linearly toward a fixed **0.95** by the deadline day (deliberately not a
config key); see [Scheduling](05-scheduling.md) for the full mechanics.

These keys are **workspace-only**: they take effect only in a directory with
an `alix.toml`. In a plain decks folder, or on a loose deck, they parse but do
nothing (no ramp, no picker readout, no doctor warning). `alix workspace
deadline` refuses a non-workspace directory and points at `alix workspace
init`.

The picker's ready percent counts a deadline's member decks as ready once
mastered (or finished, for a source-less deck), and mastery itself rests on
the [AI exam](12-the-ai-exam.md)'s sampled questions, not a check of every
card. Treat ready% as evidence toward readiness, not proof of it.

How deeply you drill is never configuration: it's the **session depth** you
pick per review (the picker's Learn ▾ menu). See
[Reveal & session depths](04-review-modes.md). The old `[review] depth` config
key (and the per-deck `[review.deck."<file>"]` override), a *dial* that fixed
the drilling depth from config, is gone, not renamed; a config that still
sets either now fails to load.

## Backends

By default all AI calls go through the [Claude Code](https://www.anthropic.com/claude-code)
CLI. You can switch to one of the other supported CLIs with `backend` in `[ask]`:

```toml
[ask]
backend = "claude"   # default, Claude Code CLI
# backend = "gemini"  # Google Gemini CLI
# backend = "codex"   # OpenAI Codex CLI
# backend = "copilot" # GitHub Copilot CLI
```

Auth is each CLI's own login: alix stores no API keys. Install whichever CLI
you want to use and run its login command once.

Each backend is granted **read-only tools only** (file reading; web fetch where
the backend supports it). Codex runs under a network-blocking sandbox rather
than a tool allowlist, so it can read local source files but can't fetch URLs:
a URL-based exam or `alix generate` will refuse and tell you to use a local
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

- **`[ask]`**: the tutor: `command` (how to invoke the CLI), `backend`,
  `permission_mode`, the tool allowlist, a `model` override, `timeout_secs`,
  and an `effort`.
- **`[generate]`**: `alix generate`'s deck drafting: `model`, `timeout_secs` (300), `max_cards` (30),
  `extra`, a full `prompt` override, and `review`.
- **`[exam]`**: the AI exam: `model`, `timeout_secs` (300), `num_questions` (5),
  `pass_threshold` (1.0), `strictness` (`balanced`), `extra`.
- **`[trace]`**: `alix generate`'s trace and plan passes: defaults `model = "opus"`
  and `effort = "high"` (the build is correctness-critical and amortized); also
  `timeout_secs`. `auto_grade` (default `false`) has the model grade your typed
  predictions during a [trace walk](13-trace-decks.md), a model call per hop,
  at the `[ask]` tier.

## Decks directory and storage

By default `alix` looks for decks in `~/decks`; set `decks_dir` to change it.
The progress store lives **in your decks folder** (`<decks_dir>/progress.json`),
the same store `alix <dir>` uses for that folder, so bare `alix` and
`alix <dir>` share one store when `<dir>` is your configured `decks_dir`. A
workspace, or any other folder you serve with `alix <dir>`, keeps its own
`progress.json` inside that folder too. The `stats`/`list`/`reset` commands
take a deck, folder, or workspace as their target and resolve its store the
same way, with `--store <path>` as an override.

### Multi-device via your cloud drive

Because your decks and their progress live in one folder, put that folder in a
cloud drive you already use (Dropbox, iCloud, OneDrive, Syncthing) and it follows
you across devices. Use one device at a time; alix stays unaware that the folder
is synced, and it writes the store atomically so a background sync never sees a
half-written file. There are no accounts and nothing is uploaded by alix itself.

For a free, no-account option that fits alix's local-first grain,
[Syncthing](https://syncthing.net) works well: install it on each machine, pair
the devices, and share your decks folder between them. It syncs the folder
peer-to-peer over your own network, with no cloud company in the middle. Because
alix does not yet merge concurrent edits, keep to one device at a time; reviewing
on two at once while offline would leave Syncthing to resolve a `progress.json`
conflict.

A card's identity is a minted token alix writes into the file as an
`<!-- id: ... -->` line, not a hash over its content. Editing any text,
including the answer, preserves a card's history; only deliberately replacing
a card starts it over. (That's the "editing is safe" rule from
[chapter 3](03-the-deck-format.md), stated precisely.)

`alix reset <target>` clears progress so cards go "new" again: a whole deck, a
folder or workspace (every member deck, plus a workspace's mastered flags and
virtual cards), a single card (`--card <id-or-front>`), or the entire store
(`--all`); it confirms once unless you pass `-y`.
