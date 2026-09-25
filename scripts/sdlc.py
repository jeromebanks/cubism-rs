#!/usr/bin/env python3
"""Deterministic controls for an issue-to-milestone delivery lifecycle.

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
from typing import Any, Callable, Iterable, NamedTuple, Optional, Sequence


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / ".sdlc" / "config.json"
# On-the-wire protocol identifiers. These are stamped into GitHub comment
# bodies and parsed back out, so they are format, not branding: renaming one
# orphans every receipt or gate record already posted. Project-neutral because
# one engine is intended to serve every project it manages, not just this one.
REVIEW_PREFIX = "<!-- nightshift-review "
REVIEW_SUFFIX = " -->"
GATE_PREFIX = "<!-- nightshift-gate "
GATE_SUFFIX = " -->"
PARENT_RE = re.compile(r"(?im)^\s*(?:parent(?:\s+epic)?|parent lifecycle epic):\s*#(\d+)\b.*$")
PART_OF_RE = re.compile(r"(?im)^\s*part of\s+#(\d+)\b.*$")
CHECKBOX_CHILD_RE = re.compile(r"(?im)^\s*-\s*\[[ xX]\]\s*#(\d+)\b")
CLOSE_RE = re.compile(r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)\b")
HEADING_RE = re.compile(r"(?m)^#{2,3}\s+(.+?)\s*$")
# A gate record is a comment whose body begins, at its first byte, with the
# marker — exactly as `set-gate` writes it. No leading whitespace is allowed:
# four spaces make an indented Markdown code block, and a comment that merely
# quotes a marker (fenced, indented or `>`) must not become the newest decision.
GATE_RECORD_RE = re.compile(r"\A" + re.escape(GATE_PREFIX) + r"(.*?)" + re.escape(GATE_SUFFIX))
LEGACY_GATE_RE = re.compile(r"\Astate=([a-z-]+)\Z")
FEEDBACK_DECISION_RE = re.compile(r"(?im)^\s*feedback decision:\s*(\S+)\s*$")
GATE_RECORD_URL_RE = re.compile(
    r"\Ahttps://github\.com/([^/\s]+/[^/\s]+)/issues/(\d+)#issuecomment-(\d+)\Z"
)
GATE_STATES = ("review", "changes-requested", "approved")
# Which agent session produced a commit. `Agent-Session` is the harness-neutral
# spelling every harness should write; `Claude-Session` is the convention
# already present in this repository's history and stays valid so that existing
# commits are not retroactively unmergeable. Harness names already appear in
# this file (`advisor`, `codex`), so naming them here introduces no new coupling.
AGENT_SESSION_TRAILERS = ("Agent-Session", "Claude-Session")
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


def create_remote_ref(ref: str, sha: str, config: dict[str, Any]) -> bool:
    """Create one remote ref. Return False if another writer created it first.

    `git push` cannot express this. Pushing a SHA to a ref that already holds
    that SHA succeeds as an up-to-date no-op, so two sessions that both observe
    the claim branch absent, then both push the same default-branch SHA, both
    see success and both report ownership. GitHub's ref-creation endpoint is
    create-only and answers 422 when the ref exists, so the loser of a race
    fails closed instead of claiming work it does not own.
    """
    proc = run_process([
        "gh", "api", "--method", "POST",
        f"repos/{config['repository']}/git/refs",
        "-f", f"ref={ref}",
        "-f", f"sha={sha}",
    ], check=False)
    if proc.returncode == 0:
        return True
    detail = f"{proc.stderr or ''}\n{proc.stdout or ''}".strip()
    if "already exists" in detail.lower():
        return False
    raise SdlcError(detail or f"could not create remote ref {ref}")


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
<title>{repository} delivery cockpit</title>
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


GateRecordLoader = Callable[[int], Optional[list[dict[str, Any]]]]


def gate_labels(config: dict[str, Any]) -> dict[str, str]:
    labels = config["labels"]
    return {
        "review": labels["human_review"],
        "changes-requested": labels["changes_requested"],
        "approved": labels["approved"],
    }


def gate_state(epic: dict[str, Any], config: dict[str, Any]) -> tuple[str | None, str | None]:
    """Return `(state, error)` for an epic's gate labels.

    Gate labels are mutually exclusive. More than one is not a state any
    command produces now, so it is refused rather than resolved by precedence:
    guessing which label is stale would be guessing the human's decision.
    """
    names = label_names(epic)
    present = [state for state, label in gate_labels(config).items() if label in names]
    if len(present) > 1:
        shown = ", ".join(f"`{gate_labels(config)[state]}`" for state in present)
        return None, (
            f"parent epic #{epic.get('number')} has conflicting gate labels ({shown}); "
            f"reconcile by recording the actual human decision with `scripts/sdlc.py "
            f"set-gate --epic {epic.get('number')} --state <state> ... --apply`"
        )
    return (present[0] if present else "none"), None


def gate_marker(state: str, checkpoint: str, decided_by: str) -> str:
    payload = json.dumps(
        {"schema": 1, "state": state, "checkpoint": checkpoint, "decided_by": decided_by},
        sort_keys=True, separators=(",", ":"),
    )
    return f"{GATE_PREFIX}{payload}{GATE_SUFFIX}"


def parse_gate_records(comments: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    records = []
    for sequence, comment in enumerate(comments):
        match = GATE_RECORD_RE.match(comment.get("body") or "")
        if not match:
            continue
        raw = match.group(1).strip()
        legacy = LEGACY_GATE_RE.match(raw)
        if legacy:
            # Pre-R2 records carry only a state: enough to show history, never
            # enough to authorize feedback (no checkpoint, no decision owner).
            payload: dict[str, Any] = {"schema": 0, "state": legacy.group(1)}
        else:
            try:
                payload = json.loads(raw)
            except json.JSONDecodeError:
                continue
            if not isinstance(payload, dict):
                continue
        payload["id"] = comment.get("id")
        payload["comment_url"] = comment.get("html_url") or comment.get("url")
        payload["created_at"] = comment.get("created_at")
        payload["sequence"] = sequence
        records.append(payload)
    return records


def complete_gate_record(record: dict[str, Any] | None, state: str) -> bool:
    """True for a schema-1 `state` record carrying what `set-gate` writes.

    Incomplete or legacy records still count as the newest record — they
    are never skipped over — but they authorize nothing.
    """
    def present(key: str) -> bool:
        value = (record or {}).get(key)
        return isinstance(value, str) and bool(value.strip())

    return (
        bool(record)
        and record.get("schema") == 1
        and record.get("state") == state
        and present("checkpoint")
        and (state == "review" or present("decided_by"))
    )


def latest_gate_record(records: Iterable[dict[str, Any]]) -> dict[str, Any] | None:
    ordered = sorted(records, key=lambda record: (record.get("created_at") or "", record.get("sequence") or 0))
    return ordered[-1] if ordered else None


def gate_admission_errors(
    issue: dict[str, Any],
    parent_number: int,
    parent: dict[str, Any] | None,
    load_gate_records: GateRecordLoader,
    config: dict[str, Any],
) -> list[str]:
    """Decide whether the parent's human gate admits this issue.

    One predicate for readiness, claim, and merge. The transition table:

    - no gate label, or `approved`: ordinary and feedback work proceed;
    - `review`: nothing proceeds, feedback included;
    - `changes-requested`: only a `type:feedback` issue citing the parent's
      current changes-requested gate record proceeds. The label alone is not
      authority; the cited human decision is;
    - conflicting labels, or an unreadable parent or record: blocked.

    Records are required even where labels alone decide: the gate is the
    labels *and* their record, and an unreadable record means the gate is
    not fully known.
    """
    labels = config["labels"]
    if parent is None or not is_epic(parent, config):
        return [f"parent epic #{parent_number} could not be loaded as an epic; its human gate is unknown"]
    records = load_gate_records(parent_number)
    if records is None:
        return [f"gate records on parent epic #{parent_number} could not be read; its human gate is unknown"]
    state, conflict = gate_state(parent, config)
    if conflict:
        return [conflict]
    if state == "review":
        return [
            f"parent epic #{parent_number} is paused for human review; "
            "no slice, including feedback, may proceed"
        ]
    if state != "changes-requested":
        return []
    if labels["feedback"] not in label_names(issue):
        return [
            f"parent epic #{parent_number} has changes requested; only "
            f"`{labels['feedback']}` slices may proceed"
        ]
    cited = FEEDBACK_DECISION_RE.findall(issue.get("body") or "")
    if len(cited) != 1:
        return [
            f"feedback during changes-requested must cite exactly one "
            f"`Feedback decision: <gate record URL>` from parent epic #{parent_number}; "
            f"found {len(cited)}"
        ]
    link = GATE_RECORD_URL_RE.match(cited[0])
    if (
        not link
        or link.group(1).lower() != config["repository"].lower()
        or int(link.group(2)) != parent_number
    ):
        return [
            f"`Feedback decision: {cited[0]}` is not a gate record comment on "
            f"{config['repository']} epic #{parent_number}"
        ]
    latest = latest_gate_record(records)
    if not complete_gate_record(latest, "changes-requested"):
        return [
            f"parent epic #{parent_number} is labeled changes-requested but its newest gate "
            "record is not a complete schema-1 changes-requested decision (checkpoint and "
            "decided_by); reconcile with "
            "`scripts/sdlc.py set-gate`"
        ]
    if str(latest.get("id")) != link.group(3):
        return [
            f"feedback cites decision {cited[0]}, but the current changes-requested "
            f"decision on epic #{parent_number} is {latest.get('comment_url')}"
        ]
    return []


def no_gate_records(_parent_number: int) -> None:
    return None


def bundle_gate_records(data: dict[str, Any]) -> GateRecordLoader:
    """Gate records from an offline bundle's `gate_comments` map.

    The map is `{"<epic number>": [issue comment, ...]}` as the comments API
    returns them. An epic absent from it is unknown, and so blocks.
    """
    stored = data.get("gate_comments")

    def load(parent_number: int) -> list[dict[str, Any]] | None:
        comments = stored.get(str(parent_number)) if isinstance(stored, dict) else None
        return parse_gate_records(comments) if isinstance(comments, list) else None

    return load


class ParentGate(NamedTuple):
    """An issue's parent epic and its gate records, as admission sees them.

    Merge reloads it live (`load_parent_gate`); the other commands take it from
    an already-fetched issue map (`load_parent_from`). `number` is None when
    the issue does not name exactly one parent; `epic` and `records` are None
    when GitHub could not supply them. Each unknown blocks: the pause must be
    observable at merge, not only at claim.
    """
    number: int | None
    epic: dict[str, Any] | None
    records: list[dict[str, Any]] | None


ADMISSION_MODES = ("check", "start", "continue")


class DependencySource(NamedTuple):
    """How admission reads prerequisites. Each reader raises when GitHub cannot
    answer completely; the caller treats that as unknown, and unknown blocks.

    - `blocked_by(n)`: every native `blocked_by` issue of #n, all pages;
    - `issue(n)`: a prerequisite with `state`, `stateReason` and
      `closedByPullRequestsReferences`;
    - `pull(n)`: a pull request with `state`, `baseRefName` and `mergeCommit`.
    """
    blocked_by: Callable[[int], list[dict[str, Any]]]
    issue: Callable[[int], dict[str, Any]]
    pull: Callable[[int], dict[str, Any]]


def _unread(what: str) -> Callable[[int], Any]:
    def read(number: int) -> Any:
        raise SdlcError(f"no {what} source was supplied for #{number}")
    return read


# The default for callers that supply nothing: every prerequisite is unknown.
NO_DEPENDENCY_DATA = DependencySource(
    _unread("blocked_by"), _unread("prerequisite"), _unread("pull request"),
)


def same_repository(repository: str, config: dict[str, Any]) -> bool:
    return repository.lower() == config["repository"].lower()


def _blocker_repository(blocker: dict[str, Any]) -> str:
    # REST: exactly https://api.github.com/repos/{owner}/{repo}; any other
    # host or shape names no repository, so the blocker counts as foreign.
    url = blocker.get("repository_url")
    match = re.fullmatch(r"https://api\.github\.com/repos/([^/]+/[^/]+)", url) if isinstance(url, str) else None
    return match.group(1) if match else ""


def issue_number(value: Any) -> int | None:
    """A GitHub issue or PR number: a positive int, never a bool."""
    if isinstance(value, int) and not isinstance(value, bool) and value > 0:
        return value
    return None


def _closing_reference(ref: Any) -> tuple[str, int] | None:
    """A closing reference's `owner/name` and PR number, or None if malformed."""
    if not isinstance(ref, dict):
        return None
    repo = ref.get("repository")
    owner = repo.get("owner") if isinstance(repo, dict) else None
    login = owner.get("login") if isinstance(owner, dict) else None
    name = repo.get("name") if isinstance(repo, dict) else None
    number = issue_number(ref.get("number"))
    if not (isinstance(login, str) and login and isinstance(name, str) and name) or number is None:
        return None
    return f"{login}/{name}", number


