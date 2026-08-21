# 0036: A heading is study structure, not deck metadata

- Status: Proposed
- Evidence: none, proposed; literal markers replace this line at
  acceptance, alongside the implementation
- Recorded: 2026-08-21
- Retrospective: No

## Context

A deck file's first `# ` heading is currently the deck title, and the
prose between it and the first card is the description; a second `# `
heading in that preamble region silently joins the description
(`src/parser/mod.rs:817`), while a `# ` line after cards begin is
ordinary card content. `###`/`####` lines are ordinary answer content
everywhere. Three pressures broke this arrangement at once. First,
the learner-facing one: cards arrive without their surroundings, so
an answer cannot be encoded, only pattern-matched; the deck author
has no way to prepare the learner for a question. Second, authors
need cards that presuppose other cards; the only gating grain is the
whole deck (`requires:` plus its exam). Third, the title role makes
`#` unusable for anything else, and the silent-join bug is unfixable
without deciding what a second `#` means.

The maintainer ruled the direction on 2026-08-21; the working spec
(`docs/specs/2026-08-21-section-context-spec.md`, local and
gitignored, summarized here so this record stands alone) carries the
full decision table and was adversarially reviewed three times the
same day.

## Decision

**The deck body belongs to the learner's material; deck metadata
belongs to the frontmatter.** Concretely:

1. **`# ` headings are section context.** A `# ` heading, first or
   not, provides context for the cards after it until the next `# `;
   the heading text plus the prose before the section's first block
   is the context, delivered on the card transfer object as a
   parallel raw/runs/units triple named distinctly from the existing
   effective-question `context` family (which cloze, region and
   table cards already require), projected under that family's law,
   and shown only on a review toggle. Nothing is shown by default.
2. **`### ` and `#### ` are sub-cards.** Parentage is one stack
   rule: a front of depth N closes every open chain at depth >= N,
   and a sub-card front parses only while the most recently opened
   front one level shallower is still open; anything else, skipped
   levels, orphans, five and more hashes, empty headings, is a
   line-numbered parse error. A sub-card is invisible to every
   selection, count and status surface until its parent block has
   graduated on the RECALL schedule (ADR 0035: left the learning
   stage), all expanded units counted, independent of the session's
   requested depth. The predicate is monotonic (relearning still
   counts as graduated, `src/store.rs:52`), so no re-lock machinery
   exists; an explicit reset of the parent re-locks. Heading depth
   and parentage never enter card identity: a re-headed card keeps
   its minted token and history, and a directive or id comment on a
   section line is a hard error rather than a silent severance.
   Structure never crosses the deck/sidecar boundary: personal
   files parse with sections and sub-cards disabled.
3. **Support has a ruled price.** Viewing context writes nothing: no
   event, no schedule change, no endpoint (the toggle is
   client-local, so ADR 0035's departure law gains no row). A future
   show-hint action caps that review's grade at `Partial` (FSRS
   `Hard`): a hinted pass still passes and still graduates, but pays
   in interval growth. The AI split is deterministic: the TUTOR always
   sees a card's section context; augmentation, the exam and its
   remediation never do. A tutor-drafted personal card is a
   context-free OUTPUT of that context-aware prompt, so the draft
   prompt demands a self-contained question and answer, because the
   saved card is top-level in the sidecar and carries no section
   context. **No fingerprint changes**: content and block
   fingerprints keep their existing inputs, so a heading edit
   invalidates nothing and moving a card between sections keeps
   every cached artifact. Augmentation is excluded because a
   distractor is a near-miss of the answer and the prompt already
   carries the answer, while section context is by construction the
   part that is not the answer; and because these stamps are
   per-card while one section spans a whole deck, so a reworded
   heading would have regenerated an entire deck's distractors.
4. **`title:` and `description:` are frontmatter keys**, frozen
   jointly with the planned frontmatter-only deck listing, which
   inherits them. `name:` was rejected because the published API
   already uses `name` for a deck's selector while display text
   travels as `label`, and because workspace manifests already say
   `title`/`description`. A third, classification key is deliberately
   not frozen: adding a key later is additive, and its name follows
   from a decision not yet made, whether classification is a picker
   filter or a cross-deck study scope. The previously parsed and
   unconsumed `tags:` key is removed. The display chain is
   `title:`, else the condensed `trace:`, else the filename stem;
   there is no heading fallback. The preamble concept is deleted.

## Consequences

Every existing deck is affected by the metadata move. The
maintainer ruled the disposition: old decks RE-MEAN SILENTLY (the old
`# Title` becomes a section heading, the drawer empties, the list
falls back to the filename) and no doctor finding is added, because
444 of the 621 initialized decks measured on 2026-08-21 are
filename-named by choice, so such a finding would warn forever on
sanctioned state. The alternative, requiring `title:` whenever `id:`
is present (the shape of the existing `format-version` rule,
`src/parser/frontmatter.rs:189`), would have failed every old
initialized deck loudly as ordinary invalid input; it was rejected
for making the key mandatory forever and forcing synthesized titles
onto those 444 decks. Neither option recognizes the old format. Neither
recognizes the old format; conversion is disposable tooling outside
the repository, and it must never rewrite `assets/` source
snapshots, whose sha256 filenames address their content. No deck
file in either corpus uses column-0 `###`/`####` (measured
2026-08-21, deck files only), so the sub-card grammar itself breaks
nothing existing; no progress is touched either way, because
identity lives in stamped comments.

