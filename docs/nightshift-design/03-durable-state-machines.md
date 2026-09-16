# Nightshift — Durable events and state machines

[Overview](index.html) · [Document index](README.md) · [Previous](02-architecture-and-work-model.md) · [Next](04-execution-scheduling-review.md)
> Design proposal · Packaged 2026-09-09 · Original sections 7 preserved below. Repository findings refer to the inspected checkpoint, not live project state.

## 7. Durable event and state model

PostgreSQL is the authority for aggregate transitions. Each command transaction:

1. Authenticates the actor and tenant.
2. Checks aggregate version, current policy, gate generation, and lease where applicable.
3. Validates preconditions and budget.
4. Appends the domain event.
5. Updates state and reservations.
6. Writes outbox records.
7. Commits before reporting acceptance.

Each event contains:

```text
event_id, tenant_id, aggregate_type, aggregate_id, aggregate_version,
event_type, actor_identity, command_id, idempotency_key,
causation_id, correlation_id, occurred_at, recorded_at,
contract_digest, policy_digest, authority_epoch,
payload_digest, evidence_refs, schema_version
```

Unique constraints enforce aggregate sequencing and command deduplication. A reused idempotency key with different input is rejected.

### Transition contract

The tables below enumerate allowed transitions. Unlisted transitions are rejected.

For every row, the idempotency key is:

```text
K = H(tenant, aggregate_id, transition_ID, expected_version, command_id)
```

The first accepted command stores its input digest and result under `K`; retries return that result. External steps additionally receive a persistent `operation_id`. Evidence columns are added to the common signed transition receipt containing actor, policy, inputs, and prior/new versions.

Actor names imply these scoped permissions:

| Actor | Authorization |
|---|---|
| Planner | `plan.propose`, `slice.propose`; cannot expand approved envelope |
| Validator | `contract.validate`, `slice.ready` |
| Scheduler | `attempt.allocate`, `lease.issue`, budget reservation |
| Runner | Current attempt’s `progress`, `checkpoint`, `candidate.submit` |
| Review service | `review.assign`, `review.aggregate`; cannot author implementation |
| Reviewer | Assigned round’s `review.submit`; no candidate write |
| Judge | `finding.adjudicate` within delegated risk limits |
| Effect broker | Exact signed source/deployment operation |
| Verifier | Assigned verification/build attestation |
| Milestone service | `bundle.assemble`, `bundle.seal`, gate coordination |
| Human | Current contract’s designated decision authority |
| Recovery controller | Reconciliation and preauthorized recovery only |
| Incident commander | Explicit containment, recovery, or exception authority |

Timeout profiles apply to every listed transition:

| Profile | Timeout and retry | Failure and recovery |
|---|---|---|
| **C: command** | 10-second request deadline; retry transaction conflicts up to three times with jitter | No partial domain commit; reject or return `retryable_conflict` |
| **A: execution** | Five-minute startup; contract runtime cap; 20-second heartbeat, 90-second lease | Expire and fence; preserve evidence; replacement gets a new attempt |
| **V: verification/review** | Default 30-minute job deadline, contract override; two infrastructure retries | Failure remains visible; exhausted execution becomes inconclusive, never passing |
| **E: external effect** | 30-second request deadline; transport ambiguity triggers reconciliation | Retry only after proving absence or using native idempotency/CAS; unresolved becomes `uncertain` |
| **H: human wait** | Default seven-day decision window; configurable reminders | Expire or remain waiting by contract; no automatic approval |
| **R: recovery** | Containment target 30 seconds; five-minute reconciliation intervals; escalation after 30 minutes | Keep affected scope fenced until verified recovery |

`T` marks terminal for that entity revision. `T/R` is a terminal attempt or round from which the parent may create a replacement. `N` is nonterminal. Terminal history is never rewritten.

### Slice lifecycle

