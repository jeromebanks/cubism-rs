# Time-Series Phase 11 Handoff

Date: 2026-08-14

Branch: `feature/timeseries-phase-0a`

Status: **Milestone 4 (Coordinator/job API) landed, narrowed in scope.** New
module `crates/cubism-iceberg/src/coordinator.rs` with `CorrectionCoordinator`
and `CorrectionRequest`, three integration tests in
`crates/cubism-iceberg/tests/coordinator.rs` (two written during the initial
pass, a third added during the advisor follow-up pass — see "What this
session built" below), full step-4 verification battery clean.
`docs/TIMESERIES_ROADMAP.md` updated: Milestone 4 marked `Done (narrowed)`,
its "What it does"/"Test"/"Done when" sections narrowed in place to describe
what was actually built — executing a caller-identified correction, not
identifying affected windows from event time or proving recompute equality
from source, both of which need an aggregation engine this crate
deliberately doesn't link. That gap is filed as
[#16](https://github.com/jeromebanks/cubism-rs/issues/16). Milestones 1-3
(already done) untouched; Milestones 5-6 untouched.

(Despite the filename, this doc documents a session slice, not "Phase 11" of
the implementation plan — same convention every prior handoff in this series
has used: plan Phase 5 is DataFusion range queries and serving; plan Phase 4
is now four milestones into its six-milestone roadmap. See
`docs/TIMESERIES_ROADMAP.md` for what "Milestone 4" means relative to plan
phases.)

**Superseded by:** `docs/TIMESERIES_PHASE_12_HANDOFF.md`, which lands
Milestone 5 (`ReconciliationRecord` + failure-injection recoverability,
narrowed) — this handoff's deferred item 1.

## What this session built

Read `docs/TIMESERIES_PHASE_10_HANDOFF.md` and `docs/TIMESERIES_ROADMAP.md`,
confirmed the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`), and read all 15 open issues (before this session's
own #16 existed). Per the roadmap's step-1 instructions, checked the
milestone list first: Milestones 1-3 were already done, Milestone 4 was the
first not-done milestone with both listed dependencies (Milestones 2 and 3)
done — the unambiguous pick.

Consulted the advisor before implementing. The advisor confirmed the pick
but flagged that Milestone 4's roadmap wording as written — "identifies
affected windows from event time, rebuilds them completely... and publishes
a new revision" plus a test claiming "equals a clean rebuild from the
corrected source" — is not honestly buildable or testable inside
`crates/cubism-iceberg` as it exists today: this crate never links an
aggregation engine (`src/lib.rs`'s own top-level doc comment rules out a
`cubism-datafusion` dependency, to avoid the DataFusion 53/54 version seam
tracked in #8), so there is no way to identify which windows a correction
touches from raw event time, and no way to independently compute a "clean
rebuild" to compare against — hand-writing both sides of that comparison
inside a test would be tautological (identical hand-built bytes compared to
themselves), exactly the overstated-claim defect this series' tests are
written to avoid.

The advisor's resolution: split the milestone. Land the executable half —
"given a caller-identified window and already-rebuilt corrected content,
run the correction" — this session, narrow the roadmap's Milestone 4 entry
in place (the same convention Milestone 3 used for its own wording
correction), and file the event-time-identification/recompute-equality half
as a tracked gap rather than a milestone, since it needs a decision about
which crate closes it before it can be sized into a bounded slice. The
advisor also surfaced three plan constraints that would have been easy to
get wrong without prompting:

- Plan line 614 ("losing writers do not republish automatically without
  rereading source and current state") means the coordinator must not
  retry a rejected CAS internally — that stays the caller's protocol,
  exactly as Milestone 2's
  `sqlite_correction_against_a_superseded_revision_is_rejected_then_succeeds_on_retry`
  already established.
- The CAS anchor (`expected_current`) must be the revision the caller
  observed at planning time, taken as an explicit parameter, not re-read
  internally at publish time — otherwise the coordinator could silently
  paper over a stale read instead of surfacing it.
- `CorrectionCoordinator` must actually consult `CorrectionPlan::select`
  and hard-error on `CorrectionStrategy::AdditiveShortcut` rather than
  silently falling through to `FullRebuild` — a silent fallthrough would
  make Milestone 3's strategy selection dead code and make Milestone 4's
  "depends on Milestone 3" nominal rather than real. No current `AggKind`
  reaches that branch (Milestone 3's own finding), so this arm is written
  defensively and is not exercised by any test today — the doc comment
  says so explicitly, matching Milestone 3's tests' own admission about
  the same branch.

- **`crates/cubism-iceberg/src/coordinator.rs`** (new): `CorrectionRequest`
  (line 50) and `CorrectionCoordinator::execute` (line 77). `execute`
  rejects `AdditiveShortcut` (line 84-86), claims a run, appends only if the
  claim came back `Claimed` (i.e., skips a redundant append on an idempotent
  retry whose run was already appended — lines 99-113), and publishes via
  CAS against `request.observed_current` with no internal retry (line 115).
  `CorrectionRequest::observed_current` is a required `WindowRevision`, not
  `Option<WindowRevision>` — a type-level statement that a correction only
  targets an already-published window; a first-time build goes through
  `AggregateWriter`/`PublicationStore` directly, never this coordinator (see
  the module's doc comment, lines 26-33).
- **`crates/cubism-iceberg/tests/coordinator.rs`** (new): three tests.
  `coordinator_correction_is_revision_isolated_from_a_from_scratch_publish`
  publishes a partial revision directly, runs a correction through
  `CorrectionCoordinator` to replace it with fuller corrected content, and
  compares the coordinator-produced read against a from-scratch publish of
  the identical content in a fresh fixture — checked via a domain-row
  comparison, not just a row count. The isolation property itself is not
  new (`tests/phase3.rs:225`'s
  `publishing_a_new_revision_replaces_visibility_of_the_prior_one` already
  proves it at the raw protocol level); this test's addition is that the
  property survives when the corrected revision is produced through
  `CorrectionCoordinator` specifically (see the test file's module doc
  comment, added during the advisor follow-up pass below).
  `coordinator_rejects_a_correction_planned_against_a_superseded_revision_and_does_not_move_current`
  publishes revision 2 out from under a correction planned against
  revision 1, asserts the coordinator surfaces `StaleRevision` unchanged,
  and asserts `current` did not move.
  `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
  (added during the advisor follow-up pass — see below) simulates a crash
  between append and publish by claiming/appending/recording directly, then
  calls `execute` with identical inputs and asserts the returned
  `Publication::aggregate_snapshot_id` matches the first append's snapshot
  ID — proving no second append happened, since a second append would
  necessarily commit a different snapshot.
- **`crates/cubism-iceberg/src/lib.rs`** (modified): added `pub mod
  coordinator;` and `pub use coordinator::{CorrectionCoordinator,
  CorrectionRequest};`, following the existing pattern for `correction`.
- **`crates/cubism-iceberg/src/error.rs`** (modified): added
  `CubismIcebergError::UnsupportedCorrectionStrategy(CorrectionStrategy)`.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): Milestone 4 marked `Done
  (narrowed)`; "What it does"/"Test"/"Done when" narrowed in place, with the
  original wording preserved in a `(Corrected: ...)` parenthetical — matching
  Milestone 3's own correction device, not a strikethrough (an earlier draft
  of this doc and of the roadmap edit both claimed a "struck through"
  convention that doesn't actually exist in this series; fixed during the
  advisor follow-up pass, see below); a new bullet added under "Phase 4 done"
  pointing at #16 for the un-met literal wording; Milestone 5's entry gained
  a note that `execute` has no failure-injection seam yet, so that milestone
  needs to add one, not just layer `ReconciliationRecord` on top unchanged.

