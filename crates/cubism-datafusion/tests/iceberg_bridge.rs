//! Milestone 7 spike (`docs/TIMESERIES_ROADMAP.md`): proves at runtime,
//! not just via `cargo tree`'s static resolution, that a batch built with
//! `datafusion::arrow::*` types survives a full round trip — written
//! through `cubism-iceberg`'s `AggregateWriter::append_window`, committed
//! to Parquet, scanned back out by `AggregateReader::read_window`, and
//! handed straight to a DataFusion 54 `SessionContext` — with **no**
//! `iceberg-datafusion` dependency anywhere in the call path and no
//! conversion step. `cargo tree -i arrow --workspace` already showed one
//! unified `arrow` 58.3.0 feeding both the DF53 and DF54 subgraphs, so
//! type-identity failing to compile was never the real risk; what that
//! static resolution *can't* see is a runtime ABI mismatch or a
//! feature-flag divergence surfacing somewhere in the write → Parquet →
//! scan → decode path. This test exercises that whole path and it works.
//! That no conversion code exists on either side is a consequence of the
//! already-known single-`arrow` resolution, not a new finding by itself.
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
use cubism_core::XUnit;
use cubism_core::encoding::canonical_xunit_content_id;
use cubism_core::temporal::{
    AllowedLateness, BucketOrigin, EventTime, FixedResolution, Resolution, TemporalSpec, WindowId,
    WindowRevision,
};
use cubism_core::{AggKind, AggregateState, AverageState, CanonicalXUnit};
use cubism_datafusion::{
    CoveragePlan, GapPolicy, ResolutionPlan, SeriesResponse, TemporalQuery, merge_average_column,
};
use cubism_iceberg::config::open_catalog;
use cubism_iceberg::{
    AggregateReader, AggregateWriter, AppendWindow, CatalogConfig, ClaimResult, PublicationStore,
    TemporalTable,
};
use datafusion::arrow::array::{Float64Array, Int64Array, RecordBatch};
use datafusion::arrow::datatypes::{DataType, Field, Schema as ArrowSchema, TimeUnit};
use datafusion::execution::context::SessionContext;
use tempfile::TempDir;

const CUBE_ID: &str = "web_analytics";

fn micros(timestamp: &str) -> i64 {
    DateTime::parse_from_rfc3339(timestamp)
        .unwrap()
        .timestamp_micros()
}

/// The real `XUnitContentId` a selector resolves to — same two calls
/// `SeriesResponse::new` itself makes (`CanonicalXUnit::from` +
/// `canonical_xunit_content_id`), and the same two calls the build side's
/// `cubism_xunit_content_id` UDF makes over a decoded `xunit_id`
/// (`crates/cubism-datafusion/src/temporal_build.rs`'s
/// `XUnitContentIdUdf::invoke_with_args`). Used so this file's hand-built
/// states batches carry byte-identical ids to what a real build would have
/// written for the same logical cell, not arbitrary sentinel bytes.
fn content_id(xunit: &XUnit) -> [u8; 32] {
    *canonical_xunit_content_id(&CanonicalXUnit::from(xunit))
        .unwrap()
        .as_bytes()
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

    let bucket_start =
        TimestampMicrosecondArray::from_iter_values(rows.iter().map(|(t, _, _, _)| micros(t)))
            .with_timezone("+00:00");
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(_, x, _, _)| *x)).unwrap();
    let count = Int64Array::from_iter_values(rows.iter().map(|(_, _, c, _)| *c));
    let sum = Float64Array::from_iter_values(rows.iter().map(|(_, _, _, s)| *s));
    RecordBatch::try_new(
        Arc::new(states_schema()),
        vec![
            Arc::new(bucket_start),
            Arc::new(xunit_id),
            Arc::new(count),
            Arc::new(sum),
        ],
    )
    .unwrap()
}

/// A states schema with one `Binary` measure column (`avg_v1`), matching
/// `AggKind::Avg`'s shape in `temporal_build::temporal_state_schema` — used
/// only by the Milestone 10b-1 test below, kept separate from
/// `states_schema`/`states_batch` above (which model `AggKind::Sum`/`Count`
/// as plain `Int64`/`Float64`, not blobs).
fn avg_states_schema() -> ArrowSchema {
    ArrowSchema::new(vec![
        Field::new(
            "bucket_start",
            DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
            true,
        ),
        Field::new("xunit_id", DataType::FixedSizeBinary(32), true),
        Field::new("avg_v1", DataType::Binary, true),
    ])
}

