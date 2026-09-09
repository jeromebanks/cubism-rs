---
name: review-milestone
description: Prepare, iterate, or record a human review checkpoint for a Cubism epic, including an HTML achievement report, demo instructions, and tracked feedback slices. Use when enough slices form a coherent milestone or the human approves or comments on one.
---

# Run a milestone checkpoint

Read `SDLC.md`, the epic, its linked slices, and
`docs/sdlc/milestone-manifest.example.json`.

## Prepare review

1. Confirm the merged slices form a coherent outcome a human can evaluate. A
   raw issue count is not sufficient.
2. Write a short gate note, then apply the transition before starting more
   ordinary slices:

   ```bash
   rtk python3 scripts/sdlc.py set-gate --epic N --state review \
     --note-file <note> --apply
   ```
3. Create `docs/milestones/epic-N/<checkpoint>.json`. Explain the integrated
   outcome in product language, list important decisions and known gaps, and
   provide a runnable demo or an honest reason none is possible.
4. Render and validate it:

   ```bash
   rtk python3 scripts/sdlc.py milestone-report \
     --manifest docs/milestones/epic-N/<checkpoint>.json
   ```

5. Open the generated HTML and verify its layout and every demo step. Link the
   report from the epic and ask the human for outcome-level approval or feedback.
6. Refresh `docs/project-status.html` so the human gate is visible.

## Record feedback

Preserve the human's words in a note and run `set-gate --state
changes-requested --note-file <note> --apply`.
Turn each independently deliverable correction into a `type:feedback` issue
using the slice template and `Parent epic: #N`. Feedback slices use `$work-slice`
and may proceed while the parent is paused; unrelated roadmap slices may not.

After all feedback slices merge, revise the manifest, generate a new checkpoint
HTML file, and request review again. Keep earlier checkpoint reports.

## Record approval

Preserve the approval in a note and run `set-gate --state approved --note-file
<note> --apply`. Refresh the cockpit. Do not infer approval from silence or from
approval of an individual PR.
