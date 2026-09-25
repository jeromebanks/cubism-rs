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
  changes-requested gate, provided it cites that decision with one
  `Feedback decision: <gate record URL>` line.
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

### Admission: start and continue

One predicate in `scripts/sdlc.py` decides whether an issue is executable, for
every command that admits work. It has three modes:

| Mode | Commands | Requires |
| --- | --- | --- |
| check | `check-slice` | open; `type:slice` or `type:feedback`; not an epic; no `status:blocked` or `needs-slicing`; the contract sections above; exactly one parent epic whose gate admits it; every native prerequisite successfully completed |
| start | `next`, `claim`, and `claim --resume` when no `issue/N` claim exists | everything `check` requires, and `status:ready` |
| continue | `claim --resume` on an existing `issue/N` claim, `merge` | open; slice or feedback; no `status:blocked` or `needs-slicing`; exactly one parent epic whose gate admits it; every native prerequisite successfully completed |

`check-slice` does not require `status:ready`: a planner runs it to decide
whether an issue may be labeled ready. An already-owned attempt continues
without regaining `status:ready` and is not re-judged on its contract sections,
but a blocker, closure or gate pause recorded after the claim still stops it.

### Prerequisites

A prerequisite is a native GitHub issue dependency: the slice's `blocked_by`
list. Every mode reads that list completely, following each prerequisite's own
`blocked_by` transitively, and admits the slice only when each direct
prerequisite is **successfully completed**:

- closed with reason `COMPLETED`, and
- closed by at least one pull request in this repository that is verified
  merged into the default branch.

Open, reopened, `NOT_PLANNED`, duplicate or hand-closed prerequisites do not
count. A closing reference that is listed but unmerged, or merged into another
branch, does not count either. A closing reference that is malformed, cannot
be read, or leads to a pull request record inconsistent with its state blocks,
even when another reference is merged. A consistent record has a known
`state`, a `baseRefName`, and a `mergeCommit` that is null unless `MERGED` and
carries a full SHA when `MERGED`. Each refusal names the
prerequisite and the missing evidence. A verification-only prerequisite, one
with no merged closing pull request, fails closed until an explicit evidence
format exists (Stage C3). The following are all unknown, and unknown blocks:

- a dependency cycle, reported with its path;
- a blocker in another repository;
- a failed or malformed page;
- a malformed or incomplete issue, reference or pull request record, or a
  closing-reference list not shown to be complete;
- a 403 or 404.

Native `blocked_by` links are the one dependency authority. An issue's
`## Dependencies` prose explains them and is never read by the tooling; when
prose and links disagree, reconcile the disagreement on the parent epic and
correct both. Add a link with the blocker's numeric issue `id`, not its number:

```bash
BLOCKER_ID=$(rtk proxy gh api repos/<owner>/<repo>/issues/<blocker> --jq .id)
rtk proxy gh api -X POST repos/<owner>/<repo>/issues/<dependent>/dependencies/blocked_by \
  -F issue_id="$BLOCKER_ID"
rtk proxy gh api repos/<owner>/<repo>/issues/<dependent>/dependencies/blocked_by \
  --jq '[.[].number]'
```

The last line reads the list back; confirm it names the intended blocker. Link
to the leaf slice that will merge, never to a container issue that is closed by
hand, because a hand-closed prerequisite never counts as completed. Removing a
link is a recorded decision on the parent epic, not a way to unblock work; see
GitHub's [issue dependencies API](https://docs.github.com/en/rest/issues/issue-dependencies).
Epic #38's open plan work was migrated to native links by R3c (#99); other
epics have not been migrated: the tooling cannot see their prose
prerequisites, so add the native link before marking such a dependent ready.

An offline `check-slice --input` bundle must supply the same data. Anything it
omits blocks:

- `"dependencies": {"<issue>": [<REST blocked_by issues>]}`, with an entry
  for the slice and for every prerequisite reached from it;
- each prerequisite in `issues`, carrying `state`, `stateReason` and the
  complete `closedByPullRequestsReferences` list
  (`[{"number", "repository": {"name", "owner": {"login"}}}]`);
- `"pull_requests": {"<pr>": {"number", "state", "baseRefName", "mergeCommit"}}`.

Each record must be the one asked for: its `number` matches, and it appears
exactly once. A blocker's `repository_url` must be exactly
`https://api.github.com/repos/<owner>/<repo>`, and every issue and pull request
number must be a positive integer.