| ID / transition | Actor and preconditions | Durable event | Profile; recovery | Evidence / result |
|---|---|---|---|---|
| S1: absent → proposed | Planner; approved program envelope | `SliceProposed` | C; reject invalid scope | Contract proposal; N |
| S2: proposed → ready | Validator; valid contract, dependencies, gate, budget feasibility | `SliceReadied` | C; remain proposed on failure | Readiness evaluation; N |
| S3: ready → active | Scheduler; capacity and exclusive lease acquired | `SliceStarted` | C/A; release reservation if launch never occurs | Attempt and lease IDs; N |
| S4: active → reviewing | Runner submits; candidate sealed, initial verification complete | `CandidateAccepted` | C; reject stale lease | Source/context/verification digests; N |
| S5: reviewing → rework | Review service; blocking findings | `ReworkRequired` | C; retain all findings | Finding set; N |
| S6: rework → active | Scheduler; fixes within scope, budget and gate permit | `ReworkStarted` | C/A; new attempt on failure | Fix contract and prior findings; N |
| S7: reviewing → merge_pending | Review service; all required rounds and checks pass | `SliceIntegrationRequested` | C/E; wait or supersede stale candidate | Review and check graph; N |
| S8: merge_pending → merged | Effect broker/reconciler; actual merge confirmed | `SliceMerged` | E; reconcile issue closure separately | Merge SHA and parentage; T |
| S9: any unmerged state → blocked | Policy/recovery controller; dependency, access, gate, budget, or safety failure | `SliceBlocked` | C/R; fence active work, settle effects | Reason and resume state; N |
| S10: blocked → ready | Validator; blocker resolved, old work reconciled | `SliceUnblocked` | C; new attempt if needed | Resolution evidence; N |
| S11: unmerged → superseded | Planner; replacement plan authorized, effects settled | `SliceSuperseded` | C; retain candidate | Replacement IDs; T |
| S12: unmerged → cancelled/failed | Authorized customer/controller; cancellation or exhausted policy | `SliceCancelled` / `SliceFailed` | C/R; drain in-flight effects first | Cause, spend, retained artifacts; T |

An already merged slice cannot be cancelled retroactively. A revert or correction is new work linked to the original.

### Execution attempt

| ID / transition | Actor and preconditions | Durable event | Profile; recovery | Evidence / result |
|---|---|---|---|---|
| A1: absent → allocated | Scheduler; reservation and lease transaction succeeds | `AttemptAllocated` | C | Runner class, model route, budget; N |
| A2: allocated → starting | Gateway; signed manifest accepted | `AttemptDispatched` | A; deduplicate launch ID | Manifest and runtime image digest; N |
| A3: starting → running | Runner supervisor; identity, workspace, policy verified | `AttemptStarted` | A; fence failed startup | Runtime attestation; N |
| A4: running → running | Runner supervisor; valid lease and increasing sequence | `AttemptProgressed` | A; heartbeat failure expires lease | Tool/test/artifact progress delta; N |
| A5: running → checkpointing | Runner/controller; context, time, or preemption threshold | `CheckpointRequested` | A; preserve last durable checkpoint | Trigger and pending operations; N |
| A6: checkpointing → suspended | Supervisor; checkpoint uploaded, effects reconciled, credentials revoked | `AttemptSuspended` | A/R | Checkpoint digest; N |
| A7: suspended → starting | Scheduler; same attempt still permitted, fresh lease generation | `AttemptResumed` | C/A | Checkpoint compatibility evaluation; N |
| A8: running → completed | Supervisor; candidate and final usage sealed, no unresolved effects | `AttemptCompleted` | C | Execution attestation; T |
| A9: allocated/starting/running/checkpointing → failed_retryable | Controller; infrastructure/provider failure | `AttemptFailedRetryable` | A/R; new attempt | Failure and last checkpoint; T/R |
| A10: active → abandoned | Lease service; expiry or owner loss | `AttemptAbandoned` | R; fence and reconcile before replacement effects | Lease history and recovery report; T/R |
| A11: active/suspended → cancelled/failed_terminal | Controller; cancellation, policy violation, exhausted limit | `AttemptCancelled` / `AttemptFailedTerminal` | R; revoke and retain | Cause and final accounting; T |

A provider failover creates a new attempt when execution semantics or context continuity cannot be preserved. A failed attempt is never renamed into its replacement.

### Review round

