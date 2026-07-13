# %% [markdown]
# # Act II — Cubism at org scale
#
# Act I cubed *your* Claude Code sessions: small data, exact answers. Now
# pretend you run agent infrastructure for a ~500-engineer org: **5 million
# tool calls** over 8 weeks, 4 user segments with different habits, planted
# friction (Bash fails 4% of the time, 13x the baseline), and a 16-dim embedding on every
# event. One spec builds the cube (~40s for all 8 measures on an
# Apple-silicon laptop; scalar-only cubes are ~10x faster); the cube's
# **sketch blobs** then answer questions that pre-aggregated tables simply
# can't:
#
# 1. **query-time set operations** — sessions that used Bash AND errored,
#    intersected from two cells that were never aggregated together
# 2. **affinity (lift)** — which tools over-index per segment / among errors,
#    pure sketch algebra, recovering the planted structure
# 3. **top-k + exemplars** — every cell carries its own leaderboard and a
#    deterministic sample of session IDs to drill into
# 4. **nearest semantic cells** — rank cells by embedding centroid, then use
#    their exemplars as retrieval handles (RAG where the retrieval unit is a
#    cube cell)
#
# ```
# uv run --with target/wheels/cubism-*.whl --with numpy python examples/02_org_scale.py
# ```

# %%
import importlib.util
import time
from pathlib import Path

import numpy as np
import pyarrow.compute as pc
import pyarrow.parquet as pq

import cubism

HERE = Path("examples") if Path("examples").is_dir() else Path(".")


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


synth = load("synth_org_traces")
EVENTS = HERE / "org_events.parquet"
N_ROWS = 5_000_000
if not EVENTS.exists() or pq.read_metadata(EVENTS).num_rows != N_ROWS:
    t0 = time.perf_counter()
    pq.write_table(synth.generate(N_ROWS), EVENTS)
    print(f"synthesized {N_ROWS:,} events in {time.perf_counter() - t0:.1f}s")

# %% [markdown]
# ## Build: one spec, 5M events

# %%
t0 = time.perf_counter()
cube = cubism.build_cube((HERE / "agent_org.yaml").read_text(), str(EVENTS))
wall = time.perf_counter() - t0
cells = {r["xunit"]: r for r in cube.to_pylist()}
g = cells["/G"]
print(f"{cube.num_rows:,} cells from {g['calls']:,.0f} events in {wall:.1f}s")
print(f"org totals: ${g['cost']:,.0f}, {g['sessions']:,.0f} sessions (est), "
      f"{g['users']:,.0f} users (est)")

# %% [markdown]
# ## Plain rollups first
#
# Scalars work like any OLAP cube — here's spend by segment, and the
# error-rate-by-tool table that flags the friction tool.

# %%
def level1(dim):
    pre = f"/{dim}/"
    out = [(x.split("=", 1)[1], c) for x, c in cells.items()
           if x.startswith(pre) and "," not in x and x.count("/") == 2]
    return sorted(out, key=lambda t: -t[1]["cost"])

print(f"{'segment':<16}{'cost':>12}{'sessions':>10}{'users':>7}")
for name, c in level1("segment"):
    print(f"{name:<16}{c['cost']:>12,.0f}{c['sessions']:>10,.0f}{c['users']:>7,.0f}")

print(f"\n{'tool':<12}{'calls':>10}{'error rate':>12}")
for name, c in sorted(level1("tool"), key=lambda t: -t[1]['calls']):
    err = cells.get(f"/outcome/outcome=error,{c['xunit']}")
    print(f"{name:<12}{c['calls']:>10,.0f}{(err['calls'] if err else 0) / c['calls']:>12.1%}")

# %% [markdown]
# ## 1. Query-time set operations
#
# "How many sessions used Bash *and* hit an error?" There is no
# `bash × error` *session* table — sessions span many rows. But both cells
# carry a KMV sketch of their session IDs, so the intersection is two blob
# reads and a merge, in microseconds, without touching the 5M events.

# %%
def sketch(xunit, measure="sessions"):
    return cubism.KmvSketch.from_bytes(cells[xunit][f"{measure}__sketch"])

bash, err = sketch("/tool/tool=Bash"), sketch("/outcome/outcome=error")
both = bash.intersection_estimate(err)
print(f"sessions using Bash:      {bash.estimate():>10,.0f}")
print(f"sessions with any error:  {err.estimate():>10,.0f}")
print(f"Bash AND error (∩):       {both:>10,.0f}")

