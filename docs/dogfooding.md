# Analyze your own Claude Code sessions

A self-contained guide for running Cubism on your own machine's Claude Code
history — no prior knowledge of this repo required. Ten minutes, nothing
leaves your laptop.

## What you get

Claude Code stores a transcript of every session under
`~/.claude/projects/`. Cubism turns the **metadata** of those transcripts
into a cube and answers:

- where your tokens and (nominal) dollars go — by project, model, and week
- which tools fail on you, and how often
- which sessions were the expensive ones (real IDs — `claude --resume` them)
- how much work the prompt cache is doing

**Privacy, stated precisely:** the importer reads only structural metadata —
tool names, model names, ok/error flags, timestamps, token counts, project
directory names, session IDs. It never reads your prompts, Claude's replies,
tool inputs, or tool outputs (`examples/claude_trace_import.py` is ~200
lines; the schema whitelist is asserted at the end — audit it). Everything
runs and stays local; the generated parquet files are gitignored.

## Prerequisites

- A Rust toolchain (`rustup`) and [`uv`](https://docs.astral.sh/uv/) —
  both installable via Homebrew
- Some Claude Code usage history (a handful of sessions is enough)

## Run it

From the repo root:

```bash
# 1. build the Python wheel (~2 min the first time)
uvx maturin build --release -m bindings/cubism-py/Cargo.toml -o target/wheels

# 2. import your transcripts (metadata only) -> parquet
uv run --with target/wheels/cubism-*.whl python examples/claude_trace_import.py \
    --output examples/claude_events.parquet

# 3. build the cube and print the analysis
uv run --with target/wheels/cubism-*.whl python examples/01_your_claude_sessions.py
```

Prefer a notebook? `examples/01_your_claude_sessions.ipynb` is the same
content (committed without outputs on purpose — run it yourself):

```bash
uv run --with target/wheels/cubism-*.whl --with jupyterlab jupyter lab \
    examples/01_your_claude_sessions.ipynb
```

## Reading the results

- **Costs are nominal API list prices** — what your usage *would* bill at
  per-token rates. On a subscription plan, read them as a utilization
  measure, not an invoice. The price table is the `PRICES` dict at the top
  of `claude_trace_import.py`; edit it to match current pricing.
- `TextReply` rows are assistant turns with no tool call — they carry
  tokens, so they're kept for honest totals.
- The costliest-session IDs are real: `claude --resume <id>` opens one.
- Token attribution is approximate at the tool-call level (usage is
  reported per message, split evenly across its tool calls) but exact at
  session/project/week granularity.

## Bonus: a live dashboard

Prefer clicking to reading printed tables? Serve the cube:

```bash
cargo run --release -p cubism-cli -- run examples/claude_local.yaml \
    --input examples/claude_events.parquet --output cube.parquet --show 0
cargo run --release -p cubism-cli -- serve cube.parquet
```

Open <http://127.0.0.1:8080>: cost/token charts by any dimension, a cell
inspector, and a set-operations panel that intersects any two slices
("sessions that used Bash AND errored") live from the stored sketches.
Local-only (binds 127.0.0.1); see `docs/serving.md`.

## Make it yours

The cube is declared in `examples/claude_local.yaml` — it's ~40 lines and
the whole pipeline. Some easy edits (validate with
`cargo run -p cubism-cli -- validate examples/claude_local.yaml`):

- raise `max_dimensions` to 3 to get project × tool × outcome cells
- add a `top_k` measure with `by: tokens` for token-hungriest sessions
- slice by `loop` (main agent vs subagent) if you use subagent-heavy flows

The result of `cubism.build_cube` is a plain `pyarrow.Table` — pandas,
DuckDB, and Polars all consume it directly (`docs/python.md` has recipes).

## Team mode: merge everyone's cubes

Every Cubism measure — including the sketch blobs — merges associatively:
`cube(A ∪ B) == merge(cube(A), cube(B))`, cell by cell. So a small team can
get org-style analytics **without anyone sharing raw transcripts**: each
member runs the import + build locally and shares only their *cube* (a few
hundred KB of aggregates), and anyone can merge the cubes.

Each member runs:

```bash
uv run --with target/wheels/cubism-*.whl python examples/claude_trace_import.py \
    --output my_events.parquet
cargo run --release -p cubism-cli -- run examples/claude_local.yaml \
    --input my_events.parquet --output alice_cube.parquet --show 0
```

Then anyone merges the shared cube files:

```python
import cubism
import pyarrow.parquet as pq
from collections import defaultdict

SCALARS = ["cost", "tokens", "cache_read", "calls"]

merged = defaultdict(lambda: {"sessions": cubism.KmvSketch(), **{m: 0 for m in SCALARS}})
for path in ["alice_cube.parquet", "bob_cube.parquet", "carol_cube.parquet"]:
    for row in pq.read_table(path).to_pylist():
        cell = merged[row["xunit"]]          # cells align on the canonical string
        for m in SCALARS:
            cell[m] += row[m]
        cell["sessions"] = cell["sessions"].merge(
            cubism.KmvSketch.from_bytes(row["sessions__sketch"]))

team = merged["/tool/tool_group=builtin/tool=Bash"]
print(f"team Bash: {team['calls']:,} calls, ${team['cost']:,.2f}, "
      f"{team['sessions'].estimate():,.0f} distinct sessions")
```

Distinct-session counts stay correct across members because KMV union is
exact set union in hash space — no double counting, no coordination. The
same pattern extends to `top_k` and `reservoir_sample` blobs
(`cubism.topk_items` / `cubism.sample_values` decode them;
`docs/scaling.md` explains why this is also the scaling strategy).

What *is* shared in a cube file, so you can decide if it's appropriate:
project directory names, session UUIDs, and per-slice token/cost
aggregates. If project names are sensitive, rename them at import time (a
one-line edit in `claude_trace_import.py`) — the cubes still merge.

## Troubleshooting

- **`no transcript events found`** — check `ls ~/.claude/projects/`; pass
  `--claude-dir` if your history lives elsewhere.
- **maturin can't find cargo** — with Homebrew rustup, cargo lives at
  `/opt/homebrew/opt/rustup/bin`; make sure it's on `PATH`.
- **Numbers look too small** — transcripts are per-machine; your other
  laptop's sessions aren't here (that's what team mode is for).
- Want the big-data version afterwards? `examples/02_org_scale.py` runs the
  same analysis on 5M synthetic events with set operations, lift, and
  semantic cells.