| ID / transition | Actor and preconditions | Durable event | Profile; recovery | Evidence / result |
|---|---|---|---|---|
| Q1: absent → planned | Review service; immutable candidate, risk policy selected | `ReviewPlanned` | C | Required classes and review scope; N |
| Q2: planned → assigned | Review service; independent eligible identities available | `ReviewAssigned` | C/A | Assignment and separation evidence; N |
| Q3: assigned → running | Reviewer supervisor; verified clean context package | `ReviewStarted` | V | Context-access manifest; N |
| Q4: running → submitted | Reviewer; complete structured findings and coverage | `ReviewSubmitted` | C | Signed review attestation; N |
| Q5: submitted → passed | Review service; coverage complete, no unresolved blocking findings | `ReviewPassed` | C | Aggregated decision; T |
| Q6: submitted → changes_required | Review service; actionable blocking finding | `ReviewChangesRequired` | C; initiate rework | Findings and reproduction evidence; T |
| Q7: submitted → disputed | Implementer/reviewer; explicit evidence-backed disagreement | `ReviewDisputed` | C/V; assign judge | Competing claims; N |
| Q8: disputed → passed/changes_required | Judge or designated human; authorized disposition | `ReviewAdjudicated` | V/H | Ruling per finding; T |
| Q9: assigned/running/submitted/disputed → inconclusive | Controller; missing evidence, timeout, exhausted retries | `ReviewInconclusive` | V; replacement round | Failure and uncovered areas; T/R |
| Q10: any current round → superseded | Review service; candidate/contract changes | `ReviewSuperseded` | C; retain findings in next round | Old/new digest link; T |

A new clean round cannot erase an earlier blocking finding. It must explicitly resolve, invalidate, or carry that finding forward.

### Merge and release

| ID / transition | Actor and preconditions | Durable event | Profile; recovery | Evidence / result |
|---|---|---|---|---|
| G1: absent → integrating | Effect broker; slice eligible, branch capability qualified | `IntegrationStarted` | E/V | Head, base, policy and gate generations; N |
| G2: integrating → eligible | Verifier; integration candidate checks pass | `IntegrationVerified` | V | Tested integration SHA/tree; N |
| G3: eligible → submitting | Effect broker; current policy/gate valid, exact operation admitted | `MergeSubmitted` | C/E | Consumed authorization and expected head; N |
| G4: submitting → merged | Broker; host confirms exact result | `MergeConfirmed` | E | Merge SHA, parents, resulting tree; T |
| G5: submitting → uncertain | Broker; outcome ambiguous | `MergeOutcomeUnknown` | E/R; prohibit blind retry | Request/response evidence; N |
| G6: uncertain → merged/rejected | Reconciler; remote state conclusively classified | `MergeReconciled` | R | Source-host observations; T |
| G7: integrating/eligible → superseded | Controller; head/base/policy invalidates evaluation | `IntegrationSuperseded` | C; regenerate candidate | Changed inputs; T/R |
| G8: absent → building | Release service; explicit release vector selected | `ReleaseBuildRequested` | C/V | Source vector and build recipe; N |
| G9: building → verified | Builder/verifier; artifacts and release checks complete | `ReleaseVerified` | V | Build provenance, SBOM, verification; N |
| G10: verified → deploying | Effect broker; target environment authorization satisfied | `DeploymentSubmitted` | E | Artifact/config digests and environment generation; N |
| G11: deploying → deployed | Deployment verifier; runtime and health criteria pass | `DeploymentConfirmed` | V/E | Deployment and runtime attestations; T |
| G12: deploying → uncertain/degraded | Controller; ambiguous API or failed health | `DeploymentUncertain` / `DeploymentDegraded` | R | Observed environment state; N |
| G13: degraded → rolled_back | Recovery controller; permitted rollback verified | `DeploymentRolledBack` | E/V | Prior artifact and health evidence; T |
| G14: building/verified → failed/cancelled | Controller; terminal verification or authorization failure | `ReleaseFailed` / `ReleaseCancelled` | C/R | Failure and retained artifacts; T |

An uncertain deployment first enters reconciliation. It reaches deployed, degraded, or confirmed-not-applied before another deployment is admitted.

### Milestone review

