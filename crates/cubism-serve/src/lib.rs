//! Serving layer over a Cubism cube file: a read-only JSON API plus an
//! embedded single-file dashboard.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let store = cubism_serve::CubeStore::from_path("cube.parquet")?;
//! cubism_serve::serve(store, 8080).await?;
//! # Ok(()) }
//! ```
//!
//! Endpoints (all GET, JSON):
//! - `/api/meta` — dimensions, measures, sketch kinds
//! - `/api/cells?dim=tool&depth=1&sort=cost&limit=25` — a sorted slice
//! - `/api/cell?xunit=/tool/tool=Bash` — one cell, sketches decoded
//! - `/api/setops?a=<xunit>&b=<xunit>[&measure=..]` — query-time set algebra
//!   (union/intersection/Jaccard/lift) over two cells' KMV sketches
//! - `/` — the dashboard (self-contained HTML, no external assets)

mod api;
pub mod store;

pub use store::{CubeStore, StoreError};

use axum::response::Html;
use axum::routing::get;
use axum::Router;
use std::sync::Arc;

const DASHBOARD: &str = include_str!("../assets/index.html");

pub fn router(store: Arc<CubeStore>) -> Router {
    Router::new()
        .route("/", get(|| async { Html(DASHBOARD) }))
        .route("/api/meta", get(api::meta))
        .route("/api/cells", get(api::cells))
        .route("/api/cell", get(api::cell))
        .route("/api/setops", get(api::setops))
        .with_state(store)
}

/// Bind on localhost and serve until interrupted.
pub async fn serve(store: CubeStore, port: u16) -> std::io::Result<()> {
    let cells = store.xunits.len();
    let app = router(Arc::new(store));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("serving {cells} cells at http://127.0.0.1:{port}");
    axum::serve(listener, app).await
}
