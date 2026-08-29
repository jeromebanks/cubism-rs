# Time-Series Phase 13 Handoff

Date: 2026-08-14

Branch: `feature/timeseries-phase-0a`

Status: **Rollback (plan lines 666-669, "Rollback point") proven to fall out
of the existing `PublicationStore::publish` CAS mechanism, with zero new
source code — confirmed empirically, not just read.** One new integration
test in `crates/cubism-iceberg/tests/durability.rs`. Milestone 6 (public
correction API) itself is **not** started this session — the advisor
redirected the slice toward the rollback determination Milestone 6's roadmap
entry explicitly asks the implementing slice to resolve first, since the
finding changes what "superseded revision" even means for the inspection API
Milestone 6 would build. Full step-4 verification battery clean.
`docs/TIMESERIES_ROADMAP.md` updated: the "Phase 4 done" rollback bullet and
"Deferred" item 2 both closed with the finding, using the `(Corrected: ...)`
convention Milestones 3-5 established. Milestone 6 remains `Not started`.

(Despite the filename, this doc documents a session slice, not "Phase 13" of
the implementation plan — same convention every prior handoff in this series
has used: plan Phase 5 is DataFusion range queries and serving; plan Phase 4
is still on its sixth and final roadmap milestone. See
`docs/TIMESERIES_ROADMAP.md` for what this session's work means relative to
plan phases.)

**Superseded by:** `docs/TIMESERIES_PHASE_14_HANDOFF.md`, which picked up
this handoff's deferred item 1 (Milestone 6's inspection API, now landed as
`RunInspection`) and deferred item 2 (issue #18, resolved by that same
work).

## What this session built

Read `docs/TIMESERIES_PHASE_12_HANDOFF.md` and `docs/TIMESERIES_ROADMAP.md`,
confirmed the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`), and listed all 17 open issues. Per the roadmap's
step-1 instructions, Milestone 6 (public correction API) was the first
not-done milestone, with its listed dependencies (Milestones 3, 4, 5) all
done — the unambiguous pick per the roadmap's ordering.

Read `crates/cubism-iceberg/src/coordinator.rs` (full),
`crates/cubism-iceberg/src/control.rs` (types, `InMemoryStore::publish` at
lines 188-226), and `crates/cubism-iceberg/src/durable_control.rs`
(`SqliteStore::open`'s schema at lines 105-153, `SqliteStore::publish` at
lines 260-339) before the first advisor call, to ground the pick in what the
control store actually persists — specifically that `control_publications`'
primary key is `(cube_id, window_id)` (one row per window, no history table),
so "inspect superseded revisions" can only be reconstructed from
`control_runs`, which keeps one permanent row per `run_id`.

Consulted the advisor twice before writing any test:

1. **First call** confirmed Milestone 6 as the pick but flagged that its
   roadmap wording bundles three things — a submit/schedule facade (which,
   like Milestone 4, cannot map a source checkpoint/time range to windows
   without an aggregation engine this crate deliberately doesn't link), an
   inspection API, and the still-open "Rollback point" question the
   "Phase 4 done" section assigns to "whichever slice implements Milestone
   6's public API." It also asked me to verify a specific ordering claim
   before building on it: whether `publish`'s early-return path (for a
   revision that already matches what's current) fires *before* the
   `UPDATE control_runs SET status = 'published'` write, on both backends.
   I verified this by reading — confirmed identical ordering in
   `control.rs:206-210` (in-memory) and `durable_control.rs:297-304`
   (SQLite), both ahead of the status/`RunState::Published` write.
2. Reading `publish`'s full body to answer that question surfaced something
   not asked for: it has **no revision-monotonicity check** — only the CAS
   against caller-supplied `expected_current` — and always writes back the
   *calling run's own fixed revision* (set once, at `claim_run` time, never
   the CAS parameter). That means re-calling `publish` with a prior run's
   own `run_id` and a fresh `expected_current` matching the window's actual
   current (higher) revision should repoint `control_publications`
   backward. **Second advisor call**, presenting this finding, reversed the
   first call's implied split (inspection this slice, rollback next):
   rollback needing zero new source code is exactly Milestone 2's
   `ExpectedRevision` shape, sized to one slice on its own, and — more
   importantly — a correct inspection API cannot be designed until rollback's
   effect on "superseded" is known, since rollback breaks both candidate
   definitions of that word (`revision < current` breaks because a
   rolled-back-past revision can be *higher* than the new current; "was ever
   current" alone doesn't distinguish it from "is still recorded as
   `Published`" either). The advisor was explicit that this had to be
   *verified by running a test*, not concluded from the read, and that if
   `publish` refused the backward repoint the right move was to stop and
   re-scope rather than force it with a source change.

The test (below) confirmed the mechanism works exactly as read, including
the load-bearing part the advisor called out specifically: not just that
`control_publications` changes, but that `AggregateReader::read_window`
becomes reader-visible for the rolled-back-to revision — `read_window`
resolves through `publications.current` and filters by `(window_id,
revision)` (established in Milestone 5's test), so a control-store row
change alone would not have been sufficient evidence.

Primary files changed:

- **`crates/cubism-iceberg/tests/durability.rs`** (modified): one new test,
  `publish_repoints_a_window_to_a_prior_published_revision_via_the_existing_cas_mechanism`
  (end of file). Publishes revision 1 (run-1, 1 row), then revision 2 as a
  correction against it (run-2, 2 rows), on a handle then dropped. A fresh
  handle confirms revision 2 and its 2 rows are current, then calls
  `publish("run-1", Some(revision_2))` — the rollback — and asserts the
  returned `Publication` is `{ revision: 1, run_id: "run-1",
  aggregate_snapshot_id: <run-1's original snapshot> }`, that `current` moves
  back to revision 1, and that `read_window` returns exactly 1 row again
  (not 2) — both appends are still on disk (Iceberg appends are additive,
  never deleted), so this specifically proves which revision the reader
  selects changed, not that rows were deleted. A third, freshly-opened
  handle pair confirms the rollback persists, this file's standing
  convention. No source file changed — the mechanism was already built.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): the "Phase 4 done" section's
  rollback bullet and "Deferred" item 2 both closed with this finding, using
  the `(Corrected: ...)` convention; both note the two plan-line-666-669
  parts *not* proven (no scheduler to "stop correction scheduling," no
  snapshot expiry/retention to "retain a configured recovery window" before)
  and cross-link the new issue below. Milestone 6 itself left `Not started`
  — this session's work was the roadmap's own prerequisite for that
  milestone, not the milestone.

