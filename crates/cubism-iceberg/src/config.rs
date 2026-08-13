use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iceberg::io::LocalFsStorageFactory;
use iceberg::memory::{MEMORY_CATALOG_WAREHOUSE, MemoryCatalogBuilder};
use iceberg::{Catalog, CatalogBuilder};
use iceberg_catalog_sql::{
    SQL_CATALOG_PROP_BIND_STYLE, SQL_CATALOG_PROP_URI, SQL_CATALOG_PROP_WAREHOUSE, SqlBindStyle,
    SqlCatalogBuilder,
};

use crate::error::{CubismIcebergError, Result};

/// How to reach the Iceberg catalog and warehouse.
///
/// A real object store (S3/GCS) is still Phase 0A gate #2
/// (`docs/PHASE_0A_RESULTS.md`) — an infrastructure/vendor decision this
/// crate does not make on its own; both variants here use
/// `LocalFsStorageFactory`. The enum is left open so a future variant can
/// be added without changing callers that already match on it exhaustively
/// via `open_catalog`.
#[derive(Debug, Clone)]
pub enum CatalogConfig {
    /// In-memory Iceberg catalog backed by a local filesystem warehouse.
    /// **Not durable across process restarts** — the namespace/table
    /// registry lives only in this process's RAM
    /// (`iceberg::memory::MemoryCatalog`'s `Mutex<NamespaceState>`, never
    /// scanned from disk at open). Suitable for one-shot local builds,
    /// tests, and CLI round-trips, not for any workflow that spans more
    /// than one process against the same warehouse. See
    /// `docs/TIMESERIES_PHASE_3_HANDOFF.md` for how this was discovered.
    Memory { warehouse: PathBuf },
    /// SQLite-backed Iceberg catalog (`iceberg-catalog-sql`) plus a local
    /// filesystem warehouse. Durable across process restarts: the
    /// namespace/table registry is a real SQLite database on disk, and
    /// SQLite's own file locking makes it safe for a second process to
    /// open the same `catalog_db` concurrently. `catalog_db` is created
    /// (`CREATE TABLE IF NOT EXISTS`, via SQLite's `mode=rwc` URI query
    /// param) if it does not already exist.
    Sqlite { warehouse: PathBuf, catalog_db: PathBuf },
}

/// Open the catalog described by `config`.
pub async fn open_catalog(config: &CatalogConfig) -> Result<Arc<dyn Catalog>> {
    match config {
        CatalogConfig::Memory { warehouse } => {
            let warehouse_path = warehouse.to_string_lossy().into_owned();
            let catalog = MemoryCatalogBuilder::default()
                .with_storage_factory(Arc::new(LocalFsStorageFactory))
                .load(
                    "cubism-memory",
                    HashMap::from([(MEMORY_CATALOG_WAREHOUSE.to_string(), warehouse_path)]),
                )
                .await
                .map_err(CubismIcebergError::Iceberg)?;
            Ok(Arc::new(catalog))
        }
        CatalogConfig::Sqlite { warehouse, catalog_db } => {
            let warehouse_path = warehouse.to_string_lossy().into_owned();
            let db_uri = format!("sqlite:{}?mode=rwc", catalog_db.to_string_lossy());
            let catalog = SqlCatalogBuilder::default()
                .with_storage_factory(Arc::new(LocalFsStorageFactory))
                .load(
                    "cubism-sqlite",
                    HashMap::from([
                        (SQL_CATALOG_PROP_URI.to_string(), db_uri),
                        (SQL_CATALOG_PROP_WAREHOUSE.to_string(), warehouse_path),
                        (SQL_CATALOG_PROP_BIND_STYLE.to_string(), SqlBindStyle::QMark.to_string()),
                    ]),
                )
                .await
                .map_err(CubismIcebergError::Iceberg)?;
            Ok(Arc::new(catalog))
        }
    }
}
