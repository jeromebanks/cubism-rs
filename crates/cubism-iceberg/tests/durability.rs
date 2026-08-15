//! Durability tests for the `CatalogConfig::Sqlite` catalog and the
//! `PublicationStore::Sqlite` control store added after Phase 3
//! (`docs/TIMESERIES_PHASE_4_HANDOFF.md`).
//!
//! The one thing Phase 3's own test suite (`tests/phase3.rs`) could not
//! catch — and did not, per its own handoff doc — is a durability bug: every
//! fixture there shares one `Arc<dyn Catalog>` for the whole test, so a
//! catalog whose namespace/table registry lives only in that one handle's
//! memory still passes every test. Every test in this file therefore opens
//! a **fresh handle** (a new `Arc<dyn Catalog>` / a new `PublicationStore`)
//! pointed at the same on-disk paths as a prior, now-dropped handle, which
//! is the same guarantee a second OS process opening those paths would
//! need — see `docs/TIMESERIES_PHASE_3_HANDOFF.md`'s `MemoryCatalog`
//! finding for the failure mode this is proving does not happen anymore.

use std::sync::Arc;

use arrow_array::{FixedSizeBinaryArray, Float64Array, Int64Array, RecordBatch, TimestampMicrosecondArray};
use arrow_schema::{DataType, Field, Schema as ArrowSchema, TimeUnit};
use chrono::DateTime;
use cubism_core::AggKind;
use cubism_core::temporal::{WindowId, WindowRevision};
use cubism_iceberg::{
    AggregateReader, AggregateWriter, AppendWindow, CatalogConfig, ClaimResult, CorrectionCoordinator,
    CorrectionRequest, CubismIcebergError, PublicationStore, ReconciliationRecord, TemporalTable,
};
use tempfile::TempDir;

const CUBE_ID: &str = "web_analytics";

fn micros(timestamp: &str) -> i64 {
    DateTime::parse_from_rfc3339(timestamp).unwrap().timestamp_micros()
}

fn xunit_id(tag: u8) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[0] = tag;
    bytes
}

fn sample_states_schema() -> ArrowSchema {
    ArrowSchema::new(vec![
        Field::new(
            "bucket_start",
            DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
            true,
        ),
        Field::new("xunit_id", DataType::FixedSizeBinary(32), true),
        Field::new("count_v1", DataType::Int64, false),
        Field::new("sum_v1", DataType::Float64, true),
    ])
}

fn states_batch(rows: &[(&str, [u8; 32], i64, f64)]) -> RecordBatch {
    let bucket_start = TimestampMicrosecondArray::from_iter_values(rows.iter().map(|(t, _, _, _)| micros(t)))
        .with_timezone("+00:00");
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(_, x, _, _)| *x)).unwrap();
    let count = Int64Array::from_iter_values(rows.iter().map(|(_, _, c, _)| *c));
    let sum = Float64Array::from_iter_values(rows.iter().map(|(_, _, _, s)| *s));
    RecordBatch::try_new(
        Arc::new(sample_states_schema()),
        vec![Arc::new(bucket_start), Arc::new(xunit_id), Arc::new(count), Arc::new(sum)],
    )
    .unwrap()
}

fn registry_batch(rows: &[([u8; 32], &[u8])]) -> RecordBatch {
    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new("xunit_id", DataType::FixedSizeBinary(32), false),
        Field::new("xunit_canonical", DataType::Binary, false),
    ]));
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(x, _)| *x)).unwrap();
    let canonical = arrow_array::BinaryArray::from_iter_values(rows.iter().map(|(_, c)| *c));
    RecordBatch::try_new(schema, vec![Arc::new(xunit_id), Arc::new(canonical)]).unwrap()
}

fn total_rows(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}

