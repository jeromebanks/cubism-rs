//! `CorrectionCoordinator` integration tests (`docs/TIMESERIES_ROADMAP.md`
//! Milestone 4, narrowed — see `crates/cubism-iceberg/src/coordinator.rs`'s
//! module doc comment for what "narrowed" means and why).
//!
//! This crate never links an aggregation engine (`src/lib.rs`), so there is
//! no way to hand-write a "clean rebuild from the corrected source" and
//! independently compute an equal answer — any such comparison here would
//! be tautological (identical hand-built bytes compared to themselves).
//! What *is* testable at this layer, and what the first test below proves
//! instead, is **revision isolation**: running a correction through
//! `CorrectionCoordinator` and reading back the published result must be
//! indistinguishable from a from-scratch single-revision publish of the
//! same corrected content — no residue from the superseded revision leaks
//! into what the reader returns. This does **not** prove that the
//! corrected content is itself a correct recomputation from source events;
//! that property belongs to whichever future crate links an aggregation
//! engine and can build both sides independently.
//!
//! The isolation property itself — publishing revision 2 fully replaces
//! what a reader sees of revision 1 — is **not new**: `tests/phase3.rs`'s
//! `publishing_a_new_revision_replaces_visibility_of_the_prior_one`
//! (line 225) already proves it at the raw claim/append/publish protocol
//! level. What the first test below adds is that the property survives
//! when the corrected revision is produced *through*
//! `CorrectionCoordinator` specifically, plus a cross-fixture domain-row
//! comparison (not just a row count) against an independently built
//! from-scratch publish.

use std::sync::Arc;

use arrow_array::FixedSizeBinaryArray;
use arrow_array::{Float64Array, Int64Array, RecordBatch, TimestampMicrosecondArray};
use arrow_schema::{DataType, Field, Schema as ArrowSchema, TimeUnit};
use chrono::DateTime;
use cubism_core::AggKind;
use cubism_core::temporal::{WindowId, WindowRevision};
use cubism_iceberg::{
    AggregateReader, AggregateWriter, AppendWindow, CatalogConfig, ClaimResult,
    CorrectionCoordinator, CorrectionRequest, CubismIcebergError, PublicationStore, TemporalTable,
};
use iceberg::Catalog;
use tempfile::TempDir;

const CUBE_ID: &str = "web_analytics";

fn micros(timestamp: &str) -> i64 {
    DateTime::parse_from_rfc3339(timestamp)
        .unwrap()
        .timestamp_micros()
}

fn xunit_id(tag: u8) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[0] = tag;
    bytes
}

