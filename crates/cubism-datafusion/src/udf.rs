//! Scalar UDFs bridging the cubism-core algebra into DataFusion SQL.
//!
//! `cubism_xunit_keys(level_cols...) -> List<Binary>` computes, per input
//! row, the pruned-lattice XUnit keys the row belongs to; SQL `unnest` then
//! explodes them into one row per cube cell. This gives streaming explode
//! semantics with zero custom ExecutionPlan surface — a deliberate M2 choice;
//! a dedicated operator is a later optimization if profiling demands it.
//!
//! `cubism_xunit_str(key) -> Utf8` decodes a binary key to the canonical
//! string form at the present boundary.

use cubism_core::encoding::{decode_xunit, encode_xunit, XUnitDictionary};
use cubism_core::lattice::generate_xunits;
use cubism_core::{CubeSpec, FilterRule};
use datafusion::arrow::array::{Array, AsArray, BinaryBuilder, ListBuilder, StringBuilder};
use datafusion::arrow::datatypes::{DataType, Field};
use datafusion::common::{exec_err, Result};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Shared per-build dictionary: the keys UDF interns into it, the decode UDF
/// reads from it.
pub type SharedDictionary = Arc<Mutex<XUnitDictionary>>;

/// How many level columns each dimension consumes, in canonical
/// (name-sorted) dimension order. Level columns are passed to the UDF
/// flattened in that same order, all cast to VARCHAR.
#[derive(Debug, Clone)]
struct ExplodeShape {
    /// (dimension index in sorted order, level attribute names)
    dims: Vec<(String, Vec<String>)>,
    filter_rules: Vec<FilterRule>,
    include_global: bool,
    /// From the spec's `maxDictionaryEntries`: fail fast when a
    /// high-cardinality dimension floods the dictionary.
    max_dictionary_entries: usize,
}

impl ExplodeShape {
    fn from_spec(spec: &CubeSpec) -> Self {
        let dims = spec
            .sorted_dimensions()
            .iter()
            .map(|d| {
                (d.name.clone(), d.effective_levels().iter().map(|l| l.name.clone()).collect())
            })
            .collect();
        ExplodeShape {
            dims,
            filter_rules: spec.filter_rules.clone(),
            include_global: spec.include_global,
            max_dictionary_entries: spec.max_dictionary_entries,
        }
    }

    fn num_level_columns(&self) -> usize {
        self.dims.iter().map(|(_, levels)| levels.len()).sum()
    }
}

/// Memo from a row's level-value tuple to its encoded lattice keys. BI data
/// has few distinct dimension combinations relative to row count, so after
/// warmup the per-row cost is one hash lookup instead of lattice generation
/// and key encoding. Buckets store the full tuple and are verified on hit,
/// so hash collisions cannot produce wrong keys.
///
/// Concurrency: per-row locking measurably serialized the whole pipeline
/// (sys-time thrash across partitions), so each `invoke` takes an `Arc`
/// snapshot of the map once per batch, reads it lock-free, and merges its
/// misses back under one short lock at batch end.
type MemoBucket = Vec<(Vec<Option<String>>, Arc<Vec<Vec<u8>>>)>;
type KeyMemo = Arc<Mutex<Arc<HashMap<u64, MemoBucket>>>>;

/// Stop growing the memo past this many hash buckets. The memo only pays
/// for itself when tuples repeat; with a high-cardinality input (every row
/// distinct) an unbounded memo is pure memory growth, and the batch-end
/// merge — which clones the shared map — would go quadratic. Past the cap,
/// rows just take the compute path; results stay identical.
const MEMO_MAX_BUCKETS: usize = 1 << 18;

fn tuple_matches(stored: &[Option<String>], cols: &[&datafusion::arrow::array::StringArray], row: usize) -> bool {
    stored.iter().zip(cols).all(|(s, col)| match s {
        None => col.is_null(row),
        Some(v) => !col.is_null(row) && col.value(row) == v,
    })
}

