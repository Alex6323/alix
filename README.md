# flash

A spaced-repetition flashcard trainer for the terminal. Decks are simple
plain-text files; it adds a ratatui-based UI, Leitner and SM-2 schedulers,
several answer modes, cloze cards, deck dependencies, an ask-Claude helper,
AI deck generation, and per-card review statistics.

## Usage

The binary is called `flash`:

```sh
flash                            # pick decks interactively (recent + ~/decks)
flash mydeck.txt                 # review due cards (flip mode, Leitner)
flash --mode typing mydeck.txt   # type the answer character by character
flash --mode fuzzy mydeck.txt    # whole-line input, small typos tolerated
flash --mode choice mydeck.txt   # multiple choice (distractors from the deck)
flash --mode line mydeck.txt     # reveal the answer one line at a time (lyrics)
flash --scheduler sm2 mydeck.txt # SM-2 intervals instead of Leitner
flash --cram mydeck.txt          # ignore cooldowns, review everything
flash browse mydeck.txt          # read through cards, no grading or scheduling
flash generate <url>             # build a deck from a web page (via Claude)
flash deps mydeck.txt            # edit a deck's prerequisites (checkbox picker)
flash stats mydeck.txt           # progress overview
flash list mydeck.txt            # every card with stage and due time
flash check mydeck.txt           # lint a deck (syntax, duplicates)
flash reset mydeck.txt           # clear stored progress (also --card / --all)
```

Several decks can be given at once; their due cards are merged into one
session. Useful flags for `review`: `--new N` (max unseen cards to introduce,
default 10), `--limit N` (cap session size), `--max-typos N` (fuzzy tolerance
per line, default 2).

Run `flash` with no deck arguments (as the desktop launcher does) to open the
**deck picker**: it lists recently reviewed decks first, then every `*.txt`
in the decks directory (`~/decks` by default, set `decks_dir` in the
config). Type to filter by name, `Space` to (de)select one or more, `Enter`
to start, `Esc` to cancel.

## Deck format

The deck format:

```
% This line is a comment.

# What is the front of the card? (this is the front)
    The answer that must be typed. (back, may span several lines)
    A second answer line.
    ! An optional note, shown after answering.
    ! Further !-lines extend the note (multi-line notes).

# Next card
    \# A back line starting with a markup char is escaped with a backslash.
```

A note is shown as a quoted block: each sentence on its own line. A
` ```fenced``` ` block inside a note is rendered verbatim instead — its
indentation is preserved and it is not reflowed, so code stays readable.

Lines are trimmed, so indentation never has to be typed. A `#` only starts
a new card at column 0 — indented `#` lines are answer content (shell
comments, Rust attributes, Dockerfile comments...), no escaping needed.
Notes (`!`) and comments (`%`) work at any indentation.

### Deck directives

A deck can set its own defaults with `% key: value` comment lines in the deck
header (before the first card), so you do not have to repeat flags on the
command line:

```
% mode: line
% order: sequential
% scheduler: sm2
```

- `mode` — default answer mode (`flip`, `typing`, `fuzzy`, `choice`, `line`);
  can also be overridden per card (see below).
- `order` — `scheduled` (the default) or `sequential` to walk the deck in
  file order, top to bottom (ideal for lyrics with `% mode: line`).
- `scheduler` — `leitner` (default) or `sm2`.
- `direction` — `forward` (default), `reverse`, or `both`; per card or deck-wide
  (see below).
- `frontend` — `any` (default), `tui`, or `web`; restricts a card (or deck) to a
  frontend. Image cards are `web` automatically (see Images below).
- `img-dir` — directory that card `% img:` / `% img-back:` filenames resolve
  against (deck header only; see Images below).

These are ordinary `%` comments, so they don't affect parsing and card hashes
are unaffected. An explicit CLI flag always wins over a directive, which wins over
the built-in default. Directives are read only from the deck(s) you ask to
review — never from prerequisites pulled in via `% requires:` (see below). When
several requested decks disagree on a setting, the default is used. `flash check
<deck>` prints a deck's directives.

