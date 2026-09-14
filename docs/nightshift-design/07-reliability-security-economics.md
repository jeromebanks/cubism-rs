# Nightshift — Reliability, security, economics, and platform choices

[Overview](index.html) · [Document index](README.md) · [Previous](06-milestones-and-product-experience.md) · [Next](08-roadmap-and-decisions.md)

> Revised design proposal · 2026-09-13 · GitHub-native, single-dispatcher profile. No Nightshift implementation is included.

## 15. Reconciliation and protected integration

### Restart procedure

Start with no executor authorized to perform effects. Acquire the local process lock and establish that this is the only dispatcher. Load protected policy and trusted broker identity; do not continue from a candidate checkout's policy or a stale local cache.

1. Enumerate selected milestone issues, native dependencies and all attempt/plan-change/decision comments with pagination. Validate schema, broker authorship, IDs, fixed inputs, explicit corrections and current gates. Duplicate/conflicting records cannot be resolved by “last comment wins.”
2. Reconstruct incomplete attempts and prepared/submitted/uncertain effects, including attempts with no candidate or session reference. Read executor-native sessions using known references and attempt correlation keys where supported. Missing correlation is uncertainty, not proof nothing ran.
3. Inspect attempt branches and commits, open/closed/merged PRs, workflow runs/run attempts, Checks/statuses and recorded effect identifiers. Match repository, exact source, role and trusted issuer; a similarly named branch/session is insufficient.
4. Reconcile pending protected effects before authorizing replacement effects. A merge already observed is recorded as merged even if the old comment says running. An unknown merge remains blocked, never automatically reissued.
5. Classify each incomplete attempt using the table below; durably record the decision and any known/unknown usage. Revalidate fixed inputs and current policy before resumption or adoption of completed results.
6. Rebuild the in-memory assignment/limit view and repair UI labels from confirmed facts. Reopen scheduling only for unaffected, fully reconciled work. Complete partial replanning or gate mutations before using the affected graph.

| Classification | Required evidence | Recovery |
|---|---|---|
| Resumable | Native session/checkpoint identified, fixed inputs still valid, no conflicting effect | Confirm resume intent; issue fresh process-local capability; retain ID for the same assignment |
| Completed-but-unrecorded | Native result or GitHub effect identifiable for exact attempt/input | Validate result, record recovered outcome; schedule missing review/verification rather than implementation |
| Abandoned | Execution is lost/terminated/unrecoverable and protected effects are settled | Revoke old authority, record limitations and possible usage; new attempt only within limits |
| Requires human inspection | Conflicting records, unknown live execution, inaccessible evidence or unresolved protected effect | Preserve references and block affected scope; no optimistic success or duplicate mutation |

A running executor without a session reference may be discoverable by its launch key. If not, it can continue wasting compute after a crash, but cannot access broker credentials or use an old capability. The operator may need to locate/stop it. Only after effect uncertainty is settled may recovery mark it abandoned and permit bounded duplicate computation. This narrow crash window is accepted for the local profile. No PostgreSQL-level transactions, launch exactly-once, immutable event replay or lossless audit guarantee is claimed.

### Merge and integration

Independent review approves the exact candidate head and applicable base/contract/policy. Before merge, the broker rereads current head/base, parent gates, blocking findings, trusted required checks and policy. A changed head invalidates review. A changed base invalidates integration evidence and any base-sensitive review.

Use [GitHub merge queue](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue) where already available and qualified, including `merge_group` checks. Otherwise serialize Nightshift merges to the target branch, require host-enforced up-to-date checks, and verify the current integration subject. The [merge REST API](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request) accepts expected head `sha`; it does not provide an expected-base precondition. A local reread alone cannot prevent another actor moving the base. Without suitable host enforcement, stop before unattended merge rather than claim exact-base safety. The initial design does not build a replacement merge queue.

Record intent before the call and confirm actual PR merge state/result SHA afterward. An ambiguous response produces `uncertain`; query PR, branch and commit facts until settled or require inspection. Do not retry merely because a request timed out. Reconcile issue closure separately. After all component slices merge, run milestone integration on the actual assembled source; individual PR checks do not prove combined acceptance.

