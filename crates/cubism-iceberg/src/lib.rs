//! Append-only Iceberg persistence for Cubism temporal aggregates (Phase 3).
//!
//! Scope for this crate, and what is deliberately out of it, is recorded in
//! `docs/TIMESERIES_PHASE_3_HANDOFF.md`. In short: one local-filesystem
//! catalog variant, an in-process (non-durable) publication control store,
//! and no DataFusion dependency anywhere in this crate — `cubism-datafusion`
//! is on DataFusion 54, the released `iceberg-datafusion` 0.10 pulls
//! DataFusion 53, and their plan/provider types are not interchangeable.
//! This crate only ever crosses that boundary via `arrow_array::RecordBatch`
//! / `arrow_schema::Schema`, which both sides resolve to the same unified
//! `arrow` 58.3 in this workspace.

pub mod config;
pub mod control;
pub mod coordinator;
pub mod correction;
mod durable_control;
pub mod error;
pub mod reader;
pub mod schema;
pub mod table;
pub mod writer;

pub use config::CatalogConfig;
pub use control::{ClaimResult, Publication, PublicationStore, RunState};
pub use coordinator::{
    CorrectionCoordinator, CorrectionRequest, ReconciliationRecord, RevisionStatus, RunInspection,
};
pub use correction::{CorrectionPlan, CorrectionStrategy};
pub use error::CubismIcebergError;
pub use reader::AggregateReader;
pub use schema::{states_iceberg_schema, xunit_registry_iceberg_schema};
pub use table::TemporalTable;
pub use writer::{AggregateWriter, AppendWindow, CommitResult};
