//! Phase 4's own "two same-window writers produce one published winner"
//! requirement (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` line 636), covered
//! against `PublicationStore::Sqlite` now that its transactions use `BEGIN
//! IMMEDIATE` (`crates/cubism-iceberg/src/durable_control.rs`).
//!
//! **What these tests actually prove — determined by experiment, not
//! assumed.** Every claim below was checked by temporarily reverting
//! `with_immediate_tx` to a plain `BEGIN` and rerunning against real SQLite,
//! not reasoned from documentation alone:
//!
//! - `two_same_window_writers_produce_exactly_one_published_winner` asserts
//!   the invariant `BEGIN IMMEDIATE` guarantees by construction (exactly one
//!   winner, the loser gets `StaleRevision`, never a lost update). Reverted
//!   to plain `BEGIN` and run 300 times, it produced **zero failures** — two
//!   `publish` calls behind a `tokio::sync::Barrier` complete in
//!   microseconds against local SQLite, too fast to reliably land both
//!   transactions' reads before either commits. So this test is a
//!   correctness assertion that holds today; it is not demonstrated to
//!   catch a reintroduced deferred-`BEGIN` regression.
//! - `publish_waits_for_a_concurrently_held_write_lock_then_succeeds` forces
//!   real contention (an external connection holds the write lock for
//!   200ms). This **did** distinguish the two mechanisms, just not the
//!   invariant above: reverted to plain `BEGIN` with `eprintln!`
//!   instrumentation, the write inside `publish` hit an immediate, repeating
//!   `SQLITE_BUSY` ("database is locked") — a lock-*upgrade* conflict, which
//!   `busy_timeout` does not wait out — resolved only by
//!   `with_immediate_tx`'s Rust-level retry loop (5 retries observed,
//!   matching its backoff schedule). With real `BEGIN IMMEDIATE`, the same
//!   scenario produced no retry-loop activity at all: SQLite's own
//!   `busy_timeout` blocked and waited transparently on the initial lock
//!   acquisition, and the call succeeded on its first attempt. Both
//!   mechanisms end in success once the holder releases — the difference is
//!   which layer does the waiting, not whether the call ultimately succeeds.
//!
//! Neither experiment reproduced a silent lost update (two writers each
//! appearing to succeed with different data). That remains a theoretical
//! risk read off SQLite's documented rollback-journal locking model — no
//! snapshot isolation to catch a stale read after the fact — not something
//! observed in this codebase. No test here forces the specific
//! two-deferred-reader-then-writer interleaving that risk describes; that
//! would need a test hook inside `with_immediate_tx` itself to pause
//! between read and write, which does not exist.
//!
//! `disjoint_windows_can_commit_concurrently_without_conflicting` covers the
//! next Phase 4 requirement (plan line 637). See its own doc comment for
//! why "concurrently" here means "without cross-window conflict," not
//! "in parallel at the database level" — `max_connections(1)` and `BEGIN
//! IMMEDIATE`'s file-level lock mean this store still serializes all
//! writers, disjoint windows included.

use std::sync::Arc;

use cubism_core::temporal::{WindowId, WindowRevision};
use cubism_iceberg::{ClaimResult, CubismIcebergError, PublicationStore, RunState};
use sqlx::Connection;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use tempfile::TempDir;
use tokio::sync::Barrier;

const CUBE_ID: &str = "web_analytics";