fn avg_states_batch(rows: &[(&str, [u8; 32], Vec<u8>)]) -> RecordBatch {
    use datafusion::arrow::array::{BinaryArray, FixedSizeBinaryArray, TimestampMicrosecondArray};

    let bucket_start =
        TimestampMicrosecondArray::from_iter_values(rows.iter().map(|(t, _, _)| micros(t)))
            .with_timezone("+00:00");
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(_, x, _)| *x)).unwrap();
    let avg = BinaryArray::from_iter_values(rows.iter().map(|(_, _, blob)| blob.as_slice()));
    RecordBatch::try_new(
        Arc::new(avg_states_schema()),
        vec![Arc::new(bucket_start), Arc::new(xunit_id), Arc::new(avg)],
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
    let config = CatalogConfig::Memory {
        warehouse: warehouse.path().to_path_buf(),
    };
    let catalog = open_catalog(&config).await.unwrap();
    let temporal_table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &states_schema())
        .await
        .unwrap();
    let publications = PublicationStore::in_memory();

    let window_id = WindowId::new("2026-08-14").unwrap();
    let revision = WindowRevision::new(1).unwrap();
    let states = vec![states_batch(&[
        ("2026-08-14T00:10:00Z", [1u8; 32], 3, 9.0),
        ("2026-08-14T00:20:00Z", [2u8; 32], 5, 11.0),
    ])];
    let registry = vec![registry_batch(&[
        ([1u8; 32], b"US/mobile"),
        ([2u8; 32], b"EU/desktop"),
    ])];

    let expected_rows: u64 = states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
    let claim = publications
        .claim_run(CUBE_ID, &window_id, "run-1", revision, expected_rows)
        .await
        .unwrap();
    assert!(matches!(claim, ClaimResult::New(_)));

    let result = AggregateWriter::append_window(
        catalog.as_ref(),
        &temporal_table,
        AppendWindow {
            window_id: &window_id,
            revision,
            run_id: "run-1",
            states: &states,
            registry: &registry,
        },
    )
    .await
    .unwrap();
    publications
        .record_append("run-1", result.snapshot_id)
        .await
        .unwrap();
    // Publish before reading: `read_window` rejects an appended-but-unpublished
    // revision with `UnpublishedWindow` (`cubism-iceberg/tests/phase3.rs`
    // proves the same), which is not the seam this milestone is testing.
    publications.publish("run-1", None).await.unwrap();

    let batches =
        AggregateReader::read_window(catalog.as_ref(), &temporal_table, &publications, &window_id)
            .await
            .unwrap();

    // No `iceberg-datafusion` anywhere above or below this line — just a
    // `Vec<arrow_array::RecordBatch>` handed to a DF54 `SessionContext`.
    let ctx = SessionContext::new();
    let df = ctx.read_batches(batches).unwrap();
    let collected = df.collect().await.unwrap();

    let total_rows: usize = collected.iter().map(RecordBatch::num_rows).sum();
    assert_eq!(
        total_rows, 2,
        "both appended rows should round-trip through the SessionContext"
    );

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
    assert_eq!(
        total_count_v1,
        3 + 5,
        "count_v1 values must decode correctly, not just round-trip a row count"
    );
}