/// Mirrors `tests/phase3.rs`'s schema helper — kept file-local rather than
/// shared, matching that file's own precedent (it does the same relative to
/// `cubism_datafusion`, see its top-of-file doc comment).
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
    let bucket_start =
        TimestampMicrosecondArray::from_iter_values(rows.iter().map(|(t, _, _, _)| micros(t)))
            .with_timezone("+00:00");
    let xunit_id = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|(_, x, _, _)| *x)).unwrap();
    let count = Int64Array::from_iter_values(rows.iter().map(|(_, _, c, _)| *c));
    let sum = Float64Array::from_iter_values(rows.iter().map(|(_, _, _, s)| *s));
    RecordBatch::try_new(
        Arc::new(sample_states_schema()),
        vec![
            Arc::new(bucket_start),
            Arc::new(xunit_id),
            Arc::new(count),
            Arc::new(sum),
        ],
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

/// Extracts `(bucket_start_micros, xunit_id, count_v1, sum_v1)` tuples from
/// a set of read-back batches, sorted, so two independently-produced
/// batch sets can be compared by domain content while ignoring physical
/// batch boundaries and the writer's injected `window_id`/`revision`/
/// `run_id` identity columns (which legitimately differ between the two
/// paths this test compares).
fn domain_rows(batches: &[RecordBatch]) -> Vec<(i64, [u8; 32], i64, f64)> {
    let mut rows = Vec::new();
    for batch in batches {
        let bucket_start = batch
            .column_by_name("bucket_start")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap();
        let xunit_id = batch
            .column_by_name("xunit_id")
            .unwrap()
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .unwrap();
        let count = batch
            .column_by_name("count_v1")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        let sum = batch
            .column_by_name("sum_v1")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        for i in 0..batch.num_rows() {
            let mut id = [0u8; 32];
            id.copy_from_slice(xunit_id.value(i));
            rows.push((bucket_start.value(i), id, count.value(i), sum.value(i)));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    rows
}

struct Fixture {
    _warehouse: TempDir,
    catalog: Arc<dyn Catalog>,
    temporal_table: TemporalTable,
    publications: PublicationStore,
}

impl Fixture {
    async fn new() -> Self {
        let warehouse = TempDir::new().unwrap();
        let config = CatalogConfig::Memory {
            warehouse: warehouse.path().to_path_buf(),
        };
        let catalog = cubism_iceberg::config::open_catalog(&config).await.unwrap();
        let temporal_table =
            TemporalTable::create(catalog.as_ref(), CUBE_ID, &sample_states_schema())
                .await
                .unwrap();
        Self {
            _warehouse: warehouse,
            catalog,
            temporal_table,
            publications: PublicationStore::in_memory(),
        }
    }

    /// Publish `states`/`registry` as a from-scratch first revision — the
    /// raw claim/append/publish protocol, bypassing the coordinator. Every
    /// window's initial build goes through this path, never
    /// `CorrectionCoordinator` (see that module's doc comment on why
    /// `observed_current` is a required `WindowRevision`, not `Option`).
    async fn publish_initial(
        &self,
        window_id: &WindowId,
        states: &[RecordBatch],
        registry: &[RecordBatch],
    ) {
        let expected_rows: u64 = states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
        let revision = WindowRevision::new(1).unwrap();
        let claim = self
            .publications
            .claim_run(CUBE_ID, window_id, "run-initial", revision, expected_rows)
            .await
            .unwrap();
        assert!(matches!(claim, ClaimResult::New(_)));
        let result = AggregateWriter::append_window(
            self.catalog.as_ref(),
            &self.temporal_table,
            AppendWindow {
                window_id,
                revision,
                run_id: "run-initial",
                states,
                registry,
            },
        )
        .await
        .unwrap();
        self.publications
            .record_append("run-initial", result.snapshot_id)
            .await
            .unwrap();
        self.publications
            .publish("run-initial", None)
            .await
            .unwrap();
    }

    async fn read(&self, window_id: &WindowId) -> cubism_iceberg::error::Result<Vec<RecordBatch>> {
        AggregateReader::read_window(
            self.catalog.as_ref(),
            &self.temporal_table,
            &self.publications,
            window_id,
        )
        .await
    }
}

#[tokio::test]
async fn coordinator_correction_is_revision_isolated_from_a_from_scratch_publish() {
    let window_id = WindowId::new("2026-08-12").unwrap();

    let corrected_states = vec![states_batch(&[
        ("2026-08-12T00:10:00Z", xunit_id(1), 3, 9.0),
        ("2026-08-12T00:20:00Z", xunit_id(2), 5, 11.0),
    ])];
    let corrected_registry = vec![registry_batch(&[
        (xunit_id(1), b"US/mobile"),
        (xunit_id(2), b"EU/desktop"),
    ])];

    // Path A: publish a partial (pre-correction) revision, then run
    // `CorrectionCoordinator` to replace it with the full corrected data.
    let fixture_a = Fixture::new().await;
    let partial_states = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        3,
        9.0,
    )])];
    let partial_registry = vec![registry_batch(&[(xunit_id(1), b"US/mobile")])];
    fixture_a
        .publish_initial(&window_id, &partial_states, &partial_registry)
        .await;

    let request = CorrectionRequest {
        window_id: &window_id,
        revision: WindowRevision::new(2).unwrap(),
        run_id: "run-correction",
        observed_current: WindowRevision::new(1).unwrap(),
        kinds: &[AggKind::Sum, AggKind::Count],
        states: &corrected_states,
        registry: &corrected_registry,
    };
    let publication = CorrectionCoordinator::execute(
        fixture_a.catalog.as_ref(),
        &fixture_a.temporal_table,
        &fixture_a.publications,
        request,
    )
    .await
    .unwrap();
    assert_eq!(publication.revision, WindowRevision::new(2).unwrap());

    let corrected_read = fixture_a.read(&window_id).await.unwrap();
    assert_eq!(
        total_rows(&corrected_read),
        2,
        "only the corrected revision's two rows should be visible, not the superseded partial revision's row too"
    );

    // Path B: a fresh fixture publishes the exact same corrected content
    // once, from scratch, as revision 1 — no correction involved.
    let fixture_b = Fixture::new().await;
    fixture_b
        .publish_initial(&window_id, &corrected_states, &corrected_registry)
        .await;
    let clean_read = fixture_b.read(&window_id).await.unwrap();

    assert_eq!(
        domain_rows(&corrected_read),
        domain_rows(&clean_read),
        "a correction's published read must be indistinguishable from a from-scratch publish of the same content"
    );
}

