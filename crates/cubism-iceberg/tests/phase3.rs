//! Phase 3 integration tests: re-proves each Phase 0A finding
//! (`docs/PHASE_0A_RESULTS.md`) against the real production schema (the
//! states and xunit_registry tables, both table-per-cube) instead of the
//! spike's placeholder long-form schema, and exercises the orchestration
//! contract (`PublicationStore` plus `AggregateWriter` plus
//! `AggregateReader` together) that makes retries and crash-recovery safe.

use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, TimestampMicrosecondArray};
use arrow_array::FixedSizeBinaryArray;
use arrow_schema::{DataType, Field, Schema as ArrowSchema, TimeUnit};
use chrono::DateTime;
use cubism_core::temporal::{WindowId, WindowRevision};
use cubism_iceberg::{
    AggregateReader, AggregateWriter, AppendWindow, CatalogConfig, ClaimResult, CubismIcebergError,
    PublicationStore, RunState, TemporalTable,
};
use futures::TryStreamExt;
use iceberg::expr::Reference;
use iceberg::spec::{Datum, Transform};
use iceberg::Catalog;
use tempfile::TempDir;

const CUBE_ID: &str = "web_analytics";

fn micros(timestamp: &str) -> i64 {
    DateTime::parse_from_rfc3339(timestamp).unwrap().timestamp_micros()
}

fn xunit_id(tag: u8) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[0] = tag;
    bytes
}

/// Mirrors `cubism_datafusion::temporal_build::temporal_state_schema`'s
/// shape (`bucket_start, xunit_id, <measure>...`) without depending on
/// `cubism-datafusion` — this crate never links DataFusion (see `src/lib.rs`).
fn sample_states_schema() -> ArrowSchema {
    ArrowSchema::new(vec![
        Field::new(
            "bucket_start",
            DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
            true,
        ),
        Field::new("xunit_id", DataType::FixedSizeBinary(32), true),
        Field::new("count_v1", DataType::Int64, false),
        Field::new("sum_v1", DataType::Float64, true),
    ])
}

fn states_batch(rows: &[(&str, [u8; 32], i64, f64)]) -> RecordBatch {
    let bucket_start = TimestampMicrosecondArray::from_iter_values(rows.iter().map(|(t, _, _, _)| micros(t)))
        .with_timezone("+00:00");
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(_, x, _, _)| *x)).unwrap();
    let count = Int64Array::from_iter_values(rows.iter().map(|(_, _, c, _)| *c));
    let sum = Float64Array::from_iter_values(rows.iter().map(|(_, _, _, s)| *s));
    RecordBatch::try_new(
        Arc::new(sample_states_schema()),
        vec![Arc::new(bucket_start), Arc::new(xunit_id), Arc::new(count), Arc::new(sum)],
    )
    .unwrap()
}

fn registry_batch(rows: &[([u8; 32], &[u8])]) -> RecordBatch {
    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new("xunit_id", DataType::FixedSizeBinary(32), false),
        Field::new("xunit_canonical", DataType::Binary, false),
    ]));
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(x, _)| *x)).unwrap();
    let canonical = arrow_array::BinaryArray::from_iter_values(rows.iter().map(|(_, c)| *c));
    RecordBatch::try_new(schema, vec![Arc::new(xunit_id), Arc::new(canonical)]).unwrap()
}

fn total_rows(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}

struct Fixture {
    _warehouse: TempDir,
    catalog: Arc<dyn Catalog>,
    temporal_table: TemporalTable,
    publications: PublicationStore,
}

impl Fixture {
    async fn new() -> Self {
        let warehouse = TempDir::new().unwrap();
        let config = CatalogConfig::Memory { warehouse: warehouse.path().to_path_buf() };
        let catalog = cubism_iceberg::config::open_catalog(&config).await.unwrap();
        let temporal_table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &sample_states_schema())
            .await
            .unwrap();
        Self { _warehouse: warehouse, catalog, temporal_table, publications: PublicationStore::new() }
    }

    async fn append(
        &self,
        window_id: &WindowId,
        revision: WindowRevision,
        run_id: &str,
        states: &[RecordBatch],
        registry: &[RecordBatch],
    ) -> cubism_iceberg::CommitResult {
        let expected_rows: u64 = states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
        let claim = self
            .publications
            .claim_run(CUBE_ID, window_id, run_id, revision, expected_rows)
            .unwrap();
        assert!(matches!(claim, ClaimResult::New(_)), "expected a fresh claim for {run_id}");

        let result = AggregateWriter::append_window(
            self.catalog.as_ref(),
            &self.temporal_table,
            AppendWindow { window_id, revision, run_id, states, registry },
        )
        .await
        .unwrap();
        self.publications.record_append(run_id, result.snapshot_id).unwrap();
        result
    }

    async fn read(&self, window_id: &WindowId) -> cubism_iceberg::error::Result<Vec<RecordBatch>> {
        AggregateReader::read_window(self.catalog.as_ref(), &self.temporal_table, &self.publications, window_id).await
    }
}

