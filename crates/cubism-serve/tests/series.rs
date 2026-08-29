//! End-to-end for `/api/series` (`docs/TIMESERIES_ROADMAP.md`'s Milestone
//! 11): build two real published windows through the durable Sqlite
//! catalog and control store, mirroring `cubism-iceberg/tests/durability.rs`'s
//! two-handle pattern. Fixture data goes in through one handle;
//! `SeriesState::open` opens a second, independent one, the same way a real
//! `cubism serve --spec .. --warehouse ..` process would after `iceberg-build`
//! populated the tables. Then query them over a live HTTP listener via
//! `cubism_serve::series_router`.
//!
//! Proves: an aligned two-window range resolves to one exact point with the
//! real merged `AverageState` value and both windows in `published`; and a
//! request naming a `window_id` that was never appended comes back as
//! `missing`/non-exact with a 200, not a 500 or a falsely-exact point (the
//! untrusted-client exposure `crate::series`'s module doc comment calls
//! out). Milestone 14 adds the scalar-measure shape of the first claim: a
//! `count` cube answers by summing its Int64 column across both windows'
//! rows. Does not prove anything about `resolution: None` (auto-select) or
//! `gap_policy: "zero"` — those are `range_query.rs`/`series_response.rs`'s
//! own unit tests' job, not this integration test's.

use chrono::DateTime;
use cubism_core::encoding::canonical_xunit_content_id;
use cubism_core::temporal::{AllowedLateness, BucketOrigin, FixedResolution, Resolution, TemporalSpec, WindowId, WindowRevision};
use cubism_core::{AggKind, AggregateState, AggregateStateConfig, AverageState, CanonicalXUnit, CubeSpec, MeasureSpec, XUnit};
use cubism_iceberg::config::open_catalog;
use cubism_iceberg::{AggregateWriter, AppendWindow, CatalogConfig, ClaimResult, PublicationStore, TemporalTable};
use cubism_datafusion::datafusion::arrow::array::{
    BinaryArray, FixedSizeBinaryArray, Int64Array, RecordBatch, TimestampMicrosecondArray,
};
use cubism_datafusion::datafusion::arrow::datatypes::{DataType, Field, Schema as ArrowSchema, TimeUnit};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;

const CUBE_ID: &str = "series_e2e";

fn micros(timestamp: &str) -> i64 {
    DateTime::parse_from_rfc3339(timestamp).unwrap().timestamp_micros()
}

/// The real global-rollup `XUnitContentId`, computed the same two calls
/// `SeriesResponse::new` itself makes (`CanonicalXUnit::from` +
/// `canonical_xunit_content_id`) — same rationale as `iceberg_bridge.rs`'s
/// own `content_id` helper: a hand-built fixture must carry a
/// byte-identical id to what a real build would write, not arbitrary
/// sentinel bytes, or `SeriesResponse`'s selector filter drops every row.
fn global_content_id() -> [u8; 32] {
    *canonical_xunit_content_id(&CanonicalXUnit::from(&XUnit::global())).unwrap().as_bytes()
}

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
    let bucket_start = TimestampMicrosecondArray::from_iter_values(rows.iter().map(|(t, _, _)| micros(t)))
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
    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new("xunit_id", DataType::FixedSizeBinary(32), false),
        Field::new("xunit_canonical", DataType::Binary, false),
    ]));
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(x, _)| *x)).unwrap();
    let canonical = BinaryArray::from_iter_values(rows.iter().map(|(_, c)| *c));
    RecordBatch::try_new(schema, vec![Arc::new(xunit_id), Arc::new(canonical)]).unwrap()
}

fn spec(temporal: TemporalSpec) -> CubeSpec {
    CubeSpec {
        api_version: "cubism/v2alpha1".into(),
        name: CUBE_ID.into(),
        dimensions: vec![],
        filter_rules: vec![],
        measures: vec![MeasureSpec {
            name: "avg".into(),
            agg: AggKind::Avg,
            input: Some("value".into()),
            by: None,
            state: AggregateStateConfig::default(),
        }],
        include_global: true,
        temporal: Some(temporal),
        max_dictionary_entries: 1_000_000,
    }
}

