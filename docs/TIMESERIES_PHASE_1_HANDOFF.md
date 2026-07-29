# Time-Series Phase 1 Handoff

Date: 2026-07-29

Branch: `feature/timeseries-phase-0a`

Status: Phase 0A and Phase 1 complete; Phase 0B is the next gate

## Executive handoff

Phase 1 establishes pure, engine-independent temporal and aggregate-state
semantics in `cubism-core`. Existing static `v1` specs and artifacts remain
supported. Temporal specs are explicit `cubism/v2alpha1`; a dimension named
`time` is never inferred or reinterpreted.

The next phase is **Phase 0B**, the bounded local Rust/DataFusion-versus-Spark
and state-layout benchmark. Do not begin Phase 2 temporal DataFusion execution
or Phase 3 production Iceberg persistence until Phase 0B selects:

1. the local Rust/Spark execution boundary;
2. the durable logical state layout;
3. the stable XUnit registry/content-ID layout;
4. representative performance and memory limits.

Phase 0B must use the Phase 1 formats as the common semantic reference rather
than benchmark engine-native scalar outputs with different merge behavior.

## What Phase 1 implemented

### Temporal types

`crates/cubism-core/src/temporal.rs` defines:

- `EventTime` and `IngestionTime` as distinct UTC microsecond types;
- validated non-empty half-open `TimeRange`;
- `BucketStart`, `BucketEnd`, `TimeBucket`, `FixedResolution`, and
  `BucketOrigin`;
- Euclidean fixed-bucket assignment for timestamps before the origin;
- `AllowedLateness`, including zero lateness;
- parsed-but-gated calendar resolutions;
- `Coverage`, `Exactness`, and explicit missing-versus-present bucket values;
- validated `WindowId` and non-zero `WindowRevision`;
- `TemporalSpec` with event/ingestion expressions, base resolution, origin,
  UTC timezone, lateness, rollups, and retention.

Only fixed-duration UTC buckets are supported. Calendar resolutions parse so
the system can reject them explicitly instead of silently approximating them.

### Versioned temporal specifications

`crates/cubism-core/src/spec.rs` adds:

- `cubism/v2alpha1`;
- an explicit `temporal` section;
- per-measure `AggregateStateConfig`;
- semantic BLAKE3-256 `spec_hash`;
- `variance` as an aggregate kind;
- validation for state versions and kind-specific parameters.

Compatibility rules:

- `v1` behavior is unchanged;
- `v1` rejects `temporal` and explicit state parameters rather than ignoring
  them;
- `v2alpha1` requires `temporal`;
- no YPath name is treated as event time;
- temporal quantiles require an explicit fixed bin width;
- temporal Top-K is rejected because the current pruned merge is not
  associative.

### Canonical XUnit identity

`crates/cubism-core/src/encoding.rs` preserves the existing build-local
dictionary encoding for static artifacts and adds a separate canonical format:

- versioned `CXU` bytes;
- normalized dimension ordering;
- hierarchy-order preservation within each YPath;
- typed null, boolean, signed/unsigned integer, finite float, UTF-8 string, and
  byte values;
- rejection of duplicate dimensions, non-finite floats, negative zero payloads,
  truncation, and non-canonical ordering;
- stable BLAKE3-256 `XUnitContentId`;
- golden bytes and content-ID tests.

The build-local dictionary key remains valid only within one build. Phase 0B
must use canonical bytes or `XUnitContentId` for cross-build layout tests.

### Aggregate-state contract

`crates/cubism-core/src/aggregate_state.rs` defines:

- validated `StateVersion`;
- `AggregateState` with accumulate, merge, present, encode/decode, and
  capability methods;
- conservative `AggregateCapabilities`;
- average state as `(sum, count)`;
- Welford/Chan variance state as `(count, mean, m2)`;
- deterministic sparse fixed-width quantile histogram state;
- versioned golden payloads.

Capability declarations are intentionally conservative:

- KMV and exemplar sampling advertise lawful associative/commutative/idempotent
  merges;
- floating sum, average, variance, and centroid do not claim exact
  associativity;
- centroid declares a numerical tolerance;
- current Top-K does not claim associativity;
- no non-implemented subtraction path is advertised.

Phase 0B only needs sum, count, and KMV. Average, variance, and quantile formats
are frozen reference states for later phases, not a reason to broaden the
benchmark.

## Intentional Phase 1 boundary

Phase 1 did **not** add:

