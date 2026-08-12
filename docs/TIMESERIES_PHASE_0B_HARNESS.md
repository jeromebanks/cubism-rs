# Time-Series Phase 0B Benchmark Harness

Date: 2026-08-08

Status: correctness harness slice complete; memory ceiling fixed and
re-baselined at 1M/10M/25M rows (2026-08-09/11, see
`docs/TIMESERIES_PHASE_0B_NOTEBOOK.md`); write-path settings
(compression/row-group/sort) pinned and recorded (2026-08-09); Spark
adapter implemented and correctness-verified at 25k rows (2026-08-11,
commit `87380bf`); process-level matrix runner done for both engines,
including a cross-engine digest check (2026-08-11, this session); range-read
cases, the actual 10-100M-row measurement for both engines, and the
resulting layout/boundary decision still pending -- see
`docs/TIMESERIES_PHASE_0B_HANDOFF.md` for the authoritative current-state
summary, this file's own "Status" line lags it.

Implementation:
[`crates/cubism-timeseries-bench`](../crates/cubism-timeseries-bench)

## What this slice establishes

The harness provides the common local input and semantic normalization needed
before running the 10–100 million-row Rust/Spark matrix:

- deterministic sparse-irregular and dense-regular Parquet source generation;
- UTC hourly `bucket_start` values;
- a sparse XUnit lattice containing global, device, region, and device-region
  cells, with no empty-bucket or Cartesian materialization;
- Rust/DataFusion aggregation of exact integer sum/count and Phase 1 KMV v1
  blobs;
- Phase 1 canonical XUnit bytes and BLAKE3-256 XUnit content IDs;
- four physical layouts written as Parquet:
  1. per-measure rows with repeated canonical XUnit bytes;
  2. wide per-bucket/XUnit rows;
  3. tagged state structs;
  4. wide state rows keyed by a 256-bit `xunit_id` plus an XUnit registry;