#[tokio::test]
async fn coordinator_rejects_a_correction_planned_against_a_superseded_revision_and_does_not_move_current()
 {
    let window_id = WindowId::new("2026-08-12").unwrap();
    let fixture = Fixture::new().await;

    let states_v1 = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        1,
        1.0,
    )])];
    let registry_v1 = vec![registry_batch(&[(xunit_id(1), b"a")])];
    fixture
        .publish_initial(&window_id, &states_v1, &registry_v1)
        .await;

    // A second writer publishes revision 2 directly (not via the
    // coordinator) before the correction below gets a chance to run —
    // simulating a concurrent build that moved `current` out from under a
    // correction that was planned against revision 1.
    let states_v2 = vec![states_batch(&[(
        "2026-08-12T00:15:00Z",
        xunit_id(2),
        2,
        2.0,
    )])];
    let registry_v2 = vec![registry_batch(&[(xunit_id(2), b"b")])];
    let claim = fixture
        .publications
        .claim_run(
            CUBE_ID,
            &window_id,
            "run-concurrent",
            WindowRevision::new(2).unwrap(),
            1,
        )
        .await
        .unwrap();
    assert!(matches!(claim, ClaimResult::New(_)));
    let result = AggregateWriter::append_window(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        AppendWindow {
            window_id: &window_id,
            revision: WindowRevision::new(2).unwrap(),
            run_id: "run-concurrent",
            states: &states_v2,
            registry: &registry_v2,
        },
    )
    .await
    .unwrap();
    fixture
        .publications
        .record_append("run-concurrent", result.snapshot_id)
        .await
        .unwrap();
    fixture
        .publications
        .publish("run-concurrent", Some(WindowRevision::new(1).unwrap()))
        .await
        .unwrap();

    // The correction below still observed revision 1 as current (planned
    // before the concurrent write above landed) — the coordinator must
    // surface the CAS rejection as-is and must not retry it internally
    // (see `coordinator.rs`'s module doc comment).
    let corrected_states = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        9,
        9.0,
    )])];
    let corrected_registry = vec![registry_batch(&[(xunit_id(1), b"a")])];
    let request = CorrectionRequest {
        window_id: &window_id,
        revision: WindowRevision::new(3).unwrap(),
        run_id: "run-correction",
        observed_current: WindowRevision::new(1).unwrap(),
        kinds: &[AggKind::Sum],
        states: &corrected_states,
        registry: &corrected_registry,
    };
    let error = CorrectionCoordinator::execute(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        &fixture.publications,
        request,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        CubismIcebergError::StaleRevision {
            expected: Some(1),
            actual: Some(2),
            ..
        }
    ));

    assert_eq!(
        fixture
            .publications
            .current(CUBE_ID, &window_id)
            .await
            .unwrap(),
        Some(WindowRevision::new(2).unwrap()),
        "a rejected correction must not move `current`"
    );
}

