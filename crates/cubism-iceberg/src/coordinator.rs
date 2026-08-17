//! Correction execution (`docs/TIMESERIES_ROADMAP.md` Milestone 4, narrowed
//! from its original wording — see the roadmap entry for the correction).
//!
//! The plan (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:599-600`) describes a
//! coordinator that both *identifies affected windows from event time* and
//! *rebuilds them completely*. This module implements only the second half.
//! Identifying affected windows requires reading and re-aggregating source
//! events, which needs an aggregation engine — this crate deliberately links
//! none (see `src/lib.rs`'s top-level doc comment: no `cubism-datafusion`
//! dependency, ever). [`CorrectionCoordinator`] instead takes a
//! caller-identified window, the already-rebuilt corrected states/registry
//! batches, and the revision the caller observed as current when it planned
//! the correction, and executes the append-then-CAS-publish protocol for
//! that one window. "Identify affected windows from event time" is left for
//! a future milestone in a crate that can aggregate (tracked in the roadmap).
//!
//! [`CorrectionCoordinator::execute`] does not retry a rejected CAS. Plan
//! line 614 ("losing writers do not republish automatically without
//! rereading source and current state") makes that the caller's protocol,
//! not the coordinator's — matching Milestone 2's
//! `sqlite_correction_against_a_superseded_revision_is_rejected_then_succeeds_on_retry`,
//! which established refresh-then-retry as something the caller does after
//! observing [`CubismIcebergError::StaleRevision`], not something hidden
//! inside the publish call.
//!
//! [`CorrectionRequest::observed_current`] is a required [`WindowRevision`],
//! not `Option<WindowRevision>`: a correction only makes sense against a
//! window that has already been published once. A first-time build for a
//! window with no prior publication should go through
//! [`crate::writer::AggregateWriter::append_window`] and
//! [`crate::control::PublicationStore::publish`] directly (as every
//! existing fixture's initial build already does), not through this
//! coordinator. This is a type-level policy, not a runtime check.
//!
//! [`ReconciliationRecord`] (Milestone 5) classifies a run's recovery status
//! from the durable [`RunState`] the control store already tracks — see its
//! own doc comment for what each stage does and does not prove is safe to
//! recover from a restart.
//!
//! [`RunInspection`] (Milestone 6, narrowed) is this crate's inspection
//! half of the plan's "submit or schedule a correction by source
//! checkpoint/time range; inspect current/superseded revisions and
//! reconciliation state" (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:604-605`).
//! Only the inspection half is built: a submit/schedule-by-checkpoint-or-
//! range facade needs the same event-time-to-window mapping Milestone 4
//! already found this crate cannot do without an aggregation engine
//! (tracked as #16, not this milestone) — narrowed to "caller-identified
//! window" the way `CorrectionCoordinator::execute` already is, such a
//! facade would be a zero-behavior wrapper over `execute`, the same
//! untested-scaffolding refusal Milestones 3 and 5 already made for unused
//! fields and a redundant persisted record. `RunInspection::inspect` pairs
//! [`ReconciliationRecord::classify`] with a live
//! [`crate::control::PublicationStore::current`] read, which is what makes
//! "superseded" answerable at all post-rollback — see
//! [`RevisionStatus`]'s doc comment for why that word itself is avoided.

use arrow_array::RecordBatch;
use cubism_core::AggKind;
use cubism_core::temporal::{WindowId, WindowRevision};
use iceberg::Catalog;

use crate::control::{Publication, PublicationStore, RunState};
use crate::correction::{CorrectionPlan, CorrectionStrategy};
use crate::error::{CubismIcebergError, Result};
use crate::reader::AggregateReader;
use crate::table::TemporalTable;
use crate::writer::{AggregateWriter, AppendWindow};