PULL_STATES = ("OPEN", "CLOSED", "MERGED")


def _pull_record_complete(pull: Any, number: int) -> bool:
    """Whether a PR record is exactly consistent with its state, as GitHub
    reports it: a MERGED pull request has a `mergeCommit` with a full SHA, and
    any other has a null `mergeCommit`. Anything else is not evidence of either
    outcome, so it is unknown rather than read as "unmerged"."""
    if not isinstance(pull, dict) or issue_number(pull.get("number")) != number:
        return False
    state, base = pull.get("state"), pull.get("baseRefName")
    if state not in PULL_STATES or not isinstance(base, str) or not base or "mergeCommit" not in pull:
        return False
    commit = pull["mergeCommit"]
    if state != "MERGED":
        return commit is None
    oid = commit.get("oid") if isinstance(commit, dict) else None
    return isinstance(oid, str) and re.fullmatch(r"[0-9a-f]{40}", oid) is not None


def _verified_merges(issue: dict[str, Any], source: DependencySource, config: dict[str, Any]) -> tuple[bool, list[str], list[str]]:
    """Whether a closing PR of `issue` is verified merged into the default
    branch, what was checked when none is, and which references could not be
    read. Every reference is read: one that is malformed, unreadable or
    incomplete is unknown, and unknown blocks even beside a merged one."""
    merged = False
    checked: list[str] = []
    unreadable: list[str] = []
    for ref in issue["closedByPullRequestsReferences"]:
        parsed = _closing_reference(ref)
        if parsed is None:
            unreadable.append(f"a closing reference is malformed ({json.dumps(ref, sort_keys=True)[:120]})")
            continue
        name, number = parsed
        if not same_repository(name, config):
            checked.append(f"{name}#{number} is outside {config['repository']}")
            continue
        try:
            pull = source.pull(number)
        except (SdlcError, OSError) as exc:
            unreadable.append(f"PR #{number} could not be read ({exc})")
            continue
        if not _pull_record_complete(pull, number):
            unreadable.append(
                f"PR #{number} is not a complete record: it needs its own number, a known state, a base branch, "
                "and a mergeCommit that is null unless MERGED and has a full SHA when MERGED"
            )
            continue
        if pull["state"] == "MERGED" and pull["baseRefName"] == config["default_branch"]:
            merged = True
            continue
        checked.append(f"PR #{number} is {pull['state']} into `{pull['baseRefName']}`")
    return merged, checked, unreadable


