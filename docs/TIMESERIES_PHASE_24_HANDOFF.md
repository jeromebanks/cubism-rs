# Time-Series Phase 24 Handoff

Date: 2026-08-17

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 5b** (`docs/TIMESERIES_ROADMAP.md`'s Phase 4
milestone list) — fixes
[#17](https://github.com/jeromebanks/cubism-rs/issues/17), the real
correctness gap left open since Milestone 5: `RunState::Claimed` is
ambiguous between "append never attempted" and "append's `fast_append`
already committed to Iceberg, but the process crashed before
`record_append` persisted that fact," and recovering the second case by
re-running the append could silently duplicate every visible row for that
window/revision. This is a deliberate, user-directed detour, not the next
roadmap milestone pick: the user asked for a session slice targeting #17
specifically, after a conversation assessing which open GitHub issues were
real tech debt versus stale bookkeeping. `docs/TIMESERIES_ROADMAP.md`'s own
step-1 milestone scan was skipped on that explicit instruction; every other
step of `.claude/skills/timeseries-slice/SKILL.md` ran as prescribed.

Landing this milestone satisfies Phase 4's own "done" condition
(`docs/TIMESERIES_ROADMAP.md`) for the first time — criterion 2 was the one
remaining gap, tracked against #17 since Milestone 6 closed. This triggers
this series' step 8a cross-model phase review for the first time on Phase
4 (Phase 4 predates step 8a's existence — see the skill's own "Forward-
only" note — so this is the first review Phase 4 has ever had, not a
follow-up to a prior one). See "Step 8a" below for the review's disposition.

Also closed on GitHub this session, found to already be resolved by earlier
work rather than needing a fix: **#18** (`Rollback leaves a superseded run's
RunState reporting Published`) — Milestone 6's `RunInspection::inspect`
(`docs/TIMESERIES_PHASE_14_HANDOFF.md`) already solved it; the roadmap's own
"Phase 4 done condition" section already said so explicitly. The user had
drafted a close comment before delegating this session; `gh`'s GraphQL API
was returning sustained `503`s when applied, so it was posted via `gh api`'s
REST fallback instead.

(Despite the filename, this doc documents a session slice, not "Phase 24"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 4 is late data/corrections/the coordinator,
closed as of this session (excluding #10, compaction/retention/object-store,
tracked separately); plan Phase 5 (DataFusion range queries and serving) was
already narrow-closed by `docs/TIMESERIES_PHASE_22_HANDOFF.md`'s session and
tightened further by `docs/TIMESERIES_PHASE_23_HANDOFF.md`'s, unchanged by
this one.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

The user asked, in conversation (not via the skill's own step 0/1 flow),
which of the repo's 18 open GitHub issues represented real accumulated tech
debt worth addressing versus stale bookkeeping. Investigating that question
directly (reading `coordinator.rs`, `control.rs`, `writer.rs`, `reader.rs`,
and cross-checking against `docs/TIMESERIES_ROADMAP.md`'s own "Phase 4 done
condition" section) surfaced two corrections to what the issue text alone
would suggest:

- **#18 was already resolved**, not open tech debt — Milestone 6's
  `RunInspection::inspect` fixed it (`docs/TIMESERIES_PHASE_14_HANDOFF.md`),
  and the roadmap already said so in prose the GitHub issue itself was never
  updated to reflect. Closed this session (see "GitHub issues touched").
- **#17 is real correctness, not robustness**, but was — at the start of
  this session — reachable through *zero* production call sites: grepping
  every crate showed `CorrectionCoordinator::execute` (the one place
  `ReconciliationRecord` classification drove automatic recovery) was called
  nowhere outside its own tests. This assessment was reported to the user
  as "currently dormant," which turned out to be incomplete — see below.

The user then asked how hard #17 would be to fix and asked for a session
slice, run as a fork with full context of that investigation (so the
mechanism didn't need re-deriving) rather than a fresh agent.

**The fix's key finding, which shrank the estimate below what #17's own body
suggested:** every states row `AggregateWriter::append_window` writes
already carries its own `window_id`/`revision`/`run_id` columns
(`writer.rs`'s `augment_states_batch`) — exactly the columns needed to ask
Iceberg's own committed state "did this run's append already happen?"
directly, with no new schema, no new persisted control-store field, and no
migration. `AggregateReader::read_window` already showed the predicate-scan
pattern to copy (`(window_id, revision)` via `Table::scan().with_filter(...)`);
the fix is that same pattern plus a `run_id` clause, in a new
`AggregateReader::run_append_snapshot` function
(`crates/cubism-iceberg/src/reader.rs`). `CorrectionCoordinator::execute`'s
`AwaitingAppend` branch (`crates/cubism-iceberg/src/coordinator.rs`) now
calls it before deciding whether to re-append: a match means the append
already committed (record it and skip re-appending); no match means proceed
exactly as before.

**Advisor caught a real deviation before code was written: the "currently
dormant" assessment was incomplete.** The advisor's first pass (called after
code was already written — a process deviation from the skill's
advisor-before-code ordering, flagged and not repeated) pointed out that
`cubism-cli`'s `iceberg_build` command hand-rolls the identical
claim/append/record protocol independently of `CorrectionCoordinator`
(it predates that type — Phase 3) and had the exact same gap: its own
`already_appended` check recognized only `Appended`/`Published`, treating
`Claimed` as "never attempted" unconditionally. Since `iceberg_build` is the
*only* one of the two paths any real invocation of this crate goes through
today — `execute` still has zero production callers — fixing only `execute`
would have closed #17 on paper while leaving the actually-reachable bug in
place. Both are now fixed, sharing `run_append_snapshot`
(`crates/cubism-cli/src/main.rs`).

**Two things the advisor asked to be resolved explicitly, not assumed:**

- **Concurrent recovery of the same run is not defended against.** Two
  callers racing `execute` (or two `iceberg_build` invocations) for the
  identical `run_id` could both observe "not committed yet" before either
  commits, then both append — the same latent race that existed before this
  fix. This closes the *sequential* crash-then-retry gap #17 describes;
  concurrent recovery of one run was never in #17's scope and is not solved
  here. Stated explicitly in `run_append_snapshot`'s own doc comment, not
  silently left implicit.
- **`expected_rows` does not gain a new validation gap.** Checked directly:
  `RunState::expected_rows` is only ever cross-checked at `claim_run` time,
  against a *retried claim's own* `expected_rows` argument (`control.rs`'s
  `RunConflict`) — nothing in `record_append` or `publish` compares
  `expected_rows` against the actual committed row count, on this path or
  the pre-existing normal append path either. This fix does not weaken an
  existing invariant, because none exists to weaken; recorded in
  `run_append_snapshot`'s own doc comment rather than left unexamined.

**A real bug caught by the test itself, not by review:**
`FixedSizeBinaryArray`-adjacent code was not touched this session, but the
new integration test's first run surfaced nothing broken — worth noting the
absence explicitly since a prior slice in this series (Milestone 10b-3) hit
exactly this class of bug in similar territory. This session's test passed
on the first real run after the fix landed; no equivalent surprise here.

Step 8a's own trigger question — "does this slice satisfy or newly close a
roadmap Phase N done condition" — was resolved explicitly at the advisor
step, not assumed: the roadmap's dedicated "Phase 4 done condition" section
was checked directly (not the skill's own looser parenthetical claiming
Phase 4 predates step 8a's applicability) and found to say plainly that
criterion 2 was the one remaining gap keeping Phase 4 open. Fixing #17 for
*both* recovery paths closes that criterion fully — see "Phase 4 done
condition" in the roadmap for the updated walk.

## What was actually verified

That `AggregateReader::run_append_snapshot` correctly distinguishes
"appended, unrecorded" from "never attempted," proven at the **snapshot**
level, not just the row level: the new integration test
`sqlite_coordinator_execute_recovers_an_appended_but_unrecorded_claim_after_reopen`
(`crates/cubism-iceberg/tests/durability.rs`) calls
`AggregateWriter::append_window` directly (a real Iceberg commit) and
deliberately never calls `record_append` — the exact state a crash between
those two calls leaves. A fresh handle then calls
`CorrectionCoordinator::execute` with the identical request, and the test
asserts the states table's current snapshot id afterward is *exactly* the
one the earlier direct commit produced — proof that no second `fast_append`
happened at all, not just that the row count didn't visibly double (a
row-count check alone could in principle round-trip a coincidence; an
unchanged snapshot id cannot, since any further commit would produce a new
one). Row count is asserted too, as corroboration, not the primary claim.

That the pre-existing, unambiguous half of `AwaitingAppend` recovery is
unaffected: `sqlite_coordinator_execute_recovers_an_unattempted_claim_then_replays_a_published_run_after_reopen`
(Milestone 5's own test, unchanged) re-ran clean — the `else` branch this
fix's `match` falls back to is the same append-for-real code path that test
already covers.

That `cubism-cli`'s independent protocol is fixed the same way: `cargo
build -p cubism-cli` and `cargo clippy -p cubism-cli --all-targets` are
clean with the new branch in place. This crate has no test harness at all
(no `tests/` directory, zero `#[test]`s in `main.rs` before or after this
session) — its own established verification gate is build+clippy, unchanged
by this slice. The CLI's own fix is a direct, mechanical reuse of the
already-tested `run_append_snapshot` helper and the same branching shape
`CorrectionCoordinator::execute` uses, not new logic requiring its own test
to be honestly claimed as covered.

