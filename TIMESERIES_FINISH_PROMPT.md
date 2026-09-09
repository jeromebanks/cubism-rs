# Prompt: finish the timeseries implementation (historical)

> **Retired:** PR #24 merged this branch. Do not execute this prompt or resume
> the shared-branch workflow. Use [`SDLC.md`](SDLC.md) and a bounded
> `work-slice` issue for future time-series changes.

You are continuing the time-series implementation of **cubism**
(`/Users/jeromebanks/dev/cubism_saas/cubism`, branch
`feature/timeseries-phase-0a`). Goal: drive the remaining timeseries work
to functional completion, ending with a **working visual demo**: a graph
of XUnit time series served from the web-analytics demo data.

## Read first (in this order)

1. `.claude/skills/timeseries-slice/SKILL.md` — the workflow this branch
   has used for 31 straight sessions: advisor-before-code, bounded slices,
   full verification battery, numbered handoff docs, evidence-linked
   GitHub issues. If your environment has no `advisor()` primitive, use a
   blank-context subagent (it must not inherit your session context) for
   both the pre-code design check and the post-commit diff review.
2. `docs/handoff_latest.md` → currently Phase 31 — latest state.
3. `docs/TIMESERIES_PHASE_0B_DECISION.md` — Rust/DataFusion is decided;
   the Spark path is closed. Do not reopen it.
4. `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` — authoritative phase spec;
   its top carries an accurate status snapshot table.
5. `docs/TIMESERIES_ROADMAP.md` — all 18 tracked milestones are Done; new
   work gets new roadmap milestones (follow how Phase 4's were added).
6. Open issues: #22 (Phase 7 backlog umbrella), #16 (engine link — the
   big one), #8 (DataFusion TableProvider), #9, #10, #11, #12, #14, plus
   retargeted #1/#2/#4. Closed history (don't reopen): #5, #6, #17, #18,
   #20, #21.

## Hard rules

