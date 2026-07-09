//! Mergeable sketch aggregators.
//!
//! Every sketch buffer here is **associative and commutative under merge** —
//! the property the whole system leans on: two-phase distributed
//! aggregation, incremental/streaming updates into existing cubes, and
//! query-time set operations over stored cells all reduce to `merge`.
//!
//! Byte formats are versioned (magic + version header) and locked by
//! golden-file tests: sketch blobs are persisted state, so any format change
//! must bump the version and keep a decode path for the old one.

pub mod kmv;

pub use kmv::KmvSketch;
