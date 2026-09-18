# Moving Cubism onto Nightshift

Ordered, with a stated end state per step. Step 0 is a prerequisite the design
does not mention.

## Status

Steps 0 and 1 are **done and merged** (PRs #66, #68, #69). Step 2 is next and
nothing in the repository has reached it yet. The record below is kept as
written so the starting position stays legible; the "as reviewed" table is
**historical**, not current.

| Now on `main` (`236ef35`) | |
|---|---|
| SDLC tooling | Landed. `SDLC.md`, `AGENTS.md`, `CLAUDE.md`, `.sdlc/`, `scripts/`, `.agents/`, `.claude/skills/` |
| Gate defects | All five fixed: applied merge, self-authored receipts, stale passes, feedback exemption, claim exclusivity |
| Tests | 8 → 28, running in CI as the `sdlc` job inside `CI required checks` |
| Engine portability | `scripts/sdlc.py` contains zero project-specific strings; markers are `nightshift-review` / `nightshift-gate` and pinned by test |
| Status page | Untracked; `status --json` is the projection surface |
| `type:slice` issues | **Still 0.** `claim` → Codex receipt → `merge --apply` has never run end to end |
| Label drift | Still open: #55 closed + `in-review`; #53 open + `in-review` with PR #65 open |
| Unmapped issues | 22 of 45 open |

## Where Cubism was, as reviewed on 2026-09-15

| Fact | Value |
|---|---|
| SDLC tooling on `main` | **None.** No `SDLC.md`, `AGENTS.md`, `CLAUDE.md`, `scripts/`, `.sdlc/`, `.agents/` |
| Skills on `main` | `spark-setup`, `timeseries-slice` — the latter explicitly retired by `SDLC.md` |
| Checkpoint branch | 56 files, 8,244 insertions, one commit, mixing tooling + `docs/sdlc/` + Nightshift package |
| Epics | 12, every one labeled `needs-slicing` |
| `type:slice` issues | **0** — `check-slice` cannot pass for anything in the repo |
| Unclassified legacy issues | ~37 (#1–#37) |
| Label drift | #55 closed + `in-review`; #53 open + `in-review` with PR #65 open |
| Successful `sdlc.py merge --apply` runs | **0** — `NameError` at `scripts/sdlc.py:686` fires before `gh pr merge` |
| `scripts/tests/test_sdlc.py` in CI | No |
| Real engineering debt | #1, #2, #8, #14, #59 (two live RUSTSEC CVEs) |

Everything the Nightshift package calls "the prototype" is unmerged work on a
branch named "before it's lost."

---

## Step 0 — Land the tooling on `main`  ·  **done, merged in #66**

Landed as **two** PRs, not three. The three-way split does not work:
`docs/sdlc/HUMAN_TUTORIAL.md` links `../../SDLC.md` and `../../scripts/sdlc.py`,
and the CI link check covers `docs/**/*.md`, so a docs-only PR cannot pass CI
without the tooling. The lifecycle policy and its human-facing explanation are
one concern.

Nothing else can proceed. But **do not merge the checkpoint branch as one
commit** — 56 files spanning three unrelated concerns is exactly the kind of
change the process it introduces would reject.

Split it into three PRs:

| PR | Contents | Notes |
|---|---|---|
| A | `SDLC.md`, `AGENTS.md`, `CLAUDE.md`, `.sdlc/config.json`, `scripts/sdlc.py`, `scripts/quality-gate.sh`, `scripts/tests/`, `.agents/skills/`, `.claude/skills/{work-slice,plan-epic,review-milestone,project-status}`, `.github/ISSUE_TEMPLATE/` | The process itself |
| B | `docs/sdlc/`, `docs/project-status.html` | Human-facing process docs |
| C | `docs/nightshift-design/`, `scripts/render-nightshift-docs.py`, requirements | Stays on its own branch + Pages; not on `main` unless you want it there |

In PR A, also delete `.claude/skills/timeseries-slice/` from `main` and move
`TIMESERIES_FINISH_PROMPT.md` and the ~45 `docs/TIMESERIES_*` files to
`docs/history/timeseries/`. `SDLC.md` already declares them retired; `main`
still ships them as the only process it has.

These three PRs cannot themselves follow the slice process — the process isn't
there yet. Say so in the PR descriptions rather than pretending otherwise.

**End state:** a fresh clone of `main` has one coherent, current process.

## Step 1 — Make the gate real  ·  **done, merged in #66 and #69**

The gate was 902 lines of untested, never-successfully-executed code. All six
items below are now on `main`. Item 6 was resolved rather than documented: the
claim is genuinely exclusive, via GitHub's create-only ref endpoint.

1. **Fix the `NameError`.** Line 686 reads `if add or remove:` — both are locals
   of `command_set_gate` at line 715. Delete the condition; the `run_text(command)`
   below it is the actual merge. Add a mock-based test asserting the exact
   `gh pr merge` argv, including `--<method>` and `--delete-branch`.
2. **Add `scripts/tests/test_sdlc.py` to `CI required checks`.** `ci.yml` today
   runs Rust fmt/clippy/test/doctest/rustdoc, schema validation, link checking,
   and a Python bindings import smoke test — and never runs the SDLC tests. The
   `python` job at `ci.yml:123` already has a 3.12 interpreter; add the step and
   list the job in the `needs:` aggregate at line 153.
3. **Reject self-authored receipts.** Compare the receipt comment author to
   `pr.author.login` in `evaluate_merge_gate` (`sdlc.py:467`) and fail on a
   match. Ten lines, and it closes the defect the design calls a "confirmed
   authorization defect."
4. **Resolve receipts newest-first** per `(kind, head_sha)` so a later `fail`
   supersedes an earlier `pass`, rather than any-pass-wins.
5. **Narrow the feedback exemption** at `sdlc.py:372` to `gate:changes-requested`
   only, and require a link to the feedback decision — matching `AGENTS.md`,
   which is already stricter than the code.
6. **Be honest about the claim race.** *(fixed in #69, not merely documented)* The `issue/N` push at `sdlc.py:594` is not
   an exclusive claim. Either fix it or write the limitation into `SDLC.md`.
   Single-operator today makes it low-risk; silently implying safety is the
   problem, not the race.

Leave the identity, attestation, and lease-service work alone. It is Stage 1+.

**End state:** every defect in §2 of the design is fixed or documented as a
known limitation, and CI proves it.

## Step 2 — Prove the loop once, end to end

File Stage 0 slices 0.1–0.5 as real `type:slice` issues under **epic #38**
("Modernize Cubism SDLC and repository governance") — it already exists and is
the natural parent. These become the first issues in the repo that pass
`check-slice`, which also validates the checker against real input.

Then run the full loop on one of them: claim → worktree → implement → PR →
fresh Codex review → `review-receipt` → `merge --apply`.

**Do not skip the merge command.** Running it once successfully is the entire
point — it has never completed.

**End state:** one slice has traversed the documented path with no manual steps.

## Step 3 — Run one real epic end to end

**Epic #40, "Expose the correction engine through CLI and operations."**

It is the best candidate because two natural candidate slices already exist as
well-specified issues: **#56** (`CorrectionEngine::correct` — 8 positional
params should become a request struct before the CLI lands) and **#57**
(`LatenessPolicy` never consulted on the executable correction path). Both are
bounded, testable in one session, and produce visible CLI behavior — which means
the milestone report and demo have something real to show.

Neither is currently linked to #40 — the epic body references no child issues
and both carry no labels. That unlinked state is itself evidence for Step 4:
the repository has good work items and no graph connecting them.

Decompose #40 with `plan-epic`, run every slice through the loop, then exercise
the human gate for real: `gate:human-review`, a milestone manifest, a rendered
report, and an actual approve-or-feedback decision from you.

Track human minutes across the epic — writing issues, reviewing the report,
adjudicating findings. That number decides whether Stage 1 is worth funding.

**End state:** one epic delivered under the process, one milestone accepted, and
a measured cost per slice.

## Step 4 — Reconcile the backlog

Only after Step 3. Ordering matters: decomposing 12 epics before the loop is
proven repeats the time-series failure — heavy planning against an unvalidated
process.

- Fix label drift: remove `in-review` from #55 (closed), reconcile #53 against
  PR #65.
- Triage #1–#37. Most are findings, not work items. Label them
  `type:finding`/backlog, close what is stale, and attach the rest to an epic.
  Do not retrofit the slice template onto issues nobody will implement.
- Decompose epics **on demand only** — when you are about to work one. Twelve
  `needs-slicing` epics is an honest backlog; twelve fully decomposed epics
  would be forty stale issues.
- Handle #59 (two live RUSTSEC CVEs in pyo3 0.28.3, blocked on arrow 58→59 +
  datafusion) outside this sequence. It is a security item, not a process
  demonstration, and should not wait on governance.

**End state:** every open issue is either a slice, attached to an epic, or
explicitly backlog.

## Step 5 — Decide about Stage 1

After Step 3 you will have the number that matters: human minutes per slice.

- **Under ~20 minutes/slice** — the process is carrying its weight. Stage 1
  (PostgreSQL kernel) becomes a real question.
- **Over that** — the process is the bottleneck. Simplify Stage 0 rather than
  building a platform to industrialize the overhead.

Either way, this is where the Nightshift-as-product decision belongs — informed
by evidence from your own repository, which is the strongest argument the design
could possibly have.

---

## What not to do

- **Don't build Stage 1 first.** PostgreSQL + Temporal + OPA is months of work
  to govern one developer's Rust library.
- **Don't decompose all 12 epics up front.** That is the time-series phase-plan
  failure mode with new labels.
- **Don't merge the checkpoint branch as-is.** Three PRs, three concerns.
- **Don't let the factory become the project.** Cubism has real unfinished
  engineering — #1, #2, #8, #14, #59. The process exists to ship those. If a
  quarter passes with SDLC commits and no crate commits, the process failed
  regardless of how well it works.

---

# Decisions of record

Made in session on 2026-09-15/16. Recorded here because they are architectural
commitments with consequences, not session notes: without a durable record the
next session either re-litigates them or silently contradicts them.

## D1 — Adopt Stage 0; defer Stages 1–4 pending evidence

Stage 0 is Cubism's process as of #66. Stages 1–4 (PostgreSQL kernel, hosted
runners, multi-tenancy, enterprise) are a **separate product decision**, to be
made on the human-minutes-per-slice figure that Step 3 produces — not on the
design's own merits.

