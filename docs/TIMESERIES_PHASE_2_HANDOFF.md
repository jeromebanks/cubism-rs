# Time-Series Phase 2 Handoff

Date: 2026-08-12

Branch: `feature/timeseries-phase-0a`

Status: **Phase 2's sparse bucketed incremental aggregation is implemented,
tested, and green** (`cargo test`/`clippy -D warnings` clean across the
touched crates and the full workspace), **CLI-wired, and includes `Coverage`
in build metadata.** Committed and pushed — see "Worktree state" below.

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
- **`TemporalBuildMetadata.coverage: Option<Coverage>`, added in a
  follow-up session.** `None` when no `window` was requested (`Coverage::
  requested` needs a `TimeRange`; an unbounded build has none). `Some`
  otherwise, computed by `compute_coverage`: a `MIN`/`MAX(bucket_start)`
  scan over the (now-cached) states, not a per-bucket occupancy check — it
  can only see a gap at the requested window's edges, not a hole in the
  middle of an otherwise fully-covered window, and `covered` is always at
  most one range. Bucket assignment is independent of the window's edges
  (`window_filter_sql` filters on event time, not bucket bounds), so a
  non-bucket-aligned window's edge bucket can extend past what the row
  filter actually produced; `covered` is clamped to `requested`'s own edges
  before comparison, or it would claim coverage of time guaranteed to have
  zero rows. `Exactness::Exact` requires `covered == requested` after
  clamping — reachable even for a non-bucket-aligned window, as long as
  observed rows fully span it (see `coverage_is_exact_for_a_non_bucket_
  aligned_window_fully_spanned_by_data`). Computing this requires knowing
  the actual observed bucket range, so **windowed builds now execute
  eagerly inside `build_temporal` (via `.cache()`)** — the previously
  lazy-until-`.collect()` behavior (see the next bullet) now only holds for
  *unwindowed* builds; `bucket_start_rejects_a_non_timestamp_event_time_
  column` deliberately uses `window: None` to keep pinning that case.
- **UDF/UDAF errors surface at `.collect()`, not at `build_temporal()` —
  for unwindowed builds only** (windowed builds compute `Coverage` eagerly,
  see above). `ctx.sql()` only builds and analyzes the logical plan; a
  `ScalarUDFImpl`'s `invoke_with_args` (where `cubism_bucket_start`'s
  TIMESTAMP-type check lives) only runs during physical execution.
  `build_temporal` can return `Ok` for a spec whose `eventTime` expression
  turns out to be non-timestamp
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

## Tests (30 in `cubism-datafusion`, all passing)

The 24 below are this phase's original session; the 6 `coverage_*` tests
added in the CLI/`Coverage` follow-up session are listed under Deferred
item 2, not repeated here. Covers the plan's named requirements directly:

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

**Follow-up session (CLI wiring + `Coverage`, same date) re-verified after
its changes:**

```text
cargo test -p cubism-datafusion --lib          # 30 passed (24 above + 6 coverage_* tests)
cargo clippy -p cubism-datafusion --all-targets --no-deps -- -D warnings   # clean
cargo build -p cubism-cli                      # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings          # clean
cargo test --workspace --exclude cubism-py     # 136 passed, 1 ignored, 0 failed
```

Plus manual CLI smoke tests against real Parquet/CSV fixtures (`--explain`
counts, window filtering actually excluding rows, single-file fixture
writes, and the missing-output/mismatched-window/bad-`--null-policy` error
paths — see Deferred item 1). An advisor review of the `Coverage` work
caught one real bug before it shipped (see Deferred item 2 and the
`coverage` field's doc comment): unclamped `covered` bounds could report
coverage of time a non-bucket-aligned window's own row filter excluded.

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

1. ~~**CLI wiring.**~~ Done in a follow-up session: `cubism-cli` gained a
   `temporal-build` subcommand (`crates/cubism-cli/src/main.rs`) wrapping
   `build_temporal`/`explain_temporal_build`/`write_temporal_fixtures`:
   `cubism temporal-build <spec.yaml> --input <events.parquet|csv>
   [--window-start <RFC3339>] [--window-end <RFC3339>]
   [--states-output <path> --registry-output <path>]
   [--null-policy reject|quarantine] [--explain]`. Flag names differ from
   this doc's original guess (`--states-output`/`--registry-output`/
   `--null-policy` instead of unnamed output args). `--explain` runs
   `explain_temporal_build` and prints the dry-run counts; otherwise both
   output paths are required (mirroring `write_temporal_fixtures`'s
   two-path signature) and the command writes real fixtures. Window bounds
   are parsed via `chrono::DateTime::parse_from_rfc3339` (new `cubism-cli`
   dependency) into `EventTime`/`TimeRange`; both flags are required
   together or not at all. Manually verified: `--explain` counts against a
   4-row/2-bucket fixture (both Parquet and CSV input — CSV's RFC3339
   timestamps are inferred as a real timestamp type by DataFusion, no
   dedicated handling needed), a narrowed `--window-start`/`--window-end`
   actually excludes rows (2/1/3 vs. the unwindowed 4/2/5 — proves the flags
   reach `window_filter_sql`, not just parse), single-file fixture writes,
   and the missing-output/mismatched-window/bad-`--null-policy` error
   paths. `cargo test --workspace --exclude cubism-py` (130 passed, 1
   ignored) and `cargo clippy -p cubism-cli --all-targets --no-deps -- -D
   warnings` both clean.