## Advisor follow-up pass

A second advisor call, made after the first commit, caught two overstated
claims and one factual error, applied here as a separate follow-up commit
(this series' `c2c92b3`/`d7efc04` pattern):

1. The append-skip branch (`coordinator.rs:99`) was described in this doc's
   first draft as "skips a redundant append on an idempotent retry," but no
   test exercised it — added
   `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
   (see "What this session built" above) rather than just softening the
   claim, since the property is real and cheaply testable.
2. The revision-isolation test's first draft (and this doc's first draft)
   implied the isolation property itself was newly proven. It wasn't —
   `tests/phase3.rs:225`'s
   `publishing_a_new_revision_replaces_visibility_of_the_prior_one` already
   proves it at the raw protocol level. Both the test file's module doc
   comment and this doc now cite that test and state precisely what's
   additional (the property surviving through `CorrectionCoordinator`).
3. The roadmap edit's first draft claimed a "struck through... per this
   series' additive-correction convention" — no such convention exists;
   Milestone 3's correction used a `(Corrected: ...)` parenthetical. Fixed
   in the roadmap to match that actual precedent instead.

The battery was re-run in full after these changes; the counts below are
the final ones.

## What was actually verified

The tests prove: (1) running a correction through `CorrectionCoordinator`
and reading back the published result is indistinguishable, row-for-row on
the domain columns (`bucket_start`/`xunit_id`/`count_v1`/`sum_v1`), from a
from-scratch publish of the identical corrected content in an unrelated
fixture — and, concretely, that only the corrected revision's two rows are
visible afterward, not three (which would mean the superseded partial
revision's row leaked through) — building on, not duplicating,
`tests/phase3.rs:225`'s existing proof that publishing a new revision
replaces visibility of the prior one at the raw protocol level; (2) a
correction whose `observed_current` no longer matches the window's actual
current revision is rejected with `CubismIcebergError::StaleRevision {
expected: Some(1), actual: Some(2), .. }` exactly as the raw
`PublicationStore::publish` CAS would reject it, with no coordinator-side
retry, and `current` is left unchanged by the rejected attempt; (3) a retry
through the coordinator against a run that was already appended (but not
yet published — simulating a crash between the two) does not perform a
second append, verified by the returned `Publication::aggregate_snapshot_id`
matching the first append's snapshot ID rather than a new one, and by the
read-back row count staying at 2, not 4.

They do **not** prove that the corrected content is itself a correct
recomputation from source events — both paths compared in the first test
hand-build identical batches, so a real aggregation bug in how "corrected
content" gets produced upstream would not be caught by this test. They also
do not prove anything about identifying which windows a correction should
touch from event time (no source events exist in this crate at all), about
`ReconciliationRecord` or failure-injection recovery (Milestone 5's scope —
and, per the roadmap's now-corrected Milestone 5 entry, `execute` has no
failure-injection seam yet for that milestone to use), or about a public
submit/inspect API (Milestone 6's scope). Both gaps are tracked in #16, not
silently left implicit.

## GitHub issues touched

- Filed [#16](https://github.com/jeromebanks/cubism-rs/issues/16): the
  event-time window-identification and recompute-equality gap this
  session's advisor consultation surfaced. Checked it against #13's full
  body (Phase 4's parent issue) and #8's full body (DataFusion
  `TableProvider`/version-seam issue) first — neither already states this
  specific crate-boundary finding; #13 lists the plan's literal wording as
  in-scope without noting the aggregation-engine gap, and #8 is about
  `TableProvider` exposure for range queries, a related but distinct
  problem. #16 cross-links both as related, non-duplicate context.

## Deferred / not done this session

1. **Milestone 5** (`ReconciliationRecord` + failure-injection
   recoverability) — the roadmap's next milestone, depends on Milestone 4
   (done, narrowed, this session). This is the next code slice per the
   roadmap.
2. **Milestone 6** — untouched, still depends on Milestone 5 (not done).
3. **Event-time window identification + recompute-equality proof** — filed
   as #16 this session; needs a decision on which crate closes it (likely
   `cubism-datafusion` gaining a persistence-side integration test, or a new
   orchestration crate) before it can be sized into a bounded milestone.
4. **Extending the roadmap to plan-Phase 5** — still deferred, unchanged
   from Phase 10.
5. **The rollback-point milestone gap** — still an open question the
   roadmap flags under "Phase 4 done," not resolved this session.
6. Everything already deferred as of Phase 10 (`#8`/`#9`/`#10`/`#11`/`#12`/
   `#14`) is unchanged — this session did not touch any of that scope.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log` for
the exact hash — this doc deliberately doesn't hardcode it, same convention
adopted after `docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a follow-up commit
to fix a self-referential hash). That commit contains:

