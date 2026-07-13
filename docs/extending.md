# Extending Cubism: sketches, UDAFs, and UDFs

Cubism has three extension surfaces, from cheapest to deepest:

1. **Spec expressions** — no code at all: dimension levels and measure
   inputs are SQL expressions evaluated by DataFusion.
2. **A new sketch/aggregator** — a mergeable buffer in `cubism-core` plus a
   DataFusion UDAF wrapper in `cubism-datafusion`.
3. **A new scalar UDF** — row-level functions (the explode and all blob
   presenters are just scalar UDFs).

This doc walks each one with toy examples, and ends with the checklist of
touch points for a full new aggregator.

## 1. Spec expressions (no code)

Level `expr` and measure `input`/`by` fields are DataFusion SQL. This is
already enough for derived dimensions and computed measures:

```yaml
dimensions:
  - name: latency_band
    levels:
      - name: band
        expr: "CASE WHEN duration_ms < 100 THEN 'fast' WHEN duration_ms < 1000 THEN 'ok' ELSE 'slow' END"
measures:
  - name: est_cost
    agg: sum
    input: "prompt_tokens * 0.000003 + completion_tokens * 0.000015"
```

If what you need is expressible as a SQL expression over one row, stop here.

## 2. A new sketch (mergeable aggregator)

### The contract

A sketch is a plain Rust struct in `cubism-core/src/sketch/` obeying four
rules:

1. **Mergeable**: `merge(&self, other) -> Self` is associative and
   commutative. (This is what makes distributed builds, incremental updates,
   and query-time set ops work — see `docs/scaling.md`.) If your merge is
   only *approximately* order-independent (like bounded top-k after
   pruning), document it.
2. **Versioned bytes**: `to_bytes()` starts with a magic + version header;
   `from_bytes()` validates and rejects unknown versions. Blobs are
   persisted state — format changes bump the version, never mutate v1.
3. **Golden test**: a test asserting the exact hex of a small sketch's
   serialization. It exists to fail loudly when someone accidentally changes
   the format (including any hash function the format depends on).
4. **Property tests**: merge associativity/commutativity/identity in
   `cubism-core/tests/properties.rs`.

### Toy example: an `Extremes` sketch (min + max of a stream)

```rust
// cubism-core/src/sketch/extremes.rs
use crate::error::CubismError;

const MAGIC: &[u8; 3] = b"EXT";
const VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct Extremes {
    min: f64,
    max: f64,
    count: u64, // 0 == empty; lets the empty sketch be the merge identity
}

impl Extremes {
    pub fn new() -> Self {
        Extremes { min: f64::INFINITY, max: f64::NEG_INFINITY, count: 0 }
    }

    pub fn add(&mut self, v: f64) {
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.count += 1;
    }

    /// min-of-mins and max-of-maxes: trivially associative + commutative.
    pub fn merge(&self, other: &Extremes) -> Extremes {
        Extremes {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
            count: self.count + other.count,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(28);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.min.to_le_bytes());
        out.extend_from_slice(&self.max.to_le_bytes());
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CubismError> {
        if bytes.len() != 28 || &bytes[0..3] != MAGIC || bytes[3] != VERSION {
            return Err(CubismError::Decode("Extremes: bad header".into()));
        }
        Ok(Extremes {
            count: u64::from_le_bytes(bytes[4..12].try_into().unwrap()),
            min: f64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            max: f64::from_le_bytes(bytes[20..28].try_into().unwrap()),
        })
    }
}
```

Real reference implementations, in ascending complexity:
`sketch/centroid.rs` (fixed-size numeric state), `sketch/sample.rs`
(bounded sorted entries, exactly associative), `sketch/kmv.rs` (the same
plus estimation math), `sketch/topk.rs` (bounded with pruning — the
approximately-associative case).

## 3. Wrapping a sketch as a DataFusion UDAF

DataFusion's `Accumulator` model is already phase-split, and it maps onto
the sketch contract one-to-one:

| Accumulator method | What it does | Sketch call |
|---|---|---|
| `update_batch(values)` | fold a batch of raw inputs | `sketch.add(...)` per row |
| `state()` | serialize the partial for shuffle | `vec![Binary(sketch.to_bytes())]` |
| `merge_batch(states)` | combine partials from other tasks | `from_bytes` + `merge` |
| `evaluate()` | final result for the group | `Binary(sketch.to_bytes())` |

Cubism UDAFs **evaluate to the blob, not a number** — the blob is the
product (stored per cell, merged at query time). A separate scalar
"presenter" UDF renders it human-readable.

```rust
// cubism-datafusion/src/udaf.rs (abridged toy: extremes over a DOUBLE column)
#[derive(Debug, PartialEq, Eq, Hash)]           // DF 54 requires Eq + Hash
struct ExtremesUdaf { signature: Signature }

impl AggregateUDFImpl for ExtremesUdaf {
    fn name(&self) -> &str { "cubism_extremes_sketch" }
    fn signature(&self) -> &Signature { &self.signature }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> { Ok(DataType::Binary) }
    fn accumulator(&self, _: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(ExtremesAccumulator { ex: Extremes::new() }))
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(format!("{}[ext]", args.name), DataType::Binary, true))])
    }
}

struct ExtremesAccumulator { ex: Extremes }
impl Accumulator for ExtremesAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let arr = values[0].as_primitive::<Float64Type>();
        arr.iter().flatten().for_each(|v| self.ex.add(v));
        Ok(())
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        for blob in iter_blobs(&states[0]) {
            self.ex = self.ex.merge(&Extremes::from_bytes(blob).map_err(to_df_err)?);
        }
        Ok(())
    }
    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Binary(Some(self.ex.to_bytes()))])
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Binary(Some(self.ex.to_bytes())))
    }
    fn size(&self) -> usize { std::mem::size_of::<Self>() }
}
```

