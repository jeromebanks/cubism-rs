# Time-Series Implementation Plan

Status: proposed plan, no production implementation started

Depends on: [`TIMESERIES_FEASIBILITY.md`](TIMESERIES_FEASIBILITY.md)

Extends: the workspace-root [`PLAN.md`](../../PLAN.md); it does not overwrite or
invalidate the existing static-cube plan

## Outcome and delivery principles

This plan adds a temporal axis around Cubism's existing sparse XUnit lattice. It
does not make time another dimension and does not materialize empty buckets or a
dimension-value Cartesian product.

The target logical contract is:

```text
observed event
  -> allowed sparse XUnits
  -> event-time bucket
  -> versioned mergeable aggregate state
  -> immutable Iceberg window revision
  -> atomically published range-query visibility
```

The first production milestone should support:

- fixed-duration UTC event-time buckets;
- sparse base-resolution aggregation;
- immutable window revisions;
- append-only Iceberg writes plus current-revision publication;
- exact aligned range queries;
- exact partial boundaries when retained raw events are available;
- explicit non-exact responses otherwise;
- sum, count, min, max, average state, variance state, KMV, reservoir sample, and
  semantic centroid;
- Top-K only after its merge semantics are resolved;
- query-time gap filling with coverage metadata.

Every phase below has an independent rollback point. Phase 0A is the correctness
gate; Phase 0B is the representative local performance gate.

## Cross-phase architecture

### Proposed crate and module boundaries

Likely additions, subject to Phase 0A dependency findings:

```text
crates/cubism-core/src/temporal.rs
crates/cubism-core/src/aggregate_state.rs
crates/cubism-datafusion/src/temporal_build.rs
crates/cubism-datafusion/src/state_udaf.rs
crates/cubism-datafusion/src/range_query.rs
crates/cubism-iceberg/                 # isolate catalog/storage version churn
crates/cubism-serve/src/series.rs
```

`cubism-core` owns semantics and pure state algebra. `cubism-datafusion` owns
Arrow conversion, physical/logical expressions, accumulators, and plans.
`cubism-iceberg` should own table schemas, catalog configuration, snapshot
commits, manifest publication, and object-store concerns. This keeps Iceberg
dependency churn out of the core crate.

### Compatibility policy

- Existing specs and static Parquet builds remain supported as `v1`.
- Temporal specs start as `cubism/v2alpha1`.
- An existing `time` dimension is never reinterpreted automatically.
- Existing static cube artifacts are not migrated in place.
- State payload format versions are independent of crate, spec, Arrow, Parquet,
  and Iceberg schema versions.
- Readers either upgrade a known old state or return a typed unsupported-version
  error; they do not guess.

### Build and publication identity

All artifacts use explicit identities:

```text
cube_id
spec_hash
window_start
window_end
revision
run_id
source_checkpoint or source_snapshot
aggregate_snapshot_id
state_format_versions
```

A revision is a complete replacement for its window. A manifest pointer, updated
with compare-and-swap, determines which revision readers see. Data files that
were appended but never published are invisible and eligible for later cleanup.

## Phase 0A — Iceberg compatibility and correctness spike

Status: local-filesystem gate passed; see
[`PHASE_0A_RESULTS.md`](PHASE_0A_RESULTS.md).

Depends on: plan approval only.

### Purpose

Resolve the smallest correctness risks before designing production temporal
APIs or running a broad performance comparison.

### Dependencies and code areas

- Keep the spike isolated in `crates/cubism-iceberg-spike`.
- Pin released Iceberg Rust and its coherent DataFusion version.
- Do not change the DataFusion version used by existing Cubism crates.
- Use an in-memory catalog and local filesystem first; production catalog and
  object-store behavior remain later integration gates.

The accepted spike combination is Iceberg Rust `0.10.0`,
`iceberg-datafusion` `0.10.0`, DataFusion `53.1.0`, Arrow/Parquet `58`, and
Rust `1.96.1`. Cubism's existing DataFusion `54.0.0` remains installed in
parallel. This proves workspace compatibility, not shared execution-plan type
compatibility across the two DataFusion major-minor versions.

### Experiments

1. Create a day-partitioned aggregate table.
2. Append long-form count state plus a binary state payload through DataFusion.
3. Read current aggregate rows back through DataFusion.
4. Round-trip UTC timestamps, including Arrow's named `"UTC"` representation.
5. Prove a one-day predicate prunes an unrelated day data file.
6. Simulate a crash after append and recover by deterministic `run_id` and
   expected row count without appending twice.
