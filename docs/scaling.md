# Scaling Cubism beyond a single container

This doc spells out the scaling strategy that is implicit in the engine's
design: **why a single DataFusion process is the right starting point, what
its limits are, and exactly how the architecture fans out past it without a
distributed-systems rewrite.**

## The property everything rests on

Every measure in a Cubism cube — sums, counts, KMV distinct-count sketches,
top-k, exemplar samples, centroids — produces a buffer that is
**associative and commutative under merge**:

```text
merge(a, merge(b, c)) == merge(merge(a, b), c)      merge(a, b) == merge(b, a)
```

(For sketches this is enforced by property tests in
`cubism-core/tests/properties.rs`; for sums/counts it's arithmetic.)

The consequence: **a cube built from a dataset equals the merge of cubes
built from any partition of that dataset.**

```text
cube(A ∪ B) == merge_by_cell( cube(A), cube(B) )
```

That single identity is the scaling strategy. Everything below is just
choosing *where* the partitions and the merges run.

## Tier 0 — one process, many cores (where we are)

DataFusion parallelizes a single cube build across all cores of one machine:
the parquet scan splits into partitions, the explode UDF runs per-partition
(with a shared lock-free memo — see the "batch-snapshot" note in
`cubism-datafusion/src/udf.rs`), and the hash aggregate is partitioned by
key.

What bounds a single container:

| Resource | What consumes it | Relief valve |
|---|---|---|
| CPU | scan + explode (~17× row fan-out on a typical spec) + hash agg | more cores — near-linear until I/O bound |
| Memory | hash-agg state ≈ #cells × (measure buffers) — **not** raw data size | DataFusion's spilling aggregate; smaller sketch `k` |
| Wall clock | total bytes scanned | Tier 1 |

Reference point: 10M rows / 18-cell lattice / sum + count + KMV builds in
~6s on a laptop. Cube cells are small relative to input (10⁴–10⁷ cells with
kilobyte-scale sketch blobs), so **output is never the bottleneck; input scan
is.** A single beefy Cloud Run Job or VM handles low-billions of rows per
build before Tier 1 is worth the complexity.

## Tier 1 — scatter/gather: partial cubes + merge

When one machine's scan is too slow, split the *input*, not the engine:

```text
                    ┌── worker 1: cubism run spec --input part-000*.parquet → partial_1.parquet
input partitioned ──┼── worker 2: cubism run spec --input part-001*.parquet → partial_2.parquet
 (files / dates /   └── worker N: ...                                       → partial_N.parquet
  buckets)
                    merge job: GROUP BY xunit over partials,
                               SUM(sums), merge(sketch blobs)  →  final cube
```

- **Workers are the unmodified single-node engine.** No coordination, no
  shuffle, no cluster — each worker is a Cloud Run Job (the SaaS's execution
  unit) over its slice of files.
- **Partial cubes are tiny** compared to input (they're already aggregated),
  so the merge job is trivially cheap — it processes N × #cells rows, not
  raw data. The merge is itself a cubism build variant: group by cell key,
  `SUM` the sums, `cubism_*_merge` the blobs.
- **Counts/avgs**: partials must carry mergeable forms — `count` merges by
  sum; `avg` must be carried as (sum, count) pairs in partials and divided
  only at present time. (The single-node engine can use plain `AVG` because
  DataFusion handles its own partials internally; the cross-container
  variant presents at merge time instead. This is the one measure-level
  subtlety.)

**Cell-key caveat (important):** the binary XUnit keys are encoded against a
*per-build dictionary*, so keys from different workers are **not**
byte-comparable. Partial cubes therefore export the **canonical string form**
of the cell (`cubism_xunit_str`) as the join key — strings are globally
canonical by construction (normalized, sorted by dimension). Re-encoding to
binary happens, if needed, in the merge job against its own dictionary. A
shared/global dictionary is a possible later optimization, not a
requirement.

This tier needs no new engine code — only orchestration: a splitter (list
files, chunk them), N job launches, one merge job. That orchestration is
exactly what the SaaS control plane (Cloud Run Jobs + run manifests) already
plans to do for cadence scheduling; scatter/gather is the same machinery with
a fan-out count.

## Tier 2 — time: incremental and streaming builds

The same identity applied along the time axis:

- **Cadenced builds**: build one partial cube per time window (day/hour) as
  data arrives; a rolling "last 30 days" cube is the merge of 30 daily
  partials — recomputing nothing. Late data re-builds only its window.
  Idempotency: a window's output manifest is keyed by
  `(spec_version, window)`; re-runs are skipped or replace atomically.
- **Streaming (post-MVP)**: a micro-batch consumer holds per-`(cell, window)`
  sketch buffers in a state store and merges each increment in. This is a
  state-management problem, not an engine change — the buffers already merge.
  The batch path and the stream path produce interchangeable partials.

## Tier 3 — if it's ever needed

Options that stay compatible with the spec format, in likely order:

1. **Compile to warehouse `GROUPING SETS`** — push the whole build into the
   customer's Snowflake/BigQuery/DuckDB with native sketch UDFs; Cubism
   becomes planner + merge/serving layer. (Also the likely fix for the
   remaining single-node gap vs DuckDB's native grouping sets.)
2. **A distributed DataFusion runtime** (Ballista et al.) — drop-in at the
   SQL layer since the engine surface is one UDF + plain aggregation.

Neither is on the roadmap until a real workload outgrows Tiers 0–2; given
that Tier 1 handles "add workers linearly," that point is far away.

## Serving-side scaling (the other half)

The serving layer never touches raw data: cube cells live in Postgres
(`xunit key + measure values + sketch blobs`), and ad-hoc set operations
(`/setops`: overlap, Jaccard) are query-time `merge` calls on kilobyte blobs
— microseconds each, embarrassingly cacheable. Scaling levers, in order:
read replicas / API replicas (stateless), pre-merged rollups for selectors
that routinely span thousands of cells, and a DuckDB-over-parquet tier for
cubes that outgrow Postgres. Query-time work scales with *cells touched*,
never with raw data volume — that's the point of the architecture.