Merge, release creation, deployment and acceptance have separate predicates. A merged PR does not authorize a release, and a published release does not prove a healthy deployment. Future multi-repository work must expose partial results and use explicit source vectors and forward fixes/reverts, never claim atomic integration across repositories.

### Pause and failure behavior

A pause first prevents new broker admissions, then waits for previously submitted effects to settle. Only then is the gate drained. Broker-mediated pause is serialized with its effects; a direct GitHub edit is observed by polling/revalidation and may race an already admitted call. Show that distinction and pending operations. Cancelling an executor or removing a label cannot undo a queued/ongoing merge.

| Failure | Behavior |
|---|---|
| GitHub unavailable/rate limited | Back off per API hints, prioritize reconciliation, stop new launches/effects requiring durable writes; bounded compute may finish |
| Allocation POST lost | Search paginated trusted markers; no launch until record confirmed |
| Executor exits before commit | Record failed attempt without candidate, preserve session/failure/usage summary |
| Session lost or provider timeout | Bounded retry only after reconciliation; preserve unknown/duplicate usage |
| CI missing/hung/expired | Timeout or rerun under limits; inconclusive is not success |
| Head/base/policy moves | Invalidate affected authorization and evidence, re-evaluate inputs |
| Comment edited/deleted or inaccessible | Block affected reconstruction; no fabricated history or zero-cost assumption |
| Human approval stale/duplicated | Reject stale subject; repeated processing yields no extra grant |
| Cubism or OTel unavailable | Continue all scheduling, review, integration and completion; expose analytics gaps |
| Host lost | GitHub summaries survive; local native sessions may not; reconcile and abandon/inspect as necessary |

Polling is sufficient initially; no webhook service is required. If later added, duplicate/lost/reordered webhook notifications only trigger rereads of current source state. They are never an authoritative ordered journal. No uptime, zero-loss RPO or automatic cross-region recovery promises accompany a foreground local tool.

## 16. Security and retention boundary

