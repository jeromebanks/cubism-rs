//! Sketch aggregate UDFs.
//!
//! `cubism_kmv_sketch(value)` aggregates values into a KMV sketch and emits
//! the **serialized sketch blob** — not just a number. The blob is the
//! product: stored per cube cell, it supports query-time union/intersection/
//! Jaccard over cells that were never pre-aggregated together, and it merges
//! with future increments. `cubism_kmv_estimate(blob)` presents a blob as a
//! cardinality estimate.
//!
//! DataFusion's Accumulator model is natively phase-split (`update_batch` /
//! `state` / `merge_batch` / `evaluate`), which maps 1:1 onto the mergeable
//! sketch algebra in `cubism_core::sketch`.

use cubism_core::sketch::kmv::{hash_value, KmvSketch, DEFAULT_SKETCH_SIZE};
use cubism_core::sketch::sample::DEFAULT_SAMPLE_CAPACITY;
use cubism_core::sketch::topk::DEFAULT_TOPK_CAPACITY;
use cubism_core::sketch::{Centroid, ExemplarSample, TopK};
use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Builder};
use datafusion::arrow::datatypes::{DataType, Field, FieldRef};
use datafusion::common::{exec_err, Result, ScalarValue};
use datafusion::logical_expr::function::{AccumulatorArgs, StateFieldsArgs};
use datafusion::logical_expr::{
    Accumulator, AggregateUDF, AggregateUDFImpl, ColumnarValue, ScalarFunctionArgs, ScalarUDF,
    ScalarUDFImpl, Signature, Volatility,
};
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq, Hash)]
struct KmvSketchUdaf {
    k: u32,
    signature: Signature,
}

impl AggregateUDFImpl for KmvSketchUdaf {
    fn name(&self) -> &str {
        "cubism_kmv_sketch"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(KmvAccumulator { sketch: KmvSketch::new(self.k) }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(
            format!("{}[kmv]", args.name),
            DataType::Binary,
            true,
        ))])
    }
}

#[derive(Debug)]
struct KmvAccumulator {
    sketch: KmvSketch,
}

impl Accumulator for KmvAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let arr = values[0].as_string::<i32>();
        let batch = KmvSketch::from_hashes(
            self.sketch.k(),
            arr.iter().flatten().map(|s| hash_value(s.as_bytes())),
        );
        self.sketch = self.sketch.merge(&batch);
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let arr = states[0].as_binary::<i32>();
        for i in 0..arr.len() {
            if arr.is_null(i) {
                continue;
            }
            let other = KmvSketch::from_bytes(arr.value(i))
                .map_err(|e| datafusion::common::DataFusionError::Execution(e.to_string()))?;
            self.sketch = self.sketch.merge(&other);
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Binary(Some(self.sketch.to_bytes()))])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Binary(Some(self.sketch.to_bytes())))
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>() + self.sketch.len() * 8
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct KmvEstimateUdf {
    signature: Signature,
}

impl ScalarUDFImpl for KmvEstimateUdf {
    fn name(&self) -> &str {
        "cubism_kmv_estimate"
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
            match KmvSketch::from_bytes(blobs.value(i)) {
                Ok(s) => out.append_value(s.estimate()),
                Err(e) => return exec_err!("{e}"),
            }
        }
        Ok(ColumnarValue::Array(Arc::new(out.finish())))
    }
}

// ---------------------------------------------------------------------------
// TopK: cubism_topk_sketch(key VARCHAR, score DOUBLE) -> blob;
//       cubism_topk_json(blob) -> JSON [{"key","score"},...]
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Hash)]
struct TopKUdaf {
    capacity: u32,
    signature: Signature,
}

impl AggregateUDFImpl for TopKUdaf {
    fn name(&self) -> &str {
        "cubism_topk_sketch"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(TopKAccumulator { topk: TopK::new(self.capacity) }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(format!("{}[topk]", args.name), DataType::Binary, true))])
    }
}

#[derive(Debug)]
struct TopKAccumulator {
    topk: TopK,
}

