# Time-Series Phase 31 Handoff

Date: 2026-08-24

Branch: `feature/timeseries-phase-0a`

Status: **Phase 0B is closed.** The maintainer made the final
Rust-versus-Spark call this session — Rust/DataFusion selected, the Spark
comparison path descoped outright — and the decision is landed as
`docs/TIMESERIES_PHASE_0B_DECISION.md`, with every open thread it
dispositioned: #5 and #6 closed as not-planned, a new Phase 7 umbrella
issue [#22](https://github.com/jeromebanks/cubism-rs/issues/22) filed for
the carried measurement work, #1/#2/#4 kept open and retargeted to Phase 7,
and `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` updated in three places (0B's
section status line, the snapshot table row, Phase 7's resolved
Spark-fallback bullet). This was a decision/documentation session: **no
source files changed**, so no test battery was re-run — the standing green
counts are Phase 30's (40+1 in `cubism-iceberg`, 216/2 workspace). (Naming
note, carried forward from every prior handoff in this series: this file's
number is a *session-slice* number, not a plan phase number.)

Context for why now: RESULTS' 2026-08-12 "Decision" had already selected
DataFusion/Rust for the axis-1 path but left six gate items open, and the
plan's status lines still read "decision pending." The maintainer resolved
the remainder by judgment call; the advisor round's job was making sure
the durable record doesn't overclaim what that judgment rests on.

**Superseded by:** `docs/TIMESERIES_PHASE_32_HANDOFF.md` — first of the
finish prompt's three demo slices: `/api/series` widened to scalar
measures (roadmap Milestone 14); this doc's deferred item 3 (main line
per finish-line discussion) is what that work plan now executes, starting
with the demo.

## What this session built

