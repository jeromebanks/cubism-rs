# Cubism documentation index

`docs/` holds three different kinds of document, and they do not carry equal
weight. This index says which is which, so that a reader — or a fresh agent
session — does not have to infer authority from a directory listing in which
roughly forty-five retired time-series handoffs outnumber everything else.

Three layers, in descending authority:

| Layer | Where | How to treat it |
|---|---|---|
| Authoritative | repository root and [`sdlc/`](sdlc/) | Describes what runs today. Follow it. |
| Current reference | the user-facing pages below | Describes the library as built. Trust, but code is final. |
| History | everything else, `TIMESERIES_*` foremost | Evidence, not instructions. Do not load unless a current slice links to it. |

## Authoritative — what runs today

Process and project definition live at the repository root and in
[`sdlc/`](sdlc/). Where any document in `docs/` disagrees with these, these win.

- [`../README.md`](../README.md) — what Cubism is and how to build it.
- [`../SDLC.md`](../SDLC.md) — the delivery policy: slices, review, merge gate,
  human milestone gate. Authoritative for every crate and binding.
- [`../AGENTS.md`](../AGENTS.md) — the agent-facing operating rules.
- [`../CLAUDE.md`](../CLAUDE.md) — Claude Code's entry point into the same
  workflow.
- [`sdlc/HUMAN_TUTORIAL.md`](sdlc/HUMAN_TUTORIAL.md) — the lifecycle explained
  for a human operator.
- [`sdlc/design.md`](sdlc/design.md) — why the lifecycle is shaped this way, and
  the maintenance boundaries.
- [`sdlc/overview-slides.html`](sdlc/overview-slides.html) — the same operating
  model as a browser deck.
- [`sdlc/milestone-manifest.example.json`](sdlc/milestone-manifest.example.json)
  — the template a checkpoint manifest is written from.

## Current reference — the library as built

These describe Cubism itself and are maintained alongside the code.

- [`architecture.md`](architecture.md) — crate layout and how the pieces fit.
- [`concepts.md`](concepts.md) — the model: sketches, merges, exactness.
- [`sketches.md`](sketches.md) — the sketch types and their guarantees.
- [`spec-reference.md`](spec-reference.md) — the serialized format.
- [`cli.md`](cli.md) — the command-line surface.
- [`python.md`](python.md) — the Python bindings.
- [`extending.md`](extending.md) — adding a sketch or an adapter.
- [`serving.md`](serving.md) — query-time serving.
- [`scaling.md`](scaling.md) — behavior as data grows.
- [`high-cardinality.md`](high-cardinality.md) — the high-cardinality regime.
- [`dogfooding.md`](dogfooding.md) — using Cubism on Cubism.

## History — evidence, not instructions

Everything below records how the project got here. It is preserved because the
technical evidence in it is useful, and for no other reason. It does not
describe a process that is still followed, and several documents contradict the
current one.

`SDLC.md` retires this material explicitly, under "Retired time-series process":
the shared `feature/timeseries-phase-0a` lane completed its purpose in PR #24,
and all future time-series work uses the ordinary slice lifecycle. Read one of
these only when a current slice links to it specifically.

- **`TIMESERIES_*.md` (~45 files)** — the phase handoffs, roadmap, architecture,
  feasibility study, and Phase 0A/0B benchmark results from the time-series
  effort. Retired as instructions; retained as measurements and rationale.
  `handoff_latest.md` is a symlink into this set and inherits its status.
- [`phase-reviews/`](phase-reviews/) — per-phase review notes from the same
  effort. Same status.
- [`sdlc-process-notes.md`](sdlc-process-notes.md) — notes predating `SDLC.md`.
  Superseded by it.
- [`CODEX_TELEMETRY_ROADMAP.md`](CODEX_TELEMETRY_ROADMAP.md) — an earlier
  telemetry plan, unexecuted.
- [`PHASE_0A_RESULTS.md`](PHASE_0A_RESULTS.md) — Phase 0A measurements.
- [`nightshift-review/`](nightshift-review/) — a review of the Nightshift design
  package and the transition plan the current lifecycle work follows. Decisions
  of record, not a process description.

## Where new documents go

A document describing what runs today belongs at the repository root or in
[`sdlc/`](sdlc/). A document describing the library as built belongs in `docs/`
alongside the current-reference pages above, and gets a line in this index. A
document that records a finished investigation belongs under History, and should
say on its first screen that it is history.
