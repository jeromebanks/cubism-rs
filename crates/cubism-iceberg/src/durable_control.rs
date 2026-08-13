//! SQLite-backed [`crate::control::PublicationStore::Sqlite`] variant: the
//! same claim/append/publish protocol as the in-process store, persisted so
//! it survives a process restart and so a freshly opened handle can
//! perform a real compare-and-swap against a prior revision.
//!
//! Each operation runs inside one `BEGIN IMMEDIATE` transaction
//! (`SqliteStore::with_immediate_tx`), not a plain deferred `BEGIN`. A plain
//! deferred transaction only takes a read (`SHARED`) lock at its first
//! statement and defers acquiring a write lock until its first write, which
//! opens a window — two concurrent deferred transactions could in principle
//! both read the not-yet-published state of a window and both conclude
//! their write is a valid compare-and-swap before either commits. `BEGIN
//! IMMEDIATE` acquires the write lock (`RESERVED`) at transaction start
//! instead, closing that window: a second writer's `BEGIN IMMEDIATE` cannot
//! proceed until the first commits, so its read of the prior state is
//! guaranteed current and the existing CAS check in `publish` correctly
//! rejects it. This is Phase 4's own "two same-window writers produce one
//! published winner" requirement (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md`
//! line 636).
//!
//! **What was verified versus what is reasoned from SQLite's documented
//! locking model.** `tests/concurrency.rs` verified two things empirically
//! by temporarily reverting this module to a plain `BEGIN` and observing
//! real behavior, not just asserting an invariant: (1) racing two
//! `PublicationStore::sqlite` handles' `publish` calls behind a barrier
//! never produced a lost update or a spurious double-success in 300 runs
//! even *before* this fix, meaning that specific black-box test does not
//! reproduce the theoretical race reliably at local-SQLite speed and is not
//! by itself proof `BEGIN IMMEDIATE` changed anything observable; and (2)
//! forcing real contention with an external connection holding the write
//! lock showed the plain-`BEGIN` code hits an immediate, repeating
//! `SQLITE_BUSY` at the write statement (SQLite's lock-upgrade conflict is
//! not absorbed by `busy_timeout`'s wait) that only the Rust-level retry
//! loop below resolves, whereas `BEGIN IMMEDIATE` blocks and waits
//! transparently inside `busy_timeout` itself and never reaches that retry
//! loop for the same scenario. Neither experiment reproduced a silent lost
//! update (two writers both appearing to succeed) — that remains a
//! theoretical risk read off SQLite's documented rollback-journal locking
//! model (no snapshot isolation to catch a stale read after the fact), not
//! something this session observed. See `tests/concurrency.rs`'s module doc
//! comment for the exact experiments and their results.
//!
//! `busy_timeout(5s)` (set on connect, below) makes SQLite itself block and
//! retry a blocked lock *acquisition* for up to 5 seconds at the C level
//! before returning `SQLITE_BUSY`/`SQLITE_LOCKED` — verified above to cover
//! `BEGIN IMMEDIATE` contention (the shipped configuration): the retry loop
//! below never fired in that experiment. `with_immediate_tx` also wraps a
//! bounded Rust-level retry (`MAX_TX_ATTEMPTS` attempts with exponential
//! backoff) around the whole begin/operate/commit sequence, as a backstop
//! beyond `busy_timeout`'s 5s window and for the one contention point
//! `BEGIN IMMEDIATE` does not remove: `COMMIT` itself still needs to
//! escalate `RESERVED` to `EXCLUSIVE`, which can block on a concurrent
//! `SHARED` reader — `.current()`/`.run_state()` (below) run outside any
//! transaction, on a connection that may belong to a different handle. This
//! backstop was only exercised under the reverted plain-`BEGIN`
//! configuration (where it resolved a lock-upgrade conflict `busy_timeout`
//! left unretried); as shipped, `is_retryable`/`backoff`/`MAX_TX_ATTEMPTS`
//! have no test forcing them to fire — a gap, not a claim of coverage. Only
//! `SQLITE_BUSY`/`SQLITE_LOCKED` (and their extended codes) are retried; a
//! genuine protocol conflict (`RunConflict`, `StaleRevision`, ...) is never
//! retried and is returned immediately.
//!
//! A single handle's own single-connection pool (`max_connections(1)`,
//! below) already serializes that handle's own operations, so this
//! contention is only ever between two *different* handles (two OS
//! processes, or — as in `tests/concurrency.rs` — two handles opened in one
//! process) pointed at the same on-disk database.
//!
//! **Regression versus the plain-`BEGIN` code this replaced: no
//! rollback-on-drop.** `sqlx::Transaction` (the old mechanism) rolls back
//! automatically on drop if never committed. `with_immediate_tx` manages a
//! raw `PoolConnection` with explicit `BEGIN IMMEDIATE`/`COMMIT`/`ROLLBACK`
//! statements instead (needed because `sqlx` 0.8.1's typed `Transaction`
//! API has no way to request `BEGIN IMMEDIATE`), so there is no automatic
//! cleanup if the future driving one of `claim_run`/`record_append`/
//! `publish` is dropped mid-operation (e.g. a `tokio::select!` cancellation
//! or an external timeout) rather than run to completion or its error
//! propagated. Because this handle's pool has exactly one connection, a
//! connection returned to the pool still mid-transaction poisons every
//! later call on this handle with "cannot start a transaction within a
//! transaction." Nothing in this codebase cancels these futures today — the
//! CLI drives them sequentially to completion and every test `.await`s them
//! — so this is a latent risk, not an active bug; flagged here so a future
//! caller that adds cancellation (a timeout wrapper, a `select!`) knows to
//! check for it.

