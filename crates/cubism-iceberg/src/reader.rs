use arrow_array::RecordBatch;
use cubism_core::temporal::{WindowId, WindowRevision};
use futures::TryStreamExt;
use iceberg::Catalog;
use iceberg::expr::Reference;
use iceberg::spec::Datum;

use crate::control::PublicationStore;
use crate::error::{CubismIcebergError, Result};
use crate::schema::{REVISION_COLUMN, RUN_ID_COLUMN, WINDOW_ID_COLUMN};
use crate::table::{TemporalTable, current_snapshot_id};

pub struct AggregateReader;

impl AggregateReader {
    /// Read the states rows for one window's currently published revision.
    ///
    /// Resolves the current revision from `publications`, then scans the
    /// states table with an Iceberg predicate on `(window_id, revision)` —
    /// native `Table::scan().with_filter(...)`, not a DataFusion join. Each
    /// row already carries its own `window_id`/`revision` (see
    /// [`crate::writer::AggregateWriter::append_window`]), so this is an
    /// equality filter over one window, not the plan's general "semijoin
    /// against every published window" — sufficient for a single-window
    /// verify/read, not a range query across many windows (that's Phase 5,
    /// gated on DataFusion 53/54 convergence per the crate's top-level doc
    /// comment).
    ///
    /// Correction (`docs/TIMESERIES_ROADMAP.md`'s Phase 5 milestones,
    /// filed against [#8](https://github.com/jeromebanks/cubism-rs/issues/8)):
    /// "gated on DataFusion 53/54 convergence" overstates it. Only *generic
    /// SQL-level* access — registering this crate's tables in a DataFusion
    /// `SessionContext` via `iceberg-datafusion`'s `TableProvider`, so
    /// arbitrary predicate pushdown/joins work — needs that convergence.
    /// Calling this function once per window from `cubism-datafusion` (DF
    /// 54) and merging the resulting `RecordBatch`es directly, the same
    /// "cross the boundary via `arrow_array`/`arrow_schema` only" strategy
    /// this crate's top-level doc comment already describes, does not:
    /// both sides resolve to the same unified `arrow` 58.3.0 (confirmed via
    /// `cargo tree -i arrow --workspace`), so a `RecordBatch` from here is
    /// already the type DF54 expects. What that direct-call path cannot do
    /// is prune storage across many windows the way a real `TableProvider`
    /// would (this function's predicate is a single `(window_id, revision)`
    /// equality, not a semijoin) — see the roadmap's Milestone 10 for where
    /// that limit is recorded against the plan's completion criteria.
    pub async fn read_window(
        catalog: &dyn Catalog,
        temporal_table: &TemporalTable,
        publications: &PublicationStore,
        window_id: &WindowId,
    ) -> Result<Vec<RecordBatch>> {
        let current = publications
            .current(&temporal_table.cube_id, window_id)
            .await?
            .ok_or_else(|| CubismIcebergError::UnpublishedWindow(window_id.as_str().to_string()))?;

        let states_table = temporal_table.states_table(catalog).await?;
        let predicate = Reference::new(WINDOW_ID_COLUMN)
            .equal_to(Datum::string(window_id.as_str()))
            .and(Reference::new(REVISION_COLUMN).equal_to(Datum::long(current.get() as i64)));

        let scan = states_table
            .scan()
            .with_filter(predicate)
            .build()
            .map_err(CubismIcebergError::Iceberg)?;
        let stream = scan.to_arrow().await.map_err(CubismIcebergError::Iceberg)?;
        stream
            .try_collect()
            .await
            .map_err(CubismIcebergError::Iceberg)
    }

