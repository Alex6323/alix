# 16 · Configuration

`alix` works out of the box; the config file is for when you want to change key
bindings, point at a different decks directory, or tune the AI features. It lives
at `~/.config/alix/config.toml` — create it with `alix config --init`, and
inspect the active key bindings with `alix config`.

## Key bindings

Every action takes a list of keys (the first is shown in the footer). To grade
self-graded cards with `j`/`k`/`l`:

```toml
[keys]
failed = ["j"]
partly = ["k"]
nailed = ["l"]
```

Keys are a single character (`"j"`), a special name (`"space"`, `"enter"`, `"tab"`,
`"esc"`, `"backspace"`), or either with a `ctrl-` prefix (`"ctrl-s"`). The
rebindable actions are `failed`, `partly`, `nailed`, `reveal`, `hint`, `submit`, `skip`,
`remove` (default `ctrl-x`), `continue`, `restart` (default `r`), and `quit`. While
you're typing an answer (typing/fuzzy mode), plain-character bindings are ignored so
they can't shadow your input — use `ctrl-`/special keys for `hint`, `skip`, and
`quit` there. Pass a different file with `--config <path>`.

`alix browse` has its own `[browse]` bindings, and the web server reads its
default port from `[serve]`:

```toml
[browse]
next = ["l", "n", "space"]
prev = ["h", "p"]
remove = ["x"]
quit = ["q", "esc", "ctrl-c"]

[serve]
port = 7777
```

(Jump-to-first/last stays fixed at `g`/`G`, and the arrow keys always move.)

## The AI sections

Each AI feature has its own section, all reusing the `[ask]` command and permission
settings:

- **`[ask]`** — the tutor: `command` (how to invoke Claude), `permission_mode`, the
  tool allowlist, a `model` override, `timeout_secs`, and an `effort`.
- **`[generate]`** — `alix deck`: `model`, `timeout_secs` (300), `max_cards` (30),
  `extra`, a full `prompt` override, and `review`.
- **`[exam]`** — `alix exam`: `model`, `timeout_secs` (300), `num_questions` (5),
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
deck file or changing a back line resets the affected cards. (That's the "editing is
safe" rule from [chapter 3](03-the-deck-format.md), stated precisely.)

`alix reset <deck>` clears progress so cards go "new" again — a whole deck, a
single card (`--card <id-or-front>`), or the entire store (`--all`); it confirms
first unless you pass `-y`.
