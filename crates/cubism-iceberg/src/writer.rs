use std::collections::HashSet;
use std::sync::Arc;

use arrow_array::{Array, Date32Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema as ArrowSchema};
use cubism_core::temporal::{WindowId, WindowRevision};
use iceberg::arrow::FieldMatchMode;
use iceberg::spec::{DataFile, DataFileFormat, Literal, PartitionKey, Struct, Transform};
use iceberg::table::Table;
use iceberg::transaction::{ApplyTransactionAction, Transaction};
use iceberg::transform::create_transform_function;
use iceberg::writer::base_writer::data_file_writer::DataFileWriterBuilder;
use iceberg::writer::file_writer::ParquetWriterBuilder;
use iceberg::writer::file_writer::location_generator::{
    DefaultFileNameGenerator, DefaultLocationGenerator,
};
use iceberg::writer::file_writer::rolling_writer::RollingFileWriterBuilder;
use iceberg::writer::partitioning::PartitioningWriter;
use iceberg::writer::partitioning::clustered_writer::ClusteredWriter;
use iceberg::writer::{IcebergWriter, IcebergWriterBuilder};
use iceberg::Catalog;
use parquet::file::properties::WriterProperties;

use crate::error::{CubismIcebergError, Result};
use crate::schema::{REVISION_COLUMN, RUN_ID_COLUMN, WINDOW_ID_COLUMN};
use crate::table::{current_snapshot_id, TemporalTable};

/// Result of one successful `append_window` commit. `snapshot_id` is the
/// **exact** committed states-table snapshot ID, read back off the `Table`
/// `Transaction::commit` returns — never a row count. That distinction is
/// Phase 0A gate #4: DataFusion `INSERT` only reports rows inserted, not
/// the snapshot it landed in; this writer never goes through DataFusion
/// `INSERT` at all, so the gate does not apply here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResult {
    pub snapshot_id: i64,
    pub states_file_count: usize,
    pub registry_file_count: usize,
    pub row_count: u64,
}

/// One build's identity and payload for [`AggregateWriter::append_window`].
/// `cube_id` is not part of this struct — it comes from the
/// `TemporalTable` passed alongside it (`TemporalTable::create`'s
/// `cube_id` argument), the single source of truth for which cube's
/// tables are being written.
pub struct AppendWindow<'a> {
    pub window_id: &'a WindowId,
    pub revision: WindowRevision,
    pub run_id: &'a str,
    pub states: &'a [RecordBatch],
    pub registry: &'a [RecordBatch],
}

pub struct AggregateWriter;

impl AggregateWriter {
    /// Append one build's states and registry batches to `temporal_table`,
    /// tagging every states row with `window_id`/`revision`/`run_id`.
    ///
    /// The registry append commits first: it is a content-addressed,
    /// idempotent-to-repeat dictionary (same `xunit_id` always maps to the
    /// same canonical bytes), so it is safe to make visible before the run
    /// it belongs to is durable. The states append — the run's actual
    /// identity, gated by `run_id`/`revision` — commits second and is what
    /// `CommitResult::snapshot_id` reports. A crash between the two commits
    /// leaves at most a harmless extra registry entry, never a states row
    /// without provenance.
    ///
    /// `states`/`registry` batches must already match
    /// `cubism_datafusion::temporal_build::temporal_state_schema` and the
    /// registry's `(xunit_id, xunit_canonical)` shape respectively, and the
    /// `states` batches must be sorted by `bucket_start` ascending overall
    /// (Phase 2's own guarantee) — this writer relies on that order to
    /// emit day-partitioned files via a [`ClusteredWriter`], which requires
    /// its partition keys to arrive in sorted order.
    pub async fn append_window(
        catalog: &dyn Catalog,
        temporal_table: &TemporalTable,
        window: AppendWindow<'_>,
    ) -> Result<CommitResult> {
        let AppendWindow { window_id, revision, run_id, states, registry } = window;

        let registry_table = temporal_table.registry_table(catalog).await?;
        let registry_files = write_unpartitioned(&registry_table, run_id, "registry", registry).await?;
        assert_unique_paths(registry_files.iter().map(DataFile::file_path))?;
        let registry_file_count = registry_files.len();
        if !registry_files.is_empty() {
            commit_append(catalog, &registry_table, registry_files).await?;
        }

        let augmented_states: Vec<RecordBatch> = states
            .iter()
            .map(|batch| augment_states_batch(batch, window_id.as_str(), revision.get() as i64, run_id))
            .collect::<Result<_>>()?;

        let states_table = temporal_table.states_table(catalog).await?;
        let states_files = write_states_partitioned(&states_table, run_id, &augmented_states).await?;
        assert_unique_paths(states_files.iter().map(DataFile::file_path))?;

        let row_count: u64 = states_files.iter().map(DataFile::record_count).sum();
        let states_file_count = states_files.len();
        let committed = commit_append(catalog, &states_table, states_files).await?;
        let snapshot_id = current_snapshot_id(committed.metadata())
            .ok_or_else(|| CubismIcebergError::Iceberg(iceberg::Error::new(
                iceberg::ErrorKind::Unexpected,
                "states table has no current snapshot immediately after a successful commit",
            )))?;

        Ok(CommitResult {
            snapshot_id,
            states_file_count,
            registry_file_count,
            row_count,
        })
    }
}

