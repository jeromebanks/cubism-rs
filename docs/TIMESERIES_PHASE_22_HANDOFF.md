# Time-Series Phase 22 Handoff

Date: 2026-08-16

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 10b-2** (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5
Milestones" section) — `SeriesResponse`, the "wiring" half of the roadmap's
deferred "Milestone 10b" note that Milestone 10b-1 (the merge primitive
alone) deliberately left undone. `SeriesResponse::new` takes a real
`CoveragePlan` plus one already-read batch list per segment and produces
one `SeriesPoint` per segment: bucket range, `is_exact`, a materialized
`value: Option<f64>` (via `merge_average_column` and
`AggregateState::present`), and the segment's `published`/`missing`
provenance. One new module
(`crates/cubism-datafusion/src/series_response.rs`, six unit tests) plus
one new integration test wiring a real `ResolutionPlan` -> `CoveragePlan` ->
`SeriesResponse` against two real published windows in an in-memory
`cubism-iceberg` catalog. Milestone 10b-2 added and flipped to `Done` in the
roadmap; two stale cross-references in `series_merge.rs` and
`range_query.rs`'s own doc comments (each previously saying `SeriesResponse`
"is not yet added to the roadmap") corrected to point at the landed type
and milestone. No new GitHub issues filed.

Landing this milestone satisfies the roadmap's Phase 5 "done" condition
(criteria 772/775, narrow-closed the same way "Phase 4 done" narrow-closes
against #10/#17, with criterion 774 excluded and pointed at #8) — this is
the first time in this series that step 8a's cross-model phase review
applies, flagged explicitly at this session's own step 1 per Phase 21's own
deferred item 1. That review ran this session; see "GitHub issues touched"
and the linked review file below for its outcome.

(Despite the filename, this doc documents a session slice, not "Phase 22"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving,
narrow-closed by this session per the roadmap's own "Phase 5 done"
condition walk; the plan's Phase 4
(`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` Phase-4 milestone done but is not itself fully
done — see that roadmap's "Phase 4 done" section, unchanged by this
session, for what remains.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_21_HANDOFF.md` and confirmed the branch in sync
with `origin` (`git rev-list --left-right --count` reported `0 0`), then
listed all open issues (`#1`-`#18`, unchanged from Phase 21). Phase 21's
deferred item 1 named "Milestone 10b-2" as the natural next slice and
explicitly flagged it as the slice expected to trigger step 8a's
phase-close review — read the roadmap's Milestone 10/10b-1 entries and
"Phase 5 done condition" walk, plus `range_query.rs`, `series_merge.rs`,
and `AggregateReader::read_window`'s doc comment, before calling the
advisor, per the skill's step 1.

The advisor confirmed Milestone 10b-2 as the right next slice and reshaped
its scope before any code was written:

- **Cut the same way Milestone 10b-1 was cut.** The roadmap's own
  Milestone 10b-1 entry bundled four things under "Milestone 10b-2": (a)
  the `SeriesResponse` type; (b) wiring it to `CoveragePlan`'s `published`
  list via `merge_average_column`; (c) widening past `AverageState`; (d)
  promoting `cubism-iceberg` to a normal dependency. Only (a)+(b) is
  slice-sized per this series' "one bounded next code slice" convention.
  Taken: `SeriesResponse` stays pure/sync over caller-supplied batches,
  mirroring `CoveragePlan::new`'s own contract exactly (one entry per
  segment, same order, same length-mismatch rejection); `cubism-iceberg`
  stays a dev-dependency; only `AverageState` is handled.
- **Most of the plan's response shape (lines 708-716) is a projection of
  what `CoveragePlan` already holds**, not new computation:
  `source_resolution` = `coverage.resolution`; `bucket_start`/`bucket_end`
  = `segment.range`; `is_exact` = `SegmentCoverage::is_exact()`; the
  missing marker and revision provenance = the existing
  `published`/`missing` fields. The only new computation this milestone
  adds is one merged value per segment via `merge_average_column`.
- **Decide the phase-close question explicitly, not by scoping around
  it.** Checked against "Phase 4 done"'s own precedent
  (`docs/TIMESERIES_ROADMAP.md:399-435`): that section narrow-closes Phase
  4 with two of its four completion criteria not fully met, each pointed
  at a tracking issue (#10, #17) rather than treated as blocking. The same
  pattern applies here: the narrow (a)+(b) cut still satisfies criteria
  772 and 775 as the roadmap words them, even scoped to `AverageState`
  only; criterion 774 stays gated on #8, same as Milestone 10's own "Done
  when" already recorded. Decided and recorded in the roadmap's new "Phase
  5 done condition" text: Phase 5, as this roadmap defines it excluding
  #8, is done as of this milestone. The advisor's explicit warning against
  cutting scope further purely to dodge step 8a (since Phase 21 already
  made that move once) was heeded — the narrow (a)+(b) cut was chosen on
  its own merits (matching Milestone 10b-1's own precedent), not to avoid
  the phase-close trigger, and the trigger was accepted rather than
  deferred again.
- **`gap_policy` is a design decision to make explicitly, not let happen
  by accident.** `TemporalQuery.gap_policy` (`range_query.rs`'s
  `GapPolicy`) has existed since Milestone 8 but nothing read it until this
  session. Checked `AggregateState::present`
  (`crates/cubism-core/src/aggregate_state.rs:251-253`, `AverageState`'s
  impl): a zero-count `AverageState` (what `merge_average_column` returns
  for a segment with zero published windows) already presents as `None`,
  not `Some(0.0)` — so "no data" and "a real zero" were already
  distinguished before `gap_policy` enters the picture. Decided:
  `SeriesResponse::new` takes `gap_policy` as an explicit parameter (the
  originating query's value, caller-supplied — no `CoveragePlan` API
  change) and substitutes `Some(0.0)` for that `None` case only when
  `gap_policy` is `GapPolicy::Zero`; `GapPolicy::Missing` leaves it `None`.
  A segment with a genuine non-zero-count merge is never touched by this
  substitution regardless of policy — proven by a dedicated unit test (see
  "What was actually verified").
- **Pre-checked the Codex runtime and searched for an existing
  `SeriesResponse` before writing code**, per the advisor's instruction:
  `node "$CODEX_SCRIPT" status --json` resolved cleanly (`sessionRuntime.mode:
  "direct"`, no prior run); `grep -rn "SeriesResponse" crates/ docs/` found
  no existing type anywhere in the codebase — only doc-comment mentions in
  `range_query.rs`/`series_merge.rs` and roadmap/handoff prose describing
  it as not-yet-built, confirming no duplication risk.

Scope fence held per the advisor's confirmation: no widening past
`AverageState`, no `cubism-iceberg` dependency promotion, no per-point
state/error metadata (a decode/merge failure for any one segment fails the
whole `SeriesResponse::new` call via `?`, not a partial response with an
error marker on just that point — recorded as a deliberate scope cut in the
roadmap entry, not a gap discovered later).

Primary files changed:

- **`crates/cubism-datafusion/src/series_response.rs`** (new file, 288
  lines): `SeriesPoint` (`:46-57`), `SeriesResponse` (`:62-65`), and
  `SeriesResponse::new` (`:67-114`, the constructor at `:74-113`) plus six
  unit tests (`:188-287`) covering one exact fully-published segment, both
  `gap_policy` branches for a no-data segment, gap_policy not touching a
  real present value, the batches-length-mismatch rejection, and a
  partially-published segment's `published`/`missing`/`is_exact: false`
  propagation.
- **`crates/cubism-datafusion/src/lib.rs`** (modified, +2 lines): adds
  `pub mod series_response;` and re-exports `SeriesPoint`/`SeriesResponse`.
- **`crates/cubism-datafusion/src/range_query.rs`** (modified, doc comment
  only, `:104-112`): corrected the stale "`SeriesResponse`'s job (candidate
  ... not yet added to the roadmap)" note to point at the landed
  `crate::series_response::SeriesResponse` and Milestone 10b-2 by name.
- **`crates/cubism-datafusion/src/series_merge.rs`** (modified, doc comment
  only, `:19-22`): same correction — "No `SeriesResponse` type" bullet now
  points at the landed type instead of an unscoped future slice.
- **`crates/cubism-datafusion/tests/iceberg_bridge.rs`** (modified): added
  `series_response_materializes_two_published_windows_through_a_real_coverage_plan`
  (`:437-594`) — the real end-to-end wiring test, and one import-line
  addition pulling in `SeriesResponse`.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): added the "Milestone 10b-2"
  section (`:866-933`), flipped to `Done`; corrected Milestone 10's and
  Milestone 10b-1's own stale "not yet scoped"/"not yet added" bullets
  pointing at 10b-2; rewrote the "Phase 5 done condition" walk (`:935-984`)
  to narrow-close Phase 5 (excluding #8, same pattern "Phase 4 done" uses
  for #10/#17) and link this session's phase-review file.
- **`docs/TIMESERIES_PHASE_21_HANDOFF.md`** (modified): added
  `**Superseded by:**` line.

No `Cargo.toml` change: `cubism-iceberg` stays a `cubism-datafusion`
dev-dependency (per the advisor's cut, same as Milestones 10 and 10b-1).

## What was actually verified

That `SeriesResponse::new` correctly materializes a value per segment for
six shapes, all pure unit tests in `series_response.rs`: a single aligned,
fully-published segment produces `is_exact: true`, `value: Some(4.0)` (the
mean of an `AverageState` accumulated with `3.0`/`5.0`), the correct
`published` entry, empty `missing`, and the correct `bucket_start`/
`bucket_end`
(`series_response_materializes_one_exact_published_segment`); a segment
with no data at all produces `value: None` under `GapPolicy::Missing`
(`series_response_no_data_is_none_under_missing_gap_policy`) and `value:
Some(0.0)` under `GapPolicy::Zero`
(`series_response_no_data_is_zero_under_zero_gap_policy`) — proving the gap
policy actually applies where the roadmap's `GapPolicy` doc comment always
said a later milestone would read it; a genuine non-zero-count merge
(`accumulate(3.0)`, count=1) is asserted unchanged by `GapPolicy::Zero`
(`series_response_gap_policy_does_not_touch_a_real_present_value`) — the
one test that directly proves the "gap policy only substitutes for the
zero-count case, never overrides a real value" claim in the module doc
comment, not just asserts it in prose; a `batches` slice shorter than
`coverage.segments` is rejected with `CubismError::Temporal`, mirroring
`CoveragePlan::new`'s own length-mismatch check
(`series_response_rejects_batches_length_mismatch`); and a segment backed
by one published and one missing window correctly reports `is_exact:
false`, a real merged `value` from only the published window's data, and
both `published`/`missing` populated
(`series_response_propagates_missing_and_published_from_partial_segment`).

That the real end-to-end wiring works, via one `#[tokio::test]` against a
real `cubism-iceberg` in-memory catalog
(`series_response_materializes_two_published_windows_through_a_real_coverage_plan`,
`iceberg_bridge.rs:437`): the same two-day-window setup as Milestone 10's
own coverage-plan test, except both windows are published (not one
published/one missing) with real `AverageState`-encoded `avg_v1` rows. A
real `ResolutionPlan` is built from a `TemporalQuery`, resolved into a
`CoveragePlan` against both windows' real `PublicationStore::current`
revisions, both windows are read back via `AggregateReader::read_window`,
and `SeriesResponse::new` is asserted to produce exactly one point with
`value` equal to `a.merge(&b).unwrap().present()` (the same merge
Milestone 10b-1's own integration test verifies, now reached via the full
`ResolutionPlan` -> `CoveragePlan` -> `SeriesResponse` path instead of a
direct `merge_average_column` call), `is_exact: true`, both windows'
`WindowId`/real `WindowRevision` pairs in `published`, and empty `missing`.
This is the test this milestone's "Done when" criterion needed: it proves
the wiring itself, not just that the merge primitive or the coverage
planner each work in isolation (both already proven by Milestones 10 and
10b-1's own tests).

That the full step-4 verification battery — including `cargo clippy … -D
warnings` — is clean with this change in place, run fresh this session
twice: once after the `series_response.rs`/`lib.rs`/`iceberg_bridge.rs`
change, and once more after the `range_query.rs`/`series_merge.rs`
doc-comment corrections and the roadmap edits were added, to confirm those
follow-up edits introduced no regression. `git status` was checked after
each verification pass; no out-of-band reformat of any untouched file
occurred (the `lib.rs`-triggers-module-tree-rustfmt hazard Phase 21
identified was avoided the same way: `lib.rs`'s own two-line diff was
verified by eye, not run through `rustfmt`).

It does **not** prove: anything about `VarianceState`/`QuantileState`/the
sketch-backed kinds — `SeriesResponse`/`merge_average_column` only handle
`AverageState`; those each need their own merge wiring, not built here. It
does not prove storage pruning across many windows (plan
completion-criterion 774, still gated on #8, unchanged by this session —
recorded explicitly as excluded from Phase 5's narrow-close, not silently
dropped). It does not prove anything about `/api/series`
(`crates/cubism-serve`) actually calling `SeriesResponse` — no HTTP-layer
wiring exists yet; that's the next roadmap extension once Phase 5 closes,
per the roadmap's own "Phase 5 done condition" text. It does not, by
itself, establish that Phase 5's narrow-close judgment call (excluding #8)
is sound — that judgment is what this session's step 8a cross-model phase
review exists to check independently; see below for that review's outcome.

## GitHub issues touched

- No new issues filed. `gap_policy` consumption, the per-point
  state/error-metadata scope cut, and the `AverageState`-only scope cut are
  all recorded in the roadmap's Milestone 10b-2 entry, per this series'
  "record scope/behavior findings in the roadmap entry" precedent
  (Milestones 9-10b-1 set it for their own cuts).
- No comments added to any other open issue. `#8` was not re-read this
  session (Phase 21 already read it in full to confirm no overlap with
  value materialization, which is a different seam than 774's SQL/pushdown
  gap this session records against it again) — no new information to add.

## Deferred / not done this session

1. **Widening past `AverageState`** (`VarianceState`/`QuantileState`/the
   sketch-backed kinds `CountDistinct`/`TopK`/`ReservoirSample`/`Centroid`)
   — not scoped to any milestone yet. Each needs its own merge-primitive
   work analogous to `merge_average_column` before `SeriesResponse` (or a
   sibling type) can carry it. Natural next roadmap extension once a
   concrete consumer needs a non-average measure.
2. **`cubism-iceberg` dependency promotion to non-dev** — confirmed again
   this session (implicitly, by not needing it) that `SeriesResponse` has
   no reason to call `AggregateReader::read_window` itself; still
   unpromoted. Would only become necessary if a future slice wants
   `SeriesResponse`/a successor to own its own I/O rather than taking
   caller-supplied batches.
3. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** — unchanged;
   not reached by Milestones 7-10b-2. This is the natural next roadmap
   extension now that Phase 5 is narrow-closed: wiring an HTTP handler that
   builds a `TemporalQuery`, resolves windows, and calls
   `SeriesResponse::new`.
4. **Plan completion-criterion 774** (storage pruning across many windows)
   and the plan's SQL-table-function unresolved decision — unchanged,
   explicitly excluded from Phase 5's narrow-close, still gated on #8.
5. **#17** (append-committed-but-not-recorded recovery) — unchanged; not
   re-read this session (Phase 19-21 already re-confirmed its state).
6. **#16** (event-time window identification + recompute-equality proof) —
   unchanged; not touched by this session's work.
7. **#10** (real object store + Iceberg maintenance) — unchanged.
8. **#11/#12** — unchanged; not touched this session.
9. **#14** (retry-loop/connection-poisoning gap) — unchanged, still open;
   not touched this session.
10. **A successor roadmap slice for what's beyond Phase 5** — with Phase 5
    narrow-closed, the natural next step is deciding what the roadmap's own
    next section covers: `/api/series` (item 3 above), or continuing to
    widen `SeriesResponse` (item 1), or something else. Not decided this
    session; left for the next slice's own step 1/advisor call.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hashes — this doc deliberately doesn't hardcode them, per
this series' established convention):

- New: `docs/TIMESERIES_PHASE_22_HANDOFF.md` (this file),
  `crates/cubism-datafusion/src/series_response.rs`,
  `docs/phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`.
- Modified: `crates/cubism-datafusion/src/lib.rs` (module declaration +
  re-export), `crates/cubism-datafusion/src/range_query.rs` (doc comment
  correction only), `crates/cubism-datafusion/src/series_merge.rs` (doc
  comment correction only), `crates/cubism-datafusion/tests/iceberg_bridge.rs`
  (new integration test + import), `docs/TIMESERIES_ROADMAP.md` (Milestone
  10b-2 added, Milestone 10/10b-1 entries corrected, Phase 5 done-condition
  walk rewritten), `docs/TIMESERIES_PHASE_21_HANDOFF.md` (added
  `**Superseded by:**` line).
- Untouched: `crates/cubism-core/src`, `crates/cubism-iceberg/src`,
  `crates/cubism-datafusion/src/{build,state_udaf,temporal_build,udaf,udf}.rs`
  (confirmed untouched via `git status` after every verification pass this
  session), `crates/cubism-serve/src`, `crates/cubism-cli/` (built and
  clippy-checked, not modified).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`, unchanged; 63 passed in `cubism-datafusion`, up from 56; 203 passed / 2 ignored in workspace, up from 196)

All figures re-run fresh this session, not carried forward from Phase 21,
and re-run a second full time after the doc-comment corrections and
roadmap edits. `cargo test -p cubism-iceberg` reports 33 passed, 1 ignored
(6 suites) — identical to Phase 21, expected since no source in that crate
changed this session. `cargo test -p cubism-datafusion` reports 63 passed
(3 suites) — up from Phase 21's 56 by exactly the seven new tests (six
`series_response` unit tests plus the one new integration test). `cargo
test --workspace --exclude cubism-py` reports 203 passed, 2 ignored (23
suites) — up from Phase 21's 196 by exactly the same seven.

## Verification performed

```text
cargo test -p cubism-datafusion --lib series_response                    # 6 passed
cargo test -p cubism-datafusion --test iceberg_bridge                    # 4 passed (3 prior + 1 new)
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test -p cubism-datafusion                                           # 63 passed (3 suites)
cargo clippy -p cubism-datafusion --all-targets --no-deps -- -D warnings  # clean
cargo test --workspace --exclude cubism-py                                # 203 passed, 2 ignored (23 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

The full battery above was run twice this session: once after the
`series_response.rs`/`lib.rs`/`iceberg_bridge.rs` change; once more after
the `range_query.rs`/`series_merge.rs` doc-comment corrections and the
roadmap edits. Both runs produced identical figures and zero warnings.
Figures shown are from the final run.

## Primary files

- [`../crates/cubism-datafusion/src/series_response.rs`](../crates/cubism-datafusion/src/series_response.rs)
  (Milestone 10b-2's `SeriesResponse`/`SeriesPoint`)
- [`../crates/cubism-datafusion/src/lib.rs`](../crates/cubism-datafusion/src/lib.rs)
  (module declaration + re-export)
- [`../crates/cubism-datafusion/src/range_query.rs`](../crates/cubism-datafusion/src/range_query.rs)
  (module doc comment correction, lines 104-112)
- [`../crates/cubism-datafusion/src/series_merge.rs`](../crates/cubism-datafusion/src/series_merge.rs)
  (module doc comment correction, lines 19-22)
- [`../crates/cubism-datafusion/tests/iceberg_bridge.rs`](../crates/cubism-datafusion/tests/iceberg_bridge.rs)
  (new integration test, line 437)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone
  10b-2, lines 866-933; "Phase 5 'done' condition," lines 935-984)
- [`TIMESERIES_PHASE_21_HANDOFF.md`](TIMESERIES_PHASE_21_HANDOFF.md) (prior
  handoff, superseded by this one)
- [`phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`](phase-reviews/TIMESERIES_PHASE_5_REVIEW.md)
  (this session's step 8a cross-model phase review, the first to run in
  this series)
- GitHub issue [`#8`](https://github.com/jeromebanks/cubism-rs/issues/8)
  (SQL/pushdown gap — completion-criterion 774 and the SQL-table-function
  unresolved decision stay excluded from Phase 5's narrow-close against
  this issue)
