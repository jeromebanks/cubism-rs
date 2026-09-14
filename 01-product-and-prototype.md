# Nightshift — Product, prototype assessment, and principles

[Overview](index.html) · [Document index](README.md) · [Next](02-architecture-and-work-model.md)

> Revised design proposal · 2026-09-13 · GitHub-native, single-dispatcher profile. No Nightshift implementation is included.

## 1. Product definition and initial boundary

Nightshift is an open-source orchestration tool for an individual developer or small team. Its first purpose is to unblock Cubism development with a modest improvement over the existing issue/PR SDLC. A milestone can require multiple dependency-aware slices, and each slice can require multiple implementation, review, repair, and integration attempts across sessions. Humans define direction and accept coherent outcomes; they need not supervise each passing PR.

The default is one foreground dispatcher on an existing developer machine, existing GitHub Issues/PRs/CI, and installed executors. It requires no PostgreSQL service, Dolt server, SQLite database, Temporal cluster, custom distributed control plane, or persistent Nightshift infrastructure. Local workspaces, disposable caches, and executor-owned session files are allowed; they are not an authoritative Nightshift database. The dispatcher need not run while waiting for a human: restart reconstructs work from GitHub and executor records.

The initial authority envelope is one repository, bounded milestone scope, approved executors/models, finite attempt/time/spend limits, independent review, verified integration, merge, and human milestone acceptance. Release and deployment are disabled unless separately authorized by protected policy. Multi-repository releases and hosted enterprise operations are deferred.

| Owner | Responsibility |
|---|---|
| Human | Intended outcome, scope changes beyond delegation, protected policy, milestone acceptance |
| GitHub Issues | Authoritative milestone/slice topology, dependencies, assignment, discovered work, replanning and completion |
| Issue comments | Compact attempt records, decisions, protected-effect intents and outcome summaries |
| Git and GitHub PRs/Checks/statuses | Candidate identity, commit-specific verification, integration and merge facts |
| Executor | Native sessions, transcripts, context, checkpoints, permission interaction and detailed tool events |
| Nightshift process | Readiness, bounded scheduling, adapter coordination, separate review and in-process trusted effects |
| Cubism | Asynchronous analytics; never scheduling or acceptance authority |

## 2. Evidence-grounded assessment of the prototype

### Current design-branch inspection

This revision starts from `docs/nightshift-design-draft` at `231a161f06a354f34b1c6184a66a813f3d84f8c0`. All eight chapters, the guide, overview, generated reading editions, and renderer were inspected. The repository already has portable slice contracts, milestone manifests, labels, `scripts/sdlc.py`, and a static project cockpit. It has no implemented Nightshift kernel requiring a database migration.