/// Milestone 10 (`docs/TIMESERIES_ROADMAP.md`): the real wiring the module
/// doc comment on `cubism_datafusion::range_query::CoveragePlan` describes —
/// `PublicationStore::current` resolved per window in this test (not inside
/// `CoveragePlan`, which stays pure/sync; see that doc comment for why) and
/// fed into `CoveragePlan::new` as the caller-supplied segment→window
/// mapping.
///
/// Proves: a `ResolutionPlan` spanning two day-resolution windows, one
/// published and one not, produces a `CoveragePlan` whose single (aligned,
/// two-window) interior segment reports the published window's real
/// `WindowRevision` as provenance and the unpublished window as `missing`;
/// and that `exact: true` fails clearly instead of silently omitting the
/// unpublished window. Does not prove anything about `AggregateState`
/// value decoding/merging — no value is read back for either window here,
/// per the module doc comment's deferred-to-a-later-slice note.
#[tokio::test]
async fn coverage_plan_resolves_real_publication_state_across_two_windows() {
    let warehouse = TempDir::new().unwrap();
    let config = CatalogConfig::Memory {
        warehouse: warehouse.path().to_path_buf(),
    };
    let catalog = open_catalog(&config).await.unwrap();
    let temporal_table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &states_schema())
        .await
        .unwrap();
    let publications = PublicationStore::in_memory();

    let day = FixedResolution::from_micros(86_400_000_000).unwrap();
    let spec = TemporalSpec {
        event_time: "ts".into(),
        ingestion_time: None,
        base_resolution: Resolution::Fixed(day),
        origin: BucketOrigin::default(),
        timezone: "UTC".into(),
        allowed_lateness: AllowedLateness::from_micros(0).unwrap(),
        rollups: vec![],
        retention: None,
    };

    // [2026-08-13T00:00:00Z, 2026-08-15T00:00:00Z) is exactly two day
    // buckets (epoch-aligned origin), so ResolutionPlan::decompose keeps it
    // as one aligned interior segment spanning both, per Milestone 9.
    let query = TemporalQuery::new(
        CUBE_ID,
        vec![XUnit::global()],
        vec!["count".into()],
        EventTime::from_unix_micros(micros("2026-08-13T00:00:00Z")),
        EventTime::from_unix_micros(micros("2026-08-15T00:00:00Z")),
        Some(Resolution::Fixed(day)),
        false,
        GapPolicy::Missing,
        &spec,
    )
    .unwrap();
    let plan = ResolutionPlan::new(&query, &spec).unwrap();
    assert_eq!(
        plan.segments.len(),
        1,
        "two contiguous day buckets must stay one aligned segment"
    );
    assert!(plan.segments[0].aligned);

    let w1 = WindowId::new("2026-08-13").unwrap();
    let w2 = WindowId::new("2026-08-14").unwrap();
    let revision = WindowRevision::new(1).unwrap();

    // Publish w1 only, mirroring the round-trip test's append/publish
    // sequence above.
    let states = vec![states_batch(&[("2026-08-13T12:00:00Z", [1u8; 32], 3, 9.0)])];
    let registry = vec![registry_batch(&[([1u8; 32], b"US/mobile")])];
    let expected_rows: u64 = states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
    let claim = publications
        .claim_run(CUBE_ID, &w1, "run-1", revision, expected_rows)
        .await
        .unwrap();
    assert!(matches!(claim, ClaimResult::New(_)));
    let result = AggregateWriter::append_window(
        catalog.as_ref(),
        &temporal_table,
        AppendWindow {
            window_id: &w1,
            revision,
            run_id: "run-1",
            states: &states,
            registry: &registry,
        },
    )
    .await
    .unwrap();
    publications
        .record_append("run-1", result.snapshot_id)
        .await
        .unwrap();
    publications.publish("run-1", None).await.unwrap();

    // w2 is never claimed, appended, or published: `current` must resolve
    // it to `Ok(None)` (unpublished), not an error (control.rs:315-318).
    let w1_current = publications.current(CUBE_ID, &w1).await.unwrap();
    let w2_current = publications.current(CUBE_ID, &w2).await.unwrap();
    assert_eq!(w1_current, Some(revision));
    assert_eq!(w2_current, None);

    let windows = vec![vec![(w1.clone(), w1_current), (w2.clone(), w2_current)]];

    let coverage = CoveragePlan::new(&plan, false, &windows)
        .expect("exact: false must not fail on a partially-missing segment");
    assert_eq!(coverage.segments.len(), 1);
    assert!(!coverage.segments[0].is_exact());
    assert_eq!(coverage.segments[0].published, vec![(w1.clone(), revision)]);
    assert_eq!(coverage.segments[0].missing, vec![w2.clone()]);

    let err = CoveragePlan::new(&plan, true, &windows)
        .expect_err("exact: true must fail clearly instead of silently omitting w2");
    assert!(err.to_string().contains("exact=true"));
}