#[tokio::test]
async fn two_same_window_writers_produce_exactly_one_published_winner() {
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let window_id = WindowId::new("2026-08-12").unwrap();

    let store_a = Arc::new(PublicationStore::sqlite(&control_db).await.unwrap());
    let store_b = Arc::new(PublicationStore::sqlite(&control_db).await.unwrap());

    let claim_a = store_a
        .claim_run(CUBE_ID, &window_id, "run-a", WindowRevision::new(1).unwrap(), 1)
        .await
        .unwrap();
    assert!(matches!(claim_a, ClaimResult::New(_)));
    store_a.record_append("run-a", 101).await.unwrap();

    let claim_b = store_b
        .claim_run(CUBE_ID, &window_id, "run-b", WindowRevision::new(2).unwrap(), 1)
        .await
        .unwrap();
    assert!(matches!(claim_b, ClaimResult::New(_)));
    store_b.record_append("run-b", 102).await.unwrap();

    // Line the two `publish` calls up behind a barrier so they genuinely
    // overlap rather than one finishing before the other starts.
    let barrier = Arc::new(Barrier::new(2));

    let task_a = {
        let store_a = Arc::clone(&store_a);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store_a.publish("run-a", None).await
        })
    };
    let task_b = {
        let store_b = Arc::clone(&store_b);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store_b.publish("run-b", None).await
        })
    };

    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let result_a = result_a.unwrap();
    let result_b = result_b.unwrap();

    let winner_revision = match (&result_a, &result_b) {
        (Ok(publication), Err(CubismIcebergError::StaleRevision { expected: None, actual: Some(actual), .. })) => {
            assert_eq!(*actual, publication.revision.get());
            publication.revision
        }
        (Err(CubismIcebergError::StaleRevision { expected: None, actual: Some(actual), .. }), Ok(publication)) => {
            assert_eq!(*actual, publication.revision.get());
            publication.revision
        }
        other => panic!(
            "expected exactly one Ok(Publication) and one Err(StaleRevision) rejecting the loser, got {other:?}"
        ),
    };

    // A third, fresh handle: proves the winning publication round-tripped
    // through SQLite, not through either racing handle's own in-process
    // state.
    let store_c = PublicationStore::sqlite(&control_db).await.unwrap();
    assert_eq!(
        store_c.current(CUBE_ID, &window_id).await.unwrap(),
        Some(winner_revision),
        "a fresh handle must see exactly the winner's revision -- no lost update, no mixed state"
    );

    // The loser's own run must still be `Appended`, never silently marked
    // `Published` -- `publish` only flips that status on the branch that
    // actually wins the CAS.
    let loser_run_id = if winner_revision.get() == 1 { "run-b" } else { "run-a" };
    let loser_state = store_c.run_state(loser_run_id).await.unwrap().unwrap();
    assert!(
        matches!(loser_state, RunState::Appended { .. }),
        "loser must not be marked published: {loser_state:?}"
    );
}

/// A single run trying to publish concurrently from two different handles
/// (rather than two different runs) must not double-apply the same write --
/// `BEGIN IMMEDIATE` still serializes them, and the second observes the
/// first's publication and takes the harmless-no-op branch.
#[tokio::test]
async fn concurrent_publish_of_the_same_run_from_two_handles_is_idempotent() {
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let window_id = WindowId::new("2026-08-12").unwrap();

    let store_a = Arc::new(PublicationStore::sqlite(&control_db).await.unwrap());
    let store_b = Arc::new(PublicationStore::sqlite(&control_db).await.unwrap());

    for store in [&store_a, &store_b] {
        let claim = store
            .claim_run(CUBE_ID, &window_id, "run-1", WindowRevision::new(1).unwrap(), 1)
            .await
            .unwrap();
        assert!(matches!(claim, ClaimResult::New(_) | ClaimResult::Existing(_)));
        store.record_append("run-1", 101).await.unwrap();
    }

    let barrier = Arc::new(Barrier::new(2));
    let task_a = {
        let store_a = Arc::clone(&store_a);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store_a.publish("run-1", None).await
        })
    };
    let task_b = {
        let store_b = Arc::clone(&store_b);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store_b.publish("run-1", None).await
        })
    };

    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let publication_a = result_a.unwrap().unwrap();
    let publication_b = result_b.unwrap().unwrap();
    assert_eq!(publication_a, publication_b, "both handles must observe the same published revision, not a race");
}