/// Proves `coordinator.rs`'s append-skip branch
/// (`if matches!(claim.state(), RunState::Claimed { .. })`) actually skips
/// a redundant append rather than merely compiling: this simulates a crash
/// between append and publish by claiming/appending/recording directly
/// (bypassing the coordinator), so a subsequent `execute` with the same
/// `run_id`/`revision`/content observes `ClaimResult::Existing(RunState::
/// Appended { .. })` instead of `Claimed`. If `execute` appended a second
/// time, that would commit a second Iceberg snapshot, so comparing the
/// returned `Publication::aggregate_snapshot_id` against the snapshot ID
/// from the first append is a direct, not incidental, check that no second
/// append occurred — a duplicated-row count would show the same thing more
/// indirectly.
#[tokio::test]
async fn coordinator_skips_a_redundant_append_when_the_run_was_already_appended() {
    let window_id = WindowId::new("2026-08-12").unwrap();
    let fixture = Fixture::new().await;

    let states_v1 = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        1,
        1.0,
    )])];
    let registry_v1 = vec![registry_batch(&[(xunit_id(1), b"a")])];
    fixture
        .publish_initial(&window_id, &states_v1, &registry_v1)
        .await;

    let corrected_states = vec![states_batch(&[
        ("2026-08-12T00:10:00Z", xunit_id(1), 1, 1.0),
        ("2026-08-12T00:20:00Z", xunit_id(2), 2, 2.0),
    ])];
    let corrected_registry = vec![registry_batch(&[(xunit_id(1), b"a"), (xunit_id(2), b"b")])];
    let expected_rows: u64 = corrected_states
        .iter()
        .map(RecordBatch::num_rows)
        .sum::<usize>() as u64;
    let revision = WindowRevision::new(2).unwrap();

    let claim = fixture
        .publications
        .claim_run(
            CUBE_ID,
            &window_id,
            "run-correction",
            revision,
            expected_rows,
        )
        .await
        .unwrap();
    assert!(matches!(claim, ClaimResult::New(_)));
    let first_append = AggregateWriter::append_window(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        AppendWindow {
            window_id: &window_id,
            revision,
            run_id: "run-correction",
            states: &corrected_states,
            registry: &corrected_registry,
        },
    )
    .await
    .unwrap();
    fixture
        .publications
        .record_append("run-correction", first_append.snapshot_id)
        .await
        .unwrap();

    let request = CorrectionRequest {
        window_id: &window_id,
        revision,
        run_id: "run-correction",
        observed_current: WindowRevision::new(1).unwrap(),
        kinds: &[AggKind::Sum, AggKind::Count],
        states: &corrected_states,
        registry: &corrected_registry,
    };
    let publication = CorrectionCoordinator::execute(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        &fixture.publications,
        request,
    )
    .await
    .unwrap();
    assert_eq!(
        publication.aggregate_snapshot_id, first_append.snapshot_id,
        "a retry through the coordinator must not perform a second append \
         (a second append would commit a different snapshot)"
    );

    let read = fixture.read(&window_id).await.unwrap();
    assert_eq!(
        total_rows(&read),
        2,
        "a retried append-skip path must not duplicate rows"
    );
}

/// Proves the benign side of `coordinator.rs`'s #20 fix: replaying
/// `execute` for a run that is already `Published`, with nothing else
/// having changed `current` in the meantime, must still return the
/// original `Publication` — same `revision` *and* `aggregate_snapshot_id`
/// — not an error. This is Milestone 5's own idempotent-replay contract
/// (`ReconciliationRecord::Published`'s doc comment), which the #20 guard
/// must preserve exactly. This specific benign shape is not entirely new
/// coverage — `tests/durability.rs`'s
/// `sqlite_coordinator_execute_recovers_an_unattempted_claim_then_replays_a_published_run_after_reopen`'s
/// "Leg 2" already replays `execute` against an already-`Published` run on
/// the SQLite backend across a restart — but this is the first coverage of
/// it in this file, against the in-memory backend, with no restart
/// involved (isolating the #20 guard's own logic from durability/restart
/// concerns, which is that other test's job).
#[tokio::test]
async fn coordinator_replaying_a_published_correction_with_nothing_changed_returns_the_same_publication()
 {
    let window_id = WindowId::new("2026-08-12").unwrap();
    let fixture = Fixture::new().await;

    let states_v1 = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        1,
        1.0,
    )])];
    let registry_v1 = vec![registry_batch(&[(xunit_id(1), b"a")])];
    fixture
        .publish_initial(&window_id, &states_v1, &registry_v1)
        .await;

    let corrected_states = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        9,
        9.0,
    )])];
    let corrected_registry = vec![registry_batch(&[(xunit_id(1), b"a")])];
    let build_request = || CorrectionRequest {
        window_id: &window_id,
        revision: WindowRevision::new(2).unwrap(),
        run_id: "run-correction",
        observed_current: WindowRevision::new(1).unwrap(),
        kinds: &[AggKind::Sum],
        states: &corrected_states,
        registry: &corrected_registry,
    };

    let first = CorrectionCoordinator::execute(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        &fixture.publications,
        build_request(),
    )
    .await
    .unwrap();
    assert_eq!(first.revision, WindowRevision::new(2).unwrap());

    // Replay with the identical request. `run-correction`'s `RunState` is
    // already `Published`, and `current` still equals its own revision —
    // no rollback, no later correction — so the #20 live-current guard
    // must let this through exactly as before the fix.
    let replay = CorrectionCoordinator::execute(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        &fixture.publications,
        build_request(),
    )
    .await
    .unwrap();
    assert_eq!(replay.revision, first.revision);
    assert_eq!(
        replay.aggregate_snapshot_id, first.aggregate_snapshot_id,
        "an identical replay of an already-published correction must return the same publication, \
         not perform a second append or CAS"
    );
}