use std::path::Path;
use std::time::Duration;

use cubism_core::temporal::{WindowId, WindowRevision};
use futures::future::BoxFuture;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqlitePool, SqlitePoolOptions, SqliteRow};

use crate::control::{ClaimResult, Publication, RunState};
use crate::error::{CubismIcebergError, Result};

/// Bounded retry count for a `BEGIN IMMEDIATE` transaction that hits
/// `SQLITE_BUSY`/`SQLITE_LOCKED`. Each attempt already waits up to the
/// connection's `busy_timeout` (5s) inside SQLite itself, so this is a
/// small number of *additional* whole-transaction retries, not a tight
/// spin loop.
const MAX_TX_ATTEMPTS: u32 = 8;

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
        let cube_id = cube_id.to_string();
        let window_id = window_id.as_str().to_string();
        let run_id = run_id.to_string();

        self.with_immediate_tx(move |conn| {
            let cube_id = cube_id.clone();
            let window_id = window_id.clone();
            let run_id = run_id.clone();
            Box::pin(async move {
                let inserted = sqlx::query(
                    "INSERT INTO control_runs (run_id, cube_id, window_id, revision, expected_rows, status, aggregate_snapshot_id)
                     VALUES (?, ?, ?, ?, ?, 'claimed', NULL)
                     ON CONFLICT(run_id) DO NOTHING",
                )
                .bind(&run_id)
                .bind(&cube_id)
                .bind(&window_id)
                .bind(revision.get() as i64)
                .bind(expected_rows as i64)
                .execute(&mut *conn)
                .await?
                .rows_affected()
                    == 1;

                let row = fetch_run_row(conn, &run_id)
                    .await?
                    .expect("row just inserted or already present");
                let state = row_to_run_state(&row)?;

                if inserted {
                    return Ok(ClaimResult::New(state));
                }

                let window_key = state_window_key(&state);
                if window_key.0 != cube_id
                    || window_key.1 != window_id
                    || state_revision(&state) != revision
                    || state_expected_rows(&state) != expected_rows
                {
                    return Err(CubismIcebergError::RunConflict {
                        run_id,
                        window_id,
                        revision: revision.get(),
                        expected_rows,
                    });
                }
                Ok(ClaimResult::Existing(state))
            })
        })
        .await
    }

    pub async fn record_append(&self, run_id: &str, aggregate_snapshot_id: i64) -> Result<RunState> {
        let run_id = run_id.to_string();

        self.with_immediate_tx(move |conn| {
            let run_id = run_id.clone();
            Box::pin(async move {
                let updated = sqlx::query(
                    "UPDATE control_runs SET status = 'appended', aggregate_snapshot_id = ?
                     WHERE run_id = ? AND status = 'claimed'",
                )
                .bind(aggregate_snapshot_id)
                .bind(&run_id)
                .execute(&mut *conn)
                .await?
                .rows_affected()
                    == 1;

                let row = fetch_run_row(conn, &run_id)
                    .await?
                    .ok_or_else(|| CubismIcebergError::UnknownRun(run_id.clone()))?;
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
                        run_id,
                        window_id: state_window_key(other).1,
                        revision: state_revision(other).get(),
                        expected_rows: state_expected_rows(other),
                    }),
                }
            })
        })
        .await
    }

    pub async fn publish(&self, run_id: &str, expected_current: Option<WindowRevision>) -> Result<Publication> {
        let run_id = run_id.to_string();

        self.with_immediate_tx(move |conn| {
            let run_id = run_id.clone();
            Box::pin(async move {
                let run_row = fetch_run_row(conn, &run_id)
                    .await?
                    .ok_or_else(|| CubismIcebergError::UnknownRun(run_id.clone()))?;
                let run_state = row_to_run_state(&run_row)?;

                let (cube_id, window_id, revision, aggregate_snapshot_id) = match run_state {
                    RunState::Appended { window_key, revision, aggregate_snapshot_id, .. }
                    | RunState::Published { window_key, revision, aggregate_snapshot_id, .. } => {
                        (window_key.0, window_key.1, revision, aggregate_snapshot_id)
                    }
                    RunState::Claimed { .. } => {
                        return Err(CubismIcebergError::RunNotAppended { run_id });
                    }
                };

                let pub_row = sqlx::query(
                    "SELECT revision, run_id, aggregate_snapshot_id FROM control_publications
                     WHERE cube_id = ? AND window_id = ?",
                )
                .bind(&cube_id)
                .bind(&window_id)
                .fetch_optional(&mut *conn)
                .await?;

                let actual_revision = pub_row
                    .as_ref()
                    .map(|row| -> Result<WindowRevision> {
                        Ok(WindowRevision::new(row.try_get::<i64, _>("revision")? as u64)?)
                    })
                    .transpose()?;

                if actual_revision == Some(revision) {
                    let row = pub_row.expect("publication just observed");
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
                .bind(&run_id)
                .bind(aggregate_snapshot_id)
                .execute(&mut *conn)
                .await?;

                sqlx::query("UPDATE control_runs SET status = 'published' WHERE run_id = ?")
                    .bind(&run_id)
                    .execute(&mut *conn)
                    .await?;

                Ok(Publication { revision, run_id, aggregate_snapshot_id })
            })
        })
        .await
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

    /// Run `op` inside a `BEGIN IMMEDIATE` transaction, committing on
    /// success and rolling back on any error. A `SQLITE_BUSY`/`SQLITE_LOCKED`
    /// error anywhere in the begin/operate/commit sequence retries the whole
    /// sequence from scratch (fresh connection, fresh `BEGIN IMMEDIATE`), up
    /// to `MAX_TX_ATTEMPTS` times with exponential backoff. Any other error
    /// — including a business-logic rejection like `RunConflict` or
    /// `StaleRevision` — is returned immediately, not retried.
    async fn with_immediate_tx<F, T>(&self, mut op: F) -> Result<T>
    where
        F: for<'c> FnMut(&'c mut SqliteConnection) -> BoxFuture<'c, Result<T>>,
    {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let mut conn = self.pool.acquire().await?;

            if let Err(err) = sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await {
                let err = CubismIcebergError::from(err);
                if attempt < MAX_TX_ATTEMPTS && is_retryable(&err) {
                    drop(conn);
                    backoff(attempt).await;
                    continue;
                }
                return Err(err);
            }

            let outcome = op(&mut conn).await;

            let outcome = match outcome {
                Ok(value) => match sqlx::query("COMMIT").execute(&mut *conn).await {
                    Ok(_) => Ok(value),
                    Err(err) => {
                        let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                        Err(CubismIcebergError::from(err))
                    }
                },
                Err(err) => {
                    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                    Err(err)
                }
            };

            match outcome {
                Ok(value) => return Ok(value),
                Err(err) if attempt < MAX_TX_ATTEMPTS && is_retryable(&err) => {
                    drop(conn);
                    backoff(attempt).await;
                }
                Err(err) => return Err(err),
            }
        }
    }
}

/// True for `SQLITE_BUSY`/`SQLITE_LOCKED` and their extended result codes
/// (e.g. `SQLITE_BUSY_TIMEOUT`) — a writer that should back off and retry,
/// never a protocol-level rejection (`RunConflict`, `StaleRevision`, ...).
fn is_retryable(err: &CubismIcebergError) -> bool {
    let CubismIcebergError::Sql(sqlx::Error::Database(db_err)) = err else {
        return false;
    };
    db_err
        .code()
        .and_then(|code| code.parse::<i32>().ok())
        .map(|code| matches!(code & 0xff, 5 | 6)) // SQLITE_BUSY | SQLITE_LOCKED
        .unwrap_or(false)
}

async fn backoff(attempt: u32) {
    let millis = 5u64.saturating_mul(1u64 << attempt.min(6));
    tokio::time::sleep(Duration::from_millis(millis)).await;
}

async fn fetch_run_row(conn: &mut SqliteConnection, run_id: &str) -> Result<Option<SqliteRow>> {
    Ok(sqlx::query(
        "SELECT cube_id, window_id, revision, expected_rows, status, aggregate_snapshot_id
         FROM control_runs WHERE run_id = ?",
    )
    .bind(run_id)
    .fetch_optional(&mut *conn)
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
