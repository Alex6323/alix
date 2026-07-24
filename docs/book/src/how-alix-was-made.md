# How `alix` was made

`alix` is an AI-built project with a human maintainer. That description is more
accurate than either "hand-written by a human" or "made autonomously by AI."
Models have produced a large share of the implementation, tests, documentation,
and design drafts. The maintainer chooses the problems, sets the constraints,
challenges the design, checks the result, and decides what enters the project.

This chapter follows the same rule: it was drafted with AI assistance for the
maintainer to review. It does not pretend to be purely human-authored.

## How a change happens

A typical change begins as a conversation, not as a model receiving the whole
repository and independently deciding what to build.

1. **The human defines the job.** The maintainer supplies the need, product
   boundary, and important constraints. For a substantial feature, that becomes
   a written specification and implementation plan before code changes.
2. **The coding agent investigates.** It reads the repository, finds the relevant
   contracts and tests, proposes a design, and raises conflicts or missing
   decisions. It then edits code, tests, documentation, and examples together.
3. **Deterministic gates check the mechanics.** Formatting, linting, unit tests,
   integration tests, contract snapshots, and end-to-end tests exercise behavior
   that can be checked repeatably.
4. **The human reviews the result.** The maintainer reviews the product behavior,
   important design choices, the diff, and any visual result. They may reject the
   approach, narrow the scope, request another implementation, or approve a
   commit. The agent is not allowed to commit or push merely because its tests
   pass.
5. **Release checks examine the candidate.** Prompt changes receive live-model
   calibration. Public documentation and screenshots receive a read-only
   semantic audit against the implementation. Release artifacts have their own
   platform checks.

The loop is deliberately conversational. The model contributes speed, breadth,
and persistence; the human supplies intent, taste, risk tolerance, and the
go/no-go decision.

## What the models produce

During development, a coding model may write almost any repository artifact:
Rust and Dart code, HTML and CSS, tests, specifications, plans, runbooks,
changelog entries, and first drafts of prose like this chapter. It may also run
the project's tools and explain the evidence it used.

That development-time model is separate from the AI backends that `alix` calls
as a product. The tutor, deck generator, trace generator, and examiner invoke a
model CLI selected by the user. Replacing that runtime backend does not rewrite
the application, and using `alix` for ordinary offline review does not require a
model at all.

Generated output is not accepted because it sounds confident. Repository
conventions push behavior into the shared Rust library, preserve stable card
identities, require tests around important logic, and keep public contracts
written down. Those constraints give both a human and another agent something
concrete to inspect.

## What the human reviews

The maintainer owns the decisions a test cannot make:

- whether a feature belongs in `alix` at all;
- whether the interaction stays calm and understandable;
- whether a plan protects user data and stable file formats;
- whether a screenshot or manual run actually feels right;
- whether the explanation matches the intended product;
- whether the remaining risk is acceptable.

This is not the same as independent review by a second engineer, and the project
should not imply that every generated line has received a deep manual audit.
AI-assisted development can produce changes faster than one person can study
them. The response is to make review evidence durable: small commits, explicit
plans, focused tests, source-linked decision decks, and release audits. These
improve traceability; they do not turn a single maintainer into two reviewers.

## What tests prove, and what they do not

`alix` separates deterministic software correctness from model-behavior quality.

The blocking `make check` gate uses ordinary Rust tests and a fake model CLI. It
can prove that known inputs produce expected state transitions, errors are
handled, contracts remain compatible, and AI plumbing behaves correctly for
canned responses. CI can repeat those claims without network access or model
variance.

Live-model calibration asks a different question: do current prompts produce
useful, appropriately strict results? It is costed and non-deterministic, so it
is run deliberately for prompt changes rather than pretending to be an ordinary
unit test. The documentation audit is also a deliberate model call: it compares
public text and images with current implementation evidence before a release.

Neither layer proves that every future model response will be good. A green
suite also cannot prove that the product decision was wise, the architecture
will remain maintainable, a migration is operationally safe, or a human
understands every changed line. Manual review, calibration, release practice,
and feedback from real use remain necessary.

## Attribution and responsibility

The repository history preserves AI assistance with `Co-Authored-By` trailers.
That is attribution, not a transfer of responsibility. A model cannot own a
release, respond to a data-loss incident, or decide what risk another person
should accept. The human who approves and publishes a change remains responsible
for it.

The useful standard is therefore not "no AI touched this." It is: the role of AI
is disclosed, important decisions are inspectable, repeatable claims have
repeatable tests, uncertain model behavior is evaluated as such, and a human
makes the final call.