7. Prove that a concurrent deterministic-run claim has one append owner.
8. Append an unpublished replacement revision and prove the manifest semijoin
   continues to expose only the previous revision.
9. Publish the replacement with compare-and-swap and prove range reads can span
   windows whose provenance names different aggregate snapshots.
10. Reject a stale same-window publication.

The reader invariant is:

```text
visible(aggregate_row) iff
  aggregate_row.window_id = manifest.window_id
  AND aggregate_row.revision = manifest.current_revision
```

Readers use a current/sufficiently recent aggregate snapshot and one manifest
semijoin. Per-window Iceberg snapshot IDs are provenance, not separate snapshots
opened by the range query.

### Public API and persistence impact

None. The crate, table, control store, and normalization helper are feasibility
artifacts, not production APIs.

### Migration and compatibility

No production artifact or spec migration occurs. Existing Cubism crates remain
on DataFusion `54.0.0`. The spike must be removable as one workspace member.

### Tests

- deterministic and concurrent run-claim tests;
- stale publication compare-and-swap test;
- real append/read integration test;
- UTC timestamp normalization and round-trip test;
- file-plan pruning assertion;
- crash/retry recovery;
- unpublished/published manifest visibility semijoin.

### Completion criteria

- The pinned released stack compiles beside current Cubism.
- Append/read and UTC timestamp round trips pass.
- Time filtering prunes at least one unrelated partition file.
- A retry protocol prevents duplicate append ownership and recovers a committed
  run by deterministic identity.
- Latest aggregate data plus the current manifest exposes exactly one revision
  per published window.
- Results and remaining non-local risks are documented.

### Unresolved decisions

- production REST/Glue/S3 Tables/SQL catalog;
- production CAS/lease backend and lease expiry;
- object-store consistency and credential behavior;
- direct low-level writer exposure to upstream timestamp issue #2478;
- DataFusion `INSERT` returns row count rather than the exact committed snapshot
  ID, so concurrent writers need a lower-level commit result or containment
  validation;
- duplicate data-file-path defense below the DataFusion insert layer;
- production orphan reconciliation.

### Rollback point

Remove `crates/cubism-iceberg-spike` from the workspace. Existing static cube
code and its DataFusion version are unaffected.

## Phase 0B — Representative local Rust-versus-Spark benchmark

Status: harness slice 1 complete; measured Rust/Spark decision pending.

The initial reproducible harness lives in `crates/cubism-timeseries-bench`.
It generates deterministic sparse/dense Parquet sources, runs bucketed
Rust/DataFusion sum/count/KMV aggregation, writes the four candidate layouts,
and rejects a run unless every layout reads back to the same semantic digest.
See `TIMESERIES_PHASE_0B_HARNESS.md`. This is a correctness foundation, not a
layout or execution-boundary decision.

Depends on: Phase 0A local go decision and Phase 1's frozen minimal temporal and
aggregate-state reference for sum/count and KMV.

### Purpose

Measure the likely local execution boundary without pulling billion-row,
distributed, every-aggregate, and production-maintenance work into the initial
correctness spike.

### Scope

- 10–100 million rows before considering larger scale;
- sparse irregular and dense regular inputs;
- high-cardinality XUnits with representative lattice restrictions;
- sum/count plus KMV as the non-scalar state;
- aligned and non-aligned short/long range reads;
- Rust/DataFusion and local Spark on the same host and source files;
- wide state, per-measure state, state struct, and stable `xunit_id` plus
  registry layouts.

Pin partitioning, sort order, file targets, hardware, concurrency, cache state,
and correctness semantics. Measure rows/s, wall time, startup, peak RSS, CPU,
spill/shuffle, files/bytes scanned, commit latency, query p50/p95, and physical
storage/metadata size.

### Completion criteria

- At least five recorded runs after warm-up for each retained configuration.
- Exact states match a common reference and KMV results stay within one shared
  error contract.
- A schema/layout and local Rust/Spark boundary are selected from evidence.
- Large distributed and billion-row comparisons are explicitly deferred to
  Phase 7.

### Rollback point

Discard the benchmark harness and keep the Phase 0A correctness result. Do not
infer performance from correctness alone.

## Phase 1 — Temporal types, state algebra, and specification semantics

Depends on: Phase 0A dependency and correctness go decision.