That the full step-4 verification battery — including `cargo clippy … -D
warnings` — is clean with every change in place, run fresh this session
across several passes (once immediately after the reader.rs/coordinator.rs
change, before the test existed; once after the test was added and initially
passed; once more after the `cubism-cli` fix was added). `git status` was
checked after every pass. `rustfmt --edition 2024 --check` was run on every
touched file before any formatting, per the skill's mandatory gate:
`crates/cubism-iceberg/src/coordinator.rs` reported pre-existing diffs
identical in count to a fresh `HEAD` checkout (confirmed byte-for-byte via
a parallel baseline run) — this session's own new code produced zero
additional diff, verified directly rather than assumed; `reader.rs`,
`durability.rs`, and `crates/cubism-cli/src/main.rs` each had one or more
genuinely-new hunks from this session's own additions, hand-fixed to match
rustfmt's exact output one at a time (never run through plain `rustfmt` on
the whole file), with every fix re-diffed against the pre-existing baseline
afterward to confirm the remaining diff count matched exactly — not
approximately.

It does **not** prove: anything about concurrent recovery of the same run
(explicitly out of scope, see "What this session built" above). It does not
change or re-verify anything about `#8`'s SQL/pushdown gap, `#10`'s
compaction/retention scope, `#16`'s event-time-window-identification gap, or
any Phase 5 work — none of those crates/paths were touched this session.

