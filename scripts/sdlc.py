#!/usr/bin/env python3
"""Deterministic controls for Cubism's issue-to-milestone SDLC.

The CLI intentionally uses only the Python standard library and the
authenticated GitHub CLI. Agents should run it instead of reimplementing
issue classification, review-receipt validation, merge gates, or HTML.
"""

from __future__ import annotations

import argparse
import datetime as dt
import html
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from typing import Any, Iterable


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / ".sdlc" / "config.json"
REVIEW_PREFIX = "<!-- cubism-sdlc-review "
REVIEW_SUFFIX = " -->"
PARENT_RE = re.compile(r"(?im)^\s*(?:parent(?:\s+epic)?|parent lifecycle epic):\s*#(\d+)\b.*$")
PART_OF_RE = re.compile(r"(?im)^\s*part of\s+#(\d+)\b.*$")
CHECKBOX_CHILD_RE = re.compile(r"(?im)^\s*-\s*\[[ xX]\]\s*#(\d+)\b")
CLOSE_RE = re.compile(r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)\b")
HEADING_RE = re.compile(r"(?m)^#{2,3}\s+(.+?)\s*$")
FAIL_CONCLUSIONS = {
    "ACTION_REQUIRED", "CANCELLED", "ERROR", "FAILURE", "STALE",
    "STARTUP_FAILURE", "TIMED_OUT",
}
PASS_CONCLUSIONS = {"SUCCESS", "NEUTRAL", "SKIPPED"}


class SdlcError(RuntimeError):
    pass


def load_config(path: Path = DEFAULT_CONFIG) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def run_json(args: list[str]) -> Any:
    command = ["rtk", *args]
    proc = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if proc.returncode != 0:
        detail = proc.stderr.strip() or proc.stdout.strip() or "command failed"
        raise SdlcError(f"{' '.join(command)}: {detail}")
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise SdlcError(f"{' '.join(command)} returned invalid JSON: {exc}") from exc


def run_text(args: list[str]) -> str:
    command = ["rtk", *args]
    proc = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if proc.returncode != 0:
        detail = proc.stderr.strip() or proc.stdout.strip() or "command failed"
        raise SdlcError(f"{' '.join(command)}: {detail}")
    return proc.stdout


