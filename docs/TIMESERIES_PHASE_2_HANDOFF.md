# Time-Series Phase 2 Handoff

Date: 2026-08-12

Branch: `feature/timeseries-phase-0a`

Status: **Phase 2's sparse bucketed incremental aggregation is implemented,
tested, and green** (`cargo test`/`clippy -D warnings` clean across the
touched crates and the full workspace). Not yet committed — see "Worktree
state" below.

## What this session built

Two new modules in `cubism-datafusion`, per the plan
(`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` lines 381-464):

- **`state_udaf.rs`** (567 lines): authoritative, re-mergeable DataFusion
  UDAFs for `avg`/`variance`/`quantile` — the three measure kinds
  `build.rs`'s static engine still can't do (`cube_sql` explicitly errors on
  `Variance`/`Quantile`; `Avg` there is a bare, non-mergeable scalar). Each
  accumulator's `state()`/`evaluate()` emit `cubism_core::aggregate_state`'s
  `encode()` bytes and `merge_batch` decodes+`merge()`s — the same
  blob-in/blob-out pattern `udaf.rs`'s existing sketch UDAFs already use for
  `count_distinct`/`top_k`/`reservoir_sample`/`centroid`, extended to the
  three kinds Phase 1 gave a state contract to but nothing implemented yet.
  Quantile gets one UDAF instance per measure (`cubism_quantile_state__
  <name>`) since its bin width is a spec-level constant, not a SQL argument.

