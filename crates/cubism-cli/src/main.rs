//! `cubism` CLI.
//!
//! ```text
//! cubism validate <spec.yaml>
//! cubism run <spec.yaml> --input <events.parquet|csv> [--output cube.parquet] [--show N]
//! cubism temporal-build <spec.yaml> --input <events.parquet|csv>
//!     [--window-start <RFC3339>] [--window-end <RFC3339>]
//!     [--states-output <states.parquet> --registry-output <registry.parquet>]
//!     [--null-policy reject|quarantine] [--explain]
//! ```

use cubism_core::{CubeSpec, EventTime, TimeRange};
use cubism_datafusion::build_cube;
use cubism_datafusion::datafusion::dataframe::DataFrameWriteOptions;
use cubism_datafusion::datafusion::prelude::{CsvReadOptions, ParquetReadOptions, SessionContext};
use cubism_datafusion::temporal_build::{
    explain_temporal_build, write_temporal_fixtures, NullEventTimePolicy,
};
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage:\n  cubism validate <spec.yaml>\n  cubism run <spec.yaml> --input <events.parquet|csv> [--output cube.parquet] [--show N]\n  cubism temporal-build <spec.yaml> --input <events.parquet|csv> [--window-start <RFC3339>] [--window-end <RFC3339>] [--states-output <path> --registry-output <path>] [--null-policy reject|quarantine] [--explain]\n  cubism serve <cube.parquet> [--port 8080]";

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

async fn serve(cube_path: &str, port: u16) -> Result<(), String> {
    let store = cubism_serve::CubeStore::from_path(cube_path).map_err(|e| e.to_string())?;
    cubism_serve::serve(store, port).await.map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("validate") if args.len() == 2 => validate(&args[1]),
        Some("serve") if args.len() >= 2 => {
            let mut port = 8080u16;
            let mut flag_err = None;
            let mut i = 2;
            while i < args.len() {
                match (args[i].as_str(), args.get(i + 1)) {
                    ("--port", Some(v)) => match v.parse() {
                        Ok(p) => port = p,
                        Err(_) => flag_err = Some(format!("--port expects a number, got '{v}'")),
                    },
                    (flag, _) => flag_err = Some(format!("unknown or incomplete flag '{flag}'")),
                }
                i += 2;
            }
            match flag_err {
                Some(e) => Err(e),
                None => serve(&args[1], port).await,
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