**Per-card mode.** A `% mode:` directive placed *after* a card's front (and
before the next one) overrides the deck's mode for that card only, so one deck
can mix modes — e.g. a `line` lyrics card among `flip` cards. The effective mode
is resolved per card: CLI `--mode` > the card's `% mode:` > the deck's
`% mode:` > the default (so `--mode` still forces every card). Other directives
(`order`, `scheduler`) stay deck-level.

### Deck dependencies

A deck can declare prerequisite decks with `% requires:` lines (repeatable):

```
% requires: rust-basics
% requires: rust-ownership
```

When you review a deck, its prerequisites are pulled in automatically —
transitively, de-duplicated — and the session is ordered **foundations
first**: a prerequisite deck's cards (both due reviews and newly introduced
ones) come before the cards of the deck that depends on it, while normal
scheduler order is kept within each deck. A prerequisite name resolves next to
the requiring deck or in the decks directory, with or without `.txt`. Missing
prerequisites and dependency cycles are reported as errors. They are ordinary
`%` comments, so hashes are unaffected.

A prerequisite contributes only its **cards** to the session, not its
directives: the `mode`, `order` and `scheduler` come from the deck you asked to
review, so requiring a `% mode: line` lyrics deck as a prerequisite won't switch
your session into line mode. Dependencies apply to `review` only — `browse` and
`stats` operate on exactly the decks you name.

You can edit a deck's prerequisites without hand-typing (and without typos)
with `flash deps <deck>` (alias `flash require`): it opens the deck picker over your
decks directory, pre-ticked to the current prerequisites. `Space` toggles,
`Enter` saves (rewriting the `% requires:` lines), `Esc` cancels; unticking
everything clears them. Since the lines are comments, editing dependencies
never affects card progress.

### Cloze cards (fill in the blank)

A front marked `#?` (no space) turns the card into a cloze card: every
`{{...}}` in its answer lines is a hole, and the card expands into one card
per hole. Each one shows the answer with that hole blanked out and the
others filled in, and you only produce the hidden text:

```
#? Complete the Rust declaration
    let {{mut}} x: {{u64}} = 0;
```

This makes two cards: `let ____ x: […] = 0;` (type `mut`) and
`let […] x: ____ = 0;` (type `u64`). The asked hole shows `____`; the other
holes are hidden as `[…]` so no card reveals its siblings' answers, and the
session queue keeps sub-cards of the same source card apart whenever other
cards are available. Only the doubled `{{` / `}}` are special — a lone `{` or
`}` is literal, so code like `let p = Foo {};` is fine in a cloze answer (write
a literal `{{` as `\{\{` if you ever need one). Progress of a cloze card
survives rewording its front and even a future change to the hole markup, but
editing its answer text or hole contents resets the affected holes.

### Dual-direction cards (`% direction:`)

A `% direction:` directive reviews a card both ways — useful for vocabulary and
other reversible facts:

```
# purported
    angeblich
    % direction: both
```

`both` makes two cards (`purported → angeblich` and `angeblich → purported`);
`reverse` keeps only the swapped one; `forward` (the default) is the card as
written. It works per card, or deck-wide as a header directive (`% direction:
both` before the first card) with per-card overrides — like `mode`. The two
directions get distinct progress, are kept apart in the queue (you won't see one
right after the other), and are removed together. The reversed card keeps the
note. Best for single-line cards; it does not apply to cloze cards.

### Images (`% img:`, `% img-back:`)

A card can show an image on the question side with `% img:`, the answer side
(revealed with the back) with `% img-back:`, or both:

```
% img-dir: /home/me/decks/img

# What phase is this?
    % img: moon-waxing.png
    Waxing gibbous

# Play this chord
    G major
    % img-back: g-major-tab.png
```

