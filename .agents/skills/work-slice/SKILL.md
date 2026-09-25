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
2. Capture the primary checkout before going anywhere — step 10 has to return
   to it — then run `rtk python3 scripts/sdlc.py claim N` and capture
   `WORKTREE=...`:

   ```bash
   PRIMARY=$(rtk proxy git rev-parse --show-toplevel)
   ```

   Perform all edits, commits, and tests in that worktree. Diff, log, and
   rebase against the printed `BASE=origin/main`, never local `main`: claim
   does not move it. Use `--resume` only after
   confirming the existing `issue/N` branch is abandoned or belongs to this
   resumed session.
3. Write a compact plan tied to acceptance criteria. If running in Claude Code
   and advisor is available, ask it to review the plan before editing.
4. Implement only the slice. If required work no longer fits one session,
   preserve the coherent portion, update the issue, and create follow-up slices.
5. Run focused tests while iterating, then
   `rtk scripts/quality-gate.sh <WORKTREE>`. Commit and push `issue/N`.

   Every agent commit records the implementing agent in a trailer:

   ```text
   Agent-Session: <agent/model> / <session id or url>
   ```

   That is the repository-wide rule in `AGENTS.md`. The merge gate enforces it
   on the **head commit** specifically: it compares that commit's trailer
   against the reviewer named in the review receipt, which is how independence
   is established when one GitHub account authors both the pull request and the
   receipt. A head commit without the trailer is refused at step 9, and amending
   afterwards changes the SHA and invalidates any receipt already recorded.
6. Self-review the committed diff. In Claude Code, request an advisor diff review
   when available. Fix findings and rerun affected validation.
7. Open a non-draft PR using the repository template. It must contain exactly
   one closing reference: `Closes #N`. Capture the PR number and its head SHA —
   later steps need both:

   ```bash
   rtk proxy gh pr create --base main --head "issue/N" --body-file <pr-body>
   PR=$(rtk proxy gh pr view --json number -q .number)
   ```

   Capture `$HEAD_SHA` with the procedure in
   [`codex-review`](../codex-review/SKILL.md) section 5, which owns it. Reading
   `headRefOid` once can return a head GitHub has not settled on yet.

   Then move the issue from `in-progress` to `in-review`:

   ```bash
   rtk python3 scripts/sdlc.py mark-in-review N
   ```
8. Have a fresh Codex agent/session review the committed PR diff, following
   [`.agents/skills/codex-review/SKILL.md`](../codex-review/SKILL.md). That file
   owns the invocation, the prompt contract, the reviewer-identity derivation,
   and the receipt.

   Two rules this step cannot delegate:

   - The reviewer must not be this session, and this session must not author the
     verdict. It runs the command and derives the verdict from the run's own
     output; those are not the same thing.
   - Every round's verdict is recorded when obtained, `fail` included. A failing
     receipt is a veto the gate honours and the next reviewer reads.

   Supply the skill with `$PR`, the issue number, `$HEAD_SHA`, and the verdicts
   of any previous rounds. It owns what happens after a fix commit.

9. Wait for CI, then run `rtk python3 scripts/sdlc.py merge --pr "$PR" --apply`.
   Treat every `BLOCKED` result as authoritative; do not bypass the gate or ask
   for routine human review.

   If the gate reports `BEHIND` because another pull request merged first,
   rebase onto the fetched remote default branch:

   ```bash
   rtk proxy git fetch origin main
   rtk proxy git rebase origin/main
   rtk proxy git push --force-with-lease origin "issue/N"
   ```

   Rebase onto `origin/main`, not a local `main` that may itself be stale. Do
   **not** use `gh pr update-branch`: it writes a GitHub-authored merge commit
   carrying no `Agent-Session:` trailer, which the gate then refuses.

   Either way the head SHA changes, which invalidates the review. Follow
   [`codex-review`](../codex-review/SKILL.md) section 5 — it owns head changes
   for rebases exactly as for fix commits — then run step 8 again.
10. GitHub closes the issue on merge but leaves its `in-review` label.
    `cleanup` clears the delivery labels, removes the worktree and local
    branch, and regenerates status last, so the view never shows the stale
    label. Run it from `$PRIMARY` — the checkout captured in step 2 — and not
    from the slice worktree:

    ```bash
    cd "$PRIMARY" && rtk python3 scripts/sdlc.py cleanup N
    ```

    It is safe to rerun after a partial failure.

    `cleanup` resolves the worktree relative to the checkout containing the
    `sdlc.py` that ran, not the current directory. Invoked from the slice
    worktree it looks for a nonexistent nested `.worktrees/issue-N` inside
    itself, then tries to delete the branch that worktree has checked out, and
    fails.

    Then report the merged PR, issue closure, validation, and any follow-up
    issue created.


Never let the implementer manufacture the independent review receipt. If a
fresh reviewer cannot be run, leave the PR in review and report that specific
blocker.
