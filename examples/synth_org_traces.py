#!/usr/bin/env python3
"""Synthesize an org-scale agent-trace dataset (default 5M tool calls).

Models a ~500-engineer org running coding agents for 8 weeks. Structure is
planted so the demo has real signal to recover:

* 4 user segments with distinct tool mixes and repo pools
* task types (feature/bugfix/refactor/research/ops) with per-segment rates
* Bash is the friction tool (10% error rate); error classes conditioned on tool
* a 16-dim `embedding` per event: cluster center by task type + segment
  offset + noise — cell centroids recover task semantics

Deterministic (seeded). Writes org_events.parquet.

Usage: python examples/synth_org_traces.py [--rows N] [--output FILE]
"""

from __future__ import annotations

import argparse
import datetime as dt

import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq

SEGMENTS = ["platform-eng", "product-eng", "data-science", "support-eng"]
SEG_USERS = [100, 200, 120, 80]  # 500 users
TASKS = ["feature", "bugfix", "refactor", "research", "ops"]
TOOLS = ["Bash", "Read", "Edit", "Grep", "WebSearch", "Task"]
MODELS = ["fable-5", "sonnet-5", "haiku-4.5"]
MODEL_USD_PER_MTOK = {"fable-5": 40.0, "sonnet-5": 6.0, "haiku-4.5": 2.0}
N_REPOS = 40
WEEKS = 8
EMB_DIM = 16

# tool mix per segment (rows sum to 1): Bash Read Edit Grep WebSearch Task
SEG_TOOL_MIX = np.array([
    [0.40, 0.20, 0.24, 0.10, 0.02, 0.04],  # platform-eng: shell-heavy
    [0.18, 0.28, 0.36, 0.10, 0.04, 0.04],  # product-eng: edit-heavy
    [0.15, 0.30, 0.10, 0.20, 0.20, 0.05],  # data-science: explore-heavy
    [0.06, 0.35, 0.06, 0.28, 0.20, 0.05],  # support-eng: read/search-heavy
])
# task-type distribution per segment
SEG_TASK_MIX = np.array([
    [0.25, 0.25, 0.20, 0.10, 0.20],  # platform-eng
    [0.45, 0.30, 0.15, 0.08, 0.02],  # product-eng
    [0.25, 0.15, 0.10, 0.45, 0.05],  # data-science
    [0.10, 0.50, 0.05, 0.25, 0.10],  # support-eng
])
TOOL_ERROR_P = {"Bash": 0.04, "Edit": 0.01, "Task": 0.01}  # others 0.003
# error-class distribution per tool: timeout permission not_found crash
ERROR_CLASSES = ["timeout", "permission", "not_found", "crash"]
TOOL_ERROR_CLASS = {
    "Bash": [0.45, 0.20, 0.10, 0.25],
    "Read": [0.05, 0.15, 0.75, 0.05],
    "Grep": [0.10, 0.10, 0.75, 0.05],
    "Edit": [0.05, 0.20, 0.55, 0.20],
    "WebSearch": [0.70, 0.05, 0.15, 0.10],
    "Task": [0.60, 0.05, 0.05, 0.30],
}


def task_centers(seed: int = 42) -> np.ndarray:
    """Unit-norm embedding cluster center per task type.

    Drawn from a dedicated RNG stream so demos can reconstruct a cluster
    center (as a stand-in for an embedded query) without replaying the
    generator's whole draw sequence.
    """
    rng = np.random.default_rng(seed + 1_000)
    centers = rng.normal(0, 1, (len(TASKS), EMB_DIM))
    return centers / np.linalg.norm(centers, axis=1, keepdims=True)


