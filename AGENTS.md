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
- Do not ask for routine human PR review. When CI and the required agent review
  pass, use the deterministic merge command in `SDLC.md`.
- Stop ordinary work under an epic labeled `gate:human-review` or
  `gate:changes-requested`. Only linked feedback slices may proceed during the
  latter state.
- Record durable decisions and feedback in GitHub or versioned docs, not a
  transient session handoff.

## Orientation by task

- Current project state: generate/open [`docs/project-status.html`](docs/project-status.html).
- Issue implementation: [`.agents/skills/work-slice/SKILL.md`](.agents/skills/work-slice/SKILL.md).
- Epic decomposition: [`.agents/skills/plan-epic/SKILL.md`](.agents/skills/plan-epic/SKILL.md).
- Milestone checkpoint: [`.agents/skills/review-milestone/SKILL.md`](.agents/skills/review-milestone/SKILL.md).
- Status refresh: [`.agents/skills/project-status/SKILL.md`](.agents/skills/project-status/SKILL.md).
- Historical time-series evidence: load only a document explicitly linked by the
  current issue.
