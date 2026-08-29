# Cubism Iceberg Phase 0A Spike

This private workspace crate is executable feasibility evidence. It is not a
production persistence crate and exposes no supported Cubism API.

## Scope

The spike pins:

- `iceberg = 0.10.0`
- `iceberg-datafusion = 0.10.0`
- `datafusion = 53.1.0`
- Arrow/Parquet 58 through those dependencies

Cubism's existing crates remain on DataFusion 54. Cargo can compile both
versions in the workspace, but their Rust plan/provider types are not
interchangeable. A production integration must eventually converge on one
DataFusion version or keep Iceberg behind a record-batch/service boundary.

## Run

```bash
CARGO_TARGET_DIR=/tmp/cubism-target-phase0a \
  cargo test -p cubism-iceberg-spike
```

The tests use an Iceberg memory catalog backed by a temporary local filesystem.

## Proven

- A day-partitioned Iceberg table can be created from Rust.
- Long-form count state and a binary payload can be appended through
  `iceberg-datafusion` and read back through DataFusion.
- A scan predicate for one day plans one file rather than the two files in the
  table.
- Arrow `Timestamp(Microsecond, "UTC")` input succeeds through the DataFusion
  insert path and reads back canonically as `+00:00`.
- The explicit `normalize_utc_timezones` adapter also preserves timestamp
  instants. It protects paths that do not receive DataFusion's target-schema
  coercion.
- A deterministic run claim selects exactly one append owner.
- A crash after append can be reconciled by `run_id`, expected row count, and
  current aggregate snapshot without repeating the append.
- Readers can scan one current aggregate snapshot and semijoin against current
  manifest revisions:

```text
visible(aggregate_row) iff
  aggregate_row.window_id = manifest.window_id
  AND aggregate_row.revision = manifest.current_revision
```

- An unpublished correction remains invisible; a successful compare-and-swap
  switches visibility; a stale publication is rejected.

## Not proven

- a production REST, Glue, S3 Tables, or SQL catalog;
- S3/GCS/Azure object-store behavior;
- a durable lease/CAS implementation or lease expiration;
- an exact commit-snapshot return value from DataFusion `INSERT` (it returns a
  row count; the spike reloads current metadata in a serial local test);
- direct low-level Iceberg writer behavior for every UTC timezone alias;
- duplicate data-file-path rejection below the DataFusion SQL insert layer;
- orphan cleanup, compaction, overwrite, or snapshot expiration;
- Spark interoperability;
- throughput, file-size targets, or cost;
- any Phase 0B Rust-versus-Spark performance claim.

See [`../../docs/PHASE_0A_RESULTS.md`](../../docs/PHASE_0A_RESULTS.md) for the
decision record.