### Types and modules

Add pure types in `cubism-core`:

```text
EventTime
IngestionTime
TimeRange              # validated [start, end)
BucketStart/BucketEnd
FixedResolution
BucketOrigin
CalendarResolution     # parsed but gated if deferred
TemporalSpec
AllowedLateness
Coverage
Exactness
WindowId
WindowRevision
StateVersion
```

Add an aggregate-state capability contract resembling:

```text
AggregateState {
  accumulate(row)
  merge(other)
  present()
  encode/decode(version)
  capabilities()
}

AggregateCapabilities {
  associative
  commutative
  idempotent
  subtractable
  exactness
}
```

This is a semantic sketch, not a commitment to trait-object dispatch. An enum,
generics, or DataFusion accumulator factory may be more efficient.

Change:

- `crates/cubism-core/src/spec.rs`
- `crates/cubism-core/src/ypath.rs`
- `crates/cubism-core/src/encoding.rs`
- scalar and sketch modules under `crates/cubism-core/src/sketch/`

Add a versioned canonical XUnit encoding that is independent of build-local
dictionary IDs. Preserve the existing encoding for static artifacts.

### Public API changes

- Add explicitly versioned `v2alpha1` spec parsing.
- Add `temporal` configuration with event/ingestion expressions, base
  resolution, origin, timezone, lateness, optional rollups, and retention.
- Add bucket-assignment and exactness/coverage APIs.
- Reject unsupported calendar semantics rather than silently approximating them.

### Persistence changes

None beyond golden representations for spec hashes, canonical XUnit bytes, and
aggregate state payloads. No Iceberg tables are written in this phase.

### Migration and compatibility

- `v1` parse and behavior remain unchanged.
- A migration helper may emit a draft `v2alpha1` spec but must require the user
  to identify event time.
- Do not infer time from a YPath name.
- The spec hash includes all temporal semantics and state parameters.

### Tests

Unit:

- half-open interval validation;
- UTC fixed-bucket assignment with negative/pre-origin times;
- exact boundary inclusion/exclusion;
- duration parsing and overflow;
- missing-versus-zero representation;
- spec validation and hash stability;
- canonical XUnit round trips including sentinel-like values, Unicode, null, and
  typed values.

Property:

- every timestamp belongs to exactly one fixed bucket;
- adjacent buckets neither overlap nor leave a gap;
- canonical XUnit order does not change identity;
- merge laws for every advertised aggregate;
- encode/decode and old-to-new upgrade laws;
- partitioned accumulation equals single-pass accumulation within declared
  numerical/sketch tolerances.

Targeted aggregate work:

- store average as `(sum,count)`;
- introduce `(count,mean,m2)` variance;
- decide and implement a mergeable quantile format;
- prove, replace, or remove the temporal merge claim for current Top-K;
- state centroid floating-point tolerance explicitly.

### Completion criteria

- Temporal semantics are unambiguous without DataFusion or Iceberg.
- Every MVP measure has a truthful capability declaration and authoritative
  merge state.
- XUnit identity is stable across independent processes/build dictionaries.
- Existing tests and static behavior remain unchanged.

### Unresolved decisions

- decimal versus floating scalar state;
- canonical encoding format and hash width;
- calendar buckets in the first release;
- quantile sketch selection;
- Top-K exact/approximate contract;
- raw-event retention required for an `exact=true` service tier.

### Rollback point

Keep the new types experimental and unused. Revert the `v2alpha1` parser without
affecting `v1` builds.

## Phase 2 — Sparse bucketed incremental aggregation

Depends on: Phase 1 temporal/aggregate-state contracts and the Phase 0B local
architecture decision.

### Types and modules

Add `cubism-datafusion/src/temporal_build.rs` and
`cubism-datafusion/src/state_udaf.rs`.

Refactor the current generated SQL/build path so temporal aggregation:

1. evaluates event time and assigns one bucket;
2. generates the same row-local XUnits as the static path;
3. appends bucket identity to the group key without placing it in the lattice;
4. accumulates authoritative state;
5. emits sorted Arrow record batches.

Use DataFusion accumulators/UDAFs where they preserve spill and vectorization.
Avoid an unbounded process-global Rust hash map.

### Public API changes

- Add an experimental temporal build entry point taking a `TimeRange`/`WindowId`.
- Return aggregate-state batches plus build metadata and source coverage.
- Add CLI dry-run/explain support that reports estimated and actual source rows,
  generated XUnits, observed buckets, and output state rows.

