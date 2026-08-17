# Time-Series Phase 23 Handoff

Date: 2026-08-16

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 10b-3** (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5
Milestones" section) — single-selector `XUnit` filtering in
`SeriesResponse::new`, fixing
[#19](https://github.com/jeromebanks/cubism-rs/issues/19) (the real
correctness gap Milestone 10b-2's own step 8a cross-model phase review
found last session: every row in a segment's batch was merged
unconditionally, with no filtering by the query's `XUnit` selector).
`SeriesResponse::new` now takes a new `selectors: &[XUnit]` parameter,
rejects anything but exactly one selector, resolves that selector to its
`XUnitContentId`, and filters every segment's `batches` to matching rows
before merging. Two new unit tests plus an update to the existing
integration test (now tagging its rows with a real, production-hash-derived
`xunit_id` instead of arbitrary sentinel bytes) prove the fix through both
the pure unit-test path and the full `ResolutionPlan` -> `CoveragePlan` ->
`SeriesResponse` path against a real Parquet write/scan. #19 is closed, not
merely narrowed around — the roadmap's Phase 5 "done" condition (criterion
772) is updated accordingly. This slice does **not** re-trigger step 8a:
Phase 5 was already narrow-closed by last session, and tightening an
already-closed "done" condition around a fix is a different thing from
newly satisfying one for the first time (confirmed with the advisor before
any code was written).

(Despite the filename, this doc documents a session slice, not "Phase 23"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving,
already narrow-closed by `docs/TIMESERIES_PHASE_22_HANDOFF.md`'s session
and tightened further by this one; the plan's Phase 4
(`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` Phase-4 milestone done but is not itself fully
done — see that roadmap's "Phase 4 done" section, unchanged by this
session, for what remains.)

**Superseded by:** [`TIMESERIES_PHASE_24_HANDOFF.md`](TIMESERIES_PHASE_24_HANDOFF.md),
a user-directed detour (not the next roadmap milestone pick) fixing
[#17](https://github.com/jeromebanks/cubism-rs/issues/17) — a real
correctness gap in crash recovery, unrelated to this session's own Phase 5
work, found via a separate conversation assessing open GitHub issues for
tech debt.

## What this session built

Read `docs/TIMESERIES_PHASE_22_HANDOFF.md` and confirmed the branch in sync
with `origin` (`git rev-list --left-right --count` reported `0 0`), then
listed all open issues (`#1`-`#19`, unchanged from Phase 22). Phase 22's
deferred item 11 left the next slice's choice open among three candidates
(#19's `XUnit` filtering, `/api/series`, or widening past `AverageState`)
and explicitly flagged #19 as "the most consequential gap." Read #19's full
body, `series_response.rs`, `series_merge.rs`, `range_query.rs`'s
`TemporalQuery`/`XUnit` handling, `cubism-core/src/encoding.rs`'s
`CanonicalXUnit`/`canonical_xunit_content_id`, `cubism-core/src/ypath.rs`'s
`XUnit`, and `temporal_build.rs`'s `xunit_identity_udfs`/`XUnitContentIdUdf`
before calling the advisor, per the skill's step 1.

The advisor confirmed #19 as the right next slice and sharpened its scope
before any code was written:

- **The key finding that shrank #19 below its own estimate.** #19's
  "Suggested next steps" assumed resolving a selector to its
  `XUnitContentId` would need "the cube's dimension structure." Reading
  `cubism-core/src/encoding.rs`'s `impl From<&XUnit> for CanonicalXUnit`
  (`:77-100`) showed this is wrong: it is a pure, dictionary-independent
  function of the selector's own `dim`/attribute strings, no dimension
  structure required. The advisor flagged this as resting on one
  unverified equality — whether a real states-table `xunit_id` byte value
  actually equals `canonical_xunit_content_id(&CanonicalXUnit::from(&selector))`
  — and instructed reading the build side's UDF body before writing any
  code, not assuming the equality. Reading
  `temporal_build.rs`'s `XUnitContentIdUdf::invoke_with_args`
  (`:301-319`) confirmed it exactly: a states row's `xunit_id` is computed
  as `canonical_xunit_content_id(&CanonicalXUnit::from(&decode_xunit(key,
  &dict)))` — the *same* two calls a query-side selector resolves through,
  just fed a dictionary-decoded `XUnit` instead of the selector's own
  already-typed one. Both sides normalize identically
  (`CanonicalXUnit::normalize` sorts by dimension regardless of input
  order), so a query-side selector and a build-side row resolve to
  byte-identical ids for the same logical cell. This turned the slice into
  "resolve one selector via functions that already exist, then filter,"
  not a new selector-resolution subsystem.
- **Scope fence: exactly one selector, not optional.** `TemporalQuery.selectors`
  is a `Vec<XUnit>`; filtering to *N* selectors and merging across them
  would be the same class of over-merge bug with the caller's blessing, and
  "multi-XUnit response shape" is already a separate, unresolved roadmap
  decision (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5 done condition",
  "Unresolved decisions" bullet). `selectors.len() != 1` is rejected with
  `CubismError::Temporal` — including the empty case, since empty must not
  be read as "no filter" (that reading is exactly the bug being fixed).
- **Filter mechanics: inside `SeriesResponse::new`, not
  `merge_average_column`.** `series_merge.rs:57-58` already documented this
  contract ("callers must pre-filter batches... nothing here enforces
  that"); the advisor confirmed `merge_average_column` should stay
  selector-agnostic by design, and the filter belongs at the one call site
  that actually has a selector to resolve. Implemented via
  `arrow::compute::filter_record_batch` against a boolean mask over the
  `xunit_id` column, built once per segment, then `merge_average_column` is
  called unchanged on the filtered batches.
- **Read-back type and null handling, flagged before writing code.** The
  advisor pointed out `merge_average_column`'s own doc comment records that
  iceberg widens `Binary` -> `LargeBinary` on scan, and asked whether
  `FixedSizeBinary(32)` survives the same round trip unwidened. Checked the
  vendored `iceberg` 0.10.0 crate's `arrow/schema.rs`
  (`ToArrowSchemaConverter`'s `PrimitiveType::Fixed(len)` arm, `:684-689`):
  a fixed length that fits `i32` (32 does) maps straight to
  `DataType::FixedSizeBinary(32)`, no widening — confirmed by reading the
  vendored source, not assumed. The `xunit_id` column is also nullable
  (`temporal_build.rs:142`); the filter treats a null `xunit_id` as
  matching nothing, stated explicitly in `filter_batches_by_xunit`'s doc
  comment.
- **One non-blocking documentation note the advisor raised:** after
  filtering, a segment whose rows are all other cells now yields
  `is_exact: true` + `value: None`, indistinguishable from genuine no-data.
  Recorded as one sentence in `SeriesResponse::new`'s doc comment
  (extending Milestone 10b-2's existing "`is_exact` is not re-verified
  against `batches`" caveat) rather than re-deriving `is_exact`, per the
  advisor's instruction not to expand scope there.
- **Roadmap placement: a new milestone under the existing "Phase 5
  Milestones" section, not a new `## Phase 6`.** The advisor's reasoning:
  #19 is a gap *in* criterion 772, not new phase scope, so tightening an
  already-narrow-closed "done" condition around a fix does not re-trigger
  step 8a the way newly satisfying one for the first time would. Recorded
  as "Milestone 10b-3."

Implementation matched this design exactly — no scope drift discovered
mid-slice. `SeriesResponse::new` gained a `selectors: &[XUnit]` parameter
(the `[selector] = selectors else { ... }` slice-pattern rejection reads
naturally for the "exactly one" invariant); a new private
`filter_batches_by_xunit` helper filters each segment's batches before
`merge_average_column` runs.

**Test-fixture correctness, not just code correctness.** Five of Milestone
10b-2's six pre-existing unit tests in `series_response.rs` constructed
their one-column `avg_v1` batches with no `xunit_id` column at all (the
sixth, the batches-length-mismatch rejection test, passes zero batches and
constructs none) — the five would have failed outright once filtering
became mandatory, not silently passed. Fixed by adding an `xunit_id` column
to the test schema and tagging every row with a real content id computed
via a new `content_id` test helper (the same `CanonicalXUnit::from` +
`canonical_xunit_content_id` pair production code calls), not an arbitrary
sentinel; all six tests were updated to pass the new `selectors` parameter.
Two new tests were
added: `series_response_filters_batches_to_the_resolved_selector_before_merging`
(a batch carrying rows for two distinct real `XUnitContentId`s — a global
cell and a `device=mobile` cell — asserts only the selector's cell's row is
merged) and `series_response_rejects_multi_selector_query` (both a
two-selector and a zero-selector call are rejected). The pre-existing
`series_response_materializes_two_published_windows_through_a_real_coverage_plan`
integration test (`crates/cubism-datafusion/tests/iceberg_bridge.rs`) had
used arbitrary `[1u8; 32]`/`[2u8; 32]` sentinel bytes for its two windows'
`xunit_id` rows (harmless before filtering existed, but silently wrong
against a query filter, since neither sentinel matches a real selector's
resolved id) — updated to tag both windows with the real global-cell
content id via a new `content_id` helper in that file, so the query's
`XUnit::global()` selector actually matches. Milestone 10b-1's own direct
`merge_average_column` test in the same file (which bypasses
`SeriesResponse` and its filter entirely) was deliberately left with its
original sentinel bytes — noted inline in the diff so this isn't
mistaken for an oversight.

**A real bug caught by the tests, not by review:** `FixedSizeBinaryArray::try_from_iter`
errors on an empty iterator (it can't infer the byte width from zero
elements), which several of this session's own zero-row test batches hit
immediately (`series_response_no_data_is_none_under_missing_gap_policy` and
two others failed with `InvalidArgumentError("Input iterable argument has
no data")` on the first test run). Fixed by switching the test helper to
`FixedSizeBinaryBuilder::new(32)` (an explicit width, independent of row
count) instead of `try_from_iter`.

## What was actually verified

That `SeriesResponse::new` correctly filters to the resolved selector
before merging, via the new unit test
`series_response_filters_batches_to_the_resolved_selector_before_merging`:
a single segment's batch carries two rows — one tagged with the real
`XUnit::global()` content id and an `AverageState` of `accumulate(3.0)` +
`accumulate(5.0)` (mean 4.0), the other tagged with a real
`device=mobile` cell's content id and an unrelated `AverageState` of
`accumulate(1000.0)` — and asserts the response's merged value is `Some(4.0)`,
not something polluted by the 1000.0 row. Both ids are computed via the
same `CanonicalXUnit::from` + `canonical_xunit_content_id` pair production
code calls, not sentinel bytes, so this is a genuine proof that the filter
selects the right rows, not a test that would pass by construction
regardless of filtering.

That the multi-selector and zero-selector cases are rejected, not silently
merged or silently treated as "no filter," via
`series_response_rejects_multi_selector_query`: both a two-`XUnit::global()`-element
selector slice and an empty selector slice return `CubismError::Temporal`.

That the fix holds through the real end-to-end wiring, not just pure unit
tests: `series_response_materializes_two_published_windows_through_a_real_coverage_plan`
(`iceberg_bridge.rs`) — the same two-published-window setup Milestone
10b-2 built, now with both windows' rows tagged with the real
`XUnit::global()` content id (computed via the file's new `content_id`
helper, calling the identical production functions) instead of arbitrary
`[1u8; 32]`/`[2u8; 32]` bytes — still asserts the response's `value` equals
`a.merge(&b).unwrap().present()` after being read back through a real
`AggregateReader::read_window` scan and filtered by
`SeriesResponse::new`'s new selector-resolution path. This proves the
resolve-then-filter logic works against real Parquet-scanned
`FixedSizeBinary(32)` data, not just hand-built in-memory batches.

That `FixedSizeBinary(32)` does not widen on iceberg scan-back the way
`Binary` does: confirmed by reading the vendored `iceberg` 0.10.0 crate's
`ToArrowSchemaConverter` (`arrow/schema.rs:684-689`) rather than assumed —
`PrimitiveType::Fixed(32)` maps to `DataType::FixedSizeBinary(32)` on both
the write-schema and (since 32 fits `i32`) the read-back path, with no
`LargeBinary` fallback (that fallback only triggers for a fixed length that
doesn't fit `i32`, which 32 never hits).

That the full step-4 verification battery — including `cargo clippy … -D
warnings` — is clean with this change in place: `cargo test -p
cubism-datafusion --lib series_response` (8 passed, up from 6),
`cargo test -p cubism-datafusion --test iceberg_bridge` (4 passed,
unchanged — one test modified in place, none added), the full
`cubism-datafusion` suite (67 passed, up from 65), the full workspace suite
(207 passed / 2 ignored, up from 205), and every clippy target, all run
fresh this session. `git status` was checked after every verification pass;
no out-of-band reformat of any untouched file occurred — `rustfmt
--edition 2024 --check` was run on all three touched source files before
any formatting, per the skill's mandatory gate:
`crates/cubism-datafusion/src/series_merge.rs` reported no diff (a
doc-comment-only change); `crates/cubism-datafusion/src/series_response.rs`
reported six diffs, all confirmed to be on lines added or modified this
session (not pre-existing content), and hand-fixed to match rustfmt's exact
output rather than run through plain `rustfmt`;
`crates/cubism-datafusion/tests/iceberg_bridge.rs` reported many diffs,
confirmed via a byte-for-byte diff against `rustfmt --emit stdout`'s full
output that all but one (the new `content_id` helper) are pre-existing,
unrelated to this session (matching
[#3](https://github.com/jeromebanks/cubism-rs/issues/3), "rustfmt --check
does not reproduce clean at HEAD") — only the one new-this-session hunk was
hand-fixed; the rest of the file's formatting was left exactly as
committed.

It does **not** prove: anything about `VarianceState`/`QuantileState`/the
sketch-backed kinds (unchanged, still `AverageState`-only, per Milestone
10b-2's own scope cut). It does not prove storage pruning across many
windows (plan completion-criterion 774, still gated on #8, unchanged by
this session). It does not prove anything about `/api/series`
(`crates/cubism-serve`) — no HTTP-layer wiring exists yet. It does not
prove the multi-XUnit/multi-measure response shape works — that remains an
explicitly unresolved roadmap decision, and this session's fix rejects
that case outright rather than guessing at an answer for it, which is a
different (and, given #19's own history, safer) thing than solving it.

## GitHub issues touched

- **Closed [#19](https://github.com/jeromebanks/cubism-rs/issues/19)** with
  a comment cross-linking Milestone 10b-3's roadmap entry and this handoff:
  the silent over-merge across `XUnit` lattice cells is fixed for the
  single-selector case `SeriesResponse::new` supports, and a multi-selector
  query is now rejected outright rather than silently answered wrong.
- No new issues filed. The multi-XUnit/multi-measure response shape
  remains recorded in the roadmap's "Phase 5 done condition" as an
  unresolved decision, not filed separately — same "record scope/behavior
  findings in the roadmap entry" precedent this series has used since
  Milestone 9.

## Deferred / not done this session

1. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** — unchanged;
   the natural next roadmap extension now that both Phase 5's original
   "done" condition and its one excluded gap (#19) are closed: wiring an
   HTTP handler that builds a `TemporalQuery`, resolves windows, and calls
   `SeriesResponse::new`, including deciding how the handler enforces the
   "exactly one selector" contract Milestone 10b-3 just added (reject a
   multi-selector HTTP request the same way, or something richer — not
   decided this session).
2. **Widening past `AverageState`** (`VarianceState`/`QuantileState`/the
   sketch-backed kinds) — unchanged, not scoped to any milestone yet.
3. **The multi-XUnit/multi-measure response shape** — still an unresolved
   roadmap decision (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5 done
   condition," "Unresolved decisions" bullet). Milestone 10b-3 deliberately
   rejects this case rather than solving it; whoever picks it up next
   should read that bullet and Milestone 10b-3's own "Deviations" before
   designing, since a real answer needs to decide what a multi-cell
   `SeriesResponse` even returns (one point per cell? a nested shape?),
   not just relax the `selectors.len() != 1` check.
4. **`cubism-iceberg` dependency promotion to non-dev** — unchanged, still
   unpromoted; not touched this session.
5. **Plan completion-criterion 774** (storage pruning across many windows)
   and the plan's SQL-table-function unresolved decision — unchanged,
   still gated on #8.
6. **#17** (append-committed-but-not-recorded recovery) — unchanged; not
   re-read this session.
7. **#16** (event-time window identification + recompute-equality proof) —
   unchanged; not touched by this session's work.
8. **#10** (real object store + Iceberg maintenance) — unchanged.
9. **#11/#12** — unchanged; not touched this session.
10. **#14** (retry-loop/connection-poisoning gap) — unchanged, still open;
    not touched this session.
11. **[#3](https://github.com/jeromebanks/cubism-rs/issues/3)**
    (`rustfmt --check` not reproducing clean at HEAD) — reconfirmed again
    this session against `iceberg_bridge.rs` (many pre-existing diffs,
    none touched); still open, still unfixed, same standing workaround
    (hand-fix only touched lines) applied again successfully.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hashes — this doc deliberately doesn't hardcode them, per
this series' established convention):

- New: `docs/TIMESERIES_PHASE_23_HANDOFF.md` (this file).
- Modified: `crates/cubism-datafusion/src/series_response.rs` (new
  `selectors` parameter on `SeriesResponse::new`, new
  `filter_batches_by_xunit` helper, updated module/function doc comments,
  two new unit tests, all six pre-existing unit tests updated for the new
  `selectors` parameter and five of them for a real `xunit_id`),
  `crates/cubism-datafusion/src/series_merge.rs`
  (doc-comment-only update recording where the #19 fix landed, no code
  change), `crates/cubism-datafusion/tests/iceberg_bridge.rs` (new
  `content_id` helper, the existing `SeriesResponse` integration test
  updated to use real content ids and the new `selectors` argument),
  `docs/TIMESERIES_ROADMAP.md` (new "Milestone 10b-3" section; Milestone
  10b-2's own #19 bullet annotated as fixed, not rewritten; "Phase 5 done
  condition"'s criterion 772 and "Net" paragraph updated),
  `docs/TIMESERIES_PHASE_22_HANDOFF.md` (added `**Superseded by:**` line),
  `docs/handoff_latest.md` (symlink repointed).
- Untouched: `crates/cubism-core/src`, `crates/cubism-iceberg/src`,
  `crates/cubism-datafusion/src/{build,range_query,state_udaf,temporal_build,udaf,udf}.rs`
  (confirmed untouched via `git status` after every verification pass this
  session), `crates/cubism-serve/src`, `crates/cubism-cli/` (built and
  clippy-checked, not modified).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`, unchanged; 67 passed in `cubism-datafusion`, up from 65; 207 passed / 2 ignored in workspace, up from 205)

All figures re-run fresh this session. `cargo test -p cubism-iceberg`
reports 33 passed, 1 ignored (6 suites) — identical to Phase 22, expected
since no source in that crate changed this session. `cargo test -p
cubism-datafusion` reports 67 passed (3 suites) — up from Phase 22's 65 by
two: `series_response_filters_batches_to_the_resolved_selector_before_merging`
and `series_response_rejects_multi_selector_query`. The integration-test
suite count is unchanged at 4 (one existing test modified in place, none
added or removed). `cargo test --workspace --exclude cubism-py` reports
207 passed, 2 ignored (23 suites) — up from Phase 22's 205 by the same two.

## Verification performed

```text
cargo test -p cubism-datafusion --lib series_response                    # 8 passed (6 prior + 2 new)
cargo test -p cubism-datafusion --test iceberg_bridge                    # 4 passed (unchanged, one modified)
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test -p cubism-datafusion                                           # 67 passed (3 suites)
cargo clippy -p cubism-datafusion --all-targets --no-deps -- -D warnings  # clean
cargo test --workspace --exclude cubism-py                                # 207 passed, 2 ignored (23 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

The `cargo test -p cubism-datafusion --lib series_response` line ran twice
this session: once immediately after the code + test changes landed, which
failed and caught the `FixedSizeBinaryArray::try_from_iter`-on-empty-iterator
bug, and once more after that fix, which passed. The rest of the battery
above — `cubism-iceberg`'s full suite plus `concurrency`/`durability`, both
`cubism-cli` steps, the full `cubism-datafusion` suite, and the full
workspace test + clippy — ran once each, after the `try_from_iter` fix was
already in place, on the tree as committed. Figures shown are from that
run.

## Primary files

- [`../crates/cubism-datafusion/src/series_response.rs`](../crates/cubism-datafusion/src/series_response.rs)
  (Milestone 10b-3's `selectors` parameter and `filter_batches_by_xunit`,
  `SeriesResponse::new` at lines 131-180)
- [`../crates/cubism-datafusion/src/series_merge.rs`](../crates/cubism-datafusion/src/series_merge.rs)
  (doc-comment update recording where the #19 fix landed)
- [`../crates/cubism-datafusion/tests/iceberg_bridge.rs`](../crates/cubism-datafusion/tests/iceberg_bridge.rs)
  (new `content_id` helper; updated
  `series_response_materializes_two_published_windows_through_a_real_coverage_plan`)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) ("Milestone
  10b-3" section; "Phase 5 'done' condition," criterion 772 and "Net"
  paragraph updates)
- [`TIMESERIES_PHASE_22_HANDOFF.md`](TIMESERIES_PHASE_22_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#19`](https://github.com/jeromebanks/cubism-rs/issues/19)
  (closed this session — the `XUnit`-selector filtering gap Milestone
  10b-3 fixes)
- GitHub issue [`#8`](https://github.com/jeromebanks/cubism-rs/issues/8)
  (SQL/pushdown gap — completion-criterion 774 stays excluded from Phase
  5's narrow-close against this issue, unchanged by this session)
- GitHub issue [`#3`](https://github.com/jeromebanks/cubism-rs/issues/3)
  (`rustfmt --check` not reproducing clean at HEAD — reconfirmed this
  session against `iceberg_bridge.rs`)
