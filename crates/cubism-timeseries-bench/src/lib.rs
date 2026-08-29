//! Reproducible Phase 0B time-series layout harness.
//!
//! This crate intentionally benchmarks the frozen Phase 1 identity and KMV
//! formats without introducing temporal production execution or Iceberg
//! persistence. Source generation is deterministic, bucket membership is UTC
//! and half-open, and every emitted layout is read back into one semantic
//! representation before a run is accepted.

use anyhow::{Context, Result, bail, ensure};
use cubism_core::encoding::{canonical_xunit_content_id, encode_canonical_xunit};
use cubism_core::{CanonicalXUnit, CubeSpec, KmvSketch, XUnit};
use cubism_datafusion::datafusion::arrow::array::{
    Array, ArrayRef, BinaryArray, BinaryBuilder, FixedSizeBinaryArray, FixedSizeBinaryBuilder,
    Int64Array, StringArray, StructArray, TimestampMicrosecondArray, UInt16Array, UInt64Array,
};
use cubism_datafusion::datafusion::arrow::datatypes::{DataType, Field, Fields, Schema, TimeUnit};
use cubism_datafusion::datafusion::arrow::record_batch::RecordBatch;
use cubism_datafusion::datafusion::execution::config::SessionConfig;
use cubism_datafusion::datafusion::execution::memory_pool::GreedyMemoryPool;
use cubism_datafusion::datafusion::execution::runtime_env::RuntimeEnvBuilder;
use cubism_datafusion::datafusion::prelude::{ParquetReadOptions, SessionContext};
use cubism_datafusion::udaf::sketch_udfs;
use cubism_datafusion::udf::cube_udfs;
use futures::StreamExt;
use parquet::arrow::ArrowWriter;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SOURCE_SCHEMA_VERSION: u16 = 1;
// v2: aggregate_ms now spans the whole streaming aggregate+write loop
// (it used to measure aggregation only, before the four layouts were
// written); v3: run.json gained explicit compression/row-group fields
// (see PINNED_COMPRESSION / LAYOUT_ROW_GROUP_ROWS below) -- no behavior
// changed, but older run.json files won't have these keys.
const HARNESS_SCHEMA_VERSION: u16 = 3;
const STATE_VERSION: u16 = 1;
// Phase 0B pinned write-path settings, applied identically to the source
// file and all four layouts. UNCOMPRESSED matches this crate's prior
// (implicit, parquet-rs default) behavior -- pinning it here makes it a
// recorded benchmark decision instead of an accident of the library
// default, per remaining-work item 5. It has NOT been evaluated for its
// effect on disk footprint at the 10-100M row target; four uncompressed
// layouts at that scale may consume much more disk than a compressed
// codec would. Revisit before trusting file-size comparisons at scale.
const PINNED_COMPRESSION: Compression = Compression::UNCOMPRESSED;
const PINNED_COMPRESSION_LABEL: &str = "uncompressed";
// Row-group target for the four output layouts (not the source file,
// which uses its own --batch-rows). Matches the value already in use;
// pinned as one named constant instead of two independent magic numbers
// so it can't silently drift between OpenLayout and write_single_batch.
const LAYOUT_ROW_GROUP_ROWS: usize = 65_536;
const BASE_BUCKET_START_US: i64 = 1_767_225_600_000_000; // 2026-01-01T00:00:00Z
const HOUR_US: i64 = 3_600_000_000;
const BUCKETS: u64 = 24 * 7;
const SEMANTIC_DIGEST_DOMAIN: &[u8] = b"cubism-phase0b-semantic-v1";

/// Default bound on DataFusion's own GROUP BY / ORDER BY execution memory,
/// so those operators spill sorted runs to disk instead of growing
/// unbounded with the row/cell count. This is independent of the harness's
/// own per-bucket buffers (see `BucketCells`), which are bounded by the
/// fixed dictionary size in `BENCH_SPEC` regardless of input row count.
/// Overridable via `RustRunConfig::memory_limit_bytes` / `--memory-limit-mb`
/// and always recorded in `RustRunMetrics` since it is pinned benchmark
/// config, not an implementation detail.
pub const DEFAULT_DATAFUSION_MEMORY_LIMIT_BYTES: usize = 2 * 1024 * 1024 * 1024;

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

// Sorted only by `bucket_start`: a single-key sort guarantees all rows for
// one bucket are contiguous in the output stream (BUCKETS = 168, so each
// bucket's cell count is bounded by the dictionary size regardless of input
// rows). Within a bucket, rows land in `run_rust`'s BucketCells (a
// BTreeMap<[u8; 32], _> keyed by content ID), which re-sorts them to match
// the semantic digest's canonical (bucket asc, content ID asc) order.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Occupancy {
    Sparse,
    Dense,
}

impl Occupancy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sparse => "sparse",
            Self::Dense => "dense",
        }
    }
}

