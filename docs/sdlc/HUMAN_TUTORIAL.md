# A human's guide to the Cubism delivery system

This guide explains how to direct long-running Cubism projects without managing
every pull request. You choose outcomes, review meaningful demonstrations, and
give feedback. Agents plan and deliver bounded slices between those checkpoints.

Start with the live [delivery cockpit](../project-status.html). It answers four
questions at a glance:

1. What has shipped?
2. What is being worked on or reviewed now?
3. What outcome comes next?
4. Where is a human decision needed?

The cockpit is a generated snapshot of GitHub. GitHub issues and pull requests
remain the authoritative record.

## The model in one minute

```text
You define an outcome
        ↓
An agent turns it into one-session slices
        ↓
Agents implement, test, review, and merge each slice
        ↓
A coherent result becomes an HTML report and demo
        ↓
You approve it or give feedback
        ↓
Approved: continue     Feedback: create correction slices, then show it again
```

There are three units to remember:

| Unit | Meaning | Human involvement |
|---|---|---|
| **Epic** | A long-term product or engineering outcome | Define direction and approve checkpoints |
| **Slice** | One testable change that fits one agent session | Normally none |
| **Checkpoint** | Several merged slices that form something coherent | Review an HTML report and, when possible, a demo |

A slice is deliberately smaller than a milestone. It carries only enough
context for one Claude Code, Codex, or Meta Muse session, including validation
and documentation.

## Your responsibilities

You own the decisions that benefit from product judgment:

- decide which epic matters next;
- clarify an outcome when the planner finds genuine ambiguity;
- review milestone reports and demonstrations;
- approve the integrated result or describe what should change;
- reprioritize when business or product needs change.

You do **not** normally need to:

- review individual implementation pull requests;
- decide how an agent divides files or writes tests;
- copy state between sessions;
- remember which branch owns an issue;
- manually merge a passing slice.

## What the agents and scripts own

| Work | Owner |
|---|---|
| Decompose an epic into bounded, ordered slices | Planning agent |
| Prevent two sessions from claiming the same issue | `scripts/sdlc.py claim` |
| Implement and test a slice in an isolated worktree | Implementation agent |
| Review a Claude plan and diff when available | Claude advisor |
| Review every committed PR independently | Fresh Codex agent/session |
| Confirm CI, current SHA, issue type, and review receipt | Deterministic merge gate |
| Merge a passing slice | Delivery agent |
| Build the cockpit and milestone HTML | Deterministic renderer |

The agent may exercise engineering judgment. It may not invent a passing review,
bypass a failed gate, or silently turn a slice into a multi-session project.

## First-time setup

The repository expects GitHub CLI authentication, Rust, Python 3, and `rtk`.
The local quality gate also uses `lychee`, `shellcheck`, and `maturin` when they
are available.

Check the important tools:

```bash
rtk gh auth status
rtk cargo --version
rtk python3 --version
rtk maturin --version
```

Install `maturin` with the already-supported `uv` tool manager:

```bash
rtk uv tool install maturin
```

Initialize or reconcile the repository's GitHub labels once:

```bash
rtk python3 scripts/sdlc.py bootstrap-labels --apply --classify-epics
```

No MCP server is required. GitHub CLI, Git, and the repository scripts provide
the complete control plane.

## Step 1: choose an epic

Open [the delivery cockpit](../project-status.html) and look at **Roadmap
outcomes**. An epic marked `needs-slicing` has a useful goal but is not yet safe
to hand to an implementation agent.

Choose one epic based on product priority. Then ask an agent:

> Plan epic #N into session-sized slices. Use the `plan-epic` skill. Do not
> implement a slice in this planning session. Show me the first checkpoint and
> the demonstration I will receive.

The planning session should produce:

- vertical slices with observable outcomes;
- explicit acceptance criteria and validation;
- dependency order;
- the first safe `status:ready` slice;
- one or more coherent human checkpoints;
- a proposed demo for each checkpoint.

Planning and implementation are separate sessions on purpose. The implementer
gets a clean context focused on one issue instead of inheriting the entire epic
discussion.

## Step 2: let agents deliver ready slices

Once a slice is ready, start a fresh implementation session with either:

> Work issue #N using the `work-slice` skill. Continue through independent
> review and automatic merge. Stop only if a deterministic gate blocks you or
> the issue no longer fits one session.

or:

> Implement the next ready slice using `work-slice`.

The session will validate and claim the issue, create an isolated worktree,
implement it, run focused and repository-level checks, open a PR, obtain a fresh
Codex review, and merge only if the deterministic gate passes.

When Claude Code is the implementer, it should request advisor review of its
plan and committed diff when the advisor is available. The advisor supplements
the required Codex review; it does not replace it.

### What “automatic merge” actually means

The merge is unattended, but it is not unconditional. The command refuses to
merge unless all of these are true:

- the PR closes exactly one issue labeled `type:slice` or `type:feedback`;
- the PR is current, mergeable, and conflict-free;
- `CI required checks` passed;
- a fresh Codex reviewer recorded a passing receipt for the exact head SHA.

A new commit invalidates the old review receipt. Missing or ambiguous state
blocks the merge rather than guessing.

## Step 3: see what is happening

Ask an agent:

> Refresh the project status and explain only what is active, blocked, awaiting
> my review, and still unmapped.

Or generate the cockpit directly:

```bash
rtk python3 scripts/sdlc.py status
rtk open docs/project-status.html
```

The main lanes mean:

