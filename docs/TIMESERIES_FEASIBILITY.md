# Time-Series Feasibility: Cubism on DataFusion and Iceberg

Status: design proposal, not implemented

Evaluated: 2026-07-28

Extends: the workspace-root [`PLAN.md`](../../PLAN.md); it does not replace that
plan

## Executive conclusion

First-class time-series analytics fit Cubism's sparse Rust architecture, but time
must become an axis orthogonal to XUnit identity rather than another YPath
dimension.

The recommended logical model is:

```text
XUnit -> ordered, observed temporal buckets -> mergeable aggregate states
```

The recommended query and durable representation is a sparse, long-form table
whose key is conceptually:

```text
(cube_id, spec_hash, xunit, resolution, bucket_start, measure_id, revision)
```

Only buckets produced by observed events and XUnits allowed by the existing
specification are written. Missing buckets remain absent. Gap filling is an
explicit query operation and must distinguish "missing" from a measured zero.

The architectural recommendation is **conditionally proceed with a hybrid
Rust/DataFusion/Iceberg design**:

- Keep XUnit generation, aggregate-state construction, range querying, and the
  low-latency serving path in Rust and DataFusion.
- Use Iceberg as the canonical metadata and snapshot layer over Parquet.
- Keep Spark as an optional, transient maintenance or large-backfill engine until
  released Iceberg Rust libraries prove that replacement, compaction, concurrent
  commit, and timestamp behavior meet Cubism's requirements. This does not imply
  an always-on Spark cluster.

Rust/DataFusion remains well justified for embedded and single-node execution,
low startup overhead, Arrow-native UDAFs, and moderate scale. It is not yet
justified to claim that this stack universally outperforms Spark. Spark has the
stronger verified path for mature distributed shuffle, structured streaming,
Iceberg overwrite/upsert operations, and table maintenance.

Phase 0A was the local correctness gate. Cubism uses DataFusion `54.0.0`,
while the released `iceberg`/`iceberg-datafusion` Rust `0.10.0` workspace pins
DataFusion `53.1.0`. Iceberg Rust `main` uses DataFusion `54.1.0`, but is not a
released compatibility target. The completed spike compiled both released
versions in one workspace and proved the local append/read, timestamp, file
pruning, retry-control, and invisible-until-published revision paths. This is an
isolation result, not a decision to retain two DataFusion versions in
production. Phase 0B performance and production catalog/object-store gates
remain open.

## Evidence boundary

The findings about current Cubism behavior below come from the checked-out Rust
repository and its tests. The repository contains no legacy Scala source and the
available Git history contains only the recent Rust rewrite. Comparisons with the
legacy Spark/Scala system are therefore architectural inferences, not a
source-level audit of the legacy implementation.

The current workspace baseline was also run:

```text
cargo test --workspace
```

All core and DataFusion tests passed. The `cubism-serve` API integration test
failed when the sandbox denied an operating-system networking operation
(`Operation not permitted`), not on an assertion about Cubism results.

## Current architecture and sparsity semantics

### XUnits are sparse over observed rows

The kernel does not enumerate the Cartesian product of distinct dimension
values. For each input row:

1. The spec evaluates a hierarchy of YPaths for each dimension.
2. [`generate_xunits`](../crates/cubism-core/src/lattice.rs) enumerates choices
   across the dimensions present in that row.
3. Within one dimension, hierarchy levels are alternatives. Across dimensions,
   subsets are combined.
4. Rules are applied while candidates are generated. `MaxDimensions` is used as
   an early bound; the remaining rules filter candidates.
5. The DataFusion build groups identical XUnits from different rows.

For a row with `h_i` available hierarchy levels in dimension `i`, the unpruned
upper bound is:

```text
product(h_i + 1) - 1
```

The extra choice per dimension is "omit this dimension"; the final subtraction
removes the all-omitted cell when the global cell is handled separately. This
formula is documented in
[`rules.rs`](../crates/cubism-core/src/rules.rs).

For `N` source rows, the pre-group work is bounded by `N` times that row-local
formula before rule pruning; durable cells are the distinct observed XUnits
remaining after grouping. Storage can therefore still grow rapidly with
dimension count and hierarchy depth, but it does not grow as the product of all
distinct values across the dataset.

This growth is exponential in the number of candidate dimensions even though it
is not a dimension-value Cartesian product. Existing controls—`MaxDimensions`,
`MinDimensions`, `NotTogether`, `NotAlone`, and attribute-level rules—are
therefore essential. The lattice tests include a legacy-compatible 17-cell
fixture in `unpruned_lattice_size_matches_legacy_spec`, the bounded 17/18-cell
case in `max_dimensions_three_matches_legacy_17_and_18_with_global`, and the
reference equivalence test `pruned_generation_equals_filtered_reference`.

