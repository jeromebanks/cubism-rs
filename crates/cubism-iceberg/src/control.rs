use std::collections::HashMap;
use std::sync::Mutex;

use cubism_core::temporal::{WindowId, WindowRevision};

use crate::error::{CubismIcebergError, Result};

/// Stable identity for one cube's build window: `cube_id` plus
/// `cubism-core`'s own validated [`WindowId`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WindowKey {
    cube_id: String,
    window_id: String,
}

impl WindowKey {
    fn new(cube_id: &str, window_id: &WindowId) -> Self {
        Self {
            cube_id: cube_id.to_string(),
            window_id: window_id.as_str().to_string(),
        }
    }
}

/// Provenance for the one revision currently visible to readers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub revision: WindowRevision,
    pub run_id: String,
    pub aggregate_snapshot_id: i64,
}

/// State used to make an append job recoverable across retries. Reuses the
/// Phase 0A spike's proven claim/append/publish protocol
/// (`crates/cubism-iceberg-spike/src/lib.rs`), keyed on `cubism-core`'s real
/// types instead of the spike's bespoke composite key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    Claimed {
        window_key: (String, String),
        revision: WindowRevision,
        expected_rows: u64,
    },
    Appended {
        window_key: (String, String),
        revision: WindowRevision,
        expected_rows: u64,
        aggregate_snapshot_id: i64,
    },
    Published {
        window_key: (String, String),
        revision: WindowRevision,
        expected_rows: u64,
        aggregate_snapshot_id: i64,
    },
}

impl RunState {
    fn window_key(&self) -> &(String, String) {
        match self {
            Self::Claimed { window_key, .. }
            | Self::Appended { window_key, .. }
            | Self::Published { window_key, .. } => window_key,
        }
    }

    fn revision(&self) -> WindowRevision {
        match self {
            Self::Claimed { revision, .. }
            | Self::Appended { revision, .. }
            | Self::Published { revision, .. } => *revision,
        }
    }

    fn expected_rows(&self) -> u64 {
        match self {
            Self::Claimed { expected_rows, .. }
            | Self::Appended { expected_rows, .. }
            | Self::Published { expected_rows, .. } => *expected_rows,
        }
    }
}

/// Result of atomically claiming a deterministic run ID. Only `New` may
/// start an append; `Existing` must reconcile the prior run instead.
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

#[derive(Debug, Default)]
struct State {
    publications: HashMap<WindowKey, Publication>,
    runs: HashMap<String, RunState>,
}

/// In-process publication control store: claims runs, records append
/// results, and publishes revisions via compare-and-swap.
///
/// **Not durable across process restarts.** This is the "memory" half of
/// Phase 0A's still-open catalog/control-store gate (`docs/PHASE_0A_RESULTS.md`
/// remaining gate #1) — the CAS *protocol* here is the one Phase 0A proved
/// correct, but the *backend* is an in-process `Mutex<HashMap>`, not a
/// crash-durable store. A production deployment needs a SQL table or
/// equivalent behind this same interface.
#[derive(Debug, Default)]
pub struct PublicationStore {
    state: Mutex<State>,
}

