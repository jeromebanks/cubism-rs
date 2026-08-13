use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iceberg::io::LocalFsStorageFactory;
use iceberg::memory::{MEMORY_CATALOG_WAREHOUSE, MemoryCatalogBuilder};
use iceberg::{Catalog, CatalogBuilder};

use crate::error::{CubismIcebergError, Result};

/// How to reach the Iceberg catalog and warehouse.
///
/// Only [`CatalogConfig::Memory`] is implemented. A durable SQL/REST/Glue
/// catalog and a real object store (S3/GCS) are Phase 0A gates #1-#2
/// (`docs/PHASE_0A_RESULTS.md`) — infrastructure/vendor decisions this
/// crate does not make on its own. The enum is left open so a future
/// variant can be added without changing callers that already match on it
/// exhaustively via `open_catalog`.
#[derive(Debug, Clone)]
pub enum CatalogConfig {
    /// In-memory Iceberg catalog backed by a local filesystem warehouse.
    /// Not durable across process restarts — suitable for local builds,
    /// tests, and CLI round-trips, not production.
    Memory { warehouse: PathBuf },
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
    }
}
