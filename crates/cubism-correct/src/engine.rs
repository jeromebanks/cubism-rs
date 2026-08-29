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
//! commits, which this stack has no transaction for. Windows are
//! independent by construction (each is a disjoint bucket of event time),
//! so a partial correction is a correct correction of a prefix, never a
//! torn one.
//!
//! That semantic is only defensible if the caller can *see* the prefix, so
//! a mid-run failure returns [`CorrectError::Partial`], which carries the
//! windows that already landed alongside the underlying error. A retry
//! resumes from the first window not in that list rather than restarting —
//! restarting would re-correct already-corrected windows and burn a
//! revision on each. Note the progress is reported on the **error** path:
//! [`CorrectionOutcome`] is returned only when every window succeeded, so
//! `?` still forces a caller to confront a partial run instead of reading
//! a success value that quietly under-reports.
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
            // Every failure below is caught rather than propagated with
            // `?`, so the windows already corrected in earlier iterations
            // travel out with the error instead of being dropped on the
            // floor. See this module's doc: the per-window-commit design
            // is only defensible because the prefix is reportable.
            match correct_one_window(
                ctx,
                spec,
                corrected_source,
                &window_id,
                range,
                &kinds,
                catalog,
                table,
                publications,
                run_id_prefix,
            )
            .await
            {
                Ok(Some(correction)) => outcome.corrected.push(correction),
                Ok(None) => outcome.skipped_unpublished.push(window_id),
                Err(source) => {
                    return Err(CorrectError::Partial {
                        corrected: outcome.corrected,
                        skipped_unpublished: outcome.skipped_unpublished,
                        source: Box::new(source),
                    });
                }
            }
        }
        Ok(outcome)
    }
}

/// Correct one window, or report it as never-published (`Ok(None)`).
///
/// Split out of the loop so a failure can be caught and paired with the
/// progress made so far; inlined with `?` it would discard that progress.
#[allow(clippy::too_many_arguments)]
async fn correct_one_window(
    ctx: &SessionContext,
    spec: &CubeSpec,
    corrected_source: &str,
    window_id: &WindowId,
    range: TimeRange,
    kinds: &[AggKind],
    catalog: &dyn iceberg::Catalog,
    table: &TemporalTable,
    publications: &PublicationStore,
    run_id_prefix: &str,
) -> Result<Option<WindowCorrection>, CorrectError> {
    // Read the CAS anchor BEFORE rebuilding: the coordinator compares
    // against what we observed at planning time, so reading it after the
    // (slow) rebuild would narrow the race window in appearance while
    // leaving it exactly as wide.
    let Some(observed_current) = publications.current(&table.cube_id, window_id).await? else {
        return Ok(None);
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

    // `WindowRevision::new` rejects only zero, so the sole way this fails
    // is a u64 wrap — hence `checked_add` and an overflow-named error
    // rather than the window-naming variant this once borrowed.
    let revision = observed_current
        .get()
        .checked_add(1)
        .and_then(|next| WindowRevision::new(next).ok())
        .ok_or_else(|| CorrectError::RevisionOverflow {
            window_id: window_id.as_str().to_string(),
            previous: observed_current.get(),
        })?;
    let run_id = format!("{run_id_prefix}-{}-r{}", window_id.as_str(), revision.get());

    CorrectionCoordinator::execute(
        catalog,
        table,
        publications,
        CorrectionRequest {
            window_id,
            revision,
            run_id: &run_id,
            observed_current,
            kinds,
            states: &states,
            registry: &registry,
        },
    )
    .await?;

    Ok(Some(WindowCorrection {
        window_id: window_id.clone(),
        previous_revision: observed_current,
        revision,
        run_id,
    }))
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
