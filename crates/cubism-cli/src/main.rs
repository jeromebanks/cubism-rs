//! `cubism` CLI.
//!
//! ```text
//! cubism validate <spec.yaml>
//! cubism run <spec.yaml> --input <events.parquet|csv> [--output cube.parquet] [--show N]
//! cubism temporal-build <spec.yaml> --input <events.parquet|csv>
//!     [--window-start <RFC3339>] [--window-end <RFC3339>]
//!     [--states-output <states.parquet> --registry-output <registry.parquet>]
//!     [--null-policy reject|quarantine] [--explain]
//! cubism iceberg-build <spec.yaml> --states-input <states.parquet>
//!     --registry-input <registry.parquet> --window-id <id> --revision <n>
//!     --run-id <id> --warehouse <path>
//!     [--catalog-db <path> --control-db <path>]
//! cubism serve <cube.parquet> [--port 8080]
//!     [--spec <spec.yaml> --warehouse <path> [--catalog-db <path> --control-db <path>]]
//! ```
//!
//! `serve`'s `--spec`/`--warehouse` pair is optional and, when given,
//! additionally serves `/api/series` against that cube's Iceberg-backed
//! temporal tables (`docs/TIMESERIES_ROADMAP.md`'s Milestone 11) alongside
//! the static-cube routes `<cube.parquet>` already provides. Same
//! `--catalog-db`/`--control-db` durability switch as `iceberg-build`.
//!
//! `iceberg-build` creates the tables (if needed), claims, appends,
//! publishes, and reads the published rows back — all in one process. This
//! is one command, not separate `init`/`append`/`publish`/`verify` steps —
//! that split is still a follow-on, even now that a durable backend exists
//! (see `docs/TIMESERIES_PHASE_4_HANDOFF.md`).
//!
//! With `--catalog-db`/`--control-db` omitted, `iceberg-build` uses
//! `CatalogConfig::Memory` and an in-process `PublicationStore`, neither of
//! which survive past this process — a second invocation against the same
//! `--warehouse` path gets a genuinely empty catalog
//! (`iceberg::catalog::memory::NamespaceState`, backed by a `Mutex`, never
//! scanned from disk at open). Passing both flags switches to
//! `CatalogConfig::Sqlite` and `PublicationStore::Sqlite`: a second
//! invocation with the same three paths (`--warehouse`, `--catalog-db`,
//! `--control-db`) observes the first's tables and publications, and a
//! retried `run_id` reconciles against its own recorded claim/append state
//! instead of re-running the whole append. See
//! `docs/TIMESERIES_PHASE_3_HANDOFF.md` for how the non-durable default was
//! discovered to be a hard limitation, not just a production gap.

use cubism_core::{CubeSpec, EventTime, TimeRange};
use cubism_datafusion::build_cube;
use cubism_datafusion::datafusion::dataframe::DataFrameWriteOptions;
use cubism_datafusion::datafusion::prelude::{CsvReadOptions, ParquetReadOptions, SessionContext};
use cubism_datafusion::temporal_build::{
    explain_temporal_build, temporal_state_schema, write_temporal_fixtures, NullEventTimePolicy,
};
use cubism_core::temporal::{WindowId, WindowRevision};
use cubism_iceberg::{AggregateReader, AggregateWriter, AppendWindow, CatalogConfig, PublicationStore, TemporalTable};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage:\n  cubism validate <spec.yaml>\n  cubism run <spec.yaml> --input <events.parquet|csv> [--output cube.parquet] [--show N]\n  cubism temporal-build <spec.yaml> --input <events.parquet|csv> [--window-start <RFC3339>] [--window-end <RFC3339>] [--states-output <path> --registry-output <path>] [--null-policy reject|quarantine] [--explain]\n  cubism iceberg-build <spec.yaml> --states-input <path> --registry-input <path> --window-id <id> --revision <n> --run-id <id> --warehouse <path> [--catalog-db <path> --control-db <path>]\n  cubism serve <cube.parquet> [--port 8080] [--spec <spec.yaml> --warehouse <path> [--catalog-db <path> --control-db <path>]]";

