# Time-Series Phase 0B Benchmark Harness

Date: 2026-08-08

Status: correctness harness slice complete; memory ceiling fixed and
measured at 1M/10M rows; 100M-row and Spark validation still pending;
benchmark decision pending

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
- `harness_schema_version` is `2`: `aggregate_ms` now spans the whole
  streaming aggregate-and-write loop (aggregation and the four layout writes
  are interleaved per bucket), not aggregation alone as under schema
  version 1. `run.json` files across schema versions are not comparable.

## Current environment gate

The harness was validated on a 10-core Apple M4 Mac mini with 16 GB memory,
macOS 15.7.7, Rust 1.96.1, and DataFusion 54.0.0.

`spark-submit` and the `pyspark` Python package were not present. Therefore:

- no Spark run has been performed;
- no Rust/Spark boundary has been selected;
- no layout has been selected;
- Phase 0B is not complete.

Spark must consume the same generated source files and emit states that
normalize to the same semantic digest. Spark's built-in approximate distinct
count is not an acceptable replacement for the Phase 1 KMV contract.

## Remaining Phase 0B work

1. Implement the local Spark adapter with byte-compatible KMV state.
2. Add aligned and non-aligned short/long range-read cases for every retained
   layout, including files and bytes scanned.
3. Add a process-level matrix runner that records warm-up plus at least five
   measured runs, release builds, cache state, concurrency, peak RSS, CPU,
   spill/shuffle, startup, and commit/write latency.
4. Measure both occupancy shapes at the actual 10-100 million row target on
   this 16 GB host (only 1M and 10M sparse are measured so far; see "Memory
   scaling" above), then retain only viable configurations.
5. Pin Parquet compression, row-group/file targets, and sort order in the
   results (the DataFusion engine memory limit is already pinned and
   recorded per run; see "Pinned semantics").
6. Write `TIMESERIES_PHASE_0B_RESULTS.md` with raw commands and artifacts.
7. Select the durable layout and local Rust/Spark boundary from that evidence.

Phase 2 temporal execution and Phase 3 production Iceberg persistence remain
blocked until those decisions are recorded.