Null or unavailable dimension levels contribute no YPath. Neither the lattice
nor the generated SQL creates cells for unobserved values. The current system
does not create empty aggregate states.

### Time currently has no first-class semantics

[`CubeSpec`](../crates/cubism-core/src/spec.rs) defines dimensions, rules,
measures, a global-cell flag, and dictionary cardinality limits. It has no event
time, bucket origin, timezone, resolution, allowed lateness, retention, or
rollup configuration.

Dimension levels are arbitrary SQL expressions. The DataFusion builder in
[`build.rs`](../crates/cubism-datafusion/src/build.rs) casts every dimension
level to `VARCHAR`, explodes the row-local lattice, and groups the result.
Therefore a date can currently be placed in a YPath, but it becomes an ordinary
dimension label.

That representation can answer equality-style cell lookups such as
"day=2026-07-28." It cannot supply robust temporal behavior:

- no typed half-open range semantics;
- no ordering guarantee;
- no distinction between event and ingestion time;
- no bucket origin, timezone, or daylight-saving policy;
- no resolution selection or aligned rollup;
- no explicit late-data or correction protocol;
- no efficient state merge across arbitrary intervals;
- no missing-versus-zero or coverage semantics.

Making time another dimension also multiplies the XUnit lattice and contaminates
the durable XUnit identity with a value that should instead bound a range scan.
The root `PLAN.md` explicitly chose this ordinary-YPath treatment and deferred
legacy dense-array time-series support. This proposal amends that decision
without reinstating dense multidimensional cube materialization.

### Current aggregate output is not uniformly mergeable

[`AggKind`](../crates/cubism-core/src/spec.rs) includes scalar aggregates and
non-scalar sketches. Some sketch implementations already serialize versioned
state:

- KMV has lawful merge behavior and property tests for commutativity and
  associativity (`kmv_merge_is_commutative_and_associative`) in
  [`tests/properties.rs`](../crates/cubism-core/tests/properties.rs).
- Reservoir sampling has a deterministic merge design.
- Semantic centroids retain count and coordinate sums, with normal
  floating-point order sensitivity.
- Top-K prunes candidates as it goes and explicitly documents order dependence;
  it cannot yet be treated as a universally associative temporal state.

The generated SQL currently persists presented scalar results for sum, count,
min, max, and average. An average value alone cannot be rolled up correctly; it
needs at least `(sum, count)`. Variance needs a mergeable state such as
`(count, mean, m2)`. The quantile build branch is currently unimplemented.

Time-series work therefore requires a common aggregate-state contract before it
requires Iceberg. That contract must describe:

- state schema and independent state-format version;
- `accumulate`, `merge`, and `present`;
- whether merge is associative, commutative, and idempotent;
- whether state is subtractable/invertible;
- exact versus approximate semantics and error metadata;
- deterministic serialization and cross-version upgrade behavior.

### Current persistence and serving are single-artifact

The CLI in
[`main.rs`](../crates/cubism-cli/src/main.rs) registers local CSV or Parquet
input and writes one Parquet cube. The
[`cubism-serve`](../crates/cubism-serve/src/lib.rs) crate loads one cube artifact
and exposes metadata, cell, and set-operation endpoints. There is no catalog,
window manifest, snapshot identity, temporal range endpoint, correction
protocol, or compaction loop.

Current binary XUnit encoding uses build-local dictionary IDs. It is compact but
not stable across independent builds. Canonical XUnit text is closer to a
durable identity, but its sentinel escaping also deserves a versioned,
round-trip-safe replacement before it becomes a storage key. Iceberg rows need a
deterministic encoding independent of build-local dictionaries.

## Requirements and non-goals

### Required invariants

1. **Sparse cube invariant:** a temporal build generates exactly the XUnits the
   non-temporal lattice would generate for an observed row, subject to the same
   rules. It adds only the row's bucket to the group key.
2. **No implicit gaps:** no bucket row exists unless at least one accepted event
   contributed to it. Query-time gap filling never mutates storage.
3. **Half-open time:** buckets and requests use `[start, end)`.
4. **Event-time authority:** event time assigns a bucket. Ingestion time records
   arrival and drives lateness policy; it does not silently replace event time.
5. **Immutable published revision:** corrections rebuild a complete affected
   window and publish a new revision atomically. They are never applied by
   blindly adding a delta to non-idempotent or non-subtractable state.
6. **Merge-state authority:** durable rows store sufficient state to merge; a
   displayed value is not the authoritative representation.
