//! Sparse bucketed incremental (temporal) aggregation — Phase 2.
//!
//! Mirrors `build.rs`'s static pipeline (`__cubism_input` -> explode ->
//! group -> present) but:
//!
//! 1. evaluates the spec's `temporal.eventTime` expression and assigns each
//!    row to one half-open UTC bucket via [`cubism_bucket_start`], reusing
//!    `cubism_core::temporal::FixedResolution::bucket`'s exact math (the same
//!    euclidean-division code the pre-origin/boundary tests in
//!    `cubism-core` already cover) instead of re-deriving bucket arithmetic
//!    in SQL or in this crate;
//! 2. generates the *same* row-local XUnits the static path would (same
//!    `level_columns`/`cubism_xunit_keys` UDF, same lattice/rules code in
//!    `cubism-core` — see [`crate::build::level_columns`]);
//! 3. appends bucket identity to the `GROUP BY` key as an ordinary extra
//!    column, never as another XUnit dimension — a temporal build's cells
//!    are keyed by `(bucket, xunit)`, and the lattice itself never sees time;
//! 4. accumulates *authoritative*, re-mergeable state: native `SUM`/`COUNT`/
//!    `MIN`/`MAX` (already associative under DataFusion's own partial
//!    aggregation), the sketch UDAFs from `udaf.rs` for
//!    `count_distinct`/`reservoir_sample`/`centroid`, and the new state
//!    UDAFs in `state_udaf.rs` for `avg`/`variance`/`quantile` (the three
//!    kinds `build.rs`'s static engine still can't do because a bare scalar
//!    isn't mergeable);
//! 5. emits Arrow batches sorted by `(bucket_start, xunit_id)`, typed per
//!    [`temporal_state_schema`] — the Phase 0B-selected `xunit_registry`
//!    layout's shape (a `bucket_start`/`xunit_id` state table plus a
//!    separate `xunit_id -> canonical bytes` registry table), generalized
//!    from that benchmark's hardcoded sum/count/kmv measures to any spec.
//!
//! The `xunit_id -> canonical` registry is built as its own `SELECT
//! DISTINCT` pass over the exploded rows (see [`build_temporal`]), not an
//! inline process-global map: unlike the per-build XUnit *dictionary*
//! (`SharedDictionary`, whose completeness never depends on how many times a
//! UDF happens to run), a registry entry's completeness under DataFusion's
//! `Volatility::Immutable` contract would depend on every row actually
//! being invoked — an inline accumulate-as-you-go map is unsafe under
//! constant-folding/reordering and is exactly the unbounded structure this
//! phase is required to avoid. `SELECT DISTINCT` is DataFusion's own
//! spillable, bounded-by-configuration dedup instead.

use crate::build::level_columns;
use crate::state_udaf::{quantile_state_udaf_name, quantile_state_udafs, state_udafs};
use crate::udaf::sketch_udfs;
use crate::udf::{cube_udfs, SharedDictionary};
use cubism_core::encoding::{canonical_xunit_content_id, decode_xunit, encode_canonical_xunit};
use cubism_core::spec::API_VERSION_V2_ALPHA1;
use cubism_core::{
    AggKind, BucketOrigin, CanonicalXUnit, Coverage, CubeSpec, Exactness, EventTime,
    FixedResolution, MeasureSpec, Resolution, TemporalSpec, TimeRange, WindowId,
};
use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, BinaryBuilder, FixedSizeBinaryBuilder, TimestampMicrosecondArray,
};
use datafusion::arrow::datatypes::{
    DataType, Field, Schema, TimeUnit, TimestampMicrosecondType, TimestampMillisecondType,
    TimestampNanosecondType, TimestampSecondType,
};
use datafusion::common::{plan_err, DataFusionError, Result};
use datafusion::dataframe::{DataFrame, DataFrameWriteOptions};
use datafusion::execution::context::SessionContext;
use datafusion::functions_aggregate::expr_fn::{max, min};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use datafusion::prelude::col;
use std::sync::Arc;

/// How rows with a null event time are handled. Neither policy is inferred
/// silently — callers must pick one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullEventTimePolicy {
    /// Fail the whole build if any source row has a null event time.
    Reject,
    /// Drop rows with a null event time; the count is reported in
    /// [`TemporalBuildMetadata::null_event_time_rows`].
    Quarantine,
}

fn to_df_err(e: cubism_core::CubismError) -> DataFusionError {
    DataFusionError::Execution(e.to_string())
}

fn temporal_spec(spec: &CubeSpec) -> Result<&TemporalSpec> {
    spec.temporal.as_ref().ok_or_else(|| {
        DataFusionError::Plan(format!(
            "cube '{}': temporal build requires apiVersion '{API_VERSION_V2_ALPHA1}' with a \
             temporal section",
            spec.name
        ))
    })
}

fn fixed_resolution(temporal: &TemporalSpec) -> Result<FixedResolution> {
    match temporal.base_resolution {
        Resolution::Fixed(resolution) => Ok(resolution),
        Resolution::Calendar(calendar) => plan_err!(
            "calendar base resolution '{calendar}' is not supported by the temporal build yet"
        ),
    }
}

/// `{measure.name}_v{stateVersion}` — state version lives in the column
/// name, matching `registry_states_schema()`'s `sum_i64_v1`-style convention
/// in the Phase 0B benchmark harness, extended to per-measure names since a
/// general spec (unlike that harness's fixed sum/count/kmv trio) can declare
/// several measures.
fn measure_column_name(measure: &MeasureSpec) -> String {
    format!("{}_v{}", measure.name, measure.state.version.get())
}

/// The durable Arrow schema for a temporal build's state rows — the
/// authority, not an accident of SQL type inference. `bucket_start`/
/// `xunit_id` match the Phase 0B-selected `xunit_registry` layout's
/// `registry_states.parquet` shape; measure columns are Float64 (sum/min/
/// max — declared non-associative-under-bytes/floating per
/// `capabilities_for`), Int64 (count), or Binary (every state kind that
/// needs more than one scalar to stay mergeable: avg/variance/quantile and
/// the existing count_distinct/reservoir_sample/centroid sketches).
///
/// Nullability here matches what DataFusion actually produces, not what is
/// semantically guaranteed: every column is non-null in practice (a
/// `GROUP BY` cell always has at least one contributing row, so no
/// aggregate — including `bucket_start`/`xunit_id`, themselves `GROUP BY`
/// key expressions — is ever actually absent), but DataFusion's planner
/// conservatively marks `GROUP BY` keys and most aggregate outputs
/// `nullable: true` regardless; `COUNT` is the one exception it knows is
/// always non-null. This function is asserted equal (name, type, *and*
/// nullability) against the real built `DataFrame`'s schema in
/// `end_to_end_temporal_matches_hand_computed_cells_and_declared_schema` —
/// declaring stricter nullability here than DataFusion emits would make
/// that assertion permanently fail, and Parquet write uses DataFusion's
/// actual schema regardless of what this function claims.
pub fn temporal_state_schema(spec: &CubeSpec) -> Result<Arc<Schema>> {
    temporal_spec(spec)?;
    let mut fields = vec![
        Field::new(
            "bucket_start",
            DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
            true,
        ),
        Field::new("xunit_id", DataType::FixedSizeBinary(32), true),
    ];
    for measure in &spec.measures {
        let (dtype, nullable) = match measure.agg {
            AggKind::Sum | AggKind::Min | AggKind::Max => (DataType::Float64, true),
            AggKind::Count => (DataType::Int64, false),
            AggKind::Avg
            | AggKind::Variance
            | AggKind::Quantile
            | AggKind::CountDistinct
            | AggKind::ReservoirSample
            | AggKind::Centroid => (DataType::Binary, true),
            AggKind::TopK => {
                return plan_err!(
                    "measure '{}': top_k is not supported for temporal aggregation because its \
                     pruned merge is not associative",
                    measure.name
                );
            }
        };
        fields.push(Field::new(measure_column_name(measure), dtype, nullable));
    }
    Ok(Arc::new(Schema::new(fields)))
}

// ---------------------------------------------------------------------------
// cubism_bucket_start(event_time) -> Timestamp(Microsecond, "+00:00")
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Hash)]
struct BucketStartUdf {
    resolution: FixedResolution,
    origin: BucketOrigin,
    signature: Signature,
}

