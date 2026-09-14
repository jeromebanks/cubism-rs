# Nightshift — Scheduling, execution, and independent review

[Overview](index.html) · [Document index](README.md) · [Previous](03-durable-state-machines.md) · [Next](05-trust-policy-evidence.md)

> Revised design proposal · 2026-09-13 · GitHub-native, single-dispatcher profile. No Nightshift implementation is included.

## 8. Single-dispatcher scheduling

The initial release runs one dispatcher for the repository and one attempt at a time, including review/repair/integration. Dependency-aware scheduling does not require parallel execution. Bounded parallel executors can be added within that same process after measured need; they do not require another dispatcher.

The operator must not run another dispatcher on another machine, clone, or overlapping repository scope. Use an OS-held exclusive lock in a trusted, repository-identity-scoped runtime directory on the designated host to reject accidental duplicate local starts; the kernel releases it on process death. It holds no work records and is not durable infrastructure. It cannot prevent a second host from starting. Moving hosts requires stopping the previous broker and revoking its credential access when its termination cannot be established. A timestamp, label or remote branch is not a distributed ownership mechanism.

The process keeps an in-memory assignment map keyed by slice/role and opaque per-invocation capabilities. Assignments are reconstructed from confirmed comments and reconciled sessions on startup, never blindly adopted. A stale executor may keep computing but cannot publish through the broker: results and effect requests must match the currently authorized attempt, role, fixed inputs and process-local capability. A resumed session receives fresh authority; old connections are not implicitly valid. This is local fencing, with the operator and host trusted.

Scheduling loop:

1. Fetch complete milestone topology, attempts and current gates. Prioritize recovery of uncertain effects over new work.
2. Reject missing criteria, cycles, inaccessible dependencies and unresolved gate/plan mutations. Recheck readiness at direct start, not only selection.
3. Prefer authorized feedback, then critical-path/explicit priority, then age and stable issue-number tie-break. A prerequisite normally requires verified merge, not merely a closed issue.
4. Match a qualified adapter and allowed model to role, scope, runtime and credential restrictions. Evaluate remaining attempt/time/spend limits without an external analytics dependency.
5. Confirm allocation comment, assign process-local authority, and launch. PR branches describe candidates, not ownership.
6. Observe bounded progress. Finish or reconcile the attempt before selecting its dependent work. Independently review and verify integration before merge.

At a human-review gate, ordinary work stops, including promotion of already-running candidates. During changes-requested, only corrective slices explicitly linked to the human decision may proceed. A feedback label alone is insufficient. Re-read gate and policy before every protected effect. An external human edit can still race that read; broker-acknowledged pause semantics and limits are in section 15.

### Retries, budgets and stalls

Illustrative defaults: three implementation/repair attempts per slice total, three review attempts per candidate, two integration attempts per subject, ten total attempts per slice, and a 45-minute runtime ceiling per attempt. All are configurable only through trusted policy. Failed startup allocations count until reconciliation proves no executor ran. A new candidate does not reset the total slice ceiling. Exhaustion blocks with a reason; it never weakens review.

Transient failures get bounded backoff and another attempt; invalid credentials, forbidden actions or repeated deterministic failure block. Changing providers/forking requires allowed routing and a new attempt. Preserve known usage and possible duplicate charges. Do not restart blindly after ambiguous launch.

A heartbeat/tool event proves activity, not progress. Look for changed candidates, reproduced/resolved failures, new verification or a bounded investigation result. Repeated commands, patch oscillation, repeated findings or rising spend without new evidence trigger a stall. Request a native checkpoint or concise summary, permit a bounded diagnostic attempt, then block or reroute. Quiet compilation follows its own job timeout rather than a chat-liveness test.

## 9. Executor contract and review

### Narrow adapter interface

Conceptual interface, not implemented APIs:

```text
capabilities() -> supported controls, versions, accounting fidelity
start(attempt_id, fixed_inputs, limits, role) -> session_ref
observe(session_ref, cursor?) -> normalized events and optional cursor
respond_permission(session_ref, request_id, decision) -> acknowledged or unsupported
interrupt(session_ref, reason) -> observed state
resume(session_ref, authorized_inputs) -> session_ref | unsupported
fork(session_ref, new_attempt_id, authorized_inputs) -> session_ref | unsupported
result(session_ref) -> candidate | review_verdict | integration_result | failure
usage(session_ref, baseline?) -> observed usage, basis, uncertainty
```

Permission requests arrive through `observe`; a response must belong to the current request and session. Unknown or unsupported permission handling pauses/fails closed. Native approval UI may be used when its operator binding can be verified; permission to run a tool never grants protected GitHub mutation. Interruption is best effort and must be observed/reconciled.