#[tokio::test]
async fn sqlite_catalog_tables_are_visible_from_a_freshly_opened_handle() {
    let warehouse = TempDir::new().unwrap();
    let catalog_dir = TempDir::new().unwrap();
    let catalog_db = catalog_dir.path().join("catalog.sqlite");
    let config = CatalogConfig::Sqlite {
        warehouse: warehouse.path().to_path_buf(),
        catalog_db: catalog_db.clone(),
    };

    // First handle: create the tables, then drop the handle entirely
    // (not just the variable — `Arc::strong_count` would still be 1 here,
    // so this really does release the only reference).
    {
        let catalog = cubism_iceberg::config::open_catalog(&config).await.unwrap();
        TemporalTable::create(catalog.as_ref(), CUBE_ID, &sample_states_schema()).await.unwrap();
    }

    // Second handle, same paths, no shared state with the first: this is
    // the exact scenario that broke `CatalogConfig::Memory` in Phase 3.
    let catalog_b = cubism_iceberg::config::open_catalog(&config).await.unwrap();
    let namespace = iceberg::NamespaceIdent::new(CUBE_ID.to_string());
    assert!(
        catalog_b.namespace_exists(&namespace).await.unwrap(),
        "a fresh handle must see the first handle's namespace"
    );
    let states_ident = iceberg::TableIdent::new(namespace.clone(), format!("{CUBE_ID}_states"));
    let registry_ident = iceberg::TableIdent::new(namespace, format!("{CUBE_ID}_xunit_registry"));
    assert!(catalog_b.table_exists(&states_ident).await.unwrap());
    assert!(catalog_b.table_exists(&registry_ident).await.unwrap());

    // TemporalTable::create must also be idempotent from the fresh handle
    // (its own `table_exists` checks must see the first handle's tables,
    // not attempt — and fail — to recreate them).
    TemporalTable::create(catalog_b.as_ref(), CUBE_ID, &sample_states_schema()).await.unwrap();
}

#[tokio::test]
async fn an_append_committed_by_one_handle_is_readable_after_publishing_from_a_second() {
    let warehouse = TempDir::new().unwrap();
    let catalog_dir = TempDir::new().unwrap();
    let catalog_db = catalog_dir.path().join("catalog.sqlite");
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let config = CatalogConfig::Sqlite {
        warehouse: warehouse.path().to_path_buf(),
        catalog_db: catalog_db.clone(),
    };

    let window_id = WindowId::new("2026-08-12").unwrap();
    let revision = WindowRevision::new(1).unwrap();
    let states = vec![states_batch(&[("2026-08-12T00:10:00Z", xunit_id(1), 3, 9.0)])];
    let registry = vec![registry_batch(&[(xunit_id(1), b"US/mobile")])];

    // First handle: create tables, claim, append. Deliberately does NOT
    // publish, and is dropped before publishing — the second handle picks
    // up the claim/append state and finishes the job.
    let snapshot_id = {
        let catalog = cubism_iceberg::config::open_catalog(&config).await.unwrap();
        let table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &sample_states_schema()).await.unwrap();
        let publications = PublicationStore::sqlite(&control_db).await.unwrap();
        let claim = publications
            .claim_run(CUBE_ID, &window_id, "run-1", revision, 3)
            .await
            .unwrap();
        assert!(matches!(claim, ClaimResult::New(_)));
        let result = AggregateWriter::append_window(
            catalog.as_ref(),
            &table,
            AppendWindow { window_id: &window_id, revision, run_id: "run-1", states: &states, registry: &registry },
        )
        .await
        .unwrap();
        publications.record_append("run-1", result.snapshot_id).await.unwrap();
        result.snapshot_id
    };

    // Second handle: fresh catalog Arc AND fresh PublicationStore, neither
    // sharing anything with the first. It reconciles the existing claim
    // (Existing(Appended)) instead of re-appending, then publishes.
    let catalog_b = cubism_iceberg::config::open_catalog(&config).await.unwrap();
    let table_b = TemporalTable::create(catalog_b.as_ref(), CUBE_ID, &sample_states_schema()).await.unwrap();
    let publications_b = PublicationStore::sqlite(&control_db).await.unwrap();

    let reclaim = publications_b.claim_run(CUBE_ID, &window_id, "run-1", revision, 3).await.unwrap();
    let recovered_snapshot = match &reclaim {
        ClaimResult::Existing(cubism_iceberg::RunState::Appended { aggregate_snapshot_id, .. }) => *aggregate_snapshot_id,
        other => panic!("expected Existing(Appended {{ .. }}) from a fresh handle, got {other:?}"),
    };
    assert_eq!(recovered_snapshot, snapshot_id, "the recovered snapshot id must match what the first handle committed");

    let expected_current = publications_b.current(CUBE_ID, &window_id).await.unwrap();
    assert_eq!(expected_current, None, "not published yet");
    publications_b.publish("run-1", expected_current).await.unwrap();

    let visible = AggregateReader::read_window(catalog_b.as_ref(), &table_b, &publications_b, &window_id)
        .await
        .unwrap();
    assert_eq!(total_rows(&visible), 1, "the first handle's append must be visible after the second handle publishes");
}

