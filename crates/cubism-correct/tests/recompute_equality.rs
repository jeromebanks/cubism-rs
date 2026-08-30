//! The proof [#16](https://github.com/jeromebanks/cubism-rs/issues/16) and
//! plan line 634 ask for: **a late-event rebuild equals a clean rebuild
//! from the corrected source.**
//!
//! `crates/cubism-iceberg/tests/coordinator.rs`'s module doc explains why
//! this test cannot live there — that crate links no aggregation engine, so
//! both sides of the comparison would have to be hand-built identically and
//! the equality would be tautological. It proves *revision isolation*
//! instead, and defers this property to "whichever future crate links an
//! aggregation engine and can build both sides independently". This crate
//! is that crate.
//!
//! What makes the comparison non-tautological here: neither side is
//! hand-written. Both are produced by running `build_temporal` over CSV
//! events and persisting through Iceberg. The two paths differ in
//! everything except the answer they must agree on:
//!
//! | | correction path | clean path |
//! |---|---|---|
//! | published revisions | 1 (partial), then 2 (corrected) | 1 only |
//! | source for the published revision | corrected CSV | corrected CSV |
//! | write protocol | `CorrectionCoordinator` (claim/append/CAS-publish) | raw claim/append/publish |
//! | warehouse | its own | a separate one |
//! | which windows to rebuild | derived from an event-time range | named directly |
//!
//! If the correction path ever republished a stale aggregate, merged the
//! superseded revision into the new one, or rebuilt the wrong window, the
//! two would diverge.

use std::sync::Arc;

use arrow_array::{Array, FixedSizeBinaryArray, RecordBatch, TimestampMicrosecondArray};
use cubism_core::CubeSpec;
use cubism_core::temporal::{EventTime, TimeRange, WindowId, WindowRevision};
use cubism_correct::{CorrectError, CorrectionEngine, CorrectionOutcome};
use cubism_datafusion::datafusion::prelude::{CsvReadOptions, SessionContext};
use cubism_datafusion::temporal_build::{
    NullEventTimePolicy, build_temporal, temporal_state_schema,
};
use cubism_iceberg::{
    AggregateReader, AggregateWriter, AppendWindow, CatalogConfig, ClaimResult, PublicationStore,
    TemporalTable,
};
use iceberg::Catalog;
use tempfile::TempDir;

const SPEC_YAML: &str = r#"
apiVersion: cubism/v2alpha1
name: corrections_demo
dimensions:
  - name: geo
    levels: [country]
measures:
  - name: revenue
    agg: sum
    input: revenue
  - name: hits
    agg: count
  - name: avg_revenue
    agg: avg
    input: revenue
includeGlobal: true
temporal:
  eventTime: timestamp
  baseResolution: 1d
  origin: unix
  timezone: UTC
  allowedLateness: 2h
  rollups: []
"#;

const HEADER: &str = "event_id,timestamp,country,revenue\n";

/// Events every build sees.
const BASE_ROWS: &str = "\
e1,2026-04-06T01:00:00+00:00,US,10\n\
e2,2026-04-06T02:00:00+00:00,GB,20\n\
e3,2026-04-07T01:00:00+00:00,US,30\n\
e4,2026-04-07T05:00:00+00:00,GB,40\n\
e5,2026-04-08T03:00:00+00:00,US,50\n";

/// The late arrivals, all landing inside 2026-04-07. Note they include a
/// brand-new XUnit (`DE`) as well as extra rows for existing ones, so the
/// correction has to change the window's registry, not only its numbers.
const LATE_ROWS: &str = "\
e6,2026-04-07T09:00:00+00:00,US,70\n\
e7,2026-04-07T11:00:00+00:00,DE,90\n";

fn spec() -> CubeSpec {
    CubeSpec::from_yaml(SPEC_YAML).expect("spec parses")
}

fn write_csv(dir: &TempDir, name: &str, body: &str) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, format!("{HEADER}{body}")).unwrap();
    path.to_str().unwrap().to_string()
}

fn at(text: &str) -> EventTime {
    EventTime::from_unix_micros(
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .timestamp_micros(),
    )
}

/// One warehouse + catalog + control store.
struct Warehouse {
    _dir: TempDir,
    catalog: Arc<dyn Catalog>,
    table: TemporalTable,
    publications: PublicationStore,
}

impl Warehouse {
    async fn new(spec: &CubeSpec) -> Self {
        let dir = TempDir::new().unwrap();
        let config = CatalogConfig::Memory {
            warehouse: dir.path().to_path_buf(),
        };
        let catalog = cubism_iceberg::config::open_catalog(&config).await.unwrap();
        let schema = temporal_state_schema(spec).unwrap();
        let table = TemporalTable::create(catalog.as_ref(), &spec.name, &schema)
            .await
            .unwrap();
        Self {
            _dir: dir,
            catalog,
            table,
            publications: PublicationStore::in_memory(),
        }
    }

