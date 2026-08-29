use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use chrono::DateTime;
use cubism_iceberg_spike::{
    ClaimResult, ControlStore, Publication, RunState, WindowId, normalize_utc_timezones,
};
use datafusion::arrow::array::{
    Int64Array, LargeBinaryArray, StringArray, TimestampMicrosecondArray, UInt64Array,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef, TimeUnit};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::execution::context::SessionContext;
use futures::TryStreamExt;
use iceberg::expr::Reference;
use iceberg::io::LocalFsStorageFactory;
use iceberg::memory::{MEMORY_CATALOG_WAREHOUSE, MemoryCatalogBuilder};
use iceberg::spec::{
    Datum, NestedField, PrimitiveType, Schema, Transform, Type, UnboundPartitionSpec,
};
use iceberg::table::Table;
use iceberg::{Catalog, CatalogBuilder, MemoryCatalog, NamespaceIdent, TableCreation, TableIdent};
use iceberg_datafusion::IcebergCatalogProvider;
use tempfile::TempDir;

const CATALOG_NAME: &str = "phase0a_catalog";
const NAMESPACE_NAME: &str = "phase0a";
const TABLE_NAME: &str = "temporal_aggregates";

static SOURCE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct AggregateRow {
    cube_id: String,
    window_id: String,
    revision: i64,
    run_id: String,
    bucket_start_micros: i64,
    xunit_id: String,
    measure_id: String,
    state_count: i64,
    state_blob: Vec<u8>,
}

impl AggregateRow {
    fn new(
        window_id: impl Into<String>,
        revision: i64,
        run_id: impl Into<String>,
        bucket_start: &str,
        state_count: i64,
    ) -> Self {
        Self {
            cube_id: "web-analytics".to_string(),
            window_id: window_id.into(),
            revision,
            run_id: run_id.into(),
            bucket_start_micros: micros(bucket_start),
            xunit_id: "sha256:country=US/device=mobile".to_string(),
            measure_id: "sessions".to_string(),
            state_count,
            state_blob: vec![0x43, 0x55, 0x42, 0x45, revision as u8],
        }
    }
}

struct IcebergFixture {
    _warehouse: TempDir,
    catalog: Arc<MemoryCatalog>,
    context: SessionContext,
    table_ident: TableIdent,
}

impl IcebergFixture {
    async fn new() -> Result<Self> {
        let warehouse = TempDir::new().context("create temporary Iceberg warehouse")?;
        let warehouse_path = warehouse.path().to_string_lossy().into_owned();
        let catalog = Arc::new(
            MemoryCatalogBuilder::default()
                .with_storage_factory(Arc::new(LocalFsStorageFactory))
                .load(
                    "phase0a-memory",
                    HashMap::from([(MEMORY_CATALOG_WAREHOUSE.to_string(), warehouse_path.clone())]),
                )
                .await
                .context("create Iceberg memory catalog")?,
        );

        let namespace = NamespaceIdent::new(NAMESPACE_NAME.to_string());
        catalog
            .create_namespace(&namespace, HashMap::new())
            .await
            .context("create Phase 0A namespace")?;

        let schema = Schema::builder()
            .with_schema_id(0)
            .with_fields(vec![
                NestedField::required(1, "cube_id", Type::Primitive(PrimitiveType::String)).into(),
                NestedField::required(2, "window_id", Type::Primitive(PrimitiveType::String))
                    .into(),
                NestedField::required(3, "revision", Type::Primitive(PrimitiveType::Long)).into(),
                NestedField::required(4, "run_id", Type::Primitive(PrimitiveType::String)).into(),
                NestedField::required(
                    5,
                    "bucket_start",
                    Type::Primitive(PrimitiveType::Timestamptz),
                )
                .into(),
                NestedField::required(6, "xunit_id", Type::Primitive(PrimitiveType::String)).into(),
                NestedField::required(7, "measure_id", Type::Primitive(PrimitiveType::String))
                    .into(),
                NestedField::required(8, "state_count", Type::Primitive(PrimitiveType::Long))
                    .into(),
                NestedField::required(9, "state_blob", Type::Primitive(PrimitiveType::Binary))
                    .into(),
            ])
            .build()
            .context("build aggregate table schema")?;
        let partition_spec = UnboundPartitionSpec::builder()
            .with_spec_id(0)
            .add_partition_field(5, "bucket_day", Transform::Day)
            .context("create day partition transform")?
            .build();
        let table_location = warehouse
            .path()
            .join(TABLE_NAME)
            .to_string_lossy()
            .into_owned();
        let creation = TableCreation::builder()
            .name(TABLE_NAME.to_string())
            .location(table_location)
            .schema(schema)
            .partition_spec(partition_spec)
            .properties(HashMap::new())
            .build();
        catalog
            .create_table(&namespace, creation)
            .await
            .context("create aggregate table")?;

        let context = SessionContext::new();
        let provider = Arc::new(
            IcebergCatalogProvider::try_new(catalog.clone())
                .await
                .context("create Iceberg DataFusion catalog provider")?,
        );
        context.register_catalog(CATALOG_NAME, provider);

        Ok(Self {
            _warehouse: warehouse,
            catalog,
            context,
            table_ident: TableIdent::new(namespace, TABLE_NAME.to_string()),
        })
    }

