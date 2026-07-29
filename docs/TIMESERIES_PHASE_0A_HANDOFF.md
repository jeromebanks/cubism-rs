# Time-Series Phase 0A Handoff

Date: 2026-07-28

Branch: `feature/timeseries-phase-0a`

Status: Phase 0A local correctness gate passed; changes are uncommitted

## Executive handoff

Phase 0A supports the planned sparse Rust/DataFusion/Iceberg architecture. It
did not reveal a reason to restore legacy dense time arrays, make time part of
the XUnit lattice, or move Cubism's aggregation/query kernel back to Spark.

It did change the execution plan in four material ways:

1. Run Phase 1's temporal and aggregate-state contract before Phase 0B, so the
   benchmark measures a frozen sum/count/KMV representation rather than a
   throwaway schema.
2. Treat DataFusion version convergence as a production gate. Released Iceberg
   Rust `0.10.0` uses DataFusion `53.1.0`; current Cubism uses `54.0.0`. Both
   compile in one workspace, but their plan/provider Rust types are not
   interchangeable.
3. Add an exact-commit-snapshot requirement. DataFusion `INSERT` returns an
   inserted-row count, not the snapshot ID it committed. A concurrent production
   writer must use a lower-level commit result or validate that a recorded later
   snapshot contains its deterministic run.
4. Make retry ownership and recovery part of the control-store contract.
   Iceberg append is not assumed idempotent: exactly one claimant may append,
   while retries reconcile by deterministic `run_id` and expected row count.

The manifest reader design is now concrete:

```text
visible(aggregate_row) iff
  aggregate_row.window_id = manifest.window_id
  AND aggregate_row.revision = manifest.current_revision
```

A range query reads one current/sufficiently recent aggregate snapshot and
semijoins the current manifest. Per-window aggregate snapshot IDs are provenance,
not separate snapshots opened for each window.

## What was implemented

An isolated, non-production workspace member:

```text
crates/cubism-iceberg-spike/
├── Cargo.toml
├── README.md
├── src/lib.rs
└── tests/phase0a.rs
```

It pins:

```text
iceberg                  0.10.0
iceberg-datafusion       0.10.0
DataFusion in spike      53.1.0
Arrow/Parquet            58
Rust                     1.96.1
```

Current Cubism remains on DataFusion `54.0.0`. `Cargo.lock` is large because it
contains both DataFusion dependency graphs plus Iceberg's dependencies. No
floating Git dependency was introduced.

The spike contains:

- a day-partitioned long-form aggregate Iceberg table;
- real DataFusion append/read tests;
- UTC timezone normalization for lower-level writer safety;
- file-plan pruning assertions;
- deterministic run claims with exactly one append owner;
- crash-after-append recovery;
- compare-and-swap publication state;
- manifest semijoin visibility tests across windows and revisions.

## What we learned

### Architecture remains viable

Iceberg Rust append/read worked locally. A two-day append planned two data
files; a one-day predicate planned one. An unpublished replacement remained
physically present but invisible through the manifest semijoin. These results
support immutable sparse window revisions over long-form rows.

### The dependency mismatch is manageable only as a spike

Cargo compiled DataFusion 53.1 and 54.0 together. That proves the released
Iceberg stack can be evaluated without downgrading current Cubism. It does not
justify carrying two DataFusion versions in production.

Before Phase 3, choose one:

1. move Cubism to the DataFusion version supported by a released Iceberg Rust;
2. wait for/retest a released Iceberg Rust compatible with Cubism's DataFusion;
3. accept an explicit Arrow record-batch or service boundary between the two.

Do not pass DataFusion plans/providers across the version boundary.

### The UTC issue is narrower than expected

The open upstream issue describes `"UTC"` versus `"+00:00"` rejection in the
low-level writer. The tested DataFusion 53.1 `INSERT` path coerced named `"UTC"`
successfully and read it back as `"+00:00"` with unchanged epoch microseconds.

Keep the explicit normalizer and direct-writer test on the remaining-gates list.
Do not claim all Iceberg Rust writer paths are fixed.

### DataFusion INSERT hides the exact snapshot result

