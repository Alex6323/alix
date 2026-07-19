# alix on your phone

There is a native Android app: the same review loop as the web app, running
the same core (parser, scheduler, progress store) compiled into the app, so
it works entirely offline, including a [trace deck](13-trace-decks.md)'s
predict/reveal/self-grade walk. It is early software with a deliberately
small surface: reviewing decks. Pairing it with a running `alix` server on
your network lends it the tutor, the AI exam (a trace's compression exam
included), deck generation, and note-taking: see
[Pairing a device](19-pairing.md).

The overflow menu's **Theme…** picks from the same 18-theme gallery the web
app ships (see [Themes](15-the-web-app.md#themes)); the app re-themes live,
no restart.

## Install

Grab `alix-arm64-v8a.apk` from the project's GitHub Releases (the
`alix mobile vX.Y.Z` releases) and install it. Android will warn about
installing outside a store; that is expected for now. The app works on
Android 7+ and ships a few sample decks so a fresh install has something to
review.

The overflow menu's **About** shows two versions: the app's own and the
embedded core's. The app has its own release stream; it does not track the
CLI's version.

## Your own decks: a shared folder

By default the app keeps decks in its private storage. To review the decks
you actually maintain, point it at a real folder on the phone:

1. Sync your decks folder to the phone with whatever you already use
   (Syncthing is the natural fit: local, no accounts).
2. In the app: the overflow menu, **Decks folder…**, then **Choose shared
   folder…**. Android 11 or newer.
3. The first time, Android opens its **All files access** page: alix reads
   and writes plain files in a folder another app manages, which is exactly
   what this permission grants. Enable it, go back, choose again.
4. Pick the folder. The app lists it immediately; progress
   (`progress.json`) is written next to the decks, exactly like the
   desktop, so it travels with the folder.

**Use app storage** in the same sheet switches back; nothing is deleted
either way. If the folder becomes unavailable (permission revoked, folder
gone), the app falls back to its private decks for that launch and says so;
fixing the cause heals it on the next start.

## Workspace deadlines

A workspace's personal "ready by" date shows on its row (date, days left,
and ready percent, colored to flag urgency inside the last week or past
due) and again once you drill in, the same readout as the web picker.
**Long-press the workspace row** to set, move, or clear it. The date lives
in the workspace's own `alix.local.toml` (see
[Workspaces](08-workspaces.md)), so a synced folder carries it between
phone and desktop, and the phone's own offline sessions bend their
scheduling toward the date exactly as the desktop does.

## One device at a time

The progress store is a single file, rewritten on every grade, and alix
does not merge concurrent histories (deliberately: fail loud beats a silent
merge that corrupts scheduling). Syncing the folder between a computer and
a phone works well under one rule: **review on one device at a time**, and
let the sync settle before switching.

Two guards back the rule:

- If another device wrote the store minutes ago, the review screen says so
  before you grade anything.
- If the folder contains a sync conflict file (Syncthing's
  `progress.sync-conflict-….json`), the deck list warns loudly. Resolve it
  by keeping the file you trust: usually delete the conflict copy if the
  newer history won, or replace `progress.json` with the conflict copy if
  it holds the reviews you want. There is no merge.

Two Syncthing tips: add `*.json.tmp` to the folder's `.stignore` (alix
writes through a temp file; there is no point syncing it), and prefer
"send & receive" on both sides so the phone's grades actually travel back.
