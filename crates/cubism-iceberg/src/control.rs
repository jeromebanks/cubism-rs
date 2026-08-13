use std::collections::HashMap;
use std::sync::Mutex;

use cubism_core::temporal::{WindowId, WindowRevision};

use crate::durable_control::SqliteStore;
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

/// In-process, `Mutex`-backed implementation of the claim/append/publish
/// protocol. **Not durable across process restarts** — see
/// [`PublicationStore::Sqlite`] for the durable variant.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    state: Mutex<State>,
}

impl InMemoryStore {
    fn claim_run(
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

    fn record_append(&self, run_id: &str, aggregate_snapshot_id: i64) -> Result<RunState> {
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

    fn publish(&self, run_id: &str, expected_current: Option<WindowRevision>) -> Result<Publication> {
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

        let key = WindowKey { cube_id: window_key.0.clone(), window_id: window_key.1.clone() };
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

    fn current(&self, cube_id: &str, window_id: &WindowId) -> Option<WindowRevision> {
        let state = self.state.lock().expect("control store mutex poisoned");
        state
            .publications
            .get(&WindowKey::new(cube_id, window_id))
            .map(|publication| publication.revision)
    }

    fn run_state(&self, run_id: &str) -> Option<RunState> {
        let state = self.state.lock().expect("control store mutex poisoned");
        state.runs.get(run_id).cloned()
    }
}

fn unknown_run(run_id: &str) -> CubismIcebergError {
    CubismIcebergError::UnknownRun(run_id.to_string())
}

/// Publication control store: claims runs, records append results, and
/// publishes revisions via compare-and-swap.
///
/// [`PublicationStore::InMemory`] is the original in-process
/// `Mutex<HashMap>` backend — not durable across process restarts.
/// [`PublicationStore::Sqlite`] persists the same claim/append/publish
/// protocol to a SQLite database, so it survives a restart and a real CAS
/// against a prior revision is possible from a freshly opened handle (see
/// `docs/TIMESERIES_PHASE_4_HANDOFF.md`). Both variants implement the exact
/// same protocol Phase 0A proved correct; only the storage backend differs.
pub enum PublicationStore {
    InMemory(InMemoryStore),
    Sqlite(SqliteStore),
}

impl PublicationStore {
    /// A non-durable, in-process store. Suitable for one-shot builds, tests,
    /// and CLI round-trips that never need a second process to observe this
    /// process's claims or publications.
    pub fn in_memory() -> Self {
        Self::InMemory(InMemoryStore::default())
    }

    /// A SQLite-backed durable store. `db_path` is created
    /// (`CREATE TABLE IF NOT EXISTS`, via SQLite's `mode=rwc` URI query
    /// param) if it does not already exist; opening the same path from a
    /// second process (or a second handle in the same process, after
    /// dropping the first) observes every claim/append/publish already
    /// recorded there.
    pub async fn sqlite(db_path: &std::path::Path) -> Result<Self> {
        Ok(Self::Sqlite(SqliteStore::open(db_path).await?))
    }

    /// Claim a deterministic run, or return the prior state for an
    /// identical retry. Reusing a `run_id` for different immutable inputs
    /// is rejected via [`CubismIcebergError::RunConflict`].
    pub async fn claim_run(
        &self,
        cube_id: &str,
        window_id: &WindowId,
        run_id: &str,
        revision: WindowRevision,
        expected_rows: u64,
    ) -> Result<ClaimResult> {
        match self {
            Self::InMemory(store) => store.claim_run(cube_id, window_id, run_id, revision, expected_rows),
            Self::Sqlite(store) => store.claim_run(cube_id, window_id, run_id, revision, expected_rows).await,
        }
    }

    /// Record the atomic Iceberg snapshot produced by an append. A retry
    /// with the same snapshot ID is harmless.
    pub async fn record_append(&self, run_id: &str, aggregate_snapshot_id: i64) -> Result<RunState> {
        match self {
            Self::InMemory(store) => store.record_append(run_id, aggregate_snapshot_id),
            Self::Sqlite(store) => store.record_append(run_id, aggregate_snapshot_id).await,
        }
    }

    /// Atomically change the current revision for a window, iff the caller
    /// observed the expected current revision (CAS). `None` means "the
    /// window has never been published."
    pub async fn publish(&self, run_id: &str, expected_current: Option<WindowRevision>) -> Result<Publication> {
        match self {
            Self::InMemory(store) => store.publish(run_id, expected_current),
            Self::Sqlite(store) => store.publish(run_id, expected_current).await,
        }
    }

    /// The currently published revision for a window, if any. `Ok(None)`
    /// means unpublished; `Err` means the durable backend itself failed
    /// (e.g. a SQLite I/O error) — callers must not conflate the two.
    pub async fn current(&self, cube_id: &str, window_id: &WindowId) -> Result<Option<WindowRevision>> {
        match self {
            Self::InMemory(store) => Ok(store.current(cube_id, window_id)),
            Self::Sqlite(store) => store.current(cube_id, window_id).await,
        }
    }

    pub async fn run_state(&self, run_id: &str) -> Result<Option<RunState>> {
        match self {
            Self::InMemory(store) => Ok(store.run_state(run_id)),
            Self::Sqlite(store) => store.run_state(run_id).await,
        }
    }
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

    #[tokio::test]
    async fn deterministic_retry_returns_existing_run_state() {
        let store = PublicationStore::in_memory();
        let w = window("2026-08-12");
        let first = store.claim_run("cube", &w, "run-1", revision(1), 10).await.unwrap();
        let retry = store.claim_run("cube", &w, "run-1", revision(1), 10).await.unwrap();
        assert!(matches!(first, ClaimResult::New(_)));
        assert!(matches!(retry, ClaimResult::Existing(_)));
        assert_eq!(first.state(), retry.state());
    }

    #[tokio::test]
    async fn claiming_the_same_run_id_with_different_inputs_is_rejected() {
        let store = PublicationStore::in_memory();
        let w = window("2026-08-12");
        store.claim_run("cube", &w, "run-1", revision(1), 10).await.unwrap();
        let error = store.claim_run("cube", &w, "run-1", revision(2), 10).await.unwrap_err();
        assert!(matches!(error, CubismIcebergError::RunConflict { .. }));
    }

    #[tokio::test]
    async fn publish_requires_the_run_to_be_appended_first() {
        let store = PublicationStore::in_memory();
        let w = window("2026-08-12");
        store.claim_run("cube", &w, "run-1", revision(1), 10).await.unwrap();
        let error = store.publish("run-1", None).await.unwrap_err();
        assert!(matches!(error, CubismIcebergError::RunNotAppended { .. }));
    }

    #[tokio::test]
    async fn stale_publish_cannot_replace_a_newer_revision() {
        let store = PublicationStore::in_memory();
        let w = window("2026-08-12");

        store.claim_run("cube", &w, "run-1", revision(1), 10).await.unwrap();
        store.record_append("run-1", 101).await.unwrap();
        store.publish("run-1", None).await.unwrap();

        store.claim_run("cube", &w, "run-2", revision(2), 10).await.unwrap();
        store.record_append("run-2", 102).await.unwrap();
        store.claim_run("cube", &w, "run-3", revision(3), 10).await.unwrap();
        store.record_append("run-3", 103).await.unwrap();

        store.publish("run-2", Some(revision(1))).await.unwrap();
        let error = store.publish("run-3", Some(revision(1))).await.unwrap_err();
        assert!(matches!(
            error,
            CubismIcebergError::StaleRevision { expected: Some(1), actual: Some(2), .. }
        ));
        assert_eq!(store.current("cube", &w).await.unwrap(), Some(revision(2)));
    }

    #[tokio::test]
    async fn a_republish_of_the_already_current_revision_is_a_harmless_no_op() {
        let store = PublicationStore::in_memory();
        let w = window("2026-08-12");
        store.claim_run("cube", &w, "run-1", revision(1), 10).await.unwrap();
        store.record_append("run-1", 101).await.unwrap();
        let first = store.publish("run-1", None).await.unwrap();
        let second = store.publish("run-1", None).await.unwrap();
        assert_eq!(first, second);
    }
}
