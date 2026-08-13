//! SQLite-backed [`crate::control::PublicationStore::Sqlite`] variant: the
//! same claim/append/publish protocol as the in-process store, persisted so
//! it survives a process restart and so a freshly opened handle can
//! perform a real compare-and-swap against a prior revision.
//!
//! Each operation runs inside one SQLite transaction (`self.pool.begin()`,
//! a plain deferred `BEGIN`), which gives correct read-then-write semantics
//! for a single writer at a time. It does **not** implement retry-on-BUSY
//! or `BEGIN IMMEDIATE` locking for genuinely concurrent writers racing on
//! the same window — that is Phase 4 scope ("two same-window writers
//! produce one published winner", `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`
//! line 636), not this durability slice's.

use std::path::Path;
use std::time::Duration;

use cubism_core::temporal::{WindowId, WindowRevision};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};

use crate::control::{ClaimResult, Publication, RunState};
use crate::error::{CubismIcebergError, Result};

pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Open (creating if missing) the control-store database at `db_path`
    /// and ensure its schema exists. Safe to call again from a second
    /// process or a second handle pointed at the same path — table
    /// creation is `IF NOT EXISTS` and every write below is transactional.
    pub async fn open(db_path: &Path) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .busy_timeout(Duration::from_secs(5));
        // A single-connection pool makes every query on this handle queue
        // behind one physical SQLite connection, so this handle never
        // races itself; a second handle/process still arbitrates through
        // SQLite's own file locking (see the module doc comment).
        let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS control_runs (
                run_id TEXT PRIMARY KEY,
                cube_id TEXT NOT NULL,
                window_id TEXT NOT NULL,
                revision INTEGER NOT NULL,
                expected_rows INTEGER NOT NULL,
                status TEXT NOT NULL,
                aggregate_snapshot_id INTEGER
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS control_publications (
                cube_id TEXT NOT NULL,
                window_id TEXT NOT NULL,
                revision INTEGER NOT NULL,
                run_id TEXT NOT NULL,
                aggregate_snapshot_id INTEGER NOT NULL,
                PRIMARY KEY (cube_id, window_id)
            )",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn claim_run(
        &self,
        cube_id: &str,
        window_id: &WindowId,
        run_id: &str,
        revision: WindowRevision,
        expected_rows: u64,
    ) -> Result<ClaimResult> {
        let mut tx = self.pool.begin().await?;

        let inserted = sqlx::query(
            "INSERT INTO control_runs (run_id, cube_id, window_id, revision, expected_rows, status, aggregate_snapshot_id)
             VALUES (?, ?, ?, ?, ?, 'claimed', NULL)
             ON CONFLICT(run_id) DO NOTHING",
        )
        .bind(run_id)
        .bind(cube_id)
        .bind(window_id.as_str())
        .bind(revision.get() as i64)
        .bind(expected_rows as i64)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;

        let row = fetch_run_row(&mut tx, run_id)
            .await?
            .expect("row just inserted or already present");
        tx.commit().await?;
        let state = row_to_run_state(&row)?;

        if inserted {
            return Ok(ClaimResult::New(state));
        }

        let window_key = state_window_key(&state);
        if window_key.0 != cube_id
            || window_key.1 != window_id.as_str()
            || state_revision(&state) != revision
            || state_expected_rows(&state) != expected_rows
        {
            return Err(CubismIcebergError::RunConflict {
                run_id: run_id.to_string(),
                window_id: window_id.as_str().to_string(),
                revision: revision.get(),
                expected_rows,
            });
        }
        Ok(ClaimResult::Existing(state))
    }

    pub async fn record_append(&self, run_id: &str, aggregate_snapshot_id: i64) -> Result<RunState> {
        let mut tx = self.pool.begin().await?;

        let updated = sqlx::query(
            "UPDATE control_runs SET status = 'appended', aggregate_snapshot_id = ?
             WHERE run_id = ? AND status = 'claimed'",
        )
        .bind(aggregate_snapshot_id)
        .bind(run_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;

        let row = fetch_run_row(&mut tx, run_id)
            .await?
            .ok_or_else(|| CubismIcebergError::UnknownRun(run_id.to_string()))?;
        tx.commit().await?;
        let state = row_to_run_state(&row)?;

        if updated {
            return Ok(state);
        }

        match &state {
            RunState::Appended { aggregate_snapshot_id: existing, .. }
            | RunState::Published { aggregate_snapshot_id: existing, .. }
                if *existing == aggregate_snapshot_id =>
            {
                Ok(state)
            }
            other => Err(CubismIcebergError::RunConflict {
                run_id: run_id.to_string(),
                window_id: state_window_key(other).1,
                revision: state_revision(other).get(),
                expected_rows: state_expected_rows(other),
            }),
        }
    }

    pub async fn publish(&self, run_id: &str, expected_current: Option<WindowRevision>) -> Result<Publication> {
        let mut tx = self.pool.begin().await?;

        let run_row = fetch_run_row(&mut tx, run_id)
            .await?
            .ok_or_else(|| CubismIcebergError::UnknownRun(run_id.to_string()))?;
        let run_state = row_to_run_state(&run_row)?;

        let (cube_id, window_id, revision, aggregate_snapshot_id) = match run_state {
            RunState::Appended { window_key, revision, aggregate_snapshot_id, .. }
            | RunState::Published { window_key, revision, aggregate_snapshot_id, .. } => {
                (window_key.0, window_key.1, revision, aggregate_snapshot_id)
            }
            RunState::Claimed { .. } => {
                return Err(CubismIcebergError::RunNotAppended { run_id: run_id.to_string() });
            }
        };

        let pub_row = sqlx::query(
            "SELECT revision, run_id, aggregate_snapshot_id FROM control_publications
             WHERE cube_id = ? AND window_id = ?",
        )
        .bind(&cube_id)
        .bind(&window_id)
        .fetch_optional(&mut *tx)
        .await?;

        let actual_revision = pub_row
            .as_ref()
            .map(|row| -> Result<WindowRevision> { Ok(WindowRevision::new(row.try_get::<i64, _>("revision")? as u64)?) })
            .transpose()?;

        if actual_revision == Some(revision) {
            let row = pub_row.expect("publication just observed");
            tx.commit().await?;
            return Ok(Publication {
                revision,
                run_id: row.try_get("run_id")?,
                aggregate_snapshot_id: row.try_get("aggregate_snapshot_id")?,
            });
        }

        if actual_revision != expected_current {
            return Err(CubismIcebergError::StaleRevision {
                window_id,
                expected: expected_current.map(WindowRevision::get),
                actual: actual_revision.map(WindowRevision::get),
            });
        }

        sqlx::query(
            "INSERT INTO control_publications (cube_id, window_id, revision, run_id, aggregate_snapshot_id)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(cube_id, window_id) DO UPDATE SET
                 revision = excluded.revision,
                 run_id = excluded.run_id,
                 aggregate_snapshot_id = excluded.aggregate_snapshot_id",
        )
        .bind(&cube_id)
        .bind(&window_id)
        .bind(revision.get() as i64)
        .bind(run_id)
        .bind(aggregate_snapshot_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE control_runs SET status = 'published' WHERE run_id = ?")
            .bind(run_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(Publication { revision, run_id: run_id.to_string(), aggregate_snapshot_id })
    }

    pub async fn current(&self, cube_id: &str, window_id: &WindowId) -> Result<Option<WindowRevision>> {
        let row = sqlx::query("SELECT revision FROM control_publications WHERE cube_id = ? AND window_id = ?")
            .bind(cube_id)
            .bind(window_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| Ok(WindowRevision::new(row.try_get::<i64, _>("revision")? as u64)?)).transpose()
    }

    pub async fn run_state(&self, run_id: &str) -> Result<Option<RunState>> {
        let row = sqlx::query(
            "SELECT cube_id, window_id, revision, expected_rows, status, aggregate_snapshot_id
             FROM control_runs WHERE run_id = ?",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| row_to_run_state(&row)).transpose()
    }
}

async fn fetch_run_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    run_id: &str,
) -> Result<Option<SqliteRow>> {
    Ok(sqlx::query(
        "SELECT cube_id, window_id, revision, expected_rows, status, aggregate_snapshot_id
         FROM control_runs WHERE run_id = ?",
    )
    .bind(run_id)
    .fetch_optional(&mut **tx)
    .await?)
}

fn row_to_run_state(row: &SqliteRow) -> Result<RunState> {
    let cube_id: String = row.try_get("cube_id")?;
    let window_id: String = row.try_get("window_id")?;
    let revision: i64 = row.try_get("revision")?;
    let expected_rows: i64 = row.try_get("expected_rows")?;
    let status: String = row.try_get("status")?;
    let aggregate_snapshot_id: Option<i64> = row.try_get("aggregate_snapshot_id")?;

    let revision = WindowRevision::new(revision as u64)?;
    let expected_rows = expected_rows as u64;
    let window_key = (cube_id, window_id);

    let corrupt = |detail: &str| CubismIcebergError::CorruptControlStore(detail.to_string());

    Ok(match status.as_str() {
        "claimed" => RunState::Claimed { window_key, revision, expected_rows },
        "appended" => RunState::Appended {
            window_key,
            revision,
            expected_rows,
            aggregate_snapshot_id: aggregate_snapshot_id
                .ok_or_else(|| corrupt("run marked 'appended' has no aggregate_snapshot_id"))?,
        },
        "published" => RunState::Published {
            window_key,
            revision,
            expected_rows,
            aggregate_snapshot_id: aggregate_snapshot_id
                .ok_or_else(|| corrupt("run marked 'published' has no aggregate_snapshot_id"))?,
        },
        other => return Err(corrupt(&format!("unknown control_runs.status '{other}'"))),
    })
}

fn state_window_key(state: &RunState) -> (String, String) {
    match state {
        RunState::Claimed { window_key, .. }
        | RunState::Appended { window_key, .. }
        | RunState::Published { window_key, .. } => window_key.clone(),
    }
}

fn state_revision(state: &RunState) -> WindowRevision {
    match state {
        RunState::Claimed { revision, .. }
        | RunState::Appended { revision, .. }
        | RunState::Published { revision, .. } => *revision,
    }
}

fn state_expected_rows(state: &RunState) -> u64 {
    match state {
        RunState::Claimed { expected_rows, .. }
        | RunState::Appended { expected_rows, .. }
        | RunState::Published { expected_rows, .. } => *expected_rows,
    }
}
