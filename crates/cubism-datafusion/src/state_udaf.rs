//! Authoritative merge-state UDAFs for temporal aggregation.
//!
//! `build.rs`'s static engine presents `AVG`/`VARIANCE`/`QUANTILE` as plain
//! scalars (or refuses them outright: `cube_sql` errors on `Variance` and
//! `Quantile` today) because a bare scalar cannot be re-merged after two
//! independent partial builds. These UDAFs close that gap by making the
//! DataFusion accumulator's state **be**
//! `cubism_core::aggregate_state::AggregateState`: `state()`/`evaluate()`
//! emit `encode()` bytes, and `merge_batch` decodes each incoming blob and
//! calls `merge()`. That is exactly the pattern `udaf.rs`'s sketch UDAFs
//! (`cubism_kmv_sketch`, `cubism_topk_sketch`, ...) already use for
//! `CountDistinct`/`TopK`/`ReservoirSample`/`Centroid` — this module extends
//! it to the three MVP aggregate kinds Phase 1 gave a state contract to but
//! Phase 0's static engine never implemented.
//!
//! `GroupsAccumulator` is deliberately not implemented here (unlike
//! `udaf.rs`'s sketch kernels): DataFusion's `GroupsAccumulatorAdapter`
//! fallback handles correctness for free, and "one multi-measure accumulator
//! versus measure-specific plans" is an explicit unresolved decision for
//! Phase 2 (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md`). `udaf.rs` measured a
//! 19x cost for skipping the vectorized path on a high-cardinality GROUP
//! BY — expect a similar gap here until that decision is made.

use cubism_core::{AggKind, AggregateState, AverageState, CubeSpec, QuantileState, VarianceState};
use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Builder, StringBuilder};
use datafusion::arrow::datatypes::{DataType, Field, FieldRef};
use datafusion::common::{Result, ScalarValue, exec_err};
use datafusion::logical_expr::function::{AccumulatorArgs, StateFieldsArgs};
use datafusion::logical_expr::{
    Accumulator, AggregateUDF, AggregateUDFImpl, ColumnarValue, ScalarFunctionArgs, ScalarUDF,
    ScalarUDFImpl, Signature, Volatility,
};
use std::sync::Arc;

fn to_df_err(e: cubism_core::CubismError) -> datafusion::common::DataFusionError {
    datafusion::common::DataFusionError::Execution(e.to_string())
}

fn iter_blobs(array: &ArrayRef) -> impl Iterator<Item = &[u8]> {
    let arr = array.as_binary::<i32>();
    (0..arr.len())
        .filter(|&i| !arr.is_null(i))
        .map(move |i| arr.value(i))
}

fn f64_input(values: &[ArrayRef]) -> impl Iterator<Item = f64> + '_ {
    values[0]
        .as_primitive::<datafusion::arrow::datatypes::Float64Type>()
        .iter()
        .flatten()
}

// ---------------------------------------------------------------------------
// Average: cubism_avg_state(x DOUBLE) -> Binary (AverageState::encode());
//          cubism_avg_present(blob) -> DOUBLE mean, NULL if empty.
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Hash)]
struct AverageStateUdaf {
    signature: Signature,
}

impl AggregateUDFImpl for AverageStateUdaf {
    fn name(&self) -> &str {
        "cubism_avg_state"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(AverageAccumulator {
            state: AverageState::new(),
        }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(
            format!("{}[avg]", args.name),
            DataType::Binary,
            true,
        ))])
    }
}

#[derive(Debug)]
struct AverageAccumulator {
    state: AverageState,
}

impl Accumulator for AverageAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        for v in f64_input(values) {
            self.state.accumulate(v).map_err(to_df_err)?;
        }
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        for blob in iter_blobs(&states[0]) {
            let other = AverageState::decode(blob).map_err(to_df_err)?;
            self.state = self.state.merge(&other).map_err(to_df_err)?;
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Binary(Some(self.state.encode()))])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Binary(Some(self.state.encode())))
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct AveragePresentUdf {
    signature: Signature,
}

impl ScalarUDFImpl for AveragePresentUdf {
    fn name(&self) -> &str {
        "cubism_avg_present"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Float64)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let blobs = arrays[0].as_binary::<i32>();
        let mut out = Float64Builder::with_capacity(blobs.len());
        for i in 0..blobs.len() {
            if blobs.is_null(i) {
                out.append_null();
                continue;
            }
            match AverageState::decode(blobs.value(i)) {
                Ok(state) => out.append_option(state.present()),
                Err(e) => return exec_err!("{e}"),
            }
        }
        Ok(ColumnarValue::Array(Arc::new(out.finish())))
    }
}

// ---------------------------------------------------------------------------
// Variance: cubism_variance_state(x DOUBLE) -> Binary (VarianceState::encode());
//           cubism_variance_present(blob) -> JSON {count,mean,populationVariance,sampleVariance}.
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Hash)]
struct VarianceStateUdaf {
    signature: Signature,
}