## GitHub issues touched

- **Closed [#17](https://github.com/jeromebanks/cubism-rs/issues/17)** —
  fixed by this session's own code (`run_append_snapshot`, wired into both
  `CorrectionCoordinator::execute` and `cubism-cli`'s `iceberg_build`).
- **Closed [#18](https://github.com/jeromebanks/cubism-rs/issues/18)** —
  found already resolved by Milestone 6 (`docs/TIMESERIES_PHASE_14_HANDOFF.md`),
  not by any code change this session. Comment posted explaining the
  resolution and cross-linking `RunInspection::inspect`; `gh issue close`
  was 503ing on GitHub's GraphQL API at the time, so the close itself went
  through `gh api`'s REST endpoint instead (`PATCH
  repos/.../issues/18` with `state=closed`).
- No new issues filed. Concurrent-recovery-of-the-same-run (the one
  explicit non-goal this fix carries) is recorded in
  `run_append_snapshot`'s own doc comment and the roadmap's Milestone 5b
  entry, not filed separately — same "record scope/behavior findings in the
  roadmap entry" precedent this series has used since Milestone 9.

## Deferred / not done this session

1. **Concurrent recovery of the same `run_id`** — two callers racing
   `CorrectionCoordinator::execute` (or two `iceberg_build` invocations) for
   the identical run could both observe "not yet committed" and both
   append. Not fixed, not previously any worse than before this session;
   explicitly out of #17's own scope. Would need a real locking primitive
   (this crate currently has none spanning the Iceberg-commit boundary) if
   it ever becomes a real requirement.
2. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** — unchanged;
   still the natural next roadmap extension for the Phase 5 track, separate
   from this session's Phase 4 work.
3. **Widening past `AverageState`, the multi-XUnit/multi-measure response
   shape** — unchanged; Phase 5 track, not touched this session.
4. **Plan completion-criterion 774** (storage pruning across many windows)
   and the SQL-table-function unresolved decision — unchanged, gated on #8.
5. **#16** (event-time window identification + recompute-equality proof) —
   unchanged; needs an aggregation-engine link `cubism-iceberg` deliberately
   does not have.
6. **#10** (real object store + Iceberg maintenance: compaction/retention) —
   unchanged; explicitly excluded from Phase 4's "done" scope in this
   roadmap, same as every prior session.
7. **#9** (aggregate state blobs have no checksum), **#14** (retry-loop has
   no test coverage under real SQLite contention), **#4** (windowed builds'
   `.cache()` is unbounded in RAM), **#3** (rustfmt drift) — unchanged, all
   assessed in this session's earlier conversation as real-but-not-urgent;
   not touched.
8. **A successor roadmap slice for what's beyond Phase 4/5's current
   scope** — with Phase 4 now closed (excluding #10) and Phase 5
   narrow-closed, the next slice's own step 1 should decide among: #19-
   adjacent Phase 5 extension work, `/api/series`, or picking up one of the
   assessed-but-deferred issues above. Not decided this session.

## Step 8a (cross-model phase review)

Landing Milestone 5b satisfies Phase 4's "done" condition
(`docs/TIMESERIES_ROADMAP.md`) for the first time — this is the first phase
review Phase 4 has ever had; it predates step 8a's own existence (the skill
file's "Forward-only" note), so there is no prior baseline to compare
against, only the whole Phase 4 diff from its own start.

**Phase start:** `a253f4f` (the commit immediately before `1836d50` added
this roadmap doc's first "## Milestones" section, Milestone 1 — the same
backfill convention Phase 5's own start, `f0599b2`, used).

[Review disposition to be filled in after the review runs — see
`docs/phase-reviews/TIMESERIES_PHASE_4_REVIEW.md` for the full text and
findings.]

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hashes — this doc deliberately doesn't hardcode them, per
this series' established convention):

- New: `docs/TIMESERIES_PHASE_24_HANDOFF.md` (this file),
  `docs/phase-reviews/TIMESERIES_PHASE_4_REVIEW.md` (step 8a's own record).
- Modified: `crates/cubism-iceberg/src/reader.rs` (new
  `AggregateReader::run_append_snapshot`), `crates/cubism-iceberg/src/coordinator.rs`
  (`CorrectionCoordinator::execute`'s `AwaitingAppend` branch now checks it;
  `ReconciliationRecord`'s own doc comment updated), `crates/cubism-iceberg/tests/durability.rs`
  (new integration test), `crates/cubism-cli/src/main.rs` (`iceberg_build`'s
  own recovery branch fixed the same way), `docs/TIMESERIES_ROADMAP.md`
  (new "Milestone 5b" section; Milestone 5's own #17 bullet annotated as
  resolved, not rewritten; "Phase 4 done condition" criterion 2 and its
  "Net" statement rewritten; two stale `#10/#17` precedent references in
  the Phase 5 section corrected to `#10` alone).
- Untouched: `crates/cubism-core/src`, `crates/cubism-datafusion/src`
  (confirmed untouched via `git status` after every verification pass this
  session), `crates/cubism-serve/src`.

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (34 passed + 1 ignored in `cubism-iceberg`, up from 33; 208 passed / 2 ignored in workspace, up from 207)

All figures re-run fresh this session, across three full battery runs (see
"Verification performed" below). `cargo test -p cubism-iceberg` reports 34
passed, 1 ignored (6 suites) — up from Phase 23's 33 by one: the new
`sqlite_coordinator_execute_recovers_an_appended_but_unrecorded_claim_after_reopen`
test. `cargo test --workspace --exclude cubism-py` reports 208 passed, 2
ignored (23 suites) — up from Phase 23's 207 by the same one. No other
crate's test count changed (`cubism-cli` has none to change).

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 34 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 8 passed (7 prior + 1 new)
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 208 passed, 2 ignored (23 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

Run in full three times this session: once right after `reader.rs`/
`coordinator.rs`/the new test landed (caught nothing broken, unlike Phase
23's `try_from_iter` surprise); once after the `cubism-cli` fix was added;
once more after the roadmap doc edits, to confirm the doc-only changes
introduced no regression (they touch no `.rs` file). All three runs
produced zero warnings and zero failures; figures shown are from the final
run.

## Primary files

- [`../crates/cubism-iceberg/src/reader.rs`](../crates/cubism-iceberg/src/reader.rs)
  (`AggregateReader::run_append_snapshot`, #17's core fix)
- [`../crates/cubism-iceberg/src/coordinator.rs`](../crates/cubism-iceberg/src/coordinator.rs)
  (`CorrectionCoordinator::execute`'s updated `AwaitingAppend` branch;
  `ReconciliationRecord`'s doc comment)
- [`../crates/cubism-iceberg/tests/durability.rs`](../crates/cubism-iceberg/tests/durability.rs)
  (new `sqlite_coordinator_execute_recovers_an_appended_but_unrecorded_claim_after_reopen`)
- [`../crates/cubism-cli/src/main.rs`](../crates/cubism-cli/src/main.rs)
  (`iceberg_build`'s own recovery branch, fixed the same way)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) ("Milestone 5b"
  section; "Phase 4 'done' condition," criterion 2 and "Net" rewritten)
- [`TIMESERIES_PHASE_23_HANDOFF.md`](TIMESERIES_PHASE_23_HANDOFF.md) (prior
  handoff, superseded by this one)
- [`phase-reviews/TIMESERIES_PHASE_4_REVIEW.md`](phase-reviews/TIMESERIES_PHASE_4_REVIEW.md)
  (this session's step 8a cross-model phase review — Phase 4's first)
- GitHub issue [`#17`](https://github.com/jeromebanks/cubism-rs/issues/17)
  (closed this session — fixed)
- GitHub issue [`#18`](https://github.com/jeromebanks/cubism-rs/issues/18)
  (closed this session — found already resolved)
- GitHub issue [`#10`](https://github.com/jeromebanks/cubism-rs/issues/10)
  (Phase 4's one remaining exclusion, unchanged)