def completion_error(number: int, source: DependencySource, config: dict[str, Any]) -> str | None:
    """None when prerequisite #number is successfully completed.

    An implementation prerequisite is complete when it is closed COMPLETED
    *and* a pull request that closed it is verified merged into the default
    branch. Closure alone — cancelled, duplicate, or hand-closed — is not.
    """
    try:
        issue = source.issue(number)
    except (SdlcError, OSError) as exc:
        return f"blocked by #{number}, which could not be read ({exc}); its completion is unknown"
    if (
        not isinstance(issue, dict)
        or issue_number(issue.get("number")) != number
        or issue.get("state") not in {"OPEN", "CLOSED"}
    ):
        return f"blocked by #{number}, whose record is malformed; its completion is unknown"
    if issue.get("state") != "CLOSED":
        return f"blocked by #{number}, which is still open"
    reason = issue.get("stateReason")
    if reason != "COMPLETED":
        return (
            f"blocked by #{number}, closed as {reason or 'an unknown reason'} rather than "
            "COMPLETED; a cancelled or duplicate prerequisite does not satisfy a dependency"
        )
    if not isinstance(issue.get("closedByPullRequestsReferences"), list):
        return f"blocked by #{number}: its closing pull requests could not be read; its completion is unknown"
    merged, checked, unreadable = _verified_merges(issue, source, config)
    if unreadable:
        return (
            f"blocked by #{number}: its closing evidence could not be read "
            f"({'; '.join(unreadable)}); its completion is unknown"
        )
    if merged:
        return None
    detail = f" (checked: {'; '.join(checked)})" if checked else ""
    return (
        f"blocked by #{number}: closed COMPLETED, but no closing pull request is verified "
        f"merged into `{config['default_branch']}`{detail}. A verification-only prerequisite "
        "needs explicit completion evidence, which admission cannot read yet"
    )


def prerequisite_errors(number: int, source: DependencySource, config: dict[str, Any]) -> list[str]:
    """Why #number's native prerequisites do not admit it; empty when they do.

    Reads `blocked_by` transitively so a cycle anywhere upstream is found and
    reported with its path. Only direct prerequisites must be complete, but
    every read must succeed: an unreadable or foreign link leaves the graph
    unknown, and unknown blocks.
    """
    errors: list[str] = []
    edges: dict[int, list[int]] = {}
    cycles: set[tuple[int, ...]] = set()

    def read(node: int) -> list[int]:
        if node in edges:
            return edges[node]
        edges[node] = []
        try:
            blockers = source.blocked_by(node)
        except (SdlcError, OSError) as exc:
            errors.append(f"prerequisites of #{node} could not be read completely ({exc}); dependencies are unknown")
            return []
        if not isinstance(blockers, list):
            errors.append(f"prerequisites of #{node} are not a list; dependencies are unknown")
            return []
        found = []
        for blocker in blockers:
            blocker_number = issue_number(blocker.get("number")) if isinstance(blocker, dict) else None
            if blocker_number is None:
                errors.append(f"a prerequisite entry of #{node} is malformed; dependencies are unknown")
                continue
            if not same_repository(_blocker_repository(blocker), config):
                errors.append(
                    f"#{node} is blocked by {blocker.get('html_url') or blocker_number}, outside "
                    f"{config['repository']}; its completion cannot be verified"
                )
                continue
            found.append(blocker_number)
        edges[node] = found
        return found

    path: list[int] = []
    finished: set[int] = set()

    def walk(node: int) -> None:
        path.append(node)
        for blocker in read(node):
            if blocker in path:
                cycle = path[path.index(blocker):]
                # One report per cycle, whichever node it was entered from.
                start = cycle.index(min(cycle))
                cycles.add(tuple(cycle[start:] + cycle[:start]))
            elif blocker not in finished:
                walk(blocker)
        path.pop()
        finished.add(node)

    walk(number)
    for cycle in sorted(cycles):
        route = " → ".join(f"#{node}" for node in (*cycle, cycle[0]))
        errors.append(f"dependency cycle: {route} (each is blocked by the next); remove one link")
    for blocker in edges.get(number, []):
        error = completion_error(blocker, source, config)
        if error:
            errors.append(error)
    return errors


def admission_errors(
    issue: dict[str, Any],
    parent: ParentGate,
    config: dict[str, Any],
    *,
    mode: str,
    prerequisites: Sequence[str],
) -> list[str]:
    """Decide whether an issue is executable work, for every command.

    One predicate, three modes:

    - `check` (check-slice): the executable-slice contract. It does not
      require `status:ready`, because a planner runs it to decide whether an
      issue may be labeled ready;
    - `start` (next, claim): `check`, and the issue is labeled ready;
    - `continue` (claim --resume, merge): an already-owned attempt. It need
      not regain `status:ready` and is not re-judged on its contract
      sections, but it must still be open, unblocked, sliced and admitted by
      its parent's gate.

    Every mode also requires each native `blocked_by` prerequisite to be
    successfully completed. The caller evaluates them with
    `prerequisite_errors` and passes the result; there is no default, so no
    command can omit them.
    """
    if mode not in ADMISSION_MODES:
        raise ValueError(f"unknown admission mode {mode!r}")
    errors: list[str] = []
    labels = config["labels"]
    names = label_names(issue)
    if issue.get("state") != "OPEN":
        errors.append("issue is not open")
    if is_epic(issue, config):
        errors.append("epics are containers, not executable slices")
    if labels["slice"] not in names and labels["feedback"] not in names:
        errors.append(f"missing `{labels['slice']}` or `{labels['feedback']}` label")
    if labels["blocked"] in names:
        errors.append(f"issue is labeled `{labels['blocked']}`; resolve the blocker and remove the label first")
    if labels["needs_slicing"] in names:
        errors.append(f"issue is labeled `{labels['needs_slicing']}`; split it into executable slices first")
    if mode == "start" and labels["ready"] not in names:
        errors.append(f"issue is not labeled `{labels['ready']}`")
    if mode != "continue":
        sections = section_contents(issue.get("body") or "")
        for heading in config["required_slice_sections"]:
            if not sections.get(heading):
                errors.append(f"missing or empty `## {heading}` section")
        acceptance = sections.get("Acceptance criteria", "")
        if acceptance and not re.search(r"(?m)^\s*-\s*\[[ xX]\]", acceptance):
            errors.append("acceptance criteria must contain at least one checklist item")
    parents = parent_numbers(issue)
    if len(parents) != 1:
        errors.append(
            f"expected exactly one parent epic (`Parent epic: #N`), found {len(parents)}; "
            "its human gate is unknown"
        )
    elif parent.number != next(iter(parents)):
        errors.append(f"parent epic #{next(iter(parents))} was not loaded; its human gate is unknown")
    else:
        records = parent.records
        errors.extend(gate_admission_errors(
            issue, parent.number, parent.epic, lambda _number: records, config,
        ))
    errors.extend(prerequisites)
    return errors


def load_parent_from(
    issue: dict[str, Any],
    all_issues: dict[int, dict[str, Any]],
    config: dict[str, Any],
    load_gate_records: GateRecordLoader,
) -> ParentGate:
    """The issue's parent from an already-fetched issue map."""
    parents = parent_numbers(issue)
    if len(parents) != 1:
        return ParentGate(None, None, None)
    number = next(iter(parents))
    epic = all_issues.get(number)
    # Records are read only for a real epic; an absent parent blocks anyway.
    records = load_gate_records(number) if epic and is_epic(epic, config) else None
    return ParentGate(number, epic, records)


def validate_slice(
    issue: dict[str, Any],
    all_issues: dict[int, dict[str, Any]],
    config: dict[str, Any],
    load_gate_records: GateRecordLoader = no_gate_records,
    dependencies: DependencySource = NO_DEPENDENCY_DATA,
    *,
    mode: str = "check",
) -> list[str]:
    parent = load_parent_from(issue, all_issues, config, load_gate_records)
    prerequisites = prerequisite_errors(int(issue["number"]), dependencies, config)
    return admission_errors(issue, parent, config, mode=mode, prerequisites=prerequisites)


