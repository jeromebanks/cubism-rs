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
//! **Correction: the paragraph above understated the gap.** Filling it
//! (`tests/concurrency.rs`'s
//! `retry_loop_resolves_a_write_lock_held_past_busy_timeout`, forcing a
//! `BEGIN IMMEDIATE` past `busy_timeout`'s 5s window) found this path did
//! not merely lack a test — it did not work at all as shipped. Observed
//! directly, not read off SQLite's documentation: a `BEGIN IMMEDIATE` that
//! fails with `SQLITE_BUSY` still leaves the connection internally marked
//! as "in a transaction," even though the write lock was never acquired.
//! The retryable-BEGIN-failure branch dropped that connection back into the
//! pool without a `ROLLBACK` first; with `max_connections(1)`, the very
//! next attempt reacquired the same poisoned connection and its own `BEGIN
//! IMMEDIATE` failed immediately with an unrelated, non-retryable
//! "cannot start a transaction within a transaction" error — so any real
//! contention past 5s made `with_immediate_tx` fail permanently instead of
//! retrying. Fixed by rolling back on every exit from a failed `BEGIN`, not
//! just the ones this session's test happened to force. See the retry
//! branch below and the cited test's doc comment for the exact before/after
//! error sequence.
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
                aggregate_snapshot_id INTEGER,
                observed_generation INTEGER
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
                generation INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (cube_id, window_id)
            )",
        )
        .execute(&pool)
        .await?;

        // Issue #21's claim-time anchor needs two columns that databases
        // created by older versions of this crate lack. SQLite has no `ADD
        // COLUMN IF NOT EXISTS`, so check `pragma_table_info` first (an
        // unconditional ALTER would fail permanently on every subsequent
        // open) and alter only when missing. These run outside
        // `with_immediate_tx` as single implicit transactions. Sequential
        // reopens are idempotent (the second open sees the columns the
        // first added); two handles racing `open()` can both observe the
        // column missing, in which case the loser's ALTER fails with a
        // duplicate-column error and `open()` returns `Err` — loud, not
        // silent, and the next open succeeds. A pre-migration row's
        // NULL `observed_generation` means "claimed before this anchor
        // existed": replay guards must fall through unprotected for those
        // rows rather than refuse them (see
        // `PublicationStore::run_observed_generation`'s doc comment).
        if !sqlite_column_exists(&pool, "control_runs", "observed_generation").await? {
            sqlx::query("ALTER TABLE control_runs ADD COLUMN observed_generation INTEGER")
                .execute(&pool)
                .await?;
        }
        if !sqlite_column_exists(&pool, "control_publications", "generation").await? {
            sqlx::query("ALTER TABLE control_publications ADD COLUMN generation INTEGER NOT NULL DEFAULT 0")
                .execute(&pool)
                .await?;
        }

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
                // Issue #21's anchor: the window generation this run
                // observes at first-claim time, recorded transactionally
                // with the insert itself. Only the fresh-insert path uses
                // it (`ON CONFLICT DO NOTHING` leaves an existing row's
                // original observation untouched), and the column is never
                // updated afterward, so a replay can trust it as "what the
                // world looked like when this run was first claimed."
                let observed_generation: i64 = sqlx::query(
                    "SELECT generation FROM control_publications WHERE cube_id = ? AND window_id = ?",
                )
                .bind(&cube_id)
                .bind(&window_id)
                .fetch_optional(&mut *conn)
                .await?
                .map(|row| row.try_get::<i64, _>("generation"))
                .transpose()?
                .unwrap_or(0);

                let inserted = sqlx::query(
                    "INSERT INTO control_runs (run_id, cube_id, window_id, revision, expected_rows, status, aggregate_snapshot_id, observed_generation)
                     VALUES (?, ?, ?, ?, ?, 'claimed', NULL, ?)
                     ON CONFLICT(run_id) DO NOTHING",
                )
                .bind(&run_id)
                .bind(&cube_id)
                .bind(&window_id)
                .bind(revision.get() as i64)
                .bind(expected_rows as i64)
                .bind(observed_generation)
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
                    "SELECT revision, run_id, aggregate_snapshot_id, generation FROM control_publications
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

                // Bump only on a *changing* write (first publication,
                // later correction, or rollback republish), mirroring the
                // in-memory backend. The idempotent same-revision
                // early-return above and rejected CAS attempts must not
                // bump: counting them would let an unrelated no-op replay
                // permanently poison every concurrently-recovering run's
                // claim anchor.
                let new_generation: i64 =
                    pub_row.as_ref().map(|row| row.try_get::<i64, _>("generation")).transpose()?.unwrap_or(0) + 1;

                sqlx::query(
                    "INSERT INTO control_publications (cube_id, window_id, revision, run_id, aggregate_snapshot_id, generation)
                     VALUES (?, ?, ?, ?, ?, ?)
                     ON CONFLICT(cube_id, window_id) DO UPDATE SET
                         revision = excluded.revision,
                         run_id = excluded.run_id,
                         aggregate_snapshot_id = excluded.aggregate_snapshot_id,
                         generation = excluded.generation",
                )
                .bind(&cube_id)
                .bind(&window_id)
                .bind(revision.get() as i64)
                .bind(&run_id)
                .bind(aggregate_snapshot_id)
                .bind(new_generation)
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

    /// The window's live publication generation (0 while unpublished,
    /// bumped by every changing publish — see
    /// [`crate::control::PublicationStore::publication_generation`]).
    pub async fn publication_generation(&self, cube_id: &str, window_id: &WindowId) -> Result<u64> {
        let row = sqlx::query("SELECT generation FROM control_publications WHERE cube_id = ? AND window_id = ?")
            .bind(cube_id)
            .bind(window_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| row.try_get::<i64, _>("generation")).transpose()?.unwrap_or(0) as u64)
    }

    /// The window generation a run observed at its first-ever claim.
    /// `None` for an unknown `run_id`, or for a row whose NULL
    /// `observed_generation` marks it as claimed by a pre-migration
    /// version of this crate — callers must treat that as "cannot verify"
    /// and fall through unprotected, not as a refusal (refusing would
    /// permanently break crash recovery across an upgrade).
    pub async fn run_observed_generation(&self, run_id: &str) -> Result<Option<u64>> {
        let row =
            sqlx::query("SELECT observed_generation FROM control_runs WHERE run_id = ?").bind(run_id).fetch_optional(&self.pool).await?;
        Ok(row
            .map(|row| row.try_get::<Option<i64>, _>("observed_generation"))
            .transpose()?
            .flatten()
            .map(|generation| generation as u64))
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
                // A failed `BEGIN IMMEDIATE` (e.g. SQLITE_BUSY) still leaves
                // this connection internally marked as "in a transaction"
                // even though the write lock was never acquired -- observed
                // directly (see `tests/concurrency.rs`'s
                // retry_loop_resolves_a_write_lock_held_past_busy_timeout doc
                // comment for the exact error sequence), not read off
                // SQLite's docs. Roll back before returning the connection to
                // the pool on every exit from this branch, retryable or not
                // -- `max_connections(1)` means a poisoned connection here
                // breaks every later call on this handle, not just this one.
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
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

/// Whether `table` already has a column named `column`, per
/// `pragma_table_info` — the deterministic existence check behind
/// [`SqliteStore::open`]'s additive migration (SQLite has no `ADD COLUMN IF
/// NOT EXISTS`, and matching on ALTER's duplicate-column error text is
/// brittle across sqlx/SQLite versions). `pragma_table_info` does not take
/// bound parameters; the interpolated table name is only ever one of the
/// hardcoded literals at this file's two call sites, never user input.
async fn sqlite_column_exists(pool: &SqlitePool, table: &str, column: &str) -> Result<bool> {
    let rows = sqlx::query(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().any(|row| row.try_get::<String, _>("name").map(|name| name == column).unwrap_or(false)))
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
