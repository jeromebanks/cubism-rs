//! Throwaway diagnostic, not part of the benchmark harness proper: prints
//! DataFusion's logical + verbose physical plan for AGGREGATE_SQL against a
//! real registered source file, to answer one question empirically instead
//! of by inference -- does the physical plan exploit the `ORDER BY
//! bucket_start`-implied sortedness to bound the GROUP BY's hash-aggregate
//! state to one bucket at a time, or does it materialize a hash-aggregate
//! over the WHOLE distinct (bucket, xunit) key space before the sort/emit
//! stage? SQL/spec text duplicated from lib.rs (both are private consts)
//! rather than changing that file's visibility for a one-off diagnostic.
//!
//! Usage: cargo run --release --example explain_aggregate -- <source.parquet>

use anyhow::Result;
use cubism_core::CubeSpec;
use cubism_datafusion::datafusion::execution::config::SessionConfig;
use cubism_datafusion::datafusion::execution::memory_pool::GreedyMemoryPool;
use cubism_datafusion::datafusion::execution::runtime_env::RuntimeEnvBuilder;
use cubism_datafusion::datafusion::prelude::{ParquetReadOptions, SessionContext};
use cubism_datafusion::udaf::sketch_udfs;
use cubism_datafusion::udf::cube_udfs;
use std::sync::Arc;

const BENCH_SPEC: &str = r#"
apiVersion: v1
name: phase0b_layout_benchmark
dimensions:
  - name: device
  - name: region
measures:
  - name: events
    agg: count
includeGlobal: true
maxDictionaryEntries: 1000000
"#;

const AGGREGATE_SQL: &str = r#"
WITH __input AS (
  SELECT
    bucket_start,
    CAST(device AS VARCHAR) AS device,
    CAST(region AS VARCHAR) AS region,
    amount,
    entity_id
  FROM events
),
__exploded AS (
  SELECT
    bucket_start,
    unnest(cubism_xunit_keys(device, region)) AS xunit_key,
    amount,
    entity_id
  FROM __input
),
__cells AS (
  SELECT
    bucket_start,
    xunit_key,
    SUM(amount) AS sum_state,
    COUNT(*) AS count_state,
    cubism_kmv_sketch(entity_id) AS kmv_state
  FROM __exploded
  GROUP BY bucket_start, xunit_key
)
SELECT
  bucket_start,
  cubism_xunit_str(xunit_key) AS xunit,
  sum_state,
  count_state,
  kmv_state
FROM __cells
ORDER BY bucket_start
"#;

#[tokio::main]
async fn main() -> Result<()> {
    let input = std::env::args()
        .nth(1)
        .expect("usage: explain_aggregate <source.parquet>");

    let runtime = RuntimeEnvBuilder::new()
        .with_memory_pool(Arc::new(GreedyMemoryPool::new(2048 * 1024 * 1024)))
        .build()?;
    let session_config = SessionConfig::new().with_target_partitions(4);
    let context = SessionContext::new_with_config_rt(session_config, Arc::new(runtime));
    context
        .register_parquet("events", &input, ParquetReadOptions::default())
        .await?;

    let spec = CubeSpec::from_yaml(BENCH_SPEC)?;
    let (keys, presenter, _dictionary) = cube_udfs(&spec);
    context.register_udf(keys);
    context.register_udf(presenter);
    let (aggregates, presenters) = sketch_udfs();
    for aggregate in aggregates {
        context.register_udaf(aggregate);
    }
    for presenter in presenters {
        context.register_udf(presenter);
    }

    println!("=== EXPLAIN (logical + verbose physical plan) ===");
    let df = context
        .sql(&format!("EXPLAIN VERBOSE {AGGREGATE_SQL}"))
        .await?;
    df.show().await?;

    Ok(())
}