fn load_spec(path: &str) -> Result<CubeSpec, String> {
    let yaml = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    CubeSpec::from_yaml(&yaml).map_err(|e| e.to_string())
}

fn validate(path: &str) -> Result<(), String> {
    let spec = load_spec(path)?;
    println!(
        "OK: cube '{}' — {} dimension(s), {} measure(s), {} filter rule(s)",
        spec.name,
        spec.dimensions.len(),
        spec.measures.len(),
        spec.filter_rules.len()
    );
    Ok(())
}

async fn run(spec_path: &str, input: &str, output: Option<&str>, show: usize) -> Result<(), String> {
    let spec = load_spec(spec_path)?;
    let ctx = SessionContext::new();

    let started = Instant::now();
    if input.ends_with(".csv") {
        ctx.register_csv("events", input, CsvReadOptions::new()).await
    } else {
        ctx.register_parquet("events", input, ParquetReadOptions::default()).await
    }
    .map_err(|e| format!("cannot read {input}: {e}"))?;

    let (df, dict) = build_cube(&ctx, &spec, "events").await.map_err(|e| e.to_string())?;

    let cube = df.cache().await.map_err(|e| e.to_string())?;
    let cells = cube.clone().count().await.map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();

    println!(
        "cube '{}': {cells} cells in {elapsed:.2?} ({} dictionary entries)",
        spec.name,
        dict.lock().unwrap().len()
    );

    if show > 0 {
        // Sketch blob columns are binary noise on a terminal — display
        // everything else.
        let display_cols: Vec<&str> = cube
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .filter(|n| !n.ends_with("__sketch"))
            .collect();
        cube.clone()
            .select_columns(&display_cols)
            .and_then(|d| {
                d.sort(vec![cubism_datafusion::datafusion::prelude::col(&spec.measures[0].name)
                    .sort(false, false)])
            })
            .and_then(|d| d.limit(0, Some(show)))
            .map_err(|e| e.to_string())?
            .show()
            .await
            .map_err(|e| e.to_string())?;
    }

    if let Some(out) = output {
        cube.write_parquet(out, DataFrameWriteOptions::new(), None)
            .await
            .map_err(|e| e.to_string())?;
        println!("wrote {out}");
    }
    Ok(())
}

fn parse_rfc3339(value: &str) -> Result<EventTime, String> {
    let dt = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|e| format!("invalid RFC3339 timestamp '{value}': {e}"))?;
    Ok(EventTime::from_unix_micros(dt.timestamp_micros()))
}

#[allow(clippy::too_many_arguments)]
async fn temporal_build(
    spec_path: &str,
    input: &str,
    window: Option<(String, String)>,
    states_output: Option<&str>,
    registry_output: Option<&str>,
    null_policy: &str,
    explain: bool,
) -> Result<(), String> {
    let spec = load_spec(spec_path)?;
    let null_policy = match null_policy {
        "reject" => NullEventTimePolicy::Reject,
        "quarantine" => NullEventTimePolicy::Quarantine,
        other => return Err(format!("--null-policy expects 'reject' or 'quarantine', got '{other}'")),
    };
    let window = match window {
        Some((start, end)) => {
            let start = parse_rfc3339(&start)?;
            let end = parse_rfc3339(&end)?;
            Some(TimeRange::new(start, end).map_err(|e| e.to_string())?)
        }
        None => None,
    };

    let ctx = SessionContext::new();
    let started = Instant::now();
    if input.ends_with(".csv") {
        ctx.register_csv("events", input, CsvReadOptions::new()).await
    } else {
        ctx.register_parquet("events", input, ParquetReadOptions::default()).await
    }
    .map_err(|e| format!("cannot read {input}: {e}"))?;

    if explain {
        let report = explain_temporal_build(&ctx, &spec, "events", window, null_policy)
            .await
            .map_err(|e| e.to_string())?;
        let elapsed = started.elapsed();
        println!(
            "cube '{}' (explain, {elapsed:.2?}):\n  source rows:       {}\n  generated xunits:  {}\n  observed buckets:  {}\n  output rows:       {}\n  null event times:  {}",
            spec.name,
            report.source_rows,
            report.generated_xunits,
            report.observed_buckets,
            report.output_rows,
            report.null_event_time_rows,
        );
        return Ok(());
    }

    let (states_path, registry_path) = match (states_output, registry_output) {
        (Some(s), Some(r)) => (s, r),
        _ => {
            return Err(
                "--states-output and --registry-output are both required (or pass --explain for a dry run)"
                    .into(),
            )
        }
    };

    let output = cubism_datafusion::build_temporal(&ctx, &spec, "events", window, None, null_policy)
        .await
        .map_err(|e| e.to_string())?;
    write_temporal_fixtures(&output, states_path, registry_path)
        .await
        .map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();

    println!(
        "cube '{}': wrote {states_path} and {registry_path} in {elapsed:.2?} ({} null event time row(s) dropped)",
        spec.name, output.metadata.null_event_time_rows,
    );
    Ok(())
}

