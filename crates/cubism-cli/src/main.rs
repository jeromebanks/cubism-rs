//! `cubism` CLI.
//!
//! ```text
//! cubism validate <spec.yaml>
//! cubism run <spec.yaml> --input <events.parquet|csv> [--output cube.parquet] [--show N]
//! ```

use cubism_core::CubeSpec;
use cubism_datafusion::build_cube;
use cubism_datafusion::datafusion::dataframe::DataFrameWriteOptions;
use cubism_datafusion::datafusion::prelude::{CsvReadOptions, ParquetReadOptions, SessionContext};
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage:\n  cubism validate <spec.yaml>\n  cubism run <spec.yaml> --input <events.parquet|csv> [--output cube.parquet] [--show N]";

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
        cube.clone()
            .sort(vec![
                cubism_datafusion::datafusion::prelude::col(&spec.measures[0].name).sort(false, false),
            ])
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

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("validate") if args.len() == 2 => validate(&args[1]),
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
