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
evidence and the boundary between them.

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
   5's, see "Phase 5 Milestones"), this roadmap's milestone list is
   exhausted — not the same claim as Phase 4 (`#13`'s scope) or Phase 5
   being finished; see "Phase 4 done" and "Phase 5 Milestones" below for
   why (as of Milestone 6 closing, Phase 4 is not: criterion 2 is open via
   #17, criteria 3-4 are out of this roadmap's scope via #10; Phase 5's own
   completion criteria are walked in that section). At that point step 1
   has no more milestones to consult here and should fall back to its
   original behavior: scan open issues and the latest handoff's deferred
   list directly, starting with #10 (compaction/retention/object-store,
   still out of this roadmap's scope) — the Phase 5 fallback this line
   used to point to is resolved now that Phase 5 has its own milestones
   below.

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

- **Status:** Done — `docs/TIMESERIES_PHASE_8_HANDOFF.md`.
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
  published), not all four.
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
     interrupted job."* **Not** met as literally worded — Milestone 5
     (narrowed) recovers three of the four stages a correction can be
     interrupted at; the fourth (`AwaitingAppend`'s ambiguous case — append
     committed but not yet recorded) is not safely recoverable by a naive
     retry, tracked in
     [#17](https://github.com/jeromebanks/cubism-rs/issues/17), not fixed
     by any milestone above.
  3. *"Compaction and retention SLOs are documented and observable."*
     **Not** met — no compaction or retention exists in this roadmap's
     scope at all, by design (this roadmap's "Scope" section above excludes
     it, tracked entirely in
     [#10](https://github.com/jeromebanks/cubism-rs/issues/10)).
  4. *"No maintenance path changes aggregate answers."* Not applicable yet
     — there is no maintenance path (compaction/retention) to check against,
     for the same reason as (3).
  Net: only criterion 1 is fully met; 2 is partially met (#17); 3 and 4 are
  out of this roadmap's scope entirely (#10). Phase 4 as this roadmap
  defines it (excluding #10) is therefore **not** done purely because
  Milestone 6 closed — #17 remains a real gap in criterion 2.
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
  stage is recoverable" is **not** fully met by Milestone 5 as narrowed: a
  correction whose Iceberg append committed but crashed before
  `record_append` persisted that fact is not safely recoverable by
  `CorrectionCoordinator::execute` today (a naive retry can duplicate
  visible rows — see `coordinator.rs`'s `ReconciliationRecord` doc
  comment). Tracked in
  [#17](https://github.com/jeromebanks/cubism-rs/issues/17), not assigned to
  a milestone above.

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
  (`crates/cubism-datafusion/src/series_response.rs:62-114`), the "wiring"
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

### Phase 5 "done" condition (for the milestones above)

Walking the plan's four completion criteria (lines 772-775), the same way
"Phase 4 done" above walks Phase 4's — narrow-closing with a real gap
tracked against an issue rather than treated as blocking, the same pattern
"Phase 4 done" used for its own criteria 2-4. Milestones 7-10b-2 are all
`Done` (10 and 10b-1 as narrowed, per their own entries' "Deviations"):

- 772 ("answers exact aligned ranges from aggregate state") — **met**.
  `SeriesResponse::new` (Milestone 10b-2) wires `CoveragePlan`'s `published`
  list to `merge_average_column` (Milestone 10b-1) and produces a real
  value; the integration test above proves this end-to-end against a real
  published window, not just in-memory logic. Scoped to `AverageState`
  only — widening to other measure kinds is not this criterion's literal
  wording and is left open (see Milestone 10b-2's "Deviations").
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
  silently dropped" treatment "Phase 4 done" gave its own unmet criteria
  against #10/#17.
- 775 ("every result reports sufficient coverage and provenance") — **met**.
  `SeriesResponse`'s `published`/`missing` fields (copied from
  `SegmentCoverage`) now sit alongside an actual materialized `value`, not
  just provenance on its own as Milestone 10 alone left it.
- The plan's "Unresolved decisions" (lines 779-783) — "SQL table-function
  interface in addition to HTTP" is exactly the #8-gated half and remains
  unresolved (same gap as 774). The other four (max raw boundary scan,
  multi-XUnit/multi-measure response shape, server-side caching,
  authorization boundary) are not addressed by any milestone above and stay
  open for a successor roadmap slice.
- `/api/series` itself (`crates/cubism-serve`) and rolling
  comparisons/trend inputs (plan Phase 6) are **not** covered by
  Milestones 7-10b-2 — those are the next roadmap extension, not assumed
  done here.

**Net: Phase 5 as this roadmap defines it (excluding #8, the same way
"Phase 4 done" excludes #10) is done as of Milestone 10b-2.** Criteria 772,
773, and 775 are met; 774 and the SQL-table-function unresolved decision
remain #8's scope, not a gap in Milestones 7-10b-2 themselves. Per this
series' step 8a (`.claude/skills/timeseries-slice/SKILL.md`), landing
Milestone 10b-2 triggers the cross-model phase review — see
[`docs/phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`](phase-reviews/TIMESERIES_PHASE_5_REVIEW.md)
for that review's findings and disposition.

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
