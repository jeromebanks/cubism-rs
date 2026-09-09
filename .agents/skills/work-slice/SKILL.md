---
name: work-slice
description: Take one ready Cubism GitHub slice through isolated implementation, validation, independent review, and automatic merge. Use for "work issue N", "implement the next slice", or completing a bounded backlog item. Do not use on Epic issues or unsliced roadmap work.
---

# Work one delivery slice

The issue is the bounded session context. Read `AGENTS.md`, `SDLC.md`, and the
issue. Load only files linked from its `Context` section plus code directly
needed for implementation.

1. If no issue was supplied, run `rtk python3 scripts/sdlc.py next` and use the
   returned `NEXT=N`. Then run `rtk python3 scripts/sdlc.py check-slice N`. If it fails, improve or split
   the issue; do not implement it as written.
2. Run `rtk python3 scripts/sdlc.py claim N` and capture `WORKTREE=...`. Perform
   all edits, commits, and tests in that worktree. Use `--resume` only after
   confirming the existing `issue/N` branch is abandoned or belongs to this
   resumed session.
3. Write a compact plan tied to acceptance criteria. If running in Claude Code
   and advisor is available, ask it to review the plan before editing.
4. Implement only the slice. If required work no longer fits one session,
   preserve the coherent portion, update the issue, and create follow-up slices.
5. Run focused tests while iterating, then
   `rtk scripts/quality-gate.sh <WORKTREE>`. Commit and push `issue/N`.
6. Self-review the committed diff. In Claude Code, request an advisor diff review
   when available. Fix findings and rerun affected validation.
7. Open a non-draft PR using the repository template. It must contain exactly
   one closing reference: `Closes #N`. Move the issue from `in-progress` to
   `in-review`.
8. Request a fresh Codex agent/session to review the PR diff. The independent
   reviewer writes its report to a file and records it with:

   ```bash
   rtk python3 scripts/sdlc.py review-receipt --pr P --kind codex \
     --verdict pass --reviewer "<agent/session>" --body-file <report>
   ```

   Use `--verdict fail` when findings remain. After any fix commit, the new SHA
   requires a new review and receipt.
9. Wait for CI, then run `rtk python3 scripts/sdlc.py merge --pr P --apply`.
   Treat every `BLOCKED` result as authoritative; do not bypass the gate or ask
   for routine human review.
10. Run `rtk python3 scripts/sdlc.py cleanup N` and
    `rtk python3 scripts/sdlc.py status`. Report the merged PR, issue closure,
    validation, and any follow-up issue created.

Never let the implementer manufacture the independent review receipt. If a
fresh reviewer cannot be run, leave the PR in review and report that specific
blocker.
