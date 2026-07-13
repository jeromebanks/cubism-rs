# %% [markdown]
# # Act I — Cubism on *your own* Claude Code sessions
#
# Claude Code keeps full session transcripts on your machine
# (`~/.claude/projects/*.jsonl`). This notebook turns their **metadata** —
# tool names, models, outcomes, token counts; never message content — into a
# cube, and answers the questions you actually have about your agent usage:
#
# * where do my tokens (and nominal dollars) go, by project / model / week?
# * which tools fail on me, and how often?
# * which sessions were the expensive ones? (real IDs — go `claude --resume` them)
# * how much is the prompt cache carrying?
#
# Everything runs locally; nothing is uploaded anywhere. Run it:
# ```
# uvx maturin build --release -m bindings/cubism-py/Cargo.toml -o target/wheels
# uv run --with target/wheels/cubism-*.whl python examples/claude_trace_import.py --output examples/claude_events.parquet
# uv run --with target/wheels/cubism-*.whl python examples/01_your_claude_sessions.py
# ```
#
# This act is small data with exact answers. Act II
# (`02_org_scale.py`) is the same idea at 5M events, where the sketch
# machinery starts doing things plain aggregation can't.

# %%
from pathlib import Path

import cubism

HERE = Path("examples") if Path("examples").is_dir() else Path(".")
EVENTS = HERE / "claude_events.parquet"
if not EVENTS.exists():
    raise SystemExit(f"run examples/claude_trace_import.py --output {EVENTS} first")

cube = cubism.build_cube((HERE / "claude_local.yaml").read_text(), str(EVENTS))
cells = {r["xunit"]: r for r in cube.to_pylist()}
g = cells["/G"]
print(f"cube: {cube.num_rows} cells from {g['calls']:,.0f} events, "
      f"{g['sessions']:,.0f} sessions, ${g['cost']:,.2f} nominal API cost")

# %% [markdown]
# *(Costs are nominal API list prices from the table in
# `claude_trace_import.py` — if you're on a subscription plan they're what
# your usage* would *bill at, i.e. a utilization measure, not an invoice.)*
#
# ## Where the money goes
#
# One-dimension cells are ordinary rollups — read them like a GROUP BY.

# %%
def level1(dim):
    """Cells one level deep in a single dimension, best-first by cost."""
    pre = f"/{dim}/"
    out = [c for x, c in cells.items()
           if x.startswith(pre) and "," not in x and x.count("/") == 2]
    return sorted(out, key=lambda c: -c["cost"])

print(f"{'project':<28}{'cost':>10}{'tokens':>12}{'sessions':>10}")
for c in level1("project")[:8]:
    name = c["xunit"].split("=", 1)[1]
    print(f"{name:<28}{c['cost']:>10,.2f}{c['tokens']:>12,.0f}{c['sessions']:>10,.0f}")

# %% [markdown]
# ## Model mix, week by week
#
# `model` and `time` are hierarchical dimensions (`model_family/model`,
# `week/day`); two-dimension cells give the trend without any query engine.

# %%
weeks = sorted(c["xunit"].split("=")[1] for c in level1("time"))
fams = [c["xunit"].split("=")[1] for c in level1("model")]
print(f"{'cost by week':<14}" + "".join(f"{w:>10}" for w in weeks))
for f in fams:
    row = [cells.get(f"/model/model_family={f},/time/week={w}") for w in weeks]
    print(f"{f:<14}" + "".join(f"{(c['cost'] if c else 0):>10,.2f}" for c in row))

# %% [markdown]
# ## Which tools fail on me?
#
# Error rate per tool needs the *pair* cells (`tool × outcome`) over the
# `tool` cells — both are already in the cube.

# %%
tools = [(c["xunit"].split("=")[-1], c) for x, c in cells.items()
         if x.startswith("/tool/") and "," not in x and x.count("/") == 3]
tools.sort(key=lambda t: -t[1]["calls"])
print(f"{'tool':<22}{'calls':>8}{'errors':>8}{'rate':>8}")
for name, c in tools[:10]:
    err = cells.get(f"/outcome/outcome=error,{c['xunit']}")
    n_err = err["calls"] if err else 0
    print(f"{name[:21]:<22}{c['calls']:>8,.0f}{n_err:>8,.0f}{n_err / c['calls']:>8.1%}")

# %% [markdown]
# ## The sessions worth investigating
#
# `costliest_sessions` is a mergeable top-k sketch; every cell carries its
# own leaderboard. These are real session IDs — inspect one with
# `claude --resume <id>`.

# %%
for sid, cost in cubism.topk_items(g["costliest_sessions__sketch"])[:5]:
    print(f"  {sid}  ${cost:,.2f}")

# %% [markdown]
# ## How hard is the prompt cache working?

# %%
for c in level1("model"):
    fam = c["xunit"].split("=")[1]
    total_in = c["cache_read"] + c["tokens"]
    print(f"  {fam:<10} {c['cache_read'] / total_in:.0%} of input volume was cache reads "
          f"({c['cache_read'] / 1e6:,.0f}M tokens)")

# %% [markdown]
# Small data, exact answers — but every measure in this cube (including the
# top-k blobs) is *mergeable*: tomorrow's sessions can be cubed alone and
# merged in, and a whole org's worth of these cubes rolls up the same way.
# That's Act II: `02_org_scale.py`.