DataFusion 54 API gotchas (learned the hard way):
- `ScalarUDFImpl` / `AggregateUDFImpl` implementors must be `Eq + Hash`
  (plan-node comparison); there is no `as_any` anymore. Stateless UDFs can
  `#[derive(PartialEq, Eq, Hash)]`; stateful ones (see the XUnit UDFs in
  `udf.rs`) implement identity via `Arc::ptr_eq`.
- Scalar UDFs implement `invoke_with_args(ScalarFunctionArgs)`; get arrays
  with `ColumnarValue::values_to_arrays`.
- **Never take a lock per row in `invoke`/`update_batch`.** A per-row mutex
  serialized the entire pipeline across partitions (82.9s → 4.5s after the
  fix). Snapshot shared state once per batch; merge changes back once at
  batch end (`udf.rs` shows the pattern).

### Also implement `GroupsAccumulator`

A plain `Accumulator` still works under GROUP BY, but DataFusion then wraps
it in `GroupsAccumulatorAdapter`, which re-partitions and gathers every
batch's input arrays once per touched group per aggregate. With thousands
of cube cells that adapter tax dwarfs the sketch work itself. The fix is a
vectorized `GroupsAccumulator` that holds `Vec<Sketch>` and walks each batch
once, updating `sketches[group_indices[row]]`:

- implement the small `SketchKernel` trait in `udaf.rs` (empty sketch,
  per-batch row walk, blob merge, serialize) and reuse the generic
  `SketchGroupsAccumulator<K>`;
- add `groups_accumulator_supported() -> true` and
  `create_groups_accumulator()` to the UDAF impl;
- `state == evaluate == the blob` for every sketch kind, so `state()` is
  one line;
- respect `opt_filter` — rows it excludes must not reach the sketch.

Two more perf lessons encoded in this repo:
- **Amortize pruning.** A bounded structure that prunes back only to its
  overflow threshold evicts one entry per insert once full, degenerating
  every add into a re-sort (a one-line `TopK::prune` bug that took a 5M-row
  build from ~4s to ~40x slower). Prune back to `capacity` so re-sorts
  amortize over the headroom.
- **Parquet row groups are the scan-parallelism unit.** A single-row-group
  input file serializes the whole plan; write test/demo data with
  `row_group_size` around 128k rows.

## 4. Row-level scalar UDFs

`udf.rs` holds the two structural ones — `cubism_xunit_keys` (the lattice
explode) and `cubism_xunit_str` (key → canonical string) — and `udaf.rs`
holds the blob presenters (`cubism_kmv_estimate`, `cubism_topk_json`, …).
A new presenter is ~30 lines: implement `ScalarUDFImpl` with a
`Binary → whatever` kernel and add it to `sketch_udfs()`. Presenters are the
natural place for new *views* of an existing blob (e.g. a
`cubism_kmv_hashes` debug view) without touching the aggregation at all.

## 5. Full checklist: adding an aggregator end-to-end

Using `top_k` as the worked example (every line below exists in the repo —
grep for `TopK`):

1. **`cubism-core/src/sketch/<name>.rs`** — struct, `merge`, versioned
   `to_bytes`/`from_bytes`, unit + golden tests. Export in `sketch/mod.rs`.
2. **`cubism-core/tests/properties.rs`** — merge algebra proptests.
3. **`cubism-core/src/spec.rs`** — add the `AggKind` variant (snake_case is
   the spec-facing name) and any validation (e.g. `top_k` requires `by`).
4. **`cubism-datafusion/src/udaf.rs`** — the UDAF + accumulator + presenter;
   register both in `sketch_udfs()`.
5. **`cubism-datafusion/src/build.rs`** — the `cube_sql` match arm: project
   inputs into `__m{i}` aliases (add them to `passthrough`), emit
   `<udaf>(...) AS "{name}__sketch"`, and present as
   `<presenter>(...) AS "{name}", "{name}__sketch"`.
6. **An end-to-end test** in `build.rs` asserting real values through the
   full pipeline (see `top_k_and_sample_measures_work_end_to_end`).

## 6. Using blobs outside the engine

`cubism-core` has no DataFusion dependency, so any service can consume cube
output directly — this is how the serving layer's set-ops work:

```rust
use cubism_core::sketch::KmvSketch;

let audience_a = KmvSketch::from_bytes(&cell_a_blob)?; // e.g. from Postgres
let audience_b = KmvSketch::from_bytes(&cell_b_blob)?;
println!("overlap ≈ {}", audience_a.intersection_estimate(&audience_b));
println!("jaccard ≈ {}", audience_a.jaccard(&audience_b));
```
