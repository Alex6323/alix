# 16 · Configuration

`alix` works out of the box; the config file is for when you want to change key
bindings, point at a different decks directory, or tune the AI features. It
lives in the platform's config directory (on Linux
`~/.config/alix/config.toml`); create it with `alix config --init`, and
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
rebindable `[keys.review]` actions are `failed`, `partly`, `passed`, `reveal`, `submit`, `skip`,
`remove` (default `ctrl-x`), `ask` (default `?`), `context` (default `c`),
`continue`, `restart` (default `r`), `quit`, `up`/`down`
(defaults `k`/`j`) to move within a multiple-choice or key-point list (the arrow keys always work too),
and the tutor's distill actions `make_note` (default `ctrl-n`) and `make_card` (default `ctrl-d`). While
you're typing an answer (a reconstruct check), plain-character bindings are ignored so
they can't shadow your input: use `ctrl-`/special keys for `skip` and
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

[log]
max_bytes = 5242880   # cap for each of the current and rolled files
verbose = false       # also record verbose targets such as HTTP timings
```

(Jump-to-first/last stays fixed at `g`/`G`, and the arrow keys always move.)

The server log is always on. `[log] max_bytes` bounds each of its two files and
must be positive. Card selection and content-free panic, AI, parser, and HTTP
failure records are written by default. `[log] verbose = true` adds HTTP
timings. For live debugging, `--log http,select,error` also
mirrors the named targets to stderr and enables verbose file records for that
run. It does not add card content, deck names, or request paths.

## Review pacing

The `[review]` section tunes the FSRS scheduler shared by all three review
depths:

```toml
[review]
retention = 0.9          # FSRS target recall probability (0.70–0.99); higher = shorter intervals
recognize_retention = 0.85  # same, for the Recognize depth alone; recognition decays slower
retire_after = "1y"      # a card rests once its Recall interval reaches this ("2w", "6m", "30d", or "never")
introduction_cooldown = "5m"  # settle gap before a new card's first quiz ("90s", "10m", "1h"; "0" = none)
max_session = 10         # cards a single sitting serves (default 10)
new_cards_percent = 30   # new-card share of max_session; the rest are due cards (default 30)
```

`retention` is the recall probability FSRS schedules for; `recognize_retention`
is the same knob for the Recognize depth alone, laxer by default (0.85) because
recognition holds far longer than production. `retire_after` is when
a card retires (rests until `alix reset`); `"never"` keeps it in rotation forever.
`introduction_cooldown` is the settle gap between seeing a new card and its first
graded quiz, and the same floor keeps *any* just-seen card (a miss, a wrong
pick) from returning immediately, so one knob paces both. A bare number is
minutes; `"0"` disables the gap.

`max_session` is how many cards one sitting serves; `new_cards_percent` is the
new-card slice of that cap (so at the defaults, three new and seven due out of
ten). Whichever pool comes up short, the other fills the cap, so a fresh deck
serves ten new and a deck with nothing new serves ten due; a big backlog just
slows introductions proportionally. A workspace can override any of these keys
for its own decks in an `alix.local.toml` (see [Workspaces](08-workspaces.md)).
The precedence for the cap is `--session N` on the launch > `max_session` > the
built-in 10; `new_cards_percent` has no launch flag.

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
mastered, or finished when they have no exam grounding. Mastery itself rests
on the [AI exam](12-the-ai-exam.md)'s sampled questions, not a check of every
card. Treat ready% as evidence toward readiness, not proof of it.

How deeply you drill is never configuration: it's the **session depth** you
pick per review (the picker's Depth… menu). See
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
a URL-based exam or a `generate` subcommand will refuse and tell you to use a local
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
  an `effort`, `source_access` (local source grounding; a workspace manifest
  may override it, see [the tutor](10-tutor.md)), and `preflight_threshold`
  (warn and confirm before spending a large model call on a local source tree
  bigger than this many bytes; `0` proceeds silently).
- **`[generate]`**: `alix deck generate`'s drafting: `model`, the absolute
  `timeout_secs` (3600), and `idle_timeout_secs` (300, or `0` to disable).
  The latter is a resetting inactivity limit for structured-event backends and
  a nonrenewing absolute fallback for unstructured backends. Other controls are
  `max_cards` (100, a soft ceiling: exceeding warns, never truncates), default `language` and `audience`, `card_style` (`mixed`,
  `plain`, `cloze`, or `authored-choices`), `extra`, a `prompt` override, and
  `review`. Per-run flags override the language, audience, and style defaults.
  The terminal shows calm live progress, but partial generated cards remain
  hidden until the result passes validation.
- **`[exam]`**: the AI exam: `model`, `timeout_secs` (300), `num_questions` (5),
  `pass_threshold` (1.0), `strictness` (`balanced`), `extra`,
  `retry_cooldown_secs` (3600; `0` disables the wait before re-sitting a
  failed trace exam; fact-deck exams never wait).
- **`[trace]`**: the `generate deck --trace` and `workspace generate` planning
  passes: `model` defaults to
  unset, which resolves to the backend's strong model where it defines one
  (Claude: `opus`), and `effort` defaults to `"high"` (the build is
  correctness-critical and amortized); also `timeout_secs` and `extra`
  (extra guidance appended to the build prompt).
- **`[ai]`**: [`alix deck augment`](15-the-web-app.md#augmenting-a-deck-from-the-picker)'s
  generation targets: a `model` override, `distractor_count` (3),
  `variant_count` (4), `keypoint_count` (5), and `timeout_secs` (300, sized
  for a whole-deck batch).

## Decks directory and storage

By default `alix` looks for decks in `~/decks`; set `decks_dir` to change it.
Shareable material and private user files default to that folder, one document
per initialized deck:

```text
<decks_dir>/
├── augment/deck-<token>.json
├── progress/deck-<token>.json
└── recent.json
```

`progress/` is private, indispensable learning state: schedules, review
history, exam state, and the last writer. `augment/` is
regenerable, shareable material: generated choices, notes, key points,
variants, and topologies. It stays beside the deck so sharing the deck can
carry its generated study material without carrying personal history. The
stable deck id (`deck-<token>`), not the Markdown filename, selects both
documents, so renaming a deck keeps their ownership stable.

Bare `alix` and `alix <dir>` use the same user-files root when `<dir>` is the
configured `decks_dir`. A workspace, or any other folder served with `alix
<dir>`, keeps its shareable `augment/` and `assets/` beside its decks. The
`stats`/`list`/`reset` commands take a deck, folder, or workspace and resolve
the same private progress documents. `--store <directory>` and a workspace's
`store = "..."` manifest setting override only the user-files root for
`progress/` and `recent.json`; they do not relocate augmentation or assets.
Relative workspace `store` values are anchored to the workspace.

Each document carries its owner ID, format version, and revision. Saves write a
sibling `.json.tmp` and atomically rename it into place. A process that can see
that its loaded revision is stale refuses to overwrite the newer document.
If the replacement commits but the final directory flush fails, Alix reports
the failure while retaining the committed revision in memory, so a later save
can retry instead of remaining stale forever.
This protects local overlapping writers; it cannot turn disconnected folder
synchronization into a transaction.

Alix is pre-1.0 and reads only the current version-1 per-deck documents. A
persisted-state format break is handled before installing that build: back up
the affected files, perform any one-time conversion outside production Alix, and
verify the result with `alix doctor <folder>`. Production does not contain
runtime compatibility branches or converters for superseded pre-1.0 layouts.

### Backing up

Everything Alix stores is plain files in one folder, so a backup is a copy of
that folder. Use whatever you already use for folders: a cloud drive, `git`,
`rsync`, Time Machine, or `cp -r`. Alix manages no backup archives or
generations of its own: a general backup could only reproduce what those tools
already do, losslessly, over the same plain files. What it does keep is one
`.bak` safety net per overwrite: [`alix deck
restore`](17-command-reference.md) swaps a deck (file, review history,
augmentations) with the backups the last overwrite left behind. Keep an
independent copy of any study history you care about.

What Alix does guarantee is that its **own** writes cannot corrupt your files.
Every state, deck, and manifest write goes to a sibling temporary file that is
flushed to disk, atomically renamed over the target, and (on Linux and macOS)
has its directory entry flushed. An interrupted save leaves the previous file
intact, never a half-written one; a kill-point fault-injection suite checks that
at every filesystem operation, including partial multi-document saves and the
deck/progress promotion boundary. Surviving a hard power loss additionally
relies on flushing before the rename, which the code does but a test cannot
simulate. That protects
against Alix; your own folder backup protects against disk failure and
accidental deletion.

### Multi-device via your cloud drive

With the defaults, your decks, augmentation, assets, and progress live in one
folder. Put that folder in a cloud drive you already use (Dropbox, iCloud,
OneDrive, Syncthing) and it follows you across devices. Set `store` when you
want progress and recent history to remain private to one device. Alix stays
unaware that the folder is synced and uploads nothing itself.

For a free, no-account option that fits alix's local-first grain,
[Syncthing](https://syncthing.net) works well: install it on each machine, pair
the devices, and share your decks folder between them. It syncs the folder
peer-to-peer over your own network, with no cloud company in the middle.

The writer boundary is now **one deck**, not the whole workspace. Different
devices may review different decks in the same synchronized folder: their
progress documents do not compete. For the same deck, use one active writer and
let synchronization settle before switching devices. Alix does not merge
concurrent same-deck reviews or decide which schedule is semantically correct.
A disconnected collision therefore remains a Syncthing conflict copy. If a
running web session's own deck is replaced under it, that session can no longer
save; the review screen shows a persistent banner and the fix is to reopen the
deck (grades made after the collision are not kept).

Run `alix doctor <folder>` before recovery. For a progress conflict, stop both
writers and synchronization, back up the folder, compare the canonical
`progress/deck-<token>.json` with its
`deck-<token>.sync-conflict-….json` copies, and deliberately keep the complete
history you trust at the canonical path. Do not combine schedules by hand.
Augmentation conflicts are regenerable: keep one complete
`augment/deck-<token>.json` or move all conflicting copies aside and regenerate
that deck's augmentation. Resume synchronization and rerun doctor only after
the canonical files are settled.

A card's identity is a minted token alix writes into the file as an
`<!-- id: ... -->` line, not a hash over its content. Editing any text,
including the answer, preserves a card's history; only deliberately replacing
a card starts it over. (That's the "editing is safe" rule from
[chapter 3](03-the-deck-format.md), stated precisely.)

`alix reset <target>` clears progress so cards go "new" again: a whole deck, a
folder or workspace (every member deck, plus a workspace's mastered flags and
personal-card schedules), a single card (`--card <id-or-front>`), or the entire store
(`--all`); it confirms once unless you pass `-y`.