Stages 1–4 describe a different product than Cubism. Cubism is a Rust
aggregation library with real unfinished engineering and two live CVEs. The
factory must not become the project: a quarter of SDLC commits with no crate
commits means the process failed, however well it works.

## D2 — `scripts/sdlc.py` is Nightshift Stage 0 and will be peeled out

The engine is intended to serve every project Nightshift manages, not only this
one. It already contains no project-specific string, and its comment protocol
markers are `nightshift-`, not `cubism-` (#69).

Consequence: changes to this engine are Nightshift design evidence, not just
Cubism maintenance. Keep it project-neutral by default; anything Cubism-specific
belongs in `quality-gate.sh` or `.sdlc/config.json`.

## D3 — The coupling surface between Nightshift and a managed project is three files

A project (Cubism, or any later one) retains exactly:

| File | Role |
|---|---|
| `.sdlc/config.json` | Tenant registration — repository, branch, labels, required checks, review policy |
| `scripts/quality-gate.sh` | **Project adapter** — "here is how you verify me." The engine invokes it and never inspects its contents |
| `.github/ISSUE_TEMPLATE/` | Nightshift's contract schema, installed per project |

Everything else — the engine, `SDLC.md`, `docs/sdlc/` — belongs to Nightshift.
Keep this surface at three files. Projects managed by Nightshift have no links
to each other; cross-project visibility is a Nightshift concern, not a
project-repo concern.

## D4 — Split the Nightshift repository when it has an executable artifact

Not before. Today Nightshift is ~15,000 words of Markdown and a renderer; a
repository for documents is overhead, not separation. Once there is a package
with a schema and tests, the split is obvious and natural.

Sequencing matters: Nightshift's only evidence is Cubism. Moving the design to a
repository with no code makes it a document about a hypothetical, reviewable only
on prose — and it cannot produce the D1 human-minutes figure.

Interim measure that captures most of the benefit: keep `docs/nightshift-design/`
off `main` (it publishes from the `nightshift-pages` branch), so its ~5,500 lines
stay out of every clone, out of `docs/**/*.md` link checking, and out of agent
context. Its evidence links are commit-pinned URLs, so they survive the move.

## D5 — Nightshift's design is missing a project-adapter layer

The design specifies **source adapters** (§5, GitHub first) and **harness
adapters** (§9, `describe_capabilities`/`qualify`/`start`). It has no equivalent
for the project: the standing declaration of how to verify, build, and demo a
given repository.

`quality-gate.sh` is empirically that thing (see D3). §6's slice contract carries
per-slice `validation.commands`, which is finer-grained and does not cover the
repository-level contract. This is the interface that makes a second project
onboardable without Nightshift knowing anything about its domain, and it should
be added to the design.

## What a handoff is for

These decisions are durable records. A session handoff is a different artifact:
reading order, what is half-done, what to start with — **pointers only**.

The retired `TIMESERIES_PHASE_N_HANDOFF.md` chain is what happens when the two
are confused: state lived in prose, drifted from reality, grew without bound, and
made every session load 20KB to learn what a label would have said.

The test is one question — *what breaks if this file disappears?* "I would have
to re-derive the reading order" is a handoff, and that is fine. "We would lose
why we chose X" means X belonged here instead.
