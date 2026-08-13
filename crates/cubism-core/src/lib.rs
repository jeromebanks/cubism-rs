//! # cubism-core
//!
//! Core cube algebra for Cubism: the XUnit/YPath multidimensional model,
//! filter rules, cube-lattice generation, the declarative cube spec, and
//! (in later milestones) mergeable sketch aggregators.
//!
//! This crate is deliberately engine-agnostic: no DataFusion dependency.
//! Execution engines (`cubism-datafusion`) and serving layers link against
//! this crate for the model, spec validation, and sketch-buffer merging.
//!
//! ## The model in one paragraph
//!
//! A [`YPath`] is one dimension's hierarchical coordinate, e.g.
//! `/geo/country=CZ/city=Prague`. An [`XUnit`] is a conjunction of YPaths,
//! at most one per dimension — a single cell of an OLAP cube lattice, e.g.
//! `/geo/country=CZ,/gender=F`. The empty XUnit is the global rollup `/G`.
//! Each input row explodes into every lattice cell it belongs to, pruned by
//! [`FilterRule`]s; aggregation then groups by the XUnit cell key.

pub mod aggregate_state;
pub mod encoding;
pub mod error;
pub mod lattice;
pub mod rules;
pub mod sketch;
pub mod spec;
pub mod temporal;
pub mod ypath;

pub use aggregate_state::{
    AggregateCapabilities, AggregateExactness, AggregateState, AverageState, QuantileState,
    StateVersion, VarianceState, capabilities_for,
};
pub use encoding::{CanonicalXUnit, XUnitContentId, XUnitDictionary};
pub use error::CubismError;
pub use rules::FilterRule;
pub use sketch::KmvSketch;
pub use spec::{AggKind, AggregateStateConfig, CubeSpec, DimensionSpec, LevelSpec, MeasureSpec};
pub use temporal::{
    AllowedLateness, BucketEnd, BucketOrigin, BucketStart, BucketValue, CalendarResolution,
    Coverage, EventTime, Exactness, FixedResolution, IngestionTime, Lateness, LatenessPolicy,
    Resolution, TemporalSpec, TimeBucket, TimeRange, WindowId, WindowRevision,
};
pub use ypath::{XUnit, YPath};
