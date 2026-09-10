# Nightshift — Migration roadmap, risks, and architectural decisions

[Overview](index.html) · [Document index](README.md) · [Previous](07-reliability-security-economics.md)
> Design proposal · Packaged 2026-09-09 · Original sections 19–21 preserved below. Repository findings refer to the inspected checkpoint, not live project state.

## 19. Migration roadmap and independently reviewable slices

The stages below are design proposals, not created issues. Each proposed slice must receive its own contract and implementation session when authorized.

### Stage 0 — Trustworthy Cubism dogfood

**Scope:** Repair known gate failures and establish honest, single-host operating limits.

**Boundary:** Existing scripts remain the user interface; one trusted dispatcher owns execution. Explicitly disable claims of safe multi-host scheduling.

Ordered vertical slices:

| Slice | Outcome and acceptance |
|---|---|
| 0.1 | Applied merge works; mock-based tests exercise the actual command construction and exact expected head |
| 0.2 | Unauthenticated and contradictory review receipts cannot satisfy the gate; trusted review execution publishes evidence independently |
| 0.3 | Claim and merge both enforce readiness, parent gates, and linked feedback exceptions |
| 0.4 | One dispatcher owns local claims through a transactional registry; abandoned work is reconciled before reassignment |
| 0.5 | Milestone approval identifies source/PR/test/demo evidence and exact bundle digest; status displays freshness |

**Demonstration:** Deliver a small Cubism correction through implementation, independent review, merge, generated evidence, and explicit milestone acceptance; replay the known adversarial fixtures.

**Human checkpoint:** Accept that the repaired workflow is safe enough for bounded dogfood under its stated trust assumptions.

**Risks:** Shared host administration remains trusted; local identity separation is weaker than hosted isolation.

**Non-goals:** Hostile tenants, arbitrary remote runners, production deployment automation.

**Exit:** Known gate cases fail closed, the delivery loop succeeds, and interruption recovery preserves evidence.

**Kill/pause:** Any unexplained accepted forged receipt, bypassed human gate, or uncertain merge treated as success.

### Stage 1 — Single-repository control-plane kernel

**Scope:** Extract domain commands, leases, evidence, and orchestration without rebuilding the whole UI.

**Boundary:** PostgreSQL becomes workflow authority. The script becomes a client; tracker labels become projections.

| Slice | Outcome and acceptance |
|---|---|
| 1.1 | One slice contract can be created, validated, and inspected through API/CLI with version checks |
| 1.2 | Two competing runners yield one authoritative lease; stale publication is rejected |
| 1.3 | One attempt survives orchestrator restart and resumes from a sealed checkpoint |
| 1.4 | One cold review produces an authenticated attestation accepted only for its assigned candidate |
| 1.5 | One milestone bundle is sealed and accepted idempotently through the decision service |

**Demonstration:** Kill the orchestrator between candidate publication and result recording; restart and finish without duplicate PRs or lost state.

**Human checkpoint:** Accept the domain model, evidence contract, and operational experience.

**Risks:** Dual-state migration and workflow versioning.

**Non-goals:** Full multi-tenancy, multiple trackers, generalized production release management.

**Exit:** Complete restart/replay qualification and one full program milestone through the kernel.

**Kill/pause:** The kernel requires manual database repair for routine crashes or costs more operationally than its demonstrated value.

### Stage 2 — Hosted multi-runner pilot

**Scope:** Hosted control plane, multiple isolated runners, two qualified harnesses, independent review pool.

**Boundary:** Privileged source effects and signing move outside all coding runners.

| Slice | Outcome and acceptance |
|---|---|
| 2.1 | A hosted runner completes one contracted slice with no source-write credential in its environment |
| 2.2 | A second harness resumes a portable checkpoint under a new attempt with complete provenance |
| 2.3 | A two-runner contention/failure scenario preserves exclusive authority and budget |
| 2.4 | Native queue or serialized integration handles a moving base and ambiguous response correctly |
| 2.5 | A hosted demo and presentation bind to an accepted bundle |

**Demonstration:** Run a short dependency DAG with concurrent independent slices, kill one runner, fail a provider, and complete the milestone.

**Human checkpoint:** Pilot customer accepts usefulness, evidence clarity, cost, and interruption rate.