- streaming read-back verification of every layout against a deterministic
  semantic BLAKE3 digest over bucket, XUnit identity, canonical bytes, sum,
  count, and KMV state, computed once during aggregation/write and compared
  against an independently recomputed digest from each layout's own file
  (see "Memory scaling" below — no layout, and no full run, ever
  materializes more than one bucket's cells in Rust memory);
- a bounded DataFusion execution memory pool (`--memory-limit-mb`, default
  2048) so the engine's own GROUP BY/ORDER BY operators spill to disk
  instead of growing with row count;
- JSON output containing aggregation+write time, layout write time, row
  count, file size, verification time, the pinned memory limit, and the
  semantic digest.

A run fails if a layout drops, duplicates, or changes a state, or if rows
within a layout are not strictly ordered by `(bucket, content ID)`. KMV blobs
are decoded during collection and verification; presented approximate counts
are not used as authoritative state.

## Commands

Use an isolated target directory so the benchmark build does not contend with
the repository target symlink.

```bash
rtk proxy env CARGO_TARGET_DIR=/tmp/cubism-target-phase0b \
  rtk cargo run --release -p cubism-timeseries-bench -- \
  generate \
  --output /tmp/phase0b/sparse-source.parquet \
  --rows 10000000 \
  --occupancy sparse \
  --seed 1 \
  --batch-rows 65536

rtk proxy env CARGO_TARGET_DIR=/tmp/cubism-target-phase0b \
  rtk cargo run --release -p cubism-timeseries-bench -- \
  rust \
  --input /tmp/phase0b/sparse-source.parquet \
  --output-dir /tmp/phase0b/sparse-rust-run-01 \
  --partitions 4
```

The harness refuses to overwrite source files or run directories. Give every
measured process a fresh output directory.

For a bounded correctness check over both occupancy shapes:

```bash
rtk proxy env CARGO_TARGET_DIR=/tmp/cubism-target-phase0b \
  rtk cargo run -p cubism-timeseries-bench -- \
  smoke \
  --work-dir /tmp/phase0b-smoke \
  --rows 25000 \
  --partitions 4
```

Check the required comparison engine explicitly:

```bash
rtk proxy env CARGO_TARGET_DIR=/tmp/cubism-target-phase0b \
  rtk cargo run -p cubism-timeseries-bench -- preflight
```

## Correctness validation performed

The crate test suite passed:

```text
cargo test -p cubism-timeseries-bench
5 tests passed, 1 ignored (release-only, run explicitly)
```

The tests lock:

1. identical generation inputs create byte-identical source Parquet;
2. `canonical_xunit_content_id()` (write path) and raw `blake3::hash()` of
   the same canonical bytes (every read/verify path) agree, so the two
   never silently diverge;
3. two pinned golden semantic digests (25,000-row sparse and dense, seed 1,
   `--partitions 4`), captured from the pre-streaming-rewrite implementation
   and reproduced byte-for-byte by the current implementation;
4. a `#[ignore]`d 1,000,000-row sparse golden digest, run explicitly via
   `cargo test --release -- --ignored`, likewise reproduced byte-for-byte;
5. a DataFusion fixture writes and reads every layout with one semantic
   result.

Golden digests are the only independent correctness oracle this harness has:
a rewrite that reproduces pinned pre-rewrite bytes has not silently changed
semantics, whereas comparing a rewrite's output only to itself (self-consistency)
is not evidence of correctness. See issue #1, finding 2.

An additional 25,000-row smoke run passed for both sparse and dense sources.
It produced 51,473 sparse cells and 51,512 dense cells. All candidate layouts
matched the reference digest in each run.

These timings and sizes are intentionally not recorded as benchmark findings:
the smoke command used a debug build, only one process per shape, a small input,
and no controlled cold/warm-cache protocol.

## Memory scaling

The original (pre-2026-08-08) implementation collected the full DataFusion
aggregate result into one `BTreeMap<(bucket, xunit_id), CellState>`, and
`verify_layouts` held up to five such maps (the reference plus one per
layout) simultaneously. Measured on the 10-core Apple M4 Mac mini described
below, 1,000,000 sparse rows peaked at 2.45 GB RSS / 3.6 GB memory footprint,
extrapolating to roughly 22-33 GB at the harness's own 10-100M row target —
more than the 16 GB host has, and not evidence-backed at that scale.

Two changes replaced the full in-memory map:

1. **Bounded DataFusion engine memory.** `run_rust` now runs against a
   `GreedyMemoryPool` (`--memory-limit-mb`, default 2048 MiB) instead of an
   unbounded pool, so the engine's GROUP BY hash table and ORDER BY sort
   spill to disk under memory pressure rather than growing with cell count.
   Verified directly: re-running the 1M-row sparse fixture with
   `--memory-limit-mb 256` fails with `Resources exhausted` from a spilling
   `ExternalSorterMerge` operator (the pool is real and enforced, not
   decorative; spilling is genuinely occurring, the merge phase just needs
   more headroom than 256 MiB at this cell count); `--memory-limit-mb 512`
   succeeds and reproduces the exact golden digest, confirming spill-and-merge
   preserves correctness.
2. **Per-bucket streaming aggregation and verification.** `AGGREGATE_SQL`
   sorts only by `bucket_start` (not also by `xunit`); a single-key sort
   guarantees every bucket's rows are contiguous in the output stream.
   `run_rust` buffers one bucket's cells at a time (`BucketCells`, a
   `BTreeMap<[u8; 32], CellState>` keyed by content ID), which is bounded by
   `BENCH_SPEC`'s fixed dictionary size regardless of input row count, not
   by total cell count. Each bucket is digested and written to all four
   layouts, then dropped, before the next bucket starts. Verification is
   symmetric: each layout is re-read in file order and folded into a digest
   with only a small in-progress-cell buffer, never a full layout's cells.
   The one deliberately-retained global structure is the `xunit_registry`
   layout's id-to-canonical-XUnit map, which is O(distinct XUnits) by
   construction (bounded ~450,009 by `BENCH_SPEC`), not O(cell count).