fn read_parquet_batches(path: &str) -> Result<Vec<arrow_array::RecordBatch>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| format!("cannot open parquet file {path}: {e}"))?
        .build()
        .map_err(|e| format!("cannot build a parquet reader for {path}: {e}"))?;
    reader
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot read parquet batches from {path}: {e}"))
}

/// Creates the tables (if needed), claims, appends, publishes, and reads
/// the published rows back — all within this one process. See the module
/// doc comment for why this is one command rather than separate
/// `init`/`append`/`publish`/`verify` steps.
///
/// `--catalog-db`/`--control-db` are optional. Given both, `iceberg-build`
/// uses [`CatalogConfig::Sqlite`] and [`PublicationStore::Sqlite`] instead
/// of the non-durable defaults, so a second invocation pointed at the same
/// three paths (`--warehouse`, `--catalog-db`, `--control-db`) sees the
/// first invocation's tables and publications — a real cross-process
/// build. Omitted, `iceberg-build` behaves exactly as before: an isolated,
/// non-durable single-process build (see the module doc comment's original
/// rationale for why that was the only option Phase 3 shipped with).
///
/// A second, durable invocation with the same `--run-id` recovers rather
/// than blindly re-appending: [issue #17](https://github.com/jeromebanks/cubism-rs/issues/17)'s
/// fix, applied here as well as in `CorrectionCoordinator::execute` —
/// `RunState::Claimed` alone does not mean the append was never attempted,
/// since a crash between `AggregateWriter::append_window`'s commit and
/// `PublicationStore::record_append` leaves exactly that state. This
/// command checks Iceberg's own committed state
/// (`AggregateReader::run_append_snapshot`) before deciding whether to
/// append again, the same as the coordinator does.
#[allow(clippy::too_many_arguments)]
async fn iceberg_build(
    spec_path: &str,
    states_input: &str,
    registry_input: &str,
    window_id: &str,
    revision: u64,
    run_id: &str,
    warehouse: &str,
    catalog_db: Option<&str>,
    control_db: Option<&str>,
) -> Result<(), String> {
    let spec = load_spec(spec_path)?;
    let window_id = WindowId::new(window_id).map_err(|e| e.to_string())?;
    let revision = WindowRevision::new(revision).map_err(|e| e.to_string())?;

    let states = read_parquet_batches(states_input)?;
    let registry = read_parquet_batches(registry_input)?;
    let expected_rows: u64 = states.iter().map(|b| b.num_rows() as u64).sum();

    let durable = catalog_db.is_some() || control_db.is_some();
    let (catalog_db, control_db) = match (catalog_db, control_db) {
        (Some(c), Some(p)) => (c, p),
        _ if durable => {
            return Err("--catalog-db and --control-db must both be given, or neither".into());
        }
        _ => ("", ""),
    };

    let states_schema = temporal_state_schema(&spec).map_err(|e| e.to_string())?;
    let config = if durable {
        CatalogConfig::Sqlite { warehouse: PathBuf::from(warehouse), catalog_db: PathBuf::from(catalog_db) }
    } else {
        CatalogConfig::Memory { warehouse: PathBuf::from(warehouse) }
    };
    let catalog = cubism_iceberg::config::open_catalog(&config).await.map_err(|e| e.to_string())?;
    let table = TemporalTable::create(catalog.as_ref(), &spec.name, &states_schema)
        .await
        .map_err(|e| e.to_string())?;

    let publications = if durable {
        PublicationStore::sqlite(std::path::Path::new(control_db)).await.map_err(|e| e.to_string())?
    } else {
        PublicationStore::in_memory()
    };
    let claim = publications
        .claim_run(&spec.name, &window_id, run_id, revision, expected_rows)
        .await
        .map_err(|e| e.to_string())?;
    // A recorded snapshot id if this run's append is already known-durable
    // (`Appended`/`Published`). `Claimed` alone does *not* mean "never
    // attempted" — see the `run_append_snapshot` check below, #17.
    let already_recorded_snapshot_id = match claim.state() {
        cubism_iceberg::RunState::Appended {
            aggregate_snapshot_id,
            ..
        }
        | cubism_iceberg::RunState::Published {
            aggregate_snapshot_id,
            ..
        } => Some(*aggregate_snapshot_id),
        cubism_iceberg::RunState::Claimed { .. } => None,
    };

    let started = Instant::now();
    let result = if let Some(snapshot_id) = already_recorded_snapshot_id {
        cubism_iceberg::CommitResult {
            snapshot_id,
            states_file_count: 0,
            registry_file_count: 0,
            row_count: expected_rows,
        }
    } else {
        // `Claimed` is ambiguous between "append never attempted" and
        // "append's `fast_append` already committed, but the process
        // crashed before `record_append` persisted that fact" — the same
        // gap `CorrectionCoordinator::execute` has, fixed the same way here
        // rather than assuming this hand-rolled protocol is exempt (#17).
        let already_committed = AggregateReader::run_append_snapshot(
            catalog.as_ref(),
            &table,
            &window_id,
            revision,
            run_id,
        )
        .await
        .map_err(|e| e.to_string())?;
        match already_committed {
            Some(snapshot_id) => {
                publications
                    .record_append(run_id, snapshot_id)
                    .await
                    .map_err(|e| e.to_string())?;
                cubism_iceberg::CommitResult {
                    snapshot_id,
                    states_file_count: 0,
                    registry_file_count: 0,
                    row_count: expected_rows,
                }
            }
            None => {
                let result = AggregateWriter::append_window(
                    catalog.as_ref(),
                    &table,
                    AppendWindow {
                        window_id: &window_id,
                        revision,
                        run_id,
                        states: &states,
                        registry: &registry,
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
                publications
                    .record_append(run_id, result.snapshot_id)
                    .await
                    .map_err(|e| e.to_string())?;
                result
            }
        }
    };

    let expected_current =
        publications.current(&spec.name, &window_id).await.map_err(|e| e.to_string())?;
    let publication = publications.publish(run_id, expected_current).await.map_err(|e| e.to_string())?;

    let visible = AggregateReader::read_window(catalog.as_ref(), &table, &publications, &window_id)
        .await
        .map_err(|e| e.to_string())?;
    let visible_rows: usize = visible.iter().map(|b| b.num_rows()).sum();
    let elapsed = started.elapsed();

    println!(
        "cube '{}': built window '{}' revision {} (run '{run_id}') in {elapsed:.2?} — snapshot {}, {} states file(s), {} registry file(s), {} row(s) written, {visible_rows} row(s) visible after publish",
        spec.name,
        window_id.as_str(),
        publication.revision.get(),
        result.snapshot_id,
        result.states_file_count,
        result.registry_file_count,
        result.row_count,
    );
    if durable {
        println!("catalog and control store are durable (--catalog-db, --control-db) — a second invocation with the same paths observes this one's tables and publications");
    } else {
        println!(
            "note: this build used the non-durable defaults — catalog and control store do not survive past this process (pass --catalog-db and --control-db for a durable build; see docs/TIMESERIES_PHASE_4_HANDOFF.md)"
        );
    }
    Ok(())
}

/// `--spec`/`--warehouse` (plus optionally `--catalog-db`/`--control-db`,
/// same durability switch `iceberg-build` uses) also serve `/api/series`
/// against that cube's temporal tables, alongside the static-cube routes
/// `cube_path` already provides. Without them, behaves exactly as before —
/// static-cube serving only (`docs/TIMESERIES_ROADMAP.md`'s Milestone 11).
#[allow(clippy::too_many_arguments)]
async fn serve(
    cube_path: &str,
    port: u16,
    spec_path: Option<&str>,
    warehouse: Option<&str>,
    catalog_db: Option<&str>,
    control_db: Option<&str>,
) -> Result<(), String> {
    let store = cubism_serve::CubeStore::from_path(cube_path).map_err(|e| e.to_string())?;
    match (spec_path, warehouse) {
        (None, None) => cubism_serve::serve(store, port).await.map_err(|e| e.to_string()),
        (Some(spec_path), Some(warehouse)) => {
            let spec = load_spec(spec_path)?;
            let durable = catalog_db.is_some() || control_db.is_some();
            let (catalog_db, control_db) = match (catalog_db, control_db) {
                (Some(c), Some(p)) => (c, p),
                _ if durable => {
                    return Err("--catalog-db and --control-db must both be given, or neither".into());
                }
                _ => ("", ""),
            };
            let config = if durable {
                CatalogConfig::Sqlite { warehouse: PathBuf::from(warehouse), catalog_db: PathBuf::from(catalog_db) }
            } else {
                CatalogConfig::Memory { warehouse: PathBuf::from(warehouse) }
            };
            let publications = if durable {
                PublicationStore::sqlite(std::path::Path::new(control_db)).await.map_err(|e| e.to_string())?
            } else {
                PublicationStore::in_memory()
            };
            let series = cubism_serve::SeriesState::open(spec, &config, publications).await?;
            cubism_serve::serve_with_series(store, series, port).await.map_err(|e| e.to_string())
        }
        _ => Err("--spec and --warehouse must be given together, or neither".into()),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("validate") if args.len() == 2 => validate(&args[1]),
        Some("serve") if args.len() >= 2 => {
            let mut port = 8080u16;
            let mut spec_path = None;
            let mut warehouse = None;
            let mut catalog_db = None;
            let mut control_db = None;
            let mut flag_err = None;
            let mut i = 2;
            while i < args.len() {
                match (args[i].as_str(), args.get(i + 1)) {
                    ("--port", Some(v)) => match v.parse() {
                        Ok(p) => port = p,
                        Err(_) => flag_err = Some(format!("--port expects a number, got '{v}'")),
                    },
                    ("--spec", Some(v)) => spec_path = Some(v.clone()),
                    ("--warehouse", Some(v)) => warehouse = Some(v.clone()),
                    ("--catalog-db", Some(v)) => catalog_db = Some(v.clone()),
                    ("--control-db", Some(v)) => control_db = Some(v.clone()),
                    (flag, _) => flag_err = Some(format!("unknown or incomplete flag '{flag}'")),
                }
                i += 2;
            }
            match flag_err {
                Some(e) => Err(e),
                None => {
                    serve(
                        &args[1],
                        port,
                        spec_path.as_deref(),
                        warehouse.as_deref(),
                        catalog_db.as_deref(),
                        control_db.as_deref(),
                    )
                    .await
                }
            }
        }
        Some("run") if args.len() >= 2 => {
            let spec_path = &args[1];
            let mut input = None;
            let mut output = None;
            let mut show = 10usize;
            let mut i = 2;
            let mut flag_err = None;
            while i < args.len() {
                match (args[i].as_str(), args.get(i + 1)) {
                    ("--input", Some(v)) => input = Some(v.clone()),
                    ("--output", Some(v)) => output = Some(v.clone()),
                    ("--show", Some(v)) => match v.parse() {
                        Ok(n) => show = n,
                        Err(_) => flag_err = Some(format!("--show expects a number, got '{v}'")),
                    },
                    (flag, _) => flag_err = Some(format!("unknown or incomplete flag '{flag}'")),
                }
                i += 2;
            }
            match (flag_err, input) {
                (Some(e), _) => Err(e),
                (None, None) => Err("--input is required".into()),
                (None, Some(input)) => run(spec_path, &input, output.as_deref(), show).await,
            }
        }
        Some("temporal-build") if args.len() >= 2 => {
            let spec_path = &args[1];
            let mut input = None;
            let mut window_start = None;
            let mut window_end = None;
            let mut states_output = None;
            let mut registry_output = None;
            let mut null_policy = "reject".to_string();
            let mut explain = false;
            let mut flag_err = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--explain" => {
                        explain = true;
                        i += 1;
                    }
                    flag @ ("--input" | "--window-start" | "--window-end" | "--states-output"
                    | "--registry-output" | "--null-policy") => match args.get(i + 1) {
                        Some(v) => {
                            match flag {
                                "--input" => input = Some(v.clone()),
                                "--window-start" => window_start = Some(v.clone()),
                                "--window-end" => window_end = Some(v.clone()),
                                "--states-output" => states_output = Some(v.clone()),
                                "--registry-output" => registry_output = Some(v.clone()),
                                "--null-policy" => null_policy = v.clone(),
                                _ => unreachable!(),
                            }
                            i += 2;
                        }
                        None => {
                            flag_err = Some(format!("{flag} expects a value"));
                            break;
                        }
                    },
                    other => {
                        flag_err = Some(format!("unknown flag '{other}'"));
                        break;
                    }
                }
            }
            match (flag_err, input) {
                (Some(e), _) => Err(e),
                (None, None) => Err("--input is required".into()),
                (None, Some(input)) => {
                    if window_start.is_some() != window_end.is_some() {
                        Err("--window-start and --window-end must be given together".into())
                    } else {
                        let window = window_start.zip(window_end);
                        temporal_build(
                            spec_path,
                            &input,
                            window,
                            states_output.as_deref(),
                            registry_output.as_deref(),
                            &null_policy,
                            explain,
                        )
                        .await
                    }
                }
            }
        }
        Some("iceberg-build") if args.len() >= 2 => {
            let spec_path = &args[1];
            let mut states_input = None;
            let mut registry_input = None;
            let mut window_id = None;
            let mut revision = None;
            let mut run_id = None;
            let mut warehouse = None;
            let mut catalog_db = None;
            let mut control_db = None;
            let mut flag_err = None;
            let mut i = 2;
            while i < args.len() {
                match (args[i].as_str(), args.get(i + 1)) {
                    ("--states-input", Some(v)) => states_input = Some(v.clone()),
                    ("--registry-input", Some(v)) => registry_input = Some(v.clone()),
                    ("--window-id", Some(v)) => window_id = Some(v.clone()),
                    ("--revision", Some(v)) => match v.parse() {
                        Ok(n) => revision = Some(n),
                        Err(_) => flag_err = Some(format!("--revision expects a positive integer, got '{v}'")),
                    },
                    ("--run-id", Some(v)) => run_id = Some(v.clone()),
                    ("--warehouse", Some(v)) => warehouse = Some(v.clone()),
                    ("--catalog-db", Some(v)) => catalog_db = Some(v.clone()),
                    ("--control-db", Some(v)) => control_db = Some(v.clone()),
                    (flag, _) => flag_err = Some(format!("unknown or incomplete flag '{flag}'")),
                }
                i += 2;
            }
            match (flag_err, states_input, registry_input, window_id, revision, run_id, warehouse) {
                (Some(e), ..) => Err(e),
                (None, Some(s), Some(r), Some(w), Some(rev), Some(run), Some(wh)) => {
                    iceberg_build(spec_path, &s, &r, &w, rev, &run, &wh, catalog_db.as_deref(), control_db.as_deref())
                        .await
                }
                _ => Err(
                    "--states-input, --registry-input, --window-id, --revision, --run-id, and --warehouse are all required"
                        .into(),
                ),
            }
        }
        _ => Err(USAGE.into()),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