/// Phase 4's "disjoint windows can commit concurrently"
/// (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` line 637, the requirement
/// right after the one the other tests in this file cover) — two different
/// windows, raced behind a barrier, must both succeed with no CAS conflict
/// between them and both end up correctly published.
///
/// **What "concurrently" means here, precisely.** `SqliteStore`'s pool is
/// `max_connections(1)` and every operation runs inside a real `BEGIN
/// IMMEDIATE`, which takes a database-file-level write lock — not a
/// per-window or per-row lock. So the two `publish` calls below do not
/// execute their transactions in parallel; SQLite still serializes them at
/// the file level, one `BEGIN IMMEDIATE`/`COMMIT` fully completing before
/// the other's begins. What this test actually demonstrates is the
/// application-level guarantee that matters for Phase 4: racing two
/// *unrelated* windows produces zero cross-window interference — neither
/// call observes the other as a `StaleRevision`/`RunConflict`, and each
/// window's published revision is exactly the one its own writer wrote, not
/// mixed up with the other window's. It is not evidence of the store
/// executing writes in parallel; the underlying `BEGIN IMMEDIATE` +
/// single-connection design (see `durable_control.rs`'s module doc comment)
/// means it cannot. That global serialization is a real scaling limit for a
/// production control store — see GitHub issue #12, which already flags
/// that `BEGIN IMMEDIATE` has no direct Postgres equivalent and a
/// Postgres/MySQL backend would need its own per-row locking strategy to
/// get genuine cross-window parallelism.
///
/// **The barrier below does no discriminating work, unlike the file's other
/// barrier-based test.** Because there is no cross-window CAS check for
/// `publish` to race against, this test would pass identically without the
/// barrier at all, or under a reverted plain-`BEGIN` — it is not a
/// regression test for `BEGIN IMMEDIATE` the way
/// `two_same_window_writers_produce_exactly_one_published_winner` is (see
/// that test's own caveat above about its 300-run, zero-failure result
/// under plain `BEGIN`). It exists to pin the cross-window-isolation
/// behavior down as an explicit assertion, not to catch a locking
/// regression.
#[tokio::test]
async fn disjoint_windows_can_commit_concurrently_without_conflicting() {
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let window_a = WindowId::new("2026-08-12").unwrap();
    let window_b = WindowId::new("2026-08-13").unwrap();

    let store_a = Arc::new(PublicationStore::sqlite(&control_db).await.unwrap());
    let store_b = Arc::new(PublicationStore::sqlite(&control_db).await.unwrap());

    let claim_a = store_a
        .claim_run(CUBE_ID, &window_a, "run-a", WindowRevision::new(1).unwrap(), 1)
        .await
        .unwrap();
    assert!(matches!(claim_a, ClaimResult::New(_)));
    store_a.record_append("run-a", 201).await.unwrap();

    let claim_b = store_b
        .claim_run(CUBE_ID, &window_b, "run-b", WindowRevision::new(1).unwrap(), 1)
        .await
        .unwrap();
    assert!(matches!(claim_b, ClaimResult::New(_)));
    store_b.record_append("run-b", 202).await.unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let task_a = {
        let store_a = Arc::clone(&store_a);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store_a.publish("run-a", None).await
        })
    };
    let task_b = {
        let store_b = Arc::clone(&store_b);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store_b.publish("run-b", None).await
        })
    };

    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let publication_a = result_a.unwrap().expect("window A's own publish must not be rejected by window B's activity");
    let publication_b = result_b.unwrap().expect("window B's own publish must not be rejected by window A's activity");
    assert_eq!(publication_a.run_id, "run-a");
    assert_eq!(publication_b.run_id, "run-b");

    let store_c = PublicationStore::sqlite(&control_db).await.unwrap();
    assert_eq!(
        store_c.current(CUBE_ID, &window_a).await.unwrap(),
        Some(publication_a.revision),
        "window A's published revision must not be clobbered by window B's concurrent write"
    );
    assert_eq!(
        store_c.current(CUBE_ID, &window_b).await.unwrap(),
        Some(publication_b.revision),
        "window B's published revision must not be clobbered by window A's concurrent write"
    );
}

/// `publish` must wait for a genuinely held write lock and then succeed,
/// not error out immediately or race past it. An external connection (not
/// going through `PublicationStore` at all) opens its own `BEGIN IMMEDIATE`
/// against the same database file and holds it for 200ms before releasing;
/// `store.publish` is only allowed to complete once that lock is gone. This
/// forces `with_immediate_tx` through its actual contended path (SQLite's
/// `busy_timeout`, and — if that alone doesn't resolve it — the Rust-level
/// retry loop in `is_retryable`/`backoff`) instead of the uncontended
/// fast path every other test in this crate exercises.
#[tokio::test]
async fn publish_waits_for_a_concurrently_held_write_lock_then_succeeds() {
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let window_id = WindowId::new("2026-08-12").unwrap();

    let store = Arc::new(PublicationStore::sqlite(&control_db).await.unwrap());
    store
        .claim_run(CUBE_ID, &window_id, "run-1", WindowRevision::new(1).unwrap(), 1)
        .await
        .unwrap();
    store.record_append("run-1", 101).await.unwrap();

    let mut holder = SqliteConnection::connect_with(
        &SqliteConnectOptions::new().filename(&control_db).busy_timeout(std::time::Duration::from_secs(5)),
    )
    .await
    .unwrap();
    sqlx::query("BEGIN IMMEDIATE").execute(&mut holder).await.unwrap();
    // Any write against the shared database file is enough to hold the
    // RESERVED lock -- this table has nothing to do with the control store's
    // own schema.
    sqlx::query("CREATE TABLE lock_probe (id INTEGER)").execute(&mut holder).await.unwrap();

    let publish_task = {
        let store = Arc::clone(&store);
        tokio::spawn(async move { store.publish("run-1", None).await })
    };

    // Give `publish` time to actually attempt (and block on) `BEGIN
    // IMMEDIATE` before releasing the holder's lock -- well under the 5s
    // `busy_timeout` both sides are configured with.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    sqlx::query("COMMIT").execute(&mut holder).await.unwrap();

    let result = publish_task.await.unwrap();
    assert!(result.is_ok(), "publish must succeed once the concurrent holder releases the lock: {result:?}");
}