/// Normalize any Arrow timestamp unit to microseconds. Deliberately explicit
/// per-unit handling (never a blind `CAST ... AS BIGINT`): this repo has
/// already been bitten once by that shortcut silently returning epoch
/// seconds instead of micros (see `docs/TIMESERIES_PHASE_0B_HANDOFF.md`'s
/// Spark-adapter traps).
fn extract_timestamp_micros(array: &ArrayRef, unit: TimeUnit) -> Result<Vec<Option<i64>>> {
    let scale_up = |value: i64, factor: i64| -> Result<i64> {
        value.checked_mul(factor).ok_or_else(|| {
            DataFusionError::Execution(
                "cubism_bucket_start: event time overflows representable microseconds".into(),
            )
        })
    };
    match unit {
        TimeUnit::Second => array
            .as_primitive::<TimestampSecondType>()
            .iter()
            .map(|v| v.map(|v| scale_up(v, 1_000_000)).transpose())
            .collect(),
        TimeUnit::Millisecond => array
            .as_primitive::<TimestampMillisecondType>()
            .iter()
            .map(|v| v.map(|v| scale_up(v, 1_000)).transpose())
            .collect(),
        TimeUnit::Microsecond => {
            Ok(array.as_primitive::<TimestampMicrosecondType>().iter().collect())
        }
        // Sub-microsecond precision is discarded, not rounded: bucket
        // boundaries are never finer than microseconds, so floor-toward
        // negative-infinity (not truncation) keeps pre-epoch timestamps
        // consistent with `FixedResolution::bucket`'s own euclidean math.
        TimeUnit::Nanosecond => Ok(array
            .as_primitive::<TimestampNanosecondType>()
            .iter()
            .map(|v| v.map(|v| v.div_euclid(1_000)))
            .collect()),
    }
}

impl ScalarUDFImpl for BucketStartUdf {
    fn name(&self) -> &str {
        "cubism_bucket_start"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let input = &arrays[0];
        let DataType::Timestamp(unit, _) = input.data_type() else {
            return plan_err!(
                "cubism_bucket_start requires a TIMESTAMP column (the temporal spec's \
                 eventTime expression must produce one; cast explicitly if the source column \
                 is a raw integer), got {}",
                input.data_type()
            );
        };
        let micros = extract_timestamp_micros(input, *unit)?;
        let mut out = Vec::with_capacity(micros.len());
        for value in micros {
            let bucket_start = match value {
                None => None,
                Some(us) => Some(
                    self.resolution
                        .bucket(EventTime::from_unix_micros(us), self.origin)
                        .map_err(|e| DataFusionError::Execution(format!("cubism_bucket_start: {e}")))?
                        .start
                        .unix_micros(),
                ),
            };
            out.push(bucket_start);
        }
        let array = TimestampMicrosecondArray::from(out).with_timezone("+00:00");
        Ok(ColumnarValue::Array(Arc::new(array)))
    }
}

// ---------------------------------------------------------------------------
// cubism_xunit_content_id(key) -> FixedSizeBinary(32);
// cubism_xunit_canonical_bytes(key) -> Binary.
//
// Pure functions of (key, dictionary) — no side-effecting state. The
// dictionary->canonical registry mapping is built separately, by
// `build_temporal`'s own `SELECT DISTINCT` pass, per this module's header.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct XUnitContentIdUdf {
    dict: SharedDictionary,
    signature: Signature,
}

impl PartialEq for XUnitContentIdUdf {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.dict, &other.dict)
    }
}
impl Eq for XUnitContentIdUdf {}
impl std::hash::Hash for XUnitContentIdUdf {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.dict) as usize).hash(state);
    }
}

impl ScalarUDFImpl for XUnitContentIdUdf {
    fn name(&self) -> &str {
        "cubism_xunit_content_id"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::FixedSizeBinary(32))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let keys = arrays[0].as_binary::<i32>();
        let dict = self.dict.lock().expect("dictionary poisoned");
        let mut builder = FixedSizeBinaryBuilder::new(32);
        for row in 0..keys.len() {
            if keys.is_null(row) {
                builder.append_null();
                continue;
            }
            let xunit = decode_xunit(keys.value(row), &dict).map_err(to_df_err)?;
            let canonical = CanonicalXUnit::from(&xunit);
            let id = canonical_xunit_content_id(&canonical).map_err(to_df_err)?;
            builder
                .append_value(id.as_bytes())
                .map_err(|e| DataFusionError::Execution(e.to_string()))?;
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish())))
    }
}

#[derive(Debug)]
struct XUnitCanonicalBytesUdf {
    dict: SharedDictionary,
    signature: Signature,
}

impl PartialEq for XUnitCanonicalBytesUdf {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.dict, &other.dict)
    }
}
impl Eq for XUnitCanonicalBytesUdf {}
impl std::hash::Hash for XUnitCanonicalBytesUdf {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.dict) as usize).hash(state);
    }
}

impl ScalarUDFImpl for XUnitCanonicalBytesUdf {
    fn name(&self) -> &str {
        "cubism_xunit_canonical_bytes"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let keys = arrays[0].as_binary::<i32>();
        let dict = self.dict.lock().expect("dictionary poisoned");
        let mut builder = BinaryBuilder::new();
        for row in 0..keys.len() {
            if keys.is_null(row) {
                builder.append_null();
                continue;
            }
            let xunit = decode_xunit(keys.value(row), &dict).map_err(to_df_err)?;
            let canonical = CanonicalXUnit::from(&xunit);
            let bytes = encode_canonical_xunit(&canonical).map_err(to_df_err)?;
            builder.append_value(&bytes);
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish())))
    }
}

fn xunit_identity_udfs(dict: SharedDictionary) -> (ScalarUDF, ScalarUDF) {
    let binary_arg = || Signature::exact(vec![DataType::Binary], Volatility::Immutable);
    (
        ScalarUDF::from(XUnitContentIdUdf { dict: Arc::clone(&dict), signature: binary_arg() }),
        ScalarUDF::from(XUnitCanonicalBytesUdf { dict, signature: binary_arg() }),
    )
}

// ---------------------------------------------------------------------------
// SQL generation.
// ---------------------------------------------------------------------------

fn rfc3339_micros(event_time: EventTime) -> Result<String> {
    use chrono::SecondsFormat;
    chrono::DateTime::from_timestamp_micros(event_time.unix_micros())
        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Micros, true))
        .ok_or_else(|| {
            DataFusionError::Plan("time range bound is out of representable range".into())
        })
}

fn window_filter_sql(temporal: &TemporalSpec, window: Option<TimeRange>) -> Result<Option<String>> {
    let Some(window) = window else { return Ok(None) };
    let start = rfc3339_micros(window.start())?;
    let end = rfc3339_micros(window.end())?;
    Ok(Some(format!(
        "{event} >= TIMESTAMP '{start}' AND {event} < TIMESTAMP '{end}'",
        event = temporal.event_time
    )))
}

struct MeasureSql {
    measure_selects: Vec<String>,
    passthrough: Vec<String>,
    agg_selects: Vec<String>,
    final_cols: Vec<String>,
}

fn measure_sql(spec: &CubeSpec) -> Result<MeasureSql> {
    let mut measure_selects = Vec::new();
    let mut passthrough = Vec::new();
    let mut agg_selects = Vec::new();
    let mut final_cols = Vec::new();

    for (i, measure) in spec.measures.iter().enumerate() {
        let alias = format!("__m{i}");
        let out_col = measure_column_name(measure);
        let input = || measure.input.as_ref().expect("validated: agg requires input");
        match measure.agg {
            AggKind::Sum | AggKind::Min | AggKind::Max => {
                let f = match measure.agg {
                    AggKind::Sum => "SUM",
                    AggKind::Min => "MIN",
                    _ => "MAX",
                };
                measure_selects.push(format!("CAST({} AS DOUBLE) AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects.push(format!("{f}({alias}) AS \"{out_col}\""));
                final_cols.push(format!("\"{out_col}\""));
            }
            AggKind::Count => {
                agg_selects.push(format!("COUNT(*) AS \"{out_col}\""));
                final_cols.push(format!("\"{out_col}\""));
            }
            AggKind::Avg => {
                measure_selects.push(format!("CAST({} AS DOUBLE) AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects.push(format!("cubism_avg_state({alias}) AS \"{out_col}\""));
                final_cols.push(format!("\"{out_col}\""));
            }
            AggKind::Variance => {
                measure_selects.push(format!("CAST({} AS DOUBLE) AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects.push(format!("cubism_variance_state({alias}) AS \"{out_col}\""));
                final_cols.push(format!("\"{out_col}\""));
            }
            AggKind::Quantile => {
                measure_selects.push(format!("CAST({} AS DOUBLE) AS {alias}", input()));
                passthrough.push(alias.clone());
                let udaf = quantile_state_udaf_name(&measure.name);
                agg_selects.push(format!("{udaf}({alias}) AS \"{out_col}\""));
                final_cols.push(format!("\"{out_col}\""));
            }
            AggKind::CountDistinct => {
                measure_selects.push(format!("CAST({} AS VARCHAR) AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects.push(format!("cubism_kmv_sketch({alias}) AS \"{out_col}\""));
                final_cols.push(format!("\"{out_col}\""));
            }
            AggKind::ReservoirSample => {
                measure_selects.push(format!("CAST({} AS VARCHAR) AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects.push(format!("cubism_sample_sketch({alias}) AS \"{out_col}\""));
                final_cols.push(format!("\"{out_col}\""));
            }
            AggKind::Centroid => {
                measure_selects.push(format!("{} AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects.push(format!("cubism_centroid_sketch({alias}) AS \"{out_col}\""));
                final_cols.push(format!("\"{out_col}\""));
            }
            AggKind::TopK => {
                return plan_err!(
                    "measure '{}': top_k is not supported for temporal aggregation",
                    measure.name
                );
            }
        }
    }
    if agg_selects.is_empty() {
        return plan_err!("cube spec has no measures");
    }
    Ok(MeasureSql { measure_selects, passthrough, agg_selects, final_cols })
}