/// Reproduces issue #20's exact bug shape and proves the fix: a delayed
/// replay of a correction request must not silently undo an intentional
/// rollback that happened in between.
///
/// Sequence (matching the issue's own numbered steps): window `W` starts
/// at revision A (`run-initial`). A correction publishes revision B
/// (`run-correction`), moving `current` to B. An operator rolls back by
/// republishing `run-initial` with `expected_current: Some(B)`, moving
/// `current` back to A — `run-correction`'s own `RunState` is untouched by
/// this, still `Published`. The *original* correction request for
/// `run-correction` (still carrying `observed_current: A` from when it was
/// first submitted) is then replayed through `execute` — before the #20
/// fix, `current == A == observed_current` would pass the CAS a second
/// time and silently republish B, reversing the rollback with no error.
///
/// This does **not** exercise the `AwaitingPublish` variant of the same
/// shape (a run that crashed before its own publish) — that gap is
/// deliberately out of scope for this fix, tracked as #21.
#[tokio::test]
async fn coordinator_rejects_a_replayed_correction_after_a_rollback_restored_its_observed_current()
{
    let window_id = WindowId::new("2026-08-12").unwrap();
    let fixture = Fixture::new().await;

    let states_v1 = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        1,
        1.0,
    )])];
    let registry_v1 = vec![registry_batch(&[(xunit_id(1), b"a")])];
    fixture
        .publish_initial(&window_id, &states_v1, &registry_v1)
        .await;
    assert_eq!(
        fixture
            .publications
            .current(CUBE_ID, &window_id)
            .await
            .unwrap(),
        Some(WindowRevision::new(1).unwrap())
    );

    // The correction: revision B, moving `current` from A (1) to B (2).
    let corrected_states = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        9,
        9.0,
    )])];
    let corrected_registry = vec![registry_batch(&[(xunit_id(1), b"a")])];
    let original_request = || CorrectionRequest {
        window_id: &window_id,
        revision: WindowRevision::new(2).unwrap(),
        run_id: "run-correction",
        observed_current: WindowRevision::new(1).unwrap(),
        kinds: &[AggKind::Sum],
        states: &corrected_states,
        registry: &corrected_registry,
    };
    let published = CorrectionCoordinator::execute(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        &fixture.publications,
        original_request(),
    )
    .await
    .unwrap();
    assert_eq!(published.revision, WindowRevision::new(2).unwrap());
    assert_eq!(
        fixture
            .publications
            .current(CUBE_ID, &window_id)
            .await
            .unwrap(),
        Some(WindowRevision::new(2).unwrap())
    );

    // The rollback: republish `run-initial` (revision A) via the same CAS
    // `publish` call, expecting the current B — the existing, already-
    // shipped rollback mechanism (`docs/TIMESERIES_PHASE_13_HANDOFF.md`),
    // not new code under test here.
    fixture
        .publications
        .publish("run-initial", Some(WindowRevision::new(2).unwrap()))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .publications
            .current(CUBE_ID, &window_id)
            .await
            .unwrap(),
        Some(WindowRevision::new(1).unwrap()),
        "rollback should have restored current to revision A"
    );

    // The delayed replay: same run_id, same revision, same
    // `observed_current: A` as the original submission above — `current`
    // is A again too, but only because of the rollback, not because
    // nothing happened. Before the #20 fix this would pass the CAS check
    // a second time and silently republish B.
    let error = CorrectionCoordinator::execute(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        &fixture.publications,
        original_request(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        CubismIcebergError::RunNoLongerCurrent {
            revision: 2,
            current: Some(1),
            ..
        }
    ));

    assert_eq!(
        fixture
            .publications
            .current(CUBE_ID, &window_id)
            .await
            .unwrap(),
        Some(WindowRevision::new(1).unwrap()),
        "a rejected replay must not undo the rollback — current must stay at revision A"
    );
}

