# Nightshift — Trust, policy, and evidence

[Overview](index.html) · [Document index](README.md) · [Previous](04-execution-scheduling-review.md) · [Next](06-milestones-and-product-experience.md)

> Revised design proposal · 2026-09-13 · GitHub-native, single-dispatcher profile. No Nightshift implementation is included.

## 10. In-process trusted effect broker

The dispatcher and broker run trusted installed code outside the candidate checkout. Agents produce local candidate commits, review findings, structured verdicts or exact effect requests. The broker performs issue/comment mutations, branch publication, PR creation/updates, statuses/Checks, merge, release and other protected effects. Initial policy disables release/deployment effects unless explicitly enabled.

No agent receives unrestricted GitHub integration credentials. Removing an environment variable alone is insufficient: executor isolation must prevent access to the developer's credential files/helpers, keychain, SSH agent, cloud metadata, broker memory/control files and writable trusted code. Use existing executor OS sandboxing or an isolated user/container boundary with narrow mounts and no inherited credential helpers. Worktrees alone isolate edits, not secrets. If the installed adapter cannot enforce this boundary, block unattended protected effects. The host operator remains trusted; this is not hostile multi-tenant isolation.

The broker accepts typed operations, never arbitrary shell/HTTP requests. Validate current attempt, role, scoped capability, immutable inputs, policy, gate and target allowlist. A review capability cannot publish code or merge. Implementer-supplied text cannot assign a reviewer or issue human approval. Publish source through trusted Git operations with hooks disabled and safe configuration; never execute candidate scripts in the credential-bearing process.

### Effect idempotency and local fencing

Use stable keys derived from the logical action, target, exact input identity and authorizing attempt; persist the key, bounded normalized arguments (or immutable input reference) and payload digest before submission. Reconciliation uses the original key even after process restart. When a new attempt proposes an equivalent effect, first look for the existing logical target (slice/PR/head or release tag); a new attempt ID must not authorize a duplicate PR or release.

| Effect | Reconciliation identity / safe behavior |
|---|---|
| Attempt comment | Marker, broker author and confirmed comment ID; reread on POST ambiguity |
| Issue or discovered slice | Plan-change ID and logical new-slice marker on parent/child; never blindly duplicate |
| Label/dependency/assignment update | Desired exact state and target; reread current topology, preserve unrelated human fields |
| Candidate branch | Stable attempt-qualified name and expected full SHA; no force overwrite of unknown state |
| PR | Stable slice/candidate marker, repository, head/base refs; search open and closed PRs before creation |
| Status/Check | Exact SHA, context/name, trusted issuer, external ID where supported; identical status updates may create duplicate native records but not new authority |
| Merge | PR identity, expected head, trusted review/check inputs and integration intent; query PR and Git result after timeout |
| Release/deployment | Explicit policy grant and native stable tag/version or deployment operation ID; unsupported ambiguous actions require inspection |

GitHub generally does not honor Nightshift keys as a universal idempotency header. Safety comes from serialized broker calls, recorded intent, native preconditions, querying outcomes and refusing blind retries. For an ambiguous non-idempotent create, absence on one read is not proof that a delayed request cannot commit: keep it uncertain until settled or inspected. Identical duplicate journal comments/computation may occur; duplicate protected semantic actions must not be deliberately dispatched to resolve uncertainty.

Dispatcher-owned allocation, graph and gate operations use explicit trusted operator/policy authority, never agent-supplied approval. Executor-requested effects must belong to a current attempt; merge is a broker step of the authorized integration attempt, whose terminal outcome is recorded after the effect settles. A completed implementation attempt cannot later request merge.

On cancellation/reassignment, revoke the old in-memory capability before authorizing a replacement. Late results can be retained as historical facts but cannot overwrite the current candidate or verdict. A submitted effect remains pending until reconciled; revocation cannot retract it. There is no cross-host fence in this profile.

## 11. Protected policy

Load policy from the protected/default branch at a recorded commit or an explicitly trusted local/configuration source outside agent write access. Never load candidate-branch policy as authority for that candidate. A policy change is reviewed under the prior trusted version and becomes active only through authorized promotion. Use trusted installed broker code as well; safe configuration cannot protect a broker script replaced by an agent.