Measured results (release build, same host, `cargo test --release -p
cubism-timeseries-bench -- --ignored` for the 1M case; ad hoc `rust` runs for
10M):

| Rows (sparse) | Cells      | Avg cells/bucket | Peak RSS | Peak footprint |
| -------------- | ---------- | ----------------- | -------- | -------------- |
| 1,000,000      | 1,936,575  | 11,527 / 450,009  | 1.09 GB  | 771 MB         |
| 10,000,000     | 15,137,276 | 90,102 / 450,009  | 3.42 GB  | 5.10 GB        |

Both runs reproduce the pinned golden digest / `cell_count` and pass the
harness's own internal layout verification. Peak RSS did **not** stay flat
from 1M to 10M: per-bucket cell density climbed from 2.6% to 20% of the
450,009-cell ceiling as the sparse generator's fixed {device, region} lattice
filled in, and the harness's own Rust-side buffers (per-bucket cells, batch
builders, the registry map) are not tracked by the DataFusion memory pool
above, so they add to peak RSS independent of the `--memory-limit-mb` bound.

**The 100M-row figure has not been measured.** A naive linear projection
from the 1M→10M growth is unsound (per-bucket density growth is convex, not
linear, as it approaches the dictionary's ceiling), so no extrapolated
number is recorded here. Running the actual 10-100M configuration is
remaining Phase 0B work (below) before a layout/boundary decision can be
evidence-backed at the required scale.

## Pinned semantics

- Buckets are UTC, one hour wide, and represented by their start instant.
- The query groups only cells reached by events; it does not manufacture gaps.
- XUnit content IDs are the BLAKE3-256 digest of Phase 1 `CXU` v1 bytes.
- Sum uses signed 64-bit integer state in this harness so cross-engine
  correctness is exact.
- Count uses unsigned 64-bit state after validating the engine result is
  non-negative.
- KMV uses the Phase 1 `KMV` v1 blob. Spark must produce byte-equivalent state
  or merge-equivalent state with the same XXH3-64 hashing and `k=1024`.
- Layout field names containing `_v1` and every explicit `state_version=1`
  belong to the harness schema; changing them requires a harness schema bump.
- The source file is generated before engine timing and is shared unchanged
  between engines.
- The DataFusion execution memory pool (`--memory-limit-mb`, default 2048)
  is pinned benchmark configuration, recorded as `memory_limit_bytes` in
  every `run.json`, not an incidental implementation detail.
- **Parquet compression is pinned to `UNCOMPRESSED`** (`PINNED_COMPRESSION`
  in `lib.rs`), applied identically to the source file and all four
  layouts, recorded as `compression`/`layout_compression` in `run.json`.
  This matches the crate's prior behavior (parquet-rs's own default), now
  made explicit rather than implicit. It has **not** been evaluated for its
  effect on disk footprint at the 10-100M row target — four uncompressed
  layouts at that scale may use substantially more disk than a compressed
  codec would; check available space on the output volume before scaling
  up, and treat this as an open question if disk becomes a constraint.
- **Layout row-group size is pinned to 65,536 rows** (`LAYOUT_ROW_GROUP_ROWS`),
  recorded as `layout_row_group_rows` in `run.json`. The source file's row
  group size remains controlled separately by `--batch-rows`
  (`GenerationMetrics.batch_rows`).