def review_marker(kind: str, head_sha: str, verdict: str, reviewer: str) -> str:
    payload = json.dumps({
        "schema": 1,
        "kind": kind,
        "head_sha": head_sha,
        "verdict": verdict,
        "reviewer": reviewer,
    }, separators=(",", ":"), sort_keys=True)
    return f"{REVIEW_PREFIX}{payload}{REVIEW_SUFFIX}"


TRAILER_LINE_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9-]*:\s|^\s")


def _is_trailer_block(block: str) -> bool:
    """Whether a paragraph is a git trailer block.

    Two conditions: it opens with a real trailer rather than a folded
    continuation, and every non-blank line is either a trailer or such a
    continuation. Folding the continuations into their values is the caller's
    job, not this predicate's.

    Git is more permissive — it also accepts a paragraph that is at least a
    quarter trailers *and* contains a known trailer — and the narrower rule is
    deliberate. Erring toward rejection keeps the gate failing closed, which is
    the property being defended; the fuzz test in the suite asserts only that
    direction, never strict parity.
    """
    lines = [line for line in block.splitlines() if line.strip()]
    if not lines or lines[0][:1].isspace():
        # A block opening with a folded continuation has nothing to fold into,
        # so git does not recognise it as a trailer block at all.
        return False
    return all(TRAILER_LINE_RE.match(line) for line in lines)


def implementer_identity(head_message: str | None) -> str | None:
    """The agent session that wrote the head commit, from its trailers.

    This is the other half of review independence. The receipt records who
    reviewed; the commit records who implemented. Both are agent identities,
    which is what `AGENTS.md` asks about. The GitHub account is a poor proxy
    for it: a single operator drives several agents from one account, so equal
    logins say nothing about whether two different contexts saw the change.
    """
    # Only git's trailer block — the final paragraph, and only when it is a
    # real trailer block — is authoritative. A message that mentions
    # `Agent-Session:` in prose, or quotes another commit, must not be able to
    # manufacture an identity and so escape the fail-closed branch.
    blocks = re.split(r"\n\s*\n", (head_message or "").strip())
    if len(blocks) < 2 or not _is_trailer_block(blocks[-1]):
        return None
    keys = {key.lower() for key in AGENT_SESSION_TRAILERS}
    trailers: list[list[str]] = []
    for line in blocks[-1].splitlines():
        if not line.strip():
            continue
        if line[:1].isspace() and trailers:
            # Git folds an indented line into the preceding trailer, preserving
            # the whitespace on both sides of the join.
            trailers[-1][1] += " " + line.strip()
            continue
        key, _, value = line.partition(":")
        trailers.append([key.strip().lower(), value.lstrip()])
    matches = [value.strip() for key, value in trailers if key in keys and value.strip()]
    if not matches:
        return None
    # Last trailer wins: `git commit --amend` and trailer tooling append.
    return matches[-1].strip() or None


def parse_review_receipts(comments: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    receipts = []
    pattern = re.compile(re.escape(REVIEW_PREFIX) + r"(\{.*?\})" + re.escape(REVIEW_SUFFIX))
    for sequence, comment in enumerate(comments):
        match = pattern.search(comment.get("body") or "")
        if not match:
            continue
        try:
            payload = json.loads(match.group(1))
        except json.JSONDecodeError:
            continue
        payload["comment_url"] = comment.get("html_url") or comment.get("url")
        payload["author"] = (comment.get("user") or {}).get("login")
        # Ordering inputs for `latest_receipt`. The issue-comments API returns
        # ascending creation order, so the list index is a correct tiebreaker
        # when two receipts share a timestamp or `created_at` is absent.
        payload["created_at"] = comment.get("created_at")
        payload["sequence"] = sequence
        receipts.append(payload)
    return receipts


def latest_receipt(
    receipts: Iterable[dict[str, Any]], kind: str, head_sha: str
) -> dict[str, Any] | None:
    """Return the most recent structurally valid receipt for one kind and head.

    Review history is resolved newest-first rather than any-pass-wins: a later
    `fail` supersedes an earlier `pass` for the same head, so an unresolved
    finding cannot be left behind by re-posting an older verdict.
    """
    matching = [
        receipt for receipt in receipts
        if receipt.get("schema") == 1
        and receipt.get("kind") == kind
        and receipt.get("head_sha") == head_sha
        and receipt.get("reviewer")
        and receipt.get("author")
    ]
    if not matching:
        return None
    matching.sort(key=lambda receipt: (receipt.get("created_at") or "", receipt.get("sequence") or 0))
    return matching[-1]


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
        "--json", "number,title,state,isDraft,headRefOid,headRefName,baseRefName,body,labels,mergeable,mergeStateStatus,statusCheckRollup,url,author",
    ])


def fetch_issue(number: int, config: dict[str, Any]) -> dict[str, Any]:
    return run_json([
        "gh", "issue", "view", str(number), "--repo", config["repository"],
        "--json", "number,title,state,body,labels,url",
    ])


def fetch_comments(number: int, config: dict[str, Any]) -> list[dict[str, Any]]:
    return run_json(["gh", "api", "--paginate", f"repos/{config['repository']}/issues/{number}/comments"])


def fetch_gate_records(number: int, config: dict[str, Any]) -> list[dict[str, Any]] | None:
    """Gate records on an epic, or None when they cannot be read (fail closed)."""
    try:
        comments = fetch_comments(number, config)
    except (SdlcError, OSError):
        return None
    if not isinstance(comments, list):
        return None
    return parse_gate_records(comments)


def fetch_blocked_by(number: int, config: dict[str, Any]) -> list[dict[str, Any]]:
    """Every native `blocked_by` issue of #number, all pages.

    `--slurp` keeps one list per page so a malformed page is detected rather
    than merged away. A failed page makes `gh` exit non-zero, which raises.
    """
    pages = run_json([
        "gh", "api", "--paginate", "--slurp",
        f"repos/{config['repository']}/issues/{number}/dependencies/blocked_by?per_page=100",
    ])
    # Even an empty answer is one page (`[[]]`); zero pages is not an answer.
    if not isinstance(pages, list) or not pages or not all(isinstance(page, list) for page in pages):
        raise SdlcError(f"blocked_by for #{number} was not a non-empty list of pages")
    blockers = [item for page in pages for item in page]
    if not all(isinstance(item, dict) for item in blockers):
        raise SdlcError(f"blocked_by for #{number} contained a non-issue entry")
    return blockers


PREREQUISITE_QUERY = """
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      number state stateReason url
      closedByPullRequestsReferences(first: 100) {
        totalCount
        pageInfo { hasNextPage }
        nodes { number repository { name owner { login } } }
      }
    }
  }
}
"""


def fetch_prerequisite(number: int, config: dict[str, Any]) -> dict[str, Any]:
    """A prerequisite with every closing reference, or raise.

    `gh issue view --json closedByPullRequestsReferences` requests only the
    first 100 and does not page, so a longer list would be silently cut. Every
    reference must be read, so this query reports completeness and a list it
    cannot show is whole is unknown.
    """
    owner, name = config["repository"].split("/", 1)
    data = run_json([
        "gh", "api", "graphql", "-f", f"query={PREREQUISITE_QUERY}",
        "-F", f"owner={owner}", "-F", f"name={name}", "-F", f"number={number}",
    ])
    issue = (((data or {}).get("data") or {}).get("repository") or {}).get("issue")
    if not isinstance(issue, dict):
        raise SdlcError(f"issue #{number} was not returned")
    refs = issue.get("closedByPullRequestsReferences")
    nodes = refs.get("nodes") if isinstance(refs, dict) else None
    if issue_number(issue.get("number")) != number:
        raise SdlcError(f"GitHub answered for #{issue.get('number')} when #{number} was asked for")
    total = refs.get("totalCount") if isinstance(refs, dict) else None
    page_info = refs.get("pageInfo") if isinstance(refs, dict) else None
    if (
        not isinstance(nodes, list)
        or not isinstance(page_info, dict)
        or page_info.get("hasNextPage") is not False
        or not isinstance(total, int) or isinstance(total, bool)
        or total != len(nodes)
    ):
        raise SdlcError(f"the closing pull requests of #{number} could not be read completely")
    return dict(issue, closedByPullRequestsReferences=nodes)


