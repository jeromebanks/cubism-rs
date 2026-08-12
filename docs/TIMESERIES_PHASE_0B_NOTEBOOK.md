# Time-Series Phase 0B Working Notebook

Date started: 2026-08-09

Purpose: running log of Phase 0B session work, one entry per task, each
opening with a resource snapshot taken *before* the task starts. This is a
working log, not a polished deliverable -- see
[`TIMESERIES_PHASE_0B_RESULTS.md`](TIMESERIES_PHASE_0B_RESULTS.md) (to be
written per remaining-work item 6) for the final measured results, and
[`TIMESERIES_PHASE_0B_HANDOFF.md`](TIMESERIES_PHASE_0B_HANDOFF.md) /
[`TIMESERIES_PHASE_0B_HARNESS.md`](TIMESERIES_PHASE_0B_HARNESS.md) for the
standing contract this work operates under.

Context for why this notebook exists: the prior session found the harness's
pre-fix memory use could exceed this host's 16GB RAM (commit `64a2b58`,
issue #1). The fix landed, but the actual 10-100M row target has still
never been run on this box, so every scale-up step here is treated as a
possible repeat of that incident until measured otherwise.

## Snapshot command (run identically every time)

```bash
memory_pressure -Q
vm_stat
sysctl vm.swapusage
uptime
df -h / /Volumes/YOTUO
ps aux | sort -rk4 | head -6
ps aux | grep -i claude | grep -v grep   # call out other live sessions explicitly
```

Threshold: if any candidate run's DataFusion `--memory-limit-mb` plus
observed harness-side RSS would plausibly exceed **~12GB** (leaving ~4GB
headroom on the 16GB host for OS + other apps), stop and ask before
proceeding rather than extrapolating through it.

---

## Entry 1 -- Baseline + preflight (Task #1)

**Before:**
- `memory_pressure -Q`: 78% system-wide free
- `vm_stat`: free 19,823 pages / speculative 34,538 / purgeable 18,363
  (page size 16384) -> ~1.1GB immediately free+speculative+purgeable;
  active 387,184 (~6.2GB), inactive 352,133 (~5.6GB), wired 118,025 (~1.9GB)
- `vm.swapusage`: 0 used
- `uptime`: load averages 1.20 3.73 8.11 (15-min figure still elevated from
  earlier in the session; 1-min figure is low and trending down)
- disk: `/` 101Gi avail; `/Volumes/YOTUO` (external, where the repo's
  `target` symlink points) 850Gi avail
- other live sessions: **two other `claude` processes running**
  (pid 17890 `claude --resume`, pid 22336 `claude`) alongside this one --
  the box is not quiet. Also Ghostty, Chrome, OrbStack helper, Claude
  desktop app running.

**Action:** ran
`CARGO_TARGET_DIR=/tmp/cubism-target-phase0b cargo run -p cubism-timeseries-bench -- preflight`
(debug build, ~90s compile).