The issue is the session handoff. Do not preload the entire epic, historical
time-series handoffs, or unrelated architecture documents. Follow only the
links in the slice's `Context` section unless implementation uncovers a specific
need for more.

## Slice lifecycle

1. **Select.** Choose a `status:ready` slice whose parent is not paused at a
   human gate. Never pick an epic or an unclassified backlog item.
2. **Validate and claim.** Run the slice checker, then
   `rtk python3 scripts/sdlc.py claim N`, which admits only in start mode. The stable remote branch `issue/N` is an
   atomic, cross-session claim: it is created through GitHub's create-only ref
   endpoint, so exactly one racing session wins and the others are told they
   lost. The command then creates an isolated worktree, adds `in-progress`, and
   prints `BASE=origin/main`.

   `origin/<default branch>` is the only ref to diff, log, or rebase a slice
   against (`git diff origin/main...HEAD`). `claim` fetches it but never moves
   the local default branch, which may be checked out in the primary checkout
   and so can be arbitrarily stale; a diff against local `main` can show
   unrelated upstream commits.

   The claim has no heartbeat and no expiry. It establishes exclusive
   acquisition, not liveness: a session that dies holds `issue/N` until a human
   judges it abandoned and re-runs the command with `--resume`. That
   admits in continue mode only while the `issue/N` claim exists; with no
   claim to resume it is a fresh start and needs `status:ready`.
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
   `in-progress` to `in-review` with `rtk python3 scripts/sdlc.py mark-in-review N`.
