# How `alix` was made

`alix` was built entirely through conversation. There was no team and no
hand-written codebase in the usual sense: one person — its author — and Claude
(Anthropic's model, through Claude Code) talked the whole thing into being, line
by line. The ideas, the product decisions, the design arguments, and every "no,
that's wrong" came from the human. The code, the tests, and most of the prose —
including this book — were drafted by the model under that direction.

`alix` is a ground-up rewrite of an earlier flashcard tool — one the author wrote
the old way, by hand, before AI could do this kind of work at all. The plain-text
format these decks use is the author's own invention, from that first tool and
predating the AI work entirely; everything since is built on it. The rewrite kept
the format but carried no code across — no import, no old progress, a clean start.
That the same person built the first version by typing every line and the second
by conversation, while the format stayed theirs the whole way, is, in small, the
shift this tool is premised on.

We say so plainly because it's the same bet the tool itself makes. `alix` holds
that in an AI-assisted world the scarce human contribution is *judgment* —
knowing what to build, what's actually good, when an answer only looks right —
and that AI is best used to do the legible work and to be argued with. The
project was made the way it tells you to learn: the human brought the
understanding and the taste; the model brought the throughput and was tested
against the human's standard.

That also means the honest disclaimer applies: a model wrote much of this code.
What kept that from being a liability is the thing this book keeps insisting on —
nothing counts until it's checked. Designs were argued rather than accepted, the
test suite and the lints are the floor, and the author uses `alix` to learn real
material, which is what exposes whatever only *looked* finished.

None of this is hidden, because hiding it would contradict the point. The
codebase is small and plain on purpose — if you want to know how something
really works, the honest answer is always: read it.
