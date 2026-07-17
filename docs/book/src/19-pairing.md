# 19 · Pairing a device

alix's web server can lend a paired phone its AI backend for the tutor and
the exam, over `/api/remote/*`: the phone keeps its own decks and progress,
the desktop only computes answers.

## The pairing token changes on every restart

`alix --lan` prints a fresh, random pairing token each time the server
starts. This is the single biggest papercut in pairing a device: if an app
that paired fine yesterday suddenly can't reach the server, the token most
likely changed on the last restart. Re-pair with the freshly printed URL, or
pin one that never changes:

```toml
[serve]
token = "pick-your-own-fixed-token"
```

With `token` set, `--lan` reuses it instead of generating a new one, so a
saved pairing survives restarts.

## What the remote surface does

Nothing under `/api/remote/*` writes the server's own progress store,
session, decks, or recent list; it only computes an answer and hands it
back. A tutor question re-sends the whole conversation with every call,
since the server keeps no session for a remote client. An AI exam sitting is
graded on the server, but the result, any remediation cards, and what counts
as mastered stay the phone's to keep.

The server side of this ships from 0.6.0; see `docs/API.md`, section 4.10,
for the wire contract if you're building against it.

## Pairing the phone app

On your computer, run `alix --lan` and note the URL it prints (the same one
`[serve] token` can pin, above). On the [phone app](18-the-phone-app.md):

1. Open the deck list's overflow menu and choose **Pair with desktop…**.
2. Paste the printed URL into the sheet and tap **Pair**.

The app checks the server before saving anything, so a bad paste or an
unreachable desktop never gets stored silently. It shows one inline line
naming what went wrong:

- an unparseable paste: `that does not look like an alix pairing URL`
- a desktop it can't reach: `no alix answered at <host>:<port>`
- a desktop too old for this app's remote surface: `alix <version> found,
  this app needs 0.6.0 or newer`
- a desktop that answers but rejects the token (most often a server that
  restarted, and minted a fresh token, since the URL was printed):
  `alix answered but refused this token. Copy a fresh pairing URL from the server.`

On success the sheet closes with a note of which host you paired with. The
same menu item reopens the sheet later, now showing the current
`host:port` and an **Unpair** button; unpairing only clears the saved
config, nothing else on the phone changes.

## The tutor and the exam, borrowed

Once paired, review gains two things it doesn't have offline:

- An **Ask** chip, shown once you've attempted the current card (revealed
  it, picked a choice, submitted a typed answer, or walked all its lines)
  but not before: the same attempt-first rule the web tutor follows. It opens
  the same question/answer flow as the desktop tutor, including **Make a
  card**, re-sending the whole exchange to the paired desktop on every turn
  (the server keeps no session of its own for a remote turn).
- A **Take the exam** chip on the session summary, for any deck that
  declares a `% source:`. It opens a full-screen exam: one question at a
  time, then a Pass/Partial/Fail breakdown per question and, on a fail, a
  **Turn the gaps into cards** button. A pass and any remediation cards it
  creates land in the phone's own progress store, exactly like an offline
  grade, matching the rule above: the server computes, the phone keeps.

Both chips depend on the phone having confirmed the paired desktop is
reachable and running at least version 0.6.0; there is no retry chrome for
a dead or too-old server, the chip simply is not there.

If the desktop answers but rejects the token partway through a review or an
exam (the restart case above, caught mid-session instead of at pairing
time), the phone shows one SnackBar: "Pairing expired. Pair again from the
deck list menu." Pinning `[serve] token` is what stops this from happening
in the first place.

## Security posture

This is plain HTTP on your local network. The bearer token guards against
someone stumbling onto the server by accident, not against a hostile
network: anyone already on your LAN who gets hold of the token can use it.
For anything beyond your own LAN, put alix behind a VPN or a reverse proxy;
alix itself will not grow TLS or accounts.
