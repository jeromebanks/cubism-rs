# Time-Series Phase 0B Results

Status: axis-1 (local, same-host) Rust/Spark comparison complete at
1M/10M/25M rows, **sparse occupancy only**; dense occupancy has no
performance data at any benchmark scale and 50-100M was explicitly
deferred, not attempted (see "Scope")

Date: 2026-08-12

Branch: `feature/timeseries-phase-0a`

Implementation:
[`crates/cubism-timeseries-bench`](../crates/cubism-timeseries-bench),
[`spark-adapter/`](../spark-adapter/)

## Decision

**Select DataFusion/Rust as the aggregation and storage engine for the
axis-1 (single-node) benchmark path.** Both engines produce byte-identical
semantic output at every scale tested (25k/1M/10M/25M rows), so this is
not a correctness call — it is a performance and reliability call, and on
both counts Rust wins decisively at every measured scale on this host: at
least ~3x faster on raw totals, and ~6x faster on the one fair-slice
measurement (10M rows) this document trusts without caveat (see
"Reconciled performance comparison" below — the 25M fair-slice figure is
confounded by I/O variance and is not cited as a headline number here),
lower peak memory, and 100% reliable across every run
this project has ever executed, versus a real, unexplained ~50% failure
rate for the Spark adapter's own long-running (20+ minute) jobs at 25M
rows (see "Spark reliability at 25M").

**Select `xunit_registry` as the durable physical layout**, with `wide` as
the fallback if a downstream consumer specifically needs self-contained
rows without a registry join. Both are ~1.6x smaller on disk than
`per_measure`/`tagged_struct` at every scale measured, write 1.6-2.5x
faster, and showed no worse range-read scan behavior. `per_measure` and
`tagged_struct` are near-identical to each other in every measured
dimension (byte size, write time, scan behavior) — the difference between
them is a schema-shape/ergonomics question this benchmark's numbers do not
decide, and neither is recommended as the primary layout given their
disk-footprint cost.

**Caveat on `xunit_registry`**: its size, write-time, and range-read
advantages are all measured on the `states` file in isolation. Any
consumer that needs XUnit identity (not just the aggregated values) must
also read `xunit_registry.parquet` and join on the 32-byte content ID —
that join was never benchmarked here. The other three layouts are
self-contained and pay no such cost. If a downstream workload's query
pattern makes that join expensive or awkward, `wide` (self-contained, 1
row/cell, still ~1.6x smaller than `per_measure`/`tagged_struct`) is the
safer default despite its larger footprint than `xunit_registry`.

**What this decision does not cover, on purpose** (see "Scope" below):
Spark's real distributed/cluster story (axis 3), Rust's own scatter/gather
distributed design (axis 2), the 50-100M row target, dense occupancy past
the 25k smoke test, and whether a *differently-implemented* Spark job
(native DataFrame `explode()` instead of this adapter's typed-Dataset
`.flatMap()`, see "Spark adapter implementation caveat") would close any
of the observed gap. None of those questions are answered here and none
should be inferred from this decision.

## Scope: what this document is and is not comparing

Per `docs/TIMESERIES_PHASE_0B_HANDOFF.md`'s scoping table, this is **axis
1 only**: one Rust/DataFusion process vs. one Spark JVM in `local[4]`
mode, both on the same physical machine, same source files, no cluster,
no real network shuffle. Axis 2 (Rust's own scatter/gather distributed
design) and axis 3 (a real Spark cluster) have never been benchmarked by
any session and are explicitly deferred to Phase 7 per
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md`. Conflating this result with
either axis would be invalid in both directions — see the handoff doc's
verbatim citation of the Fair Comparison Protocol.

**Row-count scope**: 1M, 10M, and 25M rows, sparse occupancy. The
harness's own stated target is 10-100M; 50M and 100M were explicitly
**not attempted** this session, gated on user go-ahead per every prior
session's convention, and the user chose to stop at 25M given the time
already spent on Spark's 25M reliability issue (see below) and that the
performance gap at 25M is already large enough that further scale is
unlikely to change the qualitative conclusion. **Dense occupancy** was
only ever exercised at the 25k smoke-test scale (`TIMESERIES_PHASE_0B_HARNESS.md`
"Correctness validation performed") — there is no dense-occupancy
performance data at any benchmark scale for either engine.

## Environment

10-core Apple M4 Mac mini, 16 GB memory, macOS 15.7.7, Rust 1.96.1,
DataFusion 54.0.0, `openjdk@21`, Apache Spark 4.2.0, Scala 2.13.18. This
host was **not quiet** during any of this project's sessions — 2-4 other
`claude` CLI sessions plus normal desktop applications were running
throughout, documented as a standing condition in
`docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entry 1 onward. Absolute numbers
below should be read as "measured on a contended 16GB Mac mini," not a
dedicated benchmark rig.

