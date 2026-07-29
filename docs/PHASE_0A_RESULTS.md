# Phase 0A Iceberg Compatibility and Correctness Results

Status: local correctness gate passed

Date: 2026-07-28

Branch: `feature/timeseries-phase-0a`

Implementation:
[`crates/cubism-iceberg-spike`](../crates/cubism-iceberg-spike)

## Decision

Proceed to review, Phase 1 semantics, and the bounded Phase 0B benchmark. Do not
promote the spike to a production Iceberg layer yet.

The released Iceberg Rust stack can coexist with current Cubism and supports the
minimal append/read path needed by the proposed immutable-revision model.
Phase 0A did not find a local correctness result that invalidates
Rust/DataFusion. It also did not test the production catalog, object store,
maintenance path, or performance boundary.

## Dependency result

The coherent released combination is:

```text
iceberg                  0.10.0
iceberg-datafusion       0.10.0
DataFusion in spike      53.1.0
Arrow/Parquet            58
Rust                     1.96.1
```

Current Cubism remains on DataFusion `54.0.0`. Cargo resolved both DataFusion 53
and 54 in one workspace. That avoids a premature downgrade, but it is only an
isolation strategy: DataFusion plans and providers from the two versions cannot
be passed directly across crate boundaries. Production work must either converge
versions or keep Iceberg behind Arrow record batches or a service boundary.

No floating Git dependency was introduced.

## Executed experiments

### Append and read

The spike creates a long-form aggregate table with:

```text
cube_id
window_id
revision
run_id
bucket_start timestamptz
xunit_id
measure_id
state_count
state_blob
```

It uses hidden `day(bucket_start)` partitioning, appends through DataFusion
`INSERT INTO ... SELECT`, and reads through the catalog-backed Iceberg
`TableProvider`.

Result: passed.

### UTC timestamp behavior

The upstream Iceberg Rust issue describes a failure when an Arrow array uses
timezone `"UTC"` while the Iceberg schema expects `"+00:00"`.

In this exact path, DataFusion 53.1 coerced an input
`Timestamp(Microsecond, "UTC")` to the target Iceberg schema successfully. The
value read back with timezone `"+00:00"` and the same epoch microseconds.

The spike also contains an explicit `normalize_utc_timezones` adapter and proves
that it preserves the timestamp value. This is retained because the open
upstream issue may still affect lower-level writer paths that bypass DataFusion
target-schema coercion.

Result: passed for the tested DataFusion insert path; direct writer remains an
open test.

### Partition pruning

One append wrote rows in two day partitions and produced two planned data files.
An Iceberg scan with:

```text
bucket_start >= 2026-07-28T00:00:00Z
AND bucket_start < 2026-07-29T00:00:00Z
```

planned one file, and the DataFusion query returned one row.

Result: passed at Iceberg file planning. Row-group/page pruning and
production-scale metadata planning remain Phase 0B/Phase 3 measurements.

### Retry and crash recovery

The control-store model atomically claims a deterministic `run_id`.

- Exactly one concurrent claimant receives append ownership.
- Other claimants observe the existing run and must reconcile rather than
  append.
- A simulated crash after Iceberg append but before snapshot recording is
  recovered by querying `run_id` and expected row count, recording the current
  snapshot, and publishing without a second append.
- Reusing a `run_id` with different immutable inputs is rejected.

Result: protocol passed in the in-memory CAS model. A durable backend, leases,
expiry, and recovery under process/network partitions remain unresolved.

The DataFusion `INSERT` result reports inserted row count, not the committed
Iceberg snapshot ID. The serial spike reloads current table metadata after the
append. A production writer must obtain the exact commit result through a
lower-level API or prove that a later recorded snapshot contains the run; simply
assuming that "current" is the writer's snapshot is unsafe with concurrent
writers.

### Publication visibility

The test publishes revision 1 for two windows from different aggregate
snapshots, then appends revision 2 for the first window.

Before publication, the latest aggregate table physically contains both
revisions, while the manifest semijoin returns revision 1. After a successful
compare-and-swap, it returns revision 2. A stale same-window publisher is
rejected.

The reader invariant is:

```text
visible(aggregate_row) iff
  aggregate_row.window_id = manifest.window_id
  AND aggregate_row.revision = manifest.current_revision
```

The reader uses one current/sufficiently recent aggregate snapshot. Per-window
aggregate snapshot IDs remain provenance for validation, audit, rollback, and
garbage collection; they are not separate snapshots opened by a range query.

Result: passed in the local control-store model.

## Verification

Executed:

```text
cargo test -p cubism-iceberg-spike

unit tests:        3 passed
integration tests: 3 passed
doc tests:          0

cargo test --workspace

all workspace unit, property, integration, API, and doc tests passed
```

The integration tests are:

- `append_read_and_day_partition_pruning`
- `named_utc_datafusion_append_and_explicit_normalization_round_trip`
- `retry_recovery_and_manifest_semijoin_control_visibility`

## Remaining gates

Phase 0A does not authorize production implementation. Before a production
Iceberg layer:

1. choose and test a durable catalog/control store with real compare-and-swap;
2. test S3/GCS or the selected object store, including retry and consistency
   behavior;
3. test direct writer/file inventory handling and duplicate-path defense;
4. capture or validate the snapshot containing each run under concurrent
   writers;
5. inject failures around durable commit, publication, and lease expiry;
6. define orphan reconciliation and safe snapshot/file retention;
7. run Phase 0B's bounded schema and local Rust/Spark benchmark;
8. converge DataFusion versions or accept an explicit Arrow/service boundary.

## Phase 0B boundary

Phase 0B should test 10–100 million representative rows, sum/count plus KMV,
sparse and dense occupancy, high-cardinality XUnits, and four layouts:

1. per-measure rows with repeated canonical XUnit;
2. wide per-bucket rows;
3. tagged state structs;
4. stable 256-bit `xunit_id` plus an XUnit registry.

Billion-row and distributed comparisons belong in Phase 7, not the initial
spike.