### Persistence/schema changes

- Define an Arrow logical schema matching the Phase 0B-selected Iceberg layout.
- Still write local Parquet fixtures first; do not publish production Iceberg
  windows in this phase.

### Migration and compatibility

- The static build path remains independent.
- Reuse lattice/rules code directly to prevent semantic drift.
- Ensure a temporal spec cannot accidentally invoke static time-as-YPath
  behavior unless explicitly requested as another normal dimension.

### Tests

Unit/integration:

- one event produces no XUnits beyond the static lattice result;
- unobserved buckets and value combinations are absent;
- rules prune identically in static and temporal builds;
- events at bucket boundaries go to the correct half-open bucket;
- invalid/null event times follow configured reject/quarantine policy;
- duplicate source batch/run handling is explicit;
- each aggregate state presents the expected values.

Property:

- temporal output projected without bucket identity equals a lawful merge of
  static outputs;
- arbitrary input partition/order yields equivalent states within tolerances;
- sparse row count is bounded by observed row-local XUnits, not distinct-value
  Cartesian products.

Performance:

- generated XUnits/s and peak memory at increasing dimension/cardinality levels;
- spill behavior under a configured memory limit;
- state-size amplification by measure.

### Completion criteria

- A bounded window builds deterministically from source events.
- Retrying the pure build creates byte-equivalent or semantically identical
  state rows and metadata.
- No gap or dense Cartesian materialization occurs.
- MVP aggregate states merge correctly across independently built partitions.

### Unresolved decisions

- one multi-measure accumulator versus measure-specific plans;
- direct physical extension versus generated SQL;
- Arrow dictionary use inside one build;
- quarantine behavior for malformed timestamps;
- deterministic floating-point reduction order versus tolerance.

### Rollback point

Leave temporal build behind an experimental feature flag. Static build and
serve artifacts remain the supported path.

## Phase 3 — Iceberg schema, reads, append-only writes, and publication

Depends on: Phase 0A Iceberg proof, Phase 0B layout evidence, and Phase 2 state
batches.

### Types and modules

Create `cubism-iceberg` with:

```text
CatalogConfig
TemporalTableSchema
TemporalTable
AggregateWriter
AggregateReader
SnapshotRef
WindowManifest
PublicationStore
CommitResult
```

Expose DataFusion `TableProvider`s through this boundary without leaking catalog
implementation types into `cubism-core`.

### Public API changes

- CLI commands to initialize/inspect experimental tables, append one window, and
  verify a snapshot.
- Configuration for catalog, warehouse/object store, credentials by reference,
  table names, target file size, partition spec, and sort order.
- Read APIs require a snapshot or publication view; no accidental "all
  revisions" default in user-facing range queries.

### Persistence/schema changes

Create:

1. a reference to an externally managed raw-event Iceberg table, including the
   source snapshot/checkpoint used by each build; optionally create a Cubism-owned
   raw table only in standalone mode;
2. a temporal aggregate-state table;
3. a window publication manifest, or a transactional control-store equivalent.

Aggregate table starts with hidden `days(bucket_start)` partitioning and the
Phase 0B-selected sort order. Store field IDs and canonical schema fixtures.
State blobs carry their own magic/version/checksum.

Writer protocol:

- produce deterministic file inventory;
- deduplicate data-file paths before commit;
- fast-append immutable revision files;
- validate committed snapshot;
- publish revision using expected-current compare-and-swap;
- record a reconciliation item if publication fails.

### Migration and compatibility

- No conversion of existing static cube Parquet is required.
- Table properties record supported Cubism spec/state versions.
- Readers reject an aggregate snapshot whose required state version is unknown.
- Partition and sort evolution is allowed without changing logical bucket
  semantics.

### Tests

Unit/golden:

- Iceberg/Arrow schema and field IDs;
- partition transform and sort order;
- state nullability by aggregate kind;
- deterministic file/run metadata.

Integration:

- create/append/read current snapshot;
- static snapshot reads;
- UTC timestamp regressions;
- partition/file/row-group pruning assertions;
- same logical retry;
- duplicate-path rejection;
- crash after file write, after snapshot commit, and before/after publication;
- reader sees only current manifest revision;
- local catalog plus selected production-like catalog/object store.

Performance:

- commit latency versus buffered window count;
- file-size distribution and metadata amplification;
- range scan bytes versus total table bytes.

