# Time-Series Roadmap: Phase 4-5 Session-Slice Milestones

Date: 2026-08-12 (Phase 5 section added 2026-08-15, see that section's own
note)

Branch: `feature/timeseries-phase-0a`

This is a **session-sequencing layer**, not a spec. `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`
stays the authoritative phase-level spec; this doc decomposes its Phase 4
section (lines 582-670) into an ordered list of milestones sized to what one
`.claude/skills/timeseries-slice/SKILL.md` run can land — "one bounded
type/API surface plus its test," the same size every prior slice in this
series has been. Filed in response to
[#15](https://github.com/jeromebanks/cubism-rs/issues/15).

Scope: Phase 4 minus compaction/retention/object-store (tracked separately in
[#10](https://github.com/jeromebanks/cubism-rs/issues/10)), per
[#13](https://github.com/jeromebanks/cubism-rs/issues/13)'s explicit
non-goals, **plus Phase 5** (exactness-aware DataFusion range queries and
serving — see "Phase 5 Milestones" below, added once every Phase 4 milestone
was done, per #15's own suggested steps). Phase 5's SQL/pushdown-generality
half stays gated on [#8](https://github.com/jeromebanks/cubism-rs/issues/8)
(DataFusion 53/54 convergence for `iceberg-datafusion`'s `TableProvider`);
its direct-call functional half is not — see "Phase 5 Milestones" for the
evidence and the boundary between them. **Plus "POC Milestones"** (added
2026-08-18, once Phase 4 and Phase 5 above were both narrow-closed): serving,
demo, and documentation work that makes the now-complete engine legible to
someone outside this session, not itself part of the implementation plan's
phase numbering — see that section for why it's named "POC" rather than
"Phase 6" (plan Phase 6 is rolling comparisons/trend inputs, a different,
later thing).

## How `timeseries-slice` step 1 should use this doc

When picking the next slice, check the milestone list below **before**
falling back to the ad hoc deferred-list/advisor scan the skill used before
this doc existed:

1. Find the first milestone below not yet marked done — each milestone entry
   carries a `**Status:**` line; `Not started` until a slice closes it, then
   `Done — <handoff link>` naming the handoff whose "What was actually
   verified" section confirmed the done-condition. This is the marking
   convention this doc uses; it didn't exist before Milestone 1 closed and
   is established here so a later slice has something to read instead of
   re-deriving "done" from prose.
2. Confirm its "Depends on" milestones are done. If not, something is wrong —
   stop and reconcile before picking a slice (a later milestone should never
   be reachable before its dependencies).
3. Advisor still reviews the pick — this doc narrows the candidate list to
   one, it doesn't replace the advisor's scope/locking-model check (skill
   step 1, item 3).
4. **Once every milestone below is marked done** (Phase 4's six plus Phase
   5's, see "Phase 5 Milestones", plus the "POC Milestones" section), this
   roadmap's milestone list is exhausted — not the same claim as Phase 4
   (`#13`'s scope) or Phase 5 being finished; see "Phase 4 done" and "Phase
   5 Milestones" below for why (as of Milestone 6 closing, Phase 4 is not:
   criterion 2 is open via #17, criteria 3-4 are out of this roadmap's scope
   via #10; Phase 5's own completion criteria are walked in that section).
   (Corrected: criterion 2 is now **Met** — Milestone 5b fixed #17, see
   "Phase 4 done condition" below. As of Milestone 13 landing, Phase 4 as
   this roadmap defines it has been done since Milestone 5b; the only
   still-open criteria are 3-4, out of scope via #10 by design, not a gap
   this roadmap tracks toward.)
   At that point step 1 has no more milestones to consult here and should
   fall back to its original behavior: scan open issues and the latest
   handoff's deferred list directly, starting with #10
   (compaction/retention/object-store, still out of this roadmap's scope) —
   the Phase 5 fallback this line used to point to is resolved now that
   Phase 5 has its own milestones below.

## Current state (read before drafting Milestone 1's implementation)

Grounding for what already exists, so a milestone doesn't re-discover or
re-litigate it:

- `PublicationStore::publish(run_id, expected_current: Option<WindowRevision>)`
  (`crates/cubism-iceberg/src/control.rs:188`, durable variant
  `crates/cubism-iceberg/src/durable_control.rs:260`) already implements
  optimistic compare-and-swap against a caller-supplied expected revision,
  and already rejects a stale caller with `CubismIcebergError::StaleRevision`
  (tested: `control.rs`'s `stale_publish_cannot_replace_a_newer_revision`,
  one of the 8 unit tests in the current 21-test count). This is the
  plan's "ExpectedRevision" half of "`WindowLease` or `ExpectedRevision`"
  already built — Milestone 2 below is about formalizing/naming it and
  testing it in a correction-shaped scenario, not building CAS from
  scratch.
- None of `LatenessPolicy`, `CorrectionPlan`, `ReconciliationRecord`,
  `CompactionPlan`, `RetentionPlan`, or `WindowLease` exist anywhere in
  `crates/cubism-iceberg/src` or `crates/cubism-core/src` today (confirmed
  via `rtk proxy grep -rn` over both, zero matches). Milestones 1, 3, and 5
  below start from nothing.
- `cubism-core::temporal` already has an `AllowedLateness` duration type
  (`crates/cubism-core/src/temporal.rs:377`, `pub allowed_lateness:
  AllowedLateness` on `TemporalSpec` at line 584) and an `IngestionTime`
  timestamp type (line 41) whose module doc already states "Ingestion time
  exists only for watermark/lateness decisions" (lines 4-5) — but as of
  Milestone 1's start, nothing in the workspace *consumed* `IngestionTime`
  outside its own definition/export (`rtk proxy grep -rn "IngestionTime"
  crates/ | grep -v target` returned only the definition and the `lib.rs`
  re-export). Milestone 1 wraps `AllowedLateness` rather than inventing a
  new duration type, exactly parallel to Milestone 2's "`ExpectedRevision`
  is already half-built" note above — and lands in `cubism-core`, not
  `cubism-iceberg`, because that unclaimed `IngestionTime` purpose statement
  is evidence the classification concept belongs beside it, in
  `cubism-core::temporal`, not in the Iceberg persistence crate.
- Plan lines 636 (two same-window writers produce one winner) and 637
  (disjoint windows commit concurrently) are done —
  `docs/TIMESERIES_PHASE_5_HANDOFF.md` and
  `docs/TIMESERIES_PHASE_6_HANDOFF.md` respectively. They are **not**
  milestones below; this roadmap starts at the plan's remaining Phase 4 test
  list (lines 634-635, 638-639).

## Milestones

Each entry: target file(s), the one test it adds, its dependencies, and its
done-condition. A milestone is done when its test is committed, passing, and
the full step-4 verification battery is clean — not when the type merely
compiles.

### Milestone 1 — `LatenessPolicy`

- **Status:** Done — `docs/TIMESERIES_PHASE_8_HANDOFF.md`. Corrected:
  `classify`'s deadline arithmetic was widened from `i64` to `i128` by
  Phase 4's step 8a review, which found it could overflow on valid extreme
  `bucket_end`/`allowed` inputs — see
  `docs/phase-reviews/TIMESERIES_PHASE_4_REVIEW.md`.
- **Target:** landed in `cubism-core`, not `cubism-iceberg`
  (`crates/cubism-core/src/temporal.rs`, beside `AllowedLateness`) — see
  "Current state" above for why: `IngestionTime`'s module-doc purpose
  ("watermark/lateness decisions") was unclaimed by any consumer, which is
  evidence the concept belongs in core, and it's a pure function with no
  Iceberg/control-store dependency.
- **What it does:** `LatenessPolicy::classify(event_time, bucket_end)` is a
  pure boundary check — `OnTime` iff `event_time` is strictly before
  `bucket_end + allowed` — that does **not** consult
  `PublicationStore`/`Publication` or a real ingestion-time watermark; both
  timestamps are caller-supplied. Caller contract Milestone 3+ must get
  right: `bucket_end` is the already-closed *earlier* window being tested,
  not necessarily the bucket `event_time` itself falls into, and
  `event_time` stands in for the caller's current watermark position.
  Passing an event's own `bucket_end` always yields `OnTime` (bucket
  membership already guarantees `event_time < bucket_end`, and
  `AllowedLateness` can't be negative) — see the type's doc comment for the
  full reasoning. Plan line 659's "window duration versus correction blast
  radius" is resolved:
  `LatenessPolicy` is constructed from an `AllowedLateness` alone, never a
  window's `Resolution`, so the lateness bound is independent of window
  duration by construction (recorded in the type's own doc comment,
  `temporal.rs`). Two-valued by design — plan line 662 ("too old, reject as
  backfill") is deliberately left open, not answered by this milestone.
- **Test:** `lateness_policy_classifies_on_the_allowed_lateness_boundary`
  (`crates/cubism-core/src/temporal.rs`, `mod tests`) — one event one
  microsecond before the deadline classifies `OnTime`, the event exactly at
  the deadline classifies `Late`.
- **Depends on:** nothing (first milestone).
- **Done when:** `LatenessPolicy` exists, is unit-tested, and the plan's
  "window duration versus correction blast radius" decision is recorded
  (either in this file or the type's own doc comment — a future slice
  reading this roadmap should not have to re-derive it). **Met** — decision
  recorded in `temporal.rs`'s doc comment (see "What it does" above), full
  step-4 battery clean (`docs/TIMESERIES_PHASE_8_HANDOFF.md`).

### Milestone 2 — Formalize `ExpectedRevision`, test it against a correction shape

- **Status:** Done — `docs/TIMESERIES_PHASE_9_HANDOFF.md`.
- **Target:** the existing `expected_current: Option<WindowRevision>`
  parameter on `PublicationStore::Sqlite::publish`
  (`crates/cubism-iceberg/src/durable_control.rs:260-339`) — confirmed
  sufficient as-is, no source change needed — plus a new test in
  `tests/durability.rs`.
- **What it does:** resolves plan line 660's unresolved decision ("lease
  service versus optimistic expected-revision only") explicitly in favor of
  the already-built optimistic expected-revision mechanism (no new lease
  service — see "Current state" above for why this is mostly already built),
  and adds the dedicated test plan line 639 asks for, framed around a
  correction attempt specifically. **Correction to this entry's original
  text:** it previously claimed the new test would be "distinct from
  `control.rs`'s existing `stale_publish_cannot_replace_a_newer_revision`
  test (which races two initial runs, not a correction against a superseded
  revision)" — that was wrong. `stale_publish_cannot_replace_a_newer_revision`
  already *is* step-for-step the superseded-revision shape this milestone
  describes (publish rev1, publish rev2 against expected=rev1, publish rev3
  against expected=rev1 → `StaleRevision`); the test that races two initial
  runs against `expected: None` is `tests/concurrency.rs`'s
  `two_same_window_writers_produce_exactly_one_published_winner` (lines
  63/111/115), not that one. The real gap, found via advisor review, was
  narrower: no existing test (a) ran the superseded-revision CAS shape
  against the **durable** (SQLite) backend — `durability.rs`'s only CAS-reject
  test used `Some(99)`, a value that was never valid, not a genuinely
  superseded one — and (b) followed a rejected correction through
  refresh-`current`-then-retry to success, the protocol Milestone 4's
  coordinator will need. The new test fills exactly that intersection; it
  does not duplicate either existing test.
- **Test:**
  `sqlite_correction_against_a_superseded_revision_is_rejected_then_succeeds_on_retry`
  (`crates/cubism-iceberg/tests/durability.rs`) — publish run-1 (rev1,
  `expected: None`), publish run-2 (rev2, `expected: Some(rev1)`), then
  attempt run-3 (the correction) with `expected: Some(rev1)` → asserts
  `StaleRevision { expected: Some(1), actual: Some(2), .. }` and that
  `current` did not move; then re-reads `current`, retries run-3's publish
  with the refreshed expected revision, and asserts it succeeds and is
  visible from a fresh handle.
- **Depends on:** nothing structurally new required, but should follow
  Milestone 1 so the test can plausibly describe the corrected write as
  "late" per that policy (framing only, in the test's doc comment — no code
  dependency on `LatenessPolicy`).
- **Done when:** the unresolved decision is recorded and the correction-shaped
  stale-rejection test passes. This closes plan line 639. **Met** — decision
  recorded above, full step-4 battery clean
  (`docs/TIMESERIES_PHASE_9_HANDOFF.md`).

### Milestone 3 — `CorrectionPlan`

- **Status:** Done — `docs/TIMESERIES_PHASE_10_HANDOFF.md`.
- **Target:** new type in `crates/cubism-iceberg/src`, informed by
  `cubism-core`'s existing per-`AggKind` capability declarations
  (`capabilities_for`, `crates/cubism-core/src/aggregate_state.rs:109-160`)
  to determine which are additive/subtractable and which require a full
  rebuild. (Corrected: the original wording here named `AVG`/`VAR`/`QNT`,
  which are the wire-format magic constants for three specific state
  encodings, not the `subtractable`/`idempotent` capability flags the
  landed code actually reads across all eleven `AggKind` variants.)
- **What it does:** `crates/cubism-iceberg/src/correction.rs`'s
  `CorrectionPlan::select` decides strategy only — `FullRebuild` vs.
  `AdditiveShortcut` — from the `AggKind`s a correction would touch, using
  `cubism_core::capabilities_for`'s `subtractable`/`idempotent` flags on
  each. It does **not** represent a source checkpoint, a time range, or the
  set of affected windows: nothing in this crate consumes those fields yet,
  so building them now would be untested scaffolding. That richer
  representation is Milestone 4's coordinator's job to add when it actually
  has a checkpoint/range to plan around. It also does not consult
  `LatenessPolicy` — strategy selection is a pure function of aggregate-kind
  capabilities, not of lateness.
- **Test:** plan line 635 — `full_rebuild_is_selected_for_every_current_agg_kind`
  proves every current `AggKind` is refused an additive shortcut (each
  individually and as a full set); `shortcut_requires_both_subtractable_and_idempotent`
  proves the refusal rule is genuinely the conjunction of both capability
  flags, not `subtractable` alone, using `Sum` (subtractable, not
  idempotent) as the discriminating case. No current `AggKind` satisfies
  both flags, so the `AdditiveShortcut` branch is not exercised by any real
  kind — only reachable in principle, which the tests' doc comments state
  explicitly rather than implying broader coverage.
- **Depends on:** Milestone 1 — corrected: this turned out not to be a real
  dependency. `LatenessPolicy` determines *which events* a correction
  responds to, a question this milestone's strategy-selection logic never
  needed to answer. Left as `Not started`-era wording elsewhere in this repo
  should be read in light of this correction wherever it recurs.
- **Done when:** the type exists and plan line 635's test passes.

### Milestone 4 — Coordinator/job API + late-event rebuild test

- **Status:** Done (narrowed) — `docs/TIMESERIES_PHASE_11_HANDOFF.md`.
- **Target:** new coordinator module in `crates/cubism-iceberg/src`
  (`coordinator.rs`).
- **What it does:** (Corrected: the original wording here — "identifies
  affected windows from event time, rebuilds them completely using the
  existing append/publish protocol, and publishes a new revision via the
  CAS mechanism formalized in Milestone 2" — assumed an aggregation engine
  this crate deliberately does not link; see
  `crates/cubism-iceberg/src/coordinator.rs`'s module doc comment for the
  full reasoning.) `CorrectionCoordinator::execute` takes a
  **caller-identified** window plus already-rebuilt corrected
  states/registry batches (not raw source events), consults
  `CorrectionPlan::select` (hard-erroring on `AdditiveShortcut`, which no
  current `AggKind` reaches — matching Milestone 3), appends under the
  requested revision (skipping a redundant append if the run was already
  appended — an idempotent-retry path, not a crash-recovery/reconciliation
  one; see Milestone 5 below), and publishes via the CAS mechanism
  formalized in Milestone 2 against a required `observed_current:
  WindowRevision` (not `Option`, since a correction only makes sense
  against an already-published window). It does **not** identify which
  windows a correction touches from event time — that needs an aggregation
  engine this crate deliberately does not link (`src/lib.rs`), and is
  tracked separately as #16, not assigned to a milestone in this roadmap.
- **Test:** narrowed from plan line 634's literal wording — see #16 for why
  "equals a clean rebuild from the corrected source" isn't provable inside
  this crate. `coordinator_correction_is_revision_isolated_from_a_from_scratch_publish`
  proves **revision isolation survives through the coordinator**: the
  isolation property itself (publishing revision N+1 fully replaces what a
  reader sees of revision N) is not new — `tests/phase3.rs`'s
  `publishing_a_new_revision_replaces_visibility_of_the_prior_one` (line
  225) already proves it at the raw claim/append/publish protocol level;
  this test's additional content is that the property holds when the
  corrected revision is produced through `CorrectionCoordinator`
  specifically, checked via a cross-fixture domain-row comparison, not just
  a row count.
  `coordinator_rejects_a_correction_planned_against_a_superseded_revision_and_does_not_move_current`
  proves the coordinator surfaces `StaleRevision` as-is with no internal
  retry (plan's "losing writers do not republish automatically without
  rereading source and current state") and does not move `current` on
  rejection.
  `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
  proves the append-skip branch for an already-`Appended` run actually
  skips a second append (checked via the returned `Publication`'s
  `aggregate_snapshot_id` matching the first append's snapshot, not a
  duplicated commit) rather than merely compiling — this is an
  idempotent-retry check, not a crash-recovery/reconciliation one (that's
  Milestone 5's scope, and `execute` has no failure-injection seam yet for
  it — see Milestone 5 below).
- **Depends on:** Milestone 2 (publishes via expected-revision CAS),
  Milestone 3 (needs a `CorrectionPlan` to execute).
- **Done when:** the coordinator can run a correction end-to-end for at
  least one aggregate kind and the rebuild-equality test passes. **Met, for
  the narrowed scope** — decision and full reasoning recorded in
  `coordinator.rs`'s module doc comment, full step-4 battery clean
  (`docs/TIMESERIES_PHASE_11_HANDOFF.md`). The literal "identifies affected
  windows from event time" and "equals a clean rebuild from the corrected
  source" halves are deferred to #16, not met by this milestone.

### Milestone 5 — `ReconciliationRecord` + failure-injection recoverability

- **Status:** Done (narrowed) — `docs/TIMESERIES_PHASE_12_HANDOFF.md`.
- **Target:** new type in `crates/cubism-iceberg/src/coordinator.rs`
  (`ReconciliationRecord`); `CorrectionCoordinator::execute` updated to
  consult it.
- **What it does:** (Corrected: the original wording assumed
  `ReconciliationRecord` needed to be a new persisted record and that
  `execute` needed a new per-stage injection seam. Neither turned out to be
  true — see `coordinator.rs`'s module doc comment for the full reasoning.)
  `ReconciliationRecord::classify` is a pure projection of the durable
  `RunState` the control store already tracks (`Claimed`/`Appended`/
  `Published`) into "what should happen next": `NotStarted`,
  `AwaitingAppend`, `AwaitingPublish`, or `Published`. `execute` now
  branches on it (replacing its earlier raw `RunState::Claimed` match)
  instead of leaving the type unused. No new injection seam was needed:
  `tests/coordinator.rs`'s existing
  `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
  already demonstrated the technique that generalizes to every stage —
  build the intermediate `RunState` directly via claim/append/record calls
  outside the coordinator, then call `execute` and assert on recovery —
  no per-stage hooks or resumable-step refactor required.
- **What it does not do:** `AwaitingAppend` is ambiguous between "the append
  was never attempted" and "the Iceberg append committed but the process
  crashed before `record_append` persisted that fact" — the latter is not
  safely recoverable by re-running `execute` (`fast_append` has no
  cross-commit idempotency check, and `read_window` filters by
  `(window_id, revision)`, not snapshot ID), so a naive retry can duplicate
  visible rows. This is a real, currently-unresolved gap, not fixed by this
  milestone — filed as
  [#17](https://github.com/jeromebanks/cubism-rs/issues/17). Plan line 638's
  literal "failure injection at every commit/publication stage is
  recoverable" is therefore met for three of the four interruption points a
  correction can crash at (claimed-and-unattempted, appended-and-recorded,
  published), not all four. **Resolved by Milestone 5b below** — this
  bullet is left as-is (this series' additive-pointer convention) rather
  than rewritten in place.
- **Test:** narrowed from plan line 638's literal "every stage" — see #17
  for the one stage this doesn't cover.
  `sqlite_coordinator_execute_recovers_an_unattempted_claim_then_replays_a_published_run_after_reopen`
  (`crates/cubism-iceberg/tests/durability.rs`) proves recovery from
  `AwaitingAppend` in its unambiguous form (a claim that never attempted an
  append) and from `Published` (replaying a completed correction after a
  restart returns the identical `Publication`, no duplicate rows) — both
  across real process restarts (fresh catalog and control-store handles per
  leg, this file's own convention), not just in-process retries.
  `AwaitingPublish` recovery is **not** re-proven here — it cites
  `tests/coordinator.rs`'s existing
  `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
  instead of duplicating that coverage. `ReconciliationRecord::classify`
  itself is unit-tested directly in `coordinator.rs`'s own `mod tests`
  (all four variants, mirroring `control.rs`'s existing unit-test
  convention for pure classification logic).
- **Depends on:** Milestone 4 (needs the coordinator's stages to classify
  and recover). Met.
- **Done when:** `ReconciliationRecord` exists and the failure-injection
  test passes for every *recoverable* stage the coordinator has. **Met, for
  the narrowed scope** — the one non-recoverable stage is #17, not silently
  assumed away.

### Milestone 6 — Public correction API

- **Status:** Done (narrowed) — `docs/TIMESERIES_PHASE_14_HANDOFF.md`.
- **Target:** public API surface in `crates/cubism-iceberg/src` (library
  level — plan's "Public API changes"; a CLI surface for this is explicitly
  #11's scope, not this milestone's).
- **What it does:** (Corrected: the original wording bundled two things —
  "submit/schedule a correction by source checkpoint or time range" and
  "inspect current/superseded revisions and reconciliation state." Only the
  second is built. The submit/schedule half needs the same event-time-to-
  window mapping Milestone 4 already found this crate cannot do without an
  aggregation engine — narrowed to "caller-identified window" the way
  `CorrectionCoordinator::execute` already is, it would be a zero-behavior
  wrapper over `execute`, the same untested-scaffolding refusal Milestones 3
  and 5 already made. Recorded as a non-goal, cross-linked to
  [#16](https://github.com/jeromebanks/cubism-rs/issues/16), not filed as a
  new issue.) `RunInspection::inspect`
  (`crates/cubism-iceberg/src/coordinator.rs`) pairs
  `ReconciliationRecord::classify` (Milestone 5, pure, no I/O) with a live
  read of `PublicationStore::current`, returning `RevisionStatus::Current`
  or `RevisionStatus::NotCurrent`. This is
  [#18](https://github.com/jeromebanks/cubism-rs/issues/18)'s option 1,
  implemented: a `Published` run's `RunState` alone cannot say whether its
  revision is still current post-rollback (`docs/TIMESERIES_PHASE_13_HANDOFF.md`),
  so inspection cross-checks `current` live instead of trusting the cached
  stage. Deliberately not phrased as "superseded/not superseded" — after a
  rollback, a not-current revision can be numerically *higher* than
  current, so "superseded" would misdescribe it; see `RevisionStatus`'s doc
  comment.
- **Test:** (Corrected: narrowed from "submit → coordinator runs → inspect"
  since submit is a non-goal — see above.)
  `run_inspection_pairs_reconciliation_stage_with_a_live_current_check`
  (`crates/cubism-iceberg/src/coordinator.rs`, unit, in-memory store) proves
  the pairing itself: no revision status before publish, `Current` for the
  run holding `current`, `NotCurrent` for a run rolled back past, replaying
  Phase 13's rollback shape.
  `run_inspection_distinguishes_the_rolled_back_to_run_from_the_rolled_back_past_run`
  (`crates/cubism-iceberg/tests/durability.rs`) proves the same distinction
  through the **durable SQLite backend**, from a fresh handle.
- **Depends on:** Milestones 3, 4, 5. Met.
- **Done when:** the public API is callable end-to-end with a passing
  integration test. **Met, for the narrowed (inspection-only) scope** — see
  "Phase 4 done" below for why this does **not** by itself close Phase 4's
  #13 scope; the plan's four completion criteria are walked individually
  there rather than assumed met from "Milestone 6 done."

### Milestone 5b — Resolve #17: `AwaitingAppend` recovery via a states-table idempotency check

- **Status:** Done. Fixes
  [#17](https://github.com/jeromebanks/cubism-rs/issues/17) — the one gap
  Milestone 5 left open in its own "What it does not do."
- **Target:** `AggregateReader::run_append_snapshot`, a new function in
  `crates/cubism-iceberg/src/reader.rs`;
  `CorrectionCoordinator::execute`'s `AwaitingAppend` branch
  (`crates/cubism-iceberg/src/coordinator.rs`); `cubism-cli`'s
  `iceberg_build` command (`crates/cubism-cli/src/main.rs`) — see
  "Two recovery paths, not one" below for why the CLI is in scope too.
- **The key finding that shrank this below what it might have needed:**
  every states row `AggregateWriter::append_window` writes already carries
  its own `window_id`/`revision`/`run_id` columns
  (`writer.rs`'s `augment_states_batch`) — the exact columns needed to ask
  "did this run's append already commit?" directly against Iceberg's own
  state. `AggregateReader::read_window` already showed the predicate-scan
  pattern to copy (`(window_id, revision)` via
  `Table::scan().with_filter(...)`); `run_append_snapshot` is the same
  pattern plus a `run_id` clause. No schema change, no new persisted
  control-store field, no migration.
- **What it does:** before `CorrectionCoordinator::execute` (or
  `cubism-cli`'s `iceberg_build`) re-runs `AggregateWriter::append_window`
  for a claim classified `AwaitingAppend`, it now calls
  `AggregateReader::run_append_snapshot` to check whether a states row for
  the exact `(window_id, revision, run_id)` already exists in the table's
  *current* snapshot. If found, the append already committed — the caller
  records that snapshot id via `record_append` and skips re-appending. If
  not found, it appends exactly as before. The returned snapshot id is the
  table's current snapshot at scan time, not necessarily the exact one the
  original (possibly crashed) commit produced — `aggregate_snapshot_id` is
  provenance-only, never used to filter a read (checked directly in
  `control.rs`), so this is sufficient; the function's own doc comment says
  so explicitly rather than implying more precision than it has.
- **Two recovery paths, not one — checked, not assumed.** #17 was written
  against `CorrectionCoordinator::execute`, but that function is called
  nowhere outside its own tests today (confirmed: grepped every crate).
  `cubism-cli`'s `iceberg_build` command hand-rolls the identical
  claim/append/record protocol independently (it predates
  `CorrectionCoordinator` — Phase 3) and had the *same* unfixed gap: its
  own `already_appended` check only recognized `Appended`/`Published`,
  treating every `Claimed` state as "never attempted." Since
  `iceberg_build` is the only one of the two paths any real invocation of
  this crate goes through today, fixing only `execute` would have closed
  the issue on paper while leaving the actually-reachable bug in place.
  Both are fixed the same way, sharing `run_append_snapshot`.
- **Explicit non-goal, not silently assumed:** no defense against
  concurrent recovery of the *same* run — two callers racing `execute` (or
  two `iceberg_build` invocations) for the identical `run_id` could both
  observe "not yet committed" before either commits, and both then append.
  This fix closes the *sequential* crash-then-retry gap #17 describes;
  concurrent recovery of one run was never in #17's scope and is not
  solved here. Stated in `run_append_snapshot`'s own doc comment.
- **`expected_rows` checked, not assumed safe:** `RunState::expected_rows`
  is only ever cross-checked at `claim_run` time, against a *retried
  claim's own* `expected_rows` argument (`control.rs`'s `RunConflict`).
  Nothing in `record_append` or `publish` compares `expected_rows` against
  the actual row count Iceberg committed, on this path or the pre-existing
  normal append path either — this fix does not weaken an existing
  invariant, because none exists to weaken. Recorded in
  `run_append_snapshot`'s own doc comment, not left as an unexamined gap.
- **Test:** `sqlite_coordinator_execute_recovers_an_appended_but_unrecorded_claim_after_reopen`
  (`crates/cubism-iceberg/tests/durability.rs`) — the ambiguous half of
  `AwaitingAppend` Milestone 5's own test suite deliberately did not cover.
  Simulates the crash by calling `AggregateWriter::append_window` directly
  (a real Iceberg commit) and never calling `record_append`, then a fresh
  handle calls `CorrectionCoordinator::execute` with the identical request.
  Proves the fix at the **snapshot** level: the states table's current
  snapshot id after recovery is asserted equal to the id the direct
  `append_window` call produced — stronger than a row-count check alone,
  since any further commit (duplicate or not) would produce a new snapshot
  id, where a row count could in principle round-trip a coincidence. Row
  count is asserted too, as corroboration. The pre-existing
  `sqlite_coordinator_execute_recovers_an_unattempted_claim_then_replays_a_published_run_after_reopen`
  (the unambiguous half) re-ran clean, confirming no regression to the
  `None`-branch (append-for-real) path this fix's `else` arm shares with
  the original code. A second test,
  `run_append_snapshot_returns_none_against_a_table_with_no_commits_at_all`
  (same file), proves `run_append_snapshot` doesn't error against a states
  table with zero commits at all — the shape `iceberg_build` reaches on a
  brand-new cube's first-ever build, which the crash-recovery test above
  (a table with at least one prior commit) does not exercise; found and
  added on the advisor's second pass over this slice, not the original
  design. `cubism-cli` has no test harness at all (no `tests/`
  directory, zero `#[test]`s in `main.rs`) — this crate's own established
  gate is `cargo build -p cubism-cli` + `cargo clippy`, unchanged by this
  slice; the CLI's fix is a direct, mechanical reuse of the same
  already-tested `run_append_snapshot` helper and branching shape
  `CorrectionCoordinator::execute` uses, not new logic of its own.
- **Depends on:** Milestone 5 (`Done`, narrowed — this closes the one gap
  it left open).
- **Done when:** the test passes and the full step-4 battery is clean.
  Met (35 passed + 1 ignored in `cubism-iceberg`, up from 33 — the
  crash-recovery test above plus
  `run_append_snapshot_returns_none_against_a_table_with_no_commits_at_all`,
  added on the advisor's second pass to cover the brand-new-cube,
  zero-commits case `run_append_snapshot` must also handle; full battery
  clean). See "Phase 4 done condition" below, criterion 2, for the updated
  verdict — met for **both** recovery paths this crate has, not narrowed
  to one.

## Phase 4 "done" condition

Distinct from "every milestone above is done," per plan lines 650-669:

- All eight plan Phase 4 test-list items (lines 634-641) pass — six covered
  by Milestones 1-6 above (634, 635, 638, 639 plus 636/637 already closed);
  the remaining two (640 compaction, 641 retention) are #10's scope, not
  this roadmap's.
- Plan's four completion criteria (lines 651-655), walked individually now
  that Milestone 6 (narrowed) is done — not assumed met from "milestones
  done, therefore criteria met":
  1. *"Late data produces a full, atomically published replacement."* Met —
     `CorrectionCoordinator::execute` (Milestone 4) appends under the
     requested revision and publishes via CAS; `tests/phase3.rs`'s
     `publishing_a_new_revision_replaces_visibility_of_the_prior_one` and
     Milestone 4's own revision-isolation test both prove the replacement
     is atomic and complete from the reader's view.
  2. *"A deterministic recovery run classifies and reconciles every
     interrupted job."* **Met.** Milestone 5 (narrowed) recovered three of
     the four stages a correction can be interrupted at; Milestone 5b
     resolved the fourth (`AwaitingAppend`'s ambiguous case — append
     committed but not yet recorded), fixing
     [#17](https://github.com/jeromebanks/cubism-rs/issues/17). Checked
     against **both** of this crate's real recovery paths, not just
     `CorrectionCoordinator::execute`: `cubism-cli`'s `iceberg_build`
     hand-rolls the identical claim/append/record protocol independently
     and had the same gap, unfixed by Milestone 5 alone — Milestone 5b's
     own entry records why that mattered (`execute` has zero real callers
     today; `iceberg_build` is the only path anything actually invokes).
     Both now share `AggregateReader::run_append_snapshot`. Concurrent
     recovery of the *same* run (as opposed to sequential crash-then-retry)
     remains explicitly out of scope — see Milestone 5b's own "Explicit
     non-goal" bullet — but "every interrupted job" in the plan's wording
     is about a job being interrupted and later recovered, not about two
     recovery attempts racing each other, so this does not reopen the
     criterion.
  3. *"Compaction and retention SLOs are documented and observable."*
     **Not** met — no compaction or retention exists in this roadmap's
     scope at all, by design (this roadmap's "Scope" section above excludes
     it, tracked entirely in
     [#10](https://github.com/jeromebanks/cubism-rs/issues/10)).
  4. *"No maintenance path changes aggregate answers."* Not applicable yet
     — there is no maintenance path (compaction/retention) to check against,
     for the same reason as (3).
  Net: criteria 1 and 2 are both fully met (2 as of Milestone 5b); 3 and 4
  are out of this roadmap's scope entirely (#10). **Phase 4 as this
  roadmap defines it (excluding #10, the same "excluded, not silently
  dropped" treatment used throughout this doc) is done as of Milestone
  5b.** Per this series' step 8a
  (`.claude/skills/timeseries-slice/SKILL.md`), landing Milestone 5b
  triggers the cross-model phase review — see
  [`docs/phase-reviews/TIMESERIES_PHASE_4_REVIEW.md`](phase-reviews/TIMESERIES_PHASE_4_REVIEW.md)
  for that review's findings and disposition. **Phase start (for step
  8a):** `a253f4f` — the commit immediately before `1836d50` added this
  roadmap doc's first "## Milestones" section (Milestone 1), the same
  backfill convention Phase 5's own start (`f0599b2`) used.
- Plan's "Unresolved decisions" (lines 658-663) are each either resolved (as
  Milestones 1 and 2 do for two of them) or explicitly deferred with a
  reason, not silently dropped.
- Plan's "Rollback point" (lines 666-669) — repointing a window to its prior
  published revision — is implemented and tested. (Corrected: this
  originally asked whichever slice implements Milestone 6's public API to
  determine whether rollback falls out of the existing CAS/publish
  mechanism or needs its own milestone; it does fall out of the existing
  mechanism, confirmed empirically, not just read, in
  `docs/TIMESERIES_PHASE_13_HANDOFF.md` — `PublicationStore::publish` has no
  revision-monotonicity check, so re-publishing a prior run's own `run_id`
  against a fresh `expected_current` repoints `control_publications`
  backward, reader-visibly, with zero new source code.) Only the
  *repointing* third of the plan's three-part wording is met — "stop
  correction scheduling" (no scheduler exists in this crate) and "retain a
  configured recovery window" before expiring superseded snapshots (no
  snapshot expiry/retention exists — [#10](https://github.com/jeromebanks/cubism-rs/issues/10)'s
  scope) are not. The rollback also leaves the superseded run's own
  `RunState` reporting `Published` (`ReconciliationRecord::classify` cannot
  tell it's now stale) — tracked in
  [#18](https://github.com/jeromebanks/cubism-rs/issues/18). (Corrected:
  #18 is resolved as of `docs/TIMESERIES_PHASE_14_HANDOFF.md` — Milestone
  6's `RunInspection::inspect` cross-checks a `Published` run's revision
  against live `PublicationStore::current` rather than trusting the cached
  `RunState` stage, #18's own suggested option 1. The raw `RunState` row
  itself is still unchanged by rollback, as expected; the gap #18 tracked
  was that nothing *answered* the "is this still current" question
  correctly, and now something does.)
- Plan line 634's literal wording ("a late-event rebuild equals a clean
  rebuild from the corrected source") and "identifies affected windows from
  event time" (plan's Phase 4 "Types and modules" section) are **not** met
  by Milestone 4 as narrowed — both need an aggregation-engine link this
  roadmap's home crate (`cubism-iceberg`) deliberately does not have. Not
  currently assigned to a milestone above; tracked in
  [#16](https://github.com/jeromebanks/cubism-rs/issues/16), which needs a
  decision on which crate closes the gap before it can become a milestone
  here or in a successor roadmap doc.
- Plan line 638's literal "failure injection at every commit/publication
  stage is recoverable" is now **met**: a correction whose Iceberg append
  committed but crashed before `record_append` persisted that fact is
  safely recoverable by `CorrectionCoordinator::execute` (and, as
  Milestone 5b's own entry records, `cubism-cli`'s `iceberg_build`) as of
  Milestone 5b, which fixed
  [#17](https://github.com/jeromebanks/cubism-rs/issues/17).

## Phase 5 Milestones

Added 2026-08-15 (`docs/TIMESERIES_PHASE_16_HANDOFF.md`), once every Phase 4
milestone above was `Done`, per #15's own suggested steps and this doc's
former "Deferred" item 1 below (now resolved). Same decomposition
convention as Phase 4's milestones: target file(s), the one test it adds,
its dependencies, its done-condition. **Doc-only when added** — no Phase 5
milestone below is implemented yet; every "Status" is `Not started` unless
noted.

**Phase start (for `.claude/skills/timeseries-slice/SKILL.md` step 8a's
cross-model phase review, backfilled 2026-08-16, not used retroactively —
Phase 4 predates step 8a):** `f0599b2`, the commit immediately before this
section was added by `4299d60`. When Phase 5's "done" condition below is
actually reached, step 8a diffs `f0599b2..HEAD` for that review, not just
whatever this session's own commits touch.

### Why this section exists despite #8

[#8](https://github.com/jeromebanks/cubism-rs/issues/8) says Phase 5
"cannot proceed" until `cubism-datafusion` (DataFusion 54) and
`iceberg-datafusion` 0.10 (DataFusion 53) converge, because their
`SessionContext`/`TableProvider`/`ExecutionPlan` types aren't
interchangeable. Read closely (`crates/cubism-iceberg/src/lib.rs`'s
top-level doc comment, `crates/cubism-iceberg/src/reader.rs`'s
`read_window` doc comment, `crates/cubism-datafusion/src/state_udaf.rs`),
that claim is broader than the evidence: it's true for *generic SQL-level*
access — registering this crate's Iceberg tables in a DataFusion
`SessionContext` via `iceberg-datafusion`'s `TableProvider` impl so
arbitrary predicate pushdown/joins work through the DF53 crate. It is not
true for calling `AggregateReader::read_window` (`crates/cubism-iceberg/src/reader.rs:46`)
directly from `cubism-datafusion` (a plain path dependency, no
`iceberg-datafusion` involved) and merging the resulting `RecordBatch`es
with the merge machinery `state_udaf.rs` already has
(`AggregateState::decode`+`merge`, `crates/cubism-core/src/aggregate_state.rs:169`)
— the same "cross the crate boundary via `arrow_array::RecordBatch`/
`arrow_schema::Schema` only" strategy `cubism-iceberg`'s own top-level doc
comment already describes for Phase 3. Confirmed, not assumed:
`cargo tree -i arrow --workspace` resolves a single unified `arrow` 58.3.0
across both the DF53 and DF54 dependency trees, and `cargo tree -p
cubism-iceberg` resolves the same `arrow-array`/`arrow-schema` 58.3.0 — so
a `RecordBatch` from `read_window` is already the type DF54's
`cubism-datafusion` expects, with no adapter needed. `crates/cubism-iceberg/src/reader.rs`'s
doc comment carried the same overstated framing as #8 (a `read_window`
call site describing range queries as "gated on DataFusion 53/54
convergence") — corrected in place there, additively, this session.

Commented on #8 with this narrowing (read-not-built, matching how the
`return Err(err)` branch was labeled in
`docs/TIMESERIES_PHASE_15_HANDOFF.md`) rather than closing or rewriting it:
#8 stays open and load-bearing for the SQL/pushdown half (see Milestone
10's "Depends on" and completion-criterion 774 below), narrowed rather than
resolved.

Milestone 7 below converts this from a reasoned-from-`Cargo.toml` premise
into an observed one before Milestones 8-10 build on it.

### Milestone 7 — Spike: `RecordBatch` from `cubism-iceberg` into a DF54 `SessionContext`

- **Status:** Done — `docs/TIMESERIES_PHASE_17_HANDOFF.md`. The spike
  landed as a `[dev-dependencies]` entry, not the plain `[dependencies]`
  this milestone's text originally called for (advisor-flagged deviation,
  see that handoff): Milestone 7's only consumer is
  `tests/iceberg_bridge.rs`, and a regular dependency would have
  permanently pulled `iceberg`/`sqlx`/`iceberg-catalog-sql` into
  `cubism-datafusion`'s shipped dependency graph to serve a spike.
  Milestone 10 promotes it to a normal dependency in one line when it
  needs `cubism-iceberg` from `src/`.

  **Correction (Milestone 10's entry below):** Milestone 10 landed without
  this promotion — `CoveragePlan` stays pure/synchronous and never calls
  `cubism-iceberg` from `src/`, so `cubism-iceberg` is still a
  dev-dependency only. The promotion this paragraph describes is now
  "Milestone 10b"'s, when value materialization needs `read_window`.
- **Target:** new integration test in `crates/cubism-datafusion` (e.g.
  `tests/iceberg_bridge.rs`), plus adding `cubism-iceberg` as a plain path
  dependency of `cubism-datafusion`'s `Cargo.toml` (not currently a
  dependency in either direction, confirmed via `cargo tree`).
- **What it does:** publishes one window via `cubism-iceberg` (in-memory
  catalog/control store, same pattern `tests/phase3.rs` already uses), then
  from `cubism-datafusion` calls `AggregateReader::read_window` and hands
  the resulting `Vec<RecordBatch>` to a `datafusion::execution::context::SessionContext`
  (`SessionContext::new().read_batches(...)` or equivalent) with **no**
  `iceberg-datafusion` dependency anywhere in the call path. Purely a
  premise check — no query planning, merging, or `TemporalQuery` type yet.
- **Test:** one integration test asserting the batch round-trips through a
  DF54 `SessionContext` (e.g. a trivial `SELECT count(*)` over it) without
  a version-seam type error at compile or run time.
- **Depends on:** nothing new (Milestone 6 and everything in "Current
  state" above).
- **Done when:** the test passes and the full step-4 battery is clean. If
  it does **not** pass — the "Why this section exists despite #8" reasoning
  was wrong somewhere `cargo tree` didn't catch (e.g. a runtime ABI
  mismatch `cargo tree`'s static resolution can't see) — this milestone
  becomes "resolve the seam, or build the Arrow/service-boundary fallback
  #8's own suggested next steps propose," and Milestones 8-10 stay blocked
  behind it rather than proceeding on a false premise.

### Milestone 8 — `TemporalQuery` (request shape + validation)

- **Status:** Done — `docs/TIMESERIES_PHASE_18_HANDOFF.md`. Landed as
  `crates/cubism-datafusion/src/range_query.rs:97-106`
  (`TemporalQuery`) and `:85-90` (`GapPolicy`), with `TemporalQuery::new`
  at `:114-150` (re-verified and corrected this session — Milestone 9's own
  module-doc-comment addition shifted every line below it, which had gone
  unnoticed until this pass; the original citations, `:60-69`/`:48-53`/
  `:77-114`, were accurate as of Phase 18 but are stale now). Two
  deviations from this milestone's original text,
  both advisor-confirmed before writing: (1) the struct holds `cube:
  String` (an identifier), not the `cube/spec` `TemporalSpec` itself —
  Milestone 9's own text takes a `TemporalQuery` **and** a `TemporalSpec`
  as separate arguments, so storing the spec here would make that
  signature redundant; `TemporalSpec` is instead a `&TemporalSpec`
  argument to `new()` that validates but isn't retained. (2) `timezone`/
  display options from the plan's request shape are omitted entirely —
  `TemporalSpec::validate` (`crates/cubism-core/src/temporal.rs:619-624`)
  hard-rejects any non-`"UTC"` spec, so a free-form timezone request
  field would be unimplementable without re-deriving that check, which is
  out of this milestone's scope. Measures/selectors are not validated
  against a `CubeSpec` (no `CubeSpec` is threaded through this module at
  all) — not in this milestone's "e.g." validation list.
- **Target:** `crates/cubism-datafusion/src/range_query.rs` (new file, per
  plan line 677).
- **What it does:** the plan's requested-query shape (lines 697-706) as a
  typed struct — cube/spec, `XUnit` selector(s) (`crates/cubism-core/src/ypath.rs:74`),
  measure(s), `[start, end)`, requested/auto `Resolution`
  (`crates/cubism-core/src/temporal.rs:331`), `exact: bool`, gap policy,
  timezone/display options — with construction-time validation (e.g.
  `start < end`, requested resolution is one the cube's `TemporalSpec`
  actually supports). Pure request-shape logic; touches no DataFusion
  execution types, so it doesn't depend on Milestone 7.
- **Test:** unit tests for valid construction plus at least one rejection
  case (`start >= end`; an unsupported resolution for the cube). Landed as
  four tests, `range_query.rs:318-390`: two valid-construction cases
  (explicit supported resolution, and `None`/auto) and the two required
  rejections.
- **Depends on:** nothing new.
- **Done when:** the type exists, is unit-tested, and the full step-4
  battery is clean.

### Milestone 9 — `ResolutionPlan` (non-overlapping segment selection)

- **Status:** Done — `docs/TIMESERIES_PHASE_19_HANDOFF.md`. Landed as
  `crates/cubism-datafusion/src/range_query.rs:160-163`
  (`ResolutionSegment`), `:170-173` (`ResolutionPlan`), `ResolutionPlan::new`
  at `:179-192`, `as_fixed` at `:197-206`, `auto_select_resolution` at
  `:217-232`, `decompose` at `:238-290`. One deviation from this milestone's
  original text, advisor-confirmed before writing: this milestone picks
  **one** resolution per plan (the query's `Some`, or an auto-selected one
  for `None`) and decomposes `[start, end)` into at most 3 contiguous
  segments (partial head / aligned interior / partial tail) at that single
  resolution — it does **not** build a cross-resolution cover (e.g. serving
  the interior from a coarser rollup and the edges from the base
  resolution). Plan line 741 ("resolution choice never overlaps or
  double-counts") reads as inviting the cross-resolution version, but that
  depends on the divisibility/coarseness guarantee between a spec's rollups
  and base resolution that only `TemporalSpec::validate` enforces
  (`crates/cubism-core/src/temporal.rs:634-649`) and that this constructor
  does not call — building it without that guarantee would be a false
  premise. Deferred, not filed as a separate issue (see #15's role as this
  series' Phase-4+ tracker) — revisit once a caller actually needs
  multi-resolution serving. Non-overlap/coverage for the single-resolution
  case follow from the segments being built directly off one shared
  `interior_start`/`interior_end` boundary pair, proven by
  `assert_exact_cover` in each of the five new `ResolutionPlan` tests that
  construct a plan (the two Calendar-rejection tests below construct no
  plan to check) — three checks per call: first segment starts at the
  range start, last segment ends at the range end, and each adjacent pair
  is exactly contiguous. `Resolution::Calendar` is rejected explicitly (no
  calendar equivalent of `FixedResolution::bucket` exists) at both the
  explicit-selection and auto-selection paths, each with its own test
  (`resolution_plan_rejects_calendar_resolution`,
  `resolution_plan_auto_select_rejects_calendar_base` — the latter's name
  doesn't mention "no fixed rollup": `auto_select_resolution` rejects a
  Calendar base via `as_fixed` before it ever inspects rollups, so an empty
  rollup list isn't what triggers the rejection).
  Auto-selection's rule (not dictated by the plan text) is recorded here:
  the coarsest `Fixed` candidate (base or a rollup) whose width fits within
  the query's duration, falling back to the base resolution when no rollup
  fits.
- **Target:** same file as Milestone 8.
- **What it does:** given a `TemporalQuery` and a cube's `TemporalSpec`,
  choose non-overlapping resolution segments covering `[start, end)` —
  plan line 740 ("range decomposition for aligned and unaligned
  boundaries") and line 741 ("resolution choice never overlaps or
  double-counts"). Pure computation over window boundaries; still no
  DataFusion execution types.
- **Test:** unit/property tests over synthetic boundary sets — an
  interval aligned to bucket edges, one that isn't, and an assertion that
  the chosen segments never overlap or leave a gap inside `[start, end)`.
  Landed as seven tests plus two shared helpers, `range_query.rs:392-524`
  (`query_with`, `assert_exact_cover`, and the tests at `:429-524`): aligned interval (one
  segment), unaligned interval (head/interior/tail), a range narrower than
  one bucket (single partial segment, no interior), auto-selection choosing
  the coarsest fitting rollup, auto-selection falling back to the base
  resolution, and both Calendar-rejection cases above (7 total).
- **Depends on:** Milestone 8.
- **Done when:** the test passes for both aligned and unaligned boundaries
  and the full step-4 battery is clean.

### Milestone 10 — `CoveragePlan` against real published windows

- **Status:** Done (narrowed in place upon landing, same convention prior
  milestones used). Landed as `CoveragePlan`/`SegmentCoverage`
  (`crates/cubism-datafusion/src/range_query.rs:356-461`), covering
  provenance/`missing`/`exact=true` failure only — **not**
  `SeriesResponse` (deferred as "Milestone 10b" below); title above kept
  as `CoveragePlan` alone to match what actually landed.
- **Target:** same file as Milestones 8-9. Does **not** consume
  `AggregateReader::read_window` or `PublicationStore::current` directly —
  see deviations below.
- **Deviations from the original text above** (advisor-confirmed before
  writing code, same precedent Milestones 8-9 set for their own
  deviations):
  - **`cubism-iceberg` was not promoted to a normal `cubism-datafusion`
    dependency.** It stays a dev-dependency (`crates/cubism-datafusion/Cargo.toml`).
    `CoveragePlan::new` is pure, synchronous computation — it does not call
    `read_window`/`current` itself. Those async calls happen in the
    caller (this milestone's own integration test), which resolves each
    segment's backing windows and hands `CoveragePlan::new` the results as
    `&[Vec<(WindowId, Option<WindowRevision>)>]`, one list per segment.
    Checked via `cargo tree`/reading both `Cargo.toml`s directly: no
    dependency cycle exists either direction, so promotion was possible,
    just not necessary for this milestone's scope.
  - **The segment→`WindowId` mapping is a caller-supplied input, not
    derived by this module.** No canonical encoding from a
    `TimeRange`/`Resolution` pair to a `WindowId` exists anywhere in this
    codebase (`WindowId` is an opaque caller-assigned string,
    `crates/cubism-core/src/temporal.rs:527-529`); inventing one here would
    be a durable `cubism-core` API decision smuggled into this milestone.
    A single segment can legitimately be backed by more than one window
    (Milestone 9's aligned interior segment stays one segment even when it
    spans many resolution buckets) — `CoveragePlan::new` handles that
    directly via the per-segment `Vec`.
  - **No value materialization.** `AggregateState::decode`/`merge`/
    `state_udaf.rs` are not exercised: this milestone's own **Test** bullet
    below (unchanged from the original text) asserts only provenance, the
    `missing` marker, and the `exact=true` failure — no numeric value.
    Deferred as **"Milestone 10b"**, not yet added to this roadmap as its
    own entry: build `SeriesResponse`'s value/presentation fields on top
    of the `CoveragePlan` this milestone lands, calling `read_window` and
    merging via `state_udaf.rs` for each `published` window a
    `SegmentCoverage` reports.

    **Correction (Milestone 10b-1's entry below):** "merging via
    `state_udaf.rs`" was wrong — `state_udaf.rs` is DataFusion `UDAF`
    machinery, not a plain callable merge function; the actual reusable API
    is `cubism_core::aggregate_state::AggregateState`'s `decode`/`merge`.
    "Milestone 10b" also turned out not to be one bounded slice: split into
    **10b-1** (the merge primitive alone, `Done`) and **10b-2** (the
    `SeriesResponse` type plus wiring it to `CoveragePlan`, `Done` — see
    its own entry below).
  - **`is_exact` is stricter than "backed by a current published
    revision."** A segment is exact iff it is **both**
    resolution-aligned (`ResolutionSegment::aligned`) **and** every
    window backing it is published — an unaligned (partial head/tail)
    segment is never exact, even fully published, because a published
    window's aggregate state is bucket-granularity and cannot exactly
    answer a sub-bucket range without a raw-event scan (not implemented).
    This follows directly from plan lines 721-722 ("fails clearly if
    retained buckets/raw data cannot exactly cover a partial boundary"),
    not from the "backed by a current published revision" phrasing alone.
- **What it does:** for each segment in a `ResolutionPlan`, given the
  caller-resolved publication state of its backing window(s), reports
  which windows are published (with `WindowRevision` provenance) and
  which are `missing`, and computes `is_exact` per the stricter rule
  above. `CoveragePlan::new` fails with `CubismError::Temporal`, naming
  every offending segment, when `exact: true` and any segment is not
  exact — plan lines 721-722's behavior, this milestone's explicit test.
- **Test:** integration test (real `cubism-iceberg` in-memory catalog,
  `crates/cubism-datafusion/tests/iceberg_bridge.rs:213`
  `coverage_plan_resolves_real_publication_state_across_two_windows`):
  publish window W1 (current revision, exact) at day resolution, leave W2
  (the adjacent day) unpublished; build a `ResolutionPlan` spanning both
  (kept as one aligned interior segment per Milestone 9); resolve both
  windows' real `PublicationStore::current` and feed them into
  `CoveragePlan::new`; assert the segment's `published` list contains W1
  with its real `WindowRevision` and `missing` contains W2; assert
  `exact: true` fails clearly (error message contains `"exact=true"`)
  instead of silently rounding or omitting W2. Six additional pure unit
  tests in `range_query.rs`'s own test module cover the aligned+published,
  aligned+missing, unaligned+published (the `is_exact` stricter-rule
  case), mixed-within-one-segment, and windows-length-mismatch shapes
  without the async iceberg harness.
- **Depends on:** Milestones 7, 8, 9 — all `Done` before this slice began.
- **Done when:** the test passes and the full step-4 battery is clean. Met
  (49 passed in `cubism-datafusion`, up from 42; full battery clean).
  **Does not close** plan completion-criterion 774 ("range plans prune
  storage and stay within latency/memory budgets") — `read_window`'s
  predicate is a single `(window_id, revision)` equality per call (see
  `reader.rs`'s corrected doc comment), not a semijoin against every
  published window's manifest the way a real `TableProvider` scan would
  prune; achieving 774 for many windows still needs #8's SQL/pushdown
  half, or a purpose-built multi-window batch read added to
  `cubism-iceberg` itself (not scoped to this milestone). Record that gap
  here rather than treating "774 met" as implied by "710-719 met." Also
  does not close plan-line-708 "value/presentation" or
  plan-line-716 "state/error metadata" — those are Milestone 10b's, per
  the deviations above.

### Milestone 10b-1 — `merge_average_column`: the value-materialization primitive

- **Status:** Done. Landed as `cubism_datafusion::series_merge::merge_average_column`
  (`crates/cubism-datafusion/src/series_merge.rs:51-91`), the "merging"
  half of Milestone 10's deferred "Milestone 10b" note. Split from the
  original "Milestone 10b" text (advisor-confirmed before writing anything,
  same precedent Milestones 9-10 set for their own deviations): a
  `SeriesResponse` type carrying value/presentation (plan line 708) and
  state/error metadata (plan line 716), plus promoting `cubism-iceberg` to
  a normal `cubism-datafusion` dependency, are **not** part of this
  milestone — deferred as **"Milestone 10b-2"** (`Done`, see its own entry
  below).
- **Target:** `crates/cubism-datafusion/src/series_merge.rs` (new file).
- **Deviations from the original "Milestone 10b" note** (advisor-confirmed):
  - **Only `AverageState`, not all measure kinds.** `VarianceState`/
    `QuantileState`/the sketch-backed kinds (`CountDistinct`/`TopK`/
    `ReservoirSample`/`Centroid`) each need their own merge wiring; scoping
    this milestone to one kind matches the series' "one bounded slice"
    convention. Widening to the other kinds is left for a later slice, not
    filed as a separate issue (recorded here per the same precedent).
  - **`state_udaf.rs`'s merge machinery was not reused.** The original
    "Milestone 10b" note (Milestone 10's entry, and this section's own
    former text) said "merging via `state_udaf.rs`" — checked before
    writing code and found wrong: `state_udaf.rs` is DataFusion `UDAF`
    machinery (`merge_batch` over `ArrayRef`s inside an execution plan,
    consumed by SQL aggregation), not a plain decode+merge function callable
    outside a query plan. The actual reusable API is
    `cubism_core::aggregate_state::AggregateState`'s `decode`/`merge`
    (`crates/cubism-core/src/aggregate_state.rs:169,172`), already used by
    `state_udaf.rs` internally — `merge_average_column` calls that trait
    directly instead.
  - **No I/O; `cubism-iceberg` stays a dev-dependency.** Same split
    Milestone 10 established for `CoveragePlan`: `merge_average_column`
    takes already-read `RecordBatch`es as input and never calls
    `AggregateReader::read_window` itself. The async read stays in the
    caller (this milestone's own integration test).
  - **A previously-undocumented Arrow-type widening, found while writing
    this milestone's integration test, not by inspection alone:** the
    first version of the test failed with "column 'avg_v1' is not a Binary
    array" even though `temporal_build::temporal_state_schema` declares
    `AggKind::Avg` as `DataType::Binary`. Root cause, confirmed by reading
    `iceberg` 0.10.0's own source
    (`iceberg::arrow::schema`'s `Type`-to-Arrow conversion,
    `PrimitiveType::Binary => DataType::LargeBinary`): `AggregateReader::read_window`'s
    scan always widens a Binary column to `LargeBinary` on the way out of
    Iceberg, regardless of the schema it was written with.
    `merge_average_column` now branches on the column's actual runtime
    `DataType` (`Binary` or `LargeBinary`) rather than assuming one; a unit
    test (`merge_average_column_handles_large_binary_and_mixed_batches`)
    proves both are accepted, including in the same call. Not filed as a
    separate issue: it doesn't block anything currently built (`CoveragePlan`
    never touches this column), and it's now handled, not just discovered
    — recorded here per the series' "record scope/behavior findings in the
    roadmap entry" precedent, for whichever slice builds `SeriesResponse`
    next (Milestone 10b-2 will consume this same widening).
- **What it does:** decodes and merges every non-null `AverageState` blob
  in a named column across one or more caller-supplied `RecordBatch`es, in
  row order, into a single merged `AverageState`. Returns a zero state
  (not an error) for an empty or all-null input. Fails with
  `CubismError::AggregateState` if the column is missing, is neither
  `Binary` nor `LargeBinary`, or a non-null value fails to decode.
- **Test:** integration test (real `cubism-iceberg` in-memory catalog,
  `crates/cubism-datafusion/tests/iceberg_bridge.rs:315`
  `merge_average_column_reads_and_merges_two_published_windows`): two
  `AverageState` values are encoded into two separately published windows'
  states rows, both windows are read back via `AggregateReader::read_window`,
  and `merge_average_column` over the combined batches is asserted equal to
  merging the two source states directly in memory — proving the decode+merge
  round-trips through a real Parquet write/scan, not just in-memory logic
  (six additional pure unit tests in `series_merge.rs` cover the in-memory
  decode+merge/null-skipping/empty-input/`Binary`-vs-`LargeBinary`/
  missing-column/wrong-type-column shapes without the async iceberg
  harness).
- **Depends on:** Milestone 10 (`Done`, narrowed).
- **Done when:** the test passes and the full step-4 battery is clean. Met
  (56 passed in `cubism-datafusion`, up from 49; 196 passed / 2 ignored in
  workspace, up from 189; full battery clean). **Does not close** plan
  completion-criterion 772 by itself — see the "Phase 5 done condition"
  walk below for why a merge primitive existing is not the same as
  `CoveragePlan` actually using it. Does not build `SeriesResponse` or
  touch any measure kind besides `AverageState` — both Milestone 10b-2's,
  per the deviations above.

### Milestone 10b-2 — `SeriesResponse`: wiring `CoveragePlan` to a materialized value

- **Status:** Done. Landed as `cubism_datafusion::series_response::SeriesResponse`
  (`crates/cubism-datafusion/src/series_response.rs:86-155`), the "wiring"
  half of the roadmap's deferred "Milestone 10b" note — the half Milestone
  10b-1 (the merge primitive alone) deliberately left undone.
- **Target:** `crates/cubism-datafusion/src/series_response.rs` (new file).
- **Deviations from the original "Milestone 10b" note** (advisor-confirmed,
  same "one bounded slice" cut Milestone 10b-1 made for its own scope):
  - **Only `AverageState`**, via `merge_average_column` — no widening to
    `VarianceState`/`QuantileState`/the sketch-backed kinds. Left for a
    later slice once `SeriesResponse` needs to carry more than one measure
    kind.
  - **No `cubism-iceberg` dependency promotion; no I/O.** Same "async stays
    in the caller" split every milestone since Milestone 10 has held:
    `SeriesResponse::new` is pure/sync over caller-supplied
    `&[Vec<RecordBatch>]`, one entry per `CoveragePlan` segment in the same
    order — mirroring `CoveragePlan::new`'s own `windows` parameter
    contract, including its length-mismatch rejection. `cubism-iceberg`
    stays a `cubism-datafusion` dev-dependency.
  - **No per-point state/error metadata.** A decode/merge failure for any
    one segment fails the whole `SeriesResponse::new` call via `?`, not a
    partial response with an error marker on just that point. Plan line
    716's "state/error metadata where appropriate" is not implemented at
    that per-point granularity this slice.
  - **`gap_policy` is consumed, not deferred.** `TemporalQuery.gap_policy`
    existed since Milestone 8 but nothing read it until now
    (`range_query.rs`'s `GapPolicy` doc comment named this as a later
    milestone's job). `merge_average_column` over a segment with zero
    published windows returns a zero-count `AverageState`, whose
    `AggregateState::present()` is already `None` (not `Some(0.0)`) — so
    "no data" and "a real zero" are distinguished before `gap_policy` is
    even consulted. `gap_policy` only substitutes `Some(0.0)` for that
    `None` case when it is `GapPolicy::Zero`; `GapPolicy::Missing` leaves it
    `None`. A segment with a genuine non-zero-count merge is never touched
    by this substitution.
  - **`is_exact` is copied from `CoveragePlan`, not re-verified against
    `batches`.** `SeriesResponse::new` has no way to check that `batches[i]`
    actually contains rows for the windows `coverage.segments[i]` reports as
    published — that correspondence is the caller's responsibility, not
    something this module derives (same reason `CoveragePlan` itself
    doesn't derive the segment→`WindowId` mapping). A caller that
    mis-supplies `batches` for a segment it reported as published still
    gets `is_exact: true` back, with `value: None`/`Some(0.0)` depending on
    `gap_policy` — see "Phase 5 done condition" below, criterion 772, for
    the full statement of this caveat.
  - **No `XUnit` selector filtering — a real correctness gap, caught by
    this milestone's own cross-model phase review, tracked as
    [#19](https://github.com/jeromebanks/cubism-rs/issues/19), not fixed
    this slice.** `SeriesResponse::new` merges every row in `batches`
    unconditionally; it has no awareness of `xunit_id` and does not filter
    to the query's `TemporalQuery.selectors`. A states table row's
    `xunit_id` is a content hash of one specific lattice cell — the global
    rollup and each per-dimension cell are separately aggregated rows, not
    derivable from each other by summing — so a window containing more
    than one distinct `xunit_id` (the common case for any real
    multi-dimensional cube) gets silently over-merged regardless of which
    cell the query asked for. This session's own integration test does not
    exercise this: each window it constructs has exactly one `xunit_id`
    row. See #19 for the finding in full and the suggested fix. **Fixed by
    Milestone 10b-3 below** — this bullet is left as-is (this series'
    additive-pointer convention) rather than rewritten in place.
- **What it does:** given a `CoveragePlan` and one already-read batch list
  per segment, decodes+merges each segment's `avg_v1`-style column via
  `merge_average_column` and produces one `SeriesPoint` per segment:
  `bucket_start`/`bucket_end` (from `segment.range`), `is_exact` (from
  `SegmentCoverage::is_exact`), `value: Option<f64>` (the merged state's
  `present()`, gap-policy-adjusted per above), and `published`/`missing`
  (copied straight from `SegmentCoverage`). `SeriesResponse::source_resolution`
  is `coverage.resolution`.
- **Test:** integration test (real `cubism-iceberg` in-memory catalog,
  `crates/cubism-datafusion/tests/iceberg_bridge.rs`
  `series_response_materializes_two_published_windows_through_a_real_coverage_plan`):
  the same two-window setup as Milestone 10's own coverage-plan test, except
  both windows are published with real `AverageState`-encoded `avg_v1` rows;
  a real `ResolutionPlan` -> `CoveragePlan` is built, both windows are read
  back via `AggregateReader::read_window`, and `SeriesResponse::new`
  produces one point whose `value` equals `a.merge(&b).unwrap().present()`,
  `is_exact: true`, both windows in `published` with their real
  `WindowRevision`s, and `missing` empty — the decode+merge+wiring
  round-trips through a real Parquet write/scan, reached via the full
  `ResolutionPlan` -> `CoveragePlan` -> `SeriesResponse` path rather than a
  direct `merge_average_column` call. Six pure unit tests in
  `series_response.rs`'s own test module (no async iceberg harness) cover:
  one exact fully-published segment; `gap_policy: Missing` producing `None`
  for a no-data segment; `gap_policy: Zero` producing `Some(0.0)` for the
  same; that `gap_policy: Zero` does *not* touch a genuine present value;
  the batches-length-mismatch rejection; and a partially-published segment
  correctly propagating `published`/`missing`/`is_exact: false`.
- **Depends on:** Milestone 10b-1 (`Done`).
- **Done when:** the test passes and the full step-4 battery is clean. Met
  (63 passed in `cubism-datafusion`, up from 56; 203 passed / 2 ignored in
  workspace, up from 196; full battery clean). See "Phase 5 'done'
  condition" below for what landing this milestone does and does not close.

### Milestone 10b-3 — `SeriesResponse` single-selector `XUnit` filtering

- **Status:** Done. Fixes
  [#19](https://github.com/jeromebanks/cubism-rs/issues/19) (found by
  Milestone 10b-2's own step 8a cross-model phase review): `SeriesResponse::new`
  (`crates/cubism-datafusion/src/series_response.rs:131-180`) now resolves
  its caller's single `XUnit` selector to an `XUnitContentId` and filters
  every segment's `batches` to matching rows before merging, instead of
  merging every row in a batch unconditionally regardless of lattice cell.
- **Target:** `crates/cubism-datafusion/src/series_response.rs` (new
  `selectors: &[XUnit]` parameter on `SeriesResponse::new`, plus a new
  private `filter_batches_by_xunit` helper); doc-comment-only correction to
  `crates/cubism-datafusion/src/series_merge.rs`'s `merge_average_column`
  (no code change there — see its own doc comment for why filtering
  belongs at the `SeriesResponse::new` call site, not inside the merge
  primitive).
- **The key finding that shrank this below #19's own estimate:** #19's
  "Suggested next steps" assumed resolving a selector to its
  `XUnitContentId` would need "the cube's dimension structure." Reading
  `crates/cubism-core/src/encoding.rs`'s `impl From<&XUnit> for CanonicalXUnit`
  (`:77-100`) showed this is wrong — it's a pure, dictionary-independent
  function of the selector's own `dim`/attribute strings. Reading the build
  side's `XUnitContentIdUdf::invoke_with_args`
  (`crates/cubism-datafusion/src/temporal_build.rs:301-319`) confirmed the
  exact equality this fix depends on: a states row's `xunit_id` is computed
  as `canonical_xunit_content_id(&CanonicalXUnit::from(&decode_xunit(key,
  &dict)))` — the *same* two calls (`CanonicalXUnit::from` +
  `canonical_xunit_content_id`) a query-side selector resolves through, just
  fed a dictionary-decoded `XUnit` instead of the selector's own already-typed
  one. Both sides normalize identically (`CanonicalXUnit::normalize` sorts
  by dimension regardless of input order), so a query-side selector and a
  build-side row resolve to byte-identical ids for the same logical cell.
  This meant the fix was "resolve one selector via functions that already
  exist, then filter" — not a new selector-resolution subsystem.
- **Deviations from #19's own suggested fix** (advisor-confirmed scope
  fence):
  - **Exactly one selector.** `TemporalQuery.selectors` is a `Vec<XUnit>`;
    filtering to *N* selectors and merging across them would be the same
    class of over-merge bug with the caller's blessing, and "multi-XUnit
    response shape" is already a separate, unresolved roadmap decision (see
    "Phase 5 done condition" below). `SeriesResponse::new` rejects
    `selectors.len() != 1` with `CubismError::Temporal` — including the
    empty case, since empty must not be read as "no filter" (that reading
    is exactly the bug being fixed).
  - **Filtering lives in `SeriesResponse::new`, not in
    `merge_average_column`.** `merge_average_column` stays a generic
    decode+merge primitive with no `XUnit`/`xunit_id` awareness, per its own
    original design (Milestone 10b-1); `SeriesResponse::new` filters each
    segment's `batches` via `arrow::compute::filter_record_batch` against a
    boolean mask over the `xunit_id` column (null `xunit_id` matches
    nothing) before calling it unchanged.
  - **One new documented consequence:** a segment whose batches contain
    rows for other cells but none for the resolved selector now merges to
    "no data" (`value: None`/`Some(0.0)` under `GapPolicy::Zero`) with
    `is_exact` still `true` if `CoveragePlan` reports the segment fully
    published — the same already-documented "`is_exact` is not re-verified
    against `batches`" caveat from Milestone 10b-2, now also reachable via a
    selector that matches zero rows in an otherwise-published segment.
- **Test:** two new unit tests in `series_response.rs`'s own test module —
  `series_response_filters_batches_to_the_resolved_selector_before_merging`
  (a segment's batch carries two rows for two distinct real
  `XUnitContentId`s, a global cell and a `device=mobile` cell, both computed
  via the same `CanonicalXUnit::from`/`canonical_xunit_content_id` pair
  production code uses — not sentinel bytes; asserts the merged value
  reflects only the selector's cell) and
  `series_response_rejects_multi_selector_query` (both a two-selector and a
  zero-selector call are rejected). The pre-existing
  `series_response_materializes_two_published_windows_through_a_real_coverage_plan`
  integration test (`crates/cubism-datafusion/tests/iceberg_bridge.rs`) was
  updated to tag its two windows' rows with a real global-cell content id
  (via a new `content_id` helper calling the same production functions,
  replacing the arbitrary `[1u8; 32]`/`[2u8; 32]` sentinel bytes Milestone
  10b-2 used) — proving the fix through the full `ResolutionPlan` ->
  `CoveragePlan` -> `SeriesResponse` path against a real Parquet
  write/scan, not just the pure unit tests. All six of Milestone 10b-2's
  pre-existing unit tests were updated to pass the new `selectors`
  parameter; five of the six (every one that constructs a batch) also now
  tag their `avg_v1` rows with a real, not sentinel, `xunit_id` — the sixth
  (`series_response_rejects_batches_length_mismatch`) passes zero batches,
  so no `xunit_id` is involved.
- **Depends on:** Milestone 10b-2 (`Done`).
- **Done when:** the tests pass and the full step-4 battery is clean. Met
  (67 passed in `cubism-datafusion`, up from 65: +2 new unit tests; the
  pre-existing integration test count is unchanged at 4, one test modified
  in place per "Test" above; full battery clean). Fixes #19; #19 closed
  with a comment cross-linking this entry. See "Phase 5 done condition"
  below, criterion 772, for the updated verdict.

### Phase 5 "done" condition (for the milestones above)

Walking the plan's four completion criteria (lines 772-775), the same way
"Phase 4 done" above walks Phase 4's — narrow-closing with a real gap
tracked against an issue rather than treated as blocking, the same pattern
"Phase 4 done" used for its own criteria 2-4. Milestones 7-10b-3 are all
`Done` (10, 10b-1, and 10b-3 as narrowed, per their own entries'
"Deviations"):

- 772 ("answers exact aligned ranges from aggregate state") — **met as
  narrowed: single-`XUnit`-selector queries only, and `AverageState` only.**
  Milestone 10b-2 wired `CoveragePlan`'s `published` list to
  `merge_average_column` (Milestone 10b-1) and produced a real value, but
  originally merged every row in a window's batch unconditionally with no
  filtering by the query's `XUnit` selector — a real correctness gap
  Milestone 10b-2's own cross-model phase review caught, tracked as
  [#19](https://github.com/jeromebanks/cubism-rs/issues/19). **Milestone
  10b-3 fixed this**: `SeriesResponse::new` now resolves its caller's single
  selector to an `XUnitContentId` and filters every segment's `batches` to
  matching rows before merging, so a window containing more than one
  distinct lattice cell (the global rollup and each per-dimension cell are
  separately aggregated rows, not derivable from each other — the common
  case for any real multi-dimensional cube, not an edge case) no longer
  gets silently over-merged. The remaining scope limit is not a silent gap:
  a `selectors` slice whose length is not exactly 1 is rejected outright
  (`CubismError::Temporal`), so a multi-selector query fails clearly rather
  than guessing which cell(s) to answer — the "multi-XUnit/multi-measure
  response shape" stays a separate, explicitly unresolved decision (see
  below), not something this criterion silently underdelivers on. Also
  still scoped to `AverageState` only — widening to other measure kinds is
  a separate, not-yet-scoped follow-on (see Milestone 10b-2's "Deviations").
  **Second, narrower caveat, not itself a further gap in this criterion but
  worth stating plainly:** `SeriesResponse::new` also does not verify that
  `batches` actually corresponds to the windows `coverage` reports as
  published — `is_exact: true` is copied straight from
  `SegmentCoverage::is_exact()`, which reflects only the caller-supplied
  `published`/`missing` lists. A caller that mis-supplies `batches` (e.g.
  hands an empty list for a segment it itself reported as published) gets
  an `is_exact: true` point with no data behind it; the correspondence is
  entirely the caller's responsibility, same as the segment→`WindowId`
  mapping `CoveragePlan` already doesn't derive (`series_response.rs`'s own
  module doc comment records this).
- 773 ("partial-boundary behavior is truthful and tested") — met by
  Milestone 10's `exact=true` failure test (both the missing-window and
  the unaligned-but-published cases), unchanged by this milestone.
- 774 ("range plans prune storage and stay within latency/memory
  budgets") — **not met**, same gap Milestone 10's "Done when" already
  recorded: `read_window`'s predicate is a single `(window_id, revision)`
  equality per call, not a semijoin against every published window's
  manifest. Remains gated on
  [#8](https://github.com/jeromebanks/cubism-rs/issues/8) (or a
  not-yet-scoped multi-window `cubism-iceberg` API) — same "excluded, not
  silently dropped" treatment "Phase 4 done" gives its own unmet criteria
  against #10 (#17 was Phase 4's other exclusion; Milestone 5b fixed it,
  so it is no longer a live example of this pattern).
- 775 ("every result reports sufficient coverage and provenance") — **met
  as narrowed, minus per-point state/error metadata**. `SeriesResponse`'s
  `published`/`missing` fields (copied from `SegmentCoverage`) now sit
  alongside an actual materialized `value`, not just provenance on its own
  as Milestone 10 alone left it. Plan line 716's "state/error metadata
  where appropriate" is explicitly not implemented at per-point granularity
  this slice (see Milestone 10b-2's "Deviations") — a decode/merge failure
  fails the whole `SeriesResponse::new` call, not just the offending point.
- The plan's "Unresolved decisions" (lines 779-783) — "SQL table-function
  interface in addition to HTTP" is exactly the #8-gated half and remains
  unresolved (same gap as 774). The other four (max raw boundary scan,
  multi-XUnit/multi-measure response shape, server-side caching,
  authorization boundary) are not addressed by any milestone above and stay
  open for a successor roadmap slice.
- `/api/series` itself (`crates/cubism-serve`) and rolling
  comparisons/trend inputs (plan Phase 6) are **not** covered by
  Milestones 7-10b-3 — those are the next roadmap extension, not assumed
  done here.

**Net: Phase 5 as this roadmap defines it (excluding #8, the same way
"Phase 4 done" excludes #10) is done as of Milestone 10b-3.** 773 is
met without qualification; 775 is met as narrowed (missing per-point
state/error metadata); 772 is met as narrowed — single-`XUnit`-selector
queries only, and `AverageState` only — the real multi-cell-merge gap
Milestone 10b-2's own phase review found (#19) was fixed by Milestone
10b-3, and a multi-selector query is now rejected outright rather than
silently answered wrong; 774 and the SQL-table-function unresolved
decision remain #8's scope. #19 is closed, not merely narrowed around —
see Milestone 10b-3's own entry above for the fix and its test. 774 is not
a gap *in*
Milestones 7-10b-3's own scope as narrowed — it is a real, load-bearing gap
in what those milestones answer correctly, tracked rather than silently
dropped, the same treatment "Phase 4 done" gives #10 (a real, deliberately
out-of-scope maintenance gap) alongside its narrow-close. Per this
series' step 8a (`.claude/skills/timeseries-slice/SKILL.md`), landing
Milestone 10b-2 triggered the cross-model phase review that found #19 —
see
[`docs/phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`](phase-reviews/TIMESERIES_PHASE_5_REVIEW.md)
for that review's findings and disposition.

## POC Milestones — serving, demo, and documentation

Added 2026-08-18, once Phase 4 and Phase 5 above were both narrow-closed
(Milestone 5b and Milestone 10b-3 respectively). Same decomposition
convention as the milestones above: target file(s), the one test it adds
(where "test" applies — Milestones 12 and 13 are demo/documentation assets
and define "done" behaviorally instead), its dependencies, its
done-condition. **Doc-only when added** — none of the three milestones
below is implemented yet; every "Status" is `Not started`.

Not part of `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`'s own phase numbering:
the plan's Phase 6 is rolling comparisons/trend inputs, a later and
unrelated thing. This section exists because the engine built by Phases 1-5
(narrowed) has no path to being seen or understood by anyone outside this
session series — `crates/cubism-serve` (`api.rs`, `store.rs`) has zero
references to `cubism-iceberg`, `SeriesResponse`, or `CoveragePlan`
(confirmed via `rtk proxy grep -rl`), `examples/web_analytics_demo` still
builds through the pre-timeseries static `cubism-cli run` path (its own
`README.md`), and no single doc walks the pipeline `TIMESERIES_IMPLEMENTATION_PLAN.md`'s
"Outcome and delivery principles" describes end to end against what's
actually landed.

**Does not supersede [#20](https://github.com/jeromebanks/cubism-rs/issues/20).**
#20 (a delayed correction replay can silently undo an intentional rollback)
remains the highest-priority item on `docs/TIMESERIES_PHASE_24_HANDOFF.md`'s
deferred list — a real correctness gap needing a design decision, not a
mechanical fix, and therefore not sized to this series' slice shape. This
section is a parallel track (making the already-correct engine legible),
not a claim that correctness work is finished.

**Not 8a-gated.** `.claude/skills/timeseries-slice/SKILL.md` step 8a's
cross-model phase review fires when a slice closes a roadmap `## Phase N
"done" condition` section — this section has no such subsection and isn't
meant to grow one: it isn't one of `TIMESERIES_IMPLEMENTATION_PLAN.md`'s
phases (see above) and its milestones aren't completion criteria from that
plan. Ordinary per-slice advisor review (skill step 8) still applies to
each of the three milestones below, same as everywhere else in this doc.

### Milestone 11 — Minimal `/api/series` wiring in `crates/cubism-serve`

- **Status:** Done — `docs/TIMESERIES_PHASE_25_HANDOFF.md`.
- **Why this is a prerequisite, not scope creep:** Milestone 12's demo needs
  something to query against; without this, "demo" is a CLI build log (row
  counts printed by `iceberg-build`), not a dashboard or even a curl-able
  endpoint. This is the "next roadmap extension" `/api/series` gap already
  named in "Phase 5 done condition" above.
- **Target:** `crates/cubism-serve` — a new route handler opening a durable
  `PublicationStore` + Iceberg catalog from CLI-supplied paths (mirroring
  `iceberg_build`'s `CatalogConfig::Sqlite` wiring in
  `crates/cubism-cli/src/main.rs`) and calling
  `cubism_datafusion::SeriesResponse::new`, at the same narrowed scope
  Phase 5 already has: single `XUnit` selector, `AverageState` only. Not a
  general-purpose API — matches what Milestone 10b-3 actually answers
  correctly, no wider. `cubism-iceberg` moves from `crates/cubism-serve`'s
  dev-dependencies to a real dependency (it's only a dev-dep today, for
  `tests/api.rs`'s static-cube fixture via `cubism-datafusion`).
- **Request-supplied windows, not server-derived ones — a scope decision,
  not an oversight.** `crates/cubism-datafusion/src/range_query.rs:79-85`
  records that `CoveragePlan::new`'s segment→`WindowId` mapping is
  deliberately a caller-supplied input, because no canonical encoding from
  a `TimeRange`/`Resolution` pair to a `WindowId` exists anywhere in this
  codebase, and inventing one was already considered and rejected once as
  "a durable `cubism-core` API decision smuggled into a Phase-5 milestone."
  Milestone 11 does not relitigate that: the request body carries an
  explicit per-segment window list (`window_id` + optional revision),
  mirroring `CoveragePlan::new`'s own contract exactly, and the handler is
  a thin composition — `TemporalQuery::new` -> `ResolutionPlan::new` ->
  `CoveragePlan::new(windows from request)` -> `AggregateReader::read_window`
  per published window -> `SeriesResponse::new`. This pushes bucket->`WindowId`
  derivation to the caller (Milestone 12's demo build script for its own
  dataset, not a general rule this milestone establishes).
- **A request naming a window that was never appended must not read as
  `is_exact: true` with no data behind it.** `SegmentCoverage::is_exact()`
  trusts the caller's `published` list, and `SeriesResponse` never
  re-verifies `batches` against it — both already-documented caveats
  (Milestone 10's and Milestone 10b-2/10b-3's own entries above). Over HTTP
  the caller is an untrusted client, which is a new exposure for the same
  caveat, not a new bug class. In scope for this milestone: an unappended
  window resolves to `missing`/non-exact via a normal `AggregateReader::read_window`
  miss, not a 500 and not a falsely-exact point. Out of scope: any broader
  request validation against catalog state beyond what `read_window`
  already does.
- **Deviation from this entry's own pre-written scope, found while
  implementing:** the request does **not** carry a revision per window, only
  `(window_id, bucket_start)` pairs — `crates/cubism-serve/src/series.rs`'s
  handler resolves each window's current revision itself via
  `PublicationStore::current` before building `CoveragePlan`'s
  caller-supplied list, the same way `read_window` would anyway. Windows
  are also not pre-grouped per segment by the caller: the request carries
  one flat list, and the handler assigns each entry to whichever
  `ResolutionPlan` segment's `TimeRange::contains` its `bucket_start` — a
  pure range-containment check, not a new `WindowId` encoding, so this
  doesn't reopen the scope decision above. Net effect: a client asserts
  *less* than this entry originally proposed (no revision, no
  per-segment grouping), which narrows the untrusted-client exposure noted
  below rather than widening it — a request can misname a `window_id`, but
  it cannot assert a stale or fabricated revision for one that resolves.
- **Test:** `crates/cubism-serve/tests/series.rs`'s
  `series_endpoint_answers_an_aligned_two_window_range_and_reports_an_unknown_window_as_missing` —
  builds two real published windows through a durable Sqlite catalog +
  control store (`cubism-iceberg/tests/durability.rs`'s two-handle
  pattern: fixture data through one handle, `SeriesState::open` through a
  second, independent one), then posts three requests over a live HTTP
  listener via `series_router`: an aligned two-window range (asserts one
  exact point, the real merged `AverageState` value, both windows in
  `published`); the never-appended-window case above with `exact: false`
  (asserts 200, `is_exact: false`, `missing: ["2026-08-20"]`, not a 500);
  and the same case with `exact: true` (asserts a 400 naming the segment,
  via `CoveragePlan::new`'s own `exact=true` rejection). Does not prove
  `resolution: None` (auto-select) or `gap_policy: "zero"` — those are
  `range_query.rs`/`series_response.rs`'s own unit tests' job.
- **Depends on:** Milestone 10b-3 (`Done`).
- **Done when:** the tests pass, the full step-4 verification battery is
  clean, and a manual request against a locally served demo table returns a
  real value plus `is_exact`/coverage metadata — not just a 200 status. Met:
  all three battery legs clean (`cubism-iceberg` unchanged at 35 passed/1
  ignored/6 suites; `cubism-core` unchanged at 93 across its suites;
  workspace 211 passed/2 ignored/24 suites, up from Phase 24's 210/2/23 —
  exactly the one new test file). The "manual request" clause is satisfied
  by the integration test above rather than a separate ad hoc `curl`: it
  drives the same `series_router`/`SeriesState` code path a real `cubism
  serve --spec .. --warehouse ..` invocation would, over a real HTTP
  listener, against a real durable backend — not an in-process function
  call — so a second manual pass would exercise nothing the test doesn't
  already cover **at the `series_router`/`SeriesState` layer**. It does
  not cover `cubism-cli`'s own `serve` subcommand — argument parsing, the
  `--spec`/`--warehouse` durability wiring in `crates/cubism-cli/src/main.rs`,
  or the CLI's separate, always-required `cube_path` argument — since the
  test never runs that binary. Milestone 12b (below) does run it, as a
  real process, and is what actually found the `cube_path` requirement
  this entry's text doesn't mention.

### Milestone 12 — Time-series web analytics demo

- **Status:** Superseded before implementation by **Milestone 12a** and
  **Milestone 12b** below (added 2026-08-18, same session as this entry —
  found not to be one bounded slice before any code was written, same
  "10b turned out not to be one bounded slice" pattern as Milestone 10's
  own deviations section). 12a covers this entry's done-condition clauses
  (a) and (b) (late-arriving data ingested, two published window
  revisions); 12b covers clause (c) (a real `/api/series` query
  reflecting the correction). The text below is kept as the original,
  unsplit scope for the historical record — see 12a/12b for what actually
  landed/remains.
- **Target:** `examples/web_analytics_demo/` — regenerate `events.csv` (or
  add a variant alongside it) with event timestamps that arrive out of
  order / late relative to processing time, so the demo actually exercises
  `LatenessPolicy` and `CorrectionPlan` rather than only the current
  strictly-in-order dataset; add a build script driving `temporal-build`
  then `iceberg-build` with `--catalog-db`/`--control-db` (durable mode)
  instead of the static `cubism-cli run` path the current `README.md`
  documents; update `site/` or add a small query view that hits Milestone
  11's endpoint instead of (or alongside) the existing static-cube
  dashboard.
- **Owns the bucket->`WindowId` convention Milestone 11 deliberately did
  not invent.** Because Milestone 11's endpoint takes windows as a request
  parameter rather than deriving them, this milestone's build script must
  pick its own deterministic mapping (e.g. one window per day, `WindowId`
  = the bucket's start date) and use that same mapping on both sides: the
  `--window-id` values passed to `iceberg-build` per bucket, and the
  window list the demo's query view sends to `/api/series`. This is
  demo-local, not a codebase-wide convention — the open question Milestone
  11's entry cites stays open.
- **Test:** none in the workspace `cargo test` sense — this milestone
  produces a demo asset, not library code. "Done" is defined behaviorally
  below instead, the same way Milestone 11 is not exempt from a done
  condition just because it isn't a unit test.
- **Depends on:** Milestone 11.
- **Done when:** `examples/web_analytics_demo/README.md` documents a run,
  reproducible from a clean checkout, that (a) ingests events including at
  least one late-arriving correction, (b) publishes at least two window
  revisions for the same `window_id`, visibly showing the revision bump,
  and (c) serves a real query through Milestone 11's endpoint that returns
  a value plus `is_exact`/coverage metadata reflecting the correction.
- **Note:** the current dataset's `time` dimension (`web_analytics.yaml`'s
  `week`/`day` levels) is a static lattice dimension, not event-time
  bucketing — this milestone's regenerated data and build path are what
  make it a time-series demo rather than a relabeling of the existing one.

### Milestone 12a — Time-series demo dataset and durable build showing a revision bump

- **Status:** Done — `docs/TIMESERIES_PHASE_26_HANDOFF.md`.
- **Why split from Milestone 12:** the advisor step-1 review (this
  session) found Milestone 12 as originally written was not one bounded
  slice — three independent done-condition clauses spanning a Python
  generator, a build script, and `site/` wiring, cross-language and
  multi-deliverable, unlike every prior slice's "one integration test plus
  supporting code" sizing. Split into 12a (this entry: clauses (a) and
  (b), no code beyond a spec/generator/script, no Rust changes) and 12b
  below (clause (c): the `/api/series` query view).
- **Target:** `examples/web_analytics_demo/` — a new
  `web_analytics_temporal.yaml` (`apiVersion: cubism/v2alpha1`, one `geo`
  dimension, one `avg_revenue` `Avg` measure — matches Milestone 11's own
  narrowing: single `XUnit` selector, `AverageState` only); a new
  `generate_temporal_events.py` (separate from the static demo's
  `generate_events.py`, which is untouched) producing a small (~1,000-row,
  3-day) event stream plus a "held back" variant simulating a late
  arrival; `build_temporal_demo.sh` driving `temporal-build` +
  `iceberg-build` (durable mode) once per day, twice for the middle day.
- **Does not exercise `LatenessPolicy` or `CorrectionPlan` — a scope
  narrowing found while implementing, not in the original Milestone 12
  text above.** `iceberg-build` never consults `temporal.allowedLateness`,
  and `CorrectionCoordinator` is not in this script's call path
  (confirmed via `rtk proxy grep -rn "CorrectionPlan\|CorrectionCoordinator\|LatenessPolicy"
  crates/cubism-cli/src`, no matches in executable code). The revision
  bump is produced by re-running the build over a fuller event set with a
  manually-supplied `--revision 2`, not by any policy/coordinator
  deciding to admit late data. This demonstrates *what a landed
  correction looks like*, not the admission-policy machinery — see
  `examples/web_analytics_demo/README.md`'s own "What this does and does
  not prove" note.
- **Owns the bucket->`WindowId` convention Milestone 11 deliberately did
  not invent** (same note as original Milestone 12's text): one window
  per day, `WindowId` = the bucket's start date (`2026-04-06`,
  `2026-04-07`, `2026-04-08`).
- **Test:** none in the workspace `cargo test` sense — a demo asset, not
  library code, same as Milestone 12's own "Test" bullet already noted.
  "Done" is a captured, reproducible command transcript instead (see
  `examples/web_analytics_demo/README.md`'s "Time-series variant"
  section) — this entry's own verification is that transcript, not an
  unchanged `cargo test` count.
- **Depends on:** Milestone 11 (`Done`).
- **Done when:** `build_temporal_demo.sh`, run from a clean checkout,
  (a) ingests events including 6 late-arriving `signup_completed` rows for
  window `2026-04-07`, and (b) publishes that window at revision 1 then
  revision 2, with the control store's `control_runs`/`control_publications`
  tables showing both revisions `published` and the current pointer moved
  to revision 2. Met — verbatim captured output in
  `examples/web_analytics_demo/README.md`; full step-4 battery re-run and
  clean (unchanged counts throughout: no `.rs` file touched).

### Milestone 12b — Query view onto Milestone 12a's demo, hitting `/api/series`

- **Status:** Done — `docs/TIMESERIES_PHASE_27_HANDOFF.md`.
- **Target:** `examples/web_analytics_demo/query_temporal_demo.sh` (a
  standalone script, not `site/` wiring — cheaper, and does not require
  `build_temporal_demo.sh` to have run first): builds window `2026-04-07`
  at revision 1, starts a real `cubism serve --spec .. --warehouse ..`
  process (the CLI's required positional `cube_path` argument satisfied
  with a throwaway one-row placeholder parquet the script writes itself,
  since `/api/series` never reads it but `CubeStore::from_path` still
  requires a real `xunit`-column parquet), POSTs `/api/series`, builds
  revision 2 against the same running server, and POSTs again.
- **Confirmed empirically before writing the script, not assumed:** the
  next request against an already-running `cubism serve` process returns
  the newly published revision and the corrected value, with no process
  restart in between.
- **Owns the request's `windows` list** (Milestone 11's flat
  `(window_id, bucket_start)` list, per that entry's own deviation note):
  `{"window_id": "2026-04-07", "bucket_start": 1775520000000000}`,
  `resolution: "1d"`, `selector: "/G"`, `measure: "avg_revenue"` — the
  single-window form, not a wider range (a multi-day range at `1d`
  resolution collapses to one merged point, which would blur the before/
  after delta this milestone exists to show).
- **Test:** none — demo/documentation, same as 12a.
- **Depends on:** Milestone 12a (`Done`).
- **Done when:** a captured request/response pair (curl or equivalent)
  against a `cubism serve --spec .. --warehouse ..` process shows
  `avg_revenue`'s value differing between a query made against revision 1
  and one made after revision 2 was published, both with `is_exact: true`
  and the window's real revision in `published` — proving Milestone 12's
  original clause (c) (a real query reflecting the correction) rather
  than asserting it from the control-store rows alone. Met: `avg_revenue`
  for `/G` on window `2026-04-07` moves from `0.0` (revision 1 — every
  revenue-bearing event that day happened to be among the 6 held back by
  this seed) to `1.6638655462184875` (revision 2), both responses
  `is_exact: true` with the correct revision in `published`. Verbatim
  transcript in `examples/web_analytics_demo/README.md`'s "Query view"
  section.

### Milestone 13 — Time-series architecture and implementation documentation

- **Status:** Done — `docs/TIMESERIES_PHASE_28_HANDOFF.md`.
- **Target:** `docs/TIMESERIES_ARCHITECTURE.md` — the full `observed event
  -> allowed sparse XUnits -> event-time bucket -> versioned mergeable
  aggregate state -> immutable Iceberg window revision -> atomically
  published range-query visibility` pipeline from
  `TIMESERIES_IMPLEMENTATION_PLAN.md`'s "Outcome and delivery principles",
  plus a seventh hop (served HTTP answer, Milestones 11-12b) the plan's
  own contract doesn't include. Written as a synthesis of what is actually
  built, not the plan's forward-looking spec — every claim cites either a
  source file (grep'd fresh while writing the doc) or a handoff's "What
  was actually verified" section. One worked example (Milestone 12b's
  `evt_000261`, window `2026-04-07`, `avg_revenue` `0.0` -> `1.6638655462184875`)
  is traced through all seven stages at full depth; every measure kind,
  selector shape, and gap the worked example doesn't touch gets one line
  and a cross-link instead of its own section — advisor's explicit
  recommendation for keeping "trace one event end to end" bounded without
  leaving the done-condition half-satisfiable the way a stage-by-stage
  split would have.
- **Test:** none — documentation.
- **Depends on:** none functionally (the architecture it describes already
  exists), but written after Milestones 12a and 12b landed (split from the
  original single "Milestone 12" — see those entries) so it could use the
  demo as a worked example rather than a hypothetical one.
- **Done when:** a reader unfamiliar with this session series can trace a
  single event from ingestion through to a served query answer using this
  doc alone, without reading every phase handoff to do it. Met: the worked
  example above does exactly this, end to end, with every stage's claim
  independently citation-checked against the current source (not copied
  from an earlier session's citations — several had drifted, see the
  handoff's own "What was actually verified" section for the two caught
  and fixed while writing).

### Milestone 14 — Scalar-measure widening of `/api/series` (count/sum/min/max)

- **Status:** Done — `docs/TIMESERIES_PHASE_32_HANDOFF.md`.
- **Why this exists:** first slice of the finish-prompt demo item
  (`TIMESERIES_FINISH_PROMPT.md`, committed `957645a`) — its sub-part (a),
  "widening `/api/series` enough for real web metrics", chosen ahead of the
  dashboard-chart and docs-wiring slices because everything those render
  flows through this API's shape. The advisor round picked it over a
  window-discovery endpoint first: `PublicationStore` has no
  list-current-publications query at all (both backends key every read by
  `(cube_id, window_id)` or `run_id`), so discovery is its own cross-crate
  slice, while sub-part (b)'s "listing endpoint **or convention**" branch
  is already satisfied for the demo by Milestone 12a's day-string
  convention (`WindowId` = bucket start date).
- **Target:** `crates/cubism-datafusion/src/series_merge.rs`
  (`merge_scalar_column` beside `merge_average_column` — scalar kinds need
  no blob decoding, just their own fold); `crates/cubism-datafusion/src/
  series_response.rs` (`SeriesResponse::new` takes the measure's `AggKind`
  and dispatches blob-vs-scalar via a `merged_value` helper; no-data stays
  structural — avg's zero-count state, scalars' seen-flag — before gap
  policy applies); `crates/cubism-serve/src/series.rs` (the handler gate
  widened from avg-only to avg + count/sum/min/max; module doc's narrowing
  statement extended consciously rather than silently).
- **Scope narrowing kept:** multi-selector/multi-measure response shape
  stays rejected at `SeriesResponse::new`'s exactly-one-selector check —
  it is still the recorded unresolved roadmap decision ("Phase 5 done
  condition" section), not something this slice guesses an answer for.
  Blob-backed kinds beyond avg (variance/quantile/sketches) stay deferred,
  now with an explicit `CubismError::Temporal` rejection instead of falling
  into the avg path. The wire shape is unchanged (`value` stays
  `Option<f64>`; counts are exact to 2^53 in that presentation).
- **Test:** unit tests on both merge paths in `cubism-datafusion`
  (count sums across rows/batches/null-skipping; min-of-mins/max-of-maxes;
  seen-flag `None` vs genuine-zero `Some(0.0)`; wrong-storage-type and
  non-scalar-kind rejections; selector filtering on the scalar path);
  one integration test in `crates/cubism-serve/tests/series.rs`
  (`series_endpoint_answers_a_count_measure_by_summing_the_published_windows_rows`)
  proving a count-only cube end to end through real durable Iceberg tables:
  two published windows' rows (3 + 4) sum to the served point's value with
  both windows in `published`.
- **Depends on:** Milestones 10b-3, 11 (both `Done`).
- **Done when:** a POST `/api/series` naming a scalar-kind measure answers
  the exact folded value across multiple published windows over HTTP. Met:
  the integration test above returns `value: 7.0`, `is_exact: true`,
  two-window `published`; the avg-path behavior is regression-covered by
  the pre-existing tests, updated only for the added `AggKind` parameter.

### Milestone 15 — Time-series line chart in the serve dashboard

- **Status:** Done — `docs/TIMESERIES_PHASE_33_HANDOFF.md`.
- **Why this exists:** second slice of the finish prompt's demo item
  (sub-part c): "a time-series line chart in the serve dashboard fed by
  the API". Depends on Milestone 14's widened API; deliberately does NOT
  touch any `.rs` file — the chart is pure dashboard work.
- **Target:** `crates/cubism-serve/assets/index.html` only (plus one test
  bump and a capture script): a full-width "Time series" panel that stays
  `hidden` until its first load proves `/api/series` is actually mounted
  (a raw-fetch probe classifies axum's non-JSON 404 as "absent" and hides
  silently — static-cube-only serves, the common case, degrade exactly as
  before). Controls are free-text selector/measure inputs plus a date
  range; the measure default is hardcoded (`avg_revenue`), never derived
  from `/api/meta`, whose measures belong to the *static* placeholder cube
  in this demo and would mislead. Rendering is hand-rolled SVG string-
  building matching the page's existing `barChart()` style — no external
  assets (the page is `include_str!`-embedded).
- **One request per day, not one multi-day request:** `/api/series`
  merges each contiguous segment's windows into ONE point
  (`range_query.rs`'s "(at most one) interior segment"), so a single
  multi-day request renders one dot. The panel issues one single-day
  request per day in parallel — the exact shape `query_temporal_demo.sh`
  already uses — with windows per Milestone 12a's day-string convention
  (`window_id` = day, `bucket_start` = midnight µs; JS ms ×1000).
- **Honest encoding:** solid accent markers/polyline = exact points;
  dashed amber segments + hollow markers = partial coverage; an explicit
  ✕ on the baseline = missing/no-data (never interpolated across); each
  marker's native SVG `<title>` carries date · PARTIAL · revision ·
  value; a legend line reports `source_resolution` and which revisions
  are shown.
- **Test:** `crates/cubism-serve/tests/api.rs` asserts the served `/`
  page contains the panel hooks (`id="tsPanel"`, `/api/series`,
  `bucket_start`, `is_exact`) via a new raw-text helper (the JSON-parsing
  helper nulls out HTML). The behavioral capture is
  `examples/web_analytics_demo/serve_temporal_dashboard.sh`: builds all
  three demo days, serves them behind the placeholder-parquet trick,
  greps the page for the panel tokens, then issues the exact per-day
  JSON bodies the JS constructs — asserting three exact points whose
  middle day carries revision 2 (the correction) at bucket_start
  `1775520000000000`. What cargo/script verification can NOT prove:
  browser rendering of the SVG itself — human-verified against this
  script's captured transcript (Milestone 12a precedent).
- **Depends on:** Milestone 14 (`Done`).
- **Done when:** from a clean checkout, the capture script passes end to
  end and the served page carries the panel markup. Met — verbatim
  transcript in `docs/TIMESERIES_PHASE_33_HANDOFF.md`.

## Deferred (not in scope for this roadmap doc)

1. **~~Extend this roadmap to plan-Phase 5~~ Resolved** — see "Phase 5
   Milestones" above, added `docs/TIMESERIES_PHASE_16_HANDOFF.md`. What
   remains open from the original note: `/api/series`
   (`crates/cubism-serve`) itself, and everything gated on #8's
   SQL/pushdown half (completion-criterion 774, the plan's SQL
   table-function unresolved decision) — tracked in that section, not
   re-listed here.
2. **The Rollback-point milestone gap** noted above under "Phase 4 done" —
   (Corrected: resolved in `docs/TIMESERIES_PHASE_13_HANDOFF.md`. Rollback
   needed no new milestone; it falls out of the existing `publish` CAS
   mechanism, proven by a new `durability.rs` test. The one thing it left
   open — a rolled-back-past run's `RunState` still reporting `Published`
   — is [#18](https://github.com/jeromebanks/cubism-rs/issues/18), not a
   milestone-shaped gap.)
