# Cubism

Declarative multidimensional aggregation with mergeable sketches — OLAP cube
lattices, approximate distinct counts, top-N, and ad-hoc set intersections
over pre-aggregated data.

A ground-up Rust rewrite of a battle-tested Spark aggregation library
(Qubism), built on Apache Arrow / DataFusion.

## Watch it work on your own Claude Code sessions

Claude Code keeps session transcripts on your machine. Cubism turns their
**metadata** (tools, models, outcomes, token counts — never message content;
nothing leaves your machine) into a cube in well under a second:

```bash
uvx maturin build --release -m bindings/cubism-py/Cargo.toml -o target/wheels
uv run --with target/wheels/cubism-*.whl python examples/claude_trace_import.py --output examples/claude_events.parquet
uv run --with target/wheels/cubism-*.whl python examples/01_your_claude_sessions.py
```

and answers, from one declarative spec: where your tokens and (nominal)
dollars go by project × model × week, which tools fail on you and how often,
which sessions were the expensive ones (real IDs you can `claude --resume`),
and how much the prompt cache is carrying.

Then put a dashboard on it — the cube parquet is a complete serving
artifact:

```bash
cargo run --release -p cubism-cli -- run examples/claude_local.yaml \
    --input examples/claude_events.parquet --output cube.parquet --show 0
cargo run --release -p cubism-cli -- serve cube.parquet   # -> http://127.0.0.1:8080
```

Slice charts, a cell inspector, and an interactive set-operations panel
that intersects any two cells' sketches at request time
([`docs/serving.md`](docs/serving.md)).

## Web analytics demo

The [`examples/web_analytics_demo`](examples/web_analytics_demo) directory is
a second end-to-end use case: a small Northstar product site, deterministic
web-event generator, Google Analytics-style cube spec, and instructions for
serving the resulting dashboard. It demonstrates acquisition slices,
distinct users and sessions, revenue, top pages, and query-time overlap of
audiences such as pricing visitors and completed signups.

That's **Act I** (`examples/01_your_claude_sessions.py`). **Act II**
(`examples/02_org_scale.py`, [rendered notebook](examples/02_org_scale.ipynb))
replays the same idea at org scale — 5M synthetic tool calls, 500 engineers,
8 weeks — where the sketch machinery starts doing things pre-aggregation
normally can't: query-time set intersections across cells that were never
aggregated together, affinity/lift from pure sketch algebra, per-cell top-k
and exemplar drill-downs, and embedding-centroid "nearest semantic cells".

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
then see the two-act demo: `examples/01_your_claude_sessions.py` (your real
data) and `examples/02_org_scale.py` (5M events, sketches, semantic cells).

## Workspace

| Crate | Purpose |
|---|---|
| `cubism-core` | Engine-agnostic model: spec types, XUnit algebra, lattice generation, binary keys, sketches (no DataFusion dependency) |
| `cubism-datafusion` | DataFusion execution: XUnit explode UDFs, sketch UDAFs + presenters, spec→SQL cube builds |
| `cubism-serve` | REST API + embedded dashboard over a cube file (`docs/serving.md`) |
| `cubism-cli` | `cubism validate` / `cubism run` / `cubism serve` (see `docs/cli.md`) |
| `bindings/cubism-py` | Python module (`pip`-installable wheel via maturin): `build_cube` → `pyarrow.Table`, sketch set algebra |

## Docs

Start here:

- [`docs/dogfooding.md`](docs/dogfooding.md) — run the analysis on your own
  Claude Code sessions, solo or as a team merging each member's cube.
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
- [`docs/serving.md`](docs/serving.md) — `cubism serve`: the JSON API and
  the embedded dashboard.

Operate and extend:

- [`docs/scaling.md`](docs/scaling.md) — the scaling strategy: why mergeable
  buffers make scatter/gather the scaling unit, and the tiers beyond one
  container.
- [`docs/extending.md`](docs/extending.md) — extension points: spec
  expressions, new sketches, UDAFs, and scalar UDFs, with toy examples and
  the end-to-end checklist.
- [`docs/high-cardinality.md`](docs/high-cardinality.md) — what happens
  when a dimension contains a UUID or raw timestamp, the
  `maxDictionaryEntries` guardrail, and the modeling patterns that fix it.

## Status

Pre-release. Implemented and tested end-to-end: the model, spec format,
filter rules, lattice generation, binary key encoding, DataFusion execution,
four sketch aggregators (KMV, top-k, exemplar sample, centroid) with
vectorized group accumulators, the CLI, and the Python bindings. Not yet:
quantiles, published packages (crates.io / PyPI), streaming ingestion.

```
cargo test          # 250 tests incl. property tests + legacy parity fixtures
cargo run -p cubism-cli -- validate examples/web_events.yaml
```

## Development

`rust-toolchain.toml` pins the toolchain `cargo`/`rustup` use automatically.
Required PR checks (`.github/workflows/ci.yml`) reproduce locally as:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
cargo build -p cubism-cli && for f in examples/*.yaml examples/web_analytics_demo/*.yaml; do
  ./target/debug/cubism validate "$f"
done                                                 # every example spec still validates
lychee --offline --include-fragments README.md 'docs/**/*.md'  # internal links + anchors
cd bindings/cubism-py && maturin build --out dist   # python bindings build smoke
```

Nightly-only checks (`.github/workflows/nightly.yml`, not required for
merge — see `deny.toml` for the license/advisory remediation policy):

```bash
cargo deny check                                                # licenses/advisories/bans/sources
cargo test -p cubism-timeseries-bench --lib -- --ignored golden_digest_sparse_1m
cd spark-adapter && sbt test                                     # optional Scala adapter, see #43
```

### SDLC

[`SDLC.md`](SDLC.md) defines one workflow for every subsystem: bounded GitHub
slice -> isolated worktree -> PR -> independent Codex review -> automatic merge.
Humans review coherent milestone outcomes through generated HTML reports and
demos. Generate the visual roadmap and in-flight view with:

```bash
rtk python3 scripts/sdlc.py status
```

Then open [`docs/project-status.html`](docs/project-status.html). The former
shared-branch time-series process is retired; its handoffs remain historical
evidence only. New human operators can start with the
[`SDLC tutorial`](docs/sdlc/HUMAN_TUTORIAL.md) or present the self-contained
[`overview slide deck`](docs/sdlc/overview-slides.html).

## License

Apache-2.0
