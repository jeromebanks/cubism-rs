# Python API

The `cubism` module is a pyo3 extension exposing the engine and the sketch
algebra. Cube results arrive as `pyarrow.Table` via the Arrow C data
interface — no serialization, no schema translation.

## Install / build

Until the PyPI release, build the wheel locally:

```bash
uvx maturin build --release -m bindings/cubism-py/Cargo.toml -o target/wheels
uv run --with target/wheels/cubism-*.whl python your_script.py
```

## Functions

### `cubism.build_cube(spec_yaml: str, input_path: str) -> pyarrow.Table`

Validates the spec, builds the cube over a parquet (or `.csv`) file, and
returns the result table. The GIL is released for the duration of the build.
Columns: `xunit` (canonical cell string), then measures in spec order —
sketch measures contribute `{name}` (presented) plus `{name}__sketch`
(binary blob).

Raises `ValueError` for spec problems (with every validation error listed)
and `RuntimeError` for execution problems.

```python
table = cubism.build_cube(SPEC, "events.parquet")
cells = {r["xunit"]: r for r in table.to_pylist()}   # small cubes
# or stay columnar: table.filter(...), table.to_pandas(), duckdb.from_arrow(table)
```

### `cubism.validate_spec(spec_yaml: str) -> str`

Parse + validate without building; returns the cube name. `ValueError` on
any problem — use it for fast feedback in editors/CI.

### `cubism.topk_items(blob: bytes) -> list[tuple[str, float]]`

Decode a `top_k` sketch blob into `(key, summed_score)` pairs, best-first.

### `cubism.sample_values(blob: bytes) -> list[str]`

Decode a `reservoir_sample` blob into its sampled distinct values.

## `cubism.KmvSketch`

The distinct-count sketch behind `count_distinct` measures, exposed for
query-time set algebra over stored blobs.

| Member | Meaning |
|---|---|
| `KmvSketch(k=1024)` | new empty sketch |
| `KmvSketch.from_bytes(blob)` *(static)* | deserialize a `*__sketch` value |
| `.to_bytes() -> bytes` | serialize (format v1) |
| `.merge(other) -> KmvSketch` | union (associative + commutative) |
| `.estimate() -> float` | distinct count (exact while under-full) |
| `.union_estimate(other) -> float` | ‖A ∪ B‖ |
| `.intersection_estimate(other) -> float` | ‖A ∩ B‖ (inclusion–exclusion, ≥ 0) |
| `.jaccard(other) -> float` | ‖A∩B‖ / ‖A∪B‖ |
| `.k` | sketch capacity |

Error characteristics and caveats (small intersections of big sets are
noisy): see `docs/sketches.md`.

## Recipes

### Overlap between any two slices

```python
def sketch(cells, xunit, measure="sessions"):
    return cubism.KmvSketch.from_bytes(cells[xunit][f"{measure}__sketch"])

bash, err = sketch(cells, "/tool/tool=Bash"), sketch(cells, "/outcome/outcome=error")
print(bash.intersection_estimate(err), bash.jaccard(err))
```

### All-pairs overlap matrix for one dimension

```python
slices = {x.split("=")[1]: sketch(cells, x)
          for x in cells if x.startswith("/tool/") and "," not in x}
matrix = {(a, b): sa.jaccard(sb)
          for a, sa in slices.items() for b, sb in slices.items()}
```

### Affinity (lift): which slices over-index against a segment

`lift(A, B) = P(A|B) / P(A) = (|A∩B| · N) / (|A| · |B|)` — computed entirely
from stored sketches plus the global cell:

```python
def lift(cells, a_xunit, b_xunit, measure="sessions"):
    a, b = sketch(cells, a_xunit, measure), sketch(cells, b_xunit, measure)
    n = sketch(cells, "/G", measure).estimate()
    inter = a.intersection_estimate(b)
    return (inter * n) / (a.estimate() * b.estimate()) if inter else 0.0

# tools that over-index among error sessions:
for x in cells:
    if x.startswith("/tool/") and "," not in x:
        print(x, round(lift(cells, x, "/outcome/outcome=error"), 2))
```

Lift > 1 over-indexes, < 1 under-indexes. (This is the legacy library's
"Affinity" computation, reduced to two sketch ops and the `/G` cell.)

### Incremental merge across builds

Cubes from different runs merge cell-by-cell on the `xunit` string:

```python
merged = day1_sketch.merge(day2_sketch)     # rolling-window reach, no recompute
```

### Trust check against exact counts

```python
import pyarrow.compute as pc
exact = len(pc.unique(events.filter(pc.equal(events["tool"], "Bash"))["session_id"]))
est = sketch(cells, "/tool/tool=Bash").estimate()   # expect ~3% at k=1024
```

## Worked example

`examples/agent_trace_demo.py` runs the full story on synthetic agent
traces: spend by tool, costliest sessions, Bash∩error overlap, the
tool-overlap Jaccard matrix, and an estimate-vs-exact check.