/// Milestone 10b-1 (`docs/TIMESERIES_ROADMAP.md`): the value-materialization
/// primitive `cubism_datafusion::merge_average_column` provides, wired
/// against two real published windows' states rows — the "merging" half of
/// the roadmap's deferred "Milestone 10b" note that Milestone 10 itself does
/// not do (see `range_query.rs`'s module doc comment).
///
/// Proves: reading two published windows' states rows back via
/// `AggregateReader::read_window` and feeding both windows' batches into
/// `merge_average_column` produces the same `AverageState` as merging the
/// two source states directly (`a.merge(&b)`) — the decode+merge round-trips
/// through a real Parquet write/scan, not just in memory (`series_merge.rs`'s
/// own unit tests already cover the in-memory decode+merge logic itself).
/// Does not prove anything about `VarianceState`/`QuantileState`/the
/// sketch-backed kinds (this function only handles `AverageState`), a
/// `SeriesResponse` type, or `CoveragePlan` wiring: this test builds its two
/// windows directly rather than through a `ResolutionPlan`/`CoveragePlan`,
/// since Milestone 10b-1's scope is the merge primitive alone, not query
/// planning (that's the test above, Milestone 10's own).
#[tokio::test]
async fn merge_average_column_reads_and_merges_two_published_windows() {
    let warehouse = TempDir::new().unwrap();
    let config = CatalogConfig::Memory {
        warehouse: warehouse.path().to_path_buf(),
    };
    let catalog = open_catalog(&config).await.unwrap();
    let temporal_table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &avg_states_schema())
        .await
        .unwrap();
    let publications = PublicationStore::in_memory();

    let mut a = AverageState::new();
    a.accumulate(3.0).unwrap();
    a.accumulate(5.0).unwrap();
    let mut b = AverageState::new();
    b.accumulate(10.0).unwrap();

    let w1 = WindowId::new("2026-08-13").unwrap();
    let w2 = WindowId::new("2026-08-14").unwrap();
    let revision = WindowRevision::new(1).unwrap();

    let states1 = vec![avg_states_batch(&[(
        "2026-08-13T12:00:00Z",
        [1u8; 32],
        a.encode(),
    )])];
    let registry1 = vec![registry_batch(&[([1u8; 32], b"US/mobile")])];
    let expected_rows1: u64 = states1.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
    let claim1 = publications
        .claim_run(CUBE_ID, &w1, "run-a", revision, expected_rows1)
        .await
        .unwrap();
    assert!(matches!(claim1, ClaimResult::New(_)));
    let result1 = AggregateWriter::append_window(
        catalog.as_ref(),
        &temporal_table,
        AppendWindow {
            window_id: &w1,
            revision,
            run_id: "run-a",
            states: &states1,
            registry: &registry1,
        },
    )
    .await
    .unwrap();
    publications
        .record_append("run-a", result1.snapshot_id)
        .await
        .unwrap();
    publications.publish("run-a", None).await.unwrap();

    let states2 = vec![avg_states_batch(&[(
        "2026-08-14T12:00:00Z",
        [2u8; 32],
        b.encode(),
    )])];
    let registry2 = vec![registry_batch(&[([2u8; 32], b"EU/desktop")])];
    let expected_rows2: u64 = states2.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
    let claim2 = publications
        .claim_run(CUBE_ID, &w2, "run-b", revision, expected_rows2)
        .await
        .unwrap();
    assert!(matches!(claim2, ClaimResult::New(_)));
    let result2 = AggregateWriter::append_window(
        catalog.as_ref(),
        &temporal_table,
        AppendWindow {
            window_id: &w2,
            revision,
            run_id: "run-b",
            states: &states2,
            registry: &registry2,
        },
    )
    .await
    .unwrap();
    publications
        .record_append("run-b", result2.snapshot_id)
        .await
        .unwrap();
    publications.publish("run-b", None).await.unwrap();

    let mut batches =
        AggregateReader::read_window(catalog.as_ref(), &temporal_table, &publications, &w1)
            .await
            .unwrap();
    batches.extend(
        AggregateReader::read_window(catalog.as_ref(), &temporal_table, &publications, &w2)
            .await
            .unwrap(),
    );

    let merged = merge_average_column(&batches, "avg_v1").unwrap();
    assert_eq!(merged, a.merge(&b).unwrap());
}

