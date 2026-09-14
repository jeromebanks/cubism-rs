# Nightshift — Roadmap, risks, and architectural decisions

[Overview](index.html) · [Document index](README.md) · [Previous](07-reliability-security-economics.md)

> Revised design proposal · 2026-09-13 · GitHub-native, single-dispatcher profile. No Nightshift implementation is included.

## 19. Smallest useful roadmap

These are proposed documentation-stage slices, not created issues or implemented features. Repair the existing Cubism SDLC incrementally and demonstrate one dependency-aware milestone before extracting a broader product. No stage automatically advances to hosted infrastructure.

### Stage 0 — Qualify and repair the existing boundary

| Proposed slice | Done condition |
|---|---|
| 0.1 Trusted invocation and merge repair | Known command defect fixed; exact-head merge and trusted policy origin enforced |
| 0.2 Independent receipt enforcement | Forged author/role, stale SHA, pass-then-fail and same-name wrong-issuer checks fail closed |
| 0.3 Readiness/gate correctness | Native dependencies, complete reads and linked-feedback exception checked at start and effects |
| 0.4 One local dispatcher | Duplicate same-host start rejected; no branch-as-claim; no executor access to integration credentials |

Demonstrate one bounded Cubism correction with existing contracts/CI and separate review. Do not introduce a database or rewrite Cubism code as a prerequisite. Live credential isolation and repository protections must qualify before unattended merge; their absence blocks that deployment capability, not documentation or adapter development.

### Stage 1 — One complete milestone with recoverable attempts

| Proposed slice | Done condition |
|---|---|
| 1.1 Graph and journal | One parent milestone, dependency-aware slices, compact allocation-before-launch comments; no attempt issues |
| 1.2 Executor adapter | Start/observe/permission/interrupt/result/usage and supported native resume validated with stable references |
| 1.3 Review and repair | Multiple attempts per slice, exact-subject structured verdicts, carried findings, finite total/review limits |
| 1.4 Broker reconciliation | Restart after launch, publication and merge response loss; recover or block without blind protected-effect duplication |
| 1.5 Integrated acceptance | Integration slice verifies assembled source/demo; human decision binds exact checkpoint; missing evidence blocks |

Demonstration: run a small DAG containing a dependency, force one implementation failure and one review repair, restart during an incomplete attempt, verify integration and accept the milestone. Include a pre-commit failure and an ambiguous protected effect. Account for known and unknown spend. Serial execution is sufficient.

Exit: routine recovery is understandable from GitHub/CLI, stale executors cannot obtain new effects, and the milestone completes within configured bounds. Pause if forged review, unknown merge treated as success, missing gate enforcement or repeated operator repair appears. The tool must reduce the work needed to develop Cubism.

### Stage 2 — Measure value and add only demonstrated capabilities

Add a second qualified executor if cross-executor choice saves time/cost; Droid is optional. Add asynchronous Cubism export and summary backfill; demonstrate that disconnected analytics cannot block any transition. Measure retry/review cost and human interruptions before tuning policy. Introduce bounded parallel attempts within the one dispatcher only if serial execution is the measured bottleneck.

Optional later branches of the roadmap are the Beads local profile, concurrent graph-write profile, stronger retention and already-owned Factory session/compute integration. Each needs its own measurable benefit, adapter qualification and explicit authority migration. PostgreSQL/Temporal, distributed fencing and enterprise services are not the next default stage.

## 20. Decisions, tradeoffs, risks and open questions

| Decision / tradeoff | Consequence |
|---|---|
| GitHub is graph and compact journal | No new persistent service; API availability, mutable comments and non-atomic multi-call updates |
| One dispatcher, serial attempts first | Simple ownership and limits; no multi-host availability or shared transactional claims |
| Native session ownership | Less code/state duplication; executor loss or retention may prevent resumption |
| In-process trusted broker | Small deployable surface; operator/host and broker credential isolation remain trusted |
| Native commit-bound evidence | Existing CI/GitHub UI reused; raw evidence can expire and checks cannot cover pre-commit failures |
| Separate review plus finite bounds | Preserves independent challenge while preventing unlimited review loops; still correlated model errors |
| Async Cubism | No circular correctness dependency; detailed analytics may have gaps |

### Implementation blockers versus qualification work

No unresolved product/architecture choice blocks beginning the GitHub-native implementation. Defaults are solo/small-team Cubism dogfood, one repository/dispatcher, serial attempts, one independent reviewer, merge plus milestone acceptance, and releases/deployments disabled.

The following are concrete implementation qualification gates, not reasons to provision infrastructure now:

