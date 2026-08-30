//! Python bindings for Cubism.
//!
//! ```python
//! import cubism
//! table = cubism.build_cube(spec_yaml, "events.parquet")   # -> pyarrow.Table
//! s = cubism.KmvSketch.from_bytes(blob)
//! s.jaccard(other)
//! ```
//!
//! Cube results cross the boundary via the Arrow C data interface (zero
//! semantic translation — the pyarrow Table's schema is exactly the cube
//! output: `xunit`, measures, and `*__sketch` blob columns).

use arrow::pyarrow::ToPyArrow;
use cubism_core::CubeSpec;
use cubism_core::sketch::kmv::DEFAULT_SKETCH_SIZE;
use cubism_core::sketch::{ExemplarSample, KmvSketch, TopK};
use cubism_datafusion::datafusion::prelude::{CsvReadOptions, ParquetReadOptions, SessionContext};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

fn value_err(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

fn runtime_err(e: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// Parse and validate a cube spec, returning the cube name.
/// Raises ValueError with every validation problem listed.
#[pyfunction]
fn validate_spec(spec_yaml: &str) -> PyResult<String> {
    let spec = CubeSpec::from_yaml(spec_yaml).map_err(value_err)?;
    Ok(spec.name)
}

/// Build a cube from a parquet or CSV file and return it as a pyarrow.Table.
#[pyfunction]
fn build_cube(py: Python<'_>, spec_yaml: &str, input_path: &str) -> PyResult<Py<PyAny>> {
    let spec = CubeSpec::from_yaml(spec_yaml).map_err(value_err)?;

    let (schema, batches) = py
        .detach(|| {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async {
                let ctx = SessionContext::new();
                if input_path.ends_with(".csv") {
                    ctx.register_csv("events", input_path, CsvReadOptions::new())
                        .await?;
                } else {
                    ctx.register_parquet("events", input_path, ParquetReadOptions::default())
                        .await?;
                }
                let (df, _dict) = cubism_datafusion::build_cube(&ctx, &spec, "events").await?;
                let schema = df.schema().as_arrow().clone();
                let batches = df.collect().await?;
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>((schema, batches))
            })
        })
        .map_err(runtime_err)?;

    // pyarrow.Table.from_batches, with the schema passed explicitly so an
    // empty cube still yields a well-typed table.
    let py_batches = batches
        .iter()
        .map(|b| b.to_pyarrow(py))
        .collect::<PyResult<Vec<_>>>()?;
    let py_schema = schema.to_pyarrow(py)?;
    let pyarrow = py.import("pyarrow")?;
    let table = pyarrow
        .getattr("Table")?
        .call_method1("from_batches", (py_batches, py_schema))?;
    Ok(table.unbind())
}

/// KMV distinct-count sketch: the mergeable buffer behind `count_distinct`
/// measures. Supports union, intersection, and Jaccard over cube cells.
#[pyclass(name = "KmvSketch", frozen)]
struct PyKmvSketch {
    inner: KmvSketch,
}

#[pymethods]
impl PyKmvSketch {
    #[new]
    #[pyo3(signature = (k = DEFAULT_SKETCH_SIZE))]
    fn new(k: u32) -> Self {
        PyKmvSketch {
            inner: KmvSketch::new(k),
        }
    }

    /// Deserialize a sketch blob (a `*__sketch` cube column value).
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        Ok(PyKmvSketch {
            inner: KmvSketch::from_bytes(data).map_err(value_err)?,
        })
    }

    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.to_bytes())
    }

    /// Union merge (associative + commutative).
    fn merge(&self, other: &PyKmvSketch) -> PyKmvSketch {
        PyKmvSketch {
            inner: self.inner.merge(&other.inner),
        }
    }

    /// Estimated distinct count (exact while under-full).
    fn estimate(&self) -> f64 {
        self.inner.estimate()
    }

    fn union_estimate(&self, other: &PyKmvSketch) -> f64 {
        self.inner.union_estimate(&other.inner)
    }

    fn intersection_estimate(&self, other: &PyKmvSketch) -> f64 {
        self.inner.intersection_estimate(&other.inner)
    }

    /// Estimated Jaccard similarity |A∩B| / |A∪B|.
    fn jaccard(&self, other: &PyKmvSketch) -> f64 {
        self.inner.jaccard(&other.inner)
    }

    #[getter]
    fn k(&self) -> u32 {
        self.inner.k()
    }

    fn __repr__(&self) -> String {
        format!(
            "KmvSketch(k={}, estimate={:.1})",
            self.inner.k(),
            self.inner.estimate()
        )
    }
}

/// Decode a `top_k` sketch blob into a list of (key, score) pairs.
#[pyfunction]
fn topk_items(data: &[u8]) -> PyResult<Vec<(String, f64)>> {
    Ok(TopK::from_bytes(data).map_err(value_err)?.top())
}

/// Decode a `reservoir_sample` sketch blob into the sampled values.
#[pyfunction]
fn sample_values(data: &[u8]) -> PyResult<Vec<String>> {
    Ok(ExemplarSample::from_bytes(data)
        .map_err(value_err)?
        .values()
        .map(str::to_string)
        .collect())
}

#[pymodule]
fn cubism(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(validate_spec, m)?)?;
    m.add_function(wrap_pyfunction!(build_cube, m)?)?;
    m.add_function(wrap_pyfunction!(topk_items, m)?)?;
    m.add_function(wrap_pyfunction!(sample_values, m)?)?;
    m.add_class::<PyKmvSketch>()?;
    Ok(())
}