The threat boundary is untrusted model/tool execution versus a trusted local operator, broker and protected GitHub configuration. Restrict credentials and executable trust as described in [section 10](05-trust-policy-evidence.md). Candidate tests are arbitrary code and run only within the executor/CI sandbox. [GitHub Actions security guidance](https://docs.github.com/en/actions/reference/security/secure-use) is relevant particularly to privileged workflows processing untrusted candidate code. Never execute a PR's scripts in a token-bearing broker step.

Record denial, pause, interruption and human intervention summaries without secrets. Missing protections or unsupported sandbox controls fail qualification. Host compromise and privileged GitHub administrators can defeat this profile; enterprise tenant isolation, external attestations, WORM evidence and cross-region failover are deferred, not implied guarantees.

GitHub and executor retention govern default recovery. Backing up the developer's machine is ordinary operator practice; Nightshift does not provision backup infrastructure. Stronger archival evidence is a later explicit retention requirement, not a hidden object-store prerequisite.

## 17. Cost, telemetry and Cubism

Before each launch the dispatcher reads prior attempts' confirmed/estimated usage, conservative outstanding exposure, retry counts and limits. With serial attempts it can stop further starts when a limit is reached. Use native per-request/session token/runtime caps where supported. Spend limits are admission thresholds with best-effort stopping, not strict transactional global reservations: provider reporting lag, interruption latency and unknown duplicate inference can exceed estimates. Never sell them as hard monetary caps. Unknown usage blocks further spending by default until bounded conservatively or acknowledged by the owner.

Record measured tokens, provider currency/credits, model/executor version, pricing basis/time, estimate versus reported amount, and missing fields. Native session lifetime totals must be differenced against a recorded baseline for successive attempts; forked histories are not charged twice. Subscription usage is not automatically zero and token counts do not imply an actual dollar bill. Final accounting includes failed, abandoned, review, repair and integration work. CI/compute costs can be unknown explicitly.

Asynchronously emit normalized, schema-versioned events with `event_id`, attempt/slice/milestone references, role, executor/model, event type, occurred/observed time, sequence when available, outcome/failure class, usage basis and human intervention. The adapter owns provider-specific metadata. Do not emit raw prompts, code or tool arguments by default.

Cubism computes cost per attempt, slice and milestone; retries and review cycles; executor/model effectiveness; cycle time; failure classes; and planned versus unplanned human intervention. Report merged, accepted and deployed outcomes separately. Include failed work and task/risk mix rather than rewarding pass rate or finding count alone. Price per PR/task/milestone becomes measurable without deciding a commercial billing model in the initial architecture.

Export through an optional in-memory bounded queue with timeout/drop behavior. Cubism acknowledgement is never awaited by control decisions. Restart may backfill compact lifecycle/outcome events from attempt comments using stable IDs and correction revisions; consumers deduplicate/upsert instead of counting repeated snapshots as new spend. Detailed tool events may be lost; expose completeness and observation times. A durable event delivery pipeline is deferred until measured analytics requirements justify it.

Optional [OpenTelemetry GenAI conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/) can carry execution telemetry; pin convention versions and exclude sensitive content. IDs belong on structured events/traces rather than unbounded metric-label dimensions. Telemetry is never acceptance evidence or a condition for completion.

## 18. Build-versus-buy and explicit infrastructure dispositions

| Component | Disposition and trigger |
|---|---|
| Mandatory PostgreSQL authority | DELETE from default; optional runtime coordination only for the future distributed profile |
| SQLite execution ledger | DELETE from initial architecture |
| Temporal | DEFER until durable orchestration requirements exceed the bounded poll/reconcile loop |
| Distributed lease service | DEFER until multiple active dispatchers/cross-host effects are required |
| Transactional global budget reservation | DEFER until strict shared budget enforcement is a demonstrated requirement |
| Inbox/outbox services | DEFER until durable delivery/atomic effect-intent requirements are demonstrated |
| S3-compatible object storage | DEFER until real artifact retention needs exceed existing systems |
| Full event-sourced aggregate framework | DELETE; simple records and reconciliation |
| GitHub-native work graph | KEEP as initial authority |
| Beads/Dolt | OPTIONAL later graph backend; embedded locally, server only for concurrent graph writes |
| Compact attempt comments | KEEP; human-readable modest durability |
| Executor-native sessions | KEEP through adapters; no full transcript replication |
| Trusted GitHub effect broker | KEEP in process |
| Independent review attempts | KEEP, separate role/session and exact subject |
| Content-addressed permanent artifacts | DEFER until retention requires them; keep ordinary Git SHAs/input digests |
| OpenTelemetry | KEEP optional export |
| Cubism | KEEP asynchronous and outside correctness |
| Rich operations UI | DEFER; CLI/GitHub UI first |
| Enterprise multi-tenancy, WORM evidence, cross-region failover | DEFER; no initial requirements or services |

PostgreSQL is an option only for multiple active dispatchers, cross-host effect fencing, strict transactional global budgets or a durable outbox. A future design must specify the actual atomicity boundary and retain exactly one graph backend; PostgreSQL does not retroactively give GitHub mutations transactional semantics. Beads/Dolt is suitable for work topology and replanning history, not a drop-in implementation of the old lease-and-budget protocol.

Current [Beads source](https://github.com/gastownhall/beads/blob/f56632adcfabed7da6ed0aabe4e760066b472c46/README.md) documents embedded Dolt as default/single-writer and explicit server mode for concurrent writers. Pin and qualify the version: earlier releases changed storage modes. [Dolt's server documentation](https://www.dolthub.com/docs/sql-reference/server/) distinguishes serverless `dolt sql` from MySQL-compatible `dolt sql-server`; neither makes Dolt a PostgreSQL drop-in. Local Beads adoption is optional and adds persistent graph storage, so it is outside the zero-database initial profile.

---

[Overview](index.html) · [Document index](README.md) · [Previous](06-milestones-and-product-experience.md) · [Next](08-roadmap-and-decisions.md)
