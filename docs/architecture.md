# Architecture

How a spec becomes a cube: the crates, the execution pipeline, and the
design decisions with their reasons.

## Crate layout

```text
cubism/
  crates/
    cubism-core/        THE MODEL. No DataFusion dependency.
      spec.rs             CubeSpec serde types + validation (the public contract)
      ypath.rs            YPath / XUnit + canonical string form
      rules.rs            FilterRule enum + evaluation + max-dims bound extraction
      lattice.rs          per-row lattice generation with inline pruning
      encoding.rs         binary cell keys + per-build string dictionary
      sketch/             KMV, TopK, ExemplarSample, Centroid — mergeable buffers
    cubism-datafusion/  THE ENGINE. DataFusion 54.
      udf.rs              cubism_xunit_keys (the explode), cubism_xunit_str (decode)
      udaf.rs             sketch UDAFs + blob presenters
      build.rs            spec -> SQL compiler + build_cube() entry point
    cubism-cli/         `cubism validate` / `cubism run`
  bindings/
    cubism-py/          pyo3 module: build_cube -> pyarrow.Table, sketch classes
```

**Why the core/engine split is load-bearing:** `cubism-core` is what a
serving layer links against to merge sketch blobs and validate specs — it
must stay lightweight and free of engine churn. It's also the porting
surface if another execution engine (or a warehouse-pushdown compiler) ever
hosts the same model. Dependency direction is strictly
`core ← datafusion ← {cli, py}`.

## Execution pipeline

`build_cube(ctx, spec, source)` compiles the spec to one SQL statement over
four CTE stages and hands it to DataFusion:

```sql
WITH __cubism_input AS (          -- 1. evaluate spec expressions
  SELECT CAST(country AS VARCHAR) AS __l0, CAST(city AS VARCHAR) AS __l1, ...,
         pv AS __m0, CAST(user_id AS VARCHAR) AS __m1
  FROM events
),
__cubism_exploded AS (            -- 2. explode: one row per (row × cell)
  SELECT unnest(cubism_xunit_keys(__l0, __l1, ...)) AS __xunit_key, __m0, __m1
  FROM __cubism_input
),
__cubism_cells AS (               -- 3. aggregate per cell
  SELECT __xunit_key, SUM(__m0) AS "pvs",
         cubism_kmv_sketch(__m1) AS "reach__sketch"
  FROM __cubism_exploded GROUP BY __xunit_key
)
SELECT cubism_xunit_str(__xunit_key) AS xunit,   -- 4. present
       "pvs", cubism_kmv_estimate("reach__sketch") AS "reach", "reach__sketch"
FROM __cubism_cells
```

Stage by stage:

1. **Input projection** — dimension level `expr`s and measure `input`/`by`
   expressions are raw SQL from the spec, evaluated by DataFusion. This *is*
   the expression language; the engine adds nothing bespoke.
2. **Explode** — `cubism_xunit_keys` is a scalar UDF returning
   `List<Binary>`: for each row it generates the pruned lattice
   (`cubism-core::lattice`) and encodes each cell as a compact binary key.
   SQL `unnest` turns the list into rows. *Why a UDF + unnest instead of a
   custom `ExecutionPlan` operator:* identical streaming semantics, a tenth
   of the API surface, and near-immunity to DataFusion version churn. A
   dedicated operator remains an optimization card to play if profiling ever
   demands it.
3. **Aggregate** — stock DataFusion hash aggregation. Scalar measures use
   native `SUM`/`COUNT`/…; sketch measures use Cubism UDAFs whose
   accumulators *are* the core sketch structs (DataFusion's
   `update_batch / state / merge_batch / evaluate` is exactly the mergeable
   phase-split). UDAFs evaluate to the **blob**, not a number.
4. **Present** — `cubism_xunit_str` decodes binary keys to canonical
   strings; presenter UDFs (`cubism_kmv_estimate`, `cubism_topk_json`, …)
   render blobs. Blobs also pass through: the output carries both views.

## Key design decisions

**Binary cell keys inside, canonical strings at the boundary.** The legacy
system's dominant cost was re-parsing XUnit strings everywhere. Here the
group-by key is a compact binary encoding (`dim_id:u32, n_attrs:u8,
(name_id, value_id):u32×2 ...`) against a per-build interned dictionary;
strings exist only at present time. The flip side: binary keys are
dictionary-scoped, so anything crossing builds (partial-cube merges,
serving-store joins) uses the string form, which is canonical by
construction. This trade is documented in `docs/scaling.md`.

**Lattice pruning during generation.** `max_dimensions` is monotone (every
superset of an excluded cell is excluded), so the generator refuses to
extend candidates past the bound instead of materializing-then-filtering.
Other rules run per-candidate at emit. Property tests pin
pruned ≡ generate-all-then-filter.

**The explode memo.** Real data has few distinct dimension-value
combinations relative to rows, so the explode UDF memoizes
`level-values tuple → encoded keys`. Concurrency pattern: each batch takes
an `Arc` snapshot of the memo (lock-free reads), computes misses locally,
and merges them back under one short lock. The first implementation took a
per-row lock and it serialized the whole pipeline across partitions —
82.9s → 4.5s on the 10M-row benchmark from this one fix. Rule: **never take
a lock per row in a UDF hot path.** Memo hits verify the full tuple, so
hash collisions cannot substitute wrong keys.

**Blob + presenter, not presented-only.** Aggregates evaluate to serialized
sketch bytes; presentation is a separate scalar UDF. This keeps the
mergeable state addressable — the serving layer's set-ops, incremental
window merges, and scatter/gather partials all consume blobs. Presented
values are derivable; blobs are not.

**Sketch formats are versioned and golden-tested** — including the hash
function. See `docs/sketches.md`.

**DataFusion 54 specifics** (for maintainers): UDF/UDAF impls must be
`Eq + Hash` (plan-node comparison — stateless ones derive it, stateful ones
use `Arc::ptr_eq` identity); scalar UDFs implement
`invoke_with_args(ScalarFunctionArgs)`; `state_fields` returns
`Vec<FieldRef>`. Pin DataFusion per release; the integration surface is
deliberately just UDFs + UDAFs + SQL text.

## Performance posture

10M rows × 18-cell lattice × (sum, count, KMV) ≈ 6s on an M-series laptop;
memory scales with cell count, not input size. The honest comparison:
DuckDB's native `GROUPING SETS` does the same scalar aggregation in ~0.5s —
it never materializes the ~17× exploded stream. Closing options, in order of
appeal: compile the lattice to DataFusion's own `GROUPING SETS` (removes the
explode entirely; sketch UDAFs work unchanged), then a specialized explode
operator. Neither blocks the roadmap: the engine's differentiators (sketch
blobs, set ops, mergeability) are orthogonal to that gap, and scatter/gather
(see `docs/scaling.md`) buys linear scale-out with zero engine work.

## Testing strategy

- **Property tests** (`cubism-core/tests/properties.rs`): string/binary
  round-trips; pruned-lattice ≡ filtered reference; sketch merge algebra.
- **Golden files**: exact serialized hex per sketch format — the tripwire
  for accidental format changes.
- **Legacy parity**: lattice counts reproduced from the original Scala test
  fixtures (17/18 cells, exactly-3-dims → 4).
- **End-to-end** (`cubism-datafusion/src/build.rs` tests): hand-computed
  cell values through the full SQL pipeline, including query-time set ops on
  output blobs.
- **Differential vs DuckDB** (benchmark scripts): exact cell equality for
  scalar measures; sketch estimates within error bounds vs exact
  `COUNT(DISTINCT)`.
