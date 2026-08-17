# Phase 5 Cross-Model Review

Date: 2026-08-16

Branch: `feature/timeseries-phase-0a`

Phase: 5 (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5 Milestones" section —
Milestones 7 through 10b-2). This is the first phase review this series has
run; the `timeseries-slice` skill's step 8a did not exist when Phase 4
closed, so no equivalent record exists for Phase 4.

Diff range: `f0599b2` (`timeseries: apply advisor follow-up fixes for the
retry-loop slice`) `..` `1d4cb1c` (`timeseries: apply step 8a phase-review
fixes for Phase 5 (Milestones 7-10b-2)`) — every commit that landed
Milestones 7 through 10b-2, plus this session's own step 8 and step 8a
follow-up commits.

Reviewer: Codex, via the `codex-companion.mjs` runtime (`node
"$CODEX_SCRIPT" review --wait --json --scope branch --base f0599b2`).
`sessionRuntime.mode` was `"direct"` (no prior shared runtime) at the
pre-check in step 1; the review call itself returned `codex.status: 0`
with non-empty `codex.stdout`. No fallback to a blank-context `Agent` was
needed.

## Full review text (verbatim)

> The new range-response path can merge unrelated XUnits and can certify an aligned range as exact without any backing window. It also contains a timestamp-overflow edge and references a missing required review artifact.
>
> Full review comments:
>
> - [P1] Filter batches by the requested XUnit selector — `crates/cubism-datafusion/src/series_response.rs:112-112`
>   When these batches come directly from `AggregateReader::read_window`, they contain every `(bucket_start, xunit_id)` row in the window. This call merges every state without receiving or applying `TemporalQuery.selectors`, so a window containing multiple XUnits returns a cross-XUnit average even for a selector-specific query; filter the rows by canonical `xunit_id` before merging.
>
> - [P1] Reject exact coverage with no backing windows — `crates/cubism-datafusion/src/range_query.rs:366-368`
>   For an aligned segment with `windows[i] == []`, `missing.is_empty()` makes this return true, so `CoveragePlan::new(..., exact=true, ...)` succeeds even though no retained window covers the non-empty range. `SeriesResponse` can consequently report an exact `None` or zero value; reject empty backing lists or treat them as inexact.
>
> - [P2] Widen the duration subtraction before selecting a rollup — `crates/cubism-datafusion/src/range_query.rs:276-276`
>   `TimeRange` accepts any ordered pair of `i64` timestamps, so ranges such as `[i64::MIN, i64::MAX)` overflow this subtraction. Debug builds panic and release builds can wrap to a negative duration and select the wrong resolution; calculate the duration in `i128` or use checked arithmetic.
>
> - [P2] Commit the linked phase-review artifact — `docs/TIMESERIES_ROADMAP.md:1009-1009`
>   This link and the Phase 22 handoff claim that the review was committed, but `HEAD` contains no `docs/phase-reviews` directory or `TIMESERIES_PHASE_5_REVIEW.md`. The roadmap therefore has a broken audit link and no recorded finding dispositions; add the artifact or remove the claims that it exists.

(Original findings quoted absolute paths under
`/Users/jeromebanks/dev/cubism_saas/cubism/`; shown above relative to the
repo root for readability. Line numbers are as of the commit the review
actually ran against, i.e. before this file's own follow-up fixes — see
each disposition below for the post-fix location.)

## Dispositions