The deck-level `% img-dir:` (header only) is the directory filenames resolve
against; it may be absolute or relative to the deck file. Without it, filenames
resolve against the deck file's own folder. A card value that is itself an
absolute path is used as-is. One image per side.

Images render in the **web frontend only** — the terminal can't draw them — so
an image card is automatically *web-only* (as if it declared `% frontend: web`).
In the terminal, `flash review` skips such cards with a note, and if a whole
deck is web-only it points you at `--serve`. Use `% frontend:` to force a card
or deck to a frontend explicitly. `flash check` warns about a referenced image
file that doesn't exist (it doesn't fail the check).

## Review

The default **flip** mode is Anki-style: you reveal the answer and grade
yourself (again / good / easy — easy jumps two Leitner stages). The
**typing** mode has you type the back of the card character by
character with instant green/red feedback; `TAB` reveals the next two
characters as a hint, but a hinted card counts as failed. In **fuzzy** mode
you submit whole lines with Enter and small typos are tolerated. A wrong
card goes back to stage 1 and reappears later in the same session until you
get it right. Whichever mode a card uses is shown as a small badge above the
answer (`flip`, `typing exact`, `typing fuzzy`, `choice`, `line by line`).

In **choice** mode you pick the answer out of four options with `1`–`4`.
The three wrong options are sampled from the answers of the other cards in
the session, preferring similar-looking ones (years compete with years), so
no distractors ever have to be written. Recognition is easier than recall: a
correct pick grades as *good* (never *easy*), a wrong pick fails the card.
Cloze siblings are never used as distractors, and if a session has fewer
than four distinct answers the card falls back to flip mode.

In **line** mode the back is revealed one line at a time: press the reveal
key (`Space`) to uncover the next line, recalling it first. It is meant for
lyrics, poems, or any ordered list. Once every line is shown you grade
yourself again / good / easy, exactly like flip mode. Pair it with
`--order sequential` (or `% order: sequential` in the deck) to walk the
sections top to bottom — e.g. one card per verse/chorus of a song.

To throw a card away, press the **remove** key (`Ctrl-X` by default) on it
instead of grading — it is dropped from the session without being asked again
(cloze siblings go too). The marked cards are deleted from their deck files,
and their progress is pruned, when the session ends. The same key works in
`flash browse`.

Schedulers:

- **leitner** (default): a 6-stage box system with cooldowns
  0 / 1 h / 6 h / 24 h / 1 week. Pass moves a card up one stage, fail resets
  it to stage 1.
- **sm2**: SuperMemo-2 style with per-card ease factors and growing
  intervals. Existing Leitner progress is used as a starting point, and the
  Leitner stage is kept in sync, so you can switch back and forth.

## Browse

`flash browse <deck>...` is a walk through every card — front and back shown
together, in file order — without grading or scheduling. It is for a first
read-through of a new deck or just checking its contents, without affecting
your schedule. Navigate with `l`/`h` (next/previous, vim-style — `n`/`p`, the
arrow keys, and `Space` also work), `g`/`G` (first/last, also Home/End), and
`q` to quit. Pressing the remove key (`x` by default) marks the current card;
on quit the marked cards are deleted from their deck files and their progress
is pruned — the only thing browsing ever writes. The next/previous/remove/quit
keys are configurable in the `[browse]` section of the config file (see below);
first/last stay `g`/`G`. Run `flash browse` with no deck argument to choose
decks from the same picker `flash` uses.

## Web frontend

Add `--serve` to `review` or `browse` to run it in the browser instead of the
terminal — useful on a tablet or phone, where touch (and images) beats a TUI.
It runs the same session logic and writes to the same progress store, so
a card you grade or remove in the browser shows up on the command line and vice
versa.

```
flash review rust.txt --serve              # open http://127.0.0.1:7777
flash review rust.txt --serve --port 8080
flash review rust.txt --serve --lan        # reachable from other devices on your network
flash browse rust.txt --serve              # the browse view in the browser
flash --serve                              # no decks -> pick them in the browser
```