- One bounded slice at a time ("one integration test plus supporting
  code" sizing). Advisor reviews each pick and each landed diff; fixes go
  in separate follow-up commits; push per slice.
- Verify every line-number citation mechanically (`rtk proxy grep -n` or
  Read with offset) — never from memory, never `~`-approximated.
- Before any `rustfmt --edition 2024 <file>`: run `--check` first. Most
  files report pre-existing diffs under this toolchain (issue #3); never
  run bare rustfmt on such files — hand-write matching style. Verify fmt
  diffs by reading them, not by grepping for keywords (Phase 30's lesson).
- Full battery every code slice: `cargo test -p cubism-iceberg`,
  concurrency + durability legs, clippy `-D warnings` on touched crates
  and workspace, `cargo test --workspace --exclude cubism-py`. Capture to
  files, grep results, hand-sum totals.
- Handoff doc per session: `docs/TIMESERIES_PHASE_<N>_HANDOFF.md`; next N
  is 32 (derive from `readlink docs/handoff_latest.md`, never glob);
  supersede the prior handoff; retarget the symlink; commit explicit
  paths only (never `git add -A`; `.serena/`,
  `examples/web_analytics_demo/events.csv`, `.temporal_build/` stay
  uncommitted).
- File GitHub issues for anything deferred or newly discovered; closing an
  issue requires a landing comment citing evidence (doc/handoff/commit).

## Work plan (ordered; stop cleanly at any boundary)

### 1. The demo — do this first, it is the user-visible payoff

Make it possible to actually SEE a graph of XUnit time series from the
web-analytics data:

- Exists already: `examples/web_analytics_demo/` temporal variant —
  `generate_temporal_events.py`, `web_analytics_temporal.yaml`,
  `build_temporal_demo.sh` (drives `cubism temporal-build` +
  `iceberg-build`; republishes day 2026-04-07 as revision 2 after a late
  correction), `query_temporal_demo.sh`.
- `crates/cubism-serve/src/series.rs` implements `/api/series`, but
  deliberately narrow: single XUnit selector, AverageState measures only,
  request supplies the `(window_id, bucket_start)` list itself.
- Missing: (a) widening `/api/series` enough for real web metrics
  (count-style measures; multiple selectors if cheap — respect the
  module's documented narrowing rationale and extend it consciously);
  (b) window/bucket discovery for clients (listing endpoint or
  convention); (c) a **time-series line chart in the serve dashboard**
  fed by the API; (d) wiring `build_temporal_demo.sh`'s output into
  `cubism serve`, documented in `examples/web_analytics_demo/README.md`
  and `docs/serving.md`.

Done means: run the two scripts, open the dashboard, see a rendered
time-series graph of at least one measure across the published days,
with the revision-2 correction visible. May legitimately span 2–3 slices
(API widening / chart / docs-wiring); land them separately.

### 2. Durability tail sweep (small items; close them out)

- **#9**: aggregate state blobs have magic+version framing but no
  checksum. Add one; mind the versioned byte-format contract
  (`docs/sketches.md` conventions apply to state blobs too).
- **#14**: retry loop (`is_retryable`/`backoff`/`MAX_TX_ATTEMPTS`) has
  zero CI coverage under shipped BEGIN IMMEDIATE config — see
  `tests/concurrency.rs`'s forced-contention precedent for how to force
  it.
- **Guard hoisting** (Phase 30 deferred item): hoist
  `CorrectionCoordinator::execute`'s generation guard above the append
  branch so refused replays don't commit orphaned Iceberg snapshots;
  fresh claims pass trivially.
- **AwaitingAppend repro test** (Phase 30 deferred item): replay after
  claim + intervening correction-and-rollback, asserting
  `WindowChangedSinceClaim`.
- **SQLite refusal leg** (Phase 30 deferred item): coordinator-level
  refusal test against the SQLite backend, mirroring #20's precedent.

### 3. #8 re-test spike

DataFusion 55.0.0 shipped mid-August 2026 (#8 was filed waiting on
53/54 convergence). Try unifying cubism's DataFusion with whatever
`iceberg`/`iceberg-datafusion` pin now, behind a `TableProvider` for
temporal tables (plan Phase 5's SQL half). Either land it (Phase 5 fully
done) or write down exactly which dependency pins still conflict so the
wait is concrete. One slice either way.

### 4. #16 — the engine link (the last big functional gap)

Corrections currently require the caller to identify affected windows and
supply already-rebuilt states — nothing can compute a correction
end-to-end. Build the link crate/module connecting `cubism-datafusion`
and `cubism-iceberg`: source-event time range → affected windows
(`TemporalSpec` knows the window math) → rebuild corrected states via the
existing temporal build path → feed `CorrectionCoordinator::execute`.
Roadmap-milestone this like Phase 4 was (several slices). Expect real
integration findings once the durability machinery gets its first genuine
caller — that is the point of doing it before anything else big. This
closes plan Phase 4's narrowed half ("identify affected windows from
event time") tracked since Milestone 4.

### 5. Phase 6 — rolling comparisons / trend inputs

Per the plan's Phase 6 section. Design decision FIRST (it is flagged
there): amortized sliding-window monoid vs dyadic summaries vs measured
max window width for non-subtractable sketches — never O(n·w) naive
merging, never summing displayed distinct counts. Then lag /
period-over-period / rolling windows over `/api/series`.

### 6. As capacity allows: #22 items (Rust-only measurements)

Priority order inside #22: bench timing fix (write-inclusive timing
contaminates everything else measured through the bench), compression
codecs (21–30x UNCOMPRESSED expansion is the biggest known lever),
dense-occupancy at scale, golden-digest pinning, 10–100M run (#1's
remainder). Fold #2 (aggregate-spill verification) and #4 (unbounded
`.cache()`) when touching adjacent code.

## Explicitly out of scope unless everything above lands

#10 (object store + Iceberg maintenance) and #12 (Postgres control
store) are production substrate for cloud deployment, not functional
completion — do them only if the list above is genuinely exhausted.
Do not start them early: infrastructure ahead of a functional gap is how
this branch stays "almost done" forever.

## Definition of done for the whole engagement

Functional completeness = a source-event change flows end-to-end to a
corrected, published, queryable window without a human computing the
rebuild (#16), the result is chartable (#1 demo), range queries report
coverage/exactness truthfully (Phase 5 functional criteria), rolling
comparisons work (Phase 6), and every issue touched along the way is
closed with evidence or tracked with an issue. When you stop for any
reason: leave a handoff doc, pushed branch, clean status report with the
roadmap/issue snapshot delta per SKILL.md step 9.