/// Everything [`pipeline_ctes`] needs beyond `(temporal, source, window,
/// null_policy)` — the level/measure projection columns shared with the
/// aggregation stage that only [`temporal_build_sql`] appends.
struct PipelineColumns<'a> {
    measure_selects: &'a [String],
    level_selects: &'a [String],
    level_args: &'a [String],
    measure_passthrough: &'a [String],
}

/// The `__cubism_time` / `__cubism_bucketed` / `__cubism_exploded` CTE
/// chain shared by [`temporal_build_sql`] (which appends the aggregation
/// stage) and [`exploded_only_sql`] (used to derive the XUnit registry).
/// Returns the CTE bodies (no leading `WITH`, no trailing `SELECT`).
fn pipeline_ctes(
    temporal: &TemporalSpec,
    source: &str,
    window: Option<TimeRange>,
    null_policy: NullEventTimePolicy,
    columns: PipelineColumns<'_>,
) -> Result<String> {
    let PipelineColumns { measure_selects, level_selects, level_args, measure_passthrough } =
        columns;
    let event_expr = &temporal.event_time;
    let window_clause = match window_filter_sql(temporal, window)? {
        Some(clause) => format!("\n  WHERE {clause}"),
        None => String::new(),
    };
    let quarantine_clause = match null_policy {
        NullEventTimePolicy::Quarantine => "\n  WHERE __event_time IS NOT NULL".to_string(),
        NullEventTimePolicy::Reject => String::new(),
    };

    let input_cols: Vec<String> = std::iter::once(format!("{event_expr} AS __event_time"))
        .chain(level_selects.iter().cloned())
        .chain(measure_selects.iter().cloned())
        .collect();
    let bucketed_passthrough: String =
        level_args.iter().chain(measure_passthrough).map(|a| format!(", {a}")).collect();
    let exploded_passthrough: String =
        measure_passthrough.iter().map(|a| format!(", {a}")).collect();

    Ok(format!(
        "__cubism_time AS (\n  SELECT {input}\n  FROM {source}{window_clause}\n),\n\
         __cubism_bucketed AS (\n  SELECT cubism_bucket_start(__event_time) AS __bucket_start{bucketed_passthrough}\n  \
         FROM __cubism_time{quarantine_clause}\n),\n\
         __cubism_exploded AS (\n  SELECT __bucket_start, unnest(cubism_xunit_keys({args})) AS __xunit_key{exploded_passthrough}\n  \
         FROM __cubism_bucketed\n)",
        input = input_cols.join(", "),
        args = level_args.join(", "),
    ))
}

/// Generate the temporal-build SQL for a spec (exposed for inspection/tests,
/// mirroring `build::cube_sql`).
pub fn temporal_build_sql(
    spec: &CubeSpec,
    source: &str,
    window: Option<TimeRange>,
    null_policy: NullEventTimePolicy,
) -> Result<String> {
    let temporal = temporal_spec(spec)?;
    let (level_selects, level_args) = level_columns(spec);
    let measures = measure_sql(spec)?;
    let ctes = pipeline_ctes(
        temporal,
        source,
        window,
        null_policy,
        PipelineColumns {
            measure_selects: &measures.measure_selects,
            level_selects: &level_selects,
            level_args: &level_args,
            measure_passthrough: &measures.passthrough,
        },
    )?;

    Ok(format!(
        "WITH {ctes},\n\
         __cubism_cells AS (\n  SELECT __bucket_start, __xunit_key, {aggs}\n  FROM __cubism_exploded\n  \
         GROUP BY __bucket_start, __xunit_key\n)\n\
         SELECT __bucket_start AS bucket_start, cubism_xunit_content_id(__xunit_key) AS xunit_id, {finals}\n\
         FROM __cubism_cells\n\
         ORDER BY bucket_start, xunit_id",
        aggs = measures.agg_selects.join(", "),
        finals = measures.final_cols.join(", "),
    ))
}

/// A standalone query producing every row-local XUnit key this build would
/// explode a row into, before grouping — the basis for the XUnit registry
/// (`SELECT DISTINCT` over this) and for the sparse-row-count property test.
fn exploded_only_sql(
    spec: &CubeSpec,
    source: &str,
    window: Option<TimeRange>,
    null_policy: NullEventTimePolicy,
) -> Result<String> {
    let temporal = temporal_spec(spec)?;
    let (level_selects, level_args) = level_columns(spec);
    let measures = measure_sql(spec)?;
    let ctes = pipeline_ctes(
        temporal,
        source,
        window,
        null_policy,
        PipelineColumns {
            measure_selects: &measures.measure_selects,
            level_selects: &level_selects,
            level_args: &level_args,
            measure_passthrough: &measures.passthrough,
        },
    )?;
    Ok(format!("WITH {ctes}\nSELECT __bucket_start, __xunit_key FROM __cubism_exploded"))
}

async fn null_event_time_count(ctx: &SessionContext, temporal: &TemporalSpec, source: &str) -> Result<u64> {
    let sql =
        format!("SELECT COUNT(*) AS n FROM {source} WHERE {event} IS NULL", event = temporal.event_time);
    let batches = ctx.sql(&sql).await?.collect().await?;
    let mut total: i64 = 0;
    for batch in &batches {
        let col = batch.column(0).as_primitive::<datafusion::arrow::datatypes::Int64Type>();
        for i in 0..col.len() {
            total += col.value(i);
        }
    }
    Ok(total.max(0) as u64)
}

// ---------------------------------------------------------------------------
// Public build entry point.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TemporalBuildMetadata {
    pub window: Option<TimeRange>,
    /// Caller-assigned identity for this build (idempotency/publication
    /// tracking in Phase 3); not interpreted or validated here.
    pub window_id: Option<WindowId>,
    /// Rows whose event time was null: 0 under `Reject` (the build fails
    /// instead), the drop count under `Quarantine`.
    pub null_event_time_rows: u64,
    /// `None` when no `window` was requested — [`Coverage::requested`] needs
    /// a [`TimeRange`] to assess coverage against, and an unbounded build
    /// has none. `Some` otherwise, computed by [`compute_coverage`]: a cheap
    /// `MIN`/`MAX(bucket_start)` scan, *not* a per-bucket occupancy check.
    /// **`Exactness::Exact` means the observed data reaches both edges of
    /// the requested window — it is not a claim that every bucket in
    /// between is populated.** Two events near the edges of a 30-day window
    /// with nothing in between still report `Exact`; a caller gating
    /// publication on this needs a separate occupancy check if an
    /// interior gap matters to it. `covered` is at most one element (the
    /// `[MIN(bucket_start), MAX(bucket_start) + resolution)` span, clamped
    /// to `requested`'s own edges since bucket assignment is independent of
    /// the window and would otherwise claim coverage of time the window
    /// filter guaranteed has zero rows); this function never reports
    /// multiple disjoint covered ranges. Deliberately *not* derived from
    /// `MIN`/`MAX(event_time)` over the filtered source instead, which
    /// would avoid materializing `states` below — a row can pass the
    /// window's event-time filter and still contribute zero state rows if
    /// filter rules prune all of its XUnits, so source-derived bounds would
    /// overclaim at the edges exactly like the unclamped bucket bounds did.
    pub coverage: Option<Coverage>,
}

pub struct TemporalBuildOutput {
    /// `(bucket_start, xunit_id, <measure state columns>)`, sorted, typed
    /// per [`temporal_state_schema`].
    pub states: DataFrame,
    /// `(xunit_id, xunit_canonical)`, one row per distinct XUnit this build
    /// observed.
    pub registry: DataFrame,
    pub dictionary: SharedDictionary,
    pub metadata: TemporalBuildMetadata,
}

