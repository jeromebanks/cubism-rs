# Time-Series Phase 35 Handoff

Date: 2026-08-28

Branch: `feature/timeseries-correction-engine` (branched from
`feature/timeseries-phase-0a` at `cb552cd`, the PR #23 merge)

Status: **#9 and #16 are closed. #16 was the last big functional gap** —
a source-event change now flows end to end to a corrected, published,
queryable window without a human computing the rebuild. Two commits,
pushed, no PR opened yet (deliberately — see "First decision" below).

**Superseded by:** (none yet — this is the latest handoff)

## Read first

1. This file.
2. `docs/TIMESERIES_PHASE_34_HANDOFF.md` — the prior slice (the demo).
3. `crates/cubism-correct/src/windows.rs`'s module doc — it explains the
   one design decision in this branch that reverses an earlier deliberate
   deferral, and is the thing most worth a reviewer's disagreement.
4. `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`'s status snapshot (updated).

## First decision, before anything else: the PR base

`feature/timeseries-correction-engine` is **not yet in a PR**. It sits on
`feature/timeseries-phase-0a`, which is itself PR #24 into `main`.

- If **#24 merges first**, this branch rebases onto `main` trivially and
  its PR targets `main` — which matches the repo owner's stated preference
  that major features go in independent branches/PRs off `main`.
- If you open a PR **now**, it must target `feature/timeseries-phase-0a`,
  because `main` has none of the time-series work and the diff would be
  meaningless.

Do not guess. The owner said reviews happen in a fresh session with a full
Codex review; ask which base they want before opening anything.

## What this branch contains

### #9 — checksummed aggregate-state framing (commit `5239721`)

`FORMAT_V2` = `magic(3) | version(1) | payload | crc32(4, LE)` in
`crates/cubism-core/src/aggregate_state.rs`.

- **The CRC covers the header, not just the payload.** A corrupted version
  byte would otherwise be indistinguishable from a legitimate future
  version bump — the exact confusion the framing exists to prevent.
- **Why bother, given Parquet/Iceberg already checksum:** those protect
  data *at rest, once written*. A state blob is built in memory, merged
  across windows and revisions, and re-encoded on every correction. An
  in-memory bit flip lands in Parquet as a well-formed page holding a
  wrong `f64`. Aggregates merge, so one bad blob poisons every rollup
  above it, and a correction republishes the poison as authoritative.
- **Why now:** the release progression is at stage 2 of 7 ("experimental
  local tables"). After anyone's warehouse holds data this is a migration.
- **V1 stays readable** (unchecksummed — that is what V1 is; `decode`
  cannot invent an integrity check retroactively). Nothing writes V1.
  Verified end to end, not assumed: the pre-#9 demo warehouse served with
  the new binary returns `/G avg_revenue` on `2026-04-06` as
  `1.5319148936170213`, byte-identical to the pre-change run.
- Golden V2 vectors are cross-checked against Python's `zlib.crc32`, so
  they are pinned by an independent CRC implementation rather than by
  whatever this crate emits. The old V1 vectors are kept verbatim as the
  compat path's fixtures.
- **Deliberately untouched:** sketch blobs (`docs/sketches.md`) keep their
  own separate framing — #9 scopes to AVG/VAR/QNT. The Spark/Rust BLAKE3
  oracle is unaffected (it hashes semantic values, not this framing);
  this was checked, not assumed.

### #16 — `cubism-correct`, the engine link (commit `91ecb01`)

New crate depending on `cubism-core` + `cubism-datafusion` +
`cubism-iceberg`.

- **Why a new crate:** `cubism-iceberg` deliberately links no aggregation
  engine (to stay clear of the DataFusion seam #8 tracks) and
  `cubism-datafusion` carries no Iceberg dependency. Putting the link in
  either erases the property both were built to have. `cubism-serve` and
  `cubism-cli` already depend on both and resolve to the same arrow 58.3,
  so a crate above the pair is a proven shape, not a new risk.
- **`windows.rs` reverses an earlier deliberate deferral, and that is the
  part to review hardest.** `range_query.rs` records that inventing a
  canonical `bucket_start -> WindowId` encoding was *rejected* as out of
  scope; `series.rs` consequently makes the caller supply
  `(window_id, bucket_start)` pairs; Milestone 12a gave the convention to
  the demo. #16 cannot be done under that deferral — naming an affected
  window *is* the encoding question. So the encoding now lives here and is
  canonical. The day form is pinned to Milestone 12a's `YYYY-MM-DD` so
  pre-existing warehouses resolve unchanged, asserted by
  `day_form_matches_the_existing_demo_convention`.
- **`engine.rs`** reads the CAS anchor, rebuilds each affected window from
  corrected source, publishes via `CorrectionCoordinator`.
- **Rebuilds use the whole window range, never the caller's changed
  range** — aggregate states are not all subtractable (`CorrectionPlan`
  grants no additive shortcut today), so a correction is always a full
  rebuild. Rebuilding only the changed slice publishes a window whose
  aggregate covers less than the window it claims to be.
- **`tests/recompute_equality.rs` closes plan line 634** — the proof
  `cubism-iceberg/tests/coordinator.rs`'s own module doc says belongs "to
  whichever future crate links an aggregation engine and can build both
  sides independently". Neither side is hand-written; both run
  `build_temporal` over CSV and persist through Iceberg, differing in
  revisions published, write protocol, warehouse, and whether the window
  was derived or named.

## The semantic a reviewer must agree with, not discover

`CorrectionEngine::correct` corrects windows **one at a time, each with
its own Iceberg commit**. A mid-run failure leaves a corrected *prefix*.

This is deliberate — there is no cross-commit transaction in this stack,
and the alternative (an all-or-nothing protocol spanning several Iceberg
commits) does not exist to be used. `CorrectionOutcome` reports exactly
which windows landed so a retry resumes rather than restarts, and windows
are disjoint buckets of event time so a partial correction is a correct
correction of a prefix, never a torn one.

It is still a semantic someone should sign off on rather than find out
about later.

## How the tests were validated

Both new test files passed on the first run, which for a *new equality
test* is a reason for suspicion rather than confidence. They were
mutation-checked:

| Mutation | Expected to fail | Result |
|---|---|---|
| rebuild over the caller's `changed` range instead of the window's range | recompute-equality | fails — a whole XUnit vanishes, sums drop 230/4 → 160/2 |
| drop the half-open `end - 1` adjustment | boundary test | fails — an untouched neighbour is pulled in |

There is also a vacuity guard in the equality test asserting the late
events genuinely change the window, so the equality cannot pass by both
sides being equally wrong. No `MUTANT` markers remain (checked).

## Verification performed

```text
cargo test --workspace --exclude cubism-py   # 242 passed (229 baseline, +5 #9, +8 #16)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings   # clean
cargo test -p cubism-core                    # 88 passed (V1 compat, corruption, truncation)
cargo test -p cubism-correct                 # 6 unit + 2 integration
# V1 read-compat proven against the real pre-#9 demo warehouse over HTTP
```

## Still open before `main` — the pre-merge list

This was assessed in the previous session and **none of it is done**. In
priority order:

### Blocks the merge

1. **There is no CI. At all.** `.github/workflows/` does not exist; PR #23
   reported `checks: 0`. PR #24's "229 tests pass" is an unverifiable
   assertion from one laptop. The owner's plan is to add CI on a branch off
   `main` *after* #24 merges, because GHA is awkward to set up on
   non-default branches — that is a deliberate decision, not an oversight.
   Note that **#14's title ("zero CI coverage") presumes a CI that has
   never existed** — re-read #14 in that light before scoping it.
2. **#7, #13, #15 look done but are open.** The finish prompt's own open
   list omits all three; the plan's status table cites #13 as *evidence*
   that Phase 4 is Done; #15 asks for a roadmap that exists with 16
   milestones. Verify and close with landing comments, or the open-issue
   list means nothing to a reviewer.

### Cheaper now than after the merge

3. `crates/cubism-iceberg-spike` has no dependents but is a workspace
   member, so it builds/tests/clippies on every invocation — and would in
   CI. Delete or drop from default members.
4. `TIMESERIES_FINISH_PROMPT.md` at repo root is an AI session prompt,
   superseded by this handoff series. It should not be the second thing a
   visitor to `main` sees.
5. Two broken `](../../PLAN.md)` links in `docs/` — that file lives one
   level *above* the clone and is not tracked. (36 further mentions are
   prose, not links; this was measured, not estimated.) Vendor it or
   rewrite the two links.
6. **`--exclude cubism-py` is undocumented.** It compiles and passes under
   `-p cubism-py`, but that does not prove the exclusion is stale — pyo3
   commonly links differently under `--workspace`. Find out *why* before
   baking it into a CI workflow, or the bindings stay permanently
   unverified.
7. `spark-adapter/` (Scala, frozen, outside the workspace and any CI) —
   decide explicitly whether it ships on `main` as the correctness oracle
   or is archived.
8. 37 handoff docs sit flat in `docs/` alongside ~23 real ones; a
   `docs/handoffs/` subdirectory would help.

## Next feature work, ordered

Now that #16 is closed, the ordering from the previous assessment shifts:

1. **#8 — DataFusion `TableProvider`.** Unblocks the SQL half of Phase 5.
   DataFusion 55 shipped mid-August; the pin conflict may have resolved.
   One spike either lands it or writes down exactly which pins conflict.
2. **Phase 6 — rolling comparisons / trend inputs.** Has a design decision
   up front (amortized sliding-window monoid vs dyadic summaries vs a
   measured max window width for non-subtractable sketches). Never O(n·w)
   naive merging; never sum displayed distinct counts.
3. **Wire `cubism-correct` into the CLI.** The crate closes #16 but has no
   CLI entry point — a `cubism correct --spec --source --from --to`
   subcommand would make it reachable without writing Rust, and would give
   the demo a real correction instead of a hand-supplied `--revision 2`.
   This is the obvious follow-on and is *not* tracked by an issue yet.
4. Production substrate: **#10** (object store, compaction/expiry), **#12**
   (Postgres control store), **#11** (multi-step CLI).
5. Perf: **#22** with #1/#2/#4 — bench timing fix first, since
   write-inclusive timing contaminates everything measured through it.
   Nothing has been measured past demo scale; #1's 10–100M run has never
   happened and Phase 0B's correctness proof was at 25,000 rows.

## Worktree state

Committed and pushed to `feature/timeseries-correction-engine`:

- New: `crates/cubism-correct/` (Cargo.toml, src/lib.rs, src/windows.rs,
  src/engine.rs, tests/recompute_equality.rs),
  `docs/TIMESERIES_PHASE_35_HANDOFF.md` (this file).
- Modified: `crates/cubism-core/src/aggregate_state.rs`,
  `crates/cubism-core/Cargo.toml`, `Cargo.toml` (workspace member),
  `Cargo.lock`, `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`.

Deliberately uncommitted, standing convention: `.serena/`,
`examples/web_analytics_demo/events.csv`,
`.claude/skills/timeseries-slice/SKILL.md` pre-existing edits,
`docs/CODEX_TELEMETRY_ROADMAP.md`. `.temporal_build/` is gitignored.

## Primary files

- [`../../crates/cubism-correct/src/windows.rs`](../crates/cubism-correct/src/windows.rs)
  (the encoding decision — review this first)
- [`../../crates/cubism-correct/src/engine.rs`](../crates/cubism-correct/src/engine.rs)
  (orchestration, per-window commit semantics)
- [`../../crates/cubism-correct/tests/recompute_equality.rs`](../crates/cubism-correct/tests/recompute_equality.rs)
  (plan line 634)
- [`../../crates/cubism-core/src/aggregate_state.rs`](../crates/cubism-core/src/aggregate_state.rs)
  (V2 framing, V1 compat)
- [`TIMESERIES_PHASE_34_HANDOFF.md`](TIMESERIES_PHASE_34_HANDOFF.md)
  (prior handoff, superseded by this one)
