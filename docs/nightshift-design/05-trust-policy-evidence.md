# Nightshift — Trust, policy, and evidence

[Overview](index.html) · [Document index](README.md) · [Previous](04-execution-scheduling-review.md) · [Next](06-milestones-and-product-experience.md)
> Design proposal · Packaged 2026-09-09 · Original sections 10–12 preserved below. Repository findings refer to the inspected checkpoint, not live project state.

## 10. Trust, identity, and threat model

Trust roots are:

- Customer identity provider and registered human authentication credentials.
- Nightshift service identity issuer and workload enrollment.
- Role-specific signing keys controlled outside untrusted runners.
- Customer-approved policy authority.
- Qualified source-host, CI, and build issuers.
- Infrastructure operators responsible for the VM and control-plane boundaries.

SPIFFE provides a useful workload identity model with explicit trust domains and verifiable identity documents. Its use does not itself prove that a workload is uncompromised. [SPIFFE concepts](https://spiffe.io/docs/latest/spiffe-about/spiffe-concepts/)

GitHub integration should use separate Apps or independently isolated credentials for source writes, review/check publication, and privileged provisioning. Agents receive none of these credentials directly. Installation tokens can be scoped to repositories and permissions and expire after one hour; the broker keeps them outside the runner and imposes shorter operation grants. [GitHub App token documentation](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app)

### Threat controls

| Threat | Preventive | Detective | Recovery |
|---|---|---|---|
| Malicious agent or tool misuse | Capability broker; exact operation schema; sandbox; no direct merge credentials | Denied-operation events, anomaly detection | Fence attempt, revoke grants, replace runner |
| Repository/issue/log prompt injection | Separate trusted instructions from data; restrict tools and egress regardless of model request | Injection canaries, unusual tool requests, context-access audit | Quarantine context, restart with clean package |
| Forged review | Assigned reviewer identity, signed attestation, role separation | Signature/assignment mismatch | Reject evidence, invalidate dependent eligibility |
| Stolen source credential | Broker-only storage, narrow App scope, short-lived grants | Unexpected refs/actions and installation audit | Revoke installation/token, reconcile mutations |
| Secret exfiltration | No secrets by default; scoped broker; destination-aware egress | DLP signals, network and secret access logs | Rotate secrets, contain tenant scope, notify |
| Dependency/build compromise | Pinned inputs, approved registries, isolated build, verified provenance | Dependency scanning, reproducibility checks | Rebuild from trusted inputs; revoke affected artifacts |
| Tenant boundary violation | Tenant-bound identity, DB authorization, object-store policy, isolated VM | Cross-tenant negative tests and access alarms | Disable affected path, rotate keys, investigate exposure |
| Policy tampering in PR | Policy fetched from independent signed authority | Candidate policy differs from active policy | Reject candidate’s attempted policy influence |
| Artifact substitution | Digest verification at every promotion | Digest/provenance mismatch | Quarantine artifact; rebuild |
| Human gate bypass | Decision service and effect admission enforce contract generation | Projection drift, unauthorized promotions | Stop release progression; incident and compensation |
| Compromised runner host | Dedicated trust class; no signing/merge root keys in guest | Runtime attestation, independent verification | Destroy host, invalidate affected evidence |
| Compromised orchestrator | Separate policy/decision roots and effect broker verification | Independent audit export, abnormal grant requests | Disable authority epoch; restore and reconcile |
| Compromised signer or policy authority | Separate keys, constrained signing APIs, dual control for root changes | Key-use audit and external witnesses | Revoke trust version; invalidate affected descendants |

Prompt-injection defenses constrain consequences; they do not guarantee the model will interpret hostile text correctly.

A fully compromised administrative trust root can defeat controls in its domain. Higher-assurance deployments reduce that risk through separate accounts, customer-held keys, independent evidence export, and dual authorization for root changes. Cryptographic signatures prove who attested to bytes, not that those bytes describe truthful work.

## 11. Policy as code

Customers edit a typed policy schema, not unrestricted executable code. The compiler produces:

1. OPA policy/data bundles.
2. Source-host protection requirements.
3. Runner capability manifests.
4. Secret and network policies.
5. Workflow gates and budgets.
6. UI explanations and audit predicates.

Policy versions are signed and activated independently of product PRs. The active version governs the change that proposes its successor. Emergency revocations can invalidate previously admitted-but-not-consumed grants; ordinary policy changes specify whether in-flight work is grandfathered or revalidated.

OPA supports signed bundle verification, but that verification must be explicitly configured. An unsigned bundle is not automatically rejected in an unconfigured installation. [OPA bundle signing](https://www.openpolicyagent.org/docs/management-bundles)

```yaml
schema: factory.policy/v1
id: acme-default
version: 7
default: deny
required_checks:
  - name: factory/verification
    issuer:
      github_app_id: 12345
      workflow_digest: sha256:trusted-workflow
    conclusion: success
review:
  baseline:
    classes: [cold_correctness]
    independent_execution: true
    reviewers: 1
  rules:
    - when:
        any:
          - risk_at_least: high
          - paths_match: ["auth/**", "factory/leases/**", ".github/workflows/**"]
      require:
        reviewers: 2
        classes: [cold_correctness, security_specialist]
        distinct_model_families: true
        failure_injection: true
merge:
  strategy: merge
  require_current_integration_evidence: true
  trusted_admission_issuer: factory-effect-broker
execution:
  egress: [approved-model-gateway, approved-package-proxy]
  secrets: []
  max_attempts: 3
  max_slice_usd: 30
human_gates:
  placement: milestone
  policy_relaxation_authority: tenant-security-owner
release:
  staging: automatic_if_verified
  production: require_contract_grant
exceptions:
  require_named_authority: true
  require_reason_and_expiry: true
  agents_may_self_issue: false
retention:
  raw_model_content_days: 0
  diagnostic_logs_days: 30
  accepted_evidence_days: 365
```

The complete schema also covers sensitive operations, runtime ceilings, environment-specific secrets, demonstration requirements, merge queue settings, privacy classes, residency, retention holds, and stop scopes.

Policy compilation must report unsupported enforcement. If a repository cannot enforce a required source-host invariant, onboarding cannot silently declare it qualified.

Source-host checks must be identified by issuer and job identity, not display name alone. GitHub supports selecting a specific App as the expected source of a required status check. Nightshift’s verifier additionally validates workflow origin, run attempt, event, source SHA, and required job coverage. [GitHub protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)

## 12. Evidence and attestation model

The evidence graph is:

```text
intent → contract → plan → slice → attempt → candidate commits
       → verification → reviews → integration → merge
       → build → deployment → demo → milestone bundle → human decision
```

Edges have semantics: `implements`, `verifies`, `reviews`, `built-from`, `deployed-as`, `demonstrates`, `supersedes`, `accepts`, and `invalidates`.

An evidence node contains a schema version, media type, digest, producer identity, source references, policy/contract versions, observation time, collection time, storage location, retention class, and verification status.

Artifacts are addressed by SHA-256 of exact stored bytes. Git references additionally retain repository identity and full Git object IDs. Signed manifests avoid self-reference: the bundle does not contain its own digest or the later human decision.

### Attestation schemas

| Attestation | Required fields beyond the common envelope | Signer |
|---|---|---|
| Execution | Attempt/lease, source input/output, runner image, model requested/observed, harness, prompts/skills/tools digests, context manifest, checkpoints, usage, result and limitations | Execution supervisor through constrained execution signer |
| Review | Assignment, reviewed head/base/tree, contract, coverage by class, findings/dispositions, context-access manifest, reviewer identity, model/harness versions | Review signer bound to reviewer execution |
| Verification | Test recipe, immutable source/integration subject, toolchain, environment, command, exit code, report/log digests, retries, skips, limitations | Verifier/CI identity |
| Build | Build definition, resolved dependencies, builder identity, invocation, source, artifact subjects | Qualified build platform |
| Deployment | Release and artifact digests, target environment/configuration, operation ID, observed runtime, health checks, rollback reference | Deployment verifier |
| Milestone bundle | Contract, complete release/source vector, evidence roots, demos, narrative, presentation, risks, exclusions, operational readiness | Milestone assembler; signatures do not replace referenced evidence |
| Human decision | Exact bundle/contract, decision, conditions, accepted risks, authority, authenticated challenge, expiry/supersession semantics | Human authentication assertion plus decision-service countersignature |

Use in-toto statements for subject/predicate binding and SLSA build provenance for build facts. Custom execution, review, and milestone predicates extend this model without pretending that model review is a SLSA build level. [in-toto statement specification](https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md), [SLSA build provenance](https://slsa.dev/spec/v1.2/build-provenance)

### Illustrative review attestation

```json
{
  "_type": "https://in-toto.io/Statement/v1",
  "subject": [
    {"name": "candidate-tree", "digest": {"sha256": "tree104"}}
  ],
  "predicateType": "https://nightshift.example/attestations/review/v1",
  "predicate": {
    "tenant": "acme",
    "round": "review-22",
    "assignment": "assignment-41",
    "repository": "acme/service",
    "head_sha": "full-git-head-sha",
    "base_sha": "full-git-base-sha",
    "contract_digest": "sha256:contract3",
    "policy_digest": "sha256:policy7",
    "reviewer_identity": "spiffe://review.nightshift.example/acme/review-22",
    "implementation_attempt": "attempt-9",
    "review_attempt": "attempt-r22",
    "context_manifest_digest": "sha256:context22",
    "isolation_class": "separate-vm-no-implementation-transcript",
    "model": {"provider": "approved-provider", "observed_version": "version-id"},
    "harness_digest": "sha256:harness4",
    "coverage": ["correctness", "acceptance", "security", "test-adequacy"],
    "findings": [],
    "limitations": ["No production-load benchmark performed"],
    "verdict": "pass",
    "finished_at": "2026-09-09T02:00:00Z"
  }
}
```

The statement is wrapped in a signed envelope. The signing endpoint verifies assignment and observed execution identity; it is not an arbitrary “sign this JSON” API exposed to agents.

GitHub artifact attestations are useful imported evidence and distribution mechanisms. Retain independent copies and validate the artifact, repository, workflow identity, and predicate. Their existence alone does not establish acceptance, review independence, or a trustworthy build definition. [GitHub artifact attestations](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)

Verification follows the graph backward: verify digest, signature, trust root, signer role, tenant, assignment, policy, source binding, freshness, and invalidation records. A revoked or compromised signer can invalidate descendant eligibility without deleting historical evidence.

---

[Overview](index.html) · [Document index](README.md) · [Previous](04-execution-scheduling-review.md) · [Next](06-milestones-and-product-experience.md)