7. **Deterministic identity:** XUnit and spec identities are stable across
   processes, dictionary layouts, and retries.
8. **Explicit exactness:** a response states its coverage, source resolution,
   and whether partial boundaries are exact.

### Non-goals for the first release

- forecasting models or an ML anomaly-detection service;
- a fully dense multidimensional array;
- continuous streaming with exactly-once claims;
- row-level mutations of individual aggregate rows;
- an always-on Spark, HBase, ClickHouse, or TSDB deployment;
- automatic calendar/DST semantics before fixed-duration UTC buckets are sound;
- every possible rollup resolution or a segment-tree index on day one.

## Candidate temporal models

| Model | In-memory fit | Arrow/DataFusion fit | Iceberg fit | Assessment |
|---|---|---|---|---|
| Time as a YPath dimension | Already works for labels | Equality/grouping only | Simple strings, poor pruning semantics | Reject as the first-class temporal model; preserve for backward compatibility |
| `XUnit -> ordered buckets -> state` | Excellent conceptual API; natural sparse map | Must be flattened for vectorized scans | Nested maps are awkward to update/prune | Adopt as the logical model, not the durable layout |
| Long-form temporal rows | Good if batches are built directly | Excellent filter/group/sort representation | Natural partitions, evolution, snapshots | Recommended query and durable model |
| Chunked dense arrays per XUnit | Efficient for dense, regular ranges and SIMD | Requires custom list/array operators | Correction rewrites whole chunks; weak row/file statistics | Optional serving cache for proven-dense series |
| Sparse bucket maps | Convenient incremental hash state | Map columns inhibit standard pruning and UDAFs | Poor update granularity and interoperability | Useful only as a bounded build-side structure |
| Occupancy-selected hybrid | Can optimize dense and sparse workloads | Requires dual execution and conversion paths | More schemas, compaction, and correctness cases | Revisit only after occupancy benchmarks |

Dense arrays are not intrinsically wrong. A chunk of 1,440 minute buckets for a
nearly complete metric series can be more compact and faster to scan than 1,440
row keys. They are a poor universal representation for irregular events,
high-cardinality sparse XUnits, corrections to a few buckets, and generic
Iceberg predicate pushdown. They should be considered as a derived cache only
after observed occupancy and range-query profiles justify the complexity.

## Recommended temporal model

### Three deliberately different representations

**Aggregation memory**

A DataFusion group accumulator or bounded sparse map is keyed by:

```text
(bucket_start, xunit_key, measure_id)
```

XUnit generation remains row-local. The bucket is not fed into the lattice.
Spilling should use Arrow record batches rather than serializing a nested map as
the permanent format.

**Arrow/DataFusion**

Use sorted long-form record batches. Filter by cube/spec, resolution, and time;
resolve current revisions; group and merge aggregate states; then order results.
Ordinary DataFusion window functions can compute lag, rates, and rolling scalar
calculations after state presentation. Rolling distinct, quantile, and similar
measures need state-aware merge accumulators rather than windowing a displayed
number or opaque blob.

**Iceberg/Parquet**

Use a long-form aggregate table. A candidate logical schema is:

```text
cube_id                 string, required
spec_hash               fixed[32], required
xunit_version           int, required
xunit                   string, required
xunit_hash              fixed[16 or 32], required
resolution              string, required
bucket_start            timestamptz, required
bucket_end              timestamptz, required
measure_id              string, required
agg_kind                string, required
state_version           int, required
state_count             long
state_sum               double/decimal
state_min               typed value or typed struct
state_max               typed value or typed struct
state_mean              double
state_m2                double
state_blob               binary
window_start            timestamptz, required
window_end              timestamptz, required
revision                long, required
run_id                  uuid, required
source_snapshot         string
event_watermark         timestamptz
ingested_at             timestamptz, required
```

The exact choice between one row per measure, a wide row per XUnit/bucket, or a
struct-valued state column is a Phase 0B benchmark decision:

- per-measure rows evolve easily and avoid per-cube physical schemas but multiply
  row keys;
- wide rows reduce key repetition and can be faster when most measures are read
  together, but schema evolution is coupled to cube definitions;
- a tagged state struct is more typed than a blob but still needs careful
  cross-language evolution.

A fourth candidate replaces the repeated canonical XUnit with a stable
256-bit content identifier and stores canonical content once in an XUnit
registry table. This may reduce repeated key storage, but it adds a join,
registry lifecycle, and referential-integrity failure modes. Parquet dictionary
and run-length encoding may already compress sorted canonical XUnits well, so
the registry is a benchmark candidate rather than a presumed improvement.

