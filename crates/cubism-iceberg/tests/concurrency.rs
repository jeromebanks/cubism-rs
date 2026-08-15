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
//!
//! `retry_loop_resolves_a_write_lock_held_past_busy_timeout` (`#[ignore]`d,
//! run explicitly — see its own doc comment) exercises
//! [#14](https://github.com/jeromebanks/cubism-rs/issues/14): every test
//! above holds its contended lock for at most 200ms, well inside the 5s
//! `busy_timeout` both sides are configured with, so none of them ever push
//! a real `BEGIN IMMEDIATE` transaction through `with_immediate_tx`'s
//! Rust-level `is_retryable`/`backoff`/`MAX_TX_ATTEMPTS` retry loop as
//! shipped. This one holds for 7s specifically to force that — and, doing
//! so, found and fixed a real bug in that loop (see the test's own doc
//! comment and `durable_control.rs`'s module doc comment for details). Being
//! `#[ignore]`d, it does not by itself close #14's "zero *CI* coverage"
//! framing.

use std::sync::Arc;
use std::time::Instant;

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

/// Forces `with_immediate_tx`'s Rust-level retry loop
/// (`is_retryable`/`backoff`/`MAX_TX_ATTEMPTS`,
/// `crates/cubism-iceberg/src/durable_control.rs`) to actually fire — the
/// gap #14 named ("zero CI coverage under the shipped `BEGIN IMMEDIATE`
/// config"). `publish_waits_for_a_concurrently_held_write_lock_then_succeeds`
/// above only holds its lock for 200ms, well inside SQLite's 5s
/// `busy_timeout`, so that test resolves entirely inside `busy_timeout`
/// itself and never reaches the retry loop (confirmed in this file's module
/// doc comment). This test holds an external `BEGIN IMMEDIATE` for 7s —
/// comfortably past `busy_timeout`'s 5s window — so SQLite's own wait gives
/// up, surfaces `SQLITE_BUSY` to the Rust code, and only `is_retryable` /
/// `backoff` can resolve it from there (one retry: attempt 1 burns the full
/// 5s `busy_timeout`, attempt 2's fresh `BEGIN IMMEDIATE` acquires as soon as
/// the holder commits at ~7s — nowhere near `MAX_TX_ATTEMPTS`'s bound of 8).
///
/// **This test found a real bug, not just a coverage gap.** As first
/// written, it failed every run with `SqliteError { code: 1, message:
/// "cannot start a transaction within a transaction" }` on the retry
/// attempt — never a lock/timing issue. A failed `BEGIN IMMEDIATE` that
/// hits `SQLITE_BUSY` leaves the connection internally marked as "in a
/// transaction" even though the write lock was never acquired (observed
/// directly via the attempt/error sequence below, not read off SQLite's
/// docs); `with_immediate_tx`'s retryable-BEGIN-failure branch dropped that
/// connection back into the pool without a `ROLLBACK` first. With
/// `max_connections(1)`, the very next attempt reacquired that same
/// poisoned connection, and its `BEGIN IMMEDIATE` failed immediately with
/// the transaction-nesting error above — a non-retryable code, so the whole
/// operation returned `Err` permanently instead of ever reaching the
/// holder's release. Fixed by adding a `ROLLBACK` to that branch
/// (`durable_control.rs`), matching the pattern the `op`-failure branch
/// already used a few lines below it. Observed sequence, one run:
/// `attempt 1 BEGIN failed: SQLITE_BUSY (code 5) retryable=true` →
/// `attempt 2 BEGIN failed: cannot start a transaction within a transaction
/// (code 1) retryable=false`, pre-fix; `attempt 1 BEGIN failed: SQLITE_BUSY
/// retryable=true` → `attempt 2 BEGIN succeeded`, post-fix.
///
/// The bug lived in `with_immediate_tx` itself, so it was not
/// `publish`-specific — `claim_run` and `record_append` share the same
/// retry loop and were equally exposed to it. Read off the code, not
/// independently observed by a test: the same missing-`ROLLBACK` gap also
/// exists on the final-attempt `return Err(err)` path a few lines below
/// (durable_control.rs, the non-retryable/exhausted-attempts case) — a
/// handle that hits *that* path would stay poisoned for every subsequent
/// call on it, not just the one that failed. This test's fix only covers
/// the retryable-and-continuing branch; the terminal branch's equivalent
/// gap is not exercised or fixed here.
///
/// To actually discriminate "the retry loop resolved it" from "the test
/// timing let `publish` through some other way" — rather than asserting
/// only success, which a silently-broken retry loop could also produce by
/// accident — this asserts a *lower* bound on how long `publish` itself
/// blocked, timed from inside the spawned task (timing around the task
/// instead would measure this test's own 7s holder-release sleep, which
/// elapses regardless of whether `publish` succeeded or failed fast — an
/// earlier draft of this test made exactly that mistake and its bound
/// passed even against the broken pre-fix code below). If `is_retryable`
/// ever regresses to always-`false` (the exact format-drift risk #14's
/// summary names) or `MAX_TX_ATTEMPTS` drops to 1, `with_immediate_tx`
/// would return `Err` as soon as `busy_timeout` first gives up, around the
/// 5s mark — clearly separated from this test's ~7s success case by the 1s
/// margin the `> 6s` threshold below leaves on both sides.
///
/// Real-time, not `#[tokio::test(start_paused)]`: the 5s wait happens inside
/// SQLite's C busy handler on the connection's own thread, which a paused
/// Tokio clock cannot advance, while it *would* advance `backoff`'s
/// `tokio::time::sleep`, producing a misleading pass. `#[ignore]`d for the
/// same reason #14 itself suggested — keeping the default `cargo test`
/// suite fast; run explicitly with `cargo test -p cubism-iceberg --test
/// concurrency -- --ignored --exact
/// retry_loop_resolves_a_write_lock_held_past_busy_timeout`.
///
/// What this proves: `is_retryable` correctly classifies the
/// `SQLITE_BUSY`/extended-code error `sqlx` 0.8.1 surfaces today for this
/// scenario, and that one retry through `backoff` resolves a lock held past
/// `busy_timeout`. What it does not prove: the full `MAX_TX_ATTEMPTS` bound
/// or `backoff`'s exponential schedule beyond the one retry exercised here,
/// exhaustion behavior (every attempt failing), or the module doc comment's
/// separately-described COMMIT-`RESERVED`-to-`EXCLUSIVE`-escalation
/// contention path (blocked by a concurrent `.current()`/`.run_state()`
/// reader outside a transaction, not by a concurrent writer — a different
/// scenario from this test's two-writers-on-one-lock setup). And because
/// it's `#[ignore]`d, it does not by itself close #14's "zero *CI* coverage"
/// framing — CI does not pass `--ignored` — only that the mechanism works
/// when run explicitly, on demand.
#[tokio::test]
#[ignore = "real ~7s wait to push past SQLite's busy_timeout; kept out of the default fast suite per #14"]
async fn retry_loop_resolves_a_write_lock_held_past_busy_timeout() {
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
        // Time *inside* the spawned task, not around it -- the outer scope
        // also runs the 7s holder-release sleep below, so timing around the
        // task would measure that sleep instead of how long `publish`
        // itself actually blocked, making the lower-bound assertion below
        // pass even against the pre-fix broken retry loop (which failed
        // fast, at ~5s, well before the outer scope's own 7s elapses).
        tokio::spawn(async move {
            let start = Instant::now();
            let result = store.publish("run-1", None).await;
            (result, start.elapsed())
        })
    };

    // Hold well past the 5s `busy_timeout` both sides are configured with,
    // so SQLite's own wait gives up at least once and the Rust-level retry
    // loop is the only thing left that can resolve the block.
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;
    sqlx::query("COMMIT").execute(&mut holder).await.unwrap();

    let (result, publish_elapsed) = publish_task.await.unwrap();

    assert!(result.is_ok(), "publish must succeed once the concurrent holder releases the lock: {result:?}");
    assert!(
        publish_elapsed > std::time::Duration::from_secs(6),
        "publish itself took only {publish_elapsed:?} -- at or before busy_timeout's 5s give-up point -- \
         this would mean the retry loop never actually resolved the contention"
    );
}
