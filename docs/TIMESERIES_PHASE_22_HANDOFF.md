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
and milestone.

Landing this milestone triggered this series' first-ever step 8a
cross-model phase review (flagged explicitly at this session's own step 1
per Phase 21's own deferred item 1), which found three real issues the
advisor's own earlier pass (same-session context, same reasoning) had not
caught: an `is_exact` overclaim on zero-backing-window segments in
`CoveragePlan` (Milestone 10, pre-existing — fixed this session), an
`i64` duration-subtraction overflow in `ResolutionPlan`'s auto-resolution
selection (Milestone 9, pre-existing — fixed this session), and a real,
not-fixed-this-slice correctness gap: `SeriesResponse`/`merge_average_column`
merge every row in a window's batch with no `XUnit` selector filtering,
silently over-merging across lattice cells for any real multi-dimensional
cube (filed as [#19](https://github.com/jeromebanks/cubism-rs/issues/19)).
See "What this session built" and the linked review file below for the
full findings and dispositions. Phase 5's "done" condition is still
narrow-closed (criteria 772/775, the same pattern "Phase 4 done" uses
against #10/#17) — but 772 is now stated as met only for the
single-lattice-cell-per-window case this milestone's test actually
exercises, with #19 excluded from the close the same way #8 already is.

(Despite the filename, this doc documents a session slice, not "Phase 22"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving,
narrow-closed by this session per the roadmap's own "Phase 5 done"
condition walk; the plan's Phase 4
(`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` Phase-4 milestone done but is not itself fully
done — see that roadmap's "Phase 4 done" section, unchanged by this
session, for what remains.)

**Superseded by:** [`TIMESERIES_PHASE_23_HANDOFF.md`](TIMESERIES_PHASE_23_HANDOFF.md),
which lands Milestone 10b-3 — fixing this session's own deferred item 1
(and the "highest-priority" item in its list),
[#19](https://github.com/jeromebanks/cubism-rs/issues/19)'s `XUnit`
selector filtering gap.

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

**Step 8 (same-session advisor) follow-up, applied as its own commit before
step 8a:** the advisor's second pass (after the slice's own code landed)
caught two documentation gaps neither the original design nor its own
first pass had flagged — `is_exact` reads as unconditionally trustworthy
unless a reader is told otherwise, and criteria 772/775 read as
unqualified "met" verdicts unless the narrowing is stated as part of the
verdict, not a footnote after it. Both fixed by documenting (not code
changes): `series_response.rs`'s module doc comment and the `SeriesPoint`
field doc now state plainly that `is_exact` is copied from `CoveragePlan`
and never re-verified against `batches`, with two existing unit tests
(`series_response_no_data_is_none_under_missing_gap_policy`,
`series_response_no_data_is_zero_under_zero_gap_policy`) strengthened with
an explicit `assert!(is_exact)` to make the caveat's shape visible instead
of implied; the roadmap's 772/775 bullets reworded to `met as narrowed …`
so the qualifier is part of the verdict.

**Step 8a (cross-model phase review) findings and dispositions.** This
session's own Codex review (`--scope branch --base f0599b2`, the full
Phase 5 diff, Milestones 7 through 10b-2) found three real issues the
same-session advisor passes above had not caught — exactly the failure
mode step 8a exists to catch, since `advisor()` always sees this session's
own reasoning and a blank-diff reviewer does not:

1. **[P1, fixed this session as a follow-up commit] `CoveragePlan`'s
   `is_exact()` returned `true` for a segment backed by zero windows.**
   Pre-existing since Milestone 10, not introduced this session:
   `SegmentCoverage::is_exact()` was `aligned && missing.is_empty()`; for a
   segment whose caller-supplied window list is entirely empty (as opposed
   to containing entries with `None` revisions), both `published` and
   `missing` are empty, so `missing.is_empty()` was vacuously `true`. Fixed
   by adding a `!self.published.is_empty()` conjunct
   (`crates/cubism-datafusion/src/range_query.rs`); a regression test
   (`coverage_plan_empty_window_list_is_not_exact`) pins down both the
   `exact: false` and `exact: true` shapes. Only now consequential because
   `SeriesResponse` turns "exact" into an actual materialized (wrong)
   answer instead of just unused provenance.
2. **[P2, fixed this session as a follow-up commit] `auto_select_resolution`'s
   duration computation could panic or wrap on an extreme `TimeRange`.**
   Pre-existing since Milestone 9: `range.end().unix_micros() -
   range.start().unix_micros()` is a plain `i64` subtraction, and
   `TimeRange::new` only enforces `start < end`, so a range near
   `[i64::MIN, i64::MAX)` would overflow it. Fixed by widening the
   subtraction to `i128`; a regression test
   (`auto_select_resolution_does_not_overflow_on_extreme_range`) calls the
   private function directly against exactly that input.
3. **[P1, NOT fixed this session, tracked as
   [#19](https://github.com/jeromebanks/cubism-rs/issues/19)]
   `SeriesResponse`/`merge_average_column` merge across `XUnit` lattice
   cells with no selector filtering.** A states table row's `xunit_id` is
   a content hash of one specific lattice cell
   (`cubism_core::encoding::canonical_xunit_content_id`) — the global
   rollup and each per-dimension cell are separately aggregated rows, not
   derivable from each other by summing. `merge_average_column` and
   `SeriesResponse::new` both merge every row in a batch unconditionally,
   with no awareness of `xunit_id` or the query's `TemporalQuery.selectors`
   at all. Any window whose states table contains more than one distinct
   `xunit_id` — the common case for a real multi-dimensional cube, not an
   edge case — gets silently over-merged regardless of which cell the
   query asked for. This session's own integration test does not exercise
   this: each of its two windows is constructed with exactly one
   `xunit_id` row. **Disposition: deferred, filed as #19** rather than
   fixed in-session — a correct fix needs to resolve `TemporalQuery`'s
   selectors to their `XUnitContentId`s (which needs the cube's dimension
   structure, not just the batches already in hand) before filtering,
   which is more than "one bounded slice" per this series' own convention;
   see #19 for the suggested design. Both `series_merge.rs` and
   `series_response.rs`'s doc comments, and the roadmap's Milestone 10b-2
   entry and 772 criterion, now state this gap explicitly rather than let
   "met as narrowed, `AverageState` only" imply more correctness than
   exists — 772 is now stated as met only for the single-cell-per-window
   case this milestone's test actually demonstrates.

Full review text, findings, and this disposition record are in
[`docs/phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`](phase-reviews/TIMESERIES_PHASE_5_REVIEW.md).

Primary files changed:

- **`crates/cubism-datafusion/src/series_response.rs`** (new file, 343
  lines): `SeriesPoint` (`:61-76`), `SeriesResponse` (`:81-84`), and
  `SeriesResponse::new` (`:86-155`, the constructor at `:115-154`) plus six
  unit tests (`:229-342`) covering one exact fully-published segment, both
  `gap_policy` branches for a no-data segment (with the `is_exact` caveat
  now asserted explicitly, see step 8's follow-up below), gap_policy not
  touching a real present value, the batches-length-mismatch rejection, and
  a partially-published segment's `published`/`missing`/`is_exact: false`
  propagation.
- **`crates/cubism-datafusion/src/lib.rs`** (modified, +2 lines): adds
  `pub mod series_response;` and re-exports `SeriesPoint`/`SeriesResponse`.
- **`crates/cubism-datafusion/src/range_query.rs`** (modified): doc-comment
  correction (`:104-112`) pointing the stale "`SeriesResponse`'s job
  (candidate ... not yet added to the roadmap)" note at the landed
  `crate::series_response::SeriesResponse` and Milestone 10b-2 by name;
  plus, from step 8a's own findings (see below), a real code fix to
  `SegmentCoverage::is_exact()` (`:368-384`) and to
  `auto_select_resolution`'s duration computation (`:272-293`), each with
  its own new regression test (`auto_select_resolution_does_not_overflow_on_extreme_range`
  at `:691`, `coverage_plan_empty_window_list_is_not_exact` at `:863`).
- **`crates/cubism-datafusion/src/series_merge.rs`** (modified): doc-comment
  correction (`:19-22`, same stale-reference fix as `range_query.rs`) plus,
  from step 8a's findings, a new doc-comment caveat on
  `merge_average_column` (`:51-60`) recording the `XUnit`-filtering gap
  (#19, not a code change).
- **`crates/cubism-datafusion/tests/iceberg_bridge.rs`** (modified): added
  `series_response_materializes_two_published_windows_through_a_real_coverage_plan`
  (`:437-594`) — the real end-to-end wiring test, and one import-line
  addition pulling in `SeriesResponse`.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): added the "Milestone 10b-2"
  section (`:866-958`), flipped to `Done`; corrected Milestone 10's and
  Milestone 10b-1's own stale "not yet scoped"/"not yet added" bullets
  pointing at 10b-2; rewrote the "Phase 5 done condition" walk
  (`:959-1040`) to narrow-close Phase 5 (excluding #8 and #19, same pattern
  "Phase 4 done" uses for #10/#17) and link this session's phase-review
  file.
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

That `CoveragePlan::new`'s `is_exact()` correctly rejects a segment backed
by zero windows (`coverage_plan_empty_window_list_is_not_exact`,
`range_query.rs:863`) — both the `exact: false` shape (`is_exact()` false,
`published`/`missing` both empty) and the `exact: true` shape (a hard
`CubismError::Temporal` rather than silently succeeding). That
`auto_select_resolution` does not panic or wrap on an astronomically large
`TimeRange`
(`auto_select_resolution_does_not_overflow_on_extreme_range`,
`range_query.rs:691`, calling the private function directly against
`[i64::MIN, i64::MAX)`). Both are step 8a findings, not part of the
original slice design — see below.

That the full step-4 verification battery — including `cargo clippy … -D
warnings` — is clean with this change in place, run fresh this session
three times: once after the `series_response.rs`/`lib.rs`/`iceberg_bridge.rs`
change; once more after the `range_query.rs`/`series_merge.rs` doc-comment
corrections and the roadmap edits; and once more after step 8a's own two
code fixes (`is_exact`, the duration overflow) and their regression tests
were added, to confirm those follow-up edits introduced no regression.
`git status` was checked after each verification pass; no out-of-band
reformat of any untouched file occurred (the `lib.rs`-triggers-module-tree-
rustfmt hazard Phase 21 identified was avoided the same way: `lib.rs`'s own
two-line diff was verified by eye, not run through `rustfmt`).

It does **not** prove: anything about `VarianceState`/`QuantileState`/the
sketch-backed kinds — `SeriesResponse`/`merge_average_column` only handle
`AverageState`; those each need their own merge wiring, not built here. It
does not prove storage pruning across many windows (plan
completion-criterion 774, still gated on #8, unchanged by this session —
recorded explicitly as excluded from Phase 5's narrow-close, not silently
dropped). It does not prove anything about `/api/series`
(`crates/cubism-serve`) actually calling `SeriesResponse` — no HTTP-layer
wiring exists yet; that's the next roadmap extension once Phase 5 closes,
per the roadmap's own "Phase 5 done condition" text. **It does not prove
`SeriesResponse` answers a multi-`XUnit`-cell window correctly** — the
integration test's own two windows each carry exactly one `xunit_id` row,
so it cannot and does not exercise the #19 gap (see "What this session
built" above); a real multi-dimensional cube's window would silently
over-merge under this exact test's own assertions if that gap were
triggered. This is the one place where "the test passes" reads as more
coverage than it has unless this caveat is stated plainly, matching this
series' own standing convention.

## GitHub issues touched

- **Filed [#19](https://github.com/jeromebanks/cubism-rs/issues/19)** —
  `SeriesResponse`/`merge_average_column` merge across `XUnit` lattice
  cells with no selector filtering. Found by this session's own step 8a
  cross-model phase review, not by the advisor or by writing the code
  itself. Full body covers the `XUnitContentId`/`canonical_xunit_content_id`
  mechanism, why it's a real gap (not an edge case), and a suggested fix;
  cross-linked from `series_response.rs`, `series_merge.rs`, and the
  roadmap's Milestone 10b-2 entry and 772 criterion.
- `gap_policy` consumption, the per-point state/error-metadata scope cut,
  and the `AverageState`-only scope cut needed no new issue — all recorded
  in the roadmap's Milestone 10b-2 entry, per this series' "record
  scope/behavior findings in the roadmap entry" precedent (Milestones
  9-10b-1 set it for their own cuts).
- No comments added to any other open issue. `#8` was not re-read this
  session (Phase 21 already read it in full to confirm no overlap with
  value materialization, which is a different seam than 774's SQL/pushdown
  gap this session records against it again) — no new information to add.

## Deferred / not done this session

1. **[#19](https://github.com/jeromebanks/cubism-rs/issues/19)
   (`XUnit`-selector filtering)** — the highest-priority item in this list.
   `SeriesResponse`/`merge_average_column` merge every row in a window's
   batch regardless of `xunit_id`, silently over-merging across lattice
   cells for any real multi-dimensional cube. Not fixed this session
   (needs `TemporalQuery.selectors` resolved to `XUnitContentId`s against
   the cube's dimension structure — genuinely more than "one bounded
   slice"). Whoever picks this up next should read #19's "Suggested next
   steps" and `crates/cubism-core/src/encoding.rs` before designing.
2. **Widening past `AverageState`** (`VarianceState`/`QuantileState`/the
   sketch-backed kinds `CountDistinct`/`TopK`/`ReservoirSample`/`Centroid`)
   — not scoped to any milestone yet. Each needs its own merge-primitive
   work analogous to `merge_average_column` before `SeriesResponse` (or a
   sibling type) can carry it. Natural next roadmap extension once a
   concrete consumer needs a non-average measure.
3. **`cubism-iceberg` dependency promotion to non-dev** — confirmed again
   this session (implicitly, by not needing it) that `SeriesResponse` has
   no reason to call `AggregateReader::read_window` itself; still
   unpromoted. Would only become necessary if a future slice wants
   `SeriesResponse`/a successor to own its own I/O rather than taking
   caller-supplied batches.
4. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** — unchanged;
   not reached by Milestones 7-10b-2. This is the natural next roadmap
   extension now that Phase 5 is narrow-closed: wiring an HTTP handler that
   builds a `TemporalQuery`, resolves windows, and calls
   `SeriesResponse::new`.
5. **Plan completion-criterion 774** (storage pruning across many windows)
   and the plan's SQL-table-function unresolved decision — unchanged,
   explicitly excluded from Phase 5's narrow-close, still gated on #8.
6. **#17** (append-committed-but-not-recorded recovery) — unchanged; not
   re-read this session (Phase 19-21 already re-confirmed its state).
7. **#16** (event-time window identification + recompute-equality proof) —
   unchanged; not touched by this session's work.
8. **#10** (real object store + Iceberg maintenance) — unchanged.
9. **#11/#12** — unchanged; not touched this session.
10. **#14** (retry-loop/connection-poisoning gap) — unchanged, still open;
    not touched this session.
11. **A successor roadmap slice for what's beyond Phase 5** — with Phase 5
    narrow-closed, the natural next step is deciding what the roadmap's own
    next section covers: #19's `XUnit`-filtering fix (item 1 above, the
    most consequential gap), `/api/series` (item 4), or continuing to widen
    `SeriesResponse` past `AverageState` (item 2). Not decided this
    session; left for the next slice's own step 1/advisor call.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hashes — this doc deliberately doesn't hardcode them, per
this series' established convention):

- New: `docs/TIMESERIES_PHASE_22_HANDOFF.md` (this file),
  `crates/cubism-datafusion/src/series_response.rs`,
  `docs/phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`.
- Modified: `crates/cubism-datafusion/src/lib.rs` (module declaration +
  re-export), `crates/cubism-datafusion/src/range_query.rs` (doc-comment
  correction plus, from step 8a, the `is_exact`/duration-overflow fixes and
  their two regression tests), `crates/cubism-datafusion/src/series_merge.rs`
  (doc-comment correction plus the step 8a `XUnit`-filtering caveat, no code
  change), `crates/cubism-datafusion/tests/iceberg_bridge.rs` (new
  integration test + import), `docs/TIMESERIES_ROADMAP.md` (Milestone 10b-2
  added, Milestone 10/10b-1 entries corrected, Phase 5 done-condition walk
  rewritten twice — once for the slice, once more for step 8a's findings),
  `docs/TIMESERIES_PHASE_21_HANDOFF.md` (added `**Superseded by:**` line).
- Untouched: `crates/cubism-core/src`, `crates/cubism-iceberg/src`,
  `crates/cubism-datafusion/src/{build,state_udaf,temporal_build,udaf,udf}.rs`
  (confirmed untouched via `git status` after every verification pass this
  session), `crates/cubism-serve/src`, `crates/cubism-cli/` (built and
  clippy-checked, not modified).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`, unchanged; 65 passed in `cubism-datafusion`, up from 56; 205 passed / 2 ignored in workspace, up from 196)

All figures re-run fresh this session, not carried forward from Phase 21,
across three full battery runs (see "Verification performed" below).
`cargo test -p cubism-iceberg` reports 33 passed, 1 ignored (6 suites) —
identical to Phase 21, expected since no source in that crate changed this
session. `cargo test -p cubism-datafusion` reports 65 passed (3 suites) —
up from Phase 21's 56 by nine: the seven new tests from the
`SeriesResponse` slice itself (six `series_response` unit tests plus one
integration test), plus two more from step 8a's own regression tests
(`coverage_plan_empty_window_list_is_not_exact`,
`auto_select_resolution_does_not_overflow_on_extreme_range`, both in
`range_query.rs`). `cargo test --workspace --exclude cubism-py` reports 205
passed, 2 ignored (23 suites) — up from Phase 21's 196 by the same nine.

## Verification performed

```text
cargo test -p cubism-datafusion --lib series_response                    # 6 passed
cargo test -p cubism-datafusion --lib range_query                        # 19 passed (17 prior + 2 new)
cargo test -p cubism-datafusion --test iceberg_bridge                    # 4 passed (3 prior + 1 new)
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test -p cubism-datafusion                                           # 65 passed (3 suites)
cargo clippy -p cubism-datafusion --all-targets --no-deps -- -D warnings  # clean
cargo test --workspace --exclude cubism-py                                # 205 passed, 2 ignored (23 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

The full battery above was run three times this session: once after the
`series_response.rs`/`lib.rs`/`iceberg_bridge.rs` change; once more after
the `range_query.rs`/`series_merge.rs` doc-comment corrections and the
roadmap edits (step 8's own follow-up); and once more after step 8a's own
`is_exact`/duration-overflow fixes and their regression tests. All three
runs produced zero warnings; figures shown are from the final run.

## Primary files

- [`../crates/cubism-datafusion/src/series_response.rs`](../crates/cubism-datafusion/src/series_response.rs)
  (Milestone 10b-2's `SeriesResponse`/`SeriesPoint`)
- [`../crates/cubism-datafusion/src/lib.rs`](../crates/cubism-datafusion/src/lib.rs)
  (module declaration + re-export)
- [`../crates/cubism-datafusion/src/range_query.rs`](../crates/cubism-datafusion/src/range_query.rs)
  (module doc comment correction, lines 104-112; step 8a's `is_exact` fix,
  lines 368-384; step 8a's duration-overflow fix, lines 272-293)
- [`../crates/cubism-datafusion/src/series_merge.rs`](../crates/cubism-datafusion/src/series_merge.rs)
  (module doc comment correction, lines 19-22; step 8a's `XUnit`-filtering
  caveat, lines 51-60)
- [`../crates/cubism-datafusion/tests/iceberg_bridge.rs`](../crates/cubism-datafusion/tests/iceberg_bridge.rs)
  (new integration test, line 437)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone
  10b-2, lines 866-958; "Phase 5 'done' condition," lines 959-1040)
- [`TIMESERIES_PHASE_21_HANDOFF.md`](TIMESERIES_PHASE_21_HANDOFF.md) (prior
  handoff, superseded by this one)
- [`phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`](phase-reviews/TIMESERIES_PHASE_5_REVIEW.md)
  (this session's step 8a cross-model phase review, the first to run in
  this series)
- GitHub issue [`#19`](https://github.com/jeromebanks/cubism-rs/issues/19)
  (`SeriesResponse`/`merge_average_column` merge across `XUnit` cells with
  no selector filtering — the one step 8a finding not fixed this session)
- GitHub issue [`#8`](https://github.com/jeromebanks/cubism-rs/issues/8)
  (SQL/pushdown gap — completion-criterion 774 and the SQL-table-function
  unresolved decision stay excluded from Phase 5's narrow-close against
  this issue)