The default hypothesis is normalized per-measure rows with typed columns for
common exact states and a versioned binary payload for complex sketches. Scalar
states should not be hidden in opaque bytes because Iceberg schema evolution,
statistics, debugging, and Spark interoperability are valuable. Sketch payload
versions are independent of Iceberg field IDs and table schema versions.

The `xunit_hash` is an index accelerator, not the identity by itself. Reads must
retain or verify canonical `xunit` to make collisions harmless. A new canonical
encoding should length-prefix or otherwise unambiguously encode sorted dimension,
attribute, type, null, and value fields before hashing.

### Time and bucket semantics

- MVP buckets are fixed durations in UTC with an explicit origin, not implicit
  locale boundaries.
- Event timestamps are normalized to UTC instants. The original source timezone
  may be retained as metadata.
- Calendar resolutions require an explicit IANA timezone and must store both UTC
  `bucket_start` and `bucket_end`, because DST makes local "days" nonuniform.
- A bucket contains events where `bucket_start <= event_time < bucket_end`.
- `resolution` is a typed identifier, not an arbitrary display label.
- An absent bucket is unknown/no observation. It is not zero.
- Zero filling is allowed only when the measure has a mathematical identity and
  the requested coverage is declared complete.
- Retention applies independently to raw events, base buckets, and coarser
  rollups. Expiration must not remove the only data capable of satisfying the
  advertised exactness contract.

Existing specs remain `v1` static-cube specs. A new, explicitly versioned
`v2alpha1` temporal section should add fields such as:

```yaml
apiVersion: cubism/v2alpha1
temporal:
  eventTime: event_timestamp
  ingestionTime: received_at
  baseResolution: 1m
  origin: 1970-01-01T00:00:00Z
  timezone: UTC
  allowedLateness: 48h
  rollups: [1h, 1d]
  retention:
    raw: 30d
    base: 180d
    rollups: 3y
```

No migration should silently reinterpret an existing dimension named `time`.

### Raw events and aggregates are separate tables

Temporal aggregate rows and replayable raw events have different schemas,
retention, update rates, and query patterns. Cubism should reference an
externally managed raw-event Iceberg table by default, recording its table and
source snapshot/checkpoint in each build. Owning a separate raw-event table is
an optional standalone deployment mode, not a product requirement. Raw access
itself is optional for deployments willing to give up exact partial-boundary
queries and correction replay after base retention expires.

### Window identity and publication

Use immutable build windows, for example one hour or one day depending on source
cadence:

```text
(cube_id, spec_hash, window_start, window_end, revision)
```

Do not expose newly appended aggregate files merely because they are present in
the aggregate table. A small manifest/control table records the current revision
and the aggregate snapshot that constitutes each published window. The commit
sequence is:

1. deterministically build the complete new window revision;
2. append its data files and commit an Iceberg snapshot;
3. validate row counts, bounds, state versions, and source coverage;
4. compare-and-swap the window manifest from the expected old revision to the
   new revision and snapshot;
5. make readers select only manifest-published revisions;
6. later expire losing/unpublished data through safe maintenance.

The reader invariant is:

```text
visible(aggregate_row) iff
  aggregate_row.window_id = manifest.window_id
  AND aggregate_row.revision = manifest.current_revision
```

Range queries read one sufficiently recent aggregate-table snapshot and
semijoin it against the current manifest rows. They do not open a different
Iceberg snapshot per window. The per-window aggregate snapshot ID is provenance
for validation, audit, rollback, and safe garbage collection; it is not the
primary row-selection mechanism.

This makes append-only Rust support usable for correction publication without
requiring readers to observe duplicate revisions. It also gives a rollback
pointer. The manifest update must itself provide transactional compare-and-swap
semantics; an Iceberg catalog or small control database must be selected and
tested for that contract.

Writers should claim disjoint windows where possible. `run_id`, deterministic
file names, and a source checkpoint make retries auditable. Because a current
Iceberg Rust issue reports duplicate file paths accepted within one fast-append
batch, Cubism must deduplicate paths before commit and test retry behavior rather
than assuming the library makes jobs idempotent.

## Arbitrary half-open interval queries

For a request `[start, end)`:

1. Validate that `start < end` and normalize both to UTC instants.
2. Select the coarsest available, non-overlapping buckets that exactly tile the
   aligned interior.
3. Filter Iceberg by cube/spec/resolution/time and current window revisions.
4. Merge the authoritative states by XUnit and measure.
5. Handle up to two partial boundary regions from raw events or a finer retained
   tier.
6. Present values only after all required states are merged.
7. Return exactness, coverage, and source-resolution metadata.

