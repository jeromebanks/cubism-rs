//! Phase 0A feasibility code for Cubism temporal aggregates on Iceberg.
//!
//! This crate is deliberately isolated from Cubism's production crates. It
//! proves dependency compatibility and correctness properties before any
//! temporal API or storage implementation is accepted.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use datafusion::arrow::array::{Array, ArrayRef, TimestampMicrosecondArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::error::ArrowError;
use datafusion::arrow::record_batch::RecordBatch;
use thiserror::Error;

/// Normalizes Arrow's named UTC timezone to Iceberg's canonical `+00:00`
/// timezone without changing timestamp instants.
///
/// Iceberg Rust 0.10 has an open low-level-writer issue where semantically
/// equivalent `Timestamp(Microsecond, "UTC")` arrays are rejected when the
/// Iceberg `timestamptz` schema expects
/// `Timestamp(Microsecond, "+00:00")`.
pub fn normalize_utc_timezones(batch: &RecordBatch) -> Result<RecordBatch, ArrowError> {
    let mut fields = Vec::with_capacity(batch.num_columns());
    let mut columns = Vec::with_capacity(batch.num_columns());

    for (field, column) in batch.schema().fields().iter().zip(batch.columns()) {
        match field.data_type() {
            DataType::Timestamp(
                datafusion::arrow::datatypes::TimeUnit::Microsecond,
                Some(zone),
            ) if zone.as_ref() == "UTC" => {
                let timestamps = column
                    .as_any()
                    .downcast_ref::<TimestampMicrosecondArray>()
                    .ok_or_else(|| {
                        ArrowError::CastError(format!(
                            "column {} declares a microsecond timestamp but has array type {}",
                            field.name(),
                            column.data_type()
                        ))
                    })?;
                let normalized =
                    TimestampMicrosecondArray::from_iter(timestamps.iter()).with_timezone("+00:00");
                let normalized: ArrayRef = Arc::new(normalized);
                fields.push(Arc::new(
                    Field::new(
                        field.name(),
                        normalized.data_type().clone(),
                        field.is_nullable(),
                    )
                    .with_metadata(field.metadata().clone()),
                ));
                columns.push(normalized);
            }
            _ => {
                fields.push(field.clone());
                columns.push(column.clone());
            }
        }
    }

    let schema = Arc::new(Schema::new_with_metadata(
        fields,
        batch.schema().metadata().clone(),
    ));
    RecordBatch::try_new(schema, columns)
}

/// Stable identity for one immutable build window.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowId {
    pub cube_id: String,
    pub window_id: String,
}

impl WindowId {
    pub fn new(cube_id: impl Into<String>, window_id: impl Into<String>) -> Self {
        Self {
            cube_id: cube_id.into(),
            window_id: window_id.into(),
        }
    }
}

/// Provenance for the one revision visible to readers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub revision: i64,
    pub run_id: String,
    pub aggregate_snapshot_id: i64,
}

/// State used to make an append job recoverable across retries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    Claimed {
        window: WindowId,
        revision: i64,
        expected_rows: usize,
    },
    Appended {
        window: WindowId,
        revision: i64,
        expected_rows: usize,
        aggregate_snapshot_id: i64,
    },
    Published {
        window: WindowId,
        revision: i64,
        expected_rows: usize,
        aggregate_snapshot_id: i64,
    },
}

/// Result of atomically claiming a deterministic run ID.
///
/// Only the caller receiving `New` may start an append. A caller receiving
/// `Existing` must observe/reconcile the prior run instead of appending again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimResult {
    New(RunState),
    Existing(RunState),
}

impl ClaimResult {
    pub fn state(&self) -> &RunState {
        match self {
            Self::New(state) | Self::Existing(state) => state,
        }
    }
}

impl RunState {
    pub fn window(&self) -> &WindowId {
        match self {
            Self::Claimed { window, .. }
            | Self::Appended { window, .. }
            | Self::Published { window, .. } => window,
        }
    }