/// Classifies a run's recovery status from its durable [`RunState`] —
/// Milestone 5 (`docs/TIMESERIES_ROADMAP.md`). This is **not** a new
/// persisted record: `control.rs`'s `Claimed`/`Appended`/`Published`
/// already durably encode every stage a run passes through (both the
/// in-memory and SQLite backends), and [`PublicationStore::run_state`]
/// already reads it back. `ReconciliationRecord::classify` is a pure
/// projection of that existing state into "what should happen next,"
/// exactly the same relationship Milestone 2 found between
/// `ExpectedRevision` and the CAS parameter that already existed — adding a
/// second table to track the same three stages would be untested
/// scaffolding, the thing Milestone 3 refused to do for
/// checkpoint/range fields.
///
/// **What each variant does and does not prove is recoverable:**
///
/// - [`Self::NotStarted`]: no run with this ID has ever been claimed (or
///   the run ID is unknown to this control store). Recovery: run `execute`
///   from scratch. No production code path in this crate reaches this
///   variant today — `execute` only ever calls `classify` with
///   `Some(claim.state())`, since `claim_run` always returns a `RunState`.
///   It exists because `classify` takes `Option<&RunState>` to match
///   `PublicationStore::run_state`'s own return type, for a caller
///   inspecting a run's status before deciding whether to call `execute` at
///   all. This module's own `classify_maps_every_run_state_stage_to_its_reconciliation_record`
///   unit test exercises it directly (`classify(None)`); no integration
///   test reaches it through `execute`, matching Milestone 3's own
///   admission that its `AdditiveShortcut` arm is reachable only in
///   principle, not by any end-to-end test today.
/// - [`Self::AwaitingAppend`]: the run was claimed but the control store
///   has no append recorded. Recovery: run `execute`, which appends and
///   publishes. This state is ambiguous between "the append was never
///   attempted" and "the Iceberg append committed, but the process crashed
///   before `record_append` persisted that fact" — `AggregateWriter::append_window`
///   commits via `fast_append`, which is purely additive and has no
///   idempotency check against a prior commit for the same
///   window/revision/run on its own. **Resolved by
///   [issue #17](https://github.com/jeromebanks/cubism-rs/issues/17)'s fix:**
///   `execute` no longer assumes "never attempted" — it asks
///   [`crate::reader::AggregateReader::run_append_snapshot`] whether a
///   states row for this exact `(window_id, revision, run_id)` already
///   exists in the table's current snapshot before deciding whether to
///   append again. See that function's own doc comment for the one thing
///   it deliberately does not defend against (concurrent recovery of the
///   identical run, as opposed to sequential crash-then-retry).
/// - [`Self::AwaitingPublish`]: the append committed and was recorded, but
///   no publication exists yet. Recovery: run `execute`, which skips the
///   append (see the append-skip branch below) and publishes. Already
///   proven safe by
///   `tests/coordinator.rs`'s
///   `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
///   — cited here, not re-proven.
/// - [`Self::Published`]: the run already published. Recovery: replaying
///   `execute` with the identical request must return the same
///   [`Publication`], not a new one and not [`CubismIcebergError::StaleRevision`]
///   — see `PublicationStore::publish`'s early return when the requested
///   revision is already current, which both the in-memory and SQLite
///   backends implement identically ahead of the CAS comparison.
///   **Caveat added by Milestone 6:** this variant alone does not mean the
///   run's revision is still the window's *current* one. After a rollback
///   (`docs/TIMESERIES_PHASE_13_HANDOFF.md` — repointing `current` back to
///   an earlier run via the same CAS `publish` call) a differently-run
///   revision can retake `current` while this run's own `RunState` still
///   reads `Published`, unchanged, because rollback never touches the
///   run it rolls back past
///   ([issue #18](https://github.com/jeromebanks/cubism-rs/issues/18)). Use
///   [`RunInspection::inspect`] for a live cross-check against `current`
///   rather than trusting this variant standalone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationRecord {
    NotStarted,
    AwaitingAppend { revision: WindowRevision },
    AwaitingPublish { revision: WindowRevision, aggregate_snapshot_id: i64 },
    Published { revision: WindowRevision, aggregate_snapshot_id: i64 },
}