An aggregate request is exact when its source buckets exactly cover the request.
If the request is `10:03–11:47`, hourly buckets alone cannot answer it exactly.
The engine needs raw events for `10:03–11:00` and `11:00–11:47`, or a finer tier
that exactly tiles those boundaries. If neither exists, the API must reject
`exact=true` or explicitly return a rounded/approximate result. Daily buckets are
therefore inadequate for arbitrary ad-campaign boundaries.

Aggregate algebra matters:

| State | Mergeable | Subtractable | Temporal implication |
|---|---:|---:|---|
| Sum, count | Yes | Usually | Prefix differences are possible, subject to overflow/precision |
| Average `(sum,count)` | Yes | Via components | Never average displayed averages |
| Min/max | Yes | No | Merge covered buckets; cannot use prefix subtraction |
| Variance `(n,mean,m2)` | Yes | Possible but delicate | Prefer Chan/Welford merge; test numerical behavior |
| KMV distinct | Yes | No | Merge sketches; no exact prefix subtraction |
| Quantile sketch | Depends on chosen sketch | No | Require a proven mergeable format |
| Top-N | Intended, but current implementation is order-sensitive | No | Fix or qualify semantics before temporal use |
| Semantic centroid `(count,sums)` | Yes, within FP limits | Via components | Specify deterministic/numerical tolerance |

Prefix sums help only invertible measures. Naively recomputing a
non-subtractable rolling sketch over `n` points with a width of `w` costs
`O(nw)` state merges. Phase 6 must therefore use an amortized sliding-window
monoid algorithm where applicable, add segment/dyadic summaries, or enforce a
measured bounded-window limit. A universal persisted segment tree would
duplicate non-scalar state and complicate corrections, so it is not assumed to
be the first answer. Multiple materialized resolutions should be introduced
first and selected by a coverage planner; persisted dyadic summaries remain a
measured later option.

### Expected DataFusion plan

The range plan should:

1. read the publication manifest and resolve visible revisions/snapshots;
2. push cube, spec, resolution, and bucket predicates into the Iceberg
   `TableProvider`;
3. exploit hidden time partitioning plus Parquet row-group statistics;
4. project only identity and requested state columns;
5. repartition/group on XUnit and measure where a range spans files;
6. execute custom state-merge UDAFs;
7. sort by XUnit and bucket for series responses;
8. perform requested gap filling and scalar window calculations after coverage
   is known.

DataFusion provides `date_bin`, `date_trunc`, SQL window functions, memory
limits, and spill-capable operators. Those facilities are useful, but they do
not replace the custom state contract or an exactness-aware range planner.

## Iceberg integration feasibility

### Verified capabilities

The following statements are based on official Iceberg and Iceberg Rust
documentation/source checked on the evaluation date:

- Iceberg provides atomic snapshots, optimistic concurrency, schema and
  partition evolution, hidden partition transforms, and snapshot-based reads.
- Iceberg Rust `0.10.0` exposes REST, Glue, Hive Metastore, S3 Tables, SQL, and
  in-memory catalog integrations.
- Its DataFusion integration has catalog-backed current-table and static-snapshot
  providers, so current reads and time-travel-style pinned reads are feasible.
- Filter pushdown is reported as inexact. Phase 0A verified two day files were
  reduced to one planned file for a one-day predicate; Phase 0B/3 must measure
  manifest, data-file, row-group, and page pruning at representative scale.
- The released DataFusion write path supports Parquet and `INSERT` append. Other
  DataFusion `InsertOp` modes are explicitly unimplemented in the provider.
- The Rust transaction API exposes fast append and metadata updates, including
  schema, sort order, properties, location, and snapshot expiration.
- Local/memory object stores are native and S3, GCS, Azure, and other backends
  are available through the OpenDAL storage feature.

### Material limitations and workarounds

The released Rust public API does not expose the same mature overwrite,
`MERGE INTO`, row-level update/delete, or `rewriteDataFiles` maintenance surface
available through Spark. Consequently:

- Cubism should begin with append-only immutable revisions plus manifest
  publication, not in-place row updates.
- Correction publication must hide superseded revisions at query time.
- Spark or another mature Iceberg engine may initially perform large file
  rewrites, orphan cleanup, and some snapshot maintenance.
- Any native Rust compactor must be separately designed, proven, and guarded;
  it is not implied by merely adding `iceberg-datafusion`.

Two open upstream issues are especially relevant to the spike:

- a writer mismatch between Arrow UTC timestamp timezone strings `"UTC"` and
  `"+00:00"`;
- fast append accepting duplicate data-file paths within a batch.

Both must become regression tests or explicit mitigations before a production
go decision.