Run `--serve` **without** naming any decks and the browser opens a
deck-selection screen — the same list as the terminal picker (recent decks
first), with a checkbox per deck and a Start button — so you never have to drop
back to the terminal to choose. When you finish a session, "Choose other decks"
(on the summary, or in the ⋮ menu) returns to that screen, so you can study a
different deck without restarting. Naming decks on the command line skips the
screen and goes straight to review/browse.

Every answer mode works in the browser: **flip** (reveal, then self-grade
Again / Good / Easy), **line** (reveal a verse one line at a time — it
auto-scrolls to follow the newest line), **typing** / **fuzzy** (type your
answer and submit; checked exactly or with your configured typo tolerance, each
line marked ✓/✗ with the correct answer shown), and **choice** (tap one of the
options). The note appears once the answer is shown. Controls are big tap
targets and follow your configured key bindings — the page reads them from the
server, so the chips show your own keys. The overflow menu (⋮) holds **Remove**,
which deletes the current card from its deck file and prunes its progress, and
**Choose decks**, which returns to the deck-selection screen.

It is deliberately local-only — no accounts, no database. By default it binds
to `127.0.0.1` (this machine only); `--lan` binds all interfaces so a device on
the same network can reach it at `http://<your-machine-ip>:<port>` (no
authentication, so only use `--lan` on a network you trust). `--port` and
`--lan` require `--serve`; the default port lives in the `[serve]` section of
the config file and `--port` overrides it.

## Ask Claude about a card

On any post-answer screen (feedback, revealed flip card, answered choice),
press `?` to ask Claude about the card without leaving the session. The
card (front, answer, note, deck name) is sent as context to the Claude Code
CLI (`claude -p`), so you can ask "why is that the answer?" and follow up.
One CLI conversation spans the whole review run (`--session-id` on the
first question, `--resume` afterwards), so Claude remembers earlier cards
and questions. While Claude thinks, the session stays responsive; Esc
returns exactly where you were.

While typing a question you can edit it like a normal input line: `←`/`→`
move the caret, `Home`/`End` (or `Ctrl-A`/`Ctrl-E`) jump to the ends, and
`Backspace`/`Delete` remove the character before/under it.

Decks can carry reference links as comment lines:

```
% link: https://docs.rs/async-compat
% link: https://tokio.rs/tokio/tutorial
```

They are handed to Claude with the first question as background material to
consult when useful — fetched once, remembered for the rest of the run. These
lines do not affect card hashes.

Because the CLI runs headless (`-p`), it cannot show interactive permission
prompts — an unanswerable prompt would hang the call. flash therefore
runs it with `--permission-mode dontAsk` and an exclusive tool allowlist
(`WebFetch`, `WebSearch` by default): the listed tools work without
prompting, and every other tool is silently denied, so a malicious page
behind a deck link cannot make the tutor run shell commands. Both the
permission mode and the allowlist are configurable in `[ask]`.

