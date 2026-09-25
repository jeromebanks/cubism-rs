---
name: continue-plan
description: Continue the adopted Cubism-to-Nightshift plan from live GitHub state, reconciling unfinished work and completing one eligible slice through validation, independent review, and permitted merge.
---

# Continue the Cubism-to-Nightshift plan

Use this for `$continue-plan` in Codex, `/continue-plan` in Claude Code, or:

> Continue the Cubism-to-Nightshift plan. Complete the next eligible slice through validation, independent review, and merge under repository policy. Reconcile existing work first. End with the exact next step.

Default to one completed slice, including its review and repair attempts.
An already-completed item is reconciliation, not this invocation's delivery.
Honor narrower user scope and execution limits.

## Reconstruct before selecting

Read [AGENTS.md](../../../AGENTS.md), [SDLC.md](../../../SDLC.md),
[config](../../../.sdlc/config.json), and the
[adopted plan](../../../docs/sdlc/NIGHTSHIFT_EXECUTION_PLAN.md). Read live
[epic #38](https://github.com/jeromebanks/cubism-rs/issues/38), its active-stage
mapping and continuation records, then the selected issues and their parents.
GitHub owns progress; the plan owns stage order; SDLC/config own policy.
Generated status is disposable. Prior chat and a local progress database are
never prerequisites.

Inspect current branch, dirty state and worktrees; fetch the remote default
branch without advancing another checkout. For selected plan work, read issue
bodies/comments and prerequisites, open and previously closed PRs, candidate
SHAs, required checks, all review receipts, remote claim refs and parent gates.
Follow linked evidence and paginate relevant GitHub reads to completion;
inaccessible or incomplete evidence is unknown, not permission to proceed.
`status --json` is an orientation aid, not complete evidence.

Prefer reconciling an existing implementation PR, failed review or interrupted
merge over creating another branch. Verify successful completion using the
merged PR, resulting commit, required checks and SHA-bound independent review;
closure alone (including cancellation or supersession) does not satisfy a
prerequisite. Verification-only work needs its explicit acceptance evidence.
For an ambiguous merge response, read GitHub before another mutation.

Claims have no heartbeat or expiry today. A remote `issue/N` ref, old timestamp,
or absent local process does not establish abandonment. Read recorded session
ownership and inspect available local state; resume only with evidence it is
this resumed session's work or a human-confirmed abandoned claim. Never seize
an active claim. If ownership is unresolved, record contention and stop this
continuation; do not skip ahead in the dependency chain.

## Select and complete

Select the earliest unfinished item in the adopted stage order with satisfied
prerequisites and a permitting parent gate, using the real mapping on #38.
Recheck the historical findings against current code. `sdlc.py next` currently
orders ready labels, not this plan's dependency graph: never treat its output
alone as authorization. Inspect blocked/closed/superseded state and successful
prerequisite evidence even when `check-slice` passes.

If decomposition is needed, reuse equivalent issues and create only the next
bounded slice or immediate batch, using the issue contract in
[plan-epic](../plan-epic/SKILL.md). Record dependencies as native `blocked_by`
links and the actual mapping on #38. For this continuation request, proceed
into eligible implementation in the same invocation; the user's request to
continue through implementation overrides plan-epic's normal separate-session
advice. Do not decompose the whole roadmap or claim an epic. Set readiness only
after prerequisites pass.

Delegate implementation to [work-slice](../work-slice/SKILL.md) with the explicit
issue number. It owns claim/worktree isolation, validation, commits, PRs,
review/repair, deterministic merge and cleanup; its
[codex-review](../codex-review/SKILL.md) step owns reviewer invocation, identity,
verdict, current-head capture and receipts. Do not duplicate those procedures.

For an owned existing PR, resume the appropriate work-slice step in its isolated
worktree. First reconstruct the owning primary checkout from Git worktree/common
directory metadata and verify it, retaining it as work-slice's `PRIMARY` for
cleanup. A fresh session inside a slice must not mistake that slice's
`rev-parse --show-toplevel` for the primary checkout. If the primary cannot be
established, preserve work and report the recovery blocker. Read findings and
checks, repair the candidate, and obtain independent
review for the resulting SHA. Do not rerun claim for an open PR: today's claim
command rejects one. Locate the existing worktree; if it is missing, restore
an isolated worktree for the established owned branch without creating another
claim or overwriting work. Record that recovery on the issue.

Recheck live parent gates and prerequisites before implementation and merge,
even where current tooling omits them. Human-review stops all slices;
changes-requested permits only feedback linked to the actual parent decision.
Conflicting labels or tool rejection remain blockers; do not bypass the gate.
Delegate checkpoint preparation/recording to
[review-milestone](../review-milestone/SKILL.md). Stop for an actual human
decision on the assembled version; never infer approval from silence or a PR.

## Limits and durable continuation

Before review/repair, reconstruct cumulative rounds, findings and dispositions
from prior receipts/comments across all candidate SHAs in this repair sequence.
Retain stable finding IDs (assign IDs to older unnumbered findings and link the
original report), fix/resolution evidence, and failed verdicts. A new head or
session does not erase findings or replenish an exhausted recorded allowance.
Every actual verdict is recorded through codex-review, failures included.

Honor finite limits supplied by the user, runtime or adopted policy and any
persisted exhaustion. Record the limit source, consumed/remaining amounts where
known, and unknown values honestly. R5's suggested three-round default is
proposed, not currently configured or enforced. When no numerical allowance
exists, do not invent one as repository policy: record that limitation and
bound work to one slice; stop if repair cannot make concrete progress within
the available execution limits. Do not silently restart exhausted work;
report the explicit replan or renewed authorization it requires.

Stop for a human milestone decision, unavailable required reviewer/tool,
unresolved ownership/evidence, a blocking dependency, or exhausted execution
limits. Preserve the candidate and record the exact blocker. No substitute
self-verdict, weakened check, or direct merge bypass is allowed.

Maintain an identifiable `<!-- continue-plan -->` comment on the selected issue.
Update that comment when appropriate, preserving cumulative evidence links and
prior limits; do not overwrite another active session's record. Link it from
the PR and update #38 when stage/mapping/next action changes. This is a compact
GitHub summary, not a new receipt or attempt-journal implementation:

```text
Stage / plan ID / issue / PR / candidate SHA / merged SHA (if verified):
Owner session / claim / continuation status:
Evidence: validation, checks, review receipts, decisions (links):
Cumulative review/repair: rounds; finding IDs, disposition and evidence:
Limits: source; consumed / remaining / exhausted, or unknown:
Unresolved blockers:
Next action: exact issue and prerequisite, command or missing decision:
```

Write material decisions and evidence before ending, including partial failures
between merge, closure and cleanup. Reconcile labels and cleanup through
work-slice; confirm actual merge state, then update the epic mapping.
Keep progress out of repository checklists and generated status.

End with result (completed/blocked/awaiting human decision), issue/PR links,
merged or preserved candidate SHA, delivered behavior, validation actually run,
independent-review result, limitations, exact next eligible issue and why.
State required user action or “none” and give the precise continuation prompt.

## Current capability boundary

Inspect current CLI help and code when capabilities matter. Today the owners
provide `check-slice`, `claim`, `mark-in-review`, `review-receipt`,
`merge-gate`, `merge`, `cleanup`, `status`, and milestone commands. R1–R6
propose repairs for candidate
binding, gates, dependencies, reconciliation, budgets and trust. Stage N's
attempt journal, leases, executor adapters and dispatcher do not exist merely
because this skill describes continuation. Reassess this boundary as repairs
land; never promise unattended or transactional execution from B0 alone.