This is the most important new production risk. The insert result contains only
the row count. The serial spike reloads current metadata, but in a concurrent
system "current" may already be another writer's snapshot.

Production options to investigate:

- use Iceberg's lower-level writer/transaction API and retain its commit result;
- record a later snapshot only after querying and validating that it contains
  the deterministic `run_id` and expected rows;
- include a catalog sequence/high-water mark in publication metadata.

The manifest's snapshot field should mean "validated snapshot containing this
revision" unless the writer can prove it is the exact commit snapshot.

### Retry safety belongs above append

The control-store model returns `ClaimResult::New` to exactly one concurrent
claimant. Only that caller may append. `ClaimResult::Existing` callers must
reconcile, never append blindly.

The production control store still needs leases, expiry, durable CAS, and
failure injection under process/network partitions.

## Verification completed

```text
cargo test -p cubism-iceberg-spike
  3 unit tests passed
  3 integration tests passed

cargo test --workspace
  all unit, property, integration, API, and doc tests passed

cargo clippy -p cubism-iceberg-spike --all-targets -- -D warnings
  passed

cargo fmt -p cubism-iceberg-spike -- --check
  passed
```

Workspace-wide strict Clippy is not clean because of a pre-existing
`doc_lazy_continuation` warning at
`crates/cubism-datafusion/src/udf.rs:68`. The file was clean in Git before this
work and was not changed.

Use the temporary target directory to avoid repository build-lock and duplicate
DataFusion build artifacts:

```bash
CARGO_TARGET_DIR=/tmp/cubism-target-phase0a \
  cargo test -p cubism-iceberg-spike
```

All shell commands in this repository must be prefixed with `rtk` per the
session-provided `AGENTS.md` instructions.

## Worktree state and ownership

Expected current status:

```text
feature/timeseries-phase-0a
 M Cargo.lock
 M Cargo.toml
 M README.md
?? crates/cubism-iceberg-spike/
?? docs/PHASE_0A_RESULTS.md
?? docs/TIMESERIES_FEASIBILITY.md
?? docs/TIMESERIES_IMPLEMENTATION_PLAN.md
?? docs/TIMESERIES_PHASE_0A_HANDOFF.md
?? examples/web_analytics_demo/
```

Phase 0A owns:

- `Cargo.toml` workspace-member addition;
- Iceberg/DataFusion additions in `Cargo.lock`;
- `crates/cubism-iceberg-spike/`;
- the four time-series documents in `docs/`.

Pre-existing/user work that must be preserved:

- `README.md`;
- `examples/web_analytics_demo/`.

The branch has not been committed or pushed.

## Recommended next session

1. Read this handoff and [`PHASE_0A_RESULTS.md`](PHASE_0A_RESULTS.md).
2. Inspect `git status` and preserve README/web-demo changes.
3. Review the spike and decide whether to commit Phase 0A as its own commit.
4. Proceed with Phase 1 pure semantics:
   - fixed UTC half-open bucket types;
   - event time versus ingestion time;
   - `v2alpha1` temporal spec;
   - aggregate-state capabilities;
   - average `(sum,count)`;
   - variance `(count,mean,m2)`;
   - stable canonical XUnit bytes/content ID;
   - truthful KMV/Top-K/centroid merge declarations.
5. Freeze sum/count/KMV state and table semantics.
6. Run Phase 0B's bounded local Rust-versus-Spark/layout benchmark.
7. Before Phase 3, resolve DataFusion convergence and exact snapshot capture.

Do not start production Iceberg persistence, compaction, distributed execution,
or the billion-row benchmark from this handoff.

## Primary files

- [`PHASE_0A_RESULTS.md`](PHASE_0A_RESULTS.md)
- [`TIMESERIES_FEASIBILITY.md`](TIMESERIES_FEASIBILITY.md)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
- [`../crates/cubism-iceberg-spike/README.md`](../crates/cubism-iceberg-spike/README.md)
- [`../crates/cubism-iceberg-spike/src/lib.rs`](../crates/cubism-iceberg-spike/src/lib.rs)
- [`../crates/cubism-iceberg-spike/tests/phase0a.rs`](../crates/cubism-iceberg-spike/tests/phase0a.rs)
