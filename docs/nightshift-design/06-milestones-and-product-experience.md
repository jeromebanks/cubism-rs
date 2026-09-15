# Nightshift — Milestone contracts and product experience

[Overview](index.html) · [Document index](README.md) · [Previous](05-trust-policy-evidence.md) · [Next](07-reliability-security-economics.md)
> Design proposal · Packaged 2026-09-09 · Original sections 13–14 preserved below. Repository findings refer to the inspected checkpoint, not live project state.

## 13. Human milestone contract

A milestone contract defines what the human is accepting and what that decision authorizes next.

Required contents:

- Exact included scope and release/source vector.
- Excluded and deferred scope.
- Acceptance criteria with evidence mappings.
- Demonstrated user journeys.
- Demo environment recipe, dataset, configuration, and artifact digests.
- Test, review, performance, and operational evidence.
- Known limitations and named risk acceptances.
- Approval authority and required quorum.
- Decision window, expiration, supersession, and withdrawal rules.
- Permission granted by approval: continue planning, continue implementation, release, or deploy to a named environment.

Entry requires complete required evidence, no unresolved blocking findings, an integrated candidate, a verified demonstration, and a drained gate for the included scope.

The factory generates six linked deliverables:

| Deliverable | Purpose |
|---|---|
| Executive narrative | Outcome, business value, decisions, limitations |
| Presentation | Concise progression from problem to demonstrated result |
| Live or reproducible demo | Exercise exact accepted artifacts and fixtures |
| Evidence appendix | Trace each criterion to tests, reviews, source, and artifacts |
| Decision/risk register | Explain tradeoffs, deferred scope, accepted residual risks |
| Sign-off interface | Make the exact decision and resulting permissions explicit |

A suggested deck has eight slides: intended outcome, prior state, delivered journeys, demonstration, verification, limitations, decisions/costs, and acceptance contract. Narrative claims must cite evidence; unsupported claims are flagged before presentation.

```json
{
  "schema": "factory.milestone-bundle/v1",
  "tenant": "acme",
  "milestone": "recovery-demo",
  "revision": 2,
  "contract_digest": "sha256:contract-m2",
  "plan_digest": "sha256:plan6",
  "policy_digest": "sha256:policy7",
  "included_slices": ["slice-103", "slice-104"],
  "excluded_scope": ["Multi-region recovery"],
  "sources": [
    {"repository": "acme/service", "commit": "full-merge-sha", "tree_digest": "sha256:tree104"}
  ],
  "release_digest": "sha256:release12",
  "criteria": [
    {"id": "AC1", "status": "met", "evidence": ["sha256:test17", "sha256:review22"]}
  ],
  "demo": {
    "environment_manifest": "sha256:env9",
    "dataset": "sha256:fixture2",
    "journey_results": "sha256:demo-run8",
    "recording": "sha256:recording8",
    "reproduce": "sha256:demo-recipe4"
  },
  "presentation": "sha256:deck2",
  "evidence_index": "sha256:evidence-index2",
  "risk_register": "sha256:risks2",
  "operational_readiness": "sha256:ops2",
  "cost_report": "sha256:cost2"
}
```

The bundle digest is computed after assembly and is stored outside these bytes.

### Decision semantics

- **Approve:** Accept this exact bundle and issue only the permissions named in the contract.
- **Request changes:** Keep ordinary progression paused; authorize only linked corrective work within the feedback envelope.
- **Reject:** Decline the outcome. Replanning or termination requires the specified next authority.
- **Conditional approval:** Grant only explicit permissions under explicit conditions. Code changes needed to satisfy a condition require a new bundle; they cannot silently inherit acceptance.

```json
{
  "schema": "factory.human-decision/v1",
  "decision_id": "decision-73",
  "tenant": "acme",
  "milestone": "recovery-demo",
  "milestone_revision": 2,
  "bundle_digest": "sha256:bundle2",
  "contract_digest": "sha256:contract-m2",
  "decision": "approve",
  "accepted_risks": ["risk-local-region-only"],
  "grants": ["continue:milestone-3", "deploy:staging:release12"],
  "approver": {
    "issuer": "https://id.acme.example",
    "subject": "user-42",
    "authority": "program-owner"
  },
  "challenge_id": "challenge-91",
  "authentication_assertion_digest": "sha256:assertion91",
  "decided_at": "2026-09-09T03:00:00Z",
  "grant_expires_at": "2026-09-16T03:00:00Z"
}
```