    /// Build `window_id` from `source` and publish it as revision 1 through
    /// the raw protocol — the path every window's first build takes.
    async fn build_and_publish_initial(
        &self,
        ctx: &SessionContext,
        spec: &CubeSpec,
        source: &str,
        window_id: &WindowId,
        range: TimeRange,
        run_id: &str,
    ) {
        let built = build_temporal(
            ctx,
            spec,
            source,
            Some(range),
            Some(window_id.clone()),
            NullEventTimePolicy::Quarantine,
        )
        .await
        .unwrap();
        let states = built.states.collect().await.unwrap();
        let registry = built.registry.collect().await.unwrap();
        let expected_rows: u64 = states.iter().map(RecordBatch::num_rows).sum::<usize>() as u64;
        let revision = WindowRevision::new(1).unwrap();
        let claim = self
            .publications
            .claim_run(
                &self.table.cube_id,
                window_id,
                run_id,
                revision,
                expected_rows,
            )
            .await
            .unwrap();
        assert!(matches!(claim, ClaimResult::New(_)));
        let result = AggregateWriter::append_window(
            self.catalog.as_ref(),
            &self.table,
            AppendWindow {
                window_id,
                revision,
                run_id,
                states: &states,
                registry: &registry,
            },
        )
        .await
        .unwrap();
        self.publications
            .record_append(run_id, result.snapshot_id)
            .await
            .unwrap();
        self.publications.publish(run_id, None).await.unwrap();
    }

    async fn read(&self, window_id: &WindowId) -> Vec<RecordBatch> {
        AggregateReader::read_window(
            self.catalog.as_ref(),
            &self.table,
            &self.publications,
            window_id,
        )
        .await
        .unwrap()
    }
}

/// `(bucket_start, xunit_id, <every measure column's bytes/values>)`,
/// sorted — compares two independently produced results by domain content,
/// ignoring batch boundaries and the writer's `window_id`/`revision`/
/// `run_id` identity columns, which legitimately differ between the paths.
/// Mirrors `cubism-iceberg/tests/coordinator.rs`'s `domain_rows` helper.
fn domain_rows(batches: &[RecordBatch]) -> Vec<(i64, [u8; 32], Vec<String>)> {
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
        // Every column that is not identity or key material — i.e. the
        // measure states, including the opaque AVG blob. Rendering them
        // via the Arrow display formatter keeps this helper agnostic to
        // each measure's physical type.
        let measure_columns: Vec<_> = batch
            .schema()
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                !matches!(
                    f.name().as_str(),
                    "bucket_start" | "xunit_id" | "window_id" | "revision" | "run_id"
                )
            })
            .map(|(i, _)| i)
            .collect();
        for row in 0..batch.num_rows() {
            let mut id = [0u8; 32];
            id.copy_from_slice(xunit_id.value(row));
            let values = measure_columns
                .iter()
                .map(|&col| {
                    let array = batch.column(col);
                    if array.is_null(row) {
                        "NULL".to_string()
                    } else {
                        arrow_cast::display::array_value_to_string(array, row).unwrap()
                    }
                })
                .collect();
            rows.push((bucket_start.value(row), id, values));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    rows
}