def fetch_pull_merge(number: int, config: dict[str, Any]) -> dict[str, Any]:
    return run_json([
        "gh", "pr", "view", str(number), "--repo", config["repository"],
        "--json", "number,state,baseRefName,mergedAt,mergeCommit,url",
    ])


def live_dependencies(config: dict[str, Any]) -> DependencySource:
    return DependencySource(
        lambda number: fetch_blocked_by(number, config),
        lambda number: fetch_prerequisite(number, config),
        lambda number: fetch_pull_merge(number, config),
    )


def bundle_dependencies(data: dict[str, Any]) -> DependencySource:
    """Dependency data from an offline bundle. Anything absent is unknown.

    - `dependencies`: `{"<issue>": [blocked_by issue, ...]}` in REST shape;
    - each prerequisite in `issues` carries `stateReason` and
      `closedByPullRequestsReferences`;
    - `pull_requests`: `{"<pr>": {state, baseRefName, mergeCommit}}`.
    """
    def keyed(key: str, number: int, what: str) -> Any:
        stored = data.get(key)
        value = stored.get(str(number)) if isinstance(stored, dict) else None
        if value is None:
            raise SdlcError(f"the bundle has no `{key}` entry for {what} #{number}")
        return value

    def blocked_by(number: int) -> list[dict[str, Any]]:
        blockers = keyed("dependencies", number, "issue")
        if not isinstance(blockers, list):
            raise SdlcError(f"the bundle's `dependencies` entry for #{number} is not a list")
        return blockers

    def issue(number: int) -> dict[str, Any]:
        records = [
            item for item in data.get("issues") or []
            if isinstance(item, dict) and item.get("number") == number
        ]
        if len(records) > 1:
            raise SdlcError(f"the bundle has {len(records)} records for #{number}; which is current is unknown")
        if not records or "stateReason" not in records[0]:
            raise SdlcError(f"the bundle has no prerequisite record with `stateReason` for #{number}")
        return records[0]

    return DependencySource(blocked_by, issue, lambda number: keyed("pull_requests", number, "pull request"))


def evaluate_merge_gate(
    pr: dict[str, Any],
    comments: list[dict[str, Any]],
    issue: dict[str, Any] | None,
    config: dict[str, Any],
    head_message: str | None = None,
    *,
    parent: ParentGate,
    prerequisites: Sequence[str],
    head_parent_count: int = 1,
) -> list[str]:
    errors: list[str] = []
    head = pr.get("headRefOid")
    branch = pr.get("headRefName") or "the slice branch"
    if pr.get("state") != "OPEN":
        errors.append("pull request is not open")
    if pr.get("isDraft"):
        errors.append("pull request is still a draft")
    if pr.get("baseRefName") != config["default_branch"]:
        errors.append(f"base branch is not `{config['default_branch']}`")
    if pr.get("mergeable") != "MERGEABLE":
        errors.append(f"pull request is not confirmed mergeable ({pr.get('mergeable')})")
    merge_state = pr.get("mergeStateStatus")
    if merge_state not in {"CLEAN", "HAS_HOOKS", "UNSTABLE"}:
        if merge_state == "BEHIND":
            errors.append(
                f"merge state is not ready ({merge_state}); rebase `{branch}` onto "
                f"`origin/{config['default_branch']}` and push with --force-with-lease "
                "instead of `gh pr update-branch`, which writes a GitHub-authored merge "
                f"commit carrying no `{AGENT_SESSION_TRAILERS[0]}:` trailer"
            )
        else:
            errors.append(f"merge state is not ready ({merge_state})")
    linked = linked_issue_numbers(pr)
    if len(linked) != 1:
        errors.append(f"PR must close exactly one slice issue; found {len(linked)}")
    if issue is None:
        errors.append("linked slice issue could not be loaded")
    else:
        # Re-evaluated here, not trusted from claim time: a checkpoint pause,
        # a blocker or a closure recorded after the claim must stop the merge.
        errors.extend(admission_errors(issue, parent, config, mode="continue", prerequisites=prerequisites))

    outcomes = pr_check_outcomes(pr)
    for required in config["required_status_checks"]:
        if outcomes.get(required) != "SUCCESS":
            errors.append(f"required check `{required}` is {outcomes.get(required, 'missing')}")

    receipts = parse_review_receipts(comments)
    # An implementer must not be able to write their own passing receipt.
    # Independence is established at the account level when it can be — a
    # receipt from a different GitHub account is the strongest evidence
    # available — and otherwise at the agent level, which is what `AGENTS.md`
    # actually requires: "review comes from a fresh agent". One operator
    # driving several agents from one account satisfies that, and equal logins
    # are not evidence against it. Ambiguity on both axes fails closed.
    pr_author = ((pr.get("author") or {}).get("login") or "").strip()
    if not pr_author:
        errors.append("pull request author is unknown; review independence cannot be established")
    implementer = implementer_identity(head_message)
    # `gh pr update-branch` writes a GitHub-authored merge commit with no
    # trailer of its own. Walking to a trailered parent is deliberately not
    # done here (plan R4: "do not blindly trust the first parent... or the
    # nearest arbitrary trailer"), so that head is indistinguishable from any
    # other trailerless commit except by parent count. Naming rebase instead
    # of "add a trailer" only for that shape keeps the advice accurate without
    # reading history.
    rebase_advice = (
        f"rebase `{branch}` onto `origin/{config['default_branch']}` and push "
        "with --force-with-lease instead of using `gh pr update-branch`, which "
        f"writes a GitHub-authored merge commit carrying no `{AGENT_SESSION_TRAILERS[0]}:` trailer"
    )
    for kind in config["review"]["required"]:
        candidates = [
            receipt for receipt in receipts
            if receipt.get("schema") == 1
            and receipt.get("kind") == kind
            and receipt.get("head_sha") == head
        ]
        dependent: list[dict[str, Any]] = []
        reasons: list[str] = []
        # Parallel to `reasons`: whether that specific message already embeds
        # the rebase advice. Tracked structurally, index-matched to `reasons`
        # — never sniffed from the resulting text with a substring check,
        # which a `kind` named e.g. "rebase-review" would falsely satisfy.
        reasons_named_rebase: list[bool] = []
        for receipt in candidates:
            if pr_author and (receipt.get("author") or "").lower() != pr_author.lower():
                # Separate accounts: independent, no trailer needed. This also
                # applies to a merge-commit head with no trailer of its own —
                # the rebase advice below exists only to establish identity
                # for a *same-account* receipt, which a different account
                # already establishes without it. Do not read `head_parent_count`
                # as "the gate refuses every trailerless merge-commit head";
                # it only refuses one paired with a same-account receipt.
                continue
            reviewer = (receipt.get("reviewer") or "").strip()
            if not reviewer:
                dependent.append(receipt)
                reasons.append(
                    f"`{kind}` receipt for head {head} names no reviewer; re-record it with "
                    f"`review-receipt --reviewer <agent/session> --expect-sha {head}`"
                )
                reasons_named_rebase.append(False)
            elif implementer is None:
                dependent.append(receipt)
                if head_parent_count > 1:
                    reasons.append(
                        f"`{kind}` receipt for head {head} shares the PR author "
                        f"`{pr_author}`, and head commit {head} is a merge commit "
                        f"with no `{AGENT_SESSION_TRAILERS[0]}:` trailer of its own; "
                        + rebase_advice
                    )
                    reasons_named_rebase.append(True)
                else:
                    reasons.append(
                        f"`{kind}` receipt for head {head} shares the PR author "
                        f"`{pr_author}`, and head commit {head} records no implementing "
                        f"agent; add an `{AGENT_SESSION_TRAILERS[0]}:` trailer to the "
                        "commit so the two agents can be told apart"
                    )
                    reasons_named_rebase.append(False)
            elif reviewer.lower() == implementer.lower():
                dependent.append(receipt)
                reasons.append(
                    f"`{kind}` receipt for head {head} was written by the implementing "
                    f"agent `{implementer}`; a self-review is not an independent review"
                )
                reasons_named_rebase.append(False)
        independent = [receipt for receipt in candidates if receipt not in dependent]
        # Independence gates *authorization*, not *objection*. A failing receipt
        # is a veto, and an implementer who finds a defect in their own work
        # must be able to stop the merge with it. Dropping dependent failures
        # here would let an older independent pass outrank a newer self-reported
        # failure, defeating newest-first resolution.
        considered = [
            receipt for receipt in candidates
            if receipt in independent or receipt.get("verdict") == "fail"
        ]
        latest = latest_receipt(considered, kind, head) if pr_author else None
        before = len(errors)
        already_named_rebase = False
        if latest is None:
            if reasons:
                errors.append(reasons[-1])
                already_named_rebase = reasons_named_rebase[-1]
            else:
                errors.append(f"missing clean `{kind}` review receipt for head {head}")
        elif latest.get("verdict") != "pass":
            errors.append(
                f"latest `{kind}` review receipt for head {head} is "
                f"`{latest.get('verdict')}`, not `pass`"
            )
        # Closing the whole class, not one more branch: three review rounds
        # each found a different way for a `kind` error to omit the rebase
        # advice (a fail-verdict receipt short-circuits `reasons` above; a
        # receipt naming no reviewer does too when nothing else is
        # `considered`; the next input would have found another). No matter
        # which branch above produced this kind's error, or whether the
        # receipt driving it was same- or different-account, append the note
        # once here if it isn't already present. `head_parent_count` must
        # never change *whether* the gate blocks — only whether the resulting
        # error also names rebase — so this only ever extends an existing
        # error, never creates one. `already_named_rebase` is tracked
        # structurally (never sniffed from `errors[-1]` text), so a `kind`
        # whose own name happens to contain "rebase" cannot suppress it.
        if len(errors) > before and head_parent_count > 1 and implementer is None \
                and not already_named_rebase:
            errors[-1] += (
                f"; separately, a `{kind}` receipt from the PR author's account can never "
                f"establish independence on this head (a merge commit with no "
                f"`{AGENT_SESSION_TRAILERS[0]}:` trailer of its own) — " + rebase_advice
            )
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


