# Agent instructions

Read [`SDLC.md`](SDLC.md) before nontrivial work. GitHub epics define long-term
outcomes; only validated slice issues are executable work.

## Shell usage

Prefix shell commands with `rtk`. In command chains, prefix each segment. Use
`rtk proxy <command>` when raw output is needed for debugging.

## Delivery rules

- Use `$work-slice N` (or the equivalent Claude skill) for implementation.
- Do not implement an `Epic:` issue directly or expand a slice silently.
- Work only in the isolated worktree printed by `scripts/sdlc.py claim`.
- Preserve unrelated worktree changes.
- Required review comes from a fresh agent. A self-review is not an independent
  review receipt.
- Every agent commit records the session that wrote it with an
  `Agent-Session: <agent/session>` trailer. When the receipt and the pull
  request share a GitHub account — the normal case here — the merge gate
  compares that trailer to the reviewer named in the receipt, so a commit
  without one cannot prove the review came from a different agent and fails
  closed. A receipt from a different account establishes independence on its
  own and is accepted without consulting trailers. `Claude-Session:` is
  accepted as an equivalent for existing history.
- If a PR goes `BEHIND`, rebase onto the fetched remote default branch and
  push; never run `gh pr update-branch`. It writes a GitHub-authored merge
  commit with no `Agent-Session:` trailer of its own, which the merge gate
  refuses (see `SDLC.md` step 9).
- Do not ask for routine human PR review. When CI and the required agent review
  pass, use the deterministic merge command in `SDLC.md`.
- Stop ordinary work under an epic labeled `gate:human-review` or
  `gate:changes-requested`. Only feedback slices citing the current decision
  (`Feedback decision: <gate record URL>`) may proceed during the latter state.
  Record gate transitions only with `scripts/sdlc.py set-gate`, and only for a
  decision a human actually made.
- Record durable decisions and feedback in GitHub or versioned docs, not a
  transient session handoff.

## Orientation by task

- Continue the Cubism-to-Nightshift plan: [`.agents/skills/continue-plan/SKILL.md`](.agents/skills/continue-plan/SKILL.md)
  (`$continue-plan` in Codex). If shorthand is unavailable, say:
  “Continue the Cubism-to-Nightshift plan. Complete the next eligible slice
  through validation, independent review, and merge under repository policy.
  Reconcile existing work first. End with the exact next step.”

- Current project state: run `rtk python3 scripts/sdlc.py status` (writes an
  untracked local page) or `status --json` for the projection.
- Issue implementation: [`.agents/skills/work-slice/SKILL.md`](.agents/skills/work-slice/SKILL.md).
- Epic decomposition: [`.agents/skills/plan-epic/SKILL.md`](.agents/skills/plan-epic/SKILL.md).
- Milestone checkpoint: [`.agents/skills/review-milestone/SKILL.md`](.agents/skills/review-milestone/SKILL.md).
- Status refresh: [`.agents/skills/project-status/SKILL.md`](.agents/skills/project-status/SKILL.md).
- Historical time-series evidence: load only a document explicitly linked by the
  current issue.