### Catalog and concurrency

A local SQL or in-memory catalog is useful for tests but insufficient evidence
for cloud multi-writer behavior. The production choice depends on deployment:

- REST catalog is the most portable service boundary.
- Glue is reasonable for AWS deployments.
- S3 Tables may reduce AWS table-maintenance work but increases provider
  coupling.
- Hive Metastore adds JVM-adjacent operational dependencies.
- A small SQL catalog may suit a controlled single-tenant deployment if its
  locking and compare-and-swap guarantees are verified.

The table commit and the publication pointer form a two-step protocol unless the
chosen catalog can atomically commit both. It must therefore be retryable and
recoverable. A crash after data append but before publication produces invisible,
collectable data—not a partially visible corrected window.

### Partition, sort, and file layout

Initial aggregate-table recommendation:

- hidden partition transform: `days(bucket_start)`;
- consider `hours(bucket_start)` only if measured daily partitions are too large
  and commit/file behavior remains healthy;
- optionally add a low-cardinality bucket transform on `cube_id` only after
  multi-cube measurements;
- never partition by high-cardinality XUnit.

Candidate sort order:

```text
(cube_id, spec_hash, resolution, bucket_start, xunit_hash, measure_id)
```

A series-heavy workload may prefer XUnit before time, while fleet-wide time
scans may prefer time first. Phase 0B must benchmark both representative
orderings. Target files should normally be in the 128–512 MiB range, with many
bucket results batched into each commit. A tiny file per bucket or per XUnit is
unacceptable.

Partition specs and sort orders can evolve in Iceberg. Cubism must still retain
the spec and state versions necessary to interpret old files.

## Rust/DataFusion versus Spark/Scala

| Concern | Rust/DataFusion expectation | Spark/Scala expectation | Current decision |
|---|---|---|---|
| Startup/scheduling | Low process and planning overhead | JVM and Spark scheduling overhead | Rust advantage for frequent small jobs |
| Single-node vectorized execution | Arrow-native, strong fit | Mature codegen/vectorization, heavier runtime | Benchmark; Rust likely competitive |
| Memory control | Explicit limits and spill, compact native runtime | Mature executor memory model and spill | Partially benchmarked 2026-08-09, single-node only — see below |
| Multicore scaling | Strong within one process | Strong within executor and cluster | Both viable |
| Very large distributed shuffle | Requires a chosen distributed layer and more engineering | Mature built-in distributed shuffle/retries | Spark advantage |
| Iceberg reads | Current/static providers, pruning to verify | Mature, broad integration | Spark advantage today |
| Iceberg writes/updates | Append works; overwrite/upsert/maintenance gaps | Append/overwrite/MERGE/DELETE/UPDATE and actions | Spark advantage |
| Streaming/watermarks | Must be built around batch windows/checkpoints | Structured Streaming is mature | Spark advantage for continuous streams |
| Custom merge-state UDAFs | Rust-native control, Arrow serialization | Mature UDAF APIs but JVM serialization concerns | Rust architectural fit; benchmark |
| Range-query serving | Embeddable, low-latency process | Usually batch/session-oriented | Rust advantage |
| Fault tolerance | Must be designed at window/artifact level | Mature job/task retry model | Spark advantage for distributed jobs |
| Deployment | Small native services/jobs | JVM/Spark control plane or managed service | Rust advantage at moderate scale |
| Maintenance risk | Young Iceberg Rust integration and version coupling | Mature ecosystem, larger operational surface | Hybrid until evidence improves |

This is not a blanket performance verdict. The likely boundary is:

- **Rust/DataFusion:** embedded queries, scheduled window jobs, a single large
  machine, modest scatter/gather, custom sketches, and latency-sensitive range
  serving.
- **Spark:** multi-terabyte shuffles, continuous stateful streaming,
  backfills requiring mature task recovery, and Iceberg copy-on-write or
  maintenance operations not yet exposed reliably in Rust.

The hybrid should share one table schema and aggregate-state specification.
Using Spark for maintenance must not require Spark to understand or recompute an
opaque sketch; file rewrites should preserve bytes, while any cross-engine state
merge needs a tested compatible implementation.

### Phase 0B single-node findings so far (2026-08-09, no Spark run yet)