- a temporal DataFusion build path;
- temporal UDAFs or Arrow state batches;
- local Parquet temporal fixtures;
- Iceberg production schemas or commits;
- late-window publication/replacement;
- range queries, gap filling, rolling windows, or rollups;
- compaction, distributed execution, or a production control store.

`cubism-datafusion/src/build.rs` only gained an exhaustive guard that reports
`variance` as unsupported by the current static engine. This preserves a
compiling workspace without pretending Phase 2 exists.

## Verification

Verified with an isolated target directory:

```text
CARGO_TARGET_DIR=/tmp/cubism-target-phase1 cargo test -p cubism-core
  91 tests passed across 4 suites

CARGO_TARGET_DIR=/tmp/cubism-target-phase1 cargo test --workspace
  106 tests passed across 14 suites

CARGO_TARGET_DIR=/tmp/cubism-target-phase1 \
  cargo clippy -p cubism-core --all-targets -- -D warnings
  passed

rustfmt --edition 2024 --check <Phase 1 Rust files>
  passed

git diff --check
  passed
```

The workspace API integration test requires localhost-bind permission. Its
first sandboxed run failed with `Operation not permitted`; the permitted rerun
passed.

Workspace-wide `cargo fmt --check` is not a Phase 1 gate because pre-existing
core files are not normalized by the current rustfmt version. Only Phase 1
files were formatted; unrelated formatting churn was removed.

## Phase 0B next-session scope

Use the exact Phase 0B contract in
[`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md):

- 10–100 million representative rows;
- sparse irregular and dense regular inputs;
- high-cardinality XUnits with realistic lattice restrictions;
- sum/count and KMV only;
- aligned and non-aligned short/long range reads;
- Rust/DataFusion and local Spark on the same host and source files;
- four layouts:
  1. per-measure rows with repeated canonical XUnit;
  2. wide per-bucket rows;
  3. tagged state structs;
  4. stable 256-bit `xunit_id` plus an XUnit registry.

Pin partitioning, sort order, file targets, hardware, concurrency, cache state,
and correctness semantics. Record at least five measured runs after warm-up for
each retained configuration.

Measure:

- rows/s and wall time;
- startup and commit latency;
- peak RSS, CPU, spill, and shuffle;
- files and bytes scanned;
- query p50/p95;
- physical state and metadata size.

Completion requires a recorded schema/layout decision and an evidence-backed
local Rust/Spark boundary. Billion-row and distributed comparisons remain
Phase 7 work.

## Decisions Phase 0B must not silently change

- Time is an event-time bucket axis around the sparse XUnit lattice.
- Bucket intervals are UTC and half-open.
- Missing buckets are not zero-valued buckets.
- The build must never materialize empty buckets or dimension-value Cartesian
  products.
- Raw build-local XUnit dictionary bytes are not cross-build identity.
- Presented scalar values are not authoritative merge state.
- Late data will rebuild and atomically replace a complete window revision;
  additive correction is not the design.
- Phase 0B is a benchmark and layout decision, not a production Iceberg layer.

## Recommended next session

1. Read this handoff, `PHASE_0A_RESULTS.md`, and the Phase 0B section of
   `TIMESERIES_IMPLEMENTATION_PLAN.md`.
2. Confirm the Phase 1 golden formats and tests are still green.
3. Design the smallest reproducible Phase 0B harness before generating large
   data.
4. Run a small correctness fixture through every candidate layout.
5. Scale retained layouts to 10–100 million rows and collect repeatable
   measurements.
6. Write a Phase 0B results document with raw commands, environment, artifacts,
   correctness checks, and the selected layout/execution boundary.
7. Only after the Phase 0B decision, begin Phase 2 sparse temporal aggregation.

## Primary files

- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
- [`PHASE_0A_RESULTS.md`](PHASE_0A_RESULTS.md)
- [`TIMESERIES_PHASE_0A_HANDOFF.md`](TIMESERIES_PHASE_0A_HANDOFF.md)
- [`../crates/cubism-core/src/temporal.rs`](../crates/cubism-core/src/temporal.rs)
- [`../crates/cubism-core/src/aggregate_state.rs`](../crates/cubism-core/src/aggregate_state.rs)
- [`../crates/cubism-core/src/encoding.rs`](../crates/cubism-core/src/encoding.rs)
- [`../crates/cubism-core/src/spec.rs`](../crates/cubism-core/src/spec.rs)
- [`../crates/cubism-core/tests/phase1_properties.rs`](../crates/cubism-core/tests/phase1_properties.rs)
