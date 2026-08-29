//! End-to-end temporal corrections: the aggregation/persistence link
//! [#16](https://github.com/jeromebanks/cubism-rs/issues/16) asked for.
//!
//! Before this crate, correcting a window required the caller to do two
//! things by hand that nothing in the codebase could do for them:
//!
//! 1. work out *which* windows a source-event change touches, and
//! 2. rebuild those windows' aggregate states and hand the finished
//!    batches to [`cubism_iceberg::CorrectionCoordinator::execute`].
//!
//! `cubism-iceberg` cannot do either, because both need an aggregation
//! engine and that crate deliberately links none (to stay clear of the
//! DataFusion version seam tracked by
//! [#8](https://github.com/jeromebanks/cubism-rs/issues/8)).
//! `cubism-datafusion` cannot do it either, because it carries no Iceberg
//! dependency. This crate is the one place that has both.
//!
//! - [`windows`] answers "which windows" from the spec's bucket math alone
//!   (plan line 599), and defines the canonical `bucket_start -> WindowId`
//!   encoding that question requires.
//! - [`engine`] runs the whole correction: rebuild from corrected source,
//!   then append-and-CAS-publish through the existing coordinator.

pub mod engine;
pub mod windows;

pub use engine::{CorrectionOutcome, CorrectionEngine, WindowCorrection};
pub use windows::{affected_windows, window_id_for, AffectedWindow};

/// This crate's error type. Deliberately keeps the two underlying failure
/// domains distinguishable rather than flattening them to strings: a
/// rebuild failing (bad spec, unreadable source) and a publish failing
/// (stale CAS, a concurrent correction) call for different responses from
/// a caller, and only the second is worth retrying.
#[derive(Debug, thiserror::Error)]
pub enum CorrectError {
    #[error("cannot name a window: {0}")]
    WindowNaming(String),
    #[error(
        "window '{window_id}' is at revision {previous}; the next revision is not representable"
    )]
    RevisionOverflow { window_id: String, previous: u64 },
    /// A multi-window correction that stopped part-way, carrying the
    /// progress made before it stopped.
    ///
    /// Windows are corrected one at a time, each with its own Iceberg
    /// commit, and this stack has no cross-commit transaction — so a
    /// mid-run failure genuinely leaves earlier windows corrected. This
    /// variant is how that partial progress reaches the caller: without
    /// it, `?` would discard the record of what already landed and a
    /// retry would have to restart rather than resume.
    ///
    /// `corrected` may be empty (the first window failed). The underlying
    /// failure is the [`source`](std::error::Error::source).
    #[error("correction stopped after {} window(s): {source}", .corrected.len())]
    Partial {
        corrected: Vec<engine::WindowCorrection>,
        skipped_unpublished: Vec<cubism_core::temporal::WindowId>,
        #[source]
        source: Box<CorrectError>,
    },
    #[error("cube '{0}' has no temporal spec; corrections are a temporal-only operation")]
    NotTemporal(String),
    #[error("rebuilding corrected states failed: {0}")]
    Rebuild(#[from] cubism_datafusion::datafusion::error::DataFusionError),
    #[error(transparent)]
    Persist(#[from] cubism_iceberg::CubismIcebergError),
    #[error("window '{window_id}' has never been published; a correction revises an existing window, it does not create one")]
    UnpublishedWindow { window_id: String },
}
