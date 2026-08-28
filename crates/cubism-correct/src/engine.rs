//! Run a correction end to end: corrected source events in, published
//! revision out — the second half of
//! [#16](https://github.com/jeromebanks/cubism-rs/issues/16).
//!
//! # What the caller no longer has to do
//!
//! [`cubism_iceberg::CorrectionCoordinator::execute`] takes a
//! caller-identified window plus **already-rebuilt** states and registry
//! batches, and runs the append-then-CAS-publish protocol. Producing those
//! batches meant running the aggregation engine yourself and knowing which
//! windows to run it for. [`CorrectionEngine::correct`] does both, so a
//! correction is "here is the corrected source and the time range that
//! changed" rather than a hand-assembled payload.
//!
//! # Ordering, and why each window is independent
//!
//! Windows are corrected one at a time, in ascending time order, each with
//! its own claim/append/publish cycle. A failure part-way through leaves
//! earlier windows corrected and later ones untouched — deliberately: the
//! alternative is an all-or-nothing protocol spanning several Iceberg
//! commits, which this stack has no transaction for. [`CorrectionOutcome`]
//! reports exactly which windows landed, so a retry can resume rather than
//! restart. Windows are independent by construction (each is a disjoint
//! bucket of event time), so a partial correction is a correct correction
//! of a prefix, never a torn one.
//!
//! # The CAS anchor
//!
//! `observed_current` is read immediately before the rebuild, and the
//! coordinator compare-and-swaps against it at publish time. A concurrent
//! correction landing in between makes the CAS fail with
//! [`cubism_iceberg::CubismIcebergError::StaleRevision`], which is returned
//! to the caller unretried — matching the coordinator's own documented
//! stance that retry policy belongs to the caller, not to the protocol.

use arrow_array::RecordBatch;
use cubism_core::temporal::{TimeRange, WindowId, WindowRevision};
use cubism_core::{AggKind, CubeSpec};
use cubism_datafusion::datafusion::prelude::SessionContext;
use cubism_datafusion::temporal_build::{build_temporal, NullEventTimePolicy};
use cubism_iceberg::{
    CorrectionCoordinator, CorrectionRequest, PublicationStore, TemporalTable,
};

use crate::windows::{affected_windows, AffectedWindow};
use crate::CorrectError;

/// One window's correction result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCorrection {
    pub window_id: WindowId,
    /// The revision that was current when the rebuild started — the CAS
    /// anchor the publish was checked against.
    pub previous_revision: WindowRevision,
    /// The revision this correction published.
    pub revision: WindowRevision,
    pub run_id: String,
}

/// What [`CorrectionEngine::correct`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CorrectionOutcome {
    /// Windows republished at a new revision, in the order they landed.
    pub corrected: Vec<WindowCorrection>,
    /// Windows the change's time range touches that have never been
    /// published. Reported rather than created: a correction revises an
    /// existing window, and silently publishing a brand-new one from a
    /// correction path would hide a mis-specified time range.
    pub skipped_unpublished: Vec<WindowId>,
}

pub struct CorrectionEngine;

impl CorrectionEngine {
    /// Correct every published window that `changed` touches, rebuilding
    /// each from `corrected_source`.
    ///
    /// `corrected_source` is a table name already registered on `ctx`
    /// holding the **full, corrected** event set for the affected range —
    /// not a delta. Aggregate states are not all subtractable (see
    /// `cubism_iceberg`'s `CorrectionPlan`: no current `AggKind`
    /// combination grants an additive shortcut), so a correction is always
    /// a full rebuild of the window from corrected source. Passing only the
    /// late rows would publish a window containing only those rows.
    ///
    /// Returns which windows were corrected and which were skipped as
    /// never-published.
    #[allow(clippy::too_many_arguments)]
    pub async fn correct(
        ctx: &SessionContext,
        spec: &CubeSpec,
        corrected_source: &str,
        changed: TimeRange,
        catalog: &dyn iceberg::Catalog,
        table: &TemporalTable,
        publications: &PublicationStore,
        run_id_prefix: &str,
    ) -> Result<CorrectionOutcome, CorrectError> {
        let temporal = spec
            .temporal
            .as_ref()
            .ok_or_else(|| CorrectError::NotTemporal(spec.name.clone()))?;
        let windows = affected_windows(temporal, changed)?;
        let kinds: Vec<AggKind> = spec.measures.iter().map(|m| m.agg).collect();

        let mut outcome = CorrectionOutcome::default();
        for AffectedWindow { window_id, range } in windows {
            // Read the CAS anchor BEFORE rebuilding: the coordinator
            // compares against what we observed at planning time, so
            // reading it after the (slow) rebuild would narrow the race
            // window in appearance while leaving it exactly as wide.
            let Some(observed_current) = publications.current(&table.cube_id, &window_id).await?
            else {
                outcome.skipped_unpublished.push(window_id);
                continue;
            };

            let rebuilt = build_temporal(
                ctx,
                spec,
                corrected_source,
                Some(range),
                Some(window_id.clone()),
                NullEventTimePolicy::Quarantine,
            )
            .await?;
            let states = collect(rebuilt.states).await?;
            let registry = collect(rebuilt.registry).await?;

            let revision = WindowRevision::new(observed_current.get() + 1)
                .map_err(|e| CorrectError::WindowNaming(e.to_string()))?;
            let run_id = format!("{run_id_prefix}-{}-r{}", window_id.as_str(), revision.get());

            CorrectionCoordinator::execute(
                catalog,
                table,
                publications,
                CorrectionRequest {
                    window_id: &window_id,
                    revision,
                    run_id: &run_id,
                    observed_current,
                    kinds: &kinds,
                    states: &states,
                    registry: &registry,
                },
            )
            .await?;

            outcome.corrected.push(WindowCorrection {
                window_id,
                previous_revision: observed_current,
                revision,
                run_id,
            });
        }
        Ok(outcome)
    }
}

async fn collect(
    frame: cubism_datafusion::datafusion::dataframe::DataFrame,
) -> Result<Vec<RecordBatch>, CorrectError> {
    Ok(frame.collect().await?)
}

/// Convenience: the windows a change would touch, without correcting
/// anything. Useful for a dry run, and the honest way to answer "what would
/// this correction republish" before committing to it.
pub fn plan(spec: &CubeSpec, changed: TimeRange) -> Result<Vec<AffectedWindow>, CorrectError> {
    let temporal = spec
        .temporal
        .as_ref()
        .ok_or_else(|| CorrectError::NotTemporal(spec.name.clone()))?;
    affected_windows(temporal, changed)
}