#[tokio::test]
async fn a_late_event_rebuild_equals_a_clean_rebuild_from_the_corrected_source() {
    let spec = spec();
    let dir = TempDir::new().unwrap();
    let initial_csv = write_csv(&dir, "initial.csv", BASE_ROWS);
    let corrected_csv = write_csv(&dir, "corrected.csv", &format!("{BASE_ROWS}{LATE_ROWS}"));

    let day2 = WindowId::new("2026-04-07").unwrap();
    let day2_range = TimeRange::new(
        at("2026-04-07T00:00:00+00:00"),
        at("2026-04-08T00:00:00+00:00"),
    )
    .unwrap();

    // ---- Path A: publish the partial window, then correct it -----------
    let ctx_a = SessionContext::new();
    ctx_a
        .register_csv("initial", &initial_csv, CsvReadOptions::new())
        .await
        .unwrap();
    ctx_a
        .register_csv("corrected", &corrected_csv, CsvReadOptions::new())
        .await
        .unwrap();
    let a = Warehouse::new(&spec).await;
    a.build_and_publish_initial(&ctx_a, &spec, "initial", &day2, day2_range, "run-initial")
        .await;

    // The engine is told only WHEN the change happened, not which window
    // that is — deriving the window is half of what #16 asked for.
    let outcome: CorrectionOutcome = CorrectionEngine::correct(
        &ctx_a,
        &spec,
        "corrected",
        TimeRange::new(
            at("2026-04-07T09:00:00+00:00"),
            at("2026-04-07T11:00:01+00:00"),
        )
        .unwrap(),
        a.catalog.as_ref(),
        &a.table,
        &a.publications,
        "run-correct",
    )
    .await
    .expect("the correction runs end to end");

    assert_eq!(outcome.corrected.len(), 1, "exactly one window was touched");
    let corrected = &outcome.corrected[0];
    assert_eq!(corrected.window_id.as_str(), "2026-04-07");
    assert_eq!(corrected.previous_revision.get(), 1);
    assert_eq!(
        corrected.revision.get(),
        2,
        "the correction bumped the revision"
    );
    assert!(outcome.skipped_unpublished.is_empty());

    // ---- Path B: a clean, single-revision build from corrected source --
    let ctx_b = SessionContext::new();
    ctx_b
        .register_csv("corrected", &corrected_csv, CsvReadOptions::new())
        .await
        .unwrap();
    let b = Warehouse::new(&spec).await;
    b.build_and_publish_initial(&ctx_b, &spec, "corrected", &day2, day2_range, "run-clean")
        .await;

    // ---- The property ---------------------------------------------------
    let corrected_rows = domain_rows(&a.read(&day2).await);
    let clean_rows = domain_rows(&b.read(&day2).await);

    assert!(
        !clean_rows.is_empty(),
        "the clean build produced rows to compare"
    );
    assert_eq!(
        corrected_rows, clean_rows,
        "a late-event rebuild must equal a clean rebuild from the corrected source"
    );

    // Guard against the comparison passing vacuously: the correction must
    // actually have CHANGED something relative to the pre-correction
    // aggregate. Without this, a bug that published revision 1's content
    // unchanged as revision 2 would satisfy the equality above only if the
    // clean build were also wrong — but a bug that made BOTH paths ignore
    // the late rows would slip through. Rebuild the partial window's
    // content and assert it differs.
    let partial = build_temporal(
        &ctx_a,
        &spec,
        "initial",
        Some(day2_range),
        Some(day2.clone()),
        NullEventTimePolicy::Quarantine,
    )
    .await
    .unwrap();
    let partial_rows = domain_rows(&partial.states.collect().await.unwrap());
    assert_ne!(
        partial_rows, clean_rows,
        "the late events must actually change this window, or the equality above proves nothing"
    );
    // The new XUnit from the late rows (DE) must be present after
    // correction and absent before it.
    assert!(
        corrected_rows.len() > partial_rows.len(),
        "the correction added the late rows' new XUnit: {} rows before, {} after",
        partial_rows.len(),
        corrected_rows.len()
    );
}

#[tokio::test]
async fn a_change_spanning_days_corrects_each_published_window_and_reports_unpublished_ones() {
    let spec = spec();
    let dir = TempDir::new().unwrap();
    let corrected_csv = write_csv(&dir, "corrected.csv", &format!("{BASE_ROWS}{LATE_ROWS}"));

    let ctx = SessionContext::new();
    ctx.register_csv("corrected", &corrected_csv, CsvReadOptions::new())
        .await
        .unwrap();
    let w = Warehouse::new(&spec).await;

    // Publish only two of the three days the range below spans.
    for (day, start, end) in [
        (
            "2026-04-06",
            "2026-04-06T00:00:00+00:00",
            "2026-04-07T00:00:00+00:00",
        ),
        (
            "2026-04-08",
            "2026-04-08T00:00:00+00:00",
            "2026-04-09T00:00:00+00:00",
        ),
    ] {
        let window = WindowId::new(day).unwrap();
        w.build_and_publish_initial(
            &ctx,
            &spec,
            "corrected",
            &window,
            TimeRange::new(at(start), at(end)).unwrap(),
            &format!("run-init-{day}"),
        )
        .await;
    }

    let outcome = CorrectionEngine::correct(
        &ctx,
        &spec,
        "corrected",
        TimeRange::new(
            at("2026-04-06T12:00:00+00:00"),
            at("2026-04-08T12:00:00+00:00"),
        )
        .unwrap(),
        w.catalog.as_ref(),
        &w.table,
        &w.publications,
        "run-span",
    )
    .await
    .unwrap();

    assert_eq!(
        outcome
            .corrected
            .iter()
            .map(|c| c.window_id.as_str())
            .collect::<Vec<_>>(),
        ["2026-04-06", "2026-04-08"],
        "every published window in the range is corrected, in time order"
    );
    assert!(outcome.corrected.iter().all(|c| c.revision.get() == 2));
    // 2026-04-07 was never published: a correction revises an existing
    // window, it must not conjure one into being.
    assert_eq!(
        outcome
            .skipped_unpublished
            .iter()
            .map(WindowId::as_str)
            .collect::<Vec<_>>(),
        ["2026-04-07"]
    );
    assert!(
        w.publications
            .current(&w.table.cube_id, &WindowId::new("2026-04-07").unwrap())
            .await
            .unwrap()
            .is_none(),
        "the unpublished window stayed unpublished"
    );
}