- **`temporal_build.rs`** (1,726 lines): the temporal build pipeline itself.
  `build_temporal()` compiles a spec into one `WITH`-chain (mirroring
  `build.rs::cube_sql`'s structure) that:
  1. evaluates `temporal.eventTime` and assigns one half-open UTC bucket via
     a new `cubism_bucket_start` scalar UDF, which reuses
     `cubism_core::temporal::FixedResolution::bucket`'s exact euclidean math
     directly (not re-derived) and explicitly normalizes every Arrow
     timestamp unit (s/ms/us/ns) to microseconds — no blind
     `CAST(... AS BIGINT)`, which is the exact bug this project's own Spark
     adapter hit at seconds-vs-micros;
  2. generates the same row-local XUnits the static path would, via the
     same `cubism_xunit_keys` UDF and a `level_columns()` helper factored
     out of `build.rs` for both paths to share (no re-derived lattice
     logic);
  3. appends bucket identity to `GROUP BY` as a plain extra column — the
     lattice/XUnit code never sees time;
  4. accumulates state via native `SUM`/`COUNT`/`MIN`/`MAX` (already
     mergeable), the existing sketch UDAFs for
     `count_distinct`/`reservoir_sample`/`centroid`, and the new state
     UDAFs for `avg`/`variance`/`quantile`; `top_k` is rejected (both at
     spec-validation time in `cubism-core` and defensively here);
  5. emits Arrow batches sorted by `(bucket_start, xunit_id)`, typed per a
     new `temporal_state_schema()` — a declared authority (versioned column
     names `<measure>_v<N>`, not SQL type inference), matching the
     Phase 0B-selected `xunit_registry` layout's shape generalized from that
     benchmark's fixed sum/count/kmv trio to any spec's measures.

  The XUnit registry (`xunit_id -> canonical bytes`) is built as its own
  `SELECT DISTINCT` pass over the exploded rows, not an inline
  process-global map — the module's own header comment explains why an
  inline accumulate-as-you-go registry would be unsafe under DataFusion's
  `Volatility::Immutable` contract (unlike the per-build XUnit dictionary,
  whose correctness doesn't depend on invocation count). `cubism_xunit_
  content_id`/`cubism_xunit_canonical_bytes` are pure functions of
  `(key, dictionary)`.

  Also included: `write_temporal_fixtures()` (writes the two tables to
  local Parquet — no Iceberg publication, that's Phase 3, per scope) and
  `explain_temporal_build()` (a real dry-run: source rows, generated
  XUnits, observed buckets, output rows, quarantined rows) — not yet wired
  into `cubism-cli`, see "Deferred" below.

- **Small refactors**, not new surface: `build.rs` gained `pub(crate) fn
  level_columns()` (extracted, reused by both paths) and `aggregate_state.rs`
  gained `QuantileState::bin_count()` (a trivial accessor `size()` needed).
  `cubism-datafusion`'s `Cargo.toml` gained `chrono` (RFC3339 window-bound
  literals) and `serde_json`/`proptest` (already used elsewhere in the repo
  the same way).

## Design decisions worth knowing before extending this

- **`WindowId` is a pass-through, not yet meaningful.** `build_temporal`
  takes `window_id: Option<WindowId>` and threads it into
  `TemporalBuildMetadata` unvalidated — it exists for Phase 3's
  idempotency/publication tracking, per the plan's "temporal build entry
  point taking a TimeRange/WindowId," but nothing in Phase 2 interprets it
  yet.
- **`build_temporal` takes `window_id: Option<WindowId>`** and threads it
  unvalidated into `TemporalBuildMetadata` — it exists for Phase 3's
  idempotency/publication tracking, per the plan's "temporal build entry
  point taking a TimeRange/WindowId." Nothing in Phase 2 interprets it yet.
- **No `Coverage` in the metadata yet.** The plan's "build metadata and
  source coverage" bullet is only half done: `TemporalBuildMetadata` has
  `window`/`window_id`/`null_event_time_rows` but not a `Coverage`
  (requested vs. actually-observed event-time range, per
  `cubism_core::temporal::Coverage`). Computing it correctly requires
  materializing the states `DataFrame` once (via `.cache()`) so a coverage
  query and the caller's own `.collect()` don't re-run the whole
  aggregation twice — deliberately left out this session rather than risk
  a subtle double-execution bug under time pressure. Straightforward to add
  next: cache `states`, derive `covered` from `MIN(bucket_start)`/
  `MAX(bucket_start) + resolution`, compare against `window`.
- **UDF/UDAF errors surface at `.collect()`, not at `build_temporal()`.**
  `ctx.sql()` only builds and analyzes the logical plan; a `ScalarUDFImpl`'s
  `invoke_with_args` (where `cubism_bucket_start`'s TIMESTAMP-type check
  lives) only runs during physical execution. `build_temporal` can return
  `Ok` for a spec whose `eventTime` expression turns out to be non-timestamp
  at runtime — the error appears the first time a caller collects
  `output.states`. Caught by this session's own
  `bucket_start_rejects_a_non_timestamp_event_time_column` test, which
  originally (wrongly) asserted on `build_temporal`'s own return value.
- **`temporal_state_schema()`'s nullability matches DataFusion's actual
  planner output, not semantic truth.** Every column is non-null in
  practice (a `GROUP BY` cell always has ≥1 contributing row), but
  DataFusion conservatively marks `GROUP BY` keys (`bucket_start`,
  `xunit_id`) and most aggregate outputs `nullable: true` regardless —
  `COUNT` is the one exception it knows is always non-null. The schema
  function's own doc comment and the end-to-end test's schema assertion
  (full `(name, type, nullable)` equality, not just name/type) both encode
  this; declaring stricter nullability than DataFusion emits would make
  that assertion — and downstream Parquet writes, which use DataFusion's
  real schema regardless of this function's claims — permanently
  inconsistent with reality.
- **No `GroupsAccumulator` for the new state UDAFs.** `state_udaf.rs`'s
  Average/Variance/Quantile accumulators rely on DataFusion's
  `GroupsAccumulatorAdapter` fallback. `udaf.rs`'s existing sketch kernels
  measured a 19x cost for skipping the vectorized path on a
  high-cardinality `GROUP BY` — expect a similar gap here. This is not an
  oversight: "one multi-measure accumulator versus measure-specific plans"
  is explicitly listed as an unresolved Phase 2 decision in the
  implementation plan, so it wasn't pre-optimized.
- **Sum/Min/Max/Avg/Variance/Quantile measure inputs are always cast to
  `DOUBLE`** in the generated SQL, unlike `build.rs`'s static path (which
  keeps the source column's native type for `Sum`/`Min`/`Max`). This keeps
  `temporal_state_schema()` a fixed, declared authority instead of
  something that depends on each spec's column types — consistent with
  `capabilities_for(AggKind::Sum)` already declaring floating,
  non-associative-under-bytes semantics with a `1e-12` tolerance.
- **`NullEventTimePolicy::Reject`'s pre-check scans the whole source table**
  for null event times, independent of any requested window — a null event
  time can't be known to be in/out of scope, so this fails safe rather than
  silently under-checking. `Quarantine` drops those rows via a `WHERE
  __event_time IS NOT NULL` filter and reports the count.
- **`SessionContext` reuse is sequential-only.** `build_temporal` registers
  UDFs/UDAFs under fixed names on the passed `ctx`; DataFusion resolves
  function names to their concrete `Arc` at `ctx.sql().await` (planning)
  time, not at `collect()` time, so repeated *sequential* calls on one
  context are safe (tested — see `retrying_the_build_is_idempotent`), but
  concurrent overlapping calls on the same context would race on
  registration. Documented in the function's doc comment; every test uses a
  fresh context per build.

## Tests (24 in `cubism-datafusion`, all passing)

Covers the plan's named requirements directly:

- **Schema authority**: `temporal_state_schema_names_and_types_measures_by_kind`
  (column naming/typing per `AggKind`), plus the end-to-end test asserting
  the *actual* built `DataFrame`'s schema equals the declared function's
  output — name, type, **and nullability** — catching drift between the
  two, not just each in isolation.
- **One event -> static-equivalent lattice; unobserved combinations
  absent; rules prune identically**: `filter_rules_prune_identically_
  static_and_temporal` (registry's distinct XUnit set == static build's
  distinct XUnit set, both normalized to content IDs, under a
  `max_dimensions` rule).
- **Bucket boundaries half-open**: `event_at_bucket_boundary_goes_to_the_
  correct_half_open_bucket`.
- **Timestamp unit normalization** (the exact bug class the Spark adapter
  hit): `bucket_start_normalizes_every_timestamp_unit_to_the_same_bucket`
  (seconds/millis/micros/nanos, including a pre-origin negative instant, all
  producing the same bucket) and
  `bucket_start_rejects_a_non_timestamp_event_time_column`.
- **Null event time policy**: `null_event_time_reject_errors_and_
  quarantine_drops_rows`.
- **Duplicate/retry handling explicit**: `retrying_the_build_is_idempotent`.
- **Local Parquet fixture writes**:
  `write_temporal_fixtures_writes_single_readable_parquet_files` — asserts
  `write_parquet` actually produced single files (not the directory of part
  files DataFusion writes by default; fixed via
  `DataFrameWriteOptions::with_single_file_output(true)`) and that they
  round-trip through a fresh `SessionContext` with the expected row counts.
- **Sorted output**: the end-to-end test asserts `(bucket_start, xunit_id)`
  ascending order across the whole collected batch stream, not just within
  one batch.
- **End-to-end hand-computed cells**: `end_to_end_temporal_matches_hand_
  computed_cells_and_declared_schema` (sum/count/avg/count_distinct,
  decoded and checked against hand-computed values across two buckets, in
  the style of `build.rs`'s own `end_to_end_cube_matches_hand_computed_cells`).
- **Sparse row count bounded by row-local XUnits, not a distinct-value
  Cartesian product**: `sparse_row_count_is_bounded_by_row_local_xunits_
  not_cartesian` — the oracle is computed directly from
  `cubism_core::lattice::generate_xunits` + `DimensionSpec::
  ypaths_for_values` (the same code `cubism_xunit_keys` calls), not a
  re-derived approximation; asserts the real build's row count equals that
  oracle exactly and is far below the dense `1 + D + R + D*R` bound.
- **Temporal output projected without bucket identity equals a lawful
  merge of static output** (proptest, 24 cases):
  `temporal_merge_without_bucket_equals_static` — random small row sets;
  sum/count compared exactly (integer measures, no tolerance needed); avg
  compared by decoding each bucket's `AverageState` blob, merging them
  across buckets via `AggregateState::merge` (not just re-adding the raw
  numbers), and checking the merged `present()` against static's plain
  `AVG()` scalar within `1e-9` — the actual law `state_udaf.rs` exists to
  make true, not just sum/count's already-trivial mergeability. Both paths'
  XUnits are normalized to content IDs (static emits a string, temporal
  emits a 32-byte ID) before comparing.
- Plus `state_udaf.rs`'s own 5 tests: average/variance accumulate+round-trip
  through real DataFusion queries, average merge-across-partial-states
  equals single-pass, quantile UDAF naming/construction per measure.

## Verification performed

```text
cargo test -p cubism-datafusion --lib          # 24 passed
cargo clippy -p cubism-datafusion --all-targets --no-deps -- -D warnings   # clean
cargo build --workspace --exclude cubism-py    # clean (cubism-cli, cubism-timeseries-bench
                                                #  unaffected by the level_columns refactor)
cargo test --workspace --exclude cubism-py     # all crates green, including
                                                #  cubism-timeseries-bench and
                                                #  cubism-iceberg-spike's Phase 0A tests
```

An independent review pass (advisor) caught five real gaps before this was
considered done: `write_temporal_fixtures` had never actually been
executed (it was silently writing a directory of part files, not the
single file Phase 3 will expect — fixed via `with_single_file_output`);
the schema-equality test compared only `(name, type)`, silently accepting
a nullability mismatch between the declared schema and DataFusion's real
output; the Second/Millisecond/Nanosecond branches of the timestamp-unit
normalizer were never exercised by any fixture; sortedness was asserted
by no test; and the merge-equivalence property test covered sum/count
(trivially mergeable) but not avg (the actual point of `state_udaf.rs`).
All five are now covered by the tests listed above.

**`rustfmt --edition 2024 --check` was attempted and found not to reproduce
clean** on this codebase generally — even files this session never touched
(`udaf.rs`, and by extension likely others) show diffs under plain
`rustfmt` with no `rustfmt.toml` present in the repo, despite
`TIMESERIES_PHASE_0B_RESULTS.md` recording a clean run. Whatever produced
that prior clean state (a different rustfmt version, a since-removed
config) isn't reproducible in this environment. Rather than mass-reformat
files outside this session's actual scope, new/touched files were
hand-formatted reasonably and this discrepancy is flagged here rather than
silently worked around.

## Deferred / not done this session

1. **CLI wiring.** `explain_temporal_build`/`build_temporal`/
   `write_temporal_fixtures` are not exposed via `cubism-cli` (`main.rs`
   still only has `validate`/`run`/`serve`). Straightforward to add
   (`cubism temporal-build <spec.yaml> --input ... [--window-start]
   [--window-end] [--explain]`) following the existing `run` command's
   pattern.
2. **`Coverage` in build metadata** — see "Design decisions" above.
3. **Performance tests** (generated XUnits/s and peak memory at increasing
   cardinality; spill behavior under a configured memory limit; state-size
   amplification by measure) — the plan's Phase 2 "Performance" test
   category was not exercised this session. `GroupsAccumulator`'s absence
   (see above) means a naive benchmark would currently measure the fallback
   adapter's cost, not the state UDAFs' intrinsic cost — worth resolving
   the accumulator-strategy decision first, or explicitly measuring both.
4. **Calendar buckets, dense occupancy at temporal scale, and axis-2/3
   distributed builds** remain entirely out of scope, unchanged from
   Phase 0B/1's own deferrals.

## Worktree state

Committed this session (see git log on `feature/timeseries-phase-0a` for the
exact commit). Changed/new files:

- New: `crates/cubism-datafusion/src/state_udaf.rs`,
  `crates/cubism-datafusion/src/temporal_build.rs`,
  `docs/TIMESERIES_PHASE_2_HANDOFF.md` (this file).
- Modified: `crates/cubism-datafusion/src/lib.rs` (module wiring),
  `crates/cubism-datafusion/src/build.rs` (`level_columns()` extraction),
  `crates/cubism-datafusion/src/udf.rs` (one doc-comment line, fixing a
  `clippy::doc_lazy_continuation` lint that was blocking `-D warnings` on
  this crate), `crates/cubism-datafusion/Cargo.toml` (new deps),
  `crates/cubism-core/src/aggregate_state.rs` (`QuantileState::bin_count()`
  accessor), `Cargo.lock`.
- Deliberately left uncommitted/untouched, per prior-session convention:
  `.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
  (generated demo output, matching the repo's `examples/` convention).

## Primary files

- [`../crates/cubism-datafusion/src/temporal_build.rs`](../crates/cubism-datafusion/src/temporal_build.rs)
- [`../crates/cubism-datafusion/src/state_udaf.rs`](../crates/cubism-datafusion/src/state_udaf.rs)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md) (Phase 2 spec, lines 381-464)
- [`TIMESERIES_PHASE_0B_RESULTS.md`](TIMESERIES_PHASE_0B_RESULTS.md) (the decision this phase builds on)
- [`TIMESERIES_PHASE_0B_HANDOFF.md`](TIMESERIES_PHASE_0B_HANDOFF.md)