impl FromStr for Occupancy {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "sparse" => Ok(Self::Sparse),
            "dense" => Ok(Self::Dense),
            _ => bail!("occupancy must be 'sparse' or 'dense', got '{value}'"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenerateConfig {
    pub output: PathBuf,
    pub rows: u64,
    pub occupancy: Occupancy,
    pub seed: u64,
    pub batch_rows: usize,
}

#[derive(Debug, Clone)]
pub struct RustRunConfig {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub target_partitions: usize,
    pub memory_limit_bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct GenerationMetrics {
    pub source_schema_version: u16,
    pub output: String,
    pub rows: u64,
    pub occupancy: Occupancy,
    pub seed: u64,
    pub batch_rows: usize,
    pub compression: &'static str,
    pub bytes: u64,
    pub elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct LayoutMetrics {
    pub name: &'static str,
    pub files: Vec<String>,
    pub rows: usize,
    pub bytes: u64,
    pub write_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct RustRunMetrics {
    pub harness_schema_version: u16,
    pub engine: &'static str,
    pub engine_version: &'static str,
    pub input: String,
    pub input_bytes: u64,
    pub output_dir: String,
    pub target_partitions: usize,
    pub memory_limit_bytes: usize,
    pub layout_compression: &'static str,
    pub layout_row_group_rows: usize,
    pub aggregate_ms: u128,
    pub cell_count: usize,
    pub semantic_digest_blake3: String,
    pub layouts: Vec<LayoutMetrics>,
    pub verification_ms: u128,
    pub total_ms: u128,
}

/// Phase 0B remaining-work item 2: aligned/non-aligned short/long
/// bucket-range reads over an already-written `rust` run's four layouts.
///
/// "Aligned" / "non-aligned" is defined against each layout's *own* Parquet
/// row-group boundaries, discovered from that file's row-group statistics on
/// the `bucket_start` column -- not assumed from row counts, since which
/// regime applies (many buckets per row group at small scale, vs. one bucket
/// spanning many row groups at large scale, per
/// `docs/TIMESERIES_PHASE_0B_HARNESS.md` "Memory scaling") depends on actual
/// per-bucket cell density versus `LAYOUT_ROW_GROUP_ROWS`, which is data- and
/// scale-dependent. A range start is "aligned" iff it exactly equals some row
/// group's minimum `bucket_start` (no row group has to be scanned partly for
/// rows outside the requested range); "non-aligned" iff the start falls
/// inside a row group whose own minimum is an earlier bucket, so that group's
/// leading rows are scanned but not matched.
#[derive(Debug, Clone)]
pub struct RangeReadConfig {
    pub run_dir: PathBuf,
    pub short_buckets: u64,
    pub long_buckets: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeLength {
    Short,
    Long,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeAlignment {
    Aligned,
    NonAligned,
}

#[derive(Debug, Serialize)]
pub struct RangeReadCaseMetrics {
    pub layout: &'static str,
    pub length: RangeLength,
    pub alignment: RangeAlignment,
    pub bucket_count: u64,
    pub start_bucket_us: i64,
    pub end_bucket_us: i64,
    pub row_groups_total: usize,
    pub row_groups_scanned: usize,
    pub file_bytes_total: u64,
    pub bytes_scanned: u64,
    pub rows_scanned: usize,
    pub rows_matched: usize,
    pub read_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct RangeReadSkipped {
    pub layout: &'static str,
    pub length: RangeLength,
    pub alignment: RangeAlignment,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct RangeReadReport {
    pub run_dir: String,
    pub short_buckets: u64,
    pub long_buckets: u64,
    pub cases: Vec<RangeReadCaseMetrics>,
    pub skipped: Vec<RangeReadSkipped>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct RowGroupBucketRange {
    index: usize,
    min_us: i64,
    max_us: i64,
    num_rows: i64,
    total_byte_size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CellState {
    canonical_xunit: Vec<u8>,
    sum: i64,
    count: u64,
    kmv: Vec<u8>,
}

/// One bucket's cells, keyed by canonical XUnit content ID. Bounded by the
/// fixed dictionary size in `BENCH_SPEC` (device/region cardinality),
/// independent of the total input row or cell count.
type BucketCells = BTreeMap<[u8; 32], CellState>;

pub fn generate_source(config: &GenerateConfig) -> Result<GenerationMetrics> {
    ensure!(config.rows > 0, "rows must be greater than zero");
    ensure!(
        config.batch_rows > 0,
        "batch rows must be greater than zero"
    );
    ensure!(
        !config.output.exists(),
        "refusing to overwrite existing source {}",
        config.output.display()
    );
    if let Some(parent) = config.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create source directory {}", parent.display()))?;
    }

    let schema = source_schema();
    let properties = WriterProperties::builder()
        .set_max_row_group_row_count(Some(config.batch_rows))
        .set_compression(PINNED_COMPRESSION)
        .build();
    let file = File::create(&config.output)
        .with_context(|| format!("create {}", config.output.display()))?;
    let mut writer = ArrowWriter::try_new(file, Arc::clone(&schema), Some(properties))?;
    let started = Instant::now();

    let mut offset = 0_u64;
    while offset < config.rows {
        let batch_len = usize::try_from((config.rows - offset).min(config.batch_rows as u64))?;
        writer.write(&source_batch(
            config,
            offset,
            batch_len,
            Arc::clone(&schema),
        )?)?;
        offset += batch_len as u64;
    }
    writer.close()?;

    Ok(GenerationMetrics {
        source_schema_version: SOURCE_SCHEMA_VERSION,
        output: config.output.display().to_string(),
        rows: config.rows,
        occupancy: config.occupancy,
        seed: config.seed,
        batch_rows: config.batch_rows,
        compression: PINNED_COMPRESSION_LABEL,
        bytes: file_len(&config.output)?,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

pub async fn run_rust(config: &RustRunConfig) -> Result<RustRunMetrics> {
    ensure!(
        config.input.is_file(),
        "input does not exist: {}",
        config.input.display()
    );
    ensure!(
        config.target_partitions > 0,
        "target partitions must be greater than zero"
    );
    ensure!(
        !config.output_dir.exists(),
        "refusing to overwrite output directory {}",
        config.output_dir.display()
    );
    std::fs::create_dir_all(&config.output_dir)
        .with_context(|| format!("create {}", config.output_dir.display()))?;

    let total_started = Instant::now();
    let runtime = RuntimeEnvBuilder::new()
        .with_memory_pool(Arc::new(GreedyMemoryPool::new(config.memory_limit_bytes)))
        .build()?;
    let session_config = SessionConfig::new().with_target_partitions(config.target_partitions);
    let context = SessionContext::new_with_config_rt(session_config, Arc::new(runtime));
    let input = config.input.to_string_lossy();
    context
        .register_parquet("events", input.as_ref(), ParquetReadOptions::default())
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

    let aggregate_started = Instant::now();
    let mut stream = context.sql(AGGREGATE_SQL).await?.execute_stream().await?;

    let mut writers = LayoutWriters::create(&config.output_dir)?;
    let mut bucket_cells = BucketCells::new();
    let mut current_bucket: Option<i64> = None;
    let mut hasher = blake3::Hasher::new();
    hasher.update(SEMANTIC_DIGEST_DOMAIN);
    let mut cell_count = 0_usize;

    while let Some(batch) = stream.next().await {
        let batch = batch?;
        let buckets = typed_column::<TimestampMicrosecondArray>(&batch, "bucket_start")?;
        let xunits = typed_column::<StringArray>(&batch, "xunit")?;
        let sums = typed_column::<Int64Array>(&batch, "sum_state")?;
        let counts = typed_column::<Int64Array>(&batch, "count_state")?;
        let kmvs = typed_column::<BinaryArray>(&batch, "kmv_state")?;
        for row in 0..batch.num_rows() {
            let bucket = buckets.value(row);
            if current_bucket != Some(bucket) {
                if let Some(previous_bucket) = current_bucket {
                    cell_count += bucket_cells.len();
                    flush_bucket(previous_bucket, &bucket_cells, &mut writers, &mut hasher)?;
                    bucket_cells.clear();
                }
                current_bucket = Some(bucket);
            }

            let xunit = XUnit::from_str(xunits.value(row))?;
            let canonical = CanonicalXUnit::from(&xunit);
            let canonical_xunit = encode_canonical_xunit(&canonical)?;
            let content_id = canonical_xunit_content_id(&canonical)?;
            let count = u64::try_from(counts.value(row))?;
            let kmv = kmvs.value(row).to_vec();
            KmvSketch::from_bytes(&kmv)?;
            let previous = bucket_cells.insert(
                *content_id.as_bytes(),
                CellState {
                    canonical_xunit,
                    sum: sums.value(row),
                    count,
                    kmv,
                },
            );
            ensure!(previous.is_none(), "duplicate aggregate cell");
        }
    }
    if let Some(final_bucket) = current_bucket {
        cell_count += bucket_cells.len();
        flush_bucket(final_bucket, &bucket_cells, &mut writers, &mut hasher)?;
    }
    ensure!(cell_count > 0, "aggregate query produced no cells");
    let aggregate_ms = aggregate_started.elapsed().as_millis();

    let layouts = writers.finish()?;
    let digest = hasher.finalize().to_hex().to_string();

    let verification_started = Instant::now();
    verify_layouts(&config.output_dir, &digest, cell_count)?;
    let verification_ms = verification_started.elapsed().as_millis();

    let metrics = RustRunMetrics {
        harness_schema_version: HARNESS_SCHEMA_VERSION,
        engine: "rust-datafusion",
        engine_version: "54.0.0",
        input: config.input.display().to_string(),
        input_bytes: file_len(&config.input)?,
        output_dir: config.output_dir.display().to_string(),
        target_partitions: config.target_partitions,
        memory_limit_bytes: config.memory_limit_bytes,
        layout_compression: PINNED_COMPRESSION_LABEL,
        layout_row_group_rows: LAYOUT_ROW_GROUP_ROWS,
        aggregate_ms,
        cell_count,
        semantic_digest_blake3: digest,
        layouts,
        verification_ms,
        total_ms: total_started.elapsed().as_millis(),
    };
    let metrics_path = config.output_dir.join("run.json");
    let metrics_file = File::create(&metrics_path)?;
    serde_json::to_writer_pretty(metrics_file, &metrics)?;
    Ok(metrics)
}

fn source_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "bucket_start",
            DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
            false,
        ),
        Field::new("region", DataType::Utf8, false),
        Field::new("device", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, false),
        Field::new("entity_id", DataType::Utf8, false),
    ]))
}

fn source_batch(
    config: &GenerateConfig,
    offset: u64,
    rows: usize,
    schema: Arc<Schema>,
) -> Result<RecordBatch> {
    let mut buckets = Vec::with_capacity(rows);
    let mut regions = Vec::with_capacity(rows);
    let mut devices = Vec::with_capacity(rows);
    let mut amounts = Vec::with_capacity(rows);
    let mut entities = Vec::with_capacity(rows);

    for local in 0..rows {
        let row = offset + local as u64;
        let random = splitmix64(config.seed ^ row);
        let (bucket, region, device, entity) = match config.occupancy {
            Occupancy::Sparse => (
                random % BUCKETS,
                (random.rotate_left(17) % 50_000),
                (random.rotate_left(31) % 8),
                (random.rotate_left(43) % 200_000),
            ),
            Occupancy::Dense => (
                row % BUCKETS,
                (row / BUCKETS) % 256,
                (row / 17) % 8,
                (row * 13) % 10_000,
            ),
        };
        buckets.push(BASE_BUCKET_START_US + i64::try_from(bucket)? * HOUR_US);
        regions.push(format!("region_{region:05}"));
        devices.push(format!("device_{device}"));
        amounts.push(i64::try_from(random % 97 + 1)?);
        entities.push(format!("entity_{entity:06}"));
    }

    let bucket_array = TimestampMicrosecondArray::from(buckets).with_timezone("+00:00");
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(bucket_array),
            Arc::new(StringArray::from(regions)),
            Arc::new(StringArray::from(devices)),
            Arc::new(Int64Array::from(amounts)),
            Arc::new(StringArray::from(entities)),
        ],
    )
    .map_err(Into::into)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Digests and writes one bucket's cells to all four layouts, in the same
/// (bucket asc, content ID asc) order `verify_layouts`'s streaming readers
/// reproduce from the written files. Called once per bucket boundary, so
/// `cells` never exceeds one bucket's worth of state.
fn flush_bucket(
    bucket: i64,
    cells: &BucketCells,
    writers: &mut LayoutWriters,
    hasher: &mut blake3::Hasher,
) -> Result<()> {
    for (id, state) in cells {
        digest_update(hasher, bucket, id, state);
    }
    writers.write_bucket(bucket, cells)
}

fn digest_update(hasher: &mut blake3::Hasher, bucket: i64, id: &[u8; 32], state: &CellState) {
    hasher.update(&bucket.to_be_bytes());
    hasher.update(id);
    hasher.update(&(state.canonical_xunit.len() as u64).to_be_bytes());
    hasher.update(&state.canonical_xunit);
    hasher.update(&state.sum.to_be_bytes());
    hasher.update(&state.count.to_be_bytes());
    hasher.update(&(state.kmv.len() as u64).to_be_bytes());
    hasher.update(&state.kmv);
}

/// Four persistently open Parquet writers, fed one bucket's `RecordBatch`
/// at a time so no layout ever materializes more than one bucket in Rust
/// memory. The XUnit registry dedup map is the one exception: it is
/// necessarily global (one entry per distinct XUnit across the whole run),
/// bounded by `BENCH_SPEC`'s dictionary size rather than by row count.
struct LayoutWriters {
    per_measure: OpenLayout,
    wide: OpenLayout,
    tagged_struct: OpenLayout,
    registry_states: OpenLayout,
    registry_path: PathBuf,
    registry: BTreeMap<[u8; 32], Vec<u8>>,
}

impl LayoutWriters {
    fn create(output_dir: &Path) -> Result<Self> {
        Ok(Self {
            per_measure: OpenLayout::create(
                output_dir.join("per_measure.parquet"),
                per_measure_schema(),
            )?,
            wide: OpenLayout::create(output_dir.join("wide.parquet"), wide_schema())?,
            tagged_struct: OpenLayout::create(
                output_dir.join("tagged_struct.parquet"),
                tagged_struct_schema(),
            )?,
            registry_states: OpenLayout::create(
                output_dir.join("registry_states.parquet"),
                registry_states_schema(),
            )?,
            registry_path: output_dir.join("xunit_registry.parquet"),
            registry: BTreeMap::new(),
        })
    }

    fn write_bucket(&mut self, bucket: i64, cells: &BucketCells) -> Result<()> {
        self.per_measure.write(&per_measure_batch(bucket, cells)?)?;
        self.wide.write(&wide_batch(bucket, cells)?)?;
        self.tagged_struct
            .write(&tagged_struct_batch(bucket, cells)?)?;
        self.registry_states
            .write(&registry_states_batch(bucket, cells)?)?;
        for (id, state) in cells {
            if let Some(previous) = self.registry.insert(*id, state.canonical_xunit.clone()) {
                ensure!(
                    previous == state.canonical_xunit,
                    "xunit content ID collision in registry"
                );
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Vec<LayoutMetrics>> {
        let per_measure = self.per_measure.finish("per_measure")?;
        let wide = self.wide.finish("wide")?;
        let tagged_struct = self.tagged_struct.finish("tagged_struct")?;
        let mut registry_layout = self.registry_states.finish("xunit_registry")?;

        let registry_started = Instant::now();
        let registry_rows = self.registry.len();
        let mut ids = FixedSizeBinaryBuilder::new(32);
        let mut xunits = BinaryBuilder::new();
        for (id, xunit) in &self.registry {
            ids.append_value(id)?;
            xunits.append_value(xunit);
        }
        let schema = registry_schema();
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(ids.finish()), Arc::new(xunits.finish())],
        )?;
        write_single_batch(&self.registry_path, schema, &batch)?;

        registry_layout
            .files
            .push(self.registry_path.display().to_string());
        registry_layout.rows += registry_rows;
        registry_layout.bytes += file_len(&self.registry_path)?;
        registry_layout.write_ms += registry_started.elapsed().as_millis();

        Ok(vec![per_measure, wide, tagged_struct, registry_layout])
    }
}

/// One Parquet file written incrementally across many `write` calls.
struct OpenLayout {
    path: PathBuf,
    writer: ArrowWriter<File>,
    rows: usize,
    write_time: Duration,
}

impl OpenLayout {
    fn create(path: PathBuf, schema: Arc<Schema>) -> Result<Self> {
        let properties = WriterProperties::builder()
            .set_max_row_group_row_count(Some(LAYOUT_ROW_GROUP_ROWS))
            .set_compression(PINNED_COMPRESSION)
            .build();
        let file = File::create(&path).with_context(|| format!("create {}", path.display()))?;
        let writer = ArrowWriter::try_new(file, schema, Some(properties))?;
        Ok(Self {
            path,
            writer,
            rows: 0,
            write_time: Duration::ZERO,
        })
    }

    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let started = Instant::now();
        self.writer.write(batch)?;
        self.write_time += started.elapsed();
        self.rows += batch.num_rows();
        Ok(())
    }

    fn finish(self, name: &'static str) -> Result<LayoutMetrics> {
        let Self {
            path,
            writer,
            rows,
            mut write_time,
        } = self;
        let started = Instant::now();
        writer.close()?;
        write_time += started.elapsed();
        Ok(LayoutMetrics {
            name,
            files: vec![path.display().to_string()],
            rows,
            bytes: file_len(&path)?,
            write_ms: write_time.as_millis(),
        })
    }
}

fn per_measure_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        bucket_field(),
        Field::new("xunit_canonical", DataType::Binary, false),
        Field::new("measure_id", DataType::Utf8, false),
        Field::new("state_version", DataType::UInt16, false),
        Field::new("sum_i64", DataType::Int64, true),
        Field::new("count_u64", DataType::UInt64, true),
        Field::new("state_blob", DataType::Binary, true),
    ]))
}

fn per_measure_batch(bucket: i64, cells: &BucketCells) -> Result<RecordBatch> {
    let mut canonical = BinaryBuilder::new();
    let mut measure = Vec::with_capacity(cells.len() * 3);
    let mut versions = Vec::with_capacity(cells.len() * 3);
    let mut sums = Vec::with_capacity(cells.len() * 3);
    let mut counts = Vec::with_capacity(cells.len() * 3);
    let mut blobs = BinaryBuilder::new();

    for state in cells.values() {
        for name in ["sum", "count", "kmv"] {
            canonical.append_value(&state.canonical_xunit);
            measure.push(name);
            versions.push(STATE_VERSION);
            sums.push((name == "sum").then_some(state.sum));
            counts.push((name == "count").then_some(state.count));
            if name == "kmv" {
                blobs.append_value(&state.kmv);
            } else {
                blobs.append_null();
            }
        }
    }

    let rows = measure.len();
    RecordBatch::try_new(
        per_measure_schema(),
        vec![
            timestamp_array(vec![bucket; rows]),
            Arc::new(canonical.finish()),
            Arc::new(StringArray::from(measure)),
            Arc::new(UInt16Array::from(versions)),
            Arc::new(Int64Array::from(sums)),
            Arc::new(UInt64Array::from(counts)),
            Arc::new(blobs.finish()),
        ],
    )
    .map_err(Into::into)
}

fn wide_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        bucket_field(),
        Field::new("xunit_canonical", DataType::Binary, false),
        Field::new("sum_i64_v1", DataType::Int64, false),
        Field::new("count_u64_v1", DataType::UInt64, false),
        Field::new("kmv_blob_v1", DataType::Binary, false),
    ]))
}