## Correctness results

The Spark adapter's `semantic_digest_blake3` (BLAKE3-256 over sorted
`(bucket, XUnit content ID, canonical bytes, sum, count, KMV)` tuples,
identical construction on both engines) matched Rust's own digest at
every row count where both engines were run:

| Rows | Cell count | Digest (first 16 hex chars) | Match |
| --- | --- | --- | --- |
| 25,000 | 51,473 | `59f8fdfdbd48f3ff...` | yes (adapter build session, 2026-08-11) |
| 1,000,000 | 1,936,575 | `8d5e66a9fd6e892b...` | yes (this session, matrix runner cross-check) |
| 10,000,000 | 15,137,276 | `71fbd214aafef8bf...` | yes (this session, matrix runner cross-check) |
| 25,000,000 | 28,846,459 | `f58b9b65eec78e1c...` | yes, twice (this session, direct `spark-submit` x2) |

Five independent confirmations across four scales, all exact matches. XXH3
hashing, unsigned KMV ordering, canonical XUnit encoding, sum/count
aggregation, and the 4-way lattice explosion are all byte-for-byte
consistent between a Rust/DataFusion implementation and an independently
implemented Scala/Spark one. This is the correctness oracle
`docs/TIMESERIES_PHASE_0B_HARNESS.md` issue #1 finding 3 noted the harness
never had before this project.

## Raw performance numbers

5 measured runs (Rust) / 2-5 measured runs (Spark, see "Spark reliability
at 25M") per config, `--partitions 4`, pinned `UNCOMPRESSED` compression,
65,536-row layout row groups. Full per-run data in
`/Volumes/YOTUO/phase0b/matrix/*/run_*/run.json` and
`*.watchdog.json` sidecars (Rust only — Spark's 25M runs bypassed the
watchdog, see below).

| Config | Engine | Measured runs | `total_ms` median | Peak RSS median |
| --- | --- | --- | --- | --- |
| 1M sparse | Rust | 5 | 10,822 (10.8s) | 1,690 MB |
| 10M sparse | Rust | 5 | 91,220 (91.2s) | 5,420 MB |
| 10M sparse | Spark | 5 | 303,969 (5.07 min) | 6,351 MB |
| 25M sparse | Rust | 5 | 442,498-602,021 (7.4-10.0 min), median 448,448 | 7,286 MB |
| 25M sparse | Spark | 2 | 1,290,946 / 1,397,527 (21.5 / 23.3 min) | not measured (see below) |

Spark's 25M peak RSS is unmeasured because those two successful runs were
launched as direct `spark-submit` invocations, bypassing
`matrix_runner.py`'s RSS-sampling watchdog entirely (see "Spark
reliability at 25M" for why). The JVM was given `--driver-memory 8192m`;
actual peak RSS (heap + off-heap + metaspace) was not sampled but is
expected to exceed 8192MB based on the 10M config's own pattern (6144m
driver-memory → 6,351-6,646 MB measured peak RSS, ~10% over the heap
bound).

**Raw `total_ms` is not a fair comparison and should not be read as one.**
Rust's number includes writing all 4 candidate Parquet layouts and a full
read-back verification pass; Spark's adapter writes no layouts and
performs no verification (both explicitly out of scope per its own design
— see `SparkAggregate.scala`'s `comparability_caveat` field, included
verbatim in every Spark `run.json`). See the next section for the
reconciled slice.

## Reconciled performance comparison (the fair slice)

Isolating "read source, aggregate, compute the semantic digest" on both
sides — no layout writes, no verification, since only Rust does either:

- **Rust's isolated aggregation time** = `aggregate_ms` (which spans
  aggregation *and* all 4 streamed layout writes, per
  `TIMESERIES_PHASE_0B_HARNESS.md`'s schema-version-2 note) **minus** the
  sum of that run's four `layouts[].write_ms` values. Rust's digest
  computation is fused into this same streaming loop with no separate
  timer, so this slice is "`aggregate_ms` net of time inside the writer
  calls" — not a directly-measured "pure aggregation" timer.
- **Spark's isolated aggregation+digest time** = `aggregate_ms` +
  `digest_ms` directly, since the adapter already separates them and
  performs no writes at all.

| Rows | Rust: aggregate_ms | minus layout write_ms | = aggregate_ms net of writes | Spark: aggregate_ms + digest_ms | Ratio (Spark/Rust) |
| --- | --- | --- | --- | --- | --- |
| 1M | 7,026 | 1,579 | 5,447 ms | not measured at matrix scale* | n/a |
| 10M | 60,548 | 12,288 | 48,260 ms | 180,331 + 107,152 = 287,483 ms | **~6.0x — trustworthy** |
| 25M | 238,121 | 139,529 | 98,592 ms | 1,146,793 + 245,646 = 1,392,439 ms | ~14.1x — **see caveat below, do not cite this number alone** |

\* Spark was only run at 1M-row scale as a single unmeasured probe during
this session's item-3 validation (`docs/TIMESERIES_PHASE_0B_NOTEBOOK.md`
Entry 7) — 3.2GB peak RSS, no timing trusted as a benchmark figure since
it wasn't run under the matrix runner's rigor.