The card transfer object gains the section-context field and
`locked`, loses `preamble`, and the heatmap/topology/crumb cells
widen from a bare tier string to tier plus locked, keeping the
seven-tier vocabulary intact; the API contract, snapshots, codegen
corpus, both web clients, the mobile bridge and the generator's
card-shape guidance move in the same change. A single lock-aware
eligibility predicate feeds every count, status and queue reader, so
no surface can claim a due card that cannot be served.

Card-level gating coexists with deck-level `requires:` at different
bars (Recall graduation vs exam-Finished); the manual must present
them as one concept, dependency, at two grains.

Table scope changes: any structural heading now closes an open
table (previously a hard trailing error for `# `/`### `/`#### `);
a titled table acts as a depth-2 parent whose rows are its units,
and a sub-card under a zero-row titled table is a parse error
rather than a vacuously unlocked child; an untitled table closes
the chain.

## Alternatives considered

**Dual-use files** (a document that is also a deck, cards keyed by
machine lines): explicitly rejected by the maintainer in the ruling
conversation; it dissolves the format's one-job clarity and drifts
toward the notes app that the project guide's scope list (what alix
is NOT) excludes.

**Always-on context display**: rejected in the same conversation;
the learner does not always need the context, and permanent chrome
fails the project guide's UI noise rule (only what the learner needs
right now).

**Hint as a fail or a schedule re-anchor**: rejected for the
`Partial` cap. A fail overstates (the learner did retrieve, aided);
a re-anchor adds machinery and a second cost model; the cap reuses
the existing grade vocabulary and self-corrects on the first clean
pass.

**Keeping the H1 title as a fallback**: violates the pre-1.0
no-old-format rule and keeps two homes for one fact.

**ANY-unit graduation for expanded parents**: defeats the
prerequisite purpose; one ungraduated hole means the fact is not
yet known.

**Reusing the existing `context` wire field for section context**:
rejected during review; that field is the required effective
question of cloze/region/table cards, rendered unconditionally by
both clients, and overloading it would hide real questions and
misroute the drawing surface.

**Feeding section context to the augmenter, and stamping it into
the content fingerprint**: proposed in review and rejected by the
maintainer. The distractor prompt already carries the answer, and a
distractor is a near-miss of the answer, so context adds little the
answer does not already fix; and the stamps are per-card while a
section spans a whole deck, so a reworded heading would regenerate
every distractor in it. The distractor-quality problem this was
reaching for has a different cause: augmentation is never given the
deck's own source material, which the exam does receive. Tracked
separately.

**Including section context in the block fingerprint**: the first
review recommended it and the second retracted it against the live
consumers; a context-bearing authored key could never match the
context-free candidates the dedup exists to catch, so every
sectioned deck would duplicate instead of deduplicate.

## Compatibility

Pre-1.0, so no migration and no old-format recognition.
`format-version` stays 1: a break rewrites old decks outside the
repository rather than bumping. Whichever old-deck disposition the
maintainer picks, the failure or re-meaning is ordinary
current-design behavior, never a recognized legacy path.

## Security

No trust boundary changes. No new endpoint, no new input channel:
section context is same-document text already inside the parsed
grammar, delivered over the existing card object. Gating removes
cards from selection; it exposes nothing.

## Verification

Inline summary (the local spec carries the full law list): a grammar
matrix over depth, spacing, position (including table scope, divided
fronts, sidecars, fences, escapes); identity preservation across
re-heading and across init-stamping a sectioned unstamped deck; the
gating law over Recall states, all seven tiers, expansion units and
reset; agreement of every count/status/queue surface on one locked
fixture in every order mode; separation of section context from the
effective-question field with the kids client rendering no new aid;
no context route in the route table; no exam prompt builder
accepting deck-body input, and no augment prompt builder accepting
one either; the tutor prompt carrying the labeled block and the
sectioned draft prompt carrying the self-containment instruction,
with a round-trip minting a top-level context-free personal card;
fingerprints, ids and dedup behavior unchanged for identical cards
under different headings and for a card moved between sections (a
sectioned authored card still deduplicates its remediation
candidate); the zero-row titled-table error; frontmatter validation,
id-splice survival, doctor findings with line numbers; contract
snapshot regeneration with a formatted-section projection row.

## Reversal

Evidence that would justify replacing this: sub-card gating
measurably stranding learners (cards locked for weeks behind a
parent the learner cannot graduate), or section context going unused
while its grammar cost stays. Reversal restores flat `##`-only decks
by demoting sub-card fronts and dropping the context field; identity
survives either direction because depth never entered it.
