# Time-Series Phase 14 Handoff

Date: 2026-08-15

Branch: `feature/timeseries-phase-0a`

Status: **Milestone 6 (public correction API), narrowed to its inspection
half, landed.** `RunInspection::inspect` (new,
`crates/cubism-iceberg/src/coordinator.rs`) pairs `ReconciliationRecord`'s
existing pure `RunState` classification with a live read of
`PublicationStore::current`, returning a new `RevisionStatus::Current` /
`RevisionStatus::NotCurrent`. This resolves
[#18](https://github.com/jeromebanks/cubism-rs/issues/18) (a rolled-back-past
run's `RunState` still reading `Published`): the inspection API now answers
"is this run's revision still current" correctly, by checking `current`
live rather than trusting the cached stage. The submit/schedule-by-source-
checkpoint-or-time-range half of Milestone 6's original wording is **not**
built — recorded as a non-goal, cross-linked to
[#16](https://github.com/jeromebanks/cubism-rs/issues/16), not a gap this
session leaves silently open. Two new tests (one unit, `coordinator.rs`; one
integration against the durable SQLite backend, `durability.rs`). Full
step-4 verification battery clean. `docs/TIMESERIES_ROADMAP.md` updated:
Milestone 6 marked `Done (narrowed)`; the "Phase 4 done" section's
completion-criteria bullet now walks all four plan criteria individually
(criterion 1 met, criterion 2 partially met via #17, criteria 3-4 out of
this roadmap's scope via #10) rather than asserting closure from "milestones
done."

(Despite the filename, this doc documents a session slice, not "Phase 14" of
the implementation plan — same convention every prior handoff in this series
has used: plan Phase 5 is DataFusion range queries and serving; the plan's
Phase 4 (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) is now at every
roadmap milestone done, but Phase 4 itself is not fully done — see
`docs/TIMESERIES_ROADMAP.md`'s "Phase 4 done" section for what remains.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_13_HANDOFF.md` and `docs/TIMESERIES_ROADMAP.md`,
confirmed the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`), and listed all 18 open issues. Per the roadmap's
step-1 instructions, Milestone 6 (public correction API) was the only
milestone not yet `Done`, with its listed dependencies (Milestones 3, 4, 5)
all done.

Read `crates/cubism-iceberg/src/coordinator.rs` (full, including Milestone
5's `ReconciliationRecord`), `crates/cubism-iceberg/src/control.rs`'s
`PublicationStore` surface (`rtk proxy grep -n "pub fn\|pub async fn\|pub
struct" control.rs`: `claim_run` / `record_append` / `publish` / `current` /
`run_state` — no per-window run-listing method), and issue #18's full body
before the advisor call.

Consulted the advisor before writing any code. It confirmed Milestone 6 as
the forced pick and narrowed it sharply:

- **The submit/schedule half is a non-goal, not a smaller task.** It needs
  the same source-checkpoint/time-range-to-window mapping Milestone 4
  already found this crate cannot do without an aggregation engine. Narrowed
  to "caller-identified window" the way `CorrectionCoordinator::execute`
  already is, a submit facade would be a zero-behavior wrapper with no
  logic of its own to test — the same untested-scaffolding refusal
  Milestones 3 (checkpoint/range fields) and 5 (a second persisted record)
  already made. Cross-link to #16 rather than filing a new issue for it.
- **Don't split #18 into its own slice** — its resolution *is* the
  inspection API's core semantics, not a prerequisite separable from it.
- **The design decision is already made in #18's own body:** option 1 (live
  cross-check against `PublicationStore::current`), not option 2 (a new
  durable rollback record) — matching this roadmap's established preference
  for not adding persisted records without a concrete consumer.
- **The scope-discriminating question:** does the inspection API *enumerate*
  revisions for a window, or *classify a given run*? `control.rs`'s surface
  has no per-window run-listing method and `control_publications` has no
  history table (PK is `(cube_id, window_id)`, one row per window) — so
  enumeration would require a new store method on **both** `InMemoryStore`
  and `SqliteStore` with SQL and parity work, well past one bounded slice.
  Classification over the existing `current` + `run_state` reads adds zero
  store methods and zero backend changes — exactly the Milestone 5 shape
  (a pure projection plus one live read over already-durable state). Took
  the classifier.
- **Naming caution:** don't call the not-current state "superseded." After a
  rollback, a not-current revision can be numerically *higher* than
  current, so "superseded" (implying "replaced by something newer") would
  misdescribe it. Named the states `RevisionStatus::Current` /
  `RevisionStatus::NotCurrent` instead, and said so explicitly in the type's
  doc comment.
- **Handoff-writing note, acted on below:** if Milestone 6 is marked `Done`,
  the roadmap's own "Phase 4 done" section requires walking plan lines
  651-655's four completion criteria individually, not asserting closure
  from "every milestone done." Read plan lines 580-670 directly (not just
  the roadmap's paraphrase) before writing that walkthrough.

Primary files changed:

- **`crates/cubism-iceberg/src/coordinator.rs`** (modified): new
  `RevisionStatus` enum (`Current` / `NotCurrent`) and `RunInspection`
  struct (`record: ReconciliationRecord`, `revision_status:
  Option<RevisionStatus>`) with `RunInspection::inspect(publications,
  cube_id, window_id, run_id)`. `revision_status` is `Some` only when
  `record` is `ReconciliationRecord::Published` — every earlier stage has no
  revision yet to compare against `current`. Added a caveat to
  `ReconciliationRecord::Published`'s existing doc comment pointing at this
  gap and at `RunInspection` as the fix. New unit test
  `run_inspection_pairs_reconciliation_stage_with_a_live_current_check`
  (`mod tests`) replays the Phase 13 rollback shape entirely against
  `PublicationStore::in_memory()` (no Iceberg catalog needed — `publish`
  only requires a prior `record_append`, not a real commit), proving: no
  revision status before publish, `Current` for the run holding `current`,
  and `NotCurrent` for a run whose own `RunState` still reads `Published`
  after a later rollback.
- **`crates/cubism-iceberg/tests/durability.rs`** (modified): new
  integration test
  `run_inspection_distinguishes_the_rolled_back_to_run_from_the_rolled_back_past_run`,
  built by reusing the exact rollback sequence from
  `publish_repoints_a_window_to_a_prior_published_revision_via_the_existing_cas_mechanism`
  (publish revision 1, publish revision 2 as a correction, roll back to
  revision 1), then opening a **fresh** handle and calling
  `RunInspection::inspect` for both run-1 and run-2. Proves the same
  distinction the unit test proves, but through the durable SQLite backend
  from a freshly-opened handle, this file's standing convention — the unit
  test alone doesn't prove the durable backend's `current`/`run_state` reads
  compose correctly for this purpose.
- **`crates/cubism-iceberg/src/lib.rs`** (modified): exported
  `RevisionStatus` and `RunInspection` alongside the existing
  `ReconciliationRecord` export.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): Milestone 6 marked `Done
  (narrowed)` with its "What it does"/"Test"/"Done when" entries corrected
  to the inspection-only scope; the "Phase 4 done" section's completion-
  criteria bullet rewritten to walk plan lines 651-655 individually (see
  "What was actually verified" below); the "Rollback point" bullet's #18
  note corrected to record #18 as resolved this session.

## What was actually verified

The new tests prove: (1) `RunInspection::inspect` returns `revision_status:
None` for a run that has not yet published — there is no revision yet to
compare against `current`; (2) it returns `Some(RevisionStatus::Current)`
for a run whose published revision is the window's live `current` value;
(3) it returns `Some(RevisionStatus::NotCurrent)` for a run whose `RunState`
still reads `Published` (unchanged, since rollback never touches the
rolled-back-past run's own control-store row) after a later rollback
repointed `current` away from it — the exact gap #18 tracked, now answered
correctly instead of silently trusting the stale `Published` stage; (4) this
distinction holds through the **durable SQLite backend**, read from a
handle freshly opened after the writing handle was dropped — not just the
in-memory backend the unit test exercises, and not just the writing
handle's own in-process belief about what it wrote.

It does **not** prove: a submit/schedule facade (deliberately not built —
non-goal, see above); enumeration of all revisions ever published for a
window (no store method exists for this; `RunInspection::inspect` classifies
one caller-named `run_id` at a time); that `#17`'s ambiguous
`AwaitingAppend` recovery gap is fixed (unrelated — `RunInspection` only
changes what a `Published` stage's revision-currency question answers, not
recovery from an interrupted append); or that Phase 4 (plan lines 582-670)
is fully done — see the roadmap's rewritten "Phase 4 done" walkthrough:
criterion 1 (atomic replacement) is met, criterion 2 (deterministic
recovery of every interrupted job) is only partially met (#17's ambiguous
stage remains unrecoverable), and criteria 3-4 (compaction/retention SLOs,
no maintenance path changing answers) are entirely out of this roadmap's
scope, tracked in #10.

## GitHub issues touched

- No new issues filed. The advisor's scope check confirmed the
  submit/schedule non-goal is already covered by
  [#16](https://github.com/jeromebanks/cubism-rs/issues/16) (event-time
  window identification) — filing a new issue for "Milestone 6's
  submit/schedule half" would have duplicated it.
- [#18](https://github.com/jeromebanks/cubism-rs/issues/18) is resolved by
  this session's work (recorded in `docs/TIMESERIES_ROADMAP.md`, not closed
  on GitHub — no issue in this repo has been closed via `gh issue close`;
  the established convention is to record resolutions with a `(Corrected:
  ...)` note and cross-link, which this handoff and the roadmap edit both
  do). `RunInspection::inspect` implements #18's own suggested option 1
  verbatim: a live cross-check against `PublicationStore::current`, not a
  trusted `RunState::Published` read in isolation. The raw `RunState` row
  for a rolled-back-past run is still unchanged, as #18 described and this
  session's new tests still show directly (`run2.record` still classifies
  `Published`) — what changed is that `revision_status` now answers the
  "is it actually current" question correctly instead of nothing answering
  it at all.

## Deferred / not done this session

1. **#17** (append-committed-but-not-recorded recovery) — unchanged from
   Phase 12/13; still needs a design decision before it can be sized into a
   bounded milestone. This is now the one remaining gap in Phase 4
   completion criterion 2 (see roadmap walkthrough above).
2. **#16** (event-time window identification + recompute-equality proof,
   and now also the home for Milestone 6's submit/schedule non-goal) —
   unchanged from Phase 12/13; still needs a decision on which crate closes
   it.
3. **Extending the roadmap to plan-Phase 5** — still deferred, unchanged
   from prior sessions. With every roadmap Milestone now `Done`, the
   roadmap's own step-1 instructions say future slices fall back to its
   original ad hoc deferred-list/issue scan (starting with #10 and this
   note) rather than consulting a milestone list — unless a successor
   roadmap doc extends coverage to Phase 5 first.
4. **#10** (real object store + Iceberg maintenance: compaction, retention)
   — unchanged; this is Phase 4 completion criteria 3-4's entire remaining
   scope, deliberately out of this roadmap.
5. **#11/#12/#14** — unchanged from Phase 12/13, this session did not touch
   any of that scope.
6. **A per-window revision-listing API** (enumerate every `run_id` ever
   published for a window, not just classify one caller-named run) — not
   built this session; the advisor's scope check found it would need a new
   `PublicationStore` method implemented and SQL-parity-tested on both
   `InMemoryStore` and `SqliteStore`, past one bounded slice. Not filed as
   an issue — no concrete consumer has asked for it yet, matching this
   roadmap's established preference (Milestones 3/5) for not building
   unused surface ahead of a need.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log` for
the exact hash — this doc deliberately doesn't hardcode it, the convention
adopted after `docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a follow-up commit
to fix a self-referential hash). That commit contains:

- New: `docs/TIMESERIES_PHASE_14_HANDOFF.md` (this file).
- Modified: `crates/cubism-iceberg/src/coordinator.rs` (`RevisionStatus`,
  `RunInspection`, new unit test), `crates/cubism-iceberg/src/lib.rs` (new
  exports), `crates/cubism-iceberg/tests/durability.rs` (new integration
  test), `docs/TIMESERIES_ROADMAP.md` (Milestone 6 closed, "Phase 4 done"
  criteria walkthrough, #18 resolution note), `docs/TIMESERIES_PHASE_13_HANDOFF.md`
  (added `**Superseded by:**` line).
- Untouched: `crates/cubism-core/`, `crates/cubism-cli/`,
  `crates/cubism-iceberg/src/control.rs`,
  `crates/cubism-iceberg/src/durable_control.rs`,
  `crates/cubism-iceberg/src/correction.rs` — this session read the control
  store's existing surface but added no new store methods, matching the
  advisor's scope narrowing (classification, not enumeration).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 in `cubism-iceberg`, +2 this session; 170 in workspace, +2)

`cargo test -p cubism-iceberg` reports 33 passed (6 suites): 13 unit
(`--lib`, +1 this session) + 6 Phase-3 integration (`--test phase3`,
unchanged) + 7 durability integration (`--test durability`, +1 this
session) + 4 concurrency integration (`--test concurrency`, unchanged) + 3
coordinator integration (`--test coordinator`, unchanged). All figures
confirmed by running each suite in isolation as well as the full `cargo
test -p cubism-iceberg` run, not derived by subtraction from the workspace
total. `cargo test --workspace --exclude cubism-py` reports 170 passed, 1
ignored (22 suites) — a +2 from Phase 13's 168, matching this session's two
new tests; `cargo test -p cubism-core` reports 92 passed (4 suites),
unchanged from Phase 13 — confirmed by running it directly, not assumed
from "no `cubism-core` source changed."

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 33 passed (6 suites)
cargo test -p cubism-iceberg --lib                                        # 13 passed
cargo test -p cubism-iceberg --test phase3                                # 6 passed
cargo test -p cubism-iceberg --test coordinator                           # 3 passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 170 passed, 1 ignored (22 suites; +2 from Phase 13's 168)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
cargo test -p cubism-core                                                 # 92 passed (4 suites), unchanged from Phase 13
```

## Primary files

- [`../crates/cubism-iceberg/src/coordinator.rs`](../crates/cubism-iceberg/src/coordinator.rs)
  (`RevisionStatus`, `RunInspection`, `RunInspection::inspect`, new unit
  test, corrected `ReconciliationRecord::Published` doc comment)
- [`../crates/cubism-iceberg/tests/durability.rs`](../crates/cubism-iceberg/tests/durability.rs)
  (new integration test proving the same distinction through the durable
  SQLite backend)
- [`../crates/cubism-iceberg/src/lib.rs`](../crates/cubism-iceberg/src/lib.rs)
  (new public exports)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 6 closed
  narrowed; "Phase 4 done" completion-criteria walkthrough; #18 resolution
  note)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
  (plan lines 582-670, Phase 4 in full; lines 603-604, the "Public API
  changes" bullets this session's inspection half addresses)
- [`TIMESERIES_PHASE_13_HANDOFF.md`](TIMESERIES_PHASE_13_HANDOFF.md) (the
  rollback finding this session's tests replay, and #18's origin)
- GitHub issue [`#13`](https://github.com/jeromebanks/cubism-rs/issues/13)
  (Phase 4 proper, this milestone's parent scope)
- GitHub issue [`#16`](https://github.com/jeromebanks/cubism-rs/issues/16)
  (event-time window identification — now also the cross-link for Milestone
  6's submit/schedule non-goal)
- GitHub issue [`#17`](https://github.com/jeromebanks/cubism-rs/issues/17)
  (append-committed-but-not-recorded recovery — the one remaining gap in
  Phase 4 completion criterion 2)
- GitHub issue [`#18`](https://github.com/jeromebanks/cubism-rs/issues/18)
  (resolved this session by `RunInspection::inspect`)
