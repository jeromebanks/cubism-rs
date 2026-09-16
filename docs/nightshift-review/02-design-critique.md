# Astra — comments on the Nightshift design

The analysis is genuinely good. The defect table in §2 is accurate, specific,
and reproducible; the invariant list in §4 is the best part of the package; and
the refusal to claim cross-system exactly-once (§15) is the kind of honesty most
architecture documents skip. The critique below is about proportion and about
what the design leaves out, not about correctness.

## 1. The investment ratio is inverted

Stages 2–4 — hosted multi-runner isolation, multi-tenancy, customer VPC
deployments, SPIFFE workload identity, OPA signed bundles, in-toto/SLSA
attestation chains, a separate effect broker, role-scoped signers, KMS trust
roots, regional epoch fencing — are engineer-*years* of work. They receive the
large majority of the document's 15,000 words.

Stage 0 is the only stage that pays for itself this quarter. It gets five
one-line table rows.

The consequence is practical, not aesthetic: nobody can start on Monday. Slice
0.2 reads "unauthenticated and contradictory review receipts cannot satisfy the
gate; trusted review execution publishes evidence independently." That is an
outcome statement, not a design. The actual Stage-0 fix is roughly ten lines —
compare the receipt comment author against `pr.author.login` and reject a match,
then resolve receipts newest-first per `(kind, head_sha)` so a later failure
supersedes an earlier pass. "Trusted review execution publishes evidence
independently" is a Stage-2 sentence attached to a Stage-0 slice.

**Ask:** give Stage 0 the same treatment sections 7 and 10 get. Five slices,
each with the mechanism named, the file named, and the test named. Let Stages
2–4 stay at their current altitude — that is the right altitude for work that
may never be funded.

## 2. The invariants are the best content and are not tiered

Section 4 lists eighteen invariants as one flat list. They are not one kind of
thing. Some are cheap today and fix bugs that exist right now:

| # | Invariant | Cost at Stage 0 |
|---|---|---|
| 2 | At most one implementation lease is authoritative | Local lockfile or a GitHub API CAS — hours |
| 6 | Every blocking finding stays unresolved until authorized disposition | Receipt resolution order — ~10 lines |
| 8 | Missing, ambiguous, stale, or untrusted evidence cannot become success | Already the design of `evaluate_merge_gate`; needs three cases fixed |
| 14 | Merge, release, deployment, acceptance are distinct facts | Free — it is a naming discipline |
| 17 | An agent's claim of progress is not progress evidence | Free — it is a review-prompt discipline |

Others (1, 4, 5, 9, 16) require an identity system, a signing authority, and
tenant-scoped storage. They are correct and they are not available at any price
a single-developer project can pay this year.

**Ask:** split section 4 into *"invariants enforceable now"* and *"invariants
requiring the platform."* That one edit converts the most valuable section of
the document into a checklist someone can act on, and it stops a reader
concluding that none of it is reachable.

## 3. What the design gets right and should say louder

These cost almost nothing to adopt today, and each maps to a defect that exists
in the current script. They are currently distributed across sections 3, 12, 13,
and 15 where a reader meets them one at a time:

- **Merge ≠ release ≠ deployment ≠ product acceptance** (invariant 14). The
  current process conflates "issue closed" with "outcome delivered."
- **Two-phase gate: admission closed, then drained** (§15). This is the single
  most useful idea in the package and the one most likely to be skipped as
  pedantic. It is what makes "stop the factory" mean something.
- **An uncertain merge is never success** (§15, G5/G6). The current script's
  merge path has no uncertain state at all.
- **A new review round cannot erase an earlier blocking finding** (§9, Q10). The
  current receipt logic does exactly what this forbids.
- **"Closed" is insufficient evidence that the claimed outcome exists** (§2).
  The milestone renderer today checks issue membership and nothing else.
- **Blind the initial review pass** (§9). Withholding the implementation
  transcript from the reviewer is free and immediately improves review quality.

**Ask:** collect these into a short "adopt these now, regardless of platform"
section. They survive even if Nightshift-the-product is never built, which makes
them the most durable part of the design.

## 4. The design never budgets the human's time

Every section models agent cost, budget reservation, and spend accounting. None
models the operator's hours.