def status_projection(value: Any) -> Any:
    """Strip issue/PR prose from the status model.

    Status is about lane, gate, parentage and freshness. Bodies are content,
    they are large, and GitHub already serves them — including them would make
    the projection unreadable and imply this is a mirror of the tracker rather
    than a view over it.
    """
    if isinstance(value, dict):
        return {k: status_projection(v) for k, v in value.items() if k != "body"}
    if isinstance(value, list):
        return [status_projection(item) for item in value]
    return value


def command_status(args: argparse.Namespace, config: dict[str, Any]) -> int:
    data = load_status_data(args.input, config)
    model = build_model(data, config)
    if args.json:
        # The projection, not the page. This is the seam a real cockpit reads;
        # `render_status` is the interim view that such a cockpit replaces.
        print(json.dumps(status_projection(model), indent=2, sort_keys=True, default=str))
        return 0
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
    # An offline bundle supplies gate records through `gate_comments`; one
    # without the parent's comments leaves the gate unknown and blocks.
    loader = bundle_gate_records(data) if args.input else (lambda number: fetch_gate_records(number, config))
    # Likewise its dependency data: anything the bundle omits blocks.
    dependencies = bundle_dependencies(data) if args.input else live_dependencies(config)
    errors = validate_slice(issue, issues, config, loader, dependencies)
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
        # Everything labeled ready goes through admission, so a ready issue
        # that is not a slice is reported below rather than silently dropped.
        errors = validate_slice(
            issue, issues, config, lambda number: fetch_gate_records(number, config),
            live_dependencies(config), mode="start",
        )
        if not errors:
            candidates.append(issue)
        else:
            # A slice labeled ready but refused is inconsistent state; say
            # why, rather than leaving only an empty queue.
            for error in errors:
                print(f"SKIPPED: issue #{number}: {error}", file=sys.stderr)
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
    branch = f"issue/{args.issue}"
    remote_ref = f"refs/heads/{branch}"
    # One probe of the remote claim decides both the admission mode and the
    # effect. A fresh claim starts work and needs `status:ready`. `--resume`
    # continues an attempt that already owns the claim and has moved past
    # that label, but only when that claim exists: resuming nothing would
    # start new work under the looser rule. Acting on the same probe means a
    # claim deleted after it is never recreated under continue-mode admission.
    remote = run_process(["git", "ls-remote", "--exit-code", "--heads", "origin", remote_ref], check=False)
    if remote.returncode not in {0, 2}:
        raise SdlcError(remote.stderr.strip() or "could not inspect remote claim branch")
    continuing = args.resume and remote.returncode == 0
    errors = validate_slice(
        issue, issues, config, lambda number: fetch_gate_records(number, config),
        live_dependencies(config), mode="continue" if continuing else "start",
    )
    if errors:
        for error in errors:
            print(f"BLOCKED: {error}")
        return 1
    for pr in data.get("pulls", []):
        if args.issue in linked_issue_numbers(pr):
            print(f"BLOCKED: issue #{args.issue} already has open PR #{pr['number']}")
            return 1
    if remote.returncode == 0 and not args.resume:
        print(f"BLOCKED: remote branch `{branch}` already claims issue #{args.issue}; use --resume only for an abandoned session")
        return 1

    run_text(["git", "fetch", "origin", config["default_branch"]])
    if remote.returncode == 2:
        # The `ls-remote` check above is only a fast path to a friendly message.
        # This create-only request is the actual claim: it is decided by the
        # server, so there is no check-then-act window to lose.
        base_sha = run_text(["git", "rev-parse", f"refs/remotes/origin/{config['default_branch']}"]).strip()
        if not create_remote_ref(remote_ref, base_sha, config):
            print(
                f"BLOCKED: another session claimed issue #{args.issue} first; "
                f"remote branch `{branch}` already exists"
            )
            return 1
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
    # The fetch above advanced only the remote-tracking ref. A checked-out
    # local default branch is never moved here, so it may be stale: diff and
    # rebase against this ref, never the local one.
    print(f"BASE=origin/{config['default_branch']}")
    return 0


def command_mark_in_review(args: argparse.Namespace, config: dict[str, Any]) -> int:
    issue = fetch_issue(args.issue, config)
    if issue.get("state") != "OPEN":
        print(f"BLOCKED: issue #{args.issue} is not open (state: {issue.get('state')!r}); "
              f"if it is closed, run `cleanup` instead")
        return 1
    names = label_names(issue)
    in_progress = config["labels"]["in_progress"]
    in_review = config["labels"]["in_review"]
    if in_progress not in names and in_review not in names:
        print(f"BLOCKED: issue #{args.issue} carries neither `{in_progress}` nor `{in_review}`; nothing to transition")
        return 1
    edit = ["gh", "issue", "edit", str(args.issue), "--repo", config["repository"]]
    # Two writes, add before remove, mirroring `command_set_gate`: one `gh
    # issue edit` carrying both flags is not known to be atomic, and removing
    # first could leave the issue carrying neither label. Each write is
    # skipped when its label is already in the state it would produce, so a
    # rerun after a partial failure edits nothing already fixed, and a rerun
    # after full success edits nothing at all.
    if in_review not in names:
        run_text(edit + ["--add-label", in_review])
    if in_progress in names:
        run_text(edit + ["--remove-label", in_progress])
    print(f"IN-REVIEW: issue #{args.issue}")
    return 0