Use fresh authentication with a challenge bound server-side to the bundle, decision, contract, and nonce. WebAuthn provides challenge-based public-key authentication; the decision service verifies the assertion, authority, and transaction context before countersigning the record. This is an operational acceptance record, not a blanket claim about legal enforceability. [WebAuthn specification](https://www.w3.org/TR/webauthn-3/)

Approval does not transfer when source, binaries, behavior-affecting configuration, acceptance criteria, material risks, or evidence subjects change. A new vulnerability may suspend use of an accepted release while leaving its historical acceptance intact.

URL relocation, storage replication, and additional annotations can occur without reopening acceptance when the original bytes and digests remain unchanged. Correcting a signed narrative creates a new artifact; it never overwrites what the human saw.

### Acceptance sequence

```mermaid
sequenceDiagram
  participant M as Milestone service
  participant B as Effect broker
  participant E as Evidence store
  participant H as Human
  participant D as Decision service
  participant P as Policy authority

  M->>B: Close admission for checkpoint scope
  B-->>M: Outstanding operations reconciled; gate drained
  M->>E: Seal bundle from immutable evidence
  E-->>M: Bundle digest
  M->>H: Present narrative, demo, risks, contract
  H->>D: Request decision challenge for bundle digest
  D->>P: Verify current authority and contract
  D-->>H: Bound challenge and exact granted permissions
  H->>D: Authenticated decision response
  D->>D: CAS current revision; deduplicate decision
  D->>E: Store signed decision
  D->>B: Activate permitted next actions
```

## 14. UI, API, and CLI surfaces

Within 30 seconds, a technical leader should understand:

- Which outcomes have been accepted.
- What the factory is doing now.
- The next useful result and its expected cost/date range.
- What is blocked and why.
- Whether any human decision is required.
- Whether displayed evidence and external state are current.

| Surface | Main interaction |
|---|---|
| Portfolio/program cockpit | Outcome progress, spend, critical path, gates, incidents, unmapped work |
| Work graph/factory floor | Dependencies, active attempts, queues, capability constraints |
| Slice/attempt timeline | Contract changes, tools, checkpoints, failures, retries, current candidate |
| Review confrontation | Finding, counterexample, implementation response, judge disposition, exact SHAs |
| Evidence explorer | Follow a claim to source, tests, artifacts, issuer, and limitations |
| Cost controls | Reservations, actual/estimated usage, budget runway, pause thresholds |
| Policy editor/explainer | Proposed rule, affected work, enforcement coverage, reason for denial |
| Incident console | Containment state, uncertain effects, recovery plan, authority needed |
| Milestone room | Presentation, exact demo, evidence appendix, risk register |
| Sign-off | Bundle identity, unmet criteria, accepted risks, decision and resulting grants |
| Audit export | Signed events, artifacts, policy versions, trust bundle, verification report |

Freshness is visible per adapter and evidence class. “Last observed five minutes ago” must not be rendered as live certainty. Progress separates merged slices, verified behavior, accepted milestones, and deployed releases.

Representative APIs:

```text
POST /v1/programs
POST /v1/programs/{id}/plan-revisions
POST /v1/slices/{id}/commands/start
POST /v1/attempts/{id}/checkpoints
GET  /v1/reviews/{id}/findings
POST /v1/findings/{id}/dispositions
GET  /v1/evidence/{digest}
POST /v1/milestones/{id}/bundles
POST /v1/milestones/{id}/decision-challenges
POST /v1/milestones/{id}/decisions
POST /v1/programs/{id}/stop
POST /v1/incidents/{id}/recovery-plans
GET  /v1/audit/exports/{id}
```

Commands require an idempotency key and expected version. There is no general API to set arbitrary workflow status.

Illustrative CLI:

```text
nightshift program inspect delivery-pilot
nightshift slice explain slice-104
nightshift attempt logs attempt-9
nightshift review inspect review-22
nightshift evidence verify sha256:bundle2
nightshift milestone open recovery-demo
nightshift program stop delivery-pilot --reason "Customer pause"
```

The CLI can initiate a human decision flow, but cannot convert an agent-held API token into human authority.

---

[Overview](index.html) · [Document index](README.md) · [Previous](05-trust-policy-evidence.md) · [Next](07-reliability-security-economics.md)
