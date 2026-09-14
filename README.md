# Nightshift

**Big plans. Small slices. Verified delivery.**

An open-source orchestration tool for an individual developer or small team: one milestone → dependency-aware slices → multiple implementation/review/repair/integration attempts → verified integration → human acceptance.

The recommended starting point is **GitHub-native work topology and compact attempt comments, one local dispatcher, executor-native sessions, an in-process trusted effect broker, and asynchronous Cubism analytics**. No PostgreSQL, SQLite, Dolt server, Temporal or persistent Nightshift infrastructure is required. GitHub comments provide modest durability and explicit reconciliation, not database transactions.

Start with the [HTML overview](index.html) or [HTML reading guide](README.html). This is a documentation-only revision of the existing design on `docs/nightshift-design-draft`, not a separate proposal or implemented platform.

## Reading guide

| Document | Sections | Contents |
|---|---|---|
| [Product and prototype](01-product-and-prototype.md) | 1–4 | Initial product boundary, current repository findings, preserved historical evidence, required invariants |
| [Architecture and work model](02-architecture-and-work-model.md) | 5–6 | Sources of truth, profiles, native GitHub API/CLI, topology and replanning |
| [Attempt journal and state machines](03-durable-state-machines.md) | 7 | Compact schema, allocation ordering, transitions and ambiguity |
| [Execution and independent review](04-execution-scheduling-review.md) | 8–9 | One dispatcher, retry bounds, executor interface, Factory lessons, structured review |
| [Trust, policy and evidence](05-trust-policy-evidence.md) | 10–12 | In-process broker, idempotency, protected policy, verification and retention |
| [Milestones and product experience](06-milestones-and-product-experience.md) | 13–14 | Integration, exact checkpoint acceptance, GitHub UI and CLI |
| [Reliability and economics](07-reliability-security-economics.md) | 15–18 | Restart reconciliation, merge limitations, cost/Cubism, infrastructure dispositions |
| [Roadmap and decisions](08-roadmap-and-decisions.md) | 19–21 | Small Cubism milestones, risks, qualification gates and revised ADRs |

## Revision and evidence baseline

| Item | Inspected value |
|---|---|
| Revision date | 2026-09-13 |
| Starting design branch | `docs/nightshift-design-draft` |
| Starting commit | `231a161f06a354f34b1c6184a66a813f3d84f8c0` |
| Historical Cubism assessment | `0cbaf2d3c92ea30eafe507a5460894f5322ccc69`, preserved in chapter 1 |
| Beads source inspected | `f56632adcfabed7da6ed0aabe4e760066b472c46` |
| Factory droid-action inspected | `b46affde010c543aefc51b5d82618b8e825845ca` |
| Factory VFS inspected | `4852148cecde9c5413b51e4184eac55015bfe766` |

All eight existing Markdown chapters, this guide, HTML reading editions and the hand-maintained overview were audited. Repository inspection included `AGENTS.md`, `SDLC.md`, `.sdlc/config.json`, relevant `scripts/sdlc.py` paths and the documentation renderer. The older prototype findings retain commit-pinned links and explicitly historical diagnostic results. They are not fresh runtime tests. No Nightshift code, infrastructure, issue topology or active SDLC policy is changed by this revision.

The old mandatory database/workflow architecture is superseded throughout. Section numbers remain for navigation; old ADR numbers now state revised decisions. Git history retains the prior design. Current upstream sources were checked for native GitHub relationships/CLI, Checks/retention limits, Factory execution/session/compute capabilities, Beads storage modes and Dolt. Factory VFS's manifest/README declare MIT and version 1.1.1; runtime maturity was not independently tested. No source is copied from Factory repositories. `gh` and `rtk` were unavailable in the inspection environment, so CLI flags were checked against the current official manual and underlying shell commands used for documentation work.

Live GitHub protections/installed credential capabilities and executor sandbox/session behavior remain implementation qualification gates. No unresolved architecture choice blocks starting the initial profile. Illustrative schemas, commands, identifiers and limits are proposed contracts, not shipped interfaces or provider prices.

## Profiles and deliberate limits

GitHub is the one authoritative initial work graph. Beads with embedded Dolt is an optional later local profile; Dolt server mode is for genuine concurrent graph writers. PostgreSQL becomes an option only for demonstrated multiple active dispatchers, cross-host effect fencing, strict transactional global budgets or a durable outbox. A deployment must not duplicate authoritative graph state across those backends.

Attempts remain execution records, never GitHub/Beads work items or per-attempt Dolt branches. Native session state remains executor-owned. The broker enforces protected effects under trusted policy. Recovery can lose native context or duplicate bounded computation; uncertain protected effects block rather than being blindly repeated. Comment history and raw CI artifacts are not permanent audit storage. Cubism/telemetry outages never prevent correctness.

## HTML reading editions and validation

Each Markdown document has a same-name generated HTML edition. The overview is hand-maintained and self-contained; full editions use `reader.css` and `reader.js`, loading Mermaid 11.12.0 from jsDelivr only for visual diagrams. Text and diagram source remain readable offline. The renderer embeds each Markdown source's SHA-256 and checks local links/anchors/assets plus byte-for-byte reproduction.

From the repository root, with the pinned renderer installed:

```bash
python3 -m venv /tmp/nightshift-docs-venv
/tmp/nightshift-docs-venv/bin/pip install -r scripts/nightshift-docs-requirements.txt
/tmp/nightshift-docs-venv/bin/python scripts/render-nightshift-docs.py
/tmp/nightshift-docs-venv/bin/python scripts/render-nightshift-docs.py --check
git diff --check
rg -ni 'PostgreSQL|Temporal|SQLite|lease|transaction|inbox|outbox|object stor|event.sourc' docs/nightshift-design
```

Use the repository's `rtk` prefix when installed. Review generated HTML alongside Markdown and separately inspect overview consistency. The infrastructure search intentionally finds explicit removals, limitations and future profiles; it must not find mandatory default services. Inspect attempt identity, policy origin, diagrams, recovery, retention and asynchronous analytics as semantic checks too.

## Branch and publication scope

Source documents and generated HTML live on `docs/nightshift-design-draft`. This revision updates that branch. It does not merge into `main` or publish the existing [GitHub Pages site](https://jeromebanks.github.io/cubism-rs/). The site has a separate `nightshift-pages` publication branch and may show an older design until a separately requested publication. Do not treat the live Pages content as proof that this branch has been deployed.
