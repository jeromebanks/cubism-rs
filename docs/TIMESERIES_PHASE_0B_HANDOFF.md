# Time-Series Phase 0B Handoff

Date: 2026-08-11 (supersedes the 2026-08-08 version of this file)

Branch: `feature/timeseries-phase-0a`

Status: harness correctness slice complete; memory re-baselined at
1M/10M/25M rows on this host (still nothing at the 50-100M target); Spark
adapter (remaining-work item 1) implemented and correctness-verified at
25,000 rows; Spark has **not** been run at benchmark scale, so the actual
Rust-vs-Spark performance question Phase 0B exists to answer is still
open. Phase 0B is not complete.

## Executive handoff — the chain of sessions so far

1. **2026-08-08**: fixed a memory/correctness defect serious enough to
   block the phase (harness collected its whole result into one in-memory
   map; extrapolated to 22-33 GB at the 10-100M row target on a 16 GB
   host). Filed as
   [issue #1](https://github.com/jeromebanks/cubism-rs/issues/1), fixed via
   streaming per-bucket aggregation + a bounded `GreedyMemoryPool`. See the
   git history of this file for that session's full detail (superseded
   below, not repeated).
2. **2026-08-09 to 2026-08-11 (notebook session)**: re-baselined 1M/10M
   under a new process-level matrix runner and found real RSS running
   ~55-65% higher than the 08-08 figures (host contention, not a
   regression — see `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md`). Scaled to 25M
   successfully after hardening the runner against a real USB-drive
   disconnect. Investigated a user-raised scalability concern and found a
   real, previously-undocumented gap: DataFusion's aggregate-side spill
   behavior (as opposed to the sort operator's, which was already
   verified) has never been tested — filed as
   [issue #2](https://github.com/jeromebanks/cubism-rs/issues/2). Confirmed
   the mergeable-aggregator scatter/gather pattern the architecture was
   designed around has never been exercised by any benchmark run.
3. **2026-08-11 (this session)**: implemented remaining-work item 1 — see
   below. This is the only change this handoff documents in detail;
   items 1-3 above are prior work, cited for continuity.

## What changed this session: the Spark adapter (item 1)

New Scala/sbt project at `spark-adapter/` (sibling to `crates/`, not part
of the Cargo workspace), plus an updated `.claude/skills/spark-setup/`
skill (now installs/verifies `sbt`, reflects the Scala-not-PySpark
decision). Committed as `87380bf`. Design doc:
`~/.claude/plans/flickering-popping-harp.md` (full phase-by-phase plan,
worth reading before extending this).

**Scala was chosen over PySpark deliberately**: the custom KMV merge isn't
a Spark SQL built-in, and a Python UDF implementation would pay Python/JVM
serialization overhead that plain `sum`/`count` (which compile to the same
Catalyst plan regardless of driver language) wouldn't — that overhead
would bias the benchmark against Spark for reasons that have nothing to do
with Spark's actual engine.

**What it does** (minimal-scope adapter, per the plan): reads the exact
same generated source Parquet the Rust harness consumes, explodes each row
into the same 4 fixed XUnit cells (this benchmark's `BENCH_SPEC` is a
flat, fixed 2-dim spec — the general XUnit/lattice/filter-rule engine was
deliberately **not** ported, just hardcoded), aggregates
`(bucket, xunit) -> sum/count/KMV-blob`, computes the identical BLAKE3
semantic digest via a driver-side streaming fold (not `collect()`), and
emits a `run.json`-equivalent. It does **not** write any of the 4
candidate Parquet layouts — that's out of scope for this item.

**Verified, not assumed**: XXH3-64(seed 0) via `com.dynatrace:hash4j` and
BLAKE3-256 via `io.github.rctcwyvrn:blake3` were each checked against
independent oracles (official test vectors, and a cross-check against
Python's `xxhash`/`blake3` packages) before being trusted. The KMV wire
format and the canonical-XUnit ("CXU") encoding both reproduce
`cubism-core`'s own golden fixtures byte-for-byte. End to end: running the
**pinned 25,000-row seed-1 sparse source** (same file
`assert_golden`/`GOLDEN_SPARSE_25K_*` in
`crates/cubism-timeseries-bench/src/lib.rs` uses) through the Scala
adapter reproduces `cell_count = 51473` and
`semantic_digest_blake3 = 59f8fdfdbd48f3ffa983e30c1541885b336be6f524c49d61819908b6cda021a8`
**exactly** — proving XXH3 hashing, unsigned KMV ordering, CXU byte
layout, timestamp handling, and the 4-way lattice explosion are all
correct simultaneously. First real end-to-end run, no retries needed.

**Real traps hit and fixed, worth knowing before touching this code**:
- KMV hashes sort **ascending unsigned**; the JVM has no unsigned 64-bit
  type, so every comparison goes through `Long.compareUnsigned` explicitly
  (`Kmv.scala`). The golden fixture itself exercises this (one of its
  three hashes has the high bit set).
- `CAST(bucket_start AS BIGINT)` on a `TimestampType` column silently
  returns epoch **seconds**, not microseconds. Used `unix_micros(...)` via
  `expr(...)` instead (not exposed as a `functions.unix_micros` Scala
  method in Spark 4.2.0), verified against Rust's own
  `BASE_BUCKET_START_US` constant before trusting it.
- Case classes used with Spark's `Dataset`/`Encoder` machinery must be
  defined at file/object scope, **not inside a method body** — a
  method-local case class fails with "Unable to find encoder" even though
  it's structurally identical. Hit this on the first compile.
- Grouping is done via DataFrame `.groupBy()` on a `BinaryType` column
  (Catalyst value-equality), never an RDD keyed on a raw `Array[Byte]`
  (JVM reference-equality would silently make every row its own group).
- The KMV `Aggregator[IN,BUF,OUT]` needs a serializable buffer —
  `case class KmvBuf(k: Int, hashes: Array[Long])` via `Encoders.product`,
  not the mutable `Kmv` class itself.
- `aggregate_ms`/`digest_ms` are forced to materialize explicitly (`.cache()`
  + `.count()` before timing the digest fold) — Spark's laziness means an
  unforced measurement would time ~0ms of DataFrame construction and dump
  the entire pipeline cost into whichever stage happens to trigger it.

**Not yet run**: the timing figures from the 25k validation run
(`aggregate_ms=1787`, `startup_ms=1567`, `total_ms=4744`) are dominated by
one-time JVM/SparkSession startup at this tiny scale and **must not** be
used as evidence of anything about relative performance. No performance
comparison has happened yet — see the next section.

## What exactly is Phase 0B comparing? (read this before assuming more than is scoped)

This came up explicitly this session and is worth stating precisely,
because "Rust vs Spark" is ambiguous across at least three different axes,
and only one is currently in scope:

| Axis | What it would compare | Status |
|---|---|---|
| **1. Single-node engine throughput** (Phase 0B, current scope) | One Rust/DataFusion process vs one Spark JVM process in **`local[N]`** mode — both on the **same physical machine**, same source files, no real network shuffle, no cluster. | Correctness side done (this session). Performance side **not started** — nothing beyond the 25k correctness run has executed. |
| **2. Rust's own distributed story** (scatter/gather) | Multiple Rust worker processes each building partial cubes over a data shard, merged via the mergeable-aggregator design (`docs/scaling.md`) that KMV/sum/count were specifically built to support. | **Never benchmarked, not even once** — every Phase 0B run to date, on either engine, is single-process monolithic aggregation (confirmed in the notebook session's chokepoint investigation). |
| **3. Spark's real distributed story** (a cluster) | Spark on an actual multi-node cluster — could be a local dev simulation (OrbStack/k3d/kind) or a cloud cluster (GKE/EKS) — compared against Rust's distributed story (axis 2), not against single-node Rust. | **Not scoped into any phase yet.** `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`'s Phase 0B completion criteria explicitly state: *"Large distributed and billion-row comparisons are explicitly deferred to Phase 7"* — and Phase 7 itself depends on "measured production-like workloads from Phases 3-6," i.e. it's far downstream, not a near-term task. |

The harness doc (`docs/TIMESERIES_PHASE_0B_HARNESS.md`, "Known design
chokepoints") and the feasibility doc's **Fair comparison protocol**
(`docs/TIMESERIES_FEASIBILITY.md`) are both explicit that conflating axis 1
with axis 3 would be an invalid comparison **in Spark's favor** if Rust
loses locally (Spark's real scaling story is horizontal, untested here),
and equally invalid **in Rust's favor** if Spark loses locally without
accounting for its larger fixed per-job overhead at small scale. Verbatim
from the Fair Comparison Protocol: *"Compare local Rust and local Spark on
the same host first. Compare distributed modes separately on the same
instance pool; do not present local Rust versus a Spark cluster as one
undifferentiated score."*

**Practical takeaway for the next session**: everything in remaining-work
items 2-4 and 7 below is axis 1 only (`spark-submit --master local[N]` on
this same Mac mini, same as the Rust runs). If/when axis 2 or 3 become
relevant, they need their own separate benchmark design (matched hardware
or instance pool per the Fair Comparison Protocol) — that design doesn't
exist yet and isn't implied by anything built so far.

## Remaining Phase 0B work (renumbered against
`docs/TIMESERIES_PHASE_0B_HARNESS.md`'s original list)

1. ~~Implement the local Spark adapter with byte-compatible KMV state~~
   **DONE** (this session, commit `87380bf`) — correctness-verified at
   25k rows only, see caveats above.
2. Add aligned/non-aligned short/long range-read cases per layout. **Not
   started.**
3. Process-level matrix runner with RSS watchdog. **Done for Rust**
   (`crates/cubism-timeseries-bench/scripts/matrix_runner.py`, built in
   the notebook session) — **has zero Spark awareness.** It only knows
   how to invoke the compiled Rust binary's `rust` subcommand as a
   subprocess; extending it (or writing a parallel driver) to invoke
   `spark-submit` with the same RSS-watchdog/retry/mount-check pattern is
   new, unstarted work, needed before item 4 can measure Spark at scale
   under the same rigor as Rust.
4. Measure the actual 10-100M row target (axis 1 only, per above), for
   **both** engines. Rust has 1M/10M/25M (notebook session, real numbers
   in `TIMESERIES_PHASE_0B_HARNESS.md`'s "Memory scaling" section); Spark
   has nothing past the 25k correctness check. **Gated on user go-ahead
   past 50M rows**, same as before.
5. ~~Pin Parquet compression, row-group/file targets, and sort order~~
   **Done** (notebook session, 2026-08-09).
6. Write `TIMESERIES_PHASE_0B_RESULTS.md` with raw commands/artifacts.
   **Not started.**
7. Select the durable layout and local (axis 1) Rust/Spark boundary from
   that evidence. **Blocked on item 4.**

## Environment now in place (so the next session doesn't rediscover this)

- `openjdk@21`, `apache-spark` (4.2.0), `sbt` (2.0.6) all installed via
  `.claude/skills/spark-setup/scripts/install_spark.sh`. `JAVA_HOME` is
  exported in `~/.zshrc` (marked block) but **not** inherited by Claude
  Code's own Bash tool (non-interactive shells don't source `.zshrc`) —
  export it explicitly per command if needed:
  `export JAVA_HOME="/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home"`.
- `spark-adapter/` builds via `sbt package` (thin jar,
  `spark-core`/`spark-sql` are `% "provided"`). Dependency jars are
  resolved via Coursier at
  `~/Library/Caches/Coursier/v1/https/repo1.maven.org/maven2/...` — exact
  paths used this session:
  `com/dynatrace/hash4j/hash4j/0.30.0/hash4j-0.30.0.jar` and
  `io/github/rctcwyvrn/blake3/1.3/blake3-1.3.jar`.
- Reference invocation (axis 1, local mode, matches Rust's default
  `--partitions 4`):
  ```bash
  spark-submit \
    --class cubism.bench.SparkAggregate \
    --master "local[4]" \
    --jars "$HASH4J_JAR,$BLAKE3_JAR" \
    spark-adapter/target/scala-2.13/cubism-spark-adapter_2.13-0.1.0.jar \
    --input <source.parquet> --output-dir <dir> --partitions 4
  ```
- A validated 25k-row seed-1 sparse source exists at `/tmp/p0b/src25k.parquet`
  (scratch, not committed) — regenerate via
  `cargo run --release -p cubism-timeseries-bench -- generate --output <path> --rows 25000 --occupancy sparse --seed 1`
  if it's gone.

## Worktree state and ownership

This session's commit (`87380bf`) owns: `spark-adapter/` (all files),
`.claude/skills/spark-setup/` (both files, rewritten from a PySpark-era
version).

Pre-existing/unrelated work in the tree, **not touched or committed by
this session** (deliberately left for separate review, consistent with
prior sessions' handoff convention):

- `README.md`, `crates/cubism-timeseries-bench/src/lib.rs`,
  `docs/TIMESERIES_FEASIBILITY.md`, `docs/TIMESERIES_PHASE_0B_HARNESS.md`
  (all modified, uncommitted — from the notebook session, 2026-08-09/11);
- `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md`,
  `crates/cubism-timeseries-bench/examples/`,
  `crates/cubism-timeseries-bench/scripts/`,
  `examples/web_analytics_demo/` (untracked — also notebook session);
- `.serena/` (local tooling state, never committed by any session).

## Recommended next session

1. Read this handoff, `docs/TIMESERIES_PHASE_0B_HARNESS.md`,
   `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md`, and issues
   [#1](https://github.com/jeromebanks/cubism-rs/issues/1) and
   [#2](https://github.com/jeromebanks/cubism-rs/issues/2).
2. Decide whether to review/commit the notebook session's pending
   uncommitted changes (listed above) before building further on top of
   them — they've been sitting uncommitted across two sessions now.
3. Extend `matrix_runner.py` (or write a parallel Spark-aware driver) so
   Spark gets the same RSS-watchdog/retry rigor as Rust before item 4's
   real scale-up (item 3 above).
4. Only then run item 4 — the actual axis-1 (local, same-host) 10-100M row
   comparison for both engines, gated past 50M rows same as before.
5. If axis 2 or 3 (distributed, either engine) ever become the actual
   question, that needs a new benchmark design from scratch — nothing
   built so far implies or prepares for it, per the scoping table above.

## Primary files

- [`TIMESERIES_PHASE_0B_HARNESS.md`](TIMESERIES_PHASE_0B_HARNESS.md)
- [`TIMESERIES_PHASE_0B_NOTEBOOK.md`](TIMESERIES_PHASE_0B_NOTEBOOK.md)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
- [`TIMESERIES_FEASIBILITY.md`](TIMESERIES_FEASIBILITY.md) (Fair comparison
  protocol, decision matrix)
- [`../spark-adapter/`](../spark-adapter/)
- `~/.claude/plans/flickering-popping-harp.md` (full Spark adapter design)
- [`../crates/cubism-timeseries-bench/src/lib.rs`](../crates/cubism-timeseries-bench/src/lib.rs)
- [`../.claude/skills/spark-setup/`](../.claude/skills/spark-setup/)
- [Issue #1](https://github.com/jeromebanks/cubism-rs/issues/1),
  [Issue #2](https://github.com/jeromebanks/cubism-rs/issues/2)
