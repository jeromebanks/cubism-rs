//! `cubism` CLI. M2 wires this to the DataFusion engine:
//! `cubism run spec.yaml --input data/*.parquet --output cube.parquet`.
//! For now it validates specs, which is already useful standalone.

use cubism_core::CubeSpec;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("validate") => {
            let Some(path) = args.get(2) else {
                eprintln!("usage: cubism validate <spec.yaml>");
                return ExitCode::FAILURE;
            };
            let yaml = match std::fs::read_to_string(path) {
                Ok(y) => y,
                Err(e) => {
                    eprintln!("cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match CubeSpec::from_yaml(&yaml) {
                Ok(spec) => {
                    println!(
                        "OK: cube '{}' — {} dimension(s), {} measure(s), {} filter rule(s)",
                        spec.name,
                        spec.dimensions.len(),
                        spec.measures.len(),
                        spec.filter_rules.len()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: cubism validate <spec.yaml>   (run/build arrive in M2)");
            ExitCode::FAILURE
        }
    }
}