#[tokio::test]
async fn sqlite_publication_store_cas_survives_reopen_from_a_fresh_handle() {
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let window_id = WindowId::new("2026-08-12").unwrap();

    {
        let store = PublicationStore::sqlite(&control_db).await.unwrap();
        store.claim_run(CUBE_ID, &window_id, "run-1", WindowRevision::new(1).unwrap(), 1).await.unwrap();
        store.record_append("run-1", 101).await.unwrap();
        store.publish("run-1", None).await.unwrap();
    }

    // Fresh handle: no shared state, no shared `Mutex` — proves the CAS
    // baseline (`current`) and the CAS write itself both round-trip
    // through SQLite, not through anything held in this process's memory.
    let store_b = PublicationStore::sqlite(&control_db).await.unwrap();
    let current = store_b.current(CUBE_ID, &window_id).await.unwrap();
    assert_eq!(current, Some(WindowRevision::new(1).unwrap()), "publication from the dropped handle must be visible");

    store_b.claim_run(CUBE_ID, &window_id, "run-2", WindowRevision::new(2).unwrap(), 1).await.unwrap();
    store_b.record_append("run-2", 102).await.unwrap();

    // A real CAS: this must be rejected because the caller's belief about
    // the current revision (99) does not match what the fresh handle can
    // see (1) — not a hardcoded `None` that any first publish would accept.
    let stale = store_b.publish("run-2", Some(WindowRevision::new(99).unwrap())).await;
    assert!(matches!(stale, Err(CubismIcebergError::StaleRevision { expected: Some(99), actual: Some(1), .. })));

    store_b.publish("run-2", current).await.unwrap();
    assert_eq!(store_b.current(CUBE_ID, &window_id).await.unwrap(), Some(WindowRevision::new(2).unwrap()));

    // A third handle sees run-2's publication too, not just what handle B
    // did in its own memory.
    let store_c = PublicationStore::sqlite(&control_db).await.unwrap();
    assert_eq!(store_c.current(CUBE_ID, &window_id).await.unwrap(), Some(WindowRevision::new(2).unwrap()));
}

/// Roadmap Milestone 2 (`docs/TIMESERIES_ROADMAP.md`): a correction computed
/// against a revision that *was* current, then superseded by another writer
/// before the correction could publish, must be rejected via the durable
/// `StaleRevision` path — and the same run must then be able to refresh its
/// belief (`current`) and retry successfully. This differs from the two
/// existing durability/control-store CAS tests in what "expected" holds:
/// `sqlite_publication_store_cas_survives_reopen_from_a_fresh_handle` above
/// asserts on an expected revision (99) that was never valid, so it never
/// exercises recovery from a genuinely superseded belief; `control.rs`'s
/// `stale_publish_cannot_replace_a_newer_revision` exercises a real
/// superseded revision but only against the in-memory backend. This test is
/// the intersection: durable backend, genuinely-superseded `expected`, plus
/// the refresh-and-retry step neither prior test performs. Like every other
/// test in this file, the correction's stale belief is carried across a
/// dropped-and-reopened handle, not just held in one live store's memory —
/// the shape a coordinator resuming after a restart would actually hit. It
/// does not exercise `LatenessPolicy` or any coordinator/correction-plan
/// machinery — those don't exist yet (Milestones 3-4) — it only proves the
/// CAS primitive those milestones will build on already handles this shape.
#[tokio::test]
async fn sqlite_correction_against_a_superseded_revision_is_rejected_then_succeeds_on_retry() {
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let window_id = WindowId::new("2026-08-12").unwrap();

    {
        // run-1 publishes first, uncontested: the correction's original
        // belief. run-2 supersedes it before the correction (run-3) gets to
        // publish — e.g. a concurrent normal writer closing the same window
        // again. Both happen on a handle that is then dropped, so run-3
        // below cannot see either through anything but the database.
        let store = PublicationStore::sqlite(&control_db).await.unwrap();
        store.claim_run(CUBE_ID, &window_id, "run-1", WindowRevision::new(1).unwrap(), 1).await.unwrap();
        store.record_append("run-1", 101).await.unwrap();
        store.publish("run-1", None).await.unwrap();

        store.claim_run(CUBE_ID, &window_id, "run-2", WindowRevision::new(2).unwrap(), 1).await.unwrap();
        store.record_append("run-2", 102).await.unwrap();
        store.publish("run-2", Some(WindowRevision::new(1).unwrap())).await.unwrap();
    }

    // run-3 is the correction, on a freshly-opened handle: it was computed
    // against revision 1, which was genuinely current when it started — not
    // a guessed-wrong value like the reopen test above uses, and not a
    // belief carried over in this process's memory from the block above.
    let store_b = PublicationStore::sqlite(&control_db).await.unwrap();
    store_b.claim_run(CUBE_ID, &window_id, "run-3", WindowRevision::new(3).unwrap(), 1).await.unwrap();
    store_b.record_append("run-3", 103).await.unwrap();
    let stale = store_b.publish("run-3", Some(WindowRevision::new(1).unwrap())).await;
    assert!(matches!(stale, Err(CubismIcebergError::StaleRevision { expected: Some(1), actual: Some(2), .. })));
    // The rejected correction must not have moved `current`.
    assert_eq!(store_b.current(CUBE_ID, &window_id).await.unwrap(), Some(WindowRevision::new(2).unwrap()));

    // Refresh-and-retry: the correction re-reads `current`, retries with the
    // refreshed expected revision, and succeeds — the protocol Milestone 4's
    // coordinator will need for a real correction run.
    let refreshed = store_b.current(CUBE_ID, &window_id).await.unwrap();
    store_b.publish("run-3", refreshed).await.unwrap();
    assert_eq!(store_b.current(CUBE_ID, &window_id).await.unwrap(), Some(WindowRevision::new(3).unwrap()));

    // A third, freshly-opened handle sees the corrected revision too, not
    // just handle B's in-memory belief about its own retry.
    let store_c = PublicationStore::sqlite(&control_db).await.unwrap();
    assert_eq!(store_c.current(CUBE_ID, &window_id).await.unwrap(), Some(WindowRevision::new(3).unwrap()));
}

