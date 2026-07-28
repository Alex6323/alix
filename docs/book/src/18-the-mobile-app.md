# 18 · The mobile app

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
4. Pick the folder. The app lists it immediately; each initialized deck's
   progress is written as `progress/deck-<token>.json`, exactly like the desktop,
   so it travels with the folder.

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

## One writer per deck

Progress is split into one versioned document per deck. A computer and phone
can review **different decks** in the same synced folder without rewriting the
same file. Alix does not merge concurrent histories for the **same deck**
(deliberately: fail loud beats a silent merge that corrupts scheduling), so
let sync settle before switching that deck to another device.

Two guards back the rule:

- If another device wrote that deck's progress minutes ago, the review screen
  says so before you grade anything.
- If the folder contains a sync conflict file (Syncthing's
  `progress/deck-<token>.sync-conflict-….json`), the deck list warns loudly.
  Stop both writers and sync, back up the folder, and deliberately keep the
  complete document you trust at `progress/deck-<token>.json`. Do not combine
  schedules by hand; there is no merge. `alix doctor <folder>` on the desktop
  lists every conflict and should be clean before you resume.

Two Syncthing tips: add `*.json.tmp` to the folder's `.stignore` (alix
writes through a temp file; there is no point syncing it), and prefer
"send & receive" on both sides so the phone's grades actually travel back.

Install the same Alix version on every device that writes a synchronized
folder. For a pre-1.0 persisted-state format break, stop every writer, back up
the folder, complete the release's external conversion procedure, synchronize
the resulting per-deck documents, and only then resume review.
