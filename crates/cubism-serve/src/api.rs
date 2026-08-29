//! JSON API over a [`CubeStore`].
//!
//! Read-only, spec-free: everything is derived from the cube file itself.
//! The `/api/setops` endpoint is the point of the whole layer — set algebra
//! over stored KMV blobs, computed per request over cells that were never
//! aggregated together.

use crate::store::{CubeStore, SketchKind};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cubism_core::sketch::KmvSketch;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn not_found(message: impl Into<String>) -> Self {
        ApiError { status: StatusCode::NOT_FOUND, message: message.into() }
    }
    // `pub(crate)`: reused by `crate::series`'s handler so `/api/series`
    // reports errors in the same `{"error": "..."}` envelope as the
    // static-cube endpoints above, rather than inventing a second shape.
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        ApiError { status: StatusCode::BAD_REQUEST, message: message.into() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

type Store = State<Arc<CubeStore>>;

pub async fn meta(State(store): Store) -> Json<Value> {
    Json(json!({
        "source": store.source,
        "cells": store.xunits.len(),
        "has_global": store.row("/G").is_some(),
        "dimensions": store.dimensions,
        "measures": store.measures,
        "sketches": store.sketches.iter()
            .map(|(name, kind)| json!({"measure": name, "kind": kind}))
            .collect::<Vec<_>>(),
    }))
}

#[derive(Deserialize)]
pub struct CellsParams {
    dim: String,
    #[serde(default = "one")]
    depth: usize,
    /// Measure to sort by, descending; defaults to the first measure.
    sort: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn one() -> usize {
    1
}
fn default_limit() -> usize {
    50
}

pub async fn cells(
    State(store): Store,
    Query(p): Query<CellsParams>,
) -> Result<Json<Value>, ApiError> {
    if !store.dimensions.iter().any(|d| d == &p.dim) {
        return Err(ApiError::not_found(format!(
            "unknown dimension '{}' (have: {})",
            p.dim,
            store.dimensions.join(", ")
        )));
    }
    let sort = p.sort.unwrap_or_else(|| store.measures.first().cloned().unwrap_or_default());
    if !store.measures.contains(&sort) {
        return Err(ApiError::bad_request(format!("unknown sort measure '{sort}'")));
    }

    let mut rows = store.slice(&p.dim, p.depth);
    let sort_values = &store.measure_values[&sort];
    rows.sort_by(|&a, &b| {
        let (va, vb) = (sort_values[a].as_f64(), sort_values[b].as_f64());
        vb.partial_cmp(&va).unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(p.limit);

    let cells: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "xunit": store.xunits[row],
                "label": store.label(row),
                "measures": store.measures_at(row),
            })
        })
        .collect();
    Ok(Json(json!({"dim": p.dim, "depth": p.depth, "sort": sort, "cells": cells})))
}

#[derive(Deserialize)]
pub struct CellParams {
    xunit: String,
}

pub async fn cell(
    State(store): Store,
    Query(p): Query<CellParams>,
) -> Result<Json<Value>, ApiError> {
    let row = store
        .row(&p.xunit)
        .ok_or_else(|| ApiError::not_found(format!("no cell '{}' in this cube", p.xunit)))?;
    let sketches: Value = store
        .sketches
        .iter()
        .map(|(name, _)| (name.clone(), store.decode_sketch(name, row)))
        .collect::<serde_json::Map<_, _>>()
        .into();
    Ok(Json(json!({
        "xunit": p.xunit,
        "label": store.label(row),
        "measures": store.measures_at(row),
        "sketches": sketches,
    })))
}

#[derive(Deserialize)]
pub struct SetOpsParams {
    a: String,
    b: String,
    /// KMV-backed measure; defaults to the cube's first KMV sketch.
    measure: Option<String>,
}

pub async fn setops(
    State(store): Store,
    Query(p): Query<SetOpsParams>,
) -> Result<Json<Value>, ApiError> {
    let measure = match p.measure {
        Some(m) => m,
        None => store
            .sketches
            .iter()
            .find(|(_, kind)| *kind == SketchKind::Kmv)
            .map(|(name, _)| name.clone())
            .ok_or_else(|| ApiError::bad_request("this cube has no count_distinct measure"))?,
    };
    if store.sketch_kind(&measure) != Some(SketchKind::Kmv) {
        return Err(ApiError::bad_request(format!(
            "measure '{measure}' is not a count_distinct (KMV) sketch"
        )));
    }

    let load = |xunit: &str| -> Result<KmvSketch, ApiError> {
        let row = store
            .row(xunit)
            .ok_or_else(|| ApiError::not_found(format!("no cell '{xunit}' in this cube")))?;
        let blob = store
            .blob(&measure, row)
            .ok_or_else(|| ApiError::not_found(format!("cell '{xunit}' has no '{measure}' sketch")))?;
        KmvSketch::from_bytes(blob)
            .map_err(|e| ApiError::bad_request(format!("cannot decode sketch: {e}")))
    };
    let (a, b) = (load(&p.a)?, load(&p.b)?);

    // lift needs the global population; omit it (JSON null) without /G
    let lift = store
        .row("/G")
        .and_then(|g| store.blob(&measure, g))
        .and_then(|blob| KmvSketch::from_bytes(blob).ok())
        .map(|g| {
            let inter = a.intersection_estimate(&b);
            if inter == 0.0 { 0.0 } else { inter * g.estimate() / (a.estimate() * b.estimate()) }
        });

    Ok(Json(json!({
        "measure": measure,
        "a": {"xunit": p.a, "estimate": a.estimate()},
        "b": {"xunit": p.b, "estimate": b.estimate()},
        "union": a.union_estimate(&b),
        "intersection": a.intersection_estimate(&b),
        "jaccard": a.jaccard(&b),
        "lift": lift,
    })))
}