/// Roadmap Milestone 5 (`docs/TIMESERIES_ROADMAP.md`): `ReconciliationRecord`
/// classifies a run's recovery status from the durable `RunState` the
/// control store already tracks, and `CorrectionCoordinator::execute`
/// recovers from two of the four interruption points the roadmap's plan
/// test (line 638) asks about — proven here across real process restarts
/// (every handle below is freshly opened, matching this file's convention),
/// not just in-process retries.
///
/// Leg 1 proves recovery from `ReconciliationRecord::AwaitingAppend` in its
/// **unambiguous** form: a run claimed but never attempted an append at
/// all. This is deliberately not the ambiguous form (append committed to
/// Iceberg, but the crash landed before `record_append` persisted that
/// fact) — `coordinator.rs`'s `ReconciliationRecord` doc comment explains
/// why that form is not safely recoverable by this milestone's logic
/// (`fast_append` has no idempotency check, and `AggregateReader::read_window`
/// filters by `(window_id, revision)`, not by snapshot ID, so a second
/// append would duplicate visible rows). That gap is filed as
/// [issue #17](https://github.com/jeromebanks/cubism-rs/issues/17), not
/// silently assumed safe.
///
/// Leg 2 proves recovery from `ReconciliationRecord::Published`: replaying
/// the identical correction request after the first attempt already
/// succeeded must return the same `Publication`, not a new one, and must
/// not duplicate rows.
///
/// `ReconciliationRecord::AwaitingPublish` (append committed and recorded,
/// no publication yet) is **not** re-proven here — `tests/coordinator.rs`'s
/// `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
/// already proves it, against the in-memory backend; this test does not
/// duplicate that coverage.
#[tokio::test]
async fn sqlite_coordinator_execute_recovers_an_unattempted_claim_then_replays_a_published_run_after_reopen() {
    let warehouse = TempDir::new().unwrap();
    let catalog_dir = TempDir::new().unwrap();
    let catalog_db = catalog_dir.path().join("catalog.sqlite");
    let control_dir = TempDir::new().unwrap();
    let control_db = control_dir.path().join("control.sqlite");
    let config = CatalogConfig::Sqlite { warehouse: warehouse.path().to_path_buf(), catalog_db: catalog_db.clone() };
    let window_id = WindowId::new("2026-08-12").unwrap();

    // An initial revision 1 is published directly (raw protocol) so the
    // correction below has something to replace.
    let initial_states = vec![states_batch(&[("2026-08-12T00:10:00Z", xunit_id(1), 1, 1.0)])];
    let initial_registry = vec![registry_batch(&[(xunit_id(1), b"a")])];
    {
        let catalog = cubism_iceberg::config::open_catalog(&config).await.unwrap();
        let table = TemporalTable::create(catalog.as_ref(), CUBE_ID, &sample_states_schema()).await.unwrap();
        let publications = PublicationStore::sqlite(&control_db).await.unwrap();
        let claim = publications
            .claim_run(CUBE_ID, &window_id, "run-initial", WindowRevision::new(1).unwrap(), 1)
            .await
            .unwrap();
        assert!(matches!(claim, ClaimResult::New(_)));
        let result = AggregateWriter::append_window(
            catalog.as_ref(),
            &table,
            AppendWindow {
                window_id: &window_id,
                revision: WindowRevision::new(1).unwrap(),
                run_id: "run-initial",
                states: &initial_states,
                registry: &initial_registry,
            },
        )
        .await
        .unwrap();
        publications.record_append("run-initial", result.snapshot_id).await.unwrap();
        publications.publish("run-initial", None).await.unwrap();
    }

    // The correction (revision 2) is claimed, then the process "crashes"
    // before attempting the Iceberg append — the durable control store
    // records `RunState::Claimed` for "run-correction" and nothing more.
    let corrected_states = vec![states_batch(&[
        ("2026-08-12T00:10:00Z", xunit_id(1), 1, 1.0),
        ("2026-08-12T00:20:00Z", xunit_id(2), 2, 2.0),
    ])];
    let corrected_registry = vec![registry_batch(&[(xunit_id(1), b"a"), (xunit_id(2), b"b")])];
    let expected_rows: u64 = corrected_states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
    {
        let publications = PublicationStore::sqlite(&control_db).await.unwrap();
        let claim = publications
            .claim_run(CUBE_ID, &window_id, "run-correction", WindowRevision::new(2).unwrap(), expected_rows)
            .await
            .unwrap();
        assert!(matches!(claim, ClaimResult::New(_)));
        assert_eq!(
            ReconciliationRecord::classify(Some(claim.state())),
            ReconciliationRecord::AwaitingAppend { revision: WindowRevision::new(2).unwrap() }
        );
    }

    // Leg 1: a fresh handle (fresh catalog, fresh control store — a real
    // restart) reconciles the claimed-but-unattempted run by running
    // `execute`, which appends and publishes.
    let request = CorrectionRequest {
        window_id: &window_id,
        revision: WindowRevision::new(2).unwrap(),
        run_id: "run-correction",
        observed_current: WindowRevision::new(1).unwrap(),
        kinds: &[AggKind::Sum, AggKind::Count],
        states: &corrected_states,
        registry: &corrected_registry,
    };
    let catalog_b = cubism_iceberg::config::open_catalog(&config).await.unwrap();
    let table_b = TemporalTable::create(catalog_b.as_ref(), CUBE_ID, &sample_states_schema()).await.unwrap();
    let publications_b = PublicationStore::sqlite(&control_db).await.unwrap();
    let first_publication =
        CorrectionCoordinator::execute(catalog_b.as_ref(), &table_b, &publications_b, request).await.unwrap();
    assert_eq!(first_publication.revision, WindowRevision::new(2).unwrap());

    let read_after_recovery =
        AggregateReader::read_window(catalog_b.as_ref(), &table_b, &publications_b, &window_id).await.unwrap();
    assert_eq!(
        total_rows(&read_after_recovery),
        2,
        "recovering a claimed-but-unattempted run must append exactly once"
    );

    let run_state_after = publications_b.run_state("run-correction").await.unwrap().unwrap();
    assert_eq!(
        ReconciliationRecord::classify(Some(&run_state_after)),
        ReconciliationRecord::Published {
            revision: WindowRevision::new(2).unwrap(),
            aggregate_snapshot_id: first_publication.aggregate_snapshot_id
        }
    );

    // Leg 2: another fresh handle replays the identical correction request
    // (e.g. a supervisor that does not know the first attempt already
    // succeeded). `execute` must not append a second time and must return
    // the identical `Publication`, not a new one and not `StaleRevision`.
    let request_retry = CorrectionRequest {
        window_id: &window_id,
        revision: WindowRevision::new(2).unwrap(),
        run_id: "run-correction",
        observed_current: WindowRevision::new(1).unwrap(),
        kinds: &[AggKind::Sum, AggKind::Count],
        states: &corrected_states,
        registry: &corrected_registry,
    };
    let catalog_c = cubism_iceberg::config::open_catalog(&config).await.unwrap();
    let table_c = TemporalTable::create(catalog_c.as_ref(), CUBE_ID, &sample_states_schema()).await.unwrap();
    let publications_c = PublicationStore::sqlite(&control_db).await.unwrap();
    let replayed_publication =
        CorrectionCoordinator::execute(catalog_c.as_ref(), &table_c, &publications_c, request_retry).await.unwrap();
    assert_eq!(
        replayed_publication, first_publication,
        "replaying a completed correction after a restart must return the identical publication, not a new one"
    );

    let read_after_replay =
        AggregateReader::read_window(catalog_c.as_ref(), &table_c, &publications_c, &window_id).await.unwrap();
    assert_eq!(total_rows(&read_after_replay), 2, "replaying a completed correction must not duplicate rows");
}