`docs/TIMESERIES_PHASE_0B_HARNESS.md`'s "Known design chokepoints" section
has the full evidence; summary here since it bears directly on the row
above. This does **not** answer Rust-vs-Spark — no Spark comparison has run
on this host — but it does sharpen what "Benchmark high-cardinality cases"
found: (1) `EXPLAIN` confirms `AGGREGATE_SQL`'s `GROUP BY` is a full
two-phase hash aggregate over the whole input, not a bucket-streaming one —
the sort that makes the harness's own per-bucket write buffering safe
happens *after* aggregation, not during it; (2) only the sort operator's
disk-spill under the pinned memory pool has been empirically verified
(a 256MB/512MB probe), not the aggregate operator's, including for the
custom KMV/TopK/Centroid accumulators — whether the "Memory control" row's
Rust-side "explicit limits and spill" claim actually covers aggregation
memory, not just sorting, is still open; (3) measured real peak RSS
exceeds the pinned pool by an amount that scales consistently with
distinct cell count (~190-234 bytes/cell at 10M and 25M rows) rather than
staying flat, which is more consistent with per-group aggregation state
than with the previously-blamed non-pooled harness buffers (those are
capped near-constant, not scale-proportional). None of this touches the
"Very large distributed shuffle: Spark advantage" row's premise — every
Phase 0B run so far is single-process, single-machine; the
mergeable-aggregator design's own scatter/gather path (shard, build partial
cubes, merge via KMV/sum's documented mergeability) has never actually been
exercised in a benchmark, so "modest scatter/gather" above is still an
architectural claim, not a measured one.

## Benchmark and proof of concept

### Workloads

Generate reproducible inputs with fixed seeds:

1. dense regular metrics with near-complete minute coverage;
2. sparse irregular web/events with bursty traffic and missing periods;
3. high-cardinality XUnits;
4. 8–12 candidate dimensions constrained by `MaxDimensions`, `NotTogether`, and
   other materialization rules;
5. 1% and 5% late-event/correction batches;
6. scalar sum/count/min/max/average and variance;
7. KMV distinct, a selected mergeable quantile sketch, qualified Top-N, and
   semantic centroid states;
8. short and long aligned and non-aligned range queries.

Run at scales such as 10 million, 100 million, and 1 billion source rows, or
record the largest scale supported by the fixed hardware.

### Fair comparison protocol

- Pin Rust, DataFusion, Iceberg Rust, JVM, Spark, and Iceberg versions.
- Use identical source rows, bucket semantics, XUnit rules, aggregate state
  capacity/error parameters, object store, partition transforms, target file
  size, and sort-order alternatives.
- Compare local Rust and local Spark on the same host first.
- Compare distributed modes separately on the same instance pool; do not present
  local Rust versus a Spark cluster as one undifferentiated score.
- Run cold-cache and warm-cache trials, at least five measured repetitions after
  warm-up, and publish raw results.
- Validate exact aggregates against a reference implementation and sketches
  against error bounds.
- Verify that corrections publish one current revision and that retries do not
  duplicate input or files.

### Measurements

- source rows and generated XUnits per second;
- end-to-end wall time and startup latency;
- peak RSS, CPU utilization, and spill volume;
- repartition/shuffle bytes and time;
- files and bytes scanned;
- Iceberg data/delete/manifest file counts and commit latency;
- range-query p50/p95 latency by aligned/non-aligned and short/long class;
- aggregate data size, metadata size, and small-file amplification;
- compute and storage cost at representative deployment sizes;
- correctness, sketch error, coverage, and revision-publication results.

### Gates after the local Phase 0A result

Phase 0A passed items 1, 2, the local file-pruning portion of item 3, and a
control-store model of items 4–6. Proceed to a production storage design only
after the remaining environments and Phase 0B demonstrate all of the following:

1. one supportable released or exactly pinned dependency set compiles without
   maintaining a broad DataFusion fork;
2. UTC timestamps round-trip through Iceberg Rust writes and DataFusion reads;
3. time predicates materially prune partitions/files/row groups;
4. deterministic retries produce no duplicate contribution or duplicate file
   path;
5. concurrent, disjoint window appends converge; same-window publication has a
   clear winner and an auditable loser;
6. a replacement revision remains invisible until publication and can be rolled
   back;
7. target-size files can be produced without a tiny-file-per-bucket pattern;
8. state round trips and merges are correct across files and process restarts;
9. benchmark results show a credible Rust hot path, or identify the workload
   boundary at which Spark should take over.

## Risks, unknowns, and decision points

