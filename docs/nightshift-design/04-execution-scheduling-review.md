# Nightshift — Scheduling, execution, and adversarial review

[Overview](index.html) · [Document index](README.md) · [Previous](03-durable-state-machines.md) · [Next](05-trust-policy-evidence.md)
> Design proposal · Packaged 2026-09-09 · Original sections 8–9 preserved below. Repository findings refer to the inspected checkpoint, not live project state.

## 8. Scheduling and lease protocol

The scheduling unit is a slice attempt. The authoritative lease record is keyed by `(tenant, slice, role)`; implementation permits one active owner. Review assignments use separate keys.

Acquisition uses a short PostgreSQL transaction that locks the lease and budget rows in deterministic order, verifies readiness, advances the generation, reserves resources, and records the assignment. PostgreSQL’s row-lock semantics provide the required serialization inside that database transaction. [PostgreSQL locking documentation](https://www.postgresql.org/docs/current/explicit-locking.html)

```json
{
  "schema": "factory.lease/v1",
  "tenant": "acme",
  "slice": "slice-104",
  "attempt": "attempt-9",
  "role": "implementer",
  "owner": "spiffe://runners.nightshift.example/acme/attempt-9",
  "authority_epoch": 12,
  "generation": 8,
  "fencing_token": "12:8",
  "issued_at": "2026-09-09T01:00:00Z",
  "expires_at": "2026-09-09T01:01:30Z",
  "heartbeat_sequence": 0,
  "capability_digest": "sha256:cap8"
}
```

The fence is an ordering token, not a secret. Identity and authorization are separate.

Protocol:

1. Scheduler acquires and commits the lease before dispatch.
2. Supervisor renews it every 20 seconds using owner identity, epoch, generation, and increasing sequence.
3. Database time determines expiry.
4. Renewal cannot revive an expired generation.
5. Expiry fences new effects before replacement work is admitted.
6. Replacement receives a new generation and normally a new attempt.
7. Old results may be retained as historical evidence but cannot update the current candidate.
8. Cleanup acts on exact attempt resources after checking active references.

A stale agent can still *believe* it owns work. A distributed system cannot erase that belief. The enforceable guarantee is that only the current owner can obtain acceptance of new authoritative actions.

Runners receive no reusable GitHub write token. They submit exact operations to the effect broker, which validates the current lease and consumes an operation authorization. For an already-admitted operation, the broker completes or reconciles that immutable operation; reassignment does not authorize a conflicting replacement while its outcome is unknown.

### Scheduling policy

- Weighted fair scheduling across tenants, with priority aging.
- Separate pools for implementation, review, verification, and recovery to prevent implementation from starving its own reviewers.
- Capability matching for OS, architecture, network, data classification, model provider, tool controls, and attestation class.
- Critical-path preference within each program.
- Hard tenant concurrency and spend reservations.
- Preemption only at safe checkpoints; never abandon an uncertain merge.
- Cancellation revokes new admissions, requests process termination, reconciles effects, and releases unused budget.
- Branches include slice and attempt identity, such as `factory/slice-104/attempt-9`.
- Each attempt receives an isolated clone/workspace. Hosted tenants do not share a writable checkout.

### Multi-repository semantics

There is no atomic transaction across repositories.

A program release is an immutable vector:

```text
service-A@SHA1 + client-B@SHA2 + schema-C@SHA3 + artifacts/configuration
```

Use backward-compatible interfaces, expand/contract migrations, feature flags, and staged releases. A partial integration is visible as partial. Compensation is a new revert or forward fix, not deletion of history.

Acquire any coordination locks in canonical repository order, and hold them only for bounded integration operations. Never hold cross-repository locks while waiting for model inference or human approval.

## 9. Provider-neutral execution and adversarial review

### Harness adapter contract

Claude Code, Codex, Meta Muse, OpenCode, and future harnesses implement the same interface. Astra is a model option represented in the execution manifest, not the product name or an assumed standalone harness:

```text
describe_capabilities()
qualify(runtime_manifest, policy)
start(execution_manifest, context_manifest, capability)
observe(cursor)
checkpoint(reason)
resume(checkpoint, execution_manifest)
cancel(reason)
collect_result()
collect_usage()
```

Each adapter declares:

- Supported model identifiers and provider routing.
- Headless operation, structured outputs, tool interception, and cancellation support.
- Filesystem/network controls it can honor.
- Native session continuation and export format.
- Token accounting fidelity.
- Runtime and harness version identity.
- Limits or unavailable capabilities.

A product name is not evidence that these capabilities exist. Meta Muse integration and model availability, including Astra, remain qualification targets until their actual runtime interfaces are tested. Unsupported capabilities cause routing rejection or a visibly weaker assurance class.

Context packages contain the contract, accepted ADRs, pinned source, required references, environment instructions, and typed prior findings. Every retrieved object has a digest and access classification. Repository instructions and retrieved content remain untrusted data unless separately promoted through the policy authority.

A portable checkpoint contains source state, patch digest, completed criteria, unresolved findings, observed failures, pending operation IDs, remaining budget, and artifact references. Native session state is optional, encrypted, and harness-specific.

Replay reproduces recorded orchestration decisions and inputs. It does not promise identical model outputs.

Model routing chooses from a customer-approved set. Provider failure may select an approved alternative without human interruption; it cannot silently change residency, data retention, trust class, or review-depth requirements.

### Progress detection

A heartbeat proves liveness only. Progress uses observable changes:

- New valid candidate or checkpoint.
- Acceptance criterion supported by new evidence.
- A failing test explained or resolved.
- A finding reproduced or dispositioned.
- A necessary investigation completed with a bounded conclusion.

Repeated identical commands, repeated patch reversals, repeated errors, growing cost without new evidence, and circular review disagreement trigger a stall classification.

Default response: ask the agent for a structured checkpoint, run a bounded diagnostic/advisor pass, then reroute or block after the contract’s stall budget. Long-running builds use their own progress signals so quiet compilation is not mistaken for agent failure.

### Review classes

| Class | Purpose | Counts as independent approval? |
|---|---|---|
| Self-review | Catch local omissions | No |
| Contextual advisor | Challenge plan and reasoning with implementation context | No |
| Independent cold review | Assess actual candidate without implementation narrative | Yes, if identity and isolation requirements pass |
| Security specialist | Evaluate attack surface and abuse cases | Additional required class for selected risks |
| Deterministic verification | Tests, static analysis, compatibility, benchmarks | Required evidence, not a substitute for product judgment |
| Human product review | Accept integrated usefulness, limitations, and direction | Milestone acceptance |

### Adversarial protocol

1. **Freeze the candidate.** Record repository identity, head, base, tree, contract, policy, and verification inputs.
2. **Assign independently.** The review service selects the reviewer. The implementer cannot select a favorable reviewer, mint its identity, or submit on its behalf.
3. **Create a clean environment.** Separate VM, credentials, session, scratch space, and memory namespace.
4. **Blind the initial pass.** Provide acceptance criteria, accepted ADRs, source, and diff. Withhold implementation transcript, self-review conclusion, and other reviewers’ conclusions.
5. **Require coverage.** Correctness, acceptance, threat model, security, performance, maintainability, and test adequacy each receive a result or an explicit justified non-applicability.
6. **Seek counterexamples.** Reviewers identify invariants and attempt concrete failures, including negative tests where useful.
7. **Submit structured findings.** Each finding includes severity, confidence, affected SHA/path, violated criterion, reproduction, impact, and suggested verification.
8. **Reveal selectively.** After the initial report is sealed, implementation rationale or peer findings may be disclosed for adjudication.
9. **Fix and re-review.** A changed head creates a new candidate and round; prior findings remain linked.
10. **Resolve disagreements.** A judge evaluates competing evidence. Reproduced violations cannot be waived by majority vote. Residual risk beyond delegated authority goes to the named human.
11. **Close deterministically.** Required classes complete, findings dispositioned, evidence current, and policy satisfied.

Different models can reduce correlated errors, but independence comes from separate execution identity, controlled information access, and separation of authority. The platform can attest which context it supplied and which tools accessed data. It cannot prove that model training, provider internals, or every repository comment was free of shared influence.

Default review depth:

| Change risk | Required review |
|---|---|
| Low | One cold reviewer plus deterministic verification |
| Medium | One cold reviewer from a different model family where approved; targeted integration checks |
| High | Two independently assigned reviews with different perspectives, including relevant specialist; stronger integration/failure tests |
| Critical authority, tenant isolation, release control | Two reviews, specialist verification, and the contract’s explicit governance boundary |

Same-model cold review is permitted for low-risk work under policy. Higher-risk substitution requires a preapproved fallback; outages never silently weaken review requirements.

Reviewer rotation, randomized assignment within qualified pools, held-out seeded defects, independent evaluation datasets, and restricted shared memory reduce persistent collusion and correlated blind spots. Do not reward reviewers for raw finding count or implementers for merely obtaining a pass.

---

[Overview](index.html) · [Document index](README.md) · [Previous](03-durable-state-machines.md) · [Next](05-trust-policy-evidence.md)