- **`docs/TIMESERIES_PHASE_0B_DECISION.md`** — the closure record:
  the decision and its three supports (measured axis-1 record; purpose
  overtaken by the engine being built through Phases 1–4 + 5-functional;
  unresolved items splitting cleanly into Spark-comparison-specific vs
  Rust-measurement tasks); unchanged scope honesty (no cluster claims);
  disposition of all six RESULTS gate items plus #6's fourth finding;
  artifact fates (spark-adapter frozen in-tree, bench stays active,
  oracle-loss acknowledged with digest-pinning assigned to #22); the three
  plan edits; and an explicit note that full Spark retirement is
  *stricter than* FEASIBILITY's primary branch, which it quotes verbatim.
- **Issue dispositions** (in the advisor-required order — umbrella first):
  - Filed [#22](https://github.com/jeromebanks/cubism-rs/issues/22), the
    Phase 7 performance-hardening backlog: dense occupancy at scale,
    compression codecs, the bench's write-inclusive timing fix, pinning
    the cross-engine golden digests before the frozen adapter loses
    provenance, and the 10–100M Rust measurement.
  - Closed **#6** (not planned): finding 3 mooted by descope; findings
    1/2/4 carried to #22.
  - Closed **#5** (not planned): root-causing a descoped harness's kill
    signal is no longer justified effort; the close comment states
    explicitly this reflects a descope, not an established cause.
  - Commented-and-kept-open **#1**, **#2**, **#4**, each retargeted to
    Phase 7 with its live state corrected (#1's title describes an already
    -fixed ceiling).
- **Plan edits**: 0B section status line → decided-with-pointer; snapshot
  table row rewritten (dropping its false "the Spark adapter itself is
  unbuilt" fragment — the adapter ran at four scales including twice at
  25M); Phase 7's "retirement point for the Spark maintenance fallback"
  struck through as resolved by reference.

## Advisor round (before landing)

The pre-landing advisor review returned ten findings, all addressed:

- **[P1] Misquoted governing text**: the draft claimed FEASIBILITY's
  primary branch as authority for full Spark closure, dropping its "and
  keep Spark as an optional maintenance boundary" clause. Fixed: the doc
  now quotes the branch verbatim and states plainly that the decision
  takes the Rust half *and additionally* retires that clause.
- **[P1] Wrong kill-timing claim**: the draft said kills hit "jobs running
  20+ minutes"; NOTEBOOK Entry 12 records kills within minutes of start
  too. Fixed per Entry 12.
- **[P2s]**: universal-quantifier overclaim on "every residual unknown
  clusters on local-host artifacts" rescoped; #1 described by stale title
  instead of live state (streaming rewrite landed) — fixed in doc and
  retarget comment; #6 finding 4 undispositioned — added to table and #22;
  oracle-loss unacknowledged — added with digest-pinning task; plan-edit
  scope incomplete — extended to all three edits plus the "unbuilt"
  falsehood warning.
- **[P3s]**: "Phases 1–5" → "Phases 1–4 plus Phase 5's functional half";
  "harness crashes" → "runs killed by an unexplained external signal"
  (wrapping was ruled out); oracle-role citation moved from FEASIBILITY to
  HARNESS/#1-finding-3.

Also adopted per advisor mechanics: umbrella filed before any close so
carried items never dangle; #1/#2/#4 kept separate rather than absorbed,
matching the repo's individual-tracking convention.

## What was actually verified

No code changed; the test battery was deliberately not re-run (standing
counts from Phase 30 hold). What was verified: every factual claim in the
decision doc against RESULTS/HARNESS/NOTEBOOK via the advisor round
(scales, failure rates, expansion ratio, kill timing, gate lists); issue
states after the dispositions (`gh`: #5, #6 closed; #22 open; #1/#2/#4
open with retarget comments); and the plan doc renders coherently with the
snapshot table (0B row now says Decided).

## GitHub issues touched

- **[#22](https://github.com/jeromebanks/cubism-rs/issues/22)** — filed
  (Phase 7 backlog umbrella).
- **[#5](https://github.com/jeromebanks/cubism-rs/issues/5)**,
  **[#6](https://github.com/jeromebanks/cubism-rs/issues/6)** — closed,
  not planned, evidence-linked.
- **[#1](https://github.com/jeromebanks/cubism-rs/issues/1)**,
  **[#2](https://github.com/jeromebanks/cubism-rs/issues/2)**,
  **[#4](https://github.com/jeromebanks/cubism-rs/issues/4)** — kept open,
  retargeted to Phase 7 with live-state corrections.

## Deferred / not done this session

1. **Deleting spark-adapter/** was considered and rejected — it stays
   frozen in-tree (see DECISION's artifact section). Any deletion is its
   own explicit future decision.
2. **Executing anything on #22** — this session only created the backlog;
   the highest-leverage items are probably compression and the bench
   timing fix (which gates trusting everything else measured through the
   bench).
3. The main line remains **#16** (engine link) per the finish-line
   discussion this session grew out of, then Phase 6, then #10/#12. #8
   re-test spike (DataFusion 55 shipped mid-August 2026) also still
   pending.

## Worktree state

Committed and pushed to `feature/timeseries-phase-0a` (see `git log`;
hashes deliberately not hardcoded here).

- New: `docs/TIMESERIES_PHASE_0B_DECISION.md`,
  `docs/TIMESERIES_PHASE_31_HANDOFF.md` (this file).
- Modified: `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` (three edits),
  `docs/TIMESERIES_PHASE_30_HANDOFF.md` (`Superseded by` line),
  `docs/handoff_latest.md` (symlink retarget).
- Untouched: every source file and test in every crate;
  `spark-adapter/` (deliberately frozen).

Deliberately uncommitted, prior-session convention: `.serena/`,
`examples/web_analytics_demo/events.csv`,
`.claude/skills/timeseries-slice/SKILL.md` pre-existing edits,
`.claude/skills/spark-setup/SKILL.md` (now historical per the decision,
left in place), `docs/CODEX_TELEMETRY_ROADMAP.md`.

## Tests (unchanged from Phase 30: 40 passed + 1 ignored in cubism-iceberg; 216 passed / 2 ignored workspace)

## Verification performed

```text
# No source changes — battery intentionally skipped; standing counts:
#   cargo test -p cubism-iceberg        -> 40 passed, 1 ignored (Phase 30)
#   cargo test --workspace --exclude cubism-py -> 216 passed, 2 ignored (Phase 30)
gh issue list --state open              # verified post-disposition state
```

## Primary files

- [`TIMESERIES_PHASE_0B_DECISION.md`](TIMESERIES_PHASE_0B_DECISION.md)
  (the closure record — start here)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
  (status line, snapshot row, Phase 7 bullet)
- [`TIMESERIES_PHASE_30_HANDOFF.md`](TIMESERIES_PHASE_30_HANDOFF.md)
  (prior handoff, superseded by this one)
- Issue [#22](https://github.com/jeromebanks/cubism-rs/issues/22) (filed)
- Issues [#5](https://github.com/jeromebanks/cubism-rs/issues/5),
  [#6](https://github.com/jeromebanks/cubism-rs/issues/6) (closed),
  [#1](https://github.com/jeromebanks/cubism-rs/issues/1),
  [#2](https://github.com/jeromebanks/cubism-rs/issues/2),
  [#4](https://github.com/jeromebanks/cubism-rs/issues/4) (retargeted)