impl Accumulator for TopKAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let keys = values[0].as_string::<i32>();
        let scores = values[1].as_primitive::<datafusion::arrow::datatypes::Float64Type>();
        for i in 0..keys.len() {
            if keys.is_null(i) || scores.is_null(i) {
                continue;
            }
            self.topk.add(keys.value(i), scores.value(i));
        }
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        for blob in iter_blobs(&states[0]) {
            let other = TopK::from_bytes(blob).map_err(to_df_err)?;
            self.topk = self.topk.merge(&other);
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Binary(Some(self.topk.to_bytes()))])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Binary(Some(self.topk.to_bytes())))
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>() + self.topk.len() * 32
    }
}

// ---------------------------------------------------------------------------
// ExemplarSample: cubism_sample_sketch(value VARCHAR) -> blob;
//                 cubism_sample_json(blob) -> JSON ["v1","v2",...]
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Hash)]
struct SampleUdaf {
    capacity: u32,
    signature: Signature,
}

impl AggregateUDFImpl for SampleUdaf {
    fn name(&self) -> &str {
        "cubism_sample_sketch"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(SampleAccumulator { sample: ExemplarSample::new(self.capacity) }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(format!("{}[sample]", args.name), DataType::Binary, true))])
    }
}

#[derive(Debug)]
struct SampleAccumulator {
    sample: ExemplarSample,
}

impl Accumulator for SampleAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let arr = values[0].as_string::<i32>();
        for value in arr.iter().flatten() {
            self.sample.insert(value);
        }
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        for blob in iter_blobs(&states[0]) {
            let other = ExemplarSample::from_bytes(blob).map_err(to_df_err)?;
            self.sample = self.sample.merge(&other);
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Binary(Some(self.sample.to_bytes()))])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Binary(Some(self.sample.to_bytes())))
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>() + self.sample.len() * 32
    }
}

// ---------------------------------------------------------------------------
// Centroid: cubism_centroid_sketch(vec DOUBLE[]) -> blob;
//           cubism_centroid_mean(blob) -> DOUBLE[]
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Hash)]
struct CentroidUdaf {
    signature: Signature,
}

impl AggregateUDFImpl for CentroidUdaf {
    fn name(&self) -> &str {
        "cubism_centroid_sketch"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(CentroidAccumulator { centroid: Centroid::new() }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(format!("{}[centroid]", args.name), DataType::Binary, true))])
    }
}

#[derive(Debug)]
struct CentroidAccumulator {
    centroid: Centroid,
}

impl Accumulator for CentroidAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let Some(lists) = values[0].as_list_opt::<i32>() else {
            return exec_err!(
                "cubism_centroid_sketch expects a list-of-float column, got {}",
                values[0].data_type()
            );
        };
        for i in 0..lists.len() {
            if lists.is_null(i) {
                continue;
            }
            let inner = lists.value(i);
            let Some(floats) =
                inner.as_primitive_opt::<datafusion::arrow::datatypes::Float64Type>()
            else {
                return exec_err!(
                    "cubism_centroid_sketch expects DOUBLE elements, got {}",
                    inner.data_type()
                );
            };
            let vector: Vec<f64> = floats.iter().map(|v| v.unwrap_or(0.0)).collect();
            self.centroid.add(&vector).map_err(to_df_err)?;
        }
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        for blob in iter_blobs(&states[0]) {
            let other = Centroid::from_bytes(blob).map_err(to_df_err)?;
            self.centroid = self.centroid.merge(&other).map_err(to_df_err)?;
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Binary(Some(self.centroid.to_bytes()))])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Binary(Some(self.centroid.to_bytes())))
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>() + self.centroid.dim() * 8
    }
}

// ---------------------------------------------------------------------------
// Presenters: blob -> human-readable value.
// ---------------------------------------------------------------------------