/// The partial-progress guarantee `engine.rs`'s module doc rests on: when a
/// multi-window correction fails part-way, the windows that already landed
/// must reach the caller instead of being dropped by `?`.
///
/// Before `CorrectError::Partial` existed, `correct` accumulated its
/// outcome in a local and propagated failures with `?` — so this scenario
/// returned the bare underlying error and the caller had no way to know
/// 2026-04-06 had already been republished at revision 2. A retry would
/// have restarted rather than resumed, re-correcting a corrected window
/// and burning a revision on it.
///
/// The mid-run failure is induced honestly rather than by mocking: the
/// engine's run IDs are deterministic (`{prefix}-{window}-r{revision}`), so
/// pre-claiming the *second* window's run ID with a mismatched
/// `expected_rows` makes its `claim_run` fail with `RunConflict` while the
/// first window is untouched.
#[tokio::test]
async fn a_failure_part_way_through_reports_the_windows_that_already_landed() {
    let spec = spec();
    let dir = TempDir::new().unwrap();
    let corrected_csv = write_csv(&dir, "corrected.csv", &format!("{BASE_ROWS}{LATE_ROWS}"));

    let ctx = SessionContext::new();
    ctx.register_csv("corrected", &corrected_csv, CsvReadOptions::new())
        .await
        .unwrap();
    let w = Warehouse::new(&spec).await;

    // 2026-04-06 and 2026-04-08 published; 2026-04-07 deliberately not.
    for (day, start, end) in [
        (
            "2026-04-06",
            "2026-04-06T00:00:00+00:00",
            "2026-04-07T00:00:00+00:00",
        ),
        (
            "2026-04-08",
            "2026-04-08T00:00:00+00:00",
            "2026-04-09T00:00:00+00:00",
        ),
    ] {
        let window = WindowId::new(day).unwrap();
        w.build_and_publish_initial(
            &ctx,
            &spec,
            "corrected",
            &window,
            TimeRange::new(at(start), at(end)).unwrap(),
            &format!("run-init-{day}"),
        )
        .await;
    }

    // Poison the LAST window's claim, leaving the first correctable.
    let poisoned = WindowId::new("2026-04-08").unwrap();
    w.publications
        .claim_run(
            &w.table.cube_id,
            &poisoned,
            "run-partial-2026-04-08-r2",
            WindowRevision::new(2).unwrap(),
            999_999, // deliberately not the row count the engine will compute
        )
        .await
        .unwrap();

    let err = CorrectionEngine::correct(
        &ctx,
        &spec,
        "corrected",
        TimeRange::new(
            at("2026-04-06T12:00:00+00:00"),
            at("2026-04-08T12:00:00+00:00"),
        )
        .unwrap(),
        w.catalog.as_ref(),
        &w.table,
        &w.publications,
        "run-partial",
    )
    .await
    .expect_err("the poisoned second window must fail the run");

    let CorrectError::Partial {
        corrected,
        skipped_unpublished,
        source,
    } = err
    else {
        panic!("a mid-run failure must surface as CorrectError::Partial, got: {err:?}");
    };

    // The prefix is REPORTED...
    assert_eq!(
        corrected
            .iter()
            .map(|c| c.window_id.as_str())
            .collect::<Vec<_>>(),
        ["2026-04-06"],
        "the window corrected before the failure must travel out with the error"
    );
    assert_eq!(corrected[0].revision.get(), 2);
    assert_eq!(
        skipped_unpublished
            .iter()
            .map(WindowId::as_str)
            .collect::<Vec<_>>(),
        ["2026-04-07"],
        "never-published windows are still reported on the failure path"
    );
    assert!(
        matches!(*source, CorrectError::Persist(_)),
        "the underlying failure stays typed and reachable, got: {source:?}"
    );

    // ...and the prefix is REAL: 2026-04-06 genuinely advanced, 2026-04-08
    // did not. This is what makes resuming (rather than restarting) correct.
    assert_eq!(
        w.publications
            .current(&w.table.cube_id, &WindowId::new("2026-04-06").unwrap())
            .await
            .unwrap()
            .unwrap()
            .get(),
        2,
        "the corrected prefix really was published"
    );
    assert_eq!(
        w.publications
            .current(&w.table.cube_id, &poisoned)
            .await
            .unwrap()
            .unwrap()
            .get(),
        1,
        "the failed window was left at its original revision"
    );
}