fn wide_batch(bucket: i64, cells: &BucketCells) -> Result<RecordBatch> {
    let rows = cells.len();
    let mut canonical = Vec::with_capacity(rows);
    let mut sums = Vec::with_capacity(rows);
    let mut counts = Vec::with_capacity(rows);
    let mut blobs = Vec::with_capacity(rows);
    for state in cells.values() {
        canonical.push(state.canonical_xunit.clone());
        sums.push(state.sum);
        counts.push(state.count);
        blobs.push(state.kmv.clone());
    }
    RecordBatch::try_new(
        wide_schema(),
        vec![
            timestamp_array(vec![bucket; rows]),
            Arc::new(BinaryArray::from_iter_values(canonical)),
            Arc::new(Int64Array::from(sums)),
            Arc::new(UInt64Array::from(counts)),
            Arc::new(BinaryArray::from_iter_values(blobs)),
        ],
    )
    .map_err(Into::into)
}

fn tagged_struct_schema() -> Arc<Schema> {
    let state_fields: Fields = tagged_struct_state_fields().into();
    Arc::new(Schema::new(vec![
        bucket_field(),
        Field::new("xunit_canonical", DataType::Binary, false),
        Field::new("state", DataType::Struct(state_fields), false),
    ]))
}

fn tagged_struct_state_fields() -> Vec<Arc<Field>> {
    vec![
        Arc::new(Field::new("kind", DataType::Utf8, false)),
        Arc::new(Field::new("version", DataType::UInt16, false)),
        Arc::new(Field::new("sum_i64", DataType::Int64, true)),
        Arc::new(Field::new("count_u64", DataType::UInt64, true)),
        Arc::new(Field::new("blob", DataType::Binary, true)),
    ]
}