- Verify the chosen executor can isolate broker credentials and expose stable result/session identity, interruption and the needed permission path. Unsupported optional resume/fork is reported, not simulated.
- Verify installed GitHub CLI/API capability, token permissions, trusted check issuers, branch protections and integration mode for this repository. These live permissions/protections were not established by the documentation inspection.
- Choose finite spend limits and approved models in trusted configuration; qualify usage deltas and unknown-cost handling. No provider/account pricing or availability is assumed.

Later questions do not block the initial profile: which offline Beads journal adapter to use, whether VFS is worth its platform/retention cost, whether Factory enterprise APIs are entitled, and which measured scale/safety requirement would justify distributed coordination.

### Do not build now

No generic hosted factory, tenant hierarchy, replacement source host/CI/tracker, transcript database, distributed scheduler, general event-sourcing engine, permanent artifact service, custom authentication/signing service, commercial billing system or rich operations cockpit. Do not create issues for each retry or Dolt branches for attempts. Do not silently weaken policy when routing or review fails. Do not claim exactly-once inference or cross-system atomicity.

## 21. Architectural decisions

These entries supersede the old same-numbered recommendations in this design; Git history preserves their prior rationale. They are design decisions, not claims of deployed enforcement.

| ADR | Revised decision |
|---|---|
| ADR-001 | Exactly one work-graph authority; GitHub Issues initially |
| ADR-002 | Pin attempt inputs and acceptance checkpoints; preserve explicit scope/replanning summaries without claiming immutable comment history |
| ADR-003 | Remove mandatory PostgreSQL and initial SQLite; PostgreSQL only at explicit future distributed safety/scale triggers |
| ADR-004 | A foreground poll/reconcile loop is sufficient; Temporal deferred until demonstrated need |
| ADR-005 | One dispatcher and process-local capabilities fence new effects; cross-host ownership is unsupported initially |
| ADR-006 | Agents have no integration credentials; trusted in-process broker performs protected GitHub mutations |
| ADR-007 | Separate assigned reviewer session and structured exact-subject verdict; model diversity is an optional policy constraint |
| ADR-008 | Blocking findings survive retries and require explicit supported disposition |
| ADR-009 | Trusted/default-branch policy and broker code govern candidate changes; candidate cannot approve weaker policy for itself |
| ADR-010 | Native checks, exact-head merge, qualified integration enforcement and reconciliation establish merge facts |
| ADR-011 | Merge, release, deployment and acceptance are distinct; multi-repository coordination deferred |
| ADR-012 | Durable compact summaries plus native references; permanent artifacts/attestations deferred until retention requires them |
| ADR-013 | Authorized human GitHub decision binds exact milestone checkpoint and explicit next permissions |
| ADR-014 | Pause stops admission, then settles outstanding effects; cancellation is not rollback |
| ADR-015 | Existing qualified executor sandbox/session infrastructure first; no hosted runner fleet required |
| ADR-016 | Cubism and optional OTel are asynchronous, non-authoritative and nonblocking |
| ADR-017 | Bound attempts/time and conservative spend admission; include failed work and uncertainty, not transactional global budget promises |
| ADR-018 | Analytics can propose improvements; never silently change policy or intended scope |
| ADR-019 | Restart reconciles comments, native sessions, Git/PR/CI and effect IDs; unknown outcomes block |
| ADR-020 | Incremental Cubism milestone demonstration precedes extraction or new infrastructure |
| ADR-021 | Beads with embedded Dolt is optional later; server mode only for genuine concurrent graph writes |
| ADR-022 | Factory remains an executor adapter; Sessions/Computers may replace remote execution infrastructure, Missions never owns Nightshift orchestration |

### Documentation verification procedure

Review both Markdown and HTML, including the hand-maintained overview. Rebuild reading editions and check reproducibility/local links with `python scripts/render-nightshift-docs.py --check` in an environment containing the pinned Markdown dependency. Run `git diff --check` and inspect the complete diff. Use a fresh independent reviewer as required by repository instructions.

Search all design files, case-insensitively, for `PostgreSQL|Temporal|SQLite|lease|transaction|inbox|outbox|object stor|event.sourc`. Remaining occurrences must be explicit deletions, acknowledged limits, or scoped future-profile discussion. Search also for `attempt`, `claim`, `policy`, `retention` and `Cubism` to check authority, lifecycle and telemetry consistency. Check state tables against diagrams and verify external primary-source links. None of these documentation checks qualifies a real executor or live GitHub protection setup.

---

[Overview](index.html) · [Document index](README.md) · [Previous](07-reliability-security-economics.md)
