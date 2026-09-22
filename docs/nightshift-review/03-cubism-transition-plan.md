# Moving Cubism onto Nightshift

> Historical transition proposal, not the current execution order. Its tooling
> baseline and PostgreSQL-first direction are superseded by the
> [adopted execution plan](../sdlc/NIGHTSHIFT_EXECUTION_PLAN.md).
> Follow [SDLC.md](../../SDLC.md) and
> [continue-plan](../../.agents/skills/continue-plan/SKILL.md); reconstruct
> progress from [epic #38](https://github.com/jeromebanks/cubism-rs/issues/38).
> Preserve the evidence below; do not execute these old steps as current policy.

Ordered, with a stated end state per step. Step 0 is a prerequisite the design
does not mention.

## Where Cubism actually is

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

## Step 0 — Land the tooling on `main`

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

## Step 1 — Make the gate real  ·  *done in the working tree, uncommitted*

The gate was 902 lines of untested, never-successfully-executed code. Items 1–5
below are now implemented on `docs/nightshift-design-draft`; item 6 is a design
decision, not a patch, and remains open.

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
6. **Be honest about the claim race.** *(still open)* The `issue/N` push at `sdlc.py:594` is not
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
