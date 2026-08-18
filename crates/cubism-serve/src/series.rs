//! `/api/series`: minimal time-series range-query wiring
//! (`docs/TIMESERIES_ROADMAP.md`'s Milestone 11).
//!
//! Narrowed to exactly what Milestone 10b-3 answers correctly: a single
//! `XUnit` selector, `AverageState` measures only. Not a general-purpose
//! range-query API.
//!
//! **Windows are a request-supplied input, not something this module
//! derives from the query's time range.** `crates/cubism-datafusion/src/range_query.rs:79-85`
//! records that no canonical `TimeRange`/`Resolution` -> `WindowId` encoding
//! exists anywhere in this codebase, and that inventing one was already
//! rejected once as out of scope for a milestone whose job is something
//! else. This module doesn't relitigate that: the request body carries a
//! flat list of `(window_id, bucket_start)` entries, and the handler below
//! assigns each entry to whichever `ResolutionPlan` segment's time range
//! contains its `bucket_start` (a pure range-containment check — no new
//! encoding invented) before calling `CoveragePlan::new` with the result.
//! The caller (`docs/TIMESERIES_ROADMAP.md`'s Milestone 12, for its own
//! demo dataset) owns the actual bucket->`WindowId` convention.
//!
//! **A request naming a window that was never appended must not read as
//! `is_exact: true` with no data behind it.** `SegmentCoverage::is_exact()`
//! trusts the caller's `published` list and `SeriesResponse` never
//! re-verifies `batches` against it (both already-documented caveats on
//! `CoveragePlan`/`SeriesResponse` themselves). Over HTTP the caller is an
//! untrusted client — a new *exposure* for that caveat, not a new bug
//! class. This handler closes the case that would otherwise be silently
//! wrong: an unknown `window_id` resolves `PublicationStore::current` to
//! `Ok(None)`, which is classified as `missing`, not `published`, so it can
//! never reach `read_window` or be counted toward an exact segment.
//!
//! **Read-after-plan is not atomic.** `CoveragePlan` is built from one
//! `PublicationStore::current` snapshot per window; `AggregateReader::read_window`
//! (called afterward, per published window) re-resolves `current` itself.
//! A concurrent publish landing between the two would make the returned
//! rows reflect a newer revision than the one `CoveragePlan` recorded as
//! `published`. Not solved here — the single-window CLI path
//! (`cubism-cli`'s `iceberg-build`) has the same sequencing, and closing it
//! would need a locking primitive this crate does not have (tracked
//! alongside the concurrent-recovery gap in `docs/TIMESERIES_PHASE_24_HANDOFF.md`'s
//! deferred list).

use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use cubism_core::{AggKind, CubeSpec, EventTime, FixedResolution, Resolution, WindowId, WindowRevision, XUnit};
use cubism_datafusion::temporal_build::{measure_column_name, temporal_state_schema};
use cubism_datafusion::{CoveragePlan, GapPolicy, ResolutionPlan, SeriesResponse, TemporalQuery};
use cubism_iceberg::config::open_catalog;
use cubism_iceberg::{AggregateReader, CatalogConfig, PublicationStore, TemporalTable};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::ApiError;

/// Everything `/api/series` needs to answer a query: an open Iceberg
/// catalog, the cube's two temporal tables, the publication/control store,
/// and the `CubeSpec` (for its `TemporalSpec` and measure list). One
/// `SeriesState` serves one cube.
pub struct SeriesState {
    catalog: Arc<dyn iceberg::Catalog>,
    table: TemporalTable,
    publications: PublicationStore,
    spec: CubeSpec,
}

impl SeriesState {
    /// Opens (creating if absent, same as `iceberg_build`'s own idempotent
    /// `TemporalTable::create`) the tables for `spec.name` under
    /// `catalog_config`, using `publications` as the control store.
    pub async fn open(
        spec: CubeSpec,
        catalog_config: &CatalogConfig,
        publications: PublicationStore,
    ) -> Result<Self, String> {
        if spec.temporal.is_none() {
            return Err(format!("cube '{}' has no temporal spec", spec.name));
        }
        let catalog = open_catalog(catalog_config).await.map_err(|e| e.to_string())?;
        let schema = temporal_state_schema(&spec).map_err(|e| e.to_string())?;
        let table = TemporalTable::create(catalog.as_ref(), &spec.name, &schema)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self { catalog, table, publications, spec })
    }
}

