---
name: plan-epic
description: Decompose a Cubism epic or broad backlog issue into ordered, one-session delivery slices and human checkpoints. Use when an Epic has no executable children, an issue is marked needs-slicing, or the next milestone needs planning. Do not implement the resulting slices in the same session.
---

# Plan an epic into executable slices

Read `SDLC.md`, the epic, and only the code/docs needed to understand current
state. Planning and implementation are separate sessions so the planner does not
consume the implementer's context budget.

1. Restate the epic as observable outcomes and identify dependencies, risky
   decisions, and candidate demonstrations.
2. Propose vertical slices. Each slice must deliver independently testable
   behavior, include tests/docs needed for its own Definition of Done, and fit
   one implementation/review session. Split infrastructure by usable seams, not
   arbitrary file or layer boundaries.
3. Order slices by dependency. Add `status:ready` only when dependencies are
   already satisfied. Mark future slices without `status:ready`; use
   `status:blocked` only for a real external or prerequisite blocker.
4. Place human checkpoints after coherent integrated outcomes. State what report
   and demo a human will receive; do not use a fixed slice count.
5. Create each issue with `.github/ISSUE_TEMPLATE/slice.yml`. Include exactly
   one `Parent epic: #N`, exact acceptance/validation, explicit non-goals, demo
   impact, and the minimum `Context` links.
6. Run `rtk python3 scripts/sdlc.py check-slice CHILD` for each ready child.
   Correct every failure before leaving it ready.
7. Update the epic's child checklist and checkpoint plan. Remove `needs-slicing`
   only when at least one validated ready slice exists and the next checkpoint
   has a clear outcome.
8. Run `rtk python3 scripts/sdlc.py status` and report the newly ready slice,
   dependency order, and next human checkpoint.

Do not implement a child in this planning session. This keeps each subsequent
agent context focused and gives the roadmap an auditable boundary.