impl ReconciliationRecord {
    /// Classify a run's recovery status from its durable [`RunState`], or
    /// [`Self::NotStarted`] if the run has no recorded state at all (e.g.
    /// [`PublicationStore::run_state`] returned `None`).
    pub fn classify(run_state: Option<&RunState>) -> Self {
        match run_state {
            None => Self::NotStarted,
            Some(RunState::Claimed { revision, .. }) => Self::AwaitingAppend { revision: *revision },
            Some(RunState::Appended { revision, aggregate_snapshot_id, .. }) => {
                Self::AwaitingPublish { revision: *revision, aggregate_snapshot_id: *aggregate_snapshot_id }
            }
            Some(RunState::Published { revision, aggregate_snapshot_id, .. }) => {
                Self::Published { revision: *revision, aggregate_snapshot_id: *aggregate_snapshot_id }
            }
        }
    }
}

/// Whether a run's own revision is the window's *current* revision, per a
/// live read of [`PublicationStore::current`] — Milestone 6's inspection
/// API (`docs/TIMESERIES_ROADMAP.md`), resolving
/// [issue #18](https://github.com/jeromebanks/cubism-rs/issues/18)'s option
/// 1: a live cross-check, not a trusted [`ReconciliationRecord::Published`]
/// read in isolation (the roadmap's precedent, from Milestones 3 and 5, is
/// to prefer this over #18's option 2 — a new durable rollback record —
/// absent a concrete need for queryable rollback history).
///
/// Deliberately not named `Superseded`/`NotSuperseded`: after a rollback
/// (`docs/TIMESERIES_PHASE_13_HANDOFF.md`), a not-current revision can be
/// numerically *higher* than the window's current one, so "superseded"
/// (which implies "replaced by something newer") would be misleading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionStatus {
    /// This run's revision equals `PublicationStore::current` for its
    /// window.
    Current,
    /// This run's revision is not `PublicationStore::current` for its
    /// window. This alone does not say *why* — a later correction, a
    /// rollback past this run, or (if the run's own revision was never
    /// published) nothing having happened yet. `control_runs` has no
    /// record ordering revisions or distinguishing those cases; see #18.
    NotCurrent,
}

/// A run's reconciliation stage plus, when that stage is
/// [`ReconciliationRecord::Published`], whether its revision is still the
/// window's current one. Pairing these is Milestone 6's inspection API:
/// [`ReconciliationRecord::classify`] alone is a pure projection of
/// [`RunState`] with no I/O; knowing whether a `Published` run is still
/// current requires reading [`PublicationStore::current`] too, which
/// [`Self::inspect`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunInspection {
    pub record: ReconciliationRecord,
    /// `Some` only when `record` is [`ReconciliationRecord::Published`] —
    /// every other stage has no window revision yet to compare against
    /// `current`.
    pub revision_status: Option<RevisionStatus>,
}

impl RunInspection {
    /// Inspect one run: classify its [`RunState`] and, if
    /// [`ReconciliationRecord::Published`], cross-check its revision
    /// against the window's live [`PublicationStore::current`] value.
    ///
    /// `cube_id`/`window_id` must be the run's *own* window — a type-level
    /// caller contract, not a runtime check, matching
    /// [`CorrectionRequest::observed_current`]'s. `RunState`'s variants do
    /// carry their own `window_key`, but this function does not read it
    /// back to verify agreement: passing a window other than the one
    /// `run_id` actually published into silently compares against an
    /// unrelated window's `current` and can report a confidently wrong
    /// [`RevisionStatus`].
    pub async fn inspect(
        publications: &PublicationStore,
        cube_id: &str,
        window_id: &WindowId,
        run_id: &str,
    ) -> Result<Self> {
        let run_state = publications.run_state(run_id).await?;
        let record = ReconciliationRecord::classify(run_state.as_ref());
        let revision_status = match &record {
            ReconciliationRecord::Published { revision, .. } => {
                let current = publications.current(cube_id, window_id).await?;
                Some(if current == Some(*revision) { RevisionStatus::Current } else { RevisionStatus::NotCurrent })
            }
            _ => None,
        };
        Ok(Self { record, revision_status })
    }
}

