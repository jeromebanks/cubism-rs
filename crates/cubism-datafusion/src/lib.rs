//! DataFusion execution layer for Cubism.
//!
//! The explode step runs as a scalar UDF (`cubism_xunit_keys`) + SQL
//! `unnest` — streaming execution with minimal DataFusion API surface. See
//! [`build_cube`] for the end-to-end entry point.

pub mod build;
pub mod range_query;
pub mod series_merge;
pub mod state_udaf;
pub mod temporal_build;
pub mod udaf;
pub mod udf;

pub use build::{build_cube, cube_sql};
pub use cubism_core;
pub use datafusion;
pub use range_query::{
    CoveragePlan, GapPolicy, ResolutionPlan, ResolutionSegment, SegmentCoverage, TemporalQuery,
};
pub use series_merge::merge_average_column;
pub use temporal_build::build_temporal;