**Result:** fails cleanly as documented --
`Error: spark-submit: not found; Phase 0B cannot select a Rust/Spark boundary yet`.
No Java runtime, no `pyspark` installed either. Confirms remaining-work
item 1 (Spark adapter) needs a JVM + Spark + PySpark install before it can
even start -- a multi-GB, machine-state-changing install. **Gated for
explicit user go-ahead (Task #6), not started.**

**After:** no measurable change (preflight does no data generation or
aggregation).

---

## Entry 2 -- Item 5: pin compression/row-group/sort order (Task #2)

**Before:**
- `memory_pressure -Q`: 76% free
- `uptime`: load averages 2.28 3.10 6.51
- disk: `/` 97Gi avail, `/Volumes/YOTUO` 850Gi avail
- no other new processes started since Entry 1

**Action:** in `crates/cubism-timeseries-bench/src/lib.rs`:
- Added `PINNED_COMPRESSION = Compression::UNCOMPRESSED` (matches prior
  implicit parquet-rs default -- **not changed, just made explicit**) and
  applied it to all three `WriterProperties` builders (source file, the two
  per-layout writers).
- Added `LAYOUT_ROW_GROUP_ROWS = 65_536` as one named constant replacing two
  independent magic numbers (`OpenLayout::create`, `write_single_batch`).
- Recorded both as new `run.json` fields: `GenerationMetrics.compression`,
  `RustRunMetrics.layout_compression` / `layout_row_group_rows`.
- Documented explicitly (code comment + `TIMESERIES_PHASE_0B_HARNESS.md`)
  that the aggregate-stream sort (`bucket_start` only, load-bearing for
  bounded memory) and the written/verified `(bucket, content ID)` order are
  two different, both-pinned sorts -- adding `xunit` to `AGGREGATE_SQL`'s
  `ORDER BY` would reintroduce the pre-fix unbounded-memory bug.
- Bumped `harness_schema_version` 2 -> 3 (additive fields only).

**Verification performed** (per the doc's own rule: confirm the golden
digest still reproduces before trusting any write-path touch):
- `cargo test -p cubism-timeseries-bench` (debug): 5 passed, 1 ignored --
  including `generator_is_byte_deterministic` and both 25k golden digests.
- `cargo test --release -p cubism-timeseries-bench -- --ignored`: the 1M-row
  golden digest still reproduces byte-for-byte (12.89s).
- `cargo clippy -p cubism-timeseries-bench --all-targets -- -D warnings`:
  clean for this crate. (One pre-existing, unrelated clippy failure in
  `cubism-datafusion/src/udf.rs` -- a doc-comment indentation lint -- was
  not introduced by this change and was left alone as out of scope.)
- `rustfmt --edition 2024 --check` on the touched file: clean.

**Flagged, not fixed:** compression is pinned to UNCOMPRESSED but its
effect on disk footprint at 10-100M rows x 4 layouts has not been
evaluated. Check free disk on the output volume before scaling up (see
Item 4 gate).

**After:**
- `memory_pressure -Q`: 68% free (down from 76%)
- `uptime`: load averages 5.31 7.19 7.57 (1-min jumped from 2.28) --
  attributable to the release build's parallel compilation (`cargo build
  --release`, ~3m49s, uses all cores), not a leak; expected to settle.
  Noting it anyway per the "note resource usage" instruction rather than
  silently discounting it.
- disk: `/` 96Gi avail (small drop from build artifacts in
  `/tmp/cubism-target-phase0b`, on the boot volume -- worth watching as
  more scale runs land there; consider moving `CARGO_TARGET_DIR` output
  data, not the build cache, to `/Volumes/YOTUO` for later steps if `/tmp`
  fills up)

No problems. Ready for Task #3 (matrix runner + RSS watchdog).

## Entry 3 -- Item 3: process-level matrix runner + RSS watchdog (Task #3)

**Before:** 68% free, load 2.44/5.00/6.56, `/` 96Gi avail. Same two other
live `claude` sessions as Entry 1/2, plus this session's own release
builds.

**Action:** wrote `crates/cubism-timeseries-bench/scripts/matrix_runner.py`
(stdlib-only Python, no new deps). Per config: generates/reuses a shared
source file, runs 1 warm-up + N measured `rust` subcommand invocations as
real OS subprocesses, polls each child's RSS via `ps -o rss=` on an
interval, kills and records if RSS exceeds `--rss-limit-mb` (default
12000, i.e. ~4GB headroom on this 16GB host), aborts remaining runs in a
config on a kill rather than continuing, checks free disk before each
run/generation via `--min-free-gb` (default 10), and captures the same
`memory_pressure -Q` / `vm_stat` / `vm.swapusage` / loadavg / `df`
environment snapshot this notebook uses, before and after each config, into
`matrix_results.json`.

**Validation performed:**
1. 25k-row sparse smoke config (2 measured runs): mechanics worked
   end-to-end, but wall time was 0.5s -- too fast for the 500ms poller to
   catch a real peak (`peak_rss=4MB` is a startup-sample artifact, not a
   real reading). **Known limitation: the watchdog's polling approach is
   only meaningful for runs lasting several seconds or more** -- fine for
   the 1M+ configs Phase 0B actually cares about, not for sub-second smoke
   runs.
2. 1M-row sparse config (3 measured runs, `--poll-interval-ms 200`):
   mechanics worked correctly, `peak_rss` 1649-1802MB across the 3 measured
   runs.

**Finding -- current RSS readings run well above the doc's recorded 1M
baseline, and this host is not quiet:**

`docs/TIMESERIES_PHASE_0B_HARNESS.md` "Memory scaling" records **1.09GB**
peak RSS / 771MB footprint for 1M sparse rows (measured 2026-08-08). This
session's tool reads **1.65-1.8GB** for the identical config. Cross-checked
independently with `/usr/bin/time -l` on the same binary/input/flags
(bypassing the Python watchdog entirely, to rule out a polling bug):

```
1,702,510,592  maximum resident set size   (1623.6 MB)
  672,205,560  peak memory footprint        (641.1 MB)
```

The `/usr/bin/time -l` "maximum resident set size" (1.62GB) matches the
watchdog's own reading closely -- so **the watchdog is measuring
correctly**; the discrepancy is between *today's* number and the number
recorded in the doc two days ago, not a bug in this new tool. Notably,
"peak memory footprint" (641MB) came in *lower* than the doc's 771MB while
"maximum resident set size" came in *higher* than the doc's 1.09GB -- an
inconsistent shift, not a uniform scaling factor, which argues against a
simple "the box is just busier so everything reads higher" explanation.

Not resolved in this session. Plausible contributors, none confirmed:
- This host was demonstrably **not quiet** during every run in this
  notebook: two other live `claude` CLI sessions (pids 17890, 22336 as of
  Entry 1) plus Chrome/OrbStack/Ghostty running throughout, vs. whatever
  state the box was in for the original Aug 8 measurement.
- Possible run-to-run allocator/heap-growth variance in the DataFusion
  GROUP BY / sort path, independent of system load.
- The Item 5 write-path pin (Entry 2, same session) changed compression
  settings but not the aggregation path this measurement exercises, and
  the golden digest still reproduces byte-for-byte -- unlikely to be the
  cause, but not formally ruled out by anything other than reading the
  diff.

**Why this matters:** the whole point of the watchdog is to stop a run
before it repeats the original memory incident. If real-world RSS at a
given row count now runs ~50% higher than the last documented number, the
growth curve toward 10M/25M/50M needs to be re-established from fresh
measurements, not assumed to follow the old curve. **Recommending to the
user: re-baseline both 1M and 10M under this tool before trusting any
comparison at 25M/50M**, and ideally do that re-baseline with the other
`claude` sessions/apps quiesced so the numbers reflect the harness alone.

**After:** 68% free, load settled to 2.50/4.27/6.07, `/` 91Gi avail (down
~5Gi from source generation + 4 run output dirs across the two validation
configs, all on the boot volume via `/tmp`). No kills triggered in
validation (expected -- both configs were well inside the 12000MB default
limit). No disk or memory problems, but see the finding above before
proceeding to Task #5.

## Entry 4 -- Item 4: 1M + 10M sparse re-baseline (Task #5, part 1)

User decision: re-baseline now, as-is, on the box in its current (not
fully quiet) state rather than waiting to quiesce other sessions first.

**Before:** 69% free, load 2.77/2.96/4.76, `/` 91Gi avail, 14
Claude-related processes running (two other live sessions).

**Fixed a real bug in the matrix runner first:** the initial 1M validation
run's summary undercounted `measured_run_count` (reported "2" instead of
5) because reused/skipped run directories weren't contributing their
`peak_rss_mb` to the aggregate stats -- that number was never persisted
anywhere except the watchdog's in-memory state for the process that
produced it. Fixed by writing a `{label}.watchdog.json` sidecar per run
and loading it back when a run directory is reused, instead of silently
excluding it from stats. Wiped the two smoke-test dirs
(`matrix-smoketest*`) rather than splice pre-fix and post-fix data, and
reran cleanly into `/tmp/phase0b/matrix` (later moved, see below).

**Action:** `matrix_runner.py` with both configs in one invocation, 5
measured runs each, `--rss-limit-mb 12000 --poll-interval-ms 200
--min-free-gb 10`:

| Config | Measured runs | Peak RSS median | Peak RSS max | Wall median | Cell count |
| --- | --- | --- | --- | --- | --- |
| 1M sparse | 5/5, none killed | 1690 MB | 1763 MB | 11.1s | 1,936,575 |
| 10M sparse | 5/5, none killed | 5420 MB | 5688 MB | 91.5s | 15,137,276 |

Cell counts match the doc's recorded values for both row counts exactly
-- generation is confirmed deterministic, so the RSS shift versus the doc
is a host/measurement-condition effect, not a data or logic change.

**This corroborates, not resolves, the Entry 3 finding:** both 1M and 10M
now read **~55-65% higher** than the doc's Aug 8 numbers (1M: 1.09GB ->
1.69GB median, +55%; 10M: 3.42GB -> 5.42GB median, +58%). Two points at a
consistent proportional offset is stronger evidence for a systemic effect
(host load, allocator/OS behavior under contention) than a scale-dependent
one -- but still not proven, and not chased further this session per the
user's "proceed as-is" call. **Treat 5.42GB as the real current 10M
baseline for extrapolation purposes, not 3.42GB.**

**Second problem found live, fixed immediately:** `/` (boot volume) dropped
from 96Gi to 48Gi available from just these two configs (12 run directories
x 4 uncompressed layouts) -- confirms the Item 5 disk-footprint caveat
(UNCOMPRESSED at scale is expensive) and matches advisor guidance from the
start of this session to keep large I/O off the boot volume. **Moved
`/tmp/phase0b` (47GB) to `/Volumes/YOTUO/phase0b`** (803Gi avail after the
move, vs. `/`'s 95Gi). All Task #5 remaining steps (25M, 50M) use
`--source-dir /Volumes/YOTUO/phase0b/sources --output-root
/Volumes/YOTUO/phase0b/matrix` from here on. Do not point future large runs
back at `/tmp` without a explicit reason.

**After:** 66% free, load settled to 1.66/2.24/3.39, `/` 95Gi avail
(post-move), `/Volumes/YOTUO` 803Gi avail. No kills, no correctness
problems. Proceeding to 25M sparse next.

## Entry 5 -- 25M sparse crashed: external drive disconnect, not a memory kill (Task #5)

**What happened:** launched 25M sparse (5 measured runs) against
`/Volumes/YOTUO`. Warmup + run_01/02/03 succeeded (peak RSS 7.0-7.5GB,
well under the 12GB watchdog limit, ~440s each). `run_04` died with an
unhandled `FileNotFoundError` while the watchdog tried to write its sidecar
-- not a watchdog kill (those print `KILLED` and set `result.killed`; this
was a raw crash with a Python traceback, exit code 1).

**Root cause, confirmed not assumed:** checked `diskutil info /Volumes/YOTUO`
before and after -- the device node changed from `disk7s1` (session start)
to `disk5s1` (post-crash). macOS only reassigns a device node like that on
an actual disconnect+reconnect, not a transient hiccup that stays mounted.
`log show` around the failure time (16:02:38) confirms it independently:
`storagekitd` and `appstoreagent` logged the container as `Mounted: No`
then `"Volume did appear"` for the same APFS UUID at that exact timestamp.
The user's own framing matches: this is a USB HDD, and "we might have these
issues" -- external USB drives dropping under sustained multi-minute heavy
write load (4 runs x ~440s x ~15GB of uncompressed layout writes each) is a
real, expected failure mode for this hardware, not a bug to root-cause away.
Cleaned up the orphaned ~14.9GB of partial `run_04` output (no `run.json`,
so it was mid-write when the process died).

**What was NOT the cause, to close the loop on the earlier "is our explode
too big" question:** separately confirmed the memory-growth pattern
(1M/10M/25M all reading well above the Aug 8 doc's numbers, see Entry 3/4)
is consistent with normal cube-lattice explode behavior (4x fan-out for
this 2-dim+global spec, confirmed in code: 8 devices x 50,000 regions x
1M/10M row cell counts match the doc exactly) plus known-and-documented
non-pooled harness buffers -- not a code defect, and not related to this
crash. The crash is purely a storage-layer reliability problem.

**Fix -- user asked for resilience, not root-causing the hardware:**
rewrote `matrix_runner.py`'s failure handling. Before, any exception
(including a transient I/O error) propagated all the way up and killed the
whole multi-config matrix run, discarding results already collected in
memory for the current config (though `matrix_results.json` itself was
safe -- written after each completed config). Changes:
- **`mount_ready()`**: writes/reads/deletes a sentinel file to verify a
  volume actually accepts I/O right now, with retry + backoff. Called
  before source generation and before each run, and again if a run's own
  attempt fails, so retries wait for the drive to actually be back instead
  of immediately re-hammering a still-recovering mount.
- **`run_one_with_retries()`**: each `rust` subcommand invocation is a
  clean, deterministic function of its input file, so a from-scratch retry
  is always safe -- wipes the partial `run_dir` and log, re-checks the
  mount, retries up to `--max-retries` (default 2) with backoff scaled by
  attempt number. An RSS-limit kill is explicitly NOT retried (that's the
  watchdog doing its job correctly, not a transient fault) -- returned to
  the caller immediately, same abort-remaining-runs behavior as before.
  A run that exhausts retries is recorded `failed=True` with a reason
  instead of raising -- the config's remaining runs are then intentionally
  aborted too (if the drive is still unreachable, further runs in the same
  config will likely fail the same way), but the matrix run itself
  continues to whatever config comes next.
- **`main()`**: the per-config loop now catches failures from `run_config`
  itself (e.g. source generation never recovering) and records a
  `config_error` entry rather than crashing the whole matrix; later configs
  still run.
- **`--inter-run-pause-s`** (default 3s): a deliberate pause between
  measured runs, since 4 back-to-back ~440s runs of sustained multi-GB
  writes with zero breathing room is exactly the load pattern that preceded
  the disconnect. Cheap mitigation, not a proven fix -- noted as such.
- Watchdog sidecars now persist `failed`/`fail_reason`/`attempts` too, so
  a failed run is correctly excluded from `measured_run_count` and the
  RSS/wall stats on reuse (this reuses the same sidecar mechanism added in
  Entry 4 for the undercounting bug).

**Verified before trusting it:** two isolated unit tests against the actual
`run_one_with_retries` function (not a rewrite-and-hope) using a fake
`bench-bin` shell script: (1) fails twice then succeeds on attempt 3 ->
confirmed `attempts=3`, `failed=False`, correct `run_json` recovered,
partial output from the two failed attempts correctly wiped between tries;
(2) always fails -> confirmed it gives up after `max_retries` (3 total
attempts), returns `failed=True` with a reason, and critically does
**not** raise -- the calling process stays alive. Also reran the existing
1M happy-path config end-to-end through the full CLI (not just the unit
tests) to confirm the resilience changes didn't disturb the normal
zero-failure path: 2/2 measured runs, `attempts=1` each, RSS in the same
1.67-1.75GB band as Entry 3/4.

**Not done / open:** did not add write-level retry logic inside the Rust
harness itself (deliberately -- a partial retry mid-stream inside the
correctness-critical streaming aggregation loop risks producing subtly
wrong output after a remount, since an open `File`/`ArrowWriter` handle
across a volume disconnect can't be safely resumed; failing fast and
letting the *outer* orchestrator retry the whole deterministic run is the
safer place for this). Did not verify whether `--inter-run-pause-s 3`
actually prevents another disconnect -- that's an untested hypothesis, not
a confirmed fix; if 25M or 50M drops again, the retry logic should now
survive it cleanly rather than crashing, but the underlying drive
flakiness itself has not been eliminated. Did not investigate the USB
enclosure/cable/power situation physically (out of scope for a code
change).

**Next:** retry 25M sparse under the hardened script.

## Entry 6 -- 25M retry succeeded; investigated scalability chokepoints (Task #5 + #7)

**25M retry, hardened script:** 5/5 measured runs completed. `run_04`'s
first attempt failed transiently (exit=1, no `run.json`) -- the retry logic
handled it automatically, succeeded on attempt 2, matrix continued to
`run_05` without operator intervention. Real validation of Entry 5's fix,
not just the unit tests. Results: peak RSS median 7286MB, max 7517MB
(all comfortably under the 12GB watchdog limit), wall median 448.8s.
Cell count 28,846,459.

**User asked a direct question mid-run:** worried the Rust/DataFusion
design may not be as scalable as the original Spark implementation, asked
me to document any design problems/chokepoints found. This became a real
investigation, not just answering from what was already known:

- Built `crates/cubism-timeseries-bench/examples/explain_aggregate.rs` (kept
  in the repo as a reusable diagnostic) and ran `EXPLAIN VERBOSE` against
  `AGGREGATE_SQL` in an isolated build (`/Volumes/YOTUO/phase0b/explain-target`,
  cleaned up after) so it wouldn't touch the running 25M job's binary.
  Confirmed: `SortExec` sits *above* a two-phase `AggregateExec`
  (Partial -> FinalPartitioned) in the physical plan -- the GROUP BY is
  NOT bucket-streaming despite the code comment describing bucket-ordered
  output; the aggregation must process the whole run before sorting starts.
- Checked `crates/cubism-datafusion/src/udaf.rs`: the custom sketch
  `GroupsAccumulator`s do implement `size()` (5 impls found), so per-group
  memory reporting exists in principle -- but whether DataFusion's
  aggregate operator actually spills on this pinned pool, for these custom
  accumulators, in this version, is unverified (only the *sort* operator's
  spill was ever tested, per the existing "Memory scaling" section).
- Ran a cheap 1M-row `--partitions 1` vs `--partitions 4` comparison (1501MB
  vs 1690MB median) specifically to test whether parallelism multiplies
  hash-table memory -- it doesn't meaningfully (~13%, not ~4x). Negative
  result, recorded so it isn't re-investigated later.
- Computed excess-RSS-over-pinned-pool per cell at 10M and 25M (~234 and
  ~190 bytes/cell respectively) -- consistent across a 2.5x scale change,
  which argues the excess tracks aggregation state rather than the
  previously-blamed non-pooled harness buffers (those are capped
  near-constant, so can't explain a gap that grows with scale).

Full writeup with the plan text, the table, and the "what this does/doesn't
say about Spark" framing is in
`docs/TIMESERIES_PHASE_0B_HARNESS.md`'s new "Known design chokepoints"
section, cross-linked from `docs/TIMESERIES_FEASIBILITY.md`'s Rust-vs-Spark
table and risk list. Filed as
[github.com/jeromebanks/cubism-rs#2](https://github.com/jeromebanks/cubism-rs/issues/2)
per the user's request, matching the format of issue #1. **Bottom line given to the user:** no Spark comparison
has run yet, so the original worry is still empirically open either way --
but the investigation found a real, previously-undocumented gap between
"the harness's write-side memory is bounded" (true, that's what commit
`64a2b52` fixed) and "the pipeline's overall peak memory is bounded" (not
established -- DataFusion's own aggregate-side memory has an unverified
spill story), plus confirmed the mergeable-aggregator scatter/gather
pattern that the architecture was designed around has never actually been
exercised by any benchmark run so far -- every run to date is single-process,
single-machine, monolithic aggregation.

**After:** system healthy throughout (no kills, no drive issues on this
round), `/Volumes/YOTUO` 715Gi avail after cleanup.

## Entry 7 -- Item 3: Spark-aware matrix runner (Task #7)

**Before:** picked up a fresh session per `docs/TIMESERIES_PHASE_0B_HANDOFF.md`
(2026-08-11 version). First reviewed and committed the prior notebook
session's uncommitted work (write-path pin + matrix runner +
re-baseline evidence as one commit `57825b7`; the unrelated web analytics
demo as a separate commit `354240d`, source files only -- generated
`events.csv`/`web_analytics_cube.parquet` deliberately left out, matching
this repo's existing convention for the other `examples/` demos).
`cargo test -p cubism-timeseries-bench` (5 passed, 1 ignored) reconfirmed
clean before touching anything further.

**Action:** extended `crates/cubism-timeseries-bench/scripts/matrix_runner.py`
in place (not a parallel driver -- the RSS-watchdog/retry/mount-check/env-
snapshot machinery is engine-agnostic and was worth sharing, not
duplicating) rather than writing a separate Spark driver:

- `--config` gained an optional `engine=rust|spark` key (defaults to
  `rust`, so every existing recorded invocation in
  `TIMESERIES_PHASE_0B_HARNESS.md`/`HANDOFF.md` still parses and runs
  unchanged). `engine=spark` configs require `spark-driver-memory-mb`
  instead of `memory-limit-mb` -- **deliberately not auto-derived from
  Rust's `--memory-limit-mb`**: DataFusion's `GreedyMemoryPool` bounds
  execution memory inside a process that measured 5.4GB RSS at 10M rows;
  `--driver-memory` bounds the *whole JVM heap* in `local[N]` mode (no
  separate executor process to also size). Copying the Rust number across
  risks reading as "Spark is slow/OOMs" for a sizing artifact that has
  nothing to do with either engine -- this needs a deliberate per-config
  choice when item 4 actually runs at scale, not a default in this script.
- New `build_spark_cmd()` shells out to `spark-submit --class
  cubism.bench.SparkAggregate --master local[N] --driver-memory {N}m
  [--conf spark.local.dir=...] [--jars ...] <adapter-jar> --input ...
  --output-dir ... --partitions N --memory-limit-mb {N}` (the last flag is
  informational-only in the adapter itself, per `SparkAggregate.scala`,
  but passed through so `run.json`'s `memory_limit_note` records the same
  number that was actually enforced via `--driver-memory`, not an unset
  gap).
- New `--spark-local-dir` (+ a disk-free check on it, mirroring the
  existing `output_root` check) -- Spark's shuffle spill defaults to
  `/tmp` on the boot volume otherwise, the same failure mode that dropped
  `/` from 96Gi to 48Gi in Entry 4, just on Spark's side this time instead
  of the Rust harness's uncompressed layout writes.
- New `--java-home`, overlaid onto a copy of `os.environ` (not a bare
  override dict) for the `spark-submit` subprocess only -- Claude Code's
  own non-interactive Bash tool doesn't source `~/.zshrc`, so `JAVA_HOME`
  being exported there (per HANDOFF.md) doesn't reach this script's
  subprocesses without this.
- Rust's own `config_label` format (`{occupancy}-{rows}rows-p{partitions}-
  mem{mb}mb`) is byte-identical to before this change specifically so the
  existing 1M/10M/25M run directories under `/Volumes/YOTUO/phase0b/matrix`
  are still reused via the watchdog-sidecar mechanism, not silently
  re-run. Spark configs get a `spark-...-drivermem{mb}mb` label instead --
  deliberately not the same shape as Rust's `mem{mb}mb` suffix, so the
  label itself doesn't imply the two memory knobs are equivalent.
- **Cross-engine digest check, new and not asked for by item 3's text but
  cheap given everything else being built**: when one invocation runs both
  engines at the same `(rows, occupancy)`, `main()` now compares their
  `semantic_digest_blake3` values from `run.json` and reports MATCH/
  MISMATCH (exit code 3 on mismatch). This is the same golden-digest
  discipline the harness already applies to itself
  (`docs/TIMESERIES_PHASE_0B_HARNESS.md` "Correctness validation
  performed"), now automated across engines instead of only checked once
  by hand at 25k rows during the Spark adapter's own build session. Issue
  #1 finding 3 named "no independent oracle for the aggregate values
  themselves" as a real gap; this closes it at whatever scale a given
  matrix invocation covers, for free, without adding a separate
  verification pass.

**Validated, not assumed** (per this notebook's own convention of
cross-checking a new tool against an independent measurement before
trusting it):
1. Ran a real `engine=spark` 25k-row config through the extended runner
   (existing `/Volumes/YOTUO/phase0b/sources/sparse-25000-seed1.parquet`,
   already-built `spark-adapter/target/scala-2.13/cubism-spark-adapter_2.13-0.1.0.jar`
   from commit `87380bf`, this host's Coursier-cached hash4j/blake3 jars).
   3 runs (1 warmup + 2 measured) all succeeded, `peak_rss` 808-879MB.
   `run.json`'s `semantic_digest_blake3` reproduced the pinned golden value
   (`59f8fdfd...cda021a8`, `cell_count=51473`) exactly -- the runner's
   plumbing (arg construction, env overlay, watchdog, sidecar) produces a
   real, correct result, not just an exit-0 process.
2. **Confirmed the watchdog actually observes the JVM, not a wrapper
   shell** (the one real risk in reusing `sample_rss_kb(proc.pid)`
   unchanged for a `spark-submit` invocation): re-ran the identical
   `spark-submit` command directly under `/usr/bin/time -l`, bypassing the
   Python watchdog entirely. Result: 860,307,456 bytes (~820MB) maximum
   resident set size -- matches the watchdog's own 808-879MB band closely.
   Confirms `spark-submit` execs into `java` (PID preserved, no
   fork-and-wait wrapper hiding the real process from `ps -o rss=`), same
   conclusion as Entry 3 reached for the Rust binary via the same
   cross-check technique.
3. Ran a mixed `engine=rust` + `engine=spark` invocation, both at 25k
   rows/sparse, in one matrix. Cross-engine digest check printed `rows=25000
   occupancy=sparse: MATCH -- {'rust': '59f8fdfd...', 'spark':
   '59f8fdfd...'}` with identical digests, `matrix_results.json`'s final
   shape correctly nested under `cross_engine_digest_checks`. Exit code 0.
4. `python3 -m py_compile` clean; `--help` output sane.

Scratch output (`/tmp/phase0b-spark-smoke`, `/tmp/phase0b-cross-smoke`,
`/tmp/phase0b-spark-smoke-timecheck`) cleaned up after validation -- none
of this touched `/Volumes/YOTUO/phase0b/matrix`'s existing 1M/10M/25M
results.

**Addendum -- 1M-row Spark probe, run before handing off:** advisor review
flagged a real untested risk in everything validated above: every Spark
run so far (the adapter session's 25k golden check, and this session's
three validations) used the 25k source, whose 168-bucket/51,473-cell
aggregate is tiny next to `SparkAggregate.scala`'s `toLocalIterator()`
digest fold, which pulls one Spark partition to the driver at a time --
untested at a cell count actually approaching driver-memory pressure, and
a partition-level OOM there would show up as a bare `exit != 0`, not a
digest mismatch, so the cross-engine check alone couldn't have caught it.
Ran `engine=spark,rows=1000000,occupancy=sparse,partitions=4,
spark-driver-memory-mb=4096` against the existing
`/Volumes/YOTUO/phase0b/sources/sparse-1000000-seed1.parquet` source (2
runs, ~13-16s each, peak RSS 2840-3223MB, comfortably under both the
4096m driver-memory bound and the 12000MB watchdog limit). `run.json`'s
`semantic_digest_blake3` reproduced Rust's own pinned 1M-row golden value
(`8d5e66a9fd6e892b63ac144f1456b0d76017c36377e1be608bfd935c6c0e9620`,
`cell_count=1936575`, from issue #1) exactly -- the shuffle -> AQE ->
`orderBy` -> `toLocalIterator` path holds up correctly at 1.94M cells /
4 partitions, not just at the 25k/51k-cell scale every prior Spark run
exercised. This meaningfully de-risks item 4's scale-up: the first real
exercise of that code path is now a real (if small) success, not an
unknown discovered hours into a 10M+ run.

**Not done / open, flagged for item 4:**
- The actual `spark-driver-memory-mb` value to use at 10M/25M/50M+ scale
  is still an open choice -- this session deliberately did not pick one
  (see above). Item 4 should size it from Rust's own measured peak RSS at
  each row count (`TIMESERIES_PHASE_0B_HARNESS.md` "Memory scaling"/issue
  #2 finding 3 tables), not guess. The 1M probe's own 2840-3223MB reading
  (against a 4096m bound) is one more data point for that sizing, on top
  of Rust's own 1M/10M/25M peak-RSS table.
- `run.json` schema asymmetry between engines is unchanged by this
  session and still real: Rust's `aggregate_ms` spans aggregate-and-write
  across all 4 layouts plus a separate `verification_ms`; Spark's
  `aggregate_ms`/`digest_ms` cover neither layout writes (out of scope,
  per the adapter's own design) nor verification. The adapter's own
  `comparability_caveat` field already says as much; item 6's results
  writeup needs to reconstruct a genuinely comparable time slice (e.g.
  source-read-through-digest on both sides) rather than diffing
  `aggregate_ms` raw.
- Did not run Spark past 25k rows this session -- 10M/25M/50M+ under this
  tooling is item 4, separately gated past 50M rows.

**After:** no memory/disk problems; scratch dirs removed. Ready for item 4
(the actual scale-up) once `spark-driver-memory-mb` is chosen per config.

## Entry 8 -- Item 2: range-read benchmark cases (Task #8)

Built while item 4's 10M rust+spark matrix run was in the background (see
Entry 9 for what went wrong operationally with that background run, found
during this entry's work) -- item 2 itself is self-contained and doesn't
touch the aggregation/write path Entry 4-7 hardened.

**Design, since the harness doc only specified this in one sentence
("add aligned and non-aligned short/long range-read cases for every
retained layout, including files and bytes scanned")**: each of the four
candidate layouts (`per_measure.parquet`, `wide.parquet`,
`tagged_struct.parquet`, `registry_states.parquet` -- `xunit_registry.parquet`
has no `bucket_start` column and is deliberately never touched by a
bucket-range scan) is written as ArrowWriter row groups whose boundaries
are cut by cumulative row count (`LAYOUT_ROW_GROUP_ROWS = 65,536`), not by
bucket boundary -- confirmed from the write path, not assumed. Whether a
row group spans many buckets or a single bucket spans many row groups
depends on per-bucket cell density versus that constant, which is scale-
and occupancy-dependent (`TIMESERIES_PHASE_0B_HARNESS.md` "Memory
scaling"'s 11,527-vs-90,102 cells/bucket figures at 1M vs 10M rows). So
"aligned" vs "non-aligned" is defined empirically per file, from each row
group's own min/max `bucket_start` statistics (read via `parquet-rs`'s
`ParquetMetaData`, no DataFusion involved) -- **aligned** means the range
start exactly equals some row group's minimum `bucket_start` (no row group
is scanned partly for out-of-range rows at the front); **non-aligned**
means the start falls inside a row group whose own minimum is an earlier
bucket, so that group has real leading waste. "Short" (default 4 buckets)
and "long" (default 48 buckets, both CLI-configurable) are picked
independently per alignment, so 4 layouts x 2 lengths x 2 alignments = 16
cases per `range-read --run-dir <rust-run-dir>` invocation. `bytes_scanned`
is the sum of scanned row groups' full compressed byte size from metadata
(equal to uncompressed size under the pinned `UNCOMPRESSED` codec) --
deliberately not a column-projected byte count, since a real query over a
chosen layout would read whichever columns it actually needs, and
row-group-level bytes is the right unit for measuring pruning effectiveness
independent of that choice. `rows_scanned` vs. `rows_matched` (from an
actual filtered read, projecting only `bucket_start` to keep it cheap)
quantifies row-level overscan even within the correctly-pruned row-group
set.

**Implementation**: `RangeReadConfig`/`RangeReadReport`/`RangeReadCaseMetrics`
(+ a `skipped` list, so a layout with no viable candidate at some scale is
recorded explicitly rather than silently dropped) in `lib.rs`, a new
`range-read --run-dir DIR [--short-buckets N] [--long-buckets N]` CLI
subcommand in `main.rs`, no changes to the aggregation/write path or
`run.json` schema.

**Verified, not assumed**: new `#[tokio::test]`
`range_reads_are_correct_against_an_independent_full_scan` generates a
200k-row fixture, runs it through `run_rust`, then `run_range_reads`, and
for every case (a) recomputes `rows_matched` via a completely separate
full, unfiltered scan of the same file (not by reusing the row-group-
pruning code path under test) and asserts exact equality, (b) asserts the
alignment property itself directly -- an aligned case's start must equal
some row group's own minimum, a non-aligned case's must not -- rather than
trusting the selection logic that produced it. `cargo test -p
cubism-timeseries-bench`: 6 passed (was 5), 1 ignored, unchanged.
`cargo clippy --all-targets --no-deps -- -D warnings`: clean (the
pre-existing unrelated `cubism-datafusion` doc-lint failure, noted in
Entry 2, still blocks the whole-workspace clippy run, so `--no-deps`
isolates this crate same as before). `rustfmt --edition 2024 --check`:
clean. Manually ran `range-read` against a real 1M-row Rust run dir
(`/Volumes/YOTUO/phase0b/matrix/sparse-1000000rows-p4-mem2048mb/run_01`):
16/16 cases produced, no skips, numbers sane (e.g. `wide` layout short-
non-aligned touched 1 row group / 65,536 rows scanned for 46,260 matched
-- real overscan, not a degenerate 1:1); also ran against the tiny 25k-row
fixture to check the skip path doesn't false-positive at small scale (it
didn't -- 16/16 cases even there, since even a single row group spanning
the whole file still yields valid aligned/non-aligned starts).