**The 25M subtraction is confounded and its ~14.1x ratio should be read
as an unreliable upper bound, not a measurement.** At 1M and 10M, layout
`write_ms` is 20-22% of `aggregate_ms` — a stable fraction consistent with
a fixed per-row write cost. At 25M it jumps to **59%** (139,529 /
238,121) — a discontinuity, not a trend, and the likely cause is sitting
in this project's own notebook: at 25M the four uncompressed layouts
total ~15.5 GB written to the external USB volume
(`/Volumes/YOTUO`), which `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entry 5
already documented degrading under load, with one read-back pass at this
scale taking 357s against a normal ~203s. If 25M's `write_ms` is inflated
by drive throughput rather than intrinsic write cost, subtracting it
overstates how much of `aggregate_ms` is real aggregation work. The tell
is in the derived figures themselves: the "net of writes" number grows
only 2.04x from 10M to 25M (48,260 → 98,592ms) for 2.5x the rows and
~1.9x the cells — **sublinear**, while unsubtracted `aggregate_ms` grows
3.9x — **superlinear**, which is what convex per-bucket density growth
predicts and is the more plausible shape. The sublinear "net of writes"
figure at 25M is very likely a subtraction artifact, not a real result.

**A more defensible number at 25M is the raw `total_ms` ratio**, which
doesn't depend on the write-time subtraction at all: Rust's `total_ms`
median (448,448ms — includes all 4 layout writes *and* a full read-back
verification pass, neither of which Spark's adapter performs) versus
Spark's `total_ms` (mean of its 2 successful runs, 1,344,236ms) gives
**~3.0x**. This ratio is a conservative floor: it is structurally unfair
to Rust, since Rust's number includes real work (writes, verification)
that Spark's adapter simply never does. The true fair-slice ratio at 25M
is somewhere between this ~3.0x floor and the unreliable ~14.1x subtracted
figure above; this document does not have clean enough write-cost
accounting at 25M to narrow that range further, and a claim that "the gap
widens with scale" is **not supported** by this data — that claim
appeared in an earlier draft of this document and is retracted here.

## Spark adapter implementation caveat

`SparkAggregate.scala` explodes each source row into its 4 XUnit cells via
a **typed `Dataset[SourceRow].flatMap`** (a per-row Scala closure), not a
DataFrame-native `explode(array(...))` expression. This was a deliberate
choice recorded in the adapter's own design doc
(`~/.claude/plans/flickering-popping-harp.md`) to keep the custom KMV
merge and canonical-XUnit encoding logic in ordinary Scala rather than
Catalyst expressions — but it means the measured Spark numbers reflect
**this specific adapter's implementation choices**, not a ceiling on what
Spark itself can do for this workload. A DataFrame-native rewrite of the
explode step was not attempted and might narrow the gap above; this
result should be read as "this adapter, as built, is measurably slower
than Rust/DataFusion on this host — by at least ~3x on raw totals and by
~6x on the one fair-slice measurement (10M) this document trusts," not as
a claim about Spark the engine's inherent ceiling on this workload.

## Spark reliability at 25M

Every Rust run across every config and scale this project has ever
executed succeeded on the first attempt — zero RSS-limit kills, zero
crashes, zero retries needed (`matrix_results.json` / watchdog sidecars
across dozens of runs). Spark was equally reliable through 10M (5/5
measured runs succeeded cleanly under `matrix_runner.py`). At 25M, a
**genuine, unresolved reliability problem** appeared: of 4 direct
`spark-submit` attempts at a third measured run (`run_02`), 2 succeeded
and 2 were killed by an external signal that triggered Spark's JVM
shutdown hook mid-job (`SparkContext.stop()` racing an in-flight
`Dataset.count()`, producing a clean `NullPointerException` and stack
trace — not a crash, not an OOM). Checked and ruled out: system memory
pressure (60%+ free throughout, confirmed via `memory_pressure -Q` at
each failure), disk space (`/Volumes/YOTUO` stayed at 700+ GiB available),
`matrix_runner.py`'s Python-subprocess wrapping (both a wrapped run and a
direct `spark-submit` invocation failed at different points), and
concurrent foreground work (a run with heavy concurrent `cargo build`
activity succeeded; a run with zero concurrent activity failed). **Root
cause not established** — see `docs/TIMESERIES_PHASE_0B_NOTEBOOK.md`
Entry 12 for the full investigation. Given the ~50% failure rate observed
in a small sample, this is itself a data point: whatever governs it, it
made completing a full 5-measured-run matrix at 25M impractical within
this session, and the same or a worse rate should be expected at 50-100M
without further investigation.

## Memory scaling

Peak RSS at each scale (median across measured runs), continuing the
table `TIMESERIES_PHASE_0B_HARNESS.md`'s "Memory scaling" section started:

| Rows | Rust peak RSS | Spark peak RSS | Spark memory bound |
| --- | --- | --- | --- |
| 1M | 1,690 MB | not measured at matrix scale | n/a |
| 10M | 5,420 MB | 6,351 MB (+17%) | `--driver-memory 6144m` |
| 25M | 7,286 MB | not measured (bypassed watchdog) | `--driver-memory 8192m` |

Spark's memory bound (`--driver-memory`, a whole JVM heap) is not the same
control as Rust's `--memory-limit-mb` (DataFusion's own execution-memory
pool) — see `crates/cubism-timeseries-bench/scripts/matrix_runner.py`'s
module docstring for why they were never treated as equivalent, and why
the values used here (6144m at 10M, 8192m at 25M) were chosen from Rust's
own measured peak RSS at each scale plus headroom, not copied 1:1.

## Disk footprint by layout

All four candidate layouts, `UNCOMPRESSED` (pinned per
`TIMESERIES_PHASE_0B_HARNESS.md` "Pinned semantics" — never evaluated
against a compressed codec; treat as an open question if disk becomes a
real constraint at 50-100M+ scale):

| Rows | Source bytes | per_measure | wide | tagged_struct | xunit_registry (states+registry) | Total / source ratio |
| --- | --- | --- | --- | --- | --- | --- |
| 1M | 29.7 MB | 280.5 MB | 181.9 MB | 280.5 MB | 153.3 MB | 30.2x |
| 10M | 296.3 MB | 2,488.3 MB | 1,577.1 MB | 2,488.3 MB | 962.7 MB | 25.4x |
| 25M | 740.6 MB | 5,203.9 MB | 3,250.0 MB | 5,203.9 MB | 1,914.6 MB (states: 1,861.4 MB) | 21.0x |

`per_measure` and `tagged_struct` are within 0.01% of each other in size
at every scale (expected: identical row count and near-identical schema
width, differing only in flat-columns-vs-struct encoding). Both are
**1.6x larger than `wide`** and **2.7x larger than `xunit_registry`'s
states file**, because they materialize 3 rows per cell (one per
sum/count/kmv measure) versus 1. `xunit_registry` is the most compact
per-state-row option because it references XUnits by a fixed 32-byte ID
instead of embedding variable-length canonical bytes in every row — at
the cost of a second file and a join to resolve the canonical XUnit for
any row.

Write time follows the same pattern (25M, `run_01`): `per_measure`
45.0s, `tagged_struct` 47.8s, `wide` 28.3s, `xunit_registry` 18.4s
(states file only).

The overall 21-30x expansion ratio over source bytes is **not primarily
layout bloat** — it substantially reflects the cube's own semantics: the
sparse lattice explosion (global + device + region + device×region cells
per source event) already multiplies row count well before any layout
encoding is applied, and `per_measure`/`tagged_struct` multiply it again
3x for their per-measure-row shape.

## Range-read results

Aligned/non-aligned, short (4-bucket)/long (48-bucket) bucket-range reads
against the real 25M-row Rust output
(`crates/cubism-timeseries-bench`'s new `range-read` subcommand, see
`docs/TIMESERIES_PHASE_0B_HARNESS.md` item 2 and
`docs/TIMESERIES_PHASE_0B_NOTEBOOK.md` Entries 8/11 for the tool's design
and a real bug found and fixed while validating it against this exact
data). Byte fraction scanned (row-group pruning, no column projection):

| Layout | Short range | Long range |
| --- | --- | --- |
| per_measure | 2.4% of file | 28.6% of file |
| tagged_struct | 2.4% of file | 28.6% of file |
| wide | 2.5% of file | 28.6% of file |
| xunit_registry (states) | 2.5% of file | 28.6% of file |

All four layouts show near-identical *fractional* scan behavior for a
given time range — row-group pruning works comparably well regardless of
layout choice, at this row count and row-group size. Note the asymmetry
this table doesn't capture: `xunit_registry`'s row is for its `states`
file alone — a query needing XUnit identity also pays a registry-file
read and join (see the Decision section's caveat above), which the other
three self-contained layouts never require. The **absolute**
bytes scanned differ in direct proportion to each layout's total file
size (from the disk-footprint table above), so `xunit_registry`/`wide`
are cheaper to range-query in absolute I/O terms simply because they are
smaller files, not because they prune better. Aligned vs. non-aligned
starts differ by well under 1% of scanned bytes/rows at this row-group
size (32-378 row groups touched depending on range length, out of
231-1,321 total) — alignment matters far less than layout choice at this
scale for this benchmark's specific row-group size (65,536 rows); this
may not hold at different `LAYOUT_ROW_GROUP_ROWS` settings or row counts,
which were not swept.

## Verification performed

```text
cargo test -p cubism-timeseries-bench          # 7 passed, 1 ignored
cargo test --release -p cubism-timeseries-bench -- --ignored   # 1M golden digest, passed
cargo clippy -p cubism-timeseries-bench --all-targets --no-deps -- -D warnings   # clean
rustfmt --edition 2024 --check <touched files>  # clean
```

Every reported cross-engine digest match above was produced by an actual
run of both engines against the same generated source file this session
(or the adapter-build session for 25k), not asserted from memory.

## Remaining gates before Phase 2/3

Per `docs/TIMESERIES_PHASE_0B_HARNESS.md`'s original framing, Phase 2
(temporal aggregation) and Phase 3 (production Iceberg persistence) stay
blocked until this decision is recorded — it now is (see "Decision"
above) — but several things remain open for whoever picks this back up:

1. **50-100M rows, both engines**: not attempted, gated on user go-ahead,
   deferred this session given the 25M Spark reliability finding above.
2. **Spark's `run_02` reliability root cause**: unresolved; a future
   session with more diagnostic room (dedicated host, no other concurrent
   `claude` sessions, a Spark event-log/history-server capture across
   several attempts) is needed before trusting Spark numbers at 50M+.
3. **Dense occupancy**: no performance data past the 25k smoke test at
   any scale for either engine.
4. **Compression**: `UNCOMPRESSED` is pinned and untested against a real
   codec; the 21-30x disk expansion ratio above may look very different
   compressed.
5. **The Spark adapter implementation caveat**: whether a DataFrame-native
   (non-`flatMap`) rewrite of the explode step narrows the observed gap
   was not tested.
6. **Axis 2 and axis 3** (distributed Rust scatter/gather; a real Spark
   cluster): out of scope for Phase 0B entirely, per the scoping table —
   any future distributed comparison needs its own benchmark design from
   scratch, per the Fair Comparison Protocol.