| ID / transition | Actor and preconditions | Durable event | Profile; recovery | Evidence / result |
|---|---|---|---|---|
| M1: absent → draft | Planner/human; contract proposed | `MilestoneDrafted` | C | Contract revision; N |
| M2: draft → collecting | Milestone service; contract activated, checkpoint reached | `MilestoneCollectionStarted` | C/V | Included scope and gate-drain request; N |
| M3: collecting → sealed | Milestone service; effects drained, required graph complete | `MilestoneBundleSealed` | C | Bundle digest and validation report; N |
| M4: sealed → presented | Review service; presentation and demo availability verified | `MilestonePresented` | C/H | Presentation digest, demo access record; N |
| M5: presented → approved | Human; exact current bundle, authority, freshness, challenge valid | `MilestoneApproved` | C/H | Signed decision; T |
| M6: presented → conditionally_approved | Human; explicit machine-checkable conditions and permissions | `MilestoneConditionallyApproved` | C/H | Conditions and limited grants; N |
| M7: conditionally_approved → approved | Validator; original authorized conditions satisfied without changing bundle | `MilestoneConditionsSatisfied` | C/V | Condition evidence; T |
| M8: presented/conditional → changes_requested | Human; exact bundle decision | `MilestoneChangesRequested` | C/H | Original feedback and decision; T |
| M9: presented/conditional → rejected | Human; outcome unacceptable | `MilestoneRejected` | C/H | Rationale and next authority boundary; T |
| M10: sealed/presented/conditional → expired/superseded/withdrawn | Controller or authorized human; expiry, changed bundle, or invalid evidence | Corresponding `Milestone…` event | C/H; keep downstream gate closed | Cause and successor link; T |

Changes requested produce a new milestone revision. Historical acceptance remains a historical fact; it is never transferred to new evidence.

### Human feedback

| ID / transition | Actor and preconditions | Durable event | Profile; recovery | Evidence / result |
|---|---|---|---|---|
| F1: absent → captured | Human/decision service; authenticated feedback | `FeedbackCaptured` | C | Verbatim feedback and bundle link; N |
| F2: captured → triaged | Planner; interpretation and acceptance correction proposed | `FeedbackTriaged` | C/H for material ambiguity | Original-to-interpreted mapping; N |
| F3: triaged → linked | Validator; bounded corrective slices valid | `FeedbackSlicesLinked` | C | Slice IDs and gate exception linkage; N |
| F4: linked → implementing | Scheduler; designated feedback work starts | `FeedbackImplementationStarted` | A | Attempt links; N |
| F5: implementing → verified | Verifier; correction behavior demonstrated | `FeedbackVerified` | V | Corrective tests and demo; N |
| F6: verified → resolved | Human milestone decision, or explicit contract resolution rule | `FeedbackResolved` | C/H | Acceptance/disposition record; T |
| F7: any unresolved → blocked | Controller; dependency, budget, ambiguity, or access issue | `FeedbackBlocked` | C/H | Reason and resume state; N |
| F8: blocked → prior eligible state | Validator; blocker resolved | `FeedbackUnblocked` | C | Resolution evidence; N |
| F9: unresolved → withdrawn | Original authority; explicit withdrawal | `FeedbackWithdrawn` | C/H | Signed reason; T |

A recurrence creates a new feedback item linked to the resolved item.

### Incident and recovery

| ID / transition | Actor and preconditions | Durable event | Profile; recovery | Evidence / result |
|---|---|---|---|---|
| I1: absent → detected | Monitor, human, or verifier; credible signal | `IncidentDetected` | C/R | Trigger and affected graph; N |
| I2: detected → contained | Safety controller; containment policy applies | `IncidentContained` | R; escalate failed containment | Revocations, paused scopes, preserved evidence; N |
| I3: contained → assessed | Incident commander; impact and authority established | `IncidentAssessed` | R | Scope, root-cause hypotheses, recovery plan; N |
| I4: assessed → recovering | Recovery controller; approved or preauthorized plan | `RecoveryStarted` | R/E | Exact recovery operations; N |
| I5: recovering → validating | Controller; operations reconciled | `RecoveryValidationStarted` | V | Resulting state and artifact vector; N |
| I6: validating → resolved | Verifier/commander; health and evidence checks pass | `IncidentResolved` | C/V | Recovery attestation and follow-up work; T |
| I7: assessed/recovering/validating → escalated | Controller; authority insufficient or recovery exhausted | `IncidentEscalated` | R/H | Required decision and preserved state; N |
| I8: escalated → assessed | Authorized commander/human; new direction | `IncidentRecoveryReplanned` | C/H | Revised plan and authority; N |

False alarms receive an explicit assessment and resolution record; evidence is not deleted.

---

[Overview](index.html) · [Document index](README.md) · [Previous](02-architecture-and-work-model.md) · [Next](04-execution-scheduling-review.md)