- **Two different sorts are both pinned and must not be conflated.** The
  DataFusion aggregate stream (`AGGREGATE_SQL`) sorts only by
  `bucket_start` — this single-key sort is load-bearing for the bounded-
  memory fix (it's what guarantees one bucket's rows are contiguous so only
  one bucket's cells are ever buffered; see "Memory scaling" above).
  Within a bucket, `run_rust`'s `BucketCells` (keyed by XUnit content ID)
  re-sorts to the canonical `(bucket, content ID)` order that the semantic
  digest and every layout's on-disk row order use. Adding `xunit` to
  `AGGREGATE_SQL`'s `ORDER BY` would either reintroduce the pre-fix
  unbounded-memory bug or desync from the written/verified order — it is
  not a pinning knob to touch.
- `harness_schema_version` is `3`: `run.json` gained the `compression` /
  `layout_compression` / `layout_row_group_rows` fields above (no run
  behavior changed by the bump itself). Schema `2` added `aggregate_ms`
  spanning the whole streaming aggregate-and-write loop, not aggregation
  alone as under schema version 1. `run.json` files across schema versions
  are not comparable.

## Known design chokepoints (found via EXPLAIN + empirical measurement, 2026-08-09)

Filed as [github.com/jeromebanks/cubism-rs#2](https://github.com/jeromebanks/cubism-rs/issues/2).

The user asked directly whether the Rust/DataFusion design is less scalable
than the original Spark implementation. This benchmark hasn't run Spark yet
(see "Current environment gate" below), so that comparison itself is still
unanswered — but investigating the memory-growth pattern surfaced four
concrete, evidence-backed findings about *this* design that bear on it
directly, independent of any Spark comparison.

### 1. `AGGREGATE_SQL`'s GROUP BY is a full two-phase hash aggregate, not a bucket-streaming one

Checked with `EXPLAIN VERBOSE` against a real registered source (see
`crates/cubism-timeseries-bench/examples/explain_aggregate.rs`, a throwaway
diagnostic — SQL/spec text duplicated from `lib.rs` rather than than
changing that file's visibility). The physical plan is:

```
SortExec: expr=[bucket_start ASC]
  AggregateExec: mode=FinalPartitioned, gby=[bucket_start, xunit_key]
    AggregateExec: mode=Partial, gby=[bucket_start, xunit_key]
```

This is DataFusion's standard unordered hash aggregate (Partial → repartition
by group key → FinalPartitioned), and **the sort happens after aggregation
completes**, not before. The code comment in `lib.rs` above `AGGREGATE_SQL`
("a single-key sort guarantees all rows for one bucket are contiguous in
the output stream") is correct about the *output stream* the Rust code
consumes — and that's what makes `BucketCells`' one-bucket-at-a-time
buffering valid — but it does **not** mean DataFusion's own aggregation
is bucket-scoped internally. The `AggregateExec` operators must hold (or
spill) state for every distinct `(bucket, xunit_key)` group across the
**entire input**, not one bucket at a time, before the `SortExec` can even
start. For this benchmark's fixed 168-bucket, one-week window that's a
bounded (if large) number; for a real deployment with continuous ingestion
over months or years, a from-scratch full-reprocessing aggregation pass
would face a *growing* global distinct-cell state, not a per-bucket-bounded
one. (`ts_eval.md`'s own requirements list "Incremental aggregation" as a
goal — this benchmark, by construction, only exercises full reprocessing,
so it says nothing about how an incremental/appendable pipeline would
behave, which is a separate design that hasn't been built or tested yet.)

### 2. Only the SORT operator's spill has been verified — the GROUP BY's has not

The existing "Memory scaling" section above cites the 256MB-vs-512MB
experiment as proof the pinned memory pool causes real spilling — but that
experiment's own error message names the operator: `Resources exhausted`
from a spilling `ExternalSorterMerge`. That's the **sort** operator. Whether
`AggregateExec` (Partial or FinalPartitioned) spills under pressure in this
DataFusion version, and specifically whether it does so correctly for the
*custom* KMV/TopK/Centroid `GroupsAccumulator`s (which do implement
`size()`, checked in `crates/cubism-datafusion/src/udaf.rs`, so per-group
memory reporting exists), has not been isolated or tested the same way. If
it doesn't spill — or spills but the custom accumulators' reported `size()`
undercounts their true heap cost — the pinned `--memory-limit-mb` would only
really be bounding the sort phase while aggregation memory grows
unconstrained by it. This is the single highest-value follow-up experiment:
repeat the low-memory-limit probe but construct it (e.g. via a query that
forces many buckets' worth of distinct groups to coexist, or by watching
DataFusion's own metrics/`EXPLAIN ANALYZE` spill counters) to isolate
`AggregateExec` specifically, not `SortExec`.

### 3. Real peak RSS scales with distinct cell count, well beyond the nominal pool bound — and the harness's own non-pooled buffers are too small to be the main cause

Measured, same host, same day:

| Rows | Cell count | Peak RSS (median) | Excess over 2048MB pool | Excess per cell |
|---|---|---|---|---|
| 1M | 1,936,575 | 1690 MB (p4) / 1501 MB (p1) | *under* budget | n/a |
| 10M | 15,137,276 | 5420 MB | 3372 MB | ~234 bytes/cell |
| 25M | 28,846,459 | 7286 MB | 5238 MB | ~190 bytes/cell |

The excess-per-cell figure is roughly consistent (190-234 bytes) across a
2.5x change in scale — that consistency is the useful signal. It rules out
the two non-pooled harness structures previously blamed for "buffers the
pool doesn't see" (`docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entry 3/4): the
`xunit_registry` is capped near the fixed ~450,009-XUnit dictionary
regardless of row count, and `BucketCells` is capped per-bucket at the same
ceiling — neither grows enough between 10M and 25M rows to explain a
*proportionally growing* multi-GB gap. A roughly-constant per-cell excess
instead points at the aggregation's own per-group state (the two
`AggregateExec` stages' hash-table entries plus each group's in-flight
`GroupsAccumulator` state, most plausibly the KMV sketch, before it's
finalized to a compact blob) as the leading candidate — consistent with
finding 1/2 above, but **not confirmed by heap profiling** (no
`heaptrack`/`massif`-equivalent run has been done); flagged as inference
from a consistent ratio, not a proven mechanism.

### 4. `--partitions` is not the multiplier it might look like (checked, and ruled out)

Worth recording as a negative result so it isn't re-investigated later:
compared `--partitions 1` vs `--partitions 4` at 1M rows (1501 MB vs 1690
MB median peak RSS) — only a ~13% difference, not the ~4x a naive
"N partitions → N concurrent full-size hash tables" model would predict.
DataFusion's partitioning here doesn't multiply the group-cardinality
footprint the way it might for a less key-balanced workload. Parallelism
is not the primary scaling risk in this design; findings 1-3 are.

### What this does and doesn't say about Spark

None of this says Rust/DataFusion is *less* scalable than the legacy Spark
implementation — no Spark run has happened yet on this host (see below),
so there is still no head-to-head evidence either way, and `ts_eval.md`'s
own instructions require an apples-to-apples comparison (matched hardware,
semantics, and critically **single-node Spark**, since Spark's real
scaling story is horizontal/cluster-distributed — comparing single-node
DataFusion against multi-node Spark would be a fundamentally unfair
comparison in Spark's favor, not a wash). What findings 1-3 *do* establish:
this benchmark, and the current `cube_udfs`/`AGGREGATE_SQL` design, only
ever exercises **monolithic single-process aggregation** over the whole
input in one `SessionContext`. The mergeable-aggregator design (KMV, sum,
count are explicitly associative/commutative/mergeable per
`docs/scaling.md`) was built specifically so that a scatter/gather
pattern — shard the input, build partial cubes independently (possibly in
parallel processes or on separate machines), merge the mergeable states —
is *possible*. But Phase 0B as currently scoped has never exercised that
path even once; every run so far is a single `SessionContext` processing
one file end to end. If horizontal scale-out is the real answer to "how
does this compete with a Spark cluster," it needs its own benchmark
alongside this one, not an inference from single-node numbers.

## Current environment gate

The harness was validated on a 10-core Apple M4 Mac mini with 16 GB memory,
macOS 15.7.7, Rust 1.96.1, and DataFusion 54.0.0.

**Superseded 2026-08-11**: `openjdk@21`, `apache-spark` 4.2.0, and `sbt`
are now installed on this host (see
`docs/TIMESERIES_PHASE_0B_HANDOFF.md` "Environment now in place") and a
Scala Spark adapter has run successfully, correctness-verified against the
pinned golden digest at 25,000 rows. What remains true:

- Spark has **not** been run at benchmark scale (10-100M rows) — only the
  25k correctness check and the matrix-runner validation at the same size
  (`TIMESERIES_PHASE_0B_NOTEBOOK.md` Entry 7);
- no Rust/Spark boundary has been selected;
- no layout has been selected;
- Phase 0B is not complete.

Spark must consume the same generated source files and emit states that
normalize to the same semantic digest. The matrix runner can check this
automatically now (item 3, below) — but only when a single invocation
actually runs both engines at the same rows/occupancy; a Spark-only
invocation still prints "(no rows/occupancy pair ran both engines)" and
exits 0, so this is a check available on request, not a standing
enforcement that guards every Spark run by construction. Spark's built-in
approximate distinct count is not an acceptable replacement for the Phase 1
KMV contract, and the adapter does not use it (see
`spark-adapter/src/main/scala/cubism/bench/KmvAggregator.scala`).

## Remaining Phase 0B work

1. ~~Implement the local Spark adapter with byte-compatible KMV state~~
   **Done** (2026-08-11, commit `87380bf`) — see
   `docs/TIMESERIES_PHASE_0B_HANDOFF.md` for the full writeup and caveats
   (correctness-verified at 25k rows only; performance at scale still
   unmeasured).
2. Add aligned and non-aligned short/long range-read cases for every retained
   layout, including files and bytes scanned.
3. ~~Add a process-level matrix runner that records warm-up plus at least
   five measured runs~~ **Done for Rust** (notebook session, 2026-08-09/11)
   **and for Spark** (2026-08-11, this session) --
   `crates/cubism-timeseries-bench/scripts/matrix_runner.py` now takes an
   `engine=rust|spark` config key and drives `spark-submit` with the same
   RSS-watchdog/retry/mount-check rigor as the Rust `rust` subcommand, plus
   a cross-engine `semantic_digest_blake3` check when both engines run at
   the same row count/occupancy in one invocation. See
   `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entry 7 for validation (a real
   25k-row Spark run through the tool reproduced the pinned golden digest;
   the watchdog's RSS reading was independently cross-checked against
   `/usr/bin/time -l`). Spark's own memory bound
   (`spark-driver-memory-mb`) is a required, separate config key, not
   derived from Rust's `--memory-limit-mb` -- the two knobs bound different
   things (DataFusion's execution-memory pool vs. a whole JVM heap) and
   picking the actual value per row count is deliberately left open for
   item 4, not defaulted here.
4. Measure both occupancy shapes at the actual 10-100 million row target on
   this 16 GB host (only 1M and 10M sparse are measured so far; see "Memory
   scaling" above), then retain only viable configurations. **Gated on user
   go-ahead past 50M rows** — step up gradually under the matrix runner's
   RSS watchdog rather than jumping to 100M.
5. ~~Pin Parquet compression, row-group/file targets, and sort order in the
   results~~ **Done** (2026-08-09): see "Pinned semantics" above —
   compression pinned to `UNCOMPRESSED` (unevaluated for disk footprint at
   scale), layout row-group pinned to 65,536 rows, the two distinct sorts
   (aggregate-stream vs. written/verified order) documented explicitly so
   they aren't conflated. `harness_schema_version` bumped 2→3. Re-verified
   against the 1M-row golden digest (`cargo test --release -p
   cubism-timeseries-bench -- --ignored`) and the full non-ignored suite —
   both still pass byte-for-byte / value-for-value.
6. Write `TIMESERIES_PHASE_0B_RESULTS.md` with raw commands and artifacts.
7. Select the durable layout and local Rust/Spark boundary from that evidence.

Phase 2 temporal execution and Phase 3 production Iceberg persistence remain
blocked until those decisions are recorded.
