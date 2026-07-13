#!/usr/bin/env python3
"""Import local Claude Code session transcripts into a tool-call event table.

Scans ``~/.claude/projects/*/*.jsonl`` and writes one parquet row per tool
call (plus one per text-only assistant reply, so token/cost totals are
complete). The output feeds ``examples/claude_local.yaml`` /
``examples/01_your_claude_sessions.py``.

PRIVACY: this script reads METADATA ONLY. It never extracts message text,
tool inputs, or tool outputs — the output schema is a fixed whitelist of
identifiers, names, flags, timestamps, and token counts, and nothing leaves
your machine. Keep the output parquet local (it is gitignored).

Token attribution: the API reports usage per assistant *message*, not per
tool call. We split a message's output tokens evenly across its rows and
count input + cache-read tokens once, on the message's first row. Cost uses
the per-model price table below — edit PRICES to match your plan.

A natural follow-on with the same output schema: adapters for Codex session
logs and OpenTelemetry GenAI spans.

Usage:
    python examples/claude_trace_import.py [--claude-dir DIR] [--output FILE]
"""

from __future__ import annotations

import argparse
import datetime as dt
import glob
import json
import os

import pyarrow as pa
import pyarrow.parquet as pq

# USD per million tokens: (input, output). Cache reads bill at 10% of input.
PRICES = {
    "opus": (15.0, 75.0),
    "sonnet": (3.0, 15.0),
    "haiku": (1.0, 5.0),
    "fable": (15.0, 75.0),  # placeholder — adjust to published pricing
}
DEFAULT_PRICE = (3.0, 15.0)

# The output schema is a whitelist; import_events() asserts nothing else leaks.
COLUMNS = [
    "session_id", "project", "model_family", "model", "tool_group", "tool",
    "outcome", "loop", "week", "day", "tokens_in", "tokens_out",
    "tokens_cache_read", "tokens", "cost_usd",
]


def model_family(model: str) -> str:
    for fam in PRICES:
        if fam in model:
            return fam
    return "other"


def tool_group(tool: str) -> str:
    if tool.startswith("mcp__"):
        server = tool.split("__")[1].removeprefix("plugin_").removeprefix("claude_ai_")
        words = server.split("_")
        deduped = [w for i, w in enumerate(words) if i == 0 or w != words[i - 1]]
        return "mcp:" + "_".join(deduped)
    if tool == "TextReply":
        return "text"
    return "builtin"


def time_levels(ts: str) -> tuple[str, str]:
    t = dt.datetime.fromisoformat(ts.replace("Z", "+00:00"))
    iso = t.isocalendar()
    return f"{iso.year}-W{iso.week:02d}", t.strftime("%Y-%m-%d")


def scan_session(path: str, project: str, rows: list[dict], seen_tool_use: set[str]) -> None:
    """One transcript file → rows. Two passes: collect error flags, then emit."""
    events = []
    is_error: dict[str, bool] = {}  # tool_use_id -> tool_result is_error
    with open(path, encoding="utf-8") as f:
        for line in f:
            try:
                d = json.loads(line)
            except json.JSONDecodeError:
                continue
            kind = d.get("type")
            if kind not in ("user", "assistant"):
                continue
            content = (d.get("message") or {}).get("content")
            if kind == "user":
                if isinstance(content, list):
                    for b in content:
                        if isinstance(b, dict) and b.get("type") == "tool_result":
                            tid = b.get("tool_use_id")
                            if tid:
                                is_error[tid] = bool(b.get("is_error"))
            else:
                events.append(d)

    for d in events:
        m = d.get("message") or {}
        ts = d.get("timestamp")
        if not ts:
            continue
        model = m.get("model") or ""
        if not model or model.startswith("<"):  # "<synthetic>" injected events
            continue
        content = m.get("content")
        calls = [b for b in content if isinstance(b, dict) and b.get("type") == "tool_use"] \
            if isinstance(content, list) else []
        # retried requests can repeat a message; tool_use ids are globally unique
        calls = [b for b in calls if b.get("id") and b["id"] not in seen_tool_use]
        seen_tool_use.update(b["id"] for b in calls)

        usage = m.get("usage") or {}
        tok_in = usage.get("input_tokens") or 0
        tok_out = usage.get("output_tokens") or 0
        tok_cache = usage.get("cache_read_input_tokens") or 0
        week, day = time_levels(ts)
        sid = d.get("sessionId") or os.path.basename(path).removesuffix(".jsonl")
        loop = "subagent" if d.get("isSidechain") else "main"
        cwd = d.get("cwd")
        proj = os.path.basename(cwd) if cwd else project

        units = calls or [None]  # a text-only reply still carries tokens
        out_share = tok_out / len(units)
        p_in, p_out = PRICES.get(model_family(model), DEFAULT_PRICE)
        for i, b in enumerate(units):
            row_in = tok_in if i == 0 else 0
            row_cache = tok_cache if i == 0 else 0
            tool = b["name"] if b else "TextReply"
            rows.append({
                "session_id": sid,
                "project": proj,
                "model_family": model_family(model),
                "model": model,
                "tool_group": tool_group(tool),
                "tool": tool,
                "outcome": ("error" if b and is_error.get(b["id"]) else "ok"),
                "loop": loop,
                "week": week,
                "day": day,
                "tokens_in": row_in,
                "tokens_out": int(out_share),
                "tokens_cache_read": row_cache,
                "tokens": row_in + int(out_share),
                "cost_usd": (row_in * p_in + out_share * p_out
                             + row_cache * p_in * 0.1) / 1e6,
            })


def import_events(claude_dir: str) -> pa.Table:
    rows: list[dict] = []
    seen: set[str] = set()
    files = sorted(glob.glob(os.path.join(claude_dir, "*", "*.jsonl")))
    for path in files:
        # project dir names munge the cwd; the cwd field on events is authoritative
        project = os.path.basename(os.path.dirname(path)).split("-")[-1]
        scan_session(path, project, rows, seen)
    if not rows:
        raise SystemExit(f"no transcript events found under {claude_dir}")
    assert all(set(r) == set(COLUMNS) for r in rows), "schema whitelist violated"
    return pa.Table.from_pylist(rows, schema=pa.schema(
        [(c, pa.int64() if c.startswith("tokens") else
          pa.float64() if c == "cost_usd" else pa.string()) for c in COLUMNS]))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--claude-dir", default=os.path.expanduser("~/.claude/projects"))
    ap.add_argument("--output", default="claude_events.parquet")
    args = ap.parse_args()

    table = import_events(args.claude_dir)
    pq.write_table(table, args.output)
    sessions = len(set(table["session_id"].to_pylist()))
    projects = len(set(table["project"].to_pylist()))
    calls = sum(1 for t in table["tool"].to_pylist() if t != "TextReply")
    print(f"{table.num_rows:,} events ({calls:,} tool calls) from "
          f"{sessions:,} sessions across {projects} projects -> {args.output}")


if __name__ == "__main__":
    main()