1. **[P1] Filter batches by the requested `XUnit` selector**
   (`series_response.rs:112`, pre-fix line number).
   **Disposition: deferred, filed as
   [#19](https://github.com/jeromebanks/cubism-rs/issues/19).** Confirmed
   real and more severe than the review's own wording implies on
   investigation: `xunit_id` is a content hash of one specific lattice
   cell (`cubism_core::encoding::canonical_xunit_content_id`), and the
   global rollup is its own separately-aggregated row, not derivable from
   the per-cell rows by summing — so this isn't only a "selector-specific
   query" problem, it's wrong for essentially any window containing more
   than one distinct `xunit_id`, including a naive attempt to sum
   everything for a `global` query. Not fixed in this session: a correct
   fix needs `TemporalQuery.selectors` resolved to `XUnitContentId`s
   against the cube's dimension structure, which is more than "one bounded
   slice" per the `timeseries-slice` skill's own convention for this
   series. Documented prominently in `series_response.rs`'s and
   `series_merge.rs`'s doc comments and in the roadmap's Milestone 10b-2
   entry and 772 criterion (now stated as met only for the
   single-cell-per-window case Milestone 10b-2's own test exercises), so
   the gap is visible rather than implied. #19's body records the finding
   in full plus a suggested fix.
2. **[P1] Reject exact coverage with no backing windows**
   (`range_query.rs:366-368`, pre-fix line numbers).
   **Disposition: fixed**, commit `1d4cb1c`.
   `SegmentCoverage::is_exact()` now requires `!self.published.is_empty()`
   in addition to `self.segment.aligned && self.missing.is_empty()` — see
   `range_query.rs:368-384` post-fix. Regression test
   `coverage_plan_empty_window_list_is_not_exact`
   (`range_query.rs:863`) pins down both the `exact: false` shape (not
   exact, both `published`/`missing` empty) and the `exact: true` shape (a
   hard `CubismError::Temporal`). Pre-existing since Milestone 10, not
   introduced by Milestone 10b-2 — only became consequential once
   `SeriesResponse` turned "exact" into an actual materialized answer
   instead of unused provenance. Full step-4 battery re-run clean after
   this fix (65 passed in `cubism-datafusion`, 205 passed / 2 ignored in
   workspace).
3. **[P2] Widen the duration subtraction before selecting a rollup**
   (`range_query.rs:276`, pre-fix line number).
   **Disposition: fixed**, commit `1d4cb1c`.
   `auto_select_resolution`'s duration computation widened from a plain
   `i64` subtraction to `i128` — see `range_query.rs:272-293` post-fix.
   Regression test `auto_select_resolution_does_not_overflow_on_extreme_range`
   (`range_query.rs:691`) calls the private function directly against
   `[i64::MIN, i64::MAX)` and asserts it returns `Ok` rather than
   panicking or wrapping. Pre-existing since Milestone 9, not introduced
   by Milestone 10b-2.
4. **[P2] Commit the linked phase-review artifact**
   (`docs/TIMESERIES_ROADMAP.md:1009`, pre-fix line number — the review
   ran against the commit that added this file's forward-link before the
   file itself existed).
   **Disposition: fixed by construction — this file.** The review's own
   process caught a real ordering issue: the roadmap and Phase 22 handoff
   were committed with a forward-link to this file before the review that
   produces it had run. Per the `timeseries-slice` skill's step 8a ("write
   this file even if the review is clean," "apply fixes as a separate
   follow-up commit"), this is expected — the roadmap/handoff commits land
   first, the review and its file land in the follow-up commit that
   includes this file, and by the time of `git push` the link resolves in
   the final tree state. No further action needed beyond this file's own
   existence and the roadmap edits already made in commit `1d4cb1c`
   (criteria 772/775 wording) alongside it.

## Summary

Two real, pre-existing bugs found and fixed (`is_exact` on zero-backing
segments; duration-subtraction overflow), each with a regression test and
a clean full step-4 battery re-run. One real, not-fixed correctness gap
(`XUnit`-selector filtering) found, confirmed on investigation to be more
severe than the review's own summary states, documented explicitly
everywhere a reader would encounter `SeriesResponse`/`merge_average_column`,
and filed as [#19](https://github.com/jeromebanks/cubism-rs/issues/19) —
excluded from Phase 5's narrow-close the same way completion-criterion 774
is excluded against #8, not silently dropped. This review validated the
premise `timeseries-slice`'s step 8a exists for: the same-session advisor,
which sees this session's own reasoning, did not catch any of these three;
an independent diff-only reviewer did.