/// Reject a duplicate data-file path within one append, before it ever
/// reaches `fast_append`. Deterministic file naming (see
/// [`write_states_partitioned`]/[`write_unpartitioned`]'s
/// `DefaultFileNameGenerator` prefix) makes a real collision here
/// effectively impossible in practice, but the plan's writer protocol
/// requires the defense explicitly (Phase 0A gate #3), so it is a real
/// check, not just an assumption.
fn assert_unique_paths<'a>(paths: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = HashSet::new();
    for path in paths {
        if !seen.insert(path) {
            return Err(CubismIcebergError::DuplicateDataFilePath(path.to_string()));
        }
    }
    Ok(())
}

async fn commit_append(catalog: &dyn Catalog, table: &Table, files: Vec<DataFile>) -> Result<Table> {
    let tx = Transaction::new(table);
    let action = tx.fast_append().add_data_files(files);
    let tx = action.apply(tx).map_err(CubismIcebergError::Iceberg)?;
    tx.commit(catalog).await.map_err(CubismIcebergError::Iceberg)
}

/// Day-partition `bucket_start` and write via [`ClusteredWriter`] — sorted
/// data, one active writer at a time, matching Phase 2's own guaranteed
/// output order instead of the unsorted `FanoutWriter` alternative.
async fn write_states_partitioned(
    table: &Table,
    run_id: &str,
    batches: &[RecordBatch],
) -> Result<Vec<DataFile>> {
    let schema = table.metadata().current_schema().clone();
    let partition_spec = table.metadata().default_partition_spec().as_ref().clone();
    let location_generator = DefaultLocationGenerator::new(table.metadata()).map_err(CubismIcebergError::Iceberg)?;
    let file_name_generator =
        DefaultFileNameGenerator::new(format!("states-{run_id}"), None, DataFileFormat::Parquet);
    // Callers build their Arrow batches by column name (see
    // `augment_states_batch`), not with Iceberg's `PARQUET:field_id` Arrow
    // metadata — match by name rather than the crate's default ID mode.
    let parquet_writer_builder = ParquetWriterBuilder::new(WriterProperties::default(), schema.clone())
        .with_match_mode(FieldMatchMode::Name);
    let rolling_writer_builder = RollingFileWriterBuilder::new_with_default_file_size(
        parquet_writer_builder,
        table.file_io().clone(),
        location_generator,
        file_name_generator,
    );
    let data_file_writer_builder = DataFileWriterBuilder::new(rolling_writer_builder);
    let mut writer = ClusteredWriter::new(data_file_writer_builder);

    let day_transform = create_transform_function(&Transform::Day).map_err(CubismIcebergError::Iceberg)?;

    for batch in batches {
        if batch.num_rows() == 0 {
            continue;
        }
        let bucket_start = batch
            .column_by_name("bucket_start")
            .ok_or_else(|| CubismIcebergError::Iceberg(iceberg::Error::new(
                iceberg::ErrorKind::DataInvalid,
                "states batch is missing its 'bucket_start' column",
            )))?;
        let days = day_transform.transform(bucket_start.clone()).map_err(CubismIcebergError::Iceberg)?;
        let days = days
            .as_any()
            .downcast_ref::<Date32Array>()
            .expect("Transform::Day always produces a Date32Array");

        let mut start = 0usize;
        for i in 1..=batch.num_rows() {
            if i == batch.num_rows() || days.value(i) != days.value(start) {
                let slice = batch.slice(start, i - start);
                let key = Struct::from_iter([Some(Literal::date(days.value(start)))]);
                let partition_key = PartitionKey::new(partition_spec.clone(), schema.clone(), key);
                writer
                    .write(partition_key, slice)
                    .await
                    .map_err(CubismIcebergError::Iceberg)?;
                start = i;
            }
        }
    }

    writer.close().await.map_err(CubismIcebergError::Iceberg)
}