Source inspection at this branch confirms the relevant old construction problems remain: `command_merge` references undefined `add`/`remove`; receipts accept any nonempty claimed reviewer/author and any historical pass for the head; check names collapse issuer identity; claim uses absence-then-push; and enforcement/configuration comes from the checkout. These are future repair requirements, not fixes performed here. See [the inspected script](https://github.com/jeromebanks/cubism-rs/blob/231a161f06a354f34b1c6184a66a813f3d84f8c0/scripts/sdlc.py) and [existing SDLC](https://github.com/jeromebanks/cubism-rs/blob/231a161f06a354f34b1c6184a66a813f3d84f8c0/SDLC.md).

The existing SDLC's one-session slice rule is a sizing heuristic to revise during implementation: a retry or exhausted context does not itself create new intended work. Its branch-claim procedure must not become Nightshift's ownership protocol. This documentation revision does not activate or modify that repository delivery policy.

### Preserved historical assessment

The following findings and diagnostic results are from the earlier checkpoint, retained with their original pinned evidence. They are not claims of new runtime testing or live GitHub configuration verification.


The earlier assessment inspected `chore/checkpoint-sdlc-worktree-state`, at `0cbaf2d3c92ea30eafe507a5460894f5322ccc69`. Its local `main` was `1ae95dd9fdfb56febf463c3fd5601ddafbeac4d0`, matching the brief.

The checkpoint changes 32 files, with 2,726 insertions and 441 deletions. The substantive transition is from the large historical time-series skill to portable instructions, deterministic Python tooling, issue templates, and human-facing HTML.

That earlier assessment reported its only untracked file as `astra-sdlc.txt`.

### Confirmed findings

| Finding | Classification and evidence | Consequence |
|---|---|---|
| Applied merge raises `NameError` | **Confirmed defect.** `command_merge` references `add` and `remove`, which are local to another function. [scripts/sdlc.py:675](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L675) | An otherwise eligible applied merge crashes before invoking the merge command. |
| Implementer can manufacture an acceptable receipt | **Confirmed authorization defect.** Receipt parsing obtains the comment author, but eligibility requires only nonempty author and reviewer fields. No independent execution identity is authenticated. [scripts/sdlc.py:393](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L393), [eligibility:467](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L467) | A comment written by the implementer can satisfy the review gate. |
| A later failed review does not invalidate an earlier pass | **Confirmed defect.** Eligibility searches for any matching passing receipt; it does not resolve subsequent failures or outstanding findings. [scripts/sdlc.py:467](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L467) | Review history can contain an unresolved failure while the gate passes. |
| Receipt submission can bind an old report to a new head | **Confirmed binding gap.** The submission command fetches the current PR head and attaches it to the supplied report, without requiring the report’s reviewed SHA. [scripts/sdlc.py:637](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L637) | A head change between review and submission can produce misleading provenance. |
| `issue/N` creation is not an exclusive distributed claim | **Confirmed protocol flaw by interleaving analysis.** A separate absence check precedes an ordinary push of the default-branch SHA. [scripts/sdlc.py:582](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L582) | Two workers may both report successful ownership. |
| Claims lack identity and recovery fencing | **Architectural gap.** Claim/resume uses branch presence and worktree existence, without owner, generation, heartbeat, expiry, or downstream fence. [scripts/sdlc.py:566](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L566) | Safe automated abandonment recovery is undefined. |
| Merge evaluation and mutation are detached | **Confirmed construction gap, currently masked by the `NameError`.** The merge command does not carry the evaluated SHA or a base expectation. [scripts/sdlc.py:656](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L656), [merge:675](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L675) | Repairing the exception alone would leave time-of-check/time-of-use exposure. |
| Check names overwrite one another | **Confirmed defect.** Check outcomes are stored in a dictionary keyed only by display name; the last entry wins. Issuer identity is not checked. [scripts/sdlc.py:410](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L410) | Conflicting or differently issued checks can be collapsed into success. |
| Human gates are not revalidated at merge | **Confirmed enforcement gap.** Merge eligibility checks the linked issue’s labels but does not load its parent or rerun readiness validation. [scripts/sdlc.py:437](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L437), [inputs:656](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L656) | A slice already in flight can remain eligible after its parent pauses. |
| Feedback exemption is broader than written policy | **Confirmed defect.** Feedback-labeled issues bypass both parent gate checks, including `gate:human-review`; a specific feedback decision link is not validated. [scripts/sdlc.py:372](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L372). Written policy limits the exception to linked feedback during changes requested. [AGENTS.md:21](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/AGENTS.md#L21) | A label can bypass the intended milestone pause. |
| Readiness is structural, not executable readiness | **Architectural gap.** Validation checks sections, parent, labels, and a checklist. It does not prove dependency completion, bounded effort, meaningful criteria, or even require `status:ready` in the direct claim path. [scripts/sdlc.py:350](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L350), [claim:572](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L572) | The UI’s “ready” language is stronger than the enforced contract. |
| Approval is a label/comment operation | **Architectural gap.** Any authorized invocation can apply approval labels and then post text. There is no bundle digest, authenticated human decision, or atomic relationship between the two operations. [scripts/sdlc.py:715](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L715) | Partial failure and fabricated approval remain possible. |
| Milestone evidence is issue membership | **Confirmed evidence limit.** The renderer checks required keys, epic identity, closed issues, and parent membership. It does not verify commits, merges, tests, artifacts, or demos. [scripts/sdlc.py:692](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L692) | “Closed” is insufficient evidence that the claimed outcome exists. |
| Policy and enforcement come from the executing checkout | **Architectural trust gap.** Default configuration is relative to the script; an arbitrary config path is accepted. [scripts/sdlc.py:24](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L24), [CLI:826](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L826) | The tool does not establish a trusted policy origin independent of the candidate change. |
| Status is a bounded snapshot | **Confirmed limitation.** Fetches stop at 500 issues and 200 open PRs. The committed page records `2026-09-05T00:24:30+00:00`. [scripts/sdlc.py:86](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/sdlc.py#L86), [docs/project-status.html:15](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/project-status.html#L15) | Staleness and incomplete membership can mislead operators at scale. |

The claim race does not require two simultaneous server-side ref creations:

1. A and B each observe the branch absent.
2. A pushes base SHA `X`, creating the branch.
3. B begins its push after that creation and observes the remote already at `X`.
4. B’s push succeeds as an up-to-date operation.
5. Both continue and announce ownership.

Git distinguishes an already-up-to-date ref from a newly created ref; the script accepts command success without checking exclusive acquisition. This conclusion follows from the code and documented push semantics; I did not perform a remote race experiment. [Git push documentation](https://git-scm.com/docs/git-push)

### Validation performed and its limits

During that earlier assessment, all eight existing SDLC unit tests passed. Additional in-memory checks produced:

```text
self-authored receipt gate errors: []
pass then fail gate errors: []
duplicate check names gate errors: []
feedback during human-review readiness errors: []
applied merge: NameError name 'add' is not defined
external mutations attempted: 0
```

The tests cover useful parsing and rendering behavior, but no applied merge or concurrent claim scenario. [scripts/tests/test_sdlc.py:46](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/tests/test_sdlc.py#L46)

The required CI workflow aggregates Rust, documentation, schema, link, and binding checks; it does not include these Python SDLC tests. [ci.yml:150](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/.github/workflows/ci.yml#L150)

The local quality script can report success while skipping unavailable link and binding tools; it builds the Python binding but does not perform CI’s install/import check. That is acceptable as a local preflight if reported accurately, but cannot be treated as equivalent evidence. [quality-gate.sh:47](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/scripts/quality-gate.sh#L47), [ci.yml:141](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/.github/workflows/ci.yml#L141)

Live branch protections, installed Apps, provider availability, actual independent-review sessions, hosted demo behavior, and production deployment were not verified.

### Historical lessons

The PostScript workflow already has explicit local claim recovery with heartbeats and discussion of ABA races. It is richer than the Cubism branch claim in this respect, but remains a shared-filesystem protocol without establishing cross-host ownership. Its cold-review workflow also documents stale output, wrong-worktree review, and fix-before-re-review pitfalls. [PostScript work-issue:82](https://github.com/jeromebanks/postscript_interpreter/blob/1a5bb6520dfb9fea54a0a4fe34930e9dafa5d5e7/.claude/skills/work-issue/SKILL.md#L82), [review:553](https://github.com/jeromebanks/postscript_interpreter/blob/1a5bb6520dfb9fea54a0a4fe34930e9dafa5d5e7/.claude/skills/work-issue/SKILL.md#L553)

The historical Cubism process contributed especially valuable evidence discipline: distinguish what a test proves, what it does not prove, and what was deferred. The selected Phase 20 handoff demonstrates this concretely, including its distinction between confirmed and unexplained formatting behavior. [Phase 20:195](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/TIMESERIES_PHASE_20_HANDOFF.md#L195), [limitations:240](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/TIMESERIES_PHASE_20_HANDOFF.md#L240)

The roadmap also distinguishes completed tracked milestones from completion of the broader specification. Preserve that distinction in the product. [TIMESERIES_ROADMAP.md:38](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/TIMESERIES_ROADMAP.md#L38)

The retrospective documents the earlier absence of cold review on every slice; it is historical evidence, not proof of the current review posture. [sdlc-process-notes.md:173](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/sdlc-process-notes.md#L173)


## 3. Retain / repair / replace / add

| Existing asset or assumption | Disposition |
|---|---|
| Epic, slice, feedback and milestone contracts | Retain; add native dependencies and multiple attempts per slice |
| Acceptance criteria, non-goals, validation and “does not prove” fields | Retain |
| Quality commands and report templates | Reuse behind trusted validation; do not equate issue closure with evidence |
| Branch-as-claim and ad hoc receipt authorization | Replace with one dispatcher's assignment and broker-validated session/role binding |
| Candidate-relative policy | Load trusted policy and broker code independently of the candidate |
| Tracker as a synchronized projection of internal state | Delete; GitHub is the initial authoritative work graph |
| Full transcript/checkpoint replication | Delegate to executors; retain normalized references and summaries |
| Hosted service, tenant hierarchy and separate identity/signing systems | Defer; local operator and GitHub identity first |
| Rich cockpit | Defer; CLI, GitHub UI and optional existing static reports |

Infrastructure dispositions and future triggers are centralized in [section 18](07-reliability-security-economics.md).

## 4. Required invariants

1. Exactly one authoritative work-graph backend exists per deployment.
2. Attempts are execution records, never work items, GitHub Issues or Beads objects.
3. One dispatcher is the initial concurrency boundary; a second active dispatcher is unsupported.
4. A slice can have multiple implementation, review, repair and integration attempts.
5. Every attempt has a globally unique identifier used for idempotency and reconciliation.
6. Agent progress messages are not acceptance evidence.
7. Agents do not directly perform protected GitHub effects or hold integration credentials.
8. Review is separate from implementation and returns a structured verdict bound to exact inputs.
9. Replanning changes durable topology only when intended work changes; retries do not change the graph.
10. Cubism and telemetry are never required for correctness.
11. GitHub comments provide modest durability, weaker than database transactions; ambiguity must remain visible.
12. PostgreSQL is introduced only for explicit demonstrated scale or safety triggers in the future distributed profile.
13. A stale or revoked attempt cannot authorize new broker effects; already-submitted effects must be reconciled.
14. A candidate cannot weaken its own review or merge policy. Missing or stale evidence cannot become success.
15. Blocking findings remain unresolved until explicitly dispositioned; a later clean review cannot erase them.
16. Retry/time/spend bounds include reviews and abandoned work; unknown spend is not zero.
17. Human acceptance identifies exact source, criteria and evidence. Merge, release, deployment and acceptance are distinct facts.
18. A pause closes admission first, then settles in-flight effects; cancellation never implies rollback.

These requirements are enforced by trusted code and source-host controls, not entrusted to model instructions.

---

[Overview](index.html) · [Document index](README.md) · [Next](02-architecture-and-work-model.md)
