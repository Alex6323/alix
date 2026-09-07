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

## Workspace deadlines

A workspace's personal "ready by" date shows on its row (date, days left,
and ready percent, colored to flag urgency inside the last week or past
due) and again once you drill in, the same readout as the web picker.
**Long-press the workspace row** to set, move, or clear it. The date lives
in the workspace's own `alix.local.toml` (see [Workspaces](08-workspaces.md));
the phone's own offline sessions bend their scheduling toward the date
exactly as the desktop does.