- New: `crates/cubism-iceberg/src/coordinator.rs` (new module),
  `crates/cubism-iceberg/tests/coordinator.rs` (new integration test file,
  three tests), `docs/TIMESERIES_PHASE_11_HANDOFF.md` (this file).
- Modified: `crates/cubism-iceberg/src/lib.rs` (module wiring),
  `crates/cubism-iceberg/src/error.rs` (new error variant),
  `docs/TIMESERIES_ROADMAP.md` (Milestone 4 marked done/narrowed, Phase-4-done
  bullet added for #16), `docs/TIMESERIES_PHASE_10_HANDOFF.md` (added
  `**Superseded by:**` line), `docs/handoff_latest.md` (symlink repointed).
- Untouched: `crates/cubism-core/`, `crates/cubism-cli/`, and everything
  else under `crates/cubism-iceberg/src/` except `coordinator.rs`, `lib.rs`,
  and `error.rs` — `control.rs`, `correction.rs`, `durable_control.rs`, and
  the rest of the crate are unchanged this session.

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (28 in `cubism-iceberg`, +3 this session; 165 in workspace, +3)

`cargo test -p cubism-iceberg` reports 28 passed (6 suites): 11 unit
(`--lib`, unchanged from Phase 10) + 6 Phase-3 integration (`--test
phase3`, unchanged) + 4 durability integration (`--test durability`,
unchanged) + 4 concurrency integration (`--test concurrency`, unchanged) +
3 coordinator integration (`--test coordinator`, new this session — two
from the initial pass, one added during the advisor follow-up pass above).
All figures were confirmed by running each suite in isolation (`--lib`,
`--test phase3`, `--test durability`, `--test concurrency`, `--test
coordinator`, each individually) as well as the full `cargo test -p
cubism-iceberg` run, not derived by subtraction from the workspace total.
`cargo test --workspace --exclude cubism-py` reports 165 passed, 1 ignored
(22 suites) — a +3 from Phase 10's 162, matching this session's three new
tests, and +1 suite (the new `coordinator` integration-test binary);
`cubism-core`'s 92 tests are untouched (no `cubism-core` source changed
this session; confirmed via the workspace run's per-suite breakdown: 82
unit + 3 `phase1_properties` + 7 `properties` = 92).

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 28 passed (6 suites)
cargo test -p cubism-iceberg --lib                                        # 11 passed
cargo test -p cubism-iceberg --test phase3                                # 6 passed
cargo test -p cubism-iceberg --test coordinator                           # 3 passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed
cargo test -p cubism-iceberg --test durability                            # 4 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 165 passed, 1 ignored (22 suites; +3 from Phase 10's 162)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`../crates/cubism-iceberg/src/coordinator.rs`](../crates/cubism-iceberg/src/coordinator.rs)
  (new module: `CorrectionCoordinator`, `CorrectionRequest`)
- [`../crates/cubism-iceberg/tests/coordinator.rs`](../crates/cubism-iceberg/tests/coordinator.rs)
  (new integration tests: revision isolation, stale-CAS rejection,
  redundant-append skip)
- [`../crates/cubism-iceberg/src/lib.rs`](../crates/cubism-iceberg/src/lib.rs)
  (module wiring)
- [`../crates/cubism-iceberg/src/error.rs`](../crates/cubism-iceberg/src/error.rs)
  (`UnsupportedCorrectionStrategy` variant)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 4 marked
  done/narrowed, Phase-4-done bullet added)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
  (plan lines 599-600, 614, 634 — the coordinator description, no-auto-retry
  constraint, and rebuild-equality test this milestone narrows)
- [`TIMESERIES_PHASE_10_HANDOFF.md`](TIMESERIES_PHASE_10_HANDOFF.md)
- GitHub issue [`#13`](https://github.com/jeromebanks/cubism-rs/issues/13)
  (Phase 4 proper, this milestone's parent scope)
- GitHub issue [`#16`](https://github.com/jeromebanks/cubism-rs/issues/16)
  (this session's filed gap: event-time window identification and
  recompute-equality proof)