fn tagged_struct_batch(bucket: i64, cells: &BucketCells) -> Result<RecordBatch> {
    let mut canonical = BinaryBuilder::new();
    let mut kind = Vec::with_capacity(cells.len() * 3);
    let mut versions = Vec::with_capacity(cells.len() * 3);
    let mut sums = Vec::with_capacity(cells.len() * 3);
    let mut counts = Vec::with_capacity(cells.len() * 3);
    let mut blobs = BinaryBuilder::new();
    for state in cells.values() {
        for name in ["sum", "count", "kmv"] {
            canonical.append_value(&state.canonical_xunit);
            kind.push(name);
            versions.push(STATE_VERSION);
            sums.push((name == "sum").then_some(state.sum));
            counts.push((name == "count").then_some(state.count));
            if name == "kmv" {
                blobs.append_value(&state.kmv);
            } else {
                blobs.append_null();
            }
        }
    }
    let state_fields: Fields = tagged_struct_state_fields().into();
    let state = StructArray::new(
        state_fields.clone(),
        vec![
            Arc::new(StringArray::from(kind)),
            Arc::new(UInt16Array::from(versions)),
            Arc::new(Int64Array::from(sums)),
            Arc::new(UInt64Array::from(counts)),
            Arc::new(blobs.finish()),
        ],
        None,
    );
    let rows = state.len();
    RecordBatch::try_new(
        tagged_struct_schema(),
        vec![
            timestamp_array(vec![bucket; rows]),
            Arc::new(canonical.finish()),
            Arc::new(state),
        ],
    )
    .map_err(Into::into)
}

fn registry_states_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        bucket_field(),
        Field::new("xunit_id", DataType::FixedSizeBinary(32), false),
        Field::new("sum_i64_v1", DataType::Int64, false),
        Field::new("count_u64_v1", DataType::UInt64, false),
        Field::new("kmv_blob_v1", DataType::Binary, false),
    ]))
}

fn registry_states_batch(bucket: i64, cells: &BucketCells) -> Result<RecordBatch> {
    let rows = cells.len();
    let mut ids = FixedSizeBinaryBuilder::new(32);
    let mut sums = Vec::with_capacity(rows);
    let mut counts = Vec::with_capacity(rows);
    let mut blobs = BinaryBuilder::new();
    for (id, state) in cells {
        ids.append_value(id)?;
        sums.push(state.sum);
        counts.push(state.count);
        blobs.append_value(&state.kmv);
    }
    RecordBatch::try_new(
        registry_states_schema(),
        vec![
            timestamp_array(vec![bucket; rows]),
            Arc::new(ids.finish()),
            Arc::new(Int64Array::from(sums)),
            Arc::new(UInt64Array::from(counts)),
            Arc::new(blobs.finish()),
        ],
    )
    .map_err(Into::into)
}

fn registry_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("xunit_id", DataType::FixedSizeBinary(32), false),
        Field::new("xunit_canonical", DataType::Binary, false),
    ]))
}

fn bucket_field() -> Field {
    Field::new(
        "bucket_start",
        DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
        false,
    )
}

fn timestamp_array(values: Vec<i64>) -> ArrayRef {
    Arc::new(TimestampMicrosecondArray::from(values).with_timezone("+00:00"))
}

fn write_single_batch(path: &Path, schema: Arc<Schema>, batch: &RecordBatch) -> Result<()> {
    let properties = WriterProperties::builder()
        .set_max_row_group_row_count(Some(LAYOUT_ROW_GROUP_ROWS))
        .set_compression(PINNED_COMPRESSION)
        .build();
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(properties))?;
    writer.write(batch)?;
    writer.close()?;
    Ok(())
}

/// Re-reads each written layout in file order and folds it into a digest
/// without ever materializing a full layout's cells in memory: measure
/// triples (per_measure/tagged_struct) need only a single in-progress cell
/// buffer since the writer emits them contiguously; wide/registry rows are
/// already complete per row. The registry's id -> canonical map is the one
/// layout-verification structure that stays global, matching the same
/// irreducible O(distinct XUnits) bound as the write-side registry.
fn verify_layouts(output_dir: &Path, reference_digest: &str, reference_cells: usize) -> Result<()> {
    verify_layout(
        "per_measure",
        reference_digest,
        reference_cells,
        stream_digest_per_measure(&output_dir.join("per_measure.parquet"))?,
    )?;
    verify_layout(
        "wide",
        reference_digest,
        reference_cells,
        stream_digest_wide(&output_dir.join("wide.parquet"))?,
    )?;
    verify_layout(
        "tagged_struct",
        reference_digest,
        reference_cells,
        stream_digest_tagged_struct(&output_dir.join("tagged_struct.parquet"))?,
    )?;
    verify_layout(
        "xunit_registry",
        reference_digest,
        reference_cells,
        stream_digest_registry(
            &output_dir.join("registry_states.parquet"),
            &output_dir.join("xunit_registry.parquet"),
        )?,
    )?;
    Ok(())
}

fn verify_layout(
    name: &str,
    reference_digest: &str,
    reference_cells: usize,
    actual: (String, usize),
) -> Result<()> {
    let (actual_digest, actual_cells) = actual;
    ensure!(
        actual_digest == reference_digest && actual_cells == reference_cells,
        "{name} semantic mismatch: expected {reference_cells} cells with digest {reference_digest}, got {actual_cells} cells with digest {actual_digest}"
    );
    Ok(())
}

/// A cell being reassembled from contiguous measure-triple rows
/// (per_measure/tagged_struct layouts) during a single streaming pass.
struct StreamingCell {
    bucket: i64,
    id: [u8; 32],
    canonical_xunit: Vec<u8>,
    sum: Option<i64>,
    count: Option<u64>,
    kmv: Option<Vec<u8>>,
}

impl StreamingCell {
    fn start(bucket: i64, id: [u8; 32], canonical_xunit: &[u8]) -> Self {
        Self {
            bucket,
            id,
            canonical_xunit: canonical_xunit.to_vec(),
            sum: None,
            count: None,
            kmv: None,
        }
    }