**Not done / open**: only run against Rust output so far -- the harness
doc's item 2 doesn't specify whether Spark's output needs equivalent
range-read cases, and the Spark adapter (deliberately, per its own scope
note) writes none of the 4 layouts, so there is nothing on the Spark side
to range-read; this is a Rust-layout-selection tool, not a cross-engine
one. Not yet run at 10M/25M/50M+ scale -- only 25k and 200k so far; running
it against the existing 1M/10M/25M run dirs once item 4 has produced them
is cheap (read-only, no new aggregation) and worth doing before item 6's
results writeup.

## Entry 9 -- Operational incident during item 4: a self-inflicted duplicate
background run caused a real host memory-pressure crisis (Task #6, retro)

**What happened**: launched item 4's 10M rust+spark matrix run via `nohup
... &` backgrounded through the Bash tool's own `run_in_background`. The
outer bash-tool call returned near-instantly (backgrounding via `&` means
the launcher script exits right after forking, before the real work
finishes) and was misread as "the whole matrix run finished." A `ps aux`
check immediately after found no matching process -- but that check was
run in a race window before the real work had (re)appeared, not after it
had actually died. Concluded (wrongly) that `nohup`'s detachment had let
the harness's own process-group cleanup kill the real work, and relaunched
a **second, independent** invocation against the same
`--output-root`/configs, this time via `run_in_background` alone (no
`nohup`/`&`). Continued other foreground work (a `cargo build --release` +
`cargo test` + `cargo clippy` cycle for item 2, see Entry 8) while both
believed-sequential runs were, in fact, running **concurrently** --two
`matrix_runner.py` processes and two `-Xmx6144m` Spark JVMs, both racing
writes into the identical
`spark-sparse-10000000rows-p4-drivermem6144mb/warmup` directory.

**Root cause, confirmed via `log show`, not assumed**: `memorystatus:
killing_idle_process` kernel log entries from ~22:09 to ~22:14 show a
sustained jetsam kill spree (dozens of idle daemons: `cfprefsd`,
`routined`, `photoanalysisd`, several `mdworker` Spotlight-indexing
processes killed by `SIGKILL`), consistent with severe host-wide memory
pressure -- explained by two 6GB-heap JVMs plus this session's own
concurrent release-mode `cargo build`/`clippy` (rustc/LLVM codegen is
memory-hungry) plus (per `ps aux`) **four** other live `claude` sessions on
the same 16GB host, none of it visible to the matrix runner's own RSS
watchdog, which only ever watched its own child's RSS against
`--rss-limit-mb`, not total system memory. Checked and ruled out the
USB-disconnect failure mode from Entry 5 specifically (`diskutil info
/Volumes/YOTUO` showed a stable, still-mounted volume throughout; `log
show` found no `Mounted: No`/remount events in the failure window) --
this was purely a memory-pressure incident, a different failure mode than
Entry 5's, on the same class of "host isn't quiet" risk this notebook has
flagged since Entry 1.

**Fix, this session**: identified both process trees via `ps aux` (matched
on the literal `matrix_runner.py`/`SparkSubmit` command lines, confirming
PID identity before acting), asked the user for explicit go-ahead before
`kill`ing anything (the auto-mode permission classifier itself blocked
the first unprompted attempt), terminated all four PIDs cleanly
(`SIGTERM`, both matrix_runner.py orchestrators and both Spark JVMs),
confirmed `memory_pressure -Q` recovered to 75% free system-wide, removed
the one stale artifact left behind (`warmup.log` with no `run_dir` and no
watchdog sidecar -- the runner's own reuse-check already handles this
state correctly as "not yet attempted," so no other cleanup was needed),
and relaunched a single clean invocation without doing any other
concurrent foreground work this time.

**Not done / open, flagged as a real gap in the tooling, not just this
session's mistake**: `matrix_runner.py` has no guard against a second
invocation targeting the same `--output-root`/config concurrently -- a
lock file or PID-based guard in `run_config`'s `config_dir` would have
caught this immediately and cheaply, instead of relying on the operator
(here, this session) to correctly track whether a background process is
still alive. Worth adding before further item-4 scale-up sessions,
especially multi-session ones. Also worth internalizing procedurally: do
not run heavy concurrent foreground work (release builds, clippy, other
compute) while a memory-sized background benchmark run is in flight on
this host.

**Correction, same session, found immediately after writing the above**:
after relaunching the single clean run, `ps aux | grep <exact command
line>` reported it as gone within ~1-3 minutes -- which, applying this
entry's own "verify liveness" advice literally, looked like a *third*
crash with the identical signature (dies right after Spark's
`BlockManager` initializes, before touching the source file). Chased it
seriously: checked for JVM native-crash artifacts (`hs_err_pid*.log`,
`~/Library/Logs/DiagnosticReports`) -- none found; checked `log show` for
a jetsam kill targeting `java`/`SparkSubmit` specifically -- none found,
and system-wide free memory was a healthy 66% at the time, ruling out the
memory-pressure explanation that fit the real incident above. Before
concluding a second, different bug existed, checked with `lsof` on the
run's own `warmup.log` instead of `ps aux` -- and found both the
`python3` and `java` processes very much alive, holding the file open for
writing. **`ps aux` output from this sandboxed environment's Bash tool
gave a false negative for a real, still-running background process** --
twice in this session, including the one that triggered the original
duplicate-launch mistake in the first place. The lesson isn't "verify
liveness via `ps aux`" as originally written above; it's **prefer the
harness's own background-task completion notification, or `lsof` on a
file the process demonstrably still writes to, over `ps aux` snapshots
from a fresh tool invocation** -- and don't take further action (retrying,
killing, relaunching) on a `ps`-based "it's gone" read alone.

## Entry 10 -- Item 4: first real 10M-row rust+spark result (Task #6)

**First real axis-1 performance evidence Phase 0B exists to produce**, from
the clean relaunch in Entry 9 (rust config reused its 5 already-measured
runs from the notebook session; spark config ran fresh, 5/5 measured, no
failures, no retries needed):

| Engine | Peak RSS median | Peak RSS max | Wall median | Driver/pool bound |
| --- | --- | --- | --- | --- |
| Rust (DataFusion) | 5420 MB | 5688 MB | 91.5s | `--memory-limit-mb 2048` |
| Spark (Scala adapter) | 6351 MB | 6646 MB | 305.3s | `--driver-memory 6144m` |

**Cross-engine digest check: MATCH.** Both engines produced the identical
semantic digest (`71fbd214aafef8bfa43e1bd58a3b4b3ee6aa9a6db6e5259a22f1cf2d0bb1bf62`)
over the same 10M-row source -- the first correctness confirmation at
real benchmark scale, not just the 25k/1M fixtures. Aggregation
correctness (sum/count/KMV, XXH3 hashing, unsigned KMV ordering) holds at
10M rows on both engines simultaneously.

**Read with the schema-asymmetry caveat already recorded in
`SparkAggregate.scala`'s own `comparability_caveat` field**: Rust's wall
time includes writing all 4 candidate Parquet layouts (`aggregate_ms`
spans aggregate-and-write); Spark's wall time is aggregate-plus-digest
only, writing no layouts (out of scope for the adapter by design). The
3.3x wall-time gap and ~17% higher peak RSS are therefore **not** a clean
apples-to-apples "Spark is slower/heavier" reading yet -- item 6 needs to
either reconstruct a matched time slice (e.g. Rust's `aggregate_ms` minus
its own layout-write portion) or explicitly caveat the comparison as-is.
Recorded here as raw evidence, not yet a conclusion.

**No problems**: no RSS-limit kills, no retries, no disk issues (Spark's
own output is tiny -- ~120KB total for 6 `run.json` files, since it
writes no layouts; `/Volumes/YOTUO` at 715Gi avail, `/` untouched by this
config since `spark.local.dir` was redirected there).

Proceeding to 25M next (pre-authorized per the standing convention; the
gate is past 50M rows, not 10M/25M).

## Entry 11 -- Item 2 follow-up: real bug found running range-read against the
real 10M-row output, fixed and regression-tested (Task #8, continued)

While item 4's 25M matrix run was in the background (read-only, low-memory
work chosen deliberately per Entry 9's lesson), ran the just-shipped
`range-read` tool against the real 10M-row Rust run dir for the first time
(Entry 8 had only validated it against 25k/200k fixtures). Result: **all 8
non-aligned cases were silently skipped** -- `cases: 8, skipped: 8`.

**Root cause**: at 10M rows, per-bucket cell density (~90,102 cells/bucket,
`TIMESERIES_PHASE_0B_HARNESS.md` "Memory scaling") exceeds
`LAYOUT_ROW_GROUP_ROWS` (65,536), so a single bucket's cells now span
*many* row groups -- the opposite regime from the 200k/1M fixtures the
tool was validated against, where many buckets fit in *one* row group.
`pick_range_start`'s non-aligned candidate (`straddle_group.min_us +
HOUR_US`) was being rejected because that same bucket value is *also* the
minimum of the very next row group in this dense regime (the first
"clean" group for that bucket) -- the code checked "is this candidate a
row-group boundary anywhere in the file," which is the wrong question.
Alignment is a property of the *first row group a query actually touches*,
not global uniqueness. Fixing this exposed a second, symmetric bug in the
*aligned* branch via the corrected test: a candidate `row_groups[i].min_us`
isn't really aligned if the *preceding* group's own max also reaches that
same value (Parquet min/max are inclusive, so that earlier group still
has a matching row and a real reader has to scan it too).

**Fixed both** in `pick_range_start`: the non-aligned branch no longer
checks a file-wide boundary set (removed entirely); the aligned branch now
additionally requires `row_groups[i-1].max_us < row_groups[i].min_us` (a
genuinely clean cut, not just a coincidentally-matching value).

**Regression-tested properly, not just re-validated against real data**:
the existing integration test's own alignment assertion was checking the
same wrong invariant the code had (global "is `is_boundary`"), so it never
would have caught this -- self-consistency between the code and its own
test isn't correctness, same lesson issue #1 finding 2 already established
for this crate. Rewrote that assertion to check the real property (the
first row group whose range overlaps the case's start must have
`min_us == start` for aligned, `min_us < start` for non-aligned). Added a
focused new unit test,
`pick_range_start_finds_non_aligned_when_one_bucket_spans_many_row_groups`,
that constructs the dense regime directly (a hand-built `Vec<RowGroupBucketRange>`
with a straddle group) instead of needing a slow multi-million-row fixture
to reach it naturally -- fast, deterministic, and targets the exact bug.
`cargo test -p cubism-timeseries-bench`: 7 passed (was 6), 1 ignored.
`cargo clippy --all-targets --no-deps -- -D warnings`: clean. `rustfmt
--check`: clean. Re-ran `range-read` against the real 10M run dir:
16/16 cases, 0 skipped, numbers sane (e.g. `wide` short non-aligned scans
7 row groups / 458,752 rows for 360,659 matched, vs. aligned's 6 row
groups / 393,216 rows for 359,400 matched -- real, modest leading
overscan, not a degenerate 1:1). Also re-checked 1M and 200k fixtures
still produce 16/16 with no regressions.

**Lesson for next time this tool is extended**: validate against a run dir
at the actual target scale (or a synthetic fixture built directly in the
dense regime, as the new unit test now does) before trusting it, not just
against small smoke-scale fixtures -- the two regimes (many buckets per
row group vs. one bucket per many row groups) exercise genuinely different
code paths, and this crate's own 1M-vs-10M cell-density figures were
already the documented warning sign.

## Entries to follow

Each subsequent task (50M scale-up gate) gets its own entry here with a
before-snapshot, what was run, and an after-snapshot noting any problems
(spill triggered, RSS approaching threshold, load average from other
processes, disk fill on the output volume, etc.).