### Completion criteria

- A complete window can be written, validated, published, read, and rolled back.
- Retries do not double-count.
- Readers never expose an uncommitted or unpublished revision.
- Pruning and file sizes meet Phase 0B thresholds.
- Operations have structured logs/metrics for run, snapshot, and revision IDs.

### Unresolved decisions

- manifest in Iceberg versus a strongly consistent SQL/control store;
- catalog selection and credential model;
- data encryption/key management;
- exact orphan reconciliation period;
- whether raw event ingestion is inside Cubism or supplied by an external table.

### Rollback point

Move the publication pointer to the previous revision. Disable Iceberg temporal
commands without affecting static Parquet serving. Orphan new files remain
recoverable until retention cleanup.

## Phase 4 — Late data, corrections, concurrency, and compaction

Depends on: Phase 3 append and publication protocol.

### Types and modules

Extend `cubism-iceberg` and core temporal policy with:

```text
LatenessPolicy
WindowLease or ExpectedRevision
CorrectionPlan
ReconciliationRecord
CompactionPlan
RetentionPlan
```

Add a coordinator/job API that identifies affected windows from event time,
rebuilds them completely, and publishes a new revision.

### Public API changes

- Submit or schedule a correction by source checkpoint/time range.
- Inspect current/superseded revisions and reconciliation state.
- Explicit maintenance commands with dry-run plans.
- Metrics: lateness distribution, correction count/age, publication conflicts,
  orphan bytes, file-size distribution, snapshot count, and compaction lag.

### Persistence/schema changes

- Append full immutable correction revisions.
- Compare-and-swap the manifest against the revision observed at planning time.
- Losing writers do not republish automatically without rereading source and
  current state.
- Add retention for superseded revisions, snapshots, raw events, and orphan
  files.

Initial compaction may be a Spark job that rewrites files without changing
logical rows or state payloads. A Rust compactor is admitted only after it proves
equivalent snapshot safety and file/statistics behavior.

### Migration and compatibility

- Existing published revisions remain readable.
- Changing allowed lateness affects scheduling, not historical bucket identity.
- Changing base resolution or state algorithm creates a new spec hash/table
  generation; it does not rewrite states in place.

### Tests

Property/integration:

- a late-event rebuild equals a clean rebuild from the corrected source;
- no additive correction is used for non-idempotent/non-subtractable states;
- two same-window writers produce one published winner;
- disjoint windows can commit concurrently;
- failure injection at every commit/publication stage is recoverable;
- stale leases/expected revisions cannot overwrite newer work;
- compaction preserves visible rows, state bytes, and snapshot rollback;
- retention never deletes a still-published or rollback-required artifact.

Performance:

- 1% and 5% late-data repair cost;
- commit conflicts under realistic concurrency;
- small-file and metadata growth before/after compaction;
- Spark maintenance startup/cost versus candidate Rust maintenance.

### Completion criteria

- Late data produces a full, atomically published replacement.
- A deterministic recovery run classifies and reconciles every interrupted job.
- Compaction and retention SLOs are documented and observable.
- No maintenance path changes aggregate answers.

### Unresolved decisions

- window duration versus correction blast radius;
- lease service versus optimistic expected-revision only;
- compaction trigger by bytes/files/latency;
- when a correction is too old and must be rejected or handled as a backfill;
- native Rust maintenance graduation criteria.

### Rollback point

Repoint a window to the prior published revision and stop correction scheduling.
Do not immediately expire superseded snapshots; retain a configured recovery
window.

## Phase 5 — Exactness-aware DataFusion range queries and serving API

Depends on: published temporal tables from Phases 3–4 and Phase 2 merge states.

### Types and modules

Add `cubism-datafusion/src/range_query.rs` and
`cubism-serve/src/series.rs` with:

```text
TemporalQuery
ResolutionPlan
BoundaryPlan
CoveragePlan
StateMergeExec/UDAF
SeriesResponse
```

The planner resolves current revisions, chooses non-overlapping resolution
segments, identifies partial boundaries, and decides whether raw-event scans are
required.

### Public API changes

Add a service method and `/api/series` endpoint accepting:

```text
cube/spec
XUnit selector(s)
measure(s)
[start, end)
requested/auto resolution
exact=true|false
gap policy
timezone/display options
```

Return ordered points/intervals with:

```text
value/presentation
bucket_start/bucket_end
is_exact
coverage
source_resolution
state/error metadata where appropriate
missing marker
snapshot/revision provenance
```

`exact=true` fails clearly if retained buckets/raw data cannot exactly cover a
partial boundary. It never rounds silently.

### Persistence/schema changes

No new authoritative table. Optional query/result caches are disposable and
must key on aggregate snapshot/publication identity.

### Migration and compatibility

- Existing `/api/meta`, `/api/cells`, `/api/cell`, and `/api/setops` remain.
- Static artifacts do not expose `/api/series` unless explicitly adapted with
  limited semantics.
- API versioning protects later calendar/multi-resolution changes.

### Tests

Unit:

- range decomposition for aligned and unaligned boundaries;
- resolution choice never overlaps or double-counts;
- missing/zero/gap-fill rules;
- exactness failures and metadata.

Integration:

- filters reach Iceberg scans and prune expected files;
- merge scalar and sketch states across windows/files;
- partial boundaries scan raw events or reject when unavailable;
- superseded revisions never appear;
- short/long and empty intervals;
- timezone display does not change UTC membership;
- pagination/limits cannot reorder or omit series points.

Property:

- query result equals direct raw-event reference when exact;
- different valid merge trees give equivalent lawful states;
- composing adjacent exact ranges equals the whole range for compatible
  aggregates.

Performance:

- p50/p95 short and long ranges;
- aligned versus non-aligned;
- XUnit selectivity and high-cardinality fanout;
- cold/warm cache and concurrent request behavior;
- files/bytes scanned assertions.

### Completion criteria

- The service answers exact aligned ranges from aggregate state.
- Partial-boundary behavior is truthful and tested.
- Range plans prune storage and stay within latency/memory budgets.
- Every result reports sufficient coverage and provenance for downstream use.

### Unresolved decisions

- SQL table-function interface in addition to HTTP;
- maximum raw boundary scan;
- response shape for multi-XUnit/multi-measure queries;
- server-side caching;
- authorization boundary by cube/tenant.

### Rollback point

Disable the temporal endpoint and keep temporal tables intact. Existing
non-temporal serving remains available.

## Phase 6 — Rolling comparisons and trend/anomaly inputs

Depends on: Phase 5 exactness-aware ordered series.

### Types and modules

Add query-layer operators/plans for:

- lag and rate of change;
- period-over-period comparisons;
- rolling scalar windows;
- state-aware rolling merge for distinct/quantile/etc.;
- baseline extraction with coverage/exactness propagation.

Built-in DataFusion windows can operate on presented scalar columns where the
math is valid. Serialized states require a custom state-aware accumulator or a
planned merge over each frame. Never compute rolling distinct by summing
displayed distinct counts. Naively merging all `w` buckets at each of `n`
positions is `O(nw)` for non-subtractable sketches. Before exposing an
unbounded rolling-sketch API, this phase must either implement an amortized
sliding-window monoid algorithm, use segment/dyadic summaries, or enforce a
measured maximum window width with explicit cost behavior.

### Public API changes

Extend `/api/series` or add a versioned analysis request:

```text
compare_to
lag
rolling_window
rate_unit
baseline_periods
minimum_coverage
```

Responses include null/missing behavior and denominator validity. Emit ordered
feature batches suitable for downstream anomaly/change-point/forecasting tools;
do not embed model choice into the storage layer.

### Persistence/schema changes

None initially. Frequently used baselines may later be derived temporal cubes
with their own specs and publication identities, never hidden mutations of base
state.

### Migration and compatibility

Additive API only. Exactness and coverage metadata are propagated through every
derived value.

### Tests

- lag and period alignment across missing buckets;
- zero denominators and rate units;
- rolling boundaries and minimum coverage;
- scalar rolling values against a reference;
- state-aware rolling distinct/quantile against raw-event/reference merges;
- scaling tests that distinguish amortized/summary execution from `O(nw)`
  recomputation;
- DST display cases if calendar support has graduated;
- no leakage from future buckets into baselines.

Performance:

- long rolling frames;
- many simultaneous XUnits;
- repeated period comparisons;
- state-merge cost by sketch size.

### Completion criteria

- Trends and comparisons are mathematically correct for declared state
  capabilities.
- Rolling non-subtractable states have a documented algorithmic complexity and
  enforced resource bound.
- Missing and incomplete coverage cannot masquerade as zero or an anomaly.
- Downstream systems can request stable, ordered, provenance-bearing features.

