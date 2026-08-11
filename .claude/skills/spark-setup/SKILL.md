---
name: spark-setup
description: Install and verify Apache Spark (JVM + spark-submit + sbt, plus a matched pyspark for prototyping) on macOS via Homebrew, specifically to unblock cubism's Phase 0B time-series benchmark (docs/TIMESERIES_PHASE_0B_HARNESS.md remaining-work item 1 — "the local Spark adapter"). Use this whenever the user wants to install Spark on their Mac, get `spark-submit`/`sbt` working, or their harness's `cargo run -p cubism-timeseries-bench -- preflight` fails with "spark-submit: not found". This skill only bootstraps a working Spark+sbt install — it does not write the byte-compatible KMV Spark adapter itself (that's a Scala project at `spark-adapter/`, per ~/.claude/plans/flickering-popping-harp.md), and it does not touch DataFusion's own aggregate-spill question (issue #2), which is a separate, orthogonal investigation.
---

# Spark setup (macOS, for cubism Phase 0B)

Gets a real, working `spark-submit` + JVM + `sbt` installed on this Mac,
before anyone writes the Spark-side KMV adapter on top of it. The adapter
itself is implemented in **Scala** (chosen over PySpark specifically to
avoid Python overhead biasing the Rust-vs-Spark benchmark — the KMV merge
isn't a Spark SQL built-in, so a Python implementation would run through a
Python UDF/Arrow serialization path a Scala `Aggregator` wouldn't), so `sbt`
is the load-bearing build tool this skill installs; a matched `pyspark` venv
is also set up but is prototyping/exploration convenience only, not what the
adapter is written in. This is exactly the gate
`docs/TIMESERIES_PHASE_0B_HARNESS.md` describes as **"gated on user
go-ahead"** for remaining-work item 1 — a multi-GB, machine-state-changing
install (JVM + Spark distribution + sbt, likely several hundred MB to low
GB) — so don't run it without the user having actually asked for it in this
conversation.

## Before running anything

Tell the user plainly what's about to happen:
- Installs `openjdk@21` (whatever JDK `apache-spark`'s Homebrew formula
  currently depends on — the script re-checks this at run time rather than
  assuming), `apache-spark`, and `sbt` via Homebrew. All are keg-only /
  substantial downloads.
- Verifies `sbt` with a **real build** in a throwaway scratch project
  (`scalaVersion` pinned to whatever Scala version Spark's own jars are
  built against — currently 2.13.18 for Spark 4.2.0), not just `sbt
  --version` — this is the step that actually catches an sbt/Scala-version
  resolution problem before it blocks the real `spark-adapter/` build.
- Creates an isolated Python venv at `~/.cubism-spark-venv` with `pyspark`
  pinned to the exact installed Spark version (via `uv`), so the Python and
  JVM sides can't silently drift apart. This is for prototyping/data
  exploration only — the adapter itself is Scala, not Python.
- **Appends a marked block to the user's shell profile** (`~/.zshrc` or
  `~/.bash_profile`, detected from `$SHELL`) setting `JAVA_HOME` and `PATH` —
  needed because the Homebrew JDK formula is keg-only and won't otherwise be
  found by `java`/`spark-submit` in new shells. The block is wrapped in
  `# >>> cubism spark-setup >>>` / `# <<< cubism spark-setup <<<` markers so
  it's easy to find and remove later, and the script skips re-adding it if
  already present.
- Does **not** install Homebrew itself if missing (fails with a pointer to
  https://brew.sh instead) — that's a bigger, unscoped system change the
  user should run themselves.

## Running the bootstrap

```bash
bash .claude/skills/spark-setup/scripts/install_spark.sh
```

Run with the Bash tool so the user sees each step (brew installs can take a
few minutes; the JVM download in particular is large). The script is
idempotent — every step checks current state before acting, so re-running
after a partial failure just picks up where it left off.

What it does, in order:
1. macOS check, Homebrew presence check.
2. Reads `apache-spark`'s actual JDK dependency from `brew info --json=v2`
   (falls back to a hardcoded `openjdk@21` if that lookup fails, with a
   warning — that's the value confirmed correct as of this writing).
3. `brew install` the JDK, then `brew install apache-spark` (both no-ops if
   already current).
4. Resolves `JAVA_HOME` to the keg-only JDK's actual path and confirms
   `java -version` runs.
5. Confirms `spark-submit --version` on PATH and parses the version string.
6. **Runs a real job** (`run-example SparkPi 10`), not just a version
   banner — this is the step that actually catches a `JAVA_HOME` mismatch or
   broken install, which `--version` alone won't.
7. Installs `sbt` (brew, idempotent), then **verifies it with a real `sbt
   package`** in a throwaway scratch project pinned to Spark's own Scala
   version (currently `2.13.18`) — asserts a jar actually gets produced, not
   just that the command exits 0. Scratch project is deleted afterward.
8. Creates/updates `~/.cubism-spark-venv` via `uv venv` + `uv pip install`,
   pinned to the exact Spark version found in step 5, then starts a real
   local `SparkSession` and runs `.count()` on a small range as an
   end-to-end Python-side check. (Prototyping convenience only — see above.)
9. Appends the marked `JAVA_HOME`/`PATH` block to the shell profile (skipped
   if already present).
10. Re-runs the project's own gate —
    `cargo run -p cubism-timeseries-bench -- preflight` — so the result is
    verified against the actual check the harness uses, not just this
    script's own opinion of success.

## Reading the result

- **All steps pass, ending in "harness preflight now passes"** — Spark and
  sbt are ready. Tell the user their next *interactive* shell needs `source
  ~/.zshrc` (or a new terminal) to pick up `JAVA_HOME`/`PATH`; the script
  itself already exported them for its own session, which is why the
  preflight re-run in step 10 works without that.

  **This does not carry into Claude Code's own Bash tool.** Verified
  empirically (2026-08-11): a fresh Bash tool call after this script
  finishes shows `spark-submit` reachable (it happens to sit on a different,
  already-default PATH entry — `/opt/homebrew/bin`) but `JAVA_HOME` comes
  back empty. Claude Code's Bash tool spawns a non-interactive shell per
  call and does not source `~/.zshrc` (zsh only auto-sources it for
  interactive shells), so the marked block this script appends never takes
  effect inside a Claude Code session — restarting or resuming the
  conversation does not fix this either, since it's a per-command shell
  property, not conversation state. Any later Bash command in a Claude Code
  session that needs `JAVA_HOME` (building/running the Spark KMV adapter,
  `spark-submit` on a job that shells out to `java` directly, etc.) must
  export it explicitly in that command, e.g.:
  `export JAVA_HOME="$(brew --prefix openjdk@21)/libexec/openjdk.jdk/Contents/Home"`.

- **Fails at "Homebrew not found"** — point the user to https://brew.sh and
  stop; don't attempt a workaround.

- **`SparkPi` job fails but `spark-submit --version` succeeded** — almost
  always `JAVA_HOME` pointing at the wrong JVM (e.g. a stray system Java).
  The script's own step 4 sets `JAVA_HOME` explicitly for its run, so a
  failure here usually means Spark's formula bumped its required JDK version
  since this script was written — check `brew info apache-spark`'s
  `Dependencies` line against `JDK_FORMULA` resolution in step 2's output.

- **`sbt package` fails in the scratch project** — this is the load-bearing
  failure mode, since the real adapter build depends on it. The script
  prints the last 40 lines of the sbt log and the full log path. Most likely
  cause: the globally-installed `sbt` launcher can't resolve/bootstrap the
  pinned `scalaVersion` (2.13.18 as of this writing) cleanly — check the log
  for a Coursier/Ivy resolution error specifically. If so, the fix belongs
  in `spark-adapter/`'s own `project/build.properties` (pin a different sbt
  launcher version there), not in this global install — sbt's own launcher
  version is decoupled from any given project's `scalaVersion`.

- **pyspark venv step warns "uv not found"** — the JVM/`spark-submit`/`sbt`
  side is still fully installed and usable; only the Python convenience
  layer is skipped. Either install `uv` (https://docs.astral.sh/uv/) and
  rerun, or tell the user `pip install pyspark==<version>` manually if
  they'd rather not use `uv`.

- **Step 10 warns it can't find the Cargo workspace** — the script locates
  the repo root relative to its own path
  (`.claude/skills/spark-setup/scripts/../../../..`); if this skill was
  copied somewhere else, run
  `cargo run -p cubism-timeseries-bench -- preflight` manually from the
  `cubism` repo root instead.

## What this does *not* do — read before assuming more happened

- **Does not implement the Spark-side KMV adapter.** Getting `spark-submit`
  and `sbt` working is a precondition for remaining-work item 1
  (`docs/TIMESERIES_PHASE_0B_HARNESS.md`), not the item itself. The adapter
  is a Scala sbt project at `spark-adapter/` (design in
  `~/.claude/plans/flickering-popping-harp.md`) that reimplements the exact
  xxh3-64/seed-0 KMV byte format from `cubism-core::sketch::kmv` — Spark's
  own built-in approximate-distinct (HyperLogLog) is explicitly **not** an
  acceptable substitute, per that doc. Scala was chosen deliberately over
  PySpark for this specific piece so the custom KMV merge stays on the JVM
  rather than paying Python UDF overhead that would bias the benchmark.
- **Does not tell you anything about issue #2** (the open question of
  whether DataFusion's custom sketch `GroupsAccumulator`s actually spill
  under memory pressure, vs. just the sort operator). That's a
  DataFusion-internal question a Spark install can't answer either way —
  don't conflate "Spark is now installed" with "the memory-chokepoint issue
  is understood." A Spark run can at best serve as a *reference point* for
  whether the workload is boundable in principle, once it exists.
- **Does not run the actual 10-100M row Rust-vs-Spark benchmark** (harness
  item 4, still separately gated past 50M rows) — this only proves Spark
  itself works on this machine.