Start with a small validated schema and deterministic checks; no OPA service, signature infrastructure or identity provider is required. Record source/digest in attempts and revalidate current policy at effects. Policy revocation blocks new effects immediately when observed. Criteria/scope changes invalidate the affected assignments; they do not silently inherit old approval.

Illustrative fields, not current `.sdlc/config.json` syntax:

```yaml
schema: nightshift.policy/v1
repository: OWNER/REPO
work_graph: github
execution:
  max_active_attempts: 1
  max_total_attempts_per_slice: 10
  max_implementation_repair_attempts: 3
  max_review_attempts_per_candidate: 3
  max_integration_attempts_per_subject: 2
  max_attempt_minutes: 45
  max_slice_usd: 30
review:
  independent_session: true
  required_reviewers: 1
merge:
  expected_head_required: true
  require_integration_verification: true
  required_checks: [CI required checks]
  allowed_issuers: [configured-trusted-issuer]
human:
  approver_login: configured-owner
release: {enabled: false}
deployment: {enabled: false}
```

Pin workflow/issuer identity as well as check names. Validate source SHA, workflow origin, run attempt, conclusion and required job coverage; a same-named check from another issuer is not sufficient. Required status checks can be bound to an expected App through [GitHub branch protection](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches). Qualification must report unavailable protections or insufficient identity evidence, not claim enforcement it cannot establish.

The trusted GitHub principal may be a narrowly scoped operator token for the simplest status/comment integration, or an App installation. Rich Checks integration may require an App and appropriate Checks permission; the [Checks API documentation](https://docs.github.com/en/rest/checks/runs) includes credential-specific restrictions. Use commit statuses if that avoids requiring an App. Agents receive neither credential. Integration account compromise is outside the local broker's defense; revoke access, pause and inspect remote changes.

## 12. Evidence, summaries and retention

Progress and evidence remain separate. Evidence identifies what was run, who/what produced it, the exact source subject, recipe, result, relevant limitations and native references. Git commits supply immutable content identity; comments and issue bodies remain editable and deletable. Do not describe the journal as tamper-proof, permanently immutable or independently signed audit evidence.

| Evidence | Required compact summary |
|---|---|
| Implementation | Fixed inputs, candidate full SHA, outcome and limitations |
| Independent review | Assigned attempt/session, exact reviewed head/base, verdict, coverage and unresolved findings/dispositions |
| Test/build | Trusted workflow/job/run attempt or verifier identity, tested SHA, recipe, conclusion, failures/skips and time |
| Integration | Tested head/base or integration SHA/tree, checks and actual merge/result SHA |
| Milestone | Included slices/source snapshot, criterion-to-evidence mapping, reproducible demo, gaps and human decision |

Checks and commit statuses carry commit-specific verification results; they cannot represent a failed attempt that never produced a commit. The attempt comment therefore exists first and contains the durable important summary afterward. Native Checks can prune older same-named runs (the documented suite limit is 1,000). Actions [logs/artifacts have retention limits](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts), and deleting a workflow run deletes its associated artifacts. Actions artifacts are not permanent audit storage.

Store important pass/fail summaries, findings, subject IDs, recipe and references in comments while the raw records are accessible. Reference executor-native logs/transcripts with their access/retention limitations; do not duplicate full contents. GitHub has its own account/repository availability and deletion risks. Summaries preserve understanding, not the ability to re-verify missing raw evidence indefinitely. Before a pending promotion, expired required evidence must be rerun or explicitly inspected; an old summary alone cannot silently substitute for a required live check.

A milestone report may live in existing repository docs with a Git commit and digest for exact acceptance. That does not require a content-addressed artifact service. Permanent artifact storage, stronger attestations, WORM archives and independent trust roots are deferred until a specific retention or assurance need warrants them. Limit sensitive code, prompts, secrets and personal information in comments and telemetry; a public repository's journal is public.

---

[Overview](index.html) · [Document index](README.md) · [Previous](04-execution-scheduling-review.md) · [Next](06-milestones-and-product-experience.md)