    /// Whether a states row exists for exactly `(window_id, revision,
    /// run_id)` in the states table's *current* snapshot, and if so, that
    /// snapshot's id — [issue #17](https://github.com/jeromebanks/cubism-rs/issues/17)'s
    /// fix: `RunState::Claimed` is ambiguous between "append never
    /// attempted" and "append's `fast_append` already committed, but the
    /// process crashed before `record_append` persisted that fact." This
    /// scan answers that ambiguity directly against Iceberg's own committed
    /// state, the same `(window_id, revision)` predicate [`Self::read_window`]
    /// uses plus a `run_id` clause — every states row already carries its
    /// own `run_id` (see [`crate::writer::AggregateWriter::append_window`]).
    ///
    /// The returned snapshot id is the table's *current* snapshot at scan
    /// time, not necessarily the exact snapshot the original (possibly
    /// crashed) commit produced — `aggregate_snapshot_id` is provenance-only
    /// (`control.rs`'s `RunState`/`Publication` never use it to filter a
    /// read), so "a snapshot in which this run's data is visible right now"
    /// is sufficient. Do not read more precision into it than that.
    ///
    /// **Not a defense against concurrent recovery of the same run.** Two
    /// callers racing `CorrectionCoordinator::execute` for the identical
    /// `run_id` could both observe `None` here before either commits, and
    /// both then append — the same latent race that exists today regardless
    /// of this fix. This function only closes the *sequential* crash-then-
    /// retry gap #17 describes; concurrent recovery of one run is out of
    /// scope, matching this crate's existing "one context per build if
    /// calling from concurrent tasks" precedent elsewhere (`cubism-datafusion`'s
    /// `build_temporal`) rather than a new locking primitive.
    ///
    /// **Does not verify `expected_rows`.** Checked before writing this:
    /// `RunState::expected_rows` is only ever cross-checked at `claim_run`
    /// time, against a *retried claim's own* `expected_rows` argument
    /// (`control.rs`'s `RunConflict` — catches a caller re-claiming the same
    /// `run_id` with different inputs than its first claim). Nothing in
    /// `record_append` or `publish` compares `expected_rows` against the
    /// actual row count Iceberg committed, on this path or the normal
    /// append path either — this fix does not weaken an existing
    /// invariant, because no such invariant exists to weaken. A caller of
    /// `CorrectionCoordinator::execute` that recovers a crashed run by
    /// re-supplying a *different* `states` batch than the one that actually
    /// committed would still not be caught by anything downstream of this
    /// function; that is a pre-existing gap in the crate's protocol, not
    /// one introduced or fixed here.
    pub async fn run_append_snapshot(
        catalog: &dyn Catalog,
        temporal_table: &TemporalTable,
        window_id: &WindowId,
        revision: WindowRevision,
        run_id: &str,
    ) -> Result<Option<i64>> {
        let states_table = temporal_table.states_table(catalog).await?;
        let predicate = Reference::new(WINDOW_ID_COLUMN)
            .equal_to(Datum::string(window_id.as_str()))
            .and(Reference::new(REVISION_COLUMN).equal_to(Datum::long(revision.get() as i64)))
            .and(Reference::new(RUN_ID_COLUMN).equal_to(Datum::string(run_id)));

        let scan = states_table
            .scan()
            .with_filter(predicate)
            .build()
            .map_err(CubismIcebergError::Iceberg)?;
        let stream = scan.to_arrow().await.map_err(CubismIcebergError::Iceberg)?;
        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(CubismIcebergError::Iceberg)?;

        if !batches.iter().any(|batch| batch.num_rows() > 0) {
            return Ok(None);
        }
        // A matching row was just scanned out of the table's current
        // snapshot, so it must have one — fail loudly rather than silently
        // returning `None` here, which the caller (`CorrectionCoordinator::execute`)
        // would read as "never appended" and re-append, reintroducing #17's
        // bug via this function's own internal inconsistency.
        match current_snapshot_id(states_table.metadata()) {
            Some(id) => Ok(Some(id)),
            None => Err(CubismIcebergError::Iceberg(iceberg::Error::new(
                iceberg::ErrorKind::Unexpected,
                "states table scanned a matching row but reports no current snapshot",
            ))),
        }
    }
}
