//! In-memory cube store: one loaded cube parquet, indexed by xunit.
//!
//! A cube file is small (cells, not events), so the store keeps everything
//! resident: measure values pre-converted to JSON, sketch blobs as raw
//! bytes decoded on demand per request. Sketch kinds are recognized by the
//! magic bytes of their versioned formats — the store needs no spec.

use cubism_core::sketch::{Centroid, ExemplarSample, KmvSketch, TopK};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("cannot read cube file: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot read cube parquet: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    #[error("cannot decode cube: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    #[error("not a cube file: {0}")]
    Shape(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SketchKind {
    Kmv,
    TopK,
    Sample,
    Centroid,
    Unknown,
}

impl SketchKind {
    fn detect(blob: &[u8]) -> Self {
        match blob.get(0..3) {
            Some(b"KMV") => SketchKind::Kmv,
            Some(b"TPK") => SketchKind::TopK,
            Some(b"SMP") => SketchKind::Sample,
            Some(b"CTR") => SketchKind::Centroid,
            _ => SketchKind::Unknown,
        }
    }
}

pub struct CubeStore {
    pub source: String,
    pub xunits: Vec<String>,
    pub index: HashMap<String, usize>,
    /// Measure display order (spec order, i.e. parquet column order).
    pub measures: Vec<String>,
    pub measure_values: HashMap<String, Vec<Value>>,
    pub sketches: Vec<(String, SketchKind)>,
    pub sketch_blobs: HashMap<String, Vec<Option<Vec<u8>>>>,
    pub dimensions: Vec<String>,
}

impl CubeStore {
    pub fn from_path(path: &str) -> Result<Self, StoreError> {
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        let file = std::fs::File::open(path)?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;

        let mut xunits: Vec<String> = Vec::new();
        let mut measures: Vec<String> = Vec::new();
        let mut measure_values: HashMap<String, Vec<Value>> = HashMap::new();
        let mut sketch_blobs: HashMap<String, Vec<Option<Vec<u8>>>> = HashMap::new();
        let mut sketch_order: Vec<String> = Vec::new();

        for batch in reader {
            let batch = batch?;
            for (i, field) in batch.schema().fields().iter().enumerate() {
                let name = field.name().clone();
                let col = batch.column(i);
                if name == "xunit" {
                    append_strings(&mut xunits, col)?;
                } else if let Some(measure) = name.strip_suffix("__sketch") {
                    let blobs = sketch_blobs.entry(measure.to_string()).or_insert_with(|| {
                        sketch_order.push(measure.to_string());
                        Vec::new()
                    });
                    append_blobs(blobs, col)?;
                } else {
                    let values = measure_values.entry(name.clone()).or_insert_with(|| {
                        measures.push(name.clone());
                        Vec::new()
                    });
                    append_values(values, col)?;
                }
            }
        }
        if xunits.is_empty() {
            return Err(StoreError::Shape(
                "no 'xunit' column — is this the output of `cubism run --output`?".into(),
            ));
        }

        let index = xunits.iter().enumerate().map(|(i, x)| (x.clone(), i)).collect();

        let mut dimensions: BTreeSet<String> = BTreeSet::new();
        for xunit in &xunits {
            if xunit == "/G" {
                continue;
            }
            for conjunct in xunit.split(',') {
                if let Some(dim) = conjunct.split('/').nth(1) {
                    dimensions.insert(dim.to_string());
                }
            }
        }

        // Kind per sketch column, from the first non-null blob.
        let sketches = sketch_order
            .into_iter()
            .map(|name| {
                let kind = sketch_blobs[&name]
                    .iter()
                    .flatten()
                    .next()
                    .map_or(SketchKind::Unknown, |b| SketchKind::detect(b));
                (name, kind)
            })
            .collect();

        Ok(CubeStore {
            source: path.to_string(),
            xunits,
            index,
            measures,
            measure_values,
            sketches,
            sketch_blobs,
            dimensions: dimensions.into_iter().collect(),
        })
    }

    pub fn row(&self, xunit: &str) -> Option<usize> {
        self.index.get(xunit).copied()
    }

    pub fn measures_at(&self, row: usize) -> Value {
        let entries = self
            .measures
            .iter()
            .map(|m| (m.clone(), self.measure_values[m][row].clone()));
        Value::Object(entries.collect())
    }

    pub fn blob(&self, measure: &str, row: usize) -> Option<&[u8]> {
        self.sketch_blobs.get(measure)?.get(row)?.as_deref()
    }

    pub fn sketch_kind(&self, measure: &str) -> Option<SketchKind> {
        self.sketches.iter().find(|(n, _)| n == measure).map(|(_, k)| *k)
    }

    /// Decode one sketch blob into a JSON summary (never the raw bytes).
    pub fn decode_sketch(&self, measure: &str, row: usize) -> Value {
        let Some(blob) = self.blob(measure, row) else { return Value::Null };
        match SketchKind::detect(blob) {
            SketchKind::Kmv => KmvSketch::from_bytes(blob).map_or(Value::Null, |s| {
                json!({"kind": "kmv", "estimate": s.estimate(), "k": s.k()})
            }),
            SketchKind::TopK => TopK::from_bytes(blob).map_or(Value::Null, |t| {
                json!({"kind": "top_k", "items": t.top()})
            }),
            SketchKind::Sample => ExemplarSample::from_bytes(blob).map_or(Value::Null, |s| {
                json!({"kind": "sample", "values": s.values().collect::<Vec<_>>()})
            }),
            SketchKind::Centroid => Centroid::from_bytes(blob).map_or(Value::Null, |c| {
                json!({"kind": "centroid", "dim": c.dim(), "mean": c.mean()})
            }),
            SketchKind::Unknown => Value::Null,
        }
    }

    /// Single-conjunct cells of `dim` with exactly `depth` attributes,
    /// e.g. depth 1 of `geo` = the `/geo/country=..` cells.
    pub fn slice(&self, dim: &str, depth: usize) -> Vec<usize> {
        let prefix = format!("/{dim}/");
        self.xunits
            .iter()
            .enumerate()
            .filter(|(_, x)| {
                x.starts_with(&prefix)
                    && !x.contains(',')
                    && x.matches('=').count() == depth
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Display label for a cell in a slice: its attribute values joined.
    pub fn label(&self, row: usize) -> String {
        let xunit = &self.xunits[row];
        let values: Vec<&str> = xunit
            .split('/')
            .filter_map(|seg| seg.split_once('=').map(|(_, v)| v))
            .collect();
        if values.is_empty() { xunit.clone() } else { values.join(" / ") }
    }
}

fn append_strings(out: &mut Vec<String>, col: &arrow::array::ArrayRef) -> Result<(), StoreError> {
    use arrow::array::{Array, AsArray};
    let arr = col
        .as_string_opt::<i32>()
        .ok_or_else(|| StoreError::Shape(format!("xunit column is {}", col.data_type())))?;
    for i in 0..arr.len() {
        out.push(if arr.is_null(i) { String::new() } else { arr.value(i).to_string() });
    }
    Ok(())
}

fn append_blobs(
    out: &mut Vec<Option<Vec<u8>>>,
    col: &arrow::array::ArrayRef,
) -> Result<(), StoreError> {
    use arrow::array::{Array, AsArray};
    let arr = col
        .as_binary_opt::<i32>()
        .ok_or_else(|| StoreError::Shape(format!("sketch column is {}", col.data_type())))?;
    for i in 0..arr.len() {
        out.push((!arr.is_null(i)).then(|| arr.value(i).to_vec()));
    }
    Ok(())
}

/// Convert any measure column to JSON values. Utf8 that parses as JSON is
/// embedded structurally (the top-k/sample presenter columns).
fn append_values(out: &mut Vec<Value>, col: &arrow::array::ArrayRef) -> Result<(), StoreError> {
    use arrow::array::{Array, AsArray};
    use arrow::datatypes::{DataType, Float64Type, Int64Type, UInt64Type};
    match col.data_type() {
        DataType::Int64 => {
            let arr = col.as_primitive::<Int64Type>();
            for i in 0..arr.len() {
                out.push(if arr.is_null(i) { Value::Null } else { json!(arr.value(i)) });
            }
        }
        DataType::UInt64 => {
            let arr = col.as_primitive::<UInt64Type>();
            for i in 0..arr.len() {
                out.push(if arr.is_null(i) { Value::Null } else { json!(arr.value(i)) });
            }
        }
        DataType::Float64 => {
            let arr = col.as_primitive::<Float64Type>();
            for i in 0..arr.len() {
                out.push(if arr.is_null(i) { Value::Null } else { json!(arr.value(i)) });
            }
        }
        DataType::Utf8 => {
            let arr = col.as_string::<i32>();
            for i in 0..arr.len() {
                out.push(if arr.is_null(i) {
                    Value::Null
                } else {
                    let s = arr.value(i);
                    serde_json::from_str(s).unwrap_or_else(|_| json!(s))
                });
            }
        }
        DataType::List(_) => {
            let arr = col.as_list::<i32>();
            for i in 0..arr.len() {
                if arr.is_null(i) {
                    out.push(Value::Null);
                    continue;
                }
                let inner = arr.value(i);
                match inner.as_primitive_opt::<Float64Type>() {
                    Some(f) => out.push(json!(
                        (0..f.len())
                            .map(|j| if f.is_null(j) { None } else { Some(f.value(j)) })
                            .collect::<Vec<_>>()
                    )),
                    None => out.push(Value::Null),
                }
            }
        }
        other => {
            return Err(StoreError::Shape(format!("unsupported measure column type {other}")));
        }
    }
    Ok(())
}