def command_cleanup(args: argparse.Namespace, config: dict[str, Any]) -> int:
    issue = fetch_issue(args.issue, config)
    if issue.get("state") != "CLOSED":
        print(f"BLOCKED: issue #{args.issue} is not closed")
        return 1
    # Delivery labels are cleared first and the status view rendered last, so
    # the view never shows a closed slice as still in progress or in review.
    # Only labels actually present are removed: a rerun after a partial
    # failure edits nothing it already fixed, and unrelated labels stay.
    delivery = [config["labels"][key] for key in ("in_progress", "in_review")]
    stale = [label for label in delivery if label in label_names(issue)]
    if stale:
        command = ["gh", "issue", "edit", str(args.issue), "--repo", config["repository"]]
        for label in stale:
            command += ["--remove-label", label]
        run_text(command)
        print(f"CLEARED: {', '.join(stale)} from issue #{args.issue}")
    branch = f"issue/{args.issue}"
    worktree = ROOT / ".worktrees" / f"issue-{args.issue}"
    if worktree.exists():
        # Never forced: git refuses to remove a worktree with local changes.
        run_text(["git", "worktree", "remove", str(worktree)])
    run_text(["git", "fetch", "origin", "--prune"])
    # Whether the branch is merged is judged against the fetched remote
    # default branch. `git branch -d` would judge it against a pruned
    # upstream's fallback, this checkout's HEAD, which may be a stale local
    # default branch; it then refuses a merged branch on every rerun.
    kept = False
    local = run_process(["git", "show-ref", "--verify", "--quiet", f"refs/heads/{branch}"], check=False)
    if local.returncode == 0:
        base = f"refs/remotes/origin/{config['default_branch']}"
        merged = run_process(["git", "merge-base", "--is-ancestor", f"refs/heads/{branch}", base], check=False)
        if merged.returncode == 0:
            run_text(["git", "branch", "-D", branch])
        elif merged.returncode == 1:
            kept = True
            print(f"BLOCKED: kept local branch `{branch}`: it has commits not in origin/{config['default_branch']}")
        else:
            raise SdlcError(merged.stderr.strip() or f"could not compare `{branch}` with {base}")
    elif local.returncode != 1:
        raise SdlcError("could not inspect local claim branch")
    output = ROOT / config["status"]["output"]
    write_atomic(output, render_status(build_model(fetch_status_data(config), config)))
    print(f"WROTE {output}")
    if kept:
        return 1
    print(f"CLEANED: issue #{args.issue}")
    return 0


def command_review_receipt(args: argparse.Namespace, config: dict[str, Any]) -> int:
    pr = fetch_pr(args.pr, config)
    head = pr["headRefOid"]
    # Before --dry-run deliberately: a preview built for a head the reviewer
    # never read is exactly the misleading output this check exists to stop.
    if head != args.expect_sha:
        print(f"BLOCKED: the review read {args.expect_sha} but PR #{args.pr} is now at {head}")
        print("The head moved during review, so the report describes a commit that is no")
        print("longer this branch's head. Re-run the review against the new head, then")
        print(f"record it with --expect-sha {head}.")
        return 1
    report = args.body_file.read_text(encoding="utf-8")
    marker = review_marker(args.kind, head, args.verdict, args.reviewer)
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
    print(f"RECORDED: {args.kind}={args.verdict} for {head}")
    return 0


def fetch_commit_message(sha: str, config: dict[str, Any]) -> str:
    commit = run_json(["gh", "api", f"repos/{config['repository']}/commits/{sha}"])
    return ((commit.get("commit") or {}).get("message")) or ""


def fetch_commit_parent_count(sha: str, config: dict[str, Any]) -> int:
    """Number of parents; 2+ is a merge commit, such as the one
    `gh pr update-branch` writes with no `Agent-Session:` trailer of its own.

    The gate needs this to tell that shape apart from an ordinary commit that
    simply omitted the trailer, so its error can point at rebase instead of a
    nonsensical amend.
    """
    commit = run_json(["gh", "api", f"repos/{config['repository']}/commits/{sha}"])
    return len(commit.get("parents") or [])


class GateInputs(NamedTuple):
    pr: dict[str, Any]
    comments: list[dict[str, Any]]
    issue: dict[str, Any] | None
    head_message: str
    parent: ParentGate
    prerequisites: tuple[str, ...]
    head_parent_count: int = 1


def load_parent_gate(issue: dict[str, Any] | None, config: dict[str, Any]) -> ParentGate:
    parents = parent_numbers(issue) if issue else set()
    if len(parents) != 1:
        return ParentGate(None, None, None)
    number = next(iter(parents))
    try:
        epic: dict[str, Any] | None = fetch_issue(number, config)
    except (SdlcError, OSError):
        epic = None
    return ParentGate(number, epic, fetch_gate_records(number, config))


def load_gate_inputs(pr_number: int, config: dict[str, Any]) -> GateInputs:
    pr = fetch_pr(pr_number, config)
    comments = fetch_comments(pr_number, config)
    linked = linked_issue_numbers(pr)
    issue = fetch_issue(next(iter(linked)), config) if len(linked) == 1 else None
    # The head commit carries the implementing agent's identity in a trailer.
    # Fetched here so `evaluate_merge_gate` stays a pure function of its inputs.
    head_message = fetch_commit_message(pr["headRefOid"], config) if pr.get("headRefOid") else ""
    head_parent_count = fetch_commit_parent_count(pr["headRefOid"], config) if pr.get("headRefOid") else 1
    # Reloaded live like the parent: a prerequisite reopened after the claim
    # must stop the merge.
    prerequisites = (
        tuple(prerequisite_errors(int(issue["number"]), live_dependencies(config), config))
        if issue else ()
    )
    return GateInputs(
        pr, comments, issue, head_message, load_parent_gate(issue, config), prerequisites, head_parent_count,
    )


class MergeEligibility(NamedTuple):
    head_sha: str
    errors: tuple[str, ...]


def merge_eligibility(pr_number: int, config: dict[str, Any]) -> MergeEligibility:
    pr, comments, issue, head_message, parent, prerequisites, head_parent_count = load_gate_inputs(pr_number, config)
    errors = evaluate_merge_gate(
        pr, comments, issue, config, head_message, parent=parent, prerequisites=prerequisites,
        head_parent_count=head_parent_count,
    )
    head = pr.get("headRefOid") or ""
    if not re.fullmatch(r"[0-9a-f]{40}", head):
        errors.append("evaluated candidate is not a full commit SHA")
    return MergeEligibility(head, tuple(errors))


def print_merge_eligibility(pr_number: int, result: MergeEligibility) -> None:
    if result.errors:
        for error in result.errors:
            print(f"BLOCKED: {error}")
    else:
        print(f"ELIGIBLE: PR #{pr_number} may be merged at {result.head_sha}")


def command_merge_gate(args: argparse.Namespace, config: dict[str, Any]) -> int:
    result = merge_eligibility(args.pr, config)
    print_merge_eligibility(args.pr, result)
    return int(bool(result.errors))


def confirm_merge(pr_number: int, head_sha: str, config: dict[str, Any]) -> str:
    """Read GitHub after every submission, even a failed/lost response.

    No write is retried here. An unreadable, pending, or inconsistent result
    remains blocked until a later read can establish what happened.
    """
    pr = run_json([
        "gh", "pr", "view", str(pr_number), "--repo", config["repository"],
        "--json", "state,headRefOid,baseRefName,mergedAt,mergeCommit",
    ])
    if pr.get("state") != "MERGED":
        raise SdlcError(
            f"PR #{pr_number} is not confirmed merged (state={pr.get('state')}, "
            f"head={pr.get('headRefOid')}); inspect GitHub before any retry"
        )
    if pr.get("headRefOid") != head_sha or pr.get("baseRefName") != config["default_branch"]:
        raise SdlcError("merged PR does not match the evaluated candidate and target branch")
    merge_sha = (pr.get("mergeCommit") or {}).get("oid") or ""
    if not pr.get("mergedAt") or not re.fullmatch(r"[0-9a-f]{40}", merge_sha):
        raise SdlcError("merged PR has no confirmed merge timestamp and full resulting commit SHA")
    commit = run_json(["gh", "api", f"repos/{config['repository']}/commits/{merge_sha}"])
    if commit.get("sha") != merge_sha:
        raise SdlcError("resulting commit identity could not be verified")
    if config["merge"]["method"] == "merge":
        parents = [parent.get("sha") for parent in commit.get("parents", [])]
        if len(parents) != 2 or parents[1] != head_sha:
            raise SdlcError("resulting merge commit does not have the evaluated candidate as its second parent")
    return merge_sha


