# Time-Series Phase 0B Handoff

Date: 2026-08-12, updated (supersedes the 2026-08-11 version of this file,
which itself superseded 2026-08-08)

Branch: `feature/timeseries-phase-0a`

Status: **Phase 0B's core decision is made — see
`docs/TIMESERIES_PHASE_0B_RESULTS.md`.** Select DataFusion/Rust as the
axis-1 aggregation engine and `xunit_registry` as the durable physical
layout (with `wide` as fallback). Correctness confirmed byte-identical
between Rust and Spark at 25k/1M/10M/25M rows (5 independent digest
matches); Rust wins decisively on performance (at least ~3x on raw
totals, ~6x on the one fair-slice measurement not confounded by USB I/O
variance) and reliability (100% vs. Spark's ~50% failure rate on
25M-row jobs, root cause unresolved). Scale-up was deliberately stopped
at 25M rows (user decision at the 50M gate) — **50-100M remains
unmeasured** and dense occupancy has no performance data at any scale;
neither blocks this decision but both remain open before trusting these
numbers at the harness's originally stated 10-100M target. Phase 2/3 are
now unblocked on this decision per the original gating instruction; see
"Remaining Phase 0B work" for what's still genuinely open.

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
3. **2026-08-11 (Spark adapter session)**: implemented remaining-work
   item 1 — see below. Superseded as "this session" by item 4.
4. **2026-08-11 (this session)**: committed the prior session's pending
   uncommitted work (see "Worktree state and ownership" below), then
   implemented remaining-work item 3 — extended
   `crates/cubism-timeseries-bench/scripts/matrix_runner.py` with Spark
   awareness. See `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entry 7 for full
   detail; summarized in "What changed this session" below.
5. **2026-08-11 to 2026-08-12 (this session, continued)**: implemented
   remaining-work item 2 (range-read cases — found and fixed two real bugs
   at 10M+ row density, see `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entries
   8/11), ran item 4's actual scale-up for both engines at 1M/10M/25M rows
   (stopped at the 50M gate per explicit user decision — see Entry 12 for
   the unresolved Spark `run_02` reliability finding at 25M), wrote item
   6's results document (`docs/TIMESERIES_PHASE_0B_RESULTS.md`), and made
   item 7's decision (Rust/DataFusion + `xunit_registry`, see "Status"
   above and the results doc's own "Decision" section).

## What changed this session: Spark-aware matrix runner (item 3)

`matrix_runner.py` gained an `engine=rust|spark` config key so a single
invocation can drive `spark-submit` with the same RSS-watchdog, retry,
mount-check, and disk-free rigor already built for the Rust `rust`
subcommand — previously it only knew how to invoke the Rust binary.
Spark's memory bound (`spark-driver-memory-mb`) is a required, separate
config key, **not** derived from Rust's `--memory-limit-mb`: the two knobs
bound different things (DataFusion's execution-memory pool vs. a whole JVM
heap in `local[N]` mode), and defaulting one from the other risked
producing a misleading "Spark is slow/OOMs" result for a sizing reason
unrelated to either engine. Picking the actual value per row count is
deliberately left open for item 4. Also added a cross-engine
`semantic_digest_blake3` check (not asked for by item 3's text, but cheap
given everything else being built) — when one invocation runs both engines
at the same row count/occupancy, the runner now automatically confirms
their aggregate output normalizes to the same digest, closing part of
issue #1 finding 3's "no independent oracle" gap.

**Validated with real runs, not just unit-level**: a 25k-row `engine=spark`
config through the new code path reproduced the pinned golden digest
(`59f8fdfd...cda021a8`, `cell_count=51473`) exactly. The one real risk in
reusing the existing PID-based RSS watchdog for a `spark-submit`
invocation — whether `spark-submit` execs into `java` (PID preserved) or
forks-and-waits (watchdog would watch the wrong process) — was checked
directly: the same command run under `/usr/bin/time -l` independently read
~820MB peak RSS, matching the watchdog's own 808-879MB band. A mixed
rust+spark invocation at 25k rows exercised the new cross-engine digest
check end to end and printed `MATCH`.

See `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entry 7 for the full session
log, including what's still open (the `spark-driver-memory-mb` value to
use at scale, and the `run.json` schema asymmetry between engines that
item 6 will need to account for).

## What changed the prior session: the Spark adapter (item 1)

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
   **DONE** (Spark adapter session, commit `87380bf`) — correctness-verified
   at 25k rows only, see caveats above.
2. ~~Add aligned/non-aligned short/long range-read cases per layout.~~
   **DONE** (this session, commit `5b0fb91` + bugfix `e27e274`). Two real
   bugs found and fixed against 10M+-row data (non-aligned cases were
   silently skipped; the aligned branch had a symmetric edge-case bug) —
   both fixes are regression-tested. Results in
   `docs/TIMESERIES_PHASE_0B_RESULTS.md`'s "Range-read results". Only
   swept against the 25M-row Rust output at the pinned row-group size
   (65,536 rows) — not swept across row-group sizes or other row counts.
3. ~~Process-level matrix runner with RSS watchdog~~ **DONE for both
   engines** (Rust: notebook session; Spark: this session). See "What
   changed this session" above and
   `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entry 7. `matrix_runner.py`'s
   `engine=spark` configs still need a `spark-driver-memory-mb` value
   chosen deliberately per row count before item 4 runs — not defaulted,
   see above for why.