The reality this has to survive: one developer, eight crates, ~30 open issues,
and real unfinished engineering (#1 sparse memory ceiling, #2 unverified
DataFusion aggregate bound, #8 DataFusion version convergence, #14 untested
retry loop, #59 two live RUSTSEC CVEs). If the governed process costs more than
about twenty minutes per slice of *human* time, it loses to just writing the
code — and it will lose quietly, by being abandoned mid-epic, which is how the
time-series phase process ended.

Concretely unbudgeted: writing a `type:slice` issue that passes `check-slice`
(eight required sections with acceptance checklists); authoring the milestone
manifest narrative; reviewing the milestone report; adjudicating a review
dispute. Section 13 lists six generated deliverables per milestone and never
asks how long reading them takes.

**Ask:** add a subsection to §19 Stage 0 — *"Human minutes per slice, per
milestone, and the threshold at which this process is not worth running."* State
a target. Measure it during dogfood. It is the number that decides whether this
survives contact with a real backlog.

## 5. Three gaps

**Migration is unaddressed.** §19 Stage 0 assumes an existing repaired gate and
starts from there. The real starting position is a repository with 12 epics all
labeled `needs-slicing`, **zero** `type:slice` issues (so `check-slice` cannot
pass for anything that exists today), 37 unclassified legacy issues, and label
drift — #55 is closed but still carries `in-review`; #53 is open and labeled
`in-review` with PR #65 in flight. The design has no notion of adopting a
repository that is already mid-flight. That is the normal case, not the edge
case, for any customer after the first.

**Nothing verifies the verifier.** The design requires attestation for
everything an agent produces and never asks what validates `sdlc.py` itself.
Today `.github/workflows/ci.yml` aggregates Rust fmt/clippy/test/doctest/rustdoc,
schema validation, link checking, and a Python *bindings* import smoke test —
and does not run `scripts/tests/test_sdlc.py`. The 902-line script that gates
every merge in the system is the one component in CI's blind spot. Section 11
says a PR cannot weaken the policy used to approve itself; the same logic says a
PR should not be able to break the gate that approves it undetected.

**Solo operation is not a modeled deployment mode.** §5's four modes run from
Hosted SaaS to Self-hosted enterprise; all four assume an organization. The
first and most likely user is one person who is simultaneously the program
owner, the implementer, the approver, and the operator. Every separation-of-duty
invariant degenerates in that configuration — self-review becomes structurally
unavoidable at the human layer. The design should say what Nightshift still
guarantees when the human wears every hat, because that is the dogfood
configuration and the answer is not "nothing" (the agent-vs-agent separation
still holds; the human-vs-human separation does not).

## 6. Smaller observations

- **Temporal at Stage 1 is early.** §5 wisely warns against a proprietary
  workflow engine before proving Temporal unsuitable — but the reverse question
  also applies. Stage 1 is single-repository and single-tenant. PostgreSQL plus
  a polling loop covers it, and Temporal Cloud adds an operational dependency
  and a deterministic-workflow constraint before the domain model has stabilized.
  Suggest deferring it to Stage 2, where concurrent runners make durable timers
  genuinely load-bearing.
- **§20 question 7 is on the critical path, not a side question.** "What are the
  actual executable interfaces for Meta Muse, and how are model options such as
  Astra exposed by each qualified harness?" The entire provider-neutral adapter
  contract in §9 is unfalsifiable until one non-Claude harness is actually
  driven headlessly. That is a one-week spike and it should be Stage 0, before
  the adapter interface is designed against assumption.
- **§19's kill/pause criteria are excellent and unmeasured.** "Any unexplained
  accepted forged receipt" requires an adversarial fixture suite that does not
  exist. Name the fixtures as a Stage-0 slice or the criterion cannot fire.
- **The prototype assessment should note what it could not see.** §2 is careful
  about live protections and provider capability. It does not note that the
  assessed tooling was never merged to the default branch — which materially
  changes what "the prototype" means. See `README.md` in this folder.

## 7. The recommendation I would put to Jerome

Adopt Stage 0 as Cubism's actual process. Treat Stages 1–4 as a separate product
decision to be made later, on evidence from Stage 0 — specifically on the
human-minutes number in §4 above.

Stages 1–4 describe a different product than Cubism. Cubism is a Rust
aggregation library with real unfinished engineering and two open CVEs. The
factory should not become the project. Stage 0, done properly and honestly, is
both the right amount of process for this repository *and* the only credible
evidence that the rest is worth building.