#[tokio::test]
async fn states_and_registry_tables_have_the_expected_iceberg_schema_partition_and_sort() {
    let fixture = Fixture::new().await;

    let states_table = fixture.temporal_table.states_table(fixture.catalog.as_ref()).await.unwrap();
    let metadata = states_table.metadata();
    let schema = metadata.current_schema();
    assert_eq!(schema.field_by_id(1).unwrap().name, "window_id");
    assert_eq!(schema.field_by_id(2).unwrap().name, "revision");
    assert_eq!(schema.field_by_id(3).unwrap().name, "run_id");
    assert_eq!(schema.field_by_id(4).unwrap().name, "bucket_start");
    assert_eq!(schema.field_by_id(5).unwrap().name, "xunit_id");
    assert_eq!(schema.field_by_id(6).unwrap().name, "count_v1");
    assert_eq!(schema.field_by_id(7).unwrap().name, "sum_v1");

    let partition_fields = metadata.default_partition_spec().fields();
    assert_eq!(partition_fields.len(), 1);
    assert_eq!(partition_fields[0].source_id, 4, "partitions on bucket_start's field id");
    assert_eq!(partition_fields[0].transform, Transform::Day);

    let sort_fields = &metadata.default_sort_order().fields;
    assert_eq!(sort_fields.len(), 2);
    assert_eq!(sort_fields[0].source_id, 4);
    assert_eq!(sort_fields[1].source_id, 5);

    let registry_table = fixture.temporal_table.registry_table(fixture.catalog.as_ref()).await.unwrap();
    let registry_metadata = registry_table.metadata();
    assert_eq!(registry_metadata.current_schema().field_by_id(1).unwrap().name, "xunit_id");
    assert_eq!(registry_metadata.current_schema().field_by_id(2).unwrap().name, "xunit_canonical");
    assert!(registry_metadata.default_partition_spec().fields().is_empty());
}

#[tokio::test]
async fn append_window_returns_the_exact_committed_snapshot_id_and_row_count() {
    let fixture = Fixture::new().await;
    let window_id = WindowId::new("2026-08-12").unwrap();
    let states = vec![states_batch(&[
        ("2026-08-12T00:10:00Z", xunit_id(1), 3, 9.0),
        ("2026-08-12T00:20:00Z", xunit_id(2), 5, 11.0),
    ])];
    let registry = vec![registry_batch(&[(xunit_id(1), b"US/mobile"), (xunit_id(2), b"EU/desktop")])];

    let result = fixture.append(&window_id, WindowRevision::new(1).unwrap(), "run-1", &states, &registry).await;
    assert_eq!(result.row_count, 2);

    let states_table = fixture.temporal_table.states_table(fixture.catalog.as_ref()).await.unwrap();
    let actual_snapshot = states_table.metadata().current_snapshot().unwrap().snapshot_id();
    assert_eq!(result.snapshot_id, actual_snapshot, "CommitResult must report the exact committed snapshot, not a row count");
}

#[tokio::test]
async fn retrying_the_same_run_id_after_a_simulated_crash_does_not_double_append() {
    let fixture = Fixture::new().await;
    let window_id = WindowId::new("2026-08-12").unwrap();
    let revision = WindowRevision::new(1).unwrap();
    let states = vec![states_batch(&[("2026-08-12T00:10:00Z", xunit_id(1), 3, 9.0)])];
    let registry = vec![registry_batch(&[(xunit_id(1), b"US/mobile")])];

    let first = fixture.append(&window_id, revision, "run-1", &states, &registry).await;

    // Simulate a process restart: the caller re-claims the same run_id
    // before it ever gets to `publish`.
    let retry_claim = fixture.publications.claim_run(CUBE_ID, &window_id, "run-1", revision, 1).unwrap();
    let recovered_snapshot = match retry_claim {
        ClaimResult::Existing(RunState::Appended { .. }) => {
            fixture.publications.run_state("run-1")
        }
        other => panic!("expected Existing(Appended {{ .. }}), got {other:?}"),
    };
    assert!(recovered_snapshot.is_some(), "recovered run state must be observable without a second append");

    // The caller sees the run is already appended and skips straight to
    // publish — no second `AggregateWriter::append_window` call.
    fixture.publications.publish("run-1", None).unwrap();

    let batches = fixture.read(&window_id).await.unwrap();
    assert_eq!(total_rows(&batches), 1, "a recovered retry must not have doubled the physical row count");

    let states_table = fixture.temporal_table.states_table(fixture.catalog.as_ref()).await.unwrap();
    assert_eq!(states_table.metadata().current_snapshot().unwrap().snapshot_id(), first.snapshot_id);
}