impl AggregateUDFImpl for VarianceStateUdaf {
    fn name(&self) -> &str {
        "cubism_variance_state"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(VarianceAccumulator {
            state: VarianceState::new(),
        }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(
            format!("{}[variance]", args.name),
            DataType::Binary,
            true,
        ))])
    }
}

#[derive(Debug)]
struct VarianceAccumulator {
    state: VarianceState,
}

impl Accumulator for VarianceAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        for v in f64_input(values) {
            self.state.accumulate(v).map_err(to_df_err)?;
        }
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        for blob in iter_blobs(&states[0]) {
            let other = VarianceState::decode(blob).map_err(to_df_err)?;
            self.state = self.state.merge(&other).map_err(to_df_err)?;
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Binary(Some(self.state.encode()))])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Binary(Some(self.state.encode())))
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct VariancePresentUdf {
    signature: Signature,
}

impl ScalarUDFImpl for VariancePresentUdf {
    fn name(&self) -> &str {
        "cubism_variance_present"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let blobs = arrays[0].as_binary::<i32>();
        let mut out = StringBuilder::new();
        for i in 0..blobs.len() {
            if blobs.is_null(i) {
                out.append_null();
                continue;
            }
            match VarianceState::decode(blobs.value(i)) {
                Ok(state) => match state.present() {
                    Some(presentation) => out.append_value(
                        serde_json::to_string(&presentation)
                            .expect("VariancePresentation serialization is infallible"),
                    ),
                    None => out.append_null(),
                },
                Err(e) => return exec_err!("{e}"),
            }
        }
        Ok(ColumnarValue::Array(Arc::new(out.finish())))
    }
}

// ---------------------------------------------------------------------------
// Quantile: cubism_quantile_state__<measure>(x DOUBLE) -> Binary
//   (QuantileState::encode(), one UDAF per measure since bin width is a
//   spec-level constant, not a per-row argument);
//   cubism_quantile_present__<measure>(blob) -> DOUBLE median.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct QuantileStateUdaf {
    name: String,
    bin_width: f64,
    signature: Signature,
}

// bin_width is a validated, finite f64 constant fixed at construction; two
// instances are the same function iff they share a name and bin width.
impl PartialEq for QuantileStateUdaf {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.bin_width.to_bits() == other.bin_width.to_bits()
    }
}
impl Eq for QuantileStateUdaf {}
impl std::hash::Hash for QuantileStateUdaf {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.bin_width.to_bits().hash(state);
    }
}

impl AggregateUDFImpl for QuantileStateUdaf {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(QuantileAccumulator {
            state: QuantileState::new(self.bin_width).map_err(to_df_err)?,
        }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(
            format!("{}[quantile]", args.name),
            DataType::Binary,
            true,
        ))])
    }
}

#[derive(Debug)]
struct QuantileAccumulator {
    state: QuantileState,
}

impl Accumulator for QuantileAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        for v in f64_input(values) {
            self.state.accumulate(v).map_err(to_df_err)?;
        }
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        for blob in iter_blobs(&states[0]) {
            let other = QuantileState::decode(blob).map_err(to_df_err)?;
            self.state = self.state.merge(&other).map_err(to_df_err)?;
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Binary(Some(self.state.encode()))])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Binary(Some(self.state.encode())))
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>() + self.state.bin_count() * 16
    }
}

#[derive(Debug)]
struct QuantilePresentUdf {
    name: String,
    signature: Signature,
}

impl PartialEq for QuantilePresentUdf {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl Eq for QuantilePresentUdf {}
impl std::hash::Hash for QuantilePresentUdf {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl ScalarUDFImpl for QuantilePresentUdf {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Float64)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let blobs = arrays[0].as_binary::<i32>();
        let probability = 0.5;
        let mut out = Float64Builder::with_capacity(blobs.len());
        for i in 0..blobs.len() {
            if blobs.is_null(i) {
                out.append_null();
                continue;
            }
            match QuantileState::decode(blobs.value(i)).and_then(|s| s.quantile(probability)) {
                Ok(value) => out.append_option(value),
                Err(e) => return exec_err!("{e}"),
            }
        }
        Ok(ColumnarValue::Array(Arc::new(out.finish())))
    }
}

/// Average/variance UDAFs (plus presenters): fixed shape, always safe to
/// register once regardless of whether the spec actually uses them (mirrors
/// `udaf::sketch_udfs`'s unconditional registration).
pub fn state_udafs() -> (Vec<AggregateUDF>, Vec<ScalarUDF>) {
    let double_arg = || Signature::exact(vec![DataType::Float64], Volatility::Immutable);
    let blob_arg = || Signature::exact(vec![DataType::Binary], Volatility::Immutable);
    (
        vec![
            AggregateUDF::from(AverageStateUdaf {
                signature: double_arg(),
            }),
            AggregateUDF::from(VarianceStateUdaf {
                signature: double_arg(),
            }),
        ],
        vec![
            ScalarUDF::from(AveragePresentUdf {
                signature: blob_arg(),
            }),
            ScalarUDF::from(VariancePresentUdf {
                signature: blob_arg(),
            }),
        ],
    )
}

/// SQL function name for a `Quantile` measure's per-measure state UDAF.
pub fn quantile_state_udaf_name(measure_name: &str) -> String {
    format!("cubism_quantile_state__{measure_name}")
}