def command_merge(args: argparse.Namespace, config: dict[str, Any]) -> int:
    result = merge_eligibility(args.pr, config)
    print_merge_eligibility(args.pr, result)
    if result.errors:
        return 1
    if not args.apply:
        print("DRY RUN: pass --apply to merge")
        return 0
    method = config["merge"]["method"]
    command = [
        "gh", "pr", "merge", str(args.pr), "--repo", config["repository"],
        f"--{method}", "--match-head-commit", result.head_sha,
    ]
    if config["merge"].get("delete_branch"):
        command.append("--delete-branch")
    submission_error = None
    try:
        run_text(command)
    except (SdlcError, OSError) as exc:
        submission_error = str(exc)
        print(f"MERGE RESPONSE: {submission_error}; reconciling with GitHub")
    try:
        merge_sha = confirm_merge(args.pr, result.head_sha, config)
    except (SdlcError, OSError) as exc:
        print(f"BLOCKED: merge outcome not verified: {exc}")
        return 1
    if submission_error:
        print("RECONCILED: GitHub confirms the merge despite the command error")
    print(f"MERGED: PR #{args.pr} candidate {result.head_sha} commit {merge_sha}")
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
    checkpoint = (args.checkpoint or "").strip()
    decided_by = (args.decided_by or "").strip()
    if not checkpoint:
        raise SdlcError("--checkpoint must name the checkpoint report or reference under review")
    if args.state != "review" and not decided_by:
        # An agent may faithfully record a decision it was given. It must not
        # produce one, so a decision without an attributed human is refused.
        raise SdlcError(
            f"`{args.state}` records a human decision; pass --decided-by naming the "
            "person who made it, and put their words in --note-file"
        )
    for flag, value in (("--checkpoint", checkpoint), ("--decided-by", decided_by)):
        if "-->" in value or any(char in value for char in "\r\n"):
            raise SdlcError(f"{flag} must be a single line and must not contain `-->`")

    targets = gate_labels(config)
    current = label_names(epic)
    if args.state != "review" and targets["review"] not in current:
        # A decision answers a checkpoint that is under review. This also
        # admits the legacy dual-label state (review + changes-requested),
        # which recording the actual decision reconciles.
        raise SdlcError(
            f"`{args.state}` decides a checkpoint under review, but epic #{args.epic} "
            f"is not labeled `{targets['review']}`; request review first"
        )
    if args.state != "review":
        # The decision must answer the checkpoint actually put under review,
        # as recorded by `set-gate --state review`. A missing or different
        # review record (e.g. its post failed) is reconciled by requesting
        # review again, not by deciding an unrecorded checkpoint.
        records = fetch_gate_records(args.epic, config)
        if records is None:
            raise SdlcError(f"gate records on epic #{args.epic} could not be read; the checkpoint under review is unknown")
        latest = latest_gate_record(records)
        if not complete_gate_record(latest, "review"):
            raise SdlcError(
                f"epic #{args.epic} has no current schema-1 review record; run "
                "`set-gate --state review --checkpoint <report>` first"
            )
        if latest.get("checkpoint") != checkpoint:
            raise SdlcError(
                f"--checkpoint {checkpoint!r} does not match the checkpoint under review "
                f"({latest.get('checkpoint')!r}, {latest.get('comment_url')})"
            )
    add = [targets[args.state]] if targets[args.state] not in current else []
    remove = [label for state, label in targets.items() if state != args.state and label in current]
    edit = ["gh", "issue", "edit", str(args.epic), "--repo", config["repository"]]
    # Two writes, add before remove: one `gh issue edit` carrying both flags is
    # not known to be atomic, and removing first could leave no gate at all.
    # Failing between these leaves conflicting labels, which block everything.
    label_commands = [edit + ["--add-label", name] for name in add]
    if remove:
        label_commands.append(edit + [part for name in remove for part in ("--remove-label", name)])
    record = f"{gate_marker(args.state, checkpoint, decided_by)}\n\n{note}\n"
    if not args.apply:
        for command in label_commands:
            print("DRY RUN:", " ".join(command))
        print(record)
        return 0

    def apply_labels() -> None:
        for command in label_commands:
            run_text(command)

    def post_record() -> str:
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".json", delete=False) as handle:
            json.dump({"body": record}, handle)
            temp_name = handle.name
        try:
            # The REST response returns the comment's URL, which a feedback
            # issue cites as its `Feedback decision:`.
            posted = run_json([
                "gh", "api", "--method", "POST",
                f"repos/{config['repository']}/issues/{args.epic}/comments",
                "--input", temp_name,
            ])
        finally:
            Path(temp_name).unlink(missing_ok=True)
        url = (posted or {}).get("html_url") if isinstance(posted, dict) else None
        if not url:
            raise SdlcError(
                f"gate record posted to epic #{args.epic} returned no URL; read the epic's "
                "comments before retrying"
            )
        return url

    # Tighten first, relax last, so a failure between the two writes leaves the
    # epic at least as paused as either state. `review` pauses: label first.
    # A decision relaxes the pause: its record exists before the label changes,
    # and until then the epic stays under review.
    if args.state == "review":
        apply_labels()
        record_url = post_record()
    else:
        record_url = post_record()
        apply_labels()
    print(f"GATE={args.state} epic=#{args.epic}")
    print(f"GATE_RECORD={record_url}")
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

    status = sub.add_parser("status", help="project current delivery state (local HTML view, or --json)")
    status.add_argument("--input", type=Path, help="offline JSON bundle instead of live GitHub")
    status.add_argument("--output", type=Path, help="where to write the local HTML view (untracked)")
    status.add_argument("--snapshot", type=Path, help="also write the raw fetched GitHub bundle")
    status.add_argument("--json", action="store_true", help="print the status projection instead of writing HTML")
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

    mark_in_review = sub.add_parser("mark-in-review", help="own the in-progress -> in-review transition once a slice's PR opens")
    mark_in_review.add_argument("issue", type=int)
    mark_in_review.set_defaults(func=command_mark_in_review)

    cleanup = sub.add_parser("cleanup", help="after merge: clear delivery labels, remove the worktree and local branch, then regenerate status")
    cleanup.add_argument("issue", type=int)
    cleanup.set_defaults(func=command_cleanup)

    receipt = sub.add_parser("review-receipt", help="post a SHA-bound structured review result")
    receipt.add_argument("--pr", type=int, required=True)
    receipt.add_argument("--kind", choices=["advisor", "codex"], required=True)
    receipt.add_argument("--verdict", choices=["pass", "fail"], required=True)
    receipt.add_argument("--reviewer", required=True, help="agent/session identity shown in the receipt")
    receipt.add_argument(
        "--expect-sha", required=True,
        help="full head SHA the review actually read; recording is refused if the PR head has moved",
    )
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
    gate_state.add_argument("--state", choices=GATE_STATES, required=True)
    gate_state.add_argument("--note-file", type=Path, required=True)
    gate_state.add_argument("--checkpoint", required=True, help="checkpoint report/reference the gate concerns")
    gate_state.add_argument("--decided-by", help="the human whose decision this records (required unless --state review)")
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