| Risk/unknown | Consequence | Required decision/evidence |
|---|---|---|
| Released DataFusion/Iceberg Rust version mismatch | Duplicate versions or dependency fork/downgrade | Phase 0A proved isolated coexistence; production must converge or preserve a boundary |
| Rust overwrite/compaction immaturity | Superseded files and operational debt | Append/revision protocol plus Spark fallback |
| Timestamp writer issue | Incorrect or failed temporal persistence | Exact UTC round-trip test |
| Duplicate fast-append paths | Double-counted aggregates | Precommit dedup and retry test |
| XUnit canonical string escaping | Non-stable or ambiguous durable identity | Introduce versioned canonical bytes/hash |
| Top-K order dependence | Different answers by merge tree | Replace/qualify algorithm before temporal release |
| Scalar outputs lack merge state | Incorrect rollups | State contract and schema before persistence |
| Exact arbitrary boundaries | False precision | Raw boundary scan or explicit approximation |
| Manifest/table two-step publication | Orphan data or split-brain readers | Recoverable CAS protocol and reconciliation |
| Small files from frequent buckets | Slow planning and high metadata cost | Buffered target-size writes and compaction SLO |
| High-cardinality group-by | Memory/shuffle pressure | Spill and scale benchmarks — **partially done 2026-08-09**: sort-spill verified, aggregate-spill and custom-accumulator spill NOT verified (see `TIMESERIES_PHASE_0B_HARNESS.md` "Known design chokepoints" #2) |
| Wide versus per-measure schema | Scan/row amplification tradeoff | Phase 0B schema benchmark |
| Calendar/DST buckets | Ambiguous boundaries | Defer until semantics and tests are specified |
| Legacy implementation unavailable | Comparison blind spots | Reproduce equivalent Spark reference workload |

## Conditional recommendation

**Conditional go:** accept the local Phase 0A result and proceed to Phase 0B
and/or Phase 1 semantics, not directly to production persistence.

The product semantics and Cubism's sparse kernel are compatible. The largest
uncertainty is not whether temporal XUnits can be computed; it is whether the
current Rust Iceberg stack can provide a supportable commit and maintenance path
without compromising correctness or creating an operational burden.

If Phase 0B and the later production-environment gates prove the remaining
items, proceed with Rust/DataFusion for the core and query paths and keep Spark
as an optional maintenance boundary. If catalog/object-store publication,
timestamp correctness outside the tested insert path, or pruning cannot be made
reliable without a significant fork, retain Spark for the Iceberg
write/maintenance path while continuing to use Rust for aggregation and
serving.

## Primary sources checked

- [Apache Iceberg introduction and guarantees](https://iceberg.apache.org/docs/latest/)
- [Iceberg schema, partition, and sort evolution](https://iceberg.apache.org/docs/latest/evolution/)
- [Iceberg hidden partitioning](https://iceberg.apache.org/docs/latest/partitioning/)
- [Iceberg table maintenance](https://iceberg.apache.org/docs/latest/maintenance/)
- [Iceberg Spark writes](https://iceberg.apache.org/docs/nightly/spark-writes/)
- [Iceberg Spark Structured Streaming](https://iceberg.apache.org/docs/latest/spark-structured-streaming/)
- [Iceberg Rust catalogs](https://rust.iceberg.apache.org/api.html)
- [Iceberg Rust DataFusion table provider](https://rust.iceberg.apache.org/api/iceberg_datafusion/table/index.html)
- [Iceberg Rust DataFusion provider source](https://rust.iceberg.apache.org/api/src/iceberg_datafusion/table/mod.rs.html)
- [Iceberg Rust writer source](https://rust.iceberg.apache.org/api/src/iceberg_datafusion/physical_plan/write.rs.html)
- [Iceberg Rust transaction API](https://rust.iceberg.apache.org/api/iceberg/transaction/struct.Transaction.html)
- [Iceberg Rust object-store `FileIO`](https://rust.iceberg.apache.org/api/iceberg/io/struct.FileIO.html)
- [Iceberg Rust 0.10 workspace dependencies](https://raw.githubusercontent.com/apache/iceberg-rust/v0.10.0/Cargo.toml)
- [Iceberg Rust current workspace dependencies](https://raw.githubusercontent.com/apache/iceberg-rust/main/Cargo.toml)
- [Iceberg Rust timestamp writer issue #2478](https://github.com/apache/iceberg-rust/issues/2478)
- [Iceberg Rust duplicate fast-append path issue #2507](https://github.com/apache/iceberg-rust/issues/2507)
- [DataFusion architecture](https://datafusion.apache.org/user-guide/introduction.html)
- [DataFusion date/time functions](https://datafusion.apache.org/user-guide/sql/scalar_functions.html)
- [DataFusion window functions](https://datafusion.apache.org/user-guide/sql/window_functions.html)
- [DataFusion runtime configuration](https://datafusion.apache.org/user-guide/configs.html)
- [Spark SQL performance and adaptive execution](https://spark.apache.org/docs/latest/sql-performance-tuning.html)
- [Spark Structured Streaming](https://spark.apache.org/docs/latest/streaming/index.html)