def run_process(args: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    command = ["rtk", *args]
    proc = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if check and proc.returncode != 0:
        detail = proc.stderr.strip() or proc.stdout.strip() or "command failed"
        raise SdlcError(f"{' '.join(command)}: {detail}")
    return proc


def label_names(item: dict[str, Any]) -> set[str]:
    return {
        label["name"] if isinstance(label, dict) else str(label)
        for label in item.get("labels", [])
    }


def fetch_status_data(config: dict[str, Any]) -> dict[str, Any]:
    repo = config["repository"]
    issues = run_json([
        "gh", "issue", "list", "--repo", repo, "--state", "all", "--limit", "500",
        "--json", "number,title,state,body,labels,milestone,url,updatedAt,closedAt",
    ])
    pulls = run_json([
        "gh", "pr", "list", "--repo", repo, "--state", "open", "--limit", "200",
        "--json", "number,title,body,headRefName,headRefOid,isDraft,mergeStateStatus,statusCheckRollup,url,updatedAt",
    ])
    return {
        "schema": 1,
        "repository": repo,
        "generated_at": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat(),
        "issues": issues,
        "pulls": pulls,
    }


def load_status_data(input_path: Path | None, config: dict[str, Any]) -> dict[str, Any]:
    if input_path:
        with input_path.open(encoding="utf-8") as handle:
            return json.load(handle)
    return fetch_status_data(config)


def is_epic(issue: dict[str, Any], config: dict[str, Any]) -> bool:
    return (
        config["labels"]["epic"] in label_names(issue)
        or issue.get("title", "").lower().startswith("epic:")
    )


def parent_numbers(issue: dict[str, Any]) -> set[int]:
    body = issue.get("body") or ""
    return {int(value) for value in PARENT_RE.findall(body) + PART_OF_RE.findall(body)}


def checkbox_children(issue: dict[str, Any]) -> set[int]:
    return {int(value) for value in CHECKBOX_CHILD_RE.findall(issue.get("body") or "")}


def linked_issue_numbers(pr: dict[str, Any]) -> set[int]:
    return {int(value) for value in CLOSE_RE.findall(pr.get("body") or "")}


def check_state(rollup: Iterable[dict[str, Any]]) -> str:
    items = list(rollup or [])
    if not items:
        return "none"
    values = [item.get("conclusion") or item.get("state") or item.get("status") for item in items]
    if any(value in FAIL_CONCLUSIONS for value in values):
        return "failing"
    if all(value in PASS_CONCLUSIONS for value in values):
        return "passing"
    return "pending"


def build_model(data: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
    issues = {int(item["number"]): item for item in data.get("issues", [])}
    pulls = data.get("pulls", [])
    pr_by_issue: dict[int, dict[str, Any]] = {}
    for pr in pulls:
        for number in linked_issue_numbers(pr):
            pr_by_issue[number] = pr

    epics = {number: issue for number, issue in issues.items() if is_epic(issue, config)}
    children: dict[int, set[int]] = {number: checkbox_children(issue) for number, issue in epics.items()}
    parents_by_child: dict[int, set[int]] = {}
    for number, issue in issues.items():
        parents = parent_numbers(issue)
        parents_by_child[number] = parents
        for parent in parents:
            if parent in epics:
                children.setdefault(parent, set()).add(number)

    labels = config["labels"]

    def state_for(issue: dict[str, Any]) -> str:
        names = label_names(issue)
        number = int(issue["number"])
        if issue.get("state") == "CLOSED":
            return "done"
        if labels["blocked"] in names:
            return "blocked"
        if labels["changes_requested"] in names:
            return "changes-requested"
        if labels["human_review"] in names:
            return "human-review"
        if labels["approved"] in names:
            return "approved"
        if labels["needs_slicing"] in names:
            return "needs-slicing"
        if number in pr_by_issue or labels["in_review"] in names:
            return "in-review"
        if labels["in_progress"] in names:
            return "in-progress"
        if labels["ready"] in names:
            return "ready"
        return "backlog"

    epic_models = []
    mapped: set[int] = set()
    warnings: list[str] = []
    for number, epic in sorted(epics.items()):
        child_models = []
        for child_number in sorted(children.get(number, set())):
            child = issues.get(child_number)
            if not child:
                warnings.append(f"Epic #{number} references missing issue #{child_number}.")
                continue
            mapped.add(child_number)
            child_models.append({**child, "delivery_state": state_for(child), "pr": pr_by_issue.get(child_number)})
        closed = sum(child["delivery_state"] == "done" for child in child_models)
        total = len(child_models)
        epic_models.append({
            **epic,
            "delivery_state": state_for(epic),
            "children": child_models,
            "closed_children": closed,
            "total_children": total,
            "progress": round((closed / total) * 100) if total else 0,
        })
        if not total:
            warnings.append(f"Epic #{number} has no linked slices and needs decomposition.")
        if labels["epic"] not in label_names(epic):
            warnings.append(f"Epic #{number} is inferred from its title but lacks `{labels['epic']}`.")

    all_issue_models = [
        {**issue, "delivery_state": state_for(issue), "pr": pr_by_issue.get(number)}
        for number, issue in sorted(issues.items())
    ]
    active = [item for item in all_issue_models if item["delivery_state"] in {
        "in-progress", "in-review", "human-review", "changes-requested", "blocked"
    }]
    unmapped = [
        item for item in all_issue_models
        if item.get("state") == "OPEN" and int(item["number"]) not in epics and int(item["number"]) not in mapped
    ]
    recent = sorted(
        [item for item in all_issue_models if item.get("state") == "CLOSED"],
        key=lambda item: item.get("closedAt") or "",
        reverse=True,
    )[: int(config["status"]["recently_completed"])]
    return {
        "repository": data.get("repository", config["repository"]),
        "generated_at": data.get("generated_at", "unknown"),
        "issues": all_issue_models,
        "epics": epic_models,
        "active": active,
        "unmapped": unmapped,
        "recent": recent,
        "warnings": warnings,
        "counts": {
            "open": sum(item.get("state") == "OPEN" for item in all_issue_models),
            "done": sum(item.get("state") == "CLOSED" for item in all_issue_models),
            "epics": len(epic_models),
            "active": len(active),
            "unmapped": len(unmapped),
        },
    }


def esc(value: Any) -> str:
    return html.escape(str(value), quote=True)


def issue_link(issue: dict[str, Any]) -> str:
    return f'<a href="{esc(issue.get("url", "#"))}">#{int(issue["number"])}</a>'


def state_badge(state: str) -> str:
    return f'<span class="badge state-{esc(state)}">{esc(state.replace("-", " "))}</span>'


def issue_row(issue: dict[str, Any], *, compact: bool = False) -> str:
    pr = issue.get("pr")
    pr_html = ""
    if pr:
        ci = check_state(pr.get("statusCheckRollup") or [])
        pr_html = f'<div class="meta"><a href="{esc(pr.get("url", "#"))}">PR #{pr["number"]}</a> · CI {esc(ci)}</div>'
    title = esc(issue.get("title", ""))
    if compact and len(title) > 90:
        title = title[:87] + "…"
    return (
        '<li class="issue">'
        f'<div>{issue_link(issue)} {title}</div>'
        f'<div class="issue-state">{state_badge(issue["delivery_state"])}</div>'
        f'{pr_html}</li>'
    )


def render_status(model: dict[str, Any]) -> str:
    lanes = [
        ("ready", "Ready next"),
        ("in-progress", "In progress"),
        ("in-review", "Agent review"),
        ("human-review", "Human review"),
        ("changes-requested", "Human feedback"),
        ("blocked", "Blocked"),
    ]
    lane_html = []
    for key, title in lanes:
        source = model["issues"] if key == "ready" else model["active"]
        items = [item for item in source if item["delivery_state"] == key]
        body = "".join(issue_row(item, compact=True) for item in items) or '<li class="empty">Nothing here</li>'
        lane_html.append(f'<section class="lane"><h3>{esc(title)} <span>{len(items)}</span></h3><ul>{body}</ul></section>')

    epic_html = []
    for epic in model["epics"]:
        children = "".join(issue_row(item, compact=True) for item in epic["children"])
        if not children:
            children = '<li class="empty">No slices linked yet</li>'
        epic_html.append(
            '<details class="epic" open>'
            '<summary>'
            f'<div><strong>{issue_link(epic)} {esc(epic["title"])}</strong>'
            f'<div class="meta">{epic["closed_children"]}/{epic["total_children"]} slices complete</div></div>'
            f'<div class="summary-state">{state_badge(epic["delivery_state"])}</div>'
            '</summary>'
            f'<div class="progress"><span style="width:{epic["progress"]}%"></span></div>'
            f'<ul>{children}</ul></details>'
        )

    unmapped = "".join(issue_row(item, compact=True) for item in model["unmapped"]) or '<li class="empty">None</li>'
    recent = "".join(issue_row(item, compact=True) for item in model["recent"]) or '<li class="empty">None</li>'
    warnings = "".join(f"<li>{esc(item)}</li>" for item in model["warnings"]) or "<li>None</li>"
    counts = model["counts"]
    generated = esc(model["generated_at"])
    repository = esc(model["repository"])
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Cubism delivery cockpit</title>
<style>
:root{{--ink:#172026;--muted:#66737d;--paper:#f5f3ed;--card:#fff;--line:#d9d6cd;--accent:#19647e;--green:#2e7d5b;--amber:#b56b12;--red:#a23b3b;--violet:#6d4aa2}}
*{{box-sizing:border-box}} body{{margin:0;background:var(--paper);color:var(--ink);font:15px/1.45 ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}}
main{{max-width:1440px;margin:auto;padding:32px}} a{{color:var(--accent);text-decoration:none}} a:hover{{text-decoration:underline}}
.masthead{{display:flex;justify-content:space-between;gap:24px;align-items:end;margin-bottom:24px}} h1{{font-size:clamp(28px,4vw,52px);line-height:1;margin:0;letter-spacing:-.035em}} .subtitle,.meta{{color:var(--muted);font-size:13px}}
.stats{{display:grid;grid-template-columns:repeat(5,minmax(120px,1fr));gap:12px;margin:24px 0}} .stat{{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:16px}} .stat strong{{display:block;font-size:28px}} .stat span{{color:var(--muted)}}
h2{{margin:32px 0 12px;font-size:20px}} .board{{display:grid;grid-template-columns:repeat(6,minmax(210px,1fr));gap:12px;overflow-x:auto;padding-bottom:6px}} .lane{{background:#ebe8df;border-radius:14px;padding:12px;min-height:170px}} .lane h3{{display:flex;justify-content:space-between;margin:0 0 10px;font-size:14px;text-transform:uppercase;letter-spacing:.05em}} ul{{list-style:none;margin:0;padding:0}} .issue{{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:10px;margin:0 0 8px}} .issue-state{{margin-top:7px}}
.badge{{display:inline-block;border-radius:999px;background:#e3e6e8;padding:2px 8px;font-size:11px;text-transform:uppercase;letter-spacing:.04em}} .state-done,.state-approved{{background:#d9eee5;color:#175b40}} .state-in-progress,.state-ready{{background:#dcecf2;color:#14546b}} .state-in-review,.state-human-review{{background:#eadff6;color:#593782}} .state-changes-requested,.state-needs-slicing{{background:#fff0d8;color:#7b4a0d}} .state-blocked{{background:#f6dddd;color:#7f2929}}
.roadmap{{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px}} .epic{{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:14px}} .epic summary{{cursor:pointer;display:flex;justify-content:space-between;gap:16px;list-style:none}} .epic summary::-webkit-details-marker{{display:none}} .epic ul{{margin-top:12px}} .progress{{height:7px;background:#e8e7e2;border-radius:99px;margin-top:12px;overflow:hidden}} .progress span{{display:block;height:100%;background:var(--green)}}
.split{{display:grid;grid-template-columns:1fr 1fr;gap:16px}} .panel{{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:16px}} .empty{{color:var(--muted);font-style:italic;padding:8px}} footer{{margin-top:32px;color:var(--muted);font-size:12px}}
@media(max-width:1050px){{.stats{{grid-template-columns:repeat(3,1fr)}}.roadmap,.split{{grid-template-columns:1fr}}}} @media(max-width:700px){{main{{padding:20px}}.masthead{{display:block}}.stats{{grid-template-columns:repeat(2,1fr)}}.board{{grid-template-columns:repeat(6,260px)}}}}
</style></head><body><main>
<header class="masthead"><div><h1>Delivery cockpit</h1><p class="subtitle">{repository} · live issue and pull-request state</p></div><div class="meta">Generated {generated}</div></header>
<section class="stats"><div class="stat"><strong>{counts['open']}</strong><span>open issues</span></div><div class="stat"><strong>{counts['epics']}</strong><span>epics</span></div><div class="stat"><strong>{counts['active']}</strong><span>in flight / gated</span></div><div class="stat"><strong>{counts['done']}</strong><span>completed</span></div><div class="stat"><strong>{counts['unmapped']}</strong><span>unmapped work</span></div></section>
<h2>What is happening now</h2><div class="board">{''.join(lane_html)}</div>
<h2>Roadmap outcomes</h2><div class="roadmap">{''.join(epic_html)}</div>
<h2>Backlog health</h2><div class="split"><section class="panel"><h3>Open work not mapped to an epic</h3><ul>{unmapped}</ul></section><section class="panel"><h3>Planning signals</h3><ul>{warnings}</ul></section></div>
<h2>Recently completed</h2><section class="panel"><ul>{recent}</ul></section>
<footer>Generated by <code>python3 scripts/sdlc.py status</code>. GitHub is the source of truth; this file is a snapshot.</footer>
</main></body></html>"""


def section_contents(body: str) -> dict[str, str]:
    matches = list(HEADING_RE.finditer(body))
    sections: dict[str, str] = {}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(body)
        sections[match.group(1).strip()] = body[match.end():end].strip()
    return sections


def validate_slice(issue: dict[str, Any], all_issues: dict[int, dict[str, Any]], config: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    labels = config["labels"]
    names = label_names(issue)
    if issue.get("state") != "OPEN":
        errors.append("issue is not open")
    if is_epic(issue, config):
        errors.append("epics are containers, not executable slices")
    if labels["slice"] not in names and labels["feedback"] not in names:
        errors.append(f"missing `{labels['slice']}` or `{labels['feedback']}` label")
    sections = section_contents(issue.get("body") or "")
    for heading in config["required_slice_sections"]:
        if not sections.get(heading):
            errors.append(f"missing or empty `## {heading}` section")
    parents = parent_numbers(issue)
    if len(parents) != 1:
        errors.append(f"expected exactly one `Parent epic: #N`, found {len(parents)}")
    else:
        parent_number = next(iter(parents))
        parent = all_issues.get(parent_number)
        if not parent or not is_epic(parent, config):
            errors.append(f"parent #{parent_number} is missing or is not an epic")
        elif labels["feedback"] not in names:
            parent_labels = label_names(parent)
            if labels["human_review"] in parent_labels or labels["changes_requested"] in parent_labels:
                errors.append(f"parent epic #{parent_number} is paused at a human gate")
    acceptance = sections.get("Acceptance criteria", "")
    if acceptance and not re.search(r"(?m)^\s*-\s*\[[ xX]\]", acceptance):
        errors.append("acceptance criteria must contain at least one checklist item")
    return errors


def review_marker(kind: str, head_sha: str, verdict: str, reviewer: str) -> str:
    payload = json.dumps({
        "schema": 1,
        "kind": kind,
        "head_sha": head_sha,
        "verdict": verdict,
        "reviewer": reviewer,
    }, separators=(",", ":"), sort_keys=True)
    return f"{REVIEW_PREFIX}{payload}{REVIEW_SUFFIX}"


def parse_review_receipts(comments: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    receipts = []
    pattern = re.compile(re.escape(REVIEW_PREFIX) + r"(\{.*?\})" + re.escape(REVIEW_SUFFIX))
    for comment in comments:
        match = pattern.search(comment.get("body") or "")
        if not match:
            continue
        try:
            payload = json.loads(match.group(1))
        except json.JSONDecodeError:
            continue
        payload["comment_url"] = comment.get("html_url") or comment.get("url")
        payload["author"] = (comment.get("user") or {}).get("login")
        receipts.append(payload)
    return receipts


def pr_check_outcomes(pr: dict[str, Any]) -> dict[str, str]:
    outcomes: dict[str, str] = {}
    for item in pr.get("statusCheckRollup") or []:
        name = item.get("name") or item.get("context")
        if name:
            outcomes[name] = item.get("conclusion") or item.get("state") or item.get("status") or "UNKNOWN"
    return outcomes


def fetch_pr(pr_number: int, config: dict[str, Any]) -> dict[str, Any]:
    return run_json([
        "gh", "pr", "view", str(pr_number), "--repo", config["repository"],
        "--json", "number,title,state,isDraft,headRefOid,headRefName,baseRefName,body,labels,mergeable,mergeStateStatus,statusCheckRollup,url",
    ])


def fetch_issue(number: int, config: dict[str, Any]) -> dict[str, Any]:
    return run_json([
        "gh", "issue", "view", str(number), "--repo", config["repository"],
        "--json", "number,title,state,body,labels,url",
    ])


def fetch_comments(number: int, config: dict[str, Any]) -> list[dict[str, Any]]:
    return run_json(["gh", "api", "--paginate", f"repos/{config['repository']}/issues/{number}/comments"])


def evaluate_merge_gate(pr: dict[str, Any], comments: list[dict[str, Any]], issue: dict[str, Any] | None, config: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    head = pr.get("headRefOid")
    if pr.get("state") != "OPEN":
        errors.append("pull request is not open")
    if pr.get("isDraft"):
        errors.append("pull request is still a draft")
    if pr.get("baseRefName") != config["default_branch"]:
        errors.append(f"base branch is not `{config['default_branch']}`")
    if pr.get("mergeable") != "MERGEABLE":
        errors.append(f"pull request is not confirmed mergeable ({pr.get('mergeable')})")
    if pr.get("mergeStateStatus") not in {"CLEAN", "HAS_HOOKS", "UNSTABLE"}:
        errors.append(f"merge state is not ready ({pr.get('mergeStateStatus')})")
    linked = linked_issue_numbers(pr)
    if len(linked) != 1:
        errors.append(f"PR must close exactly one slice issue; found {len(linked)}")
    if issue is None:
        errors.append("linked slice issue could not be loaded")
    else:
        names = label_names(issue)
        if config["labels"]["slice"] not in names and config["labels"]["feedback"] not in names:
            errors.append("linked issue is not labeled as a slice or feedback item")
        if config["labels"]["blocked"] in names:
            errors.append("linked issue is blocked")

    outcomes = pr_check_outcomes(pr)
    for required in config["required_status_checks"]:
        if outcomes.get(required) != "SUCCESS":
            errors.append(f"required check `{required}` is {outcomes.get(required, 'missing')}")

    receipts = parse_review_receipts(comments)
    for kind in config["review"]["required"]:
        matching = [
            receipt for receipt in receipts
            if receipt.get("schema") == 1
            and receipt.get("kind") == kind
            and receipt.get("head_sha") == head
            and receipt.get("verdict") == "pass"
            and receipt.get("reviewer")
            and receipt.get("author")
        ]
        if not matching:
            errors.append(f"missing clean `{kind}` review receipt for head {head}")
    return errors


def write_atomic(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as handle:
        handle.write(content)
        temp_name = handle.name
    os.replace(temp_name, path)


def render_milestone(manifest: dict[str, Any], epic: dict[str, Any], slices: list[dict[str, Any]], generated_at: str) -> str:
    def list_items(values: Iterable[str]) -> str:
        rendered = "".join(f"<li>{esc(value)}</li>" for value in values)
        return rendered or "<li>None recorded</li>"

    slice_rows = "".join(
        f'<tr><td>{issue_link(item)}</td><td>{esc(item["title"])}</td><td>{state_badge("done" if item.get("state") == "CLOSED" else "backlog")}</td></tr>'
        for item in slices
    )
    demo = manifest.get("demo") or {}
    demo_steps = list_items(demo.get("steps") or ["No interactive demo is available for this checkpoint."])
    return f"""<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{esc(manifest['title'])}</title><style>
body{{margin:0;background:#f5f3ed;color:#172026;font:16px/1.55 system-ui,-apple-system,sans-serif}}main{{max-width:980px;margin:auto;padding:40px 24px}}header,.card{{background:#fff;border:1px solid #d9d6cd;border-radius:16px;padding:24px;margin-bottom:16px}}h1{{font-size:clamp(30px,5vw,54px);line-height:1.05;margin:.25em 0}}h2{{margin-top:0}}.eyebrow{{color:#19647e;text-transform:uppercase;letter-spacing:.08em;font-size:12px;font-weight:700}}.meta{{color:#66737d;font-size:13px}}.grid{{display:grid;grid-template-columns:1fr 1fr;gap:16px}}table{{width:100%;border-collapse:collapse}}th,td{{padding:10px;text-align:left;border-bottom:1px solid #e4e1d9}}a{{color:#19647e}}.badge{{display:inline-block;border-radius:999px;background:#e3e6e8;padding:2px 8px;font-size:11px;text-transform:uppercase}}.state-done{{background:#d9eee5;color:#175b40}}@media(max-width:720px){{.grid{{grid-template-columns:1fr}}}}
</style></head><body><main><header><div class="eyebrow">Human checkpoint · Epic #{epic['number']}</div><h1>{esc(manifest['title'])}</h1><p>{esc(manifest['summary'])}</p><div class="meta">Generated {esc(generated_at)} · <a href="{esc(epic['url'])}">review on GitHub</a></div></header>
<div class="grid"><section class="card"><h2>What was achieved</h2><ul>{list_items(manifest.get('highlights', []))}</ul></section><section class="card"><h2>Important decisions</h2><ul>{list_items(manifest.get('decisions', []))}</ul></section></div>
<section class="card"><h2>Completed slices in this checkpoint</h2><table><thead><tr><th>Issue</th><th>Outcome</th><th>State</th></tr></thead><tbody>{slice_rows}</tbody></table></section>
<div class="grid"><section class="card"><h2>Try the demo</h2><p>{esc(demo.get('summary', ''))}</p><ol>{demo_steps}</ol></section><section class="card"><h2>Known gaps / next</h2><ul>{list_items(manifest.get('known_gaps', []))}</ul></section></div>
<section class="card"><h2>Human decision</h2><p>Review the outcome and demo, then record approval or feedback on the epic issue. Feedback is converted into linked feedback slices and this report is regenerated until approved.</p></section>
</main></body></html>"""


def command_status(args: argparse.Namespace, config: dict[str, Any]) -> int:
    data = load_status_data(args.input, config)
    model = build_model(data, config)
    output = args.output or ROOT / config["status"]["output"]
    write_atomic(output, render_status(model))
    if args.snapshot:
        write_atomic(args.snapshot, json.dumps(data, indent=2, sort_keys=True) + "\n")
    print(f"WROTE {output}")
    print(json.dumps(model["counts"], sort_keys=True))
    return 0


def command_check_slice(args: argparse.Namespace, config: dict[str, Any]) -> int:
    data = load_status_data(args.input, config)
    issues = {int(item["number"]): item for item in data.get("issues", [])}
    issue = issues.get(args.issue)
    if not issue:
        print(f"BLOCKED: issue #{args.issue} was not found", file=sys.stderr)
        return 1
    errors = validate_slice(issue, issues, config)
    if errors:
        for error in errors:
            print(f"BLOCKED: {error}")
        return 1
    print(f"READY: issue #{args.issue} satisfies the executable-slice contract")
    return 0


def command_next(args: argparse.Namespace, config: dict[str, Any]) -> int:
    data = fetch_status_data(config)
    issues = {int(item["number"]): item for item in data.get("issues", [])}
    labels = config["labels"]
    candidates = []
    for number, issue in issues.items():
        names = label_names(issue)
        if labels["ready"] not in names:
            continue
        if labels["slice"] not in names and labels["feedback"] not in names:
            continue
        errors = validate_slice(issue, issues, config)
        if not errors:
            candidates.append(issue)
    if not candidates:
        print("NONE: no executable status:ready slice is available")
        return 1
    candidates.sort(key=lambda issue: (config["labels"]["feedback"] not in label_names(issue), int(issue["number"])))
    chosen = candidates[0]
    print(f"NEXT={chosen['number']}")
    print(f"TITLE={chosen['title']}")
    print(f"URL={chosen['url']}")
    return 0


def command_claim(args: argparse.Namespace, config: dict[str, Any]) -> int:
    data = fetch_status_data(config)
    issues = {int(item["number"]): item for item in data.get("issues", [])}
    issue = issues.get(args.issue)
    if not issue:
        raise SdlcError(f"issue #{args.issue} was not found")
    errors = validate_slice(issue, issues, config)
    if errors:
        for error in errors:
            print(f"BLOCKED: {error}")
        return 1
    for pr in data.get("pulls", []):
        if args.issue in linked_issue_numbers(pr):
            print(f"BLOCKED: issue #{args.issue} already has open PR #{pr['number']}")
            return 1

    branch = f"issue/{args.issue}"
    remote_ref = f"refs/heads/{branch}"
    remote = run_process(["git", "ls-remote", "--exit-code", "--heads", "origin", remote_ref], check=False)
    if remote.returncode == 0 and not args.resume:
        print(f"BLOCKED: remote branch `{branch}` already claims issue #{args.issue}; use --resume only for an abandoned session")
        return 1
    if remote.returncode not in {0, 2}:
        raise SdlcError(remote.stderr.strip() or "could not inspect remote claim branch")

    run_text(["git", "fetch", "origin", config["default_branch"]])
    if remote.returncode == 2:
        # Creating one stable remote ref per issue is the distributed claim.
        # GitHub rejects the push if another session creates it first.
        run_text(["git", "push", "origin", f"refs/remotes/origin/{config['default_branch']}:{remote_ref}"])
        run_text(["git", "fetch", "origin", branch])

    worktree = ROOT / ".worktrees" / f"issue-{args.issue}"
    if worktree.exists():
        print(f"WORKTREE={worktree}")
    else:
        local = run_process(["git", "show-ref", "--verify", "--quiet", f"refs/heads/{branch}"], check=False)
        if local.returncode == 0:
            run_text(["git", "worktree", "add", str(worktree), branch])
        elif local.returncode == 1:
            run_text(["git", "worktree", "add", "--track", "-b", branch, str(worktree), f"origin/{branch}"])
        else:
            raise SdlcError("could not inspect local claim branch")

    run_text([
        "gh", "issue", "edit", str(args.issue), "--repo", config["repository"],
        "--add-label", config["labels"]["in_progress"],
        "--remove-label", config["labels"]["ready"],
    ])
    print(f"CLAIMED: issue #{args.issue} on `{branch}`")
    print(f"WORKTREE={worktree}")
    return 0


def command_cleanup(args: argparse.Namespace, config: dict[str, Any]) -> int:
    issue = fetch_issue(args.issue, config)
    if issue.get("state") != "CLOSED":
        print(f"BLOCKED: issue #{args.issue} is not closed")
        return 1
    branch = f"issue/{args.issue}"
    worktree = ROOT / ".worktrees" / f"issue-{args.issue}"
    if worktree.exists():
        run_text(["git", "worktree", "remove", str(worktree)])
    local = run_process(["git", "show-ref", "--verify", "--quiet", f"refs/heads/{branch}"], check=False)
    if local.returncode == 0:
        run_text(["git", "branch", "-d", branch])
    run_text(["git", "fetch", "origin", "--prune"])
    print(f"CLEANED: issue #{args.issue}")
    return 0


def command_review_receipt(args: argparse.Namespace, config: dict[str, Any]) -> int:
    pr = fetch_pr(args.pr, config)
    report = args.body_file.read_text(encoding="utf-8")
    marker = review_marker(args.kind, pr["headRefOid"], args.verdict, args.reviewer)
    body = f"{marker}\n\n## {args.kind.title()} review\n\n{report.strip()}\n"
    if args.dry_run:
        print(body)
        return 0
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False) as handle:
        handle.write(body)
        temp_name = handle.name
    try:
        run_text(["gh", "pr", "comment", str(args.pr), "--repo", config["repository"], "--body-file", temp_name])
    finally:
        Path(temp_name).unlink(missing_ok=True)
    print(f"RECORDED: {args.kind}={args.verdict} for {pr['headRefOid']}")
    return 0


def load_gate_inputs(pr_number: int, config: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]], dict[str, Any] | None]:
    pr = fetch_pr(pr_number, config)
    comments = fetch_comments(pr_number, config)
    linked = linked_issue_numbers(pr)
    issue = fetch_issue(next(iter(linked)), config) if len(linked) == 1 else None
    return pr, comments, issue


def command_merge_gate(args: argparse.Namespace, config: dict[str, Any]) -> int:
    pr, comments, issue = load_gate_inputs(args.pr, config)
    errors = evaluate_merge_gate(pr, comments, issue, config)
    if errors:
        for error in errors:
            print(f"BLOCKED: {error}")
        return 1
    print(f"ELIGIBLE: PR #{args.pr} may be merged at {pr['headRefOid']}")
    return 0


def command_merge(args: argparse.Namespace, config: dict[str, Any]) -> int:
    gate_args = argparse.Namespace(pr=args.pr)
    if command_merge_gate(gate_args, config) != 0:
        return 1
    if not args.apply:
        print("DRY RUN: pass --apply to merge")
        return 0
    method = config["merge"]["method"]
    command = ["gh", "pr", "merge", str(args.pr), "--repo", config["repository"], f"--{method}"]
    if config["merge"].get("delete_branch"):
        command.append("--delete-branch")
    if add or remove:
        run_text(command)
    print(f"MERGED: PR #{args.pr}")
    return 0


def command_milestone_report(args: argparse.Namespace, config: dict[str, Any]) -> int:
    with args.manifest.open(encoding="utf-8") as handle:
        manifest = json.load(handle)
    required = {"schema", "epic", "title", "summary", "included_slices", "highlights", "decisions", "known_gaps", "demo"}
    missing = sorted(required - set(manifest))
    if missing:
        raise SdlcError(f"manifest missing keys: {', '.join(missing)}")
    if manifest["schema"] != 1:
        raise SdlcError("unsupported milestone manifest schema")
    epic = fetch_issue(int(manifest["epic"]), config)
    if not is_epic(epic, config):
        raise SdlcError(f"issue #{manifest['epic']} is not an epic")
    slices = [fetch_issue(int(number), config) for number in manifest["included_slices"]]
    invalid = [f"#{item['number']}" for item in slices if item.get("state") != "CLOSED" or int(manifest["epic"]) not in parent_numbers(item)]
    if invalid:
        raise SdlcError(f"included slices must be closed and linked to the epic: {', '.join(invalid)}")
    output = args.output or args.manifest.with_suffix(".html")
    generated = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()
    write_atomic(output, render_milestone(manifest, epic, slices, generated))
    print(f"WROTE {output}")
    return 0


def command_set_gate(args: argparse.Namespace, config: dict[str, Any]) -> int:
    epic = fetch_issue(args.epic, config)
    if not is_epic(epic, config):
        raise SdlcError(f"issue #{args.epic} is not an epic")
    note = args.note_file.read_text(encoding="utf-8").strip()
    if not note:
        raise SdlcError("gate note must not be empty")
    labels = config["labels"]
    transitions = {
        "review": ([labels["human_review"]], [labels["changes_requested"], labels["approved"]]),
        "changes-requested": ([labels["human_review"], labels["changes_requested"]], [labels["approved"]]),
        "approved": ([labels["approved"]], [labels["human_review"], labels["changes_requested"]]),
    }
    add, remove = transitions[args.state]
    current = label_names(epic)
    add = [name for name in add if name not in current]
    remove = [name for name in remove if name in current]
    command = ["gh", "issue", "edit", str(args.epic), "--repo", config["repository"]]
    for name in add:
        command.extend(["--add-label", name])
    for name in remove:
        command.extend(["--remove-label", name])
    if not args.apply:
        print("DRY RUN:", " ".join(command))
        print(note)
        return 0
    run_text(command)
    marker = f"<!-- cubism-sdlc-gate state={args.state} -->"
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False) as handle:
        handle.write(f"{marker}\n\n{note}\n")
        temp_name = handle.name
    try:
        run_text(["gh", "issue", "comment", str(args.epic), "--repo", config["repository"], "--body-file", temp_name])
    finally:
        Path(temp_name).unlink(missing_ok=True)
    print(f"GATE={args.state} epic=#{args.epic}")
    return 0


def command_bootstrap_labels(args: argparse.Namespace, config: dict[str, Any]) -> int:
    colors = {
        "epic": ("5319E7", "Long-term outcome container"),
        "slice": ("1D76DB", "Bounded unit of work for one agent session"),
        "feedback": ("D4C5F9", "Human feedback tracked as an executable slice"),
        "ready": ("0E8A16", "Ready to be claimed"),
        "in_progress": ("FBCA04", "Actively being worked on"),
        "in_review": ("006B75", "Implementation complete; agent review or CI in progress"),
        "blocked": ("B60205", "Cannot proceed until a named dependency changes"),
        "needs_slicing": ("F9D0C4", "Too broad or underspecified for one agent session"),
        "human_review": ("6F42C1", "Development paused for milestone-level human review"),
        "changes_requested": ("D93F0B", "Human feedback must be resolved before approval"),
        "approved": ("2E7D32", "Human approved the milestone checkpoint")
    }
    for key, (color, description) in colors.items():
        name = config["labels"][key]
        command = ["gh", "label", "create", name, "--repo", config["repository"], "--color", color, "--description", description, "--force"]
        if args.apply:
            run_text(command)
            print(f"APPLIED: {name}")
        else:
            print("DRY RUN:", " ".join(command))
    if args.classify_epics:
        data = fetch_status_data(config)
        issues = {int(issue["number"]): issue for issue in data.get("issues", [])}
        children_by_epic: dict[int, set[int]] = {}
        for issue in issues.values():
            for parent in parent_numbers(issue):
                children_by_epic.setdefault(parent, set()).add(int(issue["number"]))
        for issue in data.get("issues", []):
            if issue.get("state") == "OPEN" and issue.get("title", "").lower().startswith("epic:"):
                command = [
                    "gh", "issue", "edit", str(issue["number"]), "--repo", config["repository"],
                    "--add-label", config["labels"]["epic"],
                ]
                if args.apply:
                    run_text(command)
                    print(f"CLASSIFIED: epic #{issue['number']}")
                else:
                    print("DRY RUN:", " ".join(command))
                child_numbers = checkbox_children(issue) | children_by_epic.get(int(issue["number"]), set())
                has_executable_child = any(
                    child_number in issues
                    and (
                        config["labels"]["slice"] in label_names(issues[child_number])
                        or config["labels"]["feedback"] in label_names(issues[child_number])
                    )
                    for child_number in child_numbers
                )
                if not has_executable_child:
                    slicing_command = [
                        "gh", "issue", "edit", str(issue["number"]), "--repo", config["repository"],
                        "--add-label", config["labels"]["needs_slicing"],
                    ]
                    if args.apply:
                        run_text(slicing_command)
                        print(f"CLASSIFIED: epic #{issue['number']} needs slicing")
                    else:
                        print("DRY RUN:", " ".join(slicing_command))
                elif config["labels"]["needs_slicing"] in label_names(issue):
                    ready_command = [
                        "gh", "issue", "edit", str(issue["number"]), "--repo", config["repository"],
                        "--remove-label", config["labels"]["needs_slicing"],
                    ]
                    if args.apply:
                        run_text(ready_command)
                        print(f"CLASSIFIED: epic #{issue['number']} has linked work")
                    else:
                        print("DRY RUN:", " ".join(ready_command))
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    sub = parser.add_subparsers(dest="command", required=True)

    status = sub.add_parser("status", help="generate the HTML delivery cockpit")
    status.add_argument("--input", type=Path, help="offline JSON bundle instead of live GitHub")
    status.add_argument("--output", type=Path)
    status.add_argument("--snapshot", type=Path)
    status.set_defaults(func=command_status)

    check_slice = sub.add_parser("check-slice", help="validate an issue as one executable slice")
    check_slice.add_argument("issue", type=int)
    check_slice.add_argument("--input", type=Path, help="offline JSON bundle instead of live GitHub")
    check_slice.set_defaults(func=command_check_slice)

    next_slice = sub.add_parser("next", help="select the next valid ready slice")
    next_slice.set_defaults(func=command_next)

    claim = sub.add_parser("claim", help="atomically claim a slice with a stable remote branch")
    claim.add_argument("issue", type=int)
    claim.add_argument("--resume", action="store_true", help="reuse an existing remote claim branch")
    claim.set_defaults(func=command_claim)

    cleanup = sub.add_parser("cleanup", help="remove a closed slice's worktree and local branch")
    cleanup.add_argument("issue", type=int)
    cleanup.set_defaults(func=command_cleanup)

    receipt = sub.add_parser("review-receipt", help="post a SHA-bound structured review result")
    receipt.add_argument("--pr", type=int, required=True)
    receipt.add_argument("--kind", choices=["advisor", "codex"], required=True)
    receipt.add_argument("--verdict", choices=["pass", "fail"], required=True)
    receipt.add_argument("--reviewer", required=True, help="agent/session identity shown in the receipt")
    receipt.add_argument("--body-file", type=Path, required=True)
    receipt.add_argument("--dry-run", action="store_true")
    receipt.set_defaults(func=command_review_receipt)

    gate = sub.add_parser("merge-gate", help="check deterministic auto-merge eligibility")
    gate.add_argument("--pr", type=int, required=True)
    gate.set_defaults(func=command_merge_gate)

    merge = sub.add_parser("merge", help="gate and optionally merge an eligible PR")
    merge.add_argument("--pr", type=int, required=True)
    merge.add_argument("--apply", action="store_true")
    merge.set_defaults(func=command_merge)

    report = sub.add_parser("milestone-report", help="render and validate a human checkpoint report")
    report.add_argument("--manifest", type=Path, required=True)
    report.add_argument("--output", type=Path)
    report.set_defaults(func=command_milestone_report)

    gate_state = sub.add_parser("set-gate", help="record a human milestone gate transition")
    gate_state.add_argument("--epic", type=int, required=True)
    gate_state.add_argument("--state", choices=["review", "changes-requested", "approved"], required=True)
    gate_state.add_argument("--note-file", type=Path, required=True)
    gate_state.add_argument("--apply", action="store_true")
    gate_state.set_defaults(func=command_set_gate)

    labels = sub.add_parser("bootstrap-labels", help="create or update the SDLC label vocabulary")
    labels.add_argument("--apply", action="store_true")
    labels.add_argument("--classify-epics", action="store_true")
    labels.set_defaults(func=command_bootstrap_labels)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return int(args.func(args, load_config(args.config)))
    except (OSError, SdlcError, KeyError, ValueError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