Codex, Claude Code, Factory Droid and future adapters own transcripts, native persistence, compaction/context management, tool-event streams, checkpointing and session resume/fork semantics. Nightshift stores opaque references, fixed inputs, concise outcomes and usage. It does not implement a portable conversation format or full checkpoint store. Cross-executor handoff starts a new attempt with source, criteria and concise findings; it does not claim native session continuity.

Adapters must declare unsupported features; qualify exact installed versions with a smoke run before enabling unattended effects. Model identifiers (including Astra when available) are model selections, not separate executors. An executor's own session files/databases do not become a Nightshift execution ledger.

### Factory as an optional substrate

[Droid Exec](https://docs.factory.ai/droid-exec/overview) documents headless operation, JSON results, native continuation/forking and bidirectional `stream-jsonrpc` for events, permissions and interruption. Use matching input/output `stream-jsonrpc` for that mode; older stream-JSON input is deprecated. Normalize session reference, tool lifecycle, permission requests, results and usage inside the adapter. Availability and semantics must be qualified against the pinned CLI version; no Factory dependency is required for other adapters.

The [Sessions API](https://docs.factory.ai/api-reference/sessions) exposes session lifecycle/messages and interruption, but is enabled only for selected organizations. [Droid Computers](https://docs.factory.ai/droid-computers/overview) supplies persistent execution environments. For enterprises already entitled to Factory, these can replace remote session/compute infrastructure that Nightshift would otherwise have to own. Use those facilities through the adapter; do not rebuild them. Computer IDs, API versions and Factory credit units stay in adapter-owned metadata. Native credit usage is not silently relabeled USD.

Nightshift retains milestone/slice scheduling, policy, independent review strategy, replanning and normalized outcomes. Factory Missions and Factory-native work orchestration are not dependencies or authorities.

The inspected [droid-action review validator](https://github.com/Factory-AI/droid-action/blob/b46affde010c543aefc51b5d82618b8e825845ca/src/tag/commands/review-validator.ts) limits the reasoning phase to structured findings, withholds GitHub mutation tools/token, and delegates publication to [trusted posting code](https://github.com/Factory-AI/droid-action/blob/b46affde010c543aefc51b5d82618b8e825845ca/src/entrypoints/github-post-review.ts). Adopt that separation as a pattern, not every action mode's permissions. The posting code's missing-SHA fallback is unsuitable: Nightshift rejects an unbound verdict rather than attaching it to the latest head. No repository-level license grant was found in this inspected droid-action tree/package, so do not copy its source without explicit licensing permission.

[Factory VFS](https://github.com/Factory-AI/vfs/tree/4852148cecde9c5413b51e4184eac55015bfe766) is a future workspace/session option only. The inspected manifest declares version 1.1.1 and MIT; the README describes an AgentFS fork, session handoff, Linux-first support and secondary macOS support. This is source/documentation evidence, not an independent runtime maturity assessment. Its Turso/SQLite-backed workspace design adds a database substrate and platform constraints, so it is excluded from the initial profile. Before reuse, pin a revision, verify applicable license notices/dependencies and test required recovery/platform behavior. Public visibility alone is not permission to copy source.

### Independent review protocol

1. Fix repository, head/base, contract revision and trusted policy identity before assigning a review.
2. Dispatcher assigns a fresh reviewer session and workspace. The implementer cannot choose or impersonate the reviewer or write its accepted record.
3. Give the reviewer criteria, source/diff, trusted instructions and relevant unresolved findings. Withhold implementation transcript and self-review conclusion for the first pass.
4. Require a structured verdict with attempt ID, reviewed head/base, criteria coverage, findings, limitations and verdict (`pass`, `changes_required`, `inconclusive`). Findings include stable ID, severity, violated criterion, location, reproduction/impact and disposition.
5. Broker validates session/assignment, exact subject and schema before posting the review comment and commit-specific status/Check. A narrative claiming success or exit zero is insufficient.
6. Repair uses a separate attempt on the slice. Changed head invalidates approval; changed base requires fresh integration and any base-sensitive review. Carry blocking findings forward until an explicit supported disposition resolves them.
7. Resolve disputes in a bounded independent adjudication or with the designated human when outside delegated authority. Never vote away a reproduced violation or keep cycling until one model passes.

Baseline is one cold reviewer plus deterministic checks. Self-review and an advisor are useful but not independent approval. Higher-risk paths can require additional specialist review or a different model family under protected policy; no automatic enterprise review matrix is imposed. Same-model cold review is possible but has correlated-error risk. Separate sessions and sandboxed workspaces provide practical independence, not proof against shared provider/model influences or a compromised host.

---

[Overview](index.html) · [Document index](README.md) · [Previous](03-durable-state-machines.md) · [Next](05-trust-policy-evidence.md)