/// Build one bounded temporal window's state batches from source events.
///
/// Registers this build's UDFs/UDAFs on `ctx` under fixed names
/// (`cubism_bucket_start`, `cubism_xunit_keys`, ...). Safe to call
/// repeatedly on the same context *sequentially* (each call's `ctx.sql()`
/// binds to whatever is registered at that moment, and DataFusion resolves
/// function names to their concrete `Arc` at planning time, not at
/// `collect()` time) but not concurrently — two overlapping calls would
/// race on which registration a given `ctx.sql()` observes. Use one context
/// per build if calling from concurrent tasks.
pub async fn build_temporal(
    ctx: &SessionContext,
    spec: &CubeSpec,
    source: &str,
    window: Option<TimeRange>,
    window_id: Option<WindowId>,
    null_policy: NullEventTimePolicy,
) -> Result<TemporalBuildOutput> {
    spec.validate().map_err(|e| DataFusionError::Plan(e.to_string()))?;
    let temporal = temporal_spec(spec)?;
    let resolution = fixed_resolution(temporal)?;

    let null_count = null_event_time_count(ctx, temporal, source).await?;
    if null_count > 0 && null_policy == NullEventTimePolicy::Reject {
        return plan_err!(
            "cube '{}': {null_count} source row(s) have a null event time; pass \
             NullEventTimePolicy::Quarantine to drop them instead of failing the build",
            spec.name
        );
    }

    let (keys_udf, decode_udf, dict) = cube_udfs(spec);
    ctx.register_udf(keys_udf);
    ctx.register_udf(decode_udf);
    ctx.register_udf(ScalarUDF::from(BucketStartUdf {
        resolution,
        origin: temporal.origin,
        signature: Signature::any(1, Volatility::Immutable),
    }));
    let (content_id_udf, canonical_bytes_udf) = xunit_identity_udfs(Arc::clone(&dict));
    ctx.register_udf(content_id_udf);
    ctx.register_udf(canonical_bytes_udf);

    let (sketch_udafs, sketch_presenters) = sketch_udfs();
    for udaf in sketch_udafs {
        ctx.register_udaf(udaf);
    }
    for udf in sketch_presenters {
        ctx.register_udf(udf);
    }
    let (avg_var_udafs, avg_var_presenters) = state_udafs();
    for udaf in avg_var_udafs {
        ctx.register_udaf(udaf);
    }
    for udf in avg_var_presenters {
        ctx.register_udf(udf);
    }
    let (quantile_udafs, quantile_presenters) = quantile_state_udafs(spec)?;
    for udaf in quantile_udafs {
        ctx.register_udaf(udaf);
    }
    for udf in quantile_presenters {
        ctx.register_udf(udf);
    }

    let states_sql = temporal_build_sql(spec, source, window, null_policy)?;
    let states = ctx.sql(&states_sql).await?;

    // Computing `Coverage` requires knowing the actual observed bucket
    // range, which means executing `states` here rather than leaving it a
    // lazy plan the caller executes on their own `.collect()` (the
    // documented behavior for an unwindowed build, see
    // `bucket_start_rejects_a_non_timestamp_event_time_column`). `.cache()`
    // executes once and hands back a `DataFrame` over the materialized
    // result, so this coverage query and the caller's own `.collect()` on
    // the returned `states` don't re-run the aggregation twice.
    let (states, coverage) = match window {
        Some(window) => {
            let cached = states.cache().await?;
            let coverage = compute_coverage(&cached, window, resolution).await?;
            (cached, Some(coverage))
        }
        None => (states, None),
    };

    let exploded_sql = exploded_only_sql(spec, source, window, null_policy)?;
    let registry_sql = format!(
        "SELECT DISTINCT cubism_xunit_content_id(__xunit_key) AS xunit_id, \
         cubism_xunit_canonical_bytes(__xunit_key) AS xunit_canonical \
         FROM ({exploded_sql}) AS __cubism_registry_source"
    );
    let registry = ctx.sql(&registry_sql).await?;

    Ok(TemporalBuildOutput {
        states,
        registry,
        dictionary: dict,
        metadata: TemporalBuildMetadata { window, window_id, null_event_time_rows: null_count, coverage },
    })
}

/// `requested`'s coverage, derived from the built states' observed
/// `MIN`/`MAX(bucket_start)` — see [`TemporalBuildMetadata::coverage`] for
/// what this does and doesn't detect. `cached` must already be materialized
/// (via `DataFrame::cache`) so this doesn't re-run the aggregation.
async fn compute_coverage(
    cached: &DataFrame,
    requested: TimeRange,
    resolution: FixedResolution,
) -> Result<Coverage> {
    let bounds = cached.clone().aggregate(
        vec![],
        vec![min(col("bucket_start")).alias("min_bucket"), max(col("bucket_start")).alias("max_bucket")],
    )?;
    let batches = bounds.collect().await?;
    let batch = &batches[0];
    let min_col = batch.column(0).as_primitive::<TimestampMicrosecondType>();
    let max_col = batch.column(1).as_primitive::<TimestampMicrosecondType>();

    if min_col.is_null(0) {
        return Ok(Coverage {
            requested,
            covered: vec![],
            exactness: Exactness::Inexact("no source rows observed in the requested window".into()),
        });
    }

    // Buckets are assigned from `origin`, independent of `requested`'s
    // edges — a non-bucket-aligned window's edge bucket extends past the
    // filter that produced these rows (`window_filter_sql` filters on
    // *event time*, not bucket bounds). Clamping to `requested` keeps
    // `covered` from claiming coverage of time the filter guaranteed has
    // zero rows.
    let observed_end_micros = max_col.value(0).checked_add(resolution.micros()).ok_or_else(|| {
        DataFusionError::Execution("covered bucket end overflows i64 microseconds".into())
    })?;
    let covered_start_micros = min_col.value(0).max(requested.start().unix_micros());
    let covered_end_micros = observed_end_micros.min(requested.end().unix_micros());
    let covered = TimeRange::new(
        EventTime::from_unix_micros(covered_start_micros),
        EventTime::from_unix_micros(covered_end_micros),
    )
    .map_err(to_df_err)?;

    let exactness = if covered == requested {
        Exactness::Exact
    } else {
        // `covered`'s bounds are already clamped to `requested`, so report
        // the *unclamped* observed bucket span here — otherwise this
        // message would just restate `covered`/`requested` and hide the
        // actual reason for the gap (data short of an edge vs. a bucket
        // that overhangs one).
        Exactness::Inexact(format!(
            "observed buckets span [{}, {}) unix-micros; requested window is [{}, {})",
            min_col.value(0),
            observed_end_micros,
            requested.start().unix_micros(),
            requested.end().unix_micros(),
        ))
    };

    Ok(Coverage { requested, covered: vec![covered], exactness })
}

/// Write a build's two tables to local Parquet fixtures (no Iceberg
/// publication — that is Phase 3). `states_path` matches
/// `registry_states.parquet`, `registry_path` matches `xunit_registry.parquet`
/// in the Phase 0B naming.
pub async fn write_temporal_fixtures(
    output: &TemporalBuildOutput,
    states_path: &str,
    registry_path: &str,
) -> Result<()> {
    let single_file = || DataFrameWriteOptions::new().with_single_file_output(true);
    output.states.clone().write_parquet(states_path, single_file(), None).await?;
    output.registry.clone().write_parquet(registry_path, single_file(), None).await?;
    Ok(())
}

/// Dry-run report: runs the real build (no fixture writes) and reports the
/// counts a CLI `--explain` flag would show.
#[derive(Debug, Clone)]
pub struct TemporalBuildExplain {
    pub source_rows: usize,
    pub generated_xunits: usize,
    pub observed_buckets: usize,
    pub output_rows: usize,
    pub null_event_time_rows: u64,
}

pub async fn explain_temporal_build(
    ctx: &SessionContext,
    spec: &CubeSpec,
    source: &str,
    window: Option<TimeRange>,
    null_policy: NullEventTimePolicy,
) -> Result<TemporalBuildExplain> {
    let temporal = temporal_spec(spec)?;
    let output = build_temporal(ctx, spec, source, window, None, null_policy).await?;

    let states_batches = output.states.clone().collect().await?;
    let output_rows: usize = states_batches.iter().map(|b| b.num_rows()).sum();
    let mut buckets = std::collections::BTreeSet::new();
    for batch in &states_batches {
        let col = batch.column(0).as_primitive::<datafusion::arrow::datatypes::TimestampMicrosecondType>();
        for i in 0..col.len() {
            buckets.insert(col.value(i));
        }
    }

    let window_clause = match window_filter_sql(temporal, window)? {
        Some(clause) => format!(" WHERE {clause}"),
        None => String::new(),
    };
    let source_rows_sql = format!("SELECT COUNT(*) AS n FROM {source}{window_clause}");
    let source_rows = scalar_count(ctx, &source_rows_sql).await?;

    let exploded_sql = exploded_only_sql(spec, source, window, null_policy)?;
    let generated_xunits =
        scalar_count(ctx, &format!("SELECT COUNT(*) AS n FROM ({exploded_sql}) AS t")).await?;

    Ok(TemporalBuildExplain {
        source_rows,
        generated_xunits,
        observed_buckets: buckets.len(),
        output_rows,
        null_event_time_rows: output.metadata.null_event_time_rows,
    })
}

