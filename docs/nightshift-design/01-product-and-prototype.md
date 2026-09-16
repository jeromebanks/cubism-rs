# Nightshift — Product, prototype assessment, and principles

[Overview](index.html) · [Document index](README.md) · [Next](02-architecture-and-work-model.md)
> Design proposal · Packaged 2026-09-09 · Original sections 1–4 preserved below. Repository findings refer to the inspected checkpoint, not live project state.

This design recommends a service that owns the execution of software programs from agreed intent through verified delivery and human milestone acceptance. Its central product is **reliable delegation with inspectable evidence**: a technical leader can authorize an outcome, let bounded work proceed across sessions and agents, and review coherent results without supervising every pull request.

The Cubism prototype provides a useful operating model and reusable product assets. It does not yet provide the identity, concurrency, recovery, or authorization guarantees required for unattended operation across customers.

I read `astra-sdlc.txt` completely, inspected the specified branch and its changes against `main`, examined the requested workflow materials and selected historical evidence, and ran non-mutating diagnostics. No repository files, issues, comments, branches, or GitHub settings were changed.

## 1. Executive thesis and product definition

Nightshift is a **governed software delivery service** for programs that exceed the capacity of one coding session. Humans establish outcomes, constraints, budgets, and acceptance contracts. Nightshift plans and executes bounded slices, commissions independent reviews, coordinates integration, maintains evidence, and presents milestones for human acceptance.

“Autonomous” means the service can select and complete authorized work, recover from routine failures, and continue across sessions without new human instructions. This authority is limited by an explicit execution envelope:

- Allowed repositories, environments, tools, networks, and secrets.
- Approved outcome and scope.
- Risk limits and required verification.
- Time, spend, concurrency, and retry budgets.
- Milestone boundaries and exception authority.

“Dark” means ordinary delivery does not require a human operator. Every material action remains attributable and observable. A stopped factory must explain whether it is waiting for capacity, evidence, access, a dependency, or a human decision.

The initial customer is a small engineering organization with maintained repositories, functioning CI, and a technical leader willing to delegate a bounded backlog. Start with internal tools, developer infrastructure, and well-tested backend changes. Do not initially promise unattended operation on arbitrary legacy systems or safety-critical software.

### Ownership boundary

| Party | Owns |
|---|---|
| Customer | Product direction, repository contents, acceptance authority, risk appetite, credentials granted, deployment ownership, IP |
| Nightshift | Contracts, scheduling, execution coordination, policy enforcement, agent identities, review assignment, evidence, recovery, spend accounting |
| Source host | Git objects, refs, PR records, source-host permissions and enforcement |
| Tracker | Customer-facing planning records; synchronized views of factory work |
| CI/build system | Execution of identified verification/build jobs and their native records |
| Model provider | Model inference and provider-side processing under the customer’s permitted terms |
| Human approver | Acceptance, rejection, risk acceptance, policy exceptions within assigned authority |

Nightshift does not infer production deployment authority from permission to implement or merge. Milestone acceptance, source integration, release creation, and deployment are separate actions.

Initial assumptions: GitHub is the first source adapter; Linux is the first runner platform; customers permit selected external model providers; production deployment requires an explicit environment policy. These assumptions support a complete first design and remain revisable.

## 2. Evidence-grounded assessment of the prototype

The checked-out branch is `chore/checkpoint-sdlc-worktree-state`, at `0cbaf2d3c92ea30eafe507a5460894f5322ccc69`. Local `main` is `1ae95dd9fdfb56febf463c3fd5601ddafbeac4d0`, matching the brief.

The checkpoint changes 32 files, with 2,726 insertions and 441 deletions. The substantive transition is from the large historical time-series skill to portable instructions, deterministic Python tooling, issue templates, and human-facing HTML.

The only untracked file before and after inspection was `astra-sdlc.txt`.

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

All eight existing SDLC unit tests passed. Additional in-memory checks produced:

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