    pub fn revision(&self) -> i64 {
        match self {
            Self::Claimed { revision, .. }
            | Self::Appended { revision, .. }
            | Self::Published { revision, .. } => *revision,
        }
    }

    pub fn expected_rows(&self) -> usize {
        match self {
            Self::Claimed { expected_rows, .. }
            | Self::Appended { expected_rows, .. }
            | Self::Published { expected_rows, .. } => *expected_rows,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControlError {
    #[error("run {run_id} was retried with different immutable inputs")]
    RunIdentityConflict { run_id: String },
    #[error("run {run_id} has not been claimed")]
    UnknownRun { run_id: String },
    #[error("run {run_id} has not recorded a committed aggregate snapshot")]
    RunNotAppended { run_id: String },
    #[error(
        "stale publication for {window_id}: expected revision {expected:?}, current revision {actual:?}"
    )]
    StalePublication {
        window_id: String,
        expected: Option<i64>,
        actual: Option<i64>,
    },
    #[error("control-store mutex was poisoned")]
    Poisoned,
}

#[derive(Debug, Default)]
struct ControlState {
    publications: HashMap<WindowId, Publication>,
    runs: HashMap<String, RunState>,
}

/// Minimal model of the transactional control-store contract required by Phase
/// 0A. It is not the proposed production backend.
#[derive(Debug, Default)]
pub struct ControlStore {
    state: Mutex<ControlState>,
}

impl ControlStore {
    /// Claims a deterministic run, or returns the prior state for an identical
    /// retry. Reusing a run ID for different immutable inputs is rejected.
    pub fn claim_run(
        &self,
        run_id: impl Into<String>,
        window: WindowId,
        revision: i64,
        expected_rows: usize,
    ) -> Result<ClaimResult, ControlError> {
        let run_id = run_id.into();
        let mut state = self.state.lock().map_err(|_| ControlError::Poisoned)?;

        if let Some(existing) = state.runs.get(&run_id) {
            if existing.window() != &window
                || existing.revision() != revision
                || existing.expected_rows() != expected_rows
            {
                return Err(ControlError::RunIdentityConflict { run_id });
            }
            return Ok(ClaimResult::Existing(existing.clone()));
        }

        let claimed = RunState::Claimed {
            window,
            revision,
            expected_rows,
        };
        state.runs.insert(run_id, claimed.clone());
        Ok(ClaimResult::New(claimed))
    }

    /// Records the atomic Iceberg snapshot produced by an append. A retry with
    /// the same snapshot is harmless.
    pub fn record_append(
        &self,
        run_id: &str,
        aggregate_snapshot_id: i64,
    ) -> Result<RunState, ControlError> {
        let mut state = self.state.lock().map_err(|_| ControlError::Poisoned)?;
        let current = state
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| ControlError::UnknownRun {
                run_id: run_id.to_string(),
            })?;

        let appended = match current {
            RunState::Claimed {
                window,
                revision,
                expected_rows,
            } => RunState::Appended {
                window,
                revision,
                expected_rows,
                aggregate_snapshot_id,
            },
            RunState::Appended {
                aggregate_snapshot_id: existing,
                ..
            }
            | RunState::Published {
                aggregate_snapshot_id: existing,
                ..
            } if existing == aggregate_snapshot_id => current,
            _ => {
                return Err(ControlError::RunIdentityConflict {
                    run_id: run_id.to_string(),
                });
            }
        };

        state.runs.insert(run_id.to_string(), appended.clone());
        Ok(appended)
    }

    /// Atomically changes the current revision for a window if the caller
    /// observed the expected revision.
    pub fn publish(
        &self,
        run_id: &str,
        expected_current_revision: Option<i64>,
    ) -> Result<Publication, ControlError> {
        let mut state = self.state.lock().map_err(|_| ControlError::Poisoned)?;
        let run = state
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| ControlError::UnknownRun {
                run_id: run_id.to_string(),
            })?;