async fn scalar_count(ctx: &SessionContext, sql: &str) -> Result<usize> {
    let batches = ctx.sql(sql).await?.collect().await?;
    let mut total: i64 = 0;
    for batch in &batches {
        let col = batch.column(0).as_primitive::<datafusion::arrow::datatypes::Int64Type>();
        for i in 0..col.len() {
            total += col.value(i);
        }
    }
    Ok(total.max(0) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubism_core::lattice::generate_xunits;
    use cubism_core::{AggregateState, AverageState};
    use datafusion::arrow::array::{FixedSizeBinaryArray, Int64Array, StringArray};
    use datafusion::arrow::datatypes::{Float64Type, Int64Type};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::execution::context::SessionContext;
    use proptest::prelude::*;
    use std::collections::{HashMap, HashSet};

    const HOUR_US: i64 = 3_600_000_000;

    fn content_id_of(xunit_str: &str) -> [u8; 32] {
        let xunit: cubism_core::XUnit = xunit_str.parse().unwrap();
        let canonical = CanonicalXUnit::from(&xunit);
        *canonical_xunit_content_id(&canonical).unwrap().as_bytes()
    }

    // -- schema authority --------------------------------------------------

    #[test]
    fn temporal_state_schema_names_and_types_measures_by_kind() {
        let spec = CubeSpec::from_yaml(
            r#"
apiVersion: cubism/v2alpha1
name: shapes
dimensions:
  - name: device
measures:
  - name: total
    agg: sum
    input: amount
  - name: events
    agg: count
  - name: mean
    agg: avg
    input: amount
  - name: spread
    agg: variance
    input: amount
  - name: p50
    agg: quantile
    input: amount
    state:
      quantileBinWidth: 1.0
  - name: reach
    agg: count_distinct
    input: entity_id
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
"#,
        )
        .unwrap();
        let schema = temporal_state_schema(&spec).unwrap();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names,
            vec!["bucket_start", "xunit_id", "total_v1", "events_v1", "mean_v1", "spread_v1", "p50_v1", "reach_v1"]
        );
        let types: Vec<&DataType> = schema.fields().iter().map(|f| f.data_type()).collect();
        assert_eq!(
            types,
            vec![
                &DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
                &DataType::FixedSizeBinary(32),
                &DataType::Float64, // sum
                &DataType::Int64,   // count
                &DataType::Binary,  // avg
                &DataType::Binary,  // variance
                &DataType::Binary,  // quantile
                &DataType::Binary,  // count_distinct
            ]
        );
    }

    #[test]
    fn top_k_is_rejected_for_temporal_schema_and_sql() {
        let spec = CubeSpec::from_yaml(
            r#"
apiVersion: cubism/v2alpha1
name: rejects_topk
dimensions:
  - name: device
measures:
  - name: top
    agg: top_k
    input: page
    by: score
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
"#,
        );
        // top_k is already rejected at spec validation time for temporal
        // specs (see cubism-core's spec.rs); this asserts that contract
        // still holds rather than re-testing it here.
        assert!(spec.unwrap_err().to_string().contains("top_k is not supported"));
    }

    // -- end-to-end hand-computed cells --------------------------------------

    const E2E_TEMPORAL_SPEC: &str = r#"
apiVersion: cubism/v2alpha1
name: e2e_temporal
dimensions:
  - name: device
measures:
  - name: amount_sum
    agg: sum
    input: amount
  - name: events
    agg: count
  - name: amount_avg
    agg: avg
    input: amount
  - name: reach
    agg: count_distinct
    input: entity_id
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
includeGlobal: true
"#;

    fn e2e_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "occurred_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
                false,
            ),
            Field::new("device", DataType::Utf8, false),
            Field::new("amount", DataType::Int64, false),
            Field::new("entity_id", DataType::Utf8, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(
                    TimestampMicrosecondArray::from(vec![0i64, 1_000, HOUR_US, HOUR_US + 500])
                        .with_timezone("+00:00"),
                ),
                Arc::new(StringArray::from(vec!["mobile", "mobile", "desktop", "mobile"])),
                Arc::new(Int64Array::from(vec![10, 20, 40, 80])),
                Arc::new(StringArray::from(vec!["u1", "u2", "u1", "u3"])),
            ],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn end_to_end_temporal_matches_hand_computed_cells_and_declared_schema() {
        let spec = CubeSpec::from_yaml(E2E_TEMPORAL_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", e2e_batch()).unwrap();
        let output =
            build_temporal(&ctx, &spec, "events", None, None, NullEventTimePolicy::Reject).await.unwrap();

        // The DataFrame's actual Arrow schema must equal the declared
        // authority exactly — name, type, *and* nullability.
        let expected = temporal_state_schema(&spec).unwrap();
        let actual: Vec<(String, DataType, bool)> = output
            .states
            .schema()
            .fields()
            .iter()
            .map(|f| (f.name().to_string(), f.data_type().clone(), f.is_nullable()))
            .collect();
        let wanted: Vec<(String, DataType, bool)> = expected
            .fields()
            .iter()
            .map(|f| (f.name().clone(), f.data_type().clone(), f.is_nullable()))
            .collect();
        assert_eq!(actual, wanted);

        type Cell = (f64, i64, Vec<u8>, Vec<u8>);
        let batches = output.states.collect().await.unwrap();

        // "Emits sorted Arrow record batches": (bucket_start, xunit_id)
        // ascending, across the whole stream, not just within one batch.
        let mut order: Vec<(i64, [u8; 32])> = Vec::new();
        let mut cells: HashMap<(i64, [u8; 32]), Cell> = HashMap::new();
        for batch in &batches {
            let bucket = batch.column(0).as_primitive::<TimestampMicrosecondType>();
            let ids = batch.column(1).as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
            let sum = batch.column(2).as_primitive::<Float64Type>();
            let count = batch.column(3).as_primitive::<Int64Type>();
            let avg_blob = batch.column(4).as_binary::<i32>();
            let reach_blob = batch.column(5).as_binary::<i32>();
            for i in 0..batch.num_rows() {
                let id: [u8; 32] = ids.value(i).try_into().unwrap();
                order.push((bucket.value(i), id));
                cells.insert(
                    (bucket.value(i), id),
                    (
                        sum.value(i),
                        count.value(i),
                        avg_blob.value(i).to_vec(),
                        reach_blob.value(i).to_vec(),
                    ),
                );
            }
        }
        let mut sorted_order = order.clone();
        sorted_order.sort();
        assert_eq!(order, sorted_order, "states must be sorted by (bucket_start, xunit_id)");
        assert_eq!(cells.len(), 5, "bucket0: {{/G, mobile}}, bucket1: {{/G, mobile, desktop}}");

        let global_id = content_id_of("/G");
        let mobile_id = content_id_of("/device/device=mobile");
        let desktop_id = content_id_of("/device/device=desktop");

        let (sum, count, avg, reach) = &cells[&(0, global_id)];
        assert_eq!((*sum, *count), (30.0, 2));
        assert_eq!(cubism_core::AverageState::decode(avg).unwrap().present(), Some(15.0));
        assert_eq!(cubism_core::KmvSketch::from_bytes(reach).unwrap().estimate(), 2.0);

        let (sum, count, _, _) = &cells[&(0, mobile_id)];
        assert_eq!((*sum, *count), (30.0, 2));

        let (sum, count, avg, reach) = &cells[&(HOUR_US, global_id)];
        assert_eq!((*sum, *count), (120.0, 2));
        assert_eq!(cubism_core::AverageState::decode(avg).unwrap().present(), Some(60.0));
        assert_eq!(cubism_core::KmvSketch::from_bytes(reach).unwrap().estimate(), 2.0);

        let (sum, count, _, _) = &cells[&(HOUR_US, desktop_id)];
        assert_eq!((*sum, *count), (40.0, 1));
        let (sum, count, _, _) = &cells[&(HOUR_US, mobile_id)];
        assert_eq!((*sum, *count), (80.0, 1));

        let registry_batches = output.registry.collect().await.unwrap();
        let mut registry_ids: HashSet<[u8; 32]> = HashSet::new();
        for batch in &registry_batches {
            let ids = batch.column(0).as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
            for i in 0..batch.num_rows() {
                registry_ids.insert(ids.value(i).try_into().unwrap());
            }
        }
        assert_eq!(registry_ids, HashSet::from([global_id, mobile_id, desktop_id]));
    }

    #[tokio::test]
    async fn retrying_the_build_is_idempotent() {
        let spec = CubeSpec::from_yaml(E2E_TEMPORAL_SPEC).unwrap();

        async fn run(spec: &CubeSpec) -> Vec<(i64, [u8; 32], i64)> {
            let ctx = SessionContext::new();
            ctx.register_batch("events", e2e_batch()).unwrap();
            let output =
                build_temporal(&ctx, spec, "events", None, None, NullEventTimePolicy::Reject).await.unwrap();
            let batches = output.states.collect().await.unwrap();
            let mut out = Vec::new();
            for batch in &batches {
                let bucket = batch.column(0).as_primitive::<TimestampMicrosecondType>();
                let ids = batch.column(1).as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
                let count = batch.column(3).as_primitive::<Int64Type>();
                for i in 0..batch.num_rows() {
                    out.push((bucket.value(i), ids.value(i).try_into().unwrap(), count.value(i)));
                }
            }
            out
        }

        assert_eq!(run(&spec).await, run(&spec).await);
    }

    // -- coverage -------------------------------------------------------------

    #[tokio::test]
    async fn coverage_is_none_without_a_window() {
        let spec = CubeSpec::from_yaml(E2E_TEMPORAL_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", e2e_batch()).unwrap();
        let output =
            build_temporal(&ctx, &spec, "events", None, None, NullEventTimePolicy::Reject).await.unwrap();
        assert!(output.metadata.coverage.is_none());
    }

    #[tokio::test]
    async fn coverage_is_exact_when_the_observed_bucket_span_equals_the_requested_window() {
        let spec = CubeSpec::from_yaml(E2E_TEMPORAL_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", e2e_batch()).unwrap();
        // Fixture events span [0, HOUR_US + 500), i.e. buckets [0, HOUR_US)
        // and [HOUR_US, 2*HOUR_US) — request exactly that span.
        let window =
            TimeRange::new(EventTime::from_unix_micros(0), EventTime::from_unix_micros(2 * HOUR_US))
                .unwrap();
        let output = build_temporal(&ctx, &spec, "events", Some(window), None, NullEventTimePolicy::Reject)
            .await
            .unwrap();
        let coverage = output.metadata.coverage.unwrap();
        assert_eq!(coverage.requested, window);
        assert_eq!(coverage.covered, vec![window]);
        assert_eq!(coverage.exactness, Exactness::Exact);
    }

    #[tokio::test]
    async fn coverage_is_inexact_when_observed_data_does_not_reach_the_requested_edges() {
        let spec = CubeSpec::from_yaml(E2E_TEMPORAL_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", e2e_batch()).unwrap();
        // Request a window wider than the fixture's actual event span.
        let window = TimeRange::new(
            EventTime::from_unix_micros(-HOUR_US),
            EventTime::from_unix_micros(3 * HOUR_US),
        )
        .unwrap();
        let output = build_temporal(&ctx, &spec, "events", Some(window), None, NullEventTimePolicy::Reject)
            .await
            .unwrap();
        let coverage = output.metadata.coverage.unwrap();
        assert_eq!(coverage.requested, window);
        let expected_covered =
            TimeRange::new(EventTime::from_unix_micros(0), EventTime::from_unix_micros(2 * HOUR_US))
                .unwrap();
        assert_eq!(coverage.covered, vec![expected_covered]);
        assert!(matches!(coverage.exactness, Exactness::Inexact(_)));
    }

    #[tokio::test]
    async fn coverage_clamps_the_observed_bucket_span_to_a_non_aligned_window() {
        // Bucket assignment is independent of the requested window's edges
        // (`window_filter_sql` filters on event time, not bucket bounds), so
        // a window whose edges fall inside a bucket rather than on its
        // boundary must not report coverage of time the filter guaranteed
        // has zero rows.
        let spec = CubeSpec::from_yaml(E2E_TEMPORAL_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", e2e_batch()).unwrap();
        // [30min, 90min): only the two ~1h events pass the filter, both in
        // bucket [1h, 2h) — whose natural bucket end (2h) is past the
        // window's end (90min).
        let window = TimeRange::new(
            EventTime::from_unix_micros(HOUR_US / 2),
            EventTime::from_unix_micros(3 * HOUR_US / 2),
        )
        .unwrap();
        let output = build_temporal(&ctx, &spec, "events", Some(window), None, NullEventTimePolicy::Reject)
            .await
            .unwrap();
        let coverage = output.metadata.coverage.unwrap();
        let covered = coverage.covered[0];
        assert_eq!(covered.end(), window.end(), "must not overclaim past the requested window's end");
    }

    #[tokio::test]
    async fn coverage_is_exact_for_a_non_bucket_aligned_window_fully_spanned_by_data() {
        let spec = CubeSpec::from_yaml(E2E_TEMPORAL_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", e2e_batch()).unwrap();
        // [1h+100us, 2h): only the event at 1h+500us passes the filter, in
        // bucket [1h, 2h) — whose natural bucket start (1h) is before the
        // window's start. Clamping should still report `Exact` since the
        // filtered rows fully span the (narrower) requested window.
        let window = TimeRange::new(
            EventTime::from_unix_micros(HOUR_US + 100),
            EventTime::from_unix_micros(2 * HOUR_US),
        )
        .unwrap();
        let output = build_temporal(&ctx, &spec, "events", Some(window), None, NullEventTimePolicy::Reject)
            .await
            .unwrap();
        let coverage = output.metadata.coverage.unwrap();
        assert_eq!(coverage.covered, vec![window]);
        assert_eq!(coverage.exactness, Exactness::Exact);
    }

    #[tokio::test]
    async fn coverage_reports_no_rows_observed_when_the_window_matches_no_source_rows() {
        let spec = CubeSpec::from_yaml(E2E_TEMPORAL_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", e2e_batch()).unwrap();
        let window = TimeRange::new(
            EventTime::from_unix_micros(10 * HOUR_US),
            EventTime::from_unix_micros(11 * HOUR_US),
        )
        .unwrap();
        let output = build_temporal(&ctx, &spec, "events", Some(window), None, NullEventTimePolicy::Reject)
            .await
            .unwrap();
        let coverage = output.metadata.coverage.unwrap();
        assert_eq!(coverage.covered, vec![]);
        assert!(matches!(coverage.exactness, Exactness::Inexact(_)));
    }

    #[tokio::test]
    async fn write_temporal_fixtures_writes_single_readable_parquet_files() {
        use datafusion::prelude::ParquetReadOptions;

        let spec = CubeSpec::from_yaml(E2E_TEMPORAL_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", e2e_batch()).unwrap();
        let output =
            build_temporal(&ctx, &spec, "events", None, None, NullEventTimePolicy::Reject).await.unwrap();

        let dir = std::env::temp_dir()
            .join(format!("cubism_temporal_fixture_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let states_path = dir.join("registry_states.parquet");
        let registry_path = dir.join("xunit_registry.parquet");

        write_temporal_fixtures(&output, states_path.to_str().unwrap(), registry_path.to_str().unwrap())
            .await
            .unwrap();

        // `DataFrame::write_parquet` writes a directory of part files unless
        // single-file output is requested — this is the one thing a unit
        // test on the DataFrame API alone can't catch, so it's asserted
        // against the real filesystem result.
        assert!(
            states_path.is_file(),
            "expected a single states file, dir contents: {:?}",
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect::<Vec<_>>()
        );
        assert!(
            registry_path.is_file(),
            "expected a single registry file, dir contents: {:?}",
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect::<Vec<_>>()
        );

        let read_ctx = SessionContext::new();
        read_ctx
            .register_parquet("states_readback", states_path.to_str().unwrap(), ParquetReadOptions::default())
            .await
            .unwrap();
        read_ctx
            .register_parquet(
                "registry_readback",
                registry_path.to_str().unwrap(),
                ParquetReadOptions::default(),
            )
            .await
            .unwrap();
        let states_rows = scalar_count(&read_ctx, "SELECT COUNT(*) AS n FROM states_readback").await.unwrap();
        let registry_rows =
            scalar_count(&read_ctx, "SELECT COUNT(*) AS n FROM registry_readback").await.unwrap();
        assert_eq!(states_rows, 5);
        assert_eq!(registry_rows, 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    // -- null event time policy ---------------------------------------------

    #[tokio::test]
    async fn null_event_time_reject_errors_and_quarantine_drops_rows() {
        const SPEC: &str = r#"
apiVersion: cubism/v2alpha1
name: null_policy
dimensions:
  - name: device
measures:
  - name: events
    agg: count
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
includeGlobal: true
"#;
        let spec = CubeSpec::from_yaml(SPEC).unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "occurred_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
                true,
            ),
            Field::new("device", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(
                    TimestampMicrosecondArray::from(vec![Some(0i64), None, Some(HOUR_US)])
                        .with_timezone("+00:00"),
                ),
                Arc::new(StringArray::from(vec!["mobile", "desktop", "mobile"])),
            ],
        )
        .unwrap();

        let ctx_reject = SessionContext::new();
        ctx_reject.register_batch("events", batch.clone()).unwrap();
        let result = build_temporal(&ctx_reject, &spec, "events", None, None, NullEventTimePolicy::Reject).await;
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected the build to reject a null event time"),
        };
        assert!(err.to_string().contains("null event time"));

        let ctx_quarantine = SessionContext::new();
        ctx_quarantine.register_batch("events", batch).unwrap();
        let output = build_temporal(
            &ctx_quarantine,
            &spec,
            "events",
            None,
            None,
            NullEventTimePolicy::Quarantine,
        )
        .await
        .unwrap();
        assert_eq!(output.metadata.null_event_time_rows, 1);
        let rows: usize = output.states.collect().await.unwrap().iter().map(|b| b.num_rows()).sum();
        // bucket0={mobile}: {/G, mobile}; bucket1={mobile}: {/G, mobile} = 4.
        assert_eq!(rows, 4);
    }

    // -- bucket boundaries ----------------------------------------------------

    #[tokio::test]
    async fn event_at_bucket_boundary_goes_to_the_correct_half_open_bucket() {
        const SPEC: &str = r#"
apiVersion: cubism/v2alpha1
name: boundary
dimensions:
  - name: device
measures:
  - name: events
    agg: count
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
"#;
        let spec = CubeSpec::from_yaml(SPEC).unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "occurred_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
                false,
            ),
            Field::new("device", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(
                    TimestampMicrosecondArray::from(vec![HOUR_US - 1, HOUR_US]).with_timezone("+00:00"),
                ),
                Arc::new(StringArray::from(vec!["mobile", "mobile"])),
            ],
        )
        .unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", batch).unwrap();
        let output =
            build_temporal(&ctx, &spec, "events", None, None, NullEventTimePolicy::Reject).await.unwrap();
        let batches = output.states.collect().await.unwrap();
        let mut buckets: Vec<i64> = Vec::new();
        for batch in &batches {
            let col = batch.column(0).as_primitive::<TimestampMicrosecondType>();
            for i in 0..col.len() {
                buckets.push(col.value(i));
            }
        }
        buckets.sort_unstable();
        assert_eq!(buckets, vec![0, HOUR_US]);
    }

    #[tokio::test]
    async fn bucket_start_normalizes_every_timestamp_unit_to_the_same_bucket() {
        use datafusion::arrow::array::{
            TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray,
        };

        const SPEC: &str = r#"
apiVersion: cubism/v2alpha1
name: unit_normalization
dimensions:
  - name: device
measures:
  - name: events
    agg: count
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
"#;
        let spec = CubeSpec::from_yaml(SPEC).unwrap();
        // A pre-origin (negative) instant 90 minutes before epoch and one
        // exactly at 90 minutes after: bucket -2h and bucket 1h respectively,
        // regardless of the source column's timestamp unit.
        let event_seconds = -90 * 60_i64;
        let event_at_90m_seconds = 90 * 60_i64;

        async fn run_for_unit(
            spec: &CubeSpec,
            column: ArrayRef,
        ) -> Vec<i64> {
            let schema = Arc::new(Schema::new(vec![
                Field::new("occurred_at", column.data_type().clone(), false),
                Field::new("device", DataType::Utf8, false),
            ]));
            let batch = RecordBatch::try_new(
                schema,
                vec![column, Arc::new(StringArray::from(vec!["mobile", "mobile"]))],
            )
            .unwrap();
            let ctx = SessionContext::new();
            ctx.register_batch("events", batch).unwrap();
            let output = build_temporal(&ctx, spec, "events", None, None, NullEventTimePolicy::Reject)
                .await
                .unwrap();
            let batches = output.states.collect().await.unwrap();
            let mut buckets: Vec<i64> = Vec::new();
            for batch in &batches {
                let col = batch.column(0).as_primitive::<TimestampMicrosecondType>();
                for i in 0..col.len() {
                    buckets.push(col.value(i));
                }
            }
            buckets.sort_unstable();
            buckets.dedup();
            buckets
        }

        let expected = vec![-2 * HOUR_US, HOUR_US];

        let seconds: ArrayRef = Arc::new(
            TimestampSecondArray::from(vec![event_seconds, event_at_90m_seconds])
                .with_timezone("+00:00"),
        );
        assert_eq!(run_for_unit(&spec, seconds).await, expected, "seconds");

        let millis: ArrayRef = Arc::new(
            TimestampMillisecondArray::from(vec![event_seconds * 1_000, event_at_90m_seconds * 1_000])
                .with_timezone("+00:00"),
        );
        assert_eq!(run_for_unit(&spec, millis).await, expected, "milliseconds");

        let micros: ArrayRef = Arc::new(
            TimestampMicrosecondArray::from(vec![
                event_seconds * 1_000_000,
                event_at_90m_seconds * 1_000_000,
            ])
            .with_timezone("+00:00"),
        );
        assert_eq!(run_for_unit(&spec, micros).await, expected, "microseconds");

        let nanos: ArrayRef = Arc::new(
            TimestampNanosecondArray::from(vec![
                event_seconds * 1_000_000_000,
                event_at_90m_seconds * 1_000_000_000,
            ])
            .with_timezone("+00:00"),
        );
        assert_eq!(run_for_unit(&spec, nanos).await, expected, "nanoseconds");
    }

    #[tokio::test]
    async fn bucket_start_rejects_a_non_timestamp_event_time_column() {
        const SPEC: &str = r#"
apiVersion: cubism/v2alpha1
name: rejects_non_timestamp
dimensions:
  - name: device
measures:
  - name: events
    agg: count
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
"#;
        let spec = CubeSpec::from_yaml(SPEC).unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("occurred_at", DataType::Int64, false),
            Field::new("device", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Int64Array::from(vec![0i64])), Arc::new(StringArray::from(vec!["mobile"]))],
        )
        .unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", batch).unwrap();
        // `build_temporal` only builds the (lazy) logical plan — DataFusion
        // doesn't call a UDF's `invoke_with_args` until physical execution,
        // so the type mismatch only surfaces on `.collect()`.
        let output = build_temporal(&ctx, &spec, "events", None, None, NullEventTimePolicy::Reject)
            .await
            .unwrap();
        let result = output.states.collect().await;
        let err = match result {
            Err(e) => e,
            Ok(batches) => panic!("expected rejection, got {} batches: {batches:?}", batches.len()),
        };
        assert!(err.to_string().contains("requires a TIMESTAMP column"));
    }

    // -- filter rules prune identically in both paths ------------------------

    #[tokio::test]
    async fn filter_rules_prune_identically_static_and_temporal() {
        const TEMPORAL: &str = r#"
apiVersion: cubism/v2alpha1
name: pruned_temporal
dimensions:
  - name: device
  - name: region
filterRules:
  - type: max_dimensions
    n: 1
measures:
  - name: events
    agg: count
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
includeGlobal: true
"#;
        const STATIC: &str = r#"
apiVersion: v1
name: pruned_static
dimensions:
  - name: device
  - name: region
filterRules:
  - type: max_dimensions
    n: 1
measures:
  - name: events
    agg: count
includeGlobal: true
"#;
        fn make_batch() -> RecordBatch {
            let schema = Arc::new(Schema::new(vec![
                Field::new(
                    "occurred_at",
                    DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
                    false,
                ),
                Field::new("device", DataType::Utf8, false),
                Field::new("region", DataType::Utf8, false),
            ]));
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(
                        TimestampMicrosecondArray::from(vec![0i64, 1_000, HOUR_US])
                            .with_timezone("+00:00"),
                    ),
                    Arc::new(StringArray::from(vec!["mobile", "desktop", "mobile"])),
                    Arc::new(StringArray::from(vec!["us", "us", "eu"])),
                ],
            )
            .unwrap()
        }

        let static_spec = CubeSpec::from_yaml(STATIC).unwrap();
        let ctx1 = SessionContext::new();
        ctx1.register_batch("events", make_batch()).unwrap();
        let (df, _dict) = crate::build_cube(&ctx1, &static_spec, "events").await.unwrap();
        let static_batches = df.collect().await.unwrap();
        let mut static_ids: HashSet<[u8; 32]> = HashSet::new();
        for batch in &static_batches {
            let xunit = batch.column(0).as_string::<i32>();
            for i in 0..batch.num_rows() {
                static_ids.insert(content_id_of(xunit.value(i)));
            }
        }

        let temporal_spec = CubeSpec::from_yaml(TEMPORAL).unwrap();
        let ctx2 = SessionContext::new();
        ctx2.register_batch("events", make_batch()).unwrap();
        let output = build_temporal(&ctx2, &temporal_spec, "events", None, None, NullEventTimePolicy::Reject)
            .await
            .unwrap();
        let registry_batches = output.registry.collect().await.unwrap();
        let mut temporal_ids: HashSet<[u8; 32]> = HashSet::new();
        for batch in &registry_batches {
            let ids = batch.column(0).as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
            for i in 0..batch.num_rows() {
                temporal_ids.insert(ids.value(i).try_into().unwrap());
            }
        }

        assert_eq!(static_ids, temporal_ids);
        // max_dimensions=1 forbids cross cells: /G, device={mobile,desktop}, region={us,eu}.
        assert_eq!(static_ids.len(), 5);
    }

    // -- property: sparse row count is bounded by row-local XUnits ----------

    #[tokio::test]
    async fn sparse_row_count_is_bounded_by_row_local_xunits_not_cartesian() {
        const SPARSE_SPEC: &str = r#"
apiVersion: cubism/v2alpha1
name: sparse_bound
dimensions:
  - name: device
  - name: region
measures:
  - name: events
    agg: count
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
includeGlobal: true
"#;
        let spec = CubeSpec::from_yaml(SPARSE_SPEC).unwrap();
        let device_count = 20usize;
        let region_count = 20usize;
        let row_count = 40usize;

        let mut ts = Vec::with_capacity(row_count);
        let mut devices = Vec::with_capacity(row_count);
        let mut regions = Vec::with_capacity(row_count);
        for i in 0..row_count {
            ts.push(0i64);
            devices.push(format!("device_{}", i % device_count));
            regions.push(format!("region_{}", (i * 7) % region_count));
        }

        // Oracle computed directly from the same lattice-generation code
        // `cubism_xunit_keys` calls internally (`generate_xunits` +
        // `DimensionSpec::ypaths_for_values`) — not a re-derived
        // approximation, per "reuse lattice/rules code directly".
        let dims = spec.sorted_dimensions();
        let mut oracle: HashSet<String> = HashSet::new();
        for i in 0..row_count {
            let per_dimension: Vec<Vec<cubism_core::YPath>> = dims
                .iter()
                .map(|d| {
                    let value =
                        if d.name == "device" { devices[i].clone() } else { regions[i].clone() };
                    d.ypaths_for_values(&[Some(value)])
                })
                .collect();
            for xunit in generate_xunits(&per_dimension, &spec.filter_rules, spec.include_global) {
                oracle.insert(xunit.to_string());
            }
        }

        let cartesian_bound = 1 + device_count + region_count + device_count * region_count;
        assert!(
            oracle.len() < cartesian_bound,
            "oracle {} rows should be far below the dense Cartesian bound {cartesian_bound}",
            oracle.len()
        );
        assert!(
            oracle.len() <= row_count * 4,
            "oracle {} rows should be bounded by observed row-local XUnits (<= {}), not \
             distinct-value combinations",
            oracle.len(),
            row_count * 4
        );

        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "occurred_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
                false,
            ),
            Field::new("device", DataType::Utf8, false),
            Field::new("region", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampMicrosecondArray::from(ts).with_timezone("+00:00")),
                Arc::new(StringArray::from(devices)),
                Arc::new(StringArray::from(regions)),
            ],
        )
        .unwrap();

        let ctx = SessionContext::new();
        ctx.register_batch("events", batch).unwrap();
        let output =
            build_temporal(&ctx, &spec, "events", None, None, NullEventTimePolicy::Reject).await.unwrap();
        let produced_rows: usize =
            output.states.collect().await.unwrap().iter().map(|b| b.num_rows()).sum();
        assert_eq!(produced_rows, oracle.len());
    }

    // -- property: temporal output without bucket identity equals a lawful
    // -- merge of the static output.
    //
    // Uses integer sum/count measures for exact (not tolerance-based)
    // comparison, per the plan's "byte-equivalent or semantically
    // identical" disjunction; static's xunit is a string, temporal's is a
    // content ID, so both are normalized to content IDs before comparing.

    const PT_TEMPORAL_SPEC: &str = r#"