    fn finish(self) -> Result<CellState> {
        let kmv = self.kmv.context("cell missing KMV state")?;
        KmvSketch::from_bytes(&kmv)?;
        Ok(CellState {
            canonical_xunit: self.canonical_xunit,
            sum: self.sum.context("cell missing sum state")?,
            count: self.count.context("cell missing count state")?,
            kmv,
        })
    }
}

fn check_monotonic(last_key: &mut Option<(i64, [u8; 32])>, key: (i64, [u8; 32])) -> Result<()> {
    if let Some(previous) = *last_key {
        ensure!(
            previous < key,
            "layout rows are not strictly ordered by (bucket, content ID)"
        );
    }
    *last_key = Some(key);
    Ok(())
}

fn stream_digest_per_measure(path: &Path) -> Result<(String, usize)> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SEMANTIC_DIGEST_DOMAIN);
    let mut cell_count = 0_usize;
    let mut last_key: Option<(i64, [u8; 32])> = None;
    let mut current: Option<StreamingCell> = None;
    for batch in read_batches(path)? {
        let buckets = typed_column::<TimestampMicrosecondArray>(&batch, "bucket_start")?;
        let canonical = typed_column::<BinaryArray>(&batch, "xunit_canonical")?;
        let measures = typed_column::<StringArray>(&batch, "measure_id")?;
        let versions = typed_column::<UInt16Array>(&batch, "state_version")?;
        let sums = typed_column::<Int64Array>(&batch, "sum_i64")?;
        let counts = typed_column::<UInt64Array>(&batch, "count_u64")?;
        let blobs = typed_column::<BinaryArray>(&batch, "state_blob")?;
        for row in 0..batch.num_rows() {
            ensure!(
                versions.value(row) == STATE_VERSION,
                "unexpected state version"
            );
            let bucket = buckets.value(row);
            let xunit = canonical.value(row);
            let id = *blake3::hash(xunit).as_bytes();
            let is_new_cell = current
                .as_ref()
                .is_none_or(|cell| cell.bucket != bucket || cell.id != id);
            if is_new_cell {
                if let Some(cell) = current.take() {
                    let (bucket, id) = (cell.bucket, cell.id);
                    check_monotonic(&mut last_key, (bucket, id))?;
                    let state = cell.finish()?;
                    digest_update(&mut hasher, bucket, &id, &state);
                    cell_count += 1;
                }
                current = Some(StreamingCell::start(bucket, id, xunit));
            } else if let Some(cell) = &current {
                ensure!(
                    cell.canonical_xunit == xunit,
                    "same cell ID has different canonical XUnit bytes"
                );
            }
            let cell = current.as_mut().expect("cell just inserted");
            match measures.value(row) {
                "sum" => cell.sum = Some(sums.value(row)),
                "count" => cell.count = Some(counts.value(row)),
                "kmv" => cell.kmv = Some(blobs.value(row).to_vec()),
                other => bail!("unknown measure ID '{other}'"),
            }
        }
    }
    if let Some(cell) = current.take() {
        let (bucket, id) = (cell.bucket, cell.id);
        check_monotonic(&mut last_key, (bucket, id))?;
        let state = cell.finish()?;
        digest_update(&mut hasher, bucket, &id, &state);
        cell_count += 1;
    }
    Ok((hasher.finalize().to_hex().to_string(), cell_count))
}

fn stream_digest_tagged_struct(path: &Path) -> Result<(String, usize)> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SEMANTIC_DIGEST_DOMAIN);
    let mut cell_count = 0_usize;
    let mut last_key: Option<(i64, [u8; 32])> = None;
    let mut current: Option<StreamingCell> = None;
    for batch in read_batches(path)? {
        let buckets = typed_column::<TimestampMicrosecondArray>(&batch, "bucket_start")?;
        let canonical = typed_column::<BinaryArray>(&batch, "xunit_canonical")?;
        let states = typed_column::<StructArray>(&batch, "state")?;
        let kinds = struct_column::<StringArray>(states, "kind")?;
        let versions = struct_column::<UInt16Array>(states, "version")?;
        let sums = struct_column::<Int64Array>(states, "sum_i64")?;
        let counts = struct_column::<UInt64Array>(states, "count_u64")?;
        let blobs = struct_column::<BinaryArray>(states, "blob")?;
        for row in 0..batch.num_rows() {
            ensure!(
                versions.value(row) == STATE_VERSION,
                "unexpected state version"
            );
            let bucket = buckets.value(row);
            let xunit = canonical.value(row);
            let id = *blake3::hash(xunit).as_bytes();
            let is_new_cell = current
                .as_ref()
                .is_none_or(|cell| cell.bucket != bucket || cell.id != id);
            if is_new_cell {
                if let Some(cell) = current.take() {
                    let (bucket, id) = (cell.bucket, cell.id);
                    check_monotonic(&mut last_key, (bucket, id))?;
                    let state = cell.finish()?;
                    digest_update(&mut hasher, bucket, &id, &state);
                    cell_count += 1;
                }
                current = Some(StreamingCell::start(bucket, id, xunit));
            } else if let Some(cell) = &current {
                ensure!(
                    cell.canonical_xunit == xunit,
                    "same cell ID has different canonical XUnit bytes"
                );
            }
            let cell = current.as_mut().expect("cell just inserted");
            match kinds.value(row) {
                "sum" => cell.sum = Some(sums.value(row)),
                "count" => cell.count = Some(counts.value(row)),
                "kmv" => cell.kmv = Some(blobs.value(row).to_vec()),
                other => bail!("unknown state kind '{other}'"),
            }
        }
    }
    if let Some(cell) = current.take() {
        let (bucket, id) = (cell.bucket, cell.id);
        check_monotonic(&mut last_key, (bucket, id))?;
        let state = cell.finish()?;
        digest_update(&mut hasher, bucket, &id, &state);
        cell_count += 1;
    }
    Ok((hasher.finalize().to_hex().to_string(), cell_count))
}

fn stream_digest_wide(path: &Path) -> Result<(String, usize)> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SEMANTIC_DIGEST_DOMAIN);
    let mut cell_count = 0_usize;
    let mut last_key: Option<(i64, [u8; 32])> = None;
    for batch in read_batches(path)? {
        let buckets = typed_column::<TimestampMicrosecondArray>(&batch, "bucket_start")?;
        let canonical = typed_column::<BinaryArray>(&batch, "xunit_canonical")?;
        let sums = typed_column::<Int64Array>(&batch, "sum_i64_v1")?;
        let counts = typed_column::<UInt64Array>(&batch, "count_u64_v1")?;
        let blobs = typed_column::<BinaryArray>(&batch, "kmv_blob_v1")?;
        for row in 0..batch.num_rows() {
            let bucket = buckets.value(row);
            let xunit = canonical.value(row);
            let kmv = blobs.value(row);
            KmvSketch::from_bytes(kmv)?;
            let id = *blake3::hash(xunit).as_bytes();
            check_monotonic(&mut last_key, (bucket, id))?;
            let state = CellState {
                canonical_xunit: xunit.to_vec(),
                sum: sums.value(row),
                count: counts.value(row),
                kmv: kmv.to_vec(),
            };
            digest_update(&mut hasher, bucket, &id, &state);
            cell_count += 1;
        }
    }
    Ok((hasher.finalize().to_hex().to_string(), cell_count))
}

