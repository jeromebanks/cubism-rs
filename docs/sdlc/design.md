# SDLC design and maintenance boundaries

## Comparison

| Concern | `postscript_interpreter` | Prior Cubism process | Current Cubism design |
|---|---|---|---|
| Unit of delivery | One GitHub issue | Time-series phase slices plus a newer issue workflow | A validated `type:slice` issue that fits one session |
| Isolation | Feature branch/worktree | Long-lived shared time-series branch; newer draft used worktrees | Stable `issue/N` remote claim plus isolated worktree |
| Plan review | Claude advisor or equivalent | Advisor-heavy | Advisor when available; plan stays proportional to the slice |
| Independent review | Fresh review, Codex when available | Periodic Codex review at phase boundaries | Fresh Codex review for every PR |
| Merge | Agent may merge after checks/review | Direct pushes in the retired lane; size bar in the newer draft | Deterministic gate; every passing slice PR auto-merges |
| Long-range planning | Stages in `INIT.md`/`ROADMAP.md` | Phase roadmap and many handoffs | GitHub epic issues and linked slices |
| Human checkpoint | Stage summaries and demos by convention | Phase-boundary review by convention | Explicit epic gate with versioned HTML report/demo |
| Progress view | Markdown issue summary | No unified visual view | Generated HTML cockpit plus GitHub links |
| Deterministic logic | Useful shell helpers, but much logic lives in a long skill | Detailed phase skill and shell snippets | Dependency-free `scripts/sdlc.py` and `scripts/quality-gate.sh` |
| Cross-harness use | Claude-local skills | Claude-local skills | Tool-agnostic `AGENTS.md`, Codex repo skills, Claude adapters |

## Sources of truth

Use one source for each kind of fact:

| Fact | Owner |
|---|---|
| Desired outcome, slice scope, acceptance, dependencies | GitHub issue |
| Current implementation and tests | Git commit / pull request |
| Required labels, reviews, checks, merge method | `.sdlc/config.json` |
| Lifecycle policy and human responsibilities | `SDLC.md` |
| Mechanical classification and gates | `scripts/sdlc.py` |
| Tool-specific invocation guidance | `.agents/skills/`, `.claude/skills/` |
| Current visual status | Generated `docs/project-status.html` |
| Human checkpoint narrative and demo | Versioned milestone manifest + generated HTML |

Do not restate roadmap state in handoff documents. If a current slice needs old
phase evidence, link that exact document from the issue's `Context` section.

## What belongs in scripts

Scripts own operations where an agent should not improvise:

- issue/epic classification and readiness checks;
- one-branch-per-issue claim mechanics;
- CI-equivalent command execution;
- exact-SHA review receipts;
- merge eligibility and merge command shape;
- live-state aggregation and HTML escaping/rendering;
- milestone-manifest structural and issue-state validation.

Agents retain judgment for architecture, implementation, review findings,
checkpoint selection, narrative quality, demo design, and turning human feedback
into appropriately scoped issues.

## Why no MCP server

The workflow needs authenticated GitHub CRUD, git, local tests, and static file
generation. `gh`, git, and deterministic scripts already provide those
capabilities with less maintenance and fewer failure modes. An MCP server would
be justified only if a future tracker has state that cannot be represented in
GitHub or accessed reliably through its CLI/API.

## Evolving the process

Change `.sdlc/config.json`, `SDLC.md`, scripts, and affected skills in the same
PR. Add or update script tests for mechanical behavior. Process changes follow
the same slice lifecycle as product work; they do not bypass their own gates.