fn day_spec() -> TemporalSpec {
    TemporalSpec {
        event_time: "ts".into(),
        ingestion_time: None,
        base_resolution: Resolution::Fixed(FixedResolution::from_micros(86_400_000_000).unwrap()),
        origin: BucketOrigin::default(),
        timezone: "UTC".into(),
        allowed_lateness: AllowedLateness::from_micros(0).unwrap(),
        rollups: vec![],
        retention: None,
    }
}

/// The count-measure variant of [`spec`]: same cube shape, but the only
/// measure is `count` — which carries no input field
/// (`AggKind::Count::requires_input()` is false) and is stored as a plain
/// Int64 states column (`views_v1`), not a blob.
fn count_spec(temporal: TemporalSpec) -> CubeSpec {
    let mut spec = spec(temporal);
    spec.measures = vec![MeasureSpec {
        name: "views".into(),
        agg: AggKind::Count,
        input: None,
        by: None,
        state: AggregateStateConfig::default(),
    }];
    spec
}

/// `temporal_state_schema`'s declared shape for a count measure:
/// `bucket_start`, `xunit_id`, then the measure column as non-null Int64.
fn count_states_schema() -> ArrowSchema {
    ArrowSchema::new(vec![
        Field::new(
            "bucket_start",
            DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
            true,
        ),
        Field::new("xunit_id", DataType::FixedSizeBinary(32), true),
        Field::new("views_v1", DataType::Int64, false),
    ])
}

fn count_states_batch(rows: &[(&str, [u8; 32], i64)]) -> RecordBatch {
    let bucket_start = TimestampMicrosecondArray::from_iter_values(rows.iter().map(|(t, _, _)| micros(t)))
        .with_timezone("+00:00");
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(_, x, _)| *x)).unwrap();
    let views = Int64Array::from_iter_values(rows.iter().map(|(_, _, v)| *v));
    RecordBatch::try_new(
        Arc::new(count_states_schema()),
        vec![Arc::new(bucket_start), Arc::new(xunit_id), Arc::new(views)],
    )
    .unwrap()
}