    async fn target_schema(&self) -> Result<SchemaRef> {
        let catalog = self
            .context
            .catalog(CATALOG_NAME)
            .context("registered catalog missing")?;
        let schema = catalog
            .schema(NAMESPACE_NAME)
            .context("registered schema missing")?;
        let table = schema
            .table(TABLE_NAME)
            .await
            .context("load DataFusion table provider")?
            .context("registered table missing")?;
        Ok(table.schema())
    }

    async fn batch(&self, rows: &[AggregateRow], timezone: &str) -> Result<RecordBatch> {
        let target_schema = self.target_schema().await?;
        let fields = target_schema
            .fields()
            .iter()
            .map(|field| {
                if field.name() == "bucket_start" {
                    Arc::new(
                        Field::new(
                            field.name(),
                            DataType::Timestamp(
                                TimeUnit::Microsecond,
                                Some(timezone.to_string().into()),
                            ),
                            field.is_nullable(),
                        )
                        .with_metadata(field.metadata().clone()),
                    )
                } else {
                    field.clone()
                }
            })
            .collect::<Vec<_>>();
        let schema = Arc::new(ArrowSchema::new_with_metadata(
            fields,
            target_schema.metadata().clone(),
        ));

        let timestamps = TimestampMicrosecondArray::from(
            rows.iter()
                .map(|row| Some(row.bucket_start_micros))
                .collect::<Vec<_>>(),
        )
        .with_timezone(timezone);
        let blobs =
            LargeBinaryArray::from_iter_values(rows.iter().map(|row| row.state_blob.as_slice()));

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| row.cube_id.as_str()),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| row.window_id.as_str()),
                )),
                Arc::new(Int64Array::from_iter_values(
                    rows.iter().map(|row| row.revision),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| row.run_id.as_str()),
                )),
                Arc::new(timestamps),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| row.xunit_id.as_str()),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| row.measure_id.as_str()),
                )),
                Arc::new(Int64Array::from_iter_values(
                    rows.iter().map(|row| row.state_count),
                )),
                Arc::new(blobs),
            ],
        )
        .context("build aggregate Arrow batch")
    }

    async fn append(&self, batch: RecordBatch) -> Result<usize> {
        let source_name = format!(
            "phase0a_source_{}",
            SOURCE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        self.context
            .register_batch(&source_name, batch)
            .context("register append source batch")?;

        let result = async {
            let dataframe = self
                .context
                .sql(&format!(
                    "INSERT INTO {CATALOG_NAME}.{NAMESPACE_NAME}.{TABLE_NAME} \
                     SELECT * FROM {source_name}"
                ))
                .await
                .context("plan Iceberg append")?;
            let batches = dataframe
                .collect()
                .await
                .context("execute Iceberg append")?;
            let count = batches
                .first()
                .and_then(|batch| batch.column(0).as_any().downcast_ref::<UInt64Array>())
                .map(|counts| counts.value(0) as usize)
                .context("Iceberg append did not return an inserted-row count")?;
            Ok(count)
        }
        .await;

        let _ = self.context.deregister_table(&source_name);
        result
    }

    async fn table(&self) -> Result<Table> {
        self.catalog
            .load_table(&self.table_ident)
            .await
            .context("reload current aggregate table")
    }

    async fn current_snapshot_id(&self) -> Result<i64> {
        let table = self.table().await?;
        table
            .metadata()
            .current_snapshot()
            .map(|snapshot| snapshot.snapshot_id())
            .context("aggregate table has no current snapshot")
    }

    async fn physical_row_count_for_run(&self, run_id: &str) -> Result<usize> {
        let run_id = sql_literal(run_id);
        let batches = self
            .context
            .sql(&format!(
                "SELECT COUNT(*) AS row_count \
                 FROM {CATALOG_NAME}.{NAMESPACE_NAME}.{TABLE_NAME} \
                 WHERE run_id = '{run_id}'"
            ))
            .await?
            .collect()
            .await?;
        let counts = batches
            .first()
            .and_then(|batch| batch.column(0).as_any().downcast_ref::<Int64Array>())
            .context("count query returned an unexpected type")?;
        Ok(counts.value(0) as usize)
    }

    async fn visible_rows(&self, store: &ControlStore) -> Result<Vec<VisibleRow>> {
        let publications = store.publications()?;
        let manifest_name = format!(
            "phase0a_manifest_{}",
            SOURCE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let manifest_batch = RecordBatch::try_from_iter(vec![
            (
                "cube_id",
                Arc::new(StringArray::from_iter_values(
                    publications
                        .iter()
                        .map(|(window, _)| window.cube_id.as_str()),
                )) as _,
            ),
            (
                "window_id",
                Arc::new(StringArray::from_iter_values(
                    publications
                        .iter()
                        .map(|(window, _)| window.window_id.as_str()),
                )) as _,
            ),
            (
                "current_revision",
                Arc::new(Int64Array::from_iter_values(
                    publications
                        .iter()
                        .map(|(_, publication)| publication.revision),
                )) as _,
            ),
        ])?;
        self.context
            .register_batch(&manifest_name, manifest_batch)
            .context("register current publication manifest")?;

        let result = async {
            let batches = self
                .context
                .sql(&format!(
                    "SELECT a.window_id, a.revision, a.run_id, a.state_count \
                     FROM {CATALOG_NAME}.{NAMESPACE_NAME}.{TABLE_NAME} a \
                     JOIN {manifest_name} m \
                       ON a.cube_id = m.cube_id \
                      AND a.window_id = m.window_id \
                      AND a.revision = m.current_revision \
                     ORDER BY a.window_id"
                ))
                .await?
                .collect()
                .await?;
            let mut rows = Vec::new();
            for batch in batches {
                let windows = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let revisions = batch
                    .column(1)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                let runs = batch
                    .column(2)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let counts = batch
                    .column(3)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                for index in 0..batch.num_rows() {
                    rows.push(VisibleRow {
                        window_id: windows.value(index).to_string(),
                        revision: revisions.value(index),
                        run_id: runs.value(index).to_string(),
                        state_count: counts.value(index),
                    });
                }
            }
            Ok(rows)
        }
        .await;

        let _ = self.context.deregister_table(&manifest_name);
        result
    }
}

#[derive(Debug, PartialEq, Eq)]
struct VisibleRow {
    window_id: String,
    revision: i64,
    run_id: String,
    state_count: i64,
}

fn micros(timestamp: &str) -> i64 {
    DateTime::parse_from_rfc3339(timestamp)
        .unwrap()
        .timestamp_micros()
}

fn sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

async fn append_and_record(
    fixture: &IcebergFixture,
    store: &ControlStore,
    run_id: &str,
    window: WindowId,
    revision: i64,
    row: AggregateRow,
) -> Result<Publication> {
    assert!(matches!(
        store.claim_run(run_id, window, revision, 1)?,
        ClaimResult::New(_)
    ));
    let batch = fixture.batch(&[row], "+00:00").await?;
    assert_eq!(fixture.append(batch).await?, 1);
    let snapshot_id = fixture.current_snapshot_id().await?;
    store.record_append(run_id, snapshot_id)?;
    store
        .publish(run_id, Some(revision - 1).filter(|revision| *revision > 0))
        .map_err(Into::into)
}

#[tokio::test]
async fn append_read_and_day_partition_pruning() -> Result<()> {
    let fixture = IcebergFixture::new().await?;
    let rows = vec![
        AggregateRow::new("2026-07-28", 1, "run-day-1", "2026-07-28T10:03:00Z", 10),
        AggregateRow::new("2026-07-29", 1, "run-day-2", "2026-07-29T11:47:00Z", 20),
    ];
    let batch = fixture.batch(&rows, "+00:00").await?;
    assert_eq!(fixture.append(batch).await?, 2);

    let table = fixture.table().await?;
    let all_tasks = table
        .scan()
        .build()?
        .plan_files()
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    assert_eq!(
        all_tasks.len(),
        2,
        "one append spanning two hidden day partitions should produce two files"
    );

    let start = micros("2026-07-28T00:00:00Z");
    let end = micros("2026-07-29T00:00:00Z");
    let predicate = Reference::new("bucket_start")
        .greater_than_or_equal_to(Datum::timestamptz_micros(start))
        .and(Reference::new("bucket_start").less_than(Datum::timestamptz_micros(end)));
    let selected_tasks = table
        .scan()
        .with_filter(predicate)
        .build()?
        .plan_files()
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    assert_eq!(
        selected_tasks.len(),
        1,
        "a one-day timestamp predicate should prune the other partition file"
    );
    assert_eq!(selected_tasks[0].record_count, Some(1));

    let batches = fixture
        .context
        .sql(&format!(
            "SELECT window_id, state_count \
             FROM {CATALOG_NAME}.{NAMESPACE_NAME}.{TABLE_NAME} \
             WHERE bucket_start >= TIMESTAMP '2026-07-28T00:00:00Z' \
               AND bucket_start < TIMESTAMP '2026-07-29T00:00:00Z'"
        ))
        .await?
        .collect()
        .await?;
    assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
    Ok(())
}

#[tokio::test]
async fn named_utc_datafusion_append_and_explicit_normalization_round_trip() -> Result<()> {
    let fixture = IcebergFixture::new().await?;
    let direct_row = AggregateRow::new(
        "2026-07-28",
        1,
        "run-utc-direct",
        "2026-07-28T10:03:00Z",
        10,
    );
    let named_utc = fixture.batch(&[direct_row], "UTC").await?;
    assert_eq!(fixture.append(named_utc).await?, 1);
    assert_eq!(
        fixture.physical_row_count_for_run("run-utc-direct").await?,
        1
    );

    let normalized_row = AggregateRow::new(
        "2026-07-28",
        1,
        "run-utc-normalized",
        "2026-07-28T10:04:00Z",
        11,
    );
    let named_utc = fixture.batch(&[normalized_row], "UTC").await?;
    let normalized = normalize_utc_timezones(&named_utc)?;
    assert_eq!(
        normalized.schema().field(4).data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into()))
    );
    assert_eq!(fixture.append(normalized).await?, 1);
    assert_eq!(
        fixture
            .physical_row_count_for_run("run-utc-normalized")
            .await?,
        1
    );

    let batches = fixture
        .context
        .sql(&format!(
            "SELECT run_id, bucket_start \
             FROM {CATALOG_NAME}.{NAMESPACE_NAME}.{TABLE_NAME} \
             WHERE run_id IN ('run-utc-direct', 'run-utc-normalized') \
             ORDER BY run_id"
        ))
        .await?
        .collect()
        .await?;
    let timestamps = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .context("round-trip timestamp had an unexpected Arrow type")?;
    assert_eq!(timestamps.value(0), micros("2026-07-28T10:03:00Z"));
    assert_eq!(timestamps.value(1), micros("2026-07-28T10:04:00Z"));
    assert_eq!(timestamps.timezone(), Some("+00:00"));
    Ok(())
}

