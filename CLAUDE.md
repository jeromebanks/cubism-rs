# Claude Code instructions

Read [`AGENTS.md`](AGENTS.md) and [`SDLC.md`](SDLC.md). Use the repo skills in
`.claude/skills/`; they are thin Claude entry points into the same portable
workflow used by Codex and other agents.

To continue the Cubism-to-Nightshift plan, use `/continue-plan`, backed by
[`.claude/skills/continue-plan/SKILL.md`](.claude/skills/continue-plan/SKILL.md).
The plain-language continuation prompt in [`AGENTS.md`](AGENTS.md) works when
shorthand discovery is unavailable.

For a slice, ask the advisor to review the written plan before implementation
and the committed diff before the PR when advisor is available. Advisor review
improves the work but does not replace the required fresh Codex review receipt.

The former `.claude/skills/timeseries-slice/` process is retired. Future
time-series work uses `work-slice` like every other subsystem.
