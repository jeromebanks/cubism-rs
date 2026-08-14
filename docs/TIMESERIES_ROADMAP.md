# Time-Series Roadmap: Phase 4 Session-Slice Milestones

Date: 2026-08-12

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
non-goals. **Does not yet cover plan-Phase 5** (exactness-aware DataFusion
range queries) — extending this roadmap past Phase 4 is deferred; see the
Phase 7 handoff's deferred list.

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
4. **Once every milestone below is marked done**, Phase 4 (`#13`'s scope) is
   finished. At that point step 1 has no more milestones to consult here and
   should fall back to its original behavior: scan open issues and the
   latest handoff's deferred list directly (starting with #10 and this
   roadmap's own "Deferred" note about Phase 5).

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

- **Status:** Not started.
- **Target:** new coordinator module in `crates/cubism-iceberg/src`.
- **What it does:** identifies affected windows from event time, rebuilds
  them completely using the existing append/publish protocol, and publishes
  a new revision via the CAS mechanism formalized in Milestone 2.
- **Test:** plan line 634 — a late-event rebuild produces a state
  byte-identical (or answer-identical, per whatever equality the aggregate
  state supports) to a clean rebuild from the corrected source.
- **Depends on:** Milestone 2 (publishes via expected-revision CAS),
  Milestone 3 (needs a `CorrectionPlan` to execute).
- **Done when:** the coordinator can run a correction end-to-end for at
  least one aggregate kind and the rebuild-equality test passes.

### Milestone 5 — `ReconciliationRecord` + failure-injection recoverability

- **Status:** Not started.
- **Target:** new type in `crates/cubism-iceberg/src`; coordinator from
  Milestone 4 extended to record reconciliation state at each stage.
- **What it does:** records what a coordinator run did/attempted at each
  commit/publication stage, so an interrupted job can be classified and
  reconciled on retry (plan's completion criterion: "a deterministic
  recovery run classifies and reconciles every interrupted job").
- **Test:** plan line 638 — failure injection at every commit/publication
  stage the coordinator passes through, asserting recovery reaches a
  consistent published state (not a partial/corrupt one) in every case.
- **Depends on:** Milestone 4 (needs the coordinator's stages to inject
  failures into).
- **Done when:** `ReconciliationRecord` exists and the failure-injection
  test passes for every stage the coordinator has.

### Milestone 6 — Public correction API

- **Status:** Not started.
- **Target:** public API surface in `crates/cubism-iceberg/src` (library
  level — plan's "Public API changes"; a CLI surface for this is explicitly
  #11's scope, not this milestone's).
- **What it does:** submit/schedule a correction by source checkpoint or
  time range; inspect current/superseded revisions and reconciliation state.
- **Test:** an integration test exercising submit → coordinator runs →
  inspect reconciliation state, using Milestones 3-5's types together.
- **Depends on:** Milestones 3, 4, 5.
- **Done when:** the public API is callable end-to-end with a passing
  integration test. **This closes Phase 4's #13 scope** (modulo the
  "Unresolved decisions" and "Rollback point" items below, which are
  design/ops concerns rather than a coded milestone — see "Phase 4 done"
  below).

## Phase 4 "done" condition

Distinct from "every milestone above is done," per plan lines 650-669:

- All eight plan Phase 4 test-list items (lines 634-641) pass — six covered
  by Milestones 1-6 above (634, 635, 638, 639 plus 636/637 already closed);
  the remaining two (640 compaction, 641 retention) are #10's scope, not
  this roadmap's.
- Plan's four completion criteria (lines 651-655) hold, verified explicitly
  in the handoff that closes Milestone 6 — don't just assert "milestones
  done, therefore criteria met" without checking each one against what was
  actually built.
- Plan's "Unresolved decisions" (lines 658-663) are each either resolved (as
  Milestones 1 and 2 do for two of them) or explicitly deferred with a
  reason, not silently dropped.
- Plan's "Rollback point" (lines 666-669) — repointing a window to its prior
  published revision — is implemented and tested. Not currently assigned to
  a milestone above; whichever slice implements Milestone 6's public API
  should confirm whether rollback falls out of the existing CAS/publish
  mechanism already or needs its own bounded milestone (open question,
  flagged here rather than guessed at).

## Deferred (not in scope for this roadmap doc)

1. **Extend this roadmap to plan-Phase 5** (DataFusion range queries and
   serving, exactness-aware) once Milestones 1-6 above are underway — #15's
   own suggested steps ask for this, deliberately not done in the same slice
   that drafted Phase 4's milestones (see the Phase 7 handoff for why).
2. **The Rollback-point milestone gap** noted above under "Phase 4 done" —
   needs a decision on whether it's covered by Milestone 6 or needs its own
   slice.
