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

## Workspace

| Crate | Purpose |
|---|---|
| `cubism-core` | Engine-agnostic model: spec types, XUnit algebra, lattice generation, binary keys, sketches (no DataFusion dependency) |
| `cubism-datafusion` | DataFusion execution: XUnit explode operator + sketch UDAFs |
| `cubism-cli` | `cubism validate <spec.yaml>` today; `cubism run` next |

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