/// stdlib-free-of-clients raw HTTP POST, same rationale as `tests/api.rs`'s
/// `get` helper (kept dev-deps lean) — generalized to a body and method.
async fn post(base: &str, path: &str, body: &Value) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let payload = body.to_string();
    let mut stream = tokio::net::TcpStream::connect(base).await.unwrap();
    stream
        .write_all(
            format!(
                "POST {path} HTTP/1.1\r\nHost: {base}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
                payload.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8(buf).unwrap();
    let status: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let resp_body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    (status, serde_json::from_str(resp_body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn series_endpoint_answers_an_aligned_two_window_range_and_reports_an_unknown_window_as_missing() {
    let warehouse = TempDir::new().unwrap();
    let catalog_dir = TempDir::new().unwrap();
    let catalog_db = catalog_dir.path().join("catalog.sqlite");
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let config = CatalogConfig::Sqlite { warehouse: warehouse.path().to_path_buf(), catalog_db: catalog_db.clone() };

    let cube_spec = spec(day_spec());

    let mut a = AverageState::new();
    a.accumulate(3.0).unwrap();
    a.accumulate(5.0).unwrap();
    let mut b = AverageState::new();
    b.accumulate(10.0).unwrap();
    let expected_merged = a.merge(&b).unwrap().present().unwrap();

    let w1 = WindowId::new("2026-08-13").unwrap();
    let w2 = WindowId::new("2026-08-14").unwrap();
    let revision = WindowRevision::new(1).unwrap();

    // Fixture handle: independent catalog Arc + PublicationStore from the
    // one `SeriesState::open` below will use, same as
    // `durability.rs`'s two-handle tests — proves the server reads real
    // durable state, not something wired to the same in-process objects.
    {
        let catalog = open_catalog(&config).await.unwrap();
        let table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &avg_states_schema()).await.unwrap();
        let publications = PublicationStore::sqlite(&control_db).await.unwrap();

        let global_id = global_content_id();
        for (window, bucket_ts, state) in
            [(&w1, "2026-08-13T12:00:00Z", &a), (&w2, "2026-08-14T12:00:00Z", &b)]
        {
            let states = vec![avg_states_batch(&[(bucket_ts, global_id, state.encode())])];
            let registry = vec![registry_batch(&[(global_id, b"/G")])];
            let expected_rows: u64 = states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
            let claim = publications
                .claim_run(CUBE_ID, window, &format!("run-{}", window.as_str()), revision, expected_rows)
                .await
                .unwrap();
            assert!(matches!(claim, ClaimResult::New(_)));
            let result = AggregateWriter::append_window(
                catalog.as_ref(),
                &table,
                AppendWindow {
                    window_id: window,
                    revision,
                    run_id: &format!("run-{}", window.as_str()),
                    states: &states,
                    registry: &registry,
                },
            )
            .await
            .unwrap();
            publications.record_append(&format!("run-{}", window.as_str()), result.snapshot_id).await.unwrap();
            publications.publish(&format!("run-{}", window.as_str()), None).await.unwrap();
        }
    }

    let series_publications = PublicationStore::sqlite(&control_db).await.unwrap();
    let state = cubism_serve::SeriesState::open(cube_spec, &config, series_publications).await.unwrap();
    let app = cubism_serve::series_router(Arc::new(state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // [2026-08-13T00:00:00Z, 2026-08-15T00:00:00Z) at 1d resolution is
    // exactly two aligned buckets -> one interior segment, both windows
    // backing it (Milestone 9's own decomposition rule).
    let request = json!({
        "selector": "/G",
        "measure": "avg",
        "start": micros("2026-08-13T00:00:00Z"),
        "end": micros("2026-08-15T00:00:00Z"),
        "resolution": "1d",
        "exact": false,
        "gap_policy": "missing",
        "windows": [
            {"window_id": "2026-08-13", "bucket_start": micros("2026-08-13T00:00:00Z")},
            {"window_id": "2026-08-14", "bucket_start": micros("2026-08-14T00:00:00Z")},
        ],
    });
    let (status, body) = post(&base, "/api/series", &request).await;
    assert_eq!(status, 200, "body: {body}");
    let points = body["points"].as_array().unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0]["is_exact"], true);
    assert_eq!(points[0]["value"], expected_merged);
    assert_eq!(points[0]["published"].as_array().unwrap().len(), 2);
    assert_eq!(points[0]["missing"].as_array().unwrap().len(), 0);

    // A window_id the request names but that was never claimed/appended/
    // published: `PublicationStore::current` resolves it to `Ok(None)`
    // (control.rs:315-318), which the handler must classify as `missing`,
    // not surface as a 500 or a falsely-exact point.
    let unknown_request = json!({
        "selector": "/G",
        "measure": "avg",
        "start": micros("2026-08-20T00:00:00Z"),
        "end": micros("2026-08-21T00:00:00Z"),
        "resolution": "1d",
        "exact": false,
        "gap_policy": "missing",
        "windows": [
            {"window_id": "2026-08-20", "bucket_start": micros("2026-08-20T00:00:00Z")},
        ],
    });
    let (status, body) = post(&base, "/api/series", &unknown_request).await;
    assert_eq!(status, 200, "an unknown window_id must not 500: {body}");
    let points = body["points"].as_array().unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0]["is_exact"], false);
    assert_eq!(points[0]["value"], Value::Null);
    assert_eq!(points[0]["published"].as_array().unwrap().len(), 0);
    assert_eq!(points[0]["missing"], json!(["2026-08-20"]));

    // Same unknown window, but `exact: true` — must fail clearly (400 via
    // `CoveragePlan::new`'s own `exact=true` rejection), not silently.
    let mut exact_request = unknown_request.clone();
    exact_request["exact"] = json!(true);
    let (status, body) = post(&base, "/api/series", &exact_request).await;
    assert_eq!(status, 400, "body: {body}");
    assert!(body["error"].as_str().unwrap().contains("exact=true"));
}

#[tokio::test]
async fn series_endpoint_answers_a_count_measure_by_summing_the_published_windows_rows() {
    // Milestone 14's widening end to end: the same durable two-handle
    // fixture shape as the avg test above, but the cube's only measure is
    // `count` (no input field) stored as a plain Int64 states column, and
    // the request names it. Proves: the widened gate admits a scalar-kind
    // measure; the Int64 column round-trips Iceberg -> `read_window` ->
    // `SeriesResponse`'s scalar fold; and two published windows' rows sum
    // into the one point's value. Does not prove anything about gap-policy
    // substitution or selector filtering for scalars — those are
    // `series_response.rs`'s own unit tests' job.
    let warehouse = TempDir::new().unwrap();
    let catalog_dir = TempDir::new().unwrap();
    let catalog_db = catalog_dir.path().join("catalog.sqlite");
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let config = CatalogConfig::Sqlite { warehouse: warehouse.path().to_path_buf(), catalog_db: catalog_db.clone() };

    let w1 = WindowId::new("2026-08-13").unwrap();
    let w2 = WindowId::new("2026-08-14").unwrap();
    let revision = WindowRevision::new(1).unwrap();

    {
        let catalog = open_catalog(&config).await.unwrap();
        let table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &count_states_schema()).await.unwrap();
        let publications = PublicationStore::sqlite(&control_db).await.unwrap();

        let global_id = global_content_id();
        for (window, bucket_ts, views) in [(&w1, "2026-08-13T12:00:00Z", 3i64), (&w2, "2026-08-14T12:00:00Z", 4)]
        {
            let states = vec![count_states_batch(&[(bucket_ts, global_id, views)])];
            let registry = vec![registry_batch(&[(global_id, b"/G")])];
            let expected_rows: u64 = states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
            let claim = publications
                .claim_run(CUBE_ID, window, &format!("run-{}", window.as_str()), revision, expected_rows)
                .await
                .unwrap();
            assert!(matches!(claim, ClaimResult::New(_)));
            let result = AggregateWriter::append_window(
                catalog.as_ref(),
                &table,
                AppendWindow {
                    window_id: window,
                    revision,
                    run_id: &format!("run-{}", window.as_str()),
                    states: &states,
                    registry: &registry,
                },
            )
            .await
            .unwrap();
            publications.record_append(&format!("run-{}", window.as_str()), result.snapshot_id).await.unwrap();
            publications.publish(&format!("run-{}", window.as_str()), None).await.unwrap();
        }
    }

    let series_publications = PublicationStore::sqlite(&control_db).await.unwrap();
    let state = cubism_serve::SeriesState::open(count_spec(day_spec()), &config, series_publications).await.unwrap();
    let app = cubism_serve::series_router(Arc::new(state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let request = json!({
        "selector": "/G",
        "measure": "views",
        "start": micros("2026-08-13T00:00:00Z"),
        "end": micros("2026-08-15T00:00:00Z"),
        "resolution": "1d",
        "exact": false,
        "gap_policy": "missing",
        "windows": [
            {"window_id": "2026-08-13", "bucket_start": micros("2026-08-13T00:00:00Z")},
            {"window_id": "2026-08-14", "bucket_start": micros("2026-08-14T00:00:00Z")},
        ],
    });
    let (status, body) = post(&base, "/api/series", &request).await;
    assert_eq!(status, 200, "body: {body}");
    let points = body["points"].as_array().unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0]["is_exact"], true);
    assert_eq!(points[0]["value"], json!(7.0), "3 + 4 across both windows' rows");
    assert_eq!(points[0]["published"].as_array().unwrap().len(), 2);
    assert_eq!(points[0]["missing"].as_array().unwrap().len(), 0);

    // A count measure the spec doesn't have must still be rejected by name.
    let unknown_measure = json!({
        "selector": "/G",
        "measure": "nope",
        "start": micros("2026-08-13T00:00:00Z"),
        "end": micros("2026-08-15T00:00:00Z"),
        "windows": [],
    });
    let (status, body) = post(&base, "/api/series", &unknown_measure).await;
    assert_eq!(status, 400, "body: {body}");
}