/// One caller-supplied `(window_id, bucket_start)` fact: "this window backs
/// the resolution bucket starting at this instant." See the module doc
/// comment for why this is the caller's job, not this module's.
#[derive(Deserialize)]
struct SeriesWindow {
    window_id: String,
    /// Unix microseconds — matches `EventTime`'s own wire representation
    /// (`cubism_core::temporal::EventTime` is `#[serde(transparent)]` over
    /// `i64` micros; there is no RFC3339 parsing at this boundary).
    bucket_start: i64,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum GapPolicyWire {
    #[default]
    Missing,
    Zero,
}

impl From<GapPolicyWire> for GapPolicy {
    fn from(value: GapPolicyWire) -> Self {
        match value {
            GapPolicyWire::Missing => GapPolicy::Missing,
            GapPolicyWire::Zero => GapPolicy::Zero,
        }
    }
}

#[derive(Deserialize)]
struct SeriesRequest {
    /// `XUnit` string form, e.g. `"/G"` or `"/device/device_type=mobile"` —
    /// same format `/api/cell?xunit=...` already accepts.
    selector: String,
    /// Base measure name (`CubeSpec.measures[].name`); must be an `avg`
    /// measure (Milestone 10b-3's narrowed scope, `AggKind::Avg`).
    measure: String,
    /// Unix microseconds, half-open `[start, end)`.
    start: i64,
    end: i64,
    /// e.g. `"1d"`, `"1h"`; omitted means auto-select from the spec.
    resolution: Option<String>,
    #[serde(default)]
    exact: bool,
    #[serde(default)]
    gap_policy: GapPolicyWire,
    #[serde(default)]
    windows: Vec<SeriesWindow>,
}

async fn series(
    State(state): State<Arc<SeriesState>>,
    Json(req): Json<SeriesRequest>,
) -> Result<Json<Value>, ApiError> {
    let selector: XUnit =
        req.selector.parse().map_err(|e| ApiError::bad_request(format!("bad selector: {e}")))?;

    let measure = state
        .spec
        .measures
        .iter()
        .find(|m| m.name == req.measure)
        .ok_or_else(|| ApiError::bad_request(format!("unknown measure '{}'", req.measure)))?;
    if measure.agg != AggKind::Avg {
        return Err(ApiError::bad_request(format!(
            "measure '{}' is {:?}; /api/series only supports avg measures for now",
            req.measure, measure.agg
        )));
    }
    let column = measure_column_name(measure);

    // Checked in `SeriesState::open`, but a spec is mutable state this
    // handler doesn't own — re-check rather than trust construction-time.
    let temporal_spec = state
        .spec
        .temporal
        .as_ref()
        .ok_or_else(|| ApiError::bad_request(format!("cube '{}' has no temporal spec", state.spec.name)))?;

    let resolution = req
        .resolution
        .as_deref()
        .map(|s| s.parse::<FixedResolution>().map(Resolution::Fixed))
        .transpose()
        .map_err(|e| ApiError::bad_request(format!("bad resolution: {e}")))?;

    let gap_policy: GapPolicy = req.gap_policy.into();

    let query = TemporalQuery::new(
        state.spec.name.clone(),
        vec![selector.clone()],
        vec![req.measure.clone()],
        EventTime::from_unix_micros(req.start),
        EventTime::from_unix_micros(req.end),
        resolution,
        req.exact,
        gap_policy,
        temporal_spec,
    )
    .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let plan = ResolutionPlan::new(&query, temporal_spec).map_err(|e| ApiError::bad_request(e.to_string()))?;

    // Assign each request-supplied window to the one segment whose range
    // contains its bucket_start (see the module doc comment), resolving its
    // current publication state as we go.
    let mut windows_per_segment: Vec<Vec<(WindowId, Option<WindowRevision>)>> =
        Vec::with_capacity(plan.segments.len());
    for segment in &plan.segments {
        let mut entries = Vec::new();
        for w in &req.windows {
            let bucket_start = EventTime::from_unix_micros(w.bucket_start);
            if !segment.range.contains(bucket_start) {
                continue;
            }
            let window_id = WindowId::new(w.window_id.clone())
                .map_err(|e| ApiError::bad_request(format!("bad window_id '{}': {e}", w.window_id)))?;
            let current = state
                .publications
                .current(&state.table.cube_id, &window_id)
                .await
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            entries.push((window_id, current));
        }
        windows_per_segment.push(entries);
    }

    let coverage = CoveragePlan::new(&plan, req.exact, &windows_per_segment)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let mut batches = Vec::with_capacity(coverage.segments.len());
    for segment in &coverage.segments {
        let mut segment_batches = Vec::new();
        for (window_id, _revision) in &segment.published {
            let rows = AggregateReader::read_window(
                state.catalog.as_ref(),
                &state.table,
                &state.publications,
                window_id,
            )
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
            segment_batches.extend(rows);
        }
        batches.push(segment_batches);
    }

    let response = SeriesResponse::new(&coverage, gap_policy, &column, &[selector], &batches)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    Ok(Json(json!({
        "cube": state.spec.name,
        "measure": req.measure,
        "source_resolution": response.source_resolution.to_string(),
        "points": response.points.iter().map(|p| json!({
            "bucket_start": p.bucket_start.unix_micros(),
            "bucket_end": p.bucket_end.unix_micros(),
            "is_exact": p.is_exact,
            "value": p.value,
            "published": p.published.iter().map(|(w, r)| json!({
                "window_id": w.as_str(),
                "revision": r.get(),
            })).collect::<Vec<_>>(),
            "missing": p.missing.iter().map(WindowId::as_str).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })))
}

pub fn series_router(state: Arc<SeriesState>) -> Router {
    Router::new().route("/api/series", post(series)).with_state(state)
}