## What was actually verified

The new test proves: (1) `PublicationStore::publish` (SQLite backend) can
repoint `control_publications` for a window from a higher revision back to a
lower, already-published one, using only the existing CAS call with no new
method or source change; (2) the repoint is genuinely read-path visible
through `AggregateReader::read_window`, not just a control-store row
change — the row count returned flips from 2 back to 1, matching the
original revision's content, not the corrected revision's; (3) the returned
`Publication` correctly reflects the *rolled-back-to* run's own identity
(`run_id: "run-1"`, its original `aggregate_snapshot_id`), not a synthesized
one; (4) this is durable across a real process restart — the rollback is
performed on one freshly-opened handle and confirmed visible (both `current`
and `read_window`) from a second, independently-opened handle, this file's
standing convention; (5) the rollback is **not** safely inspectable
afterward through `RunState` alone — the test's final assertion, on the
third freshly-opened handle, calls `ReconciliationRecord::classify` on
run-2's own `run_state` after the rollback and it still reports `Published`
with revision 2, even though revision 2 is no longer current. This is
proven directly by the test, not inferred from reading
`durable_control.rs:330`'s `run_id`-scoped `UPDATE`.

It does **not** prove the other two-thirds of plan lines 666-669's wording:
no scheduler exists anywhere in this crate to "stop correction scheduling,"
and no snapshot expiry/retention exists to "retain a configured recovery
window" before superseded snapshots would be expired (that's
[#10](https://github.com/jeromebanks/cubism-rs/issues/10)'s scope). Item (5)
above is a real, currently-unresolved gap in what a caller could safely
infer from `RunState` alone post-rollback; not fixed by this session, filed
as
[#18](https://github.com/jeromebanks/cubism-rs/issues/18) rather than
silently assumed away. It also does not touch Milestone 6's other two
components (submit/schedule facade, inspection API) — neither exists yet.

## GitHub issues touched

- Filed [#18](https://github.com/jeromebanks/cubism-rs/issues/18): after a
  rollback via the mechanism this session proved, the run that used to be
  current (run-2 in the test) keeps `RunState::Published` in the control
  store — nothing clears or downgrades it, since rollback only touches
  `control_publications` and the *rolled-back-to* run's own `control_runs`
  row. So `ReconciliationRecord::classify` on that now-superseded run still
  reports `Published`, which `ReconciliationRecord`'s own doc comment states
  means "the run already published" with no caveat about a later rollback —
  no longer reliably true. Checked against #17's full body first: related
  (both are `RunState` stages that don't fully describe reader-visible
  truth) but a different mechanism — #17 is an *ambiguous* stage from a
  missing durable record between two operations; #18 is an *unambiguous but
  stale* stage after an operation (rollback) that doesn't touch it. Cross-
  linked as related, non-duplicate context. Also checked
  `concurrency.rs`'s `two_same_window_writers_produce_exactly_one_published_winner`
  before deciding whether the "same-revision-different-run-id" race deserved
  its own mention — it doesn't hit this shape (that test races two
  *different* target revisions against `expected: None`, and the loser
  correctly gets `Err(StaleRevision)` before ever reaching the status
  write), so #18's body notes it in one sentence rather than treating it as
  a separate finding.

## Deferred / not done this session

1. **Milestone 6's inspection API** (submit/schedule by source checkpoint or
   time range; inspect current/superseded revisions and reconciliation
   state) — now unblocked with the rollback semantics known, but not started.
   Per this session's advisor guidance, "superseded" needs a definition that
   survives rollback (a run's `RunState::Published` cannot be trusted
   standalone — see #18) before this API can be built without asserting
   something false. The submit/schedule half also needs the same "caller-
   identified window, not source-checkpoint/time-range" narrowing Milestone
   4 already established for `CorrectionCoordinator`, per this session's
   advisor's first call.
2. **#18** (superseded run's `RunState` reporting stale `Published` after
   rollback) — needs a design decision (live cross-check against `current`
   at inspection time, vs. a new durable rollback record) before Milestone
   6's inspection API can be sized; not currently assigned to a milestone.
3. **#17** (append-committed-but-not-recorded recovery) — unchanged from
   Phase 12; still needs a design decision before it can be sized into a
   bounded milestone.
4. **#16** (event-time window identification + recompute-equality proof) —
   unchanged from Phase 12; still needs a decision on which crate closes it.
5. **Extending the roadmap to plan-Phase 5** — still deferred, unchanged
   from prior sessions.
6. Everything already deferred as of Phase 12 (`#8`/`#9`/`#10`/`#11`/`#12`/
   `#14`) is unchanged — this session did not touch any of that scope.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log` for
the exact hash — this doc deliberately doesn't hardcode it, the convention
adopted after `docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a follow-up commit
to fix a self-referential hash). That commit contains:

- New: `docs/TIMESERIES_PHASE_13_HANDOFF.md` (this file).
- Modified: `crates/cubism-iceberg/tests/durability.rs` (new integration
  test), `docs/TIMESERIES_ROADMAP.md` (rollback finding recorded in two
  places), `docs/TIMESERIES_PHASE_12_HANDOFF.md` (added `**Superseded by:**`
  line).
- Untouched: every source file under `crates/cubism-iceberg/src/`,
  `crates/cubism-core/`, `crates/cubism-cli/` — this session found the
  rollback mechanism already sufficient and added no source code, only a
  test and docs.

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (31 in `cubism-iceberg`, +1 this session; 168 in workspace, +1)

`cargo test -p cubism-iceberg` reports 31 passed (6 suites): 12 unit
(`--lib`, unchanged) + 6 Phase-3 integration (`--test phase3`, unchanged) +
6 durability integration (`--test durability`, +1 this session) + 4
concurrency integration (`--test concurrency`, unchanged) + 3 coordinator
integration (`--test coordinator`, unchanged). All figures confirmed by
running each suite in isolation as well as the full `cargo test -p
cubism-iceberg` run, not derived by subtraction from the workspace total.
`cargo test --workspace --exclude cubism-py` reports 168 passed, 1 ignored
(22 suites) — a +1 from Phase 12's 167, matching this session's one new
test; `cubism-core`'s 92 tests are untouched (no `cubism-core` source
changed this session, and this session changed no source file at all).

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 31 passed (6 suites)
cargo test -p cubism-iceberg --lib                                        # 12 passed
cargo test -p cubism-iceberg --test phase3                                # 6 passed
cargo test -p cubism-iceberg --test coordinator                           # 3 passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed
cargo test -p cubism-iceberg --test durability                            # 6 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 168 passed, 1 ignored (22 suites; +1 from Phase 12's 167)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`../crates/cubism-iceberg/tests/durability.rs`](../crates/cubism-iceberg/tests/durability.rs)
  (new integration test: rollback via the existing `publish` CAS mechanism,
  proven reader-visible, across a real restart)
- [`../crates/cubism-iceberg/src/control.rs`](../crates/cubism-iceberg/src/control.rs)
  (`InMemoryStore::publish`, read not modified — the mechanism this session
  proved)
- [`../crates/cubism-iceberg/src/durable_control.rs`](../crates/cubism-iceberg/src/durable_control.rs)
  (`SqliteStore::publish`, read not modified — same mechanism, durable
  backend)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (rollback finding
  recorded under "Phase 4 done" and "Deferred" item 2; Milestone 6 still
  `Not started`)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
  (plan lines 666-669, the rollback-point requirement this session closes)
- [`TIMESERIES_PHASE_12_HANDOFF.md`](TIMESERIES_PHASE_12_HANDOFF.md)
- GitHub issue [`#13`](https://github.com/jeromebanks/cubism-rs/issues/13)
  (Phase 4 proper, this milestone's parent scope)
- GitHub issue [`#17`](https://github.com/jeromebanks/cubism-rs/issues/17)
  (append-committed-but-not-recorded recovery, cross-linked from #18)
- GitHub issue [`#18`](https://github.com/jeromebanks/cubism-rs/issues/18)
  (this session's filed gap: a rolled-back-past run's `RunState` still
  reports `Published`)