impl PublicationStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim a deterministic run, or return the prior state for an
    /// identical retry. Reusing a `run_id` for different immutable inputs
    /// is rejected via [`CubismIcebergError::RunConflict`].
    pub fn claim_run(
        &self,
        cube_id: &str,
        window_id: &WindowId,
        run_id: &str,
        revision: WindowRevision,
        expected_rows: u64,
    ) -> Result<ClaimResult> {
        let window_key = WindowKey::new(cube_id, window_id);
        let mut state = self.state.lock().expect("control store mutex poisoned");

        if let Some(existing) = state.runs.get(run_id) {
            let existing_key = existing.window_key();
            if existing_key.0 != window_key.cube_id
                || existing_key.1 != window_key.window_id
                || existing.revision() != revision
                || existing.expected_rows() != expected_rows
            {
                return Err(CubismIcebergError::RunConflict {
                    run_id: run_id.to_string(),
                    window_id: window_key.window_id,
                    revision: revision.get(),
                    expected_rows,
                });
            }
            return Ok(ClaimResult::Existing(existing.clone()));
        }

        let claimed = RunState::Claimed {
            window_key: (window_key.cube_id, window_key.window_id),
            revision,
            expected_rows,
        };
        state.runs.insert(run_id.to_string(), claimed.clone());
        Ok(ClaimResult::New(claimed))
    }

    /// Record the atomic Iceberg snapshot produced by an append. A retry
    /// with the same snapshot ID is harmless.
    pub fn record_append(&self, run_id: &str, aggregate_snapshot_id: i64) -> Result<RunState> {
        let mut state = self.state.lock().expect("control store mutex poisoned");
        let current = state
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| unknown_run(run_id))?;

        let appended = match current {
            RunState::Claimed { window_key, revision, expected_rows } => RunState::Appended {
                window_key,
                revision,
                expected_rows,
                aggregate_snapshot_id,
            },
            RunState::Appended { aggregate_snapshot_id: existing, .. }
            | RunState::Published { aggregate_snapshot_id: existing, .. }
                if existing == aggregate_snapshot_id =>
            {
                current
            }
            other => {
                return Err(CubismIcebergError::RunConflict {
                    run_id: run_id.to_string(),
                    window_id: other.window_key().1.clone(),
                    revision: other.revision().get(),
                    expected_rows: other.expected_rows(),
                });
            }
        };

        state.runs.insert(run_id.to_string(), appended.clone());
        Ok(appended)
    }

    /// Atomically change the current revision for a window, iff the caller
    /// observed the expected current revision (CAS). `None` means "the
    /// window has never been published."
    pub fn publish(&self, run_id: &str, expected_current: Option<WindowRevision>) -> Result<Publication> {
        let mut state = self.state.lock().expect("control store mutex poisoned");
        let run = state
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| unknown_run(run_id))?;

        let (window_key, revision, expected_rows, aggregate_snapshot_id) = match run {
            RunState::Appended { window_key, revision, expected_rows, aggregate_snapshot_id }
            | RunState::Published { window_key, revision, expected_rows, aggregate_snapshot_id } => {
                (window_key, revision, expected_rows, aggregate_snapshot_id)
            }
            RunState::Claimed { .. } => {
                return Err(CubismIcebergError::RunNotAppended { run_id: run_id.to_string() });
            }
        };

        let key = WindowKey {
            cube_id: window_key.0.clone(),
            window_id: window_key.1.clone(),
        };
        let actual = state.publications.get(&key).map(|p| p.revision);
        if actual == Some(revision) {
            return Ok(state.publications.get(&key).expect("publication just observed").clone());
        }
        if actual != expected_current {
            return Err(CubismIcebergError::StaleRevision {
                window_id: window_key.1,
                expected: expected_current.map(WindowRevision::get),
                actual: actual.map(WindowRevision::get),
            });
        }

        let publication = Publication { revision, run_id: run_id.to_string(), aggregate_snapshot_id };
        state.publications.insert(key, publication.clone());
        state.runs.insert(
            run_id.to_string(),
            RunState::Published { window_key, revision, expected_rows, aggregate_snapshot_id },
        );
        Ok(publication)
    }

    /// The currently published revision for a window, if any.
    pub fn current(&self, cube_id: &str, window_id: &WindowId) -> Option<WindowRevision> {
        let state = self.state.lock().expect("control store mutex poisoned");
        state
            .publications
            .get(&WindowKey::new(cube_id, window_id))
            .map(|publication| publication.revision)
    }

    pub fn run_state(&self, run_id: &str) -> Option<RunState> {
        let state = self.state.lock().expect("control store mutex poisoned");
        state.runs.get(run_id).cloned()
    }
}

fn unknown_run(run_id: &str) -> CubismIcebergError {
    CubismIcebergError::UnknownRun(run_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str) -> WindowId {
        WindowId::new(id).unwrap()
    }

    fn revision(n: u64) -> WindowRevision {
        WindowRevision::new(n).unwrap()
    }

    #[test]
    fn deterministic_retry_returns_existing_run_state() {
        let store = PublicationStore::new();
        let w = window("2026-08-12");
        let first = store.claim_run("cube", &w, "run-1", revision(1), 10).unwrap();
        let retry = store.claim_run("cube", &w, "run-1", revision(1), 10).unwrap();
        assert!(matches!(first, ClaimResult::New(_)));
        assert!(matches!(retry, ClaimResult::Existing(_)));
        assert_eq!(first.state(), retry.state());
    }

    #[test]
    fn claiming_the_same_run_id_with_different_inputs_is_rejected() {
        let store = PublicationStore::new();
        let w = window("2026-08-12");
        store.claim_run("cube", &w, "run-1", revision(1), 10).unwrap();
        let error = store.claim_run("cube", &w, "run-1", revision(2), 10).unwrap_err();
        assert!(matches!(error, CubismIcebergError::RunConflict { .. }));
    }

    #[test]
    fn publish_requires_the_run_to_be_appended_first() {
        let store = PublicationStore::new();
        let w = window("2026-08-12");
        store.claim_run("cube", &w, "run-1", revision(1), 10).unwrap();
        let error = store.publish("run-1", None).unwrap_err();
        assert!(matches!(error, CubismIcebergError::RunNotAppended { .. }));
    }

    #[test]
    fn stale_publish_cannot_replace_a_newer_revision() {
        let store = PublicationStore::new();
        let w = window("2026-08-12");

        store.claim_run("cube", &w, "run-1", revision(1), 10).unwrap();
        store.record_append("run-1", 101).unwrap();
        store.publish("run-1", None).unwrap();

        store.claim_run("cube", &w, "run-2", revision(2), 10).unwrap();
        store.record_append("run-2", 102).unwrap();
        store.claim_run("cube", &w, "run-3", revision(3), 10).unwrap();
        store.record_append("run-3", 103).unwrap();

        store.publish("run-2", Some(revision(1))).unwrap();
        let error = store.publish("run-3", Some(revision(1))).unwrap_err();
        assert!(matches!(
            error,
            CubismIcebergError::StaleRevision { expected: Some(1), actual: Some(2), .. }
        ));
        assert_eq!(store.current("cube", &w), Some(revision(2)));
    }

    #[test]
    fn a_republish_of_the_already_current_revision_is_a_harmless_no_op() {
        let store = PublicationStore::new();
        let w = window("2026-08-12");
        store.claim_run("cube", &w, "run-1", revision(1), 10).unwrap();
        store.record_append("run-1", 101).unwrap();
        let first = store.publish("run-1", None).unwrap();
        let second = store.publish("run-1", None).unwrap();
        assert_eq!(first, second);
    }
}
