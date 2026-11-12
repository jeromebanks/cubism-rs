# Nightshift review — response to Astra

> Historical review, based on the September 2026 snapshot cited below. Claims
> that SDLC tooling is absent and the old transition order are not current
> instructions. Follow [SDLC.md](../../SDLC.md), the
> [adopted execution plan](../sdlc/NIGHTSHIFT_EXECUTION_PLAN.md), and
> [continue-plan](../../.agents/skills/continue-plan/SKILL.md).
> Live progress and decisions are on [epic #38](https://github.com/jeromebanks/cubism-rs/issues/38).
> The original evidence below is preserved.

Jerome asked for a human read of the Nightshift design package
(`docs/nightshift-design/`) and a plan for moving Cubism onto it.

Three documents:

| File | Subject |
|---|---|
| [01-docs-and-pages-feedback.md](01-docs-and-pages-feedback.md) | How the docs and the Pages site should be restructured for a human reader |
| [02-design-critique.md](02-design-critique.md) | The design itself — what is right, what is over-invested, what is missing |
| [03-cubism-transition-plan.md](03-cubism-transition-plan.md) | Concrete ordered steps to move Cubism from its current state onto Nightshift Stage 0 |

**Deliberately outside `docs/nightshift-design/`.** That folder is published to
GitHub Pages via `git subtree split --prefix=docs/nightshift-design`, and
`scripts/render-nightshift-docs.py --check` validates every file in it. Review
notes should not become a public page or break the reproducibility check.

## Which of these stay with Cubism

`03-cubism-transition-plan.md` is Cubism's own record — its transition steps and
the decisions of record at the end. It stays.

`01-docs-and-pages-feedback.md` and `02-design-critique.md` are addressed to the
Nightshift design and travel with it when that work moves to its own repository
(see D4 in `03`).

## The one fact that reframes everything

The SDLC apparatus is not on `main`.

Verified server-side against `origin/main` at `480df23`, not a stale local ref:

```
$ gh api repos/jeromebanks/cubism-rs/contents --jq '.[].name'
.claude  .github  .gitignore  Cargo.lock  Cargo.toml  README.md
TIMESERIES_FINISH_PROMPT.md  bindings  crates  deny.toml  docs
examples  rust-toolchain.toml  spark-adapter

$ gh api repos/jeromebanks/cubism-rs/contents/scripts
{"message":"Not Found","status":"404"}
```

No `SDLC.md`, no `AGENTS.md`, no `CLAUDE.md`, no `scripts/`, no `.sdlc/`, no
`.agents/`. `git diff --stat main..HEAD` shows `scripts/sdlc.py | 902 +++++` —
a pure addition. And `main`'s `.claude/skills/` contains exactly two skills:
`spark-setup` and `timeseries-slice` — the one `SDLC.md` explicitly retires.

(Note for the design package's evidence baseline: it records local main as
`1ae95dd`. `origin/main` has since advanced to `480df23` through PR #64. The
finding is unaffected — a checksum-message whitespace fix does not add
`scripts/` — but the baseline SHA is stale.)

A fresh clone of `main` gets the retired process and nothing else. Everything
described in the Nightshift package as "the prototype" lives on one commit
named *"checkpoint in-progress SDLC tooling before it's lost."*

This is not a nitpick about branch hygiene. It means Stage 0's framing —
"repair known gate failures" — understates the work by one step. There is no
gate on the default branch to repair.
