# Claude Code instructions

Read [`AGENTS.md`](AGENTS.md) and [`SDLC.md`](SDLC.md). Use the repo skills in
`.claude/skills/`; they are thin Claude entry points into the same portable
workflow used by Codex and other agents.

For a slice, ask the advisor to review the written plan before implementation
and the committed diff before the PR when advisor is available. Advisor review
improves the work but does not replace the required fresh Codex review receipt.

The former `.claude/skills/timeseries-slice/` process is retired. Future
time-series work uses `work-slice` like every other subsystem.