fn stream_digest_registry(states_path: &Path, registry_path: &Path) -> Result<(String, usize)> {
    let mut registry = HashMap::<[u8; 32], Vec<u8>>::new();
    for batch in read_batches(registry_path)? {
        let ids = typed_column::<FixedSizeBinaryArray>(&batch, "xunit_id")?;
        let xunits = typed_column::<BinaryArray>(&batch, "xunit_canonical")?;
        for row in 0..batch.num_rows() {
            let id: [u8; 32] = ids.value(row).try_into()?;
            ensure!(
                id == *blake3::hash(xunits.value(row)).as_bytes(),
                "registry content ID does not match canonical XUnit"
            );
            ensure!(
                registry.insert(id, xunits.value(row).to_vec()).is_none(),
                "duplicate XUnit registry entry"
            );
        }
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(SEMANTIC_DIGEST_DOMAIN);
    let mut cell_count = 0_usize;
    let mut last_key: Option<(i64, [u8; 32])> = None;
    for batch in read_batches(states_path)? {
        let buckets = typed_column::<TimestampMicrosecondArray>(&batch, "bucket_start")?;
        let ids = typed_column::<FixedSizeBinaryArray>(&batch, "xunit_id")?;
        let sums = typed_column::<Int64Array>(&batch, "sum_i64_v1")?;
        let counts = typed_column::<UInt64Array>(&batch, "count_u64_v1")?;
        let blobs = typed_column::<BinaryArray>(&batch, "kmv_blob_v1")?;
        for row in 0..batch.num_rows() {
            let id: [u8; 32] = ids.value(row).try_into()?;
            let bucket = buckets.value(row);
            check_monotonic(&mut last_key, (bucket, id))?;
            let canonical = registry
                .get(&id)
                .context("state row missing XUnit registry entry")?;
            let kmv = blobs.value(row);
            KmvSketch::from_bytes(kmv)?;
            let state = CellState {
                canonical_xunit: canonical.clone(),
                sum: sums.value(row),
                count: counts.value(row),
                kmv: kmv.to_vec(),
            };
            digest_update(&mut hasher, bucket, &id, &state);
            cell_count += 1;
        }
    }
    Ok((hasher.finalize().to_hex().to_string(), cell_count))
}

fn read_batches(path: &Path) -> Result<Vec<RecordBatch>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    ParquetRecordBatchReaderBuilder::try_new(file)?
        .with_batch_size(65_536)
        .build()?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn typed_column<'a, T: 'static>(batch: &'a RecordBatch, name: &str) -> Result<&'a T> {
    batch
        .column_by_name(name)
        .with_context(|| format!("missing column '{name}'"))?
        .as_any()
        .downcast_ref::<T>()
        .with_context(|| format!("column '{name}' has unexpected Arrow type"))
}

fn struct_column<'a, T: 'static>(array: &'a StructArray, name: &str) -> Result<&'a T> {
    array
        .column_by_name(name)
        .with_context(|| format!("missing struct field '{name}'"))?
        .as_any()
        .downcast_ref::<T>()
        .with_context(|| format!("struct field '{name}' has unexpected Arrow type"))
}

fn file_len(path: &Path) -> Result<u64> {
    Ok(std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .len())
}

/// Layouts (by written filename) that carry a `bucket_start` column and are
/// therefore range-readable. `xunit_registry.parquet` (the second file
/// belonging to the `xunit_registry` layout) has no `bucket_start` column
/// and is never touched by a bucket-range scan -- its absence here *is* the
/// record of that, not an oversight.
const RANGE_READ_LAYOUTS: &[(&str, &str)] = &[
    ("per_measure", "per_measure.parquet"),
    ("wide", "wide.parquet"),
    ("tagged_struct", "tagged_struct.parquet"),
    ("xunit_registry", "registry_states.parquet"),
];

pub fn run_range_reads(config: &RangeReadConfig) -> Result<RangeReadReport> {
    ensure!(
        config.short_buckets > 0,
        "short_buckets must be greater than zero"
    );
    ensure!(
        config.long_buckets > 0,
        "long_buckets must be greater than zero"
    );
    ensure!(
        config.run_dir.is_dir(),
        "run dir does not exist: {}",
        config.run_dir.display()
    );

    let mut cases = Vec::new();
    let mut skipped = Vec::new();

    for (layout_name, file_name) in RANGE_READ_LAYOUTS {
        let path = config.run_dir.join(file_name);
        ensure!(
            path.is_file(),
            "expected layout file not found: {}",
            path.display()
        );
        let row_groups = read_bucket_row_groups(&path)?;
        let file_bytes_total = file_len(&path)?;

        for length in [RangeLength::Short, RangeLength::Long] {
            let bucket_count = match length {
                RangeLength::Short => config.short_buckets,
                RangeLength::Long => config.long_buckets,
            };
            for alignment in [RangeAlignment::Aligned, RangeAlignment::NonAligned] {
                match pick_range_start(&row_groups, bucket_count, alignment) {
                    Some(start_us) => {
                        let end_us = start_us + (bucket_count as i64 - 1) * HOUR_US;
                        let case = read_range_case(
                            layout_name,
                            &path,
                            &row_groups,
                            file_bytes_total,
                            length,
                            alignment,
                            bucket_count,
                            start_us,
                            end_us,
                        )?;
                        cases.push(case);
                    }
                    None => skipped.push(RangeReadSkipped {
                        layout: layout_name,
                        length,
                        alignment,
                        reason: format!(
                            "no {} start candidate with room for {bucket_count} buckets found among {} row groups",
                            match alignment {
                                RangeAlignment::Aligned => "aligned",
                                RangeAlignment::NonAligned => "non-aligned",
                            },
                            row_groups.len()
                        ),
                    }),
                }
            }
        }
    }

    Ok(RangeReadReport {
        run_dir: config.run_dir.display().to_string(),
        short_buckets: config.short_buckets,
        long_buckets: config.long_buckets,
        cases,
        skipped,
    })
}

/// Reads every row group's (min, max) `bucket_start` statistics (column 0 in
/// every range-readable layout schema, see `bucket_field()`) without
/// deserializing any row data.
fn read_bucket_row_groups(path: &Path) -> Result<Vec<RowGroupBucketRange>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let metadata = reader.metadata();
    let mut out = Vec::with_capacity(metadata.num_row_groups());
    for (index, row_group) in metadata.row_groups().iter().enumerate() {
        let column = row_group.column(0);
        let stats = column.statistics().with_context(|| {
            format!(
                "row group {index} of {} has no statistics on column 0 (bucket_start) -- \
                 range reads need per-row-group min/max",
                path.display()
            )
        })?;
        let (min_us, max_us) = match stats {
            parquet::file::statistics::Statistics::Int64(value_stats) => (
                *value_stats
                    .min_opt()
                    .with_context(|| format!("row group {index} bucket_start min missing"))?,
                *value_stats
                    .max_opt()
                    .with_context(|| format!("row group {index} bucket_start max missing"))?,
            ),
            other => bail!(
                "row group {index} of {}: bucket_start statistics are {:?}, expected Int64 \
                 (Arrow Timestamp(Microsecond) is stored as physical INT64 in Parquet)",
                path.display(),
                other
            ),
        };
        out.push(RowGroupBucketRange {
            index,
            min_us,
            max_us,
            num_rows: row_group.num_rows(),
            total_byte_size: row_group.total_byte_size(),
        });
    }
    Ok(out)
}

/// Picks a bucket-range start with room for `bucket_count` buckets before
/// the file ends, or `None` if no such start exists for the requested
/// alignment. See `RangeReadConfig`'s doc comment for what "aligned" means.
fn pick_range_start(
    row_groups: &[RowGroupBucketRange],
    bucket_count: u64,
    alignment: RangeAlignment,
) -> Option<i64> {
    if row_groups.is_empty() {
        return None;
    }
    let width_us = (bucket_count as i64 - 1) * HOUR_US;
    let file_end_us = row_groups.last()?.max_us;

    // Prefer a candidate roughly a quarter of the way into the file rather
    // than the very first/last row group, so the case isn't a degenerate
    // file-edge read -- fall back to the rest of the row-group list if that
    // preferred region has no match with room for the requested width.
    let preferred_start = row_groups.len() / 4;
    let ordered_indices = (preferred_start..row_groups.len()).chain(0..preferred_start);

    match alignment {
        RangeAlignment::Aligned => ordered_indices
            // A candidate is only genuinely aligned if the *preceding* row
            // group's own max doesn't also reach into it -- if
            // row_groups[i-1].max_us == row_groups[i].min_us, that earlier
            // group still contains at least one row at this exact bucket
            // value (Parquet min/max are inclusive), so a real pruning
            // reader has to scan it too, defeating "no leading waste" even
            // though row_groups[i].min_us itself matches the start exactly.
            .filter(|&i| i == 0 || row_groups[i - 1].max_us < row_groups[i].min_us)
            .map(|i| row_groups[i].min_us)
            .find(|&start_us| start_us + width_us <= file_end_us),
        RangeAlignment::NonAligned => ordered_indices
            .filter(|&i| row_groups[i].max_us > row_groups[i].min_us)
            .find_map(|i| {
                // `row_groups[i]` spans past its own minimum, so any bucket
                // strictly after `min_us` and no later than `max_us` falls
                // inside it -- making `row_groups[i]` the first group a
                // pruning reader touches for that start, with min_us < start,
                // i.e. non-aligned by this module's definition. This holds
                // regardless of whether that same bucket value also happens
                // to be some *other*, later row group's own minimum
                // elsewhere in the file -- alignment is a property of the
                // first-touched group for this query, not a file-wide
                // uniqueness check (an earlier version of this function
                // wrongly rejected exactly this case, which -- at high
                // enough row density that one bucket spans many row groups,
                // e.g. 10M+ rows per docs/TIMESERIES_PHASE_0B_HARNESS.md's
                // "Memory scaling" cell-density figures -- discarded every
                // candidate and skipped every non-aligned case).
                let candidate = row_groups[i].min_us + HOUR_US;
                let has_room = candidate + width_us <= file_end_us;
                let in_this_group = candidate <= row_groups[i].max_us;
                (has_room && in_this_group).then_some(candidate)
            }),
    }
}

/// Determines which row groups a real pruning reader would touch for
/// `[start_us, end_us]`, reads exactly those (projecting only `bucket_start`
/// to count matched vs. scanned rows cheaply -- `bytes_scanned` below comes
/// from row-group metadata instead of this projected read, since a real
/// query over a chosen layout would read whichever columns it actually
/// needs, not just `bucket_start`; row-group-level bytes is the right unit
/// for measuring pruning effectiveness independent of column projection).
#[allow(clippy::too_many_arguments)]
fn read_range_case(
    layout_name: &'static str,
    path: &Path,
    row_groups: &[RowGroupBucketRange],
    file_bytes_total: u64,
    length: RangeLength,
    alignment: RangeAlignment,
    bucket_count: u64,
    start_us: i64,
    end_us: i64,
) -> Result<RangeReadCaseMetrics> {
    let started = Instant::now();
    let scanned: Vec<&RowGroupBucketRange> = row_groups
        .iter()
        .filter(|rg| rg.max_us >= start_us && rg.min_us <= end_us)
        .collect();
    ensure!(
        !scanned.is_empty(),
        "{}: no row group overlaps [{start_us}, {end_us}] -- picked start candidate is \
         inconsistent with its own file's row groups",
        path.display()
    );
    let row_groups_scanned = scanned.len();
    let bytes_scanned: u64 = scanned
        .iter()
        .map(|rg| rg.total_byte_size.max(0) as u64)
        .sum();
    let scanned_indices: Vec<usize> = scanned.iter().map(|rg| rg.index).collect();

    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let projection = ProjectionMask::leaves(builder.parquet_schema(), [0]);
    let reader = builder
        .with_row_groups(scanned_indices)
        .with_projection(projection)
        .with_batch_size(65_536)
        .build()?;

    let mut rows_scanned = 0_usize;
    let mut rows_matched = 0_usize;
    for batch in reader {
        let batch = batch?;
        let buckets = typed_column::<TimestampMicrosecondArray>(&batch, "bucket_start")?;
        rows_scanned += batch.num_rows();
        for row in 0..batch.num_rows() {
            let value = buckets.value(row);
            if value >= start_us && value <= end_us {
                rows_matched += 1;
            }
        }
    }
    let read_ms = started.elapsed().as_millis();

    Ok(RangeReadCaseMetrics {
        layout: layout_name,
        length,
        alignment,
        bucket_count,
        start_bucket_us: start_us,
        end_bucket_us: end_us,
        row_groups_total: row_groups.len(),
        row_groups_scanned,
        file_bytes_total,
        bytes_scanned,
        rows_scanned,
        rows_matched,
        read_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubism_core::YPath;
    use tempfile::tempdir;

    /// The write path keys cells by `canonical_xunit_content_id()`
    /// (`cubism-core`); every read/verify path in this file recomputes the
    /// same key as a raw `blake3::hash()` of the canonical bytes (see
    /// issue #1, finding 5). This pins that the two are the same digest
    /// over the same bytes with no extra domain separation, so a future
    /// change to `canonical_xunit_content_id`'s internals would fail this
    /// test instead of silently desyncing the write and read keys.
    #[test]
    fn content_id_matches_raw_blake3_of_canonical_bytes() {
        let xunit = XUnit::global().with_ypath(YPath::new("device").with_attribute("id", "3"));
        let canonical = CanonicalXUnit::from(&xunit);
        let encoded = encode_canonical_xunit(&canonical).unwrap();
        let content_id = canonical_xunit_content_id(&canonical).unwrap();
        assert_eq!(content_id.as_bytes(), blake3::hash(&encoded).as_bytes());
    }

    #[test]
    fn generator_is_byte_deterministic() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first.parquet");
        let second = temp.path().join("second.parquet");
        for output in [&first, &second] {
            generate_source(&GenerateConfig {
                output: output.clone(),
                rows: 1_000,
                occupancy: Occupancy::Sparse,
                seed: 7,
                batch_rows: 256,
            })
            .unwrap();
        }
        assert_eq!(
            std::fs::read(first).unwrap(),
            std::fs::read(second).unwrap()
        );
    }

    /// Golden semantic digests captured from the pre-streaming-rewrite
    /// implementation (BTreeMap aggregation + scoped verify_layouts reads).
    /// These are the only independent oracle Phase 0B has for the
    /// aggregation/write path: a rewrite that reproduces these bytes has not
    /// silently changed semantics. Do not regenerate these values from a
    /// rewritten implementation and call it verification (see issue #1,
    /// finding 2 - self-consistency is not correctness).
    const GOLDEN_SPARSE_25K_DIGEST: &str =
        "59f8fdfdbd48f3ffa983e30c1541885b336be6f524c49d61819908b6cda021a8";
    const GOLDEN_SPARSE_25K_CELLS: usize = 51_473;
    const GOLDEN_DENSE_25K_DIGEST: &str =
        "d9683d47ee83d6c1b5c2452947f6797622acc1bfcedc450a76f1044372e172bd";
    const GOLDEN_DENSE_25K_CELLS: usize = 51_512;
    const GOLDEN_SPARSE_1M_DIGEST: &str =
        "8d5e66a9fd6e892b63ac144f1456b0d76017c36377e1be608bfd935c6c0e9620";
    const GOLDEN_SPARSE_1M_CELLS: usize = 1_936_575;

    async fn assert_golden(
        occupancy: Occupancy,
        rows: u64,
        expected_cells: usize,
        expected_digest: &str,
    ) {
        let temp = tempdir().unwrap();
        let input = temp.path().join("source.parquet");
        generate_source(&GenerateConfig {
            output: input.clone(),
            rows,
            occupancy,
            seed: 1,
            batch_rows: 65_536,
        })
        .unwrap();
        let metrics = run_rust(&RustRunConfig {
            input,
            output_dir: temp.path().join("rust"),
            target_partitions: 4,
            memory_limit_bytes: DEFAULT_DATAFUSION_MEMORY_LIMIT_BYTES,
        })
        .await
        .unwrap();
        assert_eq!(metrics.cell_count, expected_cells, "cell count drifted");
        assert_eq!(
            metrics.semantic_digest_blake3, expected_digest,
            "semantic digest drifted from the pinned pre-rewrite oracle"
        );
    }

    #[tokio::test]
    async fn golden_digest_sparse_25k() {
        assert_golden(
            Occupancy::Sparse,
            25_000,
            GOLDEN_SPARSE_25K_CELLS,
            GOLDEN_SPARSE_25K_DIGEST,
        )
        .await;
    }

    #[tokio::test]
    async fn golden_digest_dense_25k() {
        assert_golden(
            Occupancy::Dense,
            25_000,
            GOLDEN_DENSE_25K_CELLS,
            GOLDEN_DENSE_25K_DIGEST,
        )
        .await;
    }

    /// Slow (~15s release / much slower debug): run manually with
    /// `cargo test --release -p cubism-timeseries-bench -- --ignored` after
    /// the streaming rewrite to confirm the 10M-row-representative fixture
    /// still reproduces the pre-rewrite oracle.
    #[tokio::test]
    #[ignore]
    async fn golden_digest_sparse_1m() {
        assert_golden(
            Occupancy::Sparse,
            1_000_000,
            GOLDEN_SPARSE_1M_CELLS,
            GOLDEN_SPARSE_1M_DIGEST,
        )
        .await;
    }

    #[tokio::test]
    async fn smoke_fixture_round_trips_all_layouts() {
        let temp = tempdir().unwrap();
        let input = temp.path().join("source.parquet");
        generate_source(&GenerateConfig {
            output: input.clone(),
            rows: 2_500,
            occupancy: Occupancy::Dense,
            seed: 11,
            batch_rows: 512,
        })
        .unwrap();
        let metrics = run_rust(&RustRunConfig {
            input,
            output_dir: temp.path().join("rust"),
            target_partitions: 2,
            memory_limit_bytes: DEFAULT_DATAFUSION_MEMORY_LIMIT_BYTES,
        })
        .await
        .unwrap();
        assert!(metrics.cell_count > 0);
        assert_eq!(metrics.layouts.len(), 4);
        assert_eq!(metrics.semantic_digest_blake3.len(), 64);
    }

    #[tokio::test]
    async fn range_reads_are_correct_against_an_independent_full_scan() {
        let temp = tempdir().unwrap();
        let input = temp.path().join("source.parquet");
        generate_source(&GenerateConfig {
            output: input.clone(),
            rows: 200_000,
            occupancy: Occupancy::Sparse,
            seed: 3,
            batch_rows: 8_192,
        })
        .unwrap();
        let run_dir = temp.path().join("rust");
        run_rust(&RustRunConfig {
            input,
            output_dir: run_dir.clone(),
            target_partitions: 2,
            memory_limit_bytes: DEFAULT_DATAFUSION_MEMORY_LIMIT_BYTES,
        })
        .await
        .unwrap();

        let report = run_range_reads(&RangeReadConfig {
            run_dir: run_dir.clone(),
            short_buckets: 4,
            long_buckets: 48,
        })
        .unwrap();

        // Every (layout, length, alignment) combination is accounted for
        // either as a case or an explicit skip -- never silently dropped.
        assert_eq!(report.cases.len() + report.skipped.len(), 16);
        for (layout, _) in RANGE_READ_LAYOUTS {
            let accounted = report.cases.iter().any(|c| c.layout == *layout)
                || report.skipped.iter().any(|s| s.layout == *layout);
            assert!(
                accounted,
                "layout {layout} missing from the report entirely"
            );
        }

        for case in &report.cases {
            let (_, file_name) = RANGE_READ_LAYOUTS
                .iter()
                .find(|(name, _)| *name == case.layout)
                .expect("case layout must be one of RANGE_READ_LAYOUTS");
            let path = run_dir.join(file_name);

            // Cross-check rows_matched against an independent full,
            // unfiltered scan -- not by reusing the row-group-pruning code
            // path under test.
            let batches = read_batches(&path).unwrap();
            let mut expected_matched = 0_usize;
            for batch in &batches {
                let buckets =
                    typed_column::<TimestampMicrosecondArray>(batch, "bucket_start").unwrap();
                for row in 0..batch.num_rows() {
                    let value = buckets.value(row);
                    if value >= case.start_bucket_us && value <= case.end_bucket_us {
                        expected_matched += 1;
                    }
                }
            }
            assert_eq!(
                case.rows_matched, expected_matched,
                "{} {:?}/{:?}: rows_matched drifted from an independent full-file scan",
                case.layout, case.length, case.alignment
            );
            assert!(case.rows_matched <= case.rows_scanned);
            assert!(case.row_groups_scanned <= case.row_groups_total);
            assert!(case.bytes_scanned <= case.file_bytes_total);

            // The alignment property itself is about the row group a
            // pruning reader would touch *first* for this start -- not
            // whether the start value happens to equal some row group's
            // minimum *anywhere* in the file (an earlier version of this
            // test checked the latter, which is the wrong invariant and
            // didn't catch a real bug: see
            // `pick_range_start_finds_non_aligned_when_one_bucket_spans_many_row_groups`
            // below for why the two differ).
            let row_groups = read_bucket_row_groups(&path).unwrap();
            let first_scanned = row_groups
                .iter()
                .find(|rg| rg.max_us >= case.start_bucket_us && rg.min_us <= case.start_bucket_us)
                .expect("some row group must overlap the case's own start");
            match case.alignment {
                RangeAlignment::Aligned => assert_eq!(
                    first_scanned.min_us, case.start_bucket_us,
                    "{}: aligned case's start isn't the first-scanned row group's own minimum",
                    case.layout
                ),
                RangeAlignment::NonAligned => assert!(
                    first_scanned.min_us < case.start_bucket_us,
                    "{}: non-aligned case's start equals the first-scanned row group's own minimum",
                    case.layout
                ),
            }
        }
    }

    /// Regression test for a real bug found running `range-read` against a
    /// genuine 10M-row run dir (docs/TIMESERIES_PHASE_0B_NOTEBOOK.md Entry
    /// 11): at high enough row density that a single bucket's cells span
    /// many row groups (per-bucket cell count > `LAYOUT_ROW_GROUP_ROWS`,
    /// which happens well before 10M rows per
    /// `TIMESERIES_PHASE_0B_HARNESS.md`'s "Memory scaling" cell-density
    /// figures), every non-aligned case was silently skipped. The prior
    /// implementation rejected a perfectly valid non-aligned candidate
    /// merely because that same bucket value also happened to be some
    /// *other*, later row group's own minimum elsewhere in the file --
    /// alignment is a property of the first row group a query touches, not
    /// a file-wide uniqueness check. Constructs the dense regime directly
    /// (a "straddle" row group spanning exactly two buckets, surrounded by
    /// single-bucket groups) rather than needing a slow multi-million-row
    /// fixture to reach it naturally.
    #[test]
    fn pick_range_start_finds_non_aligned_when_one_bucket_spans_many_row_groups() {
        let base = BASE_BUCKET_START_US;
        let row_groups = vec![
            RowGroupBucketRange {
                index: 0,
                min_us: base,
                max_us: base,
                num_rows: 65_536,
                total_byte_size: 1_000_000,
            },
            RowGroupBucketRange {
                index: 1,
                min_us: base,
                max_us: base,
                num_rows: 65_536,
                total_byte_size: 1_000_000,
            },
            // Straddle group: the last of bucket 0's rows, then the first
            // of bucket 1's rows.
            RowGroupBucketRange {
                index: 2,
                min_us: base,
                max_us: base + HOUR_US,
                num_rows: 65_536,
                total_byte_size: 1_000_000,
            },
            RowGroupBucketRange {
                index: 3,
                min_us: base + HOUR_US,
                max_us: base + HOUR_US,
                num_rows: 65_536,
                total_byte_size: 1_000_000,
            },
            RowGroupBucketRange {
                index: 4,
                min_us: base + HOUR_US,
                max_us: base + HOUR_US,
                num_rows: 65_536,
                total_byte_size: 1_000_000,
            },
            RowGroupBucketRange {
                index: 5,
                min_us: base + 2 * HOUR_US,
                max_us: base + 2 * HOUR_US,
                num_rows: 65_536,
                total_byte_size: 1_000_000,
            },
        ];

        let start = pick_range_start(&row_groups, 1, RangeAlignment::NonAligned)
            .expect("row group 2 straddles buckets 0 and 1, so a non-aligned start must exist");
        assert_eq!(start, base + HOUR_US);

        // The regression itself: `start` legitimately equals another row
        // group's own minimum (index 3) elsewhere in the file. That must
        // NOT have disqualified it above.
        assert!(
            row_groups
                .iter()
                .any(|rg| rg.index != 2 && rg.min_us == start),
            "test fixture sanity check: index 3 should also have min_us == start"
        );
    }
}