| Lane | Interpretation | What you should do |
|---|---|---|
| **Ready next** | Safe for a new agent session | Usually nothing |
| **In progress** | Claimed and being implemented | Usually nothing |
| **Agent review** | PR or independent review is active | Usually nothing |
| **Human review** | A milestone report/demo needs your decision | Review it |
| **Human feedback** | Your requested corrections are being delivered | Wait or clarify |
| **Blocked** | A real prerequisite or failed gate stopped progress | Read the linked issue |

The roadmap section shows epic progress and child slices. **Backlog health**
shows open issues that are not yet attached to an epic; those should be triaged
rather than silently disappearing from view.

Because the cockpit is static HTML, its timestamp matters. Refresh it after
merges, gate changes, or issue-triage sessions.

## Step 4: review a milestone

Agents create a checkpoint when merged slices form something coherent enough to
evaluate—not after an arbitrary number of PRs. Ordinary roadmap work under that
epic pauses while the checkpoint awaits you.

The milestone report should tell you:

- what outcome is now available;
- which slices are included;
- important decisions and tradeoffs;
- known gaps deliberately left for later;
- exact demo steps and expected results.

Review the integrated behavior, not individual code hunks. Ask:

1. Does this solve the intended problem?
2. Is the behavior understandable and useful?
3. Are the tradeoffs acceptable?
4. Does the next planned checkpoint still make sense?

### Approve

Give an explicit response such as:

> I approve checkpoint `<name>` for epic #N. Record the approval and continue
> with the next planned slice.

The agent records your words on the epic, applies `gate:approved`, refreshes the
cockpit, and allows the next roadmap slice to proceed.

### Request changes

Describe the observed result and desired result in plain language:

> Checkpoint `<name>` for epic #N needs changes. When I do X, I observe Y; I
> expected Z. Track this feedback as bounded feedback slices, pause unrelated
> epic work, and show me a revised checkpoint after they merge.

The agent preserves the feedback, applies `gate:changes-requested`, and creates
one or more `type:feedback` issues. Those correction slices may proceed while
ordinary roadmap slices remain paused. You receive a new report/demo after the
feedback merges; earlier reports remain in git as history.

Silence, a PR approval, or a casual positive comment is not milestone approval.
The decision must be explicit.

## A complete example

Imagine epic `#120` is “Users can compare two time ranges.” The planner might
create:

1. `#121` — represent comparison ranges in the query model;
2. `#122` — return comparison results through the API;
3. `#123` — render the comparison in the dashboard;
4. checkpoint — run a fixed demo comparing this week with last week.

Each slice can be implemented, tested, reviewed, and merged independently. When
all three form a coherent experience, the epic moves to Human review. You see a
report, run the demo, and either approve or request a specific correction.

The numbers above are illustrative; always use the actual GitHub issues linked
from the current epic.

## Common operating recipes

### Start a new long-term initiative

1. Create an Epic issue using the GitHub Epic form.
2. Describe the outcome and human-visible success criteria.
3. Apply `type:epic` and `needs-slicing` if the form did not do so.
4. Run a dedicated `plan-epic` session.
5. Begin fresh `work-slice` sessions only after the slice checker passes.

### Deliver an urgent bug fix

An urgent issue still needs a parent epic and the slice contract. Attach it to
the most relevant lifecycle/maintenance epic, make its scope and regression test
explicit, then mark it `status:ready`. Urgency does not waive review or CI.

### Pause work

Apply `status:blocked` only for a real dependency or external impediment and
record the reason on the issue. For a product-direction pause, use the human
milestone gate on the epic instead of pretending every child is technically
blocked.

### Resume abandoned implementation

Do not create a second claim. Ask the agent to inspect the existing `issue/N`
branch and use `claim N --resume` only after confirming ownership and state.

### Change the SDLC itself

Treat process changes as normal slices. Update `.sdlc/config.json`, `SDLC.md`,
scripts, tests, and affected skills together. Do not create an agent-only rule
that contradicts the deterministic gate.

## Troubleshooting

| Symptom | Likely meaning | Response |
|---|---|---|
| `NONE: no executable ... slice` | No valid ready work exists | Run `plan-epic` or repair slice metadata |
| `check-slice` reports missing sections | The issue is not a complete session handoff | Add the exact missing scope, validation, or context |
| Claim branch already exists | Another session claimed it, or work was abandoned | Inspect ownership; use `--resume` only deliberately |
| Merge says missing review receipt | No passing Codex review matches the current SHA | Run a fresh independent review |
| Merge says CI is missing/failing | Required checks are not green | Fix the failure or wait for CI |
| Epic is in Human review | Ordinary child work is intentionally paused | Review the linked report/demo |
| Work appears under Backlog health | It is not mapped cleanly to an epic | Triage and add an explicit parent relationship |
| Cockpit looks stale | It is a generated snapshot | Run the status command again |

Do not work around a blocked merge by merging manually. The blocker is the
system explaining which trust condition is missing.

## Command reference for operators

```bash
# Refresh/open the visual status
rtk python3 scripts/sdlc.py status
rtk open docs/project-status.html

# Validate or select work
rtk python3 scripts/sdlc.py check-slice ISSUE
rtk python3 scripts/sdlc.py next

# Inspect merge eligibility without merging
rtk python3 scripts/sdlc.py merge --pr PR

# Reconcile labels and classify old Epic-titled issues
rtk python3 scripts/sdlc.py bootstrap-labels --apply --classify-epics

# Reproduce required checks locally
rtk scripts/quality-gate.sh
```

The complete policy is in [`SDLC.md`](../../SDLC.md). The mechanics are in
[`scripts/sdlc.py`](../../scripts/sdlc.py), and the rationale and comparison to
the prior repositories are in [`docs/sdlc/design.md`](design.md).
