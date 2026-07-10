# Cubism

Declarative multidimensional aggregation with mergeable sketches — OLAP cube
lattices, approximate distinct counts, top-N, and ad-hoc set intersections
over pre-aggregated data.

A ground-up Rust rewrite of a battle-tested Spark aggregation library
(Qubism), built on Apache Arrow / DataFusion.

## The model

- **YPath** — one dimension's hierarchical coordinate: `/geo/country=CZ/city=Prague`
- **XUnit** — one cube cell: a conjunction of YPaths, e.g. `/gender/gender=F,/geo/country=CZ`
- **Filter rules** prune the cube lattice (`max_dimensions`, `not_together`, …)
- **Mergeable aggregators** — sums/counts plus sketches (KMV distinct-count,
  top-N, quantiles, embedding centroids) whose buffers merge associatively,
  enabling incremental builds and query-time set operations.

Cubes are declared in YAML:

```yaml
apiVersion: v1
name: web_events
dimensions:
  - name: geo
    levels: [country, city]
  - name: gender
filterRules:
  - type: max_dimensions
    n: 3
measures:
  - name: reach
    agg: count_distinct
    input: user_id
includeGlobal: true
```

## Python

```python
import cubism

table = cubism.build_cube(spec_yaml, "events.parquet")  # -> pyarrow.Table

cells = {r["xunit"]: r for r in table.to_pylist()}
bash = cubism.KmvSketch.from_bytes(cells["/tool/tool=Bash"]["sessions__sketch"])
err = cubism.KmvSketch.from_bytes(cells["/outcome/outcome=error"]["sessions__sketch"])
bash.intersection_estimate(err)   # sessions that used Bash AND errored —
                                  # a set op over cells never aggregated together
```

Build the wheel with `uvx maturin build --release -m bindings/cubism-py/Cargo.toml`,
then see `examples/agent_trace_demo.py` for a full agent-observability walkthrough.

## Workspace

| Crate | Purpose |
|---|---|
| `cubism-core` | Engine-agnostic model: spec types, XUnit algebra, lattice generation, binary keys, sketches (no DataFusion dependency) |
| `cubism-datafusion` | DataFusion execution: XUnit explode operator + sketch UDAFs |
| `cubism-cli` | `cubism validate <spec.yaml>` today; `cubism run` next |

## Docs

Start here:

- [`docs/concepts.md`](docs/concepts.md) — the data model: YPaths, XUnits,
  the cube lattice, filter rules, and why mergeable measures are the point.
- [`docs/spec-reference.md`](docs/spec-reference.md) — every spec field,
  filter rule, and measure kind, with validation behavior.

Go deeper:

- [`docs/architecture.md`](docs/architecture.md) — crates, the spec→SQL
  execution pipeline, and the design decisions with their reasons.
- [`docs/sketches.md`](docs/sketches.md) — sketch algorithms, error bounds,
  and the versioned byte formats.
- [`docs/python.md`](docs/python.md) — Python API reference and recipes
  (overlap, Jaccard matrices, affinity/lift, incremental merge).
- [`docs/cli.md`](docs/cli.md) — `cubism validate` / `cubism run`.

Operate and extend:

- [`docs/scaling.md`](docs/scaling.md) — the scaling strategy: why mergeable
  buffers make scatter/gather the scaling unit, and the tiers beyond one
  container.
- [`docs/extending.md`](docs/extending.md) — extension points: spec
  expressions, new sketches, UDAFs, and scalar UDFs, with toy examples and
  the end-to-end checklist.

## Status

Early development (M1 of the roadmap): the model, spec format, filter rules,
lattice generation, and binary key encoding are implemented and tested.
Execution (DataFusion), sketches, and Python bindings are next.

```
cargo test          # 41 tests incl. property tests + legacy parity fixtures
cargo run -p cubism-cli -- validate examples/web_events.yaml
```

## License

Apache-2.0