/// Reproduces issue #21's exact bug shape and proves the fix: a delayed
/// replay of a correction request whose run crashed *before its own
/// publish* (`AwaitingPublish`) must not silently override an intentional
/// rollback that happened in between.
///
/// Sequence (matching the issue's own numbered steps): window `W` starts at
/// revision A (`run-initial`). Correction `run-correction` is claimed,
/// appended, and recorded — then "crashes" before its publish (staged by
/// direct claim/append/record calls, bypassing `execute`, exactly as
/// `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
/// stages this state). A second, real correction publishes revision C;
/// an operator then rolls back to A — intentionally, choosing A over C,
/// knowing nothing about the crashed run. Replaying `run-correction`'s
/// original request would pass a revision-value CAS (`current == A ==
/// observed_current`), which is precisely why no value comparison can
/// close this hole: the guard compares publication *generations* instead —
/// generation 1 at claim vs 3 after the correction-plus-rollback cycle —
/// and refuses with `WindowChangedSinceClaim`.
///
/// This does **not** prove: anything about the SQLite backend or a restart
/// (in-memory fixture, no reopen — the durability suite's existing legs are
/// the regression net for the benign SQLite paths, and a dedicated SQLite
/// refusal leg is deliberately deferred); the `AwaitingAppend` variant of
/// the same guard (crash before the append itself, not covered by any test
/// in this file); or the two residual gaps documented in `execute`'s
/// implementation comment (pre-claim staleness; the concurrent
/// check-vs-publish race).
#[tokio::test]
async fn coordinator_rejects_a_replayed_awaiting_publish_correction_after_a_rollback_restored_its_observed_current()
 {
    let window_id = WindowId::new("2026-08-12").unwrap();
    let fixture = Fixture::new().await;

    let states_v1 = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        1,
        1.0,
    )])];
    let registry_v1 = vec![registry_batch(&[(xunit_id(1), b"a")])];
    fixture
        .publish_initial(&window_id, &states_v1, &registry_v1)
        .await;
    assert_eq!(
        fixture
            .publications
            .current(CUBE_ID, &window_id)
            .await
            .unwrap(),
        Some(WindowRevision::new(1).unwrap())
    );

    // The crashed correction: claimed + appended + recorded, never
    // published. Its request still carries `observed_current: A`.
    let corrected_states = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        9,
        9.0,
    )])];
    let corrected_registry = vec![registry_batch(&[(xunit_id(1), b"a")])];
    let expected_rows: u64 = corrected_states
        .iter()
        .map(RecordBatch::num_rows)
        .sum::<usize>() as u64;
    let crashed_request = || CorrectionRequest {
        window_id: &window_id,
        revision: WindowRevision::new(2).unwrap(),
        run_id: "run-correction",
        observed_current: WindowRevision::new(1).unwrap(),
        kinds: &[AggKind::Sum],
        states: &corrected_states,
        registry: &corrected_registry,
    };
    let claim = fixture
        .publications
        .claim_run(
            CUBE_ID,
            &window_id,
            "run-correction",
            WindowRevision::new(2).unwrap(),
            expected_rows,
        )
        .await
        .unwrap();
    assert!(matches!(claim, ClaimResult::New(_)));
    let crashed_append = AggregateWriter::append_window(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        AppendWindow {
            window_id: &window_id,
            revision: WindowRevision::new(2).unwrap(),
            run_id: "run-correction",
            states: &corrected_states,
            registry: &corrected_registry,
        },
    )
    .await
    .unwrap();
    fixture
        .publications
        .record_append("run-correction", crashed_append.snapshot_id)
        .await
        .unwrap();

    // A different, real correction completes normally: revision C (3).
    let later_states = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        5,
        5.0,
    )])];
    let later_registry = vec![registry_batch(&[(xunit_id(1), b"a")])];
    let published = CorrectionCoordinator::execute(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        &fixture.publications,
        CorrectionRequest {
            window_id: &window_id,
            revision: WindowRevision::new(3).unwrap(),
            run_id: "run-second-correction",
            observed_current: WindowRevision::new(1).unwrap(),
            kinds: &[AggKind::Sum],
            states: &later_states,
            registry: &later_registry,
        },
    )
    .await
    .unwrap();
    assert_eq!(published.revision, WindowRevision::new(3).unwrap());

    // The operator's deliberate rollback to A — undoing run-second-
    // correction, with no knowledge of the crashed run.
    fixture
        .publications
        .publish("run-initial", Some(WindowRevision::new(3).unwrap()))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .publications
            .current(CUBE_ID, &window_id)
            .await
            .unwrap(),
        Some(WindowRevision::new(1).unwrap()),
        "rollback should have restored current to revision A"
    );

    // The stale replay of the crashed run's original request. Before the
    // #21 fix this fell through the AwaitingPublish path straight to the
    // CAS publish, which passed (`current == observed_current == A`) and
    // silently published revision B over the operator's choice of A.
    let error = CorrectionCoordinator::execute(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        &fixture.publications,
        crashed_request(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error,
            CubismIcebergError::WindowChangedSinceClaim {
                observed_generation: 1,
                current_generation: 3,
                current: Some(1),
                ..
            }
        ),
        "expected WindowChangedSinceClaim with the claim-time generation (1) vs the live one after \
         correction-plus-rollback (3) and the rolled-back current revision"
    );

    assert_eq!(
        fixture
            .publications
            .current(CUBE_ID, &window_id)
            .await
            .unwrap(),
        Some(WindowRevision::new(1).unwrap()),
        "a rejected replay must not undo the rollback — current must stay at revision A"
    );
}

