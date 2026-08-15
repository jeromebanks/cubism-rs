use arrow_array::RecordBatch;
use cubism_core::temporal::WindowId;
use futures::TryStreamExt;
use iceberg::Catalog;
use iceberg::expr::Reference;
use iceberg::spec::Datum;

use crate::control::PublicationStore;
use crate::error::{CubismIcebergError, Result};
use crate::schema::{REVISION_COLUMN, WINDOW_ID_COLUMN};
use crate::table::TemporalTable;

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
        stream.try_collect().await.map_err(CubismIcebergError::Iceberg)
    }
}
