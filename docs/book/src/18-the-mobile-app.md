# 18 · The mobile app

There is a native Android app: the same review loop as the web app, running
the same core (parser, scheduler, progress store) compiled into the app, so
it works entirely offline, including a [trace deck](13-trace-decks.md)'s
predict/reveal/self-grade walk. It is early software with a deliberately
small surface: reviewing decks. Pairing it with a running `alix` server on
your network lends it the tutor, the AI exam (a trace's compression exam
included), deck generation, and note-taking: see
[Pairing a device](19-pairing.md).

Settings (the ☰ button) → **Theme** picks from the web gallery’s 18 non-kids
themes (the three Kids palettes are web-only; see
[Themes](15-the-web-app.md#themes)); the app re-themes live, no restart.

## Install

Grab `alix-arm64-v8a.apk` from the project's GitHub Releases (the
`alix mobile vX.Y.Z` releases) and install it. Android will warn about
installing outside a store; that is expected for now. The app works on
Android 7+ and keeps its decks in private app storage: a fresh install ships
a few sample decks so there is something to review immediately, and pairing
with a desktop server (see [Pairing a device](19-pairing.md)) adds the
**Generate deck** row for bringing in more.

Settings → **About** shows two versions: the app's own and the embedded
core's. The app has its own release stream; it does not track the CLI's
version.

## The deck list

The first screen lists the phone's own decks and, once paired, the pulled
copy of the desktop's below them. The two are listed separately and the
phone's own appear first; the screen stays responsive while a large
workspace is still being read, and shows nothing in place of the list until
the first answer arrives. A deck read once stays parsed in memory for as
long as the app runs, so coming back to the list after a review does not
read every deck again; a deck whose file changed on disk is read fresh.

## Syncing with the desktop

Once paired (see [Pairing a device](19-pairing.md)), a pulled entry's row
gains a **Sync** action in its `⋮` menu: it pushes every local deck whose
progress changed, then pulls that entry when the desktop's copy differs
from the phone's. The app also runs one cycle in the background each time
it opens, for every entry already on the phone, once the paired desktop
answers and still serves the same root; an entry whose files did not
change on the desktop is left as it is. A
review session's summary silently pushes that deck's progress too, with
no visible step on a normal pass.

A pull replaces the files the desktop owns for that entry (its decks,
their augment and asset files) with the desktop's current copies; anything
the phone added on its own inside the entry is left alone and reported as
phone-only. A member the desktop deleted stays on the phone while its
progress is not yet pushed, listed as kept, so nothing you reviewed is
lost before it reached the desktop. A conflicting deck, one whose progress changed on both sides
since the last sync, stops review of that deck and asks: keep the phone's
progress or take the desktop's, naming what each choice discards. An entry
the desktop no longer serves stays on the phone, reviewable, listed as
orphaned, with a Remove action once you're done with it.

A desktop entry the phone has never pulled shows below the phone's own
entries: its name and size only, in a subdued row with no `⋮` menu and no
review action. Tapping it pulls it for the first time; once it lands it
becomes an ordinary entry, reviewable like any other.

Tapping the status line above the deck list (shown while a cycle runs, or
its last report is unread) opens the full report: landed, pushed, kept,
phone-only, removed, renamed, and refused categories, plus **left out on
the desktop** (members inside an entry the desktop could not load) and
**not on this phone** (entries the desktop serves that this phone has
never pulled, the same ones shown as rows below the deck list).

Settings gains a **Paired desktop** row once a pairing is saved, opening a
sheet to choose which paired desktop's entries the picker shows below the
phone's own; the phone's own decks stay listed whichever desktop is active.

## Workspace deadlines

A workspace's personal "ready by" date shows on its row (date, days left,
and ready percent, colored to flag urgency inside the last week or past
due) and again once you drill in, the same readout as the web picker.
**Long-press the workspace row** to set, move, or clear it. The date lives
in the workspace's own `alix.local.toml` (see [Workspaces](08-workspaces.md));
the phone's own offline sessions bend their scheduling toward the date
exactly as the desktop does.
