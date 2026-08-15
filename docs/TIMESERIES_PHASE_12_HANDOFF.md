# Time-Series Phase 12 Handoff

Date: 2026-08-14

Branch: `feature/timeseries-phase-0a`

Status: **Milestone 5 (`ReconciliationRecord` + failure-injection
recoverability) landed, narrowed in scope.** New `ReconciliationRecord`
type in `crates/cubism-iceberg/src/coordinator.rs`, wired into
`CorrectionCoordinator::execute`; one new integration test in
`crates/cubism-iceberg/tests/durability.rs` proving recovery across a real
process restart for two of the four interruption points a correction can
crash at; one new unit test in `coordinator.rs` covering
`ReconciliationRecord::classify`'s four branches directly. Full step-4
verification battery clean. `docs/TIMESERIES_ROADMAP.md` updated: Milestone
5 marked `Done (narrowed)`, its "What it does"/"Test"/"Done when" sections
narrowed in place with the original wording preserved in a
`(Corrected: ...)` parenthetical (Milestone 3/4's own convention). The one
interruption point this milestone does not safely recover — a crash between
the Iceberg append commit and `record_append` — is filed as
[#17](https://github.com/jeromebanks/cubism-rs/issues/17), not silently
assumed away. Milestones 1-4 (already done) untouched; Milestone 6
untouched.

(Despite the filename, this doc documents a session slice, not "Phase 12"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving;
plan Phase 4 is now five milestones into its six-milestone roadmap. See
`docs/TIMESERIES_ROADMAP.md` for what "Milestone 5" means relative to plan
phases.)

**Superseded by:** `docs/TIMESERIES_PHASE_13_HANDOFF.md`, which picked up
this doc's first deferred item (Milestone 6) but, per its own advisor
consultation, redirected to resolving the roadmap's rollback question first
(this doc's fourth deferred item) — found to already work via the existing
`publish` CAS mechanism, no new source code, filed as
[#18](https://github.com/jeromebanks/cubism-rs/issues/18) the one gap it
leaves open.

## What this session built

Read `docs/TIMESERIES_PHASE_11_HANDOFF.md` and `docs/TIMESERIES_ROADMAP.md`,
confirmed the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`), and listed all 16 open issues (titles only — read
the full bodies of #13, #14, and #16 later, before filing #17). Per the roadmap's
step-1 instructions, checked the milestone list first: Milestones 1-4 were
already done, Milestone 5 was the first not-done milestone with its listed
dependency (Milestone 4) done — the unambiguous pick.

Consulted the advisor before implementing. The advisor confirmed the pick
but flagged that Milestone 5's roadmap wording as written — a new
`ReconciliationRecord` type plus a new per-stage failure-injection seam on
`execute` — assumed work that was already unnecessary:

1. `control.rs`'s `RunState::Claimed`/`Appended`/`Published`
   (`crates/cubism-iceberg/src/control.rs:39-57`) already durably encode
   exactly the three stages a run passes through, on both the in-memory and
   SQLite backends, and `PublicationStore::run_state`
   (`control.rs:325`) already reads it back — the Milestone 2 situation
   again ("`ExpectedRevision` is already half-built"). A new persisted
   record would be untested scaffolding, the same reasoning Milestone 3
   used to refuse building unused checkpoint/range fields.
2. No injection seam was needed: `tests/coordinator.rs`'s existing
   `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
   (added during Milestone 4's advisor follow-up pass) already demonstrated
   the technique that generalizes to every stage — build the intermediate
   `RunState` externally via direct claim/append/record calls, then call
   `execute` and assert on recovery. The roadmap's "needs per-stage hooks
   or resumable steps" note was written before that test existed and was
   stale.

The advisor also required two verification steps before any test got
written, both of which changed what the milestone could honestly claim:

- **Read `AggregateReader::read_window`** (`crates/cubism-iceberg/src/reader.rs:28`)
  to determine whether it filters by `aggregate_snapshot_id` or by
  `(window_id, revision)` columns. It's the latter (`reader.rs:40-42`), and
  `AggregateWriter::append_window`'s `commit_append` (`crates/cubism-iceberg/src/writer.rs:136-141`)
  uses `fast_append`, which is purely additive with no idempotency check
  against a prior commit for the same window/revision/run. Together this
  means a run that crashes *between* the Iceberg append committing and
  `record_append` persisting that fact is genuinely ambiguous in the
  control store (still reports `Claimed`, indistinguishable from "append
  never attempted") and **not** safely recoverable by re-running `execute`
  — a naive retry would append a second time and duplicate visible rows.
  This is a real, unresolved gap, not smoothed over: filed as
  [#17](https://github.com/jeromebanks/cubism-rs/issues/17).
- **Read `SqliteStore::publish`** (`crates/cubism-iceberg/src/durable_control.rs:260-339`)
  to confirm it early-returns the existing publication when the requested
  revision is already current (`durable_control.rs:297-304`), in the same
  order as `InMemoryStore::publish` (`control.rs:208-210`) — before the CAS
  comparison against `expected_current`. This is why replaying a completed
  correction is idempotent (returns the same `Publication`) rather than
  surfacing `StaleRevision`, on both backends identically.

Primary files changed:

- **`crates/cubism-iceberg/src/coordinator.rs`** (modified): added
  `ReconciliationRecord` (enum at line 100; `classify` at line 111),
  classifying a run's recovery status from its `Option<&RunState>` into
  `NotStarted`/`AwaitingAppend`/`AwaitingPublish`/`Published`. `execute`
  (line 178) now branches on `ReconciliationRecord::classify(Some(claim.state()))`
  instead of the earlier raw `matches!(claim.state(), RunState::Claimed { .. })`
  check — same behavior, but the type is now load-bearing rather than
  dead scaffolding. The type's own doc comment documents, per variant,
  exactly what recovery is proven safe and what isn't (the `AwaitingAppend`
  ambiguity above). Also added a `#[cfg(test)] mod tests` unit test,
  `classify_maps_every_run_state_stage_to_its_reconciliation_record`,
  covering all four branches directly — matching `control.rs`'s existing
  convention of unit-testing pure classification logic in the same file.
- **`crates/cubism-iceberg/tests/durability.rs`** (modified): one new test,
  `sqlite_coordinator_execute_recovers_an_unattempted_claim_then_replays_a_published_run_after_reopen`
  (line 317). Two legs, each on a freshly opened catalog/control-store
  handle pair (this file's own convention — a real restart, not an
  in-process retry): leg 1 publishes an initial revision 1 directly, claims
  a correction (revision 2) but never attempts its append (simulating a
  crash right after claiming — the unambiguous half of `AwaitingAppend`,
  not the ambiguous half #17 tracks), then a fresh handle runs `execute`
  and the test asserts exactly one append happened (2 rows visible, not 4)
  and that `ReconciliationRecord::classify` on the resulting `RunState`
  reports `Published`. Leg 2 replays the identical `CorrectionRequest`
  through `execute` on another fresh handle and asserts the returned
  `Publication` is identical to leg 1's (not a new one, not
  `StaleRevision`) and that the row count is still 2, not 4.
  `AwaitingPublish` recovery is deliberately not re-proven here — the test's
  doc comment cites `tests/coordinator.rs`'s existing
  `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
  instead of duplicating that coverage.
- **`crates/cubism-iceberg/src/lib.rs`** (modified): added
  `ReconciliationRecord` to the `coordinator::` re-export list.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): Milestone 5 marked `Done
  (narrowed)`; "What it does"/"Test"/"Done when" narrowed in place, original
  wording preserved in a `(Corrected: ...)` parenthetical; a new bullet
  added under "Phase 4 done" pointing at #17 for the un-met literal "every
  stage" wording.

## What was actually verified

The tests prove: (1) `ReconciliationRecord::classify` correctly maps
`None`/`Claimed`/`Appended`/`Published` to
`NotStarted`/`AwaitingAppend`/`AwaitingPublish`/`Published` respectively,
carrying through the right `revision`/`aggregate_snapshot_id` fields —
proven directly against hand-built `RunState` values, not indirectly
through the coordinator; (2) a correction claimed but never attempted (an
unambiguous `AwaitingAppend`) genuinely survives a restart — asserted via
`ReconciliationRecord::classify` on the fresh handle's own `run_state`
lookup, before `execute` is even called, so the test does not merely assert
outcomes consistent with either a real restart or a fresh claim — and is
then recovered correctly by `execute` on that same freshly opened catalog
and control-store handle: exactly one append happens (2 rows visible after
recovery, not a partial or duplicated set), and the run classifies as
`Published` afterward; (3) replaying that same
completed correction through `execute` again, on yet another freshly
opened handle, returns the identical `Publication` (not a new one, not
`StaleRevision`) and does not duplicate rows (still 2, not 4) — proving
idempotent replay of a `Published` run survives a real restart, not just an
in-process retry.

They do **not** prove that a run whose Iceberg append committed but whose
`record_append` never persisted (the ambiguous half of `AwaitingAppend`) is
recoverable — it isn't, by the current design: re-running `execute` in that
situation would append a second time and duplicate visible rows, since
`fast_append` has no cross-commit idempotency check and `read_window`
filters by `(window_id, revision)`, not by snapshot ID or run ID. This is
the one plan-line-638 interruption point ("failure injection at every
commit/publication stage is recoverable") this milestone does not close;
tracked in #17, not silently assumed safe. They also do not prove anything
about `LatenessPolicy` consultation, event-time window identification, or a
public submit/inspect API — unrelated to this milestone's scope (Milestones
1, and the still-untouched Milestone 6, respectively).

## GitHub issues touched

- Filed [#17](https://github.com/jeromebanks/cubism-rs/issues/17): the
  append-committed-but-not-recorded ambiguity in `ReconciliationRecord::AwaitingAppend`
  this session's advisor consultation surfaced (specifically, the
  instruction to read `AggregateReader::read_window` and
  `AggregateWriter::append_window` before writing any test's doc comment).
  Checked it against #14's full body (SQLite retry-loop coverage — a
  different mechanism, the control store's own transaction retries, not
  Iceberg-append/control-store staleness) and #16's full body (event-time
  window identification — a different crate-boundary gap) first; neither
  already states this specific finding. #17 cross-links both as related,
  non-duplicate context.

## Deferred / not done this session

1. **Milestone 6** (public correction API: submit/schedule by source
   checkpoint or time range, inspect current/superseded revisions and
   reconciliation state) — the roadmap's next milestone, depends on
   Milestones 3, 4, 5 (all done). This is the next code slice per the
   roadmap, and per the roadmap's own "Phase 4 done" section, closing it
   also requires explicitly checking plan's four completion criteria
   (lines 651-655) against what was actually built, not just asserting
   "milestones done, therefore criteria met."
2. **#17** (append-committed-but-not-recorded recovery) — needs a design
   decision (idempotency check in `append_window`, pre-commit durable
   marker, or accept as a documented operational constraint) before it can
   be sized into a bounded milestone; not currently assigned to one.
3. **#16** (event-time window identification + recompute-equality proof) —
   unchanged from Phase 11; still needs a decision on which crate closes
   it.
4. **The rollback-point milestone gap** — still an open question the
   roadmap flags under "Phase 4 done," not resolved this session; whichever
   slice implements Milestone 6 should confirm whether rollback falls out
   of the existing CAS/publish mechanism or needs its own bounded
   milestone.
5. **Extending the roadmap to plan-Phase 5** — still deferred, unchanged
   from Phase 10/11.
6. Everything already deferred as of Phase 11 (`#8`/`#9`/`#10`/`#11`/`#12`/
   `#14`) is unchanged — this session did not touch any of that scope.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log` for
the exact hash — this doc deliberately doesn't hardcode it, the convention
adopted after `docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a follow-up
commit to fix a self-referential hash). That commit contains:

- New: `docs/TIMESERIES_PHASE_12_HANDOFF.md` (this file).
- Modified: `crates/cubism-iceberg/src/coordinator.rs` (`ReconciliationRecord`
  type, `execute` wiring, new unit test), `crates/cubism-iceberg/src/lib.rs`
  (re-export), `crates/cubism-iceberg/tests/durability.rs` (new integration
  test), `docs/TIMESERIES_ROADMAP.md` (Milestone 5 marked done/narrowed,
  Phase-4-done bullet added for #17), `docs/TIMESERIES_PHASE_11_HANDOFF.md`
  (added `**Superseded by:**` line).
- Untouched: `crates/cubism-core/`, `crates/cubism-cli/`, and everything
  else under `crates/cubism-iceberg/src/` except `coordinator.rs` and
  `lib.rs` — `control.rs`, `correction.rs`, `durable_control.rs`, `reader.rs`,
  `writer.rs`, and the rest of the crate are unchanged this session (read,
  not modified, for the two verification steps above).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (30 in `cubism-iceberg`, +2 this session; 167 in workspace, +2)

`cargo test -p cubism-iceberg` reports 30 passed (6 suites): 12 unit
(`--lib`, +1 this session) + 6 Phase-3 integration (`--test phase3`,
unchanged) + 5 durability integration (`--test durability`, +1 this
session) + 4 concurrency integration (`--test concurrency`, unchanged) + 3
coordinator integration (`--test coordinator`, unchanged). All figures were
confirmed by running each suite in isolation (`--lib`, `--test phase3`,
`--test durability`, `--test concurrency`, `--test coordinator`, each
individually) as well as the full `cargo test -p cubism-iceberg` run, not
derived by subtraction from the workspace total.
`cargo test --workspace --exclude cubism-py` reports 167 passed, 1 ignored
(22 suites) — a +2 from Phase 11's 165, matching this session's two new
tests; `cubism-core`'s 92 tests are untouched (no `cubism-core` source
changed this session).

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 30 passed (6 suites)
cargo test -p cubism-iceberg --lib                                        # 12 passed
cargo test -p cubism-iceberg --test phase3                                # 6 passed
cargo test -p cubism-iceberg --test coordinator                           # 3 passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed
cargo test -p cubism-iceberg --test durability                            # 5 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 167 passed, 1 ignored (22 suites; +2 from Phase 11's 165)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`../crates/cubism-iceberg/src/coordinator.rs`](../crates/cubism-iceberg/src/coordinator.rs)
  (`ReconciliationRecord` type and `classify`; `execute` now uses it)
- [`../crates/cubism-iceberg/tests/durability.rs`](../crates/cubism-iceberg/tests/durability.rs)
  (new integration test: unattempted-claim recovery, then published-run
  replay, both across a real restart)
- [`../crates/cubism-iceberg/src/lib.rs`](../crates/cubism-iceberg/src/lib.rs)
  (re-export)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 5 marked
  done/narrowed, Phase-4-done bullet added)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
  (plan line 638 — the failure-injection test this milestone narrows)
- [`TIMESERIES_PHASE_11_HANDOFF.md`](TIMESERIES_PHASE_11_HANDOFF.md)
- GitHub issue [`#13`](https://github.com/jeromebanks/cubism-rs/issues/13)
  (Phase 4 proper, this milestone's parent scope)
- GitHub issue [`#17`](https://github.com/jeromebanks/cubism-rs/issues/17)
  (this session's filed gap: append-committed-but-not-recorded recovery)