/// The `xunit_registry` table has no time column to partition on.
async fn write_unpartitioned(
    table: &Table,
    run_id: &str,
    prefix: &str,
    batches: &[RecordBatch],
) -> Result<Vec<DataFile>> {
    let schema = table.metadata().current_schema().clone();
    let location_generator = DefaultLocationGenerator::new(table.metadata()).map_err(CubismIcebergError::Iceberg)?;
    let file_name_generator =
        DefaultFileNameGenerator::new(format!("{prefix}-{run_id}"), None, DataFileFormat::Parquet);
    // Callers build their Arrow batches by column name (see
    // `augment_states_batch`), not with Iceberg's `PARQUET:field_id` Arrow
    // metadata — match by name rather than the crate's default ID mode.
    let parquet_writer_builder = ParquetWriterBuilder::new(WriterProperties::default(), schema.clone())
        .with_match_mode(FieldMatchMode::Name);
    let rolling_writer_builder = RollingFileWriterBuilder::new_with_default_file_size(
        parquet_writer_builder,
        table.file_io().clone(),
        location_generator,
        file_name_generator,
    );
    let data_file_writer_builder = DataFileWriterBuilder::new(rolling_writer_builder);
    let mut writer = data_file_writer_builder.build(None).await.map_err(CubismIcebergError::Iceberg)?;

    for batch in batches {
        if batch.num_rows() == 0 {
            continue;
        }
        writer.write(batch.clone()).await.map_err(CubismIcebergError::Iceberg)?;
    }

    writer.close().await.map_err(CubismIcebergError::Iceberg)
}

/// Prepend `window_id`/`revision`/`run_id` constant columns to a Phase-2
/// states batch, matching [`crate::schema::states_iceberg_schema`]'s field
/// order and types exactly.
fn augment_states_batch(batch: &RecordBatch, window_id: &str, revision: i64, run_id: &str) -> Result<RecordBatch> {
    let n = batch.num_rows();
    let window_col: Arc<dyn Array> = Arc::new(StringArray::from_iter_values(std::iter::repeat_n(window_id, n)));
    let revision_col: Arc<dyn Array> = Arc::new(Int64Array::from(vec![revision; n]));
    let run_col: Arc<dyn Array> = Arc::new(StringArray::from_iter_values(std::iter::repeat_n(run_id, n)));

    let mut fields = vec![
        Arc::new(Field::new(WINDOW_ID_COLUMN, DataType::Utf8, false)),
        Arc::new(Field::new(REVISION_COLUMN, DataType::Int64, false)),
        Arc::new(Field::new(RUN_ID_COLUMN, DataType::Utf8, false)),
    ];
    fields.extend(batch.schema().fields().iter().cloned());
    let schema = Arc::new(ArrowSchema::new(fields));

    let mut columns: Vec<Arc<dyn Array>> = vec![window_col, revision_col, run_col];
    columns.extend(batch.columns().iter().cloned());

    RecordBatch::try_new(schema, columns).map_err(CubismIcebergError::Arrow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_paths_pass_through() {
        assert!(assert_unique_paths(["a.parquet", "b.parquet"].into_iter()).is_ok());
    }

    #[test]
    fn a_duplicate_path_is_rejected_before_commit() {
        let error = assert_unique_paths(["a.parquet", "b.parquet", "a.parquet"].into_iter()).unwrap_err();
        assert!(matches!(error, CubismIcebergError::DuplicateDataFilePath(path) if path == "a.parquet"));
    }

    #[test]
    fn augmenting_a_batch_prepends_identity_columns_in_schema_order() {
        let inner = RecordBatch::try_new(
            std::sync::Arc::new(ArrowSchema::new(vec![Field::new("x", DataType::Int64, false)])),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();
        let augmented = augment_states_batch(&inner, "w-1", 7, "run-1").unwrap();
        let schema = augmented.schema();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, vec![WINDOW_ID_COLUMN, REVISION_COLUMN, RUN_ID_COLUMN, "x"]);
        assert_eq!(augmented.num_rows(), 3);
    }
}