def generate(n: int, seed: int = 42) -> pa.Table:
    rng = np.random.default_rng(seed)
    n_sessions = max(n // 25, 1)

    user_segment = np.repeat(np.arange(4), SEG_USERS)
    n_users = len(user_segment)
    # per-segment repo preferences: a Dirichlet over the 40 repos
    seg_repo_w = rng.dirichlet(np.full(N_REPOS, 0.3), size=4)

    # sessions are the sticky unit: one user, repo, task, model, day each
    s_user = rng.integers(0, n_users, n_sessions)
    s_seg = user_segment[s_user]
    # vectorized per-segment categorical draws
    u = rng.random(n_sessions)
    s_task = (SEG_TASK_MIX[s_seg].cumsum(axis=1) < u[:, None]).sum(axis=1)
    u = rng.random(n_sessions)
    s_repo = (seg_repo_w[s_seg].cumsum(axis=1) < u[:, None]).sum(axis=1)
    s_model = rng.choice(3, n_sessions, p=[0.25, 0.55, 0.20])
    s_week = rng.integers(0, WEEKS, n_sessions)
    s_dow = rng.integers(0, 7, n_sessions)

    monday0 = dt.date(2026, 5, 4)  # 2026-W19 monday
    week_str = np.array([f"2026-W{19 + w:02d}" for w in range(WEEKS)])
    day_str = np.array([(monday0 + dt.timedelta(weeks=w, days=d)).isoformat()
                        for w in range(WEEKS) for d in range(7)])

    # events
    ev_s = rng.integers(0, n_sessions, n)
    seg, task = s_seg[ev_s], s_task[ev_s]
    u = rng.random(n)
    mix = SEG_TOOL_MIX[seg].copy()
    # research tasks lean into search tools regardless of segment
    research = task == 3
    mix[research] = mix[research] * np.array([0.5, 1.0, 0.5, 1.5, 3.0, 1.0])
    mix /= mix.sum(axis=1, keepdims=True)
    tool_i = (mix.cumsum(axis=1) < u[:, None]).sum(axis=1)
    tool = np.array(TOOLS)[tool_i]

    err_p = np.full(n, 0.003)
    for t, p in TOOL_ERROR_P.items():
        err_p[tool == t] = p
    is_err = rng.random(n) < err_p
    eclass = np.full(n, "", dtype=object)
    for i, t in enumerate(TOOLS):
        m = is_err & (tool == t)
        eclass[m] = rng.choice(ERROR_CLASSES, m.sum(), p=TOOL_ERROR_CLASS[t])

    tokens = rng.gamma(2.0, 400, n).astype("int64") + 50
    tokens[tool == "Task"] *= 8
    model = np.array(MODELS)[s_model[ev_s]]
    usd = np.array([MODEL_USD_PER_MTOK[m] for m in MODELS])[s_model[ev_s]]
    cost = tokens * usd / 1e6

    # embeddings: task-type cluster centers + segment offset + noise
    centers = task_centers(seed)
    seg_off = rng.normal(0, 1, (4, EMB_DIM)) * 0.25
    emb = centers[task] + seg_off[seg] + rng.normal(0, 0.35, (n, EMB_DIM))
    flat = emb.astype(np.float64).ravel()
    offsets = np.arange(0, (n + 1) * EMB_DIM, EMB_DIM, dtype=np.int32)
    emb_arr = pa.ListArray.from_arrays(offsets, flat)

    return pa.table({
        "session_id": np.char.add("s", ev_s.astype("U7")),
        "user": np.char.add("u", s_user[ev_s].astype("U3")),
        "segment": np.array(SEGMENTS)[seg],
        "repo": np.char.add("repo-", s_repo[ev_s].astype("U2")),
        "task_type": np.array(TASKS)[task],
        "model": model,
        "tool": tool,
        "outcome": np.where(is_err, "error", "ok"),
        "error_class": np.where(is_err, eclass.astype("U10"), None),
        "week": week_str[s_week[ev_s]],
        "day": day_str[s_week[ev_s] * 7 + s_dow[ev_s]],
        "tokens": tokens,
        "cost_usd": cost,
        "embedding": emb_arr,
    })


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rows", type=int, default=5_000_000)
    ap.add_argument("--output", default="org_events.parquet")
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()
    t = generate(args.rows, args.seed)
    # small row groups matter: they are DataFusion's scan-parallelism unit
    pq.write_table(t, args.output, row_group_size=131_072)
    print(f"{t.num_rows:,} events, {t.nbytes / 1e6:,.0f} MB in memory -> {args.output}")


if __name__ == "__main__":
    main()
