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

use arrow_array::RecordBatch;
use cubism_core::AggKind;
use cubism_core::temporal::{WindowId, WindowRevision};
use iceberg::Catalog;

use crate::control::{Publication, PublicationStore, RunState};
use crate::correction::{CorrectionPlan, CorrectionStrategy};
use crate::error::{CubismIcebergError, Result};
use crate::table::TemporalTable;
use crate::writer::{AggregateWriter, AppendWindow};

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

        if matches!(claim.state(), RunState::Claimed { .. }) {
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
            publications.record_append(request.run_id, result.snapshot_id).await?;
        }

        publications.publish(request.run_id, Some(request.observed_current)).await
    }
}
