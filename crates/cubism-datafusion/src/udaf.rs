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

/// The sketch UDFs are stateless (unlike the per-build XUnit UDFs) and
/// registered once per context.
pub fn sketch_udfs() -> (AggregateUDF, ScalarUDF) {
    (
        AggregateUDF::from(KmvSketchUdaf {
            k: DEFAULT_SKETCH_SIZE,
            signature: Signature::exact(vec![DataType::Utf8], Volatility::Immutable),
        }),
        ScalarUDF::from(KmvEstimateUdf {
            signature: Signature::exact(vec![DataType::Binary], Volatility::Immutable),
        }),
    )
}