### Unresolved decisions

- custom DataFusion window UDAF versus explicit frame plans;
- caching/materializing popular baselines;
- supported change-point/anomaly interchange schema;
- approximate rolling-window error reporting.

### Rollback point

Remove/disable derived analysis operators; base range queries and stored state
are unchanged.

## Phase 7 — Performance hardening and optional rollups

Depends on: measured production-like workloads from Phases 3–6.

### Types and modules

Add only mechanisms justified by production profiles:

- rollup planner and scheduler;
- multiple-resolution coverage index;
- occupancy statistics;
- optional dense chunk cache for high-occupancy series;
- optional dyadic/segment summaries for merge-heavy long ranges;
- native compactor if it has met the Phase 4 graduation bar;
- distributed scatter/gather window execution when one node is insufficient.

### Public API changes

- operational policies for rollup/retention/compaction;
- explain output showing chosen resolution, files, bytes, and expected exactness;
- admin visibility into occupancy and optimization recommendations.

### Persistence/schema changes

- Coarser rollups use the same state contract and distinct resolution identity.
- A rollup is built only from complete published child coverage.
- Corrections invalidate/rebuild affected ancestors; they are never patched with
  unsafe subtraction.
- Dense chunks, if adopted, are derived and disposable unless given a separate
  rigorously versioned table contract.
- Iceberg partition and sort evolution follows measured query distribution.

### Migration and compatibility

- Base-resolution data remains the correctness fallback while retained.
- New resolutions are additive and do not change existing bucket identity.
- Optimizer decisions can be disabled per cube.
- Derived caches can be dropped without data loss.

### Tests

Property/integration:

- rollup merge equals child-state merge;
- no overlap/double count in mixed-resolution plans;
- correction rebuild propagates to all affected rollups;
- dense and sparse representations return equivalent results;
- compaction and partition evolution preserve answers;
- distributed retry/idempotency tests.

Performance:

- occupancy break-even for rows versus dense chunks;
- file/sort-order A/B tests;
- long-range multi-resolution latency;
- dyadic index storage/update cost versus merge savings;
- distributed scale-up and scale-out curves;
- recurring cost and maintenance SLOs.

### Completion criteria

- Each optimization has a measured trigger and rollback.
- Query SLOs hold at target data/cardinality/concurrency.
- File, manifest, snapshot, orphan, and rollup lag stay inside operational SLOs.
- Distributed operation is introduced only where single-node or simple
  scatter/gather evidence is insufficient.

### Unresolved decisions

- occupancy threshold for dense chunks;
- fixed hierarchy versus workload-driven rollups;
- dyadic/segment index value;
- native distributed scheduler and shuffle layer;
- retirement point for the Spark maintenance fallback.

### Rollback point

Disable the optimizer or individual derived resolutions/caches and serve from
published base state. Revert partition/sort preference for future writes; Iceberg
continues reading older specs.

## End-to-end test and release strategy

### Test layers

1. **Unit:** temporal types, bucket math, state encoding, planners.
2. **Property:** merge laws, partition invariance, range composition, canonical
   identity, sparse bounds.
3. **Golden:** specs, XUnit bytes/hash, state bytes, Arrow/Iceberg schemas.
4. **Integration:** DataFusion plans, local Iceberg, selected catalog/object
   store, Rust/Spark interoperability.
5. **Failure injection:** retries, crashes, duplicate files, stale publication,
   concurrent writers, maintenance interruption.
6. **Correctness:** compare exact answers with raw events and sketch answers with
   declared error bounds.
7. **Performance:** fixed datasets/hardware with stored artifacts and regression
   thresholds.

### Release progression

```text
spike
  -> experimental local tables
  -> one internal cube, append-only
  -> late-data shadow builds
  -> read-only range API shadow comparison
  -> limited production tenant/cube
  -> general availability after maintenance SLO evidence
```

For shadow operation, build temporal output alongside the current static cube and
compare overlapping scalar/sketch results. The publication pointer supplies a
fast rollback without rewriting files.

## Next action after Phase 0A review

Phase 0A's local correctness gate has passed. Land Phase 1's pure temporal and
aggregate-state semantics next, then run Phase 0B against that frozen minimal
contract so the benchmark does not measure a throwaway schema. No production
Iceberg storage crate should be created until the Phase 0A evidence and
remaining catalog/object-store limitations are accepted.