/// Milestone 10b-2 (`docs/TIMESERIES_ROADMAP.md`): `SeriesResponse` wired to
/// a real `CoveragePlan` — the "wiring" half of the roadmap's deferred
/// "Milestone 10b" note that neither Milestone 10 (`CoveragePlan` alone,
/// provenance only) nor Milestone 10b-1 (`merge_average_column` alone, no
/// query planning) closes.
///
/// Same two-window setup as
/// `coverage_plan_resolves_real_publication_state_across_two_windows`
/// (Milestone 10's own test) — one aligned interior segment spanning two
/// day-resolution windows — except both windows are published here (not one
/// published/one missing), each carrying a real `AverageState`-encoded
/// `avg_v1` row (same encoding `merge_average_column_reads_and_merges_two_published_windows`
/// uses). Proves: `SeriesResponse::new`, fed the real `CoveragePlan` plus
/// both windows' batches read back via `AggregateReader::read_window`,
/// produces one point whose `value` equals `a.merge(&b).unwrap().present()`
/// (the same merge Milestone 10b-1's test verifies, now reached via the
/// full `ResolutionPlan` -> `CoveragePlan` -> `SeriesResponse` path instead
/// of a direct call), `is_exact: true`, both windows in `published` with
/// their real `WindowRevision`s, and `missing` empty. Does not prove
/// anything about a partially-published segment's `value` (that shape is
/// `series_response.rs`'s own
/// `series_response_propagates_missing_and_published_from_partial_segment`
/// unit test, which does not need real Iceberg I/O to prove it) or about
/// `gap_policy` (also covered purely in `series_response.rs`'s unit tests).
#[tokio::test]
async fn series_response_materializes_two_published_windows_through_a_real_coverage_plan() {
    let warehouse = TempDir::new().unwrap();
    let config = CatalogConfig::Memory {
        warehouse: warehouse.path().to_path_buf(),
    };
    let catalog = open_catalog(&config).await.unwrap();
    let temporal_table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &avg_states_schema())
        .await
        .unwrap();
    let publications = PublicationStore::in_memory();

    let day = FixedResolution::from_micros(86_400_000_000).unwrap();
    let spec = TemporalSpec {
        event_time: "ts".into(),
        ingestion_time: None,
        base_resolution: Resolution::Fixed(day),
        origin: BucketOrigin::default(),
        timezone: "UTC".into(),
        allowed_lateness: AllowedLateness::from_micros(0).unwrap(),
        rollups: vec![],
        retention: None,
    };

    // [2026-08-13T00:00:00Z, 2026-08-15T00:00:00Z) is exactly two day
    // buckets (epoch-aligned origin), so ResolutionPlan::decompose keeps it
    // as one aligned interior segment spanning both, per Milestone 9.
    let query = TemporalQuery::new(
        CUBE_ID,
        vec![XUnit::global()],
        vec!["avg_v1".into()],
        EventTime::from_unix_micros(micros("2026-08-13T00:00:00Z")),
        EventTime::from_unix_micros(micros("2026-08-15T00:00:00Z")),
        Some(Resolution::Fixed(day)),
        false,
        GapPolicy::Missing,
        &spec,
    )
    .unwrap();
    let plan = ResolutionPlan::new(&query, &spec).unwrap();
    assert_eq!(
        plan.segments.len(),
        1,
        "two contiguous day buckets must stay one aligned segment"
    );
    assert!(plan.segments[0].aligned);

    let mut a = AverageState::new();
    a.accumulate(3.0).unwrap();
    a.accumulate(5.0).unwrap();
    let mut b = AverageState::new();
    b.accumulate(10.0).unwrap();

    let w1 = WindowId::new("2026-08-13").unwrap();
    let w2 = WindowId::new("2026-08-14").unwrap();
    let revision = WindowRevision::new(1).unwrap();

    // Both windows' one row is tagged with the real global-cell content id
    // (not an arbitrary sentinel) so the query's `XUnit::global()` selector
    // — resolved to the same id inside `SeriesResponse::new` — actually
    // matches these rows; see `content_id`'s doc comment. Milestone 10b-1's
    // own test above (`merge_average_column_reads_and_merges_two_published_windows`)
    // calls `merge_average_column` directly, bypassing `SeriesResponse`'s
    // filter entirely, so its sentinel `[1u8; 32]`/`[2u8; 32]` ids are
    // unaffected and deliberately left as-is.
    let global_id = content_id(&XUnit::global());
    let states1 = vec![avg_states_batch(&[(
        "2026-08-13T12:00:00Z",
        global_id,
        a.encode(),
    )])];
    let registry1 = vec![registry_batch(&[(global_id, b"/G")])];
    let expected_rows1: u64 = states1.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
    let claim1 = publications
        .claim_run(CUBE_ID, &w1, "run-a", revision, expected_rows1)
        .await
        .unwrap();
    assert!(matches!(claim1, ClaimResult::New(_)));
    let result1 = AggregateWriter::append_window(
        catalog.as_ref(),
        &temporal_table,
        AppendWindow {
            window_id: &w1,
            revision,
            run_id: "run-a",
            states: &states1,
            registry: &registry1,
        },
    )
    .await
    .unwrap();
    publications
        .record_append("run-a", result1.snapshot_id)
        .await
        .unwrap();
    publications.publish("run-a", None).await.unwrap();

    let states2 = vec![avg_states_batch(&[(
        "2026-08-14T12:00:00Z",
        global_id,
        b.encode(),
    )])];
    let registry2 = vec![registry_batch(&[(global_id, b"/G")])];
    let expected_rows2: u64 = states2.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
    let claim2 = publications
        .claim_run(CUBE_ID, &w2, "run-b", revision, expected_rows2)
        .await
        .unwrap();
    assert!(matches!(claim2, ClaimResult::New(_)));
    let result2 = AggregateWriter::append_window(
        catalog.as_ref(),
        &temporal_table,
        AppendWindow {
            window_id: &w2,
            revision,
            run_id: "run-b",
            states: &states2,
            registry: &registry2,
        },
    )
    .await
    .unwrap();
    publications
        .record_append("run-b", result2.snapshot_id)
        .await
        .unwrap();
    publications.publish("run-b", None).await.unwrap();

    let w1_current = publications.current(CUBE_ID, &w1).await.unwrap();
    let w2_current = publications.current(CUBE_ID, &w2).await.unwrap();
    assert_eq!(w1_current, Some(revision));
    assert_eq!(w2_current, Some(revision));
    let windows = vec![vec![(w1.clone(), w1_current), (w2.clone(), w2_current)]];

    let coverage = CoveragePlan::new(&plan, false, &windows)
        .expect("exact: false must not fail when both windows are published");
    assert_eq!(coverage.segments.len(), 1);
    assert!(coverage.segments[0].is_exact());

    let w1_batches =
        AggregateReader::read_window(catalog.as_ref(), &temporal_table, &publications, &w1)
            .await
            .unwrap();
    let w2_batches =
        AggregateReader::read_window(catalog.as_ref(), &temporal_table, &publications, &w2)
            .await
            .unwrap();
    let mut segment_batches = w1_batches;
    segment_batches.extend(w2_batches);
    let batches = vec![segment_batches];

    let response = SeriesResponse::new(
        &coverage,
        GapPolicy::Missing,
        AggKind::Avg,
        "avg_v1",
        &[XUnit::global()],
        &batches,
    )
    .expect("both windows are published and avg_v1 decodes cleanly");
    assert_eq!(response.source_resolution, Resolution::Fixed(day));
    assert_eq!(response.points.len(), 1);
    let point = &response.points[0];
    assert!(point.is_exact);
    assert_eq!(
        point.value,
        a.merge(&b).unwrap().present(),
        "value must be the mean of the real merged AverageState read back from both windows"
    );
    assert_eq!(
        point.published,
        vec![(w1.clone(), revision), (w2.clone(), revision)]
    );
    assert!(point.missing.is_empty());
}
