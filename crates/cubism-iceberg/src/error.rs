use thiserror::Error;

#[derive(Debug, Error)]
pub enum CubismIcebergError {
    #[error("iceberg operation failed: {0}")]
    Iceberg(#[from] iceberg::Error),

    #[error("arrow operation failed: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),

    #[error("parquet operation failed: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),

    #[error("cubism-core error: {0}")]
    Core(#[from] cubism_core::CubismError),

    #[error("sql control/catalog store operation failed: {0}")]
    Sql(#[from] sqlx::Error),

    #[error("control store row is inconsistent with the claim/append/publish protocol: {0}")]
    CorruptControlStore(String),

    #[error(
        "publish rejected for window '{window_id}': expected current revision {expected:?}, found {actual:?}"
    )]
    StaleRevision {
        window_id: String,
        expected: Option<u64>,
        actual: Option<u64>,
    },

    #[error("duplicate data file path within one append: {0}")]
    DuplicateDataFilePath(String),

    #[error("window '{0}' has no published revision")]
    UnpublishedWindow(String),

    #[error("run '{0}' has not been claimed")]
    UnknownRun(String),

    #[error("run '{run_id}' has not recorded a committed append snapshot yet")]
    RunNotAppended { run_id: String },

    #[error("catalog kind is not implemented in this deployment: {0}")]
    UnsupportedCatalog(String),

    #[error("run '{run_id}' was already claimed with different inputs (window {window_id}, revision {revision}, expected_rows {expected_rows})")]
    RunConflict {
        run_id: String,
        window_id: String,
        revision: u64,
        expected_rows: u64,
    },

    #[error(
        "correction strategy {0:?} is not yet implemented by CorrectionCoordinator (no current AggKind selects it)"
    )]
    UnsupportedCorrectionStrategy(crate::correction::CorrectionStrategy),

    #[error(
        "run '{run_id}' (window '{window_id}', revision {revision}) already published, but is no longer window \
         '{window_id}''s current revision (found {current:?}) — refusing to replay its correction request \
         rather than risk resurrecting a superseded or rolled-back-past revision"
    )]
    RunNoLongerCurrent {
        run_id: String,
        window_id: String,
        revision: u64,
        current: Option<u64>,
    },
}

pub type Result<T> = std::result::Result<T, CubismIcebergError>;