#[derive(Debug)]
struct XUnitKeysUdf {
    shape: ExplodeShape,
    dict: SharedDictionary,
    memo: KeyMemo,
    /// One hasher state for the UDF's lifetime — memo hashes must be stable
    /// across batches and partitions.
    hash_state: std::hash::RandomState,
    signature: Signature,
}

// DataFusion 54 requires UDF impls to be Eq + Hash (plan-node comparison).
// Two instances are the same function iff they share a build dictionary.
impl PartialEq for XUnitKeysUdf {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.dict, &other.dict)
    }
}
impl Eq for XUnitKeysUdf {}
impl std::hash::Hash for XUnitKeysUdf {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.dict) as usize).hash(state);
    }
}

impl XUnitKeysUdf {
    /// The slow path: rebuild per-dimension hierarchy YPaths from the
    /// flattened, truncate-at-first-null level values, generate the pruned
    /// lattice, and encode each cell key. The caller holds the dictionary
    /// lock (once per batch, not per row).
    fn compute_row_keys(
        &self,
        dict: &mut XUnitDictionary,
        cols: &[&datafusion::arrow::array::StringArray],
        row: usize,
    ) -> Result<Vec<Vec<u8>>> {
        let mut per_dimension = Vec::with_capacity(self.shape.dims.len());
        let mut col_idx = 0;
        for (dim, levels) in &self.shape.dims {
            let mut ypaths = Vec::with_capacity(levels.len());
            let mut yp = cubism_core::YPath::new(dim.clone());
            let mut truncated = false;
            for level in levels {
                let col = cols[col_idx];
                col_idx += 1;
                if truncated || col.is_null(row) {
                    truncated = true;
                    continue;
                }
                yp = yp.with_attribute(level.clone(), col.value(row).to_string());
                ypaths.push(yp.clone());
            }
            per_dimension.push(ypaths);
        }

        let xunits =
            generate_xunits(&per_dimension, &self.shape.filter_rules, self.shape.include_global);
        let keys = xunits.iter().map(|x| encode_xunit(x, dict)).collect();
        if dict.len() > self.shape.max_dictionary_entries {
            return exec_err!(
                "cube dictionary exceeded maxDictionaryEntries ({}) — a dimension is \
                 likely high-cardinality (a UUID, raw timestamp, or free-form text). \
                 Bucket it with a level `expr` (date_trunc, CASE, substr), or model \
                 identity as a measure (count_distinct / top_k / reservoir_sample) \
                 instead of a dimension. See docs/high-cardinality.md; raise \
                 maxDictionaryEntries in the spec only if the cardinality is intended",
                self.shape.max_dictionary_entries
            );
        }
        Ok(keys)
    }
}

impl ScalarUDFImpl for XUnitKeysUdf {
    fn name(&self) -> &str {
        "cubism_xunit_keys"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::List(Arc::new(Field::new("item", DataType::Binary, true))))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let expected = self.shape.num_level_columns();
        if arrays.len() != expected {
            return exec_err!(
                "cubism_xunit_keys expects {expected} level columns, got {}",
                arrays.len()
            );
        }
        let cols: Vec<_> = arrays.iter().map(|a| a.as_string::<i32>()).collect();
        let num_rows = arrays.first().map_or(0, |a| a.len());

        let mut builder = ListBuilder::new(BinaryBuilder::new());
        use std::hash::{BuildHasher, Hash, Hasher};
        let hash_state = &self.hash_state;

        let snapshot = Arc::clone(&*self.memo.lock().expect("memo poisoned"));
        let mut batch_misses: HashMap<u64, MemoBucket> = HashMap::new();
        let memo_full = snapshot.len() >= MEMO_MAX_BUCKETS;
        // Lock the dictionary once for all of this batch's misses, not per
        // row: all-miss workloads (first batches, high-cardinality tuples)
        // would otherwise serialize the partitions on the lock.
        let mut dict_guard: Option<std::sync::MutexGuard<'_, XUnitDictionary>> = None;

