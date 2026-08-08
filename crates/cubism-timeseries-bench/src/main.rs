use anyhow::{Context, Result, bail};
use cubism_timeseries_bench::{
    DEFAULT_DATAFUSION_MEMORY_LIMIT_BYTES, GenerateConfig, Occupancy, RustRunConfig,
    generate_source, run_rust,
};
use std::path::PathBuf;

const USAGE: &str = "\
usage:
  cubism-timeseries-bench generate --output FILE [--rows N] [--occupancy sparse|dense] [--seed N] [--batch-rows N]
  cubism-timeseries-bench rust --input FILE --output-dir DIR [--partitions N] [--memory-limit-mb N]
  cubism-timeseries-bench smoke --work-dir DIR [--rows N] [--partitions N] [--memory-limit-mb N]
  cubism-timeseries-bench preflight";

fn value(args: &[String], flag: &str) -> Result<Option<String>> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    args.get(index + 1)
        .cloned()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))
}

fn required_path(args: &[String], flag: &str) -> Result<PathBuf> {
    value(args, flag)?
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("{flag} is required"))
}

fn parsed<T>(args: &[String], flag: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match value(args, flag)? {
        Some(raw) => raw
            .parse()
            .map_err(|error| anyhow::anyhow!("{flag}: {error}")),
        None => Ok(default),
    }
}

fn memory_limit_bytes(args: &[String]) -> Result<usize> {
    match value(args, "--memory-limit-mb")? {
        Some(raw) => {
            let mb: usize = raw
                .parse()
                .map_err(|error| anyhow::anyhow!("--memory-limit-mb: {error}"))?;
            Ok(mb * 1024 * 1024)
        }
        None => Ok(DEFAULT_DATAFUSION_MEMORY_LIMIT_BYTES),
    }
}

fn ensure_known_flags(args: &[String], known: &[&str]) -> Result<()> {
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        if !known.contains(&flag.as_str()) {
            bail!("unknown flag '{flag}'");
        }
        if args.get(index + 1).is_none() {
            bail!("{flag} requires a value");
        }
        index += 2;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        bail!(USAGE);
    };
    let flags = &args[1..];

    match command {
        "generate" => {
            ensure_known_flags(
                flags,
                &[
                    "--output",
                    "--rows",
                    "--occupancy",
                    "--seed",
                    "--batch-rows",
                ],
            )?;
            let occupancy = value(flags, "--occupancy")?
                .unwrap_or_else(|| "sparse".into())
                .parse()?;
            let config = GenerateConfig {
                output: required_path(flags, "--output")?,
                rows: parsed(flags, "--rows", 100_000_u64)?,
                occupancy,
                seed: parsed(flags, "--seed", 1_u64)?,
                batch_rows: parsed(flags, "--batch-rows", 65_536_usize)?,
            };
            let metrics = generate_source(&config)?;
            println!("{}", serde_json::to_string_pretty(&metrics)?);
        }
        "rust" => {
            ensure_known_flags(
                flags,
                &[
                    "--input",
                    "--output-dir",
                    "--partitions",
                    "--memory-limit-mb",
                ],
            )?;
            let config = RustRunConfig {
                input: required_path(flags, "--input")?,
                output_dir: required_path(flags, "--output-dir")?,
                target_partitions: parsed(flags, "--partitions", 4_usize)?,
                memory_limit_bytes: memory_limit_bytes(flags)?,
            };
            let metrics = run_rust(&config).await?;
            println!("{}", serde_json::to_string_pretty(&metrics)?);
        }
        "smoke" => {
            ensure_known_flags(
                flags,
                &["--work-dir", "--rows", "--partitions", "--memory-limit-mb"],
            )?;
            let work_dir = required_path(flags, "--work-dir")?;
            if work_dir.exists() {
                bail!(
                    "smoke work directory already exists: {}",
                    work_dir.display()
                );
            }
            std::fs::create_dir_all(&work_dir)
                .with_context(|| format!("create {}", work_dir.display()))?;
            let rows = parsed(flags, "--rows", 25_000_u64)?;
            let partitions = parsed(flags, "--partitions", 4_usize)?;
            let memory_limit_bytes = memory_limit_bytes(flags)?;
            for occupancy in [Occupancy::Sparse, Occupancy::Dense] {
                let name = occupancy.as_str();
                let input = work_dir.join(format!("{name}-source.parquet"));
                generate_source(&GenerateConfig {
                    output: input.clone(),
                    rows,
                    occupancy,
                    seed: 1,
                    batch_rows: 65_536,
                })?;
                let metrics = run_rust(&RustRunConfig {
                    input,
                    output_dir: work_dir.join(format!("{name}-rust")),
                    target_partitions: partitions,
                    memory_limit_bytes,
                })
                .await?;
                println!("{}", serde_json::to_string_pretty(&metrics)?);
            }
        }
        "preflight" => {
            let spark = std::process::Command::new("spark-submit")
                .arg("--version")
                .output();
            match spark {
                Ok(output) if output.status.success() => {
                    println!("spark-submit: available");
                }
                Ok(output) => {
                    bail!(
                        "spark-submit exists but failed with status {}",
                        output.status
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    bail!(
                        "spark-submit: not found; Phase 0B cannot select a Rust/Spark boundary yet"
                    );
                }
                Err(error) => return Err(error.into()),
            }
        }
        _ => bail!(USAGE),
    }
    Ok(())
}