/// One correction to execute: a window, the revision it produces, the
/// already-rebuilt corrected payload, and the revision the caller observed
/// as current at planning time (the CAS anchor — see the module doc
/// comment on why this is not re-read at publish time).
pub struct CorrectionRequest<'a> {
    pub window_id: &'a WindowId,
    pub revision: WindowRevision,
    pub run_id: &'a str,
    pub observed_current: WindowRevision,
    pub kinds: &'a [AggKind],
    pub states: &'a [RecordBatch],
    pub registry: &'a [RecordBatch],
}

pub struct CorrectionCoordinator;

impl CorrectionCoordinator {
    /// Execute one correction: consult [`CorrectionPlan`] for the involved
    /// [`AggKind`]s, append the corrected states/registry under
    /// `request.revision`, and publish via compare-and-swap against
    /// `request.observed_current`. Returns
    /// [`CubismIcebergError::UnsupportedCorrectionStrategy`] if strategy
    /// selection ever grants [`CorrectionStrategy::AdditiveShortcut`] — no
    /// current `AggKind` combination does (Milestone 3's finding), so this
    /// arm only guards against a future kind gaining that capability before
    /// this coordinator learns how to apply a shortcut in place; it is not
    /// exercised by any test today, matching Milestone 3's own tests'
    /// admission that the branch is reachable only in principle.
    ///
    /// A rejected CAS (`Err(CubismIcebergError::StaleRevision)`) is returned
    /// to the caller as-is, with no retry — see the module doc comment.
    pub async fn execute(
        catalog: &dyn Catalog,
        temporal_table: &TemporalTable,
        publications: &PublicationStore,
        request: CorrectionRequest<'_>,
    ) -> Result<Publication> {
        let plan = CorrectionPlan::select(request.kinds);
        if plan.strategy() == CorrectionStrategy::AdditiveShortcut {
            return Err(CubismIcebergError::UnsupportedCorrectionStrategy(plan.strategy()));
        }

        let expected_rows: u64 = request.states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
        let claim = publications
            .claim_run(
                &temporal_table.cube_id,
                request.window_id,
                request.run_id,
                request.revision,
                expected_rows,
            )
            .await?;

        if let ReconciliationRecord::AwaitingAppend { .. } = ReconciliationRecord::classify(Some(claim.state())) {
            // #17: `Claimed` is ambiguous between "never appended" and
            // "appended, but crashed before `record_append`." Ask Iceberg's
            // own committed state directly, rather than assuming the
            // append never happened — see `AggregateReader::run_append_snapshot`'s
            // doc comment for exactly what this does and does not prove.
            let already_committed = AggregateReader::run_append_snapshot(
                catalog,
                temporal_table,
                request.window_id,
                request.revision,
                request.run_id,
            )
            .await?;
            let snapshot_id = match already_committed {
                Some(snapshot_id) => snapshot_id,
                None => {
                    // Registry writes are idempotent-to-repeat by design
                    // (`AggregateWriter::append_window`'s own doc comment:
                    // content-addressed, safe to re-commit) and this branch
                    // only runs at all when the *states* half was not
                    // found — so re-running the whole append here, registry
                    // included, cannot reintroduce #17's states-side bug.
                    let result = AggregateWriter::append_window(
                        catalog,
                        temporal_table,
                        AppendWindow {
                            window_id: request.window_id,
                            revision: request.revision,
                            run_id: request.run_id,
                            states: request.states,
                            registry: request.registry,
                        },
                    )
                    .await?;
                    result.snapshot_id
                }
            };
            publications.record_append(request.run_id, snapshot_id).await?;
        }

        publications.publish(request.run_id, Some(request.observed_current)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revision(n: u64) -> WindowRevision {
        WindowRevision::new(n).unwrap()
    }

    fn window_key() -> (String, String) {
        ("cube".to_string(), "2026-08-12".to_string())
    }

    #[test]
    fn classify_maps_every_run_state_stage_to_its_reconciliation_record() {
        assert_eq!(ReconciliationRecord::classify(None), ReconciliationRecord::NotStarted);

        let claimed = RunState::Claimed { window_key: window_key(), revision: revision(2), expected_rows: 3 };
        assert_eq!(
            ReconciliationRecord::classify(Some(&claimed)),
            ReconciliationRecord::AwaitingAppend { revision: revision(2) }
        );

        let appended = RunState::Appended {
            window_key: window_key(),
            revision: revision(2),
            expected_rows: 3,
            aggregate_snapshot_id: 101,
        };
        assert_eq!(
            ReconciliationRecord::classify(Some(&appended)),
            ReconciliationRecord::AwaitingPublish { revision: revision(2), aggregate_snapshot_id: 101 }
        );

        let published = RunState::Published {
            window_key: window_key(),
            revision: revision(2),
            expected_rows: 3,
            aggregate_snapshot_id: 101,
        };
        assert_eq!(
            ReconciliationRecord::classify(Some(&published)),
            ReconciliationRecord::Published { revision: revision(2), aggregate_snapshot_id: 101 }
        );
    }

    /// Proves `RunInspection::inspect` (Milestone 6): no revision status
    /// before a run is published, `Current` for the run holding a window's
    /// live `current` revision, and — replaying the exact rollback shape
    /// `docs/TIMESERIES_PHASE_13_HANDOFF.md` proved — `NotCurrent` for a
    /// run whose own `RunState` still reads `Published` after a later
    /// rollback repointed `current` away from it. Uses
    /// `PublicationStore::in_memory()` only (no Iceberg catalog): `publish`
    /// requires a prior `record_append` but not a real Iceberg commit, so
    /// this test proves the inspection pairing itself, not the durable
    /// SQLite backend or reader-visibility — those are `tests/durability.rs`'s
    /// job.
    #[tokio::test]
    async fn run_inspection_pairs_reconciliation_stage_with_a_live_current_check() {
        let store = PublicationStore::in_memory();
        let w = WindowId::new("2026-08-12").unwrap();

        store.claim_run("cube", &w, "run-1", revision(1), 1).await.unwrap();
        let claimed = RunInspection::inspect(&store, "cube", &w, "run-1").await.unwrap();
        assert_eq!(claimed.record, ReconciliationRecord::AwaitingAppend { revision: revision(1) });
        assert_eq!(claimed.revision_status, None, "no revision to compare against `current` before publish");

        store.record_append("run-1", 101).await.unwrap();
        store.publish("run-1", None).await.unwrap();

        store.claim_run("cube", &w, "run-2", revision(2), 2).await.unwrap();
        store.record_append("run-2", 102).await.unwrap();
        store.publish("run-2", Some(revision(1))).await.unwrap();

        // Rollback: republish run-1's own fixed revision against the
        // window's actual current (2) — the mechanism proven in
        // `docs/TIMESERIES_PHASE_13_HANDOFF.md`.
        store.publish("run-1", Some(revision(2))).await.unwrap();

        let run1 = RunInspection::inspect(&store, "cube", &w, "run-1").await.unwrap();
        assert_eq!(
            run1.record,
            ReconciliationRecord::Published { revision: revision(1), aggregate_snapshot_id: 101 }
        );
        assert_eq!(run1.revision_status, Some(RevisionStatus::Current));

        let run2 = RunInspection::inspect(&store, "cube", &w, "run-2").await.unwrap();
        assert_eq!(
            run2.record,
            ReconciliationRecord::Published { revision: revision(2), aggregate_snapshot_id: 102 }
        );
        assert_eq!(
            run2.revision_status,
            Some(RevisionStatus::NotCurrent),
            "run-2's RunState still says Published but revision 2 is no longer current — see #18"
        );
    }
}