**Risks:** Adapter instability, review cost, infrastructure overhead, incomplete model accounting.

**Non-goals:** Broad public signup, regulated certification, unlimited harness support.

**Exit:** Suggested pilot target: at least 20 representative slices, no invariant violations in fault tests, and an agreed majority completed without unplanned human help.

**Kill/pause:** Recovery consumes more human effort than supervised coding, or accepted-outcome cost is commercially unacceptable.

### Stage 3 — Multi-tenant beta

**Scope:** Tenant isolation, quotas, billing, privacy controls, customer onboarding, reliable cockpit.

**Boundary:** Every authorization, storage, execution, and accounting path is tenant-scoped.

| Slice | Outcome and acceptance |
|---|---|
| 3.1 | Two tenants execute identical slice IDs without data or authority crossover |
| 3.2 | Object access and evidence export reject cross-tenant references |
| 3.3 | Concurrent attempts obey reserved budget and tenant fairness |
| 3.4 | A customer revokes access mid-run and the factory safely contains/reconciles work |
| 3.5 | A tenant completes export and retention/deletion workflows with verifiable scope |
| 3.6 | Restore drill fences old authority and reconstructs external outcomes |

**Demonstration:** Two customer programs run concurrently; one is stopped and one continues. Demonstrate unauthorized access rejection and restore.

**Human checkpoint:** Security and product acceptance for limited beta.

**Risks:** Cross-tenant exposure, noisy neighbors, accounting disputes, support load.

**Non-goals:** Active-active writes, every enterprise deployment mode, fully automatic policy learning.

**Exit:** Independent isolation assessment, successful disaster drill, stable cost reconciliation, and repeat customer use.

**Kill/pause:** Any unexplained tenant boundary failure or inability to identify affected evidence.

### Stage 4 — Enterprise-ready platform

**Scope:** Customer VPC/hybrid deployments, delegated authorities, multi-repository release vectors, stronger audit, enterprise support.

**Boundary:** Customer-controlled trust roots, data locality, and external governance integrate with the same kernel.

| Slice | Outcome and acceptance |
|---|---|
| 4.1 | Customer runner completes work while code/artifacts remain in the selected boundary |
| 4.2 | Customer key revocation prevents new execution and decryptions as specified |
| 4.3 | Two-repository compatible release handles partial integration and recovery |
| 4.4 | Multi-approver milestone contract rejects unauthorized, duplicate, and obsolete decisions |
| 4.5 | Regional failover demonstrates epoch fencing and stated RPO/RTO |
| 4.6 | Independent auditor verifies an exported outcome using retained trust/evidence material |

**Demonstration:** Execute and accept a multi-repository milestone with a partial deployment failure and governed recovery.

**Human checkpoint:** Enterprise operational-readiness and contract acceptance.

**Risks:** Deployment fragmentation, support economics, customer-runner trust ambiguity, compliance scope creep.

**Non-goals:** Universal legal acceptance, arbitrary safety-critical autonomy, invisible administrative bypasses.

**Exit:** Contracted SLO evidence, documented shared responsibility, support runbooks, security assessment, and paying reference customers.

**Kill/pause:** Customer-specific forks dominate development or required guarantees cannot be enforced by the selected external systems.

## 20. Top risks, prioritized questions, and what not to build

### Top risks

| Risk | Response |
|---|---|
| Review appears independent but shares compromised authority | Separate assignment, execution, signing, and source mutation privileges |
| Correct local work fails after integration | Test actual integration candidates and immutable release vectors |
| State exists in several systems with inconsistent outcomes | Authoritative domain journal, idempotency, outbox, reconciliation |
| Milestone presentation overstates evidence | Criterion-to-evidence mapping and explicit limitations |
| Models optimize for passing gates rather than solving the problem | Independent hidden verification, outcome metrics, human milestone review |
| Costs grow through rework and stalled attempts | Reservations, bounded retries, progress detection, accepted-outcome accounting |
| Prompt injection causes permitted but harmful actions | Narrow operation grants and independent acceptance verification |
| Enterprise requests fragment the architecture | One kernel, qualified deployment profiles, explicit unsupported capabilities |
| A compromised root invalidates signed evidence | Separate roots, external archives, revocation and descendant invalidation |