4. ~~Measure the actual 10-100M row target (axis 1 only, per above), for
   both engines~~ **DONE through 25M, stopped there per explicit user
   decision at the 50M gate.** Rust: 1M/10M/25M, 5 measured runs each, 100%
   reliable. Spark: 10M (5 measured runs, 100% reliable), 25M (2 measured
   runs; 2 of 4 attempts at a third run failed with an unresolved root
   cause — see `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entry 12). **50-100M
   remains genuinely unmeasured** and needs fresh user go-ahead plus (per
   Entry 12) more diagnostic room for Spark's reliability issue before
   it's trustworthy at that scale.
5. ~~Pin Parquet compression, row-group/file targets, and sort order~~
   **Done** (notebook session, 2026-08-09).
6. ~~Write `TIMESERIES_PHASE_0B_RESULTS.md` with raw commands/artifacts.~~
   **DONE** (this session, commit `c9d0ac7`).
7. ~~Select the durable layout and local (axis 1) Rust/Spark boundary from
   that evidence.~~ **DONE** (this session) — Rust/DataFusion as the
   engine, `xunit_registry` as the durable layout (`wide` as fallback).
   See `docs/TIMESERIES_PHASE_0B_RESULTS.md`'s "Decision" section for the
   full reasoning and caveats. **This satisfies the gate on starting
   Phase 2/3** (temporal aggregation / production Iceberg persistence per
   `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`).

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

All prior sessions' work is now committed (as of this session):
- `87380bf` — Spark adapter session: `spark-adapter/` (all files),
  `.claude/skills/spark-setup/` (both files).
- `57825b7` — this session, committed first as a housekeeping step before
  building further: the notebook session's write-path pin
  (`crates/cubism-timeseries-bench/src/lib.rs`, `docs/TIMESERIES_FEASIBILITY.md`,
  `docs/TIMESERIES_PHASE_0B_HARNESS.md`, `docs/TIMESERIES_PHASE_0B_HANDOFF.md`),
  the pre-Spark-awareness `matrix_runner.py`, and
  `crates/cubism-timeseries-bench/examples/explain_aggregate.rs`.
- `354240d` — this session: the unrelated web analytics demo
  (`examples/web_analytics_demo/`, `README.md`), source files only —
  generated `events.csv`/`web_analytics_cube.parquet` deliberately left
  out, matching the repo's convention for its other `examples/` demos.
- `e27621b` — item 3: Spark-aware `matrix_runner.py`, this handoff,
  `TIMESERIES_PHASE_0B_HARNESS.md`, `TIMESERIES_PHASE_0B_NOTEBOOK.md`
  Entry 7.
- `5b0fb91` — item 2: range-read subcommand and types
  (`crates/cubism-timeseries-bench/src/lib.rs`, `src/main.rs`).
- `e27e274` — range-read bugfix (non-aligned cases silently skipped at
  10M+ row density) plus regression test.
- `2fbc212`, `f62ab9f`, `0dbfa01` — notebook entries and harness-doc
  updates recording the 1M/10M/25M real runs and the Spark reliability
  investigation (item 4).
- `c9d0ac7` — item 6/7: `docs/TIMESERIES_PHASE_0B_RESULTS.md` (the results
  document and decision record).

Remaining uncommitted/untracked, **deliberately left alone**:
`.serena/` (local tooling state, never committed by any session, and not
covered by `.gitignore` on purpose per prior-session convention —
generated `examples/web_analytics_demo/events.csv` and Spark/Cargo build
output are excluded via `.gitignore` instead).

## Recommended next session

Phase 0B's core decision is made (item 7, see "Status" above) — **Phase
2/3 work can now start.** If instead the next session is continuing to
harden Phase 0B's evidence before building on it, in rough priority order:

1. Read this handoff, `docs/TIMESERIES_PHASE_0B_RESULTS.md` (the decision
   and its caveats), `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` (especially
   Entries 11/12), and issues
   [#1](https://github.com/jeromebanks/cubism-rs/issues/1) and
   [#2](https://github.com/jeromebanks/cubism-rs/issues/2).
2. **Spark's `run_02` reliability root cause at 25M** (2 of 4 direct
   `spark-submit` attempts killed by an unexplained
   `SparkContext.stop()`-races-`Dataset.count()` NullPointerException,
   memory/disk/wrapping/concurrency all ruled out) — needs a dedicated
   host with no concurrent `claude` sessions and a Spark event-log/history
   -server capture across several repeated attempts. Blocks trusting
   Spark numbers at 50M+.
3. **50-100M rows, both engines** — needs fresh user go-ahead (explicitly
   deferred, not just gated, at this session's 50M checkpoint) and
   benefits from item 2 above being resolved first so Spark's numbers at
   that scale are trustworthy.
4. **Dense occupancy** has zero performance data past the 25k smoke test
   at any scale, for either engine — `docs/TIMESERIES_PHASE_0B_RESULTS.md`
   "Remaining gates" item 3.
5. **25M's fair-slice performance ratio is confounded** by USB-drive I/O
   variance in the layout-write subtraction (`TIMESERIES_PHASE_0B_RESULTS.md`
   "Reconciled performance comparison") — re-running on faster/local disk,
   or timing aggregation independently of writes in the harness itself,
   would resolve this without needing more rows.
6. If axis 2 or 3 (distributed, either engine) ever become the actual
   question, that needs a new benchmark design from scratch — nothing
   built so far implies or prepares for it, per the scoping table above.

## Primary files

- [`TIMESERIES_PHASE_0B_RESULTS.md`](TIMESERIES_PHASE_0B_RESULTS.md) (the
  decision record — read this first)
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
