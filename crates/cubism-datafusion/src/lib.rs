//! DataFusion execution layer for Cubism (M2).
//!
//! Will provide:
//! - `XUnitExplodeExec`: a `RecordBatchStream` adapter that evaluates the
//!   spec's dimension extractor expressions per batch and emits
//!   `(xunit_key: Binary, measure columns...)` rows for the pruned lattice.
//! - Sketch UDAFs bridging `cubism-core` aggregators into DataFusion's
//!   `Accumulator`/`GroupsAccumulator` model.
//! - `build_cube(spec, ctx)`: the end-to-end entry point.

pub use cubism_core;