8. **Independent review.** A fresh Codex agent reviews the committed PR diff.
   The implementer may not impersonate this reviewer. Findings are fixed or
   explicitly resolved, then the updated head is reviewed again. The reviewer
   records a SHA-bound receipt with `rtk python3 scripts/sdlc.py review-receipt`.

   Independence is between **agent sessions**, not GitHub accounts: one operator
   drives several agents from one account, so equal logins are not evidence that
   the same context reviewed its own work. The gate compares the receipt's
   `--reviewer` against the head commit's `Agent-Session:` trailer, and treats a
   receipt from a different GitHub account as independent without consulting
   either. The reviewer string is self-declared. It prevents a session from
   grading its own work; it is not an anti-fraud control, and the durable
   guarantee is that a human can read the receipt and the reviewer's session log.

   `review-receipt` accepts an optional `--findings` JSON list of
   `{"id", "blocking", "disposition", "evidence"}` entries — `disposition`
   one of `open`, `fixed`, `resolved`; `evidence` required once it is not
   `open`. This is a separate, optional channel alongside the free-form
   numbered findings already in the report body, not a replacement for them.
   `merge`/`merge-gate` rebuild the cumulative finding set for a required
   review kind across **every** head SHA the PR has carried in its repair
   sequence — the same cross-head scope the round budget below reads — and
   block while any `blocking` finding still lacks a `fixed`/`resolved`
   disposition from some receipt of that kind, naming the id(s) and the
   receipt that last touched each. A finding raised in one round therefore
   survives a later round's clean `pass`, on the same head or a rebased one,
   until a receipt explicitly closes it with evidence. A receipt recorded
   without `--findings` carries none and is unaffected either way.

   `.agents/skills/codex-review/SKILL.md` (#116) wires a real reviewer to
   this: section 2 has the reviewer run `python3 scripts/sdlc.py findings
   --pr P --kind codex` (a new, read-only command) to see every id a prior
   round left open, and report an optional `FINDINGS: <json>` line — the
   last non-blank line before the trailing `VERDICT:` line — that section 3
   parses via `codex-envelope --field findings` and section 4 passes straight
   to `review-receipt --findings`. A reviewer that renumbers a still-open id
   instead of reusing it is not caught by any of this; the `findings` command
   only gives it the means to avoid that. Recording a disposition change is
   not independently verified beyond the same account/reviewer checks already
   applied to the verdict itself — self-attesting, like the mechanisms
   above, pending the R6 trusted-execution-boundary work.
9. **Merge automatically.** Run `rtk python3 scripts/sdlc.py merge --pr P --apply`. It
   refuses to merge unless the PR closes one slice, required CI is green, the
   branch is current and conflict-free, and every required review has a clean
   receipt for the exact head SHA. The merge command retains that evaluated SHA
   and submits it with `gh pr merge --match-head-commit`; a moved head is refused
   without an unguarded fallback. After every submission, including a command
   error or lost response, it reads GitHub to verify the merged PR, candidate,
   target branch, and resulting commit identity. For the configured merge-commit
   method it also verifies the candidate is the second parent. An unknown or
   pending outcome is blocked, with no automatic retry. Inspect GitHub before
   retrying; a command error can occur after a successful remote merge.

   The gate reloads the linked issue's parent epic and its gate records and
   applies the shared admission predicate in continue mode, so a pause, a
   `status:blocked` label or a closure recorded after the claim blocks the
   merge. An unknown or unreadable parent blocks. Native prerequisites are
   reloaded the same way, so one reopened after the claim blocks the merge. A
   race remains: GitHub cannot atomically bind an epic-label or prerequisite
   update to a PR merge, so a change between evaluation and `gh pr merge` is
   not seen. The expected-head precondition
   protects the candidate, not cross-object gates.
   There is no routine human PR gate and no
   size-based exception to the requested auto-merge policy.

   If a PR falls `BEHIND` because another slice merged first, rebase onto the
   fetched remote default branch and push, and re-run review/receipt for the
   new head (step 8):

   ```bash
   rtk proxy git fetch origin main
   rtk proxy git rebase origin/main
   rtk proxy git push --force-with-lease origin issue/N
   ```

   Do not run `gh pr update-branch`: it writes a GitHub-authored merge commit
   over the slice commit, and that merge commit carries no `Agent-Session:`
   trailer of its own. On such a head, a receipt from the PR author's account
   can never establish independence, and whatever review error the gate
   reports for it also names this rebase as the fix — a missing receipt, a
   failing one, or one the gate cannot otherwise resolve an identity for.
   Parent count never changes *whether* the gate blocks, only whether the
   error also names rebase. A different-account receipt's authorization is
   unaffected — it already established independence without a trailer
   before this existed — but if it fails for its own reasons, its error
   gains the same rebase note as an informational aside: that fact holds
   regardless of which account's receipt is being evaluated.

   Each required review `kind` also carries a finite round budget
   (`review.max_rounds` in `.sdlc/config.json`, default 3, per the adopted
   plan's R5): the count of every schema-1 review receipt of that kind ever
   recorded for the PR, across every head SHA it has carried in this repair
   sequence — not just the current head. Rebasing itself posts no receipt, so
   it does not consume a round by itself — but the re-review a `BEHIND`
   rebase requires under step 9 above does, exactly like any other round,
   once it is recorded. The count is reconstructed from GitHub on every
   read, so restarting a session or a fresh `continue-plan` invocation
   cannot reset it. Once consumed rounds
   exceed the effective budget, `merge`/`merge-gate` block — even a round
   whose own verdict is `pass` — until a human records an explicit renewal:

   ```bash
   rtk python3 scripts/sdlc.py renew-review-budget --pr P --kind codex \
     --rounds N --decided-by <human> --note-file <path> --apply
   ```

   Like a gate decision, an agent records only a renewal a human actually
   gave it; `--decided-by` is attribution, not proof, exactly as `set-gate`
   documents for its own decisions. The effective budget is the maximum of
   the config default and every structurally valid renewal recorded for that
   PR/kind — folded by maximum rather than "latest wins", so it is order-
   independent and a renewal can only raise the ceiling, never lower it. A
   malformed renewal (missing `decided_by`, or a non-positive or boolean
   `rounds`) is simply excluded from that fold. The count is scoped to one
   PR: closing it and opening a new one for the same issue resets consumed
   rounds to zero — a known, documented limitation, not a claim that every
   restart path is closed. Renewal and receipt comments are read from anyone
   who can comment on this public repository; the same trust limit already
   applies to review receipts and gate records, and is unresolved until R6.
10. **Reconcile.** Confirm the issue closed, then run
    `rtk python3 scripts/sdlc.py cleanup N` from the primary checkout. It owns
    the closing transition: it removes whichever of `in-progress` and
    `in-review` the closed issue still carries (other labels are left alone),
    removes the worktree (never forced, so local changes block it), prunes,
    and deletes the local branch only when `origin/main` contains it — a
    stale local `main` does not matter. A branch with commits not on
    `origin/main` is kept and reported as blocked. It regenerates the project
    cockpit **last**, so the view never shows the stale label. Rerunning it
    after a partial failure edits nothing already fixed. Update the parent
    epic checklist when GitHub did not do so automatically.

Any new commit invalidates prior review receipts because the recorded SHA no
longer matches. A review failure, missing tool, missing CI result, ambiguous issue
link, or merge-state uncertainty fails closed.

## Human milestone gate

Agents choose a checkpoint when merged slices form a coherent user-visible or
architecturally meaningful result—not after an arbitrary count. At that point:

1. Add `gate:human-review` to the epic with `set-gate --state review`. Normal
   slices under that epic pause.
2. Create `docs/milestones/epic-N/<checkpoint>.json` from the manifest template.
   The narrative is agent-authored; the included issue numbers and demo steps
   must be concrete.
3. Render it with `rtk python3 scripts/sdlc.py milestone-report ...`. The renderer
   verifies that every included slice is closed and belongs to the epic, then
   creates the HTML a human reviews.
4. Link the report and demo from the epic issue. The project cockpit shows the
   epic in its Human review lane.
5. Record the human response on the epic:
   - approval (`set-gate --state approved --decided-by <human>`) replaces the
     pause with `gate:approved` so the next checkpoint may proceed;
   - feedback (`set-gate --state changes-requested --decided-by <human>`)
     replaces it with `gate:changes-requested`, and each linked feedback slice
     cites the printed `GATE_RECORD=` URL. Ordinary roadmap work stays paused.
6. After feedback slices merge, regenerate the report/demo and request review
   again. Keep the prior report in git as checkpoint history.

Human approval is about the integrated outcome, product direction, and demo.
The individual implementation PRs remain agent-reviewed and automatically
merged.

### Gate admission

Gate labels are mutually exclusive, and `set-gate` owns every transition.
An epic with more than one gate label blocks all of its work until the actual
human decision is recorded with `set-gate`. Each transition posts a gate record
comment — state, checkpoint reference, the human who decided, and their words —
and prints its URL as `GATE_RECORD=`. It writes in a fail-closed order: it adds
the new label before removing old ones, so an interrupted edit leaves
conflicting labels rather than none; `review` applies its label before its
record; a decision posts its record before relaxing the label. A failed
command exits non-zero and never leaves the epic less restricted than it was
before the command began, and a decision never relaxes the pause without its
record. It cannot pause an epic whose label write fails: a `review` whose first
write fails leaves the prior, unpaused state, and must be retried.

| Epic gate | Ordinary slice | Feedback citing the current decision | Other feedback |
| --- | --- | --- | --- |
| none or `gate:approved` | proceeds | proceeds | proceeds |
| `gate:human-review` | paused | paused | paused |
| `gate:changes-requested` | paused | proceeds | paused |
| conflicting labels, or parent unreadable | blocked | blocked | blocked |

"The current decision" is the epic's newest gate record, which must be a
complete schema-1 `changes-requested` record (non-empty checkpoint and
`decided_by`, as `set-gate` writes it). An incomplete or legacy newest record
authorizes nothing; a feedback issue cites it with exactly one
`Feedback decision: <URL>` line. The same predicate runs at `next`,
`check-slice`, `claim` and `merge`, and unreadable gate records block even an
ungated epic. An offline `check-slice --input` bundle supplies them as
`"gate_comments": {"<epic>": [<issue comments>]}`; without them it blocks.

A decision answers the checkpoint actually under review: `approved` and
`changes-requested` require the epic's newest record to be a `review` record
with the same `--checkpoint`. If that record is missing — for example its post
failed, or the epic predates R2 — run `set-gate --state review` again first.

`--decided-by` is attribution, not proof. With one GitHub account, the tool
cannot tell a human's decision from an agent's claim of one. An agent records
only a decision a human actually gave it; silence or an individual PR approval
is never a decision. Stronger identity is the R6 trust-boundary work.

A decision answers a checkpoint under review, so more feedback after a
`changes-requested` decision means requesting review again and recording a new
decision. Open feedback issues that cite the earlier record are then blocked,
at claim and at merge, until they cite the new one.

## Visibility

Run:

```bash
rtk python3 scripts/sdlc.py status
```

This writes `docs/project-status.html` from live GitHub state. The file is
**untracked and deliberately not committed**: it is a local, on-demand view, not
a repository artifact. `--json` prints the same projection for tooling.

It shows:

- work in progress, in agent review, blocked, or at a human gate;
- progress and child slices for each epic;
- open work that is not mapped to an epic;
- recently completed work and planning-data warnings.

GitHub remains authoritative; the HTML is a dated snapshot you regenerate when
you want to look at it. Nothing depends on its freshness, because nothing stores
it — a committed snapshot on a protected branch would cost a pull request and a
full CI run per refresh, so it would drift and then mislead.

This view is **interim**. It stands in for a real delivery cockpit, and it is
deliberately cheap so that replacing it costs nothing. Treat `--json` as the
stable surface: it is the projection an application would consume, while the
generated page is the throwaway.

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
