# Cubism SDLC repair and Nightshift delivery plan

Prepared for Jerome · 19 September 2026

Adopted roadmap: [continuation entry point](../../.agents/skills/continue-plan/SKILL.md).
Live stage, issue mapping and completion evidence belong to
[epic #38](https://github.com/jeromebanks/cubism-rs/issues/38).
The reviewed baseline and bootstrap prompt below are retained as provenance;
proposed repairs and Nightshift capabilities become available only when their
implementation and validation evidence lands.

## Intended outcome

Start Claude Code or Codex in `jeromebanks/cubism-rs`, invoke one continuation skill or prompt, and have the session reconstruct progress, select the next justified action, implement a bounded slice, run validation, obtain independent review, merge when permitted, and report exactly what happens next.

The same entry point must work after a successful merge, an interrupted implementation, a failed review, or a milestone checkpoint. It must not require the operator to reconstruct the previous conversation.

**First make the existing process reliable. Then prove it on Cubism product work. Then add Nightshift's recoverable orchestration around it.** Keep GitHub as the initial durable record. No PostgreSQL, SQLite, Dolt server, Temporal, hosted control plane, or transcript database is required by this plan.

This is an implementation plan, not a claim that the proposed commands or skill already exist. Creating this document does not change the repository or file issues. The bootstrap prompt at the end authorizes a future coding session to install the plan through the existing lifecycle.

## 1. Evidence and authority

Reviewed baseline: `main` at `0adc88dc8a7ca7722c2bf54146d950d14a8cb99f`, confirmed when preparing this plan. The underlying review inspected source, PRs, and issue comments; it did not execute the test suite. Reproduce defects against current code before implementing fixes. A newer commit may already fix a finding.

Authoritative implementation inputs:

- [AGENTS.md](https://github.com/jeromebanks/cubism-rs/blob/main/AGENTS.md), [SDLC.md](https://github.com/jeromebanks/cubism-rs/blob/main/SDLC.md), and [configuration](https://github.com/jeromebanks/cubism-rs/blob/main/.sdlc/config.json).
- [SDLC implementation](https://github.com/jeromebanks/cubism-rs/blob/main/scripts/sdlc.py), its tests, and the canonical `.agents/skills/` procedures.
- [SDLC epic #38](https://github.com/jeromebanks/cubism-rs/issues/38) and actual linked PR/receipt history.
- [Revised Nightshift design](https://github.com/jeromebanks/cubism-rs/blob/docs/nightshift-design-draft/docs/nightshift-design/README.md), particularly chapters 2–8. Read from that branch without switching or disturbing an active worktree.

The transition notes under `docs/nightshift-review/` contain historical claims that tooling is absent and that a PostgreSQL kernel is next. Those claims do not describe the reviewed baseline or revised Nightshift direction. Preserve historical evidence, but label it and point readers to current policy.

Use one authority per fact:

| Fact | Authority |
| --- | --- |
| Scope, acceptance, prerequisites, current work state | GitHub issues and native relationships once supported |
| Delivered source, CI, review, merge | Git commits, PRs, checks, and receipts |
| Architecture and ordered stages | This plan after adoption into repository docs |
| Execution rules | SDLC.md, config, deterministic commands |
| Session instructions | Canonical continuation skill and existing work-slice/review skills |
| Current status page | Disposable projection; never a second ledger |

Stable IDs below such as R1 and N2 are plan references, not invented GitHub issue numbers. Record the real issue mapping on the tracking epic. Do not maintain completion checkboxes in several files.

## 2. What one invocation does

Proposed canonical skill: `.agents/skills/continue-plan/SKILL.md`, with a thin Claude adapter at `.claude/skills/continue-plan/SKILL.md`. Keep policy and shell mechanics in existing owners; the new skill coordinates them rather than copying them.

Suggested user invocation after installation:

```text
Continue the Cubism-to-Nightshift plan. Complete the next eligible slice through validation, independent review, and merge under repository policy. Reconcile existing work first. End with the exact next step.
```

Where discovered by the harness, use `$continue-plan` in Codex or `/continue-plan` in Claude Code. Skill discovery differs by installation; the plain-language prompt remains supported.

### Session algorithm

1. Read root instructions and the adopted plan; inspect current branch/worktrees and GitHub state. Preserve unrelated work. Do not reset, force-push, delete, or seize another session's work merely to resume.
2. Reconcile unfinished work belonging to this plan before choosing new work: active issue, claim, PR, latest head, checks, receipts, blockers, and epic gates. An existing claim is not proof its owner died. Resume only when ownership or abandonment is established; otherwise report contention or select independent eligible work.
3. Prefer finishing an eligible existing PR, addressing its recorded findings, or retrying a failed validation over opening another implementation branch. Do not rerun implementation if a valid candidate already exists.
4. Select the earliest dependency-satisfied slice in the current stage. If none is executable because decomposition is needed, create only the next bounded issue or small imminent batch, using the required issue sections. Planning should normally proceed into implementation in the same invocation.
5. Use the existing work-slice lifecycle: validate, claim, isolated worktree, proportional plan, implementation, focused tests, required checks, actual-diff review, independent review, receipt, deterministic merge, and reconciliation.
6. Respect finite review and runtime limits. Never make a failing receipt pass by rewriting it, weakening acceptance, inventing a reviewer, or bypassing the gate. If the reviewer is unavailable, retain the candidate and report the exact missing capability.
7. Write material decisions, evidence, unresolved findings, and next action to the existing issue/PR before ending. Give the user a concise continuation report.

Default unit: **one completed slice per invocation**, including its repair/review attempts. If a selected item is already done, reconcile it and continue to the next eligible item rather than spending the invocation reporting an old completion. Explicitly requested multi-slice runs may continue within limits, but always stop at a genuine human milestone decision.

### Required completion report

```text
Result: completed / blocked / checkpoint awaiting human decision
Work: issue URL, PR URL, merged or candidate SHA
Delivered: observable outcome in one or two sentences
Verification: checks actually run; independent review; material limitations
Remaining: unresolved findings or recovery work, if any
Next: next issue and why it is eligible, or the exact missing decision/capability
Continue with: the stable continuation prompt, or a targeted resume prompt
```

Say “none” when no user action is needed. Never call a slice complete merely because code was committed, CI was green, or its issue was closed. A checkpoint report must identify the assembled version being accepted.

## 3. Stage B — Install continuation without building a scheduler

### B0 — Adopt the plan and install the entry skill

Parent: existing epic #38. Reuse existing planning issues if equivalent work exists.

Deliverables:

- Adopt this plan as `docs/sdlc/NIGHTSHIFT_EXECUTION_PLAN.md`, preserving useful detail while aligning terminology with current code.
- Add the canonical continuation skill and Claude pointer; link them from agent orientation. Follow applicable skill-authoring instructions and persist changes through git.
- Put the real issue mapping and active stage on the existing tracking epic. Add a short historical notice to obsolete transition guidance, not a wholesale history rewrite.
- Create only the immediate repair issues R1–R3 initially. Record later slices as roadmap entries until they are about to run.
- Define a short GitHub continuation-comment format containing issue/PR/candidate, evidence links, unresolved finding IDs, remaining budget where known, and next action. Update an identifiable continuation comment when appropriate rather than posting every tool event.

Acceptance: a fresh session with no chat history can locate the plan, identify current work from GitHub, explain the next eligible slice, and follow the existing review/merge path. The skill must not introduce another JSON progress database or a private local handoff dependency.

Bootstrap constraint: the existing gate must still be respected. If a confirmed gate bug makes this documentation slice impossible to complete, retain its branch, link the blocker, and repair that defect through a separate bounded issue. Never disable the failing condition silently.

## 4. Stage R — Repair the current SDLC

These are correctness and operability repairs to the existing implementation, not a Nightshift rewrite. Keep Python and the current CLI. Each issue needs its own observable outcome, acceptance criteria, validation, demo/effect, non-goals, and minimum context.

### R1 — Bind merge to the evaluated candidate and reconcile its result

Finding: `command_merge()` evaluates a PR, then calls `gh pr merge` without retaining an expected-head constraint. Receipt creation's `--expect-sha` does not close this separate interval.

Work:

- Return structured eligibility information including the full evaluated head SHA; make execution consume that same result.
- Use the installed GitHub CLI/API's supported expected-head precondition. Verify the interface in the target environment; do not guess a flag or fall back to an unguarded merge.
- Confirm resulting PR merge state and resulting commit identity before reporting success. If a response is lost, reread GitHub before another mutation; unknown outcome is blocked, not assumed success or blindly retried.
- Preserve current required checks and review rules. Revalidate prerequisites as late as practical; document that GitHub cannot atomically transact an epic label change and a PR merge.

Acceptance/tests: a head moving after evaluation is rejected; unchanged eligible head merges; a failed or ambiguous response is reconciled; no unguarded fallback exists. Cover the race with deterministic fixtures and perform a controlled live check where permitted. Avoid modifying unrelated production work to manufacture a race.

### R2 — Repair human-gate transitions and enforce them before merge

Finding: `set-gate changes-requested` adds both human-review and changes-requested labels, while validation gives human-review precedence and blocks feedback. Merge evaluation does not reload the parent gate.

Work:

- Make gate states mutually exclusive and reconcile invalid combinations explicitly.
- During human-review, admit no slices. During changes-requested, admit only feedback slices linked to that parent and the actual feedback decision. A feedback label alone is not sufficient authority.
- Reload parent/gate information for merge evaluation as well as claim/readiness. Unknown parent or unreadable gate fails closed.
- Record the actual human decision and checkpoint reference. An agent may faithfully record a supplied decision; it must not manufacture approval.

Acceptance/tests: a table-driven transition matrix covers every gate and ordinary/feedback slice combination; the command-generated changes-requested state permits the intended feedback slice; pausing after claim blocks a subsequent merge check; unrelated feedback does not bypass the pause. Explicitly document the remaining cross-object race until Nightshift owns effect admission.

### R3 — Enforce readiness and dependencies

Finding: current validation checks issue structure and parent gates but does not resolve native blockers or consistently enforce the slice's blocked state.

Work:

- Centralize executable/readiness predicates for `next`, claim, and merge. Distinguish “can start new work” from “can continue an already-owned attempt”; in-progress work need not regain a ready label.
- Reject blocked, closed, cancelled, superseded, ambiguous, and structurally invalid work appropriately.
- Probe GitHub relationship capabilities, read complete selected-epic relationships, detect cycles and missing/inaccessible prerequisites, and evaluate successful prerequisite completion rather than closure alone.
- Initially, implementation prerequisites require merged PR plus required evidence; verification-only prerequisites need their explicit completion evidence. Cancellation cannot silently satisfy a dependency.
- Migrate one active epic from parsed parent/checklist conventions to native relationships deliberately. If native and legacy records disagree, stop and reconcile. Do not create two competing graph authorities.

Acceptance/tests: A blocks B until A is successfully completed; failed/cancelled A does not unblock B; missing pages or permissions remain unknown; cycles produce an actionable error; existing unrelated epics are not bulk-migrated. If too large for one slice, split local readiness fixes from native relationship adoption with an explicit dependency.

### R4 — Own lifecycle reconciliation and branch-update recovery

Reuse issue #82 and epic #38's label-transition work; reconcile overlap instead of duplicating it. Incorporate #75 only after inspecting its current status and any in-flight implementation.

Work:

- Make commands own transition to in-review and post-merge cleanup of delivery labels, then regenerate status after reconciliation.
- Make repeated reconciliation safe after partial failures. Do not erase unrelated labels or delete a dirty worktree.
- Standardize the comparison base on fetched remote state; do not advance a checked-out local main unsafely just to satisfy a diff convention.
- Resolve the #75 branch-update/implementer-identity problem with explicit semantics. Do not blindly trust the first parent of a GitHub-generated merge or the nearest arbitrary trailer. Verify topology and relevant implementing sessions; alternatively retain a clear tested rebase-only policy for this initial phase.
- Treat a changed candidate SHA as requiring fresh evidence under current policy. Do not optimize away necessary review in this repair.

Acceptance: after a simulated interruption between merge, issue closure, label cleanup, and status rendering, rerunning continuation repairs state without duplicate work. Claim ownership remains explicit; no timeout is invented for current branch claims.

### R5 — Reduce review overhead with deterministic mechanics and finite limits

Reuse #84 and #85 where appropriate. Their existing scopes constrain implementation until explicitly revised with rationale; do not quietly enlarge them.

Work:

- Move repeated receipt-producing shell mechanics out of long skill snippets into a tested helper when justified by existing repeated use: candidate capture, subprocess exit/report checks, exact verdict parsing, and reviewer identity acquisition.
- Keep the skill focused on judgment and sequencing. Qualify supported structured executor output; until available, isolate and test the existing stderr parser rather than spreading it into more files.
- Run the fast SDLC tests during SDLC development and clearly disclose local-gate omissions. CI remains authoritative.
- Introduce a configurable finite review budget. Suggested initial default: at most three review rounds per candidate repair sequence; exhaustion records unresolved findings and a blocked/replanning result. This is a proposed policy, to be adopted explicitly in config, not an existing guarantee.
- Give findings stable IDs with disposition and evidence. Separate source defects from stale PR narrative. A metadata-only correction may use a focused independent follow-up over the same source SHA, but must not be auto-approved by the implementer.
- Do not let fresh invocations reset an exhausted budget. Persist enough cumulative summary on GitHub to enforce it across sessions.
- Record observed review count, elapsed time, human interventions, and reported usage. Unknown cost is null/unknown, not zero; do not estimate dollars from invented prices.

Acceptance: malformed/empty reports and missing identity fail closed; a newer failure cannot be hidden by an older pass; blocking findings survive another session; budget exhaustion cannot be bypassed by restarting; focused metadata verification does not silently authorize changed source.

### R6 — Establish the trusted execution boundary

This is a prerequisite for unattended Nightshift mutations, not a demand for new infrastructure before useful supervised development.

- Load effect-authorizing gate code and policy from a recorded trusted default-branch version, not from the candidate being evaluated. Candidate tests and candidate policy changes remain reviewable but cannot authorize themselves.
- Qualify branch protections, check provenance, receipt authority, and credentials available in the actual environment. A differently named commenter or check is not automatically an authorized reviewer/check issuer.
- Document supervised mode's current self-declared agent identity limits. For unattended mode, the trusted process assigns reviewer attempts and records their result; it must not rely on arbitrary identity strings as proof.
- Keep GitHub mutation credentials out of unattended executor processes. Broker remains an in-process module of a local controller, not a network service.

Acceptance: candidate modifications cannot weaken the policy used to accept that candidate; unavailable trusted policy blocks effects; untrusted receipts/checks do not authorize merge. If credential isolation cannot be demonstrated, advertise supervised-only operation and keep unattended effects disabled.

Stage R exit: R1–R4 verified, R5's basic limits/evidence in use, and R6's trusted-policy behavior established. No claim of unattended execution until full isolation qualification succeeds. Cosmetic history moves and broad backlog grooming are not prerequisites.

## 5. Stage C — Prove delivery with a real Cubism milestone

Use [epic #40, correction CLI and operations](https://github.com/jeromebanks/cubism-rs/issues/40), unless current progress supplies a smaller equally useful product milestone. Existing #56 and #57 are candidate inputs, not automatically mandatory prerequisites; inspect the code and existing implementation first.

### C1 — Scope one useful checkpoint

Proposed outcome: demonstrate a supported source change flowing through correction execution, publication, and a query that shows the corrected result, with clear failure reporting.

Create only a small dependency-aware set of slices: necessary correction API cleanup, lateness-policy behavior if applicable, CLI/operational wiring, and explicit integration verification. Reuse existing issues. Do not make the entire correction roadmap one checkpoint.

Acceptance: concrete fixture, expected before/after behavior, exact commands, included issues, non-goals, and a reproducible assembled commit. Planning must fit the current product rather than forcing speculative architecture changes.

### C2 — Deliver implementation slices through the repaired loop

Run continuation repeatedly. Each invocation completes one slice or reports a concrete blocker. Measure human time, review rounds, executor usage where available, and wall-clock time. Keep source work and SDLC repairs separate when either would obscure review.

### C3 — Verify and accept the assembled milestone

- Add an explicit verification slice whose output is evidence, not necessarily source changes. Do not require an empty PR just to satisfy a code-oriented workflow.
- Execute the integration/demo against a recorded assembled SHA. Report failures honestly and create bounded repair work where necessary.
- Extend milestone manifest validation to distinguish successful implementation from merely closed issues, retain check/demo evidence, and bind acceptance to an exact checkpoint identity.
- Present the human with the report and demo. Approval accepts that checkpoint only; changed source or acceptance scope may require a new checkpoint.
- Exercise feedback handling if real feedback is given. Never fabricate human feedback or approval for a test; use fixtures for deterministic gate tests.

Stage C exit: one useful Cubism checkpoint accepted, evidence survives a new session, and actual process cost is recorded. If tooling costs dominate, repair the measured bottleneck before adding features. An arbitrary number of SDLC PRs is not success.

## 6. Stage N — Develop minimal Nightshift around the proven loop

Build incrementally within Cubism until extraction has evidence behind it. Begin with a small Python package/module boundary and a compatibility wrapper for `scripts/sdlc.py`; do not rewrite it in Rust merely because Cubism is Rust. Keep the CLI working at every stage.

### N1 — Extract reusable domain operations

Separate pure readiness/gate logic, GitHub reads/writes, executor adaptation, and presentation. Carry forward behavior tests. Move Cubism-specific validation commands and review choices into configuration. Existing `codex` review policy remains a valid default; the generic model uses a reviewer role plus executor configuration.

Acceptance: existing Cubism commands retain behavior; no new backend or independently deployed component; configuration expresses repository-specific concerns without copying the engine.

### N2 — Add durable attempt identities and journal comments

One slice may have multiple implementation, review, repair, and integration attempts. An attempt is an execution record, never a new work issue. A resumed native session may continue the same assignment; a changed role, candidate, or replacement execution gets a new linked attempt.

Record allocation before launch: UUID/ULID, role, slice, predecessor, pinned contract/policy/base, executor/model where known, limits, timestamps, state, candidate, session reference, outcome, usage provenance, findings/effects, and artifact references. Preserve a material contract summary, not only a digest of a mutable issue body.

Acceptance: ambiguous allocation POST is reconciled by marker before launch; duplicate/conflicting markers are detected; missing usage remains unknown; terminal attempts are not reopened as retries; complete selected-work reads are paginated. Native sessions/transcripts remain executor-owned.

### N3 — Qualify one executor adapter

Implement start, observe, interrupt, result, usage, and permission-wait behavior for one available harness first. Add native resume only when supported and tested. Use a small stable adapter contract, not a universal agent framework.

Acceptance: distinguish running, completed, failed, cancelled, waiting, and unknown; preserve native session references; zero exit alone does not mean accepted work; lack of optional resume does not masquerade as success. A second harness can still invoke the continuation skill even before Nightshift can launch it as an executor.

### N4 — Add the single-dispatcher loop and protected-effect broker

One foreground process, serial attempts initially. Reject a duplicate same-host dispatcher using a suitable process lock. Read GitHub, choose ready work, allocate attempt, launch, observe, record, review/repair, verify, and merge through the trusted broker.

The broker admits only authorized exact effects with matching candidate and policy. Executors request effects rather than holding reusable integration credentials. Pause stops new admission and settles already-admitted effects. Keep deployments/releases disabled in this initial profile.

Replace branch-as-claim ownership deliberately when dispatcher ownership is introduced: reconcile active legacy claims, preserve worktree isolation, and document which mechanism governs each mode. Do not run two independent authorities over the same work.

Acceptance: finite total/time/review bounds survive restart; human gates block admission; credential isolation passes R6 qualification; no second dispatcher begins overlapping work; analytics availability is irrelevant to correctness.

### N5 — Prove restart reconciliation

Inject failures after allocation, after launch before session recording, after candidate publication, during review, and after merge submission before response recording. Reconcile GitHub, executor state, and pending effect identity before deciding what to repeat.

Acceptance: valid candidates are reused; known-running sessions are observed rather than duplicated; ambiguity becomes inspection-required; bounded duplicate computation may occur but unknown protected effects are never blindly replayed. No claim of exactly-once inference or cross-system transactions.

### N6 — Deliver one milestone through Nightshift

Choose another small Cubism product checkpoint. Demonstrate A→B dependency, one controlled failed attempt, one review repair, restart with incomplete work, integration verification, and exact-checkpoint human acceptance. Include a pre-commit failure so the journal proves value beyond PR history.

Stage N exit: understandable recovery from GitHub/CLI, useful product delivered, bounded execution, and fewer manual orchestration steps. Serial operation is sufficient.

## 7. Stage E — Extract Nightshift only after the demonstration

Extract to a standalone repository/package when the generic boundary is exercised, not just renamed. Migrate relevant issues with explicit source/target links; avoid two authoritative copies. Keep a thin Cubism config/skill adapter and pin a tested Nightshift version. Use a second small repository to expose assumptions before promising general portability.

Add a second qualified executor when it provides measured value. Export attempt summaries asynchronously to Cubism for cost per slice, review amplification, repair rates, elapsed time, model comparisons, and human interventions. Backfill from GitHub summaries where possible; missing telemetry never blocks delivery.

Defer parallel attempts, Beads/Dolt, hosted runners, rich dashboards, and multi-tenant services until a measured need exists. No full issue-backlog migration, massive preplanning, or architecture-linter project is on the critical path.

## 8. Ordering and checkpoints

| Order | Work | Advance when |
| --- | --- | --- |
| 1 | B0 | Fresh-session continuation works through existing lifecycle |
| 2 | R1 → R2 → R3 | Merge, gates, and readiness pass adversarial behavior tests |
| 3 | R4 → R5 → R6 | Recovery is explicit, reviews bounded, trusted boundary established |
| 4 | C1 → C2 → C3 | A real Cubism milestone is verified and accepted |
| 5 | N1 → N2 → N3 → N4 → N5 → N6 | Local Nightshift demonstrates recoverable milestone execution |
| 6 | E | Extraction and further capabilities justified by actual usage |

These are dependency stages, not fixed-duration promises. Split any item that cannot fit a bounded implementation session; preserve the intended outcome and create explicit dependencies. Repair/review attempts can span sessions without manufacturing a new work item. A newly discovered release-blocking product defect may take priority if recorded on the tracking epic; housekeeping does not displace the next product checkpoint by default.

## 9. Copy-paste bootstrap prompt

Attach this document to the coding session or place it in that session's accessible filesystem, then use:

```text
Work in jeromebanks/cubism-rs. The attached Cubism_SDLC_to_Nightshift_Plan.md is the requested execution roadmap. Read it and inspect current repository/GitHub state before making changes. Do not assume its baseline is still current.

Implement B0: adopt the roadmap at docs/sdlc/NIGHTSHIFT_EXECUTION_PLAN.md, add a canonical continue-plan skill under .agents/skills with a thin Claude adapter, and link it from agent orientation. Follow applicable skill-authoring instructions. Reuse epic #38 and existing equivalent issues; record real issue mappings there and create only the immediate repair slices needed next. Mark obsolete transition guidance as historical without rewriting the old evidence.

The skill must reconstruct progress from GitHub, reconcile existing work before starting another slice, select by dependencies and current gates, and delegate implementation/review/merge to the existing lifecycle. Default to one slice per invocation, including review and repair attempts. It must continue through implementation rather than ending with a plan when implementation is eligible. Preserve unrelated work and never seize an active claim.

Use AGENTS.md, SDLC.md, current config, isolated worktrees, required tests, a genuinely independent reviewer, and the deterministic merge path. You are authorized to create/update the scoped planning issues, commit, push, open the PR, and merge when repository policy is satisfied. Do not bypass a broken gate, invent review evidence, or approve a human checkpoint. If a gate defect prevents B0, preserve the work and record the exact repair required.

Keep GitHub as the durable work record. Do not introduce a database, service, scheduler, or Nightshift implementation in B0. Do not copy all Nightshift design files onto main. Include finite continuation/review rules and a stable end-of-session report in the skill.

Complete B0 through verification, independent review, and merge if eligible. End with the issue/PR, resulting SHA, tests actually run, unresolved limitations, and the exact next issue or decision. The next planned implementation is R1 unless current evidence shows it is already complete or blocked. Do not merely describe how someone else could do this work.
```

## 10. Copy-paste continuation prompt

After B0 has landed, use the same prompt on every new coding session:

```text
Continue the adopted Cubism-to-Nightshift plan in jeromebanks/cubism-rs using the repository's continue-plan skill. Read current instructions and docs/sdlc/NIGHTSHIFT_EXECUTION_PLAN.md, then reconstruct progress from GitHub. Reconcile unfinished plan work before selecting the next dependency-satisfied slice. If the next item needs decomposition, create/reuse a bounded issue and proceed into implementation when eligible.

Complete one slice through the existing implementation, validation, independent review, and merge lifecycle. You are authorized to make its scoped issue/PR updates, commit, push, and merge when repository policy passes. Honor human gates and persisted limits. Do not reset budgets, fabricate receipts, bypass checks, or disturb another session's active work. If blocked, leave a durable continuation record and identify the exact blocker.

Finish with: result, issue/PR/SHA, delivered behavior, actual verification, remaining limitations, and the exact next eligible issue or human checkpoint. Tell me whether any action from me is needed and provide the precise continuation instruction. Do not stop at a proposed plan when the selected implementation is executable.
```

The entry prompt stays stable. The issue graph, durable evidence, and current stage determine what happens next.