/// Proves the benign side of the #21 fix: an interrupted correction
/// (`AwaitingPublish`) replayed through `execute` while *nothing* has
/// changed since its claim still completes its publish and returns the
/// correct `Publication`. This is the crash-recovery contract the
/// generation guard must preserve — refusing here would turn every crash
/// retry into permanent stuck state. As in
/// `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`,
/// comparing against the staged append's snapshot ID is a direct check that
/// no second append occurred.
///
/// This does **not** prove recovery across a process restart (no reopen
/// here; `tests/durability.rs` covers restart-shaped recovery for the
/// store protocol) or the SQLite backend specifically.
#[tokio::test]
async fn coordinator_completes_an_interrupted_correction_when_nothing_changed_since_claim() {
    let window_id = WindowId::new("2026-08-12").unwrap();
    let fixture = Fixture::new().await;

    let states_v1 = vec![states_batch(&[(
        "2026-08-12T00:10:00Z",
        xunit_id(1),
        1,
        1.0,
    )])];
    let registry_v1 = vec![registry_batch(&[(xunit_id(1), b"a")])];
    fixture
        .publish_initial(&window_id, &states_v1, &registry_v1)
        .await;

    // Stage the interrupted run: claimed + appended + recorded, never
    // published, nothing else touching the window afterward.
    let corrected_states = vec![states_batch(&[
        ("2026-08-12T00:10:00Z", xunit_id(1), 9, 9.0),
        ("2026-08-12T00:20:00Z", xunit_id(2), 2, 2.0),
    ])];
    let corrected_registry = vec![registry_batch(&[(xunit_id(1), b"a"), (xunit_id(2), b"b")])];
    let expected_rows: u64 = corrected_states
        .iter()
        .map(RecordBatch::num_rows)
        .sum::<usize>() as u64;
    let claim = fixture
        .publications
        .claim_run(
            CUBE_ID,
            &window_id,
            "run-correction",
            WindowRevision::new(2).unwrap(),
            expected_rows,
        )
        .await
        .unwrap();
    assert!(matches!(claim, ClaimResult::New(_)));
    let staged_append = AggregateWriter::append_window(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        AppendWindow {
            window_id: &window_id,
            revision: WindowRevision::new(2).unwrap(),
            run_id: "run-correction",
            states: &corrected_states,
            registry: &corrected_registry,
        },
    )
    .await
    .unwrap();
    fixture
        .publications
        .record_append("run-correction", staged_append.snapshot_id)
        .await
        .unwrap();

    let request = CorrectionRequest {
        window_id: &window_id,
        revision: WindowRevision::new(2).unwrap(),
        run_id: "run-correction",
        observed_current: WindowRevision::new(1).unwrap(),
        kinds: &[AggKind::Sum, AggKind::Count],
        states: &corrected_states,
        registry: &corrected_registry,
    };
    let publication = CorrectionCoordinator::execute(
        fixture.catalog.as_ref(),
        &fixture.temporal_table,
        &fixture.publications,
        request,
    )
    .await
    .unwrap();
    assert_eq!(publication.revision, WindowRevision::new(2).unwrap());
    assert_eq!(
        publication.aggregate_snapshot_id, staged_append.snapshot_id,
        "recovery must complete the interrupted run's own publish, not perform a second append"
    );
    assert_eq!(
        fixture
            .publications
            .current(CUBE_ID, &window_id)
            .await
            .unwrap(),
        Some(WindowRevision::new(2).unwrap())
    );
}
