---
name: project-status
description: Generate or explain Cubism's visual delivery cockpit from current GitHub issues and pull requests. Use when asked what is done, in flight, blocked, awaiting human review, or still unmapped.
---

# Refresh project status

Run `rtk python3 scripts/sdlc.py status`. Show the user the generated
`docs/project-status.html` and summarize only the most decision-relevant state:
active slices, human gates, blockers, epic progress, and unmapped work.

Do not manually reclassify or rewrite the generated HTML. If classification is
wrong, fix GitHub issue links/labels or `scripts/sdlc.py`, then regenerate.
GitHub is authoritative; the HTML records its generation time.
