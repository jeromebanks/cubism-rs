# %% [markdown]
# # Agent-trace analytics with Cubism
#
# Agent sessions produce a firehose of tool-call events: which model, which
# tool, how many tokens, did it error. This demo aggregates 300k synthetic
# tool calls into a **cube** with one declarative spec, then uses the cube's
# stored **sketch blobs** to answer questions pre-aggregation normally
# can't:
#
# * distinct sessions per (model, tool, outcome) slice — approximate, mergeable
# * costliest sessions per slice (top-k)
# * **ad-hoc set operations over slices that were never aggregated
#   together** — "how many sessions used Bash AND hit an error?" — computed
#   at query time by intersecting two cells' KMV sketches
# * affinity (lift): which tools over-index among error sessions
#
# Runnable as a script or a notebook (percent format):
# ```
# uv run --with target/wheels/cubism-*.whl --with numpy python examples/agent_trace_demo.py
# ```

# %% [markdown]
# ## 1. Synthesize agent traces
#
# ~300k tool calls across 20k sessions. Sessions have hidden *personas* —
# coders (Bash/Edit-heavy), researchers (WebSearch/Grep), orchestrators
# (Task-heavy) — and Bash calls fail 6× more often than anything else, so
# the overlap questions have real structure to discover.

# %%
import tempfile

import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq

import cubism

rng = np.random.default_rng(7)
n = 300_000
n_sessions = 20_000  # ~15 calls per session

models = np.array(["opus-4.8", "sonnet-5", "haiku-4.5"])
families = {"opus-4.8": "opus", "sonnet-5": "sonnet", "haiku-4.5": "haiku"}
tools = np.array(["Bash", "Read", "Edit", "Grep", "WebSearch", "Task"])

persona_of_session = rng.choice(3, n_sessions, p=[0.5, 0.3, 0.2])
tool_mix = np.array([
    [0.35, 0.25, 0.30, 0.08, 0.01, 0.01],  # coder
    [0.02, 0.35, 0.03, 0.30, 0.28, 0.02],  # researcher
    [0.05, 0.20, 0.05, 0.05, 0.05, 0.60],  # orchestrator
])

session_id = rng.integers(0, n_sessions, n)
model = models[session_id % 3]  # each session sticks to one model
persona = persona_of_session[session_id]
u = rng.random(n)
tool = tools[(tool_mix[persona].cumsum(axis=1) < u[:, None]).sum(axis=1)]

error_p = np.where(tool == "Bash", 0.12, 0.02)  # Bash fails 12%, others 2%
outcome = np.where(rng.random(n) < error_p, "error", "ok")

tokens = rng.gamma(shape=2.0, scale=400, size=n).astype("int64") + 50
tokens[tool == "Task"] *= 8  # subagents are pricey

table = pa.table({
    "session_id": pa.array([f"s{v}" for v in session_id]),
    "model_family": pa.array([families[m] for m in model]),
    "model": pa.array(model),
    "tool": pa.array(tool),
    "outcome": pa.array(outcome),
    "tokens": pa.array(tokens),
})
events = tempfile.mktemp(suffix=".parquet")
pq.write_table(table, events)
print(f"traces: {n:,} tool calls, {n_sessions:,} sessions")

# %% [markdown]
# ## 2. Declare the cube
#
# The spec *is* the pipeline: dimensions (with a model-family hierarchy),
# lattice pruning, and four measures — two scalar, two sketch-backed.

# %%
SPEC = """
apiVersion: v1
name: agent_traces
dimensions:
  - name: model
    levels: [model_family, model]
  - name: tool
  - name: outcome
filterRules:
  - type: max_dimensions
    n: 2
measures:
  - name: tokens
    agg: sum
    input: tokens
  - name: calls
    agg: count
  - name: sessions
    agg: count_distinct
    input: session_id
  - name: costliest_sessions
    agg: top_k
    input: session_id
    by: tokens
includeGlobal: true
"""

cube = cubism.build_cube(SPEC, events)
print(f"cube: {cube.num_rows} cells, columns = {cube.column_names}")

cells = {row["xunit"]: row for row in cube.to_pylist()}

# %% [markdown]
# ## 3. Ordinary slice/dice
#
# Cells are plain rows — read them like any table.

# %%
print("token spend by tool:")
tool_cells = sorted(
    (c for x, c in cells.items() if x.startswith("/tool/") and x.count("/") == 2),
    key=lambda c: -c["tokens"],
)
for c in tool_cells:
    name = c["xunit"].split("=")[1]
    print(f"  {name:<10} {c['tokens']:>12,} tokens  {c['calls']:>8,} calls  "
          f"{c['sessions']:>7,.0f} sessions")

top = cubism.topk_items(cells["/G"]["costliest_sessions__sketch"])[:3]
print("\ncostliest sessions overall:", ", ".join(f"{k} ({s:,.0f} tok)" for k, s in top))

# %% [markdown]
# ## 4. The differentiator: query-time set operations
#
# `sessions__sketch` blobs are KMV sketches — mergeable, intersectable.
# "Sessions that used Bash AND errored" intersects two cells that were never
# aggregated together, in microseconds, without re-reading a single event.

# %%
sketch = lambda xunit: cubism.KmvSketch.from_bytes(cells[xunit]["sessions__sketch"])

bash = sketch("/tool/tool=Bash")
err = sketch("/outcome/outcome=error")
print(f"sessions using Bash:               {bash.estimate():>8,.0f}")
print(f"sessions hitting errors:           {err.estimate():>8,.0f}")
print(f"sessions with BOTH (query-time ∩): {bash.intersection_estimate(err):>8,.0f}")
print(f"jaccard(Bash, error) = {bash.jaccard(err):.3f}")

# %% [markdown]
# The all-pairs overlap matrix recovers the hidden personas from nothing but
# per-cell sketches — the coder cluster (Bash↔Edit), the researcher cluster
# (Read↔Grep↔WebSearch), and the isolated orchestrators (Task).

# %%
names = ["Bash", "Read", "Edit", "Grep", "WebSearch", "Task"]
sketches = {t: sketch(f"/tool/tool={t}") for t in names}
print("            " + "".join(f"{t:>10}" for t in names))
for a in names:
    row = "".join(f"{sketches[a].jaccard(sketches[b]):>10.2f}" for b in names)
    print(f"  {a:<10}{row}")

# %% [markdown]
# ## 5. Affinity: what over-indexes among error sessions?
#
# Lift = P(tool | error) / P(tool), computed from two sketch ops and the
# global cell. Bash over-indexes (it's the friction point); Edit follows
# because coder sessions carry it; researcher tools under-index.

# %%
n_total = sketch("/G").estimate()
print("tool affinity to error sessions (lift; >1 over-indexes):")
for t in names:
    s = sketches[t]
    lift = (s.intersection_estimate(err) * n_total) / (s.estimate() * err.estimate())
    print(f"  {t:<10} {lift:5.2f}")

# %% [markdown]
# ## 6. Trust check
#
# Estimates against exact counts from the raw events — expect ~3–5% at the
# default sketch size (k=1024).

# %%
import pyarrow.compute as pc

exact_bash = len(pc.unique(table.filter(pc.equal(table["tool"], "Bash"))["session_id"]))
est_bash = bash.estimate()
print(f"exact distinct Bash sessions: {exact_bash:,}; sketch: {est_bash:,.0f} "
      f"({abs(est_bash - exact_bash) / exact_bash:.1%} err)")