        let (window, revision, expected_rows, aggregate_snapshot_id) = match run {
            RunState::Appended {
                window,
                revision,
                expected_rows,
                aggregate_snapshot_id,
            }
            | RunState::Published {
                window,
                revision,
                expected_rows,
                aggregate_snapshot_id,
            } => (window, revision, expected_rows, aggregate_snapshot_id),
            RunState::Claimed { .. } => {
                return Err(ControlError::RunNotAppended {
                    run_id: run_id.to_string(),
                });
            }
        };

        let actual = state.publications.get(&window).map(|p| p.revision);
        if actual == Some(revision) {
            return Ok(state
                .publications
                .get(&window)
                .expect("publication was just observed")
                .clone());
        }
        if actual != expected_current_revision {
            return Err(ControlError::StalePublication {
                window_id: window.window_id.clone(),
                expected: expected_current_revision,
                actual,
            });
        }

        let publication = Publication {
            revision,
            run_id: run_id.to_string(),
            aggregate_snapshot_id,
        };
        state
            .publications
            .insert(window.clone(), publication.clone());
        state.runs.insert(
            run_id.to_string(),
            RunState::Published {
                window,
                revision,
                expected_rows,
                aggregate_snapshot_id,
            },
        );
        Ok(publication)
    }

    pub fn publications(&self) -> Result<Vec<(WindowId, Publication)>, ControlError> {
        let state = self.state.lock().map_err(|_| ControlError::Poisoned)?;
        let mut rows: Vec<_> = state
            .publications
            .iter()
            .map(|(window, publication)| (window.clone(), publication.clone()))
            .collect();
        rows.sort_by(|left, right| {
            (&left.0.cube_id, &left.0.window_id).cmp(&(&right.0.cube_id, &right.0.window_id))
        });
        Ok(rows)
    }

    pub fn run_state(&self, run_id: &str) -> Result<Option<RunState>, ControlError> {
        let state = self.state.lock().map_err(|_| ControlError::Poisoned)?;
        Ok(state.runs.get(run_id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn stale_publish_cannot_replace_a_newer_revision() {
        let store = ControlStore::default();
        let window = WindowId::new("cube", "2026-07-28");

        assert!(matches!(
            store.claim_run("run-1", window.clone(), 1, 10).unwrap(),
            ClaimResult::New(_)
        ));
        store.record_append("run-1", 101).unwrap();
        store.publish("run-1", None).unwrap();

        assert!(matches!(
            store.claim_run("run-2", window.clone(), 2, 10).unwrap(),
            ClaimResult::New(_)
        ));
        store.record_append("run-2", 102).unwrap();
        assert!(matches!(
            store.claim_run("run-3", window, 3, 10).unwrap(),
            ClaimResult::New(_)
        ));
        store.record_append("run-3", 103).unwrap();

        store.publish("run-2", Some(1)).unwrap();
        let error = store.publish("run-3", Some(1)).unwrap_err();
        assert_eq!(
            error,
            ControlError::StalePublication {
                window_id: "2026-07-28".to_string(),
                expected: Some(1),
                actual: Some(2),
            }
        );
    }

    #[test]
    fn deterministic_retry_returns_existing_run_state() {
        let store = ControlStore::default();
        let window = WindowId::new("cube", "2026-07-28");
        let first = store.claim_run("run-1", window.clone(), 1, 10).unwrap();
        let retry = store.claim_run("run-1", window, 1, 10).unwrap();
        assert!(matches!(first, ClaimResult::New(_)));
        assert!(matches!(retry, ClaimResult::Existing(_)));
        assert_eq!(first.state(), retry.state());
    }

    #[test]
    fn concurrent_claim_has_exactly_one_append_owner() {
        let store = Arc::new(ControlStore::default());
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();

        for _ in 0..2 {
            let store = store.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                store
                    .claim_run("run-1", WindowId::new("cube", "2026-07-28"), 1, 10)
                    .unwrap()
            }));
        }
        barrier.wait();

        let outcomes: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, ClaimResult::New(_)))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, ClaimResult::Existing(_)))
                .count(),
            1
        );
    }
}