2. ~~**`Coverage` in build metadata**~~ Done in the same follow-up session
   as item 1 — see "Design decisions" above for `compute_coverage`'s
   semantics (MIN/MAX-only, clamped to the requested window, no interior-gap
   detection) and the eager-execution behavior change for windowed builds.
   Six new tests cover: no window (`coverage` is `None`), an exact
   bucket-aligned fit, a window wider than the data (`Inexact`, `covered`
   narrower than requested), a non-aligned window whose edge bucket is
   clamped so it doesn't overclaim past `requested`, `Exact` reachability
   for a non-aligned window fully spanned by data, and zero observed rows.
   `cargo test -p cubism-datafusion --lib` (30 passed) and `cargo clippy -p
   cubism-datafusion --all-targets --no-deps -- -D warnings` both clean.
3. **Performance tests** (generated XUnits/s and peak memory at increasing
   cardinality; spill behavior under a configured memory limit; state-size
   amplification by measure) — the plan's Phase 2 "Performance" test
   category was not exercised this session. `GroupsAccumulator`'s absence
   (see above) means a naive benchmark would currently measure the fallback
   adapter's cost, not the state UDAFs' intrinsic cost — worth resolving
   the accumulator-strategy decision first, or explicitly measuring both.
   Also unmeasured: `compute_coverage`'s `.cache()` on a windowed build
   materializes the full states `DataFrame` into an in-memory `MemTable` —
   unlike the module's own `SELECT DISTINCT` XUnit registry, which the
   header comment specifically justifies as "spillable, bounded-by-
   configuration," this path has neither property. A windowed build at
   Phase 0B's 25M-row scale now holds the whole state table in RAM; an
   unwindowed build is unaffected (still lazy, no `.cache()`). Chosen
   deliberately to avoid double-executing the aggregation for the coverage
   query (see the `coverage` field's doc comment) — worth a real measurement
   before Phase 3 windowed builds run at that scale.
4. **Calendar buckets, dense occupancy at temporal scale, and axis-2/3
   distributed builds** remain entirely out of scope, unchanged from
   Phase 0B/1's own deferrals.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` — two commits, both
on `origin`:

- `c36ba6a` — this phase's original session (`state_udaf.rs`,
  `temporal_build.rs`, the CLI/`Coverage` work not yet started).
- `a76cd65` — the CLI wiring + `Coverage` follow-up session described
  throughout this doc's updated sections.

Working tree is otherwise clean except for two files deliberately left
uncommitted, per prior-session convention: `.serena/` (local tooling
state), `examples/web_analytics_demo/events.csv` (generated demo output,
matching the repo's `examples/` convention).

Changed/new files across both commits:

- New: `crates/cubism-datafusion/src/state_udaf.rs`,
  `crates/cubism-datafusion/src/temporal_build.rs`,
  `docs/TIMESERIES_PHASE_2_HANDOFF.md` (this file).
- Modified: `crates/cubism-datafusion/src/lib.rs` (module wiring),
  `crates/cubism-datafusion/src/build.rs` (`level_columns()` extraction),
  `crates/cubism-datafusion/src/udf.rs` (one doc-comment line, fixing a
  `clippy::doc_lazy_continuation` lint that was blocking `-D warnings` on
  this crate), `crates/cubism-datafusion/Cargo.toml` (new deps),
  `crates/cubism-core/src/aggregate_state.rs` (`QuantileState::bin_count()`
  accessor), `crates/cubism-cli/src/main.rs` (`temporal-build` subcommand),
  `crates/cubism-cli/Cargo.toml` (new `chrono` dep), `Cargo.lock`.

## Primary files

- [`../crates/cubism-datafusion/src/temporal_build.rs`](../crates/cubism-datafusion/src/temporal_build.rs)
- [`../crates/cubism-datafusion/src/state_udaf.rs`](../crates/cubism-datafusion/src/state_udaf.rs)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md) (Phase 2 spec, lines 381-464)
- [`TIMESERIES_PHASE_0B_RESULTS.md`](TIMESERIES_PHASE_0B_RESULTS.md) (the decision this phase builds on)
- [`TIMESERIES_PHASE_0B_HANDOFF.md`](TIMESERIES_PHASE_0B_HANDOFF.md)