/// SQL function name for a `Quantile` measure's per-measure presenter UDF.
pub fn quantile_present_udf_name(measure_name: &str) -> String {
    format!("cubism_quantile_present__{measure_name}")
}

/// One (UDAF, presenter) pair per `Quantile` measure in `spec` — bin width is
/// a spec-level constant baked into the UDAF at construction, not a SQL
/// argument, so measures with different bin widths never collide.
pub fn quantile_state_udafs(spec: &CubeSpec) -> Result<(Vec<AggregateUDF>, Vec<ScalarUDF>)> {
    let blob_arg = || Signature::exact(vec![DataType::Binary], Volatility::Immutable);
    let double_arg = || Signature::exact(vec![DataType::Float64], Volatility::Immutable);
    let mut udafs = Vec::new();
    let mut presenters = Vec::new();
    for measure in &spec.measures {
        if measure.agg != AggKind::Quantile {
            continue;
        }
        let bin_width = measure.state.quantile_bin_width.ok_or_else(|| {
            datafusion::common::DataFusionError::Plan(format!(
                "measure '{}': quantile requires state.quantileBinWidth",
                measure.name
            ))
        })?;
        udafs.push(AggregateUDF::from(QuantileStateUdaf {
            name: quantile_state_udaf_name(&measure.name),
            bin_width,
            signature: double_arg(),
        }));
        presenters.push(ScalarUDF::from(QuantilePresentUdf {
            name: quantile_present_udf_name(&measure.name),
            signature: blob_arg(),
        }));
    }
    Ok((udafs, presenters))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubism_core::CubeSpec;
    use datafusion::arrow::array::Float64Array;
    use datafusion::arrow::datatypes::Schema;
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::execution::context::SessionContext;

    async fn run_agg(udaf_name: &str, values: &[f64]) -> Vec<u8> {
        let ctx = SessionContext::new();
        let (udafs, _presenters) = state_udafs();
        for udaf in udafs {
            ctx.register_udaf(udaf);
        }
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Float64, false)]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Float64Array::from(values.to_vec()))])
                .unwrap();
        ctx.register_batch("t", batch).unwrap();
        let out = ctx
            .sql(&format!("SELECT {udaf_name}(x) AS s FROM t"))
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        let batch = &out[0];
        let blobs = batch.column(0).as_binary::<i32>();
        blobs.value(0).to_vec()
    }

    #[tokio::test]
    async fn average_state_accumulates_and_round_trips() {
        let blob = run_agg("cubism_avg_state", &[1.0, 2.0, 6.0]).await;
        let state = AverageState::decode(&blob).unwrap();
        assert_eq!(state.present(), Some(3.0));
    }

    #[tokio::test]
    async fn variance_state_accumulates_and_round_trips() {
        let blob = run_agg("cubism_variance_state", &[1.0, 2.0, 3.0]).await;
        let state = VarianceState::decode(&blob).unwrap();
        let presentation = state.present().unwrap();
        assert_eq!(presentation.count, 3);
        assert!((presentation.mean - 2.0).abs() < 1e-12);
    }

    #[test]
    fn average_merge_across_partial_states_matches_single_pass() {
        let mut left = AverageState::new();
        let mut right = AverageState::new();
        let mut direct = AverageState::new();
        for v in [1.0, 2.0, 3.0] {
            left.accumulate(v).unwrap();
            direct.accumulate(v).unwrap();
        }
        for v in [10.0, 20.0] {
            right.accumulate(v).unwrap();
            direct.accumulate(v).unwrap();
        }
        let merged = AverageState::decode(&left.encode())
            .unwrap()
            .merge(&AverageState::decode(&right.encode()).unwrap())
            .unwrap();
        assert_eq!(merged.present(), direct.present());
    }

    #[test]
    fn quantile_udaf_names_are_namespaced_per_measure() {
        assert_eq!(
            quantile_state_udaf_name("latency_p50"),
            "cubism_quantile_state__latency_p50"
        );
        assert_eq!(
            quantile_present_udf_name("latency_p50"),
            "cubism_quantile_present__latency_p50"
        );
    }

    #[test]
    fn quantile_state_udafs_builds_one_pair_per_measure_with_distinct_bin_widths() {
        let spec = CubeSpec::from_yaml(
            r#"
apiVersion: cubism/v2alpha1
name: t
dimensions:
  - name: geo
measures:
  - name: p50
    agg: quantile
    input: latency
    state:
      quantileBinWidth: 0.5
  - name: p99
    agg: quantile
    input: latency
    state:
      quantileBinWidth: 5.0
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 0s
"#,
        )
        .unwrap();
        let (udafs, presenters) = quantile_state_udafs(&spec).unwrap();
        assert_eq!(udafs.len(), 2);
        assert_eq!(presenters.len(), 2);
        let names: Vec<&str> = udafs.iter().map(|u| u.name()).collect();
        assert!(names.contains(&"cubism_quantile_state__p50"));
        assert!(names.contains(&"cubism_quantile_state__p99"));
    }
}