        for row in 0..num_rows {
            // Hash the row's level-value tuple (null-aware) for memo lookup.
            let mut hasher = hash_state.build_hasher();
            for col in &cols {
                if col.is_null(row) {
                    0u8.hash(&mut hasher);
                } else {
                    1u8.hash(&mut hasher);
                    col.value(row).hash(&mut hasher);
                }
            }
            let memo_key = hasher.finish();

            let lookup = |bucket: Option<&MemoBucket>| {
                bucket.and_then(|b| {
                    b.iter()
                        .find(|(tuple, _)| tuple_matches(tuple, &cols, row))
                        .map(|(_, keys)| Arc::clone(keys))
                })
            };
            let cached =
                lookup(snapshot.get(&memo_key)).or_else(|| lookup(batch_misses.get(&memo_key)));
            let keys = match cached {
                Some(keys) => keys,
                None => {
                    let dict = dict_guard
                        .get_or_insert_with(|| self.dict.lock().expect("dictionary poisoned"));
                    let keys = Arc::new(self.compute_row_keys(dict, &cols, row)?);
                    if !memo_full {
                        let tuple: Vec<Option<String>> = cols
                            .iter()
                            .map(|c| (!c.is_null(row)).then(|| c.value(row).to_string()))
                            .collect();
                        batch_misses.entry(memo_key).or_default().push((tuple, Arc::clone(&keys)));
                    }
                    keys
                }
            };

            for key in keys.iter() {
                builder.values().append_value(key);
            }
            builder.append(true);
        }

        drop(dict_guard);

        if !batch_misses.is_empty() {
            let mut shared = self.memo.lock().expect("memo poisoned");
            if shared.len() < MEMO_MAX_BUCKETS {
                let mut merged = (**shared).clone();
                for (hash, bucket) in batch_misses {
                    let target = merged.entry(hash).or_default();
                    for (tuple, keys) in bucket {
                        if !target.iter().any(|(t, _)| *t == tuple) {
                            target.push((tuple, keys));
                        }
                    }
                }
                *shared = Arc::new(merged);
            }
        }

        Ok(ColumnarValue::Array(Arc::new(builder.finish())))
    }
}

#[derive(Debug)]
struct XUnitStrUdf {
    dict: SharedDictionary,
    signature: Signature,
}

impl PartialEq for XUnitStrUdf {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.dict, &other.dict)
    }
}
impl Eq for XUnitStrUdf {}
impl std::hash::Hash for XUnitStrUdf {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.dict) as usize).hash(state);
    }
}

impl ScalarUDFImpl for XUnitStrUdf {
    fn name(&self) -> &str {
        "cubism_xunit_str"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let keys = arrays[0].as_binary::<i32>();
        let dict = self.dict.lock().expect("dictionary poisoned");
        let mut builder = StringBuilder::new();
        for row in 0..keys.len() {
            if keys.is_null(row) {
                builder.append_null();
                continue;
            }
            match decode_xunit(keys.value(row), &dict) {
                Ok(xunit) => builder.append_value(xunit.to_string()),
                Err(e) => return exec_err!("cannot decode xunit key: {e}"),
            }
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish())))
    }
}

/// Build the paired (keys, decode) UDFs for one cube build, sharing a fresh
/// dictionary. Returns the UDFs plus the dictionary handle (the sidecar
/// table source).
pub fn cube_udfs(spec: &CubeSpec) -> (ScalarUDF, ScalarUDF, SharedDictionary) {
    let dict: SharedDictionary = Arc::new(Mutex::new(XUnitDictionary::new()));
    let shape = ExplodeShape::from_spec(spec);
    let keys = ScalarUDF::from(XUnitKeysUdf {
        signature: Signature::exact(
            vec![DataType::Utf8; shape.num_level_columns()],
            Volatility::Immutable,
        ),
        shape,
        dict: Arc::clone(&dict),
        memo: Arc::new(Mutex::new(Arc::new(HashMap::new()))),
        hash_state: std::hash::RandomState::new(),
    });
    let decode = ScalarUDF::from(XUnitStrUdf {
        signature: Signature::exact(vec![DataType::Binary], Volatility::Immutable),
        dict: Arc::clone(&dict),
    });
    (keys, decode, dict)
}
