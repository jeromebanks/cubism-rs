# Cubism software-development lifecycle

This is the authoritative delivery policy for every crate and binding in this
repository. Machine-enforced values live in [`.sdlc/config.json`](.sdlc/config.json).
Agent-specific instructions may explain how to operate a tool, but may not define
a different lifecycle.

## Desired operating model

Humans set direction and approve coherent product milestones. Agents execute
small slices, test them, review each other's work, and merge passing pull
requests. GitHub issues and pull requests are the durable record. Generated HTML
turns that record into a readable project cockpit and milestone demonstrations.

```text
epic outcome
   ├── ready slice → claimed → implementation → PR → agent review → auto-merge
   ├── ready slice → claimed → implementation → PR → agent review → auto-merge
   └── coherent checkpoint → HTML report/demo → HUMAN GATE
                                             ├── approve → continue
                                             └── feedback → feedback slices → repeat
```

## Work hierarchy

- **Epic** — a long-term outcome, represented by a `type:epic` GitHub issue.
  Its body links child issues and states exit criteria. An epic is never claimed
  as an implementation task.
- **Slice** — one independently testable vertical unit, represented by a
  `type:slice` issue. It must fit in one Claude Code, Codex, or Meta Muse
  session, including tests and documentation.
- **Feedback slice** — human milestone feedback converted into a
  `type:feedback` issue. It follows the same implementation and review loop as
  a normal slice and is allowed to run while its parent epic is in a
  changes-requested gate.
- **Checkpoint** — a coherent set of merged slices that is useful for a human
  to review. It need not close the whole epic.

Work that is valuable but too broad receives `needs-slicing`; it is not selected
by an implementation agent until decomposed.

## Executable-slice contract

A slice is ready only when `rtk python3 scripts/sdlc.py check-slice N` passes. Its
issue must identify exactly one parent epic and contain:

- the observable outcome;
- bounded in-scope work and explicit non-goals;
- checkable acceptance criteria;
- exact validation commands or behaviors;
- the human-visible effect or demo impact;
- the minimum context links needed by a fresh session.

The issue is the session handoff. Do not preload the entire epic, historical
time-series handoffs, or unrelated architecture documents. Follow only the
links in the slice's `Context` section unless implementation uncovers a specific
need for more.

## Slice lifecycle

1. **Select.** Choose a `status:ready` slice whose parent is not paused at a
   human gate. Never pick an epic or an unclassified backlog item.
2. **Validate and claim.** Run the slice checker, then
   `rtk python3 scripts/sdlc.py claim N`. The stable remote branch `issue/N` is an
   atomic, cross-session claim; the command creates an isolated worktree and
   adds `in-progress`.
3. **Plan.** Write a short plan against the issue's acceptance criteria. In
   Claude Code, request an advisor review when that facility is available.
   Record material decisions on the issue rather than in a transient handoff.
4. **Implement.** Stay inside the slice worktree and scope. If the work expands
   beyond one session or reveals an architectural decision, stop, update the
   issue, and split or create an RFC instead of silently absorbing more work.
5. **Verify.** Run focused tests while iterating and
   `rtk scripts/quality-gate.sh <worktree>` before opening the PR. CI is the final
   authority for required checks.
6. **Self-review.** Review the actual diff against acceptance criteria,
   regressions, error paths, and documentation. Claude should request a second
   advisor pass when available.
7. **Open the PR.** The PR closes exactly one slice (`Closes #N`), describes the
   observable result, and reports actual validation. Move the issue from
   `in-progress` to `in-review`.
8. **Independent review.** A fresh Codex agent reviews the committed PR diff.
   The implementer may not impersonate this reviewer. Findings are fixed or
   explicitly resolved, then the updated head is reviewed again. The reviewer
   records a SHA-bound receipt with `rtk python3 scripts/sdlc.py review-receipt`.
9. **Merge automatically.** Run `rtk python3 scripts/sdlc.py merge --pr P --apply`. It
   refuses to merge unless the PR closes one slice, required CI is green, the
   branch is current and conflict-free, and every required review has a clean
   receipt for the exact head SHA. There is no routine human PR gate and no
   size-based exception to the requested auto-merge policy.
