# Time-Series Phase 0B Handoff

Date: 2026-08-08

Branch: `feature/timeseries-phase-0a`

Status: harness correctness slice complete; memory ceiling fixed and
measured at 1M/10M rows; Spark adapter and the 10-100M row measurement are
not started; Phase 0B is not complete

## Executive handoff

Phase 0B's bounded local Rust-versus-Spark and layout benchmark was started
in an earlier session as `crates/cubism-timeseries-bench` but had a
correctness/memory defect serious enough to block the phase: the harness
collected its entire aggregate result into one in-memory map, measured at
2.45 GB peak RSS for 1,000,000 sparse rows and extrapolating to roughly
22-33 GB at the phase's own stated 10-100M row target — more than the 16 GB
development host has. Its correctness tests also only checked that the
harness's own output was self-consistent (every layout decodes back to the
same digest as every other layout from the same run), not that any of it
matched an independent oracle. Both were filed as
[issue #1](https://github.com/jeromebanks/cubism-rs/issues/1) and fixed this
session; see that issue and its progress comment for the full finding-by-
finding detail. This handoff covers what changed and what a next session
needs to pick up.

This did **not** change the Phase 0B plan's shape: still four candidate
layouts, still sum/count/KMV against the Phase 1 frozen state formats, still
a required local Rust-versus-Spark comparison before a layout/execution
boundary can be selected. It changed how much of that plan is evidence-
backed today: the harness itself is now memory-safe up to the row counts
actually measured (1M, 10M), but nothing at 100M, and no Spark run, exists
yet.

## What was implemented

All in `crates/cubism-timeseries-bench/src/lib.rs` (rewritten) and
`src/main.rs` (extended), on top of the harness slice from the prior
session:

- **`verify_layouts` scoping fix.** Each layout's read-back now drops
  before the next is built, instead of all four read-backs plus the
  reference staying alive simultaneously.
- **Streaming aggregation and write path.** `AGGREGATE_SQL` sorts only by
  `bucket_start`; `run_rust` consumes the DataFusion result as a stream
  (`execute_stream`, not `.collect()`) and buffers one bucket's cells at a
  time in `BucketCells` (bounded by `BENCH_SPEC`'s fixed {device, region}
  dictionary, not by row/cell count). Each bucket is digested and written
  to all four layouts through persistently-open `ArrowWriter`s
  (`LayoutWriters`/`OpenLayout`), then dropped.
- **Streaming verification.** Each layout is re-read in file order and
  folded into a digest with only a small in-progress-cell buffer (measure
  triples in `per_measure`/`tagged_struct` are contiguous by construction),
  compared against the digest computed during the write pass. A
  `check_monotonic` guard fails the run if any layout's rows are not
  strictly ordered by `(bucket, content ID)` — this is what would catch a
  future change that breaks the sortedness this design depends on.
- **Bounded DataFusion execution memory.** A `GreedyMemoryPool` via
  `RuntimeEnvBuilder`/`SessionContext::new_with_config_rt`, size controlled
  by `RustRunConfig::memory_limit_bytes` / CLI `--memory-limit-mb` (default
  2048 MiB), so the engine's own GROUP BY/ORDER BY spill to disk instead of
  growing with cell count. Recorded per run as `memory_limit_bytes` in
  `run.json` since it is pinned benchmark config, not an implementation
  detail.
- **Golden digest fixtures.** Three semantic digests captured from the
  pre-rewrite implementation (25k sparse, 25k dense, 1M sparse; seed 1,
  `--partitions 4`) pinned as regression tests. This is the harness's only
  independent correctness oracle — see issue #1 finding 2 for why
  self-consistency alone wasn't sufficient.
- **Content-ID equivalence test.** Pins that `canonical_xunit_content_id()`
  (write path) and raw `blake3::hash()` (every read/verify path) are the
  same computation over the same bytes.
- `harness_schema_version` bumped 1 → 2: `aggregate_ms` now spans the whole
  streaming aggregate-and-write loop, not aggregation alone. `run.json`
  files across schema versions are not comparable.

Full detail, including the exact SQL/ordering argument for why per-bucket
buffering reproduces the pre-rewrite digest byte-for-byte, is in
[`TIMESERIES_PHASE_0B_HARNESS.md`](TIMESERIES_PHASE_0B_HARNESS.md)'s
"Memory scaling" section.

## What was measured

Release build, 10-core Apple M4 Mac mini, 16 GB, macOS 15.7.7, Rust 1.96.1,
DataFusion 54.0.0:

| Rows (sparse) | Cells      | Avg cells/bucket (of 450,009 cap) | Peak RSS | Peak footprint |
| -------------- | ---------- | ----------------------------------- | -------- | -------------- |
| 1,000,000      | 1,936,575  | 11,527 (2.6%)                       | 1.09 GB  | 771 MB         |
| 10,000,000     | 15,137,276 | 90,102 (20.0%)                      | 3.42 GB  | 5.10 GB        |

Both reproduce the pinned golden digest and pass internal verification.

**Peak RSS is not flat with row count** — it was expected to be
(bucket-buffer size is bounded by the fixed dictionary, independent of row
count), but per-bucket cell *density* climbs toward that dictionary's
450,009-cell ceiling as row count grows (2.6% full at 1M, 20% at 10M), and
the harness's own Rust-side buffers (bucket cells, batch builders, the
`xunit_registry` layout's id-to-canonical map) aren't tracked by the
DataFusion memory pool, so they add to peak RSS independent of
`--memory-limit-mb`. Do not extrapolate the 100M figure linearly from these
two points — the growth is convex (approaching a ceiling), so a linear
projection is wrong in an unhelpful direction. **No 100M number exists.**

Also verified directly that the memory pool bound is real rather than
decorative: re-running the 1M fixture with `--memory-limit-mb 256` fails
cleanly with `Resources exhausted` from a spilling `ExternalSorterMerge`
(spill genuinely occurs; 256 MiB is too tight for the merge phase at this
cell count); `--memory-limit-mb 512` succeeds and reproduces the exact
golden digest.

## What was deliberately left open

- **Layout verification still can't catch a shared aggregation bug**
  (issue #1 finding 3). All four layouts are still derived from, and
  checked against, the same DataFusion aggregate result — a bug that
  produces the same wrong sum/count/KMV in every layout would pass. Closing
  this needs an independent brute-force Rust aggregation over raw source
  rows as a second oracle, not touched this session.
- **`write_ms`/`aggregate_ms` remain a shared-process measurement**
  (issue #1 finding 4). `write_ms` is now the honest sum of each layout's
  own `ArrowWriter::write`/`close` wall time (better than before), but all
  four layouts' writes and the aggregation stream still share one process
  and one thread pool, so none of these numbers isolate one layout's true
  cost the way separate process runs would.

## Verification completed

```text
CARGO_TARGET_DIR=/tmp/cubism-target-verify cargo test -p cubism-timeseries-bench
  5 passed, 1 ignored (3 suites)

CARGO_TARGET_DIR=/tmp/cubism-target-verify cargo test --release -p cubism-timeseries-bench -- --ignored
  1 passed (the 1M golden digest fixture)

CARGO_TARGET_DIR=/tmp/cubism-target-verify cargo test --workspace
  111 passed, 1 ignored (17 suites)

CARGO_TARGET_DIR=/tmp/cubism-target-verify cargo clippy -p cubism-timeseries-bench --all-targets
  0 errors, 0 warnings from this crate
  (1 pre-existing, unrelated doc_lazy_continuation warning in
   crates/cubism-datafusion/src/udf.rs, not touched this session)

rustfmt --edition 2024 --check crates/cubism-timeseries-bench/src/lib.rs crates/cubism-timeseries-bench/src/main.rs
  passed
```

## Worktree state and ownership

This session's commit owns:

- `Cargo.toml` (already had the `cubism-timeseries-bench` member from the
  prior session; unchanged further this session — see diff before commit);
- `Cargo.lock` (`futures` added as a direct dependency of
  `cubism-timeseries-bench`; already resolved elsewhere in the tree, no new
  transitive dependencies);
- `crates/cubism-timeseries-bench/` (rewritten `lib.rs`, extended
  `main.rs`, `futures` dependency in `Cargo.toml`);
- `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` (Phase 0B status line);
- `docs/TIMESERIES_PHASE_0B_HARNESS.md` (rewritten for the streaming
  design and measured results);
- `docs/TIMESERIES_PHASE_0B_HANDOFF.md` (this file).

Pre-existing/unrelated work in the tree, not touched or committed by this
session (same exclusion Phase 0A's handoff already documented):

- `README.md`'s "Web analytics demo" section;
- `examples/web_analytics_demo/`;
- `.serena/` (local tooling state).

## Recommended next session

1. Read this handoff, [`TIMESERIES_PHASE_0B_HARNESS.md`](TIMESERIES_PHASE_0B_HARNESS.md),
   and [issue #1](https://github.com/jeromebanks/cubism-rs/issues/1) (including
   its progress comment).
2. Run the actual 10-100M row target on the 16 GB host and record real
   numbers — do not reuse the 1M/10M figures above as a stand-in.
3. Implement the local Spark adapter with byte-compatible KMV state
   (`preflight` already checks for `spark-submit`; none was available this
   session).
4. Add aligned/non-aligned short/long range-read cases per layout, and a
   process-level matrix runner (warm-up + ≥5 measured runs per
   configuration, isolated processes so `write_ms` stops being confounded).
5. Pin Parquet compression, row-group/file targets, and sort order in the
   results (the DataFusion memory limit is already pinned and recorded).
6. Write `TIMESERIES_PHASE_0B_RESULTS.md` and select the durable layout and
   local Rust/Spark boundary from that evidence.
7. Only after that decision, begin Phase 2 sparse temporal aggregation.

Do not start Phase 2/3 from this handoff, and do not treat "no longer a
20-30 GB ceiling" as "solved at 100M scale" — it isn't measured.

## Primary files

- [`TIMESERIES_PHASE_0B_HARNESS.md`](TIMESERIES_PHASE_0B_HARNESS.md)
- [`TIMESERIES_PHASE_1_HANDOFF.md`](TIMESERIES_PHASE_1_HANDOFF.md)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
- [`../crates/cubism-timeseries-bench/src/lib.rs`](../crates/cubism-timeseries-bench/src/lib.rs)
- [`../crates/cubism-timeseries-bench/src/main.rs`](../crates/cubism-timeseries-bench/src/main.rs)
- [Issue #1](https://github.com/jeromebanks/cubism-rs/issues/1)
