# Nightshift

**Big plans. Small slices. Verified delivery.**

A governed autonomous software factory: humans define outcomes and milestone contracts; agents execute bounded slices; independent review and deterministic controls establish evidence; humans accept integrated results.

Start with the [TL;DR HTML overview on GitHub Pages](https://jeromebanks.github.io/cubism-rs/). It links to the complete HTML reading editions. The [local HTML overview](index.html) remains available for offline use.

## Brand

**Nightshift** evokes a factory that keeps delivery moving between human checkpoints. It is the proposed product name. Astra refers only to a model option; it is not the product brand. This package does not claim trademark or domain availability.

## Reading guide

| Document | Original sections | Contents |
|---|---|---|
| [Product, prototype assessment, and principles](01-product-and-prototype.md) | 1–4 | Executive thesis and product definition; Evidence-grounded assessment of the prototype; Retain / repair / replace / add; Product principles and invariants |
| [Architecture and work model](02-architecture-and-work-model.md) | 5–6 | Recommended reference architecture; Domain model and work planning |
| [Durable events and state machines](03-durable-state-machines.md) | 7 | Durable event and state model |
| [Scheduling, execution, and adversarial review](04-execution-scheduling-review.md) | 8–9 | Scheduling and lease protocol; Provider-neutral execution and adversarial review |
| [Trust, policy, and evidence](05-trust-policy-evidence.md) | 10–12 | Trust, identity, and threat model; Policy as code; Evidence and attestation model |
| [Milestone contracts and product experience](06-milestones-and-product-experience.md) | 13–14 | Human milestone contract; UI, API, and CLI surfaces |
| [Reliability, security, economics, and platform choices](07-reliability-security-economics.md) | 15–18 | Reliability, integration, and disaster recovery; Security and multi-tenancy; Observability, evaluation, economics, and commercial model; Build-versus-buy decisions |
| [Migration roadmap, risks, and architectural decisions](08-roadmap-and-decisions.md) | 19–21 | Migration roadmap and independently reviewable slices; Top risks, prioritized questions, and what not to build; Recommended architectural decisions |

## Complete section map

1. [Executive thesis and product definition](01-product-and-prototype.md)
2. [Evidence-grounded assessment of the prototype](01-product-and-prototype.md)
3. [Retain / repair / replace / add](01-product-and-prototype.md)
4. [Product principles and invariants](01-product-and-prototype.md)
5. [Recommended reference architecture](02-architecture-and-work-model.md)
6. [Domain model and work planning](02-architecture-and-work-model.md)
7. [Durable event and state model](03-durable-state-machines.md)
8. [Scheduling and lease protocol](04-execution-scheduling-review.md)
9. [Provider-neutral execution and adversarial review](04-execution-scheduling-review.md)
10. [Trust, identity, and threat model](05-trust-policy-evidence.md)
11. [Policy as code](05-trust-policy-evidence.md)
12. [Evidence and attestation model](05-trust-policy-evidence.md)
13. [Human milestone contract](06-milestones-and-product-experience.md)
14. [UI, API, and CLI surfaces](06-milestones-and-product-experience.md)
15. [Reliability, integration, and disaster recovery](07-reliability-security-economics.md)
16. [Security and multi-tenancy](07-reliability-security-economics.md)
17. [Observability, evaluation, economics, and commercial model](07-reliability-security-economics.md)
18. [Build-versus-buy decisions](07-reliability-security-economics.md)
19. [Migration roadmap and independently reviewable slices](08-roadmap-and-decisions.md)
20. [Top risks, prioritized questions, and what not to build](08-roadmap-and-decisions.md)
21. [Recommended architectural decisions](08-roadmap-and-decisions.md)

## Preservation and scope

This is the complete prior design, divided by topic rather than abridged. All 21 numbered sections, transition tables, diagrams, illustrative contracts and attestations, diagnostic results, recommendations, alternatives, stage criteria, risks, open questions, and ADR proposals are retained. The HTML is an additional summary, not a substitute for those documents.

Packaging changes:

- Rebranded the product, illustrative CLI, domains, workload identities, and diagram labels to Nightshift.
- Corrected the model/harness distinction for Astra in section 9 and the qualification question in section 20.
- Converted repository evidence links to commit-pinned GitHub links with line anchors so the package can be moved or shared. PostScript source links use the HEAD recorded during the historical comparison; those reads were from its checkout, not a separate immutable-worktree audit.
- Added navigation, document titles, and this provenance note. Removed only the chat renderer's memory-citation footer from the document body.

The original design's statements that no files or GitHub state were changed describe the earlier read-only assessment. This packaging task adds this documentation folder. It does not implement the proposed platform or revise the inspected prototype.

## Evidence baseline

| Item | Inspected value |
|---|---|
| Cubism branch | `chore/checkpoint-sdlc-worktree-state` |
| Cubism commit | `0cbaf2d3c92ea30eafe507a5460894f5322ccc69` |
| Local main | `1ae95dd9fdfb56febf463c3fd5601ddafbeac4d0` |
| PostScript HEAD observed | `1a5bb6520dfb9fea54a0a4fe34930e9dafa5d5e7` |
| Original instructions | `astra-sdlc.txt`, read completely in the preceding assessment |
| Packaging date | 2026-09-09 |

Eight existing SDLC unit tests passed during the earlier assessment. In-memory diagnostics reproduced receipt, check-name, feedback-gate, and applied-merge defects without external mutations. The branch-claim race was established by code/interleaving analysis and documented Git semantics, not a remote race experiment. Live protections, production deployments, hosted demos, and provider/harness capability were not verified.

The migration entries are proposed work, not existing GitHub issues. Illustrative identifiers, digests, prices, availability targets, and CLI commands are design examples. Markdown Mermaid blocks require a Mermaid-capable viewer; the HTML overview needs no renderer, scripts, fonts, or network resources.

External primary-source links are preserved beside their claims throughout the full documents. They support the original design assessment; this packaging pass did not refresh external platform research.

## HTML reading editions and publication

Every Markdown document, including this guide, has a same-name HTML reading edition. Start at [the overview](index.html) or [the HTML reading guide](README.html). HTML navigation stays in HTML; each page also offers the unchanged Markdown source for agents. The overview remains self-contained. Full reading editions use local `reader.css` and `reader.js`; only diagram rendering loads an external dependency (Mermaid 11.12.0 from jsDelivr). All prose, tables, examples, and diagram source remain readable without JavaScript or network access. Diagram sources collapse only after successful rendering.

The publication target is [GitHub Pages](https://jeromebanks.github.io/cubism-rs/). Source documents and generated HTML live on `docs/nightshift-design-draft`. A dedicated `nightshift-pages` branch contains only this folder, extracted with Git subtree. Pages publishes that branch's root, with `.nojekyll` preventing Markdown rewriting. Nothing is merged into `main`. This uses the repository's single Pages site; a future broader documentation site will need an explicit integration decision.

To rebuild, use an isolated Python environment with the pinned renderer. From the repository root (replace the example temporary environment path with your own):

```bash
rtk python3 -m venv /private/tmp/nightshift-docs-venv
rtk /private/tmp/nightshift-docs-venv/bin/pip install -r scripts/nightshift-docs-requirements.txt
rtk /private/tmp/nightshift-docs-venv/bin/python scripts/render-nightshift-docs.py
rtk /private/tmp/nightshift-docs-venv/bin/python scripts/render-nightshift-docs.py --check
```

The checker verifies byte-for-byte reproducibility and all local HTML links, anchors, and assets. Each reading edition embeds the SHA-256 digest of its Markdown source. `--patch` emits an `apply_patch` payload instead of writing files directly. Review and commit the generated HTML alongside source edits.

After the reviewed source changes are committed on `docs/nightshift-design-draft`, publish updates without a main-branch merge:

```bash
rtk git push origin docs/nightshift-design-draft
rtk git subtree split --prefix=docs/nightshift-design -b nightshift-pages
rtk git push origin nightshift-pages
```

The subtree command advances the local publication branch when its history is compatible; do not force-push if it reports divergence. Pages must be configured once to publish `nightshift-pages` at `/`. Check the Pages build and live URLs after every publication. Build and publishing operations do not validate or implement the proposed software-factory architecture.