apiVersion: cubism/v2alpha1
name: pt_temporal
dimensions:
  - name: device
  - name: region
measures:
  - name: total
    agg: sum
    input: amount
  - name: events
    agg: count
  - name: mean
    agg: avg
    input: amount
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
includeGlobal: true
"#;

    const PT_STATIC_SPEC: &str = r#"
apiVersion: v1
name: pt_static
dimensions:
  - name: device
  - name: region
measures:
  - name: total
    agg: sum
    input: amount
  - name: events
    agg: count
  - name: mean
    agg: avg
    input: amount
includeGlobal: true
"#;

    fn pt_batch(rows: &[(i64, &str, &str, i64)]) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "occurred_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
                false,
            ),
            Field::new("device", DataType::Utf8, false),
            Field::new("region", DataType::Utf8, false),
            Field::new("amount", DataType::Int64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(
                    TimestampMicrosecondArray::from(rows.iter().map(|r| r.0).collect::<Vec<_>>())
                        .with_timezone("+00:00"),
                ),
                Arc::new(StringArray::from(rows.iter().map(|r| r.1).collect::<Vec<_>>())),
                Arc::new(StringArray::from(rows.iter().map(|r| r.2).collect::<Vec<_>>())),
                Arc::new(Int64Array::from(rows.iter().map(|r| r.3).collect::<Vec<_>>())),
            ],
        )
        .unwrap()
    }

    async fn pt_static_totals(rows: &[(i64, &str, &str, i64)]) -> HashMap<[u8; 32], (f64, i64, f64)> {
        let spec = CubeSpec::from_yaml(PT_STATIC_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", pt_batch(rows)).unwrap();
        let (df, _dict) = crate::build_cube(&ctx, &spec, "events").await.unwrap();
        let batches = df.collect().await.unwrap();
        let mut out = HashMap::new();
        for batch in &batches {
            let xunit = batch.column(0).as_string::<i32>();
            let total = batch.column(1).as_primitive::<Int64Type>();
            let events = batch.column(2).as_primitive::<Int64Type>();
            let mean = batch.column(3).as_primitive::<Float64Type>();
            for i in 0..batch.num_rows() {
                out.insert(
                    content_id_of(xunit.value(i)),
                    (total.value(i) as f64, events.value(i), mean.value(i)),
                );
            }
        }
        out
    }

    async fn pt_temporal_totals(
        rows: &[(i64, &str, &str, i64)],
    ) -> HashMap<[u8; 32], (f64, i64, AverageState)> {
        let spec = CubeSpec::from_yaml(PT_TEMPORAL_SPEC).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", pt_batch(rows)).unwrap();
        let output =
            build_temporal(&ctx, &spec, "events", None, None, NullEventTimePolicy::Reject).await.unwrap();
        let batches = output.states.collect().await.unwrap();
        let mut out: HashMap<[u8; 32], (f64, i64, AverageState)> = HashMap::new();
        for batch in &batches {
            let ids = batch.column(1).as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
            let total = batch.column(2).as_primitive::<Float64Type>();
            let events = batch.column(3).as_primitive::<Int64Type>();
            let mean_blob = batch.column(4).as_binary::<i32>();
            for i in 0..batch.num_rows() {
                let id: [u8; 32] = ids.value(i).try_into().unwrap();
                let entry = out.entry(id).or_insert((0.0, 0, AverageState::new()));
                entry.0 += total.value(i);
                entry.1 += events.value(i);
                let decoded = AverageState::decode(mean_blob.value(i)).unwrap();
                entry.2 = entry.2.merge(&decoded).unwrap();
            }
        }
        out
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, .. ProptestConfig::default() })]
        #[test]
        fn temporal_merge_without_bucket_equals_static(
            rows in prop::collection::vec(
                (0i64..(4 * HOUR_US), 0usize..3, 0usize..3, 1i64..100),
                1..15,
            )
        ) {
            let devices = ["mobile", "desktop", "tablet"];
            let regions = ["us", "eu", "apac"];
            let materialized: Vec<(i64, &str, &str, i64)> =
                rows.iter().map(|&(t, d, r, amount)| (t, devices[d], regions[r], amount)).collect();

            let runtime = tokio::runtime::Runtime::new().unwrap();
            let (static_map, temporal_map) = runtime.block_on(async {
                (pt_static_totals(&materialized).await, pt_temporal_totals(&materialized).await)
            });

            prop_assert_eq!(static_map.len(), temporal_map.len());
            for (id, (sum, count, mean)) in &static_map {
                let (t_sum, t_count, t_mean_state) =
                    temporal_map.get(id).expect("every static xunit must appear in the temporal merge");
                prop_assert_eq!(*sum, *t_sum);
                prop_assert_eq!(*count, *t_count);
                let t_mean = t_mean_state.present().expect("a group always has at least one row");
                prop_assert!(
                    (*mean - t_mean).abs() < 1e-9,
                    "avg mismatch: static={mean} temporal (merged across buckets)={t_mean}"
                );
            }
        }
    }
}
