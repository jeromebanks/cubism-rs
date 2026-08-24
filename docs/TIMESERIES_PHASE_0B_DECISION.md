# Time-Series Phase 0B Decision: Rust/DataFusion selected; Spark path closed

Date: 2026-08-24

Branch: `feature/timeseries-phase-0a`

Status: **Final.** This closes Phase 0B. The engine question this phase
existed to answer is answered; no further session should reopen it without
new information this document does not already address.

Supersedes: `docs/TIMESERIES_PHASE_0B_RESULTS.md`'s "Decision" section
(2026-08-12), which selected DataFusion/Rust for the **axis-1 (single-node)
benchmark path** while leaving six items explicitly open ("Remaining gates
before Phase 2/3"). Those items are dispositioned below. This document does
not retract any measurement or caveat in RESULTS/HARNESS/NOTEBOOK — those
stand as written.

## The decision

**Rust/DataFusion is the aggregation, storage, and serving engine for
cubism's time-series implementation. The Spark comparison path is closed:
no further Spark-side work is planned or required** — no `run_02` root-
cause investigation (#5), no DataFrame-native adapter rewrite (RESULTS
"Remaining gates" item 5), no Spark runs at 50–100M rows (item 1), and no
Spark-side dense-occupancy or compression measurements.

The final call is the maintainer's (2026-08-24), made with full knowledge
of the caveats below. Three things support it:

1. **The measured record already favored Rust decisively** on every axis
   Phase 0B actually tested (`docs/TIMESERIES_PHASE_0B_RESULTS.md`
   "Decision"): byte-identical semantic output across both engines at
   every scale (so correctness was never the discriminator), ≥~3x faster
   raw totals and ~6x on the one fair-slice measurement trusted without
   caveat, lower peak memory, and a 100% first-attempt success rate across
   every run ever executed versus an unexplained ~50% failure rate for
   Spark's own long-running jobs at 25M rows.
2. **The original purpose of the comparison has been overtaken by events.**
   Phase 0B was framed as a gate *before* committing to building on Rust.
   The Rust engine has since been built through Phases 1–4 plus Phase 5's
   functional half (temporal semantics, sparse bucketed aggregation,
   Iceberg persistence, corrections/concurrency, exactness-aware serving).
   Spark's remaining possible roles — the write-path/maintenance fallback
   per FEASIBILITY's conditional recommendation, and an interop/correctness
   oracle (the role its digest matches actually played, see
   `docs/TIMESERIES_PHASE_0B_HARNESS.md` and #1's finding-3 history) — do
   not justify finishing an adapter whose 25M-class runs were killed
   intermittently by an unexplained external signal on the only machine
   available to test it. A fallback nobody can reliably exercise is not a
   fallback.
3. **The unresolved items split cleanly by this decision.** Everything
   still open is either (a) specific to the now-closed Spark comparison —
   `run_02`'s root cause, a DataFrame-native rewrite, Spark-side runs at
   higher scale — or (b) a Rust-engine measurement task that belongs to
   performance hardening, not engine selection (dense occupancy,
   compression, memory ceilings). The maintainer's supporting judgment:
   the kill signal appeared only in 25M-class jobs (20+ minutes *if
   completed*, though kills were observed both early and mid-run) on a
   contended 16 GB Mac mini with an external USB disk and multiple
   concurrent sessions (NOTEBOOK Entry 12; memory pressure, disk space,
   subprocess wrapping, and foreground load were all checked and ruled
   out, but a dedicated-host reproduction was never possible, and the
   root cause remains unestablished). The confounded 25M fair-slice
   figure was already retracted by RESULTS itself. Nothing unresolved
   suggests a defect in the Rust engine.

## What remains true (unchanged scope honesty)

This decision covers engine selection only. It still says nothing about:

- **Distributed execution** (RESULTS' axes 2–3): no cluster has ever been
  benchmarked by any session. Rust's own scatter/gather design is future
  work; nothing here validates or invalidates it. Scaling claims beyond
  one node remain unproven, as they have always been.
- **Production-scale measurements of the Rust engine**: see the carried
  backlog below. "Rust wins" at 1–25M sparse rows on one contended Mac
  mini does not extrapolate on its own to 100M+ dense rows in production.

## Disposition of RESULTS' six remaining gates

| Gate item | Disposition |
|---|---|
| 1. 50–100M rows, both engines | **Descoped for Spark** (path closed); the Rust side becomes part of Phase 7's performance hardening, tracked under the Phase 7 umbrella issue [#22](https://github.com/jeromebanks/cubism-rs/issues/22) and #1 |
| 2. Spark `run_02` root cause | **Closed with #5** (won't-fix: the harness under investigation is descoped) |
| 3. Dense occupancy at scale | **Carried to Phase 7** (Rust-only now), tracked under [#22](https://github.com/jeromebanks/cubism-rs/issues/22) (absorbs #6's finding 1) |
| 4. Compression codecs | **Carried to Phase 7** (the 21–30x UNCOMPRESSED expansion ratio makes this genuinely important), tracked under [#22](https://github.com/jeromebanks/cubism-rs/issues/22) (absorbs #6's finding 2) |
| 5. DataFrame-native Spark rewrite | **Closed with #6** (won't-fix: mooted by the engine decision) |
| 6. Axis 2 / axis 3 distributed comparisons | **Unchanged**: out of scope for Phase 0B as always; any future distributed work needs its own benchmark design regardless of engine |
| (#6 finding 4) Harness times aggregation inclusively of writes, confounding the 25M fair-slice subtraction | **Carried to Phase 7**: not mooted by closing Spark — it shaped RESULTS' analysis of *Rust's own* scaling shape and will contaminate any Phase 7 measurement run through the bench until fixed ("time aggregation independently of writes") — tracked under [#22](https://github.com/jeromebanks/cubism-rs/issues/22) scope item 3 |

FEASIBILITY's gate 9 ("a credible Rust hot path, or identify the workload
boundary at which Spark should take over") resolves via its first
disjunct — byte-identical outputs plus the measured 3–6x margins are a
credible local-scale hot path — with the explicit limit that this says
nothing about production scale (see "What remains true" above).

Also retargeted to Phase 7 by this decision (kept open, re-scoped):
**#1**, **#2**, **#4**. These are Rust-engine measurement/performance
questions and were never actually about Spark. Note their live state, so
Phase 7 doesn't inherit stale premises: **#1**'s original headline (a
20–30 GB sparse ceiling) was already fixed by its streaming rewrite —
measured at 1.09 GB / 3.42 GB peak for 1M/10M rows — leaving the actual
10–100M-row measurement and independent verification of the content-ID
path as its remainder; **#2**'s aggregate-spill question remains wholly
unverified; **#4**'s unbounded `.cache()` remains unfixed.

## Fate of the artifacts

- [`spark-adapter/`](../spark-adapter/) stays in-tree, frozen and
  unmaintained, as the historical record of what the comparison measured.
  Its `.gitignore`d `target/` is disposable; its sources are cited by
  RESULTS/NOTEBOOK and must not rot silently — if it ever breaks the build
  (it cannot today: it is Scala/sbt, outside every Cargo workspace), it
  gets deleted by an explicit decision, not neglect. **Acknowledged cost
  of freezing it:** the Spark digest matches were this project's only
  independent cross-engine oracle — nothing else in the repo re-derives
  sum/count/KMV values from a second implementation — and that oracle has
  no future. Its final confirmations (byte-identical digests at
  25k/1M/10M/25M) should be pinned as permanent golden fixtures (tracked
  under [#22](https://github.com/jeromebanks/cubism-rs/issues/22)) so the
  correctness anchor survives the
  adapter.
- [`crates/cubism-timeseries-bench`](../crates/cubism-timeseries-bench)
  stays active: it is the local Rust measurement harness, and Phase 7's
  backlog above will be executed through it.
- The Spark setup skill (`.claude/skills/spark-setup/SKILL.md`) is now
  historical; harmless to keep, needed only if someone re-runs the frozen
  adapter.

## Where this leaves the plan

Three edits to `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`, all made in the
commit landing this document:

1. Phase 0B's section status line ("measured Rust/Spark decision
   pending") replaced with a pointer here;
2. the status snapshot table's Phase 0B row rewritten — including
   dropping its incorrect "the Spark adapter itself is unbuilt" fragment,
   which contradicted RESULTS/HARNESS/NOTEBOOK (the adapter ran at four
   scales, including twice at 25M);
3. Phase 7's unresolved-decision bullet "retirement point for the Spark
   maintenance fallback" marked as resolved by reference to this
   document.

On FEASIBILITY's conditional recommendation: its **primary** branch reads
"proceed with Rust/DataFusion for the core and query paths **and keep
Spark as an optional maintenance boundary**." This decision takes the Rust
half of that branch **and additionally retires its optional-maintenance-
boundary clause** — full closure is stricter than the primary branch, not
identical to it. The **fallback** branch ("retain Spark for the Iceberg
write/maintenance path") is retired too; reviving either Spark role after
this would be a new decision, documented the same way this one is, and
would need new evidence — not merely a change of mind.