#[tokio::test]
async fn an_unpublished_revision_is_invisible_to_the_reader() {
    let fixture = Fixture::new().await;
    let window_id = WindowId::new("2026-08-12").unwrap();
    let states = vec![states_batch(&[("2026-08-12T00:10:00Z", xunit_id(1), 1, 1.0)])];
    let registry = vec![registry_batch(&[(xunit_id(1), b"US/mobile")])];
    fixture.append(&window_id, WindowRevision::new(1).unwrap(), "run-1", &states, &registry).await;

    let error = fixture.read(&window_id).await.unwrap_err();
    assert!(matches!(error, CubismIcebergError::UnpublishedWindow(_)));
}

#[tokio::test]
async fn publishing_a_new_revision_replaces_visibility_of_the_prior_one() {
    let fixture = Fixture::new().await;
    let window_id = WindowId::new("2026-08-12").unwrap();

    let states_v1 = vec![states_batch(&[
        ("2026-08-12T00:10:00Z", xunit_id(1), 1, 1.0),
        ("2026-08-12T00:20:00Z", xunit_id(2), 2, 2.0),
    ])];
    let registry_v1 = vec![registry_batch(&[(xunit_id(1), b"a"), (xunit_id(2), b"b")])];
    fixture.append(&window_id, WindowRevision::new(1).unwrap(), "run-1", &states_v1, &registry_v1).await;
    fixture.publications.publish("run-1", None).unwrap();

    let visible_v1 = fixture.read(&window_id).await.unwrap();
    assert_eq!(total_rows(&visible_v1), 2);

    let states_v2 = vec![states_batch(&[("2026-08-12T00:15:00Z", xunit_id(3), 9, 9.0)])];
    let registry_v2 = vec![registry_batch(&[(xunit_id(3), b"c")])];
    fixture.append(&window_id, WindowRevision::new(2).unwrap(), "run-2", &states_v2, &registry_v2).await;
    let stale = fixture.publications.publish("run-2", Some(WindowRevision::new(99).unwrap()));
    assert!(stale.is_err(), "publish must reject a caller that observed the wrong current revision");
    fixture.publications.publish("run-2", Some(WindowRevision::new(1).unwrap())).unwrap();

    let visible_v2 = fixture.read(&window_id).await.unwrap();
    assert_eq!(total_rows(&visible_v2), 1, "only revision 2's row should be visible, not both revisions summed");
}

#[tokio::test]
async fn day_partition_pruning_plans_one_file_for_a_bounded_scan() {
    let fixture = Fixture::new().await;
    let window_id = WindowId::new("2026-08-12").unwrap();
    let states = vec![states_batch(&[
        ("2026-08-12T10:00:00Z", xunit_id(1), 1, 1.0),
        ("2026-08-13T11:00:00Z", xunit_id(2), 2, 2.0),
    ])];
    let registry = vec![registry_batch(&[(xunit_id(1), b"a"), (xunit_id(2), b"b")])];
    fixture.append(&window_id, WindowRevision::new(1).unwrap(), "run-1", &states, &registry).await;

    let states_table = fixture.temporal_table.states_table(fixture.catalog.as_ref()).await.unwrap();
    let all_tasks = states_table.scan().build().unwrap().plan_files().await.unwrap().try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(all_tasks.len(), 2, "one append spanning two days should produce two day-partitioned files");

    let start = micros("2026-08-12T00:00:00Z");
    let end = micros("2026-08-13T00:00:00Z");
    let predicate = Reference::new("bucket_start")
        .greater_than_or_equal_to(Datum::timestamptz_micros(start))
        .and(Reference::new("bucket_start").less_than(Datum::timestamptz_micros(end)));
    let selected = states_table
        .scan()
        .with_filter(predicate)
        .build()
        .unwrap()
        .plan_files()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(selected.len(), 1, "a one-day predicate should prune the other day's file");
    assert_eq!(selected[0].record_count, Some(1));
}