### Prioritized unresolved questions

These do not prevent the first design.

1. **Initial authority envelope:** Does the first pilot end at merge, staging deployment, or production deployment?
2. **Initial customer:** Solo developer dogfood, a small internal engineering team, or a design-partner organization?
3. **Data boundary:** Which repositories may be sent to which model providers, and in which regions?
4. **Integration capability:** Which source-host plans and branch protections must the first pilot support?
5. **Human acceptance authority:** Single program owner, delegated technical owner, or multiple approvers?
6. **Economics:** What cost and interruption rate would make a completed milestone worth buying?
7. **Harness qualification:** What are the actual executable/API interfaces for Meta Muse, and how are model options such as Astra exposed by each qualified harness?
8. **Evidence retention:** Which artifacts must survive offboarding, and which raw content must never be stored?
9. **Deployment support:** Is customer-runner support needed for the pilot or only after hosted value is demonstrated?
10. **Regulated scope:** Which concrete control requirements matter, rather than a generic “enterprise compliant” label?

### What not to build

- A replacement source host, tracker, CI system, or identity provider.
- A universal autonomous agent with unrestricted tools.
- A proprietary workflow engine before proving Temporal unsuitable.
- A graph database merely because evidence forms a graph.
- A multi-cloud control plane at launch.
- A marketplace of unqualified harnesses.
- A model leaderboard based on unadjusted pass rates.
- Automatic policy relaxation driven by agent success metrics.
- A chat transcript presented as an audit trail.
- “Exactly once” claims spanning GitHub, providers, and deployment systems.
- A mandatory human approval for every slice.
- A production correctness guarantee based on a clean model review.
- A rewrite that discards the prototype’s contracts, evidence discipline, and human-facing reporting.

## 21. Recommended architectural decisions

These are suitable ADR starting points:

| ADR | Decision |
|---|---|
| ADR-001 | Nightshift owns durable workflow authority; source hosts own Git facts and trackers expose synchronized planning views |
| ADR-002 | Contracts, plans, candidates, evidence bundles, and human decisions are immutable versions |
| ADR-003 | PostgreSQL provides transactional domain state, lease ownership, budgets, and inbox/outbox |
| ADR-004 | Temporal coordinates long-running work; domain commands remain idempotent and independently authoritative |
| ADR-005 | Lease generations and authority epochs fence every new privileged effect |
| ADR-006 | Untrusted runners never hold source-merge, policy-signing, or human-approval credentials |
| ADR-007 | Independent review requires separate assigned identity and controlled context, with model diversity as an additional policy |
| ADR-008 | Blocking findings persist across rounds until explicit authorized disposition |
| ADR-009 | Policy is signed, activated independently, and cannot be weakened by the candidate it evaluates |
| ADR-010 | Merge assurance combines exact-head preconditions, qualified integration checks, host enforcement, and reconciliation |
| ADR-011 | Multi-repository programs use explicit non-transactional release vectors and compatible staged changes |
| ADR-012 | Evidence uses content addressing and in-toto/SLSA-compatible attestations with role-specific trust |
| ADR-013 | Human acceptance binds an exact bundle and grants explicit next actions |
| ADR-014 | Gate closure has admission and drain phases; uncertain external effects block progression |
| ADR-015 | Hosted untrusted execution begins with ephemeral VMs; Kubernetes and microVM density are later operational choices |
| ADR-016 | Telemetry excludes raw sensitive content by default and remains separate from audit evidence |
| ADR-017 | Pricing and optimization use capacity and accepted-outcome economics, including failed work |
| ADR-018 | Learning proposes versioned improvements; it never silently changes customer policy |
| ADR-019 | Disaster recovery changes authority epoch and reconciles external state before resuming |
| ADR-020 | Migration retains useful repository assets and advances only through demonstrated, human-accepted stages |

The recommended implementation path is **repair the Cubism gate under an explicit single-host trust boundary, extract a PostgreSQL/Temporal kernel, then introduce hosted isolated runners and authenticated evidence before opening multi-tenant access**. The first product demonstration should be one complete delegated milestone—including an injected failure, independent review, verified demo, and digest-bound human acceptance.

---

[Overview](index.html) · [Document index](README.md) · [Previous](07-reliability-security-economics.md)
