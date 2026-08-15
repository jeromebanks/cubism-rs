//! Milestone 7 spike (`docs/TIMESERIES_ROADMAP.md`): proves — by compiling
//! and running, not by reasoning from `cargo tree` — that a `RecordBatch`
//! read back from `cubism-iceberg`'s `AggregateReader::read_window` can be
//! handed straight to a DataFusion 54 `SessionContext` with **no**
//! `iceberg-datafusion` dependency anywhere in the call path and no
//! conversion step. Every array/schema type here comes from
//! `datafusion::arrow::*` (DF54's own re-export), never a separately
//! declared `arrow-array`/`arrow-schema` crate; if `arrow` were not unified
//! across `cubism-iceberg`'s DF53-era `iceberg` dependency and this crate's
//! DF54, `AggregateWriter::append_window`/`AggregateReader::read_window`
//! below would fail to compile against these types. The compile succeeding
//! *is* the assertion this milestone exists to make.
//!
//! What this test does **not** prove: it says nothing about
//! `iceberg-datafusion`/DF53 `TableProvider` registration (still gated on
//! #8's SQL/pushdown half), nothing about scanning more than one window
//! (`read_window`'s predicate is a single `(window_id, revision)`
//! equality — see the roadmap's Milestone 10 for that gap), and nothing
//! about `xunit_id`'s `FixedSizeBinary(32)` or `bucket_start`'s
//! tz-annotated timestamp decoding through DataFusion — only `count_v1`
//! (plain `Int64`) is asserted on after the round-trip.

use std::sync::Arc;

use chrono::DateTime;
use cubism_iceberg::config::open_catalog;
use cubism_iceberg::{
    AggregateReader, AggregateWriter, AppendWindow, CatalogConfig, ClaimResult, PublicationStore,
    TemporalTable,
};
use cubism_core::temporal::{WindowId, WindowRevision};
use datafusion::arrow::array::{Float64Array, Int64Array, RecordBatch};
use datafusion::arrow::datatypes::{DataType, Field, Schema as ArrowSchema, TimeUnit};
use datafusion::execution::context::SessionContext;
use tempfile::TempDir;

const CUBE_ID: &str = "web_analytics";

fn micros(timestamp: &str) -> i64 {
    DateTime::parse_from_rfc3339(timestamp).unwrap().timestamp_micros()
}

/// Mirrors `cubism_datafusion::temporal_build::temporal_state_schema`'s
/// shape without going through a `CubeSpec` — same simplification
/// `cubism-iceberg`'s own `tests/phase3.rs::sample_states_schema` makes,
/// built here via `datafusion::arrow::datatypes` instead so the whole
/// round-trip stays on DF54's own arrow re-export.
fn states_schema() -> ArrowSchema {
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
    use datafusion::arrow::array::{FixedSizeBinaryArray, TimestampMicrosecondArray};

    let bucket_start = TimestampMicrosecondArray::from_iter_values(rows.iter().map(|(t, _, _, _)| micros(t)))
        .with_timezone("+00:00");
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(_, x, _, _)| *x)).unwrap();
    let count = Int64Array::from_iter_values(rows.iter().map(|(_, _, c, _)| *c));
    let sum = Float64Array::from_iter_values(rows.iter().map(|(_, _, _, s)| *s));
    RecordBatch::try_new(
        Arc::new(states_schema()),
        vec![Arc::new(bucket_start), Arc::new(xunit_id), Arc::new(count), Arc::new(sum)],
    )
    .unwrap()
}

fn registry_batch(rows: &[([u8; 32], &[u8])]) -> RecordBatch {
    use datafusion::arrow::array::{BinaryArray, FixedSizeBinaryArray};

    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new("xunit_id", DataType::FixedSizeBinary(32), false),
        Field::new("xunit_canonical", DataType::Binary, false),
    ]));
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(x, _)| *x)).unwrap();
    let canonical = BinaryArray::from_iter_values(rows.iter().map(|(_, c)| *c));
    RecordBatch::try_new(schema, vec![Arc::new(xunit_id), Arc::new(canonical)]).unwrap()
}

#[tokio::test]
async fn record_batch_from_read_window_round_trips_through_a_df54_session_context() {
    let warehouse = TempDir::new().unwrap();
    let config = CatalogConfig::Memory { warehouse: warehouse.path().to_path_buf() };
    let catalog = open_catalog(&config).await.unwrap();
    let temporal_table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &states_schema()).await.unwrap();
    let publications = PublicationStore::in_memory();

    let window_id = WindowId::new("2026-08-14").unwrap();
    let revision = WindowRevision::new(1).unwrap();
    let states = vec![states_batch(&[
        ("2026-08-14T00:10:00Z", [1u8; 32], 3, 9.0),
        ("2026-08-14T00:20:00Z", [2u8; 32], 5, 11.0),
    ])];
    let registry = vec![registry_batch(&[([1u8; 32], b"US/mobile"), ([2u8; 32], b"EU/desktop")])];

    let expected_rows: u64 = states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
    let claim = publications
        .claim_run(CUBE_ID, &window_id, "run-1", revision, expected_rows)
        .await
        .unwrap();
    assert!(matches!(claim, ClaimResult::New(_)));

    let result = AggregateWriter::append_window(
        catalog.as_ref(),
        &temporal_table,
        AppendWindow { window_id: &window_id, revision, run_id: "run-1", states: &states, registry: &registry },
    )
    .await
    .unwrap();
    publications.record_append("run-1", result.snapshot_id).await.unwrap();
    // Publish before reading: `read_window` rejects an appended-but-unpublished
    // revision with `UnpublishedWindow` (`cubism-iceberg/tests/phase3.rs`
    // proves the same), which is not the seam this milestone is testing.
    publications.publish("run-1", None).await.unwrap();

    let batches = AggregateReader::read_window(catalog.as_ref(), &temporal_table, &publications, &window_id)
        .await
        .unwrap();

    // No `iceberg-datafusion` anywhere above or below this line — just a
    // `Vec<arrow_array::RecordBatch>` handed to a DF54 `SessionContext`.
    let ctx = SessionContext::new();
    let df = ctx.read_batches(batches).unwrap();
    let collected = df.collect().await.unwrap();

    let total_rows: usize = collected.iter().map(RecordBatch::num_rows).sum();
    assert_eq!(total_rows, 2, "both appended rows should round-trip through the SessionContext");

    // Beyond the row-count check above (which a schema-only round-trip
    // could satisfy without decoding any column), sum `count_v1` to prove
    // at least one column's actual data decodes correctly through DF54.
    let total_count_v1: i64 = collected
        .iter()
        .map(|batch| {
            batch
                .column_by_name("count_v1")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .iter()
                .sum::<i64>()
        })
        .sum();
    assert_eq!(total_count_v1, 3 + 5, "count_v1 values must decode correctly, not just round-trip a row count");
}