/// Generic presenter: a scalar UDF turning a sketch blob column into Utf8 via
/// a decode-and-render function.
#[derive(Debug, PartialEq, Eq, Hash)]
struct BlobPresenterUdf {
    name: &'static str,
    signature: Signature,
    kind: PresenterKind,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
enum PresenterKind {
    TopKJson,
    SampleJson,
}

impl ScalarUDFImpl for BlobPresenterUdf {
    fn name(&self) -> &str {
        self.name
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
        let mut out = datafusion::arrow::array::StringBuilder::new();
        for i in 0..blobs.len() {
            if blobs.is_null(i) {
                out.append_null();
                continue;
            }
            let rendered = match self.kind {
                PresenterKind::TopKJson => {
                    TopK::from_bytes(blobs.value(i)).map(|t| t.to_json())
                }
                PresenterKind::SampleJson => {
                    ExemplarSample::from_bytes(blobs.value(i)).map(|s| s.to_json())
                }
            };
            match rendered {
                Ok(s) => out.append_value(s),
                Err(e) => return exec_err!("{e}"),
            }
        }
        Ok(ColumnarValue::Array(Arc::new(out.finish())))
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct CentroidMeanUdf {
    signature: Signature,
}

impl ScalarUDFImpl for CentroidMeanUdf {
    fn name(&self) -> &str {
        "cubism_centroid_mean"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::List(Arc::new(Field::new("item", DataType::Float64, true))))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        use datafusion::arrow::array::ListBuilder;
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let blobs = arrays[0].as_binary::<i32>();
        let mut out = ListBuilder::new(Float64Builder::new());
        for i in 0..blobs.len() {
            if blobs.is_null(i) {
                out.append_null();
                continue;
            }
            let centroid = Centroid::from_bytes(blobs.value(i)).map_err(to_df_err)?;
            match centroid.mean() {
                Some(mean) => {
                    out.values().append_slice(&mean);
                    out.append(true);
                }
                None => out.append_null(),
            }
        }
        Ok(ColumnarValue::Array(Arc::new(out.finish())))
    }
}

// ---------------------------------------------------------------------------

fn to_df_err(e: cubism_core::CubismError) -> datafusion::common::DataFusionError {
    datafusion::common::DataFusionError::Execution(e.to_string())
}

fn iter_blobs(array: &ArrayRef) -> impl Iterator<Item = &[u8]> {
    let arr = array.as_binary::<i32>();
    (0..arr.len()).filter(|&i| !arr.is_null(i)).map(move |i| arr.value(i))
}

/// All sketch UDFs. They are stateless (unlike the per-build XUnit UDFs) and
/// registered once per context.
pub fn sketch_udfs() -> (Vec<AggregateUDF>, Vec<ScalarUDF>) {
    let binary_arg = || Signature::exact(vec![DataType::Binary], Volatility::Immutable);
    (
        vec![
            AggregateUDF::from(KmvSketchUdaf {
                k: DEFAULT_SKETCH_SIZE,
                signature: Signature::exact(vec![DataType::Utf8], Volatility::Immutable),
            }),
            AggregateUDF::from(TopKUdaf {
                capacity: DEFAULT_TOPK_CAPACITY,
                signature: Signature::exact(
                    vec![DataType::Utf8, DataType::Float64],
                    Volatility::Immutable,
                ),
            }),
            AggregateUDF::from(SampleUdaf {
                capacity: DEFAULT_SAMPLE_CAPACITY,
                signature: Signature::exact(vec![DataType::Utf8], Volatility::Immutable),
            }),
            AggregateUDF::from(CentroidUdaf { signature: Signature::any(1, Volatility::Immutable) }),
        ],
        vec![
            ScalarUDF::from(KmvEstimateUdf { signature: binary_arg() }),
            ScalarUDF::from(BlobPresenterUdf {
                name: "cubism_topk_json",
                signature: binary_arg(),
                kind: PresenterKind::TopKJson,
            }),
            ScalarUDF::from(BlobPresenterUdf {
                name: "cubism_sample_json",
                signature: binary_arg(),
                kind: PresenterKind::SampleJson,
            }),
            ScalarUDF::from(CentroidMeanUdf { signature: binary_arg() }),
        ],
    )
}