`Ctrl-N` condenses the conversation into at most three short note lines and
appends them to the card in the deck file (notes are not hashed, so the
card's progress is untouched). Requires the `claude` CLI to be installed
and logged in; the command, a `--model` override and the timeout are
configurable in the `[ask]` section of the config file.

## Generate a deck from a web page

`flash generate <url>` turns a page (a Rust book chapter, a Wikipedia article,
API docs) into a deck using the Claude CLI:

```sh
flash generate https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
flash generate <url> -o ownership      # choose the file name
flash generate <url> --cards 15        # cap the number of cards
flash generate <url> --review          # add a 2nd pass to remove redundant cards
flash generate <url> --print           # print to stdout instead of writing
```

Claude reads the page with the **WebFetch** tool (already on the ask
allowlist) and emits the deck as plain text; flash then validates it
(`parse_str`) and writes `~/decks/<slug>.txt`. Claude is never given a write
or shell tool — it only returns text — so the safe `dontAsk` +
WebFetch/WebSearch permission model is unchanged. The generated cards are
spread across four layers of understanding (facts → concepts → application →
connections), use cloze (`#?`) cards for terminology, and start with a
`% link:` line back to the source so you can use the *ask* feature on them.

The prompt tells Claude to draft, then re-read the whole set and merge or drop
cards that test the same fact, so the deck doesn't repeat itself. For a
stronger pass, `--review` (or `generate.review = true`) runs a **second**
Claude call that takes the draft and returns a deduplicated, tightened version
— an extra call, worth it when the source is repetitive.

The prompt and limits live in the `[generate]` section of the config:
`model`, `timeout_secs` (default 300), `max_cards` (default 30), `extra`
(guidance appended to the built-in prompt), `prompt` (a full override, with
`{url}` and `{max_cards}` placeholders), and `review`. It reuses the `[ask]`
command and permission settings. Review the result before relying on it — it is a starting
point, not a final deck. Generation needs the `claude` CLI installed and
logged in, and works best on a single page or chapter (a whole book overruns
the context budget).

## Configuration

Key bindings can be changed in `~/.config/flash/config.toml`. Create the
file with `flash config --init`, inspect the active bindings with
`flash config`. Every action takes a list of keys; the first one is
shown in the footer. For example, to grade flip-mode cards with j/k/l:

```toml
[keys]
again = ["j"]
good = ["k"]
easy = ["l"]
```

Keys are written as a single character (`"j"`), a special key name
(`"space"`, `"enter"`, `"tab"`, `"esc"`, `"backspace"`), or either with a
`ctrl-` prefix (`"ctrl-s"`). Rebindable actions: `again`, `good`, `easy`,
`reveal`, `hint`, `submit`, `skip`, `remove` (mark the card for deletion from
its deck file, default `ctrl-x`), `continue`, `restart` (start a new session
from the summary screen, default `r`), `quit`. While you are typing
an answer (typing and fuzzy mode), plain character bindings are ignored so
they cannot shadow text input — use `ctrl-`/special keys for `hint`, `skip`
and `quit`. A different config file can be passed with `--config <path>`.

The browser (`flash browse`) has its own bindings in a `[browse]` section:

```toml
[browse]
next = ["l", "n", "space"]    # default vim-style l, plus n and space
prev = ["h", "p"]
remove = ["x"]                # mark the current card for removal
quit = ["q", "esc", "ctrl-c"]
```

Letter bindings are case-insensitive, so jump-to-first/last stays fixed at
`g`/`G` (and Home/End); the arrow keys always move next/previous too.

The web frontend (`--serve`) reads its default port from a `[serve]`
section; `--port` overrides it:

```toml
[serve]
port = 7777
```

## Card identity and storage

- Cards are identified by an XxHash64 over the deck **file name** plus the
  back lines. Your progress survives editing a card's front and adding notes,
  but renaming a deck file or editing its back lines resets the affected
  cards.
- Progress is stored at `~/.local/share/flash/progress.json` and config at
  `~/.config/flash/config.toml` (created on first use).
- `flash reset <deck>...` clears stored progress so cards become "new" again —
  for whole decks, a single card (`--card <id-or-front-text>`), or the entire
  store (`--all`). Run it with no decks to pick from the deck list, or add
  `--cards` to pick individual cards from a checkbox list. It confirms first
  unless you pass `-y`/`--yes`.

## Desktop integration

To launch flash from the desktop menu (Cinnamon, GNOME, KDE, ...), run:

```sh
assets/install-desktop.sh
```

It installs the icon (`assets/flash.svg`, rendered to the standard
PNG sizes), a launcher that reviews everything due in `~/decks` in a
terminal, and a `.desktop` entry under `~/.local/share`. Re-run it after
editing the SVG. The launcher prefers an installed `flash` (`cargo install
--path .`) and falls back to the project build.

## Development

```sh
cargo test       # unit tests
cargo clippy     # lints
```
