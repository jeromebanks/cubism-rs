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