# trust check: exact answer from the raw events
ev = pq.read_table(EVENTS, columns=["session_id", "tool", "outcome"])
bash_s = pc.unique(ev.filter(pc.equal(ev["tool"], "Bash"))["session_id"]).to_pylist()
err_s = pc.unique(ev.filter(pc.equal(ev["outcome"], "error"))["session_id"]).to_pylist()
exact = len(set(bash_s) & set(err_s))
print(f"exact from raw events:    {exact:>10,} ({abs(both - exact) / exact:.1%} err — "
      f"k=1024 ⇒ σ≈3%)")

# %% [markdown]
# ## 2. Affinity: recover the planted structure with lift
#
# `lift(A, B) = (‖A∩B‖·N) / (‖A‖·‖B‖)` — over-/under-indexing from three
# sketch reads. The segment × tool lift matrix recovers each segment's
# planted tool mix (platform-eng lives in Bash and under-indexes WebSearch
# by 40%; support-eng is the mirror image), and segment × error lift shows
# who actually lives with the Bash friction. (Note what lift *can't* say:
# nearly every session touches Read at least once, so its session-level
# lift is pinned to 1.0 — universal items can't over-index. The call-level
# error-rate table above is the right lens for those.)

# %%
n = sketch("/G").estimate()

def lift(a, b):
    inter = a.intersection_estimate(b)
    return (inter * n) / (a.estimate() * b.estimate())

tools = [t for t, _ in sorted(level1("tool"), key=lambda x: x[0])]
tool_sk = {t: sketch(f"/tool/tool={t}") for t in tools}
print(f"{'lift':<14}" + "".join(f"{t:>10}" for t in tools))
for seg, _ in level1("segment"):
    s = sketch(f"/segment/segment={seg}")
    print(f"{seg:<14}" + "".join(f"{lift(s, tool_sk[t]):>10.2f}" for t in tools))

print("\nwhich segments over-index among error sessions:")
seg_lifts = [(seg, lift(sketch(f"/segment/segment={seg}"), err))
             for seg, _ in level1("segment")]
for seg, l in sorted(seg_lifts, key=lambda x: -x[1]):
    print(f"  {seg:<14} {l:.2f}")

# %% [markdown]
# ## 3. Drill down: top-k and exemplars
#
# Aggregates tell you *that* something is wrong; you fix *sessions*. Every
# cell carries a top-k leaderboard (costliest sessions) and a deterministic
# exemplar sample — real IDs to pull traces for.

# %%
hot = cells["/outcome/outcome=error/error_class=timeout,/tool/tool=Bash"]
print(f"cell {hot['xunit']}: {hot['calls']:,.0f} calls")
print("costliest sessions:",
      ", ".join(f"{k} (${v:,.2f})" for k, v in
                cubism.topk_items(hot["costliest_sessions__sketch"])[:3]))
print("exemplars to inspect:",
      ", ".join(cubism.sample_values(hot["exemplar_sessions__sketch"])[:5]))

# %% [markdown]
# ## 4. Nearest semantic cells
#
# Every cell also aggregated its events' embeddings into a **centroid**.
# Given a query vector, rank cells by cosine similarity and use the winning
# cells' exemplars for retrieval — the "corporate brain" pattern, where the
# retrieval unit is a *cube cell* ("research work in data-science, week 23")
# instead of a raw document.
#
# Here the query stands in for an embedded question like *"what
# exploratory/research work happened?"* — we borrow the generator's research
# cluster center as the query vector.

# %%
query = synth.task_centers()[synth.TASKS.index("research")]

def cosine(a, b):
    return float(np.dot(a, b) / (np.linalg.norm(a) * np.linalg.norm(b)))

ranked = sorted(
    ((cosine(query, np.array(c["semantics"])), x) for x, c in cells.items()
     if c["semantics"] is not None and c["calls"] > 500),
    reverse=True,
)
print("cells nearest the query vector:")
for score, x in ranked[:5]:
    print(f"  {score:+.3f}  {x}")

best = cells[ranked[0][1]]
print("\nretrieve via its exemplars:",
      ", ".join(cubism.sample_values(best["exemplar_sessions__sketch"])[:5]))

# %% [markdown]
# The top cells are exactly the planted `research` cells — recovered from
# nothing but per-cell centroid means.
#
# ## Why this composes
#
# Every measure here — sums, KMV, top-k, exemplars, centroids — merges
# associatively: `cube(A ∪ B) == merge(cube(A), cube(B))` cell by cell. So
# this exact cube can be built per-day and rolled up, per-team and
# federated, or streamed incrementally — and Act I's cube of your laptop's
# sessions is just one shard of an org-wide one. See `docs/scaling.md`.
