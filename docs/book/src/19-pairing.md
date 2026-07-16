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
for the wire contract if you're building against it. The phone app's own
pairing screen, where you paste the printed URL and it switches the tutor
and exam over to the desktop, ships in a later mobile release. Until then
the [phone app](18-the-phone-app.md) reviews decks fully offline.

## Security posture

This is plain HTTP on your local network. The bearer token guards against
someone stumbling onto the server by accident, not against a hostile
network: anyone already on your LAN who gets hold of the token can use it.
For anything beyond your own LAN, put alix behind a VPN or a reverse proxy;
alix itself will not grow TLS or accounts.