#[tokio::test]
async fn retry_recovery_and_manifest_semijoin_control_visibility() -> Result<()> {
    let fixture = IcebergFixture::new().await?;
    let store = ControlStore::default();
    let window_one = WindowId::new("web-analytics", "2026-07-28");
    let window_two = WindowId::new("web-analytics", "2026-07-29");

    // Simulate a crash after the atomic Iceberg append but before the control
    // store records its snapshot. Recovery finds the deterministic run_id and
    // expected row count, records the snapshot, and does not append twice.
    assert!(matches!(
        store.claim_run("run-w1-r1", window_one.clone(), 1, 1)?,
        ClaimResult::New(_)
    ));
    let row = AggregateRow::new("2026-07-28", 1, "run-w1-r1", "2026-07-28T10:03:00Z", 10);
    let batch = fixture.batch(&[row], "+00:00").await?;
    assert_eq!(fixture.append(batch).await?, 1);
    assert!(matches!(
        store.claim_run("run-w1-r1", window_one.clone(), 1, 1)?,
        ClaimResult::Existing(RunState::Claimed { .. })
    ));
    assert_eq!(fixture.physical_row_count_for_run("run-w1-r1").await?, 1);
    store.record_append("run-w1-r1", fixture.current_snapshot_id().await?)?;
    store.publish("run-w1-r1", None)?;

    // Retrying a published run is a no-op at the orchestration boundary.
    assert!(matches!(
        store.claim_run("run-w1-r1", window_one.clone(), 1, 1)?,
        ClaimResult::Existing(RunState::Published { .. })
    ));
    assert_eq!(fixture.physical_row_count_for_run("run-w1-r1").await?, 1);

    append_and_record(
        &fixture,
        &store,
        "run-w2-r1",
        window_two,
        1,
        AggregateRow::new("2026-07-29", 1, "run-w2-r1", "2026-07-29T11:47:00Z", 20),
    )
    .await?;

    // Revision 2 is physically committed but remains invisible.
    assert!(matches!(
        store.claim_run("run-w1-r2", window_one.clone(), 2, 1)?,
        ClaimResult::New(_)
    ));
    let replacement = AggregateRow::new("2026-07-28", 2, "run-w1-r2", "2026-07-28T10:03:00Z", 12);
    let batch = fixture.batch(&[replacement], "+00:00").await?;
    assert_eq!(fixture.append(batch).await?, 1);
    let revision_two_snapshot = fixture.current_snapshot_id().await?;
    store.record_append("run-w1-r2", revision_two_snapshot)?;

    assert_eq!(
        fixture.visible_rows(&store).await?,
        vec![
            VisibleRow {
                window_id: "2026-07-28".to_string(),
                revision: 1,
                run_id: "run-w1-r1".to_string(),
                state_count: 10,
            },
            VisibleRow {
                window_id: "2026-07-29".to_string(),
                revision: 1,
                run_id: "run-w2-r1".to_string(),
                state_count: 20,
            },
        ]
    );

    store.publish("run-w1-r2", Some(1))?;
    assert_eq!(
        fixture.visible_rows(&store).await?,
        vec![
            VisibleRow {
                window_id: "2026-07-28".to_string(),
                revision: 2,
                run_id: "run-w1-r2".to_string(),
                state_count: 12,
            },
            VisibleRow {
                window_id: "2026-07-29".to_string(),
                revision: 1,
                run_id: "run-w2-r1".to_string(),
                state_count: 20,
            },
        ]
    );

    let publications = store.publications()?;
    assert_ne!(
        publications[0].1.aggregate_snapshot_id, publications[1].1.aggregate_snapshot_id,
        "windows may be published from different aggregate snapshots"
    );
    assert_eq!(
        publications[0].1.aggregate_snapshot_id,
        revision_two_snapshot
    );
    Ok(())
}