The PostScript workflow already has explicit local claim recovery with heartbeats and discussion of ABA races. It is richer than the Cubism branch claim in this respect, but remains a shared-filesystem protocol rather than a distributed lease service. Its cold-review workflow also documents stale output, wrong-worktree review, and fix-before-re-review pitfalls. [PostScript work-issue:82](https://github.com/jeromebanks/postscript_interpreter/blob/1a5bb6520dfb9fea54a0a4fe34930e9dafa5d5e7/.claude/skills/work-issue/SKILL.md#L82), [review:553](https://github.com/jeromebanks/postscript_interpreter/blob/1a5bb6520dfb9fea54a0a4fe34930e9dafa5d5e7/.claude/skills/work-issue/SKILL.md#L553)

The historical Cubism process contributed especially valuable evidence discipline: distinguish what a test proves, what it does not prove, and what was deferred. The selected Phase 20 handoff demonstrates this concretely, including its distinction between confirmed and unexplained formatting behavior. [Phase 20:195](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/TIMESERIES_PHASE_20_HANDOFF.md#L195), [limitations:240](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/TIMESERIES_PHASE_20_HANDOFF.md#L240)

The roadmap also distinguishes completed tracked milestones from completion of the broader specification. Preserve that distinction in the product. [TIMESERIES_ROADMAP.md:38](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/TIMESERIES_ROADMAP.md#L38)

The retrospective documents the earlier absence of cold review on every slice; it is historical evidence, not proof of the current review posture. [sdlc-process-notes.md:173](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/sdlc-process-notes.md#L173)

## 3. Retain / repair / replace / add

| Existing component | Decision | Product treatment |
|---|---|---|
| Epic → bounded slice → milestone model | Retain | Extend into versioned contracts and dependency graphs |
| Scope, non-goals, acceptance, validation, context fields | Retain | Convert templates into schema-backed contracts |
| Portable agent instructions and thin harness entry points | Retain | Package as versioned execution guidance; never as authority |
| “Does not prove” evidence discipline | Retain | Required verification limitation fields |
| Quality-gate command catalog | Retain and wrap | Run inside isolated runners with structured results and provenance |
| Current merge and receipt logic | Repair for dogfood, then replace | External admission and authenticated attestations |
| Remote branch as claim | Replace | Transactional lease service; branch becomes an output |
| Markdown/labels as workflow state | Replace | Durable domain state; tracker records become synchronized projections |
| Milestone manifest and report renderer | Retain and extend | Render from an immutable evidence bundle |
| Cockpit layout and planning warnings | Retain | Live projection with freshness and completeness indicators |
| Repository-relative policy loading | Replace | Signed policy versions from an independent authority |
| Shared writable build cache | Replace for hosted execution | Tenant- and trust-scoped caches with verified keys |
| Historical handoff chain | Retire operationally | Searchable historical evidence; structured checkpoints for continuation |
| Source-host configuration reconciliation | Retain concept | Adapter detects desired/actual protection drift |
| Tenant isolation, identity, accounting, incident handling | Add | First-class platform responsibilities |

The relevant template foundations are already present in the [slice form](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/.github/ISSUE_TEMPLATE/slice.yml#L1), [feedback form](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/.github/ISSUE_TEMPLATE/feedback.yml#L1), and [milestone manifest](https://github.com/jeromebanks/cubism-rs/blob/0cbaf2d3c92ea30eafe507a5460894f5322ccc69/docs/sdlc/milestone-manifest.example.json#L1).

## 4. Product principles and invariants

These are platform requirements, not instructions entrusted to an LLM:

1. Every executable action belongs to a tenant, contract revision, attempt, and authorization.
2. At most one implementation lease is authoritative for a slice at a time.
3. An expired or superseded lease cannot authorize new effects.
4. Agents cannot issue their own identities, approve themselves, activate policy, or grant additional permissions.
5. Review and verification apply to immutable source and input identities.
6. Every blocking finding remains unresolved until an authorized disposition addresses it.
7. A PR cannot weaken the policy used to approve itself.
8. Missing, ambiguous, stale, or untrusted evidence cannot become success.
9. Human acceptance refers to one exact bundle and one contract revision.
10. A milestone gate controls admission and promotion, not merely issue selection.
11. External side effects have durable intent, reconciliation, and an explicit ambiguity state.
12. Budgets include retries, reviews, abandoned attempts, and infrastructure.
13. Cancellation never implies that an already-admitted external operation was undone.
14. Merge, release, deployment, and product acceptance remain distinct facts.
15. Scope changes create a new contract or plan revision; earlier commitments remain inspectable.
16. Cross-tenant authorization never depends solely on application filtering or prompt instructions.
17. An agent’s claim of progress is not progress evidence.
18. Audit records distinguish observation, inference, assertion, and verified result.

A model may propose a transition. Deterministic services decide whether it is permitted.

---

[Overview](index.html) · [Document index](README.md) · [Next](02-architecture-and-work-model.md)