10. **Reconcile.** Confirm the issue closed, run
    `rtk python3 scripts/sdlc.py cleanup N`, regenerate the project cockpit, and
    update the parent epic checklist when GitHub did not do so automatically.

Any new commit invalidates prior review receipts because the recorded SHA no
longer matches. A review failure, missing tool, missing CI result, ambiguous issue
link, or merge-state uncertainty fails closed.

## Human milestone gate

Agents choose a checkpoint when merged slices form a coherent user-visible or
architecturally meaningful result—not after an arbitrary count. At that point:

1. Add `gate:human-review` to the epic. Normal slices under that epic pause.
2. Create `docs/milestones/epic-N/<checkpoint>.json` from the manifest template.
   The narrative is agent-authored; the included issue numbers and demo steps
   must be concrete.
3. Render it with `rtk python3 scripts/sdlc.py milestone-report ...`. The renderer
   verifies that every included slice is closed and belongs to the epic, then
   creates the HTML a human reviews.
4. Link the report and demo from the epic issue. The project cockpit shows the
   epic in its Human review lane.
5. Record the human response on the epic:
   - approval adds `gate:approved` and removes the pause so the next checkpoint
     may proceed;
   - feedback adds `gate:changes-requested`, creates linked feedback slices,
     and retains the pause for ordinary roadmap work.
6. After feedback slices merge, regenerate the report/demo and request review
   again. Keep the prior report in git as checkpoint history.

Human approval is about the integrated outcome, product direction, and demo.
The individual implementation PRs remain agent-reviewed and automatically
merged.

## Visibility

Run:

```bash
rtk python3 scripts/sdlc.py status
```

This generates [`docs/project-status.html`](docs/project-status.html) from live
GitHub state. It shows:

- work in progress, in agent review, blocked, or at a human gate;
- progress and child slices for each epic;
- open work that is not mapped to an epic;
- recently completed work and planning-data warnings.

GitHub remains authoritative; the HTML is a dated snapshot. Regenerate it after
every merge, gate change, or feedback-tracking pass. It is intentionally static,
self-contained, and dependency-free so a human can open it directly.

For a human-oriented walkthrough, see
[`docs/sdlc/HUMAN_TUTORIAL.md`](docs/sdlc/HUMAN_TUTORIAL.md). The
[`HTML overview deck`](docs/sdlc/overview-slides.html) is a concise presentation
of the operating model and works directly in a browser.

## Labels and state ownership

`.sdlc/config.json` is the label vocabulary. Create/update the labels
idempotently with:

```bash
rtk python3 scripts/sdlc.py bootstrap-labels --apply
```

The implementation agent owns `status:ready → in-progress → in-review`. The
review/merge loop closes the slice. The human owns milestone approval or
feedback; an agent may apply the corresponding labels only while faithfully
recording the human's decision on the epic.

## Required checks

`CI required checks` is the stable branch-protection check. It aggregates Rust
formatting, clippy, workspace tests and doctests, rustdoc, example schema
validation, internal links, and Python-binding build/import smoke tests. Broader
dependency, security, Spark, and performance suites remain scheduled/nightly
unless an issue explicitly promotes them to the required gate.

## Retired time-series process

The shared `feature/timeseries-phase-0a` lane completed its purpose and merged in
PR #24. `.claude/skills/timeseries-slice/`, `TIMESERIES_FINISH_PROMPT.md`,
`docs/sdlc-process-notes.md`, and the phase handoffs are historical evidence,
not active instructions. All future time-series work uses this same slice/PR
lifecycle. Historical documents are preserved because they contain useful
technical evidence; agents should not load them unless a current slice links to
one specifically.

## Why this differs from `postscript_interpreter`

The PostScript repository supplied a strong issue → worktree → PR → independent
review → merge loop, useful CI scripts, and an issue-summary view. Cubism keeps
those strengths and changes four things:

1. it makes one-session slice readiness machine-checkable;
2. it uses one portable config and script instead of embedding mechanics in a
   very large Claude-only skill;
3. it binds review approval to the exact PR head SHA before auto-merge;
4. it adds epic progress, HTML milestone reports/demos, and an explicit human
   feedback gate.

See [`docs/sdlc/design.md`](docs/sdlc/design.md) for the detailed comparison and
maintenance boundaries.
