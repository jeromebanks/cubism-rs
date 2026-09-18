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

   Perform all edits, commits, and tests in that worktree. Use `--resume` only after
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
   HEAD_SHA=$(rtk proxy gh pr view "$PR" --json headRefOid -q .headRefOid)
   ```

   Then move the issue from `in-progress` to `in-review`. No `sdlc.py` command
   owns this transition yet, so run it directly:

   ```bash
   rtk proxy gh issue edit N --add-label in-review --remove-label in-progress
   ```
8. Have a fresh Codex agent/session review the committed PR diff. It must not
   be this session, and this session must not write its verdict.

   ```bash
   REPORT="${TMPDIR:-/tmp}/review-pr$PR.md"
   codex exec --sandbox read-only --skip-git-repo-check "<review prompt>" \
     < /dev/null > "$REPORT" 2> "$REPORT.err" \
     || { echo "review did not run; see $REPORT.err"; exit 1; }
   test -s "$REPORT" || { echo "empty report; see $REPORT.err"; exit 1; }
   ```

   All three redirections matter.

   `< /dev/null` is required, not tidiness. `codex exec` appends stdin to the
   positional prompt, so whenever the caller leaves stdin open rather than
   closing it or attaching a terminal, the command blocks indefinitely on
   `Reading additional input from stdin...` and produces nothing.

   The two output streams go to separate files. `codex exec` writes progress,
   configuration and token counts to stderr, so folding it into stdout corrupts
   the report the receipt records; discarding it instead turns a failed run into
   a silent pass. Stdout alone is exactly the review.

   Those two guards are not decoration. Without them a review that never ran
   is indistinguishable from one that passed.

   **Bind the review to the commit.** Two guards at opposite ends of the step,
   both required.

   At the reviewer's end: put `$HEAD_SHA` in the prompt and require the reviewer
   to confirm `HEAD` matches it and stop if it does not. That keeps a reviewer
   from reporting on a tree other than the one under review.

   At the recording end: pass `--expect-sha "$HEAD_SHA"` to `review-receipt`.
   It compares that against the head GitHub reports at recording time and
   refuses — non-zero, posting nothing — when they differ. That comparison used
   to be a manual `test` a careless session could skip, which is exactly the
   session that needed it; it is now in the tool, so there is no manual check to
   run here.

   A refusal means the head moved during the review: the report describes a
   commit that is no longer this branch's head, and the review must be run again
   against the new head.

   The prompt should tell the reviewer to obtain the diff and the issue itself
   (`git show HEAD`, `git diff origin/main...HEAD`, `gh issue view N`) rather
   than
   trusting anything this session pastes, name what to check, and require it to
   report numbered findings citing `file:line` and then end with exactly one
   final line, `VERDICT: pass` or `VERDICT: fail`. Findings come first; the
   verdict is the last line of the response. `review-receipt` does not parse it
   — the snippet below does, which is precisely why the contract has to be
   unambiguous.

   After any fix commit, push it and then **re-capture `HEAD_SHA` before
   re-running this step**:

   ```bash
   HEAD_SHA=$(rtk proxy gh pr view "$PR" --json headRefOid -q .headRefOid)
   ```

   Skipping that leaves the next reviewer comparing the new head against the
   old SHA and stopping, which looks like a review failure and is not one — and
   if the reviewer does not catch it, `--expect-sha` refuses the receipt.

   Both the reviewer's identity and its verdict come from the run itself, never
   from this session's judgement. Derive them, and refuse to record if either is
   missing — an empty identity string is still non-empty enough for the gate to
   accept, so check the parts, not the result:

   ```bash
   MODEL=$(sed -n 's/^model: //p' "$REPORT.err" | head -1)
   SESSION=$(sed -n 's/^session id: //p' "$REPORT.err" | head -1)
   [ -n "$MODEL" ] && [ -n "$SESSION" ] \
     || { echo "cannot identify the reviewer; refusing to record"; exit 1; }
   REVIEWER="Codex $MODEL / $SESSION"

   VERDICT=$(grep -E '^VERDICT: (pass|fail)$' "$REPORT" | tail -1 | cut -d' ' -f2)
   [ -n "$VERDICT" ] \
     || { echo "no verdict line in report; refusing to record"; exit 1; }

   rtk python3 scripts/sdlc.py review-receipt --pr "$PR" --kind codex \
     --verdict "$VERDICT" --reviewer "$REVIEWER" --body-file "$REPORT" \
     --expect-sha "$HEAD_SHA"
   ```

   Record a `fail` rather than discarding it — a failing receipt is a veto the
   gate honours, and the next round's reviewer reads it.

   A harness may also ship its own Codex review command — Claude Code's `codex`
   plugin has `/codex:review` and `/codex:adversarial-review`. Both set
   `disable-model-invocation: true`, which makes them user-invocable only, so no
   automated step may depend on one; run them as an extra pass if you like. This
   is not about SHA binding: `codex exec` reads local git state exactly as they
   do, which is why the explicit `$HEAD_SHA` check above is what actually ties a
   review to a commit. #79 tracks whether that tooling can be reused here.
   `codex:rescue` is a different job again: it delegates investigation and
   fixes, not review.
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
   carrying no `Agent-Session:` trailer, which the gate then refuses. Either way
   the head SHA changes, so re-capture `HEAD_SHA` and run step 8 again.
10. GitHub closes the issue on merge but leaves its `in-review` label, and no
    command clears it yet. Clear it **before** regenerating status, or the
    status view is built from the stale label. Then run cleanup from `$PRIMARY`
    — the checkout captured in step 2 — and not from the slice worktree:

    ```bash
    rtk proxy gh issue edit N --remove-label in-review
    cd "$PRIMARY" \
      && rtk python3 scripts/sdlc.py cleanup N \
      && rtk python3 scripts/sdlc.py status
    ```

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
